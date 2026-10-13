"""Scelta dello strato di compatibilità e costruzione del comando da eseguire.

Ogni famiglia di binari viene affidata a un "runner":

* ELF       -> esecuzione nativa (con emulatore CPU se l'architettura differisce)
* PE / MSI  -> Wine
* DOS       -> DOSBox
* Mach-O    -> Darling (.pkg -> installer di Darling)
* script    -> esecuzione nativa (l'interprete lo sceglie il kernel)

Le immagini .dmg sono contenitori: vanno risolte prima con
:func:`uos.macos.resolve_dmg`, che restituisce la .app o il .pkg da avviare.
"""

from __future__ import annotations

import os
import platform
import re
import shutil
from dataclasses import dataclass

from . import detect as d
from .profiles import Profile


class RunnerError(Exception):
    """Il programma non può essere avviato su questo sistema."""


@dataclass
class Plan:
    runner: str
    argv: list[str]
    env: dict[str, str]
    profile: str | None = None


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

# Wine compilato per x86_64, da eseguire sotto box64/FEX sugli host ARM.
X86_WINE_CANDIDATES = ["/opt/uos/wine-x86_64/bin/wine", "/opt/uos/wine-x86_64/bin/wine64"]


def _native_compatible(guest: str, host: str) -> bool:
    return guest == host or (host == d.X86_64 and guest == d.X86)


def cpu_emulator(guest: str, host: str, prefer: str | None = None) -> str | None:
    names = _CPU_EMULATORS.get((guest, host), [])
    if prefer:
        names = [prefer, *names]
    return _which(*names)


def data_dir() -> str:
    base = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    return os.path.join(base, "uos")


def safe_name(name: str) -> str:
    return re.sub(r"[^A-Za-z0-9._+-]", "_", name).strip("._") or "app"


def wine_prefix(info: d.BinaryInfo, isolate: bool, profile: Profile | None = None) -> str:
    if profile and profile.prefix:
        name = profile.prefix
    elif isolate or (profile and profile.isolate):
        name = os.path.splitext(os.path.basename(info.path))[0]
    else:
        name = "default"
    return os.path.join(data_dir(), "prefixes", safe_name(name))


def darling_path(host_path: str) -> str:
    """Darling monta il filesystem dell'host sotto /Volumes/SystemRoot."""
    return "/Volumes/SystemRoot" + os.path.abspath(host_path)


def _x86_wine() -> str | None:
    env = os.environ.get("UOS_X86_WINE")
    for p in ([env] if env else []) + X86_WINE_CANDIDATES:
        if p and os.access(p, os.X_OK):
            return p
    return None


def _wine_command(info: d.BinaryInfo, host: str, profile: Profile | None) -> list[str]:
    """Comando base di Wine (eventualmente preceduto dal traduttore CPU)."""
    if profile and profile.wine:
        if not os.access(profile.wine, os.X_OK):
            raise RunnerError(f"il profilo {profile.name} indica wine={profile.wine}, che non esiste")
        wine = profile.wine
    else:
        wine = None

    guest_x86 = info.family != d.PE or info.arch in (d.X86, d.X86_64)
    if host == d.ARM64 and guest_x86:
        # Il Wine nativo ARM esegue solo programmi Windows ARM64: per quelli
        # x86 serve un Wine x86_64 fatto girare sotto box64/FEX.
        wine = wine or _x86_wine()
        if not wine:
            raise RunnerError(
                "programma Windows x86 su ARM: serve un Wine x86_64 in /opt/uos/wine-x86_64 "
                "(o nella variabile UOS_X86_WINE)")
        emu = cpu_emulator(d.X86_64, host, profile.emulator if profile else None)
        if not emu:
            raise RunnerError("programma Windows x86 su ARM: serve box64 o FEX")
        return [emu, wine]

    wine = wine or _which("wine", "wine64")
    if not wine:
        raise RunnerError("Wine non è installato: impossibile avviare programmi Windows")
    return [wine]


def plan(info: d.BinaryInfo, args: list[str], isolate: bool = False,
         profile: Profile | None = None) -> Plan:
    host = host_arch()
    env: dict[str, str] = dict(profile.env) if profile else {}
    args = [*(profile.args if profile else []), *args]
    path = os.path.abspath(info.path)
    pname = profile.name if profile else None

    if info.family in (d.ELF, d.SCRIPT):
        if info.family == d.ELF and info.arch and not _native_compatible(info.arch, host):
            emu = cpu_emulator(info.arch, host, profile.emulator if profile else None)
            if not emu:
                raise RunnerError(
                    f"binario Linux {info.arch} su host {host}: nessun emulatore CPU "
                    f"installato (prova box64, FEX o qemu-user)")
            return Plan("native+emu", [emu, path, *args], env, pname)
        return Plan("native", [path, *args], env, pname)

    if info.family in (d.PE, d.MSI, d.WIN_BATCH):
        if info.family == d.PE and info.details.get("dll"):
            raise RunnerError("è una DLL di Windows, non un programma avviabile")
        wine = _wine_command(info, host, profile)
        env["WINEPREFIX"] = wine_prefix(info, isolate, profile)
        if profile and profile.wine_dll_overrides():
            env["WINEDLLOVERRIDES"] = profile.wine_dll_overrides()
        if info.family == d.MSI:
            argv = [*wine, "msiexec", "/i", path, *args]
        elif info.family == d.WIN_BATCH:
            argv = [*wine, "cmd", "/c", path, *args]
        else:
            argv = [*wine, path, *args]
        return Plan("wine", argv, env, pname)

    if info.family == d.DOS:
        dosbox = _which("dosbox-x", "dosbox-staging", "dosbox")
        if not dosbox:
            raise RunnerError("DOSBox non è installato: impossibile avviare programmi MS-DOS")
        return Plan("dosbox", [dosbox, path, "-exit", *args], env, pname)

    if info.family in (d.MACHO, d.MACHO_FAT, d.MAC_APP, d.MAC_PKG):
        darling = _which("darling")
        if info.family == d.MAC_PKG:
            if not darling:
                raise RunnerError("Darling non è installato: impossibile installare pacchetti .pkg")
            return Plan("darling-installer",
                        [darling, "shell", "installer", "-pkg", darling_path(path), "-target", "/"],
                        env, pname)
        target = info.executable if info.family == d.MAC_APP else path
        if not target:
            raise RunnerError("bundle .app senza eseguibile")
        archs = set(info.archs)
        if host == d.X86_64 and archs and not archs & {d.X86_64, d.X86}:
            raise RunnerError(
                "il programma macOS contiene solo codice "
                f"{', '.join(sorted(archs))}: Darling richiede una slice x86_64")
        if not darling:
            raise RunnerError("Darling non è installato: impossibile avviare programmi macOS")
        return Plan("darling", [darling, "shell", darling_path(target), *args], env, pname)

    if info.family == d.MAC_DMG:
        raise RunnerError("immagine .dmg: va prima estratta (usa uos run, che lo fa da sé)")

    if info.family == d.JAVA_CLASS:
        java = _which("java")
        if not java:
            raise RunnerError("Java non è installato")
        directory, name = os.path.split(path)
        return Plan("java", [java, "-cp", directory, os.path.splitext(name)[0], *args], env, pname)

    raise RunnerError("formato non riconosciuto")
