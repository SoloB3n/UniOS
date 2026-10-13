#!/bin/sh
# Build the UniOS Debian package: build/out/unios_<version>_<arch>.deb
#
# Environment:
#   OUT      output directory (default: build/out)
#   UOS_BIN  an already built uos binary (default: build it with cargo)
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${OUT:-$ROOT/build/out}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)

if [ -z "${UOS_BIN:-}" ]; then
    (cd "$ROOT" && cargo build --release --locked)
    UOS_BIN=$ROOT/target/release/uos
fi

# Debian architecture of the binary, from its ELF header
case "$(od -An -tx1 -j18 -N2 "$UOS_BIN" | tr -d ' \n')" in
    3e00) ARCH=amd64 ;;
    b700) ARCH=arm64 ;;
    *) echo "unsupported architecture for $UOS_BIN" >&2; exit 1 ;;
esac

# Minimum glibc version the binary needs
GLIBC=$(objdump -T "$UOS_BIN" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -n 1)
DEPENDS="libc6${GLIBC:+ (>= $GLIBC)}"

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
chmod 755 "$STAGE"

make -s -C "$ROOT" install DESTDIR="$STAGE" BIN="$UOS_BIN"
install -d "$STAGE/DEBIAN"
sed -e "s/@VERSION@/$VERSION/" -e "s/@ARCH@/$ARCH/" -e "s/@DEPENDS@/$DEPENDS/" \
    "$ROOT/packaging/deb/control" > "$STAGE/DEBIAN/control"
install -m 755 "$ROOT/packaging/deb/postinst" "$ROOT/packaging/deb/postrm" "$STAGE/DEBIAN/"

mkdir -p "$OUT"
DEB="$OUT/unios_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "$STAGE" "$DEB" >/dev/null
echo "$DEB"
