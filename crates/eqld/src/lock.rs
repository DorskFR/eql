use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

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

fn process_name(pid: u32) -> Option<String> {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    let process = system.process(pid)?;
    Some(process.name().to_string_lossy().into_owned())
}

fn me() -> Holder {
    let pid = std::process::id();
    Holder {
        pid,
        name: process_name(pid).unwrap_or_else(|| "eqld".to_string()),
    }
}

/// An advisory lock held on the file for the process lifetime: the kernel
/// drops it when the process dies, whatever its PID namespace. The holder
/// written into the file is only there to name it in a refusal.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
    holder: Holder,
    file: File,
}

impl Lock {
    /// `force` takes the path from a live holder by replacing the file, for a
    /// rig whose holder is not ours to stop.
    pub fn acquire(path: &Path, force: bool) -> Result<Self, LockError> {
        let io = |source| LockError::Io(path.to_path_buf(), source);
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
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)
                .map_err(io)?;
            match file.try_lock() {
                Ok(()) => {
                    if !is_at(path, &file) {
                        continue;
                    }
                    if let Some(previous) = read_holder(&file) {
                        tracing::warn!(
                            path = %path.display(),
                            pid = previous.pid,
                            "the lock is stale, taking it over"
                        );
                    }
                    write_holder(&file, &body).map_err(io)?;
                    return Ok(Self {
                        path: path.to_path_buf(),
                        holder,
                        file,
                    });
                }
                Err(TryLockError::WouldBlock) => {
                    if !is_at(path, &file) {
                        continue;
                    }
                    let existing = std::fs::File::open(path)
                        .ok()
                        .and_then(|file| read_holder(&file));
                    if !force {
                        return Err(LockError::Held {
                            path: path.to_path_buf(),
                            holder: existing,
                        });
                    }
                    tracing::warn!(
                        path = %path.display(),
                        pid = ?existing.map(|holder| holder.pid),
                        "--force: taking the lock from its holder"
                    );
                    match std::fs::remove_file(path) {
                        Ok(()) => {}
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                        Err(err) => return Err(io(err)),
                    }
                }
                Err(TryLockError::Error(err)) => return Err(io(err)),
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
}

impl Drop for Lock {
    /// Removes the file only while the path is still ours, so a lock taken
    /// over with `--force` survives this one's exit. Windows keeps the file:
    /// the lock, not the file, is what counts.
    fn drop(&mut self) {
        if cfg!(unix) && is_at(&self.path, &self.file) {
            if let Err(err) = std::fs::remove_file(&self.path) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(path = %self.path.display(), %err, "cannot release the lock");
                }
            }
        }
    }
}

fn read_holder(file: &File) -> Option<Holder> {
    let mut bytes = Vec::new();
    let mut reader = file;
    reader.seek(SeekFrom::Start(0)).ok()?;
    reader.read_to_end(&mut bytes).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_holder(file: &File, body: &[u8]) -> std::io::Result<()> {
    file.set_len(0)?;
    let mut writer = file;
    writer.seek(SeekFrom::Start(0))?;
    writer.write_all(body)?;
    writer.flush()
}

/// Whether `path` still names the file we opened, rather than one that
/// replaced it after a `--force` or a release.
#[cfg(unix)]
fn is_at(path: &Path, file: &File) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(path), file.metadata()) {
        (Ok(at_path), Ok(ours)) => at_path.dev() == ours.dev() && at_path.ino() == ours.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_at(path: &Path, _file: &File) -> bool {
    path.exists()
}

fn describe(holder: &Option<Holder>) -> String {
    holder
        .as_ref()
        .map(|holder| format!(" as pid {} ({})", holder.pid, holder.name))
        .unwrap_or_default()
}

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error(
        "another eqld is already running{}; \
         stop it, or start with --force to take {path} from it",
        describe(.holder)
    )]
    Held {
        path: PathBuf,
        holder: Option<Holder>,
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
    fn a_second_instance_is_refused_and_told_which_pid_holds_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let held = Lock::acquire(&path, false).unwrap();
        assert!(path.exists());
        assert_eq!(held.holder().pid, std::process::id());

        let refused = Lock::acquire(&path, false).unwrap_err();
        match &refused {
            LockError::Held {
                holder: Some(holder),
                ..
            } => assert_eq!(holder.pid, std::process::id()),
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
    fn a_lock_naming_our_own_pid_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, serde_json::to_vec(&me()).unwrap()).unwrap();
        let taken = Lock::acquire(&path, false).unwrap();
        assert_eq!(taken.holder().pid, std::process::id());
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
        if cfg!(unix) {
            assert!(!path.exists(), "a normal exit removes it");
        }
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
        let winner = Lock::acquire(&path, true).unwrap();
        drop(lost);
        assert!(path.exists());
        assert!(
            matches!(Lock::acquire(&path, false), Err(LockError::Held { .. })),
            "the winner still holds it"
        );
        drop(winner);
        assert!(Lock::acquire(&path, false).is_ok());
    }

    #[test]
    fn a_lock_whose_holder_died_without_cleaning_up_does_not_block_a_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        {
            let crashed = File::create(&path).unwrap();
            crashed.try_lock().unwrap();
            write_holder(&crashed, &serde_json::to_vec(&me()).unwrap()).unwrap();
        }
        assert!(path.exists(), "nothing cleaned the file up");
        assert!(Lock::acquire(&path, false).is_ok());
    }

    #[test]
    fn the_directory_is_created_if_it_is_not_there_yet() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(FILE_NAME);
        let _held = Lock::acquire(&path, false).unwrap();
        assert!(path.exists());
    }
}
