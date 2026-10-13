"""Installazione dei programmi e integrazione nel menu delle applicazioni.

* .app / .dmg       -> copiati in ~/Applications, voce nel menu con l'icona .icns
* AppImage / ELF    -> copiati in ~/Applications, voce nel menu
* .exe / .msi       -> l'installer viene eseguito con Wine (che crea da sé le voci)
* .pkg              -> installato con l'installer di Darling

Le voci create da uos sono file ``uos-*.desktop`` con la chiave
``X-UOS-Installed`` che punta al programma copiato, così ``uos uninstall``
sa cosa rimuovere.
"""

from __future__ import annotations

import os
import re
import shutil
import stat

from . import detect as d
from . import macos, runners

_DESKTOP_PREFIX = "uos-"


class InstallError(Exception):
    pass


def applications_dir() -> str:
    return os.path.expanduser("~/Applications")


def desktop_dir() -> str:
    base = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    return os.path.join(base, "applications")


def icon_dir() -> str:
    return os.path.join(runners.data_dir(), "icons")


def desktop_quote(arg: str) -> str:
    """Quoting di un argomento per la chiave Exec (Desktop Entry Specification)."""
    arg = arg.replace("%", "%%")
    if re.fullmatch(r"[A-Za-z0-9_./+:@=,-]+", arg):
        return arg
    escaped = re.sub(r'(["`$\\])', r"\\\1", arg)
    return f'"{escaped}"'


def _slug(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-") or "app"


def write_desktop_entry(slug: str, name: str, target: str, icon: str | None,
                        comment: str) -> str:
    os.makedirs(desktop_dir(), exist_ok=True)
    path = os.path.join(desktop_dir(), f"{_DESKTOP_PREFIX}{slug}.desktop")
    lines = [
        "[Desktop Entry]",
        "Type=Application",
        f"Name={name}",
        f"Comment={comment}",
        f"Exec=uos run {desktop_quote(target)} %U",
        f"Icon={icon or 'application-x-executable'}",
        "Terminal=false",
        "Categories=Utility;",
        f"X-UOS-Installed={target}",
        "",
    ]
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))
    return path


def _copy_into_applications(src: str) -> str:
    os.makedirs(applications_dir(), exist_ok=True)
    dest = os.path.join(applications_dir(), os.path.basename(src.rstrip("/")))
    if os.path.abspath(src) == os.path.abspath(dest):
        return dest
    if os.path.exists(dest):
        raise InstallError(f"{dest} esiste già: disinstallalo prima con uos uninstall")
    if os.path.isdir(src):
        shutil.copytree(src, dest, symlinks=True)
    else:
        shutil.copy2(src, dest)
        os.chmod(dest, os.stat(dest).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return dest


def _install_app_bundle(app: str) -> str:
    info = d.detect(app)
    if info.family != d.MAC_APP:
        raise InstallError(f"{app} non è un bundle .app valido")
    dest = _copy_into_applications(app)
    name = info.details.get("name") or os.path.splitext(os.path.basename(app))[0]
    slug = _slug(info.details.get("bundle_id") or name)
    icon = None
    png = macos.bundle_icon(dest)
    if png:
        os.makedirs(icon_dir(), exist_ok=True)
        icon = os.path.join(icon_dir(), f"{slug}.png")
        with open(icon, "wb") as f:
            f.write(png)
    return write_desktop_entry(slug, name, dest, icon, "Applicazione macOS (Darling)")


def _install_linux_binary(path: str) -> str:
    dest = _copy_into_applications(path)
    base = os.path.basename(dest)
    name = re.sub(r"(?i)[-_.]?(x86_64|amd64|aarch64|arm64)?\.appimage$", "", base) or base
    return write_desktop_entry(_slug(name), name, dest, None, "Applicazione Linux")


def install(path: str) -> runners.Plan | str:
    """Installa ``path``.

    Restituisce il percorso della voce di menu creata, oppure un
    :class:`runners.Plan` quando l'installazione consiste nell'eseguire un
    installer (Wine o Darling), che il chiamante deve avviare.
    """
    info = d.detect(path)
    if info.family == d.MAC_DMG:
        return install(macos.resolve_dmg(path))
    if info.family == d.MAC_APP:
        return _install_app_bundle(path)
    if info.family in (d.PE, d.MSI, d.MAC_PKG):
        return runners.plan(info, [])
    if info.family == d.ELF:
        return _install_linux_binary(path)
    raise InstallError(f"non so come installare un file di tipo '{info.family}'")


def installed() -> list[dict]:
    """Programmi installati da uos (nome, voce di menu, percorso)."""
    out = []
    directory = desktop_dir()
    if not os.path.isdir(directory):
        return out
    for fn in sorted(os.listdir(directory)):
        if not (fn.startswith(_DESKTOP_PREFIX) and fn.endswith(".desktop")):
            continue
        entry = {"id": fn[len(_DESKTOP_PREFIX):-len(".desktop")],
                 "desktop": os.path.join(directory, fn)}
        with open(entry["desktop"], encoding="utf-8") as f:
            for line in f:
                key, _, value = line.rstrip("\n").partition("=")
                if key in ("Name", "Icon", "X-UOS-Installed"):
                    entry[key] = value
        out.append(entry)
    return out


def uninstall(ident: str) -> list[str]:
    """Rimuove voce di menu, icona e programma copiato. Restituisce i percorsi rimossi."""
    matches = [e for e in installed()
               if ident in (e["id"], e.get("Name"), os.path.basename(e.get("X-UOS-Installed", "")))]
    if not matches:
        raise InstallError(f"nessun programma installato corrisponde a '{ident}' (vedi uos list)")
    if len(matches) > 1:
        raise InstallError(f"'{ident}' è ambiguo: {', '.join(e['id'] for e in matches)}")
    entry = matches[0]
    removed = []
    target = entry.get("X-UOS-Installed", "")
    # Rimuove solo ciò che sta dentro ~/Applications, mai file altrove
    if target and os.path.dirname(os.path.abspath(target)) == applications_dir():
        if os.path.isdir(target) and not os.path.islink(target):
            shutil.rmtree(target)
        elif os.path.lexists(target):
            os.remove(target)
        removed.append(target)
    icon = entry.get("Icon", "")
    if icon.startswith(icon_dir() + os.sep) and os.path.exists(icon):
        os.remove(icon)
        removed.append(icon)
    os.remove(entry["desktop"])
    removed.append(entry["desktop"])
    return removed
