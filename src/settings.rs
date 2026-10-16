//! How the bar looks and which skills it shows, saved next to the cached loadout.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::packets::class_of;

const FILE: &str = "settings.json";
const MAX_HOTKEY: usize = 8;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

/// When the bar is up. The rest of the time it fades to `hidden_opacity`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Show {
    #[default]
    Always,
    Combat,
    Dungeon,
    /// In combat, inside a dungeon
    DungeonCombat,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BuffStyle {
    #[default]
    Boxes,
    Bars,
}

/// What a tracked buff that isn't on us looks like
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Missing {
    #[default]
    Dim,
    Hide,
}

/// Which way a column of bars stacks from where it's placed
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Grow {
    #[default]
    Down,
    Up,
}

/// Name and icon are kept from when it was picked, since only our own class's
/// buffs can be looked up again
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TrackedBuff {
    /// `packets::buff_key`
    pub id: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
}

/// Sizes are CSS pixels. Skill ids belong to one class, so the per-skill
/// settings can be shared by every character.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub orientation: Orientation,
    pub icon_width: u32,
    pub icon_height: u32,
    pub radius: u32,
    pub gap: u32,
    /// Percent, for everything the overlay shows
    pub opacity: u32,
    pub show: Show,
    /// Percent
    pub hidden_opacity: u32,
    /// Seconds the bar stays after combat, which also bridges the gap between pulls
    pub fade_delay: u32,
    /// Hide the overlay while something other than the game is in front
    pub only_focused: bool,
    /// Skills in the order they were arranged. Anything else follows in bar order.
    pub order: Vec<u32>,
    pub hidden: Vec<u32>,
    /// The game doesn't share its keybinds, so these are typed in
    pub hotkeys: HashMap<u32, String>,
    /// Flash a skill when we use it
    pub flash_on_use: bool,
    /// Skills that pulse gold when they come off cooldown
    pub ready_pulse: Vec<u32>,
    pub buff_style: BuffStyle,
    /// Which way boxes line up; bars are always a column
    pub buff_orientation: Orientation,
    /// A box's size, or a bar's height
    pub buff_size: u32,
    pub buff_bar_width: u32,
    pub buff_missing: Missing,
    pub buff_grow: Grow,
    /// Outline a tracked buff in red for its last seconds
    pub buff_warning: bool,
    /// The playing class's, as the windows see them. Kept in `class_buffs`.
    pub tracked_buffs: Vec<TrackedBuff>,
    /// Tracked buffs by class: the first two digits of its skill ids, 0 if unknown
    pub class_buffs: HashMap<u32, Vec<TrackedBuff>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            orientation: Orientation::Horizontal,
            icon_width: 50,
            icon_height: 50,
            radius: 8,
            gap: 1,
            opacity: 100,
            show: Show::Always,
            hidden_opacity: 0,
            fade_delay: 5,
            only_focused: true,
            order: Vec::new(),
            hidden: Vec::new(),
            hotkeys: HashMap::new(),
            flash_on_use: true,
            ready_pulse: Vec::new(),
            buff_style: BuffStyle::Boxes,
            buff_orientation: Orientation::Horizontal,
            buff_size: 36,
            buff_bar_width: 180,
            buff_missing: Missing::Hide,
            buff_grow: Grow::Down,
            buff_warning: true,
            tracked_buffs: Vec::new(),
            class_buffs: HashMap::new(),
        }
    }
}

impl Settings {
    /// Keep sizes to what the settings window allows, whatever the file says
    pub fn clamped(mut self) -> Self {
        self.icon_width = self.icon_width.clamp(12, 256);
        self.icon_height = self.icon_height.clamp(12, 256);
        self.radius = self.radius.min(128);
        self.gap = self.gap.min(64);
        self.opacity = self.opacity.clamp(10, 100);
        self.hidden_opacity = self.hidden_opacity.min(100);
        self.fade_delay = self.fade_delay.min(60);
        self.buff_size = self.buff_size.clamp(12, 128);
        self.buff_bar_width = self.buff_bar_width.clamp(60, 600);
        self.hotkeys.retain(|_, key| !key.trim().is_empty());
        for key in self.hotkeys.values_mut() {
            *key = key.trim().chars().take(MAX_HOTKEY).collect();
        }
        self
    }

    /// What the windows see: `tracked_buffs` is the class's
    pub fn for_class(&self, class: u32) -> Self {
        Self { tracked_buffs: self.class_buffs.get(&class).cloned().unwrap_or_default(), ..self.clone() }
    }

    /// Files `tracked_buffs`, as the settings window left them, under the class
    pub fn keep_tracked(&mut self, class: u32) {
        let tracked = std::mem::take(&mut self.tracked_buffs);
        if tracked.is_empty() {
            self.class_buffs.remove(&class);
        } else {
            self.class_buffs.insert(class, tracked);
        }
    }

    /// Before buffs were kept per class there was one list, nearly always of one
    /// class's skills
    fn migrated(mut self) -> Self {
        let tracked = std::mem::take(&mut self.tracked_buffs);
        if self.class_buffs.is_empty() && !tracked.is_empty() {
            let mut counts: HashMap<u32, usize> = HashMap::new();
            for buff in &tracked {
                *counts.entry(class_of(buff.id)).or_default() += 1;
            }
            let class = counts.into_iter().filter(|&(c, _)| c != 0).max_by_key(|&(_, n)| n).map_or(0, |(c, _)| c);
            self.class_buffs.insert(class, tracked);
        }
        self
    }

    pub fn load(dir: &Path) -> Self {
        std::fs::read(dir.join(FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .unwrap_or_default()
            .migrated()
            .clamped()
    }

    pub fn save(&self, dir: &Path) {
        let file = Self { tracked_buffs: Vec::new(), ..self.clone() };
        if let Ok(json) = serde_json::to_vec_pretty(&file) {
            let _ = std::fs::write(dir.join(FILE), json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let s: Settings =
            serde_json::from_str(r#"{"orientation":"vertical","iconWidth":50,"show":"dungeonCombat","hotkeys":{"15210000":"S1"}}"#).unwrap();
        assert_eq!(s.orientation, Orientation::Vertical);
        assert_eq!(s.show, Show::DungeonCombat);
        assert_eq!(s.fade_delay, Settings::default().fade_delay);
        assert_eq!(s.icon_width, 50);
        assert_eq!(s.icon_height, Settings::default().icon_height);
        assert_eq!(s.hotkeys[&15_210_000], "S1");
    }

    #[test]
    fn clamps_what_the_window_wouldnt_allow() {
        let mut s = Settings { icon_width: 1, icon_height: 9_000, ..Default::default() };
        s.hotkeys.insert(1, "  ".into());
        s.hotkeys.insert(2, "Ctrl+Shift+Alt+F12".into());
        let s = s.clamped();
        assert_eq!((s.icon_width, s.icon_height), (12, 256));
        assert_eq!(s.hotkeys.len(), 1);
        assert_eq!(s.hotkeys[&2], "Ctrl+Shi");
    }

    #[test]
    fn tracked_buffs_are_kept_per_class() {
        let buff = |id| TrackedBuff { id, name: String::new(), icon: String::new() };
        // one list from before, of a Sorcerer's buffs and a Chanter's aura
        let old = Settings { tracked_buffs: vec![buff(15_310_000), buff(18_160_000), buff(15_400_000)], ..Default::default() };
        let mut s = old.migrated();
        assert!(s.tracked_buffs.is_empty());
        assert_eq!(s.for_class(15).tracked_buffs.len(), 3);
        assert!(s.for_class(18).tracked_buffs.is_empty());
        s.tracked_buffs = vec![buff(18_120_000)];
        s.keep_tracked(18);
        assert_eq!(s.for_class(18).tracked_buffs, [buff(18_120_000)]);
        assert_eq!(s.for_class(15).tracked_buffs.len(), 3);
        s.keep_tracked(15);
        assert!(s.for_class(15).tracked_buffs.is_empty() && !s.class_buffs.contains_key(&15));
    }
}
