use eqld::{config::Config, daemon::Daemon};
use std::path::Path;

const REAL: &[u8] = include_bytes!("../../../fixtures/ini/Dorsk_erudin_LO1.ini");

fn config(root: &Path, enabled: bool) -> Config {
    toml::from_str(&format!(
        r#"
        [game]
        root = "{root}"
        [api]
        url = "http://127.0.0.1:1"
        token = "t"
        [state]
        path = "{state}"
        [socials]
        enabled = {enabled}
        "#,
        root = root.display(),
        state = root.join("state.json").display(),
    ))
    .unwrap()
}

fn game_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("_characters.ini"),
        "[Characters]\r\nCharacter0=Dorsk,erudin\r\n",
    )
    .unwrap();
    std::fs::write(root.path().join("Dorsk_erudin_LO1.ini"), REAL).unwrap();
    root
}

/// The client owns this file while it runs and rewrites it on exit, so a
/// social written mid-session is thrown away.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn nothing_is_written_while_the_game_is_running() {
    let root = game_root();
    let ini = root.path().join("Dorsk_erudin_LO1.ini");
    let fake = root.path().join(eqld::overlays::GAME_PROCESS);
    std::fs::copy("/bin/sleep", &fake).unwrap();

    let mut game = tokio::process::Command::new(&fake)
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let mut daemon = Daemon::new(config(root.path(), true)).unwrap();
    assert_eq!(daemon.tick().await.socials, 0);
    assert_eq!(std::fs::read(&ini).unwrap(), REAL);

    game.kill().await.unwrap();
    game.wait().await.unwrap();
    assert_eq!(daemon.tick().await.socials, 1, "and applied once it exits");
}
