//! Global `json` que o CET entrega aos mods: `encode`/`decode` de tabelas, escrito a partir da especificação do JSON.

use crate::{Lua, Result, Table, Value};
use std::rc::Rc;

/// Tabela que se contém estoura a pilha nativa: o teto vira erro do mod.
const PROFUNDIDADE_MAXIMA: u32 = 120;

fn escapar(s: &[u8], saida: &mut Vec<u8>) {
    saida.push(b'"');
    for &c in s {
        match c {
            b'\\' => saida.extend_from_slice(b"\\\\"),
            b'"' => saida.extend_from_slice(b"\\\""),
            8 => saida.extend_from_slice(b"\\b"),
            12 => saida.extend_from_slice(b"\\f"),
            b'\n' => saida.extend_from_slice(b"\\n"),
            b'\r' => saida.extend_from_slice(b"\\r"),
            b'\t' => saida.extend_from_slice(b"\\t"),
            0..=31 | 127 => saida.extend_from_slice(format!("\\u{c:04x}").as_bytes()),
            c => saida.push(c),
        }
    }
    saida.push(b'"');
}

fn codificar_tabela(lua: &Lua, t: &Table, fundo: u32, saida: &mut Vec<u8>) -> Result<()> {
    let d = t.d.borrow();
    let tem_mapa = d.itens.iter().any(|(_, v)| !v.is_nil());
    if !d.arr.is_empty() || !tem_mapa {
        if tem_mapa {
            return Err(lua.erro("invalid table: mixed or invalid key types"));
        }
        if d.arr.iter().any(Value::is_nil) {
            return Err(lua.erro("invalid table: sparse array"));
        }
        saida.push(b'[');
        for (i, v) in d.arr.iter().enumerate() {
            if i > 0 {
                saida.push(b',');
            }
            codificar(lua, v, fundo + 1, saida)?;
        }
        saida.push(b']');
        return Ok(());
    }
    saida.push(b'{');
    let mut primeiro = true;
    for (k, v) in &d.itens {
        if v.is_nil() {
            continue;
        }
        let Value::String(k) = k else { return Err(lua.erro("invalid table: mixed or invalid key types")) };
        if !primeiro {
            saida.push(b',');
        }
        primeiro = false;
        escapar(&k.0, saida);
        saida.push(b':');
        codificar(lua, v, fundo + 1, saida)?;
    }
    saida.push(b'}');
    Ok(())
}

fn codificar(lua: &Lua, v: &Value, fundo: u32, saida: &mut Vec<u8>) -> Result<()> {
    if fundo >= PROFUNDIDADE_MAXIMA {
        return Err(lua.erro("circular reference"));
    }
    match v {
        Value::Nil => saida.extend_from_slice(b"null"),
        Value::Boolean(b) => saida.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) if !n.is_finite() => return Err(lua.erro(format!("unexpected number value '{}'", crate::padrao::numero_em_texto(*n)))),
        Value::Number(_) | Value::Integer(_) => saida.extend_from_slice(v.to_string()?.as_bytes()),
        Value::String(s) => escapar(&s.0, saida),
        Value::Table(t) => codificar_tabela(lua, t, fundo, saida)?,
        v => return Err(lua.erro(format!("unexpected type '{}'", v.nome_do_tipo()))),
    }
    Ok(())
}

struct Leitor<'a> {
    b: &'a [u8],
    i: usize,
}

impl Leitor<'_> {
    fn erro(&self, lua: &Lua, m: &str) -> crate::Error {
        let antes = &self.b[..self.i.min(self.b.len())];
        let linha = antes.iter().filter(|c| **c == b'\n').count() + 1;
        let coluna = self.i - antes.iter().rposition(|c| *c == b'\n').map_or(0, |p| p + 1) + 1;
        lua.erro(format!("{m} at line {linha} col {coluna}"))
    }

    fn vazio(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.i += 1;
        }
    }

    fn hex4(&mut self, lua: &Lua) -> Result<u32> {
        let h = self.b.get(self.i..self.i + 4).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u32::from_str_radix(h, 16).ok());
        let Some(n) = h else { return Err(self.erro(lua, "invalid unicode escape in string")) };
        self.i += 4;
        Ok(n)
    }

    fn texto(&mut self, lua: &Lua) -> Result<Vec<u8>> {
        self.i += 1;
        let mut s = Vec::new();
        loop {
            let Some(&c) = self.b.get(self.i) else { return Err(self.erro(lua, "expected closing quote for string")) };
            self.i += 1;
            match c {
                b'"' => return Ok(s),
                0..=31 => return Err(self.erro(lua, "control character in string")),
                b'\\' => {
                    let Some(&e) = self.b.get(self.i) else { return Err(self.erro(lua, "expected closing quote for string")) };
                    self.i += 1;
                    match e {
                        b'"' | b'\\' | b'/' => s.push(e),
                        b'b' => s.push(8),
                        b'f' => s.push(12),
                        b'n' => s.push(b'\n'),
                        b'r' => s.push(b'\r'),
                        b't' => s.push(b'\t'),
                        b'u' => {
                            let mut n = self.hex4(lua)?;
                            if (0xD800..0xDC00).contains(&n) && self.b[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let baixo = self.hex4(lua)?;
                                n = 0x10000 + ((n - 0xD800) << 10) + baixo.wrapping_sub(0xDC00);
                            }
                            let c = char::from_u32(n).ok_or_else(|| self.erro(lua, "invalid unicode codepoint"))?;
                            s.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        _ => return Err(self.erro(lua, "invalid escape char in string")),
                    }
                }
                c => s.push(c),
            }
        }
    }

    fn valor(&mut self, lua: &Lua, fundo: u32) -> Result<Value> {
        if fundo >= PROFUNDIDADE_MAXIMA {
            return Err(self.erro(lua, "too deeply nested"));
        }
        self.vazio();
        let Some(&c) = self.b.get(self.i) else { return Err(self.erro(lua, "unexpected end of input")) };
        match c {
            b'"' => {
                let s = self.texto(lua)?;
                Ok(Value::String(crate::LuaString(Rc::from(s))))
            }
            b'[' => {
                self.i += 1;
                let t = lua.tabela_nova();
                let mut n = 1.0;
                loop {
                    self.vazio();
                    if self.b.get(self.i) == Some(&b']') {
                        self.i += 1;
                        return Ok(Value::Table(t));
                    }
                    let v = self.valor(lua, fundo + 1)?;
                    t.d.borrow_mut().por(Value::Number(n), v);
                    n += 1.0;
                    self.vazio();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {}
                        _ => return Err(self.erro(lua, "expected ']' or ','")),
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let t = lua.tabela_nova();
                loop {
                    self.vazio();
                    match self.b.get(self.i) {
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Value::Table(t));
                        }
                        Some(b'"') => {}
                        _ => return Err(self.erro(lua, "expected string for key")),
                    }
                    let k = self.texto(lua)?;
                    self.vazio();
                    if self.b.get(self.i) != Some(&b':') {
                        return Err(self.erro(lua, "expected ':' after key"));
                    }
                    self.i += 1;
                    let v = self.valor(lua, fundo + 1)?;
                    t.d.borrow_mut().por(Value::String(crate::LuaString(Rc::from(k))), v);
                    self.vazio();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {}
                        _ => return Err(self.erro(lua, "expected '}' or ','")),
                    }
                }
            }
            _ => {
                for (lit, v) in [(&b"true"[..], Value::Boolean(true)), (b"false", Value::Boolean(false)), (b"null", Value::Nil)] {
                    if self.b[self.i..].starts_with(lit) {
                        self.i += lit.len();
                        return Ok(v);
                    }
                }
                let ini = self.i;
                while matches!(self.b.get(self.i), Some(b'0'..=b'9' | b'+' | b'-' | b'.' | b'e' | b'E')) {
                    self.i += 1;
                }
                match crate::padrao::texto_em_numero(&self.b[ini..self.i]).filter(|_| self.i > ini) {
                    Some(n) => Ok(Value::Number(n)),
                    None => Err(self.erro(lua, "unexpected character")),
                }
            }
        }
    }
}

pub(crate) fn instalar(lua: &Lua, g: &Table) {
    let t = lua.tabela_nova();
    let enc = lua.funcao_rust(Box::new(|lua, a| {
        let mut s = Vec::new();
        codificar(lua, a.first().unwrap_or(&Value::Nil), 0, &mut s)?;
        Ok(vec![Value::String(crate::LuaString(Rc::from(s)))])
    }));
    let dec = lua.funcao_rust(Box::new(|lua, a| {
        let Some(Value::String(s)) = a.first() else { return Err(lua.erro("expected argument of type string")) };
        let mut l = Leitor { b: &s.0, i: 0 };
        let v = l.valor(lua, 0)?;
        l.vazio();
        if l.i < l.b.len() {
            return Err(l.erro(lua, "trailing garbage"));
        }
        Ok(vec![v])
    }));
    t.d.borrow_mut().por(Value::texto("encode"), Value::Function(enc));
    t.d.borrow_mut().por(Value::texto("decode"), Value::Function(dec));
    g.d.borrow_mut().por(Value::texto("json"), Value::Table(t));
}
