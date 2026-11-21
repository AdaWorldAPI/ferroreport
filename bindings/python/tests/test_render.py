import datetime as dt
import decimal
import os
import pathlib

import pytest

import ferroreport

ROOT = pathlib.Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "fixtures"


def test_renders_fixture_from_path():
    fx = FIXTURES / "02-invoice"
    import json

    data = json.loads((fx / "data.json").read_text())
    params = json.loads((fx / "params.json").read_text()) if (fx / "params.json").exists() else {}
    pdf = ferroreport.render(fx / "report.jrxml", data, params)
    assert pdf.startswith(b"%PDF")


def test_python_values_and_source_text():
    template = """<?xml version="1.0"?>
<jasperReport xmlns="http://jasperreports.sourceforge.net/jasperreports" name="t" pageWidth="200" pageHeight="200" columnWidth="200" leftMargin="0" rightMargin="0" topMargin="0" bottomMargin="0">
  <field name="when" class="java.util.Date"/>
  <field name="amount" class="java.math.BigDecimal"/>
  <detail><band height="20"><textField pattern="#,##0.00"><reportElement x="0" y="0" width="200" height="20"/>
    <textFieldExpression><![CDATA[$F{amount}]]></textFieldExpression></textField></band></detail>
</jasperReport>"""
    pdf = ferroreport.render(template, [{"when": dt.datetime(2026, 1, 2), "amount": decimal.Decimal("10.50")}])
    assert pdf.startswith(b"%PDF")


def test_subreport_from_memory():
    fx = FIXTURES / "17-subreports"
    import json

    pdf = ferroreport.render(
        (fx / "report.jrxml").read_text(),
        json.loads((fx / "data.json").read_text()),
        resources={"lines.jrxml": (fx / "lines.jrxml").read_bytes()},
    )
    assert pdf.startswith(b"%PDF")


def test_inspect_and_errors():
    info = ferroreport.inspect(FIXTURES / "17-subreports" / "report.jrxml")
    assert info["name"] == "orders"
    assert "lines" in info["fields"]
    with pytest.raises(ferroreport.FerroreportError):
        ferroreport.render("<jasperReport", [])


def test_excel_and_csv(tmp_path):
    import json
    import zipfile

    fx = FIXTURES / "02-invoice"
    rows = json.loads((fx / "data.json").read_text())
    params = json.loads((fx / "params.json").read_text())
    xlsx = ferroreport.render(fx / "report.jrxml", rows, params, format="xlsx")
    assert xlsx.startswith(b"PK")
    out = tmp_path / "invoice.xlsx"
    out.write_bytes(xlsx)
    assert "xl/worksheets/sheet1.xml" in zipfile.ZipFile(out).namelist()
    csv = ferroreport.render(fx / "report.jrxml", rows, params, format="csv").decode("utf-8")
    assert csv.splitlines()[0] == ",Ferro Industrial SRL,,,INVOICE #F-2026-,,,"
    ferroreport.render_to_file(fx / "report.jrxml", tmp_path / "invoice.csv", rows, params)
    assert (tmp_path / "invoice.csv").read_text().startswith(",Ferro")
    with pytest.raises(ferroreport.FerroreportError):
        ferroreport.render(fx / "report.jrxml", rows, params, format="docx")


def test_streams_a_data_file_to_a_file(tmp_path):
    import json

    fx = FIXTURES / "33-page-x-of-y"
    rows = json.loads((fx / "data.json").read_text())
    lines = tmp_path / "rows.jsonl"
    lines.write_text("\n".join(json.dumps(r) for r in rows))
    out = tmp_path / "out.pdf"
    stats = ferroreport.render_to_file(fx / "report.jrxml", out, data_file=lines)
    assert stats == {"pages": 5, "rows": 190}
    assert out.read_bytes().startswith(b"%PDF") and not (tmp_path / "out.pdf.part").exists()
    # The same rows in memory give the same number of pages, and CSV by extension.
    assert ferroreport.render_to_file(fx / "report.jrxml", tmp_path / "out.csv", rows)["pages"] == 5
    assert (tmp_path / "out.csv").read_text().count("\n") > 190


def test_limits_and_csv_guard(tmp_path):
    import json

    fx = FIXTURES / "33-page-x-of-y"
    rows = json.loads((fx / "data.json").read_text())
    with pytest.raises(ferroreport.FerroreportError, match="more than 2 pages"):
        ferroreport.render(fx / "report.jrxml", rows, max_pages=2)
    with pytest.raises(ferroreport.FerroreportError, match="longer than"):
        ferroreport.render(fx / "report.jrxml", rows, timeout=1e-9)
    with pytest.raises(ferroreport.FerroreportError):
        ferroreport.render_to_file(fx / "report.jrxml", tmp_path / "x.pdf", rows, max_pages=2)
    assert not list(tmp_path.iterdir())
    rows[0]["name"] = "=HYPERLINK(\"http://evil\")"
    assert b"'=HYPERLINK" in ferroreport.render(fx / "report.jrxml", rows, format="csv", csv_safe=True)
    assert b"'=HYPERLINK" not in ferroreport.render(fx / "report.jrxml", rows, format="csv")


def test_rows_from_a_generator_and_html(tmp_path):
    fx = FIXTURES / "09-large"

    def rows():
        for i in range(1, 5001):
            yield {"id": i, "name": f"Employee {i}", "department": "Sales", "salary": 1000.5 + i}

    stats = ferroreport.render_to_file(fx / "report.jrxml", tmp_path / "list.csv", rows())
    assert stats["rows"] == 5000
    assert "Employee 5000" in (tmp_path / "list.csv").read_text()
    pdf = ferroreport.render(fx / "report.jrxml", rows(), max_pages=500)
    assert pdf.startswith(b"%PDF")
    html = ferroreport.render(fx / "report.jrxml", list(rows())[:50], format="html").decode()
    assert html.startswith("<!DOCTYPE html>") and "Employee 50</text>" in html
    assert ferroreport.render_to_file(fx / "report.jrxml", tmp_path / "list.html", list(rows())[:50])["pages"] == 2
