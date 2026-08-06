use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "eqld.lock";

pub fn default_path(state_path: &Path) -> PathBuf {
    state_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.join(FILE_NAME))
        .unwrap_or_else(|| PathBuf::from(FILE_NAME))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holder {
    pub pid: u32,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Held,
    Stale,
}

pub fn process_name(pid: u32) -> Option<String> {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    let process = system.process(pid)?;
    Some(process.name().to_string_lossy().into_owned())
}

fn stem(name: &str) -> String {
    name.trim()
        .trim_end_matches(".exe")
        .trim_end_matches(".EXE")
        .to_ascii_lowercase()
}

pub fn judge(holder: &Holder, running: Option<&str>) -> Verdict {
    match running {
        Some(found) if stem(found) == stem(&holder.name) => Verdict::Held,
        _ => Verdict::Stale,
    }
}

fn me() -> Holder {
    let pid = std::process::id();
    Holder {
        pid,
        name: process_name(pid).unwrap_or_else(|| "eqld".to_string()),
    }
}

#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
    holder: Holder,
}

impl Lock {
    /// `force` takes the lock even from a live holder, for recovering a rig
    /// where the pid was reused by another eqld that is not ours to stop.
    pub fn acquire(path: &Path, force: bool) -> Result<Self, LockError> {
        let holder = me();
        let body = serde_json::to_vec(&holder).map_err(LockError::Encode)?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|source| LockError::Io(parent.to_path_buf(), source))?;
        }

        for _ in 0..3 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    file.write_all(&body)
                        .map_err(|source| LockError::Io(path.to_path_buf(), source))?;
                    return Ok(Self {
                        path: path.to_path_buf(),
                        holder,
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(err) => return Err(LockError::Io(path.to_path_buf(), err)),
            }

            let existing = std::fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Holder>(&bytes).ok());
            match &existing {
                Some(existing) if !force => {
                    let running = process_name(existing.pid);
                    if judge(existing, running.as_deref()) == Verdict::Held {
                        return Err(LockError::Held {
                            path: path.to_path_buf(),
                            pid: existing.pid,
                            name: existing.name.clone(),
                        });
                    }
                    tracing::warn!(
                        path = %path.display(),
                        pid = existing.pid,
                        "the lock is stale, taking it over"
                    );
                }
                Some(existing) => tracing::warn!(
                    path = %path.display(),
                    pid = existing.pid,
                    "--force: taking the lock from its holder"
                ),
                None => tracing::warn!(
                    path = %path.display(),
                    "the lock file is unreadable, taking it over"
                ),
            }
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(LockError::Io(path.to_path_buf(), err)),
            }
        }
        Err(LockError::Contended(path.to_path_buf()))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn holder(&self) -> &Holder {
        &self.holder
    }

    /// Only removes the file while it still names us, so a lock already taken
    /// over by another instance survives this one's exit.
    pub fn release(&self) {
        let ours = std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Holder>(&bytes).ok())
            .is_some_and(|found| found.pid == self.holder.pid);
        if !ours {
            return;
        }
        if let Err(err) = std::fs::remove_file(&self.path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = %self.path.display(), %err, "cannot release the lock");
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error(
        "another eqld is already running as pid {pid} ({name}); \
         stop it, or start with --force to take {path} from it"
    )]
    Held {
        path: PathBuf,
        pid: u32,
        name: String,
    },
    #[error("{0} keeps being retaken by another instance; nothing was started")]
    Contended(PathBuf),
    #[error("encoding the lock: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("{0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dead_pid() -> u32 {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--this-test-does-not-exist")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut child = child;
        child.wait().unwrap();
        pid
    }

    #[test]
    fn the_lock_sits_beside_the_state_file() {
        assert_eq!(
            default_path(Path::new("/var/lib/eqld/state.json")),
            PathBuf::from("/var/lib/eqld/eqld.lock")
        );
        assert_eq!(
            default_path(Path::new("state.json")),
            PathBuf::from("eqld.lock")
        );
    }

    #[test]
    fn a_holder_is_only_believed_while_its_pid_is_still_that_program() {
        let holder = Holder {
            pid: 42,
            name: "eqld.exe".into(),
        };
        assert_eq!(judge(&holder, Some("eqld.exe")), Verdict::Held);
        assert_eq!(
            judge(&holder, Some("eqld")),
            Verdict::Held,
            "the same daemon built for another platform still holds it"
        );
        assert_eq!(
            judge(&holder, Some("eqgame.exe")),
            Verdict::Stale,
            "the pid was reused by something else"
        );
        assert_eq!(judge(&holder, None), Verdict::Stale, "the pid is gone");
    }

    #[test]
    fn a_second_instance_is_refused_and_told_which_pid_holds_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let held = Lock::acquire(&path, false).unwrap();
        assert!(path.exists());
        assert_eq!(held.holder().pid, std::process::id());

        let refused = Lock::acquire(&path, false).unwrap_err();
        match &refused {
            LockError::Held { pid, .. } => assert_eq!(*pid, std::process::id()),
            other => panic!("{other}"),
        }
        assert!(
            refused
                .to_string()
                .contains(&std::process::id().to_string()),
            "{refused}"
        );
        assert!(path.exists(), "a refusal never removes the live lock");
    }

    #[test]
    fn a_lock_left_by_a_crashed_process_does_not_block_a_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let gone = Holder {
            pid: dead_pid(),
            name: "eqld".into(),
        };
        std::fs::write(&path, serde_json::to_vec(&gone).unwrap()).unwrap();

        let taken = Lock::acquire(&path, false).unwrap();
        assert_eq!(taken.holder().pid, std::process::id());
    }

    #[test]
    fn a_pid_that_now_belongs_to_something_else_does_not_block_a_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(
            &path,
            serde_json::to_vec(&Holder {
                pid: std::process::id(),
                name: "eqgame.exe".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(Lock::acquire(&path, false).is_ok());
    }

    #[test]
    fn a_truncated_lock_file_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, b"{").unwrap();
        assert!(Lock::acquire(&path, false).is_ok());
    }

    #[test]
    fn force_takes_the_lock_from_a_live_holder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let _held = Lock::acquire(&path, false).unwrap();
        assert!(Lock::acquire(&path, true).is_ok());
    }

    #[test]
    fn the_lock_is_released_when_the_daemon_exits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        {
            let _held = Lock::acquire(&path, false).unwrap();
            assert!(path.exists());
        }
        assert!(!path.exists(), "a normal exit releases it");
        assert!(
            Lock::acquire(&path, false).is_ok(),
            "and the next start is not refused"
        );
    }

    #[test]
    fn a_lock_already_taken_over_is_not_removed_by_the_instance_that_lost_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let lost = Lock::acquire(&path, false).unwrap();
        std::fs::write(
            &path,
            serde_json::to_vec(&Holder {
                pid: lost.holder().pid + 1,
                name: "eqld".into(),
            })
            .unwrap(),
        )
        .unwrap();
        drop(lost);
        assert!(path.exists());
    }

    #[test]
    fn the_directory_is_created_if_it_is_not_there_yet() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(FILE_NAME);
        let _held = Lock::acquire(&path, false).unwrap();
        assert!(path.exists());
    }
}
