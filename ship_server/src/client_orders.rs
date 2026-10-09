//! [pso2_vita_offline] Client orders (CO): definitions from the client's Lua (`data/client_orders.json`, made by
//! `tools/server_client_orders.py`) and the character's taken / cleared orders.
//!
//! Only orders whose targets are all enemy kills are in the file: the server counts the kills by the enemy's
//! internal name (`NativeBeast`, ...).
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::OnceLock};

#[derive(Debug, Deserialize)]
pub struct Target {
    pub enemy: String,
    pub num: u32,
}

#[derive(Debug, Deserialize)]
pub struct Reward {
    /// 1 = item, 2 = meseta, 3 = EXP (min / max / correction), others not handled yet
    #[serde(rename = "type")]
    pub kind: i32,
    pub item: Vec<i32>,
    pub num: i32,
    pub min: i32,
    pub max: i32,
    pub correction: i32,
}

#[derive(Debug, Deserialize)]
pub struct Order {
    pub id: u32,
    pub npc: String,
    pub name: String,
    pub targets: Vec<Target>,
    pub rewards: Vec<Reward>,
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
        let f = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<OrderFile>(&b).ok());
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

/// An order the character has taken: kills counted per target
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TakenOrder {
    pub id: u32,
    pub counts: Vec<u32>,
}

impl TakenOrder {
    pub fn done(&self) -> bool {
        get(self.id).is_some_and(|o| {
            o.targets.iter().enumerate().all(|(i, t)| self.counts.get(i).copied().unwrap_or(0) >= t.num)
        })
    }
}

/// Kept with the character (`CharData`)
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientOrders {
    pub taken: Vec<TakenOrder>,
    /// order ids reported (cleared once; taking them again is not offered, provisional)
    pub cleared: Vec<u32>,
}

impl ClientOrders {
    /// Counts a kill for every taken order with this enemy as a target; returns the orders whose count changed
    pub fn count_kill(&mut self, enemy: &str) -> Vec<u32> {
        let mut changed = vec![];
        for t in &mut self.taken {
            let Some(o) = get(t.id) else { continue };
            t.counts.resize(o.targets.len(), 0);
            for (i, target) in o.targets.iter().enumerate() {
                if target.enemy == enemy && t.counts[i] < target.num {
                    t.counts[i] += 1;
                    changed.push(t.id);
                }
            }
        }
        changed
    }
}
