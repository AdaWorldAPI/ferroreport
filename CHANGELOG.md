# Changelog

Every release of ferroreport, newest first. From 1.0.0 on, versions follow
[semantic versioning](https://semver.org); what that covers is in the README under
[Stability](README.md#stability).

## Unreleased

- PDF: a character the text's font does not have is left out, as JasperReports leaves it out,
  instead of being drawn as the font's missing-glyph box. It takes no width when the lines are
  drawn and aligned, and still counts as wide as that box for the text's height and where it is
  cut, as in JasperReports (checked against 6.20.6, identical pages). Text shaped with
  rustybuzz (right to left, complex scripts) still shows the box.
- Font sets, as the `<fontSet>` of a JasperReports font extension defines them:
  `--font-set "Serif Fallback=DejaVu Serif,DejaVu Sans"` (or `Renderer::register_font_set`)
  makes a text whose font name is `Serif Fallback` draw the characters DejaVu Serif lacks in
  DejaVu Sans. The text is cut into runs as JasperReports cuts it: the longest run one family
  has, in the first family that has all of it; a character no family has stays in the first
  family. Measured, broken into lines and embedded in the PDF with the font of each run;
  checked against JasperReports 6.20.6 with a font extension that defines the same sets
  (fixture 56-font-fallback, identical PDF, XLSX and CSV).
- `--font-fallback` (`Renderer::font_fallback`) does the same for every font, without a font
  set: characters a text's font lacks are drawn in the first of the `--font` families (in the
  order given), DejaVu Sans, DejaVu Serif and DejaVu Sans Mono that has them. Off by default,
  as JasperReports does not do it; texts whose characters are all in their font are drawn as
  before either way.

## 1.1.0

### Short forms in `.ferro`

For people who do not come from JasperReports. Each one compiles to the same JRXML as its long
form, the long forms keep working, and every 1.0 template compiles as it did (checked against
the JRXML 1.0.2 made of the examples, the documentation and every fixture).

- `text` is one part for both kinds of text: `text : Overdue` is a static text, and
  `text : Invoice {number} · {customer}` is a text field that prints
  `"Invoice " + $F{number} + " · " + $F{customer}`. `{total}` alone is the value itself, so
  `pattern=` formats it, and a text that starts with `=` is an expression
  (`text : ={qty} * {price}`). `\{`, `\}`, `\$`, `\=` and `\\` print the character itself.
- `{name}` means `$F{name}` in any expression (`if : {overdue}`, `each : {lines}`,
  `variable total Double calculation=Sum : {amount}`). Strings, comments and the places where
  Java writes a brace are left alone, and so is every brace in a report with
  `language=groovy` or `javascript`.
- Fields written as `{name}` need no `field` line: they are declared where they are used (the
  report, or the `each` they are in) with class `Object`, so their values keep the type the
  data gives them. Declare a field to give it a class, such as `Date` for dates.
- Attributes with plain names, on parts and styles: `color=` (forecolor), `background=`
  (backcolor, and `mode=Opaque` so it shows), `align=left|center|right|justify` and
  `valign=top|middle|bottom` (text alignment, or image alignment on an image), and `size=`
  (font size). Colors may be named as JasperReports names them (`red`, `gray`, `orange`...)
  or written in hex. When a line also gives the JRXML name, the JRXML name is used.
- `size=` on a text or a style is now written to JRXML as `fontSize=`. ferroreport always read
  it as the font size, so nothing it renders changes; JasperReports now reads it too.
- `ferroreport convert` still writes the long forms.
- `examples/invoices-short.ferro` is `examples/invoices.ferro` in the short forms, and prints
  the same PDF.
- VS Code extension: highlighting for `text`, `{name}`, escapes, color names and alignments,
  and snippets for `text`, `text=` (an expression) and `field`; the other snippets use the
  short forms.

### OpenDocument: ODS and ODT

- New output formats: OpenDocument spreadsheet (`.ods`) and text (`.odt`), for LibreOffice and
  other OpenDocument applications. Pick them with the extension of `-o`, `--format ods|odt`,
  `"format": "ods"` in the HTTP API, `Format::Ods` / `Format::Odt` in Rust, `format="ods"` in
  Python, `{ format: "odt" }` in JavaScript and `Format.ODS` / `Format.ODT` in Java.
- ODS is laid out like the XLSX export (and JasperReports' ODS exporter): every page on one
  sheet, cells cut at element edges, merged cells, fonts, colors, borders, padding, alignment,
  rotation, column widths and row heights, images over their cells, charts and barcodes as
  pictures, anchors as named ranges. `--xlsx-numbers` (`XlsxOptions::detect_cell_type`) now
  also writes ODS numbers as numbers, with a number style made from their Java pattern.
- ODT is laid out like the DOCX export (and JasperReports' ODT exporter): a table per page,
  a page break between pages, texts with their fonts and styled runs, cell borders and
  backgrounds, pictures, links and bookmarks. Ellipses and slanted lines are drawn as
  pictures.
- Both are valid packages (`mimetype` stored first, manifest, `content.xml`, `styles.xml`) with
  the report's page size. The 50 fixtures not filled from a database open in LibreOffice
  with as many pages as their PDF.
- Not yet: `net.sf.jasperreports.export.ods.*` / `.odt.*` properties, dates as date cells,
  per-mille number styles, upside-down text in ODT. Both formats keep the document's body in
  memory until it is written.
- `tools/oracle` can also write JasperReports' own `.ods` and `.odt`, for comparison.

### A visual designer

- **A visual designer in the browser**, the first version: <https://cristianencarnacion.github.io/ferroreport/designer/>.
  Paste or upload a sample of your data (JSON) and its fields are listed with their types; drag
  them onto the report's bands, or add text, lines, rectangles, ellipses, images, the page
  number and today's date. Select, move and resize with the mouse or the keyboard, on a grid;
  change text, font, colors, alignment, borders and number or date format in a properties
  panel; undo and redo. The PDF is made again beside the page as you edit, by the WebAssembly
  engine, so nothing is uploaded. It opens `.ferro` and `.jrxml`, saves both, and downloads the
  report as PDF, Excel or Word. Parts it cannot edit yet (charts, tables, crosstabs, subreports,
  barcodes) are shown and kept as they are.
- JavaScript: `toTree(template, options?)` gives any template (JRXML 6 or 7, or `.ferro`, with
  the files it imports in `options.resources`) as a tree of its JRXML 7 form, and
  `fromTree(tree, "jrxml" | "ferro")` writes one back. Rust: `ferroreport::design::to_tree` and
  `from_tree`.
- The website is assembled by `tools/assemble-site.sh`, for the Pages workflow and to try it
  locally; `bindings/wasm/build.sh` honours `CARGO_TARGET_DIR`.

### Right-to-left and complex scripts

- Arabic and Hebrew run right to left. Lines are ordered with the Unicode Bidirectional
  Algorithm, so numbers, Latin words and parentheses inside them stay in reading order, and
  Arabic letters join. A text whose paragraph starts with a right-to-left letter runs right to
  left as a whole, as JasperReports decides it; Left, Right and Center stay where they are, and
  the last line of a justified paragraph sits on the right. The two new fixtures (52-arabic,
  53-hebrew-mixed, styled text included) match JasperReports 6.20.6 pixel for pixel, with the
  same XLSX and CSV.
- Devanagari, Thai and the other complex scripts are shaped with rustybuzz: conjuncts, reph,
  vowel signs drawn before their consonant, stacked Thai marks, and the font's own mark
  positioning. They need a font that has them (`--font`); the bundled DejaVu has Arabic and
  Hebrew but not these. JasperReports 6.20's PDF exporter does not shape them, so their
  fixtures (54-devanagari, 55-thai) are compared by page count.
- Lines break at the Unicode line-break opportunities, measured with the shaped widths. Thai,
  Lao, Khmer and Myanmar have no spaces between words and no dictionary here: lines break
  before a leading vowel, after a syllable-final sign and at spaces, and never inside a cluster.
- PDF: the glyphs used after shaping are embedded and subsetted, and copying gives the
  characters as written: each glyph maps to the characters of its cluster (a ligature to all of
  them), and left-to-right runs whose glyphs are reordered (Devanagari) carry their text as
  `/ActualText`. `pdftotext` gives back the words of all four fixtures as written; in lines
  that mix directions it orders numbers and punctuation itself, as it does for JasperReports'
  PDFs.
- HTML and the chart pictures in XLSX and DOCX draw the shaped lines; HTML marks right-to-left
  lines `direction="rtl"`. Word documents mark right-to-left paragraphs `<w:bidi/>` (with Left
  and Right kept on the same side) and give complex-script runs their bold, italic and size
  (`bCs`, `iCs`, `szCs`); this goes beyond JasperReports' DOCX exporter, which writes neither.
  XLSX and CSV hold the text as written, as before.
- Chart titles, labels and legends with right-to-left or complex text are shaped too.
- Text in Latin, Greek, Cyrillic and the other simple scripts takes the same path as before:
  every existing fixture renders to the same bytes in every format.
- Fixtures can name the fonts they need in `fonts.txt` (`Family=file.ttf[:bold]`, the syntax
  of `--font`); `cargo test`, `tools/compare.py`, `tools/compare_sheets.py`, `tools/pdfua.sh`
  and `tools/pdfa.sh` register those that are installed. The PDF/UA workflow installs
  `fonts-freefont-ttf` for them.
- New dependencies: rustybuzz, unicode-bidi, unicode-script, unicode-linebreak and self_cell
  (which keeps each font's shaping tables next to its bytes, read once). The first three were
  already in the build through resvg.
- Speed: each font's shaping tables and shape plans are read once and kept, and a one-line
  text is shaped once for filling, exporting and drawing. 20,000 rows of the Arabic fixture take about
  0.6 s, of the Devanagari one about 0.7 s. Latin text pays one check per text for characters
  that need shaping (09-large: 2% more instructions).
- API: `print::PrintLine` has a new field, `shaped` (the line's glyphs when it is shaped).
  Code that builds a `PrintLine` itself no longer compiles; code that reads one is not
  affected. This breaks the letter of the 1.x promise for a type that is only meant to be read,
  so `PrintLine` is now `#[non_exhaustive]`: fields added later will not break anyone. The new `shape` module says whether a text needs shaping (`needs_shaping`), whether
  it runs right to left (`is_rtl`), and shapes a line (`line`).
- Still to do: hyphenation, fallback to another font for characters the chosen one lacks, and
  Arabic and Hebrew vowel marks that the font moves up or sideways can come out of `pdftotext`
  apart from their letter.

## 1.0.2

- VS Code extension: an icon, a README with an animation and a screenshot, and section names
  (`title`, `detail`, `row`...) now colored by every theme.
- No change to the engine: what 1.0.1 renders, 1.0.2 renders the same.

## 1.0.1

- The VS Code extension for `.ferro` is on the Visual Studio Marketplace
  (`cristianencarnacion.ferroreport`), published from the release workflow without a token.
- npm and PyPI packages are published through trusted publishing, without tokens.
- No change to the engine: what 1.0.0 renders, 1.0.1 renders the same.

## 1.0.0

The first stable release: the APIs, the command line, the HTTP API and the `.ferro` format are
now kept compatible within 1.x. Since 0.12:

### Data from SQL databases

- `render --db <url>` and `serve --db <url>` run the template's own SQL query, as JasperReports
  runs it over JDBC: `$P{}` as a bound parameter, `$P!{}` pasted into the text, `$X{IN}`,
  `$X{NOTIN}`, `$X{EQUAL}`, `$X{BETWEEN}` and the other clause functions. PostgreSQL,
  MySQL and MariaDB, SQL Server and SQLite; the URL can be the JDBC one the project already
  has. Rows stream from the database as pages are written.
- Subreports run their own queries on `$P{REPORT_CONNECTION}`.
- TLS follows libpq's `sslmode` (`disable`, `prefer`, `require`, `verify-ca`, `verify-full`,
  `sslrootcert`).
- The server gives the connection only to the templates of `--templates`, and a request cannot
  set a parameter that is pasted into the SQL (`$P!{}`).

### Expressions

- JasperReports' function library as Jaspersoft Studio lists it: 78 functions (`IF`, `SUM`,
  `ROUND_HALF_UP`, `DATEFORMAT`, `WORKDAY`, `NETWORKDAYS`, `WEEKNUM`...), checked on 265
  cases against JasperReports.
- `DATERANGE`, with relative ranges (`"MONTH-1"`) and fixed ones, `getStart()`/`getEnd()`,
  and in `$X{}` clauses.
- Groovy: lists, maps and ranges, closures (`findAll`, `collect`, `inject`, `sort`,
  `groupBy`...), `"${...}"` strings, `?.`, `**`, `<=>`, `in`, `as`, indexing, string
  helpers; checked on 128 cases against JasperReports. BigDecimal division keeps Groovy's scale.
- An expression that does not parse is reported with its line and column in the template.

### PDF signatures

- PAdES baseline B-B signatures: `--sign-key` and `--sign-cert` (PEM: RSA or ECDSA P-256) or a
  PKCS#12 file with `FERROREPORT_SIGN_PASSWORD`; `--sign-name`, `--sign-reason`,
  `--sign-location`, `--sign-contact`. `serve` takes the same options and signs every PDF it
  returns.
- Signed PDF/A-2b passes veraPDF on every fixture, and the signatures are verified with
  poppler's `pdfsig` and with pyHanko on every push.

### `.ferro` templates

- `import "parts.ferro"` shares definitions between templates (only from the template's own
  folder).
- `if : <expression>` prints its parts only when the expression is true, and leaves no gap when
  it is not.
- `each : <expression>` repeats its parts for every item of a list in the row.
- `render --watch` (`-w`) renders again whenever the template or its data change.
- A VS Code extension for `.ferro`: highlighting, snippets, comments and brackets
  (`editors/vscode`, attached to every release as a `.vsix`).

### Barcodes and images

- Every Barbecue type but PostNet: USPS, SCC-14, GTIN, shipment number, UCC-128 with
  `applicationIdentifierExpression`, random-weight UPC-A, Bookland, Code 39 extended and
  PDF417, matched bar for bar on 115 symbols Barbecue drew. PostNet is skipped with a warning
  (Barbecue draws it as bars of one height; barcode4j's POSTNET component is drawn).
- The default height of Barbecue's Code 128 family now matches Barbecue for long texts too.
- `scaleImage="RealHeight"` and `"RealSize"`: the element takes its height (and width) from the
  picture, as in JasperReports, and moves to the next page when it does not fit.

### Fixes

- A date with no pattern prints in the locale's short date and time, as JasperReports does.
- `WEEKDAY` counts from Monday unless asked otherwise.
- Control characters take no room and draw nothing; in Excel, C1 control characters are written
  as `?`, as JasperReports writes them.
- A report without pages exports the XLSX file JasperReports writes for it.
- Fuzzing found and fixed: an overflow in decimal arithmetic, a panic in sorting with an
  inconsistent comparison, and two inputs that took too long (`BASE`, `padLeft`).
- The minimum Rust version of the library is 1.86 (the 1.80 declared before was wrong); the
  command line needs 1.89.

### For contributors

- Fuzz targets for templates, expressions, rows and SQL queries (`fuzz/`), run every week in
  CI, with the files OSS-Fuzz needs.
- Fixtures 48 to 51 (SQL, the function library, the other Barbecue types, RealHeight and
  RealSize): 48 of the 51 render pixel for pixel like JasperReports (2 charts are close, the
  SQL one is checked from the same rows as JSON), and 49 export the same Excel and CSV.

### Known differences

- Charts follow JFreeChart's look closely but are not pixel-identical.
- Text is set left to right without shaping: right-to-left and complex scripts are not right
  yet, and there is no hyphenation.
- A PNG with a resolution other than 72 dpi in a `RealSize` element is drawn a point wider
  than JasperReports 7.1.1 draws it (as JasperReports 6.20.6 draws it).
- Not yet: scriptlets and other Java code, maps, ODS/ODT/PPTX, signatures with timestamps.

## 0.12.0 and earlier

The notes of every earlier release are on
[GitHub Releases](https://github.com/cristianEncarnacion/ferroreport/releases):

- **0.12.0:** PDF/A-2 and PDF/A-3 with attachments, AES-256 encryption, links, anchors and
  bookmarks in every format, Barbecue barcodes, `.ferro` without coordinates, `ferroreport
  migrate`, a limit on the size of a row.
- **0.11.0:** the `.ferro` template format and `ferroreport convert`, Word (DOCX) output,
  PDF417, SVG images, tagged PDFs checked against PDF/UA-1, JRXML 7.
- **0.10.0:** the nine missing chart types, RTF markup, HTML lists, more barcodes, PDF form
  fields, icon labels; checked against JasperReports 7.1.1's samples too.
- **0.9.0 and before:** the engine, Excel and CSV export, every chart type, subreports, tables,
  lists, crosstabs, streaming of large reports, HTML previews, the Java, Python, JavaScript and
  HTTP interfaces.
