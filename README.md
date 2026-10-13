# UniversalOS

Un sistema operativo che esegue programmi **Linux, Windows e macOS** dallo stesso
desktop: fai doppio clic su un `.exe`, su un binario Mac o su un programma Linux e
UniversalOS sceglie da solo come avviarlo.

```
$ uos run Setup.exe             # → Wine
$ uos run TextEdit.app          # → Darling
$ uos run ./programma-linux     # → nativo
$ ./gioco.exe                   # funziona anche così, grazie a binfmt_misc
```

## Come funziona

UniversalOS non riscrive tre sistemi operativi: si basa su un kernel Linux, che
esegue i programmi Linux in modo nativo. Per gli altri sistemi usa gli strati di
compatibilità più maturi oggi disponibili, e il dispatcher `uos` li collega tra loro:

| Programma            | Formato riconosciuto              | Strato                          |
|----------------------|-----------------------------------|---------------------------------|
| Linux                | ELF                               | nativo (box64/FEX/qemu se CPU diversa) |
| Windows              | PE32 / PE32+, `.msi`, `.bat`      | Wine (+ box64 su ARM)           |
| MS-DOS               | MZ senza intestazione PE          | DOSBox                          |
| macOS                | Mach-O, universal binary, `.app`  | Darling                         |
| Java                 | `.class`                          | JRE                             |

Il formato si ricava dai **magic bytes** del file, mai dall'estensione: un `.exe`
rinominato in `.jpg` viene comunque riconosciuto come programma Windows.

Dettagli in [docs/ARCHITETTURA.md](docs/ARCHITETTURA.md).

## Uso

```sh
uos run [--isolate] [--dry-run] PROGRAMMA [ARGOMENTI...]
uos info PROGRAMMA      # formato, architettura, strato scelto
uos doctor              # quali strati di compatibilità sono installati
```

`--isolate` dà al programma Windows un prefisso Wine dedicato
(`~/.local/share/uos/prefixes/<nome>`), utile quando due programmi richiedono
configurazioni incompatibili.

## Provarlo su una distribuzione Linux esistente

```sh
sudo apt install wine dosbox qemu-user      # strati facoltativi
sudo make install
sudo systemctl restart systemd-binfmt       # per avviare ./programma.exe direttamente
uos doctor
```

## Generare la ISO

Su un host Debian/Ubuntu con `live-build`:

```sh
sudo apt install live-build
make iso            # → build/out/*.iso (live + installer Calamares)
```

La ISO include KDE Plasma, Wine (32 e 64 bit), DOSBox, qemu-user, Flatpak, Java,
Mono e il dispatcher `uos`. Darling non è nei repository Debian: va compilato come
`.deb` e copiato in `build/live/config/packages.chroot/` prima della build.

## Test

```sh
make test
```

## Limiti attuali (da sapere)

- **Windows**: Wine esegue una grande parte dei programmi e dei giochi, ma non quelli
  che richiedono driver kernel di Windows (anti-cheat kernel-level, alcuni DRM,
  driver di periferiche).
- **macOS**: Darling è molto meno maturo di Wine. Gli strumenti a riga di comando
  funzionano, le applicazioni grafiche (Cocoa) in gran parte no. I binari solo
  Apple Silicon (arm64) non partono su PC x86_64. Inoltre la licenza di macOS non
  consente di ridistribuire i framework di Apple: Darling li reimplementa.
- Le immagini `.dmg` e i pacchetti `.pkg` non sono ancora gestiti.

## Roadmap

1. Montaggio automatico dei `.dmg` e installazione dei `.pkg`
2. Profili per programma (versione di Wine/Proton, DXVK, variabili d'ambiente)
3. Integrazione nel menu: i programmi installati con Wine/Darling compaiono come app native
4. Proton/DXVK/VKD3D preconfigurati per i giochi
5. Immagine ARM64 con box64 + Wine
