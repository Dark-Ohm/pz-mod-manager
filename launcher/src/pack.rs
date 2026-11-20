//! The two things the launcher needs to know about a pack: what pz-mod-manager
//! says is installed (`pack info`), and what the release manifest offers.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug, Default)]
pub struct PackInfo {
    pub revision: u64,
    #[allow(dead_code)]
    pub pack_id: String,
    pub mods_total: usize,
    pub mods_installed: usize,
    pub missing: Vec<String>,
    pub server: String,
    pub profile: String,
}

impl PackInfo {
    pub fn complete(&self) -> bool {
        self.mods_total > 0 && self.missing.is_empty() && self.mods_installed >= self.mods_total
    }
}

/// Windows: do not flash a console window for every manager call.
pub fn manager_command(path: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = cmd.creation_flags(0x0800_0000);
    }
    cmd
}

pub fn info(manager: &str, pack: &Path, env: &str) -> Result<PackInfo> {
    let mut cmd = manager_command(manager);
    cmd.arg("pack").arg("info").arg(pack);
    if !env.trim().is_empty() {
        cmd.arg("--env").arg(env);
    }
    let out = cmd
        .output()
        .with_context(|| format!("не удалось запустить {manager}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let json =
        slice_json(&text).with_context(|| format!("менеджер не вернул JSON: {}", tail(&text)))?;
    let value: Value = serde_json::from_str(json).context("ответ менеджера не разобрался")?;
    if value["ok"].as_bool() != Some(true) {
        anyhow::bail!(
            "{}",
            value["error"].as_str().unwrap_or("менеджер вернул ошибку")
        );
    }
    let r = &value["result"];
    let mods = r["mods"].as_array().cloned().unwrap_or_default();
    let mods_total = mods.len();
    let mods_installed = mods
        .iter()
        .filter(|m| m["installed"].as_bool().unwrap_or(true))
        .count();
    let missing = r["missing_workshop_items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let server = r["server"]["connect"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            let host = r["server"]["host"].as_str()?;
            let port = r["server"]["port"].as_str().unwrap_or("16261");
            Some(format!("{host}:{port}"))
        })
        .unwrap_or_default();
    Ok(PackInfo {
        revision: r["revision"].as_u64().unwrap_or(0),
        pack_id: r["pack_id"].as_str().unwrap_or("").to_string(),
        mods_total,
        mods_installed,
        missing,
        server,
        profile: r["known_profile"]
            .as_str()
            .or_else(|| r["profile"].as_str())
            .unwrap_or("")
            .to_string(),
    })
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub pack_id: Option<String>,
    pub revision: Option<u64>,
    pub changelog: Option<String>,
    pub server: Option<ManifestServer>,
    pub pack: Option<ManifestFile>,
    pub file: Option<ManifestFile>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ManifestServer {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub connect: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ManifestFile {
    pub url: String,
    pub sha256: Option<String>,
    pub size: Option<u64>,
}

impl Manifest {
    pub fn revision(&self) -> u64 {
        self.revision.unwrap_or(0)
    }

    pub fn is_newer_than(&self, installed: u64) -> bool {
        self.revision() > installed
    }

    pub fn download_url(&self) -> Option<&str> {
        self.pack
            .as_ref()
            .map(|f| f.url.as_str())
            .filter(|u| !u.is_empty())
            .or_else(|| {
                self.file
                    .as_ref()
                    .map(|f| f.url.as_str())
                    .filter(|u| !u.is_empty())
            })
            .or(self.url.as_deref().filter(|u| !u.is_empty()))
    }

    pub fn sha256(&self) -> Option<String> {
        self.pack
            .as_ref()
            .and_then(|f| f.sha256.clone())
            .or_else(|| self.file.as_ref().and_then(|f| f.sha256.clone()))
            .or_else(|| self.sha256.clone())
    }

    pub fn declared_size(&self) -> Option<u64> {
        self.pack
            .as_ref()
            .and_then(|f| f.size)
            .or_else(|| self.file.as_ref().and_then(|f| f.size))
            .or(self.size)
    }

    pub fn connect(&self) -> Option<String> {
        let server = self.server.as_ref()?;
        if let Some(connect) = server.connect.as_ref().filter(|c| !c.is_empty()) {
            return Some(connect.clone());
        }
        let host = server.host.as_deref().filter(|h| !h.is_empty())?;
        Some(format!("{host}:{}", server.port.unwrap_or(16261)))
    }

    pub fn download_name(&self) -> String {
        format!("rev-{}.pzpack", self.revision())
    }
}

pub fn fetch(url: &str) -> Result<Manifest> {
    let response = ureq::get(url)
        .header("Accept", "application/json")
        .call()
        .with_context(|| format!("не удалось получить {url}"))?;
    let body = response
        .into_body()
        .read_to_string()
        .context("манифест оборвался при чтении")?;
    let manifest: Manifest =
        serde_json::from_str(&body).context("манифест не разобрался как JSON")?;
    if manifest.revision.is_none() {
        anyhow::bail!("в манифесте нет поля revision");
    }
    Ok(manifest)
}

fn slice_json(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then_some(&text[start..=end])
}

fn tail(text: &str) -> String {
    let trimmed = text.trim();
    let keep: String = trimmed
        .chars()
        .rev()
        .take(400)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_url_and_revision_are_flexible() {
        let m: Manifest = serde_json::from_str(
            r#"{"revision":7,"pack":{"url":"https://cdn/x.pzpack","sha256":"AB"},"server":{"host":"1.2.3.4","port":27016}}"#,
        )
        .unwrap();
        assert_eq!(m.revision(), 7);
        assert_eq!(m.download_url(), Some("https://cdn/x.pzpack"));
        assert_eq!(m.sha256().as_deref(), Some("AB"));
        assert_eq!(m.connect().as_deref(), Some("1.2.3.4:27016"));
        assert!(m.is_newer_than(6));
        assert!(!m.is_newer_than(7));
    }

    #[test]
    fn flat_manifest_with_url_is_accepted() {
        let m: Manifest =
            serde_json::from_str(r#"{"revision":3,"url":"https://cdn/p.pzpack","size":10}"#)
                .unwrap();
        assert_eq!(m.download_url(), Some("https://cdn/p.pzpack"));
        assert_eq!(m.declared_size(), Some(10));
        assert_eq!(m.connect(), None);
    }

    #[test]
    fn json_is_sliced_out_of_noisy_output() {
        assert_eq!(
            slice_json("log line\n{\"ok\":true}\n"),
            Some("{\"ok\":true}")
        );
    }
}
