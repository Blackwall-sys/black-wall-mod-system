//! Mini ferramenta: gera uma tabela de runtime a partir de `.xl`, sem o `bwms install` — o mesmo emissor que o install usa.
//!
//!     cargo run -q --example emitir_tabela -- <streaming|questphase|journal|resource-patch> <mod>=<arquivo.xl> [<mod>=<arquivo.xl> ...] > bwms-<tabela>.txt
//!
//! Cada `.xl` vira as linhas do emissor com o id de mod dado; `.xl` que não parseia aborta com a mensagem do parser. O `resource-patch` vê todos os
//! `.xl` de uma vez, na ordem dos argumentos, com os escopos de todos, como o install.
use bwms_primitives::instalacao::tabelas_de_runtime::{emit_journal_table, emit_questphase_table, emit_resource_patch_table, emit_streaming_table, resource_scope_listas};
use bwms_primitives::instalacao::xl::XlFile;
use std::env;
use std::fs;
use std::process::exit;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let emite: Option<fn(&XlFile, &str) -> String> = match args.first().map(String::as_str) {
        Some("streaming") => Some(emit_streaming_table),
        Some("questphase") => Some(emit_questphase_table),
        Some("journal") => Some(emit_journal_table),
        Some("resource-patch") => None,
        _ => {
            eprintln!("uso: emitir_tabela <streaming|questphase|journal|resource-patch> <mod>=<arquivo.xl> [...]");
            exit(2);
        }
    };
    if args.len() < 2 {
        eprintln!("falta ao menos um <mod>=<arquivo.xl>");
        exit(2);
    }
    let mut xls: Vec<(String, XlFile)> = Vec::new();
    for par in &args[1..] {
        let Some((modulo, caminho)) = par.split_once('=') else {
            eprintln!("argumento sem '=': {par}");
            exit(2);
        };
        let texto = fs::read_to_string(caminho).unwrap_or_else(|e| {
            eprintln!("não leu {caminho}: {e}");
            exit(1)
        });
        let xl = bwms_primitives::instalacao::xl::parse_xl(&texto).unwrap_or_else(|e| {
            eprintln!("{caminho} não parseia: {e}");
            exit(1)
        });
        xls.push((modulo.to_string(), xl));
    }
    let Some(emite) = emite else {
        let lidos: Vec<&XlFile> = xls.iter().map(|(_, xl)| xl).collect();
        print!("{}", emit_resource_patch_table(&xls, &resource_scope_listas(&lidos).0));
        return;
    };
    let mut saida = String::new();
    for (modulo, xl) in &xls {
        for linha in emite(xl, modulo).lines() {
            if !linha.starts_with('#') || saida.is_empty() {
                saida.push_str(linha);
                saida.push('\n');
            }
        }
    }
    print!("{saida}");
}
