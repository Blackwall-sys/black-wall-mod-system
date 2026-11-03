//! Biblioteca padrão que os mods de CET usam: base, `string`, `table`, `math`, `os`, `io`, `bit`, `package`.

use crate::padroes::{self, Captura, Casamento};
use crate::{Dados, Error, Function, Lua, LuaString, MultiValue, Result, Table, UserData, UserDataMethods, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Saida = Result<Vec<Value>>;

pub(crate) fn numero_cru(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

/// Número, aceitando texto numérico como a aritmética da linguagem.
pub(crate) fn numero_de(v: &Value) -> Option<f64> {
    match v {
        Value::String(s) => texto_em_numero(&s.0),
        v => numero_cru(v),
    }
}

pub(crate) fn iguais_cru(x: &Value, y: &Value) -> bool {
    if let (Some(a), Some(b)) = (numero_cru(x), numero_cru(y)) {
        return a == b;
    }
    match (x, y) {
        (Value::Nil, Value::Nil) => true,
        (Value::Boolean(a), Value::Boolean(b)) => a == b,
        (Value::String(a), Value::String(b)) => a.0 == b.0,
        (Value::Table(a), Value::Table(b)) => Rc::ptr_eq(&a.d, &b.d),
        (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(&a.d, &b.d),
        (Value::UserData(a), Value::UserData(b)) => Rc::ptr_eq(&a.0, &b.0),
        _ => false,
    }
}

pub(crate) fn numero_em_texto(n: f64) -> String {
    // Até 14 dígitos o `%.14g` escreve o inteiro cheio.
    if n.fract() == 0.0 && n.abs() < 1e14 {
        return (n as i64).to_string();
    }
    formato_g(n, 14, false, false)
}

pub(crate) fn texto_em_numero(b: &[u8]) -> Option<f64> {
    let t = std::str::from_utf8(b).ok()?.trim_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, sem_sinal) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t),
    };
    if let Some(h) = sem_sinal.strip_prefix("0x").or_else(|| sem_sinal.strip_prefix("0X")) {
        if h.is_empty() {
            return None;
        }
        let mut v = 0f64;
        for c in h.chars() {
            v = v * 16.0 + f64::from(c.to_digit(16)?);
        }
        return Some(if neg { -v } else { v });
    }
    t.parse().ok()
}

fn nao_finito(n: f64, maiuscula: bool) -> String {
    let t = if n.is_nan() { "nan" } else if n < 0.0 { "-inf" } else { "inf" };
    if maiuscula { t.to_uppercase() } else { t.into() }
}

fn formato_e(n: f64, precisao: usize, maiuscula: bool) -> String {
    if !n.is_finite() {
        return nao_finito(n, maiuscula);
    }
    let t = format!("{n:.precisao$e}");
    let (m, e) = t.split_once('e').unwrap_or((&t, "0"));
    let exp: i32 = e.parse().unwrap_or(0);
    let letra = if maiuscula { 'E' } else { 'e' };
    format!("{m}{letra}{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
}

fn sem_zeros_a_direita(t: &str) -> &str {
    if t.contains('.') { t.trim_end_matches('0').trim_end_matches('.') } else { t }
}

fn formato_g(n: f64, precisao: usize, maiuscula: bool, manter_zeros: bool) -> String {
    if !n.is_finite() {
        return nao_finito(n, maiuscula);
    }
    let p = precisao.max(1);
    if n == 0.0 {
        return if manter_zeros { format!("{n:.0$}", p - 1) } else if n.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let t = format!("{:.*e}", p - 1, n);
    let exp: i32 = t.split_once('e').and_then(|(_, e)| e.parse().ok()).unwrap_or(0);
    if exp < -4 || exp >= p as i32 {
        let e = formato_e(n, p - 1, maiuscula);
        if manter_zeros {
            return e;
        }
        let corte = e.find(['e', 'E']).unwrap_or(e.len());
        return format!("{}{}", sem_zeros_a_direita(&e[..corte]), &e[corte..]);
    }
    let casas = (p as i32 - 1 - exp) as usize;
    let f = format!("{n:.casas$}");
    if manter_zeros { f } else { sem_zeros_a_direita(&f).to_string() }
}

fn registrar(lua: &Lua, t: &Table, nome: &str, f: impl Fn(&Lua, Vec<Value>) -> Saida + 'static) {
    let func = lua.funcao_rust(Box::new(f));
    t.d.borrow_mut().por(Value::texto(nome), Value::Function(func));
}

fn erro_de_argumento(lua: &Lua, i: usize, funcao: &str, esperava: &str, v: Option<&Value>) -> Error {
    let veio = v.map_or("no value", |v| v.nome_do_tipo());
    lua.erro(format!("bad argument #{} to '{funcao}' ({esperava} expected, got {veio})", i + 1))
}

fn arg_num(lua: &Lua, a: &[Value], i: usize, funcao: &str) -> Result<f64> {
    a.get(i).and_then(numero_de).ok_or_else(|| erro_de_argumento(lua, i, funcao, "number", a.get(i)))
}

fn opt_num(lua: &Lua, a: &[Value], i: usize, funcao: &str, padrao: f64) -> Result<f64> {
    match a.get(i) {
        None | Some(Value::Nil) => Ok(padrao),
        _ => arg_num(lua, a, i, funcao),
    }
}

fn arg_texto(lua: &Lua, a: &[Value], i: usize, funcao: &str) -> Result<Rc<[u8]>> {
    match a.get(i) {
        Some(Value::String(s)) => Ok(s.0.clone()),
        Some(v @ (Value::Number(_) | Value::Integer(_))) => Ok(Rc::from(v.to_string()?.as_bytes())),
        v => Err(erro_de_argumento(lua, i, funcao, "string", v)),
    }
}

fn arg_tabela(lua: &Lua, a: &[Value], i: usize, funcao: &str) -> Result<Table> {
    match a.get(i) {
        Some(Value::Table(t)) => Ok(t.clone()),
        v => Err(erro_de_argumento(lua, i, funcao, "table", v)),
    }
}

fn valor_de_bytes(b: &[u8]) -> Value {
    Value::String(LuaString(Rc::from(b)))
}

fn num(n: impl Into<f64>) -> Vec<Value> {
    vec![Value::Number(n.into())]
}

pub(crate) fn em_texto(lua: &Lua, v: &Value) -> Result<Value> {
    if let Some(h) = lua.metamethod(v, "__tostring") {
        return Ok(lua.chamar_valor(&h, vec![v.clone()])?.into_iter().next().unwrap_or(Value::Nil));
    }
    match v {
        Value::String(_) => Ok(v.clone()),
        v => Ok(Value::texto(&v.to_string()?)),
    }
}

/// `chunkname` como a mensagem de erro mostra.
fn nome_do_chunk(nome: Option<&[u8]>, codigo: &[u8]) -> String {
    match nome {
        Some(n) => String::from_utf8_lossy(n).trim_start_matches(['@', '=']).to_string(),
        None => {
            let linha = codigo.split(|c| *c == b'\n').next().unwrap_or(&[]);
            format!("[string \"{}\"]", String::from_utf8_lossy(&linha[..linha.len().min(40)]))
        }
    }
}

fn carregar(lua: &Lua, codigo: &[u8], nome: Option<&[u8]>, env: Option<Table>) -> Saida {
    let n = nome_do_chunk(nome, codigo);
    Ok(match crate::parser::compilar(codigo, &n) {
        Ok(p) => vec![Value::Function(lua.funcao_lua(p, Vec::new(), env.unwrap_or_else(|| lua.globals())))],
        Err(m) => vec![Value::Nil, Value::texto(&m)],
    })
}

fn instalar_base(lua: &Lua, g: &Table) {
    registrar(lua, g, "print", |lua, a| {
        let mut partes = Vec::with_capacity(a.len());
        for v in &a {
            partes.push(em_texto(lua, v)?.to_string()?);
        }
        let linha = partes.join("\t");
        match &*lua.e.saida.borrow() {
            Some(f) => f(&linha),
            None => println!("{linha}"),
        }
        Ok(Vec::new())
    });
    registrar(lua, g, "type", |lua, a| match a.first() {
        Some(v) => Ok(vec![Value::texto(v.nome_do_tipo())]),
        None => Err(erro_de_argumento(lua, 0, "type", "value", None)),
    });
    registrar(lua, g, "tostring", |lua, a| Ok(vec![em_texto(lua, a.first().unwrap_or(&Value::Nil))?]));
    registrar(lua, g, "tonumber", |lua, a| {
        let v = a.first().unwrap_or(&Value::Nil);
        let base = opt_num(lua, &a, 1, "tonumber", 10.0)? as u32;
        if base == 10 {
            return Ok(vec![numero_de(v).map_or(Value::Nil, Value::Number)]);
        }
        if !(2..=36).contains(&base) {
            return Err(lua.erro("bad argument #2 to 'tonumber' (base out of range)"));
        }
        let t = arg_texto(lua, &a, 0, "tonumber")?;
        let t = String::from_utf8_lossy(&t);
        let t = t.trim();
        let (neg, d) = t.strip_prefix('-').map_or((false, t), |r| (true, r));
        let mut n = 0f64;
        for c in d.chars() {
            match c.to_digit(base) {
                Some(x) => n = n * f64::from(base) + f64::from(x),
                None => return Ok(vec![Value::Nil]),
            }
        }
        Ok(vec![if d.is_empty() { Value::Nil } else { Value::Number(if neg { -n } else { n }) }])
    });
    registrar(lua, g, "next", proximo);
    let next = lua.funcao_rust(Box::new(proximo));
    registrar(lua, g, "pairs", move |lua, a| {
        let t = arg_tabela(lua, &a, 0, "pairs")?;
        Ok(vec![Value::Function(next.clone()), Value::Table(t), Value::Nil])
    });
    let passo = lua.funcao_rust(Box::new(|lua, a| {
        let t = arg_tabela(lua, &a, 0, "ipairs")?;
        let i = arg_num(lua, &a, 1, "ipairs")? + 1.0;
        let v = t.d.borrow().pegar(&Value::Number(i));
        Ok(if v.is_nil() { vec![Value::Nil] } else { vec![Value::Number(i), v] })
    }));
    registrar(lua, g, "ipairs", move |lua, a| {
        let t = arg_tabela(lua, &a, 0, "ipairs")?;
        Ok(vec![Value::Function(passo.clone()), Value::Table(t), Value::Number(0.0)])
    });
    registrar(lua, g, "select", |lua, mut a| {
        if matches!(a.first(), Some(Value::String(s)) if &*s.0 == b"#") {
            return Ok(num((a.len() - 1) as f64));
        }
        let n = arg_num(lua, &a, 0, "select")? as i64;
        let total = a.len() as i64 - 1;
        let de = if n < 0 { total + n + 1 } else { n };
        if de < 1 {
            return Err(lua.erro("bad argument #1 to 'select' (index out of range)"));
        }
        Ok(if de > total { Vec::new() } else { a.split_off(de as usize) })
    });
    registrar(lua, g, "unpack", desempacotar);
    registrar(lua, g, "rawget", |lua, a| {
        let t = arg_tabela(lua, &a, 0, "rawget")?;
        let v = t.d.borrow().pegar(a.get(1).unwrap_or(&Value::Nil));
        Ok(vec![v])
    });
    registrar(lua, g, "rawset", |lua, mut a| {
        let t = arg_tabela(lua, &a, 0, "rawset")?;
        a.resize(3, Value::Nil);
        let v = a.pop().unwrap_or(Value::Nil);
        let k = a.pop().unwrap_or(Value::Nil);
        if k.is_nil() {
            return Err(lua.erro("table index is nil"));
        }
        t.d.borrow_mut().por(k, v);
        Ok(vec![Value::Table(t)])
    });
    registrar(lua, g, "rawlen", |lua, a| match a.first() {
        Some(Value::String(s)) => Ok(num(s.0.len() as f64)),
        _ => Ok(num(arg_tabela(lua, &a, 0, "rawlen")?.raw_len() as f64)),
    });
    registrar(lua, g, "rawequal", |_, a|Ok(vec![Value::Boolean(iguais_cru(a.first().unwrap_or(&Value::Nil), a.get(1).unwrap_or(&Value::Nil)))]));
    registrar(lua, g, "setmetatable", |lua, a| {
        let t = arg_tabela(lua, &a, 0, "setmetatable")?;
        let m = match a.get(1) {
            Some(Value::Table(m)) => Some(m.clone()),
            Some(Value::Nil) => None,
            v => return Err(erro_de_argumento(lua, 1, "setmetatable", "nil or table", v)),
        };
        if let Some(atual) = t.metatable() {
            if !atual.d.borrow().pegar(&Value::texto("__metatable")).is_nil() {
                return Err(lua.erro("cannot change a protected metatable"));
            }
        }
        t.set_metatable(m);
        Ok(vec![Value::Table(t)])
    });
    registrar(lua, g, "getmetatable", |lua, a| {
        let m = match a.first() {
            Some(Value::Table(t)) => t.metatable(),
            Some(Value::String(_)) => lua.e.meta_de_texto.borrow().clone(),
            _ => None,
        };
        let Some(m) = m else { return Ok(vec![Value::Nil]) };
        let protegida = m.d.borrow().pegar(&Value::texto("__metatable"));
        Ok(vec![if protegida.is_nil() { Value::Table(m) } else { protegida }])
    });
    registrar(lua, g, "pcall", |lua, mut a| {
        if a.is_empty() {
            return Err(erro_de_argumento(lua, 0, "pcall", "value", None));
        }
        let f = a.remove(0);
        Ok(match lua.chamar_valor(&f, a) {
            Ok(mut r) => {
                r.insert(0, Value::Boolean(true));
                r
            }
            Err(e) => vec![Value::Boolean(false), valor_do_erro(e)],
        })
    });
    registrar(lua, g, "xpcall", |lua, mut a| {
        if a.len() < 2 {
            return Err(erro_de_argumento(lua, 1, "xpcall", "value", a.get(1)));
        }
        let f = a.remove(0);
        let h = a.remove(0);
        Ok(match lua.chamar_valor(&f, a) {
            Ok(mut r) => {
                r.insert(0, Value::Boolean(true));
                r
            }
            Err(e) => {
                let mut r = lua.chamar_valor(&h, vec![valor_do_erro(e)])?;
                r.insert(0, Value::Boolean(false));
                r
            }
        })
    });
    registrar(lua, g, "error", |lua, a| {
        let nivel = opt_num(lua, &a, 1, "error", 1.0)?;
        Err(match a.into_iter().next().unwrap_or(Value::Nil) {
            Value::String(s) if nivel > 0.0 => lua.erro(String::from_utf8_lossy(&s.0)),
            Value::String(s) => Error::RuntimeError(String::from_utf8_lossy(&s.0).into_owned()),
            v => Error::Valor(Box::new(v)),
        })
    });
    registrar(lua, g, "assert", |lua, a| {
        if a.first().is_some_and(Value::verdadeiro) {
            return Ok(a);
        }
        if a.is_empty() {
            return Err(erro_de_argumento(lua, 0, "assert", "value", None));
        }
        Err(match a.into_iter().nth(1) {
            None | Some(Value::Nil) => Error::RuntimeError("assertion failed!".into()),
            Some(Value::String(s)) => Error::RuntimeError(String::from_utf8_lossy(&s.0).into_owned()),
            Some(v) => Error::Valor(Box::new(v)),
        })
    });
    registrar(lua, g, "loadstring", |lua, a| {
        let t = arg_texto(lua, &a, 0, "loadstring")?;
        let nome = a.get(1).filter(|v| !v.is_nil()).map(|_| arg_texto(lua, &a, 1, "loadstring")).transpose()?;
        carregar(lua, &t, nome.as_deref(), None)
    });
    registrar(lua, g, "load", |lua, a| {
        let codigo: Rc<[u8]> = match a.first() {
            Some(Value::Function(f)) => {
                let mut todo = Vec::new();
                loop {
                    match lua.chamar_funcao(f, Vec::new())?.into_iter().next() {
                        Some(Value::String(s)) if !s.0.is_empty() => todo.extend_from_slice(&s.0),
                        _ => break,
                    }
                }
                Rc::from(todo)
            }
            _ => arg_texto(lua, &a, 0, "load")?,
        };
        let nome = a.get(1).filter(|v| !v.is_nil()).map(|_| arg_texto(lua, &a, 1, "load")).transpose()?;
        let env = match a.get(3) {
            Some(Value::Table(t)) => Some(t.clone()),
            _ => None,
        };
        carregar(lua, &codigo, nome.as_deref(), env)
    });
    registrar(lua, g, "require", exigir);
    registrar(lua, g, "collectgarbage", |_, a| {
        // Tabelas moram em `Rc`: não há coleta a pedir.
        Ok(match a.first() {
            Some(Value::String(s)) if &*s.0 == b"count" => vec![Value::Number(0.0), Value::Number(0.0)],
            _ => num(0.0),
        })
    });
    registrar(lua, g, "getfenv", |lua, a| {
        Ok(vec![match a.first() {
            Some(Value::Function(f)) => match &*f.d {
                crate::CorpoDeFuncao::Lua { env, .. } => Value::Table(env.borrow().clone()),
                crate::CorpoDeFuncao::Rust(_) => Value::Table(lua.globals()),
            },
            _ => Value::Table(lua.globals()),
        }])
    });
    registrar(lua, g, "setfenv", |lua, a| {
        let t = arg_tabela(lua, &a, 1, "setfenv")?;
        if let Some(Value::Function(f)) = a.first() {
            if let crate::CorpoDeFuncao::Lua { env, .. } = &*f.d {
                *env.borrow_mut() = t;
            }
        }
        Ok(a.into_iter().take(1).collect())
    });
    g.d.borrow_mut().por(Value::texto("_G"), Value::Table(g.clone()));
    g.d.borrow_mut().por(Value::texto("_VERSION"), Value::texto("Lua 5.1"));
}

fn valor_do_erro(e: Error) -> Value {
    match e {
        Error::RuntimeError(m) => Value::texto(&m),
        Error::Valor(v) => *v,
    }
}

fn proximo(lua: &Lua, a: Vec<Value>) -> Saida {
    let t = arg_tabela(lua, &a, 0, "next")?;
    let r = t.d.borrow().proximo(a.get(1).unwrap_or(&Value::Nil));
    match r {
        Ok(Some((k, v))) => Ok(vec![k, v]),
        Ok(None) => Ok(vec![Value::Nil]),
        Err(()) => Err(lua.erro("invalid key to 'next'")),
    }
}

fn desempacotar(lua: &Lua, a: Vec<Value>) -> Saida {
    let t = arg_tabela(lua, &a, 0, "unpack")?;
    let d = t.d.borrow();
    let i = opt_num(lua, &a, 1, "unpack", 1.0)?;
    let j = opt_num(lua, &a, 2, "unpack", d.arr.len() as f64)?;
    if j - i >= 8000.0 {
        return Err(lua.erro("too many results to unpack"));
    }
    let mut r = Vec::new();
    let mut n = i;
    while n <= j {
        r.push(d.pegar(&Value::Number(n)));
        n += 1.0;
    }
    Ok(r)
}

fn exigir(lua: &Lua, a: Vec<Value>) -> Saida {
    let nome = arg_texto(lua, &a, 0, "require")?;
    let chave = Value::String(LuaString(nome.clone()));
    let Value::Table(pacote) = lua.globals().d.borrow().pegar(&Value::texto("package")) else {
        return Err(lua.erro("'package' table missing"));
    };
    let carregados = pacote.d.borrow().pegar(&Value::texto("loaded"));
    let Value::Table(carregados) = carregados else { return Err(lua.erro("'package.loaded' must be a table")) };
    let ja = carregados.d.borrow().pegar(&chave);
    if !ja.is_nil() {
        return Ok(vec![ja]);
    }
    let previo = match pacote.d.borrow().pegar(&Value::texto("preload")) {
        Value::Table(p) => p.d.borrow().pegar(&chave),
        _ => Value::Nil,
    };
    let carregador = if previo.is_nil() {
        let caminhos = pacote.d.borrow().pegar(&Value::texto("path")).to_string()?;
        let n = String::from_utf8_lossy(&nome).replace('.', "/");
        let mut achado = None;
        for modelo in caminhos.split(';') {
            let c = modelo.replace('?', &n);
            if let Ok(b) = std::fs::read(&c) {
                achado = Some((c, b));
                break;
            }
        }
        let Some((c, b)) = achado else { return Err(lua.erro(format!("module '{}' not found", String::from_utf8_lossy(&nome)))) };
        let mut r = carregar(lua, &b, Some(c.as_bytes()), None)?;
        if r.len() > 1 {
            return Err(lua.erro(r.pop().unwrap_or(Value::Nil).to_string()?));
        }
        r.pop().unwrap_or(Value::Nil)
    } else {
        previo
    };
    let r = lua.chamar_valor(&carregador, vec![chave.clone()])?.into_iter().next().unwrap_or(Value::Nil);
    let fim = if r.is_nil() {
        let posto = carregados.d.borrow().pegar(&chave);
        if posto.is_nil() { Value::Boolean(true) } else { posto }
    } else {
        r
    };
    carregados.d.borrow_mut().por(chave, fim.clone());
    Ok(vec![fim])
}

/// Posição do Lua (1 = começo, negativa = do fim) em índice de `0..=tam`.
fn posicao(i: f64, tam: usize) -> i64 {
    if i < 0.0 { tam as i64 + i as i64 + 1 } else { i as i64 }
}

fn subtexto(s: &[u8], i: f64, j: f64) -> &[u8] {
    let ini = posicao(i, s.len()).max(1);
    let fim = posicao(j, s.len()).min(s.len() as i64);
    if ini > fim { &[] } else { &s[ini as usize - 1..fim as usize] }
}

fn valor_de_captura(t: &[u8], c: Captura) -> Value {
    match c {
        Captura::Texto(a, b) => valor_de_bytes(&t[a..b]),
        Captura::Posicao(p) => Value::Number((p + 1) as f64),
    }
}

fn capturas(t: &[u8], c: &Casamento) -> Vec<Value> {
    if c.caps.is_empty() {
        return vec![valor_de_bytes(&t[c.ini..c.fim])];
    }
    c.caps.iter().map(|x| valor_de_captura(t, *x)).collect()
}

fn achar_cru(t: &[u8], p: &[u8], ini: usize) -> Option<usize> {
    if p.is_empty() {
        return Some(ini);
    }
    t[ini..].windows(p.len()).position(|w| w == p).map(|i| i + ini)
}

fn buscar(lua: &Lua, a: Vec<Value>, funcao: &str, achar: bool) -> Saida {
    let t = arg_texto(lua, &a, 0, funcao)?;
    let p = arg_texto(lua, &a, 1, funcao)?;
    let ini = (posicao(opt_num(lua, &a, 2, funcao, 1.0)?, t.len()).max(1) - 1) as usize;
    if ini > t.len() {
        return Ok(vec![Value::Nil]);
    }
    if achar && (a.get(3).is_some_and(Value::verdadeiro) || !p.iter().any(|c| b"^$*+?.([%-".contains(c))) {
        return Ok(match achar_cru(&t, &p, ini) {
            Some(i) => vec![Value::Number((i + 1) as f64), Value::Number((i + p.len()) as f64)],
            None => vec![Value::Nil],
        });
    }
    match padroes::procurar(&t, &p, ini).map_err(|m| lua.erro(m))? {
        None => Ok(vec![Value::Nil]),
        Some(c) if achar => {
            let mut r = vec![Value::Number((c.ini + 1) as f64), Value::Number(c.fim as f64)];
            r.extend(c.caps.iter().map(|x| valor_de_captura(&t, *x)));
            Ok(r)
        }
        Some(c) => Ok(capturas(&t, &c)),
    }
}

fn gsub(lua: &Lua, a: Vec<Value>) -> Saida {
    let t = arg_texto(lua, &a, 0, "gsub")?;
    let p = arg_texto(lua, &a, 1, "gsub")?;
    let troca = a.get(2).cloned().unwrap_or(Value::Nil);
    if !matches!(troca, Value::String(_) | Value::Number(_) | Value::Integer(_) | Value::Table(_) | Value::Function(_)) {
        return Err(erro_de_argumento(lua, 2, "gsub", "string/function/table", a.get(2)));
    }
    let maximo = opt_num(lua, &a, 3, "gsub", f64::INFINITY)?;
    let ancora = p.first() == Some(&b'^');
    let pi = usize::from(ancora);
    let (mut s, mut n, mut saida) = (0, 0f64, Vec::with_capacity(t.len()));
    while n < maximo {
        let c = padroes::em(&t, &p, pi, s).map_err(|m| lua.erro(m))?;
        if let Some(c) = &c {
            n += 1.0;
            substituir(lua, &t, c, &troca, &mut saida)?;
        }
        match c {
            Some(c) if c.fim > s => s = c.fim,
            _ if s < t.len() => {
                saida.push(t[s]);
                s += 1;
            }
            _ => break,
        }
        if ancora {
            break;
        }
    }
    saida.extend_from_slice(&t[s.min(t.len())..]);
    Ok(vec![valor_de_bytes(&saida), Value::Number(n)])
}

fn substituir(lua: &Lua, t: &[u8], c: &Casamento, troca: &Value, saida: &mut Vec<u8>) -> Result<()> {
    let primeira = capturas(t, c).into_iter().next().unwrap_or(Value::Nil);
    let r = match troca {
        Value::Table(_) => lua.indexar(troca, &primeira)?,
        Value::Function(f) => lua.chamar_funcao(f, capturas(t, c))?.into_iter().next().unwrap_or(Value::Nil),
        Value::String(_) | Value::Number(_) | Value::Integer(_) => {
            let molde = troca.to_string()?;
            let molde = molde.as_bytes();
            let mut i = 0;
            while i < molde.len() {
                if molde[i] != b'%' {
                    saida.push(molde[i]);
                    i += 1;
                    continue;
                }
                i += 1;
                match molde.get(i) {
                    Some(b'0') => saida.extend_from_slice(&t[c.ini..c.fim]),
                    Some(d @ b'1'..=b'9') => {
                        let n = usize::from(d - b'1');
                        let v = match capturas(t, c).into_iter().nth(n) {
                            Some(v) => v,
                            None => return Err(lua.erro(format!("invalid capture index %{}", n + 1))),
                        };
                        saida.extend_from_slice(v.to_string()?.as_bytes());
                    }
                    Some(x) => saida.push(*x),
                    None => return Err(lua.erro("invalid use of '%' in replacement string")),
                }
                i += 1;
            }
            return Ok(());
        }
        _ => Value::Nil,
    };
    match &r {
        Value::Nil | Value::Boolean(false) => saida.extend_from_slice(&t[c.ini..c.fim]),
        Value::String(s) => saida.extend_from_slice(&s.0),
        Value::Number(_) | Value::Integer(_) => saida.extend_from_slice(r.to_string()?.as_bytes()),
        v => return Err(lua.erro(format!("invalid replacement value (a {})", v.nome_do_tipo()))),
    }
    Ok(())
}

fn inteiro_de(n: f64) -> i64 {
    if n >= 0.0 && n >= i64::MAX as f64 { n as u64 as i64 } else { n as i64 }
}

fn preencher(sinal: &str, corpo: &[u8], largura: usize, esquerda: bool, zero: bool, saida: &mut Vec<u8>) {
    let tam = sinal.len() + corpo.len();
    let falta = largura.saturating_sub(tam);
    if esquerda {
        saida.extend_from_slice(sinal.as_bytes());
        saida.extend_from_slice(corpo);
        saida.resize(saida.len() + falta, b' ');
    } else if zero {
        saida.extend_from_slice(sinal.as_bytes());
        saida.resize(saida.len() + falta, b'0');
        saida.extend_from_slice(corpo);
    } else {
        saida.resize(saida.len() + falta, b' ');
        saida.extend_from_slice(sinal.as_bytes());
        saida.extend_from_slice(corpo);
    }
}

fn citar(s: &[u8], saida: &mut Vec<u8>) {
    saida.push(b'"');
    for &c in s {
        match c {
            b'"' | b'\\' | b'\n' => saida.extend_from_slice(&[b'\\', c]),
            b'\r' => saida.extend_from_slice(b"\\r"),
            0 => saida.extend_from_slice(b"\\000"),
            c => saida.push(c),
        }
    }
    saida.push(b'"');
}

fn formatar(lua: &Lua, a: Vec<Value>) -> Saida {
    let molde = arg_texto(lua, &a, 0, "format")?;
    let (mut i, mut n_arg, mut saida) = (0, 0, Vec::with_capacity(molde.len()));
    while i < molde.len() {
        if molde[i] != b'%' {
            saida.push(molde[i]);
            i += 1;
            continue;
        }
        i += 1;
        if molde.get(i) == Some(&b'%') {
            saida.push(b'%');
            i += 1;
            continue;
        }
        let (mut esquerda, mut mais, mut espaco, mut alt, mut zero) = (false, false, false, false, false);
        while let Some(c) = molde.get(i) {
            match c {
                b'-' => esquerda = true,
                b'+' => mais = true,
                b' ' => espaco = true,
                b'#' => alt = true,
                b'0' => zero = true,
                _ => break,
            }
            i += 1;
        }
        let mut largura = 0usize;
        while let Some(c @ b'0'..=b'9') = molde.get(i) {
            largura = largura * 10 + usize::from(c - b'0');
            i += 1;
        }
        let mut precisao = None;
        if molde.get(i) == Some(&b'.') {
            i += 1;
            let mut p = 0usize;
            while let Some(c @ b'0'..=b'9') = molde.get(i) {
                p = p * 10 + usize::from(c - b'0');
                i += 1;
            }
            precisao = Some(p);
        }
        let Some(&conv) = molde.get(i) else { return Err(lua.erro("invalid option '%' to 'format'")) };
        i += 1;
        n_arg += 1;
        let sinal_de = |neg: bool| if neg { "-" } else if mais { "+" } else if espaco { " " } else { "" };
        match conv {
            b'd' | b'i' => {
                let n = inteiro_de(arg_num(lua, &a, n_arg, "format")?);
                let mut d = n.unsigned_abs().to_string();
                if let Some(p) = precisao {
                    d = format!("{d:0>p$}");
                }
                preencher(sinal_de(n < 0), d.as_bytes(), largura, esquerda, zero && precisao.is_none(), &mut saida);
            }
            b'u' | b'x' | b'X' | b'o' => {
                let n = inteiro_de(arg_num(lua, &a, n_arg, "format")?) as u64;
                let mut d = match conv {
                    b'u' => n.to_string(),
                    b'x' => format!("{n:x}"),
                    b'X' => format!("{n:X}"),
                    _ => format!("{n:o}"),
                };
                if let Some(p) = precisao {
                    d = format!("{d:0>p$}");
                }
                let prefixo = match conv {
                    b'x' if alt && n != 0 => "0x",
                    b'X' if alt && n != 0 => "0X",
                    b'o' if alt => "0",
                    _ => "",
                };
                preencher(prefixo, d.as_bytes(), largura, esquerda, zero && precisao.is_none(), &mut saida);
            }
            b'c' => preencher("", &[arg_num(lua, &a, n_arg, "format")? as u8], largura, esquerda, false, &mut saida),
            b'e' | b'E' | b'f' | b'F' | b'g' | b'G' => {
                let n = arg_num(lua, &a, n_arg, "format")?;
                let p = precisao.unwrap_or(6);
                let t = match conv {
                    b'e' | b'E' => formato_e(n, p, conv == b'E'),
                    b'f' | b'F' if n.is_finite() => format!("{n:.p$}"),
                    b'f' | b'F' => nao_finito(n, conv == b'F'),
                    _ => formato_g(n, p, conv == b'G', alt),
                };
                let corpo = t.strip_prefix('-').unwrap_or(&t);
                preencher(sinal_de(t.starts_with('-')), corpo.as_bytes(), largura, esquerda, zero && n.is_finite(), &mut saida);
            }
            b's' => {
                let v = a.get(n_arg).ok_or_else(|| erro_de_argumento(lua, n_arg, "format", "string", None))?;
                let Value::String(s) = em_texto(lua, v)? else { return Err(lua.erro("'tostring' must return a string to 'format'")) };
                let corte = precisao.map_or(s.0.len(), |p| p.min(s.0.len()));
                preencher("", &s.0[..corte], largura, esquerda, false, &mut saida);
            }
            b'q' => citar(&arg_texto(lua, &a, n_arg, "format")?, &mut saida),
            c => return Err(lua.erro(format!("invalid option '%{}' to 'format'", c as char))),
        }
    }
    Ok(vec![valor_de_bytes(&saida)])
}

fn instalar_string(lua: &Lua, g: &Table) -> Table {
    let t = lua.tabela_nova();
    registrar(lua, &t, "len", |lua, a| Ok(num(arg_texto(lua, &a, 0, "len")?.len() as f64)));
    registrar(lua, &t, "sub", |lua, a| {
        let s = arg_texto(lua, &a, 0, "sub")?;
        let (i, j) = (opt_num(lua, &a, 1, "sub", 1.0)?, opt_num(lua, &a, 2, "sub", -1.0)?);
        Ok(vec![valor_de_bytes(subtexto(&s, i, j))])
    });
    registrar(lua, &t, "upper", |lua, a| Ok(vec![valor_de_bytes(&arg_texto(lua, &a, 0, "upper")?.to_ascii_uppercase())]));
    registrar(lua, &t, "lower", |lua, a| Ok(vec![valor_de_bytes(&arg_texto(lua, &a, 0, "lower")?.to_ascii_lowercase())]));
    registrar(lua, &t, "rep", |lua, a| {
        let s = arg_texto(lua, &a, 0, "rep")?;
        let n = arg_num(lua, &a, 1, "rep")?;
        let sep = match a.get(2) {
            None | Some(Value::Nil) => Rc::from(&[][..]),
            _ => arg_texto(lua, &a, 2, "rep")?,
        };
        let n = if n > 0.0 { n as usize } else { 0 };
        if (s.len() + sep.len()).saturating_mul(n) > 1 << 30 {
            return Err(lua.erro("resulting string too large"));
        }
        let mut r = Vec::with_capacity((s.len() + sep.len()) * n);
        for i in 0..n {
            if i > 0 {
                r.extend_from_slice(&sep);
            }
            r.extend_from_slice(&s);
        }
        Ok(vec![valor_de_bytes(&r)])
    });
    registrar(lua, &t, "reverse", |lua, a| {
        let mut s = arg_texto(lua, &a, 0, "reverse")?.to_vec();
        s.reverse();
        Ok(vec![valor_de_bytes(&s)])
    });
    registrar(lua, &t, "byte", |lua, a| {
        let s = arg_texto(lua, &a, 0, "byte")?;
        let i = opt_num(lua, &a, 1, "byte", 1.0)?;
        let j = opt_num(lua, &a, 2, "byte", i)?;
        Ok(subtexto(&s, i, j).iter().map(|c| Value::Number(f64::from(*c))).collect())
    });
    registrar(lua, &t, "char", |lua, a| {
        let mut s = Vec::with_capacity(a.len());
        for i in 0..a.len() {
            s.push(arg_num(lua, &a, i, "char")? as u8);
        }
        Ok(vec![valor_de_bytes(&s)])
    });
    registrar(lua, &t, "format", formatar);
    registrar(lua, &t, "find", |lua, a| buscar(lua, a, "find", true));
    registrar(lua, &t, "match", |lua, a| buscar(lua, a, "match", false));
    registrar(lua, &t, "gsub", gsub);
    registrar(lua, &t, "gmatch", |lua, a| {
        let texto = arg_texto(lua, &a, 0, "gmatch")?;
        let padrao = arg_texto(lua, &a, 1, "gmatch")?;
        let pos = Cell::new(0);
        let f = lua.funcao_rust(Box::new(move |lua, _| {
            let mut s = pos.get();
            while s <= texto.len() {
                if let Some(c) = padroes::em(&texto, &padrao, 0, s).map_err(|m| lua.erro(m))? {
                    pos.set(if c.fim == s { s + 1 } else { c.fim });
                    return Ok(capturas(&texto, &c));
                }
                s += 1;
            }
            pos.set(texto.len() + 1);
            Ok(vec![Value::Nil])
        }));
        Ok(vec![Value::Function(f)])
    });
    g.d.borrow_mut().por(Value::texto("string"), Value::Table(t.clone()));
    let meta = lua.tabela_nova();
    meta.d.borrow_mut().por(Value::texto("__index"), Value::Table(t.clone()));
    *lua.e.meta_de_texto.borrow_mut() = Some(meta);
    t
}

/// Intercalação de baixo para cima: o comparador do mod pode falhar, e não há reordenação da `std` que devolva esse erro.
fn ordenar(v: &mut Vec<Value>, menor: &mut dyn FnMut(&Value, &Value) -> Result<bool>) -> Result<()> {
    let n = v.len();
    let mut largura = 1;
    let mut aux = Vec::with_capacity(n);
    while largura < n {
        aux.clear();
        let mut i = 0;
        while i < n {
            let (meio, fim) = ((i + largura).min(n), (i + 2 * largura).min(n));
            let (mut e, mut d) = (i, meio);
            while e < meio && d < fim {
                if menor(&v[d], &v[e])? {
                    aux.push(v[d].clone());
                    d += 1;
                } else {
                    aux.push(v[e].clone());
                    e += 1;
                }
            }
            aux.extend_from_slice(&v[e..meio]);
            aux.extend_from_slice(&v[d..fim]);
            i += 2 * largura;
        }
        std::mem::swap(v, &mut aux);
        largura *= 2;
    }
    Ok(())
}

fn tirar_nulos_do_fim(d: &mut Dados) {
    while matches!(d.arr.last(), Some(Value::Nil)) {
        d.arr.pop();
    }
}

fn instalar_table(lua: &Lua, g: &Table) {
    let t = lua.tabela_nova();
    registrar(lua, &t, "insert", |lua, a| {
        let alvo = arg_tabela(lua, &a, 0, "insert")?;
        let mut d = alvo.d.borrow_mut();
        let n = d.arr.len();
        match a.len() {
            2 => {
                let v = a[1].clone();
                d.por(Value::Number((n + 1) as f64), v);
            }
            3 => {
                let pos = arg_num(lua, &a, 1, "insert")?;
                if pos >= 1.0 && pos <= n as f64 {
                    d.arr.insert(pos as usize - 1, a[2].clone());
                    tirar_nulos_do_fim(&mut d);
                    d.trazer_para_a_lista();
                } else {
                    d.por(Value::Number(pos), a[2].clone());
                }
            }
            _ => return Err(lua.erro("wrong number of arguments to 'insert'")),
        }
        Ok(Vec::new())
    });
    registrar(lua, &t, "remove", |lua, a| {
        let alvo = arg_tabela(lua, &a, 0, "remove")?;
        let mut d = alvo.d.borrow_mut();
        let n = d.arr.len();
        let pos = opt_num(lua, &a, 1, "remove", n as f64)?;
        if pos < 1.0 || pos > n as f64 {
            return Ok(Vec::new());
        }
        let v = d.arr.remove(pos as usize - 1);
        tirar_nulos_do_fim(&mut d);
        Ok(vec![v])
    });
    registrar(lua, &t, "concat", |lua, a| {
        let alvo = arg_tabela(lua, &a, 0, "concat")?;
        let sep = match a.get(1) {
            None | Some(Value::Nil) => Rc::from(&[][..]),
            _ => arg_texto(lua, &a, 1, "concat")?,
        };
        let d = alvo.d.borrow();
        let i = opt_num(lua, &a, 2, "concat", 1.0)?;
        let j = opt_num(lua, &a, 3, "concat", d.arr.len() as f64)?;
        let (mut n, mut r) = (i, Vec::new());
        while n <= j {
            match d.pegar(&Value::Number(n)) {
                Value::String(s) => r.extend_from_slice(&s.0),
                v @ (Value::Number(_) | Value::Integer(_)) => r.extend_from_slice(v.to_string()?.as_bytes()),
                v => return Err(lua.erro(format!("invalid value ({}) at index {n} in table for 'concat'", v.nome_do_tipo()))),
            }
            if n < j {
                r.extend_from_slice(&sep);
            }
            n += 1.0;
        }
        Ok(vec![valor_de_bytes(&r)])
    });
    registrar(lua, &t, "sort", |lua, a| {
        let alvo = arg_tabela(lua, &a, 0, "sort")?;
        let mut v = alvo.d.borrow().arr.clone();
        match a.get(1) {
            None | Some(Value::Nil) => ordenar(&mut v, &mut |x, y| lua.menor(x, y))?,
            Some(f) => {
                let f = f.clone();
                ordenar(&mut v, &mut |x, y| {
                    Ok(lua.chamar_valor(&f, vec![x.clone(), y.clone()])?.first().is_some_and(Value::verdadeiro))
                })?;
            }
        }
        alvo.d.borrow_mut().arr = v;
        Ok(Vec::new())
    });
    registrar(lua, &t, "unpack", desempacotar);
    registrar(lua, &t, "getn", |lua, a| Ok(num(arg_tabela(lua, &a, 0, "getn")?.raw_len() as f64)));
    registrar(lua, &t, "maxn", |lua, a| {
        let alvo = arg_tabela(lua, &a, 0, "maxn")?;
        let d = alvo.d.borrow();
        let mut m = d.arr.len() as f64;
        for (k, v) in &d.itens {
            if let (Value::Number(n), false) = (k, v.is_nil()) {
                m = m.max(*n);
            }
        }
        Ok(num(m))
    });
    registrar(lua, &t, "foreach", |lua, a| {
        let alvo = arg_tabela(lua, &a, 0, "foreach")?;
        let f = a.get(1).cloned().unwrap_or(Value::Nil);
        let mut k = Value::Nil;
        loop {
            let par = alvo.d.borrow().proximo(&k).map_err(|()| lua.erro("invalid key to 'next'"))?;
            let Some((nk, v)) = par else { return Ok(Vec::new()) };
            let r = lua.chamar_valor(&f, vec![nk.clone(), v])?;
            if r.first().is_some_and(|v| !v.is_nil()) {
                return Ok(r);
            }
            k = nk;
        }
    });
    registrar(lua, &t, "foreachi", |lua, a| {
        let alvo = arg_tabela(lua, &a, 0, "foreachi")?;
        let f = a.get(1).cloned().unwrap_or(Value::Nil);
        let n = alvo.raw_len();
        for i in 1..=n {
            let v = alvo.d.borrow().pegar(&Value::Number(i as f64));
            let r = lua.chamar_valor(&f, vec![Value::Number(i as f64), v])?;
            if r.first().is_some_and(|v| !v.is_nil()) {
                return Ok(r);
            }
        }
        Ok(Vec::new())
    });
    g.d.borrow_mut().por(Value::texto("table"), Value::Table(t));
}

fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    // Subnormal: traz para a faixa normal antes de ler o expoente.
    let (x, ajuste) = if x.abs() < f64::MIN_POSITIVE { (x * 2f64.powi(54), -54) } else { (x, 0) };
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32 - 1022;
    (f64::from_bits((bits & !(0x7ff << 52)) | (1022 << 52)), exp + ajuste)
}

fn instalar_math(lua: &Lua, g: &Table) {
    let t = lua.tabela_nova();
    macro_rules! de_um {
        ($($nome:literal => $f:expr),+ $(,)?) => {$(
            registrar(lua, &t, $nome, |lua, a| { let f: fn(f64) -> f64 = $f; Ok(num(f(arg_num(lua, &a, 0, $nome)?))) });
        )+};
    }
    de_um! {
        "abs" => f64::abs, "ceil" => f64::ceil, "floor" => f64::floor, "sqrt" => f64::sqrt, "sin" => f64::sin, "cos" => f64::cos, "tan" => f64::tan,
        "asin" => f64::asin, "acos" => f64::acos, "atan" => f64::atan, "sinh" => f64::sinh, "cosh" => f64::cosh, "tanh" => f64::tanh,
        "exp" => f64::exp, "log10" => f64::log10, "deg" => f64::to_degrees, "rad" => f64::to_radians,
    }
    registrar(lua, &t, "log", |lua, a| {
        let x = arg_num(lua, &a, 0, "log")?;
        Ok(num(match a.get(1) {
            None | Some(Value::Nil) => x.ln(),
            _ => x.log(arg_num(lua, &a, 1, "log")?),
        }))
    });
    registrar(lua, &t, "atan2", |lua, a| Ok(num(arg_num(lua, &a, 0, "atan2")?.atan2(arg_num(lua, &a, 1, "atan2")?))));
    registrar(lua, &t, "pow", |lua, a| Ok(num(arg_num(lua, &a, 0, "pow")?.powf(arg_num(lua, &a, 1, "pow")?))));
    registrar(lua, &t, "fmod", |lua, a| Ok(num(arg_num(lua, &a, 0, "fmod")? % arg_num(lua, &a, 1, "fmod")?)));
    registrar(lua, &t, "modf", |lua, a| {
        let x = arg_num(lua, &a, 0, "modf")?;
        Ok(vec![Value::Number(x.trunc()), Value::Number(if x.is_infinite() { 0.0 } else { x - x.trunc() })])
    });
    registrar(lua, &t, "frexp", |lua, a| {
        let (m, e) = frexp(arg_num(lua, &a, 0, "frexp")?);
        Ok(vec![Value::Number(m), Value::Number(f64::from(e))])
    });
    registrar(lua, &t, "ldexp", |lua, a| Ok(num(arg_num(lua, &a, 0, "ldexp")? * 2f64.powi(arg_num(lua, &a, 1, "ldexp")? as i32))));
    registrar(lua, &t, "max", |lua, a| {
        let mut m = arg_num(lua, &a, 0, "max")?;
        for i in 1..a.len() {
            m = m.max(arg_num(lua, &a, i, "max")?);
        }
        Ok(num(m))
    });
    registrar(lua, &t, "min", |lua, a| {
        let mut m = arg_num(lua, &a, 0, "min")?;
        for i in 1..a.len() {
            m = m.min(arg_num(lua, &a, i, "min")?);
        }
        Ok(num(m))
    });
    let semente = Rc::new(Cell::new(0x9E37_79B9_7F4A_7C15u64));
    let s = semente.clone();
    registrar(lua, &t, "randomseed", move |lua, a| {
        s.set((arg_num(lua, &a, 0, "randomseed")?.to_bits() ^ 0x9E37_79B9_7F4A_7C15) | 1);
        Ok(Vec::new())
    });
    registrar(lua, &t, "random", move |lua, a| {
        let mut x = semente.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        semente.set(x);
        let r = (x >> 11) as f64 / (1u64 << 53) as f64;
        let (lo, hi) = match a.len() {
            0 => return Ok(num(r)),
            1 => (1.0, arg_num(lua, &a, 0, "random")?),
            _ => (arg_num(lua, &a, 0, "random")?, arg_num(lua, &a, 1, "random")?),
        };
        if lo > hi {
            return Err(lua.erro("bad argument #1 to 'random' (interval is empty)"));
        }
        Ok(num((r * (hi - lo + 1.0)).floor() + lo))
    });
    t.d.borrow_mut().por(Value::texto("huge"), Value::Number(f64::INFINITY));
    t.d.borrow_mut().por(Value::texto("pi"), Value::Number(std::f64::consts::PI));
    g.d.borrow_mut().por(Value::texto("math"), Value::Table(t));
}

/// `struct tm` do macOS.
#[repr(C)]
struct Tm {
    sec: i32,
    min: i32,
    hora: i32,
    dia: i32,
    mes: i32,
    ano: i32,
    dia_semana: i32,
    dia_ano: i32,
    verao: i32,
    gmtoff: i64,
    zona: *const std::ffi::c_char,
}

extern "C" {
    fn localtime_r(t: *const i64, r: *mut Tm) -> *mut Tm;
    fn gmtime_r(t: *const i64, r: *mut Tm) -> *mut Tm;
    fn mktime(t: *mut Tm) -> i64;
    fn strftime(s: *mut u8, max: usize, f: *const std::ffi::c_char, t: *const Tm) -> usize;
}

fn tm_zerado() -> Tm {
    Tm { sec: 0, min: 0, hora: 0, dia: 0, mes: 0, ano: 0, dia_semana: 0, dia_ano: 0, verao: 0, gmtoff: 0, zona: std::ptr::null() }
}

fn agora() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn os_date(lua: &Lua, a: Vec<Value>) -> Saida {
    let f = match a.first() {
        None | Some(Value::Nil) => Rc::from(&b"%c"[..]),
        _ => arg_texto(lua, &a, 0, "date")?,
    };
    let quando = opt_num(lua, &a, 1, "date", agora() as f64)? as i64;
    let (utc, f) = match f.split_first() {
        Some((b'!', r)) => (true, r),
        _ => (false, &f[..]),
    };
    let mut tm = tm_zerado();
    // Ponteiros válidos pela duração da chamada; a libc só preenche `tm`.
    unsafe {
        if utc {
            gmtime_r(&quando, &mut tm);
        } else {
            localtime_r(&quando, &mut tm);
        }
    }
    if f.starts_with(b"*t") {
        let t = lua.tabela_nova();
        {
            let mut d = t.d.borrow_mut();
            let campos = [
                ("year", tm.ano + 1900),
                ("month", tm.mes + 1),
                ("day", tm.dia),
                ("hour", tm.hora),
                ("min", tm.min),
                ("sec", tm.sec),
                ("wday", tm.dia_semana + 1),
                ("yday", tm.dia_ano + 1),
            ];
            for (k, v) in campos {
                d.por(Value::texto(k), Value::Number(f64::from(v)));
            }
            d.por(Value::texto("isdst"), Value::Boolean(tm.verao > 0));
        }
        return Ok(vec![Value::Table(t)]);
    }
    let molde = std::ffi::CString::new(f).map_err(|_| lua.erro("invalid format to 'date'"))?;
    let mut b = vec![0u8; 1024];
    // `b` tem `b.len()` bytes; `molde` termina em NUL e `tm` foi preenchido acima.
    let n = unsafe { strftime(b.as_mut_ptr(), b.len(), molde.as_ptr(), &tm) };
    Ok(vec![valor_de_bytes(&b[..n])])
}

fn os_time(lua: &Lua, a: Vec<Value>) -> Saida {
    let Some(Value::Table(t)) = a.first() else { return Ok(num(agora() as f64)) };
    let d = t.d.borrow();
    let campo = |k: &str, padrao: Option<i32>| match numero_de(&d.pegar(&Value::texto(k))) {
        Some(n) => Ok(n as i32),
        None => padrao.ok_or_else(|| lua.erro(format!("field '{k}' missing in date table"))),
    };
    let mut tm = tm_zerado();
    tm.ano = campo("year", None)? - 1900;
    tm.mes = campo("month", None)? - 1;
    tm.dia = campo("day", None)?;
    tm.hora = campo("hour", Some(12))?;
    tm.min = campo("min", Some(0))?;
    tm.sec = campo("sec", Some(0))?;
    tm.verao = -1;
    // `tm` é local e completo.
    Ok(num(unsafe { mktime(&mut tm) } as f64))
}

fn resultado_de_io(r: std::io::Result<()>) -> Vec<Value> {
    match r {
        Ok(()) => vec![Value::Boolean(true)],
        Err(e) => vec![Value::Nil, Value::texto(&e.to_string()), Value::Number(f64::from(e.raw_os_error().unwrap_or(0)))],
    }
}

fn instalar_os(lua: &Lua, g: &Table) {
    let t = lua.tabela_nova();
    let inicio = std::time::Instant::now();
    registrar(lua, &t, "clock", move |_, _| Ok(num(inicio.elapsed().as_secs_f64())));
    registrar(lua, &t, "time", os_time);
    registrar(lua, &t, "date", os_date);
    registrar(lua, &t, "difftime", |lua, a| Ok(num(arg_num(lua, &a, 0, "difftime")? - opt_num(lua, &a, 1, "difftime", 0.0)?)));
    registrar(lua, &t, "getenv", |lua, a| {
        let n = arg_texto(lua, &a, 0, "getenv")?;
        Ok(vec![std::env::var(String::from_utf8_lossy(&n).as_ref()).map_or(Value::Nil, |v| Value::texto(&v))])
    });
    registrar(lua, &t, "remove", |lua, a| {
        let n = arg_texto(lua, &a, 0, "remove")?;
        Ok(resultado_de_io(std::fs::remove_file(String::from_utf8_lossy(&n).as_ref())))
    });
    registrar(lua, &t, "rename", |lua, a| {
        let (de, para) = (arg_texto(lua, &a, 0, "rename")?, arg_texto(lua, &a, 1, "rename")?);
        Ok(resultado_de_io(std::fs::rename(String::from_utf8_lossy(&de).as_ref(), String::from_utf8_lossy(&para).as_ref())))
    });
    let serie = Cell::new(0u32);
    registrar(lua, &t, "tmpname", move |_, _| {
        serie.set(serie.get() + 1);
        Ok(vec![Value::texto(&format!("{}/bwms-lua-{}-{}", std::env::temp_dir().display(), std::process::id(), serie.get()))])
    });
    g.d.borrow_mut().por(Value::texto("os"), Value::Table(t));
}

/// Arquivo aberto: o conteúdo inteiro fica em memória e volta ao disco no `flush`/`close`. Os mods leem e gravam configuração pequena.
struct Arquivo {
    caminho: String,
    dados: RefCell<Vec<u8>>,
    pos: Cell<usize>,
    le: bool,
    grava: bool,
    no_fim: bool,
    sujo: Cell<bool>,
    aberto: Cell<bool>,
}

impl Arquivo {
    fn abrir(caminho: &str, modo: &str) -> std::io::Result<Arquivo> {
        let m = modo.replace('b', "");
        let (le, grava, trunca, cria, no_fim) = match m.as_str() {
            "r" => (true, false, false, false, false),
            "r+" => (true, true, false, false, false),
            "w" => (false, true, true, true, false),
            "w+" => (true, true, true, true, false),
            "a" => (false, true, false, true, true),
            "a+" => (true, true, false, true, true),
            _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "modo inválido")),
        };
        let dados = if trunca {
            Vec::new()
        } else {
            match std::fs::read(caminho) {
                Ok(b) => b,
                Err(e) if cria && e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(e),
            }
        };
        if cria {
            std::fs::write(caminho, &dados)?;
        }
        Ok(Arquivo {
            caminho: caminho.into(),
            dados: RefCell::new(dados),
            pos: Cell::new(0),
            le,
            grava,
            no_fim,
            sujo: Cell::new(false),
            aberto: Cell::new(true),
        })
    }

    fn descarregar(&self) -> std::io::Result<()> {
        if self.sujo.replace(false) {
            std::fs::write(&self.caminho, &*self.dados.borrow())?;
        }
        Ok(())
    }

    fn linha(&self, manter_fim: bool) -> Option<Vec<u8>> {
        let d = self.dados.borrow();
        let p = self.pos.get();
        if p >= d.len() {
            return None;
        }
        let f = d[p..].iter().position(|c| *c == b'\n').map_or(d.len(), |i| p + i);
        let com_fim = (f + 1).min(d.len());
        self.pos.set(com_fim);
        Some(d[p..if manter_fim { com_fim } else { f }].to_vec())
    }

    fn numero(&self) -> Option<f64> {
        let d = self.dados.borrow();
        let mut p = self.pos.get();
        while d.get(p).is_some_and(|c| c.is_ascii_whitespace()) {
            p += 1;
        }
        let ini = p;
        while d.get(p).is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.')) {
            p += 1;
        }
        self.pos.set(p);
        texto_em_numero(&d[ini..p])
    }

    fn ler(&self, lua: &Lua, formatos: &[Value]) -> Saida {
        if !self.le {
            return Ok(vec![Value::Nil, Value::texto("Bad file descriptor"), Value::Number(9.0)]);
        }
        if formatos.is_empty() {
            return Ok(vec![self.linha(false).map_or(Value::Nil, |l| valor_de_bytes(&l))]);
        }
        let mut r = Vec::with_capacity(formatos.len());
        for f in formatos {
            let v = match f {
                Value::Number(_) | Value::Integer(_) => {
                    let n = numero_cru(f).unwrap_or(0.0) as usize;
                    let d = self.dados.borrow();
                    let p = self.pos.get();
                    if p >= d.len() && n > 0 {
                        Value::Nil
                    } else {
                        let fim = (p + n).min(d.len());
                        self.pos.set(fim);
                        valor_de_bytes(&d[p.min(fim)..fim])
                    }
                }
                Value::String(s) => match s.0.iter().copied().find(|c| *c != b'*') {
                    Some(b'a') => {
                        let d = self.dados.borrow();
                        let p = self.pos.get().min(d.len());
                        self.pos.set(d.len());
                        valor_de_bytes(&d[p..])
                    }
                    Some(b'l') => self.linha(false).map_or(Value::Nil, |l| valor_de_bytes(&l)),
                    Some(b'L') => self.linha(true).map_or(Value::Nil, |l| valor_de_bytes(&l)),
                    Some(b'n') => self.numero().map_or(Value::Nil, Value::Number),
                    _ => return Err(lua.erro("bad argument to 'read' (invalid format)")),
                },
                _ => return Err(lua.erro("bad argument to 'read' (invalid format)")),
            };
            let acabou = v.is_nil();
            r.push(v);
            if acabou {
                break;
            }
        }
        Ok(r)
    }

    fn escrever(&self, lua: &Lua, valores: &[Value]) -> Result<bool> {
        if !self.grava {
            return Ok(false);
        }
        let mut d = self.dados.borrow_mut();
        for i in 0..valores.len() {
            let b = arg_texto(lua, valores, i, "write")?;
            let p = if self.no_fim { d.len() } else { self.pos.get().min(d.len()) };
            let fim = p + b.len();
            if fim > d.len() {
                d.resize(fim, 0);
            }
            d[p..fim].copy_from_slice(&b);
            self.pos.set(fim);
        }
        self.sujo.set(true);
        Ok(true)
    }
}

impl Drop for Arquivo {
    fn drop(&mut self) {
        // Mod que esquece o `close` não perde o que gravou; sem a quem avisar aqui, falha de disco fica para quem ler o arquivo.
        let _ = self.descarregar();
    }
}

impl UserData for Arquivo {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("read", |lua, f: &Arquivo, a: MultiValue| Ok(MultiValue(f.ler(lua, &a.0)?)));
        m.add_method("write", |lua, f: &Arquivo, a: MultiValue| Ok(f.escrever(lua, &a.0)?));
        m.add_method("flush", |_, f: &Arquivo, ()| Ok(MultiValue(resultado_de_io(f.descarregar()))));
        m.add_method("close", |_, f: &Arquivo, ()| {
            f.aberto.set(false);
            Ok(MultiValue(resultado_de_io(f.descarregar())))
        });
        m.add_method("seek", |lua, f: &Arquivo, a: MultiValue| {
            let onde = match a.0.first() {
                Some(Value::String(s)) => String::from_utf8_lossy(&s.0).into_owned(),
                _ => "cur".into(),
            };
            let delta = opt_num(lua, &a.0, 1, "seek", 0.0)? as i64;
            let base = match onde.as_str() {
                "set" => 0,
                "end" => f.dados.borrow().len() as i64,
                _ => f.pos.get() as i64,
            };
            f.pos.set((base + delta).max(0) as usize);
            Ok(f.pos.get() as f64)
        });
        m.add_method("setvbuf", |_, _: &Arquivo, _: MultiValue| Ok(true));
        m.add_method("lines", |lua, _: &Arquivo, ()| Ok(Value::Function(linhas_do_objeto(lua))));
        m.add_meta_method(crate::MetaMethod::ToString, |_, f: &Arquivo, ()| {
            Ok(if f.aberto.get() { format!("file ({})", f.caminho) } else { "file (closed)".to_string() })
        });
    }
}

/// O iterador do `file:lines()`: recebe o arquivo como o estado do `for`.
fn linhas_do_objeto(lua: &Lua) -> Function {
    lua.funcao_rust(Box::new(|lua, a| match a.first() {
        Some(Value::UserData(u)) => Ok(vec![u.borrow::<Arquivo>()?.linha(false).map_or(Value::Nil, |l| valor_de_bytes(&l))]),
        v => Err(erro_de_argumento(lua, 0, "lines", "file", v)),
    }))
}

fn instalar_io(lua: &Lua, g: &Table) {
    let t = lua.tabela_nova();
    registrar(lua, &t, "open", |lua, a| {
        let c = arg_texto(lua, &a, 0, "open")?;
        let modo = match a.get(1) {
            None | Some(Value::Nil) => Rc::from(&b"r"[..]),
            _ => arg_texto(lua, &a, 1, "open")?,
        };
        let c = String::from_utf8_lossy(&c).into_owned();
        Ok(match Arquivo::abrir(&c, &String::from_utf8_lossy(&modo)) {
            Ok(f) => vec![Value::UserData(lua.create_userdata(f)?)],
            Err(e) => vec![Value::Nil, Value::texto(&format!("{c}: {e}")), Value::Number(f64::from(e.raw_os_error().unwrap_or(0)))],
        })
    });
    registrar(lua, &t, "lines", |lua, a| {
        let c = arg_texto(lua, &a, 0, "lines")?;
        let c = String::from_utf8_lossy(&c).into_owned();
        let f = Arquivo::abrir(&c, "r").map_err(|e| lua.erro(format!("{c}: {e}")))?;
        let u = lua.create_userdata(f)?;
        Ok(vec![Value::Function(linhas_do_objeto(lua)), Value::UserData(u)])
    });
    registrar(lua, &t, "close", |_, a| match a.first() {
        Some(Value::UserData(u)) => {
            let f = u.borrow::<Arquivo>()?;
            f.aberto.set(false);
            Ok(resultado_de_io(f.descarregar()))
        }
        _ => Ok(vec![Value::Boolean(true)]),
    });
    registrar(lua, &t, "write", |lua, a| {
        let mut linha = Vec::new();
        for i in 0..a.len() {
            linha.extend_from_slice(&arg_texto(lua, &a, i, "write")?);
        }
        print!("{}", String::from_utf8_lossy(&linha));
        Ok(Vec::new())
    });
    g.d.borrow_mut().por(Value::texto("io"), Value::Table(t));
}

fn para_bits(lua: &Lua, a: &[Value], i: usize, funcao: &str) -> Result<i32> {
    // Módulo 2^32, como o `bit.tobit`.
    Ok(arg_num(lua, a, i, funcao)?.round().rem_euclid(4_294_967_296.0) as u32 as i32)
}

fn instalar_bit(lua: &Lua, g: &Table) {
    let t = lua.tabela_nova();
    macro_rules! de_dobra {
        ($($nome:literal => $op:tt),+) => {$(
            registrar(lua, &t, $nome, |lua, a| {
                let mut x = para_bits(lua, &a, 0, $nome)?;
                for i in 1..a.len() {
                    x = x $op para_bits(lua, &a, i, $nome)?;
                }
                Ok(num(x))
            });
        )+};
    }
    de_dobra!("band" => &, "bor" => |, "bxor" => ^);
    macro_rules! de_desloca {
        ($($nome:literal => $f:expr),+) => {$(
            registrar(lua, &t, $nome, |lua, a| {
                let f: fn(i32, u32) -> i32 = $f;
                Ok(num(f(para_bits(lua, &a, 0, $nome)?, para_bits(lua, &a, 1, $nome)? as u32 & 31)))
            });
        )+};
    }
    de_desloca!(
        "lshift" => |x, n| x << n,
        "rshift" => |x, n| ((x as u32) >> n) as i32,
        "arshift" => |x, n| x >> n,
        "rol" => i32::rotate_left,
        "ror" => i32::rotate_right
    );
    registrar(lua, &t, "tobit", |lua, a| Ok(num(para_bits(lua, &a, 0, "tobit")?)));
    registrar(lua, &t, "bnot", |lua, a| Ok(num(!para_bits(lua, &a, 0, "bnot")?)));
    registrar(lua, &t, "bswap", |lua, a| Ok(num(para_bits(lua, &a, 0, "bswap")?.swap_bytes())));
    registrar(lua, &t, "tohex", |lua, a| {
        let x = para_bits(lua, &a, 0, "tohex")? as u32;
        let n = opt_num(lua, &a, 1, "tohex", 8.0)? as i32;
        let casas = n.unsigned_abs().clamp(1, 8) as usize;
        let h = format!("{x:08x}");
        let h = &h[8 - casas..];
        Ok(vec![Value::texto(&if n < 0 { h.to_uppercase() } else { h.to_string() })])
    });
    // O CET entrega aos mods esta mesma biblioteca com o nome `bit32`.
    g.d.borrow_mut().por(Value::texto("bit32"), Value::Table(t.clone()));
    g.d.borrow_mut().por(Value::texto("bit"), Value::Table(t));
}

fn instalar_package(lua: &Lua, g: &Table, string: Table) {
    let t = lua.tabela_nova();
    let carregados = lua.tabela_nova();
    for nome in ["string", "table", "math", "os", "io", "bit"] {
        let v = g.d.borrow().pegar(&Value::texto(nome));
        carregados.d.borrow_mut().por(Value::texto(nome), v);
    }
    carregados.d.borrow_mut().por(Value::texto("string"), Value::Table(string));
    carregados.d.borrow_mut().por(Value::texto("_G"), Value::Table(g.clone()));
    t.d.borrow_mut().por(Value::texto("loaded"), Value::Table(carregados));
    t.d.borrow_mut().por(Value::texto("preload"), Value::Table(lua.tabela_nova()));
    t.d.borrow_mut().por(Value::texto("path"), Value::texto("./?.lua;./?/init.lua"));
    t.d.borrow_mut().por(Value::texto("cpath"), Value::texto(""));
    g.d.borrow_mut().por(Value::texto("package"), Value::Table(t));
}

pub(crate) fn instalar(lua: &Lua) {
    let g = lua.globals();
    instalar_base(lua, &g);
    let string = instalar_string(lua, &g);
    instalar_table(lua, &g);
    instalar_math(lua, &g);
    instalar_os(lua, &g);
    instalar_io(lua, &g);
    instalar_bit(lua, &g);
    crate::json::instalar(lua, &g);
    instalar_package(lua, &g, string);
}
