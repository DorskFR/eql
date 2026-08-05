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
    #[serde(default)]
    pub tools: ToolsConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolsConfig {
    #[serde(default)]
    pub log_reader: LogReaderConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LogReaderConfig {
    #[serde(default)]
    pub enabled: bool,
    pub exe: Option<PathBuf>,
    pub version: Option<String>,
    #[serde(default = "default_replay_secs")]
    pub replay_secs: u64,
    #[serde(default = "default_replay_timeout_secs")]
    pub replay_timeout_secs: u64,
    #[serde(default)]
    pub overlays: Vec<String>,
}

impl LogReaderConfig {
    pub fn version(&self) -> &str {
        self.version
            .as_deref()
            .unwrap_or(crate::tools::DEFAULT_VERSION)
    }

    pub fn replay_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.replay_secs.max(10))
    }

    pub fn replay_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.replay_timeout_secs.max(10))
    }
}

fn default_replay_secs() -> u64 {
    120
}

fn default_replay_timeout_secs() -> u64 {
    600
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
        if !self.harvest.enabled && !self.tools.log_reader.enabled {
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
    fn enabling_the_log_reader_switches_harvest_on_without_a_harvest_section() {
        let config: Config = toml::from_str(
            r#"
            [game]
            root = "/games/eq"
            [api]
            url = "u"
            token = "t"
            [tools.log_reader]
            enabled = true
            "#,
        )
        .unwrap();
        assert!(config.tools.log_reader.enabled);
        assert_eq!(config.harvest_dir(), crate::harvest::default_dir());
        assert_eq!(
            config.tools.log_reader.version(),
            crate::tools::DEFAULT_VERSION
        );
        assert_eq!(config.tools.log_reader.replay_interval().as_secs(), 120);
        assert_eq!(config.tools.log_reader.replay_timeout().as_secs(), 600);
    }

    #[test]
    fn log_reader_overrides_are_read_and_clamped() {
        let config: Config = toml::from_str(
            r#"
            [game]
            root = "/games/eq"
            [api]
            url = "u"
            token = "t"
            [tools.log_reader]
            enabled = true
            exe = "/opt/eql/eql_atlas.exe"
            version = "v2.0"
            replay_secs = 0
            replay_timeout_secs = 1
            "#,
        )
        .unwrap();
        assert_eq!(
            config.tools.log_reader.exe,
            Some(PathBuf::from("/opt/eql/eql_atlas.exe"))
        );
        assert_eq!(config.tools.log_reader.version(), "v2.0");
        assert_eq!(config.tools.log_reader.replay_interval().as_secs(), 10);
        assert_eq!(config.tools.log_reader.replay_timeout().as_secs(), 10);
    }

    #[test]
    fn the_log_reader_stays_off_by_default() {
        let config: Config = toml::from_str(
            r#"
            [game]
            root = "/games/eq"
            [api]
            url = "u"
            token = "t"
            "#,
        )
        .unwrap();
        assert!(!config.tools.log_reader.enabled);
        assert_eq!(config.harvest_dir(), None);
    }

    #[test]
    fn no_overlays_are_configured_by_default() {
        let config: Config = toml::from_str(
            r#"
            [game]
            root = "/games/eq"
            [api]
            url = "u"
            token = "t"
            [tools.log_reader]
            enabled = true
            "#,
        )
        .unwrap();
        assert!(config.tools.log_reader.overlays.is_empty());
    }

    #[test]
    fn overlays_are_read_in_the_order_they_are_listed() {
        let config: Config = toml::from_str(
            r#"
            [game]
            root = "/games/eq"
            [api]
            url = "u"
            token = "t"
            [tools.log_reader]
            enabled = true
            overlays = ["dps", "session_report"]
            "#,
        )
        .unwrap();
        assert_eq!(config.tools.log_reader.overlays, ["dps", "session_report"]);
        assert_eq!(
            crate::overlays::plan(&config.tools.log_reader.overlays, true).0,
            vec![
                crate::overlays::Overlay::Dps,
                crate::overlays::Overlay::SessionReport
            ]
        );
    }

    #[test]
    fn overlays_alone_do_not_switch_the_replay_harvest_on() {
        let config: Config = toml::from_str(
            r#"
            [game]
            root = "/games/eq"
            [api]
            url = "u"
            token = "t"
            [tools.log_reader]
            overlays = ["dps"]
            "#,
        )
        .unwrap();
        assert!(!config.tools.log_reader.enabled);
        assert_eq!(config.harvest_dir(), None);
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
