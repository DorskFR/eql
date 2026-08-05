use eqld::{config::Config, daemon::Daemon};
use std::path::PathBuf;

fn config_path() -> PathBuf {
    std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("eqld.toml"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let path = config_path();
    let config = Config::load(&path)?;
    let mut daemon = Daemon::new(config)?;

    tracing::info!(
        root = %daemon.config().game.root.display(),
        api = %daemon.config().api.url,
        poll_secs = daemon.config().poll_interval().as_secs(),
        state = %daemon.state_path().display(),
        "eqld starting"
    );

    loop {
        daemon.tick().await;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutdown requested");
                break;
            }
            _ = tokio::time::sleep(daemon.delay()) => {}
        }
    }
    Ok(())
}
