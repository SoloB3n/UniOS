#!/bin/sh
# Crea il pacchetto Debian di UniversalOS in build/out/universalos_<versione>_all.deb
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${OUT:-$ROOT/build/out}
VERSION=$(sed -n 's/^__version__ = "\(.*\)"/\1/p' "$ROOT/uos/__init__.py")
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
chmod 755 "$STAGE"

make -s -C "$ROOT" install DESTDIR="$STAGE"
install -d "$STAGE/DEBIAN"
sed "s/@VERSION@/$VERSION/" "$ROOT/packaging/deb/control" > "$STAGE/DEBIAN/control"
install -m 755 "$ROOT/packaging/deb/postinst" "$ROOT/packaging/deb/prerm" "$ROOT/packaging/deb/postrm" "$STAGE/DEBIAN/"
find "$STAGE" -name __pycache__ -prune -exec rm -rf {} +

mkdir -p "$OUT"
DEB="$OUT/universalos_${VERSION}_all.deb"
dpkg-deb --root-owner-group --build "$STAGE" "$DEB" >/dev/null
echo "$DEB"
