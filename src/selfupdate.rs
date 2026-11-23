//! Самообновление: сборка «nightly» на GitHub пересоздаётся на каждый пуш в master
//! (см. .github/workflows/build.yml), в её описании первой строкой `commit: <sha>`.
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::PathBuf;

const API: &str = "https://api.github.com/repos/Dark-Ohm/pz-mod-manager/releases/tags/nightly";
pub const COMMIT: &str = env!("PZ_COMMIT");

#[cfg(windows)]
const ASSET: &str = "pz-mod-manager-windows-x64.exe";
#[cfg(not(windows))]
const ASSET: &str = "pz-mod-manager-linux-x64";

#[derive(Clone, Debug)]
pub struct Info {
    pub sha: String,
    pub notes: String,
    pub url: String,
    pub newer: bool,
}

#[derive(Deserialize)]
struct Release {
    body: Option<String>,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub fn check() -> Result<Info> {
    let r: String = ureq::get(API)
        .header("User-Agent", "pz-mod-manager")
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("GitHub недоступен или сборки ещё нет")?
        .body_mut()
        .read_to_string()?;
    let r: Release = serde_json::from_str(&r)?;
    let body = r.body.unwrap_or_default();
    let sha = body.lines().find_map(|l| l.strip_prefix("commit:")).map(|s| s.trim().to_string()).context("в описании сборки нет коммита")?;
    let url = r.assets.into_iter().find(|a| a.name == ASSET).map(|a| a.browser_download_url).with_context(|| format!("в сборке нет файла {ASSET}"))?;
    let notes = body.lines().filter(|l| !l.starts_with("commit:")).collect::<Vec<_>>().join("\n").trim().to_string();
    Ok(Info { newer: sha != COMMIT, sha, notes, url })
}

/// Скачивает новую сборку и подменяет ею запущенный файл; перезапуск делает вызывающий
pub fn apply(info: &Info) -> Result<PathBuf> {
    if !info.url.starts_with("https://github.com/Dark-Ohm/pz-mod-manager/") {
        bail!("неожиданный адрес загрузки: {}", info.url);
    }
    let exe = std::env::current_exe()?;
    let new = exe.with_extension("new");
    let mut resp = ureq::get(&info.url).header("User-Agent", "pz-mod-manager").call().context("загрузка не удалась")?;
    let mut f = std::fs::File::create(&new)?;
    let n = std::io::copy(&mut resp.body_mut().as_reader(), &mut f)?;
    drop(f);
    if n < 1_000_000 {
        let _ = std::fs::remove_file(&new);
        bail!("скачано подозрительно мало ({n} байт)");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(windows)]
    {
        // запущенный exe нельзя перезаписать, но можно переименовать
        let old = exe.with_extension("old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old)?;
    }
    std::fs::rename(&new, &exe).context("не удалось заменить файл программы")?;
    Ok(exe)
}
