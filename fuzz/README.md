# Fuzzing

Templates and expressions can come from people you do not trust, so the engine is fuzzed: any
input may be refused with an error, but none may panic, hang past the limits or run out of
memory.

| Target | Input |
|---|---|
| `template` | any text as a template (JRXML 6, JRXML 7, `.ferro`), rendered to PDF, CSV, XLSX and HTML when it loads |
| `expression` | any text as a Java or Groovy expression, evaluated with fields, parameters and variables of every kind |
| `rows` | any bytes as a report's rows (a JSON array or JSON Lines) |
| `query` | any text as a report's SQL query (`$P{}`, `$P!{}`, `$X{}`) |

```bash
cargo install cargo-fuzz
cargo +nightly fuzz run expression -- -max_total_time=300
```

CI runs each target for a few minutes every week (`.github/workflows/fuzz.yml`). `oss-fuzz/`
holds the files [OSS-Fuzz](https://google.github.io/oss-fuzz/) asks for, to run them there
continuously.

What it has found so far, all fixed: BigDecimal arithmetic that panicked on overflow instead of
failing (`ArithmeticException`), a Groovy `sort` whose closure answered inconsistently (Rust's
sort panics on such comparators; a merge sort that does not is used), and `BASE(n, radix,
length)` with a huge length.
