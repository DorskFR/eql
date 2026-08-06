use eqld::{config::Config, daemon::Daemon};
use std::{
    io::Write,
    sync::{Arc, Mutex},
};
use tempfile::TempDir;

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn take(&self) -> String {
        let mut held = self.0.lock().unwrap();
        String::from_utf8(std::mem::take(&mut *held)).unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for Buffer {
    type Writer = Buffer;

    fn make_writer(&self) -> Buffer {
        self.clone()
    }
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

fn build(game: &TempDir, extra: &str) -> Daemon {
    let config: Config = toml::from_str(&format!(
        r#"
        [game]
        root = {root}
        poll_secs = 1
        [api]
        url = "http://127.0.0.1:1"
        token = "t"
        [state]
        path = {state}
        {extra}
        "#,
        root = toml::Value::from(game.path().to_str().unwrap()),
        state = toml::Value::from(game.path().join("state.json").to_str().unwrap()),
    ))
    .unwrap();
    Daemon::new(config).unwrap()
}

fn harness(game: &TempDir, harvest: &std::path::Path) -> Daemon {
    build(
        game,
        &format!(
            "[harvest]\nenabled = true\ndir = {}",
            toml::Value::from(harvest.to_str().unwrap())
        ),
    )
}

#[tokio::test]
async fn an_unscannable_harvest_directory_is_reported_once_not_every_tick() {
    let logs = Buffer::default();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .finish(),
    )
    .unwrap();

    let game = tempfile::tempdir().unwrap();
    let harvest = game.path().join("reader");
    let mut daemon = harness(&game, &harvest);

    for _ in 0..4 {
        daemon.tick().await;
    }
    let written = logs.take();
    assert_eq!(
        count(&written, "cannot scan harvest directory"),
        1,
        "four ticks, one line:\n{written}"
    );

    std::fs::create_dir(&harvest).unwrap();
    for _ in 0..3 {
        daemon.tick().await;
    }
    let written = logs.take();
    assert_eq!(
        count(&written, "harvest directory is readable again"),
        1,
        "recovery is worth exactly one line:\n{written}"
    );
    assert_eq!(count(&written, "cannot scan harvest directory"), 0);

    std::fs::remove_dir(&harvest).unwrap();
    for _ in 0..3 {
        daemon.tick().await;
    }
    let written = logs.take();
    assert_eq!(
        count(&written, "cannot scan harvest directory"),
        1,
        "the fault coming back is news again, once:\n{written}"
    );

    let game = tempfile::tempdir().unwrap();
    let missing = game.path().join("nowhere").join("eql_atlas");
    let mut off = build(
        &game,
        &format!(
            "[tools.log_reader]\nenabled = true\nauto_install = false\nexe = {}\n\
             [harvest]\nenabled = true\ndir = {}",
            toml::Value::from(missing.to_str().unwrap()),
            toml::Value::from(game.path().to_str().unwrap()),
        ),
    );
    logs.take();
    for _ in 0..4 {
        off.tick().await;
    }
    let written = logs.take();
    assert_eq!(
        count(&written, "auto_install is off"),
        1,
        "the refusal to self-install is said once, not per tick:\n{written}"
    );
    assert_eq!(
        count(&written, "downloading and installing"),
        0,
        "and nothing is fetched behind the user's back:\n{written}"
    );
}
