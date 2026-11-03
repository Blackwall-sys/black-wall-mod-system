//! `bwms-lua`: lê e executa os `.lua` dos mods de Windows, em Rust do projeto, sem nenhuma dependência.
//!
//! A árvore do fonte é andada direto (`exec`); valores, tabelas e funções moram aqui. Erro é `Result` do começo ao fim, sem desenrolar a pilha.
//! Por dentro todo número é `f64`, como na linguagem; `Value::Integer` só existe na fronteira com o Rust, para o número inteiro chegar como
//! inteiro a quem chama.

mod casador;
mod exec;
mod json;
mod lexer;
mod padrao;
mod parser;

use std::any::{Any, TypeId};
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug)]
pub enum Error {
    RuntimeError(String),
    /// `error(valor)` com algo que não é texto: o `pcall` devolve o valor.
    Valor(Box<Value>),
}

impl Error {
    pub fn external<E: std::fmt::Display>(e: E) -> Self {
        Error::RuntimeError(e.to_string())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::RuntimeError(m) => f.write_str(m),
            Error::Valor(v) => write!(f, "{}", v.nome_do_tipo()),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LuaString(pub(crate) Rc<[u8]>);

impl LuaString {
    pub fn to_str(&self) -> Result<std::borrow::Cow<'_, str>> {
        std::str::from_utf8(&self.0).map(std::borrow::Cow::Borrowed).map_err(|_| Error::RuntimeError("texto não é UTF-8".into()))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub enum Value {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(LuaString),
    Table(Table),
    Function(Function),
    UserData(AnyUserData),
}

impl Value {
    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    pub(crate) fn verdadeiro(&self) -> bool {
        !matches!(self, Value::Nil | Value::Boolean(false))
    }

    pub(crate) fn texto(s: &str) -> Value {
        Value::String(LuaString(Rc::from(s.as_bytes())))
    }

    pub(crate) fn nome_do_tipo(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Boolean(_) => "boolean",
            Value::Integer(_) | Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Table(_) => "table",
            Value::Function(_) => "function",
            Value::UserData(_) => "userdata",
        }
    }

    /// Como o `tostring` do Lua sem metamétodo de tabela; userdata pelo `__tostring` do tipo.
    pub fn to_string(&self) -> Result<String> {
        Ok(match self {
            Value::Nil => "nil".into(),
            Value::Boolean(b) => b.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Number(n) => padrao::numero_em_texto(*n),
            Value::String(s) => String::from_utf8_lossy(&s.0).into_owned(),
            Value::Table(t) => format!("table: {:p}", Rc::as_ptr(&t.d)),
            Value::Function(f) => format!("function: {:p}", Rc::as_ptr(&f.d)),
            Value::UserData(u) => match (u.0.meta.metas.borrow().get(&MetaMethod::ToString), u.0.meta.lua.upgrade()) {
                (Some(f), Some(e)) => {
                    let r = f.chamar_com(&Lua { e }, vec![self.clone()])?;
                    r.first().map_or(Ok(String::new()), |v| v.to_string())?
                }
                _ => format!("userdata: {:p}", Rc::as_ptr(&u.0)),
            },
        })
    }
}

/// Número na fronteira com o Rust: inteiro exato sai como `Integer`.
pub(crate) fn para_rust(v: Value) -> Value {
    match v {
        Value::Number(n) if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 => Value::Integer(n as i64),
        v => v,
    }
}

pub(crate) fn de_rust(v: Value) -> Value {
    match v {
        Value::Integer(i) => Value::Number(i as f64),
        v => v,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Chave {
    Bool(bool),
    Num(u64),
    Str(Rc<[u8]>),
    Ptr(usize),
}

pub(crate) fn chave_de(v: &Value) -> Option<Chave> {
    Some(match v {
        Value::Nil => return None,
        Value::Boolean(b) => Chave::Bool(*b),
        Value::Integer(i) => Chave::Num((*i as f64).to_bits()),
        Value::Number(n) if n.is_nan() => return None,
        // +0 e -0 são a mesma chave.
        Value::Number(n) => Chave::Num((*n + 0.0).to_bits()),
        Value::String(s) => Chave::Str(s.0.clone()),
        Value::Table(t) => Chave::Ptr(Rc::as_ptr(&t.d) as *const u8 as usize),
        Value::Function(f) => Chave::Ptr(Rc::as_ptr(&f.d) as *const u8 as usize),
        Value::UserData(u) => Chave::Ptr(Rc::as_ptr(&u.0) as *const u8 as usize),
    })
}

/// Tabela: parte de lista (`arr`, índices 1..n) e parte de mapa em ordem de inserção. Chave apagada fica com valor `Nil` no lugar: o `next`
/// segue valendo se o mod apaga enquanto percorre.
#[derive(Default)]
pub(crate) struct Dados {
    pub(crate) arr: Vec<Value>,
    pub(crate) mapa: HashMap<Chave, usize>,
    pub(crate) itens: Vec<(Value, Value)>,
    pub(crate) meta: Option<Table>,
}

impl std::fmt::Debug for Dados {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tabela({} + {})", self.arr.len(), self.itens.len())
    }
}

fn indice_de_lista(k: &Value) -> Option<usize> {
    match k {
        Value::Number(n) if *n >= 1.0 && n.fract() == 0.0 && *n < 4.0e15 => Some(*n as usize),
        _ => None,
    }
}

impl Dados {
    pub(crate) fn pegar(&self, k: &Value) -> Value {
        if let Some(i) = indice_de_lista(k) {
            if let Some(v) = self.arr.get(i - 1) {
                return v.clone();
            }
        }
        match chave_de(k).and_then(|c| self.mapa.get(&c)) {
            Some(&i) => self.itens[i].1.clone(),
            None => Value::Nil,
        }
    }

    pub(crate) fn por(&mut self, k: Value, v: Value) {
        if let Some(i) = indice_de_lista(&k) {
            if i <= self.arr.len() {
                self.arr[i - 1] = v;
                while matches!(self.arr.last(), Some(Value::Nil)) {
                    self.arr.pop();
                }
                return;
            }
            if i == self.arr.len() + 1 && !v.is_nil() {
                self.arr.push(v);
                self.trazer_para_a_lista();
                return;
            }
        }
        let Some(c) = chave_de(&k) else { return };
        match self.mapa.get(&c) {
            Some(&i) => self.itens[i].1 = v,
            None if v.is_nil() => {}
            None => {
                self.mapa.insert(c, self.itens.len());
                self.itens.push((k, v));
            }
        }
    }

    /// Índices seguintes que estavam no mapa passam para a lista.
    fn trazer_para_a_lista(&mut self) {
        loop {
            let k = Value::Number((self.arr.len() + 1) as f64);
            let Some(c) = chave_de(&k) else { return };
            let Some(&i) = self.mapa.get(&c) else { return };
            if self.itens[i].1.is_nil() {
                return;
            }
            let v = std::mem::replace(&mut self.itens[i].1, Value::Nil);
            self.arr.push(v);
        }
    }

    /// `next`: o par depois de `k` (lista, depois mapa), pulando valores nulos. `Err` com chave que não existe.
    pub(crate) fn proximo(&self, k: &Value) -> std::result::Result<Option<(Value, Value)>, ()> {
        let mut i_mapa = 0;
        match k {
            Value::Nil => {
                if let Some(p) = self.par_da_lista(0) {
                    return Ok(Some(p));
                }
            }
            _ => match indice_de_lista(k).filter(|i| *i <= self.arr.len()) {
                Some(i) => {
                    if let Some(p) = self.par_da_lista(i) {
                        return Ok(Some(p));
                    }
                }
                None => match (chave_de(k).and_then(|c| self.mapa.get(&c)), indice_de_lista(k)) {
                    (Some(&i), _) => i_mapa = i + 1,
                    // Índice que a lista perdeu no meio do percurso (`t[k] = nil` no último): segue para o mapa.
                    (None, Some(_)) => {}
                    (None, None) => return Err(()),
                },
            },
        }
        Ok(self.itens[i_mapa..].iter().find(|(_, v)| !v.is_nil()).cloned())
    }

    fn par_da_lista(&self, de: usize) -> Option<(Value, Value)> {
        (de..self.arr.len()).find(|&i| !self.arr[i].is_nil()).map(|i| (Value::Number((i + 1) as f64), self.arr[i].clone()))
    }
}

#[derive(Clone, Debug)]
pub struct Table {
    pub(crate) d: Rc<RefCell<Dados>>,
    pub(crate) e: Weak<Estado>,
}

impl Table {
    fn lua(&self) -> Result<Lua> {
        self.e.upgrade().map(|e| Lua { e }).ok_or_else(|| Error::RuntimeError("estado do Lua desfeito".into()))
    }

    pub fn set<K: IntoLua, V: IntoLua>(&self, k: K, v: V) -> Result<()> {
        let lua = self.lua()?;
        let (k, v) = (de_rust(k.into_lua(&lua)?), de_rust(v.into_lua(&lua)?));
        lua.indexar_gravar(&Value::Table(self.clone()), k, v)
    }

    pub fn get<V: FromLua>(&self, k: impl IntoLua) -> Result<V> {
        let lua = self.lua()?;
        let k = de_rust(k.into_lua(&lua)?);
        let v = lua.indexar(&Value::Table(self.clone()), &k)?;
        V::from_lua(para_rust(v), &lua)
    }

    pub fn raw_len(&self) -> usize {
        self.d.borrow().arr.len()
    }

    pub fn metatable(&self) -> Option<Table> {
        self.d.borrow().meta.clone()
    }

    pub fn set_metatable(&self, m: Option<Table>) {
        self.d.borrow_mut().meta = m;
    }

    pub fn push(&self, v: impl IntoLua) -> Result<()> {
        let lua = self.lua()?;
        let v = de_rust(v.into_lua(&lua)?);
        let n = self.d.borrow().arr.len();
        self.d.borrow_mut().por(Value::Number((n + 1) as f64), v);
        Ok(())
    }

    pub fn pairs<K: FromLua, V: FromLua>(&self) -> impl Iterator<Item = Result<(K, V)>> {
        let lua = self.lua();
        let t = self.clone();
        let mut k = Value::Nil;
        std::iter::from_fn(move || {
            let lua = match &lua {
                Ok(l) => l,
                Err(e) => return Some(Err(e.clone())),
            };
            let par = t.d.borrow().proximo(&k);
            match par {
                Ok(Some((nk, v))) => {
                    k = nk.clone();
                    Some(K::from_lua(para_rust(nk), lua).and_then(|a| Ok((a, V::from_lua(para_rust(v), lua)?))))
                }
                _ => None,
            }
        })
    }

    pub fn sequence_values<V: FromLua>(&self) -> impl Iterator<Item = Result<V>> {
        let lua = self.lua();
        let t = self.clone();
        let mut i = 0;
        std::iter::from_fn(move || {
            let lua = match &lua {
                Ok(l) => l,
                Err(e) => return Some(Err(e.clone())),
            };
            let v = t.d.borrow().arr.get(i).cloned()?;
            i += 1;
            Some(V::from_lua(para_rust(v), lua))
        })
    }
}

pub(crate) type FnRust = dyn Fn(&Lua, Vec<Value>) -> Result<Vec<Value>>;

pub(crate) enum CorpoDeFuncao {
    Lua { proto: Rc<parser::Proto>, de_fora: Vec<Rc<RefCell<Value>>>, env: RefCell<Table> },
    Rust(Box<FnRust>),
}

impl std::fmt::Debug for CorpoDeFuncao {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("função")
    }
}

#[derive(Clone, Debug)]
pub struct Function {
    pub(crate) d: Rc<CorpoDeFuncao>,
    pub(crate) e: Weak<Estado>,
}

impl Function {
    pub(crate) fn chamar_com(&self, lua: &Lua, args: Vec<Value>) -> Result<Vec<Value>> {
        lua.chamar_funcao(self, args)
    }

    pub fn call<R: FromLuaMulti>(&self, args: impl IntoLuaMulti) -> Result<R> {
        let lua = self.e.upgrade().map(|e| Lua { e }).ok_or_else(|| Error::RuntimeError("estado do Lua desfeito".into()))?;
        let a = args.into_lua_multi(&lua)?.into_iter().map(de_rust).collect();
        let r = lua.chamar_funcao(self, a)?;
        R::from_lua_multi(r.into_iter().map(para_rust).collect(), &lua)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetaMethod {
    Index,
    NewIndex,
    ToString,
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Lt,
    Le,
    Len,
    Concat,
    Call,
    Unm,
}

type Lido = Rc<dyn Fn(&Lua, &AnyUserData) -> Result<Value>>;
type Gravado = Rc<dyn Fn(&Lua, &AnyUserData, Value) -> Result<()>>;

/// O que um tipo de userdata oferece ao Lua.
pub(crate) struct MetaDeTipo {
    pub(crate) lua: Weak<Estado>,
    pub(crate) metodos: RefCell<HashMap<Vec<u8>, Function>>,
    pub(crate) metas: RefCell<HashMap<MetaMethod, Function>>,
    pub(crate) lidos: RefCell<HashMap<Vec<u8>, Lido>>,
    pub(crate) gravados: RefCell<HashMap<Vec<u8>, Gravado>>,
}

pub(crate) struct DadosDeUserdata {
    pub(crate) valor: RefCell<Box<dyn Any>>,
    pub(crate) meta: Rc<MetaDeTipo>,
}

#[derive(Clone)]
pub struct AnyUserData(pub(crate) Rc<DadosDeUserdata>);

impl std::fmt::Debug for AnyUserData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("userdata")
    }
}

impl AnyUserData {
    pub fn is<T: 'static>(&self) -> bool {
        self.0.valor.try_borrow().is_ok_and(|b| b.is::<T>())
    }

    pub fn borrow<T: 'static>(&self) -> Result<Ref<'_, T>> {
        let g = self.0.valor.try_borrow().map_err(|_| Error::RuntimeError("userdata em uso".into()))?;
        Ref::filter_map(g, |b| b.downcast_ref::<T>()).map_err(|_| Error::RuntimeError("userdata de outro tipo".into()))
    }

    pub fn borrow_mut<T: 'static>(&self) -> Result<RefMut<'_, T>> {
        let g = self.0.valor.try_borrow_mut().map_err(|_| Error::RuntimeError("userdata em uso".into()))?;
        RefMut::filter_map(g, |b| b.downcast_mut::<T>()).map_err(|_| Error::RuntimeError("userdata de outro tipo".into()))
    }
}

pub trait UserData: Sized + 'static {
    fn add_fields<F: UserDataFields<Self>>(_f: &mut F) {}
    fn add_methods<M: UserDataMethods<Self>>(_m: &mut M) {}
}

pub trait UserDataMethods<T> {
    fn add_method<A, R, F>(&mut self, nome: &str, f: F)
    where
        A: FromLuaMulti + 'static,
        R: IntoLuaMulti + 'static,
        F: Fn(&Lua, &T, A) -> Result<R> + 'static;
    fn add_meta_method<A, R, F>(&mut self, m: MetaMethod, f: F)
    where
        A: FromLuaMulti + 'static,
        R: IntoLuaMulti + 'static,
        F: Fn(&Lua, &T, A) -> Result<R> + 'static;
    fn add_meta_method_mut<A, R, F>(&mut self, m: MetaMethod, f: F)
    where
        A: FromLuaMulti + 'static,
        R: IntoLuaMulti + 'static,
        F: Fn(&Lua, &mut T, A) -> Result<R> + 'static;
}

pub trait UserDataFields<T> {
    fn add_field_method_get<R, F>(&mut self, nome: &str, f: F)
    where
        R: IntoLua + 'static,
        F: Fn(&Lua, &T) -> Result<R> + 'static;
    fn add_field_method_set<A, F>(&mut self, nome: &str, f: F)
    where
        A: FromLua + 'static,
        F: Fn(&Lua, &mut T, A) -> Result<()> + 'static;
}

struct Construtor<T> {
    lua: Lua,
    meta: Rc<MetaDeTipo>,
    _t: std::marker::PhantomData<T>,
}

impl<T: 'static> Construtor<T> {
    fn funcao_de_metodo<A, R, F>(&self, f: F) -> Function
    where
        A: FromLuaMulti + 'static,
        R: IntoLuaMulti + 'static,
        F: Fn(&Lua, &T, A) -> Result<R> + 'static,
    {
        self.lua.funcao_rust(Box::new(move |lua, mut args| {
            let eu = primeiro_userdata(&mut args)?;
            let a = A::from_lua_multi(args.into_iter().map(para_rust).collect(), lua)?;
            let g = eu.borrow::<T>()?;
            let r = f(lua, &g, a)?;
            drop(g);
            Ok(r.into_lua_multi(lua)?.into_iter().map(de_rust).collect())
        }))
    }
}

fn primeiro_userdata(args: &mut Vec<Value>) -> Result<AnyUserData> {
    if args.is_empty() {
        return Err(Error::RuntimeError("método chamado sem o objeto: use `:`".into()));
    }
    match args.remove(0) {
        Value::UserData(u) => Ok(u),
        v => Err(Error::RuntimeError(format!("método chamado sobre {}: use `:` no objeto", v.nome_do_tipo()))),
    }
}

impl<T: 'static> UserDataMethods<T> for Construtor<T> {
    fn add_method<A, R, F>(&mut self, nome: &str, f: F)
    where
        A: FromLuaMulti + 'static,
        R: IntoLuaMulti + 'static,
        F: Fn(&Lua, &T, A) -> Result<R> + 'static,
    {
        let func = self.funcao_de_metodo(f);
        self.meta.metodos.borrow_mut().insert(nome.as_bytes().to_vec(), func);
    }

    fn add_meta_method<A, R, F>(&mut self, m: MetaMethod, f: F)
    where
        A: FromLuaMulti + 'static,
        R: IntoLuaMulti + 'static,
        F: Fn(&Lua, &T, A) -> Result<R> + 'static,
    {
        let func = self.funcao_de_metodo(f);
        self.meta.metas.borrow_mut().insert(m, func);
    }

    fn add_meta_method_mut<A, R, F>(&mut self, m: MetaMethod, f: F)
    where
        A: FromLuaMulti + 'static,
        R: IntoLuaMulti + 'static,
        F: Fn(&Lua, &mut T, A) -> Result<R> + 'static,
    {
        let func = self.lua.funcao_rust(Box::new(move |lua, mut args| {
            let eu = primeiro_userdata(&mut args)?;
            let a = A::from_lua_multi(args.into_iter().map(para_rust).collect(), lua)?;
            let mut g = eu.borrow_mut::<T>()?;
            let r = f(lua, &mut g, a)?;
            drop(g);
            Ok(r.into_lua_multi(lua)?.into_iter().map(de_rust).collect())
        }));
        self.meta.metas.borrow_mut().insert(m, func);
    }
}

impl<T: 'static> UserDataFields<T> for Construtor<T> {
    fn add_field_method_get<R, F>(&mut self, nome: &str, f: F)
    where
        R: IntoLua + 'static,
        F: Fn(&Lua, &T) -> Result<R> + 'static,
    {
        let lido: Lido = Rc::new(move |lua, u| {
            let g = u.borrow::<T>()?;
            let r = f(lua, &g)?;
            drop(g);
            Ok(de_rust(r.into_lua(lua)?))
        });
        self.meta.lidos.borrow_mut().insert(nome.as_bytes().to_vec(), lido);
    }

    fn add_field_method_set<A, F>(&mut self, nome: &str, f: F)
    where
        A: FromLua + 'static,
        F: Fn(&Lua, &mut T, A) -> Result<()> + 'static,
    {
        let gravado: Gravado = Rc::new(move |lua, u, v| {
            let a = A::from_lua(para_rust(v), lua)?;
            f(lua, &mut *u.borrow_mut::<T>()?, a)
        });
        self.meta.gravados.borrow_mut().insert(nome.as_bytes().to_vec(), gravado);
    }
}

/// Valor guardado no registro do estado (`create_registry_value`).
#[derive(Debug)]
pub struct RegistryKey(usize);

pub struct Estado {
    pub(crate) globais: RefCell<Option<Table>>,
    pub(crate) registro: RefCell<Vec<Value>>,
    pub(crate) meta_de_texto: RefCell<Option<Table>>,
    pub(crate) tipos: RefCell<HashMap<TypeId, Rc<MetaDeTipo>>>,
    pub(crate) onde: RefCell<(Rc<str>, u32)>,
    pub(crate) profundidade: Cell<u32>,
    pub(crate) piso_da_pilha: Cell<usize>,
    pub(crate) saida: RefCell<Option<Box<dyn Fn(&str)>>>,
}

impl std::fmt::Debug for Estado {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("estado do Lua")
    }
}

#[derive(Clone)]
pub struct Lua {
    pub(crate) e: Rc<Estado>,
}

impl Default for Lua {
    fn default() -> Self {
        Self::new()
    }
}

impl Lua {
    pub fn new() -> Self {
        let e = Rc::new(Estado {
            globais: RefCell::new(None),
            registro: RefCell::new(Vec::new()),
            meta_de_texto: RefCell::new(None),
            tipos: RefCell::new(HashMap::new()),
            onde: RefCell::new((Rc::from("?"), 0)),
            profundidade: Cell::new(0),
            piso_da_pilha: Cell::new(0),
            saida: RefCell::new(None),
        });
        let lua = Lua { e };
        let g = lua.tabela_nova();
        *lua.e.globais.borrow_mut() = Some(g);
        padrao::instalar(&lua);
        lua
    }

    pub(crate) fn tabela_nova(&self) -> Table {
        Table { d: Rc::new(RefCell::new(Dados::default())), e: Rc::downgrade(&self.e) }
    }

    pub(crate) fn funcao_rust(&self, f: Box<FnRust>) -> Function {
        Function { d: Rc::new(CorpoDeFuncao::Rust(f)), e: Rc::downgrade(&self.e) }
    }

    pub fn globals(&self) -> Table {
        self.e.globais.borrow().clone().expect("posta no Lua::new")
    }

    /// Para onde vai o `print` dos mods.
    pub fn set_saida(&self, f: Box<dyn Fn(&str)>) {
        *self.e.saida.borrow_mut() = Some(f);
    }

    pub fn create_table(&self) -> Result<Table> {
        Ok(self.tabela_nova())
    }

    pub fn create_string(&self, s: impl AsRef<[u8]>) -> Result<LuaString> {
        Ok(LuaString(Rc::from(s.as_ref())))
    }

    pub fn create_function<A, R, F>(&self, f: F) -> Result<Function>
    where
        A: FromLuaMulti + 'static,
        R: IntoLuaMulti + 'static,
        F: Fn(&Lua, A) -> Result<R> + 'static,
    {
        Ok(self.funcao_rust(Box::new(move |lua, args| {
            let a = A::from_lua_multi(args.into_iter().map(para_rust).collect(), lua)?;
            Ok(f(lua, a)?.into_lua_multi(lua)?.into_iter().map(de_rust).collect())
        })))
    }

    pub fn create_userdata<T: UserData>(&self, v: T) -> Result<AnyUserData> {
        let id = TypeId::of::<T>();
        let existente = self.e.tipos.borrow().get(&id).cloned();
        let meta = match existente {
            Some(m) => m,
            None => {
                let meta = Rc::new(MetaDeTipo {
                    lua: Rc::downgrade(&self.e),
                    metodos: RefCell::default(),
                    metas: RefCell::default(),
                    lidos: RefCell::default(),
                    gravados: RefCell::default(),
                });
                let mut c = Construtor::<T> { lua: self.clone(), meta: meta.clone(), _t: std::marker::PhantomData };
                T::add_fields(&mut c);
                T::add_methods(&mut c);
                self.e.tipos.borrow_mut().insert(id, meta.clone());
                meta
            }
        };
        Ok(AnyUserData(Rc::new(DadosDeUserdata { valor: RefCell::new(Box::new(v)), meta })))
    }

    pub fn create_registry_value(&self, v: impl IntoLua) -> Result<RegistryKey> {
        let v = de_rust(v.into_lua(self)?);
        let mut r = self.e.registro.borrow_mut();
        r.push(v);
        Ok(RegistryKey(r.len() - 1))
    }

    pub fn registry_value<T: FromLua>(&self, k: &RegistryKey) -> Result<T> {
        let v = self.e.registro.borrow().get(k.0).cloned().unwrap_or(Value::Nil);
        T::from_lua(para_rust(v), self)
    }

    pub fn load(&self, codigo: impl AsRef<[u8]>) -> Chunk {
        Chunk { lua: self.clone(), codigo: codigo.as_ref().to_vec(), nome: "=chunk".into(), env: None }
    }
}

pub struct Chunk {
    lua: Lua,
    codigo: Vec<u8>,
    nome: String,
    env: Option<Table>,
}

impl Chunk {
    pub fn set_name(mut self, nome: impl Into<String>) -> Self {
        self.nome = nome.into();
        self
    }

    pub fn set_environment(mut self, env: Table) -> Self {
        self.env = Some(env);
        self
    }

    pub fn into_function(self) -> Result<Function> {
        let nome = self.nome.trim_start_matches(['@', '=']).to_string();
        let proto = parser::compilar(&self.codigo, &nome).map_err(Error::RuntimeError)?;
        let env = self.env.unwrap_or_else(|| self.lua.globals());
        Ok(self.lua.funcao_lua(proto, Vec::new(), env))
    }

    pub fn exec(self) -> Result<()> {
        self.into_function()?.call(())
    }

    pub fn call<R: FromLuaMulti>(self, args: impl IntoLuaMulti) -> Result<R> {
        self.into_function()?.call(args)
    }
}

#[derive(Clone, Debug, Default)]
pub struct MultiValue(pub(crate) Vec<Value>);

impl MultiValue {
    pub fn from_vec(v: Vec<Value>) -> Self {
        MultiValue(v)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl IntoIterator for MultiValue {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

pub struct Variadic<T>(Vec<T>);

impl<T> IntoIterator for Variadic<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<T> FromIterator<T> for Variadic<T> {
    fn from_iter<I: IntoIterator<Item = T>>(i: I) -> Self {
        Variadic(i.into_iter().collect())
    }
}

pub trait IntoLua {
    fn into_lua(self, lua: &Lua) -> Result<Value>;
}

pub trait FromLua: Sized {
    fn from_lua(v: Value, lua: &Lua) -> Result<Self>;
}

pub trait IntoLuaMulti {
    fn into_lua_multi(self, lua: &Lua) -> Result<Vec<Value>>;
}

pub trait FromLuaMulti: Sized {
    fn from_lua_multi(args: Vec<Value>, lua: &Lua) -> Result<Self>;
}

/// Um elemento de uma lista de argumentos: tira o que precisa da frente.
pub trait ArgumentoLua: Sized {
    fn tomar(fila: &mut VecDeque<Value>, lua: &Lua) -> Result<Self>;
}

impl<T: FromLua> ArgumentoLua for T {
    fn tomar(fila: &mut VecDeque<Value>, lua: &Lua) -> Result<Self> {
        T::from_lua(fila.pop_front().unwrap_or(Value::Nil), lua)
    }
}

impl ArgumentoLua for MultiValue {
    fn tomar(fila: &mut VecDeque<Value>, _: &Lua) -> Result<Self> {
        Ok(MultiValue(fila.drain(..).collect()))
    }
}

impl<T: FromLua> ArgumentoLua for Variadic<T> {
    fn tomar(fila: &mut VecDeque<Value>, lua: &Lua) -> Result<Self> {
        fila.drain(..).map(|v| T::from_lua(v, lua)).collect::<Result<Vec<_>>>().map(Variadic)
    }
}

/// Um elemento de uma lista de valores de saída: `MultiValue` no fim de uma tupla expande.
pub trait RetornoLua {
    fn por_em(self, v: &mut Vec<Value>, lua: &Lua) -> Result<()>;
}

impl<T: IntoLua> RetornoLua for T {
    fn por_em(self, v: &mut Vec<Value>, lua: &Lua) -> Result<()> {
        v.push(self.into_lua(lua)?);
        Ok(())
    }
}

impl RetornoLua for MultiValue {
    fn por_em(self, v: &mut Vec<Value>, _: &Lua) -> Result<()> {
        v.extend(self.0);
        Ok(())
    }
}

impl<T: ArgumentoLua> FromLuaMulti for T {
    fn from_lua_multi(args: Vec<Value>, lua: &Lua) -> Result<Self> {
        T::tomar(&mut VecDeque::from(args), lua)
    }
}

impl FromLuaMulti for () {
    fn from_lua_multi(_: Vec<Value>, _: &Lua) -> Result<Self> {
        Ok(())
    }
}

macro_rules! tuplas {
    ($($t:ident),+) => {
        impl<$($t: ArgumentoLua),+> FromLuaMulti for ($($t,)+) {
            fn from_lua_multi(args: Vec<Value>, lua: &Lua) -> Result<Self> {
                let mut fila = VecDeque::from(args);
                Ok(($($t::tomar(&mut fila, lua)?,)+))
            }
        }
        impl<$($t: RetornoLua),+> IntoLuaMulti for ($($t,)+) {
            #[allow(non_snake_case)]
            fn into_lua_multi(self, lua: &Lua) -> Result<Vec<Value>> {
                let ($($t,)+) = self;
                let mut v = Vec::new();
                $($t.por_em(&mut v, lua)?;)+
                Ok(v)
            }
        }
    };
}
tuplas!(A);
tuplas!(A, B);
tuplas!(A, B, C);
tuplas!(A, B, C, D);
tuplas!(A, B, C, D, E);
tuplas!(A, B, C, D, E, F);

impl<T: IntoLua> IntoLuaMulti for T {
    fn into_lua_multi(self, lua: &Lua) -> Result<Vec<Value>> {
        Ok(vec![self.into_lua(lua)?])
    }
}

impl IntoLuaMulti for () {
    fn into_lua_multi(self, _: &Lua) -> Result<Vec<Value>> {
        Ok(Vec::new())
    }
}

impl IntoLuaMulti for MultiValue {
    fn into_lua_multi(self, _: &Lua) -> Result<Vec<Value>> {
        Ok(self.0)
    }
}

impl<T: IntoLua> IntoLuaMulti for Variadic<T> {
    fn into_lua_multi(self, lua: &Lua) -> Result<Vec<Value>> {
        self.0.into_iter().map(|v| v.into_lua(lua)).collect()
    }
}

fn erro_de_tipo(esperava: &str, v: &Value) -> Error {
    Error::RuntimeError(format!("esperava {esperava}, veio {}", v.nome_do_tipo()))
}

impl IntoLua for Value {
    fn into_lua(self, _: &Lua) -> Result<Value> {
        Ok(self)
    }
}

impl FromLua for Value {
    fn from_lua(v: Value, _: &Lua) -> Result<Self> {
        Ok(v)
    }
}

impl IntoLua for bool {
    fn into_lua(self, _: &Lua) -> Result<Value> {
        Ok(Value::Boolean(self))
    }
}

impl FromLua for bool {
    fn from_lua(v: Value, _: &Lua) -> Result<Self> {
        Ok(v.verdadeiro())
    }
}

impl IntoLua for &str {
    fn into_lua(self, _: &Lua) -> Result<Value> {
        Ok(Value::texto(self))
    }
}

impl IntoLua for String {
    fn into_lua(self, _: &Lua) -> Result<Value> {
        Ok(Value::texto(&self))
    }
}

impl IntoLua for LuaString {
    fn into_lua(self, _: &Lua) -> Result<Value> {
        Ok(Value::String(self))
    }
}

impl FromLua for LuaString {
    fn from_lua(v: Value, _: &Lua) -> Result<Self> {
        match v {
            Value::String(s) => Ok(s),
            v => Err(erro_de_tipo("texto", &v)),
        }
    }
}

impl FromLua for String {
    fn from_lua(v: Value, _: &Lua) -> Result<Self> {
        match &v {
            Value::String(s) => Ok(String::from_utf8_lossy(&s.0).into_owned()),
            Value::Integer(_) | Value::Number(_) => v.to_string(),
            _ => Err(erro_de_tipo("texto", &v)),
        }
    }
}

macro_rules! numeros {
    ($($t:ty),+) => {$(
        impl IntoLua for $t {
            fn into_lua(self, _: &Lua) -> Result<Value> {
                Ok(Value::Number(self as f64))
            }
        }
        impl FromLua for $t {
            fn from_lua(v: Value, _: &Lua) -> Result<Self> {
                match &v {
                    Value::Integer(i) => Ok(*i as $t),
                    Value::Number(n) => Ok(*n as $t),
                    Value::String(s) => padrao::texto_em_numero(&s.0).map(|n| n as $t).ok_or_else(|| erro_de_tipo("número", &v)),
                    _ => Err(erro_de_tipo("número", &v)),
                }
            }
        }
    )+};
}
numeros!(i32, i64, u32, u64, usize, f32, f64);

impl<T: IntoLua> IntoLua for Option<T> {
    fn into_lua(self, lua: &Lua) -> Result<Value> {
        self.map_or(Ok(Value::Nil), |v| v.into_lua(lua))
    }
}

impl<T: FromLua> FromLua for Option<T> {
    fn from_lua(v: Value, lua: &Lua) -> Result<Self> {
        if v.is_nil() {
            Ok(None)
        } else {
            T::from_lua(v, lua).map(Some)
        }
    }
}

impl IntoLua for Table {
    fn into_lua(self, _: &Lua) -> Result<Value> {
        Ok(Value::Table(self))
    }
}

impl IntoLua for &Table {
    fn into_lua(self, _: &Lua) -> Result<Value> {
        Ok(Value::Table(self.clone()))
    }
}

impl FromLua for Table {
    fn from_lua(v: Value, _: &Lua) -> Result<Self> {
        match v {
            Value::Table(t) => Ok(t),
            v => Err(erro_de_tipo("tabela", &v)),
        }
    }
}

impl IntoLua for Function {
    fn into_lua(self, _: &Lua) -> Result<Value> {
        Ok(Value::Function(self))
    }
}

impl FromLua for Function {
    fn from_lua(v: Value, _: &Lua) -> Result<Self> {
        match v {
            Value::Function(f) => Ok(f),
            v => Err(erro_de_tipo("função", &v)),
        }
    }
}

impl IntoLua for AnyUserData {
    fn into_lua(self, _: &Lua) -> Result<Value> {
        Ok(Value::UserData(self))
    }
}

impl FromLua for AnyUserData {
    fn from_lua(v: Value, _: &Lua) -> Result<Self> {
        match v {
            Value::UserData(u) => Ok(u),
            v => Err(erro_de_tipo("userdata", &v)),
        }
    }
}

impl<T: UserData> IntoLua for T {
    fn into_lua(self, lua: &Lua) -> Result<Value> {
        lua.create_userdata(self).map(Value::UserData)
    }
}
