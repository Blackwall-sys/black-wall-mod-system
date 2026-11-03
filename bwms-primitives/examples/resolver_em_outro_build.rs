//! Quantos endereços de uma tabela de assinaturas (`dados/enderecos-steam-2.5.0*.bin`) o resolvedor acha num binário de outro build, e quais
//! ficam sem tradução: a medida do que um build que não é o Steam 2.5.0 (GOG, Epic, patch) ligaria.
//!
//!     cargo run --release --example resolver_em_outro_build -- <tabela .bin> <binário> [saída dos sem tradução] [saída dos pares]

use bwms_primitives::enderecos as en;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 2 {
        return eprintln!("uso: resolver_em_outro_build <tabela .bin> <binário> [saída]");
    }
    let tabela = en::ler(&std::fs::read(&a[0]).expect("tabela")).expect("tabela de assinaturas");
    let bin = std::fs::read(&a[1]).expect("binário");
    let img = en::Imagem::do_arquivo(&bin).expect("Mach-O arm64 com __text");
    let achados: std::collections::HashMap<u64, u64> = en::resolver(&img, &tabela).into_iter().collect();
    let sem: Vec<String> = tabela.iter().map(|s| s.steam).filter(|e| !achados.contains_key(e)).map(|e| format!("{e:#x}")).collect();
    println!("{} de {} resolvidos; {} sem tradução", achados.len(), tabela.len(), sem.len());
    if let Some(s) = a.get(2) {
        std::fs::write(s, sem.join("\n")).expect("saída");
    }
    // Os pares resolvidos (`steam outro`, em ordem de endereço Steam): a base da tabela de tradução fixa de uma build conhecida.
    if let Some(s) = a.get(3) {
        let mut pares: Vec<(u64, u64)> = achados.into_iter().collect();
        pares.sort();
        std::fs::write(s, pares.iter().map(|(st, o)| format!("{st:#x} {o:#x}")).collect::<Vec<_>>().join("\n")).expect("pares");
    }
}
