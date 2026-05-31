# Status & Entwicklungsstand

Stand: 2026-05-30. Alles Folgende ist in QEMU verifiziert (`cargo xtask run --test`
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
- **Capabilities:** CNode/CapEntry-Grundstrukturen (noch nicht an Syscalls
  gebunden).
- **User-Space:** Ring-3-Übergang via `syscall`/`sysret`, ELF-Loader (lädt ein
  Programm als Limine-Modul), 8 Syscalls (siehe [Syscall-ABI](syscalls.md)).
- **Tastatur:** PS/2 über IO-APIC, blockierendes und nicht-blockierendes Lesen.
- **Dynamischer Speicher:** wachsender User-Heap via `SBRK`.

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

## XOS — das erste OS auf Xernel

Ein separates Userland-OS (eigenes Repo) läuft auf Xernel: interaktive Shell mit
Befehlen, Tastatureingabe, Heap. XOS und Xernel sind **getrennte Projekte**,
verbunden nur durch die Syscall-ABI. Booten ohne Kernel-Eingriff:

```sh
cargo xtask run --init /pfad/zu/xos-init.elf
```

## Noch offen

- Capabilities an die Syscalls binden (`invoke(cap, method, args)`)
- Mehrere Prozesse + Adressraum-Trennung (dann: XMM-Save im Context-Switch)
- Timer-Frequenz in Hz (LAPIC kalibrieren)
- Framebuffer/GUI, Dateisystem-API, `fork`/`exec`
- ELF-Loader vom Kernel in einen Root-Server verlagern
- Tastatur: Shift/Modifier; IO-APIC-Basis aus ACPI statt hartkodiert
