// Resolve a tabela contra o PRÓPRIO binário de onde ela foi gerada. Aqui tem que dar 100%: se não
// der, o defeito é do algoritmo ou da tabela, não do outro binário.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bin = std::fs::read(&a[1]).expect("binário do jogo");
    let img = bwms_primitives::enderecos::Imagem::do_arquivo(&bin).expect("imagem");
    let tabela = bwms_primitives::enderecos::ler(&std::fs::read(&a[2]).expect("tabela")).expect("tabela válida");
    let r: std::collections::HashMap<u64, u64> = bwms_primitives::enderecos::resolver(&img, &tabela).into_iter().collect();
    let certos = tabela.iter().filter(|s| r.get(&s.steam) == Some(&s.steam)).count();
    let errados: Vec<_> = tabela.iter().filter(|s| r.get(&s.steam).is_some_and(|v| *v != s.steam)).collect();
    let ausentes: Vec<_> = tabela.iter().filter(|s| !r.contains_key(&s.steam)).collect();
    println!("tabela: {} entradas", tabela.len());
    println!("  resolvidos NO MESMO endereço: {certos}");
    println!("  resolvidos em endereço ERRADO: {}", errados.len());
    println!("  não resolvidos: {}", ausentes.len());
    for s in ausentes.iter().take(5) {
        let n = if s.palavras.is_empty() { usize::MAX } else { bwms_primitives::enderecos::localizar(&img, s).len() };
        println!("    {:#x} {:?} {} palavras, casamentos: {}", s.steam, s.tipo, s.palavras.len(), n);
    }
}
