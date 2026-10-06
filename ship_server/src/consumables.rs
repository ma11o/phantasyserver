//! [pso2_vita_offline] Recovery items. Using one, the client plays the item's action (MovementAction, e.g.
//! "MonomateS") and then sends DealDamage from the player to itself with `attack_id` = the hash of the item's skill name
//! in `common.ski` ("Monomate" -> 930535392, see `battle_stats::actor_key` without the abs). The client sends nothing
//! else (no item uuid, no amount) and does not change its HP or the stack by itself: the server heals, takes one off
//! the stack and answers. The amounts are not in the client (T61); `data/consumables.json` (`PSO2_CONSUMABLES`
//! overrides) has them.
use pso2packetlib::protocol::items::ItemId;
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Debug, Deserialize, Clone)]
pub struct Consumable {
    /// skill name in `common.ski`; DealDamage's attack_id is its hash
    pub skill: String,
    /// item id (type, id, subid)
    pub item: [u16; 3],
    /// HP restored, percent of the max HP
    #[serde(default)]
    pub hp_percent: u32,
}

#[derive(Debug, Deserialize, Default)]
struct ConsumableFile {
    #[serde(default)]
    items: Vec<Consumable>,
}

fn file() -> &'static ConsumableFile {
    static DATA: OnceLock<ConsumableFile> = OnceLock::new();
    DATA.get_or_init(|| {
        let path = std::env::var_os("PSO2_CONSUMABLES").unwrap_or_else(|| "data/consumables.json".into());
        let f: ConsumableFile = match std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            Some(f) => f,
            None => {
                log::warn!("[pso2-item] cannot read {}", std::path::Path::new(&path).display());
                ConsumableFile::default()
            }
        };
        log::info!("[pso2-item] loaded {} consumables", f.items.len());
        f
    })
}

/// boost's hash_combine over the signed bytes of the name (the client's name hash)
fn name_hash(name: &str) -> u32 {
    name.bytes().fold(0u32, |h, c| {
        h ^ (c as i8 as i32 as u32)
            .wrapping_add(h << 6)
            .wrapping_add(h >> 2)
            .wrapping_add(0x9e37_79b9)
    })
}

/// The recovery item whose skill name hashes to `attack_id`
pub fn by_attack(attack_id: u32) -> Option<&'static Consumable> {
    file().items.iter().find(|c| name_hash(&c.skill) == attack_id)
}

impl Consumable {
    pub fn item_id(&self) -> ItemId {
        ItemId {
            item_type: self.item[0],
            id: self.item[1],
            subid: self.item[2],
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn monomate_hash() {
        assert_eq!(super::name_hash("Monomate"), 930535392);
    }
}
