//! Как запускать сервер: команда и аргументы. Сам запуск и управление процессом — runtime.rs
use crate::config::ServerCfg;
use std::path::PathBuf;

/// Что именно запускаем
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// Общие аргументы игры
pub fn game_args(s: &ServerCfg) -> Vec<String> {
    let mut a = vec![];
    if s.nosteam {
        a.push("-nosteam".to_string());
    }
    a.push(format!("-cachedir={}", s.cache_dir.display()));
    a.push("-servername".into());
    a.push(s.name.clone());
    a.extend(s.extra_args.split_whitespace().map(str::to_string));
    a
}

/// windows=true: напрямую java (как ProjectZomboidServer.bat), иначе start-server.sh
pub fn plan_for(s: &ServerCfg, windows: bool) -> Option<Plan> {
    let agent = s.mods_dir().join("ZombieBuddy/libs/ZombieBuddy.jar");
    if windows {
        let java = s.game_dir.join("jre64/bin").join("java.exe");
        if !java.exists() || !s.game_dir.join("projectzomboid.jar").exists() {
            return None;
        }
        let mut args: Vec<String> = [
            "--enable-native-access=ALL-UNNAMED",
            "--add-exports=java.base/jdk.internal.misc=ALL-UNNAMED",
            "-XX:+UseZGC",
            "-XX:-CreateCoredumpOnCrash",
            "-XX:-OmitStackTraceInFastThrow",
        ]
        .iter()
        .map(|x| x.to_string())
        .collect();
        if agent.is_file() {
            args.push(format!("-javaagent:{}", agent.display()));
        }
        args.push(format!("-Xmx{}m", s.xmx_mb.max(1024)));
        args.push("-Djava.library.path=./natives/;./natives/win64/;./".into());
        args.push("-cp".into());
        args.push("./;projectzomboid.jar".into());
        args.push("zombie.network.GameServer".into());
        args.extend(game_args(s));
        Some(Plan { program: java, args })
    } else {
        let sh = s.game_dir.join("start-server.sh");
        sh.exists().then(|| {
            let mut args = vec![];
            if agent.is_file() {
                args.push(format!("-javaagent:{}", agent.display()));
                args.push("--".into());
            }
            args.extend(game_args(s));
            Plan { program: sh, args }
        })
    }
}

pub fn launch_plan(s: &ServerCfg) -> Option<Plan> {
    plan_for(s, cfg!(windows))
}
