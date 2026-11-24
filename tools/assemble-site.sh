#!/usr/bin/env bash
# Puts the website together in one folder, as GitHub Pages serves it: site/, the browser build
# of the engine (bindings/wasm/pkg/web, from bindings/wasm/build.sh) in pkg/, the playground's
# examples and the designer's examples. Used by .github/workflows/pages.yml, and to try the site
# locally:
#
#   bindings/wasm/build.sh && tools/assemble-site.sh _site && python3 -m http.server -d _site
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-_site}"
if [ ! -f bindings/wasm/pkg/web/ferroreport.js ]; then
  echo "bindings/wasm/pkg/web is missing: run bindings/wasm/build.sh first" >&2
  exit 1
fi
rm -rf "$OUT"
mkdir -p "$OUT"
cp -r site/. "$OUT/"
mkdir -p "$OUT/pkg" "$OUT/examples"
cp bindings/wasm/pkg/web/* "$OUT/pkg/"
# The playground: templates and data of a few fixtures.
for f in 02-invoice 18-barcodes 21-markup 26-barcodes-1d; do
  mkdir -p "$OUT/examples/$f"
  cp "fixtures/$f/report.jrxml" "fixtures/$f/data.json" "$OUT/examples/$f/"
  [ -f "fixtures/$f/params.json" ] && cp "fixtures/$f/params.json" "$OUT/examples/$f/" || true
done
# The designer: the examples it starts from.
mkdir -p "$OUT/designer/examples"
cp examples/invoices.ferro examples/parts.ferro examples/invoices.json \
   examples/employees.ferro examples/employees.json "$OUT/designer/examples/"
touch "$OUT/.nojekyll"
