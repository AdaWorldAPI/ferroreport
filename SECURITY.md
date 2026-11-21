# Security

ferroreport is built to render templates and data that come from people you do not trust. This
page says what that means, what limits protect the process, and how to report a problem.

## Reporting a vulnerability

Please report security problems privately through GitHub's
[security advisories](https://github.com/cristianEncarnacion/ferroreport/security/advisories/new)
rather than in a public issue. Include a template and data that show the problem if you can. You
will get an answer within a few days, and a fix as soon as possible.

## What a template cannot do

- **Run code.** Expressions are interpreted from a fixed list of operations (string, number, date
  and collection methods, formatting). There is no reflection, no class loading, no `Runtime`, no
  Groovy `execute()`, and compiled `.jasper` files are never deserialized: a subreport named
  `x.jasper` is read from `x.jrxml`.
- **Read files outside its resource directories.** Images and subreports are read only from the
  template's own directory and the ones you add. Absolute paths, `..` and symbolic links that
  point outside those directories are refused. Without a resource directory nothing is read
  from disk (the JavaScript build never touches it).
- **Use the network.** `http://`, `https://` and `file://` locations are not fetched.
- **Expand XML entities.** Templates with a `DOCTYPE` are refused, so entity bombs and external
  entities (XXE) cannot work.

## Limits on resources

A template or its data cannot make a render use unbounded stack, memory or time:

| Limit | Value |
|---|---|
| Element nesting in a template | 100 levels |
| Expression nesting (parentheses, operators, calls) | 256 levels |
| Image size | 64 million pixels, checked before decoding |
| Subreport nesting | 8 levels |
| Subreports, tables and lists filled in one render | 500,000 |
| HTTP request body (`ferroreport serve`) | 50 MB by default (`--max-body`) |

Other inputs grow the work in proportion to their size: pages follow the rows of the data.

## The HTTP server

`ferroreport serve` listens on `127.0.0.1` unless told otherwise. When it is reachable from other
machines, set a token (`--token` or `FERROREPORT_TOKEN`): requests must then send
`Authorization: Bearer <token>`, compared in constant time. Template names in URLs must be
relative paths inside `--templates`. A failure while rendering one request answers `500` and
does not stop the server.

## Spreadsheet exports

XLSX and CSV exports contain the report's text exactly as JasperReports writes it. A value that
starts with `=`, `+`, `-` or `@` is written as text in XLSX, but spreadsheet programs may treat
it as a formula when they open a CSV file. If your data comes from untrusted users and the CSV
is opened in Excel, prefer the XLSX export.

## Dependencies

`cargo audit` reports no known vulnerabilities in the dependencies. It lists `chrono` (dates)
and `ttf-parser` (fonts) as no longer maintained, with no known flaws.
