# Repo-Layout

```
<repo>/
├── Xernel/        # gesamter CODE (Cargo-Workspace)
│   ├── kernel/        no_std Mikrokernel
│   ├── libs/          xabi, xstd, xdriver, xlibc
│   ├── servers/       rootserver, vfs, netstack, disp, pm
│   ├── drivers/       virtio (Phase 4), …
│   ├── userland/      xsh, …
│   ├── targets/       x86_64-xernel.json, …
│   ├── boot/          Limine-Config
│   ├── xtask/         Build- / Run-Automation
│   └── tests/         Integrationstests
└── docs/          gesamte DOKUMENTATION
    ├── history/       Versionierte Konzept-Snapshots
    ├── design/        ADRs
    └── book/          mdBook — The Xernel Book
```

> Code und Doku sind strikt getrennt. Das hier ist verbindlich.
