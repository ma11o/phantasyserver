//! [pso2_vita_offline] Skill tree sheets (the Vita client's "SkillTreeSheet" store). None of these packets are in
//! pso2packetlib.
//!
//! - 04-28 (S -> C): two object headers, u32 mode (1 = clear the store and fill it, needed once to create it; 2 =
//!   overwrite the sheets given), then a vec (count `(n + 0xA8) ^ 0x49E3`) of 0xCC-byte records: u8 sheet index (< 44),
//!   3 bytes (0), the 200-byte sheet. Records whose sheet flags lack bit 0 are skipped.
//! - Sheet (200 B): i8 class, u8 flags (bit 0 = exists, bit 1 = the class's applied sheet), u16 SP used, UTF-16 name
//!   (0x40 B), 0x84 B of skill levels, 4 bits per tree cell. Remaining SP shown = the class's `level2` - SP used.
//!   Which nibble belongs to which cell comes from the class's `.skt` (`data/skill_tree.json`, `tools/gen_skill_tree.py`).
//! - 04-27 (C -> S, 0x44 B body): two object headers, u8 op, u8 a, u8 b, u8 c, u32 d, u8 e, u8 (1), u16, [u16; 16] name.
//!   op 2 = learn (a sheet, b x, c y, e levels), 4 = apply (a sheet), 6 = rename (a sheet, name), 7 = add, 8 = reset
//!   (a sheet), 9 = reset all.
//! - 04-2C reply: u8 0x27 (the request's subid), u8 1 = success, u8 error, u32 op.
use pso2packetlib::protocol::{Flags, ObjectHeader, Packet, PacketHeader, models::character::ClassInfo, objects};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::OnceLock};

pub const SHEETS: u8 = 44;
const LEVEL_BYTES: usize = 0x84;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Sheet {
    pub index: u8,
    pub class: u8,
    pub active: bool,
    pub name: String,
    pub used_sp: u16,
    /// 0x84 bytes (sheet + 0x44)
    pub levels: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillTrees {
    pub sheets: Vec<Sheet>,
}

#[derive(Debug, Deserialize)]
pub struct Cell {
    pub skill: String,
    pub x: u8,
    pub y: u8,
    pub byte: usize,
    pub shift: u8,
    pub max: u8,
}

#[derive(Debug, Deserialize)]
struct ClassTree {
    name: String,
    cells: Vec<Cell>,
}

#[derive(Debug, Deserialize, Default)]
struct TreeFile {
    classes: HashMap<String, ClassTree>,
}

fn data() -> &'static TreeFile {
    static DATA: OnceLock<TreeFile> = OnceLock::new();
    DATA.get_or_init(|| {
        let path = std::env::var_os("PSO2_SKILL_TREE").unwrap_or_else(|| "data/skill_tree.json".into());
        let f: TreeFile = match std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            Some(f) => f,
            None => {
                log::warn!("[pso2-skilltree] cannot read {}", std::path::Path::new(&path).display());
                TreeFile::default()
            }
        };
        log::info!("[pso2-skilltree] loaded {} classes", f.classes.len());
        f
    })
}

fn cells(class: u8) -> &'static [Cell] {
    data().classes.get(&class.to_string()).map(|c| c.cells.as_slice()).unwrap_or_default()
}

pub fn class_name(class: u8) -> &'static str {
    data().classes.get(&class.to_string()).map(|c| c.name.as_str()).unwrap_or("")
}

/// Total SP of a class: its `level2` (the client subtracts the sheet's used SP from it)
pub fn class_sp(c: &ClassInfo, class: u8) -> u16 {
    let l = match class {
        0 => &c.hunter_info,
        1 => &c.ranger_info,
        2 => &c.force_info,
        3 => &c.fighter_info,
        4 => &c.gunner_info,
        5 => &c.techer_info,
        6 => &c.braver_info,
        7 => &c.bouncer_info,
        8 => &c.challenger_info,
        9 => &c.summoner_info,
        10 => &c.battle_warrior_info,
        11 => &c.hero_info,
        12 => &c.phantom_info,
        13 => &c.etole_info,
        _ => return 0,
    };
    l.level2
}

impl Sheet {
    fn new(index: u8, class: u8) -> Self {
        Self { index, class, active: true, levels: vec![0; LEVEL_BYTES], ..Default::default() }
    }
    pub fn level(&self, cell: &Cell) -> u8 {
        self.levels.get(cell.byte).map(|b| (b >> cell.shift) & 0xF).unwrap_or(0)
    }
    fn set_level(&mut self, cell: &Cell, lv: u8) {
        self.levels.resize(LEVEL_BYTES, 0);
        let b = &mut self.levels[cell.byte];
        *b = (*b & !(0xF << cell.shift)) | ((lv & 0xF) << cell.shift);
    }
    fn reset(&mut self) {
        self.levels = vec![0; LEVEL_BYTES];
        self.used_sp = 0;
    }
    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&[self.index, 0, 0, 0]);
        let start = out.len();
        out.push(self.class);
        out.push(1 | if self.active { 2 } else { 0 });
        out.extend_from_slice(&self.used_sp.to_le_bytes());
        let mut name: Vec<u16> = self.name.encode_utf16().take(0x1F).collect();
        name.resize(0x20, 0);
        name.iter().for_each(|c| out.extend_from_slice(&c.to_le_bytes()));
        let mut lv = self.levels.clone();
        lv.resize(LEVEL_BYTES, 0);
        out.extend_from_slice(&lv);
        debug_assert_eq!(out.len() - start, 200);
    }
}

impl SkillTrees {
    /// One applied sheet for every class that has a tree and no sheet yet
    pub fn ensure(&mut self) {
        let mut classes: Vec<u8> = data().classes.keys().filter_map(|k| k.parse().ok()).collect();
        classes.sort();
        for class in classes {
            if self.sheets.iter().any(|s| s.class == class) {
                continue;
            }
            if let Some(index) = self.free_index() {
                self.sheets.push(Sheet::new(index, class));
            }
        }
    }
    fn free_index(&self) -> Option<u8> {
        (0..SHEETS).find(|i| !self.sheets.iter().any(|s| s.index == *i))
    }
    fn get_mut(&mut self, index: u8) -> Option<&mut Sheet> {
        self.sheets.iter_mut().find(|s| s.index == index)
    }
    /// Skill name -> Lv from the class's applied sheet; `None` when no SP is spent there (the provisional build is
    /// used then, see skills.rs)
    pub fn levels(&self, class: u8) -> Option<HashMap<&'static str, u32>> {
        let s = self.sheets.iter().find(|s| s.class == class && s.active)?;
        if s.used_sp == 0 {
            return None;
        }
        Some(cells(class).iter().map(|c| (c.skill.as_str(), s.level(c) as u32)).filter(|(_, l)| *l > 0).collect())
    }
}

/// Parsed 04-27 body (after the 8-byte header)
#[derive(Debug, Default)]
pub struct Request {
    pub op: u8,
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u32,
    pub e: u8,
    pub name: String,
}

impl Request {
    pub fn parse(d: &[u8]) -> Option<Self> {
        if d.len() < 0x24 {
            return None;
        }
        let name: Vec<u16> = d.get(0x24..0x44).unwrap_or_default().chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|c| *c != 0).collect();
        Some(Self {
            op: d[0x18],
            a: d[0x19],
            b: d[0x1A],
            c: d[0x1B],
            d: u32::from_le_bytes(d[0x1C..0x20].try_into().ok()?),
            e: d[0x20],
            name: String::from_utf16_lossy(&name),
        })
    }
}

/// Applies a request; returns (success, error code, indices of the sheets to resend)
pub fn apply(trees: &mut SkillTrees, classes: &ClassInfo, r: &Request) -> (bool, u8, Vec<u8>) {
    match r.op {
        2 => {
            let Some(s) = trees.get_mut(r.a) else { return (false, 1, vec![]) };
            let Some(cell) = cells(s.class).iter().find(|c| c.x == r.b && c.y == r.c) else {
                return (false, 2, vec![]);
            };
            let add = r.e.max(1);
            let lv = s.level(cell);
            let total = class_sp(classes, s.class);
            if lv + add > cell.max.max(1) || s.used_sp + add as u16 > total {
                log::info!("[pso2-skilltree] learn {} {lv}+{add} refused (max {}, SP {}/{total})", cell.skill,
                    cell.max, s.used_sp);
                return (false, 3, vec![]);
            }
            s.set_level(cell, lv + add);
            s.used_sp += add as u16;
            log::info!("[pso2-skilltree] sheet {} learn {} Lv{} (SP {}/{total})", s.index, cell.skill, lv + add, s.used_sp);
            (true, 0, vec![s.index])
        }
        4 => {
            let Some(class) = trees.get_mut(r.a).map(|s| s.class) else { return (false, 1, vec![]) };
            let mut idx = vec![];
            for s in trees.sheets.iter_mut().filter(|s| s.class == class) {
                s.active = s.index == r.a;
                idx.push(s.index);
            }
            (true, 0, idx)
        }
        6 => {
            let Some(s) = trees.get_mut(r.a) else { return (false, 1, vec![]) };
            s.name = r.name.clone();
            (true, 0, vec![s.index])
        }
        7 => {
            // a = the class of the new sheet (not checked against a capture)
            let class = if (r.a as usize) < 14 { r.a } else { return (false, 1, vec![]) };
            let Some(index) = trees.free_index() else { return (false, 4, vec![]) };
            let mut s = Sheet::new(index, class);
            s.active = false;
            trees.sheets.push(s);
            (true, 0, vec![index])
        }
        8 => {
            let Some(s) = trees.get_mut(r.a) else { return (false, 1, vec![]) };
            s.reset();
            (true, 0, vec![s.index])
        }
        9 => {
            trees.sheets.iter_mut().for_each(Sheet::reset);
            (true, 0, trees.sheets.iter().map(|s| s.index).collect())
        }
        _ => (false, 1, vec![]),
    }
}

pub fn sheets_packet(player: ObjectHeader, mode: u32, trees: &SkillTrees, only: Option<&[u8]>) -> Packet {
    let sheets: Vec<&Sheet> =
        trees.sheets.iter().filter(|s| only.is_none_or(|o| o.contains(&s.index))).collect();
    let mut data = Vec::with_capacity(0x20 + sheets.len() * 0xCC);
    for h in [&player, &player] {
        data.extend_from_slice(&h.id.to_le_bytes());
        data.extend_from_slice(&h.unk.to_le_bytes());
        data.extend_from_slice(&(h.entity_type as u16).to_le_bytes());
        data.extend_from_slice(&h.map_id.to_le_bytes());
    }
    data.extend_from_slice(&mode.to_le_bytes());
    data.extend_from_slice(&((sheets.len() as u32 + 0xA8) ^ 0x49E3).to_le_bytes());
    sheets.iter().for_each(|s| s.encode(&mut data));
    Packet::Unknown((PacketHeader::new(0x04, 0x28, Flags::PACKED | Flags::OBJECT_RELATED), data))
}

pub fn result_packet(player: ObjectHeader, op: u8, ok: bool, err: u8) -> Packet {
    let mut unk6 = [0u8; 0x10];
    unk6[..4].copy_from_slice(&(op as u32).to_le_bytes());
    Packet::Unk042C(objects::Unk042CPacket {
        unk1: player,
        unk2: player,
        unk3: 0x27,
        unk4: ok as u8,
        unk5: err as u16,
        unk6,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sheet_is_200_bytes() {
        let mut out = vec![];
        Sheet::new(3, 0).encode(&mut out);
        assert_eq!(out.len(), 0xCC);
        assert_eq!(&out[..6], &[3, 0, 0, 0, 0, 3]);
    }
    #[test]
    fn nibbles() {
        let c = Cell { skill: "x".into(), x: 0, y: 0, byte: 1, shift: 4, max: 10 };
        let mut s = Sheet::new(0, 0);
        s.set_level(&c, 7);
        assert_eq!(s.levels[1], 0x70);
        assert_eq!(s.level(&c), 7);
    }
}
