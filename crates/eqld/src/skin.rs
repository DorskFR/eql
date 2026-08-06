use crate::config::Config;
use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("usage: eqld [config.toml] install-skin <layout-name> [--skin <name>]")]
    Usage,
    #[error("requesting {url}: {source}")]
    Request {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("{url} returned {status}: {body}")]
    Status {
        url: String,
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("the bundle is not a readable zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("bundle entry {0:?} escapes the game root")]
    UnsafePath(String),
    #[error("bundle contains no uifiles/<skin>/ directory")]
    NoSkinDir,
    #[error("{0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub layout: String,
    pub skin: Option<String>,
}

pub fn parse_args(args: &[String]) -> Result<Args, InstallError> {
    let mut layout = None;
    let mut skin = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--skin" => skin = Some(rest.next().ok_or(InstallError::Usage)?.clone()),
            other if other.starts_with("--") => return Err(InstallError::Usage),
            other if layout.is_none() => layout = Some(other.to_string()),
            _ => return Err(InstallError::Usage),
        }
    }
    Ok(Args {
        layout: layout.ok_or(InstallError::Usage)?,
        skin,
    })
}

pub fn bundle_url(config: &Config, args: &Args) -> String {
    let base = config.api.url.trim_end_matches('/');
    let layout = urlencode(&args.layout);
    match &args.skin {
        Some(skin) => format!(
            "{base}/api/v1/layouts/{layout}/bundle?skin={}",
            urlencode(skin)
        ),
        None => format!("{base}/api/v1/layouts/{layout}/bundle"),
    }
}

fn urlencode(raw: &str) -> String {
    raw.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

pub fn changed(previous: Option<&crate::state::SkinState>, args: &Args, digest: &str) -> bool {
    match previous {
        None => true,
        Some(previous) => {
            previous.digest != digest
                || previous.layout != args.layout
                || previous.name.as_deref() != args.skin.as_deref()
        }
    }
}

pub async fn fetch(config: &Config, args: &Args) -> Result<Vec<u8>, InstallError> {
    let url = bundle_url(config, args);
    let response = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&config.api.token)
        .send()
        .await
        .map_err(|source| InstallError::Request {
            url: url.clone(),
            source,
        })?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|source| InstallError::Request {
            url: url.clone(),
            source,
        })?;
    if !status.is_success() {
        return Err(InstallError::Status {
            url,
            status,
            body: String::from_utf8_lossy(&bytes).chars().take(300).collect(),
        });
    }
    Ok(bytes.to_vec())
}

pub async fn run(config: &Config, args: &[String]) -> Result<(), InstallError> {
    let args = parse_args(args)?;
    let bytes = fetch(config, &args).await?;

    let skin = install(&config.game.root, &bytes)?;
    tracing::info!(
        skin = %skin,
        root = %config.game.root.display(),
        "skin installed; now run in game: /loadskin {}",
        skin
    );
    println!("/loadskin {skin}");
    Ok(())
}

/// Returns the skin name found in the bundle, having replaced
/// `<root>/uifiles/<skin>` and any pre-existing ini after backing them up.
pub fn install(root: &Path, bundle: &[u8]) -> Result<String, InstallError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bundle))?;
    let stamp = unix_stamp();

    let mut entries: Vec<(String, Vec<u8>)> = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if !is_safe_entry(&name) {
            return Err(InstallError::UnsafePath(name));
        }
        let mut contents = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut contents)
            .map_err(|err| InstallError::Io(PathBuf::from(&name), err))?;
        entries.push((name, contents));
    }

    let skin = entries
        .iter()
        .find_map(|(name, _)| skin_of(name))
        .ok_or(InstallError::NoSkinDir)?
        .to_string();

    let skin_dir = root.join("uifiles").join(&skin);
    if skin_dir.exists() {
        let backup = sibling(&skin_dir, &stamp);
        std::fs::rename(&skin_dir, &backup)
            .map_err(|err| InstallError::Io(skin_dir.clone(), err))?;
        tracing::info!(from = %skin_dir.display(), to = %backup.display(), "backed up skin");
    }

    for (name, contents) in &entries {
        let target = root.join(name);
        if skin_of(name).is_none() && target.exists() {
            let backup = sibling(&target, &stamp);
            std::fs::copy(&target, &backup).map_err(|err| InstallError::Io(target.clone(), err))?;
            tracing::info!(from = %target.display(), to = %backup.display(), "backed up ini");
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| InstallError::Io(parent.to_path_buf(), err))?;
        }
        std::fs::write(&target, contents).map_err(|err| InstallError::Io(target, err))?;
    }
    Ok(skin)
}

fn sibling(path: &Path, stamp: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".bak.{stamp}"));
    path.with_file_name(name)
}

fn skin_of(entry: &str) -> Option<&str> {
    entry
        .strip_prefix("uifiles/")
        .and_then(|rest| rest.split('/').next())
        .filter(|skin| !skin.is_empty())
}

fn is_safe_entry(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.split('/').any(|part| part == ".." || part.is_empty())
}

fn unix_stamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().to_string())
        .unwrap_or_else(|_| "0".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn args(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|s| (*s).to_string()).collect()
    }

    fn test_config(root: &Path) -> Config {
        toml::from_str(&format!(
            r#"
            [game]
            root = {:?}
            [api]
            url = "http://127.0.0.1:9/"
            token = "s3cret"
            "#,
            root.display().to_string()
        ))
        .unwrap()
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

    const BUNDLE: &[(&str, &str)] = &[
        ("uifiles/dorskui/EQUI_PlayerWindow.xml", "<CX>660</CX>"),
        ("uifiles/dorskui/EQUI_ChatWindow.xml", "chat"),
        ("UI_Dorsk_erudin_LO1.ini", "[MainChat]\r\nWidth=1480\r\n"),
    ];

    #[test]
    fn arguments_take_a_layout_and_an_optional_skin() {
        assert_eq!(
            parse_args(&args(&["dorskui"])).unwrap(),
            Args {
                layout: "dorskui".into(),
                skin: None
            }
        );
        assert_eq!(
            parse_args(&args(&["my layout", "--skin", "v4"]))
                .unwrap()
                .skin,
            Some("v4".into())
        );
        for bad in [
            vec![],
            args(&["--skin"]),
            args(&["a", "b"]),
            args(&["--nope"]),
        ] {
            assert!(
                matches!(parse_args(&bad), Err(InstallError::Usage)),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_bundle_url_percent_encodes_names() {
        let config = test_config(Path::new("/games/eq"));
        assert_eq!(
            bundle_url(&config, &parse_args(&args(&["my layout"])).unwrap()),
            "http://127.0.0.1:9/api/v1/layouts/my%20layout/bundle"
        );
        assert_eq!(
            bundle_url(
                &config,
                &parse_args(&args(&["a", "--skin", "b c"])).unwrap()
            ),
            "http://127.0.0.1:9/api/v1/layouts/a/bundle?skin=b%20c"
        );
    }

    #[test]
    fn installing_lands_files_and_backs_up_on_a_second_run() {
        let root = tempfile::tempdir().unwrap();
        let bundle = zip_of(BUNDLE);

        assert_eq!(install(root.path(), &bundle).unwrap(), "dorskui");
        let xml = root.path().join("uifiles/dorskui/EQUI_PlayerWindow.xml");
        let ini = root.path().join("UI_Dorsk_erudin_LO1.ini");
        assert_eq!(std::fs::read_to_string(&xml).unwrap(), "<CX>660</CX>");
        assert_eq!(
            std::fs::read_to_string(&ini).unwrap(),
            "[MainChat]\r\nWidth=1480\r\n"
        );

        let second = zip_of(&[
            ("uifiles/dorskui/EQUI_PlayerWindow.xml", "<CX>500</CX>"),
            ("UI_Dorsk_erudin_LO1.ini", "[MainChat]\r\nWidth=900\r\n"),
        ]);
        assert_eq!(install(root.path(), &second).unwrap(), "dorskui");
        assert_eq!(std::fs::read_to_string(&xml).unwrap(), "<CX>500</CX>");
        assert_eq!(
            std::fs::read_to_string(&ini).unwrap(),
            "[MainChat]\r\nWidth=900\r\n"
        );

        let names: Vec<String> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let ini_backup = names
            .iter()
            .find(|name| name.starts_with("UI_Dorsk_erudin_LO1.ini.bak."))
            .expect("ini backup");
        assert_eq!(
            std::fs::read_to_string(root.path().join(ini_backup)).unwrap(),
            "[MainChat]\r\nWidth=1480\r\n"
        );

        let skins: Vec<String> = std::fs::read_dir(root.path().join("uifiles"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let skin_backup = skins
            .iter()
            .find(|name| name.starts_with("dorskui.bak."))
            .expect("skin backup");
        assert_eq!(
            std::fs::read_to_string(
                root.path()
                    .join("uifiles")
                    .join(skin_backup)
                    .join("EQUI_ChatWindow.xml")
            )
            .unwrap(),
            "chat",
            "the replaced skin dir is preserved whole, not merged"
        );
        assert!(
            !root
                .path()
                .join("uifiles/dorskui/EQUI_ChatWindow.xml")
                .exists(),
            "the new skin dir must not inherit stale files"
        );
    }

    #[test]
    fn a_bundle_is_only_reinstalled_when_something_about_it_changed() {
        let installed = crate::state::SkinState {
            layout: "dorskui".into(),
            name: Some("v4".into()),
            digest: "abc".into(),
            installed: "dorskui".into(),
            installed_at: Some(1),
        };
        let asked = Args {
            layout: "dorskui".into(),
            skin: Some("v4".into()),
        };
        assert!(!changed(Some(&installed), &asked, "abc"));
        assert!(changed(Some(&installed), &asked, "def"));
        assert!(changed(None, &asked, "abc"));
        assert!(changed(
            Some(&installed),
            &Args {
                layout: "other".into(),
                skin: Some("v4".into())
            },
            "abc"
        ));
        assert!(
            changed(
                Some(&installed),
                &Args {
                    layout: "dorskui".into(),
                    skin: None
                },
                "abc"
            ),
            "dropping --skin asks for the layout's default skin"
        );
    }

    #[test]
    fn traversal_entries_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let evil = zip_of(&[("uifiles/../../etc/passwd", "nope")]);
        assert!(matches!(
            install(root.path(), &evil),
            Err(InstallError::UnsafePath(_))
        ));
        let skinless = zip_of(&[("UI_Dorsk_erudin_LO1.ini", "x")]);
        assert!(matches!(
            install(root.path(), &skinless),
            Err(InstallError::NoSkinDir)
        ));
    }

    #[tokio::test]
    async fn a_real_server_round_trip_installs_and_prints_the_loadskin_hint() {
        let bundle = zip_of(BUNDLE);
        let served = bundle.clone();
        let app = axum::Router::new().route(
            "/api/v1/layouts/{name}/bundle",
            axum::routing::get(move |headers: axum::http::HeaderMap| {
                let served = served.clone();
                async move {
                    if headers.get("authorization").and_then(|v| v.to_str().ok())
                        != Some("Bearer s3cret")
                    {
                        return (axum::http::StatusCode::UNAUTHORIZED, Vec::new());
                    }
                    (axum::http::StatusCode::OK, served)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let root = tempfile::tempdir().unwrap();
        let mut config = test_config(root.path());
        config.api.url = format!("http://{addr}");
        run(&config, &args(&["dorskui"])).await.unwrap();
        assert!(root
            .path()
            .join("uifiles/dorskui/EQUI_PlayerWindow.xml")
            .exists());

        config.api.token = "wrong".into();
        let error = run(&config, &args(&["dorskui"])).await.unwrap_err();
        assert!(
            matches!(&error, InstallError::Status { status, .. } if *status == 401),
            "{error}"
        );
    }
}
