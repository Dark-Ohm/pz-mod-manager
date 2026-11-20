//! Launcher settings and the places this machine keeps Project Zomboid,
//! pz-mod-manager and the launcher's own files. Nothing here touches the game's
//! default `~/Zomboid` folder: the launcher uses its own cachedir.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// pz-mod-manager binary: pack info/install and update checks run through it.
    pub manager: String,
    /// The .pzpack currently deployed.
    pub pack_file: String,
    /// Optional manifest.json (GitHub release asset or CDN): revision + download URL.
    pub manifest_url: String,
    pub game_dir: String,
    pub cachedir: String,
    /// Environment id from pz-mod-manager, e.g. "10-years-later".
    pub env: String,
    pub host: String,
    pub port: u16,
    pub auto_check: bool,
    pub close_on_launch: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            manager: detect_manager().unwrap_or_default(),
            pack_file: String::new(),
            manifest_url: String::new(),
            game_dir: detect_game_dir().unwrap_or_default(),
            cachedir: default_cachedir(),
            env: "10-years-later".to_string(),
            host: String::new(),
            port: 16261,
            auto_check: true,
            close_on_launch: false,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into());
        std::fs::write(path, text)
    }

    /// Fill anything left empty on first run.
    pub fn normalize(&mut self) {
        if self.manager.is_empty() {
            self.manager = detect_manager().unwrap_or_default();
        }
        if self.game_dir.is_empty() {
            self.game_dir = detect_game_dir().unwrap_or_default();
        }
        if self.cachedir.is_empty() {
            self.cachedir = default_cachedir();
        }
        if self.env.is_empty() {
            self.env = "10-years-later".into();
        }
        if self.port == 0 {
            self.port = 16261;
        }
    }

    pub fn connect(&self) -> String {
        let host = self.host.trim();
        if host.is_empty() {
            String::new()
        } else {
            format!("{host}:{}", self.port)
        }
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pz-launcher")
}

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("pz-launcher")
}

fn default_cachedir() -> String {
    data_dir().join("client").display().to_string()
}

/// `ProjectZomboid64` in the Steam library.
pub fn game_bin(dir: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ProjectZomboid64.exe"
    } else {
        "ProjectZomboid64"
    };
    let path = dir.join(name);
    path.is_file().then_some(path)
}

pub fn detect_game_dir() -> Option<String> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    #[cfg(windows)]
    {
        if let Some(pf) = std::env::var_os("ProgramFiles(x86)") {
            candidates.push(PathBuf::from(pf).join("Steam/steamapps/common/ProjectZomboid"));
        }
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            candidates.push(PathBuf::from(pf).join("Steam/steamapps/common/ProjectZomboid"));
        }
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("Steam/steamapps/common/ProjectZomboid"));
        }
    }

    #[cfg(not(windows))]
    {
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".local/share/Steam/steamapps/common/ProjectZomboid"));
            candidates.push(home.join(".steam/steam/steamapps/common/ProjectZomboid"));
            candidates.push(home.join("Steam/steamapps/common/ProjectZomboid"));
        }
    }

    candidates
        .into_iter()
        .find(|dir| game_bin(dir).is_some())
        .map(|dir| dir.display().to_string())
}

/// Look in the launcher's own folder (release downloads put both files there),
/// then around the checkout, then on PATH.
pub fn detect_manager() -> Option<String> {
    let names: [&str; 2] = if cfg!(windows) {
        ["pz-mod-manager.exe", "pz-mod-manager"]
    } else {
        ["pz-mod-manager", "pz-mod-manager.exe"]
    };

    let mut dirs_to_try: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut at = exe.parent();
        for _ in 0..5 {
            if let Some(dir) = at {
                // Both release downloads unpacked into one folder.
                dirs_to_try.push(dir.to_path_buf());
                dirs_to_try.push(dir.join("pz-manager/target/release"));
                at = dir.parent();
            }
        }
    }
    dirs_to_try.push(PathBuf::from("../pz-manager/target/release"));
    dirs_to_try.push(PathBuf::from("../../pz-manager/target/release"));

    for dir in &dirs_to_try {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate.display().to_string());
            }
        }
    }

    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            for name in names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate.display().to_string());
                }
            }
        }
    }
    None
}
