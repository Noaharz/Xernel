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
| 13 | `IOMAP` | phys, len | user-vaddr / `u64::MAX` | Mappt Geräte-MMIO (eine PCI-BAR) uncached in den aufrufenden Prozess. |
| 14 | `DMA_ALLOC` | len, out_ptr | 0 / `u64::MAX` | Allokiert einen phys.-zusammenhängenden DMA-Puffer; schreibt `[user_vaddr, phys]` nach `out_ptr`. |
| 15 | `PORT_IN` | port, size | wert | Liest einen I/O-Port (size 1/2/4) — für Legacy-Geräte-Treiber. |
| 16 | `PORT_OUT` | port, size, value | 0 | Schreibt einen I/O-Port. |

Unbekannte Nummern liefern `u64::MAX`.

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
- Noch **nicht** verfügbar: Dateisystem, Framebuffer/GUI, Prozesse (fork/exec),
  Timer-Frequenz in Hz.

## Speicher-Layout (User-Sicht)

| Bereich | Adresse |
|---|---|
| Programm-Image (Code/Daten) | ab `0x0040_0000` (4 MiB) |
| User-Stack (64 KiB) | bis `0x0081_0000` (8 MiB) |
| Heap (per `SBRK`) | ab `0x1000_0000` (256 MiB) |
| Kernel (für Ring 3 gesperrt) | obere Hälfte (`0xffff_…`) |
