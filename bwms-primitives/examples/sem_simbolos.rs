// E se a imagem em memória não tiver a tabela de símbolos (o dyld pode não deixar o __LINKEDIT
// mapeado)? Resolve a tabela com símbolos e sem, pra separar "falta símbolo" de "assinatura falha".
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bin = std::fs::read(&a[1]).unwrap();
    let img = bwms_primitives::enderecos::Imagem::do_arquivo(&bin).unwrap();
    let tabela = bwms_primitives::enderecos::ler(&std::fs::read(&a[2]).unwrap()).unwrap();
    let com: std::collections::HashMap<u64, u64> = bwms_primitives::enderecos::resolver(&img, &tabela).into_iter().collect();
    let sem_img = bwms_primitives::enderecos::Imagem {
        texto: img.texto,
        texto_vm: img.texto_vm,
        secoes: img.secoes.clone(),
        simbolos: Default::default(),
        stubs: Default::default(),
    };
    let sem: std::collections::HashMap<u64, u64> = bwms_primitives::enderecos::resolver(&sem_img, &tabela).into_iter().collect();
    println!("com símbolos: {} de {}", com.len(), tabela.len());
    println!("sem símbolos: {} de {}", sem.len(), tabela.len());
    let por_tipo = |m: &std::collections::HashMap<u64, u64>| {
        let mut c = std::collections::BTreeMap::new();
        for s in &tabela { if m.contains_key(&s.steam) { *c.entry(format!("{:?}", s.tipo)).or_insert(0) += 1; } }
        c
    };
    println!("  por tipo, com: {:?}", por_tipo(&com));
    println!("  por tipo, sem: {:?}", por_tipo(&sem));
}
