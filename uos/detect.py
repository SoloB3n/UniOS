"""Riconoscimento del formato di un eseguibile a partire dai magic bytes.

Non si fida mai dell'estensione del file: legge l'intestazione e restituisce
famiglia (ELF / PE / Mach-O / ...), architettura CPU e dettagli utili al
dispatcher per scegliere lo strato di compatibilità corretto.
"""

from __future__ import annotations

import os
import plistlib
import struct
from dataclasses import dataclass, field

# Famiglie di formato
ELF = "elf"            # Linux / Unix
PE = "pe"              # Windows (PE32 / PE32+)
DOS = "dos"            # MS-DOS (MZ senza intestazione PE)
MSI = "msi"            # Windows Installer (OLE Compound File)
MACHO = "macho"        # macOS (thin)
MACHO_FAT = "macho-fat"  # macOS universal binary
MAC_APP = "mac-app"    # bundle .app di macOS
JAVA_CLASS = "java-class"
SCRIPT = "script"      # #! shebang
WIN_BATCH = "win-batch"  # .bat / .cmd
UNKNOWN = "unknown"

# Architetture normalizzate
X86 = "i386"
X86_64 = "x86_64"
ARM = "arm"
ARM64 = "aarch64"
RISCV64 = "riscv64"
PPC = "ppc"
PPC64 = "ppc64"

_ELF_MACHINES = {
    0x03: X86, 0x3E: X86_64, 0x28: ARM, 0xB7: ARM64, 0xF3: RISCV64,
    0x14: PPC, 0x15: PPC64,
}
_PE_MACHINES = {
    0x014C: X86, 0x8664: X86_64, 0x01C4: ARM, 0xAA64: ARM64,
}
_MACHO_CPUS = {
    7: X86, 0x01000007: X86_64, 12: ARM, 0x0100000C: ARM64,
    18: PPC, 0x01000012: PPC64,
}

_PE_SUBSYSTEMS = {1: "native", 2: "gui", 3: "console", 9: "wince-gui", 10: "efi"}

_MSI_MAGIC = bytes.fromhex("d0cf11e0a1b11ae1")
_HEADER_SIZE = 4096


@dataclass
class BinaryInfo:
    path: str
    family: str
    archs: list[str] = field(default_factory=list)
    details: dict = field(default_factory=dict)
    # Per i bundle .app: il vero eseguibile dentro Contents/MacOS
    executable: str | None = None

    @property
    def arch(self) -> str | None:
        return self.archs[0] if self.archs else None


def detect(path: str) -> BinaryInfo:
    """Analizza ``path`` e restituisce un :class:`BinaryInfo`."""
    if os.path.isdir(path):
        return _detect_app_bundle(path)

    with open(path, "rb") as f:
        head = f.read(_HEADER_SIZE)

    if head.startswith(b"\x7fELF"):
        return _detect_elf(path, head)
    if head.startswith(b"MZ"):
        return _detect_mz(path, head)
    if head.startswith(_MSI_MAGIC):
        return BinaryInfo(path, MSI)
    if len(head) >= 4:
        magic_be = struct.unpack(">I", head[:4])[0]
        if magic_be in (0xFEEDFACE, 0xFEEDFACF, 0xCEFAEDFE, 0xCFFAEDFE):
            return _detect_macho(path, head)
        if magic_be in (0xCAFEBABE, 0xCAFEBABF):
            return _detect_cafebabe(path, head, magic_be)
    if head.startswith(b"#!"):
        line = head.split(b"\n", 1)[0][2:].strip().decode("utf-8", "replace")
        return BinaryInfo(path, SCRIPT, details={"interpreter": line})
    if path.lower().endswith((".bat", ".cmd")):
        return BinaryInfo(path, WIN_BATCH)
    return BinaryInfo(path, UNKNOWN)


def _detect_elf(path: str, head: bytes) -> BinaryInfo:
    if len(head) < 20:
        return BinaryInfo(path, UNKNOWN, details={"error": "ELF troncato"})
    bits = {1: 32, 2: 64}.get(head[4])
    endian = "<" if head[5] == 1 else ">"
    machine = struct.unpack(endian + "H", head[18:20])[0]
    etype = struct.unpack(endian + "H", head[16:18])[0]
    arch = _ELF_MACHINES.get(machine, f"elf-machine-{machine:#x}")
    return BinaryInfo(path, ELF, [arch], {
        "bits": bits,
        "endian": "little" if endian == "<" else "big",
        "type": {2: "exec", 3: "dyn"}.get(etype, str(etype)),
    })


def _detect_mz(path: str, head: bytes) -> BinaryInfo:
    if len(head) < 0x40:
        return BinaryInfo(path, DOS)
    pe_off = struct.unpack("<I", head[0x3C:0x40])[0]
    pe_head = head[pe_off:pe_off + 0x100] if pe_off + 24 <= len(head) else b""
    if not pe_head.startswith(b"PE\0\0"):
        # e_lfanew oltre il primo blocco: rileggi a quell'offset
        with open(path, "rb") as f:
            f.seek(pe_off)
            pe_head = f.read(0x100)
    if not pe_head.startswith(b"PE\0\0"):
        return BinaryInfo(path, DOS)

    machine = struct.unpack("<H", pe_head[4:6])[0]
    characteristics = struct.unpack("<H", pe_head[22:24])[0]
    opt = pe_head[24:]
    details: dict = {"dll": bool(characteristics & 0x2000)}
    if len(opt) >= 2:
        opt_magic = struct.unpack("<H", opt[:2])[0]
        details["format"] = {0x10B: "PE32", 0x20B: "PE32+"}.get(opt_magic, hex(opt_magic))
        sub_off = 68
        if len(opt) >= sub_off + 2:
            sub = struct.unpack("<H", opt[sub_off:sub_off + 2])[0]
            details["subsystem"] = _PE_SUBSYSTEMS.get(sub, str(sub))
        # Data directory 14 = CLR runtime header (.NET)
        dd_base = 96 if opt_magic == 0x10B else 112
        clr = dd_base + 14 * 8
        if len(opt) >= clr + 8:
            rva, size = struct.unpack("<II", opt[clr:clr + 8])
            details["dotnet"] = bool(rva and size)
    arch = _PE_MACHINES.get(machine, f"pe-machine-{machine:#x}")
    return BinaryInfo(path, PE, [arch], details)


def _detect_macho(path: str, head: bytes) -> BinaryInfo:
    magic_be = struct.unpack(">I", head[:4])[0]
    endian = ">" if magic_be in (0xFEEDFACE, 0xFEEDFACF) else "<"
    cputype, _sub, filetype = struct.unpack(endian + "iiI", head[4:16])
    arch = _MACHO_CPUS.get(cputype & 0xFFFFFFFF, f"macho-cpu-{cputype:#x}")
    return BinaryInfo(path, MACHO, [arch], {
        "bits": 64 if magic_be in (0xFEEDFACF, 0xCFFAEDFE) else 32,
        "filetype": {2: "exec", 6: "dylib", 8: "bundle"}.get(filetype, str(filetype)),
    })


def _detect_cafebabe(path: str, head: bytes, magic: int) -> BinaryInfo:
    # 0xCAFEBABE è condiviso da universal binary Mach-O e classi Java.
    # Mach-O fat: segue nfat_arch (numero piccolo). Java: minor/major version
    # con major >= 45, quindi il campo a 32 bit è sempre >= 45.
    n = struct.unpack(">I", head[4:8])[0] if len(head) >= 8 else 0
    if magic == 0xCAFEBABE and n >= 30:
        return BinaryInfo(path, JAVA_CLASS)
    entry = 32 if magic == 0xCAFEBABF else 20
    archs = []
    for i in range(min(n, 16)):
        off = 8 + i * entry
        if off + 4 > len(head):
            break
        cpu = struct.unpack(">I", head[off:off + 4])[0]
        archs.append(_MACHO_CPUS.get(cpu, f"macho-cpu-{cpu:#x}"))
    return BinaryInfo(path, MACHO_FAT, archs, {"slices": n})


def _detect_app_bundle(path: str) -> BinaryInfo:
    plist_path = os.path.join(path, "Contents", "Info.plist")
    if not os.path.isfile(plist_path):
        return BinaryInfo(path, UNKNOWN, details={"error": "directory senza Contents/Info.plist"})
    with open(plist_path, "rb") as f:
        info = plistlib.load(f)
    exe_name = info.get("CFBundleExecutable") or os.path.splitext(os.path.basename(path))[0]
    exe = os.path.join(path, "Contents", "MacOS", exe_name)
    inner = detect(exe) if os.path.isfile(exe) else None
    return BinaryInfo(
        path, MAC_APP,
        inner.archs if inner else [],
        {"bundle_id": info.get("CFBundleIdentifier"), "name": info.get("CFBundleName")},
        executable=exe,
    )
