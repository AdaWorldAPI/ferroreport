#!/usr/bin/env python3
"""Checks what the HTTP server promises about large reports, and measures it.

    cargo build --release
    python3 tools/http_check.py                 # everything (a few minutes)
    python3 tools/http_check.py --rows 100000   # a quicker run

What it checks:

  same documents     every fixture over HTTP, in the four ways a request can be made (rows in
                     the JSON body or one per line; the answer whole or as it renders), gives
                     the bytes the command line gives, for PDF, CSV and Excel
  a million rows     the rows are never all in memory: the server's peak memory is measured
  many clients       50 at once: memory and disk stay bounded, and are given back
  a client leaves    half way through a report: the render stops and nothing is left behind
  a row is wrong     the whole-document answer is an error and not one byte of document; the
                     as-it-renders answer ends in a way no client takes for a complete document
  answers on time    a client offering HTTP/2 gets its answer; a wrong template sent with many
                     rows gets its error every time, not a reset connection
  speed              POST /render for small reports, to compare with another build (--against)

Exit status 0 when every check passes. Linux only (reads /proc for memory).
"""
import argparse
import hashlib
import http.client
import itertools
import json
import os
import pathlib
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
NOW = "2026-01-15T10:30:00"
TOKEN = "check"
NDJSON = "application/x-ndjson"
STREAM = {"Prefer": "respond-streaming"}


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Server:
    def __init__(self, binary, *args, env=None, spool_dir=True):
        self.port = free_port()
        self.spool = pathlib.Path(tempfile.mkdtemp(prefix="ferroreport-check-"))
        cmd = [str(binary), "serve", "--port", str(self.port), "--templates", str(ROOT / "fixtures"), "--token", TOKEN,
               "--timeout", "600", *args]
        if spool_dir and self.takes_spool(binary):
            cmd += ["--spool-dir", str(self.spool)]
        self.proc = subprocess.Popen(cmd, env={**os.environ, **(env or {})}, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        for _ in range(200):
            try:
                if self.request("GET", "/health")[0] == 200:
                    return
            except OSError:
                time.sleep(0.02)
        raise SystemExit("the server did not start: " + self.proc.stderr.read().decode())

    @staticmethod
    def takes_spool(binary):
        return "--spool-dir" in subprocess.run([str(binary), "--help"], capture_output=True, text=True).stdout

    def connection(self, timeout=900):
        return http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)

    def request(self, method, path, body=None, headers=None, conn=None):
        """Returns (status, headers, body). `body` may be bytes or an iterable of bytes (sent in chunks)."""
        c = conn or self.connection()
        try:
            c.request(method, path, body=body, headers={"Authorization": f"Bearer {TOKEN}", **(headers or {})},
                      encode_chunked=body is not None and not isinstance(body, (bytes, bytearray)))
            r = c.getresponse()
            return r.status, dict(r.getheaders()), r.read()
        finally:
            if conn is None:
                c.close()

    def metrics(self):
        text = self.request("GET", "/metrics")[2].decode()
        return {k: float(v) for k, v in (line.split() for line in text.splitlines() if line and not line.startswith("#"))}

    def memory(self):
        """(now, peak) resident memory of the server, in MiB."""
        status = pathlib.Path(f"/proc/{self.proc.pid}/status").read_text()
        kib = {k: int(v.split()[0]) for k, v in (line.split(":") for line in status.splitlines()) if k in ("VmRSS", "VmHWM")}
        return kib["VmRSS"] / 1024, kib["VmHWM"] / 1024

    def spool_files(self):
        return sum(1 for _ in self.spool.iterdir())

    def settle(self, seconds=20):
        """Waits until nothing is rendering; returns the metrics then."""
        end = time.time() + seconds
        while True:
            m = self.metrics()
            if m["ferroreport_renders_in_flight"] == 0 or time.time() > end:
                return m
            time.sleep(0.05)

    def stop(self):
        self.proc.terminate()
        self.proc.wait()
        shutil.rmtree(self.spool, ignore_errors=True)


def fixture_request(fx):
    """(name for /render/<name>, head of the request, rows) for a fixture."""
    head = {"now": NOW}
    if (fx / "params.json").exists():
        head["params"] = json.loads((fx / "params.json").read_text())
    rows = json.loads((fx / "data.json").read_text()) if (fx / "data.json").exists() else []
    return f"{fx.name}/report", head, rows


def lines(head, rows):
    return (json.dumps(head) + "\n" + "".join(json.dumps(r) + "\n" for r in rows)).encode()


def many_rows(n):
    """A request for fixtures/09-large with `n` rows, as an iterable of pieces of about 1 MB."""
    src = [json.dumps(r) + "\n" for r in json.loads((ROOT / "fixtures" / "09-large" / "data.json").read_text())]

    def pieces(bad_at=None):
        yield (json.dumps({"now": NOW}) + "\n").encode()
        block = []
        for i in range(n):
            block.append("{this is not a row\n" if i == bad_at else src[i % len(src)])
            if len(block) == 10_000:
                yield "".join(block).encode()
                block = []
        if block:
            yield "".join(block).encode()

    return pieces


class Checks:
    def __init__(self):
        self.rows = []
        self.failed = 0

    def add(self, name, ok, detail):
        self.rows.append({"check": name, "ok": bool(ok), "detail": detail})
        self.failed += not ok
        print(f"{'ok  ' if ok else 'FAIL'}  {name}: {detail}", flush=True)


def same_documents(binary, server, checks, tmp):
    wrong, count = [], 0
    for fx in sorted(p for p in (ROOT / "fixtures").iterdir() if (p / "report.jrxml").exists()):
        name, head, rows = fixture_request(fx)
        for fmt in ("pdf", "csv", "xlsx"):
            out = tmp / f"cli.{fmt}"
            cmd = [str(binary), "render", str(fx / "report.jrxml"), "-o", str(out), "-q", "--now", NOW]
            if (fx / "data.json").exists():
                cmd += ["-d", str(fx / "data.json")]
            if (fx / "params.json").exists():
                cmd += ["-p", str(fx / "params.json")]
            if subprocess.run(cmd, capture_output=True).returncode != 0:
                continue
            expected = out.read_bytes()
            whole = json.dumps({**head, "format": fmt, "data": rows}).encode()
            by_line = lines({**head, "format": fmt}, rows)
            ways = {
                "json": (whole, {"Content-Type": "application/json"}),
                "lines": (by_line, {"Content-Type": NDJSON}),
                "json, as it renders": (whole, {"Content-Type": "application/json", **STREAM}),
                "lines, as it renders": (by_line, {"Content-Type": NDJSON, **STREAM}),
            }
            for way, (body, headers) in ways.items():
                count += 1
                status, got_headers, got = server.request("POST", f"/render/{name}", body, headers)
                framed = "Transfer-Encoding" in got_headers if "renders" in way else got_headers.get("Content-Length") == str(len(got))
                if status != 200 or got != expected or not framed:
                    wrong.append(f"{fx.name} {fmt} ({way}): {status}")
    checks.add("same documents", not wrong, f"{count} requests, all equal to the command line's output" if not wrong else "; ".join(wrong[:5]))


def a_million_rows(binary, checks, n, results):
    make = many_rows(n)
    # Each on a server of its own, with the default settings, so that the peak is its own.
    for way, headers in (("whole", {}), ("as it renders", STREAM)):
        server = Server(binary)
        started = time.time()
        conn = server.connection()
        conn.request("POST", "/render/09-large/report", body=make(), headers={"Authorization": f"Bearer {TOKEN}", "Content-Type": NDJSON, **headers},
                     encode_chunked=True)
        r = conn.getresponse()
        size, first, digest = 0, None, hashlib.sha256()
        while True:
            piece = r.read(1 << 20)
            if not piece:
                break
            first = first or time.time() - started
            size += len(piece)
            digest.update(piece)
        conn.close()
        seconds = time.time() - started
        m = server.settle()
        peak = server.memory()[1]
        files = server.spool_files()
        server.stop()
        results[f"rows_{way.replace(' ', '_')}"] = {"rows": n, "seconds": round(seconds, 2), "first_byte_seconds": round(first, 2),
                                                     "peak_mib": round(peak, 1), "bytes": size, "sha256": digest.hexdigest()}
        ok = r.status == 200 and size > 0 and peak < 100 and m["ferroreport_spool_disk_bytes"] == 0 and files == 0
        checks.add(f"{n:,} rows, answer {way}", ok,
                   f"{seconds:.1f} s, first byte at {first:.1f} s, {size / 1e6:.0f} MB of PDF, server peak {peak:.0f} MiB")
    same = results["rows_whole"]["sha256"] == results["rows_as_it_renders"]["sha256"]
    checks.add("both answers are the same document", same, "same bytes" if same else "different bytes")


def many_clients(server, checks, clients, rows, results, key, note):
    body = b"".join(many_rows(rows)())
    statuses, sizes = [], set()
    peak_disk = [0]

    def one(streaming):
        status, _, got = server.request("POST", "/render/09-large/report", body, {"Content-Type": NDJSON, **(STREAM if streaming else {})})
        statuses.append(status)
        sizes.add(len(got))

    def watch(stop):
        while not stop.is_set():
            peak_disk[0] = max(peak_disk[0], server.metrics()["ferroreport_spool_disk_bytes"])
            time.sleep(0.05)

    stop = threading.Event()
    watcher = threading.Thread(target=watch, args=(stop,))
    watcher.start()
    started = time.time()
    threads = [threading.Thread(target=one, args=(i % 2 == 1,)) for i in range(clients)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    seconds = time.time() - started
    stop.set()
    watcher.join()
    m = server.settle()
    now, peak = server.memory()
    results[key] = {"clients": clients, "rows_each": rows, "seconds": round(seconds, 2), "peak_mib": round(peak, 1),
                          "peak_disk_mib": round(peak_disk[0] / 2**20, 1)}
    ok = statuses == [200] * clients and len(sizes) == 1 and m["ferroreport_spool_disk_bytes"] == 0 and server.spool_files() == 0
    checks.add(f"{clients} clients at once, {note} ({rows:,} rows each, half answered as it renders)", ok,
               f"{seconds:.1f} s, server peak {peak:.0f} MiB, at most {peak_disk[0] / 2**20:.0f} MiB on disk, none left after")


def a_client_leaves(server, checks, n):
    before = server.metrics()
    body = b"".join(many_rows(n)())
    sock = socket.create_connection(("127.0.0.1", server.port))
    sock.sendall((f"POST /render/09-large/report HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\n"
                  f"Content-Type: {NDJSON}\r\nPrefer: respond-streaming\r\nContent-Length: {len(body)}\r\n\r\n").encode() + body)
    got = b""
    while len(got) < (1 << 20):
        got += sock.recv(1 << 16)
    left = time.time()
    # Closes the connection with the rest of the document unread.
    sock.close()
    while time.time() - left < 30:
        m = server.metrics()
        if m["ferroreport_cancelled_total"] > before["ferroreport_cancelled_total"] and m["ferroreport_renders_in_flight"] == 0:
            break
        time.sleep(0.01)
    took = time.time() - left
    m = server.metrics()
    ok = (got.startswith(b"HTTP/1.1 200") and m["ferroreport_cancelled_total"] == before["ferroreport_cancelled_total"] + 1
          and m["ferroreport_renders_in_flight"] == 0 and m["ferroreport_spool_disk_bytes"] == 0 and server.spool_files() == 0)
    checks.add("a client leaves half way", ok, f"the render stopped {took * 1000:.0f} ms later, with nothing left on disk")


def a_row_is_wrong(server, checks, n):
    make = many_rows(n)
    status, headers, got = server.request("POST", "/render/09-large/report", make(bad_at=n - 10), {"Content-Type": NDJSON})
    ok = status == 422 and b"%PDF" not in got and "error" in json.loads(got)
    checks.add("a wrong row, answer whole", ok, f"HTTP {status} {got.decode(errors="replace")[:90]}")
    try:
        status, headers, got = server.request("POST", "/render/09-large/report", make(bad_at=n - 10), {"Content-Type": NDJSON, **STREAM})
        checks.add("a wrong row, answer as it renders", False, f"the client took {len(got)} bytes for a complete document")
    except (http.client.HTTPException, ValueError, OSError) as e:
        checks.add("a wrong row, answer as it renders", True, f"the client reports an incomplete response ({type(e).__name__})")
    # An error before the first byte is still an ordinary error.
    status, _, got = server.request("POST", "/render/09-large/report", make(bad_at=0), {"Content-Type": NDJSON, **STREAM})
    checks.add("a wrong first row, answer as it renders", status == 422, f"HTTP {status} {got.decode(errors="replace")[:90]}")
    m = server.settle()
    checks.add("nothing left behind after errors", m["ferroreport_spool_disk_bytes"] == 0 and server.spool_files() == 0, "no disk in use")


def no_room(binary, checks):
    """Without a directory, or over the disk budget, a large document is refused and small ones still work."""
    body = b"".join(many_rows(20_000)())
    _, head, rows = fixture_request(ROOT / "fixtures" / "02-invoice")
    small = json.dumps({**head, "data": rows}).encode()
    # No --spool-dir, and a temporary directory that does not exist: nowhere to write.
    s = Server(binary, "--spool-memory", "1", env={"TMPDIR": "/nonexistent/ferroreport"}, spool_dir=False)
    try:
        status, _, got = s.request("POST", "/render/09-large/report", body, {"Content-Type": NDJSON})
        ok = s.request("POST", "/render/02-invoice/report", small, {"Content-Type": "application/json"})[0]
        checks.add("no directory to write to", status == 507 and ok == 200, f"large: HTTP {status} {got.decode(errors="replace")[:60]}...; small reports: HTTP {ok}")
    finally:
        s.stop()
    s = Server(binary, "--spool-memory", "1", "--spool-disk", "1")
    try:
        status, _, got = s.request("POST", "/render/09-large/report", body, {"Content-Type": NDJSON})
        m = s.settle()
        checks.add("over the disk budget", status == 507 and m["ferroreport_spool_disk_bytes"] == 0 and s.spool_files() == 0,
                   f"HTTP {status} {got.decode(errors="replace")[:60]}...; the space was given back")
    finally:
        s.stop()


def speed(server, runs=400):
    """Median milliseconds of POST /render over one connection, per small fixture."""
    out = {}
    conn = server.connection()
    for name in ("02-invoice", "01-employee-list", "03-grouped-sales"):
        _, head, rows = fixture_request(ROOT / "fixtures" / name)
        template = (ROOT / "fixtures" / name / "report.jrxml").read_text()
        body = json.dumps({**head, "template": template, "data": rows}).encode()
        times = []
        for i in range(runs + 50):
            started = time.perf_counter()
            status, _, got = server.request("POST", "/render", body, {"Content-Type": "application/json"}, conn=conn)
            assert status == 200 and got[:4] == b"%PDF", status
            if i >= 50:
                times.append((time.perf_counter() - started) * 1e3)
        out[name] = round(statistics.median(times), 3)
    conn.close()
    return out


def answers_on_time(server, checks):
    """A client offering HTTP/2 (Java's HttpClient does on its first request) keeps its connection
    open after its body: it gets its answer all the same. A wrong template sent with many rows is
    answered with its error, the rows read and discarded rather than left to reset the connection."""
    body = json.dumps({"template": '<jasperReport name="t"/>'}).encode()
    sock = socket.create_connection(("127.0.0.1", server.port), timeout=10)
    try:
        sock.sendall((f"POST /inspect HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade, HTTP2-Settings\r\nUpgrade: h2c\r\n"
                      f"HTTP2-Settings: AAEAAEAAAAIAAAABAAMAAABkAAQBAAAAAAUAAEAA\r\nAuthorization: Bearer {TOKEN}\r\n"
                      f"Content-Length: {len(body)}\r\n\r\n").encode() + body)
        got = sock.recv(64)
    except OSError as e:
        got = repr(e).encode()
    finally:
        sock.close()
    checks.add("a client that offers HTTP/2 is answered", got.startswith(b"HTTP/1.1 200"), got.decode(errors="replace").split("\r\n")[0])
    answered = 0
    for _ in range(10):
        rows = many_rows(50_000)()
        head = json.dumps({"template": "<jasperReport"}).encode() + b"\n"
        status, _, got = server.request("POST", "/render", itertools.chain([head], rows), {"Content-Type": NDJSON})
        answered += status == 422 and b"XML" in got
    checks.add("a wrong template sent with rows is answered", answered == 10, f"{answered} of 10 times with its error")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", default=str(ROOT / "target" / "release" / "ferroreport"))
    ap.add_argument("--rows", type=int, default=1_000_000)
    ap.add_argument("--clients", type=int, default=50)
    ap.add_argument("--against", help="another build of ferroreport, to compare the speed of POST /render with")
    ap.add_argument("--json", help="write the measurements to this file")
    args = ap.parse_args()
    binary = pathlib.Path(args.bin)
    checks, results = Checks(), {}
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="ferroreport-check-out-"))
    # Documents go to disk from the first megabyte, so that the disk path is the one checked.
    server = Server(binary, "--spool-memory", "1")
    try:
        same_documents(binary, server, checks, tmp)
        a_row_is_wrong(server, checks, 20_000)
        answers_on_time(server, checks)
        a_client_leaves(server, checks, max(args.rows // 5, 50_000))
    finally:
        server.stop()
    no_room(binary, checks)
    a_million_rows(binary, checks, args.rows, results)
    for key, note, flags in (("clients", "default settings", ()), ("clients_on_disk", "documents kept on disk", ("--spool-memory", "0"))):
        server = Server(binary, *flags)
        try:
            many_clients(server, checks, args.clients, 20_000, results, key, note)
        finally:
            server.stop()
    server = Server(binary)
    try:
        results["post_render_ms"] = speed(server)
    finally:
        server.stop()
    line = ", ".join(f"{k} {v} ms" for k, v in results["post_render_ms"].items())
    if args.against:
        other = Server(pathlib.Path(args.against))
        try:
            results["post_render_ms_against"] = speed(other)
        finally:
            other.stop()
        worst = max(results["post_render_ms"][k] / results["post_render_ms_against"][k] for k in results["post_render_ms"])
        checks.add("POST /render is as fast as before", worst < 1.10,
                   f"{line}; the other build: " + ", ".join(f"{k} {v} ms" for k, v in results["post_render_ms_against"].items()))
    else:
        print(f"      POST /render: {line}")
    shutil.rmtree(tmp, ignore_errors=True)
    results["checks"] = checks.rows
    if args.json:
        pathlib.Path(args.json).write_text(json.dumps(results, indent=2) + "\n")
    print(f"\n{len(checks.rows) - checks.failed} of {len(checks.rows)} checks passed")
    sys.exit(1 if checks.failed else 0)


if __name__ == "__main__":
    main()
