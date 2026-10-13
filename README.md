# UniOS

An operating system that runs **Linux, Windows and macOS** programs from the same
desktop: double-click an `.exe`, a Mac binary or a Linux program and UniOS works
out by itself how to start it.

```
$ uos run Setup.exe             # → Wine
$ uos run TextEdit.app          # → Darling
$ uos install Firefox.dmg       # → extracts the .app, copies it to ~/Applications and adds it to the menu
$ uos run ./linux-program       # → native
$ ./game.exe                    # works too, thanks to binfmt_misc
```

## Installation

Download the ISO from the [Releases](https://github.com/SoloB3n/UniOS/releases)
page, write it to a USB stick (balenaEtcher, Ventoy or `dd`) and boot the PC from
it: you can try the system in live mode and then install it to disk with
**Install System**. Every release has step-by-step instructions.

On an existing Debian 13 or Ubuntu 24.04+, the package is enough:

```sh
sudo apt install ./unios_<version>_amd64.deb     # or _arm64.deb
```

## How it works

UniOS does not rewrite three operating systems: it is built on a Linux kernel,
which runs Linux programs natively. For the other systems it uses the most mature
compatibility layers available today, and the `uos` dispatcher ties them together:

| Program              | Recognised format                 | Layer                                  |
|----------------------|-----------------------------------|----------------------------------------|
| Linux                | ELF                               | native (box64/FEX/qemu if the CPU differs) |
| Windows              | PE32 / PE32+, `.msi`, `.bat`      | Wine (+ box64 on ARM)                  |
| MS-DOS               | MZ without a PE header            | DOSBox                                 |
| macOS                | Mach-O, universal binary, `.app`  | Darling                                |
| macOS (installers)   | `.dmg`, `.pkg`                    | 7-Zip + Darling                        |
| Java                 | `.class`                          | JRE                                    |

The format comes from the file's **magic bytes**, never from its extension: an
`.exe` renamed to `.jpg` is still recognised as a Windows program.

`uos` is a single Rust binary with no runtime dependencies besides glibc.
Details in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Usage

```sh
uos run [--isolate] [--profile NAME] [--dry-run] PROGRAM [ARGS...]
uos info PROGRAM            # format, architecture, profile and chosen layer
uos install PROGRAM         # install and add to the applications menu
uos list                    # programs installed with uos
uos uninstall NAME          # remove them (program, icon and menu entry)
uos profile [--new] PROGRAM # show or create a program's profile
uos doctor                  # which compatibility layers are installed
```

Everything after `PROGRAM` is passed to the program untouched, options included.
`--isolate` gives the Windows program a dedicated Wine prefix
(`~/.local/share/uos/prefixes/<name>`), useful when two programs need
incompatible settings.

### Installing programs

| File                    | What `uos install` does                                        |
|-------------------------|----------------------------------------------------------------|
| `.dmg`                  | extracts the image and installs the `.app` (or `.pkg`) inside  |
| `.app`                  | copies it to `~/Applications`, creates the menu entry with its icon |
| AppImage / ELF binary   | copies it to `~/Applications` and creates the menu entry       |
| `.exe` / `.msi`         | runs the installer with Wine (which creates its own menu entries) |
| `.pkg`                  | installs it with Darling's installer                           |

In the file manager, right-click → **Install with UniOS**.

### Per-program profiles

A TOML file in `~/.config/uos/profiles/` (or `/etc/uos/profiles/`) applies
dedicated settings to a program, recognised by name, SHA-256 hash or bundle
identifier: Wine prefix, Wine/Proton version, arguments, environment variables,
DLL overrides (e.g. DXVK), preferred CPU translator.

```toml
[match]
name = "MyGame*.exe"

[run]
prefix = "games"
env = { DXVK_HUD = "fps" }
dll_overrides = { d3d11 = "n,b", dxgi = "n,b" }
```

`uos profile --new game.exe` creates a template to fill in; a commented example
is in [docs/examples/game-dxvk.toml](docs/examples/game-dxvk.toml).

## Building from source

Requirements: Rust (stable, 1.85 or newer) and, for the package, `dpkg-deb`.

```sh
make build                                  # → target/release/uos
make test                                   # unit and CLI tests
make lint                                   # rustfmt + clippy
make deb                                    # → build/out/unios_<version>_<arch>.deb
sudo apt install ./build/out/unios_*.deb
sudo apt install wine 7zip dosbox qemu-user # optional compatibility layers
uos doctor
```

## Building the ISO

On a Debian/Ubuntu host with `live-build`:

```sh
sudo apt install live-build
make iso            # x86_64 PCs      → build/out/amd64/*.iso
make iso-arm64      # ARM64 UEFI PCs  → build/out/arm64/*.iso (experimental)
```

GitHub Actions (`.github/workflows/ci.yml`) builds it too: on every push it runs
the tests and builds the `.deb` for amd64 and arm64; when the workflow is started
by hand or a `v*` tag is pushed, the ISO is built in a Debian container,
**tested in QEMU** and published to Releases. The test boots the system with the
`uos.selftest` parameter, which runs a real Windows program (Wine's
`hostname.exe`) through `uos` and powers the machine off.

The ISO includes KDE Plasma, the Calamares installer, Wine (32- and 64-bit),
DOSBox, qemu-user, 7-Zip, Flatpak, Java and the `uos` dispatcher. The live
session starts in Italian (change `--bootappend-live` in `build/build-iso.sh`
for another language). The ARM64 version adds box64; x86 Windows programs also
need an x86_64 Wine (see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#arm64)).
Darling is not in the Debian archive: it has to be built as a `.deb` and copied
to `build/live/config/packages.chroot/` before the build.

## Current limitations

- **Windows**: Wine runs a large share of programs and games, but not the ones
  that need Windows kernel drivers (kernel-level anti-cheat, some DRM, device
  drivers).
- **macOS**: Darling is much less mature than Wine. Command-line tools work,
  graphical (Cocoa) applications mostly don't. Apple Silicon-only (arm64)
  binaries don't start on x86_64 PCs. The macOS licence does not allow
  redistributing Apple's frameworks: Darling reimplements them.
- `.dmg` images are extracted, not mounted: encrypted ones, or ones with a
  licence to accept on opening, are not supported.
- The ARM64 ISO is experimental: the x86_64 Wine to run under box64 has to be
  provided separately.

## Roadmap

- [x] `.dmg` images and `.pkg` packages
- [x] Per-program profiles (Wine/Proton version, DXVK, environment variables)
- [x] Installation with menu entry and icon
- [x] ARM64 image with box64 (experimental)
- [x] `.deb` package, ISO tested and published automatically
- [x] Dispatcher rewritten in Rust
- [ ] Proton/DXVK/VKD3D preconfigured for games
- [ ] Community-shared profiles for popular programs
- [ ] Graphical interface: manage programs, prefixes and profiles
- [ ] x86_64 Wine for ARM64 included automatically in the ISO
- [ ] Darling built and included automatically in the ISO
