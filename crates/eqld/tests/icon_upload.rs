use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    routing::put,
    Json, Router,
};
use eqld::{config::Config, icons, state::LastStatus};
use serde_json::json;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tempfile::TempDir;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Captured {
    sheet: u32,
    authorization: Option<String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

#[derive(Clone, Default)]
struct Recorder {
    requests: Arc<Mutex<Vec<Captured>>>,
    reject: Arc<Mutex<HashMap<u32, u16>>>,
}

impl Recorder {
    fn requests(&self) -> Vec<Captured> {
        self.requests.lock().unwrap().clone()
    }

    fn sheets(&self) -> Vec<u32> {
        self.requests().iter().map(|r| r.sheet).collect()
    }

    fn reject(&self, sheet: u32, status: u16) {
        self.reject.lock().unwrap().insert(sheet, status);
    }

    fn accept_everything(&self) {
        self.reject.lock().unwrap().clear();
    }
}

async fn ingest(
    State(recorder): State<Recorder>,
    Path(sheet): Path<u32>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<serde_json::Value>) {
    let header = |name: header::HeaderName| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(String::from)
    };
    recorder.requests.lock().unwrap().push(Captured {
        sheet,
        authorization: header(header::AUTHORIZATION),
        content_type: header(header::CONTENT_TYPE),
        body: body.to_vec(),
    });
    if let Some(status) = recorder.reject.lock().unwrap().get(&sheet) {
        return (StatusCode::from_u16(*status).unwrap(), Json(json!({})));
    }
    (StatusCode::OK, Json(json!({ "sheet": sheet, "icons": 36 })))
}

async fn spawn_server(recorder: Recorder) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/api/v1/icons/sheets/{sheet}", put(ingest))
        .with_state(recorder);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

fn sheet_bytes(sheet: u32) -> Vec<u8> {
    let mut bytes = b"DDS ".to_vec();
    bytes.extend_from_slice(&sheet.to_le_bytes());
    bytes
}

fn harness(dir: &TempDir, addr: SocketAddr, sheets: u32) -> Config {
    let art = dir.path().join("uifiles").join("default");
    std::fs::create_dir_all(&art).unwrap();
    for sheet in 1..=sheets {
        std::fs::write(art.join(format!("dragitem{sheet}.dds")), sheet_bytes(sheet)).unwrap();
    }
    std::fs::write(art.join("spells01.dds"), b"not an item sheet").unwrap();

    toml::from_str(&format!(
        r#"
        [game]
        root = {root}
        [api]
        url = "http://{addr}"
        token = "machine-token"
        [state]
        path = {state}
        "#,
        root = toml::Value::from(dir.path().to_str().unwrap()),
        state = toml::Value::from(dir.path().join("state.json").to_str().unwrap()),
    ))
    .unwrap()
}

fn state_of(dir: &TempDir) -> eqld::State {
    eqld::State::load(&dir.path().join("state.json")).unwrap()
}

#[tokio::test]
async fn uploads_every_sheet_verbatim_with_the_machine_token() {
    let recorder = Recorder::default();
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let config = harness(&dir, addr, 3);

    icons::run(&config, &[]).await.unwrap();

    let requests = recorder.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(recorder.sheets(), vec![1, 2, 3], "numeric order, no others");
    for (index, captured) in requests.iter().enumerate() {
        let sheet = index as u32 + 1;
        assert_eq!(
            captured.authorization.as_deref(),
            Some("Bearer machine-token")
        );
        assert_eq!(
            captured.content_type.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(captured.body, sheet_bytes(sheet));
    }

    let state = state_of(&dir);
    assert_eq!(state.icons.len(), 3);
    let entry = &state.icons["dragitem2.dds"];
    assert_eq!(entry.last_status, LastStatus::Uploaded);
    assert_eq!(entry.uploaded_hash.as_ref(), Some(&entry.hash));
    assert!(entry.uploaded_at.is_some());
}

#[tokio::test]
async fn a_second_run_is_a_no_op_until_forced() {
    let recorder = Recorder::default();
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let config = harness(&dir, addr, 3);

    icons::run(&config, &[]).await.unwrap();
    icons::run(&config, &[]).await.unwrap();
    assert_eq!(recorder.requests().len(), 3, "nothing re-uploads");

    icons::run(&config, &["--force".to_string()]).await.unwrap();
    assert_eq!(recorder.sheets(), vec![1, 2, 3, 1, 2, 3]);
}

#[tokio::test]
async fn a_failing_sheet_is_not_fatal_and_retries_on_the_next_run() {
    let recorder = Recorder::default();
    recorder.reject(2, 500);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let config = harness(&dir, addr, 3);

    let error = icons::run(&config, &[]).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "1 of 3 sheets did not upload",
        "the other two still went"
    );
    assert_eq!(recorder.sheets(), vec![1, 2, 3]);
    assert!(matches!(
        state_of(&dir).icons["dragitem2.dds"].last_status,
        LastStatus::Failed { .. }
    ));

    recorder.accept_everything();
    icons::run(&config, &[]).await.unwrap();
    assert_eq!(
        recorder.sheets(),
        vec![1, 2, 3, 2],
        "only the failed sheet is sent again"
    );
    assert_eq!(
        state_of(&dir).icons["dragitem2.dds"].last_status,
        LastStatus::Uploaded
    );
}

#[tokio::test]
async fn a_bad_token_parks_the_sheet_until_it_is_forced() {
    let recorder = Recorder::default();
    recorder.reject(1, 401);
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let config = harness(&dir, addr, 2);

    assert!(icons::run(&config, &[]).await.is_err());
    assert_eq!(
        state_of(&dir).icons["dragitem1.dds"].last_status,
        LastStatus::Rejected { status: 401 }
    );

    recorder.accept_everything();
    icons::run(&config, &[]).await.unwrap();
    assert_eq!(recorder.sheets(), vec![1, 2], "replaying cannot help");

    icons::run(&config, &["--force".to_string()]).await.unwrap();
    assert_eq!(recorder.sheets(), vec![1, 2, 1, 2]);
}

#[tokio::test]
async fn changed_art_uploads_again() {
    let recorder = Recorder::default();
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let config = harness(&dir, addr, 2);

    icons::run(&config, &[]).await.unwrap();
    std::fs::write(
        dir.path().join("uifiles/default/dragitem1.dds"),
        b"DDS patched",
    )
    .unwrap();
    icons::run(&config, &[]).await.unwrap();
    assert_eq!(recorder.sheets(), vec![1, 2, 1]);
}

#[tokio::test]
async fn a_root_without_sheets_is_an_error() {
    let recorder = Recorder::default();
    let addr = spawn_server(recorder.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let config = harness(&dir, addr, 0);

    let error = icons::run(&config, &[]).await.unwrap_err();
    assert!(matches!(error, icons::IconError::NoSheets(_)), "{error}");

    std::fs::remove_dir_all(dir.path().join("uifiles")).unwrap();
    let error = icons::run(&config, &[]).await.unwrap_err();
    assert!(matches!(error, icons::IconError::Io { .. }), "{error}");
    assert_eq!(recorder.requests().len(), 0);
}

#[tokio::test]
async fn a_dead_server_leaves_every_sheet_dirty() {
    let dir = tempfile::tempdir().unwrap();
    let dead: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let config = harness(&dir, dead, 2);

    let error = icons::run(&config, &[]).await.unwrap_err();
    assert_eq!(error.to_string(), "2 of 2 sheets did not upload");
    let state = state_of(&dir);
    for name in ["dragitem1.dds", "dragitem2.dds"] {
        assert!(matches!(
            state.icons[name].last_status,
            LastStatus::Failed { .. }
        ));
        assert!(state.icons[name].uploaded_hash.is_none());
    }
}
