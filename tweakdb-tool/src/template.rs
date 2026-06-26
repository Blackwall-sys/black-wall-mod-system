//! Expansão de templates `$instances` do TweakXL — pré-processamento puro da AST
//! ([`yaml::Node`]), rodado ANTES do `interpret`. Não toca no jogo nem no .bin.
//!
//! Um template é um nó (top-level ou item de array) com a chave `$instances`: uma
//! lista de mapas de dados. Para cada item, o corpo (sem `$instances`) é clonado e
//! os placeholders são substituídos pelos dados daquele item. No top-level, o
//! NOME do record também passa pela substituição (gera N records nomeados).
//!
//! Placeholder: `$(nome)` ou `${nome}`. Quando o escalar é SÓ o placeholder, ele é
//! trocado pelo valor inteiro do dado (pode ser não-escalar); embutido numa string,
//! faz substituição textual. Dado ausente/não-escalar embutido → removido (igual ao
//! `FormatString` do TweakXL).

use std::collections::HashMap;

use crate::yaml::{Kind, Node};

type Data = HashMap<String, Node>;

/// Expande os `$instances` do documento. Idempotente em docs sem templates.
pub fn expand(root: &Node) -> Result<Node, String> {
    let Kind::Map(entries) = &root.kind else {
        return Ok(root.clone());
    };
    let empty = Data::new();
    let mut out: Vec<(String, Node)> = Vec::new();
    for (key, node) in entries {
        match node.get("$instances") {
            Some(inst) => {
                let Kind::Seq(items) = &inst.kind else {
                    return Err(format!("'{key}': $instances precisa ser uma lista"));
                };
                let body = strip_key(node, "$instances");
                for it in items {
                    let data = prepare(it, &empty)?;
                    let name = format_string(key, &data);
                    let mut n = body.clone();
                    process(&mut n, &data);
                    out.push((name, n));
                }
            }
            None => {
                // Sem template no topo: ainda processa placeholders/$instances aninhados.
                let mut n = node.clone();
                process(&mut n, &empty);
                out.push((key.clone(), n));
            }
        }
    }
    Ok(Node { tag: root.tag.clone(), kind: Kind::Map(out) })
}

/// Constrói o mapa de dados de um item de `$instances` (herda os dados externos).
fn prepare(node: &Node, outer: &Data) -> Result<Data, String> {
    let Kind::Map(entries) = &node.kind else {
        return Err("cada item de $instances precisa ser um mapa de dados".into());
    };
    let mut d = outer.clone();
    for (k, v) in entries {
        d.insert(k.clone(), v.clone());
    }
    Ok(d)
}

/// Substitui placeholders no nó (recursivo), expandindo `$instances` aninhados em
/// arrays.
fn process(node: &mut Node, data: &Data) {
    match &mut node.kind {
        Kind::Scalar(s) => {
            if !s.contains('$') {
                return;
            }
            if let Some(key) = whole_placeholder(s) {
                if let Some(v) = data.get(key) {
                    let tag = node.tag.take();
                    *node = v.clone();
                    if node.tag.is_none() {
                        node.tag = tag;
                    }
                    return;
                }
            }
            *s = format_string(s, data);
        }
        Kind::Map(entries) => {
            for (_, v) in entries.iter_mut() {
                process(v, data);
            }
        }
        Kind::Seq(items) => {
            let mut expanded: Vec<Node> = Vec::with_capacity(items.len());
            for it in items.iter() {
                match it.get("$instances") {
                    Some(inst) if matches!(inst.kind, Kind::Seq(_)) => {
                        let Kind::Seq(insts) = &inst.kind else { unreachable!() };
                        let body = strip_key(it, "$instances");
                        for inst_item in insts {
                            let d = match prepare(inst_item, data) {
                                Ok(d) => d,
                                Err(_) => continue,
                            };
                            let mut n = body.clone();
                            process(&mut n, &d);
                            // `$value` (não-mapa) → o item vira só esse valor.
                            if let Some(val) = n.get("$value").cloned() {
                                if !matches!(val.kind, Kind::Map(_)) {
                                    let mut vn = val;
                                    if vn.tag.is_none() {
                                        vn.tag = it.tag.clone();
                                    }
                                    expanded.push(vn);
                                    continue;
                                }
                            }
                            expanded.push(n);
                        }
                    }
                    _ => {
                        let mut it2 = it.clone();
                        process(&mut it2, data);
                        expanded.push(it2);
                    }
                }
            }
            *items = expanded;
        }
    }
}

/// Se o escalar é SÓ um placeholder (`$(nome)`/`${nome}`), devolve `nome`.
fn whole_placeholder(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    if b.len() < 4 || b[0] != b'$' {
        return None;
    }
    let close = match b[1] {
        b'(' => b')',
        b'{' => b'}',
        _ => return None,
    };
    if *b.last().unwrap() != close {
        return None;
    }
    let inner = &s[2..s.len() - 1];
    // Placeholder puro: o fechamento só pode estar no fim.
    if inner.as_bytes().contains(&close) {
        return None;
    }
    Some(inner)
}

/// Substituição textual de placeholders `$(nome)`/`${nome}` numa string. Dado
/// ausente ou não-escalar → o placeholder é removido (igual ao TweakXL).
fn format_string(input: &str, data: &Data) -> String {
    let mut out = String::new();
    let mut rest = input;
    loop {
        let Some(pos) = rest.find('$') else {
            out.push_str(rest);
            break;
        };
        let close = match rest.as_bytes().get(pos + 1) {
            Some(b'(') => ')',
            Some(b'{') => '}',
            _ => {
                out.push_str(&rest[..=pos]);
                rest = &rest[pos + 1..];
                continue;
            }
        };
        out.push_str(&rest[..pos]);
        let inner_start = pos + 2;
        match rest[inner_start..].find(close) {
            None => {
                out.push_str(&rest[pos..]);
                break;
            }
            Some(rel) => {
                let key = &rest[inner_start..inner_start + rel];
                if let Some(v) = data.get(key).and_then(Node::as_str) {
                    out.push_str(v);
                }
                rest = &rest[inner_start + rel + 1..];
            }
        }
    }
    out
}

/// Clona o nó (mapa) sem uma chave.
fn strip_key(node: &Node, key: &str) -> Node {
    if let Kind::Map(entries) = &node.kind {
        let kept = entries.iter().filter(|(k, _)| k != key).cloned().collect();
        Node { tag: node.tag.clone(), kind: Kind::Map(kept) }
    } else {
        node.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml;

    fn top(node: &Node) -> Vec<(String, Node)> {
        match &node.kind {
            Kind::Map(m) => m.clone(),
            _ => panic!("esperava Map"),
        }
    }

    #[test]
    fn template_top_level_expande_n_records() {
        let src = "\
Items.Gun_$(name):
  $instances:
    - { name: A, dmg: 10 }
    - { name: B, dmg: 20 }
  $type: gamedataItemType_Record
  damage: $(dmg)
";
        let root = yaml::parse(src).unwrap();
        let exp = expand(&root).unwrap();
        let t = top(&exp);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].0, "Items.Gun_A");
        assert_eq!(t[1].0, "Items.Gun_B");
        // placeholder de valor substituído
        assert_eq!(t[0].1.get("damage").unwrap().as_str(), Some("10"));
        assert_eq!(t[1].1.get("damage").unwrap().as_str(), Some("20"));
        // $instances removido do corpo
        assert!(t[0].1.get("$instances").is_none());
        assert_eq!(t[0].1.get("$type").unwrap().as_str(), Some("gamedataItemType_Record"));
    }

    #[test]
    fn placeholder_embutido_em_string() {
        let mut data = Data::new();
        data.insert("x".into(), Node { tag: None, kind: Kind::Scalar("Foo".into()) });
        assert_eq!(format_string("pre_$(x)_pos", &data), "pre_Foo_pos");
        assert_eq!(format_string("${x}", &data), "Foo");
        assert_eq!(format_string("sem_marca", &data), "sem_marca");
        // desconhecido → removido
        assert_eq!(format_string("a$(y)b", &data), "ab");
    }

    #[test]
    fn doc_sem_templates_inalterado() {
        let src = "Items.Foo:\n  damage: 5\n";
        let root = yaml::parse(src).unwrap();
        let exp = expand(&root).unwrap();
        assert_eq!(top(&exp).len(), 1);
        assert_eq!(top(&exp)[0].1.get("damage").unwrap().as_str(), Some("5"));
    }

    #[test]
    fn instances_aninhado_em_array() {
        let src = "\
Items.Foo:
  tags:
    - first
    - $instances:
        - { v: B }
        - { v: C }
      $value: $(v)
    - last
";
        let root = yaml::parse(src).unwrap();
        let exp = expand(&root).unwrap();
        let foo = &top(&exp)[0].1;
        let tags = foo.get("tags").unwrap();
        match &tags.kind {
            Kind::Seq(items) => {
                let vals: Vec<&str> = items.iter().filter_map(Node::as_str).collect();
                assert_eq!(vals, vec!["first", "B", "C", "last"]);
            }
            _ => panic!("esperava Seq"),
        }
    }
}
