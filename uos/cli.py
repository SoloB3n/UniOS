"""Interfaccia a riga di comando ``uos``.

    uos run [--isolate] [--dry-run] PROGRAMMA [ARGOMENTI...]
    uos info PROGRAMMA
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
from . import runners

_BINFMT_DIR = "/proc/sys/fs/binfmt_misc"


def cmd_info(ns: argparse.Namespace) -> int:
    info = d.detect(ns.program)
    out = {"path": info.path, "family": info.family, "archs": info.archs, **info.details}
    if info.executable:
        out["executable"] = info.executable
    try:
        out["runner"] = runners.plan(info, []).runner
    except runners.RunnerError as e:
        out["runner"] = None
        out["problem"] = str(e)
    print(json.dumps(out, indent=2, ensure_ascii=False))
    return 0


def cmd_run(ns: argparse.Namespace) -> int:
    if not os.path.exists(ns.program):
        print(f"uos: file non trovato: {ns.program}", file=sys.stderr)
        return 127
    info = d.detect(ns.program)
    try:
        p = runners.plan(info, ns.args, isolate=ns.isolate)
    except runners.RunnerError as e:
        print(f"uos: {ns.program}: {e}", file=sys.stderr)
        return 126
    if ns.dry_run:
        env = " ".join(f"{k}={v}" for k, v in p.env.items())
        print(f"[{p.runner}] {env + ' ' if env else ''}{' '.join(p.argv)}")
        return 0
    if "WINEPREFIX" in p.env:
        os.makedirs(p.env["WINEPREFIX"], exist_ok=True)
    os.execvpe(p.argv[0], p.argv, {**os.environ, **p.env})


def cmd_doctor(_ns: argparse.Namespace) -> int:
    print(f"UniversalOS {__version__} — architettura host: {runners.host_arch()}\n")
    layers = [
        ("Windows (Wine)", ["wine", "wine64"]),
        ("macOS (Darling)", ["darling"]),
        ("MS-DOS (DOSBox)", ["dosbox-x", "dosbox-staging", "dosbox"]),
        ("Emulatore CPU generico", ["qemu-x86_64", "qemu-aarch64"]),
        ("Java", ["java"]),
    ]
    if runners.host_arch() != "x86_64":
        layers.append(("Traduttore x86 su ARM", ["box64", "FEXInterpreter"]))
    missing = 0
    for label, bins in layers:
        found = next((shutil.which(b) for b in bins if shutil.which(b)), None)
        mark = "OK " if found else "-- "
        missing += not found
        print(f"  [{mark}] {label:26} {found or 'non installato'}")

    print()
    if os.path.isdir(_BINFMT_DIR):
        entries = sorted(e for e in os.listdir(_BINFMT_DIR) if e.startswith("uos-"))
        if entries:
            print(f"  binfmt_misc: registrati {', '.join(entries)}")
        else:
            print("  binfmt_misc: nessuna voce uos-* (i file .exe/Mach-O non partono con ./programma)")
    else:
        print("  binfmt_misc: non montato")
    return 0 if missing == 0 else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="uos", description="Avvia programmi Linux, Windows e macOS.")
    parser.add_argument("--version", action="version", version=f"uos {__version__}")
    sub = parser.add_subparsers(dest="command", required=True)

    run = sub.add_parser("run", help="avvia un programma con lo strato di compatibilità giusto")
    run.add_argument("--isolate", action="store_true", help="usa un prefisso Wine dedicato a questo programma")
    run.add_argument("--dry-run", action="store_true", help="mostra il comando senza eseguirlo")
    run.add_argument("program")
    run.add_argument("args", nargs=argparse.REMAINDER)
    run.set_defaults(func=cmd_run)

    info = sub.add_parser("info", help="mostra formato, architettura e runner di un programma")
    info.add_argument("program")
    info.set_defaults(func=cmd_info)

    doctor = sub.add_parser("doctor", help="verifica quali strati di compatibilità sono installati")
    doctor.set_defaults(func=cmd_doctor)

    ns = parser.parse_args(argv)
    return ns.func(ns)
