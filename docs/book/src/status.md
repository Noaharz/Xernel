# Status & Entwicklungsstand

Stand: 2026-08-25. Alles Folgende ist in QEMU verifiziert (`cargo xtask run --test`
→ `boot-test PASSED`).

## Was funktioniert

- **Boot:** Limine (BIOS+UEFI), x86_64, höhere Hälfte, serielle Konsole.
- **Speicher:** Frame-Allocator aus der Limine-Memory-Map, 4-Level-Paging über
  die HHDM, Kernel-Heap.
- **Interrupts:** GDT/TSS (IST-Stacks), IDT mit allen CPU-Exceptions, LAPIC,
  PIC abgeschaltet, periodischer LAPIC-Timer.
- **Fehlerisolation (seit 0.27):** eine CPU-Exception aus Ring 3 tötet **nur den
  auslösenden Prozess** (Exit-Code `EXIT_FAULT` = 256, abholbar per `WAIT_PID`);
  der Kernel läuft weiter. Eine Exception aus Ring 0 panict weiterhin — dort ist
  nichts Sicheres mehr übrig. Ebenso wird jeder User-Zeiger in einem Syscall
  gegen die Page-Tables geprüft, nicht nur gegen einen Adressbereich: ein
  ungemappter Zeiger liefert einen Fehlercode, statt den Kernel in Ring 0
  faulten zu lassen. Beweis: ein Kind liest absichtlich von einer ungemappten
  Adresse, stirbt allein — und der TCP-Test danach läuft unverändert durch.
- **SSE/FPU** für Ring 3 aktiviert.
- **Multitasking-Kern:** Context-Switch, kooperativer Scheduler, In-Kernel-IPC
  (Demo: zwei Threads tauschen Nachrichten).
- **Capabilities:** CNode/CapEntry pro Prozess; **alle drei autoritäts-
  gewährenden Treiber-Primitive sind cap-gated** — Port-I/O an eine `IoPort`-,
  MMIO-Mapping (`IOMAP`) an eine `IoMem`- und DMA (`DMA_ALLOC`) an ein
  verbrauchbares `Untyped`-Budget gebunden. Keine ambiente Hardware-Autorität
  mehr (der virtio-Treiber darf seine Ports, seine BAR und sein DMA-Budget; ein
  System-Port wie CMOS, das Mappen von echtem RAM und unbegrenzte DMA-Allokation
  werden verweigert). Ein Prozess kann seine **eigene** Capability-Tabelle per
  `CAP_IDENTIFY` aufzählen (keine globale Sicht).
- **Prozesse sind Capabilities (seit 0.28):** `SPAWN`/`SPAWN_ENV` geben dem
  Elternprozess eine **`Process`-Capability** für das Kind zurück. `KILL`,
  `WAIT_PID`, `GET_STATUS`, `LOG_READ` und `CAP_GRANT` nehmen einen Cap-Slot,
  keine PID — aus einer erratenen Zahl entsteht keine Autorität mehr. Mit
  `CAP_GRANT` legt ein Elternprozess einem Kind gezielt Rechte in die CSpace,
  **bevor** es das erste Mal läuft; das ist die Antwort darauf, dass ein frisch
  gespawntes Programm sonst nur das geteilte Endpoint-Paar besitzt und weder
  Datei noch Socket öffnen könnte.
- **User-Space:** Ring-3-Übergang via `syscall`/`sysret`, ELF-Loader (lädt ein
  Programm als Limine-Modul), 31 Syscalls (siehe [Syscall-ABI](syscalls.md)).
- **Mehrere Prozesse** mit isolierten Adressräumen (eigene Page-Tables),
  **preemptiv** verzahnt (timer-getrieben) — plus kooperatives `YIELD`.
- **Echtes Blockieren (Wait-Queues):** ein Prozess, der auf eine Nachricht
  (`RECV`) oder ein Signal (`WAIT`) wartet, geht in einen echten `Blocked`-
  Zustand über; der Scheduler überspringt ihn, bis ihn genau das passende
  Ereignis weckt (`SEND` an den Endpoint, `SIGNAL` an die Notification). Kein
  Busy-Yield mehr — ein Warter verbrennt keine CPU.
- **Prozesse zur Laufzeit (`SPAWN`):** der Kernel bootet nur noch **einen**
  Prozess (den Root, pid 0); jeden weiteren erzeugt der Root selbst über
  `SYS_SPAWN` — wie ein echtes init. Der Neuling bekommt einen eigenen
  Adressraum, eigenen Heap und eine frisch gesäte Capability-Tabelle und wird
  vom Scheduler aufgenommen. Erst dadurch wird Xernel zum OS: ein Programm ruft
  ein anderes ins Leben.
- **Deployment-Unterstützung (SPAWN_ENV, GET_STATUS, GETENVP):** mit `SPAWN_ENV`
  überträgt der Elternprozess Environment-Variablen an ein Kind — der Block wird
  in den Kind-Adressraum an HEAP_START kopiert, `GETENVP` liefert Pointer und
  Länge. `GET_STATUS` fragt den Zustand eines Prozesses ab (running/exited/
  unknown) ohne Capability-Berechtigung. Das Fundament für Deployment-Dienste:
  PID-gesteuertes Verhalten eines Binaries.
- **Log-Streaming (LOG_READ):** jeder `WRITE`-Aufruf (fd 1 oder 2) wird
  zusätzlich in einen prozess-lokalen Ring-Buffer (64 KiB) gespiegelt. Ein
  beliebiger anderer Prozess kann diesen Puffer per `LOG_READ` auslesen und
  konsumieren — ohne Capability, ohne IPC-Verbindung. Nützlich für Debugging,
  Monitoring und Status-Abfragen zwischen Diensten und orchestrierendem Prozess.
- **Prozess beenden (KILL + WAIT_PID):** ein Prozess kann einen anderen per PID
  beenden — `KILL(pid, 15)` sendet SIGTERM (graceful), `KILL(pid, 9)` SIGKILL
  (hard). Der Zielprozess wird sofort als `Done` markiert mit dem entsprechenden
  Exit-Code. `WAIT_PID` blockiert (per Yield-Schleife) bis ein Kind beendet ist
  und liefert den Exit-Code zurück. PID 0 (Root) kann nicht getötet werden.
  Zusammen mit `GET_STATUS` bilden `KILL` + `WAIT_PID` das vollständige
  Lebenszyklus-Management für einen Orchestrator: Prozess starten (mit ENV),
  Status abfragen, beenden, Exit-Code abholen.
- **Tastatur:** PS/2 über IO-APIC, blockierendes und nicht-blockierendes Lesen.
- **Dynamischer Speicher:** wachsender User-Heap via `SBRK`.
- **Treiber im User-Space:** Kernel liefert nur Primitive (PCI-Config-Read,
  MMIO-Map, DMA-Alloc, Port-I/O). Ein **vollständiger virtio-blk-Treiber in
  Ring 3** richtet eine Virtqueue ein und bildet eine **Block-Schicht**, die
  beliebige Sektoren **liest und schreibt** (`blk_init`/`blk_rw`) — der Kernel
  kennt das Wort "virtio" nicht und braucht für das Schreiben keinen neuen Syscall.
- **Dateisystem (XernelFS):** ein kleines On-Disk-FS auf dem Block-Layer —
  Superblock, Verzeichnis (16 Dateien, flach), `format`/`create`/`read`/`list`.
  Formatiert die Disk, legt Dateien an und liest sie zurück — **komplett in
  Ring 3, ohne jede Kernel-Änderung**.
- **Inter-Prozess-IPC + Capability-Delegation (Endpoints):** zwei Prozesse
  tauschen über einen Endpoint Nachrichten aus (`SEND`/`RECV`), benannt nur über
  eine `Endpoint`-Capability. Eine Nachricht kann eine **Capability tragen**: der
  Root grantet dem Kind seine `IoPort`-Cap, woraufhin das Kind denselben Port
  lesen darf, der ihm vorher verweigert wurde — Autorität wandert explizit
  zwischen Prozessen.
- **Geteilter Speicher (Frame-Delegation):** ein Prozess allokiert eine
  physisch-zusammenhängende Seite gegen sein `Untyped`-Budget (`FRAME_ALLOC`),
  bekommt eine `Frame`-Capability und kann sie über einen Endpoint **granten**;
  der Empfänger mappt dieselbe physische Seite (`MAP_FRAME`) → **gemeinsamer
  Speicher über die Adressraumgrenze**. Beweis: der Datei-Service legt eine ganze
  Datei in eine geteilte Seite und reicht dem Client nur die Frame-Cap — der
  liest die Datei in **einem** Zug aus dem Speicher statt byteweise über IPC. Der
  Bulk-Datenpfad neben dem Nachrichten-Passing (Fundament für Socket-Puffer und
  Readiness-Ringe).
- **Datei-Service (erster Mikrokernel-Server):** das XernelFS läuft als
  **eigener Prozess**, der über ein Anfrage/Antwort-Endpoint-Paar bedient wird.
  Ein gespawnter Client **ohne jede Geräte-Capability** liest das komplette
  Dateisystem (Anzahl, Namen, Größen, Inhalte) — rein per IPC, während der
  Service die echte Disk-Arbeit macht. Die zentrale Mikrokernel-Eigenschaft
  sichtbar: ein Programm bekommt eine Leistung, ohne die Hardware-Autorität zu
  besitzen. Ganz ohne neuen Syscall — nur aus `SPAWN` + IPC + Capabilities.
- **Netzwerk (virtio-net im User-Space):** ein vollständiger NIC-Treiber in
  Ring 3 (zwei Virtqueues, RX + TX) plus ein wachsender TCP/IP-Stack — alles in
  einem Boot offline verifiziert: **DHCP** (UDP) holt eine IP, **ARP** löst das
  Gateway auf, **ICMP** pingt es, und eine echte **TCP**-Verbindung (Drei-Wege-
  Handshake, Datenstrom, Echo, FIN) zu einem Echo-Server steht. Wie der Block-
  Treiber komplett auf den Primitiven (PCI, Port-I/O, DMA) gebaut, **ohne neuen
  Syscall**. (Noch nicht produktionsreif: kein Retransmit/Fenster, kein
  passives Öffnen.)
- **Netzwerk-Service (Socket-API über IPC):** der Netz-Stack ist in PID 0
  extrahiert und wird über ein Anfrage/Antwort-Endpoint-Paar bedient
  (`OP_NET_CONNECT`/`SEND`/`RECV`/`CLOSE`, Bulk-Daten über die geteilte
  Frame-Seite aus 0.22, Bereitschaft über die Notification aus 0.21). Ein
  gespawnter Client **ohne jede Geräte-Capability** öffnet eine echte
  TCP-Verbindung (Handshake gegen den Echo-Server), sendet eine Nachricht und
  empfängt das Echo — rein per IPC, während der Service die NIC-Arbeit macht.
  Seit 0.24.0 verwaltet der Service **mehrere gleichzeitige Verbindungen**
  (`SOCK_MAX`=4): jeder Socket hat einen eigenen lokalen Port, eigenen TCP-
  Zustand und eigenen Shared Frame, das Protokoll trägt einen Socket-Index,
  und Empfangsframes werden per TCP-Zielport dem richtigen Socket zugeordnet.
  Beweis: ein Client öffnet zwei Verbindungen parallel und bekommt je das
  richtige Echo auf dem richtigen Socket zurück. Seit 0.25.0 gibt es das
  **Ready-Set** (Ticket #3, das select/epoll-Äquivalent): ein
  `OP_NET_GET_READY`-Aufruf liefert die Bitmaske aller Sockets mit
  gepufferten Daten, und der Service hebt pro Socket ein Bereitschafts-Bit
  auf der geteilten Notification — **ein `WAIT` meldet, welche von mehreren
  Verbindungen lesbar sind**. Eingehende Frames werden dabei per Socket
  gepuffert statt verworfen (Datenverlust über Socket-Grenzen hinweg ist
  ausgeschlossen), `RECV` bedient zuerst den gepufferten Bestand. Die
  nächsten Schritte sind ein Byte-Stream-Puffer/Ring pro Socket und die
  Aufspaltung des kombinierten Service-Hosts (FS+Net) in eigene Server-Crates
  (Ticket #5).

## Phasen-Überblick (Details im `history/`-Protokoll)

| Phase | Inhalt |
|---|---|
| 0.3 KernelFundament | Boot, Speicher, Interrupts, Timer, Threads + IPC |
| 0.4 RingDreiUndSyscalls | Ring 3 + `syscall`/`sysret`, erster User-Prozess, Caps |
| 0.5 EchteProgramme | ELF-Loader, separat kompilierte Programme |
| 0.6 ErstesOS | brauchbare ABI (Text, Sysinfo, Uptime) |
| 0.7 TastaturInput | PS/2-Tastatur + `READ` → interaktiv |
| 0.8 XOS_Feedback | SSE, `READ_NB`, externes Booten (`--init`), Loader-Fix |
| 0.9 UserHeap | `SBRK` → dynamischer Speicher; Stack-Alignment-Fix |
| 0.10 Framebuffer | `FB_INFO` → Pixel-Grafik aus dem User-Space |
| 0.11 Multiprocessing | Prozesse mit isolierten Adressräumen |
| 0.12 Multitasking | kooperatives Scheduling (`YIELD`) — verzahnte Prozesse |
| 0.13 Preemption | timer-getriebenes preemptives Scheduling |
| 0.14 TreiberFramework | User-Space-Treiber: PCI, MMIO, DMA, Port-I/O → virtio-blk liest Sektor 0 |
| 0.15 Capabilities | Port-I/O (`IoPort`), MMIO (`IoMem`) und DMA (`Untyped`-Budget) cap-gated — Least-Privilege für Treiber |
| 0.16 Dateisystem | Block-Layer (R/W) + **XernelFS**: Format/Verzeichnis/Datei-I/O — komplett im User-Space |
| 0.17 IPC/Delegation | Endpoint-IPC + **Capability-Delegation**: der Root grantet dem Kind eine Cap, Autorität wandert zwischen Prozessen |
| 0.18 Spawn | **`SYS_SPAWN`**: der Kernel bootet nur den Root; der Root erschafft jedes Kind selbst zur Laufzeit — Xernel wird zum OS |
| 0.19 Datei-Service | XernelFS als **eigener Prozess**: ein Client ohne Geräte-Caps liest Dateien rein per IPC — erster echter Mikrokernel-Server |
| 0.20 Netzwerk | **virtio-net** im User-Space: NIC hochgefahren, ARP-Request gesendet + Gateway-Antwort empfangen — erstes Paket auf dem Draht (M4-Start) |
| 0.20.1 IPv4/ICMP | **ping** ans Gateway: ARP-Resolve + IPv4-Header mit Prüfsumme + ICMP-Echo — Request raus, Reply rein |
| 0.20.2 UDP/DHCP | **DHCP** holt eine IP (10.0.2.15): UDP/BOOTP-DISCOVER raus, OFFER geparst — UDP funktioniert |
| 0.20.3 TCP | **TCP-Handshake + Datenstrom**: SYN/SYN-ACK/ACK zu einem Echo-Server, Zeile gesendet + zurückbekommen, FIN — TCP funktioniert |
| 0.20.4 Netzwerk-Service | **Socket-API über IPC** (M4): der Netz-Stack wird in PID 0 zum Service extrahiert; ein Client ohne Geräte-Caps öffnet per IPC eine TCP-Verbindung, sendet/empfängt über die geteilte Frame-Seite |
| 0.21.0 Readiness | **Notification-Objekt** (`SIGNAL`/`WAIT`): seL4-Async-Signal, der epoll/kqueue-Baustein — ein Service signalisiert Bereitschaft, ein Client wartet darauf |
| 0.22.0 GeteilterSpeicher | **Frame-Capabilities** (`FRAME_ALLOC`/`MAP_FRAME`): geteilter Speicher über die Adressraumgrenze — der Datei-Service legt eine Datei in eine geteilte Seite, der Client mappt sie und liest sie in einem Zug |
| 0.23.0 WaitQueues | **Echtes Blockieren**: `RECV`/`WAIT` schlafen wirklich (Prozess-Zustand `Blocked`), der Scheduler überspringt sie; ein `SEND`/`SIGNAL` weckt punktgenau — kein Busy-Yield mehr |
| 0.24.0 MultiConnection | **Socket-API mit mehreren Verbindungen** (Ticket #2): der Service verwaltet 4 TCP-Sockets gleichzeitig (eigener Port, TCP-Zustand, Shared Frame je Socket); das Protokoll trägt einen Socket-Index, Empfang wird per Zielport demuxed; ein Client ohne Geräte-Caps öffnet zwei Verbindungen parallel und verifiziert je Echo |
| 0.25.0 ReadySet | **Ready-Set / WAIT** (Ticket #3, select/epoll-Äquivalent): ein `OP_NET_GET_READY` liefert die Bitmaske der lesbaren Sockets, pro Socket wird ein Bereitschafts-Bit auf der geteilten Notification erheit — **ein `WAIT` meldet, welche von mehreren Verbindungen lesbar sind**; eingehende Frames werden gepuffert statt verworfen, `RECV` bedient zuerst den Puffer |
| 0.26.0 Deployment | **Deployment-Primitive + Kill/Wait:** SPAWN_ENV (25), GET_STATUS (26), GETENVP (27), LOG_READ (28), KILL (29), WAIT_PID (30) — sechs neue Syscalls für PID-gesteuertes Deployment mit vollem Lebenszyklus-Management |
| 0.27.0 Fehlerisolation | **Ein Prozess stirbt, der Kernel lebt:** CPU-Exceptions unterscheiden Ring 3 von Ring 0 (`EXIT_FAULT`), User-Zeiger werden gegen die Page-Tables geprüft statt nur gegen einen Bereich; nebenbei repariert: `SPAWN_ENV` schrieb den Env-Block an eine virtuelle Adresse, als wäre sie physisch, und übertrug nie Daten |
| 0.28.0 ProzessCaps | **Prozesse als Capabilities** (Ticket #7): `SPAWN` gibt eine `Process`-Cap zurück, `KILL`/`WAIT_PID`/`GET_STATUS`/`LOG_READ` sind darauf gegated statt auf eine PID; neu `CAP_GRANT` (31) — ein Elternprozess legt einem Kind Autorität in die CSpace, bevor es läuft |

## XOS — das erste OS auf Xernel

Ein separates Userland-OS (eigenes Repo) läuft auf Xernel: interaktive Shell mit
Befehlen, Tastatureingabe, Heap. XOS und Xernel sind **getrennte Projekte**,
verbunden nur durch die Syscall-ABI. Booten ohne Kernel-Eingriff:

```sh
cargo xtask run --init /pfad/zu/xos-init.elf
```

## Portierung

Xernel ist heute **x86_64-only**. Als zweites Target ist **aarch64** (QEMU
`virt`) beschlossen — noch keine Zeile Code, siehe [Vision](vision.md) für den
Umfang (Boot, MMU, EL0/EL1, GIC, PL011, virtio-mmio). Bis dahin gilt jede
Aussage in diesem Dokument ausschließlich für x86_64.

## Noch offen

- Capabilities: `invoke(cap, method, args)` als generischer Aufruf und `PCI_READ`
  per Cap — Port-I/O, `IOMAP` und `DMA_ALLOC` sind gated, Grant zwischen
  Prozessen läuft seit 0.17
- XMM-Save im Context-Switch (Adressraum-Trennung selbst läuft seit 0.11)
- Timer-Frequenz in Hz (LAPIC kalibrieren)
- **Nur ein Programm-Image.** `SPAWN` nimmt einen Image-Index entgegen, aber es
  existiert nur Index 0 (das Boot-Init). Solange das so ist, kann Xernel kein
  fremdes Programm starten — das ist der größte offene Punkt, alles andere hängt
  daran. Nötig: mehrere Boot-Module und ein Loader, der einen Index auflöst
- **Spawn ohne Suspend.** Xernel preemptet, also kann ein Kind laufen, bevor der
  Elternprozess `CAP_GRANT` aufgerufen hat. Heute wartet das Kind in einer
  Schleife; ein Spawn-suspended/Resume-Paar wäre die saubere Form
- Aufräumen nach dem Tod eines Prozesses: Frames, Adressraum und Capability-Space
  eines beendeten Prozesses werden nicht freigegeben (gilt für `exit` wie für
  `EXIT_FAULT`), und wer auf sein IPC-Endpoint wartet, erfährt nichts davon
- ELF-Loader vom Kernel in einen Root-Server verlagern
- Tastatur: Shift/Modifier; IO-APIC-Basis aus ACPI statt hartkodiert
