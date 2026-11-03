//! `fix-symtab-align` — alinha em 8 bytes o string pool do `LC_SYMTAB` de um Mach-O.
//!
//! PORQUE ISTO EXISTE: o linker do Xcode no macOS 27 às vezes emite o string pool num offset
//! `mod 8 == 4`. O dyld recusa carregar assim ("mis-aligned LINKEDIT string pool") e o jogo
//! simplesmente não abre — sem erro útil. A correção é inserir o padding exato ANTES da string
//! table e acertar os campos que apontam para ela.
//!
//! PORQUE EM RUST: era um script Python, e o projeto é Rust-first — "zero Python" vale para a
//! ferramenta também, não só para o runtime. Um `.py` no repositório aparece na barra de
//! linguagens do GitHub e contradiz o que o projeto diz de si mesmo. Sem dependência externa:
//! só `std` e o andador de comandos de carga do `bwms-primitives` (do projeto, zero-dep).
//!
//! USO (a string table tem que ser o fim do arquivo):
//!   1. `codesign --remove-signature <arquivo>`
//!   2. `fix-symtab-align <arquivo>`
//!   3. `codesign -s - --force <arquivo>`
//! Determinístico e idempotente: rodar de novo num arquivo já alinhado não faz nada.

use bwms_primitives::macho::{comandos_de_carga, LC_SEGMENT_64, LC_SYMTAB};
use std::io::{Read, Write};

const PAGE: u64 = 0x4000;

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(v)
}
fn put_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn put_u64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

fn main() {
    let caminho = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("uso: fix-symtab-align <arquivo-mach-o>");
            std::process::exit(2);
        }
    };

    let mut dados = Vec::new();
    match std::fs::File::open(&caminho).and_then(|mut f| f.read_to_end(&mut dados)) {
        Ok(_) => {}
        Err(e) => {
            eprintln!("erro: não consegui ler {caminho}: {e}");
            std::process::exit(1);
        }
    }

    let Some(comandos) = comandos_de_carga(&dados) else {
        eprintln!("erro: não é um Mach-O 64-bit (magic inesperado)");
        std::process::exit(1);
    };

    // Varre os load commands atrás do LC_SYMTAB e do segmento __LINKEDIT.
    let mut symtab_off: Option<usize> = None;
    let mut linkedit_off: Option<usize> = None;
    for comando in comandos {
        if comando.cmd == LC_SYMTAB {
            symtab_off = Some(comando.offset);
        } else if comando.cmd == LC_SEGMENT_64 && comando.offset + 24 <= dados.len() {
            let nome = &dados[comando.offset + 8..comando.offset + 24];
            let fim = nome.iter().position(|&c| c == 0).unwrap_or(nome.len());
            if &nome[..fim] == b"__LINKEDIT" {
                linkedit_off = Some(comando.offset);
            }
        }
    }

    let (symtab_off, linkedit_off) = match (symtab_off, linkedit_off) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            eprintln!("erro: LC_SYMTAB ou __LINKEDIT não encontrados");
            std::process::exit(1);
        }
    };

    let stroff = u32_at(&dados, symtab_off + 16);
    let strsize = u32_at(&dados, symtab_off + 20);
    let pad = ((8 - (stroff % 8)) % 8) as usize;
    println!(
        "stroff={stroff} (mod8={}) strsize={strsize} filelen={} pad={pad}",
        stroff % 8,
        dados.len()
    );
    if pad == 0 {
        println!("já alinhado — nada a fazer");
        return;
    }

    // O padding só é seguro se a string table for o FIM do arquivo: aí o espaço inserido é um
    // buraco morto que ninguém referencia. Com assinatura no fim, não é — daí o passo 1 do uso.
    if stroff as usize + strsize as usize != dados.len() {
        eprintln!(
            "erro: a string table não termina o arquivo ({} != {}) — rode \
             'codesign --remove-signature' antes",
            stroff as usize + strsize as usize,
            dados.len()
        );
        std::process::exit(1);
    }

    // Insere `pad` zeros imediatamente antes da string table e reaponta o LC_SYMTAB.
    let at = stroff as usize;
    dados.splice(at..at, std::iter::repeat(0u8).take(pad));
    put_u32(&mut dados, symtab_off + 16, stroff + pad as u32);

    // __LINKEDIT cresce junto: filesize (+48) e vmsize (+32), este arredondado para a página.
    let le_vmsize = u64_at(&dados, linkedit_off + 32);
    let le_filesize = u64_at(&dados, linkedit_off + 48);
    let novo_filesize = le_filesize + pad as u64;
    put_u64(&mut dados, linkedit_off + 48, novo_filesize);
    let arredondado = novo_filesize.div_ceil(PAGE) * PAGE;
    put_u64(&mut dados, linkedit_off + 32, le_vmsize.max(arredondado));

    match std::fs::File::create(&caminho).and_then(|mut f| f.write_all(&dados)) {
        Ok(_) => {}
        Err(e) => {
            eprintln!("erro: não consegui gravar {caminho}: {e}");
            std::process::exit(1);
        }
    }
    let novo = u32_at(&dados, symtab_off + 16);
    println!("OK: stroff={novo} (mod8={}), inseridos {pad} byte(s)", novo % 8);
}
