use eql_core::api::HARVEST_KINDS;
use std::path::{Path, PathBuf};

pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub kind: String,
    pub character: String,
    pub server: String,
    /// `WAR-CLR` from the DPS meter's per-build all-time files.
    pub build: Option<String>,
}

pub fn default_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        dirs::config_dir().map(|dir| dir.join("EQL Log Reader"))
    } else {
        dirs::data_dir().map(|dir| dir.join("eql-log-reader"))
    }
}

pub fn parse_filename(file_name: &str) -> Option<Identity> {
    let stem = file_name.strip_suffix(".json")?;
    let rest = stem.strip_prefix("eql_")?;
    let kind = HARVEST_KINDS.iter().find(|kind| {
        rest.strip_prefix(**kind)
            .is_some_and(|s| s.starts_with('_'))
    })?;
    let identity = &rest[kind.len() + 1..];
    let (identity, build) = match identity.split_once("__") {
        Some((identity, build)) => (identity, (!build.is_empty()).then(|| build.to_string())),
        None => (identity, None),
    };
    let (character, server) = identity.rsplit_once('_')?;
    if character.is_empty() || server.is_empty() {
        return None;
    }
    Some(Identity {
        kind: (*kind).to_string(),
        character: character.to_string(),
        server: server.to_string(),
        build,
    })
}

pub fn scan(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if entry
            .file_name()
            .to_str()
            .and_then(parse_filename)
            .is_some()
        {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(file_name: &str) -> (String, String, String, Option<String>) {
        let parsed = parse_filename(file_name).expect("parsable");
        (parsed.kind, parsed.character, parsed.server, parsed.build)
    }

    #[test]
    fn parses_each_harvested_kind() {
        assert_eq!(
            identity("eql_atlas_Dorsk_erudin.json"),
            ("atlas".into(), "Dorsk".into(), "erudin".into(), None)
        );
        assert_eq!(
            identity("eql_quest_Dorsk_erudin.json"),
            ("quest".into(), "Dorsk".into(), "erudin".into(), None)
        );
        assert_eq!(
            identity("eql_alltime_Dorsk_erudin.json"),
            ("alltime".into(), "Dorsk".into(), "erudin".into(), None)
        );
    }

    #[test]
    fn the_build_suffix_rides_beside_the_identity() {
        assert_eq!(
            identity("eql_alltime_Dorsk_erudin__WAR-CLR.json"),
            (
                "alltime".into(),
                "Dorsk".into(),
                "erudin".into(),
                Some("WAR-CLR".into())
            )
        );
        assert_eq!(
            identity("eql_alltime_Dorsk_erudin__.json"),
            ("alltime".into(), "Dorsk".into(), "erudin".into(), None)
        );
    }

    #[test]
    fn the_server_is_the_last_segment_so_odd_names_still_split() {
        assert_eq!(
            identity("eql_atlas_Dor_sk_erudin.json"),
            ("atlas".into(), "Dor_sk".into(), "erudin".into(), None)
        );
    }

    #[test]
    fn rejects_everything_that_is_not_a_per_character_file() {
        for name in [
            "eql_atlas_settings.json",
            "eql_atlas_baseline.json",
            "eql_quest_db.json.gz",
            "eql_session_report_records_Dorsk_erudin.json",
            "eql_friend_overlay_roster_Dorsk_erudin.json",
            "eql_atlas_Dorsk_erudin.json.tmp",
            "eql_spell_Dorsk_erudin.json",
            "eql_atlas_.json",
            "eql_atlas__erudin.json",
            "atlas_Dorsk_erudin.json",
        ] {
            assert!(parse_filename(name).is_none(), "{name} should be ignored");
        }
    }

    #[test]
    fn scan_finds_only_harvestable_files() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "eql_atlas_Dorsk_erudin.json",
            "eql_quest_Dorsk_erudin.json",
            "eql_alltime_Dorsk_erudin__WAR-CLR.json",
            "eql_atlas_settings.json",
            "eql_session_report_records_Dorsk_erudin.json",
            "eqlog_Dorsk_erudin.txt",
        ] {
            std::fs::write(dir.path().join(name), "{}").unwrap();
        }
        std::fs::create_dir(dir.path().join("eql_atlas_Nested_erudin.json")).unwrap();

        let found: Vec<String> = scan(dir.path())
            .unwrap()
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            found,
            vec![
                "eql_alltime_Dorsk_erudin__WAR-CLR.json",
                "eql_atlas_Dorsk_erudin.json",
                "eql_quest_Dorsk_erudin.json",
            ]
        );
    }
}
