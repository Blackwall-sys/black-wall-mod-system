fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bin = std::fs::read(&a[1]).unwrap();
    let img = bwms_primitives::enderecos::Imagem::do_arquivo(&bin).unwrap();
    let mut alvos = Vec::new();
    let mut pilha = vec![std::path::PathBuf::from(&a[2])];
    while let Some(d) = pilha.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() { pilha.push(p) } else if p.extension().is_some_and(|x| x == "rs") {
                alvos.extend(bwms_primitives::enderecos::enderecos_no_fonte(&std::fs::read_to_string(&p).unwrap_or_default()));
            }
        }
    }
    alvos.sort_unstable(); alvos.dedup();
    let (_, f) = bwms_primitives::enderecos::gerar(&img, &alvos);
    for (x, m) in f { println!("{x:#x} {m:?}"); }
}
