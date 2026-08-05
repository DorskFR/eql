use crate::config::Config;
use crate::tools::{self, Runner};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

pub async fn run(config: &Config, args: &[String]) -> Result<(), InstallError> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("usage: eqld [config.toml] install-tools [--force]");
        return Ok(());
    }
    let force = args.iter().any(|arg| arg == "--force");
    let settings = &config.tools.log_reader;

    if !force {
        if let Some(runner) = Runner::discover(settings.exe.as_deref()) {
            tracing::info!(at = %runner.program().display(), "log reader already installed");
            println!("already installed: {}", runner.program().display());
            return Ok(());
        }
    }

    let version = settings.version().to_string();
    let wanted = tools::asset_name(&version).ok_or(InstallError::UnsupportedPlatform)?;
    let client = reqwest::Client::builder()
        .user_agent("eqld")
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(InstallError::Client)?;

    let url = tools::release_api_url(settings.repo(), &version);
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|source| InstallError::Request {
            url: url.clone(),
            source,
        })?;
    if !response.status().is_success() {
        return Err(InstallError::Release {
            url,
            status: response.status(),
        });
    }
    let release: Release = response
        .json()
        .await
        .map_err(|source| InstallError::Request {
            url: url.clone(),
            source,
        })?;

    let asset = pick_asset(&release.assets, &wanted).ok_or_else(|| InstallError::MissingAsset {
        wanted: wanted.clone(),
        tag: release.tag_name.clone(),
        available: release.assets.iter().map(|a| a.name.clone()).collect(),
    })?;

    tracing::info!(asset = %asset.name, bytes = asset.size, tag = %release.tag_name, "downloading log reader");
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|source| InstallError::Request {
            url: asset.browser_download_url.clone(),
            source,
        })?
        .bytes()
        .await
        .map_err(|source| InstallError::Request {
            url: asset.browser_download_url.clone(),
            source,
        })?;

    let dir = std::env::temp_dir().join("eqld-tools");
    std::fs::create_dir_all(&dir).map_err(InstallError::Io)?;
    let path = dir.join(&asset.name);
    std::fs::write(&path, &bytes).map_err(InstallError::Io)?;

    install_asset(&path).await?;

    match Runner::discover(settings.exe.as_deref()) {
        Some(runner) => {
            tracing::info!(at = %runner.program().display(), "log reader installed");
            println!("installed: {}", runner.program().display());
            Ok(())
        }
        None => Err(InstallError::NotFoundAfterInstall(path)),
    }
}

fn pick_asset<'a>(assets: &'a [Asset], wanted: &str) -> Option<&'a Asset> {
    assets
        .iter()
        .find(|asset| asset.name == wanted)
        .or_else(|| assets.iter().find(|asset| same_shape(&asset.name, wanted)))
}

/// Upstream's tag and asset versions drift (tag `v2.0` ships `…-2.0.1-linux…`),
/// so fall back to matching the prefix and extension rather than the exact name.
fn same_shape(name: &str, wanted: &str) -> bool {
    let split = |value: &str| -> (String, String) {
        let stem = value.split('-').next().unwrap_or("").to_string();
        let ext = value
            .rsplit_once('.')
            .map(|(_, e)| e.to_string())
            .unwrap_or_default();
        (stem, ext)
    };
    split(name) == split(wanted)
}

async fn install_asset(path: &Path) -> Result<(), InstallError> {
    let status = if cfg!(windows) {
        tokio::process::Command::new(path)
            .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"])
            .status()
            .await
    } else {
        let dir = path.parent().unwrap_or(Path::new(".")).join("unpacked");
        std::fs::create_dir_all(&dir).map_err(InstallError::Io)?;
        let untar = tokio::process::Command::new("tar")
            .arg("-xzf")
            .arg(path)
            .arg("-C")
            .arg(&dir)
            .status()
            .await
            .map_err(InstallError::Spawn)?;
        if !untar.success() {
            return Err(InstallError::Unpack(untar.code()));
        }
        let script = find_installer(&dir).ok_or(InstallError::NoInstallScript)?;
        tokio::process::Command::new("sh")
            .arg(&script)
            .current_dir(script.parent().unwrap_or(&dir))
            .status()
            .await
    };
    let status = status.map_err(InstallError::Spawn)?;
    if !status.success() {
        return Err(InstallError::Installer(status.code()));
    }
    Ok(())
}

fn find_installer(dir: &Path) -> Option<PathBuf> {
    let direct = dir.join("install.sh");
    if direct.is_file() {
        return Some(direct);
    }
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let nested = entry.path().join("install.sh");
        nested.is_file().then_some(nested)
    })
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("no upstream package exists for this platform")]
    UnsupportedPlatform,
    #[error("building the http client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("requesting {url}: {source}")]
    Request {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("release lookup {url} returned {status}")]
    Release {
        url: String,
        status: reqwest::StatusCode,
    },
    #[error("release {tag} has no asset like {wanted}; it has {available:?}")]
    MissingAsset {
        wanted: String,
        tag: String,
        available: Vec<String>,
    },
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    #[error("spawning the installer: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("unpacking the archive failed with {0:?}")]
    Unpack(Option<i32>),
    #[error("the archive has no install.sh")]
    NoInstallScript,
    #[error("the installer exited with {0:?}")]
    Installer(Option<i32>),
    #[error("the installer ran but nothing was found afterwards (downloaded {0})")]
    NotFoundAfterInstall(PathBuf),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            browser_download_url: format!("https://example.com/{name}"),
            size: 1,
        }
    }

    #[test]
    fn an_exact_asset_name_wins() {
        let assets = vec![
            asset("EQL-Log-Reader-Setup.exe"),
            asset("eql-log-reader-2.0.1-linux.tar.gz"),
        ];
        assert_eq!(
            pick_asset(&assets, "eql-log-reader-2.0.1-linux.tar.gz")
                .unwrap()
                .name,
            "eql-log-reader-2.0.1-linux.tar.gz"
        );
    }

    #[test]
    fn a_drifted_patch_version_still_matches() {
        let assets = vec![asset("eql-log-reader-2.0.1-linux.tar.gz")];
        assert_eq!(
            pick_asset(&assets, "eql-log-reader-2.0-linux.tar.gz")
                .unwrap()
                .name,
            "eql-log-reader-2.0.1-linux.tar.gz",
            "tag v2.0 ships a 2.0.1 tarball"
        );
    }

    #[test]
    fn an_unrelated_asset_is_not_picked() {
        let assets = vec![asset("neon_hud.rar"), asset("EQL-Log-Reader-Setup.exe")];
        assert!(pick_asset(&assets, "eql-log-reader-2.0.1-linux.tar.gz").is_none());
    }

    /// Our own release carries the daemon beside the log reader, and the
    /// unpinned `latest` has no version to interpolate into the tarball name.
    #[test]
    fn our_release_assets_do_not_shadow_the_log_reader() {
        let assets = vec![
            asset("eqld-windows-x86_64.exe"),
            asset("eqld-macos-aarch64"),
            asset("EQL-Log-Reader-Setup.exe"),
            asset("eql-log-reader-2.0.1-linux.tar.gz"),
        ];
        assert_eq!(
            pick_asset(&assets, "EQL-Log-Reader-Setup.exe")
                .unwrap()
                .name,
            "EQL-Log-Reader-Setup.exe"
        );
        let unpinned = crate::tools::asset_name("latest")
            .and_then(|wanted| pick_asset(&assets, &wanted))
            .map(|asset| asset.name.as_str());
        if cfg!(windows) {
            assert_eq!(unpinned, Some("EQL-Log-Reader-Setup.exe"));
        } else if cfg!(target_os = "linux") {
            assert_eq!(unpinned, Some("eql-log-reader-2.0.1-linux.tar.gz"));
        } else {
            assert_eq!(unpinned, None, "no package exists for this platform");
        }
    }

    #[test]
    fn the_install_script_is_found_at_the_root_or_one_level_down() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(find_installer(dir.path()), None);

        let nested = dir.path().join("eql-log-reader-2.0.1");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("install.sh"), b"#!/bin/sh\n").unwrap();
        assert_eq!(find_installer(dir.path()), Some(nested.join("install.sh")));

        std::fs::write(dir.path().join("install.sh"), b"#!/bin/sh\n").unwrap();
        assert_eq!(
            find_installer(dir.path()),
            Some(dir.path().join("install.sh")),
            "the root script wins"
        );
    }
}
