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

pub const ALLTIME: &str = "alltime";

/// The key a build-less all-time file takes inside a merged document; the web
/// projection labels an unnamed build the same way.
pub const UNNAMED_BUILD: &str = "Current build";

/// The files behind one uploaded document. The server stores harvest docs
/// unique on (character, kind), so a character's per-build all-time files have
/// to arrive as one document or they overwrite each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub key: String,
    pub kind: String,
    pub character: String,
    pub server: String,
    pub files: Vec<(PathBuf, Option<String>)>,
}

impl Group {
    fn one(path: PathBuf, identity: Identity) -> Self {
        Self {
            key: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            kind: identity.kind,
            character: identity.character,
            server: identity.server,
            files: vec![(path, identity.build)],
        }
    }

    /// Named after the identity rather than a file, so it cannot collide with
    /// the per-file keys already in state.
    fn merged_key(&self) -> String {
        format!("{}:{}_{}", self.kind, self.character, self.server)
    }

    pub fn wraps_builds(&self) -> bool {
        self.files.iter().any(|(_, build)| build.is_some())
    }

    pub fn document(&self, docs: Vec<serde_json::Value>) -> serde_json::Value {
        if !self.wraps_builds() {
            return docs.into_iter().next().unwrap_or(serde_json::Value::Null);
        }
        let builds: serde_json::Map<String, serde_json::Value> = self
            .files
            .iter()
            .map(|(_, build)| build.clone().unwrap_or_else(|| UNNAMED_BUILD.to_string()))
            .zip(docs)
            .collect();
        serde_json::json!({ "builds": builds })
    }
}

pub fn group(paths: Vec<PathBuf>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for path in paths {
        let Some(identity) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(parse_filename)
        else {
            continue;
        };
        let existing = (identity.kind == ALLTIME)
            .then(|| {
                groups.iter_mut().find(|group| {
                    group.kind == identity.kind
                        && group.character == identity.character
                        && group.server == identity.server
                })
            })
            .flatten();
        match existing {
            Some(group) => {
                group.files.push((path, identity.build));
                group.key = group.merged_key();
            }
            None => groups.push(Group::one(path, identity)),
        }
    }
    groups
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

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|name| PathBuf::from("/h").join(name))
            .collect()
    }

    fn doc(kills: u32) -> serde_json::Value {
        serde_json::json!({ "kills": kills, "source_dmg": { "melee": 10 } })
    }

    #[test]
    fn every_kind_but_a_second_all_time_build_stands_alone() {
        let groups = group(paths(&[
            "eql_atlas_Dorsk_erudin.json",
            "eql_quest_Dorsk_erudin.json",
            "eql_alltime_Dorsk_erudin__WAR-CLR.json",
            "eql_alltime_Vala_erudin.json",
            "not-a-harvest-file.txt",
        ]));
        assert_eq!(
            groups
                .iter()
                .map(|group| group.key.as_str())
                .collect::<Vec<_>>(),
            vec![
                "eql_atlas_Dorsk_erudin.json",
                "eql_quest_Dorsk_erudin.json",
                "eql_alltime_Dorsk_erudin__WAR-CLR.json",
                "eql_alltime_Vala_erudin.json",
            ]
        );
        assert!(groups.iter().all(|group| group.files.len() == 1));
    }

    #[test]
    fn one_characters_builds_merge_into_one_document() {
        let groups = group(paths(&[
            "eql_alltime_Dorsk_erudin__WAR-CLR.json",
            "eql_alltime_Dorsk_erudin__WAR-SHM.json",
            "eql_atlas_Dorsk_erudin.json",
        ]));
        assert_eq!(groups.len(), 2);
        let merged = &groups[0];
        assert_eq!(merged.key, "alltime:Dorsk_erudin");
        assert_eq!(merged.character, "Dorsk");
        assert_eq!(merged.files.len(), 2);
        assert!(merged.wraps_builds());
        assert_eq!(
            merged.document(vec![doc(1), doc(2)]),
            serde_json::json!({
                "builds": { "WAR-CLR": doc(1), "WAR-SHM": doc(2) }
            })
        );
    }

    #[test]
    fn a_named_build_is_wrapped_even_on_its_own() {
        let groups = group(paths(&["eql_alltime_Dorsk_erudin__WAR-CLR.json"]));
        assert!(groups[0].wraps_builds());
        assert_eq!(
            groups[0].document(vec![doc(1)]),
            serde_json::json!({ "builds": { "WAR-CLR": doc(1) } })
        );
    }

    #[test]
    fn an_unnamed_build_ships_exactly_as_the_reader_wrote_it() {
        for name in [
            "eql_alltime_Dorsk_erudin.json",
            "eql_atlas_Dorsk_erudin.json",
        ] {
            let groups = group(paths(&[name]));
            assert!(!groups[0].wraps_builds());
            assert_eq!(groups[0].document(vec![doc(1)]), doc(1));
        }
    }

    #[test]
    fn a_legacy_file_beside_a_named_build_keeps_its_own_slot() {
        let groups = group(paths(&[
            "eql_alltime_Dorsk_erudin.json",
            "eql_alltime_Dorsk_erudin__WAR-CLR.json",
        ]));
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].document(vec![doc(1), doc(2)]),
            serde_json::json!({
                "builds": { UNNAMED_BUILD: doc(1), "WAR-CLR": doc(2) }
            })
        );
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
