use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
use eqld::{config::Config, daemon::Daemon, state::LastStatus};
use serde_json::Value;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc, Mutex,
    },
};
use tempfile::TempDir;

const ATLAS: &str = "eql_atlas_Dorsk_erudin.json";
const QUEST: &str = "eql_quest_Dorsk_erudin.json";
const ALLTIME: &str = "eql_alltime_Dorsk_erudin__WAR-CLR.json";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/harvest")
}

fn plant(dir: &TempDir, name: &str) {
    std::fs::copy(fixtures().join(name), dir.path().join(name)).unwrap();
}

fn plant_all(dir: &TempDir) {
    for name in [ATLAS, QUEST, ALLTIME] {
        plant(dir, name);
    }
}

#[derive(Debug, Clone)]
struct Captured {
    authorization: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct Recorder {
    requests: Arc<Mutex<Vec<Captured>>>,
    next_status: Arc<AtomicU16>,
}

impl Recorder {
    fn new(status: u16) -> Self {
        Self {
            requests: Arc::default(),
            next_status: Arc::new(AtomicU16::new(status)),
        }
    }

    fn requests(&self) -> Vec<Captured> {
        self.requests.lock().unwrap().clone()
    }

    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn kinds(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|request| {
                request.body["kind"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect()
    }
}

async fn ingest(
    State(recorder): State<Recorder>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::http::StatusCode {
    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(String::from);
    recorder.requests.lock().unwrap().push(Captured {
        authorization,
        body,
    });
    axum::http::StatusCode::from_u16(recorder.next_status.load(Ordering::SeqCst)).unwrap()
}

async fn spawn_server(recorder: Recorder) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/api/v1/harvest", post(ingest))
        .with_state(recorder);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

fn harness(game: &TempDir, harvest: Option<&TempDir>, addr: SocketAddr) -> Daemon {
    let section = match harvest {
        Some(dir) => format!(
            "[harvest]\nenabled = true\ndir = {}",
            toml::Value::from(dir.path().to_str().unwrap())
        ),
        None => String::new(),
    };
    let config: Config = toml::from_str(&format!(
        r#"
        [game]
        root = {root}
        poll_secs = 1
        [api]
        url = "http://{addr}"
        token = "machine-token"
        [state]
        path = {state}
        {section}
        "#,
        root = toml::Value::from(game.path().to_str().unwrap()),
        state = toml::Value::from(game.path().join("state.json").to_str().unwrap()),
    ))
    .unwrap();
    Daemon::new(config).unwrap()
}

#[tokio::test]
async fn ships_every_kind_once_with_the_bearer_token() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let game = tempfile::tempdir().unwrap();
    let harvest = tempfile::tempdir().unwrap();
    plant_all(&harvest);
    let mut daemon = harness(&game, Some(&harvest), addr);

    let report = daemon.tick().await;
    assert_eq!(report.harvested, 3);
    assert_eq!(recorder.count(), 3);
    assert_eq!(recorder.kinds(), vec!["alltime", "atlas", "quest"]);

    for request in recorder.requests() {
        assert_eq!(
            request.authorization.as_deref(),
            Some("Bearer machine-token")
        );
        assert_eq!(request.body["character"], "Dorsk");
        assert_eq!(request.body["server"], "erudin");
        assert!(request.body["captured_at"].as_i64().unwrap() > 0);
    }

    let by_kind = |kind: &str| {
        recorder
            .requests()
            .into_iter()
            .find(|request| request.body["kind"] == kind)
            .expect("kind was shipped")
            .body
    };
    let atlas = by_kind("atlas");
    assert_eq!(atlas["doc"]["format"], 1);
    assert_eq!(atlas["doc"]["totals"]["kills"], 137);
    assert_eq!(
        atlas["doc"]["zones"]["befallen"]["mobs"]["a skeleton"]["kills"],
        84
    );
    let quest = by_kind("quest");
    assert_eq!(quest["doc"]["current"], "1042");
    assert_eq!(quest["doc"]["quests"]["1042"]["have"]["13073"], 4);
    let alltime = by_kind("alltime");
    assert_eq!(alltime["doc"]["kills"], 1349);
    assert_eq!(alltime["doc"]["source_dmg"]["melee"], 4120334);

    let state = eqld::State::load(&game.path().join("state.json")).unwrap();
    assert_eq!(state.harvest.len(), 3);
    let atlas_state = &state.harvest[ATLAS];
    assert_eq!(atlas_state.last_status, LastStatus::Uploaded);
    assert_eq!(atlas_state.uploaded_hash.as_ref(), Some(&atlas_state.hash));
}

#[tokio::test]
async fn unchanged_docs_are_not_reshipped() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let game = tempfile::tempdir().unwrap();
    let harvest = tempfile::tempdir().unwrap();
    plant_all(&harvest);
    let mut daemon = harness(&game, Some(&harvest), addr);

    assert_eq!(daemon.tick().await.harvested, 3);
    let second = daemon.tick().await;
    assert_eq!(second.harvested, 0);
    assert_eq!(second.harvest_skipped, 3);

    plant(&harvest, ATLAS);
    let third = daemon.tick().await;
    assert_eq!(third.harvested, 0);
    assert_eq!(third.harvest_skipped, 3);
    assert_eq!(recorder.count(), 3);
}

#[tokio::test]
async fn a_changed_doc_ships_exactly_once_more() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let game = tempfile::tempdir().unwrap();
    let harvest = tempfile::tempdir().unwrap();
    plant_all(&harvest);
    let mut daemon = harness(&game, Some(&harvest), addr);
    assert_eq!(daemon.tick().await.harvested, 3);

    let path = harvest.path().join(ATLAS);
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    doc["totals"]["kills"] = Value::from(999);
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();

    let report = daemon.tick().await;
    assert_eq!(report.harvested, 1);
    assert_eq!(recorder.count(), 4);
    let last = recorder.requests().pop().unwrap();
    assert_eq!(last.body["kind"], "atlas");
    assert_eq!(last.body["doc"]["totals"]["kills"], 999);

    assert_eq!(daemon.tick().await.harvested, 0);
    assert_eq!(recorder.count(), 4);
}

#[tokio::test]
async fn harvest_stays_off_unless_it_is_enabled() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let game = tempfile::tempdir().unwrap();
    let harvest = tempfile::tempdir().unwrap();
    plant_all(&harvest);
    let mut daemon = harness(&game, None, addr);

    let report = daemon.tick().await;
    assert_eq!(report.harvested, 0);
    assert_eq!(recorder.count(), 0);
}

#[tokio::test]
async fn out_of_scope_and_oversized_files_are_left_alone() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let game = tempfile::tempdir().unwrap();
    let harvest = tempfile::tempdir().unwrap();
    plant(&harvest, QUEST);
    for name in [
        "eql_session_report_records_Dorsk_erudin.json",
        "eql_friend_overlay_roster_Dorsk_erudin.json",
        "eql_atlas_settings.json",
    ] {
        std::fs::write(harvest.path().join(name), "{}").unwrap();
    }
    let padding = "x".repeat(9 * 1024 * 1024);
    std::fs::write(
        harvest.path().join(ATLAS),
        format!(r#"{{"format":1,"pad":"{padding}"}}"#),
    )
    .unwrap();
    let mut daemon = harness(&game, Some(&harvest), addr);

    let report = daemon.tick().await;
    assert_eq!(report.harvested, 1);
    assert_eq!(report.harvest_skipped, 1);
    assert_eq!(recorder.kinds(), vec!["quest"]);
}

#[tokio::test]
async fn half_written_json_is_retried_on_the_next_tick() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let game = tempfile::tempdir().unwrap();
    let harvest = tempfile::tempdir().unwrap();
    std::fs::write(harvest.path().join(ATLAS), r#"{"format":1,"zones":"#).unwrap();
    let mut daemon = harness(&game, Some(&harvest), addr);

    let report = daemon.tick().await;
    assert_eq!(report.parse_failures, 1);
    assert_eq!(recorder.count(), 0);

    plant(&harvest, ATLAS);
    assert_eq!(daemon.tick().await.harvested, 1);
    assert_eq!(recorder.count(), 1);
}

#[tokio::test]
async fn a_server_error_replays_and_a_rejection_parks() {
    let recorder = Recorder::new(500);
    let addr = spawn_server(recorder.clone()).await;
    let game = tempfile::tempdir().unwrap();
    let harvest = tempfile::tempdir().unwrap();
    plant(&harvest, QUEST);
    let mut daemon = harness(&game, Some(&harvest), addr);

    assert_eq!(daemon.tick().await.retryable_failures, 1);
    assert_eq!(daemon.tick().await.retryable_failures, 1);
    assert_eq!(recorder.count(), 2);

    recorder.next_status.store(401, Ordering::SeqCst);
    assert_eq!(daemon.tick().await.rejections, 1);
    daemon.tick().await;
    assert_eq!(recorder.count(), 3);

    recorder.next_status.store(201, Ordering::SeqCst);
    let path = harvest.path().join(QUEST);
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    doc["current"] = Value::Null;
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(daemon.tick().await.harvested, 1);
    assert_eq!(recorder.count(), 4);
}

#[tokio::test]
async fn state_survives_a_restart() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let game = tempfile::tempdir().unwrap();
    let harvest = tempfile::tempdir().unwrap();
    plant_all(&harvest);

    let mut daemon = harness(&game, Some(&harvest), addr);
    assert_eq!(daemon.tick().await.harvested, 3);
    drop(daemon);

    let mut restarted = harness(&game, Some(&harvest), addr);
    assert_eq!(restarted.tick().await.harvested, 0);
    assert_eq!(recorder.count(), 3);
}

#[cfg(unix)]
fn stub_reader(dir: &TempDir, harvest: &TempDir) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.path().join("eql_atlas_stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n[ \"$1\" = --replay ] || exit 64\n[ -f \"$2\" ] || exit 65\ncp {src} {dst}\n",
            src = fixtures().join(ATLAS).display(),
            dst = harvest.path().join(ATLAS).display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[cfg(unix)]
fn orchestrated(game: &TempDir, harvest: &TempDir, exe: &Path, addr: SocketAddr) -> Daemon {
    let config: Config = toml::from_str(&format!(
        r#"
        [game]
        root = {root}
        poll_secs = 1
        [api]
        url = "http://{addr}"
        token = "machine-token"
        [state]
        path = {state}
        [harvest]
        enabled = true
        dir = {dir}
        [tools.log_reader]
        enabled = true
        exe = {exe}
        replay_secs = 0
        "#,
        root = toml::Value::from(game.path().to_str().unwrap()),
        state = toml::Value::from(game.path().join("state.json").to_str().unwrap()),
        dir = toml::Value::from(harvest.path().to_str().unwrap()),
        exe = toml::Value::from(exe.to_str().unwrap()),
    ))
    .unwrap();
    Daemon::new(config).unwrap()
}

/// The whole point of the orchestrator: eqld drives the reader, and the JSON it
/// writes is picked up and shipped in the same tick.
#[cfg(unix)]
#[tokio::test]
async fn the_daemon_drives_the_log_reader_and_ships_what_it_writes() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let game = tempfile::tempdir().unwrap();
    let harvest = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(game.path().join("Logs")).unwrap();
    std::fs::write(
        game.path().join("Logs").join("eqlog_Dorsk_erudin.txt"),
        "[Wed Aug 05 19:24:52 2026] Logging is now *ON*.\n",
    )
    .unwrap();

    let exe = stub_reader(&game, &harvest);
    assert!(
        !harvest.path().join(ATLAS).exists(),
        "nothing to harvest before the reader runs"
    );

    let mut daemon = orchestrated(&game, &harvest, &exe, addr);
    assert!(
        daemon.runner().is_some(),
        "the configured exe is discovered"
    );

    let report = daemon.tick().await;
    assert_eq!(
        report.harvested, 1,
        "the file the reader just wrote shipped"
    );
    assert_eq!(recorder.kinds(), vec!["atlas"]);
    assert!(harvest.path().join(ATLAS).exists());
}

#[cfg(unix)]
#[tokio::test]
async fn a_missing_reader_leaves_the_daemon_running() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let game = tempfile::tempdir().unwrap();
    let harvest = tempfile::tempdir().unwrap();
    let missing = game.path().join("not-installed");

    let mut daemon = orchestrated(&game, &harvest, &missing, addr);
    assert!(daemon.runner().is_none(), "nothing to discover");
    assert_eq!(daemon.tick().await.harvested, 0);
    assert_eq!(recorder.count(), 0);
}
