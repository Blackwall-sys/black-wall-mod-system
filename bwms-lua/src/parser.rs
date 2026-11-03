//! Da lista de tokens à árvore que o executor anda. As variáveis locais saem resolvidas aqui: cada uma tem um slot no quadro da função, e a
//! que uma função interna captura fica marcada para morar numa caixa compartilhada.

use crate::lexer::{Lexer, Tok};
use std::cell::Cell;
use std::rc::Rc;

/// Variável local declarada: o slot e se alguma função interna a captura.
#[derive(Clone, Debug)]
pub(crate) struct Var {
    pub(crate) slot: u16,
    pub(crate) capturada: Rc<Cell<bool>>,
}

/// De onde a função pega cada variável de fora ao nascer: um slot do pai ou uma variável de fora do pai.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OrigemDeFora {
    pub(crate) do_pai: bool,
    pub(crate) indice: u16,
}

#[derive(Debug)]
pub(crate) struct Proto {
    pub(crate) params: Vec<Var>,
    pub(crate) vararg: bool,
    pub(crate) slots: u16,
    pub(crate) corpo: Vec<Linha>,
    pub(crate) de_fora: Vec<OrigemDeFora>,
    pub(crate) chunk: Rc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Bin {
    Soma,
    Sub,
    Mul,
    Div,
    Resto,
    Pot,
    Concat,
    Igual,
    Dif,
    Menor,
    MenorIg,
    Maior,
    MaiorIg,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Un {
    Neg,
    Not,
    Len,
}

#[derive(Debug)]
pub(crate) enum Campo {
    Pos(Expr),
    Chave(Expr, Expr),
}

#[derive(Debug)]
pub(crate) enum Expr {
    Nil,
    True,
    False,
    Num(f64),
    Str(Rc<[u8]>),
    Vararg,
    Func(Rc<Proto>),
    Local(u16),
    Fora(u16),
    Global(Rc<[u8]>),
    Index(Box<Expr>, Box<Expr>),
    Chamada(Box<Expr>, Vec<Expr>),
    Metodo(Box<Expr>, Rc<[u8]>, Vec<Expr>),
    Bin(Bin, Box<Expr>, Box<Expr>),
    Un(Un, Box<Expr>),
    E(Box<Expr>, Box<Expr>),
    Ou(Box<Expr>, Box<Expr>),
    Tabela(Vec<Campo>),
    Parenteses(Box<Expr>),
}

#[derive(Debug)]
pub(crate) enum Alvo {
    Local(u16),
    Fora(u16),
    Global(Rc<[u8]>),
    Index(Expr, Expr),
}

/// Comando com a linha do fonte, para a mensagem de erro.
#[derive(Debug)]
pub(crate) struct Linha(pub(crate) u32, pub(crate) Stat);

#[derive(Debug)]
pub(crate) enum Stat {
    Local(Vec<Var>, Vec<Expr>),
    LocalFunc(Var, Rc<Proto>),
    Atrib(Vec<Alvo>, Vec<Expr>),
    Chamada(Expr),
    Do(Vec<Linha>),
    While(Expr, Vec<Linha>),
    Repeat(Vec<Linha>, Expr),
    If(Vec<(Expr, Vec<Linha>)>, Option<Vec<Linha>>),
    ForNum(Var, Expr, Expr, Option<Expr>, Vec<Linha>),
    ForIn(Vec<Var>, Vec<Expr>, Vec<Linha>),
    Return(Vec<Expr>),
    Break,
}

#[derive(Default)]
struct EstadoDaFuncao {
    blocos: Vec<Vec<(Rc<str>, Var)>>,
    ativos: u16,
    maximo: u16,
    de_fora: Vec<(Rc<str>, OrigemDeFora)>,
    vararg: bool,
}

struct Parser<'a> {
    lx: Lexer<'a>,
    tok: Tok,
    adiante: Option<Tok>,
    linha: u32,
    funcoes: Vec<EstadoDaFuncao>,
    chunk: Rc<str>,
}

type R<T> = Result<T, String>;

pub(crate) fn compilar(texto: &[u8], chunk: &str) -> Result<Rc<Proto>, String> {
    let chunk: Rc<str> = Rc::from(chunk);
    let mut lx = Lexer::novo(texto);
    let erro = |c: &str, l: u32, m: String| format!("{c}:{l}: {m}");
    let tok = lx.proximo().map_err(|m| erro(&chunk, lx.linha, m))?;
    let mut p = Parser { linha: lx.linha, lx, tok, adiante: None, funcoes: Vec::new(), chunk: chunk.clone() };
    let mut f = EstadoDaFuncao { vararg: true, ..Default::default() };
    f.blocos.push(Vec::new());
    p.funcoes.push(f);
    let corpo = p.bloco().and_then(|c| if p.tok == Tok::Fim { Ok(c) } else { Err(format!("esperava o fim do arquivo, achei {:?}", p.tok)) });
    let corpo = corpo.map_err(|m| erro(&chunk, p.linha, m))?;
    let f = p.funcoes.pop().unwrap_or_default();
    Ok(Rc::new(Proto { params: Vec::new(), vararg: true, slots: f.maximo, corpo, de_fora: Vec::new(), chunk }))
}

impl Parser<'_> {
    fn avancar(&mut self) -> R<()> {
        self.tok = match self.adiante.take() {
            Some(t) => t,
            None => self.lx.proximo()?,
        };
        self.linha = self.lx.linha;
        Ok(())
    }

    fn olhar_adiante(&mut self) -> R<&Tok> {
        if self.adiante.is_none() {
            self.adiante = Some(self.lx.proximo()?);
        }
        Ok(self.adiante.as_ref().expect("posto acima"))
    }

    fn e(&self, p: &str) -> bool {
        matches!(&self.tok, Tok::P(s) if *s == p)
    }

    fn aceita(&mut self, p: &str) -> R<bool> {
        let sim = self.e(p);
        if sim {
            self.avancar()?;
        }
        Ok(sim)
    }

    fn exige(&mut self, p: &str) -> R<()> {
        if self.aceita(p)? {
            Ok(())
        } else {
            Err(format!("esperava '{p}', achei {:?}", self.tok))
        }
    }

    fn nome(&mut self) -> R<Rc<str>> {
        match self.tok.clone() {
            Tok::Nome(n) => {
                self.avancar()?;
                Ok(n)
            }
            t => Err(format!("esperava um nome, achei {t:?}")),
        }
    }

    fn fun(&mut self) -> &mut EstadoDaFuncao {
        self.funcoes.last_mut().expect("sempre há a função do arquivo")
    }

    fn declarar(&mut self, nome: Rc<str>) -> Var {
        let f = self.fun();
        let v = Var { slot: f.ativos, capturada: Rc::new(Cell::new(false)) };
        f.ativos += 1;
        f.maximo = f.maximo.max(f.ativos);
        f.blocos.last_mut().expect("sempre há bloco").push((nome, v.clone()));
        v
    }

    fn abrir(&mut self) {
        self.fun().blocos.push(Vec::new());
    }

    fn fechar(&mut self) {
        let f = self.fun();
        let b = f.blocos.pop().unwrap_or_default();
        f.ativos -= b.len() as u16;
    }

    /// Variável de `nivel` que vem de fora dele, criada ao subir pelos pais; `None` = global.
    fn resolver_fora(&mut self, nome: &Rc<str>, nivel: usize) -> Option<u16> {
        if let Some(i) = self.funcoes[nivel].de_fora.iter().position(|(n, _)| n == nome) {
            return Some(i as u16);
        }
        if nivel == 0 {
            return None;
        }
        let local = self.funcoes[nivel - 1].blocos.iter().rev().find_map(|b| b.iter().rev().find(|(n, _)| n == nome).map(|(_, v)| v.clone()));
        let origem = match local {
            Some(v) => {
                v.capturada.set(true);
                OrigemDeFora { do_pai: true, indice: v.slot }
            }
            None => OrigemDeFora { do_pai: false, indice: self.resolver_fora(nome, nivel - 1)? },
        };
        let f = &mut self.funcoes[nivel];
        f.de_fora.push((nome.clone(), origem));
        Some(f.de_fora.len() as u16 - 1)
    }

    fn variavel(&mut self, nome: Rc<str>) -> Expr {
        let f = self.fun();
        if let Some(v) = f.blocos.iter().rev().find_map(|b| b.iter().rev().find(|(n, _)| *n == nome).map(|(_, v)| v.clone())) {
            return Expr::Local(v.slot);
        }
        let nivel = self.funcoes.len() - 1;
        match self.resolver_fora(&nome, nivel) {
            Some(i) => Expr::Fora(i),
            None => Expr::Global(Rc::from(nome.as_bytes())),
        }
    }

    fn fim_de_bloco(&self) -> bool {
        matches!(&self.tok, Tok::Fim) || ["end", "else", "elseif", "until"].iter().any(|p| self.e(p))
    }

    fn bloco(&mut self) -> R<Vec<Linha>> {
        let mut v = Vec::new();
        while !self.fim_de_bloco() {
            let l = self.linha;
            if self.e("return") {
                self.avancar()?;
                let exprs = if self.fim_de_bloco() || self.e(";") { Vec::new() } else { self.lista_expr()? };
                self.aceita(";")?;
                v.push(Linha(l, Stat::Return(exprs)));
                if !self.fim_de_bloco() {
                    return Err("'return' tem de ser o último comando do bloco".into());
                }
                break;
            }
            if self.e("break") {
                self.avancar()?;
                self.aceita(";")?;
                v.push(Linha(l, Stat::Break));
                continue;
            }
            if let Some(s) = self.comando()? {
                v.push(Linha(l, s));
            }
            self.aceita(";")?;
        }
        Ok(v)
    }

    fn bloco_com_escopo(&mut self) -> R<Vec<Linha>> {
        self.abrir();
        let b = self.bloco();
        self.fechar();
        b
    }

    fn comando(&mut self) -> R<Option<Stat>> {
        if self.aceita("if")? {
            let mut casos = Vec::new();
            let c = self.expr(0)?;
            self.exige("then")?;
            casos.push((c, self.bloco_com_escopo()?));
            let mut senao = None;
            loop {
                if self.aceita("elseif")? {
                    let c = self.expr(0)?;
                    self.exige("then")?;
                    casos.push((c, self.bloco_com_escopo()?));
                } else if self.aceita("else")? {
                    senao = Some(self.bloco_com_escopo()?);
                    self.exige("end")?;
                    break;
                } else {
                    self.exige("end")?;
                    break;
                }
            }
            return Ok(Some(Stat::If(casos, senao)));
        }
        if self.aceita("while")? {
            let c = self.expr(0)?;
            self.exige("do")?;
            let b = self.bloco_com_escopo()?;
            self.exige("end")?;
            return Ok(Some(Stat::While(c, b)));
        }
        if self.aceita("do")? {
            let b = self.bloco_com_escopo()?;
            self.exige("end")?;
            return Ok(Some(Stat::Do(b)));
        }
        if self.aceita("for")? {
            return self.comando_for().map(Some);
        }
        if self.aceita("repeat")? {
            // A condição enxerga as locais do corpo.
            self.abrir();
            let b = self.bloco();
            let c = b.and_then(|b| {
                self.exige("until")?;
                Ok((b, self.expr(0)?))
            });
            self.fechar();
            let (b, c) = c?;
            return Ok(Some(Stat::Repeat(b, c)));
        }
        if self.aceita("function")? {
            let mut alvo = {
                let n = self.nome()?;
                self.variavel(n)
            };
            let mut metodo = false;
            while self.e(".") || self.e(":") {
                metodo = self.e(":");
                self.avancar()?;
                let n = self.nome()?;
                alvo = Expr::Index(Box::new(alvo), Box::new(Expr::Str(Rc::from(n.as_bytes()))));
                if metodo {
                    break;
                }
            }
            let p = self.corpo_de_funcao(metodo)?;
            return Ok(Some(Stat::Atrib(vec![self.alvo_de(alvo)?], vec![Expr::Func(p)])));
        }
        if self.aceita("local")? {
            if self.aceita("function")? {
                let n = self.nome()?;
                let v = self.declarar(n);
                let p = self.corpo_de_funcao(false)?;
                return Ok(Some(Stat::LocalFunc(v, p)));
            }
            let mut nomes = vec![self.nome()?];
            // `<const>`/`<close>` não existem nesta versão da linguagem.
            while self.aceita(",")? {
                nomes.push(self.nome()?);
            }
            let exprs = if self.aceita("=")? { self.lista_expr()? } else { Vec::new() };
            let vars = nomes.into_iter().map(|n| self.declarar(n)).collect();
            return Ok(Some(Stat::Local(vars, exprs)));
        }
        let e = self.expr_sufixada()?;
        if self.e("=") || self.e(",") {
            let mut alvos = vec![self.alvo_de(e)?];
            while self.aceita(",")? {
                let e = self.expr_sufixada()?;
                alvos.push(self.alvo_de(e)?);
            }
            self.exige("=")?;
            return Ok(Some(Stat::Atrib(alvos, self.lista_expr()?)));
        }
        match e {
            Expr::Chamada(..) | Expr::Metodo(..) => Ok(Some(Stat::Chamada(e))),
            _ => Err("comando inválido: esperava atribuição ou chamada".into()),
        }
    }

    fn alvo_de(&self, e: Expr) -> R<Alvo> {
        Ok(match e {
            Expr::Local(i) => Alvo::Local(i),
            Expr::Fora(i) => Alvo::Fora(i),
            Expr::Global(n) => Alvo::Global(n),
            Expr::Index(o, k) => Alvo::Index(*o, *k),
            _ => return Err("não dá para atribuir a isto".into()),
        })
    }

    fn comando_for(&mut self) -> R<Stat> {
        let n1 = self.nome()?;
        if self.aceita("=")? {
            let ini = self.expr(0)?;
            self.exige(",")?;
            let fim = self.expr(0)?;
            let passo = if self.aceita(",")? { Some(self.expr(0)?) } else { None };
            self.exige("do")?;
            self.abrir();
            let v = self.declarar(n1);
            let b = self.bloco();
            self.fechar();
            self.exige("end")?;
            return Ok(Stat::ForNum(v, ini, fim, passo, b?));
        }
        let mut nomes = vec![n1];
        while self.aceita(",")? {
            nomes.push(self.nome()?);
        }
        self.exige("in")?;
        let exprs = self.lista_expr()?;
        self.exige("do")?;
        self.abrir();
        let vars = nomes.into_iter().map(|n| self.declarar(n)).collect();
        let b = self.bloco();
        self.fechar();
        self.exige("end")?;
        Ok(Stat::ForIn(vars, exprs, b?))
    }

    fn corpo_de_funcao(&mut self, metodo: bool) -> R<Rc<Proto>> {
        let mut f = EstadoDaFuncao::default();
        f.blocos.push(Vec::new());
        self.funcoes.push(f);
        let mut params = Vec::new();
        if metodo {
            params.push(self.declarar(Rc::from("self")));
        }
        self.exige("(")?;
        if !self.e(")") {
            loop {
                if self.aceita("...")? {
                    self.fun().vararg = true;
                    break;
                }
                let n = self.nome()?;
                params.push(self.declarar(n));
                if !self.aceita(",")? {
                    break;
                }
            }
        }
        self.exige(")")?;
        let corpo = self.bloco()?;
        self.exige("end")?;
        let f = self.funcoes.pop().unwrap_or_default();
        Ok(Rc::new(Proto {
            params,
            vararg: f.vararg,
            slots: f.maximo,
            corpo,
            de_fora: f.de_fora.iter().map(|(_, o)| *o).collect(),
            chunk: self.chunk.clone(),
        }))
    }

    fn lista_expr(&mut self) -> R<Vec<Expr>> {
        let mut v = vec![self.expr(0)?];
        while self.aceita(",")? {
            v.push(self.expr(0)?);
        }
        Ok(v)
    }

    fn expr_primaria(&mut self) -> R<Expr> {
        match self.tok.clone() {
            Tok::Nome(n) => {
                self.avancar()?;
                Ok(self.variavel(n))
            }
            Tok::P("(") => {
                self.avancar()?;
                let e = self.expr(0)?;
                self.exige(")")?;
                Ok(Expr::Parenteses(Box::new(e)))
            }
            t => Err(format!("expressão inesperada: {t:?}")),
        }
    }

    fn argumentos(&mut self) -> R<Vec<Expr>> {
        match self.tok.clone() {
            Tok::Cadeia(s) => {
                self.avancar()?;
                Ok(vec![Expr::Str(s)])
            }
            Tok::P("{") => Ok(vec![self.tabela()?]),
            Tok::P("(") => {
                self.avancar()?;
                if self.aceita(")")? {
                    return Ok(Vec::new());
                }
                let v = self.lista_expr()?;
                self.exige(")")?;
                Ok(v)
            }
            t => Err(format!("esperava argumentos, achei {t:?}")),
        }
    }

    fn expr_sufixada(&mut self) -> R<Expr> {
        let mut e = self.expr_primaria()?;
        loop {
            match self.tok.clone() {
                Tok::P(".") => {
                    self.avancar()?;
                    let n = self.nome()?;
                    e = Expr::Index(Box::new(e), Box::new(Expr::Str(Rc::from(n.as_bytes()))));
                }
                Tok::P("[") => {
                    self.avancar()?;
                    let k = self.expr(0)?;
                    self.exige("]")?;
                    e = Expr::Index(Box::new(e), Box::new(k));
                }
                Tok::P(":") => {
                    self.avancar()?;
                    let n = self.nome()?;
                    let a = self.argumentos()?;
                    e = Expr::Metodo(Box::new(e), Rc::from(n.as_bytes()), a);
                }
                Tok::P("(") | Tok::P("{") | Tok::Cadeia(_) => {
                    let a = self.argumentos()?;
                    e = Expr::Chamada(Box::new(e), a);
                }
                _ => return Ok(e),
            }
        }
    }

    fn tabela(&mut self) -> R<Expr> {
        self.exige("{")?;
        let mut campos = Vec::new();
        while !self.e("}") {
            if self.aceita("[")? {
                let k = self.expr(0)?;
                self.exige("]")?;
                self.exige("=")?;
                campos.push(Campo::Chave(k, self.expr(0)?));
            } else if matches!(self.tok, Tok::Nome(_)) && matches!(self.olhar_adiante()?, Tok::P("=")) {
                let n = self.nome()?;
                self.exige("=")?;
                campos.push(Campo::Chave(Expr::Str(Rc::from(n.as_bytes())), self.expr(0)?));
            } else {
                campos.push(Campo::Pos(self.expr(0)?));
            }
            if !self.aceita(",")? && !self.aceita(";")? {
                break;
            }
        }
        self.exige("}")?;
        Ok(Expr::Tabela(campos))
    }

    fn expr_simples(&mut self) -> R<Expr> {
        let e = match self.tok.clone() {
            Tok::Numero(n) => Expr::Num(n),
            Tok::Cadeia(s) => Expr::Str(s),
            Tok::P("nil") => Expr::Nil,
            Tok::P("true") => Expr::True,
            Tok::P("false") => Expr::False,
            Tok::P("...") => {
                if !self.fun().vararg {
                    return Err("'...' fora de função variádica".into());
                }
                Expr::Vararg
            }
            Tok::P("{") => return self.tabela(),
            Tok::P("function") => {
                self.avancar()?;
                return Ok(Expr::Func(self.corpo_de_funcao(false)?));
            }
            _ => return self.expr_sufixada(),
        };
        self.avancar()?;
        Ok(e)
    }

    /// Precedência da linguagem: (esquerda, direita) de cada operador binário; o unário vale 8.
    fn expr(&mut self, limite: u8) -> R<Expr> {
        let un = match &self.tok {
            Tok::P("not") => Some(Un::Not),
            Tok::P("-") => Some(Un::Neg),
            Tok::P("#") => Some(Un::Len),
            _ => None,
        };
        let mut e = match un {
            Some(u) => {
                self.avancar()?;
                let o = self.expr(8)?;
                match (u, o) {
                    (Un::Neg, Expr::Num(n)) => Expr::Num(-n),
                    (u, o) => Expr::Un(u, Box::new(o)),
                }
            }
            None => self.expr_simples()?,
        };
        loop {
            let (esq, dir) = match &self.tok {
                Tok::P("or") => (1, 1),
                Tok::P("and") => (2, 2),
                Tok::P("<" | ">" | "<=" | ">=" | "~=" | "==") => (3, 3),
                Tok::P("..") => (5, 4),
                Tok::P("+" | "-") => (6, 6),
                Tok::P("*" | "/" | "%") => (7, 7),
                Tok::P("^") => (10, 9),
                _ => return Ok(e),
            };
            if esq <= limite {
                return Ok(e);
            }
            let Tok::P(op) = self.tok.clone() else { return Ok(e) };
            self.avancar()?;
            let d = self.expr(dir)?;
            let (a, b) = (Box::new(e), Box::new(d));
            e = match op {
                "or" => Expr::Ou(a, b),
                "and" => Expr::E(a, b),
                "<" => Expr::Bin(Bin::Menor, a, b),
                ">" => Expr::Bin(Bin::Maior, a, b),
                "<=" => Expr::Bin(Bin::MenorIg, a, b),
                ">=" => Expr::Bin(Bin::MaiorIg, a, b),
                "~=" => Expr::Bin(Bin::Dif, a, b),
                "==" => Expr::Bin(Bin::Igual, a, b),
                ".." => Expr::Bin(Bin::Concat, a, b),
                "+" => Expr::Bin(Bin::Soma, a, b),
                "-" => Expr::Bin(Bin::Sub, a, b),
                "*" => Expr::Bin(Bin::Mul, a, b),
                "/" => Expr::Bin(Bin::Div, a, b),
                "%" => Expr::Bin(Bin::Resto, a, b),
                _ => Expr::Bin(Bin::Pot, a, b),
            };
        }
    }
}
