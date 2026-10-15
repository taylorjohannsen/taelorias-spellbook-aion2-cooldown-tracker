//! What the overlay window shows, and the one place that changes it.

use std::collections::HashMap;
use std::fmt::Display;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::skills::{self, Loadout, Skill};
use crate::tracker::Update;

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// Something that stops the overlay working, like Npcap missing
    pub error: String,
    pub character: String,
    pub unlocked: bool,
    pub skills: Vec<Skill>,
    pub cooldowns: HashMap<u32, Cooldown>,
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
    log: Mutex<Option<File>>,
    started: Instant,
}

pub fn epoch_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Overlay {
    pub fn new(app: AppHandle, dir: PathBuf) -> Self {
        let log = File::create(dir.join("tjcd.log")).ok();
        Self(Arc::new(Inner {
            app,
            dir,
            view: Mutex::default(),
            loadout: Mutex::default(),
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
        *self.0.loadout.lock().unwrap() = loadout;
        self.update(|v| {
            v.character = name;
            v.skills = bar;
        });
    }

    pub fn apply(&self, update: Update) {
        match update {
            Update::Character { name, server } => self.character(name, server),
            Update::SelfFound(entity) => self.log(format!("found our entity ({entity}) from cooldown updates")),
            Update::Cooldown { skill, ends_at, duration } => {
                self.log(format!("cast {skill}: {duration} ms cooldown"));
                self.place(skill, true);
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
        std::thread::spawn(move || match skills::fetch(&name, server) {
            Ok(loadout) => {
                overlay.log(format!("skill bar: {} slotted, {} known", loadout.bar.len(), loadout.known.len()));
                skills::save_cached(overlay.dir(), &loadout);
                overlay.set_loadout(loadout);
            }
            Err(e) => overlay.log(format!("couldn't load the skill bar: {e}")),
        });
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
            self.update(|v| v.skills.push(skill));
        }
    }
}

fn prune(view: &mut View) {
    let now = epoch_ms();
    view.cooldowns.retain(|_, c| c.ends_at > now);
}
