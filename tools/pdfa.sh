#!/usr/bin/env bash
# Renders every fixture as PDF/A and checks it with veraPDF: as PDF/A-2a (tagged, the strictest
# level) and as PDF/A-3b with a file attached (the fixture's rows, or its template).
#
#   cargo build --release
#   tools/pdfa.sh                  # needs Docker (image verapdf/cli)
#
# Writes target/pdfa/{2a,3b}/<fixture>.pdf and a report for each; prints each rule that fails
# and in which files; exit status 1 when a file fails.
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
bin=${FERROREPORT_BIN:-$root/target/release/ferroreport}
out=$root/target/pdfa
rm -rf "$out" && mkdir -p "$out/2a" "$out/3b"
for dir in "$root"/fixtures/*/; do
  name=$(basename "$dir")
  args=(-q --now 2024-01-15T10:30:00)
  [ -f "$dir/data.json" ] && args+=(-d "$dir/data.json")
  [ -f "$dir/params.json" ] && args+=(-p "$dir/params.json")
  attach="$dir/report.jrxml:text/xml"
  [ -f "$dir/data.json" ] && attach="$dir/data.json"
  "$bin" render "$dir/report.jrxml" "${args[@]}" --pdfa 2a -o "$out/2a/$name.pdf" 2>/dev/null || echo "$name: not rendered as PDF/A-2a"
  "$bin" render "$dir/report.jrxml" "${args[@]}" --pdfa 3b --attach "$attach" -o "$out/3b/$name.pdf" 2>/dev/null || echo "$name: not rendered as PDF/A-3b"
done
status=0
for flavour in 2a 3b; do
  docker run --rm -v "$out/$flavour":/data verapdf/cli:latest --flavour "$flavour" --format xml --maxfailuresdisplayed 3 /data > "$out/$flavour.xml" 2>/dev/null
  python3 "$root/tools/pdfua_summary.py" "$out/$flavour.xml" "PDF/A-$flavour" || status=1
done
exit $status
