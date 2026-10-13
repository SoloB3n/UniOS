#!/bin/sh
# Build the UniOS live/installable ISO (based on Debian 13 "trixie").
# Usage: ./build/build-iso.sh            (x86_64 PCs)
#        ARCH=arm64 ./build/build-iso.sh (ARM64 PCs with UEFI)
# Build host requirements: Debian/Ubuntu with live-build, run as root.
#
# Environment:
#   ARCH     amd64 (default) or arm64
#   WORK     work directory (default: build/out/<arch>)
#   UOS_DEB  an already built unios .deb (default: build it with build-deb.sh)
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
ARCH=${ARCH:-amd64}
WORK=${WORK:-$ROOT/build/out/$ARCH}

case "$ARCH" in
    amd64) BOOTLOADERS="grub-efi,syslinux" ;;
    arm64) BOOTLOADERS="grub-efi" ;;
    *) echo "unsupported ARCH: $ARCH (use amd64 or arm64)" >&2; exit 1 ;;
esac

command -v lb >/dev/null || { echo "install live-build: apt install live-build" >&2; exit 1; }
[ "$(id -u)" = 0 ] || { echo "must be run as root" >&2; exit 1; }

HOST_ARCH=$(dpkg --print-architecture)
if [ "$HOST_ARCH" != "$ARCH" ]; then
    echo "WARNING: building $ARCH on a $HOST_ARCH host needs qemu-user-static and is very slow." >&2
    echo "         Build the $ARCH ISO on a $ARCH machine (or VM) instead." >&2
fi

rm -rf "$WORK"
mkdir -p "$WORK"
cp -a "$ROOT/build/live/config" "$WORK/"
mkdir -p "$WORK/config/packages.chroot"

# The uos dispatcher goes into the ISO as a Debian package
if [ -n "${UOS_DEB:-}" ]; then
    cp "$UOS_DEB" "$WORK/config/packages.chroot/"
else
    OUT="$WORK/config/packages.chroot" "$ROOT/build/build-deb.sh"
fi

if ! ls "$WORK"/config/packages.chroot/darling*.deb >/dev/null 2>&1; then
    echo "WARNING: no darling*.deb in build/live/config/packages.chroot/:" >&2
    echo "         the ISO won't run macOS programs (see docs/ARCHITECTURE.md)." >&2
fi
if [ "$ARCH" = arm64 ] && [ ! -x "$WORK/config/includes.chroot/opt/uos/wine-x86_64/bin/wine" ]; then
    echo "WARNING: build/live/config/includes.chroot/opt/uos/wine-x86_64 is missing:" >&2
    echo "         the ARM64 ISO won't run x86 Windows programs (see docs/ARCHITECTURE.md)." >&2
fi

cd "$WORK"
lb config \
    --distribution trixie \
    --archive-areas "main contrib non-free non-free-firmware" \
    --architectures "$ARCH" \
    --bootloaders "$BOOTLOADERS" \
    --binary-images iso-hybrid \
    --debian-installer none \
    --iso-application "UniOS" \
    --iso-volume "UniOS" \
    --bootappend-live "boot=live components quiet splash hostname=unios locales=it_IT.UTF-8 keyboard-layouts=it timezone=Europe/Rome"

# xz-compressed squashfs: the ISO must stay below the 2 GB GitHub release limit
case "$ARCH" in
    amd64) export MKSQUASHFS_OPTIONS="-comp xz -Xbcj x86 -b 1M" ;;
    *)     export MKSQUASHFS_OPTIONS="-comp xz -b 1M" ;;
esac
lb build

ls -lh "$WORK"/*.iso
