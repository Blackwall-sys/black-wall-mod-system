//! Onde o `bwms mods` acha o `scc` quando roda da árvore do projeto (`<raiz>/bwms/target/<perfil>/bwms`): o canônico é o
//! `redscript/dist/redscript-macos/engine/tools/scc`, o mesmo dos roteiros da raiz. Roda uma cópia do binário numa árvore
//! sintética no TMPDIR, com um `scc` falso que só marca que rodou; nunca toca o jogo nem a árvore real.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;

const BINARIO: &str = env!("CARGO_BIN_EXE_bwms");

/// `<base>/raiz/bwms/target/release/bwms` (cópia do binário do teste) e um jogo mínimo com o cache que o `checar` copia.
fn arvore(teste: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("bwms-scc-{}-{teste}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let exe = base.join("raiz/bwms/target/release/bwms");
    fs::create_dir_all(exe.parent().expect("pasta do binário")).expect("árvore do teste no TMPDIR");
    fs::copy(BINARIO, &exe).expect("copiar o bwms");
    let jogo = base.join("jogo");
    fs::create_dir_all(jogo.join("r6/scripts")).expect("r6/scripts do jogo sintético");
    fs::create_dir_all(jogo.join("r6/cache")).expect("r6/cache do jogo sintético");
    fs::write(jogo.join("r6/cache/final.redscripts"), b"cache").expect("final.redscripts do jogo sintético");
    (base, jogo)
}

fn checar(base: &Path, jogo: &Path) -> Output {
    std::process::Command::new(base.join("raiz/bwms/target/release/bwms"))
        .args(["mods", "checar"])
        .arg(jogo)
        // sem scc no PATH: o achado tem de vir da árvore
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("executar o bwms")
}

#[test]
fn checar_roda_o_scc_de_redscript_dist() {
    let (base, jogo) = arvore("dist");
    let scc = base.join("raiz/redscript/dist/redscript-macos/engine/tools/scc");
    let marca = base.join("scc-rodou.txt");
    fs::create_dir_all(scc.parent().expect("pasta do scc")).expect("pasta do scc");
    fs::write(&scc, format!("#!/bin/sh\necho \"$0\" > '{}'\n", marca.display())).expect("scc falso");
    fs::set_permissions(&scc, fs::Permissions::from_mode(0o755)).expect("modo do scc falso");

    let saida = checar(&base, &jogo);
    let texto = String::from_utf8_lossy(&saida.stdout);
    assert!(!texto.contains("scc não encontrado"), "{texto}");
    let rodado = fs::read_to_string(&marca).expect("o scc da árvore não rodou");
    assert_eq!(
        fs::canonicalize(rodado.trim()).expect("caminho que o scc gravou"),
        fs::canonicalize(&scc).expect("caminho do scc falso")
    );
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn checar_sem_scc_na_arvore_avisa() {
    let (base, jogo) = arvore("sem");
    let saida = checar(&base, &jogo);
    assert!(String::from_utf8_lossy(&saida.stdout).contains("scc não encontrado"));
    assert!(!saida.status.success());
    let _ = fs::remove_dir_all(&base);
}
