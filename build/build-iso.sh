#!/bin/sh
# Genera la ISO live/installabile di UniversalOS (base Debian 13 "trixie").
# Uso: ./build/build-iso.sh            (PC x86_64)
#      ARCH=arm64 ./build/build-iso.sh (PC ARM64 con UEFI)
# Requisiti sull'host di build: Debian/Ubuntu con live-build, eseguito come root.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
ARCH=${ARCH:-amd64}            # amd64 oppure arm64
WORK="$ROOT/build/out/$ARCH"

case "$ARCH" in
    amd64) BOOTLOADERS="grub-efi,syslinux" ;;
    arm64) BOOTLOADERS="grub-efi" ;;
    *) echo "ARCH non supportata: $ARCH (usa amd64 o arm64)" >&2; exit 1 ;;
esac

HOST_ARCH=$(dpkg --print-architecture)
if [ "$HOST_ARCH" != "$ARCH" ]; then
    echo "ATTENZIONE: build $ARCH su host $HOST_ARCH: serve qemu-user-static, ed è molto lenta." >&2
    echo "            Conviene generare la ISO $ARCH su una macchina (o VM) $ARCH." >&2
fi

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
if [ "$ARCH" = arm64 ] && [ ! -x "$WORK/config/includes.chroot/opt/uos/wine-x86_64/bin/wine" ]; then
    echo "ATTENZIONE: manca build/live/config/includes.chroot/opt/uos/wine-x86_64:" >&2
    echo "            la ISO ARM64 non potrà eseguire programmi Windows x86 (vedi docs/ARCHITETTURA.md)." >&2
fi

cd "$WORK"
lb config \
    --distribution trixie \
    --archive-areas "main contrib non-free non-free-firmware" \
    --architectures "$ARCH" \
    --bootloaders "$BOOTLOADERS" \
    --binary-images iso-hybrid \
    --debian-installer live \
    --iso-application "UniversalOS" \
    --iso-volume "UniversalOS" \
    --bootappend-live "boot=live components quiet splash hostname=universalos"
lb build

ls -lh "$WORK"/*.iso
