//! The character's skill bar, from NC's public character API (the same one the
//! checklist site's armory uses). Packets give us the name; this gives us icons.

use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SITE: &str = "https://aion2.plaync.com";
const SEARCH: &str = "https://api-search.plaync.com/aion2global/search/v2/character";
const REGIONS: [&str; 5] = ["nae", "naw", "eu", "la", "as"];
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const CACHE_FILE: &str = "loadout.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Skill {
    pub id: u32,
    pub name: String,
    pub icon: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Loadout {
    pub name: String,
    pub server: u16,
    /// Skills slotted on the in-game bar
    pub bar: Vec<Skill>,
    /// Every active skill of the class, for casts of something not on the bar
    pub known: Vec<Skill>,
}

#[derive(Deserialize)]
struct Servers {
    #[serde(rename = "serverList", default)]
    list: Vec<ServerRow>,
}

#[derive(Deserialize)]
struct ServerRow {
    #[serde(rename = "serverId")]
    id: u16,
}

#[derive(Deserialize)]
struct Search {
    #[serde(default)]
    list: Vec<SearchRow>,
}

#[derive(Deserialize)]
struct SearchRow {
    #[serde(rename = "characterId")]
    id: String,
    name: String,
    #[serde(rename = "serverId")]
    server: u16,
}

#[derive(Deserialize)]
struct Equipment {
    skill: Option<SkillBlock>,
}

#[derive(Deserialize)]
struct SkillBlock {
    #[serde(rename = "skillList", default)]
    list: Vec<SkillRow>,
}

#[derive(Deserialize)]
struct SkillRow {
    id: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    icon: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    acquired: Value,
    #[serde(default)]
    equip: Value,
}

pub fn fetch(name: &str, server: u16) -> Result<Loadout, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).user_agent(USER_AGENT).build();
    let server_id = server.to_string();
    let region = region_of(&agent, server)?;

    let search: Search = get(
        &agent,
        SEARCH,
        &[("keyword", name), ("region", region), ("serverId", &server_id), ("localeInfo", "en-US"), ("page", "1"), ("size", "30")],
    )?;
    let row = search
        .list
        .into_iter()
        .find(|r| r.server == server && strip_tags(&r.name).eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("{name} isn't on NC's character search"))?;
    let character_id = percent_decode(&row.id);

    let equipment: Equipment = get(
        &agent,
        &format!("{SITE}/api/character/equipment"),
        &[("lang", "en-US"), ("region", region), ("serverId", &server_id), ("characterId", &character_id)],
    )?;
    let rows = equipment.skill.map(|s| s.list).unwrap_or_default();
    let skill = |r: &SkillRow| Skill { id: r.id, name: r.name.clone(), icon: r.icon.clone() };
    let active = |r: &&SkillRow| matches!(r.category.as_str(), "Active" | "Dp");
    Ok(Loadout {
        name: name.to_string(),
        server,
        bar: rows.iter().filter(active).filter(|r| truthy(&r.acquired) && truthy(&r.equip)).map(skill).collect(),
        known: rows.iter().filter(active).map(skill).collect(),
    })
}

fn region_of(agent: &ureq::Agent, server: u16) -> Result<&'static str, String> {
    for region in REGIONS {
        let servers: Servers = get(agent, &format!("{SITE}/en-us/api/gameinfo/servers"), &[("lang", "en-US"), ("region", region)])?;
        if servers.list.iter().any(|s| s.id == server) {
            return Ok(region);
        }
    }
    Err(format!("server {server} isn't on NC's Global list"))
}

fn get<T: DeserializeOwned>(agent: &ureq::Agent, url: &str, query: &[(&str, &str)]) -> Result<T, String> {
    let mut request = agent.get(url).set("accept", "application/json");
    for (key, value) in query {
        request = request.query(key, value);
    }
    request.call().map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())
}

fn truthy(v: &Value) -> bool {
    v.as_bool().unwrap_or(false) || v.as_i64().is_some_and(|n| n != 0)
}

/// Search wraps the matched part of a name in <strong>
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Search hands back character ids already URL-encoded
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(hi), Some(lo)) = (digit(b[i + 1]), digit(b[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn load_cached(dir: &Path) -> Option<Loadout> {
    serde_json::from_slice(&std::fs::read(dir.join(CACHE_FILE)).ok()?).ok()
}

pub fn save_cached(dir: &Path, loadout: &Loadout) {
    if let Ok(json) = serde_json::to_vec_pretty(loadout) {
        let _ = std::fs::write(dir.join(CACHE_FILE), json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_search_results() {
        assert_eq!(strip_tags("<strong>Luna</strong>"), "Luna");
        assert_eq!(percent_decode("wT1m8Q40KLB9_cR8%3D"), "wT1m8Q40KLB9_cR8=");
        assert_eq!(percent_decode("abc%3"), "abc%3");
    }

    /// Hits NC's live API: `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn fetches_a_real_skill_bar() {
        let loadout = fetch("Luna", 2109).unwrap();
        assert!(!loadout.bar.is_empty());
        assert!(loadout.bar.iter().all(|s| s.id % 10_000 == 0 && s.icon.starts_with("https://")));
        assert!(loadout.known.len() >= loadout.bar.len());
    }

    #[test]
    fn reads_nc_flags() {
        assert!(truthy(&Value::from(1)));
        assert!(truthy(&Value::from(true)));
        assert!(!truthy(&Value::from(0)));
        assert!(!truthy(&Value::Null));
    }
}
