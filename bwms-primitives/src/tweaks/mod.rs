//! Interpretador do formato declarativo de tweak (YAML/TOML → mesma AST) →
//! a lista canônica de operações ([`crate::operacoes`]): atribuição
//! de flats (escalar/array/struct), `$base` (herança = clone + overrides), `$type` (record do zero, ou flat novo tipado com `$value`),
//! `$props: AutoFlats` (prop que o record não tem vira flat novo, com o tipo deduzido do valor), as
//! tags de array (`!append`/`!prepend`/`!append-once`/`!prepend-once`/`!remove`/`!remove-all`),
//! **records inline** (mapa com `$base`/`$type` dentro de um flat foreign-key) e
//! **`!append-from`/`!merge`/`!prepend-from`** (mesclar arrays de outro record).

pub mod registro;
pub mod template;
pub mod toml;
pub mod tweak_source;
pub mod yaml;

use std::collections::HashMap;

use crate::operacoes::{lista_em_texto, EditOp, Op};
use crate::tweaks::yaml::{Kind, Node};

/// Versão do jogo contra a qual o `$game` é avaliado quando o chamador não lê a do jogo instalado (offline, testes): a da 2.5 do Mac
/// (`CFBundleShortVersionString` 2.5.0 do `Cyberpunk2077.app`). A versão é a de PRODUTO do executável (major, minor, patch); no Mac
/// sai do `CFBundleShortVersionString`. Na 2.31 a de produto era 2.31; o runtime lê a do
/// `Info.plist` ([`versao_do_info_plist`]).
pub const GAME_VERSION: (u32, u32, u32) = (2, 5, 0);

/// A versão `major.minor.patch` do `CFBundleShortVersionString` de um `Info.plist` em XML (`%d.%d.%d`,
/// componente ausente = 0). `None` sem a chave ou sem número.
pub fn versao_do_info_plist(texto: &str) -> Option<(u32, u32, u32)> {
    let depois = &texto[texto.find("<key>CFBundleShortVersionString</key>")?..];
    let valor = depois.split("<string>").nth(1)?.split("</string>").next()?.trim();
    let mut partes = valor.split('.').map(|p| p.trim().parse::<u32>().ok());
    let major = partes.next()??;
    Some((major, partes.next().flatten().unwrap_or(0), partes.next().flatten().unwrap_or(0)))
}

/// Contexto da interpretação: a origem (caminho do arquivo, entra no nome dos records
/// inline) e
/// `is_ep1`/`game_version` — avaliação real de `$dlc: EP1`/`$game` (2026-08-05, achado de
/// auditoria: o código antigo IGNORAVA `$game`/`$dlc` mesmo em runtime, onde dá pra saber de
/// verdade — um mod gateado `$dlc: EP1` aplicava sempre, sem checar se Phantom Liberty está
/// instalado; um mod gateado `$game: ">=3.0"` aplicava numa versão do jogo que não suporta).
/// Regra: `$dlc` só vale
/// se `cond=="EP1"` E `IsEP1()` (native global real do jogo, `orphans.script:39008`) for
/// true, ou se `cond` for vazio; `$game` é um range semver (grade
/// NPM completa: `^`/`~`/`x`/hífen/`||`). Implementamos aqui o SUBCONJUNTO real de uso — operador
/// de comparação único (`>=`/`<=`/`>`/`<`/`=`) ou versão nua (match parcial por componente
/// especificado, ex.: `"2"` casa qualquer `2.x.x`, `"2.31"` casa qualquer `2.31.x`) — documentado
/// como simplificação deliberada (ver [`check_game_version`]), não a grade NPM inteira (`^`/`~`/
/// `x`/hífen/`||` nunca vistos em nenhum `.xl`/`.yaml` real do projeto).
/// `auto_flats`: o `$props: AutoFlats` vale no nó atual (do documento ou do record).
struct Ctx<'a> {
    source: &'a str,
    is_ep1: bool,
    game_version: (u32, u32, u32),
    auto_flats: bool,
    /// Os filhos em processamento são propriedades de um record (nomeado, em linha ou editado no topo): mapa vira record em linha.
    em_record: bool,
    /// O record pai não define `iconPath`: o `icon` em linha o preenche.
    pai_sem_iconpath: bool,
    /// Nomes de item de array em linha já dados neste arquivo ([`nome_de_record_em_linha`]).
    itens_em_linha: HashMap<String, u32>,
}

/// O nome do record em linha de `pai` (`.yaml` e `.tweak` dão o mesmo): `<pai>$<H>`, `H` = FNV-1a 32 em hexadecimal maiúsculo de
/// `<origem>|<pai>|<tipo>`, e no item de array (`indice` ≥ 0) mais `|<indice>|<n>`, com `n` contando os nomes da mesma chave já dados no
/// arquivo (`itens_em_linha`). O nome é o `TweakDBID` do record e pode estar em save: a fórmula não muda.
pub(crate) fn nome_de_record_em_linha(origem: &str, pai: &str, tipo: &str, indice: i32, itens_em_linha: &mut HashMap<String, u32>) -> String {
    let mut chave = format!("{origem}|{pai}|{tipo}");
    if indice >= 0 {
        chave.push_str(&format!("|{indice}|"));
        let n = itens_em_linha.entry(chave.clone()).or_insert(0);
        *n += 1;
        chave.push_str(&n.to_string());
    }
    format!("{pai}${:08X}", crate::fnv1a32(chave.as_bytes()))
}

/// Avalia uma condição `$game` (string do YAML) contra a versão do jogo. Suporta:
/// - operador de comparação + versão (`">=2.0"`, `"<=2.31.0"`, `">2.0"`, `"<3.0"`, `"=2.31"`):
///   compara o TRIPLET completo (componentes ausentes na versão-alvo viram 0).
/// - versão nua sem operador (`"2"`, `"2.31"`, `"2.31.0"`): match PARCIAL — só os componentes
///   informados precisam bater (`"2"` casa qualquer versão com major==2).
///
/// NÃO implementa a grade NPM completa de range semver (`^`/`~`/
/// `x`/hífen-range/`||` OR/espaço-AND) — nenhum `.xl`/`.yaml` do corpus deste projeto usa essas
/// formas, e sem exemplo real pra testar contra, implementar às cegas arriscaria parsear errado
/// silenciosamente (categoria de bug que este próprio fix existe pra evitar). Condição vazia ou
/// não-reconhecível (não bate nenhum padrão acima) passa sempre (`true`) — mesmo fallback seguro
/// que o projeto já usa pra `$dlc` com condição vazia.
fn check_game_version(cond: &str, game: (u32, u32, u32)) -> bool {
    let cond = cond.trim();
    if cond.is_empty() {
        return true;
    }
    for op in [">=", "<=", ">", "<", "="] {
        if let Some(rest) = cond.strip_prefix(op) {
            let target = parse_version_triplet(rest.trim());
            return match op {
                ">=" => game >= target,
                "<=" => game <= target,
                ">" => game > target,
                "<" => game < target,
                "=" => game == target,
                _ => unreachable!(),
            };
        }
    }
    // Versão nua: match parcial só nos componentes informados.
    let parts: Vec<&str> = cond.split('.').collect();
    if parts.is_empty() || parts.len() > 3 || !parts.iter().all(|p| p.parse::<u32>().is_ok()) {
        return true; // não reconhecível — mesmo fallback seguro do resto da função
    }
    let major: u32 = parts[0].parse().unwrap();
    if major != game.0 {
        return false;
    }
    if let Some(minor_s) = parts.get(1) {
        let minor: u32 = minor_s.parse().unwrap();
        if minor != game.1 {
            return false;
        }
        if let Some(patch_s) = parts.get(2) {
            let patch: u32 = patch_s.parse().unwrap();
            if patch != game.2 {
                return false;
            }
        }
    }
    true
}

fn parse_version_triplet(s: &str) -> (u32, u32, u32) {
    let mut parts = s.trim().splitn(3, '.');
    let major = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

/// Interpreta um documento já parseado em uma lista ordenada de ops. `$dlc: EP1` NUNCA passa
/// (sem contexto de instalação real) — use [`interpret_from_ctx`] pra avaliação real.
pub fn interpret(root: &Node) -> Result<Vec<Op>, String> {
    interpret_from(root, "")
}

/// Como [`interpret`], mas recebe a ORIGEM (caminho do arquivo) — entra no hash
/// do nome sintético dos records inline. `$dlc: EP1` sempre
/// passa (compat com chamadores antigos/offline, que não sabem se EP1 está instalado).
pub fn interpret_from(root: &Node, source: &str) -> Result<Vec<Op>, String> {
    interpret_from_ctx(root, source, true)
}

/// Como [`interpret_from`], mas com avaliação REAL de `$dlc: EP1` — `is_ep1` deve vir de
/// `IsEP1()` (native do jogo) quando chamado do runtime ao vivo (`cp77-console`); offline,
/// passar `true` preserva o comportamento antigo (sem contexto de instalação real). O `$game` vale contra [`GAME_VERSION`].
pub fn interpret_from_ctx(root: &Node, source: &str, is_ep1: bool) -> Result<Vec<Op>, String> {
    interpret_do_jogo(root, source, is_ep1, GAME_VERSION)
}

/// Como [`interpret_from_ctx`], com o `$game` avaliado contra `versao` (a do jogo instalado, [`versao_do_info_plist`]).
pub fn interpret_do_jogo(root: &Node, source: &str, is_ep1: bool, versao: (u32, u32, u32)) -> Result<Vec<Op>, String> {
    let Kind::Map(entries) = &root.kind else {
        return Err("o documento precisa ser um mapa de records/flats".into());
    };
    let mut ctx = Ctx {
        source,
        is_ep1,
        game_version: versao,
        auto_flats: false,
        em_record: false,
        pai_sem_iconpath: false,
        itens_em_linha: HashMap::new(),
    };
    let mut ops = Vec::new();
    // `$game`/`$dlc` do topo valem pro arquivo inteiro, e o `$props` do topo é o modo de todos os records.
    if !check_conditions(entries, &ctx) {
        return Ok(ops);
    }
    ctx.auto_flats = modo_auto(entries);
    for (key, node) in entries {
        // Chave de atributo no topo (`$props`, `$game`…) não é record nem flat.
        if key.starts_with('$') {
            continue;
        }
        process_entry(key, node, true, &mut ops, &mut ctx)?;
    }
    Ok(ops)
}

/// `$props: AutoFlats` no nó.
fn modo_auto(entries: &[(String, Node)]) -> bool {
    entries.iter().any(|(k, v)| k == "$props" && v.as_str() == Some("AutoFlats"))
}

/// Olha `$game`/`$dlc`
/// nas entradas do node e devolve `false` se algum falhar — quem chama deve pular o node INTEIRO
/// (record ou bloco de props), não só a chave da condição (bug antigo: só a chave era ignorada,
/// os irmãos continuavam sendo processados incondicionalmente).
fn check_conditions(entries: &[(String, Node)], ctx: &Ctx) -> bool {
    for (k, v) in entries {
        if k == "$dlc" {
            if let Kind::Scalar(cond) = &v.kind {
                let ok = if cond == "EP1" { ctx.is_ep1 } else { cond.is_empty() };
                if !ok {
                    return false;
                }
            }
        }
        if k == "$game" {
            if let Kind::Scalar(cond) = &v.kind {
                if !check_game_version(cond, ctx.game_version) {
                    return false;
                }
            }
        }
    }
    true
}

/// A edição de `path`: com o tipo deduzido do valor (`tipo` vindo de [`deduzir_tipo`]) o flat nasce se não existir; sem ele, só edita o que
/// existe.
fn edicao(path: &str, tipo: Option<String>, op: EditOp) -> Op {
    match tipo {
        Some(tipo) => Op::Criar { flat: path.to_string(), tipo, op },
        None => Op::Edit { flat: path.to_string(), op },
    }
}

/// `top` = flat solto no topo do arquivo (o que não existe nasce pelo tipo deduzido); prop de record só cria com `$props:
/// AutoFlats`.
fn process_entry(path: &str, node: &Node, top: bool, ops: &mut Vec<Op>, ctx: &mut Ctx) -> Result<(), String> {
    let cria = top || ctx.auto_flats;
    match &node.kind {
        Kind::Scalar(s) => {
            let deduzido = if cria { deduzir_tipo(node) } else { None };
            let op = match deduzido {
                Some((tipo, valor)) => Op::Criar { flat: path.to_string(), tipo, op: EditOp::Assign(valor) },
                None => Op::Edit { flat: path.to_string(), op: EditOp::Assign(s.clone()) },
            };
            ops.push(op);
            Ok(())
        }
        Kind::Seq(items) => emit_array_ops(path, items, cria, ops, ctx),
        Kind::Map(_) => process_map(path, node, top, ops, ctx),
    }
}

fn process_map(path: &str, node: &Node, top: bool, ops: &mut Vec<Op>, ctx: &mut Ctx) -> Result<(), String> {
    let Kind::Map(entries) = &node.kind else {
        return Ok(());
    };

    // Se `$game`/`$dlc` falhar, o node INTEIRO é pulado (record ou bloco
    // de props) — checado ANTES de qualquer ramo (record/scalar/aninhado).
    if !check_conditions(entries, ctx) {
        return Ok(());
    }

    // Flat novo tipado no topo (`$type` com `$value` é tipo de flat, sem `$value` é classe de record).
    if top {
        if let (Some(tipo), Some(valor)) = (node.get("$type"), node.get("$value")) {
            let tipo = tipo.as_str().ok_or_else(|| format!("'{path}': $type precisa ser um nome de tipo"))?.to_string();
            let texto = valor_tipado(&tipo, valor).ok_or_else(|| format!("'{path}': $value não serve pro tipo '{tipo}'"))?;
            ops.push(Op::Criar { flat: path.to_string(), tipo, op: EditOp::Assign(texto) });
            return Ok(());
        }
        if node.get("$base").is_none() && node.get("$type").is_none() {
            if let Some(v) = node.get("$value") {
                return process_entry(path, v, true, ops, ctx);
            }
        }
    }

    if node.get("$base").is_some() || node.get("$type").is_some() {
        if top {
            // Record nomeado: clone/create + overrides.
            return emit_record(path, node, ops, ctx);
        }
        // Record INLINE como valor de um flat: cria o record e aponta o flat
        // (foreign key) pro nome sintético dele.
        let name = make_inline_record(path, node, ops, ctx, -1)?;
        ops.push(Op::Edit { flat: path.to_string(), op: EditOp::Assign(name) });
        return Ok(());
    }

    // Sem $base/$type, mapa só com campos de struct é valor de struct: o completo vale em
    // qualquer contexto (no topo, flat novo do tipo dos campos); o parcial só onde o tipo já se sabe, no flat vivo, com os ausentes em 0.
    if node.get("$base").is_none() && node.get("$type").is_none() && node.get("$value").is_none() {
        if top {
            if let Some(tipo) = tipo_de_struct(entries).filter(|_| struct_map_to_scalar(entries).is_some()) {
                let valor = struct_texto(tipo, entries).ok_or_else(|| format!("'{path}': struct '{tipo}' com campo que não é escalar"))?;
                ops.push(Op::Criar { flat: path.to_string(), tipo: tipo.to_string(), op: EditOp::Assign(valor) });
                return Ok(());
            }
        }
        if struct_map_to_scalar(entries).is_none() {
            if let Some(nomeado) = struct_nomeado(entries) {
                ops.push(Op::Edit { flat: path.to_string(), op: EditOp::Assign(nomeado) });
                return Ok(());
            }
        }
    }

    // Sem $base/$type: pode ser um valor-struct, `{$value: X}` ou aninhado.
    if !top {
        if let Some(scalar) = struct_map_to_scalar(entries) {
            let tipo = if ctx.auto_flats { tipo_de_struct(entries).map(str::to_string) } else { None };
            ops.push(edicao(path, tipo, EditOp::Assign(scalar)));
            return Ok(());
        }
        if let Some(v) = node.get("$value") {
            let s = v
                .as_str()
                .ok_or_else(|| format!("'{path}': $value precisa ser escalar"))?;
            ops.push(Op::Edit { flat: path.to_string(), op: EditOp::Assign(s.to_string()) });
            return Ok(());
        }
    }

    // Mapa sob propriedade de record, sem `$type`/`$base`/`$value` e sem forma de struct.
    if !top && ctx.em_record && node.get("$value").is_none() {
        return record_em_linha_pela_propriedade(path, node, ops, ctx);
    }

    // Caminho aninhado: cada filho vira um flat `path.filho`. `$game`/`$dlc` já foram
    // AVALIADOS lá em cima (check_conditions) — aqui só pulamos as chaves em si (não são props). O mapa do topo é um record editado: o
    // mapa abaixo dele é record em linha.
    let (antes, antes_rec, antes_icone) = (ctx.auto_flats, ctx.em_record, ctx.pai_sem_iconpath);
    ctx.auto_flats = antes || modo_auto(entries);
    ctx.em_record = top;
    ctx.pai_sem_iconpath = !entries.iter().any(|(k, _)| k == "iconPath");
    let r = props(path, entries, ops, ctx);
    (ctx.auto_flats, ctx.em_record, ctx.pai_sem_iconpath) = (antes, antes_rec, antes_icone);
    r
}

fn props(path: &str, entries: &[(String, Node)], ops: &mut Vec<Op>, ctx: &mut Ctx) -> Result<(), String> {
    for (k, v) in entries {
        if k == "$game" || k == "$dlc" || k == "$props" {
            continue;
        }
        if k.starts_with('$') {
            return Err(format!("'{path}': chave '{k}' não suportada offline"));
        }
        process_entry(&format!("{path}.{k}"), v, false, ops, ctx)?;
    }
    Ok(())
}

/// Emite as ops de um record (nomeado OU inline): `$base`→Clone, `$type`→Create,
/// e cada prop não-`$` vira um flat `name.prop` (recursivo — props podem ser
/// elas mesmas records inline). O `$props` do record vale pras props dele e dos inline dentro.
fn emit_record(name: &str, node: &Node, ops: &mut Vec<Op>, ctx: &mut Ctx) -> Result<(), String> {
    if let Some(base) = node.get("$base") {
        let base = base
            .as_str()
            .ok_or_else(|| format!("'{name}': $base precisa ser um nome de record"))?;
        ops.push(Op::Clone { record: name.to_string(), base: base.to_string() });
    }
    if let Some(ty) = node.get("$type") {
        let class = ty
            .as_str()
            .ok_or_else(|| format!("'{name}': $type precisa ser um nome de classe"))?;
        ops.push(Op::Create { record: name.to_string(), class: class.to_string() });
    }
    if let Kind::Map(entries) = &node.kind {
        let (antes, antes_rec, antes_icone) = (ctx.auto_flats, ctx.em_record, ctx.pai_sem_iconpath);
        ctx.auto_flats = antes || modo_auto(entries);
        ctx.em_record = true;
        ctx.pai_sem_iconpath = !entries.iter().any(|(k, _)| k == "iconPath");
        let mut r = Ok(());
        for (k, v) in entries {
            if k.starts_with('$') {
                continue;
            }
            r = process_entry(&format!("{name}.{k}"), v, false, ops, ctx);
            if r.is_err() {
                break;
            }
        }
        (ctx.auto_flats, ctx.em_record, ctx.pai_sem_iconpath) = (antes, antes_rec, antes_icone);
        r?;
    }
    Ok(())
}

/// É um record inline? (mapa com `$base`/`$type`.)
fn is_inline_record(node: &Node) -> bool {
    matches!(node.kind, Kind::Map(_)) && (node.get("$base").is_some() || node.get("$type").is_some())
}

/// Cria um record inline e devolve o NOME sintético (foreign key) que substitui
/// o nó. `item_index` ≥ 0 para itens de array, −1 para um flat escalar.
fn make_inline_record(
    parent_flat: &str,
    node: &Node,
    ops: &mut Vec<Op>,
    ctx: &mut Ctx,
    item_index: i32,
) -> Result<String, String> {
    // O "tipo" que entra no hash do nome: a classe ($type) ou o record-base ($base).
    let type_for_hash = node
        .get("$type")
        .and_then(Node::as_str)
        .or_else(|| node.get("$base").and_then(Node::as_str))
        .ok_or_else(|| format!("'{parent_flat}': record inline precisa de $type ou $base"))?
        .to_string();
    let name = compose_inline_name(parent_flat, &type_for_hash, ctx, item_index);
    emit_record(&name, node, ops, ctx)?;
    Ok(name)
}

/// Nome de record inline do flat `parent_flat`: `record_type` é o `$type`, o `$base` ou `@` (tipo da propriedade); `item_index` ≥ 0 para
/// item de array, −1 para valor de flat.
fn compose_inline_name(parent_flat: &str, record_type: &str, ctx: &mut Ctx, item_index: i32) -> String {
    nome_de_record_em_linha(ctx.source, parent_flat, record_type, item_index, &mut ctx.itens_em_linha)
}

/// Mapa sem `$type`/`$base`/`$value`, sem forma de struct, sob propriedade `path` de record: record em linha do tipo da propriedade
/// (com o bloco `icon`).
fn record_em_linha_pela_propriedade(path: &str, node: &Node, ops: &mut Vec<Op>, ctx: &mut Ctx) -> Result<(), String> {
    let curto = compose_inline_name(path, "@", ctx, -1);
    let icone = path.rsplit('.').next() == Some("icon");
    // O ícone do item mora no pacote `UIIcon.` e o jogo monta o id dele como `UIIcon.` + `iconPath`. `icon` dentro de `icon` já nasce sob
    // `UIIcon.`: o prefixo entra uma vez só.
    let nome = if icone && !curto.starts_with("UIIcon.") { format!("UIIcon.{curto}") } else { curto };
    let pai_sem_iconpath = ctx.pai_sem_iconpath;
    ops.push(Op::Create { record: nome.clone(), class: format!("@{path}") });
    emit_record(&nome, node, ops, ctx)?;
    if let Some(record_pai) = path.strip_suffix(".icon").filter(|_| icone && pai_sem_iconpath) {
        let icon_path = nome.strip_prefix("UIIcon.").unwrap_or(&nome).to_string();
        ops.push(Op::Edit { flat: format!("{record_pai}.iconPath"), op: EditOp::Assign(icon_path) });
    }
    ops.push(Op::Edit { flat: path.to_string(), op: EditOp::Assign(nome) });
    Ok(())
}

/// O texto de um elemento de array: escalar como está, struct pela forma posicional.
fn texto_do_elemento(path: &str, it: &Node) -> Result<String, String> {
    match &it.kind {
        Kind::Scalar(s) => Ok(s.clone()),
        Kind::Map(entries) => struct_map_to_scalar(entries).ok_or_else(|| format!("'{path}': elemento de array precisa ser escalar ou struct")),
        Kind::Seq(_) => Err(format!("'{path}': elemento de array precisa ser escalar ou struct")),
    }
}

fn emit_array_ops(path: &str, items: &[Node], cria: bool, ops: &mut Vec<Op>, ctx: &mut Ctx) -> Result<(), String> {
    // 1) Resolve itens que são records inline (mapa com $base/$type) → cria o
    //    record e troca o item pelo nome sintético (foreign key), preservando a tag.
    let mut resolved: Vec<Node> = Vec::with_capacity(items.len());
    for (i, it) in items.iter().enumerate() {
        if is_inline_record(it) {
            let name = make_inline_record(path, it, ops, ctx, i as i32)?;
            resolved.push(Node { aspas: false, tag: it.tag.clone(), kind: Kind::Scalar(name) });
        } else if let (true, Kind::Map(entries)) = (ctx.em_record, &it.kind) {
            // Mapa sem `$type`/`$base` num array de propriedade de record (`itemStock: - !append {item: …, quantity: […]}`): vira
            // record em linha do `foreignType` do elemento; o struct (`{x: 1, y: 2}`) fica como valor.
            if struct_map_to_scalar(entries).is_some() || struct_nomeado(entries).is_some() || it.get("$value").is_some() {
                resolved.push(it.clone());
                continue;
            }
            let name = compose_inline_name(path, "@", ctx, i as i32);
            ops.push(Op::Create { record: name.clone(), class: format!("@{path}") });
            emit_record(&name, it, ops, ctx)?;
            resolved.push(Node { aspas: false, tag: it.tag.clone(), kind: Kind::Scalar(name) });
        } else {
            resolved.push(it.clone());
        }
    }

    // Dedução de sequência: o tipo do array sai do 1º elemento (a tag de operação não conta na dedução).
    let tipo = if cria { resolved.first().and_then(deduzir_tipo).map(|(t, _)| format!("array:{t}")) } else { None };
    let elem_tipo = tipo.as_deref().and_then(|t| t.strip_prefix("array:")).map(str::to_string);
    let valor = |it: &Node| -> Result<String, String> {
        match (&elem_tipo, &it.kind) {
            (Some(t), Kind::Scalar(s)) => Ok(normalizar_para(t, s)),
            _ => texto_do_elemento(path, it),
        }
    };

    // 2) Sem nenhuma tag → atribuição da lista inteira.
    let any_tagged = resolved.iter().any(|it| it.tag.is_some());
    if !any_tagged {
        let elems = resolved.iter().map(&valor).collect::<Result<Vec<_>, _>>()?;
        ops.push(edicao(path, tipo, EditOp::Assign(lista_em_texto(&elems))));
        return Ok(());
    }

    // 3) Lista de operações por-item (cada item tem tag).
    for it in &resolved {
        let tag = it
            .tag
            .as_deref()
            .ok_or_else(|| format!("'{path}': item sem tag numa lista de operações (!append etc.)"))?;
        // `!remove-all` IGNORA o valor do item — a
        // única tag desta família sem operando. As outras exigem um valor escalar.
        if tag == "!remove-all" {
            ops.push(edicao(path, tipo.clone(), EditOp::RemoveAll));
            continue;
        }
        let v = valor(it).map_err(|_| format!("'{path}': operação '{tag}' espera valor escalar (ou record inline)"))?;
        let op = match tag {
            "!append" => EditOp::Append(v),
            "!append-once" => EditOp::AppendOnce(v),
            "!prepend" => EditOp::Prepend(v),
            "!prepend-once" => EditOp::PrependOnce(v),
            "!remove" => EditOp::Remove(v),
            "!append-from" | "!merge" => EditOp::AppendFrom(v),
            "!prepend-from" => EditOp::PrependFrom(v),
            other => return Err(format!("'{path}': tag de array desconhecida '{other}'")),
        };
        ops.push(edicao(path, tipo.clone(), op));
    }
    Ok(())
}

/// Tira do texto o invólucro de tipo (`n"X"`, `CName("X")`, `t"X"`, `TweakDBID("X")`, `l"X"`, `r"X"`, `ResRef("X")`) para o
/// tipo do flat; `LocKey(N)` vira `LocKey#N` (chave primária), `ResRef(N)` vira `N` (o hash) e `<TDBID:CRC:LEN>` vira o id em
/// hexadecimal. O resto passa como está; o decimal puro num `LocKey` segue o [`crate::lockey_chave`] (decisão do dono, F1-30). É a regra
/// única dos formatos de tweak: o intérprete a usa no flat novo e o escritor do runtime na propriedade de record,
/// onde o tipo só se sabe no TweakDB vivo.
pub fn normalizar_para(tipo: &str, s: &str) -> String {
    let dentro = |pre: &str, suf: &str| s.strip_prefix(pre).and_then(|x| x.strip_suffix(suf)).map(str::to_string);
    let achado = match tipo {
        "CName" => dentro("n\"", "\"").or_else(|| dentro("CName(\"", "\")")),
        "TweakDBID" => dentro("t\"", "\"").or_else(|| dentro("TweakDBID(\"", "\")")).or_else(|| {
            let (crc, len) = dentro("<TDBID:", ">")?.split_once(':').map(|(c, l)| (c.to_string(), l.to_string()))?;
            let (crc, len) = (u32::from_str_radix(&crc, 16).ok()?, u8::from_str_radix(&len, 16).ok()?);
            Some(format!("{:#x}", u64::from(crc) | (u64::from(len) << 32)))
        }),
        "gamedataLocKeyWrapper" => dentro("l\"", "\"")
            .or_else(|| dentro("LocKey(\"", "\")"))
            .or_else(|| dentro("LocKey(", ")").map(|n| format!("LocKey#{n}"))),
        t if t.starts_with("raRef:") => dentro("r\"", "\"").or_else(|| dentro("ResRef(\"", "\")")).or_else(|| dentro("ResRef(", ")")),
        _ => None,
    };
    achado.unwrap_or_else(|| s.to_string())
}

/// `int` do yaml-cpp: decimal com sinal ou `0x` hexadecimal, na faixa do `Int32`.
fn e_int(s: &str) -> bool {
    let (neg, corpo) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let v = match corpo.strip_prefix("0x").or_else(|| corpo.strip_prefix("0X")) {
        Some(h) if !h.is_empty() => i64::from_str_radix(h, 16).ok(),
        _ if !corpo.is_empty() && corpo.bytes().all(|b| b.is_ascii_digit()) => corpo.parse::<i64>().ok(),
        _ => None,
    };
    v.map(|v| if neg { -v } else { v }).is_some_and(|v| i32::try_from(v).is_ok())
}

/// `float` do yaml-cpp: o número inteiro do texto (sem sufixo) ou `.inf`/`.nan`.
fn e_float(s: &str) -> bool {
    matches!(s, ".inf" | ".Inf" | ".INF" | "-.inf" | "-.Inf" | "-.INF" | "+.inf" | ".nan" | ".NaN" | ".NAN")
        || (s.bytes().any(|b| b.is_ascii_digit()) && s.bytes().all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b)) && s.parse::<f32>().is_ok())
}

/// `bool` do yaml-cpp.
fn e_bool(s: &str) -> bool {
    matches!(
        s,
        "y" | "Y" | "yes" | "Yes" | "YES" | "true" | "True" | "TRUE" | "on" | "On" | "ON" | "n" | "N" | "no" | "No" | "NO" | "false" | "False"
            | "FALSE" | "off" | "Off" | "OFF"
    )
}

/// O tipo de struct de um mapa com todas as chaves do tipo (conversão estrita, nesta ordem de tentativa).
fn tipo_de_struct(entries: &[(String, Node)]) -> Option<&'static str> {
    let tem = |ks: &[&str]| ks.iter().all(|k| entries.iter().any(|(e, _)| e == k));
    [
        ("Quaternion", &["i", "j", "k", "r"][..]),
        ("EulerAngles", &["roll", "pitch", "yaw"][..]),
        ("Vector3", &["x", "y", "z"][..]),
        ("Vector2", &["x", "y"][..]),
        ("Color", &["red", "green", "blue", "alpha"][..]),
    ]
    .into_iter()
    .find(|(_, ks)| tem(ks))
    .map(|(t, _)| t)
}

/// O tipo de flat deduzido de um valor sem tipo declarado (conversões estritas nesta ordem): texto
/// entre aspas é `String`; número inteiro `Int32`; número `Float`; booleano `Bool`; `n"X"` `CName`; `t"X"` `TweakDBID`; `l"X"`/`LocKey#`
/// `gamedataLocKeyWrapper`; `r"X"` `raRef:CResource`; mapa de struct o struct; sequência `array:` do tipo do 1º elemento. Devolve o tipo
/// e o valor sem o invólucro; `None` = ambíguo (palavra solta como `Items.X`), recusado.
pub fn deduzir_tipo(node: &Node) -> Option<(String, String)> {
    match &node.kind {
        Kind::Scalar(s) => {
            let tipo = if node.aspas {
                "String"
            } else if e_int(s) {
                "Int32"
            } else if e_float(s) {
                "Float"
            } else if e_bool(s) {
                "Bool"
            } else if (s.starts_with("n\"") && s.ends_with('"') && s.len() >= 3) || (s.starts_with("CName(\"") && s.ends_with("\")")) {
                "CName"
            } else if (s.starts_with("t\"") && s.ends_with('"') && s.len() >= 3)
                || (s.starts_with("TweakDBID(\"") && s.ends_with("\")"))
                || (s.starts_with("<TDBID:") && s.ends_with('>') && s.len() == 19)
            {
                "TweakDBID"
            } else if (s.starts_with("l\"") && s.ends_with('"') && s.len() >= 3) || (s.starts_with("LocKey(") && s.ends_with(')')) || s.starts_with("LocKey#") {
                "gamedataLocKeyWrapper"
            } else if (s.starts_with("r\"") && s.ends_with('"') && s.len() >= 3) || (s.starts_with("ResRef(") && s.ends_with(')')) {
                "raRef:CResource"
            } else {
                return None;
            };
            let valor = match tipo {
                "Bool" => if matches!(s.as_str(), "y" | "Y" | "yes" | "Yes" | "YES" | "true" | "True" | "TRUE" | "on" | "On" | "ON") { "true" } else { "false" }.to_string(),
                _ => normalizar_para(tipo, s),
            };
            Some((tipo.to_string(), valor))
        }
        Kind::Map(entries) => {
            let tipo = tipo_de_struct(entries)?;
            Some((tipo.to_string(), struct_texto(tipo, entries)?))
        }
        Kind::Seq(items) => {
            let (elem, _) = deduzir_tipo(items.first()?)?;
            let valores = items.iter().map(|it| match &it.kind {
                Kind::Scalar(s) => Some(normalizar_para(&elem, s)),
                Kind::Map(e) => struct_texto(tipo_de_struct(e)?, e),
                Kind::Seq(_) => None,
            });
            Some((format!("array:{elem}"), lista_em_texto(&valores.collect::<Option<Vec<_>>>()?)))
        }
    }
}

/// Texto de `$value` para o tipo declarado `tipo` (escalar ou `array:`).
fn valor_tipado(tipo: &str, node: &Node) -> Option<String> {
    match (tipo.strip_prefix("array:"), &node.kind) {
        (Some(elem), Kind::Seq(items)) => {
            let v: Option<Vec<String>> = items.iter().map(|it| valor_tipado(elem, it)).collect();
            Some(lista_em_texto(&v?))
        }
        (Some(_), _) => None,
        (None, Kind::Scalar(s)) => Some(normalizar_para(tipo, s)),
        (None, Kind::Map(entries)) => struct_texto(tipo, entries),
        (None, Kind::Seq(_)) => None,
    }
}

/// A forma posicional do struct `tipo` (a de [`struct_map_to_scalar`]); componente ausente vale 0, como na conversão não estrita.
fn struct_texto(tipo: &str, entries: &[(String, Node)]) -> Option<String> {
    let ordem: &[&str] = match tipo {
        "Vector2" => &["x", "y"],
        "Vector3" => &["x", "y", "z"],
        "Quaternion" => &["i", "j", "k", "r"],
        "EulerAngles" => &["pitch", "yaw", "roll"],
        "Color" => &["red", "green", "blue", "alpha"],
        _ => return None,
    };
    let partes: Vec<&str> = ordem
        .iter()
        .map(|k| entries.iter().find(|(e, _)| e == k).and_then(|(_, v)| v.as_str()).unwrap_or("0"))
        .collect();
    Some(partes.join(","))
}

/// Mapa com forma de struct conhecida (Quaternion/Vector/EulerAngles/Color) →
/// string posicional que o `encode_value` entende ("f,f,…" / "r,g,b,a"; EulerAngles em pitch,yaw,roll, a ordem do `tweakdb.bin`).
fn struct_map_to_scalar(entries: &[(String, Node)]) -> Option<String> {
    let val = |k: &str| -> Option<&str> {
        entries.iter().find(|(key, _)| key == k).and_then(|(_, v)| v.as_str())
    };
    let keys: std::collections::BTreeSet<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
    let order: &[&str] = if keys == set(&["x", "y"]) {
        &["x", "y"]
    } else if keys == set(&["x", "y", "z"]) {
        &["x", "y", "z"]
    } else if keys == set(&["i", "j", "k", "r"]) {
        &["i", "j", "k", "r"]
    } else if keys == set(&["pitch", "yaw", "roll"]) {
        &["pitch", "yaw", "roll"]
    } else if keys == set(&["red", "green", "blue", "alpha"]) {
        &["red", "green", "blue", "alpha"]
    } else {
        return None;
    };
    let parts: Option<Vec<&str>> = order.iter().map(|k| val(k)).collect();
    parts.map(|p| p.join(","))
}

/// Campos de cada struct de flat, na ordem posicional do [`struct_texto`].
const CAMPOS_DE_STRUCT: [(&str, &[&str]); 5] = [
    ("Vector2", &["x", "y"]),
    ("Vector3", &["x", "y", "z"]),
    ("Quaternion", &["i", "j", "k", "r"]),
    ("EulerAngles", &["pitch", "yaw", "roll"]),
    ("Color", &["red", "green", "blue", "alpha"]),
];

/// Struct parcial (`{ y: 6.0 }`): o texto nomeado `{y:6.0}` quando todas as chaves são campos de um mesmo struct e os valores são
/// escalares; o tipo só se sabe no flat vivo, onde o [`struct_em_posicional`] o converte.
fn struct_nomeado(entries: &[(String, Node)]) -> Option<String> {
    if entries.is_empty() || !CAMPOS_DE_STRUCT.iter().any(|(_, cs)| entries.iter().all(|(k, _)| cs.contains(&k.as_str()))) {
        return None;
    }
    let pares: Option<Vec<String>> = entries.iter().map(|(k, v)| v.as_str().map(|s| format!("{k}:{s}"))).collect();
    Some(format!("{{{}}}", pares?.join(",")))
}

/// O texto nomeado de um struct (`{y:6.0}`, do [`struct_nomeado`]) na forma posicional do `tipo` (`0,6.0` no `Vector2`), com os campos
/// ausentes em 0 como na conversão não estrita. `None` com campo que o tipo não tem ou texto que não é nomeado.
pub fn struct_em_posicional(tipo: &str, texto: &str) -> Option<String> {
    let miolo = texto.trim().strip_prefix('{')?.strip_suffix('}')?;
    let (_, campos) = CAMPOS_DE_STRUCT.iter().find(|(t, _)| *t == tipo)?;
    let mut valores = vec!["0"; campos.len()];
    for par in miolo.split(',').filter(|p| !p.trim().is_empty()) {
        let (k, v) = par.split_once(':')?;
        let i = campos.iter().position(|c| *c == k.trim())?;
        valores[i] = v.trim();
    }
    Some(valores.join(","))
}

/// A tupla `(a,b,c)` do `.tweak` (os campos na ordem de memória do struct, a do `tweakdb.bin`) no texto
/// posicional do `tipo`: só o `EulerAngles` muda, `(roll,pitch,yaw)` → `pitch,yaw,roll`. Sem tipo declarado no `.tweak` a tupla chega
/// assim até o flat vivo, que diz se é `Vector3` ou `EulerAngles`. `None` se o texto não é tupla.
pub fn tupla_em_posicional(tipo: &str, texto: &str) -> Option<String> {
    let miolo = texto.trim().strip_prefix('(')?.strip_suffix(')')?;
    let partes: Vec<&str> = miolo.split(',').map(str::trim).collect();
    Some(match (tipo, &partes[..]) {
        ("EulerAngles", [r, p, y]) => format!("{p},{y},{r}"),
        _ => partes.join(","),
    })
}

fn set(items: &[&'static str]) -> std::collections::BTreeSet<&'static str> {
    items.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tweaks::yaml;

    /// Resume cada Op em texto pra checagem (Op não deriva Debug/Eq).
    fn summarize(ops: &[Op]) -> Vec<String> {
        ops.iter()
            .map(|o| match o {
                Op::Clone { record, base } => format!("clone {record} <- {base}"),
                Op::Create { record, class } => format!("create {record} : {class}"),
                Op::Edit { flat, op } => format!("edit {flat} {}", editop(op)),
                Op::Criar { flat, tipo, op } => format!("novo {tipo} {flat} {}", editop(op)),
            })
            .collect()
    }
    fn editop(op: &EditOp) -> String {
        match op {
            EditOp::Assign(v) => format!("= {v}"),
            EditOp::Append(v) => format!("append {v}"),
            EditOp::AppendOnce(v) => format!("append-once {v}"),
            EditOp::Prepend(v) => format!("prepend {v}"),
            EditOp::PrependOnce(v) => format!("prepend-once {v}"),
            EditOp::Remove(v) => format!("remove {v}"),
            EditOp::RemoveAll => "remove-all".to_string(),
            EditOp::AppendFrom(v) => format!("append-from {v}"),
            EditOp::PrependFrom(v) => format!("prepend-from {v}"),
        }
    }
    fn run(src: &str) -> Vec<String> {
        let root = yaml::parse(src).unwrap();
        let root = crate::tweaks::template::expand(&root).unwrap();
        summarize(&interpret_from(&root, "test.yaml").unwrap())
    }

    /// Mapa sob propriedade de record sem `$type`/`$base` (wiki "Inline records"): record em linha do tipo estrangeiro da propriedade, que
    /// o runtime resolve pela marca `@`; `icon` ganha o prefixo `UIIcon.` e preenche o `iconPath` do pai (wiki "Item icons").
    #[test]
    fn record_em_linha_pelo_tipo_da_propriedade_e_icone() {
        let s = run("Vehicle.X:\n  $base: Vehicle.Y\n  vehicleUIData:\n    horsepower: 250.0\n");
        assert_eq!(s[0], "clone Vehicle.X <- Vehicle.Y");
        assert!(s[1].starts_with("create Vehicle.X.vehicleUIData$") && s[1].ends_with(" : @Vehicle.X.vehicleUIData"), "{s:?}");
        let nome = s[1]["create ".len()..s[1].find(" : ").unwrap()].to_string();
        assert_eq!(s[2], format!("edit {nome}.horsepower = 250.0"));
        assert_eq!(s[3], format!("edit Vehicle.X.vehicleUIData = {nome}"));
        let s = run("Items.X:\n  $base: Items.Y\n  icon:\n    atlasResourcePath: a\\b.inkatlas\n    atlasPartName: p\n");
        let nome = s[1]["create ".len()..s[1].find(" : ").unwrap()].to_string();
        assert!(nome.starts_with("UIIcon.Items.X.icon$"), "{s:?}");
        assert_eq!(s[4], format!("edit Items.X.iconPath = {}", &nome["UIIcon.".len()..]));
        assert_eq!(s[5], format!("edit Items.X.icon = {nome}"));
        // Com `iconPath` próprio no pai, o do mod fica.
        let s = run("Items.X:\n  $base: Items.Y\n  iconPath: meu\n  icon:\n    atlasPartName: p\n");
        assert_eq!(s.iter().filter(|l| l.starts_with("edit Items.X.iconPath")).count(), 1, "{s:?}");
    }

    /// `icon` dentro de `icon`: o record já nasce sob `UIIcon.`, o prefixo não dobra, e todo `iconPath` acha o seu record por `UIIcon.` +
    /// `iconPath` (a montagem do jogo).
    #[test]
    fn icone_dentro_de_icone_tem_o_prefixo_uma_vez() {
        let s = run("Items.X:\n  icon:\n    atlasPartName: p\n    icon:\n      atlasPartName: q\n");
        assert!(s.iter().all(|l| !l.contains("UIIcon.UIIcon.")), "{s:?}");
        let criados: Vec<&str> = s.iter().filter_map(|l| l.strip_prefix("create ")?.split(" : ").next()).collect();
        let icon_paths: Vec<&str> = s.iter().filter_map(|l| l.split_once(".iconPath = ").map(|(_, v)| v)).collect();
        assert_eq!(icon_paths.len(), 2, "{s:?}");
        for p in icon_paths {
            assert!(criados.contains(&format!("UIIcon.{p}").as_str()), "iconPath {p} sem record: {s:?}");
        }
    }

    /// Mapa sem `$type`/`$base` como item de array de record (o `itemStock` de vendedor do Wannabe Edgerunner): record em linha marcado
    /// `@` com a tag do item preservada; struct no array segue como valor.
    #[test]
    fn mapa_sem_tipo_em_array_de_record_vira_record_em_linha() {
        let s = run("Vendors.V:\n  itemStock:\n  - !append\n    item: Items.A\n    quantity: [ Vendors.Always_Present ]\n");
        assert!(s[0].starts_with("create Vendors.V.itemStock$") && s[0].ends_with(" : @Vendors.V.itemStock"), "{s:?}");
        let nome = s[0]["create ".len()..s[0].find(" : ").unwrap()].to_string();
        assert_eq!(s[1], format!("edit {nome}.item = Items.A"));
        assert_eq!(s[2], format!("edit {nome}.quantity = [Vendors.Always_Present]"));
        assert_eq!(s[3], format!("edit Vendors.V.itemStock append {nome}"));
        assert_eq!(s.len(), 4, "{s:?}");
        let s = run("Condition.X:\n  pontos:\n  - !append { x: 1.0, y: 2.0 }\n");
        assert_eq!(s, vec!["edit Condition.X.pontos append 1.0,2.0"]);
    }

    /// Struct: completo no topo cria o flat do tipo dos campos; parcial vira o texto nomeado, que o
    /// flat vivo põe na ordem com os ausentes em 0; completo dentro de record segue posicional.
    #[test]
    fn struct_completo_no_topo_e_parcial() {
        assert_eq!(run("Bwms.Fmt.vec: { x: 0.5, y: -0.8, z: 0.0 }\n"), vec!["novo Vector3 Bwms.Fmt.vec = 0.5,-0.8,0.0"]);
        assert_eq!(run("Bwms.Fmt.ang:\n  roll: 30.0\n  pitch: 90.0\n  yaw: 0.0\n"), vec!["novo EulerAngles Bwms.Fmt.ang = 90.0,0.0,30.0"]);
        assert_eq!(run("Bwms.Fmt.cor: { red: 51, green: 102, blue: 0, alpha: 255 }\n"), vec!["novo Color Bwms.Fmt.cor = 51,102,0,255"]);
        assert_eq!(run("Condition.X.distance: { y: 6.0 }\n"), vec!["edit Condition.X.distance = {y:6.0}"]);
        assert_eq!(run("Condition.X:\n  distance: { y: 6.0 }\n"), vec!["edit Condition.X.distance = {y:6.0}"]);
        assert_eq!(run("Condition.X:\n  distance: { x: -1.0, y: 6.0 }\n"), vec!["edit Condition.X.distance = -1.0,6.0"]);
        assert_eq!(struct_em_posicional("Vector2", "{y:6.0}").as_deref(), Some("0,6.0"));
        assert_eq!(struct_em_posicional("EulerAngles", "{yaw:150.0}").as_deref(), Some("0,150.0,0"));
        assert_eq!(struct_em_posicional("Quaternion", "{k:0.1,r:0.2}").as_deref(), Some("0,0,0.1,0.2"));
        assert_eq!(struct_em_posicional("Vector2", "{z:1.0}"), None);
        assert_eq!(struct_em_posicional("Vector2", "1.0,2.0"), None);
    }

    #[test]
    fn flat_simples_e_base() {
        let s = run("Items.A:\n  $base: Items.B\n  damage: 10\n");
        assert_eq!(s, vec!["clone Items.A <- Items.B", "edit Items.A.damage = 10"]);
    }

    #[test]
    fn tags_de_array_por_item() {
        let s = run("Items.A:\n  tags:\n    - !append X\n    - !remove Y\n    - !append-from Items.B.tags\n    - !merge Items.C.tags\n");
        assert_eq!(
            s,
            vec![
                "edit Items.A.tags append X",
                "edit Items.A.tags remove Y",
                "edit Items.A.tags append-from Items.B.tags",
                "edit Items.A.tags append-from Items.C.tags", // !merge === !append-from
            ]
        );
    }

    #[test]
    fn tag_remove_all_ignora_o_valor_do_item_e_nao_exige_um() {
        // `!remove-all` limpa o array SEM ler o valor do item — nenhum valor associado, diferente
        // de `!remove`. O parser não pode exigir escalar aqui (item #30).
        let s = run("Items.A:\n  tags:\n    - !remove-all\n");
        assert_eq!(s, vec!["edit Items.A.tags remove-all"]);
        // Mesmo com um valor presente no item (alguns mods escrevem `!remove-all ~` por hábito
        // de copiar a forma de `!remove`), o valor é ignorado — nunca deve dar erro de parse.
        let s2 = run("Items.A:\n  tags:\n    - !remove-all Y\n");
        assert_eq!(s2, vec!["edit Items.A.tags remove-all"]);
    }

    /// `.yaml` de tweak com âncora é aceito (decisão do dono, F7-23): o alias repete o valor, com a tag, em outro record.
    #[test]
    fn ancora_e_alias_no_yaml_dos_tweaks() {
        let s = run("Items.A:\n  $base: Items.Base\n  quality: &Q Quality.Rare\n  tags: &T\n    - !append Cool\nItems.B:\n  $base: Items.Base\n  quality: *Q\n  tags: *T\n");
        assert_eq!(
            s,
            vec![
                "clone Items.A <- Items.Base",
                "edit Items.A.quality = Quality.Rare",
                "edit Items.A.tags append Cool",
                "clone Items.B <- Items.Base",
                "edit Items.B.quality = Quality.Rare",
                "edit Items.B.tags append Cool",
            ]
        );
    }

    #[test]
    fn record_inline_em_array() {
        // Cria 2 records inline, sobrescreve flats e atribui o array com os nomes.
        let s = run(
            "Items.A:\n  stats:\n    - $type: gamedataStat_Record\n      value: 1\n    - $type: gamedataStat_Record\n      value: 2\n",
        );
        // 2x (create + edit value) + 1 assign do array
        assert_eq!(s.len(), 5, "ops: {s:?}");
        assert!(s[0].starts_with("create Items.A.stats$"));
        assert!(s[0].ends_with(": gamedataStat_Record"));
        assert!(s[1].starts_with("edit Items.A.stats$") && s[1].ends_with(".value = 1"));
        assert!(s[2].starts_with("create Items.A.stats$"));
        assert!(s[3].ends_with(".value = 2"));
        // o último é o assign do array com os 2 nomes sintéticos
        assert!(s[4].starts_with("edit Items.A.stats = [Items.A.stats$"));
        // nomes únicos (índices diferentes no hash)
        assert_ne!(&s[0], &s[2]);
    }

    #[test]
    fn record_inline_como_flat_escalar() {
        let s = run("Items.A:\n  icon:\n    $type: gamedataUIIcon_Record\n    atlas: foo\n");
        assert_eq!(s.len(), 3, "ops: {s:?}");
        assert!(s[0].starts_with("create Items.A.icon$"));
        assert!(s[1].starts_with("edit Items.A.icon$") && s[1].ends_with(".atlas = foo"));
        assert!(s[2].starts_with("edit Items.A.icon = Items.A.icon$"));
    }

    #[test]
    fn nome_inline_e_deterministico() {
        // Mesma origem + mesma estrutura ⇒ mesmo nome sintético (reprodutível).
        let a = run("Items.A:\n  ref:\n    $type: gamedataX_Record\n    v: 1\n");
        let b = run("Items.A:\n  ref:\n    $type: gamedataX_Record\n    v: 1\n");
        assert_eq!(a, b);
    }

    // --- $game (2026-08-05, achado de auditoria: $game era sempre ignorado mesmo em runtime) ---

    #[test]
    fn game_version_operador_comparacao() {
        let g = (2, 31, 0);
        assert!(check_game_version(">=2.0", g));
        assert!(check_game_version(">=2.31.0", g));
        assert!(!check_game_version(">=3.0", g));
        assert!(check_game_version("<=2.31.0", g));
        assert!(!check_game_version("<=2.30", g));
        assert!(check_game_version(">2.0", g));
        assert!(!check_game_version(">2.31.0", g)); // igual não é maior
        assert!(check_game_version("<3.0", g));
        assert!(!check_game_version("<2.31", g));
        assert!(check_game_version("=2.31.0", g));
        assert!(!check_game_version("=2.30.0", g));
    }

    #[test]
    fn game_version_nua_match_parcial() {
        let g = (2, 31, 0);
        assert!(check_game_version("2", g)); // só major
        assert!(!check_game_version("1", g));
        assert!(check_game_version("2.31", g)); // major+minor
        assert!(!check_game_version("2.30", g));
        assert!(check_game_version("2.31.0", g)); // exato
        assert!(!check_game_version("2.31.1", g));
    }

    #[test]
    fn game_version_vazia_ou_nao_reconhecivel_passa_sempre() {
        let g = GAME_VERSION;
        assert!(check_game_version("", g));
        assert!(check_game_version("latest", g)); // grade NPM não suportada, fallback seguro
    }

    #[test]
    fn dlc_e_game_bloqueiam_o_node_inteiro() {
        // $dlc: EP1 sem EP1 instalado (is_ep1=false) -> node inteiro pulado, zero ops.
        let root = yaml::parse("Items.A:\n  $dlc: EP1\n  damage: 10\n").unwrap();
        let root = crate::tweaks::template::expand(&root).unwrap();
        let ops = interpret_from_ctx(&root, "test.yaml", false).unwrap();
        assert!(summarize(&ops).is_empty(), "ops: {:?}", summarize(&ops));

        // $dlc: EP1 com EP1 instalado (is_ep1=true) -> passa.
        let ops2 = interpret_from_ctx(&root, "test.yaml", true).unwrap();
        assert_eq!(summarize(&ops2), vec!["edit Items.A.damage = 10"]);

        // $game fora do range da versão suportada -> node inteiro pulado.
        let root3 = yaml::parse("Items.A:\n  $game: \">=3.0\"\n  damage: 10\n").unwrap();
        let root3 = crate::tweaks::template::expand(&root3).unwrap();
        let ops3 = interpret_from_ctx(&root3, "test.yaml", true).unwrap();
        assert!(summarize(&ops3).is_empty(), "ops: {:?}", summarize(&ops3));

        // $game dentro do range -> passa.
        let root4 = yaml::parse("Items.A:\n  $game: \">=2.0\"\n  damage: 10\n").unwrap();
        let root4 = crate::tweaks::template::expand(&root4).unwrap();
        let ops4 = interpret_from_ctx(&root4, "test.yaml", true).unwrap();
        assert_eq!(summarize(&ops4), vec!["edit Items.A.damage = 10"]);
    }

    /// A 2.5 do Mac é 2.5.0 (`CFBundleShortVersionString`): o `$game` responde como um range semver com a versão de
    /// produto 2.5.0 — `2.5` tem minor 5, menor que o 31 da 2.31.
    #[test]
    fn game_version_na_2_5() {
        let g = GAME_VERSION;
        assert_eq!(g, (2, 5, 0));
        assert!(check_game_version(">=2.3", g));
        assert!(!check_game_version(">=2.31", g));
        assert!(!check_game_version("<2.5", g));
        assert!(check_game_version("2.5", g));
        let plist = "<dict>\n\t<key>CFBundleShortVersionString</key>\n\t<string>2.5.0</string>\n\t<key>CFBundleVersion</key>\n\t<string>5814786</string>\n</dict>";
        assert_eq!(versao_do_info_plist(plist), Some((2, 5, 0)));
        assert_eq!(versao_do_info_plist("<key>CFBundleShortVersionString</key><string>2.31</string>"), Some((2, 31, 0)));
        assert_eq!(versao_do_info_plist("<key>CFBundleVersion</key><string>1</string>"), None);
        // O `$game` do arquivo contra a versão passada pelo runtime.
        let raiz = yaml::parse("Items.A:\n  $game: \">=2.31\"\n  damage: 10\n").unwrap();
        assert!(interpret_do_jogo(&raiz, "t.yaml", true, (2, 5, 0)).unwrap().is_empty());
        assert_eq!(interpret_do_jogo(&raiz, "t.yaml", true, (2, 31, 0)).unwrap().len(), 1);
    }

    fn rodar(src: &str) -> Vec<String> {
        summarize(&interpret_from(&yaml::parse(src).unwrap(), "t.yaml").unwrap())
    }

    /// Diferença 1: flat novo vindo de arquivo — `$type` com `$value` no topo é flat tipado; flat solto no topo nasce com o tipo deduzido do
    /// valor; prop de record sem `$props: AutoFlats` só edita o que existe.
    #[test]
    fn flat_novo_por_type_e_value_e_por_valor_no_topo() {
        assert_eq!(rodar("Bwms.Teste.inteiro:\n  $type: Int32\n  $value: 7\n"), ["novo Int32 Bwms.Teste.inteiro = 7"]);
        assert_eq!(
            rodar("Bwms.Teste.lista:\n  $type: array:CName\n  $value: [n\"A\", B]\n"),
            ["novo array:CName Bwms.Teste.lista = [A, B]"]
        );
        assert_eq!(rodar("Bwms.Teste.escalar: 1.5\n"), ["novo Float Bwms.Teste.escalar = 1.5"]);
        assert_eq!(rodar("Bwms.Teste.texto: \"oi, mundo\"\n"), ["novo String Bwms.Teste.texto = oi, mundo"]);
        assert_eq!(rodar("Bwms.Teste.id: t\"Items.X\"\n"), ["novo TweakDBID Bwms.Teste.id = Items.X"]);
        // Palavra solta é ambígua: só edita flat que já existe (não cria).
        assert_eq!(rodar("Bwms.Teste.ambiguo: Items.X\n"), ["edit Bwms.Teste.ambiguo = Items.X"]);
        // Atributo no topo não vira flat.
        assert_eq!(rodar("$game: \">=2.0\"\nItems.A.v: 2\n"), ["novo Int32 Items.A.v = 2"]);
        assert!(rodar("$game: \">=3.0\"\nItems.A.v: 2\n").is_empty(), "$game do topo vale pro arquivo");
        // Prop de record: sem AutoFlats, edita.
        assert_eq!(rodar("Items.A:\n  novaProp: 5\n"), ["edit Items.A.novaProp = 5"]);
    }

    /// Diferença 4: `$props: AutoFlats` (no record ou no topo) deixa a prop que o record não tem nascer com o tipo do valor.
    #[test]
    fn autoflats_cria_a_prop_com_o_tipo_do_valor() {
        assert_eq!(
            rodar("Items.A:\n  $props: AutoFlats\n  bwmsNivel: 3\n  bwmsPeso: 0.5\n  bwmsNome: \"Faca\"\n  bwmsTags: [n\"X\", n\"Y\"]\n  bwmsMira: {x: 1, y: 2, z: 3}\n  bwmsAmbiguo: Items.Y\n"),
            [
                "novo Int32 Items.A.bwmsNivel = 3",
                "novo Float Items.A.bwmsPeso = 0.5",
                "novo String Items.A.bwmsNome = Faca",
                "novo array:CName Items.A.bwmsTags = [X, Y]",
                "novo Vector3 Items.A.bwmsMira = 1,2,3",
                "edit Items.A.bwmsAmbiguo = Items.Y",
            ]
        );
        assert_eq!(
            rodar("$props: AutoFlats\nItems.B:\n  $base: Items.A\n  bwmsLigado: true\n"),
            ["clone Items.B <- Items.A", "novo Bool Items.B.bwmsLigado = true"]
        );
        // Mutação num array novo: o tipo sai do 1º item.
        assert_eq!(rodar("Items.A:\n  $props: AutoFlats\n  bwmsLista:\n    - !append 4\n"), ["novo array:Int32 Items.A.bwmsLista append 4"]);
        // O modo não vaza pro record seguinte.
        assert_eq!(rodar("Items.A:\n  $props: AutoFlats\n  p: 1\nItems.B:\n  p: 1\n"), ["novo Int32 Items.A.p = 1", "edit Items.B.p = 1"]);
    }

    /// Diferença 2: elemento de struct em array vai na lista com aspas e volta inteiro.
    #[test]
    fn array_de_struct_volta_inteiro() {
        let s = rodar("Items.A:\n  pontos:\n    - {x: 1, y: 2, z: 3}\n    - {x: 4, y: 5, z: 6}\n");
        assert_eq!(s, ["edit Items.A.pontos = [\"1,2,3\", \"4,5,6\"]"]);
        assert_eq!(crate::operacoes::texto_em_lista("[\"1,2,3\", \"4,5,6\"]").unwrap(), ["1,2,3", "4,5,6"]);
    }
}
