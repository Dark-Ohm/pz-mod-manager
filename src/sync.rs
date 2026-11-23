//! Сверка клиента и сервера: список модов и порядок загрузки (ini сервера и default.txt
//! клиента) и набор папок в mods. Игра в -nosteam берёт моды клиента из default.txt.
use crate::config::{ServerCfg, Settings};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub ini_exists: bool,
    pub client_file: PathBuf,
    pub client_file_exists: bool,
    pub server_count: usize,
    pub client_count: usize,
    /// есть на сервере, нет в списке клиента
    pub missing_on_client: Vec<String>,
    /// есть в списке клиента, нет на сервере
    pub extra_on_client: Vec<String>,
    /// первая позиция, где порядок расходится (по общим модам): (позиция с 1, сервер, клиент)
    pub first_order_diff: Option<(usize, String, String)>,
    /// папки есть только на сервере / только у клиента
    pub folders_only_server: Vec<String>,
    pub folders_only_client: Vec<String>,
}

impl Report {
    pub fn in_sync(&self) -> bool {
        self.ini_exists
            && self.client_file_exists
            && self.missing_on_client.is_empty()
            && self.extra_on_client.is_empty()
            && self.first_order_diff.is_none()
            && self.folders_only_server.is_empty()
            && self.folders_only_client.is_empty()
    }
}

pub fn client_file(s: &Settings, srv: &ServerCfg) -> PathBuf {
    srv.client_mods_dir(s).join("default.txt")
}

/// `mod = X,` из секции mods в default.txt (без `\`)
pub fn read_client_list(text: &str) -> Vec<String> {
    let mut in_mods = false;
    let mut out = vec![];
    for l in text.lines() {
        let t = l.trim();
        if t == "mods" {
            in_mods = true;
        } else if t == "maps" {
            in_mods = false;
        } else if in_mods {
            if let Some(v) = t.strip_prefix("mod") {
                if let Some(v) = v.trim_start().strip_prefix('=') {
                    let id = v.trim().trim_end_matches(',').trim().trim_start_matches('\\');
                    if !id.is_empty() {
                        out.push(id.to_string());
                    }
                }
            }
        }
    }
    out
}

pub fn client_list_text(ids: &[String]) -> String {
    let mut s = String::from("VERSION = 1,\n\nmods\n{\n");
    for id in ids {
        s.push_str(&format!("    mod = {id},\n"));
    }
    s.push_str("}\n\nmaps\n{\n}\n");
    s
}

fn folders(dir: &std::path::Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .map(|r| r.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

pub fn compare(server: &[String], client: &[String]) -> (Vec<String>, Vec<String>, Option<(usize, String, String)>) {
    let sset: BTreeSet<&String> = server.iter().collect();
    let cset: BTreeSet<&String> = client.iter().collect();
    let missing = server.iter().filter(|i| !cset.contains(i)).cloned().collect();
    let extra = client.iter().filter(|i| !sset.contains(i)).cloned().collect();
    // порядок: сравниваем только общие моды
    let a: Vec<&String> = server.iter().filter(|i| cset.contains(i)).collect();
    let b: Vec<&String> = client.iter().filter(|i| sset.contains(i)).collect();
    let diff = a.iter().zip(b.iter()).position(|(x, y)| x != y).map(|p| (p + 1, a[p].clone(), b[p].clone()));
    (missing, extra, diff)
}

pub fn check(s: &Settings, srv: &ServerCfg) -> Report {
    let ini = srv.ini();
    let (server, _) = crate::export::read_ini_mods(&ini);
    let cf = client_file(s, srv);
    let text = std::fs::read_to_string(&cf).ok();
    let client = text.as_deref().map(read_client_list).unwrap_or_default();
    let (missing_on_client, extra_on_client, first_order_diff) = compare(&server, &client);
    let (fs_, fc) = (folders(&srv.mods_dir()), folders(&srv.client_mods_dir(s)));
    Report {
        ini_exists: ini.exists(),
        client_file_exists: text.is_some(),
        client_file: cf,
        server_count: server.len(),
        client_count: client.len(),
        missing_on_client,
        extra_on_client,
        first_order_diff,
        folders_only_server: fs_.difference(&fc).cloned().collect(),
        folders_only_client: fc.difference(&fs_).cloned().collect(),
    }
}

/// Записывает список клиента из ini сервера (порядок как на сервере)
pub fn apply(s: &Settings, srv: &ServerCfg) -> anyhow::Result<usize> {
    let (server, _) = crate::export::read_ini_mods(&srv.ini());
    crate::atomic::write(&client_file(s, srv), client_list_text(&server).as_bytes())?;
    Ok(server.len())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    OnlyServer,
    OnlyClient,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Do {
    Skip,
    Copy,
    Remove,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub name: String,
    pub side: Side,
    pub act: Do,
}

/// Расхождения по папкам; по умолчанию предлагается перенести
pub fn items(rep: &Report) -> Vec<Item> {
    let mut v: Vec<Item> = rep.folders_only_server.iter().map(|n| Item { name: n.clone(), side: Side::OnlyServer, act: Do::Copy }).collect();
    v.extend(rep.folders_only_client.iter().map(|n| Item { name: n.clone(), side: Side::OnlyClient, act: Do::Copy }));
    v
}

pub fn trash_dir(srv: &ServerCfg) -> PathBuf {
    srv.cache_dir.join("backups").join("mods-removed")
}

/// Папку не стираем, а переносим в `<cachedir>/backups/mods-removed/<время>__<имя>`
pub fn move_to_trash(srv: &ServerCfg, folder: &std::path::Path) -> anyhow::Result<PathBuf> {
    let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dst = trash_dir(srv).join(format!("{}__{name}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p)?;
    }
    if std::fs::rename(folder, &dst).is_err() {
        crate::backup::copy_dir(folder, &dst)?;
        std::fs::remove_dir_all(folder)?;
    }
    Ok(dst)
}

/// Выполняет выбранное. Возвращает итоговую строку.
pub fn run(s: &Settings, srv: &ServerCfg, items: &[Item], write_list: bool) -> String {
    let (sd, cd) = (srv.mods_dir(), srv.client_mods_dir(s));
    let (mut copied, mut removed, mut errs) = (0, 0, vec![]);
    for it in items {
        let (own, other) = match it.side {
            Side::OnlyServer => (&sd, &cd),
            Side::OnlyClient => (&cd, &sd),
        };
        let r = match it.act {
            Do::Skip => continue,
            Do::Copy => crate::backup::copy_dir(&own.join(&it.name), &other.join(&it.name)).map(|_| copied += 1),
            Do::Remove => move_to_trash(srv, &own.join(&it.name)).map(|_| removed += 1),
        };
        if let Err(e) = r {
            errs.push(format!("{}: {e:#}", it.name));
        }
    }
    let mut out = format!("Синхронизация: перенесено {copied}, убрано {removed}");
    if write_list {
        match apply(s, srv) {
            Ok(n) => out.push_str(&format!(", список клиента записан ({n})")),
            Err(e) => errs.push(format!("список клиента: {e:#}")),
        }
    }
    if !errs.is_empty() {
        out.push_str(&format!(" · ошибки: {}", errs.join("; ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_compares() {
        let t = client_list_text(&["A".into(), "B".into(), "C".into()]);
        assert_eq!(read_client_list(&t), ["A", "B", "C"]);
        let s: Vec<String> = ["A", "B", "C"].map(String::from).to_vec();
        let c: Vec<String> = ["A", "C", "B", "X"].map(String::from).to_vec();
        let (m, e, d) = compare(&s, &c);
        assert!(m.is_empty());
        assert_eq!(e, ["X"]);
        assert_eq!(d, Some((2, "B".into(), "C".into())));
        assert!(compare(&s, &s).2.is_none());
    }
}
