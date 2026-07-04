//! xlang-lsp — a minimal Language Server (LSP) over stdio, pure Rust (no tokio
//! / tower-lsp, so `cargo build` stays light). Provides: live diagnostics on
//! document open/change, hover (signature), go-to-definition, and completion.
//! Uses the Phase 1 diagnostics + Phase 2 symbol index via `xlang::lsp`.
//!
//! Wire it into an editor as a language server for `.x` files.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};

use serde_json::{Value, json};

use xlang::lsp;

fn main() {
    let mut docs: HashMap<String, String> = HashMap::new();
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    while let Some(msg) = read_message(&mut reader) {
        if !handle(&msg, &mut docs) {
            break; // exit
        }
    }
}

/// Read one LSP message: `Content-Length: N` headers, blank line, then N bytes.
fn read_message<R: BufRead>(r: &mut R) -> Option<Value> {
    let mut content_len: Option<usize> = None;
    let mut line = String::new();
    loop {
        line.clear();
        let n = r.read_line(&mut line).ok()?;
        if n == 0 {
            return None; // EOF
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // end of headers
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_len = rest.trim().parse().ok();
        }
    }
    let len = content_len?;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn send(msg: &Value) {
    let body = serde_json::to_string(msg).unwrap_or_else(|_| "{}".into());
    let mut out = io::stdout().lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = out.flush();
}

/// Handle one message. Returns false to stop the server (on `exit`).
fn handle(msg: &Value, docs: &mut HashMap<String, String>) -> bool {
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => send(&json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "capabilities": {
                    "textDocumentSync": 1, // full sync
                    "hoverProvider": true,
                    "definitionProvider": true,
                    "documentSymbolProvider": true,
                    "referencesProvider": true,
                    "foldingRangeProvider": true,
                    "completionProvider": { "triggerCharacters": ["."] }
                }
            }
        })),
        "initialized" | "textDocument/didSave" | "workspace/didChangeConfiguration" => {}
        "textDocument/didOpen" => {
            let uri = uri(&params);
            let text = params["textDocument"]["text"]
                .as_str()
                .unwrap_or("")
                .to_string();
            docs.insert(uri.clone(), text.clone());
            publish(&uri, &text);
        }
        "textDocument/didChange" => {
            let uri = uri(&params);
            // full sync: the last change carries the whole document text
            let text = params["contentChanges"]
                .as_array()
                .and_then(|a| a.last())
                .and_then(|c| c.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            docs.insert(uri.clone(), text.clone());
            publish(&uri, &text);
        }
        "textDocument/hover" => {
            let (uri, line, col) = pos(&params, docs);
            let result = match lsp::hover(&uri_text(&uri, docs), &uri, line, col) {
                Some(s) => json!({ "contents": { "language": "xlang", "value": s } }),
                None => Value::Null,
            };
            send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        }
        "textDocument/definition" => {
            let (uri, line, col) = pos(&params, docs);
            let result = match lsp::definition(&uri_text(&uri, docs), &uri, line, col) {
                Some(r) => json!({
                    "uri": uri,
                    "range": lsp_range(&r)
                }),
                None => Value::Null,
            };
            send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        }
        "textDocument/completion" => {
            let uri = uri(&params);
            let names = lsp::completion_names(&uri_text(&uri, docs), &uri);
            let items: Vec<Value> = names.iter().map(|n| json!({ "label": n })).collect();
            send(&json!({ "jsonrpc": "2.0", "id": id, "result": items }));
        }
        "textDocument/documentSymbol" => {
            let uri = uri(&params);
            let entries = lsp::document_symbols(&uri_text(&uri, docs), &uri);
            let items: Vec<Value> = entries
                .iter()
                .map(|e| {
                    json!({
                        "name": e.name,
                        "kind": e.kind,
                        "range": lsp_range(&e.range),
                        "selectionRange": lsp_range(&e.range),
                    })
                })
                .collect();
            send(&json!({ "jsonrpc": "2.0", "id": id, "result": items }));
        }
        "textDocument/foldingRange" => {
            let uri = uri(&params);
            let ranges = lsp::folding_ranges(&uri_text(&uri, docs), &uri);
            let items: Vec<Value> = ranges
                .iter()
                .map(|(start, end)| json!({ "startLine": start, "endLine": end }))
                .collect();
            send(&json!({ "jsonrpc": "2.0", "id": id, "result": items }));
        }
        "textDocument/references" => {
            let (uri, line, col) = pos(&params, docs);
            let refs = lsp::references(&uri_text(&uri, docs), &uri, line, col);
            let items: Vec<Value> = refs
                .iter()
                .map(|r| json!({ "uri": uri, "range": lsp_range(r) }))
                .collect();
            send(&json!({ "jsonrpc": "2.0", "id": id, "result": items }));
        }
        "shutdown" => send(&json!({ "jsonrpc": "2.0", "id": id, "result": Value::Null })),
        "exit" => return false,
        _ => {}
    }
    true
}

fn uri(params: &Value) -> String {
    params["textDocument"]["uri"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

/// `(uri, 1-based line, 1-based col)` for hover/definition. LSP positions are
/// 0-based, so add 1 for the symbol-index lookup.
fn pos(params: &Value, docs: &HashMap<String, String>) -> (String, u32, u32) {
    let uri = uri(params);
    let line = params["position"]["line"].as_u64().unwrap_or(0) as u32 + 1;
    let col = params["position"]["character"].as_u64().unwrap_or(0) as u32 + 1;
    let _ = docs;
    (uri, line, col)
}

fn uri_text(uri: &str, docs: &HashMap<String, String>) -> String {
    docs.get(uri).cloned().unwrap_or_default()
}

/// Publish diagnostics (converting 1-based → 0-based for LSP).
fn publish(uri: &str, text: &str) {
    let diags = lsp::diagnostics(text, uri);
    let arr: Vec<Value> = diags
        .iter()
        .map(|d| {
            json!({
                "range": lsp_range_obj(
                    (d.range.line.saturating_sub(1)) as u32,
                    (d.range.col.saturating_sub(1)) as u32,
                    (d.range.end_line.saturating_sub(1)) as u32,
                    (d.range.end_col.saturating_sub(1)) as u32,
                ),
                "severity": 1, // Error
                "message": d.message,
                "source": "xlang"
            })
        })
        .collect();
    send(&json!({
        "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
        "params": { "uri": uri, "diagnostics": arr }
    }));
}

/// LSP range object from 1-based symbol-index Range.
fn lsp_range(r: &xlang::symbols::Range) -> Value {
    lsp_range_obj(
        r.line.saturating_sub(1),
        r.col.saturating_sub(1),
        r.end_line.saturating_sub(1),
        r.end_col.saturating_sub(1),
    )
}

fn lsp_range_obj(line: u32, col: u32, end_line: u32, end_col: u32) -> Value {
    json!({
        "start": { "line": line, "character": col },
        "end": { "line": end_line, "character": end_col }
    })
}
