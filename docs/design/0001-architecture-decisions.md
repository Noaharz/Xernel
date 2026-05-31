# ADR-0001 — Architektur-Grundentscheidungen für Xernel

- **Status:** Akzeptiert
- **Datum:** 2026-05-18
- **Beschluss durch:** Noah, mit Claude als Co-Designer
- **Bezug:** `../history/0.1_KernelPlanen/0.1.2_Xernel_Konzept_v0.1.md`

## Kontext

Xernel ist ein neuer Mikrokernel in Rust. Vor Beginn von Phase 0 (Bootstrap)
mussten die nicht-verhandelbaren Grundentscheidungen festgezurrt werden, damit
die spätere Implementierung nicht aus inkonsistenten Annahmen heraus driftet.

Die fünf zentralen offenen Fragen aus dem Konzeptdokument wurden am 2026-05-18
beantwortet.

## Entscheidungen

### 1. Plattform-Strategie: Multi-Architektur von Tag 1

- Phase 1 implementiert ausschließlich **x86_64** (am besten dokumentiert,
  QEMU-tauglich).
- Die HAL-Schnittstelle (`kernel/src/arch/mod.rs`) ist jedoch **von Anfang an**
  multi-arch-tauglich: AArch64- und RISC-V-Module existieren als Stubs und
  zwingen jeden neu hinzugefügten generischen Pfad, die HAL-Grenze zu respektieren.
- Fernziel: PCs, AR-Brillen, Embedded-Hardware — überall bootbar.

### 2. Lizenz: noch offen

- `Cargo.toml` führt `license = "TBD"`.
- Entscheidung wird vor 0.1-Public-Release nachgeholt.
- Keine SPDX-Header in den Source-Dateien, bis die Lizenz steht.

### 3. Linux-Unabhängigkeit: hart

- Xernel implementiert **keine** Linux-ABI. Kein WSL1-Klon, kein Linux-Syscall-Layer.
- POSIX-Kompatibilität nur als Library-OS-Schicht (`xlibc`), in Xernel-Capabilities
  ausgedrückt.
- `relibc` aus Redox wird **nicht** als Basis verwendet — wir schreiben `xlibc`
  selbst, um POSIX-Subset auf unsere ABI zu mappen.

### 4. Team: solo + Claude

- Keine Open-Source-Veröffentlichung vor Meilenstein 0.3 (Capabilities + IPC laufen).
- CI von Tag 1 trotzdem, weil sie auch solo wertvoll ist.

### 5. Hardware-Ehrgeiz: überall

- "Überall bootbar und nutzbar" — PCs, AR-Brillen, Embedded.
- Display-Server (`servers/disp`) wird so abstrahiert, dass Framebuffer, GPU
  und Stereo-HMD-Sinks denselben Front-End-Cap-Protokoll-Surface nutzen.
- Konsequenz: **kein** hartcodierter VGA-Textmodus, **kein** PC-spezifischer
  Display-Pfad im Kernel.

## Konsequenzen

- HAL-Disziplin: jede generische Codestelle muss `arch::*` aufrufen, nie
  `core::arch::asm!` direkt.
- POSIX-Layer ist Phase 6 — kein Druck, ihn früher anzugehen.
- `xlibc` als Eigenentwicklung kostet Aufwand, gibt aber volle Kontrolle und
  hält den Code frei von Linux-Erblasten.

## Status der HAL-Implementierung (Phase 0 Snapshot)

| Arch    | `init` | `serial_write` | `halt_forever` |
|---------|--------|-----------------|-----------------|
| x86_64  | UART   | 16550 / COM1   | `cli; hlt`     |
| aarch64 | stub   | stub            | `wfi`           |
| riscv64 | stub   | stub            | `wfi`           |
