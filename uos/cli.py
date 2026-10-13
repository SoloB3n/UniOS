"""Interfaccia a riga di comando ``uos``.

    uos run [--isolate] [--profile NOME] [--dry-run] PROGRAMMA [ARGOMENTI...]
    uos info PROGRAMMA
    uos install PROGRAMMA
    uos list
    uos uninstall NOME
    uos profile [--new] PROGRAMMA
    uos doctor
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys

from . import __version__
from . import detect as d
from . import install as inst
from . import macos, profiles, runners

_BINFMT_DIR = "/proc/sys/fs/binfmt_misc"


def _err(msg: str) -> None:
    print(f"uos: {msg}", file=sys.stderr)


def _resolve(program: str) -> d.BinaryInfo:
    """Rileva il formato; le immagini .dmg vengono estratte e sostituite dal loro contenuto."""
    info = d.detect(program)
    if info.family == d.MAC_DMG:
        info = d.detect(macos.resolve_dmg(program))
    return info


def _profile_for(info: d.BinaryInfo, name: str | None) -> profiles.Profile | None:
    return profiles.by_name(name) if name else profiles.find(info)


def _exec(p: runners.Plan, dry_run: bool) -> int:
    if dry_run:
        env = " ".join(f"{k}={v}" for k, v in p.env.items())
        tag = f"{p.runner}, profilo {p.profile}" if p.profile else p.runner
        print(f"[{tag}] {env + ' ' if env else ''}{' '.join(p.argv)}")
        return 0
    if "WINEPREFIX" in p.env:
        os.makedirs(p.env["WINEPREFIX"], exist_ok=True)
    os.execvpe(p.argv[0], p.argv, {**os.environ, **p.env})


def cmd_info(ns: argparse.Namespace) -> int:
    info = d.detect(ns.program)
    out = {"path": info.path, "family": info.family, "archs": info.archs, **info.details}
    if info.executable:
        out["executable"] = info.executable
    if info.family == d.MAC_DMG:
        out["runner"] = "dmg (estrazione, poi la .app contenuta)"
        print(json.dumps(out, indent=2, ensure_ascii=False))
        return 0
    try:
        profile = profiles.find(info)
        out["profile"] = profile.source if profile else None
        out["runner"] = runners.plan(info, [], profile=profile).runner
    except (runners.RunnerError, profiles.ProfileError) as e:
        out["runner"] = None
        out["problem"] = str(e)
    print(json.dumps(out, indent=2, ensure_ascii=False))
    return 0


def cmd_run(ns: argparse.Namespace) -> int:
    if not os.path.exists(ns.program):
        _err(f"file non trovato: {ns.program}")
        return 127
    try:
        info = _resolve(ns.program)
        p = runners.plan(info, ns.args, isolate=ns.isolate, profile=_profile_for(info, ns.profile))
    except (runners.RunnerError, profiles.ProfileError, macos.MacError) as e:
        _err(f"{ns.program}: {e}")
        return 126
    return _exec(p, ns.dry_run)


def cmd_install(ns: argparse.Namespace) -> int:
    if not os.path.exists(ns.program):
        _err(f"file non trovato: {ns.program}")
        return 127
    try:
        result = inst.install(ns.program)
    except (inst.InstallError, runners.RunnerError, macos.MacError, OSError) as e:
        _err(f"{ns.program}: {e}")
        return 1
    if isinstance(result, runners.Plan):
        print(f"uos: avvio dell'installer ({result.runner})", file=sys.stderr)
        return _exec(result, ns.dry_run)
    print(f"Installato. Voce di menu: {result}")
    return 0


def cmd_list(_ns: argparse.Namespace) -> int:
    entries = inst.installed()
    if not entries:
        print("Nessun programma installato con uos.")
    for e in entries:
        print(f"{e['id']:30} {e.get('Name', ''):30} {e.get('X-UOS-Installed', '')}")
    return 0


def cmd_uninstall(ns: argparse.Namespace) -> int:
    try:
        for path in inst.uninstall(ns.name):
            print(f"rimosso {path}")
    except (inst.InstallError, OSError) as e:
        _err(str(e))
        return 1
    return 0


def cmd_profile(ns: argparse.Namespace) -> int:
    if not os.path.exists(ns.program):
        _err(f"file non trovato: {ns.program}")
        return 127
    try:
        info = _resolve(ns.program)
    except macos.MacError as e:
        _err(f"{ns.program}: {e}")
        return 1
    if ns.new:
        name = os.path.splitext(os.path.basename(info.path))[0]
        path = os.path.join(profiles.user_dir(), runners.safe_name(name).lower() + ".toml")
        if os.path.exists(path):
            _err(f"il profilo esiste già: {path}")
            return 1
        os.makedirs(profiles.user_dir(), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(profiles.template(info))
        print(path)
        return 0
    try:
        profile = profiles.find(info)
    except profiles.ProfileError as e:
        _err(str(e))
        return 1
    if not profile:
        print("Nessun profilo corrisponde (crealo con: uos profile --new PROGRAMMA)")
        return 1
    print(profile.source)
    return 0


def cmd_doctor(_ns: argparse.Namespace) -> int:
    host = runners.host_arch()
    print(f"UniOS {__version__} — architettura host: {host}\n")
    layers = [
        ("Windows (Wine)", ["wine", "wine64"]),
        ("macOS (Darling)", ["darling"]),
        ("Immagini .dmg (7-Zip)", ["7zz", "7z"]),
        ("MS-DOS (DOSBox)", ["dosbox-x", "dosbox-staging", "dosbox"]),
        ("Emulatore CPU generico", ["qemu-x86_64", "qemu-aarch64"]),
        ("Java", ["java"]),
    ]
    if host != d.X86_64:
        layers.append(("Traduttore x86 su ARM", ["box64", "FEXInterpreter"]))
    missing = 0
    for label, bins in layers:
        found = next((shutil.which(b) for b in bins if shutil.which(b)), None)
        missing += not found
        print(f"  [{'OK ' if found else '-- '}] {label:26} {found or 'non installato'}")
    if host == d.ARM64:
        x86_wine = runners._x86_wine()
        missing += not x86_wine
        print(f"  [{'OK ' if x86_wine else '-- '}] {'Wine x86_64 (per box64)':26} "
              f"{x86_wine or 'non installato (/opt/uos/wine-x86_64)'}")

    print()
    if os.path.isdir(_BINFMT_DIR):
        entries = sorted(e for e in os.listdir(_BINFMT_DIR) if e.startswith("uos-"))
        if entries:
            print(f"  binfmt_misc: registrati {', '.join(entries)}")
        else:
            print("  binfmt_misc: nessuna voce uos-* (i file .exe/Mach-O non partono con ./programma)")
    else:
        print("  binfmt_misc: non montato")
    n = sum(len([f for f in os.listdir(p) if f.endswith(".toml")])
            for p in profiles.search_dirs() if os.path.isdir(p))
    print(f"  profili: {n} in {', '.join(profiles.search_dirs())}")
    return 0 if missing == 0 else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="uos", description="Avvia programmi Linux, Windows e macOS.")
    parser.add_argument("--version", action="version", version=f"uos {__version__}")
    sub = parser.add_subparsers(dest="command", required=True)

    run = sub.add_parser("run", help="avvia un programma con lo strato di compatibilità giusto")
    run.add_argument("--isolate", action="store_true", help="usa un prefisso Wine dedicato a questo programma")
    run.add_argument("--profile", metavar="NOME", help="usa questo profilo invece di quello rilevato")
    run.add_argument("--dry-run", action="store_true", help="mostra il comando senza eseguirlo")
    run.add_argument("program")
    run.add_argument("args", nargs=argparse.REMAINDER)
    run.set_defaults(func=cmd_run)

    info = sub.add_parser("info", help="mostra formato, architettura e runner di un programma")
    info.add_argument("program")
    info.set_defaults(func=cmd_info)

    install = sub.add_parser("install", help="installa un programma e lo aggiunge al menu")
    install.add_argument("--dry-run", action="store_true", help="per gli installer: mostra il comando")
    install.add_argument("program")
    install.set_defaults(func=cmd_install)

    lst = sub.add_parser("list", help="elenca i programmi installati con uos")
    lst.set_defaults(func=cmd_list)

    uninstall = sub.add_parser("uninstall", help="rimuove un programma installato con uos")
    uninstall.add_argument("name", help="id, nome o file mostrato da uos list")
    uninstall.set_defaults(func=cmd_uninstall)

    profile = sub.add_parser("profile", help="mostra o crea il profilo di un programma")
    profile.add_argument("--new", action="store_true", help="crea un profilo modello in ~/.config/uos/profiles")
    profile.add_argument("program")
    profile.set_defaults(func=cmd_profile)

    doctor = sub.add_parser("doctor", help="verifica quali strati di compatibilità sono installati")
    doctor.set_defaults(func=cmd_doctor)

    ns = parser.parse_args(argv)
    return ns.func(ns)
