//! Parser de bloco-YAML mínimo, suficiente para os tweaks declarativos do
//! TweakXL. Diferente das libs YAML de alto nível (serde_yaml, yaml-rust), este
//! **preserva as tags** (`!append`, `!remove`, …) e a **ordem** dos campos — o
//! TweakXL depende das duas, e por isso usa yaml-cpp de baixo nível.
//!
//! NÃO é YAML completo. Suporta: mapas e sequências em bloco, escalares, flow
//! simples de uma linha (`[a, b]` / `{x: 1}`), tags `!op`, comentários `#` e
//! aspas. Constructos avançados (âncoras `&`/`*`, blocos `|`/`>`, merge `<<`,
//! multi-documento) dão erro claro em vez de parse silencioso errado.

/// Um nó YAML, opcionalmente com uma tag (`!append` etc.).
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub tag: Option<String>,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Scalar(String),
    Seq(Vec<Node>),
    /// Mapa preservando ordem de inserção (importante p/ `$base` e overrides).
    Map(Vec<(String, Node)>),
}

impl Node {
    fn scalar(s: impl Into<String>) -> Self {
        Node { tag: None, kind: Kind::Scalar(s.into()) }
    }
    /// Atalho: o valor de uma chave num mapa (primeira ocorrência).
    pub fn get<'a>(&'a self, key: &str) -> Option<&'a Node> {
        match &self.kind {
            Kind::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match &self.kind {
            Kind::Scalar(s) => Some(s),
            _ => None,
        }
    }
}

/// Uma linha lógica: indentação (em espaços) e o conteúdo após ela.
struct Line {
    indent: usize,
    content: String,
}

/// Parseia um documento YAML em um [`Node`] raiz (sempre um mapa no TweakXL).
pub fn parse(text: &str) -> Result<Node, String> {
    let lines = preprocess(text)?;
    if lines.is_empty() {
        return Ok(Node { tag: None, kind: Kind::Map(Vec::new()) });
    }
    let mut i = 0;
    let node = parse_block(&lines, &mut i, lines[0].indent)?;
    if i != lines.len() {
        return Err(format!(
            "linha {}: indentação inesperada (esperava fim do documento)",
            i + 1
        ));
    }
    Ok(node)
}

/// Quebra em linhas lógicas: calcula indentação, remove comentários (respeitando
/// aspas), descarta linhas vazias e rejeita tabs e o separador de documento.
fn preprocess(text: &str) -> Result<Vec<Line>, String> {
    let mut out = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        if raw.contains('\t') && raw.trim_start_matches(' ').starts_with('\t') {
            return Err(format!("linha {}: tab na indentação (use espaços)", n + 1));
        }
        let stripped = strip_comment(raw);
        let trimmed_end = stripped.trim_end();
        let content = trimmed_end.trim_start();
        if content.is_empty() {
            continue;
        }
        if content == "---" || content == "..." {
            return Err(format!("linha {}: multi-documento não suportado", n + 1));
        }
        if content.starts_with('&') || content.starts_with('*') || content.starts_with("<<") {
            return Err(format!("linha {}: âncoras/merge YAML não suportados", n + 1));
        }
        if content.starts_with('|') || content.starts_with('>') {
            return Err(format!("linha {}: escalares de bloco (|/>) não suportados", n + 1));
        }
        let indent = trimmed_end.len() - content.len();
        out.push(Line { indent, content: content.to_string() });
    }
    Ok(out)
}

/// Remove um comentário `#` fora de aspas. Um `#` só inicia comentário no começo
/// da linha ou precedido por espaço (assim `LocKey#45371` não vira comentário).
fn strip_comment(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut prev_space = true; // começo de linha conta como "após espaço"
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'#' if !in_single && !in_double && prev_space => {
                return line[..i].to_string();
            }
            _ => {}
        }
        prev_space = b == b' ' || b == b'\t';
    }
    line.to_string()
}

/// Parseia um bloco (mapa ou sequência) cuja indentação base é `base`.
fn parse_block(lines: &[Line], i: &mut usize, base: usize) -> Result<Node, String> {
    let is_seq = is_seq_marker(&lines[*i].content);
    if is_seq {
        parse_seq(lines, i, base)
    } else {
        parse_map(lines, i, base)
    }
}

fn is_seq_marker(content: &str) -> bool {
    content == "-" || content.starts_with("- ")
}

fn parse_seq(lines: &[Line], i: &mut usize, base: usize) -> Result<Node, String> {
    let mut items = Vec::new();
    while *i < lines.len() && lines[*i].indent == base && is_seq_marker(&lines[*i].content) {
        let after = lines[*i].content[1..].trim_start().to_string(); // após o '-'
        *i += 1;
        // Constrói o sub-bloco do item: o resto da linha (re-indentado) + as
        // linhas seguintes mais fundas que o '-'.
        let item_indent = base + 2;
        let mut sub: Vec<Line> = Vec::new();
        if !after.is_empty() {
            sub.push(Line { indent: item_indent, content: after });
        }
        while *i < lines.len() && lines[*i].indent >= item_indent {
            sub.push(Line { indent: lines[*i].indent, content: lines[*i].content.clone() });
            *i += 1;
        }
        items.push(parse_item(sub, item_indent)?);
    }
    Ok(Node { tag: None, kind: Kind::Seq(items) })
}

/// Parseia o sub-bloco de um item de sequência (já isolado), tratando a tag e
/// o caso escalar/flow/mapa.
fn parse_item(sub: Vec<Line>, item_indent: usize) -> Result<Node, String> {
    if sub.is_empty() {
        return Ok(Node::scalar("")); // item vazio
    }
    // Tag no início do primeiro pedaço?
    let mut first = sub[0].content.clone();
    let tag = take_tag(&mut first);
    // Reconstrói: se sobrou conteúdo inline, ele é o valor; senão o valor é o
    // resto do sub-bloco (mapa/seq mais fundo).
    let mut node = if !first.trim().is_empty() {
        // valor inline (escalar ou flow); ignora linhas seguintes (não há, no uso real)
        if sub.len() == 1 {
            parse_scalar_or_flow(first.trim())?
        } else {
            // ex.: `- key: val` seguido de `  key2: val2` → mapa
            let mut relined = sub;
            relined[0].content = first;
            let mut j = 0;
            parse_block(&relined, &mut j, item_indent)?
        }
    } else {
        // tag sozinha na linha; valor = bloco seguinte
        if sub.len() == 1 {
            Node::scalar("")
        } else {
            let body: Vec<Line> = sub.into_iter().skip(1).collect();
            let mut j = 0;
            parse_block(&body, &mut j, body[0].indent)?
        }
    };
    node.tag = tag.or(node.tag);
    Ok(node)
}

fn parse_map(lines: &[Line], i: &mut usize, base: usize) -> Result<Node, String> {
    let mut entries: Vec<(String, Node)> = Vec::new();
    while *i < lines.len() && lines[*i].indent == base && !is_seq_marker(&lines[*i].content) {
        let line_no = *i + 1;
        let (key, rest) = split_key(&lines[*i].content)
            .ok_or_else(|| format!("linha {line_no}: esperava `chave: valor`"))?;
        *i += 1;
        let mut rest = rest.to_string();
        let tag = take_tag(&mut rest);
        let rest = rest.trim();

        let mut value = if !rest.is_empty() {
            parse_scalar_or_flow(rest)?
        } else if *i < lines.len() && lines[*i].indent > base {
            parse_block(lines, i, lines[*i].indent)?
        } else {
            Node::scalar("")
        };
        value.tag = tag.or(value.tag);
        entries.push((key, value));
    }
    Ok(Node { tag: None, kind: Kind::Map(entries) })
}

/// Separa `chave: resto` na primeira `: ` (ou `:` no fim). A chave pode estar
/// entre aspas. Retorna `None` se não houver `:`.
fn split_key(content: &str) -> Option<(String, &str)> {
    let bytes = content.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b':' if !in_single && !in_double => {
                let after = &content[i + 1..];
                // `:` precisa ser fim de linha ou seguido de espaço p/ separar.
                if after.is_empty() || after.starts_with(' ') {
                    let key = unquote(content[..i].trim());
                    return Some((key, after.trim_start()));
                }
            }
            _ => {}
        }
    }
    None
}

/// Extrai uma tag `!token` do início de `s` (consumindo-a). Devolve a tag (com o
/// `!`) ou `None`. Deixa o resto em `s`.
fn take_tag(s: &mut String) -> Option<String> {
    let t = s.trim_start();
    if !t.starts_with('!') {
        return None;
    }
    let end = t.find(char::is_whitespace).unwrap_or(t.len());
    let tag = t[..end].to_string();
    *s = t[end..].trim_start().to_string();
    Some(tag)
}

/// Escalar simples ou coleção flow de uma linha (`[..]` / `{..}`).
fn parse_scalar_or_flow(s: &str) -> Result<Node, String> {
    // Indicadores YAML não suportados na posição de valor (âncora/alias/bloco).
    if matches!(s.chars().next(), Some('&' | '*' | '|' | '>')) {
        return Err(format!("valor YAML não suportado (âncora/alias/bloco): '{s}'"));
    }
    if let Some(inner) = s.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        let items = split_flow(inner)?
            .into_iter()
            .map(|mut e| {
                let tag = take_tag(&mut e);
                let mut n = parse_scalar_or_flow(e.trim())?;
                n.tag = tag.or(n.tag);
                Ok(n)
            })
            .collect::<Result<Vec<_>, String>>()?;
        return Ok(Node { tag: None, kind: Kind::Seq(items) });
    }
    if let Some(inner) = s.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
        let mut entries = Vec::new();
        for part in split_flow(inner)? {
            if part.trim().is_empty() {
                continue;
            }
            let (k, v) = split_key(part.trim())
                .ok_or_else(|| format!("flow map inválido: '{part}'"))?;
            entries.push((k, parse_scalar_or_flow(v.trim())?));
        }
        return Ok(Node { tag: None, kind: Kind::Map(entries) });
    }
    Ok(Node::scalar(unquote(s)))
}

/// Quebra o interior de uma coleção flow por vírgulas no nível 0 (respeita
/// aspas e `[]`/`{}` aninhados).
fn split_flow(inner: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut start = 0usize;
    let bytes = inner.as_bytes();
    for i in 0..bytes.len() {
        match bytes[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'[' | b'{' if !in_single && !in_double => depth += 1,
            b']' | b'}' if !in_single && !in_double => depth -= 1,
            b',' if depth == 0 && !in_single && !in_double => {
                out.push(inner[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = inner[start..].to_string();
    if !last.trim().is_empty() || !out.is_empty() {
        out.push(last);
    }
    Ok(out)
}

/// Remove aspas simples/duplas externas, se houver.
fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map<'a>(n: &'a Node) -> &'a [(String, Node)] {
        match &n.kind {
            Kind::Map(e) => e,
            _ => panic!("esperava mapa, veio {:?}", n.kind),
        }
    }

    #[test]
    fn dotted_e_aninhado() {
        let n = parse("PreventionSystem.setup.totalEntitiesLimit: 40\nExample.struct:\n  foo: 1\n  bar: 2\n").unwrap();
        let e = map(&n);
        assert_eq!(e[0].0, "PreventionSystem.setup.totalEntitiesLimit");
        assert_eq!(e[0].1.as_str(), Some("40"));
        assert_eq!(e[1].0, "Example.struct");
        let inner = map(&e[1].1);
        assert_eq!(inner[0], ("foo".to_string(), Node::scalar("1")));
        assert_eq!(inner[1], ("bar".to_string(), Node::scalar("2")));
    }

    #[test]
    fn base_e_flow_seq() {
        let n = parse("Items.Weapon_B:\n  $base: Items.Weapon_A\n  tags: [a, b, c]\n").unwrap();
        let rec = &map(&n)[0].1;
        assert_eq!(rec.get("$base").unwrap().as_str(), Some("Items.Weapon_A"));
        let tags = rec.get("tags").unwrap();
        match &tags.kind {
            Kind::Seq(items) => assert_eq!(items.len(), 3),
            _ => panic!("tags não é seq"),
        }
    }

    #[test]
    fn seq_com_tags_inline() {
        let n = parse(
            "Vehicle.vehicle_list.list:\n  - !prepend Vehicle.v_012\n  - !append Vehicle.v_std\n  - !remove Vehicle.v_old\n",
        )
        .unwrap();
        let list = &map(&n)[0].1;
        match &list.kind {
            Kind::Seq(items) => {
                assert_eq!(items[0].tag.as_deref(), Some("!prepend"));
                assert_eq!(items[0].as_str(), Some("Vehicle.v_012"));
                assert_eq!(items[1].tag.as_deref(), Some("!append"));
                assert_eq!(items[2].tag.as_deref(), Some("!remove"));
            }
            _ => panic!("não é seq"),
        }
    }

    #[test]
    fn seq_item_tag_com_mapa() {
        // O caso statModifiers: `- !append` seguido de um mapa indentado.
        let n = parse(
            "Items.Weapon_B:\n  statModifiers:\n    - !append\n      $type: ConstantStatModifier\n      value: -0.15\n",
        )
        .unwrap();
        let rec = &map(&n)[0].1;
        let sm = rec.get("statModifiers").unwrap();
        match &sm.kind {
            Kind::Seq(items) => {
                assert_eq!(items[0].tag.as_deref(), Some("!append"));
                assert_eq!(items[0].get("$type").unwrap().as_str(), Some("ConstantStatModifier"));
                assert_eq!(items[0].get("value").unwrap().as_str(), Some("-0.15"));
            }
            _ => panic!("statModifiers não é seq"),
        }
    }

    #[test]
    fn comentario_e_lockey_hash() {
        let n = parse("# comentário\nItems.X:\n  displayName: LocKey#45371  # inline\n  range: 30\n").unwrap();
        let rec = &map(&n)[0].1;
        assert_eq!(rec.get("displayName").unwrap().as_str(), Some("LocKey#45371"));
        assert_eq!(rec.get("range").unwrap().as_str(), Some("30"));
    }

    #[test]
    fn rejeita_anchor() {
        assert!(parse("x: &anchor 1\n").is_err());
    }
}
