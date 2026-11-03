//! Expansão de templates `$instances` — pré-processamento puro da AST
//! ([`yaml::Node`]), rodado ANTES do `interpret`. Não toca no jogo nem no .bin.
//!
//! Um template é um nó (top-level ou item de array) com a chave `$instances`: uma
//! lista de mapas de dados. Para cada item, o corpo (sem `$instances`) é clonado e
//! os placeholders são substituídos pelos dados daquele item. No top-level, o
//! NOME do record também passa pela substituição (gera N records nomeados).
//!
//! Placeholder: `$(nome)` ou `${nome}`. Quando o escalar é SÓ o placeholder, ele é
//! trocado pelo valor inteiro do dado (pode ser não-escalar); embutido numa string,
//! faz substituição textual. Dado ausente/não-escalar embutido → removido.

use std::collections::HashMap;

use crate::tweaks::yaml::{Kind, Node};

type Data = HashMap<String, Node>;

/// Expande os `$instances` do documento. Idempotente em docs sem templates.
pub fn expand(root: &Node) -> Result<Node, String> {
    let Kind::Map(entradas) = &root.kind else {
        return Ok(root.clone());
    };
    let sem_dados = Data::new();
    let mut saida = Vec::with_capacity(entradas.len());
    for (chave, valor) in entradas {
        let Some(instancias) = valor.get("$instances") else {
            saida.push((chave.clone(), substituir(valor, &sem_dados)));
            continue;
        };
        let itens = instancias
            .as_seq()
            .ok_or_else(|| format!("'{chave}': $instances precisa ser uma lista de mapas (`- {{ nome: valor }}`)"))?;
        let corpo = sem_instances(valor);
        for item in itens {
            let campos = item
                .as_map()
                .ok_or_else(|| format!("'{chave}': cada item de $instances precisa ser um mapa (`- {{ nome: valor }}`)"))?;
            let dados: Data = campos.iter().cloned().collect();
            saida.push((format_string(chave, &dados), substituir(&corpo, &dados)));
        }
    }
    Ok(Node { aspas: root.aspas, tag: root.tag.clone(), kind: Kind::Map(saida) })
}

/// O mapa sem a chave `$instances`, com a mesma tag.
fn sem_instances(node: &Node) -> Node {
    let entradas = node.as_map().unwrap_or(&[]).iter().filter(|(k, _)| k != "$instances").cloned().collect();
    Node { aspas: node.aspas, tag: node.tag.clone(), kind: Kind::Map(entradas) }
}

/// O nó com os placeholders trocados pelos `dados`. Só item de array expande `$instances`: no mapa a chave fica e os itens são valor.
fn substituir(node: &Node, dados: &Data) -> Node {
    match &node.kind {
        Kind::Scalar(s) if !s.contains('$') => node.clone(),
        Kind::Scalar(s) => match whole_placeholder(s).and_then(|nome| dados.get(nome)) {
            Some(dado) => {
                let mut copia = dado.clone();
                if copia.tag.is_none() {
                    copia.tag = node.tag.clone();
                }
                copia
            }
            None => Node { aspas: node.aspas, tag: node.tag.clone(), kind: Kind::Scalar(format_string(s, dados)) },
        },
        Kind::Map(entradas) => Node {
            aspas: node.aspas,
            tag: node.tag.clone(),
            kind: Kind::Map(entradas.iter().map(|(k, v)| (k.clone(), substituir(v, dados))).collect()),
        },
        Kind::Seq(itens) => {
            let mut saida = Vec::with_capacity(itens.len());
            for item in itens {
                let Some(instancias) = item.get("$instances").and_then(Node::as_seq) else {
                    saida.push(substituir(item, dados));
                    continue;
                };
                let corpo = sem_instances(item);
                // Elemento que não é mapa não traz dado nenhum: é pulado.
                for campos in instancias.iter().filter_map(Node::as_map) {
                    let mut dados_do_item = dados.clone();
                    dados_do_item.extend(campos.iter().cloned());
                    let gerado = substituir(&corpo, &dados_do_item);
                    match gerado.get("$value") {
                        Some(valor) if !matches!(valor.kind, Kind::Map(_)) => {
                            let mut v = valor.clone();
                            if v.tag.is_none() {
                                v.tag = item.tag.clone();
                            }
                            saida.push(v);
                        }
                        _ => saida.push(gerado),
                    }
                }
            }
            Node { aspas: node.aspas, tag: node.tag.clone(), kind: Kind::Seq(saida) }
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
/// ausente ou não-escalar → o placeholder é removido.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tweaks::yaml;

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
        data.insert("x".into(), Node { aspas: false, tag: None, kind: Kind::Scalar("Foo".into()) });
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
