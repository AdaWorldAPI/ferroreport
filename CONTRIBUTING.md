# Contributing

Thanks for helping! The most valuable contribution is a **fixture**: a JRXML template plus
sample data that ferroreport renders differently from JasperReports (or cannot render).

## Layout

```
crates/ferroreport/      library: parser, expression interpreter, filler, PDF writer
crates/ferroreport-cli/  `ferroreport` command
fixtures/<name>/         report.jrxml, data.json, params.json (optional), expected.json
tools/compare.py         pixel comparison against JasperReports
tools/compare_sheets.py  cell comparison of the Excel and CSV exports
tools/bench.py           speed and memory of one fixture, cold
tools/bench_all.py       every number in the README: cold, warm JVM, 100,000 and 1,000,000 rows
tools/oracle/            the reference renderer (a small Java program around JasperReports)
```

## Everyday commands

```bash
cargo test                      # unit tests, every fixture's page count, streaming, mutation tests
HOSTILE_SEED=7 cargo test --test hostile   # other mutations of the templates than the usual ones
cargo deny check                # advisories, licenses and sources of the dependencies
cargo clippy --all-targets
cargo fmt
```

## Comparing against JasperReports

You need a JDK (17 or newer) and a directory with the JasperReports 6.x jars and their
dependencies, including `jasperreports-fonts` (the DejaVu families). Then:

```bash
cargo build --release
JASPER_LIB=/path/to/jars python3 tools/compare.py            # all fixtures
JASPER_LIB=/path/to/jars python3 tools/compare.py --only 02  # one fixture
```

Results and diff images (red: only in Jasper, blue: only in ferroreport) are written to
`target/compare/`. When a new fixture matches, record its page count in `expected.json` so
`cargo test` keeps it from regressing.

## Adding a fixture

1. Create `fixtures/NN-short-name/` with `report.jrxml` and, if needed, `data.json` (an array of
   objects keyed by field name) and `params.json`.
2. Use the DejaVu font families in the template so both engines use the same fonts.
3. Run the comparison. If it differs, open an issue with the diff image, or a pull request
   with the fix.

## Rules of thumb

- Behaviour must match JasperReports; when in doubt, measure it with the reference renderer
  and write down the rule you derived (see `text.rs` for an example).
- The expression interpreter must stay an allow-list. Never add reflection, class loading,
  file or network access.
