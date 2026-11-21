#!/bin/sh
# Renders a JRXML with the reference JasperReports engine.
# Usage: run.sh <report.jrxml> <data.json|-> <out.pdf> [params.json]
# Requires JASPER_LIB: a directory containing jasperreports-6.x.jar and its dependencies
# (including jasperreports-fonts for the DejaVu families).
set -e
DIR=$(cd "$(dirname "$0")" && pwd)
LIB=${JASPER_LIB:?set JASPER_LIB to a directory with the JasperReports jars}
OUT="$DIR/../../target/oracle"
if [ ! -f "$OUT/Oracle.class" ] || [ "$DIR/Oracle.java" -nt "$OUT/Oracle.class" ]; then
  mkdir -p "$OUT"
  javac -proc:none -nowarn -cp "$LIB/*" -d "$OUT" "$DIR/Oracle.java" 2>&1 | grep -v "Picked up" || true
fi
exec java -Djava.awt.headless=true -cp "$OUT:$LIB/*" Oracle "$@" 2>&1 | grep -v "Picked up\|^WARNING\|log4j\|SLF4J" >&2
