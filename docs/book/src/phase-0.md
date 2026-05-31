# Phase 0 — Bootstrap

Ziel: `cargo xtask run` startet QEMU mit einem Image, das auf der seriellen
Konsole `[xernel] hello, xernel — arch=x86_64, build=debug` ausgibt.

## Schritte

1. Toolchain installieren (`rustup` zieht sich die Version aus
   `rust-toolchain.toml`).
2. Limine-Binärassets holen:
   ```sh
   cd Xernel
   cargo xtask fetch-limine
   ```
3. Kernel bauen und starten:
   ```sh
   cargo xtask run
   ```

## Was bereits steht (Stand: 2026-05-18)

- Workspace + Custom-Target + Linker-Script (`linker-x86_64.ld`)
- `xtask` mit `build`, `iso`, `run`, `fetch-limine`, `clean`
- Kernel-Entry `kmain`, Panic-Handler, 16550-UART, `println!`-Makro
- HAL-Skelett für x86_64 (Serial implementiert; GDT/IDT/Paging/APIC als
  Phase-1-Stubs)
- Stubs für AArch64 und RISC-V — HAL-Disziplin von Tag 1

## Automatisierter Boot-Test

```sh
cargo xtask run --test
```

Baut den Kernel mit dem `boot-test`-Feature, hängt `isa-debug-exit` an QEMU,
läuft headless und übersetzt den QEMU-Exit-Code in PASS/FAIL. Der Kernel führt
dabei interne Selbsttests aus (Paging alloc→map→read-back, Timer-Ticks,
IPC-Summe) und beendet QEMU mit Erfolgs- oder Fehlerstatus.

## Phase 1 — erledigt und in QEMU verifiziert

- **GDT + TSS** mit IST-Stacks für `#DF` und `#PF` (`arch/x86_64/gdt.rs`)
- **IDT** mit Handlern für alle CPU-Exceptions; `#PF`/`#GP`/`#DF` mit
  Register-Dump auf Serial; `int3`-Selbsttest beim Boot (`arch/x86_64/idt.rs`)
- **Frame-Allocator** aus der Limine-Memory-Map, Bump + Free-List (`mm/frame.rs`)
- **Paging** über die HHDM via `OffsetPageTable`, `map`/`map_mmio`
  (`arch/x86_64/paging.rs`)
- **Kernel-Heap** (`linked_list_allocator`), zunächst 1 MiB Bootstrap-Arena in
  `.bss` (`mm.rs`)
- **LAPIC** + 8259-PIC-Abschaltung + periodischer LAPIC-Timer (`arch/x86_64/apic.rs`)

## Meilenstein 2.0 — erledigt

- **Context-Switch** in naked-asm (`arch/x86_64/context.rs`)
- **Kooperativer Round-Robin-Scheduler** (`sched.rs`)
- **In-Kernel-IPC-Channel** (`ipc.rs`)
- **Demo:** zwei Kernel-Threads (Producer/Consumer) tauschen 10 Nachrichten aus,
  Summe verifiziert (`demo.rs`)

## Was als Nächstes kommt (Phase 2 fortgesetzt → Phase 3)

- Capability-System (`CNode`, `Untyped`, `Endpoint`, `Notification`, …)
- Echte `Endpoint`/`Notification`-IPC statt Demo-Channel
- Syscall-Entry (`syscall`/`sysret`), Ring-3-Übergang
- ELF-Loader im Root-Server, erster User-Prozess
- Preemption über den bereits laufenden LAPIC-Timer
