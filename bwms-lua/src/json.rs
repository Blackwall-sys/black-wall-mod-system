//! Global `json` que os mods Lua recebem: `encode`/`decode` (RFC 8259 mais o contrato dos mods: vírgula final tolerada no `decode`).
//!
//! Os dois lados recorrem na pilha nativa, um quadro por nível de aninhamento; `LIMITE_DE_ANINHAMENTO` corta antes de a recursão passar da
//! folga de pilha que o executor reserva para código nativo (`exec.rs`, `FOLGA_DA_PILHA`). No `encode` o mesmo limite pega a tabela que
//! contém a si mesma por um caminho longo; a referência circular direta é pega antes, pela pilha de tabelas abertas.

use crate::padrao::numero_em_texto;
use crate::{Lua, LuaString, Table, Value};
use std::rc::Rc;

/// Configuração de mod real fica abaixo de 20 níveis; 200 deixa margem e cabe na folga de pilha mesmo em build de depuração.
const LIMITE_DE_ANINHAMENTO: usize = 200;

pub(crate) fn instalar(lua: &Lua, g: &Table) {
    let t = lua.tabela_nova();
    let enc = lua.funcao_rust(Box::new(|lua, a| {
        let mut saida = Vec::new();
        codificar(a.first().unwrap_or(&Value::Nil), &mut Vec::new(), &mut saida).map_err(|m| lua.erro(format!("json.encode: {m}")))?;
        Ok(vec![Value::String(LuaString(Rc::from(saida)))])
    }));
    let dec = lua.funcao_rust(Box::new(|lua, a| match a.first() {
        Some(Value::String(s)) => Ok(vec![decodificar(lua, &s.0).map_err(|m| lua.erro(format!("json.decode: {m}")))?]),
        v => Err(lua.erro(format!("json.decode espera texto, recebeu {}", v.map_or("nada", Value::nome_do_tipo)))),
    }));
    t.d.borrow_mut().por(Value::texto("encode"), Value::Function(enc));
    t.d.borrow_mut().por(Value::texto("decode"), Value::Function(dec));
    g.d.borrow_mut().por(Value::texto("json"), Value::Table(t));
}

fn codificar_texto(s: &[u8], saida: &mut Vec<u8>) {
    saida.push(b'"');
    for &b in s {
        match b {
            b'"' => saida.extend_from_slice(b"\\\""),
            b'\\' => saida.extend_from_slice(b"\\\\"),
            0x08 => saida.extend_from_slice(b"\\b"),
            0x0C => saida.extend_from_slice(b"\\f"),
            b'\n' => saida.extend_from_slice(b"\\n"),
            b'\r' => saida.extend_from_slice(b"\\r"),
            b'\t' => saida.extend_from_slice(b"\\t"),
            0x00..=0x1F | 0x7F => saida.extend_from_slice(format!("\\u{b:04x}").as_bytes()),
            // Bytes ≥ 0x80 passam crus: UTF-8 sai como veio.
            _ => saida.push(b),
        }
    }
    saida.push(b'"');
}

/// Como a tabela vira JSON: lista `[...]` ou objeto `{...}`. Os pares são copiados para fora do `RefCell` antes de descer nos filhos.
enum Forma {
    Lista(Vec<Value>),
    Objeto(Vec<(Rc<[u8]>, Value)>),
}

fn forma_da_tabela(t: &Table) -> Result<Forma, String> {
    let d = t.d.borrow();
    let mut numeros: Vec<(f64, Value)> = Vec::new();
    let mut textos: Vec<(Rc<[u8]>, Value)> = Vec::new();
    for (i, v) in d.arr.iter().enumerate() {
        if !v.is_nil() {
            numeros.push(((i + 1) as f64, v.clone()));
        }
    }
    for (k, v) in &d.itens {
        if v.is_nil() {
            continue;
        }
        match k {
            Value::Number(n) => numeros.push((*n, v.clone())),
            Value::Integer(i) => numeros.push((*i as f64, v.clone())),
            Value::String(s) => textos.push((s.0.clone(), v.clone())),
            outro => return Err(format!("chave de tabela do tipo {} não vira JSON", outro.nome_do_tipo())),
        }
    }
    if !numeros.is_empty() && !textos.is_empty() {
        return Err("tabela mistura chaves de lista e de objeto".into());
    }
    if !textos.is_empty() {
        return Ok(Forma::Objeto(textos));
    }
    let n = numeros.len();
    let mut lista = vec![Value::Nil; n];
    for (k, v) in numeros {
        if k.fract() != 0.0 || k < 1.0 || k > n as f64 {
            return Err("tabela com chave numérica fora de 1..n (buraco ou número não inteiro)".into());
        }
        lista[k as usize - 1] = v;
    }
    Ok(Forma::Lista(lista))
}

/// `abertas`: tabelas no caminho da raiz até aqui. Tabela repetida em ramos diferentes é permitida; dentro de si mesma, não.
fn codificar(v: &Value, abertas: &mut Vec<*const ()>, saida: &mut Vec<u8>) -> Result<(), String> {
    match v {
        Value::Nil => saida.extend_from_slice(b"null"),
        Value::Boolean(b) => saida.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Integer(i) => saida.extend_from_slice(i.to_string().as_bytes()),
        Value::Number(n) if !n.is_finite() => return Err(format!("número {} não existe em JSON", numero_em_texto(*n))),
        Value::Number(n) => saida.extend_from_slice(numero_em_texto(*n).as_bytes()),
        Value::String(s) => codificar_texto(&s.0, saida),
        Value::Table(t) => {
            let id = Rc::as_ptr(&t.d) as *const ();
            if abertas.contains(&id) {
                return Err("tabela com referência circular".into());
            }
            if abertas.len() >= LIMITE_DE_ANINHAMENTO {
                return Err(format!("aninhamento passa de {LIMITE_DE_ANINHAMENTO} níveis"));
            }
            let forma = forma_da_tabela(t)?;
            abertas.push(id);
            match forma {
                Forma::Lista(itens) => {
                    saida.push(b'[');
                    for (i, item) in itens.iter().enumerate() {
                        if i > 0 {
                            saida.push(b',');
                        }
                        codificar(item, abertas, saida)?;
                    }
                    saida.push(b']');
                }
                Forma::Objeto(pares) => {
                    saida.push(b'{');
                    for (i, (k, item)) in pares.iter().enumerate() {
                        if i > 0 {
                            saida.push(b',');
                        }
                        codificar_texto(k, saida);
                        saida.push(b':');
                        codificar(item, abertas, saida)?;
                    }
                    saida.push(b'}');
                }
            }
            abertas.pop();
        }
        outro => return Err(format!("valor do tipo {} não vira JSON", outro.nome_do_tipo())),
    }
    Ok(())
}

struct Leitor<'a> {
    b: &'a [u8],
    i: usize,
    lua: &'a Lua,
    nivel: usize,
}

fn decodificar(lua: &Lua, b: &[u8]) -> Result<Value, String> {
    let mut l = Leitor { b, i: 0, lua, nivel: 0 };
    let r = l.documento();
    r.map_err(|m| l.com_lugar(&m))
}

impl Leitor<'_> {
    /// Linha e coluna (1-based, coluna em bytes) do ponto em que a leitura parou.
    fn com_lugar(&self, m: &str) -> String {
        let antes = &self.b[..self.i.min(self.b.len())];
        let linha = 1 + antes.iter().filter(|&&c| c == b'\n').count();
        let coluna = 1 + antes.iter().rev().take_while(|&&c| c != b'\n').count();
        format!("{m} (linha {linha}, coluna {coluna})")
    }

    fn olhar(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn pular_espacos(&mut self) {
        while matches!(self.olhar(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn documento(&mut self) -> Result<Value, String> {
        self.pular_espacos();
        let v = self.valor()?;
        self.pular_espacos();
        if self.i < self.b.len() {
            return Err("sobra conteúdo depois do valor".into());
        }
        Ok(v)
    }

    fn valor(&mut self) -> Result<Value, String> {
        match self.olhar() {
            None => Err("fim inesperado do texto".into()),
            Some(b'{') => self.objeto(),
            Some(b'[') => self.lista(),
            Some(b'"') => self.texto().map(|t| Value::String(LuaString(Rc::from(t)))),
            Some(b't') => self.literal(b"true", Value::Boolean(true)),
            Some(b'f') => self.literal(b"false", Value::Boolean(false)),
            Some(b'n') => self.literal(b"null", Value::Nil),
            Some(b'-' | b'0'..=b'9') => self.numero(),
            Some(c) => Err(format!("caractere inesperado '{}'", char::from(c).escape_default())),
        }
    }

    fn literal(&mut self, palavra: &[u8], v: Value) -> Result<Value, String> {
        if !self.b[self.i..].starts_with(palavra) {
            return Err(format!("literal incompleto, esperava '{}'", String::from_utf8_lossy(palavra)));
        }
        self.i += palavra.len();
        Ok(v)
    }

    fn digitos(&mut self) -> usize {
        let ini = self.i;
        while matches!(self.olhar(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        self.i - ini
    }

    /// Gramática da RFC: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`.
    fn numero(&mut self) -> Result<Value, String> {
        let ini = self.i;
        if self.olhar() == Some(b'-') {
            self.i += 1;
        }
        match self.olhar() {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                self.digitos();
            }
            _ => return Err("número sem dígito".into()),
        }
        if self.olhar() == Some(b'.') {
            self.i += 1;
            if self.digitos() == 0 {
                return Err("número sem dígito depois do '.'".into());
            }
        }
        if matches!(self.olhar(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.olhar(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.digitos() == 0 {
                return Err("número sem dígito no expoente".into());
            }
        }
        // A fatia só tem ASCII da gramática acima, que o `parse` de `f64` aceita inteira.
        let t = std::str::from_utf8(&self.b[ini..self.i]).map_err(|_| "número inválido".to_string())?;
        t.parse::<f64>().map(Value::Number).map_err(|_| "número inválido".into())
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let Some(h) = self.b.get(self.i..self.i + 4) else { return Err("'\\u' pede 4 dígitos hexadecimais".into()) };
        let t = std::str::from_utf8(h).map_err(|_| "'\\u' pede 4 dígitos hexadecimais".to_string())?;
        if !t.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("'\\u' pede 4 dígitos hexadecimais".into());
        }
        self.i += 4;
        u32::from_str_radix(t, 16).map_err(|_| "'\\u' pede 4 dígitos hexadecimais".into())
    }

    /// `\uXXXX` já depois do `\u`; par substituto vira um code point só. Saída sempre em UTF-8.
    fn escape_unicode(&mut self, saida: &mut Vec<u8>) -> Result<(), String> {
        let alto = self.hex4()?;
        let cp = match alto {
            0xD800..=0xDBFF => {
                if !self.b[self.i..].starts_with(b"\\u") {
                    return Err("substituto alto em '\\u' sem o par baixo".into());
                }
                self.i += 2;
                let baixo = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&baixo) {
                    return Err("substituto alto em '\\u' sem o par baixo".into());
                }
                0x10000 + ((alto - 0xD800) << 10) + (baixo - 0xDC00)
            }
            0xDC00..=0xDFFF => return Err("substituto baixo em '\\u' sem o par alto".into()),
            c => c,
        };
        let c = char::from_u32(cp).ok_or("code point inválido em '\\u'")?;
        let mut buf = [0u8; 4];
        saida.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        Ok(())
    }

    fn texto(&mut self) -> Result<Vec<u8>, String> {
        self.i += 1;
        let mut saida = Vec::new();
        loop {
            let Some(c) = self.olhar() else { return Err("texto sem aspas de fechamento".into()) };
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(saida);
                }
                b'\\' => {
                    self.i += 1;
                    let Some(e) = self.olhar() else { return Err("texto sem aspas de fechamento".into()) };
                    self.i += 1;
                    match e {
                        b'"' => saida.push(b'"'),
                        b'\\' => saida.push(b'\\'),
                        b'/' => saida.push(b'/'),
                        b'b' => saida.push(0x08),
                        b'f' => saida.push(0x0C),
                        b'n' => saida.push(b'\n'),
                        b'r' => saida.push(b'\r'),
                        b't' => saida.push(b'\t'),
                        b'u' => self.escape_unicode(&mut saida)?,
                        outro => {
                            self.i -= 1;
                            return Err(format!("escape inválido '\\{}'", char::from(outro).escape_default()));
                        }
                    }
                }
                0x00..=0x1F => return Err("byte de controle cru dentro de texto (use escape)".into()),
                _ => {
                    saida.push(c);
                    self.i += 1;
                }
            }
        }
    }

    fn entrar(&mut self) -> Result<(), String> {
        self.nivel += 1;
        if self.nivel > LIMITE_DE_ANINHAMENTO {
            return Err(format!("aninhamento passa de {LIMITE_DE_ANINHAMENTO} níveis"));
        }
        self.i += 1;
        self.pular_espacos();
        Ok(())
    }

    /// Depois de um elemento: `true` = acabou (fecho consumido). A vírgula final antes do fecho é aceita: config de mod é editada à mão.
    fn separador(&mut self, fecho: u8) -> Result<bool, String> {
        self.pular_espacos();
        match self.olhar() {
            Some(b',') => {
                self.i += 1;
                self.pular_espacos();
                if self.olhar() == Some(fecho) {
                    self.i += 1;
                    return Ok(true);
                }
                Ok(false)
            }
            Some(c) if c == fecho => {
                self.i += 1;
                Ok(true)
            }
            None => Err("fim inesperado do texto".into()),
            Some(_) => Err(format!("esperava ',' ou '{}'", char::from(fecho))),
        }
    }

    fn lista(&mut self) -> Result<Value, String> {
        self.entrar()?;
        let t = self.lua.tabela_nova();
        if self.olhar() == Some(b']') {
            self.i += 1;
        } else {
            let mut indice = 0.0;
            loop {
                let v = self.valor()?;
                indice += 1.0;
                // `null` deixa o buraco: o elemento seguinte ainda vai para o seu índice.
                if !v.is_nil() {
                    t.d.borrow_mut().por(Value::Number(indice), v);
                }
                if self.separador(b']')? {
                    break;
                }
            }
        }
        self.nivel -= 1;
        Ok(Value::Table(t))
    }

    fn objeto(&mut self) -> Result<Value, String> {
        self.entrar()?;
        let t = self.lua.tabela_nova();
        if self.olhar() == Some(b'}') {
            self.i += 1;
        } else {
            loop {
                if self.olhar() != Some(b'"') {
                    return Err("chave de objeto tem de vir entre aspas".into());
                }
                let k = self.texto()?;
                self.pular_espacos();
                if self.olhar() != Some(b':') {
                    return Err("esperava ':' depois da chave".into());
                }
                self.i += 1;
                self.pular_espacos();
                let v = self.valor()?;
                t.d.borrow_mut().por(Value::String(LuaString(Rc::from(k))), v);
                if self.separador(b'}')? {
                    break;
                }
            }
        }
        self.nivel -= 1;
        Ok(Value::Table(t))
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn rodar(lua: &Lua, codigo: &str) -> crate::Result<Value> {
        lua.load(format!("return {codigo}")).call::<Value>(())
    }

    fn texto(lua: &Lua, codigo: &str) -> String {
        match rodar(lua, codigo) {
            Ok(Value::String(s)) => String::from_utf8_lossy(&s.0).into_owned(),
            outro => panic!("{codigo}: esperava texto, veio {outro:?}"),
        }
    }

    #[test]
    fn encode_valores() {
        let lua = Lua::new();
        assert_eq!(texto(&lua, "json.encode({1, 2, 3})"), "[1,2,3]");
        assert_eq!(texto(&lua, "json.encode({})"), "[]");
        assert_eq!(texto(&lua, "json.encode({a = {}})"), r#"{"a":[]}"#);
        assert_eq!(texto(&lua, r#"json.encode({a = {b = {1, "x"}}})"#), r#"{"a":{"b":[1,"x"]}}"#);
        assert_eq!(texto(&lua, r#"json.encode("he said \"hi\"\n")"#), r#""he said \"hi\"\n""#);
        assert_eq!(texto(&lua, r#"json.encode("\1\127/\\")"#), r#""\u0001\u007f/\\""#);
        assert_eq!(texto(&lua, r#"json.encode("\t\r\b\f")"#), r#""\t\r\b\f""#);
        assert_eq!(texto(&lua, "json.encode(nil)"), "null");
        assert_eq!(texto(&lua, "json.encode(1.5)"), "1.5");
        assert_eq!(texto(&lua, "json.encode(-3)"), "-3");
        assert_eq!(texto(&lua, "json.encode(1e20)"), "1e+20");
        assert_eq!(texto(&lua, "json.encode('ação')"), "\"ação\"");
    }

    #[test]
    fn encode_erros() {
        let lua = Lua::new();
        for c in [
            "json.encode(0/0)",
            "json.encode(1/0)",
            "(function() local t = {} t[1] = 1 t[3] = 3 return json.encode(t) end)()",
            "json.encode({1, 2, x = 3})",
            "json.encode({[true] = 1})",
            "json.encode({[1.5] = 1})",
            "(function() local t = {} t.self = t return json.encode(t) end)()",
            "(function() local t = {} local u = {t} t[1] = u return json.encode(t) end)()",
            "json.encode(print)",
        ] {
            assert!(rodar(&lua, c).is_err(), "{c} deveria falhar");
        }
        // Mesma tabela em dois ramos não é ciclo.
        assert_eq!(texto(&lua, "(function() local f = {1} return json.encode({f, f}) end)()"), "[[1],[1]]");
    }

    #[test]
    fn decode_valores() {
        let lua = Lua::new();
        assert_eq!(texto(&lua, r#"json.encode(json.decode(' \t\r\n[ 1 , 2 ]\n'))"#), "[1,2]");
        assert_eq!(texto(&lua, r#"json.decode('"\\u00e7"')"#), "ç");
        assert_eq!(texto(&lua, r#"json.decode('"\\ud83d\\ude00"')"#), "😀");
        assert_eq!(texto(&lua, r#"json.decode('"\\"\\\\\\/\\b\\f\\n\\r\\t"')"#), "\"\\/\u{8}\u{c}\n\r\t");
        assert!(matches!(rodar(&lua, "json.decode('1e3')"), Ok(Value::Integer(1000))));
        assert!(matches!(rodar(&lua, "json.decode('-0.5E-2')"), Ok(Value::Number(n)) if n == -0.005));
        assert!(matches!(rodar(&lua, "json.decode('null')"), Ok(Value::Nil)));
        assert!(matches!(rodar(&lua, r#"json.decode('{"a":{"b":{"c":[1,[2,[3]]]}}}').a.b.c[2][2][1]"#), Ok(Value::Integer(3))));
        assert!(matches!(rodar(&lua, "json.decode('[1,null,3]')[3]"), Ok(Value::Integer(3))));
        assert_eq!(texto(&lua, "json.encode(json.decode('[1,]'))"), "[1]");
        assert_eq!(texto(&lua, r#"json.encode(json.decode('{"a":1,}'))"#), r#"{"a":1}"#);
    }

    #[test]
    fn decode_erros() {
        let lua = Lua::new();
        for c in [
            "'[1,2'", "'{\"a\" 1}'", "'{a:1}'", "'[1] x'", "'true false'", "''", "'tru'", "'\"a' .. '\\n' .. 'b\"'", "'\"\\\\x\"'",
            "'\"\\\\ud83d\"'", "'\"\\\\ude00\"'", "'\"abc'", "123", "'[,]'", "'[1,,]'", "'-'", "'1.'", "'1e'",
        ] {
            assert!(rodar(&lua, &format!("json.decode({c})")).is_err(), "json.decode({c}) deveria falhar");
        }
        let fundo = format!("json.decode('{}')", "[".repeat(LIMITE_DE_ANINHAMENTO + 1));
        assert!(rodar(&lua, &fundo).is_err());
    }

    #[test]
    fn decode_erro_diz_linha_e_coluna() {
        let lua = Lua::new();
        let Err(e) = rodar(&lua, "json.decode('{\\n  \"a\": tru\\n}')") else { panic!("deveria falhar") };
        assert!(e.to_string().contains("linha 2, coluna 8"), "{e}");
    }
}
