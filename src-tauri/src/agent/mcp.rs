//! MCP server (stdio JSON-RPC) — expose lumen-cut as tools for Claude.
//!
//! Claude (or any MCP client) launches `lumen-cut-cli mcp serve` and talks
//! JSON-RPC over stdio. Tools mirror the read-side CLI: project_list /
//! project_show / audit / finish_check / version_list / cut_list /
//! subtitle_list / export. The HTTP claim/submit worker protocol lives in
//! `agent::http`; this stdio server lets any MCP client drive lumen-cut without
//! shelling out per command.

use std::io::BufRead;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::error::AppResult;

/// Run the MCP server over stdio. Reads JSON-RPC requests line-by-line,
/// writes one response per line.
pub fn run_stdio() -> AppResult<()> {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let resp = handle(req)?;
        if resp != Value::Null {
            println!("{}", resp);
        }
    }
    Ok(())
}

fn handle(req: Value) -> AppResult<Value> {
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let ok = |result: Value| json!({"jsonrpc":"2.0","id":id,"result":result});
    match method {
        "initialize" => Ok(ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "lumen-cut", "version": crate::VERSION}
        }))),
        "notifications/initialized" => Ok(Value::Null),
        "tools/list" => Ok(ok(json!({"tools": tools()}))),
        "tools/call" => {
            let name = req
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args = req
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            match call_tool(name, &args) {
                Ok(text) => Ok(ok(json!({"content":[{"type":"text","text":text}]}))),
                Err(e) => Ok(ok(json!({
                    "isError": true,
                    "content":[{"type":"text","text":format!("{e:#}")}]
                }))),
            }
        }
        _ => {
            Ok(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"unknown method"}}))
        }
    }
}

fn tools() -> Vec<Value> {
    fn tool(name: &str, desc: &str, required: &[&str]) -> Value {
        let props: Value = required
            .iter()
            .map(|p| (p.to_string(), json!({"type":"string"})))
            .collect();
        json!({
            "name": name,
            "description": desc,
            "inputSchema": {
                "type": "object",
                "properties": props,
                "required": required,
            }
        })
    }
    vec![
        tool(
            "project_list",
            "List lumen-cut projects under root",
            &["root"],
        ),
        tool(
            "project_show",
            "Show doc.json for a project",
            &["pid", "root"],
        ),
        tool("audit", "Run the 56-code audit", &["pid", "root"]),
        tool("finish_check", "Run finish-check", &["pid", "root"]),
        tool("version_list", "List version lineage", &["pid", "root"]),
        tool("cut_list", "List soft cuts", &["pid", "root"]),
        tool("subtitle_list", "List subtitles", &["pid", "root"]),
        tool("export", "Export srt/vtt/ass/md + cues", &["pid", "root"]),
    ]
}

fn call_tool(name: &str, args: &Value) -> AppResult<String> {
    let root = PathBuf::from(args.get("root").and_then(|v| v.as_str()).unwrap_or("."));
    let pid = args.get("pid").and_then(|v| v.as_str()).unwrap_or(".");
    let dir = if name == "project_list" {
        root.clone()
    } else {
        root.join(pid)
    };
    match name {
        "project_list" => {
            let mut pids: Vec<String> = std::fs::read_dir(&root)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|n| root.join(n).join("doc.json").exists())
                        .collect()
                })
                .unwrap_or_default();
            pids.sort();
            Ok(serde_json::to_string_pretty(&pids)?)
        }
        "project_show" => {
            let doc = crate::data::Doc::load(&dir)?;
            Ok(serde_json::to_string_pretty(&doc)?)
        }
        "audit" => {
            let doc = crate::data::Doc::load(&dir)?;
            let cuts: crate::data::soft_cut::ClipCuts =
                std::fs::read_to_string(dir.join("cuts.json"))
                    .ok()
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default();
            let broll = crate::data::broll::load(&dir)?;
            let r = crate::audit::audit_project(&doc, &cuts, &broll, &dir);
            Ok(serde_json::to_string_pretty(&r)?)
        }
        "finish_check" => {
            let doc = crate::data::Doc::load(&dir)?;
            let cuts: crate::data::soft_cut::ClipCuts =
                std::fs::read_to_string(dir.join("cuts.json"))
                    .ok()
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default();
            let committed = crate::data::version::working_head_is_committed(&dir, &doc)?;
            let broll = crate::data::broll::load(&dir)?;
            let items =
                crate::audit::finish_check_emit_for_project(&doc, &cuts, &broll, &dir, committed);
            let v: Vec<Value> = items
                .iter()
                .map(|it| {
                    json!({
                        "code": it.code.label(),
                        "pass": it.pass,
                        "blockers": it.blockers.len(),
                        "warnings": it.warnings.len(),
                    })
                })
                .collect();
            Ok(serde_json::to_string_pretty(&v)?)
        }
        "version_list" => {
            let lin = crate::data::version::Lineage::load(&dir)?;
            Ok(serde_json::to_string_pretty(&lin)?)
        }
        "cut_list" => {
            let cuts: crate::data::soft_cut::ClipCuts =
                std::fs::read_to_string(dir.join("cuts.json"))
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
            Ok(serde_json::to_string_pretty(&cuts)?)
        }
        "subtitle_list" => {
            let doc = crate::data::Doc::load(&dir)?;
            let rows =
                crate::data::subtitle::list(&doc, &crate::data::subtitle::load_hidden(&dir), None);
            Ok(serde_json::to_string_pretty(&rows)?)
        }
        "export" => {
            let doc = crate::data::Doc::load(&dir)?;
            let cuts: crate::data::soft_cut::ClipCuts =
                std::fs::read_to_string(dir.join("cuts.json"))
                    .ok()
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default();
            crate::export::write_srt_with(&doc, &cuts.cuts, &dir.join("export.srt"))?;
            crate::export::write_vtt_with(&doc, &cuts.cuts, &dir.join("export.vtt"))?;
            crate::export::write_ass_with(&doc, &cuts.cuts, &dir.join("export.ass"), 1920, 1080)?;
            crate::export::write_md_with(&doc, &cuts.cuts, &dir.join("export.md"))?;
            crate::data::cues::save(&dir, &crate::data::cues::to_cues(&doc, None))?;
            Ok(format!("exported srt+vtt+ass+md+cues to {}", dir.display()))
        }
        _ => Err(crate::error::AppError::Schema(format!(
            "unknown MCP tool: {name}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_returns_server_info() {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"initialize"});
        let resp = handle(req).unwrap();
        assert_eq!(resp["result"]["serverInfo"]["name"], "lumen-cut");
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    }

    #[test]
    fn tools_list_advertises_eight_tools() {
        let req = json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
        let resp = handle(req).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert!(tools.len() >= 8);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"audit"));
        assert!(names.contains(&"export"));
    }

    #[test]
    fn unknown_tool_returns_mcp_tool_error() {
        let req = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"nope","arguments":{}}});
        let resp = handle(req).unwrap();
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("unknown MCP tool"));
    }

    #[test]
    fn notifications_initialized_is_null() {
        let req = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        let resp = handle(req).unwrap();
        assert_eq!(resp, Value::Null);
    }

    #[test]
    fn export_uses_the_same_root_plus_pid_resolution_as_other_project_tools() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("demo");
        let doc = crate::data::Doc {
            id: "demo".into(),
            schema: 1,
            media: crate::data::MediaRef {
                path: "input.mp4".into(),
                duration_seconds: 1.0,
                sample_rate: None,
                channels: None,
            },
            meta: crate::data::Meta {
                title: "demo".into(),
                description: String::new(),
                language: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            paragraphs: Vec::new(),
            translations: Default::default(),
        };
        doc.save(&project).unwrap();
        let args = json!({
            "pid": "demo",
            "root": temp.path().to_string_lossy(),
        });
        let result = call_tool("export", &args).unwrap();
        assert!(result.contains(&project.display().to_string()));
        assert!(project.join("export.srt").exists());
        assert!(project.join("cues.json").exists());
    }
}
