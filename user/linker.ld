ENTRY(_start)

SECTIONS
{
    . = 0x400000;

    .text : ALIGN(0x1000) {
        *(.text._start)
        *(.text._start.*)
        *(.text*)
        *(.plt)
        *(.plt.got)
        *(.plt.sec)
    }

    .rodata : ALIGN(0x1000) {
        *(.rodata*)
        *(.srodata*)
    }

    .data : ALIGN(0x1000) {
        *(.data*)
        *(.sdata*)
        *(.data.rel.ro*)
        *(.got)
        *(.got.plt)
    }

    .bss : ALIGN(0x1000) {
        *(.bss*)
        *(.sbss*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.note*)
        *(.comment)
        *(.eh_frame*)
    }
}