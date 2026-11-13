//! Сборки модов: файл сборки, разбор списков id, сравнение, пакет для игроков (zip).
use crate::config::Profile;
use crate::mods::{Mod, Source};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FORMAT: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PackMod {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub workshop_id: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
}

/// Как подключиться к серверу сборки
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PackServer {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: String,
    #[serde(default)]
    pub udp_port: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PackFile {
    pub format: u32,
    pub name: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub created: String,
    pub mods: Vec<PackMod>,
    /// постоянный id сборки и номер ревизии: по ним программа получателя понимает, что файл обновился
    #[serde(default)]
    pub pack_id: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub server: Option<PackServer>,
}

/// Первая папка под корнем модов: `<root>/MyMod/42/mod.info` → `MyMod`
pub fn top_folder(m: &Mod, roots: &[PathBuf]) -> Option<String> {
    roots.iter().find_map(|r| m.path.strip_prefix(r).ok()).and_then(|rel| rel.components().next()).map(|c| c.as_os_str().to_string_lossy().into_owned())
}

pub fn to_file(p: &Profile, installed: &HashMap<String, Mod>, ws_of: &dyn Fn(&Mod) -> Option<String>, roots: &[PathBuf], server: Option<PackServer>) -> PackFile {
    PackFile {
        format: FORMAT,
        pack_id: p.pack_id.clone(),
        revision: p.revision,
        server,
        name: p.name.clone(),
        notes: p.notes.clone(),
        created: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        mods: p
            .enabled
            .iter()
            .map(|id| match installed.get(id) {
                Some(m) => PackMod { id: id.clone(), name: m.name.clone(), workshop_id: ws_of(m), folder: top_folder(m, roots) },
                None => PackMod { id: id.clone(), name: String::new(), workshop_id: None, folder: None },
            })
            .collect(),
    }
}

pub fn to_json(f: &PackFile) -> String {
    serde_json::to_string_pretty(f).unwrap_or_default()
}

pub fn from_json(s: &str) -> Result<PackFile> {
    let f: PackFile = serde_json::from_str(s.trim_start_matches('\u{feff}')).context(crate::i18n::t("файл сборки не разобран"))?;
    if f.format > FORMAT {
        bail!("{}", tf!("файл сборки новее программы (формат {})", f.format));
    }
    Ok(f)
}

#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub mods: Vec<String>,
    pub workshop: Vec<String>,
}

/// Разбор вставленного текста: строка `Mods=\a;\b;`, строка `WorkshopItems=1;2`, ссылки на воркшоп,
/// либо просто список через перенос строки, запятую, `;` или пробел. Голые числа — id воркшопа.
pub fn parse_id_list(text: &str) -> Parsed {
    let mut out = Parsed::default();
    for line in text.lines() {
        let line = line.trim();
        let (kind, body) = if let Some(b) = line.strip_prefix("Mods=") {
            (1, b)
        } else if let Some(b) = line.strip_prefix("WorkshopItems=") {
            (2, b)
        } else {
            (0, line)
        };
        // id модов бывают с пробелами («True Damage»), поэтому без ; и , строка — один токен
        let toks: Vec<&str> = if kind != 0 || body.contains(';') || body.contains(',') { body.split([';', ',']).collect() } else { vec![body] };
        for tok in toks {
            let tok = tok.trim();
            if tok.is_empty() {
                continue;
            }
            let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
            let ws = if tok.contains("id=") { crate::tracking::id_from_url(tok) } else if kind == 2 || (kind == 0 && digits(tok)) { Some(tok.trim_start_matches('\\').to_string()).filter(|s| digits(s)) } else { None };
            if let Some(w) = ws {
                if !out.workshop.contains(&w) {
                    out.workshop.push(w);
                }
            } else {
                let id = tok.strip_prefix('\\').unwrap_or(tok).to_string();
                if !id.is_empty() && !out.mods.contains(&id) {
                    out.mods.push(id);
                }
            }
        }
    }
    out
}

#[derive(Debug, Default, PartialEq)]
pub struct Diff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// есть в обеих, но стоят в другом относительном порядке
    pub moved: Vec<String>,
}

pub fn diff(old: &[String], new: &[String]) -> Diff {
    let (so, sn): (HashSet<&String>, HashSet<&String>) = (old.iter().collect(), new.iter().collect());
    let added = new.iter().filter(|x| !so.contains(x)).cloned().collect();
    let removed = old.iter().filter(|x| !sn.contains(x)).cloned().collect();
    let common_old: Vec<&String> = old.iter().filter(|x| sn.contains(x)).collect();
    let common_new: Vec<&String> = new.iter().filter(|x| so.contains(x)).collect();
    let pos: HashMap<&String, usize> = common_new.iter().enumerate().map(|(i, x)| (*x, i)).collect();
    let seq: Vec<usize> = common_old.iter().map(|x| pos[*x]).collect();
    // наибольшая возрастающая подпоследовательность: всё вне неё — «сдвинуто»
    let n = seq.len();
    let (mut best, mut prev) = (vec![1usize; n], vec![usize::MAX; n]);
    for i in 0..n {
        for j in 0..i {
            if seq[j] < seq[i] && best[j] + 1 > best[i] {
                best[i] = best[j] + 1;
                prev[i] = j;
            }
        }
    }
    let mut keep = HashSet::new();
    if let Some((mut i, _)) = best.iter().enumerate().max_by_key(|(_, b)| **b) {
        loop {
            keep.insert(i);
            if prev[i] == usize::MAX {
                break;
            }
            i = prev[i];
        }
    }
    let moved = common_old.iter().enumerate().filter(|(i, _)| !keep.contains(i)).map(|(_, x)| (*x).clone()).collect();
    Diff { added, removed, moved }
}

pub fn mods_line(order: &[String], backslash: bool) -> String {
    let pre = if backslash { "\\" } else { "" };
    order.iter().map(|i| format!("{pre}{i}")).collect::<Vec<_>>().join(";")
}

#[derive(Debug, Default)]
pub struct ZipReport {
    pub folders: usize,
    pub files: usize,
    pub bytes: u64,
    /// моды из воркшопа Steam: их игрокам надо подписаться, в zip их нет
    pub subscribe: Vec<String>,
    pub missing: Vec<String>,
}

fn install_text(en: bool, name: &str, report_subscribe: &[String]) -> String {
    let mut s = if en {
        format!(
            "Mod pack: {name}\n\nHOW TO INSTALL\n1. Close Project Zomboid.\n2. Copy the `mods` folder from this archive into your Zomboid user folder\n   (Windows: C:\\Users\\<you>\\Zomboid, Linux: ~/Zomboid), merging with the existing `mods` folder.\n3. Start the game and connect to the server. The server's mod list is the one in modlist.txt.\n\nIf the game says a mod is missing or mismatched, delete the old copy of that mod folder and copy it again.\n"
        )
    } else {
        format!(
            "Сборка модов: {name}\n\nКАК УСТАНОВИТЬ\n1. Закрой Project Zomboid.\n2. Скопируй папку `mods` из этого архива в пользовательскую папку Zomboid\n   (Windows: C:\\Users\\<ты>\\Zomboid, Linux: ~/Zomboid), слив с уже существующей папкой `mods`.\n3. Запусти игру и подключись к серверу. Список модов сервера лежит в modlist.txt.\n\nЕсли игра ругается, что мода нет или он не совпадает, удали старую копию папки этого мода и скопируй заново.\n"
        )
    };
    if !report_subscribe.is_empty() {
        s.push_str(if en { "\nWORKSHOP MODS (subscribe in Steam, they are not in the archive):\n" } else { "\nМОДЫ ИЗ ВОРКШОПА (подпишись в Steam, в архиве их нет):\n" });
        for w in report_subscribe {
            s.push_str(&format!("  https://steamcommunity.com/sharedfiles/filedetails/?id={w}\n"));
        }
    }
    s
}

/// Инструкция для ИИ-агента внутри файла сборки: что это и какими командами ставится
fn agent_text(name: &str) -> String {
    format!(
        "# {name}: Project Zomboid mod pack (.pzpack)\n\n\
This file is a zip archive made by PZ Mod Manager. If a person handed it to you and said \"sort this out\", the goal is: \
install this mod pack on their Project Zomboid game client, with the same mod order, so they can join the server.\n\n\
## Contents\n\n\
- `pack.json`: the pack. `mods` is the load order; each mod has `id`, `workshop_id` (Steam workshop item, null for a local mod) and `folder`. \
`server` is where to connect (`host`, `port`). `pack_id` + `revision` identify the pack and its version.\n\
- `mods/<folder>/`: local mods that are not on the Steam workshop.\n\
- `modlist.txt`: the `Mods=` line of the server.\n\
- `INSTALL.txt`: manual steps for a person.\n\n\
## Install with PZ Mod Manager (preferred)\n\n\
The manager keeps the pack linked to this file, so later revisions are applied as updates. \
Get it from https://github.com/Dark-Ohm/pz-mod-manager : the Releases page has a ready `pz-mod-manager-windows-x64.exe` (and a Linux binary), nothing to install. \
To build from source instead: clone the repository and run `scripts\\install.bat` (Windows) or `scripts/install.sh` (Linux); `AGENTS.md` there is the full guide. \
Then, with the path to this file:\n\n\
```\n\
pz-mod-manager pack info <file.pzpack>      # what is in the pack, what is missing on this machine\n\
pz-mod-manager pack install <file.pzpack>   # unpack local mods, download workshop items, copy the mod order\n\
pz-mod-manager pack status                  # later: is a newer revision waiting\n\
```\n\n\
Every command prints one JSON object; `pack install` reports `complete: true` when all mods are on disk, and `server.connect` is the address to join. \
The same commands exist as MCP tools `pack_info`, `pack_install`, `pack_status` (`pz-mod-manager mcp`). \
A person can instead double-click the file or use Packs → \"Open .pzpack…\" in the program window.\n\n\
When the sender changes the pack they send a new file (or the same file updates in a shared folder): run `pack install` on it again. \
It recognises the pack by `pack_id` and applies the new revision.\n\n\
## Without the manager\n\n\
1. Close the game.\n\
2. Copy every folder from `mods/` in this archive into the game's user mods folder (Windows: `C:\\Users\\<user>\\Zomboid\\mods`, Linux: `~/Zomboid/mods`).\n\
3. For each mod with a `workshop_id`, subscribe in Steam: `https://steamcommunity.com/sharedfiles/filedetails/?id=<workshop_id>`.\n\
4. Start the game and join `server.host`:`server.port`. The server sets the mod order for multiplayer.\n\n\
Do not edit files in the person's `Zomboid` folder beyond `mods/` unless they ask.\n"
    )
}

/// Пакет для игроков: mods/<папки локальных модов> + modlist.txt + INSTALL.txt + pack.json
pub fn build_player_zip(dest: &Path, p: &Profile, installed: &HashMap<String, Mod>, roots: &[PathBuf], ws_of: &dyn Fn(&Mod) -> Option<String>, en: bool, server: Option<PackServer>) -> Result<ZipReport> {
    use zip::write::SimpleFileOptions;
    let mut rep = ZipReport::default();
    let mut folders: Vec<(String, PathBuf)> = vec![];
    for id in &p.enabled {
        match installed.get(id) {
            None => rep.missing.push(id.clone()),
            Some(m) if matches!(m.source, Source::ServerLocal | Source::ClientLocal) => {
                if let Some(f) = top_folder(m, roots) {
                    if !folders.iter().any(|(n, _)| *n == f) {
                        let root = roots.iter().find(|r| m.path.starts_with(r)).cloned().unwrap_or_default();
                        folders.push((f.clone(), root.join(&f)));
                    }
                }
            }
            Some(m) => {
                if let Some(w) = ws_of(m) {
                    if !rep.subscribe.contains(&w) {
                        rep.subscribe.push(w);
                    }
                }
            }
        }
    }
    let file = std::fs::File::create(dest).with_context(|| tf!("не удалось создать {}", dest.display()))?;
    let mut zw = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    for (name, dir) in &folders {
        for e in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            let rel = e.path().strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
            let zname = if rel.is_empty() { format!("mods/{name}/") } else { format!("mods/{name}/{rel}") };
            if e.file_type().is_dir() {
                zw.add_directory(zname, opts)?;
            } else {
                zw.start_file(zname, opts)?;
                let mut f = std::fs::File::open(e.path())?;
                rep.bytes += std::io::copy(&mut f, &mut zw)?;
                rep.files += 1;
            }
        }
        rep.folders += 1;
    }
    zw.start_file("modlist.txt", opts)?;
    zw.write_all(format!("Mods={}\r\n", mods_line(&p.enabled, true)).as_bytes())?;
    zw.start_file("INSTALL.txt", opts)?;
    zw.write_all(install_text(en, &p.name, &rep.subscribe).replace('\n', "\r\n").as_bytes())?;
    zw.start_file("AGENT.md", opts)?;
    zw.write_all(agent_text(&p.name).as_bytes())?;
    zw.start_file("pack.json", opts)?;
    zw.write_all(to_json(&to_file(p, installed, ws_of, roots, server)).as_bytes())?;
    zw.finish()?;
    Ok(rep)
}
