//! `twr mcp`: minimal JSON-RPC 2.0 stdio server (bead twitter_cli-5o3.7.5).
//!
//! Scope decision (documented): the `rmcp` SDK's server/tool macros are not
//! used — hand-rolled stdio keeps zero new deps and stays testable offline.
//! The tool catalog mirrors `twr commands` output (same names + envelope
//! types) rather than duplicating it by hand: [`tool_catalog`] is built from
//! the same source.
//!
//! Protocol (MCP-shaped JSON-RPC over stdio, one object per line):
//! - `{"jsonrpc":"2.0","id":1,"method":"initialize"}` → server info.
//! - `{"jsonrpc":"2.0","id":2,"method":"tools/list"}` → tool catalog.
//! - `{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"status","arguments":{}}}`
//!   → `{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"<envelope JSON>"}]}}`
//! - `notifications/initialized` → acknowledged, no response.
//! - Unknown method → JSON-RPC `-32601 Method not found`.
//!
//! Tools return the envelope JSON as text (same contract as the CLI).

use std::io::BufRead;

/// Tool descriptor: name + description + envelope type. Mirrors `twr commands`.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub envelope_type: &'static str,
}

pub fn tool_catalog() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "status",
            description: "Auth gate — call first",
            envelope_type: "status",
        },
        ToolDef {
            name: "doctor",
            description: "Health checks",
            envelope_type: "doctor",
        },
        ToolDef {
            name: "query_ids",
            description: "Resolved query IDs per op",
            envelope_type: "query-ids",
        },
        ToolDef {
            name: "feed",
            description: "Home/feed timeline",
            envelope_type: "tweet_list",
        },
        ToolDef {
            name: "search",
            description: "Search with operator flags",
            envelope_type: "tweet_list",
        },
        ToolDef {
            name: "tweet",
            description: "Single tweet by ID/URL",
            envelope_type: "tweet_detail",
        },
        ToolDef {
            name: "show",
            description: "Nth item of last list",
            envelope_type: "tweet_detail",
        },
        ToolDef {
            name: "article",
            description: "Long-form article",
            envelope_type: "article",
        },
        ToolDef {
            name: "list",
            description: "List timeline",
            envelope_type: "tweet_list",
        },
        ToolDef {
            name: "user",
            description: "Profile by handle",
            envelope_type: "user",
        },
        ToolDef {
            name: "user_posts",
            description: "Posts by handle",
            envelope_type: "tweet_list",
        },
        ToolDef {
            name: "followers",
            description: "Followers of user id",
            envelope_type: "user_list",
        },
        ToolDef {
            name: "following",
            description: "Following of user id",
            envelope_type: "user_list",
        },
        ToolDef {
            name: "headlines",
            description: "Today's headlines (fallback)",
            envelope_type: "headline_list",
        },
        ToolDef {
            name: "cache_search",
            description: "Local SQLite cache search",
            envelope_type: "tweet_list",
        },
        ToolDef {
            name: "watch_list",
            description: "List watched handles",
            envelope_type: "watchlist",
        },
    ]
}

/// Dispatch one parsed request object → optional response object.
/// `invoke` runs the tool and returns the envelope JSON string.
pub fn handle_request(
    req: &serde_json::Value,
    invoke: &dyn Fn(&str, &serde_json::Value) -> String,
) -> Option<serde_json::Value> {
    let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = req.get("method")?.as_str()?;
    match method {
        "initialize" => Some(serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "serverInfo": {"name": "twr", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"tools": {}},
            },
        })),
        "notifications/initialized" => None,
        "tools/list" => Some(serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "tools": tool_catalog().iter().map(|t| serde_json::json!({
                    "name": t.name,
                    "description": format!("{} (envelope: {})", t.description, t.envelope_type),
                    "inputSchema": {"type": "object"},
                })).collect::<Vec<_>>(),
            },
        })),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or_default();
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_default();
            if tool_catalog().iter().all(|t| t.name != name) {
                return Some(serde_json::json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": {"code": -32602, "message": format!("unknown tool: {name}")},
                }));
            }
            let text = invoke(name, &args);
            Some(serde_json::json!({
                "jsonrpc": "2.0", "id": id,
                "result": {"content": [{"type": "text", "text": text}]},
            }))
        }
        _ => Some(serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": -32601, "message": format!("method not found: {method}")},
        })),
    }
}

/// Run the stdio loop: read lines on stdin, write responses on stdout.
/// `invoke` dispatches tool calls (wired to the real runners in main.rs).
pub fn serve_stdio(invoke: &dyn Fn(&str, &serde_json::Value) -> String) {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                println!(
                    "{}",
                    serde_json::json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}})
                );
                continue;
            }
        };
        if let Some(resp) = handle_request(&req, invoke) {
            println!("{resp}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo_invoke(name: &str, _args: &serde_json::Value) -> String {
        serde_json::json!({"tool": name}).to_string()
    }

    #[test]
    fn initialize_lists_and_calls() {
        let init = handle_request(
            &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
            &echo_invoke,
        )
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "twr");

        let list = handle_request(
            &serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            &echo_invoke,
        )
        .unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        assert!(tools.len() >= 10);
        assert!(tools.iter().any(|t| t["name"] == "search"));

        let call = handle_request(
            &serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "status", "arguments": {}}}),
            &echo_invoke,
        )
        .unwrap();
        assert!(call["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("status"));
    }

    #[test]
    fn unknown_tool_and_method_error() {
        let bad_tool = handle_request(
            &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": "nope", "arguments": {}}}),
            &echo_invoke,
        )
        .unwrap();
        assert_eq!(bad_tool["error"]["code"], -32602);
        let bad_method = handle_request(
            &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "zzz"}),
            &echo_invoke,
        )
        .unwrap();
        assert_eq!(bad_method["error"]["code"], -32601);
        assert!(handle_request(
            &serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            &echo_invoke,
        )
        .is_none());
    }
}
