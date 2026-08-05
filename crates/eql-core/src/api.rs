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
}
