//! EQL item upgrade (merge) tiers, per eqlwiki "Item Upgrade System":
//! stats gain a cumulative 10% per tier (rounded down, minimum +1 per tier),
//! weapon damage gains 5% per tier, delay never changes, weight shrinks 10%
//! per tier but never below 0.1. Partial progress toward the next tier also
//! raises stats in game, but the inventory dump only carries the tier, so
//! values here are tier-boundary values.

use serde_json::Value;

const STAT_KEYS: [&str; 18] = [
    "ac",
    "hp",
    "mana",
    "endurance",
    "hp_regen",
    "mana_regen",
    "str",
    "sta",
    "agi",
    "dex",
    "wis",
    "int",
    "cha",
    "sv_fire",
    "sv_cold",
    "sv_magic",
    "sv_disease",
    "sv_poison",
];

fn scale_stat(value: i64, tier: i64) -> i64 {
    (value * (10 + tier) / 10).max(value + tier)
}

fn scale_damage(value: i64, tier: i64) -> i64 {
    value * (20 + tier) / 20
}

fn scale_weight(value: f64, tier: i64) -> f64 {
    let scaled = ((value * (10 - tier) as f64).floor() / 10.0 * 10.0).round() / 10.0;
    scaled.max(0.1)
}

pub fn apply_upgrade(stats: &mut Value, tier: u32) {
    if tier == 0 {
        return;
    }
    let tier = i64::from(tier);
    let Some(map) = stats.as_object_mut() else {
        return;
    };
    for key in STAT_KEYS {
        if let Some(value) = map.get(key).and_then(Value::as_i64) {
            if value > 0 {
                map.insert(key.into(), scale_stat(value, tier).into());
            }
        }
    }
    if let Some(value) = map.get("damage").and_then(Value::as_i64) {
        if value > 0 {
            map.insert("damage".into(), scale_damage(value, tier).into());
        }
    }
    if let Some(value) = map.get("weight").and_then(Value::as_f64) {
        if value > 0.0 {
            map.insert("weight".into(), scale_weight(value, tier).into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ten_percent_per_tier_rounded_down() {
        let mut stats = json!({"ac": 30});
        apply_upgrade(&mut stats, 1);
        assert_eq!(stats["ac"], 33);
        let mut stats = json!({"ac": 42});
        apply_upgrade(&mut stats, 1);
        assert_eq!(stats["ac"], 46, "46.2 rounds down");
    }

    #[test]
    fn minimum_plus_one_per_tier_carries_small_stats() {
        let mut stats = json!({"ac": 1, "mana": 10, "int": 1, "weight": 0.1});
        apply_upgrade(&mut stats, 10);
        assert_eq!(stats["ac"], 11);
        assert_eq!(stats["mana"], 20);
        assert_eq!(stats["int"], 11);
        assert_eq!(stats["weight"], 0.1);
    }

    #[test]
    fn damage_five_percent_delay_untouched() {
        let mut stats = json!({"damage": 13, "delay": 27});
        apply_upgrade(&mut stats, 10);
        assert_eq!(stats["damage"], 19, "13 * 1.5 = 19.5 rounds down");
        assert_eq!(stats["delay"], 27);
    }

    #[test]
    fn weight_shrinks_but_never_below_a_tenth() {
        let mut stats = json!({"weight": 6.0});
        apply_upgrade(&mut stats, 5);
        assert_eq!(stats["weight"], 3.0);
        let mut stats = json!({"weight": 0.5});
        apply_upgrade(&mut stats, 10);
        assert_eq!(stats["weight"], 0.1);
    }

    #[test]
    fn tier_zero_null_and_negative_stats_are_untouched() {
        let mut stats = json!({"ac": 30, "cha": -5, "hp": null});
        apply_upgrade(&mut stats, 0);
        assert_eq!(stats["ac"], 30);
        apply_upgrade(&mut stats, 3);
        assert_eq!(stats["cha"], -5);
        assert_eq!(stats["hp"], Value::Null);
        assert_eq!(stats["ac"], 39);
    }
}
