#!/usr/bin/env python3
"""Every number in the README's performance section, from one command.

Three situations are measured, because they answer different questions:

  cold   a command-line render, process start to PDF on disk. What a script, a serverless
         function or a container that starts for each report pays.
  warm   renders repeated inside one process that stays up. For JasperReports: the template
         compiled once, the JIT warmed up, rows already converted to Java objects, PDF
         exported to memory. What a long-running service pays per request.
  large  100,000 and 1,000,000 rows (the 09-large fixture's rows repeated), with peak memory.

Usage: JASPER_LIB=/path/to/jars tools/bench_all.py [--quick] [--out docs/benchmarks.json]

With the jars of JasperReports 7, the fixtures are first converted to JRXML 7 (with
`ferroreport convert`), and both engines render those.
Needs a release build (`cargo build --release --example warm`), a JDK and a C compiler.
"""

import argparse
import json
import os
import platform
import statistics
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "release" / "ferroreport"
WARM = ROOT / "target" / "release" / "examples" / "warm"
CLASSES = ROOT / "target" / "oracle"
FIXTURES = ROOT / "fixtures"


def run(cmd):
    """The standard output of a command."""
    p = subprocess.run(list(map(str, cmd)), capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit(f"command failed: {' '.join(map(str, cmd))}\n{p.stderr[-2000:]}")
    return p.stdout


LAUNCHER = ROOT / "target" / "maxrss"


def measured(cmd):
    """Wall seconds and peak resident memory (MiB) of a command, run through a tiny launcher
    (tools/maxrss.c): measured from Python, a light process would report Python's memory."""
    with tempfile.NamedTemporaryFile("r", suffix=".txt") as report:
        p = subprocess.run([str(LAUNCHER), report.name, *map(str, cmd)], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        if p.returncode != 0:
            raise SystemExit(f"command failed: {' '.join(map(str, cmd))}\n{p.stderr[-2000:]}")
        seconds, kib = report.read().split()
    return float(seconds), int(kib) / 1024.0


def java(lib, *args, heap=None):
    cmd = ["java", "-Djava.awt.headless=true"]
    if heap:
        cmd.append(f"-Xmx{heap}")
    return cmd + ["-cp", f"{CLASSES}:{lib}/*", *map(str, args)]


def fixture_args(fx):
    data, params = fx / "data.json", fx / "params.json"
    return data if data.exists() else None, params if params.exists() else None


def cold(lib, name, runs, tmp):
    fx = FIXTURES / name
    data, params = fixture_args(fx)
    jasper = java(lib, "Oracle", fx / "report.jrxml", data or "-", tmp / "j.pdf", *([params] if params else []))
    ferro = [BIN, "render", fx / "report.jrxml", "-o", tmp / "f.pdf", "-q"]
    ferro += (["-d", data] if data else []) + (["-p", params] if params else [])
    out = {}
    for engine, cmd in (("jasper", jasper), ("ferroreport", ferro)):
        results = [measured(list(map(str, cmd))) for _ in range(runs)]
        out[engine] = {"seconds": statistics.median(r[0] for r in results), "mib": statistics.median(r[1] for r in results)}
    out["jasper"]["bytes"] = (tmp / "j.pdf").stat().st_size
    out["ferroreport"]["bytes"] = (tmp / "f.pdf").stat().st_size
    return out


def warm(lib, name, warmup, runs):
    fx = FIXTURES / name
    data, params = fixture_args(fx)
    # The best of three rounds of each: the machine's noise only ever makes a round slower.
    j = min(
        json.loads(run(java(lib, "Warm", fx / "report.jrxml", data or "-", params or "-", warmup, runs)).strip().splitlines()[-1])["median_ms"]
        for _ in range(3)
    )
    f = min((json.loads(run([WARM, fx, runs])) for _ in range(3)), key=lambda r: r["whole_ms"])
    return {"jasper_ms": j, "ferroreport_ms": f["whole_ms"], "ferroreport_parsed_ms": f["parsed_ms"]}


def large(lib, rows, tmp, jasper_heap=None, ferro_runs=3):
    src = json.loads((FIXTURES / "09-large" / "data.json").read_text())
    data = tmp / f"{rows}.json"
    with data.open("w") as f:
        f.write("[")
        for i in range(rows):
            f.write(("," if i else "") + json.dumps(src[i % len(src)]))
        f.write("]")
    tpl = FIXTURES / "09-large" / "report.jrxml"
    out = {"rows": rows}
    results = [measured([str(BIN), "render", str(tpl), "-d", str(data), "-o", str(tmp / "f.pdf"), "-q"]) for _ in range(ferro_runs)]
    out["ferroreport"] = {"seconds": statistics.median(r[0] for r in results), "mib": statistics.median(r[1] for r in results),
                          "bytes": (tmp / "f.pdf").stat().st_size}
    wall, mib = measured(list(map(str, java(lib, "Oracle", tpl, data, tmp / "j.pdf", heap=jasper_heap))))
    out["jasper"] = {"seconds": wall, "mib": mib, "bytes": (tmp / "j.pdf").stat().st_size, "heap": jasper_heap or "default"}
    data.unlink()
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--quick", action="store_true", help="skip the million-row run")
    ap.add_argument("--out", default=str(ROOT / "docs" / "benchmarks.json"))
    args = ap.parse_args()
    global CLASSES, FIXTURES
    lib = os.environ.get("JASPER_LIB") or sys.exit("set JASPER_LIB to a directory with the JasperReports jars")
    jar = next(Path(lib).glob("jasperreports-[0-9]*.jar"), None)
    jasper_version = jar.stem.split("-", 1)[1] if jar else "unknown"
    CLASSES = ROOT / "target" / f"oracle-bench-{jasper_version}"
    CLASSES.mkdir(parents=True, exist_ok=True)
    converted = None
    if jasper_version.startswith("7"):
        # JasperReports 7 reads JRXML 7 only: both engines render the fixtures converted to it.
        converted = tempfile.TemporaryDirectory()
        FIXTURES = Path(converted.name)
        for name in ("01-employee-list", "02-invoice", "03-grouped-sales", "09-large"):
            (FIXTURES / name).mkdir()
            for f in (ROOT / "fixtures" / name).iterdir():
                if f.suffix == ".jrxml":
                    run([BIN, "convert", f, "-o", FIXTURES / name / f.name])
                elif f.is_file():
                    (FIXTURES / name / f.name).write_bytes(f.read_bytes())
    subprocess.run(["cc", "-O2", "-o", str(LAUNCHER), str(ROOT / "tools" / "maxrss.c")], check=True)
    subprocess.run(["javac", "-proc:none", "-nowarn", "-cp", f"{lib}/*", "-d", str(CLASSES),
                    str(ROOT / "tools/oracle/Oracle.java"), str(ROOT / "tools/oracle/Warm.java")], check=True, stderr=subprocess.DEVNULL)
    version = subprocess.run([str(BIN), "--version"], capture_output=True, text=True).stdout.strip()
    jvm = subprocess.run(["java", "-version"], capture_output=True, text=True).stderr
    jvm = next((l for l in jvm.splitlines() if "version" in l and "Picked" not in l), "")
    result = {"ferroreport": version, "jasperreports": jasper_version, "java": jvm, "machine": f"{os.cpu_count()} CPUs, {platform.system()} {platform.machine()}",
              "cold": {}, "warm": {}, "large": []}
    with tempfile.TemporaryDirectory() as t:
        tmp = Path(t)
        for name, runs in (("02-invoice", 7), ("03-grouped-sales", 7), ("09-large", 5)):
            result["cold"][name] = cold(lib, name, runs, tmp)
            print("cold", name, result["cold"][name], flush=True)
        for name, warmup, runs in (("02-invoice", 500, 400), ("01-employee-list", 500, 400), ("03-grouped-sales", 500, 400), ("09-large", 4, 7)):
            result["warm"][name] = warm(lib, name, warmup, runs)
            print("warm", name, result["warm"][name], flush=True)
        for rows in (100_000,) if args.quick else (100_000, 1_000_000):
            result["large"].append(large(lib, rows, tmp))
            print("large", result["large"][-1], flush=True)
    Path(args.out).write_text(json.dumps(result, indent=1) + "\n")
    print("written", args.out)


if __name__ == "__main__":
    main()
