#!/usr/bin/env bash
# Rebuilds the AppImage from Tauri's AppDir without two things:
# - The bundled Wayland libraries. Those are older than the host's Mesa and make
#   WebKitGTK abort with "Could not create default EGL display" on current
#   distributions; every desktop that can run the app already has libwayland.
# - GIO's TLS module. GTK loads every GIO module at startup, and this one's
#   bundled crypto libraries can crash the loader (libleancrypto after
#   linuxdeploy rewrites its runpath, on CachyOS). The app doesn't need it: the
#   window only loads bundled pages, links open in the browser, and LocalSend
#   has its own TLS.
#
# Usage: repack-appimage.sh <path/to/rquickshare.AppDir> <output.AppImage>
set -euo pipefail

appdir=$1
output=$2
tool=${APPIMAGETOOL:-$(dirname "$0")/appimagetool-x86_64.AppImage}

rm -f "$appdir"/usr/lib/libwayland-*.so* "$appdir"/usr/lib/gio/modules/libgiognutls.so

if [ ! -x "$tool" ]; then
  curl -fsSL -o "$tool" \
    https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x "$tool"
fi

ARCH=x86_64 "$tool" --no-appstream "$appdir" "$output"
