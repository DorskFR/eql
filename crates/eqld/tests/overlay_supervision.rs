#![cfg(target_os = "linux")]

use eqld::{config::Config, daemon::Daemon};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn config(root: &Path, tools: &Path, overlays: &str, enabled: bool) -> Config {
    tuned(root, tools, overlays, enabled, "")
}

fn tuned(root: &Path, tools: &Path, overlays: &str, enabled: bool, extra: &str) -> Config {
    toml::from_str(&format!(
        r#"
        [game]
        root = "{root}"
        [api]
        url = "http://127.0.0.1:1"
        token = "t"
        [state]
        path = "{state}"
        [tools.log_reader]
        enabled = {enabled}
        exe = "{exe}"
        overlays = [{overlays}]
        {extra}
        "#,
        root = root.display(),
        state = root.join("state.json").display(),
        exe = tools.join("eql_atlas").display(),
    ))
    .unwrap()
}

fn install(dir: &TempDir, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.path().join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn game_log(root: &TempDir) -> PathBuf {
    let logs = root.path().join("Logs");
    std::fs::create_dir_all(&logs).unwrap();
    let log = logs.join("eqlog_Dorsk_erudin.txt");
    std::fs::write(&log, "").unwrap();
    log
}

fn fake_game(dir: &TempDir) -> std::process::Child {
    let exe = dir.path().join("eqgame.exe");
    std::fs::copy("/bin/sleep", &exe).unwrap();
    std::process::Command::new(&exe).arg("120").spawn().unwrap()
}

fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

async fn settle() {
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
}

#[tokio::test]
async fn overlays_follow_the_game_from_launch_to_exit() {
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    let log = game_log(&root);
    let marker = tools.path().join("argv.txt");
    let pidfile = tools.path().join("pid.txt");
    install(&tools, "eql_atlas", "exit 0");
    install(
        &tools,
        "eql_dps_meter",
        &format!(
            "echo \"$1\" > {marker}\necho $$ > {pidfile}\nexec sleep 120",
            marker = marker.display(),
            pidfile = pidfile.display(),
        ),
    );

    let mut daemon = Daemon::new(config(root.path(), tools.path(), "\"dps\"", false)).unwrap();
    assert_eq!(daemon.overlays(), vec!["dps"]);

    daemon.tick().await;
    assert!(
        !marker.exists(),
        "no overlay starts while the game is not running"
    );

    let mut game = fake_game(&tools);
    settle().await;
    daemon.tick().await;
    settle().await;

    assert_eq!(
        std::fs::read_to_string(&marker).unwrap().trim(),
        log.display().to_string(),
        "the overlay is pointed at the active character's log"
    );
    let pid: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(alive(pid));

    game.kill().unwrap();
    game.wait().unwrap();
    settle().await;
    daemon.tick().await;
    assert!(!alive(pid), "the overlay is stopped once the game exits");
}

#[tokio::test]
async fn shutdown_takes_the_overlays_with_it() {
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    game_log(&root);
    let pidfile = tools.path().join("pid.txt");
    install(&tools, "eql_atlas", "exit 0");
    install(
        &tools,
        "eql_friend_overlay",
        &format!("echo $$ > {}\nexec sleep 120", pidfile.display()),
    );

    let mut daemon = Daemon::new(config(root.path(), tools.path(), "\"friend\"", false)).unwrap();
    let mut game = fake_game(&tools);
    settle().await;
    daemon.tick().await;
    settle().await;

    let pid: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(alive(pid));

    daemon.shutdown().await;
    assert!(!alive(pid));

    game.kill().unwrap();
    game.wait().unwrap();
}

#[tokio::test]
async fn the_atlas_overlay_never_starts_beside_the_replay_harvest() {
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    game_log(&root);
    install(&tools, "eql_atlas", "exit 0");

    let refused = Daemon::new(config(root.path(), tools.path(), "\"atlas\"", true)).unwrap();
    assert!(
        refused.overlays().is_empty(),
        "the atlas autosave would fight --replay"
    );

    let allowed = Daemon::new(config(root.path(), tools.path(), "\"atlas\"", false)).unwrap();
    assert_eq!(allowed.overlays(), vec!["atlas"]);
}

/// The only way to ever get quest data: the Atlas overlay keeps the database
/// live, so the replay that would fight it is skipped instead of refusing it.
#[tokio::test]
async fn atlas_overlay_mode_runs_the_atlas_and_never_replays() {
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    game_log(&root);
    let argv = tools.path().join("argv.txt");
    install(
        &tools,
        "eql_atlas",
        &format!(
            "echo \"$1\" >> {}\n[ \"$1\" = --replay ] && exit 0\nexec sleep 120",
            argv.display()
        ),
    );

    let mut daemon = Daemon::new(tuned(
        root.path(),
        tools.path(),
        "\"atlas\"",
        true,
        "atlas = \"overlay\"\nreplay_secs = 0",
    ))
    .unwrap();
    assert_eq!(daemon.overlays(), vec!["atlas"]);
    assert!(daemon.hidden_overlays().is_empty());

    daemon.tick().await;
    settle().await;
    let seen = std::fs::read_to_string(&argv).unwrap_or_default();
    assert!(
        !seen.contains("--replay"),
        "the overlay owns the database, nothing replays: {seen:?}"
    );

    daemon.shutdown().await;

    let mut replaying = Daemon::new(tuned(
        root.path(),
        tools.path(),
        "\"atlas\"",
        true,
        "replay_secs = 0",
    ))
    .unwrap();
    assert!(replaying.overlays().is_empty());
    replaying.tick().await;
    assert!(
        std::fs::read_to_string(&argv).unwrap().contains("--replay"),
        "replay mode still replays"
    );
}

#[tokio::test]
async fn a_hidden_overlay_is_still_supervised() {
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    let log = game_log(&root);
    let marker = tools.path().join("argv.txt");
    install(&tools, "eql_atlas", "exit 0");
    install(
        &tools,
        "eql_dps_meter",
        &format!("echo \"$1\" > {}\nexec sleep 120", marker.display()),
    );

    let mut daemon = Daemon::new(tuned(
        root.path(),
        tools.path(),
        "\"dps\"",
        false,
        "hidden = [\"dps\"]",
    ))
    .unwrap();
    assert_eq!(daemon.hidden_overlays(), vec!["dps"]);

    let mut game = fake_game(&tools);
    settle().await;
    daemon.tick().await;
    settle().await;
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap().trim(),
        log.display().to_string()
    );

    daemon.shutdown().await;
    game.kill().unwrap();
    game.wait().unwrap();
}

#[tokio::test]
async fn a_hidden_name_that_is_not_an_overlay_is_refused_not_launched() {
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    game_log(&root);
    install(&tools, "eql_atlas", "exit 0");
    install(&tools, "eql_dps_meter", "exec sleep 120");

    let daemon = Daemon::new(tuned(
        root.path(),
        tools.path(),
        "\"dps\"",
        false,
        "hidden = [\"friend\"]",
    ))
    .unwrap();
    assert_eq!(daemon.overlays(), vec!["dps"]);
    assert!(daemon.hidden_overlays().is_empty());
}

#[tokio::test]
async fn an_uninstalled_overlay_is_skipped_without_killing_the_daemon() {
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    game_log(&root);
    install(&tools, "eql_atlas", "exit 0");

    let mut daemon = Daemon::new(config(
        root.path(),
        tools.path(),
        "\"dps\", \"friend\"",
        false,
    ))
    .unwrap();
    assert!(daemon.overlays().is_empty());

    let mut game = fake_game(&tools);
    settle().await;
    daemon.tick().await;
    game.kill().unwrap();
    game.wait().unwrap();
}
