//! [pso2_vita_offline] Client orders (CO) at the lobby NPCs (definitions: crate::client_orders).
//!
//! The order window opens only when the NPC's open flag (character flag CharaFlag_coOpen*) is 1. Opening it sends
//! 1F-01 (taken orders, `unk4` = the client's request number) and 1F-02 (the NPC's orders, `source` = NPC name,
//! `unk3` = request number). The answers 1F-08 / 1F-03 end their lists with an entry of -1s (client receivers
//! 0x82effac2 / 0x82f0107a) and carry flags (1 = clear the client's list first, 2 = last part: answer the request
//! number given next to it).
use super::HResult;
use crate::{Action, User, client_orders, mutex::MutexGuard};
use pso2packetlib::protocol::{
    Flags, Packet, PacketHeader,
    items::ItemType,
    playerstatus::GainedEXPPacket,
    orders::{ClientOrder, OrderListPacket, OrderListRequestPacket, OrderStatus, TakenOrdersPacket, TakenOrdersRequestPacket},
};

const END: ClientOrder = ClientOrder {
    unk1: u32::MAX,
    id: u32::MAX,
    status: u32::MAX,
    finish_date: u32::MAX,
};

fn padded<T: Copy>(mut v: Vec<T>, n: usize, fill: T) -> Vec<T> {
    v.truncate(n);
    v.resize(n, fill);
    v
}

pub async fn taken(user: &mut User, data: TakenOrdersRequestPacket) -> HResult {
    log::info!("[pso2-co] 1F-01 {data:?}");
    // orders no longer in data/client_orders.json are dropped
    if let Some(c) = user.character.as_mut() {
        c.client_orders.taken.retain(|t| client_orders::get(t.id).is_some());
    }
    let taken = user.character.as_ref().map(|c| c.client_orders.taken.clone()).unwrap_or_default();
    let orders: Vec<_> = taken
        .iter()
        .map(|t| ClientOrder {
            unk1: 0,
            id: t.id,
            status: 4,
            finish_date: 0,
        })
        .collect();
    let statues: Vec<_> = taken
        .iter()
        .map(|t| OrderStatus {
            unk1: t.counts.first().copied().unwrap_or(0),
            unk2: t.counts.get(1).copied().unwrap_or(0),
            unk3: t.counts.get(2).copied().unwrap_or(0),
            ..Default::default()
        })
        .collect();
    // 1F-06 (0x64C B, receiver 0x82effcd2): flags u32 (2 = clear first, 1 = loaded), 100 x (kind, id, state, 0)
    // ended by -1s. Its list (manager, 0x82d260ce) is what the window counts as taken ("(n/60)")
    let mut b = vec![];
    put(&mut b, 3);
    for i in 0..100 {
        let v = match taken.get(i) {
            Some(t) => [0, t.id, 4, 0],
            None => [u32::MAX; 4],
        };
        for w in v {
            put(&mut b, w);
        }
    }
    user.send_packet(&raw(0x06, b)).await?;
    let packet = TakenOrdersPacket {
        user: user.create_object_header(),
        orders: padded(orders, 50, END).into(),
        statues: padded(statues, 50, OrderStatus::default()).into(),
        unk1: 0,
        unk2: data.unk4,
        unk3: 3,
    };
    user.send_packet(&Packet::TakenOrders(packet)).await?;
    Ok(Action::Nothing)
}

pub async fn list(user: &mut User, data: OrderListRequestPacket) -> HResult {
    log::info!("[pso2-co] 1F-02 {data:?}");
    let npc = data.source.to_string();
    let (taken, cleared) = user
        .character
        .as_ref()
        .map(|c| {
            (
                c.client_orders.taken.iter().map(|t| t.id).collect::<Vec<_>>(),
                c.client_orders.cleared.clone(),
            )
        })
        .unwrap_or_default();
    let orders: Vec<_> = client_orders::table()
        .map(|t| t.orders.iter().filter(|o| o.npc == npc).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|o| !cleared.contains(&o.id))
        .map(|o| ClientOrder {
            unk1: 0,
            id: o.id,
            // the row's state (client list builder 0x820cbffe): 2 / 3 open the accept dialog on ○, 4 = taken
            // (○ reports it), 0 does nothing
            status: if taken.contains(&o.id) { 4 } else { 2 },
            finish_date: 0,
        })
        .collect();
    log::info!("[pso2-co] {npc}: {} orders", orders.len());
    let packet = OrderListPacket {
        user: user.create_object_header(),
        orders: padded(orders, 100, END).into(),
        unk1: 3,
        unk2: data.unk3,
    };
    user.send_packet(&Packet::OrderList(packet)).await?;
    Ok(Action::Nothing)
}

fn raw(subid: u16, body: Vec<u8>) -> Packet {
    Packet::Unknown((PacketHeader::new(0x1F, subid, Flags::default()), body))
}

fn put(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

/// 1F-05 (0x28 B, client receiver 0x82effd5e): result u32 (0 = accepted, 1 = rejected / dropped, 2 = reward,
/// 3 = already taken, ...), kind u32, order id u32, u32 (not read), request number u32, the two words handed to
/// the request's callback, flags u32 (bit 2: no order name in the message)
fn result_packet(result: u32, kind: u32, id: u32, request: u32) -> Packet {
    let mut b = vec![];
    for v in [result, kind, id, 0, request, result, u32::MAX, 0] {
        put(&mut b, v);
    }
    raw(0x05, b)
}

/// 1F-04 (0x40 B, receiver in 0x82f01110): the player, one taken order (kind, id, row state 4, 0) and its 24 B
/// status, request number (-1 = none). The status words are the kill counts per target (provisional reading)
pub fn status_packet(user: &User, kind: u32, t: &client_orders::TakenOrder) -> Packet {
    let o = user.create_object_header();
    let mut b = vec![];
    put(&mut b, o.id);
    put(&mut b, o.unk);
    b.extend_from_slice(&(o.entity_type as u16).to_le_bytes());
    b.extend_from_slice(&o.map_id.to_le_bytes());
    for v in [kind, t.id, 4, 0] {
        put(&mut b, v);
    }
    for i in 0..6 {
        put(&mut b, t.counts.get(i).copied().unwrap_or(0));
    }
    put(&mut b, u32::MAX);
    raw(0x04, b)
}

/// 1F-00 (0x38 B, senders 0x82d24c36 / 0x82d24cac / 0x82d24d22 / 0x82d24da2): action u32 (0 = take, 1 = drop,
/// 2 = report), kind u32, order id u32, 24 B status, two request numbers (the second answers in 1F-05), u32
pub async fn action(mut user_guard: MutexGuard<'_, User>, data: Vec<u8>) -> HResult {
    let user = &mut *user_guard;
    let word = |i: usize| data.get(i * 4..i * 4 + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).unwrap_or(u32::MAX);
    let (act, kind, id, ui_request, request) = (word(0), word(1), word(2), word(9), word(10));
    log::info!("[pso2-co] 1F-00 action {act} kind {kind} order {id} requests {ui_request} {request}");
    let r = action_inner(user, act, kind, id, request).await;
    // 1F-07 (u32): answers the first request number (the client's table at manager +0xd4, receiver 0x82effb92
    // via 0x82d25358). Without it the NPC talk stays open after the dialog (T92)
    let mut b = vec![];
    put(&mut b, ui_request);
    user.send_packet(&raw(0x07, b)).await?;
    r
}

async fn action_inner(user: &mut User, act: u32, kind: u32, id: u32, request: u32) -> HResult {
    let Some(order) = client_orders::get(id) else {
        log::warn!("[pso2-co] order {id} is not in data/client_orders.json");
        user.send_packet(&result_packet(1, kind, id, request)).await?;
        return Ok(Action::Nothing);
    };
    let level = user.character.as_ref().map(|c| c.character.get_level().level1 as u32).unwrap_or(1);
    let Some(character) = user.character.as_mut() else { return Ok(Action::Nothing) };
    let orders = &mut character.client_orders;
    match act {
        0 => {
            if orders.taken.iter().any(|t| t.id == id) {
                user.send_packet(&result_packet(3, kind, id, request)).await?;
                return Ok(Action::Nothing);
            }
            let t = client_orders::TakenOrder { id, counts: vec![0; order.targets.len()] };
            orders.taken.push(t.clone());
            log::info!("[pso2-co] taken {id} {}", order.name);
            user.send_packet(&result_packet(0, kind, id, request)).await?;
            let p = status_packet(user, kind, &t);
            user.send_packet(&p).await?;
        }
        1 => {
            orders.taken.retain(|t| t.id != id);
            log::info!("[pso2-co] dropped {id}");
            user.send_packet(&result_packet(1, kind, id, request)).await?;
        }
        2 => {
            let Some(pos) = orders.taken.iter().position(|t| t.id == id) else {
                user.send_packet(&result_packet(1, kind, id, request)).await?;
                return Ok(Action::Nothing);
            };
            if !orders.taken[pos].done() {
                log::info!("[pso2-co] report {id}: not done {:?}", orders.taken[pos].counts);
                user.send_packet(&result_packet(1, kind, id, request)).await?;
                return Ok(Action::Nothing);
            }
            orders.taken.remove(pos);
            if !orders.cleared.contains(&id) {
                orders.cleared.push(id);
            }
            let mut packets = vec![];
            let mut exp = 0;
            for r in &order.rewards {
                match r.kind {
                    // meseta
                    2 if r.num > 0 => packets.push(character.inventory.add_meseta(r.num as u64)),
                    // EXP: min + correction x level, at most max (matches the client's order window, T92)
                    3 => exp += (r.min.max(0) as u32 + r.correction.max(0) as u32 * level).min(r.max.max(0) as u32),
                    // consumable item (type 3). The Lua's ItemId is (type, id, 0, subid), as items.csv: グラインダー
                    // = [3, 7, 0, 1] = 3:7:1
                    1 if r.item.len() == 4 && r.item[0] == 3 => {
                        let mut item = crate::map::monomate();
                        item.id.item_type = 3;
                        item.id.id = r.item[1] as u16;
                        item.id.subid = r.item[3] as u16;
                        if let ItemType::Consumable(c) = &mut item.data {
                            c.amount = r.num.max(1) as u16;
                        }
                        packets.push(character.inventory.add_picked_item(item, &mut user.user_data.last_uuid));
                    }
                    k => log::info!("[pso2-co] reward type {k} not handled ({:?} x{})", r.item, r.num),
                }
            }
            log::info!("[pso2-co] report {id} {}: exp {exp}", order.name);
            user.send_packet(&result_packet(2, kind, id, request)).await?;
            for p in packets {
                user.send_packet(&p).await?;
            }
            if exp > 0 {
                let receiver = user.add_exp(exp)?;
                let p = Packet::GainedEXP(GainedEXPPacket { sender: user.create_object_header(), receivers: vec![receiver] });
                user.send_packet(&p).await?;
            }
        }
        _ => log::info!("[pso2-co] action {act} not handled"),
    }
    Ok(Action::Nothing)
}

/// A kill by anyone in the zone counts for every player's taken orders: 1F-04 with the new counts
pub fn count_kill(user: &mut User, enemy: &str) {
    let Some(c) = user.character.as_mut() else { return };
    let changed = c.client_orders.count_kill(enemy);
    let taken: Vec<_> = c.client_orders.taken.iter().filter(|t| changed.contains(&t.id)).cloned().collect();
    for t in taken {
        log::info!("[pso2-co] {enemy} counts for {}: {:?}", t.id, t.counts);
        let p = status_packet(user, 0, &t);
        let _ = user.try_send_packet(&p);
    }
}
