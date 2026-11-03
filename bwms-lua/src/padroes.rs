//! Padrões de texto de `string.find/match/gmatch/gsub`, escritos a partir do manual da linguagem.

const ABERTA: i64 = -1;
const POSICAO: i64 = -2;
const CAPTURAS_MAXIMAS: usize = 32;
/// Quantificadores e capturas descem pela pilha nativa: padrão patológico de mod vira erro, não estouro.
const PROFUNDIDADE_MAXIMA: u32 = 200;

#[derive(Clone, Copy)]
pub(crate) enum Captura {
    Texto(usize, usize),
    Posicao(usize),
}

pub(crate) struct Casamento {
    pub(crate) ini: usize,
    pub(crate) fim: usize,
    pub(crate) caps: Vec<Captura>,
}

struct Busca<'a> {
    t: &'a [u8],
    p: &'a [u8],
    caps: Vec<(usize, i64)>,
    fundo: u32,
}

/// Tenta casar só na posição `s`, com o padrão a partir de `pi`.
pub(crate) fn em(t: &[u8], p: &[u8], pi: usize, s: usize) -> Result<Option<Casamento>, String> {
    let mut b = Busca { t, p, caps: Vec::new(), fundo: 0 };
    let Some(fim) = b.casar(s, pi)? else { return Ok(None) };
    let mut caps = Vec::with_capacity(b.caps.len());
    for &(ini, tam) in &b.caps {
        caps.push(match tam {
            POSICAO => Captura::Posicao(ini),
            ABERTA => return Err("unfinished capture".into()),
            n => Captura::Texto(ini, ini + n as usize),
        });
    }
    Ok(Some(Casamento { ini: s, fim, caps }))
}

/// Primeira posição a partir de `ini`; `^` no começo do padrão ancora.
pub(crate) fn procurar(t: &[u8], p: &[u8], ini: usize) -> Result<Option<Casamento>, String> {
    if p.first() == Some(&b'^') {
        return em(t, p, 1, ini);
    }
    for s in ini..=t.len() {
        if let Some(c) = em(t, p, 0, s)? {
            return Ok(Some(c));
        }
    }
    Ok(None)
}

fn classe(c: u8, cl: u8) -> bool {
    let r = match cl.to_ascii_lowercase() {
        b'a' => c.is_ascii_alphabetic(),
        b'c' => c.is_ascii_control(),
        b'd' => c.is_ascii_digit(),
        b'g' => c.is_ascii_graphic(),
        b'l' => c.is_ascii_lowercase(),
        b'p' => c.is_ascii_punctuation(),
        b's' => matches!(c, 9..=13 | 32),
        b'u' => c.is_ascii_uppercase(),
        b'w' => c.is_ascii_alphanumeric(),
        b'x' => c.is_ascii_hexdigit(),
        b'z' => c == 0,
        _ => return cl == c,
    };
    r != cl.is_ascii_uppercase()
}

impl Busca<'_> {
    fn fim_da_classe(&self, mut pi: usize) -> Result<usize, String> {
        let c = self.p[pi];
        pi += 1;
        match c {
            b'%' => {
                if pi >= self.p.len() {
                    return Err("malformed pattern (ends with '%')".into());
                }
                Ok(pi + 1)
            }
            b'[' => {
                if self.p.get(pi) == Some(&b'^') {
                    pi += 1;
                }
                // O primeiro `]` depois da abertura é literal.
                loop {
                    let Some(&c) = self.p.get(pi) else { return Err("malformed pattern (missing ']')".into()) };
                    pi += 1;
                    if c == b'%' {
                        pi += 1;
                    }
                    match self.p.get(pi) {
                        Some(b']') => return Ok(pi + 1),
                        Some(_) => {}
                        None => return Err("malformed pattern (missing ']')".into()),
                    }
                }
            }
            _ => Ok(pi),
        }
    }

    fn conjunto(&self, c: u8, mut pi: usize, fecha: usize) -> bool {
        pi += 1;
        let positivo = self.p[pi] != b'^';
        if !positivo {
            pi += 1;
        }
        while pi < fecha {
            if self.p[pi] == b'%' {
                pi += 1;
                if classe(c, self.p[pi]) {
                    return positivo;
                }
                pi += 1;
            } else if pi + 2 < fecha && self.p[pi + 1] == b'-' {
                if self.p[pi] <= c && c <= self.p[pi + 2] {
                    return positivo;
                }
                pi += 3;
            } else {
                if self.p[pi] == c {
                    return positivo;
                }
                pi += 1;
            }
        }
        !positivo
    }

    fn um(&self, s: usize, pi: usize, ep: usize) -> bool {
        let Some(&c) = self.t.get(s) else { return false };
        match self.p[pi] {
            b'.' => true,
            b'%' => classe(c, self.p[pi + 1]),
            b'[' => self.conjunto(c, pi, ep - 1),
            x => x == c,
        }
    }

    fn casar(&mut self, s: usize, pi: usize) -> Result<Option<usize>, String> {
        if self.fundo >= PROFUNDIDADE_MAXIMA {
            return Err("pattern too complex".into());
        }
        self.fundo += 1;
        let r = self.casar_laco(s, pi);
        self.fundo -= 1;
        r
    }

    fn casar_laco(&mut self, mut s: usize, mut pi: usize) -> Result<Option<usize>, String> {
        loop {
            let Some(&c) = self.p.get(pi) else { return Ok(Some(s)) };
            match c {
                b'(' => {
                    let posicao = self.p.get(pi + 1) == Some(&b')');
                    return self.abrir(s, if posicao { pi + 2 } else { pi + 1 }, if posicao { POSICAO } else { ABERTA });
                }
                b')' => return self.fechar(s, pi + 1),
                b'$' if pi + 1 == self.p.len() => return Ok((s == self.t.len()).then_some(s)),
                b'%' => match self.p.get(pi + 1) {
                    Some(b'b') => {
                        let Some(depois) = self.balanceado(s, pi + 2)? else { return Ok(None) };
                        s = depois;
                        pi += 4;
                        continue;
                    }
                    Some(b'f') => {
                        pi += 2;
                        if self.p.get(pi) != Some(&b'[') {
                            return Err("missing '[' after '%f' in pattern".into());
                        }
                        let ep = self.fim_da_classe(pi)?;
                        let antes = if s == 0 { 0 } else { self.t[s - 1] };
                        let agora = self.t.get(s).copied().unwrap_or(0);
                        if !self.conjunto(antes, pi, ep - 1) && self.conjunto(agora, pi, ep - 1) {
                            pi = ep;
                            continue;
                        }
                        return Ok(None);
                    }
                    Some(d) if d.is_ascii_digit() => {
                        let i = usize::from(*d) - usize::from(b'1');
                        let Some(&(ini, tam)) = self.caps.get(i).filter(|c| c.1 >= 0) else {
                            return Err(format!("invalid capture index %{}", i + 1));
                        };
                        let tam = tam as usize;
                        if self.t.len() - s >= tam && self.t[ini..ini + tam] == self.t[s..s + tam] {
                            s += tam;
                            pi += 2;
                            continue;
                        }
                        return Ok(None);
                    }
                    _ => {}
                },
                _ => {}
            }
            let ep = self.fim_da_classe(pi)?;
            match self.p.get(ep) {
                Some(b'?') => {
                    if self.um(s, pi, ep) {
                        if let Some(r) = self.casar(s + 1, ep + 1)? {
                            return Ok(Some(r));
                        }
                    }
                    pi = ep + 1;
                }
                Some(b'+') => return if self.um(s, pi, ep) { self.expandir(s + 1, pi, ep) } else { Ok(None) },
                Some(b'*') => return self.expandir(s, pi, ep),
                Some(b'-') => return self.expandir_pouco(s, pi, ep),
                _ => {
                    if !self.um(s, pi, ep) {
                        return Ok(None);
                    }
                    s += 1;
                    pi = ep;
                }
            }
        }
    }

    fn expandir(&mut self, s: usize, pi: usize, ep: usize) -> Result<Option<usize>, String> {
        let mut i = 0;
        while self.um(s + i, pi, ep) {
            i += 1;
        }
        loop {
            if let Some(r) = self.casar(s + i, ep + 1)? {
                return Ok(Some(r));
            }
            if i == 0 {
                return Ok(None);
            }
            i -= 1;
        }
    }

    fn expandir_pouco(&mut self, mut s: usize, pi: usize, ep: usize) -> Result<Option<usize>, String> {
        loop {
            if let Some(r) = self.casar(s, ep + 1)? {
                return Ok(Some(r));
            }
            if !self.um(s, pi, ep) {
                return Ok(None);
            }
            s += 1;
        }
    }

    fn abrir(&mut self, s: usize, pi: usize, tam: i64) -> Result<Option<usize>, String> {
        if self.caps.len() >= CAPTURAS_MAXIMAS {
            return Err("too many captures".into());
        }
        self.caps.push((s, tam));
        let r = self.casar(s, pi)?;
        if r.is_none() {
            self.caps.pop();
        }
        Ok(r)
    }

    fn fechar(&mut self, s: usize, pi: usize) -> Result<Option<usize>, String> {
        let Some(i) = self.caps.iter().rposition(|c| c.1 == ABERTA) else { return Err("invalid pattern capture".into()) };
        self.caps[i].1 = (s - self.caps[i].0) as i64;
        let r = self.casar(s, pi)?;
        if r.is_none() {
            self.caps[i].1 = ABERTA;
        }
        Ok(r)
    }

    fn balanceado(&self, s: usize, pi: usize) -> Result<Option<usize>, String> {
        let (Some(&a), Some(&b)) = (self.p.get(pi), self.p.get(pi + 1)) else {
            return Err("missing arguments to '%b'".into());
        };
        if self.t.get(s) != Some(&a) {
            return Ok(None);
        }
        let mut n = 1;
        for (i, &c) in self.t.iter().enumerate().skip(s + 1) {
            if c == b {
                n -= 1;
                if n == 0 {
                    return Ok(Some(i + 1));
                }
            } else if c == a {
                n += 1;
            }
        }
        Ok(None)
    }
}
