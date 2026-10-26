#!/usr/bin/env bash
# Builds dist/ Skinvas .deb, .rpm and .AppImage on this machine.
# Needs: cargo-deb, cargo-generate-rpm (cargo install ...), network for linuxdeploy on first run.
set -euo pipefail
cd "$(dirname "$0")/.."
VER=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
mkdir -p dist
cargo build --release

cargo deb --no-build --no-strip -o "dist/skinvas_${VER}_amd64.deb"
cargo generate-rpm -o "dist/skinvas-${VER}-1.x86_64.rpm"

# AppImage: linuxdeploy + its GTK plugin bundle GTK4/libadwaita; glibc stays on the host.
TOOLS="${XDG_CACHE_HOME:-$HOME/.cache}/skinvas-build"
mkdir -p "$TOOLS"
fetch() { [ -s "$TOOLS/$1" ] || curl -fsSL -o "$TOOLS/$1" "$2"; chmod +x "$TOOLS/$1"; }
fetch linuxdeploy-x86_64.AppImage https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
fetch linuxdeploy-plugin-gtk.sh https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh
# GTK >= 4.20 no longer ships a gtk-4.0 modules dir; the plugin would abort copying it.
sed -i 's|^\(\s*\)copy_lib_tree "\$gtk4_libdir" "\$APPDIR/"$|\1if [ -d "$gtk4_libdir" ]; then copy_lib_tree "$gtk4_libdir" "$APPDIR/"; fi|' "$TOOLS/linuxdeploy-plugin-gtk.sh"
# Its hook forces GTK_THEME (breaks libadwaita styling) and X11 (GTK4 is fine on Wayland): drop both.
sed -i -e '/^export GTK_THEME="\$APPIMAGE_GTK_THEME"/d' -e '/^export GDK_BACKEND=x11/d' "$TOOLS/linuxdeploy-plugin-gtk.sh"

APPDIR=$(mktemp -d)/AppDir
mkdir -p "$APPDIR/usr/share/icons"
# Symbolic Adwaita icons (the UI uses them on any desktop); cursors are left to the host.
mkdir -p "$APPDIR/usr/share/icons/Adwaita"
cp -r /usr/share/icons/Adwaita/{index.theme,symbolic,scalable,16x16} "$APPDIR/usr/share/icons/Adwaita/"
export DEPLOY_GTK_VERSION=4 APPIMAGE_EXTRACT_AND_RUN=1 NO_STRIP=1 ARCH=x86_64 LDAI_OUTPUT="dist/Skinvas-${VER}-x86_64.AppImage"
PATH="$TOOLS:$PATH" "$TOOLS/linuxdeploy-x86_64.AppImage" --appdir "$APPDIR" \
    -e target/release/skinvas \
    -d packaging/io.github.skinvas.desktop \
    -i data/icons/hicolor/256x256/apps/io.github.skinvas.png \
    --plugin gtk --output appimage
ls -lh dist/
