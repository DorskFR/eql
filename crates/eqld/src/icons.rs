use crate::{
    config::Config,
    daemon::{bytes_hash, decide, unix_secs, Decision},
    state::{FileState, LastStatus, State},
};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub fn sheet_dir(root: &Path) -> PathBuf {
    root.join("uifiles").join("default")
}

pub fn sheet_number(file_name: &str) -> Option<u32> {
    let lower = file_name.to_ascii_lowercase();
    let digits = lower.strip_prefix("dragitem")?.strip_suffix(".dds")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

pub fn scan(dir: &Path) -> std::io::Result<Vec<(u32, PathBuf)>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let Some(sheet) = entry.file_name().to_str().and_then(sheet_number) else {
            continue;
        };
        found.push((sheet, entry.path()));
    }
    found.sort();
    Ok(found)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UploadReport {
    pub uploaded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub icons: usize,
}

#[derive(Debug, Deserialize)]
struct Ack {
    icons: usize,
}

/// Game art never changes, so an accepted sheet is skipped on later runs
/// unless `--force` is given.
pub async fn run(config: &Config, args: &[String]) -> Result<(), IconError> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("usage: eqld [config.toml] upload-icons [--force]");
        return Ok(());
    }
    let mut force = false;
    for arg in args {
        match arg.as_str() {
            "--force" => force = true,
            _ => return Err(IconError::Usage),
        }
    }

    let dir = sheet_dir(&config.game.root);
    let sheets = scan(&dir).map_err(|source| IconError::Io {
        path: dir.clone(),
        source,
    })?;
    if sheets.is_empty() {
        return Err(IconError::NoSheets(dir));
    }

    let client = reqwest::Client::builder()
        .user_agent("eqld")
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(IconError::Client)?;
    let state_path = config.state_path();
    let mut state = State::load(&state_path)?;

    tracing::info!(dir = %dir.display(), sheets = sheets.len(), force, "uploading icon sheets");
    let mut report = UploadReport::default();
    for (sheet, path) in &sheets {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let read = std::fs::metadata(path).and_then(|metadata| {
            let bytes = std::fs::read(path)?;
            Ok((metadata.modified().ok().and_then(unix_secs), bytes))
        });
        let (mtime, bytes) = match read {
            Ok(read) => read,
            Err(err) => {
                report.failed += 1;
                tracing::warn!(file = %path.display(), %err, "cannot read sheet, skipping");
                continue;
            }
        };

        let len = bytes.len() as u64;
        let hash = bytes_hash(&bytes);
        if !force && decide(state.icons.get(&file_name), &hash) != Decision::Upload {
            report.skipped += 1;
            continue;
        }

        let last_status = match upload(&client, config, *sheet, bytes).await {
            Ok(Outcome::Accepted { icons }) => {
                report.uploaded += 1;
                report.icons += icons;
                tracing::debug!(sheet, icons, "uploaded icon sheet");
                LastStatus::Uploaded
            }
            Ok(Outcome::ServerError { status }) => {
                report.failed += 1;
                tracing::warn!(sheet, status, "icon sheet hit a server error, will retry");
                LastStatus::Failed {
                    error: format!("http {status}"),
                }
            }
            Ok(Outcome::Rejected { status }) => {
                report.failed += 1;
                tracing::error!(
                    sheet,
                    status,
                    "icon sheet rejected, parked — rerun with --force once the cause is fixed"
                );
                LastStatus::Rejected { status }
            }
            Err(err) => {
                report.failed += 1;
                tracing::warn!(sheet, %err, "icon sheet upload failed");
                LastStatus::Failed {
                    error: err.to_string(),
                }
            }
        };

        let uploaded = matches!(last_status, LastStatus::Uploaded);
        state.icons.insert(
            file_name,
            FileState {
                mtime,
                len,
                hash: hash.clone(),
                uploaded_hash: uploaded.then_some(hash),
                uploaded_at: uploaded.then(|| unix_secs(SystemTime::now())).flatten(),
                last_status,
            },
        );
    }

    state.save(&state_path)?;
    tracing::info!(
        uploaded = report.uploaded,
        skipped = report.skipped,
        failed = report.failed,
        icons = report.icons,
        "icon sheets done"
    );
    println!(
        "icon sheets: {} uploaded ({} icons), {} skipped, {} failed",
        report.uploaded, report.icons, report.skipped, report.failed
    );
    if report.failed > 0 {
        return Err(IconError::Incomplete {
            failed: report.failed,
            total: sheets.len(),
        });
    }
    Ok(())
}

enum Outcome {
    Accepted { icons: usize },
    ServerError { status: u16 },
    Rejected { status: u16 },
}

async fn upload(
    client: &reqwest::Client,
    config: &Config,
    sheet: u32,
    bytes: Vec<u8>,
) -> reqwest::Result<Outcome> {
    let response = client
        .put(config.icon_sheet_endpoint(sheet))
        .bearer_auth(&config.api.token)
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(bytes)
        .send()
        .await?;
    let status = response.status();
    if status.is_server_error() {
        return Ok(Outcome::ServerError {
            status: status.as_u16(),
        });
    }
    if !status.is_success() {
        return Ok(Outcome::Rejected {
            status: status.as_u16(),
        });
    }
    let body = response.text().await?;
    Ok(Outcome::Accepted {
        icons: serde_json::from_str::<Ack>(&body).map_or(0, |ack| ack.icons),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum IconError {
    #[error("usage: eqld [config.toml] upload-icons [--force]")]
    Usage,
    #[error("building the http client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} holds no dragitem*.dds sheets; is game.root the client directory?")]
    NoSheets(PathBuf),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error("{failed} of {total} sheets did not upload")]
    Incomplete { failed: usize, total: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_numbered_dragitem_sheets_are_recognised() {
        assert_eq!(sheet_number("dragitem1.dds"), Some(1));
        assert_eq!(sheet_number("dragitem379.dds"), Some(379));
        assert_eq!(sheet_number("DragItem42.DDS"), Some(42));
        assert_eq!(sheet_number("dragitem.dds"), None);
        assert_eq!(sheet_number("dragitem+1.dds"), None);
        assert_eq!(sheet_number("dragitem1x.dds"), None);
        assert_eq!(sheet_number("dragitem1.dds.bak"), None);
        assert_eq!(sheet_number("gemicons01.dds"), None);
        assert_eq!(sheet_number("1.dds"), None);
    }

    #[test]
    fn sheets_are_scanned_in_numeric_order_and_nothing_else_is_picked_up() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "dragitem10.dds",
            "dragitem2.dds",
            "dragitem1.dds",
            "spells01.dds",
            "EQUI_Animations.xml",
        ] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        std::fs::create_dir(dir.path().join("dragitem99.dds")).unwrap();

        let found: Vec<u32> = scan(dir.path())
            .unwrap()
            .into_iter()
            .map(|(sheet, _)| sheet)
            .collect();
        assert_eq!(found, vec![1, 2, 10]);
    }

    #[test]
    fn the_sheet_directory_hangs_off_the_game_root() {
        assert_eq!(
            sheet_dir(Path::new("/games/eq")),
            Path::new("/games/eq/uifiles/default")
        );
    }

    #[tokio::test]
    async fn arguments_are_a_bare_optional_force() {
        let config: Config = toml::from_str(
            r#"
            [game]
            root = "/games/eq"
            [api]
            url = "http://127.0.0.1:9"
            token = "t"
            "#,
        )
        .unwrap();
        for bad in [vec!["--nope".to_string()], vec!["extra".to_string()]] {
            assert!(matches!(run(&config, &bad).await, Err(IconError::Usage)));
        }
        assert!(run(&config, &["--help".to_string()]).await.is_ok());
    }
}
