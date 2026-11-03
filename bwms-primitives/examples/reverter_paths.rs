//! Dado um ou mais hashes, procura no dump de recursos do jogo o path que os produz.
//!
//! Existe para o teste decisivo do `#408`: os argumentos capturados ao vivo em
//! `FindToken` (`0x1010e8754`) são mesmo `ResourcePath`? Se um deles bater com o hash de um path
//! REAL do install, a resposta é sim — e não por parecença, por igualdade.
fn main() {
    let alvos: Vec<u64> = std::env::args()
        .skip(1)
        .filter_map(|a| u64::from_str_radix(a.trim_start_matches("0x"), 16).ok())
        .collect();
    if alvos.is_empty() {
        eprintln!("uso: reverter_paths <hash_hex> [hash_hex ...]");
        std::process::exit(2);
    }
    let p = "../cp77-symbols/archive-resources.txt";
    let Ok(t) = std::fs::read_to_string(p) else {
        eprintln!("não achei {p}");
        std::process::exit(1);
    };
    let mut achados = 0usize;
    let mut linhas = 0usize;
    for l in t.lines() {
        let path = l.trim();
        if path.is_empty() {
            continue;
        }
        linhas += 1;
        let h = bwms_primitives::resource_path_hash(path);
        if alvos.contains(&h) {
            println!("{h:#x}  {path}");
            achados += 1;
        }
    }
    println!("\n{achados} de {} hash(es) revertido(s), varrendo {linhas} paths", alvos.len());
}
