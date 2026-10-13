# UniversalOS

Un sistema operativo che esegue programmi **Linux, Windows e macOS** dallo stesso
desktop: fai doppio clic su un `.exe`, su un binario Mac o su un programma Linux e
UniversalOS sceglie da solo come avviarlo.

```
$ uos run Setup.exe             # → Wine
$ uos run TextEdit.app          # → Darling
$ uos install Firefox.dmg       # → estrae la .app, la copia in ~/Applications e la mette nel menu
$ uos run ./programma-linux     # → nativo
$ ./gioco.exe                   # funziona anche così, grazie a binfmt_misc
```

## Installazione

Scarica la ISO dalla pagina
[Releases](https://github.com/SoloB3n/UniversalOS/releases), scrivila su una
chiavetta USB (balenaEtcher, Ventoy o `dd`) e avvia il PC da lì: puoi provare il
sistema in modalità live e poi installarlo su disco con **Installa sistema**.
Ogni release contiene le istruzioni passo passo.

Su Debian o Ubuntu già installati basta il pacchetto:

```sh
sudo apt install ./universalos_<versione>_all.deb
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
| macOS (installer)    | `.dmg`, `.pkg`                    | 7-Zip + Darling                 |
| Java                 | `.class`                          | JRE                             |

Il formato si ricava dai **magic bytes** del file, mai dall'estensione: un `.exe`
rinominato in `.jpg` viene comunque riconosciuto come programma Windows.

Dettagli in [docs/ARCHITETTURA.md](docs/ARCHITETTURA.md).

## Uso

```sh
uos run [--isolate] [--profile NOME] [--dry-run] PROGRAMMA [ARGOMENTI...]
uos info PROGRAMMA          # formato, architettura, profilo e strato scelto
uos install PROGRAMMA       # installa e aggiunge al menu delle applicazioni
uos list                    # programmi installati con uos
uos uninstall NOME          # li rimuove (programma, icona e voce di menu)
uos profile [--new] PROGRAMMA   # mostra o crea il profilo di un programma
uos doctor                  # quali strati di compatibilità sono installati
```

`--isolate` dà al programma Windows un prefisso Wine dedicato
(`~/.local/share/uos/prefixes/<nome>`), utile quando due programmi richiedono
configurazioni incompatibili.

### Installare programmi

| File                    | Cosa fa `uos install`                                          |
|-------------------------|----------------------------------------------------------------|
| `.dmg`                  | estrae l'immagine e installa la `.app` (o il `.pkg`) contenuta |
| `.app`                  | la copia in `~/Applications`, crea la voce di menu con l'icona |
| AppImage / binario ELF  | lo copia in `~/Applications` e crea la voce di menu            |
| `.exe` / `.msi`         | avvia l'installer con Wine (che crea da sé le voci di menu)    |
| `.pkg`                  | lo installa con l'installer di Darling                         |

Dal file manager basta il clic destro → **Installa con UniversalOS**.

### Profili per programma

Un file TOML in `~/.config/uos/profiles/` (o `/etc/uos/profiles/`) applica
impostazioni dedicate a un programma, riconosciuto per nome, hash SHA-256 o
identificativo del bundle: prefisso Wine, versione di Wine/Proton, argomenti,
variabili d'ambiente, override delle DLL (es. DXVK), traduttore CPU preferito.

```toml
[match]
name = "MioGioco*.exe"

[run]
prefix = "giochi"
env = { DXVK_HUD = "fps" }
dll_overrides = { d3d11 = "n,b", dxgi = "n,b" }
```

`uos profile --new gioco.exe` crea un modello da completare; un esempio
commentato è in [docs/esempi/gioco-dxvk.toml](docs/esempi/gioco-dxvk.toml).

## Dai sorgenti

```sh
make deb                                    # → build/out/universalos_<versione>_all.deb
sudo apt install ./build/out/universalos_*_all.deb
sudo apt install wine 7zip dosbox qemu-user # strati di compatibilità facoltativi
uos doctor
```

## Generare la ISO

Su un host Debian/Ubuntu con `live-build`:

```sh
sudo apt install live-build
make iso            # PC x86_64      → build/out/amd64/*.iso
make iso-arm64      # PC ARM64 UEFI  → build/out/arm64/*.iso (sperimentale)
```

La ISO viene costruita anche da GitHub Actions (`.github/workflows/ci.yml`):
avviando il workflow a mano o creando un tag `v*`, la ISO viene generata in un
container Debian, **collaudata in QEMU** e pubblicata nelle Releases. Il collaudo
avvia il sistema con il parametro `uos.selftest`, che esegue un vero programma
Windows (`hostname.exe` di Wine) attraverso `uos` e spegne la macchina.

La ISO include KDE Plasma, l'installer Calamares, Wine (32 e 64 bit), DOSBox,
qemu-user, 7-Zip, Flatpak, Java, Mono e il dispatcher `uos`. La versione ARM64
aggiunge box64; per i programmi Windows x86 serve anche un Wine x86_64 (vedi
[docs/ARCHITETTURA.md](docs/ARCHITETTURA.md#arm64)). Darling non è nei repository Debian: va compilato come
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
- Le immagini `.dmg` vengono estratte, non montate: quelle cifrate o con
  licenza da accettare all'apertura non sono supportate.
- La ISO ARM64 è sperimentale: il Wine x86_64 da eseguire sotto box64 va
  fornito a parte.

## Roadmap

- [x] Immagini `.dmg` e pacchetti `.pkg`
- [x] Profili per programma (versione di Wine/Proton, DXVK, variabili d'ambiente)
- [x] Installazione con voce nel menu e icona
- [x] Immagine ARM64 con box64 (sperimentale)
- [ ] Proton/DXVK/VKD3D preconfigurati per i giochi
- [ ] Profili condivisi dalla comunità per i programmi più diffusi
- [ ] Interfaccia grafica: gestione programmi, prefissi e profili
- [ ] Wine x86_64 per ARM64 incluso automaticamente nella ISO
- [x] Pacchetto .deb, ISO collaudata e pubblicata automaticamente
- [ ] Darling compilato e incluso automaticamente nella ISO
