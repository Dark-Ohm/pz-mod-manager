//! Список игроков и их последние позиции из логов сервера (<cachedir>/Logs):
//! `*_user.txt`, `*_PerkLog.txt`, `*_connections.txt` (форматы проверены на логах B42.21).
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct Player {
    pub name: String,
    pub online: bool,
    pub pos: Option<(i32, i32, i32)>,
    pub last_seen: String,
    pub role: String,
    pub ip: String,
}

fn coords(s: &str) -> Option<(i32, i32, i32)> {
    let mut it = s.split(',').map(|p| p.trim().parse::<i32>());
    Some((it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?))
}

fn stamp(line: &str) -> String {
    match (line.find('['), line.find(']')) {
        (Some(a), Some(b)) if b > a => line[a + 1..b].to_string(),
        _ => String::new(),
    }
}

fn attr(line: &str, key: &str) -> Option<String> {
    let k = format!("{key}=\"");
    let s = line.find(&k)? + k.len();
    let e = line[s..].find('"')?;
    Some(line[s..s + e].to_string())
}

/// `[ts] 1.2.3.4 "Name" fully connected (x,y,z).` / `... disconnected player (x,y,z).`
fn user_line(line: &str, m: &mut BTreeMap<String, Player>) {
    let Some(a) = line.find('"') else { return };
    let Some(len) = line[a + 1..].find('"') else { return };
    let name = &line[a + 1..a + 1 + len];
    let tail = &line[a + 2 + len..];
    let online = if tail.contains("fully connected") {
        true
    } else if tail.contains("disconnected player") {
        false
    } else {
        return;
    };
    let p = m.entry(name.to_string()).or_insert_with(|| Player { name: name.to_string(), ..Default::default() });
    p.online = online;
    p.last_seen = stamp(line);
    if let (Some(o), Some(c)) = (tail.find('('), tail.find(')')) {
        if o < c {
            if let Some(c) = coords(&tail[o + 1..c]) {
                p.pos = Some(c);
            }
        }
    }
    let ip = line[..a].split_whitespace().last().unwrap_or("");
    if ip.contains('.') {
        p.ip = ip.to_string();
    }
}

/// `[ts] [Name][x,y,z][...]`
fn perk_line(line: &str, m: &mut BTreeMap<String, Player>) {
    let parts: Vec<&str> = line.split('[').collect();
    if parts.len() < 4 {
        return;
    }
    let name = parts[2].trim_end_matches(']');
    let Some(c) = coords(parts[3].trim_end_matches(']')) else { return };
    if let Some(p) = m.get_mut(name) {
        p.pos = Some(c);
    } else {
        m.insert(name.to_string(), Player { name: name.to_string(), pos: Some(c), last_seen: stamp(line), ..Default::default() });
    }
}

fn conn_line(line: &str, m: &mut BTreeMap<String, Player>) {
    let (Some(name), Some(ev)) = (attr(line, "username"), attr(line, "event")) else { return };
    if name == "null" || name.is_empty() {
        return;
    }
    let p = m.entry(name.clone()).or_insert_with(|| Player { name, ..Default::default() });
    if let Some(r) = attr(line, "role") {
        p.role = r;
    }
    if let Some(ip) = attr(line, "ip") {
        p.ip = ip;
    }
    match ev.as_str() {
        "fully-connected" => p.online = true,
        "disconnect" => p.online = false,
        _ => {}
    }
}

pub fn parse_all(user: &str, perk: &str, conn: &str) -> Vec<Player> {
    let mut m = BTreeMap::new();
    conn.lines().for_each(|l| conn_line(l, &mut m));
    user.lines().for_each(|l| user_line(l, &mut m));
    perk.lines().for_each(|l| perk_line(l, &mut m));
    m.into_values().collect()
}

fn newest(dir: &Path, suffix: &str) -> String {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(suffix))
        .collect();
    files.sort_by_key(|e| e.file_name());
    files.last().map(|e| std::fs::read(e.path()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()).unwrap_or_default()
}

pub fn load(cache_dir: &Path) -> Vec<Player> {
    let logs = cache_dir.join("Logs");
    parse_all(&newest(&logs, "_user.txt"), &newest(&logs, "_PerkLog.txt"), &newest(&logs, "_connections.txt"))
}
