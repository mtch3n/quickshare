#!/usr/bin/env bash
# Generates the SVG sources (src/gen.py) and renders the PNG icons from them.
# Requires python3 and rsvg-convert.
set -euo pipefail
cd "$(dirname "$0")"

python3 src/gen.py
for size in 16 24 32; do
  rsvg-convert -w "$size" -h "$size" src/rquickshare-small.svg -o "${size}x${size}.png"
done
for size in 48 64 128 256 512; do
  rsvg-convert -w "$size" -h "$size" src/rquickshare.svg -o "${size}x${size}.png"
done
cp 512x512.png icon.png
cp src/rquickshare.svg ../../public/icon.svg
