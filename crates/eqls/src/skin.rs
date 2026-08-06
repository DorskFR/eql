use eql_core::layout::{Layout, Rect};
use std::io::Write;

pub const INI_NAME: &str = "UI_Dorsk_erudin_LO1.ini";
pub const TEMPLATE_WIDTH: i32 = 3840;
pub const TEMPLATE_HEIGHT: i32 = 2160;

const LAYOUT_JSON: &str = include_str!("../template/layout.json");
const INI: &str = include_str!("../template/UI_Dorsk_erudin_LO1.ini");

macro_rules! xml_files {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_str!(concat!("../template/dorskui/", $name)))),*]
    };
}

const XML_FILES: &[(&str, &str)] = xml_files![
    "EQUI_BuffWindow.xml",
    "EQUI_CastingWindow.xml",
    "EQUI_CastSpellWnd.xml",
    "EQUI_ChatWindow.xml",
    "EQUI_ExtendedTargetWnd.xml",
    "EQUI_GroupWindow.xml",
    "EQUI_HotButtonWnd.xml",
    "EQUI_PetInfoWindow.xml",
    "EQUI_PlayerWindow.xml",
    "EQUI_ShortDurationBuffWindow.xml",
    "EQUI_TargetWindow.xml",
];

/// Where a window's size lives. Position is always the ini's XPos/YPos pair;
/// `MainChat` and `Chat 1` are ini-driven instances of ChatWindow with no
/// `<Screen>` of their own, so their size is ini-only too.
const WINDOWS: &[(&str, Option<&str>)] = &[
    ("BuffWindow", Some("EQUI_BuffWindow.xml")),
    ("CastSpellWnd", Some("EQUI_CastSpellWnd.xml")),
    ("CastingWindow", Some("EQUI_CastingWindow.xml")),
    ("Chat 1", None),
    ("ExtendedTargetWnd", Some("EQUI_ExtendedTargetWnd.xml")),
    ("GroupWindow", Some("EQUI_GroupWindow.xml")),
    ("HotButtonWnd", Some("EQUI_HotButtonWnd.xml")),
    ("HotButtonWnd2", Some("EQUI_HotButtonWnd.xml")),
    ("MainChat", None),
    ("PetInfoWindow", Some("EQUI_PetInfoWindow.xml")),
    ("PlayerWindow", Some("EQUI_PlayerWindow.xml")),
    (
        "ShortDurationBuffWindow",
        Some("EQUI_ShortDurationBuffWindow.xml"),
    ),
    ("TargetWindow", Some("EQUI_TargetWindow.xml")),
];

#[derive(Debug, thiserror::Error)]
pub enum SkinError {
    #[error("unknown window {0:?}: not part of the dorskui template")]
    UnknownWindow(String),
    #[error("template {file} has no <Screen item=\"{item}\"> with a <Size>")]
    MissingScreenSize { file: String, item: String },
    #[error("template ini has no [{0}] section")]
    MissingIniSection(String),
    #[error("skin name must contain at least one of a-z, 0-9 or _")]
    EmptySkinName,
    #[error("screen size must be positive, got {0}x{1}")]
    BadScreen(i32, i32),
    #[error("packaging the bundle: {0}")]
    Zip(#[from] zip::result::ZipError),
}

pub fn template_windows() -> impl Iterator<Item = &'static str> {
    WINDOWS.iter().map(|(name, _)| *name)
}

pub fn default_layout() -> Layout {
    serde_json::from_str(LAYOUT_JSON).expect("embedded template layout.json is valid")
}

/// Lowercases and replaces every other byte with `_`, so the name is safe both
/// as a path segment and as a `/loadskin` argument.
pub fn sanitize_skin_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| match c.to_ascii_lowercase() {
            c @ ('a'..='z' | '0'..='9' | '_') => c,
            _ => '_',
        })
        .collect();
    cleaned.trim_matches('_').to_string()
}

pub fn generate_bundle(
    layout: &Layout,
    skin_name: &str,
    screen_w: i32,
    screen_h: i32,
) -> Result<Vec<(String, Vec<u8>)>, SkinError> {
    if screen_w <= 0 || screen_h <= 0 {
        return Err(SkinError::BadScreen(screen_w, screen_h));
    }
    let skin = sanitize_skin_name(skin_name);
    if skin.is_empty() {
        return Err(SkinError::EmptySkinName);
    }

    let mut by_file: Vec<(&str, Vec<(&str, Rect)>)> = XML_FILES
        .iter()
        .map(|(name, _)| (*name, Vec::new()))
        .collect();
    let mut ini_targets: Vec<(&str, Rect)> = Vec::new();

    for (name, rect) in layout.rects() {
        let (window, file) = WINDOWS
            .iter()
            .find(|(window, _)| *window == name)
            .ok_or_else(|| SkinError::UnknownWindow(name.to_string()))?;
        ini_targets.push((window, rect));
        if let Some(file) = file {
            let slot = by_file
                .iter_mut()
                .find(|(candidate, _)| candidate == file)
                .expect("every mapped file is a template file");
            slot.1.push((window, rect));
        }
    }

    let mut files = Vec::with_capacity(XML_FILES.len() + 1);
    for (name, source) in XML_FILES {
        let edits = &by_file
            .iter()
            .find(|(candidate, _)| candidate == name)
            .expect("by_file mirrors XML_FILES")
            .1;
        let mut text = (*source).to_string();
        for (item, rect) in edits {
            text = patch_screen_size(&text, item, rect.w, rect.h).ok_or_else(|| {
                SkinError::MissingScreenSize {
                    file: (*name).to_string(),
                    item: (*item).to_string(),
                }
            })?;
        }
        files.push((format!("uifiles/{skin}/{name}"), text.into_bytes()));
    }

    files.push((
        INI_NAME.to_string(),
        patch_ini(INI, &ini_targets, screen_w, screen_h)?.into_bytes(),
    ));
    Ok(files)
}

pub fn zip_bundle(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, SkinError> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (path, contents) in files {
        writer.start_file(path, options)?;
        writer
            .write_all(contents)
            .map_err(zip::result::ZipError::Io)?;
    }
    Ok(writer.finish()?.into_inner())
}

/// Replaces the `<Size>` of `<Screen item="…">`. `<Screen>` blocks are never
/// nested in EQUI, and the screen's own `<Size>` is the first one inside.
fn patch_screen_size(text: &str, item: &str, w: i32, h: i32) -> Option<String> {
    let open = format!("<Screen item=\"{item}\">");
    let start = text.find(&open)? + open.len();
    let end = start + text[start..].find("</Screen>")?;
    let block = &text[start..end];

    let size_start = start + block.find("<Size>")? + "<Size>".len();
    let size_end = size_start + text[size_start..end].find("</Size>")?;
    let size = &text[size_start..size_end];

    let cx = replace_tag(size, "CX", w)?;
    let patched = replace_tag(&cx, "CY", h)?;
    Some(format!(
        "{}{patched}{}",
        &text[..size_start],
        &text[size_end..]
    ))
}

fn replace_tag(block: &str, tag: &str, value: i32) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = block.find(&open)? + open.len();
    let end = start + block[start..].find(&close)?;
    Some(format!("{}{value}{}", &block[..start], &block[end..]))
}

/// Positions are percentages of the screen; `Width`/`Height` are rewritten only
/// where the template already carries them. Every other line — including the
/// server-authoritative `ChannelMap0..107` routing — is copied byte for byte.
fn patch_ini(
    text: &str,
    targets: &[(&str, Rect)],
    screen_w: i32,
    screen_h: i32,
) -> Result<String, SkinError> {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut current: Option<Rect> = None;

    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let ending = &line[body.len()..];

        if let Some(section) = body.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            current = targets
                .iter()
                .find(|(name, _)| *name == section)
                .map(|(name, rect)| {
                    seen.push(name);
                    *rect
                });
            out.push_str(line);
            continue;
        }

        let replacement = current.and_then(|rect| match body.split_once('=') {
            Some(("XPos", _)) => Some(percent(rect.x, screen_w)),
            Some(("YPos", _)) => Some(percent(rect.y, screen_h)),
            Some(("Width", _)) => Some(rect.w.to_string()),
            Some(("Height", _)) => Some(rect.h.to_string()),
            _ => None,
        });
        match replacement {
            Some(value) => {
                let key = body.split_once('=').expect("matched above").0;
                out.push_str(key);
                out.push('=');
                out.push_str(&value);
                out.push_str(ending);
            }
            None => out.push_str(line),
        }
    }

    if let Some((missing, _)) = targets.iter().find(|(name, _)| !seen.contains(name)) {
        return Err(SkinError::MissingIniSection((*missing).to_string()));
    }
    Ok(out)
}

fn percent(value: i32, total: i32) -> String {
    format!("{:.6}%", f64::from(value) / f64::from(total) * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Read;

    fn bundle_map(files: &[(String, Vec<u8>)]) -> BTreeMap<&str, &str> {
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), std::str::from_utf8(bytes).unwrap()))
            .collect()
    }

    fn generate(layout: &Layout) -> Vec<(String, Vec<u8>)> {
        generate_bundle(layout, "dorskui", TEMPLATE_WIDTH, TEMPLATE_HEIGHT).unwrap()
    }

    fn phone_layout() -> Layout {
        Layout(BTreeMap::from([
            ("BuffWindow".to_string(), (0, 0, 780, 150)),
            ("ShortDurationBuffWindow".to_string(), (790, 0, 490, 150)),
            ("PlayerWindow".to_string(), (0, 160, 300, 130)),
            ("PetInfoWindow".to_string(), (0, 300, 300, 110)),
            ("TargetWindow".to_string(), (980, 160, 300, 130)),
            ("ExtendedTargetWnd".to_string(), (980, 300, 300, 110)),
            ("CastSpellWnd".to_string(), (0, 430, 350, 140)),
            ("CastingWindow".to_string(), (360, 430, 330, 60)),
            ("GroupWindow".to_string(), (700, 430, 280, 140)),
            ("Chat 1".to_string(), (990, 430, 290, 140)),
            ("HotButtonWnd".to_string(), (0, 580, 350, 140)),
            ("HotButtonWnd2".to_string(), (360, 580, 350, 140)),
            ("MainChat".to_string(), (720, 580, 560, 140)),
        ]))
    }

    #[test]
    fn every_default_layout_window_is_mapped() {
        let layout = default_layout();
        assert_eq!(layout.0.len(), 13);
        for (name, _) in layout.rects() {
            assert!(
                WINDOWS.iter().any(|(window, _)| *window == name),
                "{name} is missing from the template mapping"
            );
        }
        for (window, file) in WINDOWS {
            assert!(
                layout.0.contains_key(*window),
                "{window} not in layout.json"
            );
            if let Some(file) = file {
                assert!(XML_FILES.iter().any(|(name, _)| name == file));
            }
        }
    }

    #[test]
    fn a_generated_ini_reads_back_as_the_layout_that_made_it() {
        for (layout, w, h) in [
            (default_layout(), TEMPLATE_WIDTH, TEMPLATE_HEIGHT),
            (phone_layout(), 1280, 720),
        ] {
            let files = generate_bundle(&layout, "dorskui", w, h).unwrap();
            let ini = bundle_map(&files)[INI_NAME];
            let sizes = layout
                .rects()
                .map(|(name, rect)| (name.to_string(), (rect.w, rect.h)))
                .collect();
            assert_eq!(
                eql_core::layout::from_ui_ini(ini, w, h, &sizes),
                layout,
                "{w}x{h} did not survive the round trip"
            );
        }
    }

    /// Anchors in the wild are a mix of left/center/right; the fixture the
    /// generator ships is uniform, so this pins the ones it does not write.
    #[test]
    fn anchors_the_generator_never_writes_do_not_move_a_window() {
        let sizes = BTreeMap::from([("PlayerWindow".to_string(), (300, 130))]);
        let read = |xref: &str, yref: &str| {
            eql_core::layout::from_ui_ini(
                &format!(
                    "[PlayerWindow]\r\nXRef={xref}\r\nYRef={yref}\r\nXPos=25.000000%\r\nYPos=50.000000%\r\nWidth=300\r\nHeight=130\r\n"
                ),
                1280,
                720,
                &sizes,
            )
        };
        assert_eq!(read("left", "top"), read("center", "bottom"));
        assert_eq!(read("left", "top"), read("right", "center"));
    }

    #[test]
    fn the_default_layout_regenerates_the_template_verbatim() {
        let files = generate(&default_layout());
        let map = bundle_map(&files);
        for (name, source) in XML_FILES {
            let generated = map[format!("uifiles/dorskui/{name}").as_str()];
            if *name == "EQUI_HotButtonWnd.xml" {
                continue;
            }
            assert_eq!(generated, *source, "{name} drifted from the template");
        }
        assert_eq!(map[INI_NAME], INI);
    }

    /// The hand-made template left HotButtonWnd2's `<Size>` at its 525x53
    /// default because it is Style_Sizable and the ini's 800x90 wins; the
    /// generator normalises the XML to agree.
    #[test]
    fn the_one_template_window_whose_xml_disagreed_with_the_ini_is_normalised() {
        let files = generate(&default_layout());
        let generated = bundle_map(&files)["uifiles/dorskui/EQUI_HotButtonWnd.xml"];
        assert_eq!(screen_size(INI_HOTBUTTON, "HotButtonWnd2"), (525, 53));
        assert_eq!(screen_size(generated, "HotButtonWnd2"), (800, 90));
        assert_eq!(screen_size(generated, "HotButtonWnd"), (800, 90));
        assert_eq!(
            without_screen(generated, "HotButtonWnd2"),
            without_screen(INI_HOTBUTTON, "HotButtonWnd2")
        );
    }

    const INI_HOTBUTTON: &str = include_str!("../template/dorskui/EQUI_HotButtonWnd.xml");

    #[test]
    fn patching_moves_and_resizes_only_the_named_window() {
        let mut layout = default_layout();
        layout
            .0
            .insert("PlayerWindow".into(), (960, 1080, 500, 240));
        let files = generate(&layout);
        let map = bundle_map(&files);

        let xml = map["uifiles/dorskui/EQUI_PlayerWindow.xml"];
        let (w, h) = screen_size(xml, "PlayerWindow");
        assert_eq!((w, h), (500, 240));
        assert_eq!(
            xml.replace("<CX>500</CX>", "<CX>660</CX>")
                .replace("<CY>240</CY>", "<CY>320</CY>"),
            INI_XML_PW
        );

        let ini = map[INI_NAME];
        let section = ini_section(ini, "PlayerWindow");
        assert!(section.contains("XPos=25.000000%"), "{section}");
        assert!(section.contains("YPos=50.000000%"), "{section}");
        assert!(!section.contains("Width="), "PlayerWindow has no ini Width");

        assert_eq!(
            ini_section(ini, "GroupWindow"),
            ini_section(INI, "GroupWindow"),
            "untouched windows must not move"
        );
    }

    const INI_XML_PW: &str = include_str!("../template/dorskui/EQUI_PlayerWindow.xml");

    #[test]
    fn only_geometry_bytes_change_in_a_patched_file() {
        let mut layout = default_layout();
        layout.0.insert("CastingWindow".into(), (0, 0, 111, 22));
        let files = generate(&layout);
        let map = bundle_map(&files);
        let patched = map["uifiles/dorskui/EQUI_CastingWindow.xml"];

        assert_eq!(
            patched
                .replace("<CX>111</CX>", "<CX>700</CX>")
                .replace("<CY>22</CY>", "<CY>70</CY>"),
            INI_CASTING,
            "more than the <Size> of the CastingWindow screen changed"
        );
    }

    const INI_CASTING: &str = include_str!("../template/dorskui/EQUI_CastingWindow.xml");

    #[test]
    fn channel_map_routing_is_copied_verbatim() {
        let mut layout = default_layout();
        layout.0.insert("MainChat".into(), (0, 0, 100, 100));
        layout.0.insert("Chat 1".into(), (200, 200, 100, 100));
        let files = generate(&layout);
        let ini = bundle_map(&files)[INI_NAME].to_string();

        fn channel_lines(text: &str) -> Vec<&str> {
            text.split_inclusive('\n')
                .filter(|line| line.starts_with("ChannelMap"))
                .collect()
        }
        let template = channel_lines(INI);
        assert_eq!(template.len(), 108);
        assert_eq!(channel_lines(&ini), template);

        let main = ini_section(&ini, "MainChat");
        assert!(main.contains("Width=100"), "{main}");
        assert!(main.contains("Height=100"), "{main}");
        assert!(main.contains("XPos=0.000000%"), "{main}");
    }

    #[test]
    fn chat_windows_have_no_xml_of_their_own() {
        let mut layout = Layout(BTreeMap::new());
        layout.0.insert("MainChat".into(), (10, 20, 30, 40));
        let files = generate(&layout);
        let map = bundle_map(&files);
        for (name, source) in XML_FILES {
            assert_eq!(map[format!("uifiles/dorskui/{name}").as_str()], *source);
        }
        assert!(ini_section(map[INI_NAME], "MainChat").contains("Width=30"));
    }

    #[test]
    fn unknown_windows_are_rejected() {
        let mut layout = default_layout();
        layout.0.insert("BankWindow".into(), (0, 0, 10, 10));
        let error = generate_bundle(&layout, "dorskui", 3840, 2160).unwrap_err();
        assert!(matches!(&error, SkinError::UnknownWindow(name) if name == "BankWindow"));
        assert!(error.to_string().contains("BankWindow"));
    }

    #[test]
    fn skin_names_are_sanitized_and_must_not_be_empty() {
        assert_eq!(sanitize_skin_name("Dorsk UI/../v4"), "dorsk_ui____v4");
        assert_eq!(sanitize_skin_name("My Skin!"), "my_skin");
        assert_eq!(sanitize_skin_name("DorskUI"), "dorskui");
        assert!(matches!(
            generate_bundle(&default_layout(), "///", 3840, 2160).unwrap_err(),
            SkinError::EmptySkinName
        ));
        assert!(matches!(
            generate_bundle(&default_layout(), "x", 0, 2160).unwrap_err(),
            SkinError::BadScreen(0, 2160)
        ));
        let files = generate_bundle(&default_layout(), "My Skin!", 3840, 2160).unwrap();
        assert!(files[0].0.starts_with("uifiles/my_skin/"));
    }

    #[test]
    fn the_zip_reopens_with_every_expected_path() {
        let files = generate(&default_layout());
        let bytes = zip_bundle(&files).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), XML_FILES.len() + 1);

        let names: Vec<String> = archive.file_names().map(str::to_string).collect();
        assert!(names.contains(&INI_NAME.to_string()));
        assert!(names.contains(&"uifiles/dorskui/EQUI_PlayerWindow.xml".to_string()));

        let mut entry = archive
            .by_name("uifiles/dorskui/EQUI_PlayerWindow.xml")
            .unwrap();
        let mut text = String::new();
        entry.read_to_string(&mut text).unwrap();
        assert_eq!(text, INI_XML_PW);
    }

    #[test]
    fn a_resolution_change_rescales_positions() {
        let mut layout = Layout(BTreeMap::new());
        layout.0.insert("PlayerWindow".into(), (960, 540, 660, 320));
        let files = generate_bundle(&layout, "s", 1920, 1080).unwrap();
        let section = ini_section(bundle_map(&files)[INI_NAME], "PlayerWindow");
        assert!(section.contains("XPos=50.000000%"), "{section}");
        assert!(section.contains("YPos=50.000000%"), "{section}");
    }

    fn without_screen(xml: &str, item: &str) -> String {
        let open = format!("<Screen item=\"{item}\">");
        let start = xml.find(&open).unwrap();
        let end = start + xml[start..].find("</Screen>").unwrap();
        format!("{}{}", &xml[..start], &xml[end..])
    }

    fn screen_size(xml: &str, item: &str) -> (i32, i32) {
        let open = format!("<Screen item=\"{item}\">");
        let start = xml.find(&open).unwrap() + open.len();
        let block = &xml[start..start + xml[start..].find("</Screen>").unwrap()];
        let size_start = block.find("<Size>").unwrap();
        let size = &block[size_start..];
        (tag_value(size, "CX"), tag_value(size, "CY"))
    }

    fn tag_value(block: &str, tag: &str) -> i32 {
        let open = format!("<{tag}>");
        let start = block.find(&open).unwrap() + open.len();
        block[start..start + block[start..].find(&format!("</{tag}>")).unwrap()]
            .parse()
            .unwrap()
    }

    fn ini_section<'a>(ini: &'a str, name: &str) -> &'a str {
        let head = format!("[{name}]\r\n");
        let start = ini.find(&head).unwrap() + head.len();
        let rest = &ini[start..];
        &rest[..rest.find("\r\n[").map_or(rest.len(), |end| end + 2)]
    }
}
