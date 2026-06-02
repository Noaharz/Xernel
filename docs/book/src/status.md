# Status & Entwicklungsstand

Stand: 2026-06-02. Alles Folgende ist in QEMU verifiziert (`cargo xtask run --test`
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
- **Capabilities:** CNode/CapEntry pro Prozess; **Port-I/O an eine `IoPort`-
  und MMIO-Mapping (`IOMAP`) an eine `IoMem`-Capability gebunden** — keine
  ambiente Hardware-Autorität mehr (der virtio-Treiber darf seine Ports und
  seine BAR mappen; ein System-Port wie CMOS und das Mappen von echtem RAM
  werden verweigert).
- **User-Space:** Ring-3-Übergang via `syscall`/`sysret`, ELF-Loader (lädt ein
  Programm als Limine-Modul), 16 Syscalls (siehe [Syscall-ABI](syscalls.md)).
- **Mehrere Prozesse** mit isolierten Adressräumen (eigene Page-Tables),
  **preemptiv** verzahnt (timer-getrieben) — plus kooperatives `YIELD`.
- **Tastatur:** PS/2 über IO-APIC, blockierendes und nicht-blockierendes Lesen.
- **Dynamischer Speicher:** wachsender User-Heap via `SBRK`.
- **Treiber im User-Space:** Kernel liefert nur Primitive (PCI-Config-Read,
  MMIO-Map, DMA-Alloc, Port-I/O). Ein **vollständiger virtio-blk-Treiber in
  Ring 3** richtet eine Virtqueue ein und liest Sektor 0 von der Disk — der
  Kernel kennt das Wort "virtio" nicht.

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
| 0.15 Capabilities | Port-I/O an `IoPort`-, MMIO-Mapping an `IoMem`-Capability gebunden — Least-Privilege für Treiber |

## XOS — das erste OS auf Xernel

Ein separates Userland-OS (eigenes Repo) läuft auf Xernel: interaktive Shell mit
Befehlen, Tastatureingabe, Heap. XOS und Xernel sind **getrennte Projekte**,
verbunden nur durch die Syscall-ABI. Booten ohne Kernel-Eingriff:

```sh
cargo xtask run --init /pfad/zu/xos-init.elf
```

## Noch offen

- Capabilities weiter binden: `DMA_ALLOC` gegen ein Untyped-Budget, Delegation
  (`invoke(cap, method, args)`, copy/grant) — Port-I/O und `IOMAP` sind bereits gated
- Mehrere Prozesse + Adressraum-Trennung (dann: XMM-Save im Context-Switch)
- Timer-Frequenz in Hz (LAPIC kalibrieren)
- Framebuffer/GUI, Dateisystem-API, `fork`/`exec`
- ELF-Loader vom Kernel in einen Root-Server verlagern
- Tastatur: Shift/Modifier; IO-APIC-Basis aus ACPI statt hartkodiert
