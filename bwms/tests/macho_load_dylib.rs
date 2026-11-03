//! Contrato do `bwms-helper` com o `bwms-install.sh`: `has` sai 0/1, `info` lista os LC_LOAD_DYLIB e `insert` grava o load
//! command na folga do header sem deslocar o arquivo e sem mudar o modo. Roda o binário compilado sobre um Mach-O sintético no
//! TMPDIR, nunca sobre o executável do jogo.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

const BINARIO: &str = env!("CARGO_BIN_EXE_bwms-helper");
const LIB_DO_SISTEMA: &str = "/usr/lib/libSystem.B.dylib";
// O caminho que o bwms-install.sh grava; ele confere a instalação com `has <bin> libcp77_console.dylib`.
const LIB_DO_BWMS: &str = "@executable_path/../../../red4ext/libcp77_console.dylib";

/// Mach-O 64 thin com um LC_SEGMENT_64 (uma seção, conteúdo a partir de `inicio_da_secao`) e um LC_LOAD_DYLIB.
fn macho(inicio_da_secao: u32) -> Vec<u8> {
    let mut lcs = Vec::new();
    lcs.extend_from_slice(&0x19u32.to_le_bytes());
    lcs.extend_from_slice(&(72u32 + 80).to_le_bytes());
    // segname, vmaddr, vmsize, fileoff, filesize, maxprot, initprot
    lcs.extend_from_slice(&[0u8; 16 + 8 * 4 + 4 * 2]);
    lcs.extend_from_slice(&1u32.to_le_bytes());
    lcs.extend_from_slice(&0u32.to_le_bytes());
    let mut secao = [0u8; 80];
    secao[48..52].copy_from_slice(&inicio_da_secao.to_le_bytes());
    lcs.extend_from_slice(&secao);
    let mut nome = LIB_DO_SISTEMA.as_bytes().to_vec();
    nome.push(0);
    let tamanho = (24 + nome.len() + 7) & !7;
    for v in [0xCu32, tamanho as u32, 24, 2, 0x0001_0000, 0x0001_0000] {
        lcs.extend_from_slice(&v.to_le_bytes());
    }
    lcs.extend_from_slice(&nome);
    lcs.resize(lcs.len() + tamanho - 24 - nome.len(), 0);

    let mut d = Vec::new();
    for v in [0xFEED_FACFu32, 0x0100_000C, 0, 2, 2, lcs.len() as u32, 0, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&lcs);
    d.resize(inicio_da_secao as usize, 0);
    d.extend_from_slice(&[0xAA; 16]);
    d
}

fn fim_dos_load_commands(d: &[u8]) -> u32 {
    32 + u32::from_le_bytes([d[20], d[21], d[22], d[23]])
}

fn pasta(teste: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("bwms-helper-{}-{teste}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).expect("pasta do teste no TMPDIR");
    p
}

fn roda(args: &[&str]) -> Output {
    Command::new(BINARIO).args(args).output().expect("executar o bwms-helper")
}

fn saida(o: &Output) -> (Option<i32>, String, String) {
    (o.status.code(), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
}

#[test]
fn info_e_has_leem_os_load_dylib() {
    let dir = pasta("leitura");
    let bin = dir.join("alvo");
    fs::write(&bin, macho(0x400)).expect("gravar o alvo");
    let b = bin.to_str().expect("caminho UTF-8");

    assert_eq!(saida(&roda(&["info", b])), (Some(0), format!("{LIB_DO_SISTEMA}\n"), String::new()));
    assert_eq!(roda(&["has", b, "libSystem"]).status.code(), Some(0));
    assert_eq!(roda(&["has", b, "libcp77_console.dylib"]).status.code(), Some(1));
    // Sem substring nada casa: `has` sai 1.
    assert_eq!(roda(&["has", b]).status.code(), Some(1));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn insert_grava_na_folga_preserva_modo_e_e_idempotente() {
    let dir = pasta("insert");
    let bin = dir.join("alvo");
    let original = macho(0x400);
    fs::write(&bin, &original).expect("gravar o alvo");
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o750)).expect("modo do alvo");
    let b = bin.to_str().expect("caminho UTF-8");

    let cmdsize = (24 + LIB_DO_BWMS.len() + 1 + 7) & !7;
    assert_eq!(
        saida(&roda(&["insert", b, LIB_DO_BWMS])),
        (Some(0), format!("OK: load command added (cmdsize={cmdsize})\n"), String::new())
    );
    let gravado = fs::read(&bin).expect("ler o alvo");
    assert_eq!(gravado.len(), original.len(), "o insert sobrescreve a folga, não desloca o arquivo");
    assert_eq!(&gravado[0x400..], &original[0x400..], "o conteúdo da seção fica no mesmo offset");
    assert_eq!(u32::from_le_bytes([gravado[16], gravado[17], gravado[18], gravado[19]]), 3);
    assert_eq!(fim_dos_load_commands(&gravado), fim_dos_load_commands(&original) + cmdsize as u32);
    assert_eq!(fs::metadata(&bin).expect("metadados").permissions().mode() & 0o777, 0o750);
    let restos: Vec<_> = fs::read_dir(&dir).expect("listar a pasta").filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
    assert_eq!(restos, vec![std::ffi::OsString::from("alvo")], "o temp .bwms-tmp some no rename");

    assert_eq!(roda(&["has", b, "libcp77_console.dylib"]).status.code(), Some(0));
    assert_eq!(saida(&roda(&["info", b])).1, format!("{LIB_DO_SISTEMA}\n{LIB_DO_BWMS}\n"));

    assert_eq!(
        saida(&roda(&["insert", b, LIB_DO_BWMS])),
        (Some(0), "already installed — nothing to do\n".to_string(), String::new())
    );
    assert_eq!(fs::read(&bin).expect("ler o alvo"), gravado);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn insert_sem_folga_falha_e_nao_toca_o_arquivo() {
    let dir = pasta("sem-folga");
    let bin = dir.join("alvo");
    let cmdsize = (24 + LIB_DO_BWMS.len() + 1 + 7) & !7;
    // Folga de 8 bytes a menos que o load command novo.
    let apertado = macho(fim_dos_load_commands(&macho(0x400)) + cmdsize as u32 - 8);
    fs::write(&bin, &apertado).expect("gravar o alvo");
    let b = bin.to_str().expect("caminho UTF-8");

    let (codigo, out, err) = saida(&roda(&["insert", b, LIB_DO_BWMS]));
    assert_eq!((codigo, out.as_str()), (Some(1), ""));
    assert!(err.contains("no room in the header"), "o bwms-install.sh orienta por esta frase: {err}");
    assert_eq!(fs::read(&bin).expect("ler o alvo"), apertado);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn entrada_que_nao_e_macho_thin_sai_com_erro() {
    let dir = pasta("invalido");
    let txt = dir.join("texto.txt");
    fs::write(&txt, b"nao e um Mach-O\n").expect("gravar o texto");
    let t = txt.to_str().expect("caminho UTF-8");

    assert_eq!(saida(&roda(&["has", t, "x"])), (Some(1), String::new(), "not a thin 64-bit Mach-O (arm64)\n".to_string()));
    assert_eq!(roda(&["insert", t, LIB_DO_BWMS]).status.code(), Some(1));
    assert_eq!(fs::read(&txt).expect("ler o texto"), b"nao e um Mach-O\n");
    assert_eq!(roda(&["info"]).status.code(), Some(2));
    assert_eq!(roda(&["foo", t]).status.code(), Some(1), "o helper confere o tipo do arquivo e só então o comando");
    let _ = fs::remove_dir_all(&dir);
}
