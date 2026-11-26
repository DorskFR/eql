use eqld::{config::Config, daemon::Daemon};
use std::{
    net::SocketAddr,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

/// Accepts every connection and never answers, like a half-open peer.
async fn spawn_silent_server() -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            held.push(socket);
        }
    });
    (addr, accepted)
}

fn config(root: &Path, addr: SocketAddr, skin: &str) -> Config {
    toml::from_str(&format!(
        r#"
        [game]
        root = {root}
        process = "eqg-off.exe"
        [api]
        url = "http://{addr}"
        token = "t"
        [state]
        path = {state}
        [skin]
        enabled = true
        export = true
        check_secs = 0
        screen = [1920, 1080]
        {skin}
        "#,
        root = toml::Value::from(root.to_str().unwrap()),
        state = toml::Value::from(root.join("state.json").to_str().unwrap()),
    ))
    .unwrap()
}

#[tokio::test]
async fn a_server_that_never_answers_does_not_stall_the_tick() {
    for skin in [r#"channel = "dorskui""#, r#"layout = "dorskui""#] {
        let (addr, accepted) = spawn_silent_server().await;
        let root = tempfile::tempdir().unwrap();
        let mut daemon = Daemon::with_request_timeout(
            config(root.path(), addr, skin),
            Duration::from_millis(300),
        )
        .unwrap();

        let report = tokio::time::timeout(Duration::from_secs(20), daemon.tick())
            .await
            .unwrap_or_else(|_| panic!("{skin}: tick stalled on a silent server"));
        assert_eq!(report.skins, 0, "{skin}");
        assert_eq!(report.exports, 0, "{skin}");
        assert!(
            accepted.load(Ordering::SeqCst) >= 2,
            "{skin}: both the skin and the export sync reached the server"
        );
    }
}
