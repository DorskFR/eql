use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use eqld::{config::Config, daemon::Daemon};
use serde_json::Value;
use std::{
    io::Write,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc, Mutex,
    },
};
use tempfile::TempDir;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Captured {
    authorization: Option<String>,
    body: Value,
}

#[derive(Clone)]
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

async fn ingest_events(
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
        .route("/api/v1/events", post(ingest_events))
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

const LOG_NAME: &str = "eqlog_Dorsk_erudin.txt";

fn log_path(dir: &TempDir) -> PathBuf {
    dir.path().join("Logs").join(LOG_NAME)
}

fn seed_log(dir: &TempDir, contents: &str) {
    std::fs::create_dir_all(dir.path().join("Logs")).unwrap();
    std::fs::write(log_path(dir), contents).unwrap();
}

fn append(dir: &TempDir, contents: &str) {
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(log_path(dir))
        .unwrap();
    file.write_all(contents.as_bytes()).unwrap();
}

fn offset(dir: &TempDir) -> u64 {
    eqld::State::load(&dir.path().join("state.json"))
        .unwrap()
        .logs[LOG_NAME]
        .offset
}

const HISTORY: &str = "[Tue Jul 21 20:00:00 2026] You have entered Ancient History.\n";

#[tokio::test]
async fn a_new_log_starts_at_its_end_and_ships_no_backlog() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    seed_log(&dir, HISTORY);
    let mut daemon = harness(&dir, addr, "machine-token");

    let report = daemon.tick().await;
    assert_eq!(report.log_events, 0);
    assert_eq!(recorder.count(), 0);
    assert_eq!(offset(&dir), HISTORY.len() as u64);

    assert_eq!(daemon.tick().await.log_events, 0);
    assert_eq!(recorder.count(), 0);
}

#[tokio::test]
async fn appended_lines_become_typed_events_for_the_filename_character() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    seed_log(&dir, HISTORY);
    let mut daemon = harness(&dir, addr, "machine-token");
    daemon.tick().await;

    append(
        &dir,
        "[Tue Jul 21 21:15:23 2026] You have entered East Commonlands.\n\
         [Tue Jul 21 21:15:24 2026] --You have looted a Rusty Dagger.--\n\
         [Tue Jul 21 21:15:25 2026] You have gained a level! Welcome to level 12!\n\
         [Tue Jul 21 21:15:26 2026] You have become better at Meditate! (61)\n\
         [Tue Jul 21 21:15:27 2026] Your Location is 123.45, -678.90, 12.34\n\
         [Tue Jul 21 21:15:28 2026] You have been slain by a gnoll pup!\n\
         [Tue Jul 21 21:15:29 2026] You died.\n\
         [Tue Jul 21 21:15:30 2026] Dorsk says, 'this line is not an event'\n\
         You have entered a line with no timestamp.\n",
    );

    let report = daemon.tick().await;
    assert_eq!(report.log_events, 7);
    assert_eq!(report.log_lines_dropped, 2);

    let requests = recorder.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer machine-token")
    );
    let body = &requests[0].body;
    assert_eq!(body["character"], "Dorsk");
    assert_eq!(body["server"], "erudin");

    let events = body["events"].as_array().unwrap();
    assert_eq!(events.len(), 7);
    assert_eq!(events[0]["kind"], "zone");
    assert_eq!(events[0]["zone"], "East Commonlands");
    assert_eq!(events[0]["at"], 1_784_668_523_i64);
    assert_eq!(events[1]["kind"], "loot");
    assert_eq!(events[1]["item"], "Rusty Dagger");
    assert_eq!(events[2]["kind"], "level");
    assert_eq!(events[2]["level"], 12);
    assert_eq!(events[3]["kind"], "skill");
    assert_eq!(events[3]["skill"], "Meditate");
    assert_eq!(events[3]["value"], 61);
    assert_eq!(events[4]["kind"], "location");
    assert_eq!(events[4]["y"], 123.45);
    assert_eq!(events[4]["x"], -678.90);
    assert_eq!(events[4]["z"], 12.34);
    assert_eq!(events[5]["kind"], "death");
    assert_eq!(events[5]["killer"], "a gnoll pup");
    assert_eq!(events[6]["kind"], "death");
    assert!(events[6].get("killer").is_none());

    assert_eq!(daemon.tick().await.log_events, 0);
    assert_eq!(recorder.count(), 1);
}

#[tokio::test]
async fn a_half_written_line_waits_for_its_newline() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    seed_log(&dir, HISTORY);
    let mut daemon = harness(&dir, addr, "t");
    daemon.tick().await;
    let start = offset(&dir);

    append(&dir, "[Tue Jul 21 21:15:23 2026] You have entered East Com");
    assert_eq!(daemon.tick().await.log_events, 0);
    assert_eq!(recorder.count(), 0);
    assert_eq!(offset(&dir), start);

    append(&dir, "monlands.\n");
    assert_eq!(daemon.tick().await.log_events, 1);
    let events = recorder.requests()[0].body["events"].clone();
    assert_eq!(events[0]["zone"], "East Commonlands");
}

#[tokio::test]
async fn truncation_restarts_the_offset_at_the_top() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    seed_log(&dir, HISTORY);
    let mut daemon = harness(&dir, addr, "t");
    daemon.tick().await;
    append(&dir, "[Tue Jul 21 21:15:23 2026] You died.\n");
    assert_eq!(daemon.tick().await.log_events, 1);

    std::fs::write(log_path(&dir), "[Tue Jul 21 22:00:00 2026] You died.\n").unwrap();
    let report = daemon.tick().await;
    assert_eq!(report.log_events, 1);
    assert_eq!(offset(&dir), 37);

    let requests = recorder.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].body["events"][0]["at"], 1_784_671_200_i64);
}

#[tokio::test]
async fn a_rejected_batch_is_replayed_until_it_lands() {
    let recorder = Recorder::new(500);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    seed_log(&dir, HISTORY);
    let mut daemon = harness(&dir, addr, "t");
    daemon.tick().await;
    let start = offset(&dir);

    append(&dir, "[Tue Jul 21 21:15:23 2026] You died.\n");
    let failed = daemon.tick().await;
    assert_eq!(failed.log_events, 0);
    assert_eq!(failed.retryable_failures, 1);
    assert_eq!(offset(&dir), start);

    recorder.set_status(201);
    let recovered = daemon.tick().await;
    assert_eq!(recovered.log_events, 1);
    assert_eq!(offset(&dir), start + 37);
    assert_eq!(recorder.count(), 2);
    assert_eq!(recorder.requests()[0].body, recorder.requests()[1].body);

    assert_eq!(daemon.tick().await.log_events, 0);
    assert_eq!(recorder.count(), 2);
}

#[tokio::test]
async fn a_network_outage_keeps_the_offset_for_the_next_tick() {
    let dir = tempfile::tempdir().unwrap();
    seed_log(&dir, HISTORY);
    let dead: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let mut daemon = harness(&dir, dead, "t");
    daemon.tick().await;
    let start = offset(&dir);

    append(&dir, "[Tue Jul 21 21:15:23 2026] You died.\n");
    assert_eq!(daemon.tick().await.retryable_failures, 1);
    assert_eq!(offset(&dir), start);
    assert_eq!(daemon.tick().await.retryable_failures, 1);
    assert_eq!(offset(&dir), start);
}

#[tokio::test]
async fn offsets_survive_a_restart_and_files_without_events_stay_quiet() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    seed_log(&dir, HISTORY);
    let mut daemon = harness(&dir, addr, "t");
    daemon.tick().await;
    drop(daemon);

    append(&dir, "[Tue Jul 21 21:15:23 2026] Dorsk says, 'hello'\n");
    let mut restarted = harness(&dir, addr, "t");
    let report = restarted.tick().await;
    assert_eq!(report.log_events, 0);
    assert_eq!(report.log_lines_dropped, 1);
    assert_eq!(recorder.count(), 0);
    assert_eq!(offset(&dir), HISTORY.len() as u64 + 47);
}

#[tokio::test]
async fn each_character_gets_its_own_batch() {
    let recorder = Recorder::new(201);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    seed_log(&dir, HISTORY);
    let other = dir.path().join("Logs").join("eqlog_Vala_povar.txt");
    std::fs::write(&other, HISTORY).unwrap();
    let mut daemon = harness(&dir, addr, "t");
    daemon.tick().await;

    append(&dir, "[Tue Jul 21 21:15:23 2026] You died.\n");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&other)
        .unwrap();
    file.write_all(b"[Tue Jul 21 21:15:24 2026] You have entered Povar.\n")
        .unwrap();

    assert_eq!(daemon.tick().await.log_events, 2);
    let requests = recorder.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body["character"], "Dorsk");
    assert_eq!(requests[0].body["server"], "erudin");
    assert_eq!(requests[1].body["character"], "Vala");
    assert_eq!(requests[1].body["server"], "povar");
}
