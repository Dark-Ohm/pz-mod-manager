//! SandboxVars.lua: чтение полей по категориям и точечная замена значений
//! (сохраняет отступы, запятые, комментарии, кавычки и CRLF).

pub const ROOT: &str = "Корень SandboxVars";

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Bool,
    Number,
    Text,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub category: String,
    pub key: String,
    pub value: String,
    pub kind: Kind,
    pub quoted: bool,
}

/// Разбор строки `key = value, -- comment` → (key, value, quoted, comment)
fn split_kv(line: &str) -> Option<(String, String, bool, String)> {
    let t = line.trim();
    let (k, rest) = t.split_once('=')?;
    let key = k.trim();
    if key.is_empty() || !key.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') || key.contains(' ') {
        return None;
    }
    let rest = rest.trim();
    // значение в кавычках — берём до закрывающей, иначе до запятой/комментария
    let (val, quoted, tail) = if let Some(r) = rest.strip_prefix('"') {
        let end = r.find('"')?;
        (r[..end].to_string(), true, r[end + 1..].to_string())
    } else {
        let end = rest.find([',', '-']).unwrap_or(rest.len());
        let end = if rest[end..].starts_with('-') && !rest[end..].starts_with("--") { rest.len() } else { end };
        (rest[..end].trim().to_string(), false, rest[end..].to_string())
    };
    let comment = tail.find("--").map(|p| tail[p..].to_string()).unwrap_or_default();
    Some((key.to_string(), val, quoted, comment))
}

fn opens_table(line: &str) -> Option<String> {
    let t = line.trim();
    if t.ends_with('{') {
        return Some(t.split_once('=').map(|(n, _)| n.trim()).unwrap_or("").to_string());
    }
    None
}

fn closes_table(line: &str) -> bool {
    matches!(line.trim(), "}" | "},")
}

pub fn parse(text: &str) -> Vec<Field> {
    let mut stack: Vec<String> = vec![];
    let mut out = vec![];
    for raw in text.lines() {
        let l = raw.trim();
        if l.is_empty() || l.starts_with("--") {
            continue;
        }
        if let Some(name) = opens_table(l) {
            if name != "SandboxVars" {
                stack.push(name);
            }
            continue;
        }
        if closes_table(l) {
            stack.pop();
            continue;
        }
        if let Some((key, value, quoted, _)) = split_kv(l) {
            let kind = if value == "true" || value == "false" {
                Kind::Bool
            } else if value.parse::<f64>().is_ok() {
                Kind::Number
            } else {
                Kind::Text
            };
            out.push(Field { category: stack.last().cloned().unwrap_or_else(|| ROOT.to_string()), key, value, kind, quoted });
        }
    }
    out
}

/// Применяет изменения (категория, ключ, значение). Возвращает (новый текст, сколько заменено).
pub fn update(text: &str, changes: &[(String, String, String)]) -> (String, usize) {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut stack: Vec<String> = vec![];
    let mut done = 0;
    let mut out: Vec<String> = vec![];
    for raw in text.lines() {
        let l = raw.trim();
        if let Some(name) = opens_table(l) {
            if name != "SandboxVars" {
                stack.push(name);
            }
            out.push(raw.to_string());
            continue;
        }
        if closes_table(l) {
            stack.pop();
            out.push(raw.to_string());
            continue;
        }
        if !l.starts_with("--") {
            if let Some((key, _, quoted, comment)) = split_kv(l) {
                let cat = stack.last().cloned().unwrap_or_else(|| ROOT.to_string());
                if let Some((_, _, nv)) = changes.iter().find(|(c, k, _)| *c == cat && *k == key) {
                    let indent: String = raw.chars().take_while(|c| c.is_whitespace()).collect();
                    let comma = if l.contains(',') || raw.trim_end().ends_with(',') { "," } else { "" };
                    let val = if quoted { format!("\"{nv}\"") } else { nv.clone() };
                    let sp = if comment.is_empty() { String::new() } else { format!(" {comment}") };
                    out.push(format!("{indent}{key} = {val}{comma}{sp}"));
                    done += 1;
                    continue;
                }
            }
        }
        out.push(raw.to_string());
    }
    (out.join(eol) + eol, done)
}

pub fn path(cfg: &crate::config::ServerCfg) -> std::path::PathBuf {
    cfg.cache_dir.join("Server").join(format!("{}_SandboxVars.lua", cfg.name))
}
