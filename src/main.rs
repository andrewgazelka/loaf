// CLI binary needs to print output
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI binary output"
)]

mod db;
mod nfs;
mod overlay;
mod sandbox;

use color_eyre::eyre::WrapErr as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::signal::unix::{SignalKind, signal};

const DEFAULT_LOG_FILE: &str = "/tmp/loaf.log";

#[derive(clap::Parser)]
#[command(name = "loaf", version, about = "Overlay filesystem for macOS via NFS")]
struct Cli {
    /// Enable verbose debug logging to terminal (logs always written to file)
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Log file path (default: see DEFAULT_LOG_FILE constant)
    #[arg(long, global = true, default_value = DEFAULT_LOG_FILE)]
    log_file: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Mount an overlay filesystem on a directory
    Mount {
        /// Directory to mount the overlay on
        path: PathBuf,
        /// Port for NFS server (default: OS-assigned)
        #[arg(short, long)]
        port: Option<u16>,
    },
    /// Unmount the overlay filesystem
    Unmount {
        /// Mount point to unmount
        path: PathBuf,
    },
    /// Run a command in an ephemeral overlay sandbox
    Run {
        /// Command to run
        command: String,
        /// Arguments to pass to the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
        /// Disable sandbox (for debugging)
        #[arg(long)]
        no_sandbox: bool,
    },
    /// Show pending changes in the overlay
    Diff {
        /// Path to overlay database (default: find .loaf in current/parent dirs)
        overlay: Option<PathBuf>,
    },
    /// Apply overlay changes to the real filesystem
    Accept {
        /// Path to overlay database (default: find .loaf in current/parent dirs)
        overlay: Option<PathBuf>,
    },
    /// Discard all overlay changes
    Reject {
        /// Path to overlay database (default: find .loaf in current/parent dirs)
        overlay: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    color_eyre::install()?;

    // Install panic hook for best-effort cleanup on crash
    install_panic_hook();

    let cli = <Cli as clap::Parser>::parse();

    // Set up logging - always write to file, optionally to terminal with --verbose
    let log_file_path = cli.log_file.clone();
    let log_file = std::fs::File::create(&log_file_path)
        .wrap_err_with(|| format!("failed to create log file at {}", log_file_path.display()))?;

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(log_file)
        .with_ansi(false);

    let env_filter = if cli.verbose {
        tracing_subscriber::EnvFilter::new("debug")
    } else {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };

    if cli.verbose {
        // With --verbose: log to both file and terminal
        let stdout_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .with(stdout_layer)
            .init();
    } else {
        // Default: log only to file, keep terminal clean
        tracing_subscriber::registry()
            .with(env_filter)
            .with(file_layer)
            .init();
    }

    // Clean up any stale mounts from previous crashes
    cleanup_stale_mounts().await?;

    match cli.command {
        Commands::Mount { path, port } => mount_command(path, port).await?,
        Commands::Unmount { path } => unmount_command(path).await?,
        Commands::Run {
            command,
            args,
            no_sandbox,
        } => run_command(command, args, no_sandbox).await?,
        Commands::Diff { overlay } => diff_command(overlay)?,
        Commands::Accept { overlay } => accept_command(overlay)?,
        Commands::Reject { overlay } => reject_command(overlay)?,
    }

    Ok(())
}

/// State file for tracking active mounts
#[derive(serde::Serialize, serde::Deserialize)]
struct MountState {
    port: u16,
    overlay_path: PathBuf,
}

/// Global flag to track if cleanup is in progress (for panic hook)
static CLEANUP_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Global mount path for panic hook cleanup (set when mount is active)
static ACTIVE_MOUNT_PATH: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

fn set_active_mount(path: Option<PathBuf>) {
    let mutex = ACTIVE_MOUNT_PATH.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = mutex.lock() {
        *guard = path;
    }
}

fn get_active_mount() -> Option<PathBuf> {
    ACTIVE_MOUNT_PATH
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|guard| guard.clone())
}

/// Install panic hook for best-effort cleanup
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Prevent recursive cleanup
        if CLEANUP_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            if let Some(mount_path) = get_active_mount() {
                eprintln!(
                    "\nPanic detected, attempting emergency unmount of {}...",
                    mount_path.display()
                );
                // Best-effort sync unmount via syscall
                let _ = nfs::unmount_sync(&mount_path);
            }
        }
        // Call the default hook (prints panic info)
        default_hook(info);
    }));
}

/// Check if a path is currently mounted (macOS)
async fn is_mounted(path: &std::path::Path) -> color_eyre::Result<bool> {
    let output = tokio::process::Command::new("mount")
        .output()
        .await
        .wrap_err("failed to run mount command")?;

    let mount_output = String::from_utf8_lossy(&output.stdout);
    let path_str = path.to_string_lossy();

    // macOS mount output: "localhost:/ on /path/to/mount (nfs, ...)"
    Ok(mount_output.lines().any(|line| line.contains(&*path_str)))
}

/// Detect and clean up stale mounts from previous crashes
async fn cleanup_stale_mounts() -> color_eyre::Result<()> {
    // Check current directory and parents for orphaned .loaf.state files
    let Ok(mut current) = std::env::current_dir() else {
        return Ok(());
    };

    loop {
        let state_path = current.join(".loaf.state");
        if state_path.exists() {
            tracing::info!("found state file at {}", state_path.display());

            // Read and parse state
            if let Ok(state_json) = std::fs::read_to_string(&state_path) {
                if let Ok(state) = serde_json::from_str::<MountState>(&state_json) {
                    // The mount point is the parent of .loaf (state_path is .loaf.state, overlay is .loaf)
                    let mount_point = state
                        .overlay_path
                        .parent()
                        .unwrap_or(&current)
                        .to_path_buf();

                    // Check if this mount is actually stale (no server responding)
                    if is_mounted(&mount_point).await.unwrap_or(false) {
                        // Check if NFS server is still alive by trying to connect
                        let server_alive =
                            tokio::net::TcpStream::connect(format!("127.0.0.1:{}", state.port))
                                .await
                                .is_ok();

                        if !server_alive {
                            tracing::warn!(
                                "stale mount detected at {} (server on port {} not responding), cleaning up",
                                mount_point.display(),
                                state.port
                            );

                            // Try to unmount
                            if let Err(e) = nfs::unmount_nfs(&mount_point).await {
                                tracing::warn!("failed to unmount stale mount: {e}");
                            } else {
                                println!("✓ Cleaned up stale mount at {}", mount_point.display());
                            }

                            // Remove stale state file
                            std::fs::remove_file(&state_path).ok();
                        }
                    } else {
                        // Not mounted but state file exists - just clean up the state file
                        tracing::info!("removing orphaned state file at {}", state_path.display());
                        std::fs::remove_file(&state_path).ok();
                    }
                }
            }
        }

        if !current.pop() {
            break;
        }
    }

    Ok(())
}

async fn mount_command(path: PathBuf, port: Option<u16>) -> color_eyre::Result<()> {
    // Validate path exists
    if !path.exists() {
        color_eyre::eyre::bail!(
            "mount path does not exist: {}\n\
             Create the directory first with: mkdir -p {}",
            path.display(),
            path.display()
        );
    }

    if !path.is_dir() {
        color_eyre::eyre::bail!(
            "mount path is not a directory: {}\n\
             Loaf can only mount on directories",
            path.display()
        );
    }

    let path = path
        .canonicalize()
        .wrap_err_with(|| format!("failed to canonicalize mount path {}", path.display()))?;

    // Create overlay database
    let overlay_path = path.join(".loaf");
    if overlay_path.exists() {
        color_eyre::eyre::bail!(
            "overlay already exists at {}\n\
             Either:\n\
             - Unmount first with: loaf unmount {}\n\
             - Delete existing overlay with: loaf reject {}\n\
             - Choose a different directory",
            overlay_path.display(),
            path.display(),
            path.display()
        );
    }

    tracing::info!("creating overlay database at {}", overlay_path.display());
    let overlay = overlay::OverlayFs::new(&overlay_path, &path)
        .wrap_err_with(|| format!("failed to create overlay at {}", overlay_path.display()))?;

    // Start NFS server
    tracing::info!("starting NFS server");
    let (server, server_task): (nfs::NfsServer, tokio::task::JoinHandle<()>) =
        nfs::NfsServer::start(overlay, port)
            .await
            .wrap_err("failed to start NFS server")?;

    tracing::info!("NFS server listening on port {}", server.port);

    // Mount via mount_nfs
    tracing::info!("mounting NFS filesystem at {}", path.display());
    nfs::mount_nfs(server.port, &path)
        .await
        .wrap_err_with(|| format!("failed to mount NFS at {}", path.display()))?;

    // Save mount state
    let state_path = overlay_path.with_extension("loaf.state");
    let state = MountState {
        port: server.port,
        overlay_path: overlay_path.clone(),
    };
    let state_json =
        serde_json::to_string_pretty(&state).wrap_err("failed to serialize mount state")?;
    std::fs::write(&state_path, state_json)
        .wrap_err_with(|| format!("failed to write mount state to {}", state_path.display()))?;

    // Register mount path for panic hook cleanup
    set_active_mount(Some(path.clone()));

    println!("✓ Overlay mounted at {}", path.display());
    println!("  NFS server running on port {}", server.port);
    println!("  Overlay database: {}", overlay_path.display());
    println!("\nPress Ctrl+C to unmount and stop the server");

    // Set up signal handlers for graceful shutdown
    let mut sigterm =
        signal(SignalKind::terminate()).wrap_err("failed to register SIGTERM handler")?;
    let mut sighup = signal(SignalKind::hangup()).wrap_err("failed to register SIGHUP handler")?;

    // Wait for any termination signal
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("received SIGINT (Ctrl+C), unmounting...");
        }
        _ = sigterm.recv() => {
            tracing::info!("received SIGTERM, unmounting...");
        }
        _ = sighup.recv() => {
            tracing::info!("received SIGHUP (terminal closed), unmounting...");
        }
    }

    // Clear active mount before cleanup (panic hook no longer needed)
    set_active_mount(None);

    nfs::unmount_nfs(&path)
        .await
        .wrap_err("failed to unmount")?;

    // Abort the server task since we're done
    server_task.abort();
    drop(server_task.await); // Ignore JoinError from abort

    Ok(())
}

async fn unmount_command(path: PathBuf) -> color_eyre::Result<()> {
    if !path.exists() {
        color_eyre::eyre::bail!(
            "path does not exist: {}\n\
             Check that the path is correct",
            path.display()
        );
    }

    let path = path
        .canonicalize()
        .wrap_err_with(|| format!("failed to canonicalize path {}", path.display()))?;

    tracing::info!("unmounting NFS filesystem at {}", path.display());
    nfs::unmount_nfs(&path)
        .await
        .wrap_err_with(|| format!("failed to unmount {}", path.display()))?;

    // Clean up state file
    let overlay_path = path.join(".loaf");
    let state_path = overlay_path.with_extension("loaf.state");
    if state_path.exists() {
        std::fs::remove_file(&state_path)
            .wrap_err_with(|| format!("failed to remove state file {}", state_path.display()))?;
    }

    println!("✓ Unmounted {}", path.display());
    println!("  Overlay database preserved at {}", overlay_path.display());
    println!("  Use 'loaf diff' to view changes or 'loaf accept' to apply them");

    Ok(())
}

#[expect(
    unsafe_code,
    reason = "pre_exec runs after fork in single-threaded child; sandbox is applied before exec"
)]
async fn run_command(
    command: String,
    args: Vec<String>,
    no_sandbox: bool,
) -> color_eyre::Result<()> {
    use std::io::Write as _;
    use std::os::unix::process::CommandExt as _;

    // Validate command exists
    if which::which(&command).is_err() {
        color_eyre::eyre::bail!(
            "command not found: {command}\n\
             Make sure the command is installed and available in PATH"
        );
    }

    // Get current working directory as base path
    let base_path = std::env::current_dir().wrap_err("failed to get current working directory")?;

    // Create temporary overlay database
    let temp_dir = tempfile::tempdir().wrap_err("failed to create temporary directory")?;
    let overlay_path = temp_dir.path().join("overlay.loaf");

    tracing::info!("creating temporary overlay at {}", overlay_path.display());
    let overlay = overlay::OverlayFs::new(&overlay_path, &base_path)
        .wrap_err_with(|| format!("failed to create overlay at {}", overlay_path.display()))?;

    // Start NFS server on random port
    tracing::info!("starting NFS server");
    let (server, server_task) = nfs::NfsServer::start(overlay, None)
        .await
        .wrap_err("failed to start NFS server")?;

    tracing::info!("NFS server listening on port {}", server.port);

    // Create temporary mount point
    let mount_dir = temp_dir.path().join("mount");
    tokio::fs::create_dir(&mount_dir)
        .await
        .wrap_err_with(|| format!("failed to create mount point {}", mount_dir.display()))?;
    let mount_dir = mount_dir
        .canonicalize()
        .wrap_err("failed to canonicalize mount directory")?;

    // Mount overlay
    tracing::info!("mounting NFS filesystem at {}", mount_dir.display());
    nfs::mount_nfs(server.port, &mount_dir)
        .await
        .wrap_err_with(|| format!("failed to mount NFS at {}", mount_dir.display()))?;

    println!("✓ Overlay mounted at {}", mount_dir.display());
    if !no_sandbox {
        println!("  Sandbox: enabled (project dir protected)");
    }
    println!("  Running: {command} {}", args.join(" "));
    println!();

    // Run the command with cwd set to mount point
    // Use std::process::Command for pre_exec sandbox support
    let status = {
        let mut cmd = std::process::Command::new(&command);
        cmd.args(&args).current_dir(&mount_dir);

        if !no_sandbox {
            // Generate sandbox profile - protect base_path from direct writes
            let mut profile = sandbox::generate_profile(&base_path);

            // Add debug logging if requested
            if std::env::var("LOAF_SANDBOX_DEBUG").is_ok() {
                profile = format!("(debug deny)\n{profile}");
                eprintln!("Sandbox debug mode enabled. View denied operations with:");
                eprintln!("  log stream --predicate 'process == \"sandboxd\"'");
            }

            // Closure that applies sandbox in child process
            let sandbox_fn = move || {
                // SAFETY: apply_sandbox is safe to call in child process after fork,
                // before exec. The sandbox cannot be removed once applied.
                unsafe { sandbox::apply_sandbox(&profile) }.map_err(std::io::Error::other)
            };

            // SAFETY: pre_exec runs after fork, before exec in single-threaded child.
            // The closure is safe because it only applies a macOS sandbox profile.
            unsafe {
                cmd.pre_exec(sandbox_fn);
            }
        }

        cmd.status()
            .wrap_err_with(|| format!("failed to execute command: {command}"))?
    };

    println!();
    if status.success() {
        println!("✓ Command completed successfully (exit code: 0)");
    } else {
        println!(
            "✗ Command failed (exit code: {})",
            status.code().unwrap_or(-1)
        );
    }

    // Unmount before showing diff
    tracing::info!("unmounting overlay");
    nfs::unmount_nfs(&mount_dir)
        .await
        .wrap_err_with(|| format!("failed to unmount {}", mount_dir.display()))?;

    // Abort server task (we're done with it)
    server_task.abort();
    drop(server_task.await); // Ignore abort error

    // Open overlay to check for changes
    let mut reopened_overlay =
        overlay::OverlayFs::new(&overlay_path, &base_path).wrap_err("failed to reopen overlay")?;

    let changes =
        get_overlay_changes(&reopened_overlay).wrap_err("failed to compute overlay changes")?;

    if changes.is_empty() {
        println!("\nNo changes detected in overlay.");
        return Ok(());
    }

    // Display changes
    println!("\nChanges detected:");
    for change in &changes {
        println!("  {change}");
    }

    // Prompt user to accept or reject
    println!();
    print!("Apply changes to real filesystem? [y/N]: ");
    std::io::stdout().flush()?;

    let mut response = String::new();
    std::io::stdin()
        .read_line(&mut response)
        .wrap_err("failed to read user input")?;

    let response = response.trim().to_lowercase();
    if response == "y" || response == "yes" {
        println!("\nApplying changes...");
        apply_overlay_changes(&mut reopened_overlay).wrap_err("failed to apply overlay changes")?;
        println!("✓ Changes applied successfully");
    } else {
        println!("\nChanges discarded.");
    }

    Ok(())
}

fn find_overlay_path(overlay_arg: Option<PathBuf>) -> color_eyre::Result<PathBuf> {
    use color_eyre::eyre::WrapErr as _;

    if let Some(path) = overlay_arg {
        if !path.exists() {
            color_eyre::eyre::bail!("overlay database not found at {}", path.display());
        }
        return Ok(path);
    }

    let mut current = std::env::current_dir().wrap_err("failed to get current directory")?;

    loop {
        let candidate = current.join(".loaf");
        if candidate.exists() {
            return Ok(candidate);
        }

        if !current.pop() {
            color_eyre::eyre::bail!(
                "no .loaf file found in current directory or any parent directory\n\
                 Specify path explicitly with --overlay or create one with 'loaf mount'"
            );
        }
    }
}

fn diff_command(overlay_arg: Option<PathBuf>) -> color_eyre::Result<()> {
    use color_eyre::eyre::WrapErr as _;

    let overlay_path = find_overlay_path(overlay_arg)?;

    let base_path = {
        let temp_db =
            crate::db::Database::open(&overlay_path).wrap_err("failed to open overlay database")?;
        let base_path_str = temp_db
            .get_base_path()
            .wrap_err("failed to get base path from overlay")?;
        PathBuf::from(base_path_str)
    };

    let overlay =
        overlay::OverlayFs::new(&overlay_path, &base_path).wrap_err("failed to open overlay")?;

    let changes = get_overlay_changes(&overlay).wrap_err("failed to compute changes")?;

    if changes.is_empty() {
        println!("No changes in overlay.");
        return Ok(());
    }

    println!(
        "Changes in overlay (relative to {}):\n",
        base_path.display()
    );
    for change in changes {
        println!("{change}");
    }

    Ok(())
}

fn accept_command(overlay_arg: Option<PathBuf>) -> color_eyre::Result<()> {
    use color_eyre::eyre::WrapErr as _;
    use std::io::Write as _;

    let overlay_path = find_overlay_path(overlay_arg)?;

    let base_path = {
        let temp_db =
            crate::db::Database::open(&overlay_path).wrap_err("failed to open overlay database")?;
        let base_path_str = temp_db
            .get_base_path()
            .wrap_err("failed to get base path from overlay")?;
        PathBuf::from(base_path_str)
    };

    let mut overlay =
        overlay::OverlayFs::new(&overlay_path, &base_path).wrap_err("failed to open overlay")?;

    let changes = get_overlay_changes(&overlay).wrap_err("failed to compute changes")?;

    if changes.is_empty() {
        println!("No changes to apply.");
        return Ok(());
    }

    println!("Changes to apply:\n");
    for change in &changes {
        println!("{change}");
    }

    println!();
    print!("Apply these changes to {}? [y/N]: ", base_path.display());
    std::io::stdout().flush()?;

    let mut response = String::new();
    std::io::stdin()
        .read_line(&mut response)
        .wrap_err("failed to read user input")?;

    let response = response.trim().to_lowercase();
    if response != "y" && response != "yes" {
        println!("Cancelled.");
        return Ok(());
    }

    println!("\nApplying changes...");
    apply_overlay_changes(&mut overlay).wrap_err("failed to apply changes")?;

    println!("✓ Changes applied successfully");
    println!("\nOverlay database preserved at {}", overlay_path.display());
    println!("You can delete it with 'loaf reject' or keep it for reference");

    Ok(())
}

fn reject_command(overlay_arg: Option<PathBuf>) -> color_eyre::Result<()> {
    use color_eyre::eyre::WrapErr as _;
    use std::io::Write as _;

    let overlay_path = find_overlay_path(overlay_arg)?;

    println!(
        "This will delete the overlay database at {}",
        overlay_path.display()
    );
    println!("All changes will be permanently lost.");
    print!("Continue? [y/N]: ");
    std::io::stdout().flush()?;

    let mut response = String::new();
    std::io::stdin()
        .read_line(&mut response)
        .wrap_err("failed to read user input")?;

    let response = response.trim().to_lowercase();
    if response != "y" && response != "yes" {
        println!("Cancelled.");
        return Ok(());
    }

    std::fs::remove_file(&overlay_path).wrap_err_with(|| {
        format!(
            "failed to delete overlay database at {}",
            overlay_path.display()
        )
    })?;

    let state_path = overlay_path.with_extension("loaf.state");
    if state_path.exists() {
        std::fs::remove_file(&state_path).ok();
    }

    println!("✓ Overlay database deleted");

    Ok(())
}

/// Compute list of changes from overlay database
fn get_overlay_changes(overlay: &overlay::OverlayFs) -> color_eyre::Result<Vec<String>> {
    use color_eyre::eyre::WrapErr as _;

    let mut changes = Vec::new();

    let inodes = overlay
        .get_all_inodes()
        .wrap_err("failed to get all inodes")?;

    for (path, item_type) in inodes {
        // Skip .git directory - git internals are noisy and not user-relevant
        if path.starts_with("/.git/") || path == "/.git" {
            continue;
        }
        let real_path = overlay
            .base_path()
            .join(path.strip_prefix('/').unwrap_or(&path));

        let type_str = match item_type {
            crate::db::ItemType::File => "file",
            crate::db::ItemType::Directory => "dir",
            crate::db::ItemType::Symlink => "symlink",
        };

        if real_path.exists() {
            changes.push(format!("  \x1b[33mM\x1b[0m {type_str:8} {path}"));
        } else {
            changes.push(format!("  \x1b[32mA\x1b[0m {type_str:8} {path}"));
        }
    }

    let whiteouts = overlay
        .get_all_whiteouts()
        .wrap_err("failed to get all whiteouts")?;

    for path in whiteouts {
        // Skip .git directory
        if path.starts_with("/.git/") || path == "/.git" {
            continue;
        }
        changes.push(format!("  \x1b[31mD\x1b[0m          {path}"));
    }

    changes.sort();
    Ok(changes)
}

/// Apply overlay changes to real filesystem
fn apply_overlay_changes(overlay: &mut overlay::OverlayFs) -> color_eyre::Result<()> {
    use color_eyre::eyre::WrapErr as _;
    use std::os::unix::fs::PermissionsExt as _;

    let base_path = overlay.base_path().to_path_buf();
    let inodes = overlay
        .get_all_inodes()
        .wrap_err("failed to get all inodes")?;

    for (path, item_type) in inodes {
        let real_path = base_path.join(path.strip_prefix('/').unwrap_or(&path));

        match item_type {
            crate::db::ItemType::Directory => {
                std::fs::create_dir_all(&real_path).wrap_err_with(|| {
                    format!("failed to create directory {}", real_path.display())
                })?;
            }
            crate::db::ItemType::File => {
                if let Some(parent) = real_path.parent() {
                    std::fs::create_dir_all(parent).wrap_err_with(|| {
                        format!("failed to create parent directory {}", parent.display())
                    })?;
                }

                let data = overlay
                    .read_file_data(&path)
                    .wrap_err_with(|| format!("failed to read file data for {path}"))?;

                std::fs::write(&real_path, &data)
                    .wrap_err_with(|| format!("failed to write file {}", real_path.display()))?;

                let root_id = overlay::OverlayFs::ROOT_ID;
                let inode = overlay
                    .lookup(root_id, path.strip_prefix('/').unwrap_or(&path))
                    .wrap_err_with(|| format!("failed to lookup {path}"))?;
                let attrs = overlay
                    .getattr(inode)
                    .wrap_err_with(|| format!("failed to get attrs for {path}"))?;

                let perms = std::fs::Permissions::from_mode(attrs.mode);
                std::fs::set_permissions(&real_path, perms).wrap_err_with(|| {
                    format!("failed to set permissions for {}", real_path.display())
                })?;
            }
            crate::db::ItemType::Symlink => {
                if let Some(parent) = real_path.parent() {
                    std::fs::create_dir_all(parent).wrap_err_with(|| {
                        format!("failed to create parent directory {}", parent.display())
                    })?;
                }

                let root_id = overlay::OverlayFs::ROOT_ID;
                let inode = overlay
                    .lookup(root_id, path.strip_prefix('/').unwrap_or(&path))
                    .wrap_err_with(|| format!("failed to lookup symlink {path}"))?;
                let target = overlay
                    .readlink(inode)
                    .wrap_err_with(|| format!("failed to read symlink target for {path}"))?;

                if real_path.exists() || real_path.is_symlink() {
                    std::fs::remove_file(&real_path).ok();
                }

                std::os::unix::fs::symlink(&target, &real_path).wrap_err_with(|| {
                    format!(
                        "failed to create symlink {} -> {target}",
                        real_path.display()
                    )
                })?;
            }
        }
    }

    let whiteouts = overlay
        .get_all_whiteouts()
        .wrap_err("failed to get all whiteouts")?;

    for path in whiteouts {
        let real_path = base_path.join(path.strip_prefix('/').unwrap_or(&path));

        if real_path.exists() {
            if real_path.is_dir() {
                std::fs::remove_dir_all(&real_path).wrap_err_with(|| {
                    format!("failed to remove directory {}", real_path.display())
                })?;
            } else {
                std::fs::remove_file(&real_path)
                    .wrap_err_with(|| format!("failed to remove file {}", real_path.display()))?;
            }
        }
    }

    Ok(())
}
