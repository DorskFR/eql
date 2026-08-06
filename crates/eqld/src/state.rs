use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub files: BTreeMap<String, FileState>,
    #[serde(default)]
    pub logs: BTreeMap<String, LogState>,
    #[serde(default)]
    pub harvest: BTreeMap<String, FileState>,
    #[serde(default)]
    pub icons: BTreeMap<String, FileState>,
    #[serde(default)]
    pub fights: BTreeMap<String, FightsState>,
}

/// How far into a log's fight history has been accepted, keyed by log file.
/// Fights accumulate on the server, so this is the only thing stopping a tick
/// from re-sending the whole log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FightsState {
    pub last_start_wall_ms: i64,
    pub uploaded: usize,
    pub uploaded_at: Option<i64>,
}

/// Byte offset already shipped for a tailed log; a file first seen at offset
/// `len` is never back-filled with history.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogState {
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileState {
    pub mtime: Option<i64>,
    pub len: u64,
    pub hash: String,
    pub uploaded_hash: Option<String>,
    pub last_status: LastStatus,
    pub uploaded_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LastStatus {
    Uploaded,
    Rejected { status: u16 },
    Failed { error: String },
}

impl LastStatus {
    /// A rejection is the server telling us this exact payload is unacceptable
    /// (bad token, invalid body); replaying it cannot change the answer, so the
    /// file stays parked until its contents change.
    pub fn needs_retry(&self) -> bool {
        matches!(self, LastStatus::Failed { .. })
    }
}

impl State {
    pub fn load(path: &Path) -> Result<Self, StateError> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(StateError::Decode),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(StateError::Io(err)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), StateError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(StateError::Io)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(StateError::Encode)?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text).map_err(StateError::Io)?;
        std::fs::rename(&temp, path).map_err(StateError::Io)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("state io: {0}")]
    Io(#[source] std::io::Error),
    #[error("decoding state: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("encoding state: {0}")]
    Encode(#[source] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> State {
        let mut state = State::default();
        state.files.insert(
            "Dorsk_erudin-Inventory.txt".into(),
            FileState {
                mtime: Some(1_754_390_000),
                len: 128,
                hash: "abc".into(),
                uploaded_hash: Some("abc".into()),
                last_status: LastStatus::Uploaded,
                uploaded_at: Some(1_754_390_001),
            },
        );
        state
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("state.json");
        let state = sample();
        state.save(&path).unwrap();
        assert_eq!(State::load(&path).unwrap(), state);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn overwrites_an_existing_state_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        sample().save(&path).unwrap();

        let mut updated = sample();
        updated
            .files
            .get_mut("Dorsk_erudin-Inventory.txt")
            .unwrap()
            .hash = "def".into();
        updated.save(&path).unwrap();
        assert_eq!(State::load(&path).unwrap(), updated);
    }

    #[test]
    fn log_offsets_round_trip_and_predate_state_files_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = sample();
        state
            .logs
            .insert("eqlog_Dorsk_erudin.txt".into(), LogState { offset: 4096 });
        state.save(&path).unwrap();
        assert_eq!(State::load(&path).unwrap(), state);

        std::fs::write(&path, r#"{"files":{}}"#).unwrap();
        assert!(State::load(&path).unwrap().logs.is_empty());
    }

    #[test]
    fn fight_watermarks_round_trip_and_predate_state_files_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = sample();
        state.fights.insert(
            "eqlog_Dorsk_erudin.txt".into(),
            FightsState {
                last_start_wall_ms: 1_785_931_338_000,
                uploaded: 13,
                uploaded_at: Some(1_785_931_400),
            },
        );
        state.save(&path).unwrap();
        assert_eq!(State::load(&path).unwrap(), state);

        std::fs::write(&path, r#"{"files":{}}"#).unwrap();
        assert!(State::load(&path).unwrap().fights.is_empty());
    }

    #[test]
    fn missing_file_loads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            State::load(&dir.path().join("absent.json")).unwrap(),
            State::default()
        );
    }

    #[test]
    fn only_transport_failures_ask_for_a_retry() {
        assert!(LastStatus::Failed {
            error: "connection refused".into()
        }
        .needs_retry());
        assert!(!LastStatus::Rejected { status: 401 }.needs_retry());
        assert!(!LastStatus::Uploaded.needs_retry());
    }
}
