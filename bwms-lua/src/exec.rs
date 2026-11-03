//! Executor: anda a árvore do `parser`. Metamétodos de tabela e de userdata passam todos por aqui.

use crate::parser::{Alvo, Bin, Campo, Expr, Linha, Proto, Stat, Un, Var};
use crate::{CorpoDeFuncao, Error, Function, Lua, MetaMethod, Result, Table, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// A árvore anda na pilha nativa, e o tamanho dela depende da thread do jogo que chamou: a recursão do mod vira erro quando sobra só esta folga
/// para o código nativo sem chamada de Lua no meio (padrões, json, marshalling da ponte).
const FOLGA_DA_PILHA: usize = 256 * 1024;

extern "C" {
    fn pthread_self() -> usize;
    fn pthread_get_stackaddr_np(t: usize) -> usize;
    fn pthread_get_stacksize_np(t: usize) -> usize;
}

fn piso_da_pilha() -> usize {
    // A thread corrente sempre existe; o topo e o tamanho são da própria pthread.
    unsafe {
        let t = pthread_self();
        (pthread_get_stackaddr_np(t) - pthread_get_stacksize_np(t)) + FOLGA_DA_PILHA
    }
}

enum Slot {
    Direto(Value),
    Caixa(Rc<RefCell<Value>>),
}

struct Quadro<'a> {
    slots: Vec<Slot>,
    varargs: Vec<Value>,
    de_fora: &'a [Rc<RefCell<Value>>],
    env: &'a RefCell<Table>,
    proto: &'a Proto,
    linha: u32,
}

enum Fluxo {
    Segue,
    Quebra,
    Retorna(Vec<Value>),
}

fn meta_de(lua: &Lua, v: &Value) -> Option<Table> {
    match v {
        Value::Table(t) => t.d.borrow().meta.clone(),
        Value::String(_) => lua.e.meta_de_texto.borrow().clone(),
        _ => None,
    }
}

fn metamethod_de_userdata(nome: &str) -> Option<MetaMethod> {
    Some(match nome {
        "__add" => MetaMethod::Add,
        "__sub" => MetaMethod::Sub,
        "__mul" => MetaMethod::Mul,
        "__div" => MetaMethod::Div,
        "__eq" => MetaMethod::Eq,
        "__lt" => MetaMethod::Lt,
        "__le" => MetaMethod::Le,
        "__len" => MetaMethod::Len,
        "__concat" => MetaMethod::Concat,
        "__call" => MetaMethod::Call,
        "__unm" => MetaMethod::Unm,
        "__tostring" => MetaMethod::ToString,
        _ => return None,
    })
}

impl Lua {
    pub(crate) fn funcao_lua(&self, proto: Rc<Proto>, de_fora: Vec<Rc<RefCell<Value>>>, env: Table) -> Function {
        Function { d: Rc::new(CorpoDeFuncao::Lua { proto, de_fora, env: RefCell::new(env) }), e: Rc::downgrade(&self.e) }
    }

    pub(crate) fn erro(&self, m: impl std::fmt::Display) -> Error {
        let o = self.e.onde.borrow();
        Error::RuntimeError(format!("{}:{}: {m}", o.0, o.1))
    }

    pub(crate) fn metamethod(&self, v: &Value, nome: &str) -> Option<Value> {
        if let Value::UserData(u) = v {
            let m = metamethod_de_userdata(nome)?;
            return u.0.meta.metas.borrow().get(&m).cloned().map(Value::Function);
        }
        let m = meta_de(self, v)?;
        let r = m.d.borrow().pegar(&Value::texto(nome));
        (!r.is_nil()).then_some(r)
    }

    pub(crate) fn indexar(&self, o: &Value, k: &Value) -> Result<Value> {
        let mut atual = o.clone();
        for _ in 0..100 {
            let manipulador = match &atual {
                Value::Table(t) => {
                    let v = t.d.borrow().pegar(k);
                    if !v.is_nil() {
                        return Ok(v);
                    }
                    let meta = t.d.borrow().meta.clone();
                    let Some(m) = meta else { return Ok(Value::Nil) };
                    let h = m.d.borrow().pegar(&Value::texto("__index"));
                    if h.is_nil() {
                        return Ok(Value::Nil);
                    }
                    h
                }
                Value::UserData(u) => {
                    if let Value::String(s) = k {
                        let lido = u.0.meta.lidos.borrow().get(&*s.0).cloned();
                        if let Some(f) = lido {
                            return f(self, u);
                        }
                        if let Some(f) = u.0.meta.metodos.borrow().get(&*s.0) {
                            return Ok(Value::Function(f.clone()));
                        }
                    }
                    let h = u.0.meta.metas.borrow().get(&MetaMethod::Index).cloned();
                    let Some(f) = h else { return Ok(Value::Nil) };
                    let r = self.chamar_funcao(&f, vec![atual.clone(), k.clone()])?;
                    return Ok(r.into_iter().next().unwrap_or(Value::Nil));
                }
                Value::String(_) => match self.metamethod(&atual, "__index") {
                    Some(h) => h,
                    None => return Ok(Value::Nil),
                },
                v => return Err(self.erro(format!("attempt to index a {} value", v.nome_do_tipo()))),
            };
            if let Value::Function(f) = &manipulador {
                let r = self.chamar_funcao(f, vec![atual.clone(), k.clone()])?;
                return Ok(r.into_iter().next().unwrap_or(Value::Nil));
            }
            atual = manipulador;
        }
        Err(self.erro("__index em laço"))
    }

    pub(crate) fn indexar_gravar(&self, o: &Value, k: Value, v: Value) -> Result<()> {
        let mut atual = o.clone();
        for _ in 0..100 {
            let manipulador = match &atual {
                Value::Table(t) => {
                    let (existe, meta) = {
                        let d = t.d.borrow();
                        (!d.pegar(&k).is_nil(), d.meta.clone())
                    };
                    let h = match meta {
                        Some(m) if !existe => m.d.borrow().pegar(&Value::texto("__newindex")),
                        _ => Value::Nil,
                    };
                    if h.is_nil() {
                        if matches!(k, Value::Nil) {
                            return Err(self.erro("table index is nil"));
                        }
                        t.d.borrow_mut().por(k, v);
                        return Ok(());
                    }
                    h
                }
                Value::UserData(u) => {
                    if let Value::String(s) = &k {
                        let g = u.0.meta.gravados.borrow().get(&*s.0).cloned();
                        if let Some(f) = g {
                            return f(self, u, v);
                        }
                    }
                    let h = u.0.meta.metas.borrow().get(&MetaMethod::NewIndex).cloned();
                    let Some(f) = h else { return Err(self.erro("userdata sem esse campo")) };
                    self.chamar_funcao(&f, vec![atual.clone(), k, v])?;
                    return Ok(());
                }
                outro => return Err(self.erro(format!("attempt to index a {} value", outro.nome_do_tipo()))),
            };
            if let Value::Function(f) = &manipulador {
                self.chamar_funcao(f, vec![atual.clone(), k, v])?;
                return Ok(());
            }
            atual = manipulador;
        }
        Err(self.erro("__newindex em laço"))
    }

    pub(crate) fn chamar_valor(&self, f: &Value, mut args: Vec<Value>) -> Result<Vec<Value>> {
        match f {
            Value::Function(func) => self.chamar_funcao(func, args),
            outro => match self.metamethod(outro, "__call") {
                Some(h) => {
                    args.insert(0, outro.clone());
                    self.chamar_valor(&h, args)
                }
                None => Err(self.erro(format!("attempt to call a {} value", outro.nome_do_tipo()))),
            },
        }
    }

    pub(crate) fn chamar_funcao(&self, f: &Function, args: Vec<Value>) -> Result<Vec<Value>> {
        match &*f.d {
            CorpoDeFuncao::Rust(r) => r(self, args),
            CorpoDeFuncao::Lua { proto, de_fora, env } => {
                let p = self.e.profundidade.get();
                if p == 0 {
                    self.e.piso_da_pilha.set(piso_da_pilha());
                }
                let marca = 0u8;
                if std::ptr::addr_of!(marca) as usize <= self.e.piso_da_pilha.get() {
                    return Err(self.erro("stack overflow"));
                }
                self.e.profundidade.set(p + 1);
                let r = self.rodar(proto, de_fora, env, args);
                self.e.profundidade.set(p);
                r
            }
        }
    }

    fn rodar(&self, proto: &Proto, de_fora: &[Rc<RefCell<Value>>], env: &RefCell<Table>, mut args: Vec<Value>) -> Result<Vec<Value>> {
        let mut q = Quadro {
            slots: Vec::with_capacity(proto.slots as usize),
            varargs: Vec::new(),
            de_fora,
            env,
            proto,
            linha: 0,
        };
        q.slots.resize_with(proto.slots as usize, || Slot::Direto(Value::Nil));
        let n = proto.params.len();
        if proto.vararg && args.len() > n {
            q.varargs = args.split_off(n);
        }
        let mut a = args.into_iter();
        for v in &proto.params {
            declarar(&mut q, v, a.next().unwrap_or(Value::Nil));
        }
        match self.bloco(&proto.corpo, &mut q)? {
            Fluxo::Retorna(v) => Ok(v),
            _ => Ok(Vec::new()),
        }
    }

    fn bloco(&self, b: &[Linha], q: &mut Quadro) -> Result<Fluxo> {
        for Linha(l, s) in b {
            q.linha = *l;
            match self.comando(s, q)? {
                Fluxo::Segue => {}
                f => return Ok(f),
            }
        }
        Ok(Fluxo::Segue)
    }

    fn comando(&self, s: &Stat, q: &mut Quadro) -> Result<Fluxo> {
        match s {
            Stat::Local(vars, exprs) => {
                let mut vs = self.lista(exprs, q)?.into_iter();
                for v in vars {
                    declarar(q, v, vs.next().unwrap_or(Value::Nil));
                }
            }
            Stat::LocalFunc(v, p) => {
                declarar(q, v, Value::Nil);
                let f = self.fechar(p, q);
                gravar_local(q, v.slot, f);
            }
            Stat::Atrib(alvos, exprs) => {
                // O lado direito inteiro antes de qualquer escrita; índices avaliados antes também.
                if let ([a], [e]) = (&alvos[..], &exprs[..]) {
                    let v = self.expr(e, q)?;
                    self.atribuir(a, v, q)?;
                } else {
                    let mut chaves = Vec::with_capacity(alvos.len());
                    for a in alvos {
                        chaves.push(match a {
                            Alvo::Index(o, k) => Some((self.expr(o, q)?, self.expr(k, q)?)),
                            _ => None,
                        });
                    }
                    let mut vs = self.lista(exprs, q)?.into_iter();
                    for (a, c) in alvos.iter().zip(chaves) {
                        let v = vs.next().unwrap_or(Value::Nil);
                        match c {
                            Some((o, k)) => {
                                self.marcar(q);
                                self.indexar_gravar(&o, k, v)?;
                            }
                            None => self.atribuir(a, v, q)?,
                        }
                    }
                }
            }
            Stat::Chamada(e) => {
                self.multi(e, q)?;
            }
            Stat::Do(b) => return self.bloco(b, q),
            Stat::While(c, b) => {
                while self.expr(c, q)?.verdadeiro() {
                    match self.bloco(b, q)? {
                        Fluxo::Quebra => break,
                        Fluxo::Segue => {}
                        f => return Ok(f),
                    }
                }
            }
            Stat::Repeat(b, c) => loop {
                match self.bloco(b, q)? {
                    Fluxo::Quebra => break,
                    Fluxo::Segue => {}
                    f => return Ok(f),
                }
                if self.expr(c, q)?.verdadeiro() {
                    break;
                }
            },
            Stat::If(casos, senao) => {
                for (c, b) in casos {
                    if self.expr(c, q)?.verdadeiro() {
                        return self.bloco(b, q);
                    }
                }
                if let Some(b) = senao {
                    return self.bloco(b, q);
                }
            }
            Stat::ForNum(v, ini, fim, passo, b) => {
                let num = |lua: &Lua, x: Value, o: &str| crate::padrao::numero_de(&x).ok_or_else(|| lua.erro(format!("'for' {o} must be a number")));
                let i0 = self.expr(ini, q)?;
                let f0 = self.expr(fim, q)?;
                let p0 = match passo {
                    Some(p) => self.expr(p, q)?,
                    None => Value::Number(1.0),
                };
                self.marcar(q);
                let (mut i, f, p) = (num(self, i0, "initial value")?, num(self, f0, "limit")?, num(self, p0, "step")?);
                while (p > 0.0 && i <= f) || (p <= 0.0 && i >= f) {
                    declarar(q, v, Value::Number(i));
                    match self.bloco(b, q)? {
                        Fluxo::Quebra => break,
                        Fluxo::Segue => {}
                        f => return Ok(f),
                    }
                    i += p;
                }
            }
            Stat::ForIn(vars, exprs, b) => {
                let mut vs = self.lista(exprs, q)?.into_iter();
                let (f, estado, mut controle) = (vs.next().unwrap_or(Value::Nil), vs.next().unwrap_or(Value::Nil), vs.next().unwrap_or(Value::Nil));
                loop {
                    self.marcar(q);
                    let r = self.chamar_valor(&f, vec![estado.clone(), controle.clone()])?;
                    let mut r = r.into_iter();
                    let primeiro = r.next().unwrap_or(Value::Nil);
                    if primeiro.is_nil() {
                        break;
                    }
                    controle = primeiro.clone();
                    declarar(q, &vars[0], primeiro);
                    for v in &vars[1..] {
                        declarar(q, v, r.next().unwrap_or(Value::Nil));
                    }
                    match self.bloco(b, q)? {
                        Fluxo::Quebra => break,
                        Fluxo::Segue => {}
                        f => return Ok(f),
                    }
                }
            }
            Stat::Return(exprs) => return Ok(Fluxo::Retorna(self.lista(exprs, q)?)),
            Stat::Break => return Ok(Fluxo::Quebra),
        }
        Ok(Fluxo::Segue)
    }

    /// Guarda onde o mod está, para a mensagem de erro e para o `error` nível 1.
    fn marcar(&self, q: &Quadro) {
        let mut o = self.e.onde.borrow_mut();
        o.1 = q.linha;
        if !Rc::ptr_eq(&o.0, &q.proto.chunk) {
            o.0 = q.proto.chunk.clone();
        }
    }

    fn atribuir(&self, a: &Alvo, v: Value, q: &mut Quadro) -> Result<()> {
        match a {
            Alvo::Local(i) => gravar_local(q, *i, v),
            Alvo::Fora(i) => *q.de_fora[*i as usize].borrow_mut() = v,
            Alvo::Global(n) => {
                self.marcar(q);
                let env = q.env.borrow().clone();
                self.indexar_gravar(&Value::Table(env), Value::String(crate::LuaString(n.clone())), v)?;
            }
            Alvo::Index(o, k) => {
                let (o, k) = (self.expr(o, q)?, self.expr(k, q)?);
                self.marcar(q);
                self.indexar_gravar(&o, k, v)?;
            }
        }
        Ok(())
    }

    fn fechar(&self, p: &Rc<Proto>, q: &mut Quadro) -> Value {
        let mut de_fora = Vec::with_capacity(p.de_fora.len());
        for o in &p.de_fora {
            de_fora.push(if o.do_pai {
                match &q.slots[o.indice as usize] {
                    Slot::Caixa(c) => c.clone(),
                    // Slot ainda não declarado (`local function` recursiva chega declarada; aqui é só defesa).
                    Slot::Direto(v) => Rc::new(RefCell::new(v.clone())),
                }
            } else {
                q.de_fora[o.indice as usize].clone()
            });
        }
        Value::Function(self.funcao_lua(p.clone(), de_fora, q.env.borrow().clone()))
    }

    /// Lista de expressões: a última expande todos os valores.
    fn lista(&self, exprs: &[Expr], q: &mut Quadro) -> Result<Vec<Value>> {
        let mut v = Vec::with_capacity(exprs.len());
        if let Some((ult, resto)) = exprs.split_last() {
            for e in resto {
                v.push(self.expr(e, q)?);
            }
            match ult {
                Expr::Chamada(..) | Expr::Metodo(..) | Expr::Vararg => v.extend(self.multi(ult, q)?),
                e => v.push(self.expr(e, q)?),
            }
        }
        Ok(v)
    }

    fn multi(&self, e: &Expr, q: &mut Quadro) -> Result<Vec<Value>> {
        match e {
            Expr::Chamada(f, args) => {
                let fv = self.expr(f, q)?;
                let a = self.lista(args, q)?;
                self.marcar(q);
                if !matches!(fv, Value::Function(_)) && self.metamethod(&fv, "__call").is_none() {
                    return Err(self.erro(format!("attempt to call {} (a {} value)", descrever(f), fv.nome_do_tipo())));
                }
                self.chamar_valor(&fv, a)
            }
            Expr::Metodo(o, nome, args) => {
                let ov = self.expr(o, q)?;
                self.marcar(q);
                let fv = self.indexar(&ov, &Value::String(crate::LuaString(nome.clone())))?;
                let mut a = Vec::with_capacity(args.len() + 1);
                a.push(ov);
                a.extend(self.lista(args, q)?);
                self.marcar(q);
                if !matches!(fv, Value::Function(_)) && self.metamethod(&fv, "__call").is_none() {
                    return Err(self.erro(format!("attempt to call method '{}' (a {} value)", String::from_utf8_lossy(nome), fv.nome_do_tipo())));
                }
                self.chamar_valor(&fv, a)
            }
            Expr::Vararg => Ok(q.varargs.clone()),
            e => Ok(vec![self.expr(e, q)?]),
        }
    }

    fn expr(&self, e: &Expr, q: &mut Quadro) -> Result<Value> {
        Ok(match e {
            Expr::Nil => Value::Nil,
            Expr::True => Value::Boolean(true),
            Expr::False => Value::Boolean(false),
            Expr::Num(n) => Value::Number(*n),
            Expr::Str(s) => Value::String(crate::LuaString(s.clone())),
            Expr::Vararg => q.varargs.first().cloned().unwrap_or(Value::Nil),
            Expr::Func(p) => self.fechar(p, q),
            Expr::Local(i) => match &q.slots[*i as usize] {
                Slot::Direto(v) => v.clone(),
                Slot::Caixa(c) => c.borrow().clone(),
            },
            Expr::Fora(i) => q.de_fora[*i as usize].borrow().clone(),
            Expr::Global(n) => {
                self.marcar(q);
                let env = q.env.borrow().clone();
                self.indexar(&Value::Table(env), &Value::String(crate::LuaString(n.clone())))?
            }
            Expr::Index(o, k) => {
                let ov = self.expr(o, q)?;
                let kv = self.expr(k, q)?;
                self.marcar(q);
                if !matches!(ov, Value::Table(_) | Value::UserData(_) | Value::String(_)) {
                    return Err(self.erro(format!("attempt to index {} (a {} value)", descrever(o), ov.nome_do_tipo())));
                }
                self.indexar(&ov, &kv)?
            }
            Expr::Chamada(..) | Expr::Metodo(..) => self.multi(e, q)?.into_iter().next().unwrap_or(Value::Nil),
            Expr::Bin(op, a, b) => {
                let (x, y) = (self.expr(a, q)?, self.expr(b, q)?);
                self.marcar(q);
                self.binario(*op, x, y)?
            }
            Expr::Un(op, a) => {
                let x = self.expr(a, q)?;
                self.marcar(q);
                self.unario(*op, x)?
            }
            Expr::E(a, b) => {
                let x = self.expr(a, q)?;
                if x.verdadeiro() { self.expr(b, q)? } else { x }
            }
            Expr::Ou(a, b) => {
                let x = self.expr(a, q)?;
                if x.verdadeiro() { x } else { self.expr(b, q)? }
            }
            Expr::Tabela(campos) => self.tabela(campos, q)?,
            Expr::Parenteses(a) => self.expr(a, q)?,
        })
    }

    fn tabela(&self, campos: &[Campo], q: &mut Quadro) -> Result<Value> {
        let t = self.tabela_nova();
        let mut pos = 1.0;
        for (i, c) in campos.iter().enumerate() {
            match c {
                Campo::Pos(e) if i + 1 == campos.len() && matches!(e, Expr::Chamada(..) | Expr::Metodo(..) | Expr::Vararg) => {
                    for v in self.multi(e, q)? {
                        t.d.borrow_mut().por(Value::Number(pos), v);
                        pos += 1.0;
                    }
                }
                Campo::Pos(e) => {
                    let v = self.expr(e, q)?;
                    t.d.borrow_mut().por(Value::Number(pos), v);
                    pos += 1.0;
                }
                Campo::Chave(k, e) => {
                    let (kv, v) = (self.expr(k, q)?, self.expr(e, q)?);
                    if kv.is_nil() {
                        self.marcar(q);
                        return Err(self.erro("table index is nil"));
                    }
                    t.d.borrow_mut().por(kv, v);
                }
            }
        }
        Ok(Value::Table(t))
    }

    fn por_metamethod(&self, nome: &str, x: &Value, y: &Value) -> Option<Result<Value>> {
        let h = self.metamethod(x, nome).or_else(|| self.metamethod(y, nome))?;
        Some(self.chamar_valor(&h, vec![x.clone(), y.clone()]).map(|r| r.into_iter().next().unwrap_or(Value::Nil)))
    }

    fn binario(&self, op: Bin, x: Value, y: Value) -> Result<Value> {
        use crate::padrao::numero_de;
        match op {
            Bin::Soma | Bin::Sub | Bin::Mul | Bin::Div | Bin::Resto | Bin::Pot => {
                if let (Some(a), Some(b)) = (numero_de(&x), numero_de(&y)) {
                    return Ok(Value::Number(match op {
                        Bin::Soma => a + b,
                        Bin::Sub => a - b,
                        Bin::Mul => a * b,
                        Bin::Div => a / b,
                        Bin::Resto => a - (a / b).floor() * b,
                        _ => a.powf(b),
                    }));
                }
                let nome = match op {
                    Bin::Soma => "__add",
                    Bin::Sub => "__sub",
                    Bin::Mul => "__mul",
                    Bin::Div => "__div",
                    Bin::Resto => "__mod",
                    _ => "__pow",
                };
                self.por_metamethod(nome, &x, &y).unwrap_or_else(|| {
                    let ruim = if numero_de(&x).is_none() { &x } else { &y };
                    Err(self.erro(format!("attempt to perform arithmetic on a {} value", ruim.nome_do_tipo())))
                })
            }
            Bin::Concat => {
                let texto = |v: &Value| match v {
                    Value::String(s) => Some(s.0.to_vec()),
                    Value::Number(_) | Value::Integer(_) => v.to_string().ok().map(String::into_bytes),
                    _ => None,
                };
                if let (Some(mut a), Some(b)) = (texto(&x), texto(&y)) {
                    a.extend_from_slice(&b);
                    return Ok(Value::String(crate::LuaString(Rc::from(a))));
                }
                self.por_metamethod("__concat", &x, &y).unwrap_or_else(|| {
                    let ruim = if texto(&x).is_none() { &x } else { &y };
                    Err(self.erro(format!("attempt to concatenate a {} value", ruim.nome_do_tipo())))
                })
            }
            Bin::Igual => Ok(Value::Boolean(self.iguais(&x, &y)?)),
            Bin::Dif => Ok(Value::Boolean(!self.iguais(&x, &y)?)),
            Bin::Menor => Ok(Value::Boolean(self.menor(&x, &y)?)),
            Bin::Maior => Ok(Value::Boolean(self.menor(&y, &x)?)),
            Bin::MenorIg => Ok(Value::Boolean(self.menor_ou_igual(&x, &y)?)),
            Bin::MaiorIg => Ok(Value::Boolean(self.menor_ou_igual(&y, &x)?)),
        }
    }

    pub(crate) fn iguais(&self, x: &Value, y: &Value) -> Result<bool> {
        if crate::padrao::iguais_cru(x, y) {
            return Ok(true);
        }
        match (x, y) {
            (Value::Table(_), Value::Table(_)) | (Value::UserData(_), Value::UserData(_)) => match self.por_metamethod("__eq", x, y) {
                Some(r) => Ok(r?.verdadeiro()),
                None => Ok(false),
            },
            _ => Ok(false),
        }
    }

    pub(crate) fn menor(&self, x: &Value, y: &Value) -> Result<bool> {
        use crate::padrao::numero_cru;
        match (numero_cru(x), numero_cru(y), x, y) {
            (Some(a), Some(b), ..) => Ok(a < b),
            (_, _, Value::String(a), Value::String(b)) => Ok(a.0 < b.0),
            _ => match self.por_metamethod("__lt", x, y) {
                Some(r) => Ok(r?.verdadeiro()),
                None => Err(self.erro(format!("attempt to compare {} with {}", x.nome_do_tipo(), y.nome_do_tipo()))),
            },
        }
    }

    fn menor_ou_igual(&self, x: &Value, y: &Value) -> Result<bool> {
        use crate::padrao::numero_cru;
        match (numero_cru(x), numero_cru(y), x, y) {
            (Some(a), Some(b), ..) => Ok(a <= b),
            (_, _, Value::String(a), Value::String(b)) => Ok(a.0 <= b.0),
            _ => match self.por_metamethod("__le", x, y) {
                Some(r) => Ok(r?.verdadeiro()),
                None => match self.por_metamethod("__lt", y, x) {
                    Some(r) => Ok(!r?.verdadeiro()),
                    None => Err(self.erro(format!("attempt to compare {} with {}", x.nome_do_tipo(), y.nome_do_tipo()))),
                },
            },
        }
    }

    fn unario(&self, op: Un, x: Value) -> Result<Value> {
        match op {
            Un::Not => Ok(Value::Boolean(!x.verdadeiro())),
            Un::Neg => match crate::padrao::numero_de(&x) {
                Some(n) => Ok(Value::Number(-n)),
                None => match self.metamethod(&x, "__unm") {
                    Some(h) => Ok(self.chamar_valor(&h, vec![x.clone(), x])?.into_iter().next().unwrap_or(Value::Nil)),
                    None => Err(self.erro(format!("attempt to perform arithmetic on a {} value", x.nome_do_tipo()))),
                },
            },
            Un::Len => match &x {
                Value::String(s) => Ok(Value::Number(s.0.len() as f64)),
                Value::Table(t) => Ok(Value::Number(t.d.borrow().arr.len() as f64)),
                _ => match self.metamethod(&x, "__len") {
                    Some(h) => Ok(self.chamar_valor(&h, vec![x.clone()])?.into_iter().next().unwrap_or(Value::Nil)),
                    None => Err(self.erro(format!("attempt to get length of a {} value", x.nome_do_tipo()))),
                },
            },
        }
    }
}

/// Texto para a mensagem de erro: o nome do que o mod tentou usar.
fn descrever(e: &Expr) -> String {
    match e {
        Expr::Global(n) => format!("global '{}'", String::from_utf8_lossy(n)),
        Expr::Index(_, k) => match &**k {
            Expr::Str(s) => format!("field '{}'", String::from_utf8_lossy(s)),
            _ => "a field".into(),
        },
        Expr::Local(_) => "a local".into(),
        Expr::Fora(_) => "an upvalue".into(),
        _ => "a value".into(),
    }
}

fn declarar(q: &mut Quadro, v: &Var, valor: Value) {
    q.slots[v.slot as usize] = if v.capturada.get() { Slot::Caixa(Rc::new(RefCell::new(valor))) } else { Slot::Direto(valor) };
}

fn gravar_local(q: &mut Quadro, i: u16, valor: Value) {
    match &mut q.slots[i as usize] {
        Slot::Direto(v) => *v = valor,
        Slot::Caixa(c) => *c.borrow_mut() = valor,
    }
}
