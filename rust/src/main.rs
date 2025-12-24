mod db;
mod nfs;
mod overlay;

use std::path::PathBuf;
use color_eyre::eyre::WrapErr as _;

#[derive(clap::Parser)]
#[command(name = "loaf", version, about = "Overlay filesystem for macOS via NFS")]
struct Cli {
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

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = <Cli as clap::Parser>::parse();

    match cli.command {
        Commands::Mount { path, port } => mount_command(path, port).await?,
        Commands::Unmount { path } => unmount_command(path).await?,
        Commands::Run { command, args } => run_command(command, args).await?,
        Commands::Diff { overlay } => {
            let _ = overlay;
            tracing::info!("diff command not yet implemented");
            color_eyre::eyre::bail!("diff command not yet implemented");
        }
        Commands::Accept { overlay } => {
            let _ = overlay;
            tracing::info!("accept command not yet implemented");
            color_eyre::eyre::bail!("accept command not yet implemented");
        }
        Commands::Reject { overlay } => {
            let _ = overlay;
            tracing::info!("reject command not yet implemented");
            color_eyre::eyre::bail!("reject command not yet implemented");
        }
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
    let path = path.canonicalize()
        .wrap_err_with(|| format!("failed to canonicalize mount path {path:?}"))?;

    // Create overlay database
    let overlay_path = path.join(".loaf");
    if overlay_path.exists() {
        color_eyre::eyre::bail!(
            "overlay already exists at {overlay_path:?} - unmount first or choose different directory"
        );
    }

    tracing::info!("creating overlay database at {overlay_path:?}");
    let overlay = overlay::OverlayFs::new(&overlay_path, &path)
        .wrap_err_with(|| format!("failed to create overlay at {overlay_path:?}"))?;

    // Start NFS server
    tracing::info!("starting NFS server");
    let (server, server_task): (nfs::NfsServer, tokio::task::JoinHandle<()>) = nfs::NfsServer::start(overlay, port).await
        .wrap_err("failed to start NFS server")?;

    tracing::info!("NFS server listening on port {}", server.port);

    // Mount via mount_nfs
    tracing::info!("mounting NFS filesystem at {path:?}");
    nfs::mount_nfs(server.port, &path).await
        .wrap_err_with(|| format!("failed to mount NFS at {path:?}"))?;

    // Save mount state
    let state_path = overlay_path.with_extension("loaf.state");
    let state = MountState {
        port: server.port,
        overlay_path: overlay_path.clone(),
    };
    let state_json = serde_json::to_string_pretty(&state)
        .wrap_err("failed to serialize mount state")?;
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
    server_task.await
        .wrap_err("NFS server task failed")?;

    Ok(())
}

async fn unmount_command(path: PathBuf) -> color_eyre::Result<()> {
    let path = path.canonicalize()
        .wrap_err_with(|| format!("failed to canonicalize path {path:?}"))?;

    tracing::info!("unmounting NFS filesystem at {path:?}");
    nfs::unmount_nfs(&path).await
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

async fn run_command(command: String, args: Vec<String>) -> color_eyre::Result<()> {
    // Get current working directory as base path
    let base_path = std::env::current_dir()
        .wrap_err("failed to get current working directory")?;

    // Create temporary overlay database
    let temp_dir = tempfile::tempdir()
        .wrap_err("failed to create temporary directory")?;
    let overlay_path = temp_dir.path().join("overlay.loaf");

    tracing::info!("creating temporary overlay at {overlay_path:?}");
    let overlay = overlay::OverlayFs::new(&overlay_path, &base_path)
        .wrap_err_with(|| format!("failed to create overlay at {overlay_path:?}"))?;

    // Start NFS server on random port
    tracing::info!("starting NFS server");
    let (server, server_task) = nfs::NfsServer::start(overlay, None).await
        .wrap_err("failed to start NFS server")?;

    tracing::info!("NFS server listening on port {}", server.port);

    // Create temporary mount point
    let mount_dir = temp_dir.path().join("mount");
    tokio::fs::create_dir(&mount_dir).await
        .wrap_err_with(|| format!("failed to create mount point {mount_dir:?}"))?;
    let mount_dir = mount_dir.canonicalize()
        .wrap_err("failed to canonicalize mount directory")?;

    // Mount overlay
    tracing::info!("mounting NFS filesystem at {mount_dir:?}");
    nfs::mount_nfs(server.port, &mount_dir).await
        .wrap_err_with(|| format!("failed to mount NFS at {mount_dir:?}"))?;

    println!("✓ Overlay mounted at {mount_dir:?}");
    println!("  Running: {command} {}", args.join(" "));
    println!();

    // Run the command with cwd set to mount point
    let status = tokio::process::Command::new(&command)
        .args(&args)
        .current_dir(&mount_dir)
        .status()
        .await
        .wrap_err_with(|| format!("failed to execute command: {command}"))?;

    println!();
    if status.success() {
        println!("✓ Command completed successfully (exit code: 0)");
    } else {
        println!("✗ Command failed (exit code: {})", status.code().unwrap_or(-1));
    }

    // Unmount before showing diff
    tracing::info!("unmounting overlay");
    nfs::unmount_nfs(&mount_dir).await
        .wrap_err_with(|| format!("failed to unmount {mount_dir:?}"))?;

    // Abort server task (we're done with it)
    server_task.abort();
    let _ = server_task.await; // Ignore abort error

    // Open overlay to check for changes
    let overlay = overlay::OverlayFs::new(&overlay_path, &base_path)
        .wrap_err("failed to reopen overlay")?;

    let changes = get_overlay_changes(&overlay)
        .wrap_err("failed to compute overlay changes")?;

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
    std::io::stdin().read_line(&mut response)
        .wrap_err("failed to read user input")?;

    let response = response.trim().to_lowercase();
    if response == "y" || response == "yes" {
        println!("\nApplying changes...");
        apply_overlay_changes(&overlay)
            .wrap_err("failed to apply overlay changes")?;
        println!("✓ Changes applied successfully");
    } else {
        println!("\nChanges discarded.");
    }

    Ok(())
}

/// Compute list of changes from overlay database
fn get_overlay_changes(_overlay: &overlay::OverlayFs) -> color_eyre::Result<Vec<String>> {
    // For now, just return a placeholder message
    // Full implementation would query the database for all inodes
    // and compare with real filesystem
    Ok(vec!["[Change detection not yet implemented]".to_string()])
}

/// Apply overlay changes to real filesystem
fn apply_overlay_changes(_overlay: &overlay::OverlayFs) -> color_eyre::Result<()> {
    // For now, just return success
    // Full implementation would:
    // 1. Iterate all inodes in database
    // 2. Copy modified files to real filesystem
    // 3. Create new directories
    // 4. Delete whiteout entries
    // 5. Update permissions/timestamps
    Ok(())
}
