//! `--demo`: a made-up Chanter casting on its own, to see the overlay without the game.

use std::thread;
use std::time::Duration;

use crate::overlay::{epoch_ms, Overlay};
use crate::skills::{Loadout, Skill};
use crate::tracker::Update;

const ICONS: &str = "https://assets.playnccdn.com/static-aion2-gamedata/resources";
const SKILLS: [(u32, &str, &str); 11] = [
    (18_010_000, "Onslaught", "ICON_CH_SKILL_001"),
    (18_040_000, "Incandescent Blow", "ICON_CH_SKILL_004"),
    (18_090_000, "Rushing Smash", "ICON_CH_SKILL_009"),
    (18_060_000, "Impactful Crush", "ICON_CH_SKILL_006"),
    (18_100_000, "Dark Crush", "ICON_CH_SKILL_010"),
    (18_300_000, "Gust Rampage", "ICON_CH_SKILL_030"),
    (18_150_000, "Heat Wave Blow", "ICON_CH_SKILL_015"),
    (18_120_000, "Recuperation", "ICON_CH_SKILL_013"),
    (18_210_000, "Tremor Crush", "ICON_CH_SKILL_021"),
    (18_080_000, "Wave Blow", "ICON_CH_SKILL_008"),
    (18_290_000, "Spinning Strike", "ICON_CH_SKILL_029"),
];

pub fn run(overlay: Overlay) {
    let bar: Vec<Skill> = SKILLS
        .iter()
        .map(|&(id, name, icon)| Skill { id, name: name.into(), icon: format!("{ICONS}/{icon}.png") })
        .collect();
    overlay.set_loadout(Loadout { name: "Demo".into(), server: 0, bar: bar.clone(), known: bar });

    let mut seed = epoch_ms() as u32 | 1;
    let mut random = move |n: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed % n
    };
    loop {
        thread::sleep(Duration::from_millis(700 + random(900) as u64));
        let view = overlay.view();
        let now = epoch_ms();
        let ready: Vec<u32> = view.skills.iter().map(|s| s.id).filter(|id| view.cooldowns.get(id).map_or(true, |c| c.ends_at <= now)).collect();
        if ready.is_empty() {
            continue;
        }
        let skill = ready[random(ready.len() as u32) as usize];
        let duration = 3_000 + random(22) * 1_000;
        overlay.apply(Update::Cooldown { skill, ends_at: now + duration as u64, duration });
    }
}
