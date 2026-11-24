//! Large reports are written in the memory of a few pages, in every format that streams: the
//! heap at its highest while rendering 100,000 rows, measured by counting what is allocated.
//!
//! In a file of its own: the counting allocator is the whole test binary's, and the test
//! runs alone in it.

use ferroreport::{Format, JsonRows, Renderer, RowIter};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let now = NOW.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            if new_size >= layout.size() {
                let now = NOW.fetch_add(new_size - layout.size(), Ordering::Relaxed) + new_size - layout.size();
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                NOW.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Counts what `f` writes, so the document itself is not in memory.
struct Count(usize);

impl std::io::Write for Count {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The most the heap grew by while rendering, in MiB, the size of the document, and how
/// often the rows were read.
fn render(report: &ferroreport::Report, lines: &[u8], format: Format) -> (f64, usize, usize) {
    let mut asked = 0;
    let mut rows = || -> ferroreport::Result<RowIter> {
        asked += 1;
        Ok(Box::new(JsonRows::new(report, lines)))
    };
    let mut out = Count(0);
    let start = NOW.load(Ordering::Relaxed);
    PEAK.store(start, Ordering::Relaxed);
    Renderer::new().render_rows_to(report, &mut rows, Default::default(), format, &mut out).unwrap();
    let peak = PEAK.load(Ordering::Relaxed) - start;
    (peak as f64 / (1 << 20) as f64, out.0, asked)
}

#[test]
fn large_reports_take_the_memory_of_a_few_pages() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/09-large");
    let report = ferroreport::parse(&std::fs::read_to_string(dir.join("report.jrxml")).unwrap()).unwrap();
    let few: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(dir.join("data.json")).unwrap()).unwrap();
    let lines = |n: usize| -> Vec<u8> { (0..n).flat_map(|i| format!("{}\n", few[i % few.len()]).into_bytes()).collect() };
    // Fonts and the rest of what a first render sets up, outside the measure.
    let small = lines(100);
    for format in [Format::Pdf, Format::Xlsx, Format::Ods, Format::Odt] {
        render(&report, &small, format);
    }

    let large = lines(100_000);
    let mut peaks = Vec::new();
    for (format, passes) in [(Format::Pdf, 1), (Format::Xlsx, 2), (Format::Ods, 3), (Format::Odt, 2)] {
        let (mib, bytes, asked) = render(&report, &large, format);
        eprintln!("{format:?}: {mib:.1} MiB at most, {bytes} bytes written, rows read {asked} times");
        assert_eq!(asked, passes, "{format:?}");
        assert!(bytes > 1_000_000, "{format:?}: {bytes} bytes");
        peaks.push(mib);
    }
    let streamed = peaks[0].max(peaks[1]);
    for (format, peak) in [(Format::Ods, peaks[2]), (Format::Odt, peaks[3])] {
        // Kept in memory, the body of 100,000 rows alone is over 100 MiB.
        assert!(peak < streamed * 1.5 + 4.0, "{format:?}: {peak:.1} MiB, PDF and XLSX {streamed:.1} MiB");
    }

    // A small document is still made in one pass, in memory.
    for format in [Format::Ods, Format::Odt] {
        assert_eq!(render(&report, &lines(2_000), format).2, 1, "{format:?}");
    }
}
