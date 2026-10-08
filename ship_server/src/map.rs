use crate::{
    BlockData, Error, User,
    battle_stats::{BattleResult, EnemyStats},
    mutex::{Mutex, MutexGuard},
};
use data_structs::map::{EventData, MapData, NPCData, ObjectData, TransporterData, ZoneData};
use mlua::{Lua, LuaSerdeExt, StdLib};
use pso2packetlib::protocol::{
    self, ObjectHeader, ObjectType, Packet, PacketType,
    flag::{CutsceneEndPacket, SkitItemAddRequestPacket, SkitItemAddResponsePacket},
    models::Position,
    objects::{DamageReceivePacket, EnemyActionPacket},
    playerstatus::{DealDamagePacket, GainedEXPPacket, SetPlayerIDPacket},
    questlist::{MinimapRevealPacket, RevealedRegions},
    server::{LoadLevelPacket, MapTransferPacket},
    spawn::{CharacterSpawnPacket, CharacterSpawnType, ObjectSpawnPacket},
    symbolart::{ReceiveSymbolArtPacket, SendSymbolArtPacket},
};
use rand::{prelude::Distribution, seq::IteratorRandom};
use std::{
    collections::HashMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicU32, Ordering},
    },
    time::Instant,
};

type ZoneId = u32;
type PlayerId = u32;

#[derive(Clone)]
struct MapPlayer {
    player_id: PlayerId,
    user: Weak<Mutex<User>>,
}

#[derive(Clone)]
struct OwnedMapPlayer {
    player_id: PlayerId,
    user: Arc<Mutex<User>>,
}

struct Zone {
    zone_pos: usize,
    lua: Arc<parking_lot::Mutex<LuaState>>,
    srv_zone_id: ZoneId,
    obj: ObjectHeader,
    players: Vec<MapPlayer>,
    enemies: Vec<(u32, EnemyStats)>,
    chunk_spawns: Vec<(u32, Instant)>,
    minimap_status: RevealedRegions,
    data: ZoneData,
    objects: Objects,
    // [pso2_vita_offline] quest clear: the clear telepipe (id, position), set once
    clear_pipe: Option<(u32, Position)>,
    // [pso2_vita_offline] items lying on the ground (NewItemDrop sent, not picked up yet)
    drops: Vec<FieldDrop>,
    // [pso2_vita_offline] object id of the zone's boss once spawned (`ZoneData::boss` or debug `spawn ... boss`)
    boss_id: Option<u32>,
}

/// [pso2_vita_offline] An item on the ground. The client keys its drop registry by `drop_id` and finds the entry
/// for a touched object by comparing the object header with `obj` (NewItemDrop.item_obj).
#[derive(Debug, Clone)]
pub struct FieldDrop {
    pub drop_id: u32,
    pub obj: ObjectHeader,
    pub item: protocol::items::Item,
}

/// [pso2_vita_offline] Default drop model: `ob_9900_0001` has `Behavior.Custom[..].IsSimpleObject = 2` in
/// sy_script.fpk, which the client's object factory turns into an "Object:DropItem".
pub const DROP_MODEL: &str = "ob_9900_0001";

struct Objects {
    objects: Vec<ObjectData>,
    events: Vec<EventData>,
    npcs: Vec<NPCData>,
    transporters: Vec<TransporterData>,
}

pub enum MapType {
    Lobby,
    QuestMap,
}

struct LuaState {
    lua: Lua,
    // fighting with async recursion
    to_move: Vec<(PlayerId, String)>,
    to_lobby_move: Vec<PlayerId>,
    procs: HashMap<String, String>,
}

pub struct Map {
    // lua is not `Send` so i've put it in a mutex
    // this mutex shouldn't block, because `Map` is under a mutex itself.
    lua: Arc<parking_lot::Mutex<LuaState>>,
    zones: Box<[Zone]>,
    player_cache: Vec<(u32, usize)>,
    data: MapData,
    max_id: u32,
    block_data: Option<Arc<BlockData>>,
    enemy_level: u32,
    map_type: MapType,
    quest_obj: ObjectHeader,
}
impl Map {
    pub fn new_from_data(mut data: MapData, map_obj_id: &AtomicU32) -> Result<Self, Error> {
        // will be increased as needed
        let lua_libs = StdLib::NONE;
        let lua = Arc::new(parking_lot::Mutex::new(LuaState {
            lua: Lua::new_with(lua_libs, mlua::LuaOptions::default())?,
            to_move: vec![],
            to_lobby_move: vec![],
            procs: HashMap::new(),
        }));
        let map_obj = ObjectHeader {
            id: map_obj_id.fetch_add(1, Ordering::Relaxed),
            entity_type: ObjectType::Map,
            ..Default::default()
        };
        data.map_data.map_object = map_obj;
        data.map_data.settings = data.zones[0].settings.clone();
        data.map_data.other_settings.clear();
        let mut zones = vec![];
        for (i, zone) in std::mem::take(&mut data.zones).into_iter().enumerate() {
            if !zone.is_special_zone {
                data.map_data.other_settings.push(zone.settings.clone());
            }
            let objects = data
                .objects
                .iter()
                .filter(|o| o.zone_id == zone.zone_id)
                .cloned()
                .collect();
            let events = data
                .events
                .iter()
                .filter(|o| o.zone_id == zone.zone_id)
                .cloned()
                .collect();
            let npcs = data
                .npcs
                .iter()
                .filter(|o| o.zone_id == zone.zone_id)
                .cloned()
                .collect();
            let transporters = data
                .transporters
                .iter()
                .filter(|o| o.zone_id == zone.zone_id)
                .cloned()
                .collect();
            zones.push(Zone {
                zone_pos: i,
                lua: lua.clone(),
                srv_zone_id: zone.zone_id,
                players: vec![],
                obj: ObjectHeader {
                    id: map_obj_id.fetch_add(1, Ordering::Relaxed),
                    entity_type: ObjectType::Map,
                    ..Default::default()
                },
                enemies: vec![],
                chunk_spawns: vec![],
                minimap_status: Default::default(),
                clear_pipe: None,
                drops: vec![],
                boss_id: None,
                data: zone,
                objects: Objects {
                    objects,
                    events,
                    npcs,
                    transporters,
                },
            });
        }
        let mut map = Self {
            lua: lua.clone(),
            zones: zones.into(),
            player_cache: vec![],
            data,
            max_id: 0,
            block_data: None,
            enemy_level: 0,
            map_type: MapType::QuestMap,
            quest_obj: ObjectHeader {
                entity_type: ObjectType::Quest,
                ..Default::default()
            },
        };
        map.init_lua()?;
        map.find_max_id();
        log::trace!("Map {} created", map_obj.id);
        Ok(map)
    }
    pub const fn set_map_type(&mut self, map_type: MapType) {
        self.map_type = map_type;
    }
    pub fn set_block_data(&mut self, data: Arc<BlockData>) {
        self.block_data = Some(data);
    }
    pub const fn set_enemy_level(&mut self, level: u32) {
        self.enemy_level = level;
    }
    pub const fn set_quest_obj(&mut self, obj: ObjectHeader) {
        self.quest_obj = obj;
    }
    // [pso2_vita_offline] zone lookups for the debug entry (user/debug.rs)
    pub fn has_zone(&self, name: &str) -> bool {
        self.zones.iter().any(|z| z.data.name == name)
    }
    /// [pso2_vita_offline] `ZoneSettings.world_id` of a zone (the `map_id` of its objects' headers).
    pub fn zone_world_id(&self, zone_pos: usize) -> Option<u32> {
        self.zones.get(zone_pos).map(|z| z.data.settings.world_id)
    }
    pub fn zone_name(&self, zone_pos: usize) -> Option<&str> {
        self.zones.get(zone_pos).map(|z| z.data.name.as_str())
    }
    pub fn init_zone_name(&self) -> Option<&str> {
        self.zones
            .iter()
            .find(|z| z.srv_zone_id == self.data.init_map)
            .map(|z| z.data.name.as_str())
    }
    fn find_max_id(&mut self) {
        let obj_max = self
            .data
            .objects
            .iter()
            .map(|o| o.data.object.id)
            .max()
            .unwrap_or(0);
        let npc_max = self
            .data
            .npcs
            .iter()
            .map(|o| o.data.object.id)
            .max()
            .unwrap_or(0);
        let event_max = self
            .data
            .events
            .iter()
            .map(|o| o.data.object.id)
            .max()
            .unwrap_or(0);
        let transporter_max = self
            .data
            .transporters
            .iter()
            .map(|o| o.data.object.id)
            .max()
            .unwrap_or(0);
        self.max_id = obj_max.max(npc_max).max(event_max).max(transporter_max) + 1;
    }
    fn init_lua(&mut self) -> Result<(), Error> {
        // default object handler
        for object in self.data.objects.iter() {
            let name: &str = &object.data.name;
            if self.data.luas.contains_key(name) {
                continue;
            }
            self.data.luas.insert(
                name.to_owned(),
                "if call_type == \"interaction\" then
                    print(packet.object1.id, packet.action)
                elseif call_type == \"to_vita\" then
	                for i=1,size,2 do
		                if data[i] > 50 and data[i] < 80 then
			                data[i] = data[i] - 1
		                end
	                end
                end"
                .into(),
            );
        }
        // default npc handler
        for npc in self.data.npcs.iter() {
            let name: &str = &npc.data.name;
            if self.data.luas.contains_key(name) {
                continue;
            }
            self.data.luas.insert(
                name.to_owned(),
                "if call_type == \"interaction\" then
                    if packet.action == \"READY\" then
                        local ready_data = {}; 
                        local packet_data = {};
                        packet_data.attribute = \"FavsNeutral\";
                        packet_data.receiver = packet.object3;
                        packet_data.target = packet.object1;
                        packet_data.object3 = packet.object1;
                        ready_data.SetTag = packet_data; 
                        send(sender, ready_data);
                        ready_data.SetTag.attribute = \"AP\";
                        send(sender, ready_data);
                    else
                        print(packet.object1.id, packet.action);
                    end
                end"
                .into(),
            );
        }
        self.lua.lock().procs = std::mem::take(&mut self.data.luas);
        Ok(())
    }

    pub async fn init_add_player(&mut self, new_player: Arc<Mutex<User>>) -> Result<(), Error> {
        let mut np_lock = new_player.lock().await;
        let map_data = self.data.map_data.clone();
        let obj = np_lock.create_object_header();
        let Some(p) = &np_lock.get_current_party() else {
            return Err(Error::InvalidInput("init_add_player"));
        };
        let party_obj = p.read().await.get_obj();
        np_lock
            .send_packet(&Packet::LoadLevel(LoadLevelPacket {
                host: obj,
                party: party_obj,
                world_obj: ObjectHeader {
                    id: map_data.settings.world_id,
                    entity_type: ObjectType::World,
                    ..Default::default()
                },
                quest: self.quest_obj,
                ..map_data
            }))
            .await?;
        drop(np_lock);
        self.add_player(new_player, self.data.init_map).await
    }

    pub async fn move_player_named(&mut self, id: PlayerId, name: &str) -> Result<(), Error> {
        let Some(zone) = self.zones.iter().find(|z| z.data.name == name) else {
            return Err(Error::InvalidInput("move_player_named"));
        };
        self.move_player(id, zone.srv_zone_id).await
    }

    /// [pso2_vita_offline] 03-05 `MoveZone`: looks the (client zone id, door) pair up in the warps the same way the
    /// client does (`0x81419826`: forward on (zone_id, door_id), backward on (dest_zone, backdoor_id) when unk1 & 2)
    /// and moves the player to the zone whose settings carry the destination's client zone id.
    pub async fn move_zone(&mut self, id: PlayerId, zone_id: u32, door_id: u32) -> Result<(), Error> {
        let dest = self.data.map_data.warps.iter().find_map(|w| {
            if w.zone_id == zone_id && w.door_id == door_id {
                Some(w.dest_zone)
            } else if w.dest_zone == zone_id && w.backdoor_id == door_id && w.unk1 & 2 != 0 {
                Some(w.zone_id)
            } else {
                None
            }
        });
        log::info!("[pso2-quest] MoveZone zone {zone_id} door {door_id} -> {dest:?}");
        let Some(dest) = dest else {
            return Ok(());
        };
        let Some(zone) = self
            .zones
            .iter()
            .find(|z| !z.data.is_special_zone && z.data.settings.zone_id == dest)
        else {
            return Err(Error::InvalidInput("move_zone"));
        };
        self.move_player(id, zone.srv_zone_id).await
    }

    async fn move_player(&mut self, id: PlayerId, srv_zone_id: ZoneId) -> Result<(), Error> {
        let Some(player) = self.remove_player(id).await else {
            return Err(Error::NoUserInMap(id, self.data.map_data.unk7.to_string()));
        };
        let Some(zone) = self.zones.iter().find(|z| z.srv_zone_id == srv_zone_id) else {
            return Err(Error::NoMapInMapSet(
                srv_zone_id,
                self.data.map_data.unk7.to_string(),
            ));
        };
        let mut lock = player.lock().await;
        let pid = lock.get_user_id();
        lock.send_packet(&Packet::MapTransfer(MapTransferPacket {
            map: zone.obj,
            target: ObjectHeader {
                id: pid,
                entity_type: ObjectType::Player,
                ..Default::default()
            },
            settings: zone.data.settings.clone(),
        }))
        .await?;
        drop(lock);
        self.add_player(player, srv_zone_id).await
    }

    pub async fn move_to_lobby(&mut self, id: PlayerId) -> Result<(), Error> {
        if matches!(self.map_type, MapType::Lobby) {
            return Ok(());
        }
        let Some(player) = self.remove_player(id).await else {
            return Err(Error::NoUserInMap(id, self.data.map_data.unk7.to_string()));
        };
        let lobby = player.lock().await.get_blockdata().lobby.clone();
        player.lock().await.set_map(lobby.clone());
        let mut lock = lobby.lock().await;
        lock.init_add_player(player).await
    }

    async fn add_player(
        &mut self,
        new_player: Arc<Mutex<User>>,
        srv_zone_id: ZoneId,
    ) -> Result<(), Error> {
        let Some((pos, zone)) = self
            .zones
            .iter_mut()
            .enumerate()
            .find(|(_, z)| z.srv_zone_id == srv_zone_id)
        else {
            return Err(Error::InvalidInput("add_player"));
        };
        let p_id = new_player.lock().await.get_user_id();
        zone.add_player(new_player.clone()).await?;
        self.player_cache.push((p_id, pos));
        Ok(())
    }

    pub async fn remove_player(&mut self, id: PlayerId) -> Option<Arc<Mutex<User>>> {
        let zone_pos = self.find_player(id)?;
        let (player_pos, _) = self
            .player_cache
            .iter()
            .enumerate()
            .find(|(_, (i, _))| *i == id)?;
        self.player_cache.remove(player_pos);
        self.zones[zone_pos].remove_player(id).await
    }

    fn find_player(&self, id: PlayerId) -> Option<usize> {
        self.player_cache
            .iter()
            .find(|(p_id, _)| *p_id == id)
            .map(|(_, z_pos)| *z_pos)
    }

    pub async fn send_palette_change(
        &self,
        zone_pos: usize,
        sender_id: PlayerId,
    ) -> Result<(), Error> {
        self.zones[zone_pos].send_palette_change(sender_id).await
    }
    pub async fn send_to_all(&self, zone_pos: usize, packet: &Packet) {
        exec_users(&self.zones[zone_pos].players, |_, mut player| {
            let _ = player.try_send_packet(packet);
        })
        .await;
    }

    pub async fn send_movement(&self, zone_pos: usize, packet: Packet, sender_id: PlayerId) {
        self.zones[zone_pos].send_movement(packet, sender_id).await;
    }

    pub async fn send_message(&self, zone_pos: usize, packet: Packet, id: PlayerId) {
        self.zones[zone_pos].send_message(packet, id).await;
    }

    pub async fn send_sa(&self, zone_pos: usize, data: SendSymbolArtPacket, id: PlayerId) {
        self.zones[zone_pos].send_sa(data, id).await;
    }

    pub async fn spawn_enemy(
        &mut self,
        zone_pos: usize,
        name: &str,
        pos: Position,
    ) -> Result<(), Error> {
        self.spawn_enemy_as(zone_pos, name, pos, false).await
    }
    /// [pso2_vita_offline] `spawn_enemy`; `boss` makes it the zone's boss (the quest clears when it dies)
    pub async fn spawn_enemy_as(
        &mut self,
        zone_pos: usize,
        name: &str,
        pos: Position,
        boss: bool,
    ) -> Result<(), Error> {
        let Some(block_data) = self.block_data.to_owned() else {
            return Err(Error::NoEnemyData(name.to_string()));
        };
        let id = self.zones[zone_pos]
            .spawn_enemy(&block_data, &mut self.max_id, self.enemy_level, name, pos)
            .await?;
        if boss {
            self.zones[zone_pos].boss_id = Some(id);
        }
        Ok(())
    }
    pub async fn deal_damage(
        &mut self,
        zone_pos: usize,
        dmg: DealDamagePacket,
    ) -> Result<(), Error> {
        let Some(block_data) = self.block_data.to_owned() else {
            return Err(Error::InvalidInput("deal_damage"));
        };
        let killed = self.zones[zone_pos].deal_damage(block_data, dmg).await?;
        // [pso2_vita_offline] drop tables (drops.rs): enemy -> common -> area -> meseta, spread on a small ring around
        // where the enemy was spawned
        if let Some((_, enemy_pos, _, name)) = &killed {
            let area = self.zones[zone_pos].data.name.clone();
            let items = crate::drops::roll(name, &area, self.enemy_level);
            log::info!(
                "[pso2-drop] {name} in {area}: {}",
                items.iter().map(crate::drops::describe).collect::<Vec<_>>().join(" ")
            );
            let n = items.len();
            for (i, item) in items.into_iter().enumerate() {
                let mut pos = *enemy_pos;
                if n > 1 {
                    let a = i as f32 * std::f32::consts::TAU / n as f32;
                    pos.pos_x = half::f16::from_f32(pos.pos_x.to_f32() + 0.8 * a.cos());
                    pos.pos_z = half::f16::from_f32(pos.pos_z.to_f32() + 0.8 * a.sin());
                }
                let model = crate::drops::model(&item);
                self.spawn_drop(zone_pos, item, pos, model).await;
            }
        }
        // [pso2_vita_offline] quest clear when the zone's boss dies, or, in a zone without a boss, when every
        // enemy spawned so far by the zone's chunks is dead (chunks not entered yet are not counted). The telepipe
        // goes where the last hit came from: the client closes its dialog when the player is more than ~3 m away
        // from it.
        if let (MapType::QuestMap, Some((pos, _, id, _))) = (&self.map_type, killed) {
            let zone = &self.zones[zone_pos];
            // a quest with a boss clears only when the boss dies: the zones before it never clear on their own
            let quest_has_boss = self.zones.iter().any(|z| z.boss_id.is_some() || z.data.boss.is_some());
            let clear = if zone.boss_id.is_some() || zone.data.boss.is_some() {
                zone.boss_id == Some(id)
            } else if quest_has_boss {
                false
            } else {
                zone.enemies.is_empty() && !zone.chunk_spawns.is_empty()
            };
            if clear {
                log::info!("[pso2-quest] clear: zone {} (boss {:?})", zone.data.name, zone.boss_id);
                self.quest_clear(zone_pos, pos).await?;
            }
        }
        Ok(())
    }

    /// [pso2_vita_offline] debug `tp pipe`: where the zone's clear telepipe is
    pub fn clear_pipe_pos(&self, zone_pos: usize) -> Option<Position> {
        self.zones.get(zone_pos)?.clear_pipe.map(|(_, p)| p)
    }

    /// [pso2_vita_offline] Quest clear: spawns the clear telepipe (`oa_telepipe_clear`) at `pos` for everyone in the
    /// zone and turns it On. The client then asks for its position ("InitPosition", see `interaction`); without the
    /// answer it stays where the client cannot reach it. Returns false if the zone is already cleared.
    pub async fn quest_clear(&mut self, zone_pos: usize, pos: Position) -> Result<bool, Error> {
        if self.zones[zone_pos].clear_pipe.is_some() {
            return Ok(false);
        }
        self.max_id += 1;
        let id = self.max_id;
        self.zones[zone_pos].spawn_clear_pipe(id, pos).await;
        // [pso2_vita_offline] the map's `on_quest_clear` Lua, once per player in the zone (story battles mark
        // themselves cleared and unlock the next quest here)
        let lua = self.lua.lock().procs.get("on_quest_clear").cloned();
        if let Some(lua) = lua {
            let players: Vec<_> = self.zones[zone_pos].players.iter().map(|p| p.player_id).collect();
            for player in players {
                self.run_lua(player, zone_pos, &Packet::None, "on_quest_clear", &lua)
                    .await?;
            }
            self.check_move_lua().await?;
        }
        Ok(true)
    }

    /// [pso2_vita_offline] Puts `item` on the ground at `pos` for everyone in the zone: NewItemDrop (0F-04)
    /// registers it, ObjectSpawn (08-0B) of `model` with the same header makes it visible (no ObjectSpawn when
    /// `model` is empty). Returns the drop id.
    pub async fn spawn_drop(
        &mut self,
        zone_pos: usize,
        item: protocol::items::Item,
        pos: Position,
        model: &str,
    ) -> u32 {
        self.max_id += 1;
        let id = self.max_id;
        self.zones[zone_pos].spawn_drop(id, item, pos, model).await;
        id
    }

    /// [pso2_vita_offline] debug `ehp`: sets the HP of every enemy in the zone; returns how many.
    /// [pso2_vita_offline] The zone's boss object id once it has spawned (debug `ekill`).
    pub fn zone_boss_id(&self, zone_pos: usize) -> Option<u32> {
        self.zones[zone_pos].boss_id
    }

    pub fn set_enemy_hp(&mut self, zone_pos: usize, hp: u32) -> usize {
        let enemies = &mut self.zones[zone_pos].enemies;
        enemies.iter_mut().for_each(|(_, e)| e.set_hp(hp));
        enemies.len()
    }

    /// [pso2_vita_offline] Removes a drop (picked up) and despawns it for everyone in the zone.
    pub async fn take_drop(&mut self, zone_pos: usize, drop_id: u32) -> Option<FieldDrop> {
        self.zones[zone_pos].take_drop(drop_id).await
    }

    pub async fn minimap_reveal(
        &mut self,
        zone_pos: usize,
        sender_id: PlayerId,
        packet: protocol::questlist::MinimapRevealRequestPacket,
    ) -> Result<(), Error> {
        let Some(block_data) = self.block_data.to_owned() else {
            return Err(Error::InvalidInput("minimap_reveal: no block data"));
        };

        self.zones[zone_pos]
            .minimap_reveal(
                sender_id,
                &block_data,
                &mut self.max_id,
                self.enemy_level,
                &packet,
            )
            .await?;

        self.check_move_lua().await?;
        Ok(())
    }

    pub async fn interaction(
        &mut self,
        zone_pos: usize,
        packet: protocol::objects::InteractPacket,
        sender_id: PlayerId,
    ) -> Result<(), Error> {
        if packet.action == "InitPosition"
            && self.zones[zone_pos]
                .answer_init_position(sender_id, packet.object1)
                .await?
        {
            return Ok(());
        }
        let zone_id = self.zones[zone_pos].srv_zone_id;
        let Some(lua_data) = self
            .data
            .objects
            .iter()
            .filter(|o| o.zone_id == zone_id)
            .map(|x| (x.data.object.id, &x.data.name))
            .chain(
                self.data
                    .npcs
                    .iter()
                    .filter(|o| o.zone_id == zone_id)
                    .map(|x| (x.data.object.id, &x.data.name)),
            )
            .find(|(id, _)| *id == packet.object1.id)
            .and_then(|(_, name)| self.lua.lock().procs.get(name.as_str()).cloned())
        else {
            return Ok(());
        };
        self.run_lua(sender_id, zone_pos, &packet, "interaction", &lua_data)
            .await?;
        Ok(())
    }
    pub async fn on_questwork(
        &mut self,
        zone_pos: usize,
        player: PlayerId,
        packet: SkitItemAddRequestPacket,
    ) -> Result<(), Error> {
        let Some(lua) = self.lua.lock().procs.get("on_questwork").cloned() else {
            // [pso2_vita_offline] A skit waits for the answer (black screen otherwise). Maps without their own
            // `on_questwork` Lua echo the request back, as Story EP1 700020's Lua does. Story quests are solo.
            self.send_to_all(
                zone_pos,
                &Packet::SkitItemAddResponse(SkitItemAddResponsePacket {
                    skit_name: packet.skit_name,
                    unk: packet.unk,
                }),
            )
            .await;
            return Ok(());
        };
        self.run_lua(player, zone_pos, &packet, "on_questwork", &lua)
            .await?;
        self.check_move_lua().await?;
        Ok(())
    }
    pub async fn on_cutscene_end(
        &mut self,
        zone_pos: usize,
        player: PlayerId,
        packet: CutsceneEndPacket,
    ) -> Result<(), Error> {
        let Some(lua) = self.lua.lock().procs.get("on_cutscene_end").cloned() else {
            return Ok(());
        };
        self.run_lua(player, zone_pos, &packet, "on_cutscene_end", &lua)
            .await?;
        self.check_move_lua().await?;
        Ok(())
    }

    pub async fn on_map_loaded(&mut self, zone_pos: usize, player: PlayerId) -> Result<(), Error> {
        let Some(lua) = self.lua.lock().procs.get("on_map_loaded").cloned() else {
            return Ok(());
        };
        self.run_lua(player, zone_pos, &Packet::None, "on_map_loaded", &lua)
            .await?;
        self.check_move_lua().await?;
        Ok(())
    }
    pub fn get_close_objects<F>(&self, zone_pos: usize, pred: F) -> Vec<ObjectSpawnPacket>
    where
        F: Fn(&Position) -> bool,
    {
        let mut obj = vec![];
        let zone_id = self.zones[zone_pos].srv_zone_id;
        for self_obj in self.data.objects.iter().filter(|o| o.zone_id == zone_id) {
            if pred(&self_obj.data.position) {
                obj.push(self_obj.data.clone());
            }
        }

        obj
    }

    async fn run_lua<S: serde::Serialize + Sync>(
        &mut self,
        sender_id: PlayerId,
        zone_id: usize,
        packet: &S,
        call_type: &str,
        lua_data: &str,
    ) -> Result<(), Error> {
        spawn_blocking(|| {
            self.zones[zone_id].run_lua_blocking(sender_id, packet, call_type, lua_data)
        })
        .await?
    }
    async fn check_move_lua(&mut self) -> Result<(), Error> {
        let mut lua = self.lua.lock();
        let to_move: Vec<_> = lua.to_move.drain(..).collect();
        let to_lobby_move: Vec<_> = lua.to_lobby_move.drain(..).collect();
        drop(lua);
        for (player, zone) in to_move {
            self.move_player_named(player, &zone).await?;
        }
        for player in to_lobby_move {
            self.move_to_lobby(player).await?;
        }
        Ok(())
    }
}

impl Zone {
    async fn add_player(&mut self, new_player: Arc<Mutex<User>>) -> Result<(), Error> {
        let mut other_equipment = Vec::with_capacity(self.players.len() * 2);
        let mut other_characters = Vec::with_capacity(self.players.len());
        for player in self.players.iter().filter_map(|p| p.user.upgrade()) {
            let p = player.lock().await;
            let pid = p.get_user_id();
            let Some(char_data) = &p.character else {
                unreachable!("User should be in state >= `PreInGame`")
            };
            other_equipment.push(char_data.palette.send_change_palette(pid));
            other_equipment.push(char_data.palette.send_cur_weapon(pid, &char_data.inventory));
            other_equipment.push(char_data.inventory.send_equiped(pid));
            other_characters.push((char_data.character.clone(), p.position, p.user_data.isgm));
        }
        let mut np_lock = new_player.lock().await;
        np_lock.map_id = self.data.settings.map_id;
        np_lock.zone_pos = self.zone_pos;
        let np_id = np_lock.get_user_id();
        let Some(new_character) = np_lock.character.to_owned() else {
            unreachable!("User should be in state >= `PreInGame`")
        };
        np_lock
            .send_packet(&Packet::SetPlayerID(SetPlayerIDPacket {
                player_id: np_id,
                unk2: 4,
                ..Default::default()
            }))
            .await?;
        let mut pos = self.data.default_location;
        // [pso2_vita_offline] T25: one-shot spawn position from the debug port (`goto <zone> x y z`)
        crate::user::debug::apply_spawn_override(&self.data.name, &mut pos);
        np_lock.position = pos;
        let np_gm = np_lock.user_data.isgm as u32;
        np_lock
            .spawn_character(CharacterSpawnPacket {
                position: pos,
                character: new_character.character.clone(),
                spawn_type: CharacterSpawnType::Myself,
                gm_flag: np_gm,
                player_obj: ObjectHeader {
                    id: np_id,
                    entity_type: ObjectType::Player,
                    ..Default::default()
                },
                ..Default::default()
            })
            .await?;
        Self::load_objects(&self.lua, &self.objects, &mut np_lock)?;
        for (character, position, isgm) in other_characters {
            let player_id = character.player_id;
            np_lock
                .spawn_character(CharacterSpawnPacket {
                    position,
                    spawn_type: CharacterSpawnType::Other,
                    gm_flag: isgm as u32,
                    player_obj: ObjectHeader {
                        id: player_id,
                        entity_type: ObjectType::Player,
                        ..Default::default()
                    },
                    character,
                    ..Default::default()
                })
                .await?;
        }
        for equipment in other_equipment {
            np_lock.send_packet(&equipment).await?;
        }
        let new_eqipment = (
            new_character.palette.send_change_palette(np_id),
            new_character
                .palette
                .send_cur_weapon(np_id, &new_character.inventory),
            new_character.inventory.send_equiped(np_id),
        );

        for (id, enemy) in &self.enemies {
            let (packet, mut packet2) = Self::prepare_enemy_packets(*id, enemy);
            if let Packet::EnemyAction(data) = &mut packet2 {
                data.receiver = np_lock.create_object_header();
                data.action_starter = np_lock.create_object_header();
            }
            np_lock.send_packet(&packet).await?;
            np_lock.send_packet(&packet2).await?;
        }
        drop(np_lock);

        exec_users(&self.players, |_, mut player| {
            let _ = player.try_spawn_character(CharacterSpawnPacket {
                position: pos,
                spawn_type: CharacterSpawnType::Other,
                gm_flag: np_gm,
                player_obj: ObjectHeader {
                    id: new_character.character.player_id,
                    entity_type: ObjectType::Player,
                    ..Default::default()
                },
                character: new_character.character.clone(),
                ..Default::default()
            });
            let _ = player.try_send_packet(&new_eqipment.0);
            let _ = player.try_send_packet(&new_eqipment.1);
            let _ = player.try_send_packet(&new_eqipment.2);
        })
        .await;
        self.players.push(MapPlayer {
            player_id: np_id,
            user: Arc::downgrade(&new_player),
        });

        let Some(lua) = self.lua.lock().procs.get("on_player_load").cloned() else {
            return Ok(());
        };
        self.run_lua(np_id, &Packet::None, "on_player_load", &lua)
            .await?;

        Ok(())
    }

    fn load_objects(
        lua: &parking_lot::Mutex<LuaState>,
        map_data: &Objects,
        user: &mut User,
    ) -> Result<(), Error> {
        let lua_lock = lua.lock();
        let lua = &lua_lock.lua;
        for mut obj in map_data.objects.iter().cloned() {
            if user.user_data.packet_type == PacketType::Vita {
                let lua_code = lua_lock
                    .procs
                    .get(obj.data.name.as_str())
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let globals = lua.globals();
                globals.set("data", obj.data.data.as_slice())?;
                globals.set("call_type", "to_vita")?;
                globals.set("size", obj.data.data.len())?;
                let chunk = lua.load(lua_code);
                chunk.exec()?;
                obj.data.data = globals.get::<Vec<u32>>("data")?.into();
                globals.raw_remove("data")?;
                globals.raw_remove("call_type")?;
                globals.raw_remove("size")?;
            }
            user.try_send_packet(&Packet::ObjectSpawn(obj.data))?;
        }
        for npc in map_data.npcs.iter().cloned() {
            user.try_send_packet(&Packet::NPCSpawn(npc.data))?;
        }
        for event in map_data.events.iter().cloned() {
            user.try_send_packet(&Packet::EventSpawn(event.data))?;
        }
        for tele in map_data.transporters.iter().cloned() {
            user.try_send_packet(&Packet::TransporterSpawn(tele.data))?;
        }

        Ok(())
    }

    async fn remove_player(&mut self, id: PlayerId) -> Option<Arc<Mutex<User>>> {
        let (pos, _) = self
            .players
            .iter()
            .enumerate()
            .find(|(_, p)| p.player_id == id)?;
        let user = self.players.swap_remove(pos);
        let mut packet = Packet::DespawnPlayer(protocol::objects::DespawnPlayerPacket {
            receiver: ObjectHeader {
                id: 0,
                entity_type: ObjectType::Player,
                ..Default::default()
            },
            removed_player: ObjectHeader {
                id,
                entity_type: ObjectType::Player,
                ..Default::default()
            },
        });
        exec_users(&self.players, |_, mut player| {
            if let Packet::DespawnPlayer(data) = &mut packet {
                data.receiver.id = player.get_user_id();
                let _ = player.try_send_packet(&packet);
            }
        })
        .await;
        user.user.upgrade()
    }

    async fn send_palette_change(&self, sender_id: PlayerId) -> Result<(), Error> {
        let Some(user) = self.players.iter().find(|p| p.player_id == sender_id) else {
            return Err(Error::NoUserInMap(sender_id, self.data.name.clone()));
        };
        let Some(player) = user.user.upgrade() else {
            return Err(Error::InvalidInput("send_palette_change"));
        };
        let new_eqipment = {
            let p = player.lock().await;
            let Some(character) = &p.character else {
                unreachable!("Users in map should have characters")
            };
            (
                character.palette.send_change_palette(sender_id),
                character
                    .palette
                    .send_cur_weapon(sender_id, &character.inventory),
            )
        };
        exec_users(&self.players, |_, mut player| {
            let _ = player.try_send_packet(&new_eqipment.0);
            let _ = player.try_send_packet(&new_eqipment.1);
        })
        .await;

        Ok(())
    }
    async fn send_movement(&self, packet: Packet, sender_id: PlayerId) {
        let mut out_packet = match packet {
            Packet::Movement(_) => packet,
            Packet::MovementEnd(mut data) => {
                if data.unk1.id == 0 && data.unk2.id != 0 {
                    data.unk1 = data.unk2;
                }
                Packet::MovementEnd(data)
            }
            Packet::MovementAction(data) => {
                log::debug!("[pso2-action] {data:?}");
                let packet = protocol::objects::MovementActionServerPacket {
                    performer: data.performer,
                    receiver: ObjectHeader {
                        id: 0,
                        entity_type: ObjectType::Player,
                        ..Default::default()
                    },
                    unk3: data.unk3,
                    unk4: data.unk4,
                    unk5: data.unk5,
                    unk6: data.unk6,
                    action: data.action,
                    unk7: data.unk7,
                    unk8: data.unk8,
                    unk9: data.unk9,
                    unk10: data.unk10,
                };
                Packet::MovementActionServer(packet)
            }
            Packet::ActionUpdate(data) => {
                let packet = protocol::objects::ActionUpdateServerPacket {
                    performer: data.performer,
                    unk2: data.unk2,
                    receiver: ObjectHeader {
                        id: 0,
                        entity_type: ObjectType::Player,
                        ..Default::default()
                    },
                };
                Packet::ActionUpdateServer(packet)
            }
            Packet::ActionEnd(mut data) => {
                data.unk1 = data.performer;
                Packet::ActionEnd(data)
            }
            _ => return,
        };
        exec_users(&self.players, |user, mut player| {
            if let Packet::MovementActionServer(ref mut data) = out_packet {
                data.receiver.id = player.get_user_id();
            } else if let Packet::ActionUpdateServer(ref mut data) = out_packet {
                data.receiver.id = player.get_user_id();
            }
            if user.player_id != sender_id {
                let _ = player.try_send_packet(&out_packet);
            }
        })
        .await;
    }

    async fn send_message(&self, mut packet: Packet, id: PlayerId) {
        if let Packet::ChatMessage(ref mut data) = packet {
            data.object = ObjectHeader {
                id,
                entity_type: ObjectType::Player,
                ..Default::default()
            };
        }
        exec_users(&self.players, |_, mut player| {
            let _ = player.try_send_packet(&packet);
        })
        .await;
    }

    async fn send_sa(&self, data: SendSymbolArtPacket, id: PlayerId) {
        let packet = Packet::ReceiveSymbolArt(ReceiveSymbolArtPacket {
            object: ObjectHeader {
                id,
                entity_type: ObjectType::Player,
                ..Default::default()
            },
            uuid: data.uuid,
            area: data.area,
            unk1: data.unk1,
            unk2: data.unk2,
            unk3: data.unk3,
        });
        exec_users(&self.players, |_, mut player| {
            let _ = player.try_send_packet(&packet);
        })
        .await;
    }

    fn prepare_enemy_packets(enemy_id: u32, enemy: &EnemyStats) -> (Packet, Packet) {
        let packet = enemy.create_spawn_packet(enemy_id);
        // techically this is a response to 0x04 0x2B
        let packet2 = Packet::EnemyAction(EnemyActionPacket {
            actor: packet.object,
            action_id: 7,
            ..Default::default()
        });
        let packet = Packet::EnemySpawn(packet);
        (packet, packet2)
    }

    async fn spawn_drop(&mut self, id: u32, item: protocol::items::Item, pos: Position, model: &str) {
        use pso2packetlib::protocol::items::NewItemDropPacket;
        let obj = ObjectHeader {
            id,
            entity_type: ObjectType::Object,
            map_id: self.data.settings.world_id as u16,
            ..Default::default()
        };
        let drop = Packet::NewItemDrop(NewItemDropPacket {
            item_obj: obj,
            item_id: item.id,
            unk1: meseta_amount(&item).unwrap_or(0),
            pos,
            drop_id: id,
            ..Default::default()
        });
        // data of the item drops in quest 50570's captured objects (PC property ids; 66, 2 = Vita 0x41 == 2,
        // which the client's drop state checks)
        const DATA: [u32; 19] = [
            16778264, 0, 2, 0, 0, 0, 0, 0, 58, u32::MAX, 55, 1, 57, 4, 66, 2, 4, 0, 0,
        ];
        let spawn = (!model.is_empty()).then(|| {
            Packet::ObjectSpawn(ObjectSpawnPacket {
                object: obj,
                position: pos,
                name: model.to_string().into(),
                unk2: [9, 0, 0, 0, 0],
                flags: 4,
                data: crate::user::debug::pc_to_vita_object_data(&DATA).into(),
                ..Default::default()
            })
        });
        log::info!(
            "[pso2-drop] drop {id} {:?} model {model:?} at ({:.2}, {:.2}, {:.2})",
            item.id,
            pos.pos_x.to_f32(),
            pos.pos_y.to_f32(),
            pos.pos_z.to_f32()
        );
        self.drops.push(FieldDrop {
            drop_id: id,
            obj,
            item,
        });
        // 0F-07 first: the client's drop registry ignores NewItemDrop until this sets its session value (only once,
        // while it is 0); no drops listed
        let list = drop_list(obj, DROP_SESSION);
        exec_users(&self.players, |_, mut player| {
            let _ = player.try_send_packet(&list);
            let _ = player.try_send_packet(&drop);
            if let Some(spawn) = &spawn {
                let _ = player.try_send_packet(spawn);
            }
        })
        .await;
    }

    async fn take_drop(&mut self, drop_id: u32) -> Option<FieldDrop> {
        let pos = self.drops.iter().position(|d| d.drop_id == drop_id)?;
        let drop = self.drops.remove(pos);
        exec_users(&self.players, |_, mut player| {
            let me = player.create_object_header();
            let _ = player.try_send_packet(&Packet::DespawnObject(
                protocol::objects::DespawnObjectPacket {
                    player: me,
                    item: drop.obj,
                },
            ));
        })
        .await;
        Some(drop)
    }

    /// [pso2_vita_offline] `Map::quest_clear`. The object data is Story EP1 700000's `oa_telepipe_clear` with the PC
    /// property ids turned into the Vita ones (66, 2 = Vita `get(0x41) == 2`, the final return 03-19).
    async fn spawn_clear_pipe(&mut self, id: u32, pos: Position) {
        use pso2packetlib::protocol::objects::SetTagPacket;
        const DATA: [u32; 22] = [
            1304, 1, 2, 0, 0, 0, 1677721856, 0, 58, u32::MAX, 55, 1, 57, 0, 66, 2, 67, 0, 2, 0, 0, 0,
        ];
        self.clear_pipe = Some((id, pos));
        let obj = ObjectHeader {
            id,
            entity_type: ObjectType::Object,
            map_id: self.data.settings.world_id as u16,
            ..Default::default()
        };
        let spawn = Packet::ObjectSpawn(ObjectSpawnPacket {
            object: obj,
            position: pos,
            name: "oa_telepipe_clear".to_string().into(),
            unk2: [16, 0, 0, 0, 0],
            flags: 4,
            data: crate::user::debug::pc_to_vita_object_data(&DATA).into(),
            ..Default::default()
        });
        log::info!(
            "[pso2-quest] clear: telepipe {id} at ({:.2}, {:.2}, {:.2})",
            pos.pos_x.to_f32(),
            pos.pos_y.to_f32(),
            pos.pos_z.to_f32()
        );
        exec_users(&self.players, |_, mut player| {
            let me = player.create_object_header();
            let _ = player.try_send_packet(&spawn);
            // "On": the client sends Interact "InitPosition" for it (answered in `answer_init_position`)
            let _ = player.try_send_packet(&Packet::SetTag(SetTagPacket {
                receiver: me,
                target: obj,
                object3: me,
                attribute: "On".into(),
                ..Default::default()
            }));
        })
        .await;
    }

    /// [pso2_vita_offline] Answer to Interact "InitPosition" for the clear telepipe: its position (Object Teleport
    /// Location 04-02; until then the client keeps it out of reach and closes the "return to the campship?" dialog
    /// at once) and the quest state "cleared" (0B-23; only then the client shows it and turns its collision on).
    async fn answer_init_position(&self, sender_id: PlayerId, object: ObjectHeader) -> Result<bool, Error> {
        use pso2packetlib::protocol::objects::TeleportTransferPacket;
        let Some((id, pos)) = self.clear_pipe else {
            return Ok(false);
        };
        if object.id != id {
            return Ok(false);
        }
        let Some(user) = self
            .players
            .iter()
            .find(|p| p.player_id == sender_id)
            .and_then(|p| p.user.upgrade())
        else {
            return Ok(true);
        };
        let party = user.lock().await.get_current_party();
        let party = match party {
            Some(p) => p.read().await.get_obj(),
            None => ObjectHeader {
                entity_type: ObjectType::Party,
                ..Default::default()
            },
        };
        let world = ObjectHeader {
            id: self.data.settings.world_id,
            entity_type: ObjectType::World,
            ..Default::default()
        };
        let mut lock = user.lock().await;
        lock.send_packet(&Packet::TeleportTransfer(TeleportTransferPacket {
            source_tele: object,
            location: pos,
            ..Default::default()
        }))
        .await?;
        lock.send_packet(&crate::user::handlers::server::quest_state(
            world,
            party,
            crate::user::handlers::server::QUEST_STATE_CLEARED,
        ))
        .await?;
        Ok(true)
    }

    async fn spawn_enemy(
        &mut self,
        block_data: &BlockData,
        max_id: &mut u32,
        enemy_lvl: u32,
        name: &str,
        pos: Position,
    ) -> Result<u32, Error> {
        let id = *max_id + 1;
        *max_id += 1;
        let data = EnemyStats::build(name, enemy_lvl, pos, &block_data.server_data)?;
        let (packet, mut packet2) = Zone::prepare_enemy_packets(id, &data);
        self.enemies.push((id, data));

        exec_users(&self.players, |_, mut player| {
            let _ = player.try_send_packet(&packet);
            if let Packet::EnemyAction(data) = &mut packet2 {
                data.receiver = player.create_object_header();
                data.action_starter = player.create_object_header();
                let _ = player.try_send_packet(&packet2);
            }
        })
        .await;

        Ok(id)
    }

    async fn deal_damage(
        &mut self,
        block_data: Arc<BlockData>,
        dmg: DealDamagePacket,
    ) -> Result<Option<(Position, Position, u32, String)>, Error> {
        // [pso2_vita_offline] (last hitter's position, enemy's position, enemy id) when an enemy died
        let mut killed = None;
        let (inflicter, target) = (dmg.inflicter, dmg.target);
        log::debug!(
            "[pso2-battle] DealDamage {:?} {} -> {:?} {} attack {} hitbox {}",
            inflicter.entity_type,
            inflicter.id,
            target.entity_type,
            target.id,
            dmg.attack_id,
            dmg.hitbox_id
        );
        if inflicter.entity_type == ObjectType::Player && target.entity_type == ObjectType::Player {
            // [pso2_vita_offline] a recovery item on oneself (see consumables.rs): heal, one off the stack,
            // UpdateInventory to the user and DamageReceive with the new HP to everyone
            let Some(item) = crate::consumables::by_attack(dmg.attack_id) else {
                log::warn!("[pso2-item] unknown self attack {} {dmg:?}", dmg.attack_id);
                return Ok(None);
            };
            if inflicter.id != target.id {
                log::warn!("[pso2-item] {} on another player is not handled", item.skill);
                return Ok(None);
            }
            let Some(user) = self
                .players
                .iter()
                .find(|u| u.player_id == target.id)
                .and_then(|p| p.user.upgrade())
            else {
                return Err(Error::InvalidInput("deal_damage"));
            };
            let mut lock = user.lock().await;
            let Some(inv_packet) = lock
                .character
                .as_mut()
                .and_then(|c| c.inventory.use_consumable(item.item_id()))
            else {
                log::warn!("[pso2-item] {} used but none held", item.skill);
                return Ok(None);
            };
            let (hp, max) = lock.get_stats().get_hp();
            let new_hp = if hp == 0 { 0 } else { (hp + max * item.hp_percent / 100).min(max) };
            lock.get_stats_mut().set_hp(new_hp);
            log::info!("[pso2-item] {} hp {hp} -> {new_hp}/{max} (+{}%)", item.skill, item.hp_percent);
            lock.try_send_packet(&inv_packet)?;
            drop(lock);
            let mut packet = Packet::DamageReceive(DamageReceivePacket {
                dmg_target: target,
                dmg_inflicter: inflicter,
                damage_id: dmg.attack_id,
                dmg_amount: -((new_hp - hp) as i32),
                new_hp,
                hitbox_id: dmg.hitbox_id,
                x_pos: dmg.x_pos,
                y_pos: dmg.y_pos,
                z_pos: dmg.z_pos,
                ..Default::default()
            });
            exec_users(&self.players, |_, mut player| {
                if let Packet::DamageReceive(data) = &mut packet {
                    data.receiver = player.create_object_header();
                    let _ = player.try_send_packet(&packet);
                }
            })
            .await;
            return Ok(None);
        }
        if inflicter.entity_type == ObjectType::Player && target.entity_type == ObjectType::Object {
            let Some((enemy_pos, (_, target))) = self
                .enemies
                .iter_mut()
                .enumerate()
                .find(|(_, (id, _))| *id == target.id)
            else {
                return Ok(None);
            };
            let Some(inflicter) = self
                .players
                .iter()
                .find(|u| u.player_id == inflicter.id)
                .and_then(|p| p.user.upgrade())
            else {
                return Err(Error::InvalidInput("deal_damage"));
            };
            let mut lock = inflicter.lock().await;
            let result = lock
                .get_stats_mut()
                .damage_enemy(target, &block_data.server_data, dmg)?;
            let inflicter_pos = lock.position;
            let enemy_xyz = target.position();
            drop(lock);
            match result {
                BattleResult::Damaged { dmg_packet } => {
                    let mut packet = Packet::DamageReceive(dmg_packet);
                    exec_users(&self.players, |_, mut player| {
                        if let Packet::DamageReceive(data) = &mut packet {
                            data.receiver = player.create_object_header();
                            let _ = player.try_send_packet(&packet);
                        }
                    })
                    .await;
                }
                BattleResult::Killed {
                    dmg_packet,
                    kill_packet,
                    exp_amount,
                } => {
                    let mut action_packet = Packet::EnemyAction(EnemyActionPacket {
                        actor: dmg_packet.dmg_target,
                        action_starter: dmg_packet.dmg_inflicter,
                        action_id: 4,
                        ..Default::default()
                    });
                    let mut dmg_packet = Packet::DamageReceive(dmg_packet);
                    let mut kill_packet = Packet::EnemyKilled(kill_packet);
                    let mut exp_packets = vec![];
                    exec_users(&self.players, |_, mut player| {
                        exp_packets.push(player.add_exp(exp_amount))
                    })
                    .await;
                    let exp_packets = exp_packets.into_iter().collect::<Result<Vec<_>, _>>()?;
                    let mut exp_packet = Packet::GainedEXP(GainedEXPPacket {
                        receivers: exp_packets,
                        ..Default::default()
                    });
                    exec_users(&self.players, |_, mut player| {
                        if let Packet::DamageReceive(data) = &mut dmg_packet {
                            data.receiver = player.create_object_header();
                            let _ = player.try_send_packet(&dmg_packet);
                        }
                        if let Packet::EnemyKilled(data) = &mut kill_packet {
                            data.receiver = player.create_object_header();
                            let _ = player.try_send_packet(&kill_packet);
                        }
                        if let Packet::GainedEXP(data) = &mut exp_packet {
                            data.sender = player.create_object_header();
                            let _ = player.try_send_packet(&exp_packet);
                        }
                        if let Packet::EnemyAction(data) = &mut action_packet {
                            data.receiver = player.create_object_header();
                            let _ = player.try_send_packet(&action_packet);
                        }
                    })
                    .await;
                    let (enemy_id, stats) = self.enemies.remove(enemy_pos);
                    killed = Some((inflicter_pos, enemy_xyz, enemy_id, stats.name().to_string()));
                }
            }
        } else if inflicter.entity_type == ObjectType::Object
            && target.entity_type == ObjectType::Player
        {
            let (target_id, inflicter_id) = (target.id, inflicter.id);
            let Some(target) = self
                .players
                .iter_mut()
                .find(|u| u.player_id == target.id)
                .and_then(|p| p.user.upgrade())
            else {
                return Err(Error::InvalidInput("deal_damage"));
            };
            let Some((_, inflicter)) = self.enemies.iter_mut().find(|(id, _)| *id == inflicter.id)
            else {
                return Ok(None);
            };
            let mut lock = target.lock().await;
            if lock.get_stats().get_hp().0 == 0 {
                // already incapacitated: the enemies keep hitting, don't kill again
                return Ok(None);
            }
            let result =
                inflicter.damage_player(lock.get_stats_mut(), &block_data.server_data, dmg)?;
            drop(lock);

            match result {
                BattleResult::Damaged { dmg_packet } => {
                    let mut packet = Packet::DamageReceive(dmg_packet);
                    exec_users(&self.players, |_, mut player| {
                        if let Packet::DamageReceive(data) = &mut packet {
                            data.receiver = player.create_object_header();
                            let _ = player.try_send_packet(&packet);
                        }
                    })
                    .await;
                }
                BattleResult::Killed {
                    dmg_packet,
                    kill_packet,
                    ..
                } => {
                    let mut dmg_packet = Packet::DamageReceive(dmg_packet);
                    let mut kill_packet = Packet::EnemyKilled(kill_packet);
                    exec_users(&self.players, |_, mut player| {
                        if let Packet::DamageReceive(data) = &mut dmg_packet {
                            data.receiver = player.create_object_header();
                            let _ = player.try_send_packet(&dmg_packet);
                        }
                        if let Packet::EnemyKilled(data) = &mut kill_packet {
                            data.receiver = player.create_object_header();
                            let _ = player.try_send_packet(&kill_packet);
                        }
                    })
                    .await;
                    // new_hp = 0 alone leaves the player standing; EnemyKilled (0x04-0x0F) with the player as the
                    // target makes the client fall and open its incapacitated menu. "Return to campship" there
                    // sends 03-1A (DeathToCampship), which heals on the campship.
                    log::info!("[pso2-battle] player {} incapacitated by {}", target_id, inflicter_id);
                }
            }
        }

        Ok(killed)
    }
    async fn minimap_reveal(
        &mut self,
        sender_id: PlayerId,
        block_data: &BlockData,
        max_id: &mut u32,
        enemy_lvl: u32,
        packet: &protocol::questlist::MinimapRevealRequestPacket,
    ) -> Result<(), Error> {
        let Some(user) = self
            .players
            .iter()
            .find(|p| p.player_id == sender_id)
            .cloned()
        else {
            return Err(Error::InvalidInput("minimap_reveal"));
        };

        // [pso2_vita_offline] the zone's boss, once (chunk_id u32::MAX: on the first chunk the client reports)
        if let Some(boss) = self.data.boss.clone() {
            if (boss.chunk_id == packet.chunk_id || boss.chunk_id == u32::MAX) && self.boss_id.is_none() {
                let id = self
                    .spawn_enemy(block_data, max_id, enemy_lvl, &boss.enemy_name, boss.position)
                    .await?;
                self.boss_id = Some(id);
                log::info!("[pso2-quest] boss {} spawned as {id} (chunk {})", boss.enemy_name, boss.chunk_id);
            }
        }

        if let Some(chunk) = self
            .data
            .chunks
            .iter()
            .find(|c| c.chunk_id == packet.chunk_id)
        {
            for reveal in &chunk.reveals {
                self.minimap_status[reveal.row as usize - 1].set(reveal.column as usize - 1, true);
            }

            // wow, how nested
            match chunk.enemy_spawn_type {
                data_structs::map::EnemySpawnType::Disabled => {}
                data_structs::map::EnemySpawnType::Automatic { min, max } => {
                    let count = rand::distributions::Uniform::new_inclusive(min, max)
                        .sample(&mut rand::thread_rng());
                    if !self.chunk_spawns.iter().any(|s| s.0 == chunk.chunk_id) {
                        self.chunk_spawns
                            .push((chunk.chunk_id, std::time::Instant::now()));
                        // this is length biased
                        let spawn_category = self
                            .data
                            .enemies
                            .iter()
                            .map(|e| e.spawn_category)
                            .choose(&mut rand::thread_rng())
                            .unwrap_or_default();
                        let spawn_point = chunk
                            .enemy_spawn_points
                            .iter()
                            .choose(&mut rand::thread_rng());
                        let spawn_point = match spawn_point {
                            Some(x) => *x,
                            None => user.user.upgrade().unwrap().lock().await.position,
                        };
                        for _ in 0..count {
                            let enemy = self
                                .data
                                .enemies
                                .iter()
                                .filter(|e| e.spawn_category == spawn_category)
                                .choose(&mut rand::thread_rng());
                            if let Some(enemy) = enemy {
                                let enemy_name = enemy.enemy_name.clone();
                                self.spawn_enemy(
                                    block_data,
                                    max_id,
                                    enemy_lvl,
                                    &enemy_name,
                                    spawn_point,
                                )
                                .await?;
                            }
                        }
                    }
                }
                data_structs::map::EnemySpawnType::AutomaticWithRespawn {
                    min,
                    max,
                    respawn_time,
                } => {
                    let count = rand::distributions::Uniform::new_inclusive(min, max)
                        .sample(&mut rand::thread_rng());
                    let (spawn, is_first) = if let Some(spawn) =
                        self.chunk_spawns.iter().find(|s| s.0 == chunk.chunk_id)
                    {
                        (spawn, false)
                    } else {
                        self.chunk_spawns
                            .push((chunk.chunk_id, std::time::Instant::now()));
                        (self.chunk_spawns.last().unwrap(), true)
                    };

                    if is_first || spawn.1.elapsed() > respawn_time {
                        // this is length biased
                        let spawn_category = self
                            .data
                            .enemies
                            .iter()
                            .map(|e| e.spawn_category)
                            .choose(&mut rand::thread_rng())
                            .unwrap_or_default();
                        let spawn_point = chunk
                            .enemy_spawn_points
                            .iter()
                            .choose(&mut rand::thread_rng());
                        let spawn_point = match spawn_point {
                            Some(x) => *x,
                            None => user.user.upgrade().unwrap().lock().await.position,
                        };
                        for _ in 0..count {
                            let enemy = self
                                .data
                                .enemies
                                .iter()
                                .filter(|e| e.spawn_category == spawn_category)
                                .choose(&mut rand::thread_rng());
                            if let Some(enemy) = enemy {
                                let enemy_name = enemy.enemy_name.clone();
                                self.spawn_enemy(
                                    block_data,
                                    max_id,
                                    enemy_lvl,
                                    &enemy_name,
                                    spawn_point,
                                )
                                .await?;
                            }
                        }
                    }
                }
                data_structs::map::EnemySpawnType::Manual => {
                    let proc = self.lua.lock().procs.get("spawn_enemy").cloned();
                    if let Some(lua) = proc {
                        self.run_lua(user.player_id, &packet, "spawn_enemy", &lua)
                            .await?;
                    };
                }
            }

            let mut packet = Packet::MinimapReveal(MinimapRevealPacket {
                world: ObjectHeader {
                    id: self.data.settings.world_id,
                    entity_type: ObjectType::World,
                    ..Default::default()
                },
                zone_id: self.data.settings.zone_id,
                revealed_zones: self.minimap_status.clone(),
                ..Default::default()
            });
            for player in &self.players {
                let Some(player) = player.user.upgrade() else {
                    continue;
                };
                let mut p_lock = player.lock().await;
                let party_lock = p_lock
                    .party
                    .as_ref()
                    .expect("Player should have a party at this point")
                    .read()
                    .await;
                let party_obj = party_lock.get_obj();
                drop(party_lock);
                if let Packet::MinimapReveal(p) = &mut packet {
                    p.party = party_obj;
                }
                let _ = p_lock.send_packet(&packet).await;
            }
        }

        let proc = self.lua.lock().procs.get("on_minimap_reveal").cloned();
        if let Some(lua) = proc {
            self.run_lua(user.player_id, &packet, "on_minimap_reveal", &lua)
                .await?;
        };

        Ok(())
    }

    async fn run_lua<S: serde::Serialize + Sync>(
        &mut self,
        sender_id: PlayerId,
        packet: &S,
        call_type: &str,
        lua_data: &str,
    ) -> Result<(), Error> {
        spawn_blocking(|| self.run_lua_blocking(sender_id, packet, call_type, lua_data)).await?
    }

    fn run_lua_blocking<S: serde::Serialize + Sync>(
        &mut self,
        sender_id: PlayerId,
        packet: &S,
        call_type: &str,
        lua_data: &str,
    ) -> Result<(), Error> {
        let mut scheduled_move = vec![];
        let mut lobby_moves = vec![];

        let Some(caller) = self
            .players
            .iter()
            .find(|p| p.player_id == sender_id)
            .and_then(|p| p.user.upgrade())
        else {
            unreachable!("Sender should exist in the current map");
        };
        let caller_lock = caller.lock_blocking();
        let zone_set = &self.data;
        drop(caller_lock);
        let mut lua_lock = self.lua.lock();
        {
            let lua = &lua_lock.lua;
            let globals = lua.globals();
            let player_ids: Vec<_> = self.players.iter().map(|p| p.player_id).collect();
            globals.set("zone", zone_set.name.clone())?;
            globals.set("packet", lua.to_value(&packet)?)?;
            globals.set("sender", sender_id)?;
            globals.set("players", player_ids)?;
            globals.set("call_type", call_type)?;
            lua.scope(|scope| {
                self.setup_scope(&globals, scope, &mut scheduled_move, &mut lobby_moves)?;

                /* LUA FUNCTIONS */

                // get account flag
                globals.set(
                    "get_account_flag",
                    scope.create_function_mut(|_, flag: u32| -> Result<u8, _> {
                        Ok(caller.lock_blocking().get_account_flags().get(flag as _))
                    })?,
                )?;
                // get character flag
                globals.set(
                    "get_character_flag",
                    scope.create_function_mut(|_, flag: u32| -> Result<u8, _> {
                        if let Some(f) = caller.lock_blocking().get_char_flags() {
                            Ok(f.get(flag as _))
                        } else {
                            unreachable!("Users in maps should have loaded characters")
                        }
                    })?,
                )?;

                /* LUA FUNCTIONS END */

                let chunk = lua.load(lua_data);
                chunk.exec()?;
                Ok(())
            })?;
            globals.raw_remove("packet")?;
            globals.raw_remove("sender")?;
            globals.raw_remove("players")?;
            globals.raw_remove("call_type")?;
            globals.raw_remove("zone")?;
        }
        for (receiver, mapid) in scheduled_move {
            lua_lock.to_move.push((receiver, mapid));
        }
        for receiver in lobby_moves {
            lua_lock.to_lobby_move.push(receiver);
        }
        Ok(())
    }

    fn setup_scope<'s>(
        &'s self,
        globals: &mlua::Table,
        scope: &'s mlua::Scope<'s, '_>,
        scheduled_move: &'s mut Vec<(PlayerId, String)>,
        lobby_moves: &'s mut Vec<PlayerId>,
    ) -> Result<(), mlua::Error> {
        /* LUA FUNCTIONS */

        // send packet
        let send =
            scope.create_function_mut(move |lua, (receiver, packet): (u32, mlua::Value)| {
                let packet: Packet = lua.from_value(packet)?;
                if let Some(p) = self
                    .players
                    .iter()
                    .find(|p| p.player_id == receiver)
                    .and_then(|p| p.user.upgrade())
                {
                    p.lock_blocking()
                        .send_packet_block(&packet)
                        .map_err(mlua::Error::external)?;
                }
                Ok(())
            })?;
        globals.set("send", send)?;
        // get object data
        let get_object = scope.create_function(move |lua, id: u32| {
            let object = self
                .objects
                .objects
                .iter()
                .find(|obj| obj.data.object.id == id)
                .ok_or(mlua::Error::runtime("Couldn't find requested object"))?;
            lua.to_value(&object.data)
        })?;
        globals.set("get_object", get_object)?;
        // get npc data
        let get_npc = scope.create_function(move |lua, id: u32| {
            let object = self
                .objects
                .npcs
                .iter()
                .find(|obj| obj.data.object.id == id)
                .ok_or(mlua::Error::runtime("Couldn't find requested npc"))?;
            lua.to_value(&object.data)
        })?;
        globals.set("get_npc", get_npc)?;
        // get additional data
        let get_extra_data = scope.create_function(move |lua, id: u32| {
            let object = self
                .objects
                .objects
                .iter()
                .map(|x| (x.data.object.id, &x.lua_data))
                .chain(
                    self.objects
                        .npcs
                        .iter()
                        .map(|x| (x.data.object.id, &x.lua_data)),
                )
                .find(|(obj_id, _)| *obj_id == id)
                .map(|(_, data)| data)
                .ok_or(mlua::Error::runtime("Couldn't find requested object"))?;
            match object {
                Some(d) => lua.load(d).eval::<mlua::Value>(),
                None => Ok(mlua::Value::Nil),
            }
        })?;
        globals.set("get_extra_data", get_extra_data)?;
        // move player to another submap
        globals.set(
            "move_player",
            scope.create_function_mut(|_, (receiver, zone): (u32, String)| {
                scheduled_move.push((receiver, zone));
                Ok(())
            })?,
        )?;
        // move player to lobby
        globals.set(
            "move_lobby",
            scope.create_function_mut(|_, receiver: u32| {
                lobby_moves.push(receiver);
                Ok(())
            })?,
        )?;
        // set account flag
        globals.set(
            "set_account_flag",
            scope.create_function_mut(move |_, (receiver, flag, value): (u32, u32, u8)| {
                if let Some(p) = self
                    .players
                    .iter()
                    .find(|p| p.player_id == receiver)
                    .and_then(|p| p.user.upgrade())
                {
                    p.lock_blocking()
                        .set_account_flag_block(flag, value != 0)
                        .map_err(mlua::Error::external)?;
                }
                Ok(())
            })?,
        )?;
        // set character flag
        globals.set(
            "set_character_flag",
            scope.create_function_mut(move |_, (receiver, flag, value): (u32, u32, u8)| {
                if let Some(p) = self
                    .players
                    .iter()
                    .find(|p| p.player_id == receiver)
                    .and_then(|p| p.user.upgrade())
                {
                    p.lock_blocking()
                        .set_char_flag_block(flag, value != 0)
                        .map_err(mlua::Error::external)?;
                }
                Ok(())
            })?,
        )?;
        // delete all npcs from the client
        globals.set(
            "delete_all_npcs_packets",
            scope.create_function_mut(move |lua, receiver: u32| -> Result<mlua::Value, _> {
                let mut packets = vec![];
                for object in self.objects.npcs.iter().filter(|n| n.is_active) {
                    packets.push(Packet::DespawnObject(
                        protocol::objects::DespawnObjectPacket {
                            player: ObjectHeader {
                                id: receiver,
                                entity_type: ObjectType::Player,
                                ..Default::default()
                            },
                            item: object.data.object,
                        },
                    ))
                }
                lua.to_value(&packets)
            })?,
        )?;
        // unlock one quest
        globals.set(
            "unlock_quest",
            scope.create_function_mut(
                move |_, (receiver, name_id): (u32, u32)| -> Result<(), _> {
                    if let Some(p) = self
                        .players
                        .iter()
                        .find(|p| p.player_id == receiver)
                        .and_then(|p| p.user.upgrade())
                    {
                        let mut lock = p.lock_blocking();
                        let char = lock
                            .character
                            .as_mut()
                            .expect("Character should be loaded for users in map");
                        if !char.unlocked_quests.contains(&name_id) {
                            char.unlocked_quests.push(name_id);
                            char.unlocked_quests_notif.push(name_id);
                        }
                    }
                    Ok(())
                },
            )?,
        )?;
        // [pso2-quest] mark one quest as cleared (story list clear mark)
        globals.set(
            "clear_quest",
            scope.create_function_mut(
                move |_, (receiver, name_id): (u32, u32)| -> Result<(), _> {
                    if let Some(p) = self
                        .players
                        .iter()
                        .find(|p| p.player_id == receiver)
                        .and_then(|p| p.user.upgrade())
                    {
                        let mut lock = p.lock_blocking();
                        let char = lock
                            .character
                            .as_mut()
                            .expect("Character should be loaded for users in map");
                        if !char.cleared_quests.contains(&name_id) {
                            char.cleared_quests.push(name_id);
                        }
                    }
                    Ok(())
                },
            )?,
        )?;
        // unlock multiple quests
        globals.set(
            "unlock_quests",
            scope.create_function_mut(
                move |_, (receiver, name_id): (u32, Vec<u32>)| -> Result<(), _> {
                    if let Some(p) = self
                        .players
                        .iter()
                        .find(|p| p.player_id == receiver)
                        .and_then(|p| p.user.upgrade())
                    {
                        let mut lock = p.lock_blocking();
                        let char = lock
                            .character
                            .as_mut()
                            .expect("Character should be loaded for users in map");
                        for name in name_id {
                            if !char.unlocked_quests.contains(&name) {
                                char.unlocked_quests.push(name);
                                char.unlocked_quests_notif.push(name);
                            }
                        }
                    }
                    Ok(())
                },
            )?,
        )?;

        /* LUA FUNCTIONS END */
        Ok(())
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        log::trace!("Map {} dropped", self.data.map_data.map_object.id);
    }
}

async fn exec_users<F>(users: &[MapPlayer], mut f: F)
where
    F: FnMut(OwnedMapPlayer, MutexGuard<User>) + Send,
{
    for user in users.iter().filter_map(|u| {
        u.user.upgrade().map(|p| OwnedMapPlayer {
            player_id: u.player_id,
            user: p,
        })
    }) {
        let arc = user.user.clone();
        let lock = arc.lock().await;
        f(user, lock)
    }
}

async fn spawn_blocking<F, R>(func: F) -> Result<R, Error>
where
    F: FnOnce() -> R + Send,
    R: Send + 'static,
{
    let val: Box<dyn FnOnce() -> R + Send> = Box::new(func);
    // SAFETY: this should be safe because we immediately await the function
    let func: Box<dyn FnOnce() -> R + Send + 'static> = unsafe { std::mem::transmute(val) };
    Ok(tokio::task::spawn_blocking(func).await?)
}

/// [pso2_vita_offline] one Monomate (Hunter starting item 3:1:0)
pub fn monomate() -> protocol::items::Item {
    use protocol::items::{ConsumableItem, Item, ItemId, ItemType};
    Item {
        uuid: 0,
        id: ItemId {
            item_type: 3,
            id: 1,
            subid: 0,
            ..Default::default()
        },
        data: ItemType::Consumable(ConsumableItem {
            amount: 1,
            ..Default::default()
        }),
    }
}

/// [pso2_vita_offline] A meseta drop: item type 0xFE (the client's item helpers treat 0xFE as an amount type, valid when
/// the u32 at item +0x10 is > 0), amount kept in the first 4 bytes of the item data and sent in NewItemDrop.unk1
/// (item +0x10 once the client copies the drop into an item).
pub const MESETA_TYPE: u16 = 0xFE;
pub fn meseta(amount: u32) -> protocol::items::Item {
    use protocol::items::{Item, ItemId, ItemType};
    Item {
        uuid: 0,
        id: ItemId {
            item_type: MESETA_TYPE,
            ..Default::default()
        },
        data: ItemType::Unknown(amount.to_le_bytes().to_vec().into()),
    }
}
/// [pso2_vita_offline] The amount of a meseta item from `meseta`, None for anything else.
pub fn meseta_amount(item: &protocol::items::Item) -> Option<u32> {
    match &item.data {
        protocol::items::ItemType::Unknown(b) if item.id.item_type == MESETA_TYPE && b.len() >= 4 => {
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        }
        _ => None,
    }
}

/// [pso2_vita_offline] Session value for 0F-07 (any non-zero; the client echoes it in ItemPickupRequest.unk).
const DROP_SESSION: u32 = 1;

/// [pso2_vita_offline] 0F-07 (not in pso2packetlib; reader `0x811ff272`, callback `0x82d3e710` -> `0x830130c2`):
/// object header 12 B, session u32 (copied into the client's drop registry if it is still 0; the registry ignores
/// NewItemDrop while it is 0), count `(n + 0x3E) ^ 0x32C1`, n x 0x38 B (NewItemDrop's body + drop_id). Sent with
/// n = 0 only to set the session.
pub fn drop_list(obj: ObjectHeader, session: u32) -> Packet {
    use pso2packetlib::protocol::{Flags, PacketHeader};
    let mut data = Vec::with_capacity(0x14);
    data.extend_from_slice(&obj.id.to_le_bytes());
    data.extend_from_slice(&obj.unk.to_le_bytes());
    data.extend_from_slice(&(obj.entity_type as u16).to_le_bytes());
    data.extend_from_slice(&obj.map_id.to_le_bytes());
    data.extend_from_slice(&session.to_le_bytes());
    data.extend_from_slice(&(0x3Eu32 ^ 0x32C1).to_le_bytes());
    Packet::Unknown((PacketHeader::new(0x0F, 0x07, Flags::PACKED), data))
}
