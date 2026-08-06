#![cfg(target_os = "linux")]

use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
use eqld::{config::Config, daemon::Daemon};
use serde_json::{json, Value};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc, Mutex,
    },
};
use tempfile::TempDir;

#[derive(Debug, Clone)]
struct Captured {
    authorization: Option<String>,
    body: Value,
}

#[derive(Clone)]
struct Recorder {
    requests: Arc<Mutex<Vec<Captured>>>,
    status: Arc<AtomicU16>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            requests: Arc::default(),
            status: Arc::new(AtomicU16::new(200)),
        }
    }

    fn requests(&self) -> Vec<Captured> {
        self.requests.lock().unwrap().clone()
    }

    fn answer(&self, status: u16) {
        self.status.store(status, Ordering::SeqCst);
    }
}

async fn ingest(
    State(recorder): State<Recorder>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (axum::http::StatusCode, Json<Value>) {
    let received = body["fights"].as_array().map(Vec::len).unwrap_or_default();
    recorder.requests.lock().unwrap().push(Captured {
        authorization: headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(String::from),
        body,
    });
    let status = axum::http::StatusCode::from_u16(recorder.status.load(Ordering::SeqCst)).unwrap();
    (
        status,
        Json(json!({ "received": received, "stored": received })),
    )
}

async fn spawn_server(recorder: Recorder) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/api/v1/fights", post(ingest))
        .with_state(recorder);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

fn install(dir: &TempDir, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.path().join(name);
    let staged = dir.path().join(format!("{name}.staged"));
    std::fs::write(&staged, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, &path).unwrap();
    path
}

fn fight(start: f64, zone: &str) -> Value {
    json!({
        "start_wall": start,
        "zone": zone,
        "span": 158.0,
        "active_secs": 158.0,
        "enemies": ["a greater skeleton"],
        "allies": [],
        "dmg_out_you": 7654,
        "dmg_in_you": 3142,
        "heal_out": 2166,
        "kills": 5,
        "deaths": 1,
        "stance": "Mage Hunter Stance",
        "invocation": "Spellblade",
        "abilities_dmg": { "Melee": { "total": 4322, "hits": 116 } }
    })
}

/// A stand-in for the patched `eql_fights_cli`: it honours `--out` and
/// answers `--since` from a second canned dump, and records its own argv.
fn install_fights_tool(tools: &TempDir, first: &Value, second: &Value) -> PathBuf {
    let first_path = tools.path().join("first.json");
    let second_path = tools.path().join("second.json");
    std::fs::write(&first_path, first.to_string()).unwrap();
    std::fs::write(&second_path, second.to_string()).unwrap();
    let argv = tools.path().join("argv.txt");
    install(
        tools,
        "eql_fights_cli",
        &format!(
            r#"echo "$@" >> {argv}
out=""
since=""
while [ $# -gt 0 ]; do
  case "$1" in
    --out) out=$2; shift ;;
    --since) since=$2; shift ;;
  esac
  shift
done
mkdir -p "$(dirname "$out")"
if [ -z "$since" ]; then cp {first} "$out"; else cp {second} "$out"; fi"#,
            argv = argv.display(),
            first = first_path.display(),
            second = second_path.display(),
        ),
    );
    argv
}

fn harness(root: &TempDir, tools: &TempDir, addr: SocketAddr) -> Config {
    let logs = root.path().join("Logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("eqlog_Dorsk_erudin.txt"), "").unwrap();
    install(tools, "eql_atlas", "exit 0");

    toml::from_str(&format!(
        r#"
        [game]
        root = "{root}"
        [api]
        url = "http://{addr}"
        token = "machine-token"
        [state]
        path = "{state}"
        [tools.log_reader]
        enabled = true
        exe = "{exe}"
        replay_secs = 0
        "#,
        root = root.path().display(),
        state = root.path().join("state.json").display(),
        exe = tools.path().join("eql_atlas").display(),
    ))
    .unwrap()
}

/// A fresh daemon per tick: the replay beat is clamped to ten seconds, and
/// the state file is what carries the watermark between runs anyway.
async fn tick(config: &Config) -> (eqld::TickReport, eqld::State) {
    let mut daemon = Daemon::new(config.clone()).unwrap();
    let report = daemon.tick().await;
    (report, daemon.state().clone())
}

fn argv_lines(argv: &Path) -> Vec<String> {
    std::fs::read_to_string(argv)
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

#[tokio::test]
async fn new_fights_are_uploaded_once_and_the_watermark_moves() {
    let recorder = Recorder::new();
    let addr = spawn_server(recorder.clone()).await;
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    let argv = install_fights_tool(
        &tools,
        &json!({ "character": "Dorsk", "server": "erudin",
                 "fights": [fight(1785931682.0, "Najena"), fight(1785931338.0, "Najena")] }),
        &json!({ "character": "Dorsk", "server": "erudin",
                 "fights": [fight(1785933427.0, "The Estate of Unrest")] }),
    );
    let config = harness(&root, &tools, addr);

    let (report, state) = tick(&config).await;
    assert_eq!(report.fights, 2);
    let first = &recorder.requests()[0];
    assert_eq!(
        first.authorization.as_deref(),
        Some("Bearer machine-token"),
        "the machine token rides along"
    );
    assert_eq!(first.body["character"], "Dorsk");
    assert_eq!(first.body["server"], "erudin");
    let sent = first.body["fights"].as_array().unwrap();
    assert_eq!(
        sent.iter()
            .map(|fight| fight["start_wall"].as_f64().unwrap())
            .collect::<Vec<_>>(),
        vec![1785931338.0, 1785931682.0],
        "oldest first, whatever order the tool emitted"
    );
    assert_eq!(sent[0]["zone"], "Najena", "the zone travels with the fight");
    assert_eq!(sent[0]["abilities_dmg"]["Melee"]["total"], 4322);

    assert_eq!(
        state.fights["eqlog_Dorsk_erudin.txt"].last_start_wall_ms,
        1_785_931_682_000
    );
    assert_eq!(state.fights["eqlog_Dorsk_erudin.txt"].uploaded, 2);

    let (report, _) = tick(&config).await;
    assert_eq!(report.fights, 1, "only the new one");
    assert_eq!(recorder.requests().len(), 2);
    assert_eq!(
        recorder.requests()[1].body["fights"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let lines = argv_lines(&argv);
    assert_eq!(lines.len(), 2);
    assert!(!lines[0].contains("--since"), "{}", lines[0]);
    assert!(
        lines[1].contains("--since 1785931682.000"),
        "the second run asks only for what came after: {}",
        lines[1]
    );
    assert!(lines[1].contains("eqlog_Dorsk_erudin.txt"));

    let (report, state) = tick(&config).await;
    assert_eq!(report.fights, 0, "and then nothing");
    assert_eq!(recorder.requests().len(), 2, "no empty posts");
    assert_eq!(state.fights["eqlog_Dorsk_erudin.txt"].uploaded, 3);
}

#[tokio::test]
async fn the_same_dump_twice_is_sent_once() {
    let recorder = Recorder::new();
    let addr = spawn_server(recorder.clone()).await;
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    let same = json!({ "character": "Dorsk", "server": "erudin",
                       "fights": [fight(1785931338.0, "Najena")] });
    install_fights_tool(&tools, &same, &same);
    let config = harness(&root, &tools, addr);

    assert_eq!(tick(&config).await.0.fights, 1);
    assert_eq!(
        tick(&config).await.0.fights,
        0,
        "a tool that ignores --since still cannot re-post history"
    );
    assert_eq!(recorder.requests().len(), 1);
}

#[tokio::test]
async fn a_rejected_batch_is_replayed_on_the_next_tick() {
    let recorder = Recorder::new();
    recorder.answer(500);
    let addr = spawn_server(recorder.clone()).await;
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    let dump = json!({ "character": "Dorsk", "server": "erudin",
                       "fights": [fight(1785931338.0, "Najena")] });
    install_fights_tool(&tools, &dump, &dump);
    let config = harness(&root, &tools, addr);

    let (report, state) = tick(&config).await;
    assert_eq!(report.fights, 0);
    assert_eq!(report.retryable_failures, 1);
    assert!(
        state.fights.is_empty(),
        "the watermark only moves on an accepted batch"
    );

    recorder.answer(200);
    let (report, state) = tick(&config).await;
    assert_eq!(report.fights, 1);
    assert_eq!(recorder.requests().len(), 2, "the same fight, sent again");
    assert_eq!(
        state.fights["eqlog_Dorsk_erudin.txt"].last_start_wall_ms,
        1_785_931_338_000
    );
}

#[tokio::test]
async fn a_log_reader_without_the_fights_tool_ships_nothing() {
    let recorder = Recorder::new();
    let addr = spawn_server(recorder.clone()).await;
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    let config = harness(&root, &tools, addr);

    let (report, state) = tick(&config).await;
    assert_eq!(report.fights, 0);
    assert!(recorder.requests().is_empty());
    assert!(state.fights.is_empty());
}

#[tokio::test]
async fn a_dump_that_is_not_json_is_counted_and_skipped() {
    let recorder = Recorder::new();
    let addr = spawn_server(recorder.clone()).await;
    let root = TempDir::new().unwrap();
    let tools = TempDir::new().unwrap();
    install(
        &tools,
        "eql_fights_cli",
        r#"out=""
while [ $# -gt 0 ]; do case "$1" in --out) out=$2; shift ;; esac; shift; done
mkdir -p "$(dirname "$out")"
printf 'half a fi' > "$out""#,
    );
    let config = harness(&root, &tools, addr);

    let (report, _) = tick(&config).await;
    assert_eq!(report.fights, 0);
    assert_eq!(report.parse_failures, 1);
    assert!(recorder.requests().is_empty());
}
