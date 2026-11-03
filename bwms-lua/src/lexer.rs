//! Leitura do texto de um `.lua` em tokens.

use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Tok {
    Nome(Rc<str>),
    Cadeia(Rc<[u8]>),
    Numero(f64),
    /// Palavra reservada ou símbolo.
    P(&'static str),
    Fim,
}

const RESERVADAS: [&str; 21] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then",
    "true", "until", "while",
];

const SIMBOLOS: [&str; 26] = [
    "...", "..", "==", "~=", "<=", ">=", "+", "-", "*", "/", "%", "^", "#", "<", ">", "=", "(", ")", "{", "}", "[", "]", ";", ":", ",", ".",
];

pub(crate) struct Lexer<'a> {
    b: &'a [u8],
    i: usize,
    pub(crate) linha: u32,
}

impl<'a> Lexer<'a> {
    pub(crate) fn novo(texto: &'a [u8]) -> Self {
        // `#!` na 1ª linha é comentário, como no `lua`; BOM de UTF-8 sai.
        let i = if texto.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
        let mut l = Lexer { b: texto, i, linha: 1 };
        if l.b[l.i..].starts_with(b"#") {
            while l.i < l.b.len() && l.b[l.i] != b'\n' {
                l.i += 1;
            }
        }
        l
    }

    fn olhar(&self, n: usize) -> u8 {
        *self.b.get(self.i + n).unwrap_or(&0)
    }

    /// Nível de `[==[`: `Some(n)` em `i` sobre o `[`.
    fn nivel_longo(&self) -> Option<usize> {
        let mut j = self.i + 1;
        while self.b.get(j) == Some(&b'=') {
            j += 1;
        }
        (self.b.get(j) == Some(&b'[')).then_some(j - self.i - 1)
    }

    fn cadeia_longa(&mut self, nivel: usize) -> Result<Vec<u8>, String> {
        self.i += nivel + 2;
        if self.olhar(0) == b'\r' {
            self.i += 1;
        }
        if self.olhar(0) == b'\n' {
            self.i += 1;
            self.linha += 1;
        }
        let mut s = Vec::new();
        loop {
            if self.i >= self.b.len() {
                return Err("cadeia longa sem fim".into());
            }
            let c = self.b[self.i];
            if c == b']' {
                let mut j = self.i + 1;
                while self.b.get(j) == Some(&b'=') {
                    j += 1;
                }
                if j - self.i - 1 == nivel && self.b.get(j) == Some(&b']') {
                    self.i = j + 1;
                    return Ok(s);
                }
            }
            if c == b'\n' {
                self.linha += 1;
            }
            s.push(c);
            self.i += 1;
        }
    }

    fn pular_vazio(&mut self) -> Result<(), String> {
        loop {
            match self.olhar(0) {
                b'\n' => {
                    self.linha += 1;
                    self.i += 1;
                }
                b' ' | b'\t' | b'\r' => self.i += 1,
                b'-' if self.olhar(1) == b'-' => {
                    self.i += 2;
                    if self.olhar(0) == b'[' {
                        if let Some(n) = self.nivel_longo() {
                            self.cadeia_longa(n)?;
                            continue;
                        }
                    }
                    while self.i < self.b.len() && self.b[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn cadeia(&mut self, aspa: u8) -> Result<Vec<u8>, String> {
        self.i += 1;
        let mut s = Vec::new();
        loop {
            if self.i >= self.b.len() {
                return Err("texto sem fim".into());
            }
            let c = self.b[self.i];
            self.i += 1;
            match c {
                _ if c == aspa => return Ok(s),
                b'\n' => return Err("texto sem fim na linha".into()),
                b'\\' => {
                    let e = self.olhar(0);
                    self.i += 1;
                    match e {
                        b'n' => s.push(b'\n'),
                        b't' => s.push(b'\t'),
                        b'r' => s.push(b'\r'),
                        b'a' => s.push(7),
                        b'b' => s.push(8),
                        b'f' => s.push(12),
                        b'v' => s.push(11),
                        b'\n' => {
                            self.linha += 1;
                            s.push(b'\n');
                        }
                        b'x' => {
                            let h = std::str::from_utf8(self.b.get(self.i..self.i + 2).unwrap_or(&[])).unwrap_or("");
                            s.push(u8::from_str_radix(h, 16).map_err(|_| "\\x inválido".to_string())?);
                            self.i += 2;
                        }
                        b'z' => {
                            while self.olhar(0).is_ascii_whitespace() && self.i < self.b.len() {
                                if self.olhar(0) == b'\n' {
                                    self.linha += 1;
                                }
                                self.i += 1;
                            }
                        }
                        b'0'..=b'9' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                if self.olhar(0).is_ascii_digit() {
                                    v = v * 10 + u32::from(self.olhar(0) - b'0');
                                    self.i += 1;
                                }
                            }
                            s.push(u8::try_from(v).map_err(|_| "escape decimal grande demais".to_string())?);
                        }
                        outro => s.push(outro),
                    }
                }
                _ => s.push(c),
            }
        }
    }

    fn numero(&mut self) -> Result<f64, String> {
        let ini = self.i;
        if self.olhar(0) == b'0' && matches!(self.olhar(1), b'x' | b'X') {
            self.i += 2;
            let h0 = self.i;
            while self.olhar(0).is_ascii_hexdigit() {
                self.i += 1;
            }
            let h = std::str::from_utf8(&self.b[h0..self.i]).unwrap_or("");
            let v = h.bytes().fold(0f64, |a, c| a * 16.0 + f64::from((c as char).to_digit(16).unwrap_or(0)));
            self.pular_sufixo_inteiro();
            return Ok(v);
        }
        while self.olhar(0).is_ascii_digit() || self.olhar(0) == b'.' {
            self.i += 1;
        }
        if matches!(self.olhar(0), b'e' | b'E') {
            self.i += 1;
            if matches!(self.olhar(0), b'+' | b'-') {
                self.i += 1;
            }
            while self.olhar(0).is_ascii_digit() {
                self.i += 1;
            }
        }
        let t = std::str::from_utf8(&self.b[ini..self.i]).unwrap_or("");
        let v = t.parse::<f64>().map_err(|_| format!("número inválido: {t}"))?;
        self.pular_sufixo_inteiro();
        Ok(v)
    }

    /// `1ULL`/`1LL` dos `.lua` feitos para o LuaJIT valem o número.
    fn pular_sufixo_inteiro(&mut self) {
        while matches!(self.olhar(0), b'U' | b'u' | b'L' | b'l') {
            self.i += 1;
        }
    }

    pub(crate) fn proximo(&mut self) -> Result<Tok, String> {
        self.pular_vazio()?;
        if self.i >= self.b.len() {
            return Ok(Tok::Fim);
        }
        let c = self.b[self.i];
        if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            let ini = self.i;
            while self.i < self.b.len() && (self.b[self.i].is_ascii_alphanumeric() || self.b[self.i] == b'_' || self.b[self.i] >= 0x80) {
                self.i += 1;
            }
            let t = String::from_utf8_lossy(&self.b[ini..self.i]);
            return Ok(match RESERVADAS.iter().find(|r| **r == t) {
                Some(r) => Tok::P(r),
                None => Tok::Nome(Rc::from(&*t)),
            });
        }
        if c.is_ascii_digit() || (c == b'.' && self.olhar(1).is_ascii_digit()) {
            return Ok(Tok::Numero(self.numero()?));
        }
        if c == b'"' || c == b'\'' {
            return Ok(Tok::Cadeia(Rc::from(self.cadeia(c)?)));
        }
        if c == b'[' {
            if let Some(n) = self.nivel_longo() {
                return Ok(Tok::Cadeia(Rc::from(self.cadeia_longa(n)?)));
            }
        }
        for s in SIMBOLOS {
            if self.b[self.i..].starts_with(s.as_bytes()) {
                self.i += s.len();
                return Ok(Tok::P(s));
            }
        }
        Err(format!("caractere inesperado '{}'", c as char))
    }
}
