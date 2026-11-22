//! Many renders at once inside one process, the way a service under load uses the library:
//! `cargo run --release --example concurrent -- fixtures/02-invoice 50 5` runs 50 threads for
//! 5 seconds, each rendering the report (already parsed) to PDF over and over, and prints how
//! many reports came out per second and how long each took.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().ok_or("usage: concurrent <fixture dir> [threads] [seconds]")?);
    let threads: usize = args.next().map(|n| n.parse()).transpose()?.unwrap_or(10);
    let seconds: f64 = args.next().map(|n| n.parse()).transpose()?.unwrap_or(5.0);
    let template = std::fs::read_to_string(dir.join("report.jrxml"))?;
    let read = |name: &str, default: &str| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(&std::fs::read_to_string(dir.join(name)).unwrap_or_else(|_| default.into()))?)
    };
    let shared = Arc::new((ferroreport::parse(&template)?, read("data.json", "[]")?, read("params.json", "{}")?, dir));
    let stop = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let (shared, stop) = (shared.clone(), stop.clone());
            std::thread::Builder::new()
                .stack_size(16 << 20)
                .spawn(move || {
                    let (report, rows, params, dir) = &*shared;
                    let mut times = Vec::new();
                    while !stop.load(Ordering::Relaxed) {
                        let t = Instant::now();
                        ferroreport::Renderer::new()
                            .resource_dir(dir)
                            .render_report_json(report, rows, params, ferroreport::Format::Pdf)
                            .expect("render");
                        times.push(t.elapsed().as_secs_f64() * 1e3);
                    }
                    times
                })
                .expect("thread")
        })
        .collect();
    std::thread::sleep(Duration::from_secs_f64(seconds));
    stop.store(true, Ordering::Relaxed);
    let mut all: Vec<f64> = workers.into_iter().flat_map(|w| w.join().expect("worker")).collect();
    let took = started.elapsed().as_secs_f64();
    all.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |p: f64| all[((all.len() as f64 * p) as usize).min(all.len() - 1)];
    println!(
        "{{\"threads\": {threads}, \"reports\": {}, \"per_second\": {:.1}, \"p50_ms\": {:.2}, \"p95_ms\": {:.2}, \"p99_ms\": {:.2}}}",
        all.len(),
        all.len() as f64 / took,
        at(0.5),
        at(0.95),
        at(0.99)
    );
    Ok(())
}
