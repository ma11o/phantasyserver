//! [pso2_vita_offline] Urgent quests (緊急クエスト): a notice on the ticker, then the quest is listed at the
//! counter ("緊急クエスト") for a while, then the window closes. The times count from the server start
//! (`data/urgent/schedule.json`): occurrence k gives its notice at `first + k * interval`, opens `notice_s`
//! later for `open_s`, and takes the quests in turn. `PSO2_EQ=off` disables it, `PSO2_EQ=force` gives the first
//! notice at once, `PSO2_EQ=<minutes>` sets the first notice. Debug `eq now|open|end|status` moves the clock.

use serde::Deserialize;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Deserialize)]
pub struct Entry {
    pub name_id: u32,
    pub title: String,
}

#[derive(Deserialize)]
struct Schedule {
    first_min: u64,
    interval_min: u64,
    notice_s: u64,
    open_s: u64,
    quests: Vec<Entry>,
}

fn schedule() -> &'static Schedule {
    static S: OnceLock<Schedule> = OnceLock::new();
    S.get_or_init(|| serde_json::from_str(include_str!("../../data/urgent/schedule.json")).expect("data/urgent/schedule.json"))
}

/// When occurrence 0 gives its notice (None = disabled).
static BASE: Mutex<Option<Instant>> = Mutex::new(None);
static INIT: OnceLock<()> = OnceLock::new();

fn base() -> Option<Instant> {
    INIT.get_or_init(|| {
        let s = schedule();
        let first = match std::env::var("PSO2_EQ").as_deref() {
            Ok("off") => None,
            Ok("force") => Some(0),
            Ok(v) => Some(v.parse().unwrap_or(s.first_min)),
            Err(_) => Some(s.first_min),
        };
        *BASE.lock().unwrap() = first.map(|m| Instant::now() + Duration::from_secs(m * 60));
        log::info!("[pso2-urgent] first notice in {first:?} min");
    });
    *BASE.lock().unwrap()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Idle,
    /// The notice of occurrence k, seconds until it opens.
    Notice { k: u64, left: u64 },
    /// Occurrence k is open, seconds until it closes.
    Open { k: u64, left: u64 },
    /// Occurrence k has closed (until the next notice).
    Closed { k: u64 },
}

pub fn phase() -> Phase {
    let s = schedule();
    let Some(base) = base() else { return Phase::Idle };
    let now = Instant::now();
    if now < base || s.quests.is_empty() {
        return Phase::Idle;
    }
    let el = (now - base).as_secs();
    let interval = (s.interval_min * 60).max(s.notice_s + s.open_s + 1);
    let (k, r) = (el / interval, el % interval);
    if r < s.notice_s {
        Phase::Notice { k, left: s.notice_s - r }
    } else if r < s.notice_s + s.open_s {
        Phase::Open { k, left: s.notice_s + s.open_s - r }
    } else {
        Phase::Closed { k }
    }
}

pub fn quest(k: u64) -> &'static Entry {
    let q = &schedule().quests;
    &q[(k % q.len() as u64) as usize]
}

/// Urgent quests are listed at the counter only while their occurrence is open.
pub fn is_open(name_id: u32) -> bool {
    matches!(phase(), Phase::Open { k, .. } if quest(k).name_id == name_id)
}

/// The ticker line for a phase the player has not been told yet (`seen` is updated). A closed window is
/// told only to players who saw it open or announced.
pub fn announcement(seen: &mut Option<(u64, u8)>) -> Option<String> {
    let mins = |s: u64| s.div_ceil(60);
    let (key, msg) = match phase() {
        Phase::Idle => return None,
        Phase::Notice { k, left } => ((k, 1), format!("【予告】緊急クエスト「{}」が{}分後に発生します", quest(k).title, mins(left))),
        Phase::Open { k, left } => (
            (k, 2),
            format!("【緊急】「{}」発生！カウンターの緊急クエストから受注 (残り{}分)", quest(k).title, mins(left)),
        ),
        Phase::Closed { k } => {
            let told = matches!(*seen, Some((sk, _)) if sk == k);
            if *seen == Some((k, 3)) {
                return None;
            }
            *seen = Some((k, 3));
            return told.then(|| format!("緊急クエスト「{}」の受付は終了しました", quest(k).title));
        }
    };
    if *seen == Some(key) {
        return None;
    }
    *seen = Some(key);
    Some(msg)
}

/// Debug `eq now|open|end|status`: moves the clock so that a new occurrence gives its notice now / opens now /
/// closes now (the next occurrence keeps the interval from there).
pub fn debug(op: &str) -> Option<String> {
    let s = schedule();
    base();
    let now = Instant::now();
    let back = match op {
        "now" => Some(0),
        "open" => Some(s.notice_s),
        "end" => Some(s.notice_s + s.open_s),
        "status" => None,
        _ => return None,
    };
    if let Some(b) = back {
        // keep k growing so the quests still go in turn and the players get the new lines
        let interval = (s.interval_min * 60).max(s.notice_s + s.open_s + 1);
        let k = match phase() {
            Phase::Idle => 0,
            Phase::Notice { k, .. } | Phase::Open { k, .. } | Phase::Closed { k } => k + 1,
        };
        let back = Duration::from_secs(b + k * interval);
        *BASE.lock().unwrap() = Some(now.checked_sub(back).unwrap_or(now));
    }
    Some(format!("eq {:?}", phase()))
}
