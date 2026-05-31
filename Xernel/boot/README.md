# Boot-Assets

Diese Verzeichnis enthält Bootloader-Konfiguration und vom `xtask` zur Laufzeit
zusammengezogene Binärassets.

- `limine.cfg` — Limine-Bootloader-Konfiguration (BIOS + UEFI).
- `limine/` *(nicht im Repo)* — wird vom `xtask` per `git clone` der
  `binary`-Branch des Limine-Repos initialisiert. Das ist Absicht: Limine-Binaries
  sind versioniert in einem separaten Branch und werden nicht im Source-Tree
  gespiegelt.

Erster Lauf:

```sh
cargo xtask fetch-limine
```
