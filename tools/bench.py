#!/usr/bin/env python3
"""Benchmarks ferroreport against JasperReports on a fixture.

Measures, per engine, the median wall time and the peak resident memory of a full
command-line render (process start to PDF written). For Jasper it also reports the time
spent inside fill + export as printed by the reference runner, i.e. excluding JVM start-up
and JRXML compilation, which a long-running server would amortise.

Usage: tools/bench.py [fixture] [--runs N]
"""

import argparse
import os
import re
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def measure(cmd):
    t0 = time.perf_counter()
    p = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    _, status, usage = os.wait4(p.pid, 0)
    wall = time.perf_counter() - t0
    err = p.stderr.read()
    if os.waitstatus_to_exitcode(status) != 0:
        raise SystemExit(f"command failed: {' '.join(cmd)}\n{err}")
    return wall, usage.ru_maxrss / 1024.0, err  # ru_maxrss is in KiB on Linux


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("fixture", nargs="?", default="09-large")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--oracle", default=str(ROOT / "tools" / "oracle" / "run.sh"))
    ap.add_argument("--bin", default=str(ROOT / "target" / "release" / "ferroreport"))
    args = ap.parse_args()

    fx = ROOT / "fixtures" / args.fixture
    data = fx / "data.json"
    params = fx / "params.json"
    out = ROOT / "target" / "bench"
    out.mkdir(parents=True, exist_ok=True)
    engines = {
        "JasperReports 6.20": [args.oracle, str(fx / "report.jrxml"), str(data) if data.exists() else "-", str(out / "jasper.pdf")]
        + ([str(params)] if params.exists() else []),
        "ferroreport": [args.bin, "render", str(fx / "report.jrxml"), "-o", str(out / "ferroreport.pdf"), "-q"]
        + (["-d", str(data)] if data.exists() else [])
        + (["-p", str(params)] if params.exists() else []),
    }
    print(f"fixture {args.fixture}, {args.runs} runs each\n")
    results = {}
    for name, cmd in engines.items():
        walls, mems, inner = [], [], []
        for _ in range(args.runs):
            wall, mem, err = measure(cmd)
            walls.append(wall)
            mems.append(mem)
            m = re.search(r"fill_ms=([0-9.]+) export_ms=([0-9.]+)", err)
            if m:
                inner.append((float(m.group(1)) + float(m.group(2))) / 1000)
        results[name] = (statistics.median(walls), max(mems), statistics.median(inner) if inner else None)
        line = f"{name:<20} wall {results[name][0]:7.3f} s   peak RSS {results[name][1]:7.1f} MiB"
        if inner:
            line += f"   (fill+export only: {results[name][2]:.3f} s)"
        print(line)
    (jw, jm, ji), (fw, fm, _) = results.values()
    print(f"\nferroreport is {jw / fw:.1f}x faster end to end, {ji / fw:.1f}x faster than Jasper's fill+export alone,")
    print(f"and uses {jm / fm:.1f}x less memory.")


if __name__ == "__main__":
    sys.exit(main())
