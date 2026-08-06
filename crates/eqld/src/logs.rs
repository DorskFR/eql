use eql_core::api::{LogEvent, LogEventKind};
use std::path::{Path, PathBuf};

const LOG_DIR: &str = "Logs";
const LOG_PREFIX: &str = "eqlog_";
const LOG_SUFFIX: &str = ".txt";

pub fn log_dir(root: &Path) -> PathBuf {
    root.join(LOG_DIR)
}

pub fn is_log_file(file_name: &str) -> bool {
    parse_filename(file_name).is_some()
}

/// `eqlog_<Character>_<server>.txt`; the last `_` splits the server off, so
/// characters keep any underscores the game allows.
pub fn parse_filename(file_name: &str) -> Option<(String, String)> {
    let stem = file_name
        .strip_prefix(LOG_PREFIX)?
        .strip_suffix(LOG_SUFFIX)?;
    let (character, server) = stem.rsplit_once('_')?;
    if character.is_empty() || server.is_empty() {
        return None;
    }
    Some((character.into(), server.into()))
}

pub fn scan(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let dir = log_dir(root);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if entry.file_name().to_str().is_some_and(is_log_file) {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

pub fn latest(root: &Path) -> Option<PathBuf> {
    scan(root)
        .ok()?
        .into_iter()
        .filter_map(|path| {
            let mtime = std::fs::metadata(&path).ok()?.modified().ok()?;
            Some((mtime, path))
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)))
        .map(|(_, path)| path)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The client writes `[Tue Jul 21 21:15:23 2026]` in the machine's local time
/// with no offset, and the log carries nothing to recover that offset from. We
/// read the civil fields as if they were UTC, so stored instants are the wall
/// clock the player saw, not a true instant.
pub fn parse_timestamp(stamp: &str) -> Option<i64> {
    let stamp = stamp.strip_prefix('[')?.strip_suffix(']')?;
    let mut fields = stamp.split_whitespace();
    let _weekday = fields.next()?;
    let month_name = fields.next()?;
    let month = MONTHS.iter().position(|name| *name == month_name)? as i64 + 1;
    let day: i64 = fields.next()?.parse().ok()?;
    let clock = fields.next()?;
    let year: i64 = fields.next()?.parse().ok()?;
    if fields.next().is_some() {
        return None;
    }

    let mut parts = clock.split(':');
    let hour: i64 = parts.next()?.parse().ok()?;
    let minute: i64 = parts.next()?.parse().ok()?;
    let second: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || hour > 23 || minute > 59 || second > 60 || !(1..=31).contains(&day)
    {
        return None;
    }

    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Howard's days-from-civil: days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn split_timestamp(line: &str) -> Option<(i64, &str)> {
    let end = line.find(']')?;
    let at = parse_timestamp(&line[..=end])?;
    Some((at, line[end + 1..].trim_start()))
}

fn between<'a>(text: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let inner = text.strip_prefix(prefix)?.strip_suffix(suffix)?;
    (!inner.is_empty()).then_some(inner)
}

const ANONYMOUS: &str = "ANONYMOUS";
const MAX_CLASSES: usize = 3;

fn class_list(tag: &str) -> Option<Vec<String>> {
    let classes: Vec<String> = tag.split('/').map(str::to_string).collect();
    let sane = |class: &String| {
        (2..=4).contains(&class.len()) && class.bytes().all(|byte| byte.is_ascii_uppercase())
    };
    (classes.len() <= MAX_CLASSES && classes.iter().all(sane)).then_some(classes)
}

/// `/who` lists everyone in the zone; only the row naming the character whose
/// log this is describes us.
fn parse_who(body: &str, character: &str) -> Option<LogEventKind> {
    let end = body.strip_prefix('[')?.find(']')? + 1;
    let tag = &body[1..end];
    let rest = body[end + 1..].trim_start();
    let name = rest.split_whitespace().next()?;
    if !name.eq_ignore_ascii_case(character) {
        return None;
    }
    if tag == ANONYMOUS {
        return None;
    }

    let (level, classes) = tag.split_once(' ')?;
    let level: u32 = level.parse().ok()?;
    let classes = class_list(classes)?;

    let after_name = &rest[name.len()..];
    let described = after_name
        .split_once("ZONE:")
        .map_or(after_name, |(before, _)| before);
    let race = described
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(race, _)| race.trim())
        .filter(|race| !race.is_empty() && !race.contains('('))
        .map(str::to_string);

    Some(LogEventKind::Who {
        level,
        classes,
        race,
    })
}

pub fn parse_body(body: &str, character: &str) -> Option<LogEventKind> {
    if body.starts_with('[') {
        return parse_who(body, character);
    }
    if let Some(level) = between(body, "You have gained a level! Welcome to level ", "!") {
        return level
            .parse()
            .ok()
            .map(|level| LogEventKind::Level { level });
    }
    if let Some(item) = between(body, "--You have looted a ", ".--")
        .or_else(|| between(body, "You have looted a ", "."))
    {
        return Some(LogEventKind::Loot { item: item.into() });
    }
    if let Some(zone) = between(body, "You have entered ", ".") {
        return Some(LogEventKind::Zone { zone: zone.into() });
    }
    if let Some(killer) = between(body, "You have been slain by ", "!") {
        return Some(LogEventKind::Death {
            killer: Some(killer.into()),
        });
    }
    if body == "You died." {
        return Some(LogEventKind::Death { killer: None });
    }
    if let Some(coords) = body.strip_prefix("Your Location is ") {
        let mut parts = coords.split(',').map(|part| part.trim().parse::<f64>());
        if let (Some(Ok(y)), Some(Ok(x)), Some(Ok(z)), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        {
            return Some(LogEventKind::Location { y, x, z });
        }
        return None;
    }
    if let Some(rest) = between(body, "You have become better at ", ")") {
        if let Some((skill, value)) = rest.rsplit_once("! (") {
            if let Ok(value) = value.parse() {
                return Some(LogEventKind::Skill {
                    skill: skill.into(),
                    value,
                });
            }
        }
    }
    None
}

pub fn parse_line(line: &str, character: &str) -> Option<LogEvent> {
    let (at, body) = split_timestamp(line.trim_end_matches(['\r', '\n']))?;
    parse_body(body, character).map(|kind| LogEvent { at, kind })
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Harvest {
    pub events: Vec<LogEvent>,
    pub dropped: usize,
}

/// Only whole lines are harvested; the caller advances its byte offset by
/// `consumed` so a half-written trailing line is re-read next tick.
pub fn harvest(chunk: &[u8], character: &str) -> (Harvest, usize) {
    let Some(last_newline) = chunk.iter().rposition(|byte| *byte == b'\n') else {
        return (Harvest::default(), 0);
    };
    let consumed = last_newline + 1;
    let text = String::from_utf8_lossy(&chunk[..consumed]);

    let mut harvest = Harvest::default();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match parse_line(line, character) {
            Some(event) => harvest.events.push(event),
            None => harvest.dropped += 1,
        }
    }
    (harvest, consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(line: &str) -> LogEventKind {
        parse_line(line, "Dorsk")
            .unwrap_or_else(|| panic!("no event for {line:?}"))
            .kind
    }

    #[test]
    fn filenames_split_on_the_last_underscore() {
        assert_eq!(
            parse_filename("eqlog_Dorsk_erudin.txt"),
            Some(("Dorsk".into(), "erudin".into()))
        );
        assert_eq!(
            parse_filename("eqlog_Two_Names_erudin.txt"),
            Some(("Two_Names".into(), "erudin".into()))
        );
        assert_eq!(parse_filename("eqlog_erudin.txt"), None);
        assert_eq!(parse_filename("eqlog_.txt"), None);
        assert_eq!(parse_filename("Dorsk_erudin-Inventory.txt"), None);
        assert_eq!(parse_filename("eqlog_Dorsk_erudin.txt.bak"), None);
    }

    #[test]
    fn scan_skips_a_missing_logs_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(scan(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn scan_finds_only_log_files() {
        let dir = tempfile::tempdir().unwrap();
        let logs = log_dir(dir.path());
        std::fs::create_dir(&logs).unwrap();
        std::fs::write(logs.join("eqlog_Dorsk_erudin.txt"), "").unwrap();
        std::fs::write(logs.join("eqlog_Vala_erudin.txt"), "").unwrap();
        std::fs::write(logs.join("dbg.txt"), "").unwrap();
        std::fs::create_dir(logs.join("eqlog_Nested_erudin.txt")).unwrap();

        let found: Vec<String> = scan(dir.path())
            .unwrap()
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, ["eqlog_Dorsk_erudin.txt", "eqlog_Vala_erudin.txt"]);
    }

    #[test]
    fn the_latest_log_is_the_one_most_recently_written() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(latest(dir.path()), None, "no Logs directory at all");

        let logs = log_dir(dir.path());
        std::fs::create_dir(&logs).unwrap();
        assert_eq!(latest(dir.path()), None, "an empty Logs directory");

        let old = logs.join("eqlog_Dorsk_erudin.txt");
        std::fs::write(&old, "").unwrap();
        std::fs::File::open(&old)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .unwrap();
        let fresh = logs.join("eqlog_Vala_erudin.txt");
        std::fs::write(&fresh, "").unwrap();
        std::fs::write(logs.join("dbg.txt"), "").unwrap();

        assert_eq!(latest(dir.path()), Some(fresh));
    }

    #[test]
    fn timestamps_are_read_as_utc_civil_time() {
        assert_eq!(parse_timestamp("[Thu Jan  1 00:00:00 1970]"), Some(0));
        assert_eq!(
            parse_timestamp("[Tue Jul 21 21:15:23 2026]"),
            Some(1_784_668_523)
        );
        assert_eq!(
            parse_timestamp("[Sat Feb 29 12:00:00 2020]"),
            Some(1_582_977_600)
        );
        assert_eq!(
            parse_timestamp("[Wed Dec 31 23:59:59 1999]"),
            Some(946_684_799)
        );
    }

    #[test]
    fn malformed_timestamps_are_rejected() {
        assert_eq!(parse_timestamp("[Tue Jul 21 21:15:23]"), None);
        assert_eq!(parse_timestamp("[Tue Foo 21 21:15:23 2026]"), None);
        assert_eq!(parse_timestamp("Tue Jul 21 21:15:23 2026"), None);
        assert_eq!(parse_timestamp("[Tue Jul 21 25:15:23 2026]"), None);
        assert_eq!(parse_timestamp("[Tue Jul 32 21:15:23 2026]"), None);
        assert_eq!(parse_timestamp("[Tue Jul 21 21:15 2026]"), None);
        assert_eq!(parse_timestamp("[Tue Jul 21 21:15:23 2026 extra]"), None);
    }

    #[test]
    fn recognises_every_event_pattern() {
        assert_eq!(
            kind("[Tue Jul 21 21:15:23 2026] You have gained a level! Welcome to level 42!"),
            LogEventKind::Level { level: 42 }
        );
        assert_eq!(
            kind("[Tue Jul 21 21:15:23 2026] --You have looted a Rusty Dagger.--"),
            LogEventKind::Loot {
                item: "Rusty Dagger".into()
            }
        );
        assert_eq!(
            kind("[Tue Jul 21 21:15:23 2026] You have looted a Bone Chips."),
            LogEventKind::Loot {
                item: "Bone Chips".into()
            }
        );
        assert_eq!(
            kind("[Tue Jul 21 21:15:23 2026] You have entered East Commonlands."),
            LogEventKind::Zone {
                zone: "East Commonlands".into()
            }
        );
        assert_eq!(
            kind("[Tue Jul 21 21:15:23 2026] You have been slain by a gnoll pup!"),
            LogEventKind::Death {
                killer: Some("a gnoll pup".into())
            }
        );
        assert_eq!(
            kind("[Tue Jul 21 21:15:23 2026] You died."),
            LogEventKind::Death { killer: None }
        );
        assert_eq!(
            kind("[Tue Jul 21 21:15:23 2026] Your Location is 123.45, -678.90, 12.34"),
            LogEventKind::Location {
                y: 123.45,
                x: -678.90,
                z: 12.34
            }
        );
        assert_eq!(
            kind("[Tue Jul 21 21:15:23 2026] You have become better at Meditate! (61)"),
            LogEventKind::Skill {
                skill: "Meditate".into(),
                value: 61
            }
        );
        assert_eq!(
            kind("[Tue Jul 21 21:15:23 2026] You have become better at 1H Blunt! (7)"),
            LogEventKind::Skill {
                skill: "1H Blunt".into(),
                value: 7
            }
        );
    }

    /// Verbatim from the player's own log, trailing spaces and all.
    const WHO: &str =
        "[Wed Aug 05 19:29:24 2026] [15 WAR/DRU/NEC] Morveus (Dark Elf)  ZONE: The Estate of Unrest 2 (unrest_2)  ";

    #[test]
    fn our_own_who_row_carries_level_race_and_every_class() {
        assert_eq!(
            parse_line(WHO, "Morveus").unwrap().kind,
            LogEventKind::Who {
                level: 15,
                classes: vec!["WAR".into(), "DRU".into(), "NEC".into()],
                race: Some("Dark Elf".into()),
            }
        );
        assert_eq!(parse_line(WHO, "morveus").unwrap().at, 1_785_958_164);
    }

    #[test]
    fn a_single_class_and_a_bare_row_still_parse() {
        assert_eq!(
            kind("[Tue Jul 21 21:15:23 2026] [50 WAR] Dorsk (Barbarian) <A Guild>  ZONE: Befallen (befallen)"),
            LogEventKind::Who {
                level: 50,
                classes: vec!["WAR".into()],
                race: Some("Barbarian".into()),
            }
        );
        assert_eq!(
            kind("[Tue Jul 21 21:15:23 2026] [1 ENC] Dorsk"),
            LogEventKind::Who {
                level: 1,
                classes: vec!["ENC".into()],
                race: None,
            }
        );
    }

    #[test]
    fn somebody_elses_row_is_never_mistaken_for_ours() {
        assert_eq!(parse_line(WHO, "Dorsk"), None);
        for line in [
            "[Tue Jul 21 21:15:23 2026] [ANONYMOUS] Dorsk",
            "[Tue Jul 21 21:15:23 2026] [ANONYMOUS] Dorsk  ZONE: Befallen (befallen)",
            "[Tue Jul 21 21:15:23 2026] [15 Warrior] Dorsk (Barbarian)",
            "[Tue Jul 21 21:15:23 2026] [15 WAR/DRU/NEC/ROG] Dorsk (Barbarian)",
            "[Tue Jul 21 21:15:23 2026] [fifteen WAR] Dorsk (Barbarian)",
            "[Tue Jul 21 21:15:23 2026] [15WAR] Dorsk (Barbarian)",
            "[Tue Jul 21 21:15:23 2026] [15 WAR] Dorskly (Barbarian)",
        ] {
            assert_eq!(parse_line(line, "Dorsk"), None, "{line:?}");
        }
    }

    #[test]
    fn only_our_row_survives_a_whole_who_block() {
        let block = b"[Wed Aug 05 19:29:24 2026] Players in EverQuest Legends:\r\n\
            [Wed Aug 05 19:29:24 2026] ---------------------------\r\n\
            [Wed Aug 05 19:29:24 2026] [22 ROG] Duxx (Human)  ZONE: The Estate of Unrest 2 (unrest_2)  \r\n\
            [Wed Aug 05 19:29:24 2026] [15 WAR/DRU/NEC] Morveus (Dark Elf)  ZONE: The Estate of Unrest 2 (unrest_2)  \r\n\
            [Wed Aug 05 19:29:24 2026] There is 1 player in EverQuest Legends.\r\n";
        let (harvest, _) = harvest(block, "Morveus");
        assert_eq!(
            harvest.events.iter().map(|e| &e.kind).collect::<Vec<_>>(),
            vec![&LogEventKind::Who {
                level: 15,
                classes: vec!["WAR".into(), "DRU".into(), "NEC".into()],
                race: Some("Dark Elf".into()),
            }]
        );
    }

    #[test]
    fn chatter_and_near_misses_are_not_events() {
        for line in [
            "[Tue Jul 21 21:15:23 2026] Dorsk tells the guild, 'You have entered a bad deal.'",
            "[Tue Jul 21 21:15:23 2026] You have entered.",
            "[Tue Jul 21 21:15:23 2026] You have gained a level! Welcome to level twelve!",
            "[Tue Jul 21 21:15:23 2026] Your Location is 1.0, 2.0",
            "[Tue Jul 21 21:15:23 2026] You have become better at Meditate!",
            "[Tue Jul 21 21:15:23 2026] Welcome to EverQuest!",
            "You have entered East Commonlands.",
            "",
        ] {
            assert_eq!(parse_line(line, "Dorsk"), None, "{line:?}");
        }
    }

    #[test]
    fn timestamps_travel_with_the_event() {
        let event = parse_line(
            "[Tue Jul 21 21:15:23 2026] You have entered East Commonlands.",
            "Dorsk",
        )
        .unwrap();
        assert_eq!(event.at, 1_784_668_523);
    }

    #[test]
    fn a_trailing_partial_line_is_left_for_the_next_tick() {
        let chunk = b"[Tue Jul 21 21:15:23 2026] You died.\n[Tue Jul 21 21:15:24 2026] You have en";
        let (first, consumed) = harvest(chunk, "Dorsk");
        assert_eq!(consumed, 37);
        assert_eq!(first.events.len(), 1);
        assert_eq!(first.events[0].kind, LogEventKind::Death { killer: None });

        let (nothing, consumed) = harvest(b"no newline yet", "Dorsk");
        assert_eq!(consumed, 0);
        assert_eq!(nothing, Harvest::default());
    }

    #[test]
    fn unmatched_lines_are_counted_not_kept() {
        let chunk = b"[Tue Jul 21 21:15:23 2026] You died.\n\
            [Tue Jul 21 21:15:24 2026] Dorsk says, 'hi'\n\
            plain line without a timestamp\n\
            \n";
        let (harvest, consumed) = harvest(chunk, "Dorsk");
        assert_eq!(consumed, chunk.len());
        assert_eq!(harvest.events.len(), 1);
        assert_eq!(harvest.dropped, 2);
    }

    #[test]
    fn invalid_utf8_does_not_abort_the_chunk() {
        let mut chunk = b"[Tue Jul 21 21:15:23 2026] You have looted a caf".to_vec();
        chunk.push(0xff);
        chunk.extend_from_slice(b".--\n[Tue Jul 21 21:15:24 2026] You died.\n");
        let (harvest, consumed) = harvest(&chunk, "Dorsk");
        assert_eq!(consumed, chunk.len());
        assert_eq!(harvest.events.len(), 1);
        assert_eq!(harvest.events[0].kind, LogEventKind::Death { killer: None });
        assert_eq!(harvest.dropped, 1);
    }
}
