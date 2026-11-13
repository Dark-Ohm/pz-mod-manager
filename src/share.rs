//! Общий файл сборки `.pzpack`: автор кладёт его в общую папку или присылает, у получателя программа
//! узнаёт сборку по `pack_id`, видит новую ревизию и ставит моды на клиент в том же порядке.
//! Внутри — zip: `pack.json` (порядок модов, id воркшопа, адрес сервера, ревизия) и `mods/<папка>/…`
//! для локальных модов, которых нет в воркшопе. Обычный JSON без zip тоже читается.
use crate::config::{Profile, ServerCfg};
use crate::mods::Mod;
use crate::packs::{self, PackFile, PackServer};
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const EXT: &str = "pzpack";

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// Новый постоянный id сборки
pub fn new_id(name: &str) -> String {
    let n = std::time::SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:016x}{:08x}", fnv(&format!("{name}{n}")), (n as u64 ^ std::process::id() as u64) as u32)
}

/// Адрес и порты для файла сборки. None, если ничего не задано
pub fn server_info(srv: &ServerCfg) -> Option<PackServer> {
    let ini = srv.ini();
    let port = crate::ini::read_key(&ini, "DefaultPort").unwrap_or_default();
    let udp = crate::ini::read_key(&ini, "UDPPort").unwrap_or_default();
    if srv.public_host.trim().is_empty() && port.is_empty() {
        return None;
    }
    Some(PackServer { host: srv.public_host.trim().to_string(), port, udp_port: udp, name: srv.name.clone() })
}

/// Хеш того, что попадает получателю
pub fn content_hash(p: &Profile, server: &Option<PackServer>) -> String {
    let s = format!("{}\u{1}{}\u{1}{}\u{1}{:?}", p.name, p.notes, p.enabled.join("\u{2}"), server);
    format!("{:016x}", fnv(&s))
}

/// Присваивает id и поднимает ревизию, если содержимое изменилось. true — ревизия новая
pub fn bump_if_changed(p: &mut Profile, server: &Option<PackServer>) -> bool {
    let mut changed = false;
    if p.pack_id.is_empty() {
        p.pack_id = new_id(&p.name);
        changed = true;
    }
    let h = content_hash(p, server);
    if h != p.last_hash {
        p.last_hash = h;
        p.revision += 1;
        changed = true;
    }
    changed
}

/// Запись `.pzpack` через временный файл: получатель не увидит недописанный архив
pub fn write_pack(dest: &Path, p: &Profile, installed: &HashMap<String, Mod>, roots: &[PathBuf], ws_of: &dyn Fn(&Mod) -> Option<String>, server: Option<PackServer>) -> Result<packs::ZipReport> {
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).ok();
    }
    let tmp = dest.with_extension("pzpack.tmp");
    let rep = packs::build_player_zip(&tmp, p, installed, roots, ws_of, crate::i18n::get() == crate::i18n::Lang::En, server)?;
    std::fs::rename(&tmp, dest).with_context(|| tf!("не удалось заменить {}", dest.display()))?;
    Ok(rep)
}

fn is_zip(path: &Path) -> Result<bool> {
    let mut head = [0u8; 2];
    let n = std::fs::File::open(path).with_context(|| tf!("не удалось прочитать {}", path.display()))?.read(&mut head)?;
    Ok(n == 2 && &head == b"PK")
}

/// Читает `.pzpack` (zip) или обычный JSON. Из архива берётся только pack.json: моды внутри не читаются
pub fn read_pack(path: &Path) -> Result<PackFile> {
    if is_zip(path)? {
        let mut z = zip::ZipArchive::new(std::fs::File::open(path)?).context(crate::i18n::t("архив сборки повреждён"))?;
        let mut s = String::new();
        z.by_name("pack.json").context(crate::i18n::t("в архиве нет pack.json"))?.read_to_string(&mut s)?;
        packs::from_json(&s)
    } else {
        let text = std::fs::read_to_string(path).with_context(|| tf!("не удалось прочитать {}", path.display()))?;
        packs::from_json(&text)
    }
}

/// Быстрая проверка файла: (pack_id, ревизия, время изменения)
pub fn peek(path: &Path) -> Option<(String, u64, SystemTime)> {
    let mt = std::fs::metadata(path).ok()?.modified().ok()?;
    let f = read_pack(path).ok()?;
    Some((f.pack_id, f.revision, mt))
}

/// Распаковывает локальные моды из архива в папку модов клиента. Возвращает число папок
pub fn extract_local(path: &Path, dest_mods: &Path) -> Result<usize> {
    if !is_zip(path)? {
        return Ok(0);
    }
    let mut z = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let mut tops = std::collections::BTreeSet::new();
    for i in 0..z.len() {
        let mut e = z.by_index(i)?;
        // enclosed_name отсекает «../» и абсолютные пути
        let Some(name) = e.enclosed_name() else { continue };
        let Ok(rel) = name.strip_prefix("mods") else { continue };
        let Some(top) = rel.components().next() else { continue };
        tops.insert(top.as_os_str().to_string_lossy().into_owned());
        let out = dest_mods.join(rel);
        if e.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(d) = out.parent() {
                std::fs::create_dir_all(d)?;
            }
            let mut f = std::fs::File::create(&out)?;
            std::io::copy(&mut e, &mut f)?;
        }
    }
    Ok(tops.len())
}

/// Что изменилось между тем, что у получателя, и файлом
pub fn describe(old: &[String], f: &PackFile) -> packs::Diff {
    let new: Vec<String> = f.mods.iter().map(|m| m.id.clone()).collect();
    packs::diff(old, &new)
}

pub fn check_ext(path: &Path) -> Result<()> {
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case(EXT) || e.eq_ignore_ascii_case("json") => Ok(()),
        _ => bail!("{}", tf!("ожидался файл .{}", EXT)),
    }
}
