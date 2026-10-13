#!/bin/sh
# Genera la ISO live/installabile di UniversalOS (base Debian 13 "trixie").
# Requisiti sull'host di build: Debian/Ubuntu con live-build, eseguito come root.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK="$ROOT/build/out"

command -v lb >/dev/null || { echo "installa live-build: apt install live-build" >&2; exit 1; }
[ "$(id -u)" = 0 ] || { echo "va eseguito come root" >&2; exit 1; }

rm -rf "$WORK"
mkdir -p "$WORK"
cp -a "$ROOT/build/live/config" "$WORK/"

# Il dispatcher uos viene copiato nel filesystem della ISO
make -C "$ROOT" install DESTDIR="$WORK/config/includes.chroot"

if ! ls "$WORK"/config/packages.chroot/darling*.deb >/dev/null 2>&1; then
    echo "ATTENZIONE: nessun pacchetto darling*.deb in build/live/config/packages.chroot/:" >&2
    echo "            la ISO non potrà eseguire programmi macOS (vedi docs/ARCHITETTURA.md)." >&2
fi

cd "$WORK"
lb config \
    --distribution trixie \
    --archive-areas "main contrib non-free non-free-firmware" \
    --architectures amd64 \
    --binary-images iso-hybrid \
    --debian-installer live \
    --iso-application "UniversalOS" \
    --iso-volume "UniversalOS" \
    --bootappend-live "boot=live components quiet splash hostname=universalos"
lb build

ls -lh "$WORK"/*.iso
