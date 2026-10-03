//! [pso2_vita_offline] Debug entry points (T22).
//!
//! - `PSO2_DEBUG_START="quest=1100 diff=0 zone=campship_down [wait_ms=1000]"`: after the first lobby load,
//!   accept the quest, transfer into its map (campship) and then move to `zone`. No client input needed.
//! - `PSO2_DEBUG_PORT=<port>`: loopback TCP (one port for all blocks), one command per line, one reply line per command.
//!   Commands: `goto <zone> [x y z]`, `quest <id> <diff>`, `lobby`, `spawn <enemy> [x y z]`,
//!   `send <id> <subid> <flag> <hex>`, `pos`, `tp <x> <y> <z>`, `help`. Anything else is run as a `!` chat command.
//! - T25: `goto <zone> x y z` and `PSO2_DEBUG_START="... pos=x,y,z"` replace the zone's `default_location`
//!   (position only) for the next spawn into that zone. `tp` sends `TeleportTransfer` (0x04, 0x02) to the player
//!   (the Vita client ignores it in a free field: the position snaps back, T25).
//!
//! Both are disabled unless the environment variable is set. Single player assumed (the last in-game client).
use super::{User, UserState};
use crate::{BlockData, Error, mutex::Mutex};
use pso2packetlib::protocol::{
    Flags, ObjectHeader, ObjectType, Packet, PacketHeader, chat::ChatMessage, models::Position,
    objects::TeleportTransferPacket, questlist::AcceptQuestPacket,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

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

const HELP: &str = "goto <zone> [x y z] | tp <x> <y> <z> | quest <id> <diff> | lobby | spawn <enemy> [x y z] | send <id> <subid> <flag> <hex> | pos | <any ! chat command>";

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
            let (x, y, z) = parse_xyz(xyz).ok_or(Error::InvalidInput("tp <x> <y> <z>"))?;
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
        ("lobby", []) => goto(&user, "lobby").await,
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
        ("spawn", [name, rest @ ..]) if rest.is_empty() || rest.len() == 3 => {
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
            map.lock().await.spawn_enemy(zone, name, pos).await?;
            Ok(format!("spawned {name}"))
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
