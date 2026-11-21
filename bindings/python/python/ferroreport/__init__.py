"""Render JasperReports JRXML templates to PDF without a JVM.

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


def render(
    template: Union[PathLike, str],
    data: Any = None,
    params: Optional[Mapping[str, Any]] = None,
    *,
    locale: str = "en_US",
    resources: Optional[Mapping[str, Union[bytes, PathLike]]] = None,
    resource_dirs: Optional[Iterable[PathLike]] = None,
    now: Optional[_dt.datetime] = None,
) -> bytes:
    """Renders a template to PDF and returns the PDF bytes.

    template: path to a .jrxml file, or the JRXML source itself.
    data: rows for the main data source: a list of dicts (or objects), or JSON text.
    params: report parameters by name.
    locale: number and date formatting locale (en_US, es_DO, es_MX, es_ES, ...).
    resources: images and subreport templates in memory, by the relative path the template
        uses (for example {"logo.png": b"...", "lines.jrxml": "<jasperReport ..."}).
    resource_dirs: extra directories for images and subreports. The template's own directory
        is always searched. Paths in templates can never leave these directories.
    now: value of `new Date()` in expressions (for reproducible output).
    """
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
    try:
        return _ferroreport.render(
            source,
            _to_json(data if data is not None else []),
            _to_json(dict(params or {})),
            locale,
            dirs,
            mem,
            now.isoformat() if now else None,
        )
    except ValueError as e:
        raise FerroreportError(str(e)) from None


def render_to_file(template: Union[PathLike, str], output: PathLike, *args: Any, **kwargs: Any) -> None:
    """Like render(), writing the PDF to `output`."""
    pdf = render(template, *args, **kwargs)
    with open(os.fspath(output), "wb") as f:
        f.write(pdf)


def inspect(template: Union[PathLike, str]) -> dict:
    """Parses a template and returns its name, parameters, fields and the list of features
    this engine does not support yet (`warnings`)."""
    source, _ = _load_template(template)
    try:
        return _ferroreport.inspect(source)
    except ValueError as e:
        raise FerroreportError(str(e)) from None
