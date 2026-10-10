use std::sync::{Arc, atomic::AtomicU32};

use crate::{Error, map::Map, mutex::Mutex};
use data_structs::quest::QuestData;
use pso2packetlib::protocol::{
    party::{SetPartyQuestPacket, SetQuestInfoPacket},
    questlist::{
        AcceptQuestPacket, AcceptStoryQuestPacket, AvailableQuestType, AvailableQuestsPacket,
        QuestCategoryPacket, QuestDifficulty, QuestType,
    },
};

/// [pso2_vita_offline] Story battle data (`data/story_levels.json`, tools/gen_story_levels.py): enemy level bands of the
/// two difficulties (casual / hardcore) and the first-clear items (`type:id:subid`, 1 each).
#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct StoryLevels {
    c: [u32; 2],
    h: [u32; 2],
    items: Vec<String>,
}

fn story_levels() -> &'static std::collections::HashMap<u32, StoryLevels> {
    static T: std::sync::OnceLock<std::collections::HashMap<u32, StoryLevels>> = std::sync::OnceLock::new();
    T.get_or_init(|| serde_json::from_str(include_str!("../../data/story_levels.json")).unwrap_or_default())
}

/// [pso2_vita_offline] Enemy level of a story battle: clamp(player level, band of the difficulty) (the formula is
/// unknown in the research, estimated). `PSO2_STORY_LV=min|max` pins it to the band's end. Returns (level, first-clear items).
fn story_enemy_level(name_id: u32, diff: u32, player_level: u32) -> Option<(u32, Vec<String>)> {
    let t = story_levels().get(&name_id)?;
    let [lo, hi] = if diff == 0 { t.c } else { t.h };
    let lv = match std::env::var("PSO2_STORY_LV").as_deref() {
        Ok("min") => lo,
        Ok("max") => hi,
        _ => player_level.clamp(lo, hi),
    };
    Some((lv, t.items.clone()))
}

pub struct PartyQuest {
    quest: QuestData,
    diff: u16,
    map: Arc<Mutex<Map>>,
}

pub struct Quests {
    quests: Vec<QuestData>,
}


/// Quests listed at the counter for every character: every free field that has a definition
/// (name_id 20000..30000, quest_type Expedition). The fork's test quest 200030 is not listed.
fn always_unlocked(q: &QuestData) -> bool {
    (20000..30000).contains(&q.definition.name_id) && q.definition.quest_type == QuestType::Expedition
}
impl Quests {
    pub const fn load(quests: Vec<QuestData>) -> Self {
        Self { quests }
    }
    pub fn get_availiable(&self, unlocked: &[u32]) -> AvailableQuestsPacket {
        let mut available = AvailableQuestsPacket::default();
        for quest in self
            .quests
            .iter()
            .filter(|q| unlocked.contains(&q.definition.name_id) || always_unlocked(q))
        {
            match quest.definition.quest_type {
                QuestType::Unk0 => {
                    available.unk1 += 1;
                    available.available_types |= AvailableQuestType::UNK1;
                }
                QuestType::Extreme => {
                    available.extreme_count += 1;
                    available.available_types |= AvailableQuestType::EXTREME;
                }
                QuestType::ARKS => {
                    available.arks_count += 1;
                    available.available_types |= AvailableQuestType::ARKS;
                }
                QuestType::LimitedTime => {
                    available.limited_time_count += 1;
                    available.available_types |= AvailableQuestType::LIMITED_TIME;
                }
                QuestType::ExtremeDebug => {
                    available.extreme_debug_count += 1;
                    available.available_types |= AvailableQuestType::EXTREME_DEBUG;
                }
                QuestType::Blank1 => {
                    available.blank1_count += 1;
                    available.available_types |= AvailableQuestType::BLANK1;
                }
                QuestType::NetCafe => {
                    available.net_cafe_count += 1;
                    available.available_types |= AvailableQuestType::NET_CAFE;
                }
                QuestType::WarmingDebug => {
                    available.warming_debug_count += 1;
                    available.available_types |= AvailableQuestType::WARMING_DEBUG;
                }
                QuestType::Blank2 => {
                    available.blank2_count += 1;
                    available.available_types |= AvailableQuestType::BLANK2;
                }
                QuestType::Advance => {
                    available.advance_count += 1;
                    available.available_types |= AvailableQuestType::ADVANCE;
                }
                QuestType::Expedition => {
                    available.expedition_count += 1;
                    available.available_types |= AvailableQuestType::EXPEDITION;
                }
                QuestType::FreeDebug => {
                    available.expedition_debug_count += 1;
                    available.available_types |= AvailableQuestType::FREE_DEBUG;
                }
                QuestType::ArksDebug => {
                    available.arks_debug_count += 1;
                    available.available_types |= AvailableQuestType::ARKS_DEBUG;
                }
                QuestType::Challenge => {
                    available.challenge_count += 1;
                    available.available_types |= AvailableQuestType::CHALLENGE;
                }
                QuestType::Urgent => {
                    available.urgent_count += 1;
                    available.available_types |= AvailableQuestType::URGENT;
                }
                QuestType::UrgentDebug => {
                    available.urgent_debug_count += 1;
                    available.available_types |= AvailableQuestType::URGENT_DEBUG;
                }
                QuestType::TimeAttack => {
                    available.time_attack_count += 1;
                    available.available_types |= AvailableQuestType::TIME_ATTACK;
                }
                QuestType::TimeDebug => {
                    available.time_attack_debug_count += 1;
                    available.available_types |= AvailableQuestType::TIME_DEBUG;
                }
                QuestType::ArksDebug2 => {
                    available.arks_debug2_count[0] += 1;
                    available.available_types |= AvailableQuestType::ARKS_DEBUG2;
                }
                QuestType::ArksDebug3 => {
                    available.arks_debug2_count[1] += 1;
                    available.available_types |= AvailableQuestType::ARKS_DEBUG3;
                }
                QuestType::ArksDebug4 => {
                    available.arks_debug2_count[2] += 1;
                    available.available_types |= AvailableQuestType::ARKS_DEBUG4;
                }
                QuestType::ArksDebug5 => {
                    available.arks_debug2_count[3] += 1;
                    available.available_types |= AvailableQuestType::ARKS_DEBUG5;
                }
                QuestType::ArksDebug6 => {
                    available.arks_debug2_count[4] += 1;
                    available.available_types |= AvailableQuestType::ARKS_DEBUG6;
                }
                QuestType::ArksDebug7 => {
                    available.arks_debug2_count[5] += 1;
                    available.available_types |= AvailableQuestType::ARKS_DEBUG7;
                }
                QuestType::ArksDebug8 => {
                    available.arks_debug2_count[6] += 1;
                    available.available_types |= AvailableQuestType::ARKS_DEBUG8;
                }
                QuestType::ArksDebug9 => {
                    available.arks_debug2_count[7] += 1;
                    available.available_types |= AvailableQuestType::ARKS_DEBUG9;
                }
                QuestType::ArksDebug10 => {
                    available.arks_debug2_count[8] += 1;
                    available.available_types |= AvailableQuestType::ARKS_DEBUG10;
                }
                QuestType::Blank3 => {
                    available.blank3_count += 1;
                    available.available_types |= AvailableQuestType::BLANK3;
                }
                QuestType::Recommended => {
                    available.recommended_count += 1;
                    available.available_types |= AvailableQuestType::RECOMMENDED;
                }
                QuestType::Ultimate => {
                    available.unk6 += 1;
                    available.available_types |= AvailableQuestType::ULTIMATE;
                }
                QuestType::UltimateDebug => {
                    available.ultimate_debug_count += 1;
                    available.available_types |= AvailableQuestType::ULTIMATE_DEBUG;
                }
                QuestType::AGP => {
                    available.agp_count += 1;
                    available.available_types |= AvailableQuestType::AGP;
                }
                QuestType::Bonus => {
                    available.bonus_count += 1;
                    available.available_types |= AvailableQuestType::BONUS;
                }
                QuestType::StandardTraining => {
                    available.training_count[0] += 1;
                    available.available_types |= AvailableQuestType::STANDARD_TRAINING;
                }
                QuestType::HunterTraining => {
                    available.training_count[1] += 1;
                    available.available_types |= AvailableQuestType::HUNTER_TRAINING;
                }
                QuestType::RangerTraining => {
                    available.training_count[2] += 1;
                    available.available_types |= AvailableQuestType::RANGER_TRAINING;
                }
                QuestType::ForceTraining => {
                    available.training_count[3] += 1;
                    available.available_types |= AvailableQuestType::FORCE_TRAINING;
                }
                QuestType::FighterTraining => {
                    available.training_count[4] += 1;
                    available.available_types |= AvailableQuestType::FIGHTER_TRAINING;
                }
                QuestType::GunnerTraining => {
                    available.training_count[5] += 1;
                    available.available_types |= AvailableQuestType::GUNNER_TRAINING;
                }
                QuestType::TechterTraining => {
                    available.training_count[6] += 1;
                    available.available_types |= AvailableQuestType::TECHTER_TRAINING;
                }
                QuestType::BraverTraining => {
                    available.training_count[7] += 1;
                    available.available_types |= AvailableQuestType::BRAVER_TRAINING;
                }
                QuestType::BouncerTraining => {
                    available.training_count[8] += 1;
                    available.available_types |= AvailableQuestType::BOUNCER_TRAINING;
                }
                QuestType::SummonerTraining => {
                    available.training_count[9] += 1;
                    available.available_types |= AvailableQuestType::SUMMONER_TRAINING;
                }
                QuestType::AutoAccept => {
                    available.available_types |= AvailableQuestType::AUTO_ACCEPT;
                }
                QuestType::Ridroid => {
                    available.ridroid_count += 1;
                    available.available_types |= AvailableQuestType::RIDROID;
                }
                QuestType::CafeAGP => {
                    available.net_cafe_agp_count += 1;
                    available.available_types |= AvailableQuestType::NET_CAFE_AGP;
                }
                QuestType::BattleBroken => {
                    available.battle_broken_count += 1;
                    available.available_types |= AvailableQuestType::BATTLE_BROKEN;
                }
                QuestType::BusterDebug => {
                    available.buster_debug_count += 1;
                    available.available_types |= AvailableQuestType::BUSTER_DEBUG;
                }
                QuestType::Poka12 => {
                    available.poka12_count += 1;
                    available.available_types |= AvailableQuestType::POKA12;
                }
                QuestType::StoryEP1 => {
                    available.unk9 += 1;
                    available.available_types |= AvailableQuestType::STORY_EP1;
                }
                QuestType::Buster => {
                    available.buster_count += 1;
                    available.available_types |= AvailableQuestType::BUSTER;
                }
                QuestType::HeroTraining => {
                    available.hero_training_count += 1;
                    available.available_types |= AvailableQuestType::HERO_TRAINING;
                }
                QuestType::Amplified => {
                    available.amplified_count += 1;
                    available.available_types |= AvailableQuestType::AMPLIFIED;
                }
                QuestType::DarkBlastTraining => {
                    available.dark_blast_training_count += 1;
                    available.available_types |= AvailableQuestType::DARK_BLAST_TRAINING;
                }
                QuestType::Endless => {
                    available.endless_count += 1;
                    available.available_types |= AvailableQuestType::ENDLESS;
                }
                QuestType::Blank4 => {
                    available.unk13 += 1;
                    available.available_types |= AvailableQuestType::BLANK4;
                }
                QuestType::PhantomTraining => {
                    available.phantom_training_count += 1;
                    available.available_types |= AvailableQuestType::PHANTOM_TRAINING;
                }
                QuestType::AISTraining => {
                    available.ais_training_count += 1;
                    available.available_types |= AvailableQuestType::AIS_TRAINING;
                }
                QuestType::DamageCalculation => {
                    available.damage_calc_count += 1;
                    available.available_types |= AvailableQuestType::DAMAGE_CALC;
                }
                QuestType::EtoileTraining => {
                    available.etoile_training_count += 1;
                    available.available_types |= AvailableQuestType::ETOILE_TRAINING;
                }
                QuestType::Divide => {
                    available.divide_count += 1;
                    available.available_types |= AvailableQuestType::DIVIDE;
                }
                QuestType::Stars1 => {
                    available.stars1_count += 1;
                    available.available_types |= AvailableQuestType::STARS1;
                }
                QuestType::Stars2 => {
                    available.stars2_count += 1;
                    available.available_types |= AvailableQuestType::STARS2;
                }
                QuestType::Stars3 => {
                    available.stars3_count += 1;
                    available.available_types |= AvailableQuestType::STARS3;
                }
                QuestType::Stars4 => {
                    available.unk15[0] += 1;
                    available.available_types |= AvailableQuestType::STARS4;
                }
                QuestType::Stars5 => {
                    available.unk15[1] += 1;
                    available.available_types |= AvailableQuestType::STARS5;
                }
                QuestType::Stars6 => {
                    available.unk16[0] += 1;
                    available.available_types |= AvailableQuestType::STARS6;
                }
            }
        }
        available.unk19 = available.available_types.clone();
        available
    }
    //FIXME: this will not work for limited time quests
    pub fn get_category(
        &self,
        category: QuestType,
        unlocked: &[u32],
        cleared: &[u32],
    ) -> QuestCategoryPacket {
        QuestCategoryPacket {
            quests: self
                .quests
                .iter()
                .filter(|q| unlocked.contains(&q.definition.name_id) || always_unlocked(q))
                .filter(|q| q.definition.quest_type == category)
                .map(|q| {
                    let mut def = q.definition.clone();
                    // [pso2-quest] Vita: unk8[2] (+0xF4) bit 0 = "new" icon (not cleared), bit 1 = story in progress
                    if cleared.contains(&def.name_id) {
                        def.unk8[2] &= !3;
                    }
                    def
                })
                .collect(),
        }
    }
    pub fn get_diff(&self, id: u32) -> Option<QuestDifficulty> {
        self.quests
            .iter()
            .find(|q| q.definition.quest_obj.id == id)
            // [pso2-quest] the client files the reply under its quest_obj and re-requests until the
            // requested one arrives (the test quest's data has 1101 here, its definition 1100)
            .map(|q| QuestDifficulty {
                quest_obj: q.definition.quest_obj,
                ..q.difficulties.clone()
            })
    }
    pub fn get_quest(
        &self,
        packet: AcceptQuestPacket,
        map_obj_id: &AtomicU32,
    ) -> Result<PartyQuest, Error> {
        let Some(quest) = self
            .quests
            .iter()
            .find(|q| q.definition.quest_obj.id == packet.quest_obj.id)
        else {
            return Err(Error::InvalidInput("get_quest"));
        };
        if packet.diff >= 8 {
            return Err(Error::InvalidInput("get_quest"));
        }
        let mut map = Map::new_from_data(quest.map.clone(), map_obj_id)?;
        map.set_enemy_level(quest.difficulties.diffs[packet.diff as usize].monster_level as _);
        map.set_quest(quest.definition.name_id, packet.diff as u32);
        let map = Arc::new(Mutex::new(map));
        crate::pse::spawn_ticker(Arc::downgrade(&map));
        Ok(PartyQuest {
            quest: quest.clone(),
            diff: packet.diff,
            map,
        })
    }
    pub fn get_story_quest(
        &self,
        packet: AcceptStoryQuestPacket,
        map_obj_id: &AtomicU32,
        player_level: u32,
    ) -> Result<PartyQuest, Error> {
        let Some(quest) = self
            .quests
            .iter()
            .find(|q| q.definition.name_id == packet.name_id)
        else {
            return Err(Error::InvalidInput("get_quest"));
        };
        let mut map = Map::new_from_data(quest.map.clone(), map_obj_id)?;
        // [pso2_vita_offline] `unk` is taken as the difficulty (0 casual, 1 hardcore; unverified)
        // (`PSO2_STORY_DIFF=0|1` pins it: the client's casual flag is not found yet and it sends 1 for every battle)
        let diff = std::env::var("PSO2_STORY_DIFF")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(packet.unk)
            .min(1);
        match story_enemy_level(packet.name_id, diff, player_level) {
            Some((lv, items)) => {
                log::info!("[pso2-quest] story {} diff {diff}: player Lv{player_level} -> enemy Lv{lv}", packet.name_id);
                map.set_enemy_level(lv);
                map.set_story(packet.name_id, diff, items);
            }
            None => map.set_enemy_level(quest.difficulties.diffs[0].monster_level as _),
        }
        map.set_quest_obj(quest.definition.quest_obj);
        map.set_quest(packet.name_id, diff);
        let map = Arc::new(Mutex::new(map));
        crate::pse::spawn_ticker(Arc::downgrade(&map));
        Ok(PartyQuest {
            quest: quest.clone(),
            diff: diff as u16,
            map,
        })
    }
    pub fn get_quest_by_nameid(&self, id: u32) -> Option<&QuestData> {
        self.quests.iter().find(|q| q.definition.name_id == id)
    }
}

impl PartyQuest {
    pub fn set_party_packet(&self) -> SetPartyQuestPacket {
        SetPartyQuestPacket {
            name: self.quest.definition.name_id,
            difficulty: self.diff as u32,
            quest_type: self.quest.definition.quest_type,
            quest_def: self.quest.definition.clone(),
            quest_diffs: self.quest.difficulties.clone(),
            ..Default::default()
        }
    }
    pub fn set_info_packet(&self) -> SetQuestInfoPacket {
        SetQuestInfoPacket {
            name: self.quest.definition.name_id,
            diff: self.diff as u8,
            quest_type: self.quest.definition.quest_type,
            unk3: 40,
            unk6: 64,
            ..Default::default()
        }
    }
    pub fn get_map(&self) -> Arc<Mutex<Map>> {
        self.map.clone()
    }
    pub const fn is_insta_transfer(&self) -> bool {
        self.quest.immediate_move
    }
}
