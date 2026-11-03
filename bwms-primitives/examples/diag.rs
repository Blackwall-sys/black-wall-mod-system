fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (bs, bg) = (std::fs::read(&a[1]).unwrap(), std::fs::read(&a[2]).unwrap());
    let (is, ig) = (bwms_primitives::enderecos::Imagem::do_arquivo(&bs).unwrap(), bwms_primitives::enderecos::Imagem::do_arquivo(&bg).unwrap());
    let alvos: Vec<u64> = a[3..].iter().map(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()).collect();
    let (t, f) = bwms_primitives::enderecos::gerar(&is, &alvos);
    for s in &t {
        println!("{:#x} {:?} {} palavras; no Steam em {:x?}; no GOG em {:x?}", s.steam, s.tipo, s.palavras.len(), bwms_primitives::enderecos::localizar(&is, s), bwms_primitives::enderecos::localizar(&ig, s));
    }
    println!("faltam {f:x?}");
}
