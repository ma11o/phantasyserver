use crate::{Error, User};
use data_structs::{ServerData, stats::{AttackStats, EnemyHitbox}};
use pso2packetlib::protocol::{
    models::{Position, character::Class},
    objects::{DamageReceivePacket, EnemyKilledPacket},
    playerstatus::DealDamagePacket,
    spawn::EnemySpawnPacket,
};
use rand::distributions::Distribution;

#[derive(Debug, Clone, Default)]
pub struct PlayerStats {
    max_hp: u32,
    hp: u32,
    dex: u32,

    base_mel_pwr: u32,
    weapon_mel_pwr: u32,
    base_rng_pwr: u32,
    weapon_rng_pwr: u32,
    base_tec_pwr: u32,
    weapon_tec_pwr: u32,

    base_mel_def: u32,
    base_rng_def: u32,
    base_tec_def: u32,
}

#[derive(Debug, Clone, Default)]
pub struct EnemyStats {
    name: String,
    level: u32,
    exp: u32,
    pos: Position,

    max_hp: u32,
    hp: u32,
    dex: u32,

    max_mel_pwr: u32,
    min_mel_pwr: u32,
    max_rng_pwr: u32,
    min_rng_pwr: u32,
    max_tec_pwr: u32,
    min_tec_pwr: u32,

    mel_def: u32,
    rng_def: u32,
    tec_def: u32,

    hitboxes: Vec<EnemyHitbox>,
}

pub enum BattleResult {
    Damaged {
        dmg_packet: DamageReceivePacket,
    },
    Killed {
        dmg_packet: DamageReceivePacket,
        kill_packet: EnemyKilledPacket,
        exp_amount: u32,
    },
}

/// [pso2_vita_offline] Weapon attack at grind +N as a percent of the base (old-type weapons; the result is floored).
/// Source: swiki アイテム強化 (pso2_research `data/swiki/enhancement/stat-multipliers.jsonl.gz`, PC, captured
/// 2026-10-05), not checked against the Vita client. Above +10 the +10 value is kept.
fn grind_percent(rarity: u8, grind: u8) -> u32 {
    const TABLE: [(u8, [u32; 10]); 7] = [
        (3, [104, 108, 112, 117, 122, 127, 132, 138, 144, 150]),
        (6, [104, 108, 112, 117, 122, 129, 136, 144, 152, 160]),
        (9, [104, 109, 115, 122, 129, 137, 145, 154, 164, 175]),
        (10, [104, 109, 115, 122, 130, 140, 150, 162, 175, 190]),
        (11, [105, 111, 118, 126, 135, 145, 156, 168, 181, 195]),
        (12, [106, 113, 121, 130, 140, 150, 161, 173, 186, 200]),
        (u8::MAX, [104, 108, 112, 116, 120, 124, 128, 132, 136, 140]),
    ];
    if grind == 0 {
        return 100;
    }
    let row = &TABLE.iter().find(|(max, _)| rarity <= *max).unwrap_or(&TABLE[6]).1;
    row[grind.min(10) as usize - 1]
}

impl PlayerStats {
    pub fn build(user: &User) -> Result<Self, Error> {
        let Some(char) = &user.character else {
            unreachable!("User should be in state >= `PreInGame`")
        };
        let server_data = &user.get_blockdata().server_data;

        let char_data = &char.character;
        let class = char_data.classes.main_class as usize;
        let level = char_data.get_level().level1 as usize;
        let mut resulting_stats = Self::calculate_class_stats(user, class, level);

        if char_data.classes.sub_class != Class::Unknown {
            // source: arks-visiphone
            let class = char_data.classes.sub_class as usize;
            let level = (char_data.get_sublevel().level1 as usize).min(level);
            let subclass_stats = Self::calculate_class_stats(user, class, level);
            resulting_stats.hp += subclass_stats.hp / 4;
            resulting_stats.max_hp = resulting_stats.hp;
            resulting_stats.dex += subclass_stats.dex / 4;
            resulting_stats.base_mel_pwr += subclass_stats.base_mel_pwr / 4;
            resulting_stats.base_rng_pwr += subclass_stats.base_rng_pwr / 4;
            resulting_stats.base_tec_pwr += subclass_stats.base_tec_pwr / 4;
            resulting_stats.base_mel_def += subclass_stats.base_mel_def / 4;
            resulting_stats.base_rng_def += subclass_stats.base_rng_def / 4;
            resulting_stats.base_tec_def += subclass_stats.base_tec_def / 4;
        }

        if let Some(equiped_item) = char.palette.get_current_item(&char.inventory)? {
            let ids = equiped_item.id;
            let weapon_stats = server_data
                .item_params
                .attrs
                .weapons
                .iter()
                .find(|a| a.id == ids.id && a.subid == ids.subid)
                .cloned()
                .ok_or(Error::NoItemInAttrs(ids.id, ids.subid))?;
            log::debug!(
                "[pso2-battle] weapon {}:{}:{} mel {} rng {} tec {} (base mel {})",
                ids.item_type,
                ids.id,
                ids.subid,
                weapon_stats.melee_dmg,
                weapon_stats.range_dmg,
                weapon_stats.gender_force_dmg.force_dmg,
                resulting_stats.base_mel_pwr
            );
            let grind = match &equiped_item.data {
                pso2packetlib::protocol::items::ItemType::Weapon(w) => w.grind,
                _ => 0,
            };
            let pct = grind_percent(weapon_stats.rarity, grind);
            let ground = |v: u32| v * pct / 100;
            log::debug!("[pso2-battle] weapon grind +{grind} (rarity {}) -> {pct}%", weapon_stats.rarity);
            resulting_stats.weapon_mel_pwr = ground(weapon_stats.melee_dmg as _);
            resulting_stats.weapon_rng_pwr = ground(weapon_stats.range_dmg as _);
            resulting_stats.weapon_tec_pwr = ground(weapon_stats.gender_force_dmg.force_dmg as _);
        }
        Ok(resulting_stats)
    }
    fn calculate_class_stats(user: &User, class: usize, level: usize) -> Self {
        let Some(char) = &user.character else {
            unreachable!("User should be in state >= `PreInGame`")
        };
        let mut resulting_stats = Self::default();
        let player_stats = &user.get_blockdata().server_data.player_stats;

        let stats = &player_stats.stats[class][level - 1];

        let char_data = &char.character;
        let modifier_offset = char_data.look.race as usize * 2 + char_data.look.gender as usize;
        let modifiers = &player_stats.modifiers[modifier_offset];

        resulting_stats.hp = (stats.hp + (stats.hp * 0.01 * modifiers.hp as f32).floor()) as _;
        resulting_stats.max_hp = resulting_stats.hp;
        resulting_stats.dex = (stats.dex + (stats.dex * 0.01 * modifiers.dex as f32).floor()) as _;
        resulting_stats.base_mel_pwr =
            (stats.mel_pow + (stats.mel_pow * 0.01 * modifiers.mel_pow as f32).floor()) as _;
        resulting_stats.base_rng_pwr =
            (stats.rng_pow + (stats.rng_pow * 0.01 * modifiers.rng_pow as f32).floor()) as _;
        resulting_stats.base_tec_pwr =
            (stats.tec_pow + (stats.tec_pow * 0.01 * modifiers.tec_pow as f32).floor()) as _;
        resulting_stats.base_mel_def =
            (stats.mel_def + (stats.mel_def * 0.01 * modifiers.mel_def as f32).floor()) as _;
        resulting_stats.base_rng_def =
            (stats.rng_def + (stats.rng_def * 0.01 * modifiers.rng_def as f32).floor()) as _;
        resulting_stats.base_tec_def =
            (stats.tec_def + (stats.tec_def * 0.01 * modifiers.tec_def as f32).floor()) as _;

        resulting_stats
    }
    pub fn update(player: &mut User) -> Result<(), Error> {
        let old_hp = player.get_stats().hp;
        let mut new_stats = Self::build(player)?;
        new_stats.hp = old_hp;
        *player.get_stats_mut() = new_stats;

        Ok(())
    }
    /// Weapon attack (melee, ranged, technique) after the grind.
    pub const fn weapon_pwr(&self) -> (u32, u32, u32) {
        (self.weapon_mel_pwr, self.weapon_rng_pwr, self.weapon_tec_pwr)
    }
    pub const fn get_hp(&self) -> (u32, u32) {
        (self.hp, self.max_hp)
    }
    /// Full HP (the campship heals; the client restores its own HP there).
    pub fn restore_hp(&mut self) {
        self.hp = self.max_hp;
    }
    pub fn set_hp(&mut self, hp: u32) {
        self.hp = hp.min(self.max_hp);
    }
    pub fn damage_enemy(
        &mut self,
        enemy: &mut EnemyStats,
        srv_data: &ServerData,
        attack: DealDamagePacket,
    ) -> Result<BattleResult, Error> {
        let damage = find_attack(srv_data, attack.attack_id, "player -> enemy", &enemy.name);
        // [pso2_vita_offline] unknown parts fall back to the first hitbox with a warning (the part ids of most
        // enemies are not known yet)
        let Some(hitbox) = enemy
            .hitboxes
            .iter()
            .find(|h| h.hitbox_id == attack.hitbox_id)
            .or_else(|| {
                log::warn!(
                    "[pso2-battle] unknown hitbox {} of {}, using the first one",
                    attack.hitbox_id,
                    enemy.name
                );
                enemy.hitboxes.first()
            })
            .cloned()
        else {
            return Err(Error::NoHitboxInfo(
                enemy.name.to_string(),
                attack.hitbox_id,
            ));
        };
        let (base_pwr, weapon_pwr, part_mul) = match damage.attack_type {
            data_structs::stats::AttackType::Mel => {
                (self.base_mel_pwr, self.weapon_mel_pwr, hitbox.mel_mul)
            }
            data_structs::stats::AttackType::Rng => {
                (self.base_rng_pwr, self.weapon_rng_pwr, hitbox.rng_mul)
            }
            data_structs::stats::AttackType::Tec => {
                (self.base_tec_pwr, self.weapon_tec_pwr, hitbox.tec_mul)
            }
        };
        let def = match damage.defense_type {
            data_structs::stats::AttackType::Mel => enemy.mel_def,
            data_structs::stats::AttackType::Rng => enemy.rng_def,
            data_structs::stats::AttackType::Tec => enemy.tec_def,
        };
        let total_mul = 1.0 * hitbox.damage_mul;
        let min_pure_attack =
            (base_pwr as f32 + weapon_pwr as f32 * 0.9 - def as f32).clamp(1.0, f32::MAX);
        let pure_attack = (base_pwr + weapon_pwr)
            .saturating_sub(def)
            .clamp(1, u32::MAX) as f32;
        let damage_mul = match damage.damage {
            data_structs::stats::DamageType::Generic(m) => m,
            data_structs::stats::DamageType::PA(_) => todo!(),
        };
        let min_weapon_attack = min_pure_attack / 5.0 * 1.05 * part_mul * damage_mul * total_mul;
        let max_weapon_attack = pure_attack / 5.0 * 1.05 * part_mul * damage_mul * total_mul;

        //TODO: elemental dmg

        let mut rng = rand::rngs::OsRng;
        let crit_chance = rand::distributions::Uniform::new(0, 100).sample(&mut rng);
        let dmg = if crit_chance < 5 || min_weapon_attack == max_weapon_attack {
            max_weapon_attack
        } else {
            rand::distributions::Uniform::new(min_weapon_attack, max_weapon_attack).sample(&mut rng)
        }
        .round() as u32;
        enemy.hp = enemy.hp.saturating_sub(dmg);
        log::debug!("[pso2-battle] {} took {dmg}, hp {}", enemy.name, enemy.hp);
        let dmg_packet = DamageReceivePacket {
            dmg_target: attack.target,
            dmg_inflicter: attack.inflicter,
            damage_id: damage.damage_id,
            dmg_amount: dmg as _,
            new_hp: enemy.hp,
            hitbox_id: attack.hitbox_id,
            x_pos: attack.x_pos,
            y_pos: attack.y_pos,
            z_pos: attack.z_pos,
            ..Default::default()
        };
        Ok(if enemy.hp == 0 {
            let kill_packet = EnemyKilledPacket {
                receiver: dmg_packet.receiver,
                dmg_target: dmg_packet.dmg_target,
                dmg_inflicter: dmg_packet.dmg_inflicter,
                damage_id: dmg_packet.damage_id,
                dmg_amount: dmg_packet.dmg_amount,
                new_hp: dmg_packet.new_hp,
                hitbox_id: dmg_packet.hitbox_id,
                x_pos: dmg_packet.x_pos,
                y_pos: dmg_packet.y_pos,
                z_pos: dmg_packet.z_pos,
                unk1: dmg_packet.unk1,
                unk2: dmg_packet.unk3,
                unk3: dmg_packet.unk4,
                unk4: dmg_packet.unk4,
                unk5: dmg_packet.unk5,
                unk6: dmg_packet.unk6,
                unk7: dmg_packet.unk7,
            };
            BattleResult::Killed {
                dmg_packet,
                kill_packet,
                exp_amount: enemy.exp,
            }
        } else {
            BattleResult::Damaged { dmg_packet }
        })
    }
}

impl EnemyStats {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn build(name: &str, level: u32, pos: Position, data: &ServerData) -> Result<Self, Error> {
        let mut resulting_stats = Self {
            name: name.to_string(),
            pos,
            ..Default::default()
        };
        let base_stats = &data.enemy_stats.base;
        let enemy_stats = &data
            .enemy_stats
            .enemies
            .get(name)
            .ok_or(Error::NoEnemyData(name.to_string()))?;
        resulting_stats.hitboxes.clone_from(&enemy_stats.hitboxes);
        let base_level_stats = &base_stats.levels[level as usize - 1];
        let level_stats = &enemy_stats.levels[level as usize - 1];

        resulting_stats.level = level_stats.level;
        resulting_stats.exp = (base_level_stats.exp * level_stats.exp).floor() as _;
        resulting_stats.max_hp = (base_level_stats.hp * level_stats.hp).floor() as _;
        resulting_stats.hp = resulting_stats.max_hp;
        resulting_stats.dex = (base_level_stats.dex * level_stats.dex).floor() as _;
        resulting_stats.max_mel_pwr =
            (base_level_stats.max_mel_dmg * level_stats.max_mel_dmg).floor() as _;
        resulting_stats.min_mel_pwr =
            (base_level_stats.min_mel_dmg * level_stats.min_mel_dmg).floor() as _;
        resulting_stats.max_rng_pwr =
            (base_level_stats.max_rng_dmg * level_stats.max_rng_dmg).floor() as _;
        resulting_stats.min_rng_pwr =
            (base_level_stats.min_rng_dmg * level_stats.min_rng_dmg).floor() as _;
        resulting_stats.max_tec_pwr =
            (base_level_stats.max_tec_dmg * level_stats.max_tec_dmg).floor() as _;
        resulting_stats.min_tec_pwr =
            (base_level_stats.min_tec_dmg * level_stats.min_tec_dmg).floor() as _;
        resulting_stats.mel_def = (base_level_stats.mel_def * level_stats.mel_def).floor() as _;
        resulting_stats.rng_def = (base_level_stats.rng_def * level_stats.rng_def).floor() as _;
        resulting_stats.tec_def = (base_level_stats.tec_def * level_stats.tec_def).floor() as _;

        log::info!(
            "[pso2-battle] enemy {name} Lv{level} hp {} exp {} def {}/{}/{} atk max {}/{}/{}",
            resulting_stats.max_hp,
            resulting_stats.exp,
            resulting_stats.mel_def,
            resulting_stats.rng_def,
            resulting_stats.tec_def,
            resulting_stats.max_mel_pwr,
            resulting_stats.max_rng_pwr,
            resulting_stats.max_tec_pwr
        );
        Ok(resulting_stats)
    }
    /// [pso2_vita_offline] debug `ehp`
    pub fn set_hp(&mut self, hp: u32) {
        self.hp = hp.min(self.max_hp);
    }
    /// [pso2_vita_offline] where the enemy was spawned (the server does not track enemy movement)
    pub const fn position(&self) -> Position {
        self.pos
    }
    pub fn create_spawn_packet(&self, id: u32) -> EnemySpawnPacket {
        EnemySpawnPacket {
            object: pso2packetlib::protocol::ObjectHeader {
                id,
                entity_type: pso2packetlib::protocol::ObjectType::Object,
                ..Default::default()
            },
            position: self.pos,
            name: self.name.to_string().into(),
            hp: self.hp,
            level: self.level,
            unk2: 2,
            unk4: 2,
            // unk5: 269877691,

            // contains emergency marker color
            // 0x505 - blue
            // 0x605 - purple
            // 0x705 - green
            // unk6: 0x405,
            // 0x1 - ??
            // 0x2 - ??
            // 0x4 - ??
            // 0x8 - ??
            // 0x10 - ??
            // 0x20 - ??
            // 0x40 - big enemy marker
            // 0x80 - hide enemy from minimap
            // 0x300 - emergency color
            // 0x400 - ??
            unk6: 0x405,
            unk7: 255,
            unk8: 65535,
            // [9] RandomScale (f32), [10] RaidHpRatio (f32), [12] enemy status key: the client looks up its
            // enemy_status.ens record (model and display name) by this key, see `actor_key`
            unk9: [
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                1062804813,
                3212836864,
                0,
                actor_key(&self.name),
                0,
                0,
                0,
            ],
            unk11: 1,
            unk12: 255,
            ..Default::default()
        }
    }
    pub fn damage_player(
        &mut self,
        player: &mut PlayerStats,
        srv_data: &ServerData,
        attack: DealDamagePacket,
    ) -> Result<BattleResult, Error> {
        let damage = find_attack(srv_data, attack.attack_id, "enemy -> player", &self.name);
        let (min_pwr, max_pwr) = match damage.attack_type {
            data_structs::stats::AttackType::Mel => (self.min_mel_pwr, self.max_mel_pwr),
            data_structs::stats::AttackType::Rng => (self.min_rng_pwr, self.max_rng_pwr),
            data_structs::stats::AttackType::Tec => (self.min_tec_pwr, self.max_tec_pwr),
        };
        let def = match damage.defense_type {
            data_structs::stats::AttackType::Mel => player.base_mel_def,
            data_structs::stats::AttackType::Rng => player.base_rng_def,
            data_structs::stats::AttackType::Tec => player.base_tec_def,
        };
        let total_mul = 1.0;
        let min_pure_attack = min_pwr.saturating_sub(def).clamp(1, u32::MAX) as f32;
        let pure_attack = max_pwr.saturating_sub(def).clamp(1, u32::MAX) as f32;
        let damage_mul = match damage.damage {
            data_structs::stats::DamageType::Generic(m) => m,
            data_structs::stats::DamageType::PA(_) => unimplemented!(),
        };
        let min_weapon_attack = min_pure_attack / 5.0 * 1.05 * damage_mul * total_mul;
        let max_weapon_attack = pure_attack / 5.0 * 1.05 * damage_mul * total_mul;

        //TODO: elemental res

        let mut rng = rand::rngs::OsRng;
        let crit_chance = rand::distributions::Uniform::new(0, 100).sample(&mut rng);
        let dmg = if crit_chance < 5 || min_weapon_attack == max_weapon_attack {
            max_weapon_attack
        } else {
            rand::distributions::Uniform::new(min_weapon_attack, max_weapon_attack).sample(&mut rng)
        }
        .round() as u32;
        player.hp = player.hp.saturating_sub(dmg);
        let dmg_packet = DamageReceivePacket {
            dmg_target: attack.target,
            dmg_inflicter: attack.inflicter,
            damage_id: damage.damage_id,
            dmg_amount: dmg as _,
            new_hp: player.hp,
            hitbox_id: attack.hitbox_id,
            x_pos: attack.x_pos,
            y_pos: attack.y_pos,
            z_pos: attack.z_pos,
            ..Default::default()
        };
        Ok(if player.hp == 0 {
            let kill_packet = EnemyKilledPacket {
                receiver: dmg_packet.receiver,
                dmg_target: dmg_packet.dmg_target,
                dmg_inflicter: dmg_packet.dmg_inflicter,
                damage_id: dmg_packet.damage_id,
                dmg_amount: dmg_packet.dmg_amount,
                new_hp: dmg_packet.new_hp,
                hitbox_id: dmg_packet.hitbox_id,
                x_pos: dmg_packet.x_pos,
                y_pos: dmg_packet.y_pos,
                z_pos: dmg_packet.z_pos,
                unk1: dmg_packet.unk1,
                unk2: dmg_packet.unk3,
                unk3: dmg_packet.unk4,
                unk4: dmg_packet.unk4,
                unk5: dmg_packet.unk5,
                unk6: dmg_packet.unk6,
                unk7: dmg_packet.unk7,
            };
            BattleResult::Killed {
                dmg_packet,
                kill_packet,
                exp_amount: 0,
            }
        } else {
            BattleResult::Damaged { dmg_packet }
        })
    }
}

/// Looks up an attack by the id the client sends. Unknown ids are logged and fall back to a generic melee attack
/// (multiplier 1.0, damage id = attack id) instead of failing, because the error message blocks the client's input.
fn find_attack(srv_data: &ServerData, attack_id: u32, dir: &str, enemy: &str) -> AttackStats {
    if let Some(a) = srv_data.attack_stats.iter().find(|a| a.attack_id == attack_id) {
        return a.clone();
    }
    log::warn!("[pso2-battle] unknown attack id {attack_id} (0x{attack_id:08x}) {dir} ({enemy}), using a generic attack");
    AttackStats {
        attack_id,
        damage_id: attack_id,
        attack_type: data_structs::stats::AttackType::Mel,
        defense_type: data_structs::stats::AttackType::Mel,
        damage: data_structs::stats::DamageType::Generic(1.0),
    }
}

/// Enemy status key of an actor name (`SoldierAnt` -> 0x5DA08B21): abs(i32(hash)), where the hash is
/// boost's hash_combine over the signed bytes of the name. Unknown names fall back to the client's default.
pub fn actor_key(name: &str) -> u32 {
    let h = name.bytes().fold(0u32, |h, c| {
        h ^ (c as i8 as i32 as u32)
            .wrapping_add(h << 6)
            .wrapping_add(h >> 2)
            .wrapping_add(0x9e37_79b9)
    });
    (h as i32).unsigned_abs()
}

#[cfg(test)]
mod tests {
    #[test]
    fn actor_key() {
        assert_eq!(super::actor_key("SoldierAnt"), 0x5DA0_8B21);
        assert_eq!(super::actor_key("SoldierAntElite"), 0x3464_D276);
        assert_eq!(super::actor_key("AntReaper"), 0x782D_2586);
    }
}
