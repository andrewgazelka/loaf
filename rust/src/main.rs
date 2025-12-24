mod db;
mod nfs;
mod overlay;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tracing::info!("loaf starting");

    // TODO: CLI parsing and NFS server startup

    Ok(())
}
