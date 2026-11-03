//! Mini ferramenta: gera o conteúdo de `red4ext/bwms-streaming.txt` a partir de `.xl`, sem o `bwms install`.
//!
//!     cargo run -q --example emitir_streaming -- <mod>=<arquivo.xl> [<mod>=<arquivo.xl> ...] > bwms-streaming.txt
//!
//! Cada `.xl` vira as linhas de `emit_streaming_table` com o id de mod dado; `.xl` que não parseia aborta com a mensagem do parser.
use std::env;
use std::fs;
use std::process::exit;

fn main() {
    let pares: Vec<String> = env::args().skip(1).collect();
    if pares.is_empty() {
        eprintln!("uso: emitir_streaming <mod>=<arquivo.xl> [...]");
        exit(2);
    }
    let mut saida = String::from("# bwms-streaming (gerado pela mini ferramenta emitir_streaming)\n");
    for par in &pares {
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
        for linha in bwms_primitives::instalacao::tabelas_de_runtime::emit_streaming_table(&xl, modulo).lines().filter(|l| !l.starts_with('#')) {
            saida.push_str(linha);
            saida.push('\n');
        }
    }
    print!("{saida}");
}
