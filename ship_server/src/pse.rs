//! PSE and PSE burst (1B-00 start, 1B-01 end, 1B-04 level, 1B-05 burst action).
//!
//! Vita layouts are the library's. `pse_id` is the index of the type in the client's `calor.text`
//! (3 = meseta, 5 = EXP), `level` is zero based. 1B-00 `unk1` caps the level (the client keeps min(level, unk1)),
//! so it is sent as 7. 1B-04 with `pse_burst_timer` > 0 at level 7 shows "PSE burst chance" with that countdown;
//! 1B-05 action 0 (Start) shows the burst with its timer. The client ignores 1B-05 action 2 (cross burst).
//! The chance display stays at 00:00:00 after its timer and a lower level (1B-04) does not clear it; 1B-01 does.
//! So a chance that runs out and a burst that ends both end their PSE (1B-01). Outside a burst only one type is at
//! Lv8 (the client shows one chance display; a second chance sent during a burst shows after it).
//!
//! States follow the research's playable design (pso2_research `docs/pse-playable-candidates.md`): one occurrence
//! roll per kill raises one type picked from the zone's candidates; Lv1-4 stay, Lv5/6/7 end the type after their
//! timer; Lv8 opens the chance, the same type occurring again during it bursts; during a burst every human owns
//! `slots_per_human` living burst enemies refilled after `refill_s`, a kill may extend it, another type reaching Lv8
//! crosses (+`cross_s`, one more guaranteed) and at the end a guaranteed (or lucky) one more continues it.
//! Rules and effects are `data/pse.json` (`tools/gen_pse.py`). The server applies EXP, meseta, critical rate,
//! element boosts and zero range; the rest only shows.

use pso2packetlib::protocol::{
    Packet,
    pse_burst::{PSEBurstAction, PseBurstActionPacket, PseEndPacket, PseStartPacket, SetPseLevelPacket},
};
use rand::{Rng, seq::IteratorRandom, seq::SliceRandom};
use std::time::{Duration, Instant};

pub const MAX_LEVEL: u32 = 8;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Rules {
    pub types_per_zone: usize,
    pub occur_chance: f32,
    /// seconds until a type at Lv5 / 6 / 7 ends (from reaching the level)
    pub expire_s: [u64; 3],
    pub burst_chance_s: u64,
    pub burst_s: u64,
    pub slots_per_human: u32,
    pub refill_s: f32,
    pub extend_chance: f32,
    pub extend_s: Vec<u64>,
    pub cross_s: u64,
    pub one_more_chance: f32,
    pub burst_drop_rolls: u32,
    pub zero_range_m: f32,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Type {
    pub id: u32,
    pub key: String,
    pub name: String,
    /// effect in percent per level (1..=8), None = unknown
    pub levels: Vec<Option<i32>>,
    /// exp / meseta / critical / element / zero_range: applied by the server; None: shown only
    pub apply: Option<String>,
    /// weapon element boosted by an `element` type (1 fire, 2 ice, 3 thunder, ..)
    #[serde(default)]
    pub element: Option<u8>,
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

/// `PSO2_PSE=off` turns PSE off, `PSO2_PSE=<chance>` replaces `occur_chance`, `PSO2_PSE_TYPES=3,5,10` fixes the
/// zone's candidates.
pub fn enabled() -> bool {
    std::env::var("PSO2_PSE").as_deref() != Ok("off")
}
fn occur_chance() -> f32 {
    std::env::var("PSO2_PSE").ok().and_then(|v| v.parse().ok()).unwrap_or(rules().occur_chance)
}

/// One PSE running in a zone. `level` is 1..=8.
#[derive(Debug, Clone)]
pub struct Active {
    pub id: u32,
    pub level: u32,
    /// Lv5-7: the type ends then; Lv8 outside a burst: the burst chance ends then
    pub until: Option<Instant>,
}

/// A burst enemy slot owned by a human player. `enemy` None and `refill` None: waiting for its spawn.
#[derive(Debug, Clone)]
pub struct Slot {
    pub owner: u32,
    pub enemy: Option<u32>,
    pub refill: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct Burst {
    pub id: u32,
    pub until: Instant,
    /// a cross happened: the next timeout continues once (not counted)
    pub one_more: bool,
    /// types that reached Lv8 during the burst (end with it)
    pub crossed: Vec<u32>,
    pub slots: Vec<Slot>,
}

/// A zone's PSE state.
#[derive(Debug, Default)]
pub struct ZonePse {
    /// candidate types (picked when the first enemy dies)
    pub pool: Vec<u32>,
    pub active: Vec<Active>,
    pub burst: Option<Burst>,
}

/// What the zone has to do after a PSE change.
#[derive(Debug, Default)]
pub struct Outcome {
    pub packets: Vec<Packet>,
    /// spawn one burst enemy near each of these players (slot owners), then `ZonePse::assign` it
    pub spawn: Vec<u32>,
}

/// PSE effects on a player's attack, in percent.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Bonus {
    pub critical: f32,
    /// by weapon element (index 1..=6)
    pub element: [f32; 7],
    pub zero_range: f32,
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

fn secs(until: Instant, now: Instant) -> f32 {
    until.saturating_duration_since(now).as_secs_f32().round()
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

    fn percent(a: &Active) -> Option<(&'static Type, f32)> {
        let t = by_id(a.id)?;
        let p = t.levels.get(a.level as usize - 1).copied().flatten()?;
        Some((t, p as f32))
    }

    /// Multiplier of the zone's PSE of kind `apply` ("exp" / "meseta"): 1 + percent / 100.
    pub fn mul(&self, apply: &str) -> f32 {
        self.active
            .iter()
            .filter_map(Self::percent)
            .filter(|(t, _)| t.apply.as_deref() == Some(apply))
            .map(|(_, p)| p / 100.0)
            .sum::<f32>()
            + 1.0
    }

    /// Critical rate, element boosts and zero range of the zone's PSE.
    pub fn bonus(&self) -> Bonus {
        let mut b = Bonus::default();
        for (t, p) in self.active.iter().filter_map(Self::percent) {
            match (t.apply.as_deref(), t.element) {
                (Some("critical"), _) => b.critical += p,
                (Some("zero_range"), _) => b.zero_range += p,
                (Some("element"), Some(e)) if (e as usize) < b.element.len() => b.element[e as usize] += p,
                _ => {}
            }
        }
        b
    }

    /// Drop rolls per kill (more during a burst).
    pub fn drop_rolls(&self) -> u32 {
        if self.burst.is_some() { rules().burst_drop_rolls.max(1) } else { 1 }
    }

    fn chance_open(&self) -> bool {
        self.burst.is_none() && self.active.iter().any(|a| a.level == MAX_LEVEL)
    }

    /// An occurrence of type `id`: start it at Lv1 or raise it one level; Lv5-7 (re)start the type's timer, Lv8
    /// opens the burst chance (or crosses during a burst), an occurrence at Lv8 during the chance bursts.
    pub fn occur(&mut self, id: u32, humans: &[u32], out: &mut Outcome) {
        let r = rules();
        let now = Instant::now();
        let chance_open = self.chance_open();
        let Some(i) = self.active.iter().position(|a| a.id == id) else {
            self.active.push(Active { id, level: 1, until: None });
            log::info!("[pso2-pse] start {id}");
            out.packets.push(start_packet(id, 1));
            return;
        };
        let a = &mut self.active[i];
        match a.level {
            l if l + 1 < MAX_LEVEL => {
                a.level += 1;
                a.until = (a.level >= 5).then(|| now + Duration::from_secs(r.expire_s[a.level as usize - 5]));
                log::info!("[pso2-pse] {id} Lv{} (ends in {:?} s)", a.level, a.until.map(|t| secs(t, now)));
                out.packets.push(level_packet(id, a.level, 0.0));
            }
            l if l + 1 == MAX_LEVEL => {
                if let Some(b) = &mut self.burst {
                    // cross: more time and one more guaranteed
                    a.level = MAX_LEVEL;
                    a.until = None;
                    b.until += Duration::from_secs(r.cross_s);
                    b.one_more = true;
                    b.crossed.push(id);
                    log::info!("[pso2-pse] cross {id}: burst {} +{} s -> {} s, one more", b.id, r.cross_s, secs(b.until, now));
                    out.packets.push(level_packet(id, MAX_LEVEL, 0.0));
                    out.packets.push(burst_packet(PSEBurstAction::AddTime, b.id, secs(b.until, now)));
                } else if !chance_open {
                    a.level = MAX_LEVEL;
                    a.until = Some(now + Duration::from_secs(r.burst_chance_s));
                    log::info!("[pso2-pse] {id} Lv8: burst chance {} s", r.burst_chance_s);
                    out.packets.push(level_packet(id, MAX_LEVEL, r.burst_chance_s as f32));
                }
                // another type's chance is open: stays at Lv7 (its timer runs on)
            }
            _ if self.burst.is_none() => {
                // the same type during its chance: burst
                a.until = None;
                let slots = humans
                    .iter()
                    .flat_map(|h| (0..r.slots_per_human).map(|_| Slot { owner: *h, enemy: None, refill: Some(now) }))
                    .collect();
                self.burst = Some(Burst {
                    id,
                    until: now + Duration::from_secs(r.burst_s),
                    one_more: false,
                    crossed: vec![],
                    slots,
                });
                log::info!("[pso2-pse] burst {id} for {} s", r.burst_s);
                out.packets.push(burst_packet(PSEBurstAction::Start, id, r.burst_s as f32));
            }
            _ => {}
        }
    }

    /// Burst extension by `add` seconds (1B-05 AddTime with the new remaining time).
    pub fn extend(&mut self, add: u64, out: &mut Outcome) {
        let now = Instant::now();
        if let Some(b) = &mut self.burst {
            b.until += Duration::from_secs(add);
            log::info!("[pso2-pse] burst {} extended +{add} s -> {} s", b.id, secs(b.until, now));
            out.packets.push(burst_packet(PSEBurstAction::AddTime, b.id, secs(b.until, now)));
        }
    }

    /// Enemy `enemy` died: frees its burst slot, rolls the burst extension and one occurrence.
    pub fn on_kill(&mut self, enemy: u32, humans: &[u32]) -> Outcome {
        let mut out = Outcome::default();
        if !enabled() {
            return out;
        }
        let r = rules();
        let now = Instant::now();
        self.ensure_pool();
        let mut rng = rand::thread_rng();
        if let Some(b) = &mut self.burst {
            if let Some(s) = b.slots.iter_mut().find(|s| s.enemy == Some(enemy)) {
                s.enemy = None;
                s.refill = Some(now + Duration::from_secs_f32(r.refill_s));
            }
            if rng.gen_range(0.0f32..1.0) < r.extend_chance {
                let add = r.extend_s.choose(&mut rng).copied().unwrap_or(10);
                self.extend(add, &mut out);
            }
        }
        if rng.gen_range(0.0f32..1.0) < occur_chance() {
            if let Some(&id) = self.pool.choose(&mut rng) {
                self.occur(id, humans, &mut out);
            }
        }
        out
    }

    /// Once a second: Lv5-7 and chance timers, burst slots (owners = `humans`), burst end / one more.
    pub fn tick(&mut self, humans: &[u32]) -> Outcome {
        let mut out = Outcome::default();
        let r = rules();
        let now = Instant::now();
        let mut ended = vec![];
        if let Some(b) = &mut self.burst {
            if now >= b.until {
                let lucky = rand::thread_rng().gen_range(0.0f32..1.0) < r.one_more_chance;
                if b.one_more || lucky {
                    log::info!("[pso2-pse] one more {} ({})", b.id, if b.one_more { "cross" } else { "chance" });
                    b.one_more = false;
                    b.until = now + Duration::from_secs(r.burst_s);
                    out.packets.push(burst_packet(PSEBurstAction::OneMore, b.id, r.burst_s as f32));
                } else {
                    log::info!("[pso2-pse] burst {} ended (crossed {:?})", b.id, b.crossed);
                    ended.push(b.id);
                    ended.extend(&b.crossed);
                }
            }
        }
        if !ended.is_empty() {
            // the slots close; their living enemies stay
            self.burst = None;
        }
        if let Some(b) = &mut self.burst {
            // one slot set per human in the zone
            b.slots.retain(|s| humans.contains(&s.owner));
            for h in humans {
                let n = b.slots.iter().filter(|s| s.owner == *h).count() as u32;
                for _ in n..r.slots_per_human {
                    b.slots.push(Slot { owner: *h, enemy: None, refill: Some(now) });
                }
            }
            for s in b.slots.iter_mut().filter(|s| s.refill.is_some_and(|t| now >= t)) {
                s.refill = None;
                out.spawn.push(s.owner);
            }
        }
        for a in &self.active {
            if a.until.is_some_and(|t| now >= t) {
                log::info!("[pso2-pse] {} Lv{} timed out", a.id, a.level);
                ended.push(a.id);
            }
        }
        self.active.retain(|a| !ended.contains(&a.id));
        out.packets.extend(ended.into_iter().map(end_packet));
        out
    }

    /// The burst enemy `enemy` was spawned for `owner`: it fills the owner's waiting slot.
    pub fn assign(&mut self, owner: u32, enemy: u32) {
        if let Some(s) = self
            .burst
            .as_mut()
            .and_then(|b| b.slots.iter_mut().find(|s| s.owner == owner && s.enemy.is_none() && s.refill.is_none()))
        {
            s.enemy = Some(enemy);
        }
    }

    /// Packets showing the running PSEs to a player entering the zone.
    pub fn show(&self) -> Vec<Packet> {
        self.active.iter().map(|a| start_packet(a.id, a.level)).collect()
    }

    /// Debug `pse set <id> <level>`: sets one PSE's level (0 ends it). Lv8 goes through `occur` from Lv7 (burst
    /// chance, or a cross during a burst).
    pub fn set(&mut self, id: u32, level: u32, humans: &[u32]) -> Outcome {
        let mut out = Outcome::default();
        let now = Instant::now();
        let level = level.min(MAX_LEVEL);
        if level == 0 {
            if let Some(i) = self.active.iter().position(|a| a.id == id) {
                self.active.remove(i);
                out.packets.push(end_packet(id));
            }
            return out;
        }
        let l = level.min(MAX_LEVEL - 1);
        let until = (l >= 5).then(|| now + Duration::from_secs(rules().expire_s[l as usize - 5]));
        match self.active.iter_mut().find(|a| a.id == id) {
            Some(a) => {
                a.level = l;
                a.until = until;
                out.packets.push(level_packet(id, l, 0.0));
            }
            None => {
                self.active.push(Active { id, level: l, until });
                out.packets.push(start_packet(id, l));
            }
        }
        if level == MAX_LEVEL {
            self.occur(id, humans, &mut out);
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
    fn ids_and_effects() {
        assert_eq!(by_id(3).unwrap().key, "meseta_drop");
        assert_eq!(by_id(5).unwrap().key, "experience");
        let mut z = ZonePse::default();
        z.set(3, 7, &[1]);
        z.occur(3, &[1], &mut Outcome::default()); // Lv8 (chance)
        z.set(5, 1, &[1]);
        z.set(10, 4, &[1]); // fire boost Lv4
        z.set(9, 2, &[1]); // critical Lv2
        z.set(26, 1, &[1]); // zero range Lv1
        assert_eq!(z.mul("meseta"), 6.0); // +500% at Lv8
        assert_eq!(z.mul("exp"), 1.1); // +10% at Lv1
        let b = z.bonus();
        assert_eq!((b.element[1], b.critical, b.zero_range), (35.0, 6.0, 3.0));
        assert_eq!(z.drop_rolls(), 1);
    }

    #[test]
    fn chance_burst_cross_one_more() {
        let mut z = ZonePse { pool: vec![3, 5], ..Default::default() };
        let mut out = Outcome::default();
        z.set(3, 7, &[1]);
        z.set(5, 7, &[1]);
        // Lv5-7 have a timer, Lv4 does not
        assert!(z.active.iter().all(|a| a.until.is_some()));
        z.occur(3, &[1], &mut out);
        // one chance at a time outside a burst: 5 stays at Lv7
        z.occur(5, &[1], &mut out);
        assert_eq!(z.active.iter().map(|a| a.level).collect::<Vec<_>>(), vec![8, 7]);
        // the same type during its chance bursts and opens one slot for the human
        z.occur(3, &[1], &mut out);
        let b = z.burst.as_ref().unwrap();
        assert_eq!((b.id, b.slots.len(), b.one_more), (3, 1, false));
        assert_eq!(z.drop_rolls(), 2);
        // the slot spawns, gets its enemy, and refills after it dies
        let t = z.tick(&[1]);
        assert_eq!(t.spawn, vec![1]);
        z.assign(1, 100);
        assert_eq!(z.burst.as_ref().unwrap().slots[0].enemy, Some(100));
        z.on_kill(100, &[1]);
        assert!(z.burst.as_ref().unwrap().slots[0].refill.is_some());
        // another type at Lv8 during the burst: cross, +60 s, one more guaranteed
        let before = z.burst.as_ref().unwrap().until;
        z.occur(5, &[1], &mut out);
        let b = z.burst.as_ref().unwrap();
        assert_eq!(b.until - before, Duration::from_secs(rules().cross_s));
        assert!(b.one_more);
        assert_eq!(b.crossed, vec![5]);
        // timeout with the guarantee: one more, the guarantee is spent
        z.burst.as_mut().unwrap().until = Instant::now();
        z.tick(&[1]);
        let b = z.burst.as_ref().unwrap();
        assert!(!b.one_more && b.until > Instant::now());
    }

    #[test]
    fn timers_end_types() {
        let mut z = ZonePse::default();
        z.set(3, 4, &[1]);
        z.set(5, 5, &[1]);
        z.active[1].until = Some(Instant::now());
        let out = z.tick(&[1]);
        // Lv4 stays, Lv5 ends as a whole
        assert_eq!(z.active.iter().map(|a| (a.id, a.level)).collect::<Vec<_>>(), vec![(3, 4)]);
        assert_eq!(out.packets.len(), 1);
    }
}
