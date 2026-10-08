//! [pso2_vita_offline] Power of photon arts and techniques by disc level. `data/pa_power.json` (`PSO2_PA_POWER`
//! overrides) has, per disc (`SwordPA05`), the client's power per level (`.pha`, 1.0 = 100%). A PA hit
//! (`DamageType::PA((disc id, share))`) deals `power[disc_level - 1] * share` where `share` is the hit's part from
//! the weapon's `.ski`. The disc level the character learned is not tracked, so `disc_level` from the file
//! (provisional) is used for every disc.
use serde::Deserialize;
use std::{collections::HashMap, sync::OnceLock};

#[derive(Debug, Deserialize, Default)]
struct PaFile {
    #[serde(default)]
    disc_level: u32,
    #[serde(default)]
    discs: HashMap<String, Vec<f32>>,
}

struct PaTable {
    disc_level: u32,
    /// name_to_id(disc name) -> (name, power per level)
    discs: HashMap<u32, (String, Vec<f32>)>,
}

fn table() -> &'static PaTable {
    static DATA: OnceLock<PaTable> = OnceLock::new();
    DATA.get_or_init(|| {
        let path = std::env::var_os("PSO2_PA_POWER").unwrap_or_else(|| "data/pa_power.json".into());
        let f: PaFile = match std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            Some(f) => f,
            None => {
                log::warn!("[pso2-pa] cannot read {}", std::path::Path::new(&path).display());
                PaFile::default()
            }
        };
        log::info!("[pso2-pa] loaded {} discs, disc level {}", f.discs.len(), f.disc_level);
        PaTable {
            disc_level: f.disc_level.max(1),
            discs: f
                .discs
                .into_iter()
                .map(|(k, v)| (data_structs::name_to_id(&k), (k, v)))
                .collect(),
        }
    })
}

/// Multiplier of a PA hit: the disc's power at the provisional level times the hit's share. Unknown discs use the
/// share alone, with a warning.
pub fn multiplier(disc_id: u32, share: f32) -> f32 {
    let t = table();
    match t.discs.get(&disc_id) {
        Some((name, levels)) if !levels.is_empty() => {
            let lv = (t.disc_level as usize).min(levels.len());
            let power = levels[lv - 1];
            log::debug!("[pso2-pa] {name} Lv{lv} power {power} x share {share}");
            power * share
        }
        _ => {
            log::warn!("[pso2-pa] unknown disc 0x{disc_id:08x}, using the share {share} alone");
            share
        }
    }
}
