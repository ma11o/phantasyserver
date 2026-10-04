//! [pso2_vita_offline] Data-driven drop tables. The stages follow the original's order: the enemy's own table, the
//! common table, the area's table, then meseta. Data lives in `data/drops/` (`PSO2_DROPS_DIR` overrides):
//! `enemies/<internal name>.json`, `common.json`, `areas/<zone name>.json`. Read once on first use, not compiled
//! into `com_data.mp`.
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
    /// bosses drop 4 (8 with the kill points in the original, not done)
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
            common: read_table(&dir.join("common.json")),
            areas: read_dir_tables(&dir.join("areas")),
        };
        log::info!(
            "[pso2-drop] loaded {}: {} enemies, common {}, {} areas",
            dir.display(),
            data.enemies.len(),
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

fn roll_table<R: Rng>(rng: &mut R, t: &Table, out: &mut Vec<Item>) {
    if rng.gen_range(0.0f32..1.0) >= t.rate {
        return;
    }
    let total: f32 = t.entries.iter().map(|e| e.weight.max(0.0)).sum();
    if total > 0.0 {
        for _ in 0..range(rng, t.drop_count).max(t.boss_drops) {
            let mut x = rng.gen_range(0.0f32..1.0) * total;
            let Some(e) = t.entries.iter().find(|e| {
                x -= e.weight.max(0.0);
                x < 0.0
            }) else {
                continue;
            };
            let amount = range(rng, Range { min: e.min, max: e.max });
            if let Some(item) = make_item(&e.item, amount) {
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

/// Stages: enemy table -> common -> area (its `meseta` is the meseta stage). An enemy without a table only gets
/// the common and area stages.
pub fn roll_with<R: Rng>(rng: &mut R, data: &DropData, enemy: &str, area: &str) -> Vec<Item> {
    let mut out = vec![];
    let stages = [data.enemies.get(enemy), data.common.as_ref(), data.areas.get(area)];
    for t in stages.into_iter().flatten() {
        roll_table(rng, t, &mut out);
    }
    out
}

pub fn roll(enemy: &str, area: &str) -> Vec<Item> {
    roll_with(&mut rand::thread_rng(), data(), enemy, area)
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
                Entry { item: "3:1:0".into(), weight: 70.0, min: 1, max: 1 },
                Entry { item: "3:7:0".into(), weight: 30.0, min: 1, max: 1 },
            ],
            ..Default::default()
        };
        let data = DropData { enemies: HashMap::from([("X".into(), t)]), ..Default::default() };
        let mut rng = StdRng::seed_from_u64(1);
        let mut c = [0u32; 2];
        for _ in 0..10_000 {
            for i in roll_with(&mut rng, &data, "X", "none") {
                c[(i.id.id == 7) as usize] += 1;
            }
        }
        assert!((6300..=7700).contains(&c[0]), "{c:?}");
        assert!((2700..=3300).contains(&c[1]), "{c:?}");
    }

    #[test]
    fn stages_and_meseta() {
        let data = DropData {
            common: Some(Table { entries: vec![Entry { item: "3:1:0".into(), weight: 1.0, min: 1, max: 1 }], ..Default::default() }),
            areas: HashMap::from([("a".into(), Table { meseta: Some(Meseta { min: 5, max: 5, rate: 1.0 }), ..Default::default() })]),
            ..Default::default()
        };
        let r = roll_with(&mut StdRng::seed_from_u64(2), &data, "NoTable", "a");
        assert_eq!(r.len(), 2);
        assert_eq!(crate::map::meseta_amount(&r[1]), Some(5));
    }
}
