//! `ferroreport serve`: an HTTP endpoint so any language can render reports.
//!
//! ```text
//! POST /render             {"template": "<jasperReport ...>" | "templateName": "invoice.jrxml",
//!                           "data": [...], "params": {...}, "locale": "es_DO",
//!                           "resources": {"logo.png": "<base64>"},
//!                           "format": "pdf" | "xlsx" | "csv"}           -> the file
//! POST /render/<name>      {"data": [...], "params": {...}}  (template from --templates)
//! POST /inspect            {"template": "..."}                          -> JSON
//! GET  /health                                                          -> JSON
//! ```

use ferroreport::Renderer;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tiny_http::{Header, Method, Request, Response, Server};

pub struct Config {
    pub addr: String,
    pub templates: Option<PathBuf>,
    pub resources: Vec<PathBuf>,
    pub token: Option<String>,
    pub max_body: usize,
    pub workers: usize,
}

type Reply = Response<std::io::Cursor<Vec<u8>>>;

fn json(status: u16, value: serde_json::Value) -> Reply {
    Response::from_data(value.to_string().into_bytes())
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "application/json").expect("static header"))
}

fn error(status: u16, message: impl std::fmt::Display) -> Reply {
    json(status, serde_json::json!({ "error": message.to_string() }))
}

/// Compares a secret in time that does not depend on where the inputs differ.
fn same_secret(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as usize;
    }
    diff == 0
}

/// A template name from a URL: a plain relative path inside the templates directory.
fn safe_name(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if name.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(p.to_path_buf())
}

fn render(
    cfg: &Config,
    body: &serde_json::Value,
    name: Option<&str>,
    format: ferroreport::Format,
) -> Result<Vec<u8>, (u16, String)> {
    let bad = |m: String| (400, m);
    let (source, dir) = match (name, body.get("templateName").and_then(|v| v.as_str()), body.get("template")) {
        (Some(n), _, _) | (None, Some(n), _) => {
            let root = cfg.templates.as_ref().ok_or((404, "no --templates directory configured".to_string()))?;
            let mut rel = safe_name(n).ok_or((400, format!("invalid template name `{n}`")))?;
            if rel.extension().is_none() {
                rel.set_extension("jrxml");
            }
            let path = root.join(&rel);
            // A symbolic link in the templates directory may not point outside it.
            let inside = match (path.canonicalize(), root.canonicalize()) {
                (Ok(real), Ok(real_root)) => real.starts_with(real_root),
                _ => false,
            };
            if !inside {
                return Err((404, format!("template `{n}` not found")));
            }
            let text = std::fs::read_to_string(&path).map_err(|_| (404, format!("template `{n}` not found")))?;
            (text, path.parent().map(Path::to_path_buf))
        }
        (None, None, Some(serde_json::Value::String(t))) => (t.clone(), None),
        _ => return Err(bad("send `template` (JRXML source) or `templateName`".into())),
    };
    let locale = body.get("locale").and_then(|v| v.as_str()).unwrap_or("en_US");
    let mut r = Renderer::new().locale(locale);
    if let Some(d) = dir {
        r = r.resource_dir(d);
    }
    for d in &cfg.resources {
        r = r.resource_dir(d);
    }
    if let Some(now) = body.get("now").and_then(|v| v.as_str()) {
        r = r.now(ferroreport::data::parse_datetime(now).ok_or_else(|| bad(format!("cannot parse now `{now}`")))?);
    }
    if let Some(res) = body.get("resources").and_then(|v| v.as_object()) {
        for (k, v) in res {
            let text = v.as_str().ok_or_else(|| bad(format!("resource `{k}` must be a base64 string")))?;
            let bytes = ferroreport::image::decode_base64(text).map_err(|e| bad(format!("resource `{k}`: {e}")))?;
            r.add_resource(k, bytes);
        }
    }
    let empty_rows = serde_json::Value::Array(Vec::new());
    let empty_params = serde_json::Value::Object(Default::default());
    let rows = body.get("data").unwrap_or(&empty_rows);
    let params = body.get("params").unwrap_or(&empty_params);
    r.render_json_as(&source, rows, params, format).map_err(|e| (422, e.to_string()))
}

fn handle(cfg: &Config, req: &mut Request) -> Reply {
    if let Some(token) = &cfg.token {
        let ok = req.headers().iter().any(|h| {
            h.field.equiv("Authorization")
                && h.value.as_str().strip_prefix("Bearer ").is_some_and(|t| same_secret(t.as_bytes(), token.as_bytes()))
        });
        if !ok {
            return error(401, "missing or wrong bearer token");
        }
    }
    let url = req.url().split('?').next().unwrap_or("").to_string();
    match (req.method(), url.as_str()) {
        (Method::Get, "/health") => json(200, serde_json::json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") })),
        (Method::Post, path) if path == "/render" || path.starts_with("/render/") || path == "/inspect" => {
            if req.body_length().is_some_and(|n| n > cfg.max_body) {
                return error(413, "request body too large");
            }
            let mut buf = Vec::new();
            if req.as_reader().take(cfg.max_body as u64 + 1).read_to_end(&mut buf).is_err() {
                return error(400, "cannot read request body");
            }
            if buf.len() > cfg.max_body {
                return error(413, "request body too large");
            }
            let body: serde_json::Value = match serde_json::from_slice(&buf) {
                Ok(v) => v,
                Err(e) => return error(400, format!("invalid JSON: {e}")),
            };
            if path == "/inspect" {
                let Some(t) = body.get("template").and_then(|v| v.as_str()) else {
                    return error(400, "send `template`");
                };
                return match ferroreport::parse(t) {
                    Ok(r) => json(
                        200,
                        serde_json::json!({
                            "name": r.name,
                            "parameters": r.parameters.iter().map(|p| &p.name).collect::<Vec<_>>(),
                            "fields": r.fields.iter().map(|f| &f.name).collect::<Vec<_>>(),
                            "warnings": r.warnings,
                        }),
                    ),
                    Err(e) => error(422, e),
                };
            }
            let name = path.strip_prefix("/render/").map(|n| n.trim_matches('/'));
            // `format` in the body, or `?format=` in the URL; PDF by default.
            let query = req.url().split_once('?').map(|(_, q)| q.to_string()).unwrap_or_default();
            let requested = body
                .get("format")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| query.split('&').find_map(|kv| kv.strip_prefix("format=").map(str::to_string)));
            let format = match requested {
                None => ferroreport::Format::Pdf,
                Some(f) => match ferroreport::Format::parse(&f) {
                    Some(f) => f,
                    None => return error(400, format!("unknown format `{f}` (pdf, xlsx or csv)")),
                },
            };
            match render(cfg, &body, name, format) {
                Ok(bytes) => Response::from_data(bytes)
                    .with_header(Header::from_bytes("Content-Type", format.mime()).expect("static header")),
                Err((status, message)) => error(status, message),
            }
        }
        _ => error(404, "not found: use POST /render, POST /render/<name>, POST /inspect or GET /health"),
    }
}

pub fn run(cfg: Config) -> Result<(), Box<dyn std::error::Error>> {
    let server = Arc::new(Server::http(&cfg.addr).map_err(|e| format!("{}: {e}", cfg.addr))?);
    eprintln!("ferroreport {} listening on http://{}", env!("CARGO_PKG_VERSION"), cfg.addr);
    let cfg = Arc::new(cfg);
    let workers: Vec<_> = (0..cfg.workers.max(1))
        .map(|_| {
            let (server, cfg) = (server.clone(), cfg.clone());
            // Large stacks: templates are processed recursively (nesting is capped well below
            // what this holds).
            std::thread::Builder::new()
                .stack_size(32 * 1024 * 1024)
                .spawn(move || {
                    while let Ok(mut req) = server.recv() {
                        // A bug must not take the worker down with it: answer 500 and go on.
                        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(&cfg, &mut req)))
                            .unwrap_or_else(|_| error(500, "internal error while rendering"));
                        let _ = req.respond(reply);
                    }
                })
                .expect("spawn worker")
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_compare_exactly() {
        assert!(same_secret(b"abc", b"abc"));
        assert!(!same_secret(b"abc", b"abd"));
        assert!(!same_secret(b"abc", b"abcd"));
        assert!(!same_secret(b"", b"a"));
    }

    #[test]
    fn template_names_stay_relative() {
        assert!(safe_name("sales/invoice").is_some());
        for bad in ["", "/etc/passwd", "../x", "a/../../b"] {
            assert!(safe_name(bad).is_none(), "{bad}");
        }
    }
}
