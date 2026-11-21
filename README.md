# ferroreport

**Your JasperReports templates, without the JVM.** ferroreport reads `.jrxml` files and renders
them to PDF from a single native binary: faster, with a fraction of the memory, and safe to use
with templates you did not write.

> Status: **preview (phase 2).** Every comparable template in the test suite renders pixel for
> pixel like JasperReports, including band splitting, images, subreports and QR codes. Some
> features are still missing; see [compatibility](#compatibility). Not affiliated with
> Jaspersoft or Cloud Software Group.

[Leer en español](README.es.md)

## Why

- **No JVM.** One ~8 MB binary (fonts included). Starts in milliseconds, which matters for CLIs,
  serverless functions and containers.
- **Identical output.** Every fixture is rendered by both engines and compared pixel by pixel
  (see [how output is verified](#how-output-is-verified)).
- **Safe with untrusted templates.** JasperReports compiles template expressions to Java and
  loads serialized `.jasper` files, which is how recent remote-code-execution bugs happened
  (CVE-2025-10492, CVE-2026-6009). ferroreport never deserializes objects and interprets
  expressions from a fixed allow-list: no reflection, no class loading, no network access. Images
  and subreports are only read from the template's resource directories.
- **Use it from any language.** Packages for JavaScript (Node.js, Deno, Bun, browsers) and
  Python, a Rust library, a CLI, and an HTTP API in a Docker image for everything else (Java,
  C#, PHP, Go, Ruby...).

## Use it from your language

> The npm, PyPI and crates.io packages are being registered; until they are published, use the
> [release binaries](https://github.com/cristianEncarnacion/ferroreport/releases), the Docker image, or
> build the packages from `bindings/` (see [RELEASING.md](RELEASING.md)).

**JavaScript / TypeScript** (Node.js, Deno, Bun, browsers): `npm install ferroreport`

```js
import { render } from "ferroreport";
const pdf = render(jrxmlSource, rows, { CUSTOMER: "Distribuidora Caribe" }, { locale: "es_DO" });
```

**Python**: `pip install ferroreport`

```python
import ferroreport
pdf = ferroreport.render("invoice.jrxml", rows, {"CUSTOMER": "Distribuidora Caribe"}, locale="es_DO")
```

**Any language, over HTTP** (Java, C#, PHP, Go, Ruby...):

```bash
docker run -p 8080:8080 -v "$PWD/templates:/templates" -e FERROREPORT_TEMPLATES=/templates \
  ghcr.io/cristianencarnacion/ferroreport

curl -X POST localhost:8080/render/invoice -H 'Content-Type: application/json' \
  -d '{"data": [{"item": "Cemento", "amount": 395}], "params": {"CUSTOMER": "Caribe"}}' -o invoice.pdf
```

`POST /render` also accepts the template itself (`{"template": "<jasperReport...", "data": ...,
"resources": {"logo.png": "<base64>"}}`), `POST /inspect` reports unsupported features, and
`GET /health` is there for load balancers. Set `FERROREPORT_TOKEN` to require a bearer token.

**Rust**: `cargo add ferroreport` (example below). **Command line**: download a binary from
[Releases](https://github.com/cristianEncarnacion/ferroreport/releases) or `cargo install
ferroreport-cli`; `-d -` and `-o -` read rows from stdin and write the PDF to stdout, which
makes it easy to call from scripts.

Language packages are published by the [release workflow](RELEASING.md).

## Quick start

```bash
cargo install --path crates/ferroreport-cli

ferroreport render invoice.jrxml --data rows.json --params params.json -o invoice.pdf
ferroreport doctor invoice.jrxml     # what in this template is (not yet) supported
```

`rows.json` is a JSON array of objects whose keys match the template's `<field>` names
(dotted names such as `customer.name` reach into nested objects). Values are converted to the
class each field declares (`java.math.BigDecimal`, `java.util.Date` from ISO-8601 strings,
`java.util.List` from nested arrays, `byte[]` from base64, ...).

Images and subreports are looked up next to the template (add more directories with
`--resources DIR`). A subreport that names `lines.jasper` is rendered from `lines.jrxml`: compiled
`.jasper` files are never loaded. Nested JSON arrays feed subreports, for example
`new JRMapCollectionDataSource($F{lines})` or
`((JsonDataSource)$P{REPORT_DATA_SOURCE}).subDataSource("lines")`.

As a library:

```rust
let template = std::fs::read_to_string("invoice.jrxml")?;
let rows: serde_json::Value = serde_json::from_str(&std::fs::read_to_string("rows.json")?)?;
let pdf = ferroreport::Renderer::new()
    .locale("es_DO")
    .render_json(&template, &rows, &serde_json::json!({ "CUSTOMER": "Distribuidora Caribe" }))?;
std::fs::write("invoice.pdf", pdf)?;
```

## Compatibility

| Area | Supported | Not yet |
|---|---|---|
| Templates | JRXML 6.x schema; JRXML 7 element syntax | `.jasper` files (by design), scriptlets, subdatasets, style templates |
| Bands | title, page header/footer, column header/footer, detail (multiple bands), summary, no data, background, last page footer; splitting bands across pages (`splitType` Stretch, Prevent, Immediate); multi-column layout (vertical print order, `isStartNewColumn`, column breaks) | horizontal print order |
| Elements | static text, text field, line, rectangle (rounded), ellipse, frame (split across pages), image (PNG/JPEG from files, base64 or `byte[]`), subreport, QR code and Code 128 barcodes (barcode4j components), page break (applied after its band) | chart, crosstab, table, list, other barcode types, Barbecue barcodes |
| Text | wrapping, stretch with overflow, padding, borders, left/center/right/justified alignment, all line-spacing modes, underline, strike-through, `isBlankWhenNull`, styled and HTML markup (bold, italic, underline, strike-through, fonts, sizes, colors, highlight, superscript, subscript, line breaks, lists), also across page breaks; rotation (Left, Right, UpsideDown) | RTF markup |
| Layout | `positionType` Float / FixRelativeToBottom, `stretchType`, `printWhenExpression`, `isRemoveLineWhenBlank`, `isPrintRepeatedValues` (with `isPrintInFirstWholeBand`, `printWhenGroupChanges`), `isPrintWhenDetailOverflows` | |
| Styles | default style, parent styles, element overrides, conditional styles (up to 4 per style) | |
| Images | scale modes RetainShape, FillFrame, Clip; alignment; `onErrorType` | remote URLs (by design), SVG and other formats |
| Subreports | data from nested JSON (`JRMapCollectionDataSource`, `JRBeanCollectionDataSource`, `subDataSource`, `JREmptyDataSource`), parameters, return values, splitting across pages with reprinted column headers | JDBC connections |
| Data | groups (start new page, reprint header, min height), variables: Sum, Count, DistinctCount, Average, Lowest, Highest, First, Nothing, System, StandardDeviation, Variance; reset by report/page/group; evaluation time Now/Report/Page/Group | embedded SQL queries (data comes from JSON) |
| Expressions | Java and Groovy subset: literals, `$F/$P/$V`, arithmetic with Java promotion rules, string concat, comparisons, ternary, Elvis, casts, common String/Number/BigDecimal/Date/List methods, `Math`, `String.format`, `new SimpleDateFormat(..).format(..)`, `new DecimalFormat(..).format(..)`, `Base64` decoding | arbitrary Java classes, `$R{}` resource bundles |
| Formatting | `DecimalFormat` patterns (HALF_EVEN), `SimpleDateFormat` patterns; locales en_US, es_DO, es_MX, es_ES | other locales |
| Fonts | DejaVu Sans/Serif/Mono (the `jasperreports-fonts` families) bundled; extra TTF/OTF via `--font` | font extension jars |

QR codes are encoded exactly like ZXing (same version, mask and modules) and are drawn the way
JasperReports draws them, which makes them a good fit for printed electronic invoices (for
example the Dominican e-CF stamp).

Run `ferroreport doctor template.jrxml` to get this verdict for a specific template.

## How output is verified

`tools/compare.py` renders every template in `fixtures/` with JasperReports 6.20.6 and with
ferroreport, rasterizes both PDFs and compares them pixel by pixel. A fixture matches when fewer
than 0.15% of the inked pixels differ (font anti-aliasing noise); most match exactly. Today all
22 comparable fixtures match.

| Fixture | What it exercises | Result |
|---|---|---|
| 01-employee-list | list, patterns, page numbers | identical |
| 02-invoice | parameters, BigDecimal math, boxes, stretch, "page X of Y", summary | identical |
| 03-grouped-sales | nested groups, group/page/report totals, averages, distinct counts, zebra rows | identical |
| 04b-stretch-prevent-split | frames, floating elements, stretching text, conditional printing | identical |
| 05-no-data | `whenNoDataType="NoDataSection"` | identical |
| 06-expressions | Java semantics, formatting, variables (First, Lowest, StdDev, BigDecimal average) | identical |
| 07-groovy | Groovy expressions | checked by hand¹ |
| 08-styles-and-shapes | style inheritance, borders, dashes, line spacing, shapes, Unicode | identical |
| 09-large | 20,000 rows, 477 pages | identical |
| 10-band-split-across-pages | a band larger than the space left on the page | identical |
| 11-null-values | null handling | identical |
| 12-group-options | start group on new page, reprinted headers, min height, group-time evaluation, background, last page footer | identical |
| 13-text-overflow | long texts split across pages | identical |
| 14-overflow-cells | split frames, `stretchType`, `isPrintWhenDetailOverflows` | identical |
| 15-images | PNG (RGB, alpha, grayscale), JPEG, scale modes, base64 images | identical |
| 16-layout-features | conditional styles, justified text, `isRemoveLineWhenBlank`, `isPrintRepeatedValues` | identical |
| 17-subreports | subreports over nested JSON, parameters, return values, split across pages | identical |
| 18-barcodes | QR codes (several error-correction levels), Code 128 with and without text | identical |
| 19-columns | three-column layout with groups, column headers/footers, `COLUMN_NUMBER`/`COLUMN_COUNT` | identical |
| 20-columns-summary | summary after a report that ends in the second column | identical |
| 21-markup | styled and HTML markup: mixed fonts, sizes, colors, highlight, superscript, alignment, justification | identical |
| 22-markup-overflow | styled text that continues on the next page | identical |
| 23-rotation | rotated text (left, right, upside down) with padding and alignment | identical |

¹ The reference bundle's Groovy 4.0.8 cannot run on Java 21; run the comparison on Java 17 to
include it.

To run it yourself you need a JDK and the JasperReports jars (`jasperreports`,
`jasperreports-fonts` and their dependencies) in one directory:

```bash
cargo build --release
JASPER_LIB=/path/to/jars python3 tools/compare.py      # diff images land in target/compare/
JASPER_LIB=/path/to/jars python3 tools/bench.py 02-invoice
```

## Performance

Measured with `tools/bench.py` on a 2-vCPU Linux VM (median of 5 runs, full command-line render
from process start to PDF on disk):

| Fixture | JasperReports 6.20 (Java 21) | ferroreport | Speed-up | Peak memory |
|---|---|---|---|---|
| 02-invoice (2 pages) | 3.01 s | 0.010 s | 309× | 194 MiB → 16 MiB |
| 03-grouped-sales (4 pages) | 3.22 s | 0.013 s | 253× | 205 MiB → 16 MiB |
| 09-large (20,000 rows, 477 pages) | 7.22 s | 0.39 s | 19× | 489 MiB → 52 MiB |

Most of Jasper's time in small reports is JVM start-up and JRXML compilation, which a
long-running server amortises; counting only Jasper's own fill + export phases (still in a cold
JVM) ferroreport is about 85× faster on the small reports and 13× on the large one. Its PDFs are
also smaller (1.1 MB instead of 4.5 MB for 09-large).

## Roadmap

- **Phase 2 (in progress):** done: band splitting, images, subreports, conditional styles,
  justified text, QR codes, Code 128, multi-column layout, styled/HTML text and rotated text. Next: more
  barcode types, constant memory for huge reports (streaming pages to the PDF).
- **Phase 3:** npm and PyPI packages, WebAssembly build with a browser playground, VS Code
  live preview.
- **Phase 4:** charts, tables, crosstabs, Excel export.

## Contributing

Contributions are welcome, especially new fixtures: a JRXML plus sample data that renders
differently from JasperReports is the most useful bug report. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
The bundled DejaVu fonts are under the [DejaVu license](crates/ferroreport/assets/fonts/LICENSE-DejaVu).
JasperReports and Jaspersoft are trademarks of Cloud Software Group, Inc.; this project is
independent and is not endorsed by them.
