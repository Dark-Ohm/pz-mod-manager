//! Различия Linux / Windows в одном месте.
use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// Путь для показа: домашняя папка заменяется на `~`
pub fn short(p: &Path) -> String {
    match p.strip_prefix(home()) {
        Ok(rest) if !home().as_os_str().is_empty() => format!("~/{}", rest.display()).trim_end_matches('/').to_string(),
        _ => p.display().to_string(),
    }
}

pub fn steam_root() -> PathBuf {
    if cfg!(windows) { PathBuf::from(r"C:\Program Files (x86)\Steam") } else { home().join(".local/share/Steam") }
}

/// ~/Zomboid  (на Windows C:\Users\<имя>\Zomboid)
pub fn zomboid_user_dir() -> PathBuf {
    home().join("Zomboid")
}

pub fn client_workshop_dir() -> PathBuf {
    steam_root().join("steamapps/workshop/content/108600")
}

/// Папка с сервером: на Linux «Project Zomboid Dedicated Server», на Windows — сама игра
pub fn default_game_dir() -> PathBuf {
    let common = steam_root().join("steamapps/common");
    if cfg!(windows) { common.join("ProjectZomboid") } else { common.join("Project Zomboid Dedicated Server") }
}

/// Установленный клиент игры (Steam): на Linux внутри лежит projectzomboid.sh
pub fn default_client_dir() -> PathBuf {
    steam_root().join("steamapps/common/ProjectZomboid")
}

pub fn java_bin(game_dir: &Path) -> PathBuf {
    game_dir.join("jre64/bin").join(if cfg!(windows) { "java.exe" } else { "java" })
}

pub fn cp_sep() -> &'static str {
    if cfg!(windows) { ";" } else { ":" }
}

/// Classpath игры: projectzomboid.jar и/или java/*
pub fn game_classpath(game_dir: &Path) -> String {
    let mut parts: Vec<String> = vec![];
    for p in [game_dir.join("projectzomboid.jar"), game_dir.join("java").join("*")] {
        let probe = if p.ends_with("*") { p.parent().unwrap().to_path_buf() } else { p.clone() };
        if probe.exists() {
            parts.push(p.to_string_lossy().into_owned());
        }
    }
    parts.join(cp_sep())
}

pub fn steamcmd_url() -> &'static str {
    if cfg!(windows) {
        "https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip"
    } else {
        "https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz"
    }
}

pub fn steamcmd_bin() -> &'static str {
    if cfg!(windows) { "steamcmd.exe" } else { "steamcmd.sh" }
}

/// Windows: не показывать чёрное консольное окно у дочерних процессов (java, steamcmd)
pub fn hide_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Путь → file:// URI для загрузчика картинок: `/home/a b` → `file:///home/a b`, `C:\x\m.png` → `file:///C:/x/m.png`
pub fn file_uri(path: &str) -> String {
    let p = path.replace('\\', "/");
    if p.starts_with('/') { format!("file://{p}") } else { format!("file:///{p}") }
}
