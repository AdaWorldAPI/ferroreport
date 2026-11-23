#!/usr/bin/env python3
"""Many reports at once: JasperReports in a warm JVM and ferroreport, each inside one process
with 1, 2, 10, 50 and 100 threads rendering the same report to PDF for a few seconds.

    cargo build --release --examples
    JASPER_LIB=/path/to/jars python3 tools/bench_concurrent.py

For each engine and number of threads: reports per second, the median and 95th percentile time
of one report, and the peak memory of the process. Writes docs/benchmarks-concurrent.json.
Both engines get the template ready (compiled, parsed) and the rows already converted before
the clock starts, and JasperReports gets 300 renders to warm up first.
"""
import argparse
import json
import os
import platform
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LAUNCHER = ROOT / "target" / "maxrss"


def measured(cmd):
    """(last line of output as JSON, peak MiB) of a command."""
    with tempfile.NamedTemporaryFile() as f:
        p = subprocess.run([str(LAUNCHER), f.name, *map(str, cmd)], capture_output=True, text=True)
        if p.returncode != 0:
            raise SystemExit(p.stdout + p.stderr)
        _, kib = Path(f.name).read_text().split()
    line = [l for l in p.stdout.splitlines() if l.startswith("{")][-1]
    return {**json.loads(line), "peak_mib": round(int(kib) / 1024, 1)}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lib", default=os.environ.get("JASPER_LIB"))
    ap.add_argument("--seconds", type=float, default=8)
    ap.add_argument("--fixtures", nargs="*", default=["02-invoice", "03-grouped-sales"])
    ap.add_argument("--threads", nargs="*", type=int, default=[1, 2, 10, 50, 100])
    args = ap.parse_args()
    if not args.lib:
        raise SystemExit("set JASPER_LIB to a folder with the JasperReports jars")
    subprocess.run(["cc", "-O2", "-o", str(LAUNCHER), str(ROOT / "tools" / "maxrss.c")], check=True)
    classes = ROOT / "target" / "oracle"
    classes.mkdir(parents=True, exist_ok=True)
    subprocess.run(["javac", "-proc:none", "-nowarn", "-cp", f"{args.lib}/*", "-d", str(classes),
                    str(ROOT / "tools" / "oracle" / "Oracle.java"), str(ROOT / "tools" / "oracle" / "Concurrent.java")], check=True)
    example = ROOT / "target" / "release" / "examples" / "concurrent"
    version = subprocess.run([str(ROOT / "target" / "release" / "ferroreport"), "--version"], capture_output=True, text=True).stdout.strip()
    jar = next(Path(args.lib).glob("jasperreports-[0-9]*.jar"), None)
    out = {"ferroreport": version, "jasperreports": jar.stem.split("-", 1)[1] if jar else "unknown",
           "machine": f"{os.cpu_count()} CPUs, {platform.system()} {platform.machine()}", "seconds": args.seconds, "reports": {}}
    for name in args.fixtures:
        fx = ROOT / "fixtures" / name
        data, params = fx / "data.json", fx / "params.json"
        rows = []
        for threads in args.threads:
            jasper = measured(["java", "-Djava.awt.headless=true", "-cp", f"{classes}:{args.lib}/*", "Concurrent", fx / "report.jrxml",
                               data if data.exists() else "-", params if params.exists() else "-", threads, args.seconds])
            ferro = measured([example, fx, threads, args.seconds])
            rows.append({"threads": threads, "jasper": jasper, "ferroreport": ferro})
            print(f"{name:18} {threads:4} threads  jasper {jasper['per_second']:8.1f}/s p95 {jasper['p95_ms']:7.1f} ms {jasper['peak_mib']:6.0f} MiB"
                  f"   ferroreport {ferro['per_second']:8.1f}/s p95 {ferro['p95_ms']:7.1f} ms {ferro['peak_mib']:6.0f} MiB", flush=True)
        out["reports"][name] = rows
    (ROOT / "docs" / "benchmarks-concurrent.json").write_text(json.dumps(out, indent=1) + "\n")


if __name__ == "__main__":
    main()
