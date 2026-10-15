//! Turns decoded packets into what the overlay shows: who we are, and when each
//! of our skills comes back.

use std::collections::{HashMap, VecDeque};

use crate::packets::{base_skill, Event};

/// How close a cast and a cooldown update must be to count as the same skill use
const MATCH_MS: u64 = 1_500;
const VOTES_NEEDED: u32 = 2;

#[derive(Debug, PartialEq)]
pub enum Update {
    Character { name: String, server: u16 },
    /// Worked out which entity is ours without the character packet
    SelfFound(u32),
    /// One of our skills went on cooldown
    Cooldown { skill: u32, ends_at: u64, duration: u32 },
    /// The server moved a running cooldown (a reset or a reduction)
    Remaining { skill: u32, ends_at: u64, left: u32 },
}

/// The character packet only comes with a loading screen. Started mid-session,
/// we find ourselves instead by matching casts to cooldown updates, which are
/// only ever about our own skills.
#[derive(Default)]
pub struct Tracker {
    self_id: Option<u32>,
    character: Option<(String, u16)>,
    casts: VecDeque<(u64, u32, u32)>,
    updates: VecDeque<(u64, u32)>,
    votes: HashMap<u32, u32>,
}

impl Tracker {
    pub fn handle(&mut self, event: Event, now: u64) -> Vec<Update> {
        let mut out = Vec::new();
        match event {
            Event::SelfInfo { entity, name, server } => {
                self.self_id = Some(entity);
                self.forget();
                let character = Some((name.clone(), server));
                if !name.is_empty() && self.character != character {
                    self.character = character;
                    out.push(Update::Character { name, server });
                }
            }
            Event::Cast { actor, skill, cooldown_ms, .. } => {
                let skill = base_skill(skill);
                if self.self_id.is_none() {
                    self.guess(now, Some((actor, skill)), None, &mut out);
                }
                if self.self_id == Some(actor) && cooldown_ms > 0 {
                    out.push(Update::Cooldown { skill, ends_at: now + cooldown_ms as u64, duration: cooldown_ms });
                }
            }
            Event::Cooldowns(list) => {
                for (skill, left) in list {
                    let skill = base_skill(skill);
                    if self.self_id.is_none() {
                        self.guess(now, None, Some(skill), &mut out);
                    }
                    out.push(Update::Remaining { skill, ends_at: now + left as u64, left });
                }
            }
        }
        out
    }

    fn guess(&mut self, now: u64, cast: Option<(u32, u32)>, update: Option<u32>, out: &mut Vec<Update>) {
        self.casts.retain(|&(at, ..)| now.saturating_sub(at) <= MATCH_MS);
        self.updates.retain(|&(at, _)| now.saturating_sub(at) <= MATCH_MS);
        let mut matched = Vec::new();
        if let Some((actor, skill)) = cast {
            if self.updates.iter().any(|&(_, s)| s == skill) {
                matched.push(actor);
            }
            self.casts.push_back((now, actor, skill));
        }
        if let Some(skill) = update {
            matched.extend(self.casts.iter().filter(|&&(_, _, s)| s == skill).map(|&(_, actor, _)| actor));
            self.updates.push_back((now, skill));
        }
        for actor in matched {
            let votes = self.votes.entry(actor).or_default();
            *votes += 1;
            if *votes >= VOTES_NEEDED {
                self.self_id = Some(actor);
                self.forget();
                out.push(Update::SelfFound(actor));
                return;
            }
        }
    }

    fn forget(&mut self) {
        self.casts.clear();
        self.updates.clear();
        self.votes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cast(actor: u32, skill: u32, cooldown_ms: u32) -> Event {
        Event::Cast { actor, target: 99, skill, cooldown_ms }
    }

    #[test]
    fn our_casts_go_on_cooldown_and_others_dont() {
        let mut t = Tracker::default();
        t.handle(Event::SelfInfo { entity: 7, name: "Nimara".into(), server: 1304 }, 0);
        assert_eq!(
            t.handle(cast(7, 18_010_140, 8_000), 1_000),
            [Update::Cooldown { skill: 18_010_000, ends_at: 9_000, duration: 8_000 }]
        );
        assert!(t.handle(cast(8, 18_010_140, 8_000), 1_000).is_empty());
        assert!(t.handle(cast(7, 10_000, 0), 1_000).is_empty());
    }

    #[test]
    fn character_is_announced_once() {
        let mut t = Tracker::default();
        let info = Event::SelfInfo { entity: 7, name: "Nimara".into(), server: 1304 };
        assert_eq!(t.handle(info.clone(), 0), [Update::Character { name: "Nimara".into(), server: 1304 }]);
        assert!(t.handle(info, 0).is_empty());
        assert!(t.handle(Event::SelfInfo { entity: 9, name: String::new(), server: 0 }, 0).is_empty());
    }

    #[test]
    fn finds_itself_from_cooldown_updates() {
        let mut t = Tracker::default();
        t.handle(cast(5, 18_010_140, 8_000), 0);
        t.handle(cast(7, 18_010_140, 8_000), 10);
        t.handle(Event::Cooldowns(vec![(18_010_140, 7_990)]), 20);
        t.handle(cast(7, 18_040_000, 5_000), 3_000);
        let out = t.handle(Event::Cooldowns(vec![(18_040_000, 5_000)]), 3_010);
        assert!(out.contains(&Update::SelfFound(7)));
        assert_eq!(
            t.handle(cast(7, 18_090_000, 4_000), 4_000),
            [Update::Cooldown { skill: 18_090_000, ends_at: 8_000, duration: 4_000 }]
        );
    }
}
