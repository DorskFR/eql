use serde::{Deserialize, Serialize};

use crate::inventory::InventoryEntry;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryUpload {
    pub character: String,
    pub server: String,
    /// Unix seconds; the server substitutes its own clock when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<i64>,
    pub entries: Vec<InventoryEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// `doc` stays opaque: the third-party writer changes its schema freely, so
/// nothing here may fail to parse. Projection happens in the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarvestDoc {
    pub character: String,
    pub server: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<i64>,
    pub doc: serde_json::Value,
}

pub const HARVEST_KINDS: [&str; 3] = ["atlas", "quest", "alltime"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogBatch {
    pub character: String,
    pub server: String,
    pub events: Vec<LogEvent>,
}

/// `at` is unix seconds derived from the line's own timestamp, which the client
/// writes in local time with no offset; see `eqld::logs::parse_timestamp`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEvent {
    pub at: i64,
    #[serde(flatten)]
    pub kind: LogEventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogEventKind {
    Loot {
        item: String,
    },
    Level {
        level: u32,
    },
    Zone {
        zone: String,
    },
    Death {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        killer: Option<String>,
    },
    Location {
        y: f64,
        x: f64,
        z: f64,
    },
    Skill {
        skill: String,
        value: u32,
    },
    Who {
        level: u32,
        classes: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        race: Option<String>,
    },
}

impl LogEventKind {
    pub fn tag(&self) -> &'static str {
        match self {
            LogEventKind::Loot { .. } => "loot",
            LogEventKind::Level { .. } => "level",
            LogEventKind::Zone { .. } => "zone",
            LogEventKind::Death { .. } => "death",
            LogEventKind::Location { .. } => "location",
            LogEventKind::Skill { .. } => "skill",
            LogEventKind::Who { .. } => "who",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_minimal_payload() {
        let upload = InventoryUpload {
            character: "Dorsk".into(),
            server: "erudin".into(),
            captured_at: None,
            entries: vec![InventoryEntry {
                location: "Primary".into(),
                name: "Spirit Reaver".into(),
                id: 86755,
                count: 1,
                slots: 0,
            }],
            raw: None,
        };
        let json = serde_json::to_string(&upload).unwrap();
        assert!(!json.contains("captured_at"));
        assert!(!json.contains("raw"));
        assert_eq!(
            serde_json::from_str::<InventoryUpload>(&json).unwrap(),
            upload
        );
    }

    #[test]
    fn accepts_optional_fields() {
        let json = r#"{
            "character": "Dorsk",
            "server": "erudin",
            "captured_at": 1754390000,
            "entries": [{"location":"General1","name":"Backpack","id":17963,"count":1,"slots":8}],
            "raw": "Location\tName\tID\tCount\tSlots\n"
        }"#;
        let upload: InventoryUpload = serde_json::from_str(json).unwrap();
        assert_eq!(upload.captured_at, Some(1_754_390_000));
        assert_eq!(upload.entries[0].slots, 8);
        assert!(upload.raw.is_some());
    }

    #[test]
    fn harvest_docs_keep_unknown_shapes_intact() {
        let json = r#"{
            "character": "Dorsk",
            "server": "erudin",
            "kind": "atlas",
            "captured_at": 1754390000,
            "doc": {"format": 1, "totals": {"kills": 3}, "future_field": [1, {"a": null}]}
        }"#;
        let harvest: HarvestDoc = serde_json::from_str(json).unwrap();
        assert_eq!(harvest.kind, "atlas");
        assert_eq!(harvest.captured_at, Some(1_754_390_000));
        assert_eq!(harvest.doc["totals"]["kills"], 3);
        assert_eq!(harvest.doc["future_field"][1]["a"], serde_json::Value::Null);
        let round_tripped: HarvestDoc =
            serde_json::from_str(&serde_json::to_string(&harvest).unwrap()).unwrap();
        assert_eq!(round_tripped, harvest);
    }

    #[test]
    fn harvest_docs_omit_an_absent_capture_time() {
        let harvest = HarvestDoc {
            character: "Dorsk".into(),
            server: "erudin".into(),
            kind: "quest".into(),
            captured_at: None,
            doc: serde_json::json!([]),
        };
        let json = serde_json::to_string(&harvest).unwrap();
        assert!(!json.contains("captured_at"));
        assert_eq!(serde_json::from_str::<HarvestDoc>(&json).unwrap(), harvest);
    }

    #[test]
    fn events_serialise_flat_with_a_kind_tag() {
        let batch = LogBatch {
            character: "Dorsk".into(),
            server: "erudin".into(),
            events: vec![
                LogEvent {
                    at: 1_753_132_523,
                    kind: LogEventKind::Loot {
                        item: "Rusty Dagger".into(),
                    },
                },
                LogEvent {
                    at: 1_753_132_530,
                    kind: LogEventKind::Death { killer: None },
                },
            ],
        };
        let json = serde_json::to_value(&batch).unwrap();
        assert_eq!(json["events"][0]["kind"], "loot");
        assert_eq!(json["events"][0]["item"], "Rusty Dagger");
        assert_eq!(json["events"][0]["at"], 1_753_132_523);
        assert_eq!(json["events"][1]["kind"], "death");
        assert!(json["events"][1].get("killer").is_none());
        assert_eq!(
            serde_json::from_value::<LogBatch>(json).unwrap().events,
            batch.events
        );
    }

    #[test]
    fn every_kind_round_trips_and_tags_itself() {
        let kinds = [
            LogEventKind::Loot { item: "x".into() },
            LogEventKind::Level { level: 42 },
            LogEventKind::Zone { zone: "z".into() },
            LogEventKind::Death {
                killer: Some("a gnoll".into()),
            },
            LogEventKind::Location {
                y: 1.5,
                x: -2.0,
                z: 0.25,
            },
            LogEventKind::Skill {
                skill: "Meditate".into(),
                value: 7,
            },
            LogEventKind::Who {
                level: 15,
                classes: vec!["WAR".into(), "DRU".into(), "NEC".into()],
                race: Some("Dark Elf".into()),
            },
            LogEventKind::Who {
                level: 1,
                classes: vec!["WAR".into()],
                race: None,
            },
        ];
        for kind in kinds {
            let json = serde_json::to_value(&kind).unwrap();
            assert_eq!(json["kind"], kind.tag());
            assert_eq!(serde_json::from_value::<LogEventKind>(json).unwrap(), kind);
        }
    }
}
