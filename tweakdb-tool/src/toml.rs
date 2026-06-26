//! Parser de TOML mínimo para os tweaks declarativos — front-end alternativo ao
//! [`crate::yaml`]. Produz a **mesma** AST ([`yaml::Node`]), então o
//! [`crate::tweakxl::interpret`] roda sem nenhuma mudança. Zero dependências (só
//! `std`), igual ao `yaml.rs`.
//!
//! Subset suportado (suficiente p/ os tweaks offline):
//!   - Header de tabela `[Items.MeuItem]` → o nome do record é **literal** (o
//!     ponto NÃO aninha; é o nome do record). Aspas opcionais p/ nomes exóticos:
//!     `["Items.Algum.Nome"]`.
//!   - `chave = valor` dentro da tabela → um flat. Chave bare ou entre aspas
//!     (use aspas p/ `"$base"` / `"$type"`).
//!   - Valores: string (`"..."`/`'...'`), número/bool/bareword (mantidos como
//!     texto — o tipo vem do flat já existente), array `[a, b]`, inline table
//!     `{x = 1, y = 2}` (vira struct Vector/Color etc.).
//!   - **Operação de array (tag)**: um inline table de UMA chave começando com
//!     `!`, valor escalar, usado como item de array:
//!         mods = [ { "!append" = "X" }, { "!append" = "Y" } ]
//!     equivale ao YAML `mods: [ !append X, !append Y ]`.
//!   - Comentários `#`, e valores multi-linha em arrays/inline-tables (acumula
//!     até os colchetes/chaves balancearem).
//!
//! NÃO suporta (erro claro, sem parse silencioso errado): array-of-tables
//! `[[x]]`, strings multi-linha `"""`, datetimes, dotted-keys aninhadas.

use crate::yaml::{Kind, Node};

/// Parseia um documento TOML em um [`Node`] raiz (sempre um mapa, como no YAML).
pub fn parse(text: &str) -> Result<Node, String> {
    let mut root: Vec<(String, Node)> = Vec::new();
    let mut cur: Option<usize> = None; // índice da tabela atual em `root`
    for st in statements(text)? {
        let t = st.trim();
        // Header de tabela: `[nome]` (colchetes balanceados na linha).
        if let Some(rest) = t.strip_prefix('[') {
            if rest.starts_with('[') {
                return Err("array-of-tables `[[...]]` não suportado".into());
            }
            let inner = rest
                .strip_suffix(']')
                .ok_or_else(|| format!("header de tabela malformado: `{t}`"))?;
            let name = unquote(inner.trim());
            let idx = match root.iter().position(|(k, _)| *k == name) {
                Some(i) => i,
                None => {
                    root.push((name, Node { tag: None, kind: Kind::Map(Vec::new()) }));
                    root.len() - 1
                }
            };
            cur = Some(idx);
            continue;
        }
        // `chave = valor`.
        let (k, v) = split_kv(t)?;
        let key = unquote(k.trim());
        let val = parse_value(v.trim())?;
        match cur {
            Some(i) => match &mut root[i].1.kind {
                Kind::Map(m) => m.push((key, val)),
                _ => unreachable!("tabela é sempre Map"),
            },
            None => root.push((key, val)),
        }
    }
    Ok(Node { tag: None, kind: Kind::Map(root) })
}

/// Quebra o documento em statements lógicos: cada header `[..]` e cada
/// `chave = valor` (podendo abranger várias linhas físicas enquanto colchetes /
/// chaves não balancearem). Tira comentários respeitando aspas.
fn statements(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut depth: i32 = 0;
    for raw in text.lines() {
        let (code, delta) = scan_line(raw)?;
        if depth == 0 {
            let t = code.trim();
            if t.is_empty() {
                continue;
            }
            if t.starts_with('[') && delta == 0 {
                out.push(t.to_string()); // header de tabela
                continue;
            }
            buf.push_str(code.trim_end());
            depth += delta;
        } else {
            buf.push(' ');
            buf.push_str(code.trim());
            depth += delta;
        }
        if depth <= 0 && !buf.is_empty() {
            out.push(std::mem::take(&mut buf));
            depth = 0;
        }
    }
    if !buf.trim().is_empty() {
        return Err("statement TOML incompleto (colchete/chave não fechado)".into());
    }
    Ok(out)
}

/// Remove o comentário `#` (fora de aspas) de uma linha e devolve o saldo de
/// colchetes/chaves `[]{}` (fora de aspas) dela.
fn scan_line(s: &str) -> Result<(String, i32), String> {
    let mut out = String::new();
    let mut depth = 0i32;
    let mut q: Option<char> = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match q {
            Some('"') => {
                out.push(c);
                if c == '\\' {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                } else if c == '"' {
                    q = None;
                }
            }
            Some(_) => {
                // string literal '...': sem escapes.
                out.push(c);
                if c == '\'' {
                    q = None;
                }
            }
            None => match c {
                '#' => break,
                '"' | '\'' => {
                    q = Some(c);
                    out.push(c);
                }
                '[' | '{' => {
                    depth += 1;
                    out.push(c);
                }
                ']' | '}' => {
                    depth -= 1;
                    out.push(c);
                }
                _ => out.push(c),
            },
        }
    }
    if q.is_some() {
        return Err(format!("aspas não fechadas: `{s}`"));
    }
    Ok((out, depth))
}

/// Separa `chave = valor` no primeiro `=` em nível 0 (fora de aspas/colchetes).
fn split_kv(s: &str) -> Result<(&str, &str), String> {
    let mut depth = 0i32;
    let mut q: Option<char> = None;
    for (i, c) in s.char_indices() {
        match q {
            Some('"') => {
                if c == '"' {
                    q = None;
                }
            }
            Some(_) => {
                if c == '\'' {
                    q = None;
                }
            }
            None => match c {
                '"' | '\'' => q = Some(c),
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                '=' if depth == 0 => return Ok((&s[..i], &s[i + 1..])),
                _ => {}
            },
        }
    }
    Err(format!("esperava `chave = valor` em `{s}`"))
}

/// Parseia um valor TOML em um [`Node`].
fn parse_value(s: &str) -> Result<Node, String> {
    let s = s.trim();
    let first = s.chars().next().ok_or("valor vazio")?;
    match first {
        '"' | '\'' => Ok(Node { tag: None, kind: Kind::Scalar(unquote(s)) }),
        '[' => {
            let inner = s
                .strip_prefix('[')
                .and_then(|x| x.strip_suffix(']'))
                .ok_or_else(|| format!("array malformado: `{s}`"))?;
            let mut items = Vec::new();
            for piece in split_commas(inner)? {
                items.push(parse_value(&piece)?);
            }
            Ok(Node { tag: None, kind: Kind::Seq(items) })
        }
        '{' => {
            let inner = s
                .strip_prefix('{')
                .and_then(|x| x.strip_suffix('}'))
                .ok_or_else(|| format!("inline table malformada: `{s}`"))?;
            let mut entries: Vec<(String, Node)> = Vec::new();
            for piece in split_commas(inner)? {
                let (k, v) = split_kv(&piece)?;
                entries.push((unquote(k.trim()), parse_value(v.trim())?));
            }
            // Convenção de tag: 1 chave começando com `!` → nó com tag. O valor
            // pode ser escalar (ex.: `{ "!append" = "X" }`) OU um mapa = record
            // inline (ex.: `{ "!append" = { "$type" = "...", v = 1 } }`). Um
            // array como valor da tag é ambíguo (use array de `{!op = item}`).
            if entries.len() == 1 && entries[0].0.starts_with('!') {
                let (tag, node) = entries.pop().unwrap();
                if matches!(node.kind, Kind::Seq(_)) {
                    return Err(format!(
                        "tag `{tag}` aceita escalar ou record inline (mapa), não array; p/ vários use `[ {{ \"{tag}\" = item }}, ... ]`"
                    ));
                }
                return Ok(Node { tag: Some(tag), kind: node.kind });
            }
            Ok(Node { tag: None, kind: Kind::Map(entries) })
        }
        _ => Ok(Node { tag: None, kind: Kind::Scalar(s.to_string()) }),
    }
}

/// Separa por vírgulas em nível 0 (fora de aspas/colchetes); descarta vazios
/// (tolera vírgula final).
fn split_commas(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut q: Option<char> = None;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match q {
            Some('"') => {
                if c == '"' {
                    q = None;
                }
            }
            Some(_) => {
                if c == '\'' {
                    q = None;
                }
            }
            None => match c {
                '"' | '\'' => q = Some(c),
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    let piece = s[start..i].trim();
                    if !piece.is_empty() {
                        out.push(piece.to_string());
                    }
                    start = i + 1;
                }
                _ => {}
            },
        }
    }
    if q.is_some() {
        return Err(format!("aspas não fechadas em `{s}`"));
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        out.push(last.to_string());
    }
    Ok(out)
}

/// Tira aspas (`"basic"` com escapes simples / `'literal'`); bareword volta
/// como está.
fn unquote(s: &str) -> String {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() >= 2 {
        if b[0] == b'"' && b[b.len() - 1] == b'"' {
            return unescape(&s[1..s.len() - 1]);
        }
        if b[0] == b'\'' && b[b.len() - 1] == b'\'' {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flats(node: &Node) -> Vec<(String, Node)> {
        match &node.kind {
            Kind::Map(m) => m.clone(),
            _ => panic!("esperava Map na raiz"),
        }
    }

    #[test]
    fn record_com_base_e_flats() {
        let src = r#"
            # cria uma arma a partir de uma base
            ["Items.MinhaArma"]
            "$base" = "Items.ArmaBase"
            damage = 100
            critChance = 0.5
        "#;
        let root = parse(src).unwrap();
        let top = flats(&root);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].0, "Items.MinhaArma");
        let rec = flats(&top[0].1);
        assert_eq!(rec[0], ("$base".into(), Node { tag: None, kind: Kind::Scalar("Items.ArmaBase".into()) }));
        assert_eq!(rec[1].0, "damage");
        assert_eq!(rec[1].1.as_str(), Some("100"));
        assert_eq!(rec[2].1.as_str(), Some("0.5"));
    }

    #[test]
    fn array_simples_atribuicao() {
        let root = parse(r#"tags = ["A", "B", "C"]"#).unwrap();
        let top = flats(&root);
        match &top[0].1.kind {
            Kind::Seq(items) => {
                assert_eq!(items.len(), 3);
                assert!(items.iter().all(|i| i.tag.is_none()));
                assert_eq!(items[0].as_str(), Some("A"));
            }
            k => panic!("esperava Seq, achei {k:?}"),
        }
    }

    #[test]
    fn ops_de_array_por_item_com_tag() {
        let root = parse(r#"mods = [ { "!append" = "X" }, { "!remove" = "Y" } ]"#).unwrap();
        let top = flats(&root);
        match &top[0].1.kind {
            Kind::Seq(items) => {
                assert_eq!(items[0].tag.as_deref(), Some("!append"));
                assert_eq!(items[0].as_str(), Some("X"));
                assert_eq!(items[1].tag.as_deref(), Some("!remove"));
                assert_eq!(items[1].as_str(), Some("Y"));
            }
            k => panic!("esperava Seq, achei {k:?}"),
        }
    }

    #[test]
    fn inline_table_vira_struct() {
        let root = parse(r#"pos = { x = 1.0, y = 2.0, z = 3.0 }"#).unwrap();
        let top = flats(&root);
        match &top[0].1.kind {
            Kind::Map(m) => {
                assert_eq!(m.len(), 3);
                assert_eq!(m[0].0, "x");
                assert_eq!(m[2].1.as_str(), Some("3.0"));
            }
            k => panic!("esperava Map, achei {k:?}"),
        }
    }

    #[test]
    fn multilinha_e_comentarios() {
        let src = r#"
            tags = [
              "A",   # primeiro
              "B",
            ]
        "#;
        let root = parse(src).unwrap();
        match &flats(&root)[0].1.kind {
            Kind::Seq(items) => assert_eq!(items.len(), 2),
            _ => panic!("esperava Seq"),
        }
    }

    #[test]
    fn tag_com_array_erra() {
        let err = parse(r#"x = { "!append" = ["A", "B"] }"#).unwrap_err();
        assert!(err.contains("não array"), "msg: {err}");
    }

    #[test]
    fn tag_com_mapa_inline_ok() {
        // `{ "!append" = { ...mapa... } }` = append de record inline → nó com tag + Map.
        let root = parse(r#"mods = [ { "!append" = { "$type" = "gamedataX_Record", v = 1 } } ]"#).unwrap();
        let top = match &root.kind {
            Kind::Map(m) => m,
            _ => panic!(),
        };
        match &top[0].1.kind {
            Kind::Seq(items) => {
                assert_eq!(items[0].tag.as_deref(), Some("!append"));
                assert!(matches!(items[0].kind, Kind::Map(_)));
            }
            _ => panic!("esperava Seq"),
        }
    }

    #[test]
    fn header_com_aspas() {
        let root = parse("[\"Base.With.Dots\"]\nv = 1\n").unwrap();
        assert_eq!(flats(&root)[0].0, "Base.With.Dots");
    }
}
