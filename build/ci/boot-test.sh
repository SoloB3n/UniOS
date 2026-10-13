#!/bin/sh
# Boot the ISO in QEMU with the uos.selftest parameter and check the result of
# the self-test (system/bin/uos-selftest) read from the serial port.
# Usage: build/ci/boot-test.sh <live-build work directory> [timeout in seconds]
set -eu

WORK=$1
TIMEOUT=${2:-2700}
ISO=$(ls "$WORK"/*.iso | head -n 1)
KERNEL=$(ls "$WORK"/binary/live/vmlinuz* | head -n 1)
INITRD=$(ls "$WORK"/binary/live/initrd.img* | head -n 1)
LOG=${LOG:-boot-test.log}

ACCEL="-accel tcg"
if [ -w /dev/kvm ]; then
    ACCEL="-accel kvm -cpu host"
else
    echo "KVM not available: software emulation, this will be slow" >&2
fi

# Boot kernel and initrd directly: no bootloader menu to go through.
# live-boot still finds the filesystem in the ISO attached as a CD-ROM.
# "noprompt" skips live-boot's "remove the medium and press ENTER" at shutdown.
timeout "$TIMEOUT" qemu-system-x86_64 $ACCEL -m 4096 -smp 2 \
    -cdrom "$ISO" -kernel "$KERNEL" -initrd "$INITRD" \
    -append "boot=live components noprompt console=ttyS0,115200 systemd.unit=multi-user.target uos.selftest" \
    -nographic -no-reboot -nic user,model=virtio-net-pci \
    < /dev/null 2>&1 | tee "$LOG" || true

echo
# Each result appears twice (console and kernel log): show it once
grep -o "UOS-SELFTEST: .*" "$LOG" | tr -d '\r' | awk '!seen[$0]++' || true
if grep -q "UOS-SELFTEST: RESULT OK" "$LOG"; then
    echo "ISO test passed"
else
    echo "ISO test FAILED (full log in $LOG)" >&2
    exit 1
fi
