#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
dest="$here/fonts"
mkdir -p "$dest"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL -o "$tmp/liberation.tar.gz" \
  https://github.com/liberationfonts/liberation-fonts/files/7261482/liberation-fonts-ttf-2.1.5.tar.gz
tar -xzf "$tmp/liberation.tar.gz" -C "$tmp"
cp "$tmp"/liberation-fonts-ttf-*/Liberation{Sans,Serif,Mono}-{Regular,Bold,Italic,BoldItalic}.ttf "$dest/"
cp "$tmp"/liberation-fonts-ttf-*/LICENSE "$dest/LICENSE.liberation"

ls -la "$dest"
