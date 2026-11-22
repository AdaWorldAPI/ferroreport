#!/usr/bin/env python3
"""Summarizes a veraPDF XML report: each failed rule, how many files fail it, and an example.

Usage: pdfua_summary.py report.xml [what the files are checked against, default PDF/UA-1]"""
import sys
import xml.etree.ElementTree as ET

root = ET.parse(sys.argv[1]).getroot()
rules, passed, failed = {}, 0, 0
for job in root.iter("job"):
    item = job.find("item")
    name = item.findtext("name", "?").rsplit("/", 1)[-1] if item is not None else "?"
    report = job.find("validationReport")
    if report is None:
        print(f"{name}: not validated")
        failed += 1
        continue
    if report.get("isCompliant") == "true":
        passed += 1
        continue
    failed += 1
    for rule in report.iter("rule"):
        if rule.get("status") != "failed":
            continue
        key = f"{rule.get('clause')}-{rule.get('testNumber')}"
        entry = rules.setdefault(key, {"description": rule.findtext("description", "").strip(), "files": [], "example": ""})
        entry["files"].append(name)
        check = rule.find("check")
        if check is not None and not entry["example"]:
            entry["example"] = (check.findtext("context", "") + " | " + (check.findtext("errorMessage", "") or "")).strip()
for key, e in sorted(rules.items(), key=lambda kv: -len(kv[1]["files"])):
    print(f"{key} ({len(e['files'])} files): {e['description']}")
    print(f"    e.g. {e['files'][0]}: {e['example'][:300]}")
standard = sys.argv[2] if len(sys.argv) > 2 else "PDF/UA-1"
print(f"\n{passed} of {passed + failed} PDFs comply with {standard}")
sys.exit(1 if failed else 0)
