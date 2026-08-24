# Vision & Grundentscheidungen

Drei nicht-verhandelbare Eigenschaften:

1. **Stabilität durch Isolation** — Treiber, Dateisysteme, Netz im User-Space.
2. **Sicherheit durch Rust** — Kernel ist `#![forbid(unsafe_code)]`, außer in
   klar markierten HAL-Bereichen.
3. **Sicherheit durch Capabilities** — kein „Root darf alles". Jeder Zugriff
   ist eine explizite, weitergebbare Capability (seL4-Stil).

Details siehe `docs/design/0001-architecture-decisions.md`.

## Portierbarkeit — zweites Target: aarch64

Xernel läuft heute **ausschließlich auf x86_64**. Ein zweites Target ist
beschlossen: **aarch64** (QEMU `virt`). Der Anspruch, ein *universelles*
Fundament zu sein, ist erst eingelöst, wenn der Kernel auf mehr als einer
Architektur bootet — und ein zweites Target ist die einzige ehrliche Probe
darauf, ob die Architektur-Abstraktion (`kernel/src/arch/`) trägt oder ob
x86-Annahmen ins Portable durchgesickert sind.

Was dafür ansteht (noch **keine Zeile Code**):

- Boot über QEMU `virt` statt Limine/BIOS+UEFI
- MMU: 4-Level-Paging existiert konzeptionell auch auf aarch64, aber mit
  eigenen Deskriptor-Formaten (TTBR0/TTBR1 statt CR3)
- Exception-Levels (EL0/EL1) statt Ring 3 / Ring 0, `svc` statt `syscall`
- GIC statt LAPIC/IO-APIC, Generic Timer statt LAPIC-Timer
- PL011-UART statt 16550-seriell
- virtio-**mmio** statt virtio-**pci** für Block und Netz

Die User-Space-Teile (Treiber, XernelFS, TCP/IP-Stack) sind an die Syscall-ABI
gebunden, nicht an die Architektur — sie sollten mit einem Rebuild mitkommen.
Genau diese Annahme wird die Portierung prüfen.
