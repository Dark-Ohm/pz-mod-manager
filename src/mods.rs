use crate::i18n::t;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    ServerLocal,
    ClientLocal,
    ClientWorkshop,
    ServerWorkshop,
    Cache,
}

impl Source {
    pub fn label(self) -> String {
        match self {
            Source::ServerLocal => t("сервер (локально)"),
            Source::ClientLocal => t("клиент (локально)"),
            Source::ClientWorkshop => t("клиент (Steam)"),
            Source::ServerWorkshop => t("сервер"),
            Source::Cache => t("загружено"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Mod {
    pub id: String,
    pub name: String,
    pub description: String,
    pub requires: Vec<String>,
    pub workshop_id: Option<String>,
    /// `url=` из mod.info (часто ссылка на воркшоп)
    pub url: Option<String>,
    pub path: PathBuf,
    pub source: Source,
}

fn parse_mod_info(path: &Path, source: Source) -> Option<Mod> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim_start_matches('\u{feff}');
    let (mut id, mut name, mut desc, mut req, mut url) = (None, None, String::new(), vec![], None);
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        match k.trim().to_lowercase().as_str() {
            "id" => id = Some(v.to_string()),
            "name" => name = Some(v.to_string()),
            "description" => {
                if !desc.is_empty() {
                    desc.push(' ');
                }
                desc.push_str(v)
            }
            "url" if !v.is_empty() => url = Some(v.to_string()),
            "require" => {
                req = v
                    .split(',')
                    .map(|s| s.trim().trim_start_matches('\\').to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            _ => {}
        }
    }
    let id = id.filter(|s| !s.is_empty())?;
    // .../108600/<workshop_id>/...
    let comps: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let workshop_id = comps
        .windows(2)
        .find(|w| w[0] == "108600" && !w[1].is_empty() && w[1].chars().all(|c| c.is_ascii_digit()))
        .map(|w| w[1].clone());
    Some(Mod {
        name: name.unwrap_or_else(|| id.clone()),
        id,
        description: desc,
        requires: req,
        workshop_id,
        url,
        path: path.parent()?.to_path_buf(),
        source,
    })
}

pub fn scan(root: &Path, source: Source) -> Vec<Mod> {
    let mut seen = HashSet::new();
    let mut out = vec![];
    if !root.exists() {
        return out;
    }
    for e in WalkDir::new(root).max_depth(6).into_iter().filter_map(|e| e.ok()) {
        if e.file_name() == "mod.info" {
            if let Some(m) = parse_mod_info(e.path(), source) {
                if seen.insert((m.id.clone(), m.workshop_id.clone())) {
                    out.push(m);
                }
            }
        }
    }
    out
}

/// Все моды, видимые окружению: локальные сервера и клиента, воркшоп клиента и сервера
pub struct EnvScan {
    pub all: Vec<Mod>,
    /// первый найденный мод с каждым id
    pub installed: HashMap<String, Mod>,
    /// id, под которыми лежат разные моды (разные workshop id)
    pub dups: HashSet<String>,
}

pub fn scan_env(s: &crate::config::Settings, srv: &crate::config::ServerCfg) -> EnvScan {
    let mut all = scan(&srv.mods_dir(), Source::ServerLocal);
    all.extend(scan(&srv.client_mods_dir(s), Source::ClientLocal));
    all.extend(scan(&s.client_workshop_dir, Source::ClientWorkshop));
    all.extend(scan(&srv.workshop_dir, Source::ServerWorkshop));
    let mut keys: HashMap<String, HashSet<Option<String>>> = HashMap::new();
    let mut installed = HashMap::new();
    for m in &all {
        keys.entry(m.id.clone()).or_default().insert(m.workshop_id.clone());
        installed.entry(m.id.clone()).or_insert_with(|| m.clone());
    }
    let dups = keys.into_iter().filter(|(_, v)| v.len() > 1).map(|(k, _)| k).collect();
    EnvScan { all, installed, dups }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warn,
}

#[derive(Clone)]
pub struct Issue {
    pub sev: Severity,
    pub msg: String,
}

/// Проверка порядка и зависимостей; результат по id мода
pub fn check(order: &[String], installed: &HashMap<String, Mod>, dups: &HashSet<String>) -> HashMap<String, Vec<Issue>> {
    let pos: HashMap<&str, usize> = order.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
    let mut out: HashMap<String, Vec<Issue>> = HashMap::new();
    let mut add = |id: &str, sev, msg: String| out.entry(id.to_string()).or_default().push(Issue { sev, msg });
    for (i, id) in order.iter().enumerate() {
        let Some(m) = installed.get(id) else {
            add(id, Severity::Error, t("мод не найден на диске").into());
            continue;
        };
        if dups.contains(id) {
            add(id, Severity::Warn, t("несколько разных модов с одним id").into());
        }
        for r in &m.requires {
            if !installed.contains_key(r) {
                add(id, Severity::Error, tf!("не установлена зависимость «{}»", r));
            } else if let Some(&p) = pos.get(r.as_str()) {
                if p > i {
                    add(id, Severity::Warn, tf!("«{}» должен стоять выше в списке", r));
                }
            } else {
                add(id, Severity::Error, tf!("зависимость «{}» выключена", r));
            }
        }
    }
    out
}
