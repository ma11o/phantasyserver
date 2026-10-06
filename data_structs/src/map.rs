use pso2packetlib::protocol::{
    models::Position,
    server::{LoadLevelPacket, ZoneSettings},
    spawn::{EventSpawnPacket, NPCSpawnPacket, ObjectSpawnPacket, TransporterSpawnPacket},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};

pub type ZoneId = u32;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct MapData {
    pub map_data: LoadLevelPacket,
    pub objects: Vec<ObjectData>,
    pub events: Vec<EventData>,
    pub npcs: Vec<NPCData>,
    pub transporters: Vec<TransporterData>,
    pub luas: HashMap<String, String>,
    pub init_map: ZoneId,
    pub zones: Vec<ZoneData>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ObjectData {
    pub zone_id: ZoneId,
    pub is_active: bool,
    pub data: ObjectSpawnPacket,
    pub lua_data: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct EventData {
    pub zone_id: ZoneId,
    pub is_active: bool,
    pub data: EventSpawnPacket,
    pub lua_data: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct NPCData {
    pub zone_id: ZoneId,
    pub is_active: bool,
    pub data: NPCSpawnPacket,
    pub lua_data: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct TransporterData {
    pub zone_id: ZoneId,
    pub is_active: bool,
    pub data: TransporterSpawnPacket,
    pub lua_data: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct EnemySpawn {
    pub enemy_name: String,
    pub spawn_category: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ZoneData {
    pub name: String,
    pub is_special_zone: bool,
    pub zone_id: ZoneId,
    pub settings: ZoneSettings,
    pub default_location: Position,
    pub enemies: Vec<EnemySpawn>,
    pub chunks: Vec<ZoneChunk>,
    /// [pso2_vita_offline] see `ZoneBoss`
    pub boss: Option<ZoneBoss>,
}

/// [pso2_vita_offline] The zone's boss: spawned once at `position` when the chunk `chunk_id` is revealed (`u32::MAX`:
/// the first chunk the client reports in the zone). In a zone
/// with a boss the quest clears when the boss dies (not when every spawned enemy is dead).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ZoneBoss {
    pub enemy_name: String,
    pub chunk_id: u32,
    pub position: Position,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub enum EnemySpawnType {
    #[default]
    Disabled,
    Automatic {
        min: u32,
        max: u32,
    },
    AutomaticWithRespawn {
        min: u32,
        max: u32,
        respawn_time: Duration,
    },
    Manual,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ChunkReveal {
    pub row: u32,
    pub column: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ZoneChunk {
    pub zone_id: ZoneId,
    pub chunk_id: u32,
    pub enemy_spawn_type: EnemySpawnType,
    pub enemy_spawn_points: Vec<Position>,
    pub reveals: Vec<ChunkReveal>,
}
