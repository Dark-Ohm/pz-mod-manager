//! Минимальная работа с servertest-подобными ini: чтение ключей и точечная замена
//! (сохраняет CRLF/LF, порядок строк и комментарии).
use crate::atomic;
use anyhow::Result;
use std::path::Path;

pub fn get(text: &str, key: &str) -> Option<String> {
    let pre = format!("{key}=");
    text.lines().find(|l| l.starts_with(&pre)).map(|l| l[pre.len()..].trim().to_string())
}

pub fn read_key(path: &Path, key: &str) -> Option<String> {
    get(&std::fs::read_to_string(path).ok()?, key)
}

/// Заменяет значения только у УЖЕ существующих ключей (неизвестные пропускает).
/// Возвращает (применённые ключи, путь бэкапа).
pub fn set_existing(path: &Path, kv: &[(String, String)]) -> Result<(Vec<String>, Option<std::path::PathBuf>)> {
    let text = std::fs::read_to_string(path)?;
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut applied = vec![];
    let lines: Vec<String> = text
        .lines()
        .map(|l| {
            for (k, v) in kv {
                if l.starts_with(&format!("{k}=")) {
                    applied.push(k.clone());
                    return format!("{k}={v}");
                }
            }
            l.to_string()
        })
        .collect();
    let bak = atomic::write(path, (lines.join(eol) + eol).as_bytes())?;
    Ok((applied, bak))
}
