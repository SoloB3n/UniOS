"""Scelta dello strato di compatibilità e costruzione del comando da eseguire.

Ogni famiglia di binari viene affidata a un "runner":

* ELF       -> esecuzione nativa (con emulatore CPU se l'architettura differisce)
* PE / MSI  -> Wine
* DOS       -> DOSBox
* Mach-O    -> Darling
* script    -> esecuzione nativa (l'interprete lo sceglie il kernel)
"""

from __future__ import annotations

import os
import platform
import shutil
from dataclasses import dataclass

from . import detect as d


class RunnerError(Exception):
    """Il programma non può essere avviato su questo sistema."""


@dataclass
class Plan:
    runner: str
    argv: list[str]
    env: dict[str, str]


def host_arch() -> str:
    m = platform.machine().lower()
    return {"amd64": d.X86_64, "x86_64": d.X86_64, "arm64": d.ARM64,
            "aarch64": d.ARM64, "i686": d.X86, "i386": d.X86}.get(m, m)


def _which(*names: str) -> str | None:
    for n in names:
        p = shutil.which(n)
        if p:
            return p
    return None


# Emulatori user-space per eseguire binari di un'altra architettura CPU.
# In ordine di preferenza (i traduttori dinamici sono molto più veloci di qemu).
_CPU_EMULATORS = {
    (d.X86_64, d.ARM64): ["box64", "FEXInterpreter", "qemu-x86_64"],
    (d.X86, d.ARM64): ["box86", "FEXInterpreter", "qemu-i386"],
    (d.ARM64, d.X86_64): ["qemu-aarch64"],
    (d.ARM, d.X86_64): ["qemu-arm"],
    (d.RISCV64, d.X86_64): ["qemu-riscv64"],
    (d.RISCV64, d.ARM64): ["qemu-riscv64"],
}


def _native_compatible(guest: str, host: str) -> bool:
    return guest == host or (host == d.X86_64 and guest == d.X86)


def cpu_emulator(guest: str, host: str) -> str | None:
    return _which(*_CPU_EMULATORS.get((guest, host), []))


def data_dir() -> str:
    base = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    return os.path.join(base, "uos")


def wine_prefix(info: d.BinaryInfo, isolate: bool) -> str:
    if isolate:
        name = os.path.splitext(os.path.basename(info.path))[0] or "app"
        return os.path.join(data_dir(), "prefixes", name)
    return os.path.join(data_dir(), "prefixes", "default")


def darling_path(host_path: str) -> str:
    """Darling monta il filesystem dell'host sotto /Volumes/SystemRoot."""
    return "/Volumes/SystemRoot" + os.path.abspath(host_path)


def plan(info: d.BinaryInfo, args: list[str], isolate: bool = False) -> Plan:
    host = host_arch()
    env: dict[str, str] = {}
    path = os.path.abspath(info.path)

    if info.family in (d.ELF, d.SCRIPT):
        if info.family == d.ELF and info.arch and not _native_compatible(info.arch, host):
            emu = cpu_emulator(info.arch, host)
            if not emu:
                raise RunnerError(
                    f"binario Linux {info.arch} su host {host}: nessun emulatore CPU "
                    f"installato (prova box64, FEX o qemu-user)")
            return Plan("native+emu", [emu, path, *args], env)
        return Plan("native", [path, *args], env)

    if info.family in (d.PE, d.MSI, d.WIN_BATCH):
        if info.family == d.PE and info.details.get("dll"):
            raise RunnerError("è una DLL di Windows, non un programma avviabile")
        wine = _which("wine", "wine64")
        if not wine:
            raise RunnerError("Wine non è installato: impossibile avviare programmi Windows")
        env["WINEPREFIX"] = wine_prefix(info, isolate)
        if info.family == d.MSI:
            argv = [wine, "msiexec", "/i", path, *args]
        elif info.family == d.WIN_BATCH:
            argv = [wine, "cmd", "/c", path, *args]
        else:
            argv = [wine, path, *args]
        # Su host ARM Wine per x86 va eseguito sotto un traduttore (es. box64)
        if info.family == d.PE and info.arch in (d.X86, d.X86_64) and host == d.ARM64:
            emu = cpu_emulator(d.X86_64, host)
            if not emu:
                raise RunnerError("programma Windows x86 su ARM: serve box64 o FEX")
            argv = [emu, *argv]
        return Plan("wine", argv, env)

    if info.family == d.DOS:
        dosbox = _which("dosbox-x", "dosbox-staging", "dosbox")
        if not dosbox:
            raise RunnerError("DOSBox non è installato: impossibile avviare programmi MS-DOS")
        return Plan("dosbox", [dosbox, path, "-exit", *args], env)

    if info.family in (d.MACHO, d.MACHO_FAT, d.MAC_APP):
        target = info.executable if info.family == d.MAC_APP else path
        if not target:
            raise RunnerError("bundle .app senza eseguibile")
        archs = set(info.archs)
        if host == d.X86_64 and archs and not archs & {d.X86_64, d.X86}:
            raise RunnerError(
                "il programma macOS contiene solo codice "
                f"{', '.join(sorted(archs))}: Darling richiede una slice x86_64")
        darling = _which("darling")
        if not darling:
            raise RunnerError("Darling non è installato: impossibile avviare programmi macOS")
        return Plan("darling", [darling, "shell", darling_path(target), *args], env)

    if info.family == d.JAVA_CLASS:
        java = _which("java")
        if not java:
            raise RunnerError("Java non è installato")
        directory, name = os.path.split(path)
        return Plan("java", [java, "-cp", directory, os.path.splitext(name)[0], *args], env)

    raise RunnerError("formato non riconosciuto")
