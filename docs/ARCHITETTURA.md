# Architettura di UniversalOS

## Perché una base Linux

Un sistema che esegua programmi di tre piattaforme deve offrire a ciascuno le API
che si aspetta. Linux è l'unica delle tre basi su cui esistono già reimplementazioni
libere e mature delle API degli altri due (Wine per Win32, Darling per Darwin/macOS),
e il suo kernel è open source e modificabile. Scrivere un kernel nuovo non
avvicinerebbe l'obiettivo: il lavoro difficile sta negli strati di API, non nel kernel.

```
 ┌──────────────┬──────────────────┬──────────────────┐
 │ App Linux    │ App Windows      │ App macOS        │
 │ (ELF)        │ (PE, .msi)       │ (Mach-O, .app)   │
 ├──────────────┼──────────────────┼──────────────────┤
 │              │ Wine             │ Darling          │
 │   glibc      │ (Win32 → POSIX)  │ (Darwin → Linux) │
 ├──────────────┴──────────────────┴──────────────────┤
 │        uos: dispatcher (riconosce il formato)      │
 │   binfmt_misc · associazioni MIME · uos run        │
 ├────────────────────────────────────────────────────┤
 │ box64 / FEX / qemu-user (solo se la CPU è diversa) │
 ├────────────────────────────────────────────────────┤
 │                 Kernel Linux                       │
 └────────────────────────────────────────────────────┘
```

## Il dispatcher `uos`

Codice in `uos/`:

- `detect.py` legge i primi 4 KiB del file e ne ricava famiglia e architettura:
  - **ELF**: `\x7fELF`, architettura da `e_machine`.
  - **PE**: `MZ` → `e_lfanew` → `PE\0\0`, architettura dal campo `Machine`;
    riconosce anche DLL, sottosistema (GUI/console) e assembly .NET (directory CLR).
  - **DOS**: `MZ` senza intestazione PE.
  - **MSI**: firma OLE Compound File.
  - **Mach-O**: `feedface`/`feedfacf` in entrambi gli endian; universal binary
    `cafebabe`, distinto dalle classi Java guardando il campo successivo
    (numero di slice < 30 contro major version Java ≥ 45).
  - **Bundle .app**: legge `Contents/Info.plist` (XML o binario) per trovare
    `CFBundleExecutable`.
- `runners.py` sceglie lo strato e costruisce la riga di comando, inserendo un
  traduttore di CPU quando l'architettura del programma non è quella dell'host
  (es. programma x86_64 su ARM → box64).
- `cli.py` espone `uos run`, `uos info`, `uos doctor`.

## Integrazione con il sistema

- **binfmt_misc** (`system/binfmt.d/uos.conf`): il kernel passa a `uos` ogni file
  PE o Mach-O eseguito, quindi `./setup.exe` funziona dal terminale come un
  programma Linux. I binari ELF di altre architetture sono già gestiti da
  `qemu-user-binfmt`.
- **Associazioni MIME** (`system/applications/uos-run.desktop`): il doppio clic
  nel file manager apre gli eseguibili Windows e macOS con `uos run`.
- **Prefissi Wine**: uno condiviso di default, uno per programma con `--isolate`,
  sotto `~/.local/share/uos/prefixes/`.

## La distribuzione

`build/build-iso.sh` usa `live-build` su Debian 13 "trixie":

- `config/package-lists/`: desktop KDE, installer Calamares, Wine, DOSBox,
  qemu-user, Flatpak, Java, Mono.
- `config/hooks/live/0100-wine-i386.hook.chroot`: abilita multiarch i386 per i
  programmi Windows a 32 bit.
- `config/hooks/live/0200-uos.hook.chroot`: associazioni MIME, Flathub, rimozione
  della voce binfmt di Wine in favore di quella di uos.
- `config/packages.chroot/`: `.deb` aggiuntivi, in particolare Darling.

## Darling

Darling non è nei repository Debian. Per includerlo nella ISO:

```sh
git clone --recursive https://github.com/darlinghq/darling.git
cd darling
# segui "Building as a Debian package" nella documentazione di Darling
cp ../darling*.deb /percorso/UniversalOS/build/live/config/packages.chroot/
```

Darling monta il filesystem dell'host sotto `/Volumes/SystemRoot`: `uos` traduce
automaticamente i percorsi prima di passare il programma a `darling shell`.
