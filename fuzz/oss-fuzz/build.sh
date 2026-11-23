#!/bin/bash -eu
# OSS-Fuzz build: every target in fuzz/, with its seed corpus from the fixtures.
cd $SRC/ferroreport
cargo fuzz build -O --debug-assertions
for target in template expression rows query; do
  cp fuzz/target/x86_64-unknown-linux-gnu/release/$target $OUT/
done
zip -j $OUT/template_seed_corpus.zip fixtures/*/*.jrxml examples/*.ferro
zip -j $OUT/rows_seed_corpus.zip fixtures/*/data.json
