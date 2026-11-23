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
    /// пояснение из комментариев перед ключом
    pub desc: String,
    /// варианты «N = Название» (выпадающий список)
    pub options: Vec<(String, String)>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// значение по умолчанию (для вариантов — номер варианта)
    pub default: Option<String>,
}

/// Комментарии перед ключом → (описание, варианты, min, max, default)
type Hints = (String, Vec<(String, String)>, Option<f64>, Option<f64>, Option<String>);

fn num_after(s: &str, tag: &str) -> Option<String> {
    let r = s.split_once(tag)?.1.trim_start();
    let end = r.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(r.len());
    (end > 0).then(|| r[..end].to_string())
}

fn hints(comments: &[String]) -> Hints {
    let mut desc: Vec<String> = vec![];
    let mut options = vec![];
    let (mut min, mut max, mut def) = (None, None, None);
    let mut def_label: Option<String> = None;
    for c in comments {
        let c = c.trim();
        if let Some((n, label)) = c.split_once(" = ") {
            if !n.is_empty() && n.chars().all(|ch| ch.is_ascii_digit() || ch == '-') {
                options.push((n.to_string(), label.trim().to_string()));
                continue;
            }
        }
        let mut line = c;
        if let Some((a, b)) = line.split_once("Default =") {
            def_label = Some(b.trim().to_string());
            line = a.trim();
        }
        if line.contains("Min:") {
            min = num_after(line, "Min:").and_then(|v| v.parse().ok());
            max = num_after(line, "Max:").and_then(|v| v.parse().ok());
            def = num_after(line, "Default:");
            line = line.split("Min:").next().unwrap_or("").trim();
        }
        if !line.is_empty() {
            desc.push(line.to_string());
        }
    }
    if def.is_none() {
        if let Some(l) = def_label {
            def = options.iter().find(|(_, o)| *o == l).map(|(n, _)| n.clone()).or(Some(l).filter(|l| l == "true" || l == "false"));
        }
    }
    (desc.join(" "), options, min, max, def)
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
    let mut pending: Vec<String> = vec![];
    for raw in text.lines() {
        let l = raw.trim();
        if l.is_empty() {
            pending.clear();
            continue;
        }
        if let Some(c) = l.strip_prefix("--") {
            pending.push(c.trim().to_string());
            continue;
        }
        let comments = std::mem::take(&mut pending);
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
            let (desc, options, min, max, default) = hints(&comments);
            out.push(Field { category: stack.last().cloned().unwrap_or_else(|| ROOT.to_string()), key, value, kind, quoted, desc, options, min, max, default });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hints() {
        let t = "SandboxVars = {\n    -- Zombies count. Default = Normal\n    -- 1 = Insane\n    -- 4 = Normal\n    Zombies = 4,\n    -- Loot rarity\n    -- Min: 0 Max: 100 Default: 20\n    Rate = 50,\n}\n";
        let f = parse(t);
        assert_eq!(f[0].options.len(), 2);
        assert_eq!(f[0].default.as_deref(), Some("4"));
        assert_eq!(f[0].desc, "Zombies count.");
        assert_eq!((f[1].min, f[1].max, f[1].default.as_deref()), (Some(0.0), Some(100.0), Some("20")));
        assert_eq!(f[1].desc, "Loot rarity");
    }
}
