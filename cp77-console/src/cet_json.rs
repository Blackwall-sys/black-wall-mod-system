//! `cet-utils-shippable` (fatia `json`): parser/encoder JSON mínimo, zero-dep (mesmo espírito
//! do parser `.xl` de `bwms-core`), pra CET mods que usam JSON de config sem depender de Lua
//! (`json.decode`/`json.encode` são libs Lua padrão do CET-Windows; aqui é a via nativa).
//! Cobre o suficiente pra config de mod típica: objetos, arrays, string (com escapes básicos),
//! número (i64/f64), bool, null. Não é um parser 100% RFC 8259 (sem \uXXXX, sem exponencial
//! signed duplo-checado) — suficiente pro caso de uso real (arquivos pequenos, mão-escritos).

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(BTreeMap<String, Value>),
}

pub fn parse(s: &str) -> Result<Value, String> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    let v = parse_value(&chars, &mut i)?;
    skip_ws(&chars, &mut i);
    if i != chars.len() {
        return Err(format!("lixo após o valor no offset {i}"));
    }
    Ok(v)
}

fn skip_ws(c: &[char], i: &mut usize) {
    while *i < c.len() && c[*i].is_whitespace() {
        *i += 1;
    }
}

fn parse_value(c: &[char], i: &mut usize) -> Result<Value, String> {
    skip_ws(c, i);
    if *i >= c.len() {
        return Err("fim inesperado".into());
    }
    match c[*i] {
        '{' => parse_obj(c, i),
        '[' => parse_arr(c, i),
        '"' => Ok(Value::Str(parse_str(c, i)?)),
        't' => parse_lit(c, i, "true", Value::Bool(true)),
        'f' => parse_lit(c, i, "false", Value::Bool(false)),
        'n' => parse_lit(c, i, "null", Value::Null),
        _ => parse_num(c, i),
    }
}

fn parse_lit(c: &[char], i: &mut usize, lit: &str, v: Value) -> Result<Value, String> {
    let n = lit.chars().count();
    if *i + n > c.len() || c[*i..*i + n].iter().collect::<String>() != lit {
        return Err(format!("esperava '{lit}' no offset {i}"));
    }
    *i += n;
    Ok(v)
}

fn parse_num(c: &[char], i: &mut usize) -> Result<Value, String> {
    let start = *i;
    if *i < c.len() && (c[*i] == '-' || c[*i] == '+') {
        *i += 1;
    }
    while *i < c.len() && (c[*i].is_ascii_digit() || c[*i] == '.' || c[*i] == 'e' || c[*i] == 'E' || c[*i] == '-' || c[*i] == '+') {
        *i += 1;
    }
    let s: String = c[start..*i].iter().collect();
    s.parse::<f64>().map(Value::Num).map_err(|e| format!("número inválido '{s}': {e}"))
}

fn parse_str(c: &[char], i: &mut usize) -> Result<String, String> {
    if c[*i] != '"' {
        return Err(format!("esperava '\"' no offset {i}"));
    }
    *i += 1;
    let mut out = String::new();
    while *i < c.len() && c[*i] != '"' {
        if c[*i] == '\\' && *i + 1 < c.len() {
            *i += 1;
            match c[*i] {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                other => out.push(other),
            }
        } else {
            out.push(c[*i]);
        }
        *i += 1;
    }
    if *i >= c.len() {
        return Err("string não terminada".into());
    }
    *i += 1; // fecha aspas
    Ok(out)
}

fn parse_arr(c: &[char], i: &mut usize) -> Result<Value, String> {
    *i += 1; // '['
    let mut items = Vec::new();
    skip_ws(c, i);
    if *i < c.len() && c[*i] == ']' {
        *i += 1;
        return Ok(Value::Arr(items));
    }
    loop {
        items.push(parse_value(c, i)?);
        skip_ws(c, i);
        if *i >= c.len() {
            return Err("array não terminado".into());
        }
        match c[*i] {
            ',' => {
                *i += 1;
            }
            ']' => {
                *i += 1;
                break;
            }
            other => return Err(format!("esperava ',' ou ']', achei '{other}'")),
        }
    }
    Ok(Value::Arr(items))
}

fn parse_obj(c: &[char], i: &mut usize) -> Result<Value, String> {
    *i += 1; // '{'
    let mut map = BTreeMap::new();
    skip_ws(c, i);
    if *i < c.len() && c[*i] == '}' {
        *i += 1;
        return Ok(Value::Obj(map));
    }
    loop {
        skip_ws(c, i);
        let key = parse_str(c, i)?;
        skip_ws(c, i);
        if *i >= c.len() || c[*i] != ':' {
            return Err("esperava ':' após a chave".into());
        }
        *i += 1;
        let val = parse_value(c, i)?;
        map.insert(key, val);
        skip_ws(c, i);
        if *i >= c.len() {
            return Err("objeto não terminado".into());
        }
        match c[*i] {
            ',' => {
                *i += 1;
            }
            '}' => {
                *i += 1;
                break;
            }
            other => return Err(format!("esperava ',' ou '}}', achei '{other}'")),
        }
    }
    Ok(Value::Obj(map))
}

pub fn stringify(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                n.to_string()
            }
        }
        Value::Str(s) => format!("\"{}\"", escape_str(s)),
        Value::Arr(items) => {
            let inner: Vec<String> = items.iter().map(stringify).collect();
            format!("[{}]", inner.join(","))
        }
        Value::Obj(map) => {
            let inner: Vec<String> = map.iter().map(|(k, v)| format!("\"{}\":{}", escape_str(k), stringify(v))).collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_objeto_simples() {
        let v = parse(r#"{"a": 1, "b": "texto", "c": true, "d": null}"#).unwrap();
        match v {
            Value::Obj(m) => {
                assert_eq!(m.get("a"), Some(&Value::Num(1.0)));
                assert_eq!(m.get("b"), Some(&Value::Str("texto".into())));
                assert_eq!(m.get("c"), Some(&Value::Bool(true)));
                assert_eq!(m.get("d"), Some(&Value::Null));
            }
            _ => panic!("esperava objeto"),
        }
    }

    #[test]
    fn parse_array_aninhado() {
        let v = parse(r#"[1, 2, [3, 4], {"x": 5}]"#).unwrap();
        match v {
            Value::Arr(items) => {
                assert_eq!(items.len(), 4);
                assert_eq!(items[0], Value::Num(1.0));
                assert_eq!(items[2], Value::Arr(vec![Value::Num(3.0), Value::Num(4.0)]));
            }
            _ => panic!("esperava array"),
        }
    }

    #[test]
    fn parse_string_com_escapes() {
        let v = parse(r#""linha1\nlinha2\ttab\"aspas\"""#).unwrap();
        assert_eq!(v, Value::Str("linha1\nlinha2\ttab\"aspas\"".into()));
    }

    #[test]
    fn roundtrip_stringify_parse() {
        let mut m = BTreeMap::new();
        m.insert("nome".to_string(), Value::Str("BWMS".into()));
        m.insert("versao".to_string(), Value::Num(1.0));
        m.insert("ativo".to_string(), Value::Bool(true));
        m.insert("tags".to_string(), Value::Arr(vec![Value::Str("a".into()), Value::Str("b".into())]));
        let orig = Value::Obj(m);
        let s = stringify(&orig);
        let back = parse(&s).unwrap();
        assert_eq!(orig, back);
    }

    #[test]
    fn numero_negativo_e_float() {
        let v = parse("-3.5").unwrap();
        assert_eq!(v, Value::Num(-3.5));
    }

    #[test]
    fn objeto_vazio_e_array_vazio() {
        assert_eq!(parse("{}").unwrap(), Value::Obj(BTreeMap::new()));
        assert_eq!(parse("[]").unwrap(), Value::Arr(vec![]));
    }

    #[test]
    fn erro_em_json_malformado() {
        assert!(parse("{\"a\": }").is_err());
        assert!(parse("[1, 2").is_err());
    }
}
