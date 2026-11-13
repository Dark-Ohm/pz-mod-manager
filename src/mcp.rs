//! MCP server over stdio (newline-delimited JSON-RPC 2.0). Every CLI command is a tool:
//! `server.start` → `server_start`, with the same parameters as JSON.
//! Register with e.g. `claude mcp add pz-manager -- /path/to/pz-mod-manager mcp`.
use crate::cli;
use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};

const VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Project Zomboid server/mod manager. \
An environment (env) is one server + one game client that share one game install but have separate \
-cachedir folders (e.g. stable and lab). Most tools take `env` (id from env_list; default = active). \
Typical flow: env_list → mods_check → profile_enable/profile_set_order → ini_apply → server_start (wait) → \
server_cmd / server_log → server_stop. Profiles are ordered mod lists shared by all environments. \
Mutating tools change real files: back up with backup_create before risky changes. \
State is shared with the GUI and other agents; re-read status instead of assuming it.";

fn tools() -> Value {
    json!(cli::COMMANDS
        .iter()
        .filter(|c| c.name != "schema")
        .map(|c| json!({
            "name": c.name.replace('.', "_"),
            "description": c.help,
            "inputSchema": cli::input_schema(c),
            "annotations": {"readOnlyHint": !c.mutates, "openWorldHint": c.name.starts_with("workshop.")},
        }))
        .collect::<Vec<_>>())
}

fn call(params: &Value) -> Value {
    let name = params["name"].as_str().unwrap_or_default();
    let args = match &params["arguments"] {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    let r = match cli::find(name) {
        Some(c) => cli::execute(c, args),
        None => Err(anyhow::anyhow!("unknown tool {name}")),
    };
    match r {
        Ok(v) => json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}], "isError": false}),
        Err(e) => json!({"content": [{"type": "text", "text": format!("{e:#}")}], "isError": true}),
    }
}

/// One JSON-RPC message → optional response (notifications get none)
pub fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let method = msg["method"].as_str().unwrap_or_default();
    let result = match method {
        "initialize" => {
            let asked = msg["params"]["protocolVersion"].as_str().unwrap_or_default();
            let v = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[0]);
            Ok(json!({
                "protocolVersion": v,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "pz-mod-manager", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => Ok(call(&msg["params"])),
        "resources/list" => Ok(json!({"resources": []})),
        "prompts/list" => Ok(json!({"prompts": []})),
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, m)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": m}}),
    })
}

pub fn serve() -> i32 {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let r: Vec<Value> = batch.iter().filter_map(handle).collect();
                (!r.is_empty()).then(|| json!(r))
            }
            Ok(msg) => handle(&msg),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}})),
        };
        if let Some(r) = reply {
            if writeln!(out, "{r}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_list_and_call() {
        let init = handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        let list = handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"server_start") && names.contains(&"mods_check") && !names.contains(&"schema"));
        for t in list["result"]["tools"].as_array().unwrap() {
            assert!(t["name"].as_str().unwrap().chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
            assert_eq!(t["inputSchema"]["type"], "object");
        }
        let bad = handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "nope"}})).unwrap();
        assert_eq!(bad["result"]["isError"], true);
        let v = handle(&json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "version", "arguments": {}}})).unwrap();
        assert_eq!(v["result"]["isError"], false);
        let unknown = handle(&json!({"jsonrpc": "2.0", "id": 5, "method": "x/y"})).unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
    }
}
