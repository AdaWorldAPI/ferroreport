//! `ferroreport serve`: an HTTP endpoint so any language can render reports.
//!
//! ```text
//! POST /render             {"template": "<jasperReport ...>" | "templateName": "invoice.jrxml",
//!                           "data": [...], "params": {...}, "locale": "es_DO",
//!                           "resources": {"logo.png": "<base64>"}}      -> application/pdf
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

/// A template name from a URL: a plain relative path inside the templates directory.
fn safe_name(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if name.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(p.to_path_buf())
}

fn render(cfg: &Config, body: &serde_json::Value, name: Option<&str>) -> Result<Vec<u8>, (u16, String)> {
    let bad = |m: String| (400, m);
    let (source, dir) = match (name, body.get("templateName").and_then(|v| v.as_str()), body.get("template")) {
        (Some(n), _, _) | (None, Some(n), _) => {
            let root = cfg.templates.as_ref().ok_or((404, "no --templates directory configured".to_string()))?;
            let mut rel = safe_name(n).ok_or((400, format!("invalid template name `{n}`")))?;
            if rel.extension().is_none() {
                rel.set_extension("jrxml");
            }
            let path = root.join(&rel);
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
    r.render_json(&source, rows, params).map_err(|e| (422, e.to_string()))
}

fn handle(cfg: &Config, req: &mut Request) -> Reply {
    if let Some(token) = &cfg.token {
        let ok = req
            .headers()
            .iter()
            .any(|h| h.field.equiv("Authorization") && h.value.as_str().strip_prefix("Bearer ").is_some_and(|t| t == token));
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
            match render(cfg, &body, name) {
                Ok(pdf) => Response::from_data(pdf)
                    .with_header(Header::from_bytes("Content-Type", "application/pdf").expect("static header")),
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
            std::thread::spawn(move || {
                while let Ok(mut req) = server.recv() {
                    let reply = handle(&cfg, &mut req);
                    let _ = req.respond(reply);
                }
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    Ok(())
}
