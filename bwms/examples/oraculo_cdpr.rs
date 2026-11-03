//! Oráculo do intérprete de `.tweak`: o `tweakdb.bin` do jogo é compilado das fontes do TweakDB, então cada atribuição que o intérprete tira
//! delas tem que bater com o flat compilado. Diferença = erro de semântica do intérprete (ou fonte de outra versão que o `.bin`).
//!
//! Uso:   cargo run --release --example oraculo_cdpr -- <pasta-tweaks> <tweakdb_ep1.bin> [tweakdb.bin] [saida-das-diferencas]
//! Lê só; o `.bin` do jogo não é tocado.

#[allow(dead_code)]
#[path = "../src/tweak/tweakdb.rs"]
mod tweakdb;

use bwms_primitives::operacoes::{texto_em_lista, EditOp, Op};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use tweakdb::{FlatValue, TweakDb, TWEAK_TYPES};

fn arquivos(dir: &Path, saida: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    v.sort();
    for p in v {
        if p.is_dir() {
            arquivos(&p, saida);
        } else if p.extension().is_some_and(|e| e == "tweak") {
            saida.push(p);
        }
    }
}

fn flats(db: &TweakDb, mapa: &mut HashMap<u64, (String, FlatValue)>) {
    for (i, ft) in db.flat_types.iter().enumerate() {
        let Some(r) = ft.resolved else { continue };
        let Ok(pares) = db.read_values(i) else { continue };
        for (id, v) in pares {
            mapa.entry(id).or_insert((r.label(), v));
        }
    }
}

fn num(t: &str) -> &str {
    t.trim().trim_end_matches('f')
}

/// O texto do intérprete bate com o valor compilado do tipo `base` (rótulo do [`TWEAK_TYPES`])?
fn bate(base: &str, v: &FlatValue, t: &str) -> bool {
    match v {
        FlatValue::Float(f) => num(t).parse::<f32>().is_ok_and(|x| (x - f).abs() <= 1e-4 * f.abs().max(1.0)),
        FlatValue::Bool(b) => t.eq_ignore_ascii_case(if *b { "true" } else { "false" }),
        FlatValue::I8(_) | FlatValue::I16(_) | FlatValue::I32(_) | FlatValue::I64(_) => num(t).parse::<i64>().is_ok_and(|x| x.to_string() == v.to_string()),
        FlatValue::U8(_) | FlatValue::U16(_) | FlatValue::U32(_) | FlatValue::U64(_) => num(t).parse::<u64>().is_ok_and(|x| x.to_string() == v.to_string()),
        FlatValue::Str(s) => s == t || (base == "CName" && t.is_empty() && s == "None"),
        FlatValue::Id(id) => {
            let esperado = match base {
                "TweakDBID" => match t.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok()) {
                    Some(h) => h,
                    None if t.is_empty() => 0,
                    None => bwms_primitives::tweak_db_id(t),
                },
                "CResource" => t.parse::<u64>().unwrap_or_else(|_| bwms_primitives::resource_path_hash(t)),
                _ => bwms_primitives::lockey_chave(t),
            };
            esperado == *id
        }
        FlatValue::Floats(_) if t.starts_with('(') => {
            let red = TWEAK_TYPES.iter().find(|(r, _)| *r == base).map_or(base, |(_, n)| *n);
            bwms_primitives::tweakxl::tupla_em_posicional(red, t).is_some_and(|p| bate(base, v, &p))
        }
        // O `.bin` guarda `Roll, Pitch, Yaw`; o texto é `pitch,yaw,roll`.
        FlatValue::Floats(fs) if base == "CEulerAngles" && fs.len() == 3 => bate("CVector3", &FlatValue::Floats(vec![fs[1], fs[2], fs[0]]), t),
        FlatValue::Floats(fs) => {
            let partes: Vec<&str> = t.split(',').collect();
            partes.len() == fs.len() && partes.iter().zip(fs).all(|(p, f)| num(p).parse::<f32>().is_ok_and(|x| (x - f).abs() <= 1e-4 * f.abs().max(1.0)))
        }
        FlatValue::Color(c) => t.split(',').map(|p| p.trim().parse::<u8>().ok()).collect::<Option<Vec<_>>>().is_some_and(|p| p == c),
        FlatValue::Array(itens) => texto_em_lista(t).is_some_and(|l| l.len() == itens.len() && l.iter().zip(itens).all(|(e, v)| bate(base, v, e))),
    }
}

/// O tweak `.json` do carregador nativo (mod do mod.io) na árvore do intérprete: a semântica é a do YAML do TweakXL escrita em JSON.
fn no_do_json(v: &bwms_primitives::json::Valor) -> bwms_primitives::tweakxl::yaml::Node {
    use bwms_primitives::json::Valor;
    use bwms_primitives::tweakxl::yaml::{Kind, Node};
    let escalar = |s: String, aspas: bool| Node { aspas, tag: None, kind: Kind::Scalar(s) };
    match v {
        Valor::Nulo => escalar(String::new(), false),
        Valor::Bool(b) => escalar(b.to_string(), false),
        Valor::Numero(n) => escalar(n.clone(), false),
        Valor::Texto(t) => escalar(t.clone(), true),
        Valor::Lista(l) => Node { aspas: false, tag: None, kind: Kind::Seq(l.iter().map(no_do_json).collect()) },
        Valor::Objeto(o) => Node { aspas: false, tag: None, kind: Kind::Map(o.iter().map(|(k, v)| (k.clone(), no_do_json(v))).collect()) },
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 2 {
        return eprintln!("uso: oraculo_cdpr <pasta-tweaks> <tweakdb_ep1.bin> [tweakdb.bin] [saida]");
    }
    let mut mapa = HashMap::new();
    for bin in a[1..].iter().take(2).filter(|b| b.ends_with(".bin")) {
        let db = TweakDb::open(Path::new(bin)).unwrap_or_else(|e| panic!("{bin}: {e}"));
        flats(&db, &mut mapa);
    }
    // Flat extra (`ORACULO_EXTRA=prop1,prop2`): records vanilla que têm `<record>.<prop>` e o tipo dele, contados por tipo e classe.
    if let Ok(props) = std::env::var("ORACULO_EXTRA") {
        let mut records = Vec::new();
        for bin in a[1..].iter().take(2).filter(|b| b.ends_with(".bin")) {
            records.extend(TweakDb::open(Path::new(bin)).map(|db| db.records.iter().map(|r| (r.id, r.type_key)).collect::<Vec<_>>()).unwrap_or_default());
        }
        for prop in props.split(',') {
            let mut tipos: BTreeMap<(String, u32), usize> = BTreeMap::new();
            for (rid, chave_tipo) in &records {
                let fid = bwms_primitives::tweak_db_id_derive(bwms_primitives::tweak_db_id_chave(*rid), &format!(".{prop}"));
                if let Some((rotulo, _)) = mapa.get(&fid) {
                    *tipos.entry((rotulo.clone(), *chave_tipo)).or_default() += 1;
                }
            }
            println!("{prop}: {tipos:?}");
        }
        return;
    }
    // Mod real (`ORACULO_MOD=<arquivo>`): bases e records que o arquivo edita existem no `.bin`? E as atribuições escalares (sem record em
    // linha) que mudam o vanilla (`MUDA`) ou caem em record novo (`NOVO`), para conferir no jogo.
    if let Ok(arq) = std::env::var("ORACULO_MOD") {
        use bwms_primitives::tweakxl::{interpret_from, template, tweak_source, yaml};
        let texto = std::fs::read_to_string(&arq).unwrap_or_else(|e| panic!("{arq}: {e}"));
        let ops = if arq.ends_with(".tweak") {
            tweak_source::parse(&texto)
        } else if arq.ends_with(".json") {
            bwms_primitives::json::ler(texto.strip_prefix('\u{feff}').unwrap_or(&texto)).map(|v| no_do_json(&v)).and_then(|r| interpret_from(&r, &arq))
        } else {
            yaml::parse(&texto).and_then(|r| template::expand(&r)).and_then(|r| interpret_from(&r, &arq))
        }
        .unwrap_or_else(|e| panic!("{arq}: {e}"));
        let mut existem = std::collections::HashSet::new();
        for bin in a[1..].iter().take(2).filter(|b| b.ends_with(".bin")) {
            existem.extend(TweakDb::open(Path::new(bin)).map(|db| db.records.iter().map(|r| r.id).collect::<Vec<_>>()).unwrap_or_default());
        }
        let id = bwms_primitives::tweak_db_id;
        let mut finais: BTreeMap<String, Vec<(u64, String)>> = BTreeMap::new();
        let criados: std::collections::HashSet<&str> =
            ops.iter().filter_map(|o| if let Op::Create { record, .. } | Op::Clone { record, .. } = o { Some(record.as_str()) } else { None }).collect();
        for o in &ops {
            match o {
                Op::Clone { base, .. } if !base.starts_with('?') && !criados.contains(base.as_str()) && !existem.contains(&id(base)) => {
                    println!("FALTA base {base}")
                }
                Op::Edit { flat, op } | Op::Criar { flat, op, .. } => {
                    let Some((rec, _)) = flat.rsplit_once('.') else { continue };
                    let novo = criados.contains(rec);
                    if !novo && !rec.contains('$') && !existem.contains(&id(rec)) && !mapa.contains_key(&id(flat)) {
                        println!("FALTA record {rec} ({flat})");
                        continue;
                    }
                    // Anexo em array vivo: a lista final (o vanilla + os anexados, o `-once` sem repetir id) dá o tamanho e o último.
                    if let (EditOp::Append(v) | EditOp::AppendOnce(v), Some((_, FlatValue::Array(itens)))) = (op, mapa.get(&id(flat))) {
                        let lista = finais.entry(flat.clone()).or_insert_with(|| {
                            itens.iter().filter_map(|i| if let FlatValue::Id(x) = i { Some((bwms_primitives::tweak_db_id_chave(*x), String::new())) } else { None }).collect()
                        });
                        let novo_id = id(v);
                        if !(matches!(op, EditOp::AppendOnce(_)) && lista.iter().any(|(x, _)| *x == novo_id)) {
                            lista.push((novo_id, v.clone()));
                        }
                        continue;
                    }
                    let EditOp::Assign(v) = op else { continue };
                    if flat.contains('$') || v.contains('$') {
                        continue;
                    }
                    match mapa.get(&id(flat)) {
                        _ if novo => println!("NOVO\t{flat}\t{v}"),
                        Some((rotulo, atual)) if !bate(rotulo.strip_prefix("array:").unwrap_or(rotulo), atual, v) => println!("MUDA\t{flat}\t{v}"),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        for (flat, lista) in &finais {
            let ultimo = lista.last().map_or("", |(_, n)| n.as_str());
            println!("LISTA\t{flat}\t{}\túltimo [{}] {ultimo}", lista.len(), lista.len().saturating_sub(1));
        }
        return;
    }
    // Sonda: os ids de um flat do `.bin` e qual `<record>_inline<k>` (k < 2000) cada um é.
    if let Ok(flat) = std::env::var("ORACULO_SONDA") {
        let record = flat.rsplit_once('.').map_or(flat.as_str(), |(r, _)| r);
        let ids = match mapa.get(&bwms_primitives::tweak_db_id(&flat)) {
            Some((_, FlatValue::Array(itens))) => itens.iter().filter_map(|v| if let FlatValue::Id(i) = v { Some(*i) } else { None }).collect(),
            Some((_, FlatValue::Id(i))) => vec![*i],
            _ => vec![],
        };
        for id in ids {
            let k = (0..2000).find(|k| bwms_primitives::tweak_db_id(&format!("{record}_inline{k}")) == id);
            println!("{id:016x} -> {}", k.map_or("?".into(), |k| format!("{record}_inline{k}")));
        }
        let mut existem = std::collections::HashSet::new();
        for bin in a[1..].iter().take(2).filter(|b| b.ends_with(".bin")) {
            existem.extend(TweakDb::open(Path::new(bin)).map(|db| db.records.iter().map(|r| r.id).collect::<Vec<_>>()).unwrap_or_default());
        }
        let ks: Vec<usize> = (0..2000).filter(|k| existem.contains(&bwms_primitives::tweak_db_id(&format!("{record}_inline{k}")))).collect();
        println!("records {record}_inline<k> no .bin: {} ({:?}…)", ks.len(), &ks[..ks.len().min(80)]);
        return;
    }
    let saida = a.get(3).or_else(|| a.get(2).filter(|s| !s.ends_with(".bin")));

    // Base antes do ep1: a última atribuição do mesmo flat vale, como no compilador.
    let mut lista = Vec::new();
    arquivos(Path::new(&a[0]), &mut lista);
    lista.sort_by_key(|p| p.to_string_lossy().contains("/ep1/"));
    let mut ultima: HashMap<String, (String, String)> = HashMap::new();
    let mut anexos: HashMap<String, Vec<String>> = HashMap::new();
    let (mut erros, mut outras) = (0, 0);
    let mut fontes = Vec::new();
    for p in &lista {
        let Ok(texto) = std::fs::read_to_string(p) else { continue };
        match bwms_primitives::tweakxl::tweak_source::parse(&texto) {
            Ok(ops) => fontes.push((p, ops)),
            Err(_) => erros += 1,
        }
    }

    for (p, ops) in fontes {
        for o in ops {
            match o {
                Op::Edit { flat, op: EditOp::Assign(v) } | Op::Criar { flat, op: EditOp::Assign(v), .. } => {
                    ultima.insert(flat, (v, p.display().to_string()));
                }
                Op::Edit { flat, op: EditOp::Append(v) } | Op::Criar { flat, op: EditOp::Append(v), .. } => {
                    anexos.entry(flat).or_default().push(v);
                }
                Op::Edit { .. } | Op::Criar { .. } => outras += 1,
                _ => {}
            }
        }
    }

    // Record em linha: a CDPR o chama `<record>_inline<N>` numa ordem própria; o intérprete, `pai$hash` (o `ComposeInlineName` do
    // TweakXL). O id real de cada um sai do valor do pai no `.bin`, na mesma posição do array, em passadas até nada mudar (o filho só
    // resolve depois do pai em linha).
    let mut em_linha: HashMap<String, u64> = HashMap::new();
    let id_do_flat = |flat: &str, em_linha: &HashMap<String, u64>| -> Option<u64> {
        let (r, prop) = flat.rsplit_once('.')?;
        let rid = if r.contains('$') { *em_linha.get(r)? } else { bwms_primitives::tweak_db_id(r) };
        Some(bwms_primitives::tweak_db_id_derive(bwms_primitives::tweak_db_id_chave(rid), &format!(".{prop}")))
    };
    loop {
        let mut novos = 0;
        for (flat, (t, _)) in &ultima {
            if !t.contains('$') {
                continue;
            }
            let Some((_, v)) = id_do_flat(flat, &em_linha).and_then(|id| mapa.get(&id)) else { continue };
            let pares: Vec<(String, u64)> = match (texto_em_lista(t), v) {
                (Some(l), FlatValue::Array(itens)) if l.len() == itens.len() => l
                    .into_iter()
                    .zip(itens)
                    .filter_map(|(e, v)| match v {
                        FlatValue::Id(i) if e.contains('$') => Some((e, *i)),
                        _ => None,
                    })
                    .collect(),
                (None, FlatValue::Id(i)) => vec![(t.clone(), *i)],
                _ => vec![],
            };
            for (nome, id) in pares {
                if !em_linha.contains_key(&nome) {
                    em_linha.insert(nome, id);
                    novos += 1;
                }
            }
        }
        // `+=`: os k anexados são os k últimos do array no `.bin`, na ordem.
        for (flat, vs) in &anexos {
            let Some((_, FlatValue::Array(itens))) = id_do_flat(flat, &em_linha).and_then(|id| mapa.get(&id)) else { continue };
            let Some(cauda) = itens.len().checked_sub(vs.len()).map(|i| &itens[i..]) else { continue };
            for (nome, v) in vs.iter().zip(cauda) {
                if let (true, FlatValue::Id(i)) = (nome.contains('$'), v) {
                    if !em_linha.contains_key(nome) {
                        em_linha.insert(nome.clone(), *i);
                        novos += 1;
                    }
                }
            }
        }
        if novos == 0 {
            break;
        }
    }
    let em_hex = |e: &str| em_linha.get(e).map_or_else(|| e.to_string(), |i| format!("0x{i:x}"));

    // (ok, diferente, ausente no .bin, valor com record em linha) por tipo.
    let mut placar: BTreeMap<String, [usize; 4]> = BTreeMap::new();
    let mut difs = Vec::new();
    for (flat, (t, origem)) in &ultima {
        let Some(fid) = id_do_flat(flat, &em_linha) else {
            placar.entry("(em linha sem pai)".into()).or_default()[2] += 1;
            continue;
        };
        let em_linha_no_valor = t.contains('$');
        let t = &match texto_em_lista(t) {
            Some(l) if em_linha_no_valor => bwms_primitives::operacoes::lista_em_texto(&l.iter().map(|e| em_hex(e)).collect::<Vec<_>>()),
            _ => em_hex(t),
        };
        let Some((rotulo, v)) = mapa.get(&fid) else {
            placar.entry("(ausente)".into()).or_default()[2] += 1;
            difs.push(format!("(ausente)\t{flat}\t\ttexto={t}\t\t{origem}"));
            continue;
        };
        if em_linha_no_valor {
            placar.entry(rotulo.clone()).or_default()[3] += 1;
        }
        let base = rotulo.strip_prefix("array:").unwrap_or(rotulo);
        // Fora do alvo, contados à parte: LocKey sem `LocKey#` (CDPR dá 0; TweakXL `.tweak` dá 0 no vazio e recusa o resto; o
        // `lockey_chave` dá FNV, TODO F1-30) e inteiro escrito como float (`1.`, o TweakXL recusa).
        let classe = match (base, v) {
            ("LocKey", FlatValue::Id(0)) if !t.starts_with("LocKey#") => Some("(LocKey sem prefixo)"),
            (_, FlatValue::I32(_) | FlatValue::I64(_) | FlatValue::I16(_) | FlatValue::I8(_)) if num(t).ends_with('.') => Some("(int como float)"),
            _ => None,
        };
        let linha = placar.entry(classe.map_or_else(|| rotulo.clone(), str::to_string)).or_default();
        if classe.is_some() {
            linha[0] += 1;
            continue;
        }
        debug_assert!(TWEAK_TYPES.iter().any(|(r, _)| *r == base));
        if bate(base, v, t) {
            linha[0] += 1;
        } else {
            linha[1] += 1;
            // Array: tamanho diferente (outra versão ou mutação de outro arquivo) ou o 1º elemento que difere.
            let onde = match (v, texto_em_lista(t)) {
                (FlatValue::Array(itens), Some(l)) if l.len() != itens.len() => format!("tamanho {} x {}", l.len(), itens.len()),
                (FlatValue::Array(itens), Some(l)) => l
                    .iter()
                    .zip(itens)
                    .position(|(e, v)| !bate(base, v, e))
                    .map_or(String::new(), |i| format!("elemento {i}: {} x {}", l[i], itens[i])),
                _ => String::new(),
            };
            difs.push(format!("{rotulo}\t{flat}\t{onde}\ttexto={t}\tbin={v}\t{origem}"));
        }
    }
    println!("arquivos {} (parse com erro {erros}); atribuições {}; outras mutações {outras}", lista.len(), ultima.len());
    println!("{:<22} {:>8} {:>8} {:>8} {:>8}", "tipo", "bate", "difere", "ausente", "em linha");
    for (r, [o, d, au, em]) in &placar {
        println!("{r:<22} {o:>8} {d:>8} {au:>8} {em:>8}");
    }
    difs.sort();
    if let Some(s) = saida {
        std::fs::write(s, difs.join("\n")).unwrap_or_else(|e| panic!("{s}: {e}"));
        println!("diferenças: {} em {s}", difs.len());
    }
}
