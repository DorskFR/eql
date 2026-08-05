use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct Config {
    game: GameConfig,
    api: ApiConfig,
}

#[derive(Debug, Deserialize)]
struct GameConfig {
    /// Same tree on Windows and inside the osxEQL Wine prefix.
    root: PathBuf,
}

#[derive(Debug, Deserialize)]
struct ApiConfig {
    url: String,
    token: String,
}

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
    let config: Config = toml::from_str(&std::fs::read_to_string(&path)?)?;
    tracing::info!(?config.game.root, api = %config.api.url, "eqld starting");
    // TODO: watch <root>/*-Inventory.txt, tail Logs/eqlog_*.txt, upload.
    Ok(())
}
