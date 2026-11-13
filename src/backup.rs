//! Бэкапы мира и конфигов сервера + безопасное восстановление.
//! Идея модели безопасности: Zomboid Control Panel (MIT) — снимок текущего состояния перед
//! восстановлением, подмена через `.new`/`.old`, отказ при запущенном сервере.
use crate::i18n::t;
use crate::config::ServerCfg;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn root(cfg: &ServerCfg) -> PathBuf {
    cfg.cache_dir.join("backups").join("mm")
}

fn world_dir(cfg: &ServerCfg) -> PathBuf {
    cfg.cache_dir.join("Saves/Multiplayer").join(&cfg.name)
}

/// Конфиги сервера: <имя>.ini, <имя>_SandboxVars.lua, _spawnpoints.lua, _spawnregions.lua
fn config_files(cfg: &ServerCfg) -> Vec<PathBuf> {
    ["", "_SandboxVars", "_spawnpoints", "_spawnregions"]
        .iter()
        .map(|suf| {
            let ext = if suf.is_empty() { "ini" } else { "lua" };
            cfg.cache_dir.join("Server").join(format!("{}{}.{}", cfg.name, suf, ext))
        })
        .filter(|p| p.is_file())
        .collect()
}

pub fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    for e in WalkDir::new(from).into_iter().filter_map(|e| e.ok()) {
        let dst = to.join(e.path().strip_prefix(from)?);
        if e.file_type().is_dir() {
            fs::create_dir_all(&dst)?;
        } else {
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p)?;
            }
            fs::copy(e.path(), &dst)?;
        }
    }
    Ok(())
}

pub fn dir_size(p: &Path) -> u64 {
    WalkDir::new(p).into_iter().filter_map(|e| e.ok()).filter(|e| e.file_type().is_file()).filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()
}

pub fn human(b: u64) -> String {
    let (mut v, mut u) = (b as f64, 0);
    while v >= 1024.0 && u < 4 {
        v /= 1024.0;
        u += 1;
    }
    format!("{:.1} {}", v, [t("Б"), t("КБ"), t("МБ"), t("ГБ"), t("ТБ")][u])
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub info: String,
}

/// Создаёт бэкап: <backups/mm>/<время>-<метка>/{<имя мира>/, *.ini, *.lua}
pub fn create(cfg: &ServerCfg, label: &str, keep: usize) -> Result<PathBuf> {
    crate::validate::server_name(&cfg.name).map_err(anyhow::Error::msg)?;
    let label: String = label.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
    let dest = root(cfg).join(format!("{}-{}", crate::atomic::stamp(), if label.is_empty() { "manual" } else { &label }));
    fs::create_dir_all(&dest)?;
    let w = world_dir(cfg);
    if w.is_dir() {
        copy_dir(&w, &dest.join("world"))?;
    }
    for f in config_files(cfg) {
        fs::copy(&f, dest.join(f.file_name().unwrap()))?;
    }
    let size = dir_size(&dest);
    fs::write(dest.join("BACKUP_INFO.txt"), format!("server={}\nsize={}\nworld={}\n", cfg.name, size, w.is_dir()))?;
    if keep > 0 {
        prune(cfg, keep)?;
    }
    Ok(dest)
}

pub fn list(cfg: &ServerCfg) -> Vec<Entry> {
    let mut v: Vec<Entry> = fs::read_dir(root(cfg))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let info = fs::read_to_string(e.path().join("BACKUP_INFO.txt")).unwrap_or_default();
            let size = info.lines().find_map(|l| l.strip_prefix("size=")).and_then(|s| s.parse::<u64>().ok()).map(human).unwrap_or_default();
            let world = if e.path().join("world").is_dir() { t("мир + конфиги") } else { t("только конфиги") };
            Entry { name: e.file_name().to_string_lossy().into_owned(), info: format!("{size} · {world}") }
        })
        .collect();
    v.sort_by(|a, b| b.name.cmp(&a.name));
    v
}

fn prune(cfg: &ServerCfg, keep: usize) -> Result<()> {
    let list = list(cfg);
    for e in list.iter().skip(keep) {
        let _ = fs::remove_dir_all(root(cfg).join(&e.name));
    }
    Ok(())
}

/// Восстановление: снимок текущего → подмена мира через .new/.old → возврат конфигов
pub fn restore(cfg: &ServerCfg, name: &str, running: bool) -> Result<PathBuf> {
    if running {
        bail!("{}", t("сервер запущен — останови его перед восстановлением"));
    }
    if name.contains(['/', '\\']) || name.contains("..") {
        bail!("{}", t("недопустимое имя бэкапа"));
    }
    let src = root(cfg).join(name);
    if !src.is_dir() {
        bail!("{}", tf!("бэкап не найден: {}", name));
    }
    let need = dir_size(&src);
    if let Ok(free) = fs4::available_space(&cfg.cache_dir) {
        if free < need {
            bail!("{}", tf!("не хватает места: свободно {}, нужно {}", human(free), human(need)));
        }
    }
    let safety = create(cfg, "before-restore", 0)?;

    let dest = world_dir(cfg);
    let from = src.join("world");
    if from.is_dir() {
        let new = dest.with_file_name(format!("{}.mm-new", cfg.name));
        let old = dest.with_file_name(format!("{}.mm-old", cfg.name));
        for p in [&new, &old] {
            if p.exists() {
                fs::remove_dir_all(p)?;
            }
        }
        copy_dir(&from, &new)?;
        if dir_size(&new) == 0 {
            let _ = fs::remove_dir_all(&new);
            bail!("{}", t("копия мира пустая — восстановление прервано, мир не тронут"));
        }
        if dest.exists() {
            fs::rename(&dest, &old).context(t("не удалось убрать текущий мир"))?;
        }
        fs::rename(&new, &dest)?;
        let _ = fs::remove_dir_all(&old);
    }
    for e in fs::read_dir(&src)?.filter_map(|e| e.ok()).filter(|e| e.path().is_file()) {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.starts_with(&cfg.name) {
            let target = cfg.cache_dir.join("Server").join(&n);
            crate::atomic::write(&target, &fs::read(e.path())?)?;
        }
    }
    Ok(safety)
}
