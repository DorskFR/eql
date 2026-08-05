use crate::backoff::Backoff;
use crate::config::{AtlasMode, LogReaderConfig};
use crate::tools::Runner;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

pub const GAME_PROCESS: &str = "eqgame.exe";

/// An overlay that dies sooner than this is crash-looping, so its restart
/// backoff keeps growing instead of resetting.
const STABLE_AFTER: Duration = Duration::from_secs(60);
const RESTART_BASE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Dps,
    SessionReport,
    Friend,
    Atlas,
}

pub const KNOWN: [Overlay; 4] = [
    Overlay::Dps,
    Overlay::SessionReport,
    Overlay::Friend,
    Overlay::Atlas,
];

impl Overlay {
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        KNOWN.into_iter().find(|overlay| overlay.name() == name)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Overlay::Dps => "dps",
            Overlay::SessionReport => "session_report",
            Overlay::Friend => "friend",
            Overlay::Atlas => "atlas",
        }
    }

    pub fn stem(&self) -> &'static str {
        match self {
            Overlay::Dps => "eql_dps_meter",
            Overlay::SessionReport => "eql_session_report",
            Overlay::Friend => "eql_friend_overlay",
            Overlay::Atlas => crate::tools::ATLAS_STEM,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("unknown overlay {0:?}; known overlays are dps, session_report, friend, atlas")]
    Unknown(String),
    #[error("overlay {0} is listed more than once")]
    Duplicate(&'static str),
    #[error(
        "the atlas overlay autosaves and would fight the headless --replay harvest; \
         drop it from overlays, or set [tools.log_reader] atlas = \"overlay\" to let the \
         overlay keep the database instead"
    )]
    AtlasFightsReplay,
    #[error("hidden overlay {0:?} is not in overlays; hidden must be a subset of overlays")]
    HiddenNotListed(String),
    #[error(
        "the atlas overlay cannot be hidden: quests only accrue for the ones you add by hand \
         in its quest window, so it runs visible"
    )]
    AtlasNotHideable,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Plan {
    pub wanted: Vec<Overlay>,
    pub hidden: Vec<Overlay>,
    pub refused: Vec<Refusal>,
}

/// The Atlas persists on its own schedule, so a running overlay and a
/// concurrent `--replay` overwrite each other's database.
pub fn plan(settings: &LogReaderConfig) -> Plan {
    let replay_enabled = settings.replay_enabled();
    let mut plan = Plan::default();
    for name in &settings.overlays {
        let Some(overlay) = Overlay::parse(name) else {
            plan.refused.push(Refusal::Unknown(name.clone()));
            continue;
        };
        if plan.wanted.contains(&overlay) {
            plan.refused.push(Refusal::Duplicate(overlay.name()));
            continue;
        }
        if overlay == Overlay::Atlas && replay_enabled {
            plan.refused.push(Refusal::AtlasFightsReplay);
            continue;
        }
        plan.wanted.push(overlay);
    }
    for name in &settings.hidden {
        let Some(overlay) = Overlay::parse(name).filter(|overlay| plan.wanted.contains(overlay))
        else {
            plan.refused.push(Refusal::HiddenNotListed(name.clone()));
            continue;
        };
        if overlay == Overlay::Atlas {
            plan.refused.push(Refusal::AtlasNotHideable);
            continue;
        }
        if plan.hidden.contains(&overlay) {
            plan.refused.push(Refusal::Duplicate(overlay.name()));
            continue;
        }
        plan.hidden.push(overlay);
    }
    plan
}

pub fn hiding_is_supported() -> bool {
    cfg!(windows)
}

pub fn atlas_mode_note(mode: AtlasMode) -> &'static str {
    match mode {
        AtlasMode::Replay => "the --replay tick keeps the atlas database; no quest data is written",
        AtlasMode::Overlay => {
            "the atlas overlay keeps its own database; quests accrue for the ones you track in it"
        }
    }
}

enum Handle {
    Child(Box<tokio::process::Child>),
    #[cfg(windows)]
    Hidden(crate::hidden::Process),
}

impl Handle {
    fn id(&self) -> Option<u32> {
        match self {
            Handle::Child(child) => child.id(),
            #[cfg(windows)]
            Handle::Hidden(process) => Some(process.id()),
        }
    }

    /// `Ok(None)` while it is still up; the inner option is the exit code,
    /// absent when a signal took it.
    fn try_wait(&mut self) -> std::io::Result<Option<Option<i32>>> {
        match self {
            Handle::Child(child) => Ok(child.try_wait()?.map(|status| status.code())),
            #[cfg(windows)]
            Handle::Hidden(process) => Ok(process.try_wait()?.map(Some)),
        }
    }

    async fn stop(&mut self) {
        match self {
            Handle::Child(child) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
            #[cfg(windows)]
            Handle::Hidden(process) => process.stop().await,
        }
    }
}

struct Running {
    child: Handle,
    started: Instant,
}

struct Entry {
    overlay: Overlay,
    runner: Runner,
    hidden: bool,
    running: Option<Running>,
    backoff: Backoff,
    retry_at: Option<Instant>,
}

impl Entry {
    fn new(overlay: Overlay, runner: Runner, hidden: bool) -> Self {
        Self {
            overlay,
            runner,
            hidden,
            running: None,
            backoff: Backoff::with_max(RESTART_BASE, crate::backoff::MAX_BACKOFF),
            retry_at: None,
        }
    }

    fn hold_off(&mut self) {
        self.retry_at = Some(Instant::now() + self.backoff.delay());
        self.backoff.fail();
    }

    fn reap(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        match running.child.try_wait() {
            Ok(None) => return,
            Ok(Some(code)) => {
                let stable = running.started.elapsed() >= STABLE_AFTER;
                tracing::warn!(
                    overlay = self.overlay.name(),
                    code = ?code,
                    up_secs = running.started.elapsed().as_secs(),
                    "overlay exited while the game is up, restarting"
                );
                if stable {
                    self.backoff.reset();
                }
            }
            Err(err) => tracing::warn!(
                overlay = self.overlay.name(),
                %err,
                "cannot poll overlay, treating it as gone"
            ),
        }
        self.running = None;
        self.hold_off();
    }

    fn start(&mut self, log: &Path) {
        if self.running.is_some() || self.retry_at.is_some_and(|at| Instant::now() < at) {
            return;
        }
        self.retry_at = None;
        match spawn(&self.runner, log, self.hidden) {
            Ok(child) => {
                tracing::info!(
                    overlay = self.overlay.name(),
                    pid = child.id(),
                    hidden = self.hidden && hiding_is_supported(),
                    log = %log.display(),
                    "overlay started"
                );
                self.running = Some(Running {
                    child,
                    started: Instant::now(),
                });
            }
            Err(err) => {
                tracing::warn!(
                    overlay = self.overlay.name(),
                    program = %self.runner.program().display(),
                    %err,
                    "cannot start overlay"
                );
                self.hold_off();
            }
        }
    }

    async fn stop(&mut self) {
        if let Some(mut running) = self.running.take() {
            running.child.stop().await;
            tracing::info!(overlay = self.overlay.name(), "overlay stopped");
        }
        self.backoff.reset();
        self.retry_at = None;
    }
}

fn spawn(runner: &Runner, log: &Path, hidden: bool) -> std::io::Result<Handle> {
    let args = runner.overlay_args(log);
    let dir = runner.program().parent();
    #[cfg(windows)]
    if hidden {
        return crate::hidden::spawn(runner.program(), &args, dir).map(Handle::Hidden);
    }
    #[cfg(not(windows))]
    let _ = hidden;
    let mut command = tokio::process::Command::new(runner.program());
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    command.spawn().map(|child| Handle::Child(Box::new(child)))
}

pub struct Supervisor {
    entries: Vec<Entry>,
    watching: Option<PathBuf>,
}

impl Supervisor {
    pub fn new(base: &Runner, overlays: &[Overlay], hidden: &[Overlay]) -> Self {
        let mut entries = Vec::new();
        for overlay in overlays {
            match base.sibling(overlay.stem()) {
                Some(runner) => {
                    entries.push(Entry::new(*overlay, runner, hidden.contains(overlay)))
                }
                None => tracing::warn!(
                    overlay = overlay.name(),
                    expected = overlay.stem(),
                    near = %base.program().display(),
                    "overlay is not installed, it will not be launched"
                ),
            }
        }
        Self {
            entries,
            watching: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.entries
            .iter()
            .map(|entry| entry.overlay.name())
            .collect()
    }

    pub fn hidden(&self) -> Vec<&'static str> {
        self.entries
            .iter()
            .filter(|entry| entry.hidden)
            .map(|entry| entry.overlay.name())
            .collect()
    }

    pub fn running(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.running.is_some())
            .count()
    }

    pub async fn tick(&mut self, game_running: bool, log: Option<&Path>) {
        if !game_running {
            if self.watching.is_some() {
                tracing::info!("the game is gone, stopping overlays");
            }
            self.stop().await;
            return;
        }
        let Some(log) = log else {
            return;
        };
        if self
            .watching
            .as_deref()
            .is_some_and(|current| current != log)
        {
            tracing::info!(log = %log.display(), "the active character changed, restarting overlays");
            self.stop().await;
        }
        if self.watching.is_none() {
            tracing::info!(
                overlays = ?self.names(),
                log = %log.display(),
                "the game is up, starting overlays"
            );
        }
        self.watching = Some(log.to_path_buf());
        for entry in &mut self.entries {
            entry.reap();
            entry.start(log);
        }
    }

    pub async fn stop(&mut self) {
        for entry in &mut self.entries {
            entry.stop().await;
        }
        self.watching = None;
    }
}

pub struct ProcessWatch {
    system: sysinfo::System,
}

impl ProcessWatch {
    pub fn new() -> Self {
        Self {
            system: sysinfo::System::new(),
        }
    }

    /// The daemon also runs beside a Wine prefix, where the client still
    /// reports itself as `eqgame.exe`.
    pub fn is_running(&mut self, name: &str) -> bool {
        self.system
            .refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        self.system
            .processes()
            .values()
            .filter_map(|process| process.name().to_str())
            .any(|found| found.eq_ignore_ascii_case(name))
    }
}

impl Default for ProcessWatch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn settings(overlays: &[&str], enabled: bool) -> LogReaderConfig {
        LogReaderConfig {
            enabled,
            overlays: names(overlays),
            ..LogReaderConfig::default()
        }
    }

    #[test]
    fn every_known_overlay_round_trips_through_its_config_name() {
        for overlay in KNOWN {
            assert_eq!(Overlay::parse(overlay.name()), Some(overlay));
            assert!(overlay.stem().starts_with("eql_"));
        }
        assert_eq!(Overlay::parse("  DPS "), Some(Overlay::Dps));
        assert_eq!(Overlay::parse("dps_meter"), None);
        assert_eq!(Overlay::parse(""), None);
    }

    #[test]
    fn the_atlas_overlay_is_refused_while_the_replay_harvest_is_on() {
        let plan = plan(&settings(&["dps", "atlas"], true));
        assert_eq!(plan.wanted, vec![Overlay::Dps]);
        assert_eq!(plan.refused, vec![Refusal::AtlasFightsReplay]);
        assert!(plan.refused[0].to_string().contains("autosave"));
    }

    #[test]
    fn the_atlas_overlay_is_allowed_when_nothing_replays() {
        let plan = plan(&settings(&["atlas"], false));
        assert_eq!(plan.wanted, vec![Overlay::Atlas]);
        assert!(plan.refused.is_empty());
    }

    #[test]
    fn atlas_overlay_mode_runs_the_overlay_and_skips_the_replay() {
        let harvesting = LogReaderConfig {
            atlas: AtlasMode::Overlay,
            ..settings(&["dps", "atlas"], true)
        };
        assert!(
            !harvesting.replay_enabled(),
            "the overlay keeps the database, not --replay"
        );
        let plan = plan(&harvesting);
        assert_eq!(plan.wanted, vec![Overlay::Dps, Overlay::Atlas]);
        assert!(plan.refused.is_empty());
        assert!(atlas_mode_note(AtlasMode::Overlay).contains("quests"));
    }

    #[test]
    fn replay_is_the_atlas_mode_nothing_asks_for() {
        let harvesting = settings(&["dps"], true);
        assert_eq!(harvesting.atlas, AtlasMode::Replay);
        assert!(harvesting.replay_enabled());
        assert!(!settings(&["dps"], false).replay_enabled());
    }

    #[test]
    fn hidden_overlays_must_be_a_subset_of_the_overlays() {
        let asked = LogReaderConfig {
            hidden: names(&["dps", "friend", "sparkles", "dps"]),
            ..settings(&["dps", "session_report"], false)
        };
        let plan = plan(&asked);
        assert_eq!(plan.wanted, vec![Overlay::Dps, Overlay::SessionReport]);
        assert_eq!(plan.hidden, vec![Overlay::Dps]);
        assert_eq!(
            plan.refused,
            vec![
                Refusal::HiddenNotListed("friend".into()),
                Refusal::HiddenNotListed("sparkles".into()),
                Refusal::Duplicate("dps"),
            ]
        );
    }

    #[test]
    fn the_atlas_overlay_is_never_hidden_because_quests_need_a_human() {
        let asked = LogReaderConfig {
            atlas: AtlasMode::Overlay,
            hidden: names(&["atlas"]),
            ..settings(&["atlas"], true)
        };
        let plan = plan(&asked);
        assert_eq!(plan.wanted, vec![Overlay::Atlas], "it still runs, visibly");
        assert!(plan.hidden.is_empty());
        assert_eq!(plan.refused, vec![Refusal::AtlasNotHideable]);
    }

    #[test]
    fn nothing_is_hidden_by_default() {
        let plan = plan(&settings(&["dps"], true));
        assert_eq!(plan.wanted, vec![Overlay::Dps]);
        assert!(plan.hidden.is_empty());
        assert!(plan.refused.is_empty());
    }

    #[test]
    fn unknown_and_repeated_names_are_reported_not_launched() {
        let plan = plan(&settings(&["dps", "sparkles", "dps", "friend"], true));
        assert_eq!(plan.wanted, vec![Overlay::Dps, Overlay::Friend]);
        assert_eq!(
            plan.refused,
            vec![
                Refusal::Unknown("sparkles".into()),
                Refusal::Duplicate("dps")
            ]
        );
    }

    #[test]
    fn an_empty_list_plans_nothing() {
        assert_eq!(plan(&settings(&[], true)), Plan::default());
    }

    #[test]
    fn only_installed_overlays_become_entries() {
        let dir = tempfile::tempdir().unwrap();
        let atlas = dir.path().join("eql_atlas.exe");
        std::fs::write(&atlas, b"").unwrap();
        std::fs::write(dir.path().join("eql_dps_meter.exe"), b"").unwrap();

        let supervisor = Supervisor::new(
            &Runner::Frozen(atlas),
            &[Overlay::Dps, Overlay::Friend],
            &[Overlay::Dps],
        );
        assert_eq!(supervisor.names(), vec!["dps"]);
        assert_eq!(supervisor.hidden(), vec!["dps"]);
        assert!(!supervisor.is_empty());
    }

    #[test]
    fn a_process_is_found_by_name_and_a_made_up_one_is_not() {
        let pid = sysinfo::get_current_pid().unwrap();
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, false);
        let me = system
            .process(pid)
            .unwrap()
            .name()
            .to_str()
            .unwrap()
            .to_string();

        let mut watch = ProcessWatch::new();
        assert!(watch.is_running(&me), "this test binary is running");
        assert!(!watch.is_running("eql-no-such-process.exe"));
    }

    #[cfg(unix)]
    mod supervision {
        use super::*;

        fn script(dir: &Path, name: &str, body: &str) -> Runner {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            Runner::Frozen(path)
        }

        fn supervisor(runner: Runner) -> Supervisor {
            Supervisor {
                entries: vec![Entry::new(Overlay::Dps, runner, false)],
                watching: None,
            }
        }

        async fn wait(supervisor: &mut Supervisor) {
            match &mut supervisor.entries[0].running.as_mut().unwrap().child {
                Handle::Child(child) => {
                    child.wait().await.unwrap();
                }
            }
        }

        #[tokio::test]
        async fn a_hidden_overlay_still_launches_where_desktops_do_not_exist() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("eqlog_Dorsk_erudin.txt");
            std::fs::write(&log, "").unwrap();
            let seen = dir.path().join("seen.txt");
            let runner = script(
                dir.path(),
                "overlay",
                &format!("echo \"$1\" >> {}", seen.display()),
            );
            let mut supervisor = Supervisor {
                entries: vec![Entry::new(Overlay::Dps, runner, true)],
                watching: None,
            };
            assert_eq!(supervisor.hidden(), vec!["dps"]);

            supervisor.tick(true, Some(&log)).await;
            wait(&mut supervisor).await;
            assert_eq!(
                std::fs::read_to_string(&seen).unwrap().trim(),
                log.display().to_string()
            );
        }

        #[tokio::test]
        async fn overlays_run_while_the_game_is_up_and_stop_when_it_goes() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("eqlog_Dorsk_erudin.txt");
            std::fs::write(&log, "").unwrap();
            let mut supervisor = supervisor(script(dir.path(), "overlay", "sleep 30"));

            supervisor.tick(true, Some(&log)).await;
            assert_eq!(supervisor.running(), 1);

            supervisor.tick(true, Some(&log)).await;
            assert_eq!(supervisor.running(), 1, "a live overlay is not restarted");

            supervisor.tick(false, None).await;
            assert_eq!(supervisor.running(), 0);
            assert!(supervisor.watching.is_none());
        }

        #[tokio::test]
        async fn nothing_starts_before_the_game_does() {
            let dir = tempfile::tempdir().unwrap();
            let mut supervisor = supervisor(script(dir.path(), "overlay", "sleep 30"));
            supervisor.tick(false, None).await;
            assert_eq!(supervisor.running(), 0);
        }

        #[tokio::test]
        async fn the_overlay_is_handed_the_log_path() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("eqlog_Dorsk_erudin.txt");
            std::fs::write(&log, "").unwrap();
            let seen = dir.path().join("seen.txt");
            let mut supervisor = supervisor(script(
                dir.path(),
                "overlay",
                &format!("echo \"$1\" >> {}", seen.display()),
            ));

            supervisor.tick(true, Some(&log)).await;
            wait(&mut supervisor).await;
            assert_eq!(
                std::fs::read_to_string(&seen).unwrap().trim(),
                log.display().to_string()
            );
        }

        #[tokio::test]
        async fn a_dead_overlay_is_restarted_while_the_game_is_up() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("eqlog_Dorsk_erudin.txt");
            std::fs::write(&log, "").unwrap();
            let runs = dir.path().join("runs.txt");
            let mut supervisor = supervisor(script(
                dir.path(),
                "overlay",
                &format!("echo x >> {}", runs.display()),
            ));

            supervisor.tick(true, Some(&log)).await;
            wait(&mut supervisor).await;

            supervisor.tick(true, Some(&log)).await;
            assert_eq!(supervisor.running(), 0, "the restart is held off");
            assert!(supervisor.entries[0].retry_at.is_some());

            supervisor.entries[0].retry_at = None;
            supervisor.tick(true, Some(&log)).await;
            wait(&mut supervisor).await;
            assert_eq!(std::fs::read_to_string(&runs).unwrap().lines().count(), 2);
        }

        #[tokio::test]
        async fn a_crash_loop_backs_off_further_every_time() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("eqlog_Dorsk_erudin.txt");
            std::fs::write(&log, "").unwrap();
            let mut supervisor = supervisor(Runner::Frozen(dir.path().join("not-installed")));

            let mut delays = Vec::new();
            for _ in 0..3 {
                supervisor.tick(true, Some(&log)).await;
                delays.push(supervisor.entries[0].backoff.delay());
                supervisor.entries[0].retry_at = None;
            }
            assert_eq!(
                delays,
                vec![
                    Duration::from_secs(10),
                    Duration::from_secs(20),
                    Duration::from_secs(40)
                ]
            );

            supervisor.tick(false, None).await;
            assert_eq!(
                supervisor.entries[0].backoff.delay(),
                RESTART_BASE,
                "the game going away clears the crash history"
            );
        }

        #[tokio::test]
        async fn switching_character_repoints_the_overlays_at_the_new_log() {
            let dir = tempfile::tempdir().unwrap();
            let first = dir.path().join("eqlog_Dorsk_erudin.txt");
            let second = dir.path().join("eqlog_Vala_erudin.txt");
            std::fs::write(&first, "").unwrap();
            std::fs::write(&second, "").unwrap();
            let mut supervisor = supervisor(script(dir.path(), "overlay", "sleep 30"));

            supervisor.tick(true, Some(&first)).await;
            let before = supervisor.entries[0].running.as_ref().unwrap().child.id();

            supervisor.tick(true, Some(&second)).await;
            let after = supervisor.entries[0].running.as_ref().unwrap().child.id();
            assert_ne!(before, after, "a fresh process watches the new log");
            assert_eq!(supervisor.watching.as_deref(), Some(second.as_path()));
        }

        #[tokio::test]
        async fn a_missing_log_leaves_a_running_overlay_alone() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("eqlog_Dorsk_erudin.txt");
            std::fs::write(&log, "").unwrap();
            let mut supervisor = supervisor(script(dir.path(), "overlay", "sleep 30"));

            supervisor.tick(true, Some(&log)).await;
            supervisor.tick(true, None).await;
            assert_eq!(supervisor.running(), 1);
        }
    }
}
