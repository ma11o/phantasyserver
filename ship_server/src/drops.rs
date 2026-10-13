//! [pso2_vita_offline] Data-driven drop tables. The stages follow the original's order: the enemy's own table (the
//! rare stage), the normal table (star 1-6 weapons by enemy level, plus recovery items), the common table, the
//! area's table, then meseta. Data lives in `data/drops/` (`PSO2_DROPS_DIR` overrides):
//! `enemies/<internal name>.json`, `normal.json`, `common.json`, `areas/<zone name>.json`. Read once on first use,
//! not compiled into `com_data.mp`.
use crate::map::meseta;
use pso2packetlib::protocol::items::{ConsumableItem, Item, ItemId, ItemType};
use rand::Rng;
use serde::Deserialize;
use std::{collections::HashMap, path::Path, sync::OnceLock};

#[derive(Debug, Deserialize, Clone, Copy, Default)]
#[serde(default)]
pub struct Range {
    pub min: u32,
    pub max: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Entry {
    /// "type:id:subid"
    pub item: String,
    pub weight: f32,
    pub min: u32,
    pub max: u32,
    /// enemy level band this entry can drop in (absent = always a candidate)
    #[serde(default)]
    pub level_min: Option<u32>,
    #[serde(default)]
    pub level_max: Option<u32>,
}

#[derive(Debug, Deserialize, Clone, Copy, Default)]
#[serde(default)]
pub struct Meseta {
    pub min: u32,
    pub max: u32,
    pub rate: f32,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct Table {
    /// chance that this stage drops anything
    pub rate: f32,
    pub drop_count: Range,
    /// rare-rate addition (class rare / boost): present but not used yet
    pub rare_rate: f32,
    /// enemy tables only: a boss draws this many items (8 with the kill points in the original, not done) from the
    /// normal stage, which then ignores its own rate and count
    pub boss_drops: u32,
    pub meseta: Option<Meseta>,
    pub source: String,
    pub entries: Vec<Entry>,
}

impl Default for Table {
    fn default() -> Self {
        Self {
            rate: 1.0,
            drop_count: Range { min: 1, max: 1 },
            rare_rate: 0.0,
            boss_drops: 0,
            meseta: None,
            source: String::new(),
            entries: vec![],
        }
    }
}

#[derive(Debug, Default)]
pub struct DropData {
    pub enemies: HashMap<String, Table>,
    pub normal: Option<Table>,
    pub common: Option<Table>,
    pub areas: HashMap<String, Table>,
}

fn read_dir_tables(dir: &Path) -> HashMap<String, Table> {
    let mut map = HashMap::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return map;
    };
    for f in rd.flatten() {
        let p = f.path();
        if p.extension().is_some_and(|e| e == "json") {
            match read_table(&p) {
                Some(t) => {
                    map.insert(p.file_stem().unwrap().to_string_lossy().into_owned(), t);
                }
                None => log::warn!("[pso2-drop] cannot read {}", p.display()),
            }
        }
    }
    map
}

fn read_table(p: &Path) -> Option<Table> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

impl DropData {
    pub fn load(dir: &Path) -> Self {
        let data = Self {
            enemies: read_dir_tables(&dir.join("enemies")),
            normal: read_table(&dir.join("normal.json")),
            common: read_table(&dir.join("common.json")),
            areas: read_dir_tables(&dir.join("areas")),
        };
        log::info!(
            "[pso2-drop] loaded {}: {} enemies, normal {}, common {}, {} areas",
            dir.display(),
            data.enemies.len(),
            data.normal.is_some(),
            data.common.is_some(),
            data.areas.len()
        );
        data
    }
}

pub fn data() -> &'static DropData {
    static DATA: OnceLock<DropData> = OnceLock::new();
    DATA.get_or_init(|| {
        let dir = std::env::var_os("PSO2_DROPS_DIR").unwrap_or_else(|| "data/drops".into());
        DropData::load(Path::new(&dir))
    })
}

/// "type:id:subid" with an amount (only consumables, type 3, keep it)
fn make_item(spec: &str, amount: u32) -> Option<Item> {
    let n: Vec<u16> = spec.split(':').map(|s| s.parse().ok()).collect::<Option<_>>()?;
    let [t, id, sub] = n[..] else { return None };
    Some(Item {
        uuid: 0,
        id: ItemId {
            item_type: t,
            id,
            subid: sub,
            ..Default::default()
        },
        data: if t == 3 {
            ItemType::Consumable(ConsumableItem {
                amount: amount.max(1) as u16,
                ..Default::default()
            })
        } else if t == 1 {
            ItemType::Weapon(Default::default())
        } else {
            Default::default()
        },
    })
}

fn range<R: Rng>(rng: &mut R, r: Range) -> u32 {
    if r.max <= r.min {
        r.min
    } else {
        rng.gen_range(r.min..=r.max)
    }
}

fn in_band(e: &Entry, level: u32) -> bool {
    e.level_min.map_or(true, |m| level >= m) && e.level_max.map_or(true, |m| level <= m)
}

fn roll_table<R: Rng>(rng: &mut R, t: &Table, level: u32, out: &mut Vec<Item>) {
    roll_stage(rng, t, level, t.rate, None, out)
}

/// One stage: `rate` is the chance it drops anything, `count` overrides the table's `drop_count`
fn roll_stage<R: Rng>(rng: &mut R, t: &Table, level: u32, rate: f32, count: Option<u32>, out: &mut Vec<Item>) {
    if rng.gen_range(0.0f32..1.0) >= rate {
        return;
    }
    // entries outside the enemy's level band are not candidates; an empty result drops nothing from this stage
    let entries: Vec<&Entry> = t.entries.iter().filter(|e| in_band(e, level)).collect();
    let total: f32 = entries.iter().map(|e| e.weight.max(0.0)).sum();
    if total > 0.0 {
        for _ in 0..count.unwrap_or_else(|| range(rng, t.drop_count)) {
            let mut x = rng.gen_range(0.0f32..1.0) * total;
            let Some(e) = entries.iter().find(|e| {
                x -= e.weight.max(0.0);
                x < 0.0
            }) else {
                continue;
            };
            let amount = range(rng, Range { min: e.min, max: e.max });
            if let Some(mut item) = make_item(&e.item, amount) {
                if let ItemType::Weapon(w) = &mut item.data {
                    w.affixes = crate::affixes::data().roll(rng, level);
                    log::info!("[pso2-affix] dropped {} with [{}]", e.item, crate::affixes::describe(&w.affixes));
                }
                out.push(item);
            }
        }
    }
    if let Some(m) = t.meseta {
        if rng.gen_range(0.0f32..1.0) < m.rate {
            let amount = range(rng, Range { min: m.min, max: m.max });
            if amount > 0 {
                out.push(meseta(amount));
            }
        }
    }
}

/// Stages: enemy table (rare) -> normal -> common -> area (its `meseta` is the meseta stage). An enemy without a
/// table only gets the normal, common and area stages. A boss (`boss_drops` in its table) draws that many items from
/// the normal stage every time.
pub fn roll_with<R: Rng>(rng: &mut R, data: &DropData, enemy: &str, area: &str, level: u32) -> Vec<Item> {
    let mut out = vec![];
    let own = data.enemies.get(enemy);
    if let Some(t) = own {
        roll_table(rng, t, level, &mut out);
    }
    if let Some(t) = &data.normal {
        match own.map(|t| t.boss_drops).filter(|&n| n > 0) {
            Some(n) => roll_stage(rng, t, level, 1.0, Some(n), &mut out),
            None => roll_table(rng, t, level, &mut out),
        }
    }
    for t in [data.common.as_ref(), data.areas.get(area)].into_iter().flatten() {
        roll_table(rng, t, level, &mut out);
    }
    out
}

pub fn roll(enemy: &str, area: &str, level: u32) -> Vec<Item> {
    roll_with(&mut rand::thread_rng(), data(), enemy, area, level)
}

/// Item description for logs and the debug command
pub fn describe(item: &Item) -> String {
    match crate::map::meseta_amount(item) {
        Some(n) => format!("meseta:{n}"),
        None => match &item.data {
            ItemType::Consumable(c) => format!("{}:{}:{}x{}", item.id.item_type, item.id.id, item.id.subid, c.amount),
            _ => format!("{}:{}:{}", item.id.item_type, item.id.id, item.id.subid),
        },
    }
}

/// Drop model by item type (`ob_9900_0002` has the sword shape, the rest use the plain one)
pub fn model(item: &Item) -> &'static str {
    match item.id.item_type {
        1 => "ob_9900_0002",
        _ => crate::map::DROP_MODEL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, SeedableRng};

    #[test]
    fn weights_match_expectation() {
        let t = Table {
            entries: vec![
                Entry { item: "3:1:0".into(), weight: 70.0, min: 1, max: 1, level_min: None, level_max: None },
                Entry { item: "3:7:0".into(), weight: 30.0, min: 1, max: 1, level_min: None, level_max: None },
            ],
            ..Default::default()
        };
        let data = DropData { enemies: HashMap::from([("X".into(), t)]), ..Default::default() };
        let mut rng = StdRng::seed_from_u64(1);
        let mut c = [0u32; 2];
        for _ in 0..10_000 {
            for i in roll_with(&mut rng, &data, "X", "none", 5) {
                c[(i.id.id == 7) as usize] += 1;
            }
        }
        assert!((6300..=7700).contains(&c[0]), "{c:?}");
        assert!((2700..=3300).contains(&c[1]), "{c:?}");
    }

    #[test]
    fn stages_and_meseta() {
        let data = DropData {
            common: Some(Table { entries: vec![Entry { item: "3:1:0".into(), weight: 1.0, min: 1, max: 1, level_min: None, level_max: None }], ..Default::default() }),
            areas: HashMap::from([("a".into(), Table { meseta: Some(Meseta { min: 5, max: 5, rate: 1.0 }), ..Default::default() })]),
            ..Default::default()
        };
        let r = roll_with(&mut StdRng::seed_from_u64(2), &data, "NoTable", "a", 5);
        assert_eq!(r.len(), 2);
        assert_eq!(crate::map::meseta_amount(&r[1]), Some(5));
    }

    /// The feel target (T108): one lap of Forest N (Lv10) = 35 common enemies + 1 boss.
    /// Star 1-3 weapons 2-4 per lap, rare (star 7+) about 0.1 per lap, the boss always drops 4 normal items.
    #[test]
    fn one_lap_feel() {
        let e = |item: &str, weight: f32, level_min: Option<u32>| Entry {
            item: item.into(), weight, min: 1, max: 1, level_min, level_max: None,
        };
        // normal: star 1 (id 1) / star 2 (id 2, from Lv3) weapons in the generator's proportions, then recovery
        let normal = Table {
            rate: 0.10,
            entries: vec![
                e("1:1:0", 50.0, Some(1)),
                e("1:2:1", 12.6, Some(3)),
                e("3:1:0", 60.0, None),
                e("3:1:1", 20.0, Some(10)),
                e("3:7:0", 20.0, None),
            ],
            ..Default::default()
        };
        // rare: a common enemy 1% (a third of them have a Lv10 candidate), the boss 10% x 1 and boss_drops 4
        let mob = Table { rate: 0.01, entries: vec![e("1:7:24", 4.0, Some(1))], ..Default::default() };
        let boss = Table { rate: 0.10, boss_drops: 4, entries: vec![e("1:17:33", 2.0, Some(1))], ..Default::default() };
        let data = DropData {
            enemies: HashMap::from([("Mob".into(), mob), ("Boss".into(), boss)]),
            normal: Some(normal),
            ..Default::default()
        };
        let mut rng = StdRng::seed_from_u64(7);
        let (mut low, mut rare, mut boss_items) = (0u32, 0u32, 0u32);
        let laps = 1000;
        for _ in 0..laps {
            for n in 0..35 {
                let name = if n % 3 == 0 { "Mob" } else { "NoTable" };
                for i in roll_with(&mut rng, &data, name, "none", 10) {
                    match i.id.id {
                        1 | 2 if i.id.item_type == 1 => low += 1,
                        _ if i.id.item_type == 1 => rare += 1,
                        _ => {}
                    }
                }
            }
            let r = roll_with(&mut rng, &data, "Boss", "none", 10);
            boss_items += r.iter().filter(|i| i.id.item_type != 1 || matches!(i.id.id, 1 | 2)).count() as u32;
            for i in &r {
                if i.id.item_type == 1 && !matches!(i.id.id, 1 | 2) {
                    rare += 1;
                } else if i.id.item_type == 1 {
                    low += 1;
                }
            }
        }
        let per_lap = |n: u32| n as f32 / laps as f32;
        // the boss's 4 normal picks are counted in `low` too (weapons among them); here 2-4 weapons per lap
        assert!((2.0..=4.0).contains(&per_lap(low)), "star 1-3 weapons per lap: {}", per_lap(low));
        assert!((0.03..=0.30).contains(&per_lap(rare)), "rare per lap: {}", per_lap(rare));
        assert!(per_lap(boss_items) >= 3.5, "boss normal items per lap: {}", per_lap(boss_items));
    }

    #[test]
    fn level_band_filters_entries() {
        let t = Table {
            entries: vec![
                Entry { item: "1:3:33".into(), weight: 1.0, min: 1, max: 1, level_min: Some(20), level_max: None },
                Entry { item: "3:5:322".into(), weight: 1.0, min: 1, max: 1, level_min: None, level_max: None },
            ],
            ..Default::default()
        };
        let data = DropData { enemies: HashMap::from([("X".into(), t)]), ..Default::default() };
        let mut rng = StdRng::seed_from_u64(3);
        for _ in 0..200 {
            let r = roll_with(&mut rng, &data, "X", "none", 5);
            assert!(r.iter().all(|i| i.id.item_type != 1), "high band dropped at Lv5");
            assert_eq!(r.len(), 1);
        }
        let hit = (0..200).any(|_| roll_with(&mut rng, &data, "X", "none", 25).iter().any(|i| i.id.item_type == 1));
        assert!(hit);
    }
}
