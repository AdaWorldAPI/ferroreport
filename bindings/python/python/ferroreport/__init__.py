"""Render JasperReports JRXML templates to PDF, Excel or CSV without a JVM.

>>> import ferroreport
>>> pdf = ferroreport.render("invoice.jrxml", data=[{"item": "Cemento", "amount": 395}],
...                          params={"CUSTOMER": "Distribuidora Caribe"}, locale="es_DO")
>>> open("invoice.pdf", "wb").write(pdf)
"""

from __future__ import annotations

import base64
import datetime as _dt
import decimal
import json
import os
from typing import Any, Iterable, Mapping, Optional, Union

from . import _ferroreport

__all__ = ["render", "render_to_file", "inspect", "FerroreportError", "__version__"]
__version__ = _ferroreport.__version__

PathLike = Union[str, "os.PathLike[str]"]


class FerroreportError(ValueError):
    """Raised when a template cannot be parsed, filled or exported."""


def _json_default(value: Any) -> Any:
    if isinstance(value, (_dt.datetime, _dt.date, _dt.time)):
        return value.isoformat()
    if isinstance(value, decimal.Decimal):
        return str(value)
    if isinstance(value, (bytes, bytearray, memoryview)):
        return base64.b64encode(bytes(value)).decode("ascii")
    if isinstance(value, (set, frozenset, tuple)):
        return list(value)
    if hasattr(value, "__dict__"):
        return {k: v for k, v in vars(value).items() if not k.startswith("_")}
    raise TypeError(f"cannot convert {type(value).__name__} to report data")


def _to_json(value: Any) -> str:
    if value is None:
        return "null"
    if isinstance(value, (str, bytes)) and not isinstance(value, bytes):
        # Already JSON text.
        return value
    return json.dumps(value, default=_json_default)


def _load_template(template: Union[PathLike, str]) -> "tuple[str, Optional[str]]":
    """Returns (source, directory of the template file or None)."""
    text = os.fspath(template) if not isinstance(template, str) else template
    if "<jasperReport" in text:
        return text, None
    with open(text, "r", encoding="utf-8") as f:
        source = f.read()
    return source, os.path.dirname(os.path.abspath(text))


def _prepare(template, resources, resource_dirs):
    source, directory = _load_template(template)
    dirs = [os.fspath(d) for d in (resource_dirs or [])]
    if directory:
        dirs.insert(0, directory)
    mem = {}
    for name, value in (resources or {}).items():
        if isinstance(value, (bytes, bytearray, memoryview)):
            mem[name] = bytes(value)
        elif isinstance(value, str) and "<jasperReport" in value:
            mem[name] = value.encode("utf-8")
        else:
            with open(os.fspath(value), "rb") as f:
                mem[name] = f.read()
    return source, dirs, mem


def render(
    template: Union[PathLike, str],
    data: Any = None,
    params: Optional[Mapping[str, Any]] = None,
    *,
    locale: str = "en_US",
    resources: Optional[Mapping[str, Union[bytes, PathLike]]] = None,
    resource_dirs: Optional[Iterable[PathLike]] = None,
    now: Optional[_dt.datetime] = None,
    format: str = "pdf",
    max_pages: Optional[int] = None,
    timeout: Optional[float] = None,
    csv_safe: bool = False,
    xlsx_numbers: bool = False,
) -> bytes:
    """Renders a template and returns the file's bytes: PDF by default, or an Excel workbook
    (format="xlsx") or CSV text encoded as UTF-8 (format="csv"), laid out like JasperReports'
    exporters.

    template: path to a .jrxml file, or the JRXML source itself.
    data: rows for the main data source: a list of dicts (or objects), or JSON text.
    params: report parameters by name.
    locale: number and date formatting locale (en_US, es_DO, es_MX, es_ES, ...).
    resources: images and subreport templates in memory, by the relative path the template
        uses (for example {"logo.png": b"...", "lines.jrxml": "<jasperReport ..."}).
    resource_dirs: extra directories for images and subreports. The template's own directory
        is always searched. Paths in templates can never leave these directories.
    now: value of `new Date()` in expressions (for reproducible output).
    format: "pdf", "xlsx" or "csv".
    max_pages, timeout: stop with FerroreportError if the report goes over this many pages
        or takes longer than this many seconds. Set them when templates or data come from
        other people.
    csv_safe: in CSV, put an apostrophe before values a spreadsheet would run as a formula.
    xlsx_numbers: in Excel, write numbers as numbers instead of formatted text.
    """
    source, dirs, mem = _prepare(template, resources, resource_dirs)
    try:
        return _ferroreport.render(
            source,
            _to_json(data if data is not None else []),
            _to_json(dict(params or {})),
            locale,
            dirs,
            mem,
            now.isoformat() if now else None,
            format,
            max_pages,
            timeout,
            csv_safe,
            xlsx_numbers,
        )[0]
    except ValueError as e:
        raise FerroreportError(str(e)) from None


def render_to_file(
    template: Union[PathLike, str],
    output: PathLike,
    data: Any = None,
    params: Optional[Mapping[str, Any]] = None,
    *,
    data_file: Optional[PathLike] = None,
    locale: str = "en_US",
    resources: Optional[Mapping[str, Union[bytes, PathLike]]] = None,
    resource_dirs: Optional[Iterable[PathLike]] = None,
    now: Optional[_dt.datetime] = None,
    format: Optional[str] = None,
    max_pages: Optional[int] = None,
    timeout: Optional[float] = None,
    csv_safe: bool = False,
    xlsx_numbers: bool = False,
) -> dict:
    """Like render(), writing the file to `output` as it is made: PDF and CSV pages go to
    disk as soon as they are finished, so memory stays flat however long the report is. The
    format follows the file's extension (.pdf, .xlsx or .csv) unless `format` is given.

    data_file: instead of `data`, the path of a file with the rows: a JSON array of objects,
        or one JSON object per line. It is read as the report needs it, so it can be larger
        than memory: a million rows render in about the memory of a thousand.

    Returns {"pages": ..., "rows": ...}.
    """
    if format is None:
        ext = os.path.splitext(os.fspath(output))[1].lower().lstrip(".")
        format = ext if ext in ("xlsx", "csv") else "pdf"
    if data is not None and data_file is not None:
        raise FerroreportError("give `data` or `data_file`, not both")
    source, dirs, mem = _prepare(template, resources, resource_dirs)
    try:
        _, pages, rows = _ferroreport.render(
            source,
            _to_json(data if data is not None else []),
            _to_json(dict(params or {})),
            locale,
            dirs,
            mem,
            now.isoformat() if now else None,
            format,
            max_pages,
            timeout,
            csv_safe,
            xlsx_numbers,
            os.fspath(data_file) if data_file is not None else None,
            os.fspath(output),
        )
    except ValueError as e:
        raise FerroreportError(str(e)) from None
    return {"pages": pages, "rows": rows}


def inspect(template: Union[PathLike, str]) -> dict:
    """Parses a template and returns its name, parameters, fields and the list of features
    this engine does not support yet (`warnings`)."""
    source, _ = _load_template(template)
    try:
        return _ferroreport.inspect(source)
    except ValueError as e:
        raise FerroreportError(str(e)) from None
