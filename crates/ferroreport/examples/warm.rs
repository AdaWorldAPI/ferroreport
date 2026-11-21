//! Times repeated renders of one fixture inside a single process, the way a long-running
//! service uses the library: `cargo run --release --example warm -- fixtures/02-invoice 200`.
//! Prints the median time of a whole render (template text to PDF bytes) and of a render
//! with the template already parsed.

use std::time::Instant;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().ok_or("usage: warm <fixture dir> [runs]")?);
    let runs: usize = args.next().map(|n| n.parse()).transpose()?.unwrap_or(200);
    let template = std::fs::read_to_string(dir.join("report.jrxml"))?;
    let read = |name: &str, default: &str| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(&std::fs::read_to_string(dir.join(name)).unwrap_or_else(|_| default.into()))?)
    };
    let (rows, params) = (read("data.json", "[]")?, read("params.json", "{}")?);

    let (mut whole, mut parsed) = (Vec::new(), Vec::new());
    let report = ferroreport::parse(&template)?;
    let mut size = 0;
    for _ in 0..runs {
        let t = Instant::now();
        let pdf = ferroreport::Renderer::new().resource_dir(&dir).render_json(&template, &rows, &params)?;
        whole.push(t.elapsed().as_secs_f64() * 1e3);
        size = pdf.len();

        let t = Instant::now();
        let mut r = ferroreport::Renderer::new().resource_dir(&dir);
        let records = ferroreport::data::records_from_json(&report, &rows)?;
        let doc = r.fill(&report, &records, ferroreport::params_from_json(&report, &params)?)?;
        r.to_pdf(&doc)?;
        parsed.push(t.elapsed().as_secs_f64() * 1e3);
    }
    println!("{{\"runs\": {runs}, \"whole_ms\": {:.3}, \"parsed_ms\": {:.3}, \"bytes\": {size}}}", median(whole), median(parsed));
    Ok(())
}
