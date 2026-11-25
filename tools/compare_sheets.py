#!/usr/bin/env python3
"""Compares ferroreport's Excel (XLSX) and CSV exports with JasperReports' on every fixture.

For each fixture, both engines export report.xlsx and report.csv. The CSV files must be equal,
and so must the worksheet and the styles of the workbooks (the parts that hold the cells,
merges, sizes, fonts, fills, borders and alignment) and where pictures are anchored. The first
difference is shown otherwise.

Usage: tools/compare_sheets.py [--oracle PATH] [--only NAME] [--out DIR] [--jobs N]
"""

import argparse
import os
import re
import subprocess
import zipfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PARTS = ["xl/worksheets/sheet1.xml", "xl/styles.xml", "xl/workbook.xml"]


def anchors(z: zipfile.ZipFile) -> list:
    d = z.read("xl/drawings/drawing1.xml").decode()
    return [re.sub(r"\s+", "", a) + f for a, f in re.findall(r"<xdr:from>(.*?)</xdr:to>.*?(<a:fillRect[^>]*>)", d, re.S)]


def first_difference(a: str, b: str) -> str:
    la, lb = a.replace(">", ">\n").splitlines(), b.replace(">", ">\n").splitlines()
    for i, (x, y) in enumerate(zip(la, lb)):
        if x != y:
            return f"line {i + 1}: jasper {x[:140]!r} / ours {y[:140]!r}"
    return f"jasper has {len(la)} lines, ours {len(lb)}"


def compare(fx: Path, args) -> tuple:
    out = Path(args.out) / fx.name
    out.mkdir(parents=True, exist_ok=True)
    data, params = fx / "data.json", fx / "params.json"
    notes = []
    for ext in ("xlsx", "csv"):
        jasper, ours = out / f"jasper.{ext}", out / f"ferroreport.{ext}"
        for f in (jasper, ours):
            f.unlink(missing_ok=True)
        cmd = [args.oracle, str(fx / "report.jrxml"), str(data) if data.exists() else "-", str(jasper)]
        if params.exists():
            cmd.append(str(params))
        # A fixture's own JasperReports font extension (font sets), as fontsets.txt below.
        env = dict(os.environ)
        if (fx / "jasper-extension").is_dir():
            env["ORACLE_CLASSPATH"] = str(fx / "jasper-extension")
        pj = subprocess.run(cmd, capture_output=True, text=True, cwd=fx, env=env)
        cmd = [args.bin, "render", str(fx / "report.jrxml"), "-o", str(ours), "-q", "--now", "2026-01-15T10:30:00"]
        if data.exists():
            cmd += ["-d", str(data)]
        if params.exists():
            cmd += ["-p", str(params)]
        # Fonts the fixture needs (fonts.txt: Family=file.ttf[:bold][:italic]), when installed.
        if (fx / "fonts.txt").exists():
            for spec in (fx / "fonts.txt").read_text().split():
                if Path(spec.split("=", 1)[-1].split(":")[0]).exists():
                    cmd += ["--font", spec]
        if (fx / "fontsets.txt").exists():
            for spec in (fx / "fontsets.txt").read_text().splitlines():
                if spec.strip():
                    cmd += ["--font-set", spec.strip()]
        po = subprocess.run(cmd, capture_output=True, text=True)
        if not jasper.exists():
            if (fx / "ORACLE-SKIP").exists():
                return fx.name, "SKIP", []
            return fx.name, "ERROR", ["jasper failed: " + (pj.stdout + pj.stderr)[-300:]]
        if po.returncode != 0:
            return fx.name, "ERROR", ["ferroreport failed: " + po.stderr[-300:]]
        if ext == "csv":
            a, b = jasper.read_text(encoding="utf-8"), ours.read_text(encoding="utf-8")
            if a != b:
                notes.append("csv " + first_difference(a.replace("\n", ">"), b.replace("\n", ">")))
        else:
            with zipfile.ZipFile(jasper) as za, zipfile.ZipFile(ours) as zb:
                for part in PARTS:
                    a, b = za.read(part).decode(), zb.read(part).decode()
                    if a != b:
                        notes.append(f"{part} " + first_difference(a, b))
                # Pictures: where each one is anchored and how it fills its cells (media names,
                # ids and the pixels of drawn charts differ by design).
                a, b = anchors(za), anchors(zb)
                if a != b:
                    notes.append(f"pictures: jasper {len(a)}, ours {len(b)}; first difference "
                                 + next((f"{x} / {y}" for x, y in zip(a, b) if x != y), "in count"))
    return fx.name, ("OK" if not notes else "DIFF"), notes


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--oracle", default=os.environ.get("JASPER_ORACLE", str(ROOT / "tools" / "oracle" / "run.sh")))
    ap.add_argument("--bin", default=str(ROOT / "target" / "release" / "ferroreport"))
    ap.add_argument("--only")
    ap.add_argument("--out", default=str(ROOT / "target" / "compare-sheets"))
    ap.add_argument("--jobs", type=int, default=4)
    args = ap.parse_args()
    fixtures = [
        fx
        for fx in sorted((ROOT / "fixtures").iterdir())
        # A fixture filled from a database (data.sql) needs a JDBC driver for the reference.
        if (fx / "report.jrxml").exists() and not (fx / "data.sql").exists() and (not args.only or args.only in fx.name)
    ]
    with ThreadPoolExecutor(args.jobs) as pool:
        results = list(pool.map(lambda fx: compare(fx, args), fixtures))
    ok = 0
    for name, status, notes in results:
        print(f"{status:5} {name}")
        for n in notes:
            print(f"      {n}")
        ok += status == "OK"
    compared = sum(1 for _, s, _ in results if s != "SKIP")
    print(f"\n{ok}/{compared} fixtures export the same XLSX and CSV as JasperReports")
    return 0 if ok == compared else 1


if __name__ == "__main__":
    raise SystemExit(main())
