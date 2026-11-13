//! Данные Steam Workshop без ключа: даты обновлений (публичный API) и страница изменений.
use crate::i18n::t;
use anyhow::{Context, Result, bail};
use serde_json::Value;

const API: &str = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Remote {
    pub id: String,
    pub title: String,
    pub time_updated: u64,
    pub time_created: u64,
    pub size: u64,
    /// false — элемент удалён, скрыт или недоступен
    pub ok: bool,
}

pub fn parse_details(json: &str) -> Result<Vec<Remote>> {
    let v: Value = serde_json::from_str(json).context(t("ответ Steam не JSON"))?;
    let list = v["response"]["publishedfiledetails"].as_array().context(t("в ответе нет publishedfiledetails"))?;
    let num = |x: &Value| x.as_u64().or_else(|| x.as_str().and_then(|s| s.parse().ok())).unwrap_or(0);
    Ok(list
        .iter()
        .map(|d| Remote {
            id: d["publishedfileid"].as_str().unwrap_or("").to_string(),
            title: d["title"].as_str().unwrap_or("").to_string(),
            time_updated: num(&d["time_updated"]),
            time_created: num(&d["time_created"]),
            size: num(&d["file_size"]),
            ok: d["result"].as_u64() == Some(1),
        })
        .collect())
}

/// Пачками по 100 id
pub fn fetch_details(ids: &[String]) -> Result<Vec<Remote>> {
    let mut out = vec![];
    for chunk in ids.chunks(100) {
        let mut form: Vec<(String, String)> = vec![("itemcount".into(), chunk.len().to_string())];
        for (i, id) in chunk.iter().enumerate() {
            form.push((format!("publishedfileids[{i}]"), id.clone()));
        }
        let body = ureq::post(API)
            .send_form(form.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .context(t("не удалось связаться со Steam"))?
            .into_body()
            .read_to_string()?;
        out.extend(parse_details(&body)?);
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Change {
    /// «Update: 6 Jan, 2022 @ 1:59am» как на сайте
    pub when: String,
    /// unix-время записи (id абзаца на странице совпадает с time_updated)
    pub ts: Option<u64>,
    pub text: String,
}

fn clean(html: &str) -> String {
    let mut t = html.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n").replace("</li>", "\n").replace("</p>", "\n");
    let mut out = String::new();
    let mut in_tag = false;
    for c in t.drain(..) {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&");
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    let mut res: Vec<&str> = vec![];
    for l in lines {
        if !(l.is_empty() && res.last().is_none_or(|p| p.is_empty())) {
            res.push(l);
        }
    }
    res.join("\n").trim().to_string()
}

pub fn parse_changelog(html: &str) -> Vec<Change> {
    const HEAD: &str = "changelog headline\">";
    let mut out = vec![];
    let mut rest = html;
    while let Some(i) = rest.find(HEAD) {
        rest = &rest[i + HEAD.len()..];
        let Some(end) = rest.find("</div>") else { break };
        let when = clean(&rest[..end]);
        let after = &rest[end..];
        let seg = &after[..after.find(HEAD).unwrap_or(after.len())];
        let (ts, text) = match seg.find("<p id=\"") {
            Some(p) => {
                let s = &seg[p + 7..];
                let ts = s.find('"').and_then(|q| s[..q].parse::<u64>().ok());
                let start = s.find('>').map_or(0, |g| g + 1);
                let stop = s.find("</p>").unwrap_or(s.len());
                (ts, clean(&s[start.min(stop)..stop]))
            }
            None => (None, String::new()),
        };
        out.push(Change { when, ts, text });
    }
    out
}

pub fn fetch_changelog(id: &str) -> Result<Vec<Change>> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        bail!("{}", t("некорректный id воркшопа"));
    }
    let html = match ureq::get(format!("https://steamcommunity.com/sharedfiles/filedetails/changelog/{id}")).header("User-Agent", "Mozilla/5.0 (pz-mod-manager)").header("Accept", "text/html,application/xhtml+xml").header("Accept-Language", "en-US,en;q=0.8").call() {
        Ok(r) => r.into_body().read_to_string()?,
        // без заголовка Accept Steam тоже отвечает 429, поэтому он выше; здесь — настоящий лимит частоты
        Err(ureq::Error::StatusCode(429)) => bail!("{}", t("Steam временно ограничил частоту запросов (429). Подожди минуту и нажми «Что нового» ещё раз")),
        Err(ureq::Error::StatusCode(c)) => bail!("{}", tf!("страница изменений ответила кодом {}", c)),
        Err(e) => return Err(anyhow::Error::new(e).context(t("не удалось загрузить страницу изменений"))),
    };
    Ok(parse_changelog(&html))
}

pub fn page_url(id: &str) -> String {
    format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}")
}

// ---------- кэш чейнджлогов и защита от лимита Steam ----------

/// Сколько хранить чейнджлог на диске, секунд (6 часов)
pub const CHANGELOG_TTL: u64 = 6 * 3600;
/// Пауза после ответа 429, секунд
pub const COOLDOWN_SECS: u64 = 300;
/// Минимум между двумя запросами к странице изменений, секунд
pub const MIN_GAP_SECS: u64 = 2;

fn cache_path(id: &str) -> std::path::PathBuf {
    crate::config::config_root().join("changelog-cache").join(format!("{id}.json"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Свежий кэш (не старше TTL) или None
pub fn cached_changelog(id: &str) -> Option<Vec<Change>> {
    cached_changelog_at(id, now_secs())
}

pub fn cached_changelog_at(id: &str, now: u64) -> Option<Vec<Change>> {
    #[derive(serde::Deserialize)]
    struct F {
        at: u64,
        list: Vec<Change>,
    }
    let f: F = serde_json::from_str(&std::fs::read_to_string(cache_path(id)).ok()?).ok()?;
    (now.saturating_sub(f.at) <= CHANGELOG_TTL).then_some(f.list)
}

pub fn store_changelog(id: &str, list: &[Change]) {
    let p = cache_path(id);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let body = serde_json::json!({ "at": now_secs(), "list": list });
    let _ = std::fs::write(p, body.to_string());
}

/// fetch_changelog с кэшем: из сети берём только то, чего нет в свежем кэше
pub fn fetch_changelog_cached(id: &str) -> Result<Vec<Change>> {
    if let Some(c) = cached_changelog(id) {
        return Ok(c);
    }
    let c = fetch_changelog(id)?;
    store_changelog(id, &c);
    Ok(c)
}
