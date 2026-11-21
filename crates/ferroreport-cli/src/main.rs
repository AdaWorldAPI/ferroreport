//! `ferroreport` command-line interface.

use ferroreport::model::{Band, ElementKind, Report};
use ferroreport::Renderer;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "\
ferroreport — render JasperReports JRXML templates to PDF, no JVM required

USAGE:
    ferroreport render <template.jrxml> [OPTIONS]
    ferroreport doctor <template.jrxml>

RENDER OPTIONS:
    -d, --data <file.json>      JSON array of rows (omit for reports without data)
    -p, --params <file.json>    JSON object with parameter values
    -o, --output <file.pdf>     Output file (default: <template>.pdf)
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
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("render") => render(&args[1..]),
        Some("doctor") => doctor(&args[1..]),
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
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?)
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
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "-d" | "--data" => data = Some(value(a)?),
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
    let output = output.map(PathBuf::from).unwrap_or_else(|| PathBuf::from(&template).with_extension("pdf"));

    let started = Instant::now();
    let source = std::fs::read_to_string(&template).map_err(|e| format!("{template}: {e}"))?;
    let report = ferroreport::parse(&source)?;
    let rows = match &data {
        Some(p) => read_json(p)?,
        None => serde_json::Value::Array(Vec::new()),
    };
    let params_json = match &params {
        Some(p) => read_json(p)?,
        None => serde_json::Value::Object(Default::default()),
    };
    let template_dir = PathBuf::from(&template).parent().map(PathBuf::from).unwrap_or_default();
    let template_dir = if template_dir.as_os_str().is_empty() { PathBuf::from(".") } else { template_dir };
    let mut renderer = Renderer::new().locale(&locale).resource_dir(template_dir);
    for dir in &resources {
        renderer = renderer.resource_dir(dir);
    }
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
    let records = ferroreport::data::records_from_json(&report, &rows)?;
    let params = ferroreport::params_from_json(&report, &params_json)?;
    let parsed = started.elapsed();
    let doc = renderer.fill(&report, &records, params)?;
    let filled = started.elapsed();
    let pdf = renderer.to_pdf(&doc)?;
    std::fs::write(&output, &pdf).map_err(|e| format!("{}: {e}", output.display()))?;
    let total = started.elapsed();

    for w in &report.warnings {
        eprintln!("warning: {w}");
    }
    for (from, to) in renderer.font_substitutions() {
        eprintln!("warning: font `{from}` is not available, using `{to}` (register it with --font)");
    }
    if !quiet {
        eprintln!(
            "{} -> {} ({} pages, {} rows) parse {:.1} ms, fill {:.1} ms, export {:.1} ms",
            template,
            output.display(),
            doc.pages.len(),
            records.len(),
            parsed.as_secs_f64() * 1e3,
            (filled - parsed).as_secs_f64() * 1e3,
            (total - filled).as_secs_f64() * 1e3,
        );
    }
    Ok(())
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
                ElementKind::Break => "break".into(),
                ElementKind::Image(_) => "image".into(),
                ElementKind::Subreport(_) | ElementKind::Content(_) => "subreport".into(),
                ElementKind::Barcode(b) => match b.symbology {
                    ferroreport::model::Symbology::Qr { .. } => "QRCode".into(),
                    ferroreport::model::Symbology::Code128 { .. } => "Code128".into(),
                },
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
    let report: Report = match ferroreport::parse(&source) {
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
