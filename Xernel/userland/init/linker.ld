/* Linker script for Xernel user programs.
 *
 * Links statically at a fixed low-half base address so the kernel's ELF loader
 * can map each PT_LOAD segment at its p_vaddr without relocations. Sections are
 * page-aligned so the read-execute and read-write segments never share a page,
 * which keeps the loader's per-page W^X mapping simple. */

ENTRY(_start)

SECTIONS
{
    . = 0x400000;

    .text : {
        *(.text .text.*)
    }

    . = ALIGN(0x1000);
    .rodata : {
        *(.rodata .rodata.*)
    }

    . = ALIGN(0x1000);
    .data : {
        *(.data .data.*)
    }
    .bss : {
        *(COMMON)
        *(.bss .bss.*)
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.note .note.*)
        *(.comment)
    }
}
