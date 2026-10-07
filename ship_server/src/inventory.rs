use crate::Error;
use data_structs::inventory::{AccountStorages, ItemParameters, StorageInventory};
use pso2packetlib::protocol::{
    ObjectHeader, Packet, ProtocolRW,
    items::{
        AddedItemPacket, DiscardItemRequestPacket, DiscardStorageItemRequestPacket, EquipedItem,
        InventoryMesetaPacket, Item, ItemId, ItemType, LoadEquipedPacket, LoadItemPacket,
        LoadPlayerInventoryPacket, LoadStoragesPacket, MesetaDirection, MoveMesetaPacket,
        MoveStoragesPacket, MoveStoragesRequestPacket, MoveToInventoryPacket,
        MoveToInventoryRequestPacket, MoveToStoragePacket, MoveToStorageRequestPacket, NamedId,
        NewInventoryItem, NewStorageItem, StorageMesetaPacket, UpdateInventoryPacket,
        UpdateStoragePacket,
    },
    login::Language,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Inventory {
    pub(crate) inventory: PlayerInventory,
    pub(crate) character: StorageInventory,
    #[serde(skip)]
    pub(crate) storages: AccountStorages,

    #[serde(skip)]
    loaded_items: Vec<ItemId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerInventory {
    meseta: u64,
    max_capacity: u32,
    items: Vec<Item>,
    equiped: Vec<(u32, u64)>,
}

enum ChangeItemResult {
    Changed {
        uuid: u64,
        new_amount: u16,
        moved: u16,
        item: Item,
    },
    New {
        item: Item,
        amount: u16,
    },
    Removed {
        item: Item,
        amount: u16,
    },
}

impl Inventory {
    pub fn send(
        &mut self,
        player_id: u32,
        name: String,
        item_names: &ItemParameters,
        lang: Language,
    ) -> Vec<Packet> {
        let mut packets = vec![];

        // load inventory
        if let Some(x) = load_items_inner(
            &mut self.loaded_items,
            &self.inventory.items,
            item_names,
            lang,
        ) {
            packets.push(Packet::LoadItem(x));
        }
        packets.push(Packet::LoadPlayerInventory(LoadPlayerInventoryPacket {
            object: ObjectHeader {
                id: player_id,
                entity_type: pso2packetlib::protocol::ObjectType::Player,
                ..Default::default()
            },
            name,
            meseta: self.inventory.meseta,
            max_capacity: self.inventory.max_capacity,
            items: self.inventory.items.clone(),
        }));
        packets.push(self.send_equiped(player_id));

        // load storages
        //BUG: i think that this packet should be split if there are too many items
        let mut storage_items = vec![];
        let mut infos = vec![];
        // character storage
        if let Some(x) = load_items_inner(
            &mut self.loaded_items,
            &self.character.items,
            item_names,
            lang,
        ) {
            packets.push(Packet::LoadItem(x));
        }
        storage_items.extend_from_slice(&self.character.items);
        infos.push(self.character.generate_info());

        // default storage
        if let Some(x) = load_items_inner(
            &mut self.loaded_items,
            &self.storages.default.items,
            item_names,
            lang,
        ) {
            packets.push(Packet::LoadItem(x));
        }
        storage_items.extend_from_slice(&self.storages.default.items);
        infos.push(self.storages.default.generate_info());

        // premium storage
        if let Some(x) = load_items_inner(
            &mut self.loaded_items,
            &self.storages.premium.items,
            item_names,
            lang,
        ) {
            packets.push(Packet::LoadItem(x));
        }
        storage_items.extend_from_slice(&self.storages.premium.items);
        infos.push(self.storages.premium.generate_info());

        // extend1 storage
        if let Some(x) = load_items_inner(
            &mut self.loaded_items,
            &self.storages.extend1.items,
            item_names,
            lang,
        ) {
            packets.push(Packet::LoadItem(x));
        }
        storage_items.extend_from_slice(&self.storages.extend1.items);
        infos.push(self.storages.extend1.generate_info());

        packets.push(Packet::LoadStorages(LoadStoragesPacket {
            stored_meseta: self.storages.storage_meseta,
            unk1: infos,
            unk2: 2,
            items: storage_items,
        }));
        packets
    }
    pub fn send_equiped(&self, player_id: u32) -> Packet {
        let mut equiped_items = LoadEquipedPacket::default();
        for (pos, uuid) in &self.inventory.equiped {
            let Some(item) = self.inventory.items.iter().find(|x| x.uuid == *uuid) else {
                continue;
            };
            equiped_items.items.push(EquipedItem {
                item: item.clone(),
                // 0-back unit
                // 1-arm unit
                // 2-leg unit
                // 3-outfit
                // 9-weapon
                unk: *pos,
            });
        }
        equiped_items.player = ObjectHeader {
            id: player_id,
            entity_type: pso2packetlib::protocol::ObjectType::Player,
            ..Default::default()
        };
        Packet::LoadEquiped(equiped_items)
    }
    pub fn equip_item(&mut self, uuid: u64, pos: u32) -> Result<(), Error> {
        if self.inventory.equiped.iter().any(|&x| x.1 == uuid) {
            return Ok(());
        }
        if let Some((pos, _)) = self
            .inventory
            .equiped
            .iter()
            .enumerate()
            .find(|(_, (item_pos, _))| *item_pos == pos)
        {
            self.inventory.equiped.remove(pos);
        }
        self.inventory
            .items
            .iter()
            .find(|x| x.uuid == uuid)
            .ok_or(Error::InvalidInput("equip_item"))?;
        self.inventory.equiped.push((pos, uuid));
        Ok(())
    }
    pub fn unequip_item(&mut self, uuid: u64) -> Result<(), Error> {
        if let Some((pos, _)) = self
            .inventory
            .equiped
            .iter()
            .enumerate()
            .find(|(_, x)| x.1 == uuid)
        {
            self.inventory.equiped.remove(pos);
        }
        Ok(())
    }
    /// [pso2_vita_offline] Mutable access to an inventory item (debug edits).
    pub fn get_inv_item_mut(&mut self, uuid: u64) -> Option<&mut Item> {
        self.inventory.items.iter_mut().find(|x| x.uuid == uuid)
    }
    pub fn get_inv_item(&self, uuid: u64) -> Result<Item, Error> {
        self.inventory
            .items
            .iter()
            .find(|x| x.uuid == uuid)
            .ok_or(Error::InvalidInput("get_inv_item"))
            .cloned()
    }
    pub fn move_to_storage(
        &mut self,
        packet: MoveToStorageRequestPacket,
        new_uuid: &mut u64,
    ) -> Result<Packet, Error> {
        let mut packet_out = MoveToStoragePacket::default();
        for info in packet.uuids {
            let storage = match info.storage_id {
                0 => &mut self.storages.default,
                1 => &mut self.storages.premium,
                2 => &mut self.storages.extend1,
                14 => &mut self.character,
                _ => return Err(Error::InvalidInput("move_to_storage")),
            };
            let result = decrease_item(&mut self.inventory.items, info.uuid, info.amount as u16)?;
            let (item, amount) = match result {
                ChangeItemResult::Changed {
                    uuid,
                    new_amount,
                    moved,
                    mut item,
                } => {
                    packet_out.updated_inventory.push(
                        pso2packetlib::protocol::items::UpdatedInventoryItem {
                            uuid,
                            new_amount,
                            moved,
                        },
                    );
                    *new_uuid += 1;
                    item.uuid = *new_uuid;
                    (item, moved)
                }
                ChangeItemResult::Removed { item, amount } => {
                    packet_out.updated_inventory.push(
                        pso2packetlib::protocol::items::UpdatedInventoryItem {
                            uuid: item.uuid,
                            new_amount: 0,
                            moved: amount,
                        },
                    );
                    (item, amount)
                }
                _ => unreachable!(),
            };
            match increase_item(&mut storage.items, item, amount)? {
                ChangeItemResult::Changed {
                    uuid, new_amount, ..
                } => {
                    packet_out
                        .updated
                        .push(pso2packetlib::protocol::items::UpdatedItem {
                            uuid,
                            new_amount: new_amount as u32,
                            storage_id: storage.storage_id as u32,
                        });
                }
                ChangeItemResult::New { item, .. } => {
                    packet_out.new_items.push(NewStorageItem {
                        item,
                        storage_id: storage.storage_id as u32,
                    });
                }
                _ => unreachable!(),
            }
        }
        Ok(Packet::MoveToStorage(packet_out))
    }
    pub fn move_to_inventory(
        &mut self,
        packet: MoveToInventoryRequestPacket,
        new_uuid: &mut u64,
    ) -> Result<Packet, Error> {
        let mut packet_out = MoveToInventoryPacket::default();
        for info in packet.uuids {
            let storage = match info.storage_id {
                0 => &mut self.storages.default,
                1 => &mut self.storages.premium,
                2 => &mut self.storages.extend1,
                14 => &mut self.character,
                _ => return Err(Error::InvalidInput("move_to_inventory")),
            };
            let result = decrease_item(&mut storage.items, info.uuid, info.amount as u16)?;
            let (item, amount) = match result {
                ChangeItemResult::Changed {
                    uuid,
                    new_amount,
                    moved,
                    mut item,
                } => {
                    packet_out
                        .updated
                        .push(pso2packetlib::protocol::items::UpdatedStorageItem {
                            uuid,
                            new_amount,
                            storage_id: storage.storage_id as u32,
                            moved,
                        });
                    *new_uuid += 1;
                    item.uuid = *new_uuid;
                    (item, moved)
                }
                ChangeItemResult::Removed { item, amount } => {
                    packet_out
                        .updated
                        .push(pso2packetlib::protocol::items::UpdatedStorageItem {
                            uuid: item.uuid,
                            new_amount: 0,
                            storage_id: storage.storage_id as u32,
                            moved: amount,
                        });
                    (item, amount)
                }
                _ => unreachable!(),
            };
            match increase_item(&mut self.inventory.items, item, amount)? {
                ChangeItemResult::Changed {
                    new_amount, item, ..
                } => {
                    packet_out.new_items.push(NewInventoryItem {
                        item,
                        amount: new_amount,
                        is_new: 0,
                    });
                }
                ChangeItemResult::New { item, amount } => {
                    packet_out.new_items.push(NewInventoryItem {
                        item,
                        amount,
                        is_new: 1,
                    });
                }
                _ => unreachable!(),
            }
        }
        Ok(Packet::MoveToInventory(packet_out))
    }
    pub fn move_storages(
        &mut self,
        packet: MoveStoragesRequestPacket,
        new_uuid: &mut u64,
    ) -> Result<Packet, Error> {
        let mut packet_out = MoveStoragesPacket::default();
        for info in packet.items {
            let storage_src = match packet.old_id {
                0 => &mut self.storages.default,
                1 => &mut self.storages.premium,
                2 => &mut self.storages.extend1,
                14 => &mut self.character,
                _ => return Err(Error::InvalidInput("move_storages")),
            };
            let result = decrease_item(&mut storage_src.items, info.uuid, info.amount)?;
            let (item, amount) = match result {
                ChangeItemResult::Changed {
                    uuid,
                    new_amount,
                    moved,
                    mut item,
                } => {
                    packet_out.updated_old.push(
                        pso2packetlib::protocol::items::UpdatedStorageItem {
                            uuid,
                            new_amount,
                            storage_id: storage_src.storage_id as u32,
                            moved,
                        },
                    );
                    *new_uuid += 1;
                    item.uuid = *new_uuid;
                    (item, moved)
                }
                ChangeItemResult::Removed { item, amount } => {
                    packet_out.updated_old.push(
                        pso2packetlib::protocol::items::UpdatedStorageItem {
                            uuid: item.uuid,
                            new_amount: 0,
                            storage_id: storage_src.storage_id as u32,
                            moved: amount,
                        },
                    );
                    (item, amount)
                }
                _ => unreachable!(),
            };
            let storage_dst = match packet.new_id {
                0 => &mut self.storages.default,
                1 => &mut self.storages.premium,
                2 => &mut self.storages.extend1,
                14 => &mut self.character,
                _ => return Err(Error::InvalidInput("move_storages")),
            };
            match increase_item(&mut storage_dst.items, item, amount)? {
                ChangeItemResult::Changed {
                    new_amount,
                    uuid,
                    moved,
                    ..
                } => {
                    packet_out.updated_new.push(
                        pso2packetlib::protocol::items::UpdatedStorageItem {
                            uuid,
                            new_amount,
                            moved,
                            storage_id: storage_dst.storage_id as u32,
                        },
                    );
                }
                ChangeItemResult::New { item, .. } => {
                    packet_out.new_items.push(NewStorageItem {
                        item,
                        storage_id: storage_dst.storage_id as u32,
                    });
                }
                _ => unreachable!(),
            }
        }
        Ok(Packet::MoveStorages(packet_out))
    }
    pub fn discard_inventory(&mut self, packet: DiscardItemRequestPacket) -> Result<Packet, Error> {
        let mut packet_out = UpdateInventoryPacket {
            unk2: 1,
            ..Default::default()
        };
        for info in packet.items {
            match decrease_item(&mut self.inventory.items, info.uuid, info.amount)? {
                ChangeItemResult::Changed {
                    new_amount, moved, ..
                } => {
                    packet_out
                        .updated
                        .push(pso2packetlib::protocol::items::UpdatedInventoryItem {
                            uuid: info.uuid,
                            new_amount,
                            moved,
                        })
                }
                ChangeItemResult::Removed { amount, .. } => {
                    packet_out
                        .updated
                        .push(pso2packetlib::protocol::items::UpdatedInventoryItem {
                            uuid: info.uuid,
                            new_amount: 0,
                            moved: amount,
                        })
                }
                _ => unreachable!(),
            }
        }
        Ok(Packet::UpdateInventory(packet_out))
    }
    pub fn discard_storage(
        &mut self,
        packet: DiscardStorageItemRequestPacket,
    ) -> Result<Packet, Error> {
        let mut packet_out = UpdateStoragePacket {
            unk2: 1,
            ..Default::default()
        };
        for info in packet.items {
            let storage = match info.storage_id {
                0 => &mut self.storages.default,
                1 => &mut self.storages.premium,
                2 => &mut self.storages.extend1,
                14 => &mut self.character,
                _ => return Err(Error::InvalidInput("discard_storage")),
            };
            match decrease_item(&mut storage.items, info.uuid, info.amount as u16)? {
                ChangeItemResult::Changed {
                    new_amount, moved, ..
                } => packet_out
                    .updated
                    .push(pso2packetlib::protocol::items::UpdatedStorageItem {
                        uuid: info.uuid,
                        new_amount,
                        moved,
                        storage_id: storage.storage_id as u32,
                    }),
                ChangeItemResult::Removed { amount, .. } => {
                    packet_out
                        .updated
                        .push(pso2packetlib::protocol::items::UpdatedStorageItem {
                            uuid: info.uuid,
                            new_amount: 0,
                            moved: amount,
                            storage_id: storage.storage_id as u32,
                        })
                }
                _ => unreachable!(),
            }
        }
        Ok(Packet::UpdateStorage(packet_out))
    }
    pub fn move_meseta(&mut self, packet: MoveMesetaPacket) -> Vec<Packet> {
        let mut packets = vec![];
        let (src, dest) = match packet.direction {
            MesetaDirection::ToStorage => (
                &mut self.inventory.meseta,
                &mut self.storages.storage_meseta,
            ),
            MesetaDirection::ToInventory => (
                &mut self.storages.storage_meseta,
                &mut self.inventory.meseta,
            ),
        };
        let to_move = u64::min(*src, packet.meseta);
        *dest += to_move;
        *src -= to_move;
        packets.push(Packet::InventoryMeseta(InventoryMesetaPacket {
            meseta: self.inventory.meseta,
        }));
        packets.push(Packet::StorageMeseta(StorageMesetaPacket {
            meseta: self.storages.storage_meseta,
        }));
        packets
    }
    pub fn add_item(&mut self, item: Item) -> Packet {
        let packet = Packet::AddedItem(AddedItemPacket {
            item: item.clone(),
            ..Default::default()
        });
        self.inventory.items.push(item);
        packet
    }
    /// [pso2_vita_offline] An item picked up from the ground. A consumable goes onto a stack of the same id while it
    /// stays at 10 or less (UpdateInventory 0F-06, as discarding / using does); anything else is a new entry
    /// (AddedItem 0F-05) with a fresh uuid.
    pub fn add_picked_item(&mut self, mut item: Item, uuid: &mut u64) -> Packet {
        if let ItemType::Consumable(new) = &item.data {
            let add = new.amount;
            if let Some((stack_uuid, stack)) = self.inventory.items.iter_mut().find_map(|i| match &mut i.data {
                ItemType::Consumable(c) if i.id == item.id && c.amount + add <= 10 => Some((i.uuid, c)),
                _ => None,
            }) {
                stack.amount += add;
                return Packet::UpdateInventory(UpdateInventoryPacket {
                    updated: vec![pso2packetlib::protocol::items::UpdatedInventoryItem {
                        uuid: stack_uuid,
                        new_amount: stack.amount,
                        moved: add,
                    }],
                    unk2: 1,
                    ..Default::default()
                });
            }
        }
        // the client keys its inventory by uuid; last_uuid can be behind the inventory's uuids (1 vs 4 seen), so never
        // go below them. The client drops an AddedItem of an id missing from the item attributes it was sent
        // (max stack 0), see data_compiler's Vita item_parameter.bin
        let next = self.inventory.items.iter().map(|i| i.uuid).max().unwrap_or(0) + 1;
        log::info!("[pso2-drop] new inventory entry: last_uuid {} inventory max+1 {next}", *uuid);
        *uuid = (*uuid).max(next);
        item.uuid = *uuid;
        *uuid += 1;
        self.add_item(item)
    }
    /// [pso2_vita_offline] One of a consumable used (T62): the first stack of `id` goes down by one, answered with
    /// UpdateInventory (0F-06) as discarding is. None when none is held.
    pub fn use_consumable(&mut self, id: ItemId) -> Option<Packet> {
        let uuid = self
            .inventory
            .items
            .iter()
            .find(|i| i.id == id && matches!(i.data, ItemType::Consumable(_)))?
            .uuid;
        let new_amount = match decrease_item(&mut self.inventory.items, uuid, 1).ok()? {
            ChangeItemResult::Changed { new_amount, .. } => new_amount,
            _ => 0,
        };
        Some(Packet::UpdateInventory(UpdateInventoryPacket {
            updated: vec![pso2packetlib::protocol::items::UpdatedInventoryItem {
                uuid,
                new_amount,
                moved: 1,
            }],
            unk2: 1,
            ..Default::default()
        }))
    }
    /// [pso2_vita_offline] Shop (shop.rs): the bag's entries
    pub fn bag_items(&self) -> &[Item] {
        &self.inventory.items
    }
    /// [pso2_vita_offline] Shop (shop.rs): LoadItem (0F-30) for the ids the client has no name for yet
    pub fn load_names(&mut self, items: &[Item], item_names: &ItemParameters, lang: Language) -> Option<Packet> {
        load_items_inner(&mut self.loaded_items, items, item_names, lang).map(Packet::LoadItem)
    }
    /// [pso2_vita_offline] Shop (shop.rs): `amount` of a bought item into the bag. A consumable fills stacks of the
    /// same id up to `stack` and opens new ones; anything else is one new entry per piece. Returns every touched
    /// entry as it is now. `uuid` is the user's last_uuid (never below the bag's uuids, as add_picked_item).
    pub fn add_bought(&mut self, item: &Item, amount: u16, stack: u16, uuid: &mut u64) -> Vec<Item> {
        let mut out = vec![];
        let mut left = amount;
        let mut next_uuid = |items: &Vec<Item>| {
            let next = items.iter().map(|i| i.uuid).max().unwrap_or(0) + 1;
            *uuid = (*uuid).max(next);
            let u = *uuid;
            *uuid += 1;
            u
        };
        if let ItemType::Consumable(_) = &item.data {
            for i in self.inventory.items.iter_mut().filter(|i| i.id == item.id) {
                if let ItemType::Consumable(c) = &mut i.data {
                    let add = left.min(stack.saturating_sub(c.amount));
                    if add > 0 {
                        c.amount += add;
                        left -= add;
                        out.push(i.clone());
                    }
                }
            }
            while left > 0 {
                let n = left.min(stack.max(1));
                let mut new = item.clone();
                if let ItemType::Consumable(c) = &mut new.data {
                    c.amount = n;
                }
                new.uuid = next_uuid(&self.inventory.items);
                self.inventory.items.push(new.clone());
                out.push(new);
                left -= n;
            }
        } else {
            for _ in 0..amount {
                let mut new = item.clone();
                new.uuid = next_uuid(&self.inventory.items);
                self.inventory.items.push(new.clone());
                out.push(new);
            }
        }
        out
    }
    /// [pso2_vita_offline] Shop: take `amount` of a bag entry to sell. Returns the entry as it was and the amount left
    /// (0 = gone), or None when it is not in the bag.
    pub fn take_for_sale(&mut self, uuid: u64, amount: u16) -> Option<(Item, u16)> {
        let before = self.inventory.items.iter().find(|i| i.uuid == uuid)?.clone();
        match decrease_item(&mut self.inventory.items, uuid, amount.max(1)).ok()? {
            ChangeItemResult::Changed { new_amount, .. } => Some((before, new_amount)),
            _ => Some((before, 0)),
        }
    }
    /// [pso2_vita_offline] Item lab (lab.rs): held meseta
    pub fn meseta(&self) -> u64 {
        self.inventory.meseta
    }
    /// [pso2_vita_offline] Item lab: pay `amount` meseta if there is enough
    pub fn take_meseta(&mut self, amount: u64) -> bool {
        if self.inventory.meseta < amount {
            return false;
        }
        self.inventory.meseta -= amount;
        true
    }
    /// [pso2_vita_offline] Item lab: how many of a consumable (all stacks) are in the bag
    pub fn count_items(&self, id: ItemId) -> u32 {
        self.inventory
            .items
            .iter()
            .filter(|i| i.id == id)
            .map(|i| match &i.data {
                ItemType::Consumable(c) => c.amount as u32,
                _ => 1,
            })
            .sum()
    }
    /// [pso2_vita_offline] Item lab: take `amount` of a consumable from the bag's stacks. Returns (uuid, amount left)
    /// per touched stack (0 = the stack is gone). Call `count_items` first.
    pub fn take_items(&mut self, id: ItemId, mut amount: u16) -> Vec<(u64, u16)> {
        let mut out = vec![];
        while amount > 0 {
            let Some(uuid) = self.inventory.items.iter().find(|i| i.id == id).map(|i| i.uuid) else {
                break;
            };
            match decrease_item(&mut self.inventory.items, uuid, amount) {
                Ok(ChangeItemResult::Changed { new_amount, moved, .. }) => {
                    amount -= moved;
                    out.push((uuid, new_amount));
                }
                Ok(ChangeItemResult::Removed { amount: taken, .. }) => {
                    amount = amount.saturating_sub(taken);
                    out.push((uuid, 0));
                }
                _ => break,
            }
        }
        out
    }
    /// [pso2_vita_offline] Picked-up meseta: the held amount goes up, InventoryMeseta (0F-14) with the new total.
    pub fn add_meseta(&mut self, amount: u64) -> Packet {
        self.inventory.meseta += amount;
        log::info!("[pso2-drop] meseta +{amount} -> {}", self.inventory.meseta);
        Packet::InventoryMeseta(InventoryMesetaPacket {
            meseta: self.inventory.meseta,
        })
    }
    pub fn add_default_item(&mut self, uuid: &mut u64, item_id: ItemId) -> Packet {
        let item = Item {
            uuid: *uuid,
            id: item_id,
            data: ItemType::default(),
        };
        *uuid += 1;

        // transform item data into known item data
        let packet = Packet::AddedItem(AddedItemPacket {
            item,
            ..Default::default()
        })
        .write(pso2packetlib::protocol::PacketType::NA);
        let packet = Packet::read(&packet, pso2packetlib::protocol::PacketType::NA)
            .expect("Reading from memory shouldn't fail")
            .pop()
            .expect("Should always contain an item");
        let Packet::AddedItem(added_item) = &packet else {
            unreachable!("Read and write impls should agree");
        };
        self.inventory.items.push(added_item.item.clone());

        packet
    }
}
fn load_items_inner(
    loaded: &mut Vec<ItemId>,
    items: &[Item],
    item_names: &ItemParameters,
    lang: Language,
) -> Option<LoadItemPacket> {
    let mut load_items = LoadItemPacket::default();
    for item in items {
        if !loaded.contains(&item.id) {
            loaded.push(item.id);
            match item_names.names.iter().find(|x| x.id == item.id) {
                Some(name) => load_items.items.push(NamedId {
                    name: match lang {
                        Language::English => name.en_name.clone(),
                        Language::Japanese => name.jp_name.clone(),
                    },
                    id: item.id,
                }),
                None => {
                    log::debug!("No item name for {:?}", item.id);
                    continue;
                }
            }
        }
    }
    if load_items.items.is_empty() {
        None
    } else {
        Some(load_items)
    }
}

fn decrease_item(items: &mut Vec<Item>, uuid: u64, amount: u16) -> Result<ChangeItemResult, Error> {
    let (pos, item) = items
        .iter_mut()
        .enumerate()
        .find(|(_, x)| x.uuid == uuid)
        .ok_or(Error::InvalidInput("decrease_item"))?;
    if let ItemType::Consumable(data) = &mut item.data {
        let taken = u16::min(amount, data.amount);
        let new_amount = data.amount.saturating_sub(taken);
        if new_amount == 0 {
            Ok(ChangeItemResult::Removed {
                item: items.swap_remove(pos),
                amount: taken,
            })
        } else {
            data.amount = new_amount;
            let mut data = data.clone();
            data.amount = taken;
            let item = Item {
                uuid: 0,
                id: item.id,
                data: ItemType::Consumable(data),
            };
            Ok(ChangeItemResult::Changed {
                uuid,
                new_amount,
                moved: taken,
                item,
            })
        }
    } else {
        if amount > 1 {
            return Err(Error::InvalidInput("decrease_item"));
        }
        Ok(ChangeItemResult::Removed {
            item: items.swap_remove(pos),
            amount: 1,
        })
    }
}

fn increase_item(
    items: &mut Vec<Item>,
    item: Item,
    amount: u16,
) -> Result<ChangeItemResult, Error> {
    // [pso2_vita_offline] only consumables stack; a second weapon of the same id is its own entry (fork元 failed
    // with InvalidInput after the item had already left the other side, losing it)
    let inv_item = items
        .iter_mut()
        .find(|x| x.id == item.id && matches!((&x.data, &item.data), (ItemType::Consumable(_), ItemType::Consumable(_))));
    match inv_item {
        Some(i_item) => {
            if let ItemType::Consumable(i_data) = &mut i_item.data {
                i_data.amount += amount;
                Ok(ChangeItemResult::Changed {
                    uuid: i_item.uuid,
                    new_amount: i_data.amount,
                    moved: amount,
                    item: i_item.clone(),
                })
            } else {
                Err(Error::InvalidInput("increase_item"))
            }
        }
        None => {
            items.push(item.clone());
            Ok(ChangeItemResult::New { item, amount })
        }
    }
}

impl Default for PlayerInventory {
    fn default() -> Self {
        Self {
            meseta: 0,
            max_capacity: 50,
            items: vec![],
            equiped: vec![],
        }
    }
}
