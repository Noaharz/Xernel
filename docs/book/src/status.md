# Status & Entwicklungsstand

Stand: 2026-06-04. Alles Folgende ist in QEMU verifiziert (`cargo xtask run --test`
→ `boot-test PASSED`).

## Was funktioniert

- **Boot:** Limine (BIOS+UEFI), x86_64, höhere Hälfte, serielle Konsole.
- **Speicher:** Frame-Allocator aus der Limine-Memory-Map, 4-Level-Paging über
  die HHDM, Kernel-Heap.
- **Interrupts:** GDT/TSS (IST-Stacks), IDT mit allen CPU-Exceptions, LAPIC,
  PIC abgeschaltet, periodischer LAPIC-Timer.
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
- **User-Space:** Ring-3-Übergang via `syscall`/`sysret`, ELF-Loader (lädt ein
  Programm als Limine-Modul), 19 Syscalls (siehe [Syscall-ABI](syscalls.md)).
- **Mehrere Prozesse** mit isolierten Adressräumen (eigene Page-Tables),
  **preemptiv** verzahnt (timer-getrieben) — plus kooperatives `YIELD`.
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
  zwischen Prozessen. Grundlage für Mikrokernel-Dienste (Datei-/Block-/Netz-
  Service als eigene Prozesse).

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

## XOS — das erste OS auf Xernel

Ein separates Userland-OS (eigenes Repo) läuft auf Xernel: interaktive Shell mit
Befehlen, Tastatureingabe, Heap. XOS und Xernel sind **getrennte Projekte**,
verbunden nur durch die Syscall-ABI. Booten ohne Kernel-Eingriff:

```sh
cargo xtask run --init /pfad/zu/xos-init.elf
```

## Noch offen

- Capabilities: Delegation (`invoke(cap, method, args)`, copy/grant zwischen
  Prozessen), `PCI_READ` per Cap — Port-I/O, `IOMAP` und `DMA_ALLOC` sind bereits gated
- Mehrere Prozesse + Adressraum-Trennung (dann: XMM-Save im Context-Switch)
- Timer-Frequenz in Hz (LAPIC kalibrieren)
- Framebuffer/GUI, Dateisystem-API, `fork`/`exec`
- ELF-Loader vom Kernel in einen Root-Server verlagern
- Tastatur: Shift/Modifier; IO-APIC-Basis aus ACPI statt hartkodiert
