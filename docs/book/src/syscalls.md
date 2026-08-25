# Syscall-ABI

Dies ist der verbindliche Vertrag zwischen einem User-Programm (Ring 3) und dem
Xernel-Kernel. Wer ein Programm für Xernel schreibt, programmiert gegen genau
diese Schnittstelle — der Kernel wird **nie** von Hand verändert.

## Aufruf-Konvention

| | |
|---|---|
| Instruktion | `syscall` |
| Nummer | `rax` |
| Argumente | `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` (Arg 1–6) |
| Rückgabe | `rax` |
| Zerstört | `rcx` und `r11` (durch die `syscall`-Instruktion) |

> Das 4. Argument liegt in `r10`, **nicht** `rcx` — `syscall` überschreibt `rcx`
> mit der Rücksprungadresse.

## Syscall-Tabelle

| Nr | Name | Argumente | Rückgabe | Wirkung |
|----|------|-----------|----------|---------|
| 1 | `WRITE` | fd, ptr, len | #bytes / `u64::MAX` | Schreibt `len` Bytes ab User-Adresse `ptr` auf die Konsole (fd 1/2 → seriell). |
| 2 | `EXIT` | code | — | Beendet das Programm. |
| 3 | `DEBUG` | value | 0 | Druckt `value` als Hex (Register-Debugging). |
| 4 | `GET_TICKS` | — | ticks | Timer-Ticks seit Boot (grobe Uptime). |
| 5 | `SYSINFO` | which | wert / `u64::MAX` | which: 0 = RAM gesamt, 1 = RAM benutzt, 2 = Frame-Größe (jeweils Bytes). |
| 6 | `READ` | fd, ptr, len | #bytes / `u64::MAX` | Liest Tastatur in den Puffer, **blockiert** bis ≥ 1 Byte da ist. |
| 7 | `READ_NB` | fd, ptr, len | #bytes (0 = leer) | Wie `READ`, aber **nie blockierend** (für Idle-/Animations-Loops). |
| 8 | `SBRK` | delta (i64) | alter Break / `u64::MAX` | Verschiebt den Heap-Break (Unix-`sbrk`); `delta = 0` fragt ab. |
| 9 | `FB_INFO` | ptr | 0 / `u64::MAX` | Mappt den Framebuffer in User-Space; schreibt `[addr, width, height, pitch, bpp]` (5×u64) nach `ptr`. |
| 10 | `GETPID` | — | pid | PID des aktuellen Prozesses. |
| 11 | `YIELD` | — | 0 | Gibt die CPU an den nächsten bereiten Prozess ab (kooperativ). |
| 12 | `PCI_READ` | bus, dev, func, offset | dword | Liest 32 Bit aus dem PCI-Config-Space (für User-Space-Treiber). |
| 13 | `IOMAP` | phys, len | user-vaddr / `u64::MAX` | Mappt Geräte-MMIO (eine PCI-BAR) uncached in den aufrufenden Prozess. **Gated:** braucht eine `IoMem`-Capability über `[phys, phys+len)`. |
| 14 | `DMA_ALLOC` | len, out_ptr | 0 / `u64::MAX` | Allokiert einen phys.-zusammenhängenden DMA-Puffer; schreibt `[user_vaddr, phys]` nach `out_ptr`. **Gated:** verrechnet gegen ein `Untyped`-Budget. |
| 15 | `PORT_IN` | port, size | wert / `u64::MAX` | Liest einen I/O-Port (size 1/2/4). **Gated:** braucht eine `IoPort`-Capability über den Port. |
| 16 | `PORT_OUT` | port, size, value | 0 / `u64::MAX` | Schreibt einen I/O-Port. **Gated:** braucht eine `IoPort`-Capability über den Port. |
| 17 | `CAP_IDENTIFY` | slot, out_ptr | 0 / `u64::MAX` | Beschreibt die Capability im eigenen CNode-Slot; schreibt `[type, a, b]` (3×u64, normalisiert) nach `out_ptr`. |
| 18 | `SEND` | ep_slot, word, cap_slot | 0 / `u64::MAX` | Sendet `word` (+ optional die Cap in `cap_slot`, sonst `u64::MAX`) über den Endpoint in `ep_slot`. Nicht blockierend. |
| 19 | `RECV` | ep_slot, out_ptr, dst_slot | 0 / `u64::MAX` | Blockiert bis eine Nachricht da ist; schreibt das Wort nach `out_ptr`, installiert eine mitgeschickte Cap in `dst_slot` (`u64::MAX` = verwerfen). |
| 20 | `SPAWN` | module, cap_slot | pid / `u64::MAX` | Erzeugt einen neuen Prozess aus Programm-Image `module` (heute nur 0 = init-Image): eigener Adressraum, frisch gesäte Caps, als bereit eingehängt. Legt eine **`Process`-Capability** für das Kind in `cap_slot` des Aufrufers (`u64::MAX` = kein Handle) — ohne dieses Handle kann der Elternprozess sein Kind später nicht anfassen. |
| 21 | `SIGNAL` | notif_slot, bits | 0 / `u64::MAX` | Signalisiert eine Notification: ODER-t `bits` in ihr Signal-Wort (nicht blockierend, akkumuliert). Braucht eine `Notification`-Capability in `notif_slot`. |
| 22 | `WAIT` | notif_slot | bits (≠0) / 0 | Blockiert, bis die Notification in `notif_slot` Bits ≠ 0 hat, gibt sie zurück und löscht sie. Das Readiness-Primitiv (epoll/kqueue-Form): ein `WAIT` deckt viele Quellen ab. `0` = keine Cap. |
| 23 | `FRAME_ALLOC` | pages, cap_slot, out_ptr | 0 / `u64::MAX` | Allokiert `pages` physisch-zusammenhängende, genullte Seiten, mappt sie in den Aufrufer, legt eine `Frame`-Capability in `cap_slot` und schreibt die User-Adresse nach `out_ptr`. **Gated:** verrechnet gegen das `Untyped`-Budget. Die Frame-Cap lässt sich per `SEND` granten → geteilter Speicher. |
| 24 | `MAP_FRAME` | cap_slot, out_ptr | 0 / `u64::MAX` | Mappt den von der `Frame`-Cap in `cap_slot` benannten Speicher in den Aufrufer und schreibt die User-Adresse nach `out_ptr`. Keine Budget-Verrechnung — die empfangende Hälfte von Shared Memory: zwei Prozesse, die dieselbe delegierte Frame-Cap mappen, sehen denselben RAM. |
| 25 | `SPAWN_ENV` | module, envp, len, cap_slot | pid / `u64::MAX` | Wie `SPAWN`, überträgt zusätzlich einen Environment-Block (`envp`, `len` Bytes) an das Kind, das ihn per `GETENVP` abruft. Die Env-Daten werden in den Kind-Adressraum an HEAP_START kopiert. |
| 26 | `GET_STATUS` | proc_slot | status / `u64::MAX` | Fragt den Zustand des Prozesses ab, den die `Process`-Cap in `proc_slot` benennt: `0` = running, `1` = exited. **Gated:** ohne `Process`-Cap `u64::MAX`. |
| 27 | `GETENVP` | out_ptr | 0 / `u64::MAX` | Schreibt `[envp, len]` (2×u64) des aktuellen Prozesses nach `out_ptr`. `envp` ist die User-Adresse des Environment-Blocks (bei `SPAWN_ENV` gesetzt, `0` wenn kein Env vorhanden), `len` ist die Größe in Bytes. |
| 28 | `LOG_READ` | proc_slot, out_ptr, max | bytes / `u64::MAX` | Liest und konsumiert bis zu `max` Bytes (max. 64 KiB) aus dem Log-Ringpuffer des Prozesses, den die `Process`-Cap in `proc_slot` benennt. **Gated.** |
| 29 | `KILL` | proc_slot, signal | 0 / `u64::MAX` | Sendet ein Signal an den Prozess, den die `Process`-Cap in `proc_slot` benennt: `15` = SIGTERM, `9` = SIGKILL. Markiert ihn als beendet mit dem entsprechenden Exit-Code. **Gated.** PID 0 (Root) kann nicht getötet werden. |
| 30 | `WAIT_PID` | proc_slot | exit_code / `u64::MAX` | Blockiert (mit Yield-Schleife) bis der Prozess beendet ist, den die `Process`-Cap in `proc_slot` benennt, und gibt seinen Exit-Code zurück (`256` = vom Kernel nach CPU-Exception getötet). **Gated.** |
| 31 | `CAP_GRANT` | proc_slot, src_slot, dst_slot | 0 / `u64::MAX` | Kopiert die eigene Capability aus `src_slot` in Slot `dst_slot` der CSpace des Prozesses, den die `Process`-Cap in `proc_slot` benennt. **Gated.** Die Delegation, die *kein* Handshake mit dem Kind braucht: ein frisch gespawntes Programm bekommt damit genau die Autorität, die es braucht — vorher hat es nur das geteilte Endpoint-Paar aus `seed_caps`. |

Unbekannte Nummern liefern `u64::MAX`. Die **gated** Syscalls (13–16, 23, 26, 28–31) prüfen
eine Capability des aufrufenden Prozesses und liefern `u64::MAX` ohne Wirkung,
wenn sie fehlt — es gibt keine ambiente Hardware-Autorität. Mit `CAP_IDENTIFY` kann
ein Prozess seine eigenen Capabilities aufzählen (nur die eigenen — keine
globale Sicht).

**Autorität über andere Prozesse.** Ein Prozess kann einen anderen nur anfassen,
wenn er eine **`Process`-Capability** für ihn hält — `SPAWN`/`SPAWN_ENV` geben sie
dem Elternprozess zurück. Es gibt keinen Weg, aus einer PID Autorität zu machen:
`KILL`, `WAIT_PID`, `GET_STATUS`, `LOG_READ` und `CAP_GRANT` nehmen alle einen
Cap-Slot, keine PID. Wie jede Capability lässt sie sich per `SEND` weiterreichen —
so bekommt ein Monitoring-Dienst ein Handle, ohne selbst spawnen zu müssen.

**User-Zeiger.** Jeder Zeiger, den ein Syscall entgegennimmt, wird nicht nur
gegen den User-Adressbereich geprüft, sondern Seite für Seite gegen die
Page-Tables: vorhanden, `USER_ACCESSIBLE`, bei schreibenden Syscalls auch
schreibbar. Ein ungemappter oder fremder Zeiger liefert `u64::MAX` — der Kernel
faultet nicht im Namen des Aufrufers.

**Abstürze.** Löst ein Prozess eine CPU-Exception aus (`#PF`, `#GP`, `#UD`, …),
beendet der Kernel **nur diesen Prozess** und läuft weiter. Sein Exit-Code ist
dann `256` (`EXIT_FAULT`) — außerhalb dessen, was `EXIT` selbst übergeben kann,
sodass `WAIT_PID` "abgestürzt" von "regulär beendet" unterscheidet.

> Jeder Prozess läuft in seinem **eigenen Adressraum** (eigene Page-Table) —
> Speicher ist zwischen Prozessen isoliert. Prozesse laufen **verzahnt**
> (kooperatives Multitasking über `YIELD`).

## Minimaler Rust-Wrapper (Kopiervorlage)

```rust
use core::arch::asm;

#[inline]
fn syscall3(nr: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!("syscall",
             inlateout("rax") nr => ret,
             in("rdi") a1, in("rsi") a2, in("rdx") a3,
             lateout("rcx") _, lateout("r11") _,
             options(nostack));
    }
    ret
}

fn write(s: &[u8])       { syscall3(1, 1, s.as_ptr() as u64, s.len() as u64); }
fn exit(code: u64) -> !  { syscall3(2, code, 0, 0); loop {} }
fn ticks() -> u64        { syscall3(4, 0, 0, 0) }
fn sysinfo(w: u64) -> u64 { syscall3(5, w, 0, 0) }
fn read(buf: &mut [u8]) -> u64    { syscall3(6, 0, buf.as_mut_ptr() as u64, buf.len() as u64) }
fn read_nb(buf: &mut [u8]) -> u64 { syscall3(7, 0, buf.as_mut_ptr() as u64, buf.len() as u64) }
fn sbrk(delta: i64) -> u64        { syscall3(8, delta as u64, 0, 0) }
fn fb_info(out: &mut [u64; 5]) -> u64 { syscall3(9, out.as_mut_ptr() as u64, 0, 0) }
fn spawn_env(module: u64, envp: *const u8, len: u64) -> u64 { syscall3(25, module, envp as u64, len) }
fn get_status(pid: u64) -> u64     { syscall3(26, pid, 0, 0) }
fn getenvp(out: &mut [u64; 2]) -> u64 { syscall3(27, out.as_mut_ptr() as u64, 0, 0) }
fn log_read(pid: u64, out: &mut [u8], max: u64) -> u64 { syscall3(28, pid, out.as_mut_ptr() as u64, max) }
fn kill(pid: u64, signal: u64) -> u64 { syscall3(29, pid, signal, 0) }
fn wait_pid(pid: u64) -> u64 { syscall3(30, pid, 0, 0) }
```

## Grafik (Framebuffer)

`FB_INFO` füllt `[addr, width, height, pitch, bpp]` und mappt den Framebuffer
user-schreibbar. Pixel (32 bpp, `0x00RRGGBB`) zeichnen:

```rust
let mut fb = [0u64; 5];
if fb_info(&mut fb) == 0 {
    let (addr, w, h, pitch) = (fb[0], fb[1], fb[2], fb[3]);
    let stride = (pitch / 4) as usize;        // Pixel pro Zeile
    let buf = addr as *mut u32;
    unsafe { buf.add(y * stride + x).write_volatile((r << 16) | (g << 8) | b); }
}
```

## Pointer-Regeln (WRITE / READ)

Der Kernel liest/schreibt den Puffer direkt im gemeinsamen Adressraum. Validiert
wird: `ptr != 0`, `len ≤ 1 MiB`, und `ptr`/`ptr+len` liegen in der unteren
kanonischen Hälfte (`< 0x0000_8000_0000_0000`). Sonst `u64::MAX`. Der Bereich
muss im Programm gemappt sein (eigenes `.rodata`/`.data`/Stack/Heap).

## Dynamischer Speicher

Mit `SBRK` lässt sich ein `#[global_allocator]` bauen → `Vec`, `String`, `Box`
usw. Die Heap-Region liegt bei `0x1000_0000` und wächst nach oben; Pages werden
lazy gemappt (nur Angefasstes kostet RAM).

## Laufzeit-Umgebung eines Programms

- x86_64, **Ring 3**, Interrupts an (Timer tickt im Hintergrund).
- Statische ELF, gelinkt ab **`0x400000`**, Entry `_start`.
- **SSE/FPU verfügbar** (normaler x86_64-Build möglich, kein Soft-float-Zwang).
- Stack: 64 KiB, ABI-korrekt ausgerichtet (`rsp % 16 == 8` bei Eintritt).
- Kein `std`. `alloc` ist über `SBRK` möglich.
- Prozesse: `SPAWN` erzeugt einen neuen Prozess; `SPAWN_ENV` zusätzlich mit
  Environment-Variablen; `GET_STATUS` fragt den Zustand eines Prozesses ab;
  `GETENVP` ruft das Env im Kind ab; `LOG_READ` liest den stdout/stderr-Log
  eines Prozesses (Ring-Buffer, 64 KiB); `KILL` beendet einen Prozess per
  Signal (SIGTERM/SIGKILL); `WAIT_PID` blockiert bis ein Kind beendet ist und
  liefert den Exit-Code.
- Noch **nicht** verfügbar: Dateisystem-*Syscalls* (XernelFS ist eine Ring-3-
  Bibliothek über die Block-Primitive), Timer-Frequenz in Hz.

## Speicher-Layout (User-Sicht)

| Bereich | Adresse |
|---|---|
| Programm-Image (Code/Daten) | ab `0x0040_0000` (4 MiB) |
| User-Stack (64 KiB) | bis `0x0081_0000` (8 MiB) |
| Heap (per `SBRK`) | ab `0x1000_0000` (256 MiB) |
| Geräte-MMIO (`IOMAP`) | `0x5000_0000`–`0x6000_0000` |
| DMA-Puffer (`DMA_ALLOC`) | `0x6000_0000`–`0x7000_0000` |
| Geteilter Speicher (`FRAME_ALLOC`/`MAP_FRAME`) | `0x7000_0000`–`0x8000_0000` |
| Kernel (für Ring 3 gesperrt) | obere Hälfte (`0xffff_…`) |
