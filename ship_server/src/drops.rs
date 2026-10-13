//! [pso2_vita_offline] Data-driven drop tables. One kill runs a priority list a few times and the first stage that
//! succeeds ends a run, in the original's order (swiki "ドロップ条件まとめ", see `run_once`): the enemy's own table (the
//! rare stage) at the current and one lower enemy-level band, the boss's class-matching rare, the normal table (star
//! 1-6 weapons by enemy level and class), the common table, the area's table, then meseta. Data lives in `data/drops/` (`PSO2_DROPS_DIR` overrides):
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
    /// star rating of a weapon entry (the star 1-3 class filter, the class-rare stage's star 7+)
    #[serde(default)]
    pub rarity: Option<u32>,
}

#[derive(Debug, Deserialize, Clone, Copy, Default)]
#[serde(default)]
pub struct Meseta {
    pub min: u32,
    pub max: u32,
    pub rate: f32,
    /// amount x (1 + level_mul x enemy level)
    pub level_mul: f32,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct Table {
    /// chance that this stage drops anything
    pub rate: f32,
    pub drop_count: Range,
    /// rare-rate addition (class rare / boost): present but not used yet
    pub rare_rate: f32,
    /// enemy tables only: a boss runs the priority list this many times (more with the quest's kills, up to +4)
    pub boss_drops: u32,
    /// enemy tables only: priority-list runs per kill for a leader / large enemy (0 and 1 = one)
    pub draws: u32,
    /// rare-rate boost in percent this table has by itself (added to the effects' boost)
    pub boost: f32,
    /// normal table only: weapon category (the item id's second field) -> bit set of the classes (`Class` order) that
    /// can equip it
    pub weapon_classes: HashMap<String, u32>,
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
            draws: 1,
            boost: 0.0,
            weapon_classes: HashMap::new(),
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

/// Enemy-level tiers of the difficulties (N / H / VH / SH / XH), from `fields_difficulty.csv` (estimated: the lowest
/// enemy level of each difficulty over all fields; an N field tops out below 31).
const TIER_FLOORS: [u32; 4] = [31, 41, 62, 79];
/// The enemy level the "one band lower" stage draws at: the top of the next lower difficulty's level range (None in
/// N, which has no lower band).
pub fn lower_level(level: u32) -> Option<u32> {
    TIER_FLOORS.iter().rev().find(|&&f| level > f).copied()
}

/// Rate multiplier of the one-band-lower enemy stage against the current band's (tuned: a second chance, not a second
/// full rate)
const LOWER_RATE_MUL: f32 = 0.5;
/// Rate multiplier of the boss's class-rare stage against the enemy table's rate (tuned)
const CLASS_RARE_RATE_MUL: f32 = 0.5;
/// Kills in the quest per extra boss drop (estimated: 35 kills in a Forest N lap reach 8 drops, 4 + 4)
pub const KILLS_PER_EXTRA: u32 = 8;
/// Extra boss drops at most (4 -> 8)
const MAX_EXTRA_DRAWS: u32 = 4;

/// Everything one kill's roll depends on
#[derive(Debug, Clone, Copy)]
pub struct Ctx<'a> {
    pub enemy: &'a str,
    pub area: &'a str,
    pub level: u32,
    /// the last hitter's main class (`Class` as u8), None = unknown (no class filter)
    pub class: Option<u8>,
    /// kills in the quest before this one
    pub kills: u32,
    /// rare-rate boost in percent from the zone's effects (PSE)
    pub boost: f32,
}

impl<'a> Ctx<'a> {
    pub fn new(enemy: &'a str, area: &'a str, level: u32) -> Self {
        Self { enemy, area, level, class: None, kills: 0, boost: 0.0 }
    }
}

fn weapon_type(spec: &str) -> Option<&str> {
    let mut it = spec.split(':');
    (it.next()? == "1").then(|| it.next()).flatten()
}

/// Can `class` use the weapon category of `spec`? Unknown category or class: yes.
fn class_matches(weapon_classes: &HashMap<String, u32>, class: Option<u8>, spec: &str) -> bool {
    match (class, weapon_type(spec).and_then(|t| weapon_classes.get(t))) {
        (Some(c), Some(mask)) => mask >> c & 1 == 1,
        _ => true,
    }
}

/// Candidates of the boss's class-rare stage: a star 7+ weapon of a category the main class can use
fn class_rare_ok(wc: &HashMap<String, u32>, class: Option<u8>, e: &Entry) -> bool {
    weapon_type(&e.item).is_some() && e.rarity.map_or(false, |r| r >= 7) && class_matches(wc, class, &e.item)
}

/// One stage: succeeds (and pushes `count` items) when the rate roll passes and an entry in the level band passes
/// `ok`. The caller stops the priority list on success.
fn stage<R: Rng>(
    rng: &mut R,
    t: &Table,
    level: u32,
    rate: f32,
    ok: impl Fn(&Entry) -> bool,
    out: &mut Vec<Item>,
) -> bool {
    let entries: Vec<&Entry> = t.entries.iter().filter(|e| in_band(e, level) && ok(e)).collect();
    let total: f32 = entries.iter().map(|e| e.weight.max(0.0)).sum();
    if total <= 0.0 || rng.gen_range(0.0f32..1.0) >= rate {
        return false;
    }
    let before = out.len();
    for _ in 0..range(rng, t.drop_count).max(1) {
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
    out.len() > before
}

/// How many times the priority list runs for this kill and how many of those runs include the rare stages (enemy
/// table, class rare). Ordinary enemies: 1 (2 for a leader / large one, `draws`); once the quest's kills reach
/// `KILLS_PER_EXTRA` a 1-draw enemy gets a second draw half of the time. Boss (`boss_drops` base): base + 1 per
/// `KILLS_PER_EXTRA` kills up to +4, with 1 rare check + the extra ones (4 drops -> 1, 8 -> 5).
pub fn draw_counts<R: Rng>(rng: &mut R, own: Option<&Table>, kills: u32) -> (u32, u32) {
    let Some(t) = own else {
        return (1 + (kills >= KILLS_PER_EXTRA && rng.gen_bool(0.5)) as u32, u32::MAX);
    };
    if t.boss_drops > 0 {
        let extra = (kills / KILLS_PER_EXTRA).min(MAX_EXTRA_DRAWS);
        return (t.boss_drops + extra, 1 + extra);
    }
    let base = t.draws.max(1);
    (base + (base == 1 && kills >= KILLS_PER_EXTRA && rng.gen_bool(0.5)) as u32, u32::MAX)
}

/// One run of the priority list; the first stage that succeeds ends it. Order: enemy table at the current level ->
/// enemy table one band lower -> [boss] class-matching rare weapon -> normal (class-matching star 1-3 weapons, star 4+
/// as they are) -> common -> area -> meseta (a boss always gets this one).
fn run_once<R: Rng>(rng: &mut R, data: &DropData, c: &Ctx, own: Option<&Table>, rare: bool, out: &mut Vec<Item>) {
    let wc = data.normal.as_ref().map(|t| &t.weapon_classes);
    let empty = HashMap::new();
    let wc = wc.unwrap_or(&empty);
    if let (true, Some(t)) = (rare, own) {
        let mul = 1.0 + (t.boost + c.boost) / 100.0;
        if stage(rng, t, c.level, t.rate * mul, |_| true, out) {
            return;
        }
        if let Some(low) = lower_level(c.level) {
            if stage(rng, t, low, t.rate * mul * LOWER_RATE_MUL, |_| true, out) {
                return;
            }
        }
        if t.boss_drops > 0 && c.class.is_some() {
            if stage(rng, t, c.level, t.rate * CLASS_RARE_RATE_MUL * mul, |e| class_rare_ok(wc, c.class, e), out) {
                return;
            }
        }
    }
    if let Some(t) = &data.normal {
        let ok = |e: &Entry| e.rarity.map_or(true, |r| r > 3) || class_matches(wc, c.class, &e.item);
        if stage(rng, t, c.level, t.rate, ok, out) {
            return;
        }
    }
    for t in [data.common.as_ref(), data.areas.get(c.area)].into_iter().flatten() {
        if stage(rng, t, c.level, t.rate, |_| true, out) {
            return;
        }
    }
    let boss = own.is_some_and(|t| t.boss_drops > 0);
    let m = data.areas.get(c.area).and_then(|t| t.meseta).or_else(|| data.common.as_ref().and_then(|t| t.meseta));
    if let Some(m) = m {
        if boss || rng.gen_range(0.0f32..1.0) < m.rate {
            let base = range(rng, Range { min: m.min, max: m.max }) as f32;
            let amount = (base * (1.0 + m.level_mul * c.level as f32)).round() as u32;
            if amount > 0 {
                out.push(meseta(amount));
            }
        }
    }
}

/// All the drops of one kill: the priority list run `draw_counts` times
pub fn roll_ctx<R: Rng>(rng: &mut R, data: &DropData, c: &Ctx) -> Vec<Item> {
    let own = data.enemies.get(c.enemy);
    let (n, rare_checks) = draw_counts(rng, own, c.kills);
    let mut out = vec![];
    for i in 0..n {
        run_once(rng, data, c, own, i < rare_checks, &mut out);
    }
    out
}

pub fn roll_with<R: Rng>(rng: &mut R, data: &DropData, enemy: &str, area: &str, level: u32) -> Vec<Item> {
    roll_ctx(rng, data, &Ctx::new(enemy, area, level))
}

pub fn roll(c: &Ctx) -> Vec<Item> {
    roll_ctx(&mut rand::thread_rng(), data(), c)
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

    fn e(item: &str, weight: f32, level_min: Option<u32>, rarity: Option<u32>) -> Entry {
        Entry { item: item.into(), weight, min: 1, max: 1, level_min, level_max: None, rarity }
    }
    fn meseta_only(amount: u32, rate: f32) -> Table {
        Table { meseta: Some(Meseta { min: amount, max: amount, rate, level_mul: 0.0 }), ..Default::default() }
    }

    #[test]
    fn weights_match_expectation() {
        let t = Table {
            entries: vec![e("3:1:0", 70.0, None, None), e("3:7:0", 30.0, None, None)],
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

    /// The first stage that succeeds ends the run: with the common stage always succeeding the area's meseta is not
    /// reached, and with a failing common stage it is.
    #[test]
    fn short_circuit_stops_at_first_success() {
        let common = |rate: f32| Table { rate, entries: vec![e("3:1:0", 1.0, None, None)], ..Default::default() };
        let mut data = DropData {
            common: Some(common(1.0)),
            areas: HashMap::from([("a".into(), meseta_only(5, 1.0))]),
            ..Default::default()
        };
        let mut rng = StdRng::seed_from_u64(2);
        for _ in 0..100 {
            let r = roll_with(&mut rng, &data, "NoTable", "a", 5);
            assert_eq!(r.len(), 1);
            assert_eq!(r[0].id.item_type, 3);
        }
        data.common = Some(common(0.0));
        let r = roll_with(&mut rng, &data, "NoTable", "a", 5);
        assert_eq!(r.len(), 1);
        assert_eq!(crate::map::meseta_amount(&r[0]), Some(5));
    }

    /// The enemy's rare stage succeeding means the normal stage is not drawn
    #[test]
    fn enemy_stage_success_skips_normal() {
        let mob = Table { rate: 1.0, entries: vec![e("1:7:24", 1.0, None, Some(8))], ..Default::default() };
        let normal = Table { rate: 1.0, entries: vec![e("3:1:0", 1.0, None, None)], ..Default::default() };
        let data = DropData {
            enemies: HashMap::from([("Mob".into(), mob)]),
            normal: Some(normal),
            ..Default::default()
        };
        let mut rng = StdRng::seed_from_u64(3);
        for _ in 0..50 {
            let r = roll_with(&mut rng, &data, "Mob", "none", 10);
            assert_eq!(r.len(), 1);
            assert_eq!(r[0].id.item_type, 1, "the normal stage was drawn after the rare one succeeded");
        }
    }

    /// A boss always drops something (the meseta stage) and its draws grow with the quest's kills: 4 at the start,
    /// 8 with 35 kills, with 1 and then 5 rare checks.
    #[test]
    fn boss_draws_and_rare_checks() {
        let boss = Table { rate: 0.0, boss_drops: 4, ..Default::default() };
        let data = DropData {
            enemies: HashMap::from([("Boss".into(), boss.clone())]),
            areas: HashMap::from([("a".into(), meseta_only(10, 0.0))]),
            ..Default::default()
        };
        let mut rng = StdRng::seed_from_u64(4);
        assert_eq!(draw_counts(&mut rng, Some(&boss), 0), (4, 1));
        assert_eq!(draw_counts(&mut rng, Some(&boss), 35), (8, 5));
        assert_eq!(draw_counts(&mut rng, Some(&boss), 500), (8, 5));
        for kills in [0, 35] {
            let c = Ctx { kills, ..Ctx::new("Boss", "a", 10) };
            let r = roll_ctx(&mut rng, &data, &c);
            assert_eq!(r.len(), if kills == 0 { 4 } else { 8 });
            assert!(r.iter().all(|i| crate::map::meseta_amount(i).is_some()));
        }
    }

    /// A star 1-3 weapon is only dropped for a class that can equip its category; star 4+ is class-independent
    #[test]
    fn class_match_filters_low_star_weapons() {
        // category 1 (sword): Hunter (0) only; category 9 (rod): Force (2) only
        let normal = Table {
            rate: 1.0,
            weapon_classes: HashMap::from([("1".into(), 0b001), ("9".into(), 0b100)]),
            entries: vec![
                e("1:1:0", 1.0, None, Some(1)),
                e("1:9:0", 1.0, None, Some(1)),
                e("1:9:30", 1.0, None, Some(5)),
            ],
            ..Default::default()
        };
        let data = DropData { normal: Some(normal), ..Default::default() };
        let mut rng = StdRng::seed_from_u64(5);
        let (mut rod_star1, mut rod_star5) = (0, 0);
        for _ in 0..600 {
            let c = Ctx { class: Some(0), ..Ctx::new("NoTable", "none", 10) };
            for i in roll_ctx(&mut rng, &data, &c) {
                match (i.id.id, i.id.subid) {
                    (9, 0) => rod_star1 += 1,
                    (9, 30) => rod_star5 += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(rod_star1, 0, "a rod star 1 dropped for a Hunter");
        assert!(rod_star5 > 0, "star 5 is class-independent");
    }

    /// The boss's class rare: only star 7+ weapons of the main class's category
    #[test]
    fn boss_class_rare_matches_main_class() {
        let wc = HashMap::from([("1".to_string(), 0b001u32), ("9".to_string(), 0b100)]);
        let sword = e("1:1:50", 1.0, None, Some(9));
        let rod = e("1:9:50", 1.0, None, Some(9));
        let low_sword = e("1:1:0", 1.0, None, Some(2));
        let potion = e("3:1:0", 1.0, None, None);
        assert!(class_rare_ok(&wc, Some(0), &sword));
        assert!(!class_rare_ok(&wc, Some(0), &rod), "a rod for a Hunter");
        assert!(class_rare_ok(&wc, Some(2), &rod));
        assert!(!class_rare_ok(&wc, Some(0), &low_sword), "star 2 is not a class rare");
        assert!(!class_rare_ok(&wc, Some(0), &potion));
        // and the stage picks from the matching ones only
        let boss = Table { entries: vec![sword, rod], ..Default::default() };
        let mut rng = StdRng::seed_from_u64(6);
        for _ in 0..50 {
            let mut out = vec![];
            assert!(stage(&mut rng, &boss, 10, 1.0, |en| class_rare_ok(&wc, Some(0), en), &mut out));
            assert_eq!(out[0].id.id, 1);
        }
    }

    /// Boost is relative: rate x (1 + boost / 100)
    #[test]
    fn boost_scales_the_rare_rate() {
        let mob = Table { rate: 0.1, entries: vec![e("1:7:24", 1.0, None, Some(8))], ..Default::default() };
        let data = DropData { enemies: HashMap::from([("Mob".into(), mob)]), ..Default::default() };
        let mut rng = StdRng::seed_from_u64(8);
        let mut count = |boost: f32| {
            (0..20_000)
                .map(|_| roll_ctx(&mut rng, &data, &Ctx { boost, ..Ctx::new("Mob", "none", 10) }).len() as u32)
                .sum::<u32>()
        };
        let (base, boosted) = (count(0.0), count(150.0));
        // 0.1 -> 0.25
        assert!((1700..=2300).contains(&base), "{base}");
        assert!((4500..=5500).contains(&boosted), "{boosted}");
    }

    #[test]
    fn lower_band_levels() {
        assert_eq!(lower_level(10), None);
        assert_eq!(lower_level(31), None);
        assert_eq!(lower_level(32), Some(31));
        assert_eq!(lower_level(44), Some(41));
        assert_eq!(lower_level(64), Some(62));
        assert_eq!(lower_level(80), Some(79));
    }

    /// The feel target (T108, T109): one lap of Forest N (Lv10) = 35 common enemies + 1 boss.
    /// Star 1-3 weapons 2-4 per lap, rare (star 7+) about 0.1 per lap, a common enemy drops nothing about half of the
    /// time (T110), the boss always drops its draws.
    #[test]
    fn one_lap_feel() {
        // normal: star 1 (id 1) / star 2 (id 2, from Lv3) weapons in the generator's proportions
        let normal = Table {
            rate: 0.06,
            entries: vec![e("1:1:0", 50.0, Some(1), Some(1)), e("1:2:1", 12.6, Some(3), Some(2))],
            ..Default::default()
        };
        let common = Table {
            rate: 0.10,
            entries: vec![e("3:1:0", 60.0, None, None), e("3:1:1", 20.0, Some(10), None), e("3:7:0", 20.0, None, None)],
            ..Default::default()
        };
        let area = Table {
            rate: 0.07,
            entries: vec![e("3:1:0", 1.0, None, None)],
            meseta: Some(Meseta { min: 20, max: 60, rate: 0.26, level_mul: 0.2 }),
            ..Default::default()
        };
        // rare: a common enemy 0.5% (a third of them have a Lv10 candidate), the boss 1.5% per rare check and 4 drops
        let mob = Table { rate: 0.005, entries: vec![e("1:7:24", 4.0, Some(1), Some(8))], ..Default::default() };
        let boss = Table {
            rate: 0.015,
            boss_drops: 4,
            entries: vec![e("1:17:33", 2.0, Some(1), Some(8))],
            ..Default::default()
        };
        let data = DropData {
            enemies: HashMap::from([("Mob".into(), mob), ("Boss".into(), boss)]),
            normal: Some(normal),
            common: Some(common),
            areas: HashMap::from([("none".into(), area)]),
        };
        let mut rng = StdRng::seed_from_u64(7);
        let (mut low, mut rare, mut mob_kills, mut mob_empty) = (0u32, 0u32, 0u32, 0u32);
        let (mut boss_items, mut boss_laps_short) = (0u32, 0u32);
        let laps = 1000;
        let is_weapon_low = |i: &Item| i.id.item_type == 1 && matches!(i.id.id, 1 | 2);
        for _ in 0..laps {
            for n in 0..35 {
                let name = if n % 3 == 0 { "Mob" } else { "NoTable" };
                let c = Ctx { class: Some(0), kills: n, ..Ctx::new(name, "none", 10) };
                let r = roll_ctx(&mut rng, &data, &c);
                mob_kills += 1;
                // the lap's kills reach KILLS_PER_EXTRA, which adds a second draw to some: compare a first-draw sample
                if n < KILLS_PER_EXTRA && r.is_empty() {
                    mob_empty += 1;
                }
                for i in &r {
                    if is_weapon_low(i) {
                        low += 1;
                    } else if i.id.item_type == 1 {
                        rare += 1;
                    }
                }
            }
            let c = Ctx { class: Some(0), kills: 35, ..Ctx::new("Boss", "none", 10) };
            let r = roll_ctx(&mut rng, &data, &c);
            boss_items += r.len() as u32;
            boss_laps_short += (r.len() < 8) as u32;
            for i in &r {
                if is_weapon_low(i) {
                    low += 1;
                } else if i.id.item_type == 1 {
                    rare += 1;
                }
            }
        }
        let per_lap = |n: u32| n as f32 / laps as f32;
        assert!((2.0..=4.0).contains(&per_lap(low)), "star 1-3 weapons per lap: {}", per_lap(low));
        assert!((0.03..=0.30).contains(&per_lap(rare)), "rare per lap: {}", per_lap(rare));
        assert_eq!(boss_laps_short, 0, "the boss dropped fewer than 8 at 35 kills ({} per lap)", per_lap(boss_items));
        let early = (KILLS_PER_EXTRA * laps) as f32;
        let empty = mob_empty as f32 / early;
        assert!((0.45..=0.65).contains(&empty), "a common enemy's empty rate: {empty} ({mob_kills} kills)");
    }

    #[test]
    fn level_band_filters_entries() {
        let t = Table {
            entries: vec![
                Entry { item: "1:3:33".into(), weight: 1.0, min: 1, max: 1, level_min: Some(20), level_max: None, rarity: None },
                Entry { item: "3:5:322".into(), weight: 1.0, min: 1, max: 1, level_min: None, level_max: None, rarity: None },
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
