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
