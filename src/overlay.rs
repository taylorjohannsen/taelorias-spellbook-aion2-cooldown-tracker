//! What the overlay window shows, and the one place that changes it.

use std::collections::HashMap;
use std::fmt::Display;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::packets::{buff_key, class_of, is_dungeon};
use crate::skills::{self, Loadout, Skill};
use crate::tracker::Update;

/// The last map, so a restart inside a dungeon still knows it's in one
const MAP_FILE: &str = "map.json";
/// How many recent buffs the settings window offers to track
const SEEN_BUFFS: usize = 40;
/// Kept between sessions, so Spellbook learns which skills buff us
const SEEN_FILE: &str = "seen-buffs.json";
/// Shorter ones, like the game's 200 ms movement states, would bury the rest
const MIN_SEEN_MS: u32 = 1_000;

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// Something that stops the overlay working, like Npcap missing
    pub error: String,
    /// The game's traffic has been seen
    pub traffic: bool,
    /// We know which character is ours, from a loading screen or our casts
    pub connected: bool,
    /// The game, or one of our own windows, is in front
    pub focused: bool,
    pub character: String,
    /// The character's class (see `class_of`), 0 until its skills are known
    pub class: u32,
    pub unlocked: bool,
    pub skills: Vec<Skill>,
    pub cooldowns: HashMap<u32, Cooldown>,
    /// In combat, as the server sees it
    pub combat: bool,
    /// Unix ms when combat last ended, so the bar can stay up a little after
    pub combat_ended: u64,
    pub dungeon: bool,
    /// The settings window is open, so the bar shows even when it would be hidden
    pub editing: bool,
    /// Buffs on us, by `buff_key`. An `ends_at` of 0 lasts until it's removed.
    pub buffs: HashMap<u32, Cooldown>,
    /// Buffs that have landed on us, newest first, for picking what to track
    pub seen_buffs: Vec<BuffInfo>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BuffInfo {
    /// `buff_key`
    pub id: u32,
    /// The skill behind it, from NC; empty for buffs that aren't a class's
    pub name: String,
    pub icon: String,
    pub duration: u32,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Cooldown {
    /// Unix ms, so the window can count down on its own clock
    pub ends_at: u64,
    pub duration: u32,
}

#[derive(Clone)]
pub struct Overlay(Arc<Inner>);

struct Inner {
    app: AppHandle,
    dir: PathBuf,
    view: Mutex<View>,
    loadout: Mutex<Loadout>,
    /// Every class's skills, by id, to name other players' buffs
    classes: Mutex<HashMap<u32, Skill>>,
    log: Mutex<Option<File>>,
    started: Instant,
}

pub fn epoch_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Overlay {
    pub fn new(app: AppHandle, dir: PathBuf) -> Self {
        let log = File::create(dir.join("spellbook.log")).ok();
        let map: Option<u32> = std::fs::read(dir.join(MAP_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok());
        let seen_buffs = std::fs::read(dir.join(SEEN_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let view = View { dungeon: map.is_some_and(is_dungeon), seen_buffs, ..Default::default() };
        Self(Arc::new(Inner {
            app,
            dir,
            view: Mutex::new(view),
            loadout: Mutex::default(),
            classes: Mutex::default(),
            log: Mutex::new(log),
            started: Instant::now(),
        }))
    }

    pub fn dir(&self) -> &Path {
        &self.0.dir
    }

    pub fn view(&self) -> View {
        self.0.view.lock().unwrap().clone()
    }

    pub fn class(&self) -> u32 {
        self.0.view.lock().unwrap().class
    }

    pub fn update(&self, change: impl FnOnce(&mut View)) {
        let view = {
            let mut view = self.0.view.lock().unwrap();
            change(&mut view);
            view.clone()
        };
        let _ = self.0.app.emit("view", view);
    }

    pub fn log(&self, message: impl Display) {
        let line = format!("[{:>9.3}] {message}", self.0.started.elapsed().as_secs_f64());
        #[cfg(debug_assertions)]
        eprintln!("{line}");
        if let Some(file) = self.0.log.lock().unwrap().as_mut() {
            let _ = writeln!(file, "{line}");
        }
    }

    pub fn error(&self, message: impl Into<String>) {
        let message = message.into();
        self.log(format!("error: {message}"));
        self.update(|v| v.error = message);
    }

    /// Show a character's bar, keeping skills that were placed by casting them
    pub fn set_loadout(&self, loadout: Loadout) {
        let mut bar = loadout.bar.clone();
        for skill in self.view().skills {
            if !bar.iter().any(|s| s.id == skill.id) {
                if let Some(known) = loadout.known.iter().find(|k| k.id == skill.id) {
                    bar.push(known.clone());
                }
            }
        }
        let name = loadout.name.clone();
        let known = class_skills(&loadout);
        *self.0.loadout.lock().unwrap() = loadout;
        self.update(|v| {
            v.character = name;
            v.class = class_of(bar.iter().chain(&known).map(|s| s.id).find(|&id| class_of(id) != 0).unwrap_or(0));
            v.skills = bar;
            // buffs seen before the skill list arrived
            for buff in v.seen_buffs.iter_mut().filter(|b| b.name.is_empty()) {
                if let Some(skill) = known.iter().find(|k| k.id == buff.id) {
                    buff.name = skill.name.clone();
                    buff.icon = skill.icon.clone();
                }
            }
        });
    }

    pub fn apply(&self, update: Update) {
        match update {
            Update::Character { name, server } => {
                self.update(|v| v.connected = true);
                self.character(name, server);
            }
            Update::SelfFound(entity) => {
                self.log(format!("found our entity ({entity}) from cooldown updates"));
                self.update(|v| v.connected = true);
            }
            Update::Cooldown { skill, ends_at, duration } => {
                self.log(format!("cast {skill}: {duration} ms cooldown"));
                self.place(skill, true);
                let _ = self.0.app.emit("cast", skill);
                self.update(|v| {
                    v.cooldowns.insert(skill, Cooldown { ends_at, duration });
                    prune(v);
                });
            }
            Update::Remaining { skill, ends_at, left } => {
                self.log(format!("cooldown {skill}: {left} ms left"));
                self.place(skill, false);
                let now = epoch_ms();
                self.update(|v| {
                    if left == 0 {
                        v.cooldowns.remove(&skill);
                    } else {
                        // a reduction keeps the original length so the sweep doesn't jump
                        let duration = v
                            .cooldowns
                            .get(&skill)
                            .filter(|c| c.ends_at > now && c.duration >= left)
                            .map_or(left, |c| c.duration);
                        v.cooldowns.insert(skill, Cooldown { ends_at, duration });
                    }
                    prune(v);
                });
            }
            Update::Combat(on) => {
                self.log(format!("combat {}", if on { "on" } else { "off" }));
                self.update(|v| {
                    if v.combat && !on {
                        v.combat_ended = epoch_ms();
                    }
                    v.combat = on;
                });
            }
            Update::Buff { buff, ends_at, duration } => {
                let id = buff_key(buff);
                let skill = self.buff_info(id);
                let mut added = None;
                self.update(|v| {
                    v.buffs.insert(id, Cooldown { ends_at, duration });
                    let now = epoch_ms();
                    v.buffs.retain(|_, b| b.ends_at == 0 || b.ends_at > now);
                    // a refresh keeps its place, so the list doesn't shuffle while it's read
                    if let Some(seen) = v.seen_buffs.iter_mut().find(|b| b.id == id) {
                        seen.duration = duration;
                    } else if duration == 0 || duration >= MIN_SEEN_MS {
                        let (name, icon) = skill.map(|s| (s.name, s.icon)).unwrap_or_default();
                        v.seen_buffs.insert(0, BuffInfo { id, name, icon, duration });
                        v.seen_buffs.truncate(SEEN_BUFFS);
                        added = Some(v.seen_buffs.clone());
                    }
                });
                if let Some(seen) = added {
                    self.save_seen(&seen);
                }
            }
            Update::BuffEnd(buff) => {
                let id = buff_key(buff);
                self.update(|v| {
                    v.buffs.remove(&id);
                });
            }
            Update::Map(map) => {
                let dungeon = is_dungeon(map);
                self.log(format!("map {map}{}", if dungeon { " (dungeon)" } else { "" }));
                let _ = std::fs::write(self.0.dir.join(MAP_FILE), map.to_string());
                // a loading screen ends any fight
                self.update(|v| {
                    v.combat = false;
                    v.dungeon = dungeon;
                });
            }
        }
    }

    fn character(&self, name: String, server: u16) {
        self.log(format!("character: {name} ({server})"));
        let known = {
            let loadout = self.0.loadout.lock().unwrap();
            loadout.name == name && loadout.server == server
        };
        if !known {
            self.update(|v| {
                v.skills.clear();
                v.cooldowns.clear();
            });
            self.set_loadout(Loadout { name: name.clone(), server, ..Default::default() });
        }
        // the bar may have changed since last time, so ask NC every session
        let overlay = self.clone();
        std::thread::spawn(move || {
            match skills::fetch(&name, server) {
                Ok(loadout) => {
                    overlay.log(format!("skill bar: {} slotted, {} known", loadout.bar.len(), loadout.known.len()));
                    skills::save_cached(overlay.dir(), &loadout);
                    overlay.set_loadout(loadout);
                }
                Err(e) => overlay.log(format!("couldn't load the skill bar: {e}")),
            }
            overlay.refresh_classes(server);
        });
    }

    /// Everything the class has, for picking buffs to track
    pub fn class_skills(&self) -> Vec<Skill> {
        class_skills(&self.0.loadout.lock().unwrap())
    }

    /// A name and icon for a buff tracked by `buff_key`: the skill behind it,
    /// ours or another class's
    pub fn buff_info(&self, id: u32) -> Option<Skill> {
        let skill = class_skills(&self.0.loadout.lock().unwrap()).into_iter().find(|s| s.id == id);
        skill.or_else(|| self.0.classes.lock().unwrap().get(&id).cloned())
    }

    /// Names everything, including buffs seen before the classes were known.
    /// "names" tells the app, which names tracked buffs.
    pub fn set_classes(&self, skills: Vec<Skill>) {
        *self.0.classes.lock().unwrap() = skills.into_iter().map(|s| (s.id, s)).collect();
        let unnamed: Vec<u32> = self.view().seen_buffs.iter().filter(|b| b.name.is_empty()).map(|b| b.id).collect();
        let found: Vec<Skill> = unnamed.into_iter().filter_map(|id| self.buff_info(id)).collect();
        if !found.is_empty() {
            self.update(|v| {
                for skill in found {
                    if let Some(seen) = v.seen_buffs.iter_mut().find(|b| b.id == skill.id) {
                        seen.name = skill.name;
                        seen.icon = skill.icon;
                    }
                }
            });
            self.save_seen(&self.view().seen_buffs);
        }
        let _ = self.0.app.emit("names", ());
    }

    /// Other classes' skills, which name their buffs, from NC when they're a week
    /// old. They only change with patches. Blocks while it asks.
    pub fn refresh_classes(&self, server: u16) {
        if skills::load_classes(self.dir()).is_some_and(|(_, fresh)| fresh) {
            return;
        }
        match skills::fetch_classes(server) {
            Ok(classes) => {
                self.log(format!("class skills: {}", classes.len()));
                skills::save_classes(self.dir(), &classes);
                self.set_classes(classes);
            }
            Err(e) => self.log(format!("couldn't load other classes' skills: {e}")),
        }
    }

    fn save_seen(&self, seen: &[BuffInfo]) {
        if let Ok(json) = serde_json::to_vec(seen) {
            let _ = std::fs::write(self.0.dir.join(SEEN_FILE), json);
        }
    }

    /// Put a skill we used on the bar if it isn't there: one NC doesn't list as
    /// slotted, or anything with a cooldown when NC couldn't be reached
    fn place(&self, skill: u32, cast: bool) {
        if self.0.view.lock().unwrap().skills.iter().any(|s| s.id == skill) {
            return;
        }
        let found = {
            let loadout = self.0.loadout.lock().unwrap();
            match loadout.known.iter().find(|s| s.id == skill) {
                Some(s) => Some(s.clone()),
                None if loadout.known.is_empty() && cast => Some(Skill { id: skill, name: String::new(), icon: String::new() }),
                None => None,
            }
        };
        if let Some(skill) = found {
            self.update(|v| {
                if v.class == 0 {
                    v.class = class_of(skill.id);
                }
                v.skills.push(skill);
            });
        }
    }
}

fn class_skills(loadout: &Loadout) -> Vec<Skill> {
    loadout.known.iter().chain(&loadout.passive).cloned().collect()
}

fn prune(view: &mut View) {
    let now = epoch_ms();
    view.cooldowns.retain(|_, c| c.ends_at > now);
}
