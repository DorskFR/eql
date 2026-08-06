use crate::config::Config;
use std::path::{Path, PathBuf};

pub const NAME: &str = "EQLD";
pub const COLOR: &str = "18";

/// `/log on` must stay first: with logging off nothing else here is observable.
pub const LINES: [&str; 5] = [
    "/log on",
    "/who",
    "/outputfile inventory",
    "/outputfile spellbook",
    "/outputfile missingspells",
];

const SECTION: &str = "Socials";
const PAGES: u32 = 10;
const BUTTONS: u32 = 12;
const BARS: u32 = 10;
const HOTBAR_SECTION: &str = "HotButtons";

/// `HotButtonData` is serialised `%c%d,%c%d,%s,%d,%s,%s`: type and slot, icon
/// type and slot, item guid, item id, label, item name. The type char is
/// `'A' + type`, so a social (type 4) is `E`, and `@-1` is icon type -1.
const HOTBUTTON_TAIL: &str = "@-1,0000000000000000,0";

const CHARACTERS_INI: &str = "_characters.ini";
const INI_SUFFIX: &str = "_LO1.ini";
const UI_PREFIX: &str = "UI_";
const BACKUP_SUFFIX: &str = ".eqld.bak";
const TEMP_SUFFIX: &str = ".eqld.tmp";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    page: u32,
    button: u32,
}

impl Slot {
    fn key(&self, suffix: &str) -> String {
        format!("Page{}Button{}{}", self.page, self.button, suffix)
    }
}

fn split_lines(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            lines.push(bytes[start..=index].to_vec());
            start = index + 1;
        }
    }
    if start < bytes.len() {
        lines.push(bytes[start..].to_vec());
    }
    lines
}

fn line_ending(lines: &[Vec<u8>]) -> &'static [u8] {
    let bare = lines
        .iter()
        .any(|line| line.ends_with(b"\n") && !line.ends_with(b"\r\n"));
    let carriage = lines.iter().any(|line| line.ends_with(b"\r\n"));
    if bare && !carriage {
        b"\n"
    } else {
        b"\r\n"
    }
}

fn text(line: &[u8]) -> String {
    String::from_utf8_lossy(line)
        .trim_end_matches(['\r', '\n'])
        .to_string()
}

fn header_of(line: &[u8]) -> Option<String> {
    let trimmed = text(line).trim().to_string();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    Some(inner.to_string())
}

fn strip_prefix_ci<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

fn take_number(value: &str) -> Option<(u32, &str)> {
    let end = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    Some((value[..end].parse().ok()?, &value[end..]))
}

fn parse_slot(key: &str) -> Option<(Slot, &str)> {
    let rest = strip_prefix_ci(key.trim(), "Page")?;
    let (page, rest) = take_number(rest)?;
    let rest = strip_prefix_ci(rest, "Button")?;
    let (button, suffix) = take_number(rest)?;
    Some((Slot { page, button }, suffix))
}

fn slot_of(key: &str) -> Option<(Slot, &str)> {
    let (slot, suffix) = parse_slot(key)?;
    (!suffix.is_empty()).then_some((slot, suffix))
}

/// A hotbutton key carries no suffix: `Page1Button1=E12,…`.
fn hotbutton_slot_of(key: &str) -> Option<Slot> {
    let (slot, suffix) = parse_slot(key)?;
    suffix.is_empty().then_some(slot)
}

fn entry_of(line: &[u8]) -> Option<(Slot, String, String)> {
    let raw = text(line);
    let (key, value) = raw.split_once('=')?;
    let (slot, suffix) = slot_of(key)?;
    Some((slot, suffix.to_string(), value.trim().to_string()))
}

fn render(slot: Slot, suffix: &str, value: &str, ending: &[u8]) -> Vec<u8> {
    let mut line = format!("{}={value}", slot.key(suffix)).into_bytes();
    line.extend_from_slice(ending);
    line
}

fn wanted(color: &str) -> Vec<(String, String)> {
    let mut wanted = vec![
        ("Name".to_string(), NAME.to_string()),
        ("Color".to_string(), color.to_string()),
    ];
    for (index, line) in LINES.iter().enumerate() {
        wanted.push((format!("Line{}", index + 1), (*line).to_string()));
    }
    wanted
}

fn free_slot(entries: &[(Slot, String, String)]) -> Option<Slot> {
    (1..=PAGES)
        .flat_map(|page| (1..=BUTTONS).map(move |button| Slot { page, button }))
        .find(|slot| {
            !entries
                .iter()
                .any(|(taken, _, value)| taken == slot && !value.is_empty())
        })
}

/// Which hotbar the button is placed on. Bar 1 is `[HotButtons]` and bars 2-10
/// are `[HotButtons2]`…`[HotButtons10]`; bar `0` leaves every bar alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub bar: u32,
    pub page: u32,
}

impl Default for Placement {
    fn default() -> Self {
        Self { bar: 1, page: 1 }
    }
}

impl Placement {
    pub fn wanted(&self) -> bool {
        self.bar >= 1 && self.bar <= BARS && self.page >= 1 && self.page <= PAGES
    }

    fn section(&self) -> String {
        match self.bar {
            1 => HOTBAR_SECTION.to_string(),
            bar => format!("{HOTBAR_SECTION}{bar}"),
        }
    }
}

/// A social's slot is its zero-based index over the 12-per-page grid. From 120
/// up the same field means an alternate advancement group instead, which the
/// 10x12 grid cannot reach.
fn social_index(slot: Slot) -> u32 {
    (slot.page - 1) * BUTTONS + (slot.button - 1)
}

fn is_hotbar(name: &str) -> bool {
    let Some(rest) = strip_prefix_ci(name.trim(), HOTBAR_SECTION) else {
        return false;
    };
    rest.is_empty() || rest.parse::<u32>().is_ok()
}

/// The social index of a hotbutton that already carries our label.
fn ours(value: &str) -> Option<u32> {
    let fields: Vec<&str> = value.split(',').collect();
    if fields.get(4) != Some(&NAME) {
        return None;
    }
    strip_prefix_ci(fields.first()?.trim(), "E")?.parse().ok()
}

fn hotbutton_value(slot: Slot) -> String {
    format!("E{},{HOTBUTTON_TAIL},{NAME},", social_index(slot))
}

/// An existing EQLD button is corrected wherever it already sits, on any bar;
/// otherwise an empty button is claimed. A button holding anything else is
/// never taken.
fn place(
    lines: Vec<Vec<u8>>,
    slot: Slot,
    placement: Placement,
    ending: &[u8],
) -> Result<Vec<Vec<u8>>, SocialsError> {
    let wanted = hotbutton_value(slot);
    let mut lines = lines;

    let mut inside = false;
    let mut existing = None;
    for (index, line) in lines.iter().enumerate() {
        if let Some(name) = header_of(line) {
            inside = is_hotbar(&name);
            continue;
        }
        if !inside {
            continue;
        }
        let raw = text(line);
        let Some((key, value)) = raw.split_once('=') else {
            continue;
        };
        let Some(taken) = hotbutton_slot_of(key) else {
            continue;
        };
        if ours(value.trim()).is_some() {
            existing = Some((index, taken));
            break;
        }
    }

    if let Some((index, taken)) = existing {
        let mut line = format!("Page{}Button{}={wanted}", taken.page, taken.button).into_bytes();
        line.extend_from_slice(ending);
        lines[index] = line;
        return Ok(lines);
    }

    let name = placement.section();
    let header = lines
        .iter()
        .position(|line| header_of(line).is_some_and(|found| found.eq_ignore_ascii_case(&name)));
    let (header, end) = match header {
        Some(header) => {
            let end = lines[header + 1..]
                .iter()
                .position(|line| header_of(line).is_some())
                .map_or(lines.len(), |offset| header + 1 + offset);
            (header, end)
        }
        None => {
            let mut section_header = format!("[{name}]").into_bytes();
            section_header.extend_from_slice(ending);
            lines.push(section_header);
            (lines.len() - 1, lines.len())
        }
    };

    let taken: Vec<Slot> = lines[header + 1..end]
        .iter()
        .filter_map(|line| {
            let raw = text(line);
            let (key, value) = raw.split_once('=')?;
            let slot = hotbutton_slot_of(key)?;
            (!value.trim().is_empty()).then_some(slot)
        })
        .collect();
    let free = (1..=BUTTONS)
        .map(|button| Slot {
            page: placement.page,
            button,
        })
        .find(|slot| !taken.contains(slot))
        .ok_or(SocialsError::NoFreeHotbutton {
            bar: placement.bar,
            page: placement.page,
        })?;

    let mut line = format!("Page{}Button{}={wanted}", free.page, free.button).into_bytes();
    line.extend_from_slice(ending);
    lines.insert(end, line);
    Ok(lines)
}

pub fn apply(bytes: &[u8], placement: Placement) -> Result<Vec<u8>, SocialsError> {
    let lines = split_lines(bytes);
    let ending = line_ending(&lines).to_vec();

    let header = lines
        .iter()
        .position(|line| header_of(line).is_some_and(|name| name.eq_ignore_ascii_case(SECTION)));
    let (head, section, tail) = match header {
        Some(header) => {
            let end = lines[header + 1..]
                .iter()
                .position(|line| header_of(line).is_some())
                .map_or(lines.len(), |offset| header + 1 + offset);
            (
                lines[..=header].to_vec(),
                lines[header + 1..end].to_vec(),
                lines[end..].to_vec(),
            )
        }
        None => {
            let mut head = lines;
            let mut section_header = format!("[{SECTION}]").into_bytes();
            section_header.extend_from_slice(&ending);
            head.push(section_header);
            (head, Vec::new(), Vec::new())
        }
    };

    let entries: Vec<(Slot, String, String)> = section.iter().filter_map(|l| entry_of(l)).collect();
    let slot = entries
        .iter()
        .find(|(_, suffix, value)| suffix.eq_ignore_ascii_case("Name") && value == NAME)
        .map(|(slot, _, _)| *slot)
        .or_else(|| free_slot(&entries))
        .ok_or(SocialsError::NoFreeSlot)?;
    let color = entries
        .iter()
        .find(|(taken, suffix, value)| {
            *taken == slot && suffix.eq_ignore_ascii_case("Color") && !value.is_empty()
        })
        .map_or_else(|| COLOR.to_string(), |(_, _, value)| value.clone());

    let wanted = wanted(&color);
    let mut done = vec![false; wanted.len()];
    let mut kept: Vec<Vec<u8>> = Vec::with_capacity(section.len() + wanted.len());
    let mut after_slot = None;
    for line in &section {
        let owned = entry_of(line).filter(|(taken, _, _)| *taken == slot);
        let Some((_, suffix, _)) = owned else {
            kept.push(line.clone());
            continue;
        };
        match wanted
            .iter()
            .position(|(key, _)| key.eq_ignore_ascii_case(&suffix))
        {
            Some(index) if done[index] => continue,
            Some(index) => {
                done[index] = true;
                kept.push(render(slot, &wanted[index].0, &wanted[index].1, &ending));
            }
            None => kept.push(line.clone()),
        }
        after_slot = Some(kept.len());
    }

    let missing: Vec<Vec<u8>> = wanted
        .iter()
        .enumerate()
        .filter(|(index, _)| !done[*index])
        .map(|(_, (suffix, value))| render(slot, suffix, value, &ending))
        .collect();
    let at = after_slot.unwrap_or(kept.len());
    kept.splice(at..at, missing);

    let mut out: Vec<Vec<u8>> = head;
    out.extend(kept);
    out.extend(tail);
    if placement.wanted() {
        out = place(out, slot, placement, &ending)?;
    }
    let last = out.len().saturating_sub(1);
    for line in out.iter_mut().take(last) {
        if !line.ends_with(b"\n") {
            line.extend_from_slice(&ending);
        }
    }
    Ok(out.concat())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Written,
    Unchanged,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub character: String,
    pub server: String,
    pub path: PathBuf,
    pub outcome: Outcome,
}

pub fn ini_name(character: &str, server: &str) -> String {
    format!("{character}_{server}{INI_SUFFIX}")
}

fn from_ini_name(file_name: &str) -> Option<(String, String)> {
    if file_name.starts_with(UI_PREFIX) {
        return None;
    }
    let stem = file_name.strip_suffix(INI_SUFFIX)?;
    let (character, server) = stem.rsplit_once('_')?;
    (!character.is_empty() && !server.is_empty())
        .then(|| (character.to_string(), server.to_string()))
}

pub fn characters(root: &Path) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    let mut push = |character: String, server: String| {
        if !found.iter().any(|(name, on)| {
            name.eq_ignore_ascii_case(&character) && on.eq_ignore_ascii_case(&server)
        }) {
            found.push((character, server));
        }
    };

    if let Ok(bytes) = std::fs::read(root.join(CHARACTERS_INI)) {
        let mut inside = false;
        for line in split_lines(&bytes) {
            if let Some(header) = header_of(&line) {
                inside = header.eq_ignore_ascii_case("Characters");
                continue;
            }
            if !inside {
                continue;
            }
            let raw = text(&line);
            let Some((_, value)) = raw.split_once('=') else {
                continue;
            };
            let Some((character, server)) = value.trim().split_once(',') else {
                continue;
            };
            let (character, server) = (character.trim(), server.trim());
            if !character.is_empty() && !server.is_empty() {
                push(character.to_string(), server.to_string());
            }
        }
    }

    let mut loose: Vec<(String, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            if let Some(pair) = entry.file_name().to_str().and_then(from_ini_name) {
                loose.push(pair);
            }
        }
    }
    loose.sort();
    for (character, server) in loose {
        push(character, server);
    }
    found
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

pub fn install_file(path: &Path, placement: Placement) -> Result<Outcome, SocialsError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Outcome::Missing),
        Err(err) => return Err(SocialsError::Io(path.to_path_buf(), err)),
    };
    let next = apply(&bytes, placement)?;
    if next == bytes {
        return Ok(Outcome::Unchanged);
    }
    let backup = sibling(path, BACKUP_SUFFIX);
    std::fs::write(&backup, &bytes).map_err(|err| SocialsError::Io(backup, err))?;
    let temp = sibling(path, TEMP_SUFFIX);
    std::fs::write(&temp, &next).map_err(|err| SocialsError::Io(temp.clone(), err))?;
    std::fs::rename(&temp, path).map_err(|err| SocialsError::Io(path.to_path_buf(), err))?;
    Ok(Outcome::Written)
}

pub fn install(root: &Path, placement: Placement) -> Vec<Result<Report, SocialsError>> {
    characters(root)
        .into_iter()
        .map(|(character, server)| {
            let path = root.join(ini_name(&character, &server));
            install_file(&path, placement).map(|outcome| Report {
                character,
                server,
                path,
                outcome,
            })
        })
        .collect()
}

pub fn game_is_running(config: &Config) -> Option<bool> {
    config
        .game_process()
        .map(|name| crate::overlays::ProcessWatch::new().is_running(name))
}

pub fn run(config: &Config, args: &[String]) -> Result<(), SocialsError> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("usage: eqld [config.toml] install-social");
        return Ok(());
    }
    match game_is_running(config) {
        Some(true) => return Err(SocialsError::GameRunning),
        Some(false) => {}
        None => eprintln!(
            "[game] process is empty, so whether the client is running cannot be checked; \
             make sure the game is closed, it rewrites this file when it exits"
        ),
    }

    let root = &config.game.root;
    let placement = config.socials.placement();
    let reports = install(root, placement);
    if reports.is_empty() {
        println!("no characters found under {}", root.display());
        return Ok(());
    }
    let mut failure = None;
    for report in reports {
        match report {
            Ok(report) => println!(
                "{}: {} ({})",
                report.path.display(),
                match report.outcome {
                    Outcome::Written => "social installed",
                    Outcome::Unchanged => "social already installed",
                    Outcome::Missing => "no character ini yet; log in once",
                },
                report.character
            ),
            Err(err) => {
                eprintln!("{err}");
                failure = Some(err);
            }
        }
    }
    match failure {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SocialsError {
    #[error("the game is running; close it first, the client rewrites this file when it exits")]
    GameRunning,
    #[error("every social slot is taken by something else")]
    NoFreeSlot,
    #[error(
        "every button of hotbar {bar} page {page} is taken by something else; \
         point [socials] bar/page at a free one, or set bar = 0 to leave the bars alone"
    )]
    NoFreeHotbutton { bar: u32, page: u32 },
    #[error("io on {0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &[u8] = include_bytes!("../../../fixtures/ini/Dorsk_erudin_LO1.ini");
    const BAR1: Placement = Placement { bar: 1, page: 1 };
    const NO_BAR: Placement = Placement { bar: 0, page: 0 };

    fn section(bytes: &[u8], name: &str) -> Vec<String> {
        sections(bytes)
            .into_iter()
            .find(|(found, _)| found.eq_ignore_ascii_case(name))
            .map(|(_, body)| body)
            .unwrap_or_default()
    }

    fn sections(bytes: &[u8]) -> Vec<(String, Vec<String>)> {
        let mut sections: Vec<(String, Vec<String>)> = Vec::new();
        for line in split_lines(bytes) {
            match header_of(&line) {
                Some(name) => sections.push((name, Vec::new())),
                None => {
                    if let Some((_, body)) = sections.last_mut() {
                        body.push(text(&line));
                    }
                }
            }
        }
        sections
    }

    fn socials(bytes: &[u8]) -> Vec<String> {
        sections(bytes)
            .into_iter()
            .find(|(name, _)| name == SECTION)
            .expect("a socials section")
            .1
    }

    #[test]
    fn the_real_character_ini_keeps_every_other_section_byte_identical() {
        let before = sections(REAL);
        let after = sections(&apply(REAL, BAR1).unwrap());
        assert_eq!(
            before.iter().map(|(name, _)| name).collect::<Vec<_>>(),
            after.iter().map(|(name, _)| name).collect::<Vec<_>>(),
            "no section is added, removed or reordered"
        );
        for ((name, body), (_, next)) in before.iter().zip(&after) {
            if name == SECTION {
                continue;
            }
            assert_eq!(body, next, "[{name}] was touched");
        }
    }

    #[test]
    fn the_existing_eqld_social_is_updated_in_place() {
        let out = apply(REAL, BAR1).unwrap();
        assert_eq!(
            socials(&out),
            vec![
                "Page2Button1Name=EQLD",
                "Page2Button1Color=18",
                "Page2Button1Line1=/log on",
                "Page2Button1Line2=/who",
                "Page2Button1Line3=/outputfile inventory",
                "Page2Button1Line4=/outputfile spellbook",
                "Page2Button1Line5=/outputfile missingspells",
            ],
            "the slot the user already bound a hotbutton to is reused"
        );
        assert!(String::from_utf8(out.clone()).unwrap().ends_with("\r\n"));
        assert_eq!(
            apply(&out, BAR1).unwrap(),
            out,
            "applying twice changes nothing"
        );
    }

    #[test]
    fn the_hotbutton_the_user_already_has_is_recognised_and_left_where_it_is() {
        let out = apply(REAL, BAR1).unwrap();
        assert_eq!(
            section(&out, "HotButtons4"),
            section(REAL, "HotButtons4"),
            "the bar the user put it on is reproduced byte for byte"
        );
        assert!(
            section(&out, "HotButtons4")
                .contains(&"Page1Button1=E12,@-1,0000000000000000,0,EQLD,".to_string()),
            "{:?}",
            section(&out, "HotButtons4")
        );
        assert_eq!(
            section(&out, HOTBAR_SECTION),
            section(REAL, HOTBAR_SECTION),
            "no second button is added to the configured bar 1"
        );
    }

    #[test]
    fn a_social_that_moved_slot_drags_its_hotbutton_with_it() {
        let ini = b"[Socials]\r\nPage1Button1Name=EQLD\r\n\
                    [HotButtons3]\r\nPage1Button7=E99,@-1,0000000000000000,0,EQLD,\r\n";
        let out = String::from_utf8(apply(ini, BAR1).unwrap()).unwrap();
        assert!(
            out.contains("Page1Button7=E0,@-1,0000000000000000,0,EQLD,"),
            "the stale index is corrected where the button already sits: {out:?}"
        );
        assert!(
            !out.contains("[HotButtons]"),
            "and it is not duplicated onto bar 1: {out:?}"
        );
    }

    #[test]
    fn a_button_holding_something_else_is_never_taken() {
        let ini = b"[Socials]\r\nPage1Button1Name=EQLD\r\n\
                    [HotButtons]\r\nPage1Button1=B0,@-1,0000000000000000,0,Melee,\r\n\
                    Page1Button2=,\r\n";
        let out = String::from_utf8(apply(ini, BAR1).unwrap()).unwrap();
        assert!(
            out.contains("Page1Button1=B0,@-1,0000000000000000,0,Melee,"),
            "{out:?}"
        );
        assert!(
            out.contains("Page1Button3=E0,@-1,0000000000000000,0,EQLD,"),
            "button 2 holds a bare comma, which is not empty: {out:?}"
        );
    }

    #[test]
    fn a_full_hotbar_page_is_an_error_not_a_stolen_button() {
        let mut ini = String::from("[Socials]\r\nPage1Button1Name=EQLD\r\n[HotButtons2]\r\n");
        for button in 1..=BUTTONS {
            ini.push_str(&format!(
                "Page1Button{button}=J30,@-1,0000000000000000,0,Kick,\r\n"
            ));
        }
        let placement = Placement { bar: 2, page: 1 };
        assert!(matches!(
            apply(ini.as_bytes(), placement),
            Err(SocialsError::NoFreeHotbutton { bar: 2, page: 1 })
        ));

        assert!(
            apply(ini.as_bytes(), Placement { bar: 2, page: 2 }).is_ok(),
            "another page of the same bar is still free"
        );
    }

    #[test]
    fn a_social_never_indexes_into_the_alternate_advancement_range() {
        let highest = Slot {
            page: PAGES,
            button: BUTTONS,
        };
        assert_eq!(social_index(Slot { page: 1, button: 1 }), 0);
        assert_eq!(social_index(Slot { page: 2, button: 1 }), 12);
        assert_eq!(social_index(highest), 119, "120 and up mean an AA group");
    }

    #[test]
    fn every_bar_maps_to_the_section_the_client_writes() {
        assert_eq!(Placement { bar: 1, page: 1 }.section(), "HotButtons");
        assert_eq!(Placement { bar: 4, page: 1 }.section(), "HotButtons4");
        assert_eq!(Placement { bar: 10, page: 1 }.section(), "HotButtons10");
        assert!(Placement::default().wanted());
        assert!(!NO_BAR.wanted(), "bar 0 is the opt out");
        assert!(
            !Placement { bar: 11, page: 1 }.wanted(),
            "there are ten bars"
        );
        assert!(!Placement { bar: 1, page: 11 }.wanted());

        assert!(is_hotbar("HotButtons"));
        assert!(is_hotbar("HotButtons4"));
        assert!(!is_hotbar("HotButtonsWnd"));
        assert!(!is_hotbar("Socials"));
    }

    #[test]
    fn only_a_button_carrying_our_label_is_claimed_as_ours() {
        assert_eq!(ours("E12,@-1,0000000000000000,0,EQLD,"), Some(12));
        assert_eq!(
            ours("E451,@-1,0000000000000000,0,,"),
            None,
            "an unnamed alternate advancement button is not ours"
        );
        assert_eq!(ours("E6120,@-1,0000000000000000,0,Bazaar Portal,"), None);
        assert_eq!(
            ours("J30,@-1,0000000000000000,0,EQLD,"),
            None,
            "a skill the user happened to label EQLD is not a social"
        );
    }

    #[test]
    fn a_social_the_user_named_is_never_overwritten() {
        let ini = b"[Socials]\r\nPage1Button1Name=Sit\r\nPage1Button1Line1=/sit\r\n".to_vec();
        let out = apply(&ini, BAR1).unwrap();
        assert_eq!(
            socials(&out),
            vec![
                "Page1Button1Name=Sit",
                "Page1Button1Line1=/sit",
                "Page1Button2Name=EQLD",
                "Page1Button2Color=18",
                "Page1Button2Line1=/log on",
                "Page1Button2Line2=/who",
                "Page1Button2Line3=/outputfile inventory",
                "Page1Button2Line4=/outputfile spellbook",
                "Page1Button2Line5=/outputfile missingspells",
            ]
        );
    }

    #[test]
    fn a_missing_socials_section_is_appended_with_the_first_slot() {
        let ini = b"[Defaults]\r\nMusic=10\r\n".to_vec();
        let out = apply(&ini, BAR1).unwrap();
        assert_eq!(
            String::from_utf8(out.clone()).unwrap(),
            "[Defaults]\r\nMusic=10\r\n[Socials]\r\nPage1Button1Name=EQLD\r\n\
             Page1Button1Color=18\r\nPage1Button1Line1=/log on\r\nPage1Button1Line2=/who\r\n\
             Page1Button1Line3=/outputfile inventory\r\nPage1Button1Line4=/outputfile spellbook\r\n\
             Page1Button1Line5=/outputfile missingspells\r\n\
             [HotButtons]\r\nPage1Button1=E0,@-1,0000000000000000,0,EQLD,\r\n",
            "the first social slot is index zero, and the bar is created for it"
        );
        assert_eq!(apply(&out, BAR1).unwrap(), out);
    }

    #[test]
    fn a_file_that_does_not_end_in_a_newline_is_terminated_before_appending() {
        let out = apply(b"[Defaults]\r\nMusic=10", BAR1).unwrap();
        assert!(String::from_utf8(out)
            .unwrap()
            .contains("Music=10\r\n[Socials]"));
    }

    #[test]
    fn bare_newlines_are_kept_bare() {
        let out = apply(b"[Socials]\nPage1Button1Name=EQLD\n", BAR1).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(!out.contains('\r'), "{out:?}");
        assert!(
            out.ends_with("[HotButtons]\nPage1Button1=E0,@-1,0000000000000000,0,EQLD,\n"),
            "{out:?}"
        );
    }

    #[test]
    fn the_social_stays_where_it_is_even_with_later_sections_behind_it() {
        let ini = b"[Socials]\r\nPage3Button4Name=EQLD\r\n[Friends]\r\nSendToUChat=0\r\n";
        let out = String::from_utf8(apply(ini, BAR1).unwrap()).unwrap();
        assert!(
            out.contains(
                "Page3Button4Line5=/outputfile missingspells\r\n[Friends]\r\nSendToUChat=0\r\n"
            ),
            "{out:?}"
        );
        assert!(
            out.ends_with("[HotButtons]\r\nPage1Button1=E27,@-1,0000000000000000,0,EQLD,\r\n"),
            "page 3 button 4 is index (3-1)*12+3 = 27: {out:?}"
        );

        let untouched = String::from_utf8(apply(ini, NO_BAR).unwrap()).unwrap();
        assert!(
            !untouched.contains("HotButtons"),
            "bar 0 leaves every hotbar alone: {untouched:?}"
        );
    }

    #[test]
    fn a_hand_edited_line_is_corrected_and_duplicates_collapse() {
        let ini =
            b"[Socials]\r\nPage1Button1Name=EQLD\r\nPage1Button1Line1=/outputfile inventory\r\n\
                    Page1Button1Line1=/rude\r\nPage1Button1Color=7\r\n";
        assert_eq!(
            socials(&apply(ini, BAR1).unwrap()),
            vec![
                "Page1Button1Name=EQLD",
                "Page1Button1Line1=/log on",
                "Page1Button1Color=7",
                "Page1Button1Line2=/who",
                "Page1Button1Line3=/outputfile inventory",
                "Page1Button1Line4=/outputfile spellbook",
                "Page1Button1Line5=/outputfile missingspells",
            ],
            "the colour the user picked survives"
        );
    }

    #[test]
    fn a_full_page_of_socials_is_an_error_not_a_stolen_slot() {
        let mut ini = String::from("[Socials]\r\n");
        for page in 1..=PAGES {
            for button in 1..=BUTTONS {
                ini.push_str(&format!("Page{page}Button{button}Name=Taken\r\n"));
            }
        }
        assert!(matches!(
            apply(ini.as_bytes(), BAR1),
            Err(SocialsError::NoFreeSlot)
        ));
    }

    #[test]
    fn keys_are_read_the_way_the_client_writes_them_and_no_looser() {
        assert_eq!(
            slot_of("Page2Button11Line3"),
            Some((
                Slot {
                    page: 2,
                    button: 11
                },
                "Line3"
            ))
        );
        assert_eq!(slot_of("Page2Button1"), None);
        assert_eq!(slot_of("PageButton1Name"), None);
        assert_eq!(slot_of("SpellLoadout1.name"), None);

        assert_eq!(
            hotbutton_slot_of("Page1Button12"),
            Some(Slot {
                page: 1,
                button: 12
            }),
            "a hotbutton key ends on the number, with no suffix to find"
        );
        assert_eq!(hotbutton_slot_of("Page1Button1Name"), None);
        assert_eq!(hotbutton_slot_of("Page1Button"), None);
    }

    #[test]
    fn writing_backs_the_file_up_and_leaves_no_temporary_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(ini_name("Dorsk", "erudin"));
        std::fs::write(&path, REAL).unwrap();

        assert_eq!(install_file(&path, BAR1).unwrap(), Outcome::Written);
        assert_eq!(std::fs::read(sibling(&path, BACKUP_SUFFIX)).unwrap(), REAL);
        assert!(!sibling(&path, TEMP_SUFFIX).exists());
        assert_eq!(std::fs::read(&path).unwrap(), apply(REAL, BAR1).unwrap());

        assert_eq!(
            install_file(&path, BAR1).unwrap(),
            Outcome::Unchanged,
            "a second run neither writes nor re-backs-up"
        );
        assert_eq!(
            install_file(&dir.path().join("Nobody_erudin_LO1.ini"), BAR1).unwrap(),
            Outcome::Missing
        );
    }

    #[test]
    fn the_roster_and_the_files_beside_it_are_both_read() {
        let dir = tempfile::tempdir().unwrap();
        assert!(characters(dir.path()).is_empty());

        std::fs::write(
            dir.path().join(CHARACTERS_INI),
            "[Characters]\r\nCharacter0=Dorsk,erudin\r\nCharacter1= Vala , erudin \r\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("Morveus_erudin_LO1.ini"), "[Socials]\r\n").unwrap();
        std::fs::write(dir.path().join("UI_Dorsk_erudin_LO1.ini"), "[Main]\r\n").unwrap();
        std::fs::write(dir.path().join("eqclient.ini"), "[Defaults]\r\n").unwrap();

        assert_eq!(
            characters(dir.path()),
            vec![
                ("Dorsk".to_string(), "erudin".to_string()),
                ("Vala".to_string(), "erudin".to_string()),
                ("Morveus".to_string(), "erudin".to_string()),
            ],
            "the roster leads, the ui ini is not a character"
        );
    }

    #[test]
    fn installing_reports_a_character_who_has_never_logged_in() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CHARACTERS_INI),
            "[Characters]\r\nCharacter0=Dorsk,erudin\r\nCharacter1=Vala,erudin\r\n",
        )
        .unwrap();
        std::fs::write(dir.path().join(ini_name("Dorsk", "erudin")), REAL).unwrap();

        let reports: Vec<Report> = install(dir.path(), BAR1)
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            reports
                .iter()
                .map(|report| (report.character.as_str(), report.outcome))
                .collect::<Vec<_>>(),
            vec![("Dorsk", Outcome::Written), ("Vala", Outcome::Missing)]
        );
    }
}
