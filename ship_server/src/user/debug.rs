//! [pso2_vita_offline] Debug entry points (T22).
//!
//! - `PSO2_DEBUG_START="quest=1100 diff=0 zone=campship_down [wait_ms=1000]"`: after the first lobby load,
//!   accept the quest, transfer into its map (campship) and then move to `zone`. No client input needed.
//! - `PSO2_DEBUG_PORT=<port>`: loopback TCP (one port for all blocks), one command per line, one reply line per command.
//!   Commands: `goto <zone> [x y z]`, `quest <id> <diff>`, `lobby`, `spawn <enemy> [x y z]`,
//!   `send <id> <subid> <flag> <hex>`, `pos`, `tp <x> <y> <z>`, `finish [hide] [ff] [now]`, `result [hide] [ff]`, `help`.
//!   Anything else is run as a `!` chat command.
//! - `pipe <clear|start> [x y z]`: spawn the Story EP1 `oa_telepipe_clear` / `oa_telepipe_start` object (data copied
//!   from 700000) at the player (or x y z), for this player only.
//! - `tag <object id> <attribute>`: send `SetTag` (04-15) for an object of the current zone (e.g. `On` lights a
//!   clear telepipe, T19).
//! - `finish`: same as a final return (03-19): move to `campship` and send `QuestResult` on the next `MapLoaded`
//!   (`now`: right after `MapTransfer` instead). `result`: send `QuestResult` now.
//! - T25: `goto <zone> x y z` and `PSO2_DEBUG_START="... pos=x,y,z"` replace the zone's `default_location`
//!   (position only) for the next spawn into that zone. `tp` sends `TeleportTransfer` (0x04, 0x02) to the player
//!   (the Vita client ignores it in a free field: the position snaps back, T25).
//!
//! Both are disabled unless the environment variable is set. Single player assumed (the last in-game client).
use super::{User, UserState};
use crate::{BlockData, Error, mutex::Mutex};
use pso2packetlib::protocol::{
    Flags, ObjectHeader, ObjectType, Packet, PacketHeader, chat::ChatMessage, models::Position,
    objects::{SetTagPacket, TeleportTransferPacket}, playerstatus::{DealDamagePacket, GainedEXPPacket}, questlist::{AcceptQuestPacket, SetQuestPointsPacket}, spawn::ObjectSpawnPacket,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

/// PC object data -> Vita: property ids 51..79 (even positions) are one lower on the Vita
/// (same rule as `data/maps/lobby/luas/oa_transfer_machine.lua`'s `to_vita`; T19: PC 66 = the client's `get(0x41)`).
pub fn pc_to_vita_object_data(data: &[u32]) -> Vec<u32> {
    let mut d = data.to_vec();
    for v in d.iter_mut().step_by(2) {
        if *v > 50 && *v < 80 {
            *v -= 1;
        }
    }
    d
}

/// Object ids for `pipe` (well above the map data's ids).
static PIPE_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0xF000);

#[derive(Debug, Clone)]
pub struct DebugStart {
    pub quest: u32,
    pub diff: u16,
    pub zone: String,
    pub wait_ms: u64,
    pub pos: Option<(f32, f32, f32)>,
}

/// T25: (zone name, position) used once by the next spawn into that zone (`map.rs`, `Zone::add_player`).
static SPAWN_OVERRIDE: std::sync::Mutex<Option<(String, (f32, f32, f32))>> = std::sync::Mutex::new(None);

pub fn set_spawn_override(zone: &str, p: (f32, f32, f32)) {
    *SPAWN_OVERRIDE.lock().unwrap() = Some((zone.to_string(), p));
}

pub fn apply_spawn_override(zone: &str, pos: &mut Position) {
    let mut o = SPAWN_OVERRIDE.lock().unwrap();
    if o.as_ref().is_some_and(|(z, _)| z == zone) {
        let (_, (x, y, z)) = o.take().unwrap();
        pos.pos_x = half::f16::from_f32(x);
        pos.pos_y = half::f16::from_f32(y);
        pos.pos_z = half::f16::from_f32(z);
        log::info!("[pso2-debug] spawn override {zone} ({x}, {y}, {z})");
    }
}

fn parse_xyz(a: &[&str]) -> Option<(f32, f32, f32)> {
    match a {
        [x, y, z] => Some((x.parse().ok()?, y.parse().ok()?, z.parse().ok()?)),
        _ => None,
    }
}

pub fn debug_start() -> Option<DebugStart> {
    let s = std::env::var("PSO2_DEBUG_START").ok()?;
    let mut ds = DebugStart {
        quest: 1100,
        diff: 0,
        zone: "campship_down".into(),
        wait_ms: 1000,
        pos: None,
    };
    for kv in s.split_whitespace() {
        let Some((k, v)) = kv.split_once('=') else {
            continue;
        };
        match k {
            "quest" => ds.quest = v.parse().ok()?,
            "diff" => ds.diff = v.parse().ok()?,
            "zone" => ds.zone = v.into(),
            "wait_ms" => ds.wait_ms = v.parse().ok()?,
            "pos" => ds.pos = Some(parse_xyz(&v.split(',').collect::<Vec<_>>())?),
            _ => log::warn!("[pso2-debug] unknown PSO2_DEBUG_START key {k}"),
        }
    }
    Some(ds)
}

/// All blocks of this ship (the command port searches every block for the player).
static BLOCKS: std::sync::Mutex<Vec<Arc<BlockData>>> = std::sync::Mutex::new(Vec::new());

/// Saves every user of every block (shutdown: the process exits without dropping them).
pub async fn save_all() {
    let blocks = BLOCKS.lock().unwrap().clone();
    let mut n = 0;
    for b in blocks {
        let clients: Vec<_> = b.clients.lock().await.iter().map(|(_, c)| c.clone()).collect();
        for c in clients {
            let user = c.lock().await;
            if user.character.is_none() {
                continue;
            }
            match user.save().await {
                Ok(()) => n += 1,
                Err(e) => log::warn!("[pso2-save] user {}: {e}", user.user_data.id),
            }
        }
    }
    log::info!("[pso2-save] saved {n} user(s) on shutdown");
}

async fn find_in_game() -> Option<Arc<Mutex<User>>> {
    let blocks = BLOCKS.lock().unwrap().clone();
    for b in blocks {
        if let Some(u) = find_user(&b, None).await {
            return Some(u);
        }
    }
    None
}

async fn find_user(block: &BlockData, conn_id: Option<usize>) -> Option<Arc<Mutex<User>>> {
    let clients = block.clients.lock().await;
    for (id, c) in clients.iter().rev() {
        if let Some(conn_id) = conn_id {
            if *id == conn_id {
                return Some(c.clone());
            }
            continue;
        }
        if c.lock().await.state == UserState::InGame {
            return Some(c.clone());
        }
    }
    None
}

/// Called at the end of `map_loaded` (after `FinishLoading` was sent).
pub async fn on_map_loaded(user: &mut User) {
    let first = !user.debug_started;
    let pending = user.debug_pending.clone();
    if !first && pending.is_none() {
        return;
    }
    let ds = if first { debug_start() } else { None };
    if first {
        user.debug_started = true;
        if ds.is_none() {
            return;
        }
    }
    let block = user.blockdata.clone();
    let conn_id = user.conn_id;
    let wait = ds.as_ref().map(|d| d.wait_ms).unwrap_or(500);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(wait)).await;
        let Some(user) = find_user(&block, Some(conn_id)).await else {
            return;
        };
        let res = if let Some(ds) = ds {
            log::info!("[pso2-debug] start {ds:?}");
            if let Some(p) = ds.pos {
                set_spawn_override(&ds.zone, p);
            }
            match start_quest(&user, ds.quest, ds.diff).await {
                Ok(()) => goto(&user, &ds.zone).await,
                Err(e) => Err(e),
            }
        } else if let Some(zone) = pending {
            user.lock().await.debug_pending = None;
            goto(&user, &zone).await
        } else {
            Ok(String::new())
        };
        match res {
            Ok(m) => log::info!("[pso2-debug] {m}"),
            Err(e) => log::warn!("[pso2-debug] error: {e}"),
        }
    });
}

async fn start_quest(user: &Arc<Mutex<User>>, quest: u32, diff: u16) -> Result<(), Error> {
    let packet = AcceptQuestPacket {
        quest_obj: ObjectHeader {
            id: quest,
            entity_type: ObjectType::Quest,
            ..Default::default()
        },
        diff,
        ..Default::default()
    };
    let lock = user.lock().await;
    let quest = lock
        .blockdata
        .quests
        .get_quest(packet, &lock.blockdata.latest_mapid)?;
    super::handlers::quest::start_quest(lock, quest).await?;
    Ok(())
}

/// Moves the player to `zone`: `lobby`, a zone of the current map, or a zone of the party quest map
/// (enters the quest map first, then moves on the next `MapLoaded`).
async fn goto(user: &Arc<Mutex<User>>, zone: &str) -> Result<String, Error> {
    // lock order as upstream: never hold the user while locking a map
    let (id, map, zone_pos, party) = {
        let lock = user.lock().await;
        (
            lock.get_user_id(),
            lock.get_current_map().ok_or(Error::InvalidInput("goto: no map"))?,
            lock.zone_pos,
            lock.get_current_party(),
        )
    };
    if zone == "lobby" {
        map.lock().await.move_to_lobby(id).await?;
        return Ok("goto lobby".into());
    }
    {
        let mut map_lock = map.lock().await;
        if map_lock.has_zone(zone) {
            if map_lock.zone_name(zone_pos) == Some(zone) {
                return Ok(format!("already in {zone}"));
            }
            map_lock.move_player_named(id, zone).await?;
            return Ok(format!("goto {zone}"));
        }
    }
    let quest_map = match party {
        Some(p) => p.read().await.get_quest_map(),
        None => None,
    };
    let Some(quest_map) = quest_map else {
        return Err(Error::InvalidInput("goto: unknown zone (no quest accepted)"));
    };
    if Arc::ptr_eq(&quest_map, &map) {
        return Err(Error::InvalidInput("goto: unknown zone"));
    }
    let (has, init) = {
        let q = quest_map.lock().await;
        (q.has_zone(zone), q.init_zone_name().map(str::to_string))
    };
    if !has {
        return Err(Error::InvalidInput("goto: zone not in quest map"));
    }
    if init.as_deref() != Some(zone) {
        user.lock().await.debug_pending = Some(zone.to_string());
    }
    let player = map
        .lock()
        .await
        .remove_player(id)
        .await
        .ok_or(Error::InvalidInput("goto: player not in map"))?;
    player.lock().await.set_map(quest_map.clone());
    quest_map.lock().await.init_add_player(player).await?;
    Ok(format!("enter quest map, then {zone}"))
}

fn parse_hex(s: &str) -> Option<Vec<u8>> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn parse_num<T: TryFrom<u64>>(s: &str) -> Option<T> {
    let v = if let Some(h) = s.strip_prefix("0x") {
        u64::from_str_radix(h, 16).ok()?
    } else {
        s.parse().ok()?
    };
    T::try_from(v).ok()
}

fn flags_from(v: u8) -> Flags {
    let mut f = Flags::default();
    for (bit, flag) in [
        (1 << 2, Flags::PACKED),
        (1 << 4, Flags::FLAG_10),
        (1 << 5, Flags::FULL_MOVEMENT),
        (1 << 6, Flags::OBJECT_RELATED),
    ] {
        if v & bit != 0 {
            f |= flag;
        }
    }
    f
}

const HELP: &str = "goto <zone> [x y z] | clear [x y z] | pipe <clear|start> [x y z] | tag <obj id> <attr> | finish [hide] [ff] [now] | result [hide] [ff] [rank=S meseta=N exp=N kills=N score=N/M] | tp <x> <y> <z> | tp pipe | place <obj id> <x> <y> <z> | points <total> [gained] | quest <id> <diff> | lobby | spawn <enemy> [x y z] [boss] | send <id> <subid> <flag> <hex> | ehp <n> | ekill | drop [model|-] [type:id:subid] [x y z] | pos | hp [n] | exp <n> | unlock <name_id>... | <any ! chat command>";

async fn run_command(line: &str) -> Result<String, Error> {
    let mut args = line.split_whitespace();
    let Some(cmd) = args.next() else {
        return Ok(String::new());
    };
    if cmd == "help" {
        return Ok(HELP.into());
    }
    let user = find_in_game()
        .await
        .ok_or(Error::InvalidInput("no in-game player"))?;
    let a: Vec<&str> = args.collect();
    match (cmd, a.as_slice()) {
        ("goto", [zone]) => goto(&user, zone).await,
        ("goto", [zone, xyz @ ..]) => {
            let p = parse_xyz(xyz).ok_or(Error::InvalidInput("goto <zone> [x y z]"))?;
            set_spawn_override(zone, p);
            goto(&user, zone).await
        }
        ("tp", xyz) => {
            // `tp pipe`: 1.6 m short of the clear telepipe (the client checks the distance, ~3 m)
            let xyz: Vec<String> = if let ["pipe"] = xyz {
                let lock = user.lock().await;
                let map = lock.get_current_map().ok_or(Error::InvalidInput("no map"))?;
                let zone = lock.zone_pos;
                drop(lock);
                let p = map.lock().await.clear_pipe_pos(zone).ok_or(Error::InvalidInput("no clear telepipe"))?;
                vec![p.pos_x.to_f32().to_string(), p.pos_y.to_f32().to_string(), (p.pos_z.to_f32() - 1.6).to_string()]
            } else {
                xyz.iter().map(|s| s.to_string()).collect()
            };
            let xyz: Vec<&str> = xyz.iter().map(|s| s.as_str()).collect();
            let (x, y, z) = parse_xyz(&xyz).ok_or(Error::InvalidInput("tp <x> <y> <z> | tp pipe"))?;
            let mut lock = user.lock().await;
            let mut pos = lock.position;
            pos.pos_x = half::f16::from_f32(x);
            pos.pos_y = half::f16::from_f32(y);
            pos.pos_z = half::f16::from_f32(z);
            let id = lock.get_user_id();
            lock.send_packet(&Packet::TeleportTransfer(TeleportTransferPacket {
                source_tele: ObjectHeader {
                    id,
                    entity_type: ObjectType::Player,
                    ..Default::default()
                },
                location: pos,
                ..Default::default()
            }))
            .await?;
            lock.position = pos;
            Ok(format!("tp ({x}, {y}, {z})"))
        }
        ("place", [id, xyz @ ..]) => {
            // Object Teleport Location (04-02) for an object, e.g. the answer to a telepipe "InitPosition"
            let id: u32 = parse_num(id).ok_or(Error::InvalidInput("place <obj id> <x> <y> <z>"))?;
            let (x, y, z) = parse_xyz(xyz).ok_or(Error::InvalidInput("place <obj id> <x> <y> <z>"))?;
            let lock = user.lock().await;
            let map = lock.get_current_map().ok_or(Error::InvalidInput("no map"))?;
            let zone = lock.zone_pos;
            let mut pos = lock.position;
            drop(lock);
            let world_id = map.lock().await.zone_world_id(zone).unwrap_or(0);
            pos.pos_x = half::f16::from_f32(x);
            pos.pos_y = half::f16::from_f32(y);
            pos.pos_z = half::f16::from_f32(z);
            user.lock()
                .await
                .send_packet(&Packet::TeleportTransfer(TeleportTransferPacket {
                    source_tele: ObjectHeader {
                        id,
                        entity_type: ObjectType::Object,
                        map_id: world_id as u16,
                        ..Default::default()
                    },
                    location: pos,
                    ..Default::default()
                }))
                .await?;
            Ok(format!("place {id} ({x}, {y}, {z})"))
        }
        ("lobby", []) => goto(&user, "lobby").await,
        ("pipe", [kind, xyz @ ..]) => {
            // Story EP1 700000 map.json (objects 81 / 82)
            let (name, data): (&str, &[u32]) = match *kind {
                "clear" => ("oa_telepipe_clear", &[
                    1304, 1, 2, 0, 0, 0, 1677721856, 0, 58, u32::MAX, 55, 1, 57, 0, 66, 2, 67, 0, 2, 0, 0, 0,
                ]),
                "start" => ("oa_telepipe_start", &[
                    1304, 1, 2, 0, 0, 0, 1811939584, 1869181801, 58, u32::MAX, 55, 1, 57, 0, 66, 1, 67, 0, 2,
                    0, 0, 0,
                ]),
                _ => return Err(Error::InvalidInput("pipe <clear|start> [x y z]")),
            };
            let lock = user.lock().await;
            let map = lock.get_current_map().ok_or(Error::InvalidInput("no map"))?;
            let zone = lock.zone_pos;
            let mut pos = lock.position;
            if !xyz.is_empty() {
                let (x, y, z) = parse_xyz(xyz).ok_or(Error::InvalidInput("pipe <kind> [x y z]"))?;
                pos.pos_x = half::f16::from_f32(x);
                pos.pos_y = half::f16::from_f32(y);
                pos.pos_z = half::f16::from_f32(z);
            }
            drop(lock);
            let world_id = map.lock().await.zone_world_id(zone).unwrap_or(0);
            let id = PIPE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let packet = ObjectSpawnPacket {
                object: ObjectHeader {
                    id,
                    entity_type: ObjectType::Object,
                    map_id: world_id as u16,
                    ..Default::default()
                },
                position: pos,
                name: name.to_string().into(),
                unk2: [16, 0, 0, 0, 0],
                flags: 4,
                data: pc_to_vita_object_data(data).into(),
                ..Default::default()
            };
            user.lock().await.send_packet(&Packet::ObjectSpawn(packet)).await?;
            Ok(format!(
                "pipe {name} id={id} map_id={world_id} at ({:.2}, {:.2}, {:.2})",
                pos.pos_x.to_f32(),
                pos.pos_y.to_f32(),
                pos.pos_z.to_f32()
            ))
        }
        ("roll", [enemy, rest @ ..]) => {
            // roll <enemy> [n] [area]: rolls the drop tables n times (default 1000, area default area2) and counts the results
            let n: u32 = rest.first().and_then(|s| parse_num(s)).unwrap_or(1000);
            let area = rest.get(1).copied().unwrap_or("area2");
            let level: u32 = rest.get(2).and_then(|s| parse_num(s)).unwrap_or(1);
            let mut counts = std::collections::BTreeMap::<String, (u32, u64)>::new();
            for _ in 0..n {
                for item in crate::drops::roll(enemy, area, level) {
                    let (key, amount) = match crate::map::meseta_amount(&item) {
                        Some(a) => ("meseta".to_string(), a as u64),
                        None => {
                            let a = match &item.data {
                                pso2packetlib::protocol::items::ItemType::Consumable(c) => c.amount as u64,
                                _ => 1,
                            };
                            (format!("{}:{}:{}", item.id.item_type, item.id.id, item.id.subid), a)
                        }
                    };
                    let e = counts.entry(key).or_default();
                    e.0 += 1;
                    e.1 += amount;
                }
            }
            let list: Vec<String> = counts.iter().map(|(k, (c, a))| format!("{k}={c}(sum {a})")).collect();
            Ok(format!("roll {enemy} x{n} in {area}: {}", if list.is_empty() { "nothing".into() } else { list.join(" ") }))
        }
        ("ekill", []) => {
            // The zone's boss to 1 HP, then one player hit (a sword normal attack) through the normal damage path
            // (kill -> drops -> clear). For unattended runs where the boss moves or keeps its distance.
            let lock = user.lock().await;
            let map = lock.get_current_map().ok_or(Error::InvalidInput("no map"))?;
            let zone = lock.zone_pos;
            let player_id = lock.get_user_id();
            drop(lock);
            let mut map = map.lock().await;
            let boss = map.zone_boss_id(zone).ok_or(Error::InvalidInput("no boss in this zone yet"))?;
            map.set_enemy_hp(zone, 1);
            map.deal_damage(
                zone,
                DealDamagePacket {
                    inflicter: ObjectHeader {
                        id: player_id,
                        entity_type: ObjectType::Player,
                        ..Default::default()
                    },
                    target: ObjectHeader {
                        id: boss,
                        entity_type: ObjectType::Object,
                        ..Default::default()
                    },
                    attack_id: 3813693250,
                    ..Default::default()
                },
            )
            .await?;
            Ok(format!("ekill boss {boss}"))
        }
        ("ehp", [hp]) => {
            // HP of every enemy in the player's zone (the next hit goes through the normal damage path)
            let hp: u32 = parse_num(hp).ok_or(Error::InvalidInput("ehp <n>"))?;
            let lock = user.lock().await;
            let map = lock.get_current_map().ok_or(Error::InvalidInput("no map"))?;
            let zone = lock.zone_pos;
            drop(lock);
            let n = map.lock().await.set_enemy_hp(zone, hp);
            Ok(format!("ehp {hp} ({n} enemies)"))
        }
        ("drop", opts) => {
            // drop [model|-] [type:id:subid|meseta:<n>] [x y z]: NewItemDrop (+ ObjectSpawn of model unless "-") at the
            // player (or x y z); default model ob_9900_0001, item Monomate
            let usage = "drop [model|-] [type:id:subid|meseta:<n>] [grind=|gp=|element=|force=|potential=|affix=a,b ..] [x y z]";
            let mut model = crate::map::DROP_MODEL.to_string();
            let mut item = crate::map::monomate();
            let mut rest = opts;
            while let Some(first) = rest.first() {
                if *first == "-" {
                    model.clear();
                } else if first.starts_with("o") {
                    model = first.to_string();
                } else if let Some(n) = first.strip_prefix("meseta:") {
                    item = crate::map::meseta(parse_num(n).ok_or(Error::InvalidInput(usage))?);
                } else if first.contains(':') {
                    let n: Vec<u16> = first
                        .split(':')
                        .map(parse_num)
                        .collect::<Option<_>>()
                        .ok_or(Error::InvalidInput(usage))?;
                    let [t, id, sub] = n[..] else {
                        return Err(Error::InvalidInput(usage));
                    };
                    item.id.item_type = t;
                    item.id.id = id;
                    item.id.subid = sub;
                    if t != 3 {
                        item.data = Default::default();
                    }
                    if t == 1 {
                        item.data = pso2packetlib::protocol::items::ItemType::Weapon(Default::default());
                    }
                } else if let Some((k, v)) = first.split_once('=') {
                    // weapon fields: grind= gp= element= force= potential= affix=a,b,..
                    let pso2packetlib::protocol::items::ItemType::Weapon(w) = &mut item.data else {
                        return Err(Error::InvalidInput("drop: key=value only after a weapon type:id:subid"));
                    };
                    match k {
                        "grind" => w.grind = parse_num(v).ok_or(Error::InvalidInput(usage))?,
                        "gp" => w.grind_percent = parse_num(v).ok_or(Error::InvalidInput(usage))?,
                        "element" => w.element = parse_num(v).ok_or(Error::InvalidInput(usage))?,
                        "force" => w.force = parse_num(v).ok_or(Error::InvalidInput(usage))?,
                        "potential" => w.potential = parse_num(v).ok_or(Error::InvalidInput(usage))?,
                        "affix" => {
                            for (slot, a) in w.affixes.iter_mut().zip(v.split(',')) {
                                *slot = parse_num(a).ok_or(Error::InvalidInput(usage))?;
                            }
                        }
                        _ => return Err(Error::InvalidInput(usage)),
                    }
                } else {
                    break;
                }
                rest = &rest[1..];
            }
            let lock = user.lock().await;
            let map = lock.get_current_map().ok_or(Error::InvalidInput("no map"))?;
            let zone = lock.zone_pos;
            let mut pos = lock.position;
            drop(lock);
            if !rest.is_empty() {
                let (x, y, z) = parse_xyz(rest).ok_or(Error::InvalidInput(usage))?;
                pos.pos_x = half::f16::from_f32(x);
                pos.pos_y = half::f16::from_f32(y);
                pos.pos_z = half::f16::from_f32(z);
            }
            let id = map.lock().await.spawn_drop(zone, item, pos, &model).await;
            Ok(format!(
                "drop {id} model={model:?} at ({:.2}, {:.2}, {:.2})",
                pos.pos_x.to_f32(),
                pos.pos_y.to_f32(),
                pos.pos_z.to_f32()
            ))
        }
        ("give", [id, rest @ ..]) if rest.len() <= 1 => {
            // give type:id:subid [n]: a consumable straight into the inventory, as a pickup does (UpdateInventory onto
            // a stack of 10 or less, else AddedItem)
            let usage = "give type:id:subid [n]";
            let n: Vec<u16> = id.split(':').map(parse_num).collect::<Option<_>>().ok_or(Error::InvalidInput(usage))?;
            let [t, i, sub] = n[..] else { return Err(Error::InvalidInput(usage)) };
            let amount = match rest { [a] => parse_num::<u16>(a).ok_or(Error::InvalidInput(usage))?, _ => 1 };
            let mut item = crate::map::monomate();
            item.id.item_type = t;
            item.id.id = i;
            item.id.subid = sub;
            if let pso2packetlib::protocol::items::ItemType::Consumable(c) = &mut item.data {
                c.amount = amount;
            }
            let mut lock = user.lock().await;
            let lock = &mut *lock;
            let packet = lock
                .character
                .as_mut()
                .ok_or(Error::InvalidInput("no character"))?
                .inventory
                .add_picked_item(item, &mut lock.user_data.last_uuid);
            lock.send_packet(&packet).await?;
            Ok(format!("give {t}:{i}:{sub} x{amount}"))
        }
        ("meseta", [n]) => {
            // meseta <n>: add n meseta to the bag (InventoryMeseta 0F-14 with the new total)
            let n: u64 = parse_num(n).ok_or(Error::InvalidInput("meseta <n>"))?;
            let mut lock = user.lock().await;
            let packet = lock
                .character
                .as_mut()
                .ok_or(Error::InvalidInput("no character"))?
                .inventory
                .add_meseta(n);
            lock.send_packet(&packet).await?;
            Ok(format!("meseta +{n}"))
        }
        ("grind", opts) => {
            // grind <n> [element=E force=F affix=a,b,..]: set the equipped weapon's grind (and element, affixes),
            // recompute the stats and resend it as AddedItem 0F-05 with the same uuid
            let usage = "grind <n> [element=E force=F affix=a,b,..]";
            let [n, rest @ ..] = opts else {
                return Err(Error::InvalidInput(usage));
            };
            let grind: u8 = parse_num(n).ok_or(Error::InvalidInput(usage))?;
            let mut lock = user.lock().await;
            let char = lock.character.as_mut().ok_or(Error::InvalidInput("no character"))?;
            let uuid = char.palette.get_current_item(&char.inventory)?.ok_or(Error::InvalidInput("no weapon"))?.uuid;
            let item = char.inventory.get_inv_item_mut(uuid).ok_or(Error::InvalidInput("no weapon"))?;
            if !matches!(item.data, pso2packetlib::protocol::items::ItemType::Weapon(_)) {
                item.data = pso2packetlib::protocol::items::ItemType::Weapon(Default::default());
            }
            let pso2packetlib::protocol::items::ItemType::Weapon(w) = &mut item.data else {
                unreachable!()
            };
            w.grind = grind;
            for kv in rest {
                match kv.split_once('=') {
                    Some(("element", v)) => w.element = parse_num(v).ok_or(Error::InvalidInput(usage))?,
                    Some(("force", v)) => w.force = parse_num(v).ok_or(Error::InvalidInput(usage))?,
                    Some(("affix", v)) => {
                        w.affixes = [0; 8];
                        for (slot, a) in w.affixes.iter_mut().zip(v.split(',')) {
                            *slot = parse_num(a).ok_or(Error::InvalidInput(usage))?;
                        }
                    }
                    _ => return Err(Error::InvalidInput(usage)),
                }
            }
            let item = item.clone();
            crate::battle_stats::PlayerStats::update(&mut lock)?;
            let pwr = lock.get_stats().weapon_pwr();
            let hp = lock.get_stats().get_hp();
            lock.send_packet(&Packet::AddedItem(pso2packetlib::protocol::items::AddedItemPacket {
                item: item.clone(),
                ..Default::default()
            }))
            .await?;
            Ok(format!("grind {:?} -> weapon pwr {pwr:?} hp {hp:?}", item.data))
        }
        ("clear", xyz) => {
            // same as the last enemy dying: clear telepipe at the player (or x y z), On, cleared state on InitPosition
            let lock = user.lock().await;
            let map = lock.get_current_map().ok_or(Error::InvalidInput("no map"))?;
            let zone = lock.zone_pos;
            let mut pos = lock.position;
            drop(lock);
            if !xyz.is_empty() {
                let (x, y, z) = parse_xyz(xyz).ok_or(Error::InvalidInput("clear [x y z]"))?;
                pos.pos_x = half::f16::from_f32(x);
                pos.pos_y = half::f16::from_f32(y);
                pos.pos_z = half::f16::from_f32(z);
            }
            let done = map.lock().await.quest_clear(zone, pos).await?;
            Ok(format!("clear {}", if done { "ok" } else { "already cleared" }))
        }
        ("tag", [id, attr]) => {
            let id: u32 = parse_num(id).ok_or(Error::InvalidInput("tag <obj id> <attr>"))?;
            let (map, zone, pid) = {
                let lock = user.lock().await;
                (
                    lock.get_current_map().ok_or(Error::InvalidInput("no map"))?,
                    lock.zone_pos,
                    lock.get_user_id(),
                )
            };
            let world_id = map.lock().await.zone_world_id(zone).unwrap_or(0);
            let obj = ObjectHeader {
                id,
                entity_type: ObjectType::Object,
                map_id: world_id as u16,
                ..Default::default()
            };
            // object3 is the actor (the client looks it up for "Access" / "SetPartyId")
            let player = ObjectHeader {
                id: pid,
                entity_type: ObjectType::Player,
                ..Default::default()
            };
            let packet = Packet::SetTag(SetTagPacket {
                receiver: player,
                target: obj,
                object3: player,
                attribute: attr.to_string().into(),
                ..Default::default()
            });
            user.lock().await.send_packet(&packet).await?;
            Ok(format!("tag {id} {attr}"))
        }
        ("finish" | "result", opts) => {
            use super::handlers::server::{RESULT_FF, RESULT_HIDE, move_to_campship, quest_result};
            use pso2packetlib::protocol::questlist::QuestResultRank;
            const USAGE: &str = "finish|result [hide] [ff] [now] [rank=C|B|A|S] [meseta=N] [exp=N] [kills=N] [score=N/M]";
            let mut bits = 0;
            let mut now = cmd == "result";
            // values for `result` (sent at once): rank, meseta, exp, enemy kills, total score achieved / total
            let mut values: Vec<(&str, &str)> = vec![];
            for o in opts {
                match *o {
                    "hide" => bits |= RESULT_HIDE,
                    "ff" => bits |= RESULT_FF,
                    "now" => now = true,
                    kv if kv.contains('=') => values.extend(kv.split_once('=')),
                    _ => return Err(Error::InvalidInput(USAGE)),
                }
            }
            if cmd == "finish" {
                move_to_campship(user.lock().await, (!now).then_some(bits)).await?;
            }
            if now {
                let mut packet = quest_result(bits);
                if let Packet::QuestResult(p) = &mut packet {
                    for (k, v) in &values {
                        let n = || parse_num::<u32>(v).ok_or(Error::InvalidInput(USAGE));
                        match *k {
                            "rank" => {
                                p.rank = match *v {
                                    "S" => QuestResultRank::S,
                                    "A" => QuestResultRank::A,
                                    "B" => QuestResultRank::B,
                                    _ => QuestResultRank::C,
                                }
                            }
                            "meseta" => p.meseta_earned = n()?,
                            "exp" => p.exp_earned = n()?,
                            "kills" => p.enemy_kills.value = n()?,
                            "score" => {
                                let (a, b) = v.split_once('/').ok_or(Error::InvalidInput(USAGE))?;
                                p.total_score_achieved = parse_num(a).ok_or(Error::InvalidInput(USAGE))?;
                                p.total_score = parse_num(b).ok_or(Error::InvalidInput(USAGE))?;
                            }
                            _ => return Err(Error::InvalidInput(USAGE)),
                        }
                    }
                }
                user.lock().await.send_packet(&packet).await?;
            }
            Ok(format!("{cmd} opts={bits} now={now}"))
        }
        ("points", [total, rest @ ..]) if rest.len() <= 1 => {
            // SetQuestPoints (0B-1F): the client compares unk1 with the zone's world header and party with its own
            // party, stores the total and opens the area lock ("ArealockOpened") when the total reaches the quest's
            // required points (0 for the test quest: never)
            let (Some(total), Some(gained)) =
                (parse_num::<u32>(total), rest.first().map_or(Some(0), |g| parse_num::<u32>(g)))
            else {
                return Err(Error::InvalidInput("points <total> [gained]"));
            };
            let lock = user.lock().await;
            let map = lock.get_current_map().ok_or(Error::InvalidInput("no map"))?;
            let party = lock.get_current_party().ok_or(Error::InvalidInput("no party"))?;
            drop(lock);
            let zone = user.lock().await.zone_pos;
            let world = ObjectHeader {
                id: map.lock().await.zone_world_id(zone).unwrap_or(0),
                entity_type: ObjectType::World,
                ..Default::default()
            };
            let party = party.read().await.get_obj();
            let packet = Packet::SetQuestPoints(SetQuestPointsPacket {
                unk1: world,
                party,
                total,
                gained,
            });
            user.lock().await.send_packet(&packet).await?;
            Ok(format!("points {total} gained {gained} world={} party={}", world.id, party.id))
        }
        ("quest", [id, diff]) => {
            let (Some(id), Some(diff)) = (parse_num(id), parse_num(diff)) else {
                return Err(Error::InvalidInput("quest <id> <diff>"));
            };
            start_quest(&user, id, diff).await?;
            Ok(format!("quest {id} diff {diff} accepted"))
        }
        ("pos", []) => {
            let lock = user.lock().await;
            let p = lock.position;
            let zone = match lock.get_current_map() {
                Some(m) => {
                    let pos = lock.zone_pos;
                    drop(lock);
                    m.lock().await.zone_name(pos).unwrap_or("?").to_string()
                }
                None => "-".into(),
            };
            let msg = format!(
                "zone={zone} pos=({:.2}, {:.2}, {:.2}) rot=({:.2}, {:.2}, {:.2}, {:.2})",
                p.pos_x.to_f32(),
                p.pos_y.to_f32(),
                p.pos_z.to_f32(),
                p.rot_x.to_f32(),
                p.rot_y.to_f32(),
                p.rot_z.to_f32(),
                p.rot_w.to_f32()
            );
            log::info!("[pso2-debug] {msg}");
            Ok(msg)
        }
        ("hp", rest) if rest.len() <= 1 => {
            // server-side HP only (the client follows new_hp of the next DamageReceive)
            let mut lock = user.lock().await;
            if let [n] = rest {
                let n = parse_num::<u32>(n).ok_or(Error::InvalidInput("hp [n]"))?;
                lock.get_stats_mut().set_hp(n);
            }
            let (hp, max) = lock.get_stats().get_hp();
            Ok(format!("hp {hp}/{max}"))
        }
        ("exp", [n]) => {
            // same path as a kill: add_exp, then GainedEXP to this player only
            let n = parse_num::<u32>(n).ok_or(Error::InvalidInput("exp <n>"))?;
            let mut lock = user.lock().await;
            let receiver = lock.add_exp(n)?;
            let msg = format!(
                "exp +{n}: total {} level {} ({}), hp {:?}",
                receiver.total,
                receiver.level,
                receiver.level2,
                lock.get_stats().get_hp()
            );
            let packet = Packet::GainedEXP(GainedEXPPacket {
                sender: lock.create_object_header(),
                receivers: vec![receiver],
            });
            lock.send_packet(&packet).await?;
            log::info!("[pso2-debug] {msg}");
            Ok(msg)
        }
        ("unlock", ids) if !ids.is_empty() => {
            // unlocks quests for this character (the story list shows them on the next counter visit)
            let ids = ids
                .iter()
                .map(|i| parse_num::<u32>(i))
                .collect::<Option<Vec<_>>>()
                .ok_or(Error::InvalidInput("unlock <name_id>..."))?;
            let mut lock = user.lock().await;
            let char = lock.character.as_mut().ok_or(Error::InvalidInput("no character"))?;
            for id in &ids {
                if !char.unlocked_quests.contains(id) {
                    char.unlocked_quests.push(*id);
                }
            }
            Ok(format!("unlocked {ids:?}"))
        }
        ("spawn", [name, rest @ ..]) if (0..=4).contains(&rest.len()) => {
            // [pso2_vita_offline] a trailing `boss` makes it the zone's boss (the quest clears when it dies)
            let (rest, boss) = match rest {
                [rest @ .., b] if *b == "boss" => (rest, true),
                _ => (rest, false),
            };
            if !(rest.is_empty() || rest.len() == 3) {
                return Err(Error::InvalidInput("spawn <enemy> [x y z] [boss]"));
            }
            let lock = user.lock().await;
            let map = lock.get_current_map().ok_or(Error::InvalidInput("no map"))?;
            let zone = lock.zone_pos;
            let mut pos = lock.position;
            drop(lock);
            if let [x, y, z] = rest {
                let (Ok(x), Ok(y), Ok(z)) = (x.parse::<f32>(), y.parse::<f32>(), z.parse::<f32>())
                else {
                    return Err(Error::InvalidInput("spawn <enemy> [x y z]"));
                };
                pos.pos_x = half::f16::from_f32(x);
                pos.pos_y = half::f16::from_f32(y);
                pos.pos_z = half::f16::from_f32(z);
            }
            map.lock().await.spawn_enemy_as(zone, name, pos, boss).await?;
            Ok(format!("spawned {name}{}", if boss { " (boss)" } else { "" }))
        }
        ("co", [op, id]) => {
            // [pso2_vita_offline] co take|drop <order id>: client orders without the NPC window (counts start at 0;
            // the window shows them after it is opened again)
            let id: u32 = parse_num(id).ok_or(Error::InvalidInput("co take|drop <id>"))?;
            let order = crate::client_orders::get(id).ok_or(Error::InvalidInput("no such order"))?;
            let mut lock = user.lock().await;
            let c = lock.character.as_mut().ok_or(Error::InvalidInput("no character"))?;
            match *op {
                "take" => {
                    c.client_orders.taken.retain(|t| t.id != id);
                    c.client_orders.taken.push(crate::client_orders::TakenOrder { id, counts: vec![0; order.targets.len()] });
                }
                "drop" => c.client_orders.taken.retain(|t| t.id != id),
                _ => return Err(Error::InvalidInput("co take|drop <id>")),
            }
            Ok(format!("co {op} {id} {}", order.name))
        }
        ("send", [id, subid, flag, hex @ ..]) => {
            let (Some(id), Some(subid), Some(flag)) =
                (parse_num::<u8>(id), parse_num::<u16>(subid), parse_num::<u8>(flag))
            else {
                return Err(Error::InvalidInput("send <id> <subid> <flag> <hex>"));
            };
            let data = parse_hex(&hex.concat()).ok_or(Error::InvalidInput("bad hex"))?;
            let len = data.len();
            let packet = Packet::Unknown((PacketHeader::new(id, subid, flags_from(flag)), data));
            user.lock().await.send_packet(&packet).await?;
            Ok(format!("sent 0x{id:02X}-0x{subid:02X} flag 0x{flag:02X} {len} B"))
        }
        _ => {
            // same path as a "!" chat command typed by the player
            let lock = user.lock().await;
            let packet = Packet::ChatMessage(ChatMessage {
                object: lock.create_object_header(),
                message: format!("!{line}"),
                ..Default::default()
            });
            super::packet_handler(lock, packet).await?;
            Ok(format!("ran !{line} (replies go to the in-game chat)"))
        }
    }
}

/// Spawned from `init_block` for every block. The first block binds `PSO2_DEBUG_PORT`; commands go to
/// the in-game player of any block.
pub async fn command_listener(block: Arc<BlockData>) {
    let first = {
        let mut blocks = BLOCKS.lock().unwrap();
        blocks.push(block.clone());
        blocks.len() == 1
    };
    let Some(port) = std::env::var("PSO2_DEBUG_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
    else {
        return;
    };
    if !first {
        return;
    }
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            log::warn!("[pso2-debug] command port {port}: {e}");
            return;
        }
    };
    log::info!("[pso2-debug] command port 127.0.0.1:{port}");
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        tokio::spawn(async move {
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                log::info!("[pso2-debug] cmd: {line}");
                let reply = match run_command(&line).await {
                    Ok(m) => format!("ok {m}\n"),
                    Err(e) => format!("err {e}\n"),
                };
                log::info!("[pso2-debug] {}", reply.trim_end());
                if w.write_all(reply.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
    }
}
