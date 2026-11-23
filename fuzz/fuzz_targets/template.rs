//! Any text as a template (JRXML 6, JRXML 7 or .ferro), rendered when it loads: no panic, no
//! hang beyond the limits, whatever the template says.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let limits = ferroreport::Limits { max_pages: Some(20), timeout: Some(std::time::Duration::from_secs(2)) };
    let mut renderer = ferroreport::Renderer::new().limits(limits);
    let Ok(report) = renderer.parse(text) else { return };
    for format in [ferroreport::Format::Pdf, ferroreport::Format::Csv, ferroreport::Format::Xlsx, ferroreport::Format::Html] {
        let rows: ferroreport::RowIter = Box::new(std::iter::empty());
        let _ = renderer.render_to(&report, rows, Default::default(), format, std::io::sink());
    }
});
