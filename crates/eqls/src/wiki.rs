use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemEffect {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub restriction: Option<String>,
    #[serde(default)]
    pub casting_time: Option<String>,
    #[serde(default)]
    pub level: Option<i64>,
    #[serde(default)]
    pub cooldown_seconds: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemStats {
    pub name: String,
    pub icon: Option<i64>,
    pub slots: Vec<String>,
    pub classes: Vec<String>,
    pub races: Vec<String>,
    pub deity: Option<String>,
    pub item_type: Option<String>,
    pub ac: Option<i64>,
    pub hp: Option<i64>,
    pub mana: Option<i64>,
    pub endurance: Option<i64>,
    pub hp_regen: Option<i64>,
    pub mana_regen: Option<i64>,
    #[serde(rename = "str")]
    pub strength: Option<i64>,
    pub sta: Option<i64>,
    pub agi: Option<i64>,
    pub dex: Option<i64>,
    pub wis: Option<i64>,
    #[serde(rename = "int")]
    pub intelligence: Option<i64>,
    pub cha: Option<i64>,
    pub sv_fire: Option<i64>,
    pub sv_cold: Option<i64>,
    pub sv_magic: Option<i64>,
    pub sv_disease: Option<i64>,
    pub sv_poison: Option<i64>,
    pub damage: Option<i64>,
    pub delay: Option<i64>,
    pub backstab: Option<i64>,
    pub range: Option<i64>,
    pub haste: Option<i64>,
    pub weight: Option<f64>,
    pub size: Option<String>,
    pub capacity: Option<i64>,
    pub size_capacity: Option<String>,
    pub weight_reduction: Option<i64>,
    pub charges: Option<i64>,
    pub required_level: Option<i64>,
    pub magic: bool,
    pub lore: bool,
    pub no_drop: bool,
    pub no_trade: bool,
    pub temporary: bool,
    pub expendable: bool,
    pub quest_item: bool,
    pub effects: Vec<ItemEffect>,
    pub focus_effect: Option<String>,
    pub unparsed: Vec<String>,
}

const SLOT_NAMES: &[&str] = &[
    "CHARM",
    "EAR",
    "EARS",
    "HEAD",
    "FACE",
    "NECK",
    "SHOULDER",
    "SHOULDERS",
    "ARMS",
    "BACK",
    "WRIST",
    "WRISTS",
    "RANGE",
    "HANDS",
    "PRIMARY",
    "SECONDARY",
    "FINGER",
    "FINGERS",
    "CHEST",
    "LEGS",
    "FEET",
    "WAIST",
    "AMMO",
    "POWER SOURCE",
];

const FLAG_PHRASES: &[(&str, u8)] = &[
    ("MAGIC ITEM", 0),
    ("LORE ITEM", 1),
    ("NO DROP", 2),
    ("NO TRADE", 3),
    ("QUEST ITEM", 4),
    ("TEMPORARY", 5),
    ("EXPENDABLE", 6),
    ("MAGIC", 0),
    ("LORE", 1),
];

fn key_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"([A-Za-z][A-Za-z0-9'\-]*(?:[ ][A-Za-z][A-Za-z0-9'\-]*)?)[ ]*:").unwrap()
    })
}

fn level_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bat +level +(\d+)").unwrap())
}

fn tag_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<[^>]*>").unwrap())
}

/// Splits a `{{Template|a=1|b=2}}` call into its named parameters, ignoring
/// pipes nested inside templates, links and tables.
pub fn template_params(wikitext: &str, template: &str) -> Option<Vec<(String, String)>> {
    let needle = format!("{{{{{template}");
    let start = wikitext.find(&needle)?;
    let body = &wikitext[start + 2..];

    let bytes = body.as_bytes();
    let mut brace = 1i32;
    let mut link = 0i32;
    let mut cuts = Vec::new();
    let mut i = 0usize;
    let end = loop {
        if i >= bytes.len() {
            return None;
        }
        if bytes[i..].starts_with(b"{{") {
            brace += 1;
            i += 2;
        } else if bytes[i..].starts_with(b"}}") {
            brace -= 1;
            if brace == 0 {
                break i;
            }
            i += 2;
        } else if bytes[i..].starts_with(b"[[") {
            link += 1;
            i += 2;
        } else if bytes[i..].starts_with(b"]]") {
            link = (link - 1).max(0);
            i += 2;
        } else {
            if bytes[i] == b'|' && brace == 1 && link == 0 {
                cuts.push(i);
            }
            i += 1;
        }
    };

    let mut params = Vec::new();
    for (n, &cut) in cuts.iter().enumerate() {
        let stop = cuts.get(n + 1).copied().unwrap_or(end);
        let chunk = &body[cut + 1..stop];
        if let Some((key, value)) = chunk.split_once('=') {
            let key = key.trim();
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                params.push((key.to_ascii_lowercase(), value.trim().to_string()));
            }
        }
    }
    Some(params)
}

pub fn parse_item(page_title: &str, wikitext: &str) -> Option<ItemStats> {
    let params = template_params(wikitext, "Itempage")?;
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };

    let name = get("itemname")
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(page_title)
        .to_string();

    let mut stats = ItemStats {
        name,
        icon: get("lucy_img_id").and_then(|v| v.trim().parse().ok()),
        ..Default::default()
    };

    if let Some(block) = get("statsblock") {
        parse_statsblock(block, &mut stats);
    }
    if let Some(focus) = get("focus_effect").map(str::trim).filter(|f| !f.is_empty()) {
        let focus = clean_text(focus);
        stats.effects.push(ItemEffect {
            kind: "focus".into(),
            name: focus.clone(),
            ..Default::default()
        });
        stats.focus_effect = Some(focus);
    }
    Some(stats)
}

fn parse_statsblock(block: &str, stats: &mut ItemStats) {
    for raw in block
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("<br>", "\n")
        .lines()
    {
        let line = clean_text(raw);
        if line.is_empty() {
            continue;
        }
        parse_line(&line, stats);
    }
}

fn parse_line(line: &str, stats: &mut ItemStats) {
    if let Some((label, rest)) = effect_label(line) {
        parse_effect(label, rest, stats);
        return;
    }

    let matches: Vec<_> = key_regex().find_iter(line).collect();
    if matches.is_empty() {
        absorb_flags(line, stats);
        return;
    }

    absorb_flags(&line[..matches[0].start()], stats);
    for (n, m) in matches.iter().enumerate() {
        let stop = matches.get(n + 1).map_or(line.len(), |next| next.start());
        let raw_key = m.as_str().trim_end_matches(':').trim();
        let value = line[m.end()..stop].trim();
        let (label, leftover) = resolve_label(raw_key);
        if let Some(leftover) = leftover {
            absorb_flags(leftover, stats);
        }
        match label {
            Some(label) => assign(label, value, stats),
            None => stats.unparsed.push(raw_key.to_ascii_uppercase()),
        }
    }
}

/// Splits a two-word key whose head is noise (`Expendable Charges`) so the tail
/// still lands in a field and the head is scanned for flags.
fn resolve_label(raw_key: &str) -> (Option<String>, Option<&str>) {
    let upper = normalize_key(raw_key);
    if is_known_label(&upper) {
        return (Some(upper), None);
    }
    if let Some((head, tail)) = raw_key.rsplit_once(' ') {
        let tail_upper = normalize_key(tail);
        if is_known_label(&tail_upper) {
            return (Some(tail_upper), Some(head));
        }
    }
    (None, None)
}

fn normalize_key(key: &str) -> String {
    key.trim().to_ascii_uppercase().replace('_', " ")
}

fn is_known_label(label: &str) -> bool {
    matches!(
        label,
        "SLOT"
            | "CLASS"
            | "CLASSES"
            | "RACE"
            | "RACES"
            | "DEITY"
            | "SKILL"
            | "AC"
            | "HP"
            | "HIT POINTS"
            | "MANA"
            | "END"
            | "ENDURANCE"
            | "HP REGEN"
            | "MANA REGEN"
            | "STR"
            | "STA"
            | "AGI"
            | "DEX"
            | "WIS"
            | "INT"
            | "CHA"
            | "SV FIRE"
            | "SV COLD"
            | "SV MAGIC"
            | "SV DISEASE"
            | "SV POISON"
            | "DMG"
            | "DAMAGE"
            | "BACKSTAB"
            | "ATK DELAY"
            | "DELAY"
            | "RANGE"
            | "HASTE"
            | "WT"
            | "WEIGHT"
            | "SIZE"
            | "CAPACITY"
            | "SIZE CAPACITY"
            | "WEIGHT REDUCTION"
            | "CHARGES"
            | "REQUIRED LEVEL"
            | "RECOMMENDED LEVEL"
    )
}

fn assign(label: String, value: &str, stats: &mut ItemStats) {
    let num = |stats: &mut ItemStats| match parse_int(value) {
        Some(n) => Some(n),
        None => {
            stats.unparsed.push(label.clone());
            None
        }
    };
    match label.as_str() {
        "SLOT" => stats.slots.extend(tokens(value)),
        "CLASS" | "CLASSES" => stats.classes.extend(tokens(value)),
        "RACE" | "RACES" => stats.races.extend(tokens(value)),
        "DEITY" => stats.deity = Some(value.to_string()),
        "SKILL" => stats.item_type = Some(value.to_string()),
        "SIZE" => stats.size = Some(value.to_ascii_uppercase()),
        "SIZE CAPACITY" => stats.size_capacity = Some(value.to_ascii_uppercase()),
        "AC" => stats.ac = num(stats),
        "HP" | "HIT POINTS" => stats.hp = num(stats),
        "MANA" => stats.mana = num(stats),
        "END" | "ENDURANCE" => stats.endurance = num(stats),
        "HP REGEN" => stats.hp_regen = num(stats),
        "MANA REGEN" => stats.mana_regen = num(stats),
        "STR" => stats.strength = num(stats),
        "STA" => stats.sta = num(stats),
        "AGI" => stats.agi = num(stats),
        "DEX" => stats.dex = num(stats),
        "WIS" => stats.wis = num(stats),
        "INT" => stats.intelligence = num(stats),
        "CHA" => stats.cha = num(stats),
        "SV FIRE" => stats.sv_fire = num(stats),
        "SV COLD" => stats.sv_cold = num(stats),
        "SV MAGIC" => stats.sv_magic = num(stats),
        "SV DISEASE" => stats.sv_disease = num(stats),
        "SV POISON" => stats.sv_poison = num(stats),
        "DMG" | "DAMAGE" => stats.damage = num(stats),
        "BACKSTAB" => stats.backstab = num(stats),
        "ATK DELAY" | "DELAY" => stats.delay = num(stats),
        "RANGE" => stats.range = num(stats),
        "HASTE" => stats.haste = num(stats),
        "CAPACITY" => stats.capacity = num(stats),
        "WEIGHT REDUCTION" => stats.weight_reduction = num(stats),
        "CHARGES" => stats.charges = num(stats),
        "REQUIRED LEVEL" | "RECOMMENDED LEVEL" => stats.required_level = num(stats),
        "WT" | "WEIGHT" => match parse_float(value) {
            Some(w) => stats.weight = Some(w),
            None => stats.unparsed.push(label),
        },
        _ => stats.unparsed.push(label),
    }
}

fn effect_label(line: &str) -> Option<(&'static str, &str)> {
    for (prefix, kind) in [
        ("effect:", "proc"),
        ("click effect:", "click"),
        ("clicky effect:", "click"),
        ("combat effect:", "proc"),
        ("worn effect:", "worn"),
        ("focus effect:", "focus"),
    ] {
        if line.len() >= prefix.len() && line[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return Some((kind, line[prefix.len()..].trim()));
        }
    }
    None
}

fn parse_effect(kind: &str, rest: &str, stats: &mut ItemStats) {
    let mut effect = ItemEffect {
        kind: kind.to_string(),
        ..Default::default()
    };

    let (head, tail) = match rest.split_once('(') {
        Some((head, tail)) => (head, tail),
        None => (rest, ""),
    };
    effect.name = clean_text(head).trim_end_matches(['-', ',']).trim().into();

    let (inside, after) = match tail.rsplit_once(')') {
        Some((inside, after)) => (inside, after),
        None => ("", tail),
    };

    for (n, part) in inside.split(',').map(str::trim).enumerate() {
        if part.is_empty() {
            continue;
        }
        match part.split_once(':') {
            Some((key, value)) if key.trim().eq_ignore_ascii_case("casting time") => {
                effect.casting_time = Some(value.trim().to_string());
            }
            _ if n == 0 => effect.restriction = Some(part.to_string()),
            _ => stats
                .unparsed
                .push(format!("EFFECT/{}", part.to_ascii_uppercase())),
        }
    }

    if let Some(caps) = level_regex().captures(after) {
        effect.level = caps[1].parse().ok();
    }
    for part in after.split(',') {
        let Some((key, value)) = part.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key
            .trim()
            .trim_start_matches('-')
            .trim()
            .to_ascii_uppercase()
            .as_str()
        {
            "CAST TIME" | "CASTING TIME" => effect.casting_time = Some(value.to_string()),
            "REQUIRED LEVEL" | "LEVEL" => {
                effect.level = parse_int(value);
                stats.required_level = effect.level;
            }
            "COOLDOWN" | "RECAST" => effect.cooldown_seconds = parse_int(value),
            other => stats.unparsed.push(format!("EFFECT/{other}")),
        }
    }

    if effect.kind == "proc" {
        effect.kind = match effect.restriction.as_deref() {
            Some(r) if r.eq_ignore_ascii_case("combat") => "proc",
            Some(r) if r.eq_ignore_ascii_case("worn") => "worn",
            Some(_) => "click",
            None => "unknown",
        }
        .to_string();
    }
    stats.effects.push(effect);
}

fn absorb_flags(text: &str, stats: &mut ItemStats) {
    let mut remaining = text.to_ascii_uppercase();
    for (phrase, bit) in FLAG_PHRASES {
        while let Some(at) = remaining.find(phrase) {
            remaining.replace_range(at..at + phrase.len(), " ");
            match bit {
                0 => stats.magic = true,
                1 => stats.lore = true,
                2 => stats.no_drop = true,
                3 => stats.no_trade = true,
                4 => stats.quest_item = true,
                5 => stats.temporary = true,
                _ => stats.expendable = true,
            }
        }
    }
    for token in remaining.split_whitespace() {
        let token = token.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if token.is_empty() {
            continue;
        }
        if SLOT_NAMES.contains(&token) {
            stats.slots.push(token.to_string());
        } else {
            stats.unparsed.push(token.to_string());
        }
    }
}

fn tokens(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(|t| t.trim_matches(',').to_ascii_uppercase())
        .filter(|t| !t.is_empty())
        .collect()
}

fn parse_int(value: &str) -> Option<i64> {
    let cleaned: String = value
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '+' || *c == '-' || *c == '%')
        .filter(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    if cleaned.is_empty() || cleaned == "-" {
        return None;
    }
    let negative = value.trim_start().starts_with('-');
    cleaned
        .trim_start_matches('-')
        .parse::<i64>()
        .ok()
        .map(|n| if negative { -n } else { n })
}

fn parse_float(value: &str) -> Option<f64> {
    let cleaned: String = value
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .filter(|c| *c != '+')
        .collect();
    cleaned.parse().ok()
}

fn clean_text(text: &str) -> String {
    let text = tag_regex().replace_all(text, " ");
    let text = text
        .replace("&nbsp;", " ")
        .replace("'''", "")
        .replace("''", "");
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(at) = rest.find("[[") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        match after.find("]]") {
            Some(close) => {
                let inner = &after[..close];
                out.push_str(inner.split('|').next_back().unwrap_or(inner));
                rest = &after[close + 2..];
            }
            None => {
                out.push_str(after);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/wiki/");
        std::fs::read_to_string(format!("{path}{name}.wikitext"))
            .unwrap_or_else(|e| panic!("fixture {name}: {e}"))
    }

    fn parse(name: &str) -> ItemStats {
        let text = fixture(name);
        parse_item(&name.replace('_', " "), &text).expect("Itempage template")
    }

    #[test]
    fn multibyte_chars_between_delimiters_do_not_panic() {
        let text =
            "{{Itempage|itemname=Test – Item|statsblock=WT: 0.5 – Size: SMALL|notes=café ×2}}";
        let params = template_params(text, "Itempage").expect("params");
        assert_eq!(params[0].1, "Test – Item");
    }

    #[test]
    fn weapon_with_proc() {
        let item = parse("Spirit_Reaver");
        assert_eq!(item.icon, Some(576));
        assert_eq!(item.name, "Spirit Reaver");
        assert_eq!(item.slots, ["PRIMARY"]);
        assert_eq!(item.item_type.as_deref(), Some("1H Slashing"));
        assert_eq!(item.damage, Some(13));
        assert_eq!(item.delay, Some(27));
        assert_eq!(item.weight, Some(0.4));
        assert_eq!(item.size.as_deref(), Some("MEDIUM"));
        assert_eq!(item.classes, ["SHD"]);
        assert_eq!(item.races, ["ALL"]);
        assert!(item.magic && item.lore);
        assert!(!item.no_drop);
        assert_eq!(item.ac, None);
        assert_eq!(
            item.effects,
            [ItemEffect {
                kind: "proc".into(),
                name: "Lifespike".into(),
                restriction: Some("Combat".into()),
                casting_time: Some("Instant".into()),
                level: Some(20),
                cooldown_seconds: None,
            }]
        );
        assert!(item.unparsed.is_empty(), "{:?}", item.unparsed);
    }

    #[test]
    fn armor_with_regen() {
        let item = parse("Rubicite_Breastplate");
        assert_eq!(item.icon, Some(624));
        assert_eq!(item.name, "Rubicite Breastplate");
        assert_eq!(item.slots, ["CHEST"]);
        assert_eq!(item.ac, Some(19));
        assert_eq!(item.hp_regen, Some(6));
        assert_eq!(item.weight, Some(6.0));
        assert_eq!(item.size.as_deref(), Some("LARGE"));
        assert_eq!(
            item.classes,
            ["WAR", "CLR", "PAL", "RNG", "SHD", "BRD", "ROG", "SHM"]
        );
        assert!(item.magic && item.lore && !item.no_drop);
        assert!(item.effects.is_empty());
        assert!(item.unparsed.is_empty(), "{:?}", item.unparsed);
    }

    #[test]
    fn resists_and_haste() {
        let item = parse("Cloak_of_Flames");
        assert_eq!(item.icon, Some(658));
        assert_eq!(item.slots, ["BACK"]);
        assert_eq!(item.ac, Some(10));
        assert_eq!(item.dex, Some(9));
        assert_eq!(item.agi, Some(9));
        assert_eq!(item.hp, Some(50));
        assert_eq!(item.sv_fire, Some(15));
        assert_eq!(item.sv_cold, None);
        assert_eq!(item.haste, Some(36));
        assert_eq!(item.weight, Some(0.1));
        assert_eq!(item.classes, ["ALL"]);
        assert!(item.magic && !item.lore);
        assert!(item.unparsed.is_empty(), "{:?}", item.unparsed);
    }

    #[test]
    fn container() {
        let item = parse("Bag_of_the_Tinkerers");
        assert_eq!(item.icon, Some(557));
        assert_eq!(item.name, "Bag of the Tinkerers");
        assert_eq!(item.weight, Some(1.0));
        assert_eq!(item.weight_reduction, Some(100));
        assert_eq!(item.capacity, Some(10));
        assert_eq!(item.size_capacity.as_deref(), Some("GIANT"));
        assert!(item.slots.is_empty());
        assert!(item.classes.is_empty());
        assert_eq!(item.ac, None);
        assert!(item.unparsed.is_empty(), "{:?}", item.unparsed);
    }

    #[test]
    fn simple_no_stat_item() {
        let item = parse("Bone_Chips");
        assert_eq!(item.icon, Some(804));
        assert_eq!(item.name, "Bone Chips");
        assert_eq!(item.weight, Some(0.1));
        assert_eq!(item.size.as_deref(), Some("SMALL"));
        assert_eq!(item.classes, ["ALL"]);
        assert_eq!(item.races, ["ALL"]);
        assert!(!item.magic && !item.lore && !item.no_drop);
        assert_eq!(item.ac, None);
        assert_eq!(item.hp, None);
        assert!(item.effects.is_empty());
        assert!(item.unparsed.is_empty(), "{:?}", item.unparsed);
    }

    #[test]
    fn focus_effect_param() {
        let item = parse("Azure_Sleeves");
        assert_eq!(item.icon, Some(669));
        assert_eq!(item.slots, ["ARMS"]);
        assert_eq!(item.ac, Some(12));
        assert_eq!(item.focus_effect.as_deref(), Some("Improved Damage I"));
        assert_eq!(
            item.effects,
            [ItemEffect {
                kind: "focus".into(),
                name: "Improved Damage I".into(),
                ..Default::default()
            }]
        );
        assert!(item.unparsed.is_empty(), "{:?}", item.unparsed);
    }

    #[test]
    fn click_effect_with_required_level() {
        let item = parse("Azarack_Skin_Wristwraps");
        assert_eq!(item.icon, Some(637));
        assert_eq!(item.slots, ["WRIST"]);
        assert!(item.no_trade && !item.magic);
        assert_eq!(item.classes, ["BST"]);
        assert_eq!(item.ac, Some(5));
        assert_eq!(item.hp, Some(35));
        assert_eq!(item.endurance, Some(10));
        assert_eq!(item.strength, Some(5));
        assert_eq!(item.sta, Some(5));
        assert_eq!(item.dex, Some(5));
        assert_eq!(item.required_level, Some(46));
        assert_eq!(
            item.effects,
            [ItemEffect {
                kind: "click".into(),
                name: "Whirl Bolt".into(),
                restriction: Some("Must Equip".into()),
                casting_time: Some("1.0 seconds".into()),
                level: Some(46),
                cooldown_seconds: Some(240),
            }]
        );
        assert!(item.unparsed.is_empty(), "{:?}", item.unparsed);
    }

    #[test]
    fn every_fixture_parses() {
        for entry in
            std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/wiki")).unwrap()
        {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let item = parse_item("fallback", &text)
                .unwrap_or_else(|| panic!("no Itempage in {}", path.display()));
            assert_ne!(item.name, "fallback", "{}", path.display());
        }
    }

    #[test]
    fn redirect_page_has_no_item() {
        assert!(parse_item("Fungi Tunic", "#REDIRECT [[Fungus Covered Scale Tunic]]").is_none());
    }

    #[test]
    fn unknown_fields_are_counted_not_fatal() {
        let text = "{{Itempage|itemname = Odd Thing|statsblock = \n\
             MAGIC ITEM<br>\nWobble: 12<br>\nAC: 4<br>\n}}";
        let item = parse_item("Odd Thing", text).unwrap();
        assert_eq!(item.ac, Some(4));
        assert!(item.magic);
        assert_eq!(item.unparsed, ["WOBBLE"]);
    }

    #[test]
    fn template_params_ignore_nested_pipes() {
        let params = template_params(
            "{{Itempage|notes = {{!}} a {{!}} b [[Neriak|Third Gate]]|itemname = X}}",
            "Itempage",
        )
        .unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[1], ("itemname".into(), "X".into()));
    }
}
