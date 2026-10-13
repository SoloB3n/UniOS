"""Contenitori macOS: immagini .dmg, bundle .app e icone .icns.

Le immagini .dmg non vengono montate (servirebbero privilegi di root e il
supporto APFS/HFS+ nel kernel): vengono estratte con 7-Zip in una cache per
utente, da cui si avviano o si installano le .app contenute.
"""

from __future__ import annotations

import hashlib
import os
import shutil
import struct
import subprocess



class MacError(Exception):
    pass


def cache_dir() -> str:
    base = os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache")
    return os.path.join(base, "uos", "dmg")


def _seven_zip() -> str:
    # 7zz/7z gestiscono DMG, HFS+ e APFS; 7za/7zr no.
    for name in ("7zz", "7z"):
        p = shutil.which(name)
        if p:
            return p
    raise MacError("7-Zip non è installato: impossibile aprire immagini .dmg")


def _cache_key(path: str) -> str:
    st = os.stat(path)
    raw = f"{os.path.abspath(path)}:{st.st_size}:{st.st_mtime_ns}"
    name = os.path.splitext(os.path.basename(path))[0]
    return f"{name}-{hashlib.sha256(raw.encode()).hexdigest()[:12]}"


_VOLUME_IMAGES = (".hfs", ".hfsx", ".apfs", ".img")


def extract_dmg(path: str) -> str:
    """Estrae ``path`` nella cache (una volta sola) e restituisce la directory."""
    dest = os.path.join(cache_dir(), _cache_key(path))
    done = os.path.join(dest, ".uos-complete")
    if os.path.exists(done):
        return dest

    seven = _seven_zip()
    shutil.rmtree(dest, ignore_errors=True)
    os.makedirs(dest)
    _run_7z(seven, path, dest)

    # Alcune versioni di 7-Zip estraggono la partizione come file .hfs/.apfs
    # invece del suo contenuto: in quel caso va estratta una seconda volta.
    entries = [e for e in os.listdir(dest) if not e.startswith(".")]
    if len(entries) == 1 and entries[0].lower().endswith(_VOLUME_IMAGES):
        inner = os.path.join(dest, entries[0])
        _run_7z(seven, inner, dest)
        os.remove(inner)

    open(done, "w").close()
    return dest


def _run_7z(seven: str, archive: str, dest: str) -> None:
    r = subprocess.run([seven, "x", "-y", f"-o{dest}", archive],
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    if r.returncode != 0:
        raise MacError(f"estrazione di {os.path.basename(archive)} fallita: {r.stderr.strip()[-200:]}")


def find_contents(root: str) -> tuple[list[str], list[str]]:
    """Restituisce (bundle .app, pacchetti .pkg) trovati sotto ``root``.

    Non scende dentro le .app (che contengono a loro volta helper .app) e
    ignora i link simbolici, come quello ad /Applications presente in molti DMG.
    """
    apps, pkgs = [], []
    for dirpath, dirnames, filenames in os.walk(root):
        keep = []
        for dn in sorted(dirnames):
            full = os.path.join(dirpath, dn)
            if os.path.islink(full) or dn.startswith("."):
                continue
            if dn.endswith(".app"):
                apps.append(full)
            elif dn.endswith(".pkg"):  # pacchetti "bundle" (directory)
                pkgs.append(full)
            else:
                keep.append(dn)
        dirnames[:] = keep
        pkgs += [os.path.join(dirpath, f) for f in sorted(filenames)
                 if f.endswith((".pkg", ".mpkg"))]
    return apps, pkgs


def resolve_dmg(path: str) -> str:
    """Estrae il DMG e restituisce l'unica .app o .pkg che contiene."""
    apps, pkgs = find_contents(extract_dmg(path))
    candidates = apps or pkgs
    if not candidates:
        raise MacError("l'immagine .dmg non contiene applicazioni né pacchetti .pkg")
    if len(candidates) > 1:
        names = ", ".join(os.path.basename(c) for c in candidates)
        raise MacError(f"l'immagine contiene più programmi ({names}): avviane uno con uos run <percorso>")
    return candidates[0]


# --- Icone ------------------------------------------------------------------

_PNG_MAGIC = b"\x89PNG\r\n\x1a\n"


def icns_largest_png(data: bytes) -> bytes | None:
    """Estrae la PNG più grande da un file .icns (icone moderne: ic07..ic14)."""
    if data[:4] != b"icns":
        return None
    best: tuple[int, bytes] | None = None
    off, end = 8, min(len(data), struct.unpack(">I", data[4:8])[0])
    while off + 8 <= end:
        _kind, size = struct.unpack(">4sI", data[off:off + 8])
        if size < 8:
            break
        chunk = data[off + 8:off + size]
        if chunk.startswith(_PNG_MAGIC) and len(chunk) >= 24:
            width = struct.unpack(">I", chunk[16:20])[0]
            if best is None or width > best[0]:
                best = (width, chunk)
        off += size
    return best[1] if best else None


def bundle_icon(app: str) -> bytes | None:
    """PNG dell'icona di una .app, se ricavabile da Contents/Resources."""
    import plistlib

    contents = os.path.join(app, "Contents")
    try:
        with open(os.path.join(contents, "Info.plist"), "rb") as f:
            name = plistlib.load(f).get("CFBundleIconFile") or ""
    except (OSError, ValueError):
        return None
    if not name:
        return None
    if not name.endswith(".icns"):
        name += ".icns"
    try:
        with open(os.path.join(contents, "Resources", name), "rb") as f:
            return icns_largest_png(f.read())
    except OSError:
        return None
