//! Interpretador do formato declarativo do TweakXL (YAML/TOML → mesma AST) →
//! operações no [`Model`]. Cobre o subconjunto offline de maior valor: atribuição
//! de flats (escalar/array/struct), `$base` (herança = clone + overrides), as
//! tags de array (`!append`/`!prepend`/`!append-once`/`!prepend-once`/`!remove`),
//! **records inline** (mapa com `$base`/`$type` dentro de um flat foreign-key) e
//! **`!append-from`/`!merge`/`!prepend-from`** (mesclar arrays de outro record).
//!
//! Fora do escopo (erro claro, não parse silencioso errado): `$type`
//! create-from-scratch sem record-amostra da classe e templates `$instances`.

use std::collections::HashMap;

use crate::hashes::fnv1a32;
use crate::yaml::{Kind, Node};

/// Uma operação de alto nível extraída do documento.
pub enum Op {
    /// `$base`: clonar `base` em `record`.
    Clone { record: String, base: String },
    /// `$type`: criar `record` da classe `class` (offline = clona uma amostra).
    Create { record: String, class: String },
    /// Editar um flat.
    Edit { flat: String, op: EditOp },
}

/// Operação de edição de um flat. As variantes de array espelham as tags do
/// TweakXL (`!append`, `!prepend`, `!append-once`, `!prepend-once`, `!remove`). Mora aqui (não em
/// `writer.rs`) porque é o tipo de INTENÇÃO (Op::Edit) — desacoplado do writer offline, permite
/// expor `tweakxl.rs` numa lib SEM arrastar `writer`/`tweakdb`/`kraken` (ver `lib.rs`).
pub enum EditOp {
    /// `= valor` (escalar) ou `= [a, b, c]` (array inteiro).
    Assign(String),
    /// `+= valor` / `!append` — adiciona um elemento ao FIM do array.
    Append(String),
    /// `!append-once` — adiciona ao fim só se ainda não estiver presente.
    AppendOnce(String),
    /// `!prepend` — adiciona um elemento ao INÍCIO do array.
    Prepend(String),
    /// `!prepend-once` — adiciona ao início só se ainda não estiver presente.
    PrependOnce(String),
    /// `-= valor` / `!remove` — remove os elementos iguais (igualdade de bytes).
    Remove(String),
    /// `!append-from` / `!merge` — anexa ao FIM os elementos de outro flat array
    /// (pelo nome). O TweakXL trata `!merge` e `!append-from` como o mesmo op.
    AppendFrom(String),
    /// `!prepend-from` — anexa ao INÍCIO os elementos de outro flat array.
    PrependFrom(String),
}

/// Contexto da interpretação: a origem (caminho do arquivo, p/ o hash do nome
/// inline) e o contador por-hash que espelha o `m_inlineIndexSuffix` do TweakXL.
struct Ctx<'a> {
    source: &'a str,
    inline_counter: HashMap<String, u32>,
}

/// Interpreta um documento já parseado em uma lista ordenada de ops.
pub fn interpret(root: &Node) -> Result<Vec<Op>, String> {
    interpret_from(root, "")
}

/// Como [`interpret`], mas recebe a ORIGEM (caminho do arquivo) — entra no hash
/// do nome sintético dos records inline, igual ao `m_path` do TweakXL.
pub fn interpret_from(root: &Node, source: &str) -> Result<Vec<Op>, String> {
    let Kind::Map(entries) = &root.kind else {
        return Err("o documento precisa ser um mapa de records/flats".into());
    };
    let mut ctx = Ctx { source, inline_counter: HashMap::new() };
    let mut ops = Vec::new();
    for (key, node) in entries {
        process_entry(key, node, true, &mut ops, &mut ctx)?;
    }
    Ok(ops)
}

fn process_entry(path: &str, node: &Node, top: bool, ops: &mut Vec<Op>, ctx: &mut Ctx) -> Result<(), String> {
    match &node.kind {
        Kind::Scalar(s) => {
            ops.push(Op::Edit { flat: path.to_string(), op: EditOp::Assign(s.clone()) });
            Ok(())
        }
        Kind::Seq(items) => emit_array_ops(path, items, ops, ctx),
        Kind::Map(_) => process_map(path, node, top, ops, ctx),
    }
}

fn process_map(path: &str, node: &Node, top: bool, ops: &mut Vec<Op>, ctx: &mut Ctx) -> Result<(), String> {
    let Kind::Map(entries) = &node.kind else {
        return Ok(());
    };

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

    // Sem $base/$type: pode ser um valor-struct, `{$value: X}` ou aninhado.
    if !top {
        if let Some(scalar) = struct_map_to_scalar(entries) {
            ops.push(Op::Edit { flat: path.to_string(), op: EditOp::Assign(scalar) });
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

    // Caminho aninhado: cada filho vira um flat `path.filho`.
    for (k, v) in entries {
        if k == "$game" || k == "$dlc" {
            // Condições de game/DLC: offline aplicamos sempre (não dá pra avaliar).
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
/// elas mesmas records inline).
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
        for (k, v) in entries {
            if k.starts_with('$') {
                continue;
            }
            process_entry(&format!("{name}.{k}"), v, false, ops, ctx)?;
        }
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

/// Replica o `ComposeInlineName` do TweakXL: `parentFlat$XXXXXXXX`, onde XXXXXXXX
/// é o FNV1a32 (8 hex MAIÚSCULOS) de `source|parentFlat|recordType[|idx|counter]`.
fn compose_inline_name(parent_flat: &str, record_type: &str, ctx: &mut Ctx, item_index: i32) -> String {
    let mut h = String::new();
    h.push_str(ctx.source);
    h.push('|');
    h.push_str(parent_flat);
    h.push('|');
    h.push_str(record_type);
    if item_index >= 0 {
        h.push('|');
        h.push_str(&item_index.to_string());
        h.push('|');
        // O contador é indexado pelo hash ATÉ AQUI (com o separador final), igual
        // ao `m_inlineIndexSuffix[inlineHash]` do TweakXL.
        let c = ctx.inline_counter.entry(h.clone()).or_insert(0);
        *c += 1;
        let c = *c;
        h.push_str(&c.to_string());
    }
    format!("{parent_flat}${:08X}", fnv1a32(h.as_bytes()))
}

fn emit_array_ops(path: &str, items: &[Node], ops: &mut Vec<Op>, ctx: &mut Ctx) -> Result<(), String> {
    // 1) Resolve itens que são records inline (mapa com $base/$type) → cria o
    //    record e troca o item pelo nome sintético (foreign key), preservando a tag.
    let mut resolved: Vec<Node> = Vec::with_capacity(items.len());
    for (i, it) in items.iter().enumerate() {
        if is_inline_record(it) {
            let name = make_inline_record(path, it, ops, ctx, i as i32)?;
            resolved.push(Node { tag: it.tag.clone(), kind: Kind::Scalar(name) });
        } else {
            resolved.push(it.clone());
        }
    }

    // 2) Sem nenhuma tag → atribuição da lista inteira.
    let any_tagged = resolved.iter().any(|it| it.tag.is_some());
    if !any_tagged {
        let mut elems = Vec::with_capacity(resolved.len());
        for it in &resolved {
            let s = it
                .as_str()
                .ok_or_else(|| format!("'{path}': elemento de array precisa ser escalar"))?;
            elems.push(s.to_string());
        }
        ops.push(Op::Edit {
            flat: path.to_string(),
            op: EditOp::Assign(format!("[{}]", elems.join(", "))),
        });
        return Ok(());
    }

    // 3) Lista de operações por-item (cada item tem tag).
    for it in &resolved {
        let tag = it
            .tag
            .as_deref()
            .ok_or_else(|| format!("'{path}': item sem tag numa lista de operações (!append etc.)"))?;
        let v = it
            .as_str()
            .ok_or_else(|| format!("'{path}': operação '{tag}' espera valor escalar (ou record inline)"))?
            .to_string();
        let op = match tag {
            "!append" => EditOp::Append(v),
            "!append-once" => EditOp::AppendOnce(v),
            "!prepend" => EditOp::Prepend(v),
            "!prepend-once" => EditOp::PrependOnce(v),
            "!remove" | "!remove-all" => EditOp::Remove(v),
            "!append-from" | "!merge" => EditOp::AppendFrom(v),
            "!prepend-from" => EditOp::PrependFrom(v),
            other => return Err(format!("'{path}': tag de array desconhecida '{other}'")),
        };
        ops.push(Op::Edit { flat: path.to_string(), op });
    }
    Ok(())
}

/// Mapa com forma de struct conhecida (Quaternion/Vector/EulerAngles/Color) →
/// string posicional que o `encode_value` entende ("f,f,…" / "r,g,b,a").
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

fn set(items: &[&'static str]) -> std::collections::BTreeSet<&'static str> {
    items.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml;

    /// Resume cada Op em texto pra checagem (Op não deriva Debug/Eq).
    fn summarize(ops: &[Op]) -> Vec<String> {
        ops.iter()
            .map(|o| match o {
                Op::Clone { record, base } => format!("clone {record} <- {base}"),
                Op::Create { record, class } => format!("create {record} : {class}"),
                Op::Edit { flat, op } => format!("edit {flat} {}", editop(op)),
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
            EditOp::AppendFrom(v) => format!("append-from {v}"),
            EditOp::PrependFrom(v) => format!("prepend-from {v}"),
        }
    }
    fn run(src: &str) -> Vec<String> {
        let root = yaml::parse(src).unwrap();
        let root = crate::template::expand(&root).unwrap();
        summarize(&interpret_from(&root, "test.yaml").unwrap())
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
}
