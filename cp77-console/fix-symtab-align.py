#!/usr/bin/env python3
# fix-symtab-align.py — alinha o LC_SYMTAB string pool em 8 bytes inserindo padding zero ANTES
# da string table. Workaround do bug do ld no macOS 27: o build SEM a feature `lua` sai com
# stroff mod8=4 e o dyld recusa carregar ("mis-aligned LINKEDIT string pool") → o jogo não abre.
# Uso (a string table tem que ser o fim do arquivo): primeiro `codesign --remove-signature`,
# rode este script, depois `codesign -s - --force`. Determinístico, reusável a cada build.
import sys, struct

path = sys.argv[1]
data = bytearray(open(path, 'rb').read())

magic = struct.unpack_from('<I', data, 0)[0]
assert magic == 0xfeedfacf, f"esperado MH_MAGIC_64, achei {magic:#x}"
ncmds = struct.unpack_from('<I', data, 16)[0]

off = 32  # mach_header_64 = 32 bytes
symtab_off = linkedit_off = None
for _ in range(ncmds):
    cmd, cmdsize = struct.unpack_from('<II', data, off)
    if cmd == 0x2:  # LC_SYMTAB
        symtab_off = off
    elif cmd == 0x19:  # LC_SEGMENT_64
        if data[off + 8:off + 24].split(b'\x00', 1)[0] == b'__LINKEDIT':
            linkedit_off = off
    off += cmdsize

assert symtab_off is not None and linkedit_off is not None, "LC_SYMTAB/__LINKEDIT não achados"

stroff = struct.unpack_from('<I', data, symtab_off + 16)[0]
strsize = struct.unpack_from('<I', data, symtab_off + 20)[0]
pad = (8 - (stroff % 8)) % 8
print(f"stroff={stroff} (mod8={stroff % 8}) strsize={strsize} filelen={len(data)} pad={pad}")
if pad == 0:
    print("já alinhado — nada a fazer")
    sys.exit(0)
assert stroff + strsize == len(data), \
    f"strtab não é o fim ({stroff + strsize} != {len(data)}) — rode 'codesign --remove-signature' antes"

# insere `pad` bytes zero logo antes da string table (gap morto, ninguém referencia)
data[stroff:stroff] = b'\x00' * pad
struct.pack_into('<I', data, symtab_off + 16, stroff + pad)  # LC_SYMTAB.stroff += pad

# __LINKEDIT filesize (+48) e vmsize (+32) += pad
le_vmsize = struct.unpack_from('<Q', data, linkedit_off + 32)[0]
le_filesize = struct.unpack_from('<Q', data, linkedit_off + 48)[0]
struct.pack_into('<Q', data, linkedit_off + 48, le_filesize + pad)
page = 0x4000
struct.pack_into('<Q', data, linkedit_off + 32,
                 max(le_vmsize, ((le_filesize + pad + page - 1) // page) * page))

open(path, 'wb').write(data)
ns = struct.unpack_from('<I', data, symtab_off + 16)[0]
print(f"OK: stroff={ns} (mod8={ns % 8}), inseridos {pad} byte(s)")
