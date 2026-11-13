//! Подбор привязок «локальный мод → элемент воркшопа» по названию через Steam Web API
//! (`IPublishedFileService/QueryFiles`, нужен бесплатный ключ). Ничего не применяется само:
//! результат — предложения, которые пользователь подтверждает.
use crate::i18n::t;
use anyhow::{Result, bail};
use serde_json::Value;

const API: &str = "https://api.steampowered.com/IPublishedFileService/QueryFiles/v1/";

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub title: String,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct Proposal {
    pub mod_id: String,
    pub mod_name: String,
    /// лучшие первыми
    pub cands: Vec<Candidate>,
    pub chosen: usize,
    pub accept: bool,
}

/// «Fists of Survival [B42] (v1.2)» → «fists of survival»
pub fn clean_query(name: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in name.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.is_empty() { name.trim().to_string() } else { out }
}

/// CamelCase id → слова: «ViewpointAutoView» → «Viewpoint Auto View»
pub fn split_camel(id: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in id.chars() {
        if c.is_uppercase() && prev_lower {
            out.push(' ');
        }
        out.push(if c == '_' || c == '-' { ' ' } else { c });
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
    }
    out
}

fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && !matches!(*t, "b42" | "b41" | "42" | "41" | "build" | "mod" | "the"))
        .map(str::to_string)
        .collect()
}

/// 1.0 — названия совпали; иначе доля общих слов (+ бонус, если одно название целиком входит в другое)
pub fn score(local: &str, remote_title: &str) -> f32 {
    let (a, b) = (tokens(local), tokens(remote_title));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let inter = a.iter().filter(|t| b.contains(t)).count() as f32;
    let uni = (a.len() + b.len()) as f32 - inter;
    let mut s = inter / uni;
    let subset = a.iter().all(|t| b.contains(t)) || b.iter().all(|t| a.contains(t));
    if subset {
        s = (s + 0.2).min(0.95);
    }
    s
}

pub fn parse_search(json: &str, local_name: &str) -> Result<Vec<Candidate>> {
    let v: Value = serde_json::from_str(json)?;
    let list = v["response"]["publishedfiledetails"].as_array().cloned().unwrap_or_default();
    let mut out: Vec<Candidate> = list
        .iter()
        .filter(|d| d["publishedfileid"].is_string())
        .map(|d| {
            let title = d["title"].as_str().unwrap_or("").to_string();
            Candidate { id: d["publishedfileid"].as_str().unwrap().to_string(), score: score(local_name, &title), title }
        })
        .collect();
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

pub fn search(key: &str, text: &str) -> Result<Vec<Candidate>> {
    let resp = ureq::get(API)
        .header("Accept", "application/json")
        .query("key", key)
        .query("query_type", "12") // RankedByTextSearch
        .query("page", "1")
        .query("numperpage", "6")
        .query("appid", "108600")
        // без return_details Steam отдаёт только id, без названий (проверено на реальном ответе)
        .query("return_details", "true")
        .query("search_text", text)
        .call();
    let body = match resp {
        Ok(r) => r.into_body().read_to_string()?,
        Err(ureq::Error::StatusCode(401 | 403)) => bail!("{}", t("Steam отклонил ключ API — проверь его в настройках")),
        Err(ureq::Error::StatusCode(429)) => bail!("{}", t("Steam ограничил частоту запросов (429) — повтори позже")),
        Err(e) => return Err(anyhow::Error::new(e).context(t("запрос поиска не удался"))),
    };
    parse_search(&body, text)
}

/// Параллельных запросов к Steam
pub const WORKERS: usize = 6;

/// Кандидаты для одного мода. Порог — 0.5; иначе второй запрос по id, разбитому на слова.
fn propose_one(key: &str, id: &str, name: &str) -> Result<Option<Proposal>> {
    let q = clean_query(name);
    let mut cands = search(key, &q)?;
    if cands.first().is_none_or(|c| c.score < 0.5) {
        let alt = split_camel(id);
        if alt.to_lowercase() != q.to_lowercase() {
            for mut c in search(key, &alt)? {
                c.score = c.score.max(score(name, &c.title));
                if !cands.iter().any(|x| x.id == c.id) {
                    cands.push(c);
                }
            }
            cands.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        }
    }
    cands.truncate(5);
    Ok((!cands.is_empty()).then(|| {
        let accept = cands[0].score >= 0.999;
        Proposal { mod_id: id.to_string(), mod_name: name.to_string(), cands, chosen: 0, accept }
    }))
}

/// Подбирает кандидатов для каждого мода (id, название) в несколько потоков.
/// Порядок результата совпадает с порядком входа. Первая ошибка (ключ, лимит) останавливает всех.
pub fn propose(key: &str, mods: &[(String, String)], progress: &(dyn Fn(usize, usize) + Sync), cancel: &(dyn Fn() -> bool + Sync)) -> Result<Vec<Proposal>> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, Proposal)>> = Mutex::new(vec![]);
    let error: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    std::thread::scope(|sc| {
        for _ in 0..WORKERS.min(mods.len().max(1)) {
            sc.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= mods.len() || cancel() || error.lock().unwrap().is_some() {
                        break;
                    }
                    match propose_one(key, &mods[i].0, &mods[i].1) {
                        Ok(Some(p)) => results.lock().unwrap().push((i, p)),
                        Ok(None) => {}
                        Err(e) => {
                            error.lock().unwrap().get_or_insert(e);
                            break;
                        }
                    }
                    progress(done.fetch_add(1, Ordering::SeqCst) + 1, mods.len());
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            });
        }
    });
    if let Some(e) = error.into_inner().unwrap() {
        return Err(e);
    }
    let mut v = results.into_inner().unwrap();
    v.sort_by_key(|(i, _)| *i);
    Ok(v.into_iter().map(|(_, p)| p).collect())
}
