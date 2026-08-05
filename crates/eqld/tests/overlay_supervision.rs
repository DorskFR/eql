#![cfg(target_os = "linux")]

use eqld::{config::Config, daemon::Daemon};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn config(root: &Path, tools: &Path, overlays: &str, enabled: bool) -> Config {
    tuned(root, tools, overlays, enabled, "")
}

fn tuned(root: &Path, tools: &Path, overlays: &str, enabled: bool, extra: &str) -> Config {
    toml::from_str(&text(root, tools, overlays, enabled, extra)).unwrap()
}

fn text(root: &Path, tools: &Path, overlays: &str, enabled: bool, extra: &str) -> String {
    format!(
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
    )
}

/// Written aside and renamed in: a sibling test forking while the final path
/// held a write descriptor would fail the exec with ETXTBSY.
fn install(dir: &TempDir, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.path().join(name);
    let staged = dir.path().join(format!("{name}.staged"));
    std::fs::write(&staged, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, &path).unwrap();
    path
}

fn game_log(root: &TempDir) -> PathBuf {
    let logs = root.path().join("Logs");
    std::fs::create_dir_all(&logs).unwrap();
    let log = logs.join("eqlog_Dorsk_erudin.txt");
    std::fs::write(&log, "").unwrap();
    log
}

/// `eqgame.exe` is looked up machine-wide, so two tests cannot disagree about
/// whether the game is up at the same time.
static GAME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    let _game = GAME.lock().await;
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
    let _game = GAME.lock().await;
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
    let _game = GAME.lock().await;
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
    let _game = GAME.lock().await;
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
    let _game = GAME.lock().await;
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
    let _game = GAME.lock().await;
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

fn pid_of(path: &Path) -> u32 {
    std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn overlays_are_toggled_by_editing_the_config_while_the_daemon_runs() {
    let _game = GAME.lock().await;
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    game_log(&root);
    let dps_pid = tools.path().join("dps.pid");
    let friend_pid = tools.path().join("friend.pid");
    install(&tools, "eql_atlas", "exit 0");
    install(
        &tools,
        "eql_dps_meter",
        &format!("echo $$ > {}\nexec sleep 120", dps_pid.display()),
    );
    install(
        &tools,
        "eql_friend_overlay",
        &format!("echo $$ > {}\nexec sleep 120", friend_pid.display()),
    );

    let config = root.path().join("eqld.toml");
    let write = |overlays: &str| {
        std::fs::write(
            &config,
            text(root.path(), tools.path(), overlays, false, ""),
        )
        .unwrap()
    };
    write("\"dps\"");

    let mut daemon = Daemon::new(Config::load(&config).unwrap())
        .unwrap()
        .watching(config.clone());
    let mut game = fake_game(&tools);
    settle().await;
    daemon.tick().await;
    settle().await;
    let dps = pid_of(&dps_pid);
    assert!(alive(dps));
    assert!(!friend_pid.exists());

    write("\"dps\", \"friend\"");
    daemon.tick().await;
    settle().await;
    assert_eq!(daemon.overlays(), vec!["dps", "friend"]);
    assert_eq!(
        pid_of(&dps_pid),
        dps,
        "the running overlay is not restarted"
    );
    assert!(alive(dps));
    let friend = pid_of(&friend_pid);
    assert!(alive(friend));

    write("\"friend\"");
    daemon.tick().await;
    settle().await;
    assert_eq!(daemon.overlays(), vec!["friend"]);
    assert!(!alive(dps), "the de-listed overlay is stopped");
    assert_eq!(pid_of(&friend_pid), friend, "the kept one is untouched");
    assert!(alive(friend));

    write("");
    daemon.tick().await;
    settle().await;
    assert!(daemon.overlays().is_empty());
    assert!(!alive(friend));

    daemon.shutdown().await;
    game.kill().unwrap();
    game.wait().unwrap();
}

#[tokio::test]
async fn a_broken_config_edit_leaves_the_daemon_on_the_last_good_one() {
    let _game = GAME.lock().await;
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    game_log(&root);
    let pidfile = tools.path().join("dps.pid");
    install(&tools, "eql_atlas", "exit 0");
    install(
        &tools,
        "eql_dps_meter",
        &format!("echo $$ > {}\nexec sleep 120", pidfile.display()),
    );

    let config = root.path().join("eqld.toml");
    std::fs::write(
        &config,
        text(root.path(), tools.path(), "\"dps\"", false, ""),
    )
    .unwrap();

    let mut daemon = Daemon::new(Config::load(&config).unwrap())
        .unwrap()
        .watching(config.clone());
    let mut game = fake_game(&tools);
    settle().await;
    daemon.tick().await;
    settle().await;
    let pid = pid_of(&pidfile);
    assert!(alive(pid));

    std::fs::write(&config, "[tools.log_reader\noverlays = [").unwrap();
    daemon.tick().await;
    settle().await;
    assert_eq!(daemon.overlays(), vec!["dps"]);
    assert_eq!(pid_of(&pidfile), pid);
    assert!(alive(pid), "a config that will not parse changes nothing");

    daemon.shutdown().await;
    game.kill().unwrap();
    game.wait().unwrap();
}

#[tokio::test]
async fn an_edit_to_a_restart_only_field_keeps_the_running_value() {
    let _game = GAME.lock().await;
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    game_log(&root);
    install(&tools, "eql_atlas", "exit 0");
    install(&tools, "eql_dps_meter", "exec sleep 120");

    let config = root.path().join("eqld.toml");
    std::fs::write(
        &config,
        text(root.path(), tools.path(), "\"dps\"", false, ""),
    )
    .unwrap();
    let mut daemon = Daemon::new(Config::load(&config).unwrap())
        .unwrap()
        .watching(config.clone());

    let moved = TempDir::new().unwrap();
    std::fs::write(
        &config,
        text(moved.path(), tools.path(), "\"dps\"", false, "")
            .replace("token = \"t\"", "token = \"rotated\""),
    )
    .unwrap();
    daemon.tick().await;

    assert_eq!(daemon.config().game.root, root.path());
    assert_eq!(daemon.config().api.token, "t");
    assert_eq!(daemon.overlays(), vec!["dps"]);
}

#[tokio::test]
async fn switching_the_atlas_mode_swaps_the_replay_tick_for_the_overlay() {
    let _game = GAME.lock().await;
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

    let config = root.path().join("eqld.toml");
    let write = |mode: &str| {
        std::fs::write(
            &config,
            text(
                root.path(),
                tools.path(),
                "\"atlas\"",
                true,
                &format!("replay_secs = 0\n{mode}"),
            ),
        )
        .unwrap()
    };
    write("");

    let mut daemon = Daemon::new(Config::load(&config).unwrap())
        .unwrap()
        .watching(config.clone());
    assert!(
        daemon.overlays().is_empty(),
        "the atlas overlay is refused while the replay tick owns the database"
    );
    let mut game = fake_game(&tools);
    settle().await;
    daemon.tick().await;
    settle().await;
    assert!(std::fs::read_to_string(&argv).unwrap().contains("--replay"));
    std::fs::write(&argv, "").unwrap();

    write("atlas = \"overlay\"");
    daemon.tick().await;
    settle().await;
    assert_eq!(daemon.overlays(), vec!["atlas"]);
    let seen = std::fs::read_to_string(&argv).unwrap();
    assert!(
        !seen.contains("--replay"),
        "the replay tick stopped: {seen:?}"
    );
    assert!(
        seen.contains("eqlog_Dorsk_erudin"),
        "the overlay started: {seen:?}"
    );

    write("");
    daemon.tick().await;
    settle().await;
    assert!(
        daemon.overlays().is_empty(),
        "the atlas overlay is refused again"
    );
    assert!(
        std::fs::read_to_string(&argv).unwrap().contains("--replay"),
        "and the replay tick is back"
    );

    daemon.shutdown().await;
    game.kill().unwrap();
    game.wait().unwrap();
}

#[tokio::test]
async fn an_uninstalled_overlay_is_skipped_without_killing_the_daemon() {
    let _game = GAME.lock().await;
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
