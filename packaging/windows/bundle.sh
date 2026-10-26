#!/usr/bin/env bash
# Runs inside MSYS2 UCRT64 after `cargo build --release`: makes dist/Skinvas (exe + GTK runtime) and a portable zip.
set -euo pipefail
VER=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT=dist/Skinvas
rm -rf "$OUT" && mkdir -p "$OUT/bin" "$OUT/lib" "$OUT/share/glib-2.0/schemas" "$OUT/share/icons/hicolor/256x256/apps"
cp target/release/skinvas.exe "$OUT/bin/"

# Every DLL the exe (and the image loaders) pull in from UCRT64; system DLLs stay out.
deps() { ldd "$@" | awk '$3 ~ /ucrt64/ {print $3}' | sort -u; }
cp -r /ucrt64/lib/gdk-pixbuf-2.0 "$OUT/lib/"
deps "$OUT/bin/skinvas.exe" "$OUT"/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.dll | xargs -I{} cp -n {} "$OUT/bin/"

cp /ucrt64/share/glib-2.0/schemas/gschemas.compiled "$OUT/share/glib-2.0/schemas/"
# Adwaita symbolic icons drive the whole UI; Windows draws its own cursors.
mkdir -p "$OUT/share/icons/Adwaita"
for d in /ucrt64/share/icons/Adwaita/*; do [ "$(basename "$d")" = cursors ] || cp -r "$d" "$OUT/share/icons/Adwaita/"; done
cp /ucrt64/share/icons/hicolor/index.theme "$OUT/share/icons/hicolor/"
cp data/icons/hicolor/256x256/apps/io.github.skinvas.png "$OUT/share/icons/hicolor/256x256/apps/"
magick data/icons/hicolor/256x256/apps/io.github.skinvas.png -define icon:auto-resize=256,128,64,48,32,16 "$OUT/skinvas.ico"
cp LICENSE "$OUT/LICENSE.txt"

(cd dist && rm -f "Skinvas-$VER-windows-portable.zip" && zip -qr "Skinvas-$VER-windows-portable.zip" Skinvas)
ls -lh dist
