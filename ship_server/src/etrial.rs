//! Emergency trials (15-02 start, 15-05 progress, 15-03 end).
//!
//! Vita 15-02 / 15-03 have the same field order as the library structs. The client drops a 15-02 unless:
//! the object is an `Object` (type 6); both 0x18 B target slots of `unk1` are valid (an empty slot is
//! kind 0 / sub 0xFF with a null header); `unk1[0x38]` (flags) < 32; `unk1[0x3C]` in 1..=3; `unk1[0x3D]` in 0..=2;
//! the fail / pass counts (`unk8` / `unk9`) are < 4 / < 3; `unk14` (float) >= 0; `unk15[0x18]` is a known kind;
//! `unk16` <= 0x14. `trial_id` is the text file (`ii_<name>`), the strings are keys in it.
//! 15-03: exactly one of bit 0 (failure) / bit 1 (success) in `unk1`, other bits only from 15..=26;
//! `unk5` = { meseta, exp, ?, item (0x10) } for the result line.

use pso2packetlib::{
    fixed_types::FixedBytes,
    protocol::{
        ObjectHeader, ObjectType,
        emergency::{EmergencyCondition, EmergencyEndPacket, EmergencyProgressPacket, SpawnEmergencyPacket, Unk1502_1},
    },
};

/// Knobs for a 15-02 (the debug command sets them one by one).
#[derive(Debug, Clone)]
pub struct StartParams {
    pub obj_id: u32,
    pub map_id: u16,
    pub trial_id: String,
    pub name_key: String,
    pub abstract_key: String,
    pub fail_keys: Vec<String>,
    pub pass_keys: Vec<String>,
    pub begin_key: String,
    pub key12: String,
    pub key18: String,
    pub delay: f32,
    pub flags: u32,
    pub disp: u8,
    pub b3d: u8,
    pub b3e: u8,
    pub b3f: u8,
    pub kind15: u32,
    pub op: u32,
    pub unk17: u32,
    pub unk21: u32,
    /// `unk1` target slots: (kind +0x14, sub +0x15, u32 +0x00, u32 +0x04); `None` = empty (kind 0 / sub 0xFF)
    pub slots: [Option<(u8, u8, u32, u32)>; 2],
    /// `$(0)` of the name / abstract / begin line / begin message (`unk3` / `unk5` / `unk11` / `unk13`)
    pub args: Vec<Unk1502_1>,
}

impl Default for StartParams {
    fn default() -> Self {
        Self {
            obj_id: 0x7000_0001,
            map_id: 0,
            trial_id: "ii_boss_ahl_3boss".into(),
            name_key: "IncidentName".into(),
            abstract_key: "TrialAbstract".into(),
            fail_keys: vec!["FCondTimeLimitOver".into()],
            pass_keys: vec!["ProgNoDataAllEnemyKill".into()],
            begin_key: "NpcComOnBegin".into(),
            key12: "TrialBeginMsg".into(),
            key18: String::new(),
            delay: 0.0,
            flags: 0,
            disp: 1,
            b3d: 0,
            b3e: 2,
            b3f: 0,
            kind15: 0,
            op: 0,
            unk17: 0,
            unk21: 0,
            slots: [None, None],
            args: vec![],
        }
    }
}

pub fn object(obj_id: u32, map_id: u16) -> ObjectHeader {
    ObjectHeader { id: obj_id, entity_type: ObjectType::Object, map_id, ..Default::default() }
}

fn cond(key: &str) -> EmergencyCondition {
    EmergencyCondition { cond_name: key.into(), cond_data: vec![] }
}

pub fn start_packet(p: &StartParams) -> SpawnEmergencyPacket {
    // unk1: two empty target slots (0x18 each), then u32 x2, flags, 4 bytes
    let mut unk1 = vec![0u8; 0x40];
    for (i, slot) in p.slots.iter().enumerate() {
        let o = i * 0x18;
        match slot {
            Some((kind, sub, a, b)) => {
                unk1[o..o + 4].copy_from_slice(&a.to_le_bytes());
                unk1[o + 4..o + 8].copy_from_slice(&b.to_le_bytes());
                unk1[o + 0x14] = *kind;
                unk1[o + 0x15] = *sub;
            }
            None => unk1[o + 0x15] = 0xFF,
        }
    }
    unk1[0x38..0x3C].copy_from_slice(&p.flags.to_le_bytes());
    unk1[0x3C] = p.disp;
    unk1[0x3D] = p.b3d;
    unk1[0x3E] = p.b3e;
    unk1[0x3F] = p.b3f;
    let mut unk15 = vec![0u8; 0x20];
    unk15[0x18..0x1C].copy_from_slice(&p.kind15.to_le_bytes());
    let mut unk14 = [0u8; 4];
    unk14.copy_from_slice(&p.delay.to_le_bytes());
    SpawnEmergencyPacket {
        object: object(p.obj_id, p.map_id),
        trial_id: p.trial_id.as_str().into(),
        unk1: unk1.into(),
        unk2: p.name_key.as_str().into(),
        unk3: p.args.clone(),
        unk4: p.abstract_key.as_str().into(),
        unk5: p.args.clone(),
        fail_conds: p.fail_keys.iter().map(|k| cond(k)).collect::<Vec<_>>().into(),
        pass_conds: p.pass_keys.iter().map(|k| cond(k)).collect::<Vec<_>>().into(),
        unk8: p.fail_keys.len() as u32,
        unk9: p.pass_keys.len() as u32,
        unk10: p.begin_key.as_str().into(),
        unk11: p.args.clone(),
        unk12: p.key12.as_str().into(),
        unk13: p.args.clone(),
        unk14: u32::from_le_bytes(unk14),
        unk15: unk15.into(),
        unk16: p.op,
        unk17: p.unk17,
        unk18: p.key18.as_str().into(),
        unk21: p.unk21,
        ..Default::default()
    }
}

pub fn end_packet(obj_id: u32, map_id: u16, success: bool, flags: u32, meseta: u32, exp: u32, npc_key: &str) -> EmergencyEndPacket {
    let mut unk5 = vec![0u8; 0x1C];
    unk5[0..4].copy_from_slice(&meseta.to_le_bytes());
    unk5[4..8].copy_from_slice(&exp.to_le_bytes());
    EmergencyEndPacket {
        object: object(obj_id, map_id),
        unk1: (if success { 2 } else { 1 }) | flags,
        unk5: FixedBytes::from(unk5),
        unk9: npc_key.into(),
        ..Default::default()
    }
}

/// A text argument: a string (type 2) for role `role` (`DispTextArg`: 12 = NameEnemyAtAssign, the enemy's
/// internal name, shown with its display name).
pub fn text_arg(text: &str, role: u32) -> Unk1502_1 {
    let mut b = vec![0u8; 0x24];
    let t = text.as_bytes();
    let n = t.len().min(0x1E);
    b[..n].copy_from_slice(&t[..n]);
    b[0x1F] = 2;
    b[0x20..0x24].copy_from_slice(&role.to_le_bytes());
    Unk1502_1 { unk1: b.into() }
}

pub fn progress_packet(obj_id: u32, map_id: u16, unk2: u32, unk3: u32, done: u32, unk5: u32) -> EmergencyProgressPacket {
    EmergencyProgressPacket { emergency: object(obj_id, map_id), unk2, unk3, done, unk5 }
}

/// One trial kind (`data/etrials/<kind>.json`, made by `tools/gen_etrials.py`).
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct Def {
    pub trial_id: String,
    pub kind: u8,
    pub color: u8,
    pub name_key: String,
    pub abstract_key: String,
    pub begin_key: String,
    pub success_key: String,
    pub failure_key: String,
    pub fail_keys: Vec<String>,
    pub pass_keys: Vec<String>,
    /// `TrialBeginMsg` (the band under the code)
    pub begin_msg_key: String,
    /// `unk1[0x38]`: 8 = NPC lines (begin / end)
    pub flags: u32,
    /// `unk16`, one picked at random: the NPC who speaks (0 Brigitta, 1 Hilda, 2 Melita, 3 Henrietta, 4 Xiera)
    pub npcs: Vec<u32>,
    /// target slot 0 (`TrialObjectiveKind`): (slot kind, kind); the second u32 is the enemy count
    pub slot: Option<(u8, u8)>,
    /// fixed enemies (internal names, one picked per spawn); empty = the zone's enemies
    pub enemy_names: Vec<String>,
    /// pass the first enemy's name as `$(0)` (`ii_areaboss`: "$(0)討伐")
    pub name_arg: bool,
    /// relative weight among the kinds when one starts
    pub weight: u32,
    /// reward multiplier
    pub reward_scale: f32,
    pub enemies: (u32, u32),
    pub time_limit_s: u64,
    pub meseta_per_level: u32,
    pub exp_points: Vec<(u32, u32)>,
    pub failure_ratio: f32,
    pub chance: f32,
    pub max_per_zone: usize,
    pub max_per_run: u32,
}

/// The trial kinds (`data/etrials/<kind>.json`). The start rules (chance, caps) are the first kind's.
pub fn kinds() -> &'static [Def] {
    static DEFS: std::sync::OnceLock<Vec<Def>> = std::sync::OnceLock::new();
    DEFS.get_or_init(|| {
        [include_str!("../../data/etrials/annihilation.json"), include_str!("../../data/etrials/areaboss.json")]
            .iter()
            .filter_map(|s| serde_json::from_str(s).map_err(|e| log::warn!("[pso2-etrial] bad def: {e}")).ok())
            .collect()
    })
}

pub fn rules() -> &'static Def {
    static EMPTY: std::sync::OnceLock<Def> = std::sync::OnceLock::new();
    kinds().first().unwrap_or_else(|| EMPTY.get_or_init(Def::default))
}

/// A kind index by weight. `PSO2_ETRIAL_KIND=<trial_id>` picks that one.
pub fn pick_kind() -> Option<usize> {
    let k = kinds();
    if let Ok(id) = std::env::var("PSO2_ETRIAL_KIND") {
        return k.iter().position(|d| d.trial_id == id);
    }
    let total: u32 = k.iter().map(|d| d.weight).sum();
    if total == 0 {
        return None;
    }
    let mut r = rand::random::<u32>() % total;
    k.iter().position(|d| {
        if r < d.weight {
            true
        } else {
            r -= d.weight;
            false
        }
    })
}

impl Def {
    /// (meseta, exp) for a success at the quest's enemy level. EXP: log-log interpolation of `exp_points`,
    /// extended past both ends with the nearest segment.
    pub fn reward(&self, level: u32) -> (u32, u32) {
        let meseta = (self.meseta_per_level * level.max(1)) as f32 * self.reward_scale;
        let p = &self.exp_points;
        let exp = if p.len() < 2 {
            0.0
        } else {
            let lv = level.max(1) as f64;
            let i = p.windows(2).position(|w| lv <= w[1].0 as f64).unwrap_or(p.len() - 2);
            let ((l0, e0), (l1, e1)) = (p[i], p[i + 1]);
            let (l0, e0, l1, e1) = ((l0 as f64).ln(), (e0 as f64).ln(), (l1 as f64).ln(), (e1 as f64).ln());
            (e0 + (lv.ln() - l0) * (e1 - e0) / (l1 - l0)).exp()
        };
        (meseta.round() as u32, (exp * self.reward_scale as f64).round() as u32)
    }

    pub fn start_params(&self, obj_id: u32, map_id: u16, enemy_name: &str, count: u32) -> StartParams {
        let npc = self.npcs.get(rand::random::<usize>() % self.npcs.len().max(1)).copied().unwrap_or(0);
        StartParams {
            obj_id,
            map_id,
            trial_id: self.trial_id.clone(),
            name_key: self.name_key.clone(),
            abstract_key: self.abstract_key.clone(),
            fail_keys: self.fail_keys.clone(),
            pass_keys: self.pass_keys.clone(),
            begin_key: self.begin_key.clone(),
            key12: self.begin_msg_key.clone(),
            flags: self.flags,
            op: npc,
            disp: self.color,
            b3e: self.kind,
            slots: [self.slot.map(|(k, s)| (k, s, 0, count)), None],
            args: if self.name_arg { vec![text_arg(enemy_name, 12)] } else { vec![] },
            ..Default::default()
        }
    }
}

/// A running trial in a zone: the enemies it spawned that are still alive.
#[derive(Debug, Clone)]
pub struct Active {
    pub obj_id: u32,
    /// index in `kinds()`
    pub kind: usize,
    pub total: u32,
    pub enemies: Vec<u32>,
    pub started: std::time::Instant,
}

/// `PSO2_ETRIAL`: `off` (never), `force` (every chunk opened, still within the caps) or a chance 0..1;
/// unset = the definition's chance.
pub fn chance(def: &Def) -> f32 {
    match std::env::var("PSO2_ETRIAL").as_deref() {
        Ok("off") => 0.0,
        Ok("force") => 1.0,
        Ok(v) => v.parse().unwrap_or(def.chance),
        Err(_) => def.chance,
    }
}

/// Fails the trial after `secs` unless it ended before (the map may be gone by then).
pub fn spawn_timeout(map: std::sync::Weak<crate::mutex::Mutex<crate::map::Map>>, zone_pos: usize, obj_id: u32, secs: u64) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
        if let Some(map) = map.upgrade() {
            if let Err(e) = map.lock().await.etrial_end(zone_pos, obj_id, false).await {
                log::warn!("[pso2-etrial] timeout of {obj_id}: {e}");
            }
        }
    });
}
