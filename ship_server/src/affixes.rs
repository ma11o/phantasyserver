//! [pso2_vita_offline] Weapon affixes (特殊能力). A slot holds `(section << 9) | index` (section 1 = BOOSTER ..
//! 6 = USERSKILL2, 0 = empty), as the Vita client reads it. `data/affixes.json` (`PSO2_AFFIXES` overrides) has the
//! stats of the BOOSTER section out of the client's `item_param_element.itel`; `data/drops/affixes.json` says how
//! many and which affixes a dropped weapon gets (provisional). Read once on first use, not compiled into
//! `com_data.mp`.
use rand::Rng;
use serde::Deserialize;
use std::{collections::HashMap, path::Path, sync::OnceLock};

/// Stat bonus of one affix (or the sum of a weapon's affixes)
#[derive(Debug, Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct Bonus {
    pub s_atk: i32,
    pub r_atk: i32,
    pub t_atk: i32,
    pub s_def: i32,
    pub r_def: i32,
    pub t_def: i32,
    pub hp: i32,
    pub pp: i32,
    pub dex: i32,
}

impl std::ops::AddAssign<&Bonus> for Bonus {
    fn add_assign(&mut self, o: &Bonus) {
        self.s_atk += o.s_atk;
        self.r_atk += o.r_atk;
        self.t_atk += o.t_atk;
        self.s_def += o.s_def;
        self.r_def += o.r_def;
        self.t_def += o.t_def;
        self.hp += o.hp;
        self.pp += o.pp;
        self.dex += o.dex;
    }
}

#[derive(Debug, Deserialize)]
struct Affix {
    id: u16,
    name: String,
    #[serde(flatten)]
    bonus: Bonus,
}

#[derive(Debug, Deserialize)]
struct AffixFile {
    affixes: Vec<Affix>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Candidate {
    pub id: u16,
    pub weight: f32,
    #[serde(default)]
    pub level_min: Option<u32>,
    #[serde(default)]
    pub level_max: Option<u32>,
}

/// How many affixes a dropped weapon gets (`count_weights[n]` = weight of n affixes) and which
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(default)]
pub struct DropRule {
    pub source: String,
    pub count_weights: Vec<f32>,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Default)]
pub struct AffixData {
    names: HashMap<u16, String>,
    bonus: HashMap<u16, Bonus>,
    pub drop: DropRule,
}

impl AffixData {
    pub fn load(table: &Path, drop: &Path) -> Self {
        let mut data = Self::default();
        match std::fs::read(table).ok().and_then(|b| serde_json::from_slice::<AffixFile>(&b).ok()) {
            Some(f) => {
                for a in f.affixes {
                    data.names.insert(a.id, a.name);
                    data.bonus.insert(a.id, a.bonus);
                }
            }
            None => log::warn!("[pso2-affix] cannot read {}", table.display()),
        }
        match std::fs::read(drop).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            Some(d) => data.drop = d,
            None => log::warn!("[pso2-affix] cannot read {}", drop.display()),
        }
        log::info!(
            "[pso2-affix] loaded {} affixes, drop rule {} candidates",
            data.names.len(),
            data.drop.candidates.len()
        );
        data
    }
    pub fn name(&self, id: u16) -> &str {
        self.names.get(&id).map_or("?", |s| s.as_str())
    }
    /// Sum of the stats of the affixes in a weapon's slots (unknown ids and other sections add nothing)
    pub fn bonus(&self, slots: &[u16]) -> Bonus {
        let mut sum = Bonus::default();
        for b in slots.iter().filter(|&&id| id != 0).filter_map(|id| self.bonus.get(id)) {
            sum += b;
        }
        sum
    }
    /// Affixes for a weapon dropped by an enemy of `level`: a count by `count_weights`, then distinct candidates
    /// (in the enemy's level band) by weight
    pub fn roll<R: Rng>(&self, rng: &mut R, level: u32) -> [u16; 8] {
        let mut slots = [0u16; 8];
        let count = pick(rng, &self.drop.count_weights).unwrap_or(0).min(8);
        let mut pool: Vec<&Candidate> = self
            .drop
            .candidates
            .iter()
            .filter(|c| c.level_min.map_or(true, |m| level >= m) && c.level_max.map_or(true, |m| level <= m))
            .collect();
        for slot in slots.iter_mut().take(count) {
            let weights: Vec<f32> = pool.iter().map(|c| c.weight).collect();
            let Some(i) = pick(rng, &weights) else { break };
            *slot = pool.remove(i).id;
        }
        slots
    }
}

/// Index by weight (None if nothing has a positive weight)
fn pick<R: Rng>(rng: &mut R, weights: &[f32]) -> Option<usize> {
    let total: f32 = weights.iter().map(|w| w.max(0.0)).sum();
    if total <= 0.0 {
        return None;
    }
    let mut x = rng.gen_range(0.0f32..1.0) * total;
    weights.iter().position(|w| {
        x -= w.max(0.0);
        x < 0.0
    })
}

pub fn data() -> &'static AffixData {
    static DATA: OnceLock<AffixData> = OnceLock::new();
    DATA.get_or_init(|| {
        let table = std::env::var_os("PSO2_AFFIXES").unwrap_or_else(|| "data/affixes.json".into());
        let drops = std::env::var_os("PSO2_DROPS_DIR").unwrap_or_else(|| "data/drops".into());
        AffixData::load(Path::new(&table), &Path::new(&drops).join("affixes.json"))
    })
}

/// "name, name" of a weapon's affixes for logs
pub fn describe(slots: &[u16]) -> String {
    let d = data();
    slots
        .iter()
        .filter(|&&id| id != 0)
        .map(|&id| format!("{id} {}", d.name(id)))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, SeedableRng};

    fn sample() -> AffixData {
        let mut d = AffixData::default();
        d.bonus.insert(512, Bonus { s_atk: 10, ..Default::default() });
        d.bonus.insert(581, Bonus { pp: 4, dex: 30, ..Default::default() });
        d.drop = DropRule {
            count_weights: vec![0.0, 0.0, 1.0],
            candidates: vec![
                Candidate { id: 512, weight: 1.0, level_min: None, level_max: None },
                Candidate { id: 581, weight: 1.0, level_min: None, level_max: None },
                Candidate { id: 513, weight: 100.0, level_min: Some(11), level_max: None },
            ],
            ..Default::default()
        };
        d
    }

    #[test]
    fn bonus_sums_known_slots() {
        let b = sample().bonus(&[512, 581, 1024, 0, 0, 0, 0, 0]);
        assert_eq!(b, Bonus { s_atk: 10, pp: 4, dex: 30, ..Default::default() });
    }

    #[test]
    fn roll_distinct_in_band() {
        let d = sample();
        let mut rng = StdRng::seed_from_u64(5);
        for _ in 0..100 {
            let s = d.roll(&mut rng, 5);
            let mut got: Vec<u16> = s.iter().copied().filter(|&x| x != 0).collect();
            got.sort();
            assert_eq!(got, vec![512, 581]);
        }
    }

    #[test]
    fn real_files_load() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("data");
        let d = AffixData::load(&root.join("affixes.json"), &root.join("drops").join("affixes.json"));
        assert_eq!(d.name(512), "パワーⅠ");
        assert_eq!(d.bonus(&[581]), Bonus { pp: 4, dex: 30, ..Default::default() });
        for c in &d.drop.candidates {
            assert_ne!(d.name(c.id), "?", "unknown candidate {}", c.id);
        }
    }
}
