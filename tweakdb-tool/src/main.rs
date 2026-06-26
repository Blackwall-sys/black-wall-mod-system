//! tweakdb-tool — CLI para inspecionar o `tweakdb.bin` do Cyberpunk 2077 (macOS).
//!
//! Comandos:
//!   info  [<file|ep1>]                 Resumo de uma linha.
//!   map   [<file|ep1>]                 Estrutura: header, flat types, contagens.
//!   sample [<file|ep1>] [--type <T>] [-n N]
//!                                       Mostra N pares chave→valor de flats.
//!   batch  <changeset> [<file|ep1>] [-o <saida>]
//!                                       Aplica vários edits de uma vez (mesmo
//!                                       mecanismo do `set`) num arquivo novo.
//!
//! O `tweakdb.bin` do jogo vem embutido (este Mac); override por `CP77_DIR`.
//! `ep1` seleciona o `tweakdb_ep1.bin` (Phantom Liberty).

mod hashes;
mod kraken;
mod names;
mod template;
mod toml;
mod tweakdb;
mod tweakxl;
mod ui;
mod writer;
mod yaml;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use names::NameDb;
use tweakdb::TweakDb;
use writer::{EditOp, Model, SetOutcome};

const USAGE: &str = "\
tweakdb-tool — inspeciona o tweakdb.bin do Cyberpunk 2077 (macOS)

USO:
    tweakdb-tool info   [<file|ep1>]
    tweakdb-tool map    [<file|ep1>]
    tweakdb-tool sample [<file|ep1>] [--type <tipo>] [-n <N>] [--no-names]
    tweakdb-tool find   <texto> [<file|ep1>] [-n <N>]
    tweakdb-tool dump   [<file|ep1>] [--type <tipo>] [--filter <texto>] [-o <out.tsv>]
    tweakdb-tool get    <flat> [<file|ep1>]
    tweakdb-tool record <Record.Name> [<file|ep1>]
    tweakdb-tool records [--like <Record>|--class <ClasseRED>|--key <hex>] [-n N] [<file|ep1>]
    tweakdb-tool clone  <Src.Record> <Dst.Record> [<file|ep1>] [-o <saida>]
    tweakdb-tool create <Dst.Record> --class <ClasseRED> [<file|ep1>] [-o <saida>]
    tweakdb-tool set    <flat> <valor> [<file|ep1>] [-o <saida>]
    tweakdb-tool batch  <changeset> [<file|ep1>] [-o <saida>]
    tweakdb-tool check  <changeset> [<file|ep1>]
    tweakdb-tool apply-yaml <arquivo.yaml> [<file|ep1>] [-o <saida>] [--check]
    tweakdb-tool apply-toml <arquivo.toml> [<file|ep1>] [-o <saida>] [--check]
    tweakdb-tool install [<patched.bin>] [ep1]
    tweakdb-tool uninstall [ep1]
    tweakdb-tool backup  [ep1]
    tweakdb-tool status  [ep1]
    tweakdb-tool ui      [<addr>]
    tweakdb-tool roundtrip [<file|ep1>]

Sem <file>, usa o tweakdb.bin do jogo (embutido; override CP77_DIR). `ep1` usa
o tweakdb_ep1.bin (Phantom Liberty). Os nomes (TweakDBID → texto) vêm da
tweakdbstr.kark do projeto (embutida); `--no-names` desliga.

COMANDOS:
    info       Resumo de uma linha (versões, contagens).
    map        Header + tabela de flat types (tipo, #valores, #chaves) + contagens.
    sample     Exemplos de flats (nome/TweakDBID → valor). --type filtra por tipo.
    find       Procura flats cujo NOME contém <texto> e mostra nome + valor.
    dump       Exporta TODOS os flats (nome<TAB>tipo<TAB>valor) para um .tsv.
    get        Lê o valor de UM flat pelo nome.
    record     Mostra todos os flats (propriedades) de um Record (`Items.X`).
    records    Navega records por CLASSE (type_key). Sem flag: histograma de
               classes (type_key, #records, exemplo). `--like <Record>` lista
               todos da mesma classe de um record conhecido; `--class <ClasseRED>`
               via murmur3 do nome da classe; `--key <hex>` por type_key cru.
    clone      Cria um Record NOVO copiando outro (`clone Items.A Items.B`): copia
               todos os flats + a entrada de record. Depois use `set Items.B.x ...`.
    create     Cria um Record do ZERO a partir de uma CLASSE RED (`--class Clothing`
               ou `gamedataWeaponItem_Record`). Infere o schema de uma amostra da
               classe no próprio bin e copia (você sobrescreve os flats depois).
    set        Edita um flat e grava arquivo NOVO (nunca sobrescreve o original).
               Escalar: `set Items.X.range 30` · `set Items.X.name SomeName`
                        `set Items.X.fxAppearance \"$Foo.Bar\"` (TweakDBID por nome).
               Array:  `set Items.X.tags \"[a, b, c]\"` (lista inteira).
    batch      Aplica VÁRIOS edits de um changeset (arquivo). Linhas vazias e
               `# comentário` ignoradas. Operadores:
                 `Flat = valor`      escalar (ou `= [a, b, c]` array inteiro)
                 `Flat += valor`     adiciona elemento ao array
                 `Flat -= valor`     remove elemento do array
               Um arquivo NOVO; reporta aplicado / não-encontrado / não-editável.
    check      Valida um changeset SEM gravar nada (dry-run): confere se cada flat
               existe, se o tipo é editável e se o valor parseia (faixa/sintaxe).
               Sai com erro se houver qualquer problema — use antes de instalar.
    apply-yaml Aplica um tweak no formato declarativo do TweakXL (.yaml): flats
               escalar/array/struct, `$base` (clone+overrides), `$type` (cria de
               amostra), tags de array (!append/!prepend/-once/!remove), records
               INLINE (mapa com $type/$base num flat), !append-from/!merge/
               !prepend-from (mescla array de outro flat) e templates $instances.
               `--check` valida sem gravar. (Schema de classe SEM amostra = runtime.)
    apply-toml Igual ao apply-yaml, mas em TOML (front-end nativo, zero-dep). Record
               = [Items.MeuItem]; flat = `chave = valor`; `'$base' = '...'`; op de
               array = item `{ '!append' = 'X' }` num array. Mesma engine/saida.
    install    Copia um tweakdb editado para o lugar que o JOGO lê (r6/cache/).
               Sem arquivo, instala o `<...>.patched.bin` padrão. Salva o pristino
               (.orig) na 1ª vez e NUNCA o sobrescreve. Valide o efeito in-game.
    uninstall  Restaura o vanilla a partir do pristino (.orig). [ep1] p/ Phantom Liberty.
    backup     Copia o alvo atual para `<alvo>.<epoch>.bak` (snapshot do estado atual).
    status     Mostra alvo, pristino e se o estado é VANILLA ou MODIFICADO.
    ui         Sobe uma GUI WEB local (http://127.0.0.1:7077) pra modar sem linha
               de comando: navegar áreas/records, editar flats, validar e instalar.
    roundtrip  Reescreve sem mudar nada e confirma byte-a-byte (teste do writer).

    -h, --help     ajuda      -V, --version   versão
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
            println!("tweakdb-tool {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "info" => cmd_info(&args[1..]),
        "map" => cmd_map(&args[1..]),
        "sample" => cmd_sample(&args[1..]),
        "find" => cmd_find(&args[1..]),
        "dump" => cmd_dump(&args[1..]),
        "bake" => cmd_bake(&args[1..]),
        "get" => cmd_get(&args[1..]),
        "record" => cmd_record(&args[1..]),
        "records" => cmd_records(&args[1..]),
        "list-areas" | "areas" => cmd_list_areas(&args[1..]),
        "clone" => cmd_clone(&args[1..]),
        "create" => cmd_create(&args[1..]),
        "set" => cmd_set(&args[1..]),
        "batch" => cmd_batch(&args[1..]),
        "check" | "validate" => cmd_check(&args[1..]),
        "apply-yaml" | "yaml" => cmd_apply_yaml(&args[1..]),
        "apply-toml" | "toml" => cmd_apply_toml(&args[1..]),
        "install" => cmd_install(&args[1..]),
        "uninstall" | "restore" => cmd_uninstall(&args[1..]),
        "backup" => cmd_backup(&args[1..]),
        "status" => cmd_status(&args[1..]),
        "ui" => cmd_ui(&args[1..]),
        "roundtrip" => cmd_roundtrip(&args[1..]),
        other => Err(format!("comando desconhecido '{other}'. Use --help.")),
    }
}

/// Raiz do jogo. Ordem: `CP77_DIR` → o path compilado (dev) se existir →
/// locais comuns de instalação no macOS → o path compilado como fallback (mesmo
/// inexistente, p/ a mensagem de erro mostrar algo). Faz o binário shipado achar
/// o jogo em OUTRAS máquinas, não só na minha.
fn game_root() -> PathBuf {
    if let Some(d) = std::env::var_os("CP77_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(h) = &home {
        candidates.push(h.join("Library/Application Support/Steam/steamapps/common/Cyberpunk 2077"));
        candidates.push(h.join("Games/Cyberpunk 2077"));
    }
    candidates.push(PathBuf::from("/Applications/Cyberpunk 2077"));
    candidates
        .iter()
        .find(|p| p.join("r6/cache/tweakdb.bin").is_file())
        .cloned()
        .unwrap_or_else(|| candidates.first().cloned().unwrap_or_else(|| PathBuf::from(".")))
}

/// Caminho da lista de nomes (tweakdbstr.kark). Ordem: `TWEAKDB_NAMES` → ao lado
/// do executável (binário shipado leva a .kark junto) → o path compilado (dev).
fn default_names_path() -> PathBuf {
    if let Some(p) = std::env::var_os("TWEAKDB_NAMES") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside = dir.join("tweakdbstr.kark");
            if beside.is_file() {
                return beside;
            }
        }
    }
    // sem env! (não embute o caminho do projeto no binário). Sem o arquivo ao lado do
    // exe nem TWEAKDB_NAMES, segue sem nomes (não-fatal).
    PathBuf::from("tweakdbstr.kark")
}

/// Carrega os nomes (se não desligado e o arquivo existir). Erros não são fatais.
fn load_names(no_names: bool) -> Option<NameDb> {
    if no_names {
        return None;
    }
    let path = default_names_path();
    if !path.is_file() {
        return None;
    }
    match NameDb::load(&path) {
        Ok(db) => {
            eprintln!(
                "nomes: {} ({} records, {} flats, {} queries) de {}",
                db.len(),
                db.records,
                db.flats,
                db.queries,
                path.display()
            );
            Some(db)
        }
        Err(e) => {
            eprintln!("aviso: não consegui carregar nomes ({e}); seguindo só com hashes.");
            None
        }
    }
}

/// Resolve o argumento de arquivo: caminho existente, `ep1`, ou o tweakdb.bin padrão.
fn resolve_file(arg: Option<&str>) -> PathBuf {
    match arg {
        Some(a) if Path::new(a).is_file() => PathBuf::from(a),
        Some("ep1") => game_root().join("r6/cache/tweakdb_ep1.bin"),
        Some("base") | None => game_root().join("r6/cache/tweakdb.bin"),
        Some(a) => PathBuf::from(a), // caminho dado (pode não existir → erro ao abrir)
    }
}

fn open(arg: Option<&str>) -> Result<TweakDb, String> {
    let path = resolve_file(arg);
    TweakDb::open(&path).map_err(|e| format!("{} — {e}", path.display()))
}

fn positional(args: &[String]) -> Option<&str> {
    args.iter().find(|a| !a.starts_with('-')).map(String::as_str)
}

fn cmd_info(args: &[String]) -> Result<(), String> {
    let db = open(positional(args))?;
    println!(
        "{}: tweakdb blob v{} parser v{} · {} flat types · {} flats (chaves) · {} records · {} queries · {} group tags · checksum {:#010x}",
        db.path.file_name().unwrap_or_default().to_string_lossy(),
        db.blob_version,
        db.parser_version,
        db.flat_types.len(),
        db.total_flat_keys(),
        db.records.len(),
        db.query_count,
        db.group_tag_count,
        db.record_checksum,
    );
    Ok(())
}

fn cmd_map(args: &[String]) -> Result<(), String> {
    let db = open(positional(args))?;
    println!("# tweakdb — {}", db.path.display());
    println!(
        "blob v{} · parser v{} · checksum {:#010x}",
        db.blob_version, db.parser_version, db.record_checksum
    );
    println!(
        "offsets: flats={} records={} queries={} groupTags={}",
        db.flats_offset, db.records_offset, db.queries_offset, db.group_tags_offset
    );
    println!();

    println!("## Flat types ({})", db.flat_types.len());
    println!("{:<26} {:>10} {:>10}  typeHash", "tipo", "#valores", "#chaves");
    let mut unresolved = 0;
    for ft in &db.flat_types {
        let label = match ft.resolved {
            Some(r) => r.label(),
            None => {
                unresolved += 1;
                "??? (não resolvido)".to_string()
            }
        };
        println!(
            "{:<26} {:>10} {:>10}  {:016x}",
            label, ft.value_count, ft.key_count, ft.type_hash
        );
    }
    println!();
    println!(
        "total: {} flats (chaves) · {} records ({} tipos) · {} queries · {} group tags",
        db.total_flat_keys(),
        db.records.len(),
        db.distinct_record_types(),
        db.query_count,
        db.group_tag_count
    );
    if let Some(r) = db.records.first() {
        println!("  ex. de record: id={:016x} typeKey={:08x}", r.id, r.type_key);
    }
    if unresolved > 0 {
        println!("aviso: {unresolved} flat types com typeHash não reconhecido.");
    } else {
        println!("todos os {} flat types resolvidos por nome. ✓", db.flat_types.len());
    }
    Ok(())
}

/// Rótulo de uma chave: o nome resolvido, senão o TweakDBID em hex.
fn key_label(names: &Option<NameDb>, id: u64) -> String {
    match names.as_ref().and_then(|n| n.resolve(id)) {
        Some(name) => name.to_string(),
        None => format!("<{id:016x}>"),
    }
}

fn cmd_sample(args: &[String]) -> Result<(), String> {
    let mut file: Option<&str> = None;
    let mut type_filter: Option<String> = None;
    let mut n = 5usize;
    let mut no_names = false;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--type" => type_filter = Some(it.next().ok_or("--type exige um valor")?.clone()),
            "--no-names" => no_names = true,
            "-n" => {
                n = it
                    .next()
                    .ok_or("-n exige um número")?
                    .parse()
                    .map_err(|_| "-n inválido")?
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if !other.starts_with('-') && file.is_none() => file = Some(other),
            other => return Err(format!("argumento inesperado: '{other}'")),
        }
    }

    let db = open(file)?;
    let names = load_names(no_names);
    let mut shown_types = 0;
    for (i, ft) in db.flat_types.iter().enumerate() {
        let Some(resolved) = ft.resolved else { continue };
        let label = resolved.label();
        if let Some(filter) = &type_filter {
            if !label.eq_ignore_ascii_case(filter) {
                continue;
            }
        }
        let pairs = db
            .read_values(i)
            .map_err(|e| format!("lendo flats de {label}: {e}"))?;
        println!("## {label} — {} chaves", pairs.len());
        for (key_id, value) in pairs.iter().take(n) {
            println!("  {} = {value}", key_label(&names, *key_id));
        }
        if pairs.len() > n {
            println!("  … e mais {} chaves", pairs.len() - n);
        }
        println!();
        shown_types += 1;
    }
    if shown_types == 0 {
        return Err("nenhum flat type resolvido bateu com o filtro".into());
    }
    Ok(())
}

fn cmd_find(args: &[String]) -> Result<(), String> {
    let mut file: Option<&str> = None;
    let mut needle: Option<&str> = None;
    let mut n = 40usize;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-n" => {
                n = it
                    .next()
                    .ok_or("-n exige um número")?
                    .parse()
                    .map_err(|_| "-n inválido")?
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if !other.starts_with('-') && needle.is_none() => needle = Some(other),
            other if !other.starts_with('-') && file.is_none() => file = Some(other),
            other => return Err(format!("argumento inesperado: '{other}'")),
        }
    }
    let needle = needle.ok_or("find exige <texto>")?;
    let names = load_names(false).ok_or("lista de nomes indisponível (precisa da tweakdbstr.kark)")?;
    let db = open(file)?;

    // Constrói o índice nome→valor: lê todos os flats resolvidos uma vez.
    let mut value_of: std::collections::HashMap<u64, tweakdb::FlatValue> =
        std::collections::HashMap::new();
    for (i, ft) in db.flat_types.iter().enumerate() {
        if ft.resolved.is_none() {
            continue;
        }
        if let Ok(pairs) = db.read_values(i) {
            for (id, v) in pairs {
                value_of.insert(id, v);
            }
        }
    }

    // Resultados ordenados por nome, limitados a `n`.
    let mut hits: Vec<(u64, &str)> = names.search(needle).collect();
    hits.sort_by(|a, b| a.1.cmp(b.1));
    let total = hits.len();
    println!("'{needle}': {total} nome(s); mostrando até {n} (com valor de flat):");
    for (id, name) in hits.into_iter().take(n) {
        match value_of.get(&id) {
            Some(v) => println!("  {name} = {v}"),
            None => println!("  {name}  (sem flat neste tweakdb)"),
        }
    }
    if total > n {
        println!("  … e mais {} nome(s)", total - n);
    }
    Ok(())
}

fn cmd_dump(args: &[String]) -> Result<(), String> {
    use std::io::Write;

    let mut file: Option<&str> = None;
    let mut type_filter: Option<String> = None;
    let mut name_filter: Option<String> = None;
    let mut out_path: Option<PathBuf> = None;
    let mut no_names = false;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--type" => type_filter = Some(it.next().ok_or("--type exige um valor")?.clone()),
            "--filter" => {
                name_filter = Some(it.next().ok_or("--filter exige um texto")?.to_ascii_lowercase())
            }
            "-o" | "--output" => out_path = Some(PathBuf::from(it.next().ok_or("-o exige um caminho")?)),
            "--no-names" => no_names = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if !other.starts_with('-') && file.is_none() => file = Some(other),
            other => return Err(format!("argumento inesperado em dump: '{other}'")),
        }
    }

    let db = open(file)?;
    let names = load_names(no_names);
    let out = out_path.unwrap_or_else(|| PathBuf::from("tweakdb-flats.tsv"));
    let handle =
        std::fs::File::create(&out).map_err(|e| format!("criando {}: {e}", out.display()))?;
    let mut w = std::io::BufWriter::new(handle);
    writeln!(w, "# nome\ttipo\tvalor").map_err(|e| e.to_string())?;

    let mut total = 0u64;
    let mut resolved = 0u64;
    let mut namespaces: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    for (i, ft) in db.flat_types.iter().enumerate() {
        let Some(r) = ft.resolved else { continue };
        let label = r.label();
        if let Some(tf) = &type_filter {
            if !label.eq_ignore_ascii_case(tf) {
                continue;
            }
        }
        let pairs = db.read_values(i).map_err(|e| format!("lendo {label}: {e}"))?;
        for (id, value) in pairs {
            let name = names.as_ref().and_then(|n| n.resolve(id));
            if let Some(nf) = &name_filter {
                let hay = name.map(|s| s.to_ascii_lowercase()).unwrap_or_default();
                if !hay.contains(nf) {
                    continue;
                }
            }
            total += 1;
            let display = match name {
                Some(s) => {
                    resolved += 1;
                    let head = s.split('.').next().unwrap_or(s);
                    *namespaces.entry(head.to_string()).or_insert(0) += 1;
                    s.to_string()
                }
                None => format!("<{id:016x}>"),
            };
            writeln!(w, "{display}\t{label}\t{value}").map_err(|e| e.to_string())?;
        }
    }
    w.flush().map_err(|e| e.to_string())?;

    eprintln!("dump: {total} flats ({resolved} com nome) → {}", out.display());
    let mut top: Vec<(String, u64)> = namespaces.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    eprintln!("categorias por prefixo (top 20):");
    for (k, v) in top.iter().take(20) {
        eprintln!("  {v:>9}  {k}");
    }
    Ok(())
}

/// Emite um binário compacto só com os flats ESCALARES que o CET lê
/// (Float/Int/Bool/CName/TweakDBID), ordenado por TweakDBID p/ busca binária.
/// Formato: "BWTDB01\0" (8) + count u64 LE (8) + N×{id u64 LE, val u64 LE, tag u8}.
/// tag: 1=Float (val=f32 bits), 2=Int (val=i64), 3=Bool, 4=CName (val=FNV1a64 da
/// string), 5=TweakDBID (val=u64). É a via do `TweakDB():GetFloat(...)` do console
/// sem chamar o jogo (o getter in-game trava). Ver memória cp77-frida-console-plan.
fn cmd_bake(args: &[String]) -> Result<(), String> {
    use std::io::Write;
    use tweakdb::FlatValue;

    let mut file: Option<&str> = None;
    let mut out_path: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => {
                out_path = Some(PathBuf::from(it.next().ok_or("-o exige um caminho")?))
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if !other.starts_with('-') && file.is_none() => file = Some(other),
            other => return Err(format!("argumento inesperado em bake: '{other}'")),
        }
    }

    let db = open(file)?;
    let out = out_path.unwrap_or_else(|| PathBuf::from("tweakdb-scalars.bin"));

    let mut recs: Vec<(u64, u64, u8)> = Vec::new();
    for (i, ft) in db.flat_types.iter().enumerate() {
        let Some(r) = ft.resolved else { continue };
        if r.is_array {
            continue;
        }
        // mapeia o índice de TWEAK_TYPES → tag escalar do console
        let tag: u8 = match r.index {
            4 => 1,                                  // CFloat
            6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 => 2,  // Uint8/16/32/64, Int8/16/32/64
            5 => 3,                                  // CBool
            0 => 4,                                  // CName (string no disco → FNV1a64)
            2 => 5,                                  // TweakDBID
            _ => continue,                           // CString/CResource/LocKey/vec/color: v1 pula
        };
        let Ok(pairs) = db.read_values(i) else { continue };
        for (id, value) in pairs {
            let val: u64 = match (&value, tag) {
                (FlatValue::Float(f), 1) => f.to_bits() as u64,
                (FlatValue::Bool(b), 3) => *b as u64,
                (FlatValue::U8(v), 2) => *v as u64,
                (FlatValue::U16(v), 2) => *v as u64,
                (FlatValue::U32(v), 2) => *v as u64,
                (FlatValue::U64(v), 2) => *v,
                (FlatValue::I8(v), 2) => *v as i64 as u64,
                (FlatValue::I16(v), 2) => *v as i64 as u64,
                (FlatValue::I32(v), 2) => *v as i64 as u64,
                (FlatValue::I64(v), 2) => *v as u64,
                (FlatValue::Str(s), 4) => crate::hashes::fnv1a64(s.as_bytes()),
                (FlatValue::Id(v), 5) => *v,
                _ => continue,
            };
            recs.push((id, val, tag));
        }
    }
    recs.sort_by_key(|r| r.0);
    recs.dedup_by_key(|r| r.0); // ids únicos p/ busca binária

    let handle =
        std::fs::File::create(&out).map_err(|e| format!("criando {}: {e}", out.display()))?;
    let mut w = std::io::BufWriter::new(handle);
    w.write_all(b"BWTDB01\0").map_err(|e| e.to_string())?;
    w.write_all(&(recs.len() as u64).to_le_bytes())
        .map_err(|e| e.to_string())?;
    for (id, val, tag) in &recs {
        w.write_all(&id.to_le_bytes()).map_err(|e| e.to_string())?;
        w.write_all(&val.to_le_bytes()).map_err(|e| e.to_string())?;
        w.write_all(std::slice::from_ref(tag)).map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())?;

    let bytes = 16 + recs.len() * 17;
    eprintln!(
        "bake: {} flats escalares → {} ({:.1} MB)",
        recs.len(),
        out.display(),
        bytes as f64 / 1.0e6
    );
    Ok(())
}

fn cmd_get(args: &[String]) -> Result<(), String> {
    let mut pos: Vec<&str> = Vec::new();
    for a in args {
        if a == "-h" || a == "--help" {
            print!("{USAGE}");
            return Ok(());
        }
        if !a.starts_with('-') {
            pos.push(a);
        }
    }
    let flat = *pos.first().ok_or("get exige <flat>")?;
    let db = open(pos.get(1).copied())?;
    let id = hashes::tweak_db_id(flat);
    for (i, ft) in db.flat_types.iter().enumerate() {
        let Some(r) = ft.resolved else { continue };
        let pairs = db.read_values(i).map_err(|e| e.to_string())?;
        if let Some((_, v)) = pairs.iter().find(|(kid, _)| *kid == id) {
            println!("{flat} ({}) = {v}", r.label());
            return Ok(());
        }
    }
    Err(format!("flat '{flat}' (id {id:016x}) não encontrado neste tweakdb"))
}

/// Formata um valor resolvendo TweakDBIDs (Id, e arrays de Id) -> `$Nome` via a NameDb.
/// Sem isso, refs saem como `TDBID(0000...)` cru (ilegível). Os demais tipos usam o Display normal.
fn fmt_value_resolved(v: &crate::tweakdb::FlatValue, names: &crate::names::NameDb) -> String {
    use crate::tweakdb::FlatValue;
    match v {
        FlatValue::Id(id) => match names.resolve(*id) {
            Some(name) => format!("${name}"),
            None => format!("TDBID({id:016x})"),
        },
        FlatValue::Array(items) => {
            let inner: Vec<String> = items.iter().map(|it| fmt_value_resolved(it, names)).collect();
            format!("[{}]", inner.join(", "))
        }
        other => other.to_string(),
    }
}

/// `list-areas` — mapa do banco por prefixo de nome (Items/Character/Vehicle/...). Roda o MESMO
/// histograma do `dump` (main: namespaces por prefixo) mas SÓ o resumo, sem escrever o TSV de ~248MB.
fn cmd_list_areas(args: &[String]) -> Result<(), String> {
    let mut pos: Vec<&str> = Vec::new();
    for a in args {
        if a == "-h" || a == "--help" {
            print!("{USAGE}");
            return Ok(());
        }
        if !a.starts_with('-') {
            pos.push(a);
        }
    }
    let names = load_names(false).ok_or("list-areas precisa da lista de nomes (tweakdbstr.kark)")?;
    let db = open(pos.first().copied())?;
    let mut areas: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let (mut total, mut resolved) = (0u64, 0u64);
    for (i, ft) in db.flat_types.iter().enumerate() {
        if ft.resolved.is_none() {
            continue;
        }
        for (id, _v) in db.read_values(i).map_err(|e| e.to_string())? {
            total += 1;
            if let Some(name) = names.resolve(id) {
                resolved += 1;
                let head = name.split('.').next().unwrap_or(name);
                *areas.entry(head.to_string()).or_insert(0) += 1;
            }
        }
    }
    let mut top: Vec<(String, u64)> = areas.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("# áreas por prefixo — {total} flats ({resolved} com nome) em {} áreas", top.len());
    for (k, v) in &top {
        println!("  {v:>9}  {k}");
    }
    Ok(())
}

fn cmd_record(args: &[String]) -> Result<(), String> {
    let mut pos: Vec<&str> = Vec::new();
    for a in args {
        if a == "-h" || a == "--help" {
            print!("{USAGE}");
            return Ok(());
        }
        if !a.starts_with('-') {
            pos.push(a);
        }
    }
    let record = *pos.first().ok_or("record exige <Record.Name>")?;
    let names = load_names(false).ok_or("record precisa da lista de nomes (tweakdbstr.kark)")?;
    let db = open(pos.get(1).copied())?;

    let prefix = format!("{record}.");
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for (i, ft) in db.flat_types.iter().enumerate() {
        let Some(r) = ft.resolved else { continue };
        let label = r.label();
        for (id, v) in db.read_values(i).map_err(|e| e.to_string())? {
            if let Some(name) = names.resolve(id) {
                if let Some(prop) = name.strip_prefix(&prefix) {
                    rows.push((prop.to_string(), label.clone(), fmt_value_resolved(&v, &names)));
                }
            }
        }
    }
    if rows.is_empty() {
        return Err(format!("nenhum flat encontrado para o record '{record}'"));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    println!("# {record} — {} propriedades", rows.len());
    for (prop, ty, v) in &rows {
        println!("  {prop} ({ty}) = {v}");
    }
    Ok(())
}

fn cmd_records(args: &[String]) -> Result<(), String> {
    let mut pos: Vec<&str> = Vec::new();
    let mut like: Option<&str> = None;
    let mut class: Option<&str> = None;
    let mut key_hex: Option<&str> = None;
    let mut limit: usize = 40;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "--like" => like = Some(it.next().ok_or("--like exige <Record.Name>")?),
            "--class" => class = Some(it.next().ok_or("--class exige <ClasseRED>")?),
            "--key" => key_hex = Some(it.next().ok_or("--key exige <hex>")?),
            "-n" => {
                limit = it
                    .next()
                    .ok_or("-n exige um número")?
                    .parse()
                    .map_err(|_| "-n inválido".to_string())?
            }
            other if !other.starts_with('-') => pos.push(other),
            other => return Err(format!("opção desconhecida em records: '{other}'")),
        }
    }
    let names = load_names(false).ok_or("records precisa da lista de nomes (tweakdbstr.kark)")?;
    let db = open(pos.first().copied())?;

    // --like / --class / --key escolhem um type_key e listam os records dele.
    let filter_key: Option<u32> = if let Some(name) = like {
        let id = crate::hashes::tweak_db_id(name);
        let rec = db
            .records
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("record '{name}' não encontrado"))?;
        println!("# records da mesma classe de {name} (type_key {:08x})", rec.type_key);
        Some(rec.type_key)
    } else if let Some(cls) = class {
        let k = crate::hashes::record_type_key(cls);
        println!("# records da classe {cls} (type_key {k:08x})");
        Some(k)
    } else if let Some(hx) = key_hex {
        let k = u32::from_str_radix(hx.trim_start_matches("0x"), 16)
            .map_err(|_| format!("--key hex inválido: '{hx}'"))?;
        println!("# records com type_key {k:08x}");
        Some(k)
    } else {
        None
    };

    if let Some(k) = filter_key {
        let mut named: Vec<&str> = Vec::new();
        let mut unnamed = 0usize;
        for r in &db.records {
            if r.type_key == k {
                match names.resolve(r.id) {
                    Some(n) => named.push(n),
                    None => unnamed += 1,
                }
            }
        }
        named.sort_unstable();
        println!("{} records ({} com nome)", named.len() + unnamed, named.len());
        for n in named.iter().take(limit) {
            println!("  {n}");
        }
        if named.len() > limit {
            println!("  … (+{} — use -n)", named.len() - limit);
        }
        if unnamed > 0 {
            println!("  ({unnamed} sem nome resolvido)");
        }
        return Ok(());
    }

    // Histograma: agrupa todos os records por type_key (classe RED).
    use std::collections::HashMap;
    let mut by_key: HashMap<u32, (usize, Option<&str>)> = HashMap::new();
    for r in &db.records {
        let e = by_key.entry(r.type_key).or_insert((0, None));
        e.0 += 1;
        if e.1.is_none() {
            e.1 = names.resolve(r.id);
        }
    }
    let mut top: Vec<(u32, usize, Option<&str>)> =
        by_key.into_iter().map(|(k, (c, n))| (k, c, n)).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!(
        "# {} classes de record ({} records) — ordenado por contagem",
        top.len(),
        db.records.len()
    );
    println!("{:>8}  {:>9}  exemplo de record", "type_key", "#records");
    for (k, c, ex) in top.iter().take(limit) {
        println!("{k:08x}  {c:>9}  {}", ex.unwrap_or("(sem nome resolvido)"));
    }
    if top.len() > limit {
        println!("… (+{} classes — use -n)", top.len() - limit);
    }
    Ok(())
}

fn cmd_create(args: &[String]) -> Result<(), String> {
    let mut pos: Vec<&str> = Vec::new();
    let mut class: Option<&str> = None;
    let mut out_path: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--class" => class = Some(it.next().ok_or("--class exige <ClasseRED>")?),
            "-o" | "--output" => out_path = Some(PathBuf::from(it.next().ok_or("-o exige um caminho")?)),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if !other.starts_with('-') => pos.push(other),
            other => return Err(format!("opção desconhecida em create: '{other}'")),
        }
    }
    let dst = *pos.first().ok_or("create exige <Dst.Record>")?;
    let class = class
        .ok_or("create exige --class <ClasseRED> (ex.: Clothing, gamedataWeaponItem_Record)")?;
    let file = pos.get(1).copied();

    let names = load_names(false).ok_or("create precisa da lista de nomes (tweakdbstr.kark)")?;
    let in_path = resolve_file(file);
    let db = open(file)?;
    let mut model = Model::from_db(&db).map_err(|e| format!("modelo: {e}"))?;
    let (sample, n) = model.create_record(dst, class, &names)?;
    let bytes = model.serialize();
    let out = out_path.unwrap_or_else(|| default_patched_path(&in_path));
    std::fs::write(&out, &bytes).map_err(|e| format!("gravando {}: {e}", out.display()))?;
    println!(
        "criado {dst} (classe {class}) a partir da amostra {sample}: {n} flats  →  {} ({} bytes)",
        out.display(),
        bytes.len()
    );
    eprintln!("customize com `set {dst}.<prop> <valor>` (ou apply-yaml) e instale com `install`.");
    Ok(())
}

fn cmd_clone(args: &[String]) -> Result<(), String> {
    let mut pos: Vec<&str> = Vec::new();
    let mut out_path: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => out_path = Some(PathBuf::from(it.next().ok_or("-o exige um caminho")?)),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if !other.starts_with('-') => pos.push(other),
            other => return Err(format!("opção desconhecida em clone: '{other}'")),
        }
    }
    let src = *pos.first().ok_or("clone exige <Src.Record>")?;
    let dst = *pos.get(1).ok_or("clone exige <Dst.Record>")?;
    let file = pos.get(2).copied();

    let names = load_names(false).ok_or("clone precisa da lista de nomes (tweakdbstr.kark)")?;
    let in_path = resolve_file(file);
    let db = open(file)?;
    let mut model = Model::from_db(&db).map_err(|e| format!("modelo: {e}"))?;
    let n = model.clone_record(src, dst, &names)?;
    let bytes = model.serialize();

    let out = out_path.unwrap_or_else(|| default_patched_path(&in_path));
    std::fs::write(&out, &bytes).map_err(|e| format!("gravando {}: {e}", out.display()))?;
    println!(
        "clonado {src} → {dst}: {n} flats + 1 record  →  {} ({} bytes)",
        out.display(),
        bytes.len()
    );
    eprintln!("customize com `set {dst}.<prop> <valor>` e troque o tweakdb.bin (backup antes).");
    Ok(())
}

fn cmd_roundtrip(args: &[String]) -> Result<(), String> {
    let db = open(positional(args))?;
    let model = Model::from_db(&db).map_err(|e| format!("modelo: {e}"))?;
    let out = model.serialize();
    if out == db.data {
        println!("round-trip byte-idêntico ✓ ({} bytes)", out.len());
        Ok(())
    } else {
        let first = out
            .iter()
            .zip(&db.data)
            .position(|(a, b)| a != b)
            .map(|p| p.to_string())
            .unwrap_or_else(|| "(só o tamanho difere)".into());
        Err(format!(
            "round-trip DIFERE: {} vs {} bytes; 1ª diferença no offset {first}",
            out.len(),
            db.data.len()
        ))
    }
}

fn cmd_set(args: &[String]) -> Result<(), String> {
    let mut pos: Vec<&str> = Vec::new();
    let mut out_path: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => out_path = Some(PathBuf::from(it.next().ok_or("-o exige um caminho")?)),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if !other.starts_with('-') => pos.push(other),
            other => return Err(format!("opção desconhecida em set: '{other}'")),
        }
    }
    let flat = *pos.first().ok_or("set exige <flat> (nome do flat)")?;
    let value = *pos.get(1).ok_or("set exige <valor>")?;
    let file = pos.get(2).copied();

    let in_path = resolve_file(file);
    let db = open(file)?;
    let mut model = Model::from_db(&db).map_err(|e| format!("modelo: {e}"))?;
    let ty = model.set_flat(flat, value)?;
    let bytes = model.serialize();

    let out = out_path.unwrap_or_else(|| default_patched_path(&in_path));
    std::fs::write(&out, &bytes).map_err(|e| format!("gravando {}: {e}", out.display()))?;
    println!(
        "set {flat} ({ty}) = {value}  →  {} ({} bytes)",
        out.display(),
        bytes.len()
    );
    eprintln!(
        "para usar no jogo, substitua o tweakdb.bin por este arquivo (faça backup do original)."
    );
    Ok(())
}

/// Uma linha de changeset já analisada: nome do flat e a operação.
struct ChangeEdit {
    line: usize,
    flat: String,
    op: EditOp,
}

/// Lê um changeset. Linhas vazias e `#` = comentário. Operadores:
/// `Nome.Flat = valor` (escalar) · `Nome.Flat = [a, b, c]` (array inteiro) ·
/// `Nome.Flat += valor` (append) · `Nome.Flat -= valor` (remove).
fn parse_changeset(text: &str) -> Result<Vec<ChangeEdit>, String> {
    let mut edits = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Acha o '=' e decide pela operação pelo char anterior (+= / -= / =).
        let eq = trimmed
            .find('=')
            .ok_or_else(|| format!("linha {line}: falta '=' (use `Flat = v`, `+= v` ou `-= v`)"))?;
        let (name_part, op_ctor): (&str, fn(String) -> EditOp) = match trimmed[..eq].chars().last() {
            Some('+') => (&trimmed[..eq - 1], EditOp::Append),
            Some('-') => (&trimmed[..eq - 1], EditOp::Remove),
            _ => (&trimmed[..eq], EditOp::Assign),
        };
        let flat = name_part.trim();
        let value = trimmed[eq + 1..].trim();
        if flat.is_empty() {
            return Err(format!("linha {line}: nome do flat vazio"));
        }
        if value.is_empty() {
            return Err(format!("linha {line}: valor vazio para '{flat}'"));
        }
        edits.push(ChangeEdit {
            line,
            flat: flat.to_string(),
            op: op_ctor(value.to_string()),
        });
    }
    Ok(edits)
}

fn cmd_batch(args: &[String]) -> Result<(), String> {
    let mut pos: Vec<&str> = Vec::new();
    let mut out_path: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => out_path = Some(PathBuf::from(it.next().ok_or("-o exige um caminho")?)),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if !other.starts_with('-') => pos.push(other),
            other => return Err(format!("opção desconhecida em batch: '{other}'")),
        }
    }
    let changeset = *pos.first().ok_or("batch exige <changeset> (arquivo de edits)")?;
    let file = pos.get(1).copied();

    let text = std::fs::read_to_string(changeset)
        .map_err(|e| format!("lendo changeset {changeset}: {e}"))?;
    let edits = parse_changeset(&text)?;
    if edits.is_empty() {
        return Err(format!("changeset {changeset} não tem nenhum edit"));
    }

    let in_path = resolve_file(file);
    let db = open(file)?;
    let mut model = Model::from_db(&db).map_err(|e| format!("modelo: {e}"))?;

    let mut applied = 0usize;
    let mut not_found = 0usize;
    let mut not_editable = 0usize;
    for e in &edits {
        match model.apply(&e.flat, &e.op) {
            SetOutcome::Applied(ty) => {
                applied += 1;
                println!("  aplicado       {} ({ty})", e.flat);
            }
            SetOutcome::NotFound => {
                not_found += 1;
                println!("  não-encontrado {} (linha {})", e.flat, e.line);
            }
            SetOutcome::NotEditable { ty, reason } => {
                not_editable += 1;
                println!("  não-editável   {} ({ty}, linha {}): {reason}", e.flat, e.line);
            }
        }
    }

    println!(
        "batch: {} edits — {applied} aplicado(s), {not_found} não-encontrado(s), {not_editable} tipo-não-editável",
        edits.len()
    );

    if applied == 0 {
        return Err("nenhum edit aplicado; nada gravado".into());
    }

    let bytes = model.serialize();
    let out = out_path.unwrap_or_else(|| default_patched_path(&in_path));
    std::fs::write(&out, &bytes).map_err(|e| format!("gravando {}: {e}", out.display()))?;
    println!("→ {} ({} bytes)", out.display(), bytes.len());
    eprintln!(
        "para usar no jogo, substitua o tweakdb.bin por este arquivo (faça backup do original)."
    );
    Ok(())
}

fn cmd_check(args: &[String]) -> Result<(), String> {
    let mut pos: Vec<&str> = Vec::new();
    for a in args {
        if a == "-h" || a == "--help" {
            print!("{USAGE}");
            return Ok(());
        }
        if a.starts_with('-') {
            return Err(format!("opção desconhecida em check: '{a}'"));
        }
        pos.push(a);
    }
    let changeset = *pos.first().ok_or("check exige <changeset> (arquivo de edits)")?;
    let file = pos.get(1).copied();

    let text = std::fs::read_to_string(changeset)
        .map_err(|e| format!("lendo changeset {changeset}: {e}"))?;
    let edits = parse_changeset(&text)?;
    if edits.is_empty() {
        return Err(format!("changeset {changeset} não tem nenhum edit"));
    }

    // Dry-run: aplica no modelo em memória pra validar (existe? tipo editável?
    // valor parseia/faixa?), mas NUNCA serializa nem grava. Aplica em sequência
    // para que `+=`/`-=` validem contra o estado já editado (igual ao `batch`).
    let db = open(file)?;
    let mut model = Model::from_db(&db).map_err(|e| format!("modelo: {e}"))?;

    let mut ok = 0usize;
    let mut not_found = 0usize;
    let mut invalid = 0usize;
    for e in &edits {
        match model.apply(&e.flat, &e.op) {
            SetOutcome::Applied(ty) => {
                ok += 1;
                println!("  ok          {} ({ty})", e.flat);
            }
            SetOutcome::NotFound => {
                not_found += 1;
                println!("  ✗ inexistente {} (linha {})", e.flat, e.line);
            }
            SetOutcome::NotEditable { ty, reason } => {
                invalid += 1;
                println!("  ✗ inválido    {} ({ty}, linha {}): {reason}", e.flat, e.line);
            }
        }
    }

    let problems = not_found + invalid;
    println!(
        "check: {} edits — {ok} ok, {not_found} inexistente(s), {invalid} inválido(s) — {}",
        edits.len(),
        if problems == 0 { "tudo certo ✓ (nada gravado)" } else { "NÃO instalar" }
    );
    if problems > 0 {
        return Err(format!("{problems} problema(s) no changeset; corrija antes de aplicar"));
    }
    Ok(())
}

/// O tweakdb que o JOGO lê no startup (loose-files no macOS): `r6/cache/`.
/// `ep1` = a base do Phantom Liberty.
fn install_target(ep1: bool) -> PathBuf {
    let name = if ep1 { "tweakdb_ep1.bin" } else { "tweakdb.bin" };
    game_root().join("r6/cache").join(name)
}

/// O pristino (vanilla) guardado ao lado do alvo: `<alvo>.orig`. Criado UMA vez
/// no 1º install e NUNCA sobrescrito → `uninstall` sempre volta ao vanilla real.
fn pristine_path(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(".orig");
    PathBuf::from(s)
}

/// `ep1` entre os args posicionais? (escolhe a base do Phantom Liberty).
fn wants_ep1(args: &[String]) -> bool {
    args.iter().any(|a| a == "ep1")
}

/// Segundos desde a época Unix (sufixo de backup, zero-dep e ordenável).
fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cmd_apply_yaml(args: &[String]) -> Result<(), String> {
    apply_decl(args, "apply-yaml", crate::yaml::parse)
}

fn cmd_apply_toml(args: &[String]) -> Result<(), String> {
    apply_decl(args, "apply-toml", crate::toml::parse)
}

/// Núcleo comum de `apply-yaml`/`apply-toml`: lê o arquivo declarativo, parseia
/// pelo `parse` dado (YAML ou TOML → mesma AST), interpreta e aplica no Model.
fn apply_decl(
    args: &[String],
    cmd: &str,
    parse: fn(&str) -> Result<crate::yaml::Node, String>,
) -> Result<(), String> {
    let mut pos: Vec<&str> = Vec::new();
    let mut out_path: Option<PathBuf> = None;
    let mut check_only = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => out_path = Some(PathBuf::from(it.next().ok_or("-o exige um caminho")?)),
            "--check" => check_only = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if !other.starts_with('-') => pos.push(other),
            other => return Err(format!("opção desconhecida em {cmd}: '{other}'")),
        }
    }
    let src_path = *pos.first().ok_or_else(|| format!("{cmd} exige <arquivo>"))?;
    let file = pos.get(1).copied();

    let text = std::fs::read_to_string(src_path).map_err(|e| format!("lendo {src_path}: {e}"))?;
    let root = parse(&text)?;
    let root = crate::template::expand(&root)?; // $instances → records concretos
    let ops = crate::tweakxl::interpret_from(&root, src_path)?;
    if ops.is_empty() {
        return Err(format!("{src_path} não produziu nenhuma operação"));
    }

    let names = load_names(false).ok_or_else(|| format!("{cmd} precisa da lista de nomes (tweakdbstr.kark)"))?;
    let in_path = resolve_file(file);
    let db = open(file)?;
    let mut model = Model::from_db(&db).map_err(|e| format!("modelo: {e}"))?;

    let results = crate::tweakxl::apply_ops(&mut model, &names, &ops);
    let mut ok = 0usize;
    let mut fail = 0usize;
    for r in &results {
        if r.ok {
            ok += 1;
            println!("  ok    {} ({})", r.desc, r.detail);
        } else {
            fail += 1;
            println!("  ✗     {} — {}", r.desc, r.detail);
        }
    }
    println!("{src_path}: {} ops — {ok} ok, {fail} com erro", results.len());

    if check_only {
        if fail > 0 {
            return Err(format!("{fail} problema(s); nada gravado (--check)"));
        }
        println!("tudo certo ✓ (--check, nada gravado)");
        return Ok(());
    }
    if fail > 0 {
        return Err(format!("{fail} op(s) falharam; nada gravado"));
    }

    let bytes = model.serialize();
    let out = out_path.unwrap_or_else(|| default_patched_path(&in_path));
    std::fs::write(&out, &bytes).map_err(|e| format!("gravando {}: {e}", out.display()))?;
    println!("→ {} ({} bytes)", out.display(), bytes.len());
    eprintln!("valide no jogo e instale com `install` (backup automático do pristino).");
    Ok(())
}

fn cmd_install(args: &[String]) -> Result<(), String> {
    let mut pos: Vec<&str> = Vec::new();
    for a in args {
        if a == "-h" || a == "--help" {
            print!("{USAGE}");
            return Ok(());
        }
        if a.starts_with('-') {
            return Err(format!("opção desconhecida em install: '{a}'"));
        }
        pos.push(a);
    }
    let ep1 = pos.iter().any(|a| *a == "ep1");
    let target = install_target(ep1);
    // O arquivo a instalar: 1º posicional que não seja "ep1"; senão o .patched.bin padrão.
    let patched: PathBuf = match pos.iter().find(|a| **a != "ep1") {
        Some(p) => PathBuf::from(p),
        None => default_patched_path(&target),
    };

    if !patched.is_file() {
        return Err(format!(
            "arquivo a instalar não existe: {} (gere com `set`/`batch` ou passe o caminho)",
            patched.display()
        ));
    }
    if !target.is_file() {
        return Err(format!(
            "alvo do jogo não existe: {} (CP77_DIR correto? jogo instalado?)",
            target.display()
        ));
    }
    // Sanidade: o arquivo a instalar é um tweakdb válido (não instalar lixo).
    TweakDb::open(&patched).map_err(|e| format!("{} não é um tweakdb válido — {e}", patched.display()))?;

    // Snapshot do pristino, só na 1ª vez.
    let orig = pristine_path(&target);
    if !orig.exists() {
        std::fs::copy(&target, &orig)
            .map_err(|e| format!("salvando pristino {}: {e}", orig.display()))?;
        println!("pristino salvo: {}", orig.display());
    } else {
        println!("pristino já existe (mantido): {}", orig.display());
    }

    std::fs::copy(&patched, &target)
        .map_err(|e| format!("instalando em {}: {e}", target.display()))?;
    println!("instalado: {} → {}", patched.display(), target.display());
    eprintln!(
        "lance o jogo e VALIDE in-game (mude algo visível, ex. dano de arma). `uninstall` volta ao vanilla."
    );
    Ok(())
}

fn cmd_uninstall(args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    let target = install_target(wants_ep1(args));
    let orig = pristine_path(&target);
    if !orig.is_file() {
        return Err(format!(
            "sem pristino para restaurar ({} não existe) — nada foi instalado por este tool",
            orig.display()
        ));
    }
    std::fs::copy(&orig, &target)
        .map_err(|e| format!("restaurando {}: {e}", target.display()))?;
    println!("restaurado o vanilla: {} → {}", orig.display(), target.display());
    Ok(())
}

fn cmd_backup(args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    let target = install_target(wants_ep1(args));
    if !target.is_file() {
        return Err(format!("alvo não existe: {}", target.display()));
    }
    let mut s = target.as_os_str().to_os_string();
    s.push(format!(".{}.bak", epoch_secs()));
    let bak = PathBuf::from(s);
    std::fs::copy(&target, &bak).map_err(|e| format!("backup {}: {e}", bak.display()))?;
    println!("backup: {} → {}", target.display(), bak.display());
    Ok(())
}

fn cmd_ui(args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    let addr = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(String::as_str)
        .unwrap_or("127.0.0.1:7077");
    ui::serve(addr)
}

fn cmd_status(args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    let ep1 = wants_ep1(args);
    let target = install_target(ep1);
    let orig = pristine_path(&target);
    println!("# status do tweakdb ({})", if ep1 { "ep1" } else { "base" });

    let t_meta = std::fs::metadata(&target);
    match &t_meta {
        Ok(m) => println!("  alvo do jogo : {} ({} bytes)", target.display(), m.len()),
        Err(_) => {
            println!("  alvo do jogo : {} (NÃO existe)", target.display());
            return Ok(());
        }
    }

    if !orig.is_file() {
        println!("  pristino     : nenhum (.orig) — estado: VANILLA (nada instalado por este tool)");
        return Ok(());
    }
    let o_len = std::fs::metadata(&orig).map(|m| m.len()).unwrap_or(0);
    println!("  pristino     : {} ({o_len} bytes)", orig.display());
    // Igual ao pristino byte-a-byte? (comparação barata: tamanho, depois conteúdo).
    let same = match (std::fs::read(&target), std::fs::read(&orig)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    println!("  estado       : {}", if same { "VANILLA (alvo == pristino)" } else { "MODIFICADO (alvo != pristino) — `uninstall` reverte" });
    Ok(())
}

/// Caminho de saída padrão do `set`: `<stem>.patched.bin` ao lado do original.
fn default_patched_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tweakdb".into());
    input.with_file_name(format!("{stem}.patched.bin"))
}

#[cfg(test)]
mod tests {
    use super::{parse_changeset, ChangeEdit, EditOp};

    /// Extrai o texto-valor de um op (qualquer variante) pra checagem nos testes.
    fn op_value(e: &ChangeEdit) -> &str {
        match &e.op {
            EditOp::Assign(v)
            | EditOp::Append(v)
            | EditOp::AppendOnce(v)
            | EditOp::Prepend(v)
            | EditOp::PrependOnce(v)
            | EditOp::Remove(v)
            | EditOp::AppendFrom(v)
            | EditOp::PrependFrom(v) => v,
        }
    }
    fn is_append(e: &ChangeEdit) -> bool {
        matches!(e.op, EditOp::Append(_))
    }
    fn is_remove(e: &ChangeEdit) -> bool {
        matches!(e.op, EditOp::Remove(_))
    }

    #[test]
    fn changeset_ignora_vazias_e_comentarios() {
        let txt = "\
# comentário
Items.Foo.range = 30

   # outro comentário
Items.Bar.damage = 12.5
Weapons.Baz.enabled = true
";
        let edits = parse_changeset(txt).unwrap();
        assert_eq!(edits.len(), 3);
        assert_eq!(edits[0].flat, "Items.Foo.range");
        assert_eq!(op_value(&edits[0]), "30");
        assert_eq!(edits[1].flat, "Items.Bar.damage");
        assert_eq!(op_value(&edits[1]), "12.5");
        assert_eq!(edits[2].flat, "Weapons.Baz.enabled");
        assert_eq!(op_value(&edits[2]), "true");
    }

    #[test]
    fn changeset_apara_espacos_e_aceita_igual_no_valor() {
        // Só o 1º '=' separa; valor pode conter '='.
        let edits = parse_changeset("  Items.X.expr  =  a=b  \n").unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].flat, "Items.X.expr");
        assert_eq!(op_value(&edits[0]), "a=b");
    }

    #[test]
    fn changeset_detecta_operadores_de_array() {
        let edits = parse_changeset(
            "Items.X.tags += Stealth\nItems.X.tags -= Loud\nItems.X.list = [a, b]\n",
        )
        .unwrap();
        assert_eq!(edits.len(), 3);
        assert!(is_append(&edits[0]) && edits[0].flat == "Items.X.tags" && op_value(&edits[0]) == "Stealth");
        assert!(is_remove(&edits[1]) && op_value(&edits[1]) == "Loud");
        assert!(matches!(edits[2].op, EditOp::Assign(_)) && op_value(&edits[2]) == "[a, b]");
    }

    #[test]
    fn changeset_erra_sem_igual() {
        assert!(parse_changeset("Items.SemIgual 30\n").is_err());
    }

    #[test]
    fn changeset_erra_flat_ou_valor_vazio() {
        assert!(parse_changeset(" = 30\n").is_err());
        assert!(parse_changeset("Items.X = \n").is_err());
    }
}
