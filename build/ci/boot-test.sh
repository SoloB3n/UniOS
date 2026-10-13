#!/bin/sh
# Avvia la ISO in QEMU con il parametro uos.selftest e verifica l'esito
# dell'autoverifica (system/bin/uos-selftest) letto dalla porta seriale.
# Uso: build/ci/boot-test.sh <directory di lavoro di live-build> [timeout in secondi]
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
    echo "KVM non disponibile: emulazione software, sarà lenta" >&2
fi

# Avvio diretto di kernel e initrd: niente menu del bootloader da attraversare.
# live-boot trova comunque il filesystem nella ISO collegata come CD-ROM.
timeout "$TIMEOUT" qemu-system-x86_64 $ACCEL -m 4096 -smp 2 \
    -cdrom "$ISO" -kernel "$KERNEL" -initrd "$INITRD" \
    -append "boot=live components console=ttyS0,115200 systemd.unit=multi-user.target uos.selftest" \
    -nographic -no-reboot -nic user,model=virtio-net-pci \
    < /dev/null 2>&1 | tee "$LOG" || true

echo
grep "UOS-SELFTEST:" "$LOG" || true
if grep -q "UOS-SELFTEST: RISULTATO OK" "$LOG"; then
    echo "Collaudo della ISO riuscito"
else
    echo "Collaudo della ISO FALLITO (log completo in $LOG)" >&2
    exit 1
fi
