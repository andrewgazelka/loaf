mod db;
mod nfs;
mod overlay;
mod sandbox;

use color_eyre::eyre::WrapErr as _;
use std::path::PathBuf;

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
    color_eyre::install()?;

    let cli = <Cli as clap::Parser>::parse();

    // Set up logging - always write to file, optionally to terminal with --verbose
    let log_file_path = cli.log_file.clone();
    let log_file = std::fs::File::create(&log_file_path)
        .wrap_err_with(|| format!("failed to create log file at {:?}", log_file_path))?;

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(log_file)
        .with_ansi(false);

    let env_filter = if cli.verbose {
        tracing_subscriber::EnvFilter::new("debug")
    } else {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };

    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

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

    match cli.command {
        Commands::Mount { path, port } => mount_command(path, port).await?,
        Commands::Unmount { path } => unmount_command(path).await?,
        Commands::Run {
            command,
            args,
            no_sandbox,
        } => run_command(command, args, no_sandbox).await?,
        Commands::Diff { overlay } => diff_command(overlay).await?,
        Commands::Accept { overlay } => accept_command(overlay).await?,
        Commands::Reject { overlay } => reject_command(overlay).await?,
    }

    Ok(())
}

/// State file for tracking active mounts
#[derive(serde::Serialize, serde::Deserialize)]
struct MountState {
    port: u16,
    overlay_path: PathBuf,
}

async fn mount_command(path: PathBuf, port: Option<u16>) -> color_eyre::Result<()> {
    // Validate path exists
    if !path.exists() {
        color_eyre::eyre::bail!(
            "mount path does not exist: {path:?}\n\
             Create the directory first with: mkdir -p {path:?}"
        );
    }

    if !path.is_dir() {
        color_eyre::eyre::bail!(
            "mount path is not a directory: {path:?}\n\
             Loaf can only mount on directories"
        );
    }

    let path = path
        .canonicalize()
        .wrap_err_with(|| format!("failed to canonicalize mount path {path:?}"))?;

    // Create overlay database
    let overlay_path = path.join(".loaf");
    if overlay_path.exists() {
        color_eyre::eyre::bail!(
            "overlay already exists at {overlay_path:?}\n\
             Either:\n\
             - Unmount first with: loaf unmount {path:?}\n\
             - Delete existing overlay with: loaf reject {path:?}\n\
             - Choose a different directory"
        );
    }

    tracing::info!("creating overlay database at {overlay_path:?}");
    let overlay = overlay::OverlayFs::new(&overlay_path, &path)
        .wrap_err_with(|| format!("failed to create overlay at {overlay_path:?}"))?;

    // Start NFS server
    tracing::info!("starting NFS server");
    let (server, server_task): (nfs::NfsServer, tokio::task::JoinHandle<()>) =
        nfs::NfsServer::start(overlay, port)
            .await
            .wrap_err("failed to start NFS server")?;

    tracing::info!("NFS server listening on port {}", server.port);

    // Mount via mount_nfs
    tracing::info!("mounting NFS filesystem at {path:?}");
    nfs::mount_nfs(server.port, &path)
        .await
        .wrap_err_with(|| format!("failed to mount NFS at {path:?}"))?;

    // Save mount state
    let state_path = overlay_path.with_extension("loaf.state");
    let state = MountState {
        port: server.port,
        overlay_path: overlay_path.clone(),
    };
    let state_json =
        serde_json::to_string_pretty(&state).wrap_err("failed to serialize mount state")?;
    std::fs::write(&state_path, state_json)
        .wrap_err_with(|| format!("failed to write mount state to {state_path:?}"))?;

    println!("✓ Overlay mounted at {path:?}");
    println!("  NFS server running on port {}", server.port);
    println!("  Overlay database: {overlay_path:?}");
    println!("\nPress Ctrl+C to unmount and stop the server");

    // Install signal handler for graceful shutdown
    let path_for_signal = path.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("received Ctrl+C, unmounting...");
        if let Err(e) = nfs::unmount_nfs(&path_for_signal).await {
            tracing::error!("failed to unmount: {}", e);
        }
        std::process::exit(0);
    });

    // Wait for server task (runs until killed)
    server_task.await.wrap_err("NFS server task failed")?;

    Ok(())
}

async fn unmount_command(path: PathBuf) -> color_eyre::Result<()> {
    if !path.exists() {
        color_eyre::eyre::bail!(
            "path does not exist: {path:?}\n\
             Check that the path is correct"
        );
    }

    let path = path
        .canonicalize()
        .wrap_err_with(|| format!("failed to canonicalize path {path:?}"))?;

    tracing::info!("unmounting NFS filesystem at {path:?}");
    nfs::unmount_nfs(&path)
        .await
        .wrap_err_with(|| format!("failed to unmount {path:?}"))?;

    // Clean up state file
    let overlay_path = path.join(".loaf");
    let state_path = overlay_path.with_extension("loaf.state");
    if state_path.exists() {
        std::fs::remove_file(&state_path)
            .wrap_err_with(|| format!("failed to remove state file {state_path:?}"))?;
    }

    println!("✓ Unmounted {path:?}");
    println!("  Overlay database preserved at {overlay_path:?}");
    println!("  Use 'loaf diff' to view changes or 'loaf accept' to apply them");

    Ok(())
}

async fn run_command(
    command: String,
    args: Vec<String>,
    no_sandbox: bool,
) -> color_eyre::Result<()> {
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

    tracing::info!("creating temporary overlay at {overlay_path:?}");
    let overlay = overlay::OverlayFs::new(&overlay_path, &base_path)
        .wrap_err_with(|| format!("failed to create overlay at {overlay_path:?}"))?;

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
        .wrap_err_with(|| format!("failed to create mount point {mount_dir:?}"))?;
    let mount_dir = mount_dir
        .canonicalize()
        .wrap_err("failed to canonicalize mount directory")?;

    // Mount overlay
    tracing::info!("mounting NFS filesystem at {mount_dir:?}");
    nfs::mount_nfs(server.port, &mount_dir)
        .await
        .wrap_err_with(|| format!("failed to mount NFS at {mount_dir:?}"))?;

    println!("✓ Overlay mounted at {mount_dir:?}");
    if !no_sandbox {
        println!("  Sandbox: enabled (writes restricted to overlay)");
    }
    println!("  Running: {command} {}", args.join(" "));
    println!();

    // Run the command with cwd set to mount point
    // Use std::process::Command for pre_exec sandbox support
    let status = {
        use std::os::unix::process::CommandExt as _;

        let mut cmd = std::process::Command::new(&command);
        cmd.args(&args).current_dir(&mount_dir);

        if !no_sandbox {
            // Generate sandbox profile
            let mut profile = sandbox::generate_profile(&mount_dir);

            // Add debug logging if requested
            if std::env::var("LOAF_SANDBOX_DEBUG").is_ok() {
                profile = format!("(debug deny)\n{profile}");
                eprintln!("Sandbox debug mode enabled. View denied operations with:");
                eprintln!("  log stream --predicate 'process == \"sandboxd\"'");
            }

            // SAFETY: pre_exec runs after fork, before exec in single-threaded child
            unsafe {
                cmd.pre_exec(move || {
                    sandbox::apply_sandbox(&profile)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                });
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
        .wrap_err_with(|| format!("failed to unmount {mount_dir:?}"))?;

    // Abort server task (we're done with it)
    server_task.abort();
    let _ = server_task.await; // Ignore abort error

    // Open overlay to check for changes
    let mut overlay =
        overlay::OverlayFs::new(&overlay_path, &base_path).wrap_err("failed to reopen overlay")?;

    let changes = get_overlay_changes(&overlay).wrap_err("failed to compute overlay changes")?;

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
    use std::io::Write as _;
    std::io::stdout().flush()?;

    let mut response = String::new();
    std::io::stdin()
        .read_line(&mut response)
        .wrap_err("failed to read user input")?;

    let response = response.trim().to_lowercase();
    if response == "y" || response == "yes" {
        println!("\nApplying changes...");
        apply_overlay_changes(&mut overlay).wrap_err("failed to apply overlay changes")?;
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
            color_eyre::eyre::bail!("overlay database not found at {path:?}");
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

async fn diff_command(overlay_arg: Option<PathBuf>) -> color_eyre::Result<()> {
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

    println!("Changes in overlay (relative to {base_path:?}):\n");
    for change in changes {
        println!("{change}");
    }

    Ok(())
}

async fn accept_command(overlay_arg: Option<PathBuf>) -> color_eyre::Result<()> {
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
    print!("Apply these changes to {base_path:?}? [y/N]: ");
    use std::io::Write as _;
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
    println!("\nOverlay database preserved at {overlay_path:?}");
    println!("You can delete it with 'loaf reject' or keep it for reference");

    Ok(())
}

async fn reject_command(overlay_arg: Option<PathBuf>) -> color_eyre::Result<()> {
    use color_eyre::eyre::WrapErr as _;

    let overlay_path = find_overlay_path(overlay_arg)?;

    println!("This will delete the overlay database at {overlay_path:?}");
    println!("All changes will be permanently lost.");
    print!("Continue? [y/N]: ");
    use std::io::Write as _;
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

    std::fs::remove_file(&overlay_path)
        .wrap_err_with(|| format!("failed to delete overlay database at {overlay_path:?}"))?;

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

    let base_path = overlay.base_path().to_path_buf();
    let inodes = overlay
        .get_all_inodes()
        .wrap_err("failed to get all inodes")?;

    for (path, item_type) in inodes {
        let real_path = base_path.join(path.strip_prefix('/').unwrap_or(&path));

        match item_type {
            crate::db::ItemType::Directory => {
                std::fs::create_dir_all(&real_path)
                    .wrap_err_with(|| format!("failed to create directory {real_path:?}"))?;
            }
            crate::db::ItemType::File => {
                if let Some(parent) = real_path.parent() {
                    std::fs::create_dir_all(parent).wrap_err_with(|| {
                        format!("failed to create parent directory {parent:?}")
                    })?;
                }

                let data = overlay
                    .read_file_data(&path)
                    .wrap_err_with(|| format!("failed to read file data for {path:?}"))?;

                std::fs::write(&real_path, &data)
                    .wrap_err_with(|| format!("failed to write file {real_path:?}"))?;

                let root_id = overlay.root_id();
                let inode = overlay
                    .lookup(root_id, path.strip_prefix('/').unwrap_or(&path))
                    .wrap_err_with(|| format!("failed to lookup {path:?}"))?;
                let attrs = overlay
                    .getattr(inode)
                    .wrap_err_with(|| format!("failed to get attrs for {path:?}"))?;

                use std::os::unix::fs::PermissionsExt as _;
                let perms = std::fs::Permissions::from_mode(attrs.mode);
                std::fs::set_permissions(&real_path, perms)
                    .wrap_err_with(|| format!("failed to set permissions for {real_path:?}"))?;
            }
            crate::db::ItemType::Symlink => {
                if let Some(parent) = real_path.parent() {
                    std::fs::create_dir_all(parent).wrap_err_with(|| {
                        format!("failed to create parent directory {parent:?}")
                    })?;
                }

                let root_id = overlay.root_id();
                let inode = overlay
                    .lookup(root_id, path.strip_prefix('/').unwrap_or(&path))
                    .wrap_err_with(|| format!("failed to lookup symlink {path:?}"))?;
                let target = overlay
                    .readlink(inode)
                    .wrap_err_with(|| format!("failed to read symlink target for {path:?}"))?;

                if real_path.exists() || real_path.is_symlink() {
                    std::fs::remove_file(&real_path).ok();
                }

                std::os::unix::fs::symlink(&target, &real_path).wrap_err_with(|| {
                    format!("failed to create symlink {real_path:?} -> {target:?}")
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
                std::fs::remove_dir_all(&real_path)
                    .wrap_err_with(|| format!("failed to remove directory {real_path:?}"))?;
            } else {
                std::fs::remove_file(&real_path)
                    .wrap_err_with(|| format!("failed to remove file {real_path:?}"))?;
            }
        }
    }

    Ok(())
}
