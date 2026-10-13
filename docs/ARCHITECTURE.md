# UniOS architecture

## Why a Linux base

A system that runs programs from three platforms has to give each of them the
APIs it expects. Linux is the only one of the three bases where free and mature
reimplementations of the other two already exist (Wine for Win32, Darling for
Darwin/macOS), and its kernel is open source and modifiable. Writing a new kernel
would not bring the goal any closer: the hard work is in the API layers, not in
the kernel.

```
 ┌──────────────┬──────────────────┬──────────────────┐
 │ Linux apps   │ Windows apps     │ macOS apps       │
 │ (ELF)        │ (PE, .msi)       │ (Mach-O, .app)   │
 ├──────────────┼──────────────────┼──────────────────┤
 │              │ Wine             │ Darling          │
 │   glibc      │ (Win32 → POSIX)  │ (Darwin → Linux) │
 ├──────────────┴──────────────────┴──────────────────┤
 │      uos: dispatcher (recognises the format)       │
 │   binfmt_misc · MIME associations · uos run        │
 ├────────────────────────────────────────────────────┤
 │ box64 / FEX / qemu-user (only if the CPU differs)  │
 ├────────────────────────────────────────────────────┤
 │                  Linux kernel                      │
 └────────────────────────────────────────────────────┘
```

## The `uos` dispatcher

A single Rust binary; the code is in `src/`:

- `detect.rs` reads the first 4 KiB of the file (and the last 512 bytes) and
  derives family and architecture:
  - **ELF**: `\x7fELF`, architecture from `e_machine`.
  - **PE**: `MZ` → `e_lfanew` → `PE\0\0`, architecture from the `Machine` field;
    it also recognises DLLs, the subsystem (GUI/console) and .NET assemblies
    (CLR directory).
  - **DOS**: `MZ` without a PE header.
  - **MSI**: OLE Compound File signature.
  - **Mach-O**: `feedface`/`feedfacf` in both endiannesses; universal binaries
    `cafebabe`, told apart from Java classes by the next field (number of
    slices < 30 versus Java major version ≥ 45).
  - **.pkg**: `xar!` archive. **.dmg**: UDIF images end with a `koly` trailer.
  - **.app bundles**: reads `Contents/Info.plist` (XML or binary) to find
    `CFBundleExecutable`.
- `runners.rs` picks the layer and builds the command line, adding a CPU
  translator when the program's architecture is not the host's (e.g. an x86_64
  program on ARM → box64).
- `macos.rs` handles macOS containers: it extracts `.dmg` images with 7-Zip into
  `~/.cache/uos/dmg/`, only once per file; if 7-Zip produces the partition image
  (`.hfs`/`.apfs`) it extracts that a second time. It then looks for `.app`
  bundles without descending into them (they contain helpers) and ignoring the
  link to `/Applications` found in almost every DMG. It also extracts the largest
  PNG icon from `.icns` files.
- `profiles.rs` loads the TOML profiles and picks the one matching the program.
- `install.rs` installs programs into `~/Applications` and creates the
  `uos-*.desktop` menu entries; `uos uninstall` only removes what sits in
  `~/Applications` and in uos's icon folder.
- `cli.rs` exposes `uos run`, `info`, `install`, `list`, `uninstall`, `profile`,
  `doctor`.
- `host.rs` and `paths.rs` isolate everything that depends on the machine
  (installed tools, CPU, XDG directories), so the tests run against a fake host
  in a temporary directory.

`.pkg` packages are handed to `darling shell installer`, which also runs the
installation scripts they contain.

## System integration

- **binfmt_misc** (`system/binfmt.d/uos.conf`): the kernel hands every executed
  PE or Mach-O file to `uos`, so `./setup.exe` works from the terminal like a
  Linux program. ELF binaries of other architectures are already handled by
  `qemu-user-binfmt`.
- **MIME associations** (`system/applications/uos-run.desktop`): double-clicking
  in the file manager opens Windows and macOS executables with `uos run`.
- **Wine prefixes**: a shared one by default, one per program with `--isolate`,
  or one named by the profile, under `~/.local/share/uos/prefixes/`.
- **Context menu**: "Install with UniOS" on executables, `.dmg` and `.pkg` files.

## Profiles

Search order: `~/.config/uos/profiles/*.toml`, then `/etc/uos/profiles/*.toml`;
the first match wins. A profile without a `[match]` section never applies by
itself, but it can be forced with `uos run --profile NAME`. Every criterion in
`[match]` must hold. Unknown keys and wrong types are reported as errors.

| `[run]` key      | Effect                                                  |
|------------------|---------------------------------------------------------|
| `prefix`         | named Wine prefix (can be shared between programs)      |
| `isolate`        | prefix dedicated to the program                         |
| `wine`           | alternative Wine executable (e.g. Proton, Wine-GE)      |
| `args`           | arguments added before the user's ones                  |
| `env`            | environment variables                                   |
| `dll_overrides`  | becomes `WINEDLLOVERRIDES`                              |
| `emulator`       | preferred CPU translator (box64, FEXInterpreter, …)     |

## The distribution

`build/build-iso.sh` uses `live-build` on Debian 13 "trixie":

- `config/package-lists/`: KDE desktop, Calamares installer, Wine, DOSBox,
  qemu-user, 7-Zip, Flatpak, Java.
- `config/hooks/live/0100-wine-i386.hook.chroot`: enables i386 multiarch for
  32-bit Windows programs.
- `config/hooks/live/0200-uos.hook.chroot`: MIME associations, Flathub, the
  boot-time self-test, and removal of Wine's binfmt entry in favour of uos's.
- `config/packages.chroot/`: extra `.deb` packages: the `unios` package itself
  and, optionally, Darling.

### Automatic testing

`build/ci/boot-test.sh` boots the ISO in QEMU with the `uos.selftest` kernel
parameter. The `uos-selftest` service then runs `uos doctor`, a Linux program
and a real Windows program (Wine's `hostname.exe`) through `uos`, writes the
results to the kernel log (which reaches the serial console even when a getty
takes over the port) and powers the machine off.

## ARM64

`ARCH=arm64 ./build/build-iso.sh` (or `make iso-arm64`) builds an ISO for ARM64
PCs with UEFI. ARM64 Linux programs run natively, x86_64 Linux ones under
box64, and Debian's arm64 Wine runs ARM64 Windows programs.

x86/x86_64 Windows programs, i.e. nearly all of them, need a **Wine built for
x86_64** running under box64: Debian's arm64 Wine cannot be translated by box64.
`uos` looks for it in `/opt/uos/wine-x86_64/bin/wine` (or in the `UOS_X86_WINE`
variable). To include it in the ISO, extract an x86_64 Wine build into
`build/live/config/includes.chroot/opt/uos/wine-x86_64/` before building.

## Darling

Darling is not in the Debian archive. To include it in the ISO:

```sh
git clone --recursive https://github.com/darlinghq/darling.git
cd darling
# follow "Building as a Debian package" in Darling's documentation
cp ../darling*.deb /path/to/UniOS/build/live/config/packages.chroot/
```

Darling mounts the host filesystem under `/Volumes/SystemRoot`: `uos`
translates paths automatically before handing the program to `darling shell`.
