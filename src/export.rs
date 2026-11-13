use crate::i18n::t;
use crate::mods::Mod;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

/// Строки Mods= и WorkshopItems= для servertest.ini
/// При nosteam WorkshopItems пустой: воркшоп без Steam не работает
/// backslash: формат B42, где id в Mods= пишутся как \\ModId
pub fn build(order: &[String], installed: &HashMap<String, Mod>, nosteam: bool, backslash: bool) -> (String, String) {
    let pre = if backslash { "\\" } else { "" };
    let mods = order.iter().map(|i| format!("{pre}{i}")).collect::<Vec<_>>().join(";");
    let mut ws: Vec<String> = vec![];
    for id in order.iter().filter(|_| !nosteam) {
        if let Some(w) = installed.get(id).and_then(|m| m.workshop_id.clone()) {
            if !ws.contains(&w) {
                ws.push(w);
            }
        }
    }
    (mods, ws.join(";"))
}

/// Записывает значения в ini. Сохраняет CRLF/LF и завершающий «;», атомарно, бэкап `.bak-<время>`
pub fn write_ini(path: &Path, mods: &str, workshop: &str) -> Result<()> {
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let eol = if old.contains("\r\n") { "\r\n" } else { "\n" };
    let fmt = |key: &str, val: &str, prev: Option<&str>| {
        let semi = prev.is_some_and(|p| p.trim_end().ends_with(';')) && !val.is_empty();
        format!("{key}={val}{}", if semi { ";" } else { "" })
    };
    let (mut has_m, mut has_w) = (false, false);
    let mut lines: Vec<String> = old
        .lines()
        .map(|l| {
            if l.starts_with("Mods=") {
                has_m = true;
                fmt("Mods", mods, Some(l))
            } else if l.starts_with("WorkshopItems=") {
                has_w = true;
                fmt("WorkshopItems", workshop, Some(l))
            } else {
                l.to_string()
            }
        })
        .collect();
    if !has_m {
        lines.push(format!("Mods={mods}"));
    }
    if !has_w {
        lines.push(format!("WorkshopItems={workshop}"));
    }
    crate::atomic::write(path, (lines.join(eol) + eol).as_bytes())?;
    Ok(())
}

/// Читает Mods= из ini: id без ведущего «\» и признак B42-стиля (по умолчанию да)
pub fn read_ini_mods(path: &Path) -> (Vec<String>, bool) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let line = text.lines().find(|l| l.starts_with("Mods=")).unwrap_or("");
    let raw: Vec<&str> = line["Mods=".len().min(line.len())..].split(';').map(str::trim).filter(|s| !s.is_empty()).collect();
    let backslash = raw.first().map_or(true, |s| s.starts_with('\\'));
    (raw.iter().map(|s| s.strip_prefix('\\').unwrap_or(s).to_string()).collect(), backslash)
}
