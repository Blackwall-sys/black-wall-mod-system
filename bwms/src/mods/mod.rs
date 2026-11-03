//! `bwms mods checar` — compila os scripts do jogo numa saída temporária, sem tocar no cache, e mostra os erros (com as dicas para mod
//! feito para o Windows) e cada `native func` que nem o BWMS nem um plugin registra, com arquivo e linha. Sem crate de terceiro: std +
//! bwms-primitives.

mod runtime;
mod scc_api;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
bwms mods — checagem dos scripts do jogo (macOS)

USO:
    bwms mods checar [game] [--extra <pasta>]  compila os scripts sem tocar no cache, mostra os erros (com dicas)
                                               e aponta arquivo e linha de cada native func que nenhum dylib registra

[game] = raiz do jogo; default = $CP77_DIR ou o caminho Steam conhecido.
Mod entra no jogo pela pasta Mods/ do jogo ou pelo menu Mods do próprio jogo (mod.io).
";

/// Raiz do jogo: arg explícito → $CP77_DIR → caminho Steam conhecido.
fn game_root(arg: Option<&String>) -> Option<PathBuf> {
    if let Some(a) = arg {
        let p = PathBuf::from(a);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(d) = std::env::var("CP77_DIR") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Some(h) = std::env::var_os("HOME").map(PathBuf::from) {
        let p = h.join("Library/Application Support/Steam/steamapps/common/Cyberpunk 2077");
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// `bwms mods …`. `args` vem como o argv de um binário: `args[0]` é o próprio `mods` e o subcomando está em `args[1]`.
pub fn run(args: &[String]) -> ExitCode {
    match args.get(1).map(String::as_str) {
        Some("checar") => cmd_checar(game_root(args.get(2).filter(|a| !a.starts_with("--"))), args.iter().position(|a| a == "--extra").and_then(|i| args.get(i + 1)).map(PathBuf::from)),
        None | Some("-h") | Some("--help") | Some("help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("comando desconhecido '{other}': o bwms mods só checa os scripts (bwms mods checar). Mod entra no jogo pela pasta Mods/ ou pelo menu Mods do jogo.");
            ExitCode::from(2)
        }
    }
}

fn cmd_checar(game: Option<PathBuf>, extra: Option<PathBuf>) -> ExitCode {
    let Some(game) = game else {
        eprintln!("erro: raiz do jogo não encontrada (defina CP77_DIR ou passe [game]).");
        return ExitCode::from(1);
    };
    let (passou, relatorio) = runtime::checar_redscript(&game, extra.as_deref());
    println!("{relatorio}");
    if passou { ExitCode::SUCCESS } else { ExitCode::from(1) }
}
