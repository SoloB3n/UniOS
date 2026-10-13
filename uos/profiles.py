"""Profili per programma: impostazioni dedicate in file TOML.

I profili si cercano in ``~/.config/uos/profiles/`` e poi in
``/etc/uos/profiles/`` (il primo che corrisponde vince; a parità di
directory conta l'ordine alfabetico dei file). Esempio::

    # ~/.config/uos/profiles/notepadpp.toml
    [match]
    name = "notepad++*.exe"      # glob sul nome del file (senza maiuscole)
    # sha256 = "…"               # oppure l'hash esatto
    # bundle_id = "com.foo.bar"  # oppure l'identificativo di una .app

    [run]
    prefix = "editor"            # prefisso Wine con nome
    wine = "/opt/proton/files/bin/wine"
    args = ["-multiInst"]
    env = { DXVK_HUD = "fps" }
    dll_overrides = { d3d11 = "n,b", dxgi = "n,b" }
    emulator = "FEXInterpreter"  # traduttore CPU da preferire
"""

from __future__ import annotations

import fnmatch
import hashlib
import os
import tomllib
from dataclasses import dataclass, field

from . import detect as d

SYSTEM_DIR = "/etc/uos/profiles"


class ProfileError(Exception):
    pass


@dataclass
class Profile:
    name: str
    source: str = ""
    match: dict = field(default_factory=dict)
    prefix: str | None = None
    isolate: bool = False
    wine: str | None = None
    emulator: str | None = None
    args: list[str] = field(default_factory=list)
    env: dict[str, str] = field(default_factory=dict)
    dll_overrides: dict[str, str] = field(default_factory=dict)

    def wine_dll_overrides(self) -> str | None:
        if not self.dll_overrides:
            return None
        return ";".join(f"{dll}={mode}" for dll, mode in sorted(self.dll_overrides.items()))


def user_dir() -> str:
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")
    return os.path.join(base, "uos", "profiles")


def search_dirs() -> list[str]:
    return [user_dir(), SYSTEM_DIR]


_RUN_KEYS = {"prefix": str, "isolate": bool, "wine": str, "emulator": str,
             "args": list, "env": dict, "dll_overrides": dict}


def load(path: str) -> Profile:
    try:
        with open(path, "rb") as f:
            data = tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError) as e:
        raise ProfileError(f"{path}: {e}") from e
    run = data.get("run", {})
    for key, value in run.items():
        expected = _RUN_KEYS.get(key)
        if expected is None:
            raise ProfileError(f"{path}: chiave sconosciuta [run].{key}")
        if not isinstance(value, expected):
            raise ProfileError(f"{path}: [run].{key} deve essere di tipo {expected.__name__}")
    name = os.path.splitext(os.path.basename(path))[0]
    return Profile(
        name=name, source=path, match=data.get("match", {}),
        args=[str(a) for a in run.get("args", [])],
        env={k: str(v) for k, v in run.get("env", {}).items()},
        dll_overrides={k: str(v) for k, v in run.get("dll_overrides", {}).items()},
        **{k: run[k] for k in ("prefix", "isolate", "wine", "emulator") if k in run},
    )


def _sha256(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def matches(profile: Profile, info: d.BinaryInfo) -> bool:
    m = profile.match
    if not m:
        return False
    if "name" in m and not fnmatch.fnmatch(os.path.basename(info.path).lower(), m["name"].lower()):
        return False
    if "bundle_id" in m and info.details.get("bundle_id") != m["bundle_id"]:
        return False
    if "sha256" in m:
        if os.path.isdir(info.path) or _sha256(info.path) != m["sha256"].lower():
            return False
    return True


def _all_profiles():
    for directory in search_dirs():
        if not os.path.isdir(directory):
            continue
        for fn in sorted(os.listdir(directory)):
            if fn.endswith(".toml"):
                yield os.path.join(directory, fn)


def find(info: d.BinaryInfo) -> Profile | None:
    for path in _all_profiles():
        profile = load(path)
        if matches(profile, info):
            return profile
    return None


def by_name(name: str) -> Profile:
    for directory in search_dirs():
        path = os.path.join(directory, name + ".toml")
        if os.path.isfile(path):
            return load(path)
    raise ProfileError(f"profilo '{name}' non trovato in {', '.join(search_dirs())}")


def template(info: d.BinaryInfo) -> str:
    """Testo di un nuovo profilo che corrisponde a ``info``."""
    lines = ["[match]"]
    if info.details.get("bundle_id"):
        lines.append(f'bundle_id = "{info.details["bundle_id"]}"')
    else:
        lines.append(f'name = "{os.path.basename(info.path)}"')
    lines += [
        "",
        "[run]",
        "# prefix = \"" + (os.path.splitext(os.path.basename(info.path))[0] or "app") + "\"",
        "# args = []",
        "# env = { }",
        "# dll_overrides = { }",
        "# wine = \"/percorso/di/wine\"",
        "# emulator = \"box64\"",
        "",
    ]
    return "\n".join(lines)
