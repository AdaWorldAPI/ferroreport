#!/usr/bin/env bash
# Renders every fixture as PDF/A and checks it with veraPDF: as PDF/A-2a (tagged, the strictest
# level), as PDF/A-3b with a file attached (the fixture's rows, or its template), and as
# PDF/A-2b signed (PAdES B-B, with a throwaway key made by openssl).
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
rm -rf "$out" && mkdir -p "$out/2a" "$out/3b" "$out/2b"
openssl req -x509 -newkey rsa:2048 -nodes -keyout "$out/sign.key" -out "$out/sign.crt" -days 2 -subj "/CN=pdfa check" 2>/dev/null
for dir in "$root"/fixtures/*/; do
  name=$(basename "$dir")
  args=(-q --now 2024-01-15T10:30:00)
  [ -f "$dir/data.json" ] && args+=(-d "$dir/data.json")
  [ -f "$dir/params.json" ] && args+=(-p "$dir/params.json")
  # Fonts a fixture needs (fonts.txt: Family=file.ttf[:bold][:italic]), when installed.
  if [ -f "$dir/fonts.txt" ]; then
    while IFS= read -r f; do p=${f#*=}; [ -n "$f" ] && [ -f "${p%%:*}" ] && args+=(--font "$f"); done < "$dir/fonts.txt"
  fi
  attach="$dir/report.jrxml:text/xml"
  [ -f "$dir/data.json" ] && attach="$dir/data.json"
  "$bin" render "$dir/report.jrxml" "${args[@]}" --pdfa 2a -o "$out/2a/$name.pdf" 2>/dev/null || echo "$name: not rendered as PDF/A-2a"
  "$bin" render "$dir/report.jrxml" "${args[@]}" --pdfa 3b --attach "$attach" -o "$out/3b/$name.pdf" 2>/dev/null || echo "$name: not rendered as PDF/A-3b"
  "$bin" render "$dir/report.jrxml" "${args[@]}" --pdfa 2b --sign-key "$out/sign.key" --sign-cert "$out/sign.crt" -o "$out/2b/$name.pdf" 2>/dev/null || echo "$name: not rendered as signed PDF/A-2b"
done
status=0
for flavour in 2a 3b 2b; do
  docker run --rm -v "$out/$flavour":/data verapdf/cli:latest --flavour "$flavour" --format xml --maxfailuresdisplayed 3 /data > "$out/$flavour.xml" 2>/dev/null
  python3 "$root/tools/pdfua_summary.py" "$out/$flavour.xml" "PDF/A-$flavour" || status=1
done
exit $status
