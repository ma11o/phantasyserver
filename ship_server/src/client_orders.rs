//! [pso2_vita_offline] Client orders (CO): definitions from the client's Lua (`data/client_orders.json`, made by
//! `tools/server_client_orders.py`) and the character's taken / cleared orders.
//!
//! Only orders whose targets the server can count are in the file: enemy kills (by the enemy's internal name,
//! `NativeBeast`, ...), items handed in (counted in the bag, taken on report) and quest clears (quest name_id, with
//! a difficulty condition).
use pso2packetlib::protocol::{
    items::ItemId,
    models::character::{ClassInfo, ClassLevel},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::OnceLock};

/// The client's TargetType: 8 = kill, 1 = item, 2 = quest clear
#[derive(Debug, Deserialize)]
pub struct Target {
    #[serde(rename = "type")]
    pub kind: i32,
    #[serde(default)]
    pub enemy: String,
    #[serde(default)]
    pub item: Vec<i32>,
    #[serde(default)]
    pub quest: u32,
    /// difficulty (0 = N, 1 = H, 2 = VH, 3 = SH); -1 = any
    #[serde(default = "minus_one")]
    pub rank: i32,
    /// 1 = at least `rank`, 0 = exactly `rank` (unverified), -1 = any
    #[serde(default = "minus_one")]
    pub rank_cond: i32,
    pub num: u32,
}

fn minus_one() -> i32 {
    -1
}

impl Target {
    pub fn item_id(&self) -> ItemId {
        ItemId {
            item_type: self.item.first().copied().unwrap_or(0) as u16,
            id: self.item.get(1).copied().unwrap_or(0) as u16,
            subid: self.item.get(3).copied().unwrap_or(0) as u16,
            ..Default::default()
        }
    }
    fn quest_matches(&self, quest: u32, diff: u32) -> bool {
        self.kind == 2
            && self.quest == quest
            && match self.rank_cond {
                1 => diff as i32 >= self.rank,
                0 => diff as i32 == self.rank,
                _ => true,
            }
    }
}

#[derive(Debug, Deserialize)]
pub struct Reward {
    /// 1 = item, 2 = meseta, 3 = EXP (min / max / correction), 5 = SP of class `job`, others not handled yet
    #[serde(rename = "type")]
    pub kind: i32,
    pub item: Vec<i32>,
    pub num: i32,
    pub min: i32,
    pub max: i32,
    pub correction: i32,
    #[serde(default = "minus_one")]
    pub job: i32,
}

/// One occurrence condition (the client's OrderOccur). `cond` 0 = must hold, 1 = one of the `cond` 1 group must hold
/// (unverified reading)
#[derive(Debug, Deserialize)]
pub struct Occur {
    /// 1 = order `num` cleared, 2 = quest `quest` cleared, 5 = class `job` (-1 = main) at level `num` or more,
    /// 7 = character flag `flag_id` is `state`, others (6 = NPC favour, 9 = ...) are taken as met
    #[serde(rename = "type")]
    pub kind: i32,
    pub cond: i32,
    pub num: i32,
    pub job: i32,
    pub quest: i32,
    pub flag: String,
    pub flag_id: i32,
    pub state: i32,
}

#[derive(Debug, Deserialize)]
pub struct Order {
    pub id: u32,
    pub npc: String,
    pub name: String,
    /// hours before it can be taken again after the report; < 0 = once only (the client's RevivalTime, T95 reading)
    #[serde(default = "minus_one_f")]
    pub revival: f64,
    pub targets: Vec<Target>,
    pub rewards: Vec<Reward>,
    #[serde(default)]
    pub occur: Vec<Occur>,
}

fn minus_one_f() -> f64 {
    -1.0
}

#[derive(Debug, Deserialize)]
pub struct OpenFlag {
    pub id: u32,
}

#[derive(Debug, Deserialize)]
pub struct OrderFile {
    /// NPC -> character flag that opens its order window (CharaFlag_coOpen*)
    pub open_flags: HashMap<String, OpenFlag>,
    pub orders: Vec<Order>,
}

pub fn table() -> Option<&'static OrderFile> {
    static DATA: OnceLock<Option<OrderFile>> = OnceLock::new();
    DATA.get_or_init(|| {
        let path = std::env::var_os("PSO2_CLIENT_ORDERS").unwrap_or_else(|| "data/client_orders.json".into());
        let f = std::fs::read(&path).ok().and_then(|b| match serde_json::from_slice::<OrderFile>(&b) {
            Ok(f) => Some(f),
            Err(e) => {
                log::warn!("[pso2-co] {e}");
                None
            }
        });
        match &f {
            Some(f) => log::info!("[pso2-co] loaded {} client orders", f.orders.len()),
            None => log::warn!("[pso2-co] cannot read {}", std::path::Path::new(&path).display()),
        }
        f
    })
    .as_ref()
}

pub fn get(id: u32) -> Option<&'static Order> {
    table()?.orders.iter().find(|o| o.id == id)
}

/// The level entry of class `class` (the client's Job number = `Class` order)
pub fn class_level(c: &ClassInfo, class: i32) -> Option<&ClassLevel> {
    Some(match class {
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
        _ => return None,
    })
}

pub fn class_level_mut(c: &mut ClassInfo, class: i32) -> Option<&mut ClassLevel> {
    Some(match class {
        0 => &mut c.hunter_info,
        1 => &mut c.ranger_info,
        2 => &mut c.force_info,
        3 => &mut c.fighter_info,
        4 => &mut c.gunner_info,
        5 => &mut c.techer_info,
        6 => &mut c.braver_info,
        7 => &mut c.bouncer_info,
        8 => &mut c.challenger_info,
        9 => &mut c.summoner_info,
        10 => &mut c.battle_warrior_info,
        11 => &mut c.hero_info,
        12 => &mut c.phantom_info,
        13 => &mut c.etole_info,
        _ => return None,
    })
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// What the occurrence conditions look at
pub struct Context<'a> {
    pub classes: &'a ClassInfo,
    pub main_level: u32,
    /// story clear marks (`CharData::cleared_quests`)
    pub story_cleared: &'a [u32],
}

/// An order the character has taken: progress per target (kills, quest clears; item targets are counted in the bag)
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TakenOrder {
    pub id: u32,
    pub counts: Vec<u32>,
}

/// Kept with the character (`CharData`)
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientOrders {
    pub taken: Vec<TakenOrder>,
    /// order ids reported at least once
    pub cleared: Vec<u32>,
    /// order id -> unix time of the last report (repeatable orders come back `revival` hours later)
    pub reported_at: HashMap<u32, u64>,
    /// quest name_ids cleared on any difficulty (CO occurrence conditions)
    pub quests: Vec<u32>,
}

impl ClientOrders {
    /// Counts a kill for every taken order with this enemy as a target; returns the orders whose count changed
    pub fn count_kill(&mut self, enemy: &str) -> Vec<u32> {
        self.count(|t| t.kind == 8 && t.enemy == enemy)
    }

    /// Counts a quest clear (name_id, difficulty) for the taken orders; returns the orders whose count changed
    pub fn count_quest(&mut self, quest: u32, diff: u32) -> Vec<u32> {
        if !self.quests.contains(&quest) {
            self.quests.push(quest);
        }
        self.count(|t| t.quest_matches(quest, diff))
    }

    fn count(&mut self, hit: impl Fn(&Target) -> bool) -> Vec<u32> {
        let mut changed = vec![];
        for t in &mut self.taken {
            let Some(o) = get(t.id) else { continue };
            t.counts.resize(o.targets.len(), 0);
            for (i, target) in o.targets.iter().enumerate() {
                if hit(target) && t.counts[i] < target.num {
                    t.counts[i] += 1;
                    changed.push(t.id);
                }
            }
        }
        changed
    }

    /// The progress words of a taken order (1F-04 / 1F-08): counts, item targets from the bag
    pub fn progress(&self, t: &TakenOrder, bag: &crate::inventory::Inventory) -> Vec<u32> {
        let Some(o) = get(t.id) else { return t.counts.clone() };
        o.targets
            .iter()
            .enumerate()
            .map(|(i, target)| match target.kind {
                1 => bag.count_items(target.item_id()).min(target.num),
                _ => t.counts.get(i).copied().unwrap_or(0),
            })
            .collect()
    }

    pub fn done(&self, t: &TakenOrder, bag: &crate::inventory::Inventory) -> bool {
        get(t.id).is_some_and(|o| {
            let p = self.progress(t, bag);
            o.targets.iter().enumerate().all(|(i, target)| p.get(i).copied().unwrap_or(0) >= target.num)
        })
    }

    /// Reported and not back yet: once-only orders forever, repeatable ones until `revival` hours have passed
    pub fn resting(&self, o: &Order, now: u64) -> bool {
        if !self.cleared.contains(&o.id) {
            return false;
        }
        if o.revival < 0.0 {
            return true;
        }
        let at = self.reported_at.get(&o.id).copied().unwrap_or(0);
        now < at + (o.revival * 3600.0) as u64
    }

    /// The order's occurrence conditions hold (the NPC's open flag itself is not looked at: it is what this decides)
    pub fn occurs(&self, o: &Order, ctx: &Context) -> bool {
        let met = |c: &Occur| match c.kind {
            1 => c.num < 0 || self.cleared.contains(&(c.num as u32)),
            2 => c.quest < 0 || self.quests.contains(&(c.quest as u32)) || ctx.story_cleared.contains(&(c.quest as u32)),
            5 => {
                let lv = if c.job < 0 {
                    ctx.main_level
                } else {
                    class_level(ctx.classes, c.job).map(|l| l.level1 as u32).unwrap_or(0)
                };
                lv as i32 >= c.num
            }
            // 7: the CO open flags are what this decides, other character flags (story, events, ...) are not kept by
            // the server; 6 (NPC favour), 9, ...: not read. All taken as met (playable side)
            _ => true,
        };
        let all = o.occur.iter().filter(|c| c.cond != 1).all(met);
        let any = o.occur.iter().filter(|c| c.cond == 1).collect::<Vec<_>>();
        all && (any.is_empty() || any.into_iter().any(met))
    }

    /// The orders to offer at `npc` (occurring, not taken, not resting) and the taken ones there
    pub fn offered(&self, npc: &str, ctx: &Context, now: u64) -> Vec<&'static Order> {
        table()
            .map(|t| {
                t.orders
                    .iter()
                    .filter(|o| o.npc == npc)
                    .filter(|o| self.taken.iter().any(|t| t.id == o.id) || (!self.resting(o, now) && self.occurs(o, ctx)))
                    .collect()
            })
            .unwrap_or_default()
    }
}
