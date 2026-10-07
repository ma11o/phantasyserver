//! [pso2_vita_offline] Item lab (Monica, Npc_CounterLab1): the menu and old-type weapon grinding.
//!
//! Talking to her, the client opens the lab menu with every server-backed entry greyed out and sends eight empty
//! requests (0F-3B, 0F-C9, 0F-28, 0F-34, 0F-64, 0F-A3, 0F-A1, 0F-F6). Each answer is handed to the menu (listener
//! slots of g_player_inventory+0x3bac), which keeps it and re-checks its entries (client 0x82582196 etc.):
//! "アイテム強化" needs the answers to 0F-3B (0F-3C), 0F-C9 (0F-CA) and 0F-64 (0F-65), each with a non-zero first
//! field (the receivers drop it otherwise). Only those three are answered here.
//!
//! The grind screen takes 0F-3C as its table: the grind limit and, per ★ range, the meseta of one grind. Picking a
//! weapon sends 0F-3D; 0F-3E answers with the rate, grinders and risk for the confirm screen. Confirming sends 0F-23;
//! the answer is 0F-24 (new meseta, the weapon as it is now, the grinder stack's amount left) and then 0F-78 (success
//! or failure), which ends the waiting animation and shows the result (the client tells "dropped" from "unchanged" by
//! the grind before/after).
//! Rates, risk and grinders per step are research values; meseta is provisional (`data/grind.json`,
//! `tools/server_grind.py`).
use super::HResult;
use crate::{Action, Error, User, mutex::MutexGuard};
use pso2packetlib::protocol::{
    Flags, Packet, PacketHeader, ProtocolRW,
    items::{InventoryMesetaPacket, Item, ItemId, ItemType, PotentialListPacket},
};
use rand::Rng;
use serde::Deserialize;
use std::sync::OnceLock;

fn raw(subid: u16, flag: Flags, data: Vec<u8>) -> Packet {
    Packet::Unknown((PacketHeader::new(0x0F, subid, flag), data))
}

/// variable-length count as the client reads it: `(raw ^ xor) - sub`
fn count(n: u32, xor: u32, sub: u32) -> [u8; 4] {
    ((n + sub) ^ xor).to_le_bytes()
}

#[derive(Debug, Deserialize)]
struct Step {
    from: u8,
    success: u32,
    drop_min: u8,
    drop_max: u8,
    grinders: u16,
}

#[derive(Debug, Deserialize)]
struct Group {
    rarity_min: u8,
    rarity_max: u8,
    meseta: u32,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
struct GrindFile {
    grinder: [u16; 3],
    max_grind: u8,
    groups: Vec<Group>,
}

fn table() -> Option<&'static GrindFile> {
    static DATA: OnceLock<Option<GrindFile>> = OnceLock::new();
    DATA.get_or_init(|| {
        let path = std::env::var_os("PSO2_GRIND").unwrap_or_else(|| "data/grind.json".into());
        let f = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<GrindFile>(&b).ok());
        match &f {
            Some(f) => log::info!("[pso2-lab] loaded grind table, {} rarity groups", f.groups.len()),
            None => log::warn!("[pso2-lab] cannot read {}", std::path::Path::new(&path).display()),
        }
        f
    })
    .as_ref()
}

fn grinder_id(t: &GrindFile) -> ItemId {
    ItemId {
        item_type: t.grinder[0],
        id: t.grinder[1],
        subid: t.grinder[2],
        ..Default::default()
    }
}

/// 0F-3B -> 0F-3C (client reader 0x81205e8e, ctor 0x82e03e60): u32 (non-zero), Vec<u32> values, Vec<[lo, hi, n]>
/// weapon ranges, Vec<[lo, hi, n]> unit ranges (counts (n + 0xCB) ^ 0xE306, each range takes the next n values),
/// u32 grind limit, u32 (unknown, 0). The client's fee (0x83462928) is X x values[min(X - 1, 5)] of the range holding
/// the item's ★, X from the item attributes (1 for a ソード, every grind level), so each range gets 6 copies of its fee.
pub async fn open_3b(user: &mut User) -> HResult {
    let tbl = table().ok_or(Error::InvalidInput("no grind table"))?;
    let mut d = 1u32.to_le_bytes().to_vec();
    d.extend(count(tbl.groups.len() as u32 * 6, 0xE306, 0xCB));
    for g in &tbl.groups {
        for _ in 0..6 {
            d.extend(g.meseta.to_le_bytes());
        }
    }
    let mut ranges = count(tbl.groups.len() as u32, 0xE306, 0xCB).to_vec();
    for g in &tbl.groups {
        ranges.extend([g.rarity_min, g.rarity_max, 6]);
    }
    while ranges.len() % 4 != 0 {
        ranges.push(0);
    }
    d.extend(ranges);
    // units: none
    d.extend(count(0, 0xE306, 0xCB));
    d.extend((tbl.max_grind as u32).to_le_bytes());
    d.extend(0u32.to_le_bytes());
    user.send_packet(&raw(0x3C, Flags::PACKED, d)).await?;
    Ok(Action::Nothing)
}

/// 0F-C9 -> 0F-CA, fixed 0x64 B (client ctor 0x82e03a0c): u8 (non-zero), 4 x u8, u8, u8, 15 B, 15 B, pad, 15 x u32.
/// The grind screen did not show any of it (marker values tried); zeros.
pub async fn open_c9(user: &mut User) -> HResult {
    let mut d = vec![0u8; 0x64];
    d[0] = 1;
    user.send_packet(&raw(0xCA, Flags::default(), d)).await?;
    Ok(Action::Nothing)
}

/// 0F-64 -> 0F-65 PotentialList (the library's layout matches the Vita reader 0x81209880); unk1 non-zero
pub async fn open_64(user: &mut User) -> HResult {
    user.send_packet(&Packet::PotentialList(PotentialListPacket {
        unk1: 1,
        ..Default::default()
    }))
    .await?;
    Ok(Action::Nothing)
}

/// The weapon, its ★, the step from its grind and the fee
fn lookup(user: &User, uuid: u64) -> Result<Option<(Item, u8, &'static Step, u64)>, Error> {
    let tbl = table().ok_or(Error::InvalidInput("no grind table"))?;
    let item = user
        .character
        .as_ref()
        .ok_or(Error::InvalidInput("no character"))?
        .inventory
        .get_inv_item(uuid)?;
    let ItemType::Weapon(w) = &item.data else {
        return Ok(None);
    };
    let rarity = user
        .blockdata
        .server_data
        .item_params
        .attrs
        .weapons
        .iter()
        .find(|a| a.id == item.id.id && a.subid == item.id.subid)
        .map(|a| a.rarity)
        .unwrap_or(1);
    let from = w.grind;
    let Some(group) = tbl.groups.iter().find(|g| (g.rarity_min..=g.rarity_max).contains(&rarity)) else {
        return Ok(None);
    };
    let Some(step) = group.steps.iter().find(|s| s.from == from).filter(|_| from < tbl.max_grind) else {
        return Ok(None);
    };
    Ok(Some((item, rarity, step, group.meseta as u64)))
}

fn uuid_of(data: &[u8], what: &'static str) -> Result<u64, Error> {
    Ok(u64::from_le_bytes(data.get(0..8).ok_or(Error::InvalidInput(what))?.try_into().unwrap()))
}

/// 0F-3D (weapon uuid u64, support items) -> 0F-3E, fixed 0x18 B (client handler 0x82576db2, confirm screen
/// 0x8255fbcc / 0x8255d4c2): u16 non-zero, u16 success rate (the client shows it as value / 500 %: 50000 = 100 %),
/// i8 grinders needed, i8 max risk (levels lost on failure), u16 (unknown, 0), ItemId of the material (the client
/// counts it in the bag), 8 B (unknown).
pub async fn preview_3d(user: &mut User, data: Vec<u8>) -> HResult {
    let uuid = uuid_of(&data, "0F-3D")?;
    let tbl = table().ok_or(Error::InvalidInput("no grind table"))?;
    let mut d = vec![0u8; 0x18];
    match lookup(user, uuid)? {
        Some((item, rarity, step, cost)) => {
            log::info!(
                "[pso2-lab] preview {}:{}:{} (rarity {rarity}) +{}: {}%, {} grinders, risk {}, {cost} meseta",
                item.id.item_type,
                item.id.id,
                item.id.subid,
                step.from,
                step.success,
                step.grinders,
                step.drop_max
            );
            d[0] = 1;
            d[2..4].copy_from_slice(&((step.success * 500).min(u16::MAX as u32) as u16).to_le_bytes());
            d[4] = step.grinders as u8;
            d[5] = step.drop_max;
            let g = grinder_id(tbl);
            d[8..10].copy_from_slice(&g.item_type.to_le_bytes());
            d[10..12].copy_from_slice(&g.id.to_le_bytes());
            d[12..14].copy_from_slice(&g.unk3.to_le_bytes());
            d[14..16].copy_from_slice(&g.subid.to_le_bytes());
        }
        // u16 0 = refused (the client's error path)
        None => log::warn!("[pso2-lab] preview {uuid:#x}: no grind step"),
    }
    user.send_packet(&raw(0x3E, Flags::default(), d)).await?;
    Ok(Action::Nothing)
}

/// 0F-23 grind request (client reader 0x812023c0): weapon uuid u64, Vec<ItemId> support items (count
/// (n + 0x75) ^ 0xD380), u32, u16, u16. Support items are not used.
pub async fn grind(mut user_guard: MutexGuard<'_, User>, data: Vec<u8>) -> HResult {
    let user: &mut User = &mut user_guard;
    let uuid = uuid_of(&data, "0F-23")?;
    let tbl = table().ok_or(Error::InvalidInput("no grind table"))?;
    let grinder = grinder_id(tbl);
    let Some((item, rarity, step, cost)) = lookup(user, uuid)? else {
        log::warn!("[pso2-lab] grind {uuid:#x}: no grind step");
        return grind_result(user, false).await;
    };
    let from = step.from;
    let inv = &mut user.character.as_mut().ok_or(Error::InvalidInput("no character"))?.inventory;
    if inv.count_items(grinder) < step.grinders as u32 || inv.meseta() < cost {
        log::warn!(
            "[pso2-lab] grind +{from}: short (grinders {} / {}, meseta {} / {cost})",
            inv.count_items(grinder),
            step.grinders,
            inv.meseta()
        );
        return grind_result(user, false).await;
    }
    inv.take_meseta(cost);
    let taken = inv.take_items(grinder, step.grinders);
    let (ok, to) = {
        let mut rng = rand::thread_rng();
        let ok = rng.gen_range(0..100) < step.success;
        (ok, if ok { from + 1 } else { from.saturating_sub(rng.gen_range(step.drop_min..=step.drop_max)) })
    };
    let new_item = {
        let it = inv.get_inv_item_mut(uuid).ok_or(Error::InvalidInput("grind item"))?;
        if let ItemType::Weapon(w) = &mut it.data {
            w.grind = to;
        }
        it.clone()
    };
    let meseta = inv.meseta();
    log::info!(
        "[pso2-lab] grind {}:{}:{} (rarity {rarity}) +{from} -> +{to} ({}, {}%), -{cost} meseta, -{} grinders",
        item.id.item_type,
        item.id.id,
        item.id.subid,
        if ok { "success" } else { "failure" },
        step.success,
        step.grinders
    );

    // 0F-24 (client reader 0x812025b6, receiver 0x82d4516c): u64 meseta, Item 0x38 B, u64 grinder stack uuid,
    // u32 its amount left, 3 x u8 (unknown), i8 place (0 = bag, 1 = storage, < 0 = error), Vec<u64 uuid, i16 left,
    // u16> other touched stacks, Vec<ItemId, u32, u32> (count (n + 0xC0) ^ 0x593E)
    let (stack_uuid, left) = taken.first().copied().unwrap_or((0, 0));
    let mut d = meseta.to_le_bytes().to_vec();
    d.extend(item_bytes(&new_item));
    d.extend(stack_uuid.to_le_bytes());
    d.extend((left as u32).to_le_bytes());
    d.extend([0, 0, 0, 0]);
    d.extend(count(taken.len().saturating_sub(1) as u32, 0x593E, 0xC0));
    for (u, n) in taken.iter().skip(1) {
        d.extend(u.to_le_bytes());
        d.extend((*n as i16).to_le_bytes());
        d.extend([0, 0]);
    }
    d.extend(count(0, 0x593E, 0xC0));
    user.send_packet(&raw(0x24, Flags::PACKED, d)).await?;
    user.send_packet(&Packet::InventoryMeseta(InventoryMesetaPacket { meseta })).await?;
    crate::battle_stats::PlayerStats::update(user)?;
    grind_result(user, ok).await
}

/// 0F-78 (client reader 0x8120b056, receiver 0x82d45528 -> grind screen 0x82579ae8): Vec<Item 0x38 B + u32> (count
/// (n + 0x66) ^ 0x3B7A; extra items handed out, none), u32 result (0 = failure, else success). Ends the waiting
/// animation; the result screen (0x8257a0ea) compares the weapon's grind with the one before.
async fn grind_result(user: &mut User, ok: bool) -> HResult {
    let mut d = count(0, 0x3B7A, 0x66).to_vec();
    d.extend((ok as u32).to_le_bytes());
    user.send_packet(&raw(0x78, Flags::PACKED, d)).await?;
    Ok(Action::Nothing)
}

/// An `Item` as the client reads it (0x38 B: uuid, ItemId, 0x28 B data), through the library's own writer
pub(super) fn item_bytes(item: &Item) -> Vec<u8> {
    let p = Packet::AddedItem(pso2packetlib::protocol::items::AddedItemPacket {
        item: item.clone(),
        ..Default::default()
    })
    .write(pso2packetlib::protocol::PacketType::Vita);
    // header 8 B, then the item
    p[8..8 + 0x38].to_vec()
}
