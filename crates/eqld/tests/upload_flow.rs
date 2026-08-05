use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use eqld::{config::Config, daemon::Daemon, state::LastStatus};
use serde_json::Value;
use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc, Mutex,
    },
};
use tempfile::TempDir;

const SAMPLE: &str = "Location\tName\tID\tCount\tSlots\n\
    Charm\tEmpty\t0\t1\t0\n\
    Primary\tSpirit Reaver\t86755\t1\t0\n";

const CHANGED: &str = "Location\tName\tID\tCount\tSlots\n\
    Charm\tEmpty\t0\t1\t0\n\
    Primary\tSpirit Reaver\t86755\t1\t0\n\
    General1\tBackpack\t17963\t1\t8\n";

#[derive(Debug, Clone, PartialEq, Eq)]
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

    fn set_status(&self, status: u16) {
        self.next_status.store(status, Ordering::SeqCst);
    }

    fn requests(&self) -> Vec<Captured> {
        self.requests.lock().unwrap().clone()
    }

    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

async fn ingest(
    State(recorder): State<Recorder>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> StatusCode {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(String::from);
    recorder.requests.lock().unwrap().push(Captured {
        authorization,
        body,
    });
    StatusCode::from_u16(recorder.next_status.load(Ordering::SeqCst)).unwrap()
}

async fn spawn_server(recorder: Recorder) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/api/v1/inventory", post(ingest))
        .with_state(recorder);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

fn harness(dir: &TempDir, addr: SocketAddr, token: &str) -> Daemon {
    let config: Config = toml::from_str(&format!(
        r#"
        [game]
        root = {root}
        poll_secs = 1
        [api]
        url = "http://{addr}"
        token = "{token}"
        [state]
        path = {state}
        "#,
        root = toml::Value::from(dir.path().to_str().unwrap()),
        state = toml::Value::from(dir.path().join("state.json").to_str().unwrap()),
    ))
    .unwrap();
    Daemon::new(config).unwrap()
}

fn write_dump(dir: &TempDir, contents: &str) {
    std::fs::write(dir.path().join("Dorsk_erudin-Inventory.txt"), contents).unwrap();
}

#[tokio::test]
async fn uploads_a_new_dump_with_bearer_token_and_full_payload() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    write_dump(&dir, SAMPLE);
    let mut daemon = harness(&dir, addr, "machine-token");

    let report = daemon.tick().await;
    assert_eq!(report.uploaded, 1);

    let requests = recorder.requests();
    assert_eq!(requests.len(), 1);
    let Captured {
        authorization,
        body,
    } = &requests[0];
    assert_eq!(authorization.as_deref(), Some("Bearer machine-token"));
    assert_eq!(body["character"], "Dorsk");
    assert_eq!(body["server"], "erudin");
    assert_eq!(body["entries"].as_array().unwrap().len(), 2);
    assert_eq!(body["entries"][1]["name"], "Spirit Reaver");
    assert_eq!(body["entries"][1]["id"], 86755);
    assert_eq!(body["raw"], SAMPLE);
    assert!(body["captured_at"].as_i64().unwrap() > 0);

    let state = eqld::State::load(&dir.path().join("state.json")).unwrap();
    let file = &state.files["Dorsk_erudin-Inventory.txt"];
    assert_eq!(file.last_status, LastStatus::Uploaded);
    assert_eq!(file.uploaded_hash.as_ref(), Some(&file.hash));
    assert!(file.uploaded_at.is_some());
}

#[tokio::test]
async fn identical_content_is_uploaded_once() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    write_dump(&dir, SAMPLE);
    let mut daemon = harness(&dir, addr, "t");

    assert_eq!(daemon.tick().await.uploaded, 1);
    assert_eq!(daemon.tick().await.uploaded, 0);

    write_dump(&dir, SAMPLE);
    let report = daemon.tick().await;
    assert_eq!(report.uploaded, 0);
    assert_eq!(report.skipped, 1);
    assert_eq!(recorder.count(), 1);
}

#[tokio::test]
async fn changed_content_uploads_again() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    write_dump(&dir, SAMPLE);
    let mut daemon = harness(&dir, addr, "t");

    assert_eq!(daemon.tick().await.uploaded, 1);
    write_dump(&dir, CHANGED);
    assert_eq!(daemon.tick().await.uploaded, 1);

    let requests = recorder.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body["entries"].as_array().unwrap().len(), 2);
    assert_eq!(requests[1].body["entries"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn server_error_is_retried_on_the_next_tick_with_backoff() {
    let recorder = Recorder::new(500);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    write_dump(&dir, SAMPLE);
    let mut daemon = harness(&dir, addr, "t");

    let first = daemon.tick().await;
    assert_eq!(first.uploaded, 0);
    assert_eq!(first.retryable_failures, 1);
    assert_eq!(daemon.delay().as_secs(), 2);
    assert!(matches!(
        daemon.state().files["Dorsk_erudin-Inventory.txt"].last_status,
        LastStatus::Failed { .. }
    ));

    assert_eq!(daemon.tick().await.retryable_failures, 1);
    assert_eq!(daemon.delay().as_secs(), 4);

    recorder.set_status(201);
    let recovered = daemon.tick().await;
    assert_eq!(recovered.uploaded, 1);
    assert_eq!(daemon.delay().as_secs(), 1);
    assert_eq!(recorder.count(), 3);

    assert_eq!(daemon.tick().await.uploaded, 0);
    assert_eq!(recorder.count(), 3);
}

#[tokio::test]
async fn unauthorized_is_not_retried_until_the_dump_changes() {
    let recorder = Recorder::new(401);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    write_dump(&dir, SAMPLE);
    let mut daemon = harness(&dir, addr, "wrong");

    let first = daemon.tick().await;
    assert_eq!(first.rejections, 1);
    assert_eq!(first.retryable_failures, 0);
    assert_eq!(daemon.delay().as_secs(), 1);

    daemon.tick().await;
    write_dump(&dir, SAMPLE);
    daemon.tick().await;
    assert_eq!(recorder.count(), 1);

    recorder.set_status(201);
    write_dump(&dir, CHANGED);
    assert_eq!(daemon.tick().await.uploaded, 1);
    assert_eq!(recorder.count(), 2);
}

#[tokio::test]
async fn unparsable_dump_is_skipped_and_retried_later() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    write_dump(&dir, "Location\tName\n");
    let mut daemon = harness(&dir, addr, "t");

    let report = daemon.tick().await;
    assert_eq!(report.parse_failures, 1);
    assert_eq!(recorder.count(), 0);

    write_dump(&dir, SAMPLE);
    assert_eq!(daemon.tick().await.uploaded, 1);
    assert_eq!(recorder.count(), 1);
}

#[tokio::test]
async fn network_failure_keeps_the_file_dirty() {
    let dir = tempfile::tempdir().unwrap();
    write_dump(&dir, SAMPLE);
    let dead: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let mut daemon = harness(&dir, dead, "t");

    let report = daemon.tick().await;
    assert_eq!(report.retryable_failures, 1);
    assert!(matches!(
        daemon.state().files["Dorsk_erudin-Inventory.txt"].last_status,
        LastStatus::Failed { .. }
    ));
    assert_eq!(daemon.tick().await.retryable_failures, 1);
}

#[tokio::test]
async fn state_survives_a_restart() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    write_dump(&dir, SAMPLE);

    let mut daemon = harness(&dir, addr, "t");
    assert_eq!(daemon.tick().await.uploaded, 1);
    drop(daemon);

    let mut restarted = harness(&dir, addr, "t");
    assert_eq!(restarted.tick().await.uploaded, 0);
    assert_eq!(recorder.count(), 1);
}

#[tokio::test]
async fn non_inventory_files_are_ignored() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("eqlog_Dorsk_erudin.txt"), SAMPLE).unwrap();
    std::fs::write(dir.path().join("notes.txt"), SAMPLE).unwrap();
    let mut daemon = harness(&dir, addr, "t");

    let report = daemon.tick().await;
    assert_eq!(report.uploaded, 0);
    assert_eq!(recorder.count(), 0);
}
