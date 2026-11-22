#!/usr/bin/env python3
"""Checks `ferroreport convert` against JasperReports' own conversions.

JasperReports ships the same samples written as JRXML 6 (in 6.20) and as JRXML 7 (in 7.x, where
they were converted by JasperReports' own tools). For every template in both:

  1. JRXML 6 -> `ferroreport convert --to jrxml` is compared with JasperReports' JRXML 7;
  2. JRXML 7 -> `.ferro` -> JRXML 7 is compared with the JRXML 7 it came from.

Templates are compared as trees: attribute order, number spelling (8 and 8.0), whitespace around
texts, uuids, empty elements and the defaults JasperReports writes out (box styles,
columnBreakOffset="10", x/y="0") do not count. Some samples were edited between the two
releases; those show up as differences in the first check and are listed so they can be read.

Usage: tools/convert_check.py --v6 <6.20 demo/samples> --v7 <7.x demo/samples> [--bin PATH]
"""

import argparse
import subprocess
import sys
from collections import Counter
from pathlib import Path
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
TAGGED = "net.sf.jasperreports.export.pdf.tagged"


def local(tag: str) -> str:
    return tag.split("}")[-1]


def number(v: str) -> str:
    try:
        return repr(float(v))
    except ValueError:
        return v


def ignored(e, k: str, v: str) -> bool:
    k = local(k)
    return (
        k == "uuid"
        or (local(e.tag) == "box" and k == "style")
        or (k == "columnBreakOffset" and number(v) == "10.0")
        or (k in ("x", "y") and number(v) == "0.0")
    )


def canon(e):
    attrs = tuple(sorted((local(k), number(v)) for k, v in e.attrib.items() if not ignored(e, k, v)))
    kids = [k for k in (canon(c) for c in e if not (local(c.tag) == "property" and c.get("name") == TAGGED)) if k]
    text = (e.text or "").strip() if not len(e) else ""
    if not attrs and not kids and not text and local(e.tag) not in ("element", "band", "detail", "text", "expression"):
        return None
    return (local(e.tag), attrs, text, kids)


def first_diff(a, b, path=""):
    if a[0] != b[0]:
        return f"{path}: <{a[0]}> vs <{b[0]}>"
    kind = dict(a[1]).get("kind")
    p = f"{path}/{a[0]}" + (f"[{kind}]" if kind else "")
    if a[1] != b[1]:
        return f"{p}: attributes {sorted(set(a[1]) - set(b[1]))} vs {sorted(set(b[1]) - set(a[1]))}"
    if a[2] != b[2]:
        return f"{p}: text {a[2][:60]!r} vs {b[2][:60]!r}"
    ka, kb = sorted(a[3], key=lambda k: k[0]), sorted(b[3], key=lambda k: k[0])
    if [k[0] for k in ka] != [k[0] for k in kb]:
        ca, cb = Counter(k[0] for k in ka), Counter(k[0] for k in kb)
        return f"{p}: children {dict(ca - cb)} vs {dict(cb - ca)}"
    for x, y in zip(ka, kb):
        d = first_diff(x, y, p)
        if d:
            return d
    return None


def convert(exe: str, src: Path, to: str, text: str = None) -> str:
    args = [exe, "convert", "-" if text is not None else str(src), "--to", to]
    r = subprocess.run(args, input=text, capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError(r.stderr.strip())
    return r.stdout


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--v6", required=True, type=Path, help="JasperReports 6.20 demo/samples folder")
    ap.add_argument("--v7", required=True, type=Path, help="JasperReports 7.x demo/samples folder")
    ap.add_argument("--bin", default=str(ROOT / "target" / "release" / "ferroreport"))
    args = ap.parse_args()

    same, edited = 0, []
    for a in sorted(args.v6.rglob("*.jrxml")):
        rel = a.relative_to(args.v6)
        b = args.v7 / rel
        if not b.exists():
            continue
        try:
            ours = ET.fromstring(convert(args.bin, a, "jrxml").encode())
        except Exception as e:  # noqa: BLE001
            edited.append((rel, f"failed: {e}"))
            continue
        d = first_diff(canon(ours), canon(ET.parse(b).getroot()))
        if d:
            edited.append((rel, d))
        else:
            same += 1
    print(f"JRXML 6 -> 7: {same} of {same + len(edited)} samples as JasperReports converted them")
    for rel, d in edited:
        print(f"  {rel}: {d}")

    same, lost = 0, []
    for b in sorted(args.v7.rglob("*.jrxml")):
        rel = b.relative_to(args.v7)
        try:
            ferro = convert(args.bin, b, "ferro")
            back = ET.fromstring(convert(args.bin, b, "jrxml", text=ferro).encode())
        except Exception as e:  # noqa: BLE001
            lost.append((rel, f"failed: {e}"))
            continue
        d = first_diff(canon(back), canon(ET.parse(b).getroot()))
        if d:
            lost.append((rel, d))
        else:
            same += 1
    print(f"JRXML 7 -> .ferro -> JRXML 7: {same} of {same + len(lost)} samples come back the same")
    for rel, d in lost:
        print(f"  {rel}: {d}")
    return 1 if any(d.startswith("failed") for _, d in edited + lost) else 0


if __name__ == "__main__":
    sys.exit(main())
