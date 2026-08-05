use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryEntry {
    pub location: String,
    pub name: String,
    pub id: u64,
    pub count: u32,
    pub slots: u32,
}

impl InventoryEntry {
    pub fn is_empty_slot(&self) -> bool {
        self.name == "Empty"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventorySnapshot {
    pub character: String,
    pub server: String,
    pub entries: Vec<InventoryEntry>,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("missing or malformed header line")]
    BadHeader,
    #[error("line {0}: expected 5 tab-separated fields, got {1}")]
    BadFieldCount(usize, usize),
    #[error("line {0}: {1}")]
    BadNumber(usize, std::num::ParseIntError),
    #[error("filename {0:?} does not match <Player>_<server>-Inventory.txt")]
    BadFilename(String),
}

pub fn parse_filename(file_name: &str) -> Result<(String, String), ParseError> {
    let stem = file_name
        .strip_suffix("-Inventory.txt")
        .ok_or_else(|| ParseError::BadFilename(file_name.into()))?;
    let (character, server) = stem
        .rsplit_once('_')
        .ok_or_else(|| ParseError::BadFilename(file_name.into()))?;
    if character.is_empty() || server.is_empty() {
        return Err(ParseError::BadFilename(file_name.into()));
    }
    Ok((character.into(), server.into()))
}

pub fn parse(contents: &str) -> Result<Vec<InventoryEntry>, ParseError> {
    let mut lines = contents.lines().enumerate();
    let (_, header) = lines.next().ok_or(ParseError::BadHeader)?;
    let header_fields: Vec<&str> = header.trim_end_matches('\r').split('\t').collect();
    if header_fields != ["Location", "Name", "ID", "Count", "Slots"] {
        return Err(ParseError::BadHeader);
    }

    let mut entries = Vec::new();
    for (idx, line) in lines {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 5 {
            return Err(ParseError::BadFieldCount(idx + 1, fields.len()));
        }
        let num = |s: &str| s.parse().map_err(|e| ParseError::BadNumber(idx + 1, e));
        entries.push(InventoryEntry {
            location: fields[0].into(),
            name: fields[1].into(),
            id: num(fields[2])?,
            count: num(fields[3])? as u32,
            slots: num(fields[4])? as u32,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Location\tName\tID\tCount\tSlots\n\
        Charm\tEmpty\t0\t1\t0\n\
        Primary\tSpirit Reaver\t86755\t1\t0\n\
        General1\tBackpack\t17963\t1\t8\n\
        General1-Slot1\tBone Chips\t13073\t20\t0\n";

    #[test]
    fn parses_sample_dump() {
        let entries = parse(SAMPLE).unwrap();
        assert_eq!(entries.len(), 4);
        assert!(entries[0].is_empty_slot());
        assert_eq!(entries[1].name, "Spirit Reaver");
        assert_eq!(entries[1].id, 86755);
        assert_eq!(entries[3].location, "General1-Slot1");
        assert_eq!(entries[3].count, 20);
    }

    #[test]
    fn tolerates_crlf() {
        let crlf = SAMPLE.replace('\n', "\r\n");
        assert_eq!(parse(&crlf).unwrap().len(), 4);
    }

    #[test]
    fn rejects_bad_header() {
        assert!(matches!(
            parse("Nope\tHeader\n"),
            Err(ParseError::BadHeader)
        ));
    }

    #[test]
    fn filename_roundtrip() {
        let (c, s) = parse_filename("Dorsk_erudin-Inventory.txt").unwrap();
        assert_eq!((c.as_str(), s.as_str()), ("Dorsk", "erudin"));
        assert!(parse_filename("eqlog_Dorsk_erudin.txt").is_err());
    }
}
