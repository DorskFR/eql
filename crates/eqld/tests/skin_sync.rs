use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Router,
};
use eqld::{config::Config, daemon::Daemon};
use std::{
    collections::HashMap,
    io::{Cursor, Write},
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
};

type Asked = (String, Option<String>);

#[derive(Clone, Default)]
struct Server {
    bundle: Arc<Mutex<Vec<u8>>>,
    asked: Arc<Mutex<Vec<Asked>>>,
}

impl Server {
    fn serve(&self, files: &[(&str, &str)]) {
        *self.bundle.lock().unwrap() = zip_of(files);
    }

    fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }
}

fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
    for (name, body) in files {
        writer.start_file(*name, options).unwrap();
        writer.write_all(body.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

async fn bundle(
    State(server): State<Server>,
    axum::extract::Path(layout): axum::extract::Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> (StatusCode, Vec<u8>) {
    server
        .asked
        .lock()
        .unwrap()
        .push((layout, query.get("skin").cloned()));
    (StatusCode::OK, server.bundle.lock().unwrap().clone())
}

async fn spawn_server(server: Server) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/api/v1/layouts/{name}/bundle", get(bundle))
        .with_state(server);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

fn config(root: &Path, addr: SocketAddr, skin: &str, process: &str) -> Config {
    toml::from_str(&format!(
        r#"
        [game]
        root = {root}
        {process}
        [api]
        url = "http://{addr}"
        token = "t"
        [state]
        path = {state}
        [skin]
        {skin}
        "#,
        root = toml::Value::from(root.to_str().unwrap()),
        state = toml::Value::from(root.join("state.json").to_str().unwrap()),
    ))
    .unwrap()
}

const CLOSED: &str = r#"process = "eqg-off.exe""#;

const FIRST: &[(&str, &str)] = &[
    ("uifiles/dorskui/EQUI_PlayerWindow.xml", "<CX>660</CX>"),
    ("UI_Dorsk_erudin_LO1.ini", "[MainChat]\r\nWidth=1480\r\n"),
];
const SECOND: &[(&str, &str)] = &[
    ("uifiles/dorskui/EQUI_PlayerWindow.xml", "<CX>500</CX>"),
    ("UI_Dorsk_erudin_LO1.ini", "[MainChat]\r\nWidth=900\r\n"),
];

#[tokio::test]
async fn the_skin_is_installed_once_and_again_only_when_the_bundle_changes() {
    let server = Server::default();
    server.serve(FIRST);
    let addr = spawn_server(server.clone()).await;
    let root = tempfile::tempdir().unwrap();
    let config = config(
        root.path(),
        addr,
        "enabled = true\nlayout = \"dorskui\"\ncheck_secs = 0",
        CLOSED,
    );
    let mut daemon = Daemon::new(config).unwrap();
    let xml = root.path().join("uifiles/dorskui/EQUI_PlayerWindow.xml");

    assert_eq!(daemon.tick().await.skins, 1);
    assert_eq!(std::fs::read_to_string(&xml).unwrap(), "<CX>660</CX>");
    assert_eq!(server.asked(), vec![("dorskui".to_string(), None)]);

    let installed = daemon.state().skin.clone().expect("a recorded skin");
    assert_eq!(installed.layout, "dorskui");
    assert_eq!(installed.installed, "dorskui");
    assert!(installed.installed_at.is_some());
    assert_eq!(
        eqld::State::load(&root.path().join("state.json"))
            .unwrap()
            .skin,
        Some(installed.clone()),
        "the digest survives a restart"
    );

    assert_eq!(
        daemon.tick().await.skins,
        0,
        "an unchanged bundle is not written over the client's uifiles"
    );
    assert_eq!(
        daemon.state().skin.as_ref().unwrap().digest,
        installed.digest
    );

    server.serve(SECOND);
    assert_eq!(daemon.tick().await.skins, 1);
    assert_eq!(std::fs::read_to_string(&xml).unwrap(), "<CX>500</CX>");
    assert_ne!(
        daemon.state().skin.as_ref().unwrap().digest,
        installed.digest
    );
}

#[tokio::test]
async fn nothing_is_fetched_until_the_skin_is_switched_on() {
    let server = Server::default();
    server.serve(FIRST);
    let addr = spawn_server(server.clone()).await;
    let root = tempfile::tempdir().unwrap();

    let mut off = Daemon::new(config(root.path(), addr, "layout = \"dorskui\"", CLOSED)).unwrap();
    assert_eq!(off.tick().await.skins, 0);

    let mut nameless = Daemon::new(config(root.path(), addr, "enabled = true", CLOSED)).unwrap();
    assert_eq!(nameless.tick().await.skins, 0);
    assert!(server.asked().is_empty(), "no layout, no request");
    assert!(!root.path().join("uifiles").exists());
}

#[tokio::test]
async fn a_client_that_cannot_be_seen_is_never_assumed_to_be_closed() {
    let server = Server::default();
    server.serve(FIRST);
    let addr = spawn_server(server.clone()).await;
    let root = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::new(config(
        root.path(),
        addr,
        "enabled = true\nlayout = \"dorskui\"\ncheck_secs = 0",
        r#"process = """#,
    ))
    .unwrap();

    assert_eq!(daemon.tick().await.skins, 0);
    assert!(server.asked().is_empty());
    assert!(
        !root.path().join("uifiles").exists(),
        "the client's files are left alone when the game cannot be found"
    );
    assert!(daemon.state().skin.is_none());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn the_skin_waits_for_the_game_to_close() {
    let server = Server::default();
    server.serve(FIRST);
    let addr = spawn_server(server.clone()).await;
    let root = tempfile::tempdir().unwrap();
    let process = "eqg-skin.exe";
    let fake = root.path().join(process);
    std::fs::copy("/bin/sleep", &fake).unwrap();
    let mut game = tokio::process::Command::new(&fake)
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let mut daemon = Daemon::new(config(
        root.path(),
        addr,
        "enabled = true\nlayout = \"dorskui\"\ncheck_secs = 0",
        &format!("process = {:?}", process),
    ))
    .unwrap();
    assert_eq!(daemon.tick().await.skins, 0);
    assert!(server.asked().is_empty());

    game.kill().await.unwrap();
    game.wait().await.unwrap();
    assert_eq!(daemon.tick().await.skins, 1, "and lands once it exits");
}

#[tokio::test]
async fn a_new_layout_named_in_the_config_is_picked_up_without_a_restart() {
    let server = Server::default();
    server.serve(FIRST);
    let addr = spawn_server(server.clone()).await;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("eqld.toml");
    let write = |skin: &str| {
        std::fs::write(
            &path,
            format!(
                r#"
                [game]
                root = {root}
                {CLOSED}
                [api]
                url = "http://{addr}"
                token = "t"
                [state]
                path = {state}
                [skin]
                {skin}
                "#,
                root = toml::Value::from(root.path().to_str().unwrap()),
                state = toml::Value::from(root.path().join("state.json").to_str().unwrap()),
            ),
        )
        .unwrap();
    };
    write("enabled = false");

    let mut daemon = Daemon::new(Config::load(&path).unwrap())
        .unwrap()
        .watching(path.clone());
    assert_eq!(daemon.tick().await.skins, 0);

    write("enabled = true\nlayout = \"dorskui\"\nname = \"v4\"\ncheck_secs = 0");
    assert_eq!(daemon.tick().await.skins, 1);
    assert_eq!(
        server.asked(),
        vec![("dorskui".to_string(), Some("v4".to_string()))]
    );
}
