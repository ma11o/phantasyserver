//! [pso2_vita_offline] Passive skills of the main class (provisional build). `data/skills.json` (`PSO2_SKILLS`
//! overrides) has, per skill, the client's per-Lv values (`.usk`) and `lv_div`: the skill's Lv is the character's
//! level / `lv_div`, at least 1 and at most the table's length. Which skills are taken is not modelled (no
//! skill-tree packets are handled), so the whole list is always on. Only hunter is filled in.
use serde::Deserialize;
use std::{collections::HashMap, sync::OnceLock};

#[derive(Debug, Deserialize)]
struct Skill {
    skill: String,
    lv_div: u32,
    fields: HashMap<String, Vec<f32>>,
}

#[derive(Debug, Deserialize, Default)]
struct SkillFile {
    #[serde(default)]
    class: String,
    #[serde(default)]
    skills: Vec<Skill>,
}

/// Sum of the effects (flat adds, multipliers of the attack, extra damage taken in percent)
#[derive(Debug, Clone, PartialEq)]
pub struct SkillBonus {
    pub hp: u32,
    pub s_atk: u32,
    pub s_def: u32,
    pub dex: u32,
    pub s_atk_mul: f32,
    pub r_atk_mul: f32,
    /// FuryStance's `DamageStrikeRate` (1.15 at Lv1 .. 1.0 at Lv10) as a multiplier of the damage taken
    pub taken_mul: f32,
}

impl Default for SkillBonus {
    fn default() -> Self {
        Self { hp: 0, s_atk: 0, s_def: 0, dex: 0, s_atk_mul: 1.0, r_atk_mul: 1.0, taken_mul: 1.0 }
    }
}

fn file() -> &'static SkillFile {
    static DATA: OnceLock<SkillFile> = OnceLock::new();
    DATA.get_or_init(|| {
        let path = std::env::var_os("PSO2_SKILLS").unwrap_or_else(|| "data/skills.json".into());
        let f: SkillFile = match std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            Some(f) => f,
            None => {
                log::warn!("[pso2-skill] cannot read {}", std::path::Path::new(&path).display());
                SkillFile::default()
            }
        };
        log::info!("[pso2-skill] loaded {} {} skills", f.skills.len(), f.class);
        f
    })
}

/// Effects of the passives for a character of `class_name` ("hunter") and `level`, with a log line per skill
pub fn bonus(class_name: &str, level: u32) -> SkillBonus {
    let f = file();
    let mut b = SkillBonus::default();
    if f.class != class_name {
        return b;
    }
    for s in &f.skills {
        let n = s.fields.values().map(Vec::len).max().unwrap_or(0) as u32;
        let lv = (level / s.lv_div.max(1)).clamp(1, n.max(1));
        let v = |k: &str| s.fields.get(k).and_then(|t| t.get(lv as usize - 1)).copied();
        if let Some(x) = v("HpMax") { b.hp += x as u32; }
        if let Some(x) = v("Strike") { b.s_atk += x as u32; }
        if let Some(x) = v("DfpStrike") { b.s_def += x as u32; }
        if let Some(x) = v("Dex") { b.dex += x as u32; }
        if let Some(x) = v("AtpStrikeRate") { b.s_atk_mul *= x; }
        if let Some(x) = v("AtpFirearmRate") { b.r_atk_mul *= x; }
        if let Some(x) = v("DamageStrikeRate") { b.taken_mul *= x; }
        log::debug!("[pso2-skill] {} Lv{lv} {:?}", s.skill, s.fields.keys().map(|k| (k.as_str(), v(k))).collect::<Vec<_>>());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_neutral() {
        assert_eq!(bonus("nobody", 10), SkillBonus::default());
    }
}
