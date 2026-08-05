use crate::config::LogReaderConfig;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

pub const REPO: &str = "blastlaster/eql-log-reader";
pub const DEFAULT_VERSION: &str = "v2.0";

/// The Atlas is the only tool upstream exposes headlessly (`--replay`, no UI);
/// the DPS meter and quest credit still need its GUI.
pub const ATLAS_STEM: &str = "eql_atlas";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Runner {
    Frozen(PathBuf),
    Source { python: PathBuf, script: PathBuf },
}

impl Runner {
    pub fn program(&self) -> &Path {
        match self {
            Runner::Frozen(exe) => exe,
            Runner::Source { python, .. } => python,
        }
    }

    pub fn replay_args(&self, log: &Path) -> Vec<PathBuf> {
        let mut args = self.leading_args();
        args.push(PathBuf::from("--replay"));
        args.push(log.to_path_buf());
        args
    }

    /// Every GUI tool takes the log path as its only argument; started without
    /// one they pop a file-picker and hang waiting for a human.
    pub fn overlay_args(&self, log: &Path) -> Vec<PathBuf> {
        let mut args = self.leading_args();
        args.push(log.to_path_buf());
        args
    }

    fn leading_args(&self) -> Vec<PathBuf> {
        match self {
            Runner::Source { script, .. } => vec![script.clone()],
            Runner::Frozen(_) => Vec::new(),
        }
    }

    pub fn discover(explicit: Option<&Path>) -> Option<Self> {
        if let Some(path) = explicit {
            return Self::at(path);
        }
        candidates(ATLAS_STEM)
            .into_iter()
            .find_map(|path| Self::at(&path))
    }

    /// Upstream ships every tool of the suite in one directory.
    pub fn sibling(&self, stem: &str) -> Option<Self> {
        match self {
            Runner::Frozen(exe) => {
                let name = match exe.extension().and_then(|ext| ext.to_str()) {
                    Some(ext) => format!("{stem}.{ext}"),
                    None => stem.to_string(),
                };
                let path = exe.with_file_name(name);
                path.is_file().then_some(Runner::Frozen(path))
            }
            Runner::Source { python, script } => {
                let path = script.with_file_name(format!("{stem}.py"));
                path.is_file().then(|| Runner::Source {
                    python: python.clone(),
                    script: path,
                })
            }
        }
    }

    fn at(path: &Path) -> Option<Self> {
        if !path.is_file() {
            return None;
        }
        if path.extension().is_some_and(|ext| ext == "py") {
            return python().map(|python| Runner::Source {
                python,
                script: path.to_path_buf(),
            });
        }
        Some(Runner::Frozen(path.to_path_buf()))
    }
}

fn python() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["python.exe", "python3.exe"]
    } else {
        &["python3", "python"]
    };
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| {
        names
            .iter()
            .map(|name| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

fn candidates(stem: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let exe = format!("{stem}{}", std::env::consts::EXE_SUFFIX);
    if cfg!(windows) {
        for key in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(key) {
                out.push(Path::new(&base).join("EQL Log Reader").join(&exe));
            }
        }
        if let Some(local) = dirs::data_local_dir() {
            out.push(local.join("Programs").join("EQL Log Reader").join(&exe));
        }
    }
    if let Some(data) = dirs::data_dir() {
        let base = data.join("eql-log-reader");
        out.push(base.join(&exe));
        out.push(base.join(format!("{stem}.py")));
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayReport {
    pub ran: usize,
    pub failed: usize,
}

/// Upstream persists its own byte offset, so repeated replays are incremental.
pub async fn replay_all(runner: &Runner, logs: &[PathBuf], timeout: Duration) -> ReplayReport {
    let mut report = ReplayReport { ran: 0, failed: 0 };
    for log in logs {
        match replay(runner, log, timeout).await {
            Ok(()) => report.ran += 1,
            Err(err) => {
                report.failed += 1;
                tracing::warn!(log = %log.display(), %err, "log-reader replay failed");
            }
        }
    }
    report
}

pub async fn replay(runner: &Runner, log: &Path, timeout: Duration) -> Result<(), ReplayError> {
    let started = Instant::now();
    let mut command = tokio::process::Command::new(runner.program());
    command
        .args(runner.replay_args(log))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(ReplayError::Spawn)?;
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => status.map_err(ReplayError::Wait)?,
        Err(_) => {
            let _ = child.start_kill();
            return Err(ReplayError::TimedOut(timeout));
        }
    };
    if !status.success() {
        return Err(ReplayError::Exit(status.code()));
    }
    tracing::debug!(log = %log.display(), ms = started.elapsed().as_millis(), "replayed");
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("spawning the log reader: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("waiting on the log reader: {0}")]
    Wait(#[source] std::io::Error),
    #[error("log reader exited with {0:?}")]
    Exit(Option<i32>),
    #[error("log reader still running after {0:?}")]
    TimedOut(Duration),
}

/// The release asset for the host platform. Windows ships an Inno installer,
/// Linux a per-user tarball; macOS has no package upstream.
pub fn asset_name(version: &str) -> Option<String> {
    let version = version.trim_start_matches('v');
    if cfg!(windows) {
        Some("EQL-Log-Reader-Setup.exe".to_string())
    } else if cfg!(target_os = "linux") {
        Some(format!("eql-log-reader-{version}-linux.tar.gz"))
    } else {
        None
    }
}

pub fn release_api_url(version: &str) -> String {
    format!("https://api.github.com/repos/{REPO}/releases/tags/{version}")
}

pub fn install_hint(config: &LogReaderConfig) -> String {
    format!(
        "run `eqld <config.toml> install-tools` (upstream {REPO} {})",
        config.version()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_and_source_runners_build_the_replay_command() {
        let frozen = Runner::Frozen(PathBuf::from("/opt/eql/eql_atlas.exe"));
        assert_eq!(frozen.program(), Path::new("/opt/eql/eql_atlas.exe"));
        assert_eq!(
            frozen.replay_args(Path::new("/logs/eqlog_Dorsk_erudin.txt")),
            vec![
                PathBuf::from("--replay"),
                PathBuf::from("/logs/eqlog_Dorsk_erudin.txt")
            ]
        );

        let source = Runner::Source {
            python: PathBuf::from("/usr/bin/python3"),
            script: PathBuf::from("/src/eql_atlas.py"),
        };
        assert_eq!(source.program(), Path::new("/usr/bin/python3"));
        assert_eq!(
            source.replay_args(Path::new("/logs/a.txt")),
            vec![
                PathBuf::from("/src/eql_atlas.py"),
                PathBuf::from("--replay"),
                PathBuf::from("/logs/a.txt")
            ],
            "the script comes before the flag"
        );
    }

    #[test]
    fn an_overlay_is_handed_the_log_and_nothing_else() {
        let frozen = Runner::Frozen(PathBuf::from("/opt/eql/eql_dps_meter.exe"));
        assert_eq!(
            frozen.overlay_args(Path::new("/logs/a.txt")),
            vec![PathBuf::from("/logs/a.txt")]
        );

        let source = Runner::Source {
            python: PathBuf::from("/usr/bin/python3"),
            script: PathBuf::from("/src/eql_dps_meter.py"),
        };
        assert_eq!(
            source.overlay_args(Path::new("/logs/a.txt")),
            vec![
                PathBuf::from("/src/eql_dps_meter.py"),
                PathBuf::from("/logs/a.txt")
            ]
        );
    }

    #[test]
    fn siblings_keep_the_shape_of_the_runner_that_found_them() {
        let dir = tempfile::tempdir().unwrap();
        let atlas = dir.path().join("eql_atlas.exe");
        std::fs::write(&atlas, b"").unwrap();
        let meter = dir.path().join("eql_dps_meter.exe");
        std::fs::write(&meter, b"").unwrap();

        let frozen = Runner::Frozen(atlas.clone());
        assert_eq!(frozen.sibling("eql_dps_meter"), Some(Runner::Frozen(meter)));
        assert_eq!(
            frozen.sibling("eql_friend_overlay"),
            None,
            "a tool that is not installed is not a runner"
        );

        let script = dir.path().join("eql_atlas.py");
        std::fs::write(&script, b"").unwrap();
        let report = dir.path().join("eql_session_report.py");
        std::fs::write(&report, b"").unwrap();
        let source = Runner::Source {
            python: PathBuf::from("/usr/bin/python3"),
            script,
        };
        assert_eq!(
            source.sibling("eql_session_report"),
            Some(Runner::Source {
                python: PathBuf::from("/usr/bin/python3"),
                script: report,
            })
        );
        assert_eq!(
            source.sibling("eql_dps_meter"),
            None,
            "the .exe is not a script"
        );
    }

    #[test]
    fn an_extensionless_runner_yields_extensionless_siblings() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("eql_atlas"), b"").unwrap();
        let meter = dir.path().join("eql_dps_meter");
        std::fs::write(&meter, b"").unwrap();
        let frozen = Runner::Frozen(dir.path().join("eql_atlas"));
        assert_eq!(frozen.sibling("eql_dps_meter"), Some(Runner::Frozen(meter)));
    }

    #[test]
    fn discovery_ignores_missing_paths_and_classifies_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Runner::discover(Some(&dir.path().join("nope.exe"))), None);

        let frozen = dir.path().join("eql_atlas.bin");
        std::fs::write(&frozen, b"").unwrap();
        assert_eq!(
            Runner::discover(Some(&frozen)),
            Some(Runner::Frozen(frozen.clone()))
        );

        let script = dir.path().join("eql_atlas.py");
        std::fs::write(&script, b"").unwrap();
        match Runner::discover(Some(&script)) {
            Some(Runner::Source { script: found, .. }) => assert_eq!(found, script),
            other => assert!(
                other.is_none(),
                "a .py path is only a Source runner, got {other:?}"
            ),
        }
    }

    #[test]
    fn asset_name_matches_the_platform_upstream_actually_ships() {
        let asset = asset_name("2.0.1");
        if cfg!(windows) {
            assert_eq!(asset.as_deref(), Some("EQL-Log-Reader-Setup.exe"));
        } else if cfg!(target_os = "linux") {
            assert_eq!(asset.as_deref(), Some("eql-log-reader-2.0.1-linux.tar.gz"));
        } else {
            assert_eq!(asset, None, "no macOS package exists upstream");
        }
        assert_eq!(asset_name("v2.0.1"), asset, "a leading v is tolerated");
    }

    #[test]
    fn release_url_targets_the_pinned_tag() {
        assert_eq!(
            release_api_url("v2.0"),
            "https://api.github.com/repos/blastlaster/eql-log-reader/releases/tags/v2.0"
        );
    }

    #[tokio::test]
    async fn a_failing_replay_is_reported_not_fatal() {
        let runner = Runner::Frozen(PathBuf::from("/nonexistent/eql_atlas"));
        let report = replay_all(
            &runner,
            &[PathBuf::from("/logs/a.txt")],
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(report, ReplayReport { ran: 0, failed: 1 });
    }
}
