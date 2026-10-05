use super::HResult;
use crate::{Action, Error, User, UserState, mutex::MutexGuard, party};
use pso2packetlib::protocol::{
    self, ObjectHeader, Packet,
    flag::{FlagType, SetFlagPacket},
    server::{
        BridgeToLobbyPacket, BridgeTransportPacket, CafeToLobbyPacket, CafeTransportPacket,
        CampshipDownPacket, CampshipToLobbyPacket, CasinoToLobbyPacket, CasinoTransportPacket,
        DeathToCampshipPacket, MapLoadedPacket, MoveZonePacket, ReturnToCampshipFinalPacket,
        ReturnToCampshipPacket, StoryToLobbyPacket, ToCampshipPacket,
    },
};
use std::sync::atomic::Ordering;

pub async fn initial_load(mut user: MutexGuard<'_, User>) -> HResult {
    let conn_id = user.conn_id;
    let blockdata = user.blockdata.clone();

    user.set_map(blockdata.lobby.clone());
    let party_id = blockdata.latest_partyid.fetch_add(1, Ordering::Relaxed);
    drop(user);

    let clients = blockdata.clients.lock().await;
    let Some((_, user)) = clients
        .iter()
        .find(|(c_conn_id, _)| *c_conn_id == conn_id)
        .cloned()
    else {
        unreachable!();
    };
    drop(clients);

    party::Party::init_player(user.clone(), party_id).await?;
    blockdata
        .lobby
        .lock()
        .await
        .init_add_player(user.clone())
        .await?;
    let mut user_lock = user.lock().await;
    user_lock.state = UserState::InGame;
    Ok(Action::Nothing)
}

pub async fn move_to_bridge(user: MutexGuard<'_, User>, _: BridgeTransportPacket) -> HResult {
    let map = user.get_current_map();
    let id = user.get_user_id();
    drop(user);
    if let Some(map) = map {
        let mut lock = map.lock().await;
        lock.move_player_named(id, "bridge").await?;
    }

    Ok(Action::Nothing)
}

pub async fn move_from_bridge(user: MutexGuard<'_, User>, _: BridgeToLobbyPacket) -> HResult {
    let map = user.get_current_map();
    let id = user.get_user_id();
    drop(user);
    if let Some(map) = map {
        let mut lock = map.lock().await;
        lock.move_player_named(id, "lobby").await?;
    }

    Ok(Action::Nothing)
}

pub async fn move_to_casino(user: MutexGuard<'_, User>, _: CasinoTransportPacket) -> HResult {
    let map = user.get_current_map();
    let id = user.get_user_id();
    drop(user);
    if let Some(map) = map {
        let mut lock = map.lock().await;
        lock.move_player_named(id, "casino").await?;
    }

    Ok(Action::Nothing)
}

pub async fn move_from_casino(user: MutexGuard<'_, User>, _: CasinoToLobbyPacket) -> HResult {
    let map = user.get_current_map();
    let id = user.get_user_id();
    drop(user);
    if let Some(map) = map {
        let mut lock = map.lock().await;
        lock.move_player_named(id, "lobby").await?;
    }

    Ok(Action::Nothing)
}

pub async fn move_to_cafe(user: MutexGuard<'_, User>, _: CafeTransportPacket) -> HResult {
    let map = user.get_current_map();
    let id = user.get_user_id();
    drop(user);
    if let Some(map) = map {
        let mut lock = map.lock().await;
        lock.move_player_named(id, "cafe").await?;
    }

    Ok(Action::Nothing)
}

pub async fn move_from_cafe(user: MutexGuard<'_, User>, _: CafeToLobbyPacket) -> HResult {
    let map = user.get_current_map();
    let id = user.get_user_id();
    drop(user);
    if let Some(map) = map {
        let mut lock = map.lock().await;
        lock.move_player_named(id, "lobby").await?;
    }

    Ok(Action::Nothing)
}

pub async fn campship_down(user: MutexGuard<'_, User>, _: CampshipDownPacket) -> HResult {
    let map = user.get_current_map();
    let id = user.get_user_id();
    drop(user);
    if let Some(map) = map {
        let mut lock = map.lock().await;
        lock.move_player_named(id, "campship_down").await?;
    }

    Ok(Action::Nothing)
}

/// [pso2_vita_offline] 03-05: sent by an area exit (`Object:GoalTransporter`) when stepped on.
/// `door_id` is the object's property 0x1b; the destination comes from the map's warps.
pub async fn move_zone(user: MutexGuard<'_, User>, data: MoveZonePacket) -> HResult {
    let map = user.get_current_map();
    let id = user.get_user_id();
    drop(user);
    if let Some(map) = map {
        let mut lock = map.lock().await;
        lock.move_zone(id, data.current_zone_id, data.door_id).await?;
    }

    Ok(Action::Nothing)
}

pub async fn map_loaded(mut user_guard: MutexGuard<'_, User>, _: MapLoadedPacket) -> HResult {
    let user = &mut *user_guard;
    let user_id = user.get_user_id();
    let Some(character) = &mut user.character else {
        unreachable!("Character should be loaded here");
    };
    let inventory_packets = character.inventory.send(
        user_id,
        character.character.name.clone(),
        &user.blockdata.server_data.item_params,
        user.user_data.lang,
    );
    let palette = character.palette.send_palette();
    let default_pa_packet = character.palette.send_default_pa();
    let equiped = character.inventory.send_equiped(user_id);
    let change_palette = character.palette.send_change_palette(user_id);

    let char_flags = character.flags.to_char_flags();
    for packet in inventory_packets {
        user.send_packet(&packet).await?;
    }
    if user.firstload {
        let flags = user.user_data.accountflags.to_account_flags();
        user.send_packet(&flags).await?;
        user.send_packet(&char_flags).await?;
    }

    user.send_packet(&Packet::LoadPAs(protocol::objects::LoadPAsPacket {
        receiver: protocol::ObjectHeader {
            id: user_id,
            entity_type: protocol::ObjectType::Player,
            ..Default::default()
        },
        target: protocol::ObjectHeader {
            id: user_id,
            entity_type: protocol::ObjectType::Player,
            ..Default::default()
        },
        levels: vec![1; 0xee].into(),
        ..Default::default()
    }))
    .await?;

    user.send_packet(&palette).await?;
    user.send_packet(&default_pa_packet).await?;
    user.send_packet(&equiped).await?;
    user.send_packet(&change_palette).await?;
    // unlock controls?
    user.send_packet(&Packet::UnlockControls).await?;
    user.send_packet(&Packet::FinishLoading).await?;
    let packet = protocol::unk19::LobbyMonitorPacket { video_id: 121 };
    user.send_packet(&Packet::LobbyMonitor(packet)).await?;
    user.firstload = false;
    if let Some(opts) = user.pending_result.take() {
        user.send_packet(&quest_result(opts)).await?;
    }
    crate::user::debug::on_map_loaded(user).await;

    let map = user.map.clone().unwrap();
    let player_id = user.get_user_id();
    let zone = user.zone_pos;
    drop(user_guard);
    map.lock().await.on_map_loaded(zone, player_id).await?;

    Ok(Action::Nothing)
}

pub async fn set_flag(user: &mut User, data: SetFlagPacket) -> HResult {
    match data.flag_type {
        FlagType::Account => user
            .user_data
            .accountflags
            .set(data.id as usize, data.value as u8),
        FlagType::Character => user
            .character
            .as_mut()
            .unwrap()
            .flags
            .set(data.id as usize, data.value as u8),
    }
    Ok(Action::Nothing)
}

pub async fn to_campship(user: MutexGuard<'_, User>, _: ToCampshipPacket) -> HResult {
    let Some(party) = user.get_current_party() else {
        unreachable!("User should be in state >= 'PreInGame'");
    };
    let Some(map) = user.get_current_map() else {
        unreachable!("User should be in state >= 'PreInGame'");
    };
    let player_id = user.get_user_id();
    drop(user);
    let quest_map = party
        .read()
        .await
        .get_quest_map()
        .ok_or_else(|| Error::InvalidInput("to_campship"))?;
    let player = map
        .lock()
        .await
        .remove_player(player_id)
        .await
        .ok_or_else(|| Error::InvalidInput("to_campship"))?;
    player.lock().await.set_map(quest_map.clone());
    quest_map.lock().await.init_add_player(player).await?;
    Ok(Action::Nothing)
}

pub async fn move_from_story(user: MutexGuard<'_, User>, _: StoryToLobbyPacket) -> HResult {
    let Some(map) = user.get_current_map() else {
        unreachable!("User should be in state >= 'PreInGame'");
    };
    let lobby = user.blockdata.lobby.clone();
    let id = user.get_user_id();
    drop(user);
    let player = map
        .lock()
        .await
        .remove_player(id)
        .await
        .ok_or_else(|| Error::InvalidInput("move_from_story"))?;
    player.lock().await.set_map(lobby.clone());
    lobby.lock().await.init_add_player(player).await?;

    Ok(Action::Nothing)
}

/// `QuestResult` options: do not show the result screen.
pub const RESULT_HIDE: u8 = 1 << 0;
/// `QuestResult` options: first u32 of `unk1` / `unk2` entries = 0xFFFFFFFF (the Vita ctor's default).
pub const RESULT_FF: u8 = 1 << 1;

/// Empty quest result (all scores 0, rank C).
pub fn quest_result(opts: u8) -> Packet {
    use pso2packetlib::protocol::questlist::{QuestResultPacket, QuestResultRank, QuestResultUnk1};
    let mut packet = QuestResultPacket {
        rank: QuestResultRank::C,
        total_score_achieved: 0,
        total_score: 0,
        hide_results: (opts & RESULT_HIDE != 0) as u8,
        ..Default::default()
    };
    if opts & RESULT_FF != 0 {
        let unk = || {
            vec![
                QuestResultUnk1 {
                    unk1: u32::MAX,
                    ..Default::default()
                };
                3
            ]
            .into()
        };
        packet.unk1 = unk();
        packet.unk2 = unk();
    }
    Packet::QuestResult(packet)
}

/// `quest_state` flags: bit 0 = the clear event (`0x82ec8c58`), bit 1 = cleared (clear telepipe shown, the campship
/// exit becomes "quest end" / 03-1C instead of "abandon"), bit 3 = later states are ignored.
pub const QUEST_STATE_CLEARED: u32 = 0b11;

/// Quest state (0x0B-0x23, not in pso2packetlib). The Vita client compares `world` with the world header of the zone
/// it is in (`quest manager+0x104`) and copies `flags` / `points` into the quest progress (`quest manager+0x24c`).
/// Layout: world header, party header (not checked), two variable-length strings (empty here), 8 + 8 bytes, u8 x 4,
/// flags u32, points u32 x 5. 0x54 bytes with empty strings.
pub fn quest_state(world: ObjectHeader, party: ObjectHeader, flags: u32) -> Packet {
    use pso2packetlib::protocol::{Flags, PacketHeader};
    fn header(out: &mut Vec<u8>, h: &ObjectHeader) {
        out.extend_from_slice(&h.id.to_le_bytes());
        out.extend_from_slice(&h.unk.to_le_bytes());
        out.extend_from_slice(&(h.entity_type as u16).to_le_bytes());
        out.extend_from_slice(&h.map_id.to_le_bytes());
    }
    // empty variable-length string: (len + 0xFF) ^ 0xDCD7
    let empty = (0xFFu32 ^ 0xDCD7).to_le_bytes();
    let mut data = Vec::with_capacity(0x4C);
    header(&mut data, &world);
    header(&mut data, &party);
    data.extend_from_slice(&empty);
    data.extend_from_slice(&empty);
    data.extend_from_slice(&[0; 16 + 4]);
    data.extend_from_slice(&flags.to_le_bytes());
    data.extend_from_slice(&[0; 0x14]);
    Packet::Unknown((PacketHeader::new(0x0B, 0x23, Flags::PACKED), data))
}

/// Moves the player to the quest map's campship zone; `result` = send `QuestResult` on the next `MapLoaded`.
pub async fn move_to_campship(user: MutexGuard<'_, User>, result: Option<u8>) -> HResult {
    let mut user = user;
    let map = user.get_current_map();
    let id = user.get_user_id();
    user.pending_result = result;
    user.get_stats_mut().restore_hp();
    drop(user);
    if let Some(map) = map {
        map.lock().await.move_player_named(id, "campship").await?;
    }
    Ok(Action::Nothing)
}

pub async fn return_to_campship(user: MutexGuard<'_, User>, p: ReturnToCampshipPacket) -> HResult {
    log::info!("[pso2-quest] ReturnToCampship {:?}", p.world);
    move_to_campship(user, None).await
}

pub async fn return_to_campship_final(
    user: MutexGuard<'_, User>,
    p: ReturnToCampshipFinalPacket,
) -> HResult {
    log::info!("[pso2-quest] ReturnToCampshipFinal {:?}", p.world);
    move_to_campship(user, Some(0)).await
}

pub async fn death_to_campship(user: MutexGuard<'_, User>, p: DeathToCampshipPacket) -> HResult {
    log::info!("[pso2-quest] DeathToCampship {:?}", p.world);
    move_to_campship(user, None).await
}

pub async fn campship_to_lobby(user: MutexGuard<'_, User>, p: CampshipToLobbyPacket) -> HResult {
    log::info!("[pso2-quest] CampshipToLobby {:?}", p.world);
    let map = user.get_current_map();
    let id = user.get_user_id();
    drop(user);
    if let Some(map) = map {
        map.lock().await.move_to_lobby(id).await?;
    }
    Ok(Action::Nothing)
}
