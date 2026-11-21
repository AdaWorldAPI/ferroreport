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
//! GET  /metrics                                 -> counters in Prometheus text format
//! ```
//!
//! Parsed templates are kept in a small cache, so a template sent again (or read again from
//! `--templates`) is not parsed twice. Every render runs under the limits given at start-up.

use ferroreport::Renderer;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tiny_http::{Header, Method, Request, Response, Server};

pub struct Config {
    pub addr: String,
    pub templates: Option<PathBuf>,
    pub resources: Vec<PathBuf>,
    pub token: Option<String>,
    pub max_body: usize,
    pub workers: usize,
    /// Limits every render runs under.
    pub limits: ferroreport::Limits,
    /// Parsed templates kept (0 turns the cache off).
    pub cache: usize,
    /// Print a line per request to standard error.
    pub access_log: bool,
}

/// Parsed templates by their source text, the least recently used dropped first.
pub struct TemplateCache {
    capacity: usize,
    entries: Mutex<(u64, HashMap<u64, CacheEntry>)>,
}

struct CacheEntry {
    source: Arc<str>,
    report: Arc<ferroreport::Report>,
    used: u64,
}

impl TemplateCache {
    pub fn new(capacity: usize) -> TemplateCache {
        TemplateCache { capacity, entries: Mutex::new((0, HashMap::new())) }
    }

    /// The parsed form of `source`, and whether it came from the cache.
    fn get(&self, source: &str) -> ferroreport::Result<(Arc<ferroreport::Report>, bool)> {
        use std::hash::{Hash, Hasher};
        if self.capacity == 0 {
            return Ok((Arc::new(ferroreport::parse(source)?), false));
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut h);
        let key = h.finish();
        {
            let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            let (tick, map) = &mut *guard;
            *tick += 1;
            if let Some(e) = map.get_mut(&key) {
                // The text is compared too: two templates must never answer for each other.
                if &*e.source == source {
                    e.used = *tick;
                    return Ok((e.report.clone(), true));
                }
            }
        }
        // Parsed outside the lock, so a slow template does not hold the others up.
        let report = Arc::new(ferroreport::parse(source)?);
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let (tick, map) = &mut *guard;
        if map.len() >= self.capacity && !map.contains_key(&key) {
            if let Some(oldest) = map.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| *k) {
                map.remove(&oldest);
            }
        }
        map.insert(key, CacheEntry { source: source.into(), report: report.clone(), used: *tick });
        Ok((report, false))
    }
}

/// Counters for `GET /metrics`.
#[derive(Default)]
pub struct Metrics {
    renders_ok: AtomicU64,
    renders_failed: AtomicU64,
    limited: AtomicU64,
    unauthorized: AtomicU64,
    panics: AtomicU64,
    render_micros: AtomicU64,
    pages: AtomicU64,
    rows: AtomicU64,
    bytes_out: AtomicU64,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    in_flight: AtomicU64,
}

impl Metrics {
    fn text(&self) -> String {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let mut out = String::new();
        let mut line = |name: &str, kind: &str, help: &str, value: String| {
            out.push_str(&format!("# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {value}\n"));
        };
        line("ferroreport_renders_total", "counter", "Reports rendered.", g(&self.renders_ok).to_string());
        line(
            "ferroreport_render_errors_total",
            "counter",
            "Renders that failed (bad template or data).",
            g(&self.renders_failed).to_string(),
        );
        line(
            "ferroreport_render_limited_total",
            "counter",
            "Renders stopped by a page or time limit.",
            g(&self.limited).to_string(),
        );
        line(
            "ferroreport_unauthorized_total",
            "counter",
            "Requests refused for a missing or wrong token.",
            g(&self.unauthorized).to_string(),
        );
        line(
            "ferroreport_panics_total",
            "counter",
            "Requests answered 500 after an internal error.",
            g(&self.panics).to_string(),
        );
        line(
            "ferroreport_render_seconds_total",
            "counter",
            "Time spent rendering.",
            format!("{:.6}", g(&self.render_micros) as f64 / 1e6),
        );
        line("ferroreport_pages_total", "counter", "Pages rendered.", g(&self.pages).to_string());
        line("ferroreport_rows_total", "counter", "Data rows read.", g(&self.rows).to_string());
        line("ferroreport_response_bytes_total", "counter", "Bytes of rendered documents sent.", g(&self.bytes_out).to_string());
        line(
            "ferroreport_template_cache_hits_total",
            "counter",
            "Templates found already parsed.",
            g(&self.cache_hits).to_string(),
        );
        line(
            "ferroreport_template_cache_misses_total",
            "counter",
            "Templates that had to be parsed.",
            g(&self.cache_misses).to_string(),
        );
        line("ferroreport_renders_in_flight", "gauge", "Renders running now.", g(&self.in_flight).to_string());
        out
    }
}

/// What the workers share.
pub struct State {
    pub cfg: Config,
    pub cache: TemplateCache,
    pub metrics: Metrics,
}

/// A rendered document and what it took.
struct Rendered {
    bytes: Vec<u8>,
    pages: usize,
    rows: usize,
    parse_ms: f64,
    render_ms: f64,
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
    state: &State,
    body: &serde_json::Value,
    name: Option<&str>,
    format: ferroreport::Format,
) -> Result<Rendered, (u16, String)> {
    let cfg = &state.cfg;
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
    let mut r = Renderer::new().locale(locale).limits(cfg.limits);
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
    let failed = |e: ferroreport::Error| match e {
        ferroreport::Error::Limit(_) => (413, e.to_string()),
        _ => (422, e.to_string()),
    };
    let started = Instant::now();
    let (report, cached) = state.cache.get(&source).map_err(failed)?;
    let counter = if cached { &state.metrics.cache_hits } else { &state.metrics.cache_misses };
    counter.fetch_add(1, Ordering::Relaxed);
    let parse_ms = started.elapsed().as_secs_f64() * 1e3;
    let records = ferroreport::data::records_from_json(&report, rows).map_err(failed)?;
    let params = ferroreport::params_from_json(&report, params).map_err(failed)?;
    let mut bytes = Vec::new();
    let stats = r.render_to(&report, Box::new(records.into_iter().map(Ok)), params, format, &mut bytes).map_err(failed)?;
    let render_ms = started.elapsed().as_secs_f64() * 1e3 - parse_ms;
    Ok(Rendered { bytes, pages: stats.pages, rows: stats.rows, parse_ms, render_ms })
}

fn handle(state: &State, req: &mut Request) -> Reply {
    let cfg = &state.cfg;
    let url = req.url().split('?').next().unwrap_or("").to_string();
    // Load balancers ask for /health without credentials; it says nothing but "ok".
    if (req.method(), url.as_str()) == (&Method::Get, "/health") {
        return json(200, serde_json::json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }));
    }
    if let Some(token) = &cfg.token {
        let ok = req.headers().iter().any(|h| {
            h.field.equiv("Authorization")
                && h.value.as_str().strip_prefix("Bearer ").is_some_and(|t| same_secret(t.as_bytes(), token.as_bytes()))
        });
        if !ok {
            state.metrics.unauthorized.fetch_add(1, Ordering::Relaxed);
            return error(401, "missing or wrong bearer token");
        }
    }
    match (req.method(), url.as_str()) {
        (Method::Get, "/metrics") => Response::from_data(state.metrics.text().into_bytes())
            .with_header(Header::from_bytes("Content-Type", "text/plain; version=0.0.4").expect("static header")),
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
            let m = &state.metrics;
            m.in_flight.fetch_add(1, Ordering::Relaxed);
            let started = Instant::now();
            let result = render(state, &body, name, format);
            m.in_flight.fetch_sub(1, Ordering::Relaxed);
            m.render_micros.fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
            match result {
                Ok(done) => {
                    m.renders_ok.fetch_add(1, Ordering::Relaxed);
                    m.pages.fetch_add(done.pages as u64, Ordering::Relaxed);
                    m.rows.fetch_add(done.rows as u64, Ordering::Relaxed);
                    m.bytes_out.fetch_add(done.bytes.len() as u64, Ordering::Relaxed);
                    let timing = format!("parse;dur={:.2}, render;dur={:.2}", done.parse_ms, done.render_ms);
                    Response::from_data(done.bytes)
                        .with_header(Header::from_bytes("Content-Type", format.mime()).expect("static header"))
                        .with_header(Header::from_bytes("Server-Timing", timing).expect("ascii header"))
                        .with_header(Header::from_bytes("X-Report-Pages", done.pages.to_string()).expect("ascii header"))
                }
                Err((status, message)) => {
                    let counter = if status == 413 { &m.limited } else { &m.renders_failed };
                    counter.fetch_add(1, Ordering::Relaxed);
                    error(status, message)
                }
            }
        }
        _ => error(404, "not found: use POST /render, POST /render/<name>, POST /inspect, GET /health or GET /metrics"),
    }
}

pub fn run(cfg: Config) -> Result<(), Box<dyn std::error::Error>> {
    let server = Arc::new(Server::http(&cfg.addr).map_err(|e| format!("{}: {e}", cfg.addr))?);
    eprintln!("ferroreport {} listening on http://{}", env!("CARGO_PKG_VERSION"), cfg.addr);
    let local = ["127.", "localhost", "[::1]"].iter().any(|p| cfg.addr.starts_with(p));
    if cfg.token.is_none() && !local {
        eprintln!(
            "warning: listening on {} without a token: anyone who can reach this port can render reports. \
             Set --token (or FERROREPORT_TOKEN), or keep the port private.",
            cfg.addr
        );
    }
    let state = Arc::new(State { cache: TemplateCache::new(cfg.cache), metrics: Metrics::default(), cfg });
    let workers: Vec<_> = (0..state.cfg.workers.max(1))
        .map(|_| {
            let (server, state) = (server.clone(), state.clone());
            // Large stacks: templates are processed recursively (nesting is capped well below
            // what this holds).
            std::thread::Builder::new()
                .stack_size(32 * 1024 * 1024)
                .spawn(move || {
                    while let Ok(mut req) = server.recv() {
                        // A bug must not take the worker down with it: answer 500 and go on.
                        let started = Instant::now();
                        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(&state, &mut req)))
                            .unwrap_or_else(|_| {
                                state.metrics.panics.fetch_add(1, Ordering::Relaxed);
                                error(500, "internal error while rendering")
                            });
                        if state.cfg.access_log {
                            eprintln!(
                                "{} {} {} {:.1}ms",
                                req.method(),
                                req.url().split('?').next().unwrap_or(""),
                                reply.status_code().0,
                                started.elapsed().as_secs_f64() * 1e3
                            );
                        }
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
    fn the_template_cache_reuses_and_forgets() {
        let t = |name: &str| {
            format!(
                r#"<jasperReport name="{name}" pageWidth="100" pageHeight="100" columnWidth="100" leftMargin="0" rightMargin="0" topMargin="0" bottomMargin="0"/>"#
            )
        };
        let cache = TemplateCache::new(2);
        assert!(!cache.get(&t("a")).unwrap().1);
        assert!(cache.get(&t("a")).unwrap().1);
        assert!(!cache.get(&t("b")).unwrap().1);
        assert!(cache.get(&t("a")).unwrap().1);
        // A third template pushes out the one used longest ago ("b").
        assert!(!cache.get(&t("c")).unwrap().1);
        assert!(cache.get(&t("a")).unwrap().1);
        assert!(!cache.get(&t("b")).unwrap().1);
        assert_eq!(cache.get(&t("b")).unwrap().0.name, "b");
        assert!(cache.get("<nonsense").is_err());
        assert!(!TemplateCache::new(0).get(&t("a")).unwrap().1 && !TemplateCache::new(0).get(&t("a")).unwrap().1);
    }

    #[test]
    fn metrics_are_prometheus_text() {
        let m = Metrics::default();
        m.renders_ok.fetch_add(3, Ordering::Relaxed);
        m.render_micros.fetch_add(1_500_000, Ordering::Relaxed);
        let text = m.text();
        assert!(text.contains("# TYPE ferroreport_renders_total counter\nferroreport_renders_total 3\n"), "{text}");
        assert!(text.contains("ferroreport_render_seconds_total 1.500000\n"), "{text}");
    }

    #[test]
    fn template_names_stay_relative() {
        assert!(safe_name("sales/invoice").is_some());
        for bad in ["", "/etc/passwd", "../x", "a/../../b"] {
            assert!(safe_name(bad).is_none(), "{bad}");
        }
    }
}
