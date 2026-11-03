// Endereços com assinatura no Steam que não resolvem num outro binário, com o tipo e quantos casamentos a janela teve lá.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (bs, bo) = (std::fs::read(&a[1]).unwrap(), std::fs::read(&a[2]).unwrap());
    let (is, io) = (bwms_primitives::enderecos::Imagem::do_arquivo(&bs).unwrap(), bwms_primitives::enderecos::Imagem::do_arquivo(&bo).unwrap());
    let tabela = bwms_primitives::enderecos::ler(&std::fs::read(&a[3]).unwrap()).unwrap();
    let r: std::collections::HashMap<u64, u64> = bwms_primitives::enderecos::resolver(&io, &tabela).into_iter().collect();
    for s in &tabela {
        if !r.contains_key(&s.steam) {
            let n = if s.palavras.is_empty() { usize::MAX } else { bwms_primitives::enderecos::localizar(&io, s).len() };
            println!("{:#x} {:?} {} palavras, casamentos no outro: {}, no Steam: {}", s.steam, s.tipo, s.palavras.len(), n, if s.palavras.is_empty() { 0 } else { bwms_primitives::enderecos::localizar(&is, s).len() });
        }
    }
}
