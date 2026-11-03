// Quais entradas dependem da tabela de símbolos, e se o NÚCLEO (sem o qual nenhum gancho instala)
// está entre elas.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bin = std::fs::read(&a[1]).unwrap();
    let img = bwms_primitives::enderecos::Imagem::do_arquivo(&bin).unwrap();
    let tabela = bwms_primitives::enderecos::ler(&std::fs::read(&a[2]).unwrap()).unwrap();
    let sem_img = bwms_primitives::enderecos::Imagem {
        texto: img.texto, texto_vm: img.texto_vm, secoes: img.secoes.clone(),
        simbolos: Default::default(), stubs: Default::default(),
    };
    let sem: std::collections::HashMap<u64, u64> = bwms_primitives::enderecos::resolver(&sem_img, &tabela).into_iter().collect();
    let faltam: Vec<_> = tabela.iter().filter(|s| !sem.contains_key(&s.steam)).collect();
    println!("sem tabela de símbolos, ficam sem tradução: {}", faltam.len());
    let mut por_tipo = std::collections::BTreeMap::new();
    for s in &faltam { *por_tipo.entry(format!("{:?}", s.tipo)).or_insert(0) += 1; }
    println!("  por tipo: {por_tipo:?}");
    // o núcleo: executor, RTTI, bind
    const NUCLEO: [u64; 3] = [0x1_0109_9fbc, 0x1_010b_3cac, 0x1_011a_8e58];
    for n in NUCLEO {
        let t = tabela.iter().find(|s| s.steam == n);
        println!("  núcleo {n:#x}: tipo={:?} resolve sem símbolos? {}", t.map(|s| format!("{:?}", s.tipo)), sem.contains_key(&n));
    }
}
