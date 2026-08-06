use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const EXE: &str = env!("CARGO_BIN_EXE_eqld");

fn harness(dir: &Path) -> PathBuf {
    let config = dir.join("eqld.toml");
    std::fs::write(
        &config,
        format!(
            r#"
            [game]
            root = {root}
            poll_secs = 1
            [api]
            url = "http://127.0.0.1:1"
            token = "t"
            [state]
            path = {state}
            "#,
            root = toml::Value::from(dir.to_str().unwrap()),
            state = toml::Value::from(dir.join("state.json").to_str().unwrap()),
        ),
    )
    .unwrap();
    config
}

fn daemon(config: &Path, args: &[&str]) -> std::process::Child {
    Command::new(EXE)
        .arg(config)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_for(path: &Path, present: bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while path.exists() != present && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        path.exists(),
        present,
        "timed out waiting for {} to {}exist",
        path.display(),
        if present { "" } else { "stop " }
    );
}

fn try_start(config: &Path, args: &[&str]) -> std::process::Output {
    Command::new(EXE)
        .arg(config)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

#[test]
fn a_second_daemon_refuses_to_start_while_the_first_holds_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let config = harness(dir.path());
    let lock = dir.path().join(eqld::lock::FILE_NAME);

    let mut first = daemon(&config, &[]);
    wait_for(&lock, true);

    let second = try_start(&config, &[]);
    assert!(!second.status.success(), "the second instance kept running");
    let complaint = String::from_utf8_lossy(&second.stderr);
    assert!(
        complaint.contains(&first.id().to_string()),
        "the refusal must name the pid that holds it: {complaint}"
    );
    assert!(complaint.contains("--force"), "{complaint}");

    let holder: eqld::lock::Holder =
        serde_json::from_slice(&std::fs::read(&lock).unwrap()).unwrap();
    assert_eq!(holder.pid, first.id(), "the live lock is untouched");

    first.kill().unwrap();
    first.wait().unwrap();
}

#[test]
fn a_lock_left_behind_by_a_killed_daemon_does_not_block_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let config = harness(dir.path());
    let lock = dir.path().join(eqld::lock::FILE_NAME);

    let mut killed = daemon(&config, &[]);
    wait_for(&lock, true);
    killed.kill().unwrap();
    killed.wait().unwrap();
    assert!(lock.exists(), "a hard kill leaves the file behind");

    let mut next = daemon(&config, &[]);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let holder: Option<eqld::lock::Holder> = std::fs::read(&lock)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        if holder.is_some_and(|holder| holder.pid == next.id()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the stale lock blocked the start"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        next.try_wait().unwrap().is_none(),
        "and it is still running"
    );
    next.kill().unwrap();
    next.wait().unwrap();
}

#[test]
fn force_starts_anyway_and_the_loser_leaves_the_lock_alone() {
    let dir = tempfile::tempdir().unwrap();
    let config = harness(dir.path());
    let lock = dir.path().join(eqld::lock::FILE_NAME);

    let mut first = daemon(&config, &[]);
    wait_for(&lock, true);

    let mut forced = daemon(&config, &["--force"]);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let holder: Option<eqld::lock::Holder> = std::fs::read(&lock)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        if holder.is_some_and(|holder| holder.pid == forced.id()) {
            break;
        }
        assert!(Instant::now() < deadline, "--force did not take the lock");
        std::thread::sleep(Duration::from_millis(50));
    }

    first.kill().unwrap();
    first.wait().unwrap();
    assert!(
        lock.exists(),
        "the instance that lost the lock must not delete the winner's"
    );
    forced.kill().unwrap();
    forced.wait().unwrap();
}

#[cfg(unix)]
#[test]
fn a_daemon_that_shuts_down_cleanly_releases_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let config = harness(dir.path());
    let lock = dir.path().join(eqld::lock::FILE_NAME);

    let mut running = daemon(&config, &[]);
    wait_for(&lock, true);

    let killed = Command::new("kill")
        .args(["-INT", &running.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    assert!(running.wait().unwrap().success());
    wait_for(&lock, false);

    let mut next = daemon(&config, &[]);
    wait_for(&lock, true);
    next.kill().unwrap();
    next.wait().unwrap();
}
