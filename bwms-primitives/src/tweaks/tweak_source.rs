//! Parser do formato-texto NATIVO `.tweak` do TweakDB (item `#48` do catálogo exaustivo)
//! — a 3ª via de autoria de mod, distinta do YAML já coberto
//! por `yaml.rs`/`tweaks/mod.rs`. Produz a MESMA `Vec<Op>` que o pipeline YAML já consome
//! (`operacoes::Op`/`EditOp`) — reusa `apply_ops_runtime` existente sem duplicar nada runtime.
//!
//! Mapeamento semântico:
//! - grupo `{name, base}` (sem inline) → `base` não-vazio vira `Op::Clone{record:name,base}`
//!   (equivalente a `$base` do YAML); sem `base`, edita o record já existente direto (mesmo
//!   comportamento de um record YAML top-level sem `$base`/`$type`).
//! - flat `{name, operation, values, isArray}` → `group.name` vira o `flat` de 1+ `Op::Edit`.
//!   `Assign` sempre 1 op; `Append`/`Remove` com valor-array (`tags[] += [A,B,C];`) viram VÁRIOS
//!   `Op::Edit{EditOp::Append(X)}` (1 por elemento) — o `.tweak` real permite múltiplos elementos
//!   por statement (distinto do YAML, que tageia item-por-item); componível 1:1 na `EditOp`
//!   existente sem precisar de variante nova.
//! - valor escalar (bool/number/string) → texto cru (sufixo `f` de float REMOVIDO, já
//!   que o resto do pipeline espera número puro parseável); struct (`Vector3` etc.) → componentes
//!   NUMÉRICOS comma-joined (mesmo formato `"f,f,…"` que `struct_map_to_scalar`/`encode_value`
//!   já esperam do lado YAML).
//!
//! Record em linha (`{ … }` como valor ou item de array, com `: Base` opcional depois) vira record com nome sintético
//! (o mesmo dos records inline do YAML); sem base, a classe vai como `@Pai.prop` para o runtime resolver pela propriedade. `package` prefixa o
//! nome dos grupos e, com os `using`, dá os candidatos da base sem ponto (`?Base|pacote.Base`), que o runtime resolve (tipo de record,
//! record existente).
//!
//! Fora do escopo (erro claro, não parse silencioso errado — mesma filosofia de `tweaks/mod.rs`):
//! **pacotes de schema (`RTDB`) / query (`Query`)** — nenhum mod real usa `.tweak`
//! ainda (achado da triagem, 2026-08-09: "não havia nenhum arquivo `.tweak` real
//! disponível pra testar"), então a prioridade foi cobrir o caso comum (mod regular: `package`
//! opcional, `using` opcional, groups com/sem herança, flats escalar/struct/array com `=`/`+=`/
//! `-=`) com fidelidade total, em vez de arriscar mapear semântica de schema/query sem exemplo
//! real pra validar contra (mesma disciplina já aplicada a `$dlc`/`$game`/`$instances`).

use std::collections::HashMap;

use crate::operacoes::{EditOp, Op};

/// Entrada principal: parseia uma fonte `.tweak` inteira em uma lista de `Op`, na MESMA
/// representação que `tweaks::interpret` produz a partir de YAML.
pub fn parse(src: &str) -> Result<Vec<Op>, String> {
    parse_com_origem(src, "")
}

/// Como [`parse`], com o caminho do arquivo: ele entra no nome sintético dos records em linha.
pub fn parse_com_origem(src: &str, origem: &str) -> Result<Vec<Op>, String> {
    let mut p = P::new(src.strip_prefix('\u{feff}').unwrap_or(src));
    p.origem = origem.to_string();
    p.skip_ws();
    if p.eat_keyword("package") {
        p.skip_ws1()?;
        let pacote = p.parse_package_id()?;
        if pacote == "RTDB" || pacote == "Query" {
            return Err(format!(
                "package {pacote}: é pacote de schema/consulta do TweakDB, não de mod; troque `package {pacote}` pelo nome do seu mod"
            ));
        }
        p.package = Some(pacote);
        p.skip_ws();
    }
    if p.eat_keyword("using") {
        p.skip_ws1()?;
        let primeiro = p.parse_package_id()?;
        p.usings.push(primeiro);
        p.skip_ws();
        while p.eat_char(b',') {
            p.skip_ws();
            let pacote = p.parse_package_id()?;
            p.usings.push(pacote);
            p.skip_ws();
        }
    }
    let mut ops = Vec::new();
    // Flat solto é do `package`; entra depois de todos os grupos. Sem `package` não tem record dono: é lido (erro de sintaxe vale) e fica fora.
    let mut soltos = Vec::new();
    loop {
        p.skip_ws();
        if p.at_end() {
            break;
        }
        if p.e_grupo() {
            p.parse_group(&mut ops)?;
            continue;
        }
        let pai = p.package.clone().unwrap_or_default();
        let mut do_flat = Vec::new();
        p.parse_flat(&pai, &mut do_flat)?;
        if p.package.is_some() {
            soltos.append(&mut do_flat);
        }
    }
    ops.append(&mut soltos);
    Ok(ops)
}

#[derive(Clone, Copy, PartialEq)]
enum FlatOp {
    Assign,
    Append,
    Remove,
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
    /// Tipo de flat do motor declarado no flat em leitura (`int` → `Int32`, `fk<X>[]` → `array:TweakDBID`): com ele o flat nasce se não
    /// existir (o tipo declarado vale pra prop que o record não tem).
    tipo: Option<String>,
    /// `package` do arquivo: prefixo do nome dos grupos.
    package: Option<String>,
    /// Os `using` do cabeçalho.
    usings: Vec<String>,
    /// Caminho do arquivo, no nome dos records em linha.
    origem: String,
    /// Nomes de item de array em linha já dados neste arquivo (`tweaks::nome_de_record_em_linha`).
    itens_em_linha: HashMap<String, u32>,
}

fn is_name_first(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}
fn is_name_other(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl<'a> P<'a> {
    fn new(s: &'a str) -> Self {
        P { s: s.as_bytes(), i: 0, tipo: None, package: None, usings: Vec::new(), origem: String::new(), itens_em_linha: HashMap::new() }
    }
    fn at_end(&self) -> bool {
        self.i >= self.s.len()
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn rest_starts_with(&self, pat: &[u8]) -> bool {
        self.s[self.i..].starts_with(pat)
    }

    fn skip_ws(&mut self) {
        loop {
            while matches!(self.peek(), Some(b) if b.is_ascii_whitespace()) {
                self.i += 1;
            }
            if self.rest_starts_with(b"//") {
                while !self.at_end() && self.peek() != Some(b'\n') {
                    self.i += 1;
                }
                continue;
            }
            if self.rest_starts_with(b"/*") {
                self.i += 2;
                while !self.at_end() && !self.rest_starts_with(b"*/") {
                    self.i += 1;
                }
                self.i = (self.i + 2).min(self.s.len());
                continue;
            }
            break;
        }
    }

    /// Exige AO MENOS 1 espaço/comentário (separador obrigatório entre keyword e token, ex.
    /// `package Foo` — sem isso `packageFoo` seria mal-interpretado).
    fn skip_ws1(&mut self) -> Result<(), String> {
        let before = self.i;
        self.skip_ws();
        if self.i == before {
            return Err(format!("esperava espaço na posição {before}"));
        }
        Ok(())
    }

    fn eat_char(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect_char(&mut self, c: u8) -> Result<(), String> {
        if self.eat_char(c) {
            Ok(())
        } else {
            Err(format!("esperava '{}' na posição {}", c as char, self.i))
        }
    }

    fn eat_str(&mut self, s: &str) -> bool {
        if self.rest_starts_with(s.as_bytes()) {
            self.i += s.len();
            true
        } else {
            false
        }
    }

    /// Keyword só casa se NÃO for prefixo de um identificador maior (`package` não casa em
    /// `packageFoo`).
    fn eat_keyword(&mut self, kw: &str) -> bool {
        if self.rest_starts_with(kw.as_bytes()) {
            let next = self.s.get(self.i + kw.len()).copied();
            if !matches!(next, Some(b) if is_name_other(b)) {
                self.i += kw.len();
                return true;
            }
        }
        false
    }

    fn parse_ident(&mut self) -> Result<String, String> {
        let start = self.i;
        match self.peek() {
            Some(b) if is_name_first(b) => self.i += 1,
            _ => return Err(format!("esperava identificador na posição {start}")),
        }
        while matches!(self.peek(), Some(b) if is_name_other(b)) {
            self.i += 1;
        }
        Ok(std::str::from_utf8(&self.s[start..self.i]).unwrap().to_string())
    }

    /// `path`/`group_path`/`group_id` da gramática real são, na prática, todos "identificador
    /// com pontos permitidos no meio" — nunca precisamos distinguir package-qualificado de
    /// não-qualificado pra tradução em `Op` (o nome vira o `record`/`base` como está, cru).
    fn parse_path(&mut self) -> Result<String, String> {
        let start = self.i;
        match self.peek() {
            Some(b) if is_name_first(b) => self.i += 1,
            _ => return Err(format!("esperava path na posição {start}")),
        }
        while matches!(self.peek(), Some(b) if is_name_other(b) || b == b'.') {
            self.i += 1;
        }
        Ok(std::str::from_utf8(&self.s[start..self.i]).unwrap().to_string())
    }

    /// `0x` + 9-10 dígitos hex (`hash` da gramática real).
    fn parse_hash(&mut self) -> Option<String> {
        if self.rest_starts_with(b"0x") {
            let start = self.i;
            self.i += 2;
            let hstart = self.i;
            while matches!(self.peek(), Some(b) if b.is_ascii_hexdigit()) {
                self.i += 1;
            }
            let n = self.i - hstart;
            if (9..=10).contains(&n) {
                return Some(std::str::from_utf8(&self.s[start..self.i]).unwrap().to_string());
            }
            self.i = start;
        }
        None
    }

    /// `package_id = sor<hash, name>` (sem pontos).
    fn parse_package_id(&mut self) -> Result<String, String> {
        if let Some(h) = self.parse_hash() {
            return Ok(h);
        }
        self.parse_ident()
    }

    /// `group_id`/`group_base`/`group_path` = `sor<hash, path>` (com pontos permitidos).
    fn parse_group_id(&mut self) -> Result<String, String> {
        if let Some(h) = self.parse_hash() {
            return Ok(h);
        }
        self.parse_path()
    }

    /// `tags = star<tag, _>`, `tag = [ _ tag_name _ ]` — coletadas e DESCARTADAS (sem via de
    /// aplicação em runtime ainda pra tags arbitrárias de record/flat, distinto das tags de
    /// OPERAÇÃO do YAML que viram `EditOp`).
    fn parse_tags(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            if !self.eat_char(b'[') {
                return Ok(());
            }
            self.skip_ws();
            self.parse_ident()?;
            self.skip_ws();
            self.expect_char(b']')?;
        }
    }

    /// O que vem é grupo (`tags nome [: base] {`)? Senão é flat (`tags [tipo] nome =`). Só olha, não consome.
    fn e_grupo(&mut self) -> bool {
        let inicio = self.i;
        let grupo = (|| {
            self.parse_tags().ok()?;
            self.skip_ws();
            self.parse_group_id().ok()?;
            self.skip_ws();
            if self.eat_char(b':') {
                self.skip_ws();
                self.parse_group_id().ok()?;
                self.skip_ws();
            }
            (self.peek() == Some(b'{')).then_some(())
        })()
        .is_some();
        self.i = inicio;
        grupo
    }

    fn parse_group(&mut self, ops: &mut Vec<Op>) -> Result<(), String> {
        self.parse_tags()?;
        self.skip_ws();
        let curto = self.parse_group_id()?;
        let name = match &self.package {
            Some(pacote) => format!("{pacote}.{curto}"),
            None => curto,
        };
        self.skip_ws();
        let base = if self.eat_char(b':') {
            self.skip_ws();
            let b = self.parse_group_id()?;
            self.skip_ws();
            Some(b)
        } else {
            None
        };
        self.expect_char(b'{')?;
        if let Some(b) = &base {
            ops.push(Op::Clone { record: name.clone(), base: self.base_a_resolver(b) });
        }
        loop {
            self.skip_ws();
            if self.eat_char(b'}') {
                break;
            }
            if self.at_end() {
                return Err(format!("'{name}': grupo sem fechar '}}'"));
            }
            self.parse_flat(&name, ops)?;
        }
        Ok(())
    }

    /// O texto do `base` de `Op::Clone` para a base `base` escrita em `X : Base` (grupo ou record em linha). Nome com ponto é record
    /// completo; sem ponto (ou tipo `_Record`) vira os candidatos `?base|pacote.base|using.base…`, que o runtime resolve em ordem (tipo de
    /// record, depois record existente).
    fn base_a_resolver(&self, base: &str) -> String {
        if base.contains('.') && !base.ends_with("_Record") {
            return base.to_string();
        }
        let mut candidatos = format!("?{base}");
        for pacote in self.package.iter().chain(&self.usings) {
            candidatos.push_str(&format!("|{pacote}.{base}"));
        }
        candidatos
    }

    /// O nome do record em linha valor do flat `pai` (`indice` = -1) ou item `indice` (≥ 0) de array, com a base escrita depois do `}`
    /// (`None` sem base). A base entra como escrita, sem [`P::base_a_resolver`]: é o que dá o mesmo nome que o `.yaml`.
    fn nome_em_linha(&mut self, pai: &str, base: Option<&str>, indice: i32) -> String {
        super::nome_de_record_em_linha(&self.origem, pai, base.unwrap_or("@"), indice, &mut self.itens_em_linha)
    }

    /// Record em linha (`{ … }` com `: Base` opcional depois), valor do flat `pai` ou item `indice` (≥ 0) de array: as operações dele e o
    /// nome sintético ([`P::nome_em_linha`]). Sem base, a classe vai como `@pai` (o tipo da propriedade, resolvido no runtime).
    fn parse_em_linha(&mut self, pai: &str, indice: i32, ops: &mut Vec<Op>) -> Result<String, String> {
        let inicio = self.i;
        self.expect_char(b'{')?;
        let mut nivel = 1;
        while nivel > 0 {
            match self.peek() {
                None => return Err(format!("'{pai}': record em linha sem fechar '}}'")),
                Some(b'"') => {
                    self.parse_string_literal()?;
                    continue;
                }
                Some(b'/') if matches!(self.s.get(self.i + 1), Some(b'/') | Some(b'*')) => {
                    self.skip_ws();
                    continue;
                }
                Some(b'{') => nivel += 1,
                Some(b'}') => nivel -= 1,
                _ => {}
            }
            self.i += 1;
        }
        let depois_do_corpo = self.i;
        self.skip_ws();
        let base = if self.eat_char(b':') {
            self.skip_ws();
            Some(self.parse_group_id()?)
        } else {
            self.i = depois_do_corpo;
            None
        };
        let fim = self.i;
        let nome = self.nome_em_linha(pai, base.as_deref(), indice);
        match &base {
            Some(b) => ops.push(Op::Clone { record: nome.clone(), base: self.base_a_resolver(b) }),
            None => ops.push(Op::Create { record: nome.clone(), class: format!("@{pai}") }),
        }
        // O corpo, agora com o nome: os flats dele são `nome.prop`.
        let tipo_de_fora = self.tipo.take();
        self.i = inicio + 1;
        loop {
            self.skip_ws();
            if self.eat_char(b'}') {
                break;
            }
            self.parse_flat(&nome, ops)?;
        }
        self.tipo = tipo_de_fora;
        self.i = fim;
        Ok(nome)
    }

    /// `fk<Name>` (foreign-key type): o valor é `TweakDBID`.
    fn try_parse_fk(&mut self) -> Option<()> {
        let save = self.i;
        if !self.eat_str("fk<") {
            return None;
        }
        self.skip_ws();
        if self.parse_ident().is_err() {
            self.i = save;
            return None;
        }
        self.skip_ws();
        if !self.eat_char(b'>') {
            self.i = save;
            return None;
        }
        Some(())
    }

    /// `flat_stmt` começa com `tags, opt<flat_type>, flat_name` — `flat_type` (se presente) é
    /// SEMPRE seguido de espaço obrigatório antes do nome real (gramática: `flat_type = seq<
    /// type_expr, space>`). Distingue "token1 é o TIPO, token2 é o NOME" de "token1 já é o NOME
    /// (sem tipo declarado)" por LOOKAHEAD: se depois de token1 (+ opcional `[]`) + espaço vier
    /// OUTRO identificador, token1 era tipo; senão token1 já era o nome. Devolve o tipo de flat do motor e o nome.
    fn parse_flat_name(&mut self) -> Result<(Option<String>, String), String> {
        let save = self.i;
        // tenta consumir um token de TIPO candidato (fk<...> ou identificador simples).
        let consumed_fk = self.try_parse_fk().is_some();
        if !consumed_fk {
            if self.parse_ident().is_err() {
                self.i = save;
                return Err(format!("'{save}': esperava nome de flat"));
            }
        }
        let token1 = std::str::from_utf8(&self.s[save..self.i]).unwrap().to_string();
        // sufixo de array opcional, só faz sentido se token1 for tipo.
        let after_token1 = self.i;
        let e_array = self.eat_str("[]");
        let ws_before = self.i;
        self.skip_ws();
        let had_ws = self.i > ws_before;
        if had_ws && matches!(self.peek(), Some(b) if is_name_first(b)) {
            // token1 (+ opcional []) ERA o tipo — o nome de verdade vem agora.
            let base = if consumed_fk { Some("TweakDBID") } else { tipo_de_flat(&token1) };
            let tipo = base.map(|b| if e_array { format!("array:{b}") } else { b.to_string() });
            return Ok((tipo, self.parse_ident()?));
        }
        // token1 já era o nome — reverte qualquer `[]`/espaço que tenha comido de propósito.
        self.i = after_token1;
        Ok((None, token1))
    }

    fn parse_flat(&mut self, group: &str, ops: &mut Vec<Op>) -> Result<(), String> {
        self.parse_tags()?;
        self.skip_ws();
        let (tipo, name) = self.parse_flat_name()?;
        self.tipo = tipo;
        self.skip_ws();
        let op = if self.eat_str("+=") {
            FlatOp::Append
        } else if self.eat_str("-=") {
            FlatOp::Remove
        } else if self.eat_char(b'=') {
            FlatOp::Assign
        } else {
            return Err(format!("'{group}.{name}': esperava '='/'+='/'-=' na posição {}", self.i));
        };
        self.skip_ws();
        let full = format!("{group}.{name}");
        self.parse_flat_value(&full, op, ops)?;
        self.skip_ws();
        self.expect_char(b';')?;
        self.tipo = None;
        Ok(())
    }

    /// `flat_value = sor<scalar_expr, struct_expr, array_expr, inline_expr>`.
    fn parse_flat_value(&mut self, flat: &str, op: FlatOp, ops: &mut Vec<Op>) -> Result<(), String> {
        match self.peek() {
            Some(b'(') => {
                let v = self.parse_struct_expr()?;
                push_scalar_op(ops, flat, op, v, &self.tipo);
                Ok(())
            }
            Some(b'[') => self.parse_array_expr(flat, op, ops),
            Some(b'{') => {
                let nome = self.parse_em_linha(flat, -1, ops)?;
                push_scalar_op(ops, flat, op, nome, &self.tipo);
                Ok(())
            }
            _ => {
                let v = self.parse_scalar_expr()?;
                push_scalar_op(ops, flat, op, v, &self.tipo);
                Ok(())
            }
        }
    }

    /// `scalar_expr = sor<scalar_bool, scalar_number, scalar_string>`. Devolve o valor já
    /// FORMATADO pro resto do pipeline (`f` de float removido; string sem aspas).
    fn parse_scalar_expr(&mut self) -> Result<String, String> {
        if self.eat_str("true") {
            return Ok("true".into());
        }
        if self.eat_str("false") {
            return Ok("false".into());
        }
        if self.peek() == Some(b'"') {
            return self.parse_string_literal();
        }
        self.parse_number_literal()
    }

    fn parse_string_literal(&mut self) -> Result<String, String> {
        let start = self.i;
        self.expect_char(b'"')?;
        let content_start = self.i;
        while !self.at_end() && self.peek() != Some(b'"') {
            self.i += 1;
        }
        if self.at_end() {
            return Err(format!("string sem fechar (iniciada na posição {start})"));
        }
        let content = std::str::from_utf8(&self.s[content_start..self.i]).unwrap().to_string();
        self.i += 1; // consome a aspa final
        Ok(content)
    }

    /// `scalar_number = opt<'-'>, (.digits | digits(.digits)?), opt<'f'>` — devolve SEM o
    /// sufixo `f` (o resto do pipeline espera número puro parseável em `f32`/`i32`).
    fn parse_number_literal(&mut self) -> Result<String, String> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        let digits_start = self.i;
        if self.peek() == Some(b'.') {
            self.i += 1;
            let d0 = self.i;
            while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                self.i += 1;
            }
            if self.i == d0 {
                self.i = start;
                return Err(format!("número inválido na posição {start}"));
            }
        } else {
            let d0 = self.i;
            while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                self.i += 1;
            }
            if self.i == d0 {
                self.i = start;
                return Err(format!("número inválido na posição {start}"));
            }
            if self.peek() == Some(b'.') {
                self.i += 1;
                while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                    self.i += 1;
                }
            }
        }
        let _ = digits_start;
        let text_end = self.i;
        let text = std::str::from_utf8(&self.s[start..text_end]).unwrap().to_string();
        self.eat_char(b'f'); // sufixo de float, descartado (não faz parte do valor numérico)
        Ok(text)
    }

    /// `struct_expr = ( num, num [, num [, num]] )` — 2 a 4 componentes numéricos. Devolve
    /// `(a,b,c)` sem espaço, na ordem da fonte (a de memória do struct); o [`na_ordem_do_texto`] põe na do pipeline.
    fn parse_struct_expr(&mut self) -> Result<String, String> {
        self.expect_char(b'(')?;
        self.skip_ws();
        let mut parts = Vec::new();
        parts.push(self.parse_number_literal()?);
        self.skip_ws();
        self.expect_char(b',')?;
        self.skip_ws();
        parts.push(self.parse_number_literal()?);
        loop {
            self.skip_ws();
            if !self.eat_char(b',') {
                break;
            }
            self.skip_ws();
            parts.push(self.parse_number_literal()?);
            if parts.len() >= 4 {
                break;
            }
        }
        self.skip_ws();
        self.expect_char(b')')?;
        Ok(format!("({})", parts.join(",")))
    }

    /// `array_expr = [ item (, item)* ]`, `item = sor<scalar_expr, struct_expr, inline_expr>`.
    /// `Assign` empacota tudo num `EditOp::Assign("[a, b, c]")` (formato já usado por
    /// `EditOp::Assign` de array no lado YAML); `Append`/`Remove` viram 1 `Op::Edit` POR
    /// ELEMENTO (mesmo efeito líquido, sem precisar de variante nova de `EditOp` "append-many").
    fn parse_array_expr(&mut self, flat: &str, op: FlatOp, ops: &mut Vec<Op>) -> Result<(), String> {
        self.expect_char(b'[')?;
        self.skip_ws();
        let mut items = Vec::new();
        while self.peek() != Some(b']') {
            if self.at_end() {
                return Err(format!("'{flat}': array sem fechar ']'"));
            }
            let v = match self.peek() {
                Some(b'(') => self.parse_struct_expr()?,
                Some(b'{') => self.parse_em_linha(flat, items.len() as i32, ops)?,
                _ => self.parse_scalar_expr()?,
            };
            items.push(v);
            self.skip_ws();
            if self.eat_char(b',') {
                self.skip_ws();
            }
        }
        self.expect_char(b']')?;
        let elem = self.tipo.as_deref().map(|t| t.strip_prefix("array:").unwrap_or(t).to_string());
        let items: Vec<String> = items.into_iter().map(|v| na_ordem_do_texto(elem.as_deref(), v)).collect();
        match op {
            FlatOp::Assign => ops.push(op_tipada(flat, &self.tipo, EditOp::Assign(crate::operacoes::lista_em_texto(&items)))),
            FlatOp::Append => {
                for v in items {
                    ops.push(op_tipada(flat, &self.tipo, EditOp::Append(v)));
                }
            }
            FlatOp::Remove => {
                for v in items {
                    ops.push(op_tipada(flat, &self.tipo, EditOp::Remove(v)));
                }
            }
        }
        Ok(())
    }
}

fn push_scalar_op(ops: &mut Vec<Op>, flat: &str, op: FlatOp, value: String, tipo: &Option<String>) {
    let elem = tipo.as_deref().map(|t| t.strip_prefix("array:").unwrap_or(t));
    let value = na_ordem_do_texto(elem, value);
    let edit = match op {
        FlatOp::Assign => EditOp::Assign(value),
        FlatOp::Append => EditOp::Append(value),
        FlatOp::Remove => EditOp::Remove(value),
    };
    ops.push(op_tipada(flat, tipo, edit));
}

/// Com tipo declarado o flat nasce se não existir; sem tipo, só edita.
fn op_tipada(flat: &str, tipo: &Option<String>, op: EditOp) -> Op {
    match tipo {
        Some(t) => Op::Criar { flat: flat.to_string(), tipo: t.clone(), op },
        None => Op::Edit { flat: flat.to_string(), op },
    }
}

/// O tipo de flat do motor de cada palavra de tipo do `.tweak`.
fn tipo_de_flat(palavra: &str) -> Option<&'static str> {
    Some(match palavra {
        "int" => "Int32",
        "float" => "Float",
        "bool" => "Bool",
        "string" => "String",
        "CName" => "CName",
        "ResRef" => "raRef:CResource",
        "LocKey" => "gamedataLocKeyWrapper",
        "Quaternion" => "Quaternion",
        "EulerAngles" => "EulerAngles",
        "Vector3" => "Vector3",
        "Vector2" => "Vector2",
        "Color" => "Color",
        _ => return None,
    })
}

/// Tupla do `.tweak` com o tipo declarado: na ordem do texto do pipeline ([`super::tupla_em_posicional`]). Sem tipo fica `(a,b,c)`, para o
/// flat vivo resolver.
fn na_ordem_do_texto(elem: Option<&str>, v: String) -> String {
    match elem {
        Some(t) => super::tupla_em_posicional(t, &v).unwrap_or(v),
        None => v,
    }
}

/// Um grupo de 1º nível de uma fonte `.tweak` do jogo: nome, base declarada (`Nome : Base { }`) e se é `[Debug]`.
pub struct GrupoDaFonte {
    pub nome: String,
    pub base: Option<String>,
    pub debug: bool,
}

/// Os grupos de 1º nível de um arquivo de fonte `.tweak` (as fontes do TweakDB que o REDmod traz em `tools/redmod/tweaks`), com o `package`
/// e os `using` do arquivo: o que o mapa de herança precisa ([`crate::operacoes::MapaDeHeranca::das_fontes`]). Os corpos dos grupos (flats,
/// inline) são pulados sem interpretar.
pub struct FonteDeGrupos {
    pub pacote: Option<String>,
    pub usings: Vec<String>,
    pub grupos: Vec<GrupoDaFonte>,
}

pub fn grupos_da_fonte(src: &str) -> Result<FonteDeGrupos, String> {
    let mut p = P::new(src);
    p.skip_ws();
    let mut pacote = None;
    let mut usings = Vec::new();
    if p.eat_keyword("package") {
        p.skip_ws1()?;
        pacote = Some(p.parse_package_id()?);
        p.skip_ws();
    }
    if p.eat_keyword("using") {
        p.skip_ws1()?;
        usings.push(p.parse_package_id()?);
        p.skip_ws();
        while p.eat_char(b',') {
            p.skip_ws();
            usings.push(p.parse_package_id()?);
            p.skip_ws();
        }
    }
    let mut grupos = Vec::new();
    loop {
        p.skip_ws();
        if p.at_end() {
            break;
        }
        if !p.e_grupo() {
            p.parse_flat("", &mut Vec::new())?;
            continue;
        }
        let mut debug = false;
        loop {
            p.skip_ws();
            if !p.eat_char(b'[') {
                break;
            }
            p.skip_ws();
            debug |= p.parse_ident()? == "Debug";
            p.skip_ws();
            p.expect_char(b']')?;
        }
        p.skip_ws();
        let nome = p.parse_group_id()?;
        p.skip_ws();
        let base = if p.eat_char(b':') {
            p.skip_ws();
            let b = p.parse_group_id()?;
            p.skip_ws();
            Some(b)
        } else {
            None
        };
        p.expect_char(b'{')?;
        p.pular_bloco().map_err(|e| format!("'{nome}': {e}"))?;
        grupos.push(GrupoDaFonte { nome, base, debug });
    }
    Ok(FonteDeGrupos { pacote, usings, grupos })
}

impl P<'_> {
    /// Anda até o `}` que fecha o `{` já consumido, passando por cima de texto entre aspas e comentários.
    fn pular_bloco(&mut self) -> Result<(), String> {
        let mut nivel = 1usize;
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err("grupo sem fechar '}'".into()),
                Some(b'"') => {
                    self.parse_string_literal()?;
                }
                Some(b'{') => {
                    nivel += 1;
                    self.i += 1;
                }
                Some(b'}') => {
                    self.i += 1;
                    nivel -= 1;
                    if nivel == 0 {
                        return Ok(());
                    }
                }
                Some(_) => self.i += 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Formata a lista de ops pra string legível, mesmo padrão de teste já usado em
    /// `tweaks/mod.rs` (evita precisar de `Debug`/`PartialEq` em `Op`/`EditOp`, que não derivam).
    fn fmt(ops: &[Op]) -> Vec<String> {
        ops.iter()
            .map(|o| match o {
                Op::Clone { record, base } => format!("clone {record} <- {base}"),
                Op::Create { record, class } => format!("create {record} : {class}"),
                Op::Edit { flat, op } | Op::Criar { flat, op, .. } => format!("edit {flat} {}", fmt_editop(op)),
            })
            .collect()
    }
    fn fmt_editop(op: &EditOp) -> String {
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
        fmt(&parse(src).expect("parse falhou"))
    }
    /// `Op` não deriva `Debug` (mesma convenção de `tweaks/mod.rs`) — `.unwrap_err()` exigiria
    /// `Vec<Op>: Debug` pro branch Ok, então extrai o erro manualmente.
    fn err_of(src: &str) -> String {
        match parse(src) {
            Ok(_) => panic!("esperava erro, parse teve sucesso"),
            Err(e) => e,
        }
    }

    #[test]
    fn group_com_heranca_e_flats_escalares() {
        let s = run(
            "package MyMod\n\nItems.MyWeapon : Items.Preset_Lexington_Default {\n    int damage = 50;\n    float attackRadius = 3.5;\n    bool isMelee = false;\n}\n",
        );
        assert_eq!(
            s,
            vec![
                // `package` prefixa o nome do grupo.
                "clone MyMod.Items.MyWeapon <- Items.Preset_Lexington_Default",
                "edit MyMod.Items.MyWeapon.damage = 50",
                "edit MyMod.Items.MyWeapon.attackRadius = 3.5",
                "edit MyMod.Items.MyWeapon.isMelee = false",
            ]
        );
    }

    #[test]
    fn bom_fica_fora_do_primeiro_identificador() {
        assert_eq!(run("\u{feff}Items.Existing {\n    damage = 99;\n}\n"), vec!["edit Items.Existing.damage = 99"]);
    }

    #[test]
    fn group_sem_heranca_edita_record_existente() {
        let s = run("Items.Existing {\n    damage = 99;\n}\n");
        assert_eq!(s, vec!["edit Items.Existing.damage = 99"]);
    }

    #[test]
    fn flat_sem_tipo_declarado() {
        // "damage = 99;" — sem tipo, `damage` já É o nome (achado do lookahead).
        let s = run("Items.A { damage = 1; }");
        assert_eq!(s, vec!["edit Items.A.damage = 1"]);
    }

    #[test]
    fn string_e_bool_e_float_com_sufixo() {
        let s = run("Items.A {\n  string label = \"Arma X\";\n  bool active = true;\n  float scale = 1.5f;\n}\n");
        assert_eq!(
            s,
            vec!["edit Items.A.label = Arma X", "edit Items.A.active = true", "edit Items.A.scale = 1.5"]
        );
    }

    #[test]
    fn numero_negativo_e_sem_parte_fracionaria() {
        let s = run("Items.A {\n  int offset = -3;\n  float half = .5;\n}\n");
        assert_eq!(s, vec!["edit Items.A.offset = -3", "edit Items.A.half = .5"]);
    }

    #[test]
    fn struct_vector3() {
        let s = run("Items.A {\n  Vector3 pos = (1.0, 2.5, -3.0);\n}\n");
        assert_eq!(s, vec!["edit Items.A.pos = 1.0,2.5,-3.0"]);
    }

    /// Tupla sem tipo declarado (`world_map.tweak` da CDPR: `rotationMin = ( 0.f, -85.f, -360.f );` num `EulerAngles`): fica na ordem da
    /// fonte, com parênteses, para o flat vivo decidir; o `tweakdb.bin` da 2.5 confirma `Roll, Pitch, Yaw` = a ordem da fonte.
    #[test]
    fn tupla_sem_tipo_fica_para_o_flat_vivo() {
        assert_eq!(run("WorldMap.X {\n  rotationMin = ( 0.f, -85.f, -360.f );\n}\n"), ["edit WorldMap.X.rotationMin = (0.,-85.,-360.)"]);
        assert_eq!(run("X {\n  a = [ (1, 2, 3) ];\n}\n"), ["edit X.a = [\"(1,2,3)\"]"]);
        assert_eq!(super::super::tupla_em_posicional("EulerAngles", "(0.,-85.,-360.)").as_deref(), Some("-85.,-360.,0."));
        assert_eq!(super::super::tupla_em_posicional("Vector3", "(0.,-85.,-360.)").as_deref(), Some("0.,-85.,-360."));
        assert_eq!(super::super::tupla_em_posicional("Vector3", "0.,-85.,-360."), None);
    }

    #[test]
    fn array_assign_vira_1_op_bracket() {
        let s = run("Items.A {\n  CName[] tags = [\"Tag1\", \"Tag2\", \"Tag3\"];\n}\n");
        assert_eq!(s, vec!["edit Items.A.tags = [Tag1, Tag2, Tag3]"]);
    }

    #[test]
    fn array_append_vira_1_op_por_elemento() {
        let s = run("Items.A {\n  tags += [\"Tag1\", \"Tag2\"];\n}\n");
        assert_eq!(s, vec!["edit Items.A.tags append Tag1", "edit Items.A.tags append Tag2"]);
    }

    #[test]
    fn array_remove_vira_1_op_por_elemento() {
        let s = run("Items.A {\n  tags -= [\"Tag1\", \"Tag2\"];\n}\n");
        assert_eq!(s, vec!["edit Items.A.tags remove Tag1", "edit Items.A.tags remove Tag2"]);
    }

    #[test]
    fn append_de_valor_escalar_unico_sem_colchetes() {
        let s = run("Items.A {\n  tags += \"SoloTag\";\n}\n");
        assert_eq!(s, vec!["edit Items.A.tags append SoloTag"]);
    }

    #[test]
    fn fk_type_e_using_statement_ignorados_sem_quebrar() {
        let s = run("package Mod\nusing Core, Items\n\nItems.A : Items.B {\n  fk<gamedataItem_Record> baseItem = \"Items.C\";\n}\n");
        assert_eq!(s, vec!["clone Mod.Items.A <- Items.B", "edit Mod.Items.A.baseItem = Items.C"]);
    }

    #[test]
    fn tags_de_record_e_de_flat_sao_ignoradas_sem_quebrar() {
        let s = run("[Vehicle] [Melee]\nItems.A : Items.B {\n  [Hidden] damage = 1;\n}\n");
        assert_eq!(s, vec!["clone Items.A <- Items.B", "edit Items.A.damage = 1"]);
    }

    #[test]
    fn comentarios_linha_e_bloco_ignorados() {
        let s = run("// comentário de linha\nItems.A { /* bloco\nmulti-linha */ damage = 1; // fim\n}\n");
        assert_eq!(s, vec!["edit Items.A.damage = 1"]);
    }

    #[test]
    fn multiplos_groups_no_mesmo_arquivo() {
        let s = run("Items.A : Items.X { damage = 1; }\nItems.B : Items.Y { damage = 2; }\n");
        assert_eq!(
            s,
            vec!["clone Items.A <- Items.X", "edit Items.A.damage = 1", "clone Items.B <- Items.Y", "edit Items.B.damage = 2"]
        );
    }

    #[test]
    fn hash_como_nome_de_group_e_flat_value() {
        // `hash = 0x + 9-10 hex digits` — forma alternativa de referenciar record por TweakDBID cru.
        let s = run("0x123456789 : Items.B { damage = 1; }\n");
        assert_eq!(s, vec!["clone 0x123456789 <- Items.B", "edit 0x123456789.damage = 1"]);
    }

    #[test]
    /// Record em linha: como valor, sem tipo (o runtime resolve pela propriedade), e em
    /// array, com `: Tipo`; o nome sintético é o mesmo para a mesma origem.
    fn record_em_linha_como_valor_e_em_array() {
        let s = run("Items.A {\n  baseItem = { damage = 1; };\n}\n");
        assert!(s[0].starts_with("create Items.A.baseItem$") && s[0].ends_with(" : @Items.A.baseItem"), "{s:?}");
        let nome = s[0]["create ".len()..s[0].find(" : ").unwrap()].to_string();
        assert_eq!(s[1..], [format!("edit {nome}.damage = 1"), format!("edit Items.A.baseItem = {nome}")]);
        let s = run("Items.B : Items.C {\n  statModifiers += [ { value = 3.0; } : ConstantStatModifier, { min = 0.0; } : RandomStatModifier ];\n}\n");
        assert_eq!(s[0], "clone Items.B <- Items.C");
        assert!(s[1].starts_with("clone Items.B.statModifiers$") && s[1].ends_with(" <- ?ConstantStatModifier"), "{s:?}");
        assert!(s[3].ends_with(" <- ?RandomStatModifier"), "{s:?}");
        assert!(s[5].starts_with("edit Items.B.statModifiers append Items.B.statModifiers$"), "{s:?}");
        assert_eq!(fmt(&parse_com_origem("X { y = { z = 1; }; }", "a.tweak").unwrap())[0], fmt(&parse_com_origem("X { y = { z = 1; }; }", "a.tweak").unwrap())[0]);
        assert_ne!(fmt(&parse_com_origem("X { y = { z = 1; }; }", "a.tweak").unwrap())[0], fmt(&parse_com_origem("X { y = { z = 1; }; }", "b.tweak").unwrap())[0]);
    }

    /// `package` e `using` dão os candidatos da base sem ponto (o package primeiro).
    #[test]
    fn package_e_using_na_base_sem_ponto() {
        let s = run("package Attacks\nusing AttackModifier\n\nGenjiReflect : BaseExplosion {\n  float x = 1;\n}\n");
        assert_eq!(s[0], "clone Attacks.GenjiReflect <- ?BaseExplosion|Attacks.BaseExplosion|AttackModifier.BaseExplosion");
        assert_eq!(run("Items.R : Clothing_Record { }\n")[0], "clone Items.R <- ?Clothing_Record");
    }

    /// Flat solto no arquivo (`camera/freecameraparams.tweak` da CDPR): filho do `package`, depois de todos os grupos; sem `package`, nada.
    #[test]
    fn flat_solto_no_package_vem_depois_dos_grupos() {
        let s = run("package FreeCameraParams\n\nfloat baseSpeed = 10.f;\nVector2 max = ( 3.f, 10.f );\nG { x = 1; }\n");
        assert_eq!(s, ["edit FreeCameraParams.G.x = 1", "edit FreeCameraParams.baseSpeed = 10.", "edit FreeCameraParams.max = 3.,10."]);
        assert!(run("float solto = 1.f;\nItems.A { d = 1; }\n").iter().all(|l| l.starts_with("edit Items.A")));
        let g = grupos_da_fonte("package P\nbool b = false;\nA : B { c = 1; }\n").unwrap();
        assert_eq!(g.grupos.iter().map(|x| x.nome.as_str()).collect::<Vec<_>>(), ["A"]);
    }

    #[test]
    fn pacote_schema_rtdb_da_erro_claro() {
        let err = err_of("package RTDB;\nItems.A { damage = 1; }\n");
        assert!(err.contains("RTDB"), "erro devia mencionar RTDB, veio: {err}");
    }

    #[test]
    fn pacote_query_da_erro_claro() {
        let err = err_of("package Query;\nItems.A { damage = 1; }\n");
        assert!(err.contains("Query"), "erro devia mencionar Query, veio: {err}");
    }

    #[test]
    fn grupo_sem_fechar_chave_da_erro() {
        assert!(parse("Items.A {\n  damage = 1;\n").is_err());
    }

    #[test]
    fn flat_sem_ponto_e_virgula_da_erro() {
        assert!(parse("Items.A {\n  damage = 1\n}\n").is_err());
    }

    /// Diferença 1 no `.tweak`: o tipo declarado vai na operação e o flat nasce com ele se o record não tiver a prop.
    #[test]
    fn tipo_declarado_vai_na_operacao() {
        let ops = parse("Items.A {\n  int bwmsNivel = 3;\n  float[] bwmsPesos = [1.5, 2];\n  fk<Item>[] bwmsIds += [\"Items.B\"];\n  EulerAngles giro = (10, 20, 30);\n  damage = 4;\n}\n").unwrap();
        let tipos: Vec<String> = ops
            .iter()
            .map(|o| match o {
                Op::Criar { flat, tipo, op: EditOp::Assign(v) | EditOp::Append(v) } => format!("{tipo} {flat} {v}"),
                Op::Edit { flat, .. } => format!("sem tipo {flat}"),
                _ => "outro".into(),
            })
            .collect();
        assert_eq!(
            tipos,
            [
                "Int32 Items.A.bwmsNivel 3",
                "array:Float Items.A.bwmsPesos [1.5, 2]",
                "array:TweakDBID Items.A.bwmsIds Items.B",
                "EulerAngles Items.A.giro 20,30,10",
                "sem tipo Items.A.damage",
            ]
        );
    }
}
