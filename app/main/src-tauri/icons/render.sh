#!/usr/bin/env bash
# Renders the PNG icons from the SVG sources in src/. Requires rsvg-convert.
set -euo pipefail
cd "$(dirname "$0")"

for size in 32 48 64 128 256 512; do
  rsvg-convert -w "$size" -h "$size" src/rquickshare.svg -o "${size}x${size}.png"
done
cp 512x512.png icon.png
