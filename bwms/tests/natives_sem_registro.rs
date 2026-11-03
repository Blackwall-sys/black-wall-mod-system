//! Contrato do `bwms-helper natives` com o `bwms-install.sh` (passo 6) e o `dist/pack-bwms.sh` (gate `[2c/4]`): os nomes sem registro em
//! stdout, 1 por linha e em ordem; código 0 = todas registradas, 1 = alguma falta, 2 = uso ou leitura. Roda o binário compilado sobre um
//! bundle sintético no TMPDIR, nunca sobre a pasta do jogo.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BINARIO: &str = env!("CARGO_BIN_EXE_bwms-helper");

/// Dylib de teste: cabeçalho de Mach-O 64 thin sem comandos de carga e os nomes colados depois, sem zero entre eles, como o Rust deixa os
/// literais de texto.
fn dylib(nomes: &[&str]) -> Vec<u8> {
    let mut d = Vec::new();
    for v in [0xFEED_FACFu32, 0x0100_000C, 0, 6, 0, 0, 0, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    for n in nomes {
        d.extend_from_slice(n.as_bytes());
    }
    d
}

fn pasta(teste: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("bwms-natives-{}-{teste}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).expect("pasta do teste no TMPDIR");
    p
}

fn grava(caminho: &Path, conteudo: &[u8]) {
    fs::create_dir_all(caminho.parent().expect("pasta do arquivo")).expect("criar a pasta do arquivo");
    fs::write(caminho, conteudo).expect("gravar o arquivo do teste");
}

fn natives(args: &[&Path]) -> (Option<i32>, String, String) {
    let o: Output = Command::new(BINARIO).arg("natives").args(args).output().expect("executar o bwms-helper");
    (o.status.code(), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
}

/// `engine/libcp77_console.dylib` e `engine/redscript/blackwall-mods/` com uma subpasta, como no pacote.
fn bundle(teste: &str, registradas: &[&str]) -> (PathBuf, PathBuf, PathBuf) {
    let base = pasta(teste);
    let dylib_do_bundle = base.join("engine/libcp77_console.dylib");
    grava(&dylib_do_bundle, &dylib(registradas));
    let reds = base.join("engine/redscript/blackwall-mods");
    grava(
        &reds.join("a.reds"),
        b"public static native func BwmsExiste() -> Bool;\nnative func BwmsOutra(x: Int32)\n// native func BwmsComentada()\n",
    );
    grava(&reds.join("sub/b.reds"), b"  native func BwmsFalta()\npublic native func BwmsFaltaTambem()\nnative func BwmsExiste() -> Bool\n");
    grava(&reds.join("leia.txt"), b"native func BwmsForaDeReds()\n");
    (base, dylib_do_bundle, reds)
}

#[test]
fn acusa_so_as_declaradas_que_o_dylib_nao_tem() {
    let (base, dylib_do_bundle, reds) = bundle("falta", &["xxBwmsExiste", "BwmsOutraZ"]);
    let (codigo, out, err) = natives(&[&dylib_do_bundle, &reds]);
    assert_eq!((codigo, out.as_str()), (Some(1), "BwmsFalta\nBwmsFaltaTambem\n"));
    assert_eq!(err, "natives: 4 declared in 2 .reds files, 2 missing from libcp77_console.dylib\n");
    // O pack passa a raiz do pacote inteira; o instalador, a pasta dos scripts. O resultado é o mesmo.
    assert_eq!(natives(&[&dylib_do_bundle, &base]).1, out);
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn tudo_registrado_sai_zero_sem_nada_em_stdout() {
    let (base, dylib_do_bundle, reds) = bundle("ok", &["BwmsExisteBwmsOutraBwmsFaltaBwmsFaltaTambem"]);
    assert_eq!(
        natives(&[&dylib_do_bundle, &reds]),
        (Some(0), String::new(), "natives: 4 declared in 2 .reds files, 0 missing from libcp77_console.dylib\n".into())
    );
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn aceita_arquivo_e_varias_fontes() {
    let (base, dylib_do_bundle, reds) = bundle("arquivos", &["BwmsExiste"]);
    let (codigo, out, _) = natives(&[&dylib_do_bundle, &reds.join("a.reds"), &reds.join("sub/b.reds")]);
    assert_eq!((codigo, out.as_str()), (Some(1), "BwmsFalta\nBwmsFaltaTambem\nBwmsOutra\n"));
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn erro_de_uso_ou_leitura_sai_dois() {
    let (base, dylib_do_bundle, reds) = bundle("erros", &["BwmsExiste"]);
    let texto = base.join("nao-e-macho.dylib");
    grava(&texto, b"BwmsExisteBwmsOutraBwmsFaltaBwmsFaltaTambem");
    let ausente = base.join("ausente");

    let (codigo, out, err) = natives(&[&dylib_do_bundle]);
    assert_eq!((codigo, out.as_str()), (Some(2), ""));
    assert!(err.starts_with("usage: bwms-helper natives"), "{err}");
    assert_eq!(natives(&[&ausente, &reds]).0, Some(2), "dylib que não existe");
    let (codigo, _, err) = natives(&[&texto, &reds]);
    assert_eq!(codigo, Some(2), "dylib que não é Mach-O");
    assert!(err.contains("not a thin 64-bit Mach-O"), "{err}");
    let (codigo, out, _) = natives(&[&dylib_do_bundle, &reds, &ausente]);
    assert_eq!((codigo, out.as_str()), (Some(2), ""), "fonte que não existe: nenhum veredito");
    assert_eq!(Command::new(BINARIO).arg("natives").output().expect("executar o bwms-helper").status.code(), Some(2));
    let _ = fs::remove_dir_all(&base);
}
