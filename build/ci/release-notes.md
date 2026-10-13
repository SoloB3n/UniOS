## Installare UniOS @VERSION@

1. Scarica `@NAME@` (e `SHA256SUMS` per verificarla: `sha256sum -c SHA256SUMS`).
2. Scrivila su una chiavetta USB da almeno 8 GB con
   [balenaEtcher](https://etcher.balena.io/), [Ventoy](https://www.ventoy.net/) o, da Linux,
   `sudo dd if=@NAME@ of=/dev/sdX bs=4M status=progress oflag=sync`
   (`/dev/sdX` è la chiavetta: tutto il suo contenuto verrà cancellato).
3. Avvia il PC dalla chiavetta (menu di avvio del firmware, di solito F12, F11 o Esc).
4. Si apre la sessione live: puoi provare il sistema senza installarlo.
   Per installarlo su disco, apri **Installa sistema** (Calamares) dal menu.

Si può provare anche in una macchina virtuale (VirtualBox, GNOME Boxes, virt-manager):
almeno 4 GB di RAM e 30 GB di disco.

### Su Debian o Ubuntu già installati

```sh
sudo apt install ./unios_@VERSION@_all.deb
sudo apt install wine 7zip dosbox     # strati di compatibilità
uos doctor
```

### Cosa contiene

KDE Plasma, Wine (programmi Windows a 32 e 64 bit), DOSBox, qemu-user, 7-Zip, Flatpak
con Flathub, Java e il dispatcher `uos`. Ogni ISO è collaudata in automatico: viene avviata
in una macchina virtuale, che esegue un vero programma Windows tramite `uos` e Wine.

**Non ancora incluso:** Darling (programmi macOS), va compilato a parte.

<!-- parti -->
### ISO divisa in parti

La ISO supera i 2 GB consentiti da GitHub per un singolo file ed è divisa in parti.
Scaricale tutte e riuniscile prima di scriverle sulla chiavetta:

- Linux / macOS: `cat @NAME@.part* > @NAME@`
- Windows (Prompt dei comandi): `copy /b @NAME@.part0 + @NAME@.part1 @NAME@`

Poi verifica con `sha256sum -c SHA256SUMS`.
<!-- /parti -->
