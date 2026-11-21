# ferroreport for Python

The fastest way to render **JasperReports `.jrxml` templates to PDF, Excel or CSV** from Python. No JVM, no
Java bridge: the engine is native Rust, about 300× faster than JasperReports end to end on
everyday reports ([benchmarks](https://github.com/cristianEncarnacion/ferroreport#performance)). Output is verified pixel by pixel against JasperReports 6.20, it is
hundreds of times faster to start, and templates cannot run code, so they are safe to accept
from users.

```bash
pip install ferroreport
```

```python
import ferroreport

pdf = ferroreport.render(
    "invoice.jrxml",                                   # path or JRXML source text
    data=[{"item": "Cemento Portland", "qty": 10, "price": 395}],
    params={"CUSTOMER": "Distribuidora Caribe"},
    locale="es_DO",
)
with open("invoice.pdf", "wb") as f:
    f.write(pdf)
```

- `data` is a list of dicts (or objects); `datetime`, `date`, `Decimal` and `bytes` (images)
  are converted to the classes the template declares. Nested lists feed subreports.
- Images and subreports are found next to the template; add `resource_dirs=[...]`, or pass
  them in memory with `resources={"logo.png": b"...", "lines.jrxml": "..."}`. A subreport
  named `lines.jasper` is read from `lines.jrxml`; compiled `.jasper` files are never loaded.
- `format="xlsx"` returns an Excel workbook and `format="csv"` CSV text (UTF-8 bytes), laid out
  like JasperReports' exporters.
- `ferroreport.render_to_file(template, "out.pdf", data, params)` writes the file directly; the
  format follows the extension (`.pdf`, `.xlsx`, `.csv`).
- `ferroreport.inspect(template)` returns the template's parameters, fields and the features
  the engine does not support yet.
- Rendering releases the GIL, so it runs in parallel from threads (Django, FastAPI, Celery).

Errors raise `ferroreport.FerroreportError`.

See the [project README](https://github.com/cristianEncarnacion/ferroreport) for supported
JasperReports features, benchmarks and other languages.
