//! archive-tool — CLI para o formato `.archive` (RDAR) do Cyberpunk 2077 no
//! macOS (Apple Silicon).
//!
//! Comandos:
//!   list    [<dir>]                       Lista os .archive (padrão: diretório do jogo).
//!   info    <nome|caminho>                Resumo de uma linha.
//!   datamap <nome|caminho> [-o ...]       Gera o datamap.md do índice.
//!   extract <nome|caminho> [<dest>]       Extrai para <dest>/<nome>/ (padrão: ao lado do archive).
//!   extract --all [<dest>]                Extrai TODOS os archives do jogo, cada um na sua pasta.
//!
//! Local do jogo: vem embutido (este Mac), e pode ser trocado por `CP77_CONTENT`
//! (aponta direto p/ a pasta content) ou `CP77_DIR` (raiz do jogo). Os nomes dos
//! recursos são resolvidos automaticamente pela `usedhashes.kark` do projeto.

mod archive;
mod cr2w;
mod datamap;
mod extract;
mod hashes;
mod kraken;
mod time;
mod writer;

use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use archive::Archive;
use hashes::PathDictionary;

/// Locais padrão deste Mac (descobertos uma vez; overridáveis por env).
mod defaults {
    use std::path::PathBuf;

    // Sem const fixa: a raiz vem de env (CP77_CONTENT/CP77_DIR) ou da instalação
    // Steam padrão sob o HOME do usuário (portável, não embute a máquina).

    /// Lista hash→path da comunidade (`usedhashes.kark` do WolvenKit), no projeto.
    /// Resolvida em tempo de compilação relativa ao crate; usada se existir.
    pub fn hashes_path() -> PathBuf {
        // ao lado do exe se existir; senão relativo (sem env! que embute o path do projeto).
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let beside = dir.join("usedhashes.kark");
                if beside.is_file() {
                    return beside;
                }
            }
        }
        PathBuf::from("usedhashes.kark")
    }

    pub fn content_dir() -> PathBuf {
        if let Some(c) = std::env::var_os("CP77_CONTENT") {
            return PathBuf::from(c);
        }
        if let Some(root) = std::env::var_os("CP77_DIR") {
            let root = PathBuf::from(root);
            let mac = root.join("archive/Mac/content");
            if mac.is_dir() {
                return mac;
            }
            let pc = root.join("archive/pc/content");
            if pc.is_dir() {
                return pc;
            }
            return mac;
        }
        if let Some(h) = std::env::var_os("HOME").map(PathBuf::from) {
            return h.join("Library/Application Support/Steam/steamapps/common/Cyberpunk 2077/archive/Mac/content");
        }
        PathBuf::from("archive/Mac/content")
    }
}

const USAGE: &str = "\
archive-tool — lê/extrai .archive (RDAR) do Cyberpunk 2077 (macOS)

USO:
    archive-tool list    [<dir>]
    archive-tool info    <nome|caminho>
    archive-tool datamap <nome|caminho> [-o <out.md|->] [--hashes <lista>] [--no-hashes]
    archive-tool extract <nome|caminho> [<dest>] [opções]
    archive-tool extract --all [<dest>] [opções]
    archive-tool cr2w    <resource-file>   (índice CR2W de um resource extraído)
    archive-tool locedit <onscreens.json> [<out>]  (edita localização + re-empacota)
    archive-tool pack    <cr2w> <game-path> <out.archive>  (empacota mod .archive)
    archive-tool packdir <pasta> <out.archive>  (empacota uma pasta-mod inteira)
    archive-tool extract-one <archive> <game-path> <out>  (1 recurso, sem extrair tudo)
    archive-tool c2dadd  <csv> <out> <linha>...  (adiciona linha a um C2dArray/factory)

O <nome> pode ser só o nome do archive (ex.: `basegame_2_mainmenu`): é procurado
no diretório do jogo, que já vem embutido. Override: CP77_CONTENT ou CP77_DIR.

COMANDOS:
    list      Lista os .archive do diretório do jogo (ou de <dir>).
    info      Resumo de uma linha (versão, contagens, tamanhos).
    datamap   Gera datamap.md do índice. Padrão: <archive>.datamap.md
    extract   Extrai os recursos. Por padrão cria, AO LADO do archive, uma pasta
              com o nome dele contendo o conteúdo: <dest>/<nome>/...
              Com --all, faz isso para todos os archives do jogo.
    cr2w      Lê o ÍNDICE CR2W de um resource já extraído (magic + FileHeader + as
              10 tabelas + string dict + names + chunks). Self-checks: round-trip do
              índice/chunks + crc32 (header+tabelas) + re-pack. Se for localization
              onscreens, extrai as primeiras chaves→texto. (Formato de record interno
              em Rust puro — porte WolvenKit, leitura E escrita/re-pack.)
    locedit   EDITA a localização (onscreens/subtitles) e RE-EMPACOTA em Rust puro:
              troca o texto de entradas por chave, recomputa offsets/tamanhos/crc32 e
              grava um CR2W válido pra dropar num .archive de mod (tradução ArchiveXL).
              Sem <out> = só a prova offline (no-op byte-idêntico + edição verificada).
    pack      Empacota UM recurso CR2W num .archive RDAR NÃO-COMPRIMIDO (sem Kraken
              encode). <game-path> = path REDengine (ex.: base\\localization\\en-us\\...)
              -> name_hash FNV-1a64 (bate com o do jogo). Auto-verifica pelo reader
              (crc64 do índice + sha1 + extração byte-exata). Fecha o mod de tradução.

OPÇÕES de extract:
    --all                  extrai todos os archives do diretório do jogo
    --datamap              também grava <dest>/<nome>/datamap.md
    --hashes <lista>       lista de paths (texto ou .kark) para resolver nomes
    --no-hashes            não usar a lista de hashes embutida
    --decompress-buffers   descomprime também os segmentos de buffer
    --skip-unresolved      não extrai recursos sem nome (padrão: unknown/<hash>.bin)

GLOBAIS:
    -h, --help       mostra esta ajuda
    -V, --version    mostra a versão
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("erro: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let Some(first) = args.first() else {
        print!("{USAGE}");
        return Ok(());
    };

    match first.as_str() {
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        "-V" | "--version" => {
            println!("archive-tool {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "list" => cmd_list(&args[1..]),
        "datamap" => cmd_datamap(&args[1..]),
        "extract" => cmd_extract(&args[1..]),
        "info" => cmd_info(&args[1..]),
        "cr2w" => cmd_cr2w(&args[1..]),
        "cr2wchunk" => cmd_cr2wchunk(&args[1..]),
        "cr2wall" => cmd_cr2wall(&args[1..]),
        "imports" => cmd_imports(&args[1..]),
        "locedit" => cmd_locedit(&args[1..]),
        "pack" => cmd_pack(&args[1..]),
        "packdir" => cmd_packdir(&args[1..]),
        "extract-one" => cmd_extract_one(&args[1..]),
        "c2dadd" => cmd_c2dadd(&args[1..]),
        "entbuf" => cmd_entbuf(&args[1..]),
        "lookuphash" => cmd_lookuphash(&args[1..]),
        "searchpath" => cmd_searchpath(&args[1..]),
        other => Err(format!(
            "comando desconhecido '{other}'. Use `archive-tool --help`."
        )),
    }
}

/// Extrai TODAS as entradas de localização de um resource onscreens e grava um TSV
/// `primaryKey<TAB>secondaryKey<TAB>femaleVariant<TAB>maleVariant` (pronto p/ tradução). Devolve
/// quantas gravou. Reusa o parser CR2W (índice → dict → names → chunk `entries` → array).
fn dump_localization(data: &[u8], idx: &crate::cr2w::Cr2wIndex, out: &str) -> Result<usize, String> {
    use std::io::Write;
    let dict = crate::cr2w::read_string_dict(data, &idx.tables[0])?;
    let names = crate::cr2w::read_names(data, &idx.tables[1], &dict)?;
    let exports = crate::cr2w::read_exports(data, &idx.tables[4], &names)?;
    for c in &exports {
        let (fields, _appendix) = crate::cr2w::read_chunk_fields(data, c, &names)?;
        for f in &fields {
            if f.name == "entries" && f.red_type.contains("localizationPersistence") {
                let (_total, entries) = crate::cr2w::extract_localization(&f.value, &names, 0)?;
                let mut w = std::io::BufWriter::new(
                    std::fs::File::create(out).map_err(|e| format!("não criou '{out}': {e}"))?,
                );
                writeln!(w, "primaryKey\tsecondaryKey\tfemaleVariant\tmaleVariant").ok();
                for e in &entries {
                    // escapa TAB/newline dos valores p/ não quebrar o TSV.
                    let esc = |s: &str| s.replace('\t', " ").replace('\n', " ");
                    writeln!(w, "{}\t{}\t{}\t{}", e.primary_key, esc(&e.secondary_key), esc(&e.female), esc(&e.male)).ok();
                }
                w.flush().ok();
                return Ok(entries.len());
            }
        }
    }
    Err("nenhum campo de localização onscreen encontrado neste resource".into())
}

/// `cr2w <resource-file> [<out.tsv>]` — lê o ÍNDICE CR2W (magic + FileHeader + 10 tabelas + dict +
/// names + chunks) e imprime um resumo. Com `<out.tsv>`, grava TODAS as entradas de localização
/// (se for um resource onscreens) num TSV pronto p/ traduzir.
/// Aplica um TSV de tradução (mesmo formato do dump: `primaryKey\tsecondaryKey\tfemale\tmale`) a um
/// CR2W de localização e grava o resultado. Só as linhas cujo texto DIFERE do original viram edição
/// (compara contra o dump ESCAPADO do original, então linhas intactas não re-encodam). Verifica os
/// crc32 da saída. É a ponta usável: dump → traduz o TSV → `locedit <in> <out.cr2w> <edits.tsv>` → `pack`.
fn apply_tsv_edits(data: &[u8], out: &str, tsv: &str) -> Result<(), String> {
    // 1) originais (primaryKey → female/male), na mesma forma escapada do dump.
    let idx = crate::cr2w::parse_cr2w_index(data)?;
    let dict = crate::cr2w::read_string_dict(data, &idx.tables[0])?;
    let names = crate::cr2w::read_names(data, &idx.tables[1], &dict)?;
    let exports = crate::cr2w::read_exports(data, &idx.tables[4], &names)?;
    let esc = |s: &str| s.replace('\t', " ").replace('\n', " ");
    let mut orig: std::collections::HashMap<u64, (String, String)> = std::collections::HashMap::new();
    for e in &exports {
        if let Ok((fields, _)) = crate::cr2w::read_chunk_fields(data, e, &names) {
            if let Some(f) = fields.iter().find(|f| f.name == "entries") {
                let (_t, ents) = crate::cr2w::extract_localization(&f.value, &names, 0)?;
                for en in &ents {
                    orig.insert(en.primary_key, (esc(&en.female), esc(&en.male)));
                }
                break;
            }
        }
    }
    if orig.is_empty() {
        return Err("o CR2W de entrada não tem entradas de localização".into());
    }

    // 2) lê o TSV; linha mudada (vs original escapado) → edição.
    let text = std::fs::read_to_string(tsv).map_err(|e| format!("não leu '{tsv}': {e}"))?;
    let mut edits = crate::cr2w::LocEdits::new();
    let (mut changed, mut skipped_key, mut unchanged) = (0usize, 0usize, 0usize);
    for (ln, line) in text.lines().enumerate() {
        if ln == 0 && line.starts_with("primaryKey") {
            continue; // cabeçalho
        }
        if line.trim().is_empty() {
            continue;
        }
        let col: Vec<&str> = line.splitn(4, '\t').collect();
        if col.len() < 4 {
            continue;
        }
        let key: u64 = match col[0].trim().parse() {
            Ok(k) => k,
            Err(_) => continue,
        };
        let (nf, nm) = (col[2], col[3]);
        match orig.get(&key) {
            None => skipped_key += 1, // chave não existe no original (ignora, não inventa)
            Some((of, om)) => {
                let f_new = if nf != of { Some(nf.to_string()) } else { None };
                let m_new = if nm != om { Some(nm.to_string()) } else { None };
                if f_new.is_some() || m_new.is_some() {
                    edits.insert(key, (f_new, m_new));
                    changed += 1;
                } else {
                    unchanged += 1;
                }
            }
        }
    }
    println!("TSV: {changed} linhas mudadas, {unchanged} intactas, {skipped_key} chaves inexistentes (ignoradas)");
    if changed == 0 {
        return Err("nenhuma mudança no TSV vs o original — nada a fazer".into());
    }

    // 3) aplica + verifica + grava.
    let edited = crate::cr2w::repack_localization_edit(data, &edits)?;
    let eidx = crate::cr2w::parse_cr2w_index(&edited)?;
    let mut crc_ok = crate::cr2w::header_crc32(&eidx.header, &eidx.tables) == eidx.header.crc32;
    for (i, t) in eidx.tables.iter().enumerate() {
        if t.item_count > 0 && crate::cr2w::crc32(crate::cr2w::table_bytes(&edited, t, i)) != t.crc32 {
            crc_ok = false;
        }
    }
    if !crc_ok {
        return Err("crc32 da saída inválido — abortado (não grava CR2W corrompido)".into());
    }
    std::fs::write(out, &edited).map_err(|e| format!("não gravou '{out}': {e}"))?;
    println!("gravou '{out}' ({} bytes, delta {:+}) · crc32 ✓ · agora: archive-tool pack '{out}' <game-path> <mod.archive>",
        edited.len(), edited.len() as i64 - data.len() as i64);
    Ok(())
}

/// Prova ponta-a-ponta da EDIÇÃO de localização + re-pack (100% offline). Passos:
///   1. no-op: edição vazia → o CR2W tem que sair byte-idêntico (re-pack não corrompe nada).
///   2. edição real: troca o texto de UMA entrada por um marcador, re-empacota, re-parseia a saída,
///      confirma que o texto mudou E que todos os crc32 (header+tabelas) validam.
/// Opcional: `locedit <file> <out.json>` também GRAVA o CR2W editado (para dropar num .archive de mod).
fn cmd_locedit(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("uso: archive-tool locedit <onscreens.json> [<out.cr2w> <edits.tsv>]")?;
    let data = std::fs::read(path).map_err(|e| format!("não leu '{path}': {e}"))?;

    // MODO REAL (3 args): aplica um TSV editado (primaryKey\tsecondaryKey\tfemale\tmale) e grava.
    if let (Some(out), Some(tsv)) = (args.get(1), args.get(2)) {
        return apply_tsv_edits(&data, out, tsv);
    }
    let idx = crate::cr2w::parse_cr2w_index(&data)?;
    let dict = crate::cr2w::read_string_dict(&data, &idx.tables[0])?;
    let names = crate::cr2w::read_names(&data, &idx.tables[1], &dict)?;
    let exports = crate::cr2w::read_exports(&data, &idx.tables[4], &names)?;

    // acha o campo `entries` e a 1ª entrada com texto, p/ ter uma chave real de teste.
    let mut sample_key = 0u64;
    let mut sample_old = String::new();
    for e in &exports {
        if let Ok((fields, _)) = crate::cr2w::read_chunk_fields(&data, e, &names) {
            if let Some(f) = fields.iter().find(|f| f.name == "entries") {
                if let Ok((total, ents)) = crate::cr2w::extract_localization(&f.value, &names, 8) {
                    println!("entries: {total} no total; amostra das primeiras:");
                    for en in ents.iter().take(4) {
                        println!("  key={:#018x} sec='{}' male='{}'", en.primary_key, en.secondary_key,
                            en.male.chars().take(40).collect::<String>());
                    }
                    if let Some(en) = ents.iter().find(|e| !e.male.is_empty() || !e.female.is_empty()) {
                        sample_key = en.primary_key;
                        sample_old = if !en.male.is_empty() { en.male.clone() } else { en.female.clone() };
                    }
                }
                break;
            }
        }
    }
    if sample_key == 0 {
        return Err("não achei entrada de localização com texto".into());
    }

    // PASSO 1 — no-op: edição vazia deve sair byte-idêntica.
    let empty = crate::cr2w::LocEdits::new();
    let noop = crate::cr2w::repack_localization_edit(&data, &empty)?;
    println!("\n[1] edição no-op: {}", if noop == data { "byte-idêntico ✓" } else { "DIFERE ✗" });

    // PASSO 2 — edição real: troca male E female da entrada-amostra por um marcador.
    let marker = format!("[BWMS-EDIT] {}", sample_old.chars().take(20).collect::<String>());
    let mut edits = crate::cr2w::LocEdits::new();
    edits.insert(sample_key, (Some(marker.clone()), Some(marker.clone())));
    let edited = crate::cr2w::repack_localization_edit(&data, &edits)?;
    println!("[2] edição real key={sample_key:#018x}: {} → '{}'", sample_old.chars().take(30).collect::<String>(), marker);
    println!("    tamanho: {}B → {}B (delta {:+})", data.len(), edited.len(), edited.len() as i64 - data.len() as i64);

    // re-parseia a SAÍDA e valida: crc32 de tudo + o texto novo presente.
    let eidx = crate::cr2w::parse_cr2w_index(&edited)?;
    let hc = crate::cr2w::header_crc32(&eidx.header, &eidx.tables);
    let mut crc_ok = hc == eidx.header.crc32;
    for (i, t) in eidx.tables.iter().enumerate() {
        if t.item_count > 0 && crate::cr2w::crc32(crate::cr2w::table_bytes(&edited, t, i)) != t.crc32 {
            crc_ok = false;
        }
    }
    println!("    crc32 da saída (header+tabelas): {}", if crc_ok { "todos válidos ✓" } else { "INVÁLIDO ✗" });

    let edict = crate::cr2w::read_string_dict(&edited, &eidx.tables[0])?;
    let enames = crate::cr2w::read_names(&edited, &eidx.tables[1], &edict)?;
    let eexports = crate::cr2w::read_exports(&edited, &eidx.tables[4], &enames)?;
    let mut found = false;
    for e in &eexports {
        if let Ok((fields, _)) = crate::cr2w::read_chunk_fields(&edited, e, &enames) {
            if let Some(f) = fields.iter().find(|f| f.name == "entries") {
                if let Ok((_, ents)) = crate::cr2w::extract_localization(&f.value, &enames, 0) {
                    if let Some(en) = ents.iter().find(|e| e.primary_key == sample_key) {
                        // o marcador entra no variant que EXISTIA (female nos onscreens; male é opcional).
                        found = en.female == marker || en.male == marker;
                        let shown = if en.female == marker { &en.female } else { &en.male };
                        println!("    re-parse da entrada editada: variant='{shown}' {}", if found { "✓" } else { "✗" });
                    }
                }
            }
        }
    }
    println!("\n>>> EDIÇÃO DE LOCALIZAÇÃO + RE-PACK {}", if noop == data && crc_ok && found { "PROVADO OFFLINE <<<" } else { "FALHOU <<<" });

    // PASSO 3 — ADIÇÃO: adiciona uma entrada nova com chave inédita e confirma que aparece no re-parse.
    let add_key: u64 = 0x7FFF_FFFF_0000_0001;
    let add = vec![crate::cr2w::LocAdd {
        primary_key: add_key,
        secondary_key: "BWMS-Added-Key".to_string(),
        female: "Entrada Nova BWMS".to_string(),
        male: String::new(),
    }];
    match crate::cr2w::repack_localization_add(&data, &add) {
        Ok(added) => {
            let aidx = crate::cr2w::parse_cr2w_index(&added).unwrap();
            let mut a_crc = crate::cr2w::header_crc32(&aidx.header, &aidx.tables) == aidx.header.crc32;
            for (i, t) in aidx.tables.iter().enumerate() {
                if t.item_count > 0 && crate::cr2w::crc32(crate::cr2w::table_bytes(&added, t, i)) != t.crc32 {
                    a_crc = false;
                }
            }
            let adict = crate::cr2w::read_string_dict(&added, &aidx.tables[0]).unwrap_or_default();
            let anames = crate::cr2w::read_names(&added, &aidx.tables[1], &adict).unwrap_or_default();
            let aexports = crate::cr2w::read_exports(&added, &aidx.tables[4], &anames).unwrap_or_default();
            let mut a_found = false;
            for e in &aexports {
                if let Ok((fields, _)) = crate::cr2w::read_chunk_fields(&added, e, &anames) {
                    if let Some(f) = fields.iter().find(|f| f.name == "entries") {
                        if let Ok((_, ents)) = crate::cr2w::extract_localization(&f.value, &anames, 0) {
                            if let Some(en) = ents.iter().find(|e| e.primary_key == add_key) {
                                a_found = en.female == "Entrada Nova BWMS";
                                println!("[3] adição key={add_key:#018x}: re-parse female='{}' {}", en.female, if a_found { "✓" } else { "✗" });
                            }
                        }
                    }
                }
            }
            println!(">>> ADIÇÃO DE ENTRADA {}", if a_crc && a_found { "PROVADA OFFLINE (crc32 ✓) <<<" } else { "FALHOU <<<" });
        }
        Err(e) => println!("[3] adição falhou: {e}"),
    }

    if let Some(out) = args.get(1) {
        std::fs::write(out, &edited).map_err(|e| format!("não gravou '{out}': {e}"))?;
        println!("gravou o CR2W editado em '{out}'");
    }
    Ok(())
}

/// Adiciona linha(s) a um C2dArray (factory/stat .csv) e grava. Cada `<linha>` = células separadas por
/// `,` (a forma comma-joined vira 1 célula). `c2dadd <csv> <out> <linha1> [<linha2> ...]`. Verifica o
/// round-trip da leitura + que a(s) linha(s) aparecem no re-parse.
/// `entbuf <file.ent> [<out.bin>]` — extrai o buffer `compiledData` (o pacote de componentes da
/// entity, table[5]) do .ent, descomprime (Kraken cru, sem KARK) e dumpa o pacote descomprimido +
/// as strings legíveis (nomes de componentes + classes). Usado pra descobrir por que um componente
/// (ex.: `tppCamera` do JB TPP mod) não instancia — comparar o pacote JB vs vanilla.
fn cmd_entbuf(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or("uso: archive-tool entbuf <file.ent> [<out.bin>]")?;
    let data = std::fs::read(path).map_err(|e| format!("não leu '{path}': {e}"))?;
    let idx = crate::cr2w::parse_cr2w_index(&data)?;
    let t = &idx.tables[5];
    println!("buffers (table[5]): {} entradas @ {}", t.item_count, t.offset);
    if t.item_count == 0 {
        return Err("sem buffers (table[5] vazia) — este .ent não tem compiledData?".into());
    }
    // cada entrada de buffer = 24 bytes: flags(u32) index(u32) offset(u32) diskSize(u32) memSize(u32) crc32(u32)
    for i in 0..t.item_count as usize {
        let e = t.offset as usize + i * 24;
        if e + 24 > data.len() {
            return Err(format!("buffer[{i}] fora do arquivo"));
        }
        let rd = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
        let (flags, index, offset, disksz, memsz, crc) =
            (rd(e), rd(e + 4), rd(e + 8), rd(e + 12), rd(e + 16), rd(e + 20));
        println!(
            "buf[{i}] flags={flags:#x} index={index} offset={offset} diskSize={disksz} memSize={memsz} crc={crc:#x}"
        );
        // Peek dos 4 primeiros bytes p/ decidir compressão (KARK vs RAW). ACHADO 2026-07-16: pra
        // buffer RAW (não-KARK), o tamanho em disco é o memSize (não diskSize — que só vale como
        // tamanho-se-comprimido); `offset+memSize` fecha exatamente no fim do arquivo p/ o .ent do player.
        let first4 = data
            .get(offset as usize..offset as usize + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()));
        let is_kark = first4 == Some(crate::archive::KARK_MAGIC);
        let raw_size = if is_kark { disksz as usize } else { memsz as usize };
        let end = (offset as usize + raw_size).min(data.len());
        let blob = &data[offset as usize..end];
        let decompressed: Vec<u8> = if is_kark {
            match crate::kraken::decompress(&blob[8..], memsz as usize) {
                Ok(d) => {
                    println!("  descomprimido Kraken (KARK): {} -> {} bytes ✓", disksz, memsz);
                    d
                }
                Err(err) => return Err(format!("Kraken falhou no buffer[{i}]: {err}")),
            }
        } else {
            println!("  RAW (redPackage, não comprimido): {} bytes (memSize runtime={})", disksz, memsz);
            blob.to_vec()
        };
        // dump das strings ASCII legíveis (>=4 chars) — nomes de componente + classes + refs
        println!("  === strings do pacote descomprimido (nomes/classes/refs) ===");
        let mut cur = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for &b in &decompressed {
            if (0x20..0x7f).contains(&b) {
                cur.push(b);
            } else {
                if cur.len() >= 4 {
                    if let Ok(s) = std::str::from_utf8(&cur) {
                        seen.insert(s.to_string());
                    }
                }
                cur.clear();
            }
        }
        for s in &seen {
            println!("    {s}");
        }
        if let Some(out) = args.get(1) {
            let of = if t.item_count > 1 {
                format!("{out}.buf{i}")
            } else {
                out.clone()
            };
            std::fs::write(&of, &decompressed).map_err(|e| format!("não gravou '{of}': {e}"))?;
            println!("  pacote descomprimido salvo -> {of}");
        }
    }
    Ok(())
}

fn cmd_lookuphash(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("uso: archive-tool lookuphash <hex1> [<hex2>...]  (ex.: 0x80e26b2a67ecbdcd)".into());
    }
    let dict = load_base_dict(resolve_hashes(None, false).as_deref())?;
    for a in args {
        let h = u64::from_str_radix(a.trim_start_matches("0x"), 16)
            .map_err(|e| format!("hash inválido '{a}': {e}"))?;
        match dict.resolve(h) {
            Some(p) => println!("{a} -> {p}"),
            None => println!("{a} -> (sem match no dicionário)"),
        }
    }
    Ok(())
}

fn cmd_searchpath(args: &[String]) -> Result<(), String> {
    let needle = args.first().ok_or("uso: archive-tool searchpath <substring> [limite]")?.to_ascii_lowercase();
    let limit: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(40);
    let path = resolve_hashes(None, false).ok_or("usedhashes.kark não encontrado")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("não leu {}: {e}", path.display()))?;
    let text = hashes::decode_maybe_kark(&bytes).map_err(|e| e.to_string())?;
    let mut n = 0;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l.to_ascii_lowercase().contains(&needle) {
            println!("{l}");
            n += 1;
            if n >= limit {
                break;
            }
        }
    }
    eprintln!("{n} match(es) (limite {limit})");
    Ok(())
}

fn cmd_c2dadd(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("uso: archive-tool c2dadd <csv> <out> <linha1> [<linha2>...]")?;
    let out = args.get(1).ok_or("falta o <out>")?;
    let rows_in: Vec<&String> = args.iter().skip(2).collect();
    if rows_in.is_empty() {
        return Err("dê pelo menos uma <linha> (células separadas por vírgula)".into());
    }
    let data = std::fs::read(path).map_err(|e| format!("não leu '{path}': {e}"))?;

    // descobre a largura das linhas existentes p/ montar as novas na mesma forma.
    let idx = crate::cr2w::parse_cr2w_index(&data)?;
    let dict = crate::cr2w::read_string_dict(&data, &idx.tables[0])?;
    let names = crate::cr2w::read_names(&data, &idx.tables[1], &dict)?;
    let exports = crate::cr2w::read_exports(&data, &idx.tables[4], &names)?;
    let mut width = 1usize;
    for e in &exports {
        if e.class_name == "C2dArray" {
            if let Ok((fields, _)) = crate::cr2w::read_chunk_fields(&data, e, &names) {
                let hv = fields.iter().find(|f| f.name == "headers").map(|f| f.value.clone());
                let dv = fields.iter().find(|f| f.name == "data").map(|f| f.value.clone());
                if let (Some(hv), Some(dv)) = (hv, dv) {
                    if let Ok((_, rows)) = crate::cr2w::read_c2d_array(&hv, &dv) {
                        width = rows.first().map(|r| r.len()).unwrap_or(1);
                    }
                }
            }
            break;
        }
    }
    // monta as linhas novas: se o arquivo usa 1 célula (comma-joined), a linha inteira é 1 célula;
    // senão, quebra por vírgula em `width` células.
    let new_rows: Vec<Vec<String>> = rows_in.iter().map(|l| {
        if width <= 1 { vec![l.to_string()] } else { l.split(',').map(|s| s.to_string()).collect() }
    }).collect();
    println!("forma do C2dArray: {width} célula(s)/linha; adicionando {} linha(s)", new_rows.len());

    let edited = crate::cr2w::repack_c2d_add(&data, &new_rows)?;
    // valida crc32 + as linhas presentes.
    let eidx = crate::cr2w::parse_cr2w_index(&edited)?;
    let mut crc_ok = crate::cr2w::header_crc32(&eidx.header, &eidx.tables) == eidx.header.crc32;
    for (i, t) in eidx.tables.iter().enumerate() {
        if t.item_count > 0 && crate::cr2w::crc32(crate::cr2w::table_bytes(&edited, t, i)) != t.crc32 {
            crc_ok = false;
        }
    }
    let edict = crate::cr2w::read_string_dict(&edited, &eidx.tables[0])?;
    let enames = crate::cr2w::read_names(&edited, &eidx.tables[1], &edict)?;
    let eexports = crate::cr2w::read_exports(&edited, &eidx.tables[4], &enames)?;
    let mut nrows = 0;
    for e in &eexports {
        if e.class_name == "C2dArray" {
            if let Ok((fields, _)) = crate::cr2w::read_chunk_fields(&edited, e, &enames) {
                let hv = fields.iter().find(|f| f.name == "headers").map(|f| f.value.clone());
                let dv = fields.iter().find(|f| f.name == "data").map(|f| f.value.clone());
                if let (Some(hv), Some(dv)) = (hv, dv) {
                    if let Ok((_, rows)) = crate::cr2w::read_c2d_array(&hv, &dv) { nrows = rows.len(); }
                }
            }
        }
    }
    std::fs::write(out, &edited).map_err(|e| format!("não gravou '{out}': {e}"))?;
    println!("gravou '{out}' ({} linhas no C2dArray agora) · crc32 {} · delta {:+}",
        nrows, if crc_ok { "✓" } else { "✗ INVÁLIDO" }, edited.len() as i64 - data.len() as i64);
    println!(">>> C2D ADD {}", if crc_ok { "OK <<<" } else { "FALHOU <<<" });
    Ok(())
}

/// Extrai UM recurso de um archive por game-path, sem materializar o resto (útil p/ archives gigantes).
/// `extract-one <archive> <game-path> <out>`. Descomprime (Kraken se preciso).
fn cmd_extract_one(args: &[String]) -> Result<(), String> {
    let arg = args.first().ok_or("uso: archive-tool extract-one <archive> <game-path> <out>")?;
    let game_path = args.get(1).ok_or("falta o <game-path>")?;
    let out = args.get(2).ok_or("falta o <out>")?;
    let apath = resolve_archive_arg(arg)?;
    let ar = crate::archive::Archive::open(&apath).map_err(|e| format!("{} — {e}", apath.display()))?;
    let hash = crate::hashes::fnv1a64(crate::hashes::canonical(game_path).as_bytes());
    let bytes = crate::extract::extract_one(&ar, hash)?;
    std::fs::write(out, &bytes).map_err(|e| format!("não gravou '{out}': {e}"))?;
    let is_cr2w = bytes.len() >= 4 && &bytes[..4] == b"CR2W";
    println!("extraído {game_path} ({hash:#018x}) → {out} ({} bytes{})", bytes.len(),
        if is_cr2w { ", CR2W" } else { "" });
    Ok(())
}

/// Coleta recursivamente todos os arquivos regulares sob `dir`, com o caminho relativo (p/ derivar
/// o game-path). Zero-dep (sem walkdir).
fn walk_files(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    for ent in std::fs::read_dir(dir).map_err(|e| format!("não leu dir '{}': {e}", dir.display()))? {
        let p = ent.map_err(|e| e.to_string())?.path();
        if p.is_dir() {
            walk_files(&p, base, out)?;
        } else if p.is_file() {
            out.push(p);
        }
    }
    Ok(())
}

/// Empacota uma PASTA inteira de recursos CR2W num `.archive` (um mod de tradução com vários arquivos,
/// ex.: dezenas de subtitles). O game-path de cada arquivo = caminho relativo à pasta (`/`→`\`), e o
/// name_hash = FNV-1a64 disso (bate com o do jogo). `packdir <pasta> <out.archive>`. A pasta deve
/// espelhar a árvore REDengine (ex.: `<pasta>/base/localization/en-us/...`). Auto-verifica pelo reader.
fn cmd_packdir(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("uso: archive-tool packdir <pasta> <out.archive>")?;
    let out_path = args.get(1).ok_or("falta o <out.archive>")?;
    let base = std::path::Path::new(dir);
    if !base.is_dir() {
        return Err(format!("'{dir}' não é uma pasta"));
    }
    let mut files = Vec::new();
    walk_files(base, base, &mut files)?;
    files.sort();
    if files.is_empty() {
        return Err("pasta vazia".into());
    }

    let mut resources: Vec<(u64, Vec<u8>)> = Vec::with_capacity(files.len());
    for f in &files {
        let rel = f.strip_prefix(base).map_err(|_| "strip_prefix".to_string())?;
        let game_path = rel.to_string_lossy().replace('/', "\\");
        let bytes = std::fs::read(f).map_err(|e| format!("não leu '{}': {e}", f.display()))?;
        // só CR2W (pula lixo tipo .DS_Store)
        if bytes.len() < 4 || &bytes[..4] != b"CR2W" {
            println!("  pulado (não-CR2W): {game_path}");
            continue;
        }
        let hash = crate::hashes::fnv1a64(crate::hashes::canonical(&game_path).as_bytes());
        println!("  + {game_path}  ({hash:#018x}, {} B)", bytes.len());
        resources.push((hash, bytes));
    }
    if resources.is_empty() {
        return Err("nenhum CR2W na pasta".into());
    }

    let archive = crate::writer::pack_uncompressed(&resources)?;
    std::fs::write(out_path, &archive).map_err(|e| format!("não gravou '{out_path}': {e}"))?;
    println!("\ngravou {out_path} — {} recursos, {} bytes", resources.len(), archive.len());

    // auto-verificação pelo reader: crc64 + todo recurso presente + extração byte-exata.
    let ar = crate::archive::Archive::open(std::path::Path::new(out_path)).map_err(|e| format!("reabrir: {e}"))?;
    let raw = std::fs::read(out_path).unwrap();
    let mut all_ok = ar.crc_ok && ar.entries.len() == resources.len();
    for (hash, bytes) in &resources {
        match ar.entries.iter().find(|e| e.name_hash == *hash) {
            Some(entry) => {
                let seg = ar.segments_of(entry).first().copied();
                let good = seg.is_some_and(|s| &raw[s.offset as usize..s.offset as usize + s.zsize as usize] == &bytes[..]);
                if !good { all_ok = false; }
            }
            None => all_ok = false,
        }
    }
    println!("auto-verificação: índice crc64 {} · {} recursos · extração byte-exata {}",
        if ar.crc_ok { "✓" } else { "✗" }, ar.entries.len(), if all_ok { "✓" } else { "✗" });
    println!(">>> PACKDIR {}", if all_ok { "VÁLIDO <<<" } else { "FALHOU <<<" });
    Ok(())
}

/// Empacota UM recurso CR2W num `.archive` RDAR não-comprimido (mod de tradução). O name_hash é o
/// FNV-1a64 do path REDengine (minúsculas, `\`). Auto-verifica: re-abre pelo reader (crc64 do índice
/// + extrai o segmento) e confere byte-exato com a entrada. `pack <cr2w> <game-path> <out.archive>`.
fn cmd_pack(args: &[String]) -> Result<(), String> {
    let cr2w_path = args.first().ok_or("uso: archive-tool pack <cr2w-file> <game-path> <out.archive>")?;
    let game_path = args.get(1).ok_or("falta o <game-path> (ex.: base\\localization\\en-us\\onscreens\\onscreens_final.json)")?;
    let out_path = args.get(2).ok_or("falta o <out.archive>")?;
    let cr2w = std::fs::read(cr2w_path).map_err(|e| format!("não leu '{cr2w_path}': {e}"))?;

    let hash = crate::hashes::fnv1a64(crate::hashes::canonical(game_path).as_bytes());
    println!("recurso: {game_path}\n  name_hash (FNV-1a64) = {hash:#018x}  ·  CR2W {} bytes", cr2w.len());

    let archive = crate::writer::pack_uncompressed(&[(hash, cr2w.clone())])?;
    std::fs::write(out_path, &archive).map_err(|e| format!("não gravou '{out_path}': {e}"))?;
    println!("gravou {} ({} bytes)", out_path, archive.len());

    // AUTO-VERIFICAÇÃO: re-abre pelo reader (porte fiel do WolvenKit) e confere tudo.
    let ar = crate::archive::Archive::open(std::path::Path::new(out_path))
        .map_err(|e| format!("reabrir falhou: {e}"))?;
    let entry = ar.entries.iter().find(|e| e.name_hash == hash)
        .ok_or("o recurso não apareceu no índice re-lido")?;
    let segs = ar.segments_of(entry);
    let seg = segs.first().ok_or("recurso sem segmento")?;
    // segmento não-comprimido → os bytes em [offset..offset+zsize) são o CR2W cru.
    let raw = std::fs::read(out_path).unwrap();
    let extracted = &raw[seg.offset as usize..seg.offset as usize + seg.zsize as usize];
    let ok_bytes = extracted == &cr2w[..];
    let ok_sha1 = entry.sha1 == crate::archive::sha1(&cr2w);

    println!("\n── auto-verificação (reader independente) ──");
    println!("  índice crc64: {}", if ar.crc_ok { "✓" } else { "✗" });
    println!("  recurso no índice: {} · 1 segmento · zsize==size: {}", ar.entries.len() == 1, seg.zsize == seg.size);
    println!("  sha1 do FileEntry: {}", if ok_sha1 { "✓" } else { "✗" });
    println!("  extração byte-exata do CR2W: {}", if ok_bytes { "✓" } else { "✗" });
    // e o CR2W extraído ainda passa nos self-checks CR2W (crc32 interno)?
    let cidx = crate::cr2w::parse_cr2w_index(extracted)?;
    let cr2w_hdr_ok = crate::cr2w::header_crc32(&cidx.header, &cidx.tables) == cidx.header.crc32;
    println!("  CR2W extraído íntegro (header crc32): {}", if cr2w_hdr_ok { "✓" } else { "✗" });
    println!("\n>>> PACK .archive {}", if ar.crc_ok && ok_bytes && ok_sha1 && cr2w_hdr_ok { "VÁLIDO (round-trip pelo reader) <<<" } else { "FALHOU <<<" });
    Ok(())
}

/// `cr2wall <resource-file> [--type <ClassName>]`: dump COMPLETO de todos os chunks/campos (sem
/// `.take()` de amostra) — ferramenta de dev pra RE de appearance/entity (`axl-cr2w-appearance-
/// schema`, 2026-07-15). `--type` filtra só chunks dessa classe (ex.: appearanceAppearanceDefinition).
fn cmd_imports(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("uso: archive-tool imports <resource-file>")?;
    let data = std::fs::read(path).map_err(|e| format!("não leu '{path}': {e}"))?;
    let idx = crate::cr2w::parse_cr2w_index(&data)?;
    let dict = crate::cr2w::read_string_dict(&data, &idx.tables[0])?;
    let nm = crate::cr2w::read_names(&data, &idx.tables[1], &dict)?;
    let imports = crate::cr2w::read_imports(&data, &idx.tables[2], &dict, &nm)?;
    println!("{} imports", imports.len());
    for (i, imp) in imports.iter().enumerate() {
        println!("[{}] '{}' class={} flags={:#06x}", i + 1, imp.depot_path, imp.class_name, imp.flags);
    }
    Ok(())
}

fn cmd_cr2wall(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("uso: archive-tool cr2wall <resource-file> [--type <ClassName>] [--names]")?;
    let type_filter = args.iter().position(|a| a == "--type").and_then(|i| args.get(i + 1)).map(|s| s.as_str());
    let show_names = args.iter().any(|a| a == "--names");
    let data = std::fs::read(path).map_err(|e| format!("não leu '{path}': {e}"))?;
    let idx = crate::cr2w::parse_cr2w_index(&data)?;
    let dict = crate::cr2w::read_string_dict(&data, &idx.tables[0])?;
    let nm = crate::cr2w::read_names(&data, &idx.tables[1], &dict)?;
    let ex = crate::cr2w::read_exports(&data, &idx.tables[4], &nm)?;
    if show_names {
        for (i, (_, s)) in nm.iter().enumerate() {
            println!("names[{i}] = {s}");
        }
    }
    println!("{} chunks totais", ex.len());
    for (i, c) in ex.iter().enumerate() {
        if let Some(t) = type_filter {
            if c.class_name != t {
                continue;
            }
        }
        println!("[{i}] '{}' data@{} size={}", c.class_name, c.data_offset, c.data_size);
        match crate::cr2w::read_chunk_fields(&data, c, &nm) {
            Ok((fs, appendix)) => {
                for f in &fs {
                    println!("    .{}: {} = {}", f.name, f.red_type, crate::cr2w::decode_field_value(f, &nm));
                }
                if !appendix.is_empty() {
                    println!("    [appendix {}B]", appendix.len());
                }
            }
            Err(e) => println!("    ERRO lendo campos: {e}"),
        }
    }
    Ok(())
}

/// Dumpa UM chunk específico (por índice 0-based na tabela de exports) por completo — TODOS os
/// campos, sem o `.take(4)`/`.take(6)` do `cmd_cr2w` (feito pra inspecionar um chunk fundo demais
/// pro dump padrão alcançar, ex.: `animAnimVariableContainer` de um `.animgraph` com milhares de
/// chunks). Uso: `archive-tool cr2wchunk <file> <index>`.
fn cmd_cr2wchunk(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("uso: archive-tool cr2wchunk <resource-file> <chunk-index>")?;
    let idx_n: usize = args
        .get(1)
        .ok_or("uso: archive-tool cr2wchunk <resource-file> <chunk-index>")?
        .parse()
        .map_err(|_| "chunk-index precisa ser um número".to_string())?;
    let data = std::fs::read(path).map_err(|e| format!("não leu '{path}': {e}"))?;
    let idx = crate::cr2w::parse_cr2w_index(&data)?;
    let dict = crate::cr2w::read_string_dict(&data, &idx.tables[0])?;
    let nm = crate::cr2w::read_names(&data, &idx.tables[1], &dict)?;
    let ex = crate::cr2w::read_exports(&data, &idx.tables[4], &nm)?;
    let c = ex.get(idx_n).ok_or_else(|| format!("chunk #{idx_n} não existe (só há {} chunks)", ex.len()))?;
    println!("chunk #{idx_n}: '{}' data@{} size={}", c.class_name, c.data_offset, c.data_size);
    let (fs, _appendix) = crate::cr2w::read_chunk_fields(&data, c, &nm)?;
    for f in &fs {
        println!("    .{}: {} = {}", f.name, f.red_type, crate::cr2w::decode_field_value(f, &nm));
    }
    Ok(())
}

fn cmd_cr2w(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or("uso: archive-tool cr2w <resource-file> [<out.tsv>]")?;
    let data = std::fs::read(path).map_err(|e| format!("não leu '{path}': {e}"))?;
    let idx = crate::cr2w::parse_cr2w_index(&data)?;
    if let Some(out) = args.get(1) {
        let n = dump_localization(&data, &idx, out)?;
        println!("gravou {n} entradas de localização em '{out}'");
        return Ok(());
    }
    let h = &idx.header;
    println!("CR2W  version={} flags={:#x} buildVersion={} numChunks={}", h.version, h.flags, h.build_version, h.num_chunks);
    println!("      objectsEnd={} buffersEnd={} (arquivo={}B)", h.objects_end, h.buffers_end, data.len());
    // self-check do writer do índice: re-serializa o envelope (160B) e compara.
    let idx_bytes = crate::cr2w::write_cr2w_index(&idx);
    println!("  índice round-trip: {}", if data.get(..idx_bytes.len()) == Some(&idx_bytes[..]) { "byte-exato ✓" } else { "DIFERE ✗" });
    // self-check do crc32 do HEADER: recomputa exatamente como CalculateHeaderCRC32 (0xDEADBEEF no
    // lugar do campo). Se bater, meu crc32 + a montagem do header estão bit-exatos com o WolvenKit.
    {
        let hc = crate::cr2w::header_crc32(h, &idx.tables);
        println!("  header crc32={:#010x} (calc {hc:#010x} {})", h.crc32, if hc == h.crc32 { "✓" } else { "≠" });
    }
    for (i, t) in idx.tables.iter().enumerate() {
        if t.item_count > 0 {
            // self-check do crc32: por-tabela = CRC32 dos bytes crus da tabela (WolvenKit CR2WWriter.File.cs).
            let computed = crate::cr2w::crc32(crate::cr2w::table_bytes(&data, t, i));
            let ok = if computed == t.crc32 { "✓" } else { "≠" };
            println!("  tabela[{i}]  offset={} itens={} crc32={:#010x} (calc {computed:#010x} {ok})", t.offset, t.item_count, t.crc32);
        }
    }
    // self-check do RE-PACK: re-empacota cada chunk com os PRÓPRIOS bytes originais (delta 0). A saída
    // TEM que ser byte-idêntica ao arquivo inteiro — prova offline da recomputação de offset/crc do
    // repack_replace_chunk (o núcleo da edição de localização), sem ligar o jogo.
    if idx.tables[4].item_count > 0 {
        if let Ok(nm) = crate::cr2w::read_names(&data, &idx.tables[1], &crate::cr2w::read_string_dict(&data, &idx.tables[0]).unwrap_or_default()) {
            if let Ok(ex) = crate::cr2w::read_exports(&data, &idx.tables[4], &nm) {
                let mut all_ok = true;
                for (ci, e) in ex.iter().enumerate() {
                    let (o, s) = (e.data_offset as usize, e.data_size as usize);
                    if o + s > data.len() { continue; }
                    let orig = &data[o..o + s];
                    match crate::cr2w::repack_replace_chunk(&data, ci, orig) {
                        Ok(rebuilt) => { if rebuilt != data { all_ok = false; } }
                        Err(_) => all_ok = false,
                    }
                }
                println!("  re-pack round-trip ({} chunks): {}", ex.len(), if all_ok { "byte-exato ✓" } else { "DIFERE ✗" });
            }
        }
    }
    // string dict (tabela 0): os type/field names que o resto do arquivo referencia.
    if idx.tables[0].item_count > 0 {
        match crate::cr2w::read_string_dict(&data, &idx.tables[0]) {
            Ok(dict) => {
                let mut names: Vec<&String> = dict.values().filter(|s| !s.is_empty()).collect();
                names.sort();
                println!("  string dict: {} strings (ex.: {})", dict.len(),
                    names.iter().take(6).map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
                // tabela 1 = names (resolvidos via dict).
                if idx.tables[1].item_count > 0 {
                    match crate::cr2w::read_names(&data, &idx.tables[1], &dict) {
                        Ok(nm) => {
                            let list: Vec<&str> = nm.iter().map(|(_, s)| s.as_str()).take(8).collect();
                            println!("  names: {} (ex.: {})", nm.len(), list.join(", "));
                            // tabela 4 = chunks/exports (o tipo resolvido via names + payload).
                            if idx.tables[4].item_count > 0 {
                                match crate::cr2w::read_exports(&data, &idx.tables[4], &nm) {
                                    Ok(ex) => {
                                        println!("  chunks: {}", ex.len());
                                        for c in ex.iter().take(4) {
                                            println!("    '{}' data@{} size={}", c.class_name, c.data_offset, c.data_size);
                                            // deserializa os campos do chunk (estrutura, valor cru).
                                            match crate::cr2w::read_chunk_fields(&data, c, &nm) {
                                                Ok((fs, appendix)) => {
                                                    for f in fs.iter().take(6) {
                                                        println!("        .{}: {} = {}", f.name, f.red_type, crate::cr2w::decode_field_value(f, &nm));
                                                        // se for o array de localização, extrai as primeiras entradas.
                                                        if f.name == "entries" && f.red_type.contains("localizationPersistence") {
                                                            match crate::cr2w::extract_localization(&f.value, &nm, 3) {
                                                                Ok((total, entries)) => {
                                                                    if entries.is_empty() {
                                                                        println!("          -> {total} elementos, mas 0 com texto direto (é um ÍNDICE, não texto)");
                                                                    } else {
                                                                        println!("          -> {} entradas COM texto (de {total} elementos); primeiras:", entries.len());
                                                                        for e in &entries {
                                                                            let key = if e.secondary_key.is_empty() { e.primary_key.to_string() } else { e.secondary_key.clone() };
                                                                            println!("             '{key}' = \"{}\"", e.female);
                                                                        }
                                                                    }
                                                                }
                                                                Err(err) => println!("          -> loc: {err}"),
                                                            }
                                                        }
                                                    }
                                                    // C2dArray (.csv do jogo: factories/stats): dump da tabela.
                                                    if c.class_name == "C2dArray" {
                                                        let hv = fs.iter().find(|f| f.name == "headers").map(|f| f.value.as_slice());
                                                        let dv = fs.iter().find(|f| f.name == "data").map(|f| f.value.as_slice());
                                                        // factory VAZIO = só headers, sem campo `data` (test/quest). O chunk
                                                        // round-trip byte-exato mesmo assim; só não há linhas p/ dumpar.
                                                        if hv.is_some() && dv.is_none() {
                                                            println!("          C2dArray: factory vazio (só headers, 0 linhas — sem campo data)");
                                                        }
                                                        if let (Some(hv), Some(dv)) = (hv, dv) {
                                                            match crate::cr2w::read_c2d_array(hv, dv) {
                                                                Ok((cols, rows)) => {
                                                                    println!("          C2dArray: {} header(s), {} linhas; header=[{}]",
                                                                        cols.len(), rows.len(), cols.join(" ¦ "));
                                                                    for r in rows.iter().take(3) {
                                                                        println!("             {}", r.join(" ¦ "));
                                                                    }
                                                                    // round-trip: re-encode == valores originais? (prova a leitura)
                                                                    let (rhv, rdv) = crate::cr2w::write_c2d_array(&cols, &rows);
                                                                    println!("          C2dArray round-trip: {}", if rhv == hv && rdv == dv { "byte-exato ✓" } else { "DIFERE ✗" });
                                                                }
                                                                Err(e) => println!("          C2dArray: {e}"),
                                                            }
                                                        }
                                                    }
                                                    // SELF-CHECK do writer: re-serializa os campos e compara com o
                                                    // chunk original (byte-exato) — prova o write_chunk_fields no record real.
                                                    let idx_of = |name: &str| nm.iter().position(|(_, s)| s == name).map(|i| i as u16);
                                                    let orig = &data[c.data_offset as usize..(c.data_offset + c.data_size) as usize];
                                                    match crate::cr2w::write_chunk_fields(&fs, &appendix, &idx_of, true) {
                                                        Ok(w) => println!("        round-trip do writer: {}", if w == orig { "byte-exato ✓" } else { "DIFERE ✗" }),
                                                        Err(e) => println!("        round-trip: {e}"),
                                                    }
                                                }
                                                Err(e) => println!("        (campos: {e})"),
                                            }
                                        }
                                    }
                                    Err(e) => println!("  chunks: erro — {e}"),
                                }
                            }
                        }
                        Err(e) => println!("  names: erro — {e}"),
                    }
                }
            }
            Err(e) => println!("  string dict: erro — {e}"),
        }
    }
    let issues = idx.structural_issues(data.len());
    if issues.is_empty() {
        println!("  consistência: OK");
    } else {
        for p in issues {
            println!("  ⚠ {p}");
        }
    }
    Ok(())
}

// ---- resolução de archives, dicionário e helpers ----

/// Resolve um argumento de archive: caminho existente, ou nome procurado no
/// diretório do jogo (com ou sem a extensão `.archive`).
fn resolve_archive_arg(arg: &str) -> Result<PathBuf, String> {
    let direct = PathBuf::from(arg);
    if direct.is_file() {
        return Ok(direct);
    }
    let content = defaults::content_dir();
    for candidate in [content.join(arg), content.join(format!("{arg}.archive"))] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "archive '{arg}' não encontrado (nem como caminho, nem em {}). \
         Ajuste CP77_CONTENT/CP77_DIR ou passe um caminho.",
        content.display()
    ))
}

/// Lista os `.archive` de um diretório, ordenados por nome.
fn list_archives(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let read = std::fs::read_dir(dir)
        .map_err(|e| format!("não consegui ler {}: {e}", dir.display()))?;
    let mut archives: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("archive"))
        })
        .collect();
    archives.sort();
    Ok(archives)
}

/// Decide qual lista de hashes usar: explícita > embutida (se existir) > nenhuma.
fn resolve_hashes(explicit: Option<PathBuf>, no_hashes: bool) -> Option<PathBuf> {
    if no_hashes {
        return None;
    }
    if explicit.is_some() {
        return explicit;
    }
    let default = defaults::hashes_path();
    default.is_file().then_some(default)
}

/// Dicionário base, carregado uma vez (a lista de hashes vale para todos os
/// archives). Os paths embutidos de cada archive entram depois, via [`add_archive_paths`].
fn load_base_dict(hashes: Option<&Path>) -> Result<PathDictionary, String> {
    let mut dict = PathDictionary::new();
    if let Some(list) = hashes {
        let n = dict
            .load_list(list)
            .map_err(|e| format!("não consegui ler a lista de hashes {}: {e}", list.display()))?;
        eprintln!("dicionário: {n} paths de {}", list.display());
    }
    Ok(dict)
}

/// Adiciona ao dicionário os paths embutidos no LxrsFooter do archive (corretos
/// globalmente: são mapeamentos hash→path, válidos para qualquer archive).
fn add_archive_paths(dict: &mut PathDictionary, ar: &Archive) {
    for p in &ar.custom_paths {
        dict.insert_path(p);
    }
    if !ar.custom_paths.is_empty() {
        eprintln!("dicionário: +{} paths embutidos do LxrsFooter", ar.custom_paths.len());
    }
    if ar.custom_paths_need_kraken {
        eprintln!("aviso: LxrsFooter comprimido; nomes embutidos exigem Kraken para ler.");
    }
}

fn write_datamap_file(
    ar: &Archive,
    dict: &PathDictionary,
    path: &Path,
) -> Result<datamap::Stats, String> {
    let file = std::fs::File::create(path)
        .map_err(|e| format!("não consegui criar {}: {e}", path.display()))?;
    let mut w = BufWriter::new(file);
    let stats = datamap::write_datamap(ar, dict, &mut w)
        .map_err(|e| format!("falha ao escrever datamap: {e}"))?;
    w.flush().map_err(|e| e.to_string())?;
    eprintln!("datamap escrito em {}", path.display());
    Ok(stats)
}

// ---- comandos ----

fn cmd_list(args: &[String]) -> Result<(), String> {
    let dir = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .unwrap_or_else(defaults::content_dir);
    let archives = list_archives(&dir)?;
    if archives.is_empty() {
        return Err(format!("nenhum .archive em {}", dir.display()));
    }
    println!("{} archives em {}:", archives.len(), dir.display());
    let mut total = 0u64;
    for p in &archives {
        let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        total += size;
        println!(
            "  {:>13}  {}",
            human(size),
            p.file_name().unwrap_or_default().to_string_lossy()
        );
    }
    println!("total: {}", human(total));
    Ok(())
}

fn cmd_info(args: &[String]) -> Result<(), String> {
    let arg = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or("info exige <nome|caminho>")?;
    let path = resolve_archive_arg(arg)?;
    let ar = Archive::open(&path).map_err(|e| format!("{} — {e}", path.display()))?;
    let disk: u64 = ar.segments.iter().map(|s| u64::from(s.zsize)).sum();
    let raw: u64 = ar.segments.iter().map(|s| u64::from(s.size)).sum();
    let comp = ar.segments.iter().filter(|s| s.size_differs()).count();
    println!(
        "{}: RDAR v{} · {} recursos · {} segmentos ({comp} comprimidos) · {} deps · disco {} · descomprimido {} · índice crc64 {}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        ar.header.version,
        ar.entries.len(),
        ar.segments.len(),
        ar.dependencies.len(),
        human(disk),
        human(raw),
        if ar.crc_ok { "✓" } else { "✗" },
    );
    Ok(())
}

fn cmd_datamap(args: &[String]) -> Result<(), String> {
    let mut archive: Option<String> = None;
    let mut out: Option<String> = None;
    let mut hashes: Option<PathBuf> = None;
    let mut no_hashes = false;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-o" | "--output" => out = Some(it.next().ok_or("-o exige um caminho")?.clone()),
            "--hashes" => hashes = Some(PathBuf::from(it.next().ok_or("--hashes exige um caminho")?)),
            "--no-hashes" => no_hashes = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if archive.is_none() && !other.starts_with('-') => archive = Some(other.to_string()),
            other => return Err(format!("argumento inesperado em datamap: '{other}'")),
        }
    }

    let archive = resolve_archive_arg(&archive.ok_or("datamap exige <nome|caminho>")?)?;
    let mut dict = load_base_dict(resolve_hashes(hashes, no_hashes).as_deref())?;
    let ar = Archive::open(&archive).map_err(|e| format!("{} — {e}", archive.display()))?;
    add_archive_paths(&mut dict, &ar);

    // Saída: stdout se "-", senão o caminho dado, senão <archive>.datamap.md.
    let out_path = match out.as_deref() {
        Some("-") => None,
        Some(p) => Some(PathBuf::from(p)),
        None => Some(default_datamap_path(&archive)),
    };

    let stats = match &out_path {
        None => {
            let stdout = io::stdout();
            let mut w = BufWriter::new(stdout.lock());
            // Cano fechado a jusante (ex.: `| head`) é saída limpa, padrão Unix.
            let stats = match datamap::write_datamap(&ar, &dict, &mut w) {
                Ok(s) => s,
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
                Err(e) => return Err(format!("falha ao escrever datamap: {e}")),
            };
            if let Err(e) = w.flush() {
                if e.kind() == io::ErrorKind::BrokenPipe {
                    return Ok(());
                }
                return Err(e.to_string());
            }
            stats
        }
        Some(path) => write_datamap_file(&ar, &dict, path)?,
    };

    eprintln!(
        "{} recursos · {} segmentos ({} comprimidos) · {} deps · {} nomes resolvidos · {} em disco / {} descomprimidos",
        stats.entries,
        stats.segments,
        stats.compressed_segments,
        stats.dependencies,
        stats.resolved,
        human(stats.total_disk),
        human(stats.total_uncompressed),
    );
    Ok(())
}

fn cmd_extract(args: &[String]) -> Result<(), String> {
    let mut positionals: Vec<String> = Vec::new();
    let mut hashes: Option<PathBuf> = None;
    let mut no_hashes = false;
    let mut all = false;
    let mut also_datamap = false;
    let mut opts = extract::ExtractOptions::default();

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--all" => all = true,
            "--hashes" => hashes = Some(PathBuf::from(it.next().ok_or("--hashes exige um caminho")?)),
            "--no-hashes" => no_hashes = true,
            "--datamap" => also_datamap = true,
            "--decompress-buffers" => opts.decompress_buffers = true,
            "--skip-unresolved" => opts.keep_unresolved = false,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if other.starts_with('-') => {
                return Err(format!("opção desconhecida em extract: '{other}'"));
            }
            other => positionals.push(other.to_string()),
        }
    }

    let mut dict = load_base_dict(resolve_hashes(hashes, no_hashes).as_deref())?;

    // Define a lista de archives e a base de saída.
    let (archives, base): (Vec<PathBuf>, PathBuf) = if all {
        let content = defaults::content_dir();
        let archives = list_archives(&content)?;
        if archives.is_empty() {
            return Err(format!("nenhum .archive em {}", content.display()));
        }
        // base de saída = positional[0] ou o próprio diretório do jogo (ao lado).
        let base = positionals.first().map(PathBuf::from).unwrap_or_else(|| content.clone());
        eprintln!("--all: {} archives de {}", archives.len(), content.display());
        (archives, base)
    } else {
        let arg = positionals.first().ok_or("extract exige <nome|caminho> ou --all")?;
        let ar_path = resolve_archive_arg(arg)?;
        // base de saída = positional[1] ou o diretório do próprio archive (ao lado).
        let base = positionals
            .get(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| ar_path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(".")));
        (vec![ar_path], base)
    };

    if !kraken::is_available() {
        eprintln!(
            "aviso: Kraken indisponível (build --no-default-features) — recursos KARK serão pulados."
        );
    }

    let multi = archives.len() > 1;
    let (mut g_ext, mut g_skip, mut g_err) = (0usize, 0usize, 0usize);

    for ar_path in &archives {
        let ar = Archive::open(ar_path).map_err(|e| format!("{} — {e}", ar_path.display()))?;
        add_archive_paths(&mut dict, &ar);

        // Pasta de saída = <base>/<nome-do-archive-sem-extensão>/
        let stem = ar_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "archive".into());
        let target = base.join(&stem);
        std::fs::create_dir_all(&target)
            .map_err(|e| format!("não consegui criar {}: {e}", target.display()))?;
        eprintln!(
            "extraindo {} -> {}/",
            ar_path.file_name().unwrap_or_default().to_string_lossy(),
            target.display()
        );

        if also_datamap {
            write_datamap_file(&ar, &dict, &target.join("datamap.md"))?;
        }

        let report = extract::extract_all(&ar, &dict, &target, &opts)
            .map_err(|e| format!("falha na extração de {}: {e}", ar_path.display()))?;
        println!(
            "  {} extraídos · {} pulados · {} erros",
            report.extracted, report.skipped_need_kraken, report.errors
        );
        g_ext += report.extracted;
        g_skip += report.skipped_need_kraken;
        g_err += report.errors;

        // Amostras só no modo single (evita poluir a saída do --all).
        if !multi {
            print_samples(&report);
        }
    }

    if multi {
        println!("TOTAL: {g_ext} extraídos · {g_skip} pulados · {g_err} erros em {} archives", archives.len());
    }
    Ok(())
}

fn print_samples(report: &extract::ExtractReport) {
    if !report.skipped_samples.is_empty() {
        eprintln!("amostra de pulados (precisam de Kraken):");
        for (hash, name) in &report.skipped_samples {
            eprintln!("  {hash:016x}  {name}");
        }
        if report.skipped_need_kraken > report.skipped_samples.len() {
            eprintln!(
                "  … e mais {} recursos",
                report.skipped_need_kraken - report.skipped_samples.len()
            );
        }
    }
    if !report.error_samples.is_empty() {
        eprintln!("amostra de erros de descompressão:");
        for (hash, motivo) in &report.error_samples {
            eprintln!("  {hash:016x}  {motivo}");
        }
    }
}

fn default_datamap_path(archive: &Path) -> PathBuf {
    let mut name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    name.push_str(".datamap.md");
    archive.with_file_name(name)
}

/// Tamanho legível (B/KiB/MiB/GiB) com o número cru entre ().
fn human(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caminho_padrao_do_datamap() {
        let p = default_datamap_path(Path::new("/x/basegame.archive"));
        assert_eq!(p, PathBuf::from("/x/basegame.archive.datamap.md"));
    }

    #[test]
    fn human_formata() {
        assert_eq!(human(512), "512 B");
        assert!(human(2048).starts_with("2.0 KiB"));
    }
}
