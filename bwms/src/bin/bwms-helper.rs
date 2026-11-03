//! bwms-helper — substitui `otool`/`python3` (que são Xcode Command Line Tools e
//! NÃO existem num Mac de fábrica) por um binário nativo arm64 auto-contido.
//! Lê e edita Mach-O 64 thin (o binário do Cyberpunk no mac) e confere os `.reds` contra o dylib:
//!   has     <bin> <substr>             -> exit 0 se um LC_LOAD_DYLIB contém <substr>, senão 1
//!   info    <bin>                      -> lista os caminhos de LC_LOAD_DYLIB (1 por linha)
//!   insert  <bin> <dylib>              -> adiciona um LC_LOAD_DYLIB (na padding do header)
//!   natives <dylib> <.reds|pasta>...   -> lista as `native func` declaradas que o dylib não registra (exit 0 nenhuma, 1 alguma, 2 erro)
//!   migrar  <jogo>                     -> leva os mods ligados da biblioteca antiga para a `Mods/` (exit 0 feito, 1 pendente, 2 erro)
//! Só `std`, o andador de comandos de carga, a conferência de natives e a migração do `bwms-primitives` (do projeto, zero-dep), sem CLT,
//! roda no Mac liso. (codesign e xattr já são base.)

use bwms_primitives::macho::{comandos_de_carga, LC_LOAD_DYLIB, LC_SEGMENT_64, TAMANHO_DO_CABECALHO};
use bwms_primitives::native_registrada::{declaradas_em, registrada_em, registrada_no_jogo};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

fn rd_u32(d: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([d[off], d[off + 1], d[off + 2], d[off + 3]])
}

fn str_at(d: &[u8], s: usize) -> (&[u8], usize) {
    let end = d[s..].iter().position(|&b| b == 0).map(|p| s + p).unwrap_or(s);
    (&d[s..end], end)
}

/// Os `.reds` de `caminho`: o próprio arquivo, ou os da pasta em qualquer profundidade, como o `find <pasta> -name '*.reds'` (link de
/// pasta não é seguido).
fn reds_em(caminho: &Path, achados: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !fs::metadata(caminho)?.is_dir() {
        achados.push(caminho.to_path_buf());
        return Ok(());
    }
    let mut pilha = vec![caminho.to_path_buf()];
    while let Some(pasta) = pilha.pop() {
        for entrada in fs::read_dir(&pasta)? {
            let entrada = entrada?;
            let p = entrada.path();
            if entrada.file_type()?.is_dir() {
                pilha.push(p);
            } else if p.extension().is_some_and(|x| x == "reds") {
                achados.push(p);
            }
        }
    }
    Ok(())
}

/// `natives <dylib> <.reds|pasta>...`: a checagem canônica de native declarada × registrada. O `bwms-install.sh` (passo 6, antes de
/// compilar os scripts) e o `dist/pack-bwms.sh` (gate `[2c/4]`) rodam esta: uma `native func` que o dylib não registra derruba o bind dos
/// scripts com SIGTRAP, e o jogo fecha sem mensagem. Os nomes sem registro vão para stdout (1 por linha, em ordem), o resumo ou o erro para
/// stderr. Devolve o código de saída: 0 todas registradas, 1 alguma falta, 2 uso ou leitura.
fn natives(args: &[String]) -> i32 {
    let Some((dylib, fontes)) = args.split_first().filter(|(_, fontes)| !fontes.is_empty()) else {
        eprintln!("usage: bwms-helper natives <dylib> <file.reds|folder>...");
        return 2;
    };
    let binario = match fs::read(dylib) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error reading {dylib}: {e}");
            return 2;
        }
    };
    if comandos_de_carga(&binario).is_none() {
        eprintln!("{dylib}: not a thin 64-bit Mach-O (arm64)");
        return 2;
    }
    let mut arquivos = Vec::new();
    for fonte in fontes {
        if let Err(e) = reds_em(Path::new(fonte), &mut arquivos) {
            eprintln!("error reading {fonte}: {e}");
            return 2;
        }
    }
    let mut declaradas = BTreeSet::new();
    for arquivo in &arquivos {
        match fs::read(arquivo) {
            Ok(texto) => declaradas.extend(declaradas_em(&String::from_utf8_lossy(&texto)).iter().map(|d| d.nome.to_owned())),
            Err(e) => {
                eprintln!("error reading {}: {e}", arquivo.display());
                return 2;
            }
        }
    }
    let binarios = [binario];
    let mut faltam: Vec<&String> = declaradas.iter().filter(|nome| !registrada_em(nome, &binarios)).collect();
    // O que o dylib não registra pode ser native do próprio jogo que a ponte declara de novo: com `BWMS_JOGO=<executável>` (o instalador
    // passa o binário do jogo), ela conta como registrada. O executável só é lido nesse caso.
    if !faltam.is_empty() {
        if let Some(exe) = std::env::var_os("BWMS_JOGO").and_then(|p| fs::read(p).ok()) {
            faltam.retain(|nome| !registrada_no_jogo(nome, &exe));
        }
    }
    for nome in &faltam {
        println!("{nome}");
    }
    let nome_do_dylib = Path::new(dylib).file_name().map_or_else(|| dylib.into(), |n| n.to_string_lossy());
    eprintln!(
        "natives: {} declared in {} .reds files, {} missing from {nome_do_dylib}",
        declaradas.len(),
        arquivos.len(),
        faltam.len()
    );
    i32::from(!faltam.is_empty())
}

/// `migrar <jogo>`: leva os mods ligados da biblioteca da 0.1.2–0.1.4 para a `Mods/` e tira do jogo o que ela pôs lá
/// (`bwms_primitives::instalacao::migracao`). O `bwms-install.sh` roda esta antes de compilar os scripts. Imprime uma linha por item (o
/// mesmo texto que o relatório da `Mods/` mostra) e devolve 0 com tudo feito, 1 com algo que ficou para a próxima instalação e 2 sem
/// conseguir gravar o registro.
fn migrar(args: &[String]) -> i32 {
    use bwms_primitives::instalacao::migracao::{self, Item};
    let Some(jogo) = args.first() else {
        eprintln!("usage: bwms-helper migrar <game folder>");
        return 2;
    };
    match migracao::migrar(Path::new(jogo)) {
        Ok(itens) => {
            for i in &itens {
                println!("  · {}", i.texto());
            }
            let pendente = itens.iter().any(|i| matches!(i, Item::Falhou { .. } | Item::LimpezaAdiada | Item::FicouNoJogo { .. } | Item::Ilegivel { .. }));
            i32::from(pendente)
        }
        Err(e) => {
            eprintln!("  · {e}");
            2
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: bwms-helper <has|info|insert> <binary> [substr|dylib]");
        eprintln!("       bwms-helper natives <dylib> <file.reds|folder>...");
        eprintln!("       bwms-helper migrar <game folder>");
        exit(2);
    }
    // `natives` lê o dylib e os `.reds` por conta própria e tem o 1 reservado para "native sem registro".
    if args[1] == "natives" {
        exit(natives(&args[2..]));
    }
    if args[1] == "migrar" {
        exit(migrar(&args[2..]));
    }
    let cmd = args[1].as_str();
    let path = &args[2];
    let mut data = match fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error reading {path}: {e}");
            exit(1);
        }
    };
    let Some(comandos) = comandos_de_carga(&data) else {
        eprintln!("not a thin 64-bit Mach-O (arm64)");
        exit(1);
    };
    let ncmds = rd_u32(&data, 16);
    let sizeofcmds = rd_u32(&data, 20) as usize;
    let lc_end = TAMANHO_DO_CABECALHO + sizeofcmds;

    match cmd {
        "has" | "info" => {
            let needle = args.get(3).map(|s| s.as_str()).unwrap_or("");
            let mut found = false;
            for c in comandos.filter(|c| c.cmd == LC_LOAD_DYLIB) {
                let nameoff = rd_u32(&data, c.offset + 8) as usize;
                let (name, _) = str_at(&data, c.offset + nameoff);
                let name = String::from_utf8_lossy(name);
                if cmd == "info" {
                    println!("{name}");
                }
                if !needle.is_empty() && name.contains(needle) {
                    found = true;
                }
            }
            if cmd == "has" {
                exit(if found { 0 } else { 1 });
            }
        }
        "insert" => {
            let dylib = match args.get(3) {
                Some(s) => s.clone(),
                None => {
                    eprintln!("insert requires <dylib>");
                    exit(2);
                }
            };
            // 1a passada (só leitura): idempotência + headroom da padding.
            let mut headroom_limit = data.len();
            for c in comandos {
                if c.cmd == LC_LOAD_DYLIB {
                    let nameoff = rd_u32(&data, c.offset + 8) as usize;
                    let (name, _) = str_at(&data, c.offset + nameoff);
                    if name == dylib.as_bytes() {
                        println!("already installed — nothing to do");
                        return;
                    }
                } else if c.cmd == LC_SEGMENT_64 {
                    let nsects = rd_u32(&data, c.offset + 64) as usize;
                    for sx in 0..nsects {
                        let p = c.offset + 72 + sx * 80 + 48;
                        if p + 4 <= data.len() {
                            let soff = rd_u32(&data, p) as usize;
                            if soff > 0 && soff < headroom_limit {
                                headroom_limit = soff;
                            }
                        }
                    }
                }
            }
            let mut name = dylib.into_bytes();
            name.push(0);
            let cmdsize_new = (24 + name.len() + 7) & !7;
            if headroom_limit < lc_end || headroom_limit - lc_end < cmdsize_new {
                eprintln!("no room in the header (needs {cmdsize_new}B). Restore via Steam.");
                exit(1);
            }
            let mut lc: Vec<u8> = Vec::with_capacity(cmdsize_new);
            lc.extend_from_slice(&LC_LOAD_DYLIB.to_le_bytes());
            lc.extend_from_slice(&(cmdsize_new as u32).to_le_bytes());
            lc.extend_from_slice(&24u32.to_le_bytes()); // name.offset
            lc.extend_from_slice(&2u32.to_le_bytes()); // timestamp
            lc.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // current_version
            lc.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // compat_version
            lc.extend_from_slice(&name);
            lc.resize(cmdsize_new, 0);
            // OVERWRITE a padding (sem deslocar o arquivo — fileoffs continuam validos)
            data[lc_end..lc_end + cmdsize_new].copy_from_slice(&lc);
            data[16..20].copy_from_slice(&(ncmds + 1).to_le_bytes());
            data[20..24].copy_from_slice(&((sizeofcmds + cmdsize_new) as u32).to_le_bytes());
            // Escreve via temp + rename (replace atômico do inode): truncar IN-PLACE um
            // Mach-O já assinado e VALIDADO pelo kernel (binário já lançado uma vez) dá
            // EPERM por proteção de code-signing. Trocar o inode contorna — o dir só
            // precisa ser writable (já é). Vale Steam e GOG.
            let tmp = format!("{path}.bwms-tmp");
            if let Err(e) = fs::write(&tmp, &data) {
                eprintln!("error writing temp: {e}");
                exit(1);
            }
            // CRÍTICO: fs::write cria o arquivo 0644 (SEM bit de execução). Renomear por cima
            // deixaria o binário não-executável → launchd "spawn failed" (erro 111) / zsh
            // "permission denied". Copia o modo do ORIGINAL (que é executável) pro temp.
            match fs::metadata(path).map(|m| m.permissions()) {
                Ok(perm) => {
                    if let Err(e) = fs::set_permissions(&tmp, perm) {
                        let _ = fs::remove_file(&tmp);
                        eprintln!("error setting permissions: {e}");
                        exit(1);
                    }
                }
                Err(e) => {
                    let _ = fs::remove_file(&tmp);
                    eprintln!("error reading original permissions: {e}");
                    exit(1);
                }
            }
            if let Err(e) = fs::rename(&tmp, path) {
                let _ = fs::remove_file(&tmp);
                eprintln!("error replacing binary: {e}");
                exit(1);
            }
            println!("OK: load command added (cmdsize={cmdsize_new})");
        }
        other => {
            eprintln!("unknown command: {other}");
            exit(2);
        }
    }
}
