# Xernel — Dokumentation

Dies ist der Doku-Ordner. **Kein Code hier.** Code lebt im Unterordner
`Xernel/` (Repo-Wurzel → `Xernel/`).

## Struktur

- `history/` — versionierte Konzept-Snapshots und alte Pläne. Wird **nicht**
  überschrieben; neue Versionen kommen als eigene Unterordner dazu.
- `design/` — Architecture Decision Records (ADRs). Kurze, datierte Notizen
  zu jeder signifikanten Entscheidung.
- `book/` — mdBook „The Xernel Book". Lange Form, wächst mit dem System mit.

## Bauen

```sh
cd docs/book && mdbook serve
```
