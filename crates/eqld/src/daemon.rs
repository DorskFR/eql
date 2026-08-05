use crate::{
    backoff::Backoff,
    config::Config,
    logs,
    state::{FileState, LastStatus, LogState, State},
};
use eql_core::{
    api::{InventoryUpload, LogBatch},
    inventory,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const INVENTORY_SUFFIX: &str = "-Inventory.txt";

pub fn is_inventory_file(file_name: &str) -> bool {
    file_name.len() > INVENTORY_SUFFIX.len() && file_name.ends_with(INVENTORY_SUFFIX)
}

pub fn scan(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        if name.to_str().is_some_and(is_inventory_file) {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

pub fn unix_secs(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
}

pub fn content_hash(contents: &str) -> String {
    Sha256::digest(contents.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

pub fn needs_read(previous: Option<&FileState>, mtime: Option<i64>, len: u64) -> bool {
    match previous {
        None => true,
        Some(previous) => {
            previous.last_status.needs_retry()
                || previous.len != len
                || mtime.is_none()
                || previous.mtime != mtime
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Upload,
    SkipAlreadyUploaded,
    SkipRejected,
}

pub fn decide(previous: Option<&FileState>, hash: &str) -> Decision {
    match previous {
        None => Decision::Upload,
        Some(previous) if previous.uploaded_hash.as_deref() == Some(hash) => {
            Decision::SkipAlreadyUploaded
        }
        Some(previous) => match &previous.last_status {
            LastStatus::Rejected { .. } if previous.hash == hash => Decision::SkipRejected,
            _ => Decision::Upload,
        },
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TickReport {
    pub uploaded: usize,
    pub skipped: usize,
    pub parse_failures: usize,
    pub retryable_failures: usize,
    pub rejections: usize,
    pub log_events: usize,
    pub log_lines_dropped: usize,
}

/// One tick reads at most this much of each log; the rest waits for the next
/// tick so a long-idle daemon cannot pull a whole session into memory.
const MAX_LOG_CHUNK: u64 = 4 * 1024 * 1024;

pub struct Daemon {
    config: Config,
    client: reqwest::Client,
    state: State,
    state_path: PathBuf,
    backoff: Backoff,
}

impl Daemon {
    pub fn new(config: Config) -> Result<Self, DaemonError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(DaemonError::Client)?;
        let state_path = config.state_path();
        let state = State::load(&state_path).map_err(DaemonError::State)?;
        let backoff = Backoff::new(config.poll_interval());
        Ok(Self {
            config,
            client,
            state,
            state_path,
            backoff,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    pub fn delay(&self) -> Duration {
        self.backoff.delay()
    }

    pub async fn tick(&mut self) -> TickReport {
        let mut report = TickReport::default();
        let root = self.config.game.root.clone();
        let mut dirty_state = false;

        match scan(&root) {
            Ok(paths) => {
                for path in paths {
                    match self.process(&path, &mut report).await {
                        Ok(changed) => dirty_state |= changed,
                        Err(err) => {
                            tracing::warn!(file = %path.display(), %err, "skipping file this tick")
                        }
                    }
                }
            }
            Err(err) => tracing::warn!(root = %root.display(), %err, "cannot scan game root"),
        }

        dirty_state |= self.tail_logs(&root, &mut report).await;

        if report.log_events > 0 || report.log_lines_dropped > 0 {
            tracing::info!(
                events = report.log_events,
                dropped_lines = report.log_lines_dropped,
                "harvested log events"
            );
        }

        if dirty_state {
            if let Err(err) = self.state.save(&self.state_path) {
                tracing::error!(path = %self.state_path.display(), %err, "cannot persist state");
            }
        }

        if report.retryable_failures > 0 {
            self.backoff.fail();
            tracing::warn!(
                retry_in_secs = self.backoff.delay().as_secs(),
                "upload failures, backing off"
            );
        } else if report.uploaded > 0 {
            self.backoff.reset();
        }
        report
    }

    async fn process(
        &mut self,
        path: &Path,
        report: &mut TickReport,
    ) -> Result<bool, std::io::Error> {
        let Some(file_name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(String::from)
        else {
            return Ok(false);
        };
        let metadata = std::fs::metadata(path)?;
        let len = metadata.len();
        let mtime = metadata.modified().ok().and_then(unix_secs);

        if !needs_read(self.state.files.get(&file_name), mtime, len) {
            report.skipped += 1;
            return Ok(false);
        }

        let contents = std::fs::read_to_string(path)?;
        let hash = content_hash(&contents);
        match decide(self.state.files.get(&file_name), &hash) {
            Decision::SkipAlreadyUploaded => {
                report.skipped += 1;
                let previous = self.state.files.get_mut(&file_name).expect("decided on it");
                let changed = previous.mtime != mtime || previous.len != len;
                previous.mtime = mtime;
                previous.len = len;
                return Ok(changed);
            }
            Decision::SkipRejected => {
                report.skipped += 1;
                return Ok(false);
            }
            Decision::Upload => {}
        }

        let (character, server) = match inventory::parse_filename(&file_name) {
            Ok(parts) => parts,
            Err(err) => {
                report.parse_failures += 1;
                tracing::warn!(file = %file_name, %err, "unparsable inventory filename");
                return Ok(false);
            }
        };
        let entries = match inventory::parse(&contents) {
            Ok(entries) => entries,
            Err(err) => {
                report.parse_failures += 1;
                tracing::warn!(file = %file_name, %err, "inventory dump unparsable, retrying next tick");
                return Ok(false);
            }
        };

        let upload = InventoryUpload {
            character: character.clone(),
            server: server.clone(),
            captured_at: mtime,
            entries,
            raw: Some(contents),
        };
        let entry_count = upload.entries.len();
        let outcome = self.send(self.config.endpoint(), &upload).await;
        let now = unix_secs(SystemTime::now());

        let last_status = match &outcome {
            Ok(status) if status.is_success() => {
                report.uploaded += 1;
                tracing::info!(
                    character = %character,
                    server = %server,
                    entries = entry_count,
                    status = status.as_u16(),
                    "uploaded inventory"
                );
                LastStatus::Uploaded
            }
            Ok(status) if status.is_server_error() => {
                report.retryable_failures += 1;
                tracing::warn!(character = %character, server = %server, status = status.as_u16(), "server error, will retry");
                LastStatus::Failed {
                    error: format!("http {}", status.as_u16()),
                }
            }
            Ok(status) => {
                report.rejections += 1;
                if status.as_u16() == 401 {
                    tracing::error!(character = %character, server = %server, "upload rejected: bad machine token, parked until the dump changes");
                } else {
                    tracing::error!(character = %character, server = %server, status = status.as_u16(), "upload rejected, parked until the dump changes");
                }
                LastStatus::Rejected {
                    status: status.as_u16(),
                }
            }
            Err(err) => {
                report.retryable_failures += 1;
                tracing::warn!(character = %character, server = %server, %err, "upload failed, will retry");
                LastStatus::Failed {
                    error: err.to_string(),
                }
            }
        };

        let uploaded = matches!(last_status, LastStatus::Uploaded);
        let previous_uploaded_hash = self
            .state
            .files
            .get(&file_name)
            .and_then(|previous| previous.uploaded_hash.clone());
        self.state.files.insert(
            file_name,
            FileState {
                mtime,
                len,
                hash: hash.clone(),
                uploaded_hash: if uploaded {
                    Some(hash)
                } else {
                    previous_uploaded_hash
                },
                uploaded_at: if uploaded { now } else { None },
                last_status,
            },
        );
        Ok(true)
    }

    async fn tail_logs(&mut self, root: &Path, report: &mut TickReport) -> bool {
        let paths = match logs::scan(root) {
            Ok(paths) => paths,
            Err(err) => {
                tracing::warn!(dir = %logs::log_dir(root).display(), %err, "cannot scan log directory");
                return false;
            }
        };
        let mut dirty = false;
        for path in paths {
            match self.tail(&path, report).await {
                Ok(changed) => dirty |= changed,
                Err(err) => tracing::warn!(file = %path.display(), %err, "skipping log this tick"),
            }
        }
        dirty
    }

    /// Delivery is at-least-once: the offset only advances after the batch is
    /// accepted, so a failed post replays the same lines on the next tick.
    async fn tail(&mut self, path: &Path, report: &mut TickReport) -> Result<bool, std::io::Error> {
        let Some(file_name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(String::from)
        else {
            return Ok(false);
        };
        let Some((character, server)) = logs::parse_filename(&file_name) else {
            return Ok(false);
        };
        let len = std::fs::metadata(path)?.len();

        let Some(previous) = self.state.logs.get(&file_name).copied() else {
            self.state.logs.insert(file_name, LogState { offset: len });
            tracing::info!(character = %character, server = %server, offset = len, "tailing log from its end");
            return Ok(true);
        };

        let mut offset = previous.offset;
        let mut dirty = false;
        if offset > len {
            tracing::info!(character = %character, server = %server, "log rotated or truncated, reading from the top");
            offset = 0;
            dirty = true;
        }

        let mut chunk = Vec::new();
        if offset < len {
            let mut file = std::fs::File::open(path)?;
            file.seek(SeekFrom::Start(offset))?;
            file.take(MAX_LOG_CHUNK).read_to_end(&mut chunk)?;
        }
        let (harvest, consumed) = logs::harvest(&chunk);
        if consumed == 0 {
            if dirty {
                self.state.logs.insert(file_name, LogState { offset });
            }
            return Ok(dirty);
        }
        report.log_lines_dropped += harvest.dropped;

        if !harvest.events.is_empty() {
            let batch = LogBatch {
                character: character.clone(),
                server: server.clone(),
                events: harvest.events,
            };
            let count = batch.events.len();
            match self.send(self.config.events_endpoint(), &batch).await {
                Ok(status) if status.is_success() => {
                    report.log_events += count;
                    tracing::info!(character = %character, server = %server, events = count, "uploaded log events");
                }
                Ok(status) if status.is_server_error() => {
                    report.retryable_failures += 1;
                    tracing::warn!(character = %character, server = %server, status = status.as_u16(), "log events rejected by server error, replaying next tick");
                    return Ok(dirty);
                }
                Ok(status) => {
                    report.rejections += 1;
                    tracing::error!(character = %character, server = %server, status = status.as_u16(), "log events rejected, replaying next tick");
                    return Ok(dirty);
                }
                Err(err) => {
                    report.retryable_failures += 1;
                    tracing::warn!(character = %character, server = %server, %err, "log event upload failed, replaying next tick");
                    return Ok(dirty);
                }
            }
        }

        self.state.logs.insert(
            file_name,
            LogState {
                offset: offset + consumed as u64,
            },
        );
        Ok(true)
    }

    async fn send<T: Serialize>(
        &self,
        url: String,
        body: &T,
    ) -> reqwest::Result<reqwest::StatusCode> {
        self.client
            .post(url)
            .bearer_auth(&self.config.api.token)
            .json(body)
            .send()
            .await
            .map(|response| response.status())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("building http client: {0}")]
    Client(#[source] reqwest::Error),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_state(mtime: i64, len: u64, hash: &str, status: LastStatus) -> FileState {
        FileState {
            mtime: Some(mtime),
            len,
            hash: hash.into(),
            uploaded_hash: matches!(status, LastStatus::Uploaded).then(|| hash.to_string()),
            last_status: status,
            uploaded_at: Some(mtime),
        }
    }

    #[test]
    fn filters_inventory_filenames() {
        assert!(is_inventory_file("Dorsk_erudin-Inventory.txt"));
        assert!(!is_inventory_file("-Inventory.txt"));
        assert!(!is_inventory_file("eqlog_Dorsk_erudin.txt"));
        assert!(!is_inventory_file("Dorsk_erudin-Inventory.txt.bak"));
        assert!(!is_inventory_file("Dorsk_erudin-Bank.txt"));
    }

    #[test]
    fn scan_ignores_non_inventory_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Dorsk_erudin-Inventory.txt"), "x").unwrap();
        std::fs::write(dir.path().join("Vala_erudin-Inventory.txt"), "x").unwrap();
        std::fs::write(dir.path().join("eqlog_Dorsk_erudin.txt"), "x").unwrap();
        std::fs::create_dir(dir.path().join("Nested-Inventory.txt")).unwrap();

        let found: Vec<String> = scan(dir.path())
            .unwrap()
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            found,
            vec!["Dorsk_erudin-Inventory.txt", "Vala_erudin-Inventory.txt"]
        );
    }

    #[test]
    fn hash_is_stable_and_content_sensitive() {
        assert_eq!(content_hash("abc").len(), 64);
        assert_eq!(content_hash("abc"), content_hash("abc"));
        assert_ne!(content_hash("abc"), content_hash("abd"));
    }

    #[test]
    fn pre_epoch_mtime_does_not_panic() {
        let before_epoch = UNIX_EPOCH - Duration::from_secs(60);
        assert_eq!(unix_secs(before_epoch), None);
        assert_eq!(unix_secs(UNIX_EPOCH), Some(0));
    }

    #[test]
    fn unseen_and_changed_files_are_read() {
        assert!(needs_read(None, Some(10), 5));
        let previous = file_state(10, 5, "h", LastStatus::Uploaded);
        assert!(!needs_read(Some(&previous), Some(10), 5));
        assert!(needs_read(Some(&previous), Some(11), 5));
        assert!(needs_read(Some(&previous), Some(10), 6));
        assert!(needs_read(Some(&previous), None, 5));
    }

    #[test]
    fn failed_uploads_stay_dirty_across_ticks() {
        let previous = file_state(
            10,
            5,
            "h",
            LastStatus::Failed {
                error: "boom".into(),
            },
        );
        assert!(needs_read(Some(&previous), Some(10), 5));
        assert_eq!(decide(Some(&previous), "h"), Decision::Upload);
    }

    #[test]
    fn touched_but_identical_content_is_not_reuploaded() {
        let previous = file_state(10, 5, "h", LastStatus::Uploaded);
        assert!(needs_read(Some(&previous), Some(99), 5));
        assert_eq!(decide(Some(&previous), "h"), Decision::SkipAlreadyUploaded);
    }

    #[test]
    fn changed_content_uploads_again() {
        let previous = file_state(10, 5, "h", LastStatus::Uploaded);
        assert_eq!(decide(Some(&previous), "other"), Decision::Upload);
        assert_eq!(decide(None, "h"), Decision::Upload);
    }

    #[test]
    fn rejected_files_park_until_content_changes() {
        let previous = file_state(10, 5, "h", LastStatus::Rejected { status: 401 });
        assert_eq!(decide(Some(&previous), "h"), Decision::SkipRejected);
        assert_eq!(decide(Some(&previous), "changed"), Decision::Upload);
    }
}
