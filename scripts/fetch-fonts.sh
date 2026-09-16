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

for family in carlito caladea; do
  case $family in
    carlito) name=Carlito ;;
    caladea) name=Caladea ;;
  esac
  for style in Regular Bold Italic BoldItalic; do
    curl -fsSL -o "$dest/$name-$style.ttf" \
      "https://github.com/google/fonts/raw/main/ofl/$family/$name-$style.ttf"
  done
  curl -fsSL -o "$dest/LICENSE.$family" "https://github.com/google/fonts/raw/main/ofl/$family/OFL.txt"
done

ls -la "$dest"
