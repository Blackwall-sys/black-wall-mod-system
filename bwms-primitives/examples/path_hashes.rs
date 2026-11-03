//! Vetores de teste com hashes REAIS capturados do motor (via `reslinkdump`, proof de 2026-07-17).
//!
//! Se `resource_path_hash` reproduz esses valores, ela é fiel ao motor — e aí os hashes servem pra
//! identificar um `HashMap` em memória viva por CONTEÚDO, não só por forma (stride/capacidade).
fn main() {
    let esperado: &[(&str, u64)] = &[
        ("base\\gameplay\\gui\\common\\icons\\atlas_common.xbm", 0x4d275e2c6198e8d0),
        ("base\\gameplay\\gui\\common\\icons\\atlas_common.inkatlas", 0xcb91e1aeaf64dd2c),
    ];
    let mut todos_ok = true;
    for (caminho, esp) in esperado {
        let got = bwms_primitives::resource_path_hash(caminho);
        let ok = got == *esp;
        todos_ok &= ok;
        println!("{} {:#018x} (esperado {:#018x})  {}", if ok { "OK  " } else { "FALHA" }, got, esp, caminho);
    }
    println!("{}", if todos_ok { ">>> hash FIEL ao motor" } else { ">>> DIVERGE do motor" });
}
