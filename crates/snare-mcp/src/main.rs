//! `snare-mcp` — Model Context Protocol server over stdio (§10).
//!
//! Exposes Snare's captured flows to AI agents. Phase-0 transport is a small
//! hand-rolled JSON-RPC 2.0 loop (newline-delimited) — no external MCP SDK — so
//! it is dependency-light and stable. It reads the SQLite store directly.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::Result;
use serde_json::{json, Value};
use snare_core::store::{FlowQuery, FlowStore};
use snare_store_sqlite::SqliteStore;

const PROTOCOL_VERSION: &str = "2024-11-05";

fn db_path() -> PathBuf {
    if let Ok(home) = std::env::var("SNARE_HOME") {
        return PathBuf::from(home).join("data").join("flows.sqlite");
    }
    directories::ProjectDirs::from("dev", "Snare", "snare")
        .map(|pd| pd.data_dir().join("flows.sqlite"))
        .unwrap_or_else(|| PathBuf::from("flows.sqlite"))
}

fn main() -> Result<()> {
    let store = SqliteStore::open(db_path())?;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[snare-mcp] bad json: {e}");
                continue;
            }
        };

        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        eprintln!("[snare-mcp] -> {method}");

        // Notifications (no id) get no reply.
        let Some(id) = id else {
            continue;
        };

        let reply = match handle(method, msg.get("params"), &store) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32603, "message": e.to_string() }
            }),
        };
        writeln!(stdout, "{}", serde_json::to_string(&reply)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn handle(method: &str, params: Option<&Value>, store: &SqliteStore) -> Result<Value> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "snare-mcp", "version": env!("CARGO_PKG_VERSION") }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_specs() })),
        "tools/call" => tools_call(params, store),
        other => anyhow::bail!("method not found: {other}"),
    }
}

fn tool_specs() -> Value {
    json!([
        {
            "name": "proxy_list_flows",
            "description": "List captured HTTP flows (newest first). Optional case-insensitive substring search over method/host/path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "search": { "type": "string", "description": "substring filter" },
                    "limit": { "type": "integer", "description": "max rows (default 50)" }
                }
            }
        },
        {
            "name": "proxy_get_flow",
            "description": "Fetch one full flow (request + response) by id. `part` = request|response|all (default all).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "part": { "type": "string", "enum": ["request", "response", "all"] }
                },
                "required": ["id"]
            }
        },
        {
            "name": "proxy_stats",
            "description": "Total number of flows captured.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

fn text_result(text: String) -> Value {
    json!({ "content": [ { "type": "text", "text": text } ] })
}

fn tools_call(params: Option<&Value>, store: &SqliteStore) -> Result<Value> {
    let params = params.cloned().unwrap_or_else(|| json!({}));
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

    match name {
        "proxy_stats" => {
            let n = store.count()?;
            Ok(text_result(json!({ "flows": n }).to_string()))
        }
        "proxy_list_flows" => {
            let q = FlowQuery {
                search: args.get("search").and_then(|s| s.as_str()).map(String::from),
                host: None,
                limit: args.get("limit").and_then(|l| l.as_i64()).unwrap_or(50),
                offset: 0,
            };
            let flows = store.list_flows(&q)?;
            let rows: Vec<Value> = flows
                .iter()
                .map(|f| {
                    json!({
                        "id": f.id,
                        "method": f.method,
                        "url": format!("{}://{}{}", f.scheme, f.host, f.path),
                        "status": f.status,
                        "mime": f.mime,
                        "size": f.resp_size,
                        "ms": f.duration_ms
                    })
                })
                .collect();
            Ok(text_result(serde_json::to_string_pretty(&rows)?))
        }
        "proxy_get_flow" => {
            let id = args
                .get("id")
                .and_then(|i| i.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing `id`"))?;
            let part = args.get("part").and_then(|p| p.as_str()).unwrap_or("all");
            let flow = store
                .get_flow(id)?
                .ok_or_else(|| anyhow::anyhow!("flow {id} not found"))?;
            let mut out = serde_json::Map::new();
            out.insert("id".into(), json!(flow.id));
            if part == "request" || part == "all" {
                out.insert(
                    "request".into(),
                    json!({
                        "method": flow.request.method,
                        "url": flow.request.url(),
                        "http_version": flow.request.http_version,
                        "headers": flow.request.headers,
                        "body": String::from_utf8_lossy(&flow.request.body),
                    }),
                );
            }
            if part == "response" || part == "all" {
                if let Some(resp) = &flow.response {
                    out.insert(
                        "response".into(),
                        json!({
                            "status": resp.status,
                            "headers": resp.headers,
                            "body": String::from_utf8_lossy(&resp.body),
                        }),
                    );
                }
            }
            Ok(text_result(serde_json::to_string_pretty(&out)?))
        }
        other => anyhow::bail!("unknown tool: {other}"),
    }
}
