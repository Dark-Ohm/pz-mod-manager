//! Установка обновления локального мода и откат. Старая версия никогда не удаляется:
//! перед заменой папка уходит в `<cachedir>/backups/mods-old/<время>__<workshop id>__<цель>/`.
use crate::i18n::t;
use crate::backup::copy_dir;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub fn backup_root(cache_dir: &Path) -> PathBuf {
    cache_dir.join("backups").join("mods-old")
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Meta {
    pub workshop_id: String,
    pub target: String,
    pub baseline_before: Option<u64>,
}

/// Переносит папку (rename, при неудаче — копия и удаление)
fn move_dir(from: &Path, to: &Path) -> Result<()> {
    if let Some(p) = to.parent() {
        fs::create_dir_all(p)?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_dir(from, to)?;
    fs::remove_dir_all(from)?;
    Ok(())
}

/// Ставит `src` на место `dst`; прежняя версия `dst` уходит в `backup_dir/<имя>`
fn replace_dir(src: &Path, dst: &Path, backup_dir: &Path) -> Result<()> {
    let name = dst.file_name().context(t("пустое имя папки"))?.to_string_lossy().into_owned();
    let new = dst.with_file_name(format!("{name}.mm-new"));
    if new.exists() {
        fs::remove_dir_all(&new)?;
    }
    copy_dir(src, &new)?;
    if dst.exists() {
        move_dir(dst, &backup_dir.join(&name)).with_context(|| tf!("не удалось убрать старую версию {}", name))?;
    }
    fs::rename(&new, dst)?;
    Ok(())
}

fn mod_folders(item_dir: &Path) -> Result<Vec<PathBuf>> {
    let mods = item_dir.join("mods");
    let src = if mods.is_dir() { mods } else { item_dir.to_path_buf() };
    let v: Vec<PathBuf> = fs::read_dir(&src)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    if v.is_empty() {
        bail!("{}", t("в скачанном элементе нет папок модов"));
    }
    Ok(v)
}

/// Устанавливает скачанный элемент (`item_dir`) в `target_mods_dir`. Возвращает имена заменённых папок.
pub fn install(item_dir: &Path, ws_id: &str, tag: &str, target_mods_dir: &Path, cache_dir: &Path, baseline_before: Option<u64>) -> Result<Vec<String>> {
    let folders = mod_folders(item_dir)?;
    let ts = crate::atomic::stamp();
    let bdir = backup_root(cache_dir).join(format!("{ts}__{ws_id}__{tag}"));
    fs::create_dir_all(&bdir)?;
    fs::write(bdir.join("meta.json"), serde_json::to_string_pretty(&Meta { workshop_id: ws_id.into(), target: target_mods_dir.to_string_lossy().into_owned(), baseline_before })?)?;
    let mut done = vec![];
    for f in folders {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        replace_dir(&f, &target_mods_dir.join(&name), &bdir)?;
        done.push(name);
    }
    Ok(done)
}

/// Все откаты для элемента, новые первыми
pub fn backups_for(cache_dir: &Path, ws_id: &str) -> Vec<(PathBuf, Meta)> {
    let mut v: Vec<(PathBuf, Meta)> = fs::read_dir(backup_root(cache_dir))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| !p.to_string_lossy().ends_with(".used"))
        .filter_map(|p| {
            let m: Meta = serde_json::from_str(&fs::read_to_string(p.join("meta.json")).ok()?).ok()?;
            (m.workshop_id == ws_id).then_some((p, m))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v
}

/// Откат на предыдущую версию: текущая уходит в новый бэкап, прежняя возвращается на место.
/// Возвращает baseline, который нужно восстановить.
pub fn rollback(cache_dir: &Path, ws_id: &str, current_baseline: Option<u64>) -> Result<Option<u64>> {
    let (dir, meta) = backups_for(cache_dir, ws_id).into_iter().next().context(t("нет сохранённой предыдущей версии"))?;
    let target = PathBuf::from(&meta.target);
    let ts = crate::atomic::stamp();
    let newb = backup_root(cache_dir).join(format!("{ts}__{ws_id}__rollback"));
    fs::create_dir_all(&newb)?;
    fs::write(newb.join("meta.json"), serde_json::to_string_pretty(&Meta { workshop_id: ws_id.into(), target: meta.target.clone(), baseline_before: current_baseline })?)?;
    for e in fs::read_dir(&dir)?.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()) {
        replace_dir(&e.path(), &target.join(e.file_name()), &newb)?;
    }
    // использованный бэкап помечаем, чтобы повторный откат не брал его же
    fs::rename(&dir, dir.with_file_name(format!("{}.used", dir.file_name().unwrap().to_string_lossy())))?;
    Ok(meta.baseline_before)
}

/// Защита от неверной привязки: скачанный элемент должен содержать хотя бы один из ожидаемых модов
pub fn item_matches(found_ids: &[String], expected: &[String]) -> bool {
    expected.is_empty() || expected.iter().any(|e| found_ids.contains(e))
}
