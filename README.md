# ferroreport

**The fastest way to render JasperReports.** ferroreport reads the `.jrxml` templates you
already have and renders them to PDF, Excel (XLSX) or CSV in milliseconds, without a JVM: about
**300× faster** than JasperReports on everyday reports, with **a tenth of the memory**, PDFs that
match **pixel for pixel**, spreadsheets with **the same cells** Jasper produces, and templates
that **cannot run code** on your server.

<p align="center"><a href="https://cristianencarnacion.github.io/ferroreport/"><img src="docs/race.gif" alt="Race: JasperReports renders 1 invoice in 3.01 s; ferroreport renders 301 in the same time" width="760"></a></p>

**[Try it in your browser](https://cristianencarnacion.github.io/ferroreport/)** ·
[Download](https://github.com/cristianEncarnacion/ferroreport/releases) ·
[Docker](https://github.com/cristianEncarnacion/ferroreport/pkgs/container/ferroreport) ·
[Leer en español](README.es.md)

> Status: **preview.** All 29 comparable test templates render pixel for pixel like
> JasperReports 6.20, including band splitting, images, subreports, tables, lists, crosstabs, QR codes and barcodes, styled text and
> multi-column layouts; charts follow JFreeChart's look closely. All 31 also export the same
> Excel and CSV files as Jasper. Some features are still missing; see
> [compatibility](#compatibility). Not affiliated with Jaspersoft or Cloud Software Group.

## Why

- **No JVM.** One ~8 MB binary (fonts included). Starts in milliseconds, which matters for CLIs,
  serverless functions and containers.
- **Identical output.** Every fixture is rendered by both engines and compared pixel by pixel,
  and its Excel and CSV exports cell by cell (see [how output is verified](#how-output-is-verified)).
- **PDF, Excel and CSV.** The same template gives a PDF, an `.xlsx` workbook laid out like
  JasperReports' (merged cells, fonts, fills, borders, row heights, images, charts) or a CSV.
- **Safe with untrusted templates.** JasperReports compiles template expressions to Java and
  loads serialized `.jasper` files, which is how recent remote-code-execution bugs happened
  (CVE-2025-10492, CVE-2026-6009). ferroreport never deserializes objects and interprets
  expressions from a fixed allow-list: no reflection, no class loading, no network access. Images
  and subreports are only read from the template's resource directories, and limits on nesting,
  image size and subreports keep a hostile template from exhausting memory or time
  ([SECURITY.md](SECURITY.md)).
- **Use it from any language.** Packages for JavaScript (Node.js, Deno, Bun, browsers) and
  Python, a Rust library, a CLI, and an HTTP API in a Docker image for everything else (Java,
  C#, PHP, Go, Ruby...).

## Use it from your language

> **[Full documentation](https://cristianencarnacion.github.io/ferroreport/docs.html)**: every command, endpoint, function and option.
> Available now: the [npm package](https://www.npmjs.com/package/ferroreport), the
> [crates](https://crates.io/crates/ferroreport), the
> [release binaries](https://github.com/cristianEncarnacion/ferroreport/releases) and the Docker image.
> The PyPI package is being published.

**JavaScript / TypeScript** (Node.js, Deno, Bun, browsers): `npm install ferroreport`

```js
import { render } from "ferroreport";
const pdf = render(jrxmlSource, rows, { CUSTOMER: "Distribuidora Caribe" }, { locale: "es_DO" });
const xlsx = render(jrxmlSource, rows, params, { format: "xlsx" }); // or "csv"
```

**Python**: `pip install ferroreport`

```python
import ferroreport
pdf = ferroreport.render("invoice.jrxml", rows, {"CUSTOMER": "Distribuidora Caribe"}, locale="es_DO")
ferroreport.render_to_file("invoice.jrxml", "invoice.xlsx", rows)  # format from the extension
```

**Any language, over HTTP** (Java, C#, PHP, Go, Ruby...):

```bash
docker run -p 8080:8080 -v "$PWD/templates:/templates" -e FERROREPORT_TEMPLATES=/templates \
  ghcr.io/cristianencarnacion/ferroreport

curl -X POST localhost:8080/render/invoice -H 'Content-Type: application/json' \
  -d '{"data": [{"item": "Cemento", "amount": 395}], "params": {"CUSTOMER": "Caribe"}}' -o invoice.pdf
```

`POST /render` also accepts the template itself (`{"template": "<jasperReport...", "data": ...,
"resources": {"logo.png": "<base64>"}}`); add `"format": "xlsx"` or `"csv"` (or `?format=xlsx`)
for Excel or CSV. `POST /inspect` reports unsupported features, and `GET /health` is there for
load balancers. Set `FERROREPORT_TOKEN` to require a bearer token.

**Rust**: `cargo add ferroreport` (example below). **Command line**: download a binary from
[Releases](https://github.com/cristianEncarnacion/ferroreport/releases) or `cargo install
ferroreport-cli`; `-d -` and `-o -` read rows from stdin and write the PDF to stdout, which
makes it easy to call from scripts.

Language packages are published by the [release workflow](RELEASING.md).

## Quick start

```bash
cargo install --path crates/ferroreport-cli

ferroreport render invoice.jrxml --data rows.json --params params.json -o invoice.pdf
ferroreport render invoice.jrxml --data rows.json -o invoice.xlsx   # Excel (or .csv)
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
// Excel or CSV: render_json_as(&template, &rows, &params, ferroreport::Format::Xlsx)
```

### Excel and CSV

The XLSX and CSV exports follow JasperReports' own exporters with their default settings, so a
spreadsheet made from a template looks the way it did with Jasper:

- All pages go on one sheet named after the report. Element edges and page margins cut the page
  into columns and rows; each text takes the cells it covers (merged), with its font, colors,
  borders, alignment and rotation; styled and HTML text keeps its runs.
- Lines become cell borders, rectangles and frames fill and outline their cells, and images sit
  over their cells with the same scale mode and alignment. Charts and barcodes are drawn as
  pictures (at twice Jasper's resolution, so they stay sharp).
- Values are written as the formatted text, like Jasper. Pass `--xlsx-numbers` (or
  `XlsxOptions { detect_cell_type: true }`) to write numbers as numbers with their format.
- CSV has one row for every line where a text starts, and one field for every column where a
  text starts on that page.

## Compatibility

| Area | Supported | Not yet |
|---|---|---|
| Templates | JRXML 6.x schema; JRXML 7 element syntax; subdatasets (for tables) | `.jasper` files (by design), scriptlets, style templates, subdataset queries |
| Bands | title, page header/footer, column header/footer, detail (multiple bands), summary, no data, background, last page footer; splitting bands across pages (`splitType` Stretch, Prevent, Immediate); multi-column layout (vertical print order, `isStartNewColumn`, column breaks) | horizontal print order |
| Elements | static text, text field, line, rectangle (rounded), ellipse, frame (split across pages), image (PNG/JPEG from files, base64 or `byte[]`), subreport, crosstab (row and column groups with totals at the start or end, header positions, several measures with Sum, Count, DistinctCount, Average, Highest, Lowest and First, percentages of the grand total (`percentageOf`), buckets ordered by a total (`orderByExpression`), `$V{measure_group_ALL}` totals, crosstab parameters, data from the report or a subdataset; column parts as wide as the element and rows continuing on the next pages with repeated headers), table component (column groups, table/column/group headers and footers, hidden columns, no-data cell, headers repeated on every page), list component (vertical), charts, barcodes (barcode4j components: QR code, Code 128, Code 39 with full ASCII, EAN-13, EAN-8, UPC-A, Interleaved 2 of 5, Codabar), page break (applied after its band) | crosstab bucket comparators, crosstabs over a group's rows, horizontal lists, UPC-E, EAN-128, PDF417, DataMatrix, UPC/EAN add-ons, rotated barcodes, Barbecue barcodes |
| Text | wrapping, stretch with overflow, padding, borders, left/center/right/justified alignment, all line-spacing modes, underline, strike-through, `isBlankWhenNull`, styled and HTML markup (bold, italic, underline, strike-through, fonts, sizes, colors, highlight, superscript, subscript, line breaks, lists), also across page breaks; rotation (Left, Right, UpsideDown) | RTF markup |
| Layout | `positionType` Float / FixRelativeToBottom, `stretchType`, `printWhenExpression`, `isRemoveLineWhenBlank`, `isPrintRepeatedValues` (with `isPrintInFirstWholeBand`, `printWhenGroupChanges`), `isPrintWhenDetailOverflows` | |
| Styles | default style, parent styles, element overrides, conditional styles (up to 4 per style) | |
| Images | scale modes RetainShape, FillFrame, Clip; alignment; `onErrorType` | remote URLs (by design), SVG and other formats |
| Charts | pie, 3D pie, bar, 3D bar, stacked bar (vertical and horizontal), line, area, stacked area, XY line, XY area, scatter; titles, legends, axis labels, series colors, range limits, tick formats; datasets by record, group, page or report, with `evaluationTime` | time series, meter, thermometer, Gantt, candlestick, bubble, multi-axis, chart customizers. Charts follow JFreeChart's default look closely but are not pixel-identical |
| Subreports | data from nested JSON (`JRMapCollectionDataSource`, `JRBeanCollectionDataSource`, `subDataSource`, `JREmptyDataSource`), parameters, return values, splitting across pages with reprinted column headers | JDBC connections |
| Data | groups (start new page, reprint header, min height), variables: Sum, Count, DistinctCount, Average, Lowest, Highest, First, Nothing, System, StandardDeviation, Variance; reset by report/page/group; evaluation time Now/Report/Page/Group | embedded SQL queries (data comes from JSON) |
| Expressions | Java and Groovy subset: literals, `$F/$P/$V`, arithmetic with Java promotion rules, string concat, comparisons, ternary, Elvis, casts, common String/Number/BigDecimal/Date/List methods, `Math`, `String.format`, `new SimpleDateFormat(..).format(..)`, `new DecimalFormat(..).format(..)`, `Base64` decoding | arbitrary Java classes, `$R{}` resource bundles |
| Formatting | `DecimalFormat` patterns (HALF_EVEN), `SimpleDateFormat` patterns; locales en_US, es_DO, es_MX, es_ES | other locales |
| Fonts | DejaVu Sans/Serif/Mono (the `jasperreports-fonts` families) bundled; extra TTF/OTF via `--font` | font extension jars |
| Output | PDF; XLSX and CSV laid out like Jasper's exporters (default settings) | DOCX, HTML, ODS/ODT, XLS (old format), exporter properties (`net.sf.jasperreports.export.*`) other than number detection |

QR codes are encoded exactly like ZXing (same version, mask and modules) and are drawn the way
JasperReports draws them, which makes them a good fit for printed electronic invoices (for
example the Dominican e-CF stamp).

Run `ferroreport doctor template.jrxml` to get this verdict for a specific template.

## How output is verified

`tools/compare.py` renders every template in `fixtures/` with JasperReports 6.20.6 and with
ferroreport, rasterizes both PDFs and compares them pixel by pixel. A fixture matches when fewer
than 0.15% of the inked pixels differ (font anti-aliasing noise); most match exactly. Today all
29 comparable fixtures match. Barcode geometry is also checked bar by bar against barcode4j itself
(`crates/ferroreport/tests/barcode4j.rs`).

`tools/compare_sheets.py` does the same for spreadsheets: both engines export every fixture to
XLSX and CSV, and the worksheet XML, the styles XML and the CSV text must be byte for byte the
same, and every picture anchored to the same cells. All 31 fixtures pass. Jasper's output for five
of them is kept in `fixtures/*/jasper-xlsx/`, so `cargo test` checks them without Java.

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
| 24-charts | pie, 3D pie, bar, stacked horizontal bar, line and area charts | same layout (charts are not compared pixel by pixel) |
| 25-charts-xy | XY line and scatter charts per group (`evaluationTime="Group"`) | same layout (charts are not compared pixel by pixel) |
| 26-barcodes-1d | EAN-13, UPC-A (text on top), EAN-8, Code 39 with check character, Interleaved 2 of 5, Codabar | identical |
| 27-table | Table component as Jaspersoft Studio's wizard builds it: styled cells, column group header, stretching rows, column footers and table footer, a table spanning pages | identical |
| 28-table-groups | Table with subdataset groups (group header/footer cells), nested column groups, a hidden column, multi-row headers, no-data cell | identical |
| 29-list | List component: contact cards with stretching notes, an empty list, a list spanning pages | identical |
| 30-crosstab | Crosstab as Jaspersoft Studio builds it: two row groups and a column group with totals, header cell, styled cells | identical |
| 31-crosstab-pages | Crosstab per detail row over a subdataset: two column groups, totals at the start, descending order, centered headers, Sum/Count/Average measures, crosstab parameter, column parts and rows across pages | identical |
| 32-crosstab-percent | Crosstab with percentages of the grand total (BigDecimal and Integer) and regions ordered by their total sales | identical |

¹ The reference bundle's Groovy 4.0.8 cannot run on Java 21; run the comparison on Java 17 to
include it.

To run it yourself you need a JDK and the JasperReports jars (`jasperreports`,
`jasperreports-fonts` and their dependencies) in one directory:

```bash
cargo build --release
JASPER_LIB=/path/to/jars python3 tools/compare.py      # diff images land in target/compare/
JASPER_LIB=/path/to/jars python3 tools/compare_sheets.py   # XLSX and CSV
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
  justified text, QR codes and barcodes, multi-column layout, styled/HTML text, rotated text, charts and
  tables, lists and crosstabs. Next: constant memory for huge reports (streaming pages to the PDF).
- **Phase 3:** npm and PyPI packages, WebAssembly build with a browser playground, VS Code
  live preview.
- **Phase 4:** more chart types; done: Excel (XLSX) and CSV export.

## Contributing

Contributions are welcome, especially new fixtures: a JRXML plus sample data that renders
differently from JasperReports is the most useful bug report. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
The bundled DejaVu fonts are under the [DejaVu license](crates/ferroreport/assets/fonts/LICENSE-DejaVu).
JasperReports and Jaspersoft are trademarks of Cloud Software Group, Inc.; this project is
independent and is not endorsed by them.
