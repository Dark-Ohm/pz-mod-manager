use crate::i18n::t;
use anyhow::{Context, Result, bail};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

pub const APP_ID: &str = "108600";
pub enum Msg {
    Log(String),
    Done(bool),
}

pub fn steamcmd_dir() -> PathBuf {
    crate::config::data_dir().join("steamcmd")
}

pub fn cache_dir() -> PathBuf {
    crate::config::data_dir().join("cache")
}

pub fn cache_content() -> PathBuf {
    cache_dir().join("steamapps/workshop/content").join(APP_ID)
}

/// Вытащить id воркшопа из ссылок и голых чисел
pub fn parse_ids(text: &str) -> Vec<String> {
    let mut ids: Vec<String> = vec![];
    for tok in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let tok = tok.trim();
        let id = if let Some(p) = tok.find("id=") {
            tok[p + 3..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>()
        } else if !tok.is_empty() && tok.chars().all(|c| c.is_ascii_digit()) {
            tok.to_string()
        } else {
            continue;
        };
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

fn ensure_steamcmd(tx: &Sender<Msg>) -> Result<PathBuf> {
    let dir = steamcmd_dir();
    let bin = dir.join(crate::platform::steamcmd_bin());
    if bin.exists() {
        return Ok(bin);
    }
    let _ = tx.send(Msg::Log(t("Скачиваю SteamCMD…").into()));
    std::fs::create_dir_all(&dir)?;
    let resp = ureq::get(crate::platform::steamcmd_url()).call().context(t("не удалось скачать SteamCMD"))?;
    let mut reader = resp.into_body().into_reader();
    if cfg!(windows) {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        zip::ZipArchive::new(std::io::Cursor::new(buf))?.extract(&dir).context(t("распаковка SteamCMD"))?;
    } else {
        tar::Archive::new(flate2::read::GzDecoder::new(reader)).unpack(&dir).context(t("распаковка SteamCMD"))?;
    }
    if !bin.exists() {
        bail!("{}", tf!("{} не найден после распаковки", crate::platform::steamcmd_bin()));
    }
    Ok(bin)
}

/// Скачивание в фоне; прогресс идёт через канал
pub fn download(ids: Vec<String>, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let ok = match run(&ids, &tx) {
            Ok(ok) => ok,
            Err(e) => {
                let _ = tx.send(Msg::Log(tf!("Ошибка: {}", format!("{e:#}"))));
                false
            }
        };
        let _ = tx.send(Msg::Done(ok));
    });
}

/// Синхронная загрузка (для фоновых задач, которые сами крутятся в потоке)
pub fn run_blocking(ids: &[String], tx: &Sender<Msg>) -> Result<bool> {
    run(ids, tx)
}

fn run(ids: &[String], tx: &Sender<Msg>) -> Result<bool> {
    let sh = ensure_steamcmd(tx)?;
    std::fs::create_dir_all(cache_dir())?;
    let missing = |ids: &[String]| ids.iter().filter(|i| !cache_content().join(i).is_dir()).cloned().collect::<Vec<_>>();
    // SteamCMD при первом запуске обновляет сам себя и может завершиться с кодом 7 —
    // поэтому успех определяем по появившимся папкам, а при неудаче пробуем ещё раз.
    for attempt in 1..=2 {
        let todo = missing(ids);
        if todo.is_empty() {
            break;
        }
        if attempt == 2 {
            let _ = tx.send(Msg::Log(t("Повторяю загрузку после обновления SteamCMD…").into()));
        }
        // Каждая загрузка идёт в свою чистую папку. Общая папка кэша хранит состояние всех
        // скачанных ранее предметов, и SteamCMD обходит их все (часы работы и зависания).
        let scratch = fresh_scratch_dir()?;
        let mut cmd = Command::new(&sh);
        cmd.current_dir(steamcmd_dir()).arg("+force_install_dir").arg(&scratch).args(["+login", "anonymous"]);
        for id in &todo {
            cmd.args(["+workshop_download_item", APP_ID, id]);
        }
        cmd.arg("+quit").stdout(Stdio::piped()).stderr(Stdio::piped());
        crate::platform::hide_window(&mut cmd);
        let mut child = cmd.spawn().context(t("не удалось запустить SteamCMD"))?;
        if let Some(mut err) = child.stderr.take() {
            let tx2 = tx.clone();
            std::thread::spawn(move || pump(&mut err, &tx2));
        }
        if let Some(mut out) = child.stdout.take() {
            pump(&mut out, tx);
        }
        let _ = child.wait()?;
        let moved = adopt_downloads(&scratch, &todo);
        let _ = std::fs::remove_dir_all(&scratch);
        moved?;
    }
    let left = missing(ids);
    if !left.is_empty() {
        let _ = tx.send(Msg::Log(tf!("Не скачано: {}", left.join(", "))));
    }
    Ok(left.is_empty())
}

/// Чистая временная папка для одной загрузки, на том же диске, что и кэш (перенос без копирования)
fn fresh_scratch_dir() -> Result<PathBuf> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let dir = crate::config::data_dir().join("downloading").join(format!("{}-{}", std::process::id(), stamp));
    std::fs::create_dir_all(&dir).context(t("не удалось создать временную папку загрузки"))?;
    Ok(dir)
}

/// Переносит скачанные предметы из временной папки в кэш; неполный остаток прошлой попытки заменяется
fn adopt_downloads(scratch: &Path, ids: &[String]) -> Result<()> {
    let from_root = scratch.join("steamapps/workshop/content").join(APP_ID);
    let to_root = cache_content();
    std::fs::create_dir_all(&to_root)?;
    for id in ids {
        let from = from_root.join(id);
        if !from.is_dir() {
            continue;
        }
        let to = to_root.join(id);
        if to.exists() {
            std::fs::remove_dir_all(&to)?;
        }
        if std::fs::rename(&from, &to).is_err() {
            copy_dir(&from, &to)?;
        }
    }
    Ok(())
}

/// Убирает цветовые escape-коды терминала (SteamCMD пишет `\x1b[0m` и т.п.)
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                // CSI: параметры и промежуточные байты, затем финальный символ @..~
                for d in it.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out.trim().to_string()
}

/// steamcmd рисует прогресс через \r, поэтому режем и по \r, и по \n
pub fn pump(r: &mut impl Read, tx: &Sender<Msg>) {
    let mut buf = [0u8; 1024];
    let mut line = Vec::new();
    while let Ok(n) = r.read(&mut buf) {
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            if b == b'\n' || b == b'\r' {
                if !line.is_empty() {
                    let _ = tx.send(Msg::Log(strip_ansi(&String::from_utf8_lossy(&line))));
                    line.clear();
                }
            } else {
                line.push(b);
            }
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    for e in walkdir::WalkDir::new(from).into_iter().filter_map(|e| e.ok()) {
        let rel = e.path().strip_prefix(from)?;
        let dst = to.join(rel);
        if e.file_type().is_dir() {
            std::fs::create_dir_all(&dst)?;
        } else {
            if let Some(p) = dst.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(e.path(), &dst)?;
        }
    }
    Ok(())
}

/// Куда ставить на сервер
pub enum ServerTarget<'a> {
    /// Локальные моды (-nosteam): <dir>/<ModName>
    Local(&'a Path),
    /// Воркшоп-раскладка: <dir>/<workshop_id>
    Workshop(&'a Path),
}

/// Установка скачанного элемента воркшопа на клиент и/или сервер
pub fn install(workshop_id: &str, client_mods: Option<&Path>, server: Option<ServerTarget>) -> Result<String> {
    let item = cache_content().join(workshop_id);
    if !item.exists() {
        bail!("{}", tf!("элемент {} не найден в кэше", workshop_id));
    }
    let copy_mods = |dst: &Path| -> Result<()> {
        let mods = item.join("mods");
        let src = if mods.exists() { mods } else { item.clone() };
        for e in std::fs::read_dir(&src)?.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()) {
            copy_dir(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    };
    let mut done = vec![];
    if let Some(dst) = client_mods {
        copy_mods(dst)?;
        done.push(t("клиент"));
    }
    match server {
        Some(ServerTarget::Local(dst)) => {
            copy_mods(dst)?;
            done.push(t("сервер (локальные моды)"));
        }
        Some(ServerTarget::Workshop(dst)) => {
            copy_dir(&item, &dst.join(workshop_id))?;
            done.push(t("сервер (воркшоп)"));
        }
        None => {}
    }
    Ok(tf!("{}: установлено → {}", workshop_id, done.join(" + ")))
}

/// Папки модов внутри скачанного элемента: то, что копируется в mods/ при локальной установке
pub fn item_mod_folders(workshop_id: &str) -> Vec<String> {
    let item = cache_content().join(workshop_id);
    let mods = item.join("mods");
    let src = if mods.exists() { mods } else { item };
    let mut v: Vec<String> = std::fs::read_dir(&src)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// Где стоит скачанный элемент в окружении
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Placed {
    pub server: bool,
    pub client: bool,
}

pub fn placed(workshop_id: &str, s: &crate::config::Settings, srv: &crate::config::ServerCfg) -> Placed {
    let folders = item_mod_folders(workshop_id);
    let all_in = |dir: &Path| !folders.is_empty() && folders.iter().all(|f| dir.join(f).is_dir());
    let server = if srv.nosteam { all_in(&srv.mods_dir()) } else { srv.workshop_dir.join(workshop_id).is_dir() };
    Placed { server, client: all_in(&srv.client_mods_dir(s)) }
}

/// Установка скачанного элемента в окружение: на его клиент и/или сервер
pub fn install_into(workshop_id: &str, s: &crate::config::Settings, srv: &crate::config::ServerCfg, client: bool, server: bool) -> Result<String> {
    if !client && !server {
        bail!("{}", t("не выбрано, куда ставить"));
    }
    let client_dir = srv.client_mods_dir(s);
    let mods_dir = srv.mods_dir();
    let target = server.then(|| if srv.nosteam { ServerTarget::Local(mods_dir.as_path()) } else { ServerTarget::Workshop(srv.workshop_dir.as_path()) });
    let msg = install(workshop_id, client.then_some(client_dir.as_path()), target)?;
    Ok(format!("[{}] {msg}", srv.label))
}
