#!/usr/bin/env bash
# Builds an AppImage from the release binaries.
# Usage: packaging/linux/build-appimage.sh <version> <output.AppImage>
# Needs: target/release/designcomputer-app and target/release/designcomputer (cargo build --release),
# and appimagetool on PATH or APPIMAGETOOL set.
set -euo pipefail
version="${1:?version}"; out="${2:?output file}"
here="$(cd "$(dirname "$0")" && pwd)"; root="$here/../.."
appdir="$(mktemp -d)/DesignComputer.AppDir"
mkdir -p "$appdir/usr/bin"
cp "$root/target/release/designcomputer-app" "$root/target/release/designcomputer" "$appdir/usr/bin/"
cp "$here/designcomputer.desktop" "$appdir/"
cp "$here/designcomputer.png" "$appdir/"
cp "$root/LICENSE" "$appdir/"
cat > "$appdir/AppRun" <<'EOF'
#!/bin/sh
here="$(dirname "$(readlink -f "$0")")"
export PATH="$here/usr/bin:$PATH"
exec "$here/usr/bin/designcomputer-app" "$@"
EOF
chmod +x "$appdir/AppRun"
# The graphics stack (OpenGL, X11, Wayland) is deliberately not bundled: it must match the
# machine's driver, and the app loads it at run time.
# --appimage-extract-and-run lets the tool itself run on machines without FUSE (CI runners).
ARCH=x86_64 "${APPIMAGETOOL:-appimagetool}" --appimage-extract-and-run "$appdir" "$out"
chmod +x "$out"
echo "built $out ($version)"
