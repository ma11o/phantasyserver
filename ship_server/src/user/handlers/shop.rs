//! [pso2_vita_offline] NPC shops: アイテムショップ (フェリシア, shop 2) and ウェポンショップ (アラガス, shop 1).
//!
//! Talking to a shop NPC shows the shop menu and sends 34-00 (u32 shop). 34-01 answers with the stock (client reader
//! 0x81215a30, receiver 0x82d41d04 hands it to the menu's listeners). Buying sends 34-02 after the confirm dialog,
//! answered by 34-03 (receiver 0x82d41e58); selling sends 34-04, answered by 34-05 (receiver 0x82d41f5e). Both
//! answers carry the new meseta and the touched bag entries.
//!
//! The item shop's stock and prices are research values, the weapon shop draws a random stock from the client's
//! weapon table per class, class level, block and time slot (rules from research, numbers provisional):
//! `data/shops.json`, made by `tools/server_shops.py`.
use super::HResult;
use crate::{Action, Error, User};
use pso2packetlib::protocol::{
    Flags, Packet, PacketHeader,
    items::{ConsumableItem, Item, ItemId, ItemType, WeaponItem},
};
use rand::{SeedableRng, seq::SliceRandom};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

#[derive(Debug, Deserialize)]
struct ShopItem {
    item: String,
    price: u32,
    sell: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ItemShop {
    id: u32,
    items: Vec<ShopItem>,
}

#[derive(Debug, Deserialize)]
struct Level {
    level_min: u32,
    rarity_max: u8,
    count: usize,
}

#[derive(Debug, Deserialize)]
struct WeaponShop {
    id: u32,
    refresh_minutes: u64,
    levels: Vec<Level>,
    per_rarity: u32,
    per_affix: u32,
}

#[derive(Debug, Deserialize)]
struct ShopFile {
    item_shop: ItemShop,
    weapon_shop: WeaponShop,
    sell_ratio: f32,
    sell_default: u32,
}

fn table() -> Option<&'static ShopFile> {
    static DATA: OnceLock<Option<ShopFile>> = OnceLock::new();
    DATA.get_or_init(|| {
        let path = std::env::var_os("PSO2_SHOPS").unwrap_or_else(|| "data/shops.json".into());
        let f = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<ShopFile>(&b).ok());
        match &f {
            Some(f) => log::info!("[pso2-shop] loaded shops, {} items in the item shop", f.item_shop.items.len()),
            None => log::warn!("[pso2-shop] cannot read {}", std::path::Path::new(&path).display()),
        }
        f
    })
    .as_ref()
}

/// The shop each user has open (34-02 / 34-04 do not name it)
fn open_shops() -> &'static Mutex<HashMap<u32, u32>> {
    static OPEN: OnceLock<Mutex<HashMap<u32, u32>>> = OnceLock::new();
    OPEN.get_or_init(Default::default)
}

fn raw(subid: u16, flag: Flags, data: Vec<u8>) -> Packet {
    Packet::Unknown((PacketHeader::new(0x34, subid, flag), data))
}

/// variable-length count as the client reads it: `(raw ^ xor) - sub`
fn count(n: u32, xor: u32, sub: u32) -> [u8; 4] {
    ((n + sub) ^ xor).to_le_bytes()
}

fn read_count(data: &[u8], at: usize, xor: u32, sub: u32) -> u32 {
    (u32_at(data, at) ^ xor).wrapping_sub(sub)
}

fn u16_at(data: &[u8], at: usize) -> u16 {
    data.get(at..at + 2).map_or(0, |b| u16::from_le_bytes(b.try_into().unwrap()))
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    data.get(at..at + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn u64_at(data: &[u8], at: usize) -> u64 {
    data.get(at..at + 8).map_or(0, |b| u64::from_le_bytes(b.try_into().unwrap()))
}

fn item_id(data: &[u8], at: usize) -> ItemId {
    ItemId {
        item_type: u16_at(data, at),
        id: u16_at(data, at + 2),
        unk3: u16_at(data, at + 4),
        subid: u16_at(data, at + 6),
    }
}

fn parse_id(spec: &str) -> Option<ItemId> {
    let n: Vec<u16> = spec.split(':').map(|s| s.parse().ok()).collect::<Option<_>>()?;
    let [t, id, subid] = n[..] else { return None };
    Some(ItemId { item_type: t, id, subid, ..Default::default() })
}

fn new_item(id: ItemId) -> Item {
    Item {
        uuid: 0,
        id,
        data: if id.item_type == 3 {
            ItemType::Consumable(ConsumableItem { amount: 1, ..Default::default() })
        } else {
            ItemType::Weapon(Default::default())
        },
    }
}

/// One line of the stock: the item as it will be handed out and its price
struct Offer {
    item: Item,
    price: u32,
}

fn weapon_rarity(user: &User, id: ItemId) -> Option<u8> {
    user.blockdata
        .server_data
        .item_params
        .attrs
        .weapons
        .iter()
        .find(|a| a.id == id.id && a.subid == id.subid)
        .map(|a| a.rarity)
}

fn weapon_price(shop: &WeaponShop, rarity: u8, w: &WeaponItem) -> u32 {
    rarity as u32 * shop.per_rarity + w.affixes.iter().filter(|&&a| a != 0).count() as u32 * shop.per_affix
}

/// The weapon shop's stock: weapons of the client's table that the main class can equip, ★ 1 up to the class
/// level's limit, with a name the server can send; `count` of them drawn with a seed of (user, class, block, time
/// slot), so the same class / block / slot shows the same list again. Each gets affixes as a drop of the class
/// level does (T57).
fn weapon_stock(user: &User, shop: &WeaponShop) -> Vec<Offer> {
    let Some(c) = user.character.as_ref() else { return vec![] };
    let class = c.character.classes.main_class;
    let level = c.character.get_level().level1 as u32;
    let Some(band) = shop.levels.iter().filter(|l| level >= l.level_min).last() else { return vec![] };
    let class_bit = 1u16 << (class as u16);
    let params = &user.blockdata.server_data.item_params;
    let mut candidates: Vec<(ItemId, u8)> = params
        .attrs
        .weapons
        .iter()
        .filter(|w| (1..=band.rarity_max).contains(&w.rarity) && w.class.bits() & class_bit != 0)
        .map(|w| (ItemId { item_type: 1, id: w.id, subid: w.subid, ..Default::default() }, w.rarity))
        .filter(|(id, _)| params.names.iter().any(|n| n.id == *id))
        .collect();
    let slot = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 60 / shop.refresh_minutes.max(1));
    let seed = (user.get_user_id() as u64) << 40 ^ (class as u64) << 32 ^ (user.blockdata.block_id as u64) << 24 ^ slot;
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    candidates.shuffle(&mut rng);
    candidates.truncate(band.count);
    candidates.sort_by_key(|(id, r)| (*r, id.id, id.subid));
    candidates
        .into_iter()
        .map(|(id, rarity)| {
            let mut item = new_item(id);
            if let ItemType::Weapon(w) = &mut item.data {
                w.affixes = crate::affixes::data().roll(&mut rng, level);
            }
            let price = match &item.data {
                ItemType::Weapon(w) => weapon_price(shop, rarity, w),
                _ => 0,
            };
            Offer { item, price }
        })
        .collect()
}

fn stock(user: &User, shop: u32) -> Vec<Offer> {
    let Some(t) = table() else { return vec![] };
    if shop == t.item_shop.id {
        t.item_shop
            .items
            .iter()
            .filter_map(|s| Some(Offer { item: new_item(parse_id(&s.item)?), price: s.price }))
            .collect()
    } else if shop == t.weapon_shop.id {
        weapon_stock(user, &t.weapon_shop)
    } else {
        vec![]
    }
}

/// A stock line as the client reads it (0x40 B, client 0x813b6290 -> 0x813b6100): ItemId 8 B, item data 0x30 B by
/// item type (weapon: affixes u16 x 8 first), u32 price, u32 (unknown, 0). No uuid.
fn offer_bytes(o: &Offer) -> Vec<u8> {
    let id = o.item.id;
    let mut e = vec![];
    for v in [id.item_type, id.id, id.unk3, id.subid] {
        e.extend(v.to_le_bytes());
    }
    if let ItemType::Weapon(w) = &o.item.data {
        for a in w.affixes {
            e.extend(a.to_le_bytes());
        }
    }
    e.resize(0x38, 0);
    e.extend(o.price.to_le_bytes());
    e.extend(0u32.to_le_bytes());
    e
}

/// 34-00 (u32 shop) -> 34-01: u16 shop, u16 lines (must equal the Vec's), u32 (unknown, 0), Vec<line 0x40 B> (count
/// (n + 0xBE) ^ 0x79EE), Vec<u16, u16> (same count coding, to the listener at 0x83ef275c; none). Names first (0F-30).
pub async fn open(user: &mut User, data: Vec<u8>) -> HResult {
    let shop = u32_at(&data, 0);
    open_shops().lock().unwrap().insert(user.get_user_id(), shop);
    let stock = stock(user, shop);
    log::info!("[pso2-shop] open shop {shop}: {} lines", stock.len());
    if let Some(c) = user.character.as_mut() {
        let items: Vec<Item> = stock.iter().map(|s| s.item.clone()).collect();
        if let Some(p) = c.inventory.load_names(&items, &user.blockdata.server_data.item_params, user.user_data.lang) {
            user.send_packet(&p).await?;
        }
    }
    let mut d = (shop as u16).to_le_bytes().to_vec();
    d.extend((stock.len() as u16).to_le_bytes());
    d.extend(0u32.to_le_bytes());
    d.extend(count(stock.len() as u32, 0x79EE, 0xBE));
    for o in &stock {
        d.extend(offer_bytes(o));
    }
    d.extend(count(0, 0x79EE, 0xBE));
    user.send_packet(&raw(0x01, Flags::PACKED, d)).await?;
    send_sell_prices(user).await?;
    Ok(Action::Nothing)
}

/// 34-09 (client reader 0x81216e90, receiver 0x82d41cda -> the bag 0x83010852 and the shop UI): Vec<u64 uuid, u32
/// price> (count (n + 0x17) ^ 0xA7DB). The sell list looks a bag entry's price up here by uuid (client 0x829cc7fc,
/// UI +0x10C); without it every price is 0 and nothing can be picked.
async fn send_sell_prices(user: &mut User) -> Result<(), Error> {
    let items: Vec<Item> = match user.character.as_ref() {
        Some(c) => c.inventory.bag_items().to_vec(),
        None => return Ok(()),
    };
    let mut d = count(items.len() as u32, 0xA7DB, 0x17).to_vec();
    for it in &items {
        d.extend(it.uuid.to_le_bytes());
        d.extend(sell_price(user, it).to_le_bytes());
    }
    user.send_packet(&raw(0x09, Flags::PACKED, d)).await?;
    Ok(())
}

/// Stack size of a bought consumable. The library's reading of the Vita consumable attributes is off for most
/// entries (ディメイト 138), so a flat 10 (provisional; the client limited a モノメイト purchase to 10 - held).
const STACK: u16 = 10;

/// 34-02 buy (client reader 0x81215de6): Vec<ItemId 8 B, u16 line, u16 amount> (count (n + 9) ^ 0xFFAC), u32 (1
/// seen; unknown, maybe bag vs storage). -> 34-03 (reader 0x81215fe4): u32 result (0 = failed), u64 meseta,
/// Vec<Item 0x38 B, i16 (0), i16 (non-zero = put in the bag)> (count (n + 0x54) ^ 0x8569) the touched bag entries,
/// Vec<Item, i16, i16, u32> (same coding; the client uses it instead of the first when non-empty; none).
pub async fn buy(user: &mut User, data: Vec<u8>) -> HResult {
    let shop = open_shops().lock().unwrap().get(&user.get_user_id()).copied().unwrap_or(0);
    let n = read_count(&data, 0, 0xFFAC, 9) as usize;
    let stock = stock(user, shop);
    let mut wanted = vec![];
    for i in 0..n.min(64) {
        let at = 4 + i * 12;
        let (id, line, amount) = (item_id(&data, at), u16_at(&data, at + 8) as usize, u16_at(&data, at + 10));
        let offer = stock.get(line).filter(|o| o.item.id == id).or_else(|| stock.iter().find(|o| o.item.id == id));
        match offer {
            Some(o) if amount > 0 => wanted.push((o, amount)),
            _ => log::warn!("[pso2-shop] buy {id:?} line {line} x{amount}: not in shop {shop}"),
        }
    }
    let cost: u64 = wanted.iter().map(|(o, a)| o.price as u64 * *a as u64).sum();
    let last_uuid = &mut user.user_data.last_uuid;
    let inv = &mut user.character.as_mut().ok_or(Error::InvalidInput("no character"))?.inventory;
    let ok = !wanted.is_empty() && inv.take_meseta(cost);
    let mut touched: Vec<Item> = vec![];
    if ok {
        for (o, a) in &wanted {
            for it in inv.add_bought(&o.item, *a, STACK, last_uuid) {
                touched.retain(|t| t.uuid != it.uuid);
                touched.push(it);
            }
            log::info!("[pso2-shop] bought {:?} x{a} at {} meseta", o.item.id, o.price);
        }
    } else {
        log::warn!("[pso2-shop] buy refused: cost {cost}, meseta {}", inv.meseta());
    }
    let meseta = inv.meseta();
    let mut d = (ok as u32).to_le_bytes().to_vec();
    d.extend(meseta.to_le_bytes());
    d.extend(count(touched.len() as u32, 0x8569, 0x54));
    for it in &touched {
        d.extend(super::lab::item_bytes(it));
        d.extend(0i16.to_le_bytes());
        d.extend(1i16.to_le_bytes());
    }
    d.extend(count(0, 0x8569, 0x54));
    user.send_packet(&raw(0x03, Flags::PACKED, d)).await?;
    send_sell_prices(user).await?;
    Ok(Action::Nothing)
}

/// What the shop pays for one of `item`
fn sell_price(user: &User, item: &Item) -> u32 {
    let Some(t) = table() else { return 0 };
    if let Some(s) = t.item_shop.items.iter().find(|s| parse_id(&s.item) == Some(item.id)) {
        return s.sell.unwrap_or((s.price as f32 * t.sell_ratio) as u32);
    }
    if let ItemType::Weapon(w) = &item.data {
        if let Some(r) = weapon_rarity(user, item.id) {
            return (weapon_price(&t.weapon_shop, r, w) as f32 * t.sell_ratio) as u32;
        }
    }
    t.sell_default
}

/// 34-04 sell (client reader 0x812163f2): Vec<20 B> (count (n + 0x9F) ^ 0x0B27), u32. -> 34-05 (reader 0x812165ea):
/// u32 result (0 = failed), u64 meseta, Vec<Item 0x38 B, u32 left (0 = gone), u32, u32> (count (n + 0xEA) ^ 0x90E5).
/// The 20 B line is read as ItemId 8 B, uuid u64, u32 amount, falling back to the uuid first (unconfirmed).
pub async fn sell(user: &mut User, data: Vec<u8>) -> HResult {
    log::info!("[pso2-shop] sell {:02x?}", data);
    let n = read_count(&data, 0, 0x0B27, 0x9F) as usize;
    let mut sold = vec![];
    let mut gained = 0u64;
    for i in 0..n.min(64) {
        let at = 4 + i * 20;
        let amount = u32_at(&data, at + 16).max(1) as u16;
        let held = |u: u64| user.character.as_ref().is_some_and(|c| c.inventory.get_inv_item(u).is_ok());
        let uuid = [u64_at(&data, at + 8), u64_at(&data, at)].into_iter().find(|&u| u != 0 && held(u));
        let Some(uuid) = uuid else {
            log::warn!("[pso2-shop] sell line {i}: no bag entry");
            continue;
        };
        let item = user.character.as_ref().unwrap().inventory.get_inv_item(uuid)?;
        let each = sell_price(user, &item);
        let inv = &mut user.character.as_mut().unwrap().inventory;
        if let Some((before, left)) = inv.take_for_sale(uuid, amount) {
            let got = each as u64 * amount as u64;
            gained += got;
            log::info!("[pso2-shop] sold {:?} x{amount} at {each} meseta, {left} left", before.id);
            sold.push((before, left, amount, got));
        }
    }
    let inv = &mut user.character.as_mut().ok_or(Error::InvalidInput("no character"))?.inventory;
    inv.add_meseta(gained);
    let meseta = inv.meseta();
    let mut d = (!sold.is_empty() as u32).to_le_bytes().to_vec();
    d.extend(meseta.to_le_bytes());
    d.extend(count(sold.len() as u32, 0x90E5, 0xEA));
    for (item, left, amount, got) in &sold {
        let mut item = item.clone();
        if let ItemType::Consumable(c) = &mut item.data {
            c.amount = *left;
        }
        d.extend(super::lab::item_bytes(&item));
        d.extend((*left as u32).to_le_bytes());
        d.extend((*amount as u32).to_le_bytes());
        d.extend((*got as u32).to_le_bytes());
    }
    user.send_packet(&raw(0x05, Flags::PACKED, d)).await?;
    send_sell_prices(user).await?;
    Ok(Action::Nothing)
}
