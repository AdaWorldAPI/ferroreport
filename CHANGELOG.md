# Changelog

Every release of ferroreport, newest first. From 1.0.0 on, versions follow
[semantic versioning](https://semver.org); what that covers is in the README under
[Stability](README.md#stability).

## Unreleased

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
  Code that builds a `PrintLine` itself needs `shaped: None`; code that reads one is not
  affected. The new `shape` module says whether a text needs shaping (`needs_shaping`), whether
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
