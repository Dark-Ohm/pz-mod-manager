//! Атомарная запись: временный файл рядом → rename. Старую версию сохраняем как `<имя>.bak-<время>`.
//! rename заменяет файл целиком, поэтому жёсткие ссылки на старый файл не портятся.
use crate::i18n::t;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

pub fn bak_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    path.with_file_name(format!("{name}.bak-{}", stamp()))
}

/// Возвращает путь созданного бэкапа (если файл уже был)
pub fn write(path: &Path, content: &[u8]) -> Result<Option<PathBuf>> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let bak = if path.exists() {
        let b = bak_path(path);
        std::fs::copy(path, &b)?;
        Some(b)
    } else {
        None
    };
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp-mm"));
    std::fs::write(&tmp, content)?;
    if let Ok(f) = std::fs::File::open(&tmp) {
        let _ = f.sync_all();
    }
    std::fs::rename(&tmp, path)?;
    Ok(bak)
}
