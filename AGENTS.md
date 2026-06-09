# AGENTS.md — Handbuch für KI-Agents an Xernel

Dieses Dokument ist für **dich, den nächsten KI-Agenten**, der an Xernel
weiterarbeitet. Es fasst alles zusammen, was nicht aus dem Code allein ersichtlich
ist: die Regeln, den Build/Test-Loop, die Architektur und — am wichtigsten —
**Schritt-für-Schritt-Rezepte**, mit denen du Features hinzufügst, ohne dir das
große Ganze neu herleiten zu müssen. Lies das einmal ganz, bevor du etwas änderst.

> Faustregel: Der schwere konzeptionelle Teil (Capability-Modell, IPC, geteilter
> Speicher, der Netz-Stack) ist **schon gebaut und in QEMU verifiziert**. Deine
> Aufgabe ist meistens *additiv und nach Muster*: ein Rezept unten befolgen, bauen,
> testen, dokumentieren. Erfinde nichts neu, kopiere das nächstgelegene Vorbild.

---

## 0. Was Xernel ist (in drei Sätzen)

Xernel ist ein **Capability-basierter Mikrokernel** in Rust (x86_64, Limine-Boot),
im Stil von seL4: jede Autorität ist eine unfälschbare Capability, es gibt **keine
ambiente Berechtigung**. Treiber, Dateisystem und Netz-Stack laufen in **Ring 3**;
der Kernel liefert nur Primitive (Speicher, Adressräume, IPC, Port-/MMIO-/DMA-
Zugang hinter Capabilities). Ziel ist ein **universelles** Fundament, das überall
läuft — aber bewusst **nicht** Linux-ABI-kompatibel (eine eigene POSIX-Schicht
käme später in den User-Space).

---

## 1. Goldene Regeln (NICHT verletzen)

1. **Code nur in `Xernel/Xernel/`, Doku nur in `Xernel/docs/`.** Strikt getrennt.
2. **0 Warnungen.** `cargo` muss warnungsfrei bauen. (Ausnahme: eine *vorbestehende*
   `unused_unsafe`-Warnung in `kernel/src/arch/x86_64/syscall.rs:118` wurde bewusst
   stehen gelassen — fass sie nicht an, erzeuge aber keine neuen.)
3. **Jeder Schritt wird dokumentiert** als `.txt`-Protokoll unter
   `docs/history/` (Format → Rezept E). Reproduzierbares Erzähl-Protokoll, deutsch.
4. **Vor JEDEM `git push`: gründlicher PII-/Secret-Scan.** Siehe Abschnitt 7. Das
   ist nicht verhandelbar.
5. **Commit-Nachrichten enden mit:**
   `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`
6. **Universelle Fundamente bauen**, nicht auf einen einzelnen Nutzer (z. B.
   "NexusCloud") hin optimieren. Wenn ein externer Wunsch kommt: bau die *allgemeine*
   Form davon (Beispiel: aus "wir brauchen epoll" wurde das generische
   Notification-Primitiv, das jedem Service nützt).
7. **Sei ehrlich, nicht gefällig.** Sag, was *läuft* vs. was *produktionsreif* ist.
   Lass dich von Druck/Deadlines nicht zu Pfusch drängen.

---

## 2. Repo-Karte (wo liegt was)

Die Repo-Wurzel enthält `Xernel/` (den Cargo-Code-Workspace) und `docs/` (Doku).
Alle Pfade unten sind **relativ zur Repo-Wurzel**.

```
Xernel/                         <- der Cargo-Workspace (hier baust du)
  kernel/src/
    main.rs        Kernel-Einstieg, mod-Liste
    process.rs     Prozesse, Scheduler, Adressräume, Capability-Seeding
    syscall.rs     Syscall-Dispatch + alle Handler   <- HIER landen neue Syscalls
    cap.rs         CapEntry/CNode — das Capability-Datenmodell
    endpoint.rs    IPC-Endpoints (SEND/RECV)
    notification.rs Notification-Objekte (SIGNAL/WAIT, Readiness)
    elf.rs         ELF-Loader (lädt das init-Image)
    mm/, mm.rs     Frame-Allocator + Paging-Glue
    arch/x86_64/   die echte Architektur (paging, vspace, syscall-Entry, gdt …)
    arch/aarch64, arch/riscv64   nur Stubs (unimplemented)
  libs/
    xabi/          die ABI-KONSTANTEN, von Kernel UND Userland geteilt
                   (z. B. CapType lebt in xabi/src/cap.rs)
    xstd, xdriver, xlibc   weitere User-Libs (teils Skelett)
  userland/
    init/src/main.rs   DAS GROSSE USERLAND-PROGRAMM (~1500 Zeilen).
                       Enthält ALLES: virtio-blk-Treiber, XernelFS, Datei-Service,
                       virtio-net + TCP/IP-Stack, IPC-Client, Shared-Memory-Demo.
                       Ein Binary; per PID nimmt es eine Rolle ein
                       (pid 0 = Root/Treiber/Service, pid != 0 = Client).
    xsh/           Shell-Skelett
  xtask/src/main.rs    der Build/Run-Treiber (QEMU-Argumente, Disk-Image)
  servers/  drivers/   14-Zeilen-STUBS (rootserver, vfs, netstack, disp, pm,
                       virtio) — die geplante Mikrokernel-Zerlegung. Noch LEER;
                       heute lebt alles in userland/init.
docs/
  book/src/        mdBook: status.md (Stand), syscalls.md (die ABI) …
  history/0.X_Name/0.X.Y_Name.txt   das chronologische Protokoll
```

**Merke:** Der heutige *echte* Code ist `kernel/` + `userland/init/src/main.rs`.
Die `servers/`-Crates sind noch Platzhalter — eine spätere Aufgabe ist, den
Datei-Service und Netz-Service aus `init` in eigene Server-Crates zu ziehen.

---

## 3. Der Build/Test-Loop (auswendig lernen)

Aus dem Workspace-Verzeichnis `Xernel/` (das ist wichtig — nach `git`-Befehlen
driftet das Arbeitsverzeichnis manchmal zur Repo-Wurzel; dann zurück nach
`Xernel/` wechseln):

```sh
cd <repo>/Xernel        # das Workspace-Verzeichnis (enthält Cargo.toml)
cargo run --package xtask --release -- run --test
```

Das baut Kernel + Userland, packt eine ISO, startet QEMU mit `--test`, und das
init-Programm fährt am Ende `arch::exit(true)` → der Lauf endet von selbst. Erfolg
siehst du an der **letzten Zeile**:

```
xtask: boot-test PASSED
```

Hinweise:
- **macOS hat kein `timeout`.** Verlass dich darauf, dass `--test` selbst beendet;
  hänge kein `timeout` davor.
- Pipe durch `| tail -60` oder `| grep`, die Ausgabe ist lang.
- Ohne `--test` läuft QEMU interaktiv weiter (für manuelles Ausprobieren).
- Es gibt KEINE Linux-`std` im Kernel/Userland — `#![no_std]`, `alloc` nur über
  eigene Allokatoren.

---

## 4. Architektur in einem Bildschirm

- **Boot:** Limine lädt Kernel + das init-ELF als Modul. Kernel richtet GDT/IDT,
  Paging (HHDM), Heap, LAPIC-Timer ein.
- **Prozesse:** Der Kernel bootet **genau einen** Prozess (Root, pid 0). Jeder
  weitere entsteht per `SYS_SPAWN`, vom Root selbst (wie ein echtes init). Jeder
  hat einen eigenen Adressraum (eigene PML4), eigenen Kernel-Stack, eigene
  Capability-Tabelle.
- **Scheduling:** preemptiv (Timer) + kooperativ (`YIELD`). Blockierende Syscalls
  (`RECV`, `WAIT`) **blockieren echt** (seit 0.23.0): der Prozess geht in
  `State::Blocked(BlockReason)`, der Scheduler überspringt ihn, ein `wake` aus
  `SEND`/`SIGNAL` macht ihn wieder `Ready`. Kein Busy-Yield mehr.
- **Capabilities:** Jeder Prozess hat ein CNode mit 64 Slots. Eine `CapEntry` ist
  `{ cap_type, object, badge }`. Slot-Belegung beim Seeding (`process.rs::seed_caps`):
  | Slot | Inhalt | wer |
  |---|---|---|
  | 0 | `IoPort` (PCI-I/O-Fenster) | nur pid 0 |
  | 1 | `IoMem` (PCI-MMIO-Fenster) | nur pid 0 |
  | 2 | `Untyped` (DMA-/Frame-Budget, 256 KiB) | nur pid 0 |
  | 3 | `Endpoint 0` (Anfragen Client→Service) | alle |
  | 4 | `Endpoint 1` (Antworten Service→Client) | alle |
  | 5 | `Notification 0` (Readiness) | alle |
  | 6+ | frei (z. B. delegierte `Frame`-Caps) | — |
- **Treiber-Philosophie:** Der Kernel kennt das Wort "virtio" nicht. Er gibt nur
  `PCI_READ`, `IOMAP`, `DMA_ALLOC`, `PORT_IN/OUT` (alle cap-gated). Die ganze
  Geräte-*Policy* liegt in Ring 3.
- **Zwei IPC-Datenpfade:**
  1. *Nachrichten* (`SEND`/`RECV`): ein `u64`-Wort + optional eine Capability —
     für Steuerung/Delegation. **`SEND` kopiert die Cap** (der Sender behält sie).
  2. *Geteilter Speicher* (`FRAME_ALLOC`/`MAP_FRAME` + Frame-Cap granten): beide
     Prozesse mappen dieselbe physische Seite — für Masse-Daten.
- **Readiness** (`SIGNAL`/`WAIT`): Notification-Objekt, das epoll/kqueue-Primitiv.
  Bits werden ODER-verknüpft (gehen nie verloren), ein `WAIT` deckt viele Quellen.

Die **ABI** (der Vertrag) steht vollständig in `docs/book/src/syscalls.md`. Halte
sie aktuell — sie ist die Quelle der Wahrheit für jeden, der gegen Xernel
programmiert.

---

## 5. REZEPTE (das hier ist der Kern dieses Dokuments)

### Rezept A — Einen neuen Syscall hinzufügen

Vorbild: schau dir `SYS_FRAME_ALLOC` (Nr. 23) an, es berührt genau diese Stellen.

1. **`kernel/src/syscall.rs`**
   - Nummer als `pub const SYS_NAME: u64 = N;` (mit Doc-Kommentar) definieren.
     Nimm die nächste freie Nummer (heute zuletzt 24).
   - Im `match nr` in `dispatch()` einen Arm `SYS_NAME => sys_name(args[…]),`.
   - Die Handler-Funktion `fn sys_name(...) -> u64` schreiben. Gib bei Fehler
     `u64::MAX` zurück (Konvention). User-Puffer nur über `user_slice` /
     `user_slice_mut` anfassen (die validieren die Adresse).
2. **Gated?** Wenn der Syscall Autorität braucht, prüfe eine Capability über einen
   `crate::process::current_*`-Helper und gib bei Fehlen `u64::MAX` zurück mit
   `println!("[cap] DENY …")`. Vorbild: `sys_port_in`, `sys_frame_alloc`.
3. **`userland/init/src/main.rs`**: dieselbe Nummer als `const SYS_NAME: u64 = N;`
   und einen dünnen Wrapper `fn name(...) -> u64 { syscall3(SYS_NAME, …) }`.
4. **`docs/book/src/syscalls.md`**: eine Tabellenzeile ergänzen. Bei gated-Syscalls
   die "(13–16, 23)"-Liste erweitern.
5. **`docs/book/src/status.md`**: Syscall-Zähler hochsetzen ("24 Syscalls").
6. Bauen + testen (Abschnitt 3), dann dokumentieren (Rezept E).

### Rezept B — Einen neuen Capability-Typ verwenden/hinzufügen

Die Typen stehen in **`libs/xabi/src/cap.rs`** (`enum CapType`, von Kernel und
Userland geteilt). Mehrere sind schon reserviert, aber noch ungenutzt
(`PageTable`, `Thread`, `VSpace`, `IrqHandler`). Um einen zu aktivieren:
- In **`kernel/src/cap.rs`** einen Konstruktor `CapEntry::xyz(...)` ergänzen
  (Vorbild: `CapEntry::frame` — `object`/`badge` belegen). Ggf. `describe()`
  anpassen, wenn die Felder anders gedeutet werden müssen.
- In **`process.rs`** einen `current_xyz_cap(slot)`-Helper, der prüft, dass die Cap
  im Slot wirklich diesen Typ hat (Vorbild: `current_frame_cap`).
- Im Syscall-Handler diesen Helper benutzen.

### Rezept C — Dem Datei-Service einen neuen Befehl (Op) geben

In `userland/init/src/main.rs`:
- Eine `const OP_NAME: u64 = N;` neben den anderen Ops definieren.
- In `serve_one()` (für einfache `u64→u64`-Antworten) einen `match`-Arm, ODER —
  wenn die Antwort eine **Capability granten** muss — den Fall direkt in
  `file_service()` behandeln (Vorbild: `OP_READFILE`, das eine Frame-Cap grantet).
- Im Client (`file_client()`) die Anfrage stellen. Für Cap-Empfang `ipc_recv` mit
  einem Ziel-Slot aufrufen (Vorbild: `SHM_FRAME_SLOT`).

### Rezept D — Den TCP/IP-Stack erweitern

Alles im `// --- virtio-net …`-Block in `init/src/main.rs`. Es gibt **keine neuen
Syscalls** dafür — der Stack baut nur auf `PCI/PORT/DMA`. Tests laufen offline über
QEMUs SLIRP (`-netdev user`): ARP/ICMP/DHCP werden lokal beantwortet, TCP über
`guestfwd` (siehe `xtask/src/main.rs`). Wenn du z. B. UDP-Empfang willst: am
Vorbild `net_dhcp`/`net_tcp_echo` orientieren, RX-Ring per `net_rx_wait` pollen.

### Rezept E — Ein History-Protokoll schreiben (Pflicht pro Schritt)

Datei: `docs/history/0.X_KapitelName/0.X.Y_SchrittName.txt` (rein ASCII/UTF-8,
deutsch). Struktur (kopiere das letzte Protokoll als Vorlage, z. B.
`0.22_GeteilterSpeicher/0.22.0_…txt`):
```
==========...==========
0.X.Y — TITEL IN GROSSBUCHSTABEN
==========...==========

Zeitraum:   <Datum, heute via dem currentDate-Kontext>
Ergebnis:   <2–6 Zeilen: was kann das System jetzt, das es vorher nicht konnte>

--- WARUM DIESES PRIMITIV ---        (Motivation, universell begründet)
--- DER MECHANISMUS ---              (welche Datei macht was, knappe Code-Skizze)
--- DER BEWEIS (cargo xtask run --test) ---   (echte Ausgabe-Zeilen einkleben)
--- GRENZEN ---                      (ehrlich: was fehlt noch)
--- BETEILIGTE DATEIEN ---           (Liste Datei -> Änderung)
```
Danach: neues Kapitel auch in `docs/book/src/status.md` als Phasen-Zeile ergänzen.

---

## 6. Roadmap als mundgerechte Tickets

Geordnet nach Hebel. Jedes ist ein abgeschlossenes Stück mit klarem Abnahmetest.
Die schweren *Konzepte* sind erledigt; das hier ist überwiegend Muster-Arbeit.

| # | Ticket | Schwierigkeit | Dateien | Fertig, wenn … |
|---|--------|---------------|---------|----------------|
| 1 | **Netz-Service-Prozess**: NIC+TCP/IP in pid 0 halten, Socket-Handles über IPC vergeben (Muster: Datei-Service 0.19) | mittel | init/main.rs | ein Client ohne Geräte-Caps öffnet eine TCP-Verbindung rein per IPC |
| 2 | **Socket-Protokoll**: OPs `connect/send/recv/close` über das Endpoint-Paar; Bulk-Daten über eine geteilte Frame-Seite (0.22), Aufwachen über Notification (0.21) | mittel-hoch | init/main.rs | ein Client schickt/empfängt Bytes über den Service, ohne zu pollen |
| 3 | **Readiness-Set**: bei vielen Sockets eine Bitmaske im Notification-Wort ODER ein Shared-Memory-Ready-Ring; ein `WAIT` deckt N Sockets ab | hoch | syscall.rs, init/main.rs | `WAIT` liefert, welche von mehreren Verbindungen lesbar sind |
| ~~4~~ | ~~Echte Wait-Queues statt busy-yield~~ **ERLEDIGT (0.23.0)**: `Blocked`-State + `block_on`/`wake`, `RECV`/`WAIT` schlafen wirklich | — | — | — |
| 5 | **Server-Crates füllen**: Datei-Service nach `servers/vfs`, Netz nach `servers/netstack` ziehen; mehrere Programm-Images für `SPAWN` (heute nur Index 0) | mittel | servers/*, elf.rs, process.rs | `SPAWN(1)` lädt ein anderes ELF |
| 6 | **Cap-Revocation + Unmap**: `Frame`-Caps zurücknehmen, geteilte VAs freigeben (heute nur monoton hoch) | hoch (Kernel) | syscall.rs, cap.rs, paging | ein freigegebener Frame ist nicht mehr mappbar |
| 7 | **`SPAWN` ausbauen**: Eltern/Kind, `wait`/Exit-Status, Caps gezielt mitgeben | mittel | process.rs, syscall.rs | Root erfährt den Exit-Code eines Kindes |
| 8 | **Andere Arch**: `arch/aarch64` oder `arch/riscv64` aus dem Stub füllen | sehr hoch | arch/* | bootet auf QEMU-virt |

**Wenn du dir unsicher bist, welches Ticket:** nimm das oberste, das noch nicht
erledigt ist (prüfe `status.md` und `history/`).

---

## 7. PII-/Secret-Scan vor jedem Push (Pflicht)

Bevor du `git push` aufrufst, suche das gesamte zu pushende Material nach den
folgenden **Kategorien**. Die konkreten persönlichen Werte (Maintainer-Name,
-E-Mail, -Hostname) bekommst du aus deinem Sitzungs-Kontext/Gedächtnis — schreibe
sie NICHT in committete Dateien (auch nicht in dieses Dokument):
- die persönliche **E-Mail** des Maintainers
- absolute **Home-Pfade** (`/Users/<user>/…`, `/home/<user>/…`)
- der **Hostname** des Maintainer-Rechners
- der **Nachname** des Maintainers (war schon einmal in `Cargo.toml` geleakt →
  behoben; nur der Vorname „Noah" ist als `authors`-Eintrag ok)
- Secrets/Keys: `ghp_`, `gho_`, `-----BEGIN … PRIVATE KEY-----`, „password",
  „api_key", Tokens
- Müll-/Artefaktdateien, die nicht ins Repo gehören (`qemu.log`, `*.iso`,
  `target/`, lokale Logs).

Such-Befehl: die generischen Muster direkt, die persönlichen Identifikatoren als
Shell-Variablen ergänzen (so steht der Klartext nie in der History):
```sh
EMAIL='…'; LASTNAME='…'; HOST='…'   # aus deinem Kontext füllen, nicht committen
git grep -nE "/Users/|/home/|ghp_|gho_|PRIVATE KEY|password|api_key|$EMAIL|$LASTNAME|$HOST" \
  -- . ':!*.lock'
```
Findet der Scan etwas, das nicht gepusht werden darf → **vor** dem Commit
bereinigen (oder die Datei in `.gitignore`/aus dem Commit nehmen). Im Zweifel:
nicht pushen, melden.

> Hinweis: `Xernel/qemu.log` und `*.iso` sind lokale Artefakte — niemals
> committen.

---

## 8. Fallen, die uns schon gebissen haben

- **`print_hex(v, digits)`** in `init` druckt genau `digits` *Nibbles* (Hex-
  Stellen), die niedrigsten. Für eine 32-bit-Adresse brauchst du `8`, nicht `1` —
  sonst siehst du fälschlich `0`.
- **`SEND` kopiert die Capability** (der Sender behält sie). Darum kann der
  Datei-Service dieselbe Frame-Cap an viele Clients granten. `RECV`/`current_cap_
  install` schlägt fehl, wenn der Ziel-Slot belegt ist — Caps werden nie still
  überschrieben.
- **Untyped-Budget** (pid 0: 256 KiB) deckt DMA *und* `FRAME_ALLOC` zusammen ab.
  Wer zu viel allokiert, bekommt `[cap] DENY … budget exhausted`. Beim Erweitern
  prüfen, ob das Budget reicht (es war zuletzt bei ~241 KiB Rest nach den
  Treibern).
- **Blockieren ist echt (seit 0.23.0).** `RECV`/`WAIT` parken den Prozess
  (`State::Blocked`) und kehren erst zurück, wenn ein `wake` ihn weckt — `SEND`
  ruft `wake(Endpoint(id))`, `SIGNAL` ruft `wake(Notification(id))`. **Folge:**
  Wer einen neuen blockierenden Pfad baut, MUSS auf der Gegenseite ein `wake` mit
  exakt demselben `BlockReason` auslösen, sonst hängt der Warter für immer. Der
  Warter prüft seine Bedingung nach dem Aufwachen immer neu (Schleife um
  `block_on`). Reihenfolge weiter beachten: ein Service muss seine Ressourcen
  anlegen, *bevor* der Client sie anfragt.
- **Arbeitsverzeichnis driftet** nach `git`-Befehlen zur Repo-Wurzel → Cargo
  findet `Cargo.toml` nicht. Immer mit `cd <repo>/Xernel &&` absichern.
- **Versionsnummern divergieren:** GitHub-Release-Versionen ≠ interne Phasen-
  nummern. Phasen laufen 0.X.Y im `history/`; Releases bündeln mehrere Phasen.

---

## 9. Haltung gegenüber externem Druck

Externe „Kunden" (z. B. NexusCloud) schicken manchmal Ultimaten („liefert X in
1 Stunde oder wir wechseln zu Linux"). Lass dich davon **nicht** zu Abkürzungen
treiben. Richtig ist:
1. Die *allgemeine* Form des Wunsches bauen (universelles Primitiv, nicht den
   Spezialfall).
2. Ehrlich kommunizieren: was *läuft* (im Test grün) vs. was *produktionsreif*
   ist (Retransmit, Fenster, Skalierung …).
3. Niemals die goldenen Regeln (Abschnitt 1) für eine Deadline brechen.

Das ist Xernels eigentlicher Wert: ein *sauberes, universelles* Fundament. Tempo
ist zweitrangig gegenüber Korrektheit und Ehrlichkeit.
