//! Leitor do subconjunto de YAML que o BWMS lê: os tweaks declarativos (`.yaml`, lidos pelo `tweaks`) e o `.xl`
//! (`instalacao::xl`). Um leitor só dá aos dois a mesma regra. Diferente das
//! libs YAML de alto nível (serde_yaml, yaml-rust), este **preserva as tags** (`!append`, `!exclude`, …) e a **ordem** dos
//! campos: os dois formatos dependem das duas.
//!
//! Suporta: mapas e sequências em bloco, inclusive a sequência no recuo da própria chave (`chave:` e, embaixo, `- item` no
//! mesmo recuo, a forma do mapa de herança); escalares; flow de uma linha (`[a, b]`, `{x: 1}`, aninhados; a
//! vírgula final não abre item); tags `!op`; âncora `&nome` e alias `*nome` como valor inteiro (o `.xl` de customização
//! usa); comentários `#`; aspas.
//!
//! Fora do subconjunto, a lista única:
//! - dão erro: merge `<<: *x`; marcador de documento `---`/`...` (multi-documento); escalar de bloco `|`/`>`; tab na
//!   indentação; âncora ou alias sozinhos numa linha; alias sem a âncora antes;
//! - ficam literais, sem erro: âncora e alias dentro de `[..]`/`{..}` (`[*x]` é o escalar `*x`); escape desconhecido dentro de aspas
//!   duplas (`"base\x"` fica `base\x`). Os escapes do YAML (`''` na simples; `\\`, `\"`, `\n`… na dupla) viram o caractere (`unquote`).

use std::collections::HashMap;

/// Um nó YAML, opcionalmente com uma tag (`!append`, `!exclude`, guardada com o `!`).
#[derive(Debug, Clone)]
pub struct Node {
    /// Escalar escrito entre aspas. Só escalar com aspas deduz `String` quando cria um
    /// flat sem tipo declarado; fora disso as aspas não mudam o valor.
    pub aspas: bool,
    pub tag: Option<String>,
    pub kind: Kind,
}

/// A forma do documento (tag e conteúdo); as aspas só servem à dedução de tipo.
impl PartialEq for Node {
    fn eq(&self, outro: &Self) -> bool {
        self.tag == outro.tag && self.kind == outro.kind
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Scalar(String),
    Seq(Vec<Node>),
    /// Mapa preservando ordem de inserção (importante p/ `$base` e overrides). Chave pode repetir, como no YAML.
    Map(Vec<(String, Node)>),
}

impl Node {
    fn scalar(s: impl Into<String>) -> Self {
        Node { aspas: false, tag: None, kind: Kind::Scalar(s.into()) }
    }
    /// Atalho: o valor de uma chave num mapa (primeira ocorrência).
    pub fn get<'a>(&'a self, key: &str) -> Option<&'a Node> {
        self.as_map()?.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub fn as_str(&self) -> Option<&str> {
        match &self.kind {
            Kind::Scalar(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_seq(&self) -> Option<&[Node]> {
        match &self.kind {
            Kind::Seq(items) => Some(items),
            _ => None,
        }
    }
    pub fn as_map(&self) -> Option<&[(String, Node)]> {
        match &self.kind {
            Kind::Map(entries) => Some(entries),
            _ => None,
        }
    }
}

/// Uma linha lógica: o número dela no arquivo (para a mensagem de erro), a indentação (em espaços) e o conteúdo após ela.
struct Line {
    n: usize,
    indent: usize,
    content: String,
}

/// Parseia um documento YAML em um [`Node`] raiz (um mapa no tweak e no `.xl`; documento vazio = mapa vazio).
pub fn parse(text: &str) -> Result<Node, String> {
    // BOM UTF-8 no início (há editor que grava os .yaml com ele): fica fora do texto, senão gruda no nome do 1º record.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines = preprocess(text)?;
    let Some(base) = lines.first().map(|l| l.indent) else {
        return Ok(Node { aspas: false, tag: None, kind: Kind::Map(Vec::new()) });
    };
    let mut parser = Parser { lines, pos: 0, anchors: HashMap::new() };
    let node = parser.block(base)?;
    if let Some(sobra) = parser.lines.get(parser.pos) {
        return Err(format!(
            "linha {}: indentação inesperada (esperava fim do documento); alinhe a linha com as chaves do bloco dela",
            sobra.n
        ));
    }
    Ok(node)
}

/// Quebra em linhas lógicas: calcula indentação, remove comentários (respeitando aspas), descarta linhas vazias e rejeita o que
/// a lista de fora do subconjunto dá como erro numa linha só.
fn preprocess(text: &str) -> Result<Vec<Line>, String> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let trimmed_end = strip_comment(raw).trim_end();
        let content = trimmed_end.trim_start();
        if content.is_empty() {
            continue;
        }
        if trimmed_end.trim_start_matches(' ').starts_with('\t') {
            return Err(format!("linha {n}: tab na indentação; use espaços"));
        }
        if content == "---" || content == "..." {
            return Err(format!(
                "linha {n}: marcador de documento `{content}` não suportado; deixe um documento só por arquivo, sem `{content}`"
            ));
        }
        if content.starts_with('&') || content.starts_with('*') {
            return Err(format!(
                "linha {n}: âncora ou alias sozinho na linha não é suportado; escreva na linha da chave (`chave: &nome` ou `chave: *nome`)"
            ));
        }
        if content.starts_with('|') || content.starts_with('>') {
            return Err(format!("linha {n}: escalar de bloco (`|`/`>`) não suportado; escreva o valor numa linha só"));
        }
        let indent = trimmed_end.len() - content.len();
        out.push(Line { n, indent, content: content.to_string() });
    }
    Ok(out)
}

/// Diz se o que vem logo depois de `prefixo` (o trecho da linha até ali) é o começo de um escalar, onde a aspa abre trecho entre
/// aspas e o `[`/`{` abre coleção flow: o começo do conteúdo; depois de `: ` ou de `- `; depois de tag ou âncora (`!t `, `&a `)
/// numa dessas posições; e, dentro de `[..]`/`{..}` (`em_fluxo`), depois de `[`, `{` ou `,`. No meio de um escalar simples a
/// aspa é letra, mesmo depois de espaço (`V's car`, `Rock 'n roll`), como no YAML. Laço em vez de recursão: uma linha com
/// milhares de `- ` ou de tags não estoura a pilha da thread do jogo.
fn inicio_de_escalar(mut prefixo: &[u8], em_fluxo: bool) -> bool {
    let separa = |b: &u8| matches!(b, b' ' | b'\t') || (em_fluxo && matches!(b, b'[' | b'{' | b','));
    loop {
        let fim = prefixo.iter().rposition(|b| !matches!(b, b' ' | b'\t')).map_or(0, |p| p + 1);
        let (texto, espaco) = (&prefixo[..fim], fim < prefixo.len());
        prefixo = match texto.last() {
            None => return true,
            Some(b'[' | b'{' | b',') if em_fluxo => return true,
            Some(b':') => return espaco,
            Some(b'-') if espaco => &texto[..fim - 1],
            Some(_) if espaco => {
                let token = texto.iter().rposition(separa).map_or(0, |p| p + 1);
                if !matches!(texto[token], b'!' | b'&') {
                    return false;
                }
                &texto[..token]
            }
            Some(_) => return false,
        };
    }
}

/// Remove um comentário `#` fora de aspas. Um `#` só inicia comentário no começo da linha ou precedido por espaço (assim
/// `LocKey#45371` não vira comentário).
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    // Profundidade de `[..]`/`{..}`: lá dentro a aspa também abre depois de `[`, `{` e `,`.
    let mut fluxo = 0usize;
    let mut prev_space = true; // começo de linha conta como "após espaço"
    let mut it = bytes.iter().enumerate();
    while let Some((i, &b)) = it.next() {
        let aspas = in_single || in_double;
        match b {
            // Escapes do YAML, com o trecho entre aspas ainda aberto: `''` dentro de aspa simples e `\` com o byte seguinte
            // dentro de aspa dupla (`\"`, `\\`).
            b'\'' if in_single && bytes.get(i + 1) == Some(&b'\'') => {
                it.next();
            }
            b'\\' if in_double => {
                it.next();
            }
            b'\'' if !in_double && (in_single || inicio_de_escalar(&bytes[..i], fluxo > 0)) => in_single = !in_single,
            b'"' if !in_single && (in_double || inicio_de_escalar(&bytes[..i], fluxo > 0)) => in_double = !in_double,
            b'[' | b'{' if !aspas && (fluxo > 0 || inicio_de_escalar(&bytes[..i], false)) => fluxo += 1,
            b']' | b'}' if !aspas && fluxo > 0 => fluxo -= 1,
            b'#' if !aspas && prev_space => return &line[..i],
            _ => {}
        }
        prev_space = b == b' ' || b == b'\t';
    }
    line
}

fn is_seq_marker(content: &str) -> bool {
    content == "-" || content.starts_with("- ")
}

/// Separa `chave: resto` no primeiro `:` fora de aspas seguido de espaço, tab ou fim de linha (`LocKey:1` não é chave). A chave
/// pode estar entre aspas. Retorna `None` se não houver esse `:`.
fn split_key(content: &str) -> Option<(String, &str)> {
    let bytes = content.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut it = bytes.iter().enumerate();
    while let Some((i, &b)) = it.next() {
        match b {
            // Escapes do YAML, com o trecho entre aspas ainda aberto: `''` dentro de aspa simples e `\` com o byte seguinte
            // dentro de aspa dupla (`\"`, `\\`).
            b'\'' if in_single && bytes.get(i + 1) == Some(&b'\'') => {
                it.next();
            }
            b'\\' if in_double => {
                it.next();
            }
            b'\'' if !in_double && (in_single || inicio_de_escalar(&bytes[..i], false)) => in_single = !in_single,
            b'"' if !in_single && (in_double || inicio_de_escalar(&bytes[..i], false)) => in_double = !in_double,
            b':' if !in_single && !in_double => {
                let after = &content[i + 1..];
                if after.is_empty() || after.starts_with([' ', '\t']) {
                    return Some((unquote(content[..i].trim()), after.trim_start()));
                }
            }
            _ => {}
        }
    }
    None
}

/// Tira do começo de `s` a propriedade (`!tag` ou `&âncora`) até o primeiro espaço e a devolve; o resto fica em `s`.
fn take_token(s: &mut String) -> String {
    let t = s.trim_start();
    let end = t.find(char::is_whitespace).unwrap_or(t.len());
    let token = t[..end].to_string();
    *s = t[end..].trim_start().to_string();
    token
}

/// Tira as propriedades do começo do valor: a tag (`!append`, guardada com o `!`) e a âncora (`&nome`, guardada sem o `&`), em
/// qualquer ordem.
fn take_props(s: &mut String) -> (Option<String>, Option<String>) {
    let (mut tag, mut anchor) = (None, None);
    loop {
        if tag.is_none() && s.starts_with('!') {
            tag = Some(take_token(s));
        } else if anchor.is_none() && s.starts_with('&') {
            anchor = Some(take_token(s)[1..].to_string());
        } else {
            return (tag, anchor);
        }
    }
}

/// Estado do leitor: as linhas, a posição e as âncoras (`&nome`) já lidas, que os aliases (`*nome`) copiam.
struct Parser {
    lines: Vec<Line>,
    pos: usize,
    anchors: HashMap<String, Node>,
}

impl Parser {
    /// Bloco (mapa ou sequência) que começa na linha atual, no recuo `indent`.
    fn block(&mut self, indent: usize) -> Result<Node, String> {
        if is_seq_marker(&self.lines[self.pos].content) {
            self.seq(indent)
        } else {
            self.map(indent)
        }
    }

    fn map(&mut self, indent: usize) -> Result<Node, String> {
        let mut entries = Vec::new();
        while let Some(line) = self.lines.get(self.pos) {
            if line.indent != indent || is_seq_marker(&line.content) {
                break;
            }
            let n = line.n;
            if line.content.starts_with("<<") {
                return Err(format!("linha {n}: merge `<<` não é suportado; repita as chaves no mapa em vez de mesclar a âncora"));
            }
            let (key, rest) = split_key(&line.content)
                .ok_or_else(|| format!("linha {n}: esperava `chave: valor`; confira o recuo e o espaço depois do `:`"))?;
            let rest = rest.to_string();
            self.pos += 1;
            let value = self.value(&rest, indent, true, n)?;
            entries.push((key, value));
        }
        Ok(Node { aspas: false, tag: None, kind: Kind::Map(entries) })
    }

    fn seq(&mut self, indent: usize) -> Result<Node, String> {
        let mut items = Vec::new();
        while let Some(line) = self.lines.get(self.pos) {
            if line.indent != indent || !is_seq_marker(&line.content) {
                break;
            }
            let after = line.content[1..].trim_start();
            if after.is_empty() {
                self.pos += 1;
                items.push(self.nested(indent, false)?);
                continue;
            }
            // O item começa na coluna do texto depois do `- `: a linha vira a primeira do item nesse recuo, e as chaves seguintes de
            // um item-mapa se alinham com ela, como no YAML.
            let col = indent + line.content.len() - after.len();
            let after = after.to_string();
            let line = &mut self.lines[self.pos];
            line.indent = col;
            line.content = after;
            items.push(self.item(col, indent)?);
        }
        Ok(Node { aspas: false, tag: None, kind: Kind::Seq(items) })
    }

    /// Item de sequência já reescrito como linha própria no recuo `col`: seq aninhada (`- - x`), mapa (`- chave: valor` e as chaves
    /// seguintes em `col`; tag e âncora antes da 1ª chave valem para o mapa, `- !append $type: X`), ou valor (alias, inline, ou tag e
    /// âncora sozinhas com o bloco mais fundo que o `-`).
    fn item(&mut self, col: usize, seq_indent: usize) -> Result<Node, String> {
        let Line { n, content, .. } = &self.lines[self.pos];
        let (n, content) = (*n, content.clone());
        if is_seq_marker(&content) {
            return self.seq(col);
        }
        let mut rest = content.clone();
        let (tag, anchor) = take_props(&mut rest);
        if rest.starts_with(['[', '{', '*']) || split_key(&rest).is_none() {
            self.pos += 1;
            return self.value(&content, seq_indent, false, n);
        }
        self.lines[self.pos].content = rest;
        let node = self.map(col)?;
        Ok(self.with_props(node, tag, anchor))
    }

    /// Valor depois de `chave:` (ou de `-`): alias, ou tag/âncora e o valor inline ou o bloco abaixo. `compact` aceita a sequência no
    /// recuo da própria chave, que só vale depois de `chave:`.
    fn value(&mut self, rest: &str, indent: usize, compact: bool, n: usize) -> Result<Node, String> {
        if let Some(name) = rest.strip_prefix('*') {
            let name = name.trim();
            return self
                .anchors
                .get(name)
                .cloned()
                .ok_or_else(|| format!("linha {n}: alias *{name} sem âncora antes; defina `&{name}` antes de usar `*{name}`"));
        }
        let mut rest = rest.to_string();
        let (tag, anchor) = take_props(&mut rest);
        let node = if rest.is_empty() { self.nested(indent, compact)? } else { inline(&rest, n)? };
        Ok(self.with_props(node, tag, anchor))
    }

    /// Valor de bloco de `chave:`/`-` sem nada inline: o bloco mais fundo que `indent`; com `compact`, também a sequência no recuo
    /// `indent`; sem nada disso, o escalar vazio.
    fn nested(&mut self, indent: usize, compact: bool) -> Result<Node, String> {
        match self.lines.get(self.pos) {
            Some(l) if l.indent > indent => {
                let child = l.indent;
                self.block(child)
            }
            Some(l) if compact && l.indent == indent && is_seq_marker(&l.content) => self.seq(indent),
            _ => Ok(Node::scalar("")),
        }
    }

    /// Aplica a tag ao nó e guarda a âncora com o nó já marcado (o alias copia a tag junto).
    fn with_props(&mut self, mut node: Node, tag: Option<String>, anchor: Option<String>) -> Node {
        node.tag = tag.or(node.tag);
        if let Some(name) = anchor {
            self.anchors.insert(name, node.clone());
        }
        node
    }
}

/// Valor escrito na própria linha: escalar ou flow (`[..]`/`{..}`).
fn inline(s: &str, n: usize) -> Result<Node, String> {
    if matches!(s.chars().next(), Some('&' | '*' | '|' | '>')) {
        return Err(format!(
            "linha {n}: valor `{s}` não suportado; escalar de bloco (`|`/`>`) vai numa linha só, e o valor leva no máximo uma \
             âncora, com o alias `*nome` sozinho"
        ));
    }
    flow_or_scalar(s).map_err(|e| format!("linha {n}: {e}"))
}

/// Escalar simples ou coleção flow de uma linha (`[..]` / `{..}`).
fn flow_or_scalar(s: &str) -> Result<Node, String> {
    if s.starts_with('[') {
        let inner = s
            .strip_prefix('[')
            .and_then(|x| x.strip_suffix(']'))
            .ok_or_else(|| format!("sequência inline malformada: '{s}'; feche com `]` na mesma linha"))?
            .trim();
        // Em YAML a vírgula final não abre item: `[ 1, 2, ]` tem 2. `[ , ]` fica com 2 itens vazios: virar `[]` mudaria o pedido
        // (no `.xl`, `[]` é o hide-all e o node path vazio).
        let inner = match inner.strip_suffix(',') {
            Some(resto) if !resto.trim().is_empty() => resto,
            _ => inner,
        };
        let items = split_flow(inner).into_iter().map(flow_item).collect::<Result<Vec<_>, String>>()?;
        return Ok(Node { aspas: false, tag: None, kind: Kind::Seq(items) });
    }
    if s.starts_with('{') {
        let inner = s
            .strip_prefix('{')
            .and_then(|x| x.strip_suffix('}'))
            .ok_or_else(|| format!("mapa inline malformado: '{s}'; feche com `}}` na mesma linha"))?;
        let mut entries = Vec::new();
        for part in split_flow(inner) {
            if part.trim().is_empty() {
                continue;
            }
            let (k, v) = split_key(part.trim()).ok_or_else(|| format!("flow map inválido: '{part}'; use `{{chave: valor}}`"))?;
            entries.push((k, flow_item(v)?));
        }
        return Ok(Node { aspas: false, tag: None, kind: Kind::Map(entries) });
    }
    let aspas = s.len() >= 2 && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')));
    Ok(Node { aspas, tag: None, kind: Kind::Scalar(unquote(s)) })
}

/// Item ou valor dentro de `[..]`/`{..}`: tag, flow aninhado ou escalar literal (âncora e alias não valem aqui).
fn flow_item(raw: &str) -> Result<Node, String> {
    let mut s = raw.trim().to_string();
    let tag = if s.starts_with('!') { Some(take_token(&mut s)) } else { None };
    let mut node = flow_or_scalar(&s)?;
    node.tag = tag.or(node.tag);
    Ok(node)
}

/// Quebra o interior de uma coleção flow por vírgulas no nível 0 (respeita aspas e `[]`/`{}` aninhados).
fn split_flow(inner: &str) -> Vec<&str> {
    let bytes = inner.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut start = 0usize;
    let mut it = bytes.iter().enumerate();
    while let Some((i, &b)) = it.next() {
        match b {
            // Escapes do YAML, com o trecho entre aspas ainda aberto: `''` dentro de aspa simples e `\` com o byte seguinte
            // dentro de aspa dupla (`\"`, `\\`).
            b'\'' if in_single && bytes.get(i + 1) == Some(&b'\'') => {
                it.next();
            }
            b'\\' if in_double => {
                it.next();
            }
            b'\'' if !in_double && (in_single || inicio_de_escalar(&bytes[..i], true)) => in_single = !in_single,
            b'"' if !in_single && (in_double || inicio_de_escalar(&bytes[..i], true)) => in_double = !in_double,
            b'[' | b'{' if !in_single && !in_double => depth += 1,
            b']' | b'}' if !in_single && !in_double => depth -= 1,
            b',' if depth == 0 && !in_single && !in_double => {
                out.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = &inner[start..];
    if !last.trim().is_empty() || !out.is_empty() {
        out.push(last);
    }
    out
}

/// Remove aspas simples/duplas externas, se houver.
/// Tira as aspas das pontas e resolve os escapes como o YAML: `''` na simples; na dupla `\\`,
/// `\"`, `\/`, `\n`, `\t`, `\r`, `\0`. Escape desconhecido fica como está, para o caminho Windows entre aspas (`"base\characters\x.ent"`,
/// erro no yaml-cpp) continuar inteiro.
fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        return s[1..s.len() - 1].replace("''", "'");
    }
    if !(s.len() >= 2 && s.starts_with('"') && s.ends_with('"')) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s[1..s.len() - 1].chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('/') => out.push('/'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some(outro) => {
                out.push('\\');
                out.push(outro);
            }
            None => out.push('\\'),
        }
    }
    out
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

    /// Um `.yaml` gravado com BOM: o nome do 1º record sai sem ele.
    #[test]
    fn bom_fica_fora_do_nome_do_primeiro_record() {
        let n = parse("\u{feff}Character.X_Photomode_Puppet:\n  $type: Character\n").unwrap();
        assert_eq!(map(&n)[0].0, "Character.X_Photomode_Puppet");
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

    /// Escape dentro de aspas mantém o trecho aberto até a aspa que fecha: `''` dentro de aspa simples e `\` com o byte seguinte
    /// dentro de aspa dupla, como no YAML; o `unquote` resolve o escape no valor. Cobre o `#`, o
    /// `:` e a vírgula depois do escape, no valor, no item, no flow e na chave.
    #[test]
    fn escape_dentro_de_aspas_mantem_o_trecho_aberto() {
        let seq = |itens: &[&str]| Node { aspas: false, tag: None, kind: Kind::Seq(itens.iter().map(|s| Node::scalar(*s)).collect()) };
        let mapa = |k: &str, v: &str| Node { aspas: false, tag: None, kind: Kind::Map(vec![(k.to_string(), Node::scalar(v))]) };
        assert_eq!(parse("a: 'it''s # x'\n").unwrap().get("a").unwrap().as_str(), Some("it's # x"));
        assert_eq!(parse("l:\n  - 'It''s: here'\n").unwrap().get("l"), Some(&seq(&["It's: here"])));
        assert_eq!(parse("tags: [ 'V''s car, red', Quest ]\n").unwrap().get("tags"), Some(&seq(&["V's car, red", "Quest"])));
        assert_eq!(parse("a: {k: 'a''b, c'}\n").unwrap().get("a"), Some(&mapa("k", "a'b, c")));
        assert_eq!(parse("'it''s: x': y\n").unwrap().get("it's: x").unwrap().as_str(), Some("y"));

        assert_eq!(parse("a: \"x\\\" # y\"\n").unwrap().get("a").unwrap().as_str(), Some("x\" # y"));
        assert_eq!(parse("l: [\"a\\\", b\", c]\n").unwrap().get("l"), Some(&seq(&["a\", b", "c"])));
        assert_eq!(parse("k: {a: \"x\\\", y\"}\n").unwrap().get("k"), Some(&mapa("a", "x\", y")));
        assert_eq!(parse("\"a\\\": b\": c\n").unwrap().get("a\": b").unwrap().as_str(), Some("c"));
        // `\\` é um escape inteiro: a aspa seguinte fecha o trecho e o ` #` depois dela é comentário.
        assert_eq!(parse("a: \"x\\\\\" # y\n").unwrap().get("a").unwrap().as_str(), Some("x\\"));
    }

    #[test]
    fn comentario_e_lockey_hash() {
        let n = parse("# comentário\nItems.X:\n  displayName: LocKey#45371  # inline\n  range: 30\n").unwrap();
        let rec = &map(&n)[0].1;
        assert_eq!(rec.get("displayName").unwrap().as_str(), Some("LocKey#45371"));
        assert_eq!(rec.get("range").unwrap().as_str(), Some("30"));
    }

    /// Âncora e alias valem no tweak como no `.xl` (decisão do dono, F7-23): no valor de chave e de item, com o bloco embaixo, e
    /// com a tag antes ou depois da âncora. O alias copia o nó com a tag.
    #[test]
    fn ancora_e_alias_como_valor_inteiro() {
        let n = parse("x: &anchor 1\ny: *anchor\n").unwrap();
        assert_eq!(n.get("y"), Some(&Node::scalar("1")));

        let n = parse("a: &M\n  k: v\nb: *M\nl:\n  - &I item\n  - *I\nt: !append &T X\nu: &U !remove Y\nv: *T\nw: *U\n").unwrap();
        assert_eq!(n.get("a"), n.get("b"));
        assert_eq!(n.get("b").unwrap().get("k").unwrap().as_str(), Some("v"));
        assert_eq!(n.get("l").unwrap().as_seq().unwrap(), &[Node::scalar("item"), Node::scalar("item")]);
        assert_eq!(n.get("v"), Some(&Node { aspas: false, tag: Some("!append".into()), kind: Kind::Scalar("X".into()) }));
        assert_eq!(n.get("w"), Some(&Node { aspas: false, tag: Some("!remove".into()), kind: Kind::Scalar("Y".into()) }));

        let e = parse("a: *nada\n").unwrap_err();
        assert!(e.starts_with("linha 1: alias *nada sem âncora antes"), "{e}");
    }

    #[test]
    fn limites_do_subconjunto_yaml() {
        // Os limites que o doc do módulo declara: `---` inicial derruba o documento, alias dentro de `[..]` fica literal.
        assert!(parse("---\na: b\n").is_err());
        let y = parse("a: &x v\nb: [*x]\nc: *x\n").unwrap();
        assert_eq!(y.get("b"), Some(&Node { aspas: false, tag: None, kind: Kind::Seq(vec![Node::scalar("*x")]) }));
        assert_eq!(y.get("c"), Some(&Node::scalar("v")));
    }

    /// O resto da lista de fora do subconjunto: cada um dá erro com a linha, em vez de um parse silencioso errado.
    #[test]
    fn fora_do_subconjunto_da_erro_com_a_linha() {
        for (src, inicio) in [
            ("a: &x\n  k: v\nb:\n  <<: *x\n", "linha 4: merge `<<`"),
            ("l:\n  - <<: *x\n", "linha 2: merge `<<`"),
            ("a: 1\n...\n", "linha 2: marcador de documento"),
            ("a: |\n  texto\n", "linha 1: valor `|`"),
            ("a:\n  > texto\n", "linha 2: escalar de bloco"),
            ("a:\n\tb: 1\n", "linha 2: tab na indentação"),
            ("a:\n  &x\n  k: v\n", "linha 2: âncora ou alias sozinho"),
            ("a: [x, y\n", "linha 1: sequência inline malformada"),
            ("a: {x: 1\n", "linha 1: mapa inline malformado"),
            ("- a: 1\n    b: 2\n", "linha 2: indentação inesperada"),
        ] {
            let e = parse(src).unwrap_err();
            assert!(e.starts_with(inicio), "{src:?} → {e}");
        }
        // Tab que não é indentação passa: comentário e linha em branco com tab, e o tab depois do `:`.
        let n = parse("a:\tb\n\t# nota\n \t\nc: d\n").unwrap();
        assert_eq!((n.get("a").unwrap().as_str(), n.get("c").unwrap().as_str()), (Some("b"), Some("d")));
    }

    /// A sequência no recuo da própria chave, a forma do mapa de herança e de `.xl` escritos à mão, também dentro do
    /// mapa de um item. Depois de `-` vazio ela não vale: o item é vazio e o `-` seguinte é o próximo item.
    #[test]
    fn sequencia_no_recuo_da_chave() {
        let n = parse("Items.A:\n- Items.B\n- Items.C\nItems.D:\n- Items.E\n").unwrap();
        assert_eq!(map(&n).len(), 2);
        assert_eq!(n.get("Items.A").unwrap().as_seq().unwrap(), &[Node::scalar("Items.B"), Node::scalar("Items.C")]);
        assert_eq!(n.get("Items.D").unwrap().as_seq().unwrap(), &[Node::scalar("Items.E")]);

        let n = parse("l:\n  - path: a\n    tags:\n    - x\n  - path: b\n").unwrap();
        let l = n.get("l").unwrap().as_seq().unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].get("tags").unwrap().as_seq().unwrap(), &[Node::scalar("x")]);
        assert_eq!(l[1].get("path").unwrap().as_str(), Some("b"));

        let n = parse("l:\n  -\n  - b\n").unwrap();
        assert_eq!(n.get("l").unwrap().as_seq().unwrap(), &[Node::scalar(""), Node::scalar("b")]);
    }

    /// O item-mapa numa linha só é mapa (`- $type: X` vira record inline), as chaves seguintes se alinham com a coluna do texto
    /// depois do `- `, e a tag antes da 1ª chave vale para o mapa.
    #[test]
    fn item_mapa_alinhado_pela_coluna_do_texto() {
        let n = parse("l:\n  - $type: X\n  -   a: 1\n      b: 2\n  - !append $type: Y\n    value: 3\n").unwrap();
        let l = n.get("l").unwrap().as_seq().unwrap();
        assert_eq!(l[0].get("$type").unwrap().as_str(), Some("X"));
        assert_eq!(map(&l[1]).iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(l[2].tag.as_deref(), Some("!append"));
        assert_eq!(l[2].get("value").unwrap().as_str(), Some("3"));
        // `- - x`: sequência dentro do item
        let n = parse("- - x\n  - y\n").unwrap();
        assert_eq!(n.as_seq().unwrap()[0].as_seq().unwrap(), &[Node::scalar("x"), Node::scalar("y")]);
    }

    #[test]
    fn virgula_final_nao_abre_item() {
        let n = parse("a: [x, y, ]\nb: [ , ]\nc: []\nd: [[1, 2,], z]\n").unwrap();
        assert_eq!(n.get("a").unwrap().as_seq().unwrap(), &[Node::scalar("x"), Node::scalar("y")]);
        assert_eq!(n.get("b").unwrap().as_seq().unwrap(), &[Node::scalar(""), Node::scalar("")]);
        assert!(n.get("c").unwrap().as_seq().unwrap().is_empty());
        let d = n.get("d").unwrap().as_seq().unwrap();
        assert_eq!(d[0].as_seq().unwrap(), &[Node::scalar("1"), Node::scalar("2")]);
    }

    /// A aspa só abre trecho no começo do escalar (começo, depois de `: `, `- `, tag ou âncora e, dentro de `[..]`, de `[` ou
    /// `,`). No meio do escalar ela é letra, mesmo depois de espaço, e o `#` depois de espaço é comentário, como no yaml-cpp.
    #[test]
    fn aspas_so_no_comeco_do_escalar() {
        let n = parse("a: V's car # nota\nb: 'x # y'\n\"c: d\": e\nl:\n  - \"k: v\"\n  - it's: ok\n").unwrap();
        assert_eq!(n.get("a").unwrap().as_str(), Some("V's car"));
        assert_eq!(n.get("b").unwrap().as_str(), Some("x # y"));
        assert_eq!(n.get("c: d").unwrap().as_str(), Some("e"));
        let l = n.get("l").unwrap().as_seq().unwrap();
        assert_eq!(l[0], Node::scalar("k: v"));
        assert_eq!(l[1].get("it's").unwrap().as_str(), Some("ok"));

        // Aspa depois de espaço no meio do escalar: letra.
        let n = parse(
            "a: Rock 'n roll # nota\nb: x \"y # z\"\nc: [Rock 'n roll, b.csv]\nd: [a 'b, c]\ne: x, 'y # z'\nr:\n  link:\n    a 'b: c\n",
        )
        .unwrap();
        assert_eq!(n.get("a").unwrap().as_str(), Some("Rock 'n roll"));
        assert_eq!(n.get("b").unwrap().as_str(), Some("x \"y"));
        assert_eq!(n.get("c").unwrap().as_seq().unwrap(), &[Node::scalar("Rock 'n roll"), Node::scalar("b.csv")]);
        assert_eq!(n.get("d").unwrap().as_seq().unwrap(), &[Node::scalar("a 'b"), Node::scalar("c")]);
        assert_eq!(n.get("e").unwrap().as_str(), Some("x, 'y"));
        assert_eq!(n.get("r").unwrap().get("link").unwrap().get("a 'b").unwrap().as_str(), Some("c"));

        // Aspa no começo do escalar: abre depois de `- `, de tag e de âncora e, no flow, de `[` e `,`.
        let n = parse("l:\n  - 'x # y'\n  - !t \"u # v\"\nm: &a 'p # q'\nf: [a, 'b # c', !t 'd, e']\ng: ['h, i']\n").unwrap();
        let l = n.get("l").unwrap().as_seq().unwrap();
        assert_eq!(l[0], Node::scalar("x # y"));
        assert_eq!(l[1], Node { aspas: false, tag: Some("!t".into()), kind: Kind::Scalar("u # v".into()) });
        assert_eq!(n.get("m").unwrap().as_str(), Some("p # q"));
        let f = n.get("f").unwrap().as_seq().unwrap();
        assert_eq!(f[..2], [Node::scalar("a"), Node::scalar("b # c")]);
        assert_eq!(f[2], Node { aspas: false, tag: Some("!t".into()), kind: Kind::Scalar("d, e".into()) });
        assert_eq!(n.get("g").unwrap().as_seq().unwrap(), &[Node::scalar("h, i")]);
    }
}
