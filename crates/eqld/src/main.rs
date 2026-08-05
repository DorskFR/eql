use eqld::{config::Config, daemon::Daemon, install, skin};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

const SUBCOMMANDS: &[&str] = &["install-skin", "install-tools"];

/// `eqld [config.toml] [subcommand [args…]]` — the optional leading config path
/// is anything that is not a subcommand name.
fn split_args(args: Vec<String>) -> (PathBuf, Vec<String>) {
    let leading_config = args
        .first()
        .is_some_and(|first| !SUBCOMMANDS.contains(&first.as_str()));
    let (path, rest) = if leading_config {
        (PathBuf::from(&args[0]), &args[1..])
    } else {
        (PathBuf::from("eqld.toml"), &args[..])
    };
    (path, rest.to_vec())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let (path, rest) = split_args(std::env::args().skip(1).collect());
    let config = Config::load(&path)?;

    if let Some(subcommand) = rest.first() {
        return match subcommand.as_str() {
            "install-skin" => Ok(skin::run(&config, &rest[1..]).await?),
            "install-tools" => Ok(install::run(&config, &rest[1..]).await?),
            other => Err(format!("unknown subcommand {other:?}").into()),
        };
    }

    let mut daemon = Daemon::new(config)?;

    let harvest = daemon
        .config()
        .harvest_dir()
        .map_or_else(|| "disabled".into(), |dir| dir.display().to_string());
    tracing::info!(
        root = %daemon.config().game.root.display(),
        api = %daemon.config().api.url,
        poll_secs = daemon.config().poll_interval().as_secs(),
        state = %daemon.state_path().display(),
        state_exists = daemon.state_path().exists(),
        harvest = %harvest,
        overlays = ?daemon.overlays(),
        hidden = ?daemon.hidden_overlays(),
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
    daemon.shutdown().await;
    Ok(())
}
