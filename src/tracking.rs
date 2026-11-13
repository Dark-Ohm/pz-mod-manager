//! Слежение за обновлениями ЛОКАЛЬНЫХ модов: привязка к элементу воркшопа, «моя версия» (baseline),
//! статусы. Установка обновления — только по кнопке. Хранится отдельно для каждого сервера.
use crate::i18n::t;
use crate::mods::Mod;
use crate::workshop_api::Remote;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Item {
    /// time_updated той версии, что стоит у нас (None — ещё не зафиксирована)
    pub baseline: Option<u64>,
    /// не проверять и не предлагать обновления
    pub frozen: bool,
    /// обновления до этой метки времени включительно пропущены
    pub skipped_until: Option<u64>,
    pub title: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ServerTrack {
    /// workshop id -> состояние
    pub items: BTreeMap<String, Item>,
    /// id мода (из mod.info) -> workshop id, заданные вручную/импортом
    pub links: BTreeMap<String, String>,
    /// моды, за которыми не следим (свои патчи и т. п.)
    pub ignored: std::collections::BTreeSet<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Tracking {
    /// ключ — папка данных сервера
    pub servers: BTreeMap<String, ServerTrack>,
}

impl Tracking {
    fn path() -> PathBuf {
        crate::config::config_root().join("tracking.json")
    }
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        let p = Self::path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = crate::atomic::write(&p, t.as_bytes());
        }
    }
    pub fn server(&mut self, key: &str) -> &mut ServerTrack {
        self.servers.entry(key.to_string()).or_default()
    }
}

/// id воркшопа из ссылки (`...?id=123&...`) или из голого числа
pub fn id_from_url(s: &str) -> Option<String> {
    let s = s.trim();
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
        return Some(s.to_string());
    }
    let p = s.find("id=")?;
    let id: String = s[p + 3..].chars().take_while(|c| c.is_ascii_digit()).collect();
    (!id.is_empty()).then_some(id)
}

/// К какому элементу воркшопа относится мод: ручная привязка → путь в Steam → `url=` в mod.info → найденное при скане
pub fn workshop_of(m: &Mod, st: &ServerTrack, scan_links: &HashMap<String, String>) -> Option<String> {
    st.links
        .get(&m.id)
        .cloned()
        .or_else(|| m.workshop_id.clone())
        .or_else(|| m.url.as_deref().filter(|u| u.contains("steamcommunity.com")).and_then(id_from_url))
        .or_else(|| scan_links.get(&m.id).cloned())
}

/// Связи из JSON-манифеста: список `{ "workshop_id": "...", "mods": [ { "id": "..." } ] }`
pub fn import_manifest(json: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json.trim_start_matches('\u{feff}')) else { return vec![] };
    let mut out = vec![];
    for item in v.as_array().cloned().unwrap_or_default() {
        let Some(ws) = item["workshop_id"].as_str().map(str::to_string).or_else(|| item["workshop_id"].as_u64().map(|n| n.to_string())) else { continue };
        for m in item["mods"].as_array().cloned().unwrap_or_default() {
            if let Some(id) = m["id"].as_str().filter(|s| !s.is_empty()) {
                out.push((id.to_string(), ws.clone()));
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// нет привязки к воркшопу
    NoLink,
    /// привязка есть, но «моя версия» не зафиксирована
    NoBaseline,
    /// ещё не проверяли
    Unknown,
    UpToDate,
    /// есть обновление (время обновления в Steam)
    Update(u64),
    /// обновление есть, но пропущено
    Skipped(u64),
    Frozen,
    /// элемент удалён или скрыт автором
    Gone,
}

pub fn status(item: Option<&Item>, remote: Option<&Remote>) -> Status {
    if item.is_some_and(|i| i.frozen) {
        return Status::Frozen;
    }
    let Some(r) = remote else { return if item.and_then(|i| i.baseline).is_none() { Status::NoBaseline } else { Status::Unknown } };
    if !r.ok {
        return Status::Gone;
    }
    let Some(base) = item.and_then(|i| i.baseline) else { return Status::NoBaseline };
    if r.time_updated <= base {
        return Status::UpToDate;
    }
    if item.and_then(|i| i.skipped_until).is_some_and(|s| r.time_updated <= s) {
        return Status::Skipped(r.time_updated);
    }
    Status::Update(r.time_updated)
}

/// Моды по элементам воркшопа и список без привязки: (workshop id -> id модов, id модов без привязки)
pub fn group(mods: &[&Mod], resolve: impl Fn(&Mod) -> Option<String>) -> (BTreeMap<String, Vec<String>>, Vec<String>) {
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut none = vec![];
    for m in mods {
        match resolve(m) {
            Some(ws) => by.entry(ws).or_default().push(m.id.clone()),
            None => none.push(m.id.clone()),
        }
    }
    (by, none)
}

pub fn fmt_ts(ts: u64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0).map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| "?".into())
}
