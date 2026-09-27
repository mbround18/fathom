#!/usr/bin/env bash
# Repack a Tauri-built AppImage without the libraries that must come from the host.
#
# Why: linuxdeploy copies libwayland-client.so.0 from the build host into the AppImage. The
# host's Mesa EGL driver links the host's libwayland-client; when the older bundled copy shadows
# it, WebKitGTK aborts at start with `Could not create default EGL display: EGL_BAD_PARAMETER`
# and the window stays blank. Reproduced on stock Ubuntu 26.04 and Fedora 44 (Xvfb, Mesa); with
# only that one file removed the app renders on both (T126, see
# specs/001-tauri-shadcn-frontend/checklists/linux-release-validation.md). Every GTK3 desktop
# ships libwayland-client (GTK3 itself links it), so leaving it to the host costs nothing.
#
# The Tauri bundler cannot pass `--exclude-library` to linuxdeploy, so run this after
# `pnpm -C ui tauri build`:
#
#   scripts/appimage-host-libs.sh target/release/bundle/appimage/Fathom_*.AppImage
#
# It reuses the AppImage's own runtime (no network) and the appimagetool that Tauri's
# linuxdeploy-plugin-appimage already cached in ~/.cache/tauri (override with APPIMAGETOOL).
# HOST_LIBS overrides the glob list of libraries to remove.
set -euo pipefail

img=$(realpath "${1:?usage: $0 <path/to/App.AppImage>}")
patterns=${HOST_LIBS:-libwayland-client.so*}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

tool=${APPIMAGETOOL:-}
if [ -z "$tool" ]; then
  plugin="$HOME/.cache/tauri/linuxdeploy-plugin-appimage.AppImage"
  [ -x "$plugin" ] || { echo "no appimagetool: set APPIMAGETOOL or run a tauri build first" >&2; exit 1; }
  (cd "$work" && "$plugin" --appimage-extract >/dev/null && mv squashfs-root plugin)
  tool="$work/plugin/appimagetool-prefix/AppRun"
fi

chmod +x "$img"
offset=$(APPIMAGE_EXTRACT_AND_RUN=1 "$img" --appimage-offset)
head -c "$offset" "$img" > "$work/runtime"
(cd "$work" && "$img" --appimage-extract >/dev/null)

removed=0
for p in $patterns; do
  n=$(find "$work/squashfs-root/usr/lib" -name "$p" -print -delete | wc -l)
  removed=$((removed + n))
done
if [ "$removed" -eq 0 ]; then
  echo "nothing to remove from $(basename "$img"); left unchanged"
  exit 0
fi

if ! ARCH=$(uname -m) "$tool" --no-appstream --runtime-file "$work/runtime" \
  "$work/squashfs-root" "$work/out.AppImage" > "$work/appimagetool.log" 2>&1; then
  cat "$work/appimagetool.log" >&2
  exit 1
fi
chmod +x "$work/out.AppImage"
mv "$work/out.AppImage" "$img"
echo "removed $removed host-provided librar(y/ies) ($patterns) from $(basename "$img")"
