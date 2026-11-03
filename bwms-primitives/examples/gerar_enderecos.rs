//! Gera as tabelas de assinaturas dos endereços que o código do BWMS cita, a partir do binário Steam 2.5.0.
//!
//!     cargo run --release --example gerar_enderecos -- <binário Steam> <pasta src do cp77-console> <saída inteira .bin> <saída pública .bin>
//!
//! A inteira (o dylib de dev) leva todo `.rs` da pasta. A pública leva só os arquivos que o build sem feature compila: a árvore de módulos a
//! partir do `lib.rs`, com o `#[cfg]` e o `#[path]` de cada `mod` avaliados sem feature nenhuma. A regra é o próprio fonte; não há lista à parte.

use bwms_primitives::enderecos as en;
use std::path::{Path, PathBuf};

/// Todo `.rs` sob `pasta`.
fn todos_os_rs(pasta: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let mut pilha = vec![pasta.to_path_buf()];
    while let Some(d) = pilha.pop() {
        for e in std::fs::read_dir(&d).expect("pasta src").flatten() {
            let p = e.path();
            if p.is_dir() {
                pilha.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                v.push(p);
            }
        }
    }
    v
}

/// Os arquivos que o compilador lê a partir de `raiz` com `features` ligadas. `Err` = `cfg` que a avaliação não conhece, ou `mod` sem arquivo.
fn arvore_de_modulos(raiz: &Path, features: &[&str]) -> Result<Vec<PathBuf>, String> {
    let mut v = Vec::new();
    let mut pilha = vec![raiz.to_path_buf()];
    while let Some(arquivo) = pilha.pop() {
        let fonte = std::fs::read_to_string(&arquivo).map_err(|e| format!("{}: {e}", arquivo.display()))?;
        let pasta_do_arquivo = arquivo.parent().unwrap_or(Path::new("."));
        let dono = matches!(arquivo.file_name().and_then(|n| n.to_str()), Some("lib.rs" | "mod.rs" | "main.rs"));
        let pasta_dos_filhos =
            if dono { pasta_do_arquivo.to_path_buf() } else { pasta_do_arquivo.join(arquivo.file_stem().unwrap_or_default()) };
        for m in en::modulos_declarados(&fonte) {
            let mut liga = true;
            for c in &m.cfg {
                liga &= en::cfg_ativo(c, features).ok_or_else(|| format!("{}: `#[cfg({c})]` em `mod {}`", arquivo.display(), m.nome))?;
            }
            if !liga {
                continue;
            }
            let filho = match &m.caminho {
                Some(p) => pasta_do_arquivo.join(p),
                None => [pasta_dos_filhos.join(format!("{}.rs", m.nome)), pasta_dos_filhos.join(&m.nome).join("mod.rs")]
                    .into_iter()
                    .find(|p| p.is_file())
                    .ok_or_else(|| format!("{}: `mod {}` sem arquivo", arquivo.display(), m.nome))?,
            };
            pilha.push(filho);
        }
        v.push(arquivo);
    }
    Ok(v)
}

/// Gera, confere no próprio binário e grava a tabela dos endereços citados em `arquivos`. `false` = algum fora do lugar.
fn gerar_tabela(img: &en::Imagem, arquivos: &[PathBuf], saida: &str, rotulo: &str) -> bool {
    let mut alvos = Vec::new();
    for p in arquivos {
        alvos.extend(en::enderecos_no_fonte(&std::fs::read_to_string(p).unwrap_or_default()));
    }
    alvos.sort_unstable();
    alvos.dedup();
    let (tabela, faltam) = en::gerar(img, &alvos);
    // A própria tabela resolvida no binário de origem tem de dar a identidade.
    let volta = en::resolver(img, &tabela);
    let erradas = volta.iter().filter(|(s, r)| s != r).count();
    let mut motivos = std::collections::BTreeMap::new();
    for (_, m) in &faltam {
        *motivos.entry(format!("{m:?}")).or_insert(0) += 1;
    }
    let bytes = en::serializar(&tabela);
    std::fs::write(saida, &bytes).expect("saída");
    println!(
        "{rotulo}: {} arquivos, {} endereços no fonte: {} com assinatura ({} de código, {} de dado), sem assinatura {:?}; no próprio binário {} de {} resolvidos, {} fora do lugar; {} B",
        arquivos.len(),
        alvos.len(),
        tabela.len(),
        tabela.iter().filter(|s| matches!(s.tipo, en::Tipo::Codigo { .. })).count(),
        tabela.iter().filter(|s| matches!(s.tipo, en::Tipo::Dado { .. })).count(),
        motivos,
        volta.len(),
        tabela.len(),
        erradas,
        bytes.len()
    );
    erradas == 0
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 5 {
        eprintln!("uso: gerar_enderecos <binário Steam> <pasta src> <saída inteira .bin> <saída pública .bin>");
        std::process::exit(2);
    }
    let bin = std::fs::read(&a[1]).expect("binário");
    let img = en::Imagem::do_arquivo(&bin).expect("Mach-O arm64 com __text");
    // Os endereços do código são do Steam 2.5.0: um binário que não tem o executor no lugar é outro build, e a tabela fica. O prólogo sozinho
    // não basta (o mesmo `stp x28,x27` + `stp x26,x25` abre muitas funções); o `bl` em +0x8c tem de ir ao 1º auxiliar do executor.
    let exec = ((0x1_0109_9fbc - img.texto_vm) / 4) as usize;
    let prologo = (en::palavra(img.texto, exec + 1) as u64) << 32 | en::palavra(img.texto, exec) as u64;
    let bl = en::palavra(img.texto, exec + 0x8c / 4);
    let alvo_do_bl = (0x1_0109_9fbc_i64 + 0x8c) + ((((bl & 0x03ff_ffff) as i64) << 38) >> 36);
    if prologo != 0xa901_67fa_a9ba_6ffc || bl & 0xfc00_0000 != 0x9400_0000 || alvo_do_bl != 0x1_0109_8c5c {
        eprintln!("o binário não é o Steam 2.5.0 (prólogo do executor {prologo:#x}, bl {bl:#x}); tabela mantida");
        return;
    }
    let src = Path::new(&a[2]);
    let publico = match arvore_de_modulos(&src.join("lib.rs"), &[]) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("árvore de módulos do build público: {e}; tabelas mantidas");
            std::process::exit(1);
        }
    };
    let inteira_ok = gerar_tabela(&img, &todos_os_rs(src), &a[3], "inteira");
    let publica_ok = gerar_tabela(&img, &publico, &a[4], "pública");
    if !(inteira_ok && publica_ok) {
        std::process::exit(1);
    }
}
