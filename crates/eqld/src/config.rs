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
    pub repo: Option<String>,
    pub version: Option<String>,
    #[serde(default = "default_replay_secs")]
    pub replay_secs: u64,
    #[serde(default = "default_replay_timeout_secs")]
    pub replay_timeout_secs: u64,
    #[serde(default)]
    pub overlays: Vec<String>,
    /// A subset of `overlays` that runs without a window: the headless build
    /// where one exists, an isolated Windows desktop otherwise.
    #[serde(default)]
    pub hidden: Vec<String>,
    #[serde(default)]
    pub atlas: AtlasMode,
}

/// Who keeps the Atlas database: the headless `--replay` tick, or a live Atlas
/// overlay. Only the overlay tracks quests, and only if a human curates them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AtlasMode {
    #[default]
    Replay,
    Overlay,
}

impl LogReaderConfig {
    pub fn replay_enabled(&self) -> bool {
        self.enabled && self.atlas == AtlasMode::Replay
    }

    pub fn repo(&self) -> &str {
        self.repo.as_deref().unwrap_or(crate::tools::DEFAULT_REPO)
    }

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

    pub fn fights_endpoint(&self) -> String {
        self.api_url("fights")
    }

    /// Ours, beside the state file: the log reader's own directory is scanned
    /// and shipped wholesale, and fights do not travel that way.
    pub fn fights_dir(&self) -> PathBuf {
        self.state_path()
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.join("fights"))
            .unwrap_or_else(|| PathBuf::from("fights"))
    }

    pub fn icon_sheet_endpoint(&self, sheet: u32) -> String {
        self.api_url(&format!("icons/sheets/{sheet}"))
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

    /// Fields a running daemon cannot swap under itself: the state file it
    /// holds open, the tree it walks, and the endpoint its client posts to.
    pub fn frozen_changes(&self, next: &Config) -> Vec<&'static str> {
        let mut frozen = Vec::new();
        if self.game.root != next.game.root {
            frozen.push("game.root");
        }
        if self.game.poll_secs != next.game.poll_secs {
            frozen.push("game.poll_secs");
        }
        if self.api.url != next.api.url {
            frozen.push("api.url");
        }
        if self.api.token != next.api.token {
            frozen.push("api.token");
        }
        if self.state.path != next.state.path {
            frozen.push("state.path");
        }
        frozen
    }

    /// Keeps every frozen field of `self`, takes the rest from `next`.
    pub fn hot_swap(&self, next: Config) -> Config {
        Config {
            game: self.game.clone(),
            api: self.api.clone(),
            state: self.state.clone(),
            harvest: next.harvest,
            tools: next.tools,
        }
    }
}

/// Polls one config file for edits. A file that cannot be read or parsed is
/// reported once and then stays quiet until its bytes change again, so the
/// daemon keeps running on the last config that worked.
pub struct Watch {
    path: PathBuf,
    seen: Option<String>,
}

impl Watch {
    pub fn new(path: PathBuf) -> Self {
        let seen = std::fs::read(&path)
            .ok()
            .map(|bytes| crate::daemon::bytes_hash(&bytes));
        Self { path, seen }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `None` while the file is byte-identical to the last one reported.
    pub fn poll(&mut self) -> Option<Result<Config, ConfigError>> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(source) => {
                let first = self.seen.take();
                return first.map(|_| Err(ConfigError::Read(self.path.clone(), source)));
            }
        };
        let digest = crate::daemon::bytes_hash(&bytes);
        if self.seen.as_deref() == Some(digest.as_str()) {
            return None;
        }
        self.seen = Some(digest);
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(err) => {
                return Some(Err(ConfigError::Read(
                    self.path.clone(),
                    std::io::Error::new(std::io::ErrorKind::InvalidData, err),
                )));
            }
        };
        Some(toml::from_str(&text).map_err(|source| ConfigError::Parse(self.path.clone(), source)))
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
        assert_eq!(
            config.icon_sheet_endpoint(379),
            "http://localhost:8080/api/v1/icons/sheets/379"
        );
        assert_eq!(
            config.fights_endpoint(),
            "http://localhost:8080/api/v1/fights"
        );
        assert_eq!(config.fights_dir(), PathBuf::from("fights"));
    }

    #[test]
    fn fights_are_staged_beside_the_state_file() {
        let config: Config = toml::from_str(
            r#"
            [game]
            root = "/games/eq"
            [api]
            url = "u"
            token = "t"
            [state]
            path = "/var/lib/eqld/state.json"
            "#,
        )
        .unwrap();
        assert_eq!(
            config.fights_dir(),
            PathBuf::from("/var/lib/eqld/fights"),
            "never the log reader's directory, which is uploaded wholesale"
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
        assert_eq!(config.tools.log_reader.repo(), crate::tools::DEFAULT_REPO);
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
            repo = "blastlaster/eql-log-reader"
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
        assert_eq!(
            config.tools.log_reader.repo(),
            "blastlaster/eql-log-reader",
            "a rig can still be pointed back at stock upstream"
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
            crate::overlays::plan(&config.tools.log_reader).wanted,
            vec![
                crate::overlays::Overlay::Dps,
                crate::overlays::Overlay::SessionReport
            ]
        );
    }

    #[test]
    fn the_atlas_is_kept_by_replay_unless_the_overlay_is_asked_for() {
        let with = |mode: &str| -> Config {
            toml::from_str(&format!(
                r#"
                [game]
                root = "/games/eq"
                [api]
                url = "u"
                token = "t"
                [tools.log_reader]
                enabled = true
                overlays = ["atlas"]
                {mode}
                "#
            ))
            .unwrap()
        };
        let default = with("");
        assert_eq!(default.tools.log_reader.atlas, AtlasMode::Replay);
        assert!(default.tools.log_reader.replay_enabled());

        let overlay = with(r#"atlas = "overlay""#);
        assert_eq!(overlay.tools.log_reader.atlas, AtlasMode::Overlay);
        assert!(!overlay.tools.log_reader.replay_enabled());
        assert!(
            overlay.harvest_dir().is_some(),
            "the files are still shipped"
        );
    }

    #[test]
    fn hidden_overlays_are_read_and_default_to_none() {
        let config: Config = toml::from_str(
            r#"
            [game]
            root = "/games/eq"
            [api]
            url = "u"
            token = "t"
            [tools.log_reader]
            enabled = true
            overlays = ["dps"]
            hidden = ["dps"]
            "#,
        )
        .unwrap();
        assert_eq!(config.tools.log_reader.hidden, ["dps"]);
        assert_eq!(
            crate::overlays::plan(&config.tools.log_reader).hidden,
            vec![crate::overlays::Overlay::Dps]
        );

        let bare: Config = toml::from_str(
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
        assert!(bare.tools.log_reader.hidden.is_empty());
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

    fn write(path: &Path, overlays: &str) {
        std::fs::write(
            path,
            format!(
                r#"
                [game]
                root = "/games/eq"
                [api]
                url = "u"
                token = "t"
                [tools.log_reader]
                enabled = true
                overlays = [{overlays}]
                "#
            ),
        )
        .unwrap();
    }

    #[test]
    fn a_config_that_has_not_changed_is_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eqld.toml");
        write(&path, "\"dps\"");
        let mut watch = Watch::new(path.clone());
        assert!(watch.poll().is_none());

        write(&path, "\"dps\"");
        assert!(watch.poll().is_none(), "identical bytes are not a change");

        write(&path, "\"dps\", \"friend\"");
        let reloaded = watch.poll().unwrap().unwrap();
        assert_eq!(reloaded.tools.log_reader.overlays, ["dps", "friend"]);
        assert!(watch.poll().is_none(), "the change is reported once");
    }

    #[test]
    fn a_config_written_after_the_watch_started_is_picked_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eqld.toml");
        let mut watch = Watch::new(path.clone());
        assert_eq!(watch.path(), path);
        assert!(watch.poll().is_none(), "it was never there to begin with");

        write(&path, "\"dps\"");
        assert_eq!(
            watch.poll().unwrap().unwrap().tools.log_reader.overlays,
            ["dps"]
        );
    }

    #[test]
    fn a_malformed_config_is_reported_once_and_then_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eqld.toml");
        write(&path, "\"dps\"");
        let mut watch = Watch::new(path.clone());

        std::fs::write(&path, "overlays = [\"dps\"").unwrap();
        let err = watch.poll().unwrap().unwrap_err();
        assert!(matches!(err, ConfigError::Parse(..)), "{err}");
        assert!(
            watch.poll().is_none(),
            "the same broken file is not reported again"
        );

        write(&path, "\"friend\"");
        assert_eq!(
            watch.poll().unwrap().unwrap().tools.log_reader.overlays,
            ["friend"]
        );
    }

    #[test]
    fn a_config_that_disappears_is_reported_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eqld.toml");
        write(&path, "\"dps\"");
        let mut watch = Watch::new(path.clone());

        std::fs::remove_file(&path).unwrap();
        let err = watch.poll().unwrap().unwrap_err();
        assert!(matches!(err, ConfigError::Read(..)), "{err}");
        assert!(watch.poll().is_none(), "a still-missing file stays quiet");

        write(&path, "\"dps\"");
        assert_eq!(
            watch.poll().unwrap().unwrap().tools.log_reader.overlays,
            ["dps"]
        );
    }

    #[test]
    fn only_the_fields_a_running_daemon_can_swap_are_taken_from_a_reload() {
        let before: Config = toml::from_str(
            r#"
            [game]
            root = "/games/eq"
            poll_secs = 5
            [api]
            url = "https://a"
            token = "t"
            [state]
            path = "a.json"
            [tools.log_reader]
            overlays = ["dps"]
            "#,
        )
        .unwrap();
        let after: Config = toml::from_str(
            r#"
            [game]
            root = "/games/other"
            poll_secs = 30
            [api]
            url = "https://b"
            token = "u"
            [state]
            path = "b.json"
            [harvest]
            enabled = true
            [tools.log_reader]
            enabled = true
            overlays = ["dps", "friend"]
            hidden = ["friend"]
            atlas = "overlay"
            "#,
        )
        .unwrap();

        assert_eq!(
            before.frozen_changes(&after),
            vec![
                "game.root",
                "game.poll_secs",
                "api.url",
                "api.token",
                "state.path"
            ]
        );
        assert!(before.frozen_changes(&before.clone()).is_empty());

        let merged = before.hot_swap(after);
        assert_eq!(merged.game.root, PathBuf::from("/games/eq"));
        assert_eq!(merged.game.poll_secs, 5);
        assert_eq!(merged.api.url, "https://a");
        assert_eq!(merged.api.token, "t");
        assert_eq!(merged.state_path(), PathBuf::from("a.json"));
        assert!(merged.harvest.enabled);
        assert!(merged.tools.log_reader.enabled);
        assert_eq!(merged.tools.log_reader.overlays, ["dps", "friend"]);
        assert_eq!(merged.tools.log_reader.hidden, ["friend"]);
        assert_eq!(merged.tools.log_reader.atlas, AtlasMode::Overlay);
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
