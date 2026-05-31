# Einführung

Xernel ist ein moderner Mikrokernel in Rust mit Capability-basierter
Sicherheit. Linux-unabhängig. Multi-Architektur. Überall bootbar.

Dieses Buch wächst mit dem System mit.

- **[Status & Entwicklungsstand](./status.md)** — was heute funktioniert und der
  Phasen-Überblick.
- **[Syscall-ABI](./syscalls.md)** — der Vertrag, gegen den User-Programme
  geschrieben werden.
- [Vision & Grundentscheidungen](./vision.md) und [Repo-Layout](./layout.md) für
  das große Bild.

Der vollständige, chronologische Bau-Verlauf — jeder Schritt mit Code, Begründung
und Verifikation — liegt als Erzähl-Protokoll unter `docs/history/` (Phasen
0.3–0.9).
