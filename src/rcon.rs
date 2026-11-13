//! Команды серверу. Два пути:
//! 1) сервер запущен менеджером → пишем прямо в его консоль (stdin);
//! 2) иначе — встроенный RCONClient игры (сырой Source RCON PZ не понимает).
use crate::i18n::t;
use crate::{ini, platform};
use anyhow::{Result, bail};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct Rcon {
    pub port: u16,
    pub password: String,
}

/// RCONPort / RCONPassword из ini сервера
pub fn from_ini(ini_path: &Path) -> Option<Rcon> {
    let text = std::fs::read_to_string(ini_path).ok()?;
    let password = ini::get(&text, "RCONPassword").unwrap_or_default();
    if password.is_empty() {
        return None;
    }
    let port = ini::get(&text, "RCONPort").and_then(|p| p.parse().ok()).unwrap_or(27015);
    Some(Rcon { port, password })
}

pub fn client_args(game_dir: &Path, r: &Rcon, command: &str) -> Vec<String> {
    vec![
        "-cp".into(),
        platform::game_classpath(game_dir),
        "zombie.network.RCONClient".into(),
        "-ip".into(),
        "127.0.0.1".into(),
        "-port".into(),
        r.port.to_string(),
        "-password".into(),
        r.password.clone(),
        "-command".into(),
        command.to_string(),
    ]
}

pub fn run(game_dir: &Path, r: &Rcon, command: &str) -> Result<String> {
    crate::validate::command(command).map_err(anyhow::Error::msg)?;
    let java = platform::java_bin(game_dir);
    if !java.exists() {
        bail!("{}", tf!("не найден {}", java.display()));
    }
    let mut cmd = Command::new(java);
    cmd.args(client_args(game_dir, r, command)).current_dir(game_dir);
    platform::hide_window(&mut cmd);
    let out = cmd.output()?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        bail!("RCON: {}", s.trim());
    }
    Ok(s.trim().to_string())
}
