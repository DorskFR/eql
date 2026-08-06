use eqld::{config::Config, daemon::Daemon, socials};
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

#[tokio::test]
async fn the_daemon_leaves_the_character_ini_alone_until_socials_are_switched_on() {
    let root = game_root();
    let ini = root.path().join("Dorsk_erudin_LO1.ini");

    let mut daemon = Daemon::new(config(root.path(), false)).unwrap();
    assert_eq!(daemon.tick().await.socials, 0);
    assert_eq!(std::fs::read(&ini).unwrap(), REAL);

    let mut daemon = Daemon::new(config(root.path(), true)).unwrap();
    assert_eq!(daemon.tick().await.socials, 1);
    let written = String::from_utf8(std::fs::read(&ini).unwrap()).unwrap();
    assert!(written.contains("Page2Button1Line1=/log on"), "{written}");
    assert!(written.contains("[SpellLoadouts]"), "{written}");
    assert_eq!(
        std::fs::read(root.path().join("Dorsk_erudin_LO1.ini.eqld.bak")).unwrap(),
        REAL
    );

    assert_eq!(
        daemon.tick().await.socials,
        0,
        "a social already installed is not rewritten every tick"
    );
}

#[tokio::test]
async fn a_reapplied_social_survives_the_client_rewriting_the_file() {
    let root = game_root();
    let ini = root.path().join("Dorsk_erudin_LO1.ini");
    let mut daemon = Daemon::new(config(root.path(), true)).unwrap();
    daemon.tick().await;

    std::fs::write(&ini, REAL).unwrap();
    assert_eq!(daemon.tick().await.socials, 1);
    assert_eq!(std::fs::read(&ini).unwrap(), socials::apply(REAL).unwrap());
}
