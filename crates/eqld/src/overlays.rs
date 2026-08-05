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

    /// The tool that does this overlay's work with no window at all. Only the
    /// DPS meter has one; the rest are windows and nothing else.
    pub fn headless_stem(&self) -> Option<&'static str> {
        match self {
            Overlay::Dps => Some(crate::tools::HEADLESS_STEM),
            Overlay::SessionReport | Overlay::Friend | Overlay::Atlas => None,
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
        "the atlas overlay has no headless build to run instead, and hidden it is only a map \
         nobody can read; track quests with `eql_atlas --quest <log> add` instead"
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
        AtlasMode::Replay => {
            "the --replay tick keeps the atlas database and credits the quests already tracked"
        }
        AtlasMode::Overlay => {
            "the atlas overlay keeps its own database; quests accrue for the ones you track in it"
        }
    }
}

/// How long a headless tool gets to run its final save after the console break
/// before it is killed outright.
#[cfg(windows)]
const BREAK_GRACE: Duration = Duration::from_secs(10);

enum Handle {
    Child {
        child: Box<tokio::process::Child>,
        /// Its own process group, so it can be sent a console break.
        group: bool,
    },
    #[cfg(windows)]
    Hidden(crate::hidden::Process),
}

impl Handle {
    fn id(&self) -> Option<u32> {
        match self {
            Handle::Child { child, .. } => child.id(),
            #[cfg(windows)]
            Handle::Hidden(process) => Some(process.id()),
        }
    }

    /// `Ok(None)` while it is still up; the inner option is the exit code,
    /// absent when a signal took it.
    fn try_wait(&mut self) -> std::io::Result<Option<Option<i32>>> {
        match self {
            Handle::Child { child, .. } => Ok(child.try_wait()?.map(|status| status.code())),
            #[cfg(windows)]
            Handle::Hidden(process) => Ok(process.try_wait()?.map(Some)),
        }
    }

    async fn stop(&mut self) {
        match self {
            #[cfg_attr(not(windows), allow(unused_variables))]
            Handle::Child { child, group } => {
                #[cfg(windows)]
                if *group {
                    if let Some(pid) = child.id() {
                        match tokio::task::spawn_blocking(move || crate::ctrl::break_group(pid))
                            .await
                        {
                            Ok(Ok(())) => {
                                if tokio::time::timeout(BREAK_GRACE, child.wait())
                                    .await
                                    .is_ok()
                                {
                                    return;
                                }
                                tracing::warn!(
                                    pid,
                                    "the headless tool is still saving, killing it"
                                );
                            }
                            Ok(Err(err)) => tracing::warn!(
                                pid,
                                %err,
                                "cannot raise a console break, the last stats are lost"
                            ),
                            Err(err) => tracing::warn!(pid, %err, "console break task failed"),
                        }
                    }
                }
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
    headless: bool,
    running: Option<Running>,
    backoff: Backoff,
    retry_at: Option<Instant>,
}

impl Entry {
    fn new(overlay: Overlay, (runner, headless): (Runner, bool), hidden: bool) -> Self {
        Self {
            overlay,
            runner,
            hidden,
            headless,
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
        match spawn(
            &self.runner,
            log,
            self.hidden && !self.headless,
            self.headless,
        ) {
            Ok(child) => {
                tracing::info!(
                    overlay = self.overlay.name(),
                    pid = child.id(),
                    hidden = self.hidden && (self.headless || hiding_is_supported()),
                    headless = self.headless,
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

/// `headless` gets its own process group: Windows delivers no SIGTERM, so the
/// only way to let the tool run its final save is a console break, and that is
/// addressed to a group.
fn spawn(
    runner: &Runner,
    log: &Path,
    hidden_desktop: bool,
    headless: bool,
) -> std::io::Result<Handle> {
    let args = runner.overlay_args(log);
    let dir = runner.program().parent();
    #[cfg(windows)]
    if hidden_desktop {
        return crate::hidden::spawn(runner.program(), &args, dir).map(Handle::Hidden);
    }
    #[cfg(not(windows))]
    let _ = hidden_desktop;
    let mut command = tokio::process::Command::new(runner.program());
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    #[cfg(windows)]
    if headless {
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    command.spawn().map(|child| Handle::Child {
        child: Box::new(child),
        group: headless,
    })
}

/// A hidden overlay prefers the tool that opens no window at all; an install
/// without it (stock upstream) falls back to the isolated desktop.
fn resolve(base: &Runner, overlay: Overlay, hidden: bool) -> Option<(Runner, bool)> {
    if hidden {
        if let Some(runner) = overlay.headless_stem().and_then(|stem| base.sibling(stem)) {
            return Some((runner, true));
        }
    }
    base.sibling(overlay.stem()).map(|runner| (runner, false))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Changes {
    pub enabled: Vec<&'static str>,
    pub disabled: Vec<&'static str>,
    pub moved: Vec<&'static str>,
    pub missing: Vec<&'static str>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty()
            && self.disabled.is_empty()
            && self.moved.is_empty()
            && self.missing.is_empty()
    }
}

#[derive(Default)]
pub struct Supervisor {
    entries: Vec<Entry>,
    watching: Option<PathBuf>,
}

impl Supervisor {
    pub fn new(base: &Runner, overlays: &[Overlay], hidden: &[Overlay]) -> Self {
        let mut entries = Vec::new();
        for overlay in overlays {
            let hide = hidden.contains(overlay);
            match resolve(base, *overlay, hide) {
                Some(found) => entries.push(Entry::new(*overlay, found, hide)),
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

    /// An overlay that is still wanted, still installed and still on the same
    /// desktop keeps the process it already has.
    pub async fn reconcile(
        &mut self,
        base: Option<&Runner>,
        wanted: &[Overlay],
        hidden: &[Overlay],
    ) -> Changes {
        let mut changes = Changes::default();
        let mut previous = std::mem::take(&mut self.entries);
        let mut kept = Vec::with_capacity(wanted.len());
        for overlay in wanted {
            let hide = hidden.contains(overlay);
            match previous.iter().position(|entry| entry.overlay == *overlay) {
                Some(at) => {
                    let mut entry = previous.remove(at);
                    if entry.hidden != hide {
                        entry.stop().await;
                        entry.hidden = hide;
                        if let Some((runner, headless)) =
                            base.and_then(|base| resolve(base, *overlay, hide))
                        {
                            entry.runner = runner;
                            entry.headless = headless;
                        }
                        changes.moved.push(overlay.name());
                        tracing::info!(
                            overlay = overlay.name(),
                            hidden = hide,
                            headless = entry.headless,
                            "overlay changed how it runs, relaunching"
                        );
                    }
                    kept.push(entry);
                }
                None => match base.and_then(|base| resolve(base, *overlay, hide)) {
                    Some(found) => {
                        let headless = found.1;
                        kept.push(Entry::new(*overlay, found, hide));
                        changes.enabled.push(overlay.name());
                        tracing::info!(
                            overlay = overlay.name(),
                            hidden = hide,
                            headless,
                            "overlay enabled"
                        );
                    }
                    None => {
                        changes.missing.push(overlay.name());
                        tracing::warn!(
                            overlay = overlay.name(),
                            expected = overlay.stem(),
                            "overlay is not installed, it will not be launched"
                        );
                    }
                },
            }
        }
        for mut entry in previous {
            entry.stop().await;
            changes.disabled.push(entry.overlay.name());
            tracing::info!(overlay = entry.overlay.name(), "overlay disabled");
        }
        self.entries = kept;
        if self.entries.is_empty() {
            self.watching = None;
        }
        changes
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

    pub fn headless(&self) -> Vec<&'static str> {
        self.entries
            .iter()
            .filter(|entry| entry.headless)
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
        if self.entries.is_empty() {
            self.watching = None;
            return;
        }
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
    fn only_the_dps_meter_can_run_without_a_window_of_its_own() {
        assert_eq!(Overlay::Dps.headless_stem(), Some("eql_headless"));
        for overlay in [Overlay::SessionReport, Overlay::Friend, Overlay::Atlas] {
            assert_eq!(
                overlay.headless_stem(),
                None,
                "{} is a window and nothing else, so it still needs a desktop to hide on",
                overlay.name()
            );
        }
    }

    fn suite_with(dir: &Path, stems: &[&str]) -> Runner {
        for stem in stems {
            std::fs::write(dir.join(format!("{stem}.exe")), b"").unwrap();
        }
        Runner::Frozen(dir.join("eql_atlas.exe"))
    }

    #[test]
    fn a_hidden_dps_runs_the_headless_tool_instead_of_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let base = suite_with(
            dir.path(),
            &[
                "eql_atlas",
                "eql_dps_meter",
                "eql_headless",
                "eql_friend_overlay",
            ],
        );

        let supervisor = Supervisor::new(&base, &[Overlay::Dps, Overlay::Friend], &[Overlay::Dps]);
        assert_eq!(supervisor.headless(), vec!["dps"]);
        assert_eq!(supervisor.hidden(), vec!["dps"]);
        assert_eq!(
            supervisor.entries[0].runner.program(),
            dir.path().join("eql_headless.exe")
        );
        assert_eq!(
            supervisor.entries[1].runner.program(),
            dir.path().join("eql_friend_overlay.exe"),
            "an overlay that is not hidden is untouched"
        );
    }

    #[test]
    fn a_visible_dps_is_the_window_even_where_the_headless_tool_exists() {
        let dir = tempfile::tempdir().unwrap();
        let base = suite_with(dir.path(), &["eql_atlas", "eql_dps_meter", "eql_headless"]);
        let supervisor = Supervisor::new(&base, &[Overlay::Dps], &[]);
        assert!(supervisor.headless().is_empty());
        assert_eq!(
            supervisor.entries[0].runner.program(),
            dir.path().join("eql_dps_meter.exe")
        );
    }

    #[test]
    fn a_stock_upstream_install_still_hides_the_meter_on_a_desktop() {
        let dir = tempfile::tempdir().unwrap();
        let base = suite_with(dir.path(), &["eql_atlas", "eql_dps_meter"]);
        let supervisor = Supervisor::new(&base, &[Overlay::Dps], &[Overlay::Dps]);
        assert_eq!(supervisor.hidden(), vec!["dps"]);
        assert!(
            supervisor.headless().is_empty(),
            "there is no eql_headless to run"
        );
        assert_eq!(
            supervisor.entries[0].runner.program(),
            dir.path().join("eql_dps_meter.exe")
        );
    }

    #[tokio::test]
    async fn hiding_a_running_meter_swaps_it_for_the_headless_tool() {
        let dir = tempfile::tempdir().unwrap();
        let base = suite_with(dir.path(), &["eql_atlas", "eql_dps_meter", "eql_headless"]);

        let mut supervisor = Supervisor::new(&base, &[Overlay::Dps], &[]);
        let changes = supervisor
            .reconcile(Some(&base), &[Overlay::Dps], &[Overlay::Dps])
            .await;
        assert_eq!(changes.moved, vec!["dps"]);
        assert_eq!(supervisor.headless(), vec!["dps"]);
        assert_eq!(
            supervisor.entries[0].runner.program(),
            dir.path().join("eql_headless.exe")
        );

        let changes = supervisor
            .reconcile(Some(&base), &[Overlay::Dps], &[])
            .await;
        assert_eq!(changes.moved, vec!["dps"]);
        assert!(supervisor.headless().is_empty());
        assert_eq!(
            supervisor.entries[0].runner.program(),
            dir.path().join("eql_dps_meter.exe"),
            "unhiding it brings the window back"
        );
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

        /// Interpreted, not executed: a sibling test forking while this file is
        /// still open for writing fails the exec with ETXTBSY.
        fn script(dir: &Path, name: &str, body: &str) -> Runner {
            let path = dir.join(name);
            std::fs::write(&path, format!("{body}\n")).unwrap();
            Runner::Source {
                python: PathBuf::from("/bin/sh"),
                script: path,
            }
        }

        fn supervisor(runner: Runner) -> Supervisor {
            Supervisor {
                entries: vec![Entry::new(Overlay::Dps, (runner, false), false)],
                watching: None,
            }
        }

        async fn wait(supervisor: &mut Supervisor) {
            match &mut supervisor.entries[0].running.as_mut().unwrap().child {
                Handle::Child { child, .. } => {
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
                entries: vec![Entry::new(Overlay::Dps, (runner, true), true)],
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

        fn suite(dir: &Path) -> Runner {
            for stem in KNOWN.map(|overlay| overlay.stem()) {
                std::fs::write(dir.join(format!("{stem}.py")), "").unwrap();
            }
            Runner::Source {
                python: PathBuf::from("/bin/sh"),
                script: dir.join(format!("{}.py", Overlay::Atlas.stem())),
            }
        }

        #[tokio::test]
        async fn enabling_an_overlay_adds_it_without_touching_the_others() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("eqlog_Dorsk_erudin.txt");
            std::fs::write(&log, "").unwrap();
            let base = suite(dir.path());
            std::fs::write(dir.path().join("eql_dps_meter.py"), "sleep 30\n").unwrap();
            std::fs::write(dir.path().join("eql_friend_overlay.py"), "sleep 30\n").unwrap();

            let mut supervisor = Supervisor::new(&base, &[Overlay::Dps], &[]);
            supervisor.tick(true, Some(&log)).await;
            let before = supervisor.entries[0].running.as_ref().unwrap().child.id();

            let changes = supervisor
                .reconcile(Some(&base), &[Overlay::Dps, Overlay::Friend], &[])
                .await;
            assert_eq!(changes.enabled, vec!["friend"]);
            assert!(changes.disabled.is_empty());
            assert_eq!(supervisor.names(), vec!["dps", "friend"]);
            assert_eq!(
                supervisor.entries[0].running.as_ref().unwrap().child.id(),
                before,
                "the overlay that was already running is left alone"
            );

            supervisor.tick(true, Some(&log)).await;
            assert_eq!(supervisor.running(), 2);
            supervisor.stop().await;
        }

        #[tokio::test]
        async fn disabling_an_overlay_stops_it_and_leaves_the_rest_running() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("eqlog_Dorsk_erudin.txt");
            std::fs::write(&log, "").unwrap();
            let base = suite(dir.path());
            std::fs::write(dir.path().join("eql_dps_meter.py"), "sleep 30\n").unwrap();
            std::fs::write(dir.path().join("eql_friend_overlay.py"), "sleep 30\n").unwrap();

            let mut supervisor = Supervisor::new(&base, &[Overlay::Dps, Overlay::Friend], &[]);
            supervisor.tick(true, Some(&log)).await;
            assert_eq!(supervisor.running(), 2);
            let kept = supervisor.entries[0].running.as_ref().unwrap().child.id();

            let changes = supervisor
                .reconcile(Some(&base), &[Overlay::Dps], &[])
                .await;
            assert_eq!(changes.disabled, vec!["friend"]);
            assert!(changes.enabled.is_empty());
            assert_eq!(supervisor.names(), vec!["dps"]);
            assert_eq!(supervisor.running(), 1);
            assert_eq!(
                supervisor.entries[0].running.as_ref().unwrap().child.id(),
                kept
            );
            supervisor.stop().await;
        }

        #[tokio::test]
        async fn reconciling_the_same_plan_changes_nothing() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("eqlog_Dorsk_erudin.txt");
            std::fs::write(&log, "").unwrap();
            let base = suite(dir.path());
            std::fs::write(dir.path().join("eql_dps_meter.py"), "sleep 30\n").unwrap();

            let mut supervisor = Supervisor::new(&base, &[Overlay::Dps], &[]);
            supervisor.tick(true, Some(&log)).await;
            let before = supervisor.entries[0].running.as_ref().unwrap().child.id();

            let changes = supervisor
                .reconcile(Some(&base), &[Overlay::Dps], &[])
                .await;
            assert!(changes.is_empty());
            assert_eq!(
                supervisor.entries[0].running.as_ref().unwrap().child.id(),
                before,
                "a no-op reload never restarts anything"
            );
            supervisor.stop().await;
        }

        #[tokio::test]
        async fn hiding_a_running_overlay_relaunches_it_on_the_other_desktop() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("eqlog_Dorsk_erudin.txt");
            std::fs::write(&log, "").unwrap();
            let base = suite(dir.path());
            std::fs::write(dir.path().join("eql_dps_meter.py"), "sleep 30\n").unwrap();

            let mut supervisor = Supervisor::new(&base, &[Overlay::Dps], &[]);
            supervisor.tick(true, Some(&log)).await;
            let before = supervisor.entries[0].running.as_ref().unwrap().child.id();

            let changes = supervisor
                .reconcile(Some(&base), &[Overlay::Dps], &[Overlay::Dps])
                .await;
            assert_eq!(changes.moved, vec!["dps"]);
            assert_eq!(supervisor.hidden(), vec!["dps"]);
            assert_eq!(supervisor.running(), 0);

            supervisor.tick(true, Some(&log)).await;
            assert_ne!(
                supervisor.entries[0].running.as_ref().unwrap().child.id(),
                before
            );
            supervisor.stop().await;
        }

        #[tokio::test]
        async fn an_overlay_that_is_not_installed_is_reported_not_added() {
            let dir = tempfile::tempdir().unwrap();
            let base = suite(dir.path());
            std::fs::remove_file(dir.path().join("eql_friend_overlay.py")).unwrap();

            let mut supervisor = Supervisor::default();
            let changes = supervisor
                .reconcile(Some(&base), &[Overlay::Friend], &[])
                .await;
            assert_eq!(changes.missing, vec!["friend"]);
            assert!(changes.enabled.is_empty());
            assert!(supervisor.is_empty());

            let changes = supervisor.reconcile(None, &[Overlay::Dps], &[]).await;
            assert_eq!(
                changes.missing,
                vec!["dps"],
                "without a log reader nothing can be started either"
            );
        }

        #[tokio::test]
        async fn overlays_are_still_stopped_when_the_log_reader_went_away() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("eqlog_Dorsk_erudin.txt");
            std::fs::write(&log, "").unwrap();
            let base = suite(dir.path());
            std::fs::write(dir.path().join("eql_dps_meter.py"), "sleep 30\n").unwrap();

            let mut supervisor = Supervisor::new(&base, &[Overlay::Dps], &[]);
            supervisor.tick(true, Some(&log)).await;
            assert_eq!(supervisor.running(), 1);

            let changes = supervisor.reconcile(None, &[], &[]).await;
            assert_eq!(changes.disabled, vec!["dps"]);
            assert!(supervisor.is_empty());
        }

        #[tokio::test]
        async fn an_emptied_plan_stops_everything_and_forgets_the_log() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("eqlog_Dorsk_erudin.txt");
            std::fs::write(&log, "").unwrap();
            let base = suite(dir.path());
            std::fs::write(dir.path().join("eql_dps_meter.py"), "sleep 30\n").unwrap();

            let mut supervisor = Supervisor::new(&base, &[Overlay::Dps], &[]);
            supervisor.tick(true, Some(&log)).await;
            assert_eq!(supervisor.running(), 1);

            let changes = supervisor.reconcile(Some(&base), &[], &[]).await;
            assert_eq!(changes.disabled, vec!["dps"]);
            assert!(supervisor.is_empty());
            assert!(supervisor.watching.is_none());

            supervisor.tick(true, Some(&log)).await;
            assert_eq!(supervisor.running(), 0);
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
