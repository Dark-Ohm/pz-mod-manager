//! Process control for the server and client of an environment, shared by the GUI, the CLI and MCP.
//!
//! Everything lives on disk so every frontend (and every agent) sees the same state:
//!   <data_dir>/run/<key>/server.pid, server.log, server.stdin (FIFO, unix only)
//!   <data_dir>/run/<key>/client.pid, client.log
//! `<key>` is derived from the cachedir, so two environments never share run files.
//!
//! Detection does not rely on our pid files alone: on Linux /proc is scanned for any process
//! started with the same `-cachedir=`, so a server or client started by scripts/pz.sh, by hand
//! or by another manager instance is seen as well.
//!
//! The server console reads stdin and treats EOF as "console closed" (GameServer.launchCommandHandler),
//! so on unix the server gets a FIFO opened read-write: it never sees EOF and anyone can write
//! a command into it later. On Windows commands go through RCON.
use crate::config::{ServerCfg, Settings};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Marker printed by GameServer.startServer() once the UDP engine is up
pub const READY_MARKER: &str = "*** SERVER STARTED";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Server,
    Client,
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Role::Server => "server",
            Role::Client => "client",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub role: Role,
    pub running: bool,
    /// all processes started with this cachedir (launcher script + game)
    pub pids: Vec<u32>,
    /// started by the manager (pid file points to a live process)
    pub managed: bool,
    pub cache_dir: PathBuf,
    /// server only: the ready marker is in the log of the current run
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready: Option<bool>,
    /// log that belongs to the current/last run
    pub log: PathBuf,
    /// server only: the console FIFO exists and the server can take commands through it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub console: Option<bool>,
}

/// FNV-1a: stable across Rust versions (DefaultHasher is not)
fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn norm(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

pub fn run_dir(cache_dir: &Path) -> PathBuf {
    let n = norm(cache_dir);
    let tail: String = n
        .file_name()
        .map(|s| s.to_string_lossy().chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect())
        .unwrap_or_default();
    crate::config::data_dir().join("run").join(format!("{tail}-{:016x}", fnv(&n.to_string_lossy())))
}

fn pid_file(cache_dir: &Path, role: Role) -> PathBuf {
    run_dir(cache_dir).join(format!("{}.pid", role.name()))
}

/// When the manager started this process (pid file time); None if it was not started by the manager
pub fn started_at(cache_dir: &Path, role: Role) -> Option<std::time::SystemTime> {
    let p = pid_file(cache_dir, role);
    read_pid(&p).filter(|pid| pid_alive(*pid))?;
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

pub fn log_file(cache_dir: &Path, role: Role) -> PathBuf {
    run_dir(cache_dir).join(format!("{}.log", role.name()))
}

pub fn fifo(cache_dir: &Path) -> PathBuf {
    run_dir(cache_dir).join("server.stdin")
}

fn cache_of(s: &Settings, srv: &ServerCfg, role: Role) -> PathBuf {
    match role {
        Role::Server => srv.cache_dir.clone(),
        Role::Client => srv.client_cache_dir(s),
    }
}

// ---------- process discovery ----------

#[cfg(target_os = "linux")]
fn pids_with_cachedir(cache_dir: &Path) -> Vec<u32> {
    let want: Vec<String> = {
        let mut v = vec![format!("-cachedir={}", cache_dir.display())];
        let n = norm(cache_dir);
        if n != cache_dir {
            v.push(format!("-cachedir={}", n.display()));
        }
        v
    };
    let me = std::process::id();
    let mut out = vec![];
    for e in std::fs::read_dir("/proc").into_iter().flatten().filter_map(|e| e.ok()) {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        if pid == me {
            continue;
        }
        let Ok(raw) = std::fs::read(e.path().join("cmdline")) else { continue };
        let hit = raw.split(|b| *b == 0).any(|a| {
            let a = String::from_utf8_lossy(a);
            let a = a.trim_end_matches('/');
            want.iter().any(|w| a == w.trim_end_matches('/'))
        });
        if hit {
            out.push(pid);
        }
    }
    out.sort();
    out
}

#[cfg(not(target_os = "linux"))]
fn pids_with_cachedir(_cache_dir: &Path) -> Vec<u32> {
    vec![]
}

pub fn pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        // a zombie keeps its /proc entry but has an empty cmdline
        std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| !c.is_empty())
    }
    #[cfg(windows)]
    {
        let mut cmd = Command::new("tasklist");
        cmd.args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"]);
        crate::platform::hide_window(&mut cmd);
        cmd.output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        Command::new("kill").args(["-0", &pid.to_string()]).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }
}

fn read_pid(p: &Path) -> Option<u32> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

pub fn status(s: &Settings, srv: &ServerCfg, role: Role) -> Status {
    let cache = cache_of(s, srv, role);
    let mut pids = pids_with_cachedir(&cache);
    let managed_pid = read_pid(&pid_file(&cache, role)).filter(|p| pid_alive(*p));
    if let Some(p) = managed_pid {
        if !pids.contains(&p) {
            pids.push(p);
        }
    }
    let running = !pids.is_empty();
    let managed = managed_pid.is_some();
    // the manager's own log when we started it (or it is newer), else the game's console log
    let own = log_file(&cache, role);
    let game_log = match role {
        Role::Server => cache.join("server-console.txt"),
        Role::Client => cache.join("console.txt"),
    };
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let log = if managed || (own.exists() && mtime(&own) >= mtime(&game_log)) { own } else { game_log };
    let (ready, console) = match role {
        Role::Server => (
            Some(running && tail_text(&log, 4 << 20).contains(READY_MARKER)),
            Some(running && managed && cfg!(unix) && fifo(&cache).exists()),
        ),
        Role::Client => (None, None),
    };
    Status { role, running, pids, managed, cache_dir: cache, ready, log, console }
}

// ---------- logs ----------

/// Last `max` bytes of a file as text
pub fn tail_text(p: &Path, max: u64) -> String {
    let Ok(mut f) = std::fs::File::open(p) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len > max {
        let _ = f.seek(SeekFrom::Start(len - max));
    }
    let mut buf = vec![];
    let _ = f.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

pub fn tail_lines(p: &Path, n: usize) -> Vec<String> {
    let text = tail_text(p, (n as u64 * 400).max(64 << 10));
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|s| s.to_string()).collect()
}

/// Text appended to a file after byte offset `from`
pub fn read_from(p: &Path, from: u64) -> String {
    let Ok(mut f) = std::fs::File::open(p) else { return String::new() };
    if f.seek(SeekFrom::Start(from)).is_err() {
        return String::new();
    }
    let mut buf = vec![];
    let _ = f.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

pub fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

// ---------- launching ----------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

/// Client launcher: Linux projectzomboid.sh, Windows ProjectZomboid64.exe
pub fn client_plan(s: &Settings, srv: &ServerCfg, nosteam: bool, extra: &[String]) -> Option<Launch> {
    let dir = &s.client_game_dir;
    let program = if cfg!(windows) {
        dir.join("ProjectZomboid64.exe")
    } else {
        [dir.join("projectzomboid.sh"), dir.join("projectzomboid/projectzomboid.sh")].into_iter().find(|p| p.exists())?
    };
    if !program.exists() {
        return None;
    }
    let mut args = vec![];
    if zombiebuddy_present(&program) {
        // JVM arguments go before the first `--`, game arguments after it (pzexe launcher)
        args.push("-javaagent:ZombieBuddy.jar".to_string());
        args.push("--".to_string());
    }
    args.push(format!("-cachedir={}", srv.client_cache_dir(s).display()));
    // Клиент всегда запускается через Steam (часы в профиле), флаг nosteam для него игнорируется.
    let _ = nosteam;
    args.extend(extra.iter().cloned().filter(|a| a != "-nosteam"));
    Some(Launch { cwd: program.parent()?.to_path_buf(), program, args })
}

/// Linux: ZombieBuddy.jar next to the game's jars (where projectzomboid.sh runs the game)
/// means mods that need ZombieBuddy are in use, so the client is started with its Java agent.
fn zombiebuddy_present(launcher: &Path) -> bool {
    if cfg!(windows) {
        return false;
    }
    let Some(dir) = launcher.parent() else { return false };
    dir.join("projectzomboid/ZombieBuddy.jar").is_file() || dir.join("ZombieBuddy.jar").is_file()
}

fn detach(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // own process group: Ctrl+C in the terminal that ran the CLI does not reach the game
        cmd.process_group(0);
    }
    crate::platform::hide_window(cmd);
}

/// Hide `-adminpassword <x>` in anything we print or log
pub fn mask_args(args: &[String]) -> Vec<String> {
    let mut out = vec![];
    let mut hide = false;
    for a in args {
        out.push(if hide { "***".into() } else { a.clone() });
        hide = matches!(a.as_str(), "-adminpassword" | "-password");
    }
    out
}

#[cfg(unix)]
fn make_fifo(p: &Path) -> Result<std::fs::File> {
    let _ = std::fs::remove_file(p);
    let st = Command::new("mkfifo").arg("-m").arg("600").arg(p).status().context("mkfifo")?;
    if !st.success() {
        bail!("mkfifo {} failed", p.display());
    }
    // read+write: opening does not block and the server never gets EOF
    Ok(std::fs::OpenOptions::new().read(true).write(true).open(p)?)
}

pub struct Started {
    pub child: Child,
    pub launch: Launch,
    pub log: PathBuf,
}

/// Start the environment's server detached. `admin_password` is only needed on the very first
/// start (no ini yet): the server would otherwise wait for it on the console.
pub fn start_server(s: &Settings, srv: &ServerCfg, admin_password: Option<&str>) -> Result<Started> {
    crate::validate::server_name(&srv.name).map_err(anyhow::Error::msg)?;
    let st = status(s, srv, Role::Server);
    if st.running {
        bail!("server is already running for {} (pids {:?})", srv.cache_dir.display(), st.pids);
    }
    let first_run = !srv.ini().exists();
    if first_run && admin_password.is_none_or(|p| p.trim().is_empty()) {
        bail!(
            "{} does not exist yet: the first start creates it and needs an admin password (pass admin_password)",
            srv.ini().display()
        );
    }
    let plan = crate::server::launch_plan(srv).with_context(|| format!("no server launcher for this OS in {}", srv.game_dir.display()))?;
    let mut args = plan.args.clone();
    if let Some(p) = admin_password.filter(|p| !p.trim().is_empty()) {
        args.push("-adminpassword".into());
        args.push(p.trim().into());
    }
    std::fs::create_dir_all(&srv.cache_dir)?;
    let rd = run_dir(&srv.cache_dir);
    std::fs::create_dir_all(&rd)?;
    let log = log_file(&srv.cache_dir, Role::Server);
    let mut lf = std::fs::File::create(&log)?;
    {
        use std::io::Write;
        let _ = writeln!(lf, "$ {} {}", plan.program.display(), mask_args(&args).join(" "));
    }
    let mut cmd = Command::new(&plan.program);
    cmd.args(&args).current_dir(&srv.game_dir).stdout(lf.try_clone()?).stderr(lf);
    #[cfg(unix)]
    cmd.stdin(make_fifo(&fifo(&srv.cache_dir))?);
    #[cfg(not(unix))]
    cmd.stdin(Stdio::null());
    detach(&mut cmd);
    let child = cmd.spawn().with_context(|| format!("failed to start {}", plan.program.display()))?;
    std::fs::write(pid_file(&srv.cache_dir, Role::Server), child.id().to_string())?;
    let launch = Launch { program: plan.program, args: mask_args(&args), cwd: srv.game_dir.clone() };
    Ok(Started { child, launch, log })
}

pub fn start_client(s: &Settings, srv: &ServerCfg, nosteam: bool, extra: &[String]) -> Result<Started> {
    let st = status(s, srv, Role::Client);
    if st.running {
        bail!("client is already running for {} (pids {:?})", st.cache_dir.display(), st.pids);
    }
    let cache = srv.client_cache_dir(s);
    let plan = client_plan(s, srv, nosteam, extra).with_context(|| format!("no client launcher found in {}", s.client_game_dir.display()))?;
    std::fs::create_dir_all(&cache)?;
    std::fs::create_dir_all(run_dir(&cache))?;
    let log = log_file(&cache, Role::Client);
    let mut lf = std::fs::File::create(&log)?;
    {
        use std::io::Write;
        let _ = writeln!(lf, "$ {} {}", plan.program.display(), plan.args.join(" "));
    }
    let mut cmd = Command::new(&plan.program);
    cmd.args(&plan.args).current_dir(&plan.cwd).stdin(Stdio::null()).stdout(lf.try_clone()?).stderr(lf);
    detach(&mut cmd);
    let child = cmd.spawn().with_context(|| format!("failed to start {}", plan.program.display()))?;
    std::fs::write(pid_file(&cache, Role::Client), child.id().to_string())?;
    Ok(Started { child, launch: plan, log })
}

// ---------- commands ----------

#[derive(Debug, Serialize)]
pub struct CmdResult {
    /// "rcon" or "console"
    pub via: String,
    /// RCON reply, or console log lines that appeared right after the command
    pub output: String,
}

fn write_fifo(p: &Path, line: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "linux")]
        const O_NONBLOCK: i32 = 0o4000;
        #[cfg(not(target_os = "linux"))]
        const O_NONBLOCK: i32 = 0x4;
        // non-blocking: if nobody holds the read end, fail instead of hanging forever
        let mut f = std::fs::OpenOptions::new().write(true).custom_flags(O_NONBLOCK).open(p).context("console FIFO has no reader")?;
        f.write_all(format!("{line}\n").as_bytes())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (p, line);
        bail!("console FIFO is not available on this OS")
    }
}

/// Send a console command: RCON when configured (it returns a reply), else the console FIFO
/// of a server we started (the reply is whatever the log shows within `wait`).
pub fn send_command(s: &Settings, srv: &ServerCfg, command: &str, wait: Duration) -> Result<CmdResult> {
    crate::validate::command(command).map_err(anyhow::Error::msg)?;
    let st = status(s, srv, Role::Server);
    if !st.running {
        bail!("server is not running");
    }
    if let Some(r) = crate::rcon::from_ini(&srv.ini()) {
        if let Ok(out) = crate::rcon::run(&srv.game_dir, &r, command) {
            return Ok(CmdResult { via: "rcon".into(), output: out });
        }
    }
    if st.console == Some(true) {
        let before = file_len(&st.log);
        write_fifo(&fifo(&srv.cache_dir), command.trim())?;
        std::thread::sleep(wait);
        return Ok(CmdResult { via: "console".into(), output: read_from(&st.log, before) });
    }
    bail!("cannot reach the server: it was not started by the manager (no console) and RCON is not configured or failed (set RCONPassword in the ini)")
}

// ---------- stopping ----------

fn signal(pids: &[u32], force: bool) {
    if pids.is_empty() {
        return;
    }
    #[cfg(unix)]
    {
        let mut cmd = Command::new("kill");
        cmd.arg(if force { "-KILL" } else { "-TERM" });
        for p in pids {
            cmd.arg(p.to_string());
        }
        let _ = cmd.stderr(Stdio::null()).status();
    }
    #[cfg(windows)]
    {
        for p in pids {
            let mut cmd = Command::new("taskkill");
            cmd.args(["/PID", &p.to_string(), "/T"]);
            if force {
                cmd.arg("/F");
            }
            crate::platform::hide_window(&mut cmd);
            let _ = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
    }
}

fn wait_stopped(s: &Settings, srv: &ServerCfg, role: Role, timeout: Duration) -> bool {
    let until = Instant::now() + timeout;
    loop {
        if !status(s, srv, role).running {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[derive(Debug, Serialize)]
pub struct StopResult {
    pub stopped: bool,
    /// what was done: "not-running", "quit", "terminated", "killed"
    pub method: String,
    pub note: String,
}

/// Graceful stop: `quit` (saves the world) and wait; with `force`, terminate what is left.
pub fn stop_server(s: &Settings, srv: &ServerCfg, timeout: Duration, force: bool) -> Result<StopResult> {
    let st = status(s, srv, Role::Server);
    if !st.running {
        cleanup(&srv.cache_dir, Role::Server);
        return Ok(StopResult { stopped: true, method: "not-running".into(), note: String::new() });
    }
    let sent = send_command(s, srv, "quit", Duration::from_millis(200));
    if sent.is_ok() && wait_stopped(s, srv, Role::Server, timeout) {
        cleanup(&srv.cache_dir, Role::Server);
        return Ok(StopResult { stopped: true, method: "quit".into(), note: String::new() });
    }
    let note = match &sent {
        Ok(_) => format!("quit sent but the server is still running after {}s", timeout.as_secs()),
        Err(e) => format!("quit could not be sent: {e:#}"),
    };
    if !force {
        return Ok(StopResult { stopped: false, method: "quit".into(), note: note + " (use force to terminate; the world may not be saved)" });
    }
    let r = kill(s, srv, Role::Server)?;
    Ok(StopResult { note, ..r })
}

/// Terminate, then kill what survives 10 s
pub fn kill(s: &Settings, srv: &ServerCfg, role: Role) -> Result<StopResult> {
    let st = status(s, srv, role);
    if !st.running {
        cleanup(&st.cache_dir, role);
        return Ok(StopResult { stopped: true, method: "not-running".into(), note: String::new() });
    }
    signal(&st.pids, false);
    if wait_stopped(s, srv, role, Duration::from_secs(10)) {
        cleanup(&st.cache_dir, role);
        return Ok(StopResult { stopped: true, method: "terminated".into(), note: String::new() });
    }
    signal(&status(s, srv, role).pids, true);
    let stopped = wait_stopped(s, srv, role, Duration::from_secs(5));
    if stopped {
        cleanup(&st.cache_dir, role);
    }
    Ok(StopResult { stopped, method: "killed".into(), note: String::new() })
}

fn cleanup(cache: &Path, role: Role) {
    let _ = std::fs::remove_file(pid_file(cache, role));
    if role == Role::Server {
        let _ = std::fs::remove_file(fifo(cache));
    }
}

/// Wait until the server prints the ready marker (or exits)
pub fn wait_ready(s: &Settings, srv: &ServerCfg, timeout: Duration) -> Status {
    let until = Instant::now() + timeout;
    loop {
        let st = status(s, srv, Role::Server);
        if st.ready == Some(true) || !st.running || Instant::now() >= until {
            return st;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn client_plan_adds_zombiebuddy_agent_before_game_args() {
        let d = std::env::temp_dir().join(format!("pzmm-zb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("projectzomboid")).unwrap();
        std::fs::write(d.join("projectzomboid.sh"), "").unwrap();
        let mut s = Settings::default();
        s.client_game_dir = d.clone();
        let srv = ServerCfg::default();
        let plain = client_plan(&s, &srv, true, &[]).unwrap();
        assert!(plain.args[0].starts_with("-cachedir="));
        std::fs::write(d.join("projectzomboid/ZombieBuddy.jar"), "").unwrap();
        let zb = client_plan(&s, &srv, true, &[]).unwrap();
        assert_eq!(&zb.args[..2], ["-javaagent:ZombieBuddy.jar", "--"]);
        assert!(zb.args[2].starts_with("-cachedir="));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn run_dir_is_stable_and_distinct() {
        let a = run_dir(Path::new("/x/pz-server"));
        assert_eq!(a, run_dir(Path::new("/x/pz-server")));
        assert_ne!(a, run_dir(Path::new("/x/pz-lab-server")));
        assert!(a.file_name().unwrap().to_string_lossy().starts_with("pz-server-"));
    }

    #[test]
    fn admin_password_is_masked() {
        let a: Vec<String> = ["-cachedir=/x", "-adminpassword", "secret", "-servername", "s"].iter().map(|s| s.to_string()).collect();
        assert_eq!(mask_args(&a), ["-cachedir=/x", "-adminpassword", "***", "-servername", "s"]);
    }

    #[test]
    fn tail_and_read_from() {
        let p = std::env::temp_dir().join(format!("pzmm-tail-{}", std::process::id()));
        std::fs::write(&p, "a\nb\nc\n").unwrap();
        assert_eq!(tail_lines(&p, 2), ["b", "c"]);
        assert_eq!(read_from(&p, 2), "b\nc\n");
        let _ = std::fs::remove_file(&p);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn finds_process_by_cachedir_and_fifo_never_eofs() {
        let dir = std::env::temp_dir().join(format!("pzmm-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // a fake "server": reads its stdin (the FIFO) and echoes lines until it sees quit
        let f = dir.join("in");
        let fh = make_fifo(&f).unwrap();
        let out = dir.join("out");
        let script = format!("while read l; do echo \"got $l\" >> {}; [ \"$l\" = quit ] && exit 0; done", out.display());
        let mut child = Command::new("sh")
            .args(["-c", &script, "sh", &format!("-cachedir={}", dir.display())])
            .stdin(fh)
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(pids_with_cachedir(&dir).contains(&child.id()));
        write_fifo(&f, "hello").unwrap();
        write_fifo(&f, "quit").unwrap();
        let st = child.wait().unwrap();
        assert!(st.success());
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "got hello\ngot quit\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
