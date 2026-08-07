use crate::config::Config;
use eql_core::layout::Layout;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub type WindowSizes = BTreeMap<String, (i32, i32)>;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
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
    #[error("no [skin] layout is named, so there is nothing to size the export against")]
    NoLayout,
    #[error("cannot read the render size from eqclient.ini; set [skin] screen manually")]
    NoResolution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    pub name: String,
    pub screen_w: i32,
    pub screen_h: i32,
    pub layout: Layout,
    pub digest: String,
}

pub fn ui_ini_path(root: &Path, character: &str, server: &str) -> PathBuf {
    root.join(format!("UI_{character}_{server}_LO1.ini"))
}

/// Fullscreen and windowed are separate key pairs and the client keeps both;
/// windowed wins because that is the mode a layout is ever authored in.
pub fn resolution(root: &Path) -> Option<(i32, i32)> {
    let text = std::fs::read_to_string(root.join("eqclient.ini")).ok()?;
    let find = |key: &str| -> Option<i32> {
        text.lines()
            .map(|line| line.trim_end_matches('\r'))
            .find_map(|line| {
                line.strip_prefix(key)?
                    .strip_prefix('=')?
                    .trim()
                    .parse()
                    .ok()
            })
    };
    let windowed = find("WindowedWidth").zip(find("WindowedHeight"));
    windowed
        .or_else(|| find("Width").zip(find("Height")))
        .filter(|(w, h)| *w > 0 && *h > 0)
}

/// Only the geometry of the windows being tracked feeds the hash. The client
/// rewrites unrelated keys (chat routing, bag positions) constantly, and those
/// must not read as a layout change.
pub fn geometry_digest(text: &str, windows: &WindowSizes) -> String {
    let layout = eql_core::layout::from_ui_ini(text, REFERENCE_SCREEN, REFERENCE_SCREEN, windows);
    crate::daemon::bytes_hash(format!("{:?}", layout.0).as_bytes())
}

/// Arbitrary but fixed: the digest only has to change when the geometry does,
/// and resolving against a constant keeps it stable across resolution changes.
const REFERENCE_SCREEN: i32 = 10_000;

pub fn auto_name(character: &str, server: &str, screen_w: i32, screen_h: i32, unix: i64) -> String {
    let slug = |raw: &str| -> String {
        raw.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect()
    };
    format!(
        "{}-{}-{screen_w}x{screen_h}-{}",
        slug(character),
        slug(server),
        utc_stamp(unix)
    )
}

/// `YYYYmmdd-HHMMSS`, so the generated names sort chronologically in the
/// layout list. Civil-from-days, to avoid a date dependency for one string.
fn utc_stamp(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = era * 400 + yoe + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

pub fn read(
    root: &Path,
    character: &str,
    server: &str,
    sizes: &WindowSizes,
    screen: (i32, i32),
    unix: i64,
) -> Result<Option<Export>, ExportError> {
    let path = ui_ini_path(root, character, server);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(ExportError::Io(path, err)),
    };
    let (screen_w, screen_h) = screen;
    let layout = eql_core::layout::from_ui_ini(&text, screen_w, screen_h, sizes);
    if layout.0.is_empty() {
        return Ok(None);
    }
    Ok(Some(Export {
        name: auto_name(character, server, screen_w, screen_h, unix),
        screen_w,
        screen_h,
        layout,
        digest: geometry_digest(&text, sizes),
    }))
}

pub async fn upload(config: &Config, export: &Export) -> Result<Vec<String>, ExportError> {
    let base = config.api.url.trim_end_matches('/');
    let url = format!("{base}/api/v1/layouts/{}", urlencode(&export.name));
    let body = serde_json::json!({
        "screen_w": export.screen_w,
        "screen_h": export.screen_h,
        "layout": export.layout,
    });
    let response = reqwest::Client::new()
        .put(&url)
        .bearer_auth(&config.api.token)
        .json(&body)
        .send()
        .await
        .map_err(|source| ExportError::Request {
            url: url.clone(),
            source,
        })?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|source| ExportError::Request {
            url: url.clone(),
            source,
        })?;
    if !status.is_success() {
        return Err(ExportError::Status {
            url,
            status,
            body: text.chars().take(300).collect(),
        });
    }
    Ok(serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|value| value.get("problems").cloned())
        .and_then(|problems| serde_json::from_value(problems).ok())
        .unwrap_or_default())
}

pub async fn sizes_for(config: &Config, layout: &str) -> Result<WindowSizes, ExportError> {
    let base = config.api.url.trim_end_matches('/');
    let url = format!("{base}/api/v1/layouts/{}", urlencode(layout));
    let response = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&config.api.token)
        .send()
        .await
        .map_err(|source| ExportError::Request {
            url: url.clone(),
            source,
        })?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|source| ExportError::Request {
            url: url.clone(),
            source,
        })?;
    if !status.is_success() {
        return Err(ExportError::Status {
            url,
            status,
            body: text.chars().take(300).collect(),
        });
    }
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| ExportError::NoLayout)?;
    let layout: Layout = value
        .get("layout")
        .cloned()
        .and_then(|inner| serde_json::from_value(inner).ok())
        .ok_or(ExportError::NoLayout)?;
    Ok(layout
        .rects()
        .map(|(name, rect)| (name.to_string(), (rect.w, rect.h)))
        .collect())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes() -> WindowSizes {
        BTreeMap::from([
            ("PlayerWindow".to_string(), (300, 130)),
            ("BuffWindow".to_string(), (780, 150)),
        ])
    }

    const INI: &str = "[PlayerWindow]\r\nXPos=25.000000%\r\nYPos=50.000000%\r\nWidth=300\r\nHeight=130\r\n[BuffWindow]\r\nXPos=0.000000%\r\nYPos=0.000000%\r\n[Chat 9]\r\nXPos=1.000000%\r\nYPos=2.000000%\r\n";

    #[test]
    fn windowed_size_wins_over_the_fullscreen_pair() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("eqclient.ini"),
            "[Defaults]\r\nWidth=1920\r\nHeight=1080\r\nWindowedWidth=1280\r\nWindowedHeight=720\r\n",
        )
        .unwrap();
        assert_eq!(resolution(dir.path()), Some((1280, 720)));
    }

    #[test]
    fn the_fullscreen_pair_is_the_fallback() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("eqclient.ini"),
            "[Defaults]\r\nWidth=1920\r\nHeight=1080\r\n",
        )
        .unwrap();
        assert_eq!(resolution(dir.path()), Some((1920, 1080)));
    }

    #[test]
    fn a_missing_or_sizeless_eqclient_ini_reads_as_unknown() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolution(dir.path()), None);
        std::fs::write(dir.path().join("eqclient.ini"), "[Defaults]\r\n").unwrap();
        assert_eq!(resolution(dir.path()), None);
    }

    #[test]
    fn churn_outside_the_tracked_windows_is_not_a_layout_change() {
        let noisy = INI.replace("[Chat 9]\r\nXPos=1.000000%", "[Chat 9]\r\nXPos=77.000000%");
        assert_ne!(INI, noisy);
        assert_eq!(
            geometry_digest(INI, &sizes()),
            geometry_digest(&noisy, &sizes()),
            "an untracked window moved and the digest followed it"
        );
    }

    #[test]
    fn moving_a_tracked_window_does_change_the_digest() {
        let moved = INI.replace("XPos=25.000000%", "XPos=26.000000%");
        assert_ne!(
            geometry_digest(INI, &sizes()),
            geometry_digest(&moved, &sizes())
        );
    }

    #[test]
    fn the_digest_survives_a_resolution_change() {
        let before = geometry_digest(INI, &sizes());
        assert_eq!(before, geometry_digest(INI, &sizes()));
    }

    #[test]
    fn generated_names_carry_character_server_screen_and_sort_by_time() {
        let early = auto_name("Dorsk", "erudin", 1280, 720, 1_754_000_000);
        let late = auto_name("Dorsk", "erudin", 1280, 720, 1_754_086_400);
        assert_eq!(early, "dorsk-erudin-1280x720-20250731-221320");
        assert!(early < late, "{early} !< {late}");
    }

    #[test]
    fn awkward_characters_in_a_name_cannot_escape_the_url() {
        let name = auto_name("Bo b/../x", "eru din", 800, 600, 0);
        assert_eq!(name, "bo_b____x-eru_din-800x600-19700101-000000");
        assert!(!name.contains(['/', '.']), "{name}");
    }

    #[test]
    fn reading_an_absent_ini_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read(dir.path(), "Dorsk", "erudin", &sizes(), (1280, 720), 0).unwrap(),
            None
        );
    }

    #[test]
    fn a_read_export_carries_the_pixels_the_ini_described() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(ui_ini_path(dir.path(), "Dorsk", "erudin"), INI).unwrap();
        let export = read(dir.path(), "Dorsk", "erudin", &sizes(), (1280, 720), 0)
            .unwrap()
            .expect("the ini positions tracked windows");
        assert_eq!(export.screen_w, 1280);
        assert_eq!(export.layout.0["PlayerWindow"], (320, 360, 300, 130));
        assert_eq!(export.layout.0["BuffWindow"], (0, 0, 780, 150));
    }
}
