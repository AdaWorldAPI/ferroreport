//! `ferroreport` command-line interface.

use ferroreport::model::{Band, ElementKind, Report};
use ferroreport::Renderer;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

mod serve;
mod spool;

const USAGE: &str = "\
ferroreport — report engine: PDF, Excel, Word, CSV and HTML from JasperReports JRXML templates
or .ferro templates, no JVM required

USAGE:
    ferroreport render <template.jrxml|template.ferro> [OPTIONS]
    ferroreport doctor <template.jrxml|template.ferro>
    ferroreport serve [OPTIONS]
    ferroreport convert <template> [-o <output>] [--to ferro|jrxml]

RENDER OPTIONS:
    -d, --data <file.json>      Rows: a JSON array of objects, or one JSON object per line
                                (omit for reports without data; - reads stdin). Rows are read
                                as they are needed, so the file can be larger than memory
    -p, --params <file.json>    JSON object with parameter values
    -o, --output <file>         Output file (default: <template>.pdf; - writes stdout). The
                                extension picks the format: .pdf, .xlsx, .docx, .csv or .html
    -f, --format <pdf|xlsx|docx|csv|html>
                                Output format, when the file name does not say it. html is a
                                web page that looks like the PDF, for previews
        --xlsx-numbers          In XLSX, write numbers as numbers (default: as formatted text,
                                like JasperReports)
        --csv-safe              In CSV, put an apostrophe before values a spreadsheet would
                                run as a formula (use it when the data comes from other people)
        --tagged                In PDF, say what the content is (tagged PDF) for screen readers:
                                texts as paragraphs, images as figures, the rest as decoration
        --max-pages <n>         Stop if the report goes over this many pages
        --timeout <seconds>     Stop if the render takes longer than this
    -l, --locale <tag>          Locale for numbers and dates (default: en_US)
        --font <Family=file.ttf[:bold][:italic]>
                                Register an extra font (repeatable)
    -r, --resources <dir>       Extra directory for images and other resources (repeatable;
                                the template's own directory is always searched)
        --now <YYYY-MM-DDTHH:MM:SS>
                                Fixed value for `new Date()`
    -q, --quiet                 Do not print timing information

DOCTOR:
    Lists the features of a template and whether ferroreport can render them.

CONVERT:
    Rewrites a template: JRXML (JasperReports 6 or 7) as a .ferro template, ferroreport's own
    format, or a .ferro template or a JRXML 6 one as JRXML 7. The output's extension (or --to)
    picks the format; without either, a JRXML becomes .ferro and a .ferro becomes JRXML.
    Templates are rendered the same in either format.

SERVE OPTIONS (HTTP API for any language: POST /render, POST /render/<name>, POST /inspect;
add format xlsx, docx, csv or html to the request for Excel, Word, CSV or a web preview):
        --host <addr>           Address to listen on (default: 127.0.0.1; 0.0.0.0 in Docker)
    -P, --port <n>              Port (default: 8080)
    -t, --templates <dir>       Directory of templates for POST /render/<name>
    -r, --resources <dir>       Extra directory for images and subreports (repeatable)
        --token <secret>        Require `Authorization: Bearer <secret>` (or FERROREPORT_TOKEN).
                                Needed to listen beyond localhost, unless:
        --allow-anonymous       Listen beyond localhost without a token (or
                                FERROREPORT_ALLOW_ANONYMOUS=1): only for ports your own
                                services alone can reach
        --rate-limit <n>        Most requests per minute from one client address (answers 429)
        --spool-memory <MB>     How much of one document is kept in memory until it is complete
                                (default: 32; the rest goes to a file)
        --spool-dir <dir>       Where those files go (default: the system's temporary directory;
                                or FERROREPORT_SPOOL_DIR)
        --spool-disk <MB>       Disk all requests together may use for them (default: 4096)
    Large reports: POST with Content-Type application/x-ndjson, the request on the first line
    and one row per line after it. Add `Prefer: respond-streaming` to get the document as it
    renders instead of once it is complete.
        --max-body <MB>         Largest request accepted (default: 50)
        --workers <n>           Requests rendered in parallel (default: CPU count)
        --timeout <seconds>     Longest a render may take (default: 60; 0 for no limit)
        --max-pages <n>         Most pages a report may have (default: no limit)
        --cache <n>             Parsed templates kept in memory (default: 128; 0 for none)
        --access-log            Print a line per request
    GET /health answers without a token; GET /metrics gives counters for Prometheus.
";

fn main() -> ExitCode {
    // Work on a thread with a large stack: templates are processed recursively, and the main
    // thread's stack is small on some systems (1 MB on Windows).
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)
        .ok()
        .and_then(|t| t.join().ok())
        .unwrap_or(ExitCode::FAILURE)
}

fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("render") => render(&args[1..]),
        Some("doctor") => doctor(&args[1..]),
        Some("serve") => serve(&args[1..]),
        Some("convert") => convert(&args[1..]),
        Some("-V") | Some("--version") => {
            println!("ferroreport {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

type CliResult = Result<(), Box<dyn std::error::Error>>;

fn read_json(path: &str) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let text = if path == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).map_err(|e| format!("stdin: {e}"))?;
        s
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?
    };
    Ok(serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?)
}

/// A temporary file that goes away with the value.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn convert(args: &[String]) -> CliResult {
    let mut input = None;
    let mut output = None;
    let mut to = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => output = it.next().cloned(),
            "-t" | "--to" => to = it.next().cloned(),
            s if s.starts_with('-') && s != "-" => return Err(format!("unknown option {s}").into()),
            s => input = Some(s.to_string()),
        }
    }
    let input = input.ok_or("convert needs a template: ferroreport convert <template> [-o <output>] [--to ferro|jrxml]")?;
    let source = if input == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).map_err(|e| format!("stdin: {e}"))?;
        s
    } else {
        std::fs::read_to_string(&input).map_err(|e| format!("{input}: {e}"))?
    };
    let from_template = ferroreport::template::is_template(&source);
    // What to write: --to, else the output's extension, else the other format.
    let extension =
        output.as_deref().and_then(|o| Path::new(o).extension()).and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let to = match (to.as_deref(), extension.as_deref()) {
        (Some("ferro"), _) | (None, Some("ferro")) => "ferro",
        (Some("jrxml" | "jrtx"), _) | (None, Some("jrxml" | "jrtx" | "xml")) => "jrxml",
        (Some(other), _) => return Err(format!("--to {other}: say ferro or jrxml").into()),
        (None, _) if from_template => "jrxml",
        (None, _) => "ferro",
    };
    let converted = match (from_template, to) {
        (true, "jrxml") => ferroreport::template::to_jrxml(&source)?,
        (true, _) => ferroreport::template::from_jrxml(&ferroreport::template::to_jrxml(&source)?)?,
        (false, "jrxml") => ferroreport::convert::jrxml_to_v7(&source)?,
        (false, _) => ferroreport::template::from_jrxml(&source)?,
    };
    match output.as_deref() {
        None | Some("-") => print!("{converted}"),
        Some(path) => std::fs::write(path, converted).map_err(|e| format!("{path}: {e}"))?,
    }
    Ok(())
}

fn render(args: &[String]) -> CliResult {
    let mut template = None;
    let mut data = None;
    let mut params = None;
    let mut output = None;
    let mut locale = "en_US".to_string();
    let mut fonts = Vec::new();
    let mut now = None;
    let mut quiet = false;
    let mut resources = Vec::new();
    let mut format = None;
    let mut xlsx_numbers = false;
    let mut csv_safe = false;
    let mut tagged = false;
    let mut limits = ferroreport::Limits::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "-d" | "--data" => data = Some(value(a)?),
            "-f" | "--format" => format = Some(value(a)?.to_ascii_lowercase()),
            "--xlsx-numbers" => xlsx_numbers = true,
            "--csv-safe" => csv_safe = true,
            "--tagged" => tagged = true,
            "--max-pages" => limits.max_pages = Some(value(a)?.parse().map_err(|_| "--max-pages expects a number")?),
            "--timeout" => {
                let secs: f64 = value(a)?.parse().map_err(|_| "--timeout expects seconds")?;
                limits.timeout = Some(std::time::Duration::from_secs_f64(secs.max(0.0)));
            }
            "-p" | "--params" => params = Some(value(a)?),
            "-o" | "--output" => output = Some(value(a)?),
            "-l" | "--locale" => locale = value(a)?,
            "--font" => fonts.push(value(a)?),
            "--now" => now = Some(value(a)?),
            "-q" | "--quiet" => quiet = true,
            "-r" | "--resources" => resources.push(value(a)?),
            s if s.starts_with('-') => return Err(format!("unknown option {s}").into()),
            s => template = Some(s.to_string()),
        }
    }
    let template = template.ok_or("missing template path")?;
    let format = match format {
        Some(f) => ferroreport::Format::parse(&f).ok_or_else(|| format!("unknown format {f} (pdf, xlsx, docx, csv or html)"))?,
        None => output
            .as_deref()
            .and_then(|o| PathBuf::from(o).extension().and_then(|e| ferroreport::Format::parse(&e.to_string_lossy())))
            .unwrap_or_default(),
    };
    let output = output.map(PathBuf::from).unwrap_or_else(|| PathBuf::from(&template).with_extension(format.extension()));

    let started = Instant::now();
    let source = std::fs::read_to_string(&template).map_err(|e| format!("{template}: {e}"))?;
    let params_json = match &params {
        Some(p) => read_json(p)?,
        None => serde_json::Value::Object(Default::default()),
    };
    let template_dir = PathBuf::from(&template).parent().map(PathBuf::from).unwrap_or_default();
    let template_dir = if template_dir.as_os_str().is_empty() { PathBuf::from(".") } else { template_dir };
    let mut renderer = Renderer::new()
        .locale(&locale)
        .resource_dir(template_dir)
        .limits(limits)
        .xlsx_options(ferroreport::xlsx::XlsxOptions { detect_cell_type: xlsx_numbers })
        .tagged(tagged)
        .csv_options(ferroreport::csv::CsvOptions { guard_formulas: csv_safe, ..Default::default() });
    for dir in &resources {
        renderer = renderer.resource_dir(dir);
    }
    // Parsed here, where the resource folders are known: style templates are read from them.
    let report = renderer.parse(&source)?;
    if let Some(n) = now {
        let dt = ferroreport::data::parse_datetime(&n).ok_or("--now expects YYYY-MM-DDTHH:MM:SS")?;
        renderer = renderer.now(dt);
    }
    for spec in &fonts {
        let (family, rest) = spec.split_once('=').ok_or("--font expects Family=file.ttf")?;
        let mut parts = rest.split(':');
        let path = parts.next().unwrap_or_default();
        let flags: Vec<&str> = parts.collect();
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        renderer.register_font(family, flags.contains(&"bold"), flags.contains(&"italic"), bytes)?;
    }
    let params = ferroreport::params_from_json(&report, &params_json)?;
    let parsed = started.elapsed();
    // Rows are read as they are needed and pages are written as they are finished, so the
    // size of the data does not decide how much memory the render takes.
    // A large workbook reads its rows twice (see `Renderer::render_rows_to`), so rows from
    // standard input are put in a file first for that format.
    let mut kept = None;
    let data = match data.as_deref() {
        Some("-") if format == ferroreport::Format::Xlsx => {
            let path = std::env::temp_dir().join(format!("ferroreport-rows-{}.json", std::process::id()));
            let keep = RemoveOnDrop(path.clone());
            let mut file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            std::io::copy(&mut std::io::stdin().lock(), &mut file).map_err(|e| format!("stdin: {e}"))?;
            kept = Some(keep);
            Some(path.to_string_lossy().into_owned())
        }
        other => other.map(str::to_string),
    };
    let mut rows = || -> ferroreport::Result<ferroreport::RowIter> {
        Ok(match data.as_deref() {
            Some("-") => Box::new(ferroreport::JsonRows::new(&report, std::io::stdin().lock())),
            Some(path) => {
                let file = std::fs::File::open(path).map_err(|e| ferroreport::Error::Io(format!("{path}: {e}")))?;
                Box::new(ferroreport::JsonRows::new(&report, file))
            }
            None => Box::new(std::iter::empty()),
        })
    };
    let to_stdout = output.as_os_str() == "-";
    let stats = if to_stdout {
        quiet = true;
        renderer.render_rows_to(&report, &mut rows, params, format, std::io::BufWriter::new(std::io::stdout().lock()))?
    } else {
        // Written beside the output and moved into place, so a failed render leaves no
        // half-written file behind.
        let mut name = output.file_name().unwrap_or_default().to_os_string();
        name.push(".part");
        let part = output.with_file_name(name);
        let file = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", output.display()))?;
        match renderer.render_rows_to(&report, &mut rows, params, format, std::io::BufWriter::with_capacity(1 << 16, file)) {
            Ok(stats) => {
                std::fs::rename(&part, &output).map_err(|e| format!("{}: {e}", output.display()))?;
                stats
            }
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                return Err(e.into());
            }
        }
    };
    let total = started.elapsed();
    drop(kept);

    for w in &report.warnings {
        eprintln!("warning: {w}");
    }
    for (from, to) in renderer.font_substitutions() {
        eprintln!("warning: font `{from}` is not available, using `{to}` (register it with --font)");
    }
    if !quiet {
        eprintln!(
            "{} -> {} ({} pages, {} rows) parse {:.1} ms, render {:.1} ms",
            template,
            output.display(),
            stats.pages,
            stats.rows,
            parsed.as_secs_f64() * 1e3,
            (total - parsed).as_secs_f64() * 1e3,
        );
    }
    Ok(())
}

fn serve(args: &[String]) -> CliResult {
    let mut host = std::env::var("FERROREPORT_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let mut port = std::env::var("FERROREPORT_PORT").unwrap_or_else(|_| "8080".into());
    let mut templates = std::env::var("FERROREPORT_TEMPLATES").ok().map(PathBuf::from);
    let mut resources = Vec::new();
    let mut token = std::env::var("FERROREPORT_TOKEN").ok().filter(|t| !t.is_empty());
    let mut max_body_mb: usize = 50;
    let mut workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    // A request that runs for a minute is almost always a mistake or an attack.
    let mut limits = ferroreport::Limits { max_pages: None, timeout: Some(std::time::Duration::from_secs(60)) };
    let mut cache = 128usize;
    let mut access_log = false;
    let mut allow_anonymous = std::env::var("FERROREPORT_ALLOW_ANONYMOUS").is_ok_and(|v| !v.is_empty() && v != "0");
    let mut rate_limit = None;
    let megabytes = |name: &str, default: u64| std::env::var(name).ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(default);
    let mut spool_memory = megabytes("FERROREPORT_SPOOL_MEMORY", 32);
    let mut spool_disk = megabytes("FERROREPORT_SPOOL_DISK", 4096);
    let mut spool_dir = std::env::var("FERROREPORT_SPOOL_DIR").ok().filter(|d| !d.is_empty()).map(PathBuf::from);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--host" => host = value(a)?,
            "-P" | "--port" => port = value(a)?,
            "-t" | "--templates" => templates = Some(PathBuf::from(value(a)?)),
            "-r" | "--resources" => resources.push(PathBuf::from(value(a)?)),
            "--token" => token = Some(value(a)?),
            "--max-body" => max_body_mb = value(a)?.parse().map_err(|_| "--max-body expects megabytes")?,
            "--workers" => workers = value(a)?.parse().map_err(|_| "--workers expects a number")?,
            "--max-pages" => limits.max_pages = Some(value(a)?.parse().map_err(|_| "--max-pages expects a number")?),
            "--timeout" => {
                let secs: f64 = value(a)?.parse().map_err(|_| "--timeout expects seconds (0 for none)")?;
                limits.timeout = (secs > 0.0).then(|| std::time::Duration::from_secs_f64(secs));
            }
            "--cache" => cache = value(a)?.parse().map_err(|_| "--cache expects a number of templates")?,
            "--access-log" => access_log = true,
            "--allow-anonymous" => allow_anonymous = true,
            "--spool-dir" => spool_dir = Some(PathBuf::from(value(a)?)),
            "--spool-memory" => spool_memory = value(a)?.parse().map_err(|_| "--spool-memory expects megabytes")?,
            "--spool-disk" => spool_disk = value(a)?.parse().map_err(|_| "--spool-disk expects megabytes")?,
            "--rate-limit" => rate_limit = Some(value(a)?.parse().map_err(|_| "--rate-limit expects requests per minute")?),
            s => return Err(format!("unknown option {s}").into()),
        }
    }
    serve::run(serve::Config {
        addr: format!("{host}:{port}"),
        templates,
        resources,
        token,
        max_body: max_body_mb * 1024 * 1024,
        workers,
        limits,
        cache,
        access_log,
        allow_anonymous,
        rate_limit,
        spool_memory: (spool_memory as usize).saturating_mul(1024 * 1024),
        spool_dir,
        spool_disk: spool_disk.saturating_mul(1024 * 1024),
    })
}

fn count_elements(band: &Band, counts: &mut std::collections::BTreeMap<String, usize>) {
    fn walk(els: &[ferroreport::model::Element], counts: &mut std::collections::BTreeMap<String, usize>) {
        for e in els {
            let name = match &e.kind {
                ElementKind::Text(t) => match t.content {
                    ferroreport::model::TextContent::Static(_) => "staticText".to_string(),
                    _ => "textField".to_string(),
                },
                ElementKind::Line { .. } => "line".into(),
                ElementKind::Rectangle { .. } => "rectangle".into(),
                ElementKind::Ellipse { .. } => "ellipse".into(),
                ElementKind::Frame { children, .. } => {
                    walk(children, counts);
                    "frame".into()
                }
                ElementKind::Break | ElementKind::ColumnBreak => "break".into(),
                ElementKind::Image(_) => "image".into(),
                ElementKind::Subreport(_) | ElementKind::Content { .. } => "subreport".into(),
                ElementKind::Table(_) => "table".into(),
                ElementKind::List(_) => "list".into(),
                ElementKind::Crosstab(_) | ElementKind::CrosstabRest(_) => "crosstab".into(),
                ElementKind::Chart(c) => format!("{:?} chart", c.kind),
                ElementKind::Barcode(b) => b.symbology.component().into(),
                ElementKind::Unsupported(n) => format!("{n} (unsupported)"),
            };
            *counts.entry(name).or_default() += 1;
        }
    }
    walk(&band.elements, counts);
}

fn doctor(args: &[String]) -> CliResult {
    let path = args.first().ok_or("missing template path")?;
    let source = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let dir = Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let report: Report = match Renderer::new().resource_dir(dir).parse(&source) {
        Ok(r) => r,
        Err(e) => {
            println!("✗ {path}: cannot be loaded\n  {e}");
            return Err("template not supported".into());
        }
    };
    println!("{path}: report `{}`", report.name);
    println!(
        "  page {}x{} pt, margins {}/{}/{}/{}, language {:?}",
        report.page_width,
        report.page_height,
        report.margins.top,
        report.margins.right,
        report.margins.bottom,
        report.margins.left,
        report.language
    );
    println!(
        "  {} parameters, {} fields, {} variables, {} groups",
        report.parameters.len(),
        report.fields.len(),
        report.variables.len(),
        report.groups.len()
    );
    let mut counts = std::collections::BTreeMap::new();
    let bands = [
        &report.background,
        &report.title,
        &report.page_header,
        &report.column_header,
        &report.column_footer,
        &report.page_footer,
        &report.last_page_footer,
        &report.summary,
        &report.no_data,
    ];
    for b in bands.into_iter().flatten() {
        count_elements(b, &mut counts);
    }
    for b in &report.detail {
        count_elements(b, &mut counts);
    }
    for g in &report.groups {
        for b in g.header.iter().chain(&g.footer) {
            count_elements(b, &mut counts);
        }
    }
    let list: Vec<String> = counts.iter().map(|(k, v)| format!("{k} ×{v}")).collect();
    println!("  elements: {}", if list.is_empty() { "none".into() } else { list.join(", ") });
    if report.warnings.is_empty() {
        println!("✓ every feature used by this template is supported");
    } else {
        println!("! {} issue(s):", report.warnings.len());
        for w in &report.warnings {
            println!("  - {w}");
        }
    }
    Ok(())
}
