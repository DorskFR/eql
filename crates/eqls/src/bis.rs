use crate::wiki::ItemStats;

/// Empty tokens = the EQL "Any Slot" sockets, which take any equippable item.
/// `SECONDAY` is a real typo in the scraped data.
pub const SLOTS: &[(&str, &[&str])] = &[
    ("Focus", &[]),
    ("Ear", &["EAR", "EARS"]),
    ("Head", &["HEAD"]),
    ("Face", &["FACE"]),
    ("Neck", &["NECK"]),
    ("Shoulders", &["SHOULDER", "SHOULDERS"]),
    ("Arms", &["ARMS"]),
    ("Back", &["BACK"]),
    ("Wrist", &["WRIST", "WRISTS"]),
    ("Range", &["RANGE"]),
    ("Hands", &["HANDS"]),
    ("Primary", &["PRIMARY"]),
    ("Secondary", &["SECONDARY", "SECONDAY"]),
    ("Fingers", &["FINGER", "FINGERS"]),
    ("Chest", &["CHEST"]),
    ("Legs", &["LEGS"]),
    ("Feet", &["FEET"]),
    ("Waist", &["WAIST"]),
    ("Extra", &[]),
    ("Ammo", &["AMMO"]),
];

/// The scrape tokenises "ALL EXCEPT X Y" as `ALL, EXCEPT, X, Y`.
pub fn usable_by(classes: &[String], loadout: &[String]) -> bool {
    if classes.is_empty() {
        return true;
    }
    let has = |token: &str| classes.iter().any(|c| c.eq_ignore_ascii_case(token));
    if has("NONE") {
        return false;
    }
    let in_loadout = loadout.iter().any(|class| has(class));
    if has("ALL") {
        return !(has("EXCEPT") && in_loadout);
    }
    in_loadout
}

pub fn fits_slot(slots: &[String], tokens: &[&str]) -> bool {
    if tokens.is_empty() {
        return !slots.is_empty();
    }
    slots
        .iter()
        .any(|slot| tokens.iter().any(|token| slot.eq_ignore_ascii_case(token)))
}

/// EQL runs classic-1999 content only. Untagged wiki pages are kept: plenty of
/// genuinely classic items (e.g. Bladestopper) carry no era template.
const EXPANSION_ERAS: &[&str] = &[
    "kunark",
    "chardok",
    "chardok revamp",
    "epics",
    "epicquests",
    "velious",
    "luclin",
    "fearhaterevamp",
];

pub fn in_classic_era(era: Option<&str>) -> bool {
    match era {
        None => true,
        Some(era) => {
            let era = era.trim().to_lowercase();
            !EXPANSION_ERAS.contains(&era.as_str())
        }
    }
}

pub fn level_ok(required: Option<i64>, level: Option<i64>) -> bool {
    match (required, level) {
        (Some(required), Some(level)) => required <= level,
        _ => true,
    }
}

pub fn rank(stats: &ItemStats, weapon_slot: bool) -> (i64, i64) {
    let ratio = match (stats.damage, stats.delay) {
        (Some(damage), Some(delay)) if weapon_slot && delay > 0 => damage * 1000 / delay,
        _ => 0,
    };
    (ratio, score(stats))
}

pub fn score(stats: &ItemStats) -> i64 {
    let v = |value: Option<i64>| value.unwrap_or(0);
    let attributes = v(stats.strength)
        + v(stats.sta)
        + v(stats.agi)
        + v(stats.dex)
        + v(stats.wis)
        + v(stats.intelligence)
        + v(stats.cha);
    let resists = v(stats.sv_fire)
        + v(stats.sv_cold)
        + v(stats.sv_magic)
        + v(stats.sv_disease)
        + v(stats.sv_poison);
    3 * v(stats.ac)
        + v(stats.hp)
        + v(stats.mana)
        + v(stats.endurance)
        + 5 * (v(stats.hp_regen) + v(stats.mana_regen))
        + 2 * attributes
        + resists
        + 8 * v(stats.haste)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn unrestricted_and_all_are_usable() {
        assert!(usable_by(&[], &strings(&["SHD"])));
        assert!(usable_by(&strings(&["ALL"]), &strings(&["SHD"])));
    }

    #[test]
    fn class_list_needs_an_overlap() {
        let classes = strings(&["CLR", "SHM"]);
        assert!(usable_by(&classes, &strings(&["SHD", "SHM", "NEC"])));
        assert!(!usable_by(&classes, &strings(&["SHD", "NEC", "WIZ"])));
    }

    #[test]
    fn all_except_excludes_the_listed_classes() {
        let classes = strings(&["ALL", "EXCEPT", "BRD", "ROG"]);
        assert!(usable_by(&classes, &strings(&["SHD", "NEC"])));
        assert!(!usable_by(&classes, &strings(&["SHD", "ROG"])));
    }

    #[test]
    fn none_is_unusable() {
        assert!(!usable_by(&strings(&["NONE"]), &strings(&["SHD"])));
    }

    #[test]
    fn any_slot_takes_any_equippable_item() {
        assert!(fits_slot(&strings(&["SECONDARY"]), &[]));
        assert!(!fits_slot(&[], &[]));
    }

    #[test]
    fn slot_tokens_match_case_insensitively_with_variants() {
        assert!(fits_slot(
            &strings(&["SECONDAY"]),
            &["SECONDARY", "SECONDAY"]
        ));
        assert!(!fits_slot(&strings(&["HEAD"]), &["SECONDARY", "SECONDAY"]));
    }

    #[test]
    fn expansion_eras_are_out_untagged_and_classic_are_in() {
        assert!(in_classic_era(None));
        assert!(in_classic_era(Some("Classic")));
        assert!(in_classic_era(Some("Sky")));
        assert!(in_classic_era(Some("Unknown")));
        assert!(!in_classic_era(Some("Velious")));
        assert!(!in_classic_era(Some("kunark")));
        assert!(!in_classic_era(Some("Chardok Revamp")));
        assert!(!in_classic_era(Some("EpicQuests")));
    }

    #[test]
    fn level_gate_only_applies_when_both_known() {
        assert!(level_ok(None, Some(10)));
        assert!(level_ok(Some(10), None));
        assert!(level_ok(Some(10), Some(10)));
        assert!(!level_ok(Some(11), Some(10)));
    }

    #[test]
    fn weapon_slots_rank_by_ratio_first() {
        let slow_hard = ItemStats {
            damage: Some(50),
            delay: Some(40),
            ac: Some(30),
            ..Default::default()
        };
        let fast = ItemStats {
            damage: Some(20),
            delay: Some(19),
            ..Default::default()
        };
        assert!(rank(&fast, true) < rank(&slow_hard, true));
        assert!(rank(&fast, false) < rank(&slow_hard, false));
        let dagger = ItemStats {
            damage: Some(10),
            delay: Some(7),
            ..Default::default()
        };
        assert!(rank(&dagger, true) > rank(&slow_hard, true));
    }
}
