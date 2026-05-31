# Vision & Grundentscheidungen

Drei nicht-verhandelbare Eigenschaften:

1. **Stabilität durch Isolation** — Treiber, Dateisysteme, Netz im User-Space.
2. **Sicherheit durch Rust** — Kernel ist `#![forbid(unsafe_code)]`, außer in
   klar markierten HAL-Bereichen.
3. **Sicherheit durch Capabilities** — kein „Root darf alles". Jeder Zugriff
   ist eine explizite, weitergebbare Capability (seL4-Stil).

Details siehe `docs/design/0001-architecture-decisions.md`.
