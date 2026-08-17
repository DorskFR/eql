use crate::wiki::ItemStats;
use eql_core::inventory::InventoryEntry;
use serde::Serialize;
use std::collections::BTreeSet;

pub const CLASSES: [&str; 16] = [
    "WAR", "CLR", "PAL", "RNG", "SHD", "DRU", "MNK", "BRD", "ROG", "SHM", "NEC", "WIZ", "MAG",
    "ENC", "BST", "BER",
];

const LOADOUT_SIZE: usize = 3;
const CONTAINER_PREFIXES: [&str; 3] = ["General", "Bank", "SharedBank"];

/// STR STA AGI DEX WIS INT CHA, from eqlwiki "Statistics".
const RACE_BASE: [(&str, [i64; 7]); 15] = [
    ("Barbarian", [103, 95, 82, 70, 70, 60, 55]),
    ("Dark Elf", [60, 65, 90, 75, 83, 99, 60]),
    ("Dwarf", [90, 90, 70, 90, 83, 60, 45]),
    ("Erudite", [60, 70, 70, 70, 83, 107, 70]),
    ("Froglok", [70, 80, 100, 100, 75, 75, 50]),
    ("Gnome", [60, 70, 85, 85, 67, 98, 60]),
    ("Half-Elf", [70, 70, 90, 85, 60, 75, 75]),
    ("Halfling", [70, 75, 95, 90, 80, 67, 50]),
    ("High Elf", [55, 65, 85, 70, 95, 92, 80]),
    ("Human", [75, 75, 75, 75, 75, 75, 75]),
    ("Iksar", [70, 70, 90, 85, 80, 75, 55]),
    ("Kerra", [90, 75, 90, 70, 70, 65, 65]),
    ("Ogre", [130, 127, 70, 70, 67, 60, 37]),
    ("Troll", [108, 114, 83, 75, 60, 52, 40]),
    ("Wood Elf", [65, 65, 95, 80, 80, 75, 75]),
];

const CLASS_MOD: [(&str, [i64; 7]); 16] = [
    ("BRD", [5, 0, 0, 10, 0, 0, 15]),
    ("BST", [0, 10, 5, 0, 10, 0, 5]),
    ("BER", [15, 5, 0, 10, 0, 0, 0]),
    ("CLR", [5, 10, 0, 0, 15, 0, 0]),
    ("DRU", [0, 15, 0, 0, 15, 0, 0]),
    ("ENC", [0, 0, 0, 0, 0, 15, 15]),
    ("MAG", [0, 15, 0, 0, 0, 15, 0]),
    ("MNK", [5, 5, 10, 10, 0, 0, 0]),
    ("NEC", [0, 0, 0, 15, 0, 15, 0]),
    ("PAL", [10, 5, 0, 0, 5, 0, 10]),
    ("RNG", [5, 10, 10, 0, 5, 0, 0]),
    ("ROG", [0, 0, 15, 15, 0, 0, 0]),
    ("SHD", [10, 5, 0, 0, 0, 10, 5]),
    ("SHM", [0, 10, 0, 0, 15, 0, 5]),
    ("WAR", [10, 15, 5, 0, 0, 0, 0]),
    ("WIZ", [0, 15, 0, 0, 0, 15, 0]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BaseAttributes {
    #[serde(rename = "str")]
    pub strength: i64,
    pub sta: i64,
    pub agi: i64,
    pub dex: i64,
    pub wis: i64,
    #[serde(rename = "int")]
    pub intelligence: i64,
    pub cha: i64,
}

/// Race base plus the primary (first) class's creation modifier. Points a
/// player allocated at creation are invisible to the dumps, so in-game values
/// can sit a little above these.
pub fn base_attributes(race: &str, primary_class: Option<&str>) -> Option<BaseAttributes> {
    let mut values = RACE_BASE
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(race))
        .map(|(_, values)| *values)?;
    if let Some(class) = primary_class {
        if let Some((_, modifier)) = CLASS_MOD
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(class))
        {
            for (value, extra) in values.iter_mut().zip(modifier) {
                *value += extra;
            }
        }
    }
    let [strength, sta, agi, dex, wis, intelligence, cha] = values;
    Some(BaseAttributes {
        strength,
        sta,
        agi,
        dex,
        wis,
        intelligence,
        cha,
    })
}

pub fn is_equipped_location(location: &str) -> bool {
    !location.contains("-Slot")
        && !CONTAINER_PREFIXES
            .iter()
            .any(|prefix| location.starts_with(prefix))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct WeaponSummary {
    pub name: String,
    pub item_type: Option<String>,
    pub damage: Option<i64>,
    pub delay: Option<i64>,
    pub ratio: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ItemClasses {
    pub location: String,
    pub name: String,
    pub classes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GearStats {
    pub ac: i64,
    pub hp: i64,
    pub mana: i64,
    pub endurance: i64,
    pub hp_regen: i64,
    pub mana_regen: i64,
    #[serde(rename = "str")]
    pub strength: i64,
    pub sta: i64,
    pub agi: i64,
    pub dex: i64,
    pub wis: i64,
    #[serde(rename = "int")]
    pub intelligence: i64,
    pub cha: i64,
    pub sv_fire: i64,
    pub sv_cold: i64,
    pub sv_magic: i64,
    pub sv_disease: i64,
    pub sv_poison: i64,
    pub haste: i64,
    pub weight: f64,
    pub equipped_count: usize,
    pub known_items: usize,
    pub unknown_items: usize,
    pub primary: Option<WeaponSummary>,
    pub secondary: Option<WeaponSummary>,
    pub usable_by: Vec<String>,
    pub no_single_class_can_use_all: bool,
    pub min_classes_needed: Option<usize>,
    pub item_classes: Vec<ItemClasses>,
}

/// Item-derived only: race/class/level base stats are unknown, so a base-stat
/// table would be summed on top of this.
pub fn derive_gear_stats(entries: &[(InventoryEntry, Option<ItemStats>)]) -> GearStats {
    let mut gear = GearStats::default();
    let mut restrictions: Vec<Vec<String>> = Vec::new();

    for (entry, stats) in entries {
        if entry.is_empty_slot() || !is_equipped_location(&entry.location) {
            continue;
        }
        gear.equipped_count += 1;
        let Some(stats) = stats else {
            gear.unknown_items += 1;
            continue;
        };
        gear.known_items += 1;

        let add = |target: &mut i64, value: Option<i64>| *target += value.unwrap_or(0);
        add(&mut gear.ac, stats.ac);
        add(&mut gear.hp, stats.hp);
        add(&mut gear.mana, stats.mana);
        add(&mut gear.endurance, stats.endurance);
        add(&mut gear.hp_regen, stats.hp_regen);
        add(&mut gear.mana_regen, stats.mana_regen);
        add(&mut gear.strength, stats.strength);
        add(&mut gear.sta, stats.sta);
        add(&mut gear.agi, stats.agi);
        add(&mut gear.dex, stats.dex);
        add(&mut gear.wis, stats.wis);
        add(&mut gear.intelligence, stats.intelligence);
        add(&mut gear.cha, stats.cha);
        add(&mut gear.sv_fire, stats.sv_fire);
        add(&mut gear.sv_cold, stats.sv_cold);
        add(&mut gear.sv_magic, stats.sv_magic);
        add(&mut gear.sv_disease, stats.sv_disease);
        add(&mut gear.sv_poison, stats.sv_poison);
        gear.weight += stats.weight.unwrap_or(0.0);
        gear.haste = gear.haste.max(stats.haste.unwrap_or(0));

        if entry.location.eq_ignore_ascii_case("primary") {
            gear.primary = Some(weapon(entry, stats));
        } else if entry.location.eq_ignore_ascii_case("secondary") {
            gear.secondary = Some(weapon(entry, stats));
        }

        let classes = class_union(stats);
        gear.item_classes.push(ItemClasses {
            location: entry.location.clone(),
            name: entry.name.clone(),
            classes: classes.clone(),
        });
        if !classes.is_empty() {
            restrictions.push(classes);
        }
    }

    gear.weight = (gear.weight * 100.0).round() / 100.0;
    gear.usable_by = intersect(&restrictions);
    gear.no_single_class_can_use_all = gear.usable_by.is_empty();
    gear.min_classes_needed = min_classes_needed(&restrictions);
    gear
}

fn weapon(entry: &InventoryEntry, stats: &ItemStats) -> WeaponSummary {
    let ratio = match (stats.damage, stats.delay) {
        (Some(damage), Some(delay)) if delay > 0 => {
            Some((damage as f64 / delay as f64 * 100.0).round() / 100.0)
        }
        _ => None,
    };
    WeaponSummary {
        name: entry.name.clone(),
        item_type: stats.item_type.clone(),
        damage: stats.damage,
        delay: stats.delay,
        ratio,
    }
}

/// `ALL` and an absent class list both mean "no restriction", which we model as
/// an empty union so it never constrains the intersection or the cover.
fn class_union(stats: &ItemStats) -> Vec<String> {
    if stats
        .classes
        .iter()
        .any(|class| class.eq_ignore_ascii_case("ALL"))
    {
        return Vec::new();
    }
    let mut union: Vec<String> = stats
        .classes
        .iter()
        .map(|class| class.to_ascii_uppercase())
        .collect();
    union.sort();
    union.dedup();
    union
}

fn intersect(restrictions: &[Vec<String>]) -> Vec<String> {
    let Some((first, rest)) = restrictions.split_first() else {
        return CLASSES.iter().map(|c| (*c).to_string()).collect();
    };
    let mut usable: BTreeSet<&str> = first.iter().map(String::as_str).collect();
    for classes in rest {
        usable.retain(|class| classes.iter().any(|c| c == class));
    }
    usable.into_iter().map(str::to_string).collect()
}

/// Brute force over C(16, <=3); the loadout holds at most three classes so a
/// larger cover is reported as unreachable.
fn min_classes_needed(restrictions: &[Vec<String>]) -> Option<usize> {
    let mut candidates: BTreeSet<&str> = CLASSES.into_iter().collect();
    candidates.extend(restrictions.iter().flatten().map(String::as_str));
    let candidates: Vec<&str> = candidates.into_iter().collect();

    let covers = |picked: &[&str]| {
        restrictions
            .iter()
            .all(|classes| classes.iter().any(|c| picked.contains(&c.as_str())))
    };
    if covers(&[]) {
        return Some(0);
    }
    for &a in &candidates {
        if covers(&[a]) {
            return Some(1);
        }
    }
    for a in 0..candidates.len() {
        for b in a + 1..candidates.len() {
            if covers(&[candidates[a], candidates[b]]) {
                return Some(2);
            }
        }
    }
    for a in 0..candidates.len() {
        for b in a + 1..candidates.len() {
            for c in b + 1..candidates.len() {
                if covers(&[candidates[a], candidates[b], candidates[c]]) {
                    return Some(LOADOUT_SIZE);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(location: &str, name: &str) -> InventoryEntry {
        InventoryEntry {
            location: location.into(),
            name: name.into(),
            id: 1,
            count: 1,
            slots: 0,
        }
    }

    fn item(classes: &[&str]) -> ItemStats {
        ItemStats {
            classes: classes.iter().map(|c| (*c).to_string()).collect(),
            ..Default::default()
        }
    }

    fn known(location: &str, name: &str, stats: ItemStats) -> (InventoryEntry, Option<ItemStats>) {
        (entry(location, name), Some(stats))
    }

    #[test]
    fn sums_stats_over_equipped_items() {
        let gear = derive_gear_stats(&[
            known(
                "Chest",
                "Rubicite Breastplate",
                ItemStats {
                    ac: Some(19),
                    hp: Some(50),
                    strength: Some(5),
                    sv_fire: Some(10),
                    weight: Some(6.0),
                    ..Default::default()
                },
            ),
            known(
                "Back",
                "Cloak of Flames",
                ItemStats {
                    ac: Some(10),
                    hp: Some(50),
                    mana: Some(25),
                    endurance: Some(7),
                    cha: Some(3),
                    sv_fire: Some(15),
                    weight: Some(0.1),
                    ..Default::default()
                },
            ),
        ]);
        assert_eq!(gear.ac, 29);
        assert_eq!(gear.hp, 100);
        assert_eq!(gear.mana, 25);
        assert_eq!(gear.endurance, 7);
        assert_eq!(gear.strength, 5);
        assert_eq!(gear.cha, 3);
        assert_eq!(gear.sv_fire, 25);
        assert_eq!(gear.sv_cold, 0);
        assert_eq!(gear.weight, 6.1);
        assert_eq!((gear.known_items, gear.unknown_items), (2, 0));
        assert_eq!(gear.equipped_count, 2);
    }

    #[test]
    fn haste_takes_the_maximum_not_the_sum() {
        let gear = derive_gear_stats(&[
            known(
                "Back",
                "Cloak of Flames",
                ItemStats {
                    haste: Some(36),
                    ..Default::default()
                },
            ),
            known(
                "Wrist1",
                "Journeyman's Boots",
                ItemStats {
                    haste: Some(21),
                    ..Default::default()
                },
            ),
        ]);
        assert_eq!(gear.haste, 36);
    }

    #[test]
    fn weapons_carry_damage_delay_and_ratio() {
        let gear = derive_gear_stats(&[
            known(
                "Primary",
                "Spirit Reaver",
                ItemStats {
                    item_type: Some("1H Slashing".into()),
                    damage: Some(13),
                    delay: Some(27),
                    ..Default::default()
                },
            ),
            known(
                "Secondary",
                "Shield",
                ItemStats {
                    ac: Some(9),
                    ..Default::default()
                },
            ),
        ]);
        let primary = gear.primary.expect("primary");
        assert_eq!(primary.damage, Some(13));
        assert_eq!(primary.delay, Some(27));
        assert_eq!(primary.ratio, Some(0.48));
        assert_eq!(primary.item_type.as_deref(), Some("1H Slashing"));
        let secondary = gear.secondary.expect("secondary");
        assert_eq!(secondary.ratio, None);
    }

    #[test]
    fn all_and_empty_class_lists_do_not_constrain_usable_by() {
        let gear = derive_gear_stats(&[
            known("Charm", "Bone Chips", item(&["ALL"])),
            known("Neck", "Odd Trinket", item(&[])),
        ]);
        assert_eq!(gear.usable_by.len(), CLASSES.len());
        assert!(!gear.no_single_class_can_use_all);
        assert_eq!(gear.min_classes_needed, Some(0));
        assert_eq!(gear.item_classes.len(), 2);
        assert!(gear.item_classes.iter().all(|i| i.classes.is_empty()));
    }

    #[test]
    fn intersection_narrows_to_a_single_class() {
        let gear = derive_gear_stats(&[
            known("Chest", "Plate", item(&["WAR", "CLR", "PAL"])),
            known("Legs", "Greaves", item(&["PAL", "SHD"])),
            known("Head", "Helm", item(&["ALL"])),
        ]);
        assert_eq!(gear.usable_by, ["PAL"]);
        assert!(!gear.no_single_class_can_use_all);
        assert_eq!(gear.min_classes_needed, Some(1));
    }

    #[test]
    fn three_disjoint_single_class_items_force_a_trio() {
        let gear = derive_gear_stats(&[
            known("Primary", "Warrior Blade", item(&["WAR"])),
            known("Secondary", "Cleric Shield", item(&["CLR"])),
            known("Head", "Wizard Cap", item(&["WIZ"])),
        ]);
        assert!(gear.usable_by.is_empty());
        assert!(gear.no_single_class_can_use_all);
        assert_eq!(gear.min_classes_needed, Some(3));
    }

    #[test]
    fn four_disjoint_single_class_items_exceed_a_loadout() {
        let gear = derive_gear_stats(&[
            known("Primary", "A", item(&["WAR"])),
            known("Secondary", "B", item(&["CLR"])),
            known("Head", "C", item(&["WIZ"])),
            known("Feet", "D", item(&["ROG"])),
        ]);
        assert_eq!(gear.min_classes_needed, None);
    }

    #[test]
    fn unknown_items_contribute_nothing_but_still_count() {
        let gear = derive_gear_stats(&[
            (entry("Head", "Mystery Hat"), None),
            known(
                "Chest",
                "Plate",
                ItemStats {
                    ac: Some(20),
                    ..Default::default()
                },
            ),
        ]);
        assert_eq!(gear.ac, 20);
        assert_eq!(gear.equipped_count, 2);
        assert_eq!((gear.known_items, gear.unknown_items), (1, 1));
    }

    #[test]
    fn numbered_worn_slots_are_equipped_containers_are_not() {
        for location in [
            "Ear", "Ear1", "Ear2", "Wrist1", "Wrist2", "Fingers2", "Ammo",
        ] {
            assert!(is_equipped_location(location), "{location}");
        }
        for location in [
            "General1",
            "General1-Slot3",
            "Bank2",
            "Bank2-Slot1",
            "SharedBank1",
        ] {
            assert!(!is_equipped_location(location), "{location}");
        }
    }

    #[test]
    fn only_equipped_and_filled_slots_are_summed() {
        let stats = ItemStats {
            ac: Some(5),
            weight: Some(1.0),
            ..Default::default()
        };
        let gear = derive_gear_stats(&[
            known("Ear1", "Earring", stats.clone()),
            known("Wrist2", "Bracer", stats.clone()),
            known("General1-Slot3", "Bagged Plate", stats.clone()),
            known("Bank2", "Banked Plate", stats.clone()),
            known("General1", "Backpack", stats.clone()),
            (entry("Charm", "Empty"), Some(stats)),
        ]);
        assert_eq!(gear.ac, 10);
        assert_eq!(gear.weight, 2.0);
        assert_eq!(gear.equipped_count, 2);
    }

    #[test]
    fn base_attributes_add_the_primary_class_modifier() {
        let base = base_attributes("Dark Elf", Some("SHD")).unwrap();
        assert_eq!(
            base,
            BaseAttributes {
                strength: 70,
                sta: 70,
                agi: 90,
                dex: 75,
                wis: 83,
                intelligence: 109,
                cha: 65,
            }
        );
        let plain = base_attributes("dark elf", None).unwrap();
        assert_eq!(plain.strength, 60);
        assert_eq!(plain.intelligence, 99);
        assert!(base_attributes("Vulcan", Some("SHD")).is_none());
    }

    #[test]
    fn empty_inventory_is_all_zeroes() {
        let gear = derive_gear_stats(&[]);
        assert_eq!(
            gear,
            GearStats {
                usable_by: CLASSES.iter().map(|c| (*c).to_string()).collect(),
                min_classes_needed: Some(0),
                ..Default::default()
            }
        );
    }
}
