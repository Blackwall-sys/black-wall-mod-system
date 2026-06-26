#!/usr/bin/env python3
"""Realinha o string table do __LINKEDIT pra 8 bytes.

O linker novo do Xcode (ld-prime) às vezes gera o string pool num offset
4-alinhado (por causa da tabela de símbolos indiretos de 4 bytes com contagem
ímpar). O dyld novo recusa com 'mis-aligned LINKEDIT string pool'. Aqui a gente
insere padding pra alinhar em 8. Depois é preciso re-assinar (codesign --force).
"""
import struct
import sys


def realign(path: str) -> bool:
    d = bytearray(open(path, "rb").read())
    if struct.unpack_from("<I", d, 0)[0] != 0xFEEDFACF:
        print("nao e Mach-O 64 arm64", file=sys.stderr)
        return False
    ncmds = struct.unpack_from("<I", d, 16)[0]
    off = 32
    stroff = spos = None
    le_pos = le_sz = None
    cs_pos = cs_off = None
    for _ in range(ncmds):
        cmd, sz = struct.unpack_from("<II", d, off)
        if cmd == 0x2:  # LC_SYMTAB
            stroff = struct.unpack_from("<I", d, off + 16)[0]
            spos = off + 16
        elif cmd == 0x19 and d[off + 8:off + 24].split(b"\x00")[0] == b"__LINKEDIT":
            le_pos = off + 48
            le_sz = struct.unpack_from("<Q", d, off + 48)[0]
        elif cmd == 0x1D:  # LC_CODE_SIGNATURE
            cs_pos = off + 8
            cs_off = struct.unpack_from("<I", d, off + 8)[0]
        off += sz
    if stroff is None:
        print("sem LC_SYMTAB", file=sys.stderr)
        return False
    new = (stroff + 7) & ~7
    pad = new - stroff
    if pad == 0:
        print("string table ja alinhado")
        return True
    d[stroff:stroff] = b"\x00" * pad
    struct.pack_into("<I", d, spos, new)
    if le_pos is not None:
        struct.pack_into("<Q", d, le_pos, le_sz + pad)
    if cs_pos is not None and cs_off >= stroff:
        struct.pack_into("<I", d, cs_pos, cs_off + pad)
    open(path, "wb").write(d)
    print(f"realinhado: stroff {hex(stroff)} -> {hex(new)} (pad {pad})")
    return True


if __name__ == "__main__":
    ok = realign(sys.argv[1])
    sys.exit(0 if ok else 1)
