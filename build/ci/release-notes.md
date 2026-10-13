## Installing UniOS @VERSION@

1. Download `@NAME@` (and `SHA256SUMS` to verify it: `sha256sum -c SHA256SUMS --ignore-missing`).
2. Write it to a USB stick of at least 8 GB with
   [balenaEtcher](https://etcher.balena.io/), [Ventoy](https://www.ventoy.net/) or, from Linux,
   `sudo dd if=@NAME@ of=/dev/sdX bs=4M status=progress oflag=sync`
   (`/dev/sdX` is the stick: everything on it will be erased).
3. Boot the PC from the stick (firmware boot menu, usually F12, F11 or Esc).
   If it does not start, temporarily disable Secure Boot in the firmware settings.
4. The live session opens: you can try the system without installing it.
   To install it to disk, open **Install System** (Calamares) from the menu.

It can also be tried in a virtual machine (VirtualBox, GNOME Boxes, virt-manager):
at least 4 GB of RAM, 30 GB of disk, EFI enabled. In VirtualBox:

- tick **Skip Unattended Installation** when creating the machine (its unattended
  mode only knows the stock Debian/Ubuntu installers);
- if the screen stays black after a `vmwgfx ... unsupported hypervisor` message, set
  **Settings → Display → Graphics Controller** to **VBoxSVGA** and disable 3D acceleration;
- the live session user is `user`, password `live`;
- after installing, remove the ISO from **Settings → Storage** before rebooting.

### On an existing Debian 13 or Ubuntu 24.04+

```sh
sudo apt install ./unios_@VERSION@_amd64.deb     # or _arm64.deb
sudo apt install wine 7zip dosbox                # compatibility layers
uos doctor
```

### What's inside

KDE Plasma, Wine (32- and 64-bit Windows programs), DOSBox, qemu-user, 7-Zip, Flatpak
with Flathub, Java and the `uos` dispatcher. Every ISO is tested automatically: it is booted
in a virtual machine, which runs a real Windows program through `uos` and Wine.

**Not included yet:** Darling (macOS programs), which has to be built separately.

<!-- parts -->
### ISO split into parts

The ISO is larger than the 2 GB GitHub allows for a single file, so it is split into parts.
Download all of them and join them before writing the stick:

- Linux / macOS: `cat @NAME@.part* > @NAME@`
- Windows (Command Prompt): `copy /b @NAME@.part0 + @NAME@.part1 @NAME@`

Then verify it with `sha256sum -c SHA256SUMS --ignore-missing`.
<!-- /parts -->
