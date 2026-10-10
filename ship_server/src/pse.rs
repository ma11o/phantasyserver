//! PSE and PSE burst (1B-00 start, 1B-01 end, 1B-04 level, 1B-05 burst action).
//!
//! Vita layouts are the library's. `pse_id` is the index of the type in the client's `calor.text`
//! (3 = meseta, 5 = EXP), `level` is zero based. 1B-00 `unk1` caps the level (the client keeps min(level, unk1)),
//! so it is sent as 7. 1B-04 with `pse_burst_timer` > 0 at level 7 shows "PSE burst chance" with that countdown;
//! 1B-05 action 0 (Start) shows the burst with its timer. The client ignores 1B-05 action 2 (cross burst).
//! The chance display stays at 00:00:00 after its timer and a lower level (1B-04) does not clear it; 1B-01 does.
//! So a chance that runs out and a burst that ends both end their PSE (1B-01). Only one type is at Lv8 at a time
//! (the client shows one chance / burst display; a second chance sent during a burst shows after it).
//! Rules and effects are `data/pse.json` (`tools/gen_pse.py`). The server applies EXP and meseta, the rest only shows.

use pso2packetlib::protocol::{
    Packet,
    pse_burst::{PSEBurstAction, PseBurstActionPacket, PseEndPacket, PseStartPacket, SetPseLevelPacket},
};
use rand::{Rng, seq::IteratorRandom};
use std::time::Instant;

pub const MAX_LEVEL: u32 = 8;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Rules {
    pub types_per_zone: usize,
    pub start_chance: f32,
    pub up_chance: f32,
    pub decay_s: u64,
    pub burst_chance_s: u64,
    pub burst_s: u64,
    pub burst_spawn_every_s: u64,
    pub burst_spawn: (u32, u32),
    pub burst_max_enemies: u32,
    pub burst_drop_rolls: u32,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Type {
    pub id: u32,
    pub key: String,
    pub name: String,
    /// effect in percent per level (1..=8), None = unknown
    pub levels: Vec<Option<i32>>,
    /// "exp" / "meseta": applied by the server; None: shown only
    pub apply: Option<String>,
    /// can occur in free fields
    pub pool: bool,
}

#[derive(Debug, serde::Deserialize)]
struct Data {
    rules: Rules,
    types: Vec<Type>,
}

fn data() -> &'static Data {
    static DATA: std::sync::OnceLock<Data> = std::sync::OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("../../data/pse.json")).expect("data/pse.json"))
}
pub fn rules() -> &'static Rules {
    &data().rules
}
pub fn types() -> &'static [Type] {
    &data().types
}
pub fn by_id(id: u32) -> Option<&'static Type> {
    types().iter().find(|t| t.id == id)
}

/// `PSO2_PSE=off` turns PSE off, `PSO2_PSE=<chance>` replaces `start_chance`, `PSO2_PSE_UP=<chance>` replaces
/// `up_chance`, `PSO2_PSE_TYPES=3,5,10` fixes the zone's candidates.
pub fn enabled() -> bool {
    std::env::var("PSO2_PSE").as_deref() != Ok("off")
}
fn up_chance() -> f32 {
    std::env::var("PSO2_PSE_UP").ok().and_then(|v| v.parse().ok()).unwrap_or(rules().up_chance)
}
fn start_chance() -> f32 {
    std::env::var("PSO2_PSE").ok().and_then(|v| v.parse().ok()).unwrap_or(rules().start_chance)
}

/// One PSE running in a zone. `level` is 1..=8.
#[derive(Debug, Clone)]
pub struct Active {
    pub id: u32,
    pub level: u32,
    /// Lv8 reached: the burst chance runs until then
    pub chance_until: Option<Instant>,
}

/// A zone's PSE state.
#[derive(Debug, Default)]
pub struct ZonePse {
    /// candidate types (picked when the first enemy dies)
    pub pool: Vec<u32>,
    pub active: Vec<Active>,
    pub last_kill: Option<Instant>,
    /// (bursting type, end, next spawn)
    pub burst: Option<(u32, Instant, Instant)>,
}

/// What the zone has to do after a PSE change.
#[derive(Debug, Default)]
pub struct Outcome {
    pub packets: Vec<Packet>,
    /// spawn this many enemies near the players (burst)
    pub spawn: u32,
}

pub fn start_packet(id: u32, level: u32) -> Packet {
    Packet::PseStart(PseStartPacket { pse_id: id, level: level - 1, unk1: MAX_LEVEL - 1, unk2: 0 })
}
pub fn level_packet(id: u32, level: u32, timer: f32) -> Packet {
    Packet::SetPseLevel(SetPseLevelPacket { pse_id: id, level: level - 1, pse_burst_timer: timer })
}
pub fn end_packet(id: u32) -> Packet {
    Packet::PseEnd(PseEndPacket { pse_id: id })
}
pub fn burst_packet(action: PSEBurstAction, id: u32, timer: f32) -> Packet {
    Packet::PseBurstAction(PseBurstActionPacket { action, pse_id: id, unk3: 0, timer })
}

impl ZonePse {
    fn ensure_pool(&mut self) {
        if !self.pool.is_empty() {
            return;
        }
        if let Ok(v) = std::env::var("PSO2_PSE_TYPES") {
            self.pool = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            return;
        }
        let mut rng = rand::thread_rng();
        self.pool = types()
            .iter()
            .filter(|t| t.pool)
            .map(|t| t.id)
            .choose_multiple(&mut rng, rules().types_per_zone);
    }

    /// Multiplier of the zone's PSE of kind `apply` ("exp" / "meseta"): 1 + percent / 100.
    pub fn mul(&self, apply: &str) -> f32 {
        self.active
            .iter()
            .filter_map(|a| {
                let t = by_id(a.id)?;
                (t.apply.as_deref() == Some(apply))
                    .then(|| t.levels.get(a.level as usize - 1).copied().flatten())
                    .flatten()
            })
            .map(|p| p as f32 / 100.0)
            .sum::<f32>()
            + 1.0
    }

    /// Drop rolls per kill (2 during a burst).
    pub fn drop_rolls(&self) -> u32 {
        if self.burst.is_some() { rules().burst_drop_rolls.max(1) } else { 1 }
    }

    /// An enemy died: start a new PSE, raise levels, start the burst chance at Lv8 or the burst itself.
    pub fn on_kill(&mut self) -> Outcome {
        let mut out = Outcome::default();
        if !enabled() {
            return out;
        }
        let r = rules();
        let now = Instant::now();
        self.last_kill = Some(now);
        self.ensure_pool();
        let mut rng = rand::thread_rng();
        // one type at Lv8 at a time: the others stop at Lv7
        let mut lv8_taken = self.burst.is_some() || self.active.iter().any(|a| a.level == MAX_LEVEL);
        for a in &mut self.active {
            if rng.gen_range(0.0f32..1.0) >= up_chance() {
                continue;
            }
            if a.level < MAX_LEVEL {
                if a.level + 1 == MAX_LEVEL {
                    if lv8_taken {
                        continue;
                    }
                    lv8_taken = true;
                }
                a.level += 1;
                let timer = if a.level == MAX_LEVEL {
                    a.chance_until = Some(now + std::time::Duration::from_secs(r.burst_chance_s));
                    r.burst_chance_s as f32
                } else {
                    0.0
                };
                log::info!("[pso2-pse] {} Lv{}", a.id, a.level);
                out.packets.push(level_packet(a.id, a.level, timer));
            } else if a.chance_until.is_some() && self.burst.is_none() {
                // a further rank up during the burst chance: burst
                a.chance_until = None;
                let end = now + std::time::Duration::from_secs(r.burst_s);
                self.burst = Some((a.id, end, now));
                log::info!("[pso2-pse] burst {} for {} s", a.id, r.burst_s);
                out.packets.push(burst_packet(PSEBurstAction::Start, a.id, r.burst_s as f32));
            }
        }
        let free: Vec<u32> = self.pool.iter().copied().filter(|id| !self.active.iter().any(|a| a.id == *id)).collect();
        if !free.is_empty() && rng.gen_range(0.0f32..1.0) < start_chance() {
            let id = free[rng.gen_range(0..free.len())];
            self.active.push(Active { id, level: 1, chance_until: None });
            log::info!("[pso2-pse] start {id}");
            out.packets.push(start_packet(id, 1));
        }
        out
    }

    /// Once a second: level decay, burst chance timeout, burst spawns (up to `burst_max_enemies` alive, `enemies` =
    /// alive now) and burst end.
    pub fn tick(&mut self, enemies: u32) -> Outcome {
        let mut out = Outcome::default();
        let r = rules();
        let now = Instant::now();
        if let Some((id, end, next)) = self.burst {
            if now >= end {
                // the burst ends with its PSE
                self.burst = None;
                self.active.retain(|a| a.id != id);
                log::info!("[pso2-pse] burst {id} ended");
                out.packets.push(end_packet(id));
                self.last_kill = Some(now);
            } else if now >= next {
                let (lo, hi) = r.burst_spawn;
                out.spawn = rand::thread_rng().gen_range(lo.min(hi)..=hi.max(lo)).min(r.burst_max_enemies.saturating_sub(enemies));
                self.burst = Some((id, end, now + std::time::Duration::from_secs(r.burst_spawn_every_s)));
            }
            return out;
        }
        // the chance ran out: the PSE ends
        self.active.retain(|a| {
            let over = a.chance_until.is_some_and(|t| now >= t);
            if over {
                log::info!("[pso2-pse] {} burst chance over", a.id);
                out.packets.push(end_packet(a.id));
            }
            !over
        });
        // no kill for decay_s: every PSE loses a level, Lv1 ends
        if self.last_kill.is_some_and(|t| now.duration_since(t).as_secs() >= r.decay_s) && !self.active.is_empty() {
            self.last_kill = Some(now);
            for a in &mut self.active {
                if a.chance_until.is_some() {
                    continue;
                }
                a.level -= 1;
                out.packets.push(if a.level == 0 { end_packet(a.id) } else { level_packet(a.id, a.level, 0.0) });
            }
            self.active.retain(|a| a.level > 0);
            log::info!("[pso2-pse] decay: {:?}", self.active.iter().map(|a| (a.id, a.level)).collect::<Vec<_>>());
        }
        out
    }

    /// Packets showing the running PSEs to a player entering the zone.
    pub fn show(&self) -> Vec<Packet> {
        self.active.iter().map(|a| start_packet(a.id, a.level)).collect()
    }

    /// Debug `pse set <id> <level>`: starts or changes one PSE (level 0 ends it). Lv8 starts the burst chance.
    pub fn set(&mut self, id: u32, level: u32) -> Outcome {
        let mut out = Outcome::default();
        let now = Instant::now();
        self.last_kill = Some(now);
        let level = level.min(MAX_LEVEL);
        let i = self.active.iter().position(|a| a.id == id);
        match (i, level) {
            (None, 0) => {}
            (Some(i), 0) => {
                self.active.remove(i);
                out.packets.push(end_packet(id));
            }
            (None, l) => {
                self.active.push(Active { id, level: l, chance_until: None });
                out.packets.push(start_packet(id, l));
            }
            (Some(i), l) => {
                let a = &mut self.active[i];
                a.level = l;
                let timer = if l == MAX_LEVEL {
                    a.chance_until = Some(now + std::time::Duration::from_secs(rules().burst_chance_s));
                    rules().burst_chance_s as f32
                } else {
                    a.chance_until = None;
                    0.0
                };
                out.packets.push(level_packet(id, l, timer));
            }
        }
        out
    }
}

/// One task per quest map: `Map::pse_tick` once a second until the map is dropped.
pub fn spawn_ticker(map: std::sync::Weak<crate::mutex::Mutex<crate::map::Map>>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let Some(map) = map.upgrade() else { break };
            if let Err(e) = map.lock().await.pse_tick().await {
                log::warn!("[pso2-pse] tick: {e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_multipliers() {
        assert_eq!(by_id(3).unwrap().key, "meseta_drop");
        assert_eq!(by_id(5).unwrap().key, "experience");
        let mut z = ZonePse::default();
        z.set(3, 8);
        z.set(5, 1);
        z.set(10, 4);
        assert_eq!(z.mul("meseta"), 6.0); // +500% at Lv8
        assert_eq!(z.mul("exp"), 1.1); // +10% at Lv1
        assert_eq!(z.drop_rolls(), 1);
    }

    #[test]
    fn one_type_at_lv8() {
        // SAFETY: tests in this module only read these variables
        unsafe { std::env::set_var("PSO2_PSE_UP", "1") };
        let mut z = ZonePse { pool: vec![3, 5], ..Default::default() };
        z.set(3, 7);
        z.set(5, 7);
        z.on_kill();
        let lv8 = z.active.iter().filter(|a| a.level == MAX_LEVEL).count();
        assert_eq!(lv8, 1);
        // the next kill bursts the Lv8 type; the other stays at Lv7
        z.on_kill();
        assert!(z.burst.is_some());
        assert_eq!(z.drop_rolls(), 2);
        assert_eq!(z.active.iter().filter(|a| a.level == MAX_LEVEL).count(), 1);
    }
}
