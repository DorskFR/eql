use crate::{
    backoff::Backoff,
    config::Config,
    fights, harvest, logs,
    state::{FightsState, FileState, LastStatus, LogState, State},
};
use eql_core::{
    api::{HarvestDoc, InventoryUpload, LogBatch},
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
    bytes_hash(contents.as_bytes())
}

pub fn bytes_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
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
    pub harvested: usize,
    pub harvest_skipped: usize,
    pub fights: usize,
}

/// One tick reads at most this much of each log; the rest waits for the next
/// tick so a long-idle daemon cannot pull a whole session into memory.
const MAX_LOG_CHUNK: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Watched {
    inventories: usize,
    logs: usize,
    harvest: Option<usize>,
}

fn fights_tool(
    installed: Option<&crate::tools::Runner>,
    enabled: bool,
) -> Option<crate::tools::Runner> {
    if !enabled {
        return None;
    }
    let found = installed.and_then(|base| base.sibling(crate::tools::FIGHTS_STEM));
    if installed.is_some() && found.is_none() {
        tracing::warn!(
            tool = crate::tools::FIGHTS_STEM,
            "the installed log reader has no fights tool; no fight history will be uploaded"
        );
    }
    found
}

pub struct Daemon {
    config: Config,
    client: reqwest::Client,
    state: State,
    state_path: PathBuf,
    backoff: Backoff,
    watched: Option<Watched>,
    installed: Option<crate::tools::Runner>,
    runner: Option<crate::tools::Runner>,
    fights_tool: Option<crate::tools::Runner>,
    last_replay: Option<std::time::Instant>,
    overlays: Option<crate::overlays::Supervisor>,
    processes: crate::overlays::ProcessWatch,
    config_watch: Option<crate::config::Watch>,
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
        let settings = &config.tools.log_reader;
        let plan = crate::overlays::plan(settings);
        for refusal in &plan.refused {
            tracing::warn!(%refusal, "overlay not launched");
        }
        if !crate::overlays::hiding_is_supported() {
            let windowed: Vec<_> = plan
                .hidden
                .iter()
                .filter(|overlay| overlay.headless_stem().is_none())
                .map(|overlay| overlay.name())
                .collect();
            if !windowed.is_empty() {
                tracing::warn!(
                    hidden = ?windowed,
                    "these overlays have no headless build and hiding one needs windows; they will be launched normally"
                );
            }
        }

        let asked_for = settings.enabled || !plan.wanted.is_empty();
        if asked_for {
            tracing::info!(
                atlas = ?settings.atlas,
                note = crate::overlays::atlas_mode_note(settings.atlas),
                replay = settings.replay_enabled(),
                "atlas mode"
            );
        }
        let installed = asked_for
            .then(|| crate::tools::Runner::discover(settings.exe.as_deref()))
            .flatten();
        if asked_for && installed.is_none() {
            tracing::warn!(
                hint = crate::tools::install_hint(settings),
                harvest = settings.enabled,
                overlays = ?plan.wanted.iter().map(|o| o.name()).collect::<Vec<_>>(),
                "log reader not found; nothing will be harvested and no overlay can start"
            );
        }
        let runner = settings
            .replay_enabled()
            .then(|| installed.clone())
            .flatten();
        let fights_tool = fights_tool(installed.as_ref(), settings.enabled);
        let overlays = installed
            .as_ref()
            .map(|base| crate::overlays::Supervisor::new(base, &plan.wanted, &plan.hidden));

        Ok(Self {
            config,
            client,
            state,
            state_path,
            backoff,
            watched: None,
            installed,
            runner,
            fights_tool,
            last_replay: None,
            overlays,
            processes: crate::overlays::ProcessWatch::new(),
            config_watch: None,
        })
    }

    /// Re-reads `path` on every tick and applies what can be applied live.
    pub fn watching(mut self, path: PathBuf) -> Self {
        self.config_watch = Some(crate::config::Watch::new(path));
        self
    }

    async fn reload(&mut self) {
        let Some(watch) = &mut self.config_watch else {
            return;
        };
        let path = watch.path().to_path_buf();
        match watch.poll() {
            None => {}
            Some(Ok(config)) => {
                tracing::info!(path = %path.display(), "config changed, applying");
                self.apply(config).await;
            }
            Some(Err(err)) => tracing::error!(
                path = %path.display(),
                %err,
                "config is unusable, staying on the last good one"
            ),
        }
    }

    async fn apply(&mut self, config: Config) {
        let frozen = self.config.frozen_changes(&config);
        if !frozen.is_empty() {
            tracing::warn!(
                fields = ?frozen,
                "these fields require a restart; the running values are kept"
            );
        }
        self.config = self.config.hot_swap(config);

        let settings = self.config.tools.log_reader.clone();
        let plan = crate::overlays::plan(&settings);
        for refusal in &plan.refused {
            tracing::warn!(%refusal, "overlay not launched");
        }

        let asked_for = settings.enabled || !plan.wanted.is_empty();
        self.installed = asked_for
            .then(|| crate::tools::Runner::discover(settings.exe.as_deref()))
            .flatten();

        let replay = settings
            .replay_enabled()
            .then(|| self.installed.clone())
            .flatten();
        if replay.is_some() != self.runner.is_some() {
            tracing::info!(
                enabled = replay.is_some(),
                atlas = ?settings.atlas,
                note = crate::overlays::atlas_mode_note(settings.atlas),
                "replay harvest"
            );
            self.last_replay = None;
        }
        self.runner = replay;
        self.fights_tool = fights_tool(self.installed.as_ref(), settings.enabled);

        let base = self.installed.clone();
        if base.is_none()
            && plan.wanted.is_empty()
            && self
                .overlays
                .as_ref()
                .is_none_or(crate::overlays::Supervisor::is_empty)
        {
            return;
        }
        let supervisor = self.overlays.get_or_insert_with(Default::default);
        let changes = supervisor
            .reconcile(base.as_ref(), &plan.wanted, &plan.hidden)
            .await;
        if !changes.missing.is_empty() {
            tracing::warn!(
                hint = crate::tools::install_hint(&settings),
                missing = ?changes.missing,
                "the log reader does not have these overlays; nothing to start"
            );
        }
        if changes.is_empty() {
            tracing::info!(overlays = ?supervisor.names(), "no overlay change");
        }
    }

    pub fn runner(&self) -> Option<&crate::tools::Runner> {
        self.runner.as_ref()
    }

    fn replay_due(&self) -> bool {
        self.last_replay
            .is_none_or(|at| at.elapsed() >= self.config.tools.log_reader.replay_interval())
    }

    async fn run_replay(&mut self, root: &Path, report: &mut TickReport) -> bool {
        if self.runner.is_none() && self.fights_tool.is_none() {
            return false;
        }
        if !self.replay_due() {
            return false;
        }
        let Ok(paths) = logs::scan(root) else {
            return false;
        };
        self.last_replay = Some(std::time::Instant::now());
        if let Some(runner) = self.runner.clone() {
            let replayed = crate::tools::replay_all(
                &runner,
                &paths,
                self.config.tools.log_reader.replay_timeout(),
            )
            .await;
            if replayed.ran > 0 || replayed.failed > 0 {
                tracing::debug!(
                    replayed = replayed.ran,
                    failed = replayed.failed,
                    "log reader replay"
                );
            }
        }
        self.collect_fights(&paths, report).await
    }

    async fn collect_fights(&mut self, paths: &[PathBuf], report: &mut TickReport) -> bool {
        let Some(tool) = self.fights_tool.clone() else {
            return false;
        };
        let dir = self.config.fights_dir();
        let mut dirty = false;
        for path in paths {
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some((character, server)) = logs::parse_filename(file_name) else {
                continue;
            };
            let key = file_name.to_string();
            let watermark = self
                .state
                .fights
                .get(&key)
                .map(|state| state.last_start_wall_ms);
            let out = fights::out_path(&dir, &character, &server);
            let since = watermark.map(fights::since_arg);
            if let Err(err) = crate::tools::fights(
                &tool,
                path,
                &out,
                since.as_deref(),
                self.config.tools.log_reader.replay_timeout(),
            )
            .await
            {
                tracing::warn!(log = %path.display(), %err, "fights dump failed");
                continue;
            }
            let text = match std::fs::read_to_string(&out) {
                Ok(text) => text,
                Err(err) => {
                    tracing::warn!(file = %out.display(), %err, "no fights dump to read");
                    continue;
                }
            };
            let emitted = match fights::parse(&text) {
                Ok(fights) => fights,
                Err(err) => {
                    report.parse_failures += 1;
                    tracing::warn!(file = %out.display(), %err, "fights dump is not json");
                    continue;
                }
            };
            let new = fights::newer_than(emitted, watermark);
            if new.is_empty() {
                continue;
            }
            dirty |= self
                .send_fights(&character, &server, key, &new, report)
                .await;
        }
        dirty
    }

    async fn send_fights(
        &mut self,
        character: &str,
        server: &str,
        key: String,
        new: &[serde_json::Value],
        report: &mut TickReport,
    ) -> bool {
        let upload = fights::Upload {
            character,
            server,
            fights: new,
        };
        let count = new.len();
        tracing::info!(%character, %server, fights = count, "uploading fights");
        match self.send(self.config.fights_endpoint(), &upload).await {
            Ok(status) if status.is_success() => {
                report.fights += count;
                let previous = self.state.fights.get(&key).copied().unwrap_or_default();
                let last_start_wall_ms = fights::newest(new).unwrap_or(previous.last_start_wall_ms);
                self.state.fights.insert(
                    key,
                    FightsState {
                        last_start_wall_ms,
                        uploaded: previous.uploaded + count,
                        uploaded_at: unix_secs(SystemTime::now()),
                    },
                );
                tracing::info!(%character, %server, fights = count, status = status.as_u16(), "uploaded fights");
                true
            }
            Ok(status) if status.is_server_error() => {
                report.retryable_failures += 1;
                tracing::warn!(%character, %server, status = status.as_u16(), "server error, fights replay next tick");
                false
            }
            Ok(status) => {
                report.rejections += 1;
                tracing::error!(%character, %server, status = status.as_u16(), "fights rejected, replaying next tick");
                false
            }
            Err(err) => {
                report.retryable_failures += 1;
                tracing::warn!(%character, %server, %err, "fights upload failed, replaying next tick");
                false
            }
        }
    }

    async fn supervise_overlays(&mut self, root: &Path) {
        let Self {
            overlays: Some(supervisor),
            processes,
            ..
        } = self
        else {
            return;
        };
        let running = processes.is_running(crate::overlays::GAME_PROCESS);
        let log = running.then(|| logs::latest(root)).flatten();
        supervisor.tick(running, log.as_deref()).await;
    }

    pub async fn shutdown(&mut self) {
        if let Some(supervisor) = &mut self.overlays {
            supervisor.stop().await;
        }
    }

    pub fn overlays(&self) -> Vec<&'static str> {
        self.overlays
            .as_ref()
            .map(crate::overlays::Supervisor::names)
            .unwrap_or_default()
    }

    pub fn hidden_overlays(&self) -> Vec<&'static str> {
        self.overlays
            .as_ref()
            .map(crate::overlays::Supervisor::hidden)
            .unwrap_or_default()
    }

    pub fn headless_overlays(&self) -> Vec<&'static str> {
        self.overlays
            .as_ref()
            .map(crate::overlays::Supervisor::headless)
            .unwrap_or_default()
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
        self.reload().await;
        let mut report = TickReport::default();
        let root = self.config.game.root.clone();
        let mut dirty_state = false;

        let mut inventories = 0;
        match scan(&root) {
            Ok(paths) => {
                inventories = paths.len();
                for path in paths {
                    match self.process(&path, &mut report).await {
                        Ok(changed) => dirty_state |= changed,
                        Err(err) => {
                            tracing::warn!(file = %path.display(), %err, "skipping file this tick")
                        }
                    }
                }
            }
            Err(err) => {
                tracing::error!(root = %root.display(), %err, "cannot scan game root, nothing will upload")
            }
        }

        let (logs_dirty, log_files) = self.tail_logs(&root, &mut report).await;
        dirty_state |= logs_dirty;

        dirty_state |= self.run_replay(&root, &mut report).await;
        self.supervise_overlays(&root).await;

        let mut harvest_files = None;
        if let Some(dir) = self.config.harvest_dir() {
            let (harvest_dirty, count) = self.harvest_docs(&dir, &mut report).await;
            dirty_state |= harvest_dirty;
            harvest_files = Some(count);
        }

        let watched = Watched {
            inventories,
            logs: log_files,
            harvest: harvest_files,
        };
        if self.watched != Some(watched) {
            match watched.harvest {
                Some(harvest) => tracing::info!(
                    inventory_dumps = watched.inventories,
                    log_files = watched.logs,
                    harvest_files = harvest,
                    "watching"
                ),
                None => tracing::info!(
                    inventory_dumps = watched.inventories,
                    log_files = watched.logs,
                    harvest = "disabled",
                    "watching"
                ),
            }
            self.watched = Some(watched);
        }

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
                tracing::error!(file = %file_name, %err, "inventory dump unparsable, retrying next tick");
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
        tracing::info!(character = %character, server = %server, entries = entry_count, "uploading inventory");
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
                    tracing::error!(character = %character, server = %server, "upload rejected: bad machine token, parked — will not retry until the dump changes");
                } else {
                    tracing::error!(character = %character, server = %server, status = status.as_u16(), "upload rejected, parked — will not retry until the dump changes");
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

    async fn tail_logs(&mut self, root: &Path, report: &mut TickReport) -> (bool, usize) {
        let paths = match logs::scan(root) {
            Ok(paths) => paths,
            Err(err) => {
                tracing::warn!(dir = %logs::log_dir(root).display(), %err, "cannot scan log directory");
                return (false, 0);
            }
        };
        let count = paths.len();
        let mut dirty = false;
        for path in paths {
            match self.tail(&path, report).await {
                Ok(changed) => dirty |= changed,
                Err(err) => tracing::warn!(file = %path.display(), %err, "skipping log this tick"),
            }
        }
        (dirty, count)
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
            tracing::info!(character = %character, server = %server, events = count, "uploading log events");
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

    async fn harvest_docs(&mut self, dir: &Path, report: &mut TickReport) -> (bool, usize) {
        let paths = match harvest::scan(dir) {
            Ok(paths) => paths,
            Err(err) => {
                tracing::warn!(dir = %dir.display(), %err, "cannot scan harvest directory");
                return (false, 0);
            }
        };
        let count = paths.len();
        let mut dirty = false;
        for group in harvest::group(paths) {
            match self.harvest_group(&group, report).await {
                Ok(changed) => dirty |= changed,
                Err(err) => {
                    tracing::warn!(file = %group.key, %err, "skipping harvest file this tick")
                }
            }
        }
        (dirty, count)
    }

    async fn harvest_group(
        &mut self,
        group: &harvest::Group,
        report: &mut TickReport,
    ) -> Result<bool, std::io::Error> {
        let file_name = group.key.clone();
        let mut len = 0;
        let mut mtime = None;
        // Hashed whole every tick, unlike inventory: a same-second rewrite of
        // equal length slips past an mtime+len gate, and these files are capped.
        let mut contents = Vec::new();
        for (path, _) in &group.files {
            let metadata = std::fs::metadata(path)?;
            len += metadata.len();
            mtime = mtime.max(metadata.modified().ok().and_then(unix_secs));
            contents.push(std::fs::read_to_string(path)?);
        }

        if len > harvest::MAX_BYTES {
            report.harvest_skipped += 1;
            tracing::warn!(
                file = %file_name,
                len,
                limit = harvest::MAX_BYTES,
                "harvest file is too large to ship, skipping"
            );
            return Ok(false);
        }
        let hash = content_hash(&contents.concat());
        match decide(self.state.harvest.get(&file_name), &hash) {
            Decision::SkipAlreadyUploaded => {
                report.harvest_skipped += 1;
                let previous = self
                    .state
                    .harvest
                    .get_mut(&file_name)
                    .expect("decided on it");
                let changed = previous.mtime != mtime || previous.len != len;
                previous.mtime = mtime;
                previous.len = len;
                return Ok(changed);
            }
            Decision::SkipRejected => {
                report.harvest_skipped += 1;
                return Ok(false);
            }
            Decision::Upload => {}
        }

        let mut docs = Vec::with_capacity(contents.len());
        for text in &contents {
            match serde_json::from_str(text) {
                Ok(doc) => docs.push(doc),
                Err(err) => {
                    report.parse_failures += 1;
                    tracing::warn!(file = %file_name, %err, "harvest file is not json yet, retrying next tick");
                    return Ok(false);
                }
            }
        }

        let upload = HarvestDoc {
            character: group.character.clone(),
            server: group.server.clone(),
            kind: group.kind.clone(),
            captured_at: mtime,
            doc: group.document(docs),
        };
        tracing::info!(
            character = %group.character,
            server = %group.server,
            kind = %group.kind,
            files = group.files.len(),
            "uploading harvest doc"
        );
        let outcome = self.send(self.config.harvest_endpoint(), &upload).await;
        let now = unix_secs(SystemTime::now());

        let last_status = match &outcome {
            Ok(status) if status.is_success() => {
                report.harvested += 1;
                tracing::info!(
                    character = %group.character,
                    server = %group.server,
                    kind = %group.kind,
                    builds = ?group.files.iter().filter_map(|(_, build)| build.as_deref()).collect::<Vec<_>>(),
                    status = status.as_u16(),
                    "uploaded harvest doc"
                );
                LastStatus::Uploaded
            }
            Ok(status) if status.is_server_error() => {
                report.retryable_failures += 1;
                tracing::warn!(file = %file_name, status = status.as_u16(), "server error, will retry");
                LastStatus::Failed {
                    error: format!("http {}", status.as_u16()),
                }
            }
            Ok(status) => {
                report.rejections += 1;
                tracing::error!(file = %file_name, status = status.as_u16(), "harvest rejected, parked — will not retry until the file changes");
                LastStatus::Rejected {
                    status: status.as_u16(),
                }
            }
            Err(err) => {
                report.retryable_failures += 1;
                tracing::warn!(file = %file_name, %err, "harvest upload failed, will retry");
                LastStatus::Failed {
                    error: err.to_string(),
                }
            }
        };

        let uploaded = matches!(last_status, LastStatus::Uploaded);
        let previous_uploaded_hash = self
            .state
            .harvest
            .get(&file_name)
            .and_then(|previous| previous.uploaded_hash.clone());
        self.state.harvest.insert(
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
