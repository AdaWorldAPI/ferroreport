#!/bin/sh
# Renders a JRXML with the reference JasperReports engine.
# Usage: run.sh <report.jrxml> <data.json|-> <out.pdf> [params.json]
# ORACLE_JAVA picks the java to run: a Java 17 runtime for Groovy templates (the Groovy 4.0.8
# of JasperReports 6.20.6 cannot compile on Java 21).
# Requires JASPER_LIB: a directory containing jasperreports-6.x.jar and its dependencies
# (including jasperreports-fonts for the DejaVu families).
set -e
DIR=$(cd "$(dirname "$0")" && pwd)
LIB=${JASPER_LIB:?set JASPER_LIB to a directory with the JasperReports jars}
OUT="$DIR/../../target/oracle"
if [ ! -f "$OUT/Oracle.class" ] || [ "$DIR/Oracle.java" -nt "$OUT/Oracle.class" ]; then
  mkdir -p "$OUT"
  javac --release 17 -proc:none -nowarn -cp "$LIB/*" -d "$OUT" "$DIR/Oracle.java" 2>&1 | grep -v "Picked up" || true
fi
# The Java runtime's logical fonts (SansSerif...) as on a stock Linux server: DejaVu.
export FONTCONFIG_FILE="$DIR/fonts.conf"
exec "${ORACLE_JAVA:-java}" -Djava.awt.headless=true -Duser.home="$OUT/home" -cp "$OUT:$LIB/*" Oracle "$@" 2>&1 | grep -v "Picked up\|^WARNING\|log4j\|SLF4J" >&2
