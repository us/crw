#!/bin/bash -eu
cd $SRC/crw/fuzz
cargo fuzz build -O
FUZZ_TARGET_OUTPUT_DIR=target/x86_64-unknown-linux-gnu/release
for f in fuzz_targets/*.rs; do
  target=$(basename "${f%.*}")
  cp "$FUZZ_TARGET_OUTPUT_DIR/$target" "$OUT/"
done
