//! Onde, nas seções de dado, há ponteiro (com os bits de fixup encadeado) pra um endereço — pra achar a vtable que contém uma função.
//!
//!     cargo run --release --example ponteiros -- <binário> <endereço>...

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bin = std::fs::read(&a[1]).expect("binário");
    let m = bwms_primitives::enderecos::fatia_arm64(&bin).expect("arm64");
    let secoes = bwms_primitives::enderecos::secoes(m).expect("Mach-O");
    for alvo in a[2..].iter().map(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).expect("hex")) {
        let off = alvo - bwms_primitives::enderecos::LINK_BASE;
        for s in secoes.iter().filter(|s| s.segmento.starts_with("__DATA") && s.arquivo != 0) {
            let ini = s.arquivo as usize;
            for k in 0..(s.tamanho as usize / 8) {
                let w = u64::from_le_bytes(m[ini + k * 8..ini + k * 8 + 8].try_into().unwrap());
                if w & 0xF_FFFF_FFFF == off && w != 0 {
                    println!("{alvo:#x} em {:#x} ({},{} +{:#x})", s.vm + k as u64 * 8, s.segmento, s.nome, k * 8);
                }
            }
        }
    }
}
