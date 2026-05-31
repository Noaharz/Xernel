# Xernel

> Ein moderner Mikrokernel in Rust mit Capability-basierter Sicherheit.
> **Linux-unabhängig. Multi-Architektur. Überall bootbar.**

**Status:** 0.0.1 — Phase 0 (Bootstrap)
**Lizenz:** TBD (siehe `Cargo.toml`)
**Doku:** `../docs/` (insbesondere `../docs/history/0.1_KernelPlanen/0.1.2_Xernel_Konzept_v0.1.md`)

## Schnellstart

Voraussetzungen:

- `rustup` (Toolchain wird über `rust-toolchain.toml` auto-installiert)
- `qemu-system-x86_64`
- `xorriso` (für ISO-Erzeugung)
- `nasm` (optional, Bootassets)

```sh
cd Xernel        # WICHTIG: in den Code-Ordner wechseln
cargo xtask run  # baut Kernel, packt ISO, startet QEMU
```

Erwartete Ausgabe auf der seriellen Konsole:

```
[xernel] hello, xernel — arch=x86_64, build=debug
```

## Repo-Layout (Workspace-Wurzel: `Xernel/Xernel/`)

```
kernel/      # no_std Mikrokernel — Arch-HAL, MM, Sched, Cap, IPC, Syscalls
libs/        # xabi, xstd, xdriver, xlibc — Userland-Bibliotheken
servers/     # rootserver, vfs, netstack, disp, pm — User-Space-Dienste
drivers/     # User-Space-Treiber (virtio zuerst)
userland/    # xsh und weitere User-Programme
targets/     # Custom-Target-JSONs
boot/        # Limine-Konfiguration und Boot-Assets
xtask/       # Build-/Run-/QEMU-Automation
tests/       # Integrationstests in QEMU
```

Dokumentation liegt **außerhalb** dieses Workspaces in `../docs/`.
