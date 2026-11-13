//! Headless interface for scripts and AI agents.
//!
//! One command table drives both frontends:
//!   * CLI:  `pz-mod-manager <group> <action> [positional…] [--param value] [--flag] [--json '{…}']`
//!   * MCP:  `pz-mod-manager mcp` (stdio JSON-RPC, see mcp.rs) — every command is a tool `<group>_<action>`
//!
//! Output is always JSON: `{"ok":true,"result":…}` or `{"ok":false,"error":"…"}` (exit code 1).
//! State is shared with the GUI through config.toml (locked + atomic writes) and runtime.rs (run files).
use crate::config::{Profile, ServerCfg, Settings, Store};
use crate::mods::{self, Mod, Severity, Source};
use crate::runtime::{self, Role};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum K {
    Str,
    Int,
    Bool,
    List,
}

pub struct P {
    pub name: &'static str,
    pub kind: K,
    pub required: bool,
    /// filled from positional CLI arguments (in declaration order; a List takes the rest)
    pub positional: bool,
    pub help: &'static str,
}

const fn p(name: &'static str, kind: K, help: &'static str) -> P {
    P { name, kind, required: false, positional: false, help }
}
const fn req(name: &'static str, kind: K, help: &'static str) -> P {
    P { name, kind, required: true, positional: true, help }
}
const fn pos(name: &'static str, kind: K, help: &'static str) -> P {
    P { name, kind, required: false, positional: true, help }
}

pub struct Cmd {
    /// "group.action"
    pub name: &'static str,
    pub help: &'static str,
    pub params: &'static [P],
    /// takes the `env` parameter (environment id/label/index; default: the active one)
    pub env: bool,
    /// changes config.toml (runs under the config lock and saves)
    pub writes: bool,
    /// may change files or processes (MCP hint: not read-only)
    pub mutates: bool,
    pub run: fn(&mut Ctx, &Args) -> Result<Value>,
}

const ENV_P: P = p("env", K::Str, "environment id, label or index (default: the active environment)");
/// for commands without positional parameters of their own: `server start lab`, `env use lab`
const ENV_POS: P = pos("env", K::Str, "environment id, label or index (default: the active environment)");
const ALLOW_P: P = p("allow_user_dir", K::Bool, "allow touching an environment whose server cachedir is the game's default ~/Zomboid (only when the user asked for it)");

/// Commands that change files or processes of an environment's server (not just config.toml).
/// They refuse to work on the game's default ~/Zomboid unless allow_user_dir: on a fresh machine the
/// default environment points there, and that folder holds the user's own saves and servers.
fn guards_user_dir(c: &Cmd) -> bool {
    // pack.* only touch the client's mods folder, like client.*
    c.env && c.mutates && !c.writes && !c.name.starts_with("client.") && !c.name.starts_with("pack.")
}

fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    let n = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    n(a) == n(b) || a.to_string_lossy().trim_end_matches(['/', '\\']) == b.to_string_lossy().trim_end_matches(['/', '\\'])
}

pub struct Args(pub Map<String, Value>);

impl Args {
    pub fn str(&self, k: &str) -> Option<String> {
        match self.0.get(k)? {
            Value::String(s) => Some(s.clone()),
            Value::Null => None,
            v => Some(v.to_string()),
        }
    }
    pub fn need(&self, k: &str) -> Result<String> {
        self.str(k).filter(|s| !s.trim().is_empty()).ok_or_else(|| anyhow!("missing parameter: {k}"))
    }
    pub fn int(&self, k: &str) -> Option<i64> {
        match self.0.get(k)? {
            Value::Number(n) => n.as_i64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
    pub fn flag(&self, k: &str) -> bool {
        match self.0.get(k) {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => matches!(s.as_str(), "1" | "true" | "yes" | "on"),
            Some(Value::Number(n)) => n.as_i64() != Some(0),
            _ => false,
        }
    }
    pub fn list(&self, k: &str) -> Vec<String> {
        match self.0.get(k) {
            Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string).or_else(|| (!v.is_null()).then(|| v.to_string()))).flat_map(|s| split_list(&s)).collect(),
            Some(Value::String(s)) => split_list(s),
            _ => vec![],
        }
    }
}

fn split_list(s: &str) -> Vec<String> {
    s.split([',', ';', '\n']).map(|x| x.trim().trim_start_matches('\\').to_string()).filter(|x| !x.is_empty()).collect()
}

pub struct Ctx {
    pub store: Store,
}

impl Ctx {
    fn env_index(&self, a: &Args) -> Result<usize> {
        let s = &self.store.settings;
        match a.str("env").filter(|e| !e.trim().is_empty()) {
            Some(q) => s.find_env(&q).ok_or_else(|| {
                anyhow!("unknown environment '{q}', known: {}", s.servers.iter().map(|c| c.key()).collect::<Vec<_>>().join(", "))
            }),
            None => Ok(s.active_server.min(s.servers.len() - 1)),
        }
    }
    fn env(&self, a: &Args) -> Result<ServerCfg> {
        Ok(self.store.settings.servers[self.env_index(a)?].clone())
    }
    fn profile_index(&self, a: &Args) -> Result<usize> {
        match a.str("profile").filter(|e| !e.trim().is_empty()) {
            Some(q) => self
                .store
                .profiles
                .iter()
                .position(|p| p.name == q)
                .or_else(|| self.store.profiles.iter().position(|p| p.name.eq_ignore_ascii_case(&q)))
                .or_else(|| q.parse::<usize>().ok().filter(|i| *i < self.store.profiles.len()))
                .ok_or_else(|| anyhow!("unknown profile '{q}', known: {}", self.store.profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "))),
            None => Ok(self.store.active.min(self.store.profiles.len() - 1)),
        }
    }
    fn profile(&self, a: &Args) -> Result<Profile> {
        Ok(self.store.profiles[self.profile_index(a)?].clone())
    }
}

// ---------- JSON views ----------

fn source_name(s: Source) -> &'static str {
    match s {
        Source::ServerLocal => "server_local",
        Source::ClientLocal => "client_local",
        Source::ClientWorkshop => "client_workshop",
        Source::ServerWorkshop => "server_workshop",
        Source::Cache => "download_cache",
    }
}

fn mod_json(m: &Mod) -> Value {
    json!({
        "id": m.id, "name": m.name, "source": source_name(m.source), "workshop_id": m.workshop_id,
        "requires": m.requires, "path": m.path, "url": m.url,
    })
}

fn issues_json(order: &[String], installed: &HashMap<String, Mod>, dups: &HashSet<String>) -> Value {
    let issues = mods::check(order, installed, dups);
    let mut list = vec![];
    for id in order {
        for i in issues.get(id).into_iter().flatten() {
            list.push(json!({"id": id, "severity": if i.sev == Severity::Error { "error" } else { "warning" }, "message": i.msg}));
        }
    }
    let errors = list.iter().filter(|v| v["severity"] == "error").count();
    json!({"errors": errors, "warnings": list.len() - errors, "issues": list})
}

fn status_json(st: &runtime::Status) -> Value {
    serde_json::to_value(st).unwrap_or(Value::Null)
}

fn env_json(s: &Settings, i: usize, detail: bool) -> Value {
    let c = &s.servers[i];
    let srv = runtime::status(s, c, Role::Server);
    let cli = runtime::status(s, c, Role::Client);
    let mut v = json!({
        "index": i,
        "id": c.key(),
        "label": c.label,
        "active": i == s.active_server,
        "server": {
            "cache_dir": c.cache_dir, "name": c.name, "ini": c.ini(), "ini_exists": c.ini().exists(),
            "game_dir": c.game_dir, "nosteam": c.nosteam, "extra_args": c.extra_args, "public_host": c.public_host,
            "mods_dir": c.mods_dir(), "workshop_dir": c.workshop_dir,
            "running": srv.running, "ready": srv.ready, "pids": srv.pids,
        },
        "client": {
            "cache_dir": c.client_cache_dir(s), "mods_dir": c.client_mods_dir(s), "game_dir": s.client_game_dir,
            "running": cli.running, "pids": cli.pids,
        },
    });
    if detail {
        v["server"]["status"] = status_json(&srv);
        v["server"]["sandbox"] = json!(crate::sandbox::path(c));
        v["server"]["backups_dir"] = json!(crate::backup::root(c));
        v["server"]["ini_names"] = json!(c.list_names());
        v["server"]["launch"] = json!(crate::server::launch_plan(c).map(|p| json!({"program": p.program, "args": p.args})));
        v["client"]["status"] = status_json(&cli);
        v["client"]["launch"] = json!(runtime::client_plan(s, c, false, &[]));
        v["run_dir"] = json!(runtime::run_dir(&c.cache_dir));
    }
    v
}

// ---------- commands ----------

fn env_list(c: &mut Ctx, _: &Args) -> Result<Value> {
    let s = &c.store.settings;
    Ok(json!((0..s.servers.len()).map(|i| env_json(s, i, false)).collect::<Vec<_>>()))
}

fn env_show(c: &mut Ctx, a: &Args) -> Result<Value> {
    let i = c.env_index(a)?;
    Ok(env_json(&c.store.settings, i, true))
}

fn env_use(c: &mut Ctx, a: &Args) -> Result<Value> {
    let i = c.env_index(a)?;
    c.store.settings.active_server = i;
    Ok(env_json(&c.store.settings, i, false))
}

fn apply_env_fields(e: &mut ServerCfg, a: &Args) -> Result<Vec<&'static str>> {
    let mut changed = vec![];
    for k in ["id", "label", "name", "extra_args"] {
        if let Some(v) = a.str(k) {
            match k {
                "id" => e.id = v.trim().to_string(),
                "label" => e.label = v,
                "name" => {
                    crate::validate::server_name(&v).map_err(anyhow::Error::msg)?;
                    e.name = v.trim().to_string()
                }
                _ => e.extra_args = v,
            }
            changed.push(k);
        }
    }
    for k in ["cache_dir", "client_cache_dir", "game_dir", "workshop_dir"] {
        if let Some(v) = a.str(k) {
            let p = PathBuf::from(v.trim());
            match k {
                "cache_dir" => e.cache_dir = p,
                "client_cache_dir" => e.client_cache_dir = p,
                "game_dir" => e.game_dir = p,
                _ => e.workshop_dir = p,
            }
            changed.push(k);
        }
    }
    if a.0.contains_key("nosteam") {
        e.nosteam = a.flag("nosteam");
        changed.push("nosteam");
    }
    if let Some(v) = a.str("public_host") {
        e.public_host = v.trim().to_string();
        changed.push("public_host");
    }
    Ok(changed)
}

fn check_unique(s: &Settings) -> Result<()> {
    let mut seen = HashSet::new();
    for c in &s.servers {
        if !seen.insert(c.key()) {
            bail!("two environments share the id '{}': give one of them another id", c.key());
        }
    }
    let mut dirs = HashSet::new();
    for c in &s.servers {
        if !dirs.insert(c.cache_dir.clone()) {
            bail!("two environments share the server cachedir {} (the world database would be shared)", c.cache_dir.display());
        }
    }
    Ok(())
}

fn env_set(c: &mut Ctx, a: &Args) -> Result<Value> {
    let i = c.env_index(a)?;
    let changed = apply_env_fields(&mut c.store.settings.servers[i], a)?;
    if changed.is_empty() {
        bail!("nothing to change: pass one of id, label, name, cache_dir, client_cache_dir, game_dir, workshop_dir, nosteam, extra_args");
    }
    check_unique(&c.store.settings)?;
    Ok(json!({"changed": changed, "env": env_json(&c.store.settings, i, false)}))
}

fn env_add(c: &mut Ctx, a: &Args) -> Result<Value> {
    let id = a.need("id")?;
    let base = match a.str("copy_from") {
        Some(q) => c.store.settings.servers[c.store.settings.find_env(&q).ok_or_else(|| anyhow!("unknown environment '{q}'"))?].clone(),
        None => ServerCfg::default(),
    };
    let mut e = ServerCfg { id: id.clone(), label: id.clone(), client_cache_dir: PathBuf::new(), ..base };
    apply_env_fields(&mut e, a)?;
    if a.str("cache_dir").is_none() {
        bail!("cache_dir is required: every environment needs its own server cachedir");
    }
    c.store.settings.servers.push(e);
    check_unique(&c.store.settings)?;
    let i = c.store.settings.servers.len() - 1;
    Ok(env_json(&c.store.settings, i, false))
}

fn env_remove(c: &mut Ctx, a: &Args) -> Result<Value> {
    if a.str("env").is_none_or(|e| e.trim().is_empty()) {
        bail!("pass env explicitly to remove an environment");
    }
    let i = c.env_index(a)?;
    if c.store.settings.servers.len() == 1 {
        bail!("cannot remove the only environment");
    }
    let e = c.store.settings.servers.remove(i);
    let s = &mut c.store.settings;
    if s.active_server >= s.servers.len() || s.active_server == i {
        s.active_server = 0;
    } else if s.active_server > i {
        s.active_server -= 1;
    }
    Ok(json!({"removed": e.key(), "note": "only the configuration entry was removed; no files were deleted"}))
}

fn config_show(c: &mut Ctx, _: &Args) -> Result<Value> {
    let s = &c.store.settings;
    Ok(json!({
        "config_file": Store::path(),
        "data_dir": crate::config::data_dir(),
        "download_cache": crate::steam::cache_content(),
        "client_game_dir": s.client_game_dir,
        "client_workshop_dir": s.client_workshop_dir,
        "client_mods_dir": s.client_mods_dir,
        "language": s.language,
        "steam_api_key_set": !s.steam_api_key.is_empty(),
        "active_env": s.server().key(),
        "active_profile": c.store.profile().name,
        "envs": s.servers.iter().map(|e| e.key()).collect::<Vec<_>>(),
    }))
}

fn config_set(c: &mut Ctx, a: &Args) -> Result<Value> {
    let s = &mut c.store.settings;
    let mut changed = vec![];
    for k in ["client_game_dir", "client_workshop_dir", "client_mods_dir"] {
        if let Some(v) = a.str(k) {
            let p = PathBuf::from(v.trim());
            match k {
                "client_game_dir" => s.client_game_dir = p,
                "client_workshop_dir" => s.client_workshop_dir = p,
                _ => s.client_mods_dir = p,
            }
            changed.push(k);
        }
    }
    if let Some(l) = a.str("language") {
        if !matches!(l.as_str(), "" | "ru" | "en") {
            bail!("language must be '', 'ru' or 'en'");
        }
        s.language = l;
        changed.push("language");
    }
    if let Some(k) = a.str("steam_api_key") {
        s.steam_api_key = k.trim().to_string();
        changed.push("steam_api_key");
    }
    if changed.is_empty() {
        bail!("nothing to change");
    }
    Ok(json!({"changed": changed}))
}

fn mods_list(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let prof = c.profile(a)?;
    let sc = mods::scan_env(&c.store.settings, &srv);
    let issues = mods::check(&prof.enabled, &sc.installed, &sc.dups);
    let q = a.str("filter").unwrap_or_default().to_lowercase();
    let only = a.flag("enabled_only");
    let mut out: Vec<Value> = vec![];
    let mut ids: Vec<&String> = prof.enabled.iter().collect();
    let mut rest: Vec<&Mod> = sc.installed.values().filter(|m| !prof.enabled.contains(&m.id)).collect();
    rest.sort_by_key(|m| m.name.to_lowercase());
    if !only {
        ids.extend(rest.iter().map(|m| &m.id));
    }
    for id in ids {
        let m = sc.installed.get(id);
        if !q.is_empty() && !id.to_lowercase().contains(&q) && !m.is_some_and(|m| m.name.to_lowercase().contains(&q)) {
            continue;
        }
        let mut v = m.map(mod_json).unwrap_or_else(|| json!({"id": id, "missing": true}));
        let position = prof.enabled.iter().position(|x| x == id);
        v["enabled"] = json!(position.is_some());
        v["position"] = json!(position.map(|p| p + 1));
        v["duplicate_id"] = json!(sc.dups.contains(id));
        let iss: Vec<Value> = issues
            .get(id)
            .into_iter()
            .flatten()
            .map(|i| json!({"severity": if i.sev == Severity::Error { "error" } else { "warning" }, "message": i.msg}))
            .collect();
        if !iss.is_empty() {
            v["issues"] = json!(iss);
        }
        out.push(v);
    }
    Ok(json!({"env": srv.key(), "profile": prof.name, "installed": sc.installed.len(), "enabled": prof.enabled.len(), "mods": out}))
}

fn mods_check(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let prof = c.profile(a)?;
    let sc = mods::scan_env(&c.store.settings, &srv);
    let mut v = issues_json(&prof.enabled, &sc.installed, &sc.dups);
    v["env"] = json!(srv.key());
    v["profile"] = json!(prof.name);
    let (sorted, warns) = crate::sort::smart_sort(&prof.enabled, &sc.installed);
    v["order_ok"] = json!(sorted == prof.enabled);
    if !warns.is_empty() {
        v["sort_warnings"] = json!(warns);
    }
    Ok(v)
}

fn profile_list(c: &mut Ctx, _: &Args) -> Result<Value> {
    Ok(json!(c
        .store
        .profiles
        .iter()
        .enumerate()
        .map(|(i, p)| json!({"index": i, "name": p.name, "active": i == c.store.active, "stable": p.stable, "mods": p.enabled.len(), "notes": p.notes}))
        .collect::<Vec<_>>()))
}

fn profile_show(c: &mut Ctx, a: &Args) -> Result<Value> {
    let i = c.profile_index(a)?;
    let p = &c.store.profiles[i];
    Ok(json!({"index": i, "name": p.name, "active": i == c.store.active, "stable": p.stable, "notes": p.notes, "enabled": p.enabled}))
}

fn profile_use(c: &mut Ctx, a: &Args) -> Result<Value> {
    if a.str("profile").is_none() {
        bail!("missing parameter: profile");
    }
    c.store.active = c.profile_index(a)?;
    profile_show(c, &Args(Map::new()))
}

fn profile_create(c: &mut Ctx, a: &Args) -> Result<Value> {
    let name = a.need("name")?.trim().to_string();
    if c.store.profiles.iter().any(|p| p.name == name) {
        bail!("profile '{name}' already exists");
    }
    let enabled = match a.str("copy_from") {
        Some(q) => c.profile(&Args(Map::from_iter([("profile".to_string(), json!(q))])))?.enabled,
        None => vec![],
    };
    c.store.profiles.push(Profile { name, enabled, notes: a.str("notes").unwrap_or_default(), stable: a.flag("stable"), ..Default::default() });
    if !a.flag("no_switch") {
        c.store.active = c.store.profiles.len() - 1;
    }
    profile_show(c, &Args(Map::from_iter([("profile".to_string(), json!(c.store.profiles.len() - 1))])))
}

fn profile_delete(c: &mut Ctx, a: &Args) -> Result<Value> {
    if a.str("profile").is_none() {
        bail!("missing parameter: profile");
    }
    let i = c.profile_index(a)?;
    if c.store.profiles.len() == 1 {
        bail!("cannot delete the only profile");
    }
    let p = c.store.profiles.remove(i);
    if c.store.active >= c.store.profiles.len() || c.store.active == i {
        c.store.active = 0;
    } else if c.store.active > i {
        c.store.active -= 1;
    }
    Ok(json!({"deleted": p.name, "mods": p.enabled}))
}

fn profile_update(c: &mut Ctx, a: &Args) -> Result<Value> {
    let i = c.profile_index(a)?;
    let p = &mut c.store.profiles[i];
    let mut changed = vec![];
    if let Some(n) = a.str("rename") {
        p.name = n.trim().to_string();
        changed.push("name");
    }
    if let Some(n) = a.str("notes") {
        p.notes = n;
        changed.push("notes");
    }
    if a.0.contains_key("stable") {
        p.stable = a.flag("stable");
        changed.push("stable");
    }
    if changed.is_empty() {
        bail!("nothing to change: pass rename, notes or stable");
    }
    let name = p.name.clone();
    if c.store.profiles.iter().filter(|p| p.name == name).count() > 1 {
        bail!("profile '{name}' already exists");
    }
    Ok(json!({"changed": changed, "name": name}))
}

/// Ids not installed in the environment: an error unless `force` (typos are the usual cause)
fn unknown_ids(c: &Ctx, a: &Args, ids: &[String]) -> Result<Vec<String>> {
    let sc = mods::scan_env(&c.store.settings, &c.env(a)?);
    let unknown: Vec<String> = ids.iter().filter(|i| !sc.installed.contains_key(*i)).cloned().collect();
    if !unknown.is_empty() && !a.flag("force") {
        bail!("not installed in environment '{}': {} (install them first, or pass force)", c.env(a)?.key(), unknown.join(", "));
    }
    Ok(unknown)
}

fn profile_enable(c: &mut Ctx, a: &Args) -> Result<Value> {
    let ids = a.list("ids");
    if ids.is_empty() {
        bail!("missing parameter: ids");
    }
    let unknown = unknown_ids(c, a, &ids)?;
    let i = c.profile_index(a)?;
    let en = &mut c.store.profiles[i].enabled;
    let mut at = a.int("position").map(|p| (p.max(1) as usize - 1).min(en.len()));
    let mut added = vec![];
    for id in ids {
        if en.contains(&id) {
            continue;
        }
        match at.as_mut() {
            Some(p) => {
                en.insert(*p, id.clone());
                *p += 1;
            }
            None => en.push(id.clone()),
        }
        added.push(id);
    }
    Ok(json!({"profile": c.store.profiles[i].name, "added": added, "not_installed": unknown, "enabled": c.store.profiles[i].enabled}))
}

fn profile_disable(c: &mut Ctx, a: &Args) -> Result<Value> {
    let ids = a.list("ids");
    if ids.is_empty() {
        bail!("missing parameter: ids");
    }
    let i = c.profile_index(a)?;
    let en = &mut c.store.profiles[i].enabled;
    let before = en.len();
    en.retain(|x| !ids.contains(x));
    let removed = before - en.len();
    Ok(json!({"profile": c.store.profiles[i].name, "removed": removed, "enabled": c.store.profiles[i].enabled}))
}

fn profile_set_order(c: &mut Ctx, a: &Args) -> Result<Value> {
    let mut ids = a.list("ids");
    let mut seen = HashSet::new();
    ids.retain(|x| seen.insert(x.clone()));
    let unknown = unknown_ids(c, a, &ids)?;
    let i = c.profile_index(a)?;
    let diff = crate::packs::diff(&c.store.profiles[i].enabled, &ids);
    c.store.profiles[i].enabled = ids;
    Ok(json!({"profile": c.store.profiles[i].name, "added": diff.added, "removed": diff.removed, "moved": diff.moved, "not_installed": unknown}))
}

fn profile_sort(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let i = c.profile_index(a)?;
    let sc = mods::scan_env(&c.store.settings, &srv);
    let cur = c.store.profiles[i].enabled.clone();
    let (sorted, warns) = crate::sort::smart_sort(&cur, &sc.installed);
    let changed = sorted != cur;
    c.store.profiles[i].enabled = sorted;
    Ok(json!({"changed": changed, "warnings": warns, "enabled": c.store.profiles[i].enabled}))
}

fn ini_show(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let prof = c.profile(a)?;
    let ini = srv.ini();
    let (ini_mods, backslash) = crate::export::read_ini_mods(&ini);
    let text = std::fs::read_to_string(&ini).unwrap_or_default();
    let sc = mods::scan_env(&c.store.settings, &srv);
    let (m, w) = crate::export::build(&prof.enabled, &sc.installed, srv.nosteam, backslash);
    let d = crate::packs::diff(&ini_mods, &prof.enabled);
    Ok(json!({
        "env": srv.key(), "ini": ini, "exists": ini.exists(), "profile": prof.name,
        "ini_mods": ini_mods, "ini_workshop_items": crate::ini::get(&text, "WorkshopItems").unwrap_or_default(),
        "b42_backslash_style": backslash,
        "from_profile": {"Mods": m, "WorkshopItems": w},
        "in_sync": d.added.is_empty() && d.removed.is_empty() && d.moved.is_empty(),
        "profile_vs_ini": {"added": d.added, "removed": d.removed, "moved": d.moved},
    }))
}

fn ini_apply(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let prof = c.profile(a)?;
    let ini = srv.ini();
    if !ini.exists() {
        bail!("{} does not exist: start the server once (server.start with admin_password) to create it", ini.display());
    }
    let sc = mods::scan_env(&c.store.settings, &srv);
    let check = issues_json(&prof.enabled, &sc.installed, &sc.dups);
    if check["errors"].as_u64().unwrap_or(0) > 0 && !a.flag("force") {
        bail!("profile '{}' has {} error(s), not writing (fix them or pass force): {}", prof.name, check["errors"], check["issues"]);
    }
    let (old, backslash) = crate::export::read_ini_mods(&ini);
    let (m, w) = crate::export::build(&prof.enabled, &sc.installed, srv.nosteam, backslash);
    crate::export::write_ini(&ini, &m, &w)?;
    let d = crate::packs::diff(&old, &prof.enabled);
    let running = runtime::status(&c.store.settings, &srv, Role::Server).running;
    Ok(json!({
        "env": srv.key(), "ini": ini, "profile": prof.name, "mods": prof.enabled.len(),
        "workshop_items": if w.is_empty() { 0 } else { w.split(';').count() },
        "added": d.added, "removed": d.removed, "moved": d.moved,
        "backup": "previous ini kept as <ini>.bak-<time>",
        "note": if running { "the server is running: restart it to load the new mod list" } else { "" },
        "check": check,
    }))
}

fn ini_import(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let (ids, _) = crate::export::read_ini_mods(&srv.ini());
    if ids.is_empty() && !srv.ini().exists() {
        bail!("{} does not exist", srv.ini().display());
    }
    let i = c.profile_index(a)?;
    let d = crate::packs::diff(&c.store.profiles[i].enabled, &ids);
    c.store.profiles[i].enabled = ids;
    Ok(json!({"profile": c.store.profiles[i].name, "mods": c.store.profiles[i].enabled.len(), "added": d.added, "removed": d.removed, "moved": d.moved}))
}

const SECRET_KEYS: &[&str] = &["Password", "RCONPassword", "DiscordToken", "AdminPassword"];

fn ini_get(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let text = std::fs::read_to_string(srv.ini()).with_context(|| format!("{}", srv.ini().display()))?;
    let keys = a.list("keys");
    let show_secrets = a.flag("show_secrets");
    let mut out = Map::new();
    for l in text.lines() {
        let Some((k, v)) = l.split_once('=') else { continue };
        if k.starts_with('#') || k.trim().is_empty() || (!keys.is_empty() && !keys.iter().any(|x| x == k)) {
            continue;
        }
        let v = if SECRET_KEYS.contains(&k) && !v.is_empty() && !show_secrets { "***".to_string() } else { v.to_string() };
        out.insert(k.to_string(), json!(v));
    }
    let missing: Vec<&String> = keys.iter().filter(|k| !out.contains_key(*k)).collect();
    Ok(json!({"env": srv.key(), "ini": srv.ini(), "values": out, "missing": missing}))
}

fn kv_list(a: &Args, k: &str) -> Result<Vec<(String, String)>> {
    let mut out = vec![];
    for item in match a.0.get(k) {
        Some(Value::Object(m)) => m.iter().map(|(k, v)| format!("{k}={}", v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))).collect(),
        Some(Value::Array(v)) => v.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect(),
        Some(Value::String(s)) => s.lines().map(str::to_string).collect::<Vec<_>>(),
        _ => vec![],
    } {
        let (key, val) = item.split_once('=').ok_or_else(|| anyhow!("expected Key=Value, got '{item}'"))?;
        if val.contains(['\n', '\r']) {
            bail!("value of {key} must be one line");
        }
        out.push((key.trim().to_string(), val.trim().to_string()));
    }
    if out.is_empty() {
        bail!("missing parameter: {k} (Key=Value pairs)");
    }
    Ok(out)
}

fn ini_set(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    if !srv.ini().exists() {
        bail!("{} does not exist: start the server once (server.start with admin_password) to create it", srv.ini().display());
    }
    let kv = kv_list(a, "values")?;
    for (k, _) in &kv {
        if matches!(k.as_str(), "Mods" | "WorkshopItems") {
            bail!("{k} is managed by profiles: use ini.apply");
        }
    }
    let (applied, bak) = crate::ini::set_existing(&srv.ini(), &kv)?;
    let skipped: Vec<&String> = kv.iter().map(|(k, _)| k).filter(|k| !applied.contains(k)).collect();
    Ok(json!({"env": srv.key(), "applied": applied, "unknown_keys_skipped": skipped, "backup": bak,
        "note": if runtime::status(&c.store.settings, &srv, Role::Server).running { "the server is running: most options apply after a restart (or `reloadoptions`)" } else { "" }}))
}

fn sandbox_cat(cat: &str) -> String {
    if cat == crate::sandbox::ROOT { String::new() } else { cat.to_string() }
}

fn sandbox_get(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let path = crate::sandbox::path(&srv);
    let text = std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
    let cat = a.str("category");
    let key = a.str("key").map(|k| k.to_lowercase());
    let fields: Vec<Value> = crate::sandbox::parse(&text)
        .into_iter()
        .filter(|f| cat.as_ref().is_none_or(|c| sandbox_cat(&f.category).eq_ignore_ascii_case(c)))
        .filter(|f| key.as_ref().is_none_or(|k| f.key.to_lowercase().contains(k)))
        .map(|f| {
            json!({"category": sandbox_cat(&f.category), "key": f.key, "value": f.value,
                "type": match f.kind { crate::sandbox::Kind::Bool => "bool", crate::sandbox::Kind::Number => "number", crate::sandbox::Kind::Text => "text" }})
        })
        .collect();
    Ok(json!({"env": srv.key(), "file": path, "fields": fields}))
}

fn sandbox_set(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    if runtime::status(&c.store.settings, &srv, Role::Server).running {
        bail!("the server is running: stop it first (it keeps its own copy of SandboxVars and may overwrite the file)");
    }
    let path = crate::sandbox::path(&srv);
    let text = std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
    let fields = crate::sandbox::parse(&text);
    let mut changes = vec![];
    for (k, v) in kv_list(a, "values")? {
        let (cat, key) = match k.rsplit_once('.') {
            Some((c, k)) => (c.to_string(), k.to_string()),
            None => (String::new(), k.clone()),
        };
        let f = fields
            .iter()
            .find(|f| f.key == key && sandbox_cat(&f.category) == cat)
            .ok_or_else(|| anyhow!("no sandbox option '{k}' (use Category.Key for options inside a category)"))?;
        match f.kind {
            crate::sandbox::Kind::Bool if !matches!(v.as_str(), "true" | "false") => bail!("{k} is a bool: use true or false"),
            crate::sandbox::Kind::Number if v.parse::<f64>().is_err() => bail!("{k} is a number"),
            _ => {}
        }
        changes.push((f.category.clone(), key, v));
    }
    let (new, n) = crate::sandbox::update(&text, &changes);
    let bak = crate::atomic::write(&path, new.as_bytes())?;
    Ok(json!({"env": srv.key(), "changed": n, "file": path, "backup": bak}))
}

fn srv_status(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let mut v = status_json(&runtime::status(&c.store.settings, &srv, Role::Server));
    v["env"] = json!(srv.key());
    v["ini_exists"] = json!(srv.ini().exists());
    Ok(v)
}

fn srv_start(c: &mut Ctx, a: &Args) -> Result<Value> {
    let mut srv = c.env(a)?;
    if a.0.contains_key("nosteam") {
        // one-off override; the environment's setting is not changed
        srv.nosteam = a.flag("nosteam");
    }
    let pw = a.str("admin_password");
    let started = runtime::start_server(&c.store.settings, &srv, pw.as_deref())?;
    drop(started.child);
    let mut v = json!({"env": srv.key(), "launch": started.launch, "log": started.log});
    if a.flag("wait") {
        let st = runtime::wait_ready(&c.store.settings, &srv, Duration::from_secs(a.int("timeout").unwrap_or(600).max(1) as u64));
        v["ready"] = json!(st.ready);
        v["running"] = json!(st.running);
        if !st.running {
            v["log_tail"] = json!(runtime::tail_lines(&st.log, 40));
            bail!("the server exited during startup: {}", v);
        }
    } else {
        v["hint"] = json!("loading takes a while: use server.wait (or server.status until ready=true)");
    }
    Ok(v)
}

fn srv_wait(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let st = runtime::wait_ready(&c.store.settings, &srv, Duration::from_secs(a.int("timeout").unwrap_or(600).max(1) as u64));
    let mut v = status_json(&st);
    if !st.running {
        v["log_tail"] = json!(runtime::tail_lines(&st.log, 40));
    }
    Ok(v)
}

fn srv_stop(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let r = runtime::stop_server(&c.store.settings, &srv, Duration::from_secs(a.int("timeout").unwrap_or(120).max(1) as u64), a.flag("force"))?;
    Ok(serde_json::to_value(r)?)
}

fn srv_kill(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    Ok(serde_json::to_value(runtime::kill(&c.store.settings, &srv, Role::Server)?)?)
}

fn srv_cmd(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let command = a.need("command")?;
    let r = runtime::send_command(&c.store.settings, &srv, &command, Duration::from_millis(a.int("wait_ms").unwrap_or(1500).clamp(0, 30000) as u64))?;
    Ok(serde_json::to_value(r)?)
}

fn log_of(c: &Ctx, a: &Args, role: Role) -> Result<Value> {
    let srv = c.env(a)?;
    let st = runtime::status(&c.store.settings, &srv, role);
    let n = a.int("lines").unwrap_or(100).clamp(1, 5000) as usize;
    let mut lines = runtime::tail_lines(&st.log, n);
    if let Some(g) = a.str("grep").filter(|g| !g.is_empty()) {
        let g = g.to_lowercase();
        lines = runtime::tail_lines(&st.log, 20000).into_iter().filter(|l| l.to_lowercase().contains(&g)).collect();
        lines = lines[lines.len().saturating_sub(n)..].to_vec();
    }
    Ok(json!({"env": srv.key(), "log": st.log, "running": st.running, "lines": lines}))
}

fn srv_log(c: &mut Ctx, a: &Args) -> Result<Value> {
    log_of(c, a, Role::Server)
}

fn cli_status(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let mut v = status_json(&runtime::status(&c.store.settings, &srv, Role::Client));
    v["env"] = json!(srv.key());
    v["launch"] = json!(runtime::client_plan(&c.store.settings, &srv, a.flag("nosteam"), &[]));
    Ok(v)
}

fn cli_start(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let started = runtime::start_client(&c.store.settings, &srv, a.flag("nosteam"), &a.list("args"))?;
    drop(started.child);
    Ok(json!({"env": srv.key(), "launch": started.launch, "log": started.log,
        "note": "without nosteam the Steam client must be running"}))
}

fn cli_stop(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    Ok(serde_json::to_value(runtime::kill(&c.store.settings, &srv, Role::Client)?)?)
}

fn cli_log(c: &mut Ctx, a: &Args) -> Result<Value> {
    log_of(c, a, Role::Client)
}

fn backup_list(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    Ok(json!({"env": srv.key(), "dir": crate::backup::root(&srv),
        "backups": crate::backup::list(&srv).into_iter().map(|e| json!({"name": e.name, "info": e.info})).collect::<Vec<_>>()}))
}

fn backup_create(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let running = runtime::status(&c.store.settings, &srv, Role::Server).running;
    if running {
        // make the files on disk current before copying them
        let _ = runtime::send_command(&c.store.settings, &srv, "save", Duration::from_secs(5));
    }
    let keep = a.int("keep").map(|k| k.max(0) as usize).unwrap_or(srv.backup_keep);
    let p = crate::backup::create(&srv, &a.str("label").unwrap_or_default(), keep)?;
    Ok(json!({"env": srv.key(), "backup": p, "server_was_running": running}))
}

fn backup_restore(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let running = runtime::status(&c.store.settings, &srv, Role::Server).running;
    let safety = crate::backup::restore(&srv, &a.need("name")?, running)?;
    Ok(json!({"env": srv.key(), "restored": a.need("name")?, "safety_backup_of_previous_state": safety}))
}

fn players(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let list: Vec<Value> = crate::players::load(&srv.cache_dir)
        .into_iter()
        .map(|p| json!({"name": p.name, "online": p.online, "pos": p.pos.map(|(x, y, z)| json!({"x": x, "y": y, "z": z})), "last_seen": p.last_seen, "role": p.role, "ip": p.ip}))
        .collect();
    Ok(json!({"env": srv.key(), "source": "server logs (<cachedir>/Logs)", "players": list}))
}

fn ws_cached(c: &mut Ctx, _: &Args) -> Result<Value> {
    let s = &c.store.settings;
    let cache = mods::scan(&crate::steam::cache_content(), Source::Cache);
    let mut items: Vec<(String, Vec<Value>)> = vec![];
    for m in &cache {
        let w = m.workshop_id.clone().unwrap_or_default();
        let v = json!({"id": m.id, "name": m.name, "requires": m.requires});
        match items.iter_mut().find(|(k, _)| *k == w) {
            Some((_, l)) => l.push(v),
            None => items.push((w, vec![v])),
        }
    }
    let items: Vec<Value> = items
        .into_iter()
        .map(|(w, m)| {
            let placed: Map<String, Value> = s.servers.iter().map(|e| (e.key(), json!(crate::steam::placed(&w, s, e)))).collect();
            json!({"workshop_id": w, "mods": m, "installed": placed})
        })
        .collect();
    Ok(json!({"dir": crate::steam::cache_content(), "items": items}))
}

fn ws_download(_: &mut Ctx, a: &Args) -> Result<Value> {
    let ids = crate::steam::parse_ids(&a.list("ids").join("\n"));
    if ids.is_empty() {
        bail!("missing parameter: ids (workshop ids or links)");
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let ok = crate::steam::run_blocking(&ids, &tx);
    drop(tx);
    let log: Vec<String> = rx.try_iter().filter_map(|m| if let crate::steam::Msg::Log(l) = m { Some(l) } else { None }).collect();
    let present: Vec<&String> = ids.iter().filter(|i| crate::steam::cache_content().join(i).is_dir()).collect();
    let ok = ok?;
    Ok(json!({"ok": ok, "requested": ids, "in_cache": present, "log_tail": log[log.len().saturating_sub(30)..].to_vec(),
        "next": "workshop.install installs a downloaded item into an environment"}))
}

fn ws_install(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let s = &c.store.settings;
    let (client, server) = (a.flag("client"), a.flag("server"));
    if !client && !server {
        bail!("pass client and/or server");
    }
    let mut done = vec![];
    for w in crate::steam::parse_ids(&a.list("ids").join("\n")) {
        done.push(crate::steam::install_into(&w, s, &srv, client, server)?);
    }
    if done.is_empty() {
        bail!("missing parameter: ids");
    }
    Ok(json!({"env": srv.key(), "installed": done, "client_mods_dir": client.then(|| srv.client_mods_dir(s)), "server_mods_dir": server.then(|| if srv.nosteam { srv.mods_dir() } else { srv.workshop_dir.clone() }),
        "next": "profile.enable the mod ids, then ini.apply"}))
}

// ---------- shared pack file (.pzpack) ----------

fn pack_path(a: &Args) -> Result<PathBuf> {
    let p = PathBuf::from(a.need("file")?);
    if !p.is_file() {
        bail!("file not found: {}", p.display());
    }
    Ok(p)
}

/// What a pack file means for this machine: is it known, is it newer, what is missing
fn pack_view(store: &Store, srv: &ServerCfg, path: &std::path::Path, f: &crate::packs::PackFile) -> Value {
    let s = &store.settings;
    let installed = mods::scan_env(s, srv).installed;
    let known = (!f.pack_id.is_empty()).then(|| store.profiles.iter().position(|p| p.pack_id == f.pack_id)).flatten();
    let old = known.map(|i| store.profiles[i].enabled.clone()).unwrap_or_default();
    let d = crate::share::describe(&old, f);
    let mut missing_ws: Vec<&String> = vec![];
    for m in &f.mods {
        if let Some(w) = &m.workshop_id {
            if !installed.contains_key(&m.id) && !missing_ws.contains(&w) {
                missing_ws.push(w);
            }
        }
    }
    let applied = known.map(|i| store.profiles[i].applied_revision);
    json!({
        "file": path, "name": f.name, "notes": f.notes, "created": f.created, "pack_id": f.pack_id, "revision": f.revision,
        "server": f.server.as_ref().map(|x| json!({"host": x.host, "port": x.port, "udp_port": x.udp_port, "name": x.name,
            "connect": if x.host.is_empty() { Value::Null } else { json!(format!("{}:{}", x.host, if x.port.is_empty() { "16261" } else { &x.port })) }})),
        "mods": f.mods.iter().map(|m| json!({"id": m.id, "name": m.name, "workshop_id": m.workshop_id, "local_folder": m.workshop_id.is_none().then(|| m.folder.clone()).flatten(), "installed": installed.contains_key(&m.id)})).collect::<Vec<_>>(),
        "env": srv.key(), "client_mods_dir": srv.client_mods_dir(s),
        "known_profile": known.map(|i| store.profiles[i].name.clone()), "applied_revision": applied,
        "update_available": applied.is_none_or(|r| f.revision > r) || known.is_none(),
        "changes": {"added": d.added, "removed": d.removed, "moved": d.moved},
        "missing_workshop_items": missing_ws,
    })
}

fn pack_info(c: &mut Ctx, a: &Args) -> Result<Value> {
    let path = pack_path(a)?;
    let f = crate::share::read_pack(&path)?;
    let srv = c.env(a)?;
    let mut v = pack_view(&c.store, &srv, &path, &f);
    v["next"] = json!("pack.install <file> puts the mods on this environment's client and copies the mod order");
    Ok(v)
}

fn pack_install(c: &mut Ctx, a: &Args) -> Result<Value> {
    let path = pack_path(a)?;
    let f = crate::share::read_pack(&path)?;
    let srv = c.env(a)?;
    let settings = c.store.settings.clone();
    let before = pack_view(&c.store, &srv, &path, &f);
    let dest = srv.client_mods_dir(&settings);
    // 1. local mods from the archive
    let local = crate::share::extract_local(&path, &dest).context("unpacking local mods")?;
    // 2. workshop items that are not on disk yet (the config is not locked while SteamCMD runs)
    let missing: Vec<String> = before["missing_workshop_items"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
    let (mut ok_ws, mut failed, mut log_tail) = (vec![], vec![], vec![]);
    if !missing.is_empty() && !a.flag("no_download") {
        let (tx, rx) = std::sync::mpsc::channel();
        let res = crate::steam::run_blocking(&missing, &tx);
        drop(tx);
        let log: Vec<String> = rx.try_iter().filter_map(|m| if let crate::steam::Msg::Log(l) = m { Some(l) } else { None }).collect();
        if !matches!(res, Ok(true)) {
            log_tail = log[log.len().saturating_sub(15)..].to_vec();
            if let Err(e) = res {
                log_tail.push(format!("{e:#}"));
            }
        }
        for w in &missing {
            match crate::steam::install_into(w, &settings, &srv, true, false) {
                Ok(_) => ok_ws.push(w.clone()),
                Err(e) => failed.push(json!({"workshop_id": w, "error": format!("{e:#}")})),
            }
        }
    }
    // 3. remember the pack: profile with the same order, linked to the file
    let _lock = Store::lock()?;
    let mut store = Store::load_strict()?;
    let known = (!f.pack_id.is_empty()).then(|| store.profiles.iter().position(|p| p.pack_id == f.pack_id)).flatten();
    let idx = known.unwrap_or_else(|| {
        store.profiles.push(Profile::default());
        store.profiles.len() - 1
    });
    {
        let p = &mut store.profiles[idx];
        p.name = f.name.clone();
        p.notes = f.notes.clone();
        p.enabled = f.mods.iter().map(|m| m.id.clone()).collect();
        if !f.pack_id.is_empty() {
            p.pack_id = f.pack_id.clone();
            p.source_path = std::fs::canonicalize(&path).unwrap_or(path.clone()).to_string_lossy().into_owned();
            p.applied_revision = f.revision;
        }
    }
    store.active = idx;
    store.try_save()?;
    let mut tr = crate::tracking::Tracking::load();
    let sv = tr.server(&srv.cache_dir.to_string_lossy());
    for m in &f.mods {
        if let Some(w) = &m.workshop_id {
            sv.links.entry(m.id.clone()).or_insert_with(|| w.clone());
        }
    }
    tr.save();
    c.store = store;
    let installed = mods::scan_env(&settings, &srv).installed;
    let still: Vec<&String> = c.store.profiles[idx].enabled.iter().filter(|i| !installed.contains_key(*i)).collect();
    Ok(json!({
        "pack": f.name, "revision": f.revision, "profile": c.store.profiles[idx].name, "updated_existing": known.is_some(),
        "env": srv.key(), "client_mods_dir": dest, "mods": f.mods.len(),
        "local_folders_unpacked": local, "workshop_installed": ok_ws, "workshop_failed": failed,
        "skipped_download": a.flag("no_download").then_some(&missing),
        "not_on_disk": still, "steamcmd_log_tail": log_tail,
        "server": before["server"], "changes": before["changes"],
        "complete": still.is_empty(),
        "next": if still.is_empty() { "done: start the game (client.start) and connect to the server address" } else { "some mods are still missing: retry pack.install, or workshop.download the failed ids" },
    }))
}

fn pack_save(c: &mut Ctx, a: &Args) -> Result<Value> {
    let srv = c.env(a)?;
    let idx = c.profile_index(a)?;
    let settings = c.store.settings.clone();
    let dest = match a.str("file").filter(|f| !f.trim().is_empty()) {
        Some(f) => {
            let mut p = PathBuf::from(f);
            if p.extension().is_none() {
                p.set_extension(crate::share::EXT);
            }
            p
        }
        None if !c.store.profiles[idx].sync_path.is_empty() => PathBuf::from(&c.store.profiles[idx].sync_path),
        None => bail!("missing parameter: file (this profile has no auto-updated file yet)"),
    };
    if !c.store.profiles[idx].source_path.is_empty() {
        bail!("profile '{}' was received from {}: it follows that file and is not shared from here", c.store.profiles[idx].name, c.store.profiles[idx].source_path);
    }
    let info = crate::share::server_info(&srv);
    let bumped = crate::share::bump_if_changed(&mut c.store.profiles[idx], &info);
    if a.flag("sync") {
        let abs = if dest.is_absolute() { dest.clone() } else { std::env::current_dir()?.join(&dest) };
        c.store.profiles[idx].sync_path = abs.to_string_lossy().into_owned();
        c.store.profiles[idx].sync_env = srv.key();
    }
    let p = c.store.profiles[idx].clone();
    let sc = mods::scan_env(&settings, &srv);
    let cache = mods::scan(&crate::steam::cache_content(), Source::Cache);
    let links = crate::App::compute_scan_links(&sc.all, &cache);
    let st = crate::tracking::Tracking::load().server(&srv.cache_dir.to_string_lossy()).clone();
    let roots = vec![srv.mods_dir(), srv.client_mods_dir(&settings)];
    let ws = |m: &Mod| crate::tracking::workshop_of(m, &st, &links);
    let rep = crate::share::write_pack(&dest, &p, &sc.installed, &roots, &ws, info.clone())?;
    Ok(json!({
        "file": dest, "profile": p.name, "pack_id": p.pack_id, "revision": p.revision, "revision_bumped": bumped,
        "mods": p.enabled.len(), "local_folders_included": rep.folders, "workshop_items": rep.subscribe, "not_on_disk": rep.missing,
        "server": info.map(|x| json!({"host": x.host, "port": x.port, "udp_port": x.udp_port})),
        "auto_update": (!p.sync_path.is_empty()).then(|| json!({"file": p.sync_path, "env": p.sync_env, "note": "the GUI rewrites this file on every change; from the CLI run pack.save again"})),
        "warning": srv.public_host.trim().is_empty().then_some("no public address set for this environment: env.set --public-host <ip or domain>, then pack.save again"),
        "next": "give the file to the other player: their manager installs it with pack.install <file>",
    }))
}

fn pack_status(c: &mut Ctx, _: &Args) -> Result<Value> {
    let rows: Vec<Value> = c
        .store
        .profiles
        .iter()
        .filter(|p| !p.sync_path.is_empty() || !p.source_path.is_empty())
        .map(|p| {
            let received = !p.source_path.is_empty();
            let file = if received { &p.source_path } else { &p.sync_path };
            let in_file = crate::share::peek(std::path::Path::new(file)).filter(|(id, _, _)| *id == p.pack_id).map(|(_, r, _)| r);
            if received {
                json!({"profile": p.name, "role": "received", "file": file, "applied_revision": p.applied_revision, "file_revision": in_file,
                    "update_available": in_file.is_some_and(|r| r > p.applied_revision), "next": "pack.install <file> applies the update"})
            } else {
                json!({"profile": p.name, "role": "shared", "file": file, "env": p.sync_env, "revision": p.revision, "file_revision": in_file,
                    "file_is_current": in_file == Some(p.revision), "next": "pack.save --profile <name> rewrites the file"})
            }
        })
        .collect();
    Ok(json!(rows))
}

fn schema(_: &mut Ctx, _: &Args) -> Result<Value> {
    Ok(json!(COMMANDS.iter().map(cmd_schema).collect::<Vec<_>>()))
}

fn version(_: &mut Ctx, _: &Args) -> Result<Value> {
    Ok(json!({"name": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION"), "config": Store::path(), "data_dir": crate::config::data_dir()}))
}

macro_rules! cmd {
    ($name:literal, $help:literal, [$($p:expr),* $(,)?], env=$env:literal, writes=$w:literal, mutates=$m:literal, $f:ident) => {
        Cmd { name: $name, help: $help, params: &[$($p),*], env: $env, writes: $w, mutates: $m, run: $f }
    };
}

pub static COMMANDS: &[Cmd] = &[
    cmd!("env.list", "List environments (stable, lab, …) with their server/client cachedirs and what is running", [], env = false, writes = false, mutates = false, env_list),
    cmd!("env.show", "Everything about one environment: paths, launch commands, run files, process status", [], env = true, writes = false, mutates = false, env_show),
    cmd!("env.use", "Make an environment the active one (default for commands and the GUI)", [], env = true, writes = true, mutates = true, env_use),
    cmd!("env.set", "Change fields of an environment", [p("id", K::Str, "short id used by --env"), p("label", K::Str, "display name"), p("name", K::Str, "server name = ini file name"), p("cache_dir", K::Str, "server -cachedir"), p("client_cache_dir", K::Str, "client -cachedir"), p("game_dir", K::Str, "dedicated server install"), p("workshop_dir", K::Str, "server workshop dir (Steam mode)"), p("nosteam", K::Bool, "-nosteam: local mods only"), p("extra_args", K::Str, "extra server arguments"), p("public_host", K::Str, "address players connect to (IP or domain); goes into shared pack files")], env = true, writes = true, mutates = true, env_set),
    cmd!("env.add", "Add an environment", [req("id", K::Str, "short id"), p("copy_from", K::Str, "environment to copy settings from"), p("label", K::Str, ""), p("name", K::Str, "server name"), p("cache_dir", K::Str, "server -cachedir (required)"), p("client_cache_dir", K::Str, "client -cachedir"), p("game_dir", K::Str, ""), p("workshop_dir", K::Str, ""), p("nosteam", K::Bool, ""), p("extra_args", K::Str, "")], env = false, writes = true, mutates = true, env_add),
    cmd!("env.remove", "Remove an environment from the config (files stay)", [], env = true, writes = true, mutates = true, env_remove),
    cmd!("config.show", "Global settings and file locations", [], env = false, writes = false, mutates = false, config_show),
    cmd!("config.set", "Change global settings", [p("client_game_dir", K::Str, "installed game client"), p("client_workshop_dir", K::Str, "Steam workshop content/108600"), p("client_mods_dir", K::Str, "client mods for environments without client_cache_dir"), p("language", K::Str, "'', ru or en"), p("steam_api_key", K::Str, "Steam Web API key")], env = false, writes = true, mutates = true, config_set),
    cmd!("mods.list", "Installed mods visible to an environment, enabled ones first in load order, with problems", [p("profile", K::Str, "profile name (default: active)"), p("filter", K::Str, "substring of id or name"), p("enabled_only", K::Bool, "only mods enabled in the profile")], env = true, writes = false, mutates = false, mods_list),
    cmd!("mods.check", "Check a profile's mod order against an environment: missing mods, missing/disabled/late dependencies", [p("profile", K::Str, "")], env = true, writes = false, mutates = false, mods_check),
    cmd!("profile.list", "Mod profiles (ordered mod lists)", [], env = false, writes = false, mutates = false, profile_list),
    cmd!("profile.show", "Mods of a profile in load order", [pos("profile", K::Str, "default: active")], env = false, writes = false, mutates = false, profile_show),
    cmd!("profile.use", "Make a profile active", [req("profile", K::Str, "")], env = false, writes = true, mutates = true, profile_use),
    cmd!("profile.create", "Create a profile (and make it active unless no_switch)", [req("name", K::Str, ""), p("copy_from", K::Str, "profile to copy the mod list from"), p("notes", K::Str, ""), p("stable", K::Bool, "mark as stable"), p("no_switch", K::Bool, "keep the current active profile")], env = false, writes = true, mutates = true, profile_create),
    cmd!("profile.delete", "Delete a profile", [req("profile", K::Str, "")], env = false, writes = true, mutates = true, profile_delete),
    cmd!("profile.update", "Rename a profile or change notes/stable", [pos("profile", K::Str, "default: active"), p("rename", K::Str, "new name"), p("notes", K::Str, ""), p("stable", K::Bool, "")], env = false, writes = true, mutates = true, profile_update),
    cmd!("profile.enable", "Enable mods (append, or insert at a 1-based position)", [req("ids", K::List, "mod ids"), p("profile", K::Str, ""), p("position", K::Int, "1-based insert position"), p("force", K::Bool, "allow ids not installed in the environment")], env = true, writes = true, mutates = true, profile_enable),
    cmd!("profile.disable", "Disable mods", [req("ids", K::List, "mod ids"), p("profile", K::Str, "")], env = false, writes = true, mutates = true, profile_disable),
    cmd!("profile.set_order", "Replace the whole ordered mod list of a profile", [req("ids", K::List, "mod ids in load order"), p("profile", K::Str, ""), p("force", K::Bool, "allow ids not installed in the environment")], env = true, writes = true, mutates = true, profile_set_order),
    cmd!("profile.sort", "Sort a profile so dependencies load before their dependents", [p("profile", K::Str, "")], env = true, writes = true, mutates = true, profile_sort),
    cmd!("ini.show", "Mods= / WorkshopItems= in the server ini vs the profile", [p("profile", K::Str, "")], env = true, writes = false, mutates = false, ini_show),
    cmd!("ini.apply", "Write the profile's mod list into the server ini (refuses on errors unless force)", [p("profile", K::Str, ""), p("force", K::Bool, "write even with errors")], env = true, writes = false, mutates = true, ini_apply),
    cmd!("ini.import", "Load the ini's Mods= into a profile", [p("profile", K::Str, "")], env = true, writes = true, mutates = true, ini_import),
    cmd!("ini.get", "Read server ini options (secrets masked unless show_secrets)", [pos("keys", K::List, "keys (default: all)"), p("show_secrets", K::Bool, "")], env = true, writes = false, mutates = false, ini_get),
    cmd!("ini.set", "Set existing server ini options, e.g. DefaultPort=16271 UDPPort=16272", [req("values", K::List, "Key=Value pairs")], env = true, writes = false, mutates = true, ini_set),
    cmd!("sandbox.get", "Read SandboxVars options", [p("category", K::Str, "'' = root"), p("key", K::Str, "substring of the key")], env = true, writes = false, mutates = false, sandbox_get),
    cmd!("sandbox.set", "Change SandboxVars options (server must be stopped)", [req("values", K::List, "Key=Value or Category.Key=Value")], env = true, writes = false, mutates = true, sandbox_set),
    cmd!("server.status", "Is the environment's server running/ready, pids, log file", [], env = true, writes = false, mutates = false, srv_status),
    cmd!("server.start", "Start the server detached (first start needs admin_password)", [p("admin_password", K::Str, "only for the first start, when the ini does not exist yet"), p("nosteam", K::Bool, "override the environment's mode for this start: --nosteam or --no-nosteam (Steam)"), p("wait", K::Bool, "block until ready"), p("timeout", K::Int, "seconds for wait (default 600)")], env = true, writes = false, mutates = true, srv_start),
    cmd!("server.wait", "Wait until the server is ready (or exits)", [p("timeout", K::Int, "seconds (default 600)")], env = true, writes = false, mutates = false, srv_wait),
    cmd!("server.stop", "Graceful stop: quit (saves the world) and wait; force terminates if it does not exit", [p("timeout", K::Int, "seconds to wait after quit (default 120)"), p("force", K::Bool, "terminate if still running")], env = true, writes = false, mutates = true, srv_stop),
    cmd!("server.kill", "Terminate the server process without saving", [], env = true, writes = false, mutates = true, srv_kill),
    cmd!("server.cmd", "Run a server console command (RCON if configured, else the console of a server started by the manager)", [req("command", K::Str, "e.g. players, save, servermsg \"hi\""), p("wait_ms", K::Int, "console mode: how long to collect log output (default 1500)")], env = true, writes = false, mutates = true, srv_cmd),
    cmd!("server.log", "Last lines of the server log", [p("lines", K::Int, "default 100"), p("grep", K::Str, "only lines containing this (case-insensitive)")], env = true, writes = false, mutates = false, srv_log),
    cmd!("client.status", "Is the environment's game client running", [], env = true, writes = false, mutates = false, cli_status),
    cmd!("client.start", "Start the game client with the environment's client cachedir", [p("nosteam", K::Bool, "start without Steam"), p("args", K::List, "extra game arguments")], env = true, writes = false, mutates = true, cli_start),
    cmd!("client.stop", "Terminate the environment's game client", [], env = true, writes = false, mutates = true, cli_stop),
    cmd!("client.log", "Last lines of the client log", [p("lines", K::Int, "default 100"), p("grep", K::Str, "")], env = true, writes = false, mutates = false, cli_log),
    cmd!("backup.list", "Backups of the environment's world and configs", [], env = true, writes = false, mutates = false, backup_list),
    cmd!("backup.create", "Back up world + server configs (asks a running server to save first)", [p("label", K::Str, ""), p("keep", K::Int, "how many backups to keep")], env = true, writes = false, mutates = true, backup_create),
    cmd!("backup.restore", "Restore a backup (server must be stopped; the current state is backed up first)", [req("name", K::Str, "backup name from backup.list")], env = true, writes = false, mutates = true, backup_restore),
    cmd!("players.list", "Players and last known positions from the server logs", [], env = true, writes = false, mutates = false, players),
    cmd!("pack.info", "Read a shared pack file (.pzpack): mods in order, server address, revision, what is missing here and what changed since the applied revision", [pos("file", K::Str, "path to the .pzpack")], env = true, writes = false, mutates = false, pack_info),
    cmd!("pack.install", "Install or update a shared pack on an environment's client: unpack its local mods, download missing workshop items with SteamCMD, copy the mod order into a profile and remember the revision (blocks until done)", [pos("file", K::Str, "path to the .pzpack"), p("no_download", K::Bool, "skip SteamCMD: only local mods and the mod order")], env = true, writes = false, mutates = true, pack_install),
    cmd!("pack.save", "Write a profile as a shared pack file (.pzpack) for another player: mod order, workshop ids, local mods, server address. The revision grows when the content changed", [pos("file", K::Str, "where to write (default: the profile's auto-updated file)"), p("profile", K::Str, "default: active"), p("sync", K::Bool, "remember this file as the auto-updated one")], env = true, writes = true, mutates = true, pack_save),
    cmd!("pack.status", "Profiles linked to pack files: shared ones (is the file current) and received ones (is an update waiting)", [], env = false, writes = false, mutates = false, pack_status),
    cmd!("workshop.cached", "Workshop items already downloaded into the manager's cache, and in which environments (server/client) each one is installed", [], env = false, writes = false, mutates = false, ws_cached),
    cmd!("workshop.download", "Download workshop items with SteamCMD into the cache (blocks until done)", [req("ids", K::List, "workshop ids or links")], env = false, writes = false, mutates = true, ws_download),
    cmd!("workshop.install", "Install downloaded workshop items into an environment's client and/or server mods (test in lab, then install the same ids into stable to promote)", [req("ids", K::List, "workshop ids"), p("client", K::Bool, "into the client mods"), p("server", K::Bool, "into the server mods")], env = true, writes = false, mutates = true, ws_install),
    cmd!("schema", "All commands with parameters as JSON", [], env = false, writes = false, mutates = false, schema),
    cmd!("version", "Version and file locations", [], env = false, writes = false, mutates = false, version),
];

pub fn find(name: &str) -> Option<&'static Cmd> {
    let n = name.replace(['_', '-'], ".");
    COMMANDS.iter().find(|c| c.name == name || c.name == n || c.name.replace('.', "_") == name)
}

pub fn params(c: &Cmd) -> Vec<&P> {
    let mut v: Vec<&P> = c.params.iter().collect();
    if c.env {
        v.push(if c.params.iter().any(|p| p.positional) { &ENV_P } else { &ENV_POS });
    }
    if guards_user_dir(c) {
        v.push(&ALLOW_P);
    }
    v
}

fn kind_schema(k: K) -> Value {
    match k {
        K::Str => json!({"type": "string"}),
        K::Int => json!({"type": "integer"}),
        K::Bool => json!({"type": "boolean"}),
        K::List => json!({"type": "array", "items": {"type": "string"}}),
    }
}

pub fn input_schema(c: &Cmd) -> Value {
    let mut props = Map::new();
    let mut required = vec![];
    for p in params(c) {
        let mut s = kind_schema(p.kind);
        if !p.help.is_empty() {
            s["description"] = json!(p.help);
        }
        props.insert(p.name.to_string(), s);
        if p.required {
            required.push(p.name);
        }
    }
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

fn cmd_schema(c: &Cmd) -> Value {
    json!({"name": c.name, "description": c.help, "mutates": c.mutates, "input": input_schema(c)})
}

/// Run a command with JSON arguments (shared by the CLI and MCP)
pub fn execute(c: &Cmd, args: Map<String, Value>) -> Result<Value> {
    let known: Vec<&str> = params(c).iter().map(|p| p.name).collect();
    if let Some(k) = args.keys().find(|k| !known.contains(&k.as_str())) {
        bail!("unknown parameter '{k}' for {} (accepted: {})", c.name, known.join(", "));
    }
    for p in params(c) {
        if p.required && args.get(p.name).is_none_or(|v| v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty()) || v.as_array().is_some_and(|a| a.is_empty())) {
            bail!("missing parameter: {}", p.name);
        }
    }
    let a = Args(args);
    // the lock is held across load → change → save so the GUI or another agent cannot interleave
    let _lock = if c.writes { Some(Store::lock()?) } else { None };
    let mut ctx = Ctx { store: Store::load_strict()? };
    if guards_user_dir(c) && !a.flag("allow_user_dir") {
        let e = ctx.env(&a)?;
        if same_dir(&e.cache_dir, &crate::platform::zomboid_user_dir()) {
            bail!(
                "environment '{}' uses {} as its server cachedir: that is the game's default user folder (the user's own saves and servers). \
                 Create a separate environment instead (env add <id> --cache-dir <folder> --client-cache-dir <folder>), \
                 or pass allow_user_dir only if the user explicitly asked to work there",
                e.key(),
                e.cache_dir.display()
            );
        }
    }
    let out = (c.run)(&mut ctx, &a)?;
    if c.writes {
        ctx.store.try_save()?;
    }
    Ok(out)
}

// ---------- argv ----------

/// `group action [positional…] [--key value | --key=value | --flag | --no-flag] [--json '{…}']`
pub fn parse_argv(argv: &[String]) -> Result<(&'static Cmd, Map<String, Value>)> {
    let (cmd, rest) = match argv {
        [a, b, rest @ ..] if find(&format!("{a}.{b}")).is_some() => (find(&format!("{a}.{b}")).unwrap(), rest),
        [a, rest @ ..] if find(a).is_some() => (find(a).unwrap(), rest),
        [_, ..] => bail!("unknown command '{}' (try: help)", argv.iter().take(2).cloned().collect::<Vec<_>>().join(" ")),
        [] => bail!("no command"),
    };
    let ps = params(cmd);
    let mut out = Map::new();
    let mut positional: Vec<String> = vec![];
    let mut i = 0;
    while i < rest.len() {
        let t = &rest[i];
        if t == "--" {
            positional.extend(rest[i + 1..].iter().cloned());
            break;
        }
        if let Some(body) = t.strip_prefix("--") {
            let (key, inline) = match body.split_once('=') {
                Some((k, v)) => (k.replace('-', "_"), Some(v.to_string())),
                None => (body.replace('-', "_"), None),
            };
            if key == "json" {
                let v = inline.or_else(|| rest.get(i + 1).cloned()).ok_or_else(|| anyhow!("--json needs a value"))?;
                if !t.contains('=') {
                    i += 1;
                }
                match serde_json::from_str::<Value>(&v)? {
                    Value::Object(m) => out.extend(m),
                    _ => bail!("--json must be an object"),
                }
                i += 1;
                continue;
            }
            let (key, negated) = match key.strip_prefix("no_") {
                Some(k) if ps.iter().any(|p| p.name == k && p.kind == K::Bool) => (k.to_string(), true),
                _ => (key, false),
            };
            let p = ps.iter().find(|p| p.name == key).ok_or_else(|| anyhow!("unknown option --{} for {} (accepted: {})", key.replace('_', "-"), cmd.name, ps.iter().map(|p| format!("--{}", p.name.replace('_', "-"))).collect::<Vec<_>>().join(" ")))?;
            let val = match (p.kind, inline) {
                (K::Bool, Some(v)) => json!(matches!(v.as_str(), "1" | "true" | "yes" | "on")),
                (K::Bool, None) => json!(!negated),
                (_, Some(v)) => json!(v),
                (_, None) => {
                    i += 1;
                    json!(rest.get(i).ok_or_else(|| anyhow!("--{} needs a value", key.replace('_', "-")))?)
                }
            };
            if p.kind == K::List {
                let s = val.as_str().unwrap_or_default().to_string();
                match out.entry(key).or_insert_with(|| json!([])) {
                    Value::Array(a) => a.push(json!(s)),
                    v => *v = json!([s]),
                }
            } else {
                out.insert(key, val);
            }
        } else {
            positional.push(t.clone());
        }
        i += 1;
    }
    let slots: Vec<&&P> = ps.iter().filter(|p| p.positional && !out.contains_key(p.name)).collect();
    let mut pi = positional.into_iter().peekable();
    let mut si = 0;
    while pi.peek().is_some() {
        let Some(p) = slots.get(si) else {
            bail!("too many arguments for {}: {}", cmd.name, pi.collect::<Vec<_>>().join(" "));
        };
        si += 1;
        if p.kind == K::List {
            out.insert(p.name.into(), json!(pi.by_ref().collect::<Vec<_>>()));
        } else if si == slots.len() && p.kind == K::Str && p.name != "env" {
            // the last text slot takes the rest: `server cmd servermsg "hi all"` works without quoting
            out.insert(p.name.into(), json!(pi.by_ref().collect::<Vec<_>>().join(" ")));
        } else {
            out.insert(p.name.into(), json!(pi.next()));
        }
    }
    Ok((cmd, out))
}

fn help_text() -> String {
    let mut s = String::from(
        "pz-mod-manager — Project Zomboid mod/server manager\n\n\
         Without arguments (or with `gui`) the GUI starts. Otherwise:\n  \
         pz-mod-manager <group> <action> [args…] [--option value] [--flag] [--json '{…}']\n  \
         pz-mod-manager mcp            MCP server on stdio (tools = commands below)\n  \
         pz-mod-manager help <command> parameters of one command\n\n\
         Output is JSON: {\"ok\":true,\"result\":…} or {\"ok\":false,\"error\":…} (exit code 1).\n\
         Most commands take --env <id> (stable, lab, …); default is the active environment.\n\nCommands:\n",
    );
    for c in COMMANDS {
        s.push_str(&format!("  {:<18} {}\n", c.name.replace('.', " "), c.help));
    }
    s
}

fn cmd_help(c: &Cmd) -> String {
    let mut s = format!("{} — {}\n\n", c.name.replace('.', " "), c.help);
    for p in params(c) {
        let kind = match p.kind {
            K::Str => "<text>",
            K::Int => "<number>",
            K::Bool => "",
            K::List => "<a,b,…>",
        };
        let how = if p.positional { format!("[{}] or --{} {kind}", p.name, p.name.replace('_', "-")) } else { format!("--{} {kind}", p.name.replace('_', "-")) };
        s.push_str(&format!("  {:<34} {}{}\n", how, if p.required { "(required) " } else { "" }, p.help));
    }
    s
}

pub fn print(v: &Value) {
    let s = if std::io::stdout().is_terminal() { serde_json::to_string_pretty(v) } else { serde_json::to_string(v) };
    println!("{}", s.unwrap_or_default());
}

/// Entry point for `pz-mod-manager <args…>`; returns the process exit code
pub fn main(argv: Vec<String>) -> i32 {
    #[cfg(windows)]
    attach_console();
    // agents get stable English messages; PZMM_LANG=ru for Russian
    crate::i18n::apply_setting(&std::env::var("PZMM_LANG").unwrap_or_else(|_| "en".into()));
    match argv[0].as_str() {
        "help" | "--help" | "-h" => {
            match argv.get(1).and_then(|n| find(&argv[1..].join(".")).or_else(|| find(n))) {
                Some(c) => print!("{}", cmd_help(c)),
                None => print!("{}", help_text()),
            }
            return 0;
        }
        "mcp" => return crate::mcp::serve(),
        "--version" | "-V" => {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            return 0;
        }
        _ => {}
    }
    if argv.iter().any(|a| a == "--help" || a == "-h") {
        if let Ok((c, _)) = parse_argv(&argv.iter().filter(|a| *a != "--help" && *a != "-h").cloned().collect::<Vec<_>>()) {
            print!("{}", cmd_help(c));
            return 0;
        }
    }
    let r = parse_argv(&argv).and_then(|(c, args)| execute(c, args));
    match r {
        Ok(v) => {
            print(&json!({"ok": true, "result": v}));
            0
        }
        Err(e) => {
            print(&json!({"ok": false, "error": format!("{e:#}")}));
            1
        }
    }
}

/// Release builds on Windows are GUI-subsystem binaries: attach to the parent console so CLI output is visible
#[cfg(windows)]
fn attach_console() {
    unsafe extern "system" {
        fn AttachConsole(pid: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn parses_groups_positionals_options_and_flags() {
        let (c, a) = parse_argv(&argv("server start --env lab --wait --timeout 30")).unwrap();
        assert_eq!(c.name, "server.start");
        assert_eq!(a["env"], "lab");
        assert_eq!(a["wait"], true);
        assert_eq!(a["timeout"], "30");
        let (c, a) = parse_argv(&argv("profile enable ModA ModB --position=2")).unwrap();
        assert_eq!(c.name, "profile.enable");
        assert_eq!(a["ids"], json!(["ModA", "ModB"]));
        assert_eq!(a["position"], "2");
        let (_, a) = parse_argv(&argv("server cmd servermsg hello --env=stable")).unwrap();
        assert_eq!(a["command"], "servermsg hello");
        let (_, a) = parse_argv(&argv("ini set DefaultPort=16271 UDPPort=16272")).unwrap();
        assert_eq!(a["values"], json!(["DefaultPort=16271", "UDPPort=16272"]));
        let (_, a) = parse_argv(&argv("server start lab --no-nosteam")).unwrap();
        assert_eq!((a["env"].as_str(), a["nosteam"].as_bool()), (Some("lab"), Some(false)));
        let (_, a) = parse_argv(&argv("mods list --no-enabled-only")).unwrap();
        assert_eq!(a["enabled_only"], false);
        let (c, a) = parse_argv(&["server_status".to_string(), "--json".to_string(), "{\"env\":\"lab\"}".to_string()]).unwrap();
        assert_eq!(c.name, "server.status");
        assert_eq!(a["env"], "lab");
    }

    #[test]
    fn env_is_positional_only_where_nothing_else_is() {
        let (c, a) = parse_argv(&argv("env use lab")).unwrap();
        assert_eq!((c.name, a["env"].as_str()), ("env.use", Some("lab")));
        let (_, a) = parse_argv(&argv("server start lab --wait")).unwrap();
        assert_eq!(a["env"], "lab");
        let (_, a) = parse_argv(&argv("server cmd players --env lab")).unwrap();
        assert_eq!((a["command"].as_str(), a["env"].as_str()), (Some("players"), Some("lab")));
        assert!(parse_argv(&argv("env use lab extra")).is_err());
    }

    #[test]
    fn user_zomboid_dir_is_guarded_for_server_side_changes_only() {
        for (name, guarded) in [("server.start", true), ("ini.apply", true), ("backup.restore", true), ("workshop.install", true), ("server.status", false), ("client.start", false), ("env.set", false), ("profile.enable", false), ("ini.import", false)] {
            assert_eq!(guards_user_dir(find(name).unwrap()), guarded, "{name}");
        }
        assert!(params(find("server.start").unwrap()).iter().any(|p| p.name == "allow_user_dir"));
        assert!(same_dir(std::path::Path::new("/a/b/"), std::path::Path::new("/a/b")));
        // a fresh config: the default environment points at ~/Zomboid → refused before anything runs
        let _l = Store::lock().unwrap();
        let _ = std::fs::remove_file(Store::path());
        let e = execute(find("server.start").unwrap(), Map::new()).unwrap_err().to_string();
        assert!(e.contains("default user folder"), "{e}");
        let e = execute(find("ini.set").unwrap(), Map::from_iter([("values".to_string(), json!(["A=1"]))])).unwrap_err().to_string();
        assert!(e.contains("default user folder"), "{e}");
    }

    #[test]
    fn rejects_unknown_things() {
        assert!(parse_argv(&argv("nope")).is_err());
        assert!(parse_argv(&argv("server start --bogus 1")).is_err());
        assert!(parse_argv(&argv("env list extra")).is_err());
        let c = find("env.list").unwrap();
        assert!(execute(c, Map::from_iter([("x".to_string(), json!(1))])).is_err());
        assert!(execute(find("profile.enable").unwrap(), Map::new()).unwrap_err().to_string().contains("ids"));
    }

    #[test]
    fn args_helpers_accept_strings_from_the_cli_and_typed_json_from_mcp() {
        let a = Args(Map::from_iter([
            ("n".to_string(), json!("5")),
            ("m".to_string(), json!(7)),
            ("b".to_string(), json!("true")),
            ("l".to_string(), json!(["a,b", "\\c"])),
        ]));
        assert_eq!(a.int("n"), Some(5));
        assert_eq!(a.int("m"), Some(7));
        assert!(a.flag("b"));
        assert_eq!(a.list("l"), ["a", "b", "c"]);
    }

    #[test]
    fn every_command_has_a_valid_schema_and_unique_name() {
        let mut seen = HashSet::new();
        for c in COMMANDS {
            assert!(seen.insert(c.name), "duplicate {}", c.name);
            assert!(c.name.contains('.') || matches!(c.name, "schema" | "version"));
            let s = input_schema(c);
            for r in s["required"].as_array().unwrap() {
                assert!(s["properties"][r.as_str().unwrap()].is_object());
            }
            assert!(!c.writes || c.mutates, "{} writes config but is marked read-only", c.name);
        }
    }

    #[test]
    fn env_lookup_and_config_roundtrip() {
        let _l = Store::lock().unwrap();
        let mut st = Store::default();
        st.settings.servers[0].id = "stable".into();
        st.settings.servers[0].cache_dir = "/tmp/pzmm-a".into();
        let mut lab = ServerCfg::default();
        lab.id = String::new(); // key derived from the label
        lab.label = "Lab Server".into();
        lab.cache_dir = "/tmp/pzmm-b".into();
        lab.client_cache_dir = "/tmp/pzmm-bc".into();
        st.settings.servers.push(lab);
        assert_eq!(st.settings.find_env("STABLE"), Some(0));
        assert_eq!(st.settings.find_env("lab-server"), Some(1));
        assert_eq!(st.settings.find_env("Lab Server"), Some(1));
        assert_eq!(st.settings.find_env("1"), Some(1));
        assert_eq!(st.settings.find_env("zzz"), None);
        assert_eq!(st.settings.servers[1].client_mods_dir(&st.settings), PathBuf::from("/tmp/pzmm-bc/mods"));
        assert_eq!(st.settings.servers[0].client_mods_dir(&st.settings), st.settings.client_mods_dir);
        check_unique(&st.settings).unwrap();
        st.try_save().unwrap();
        let back = Store::load_strict().unwrap();
        assert_eq!(back.settings.servers[1].client_cache_dir, PathBuf::from("/tmp/pzmm-bc"));
        std::fs::write(Store::path(), "not = [valid").unwrap();
        assert!(Store::load_strict().is_err(), "a broken config must not silently become defaults");
        let _ = std::fs::remove_file(Store::path());
    }
}
