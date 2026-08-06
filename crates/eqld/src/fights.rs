use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A fight is history, not a snapshot: the server stores every one it has not
/// seen before, keyed on (character, server, start_wall), so a fight sent
/// twice is dropped rather than replacing anything.
#[derive(Debug, Serialize)]
pub struct Upload<'a> {
    pub character: &'a str,
    pub server: &'a str,
    pub fights: &'a [Value],
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Emitted {
    Envelope {
        #[serde(default)]
        fights: Vec<Value>,
    },
    Bare(Vec<Value>),
}

impl Emitted {
    fn fights(self) -> Vec<Value> {
        match self {
            Emitted::Envelope { fights } => fights,
            Emitted::Bare(fights) => fights,
        }
    }
}

pub fn out_path(dir: &Path, character: &str, server: &str) -> PathBuf {
    dir.join(format!("eql_fights_{character}_{server}.json"))
}

pub fn parse(text: &str) -> Result<Vec<Value>, serde_json::Error> {
    serde_json::from_str::<Emitted>(text).map(Emitted::fights)
}

/// Whole seconds would collide with the log's one-second stamps, so the
/// watermark is milliseconds — an integer, which `State` can compare.
pub fn start_wall_ms(fight: &Value) -> Option<i64> {
    let seconds = fight.get("start_wall")?.as_f64()?;
    if !seconds.is_finite() {
        return None;
    }
    Some((seconds * 1000.0).round() as i64)
}

pub fn since_arg(watermark: i64) -> String {
    format!("{:.3}", watermark as f64 / 1000.0)
}

/// The tool is asked for these already; filtering again means a build that
/// ignores `--since` still cannot make us re-post what we have shipped.
pub fn newer_than(fights: Vec<Value>, watermark: Option<i64>) -> Vec<Value> {
    let mut fights: Vec<Value> = fights
        .into_iter()
        .filter(|fight| match (start_wall_ms(fight), watermark) {
            (Some(at), Some(watermark)) => at > watermark,
            (Some(_), None) => true,
            (None, _) => false,
        })
        .collect();
    fights.sort_by_key(|fight| start_wall_ms(fight).unwrap_or(i64::MIN));
    fights
}

pub fn newest(fights: &[Value]) -> Option<i64> {
    fights.iter().filter_map(start_wall_ms).max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fight(start: f64) -> Value {
        json!({ "start_wall": start, "zone": "Clan Crushbone", "kills": 3 })
    }

    #[test]
    fn the_out_file_is_named_after_the_character_and_server() {
        assert_eq!(
            out_path(Path::new("/var/eqld/fights"), "Dorsk", "erudin"),
            PathBuf::from("/var/eqld/fights/eql_fights_Dorsk_erudin.json")
        );
    }

    #[test]
    fn reads_the_tools_envelope_and_ignores_the_rest_of_it() {
        let fights = parse(
            r#"{"character":"Dorsk","server":"erudin","log":"/l.txt",
                "fights":[{"start_wall":1785931338.0}]}"#,
        )
        .unwrap();
        assert_eq!(fights.len(), 1);
        assert_eq!(start_wall_ms(&fights[0]), Some(1_785_931_338_000));

        assert!(parse(r#"{"character":"Dorsk"}"#).unwrap().is_empty());
        assert_eq!(
            parse(r#"[{"start_wall":1785931338.0}]"#).unwrap().len(),
            1,
            "a bare list of fights is the same dump without its envelope"
        );
        assert!(parse("{ not json").is_err());
    }

    #[test]
    fn a_fight_without_a_usable_start_is_dropped() {
        assert_eq!(start_wall_ms(&json!({})), None);
        assert_eq!(start_wall_ms(&json!({ "start_wall": "soon" })), None);
        assert_eq!(start_wall_ms(&json!({ "start_wall": f64::NAN })), None);
        assert!(newer_than(vec![json!({ "kills": 1 })], None).is_empty());
    }

    #[test]
    fn only_fights_past_the_watermark_survive_and_they_come_out_in_order() {
        let fights = vec![fight(300.0), fight(100.0), fight(200.5)];
        assert_eq!(newer_than(fights.clone(), None).len(), 3);

        let new = newer_than(fights.clone(), Some(100_000));
        assert_eq!(
            new.iter().filter_map(start_wall_ms).collect::<Vec<_>>(),
            vec![200_500, 300_000]
        );
        assert_eq!(newest(&new), Some(300_000));
        assert!(newer_than(fights, Some(300_000)).is_empty());
        assert_eq!(newest(&[]), None);
    }

    #[test]
    fn the_watermark_goes_back_to_the_tool_as_seconds() {
        assert_eq!(since_arg(1_785_931_338_000), "1785931338.000");
        assert_eq!(since_arg(1_785_931_338_500), "1785931338.500");
    }
}
