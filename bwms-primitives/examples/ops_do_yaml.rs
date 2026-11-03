//! Operações que um arquivo de tweak (`.yaml` ou `.tweak`) vira, sem o jogo: `cargo run --example ops_do_yaml -- <arquivo>`.
use bwms_primitives::operacoes::{EditOp, Op};

fn edicao(op: &EditOp) -> String {
    match op {
        EditOp::Assign(v) => format!("= {v}"),
        EditOp::Append(v) => format!("!append {v}"),
        EditOp::AppendOnce(v) => format!("!append-once {v}"),
        EditOp::Prepend(v) => format!("!prepend {v}"),
        EditOp::PrependOnce(v) => format!("!prepend-once {v}"),
        EditOp::Remove(v) => format!("!remove {v}"),
        _ => "(outra mutação)".to_string(),
    }
}

fn main() {
    let Some(caminho) = std::env::args().nth(1) else { return eprintln!("uso: ops_do_yaml <arquivo.yaml>") };
    let texto = std::fs::read_to_string(&caminho).unwrap_or_else(|e| panic!("{caminho}: {e}"));
    let ops = if caminho.ends_with(".tweak") {
        bwms_primitives::tweakxl::tweak_source::parse(&texto)
    } else {
        bwms_primitives::tweakxl::yaml::parse(&texto)
            .and_then(|r| bwms_primitives::tweakxl::template::expand(&r))
            .and_then(|r| bwms_primitives::tweakxl::interpret_from(&r, &caminho))
    };
    match ops {
        Ok(ops) => {
            for o in &ops {
                match o {
                    Op::Clone { record, base } => println!("clone  {record} <- {base}"),
                    Op::Create { record, class } => println!("create {record} : {class}"),
                    Op::Edit { flat, op } => println!("edit   {flat} {}", edicao(op)),
                    Op::Criar { flat, tipo, op } => println!("criar  {flat} ({tipo}) {}", edicao(op)),
                }
            }
        }
        Err(e) => println!("ERRO: {e}"),
    }
}
