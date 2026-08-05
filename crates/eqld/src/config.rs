use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub game: GameConfig,
    pub api: ApiConfig,
    #[serde(default)]
    pub state: StateConfig,
    #[serde(default)]
    pub harvest: HarvestConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GameConfig {
    /// Same tree on Windows and inside the osxEQL Wine prefix.
    pub root: PathBuf,
    #[serde(default = "default_poll_secs")]
    pub poll_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiConfig {
    pub url: String,
    pub token: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StateConfig {
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HarvestConfig {
    #[serde(default)]
    pub enabled: bool,
    pub dir: Option<PathBuf>,
}

fn default_poll_secs() -> u64 {
    5
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)
            .map_err(|source| ConfigError::Read(path.to_path_buf(), source))?;
        toml::from_str(&text).map_err(|source| ConfigError::Parse(path.to_path_buf(), source))
    }

    pub fn poll_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.game.poll_secs.max(1))
    }

    pub fn state_path(&self) -> PathBuf {
        self.state.path.clone().unwrap_or_else(default_state_path)
    }

    pub fn endpoint(&self) -> String {
        self.api_url("inventory")
    }

    pub fn events_endpoint(&self) -> String {
        self.api_url("events")
    }

    pub fn harvest_endpoint(&self) -> String {
        self.api_url("harvest")
    }

    pub fn harvest_dir(&self) -> Option<PathBuf> {
        if !self.harvest.enabled {
            return None;
        }
        self.harvest
            .dir
            .clone()
            .or_else(crate::harvest::default_dir)
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/api/v1/{path}", self.api.url.trim_end_matches('/'))
    }
}

pub fn default_state_path() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|dir| dir.join("eqld").join("state.json"))
        .unwrap_or_else(|| PathBuf::from("eqld-state.json"))
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading {0}: {1}")]
    Read(PathBuf, #[source] std::io::Error),
    #[error("parsing {0}: {1}")]
    Parse(PathBuf, #[source] toml::de::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_defaults() {
        let config: Config = toml::from_str(
            r#"
            [game]
            root = "/games/eq"
            [api]
            url = "https://eql.example.com"
            token = "secret"
            "#,
        )
        .unwrap();
        assert_eq!(config.game.poll_secs, 5);
        assert_eq!(config.poll_interval().as_secs(), 5);
        assert!(config.state.path.is_none());
        assert!(!config.harvest.enabled);
        assert_eq!(config.harvest_dir(), None);
        assert_eq!(config.state_path(), default_state_path());
    }

    #[test]
    fn reads_explicit_values() {
        let config: Config = toml::from_str(
            r#"
            [game]
            root = "C:\\EQ"
            poll_secs = 30
            [api]
            url = "http://localhost:8080/"
            token = "t"
            [state]
            path = "state.json"
            "#,
        )
        .unwrap();
        assert_eq!(config.game.poll_secs, 30);
        assert_eq!(config.state_path(), PathBuf::from("state.json"));
        assert_eq!(config.endpoint(), "http://localhost:8080/api/v1/inventory");
        assert_eq!(
            config.events_endpoint(),
            "http://localhost:8080/api/v1/events"
        );
        assert_eq!(
            config.harvest_endpoint(),
            "http://localhost:8080/api/v1/harvest"
        );
    }

    #[test]
    fn harvest_stays_off_until_it_is_switched_on() {
        let with = |harvest: &str| -> Config {
            toml::from_str(&format!(
                r#"
                [game]
                root = "/games/eq"
                [api]
                url = "u"
                token = "t"
                {harvest}
                "#
            ))
            .unwrap()
        };
        assert_eq!(with("[harvest]\nenabled = false").harvest_dir(), None);
        assert_eq!(
            with("[harvest]\nenabled = true\ndir = \"/tmp/reader\"").harvest_dir(),
            Some(PathBuf::from("/tmp/reader"))
        );
        assert_eq!(
            with("[harvest]\nenabled = true").harvest_dir(),
            crate::harvest::default_dir()
        );
        assert_eq!(with("[harvest]\ndir = \"/tmp/reader\"").harvest_dir(), None);
    }

    #[test]
    fn clamps_zero_poll_to_one_second() {
        let config: Config = toml::from_str(
            r#"
            [game]
            root = "/games/eq"
            poll_secs = 0
            [api]
            url = "u"
            token = "t"
            "#,
        )
        .unwrap();
        assert_eq!(config.poll_interval().as_secs(), 1);
    }
}
