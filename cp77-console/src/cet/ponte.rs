//! Ponte dos mods CET entre valor Lua e valor do RTTI: o tipo (do parâmetro, do campo ou do retorno) decide a conversão, nos dois sentidos.
//! Struct vira [`Struct`] (os bytes e o tipo; campo pelo nome sem diferenciar maiúscula, que é o `vec.x`/`euler.pitch`/`quat.i` do CET sobre
//! os `X`/`Pitch`/`i` do RTTI), handle vira [`Handle`], enum vira número. Tipo de RTTI por nome é um [`Tipo`]: `Vector4.new(…)`,
//! `GetSingleton('Quaternion'):ToEulerAngles(q)` e os estáticos.

use super::{handle_alive, CName, Handle, ItemId, TweakDBID, Vec4};
use crate::rtti::ertti;
use std::ffi::c_void;

/// `ERTTIType` que o `ertti` não nomeia fora do dev: `Class` (struct quando vem por valor) e `BitField` (o CET 1.37.1 entrega como inteiro).
const CLASSE: u8 = 2;
const BITFIELD: u8 = 13;

/// Valor de struct do RTTI: os bytes no layout do motor e o tipo dele.
pub(crate) struct Struct {
    pub(crate) tipo: *mut c_void,
    pub(crate) bytes: Vec<u8>,
}

/// Tipo de RTTI por nome, como o CET põe cada tipo do jogo no global: `.new` constrói, o resto é função estática da classe.
pub(crate) struct Tipo(pub(crate) *mut c_void);

unsafe fn nome_do_tipo(ty: *mut c_void) -> String {
    crate::cname::resolve_cname(crate::rtti::type_name_getname(ty))
}

/// Props de `ty` em ordem de offset (a ordem posicional do `.new` do CET: `Vector4.new(x, y, z, w)`, `EulerAngles.new(roll, pitch, yaw)`).
unsafe fn campos_em_ordem(ty: *mut c_void) -> Vec<(u64, *mut c_void, u32)> {
    let mut v = crate::rtti::propriedades(ty);
    v.sort_by_key(|&(_, _, off)| off);
    v
}

/// Prop `campo` de `ty` sem diferenciar maiúscula: `(tipo, offset)`.
unsafe fn campo(ty: *mut c_void, campo: &str) -> Option<(*mut c_void, u32)> {
    let exato = crate::cname::cname(campo);
    let props = crate::rtti::propriedades(ty);
    props
        .iter()
        .find(|&&(n, _, _)| n == exato)
        .or_else(|| props.iter().find(|&&(n, _, _)| crate::cname::resolve_cname(n).eq_ignore_ascii_case(campo)))
        .map(|&(_, t, off)| (t, off))
}

/// Inteiro de `n` bytes em `p`, com sinal quando `com_sinal`.
unsafe fn inteiro(p: *const u8, n: usize, com_sinal: bool) -> i64 {
    match (n, com_sinal) {
        (1, true) => (p as *const i8).read() as i64,
        (1, false) => p.read() as i64,
        (2, true) => (p as *const i16).read_unaligned() as i64,
        (2, false) => (p as *const u16).read_unaligned() as i64,
        (4, true) => (p as *const i32).read_unaligned() as i64,
        (4, false) => (p as *const u32).read_unaligned() as i64,
        _ => (p as *const i64).read_unaligned(),
    }
}

/// Largura e sinal dos fundamentais inteiros.
fn inteiro_do_tipo(nome: &str) -> Option<(usize, bool)> {
    Some(match nome {
        "Int8" => (1, true),
        "Uint8" => (1, false),
        "Int16" => (2, true),
        "Uint16" => (2, false),
        "Int32" => (4, true),
        "Uint32" => (4, false),
        "Int64" => (8, true),
        "Uint64" => (8, false),
        _ => return None,
    })
}

/// `DynArray` em `p` (`{entries, capacity, size}`) como tabela, um valor por elemento.
unsafe fn ler_lista(lua: &bwms_lua::Lua, p: *const u8, ty: *mut c_void) -> bwms_lua::Result<bwms_lua::Value> {
    let t = lua.create_table()?;
    let (itens, tamanho, _) = crate::jogo::listas::dynarray_no_campo(p, 0);
    let de_item = crate::rtti::inner_type(ty);
    let passo = if crate::rtti::sane(de_item) { crate::rtti::type_size(de_item) as usize } else { 0 };
    if !itens.is_null() && passo > 0 && crate::memoria::is_readable(itens as *const c_void, tamanho * passo) {
        for i in 0..tamanho {
            t.set(i + 1, ler_valor(lua, itens.add(i * passo), de_item)?)?;
        }
    }
    Ok(bwms_lua::Value::Table(t))
}

/// Valor de tipo `ty` em `p` como valor Lua. `nil` para o que a ponte não converte (referência de recurso).
pub(crate) unsafe fn ler_valor(lua: &bwms_lua::Lua, p: *const u8, ty: *mut c_void) -> bwms_lua::Result<bwms_lua::Value> {
    use bwms_lua::Value as V;
    if p.is_null() || !crate::rtti::sane(ty) {
        return Ok(V::Nil);
    }
    let kind = crate::rtti::type_kind(ty);
    if matches!(kind, Some(ertti::HANDLE) | Some(ertti::WEAK_HANDLE)) {
        let inst = handle_alive(p);
        return Ok(if inst.is_null() { V::Nil } else { V::UserData(lua.create_userdata(Handle(inst, crate::rtti::inner_type(ty)))?) });
    }
    if kind == Some(ertti::ARRAY) {
        return ler_lista(lua, p, ty);
    }
    let n = crate::rtti::type_size(ty) as usize;
    if kind == Some(ertti::ENUM) {
        return Ok(V::Integer(inteiro(p, n.clamp(1, 8), true)));
    }
    if kind == Some(BITFIELD) {
        return Ok(V::Integer(inteiro(p, n.clamp(1, 8), false)));
    }
    let nome = nome_do_tipo(ty);
    Ok(match nome.as_str() {
        "Float" => V::Number(f64::from((p as *const f32).read_unaligned())),
        "Double" => V::Number((p as *const f64).read_unaligned()),
        "Bool" => V::Boolean(p.read() != 0),
        "CName" => V::UserData(lua.create_userdata(CName((p as *const u64).read_unaligned()))?),
        "TweakDBID" => V::UserData(lua.create_userdata(TweakDBID((p as *const [u8; 8]).read_unaligned()))?),
        "gameItemID" | "ItemID" => V::UserData(lua.create_userdata(ItemId((p as *const [u8; 16]).read_unaligned()))?),
        "String" => V::String(lua.create_string(crate::jogo::cstring::texto(p))?),
        _ => match inteiro_do_tipo(&nome) {
            Some((w, s)) => V::Integer(inteiro(p, w, s)),
            None if kind == Some(CLASSE) && n > 0 => {
                V::UserData(lua.create_userdata(Struct { tipo: ty, bytes: std::slice::from_raw_parts(p, n).to_vec() })?)
            }
            None => V::Nil,
        },
    })
}

/// Número de um valor Lua (inteiro, real ou booleano).
fn numero(v: &bwms_lua::Value) -> Option<f64> {
    match v {
        bwms_lua::Value::Integer(i) => Some(*i as f64),
        bwms_lua::Value::Number(n) => Some(*n),
        bwms_lua::Value::Boolean(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

/// Grava `v` em `p` no layout do tipo `ty` (os `type_size(ty)` bytes). `false` quando o valor não serve para o tipo; `String` fica de fora
/// (o slot de parâmetro `String` vai pelo construtor do motor, [`crate::rtti::Arg::Str`]).
pub(crate) unsafe fn escrever_valor(p: *mut u8, ty: *mut c_void, v: &bwms_lua::Value) -> bool {
    use bwms_lua::Value as V;
    let kind = crate::rtti::type_kind(ty);
    let n = crate::rtti::type_size(ty) as usize;
    if matches!(kind, Some(ertti::HANDLE) | Some(ertti::WEAK_HANDLE)) {
        let inst = match v {
            V::Nil => std::ptr::null_mut(),
            V::UserData(ud) => match ud.borrow::<Handle>() {
                Ok(h) => h.0,
                Err(_) => return false,
            },
            _ => return false,
        };
        (p as *mut *mut c_void).write_unaligned(inst);
        let refcnt = if inst.is_null() { std::ptr::null_mut() } else { crate::rtti::refcnt() };
        (p.add(8) as *mut *mut c_void).write_unaligned(refcnt);
        return true;
    }
    if kind == Some(BITFIELD) {
        return numero(v).map(|x| std::ptr::copy_nonoverlapping((x as i64).to_le_bytes().as_ptr(), p, n.clamp(1, 8))).is_some();
    }
    if kind == Some(ertti::ENUM) {
        let valor = match v {
            V::String(s) => match s.to_str().ok().and_then(|s| crate::rtti::enum_valor(ty as *const u8, crate::cname::cname(&s))) {
                Some(x) => x as i64,
                None => return false,
            },
            _ => match numero(v) {
                Some(x) => x as i64,
                None => return false,
            },
        };
        std::ptr::copy_nonoverlapping(valor.to_le_bytes().as_ptr(), p, n.clamp(1, 8));
        return true;
    }
    let nome = nome_do_tipo(ty);
    match nome.as_str() {
        "Float" => numero(v).map(|x| (p as *mut f32).write_unaligned(x as f32)).is_some(),
        "Double" => numero(v).map(|x| (p as *mut f64).write_unaligned(x)).is_some(),
        "Bool" => match v {
            V::Boolean(b) => {
                p.write(u8::from(*b));
                true
            }
            _ => numero(v).map(|x| p.write(u8::from(x != 0.0))).is_some(),
        },
        "CName" => {
            let h = match v {
                V::String(s) => s.to_str().map(|s| crate::cname::cname(&s)).ok(),
                V::UserData(ud) => ud.borrow::<CName>().map(|c| c.0).ok(),
                V::Nil => Some(0),
                _ => None,
            };
            h.map(|h| (p as *mut u64).write_unaligned(h)).is_some()
        }
        "TweakDBID" => {
            let id = match v {
                V::String(s) => s.to_str().map(|s| bwms_primitives::tweak_db_id(&s).to_le_bytes()).ok(),
                V::UserData(ud) => ud.borrow::<TweakDBID>().map(|t| t.0).ok(),
                _ => None,
            };
            id.map(|b| std::ptr::copy_nonoverlapping(b.as_ptr(), p, 8)).is_some()
        }
        "gameItemID" | "ItemID" => match v {
            V::UserData(ud) => ud.borrow::<ItemId>().map(|i| std::ptr::copy_nonoverlapping(i.0.as_ptr(), p, 16)).is_ok(),
            _ => false,
        },
        _ => match inteiro_do_tipo(&nome) {
            Some((w, _)) => numero(v).map(|x| std::ptr::copy_nonoverlapping((x as i64).to_le_bytes().as_ptr(), p, w)).is_some(),
            None if kind == Some(CLASSE) => escrever_struct(p, ty, n, v),
            None => false,
        },
    }
}

/// Struct por valor: outro [`Struct`] do mesmo tamanho, o `Vec4` antigo (16 B), ou uma tabela com os campos pelo nome.
unsafe fn escrever_struct(p: *mut u8, ty: *mut c_void, n: usize, v: &bwms_lua::Value) -> bool {
    match v {
        bwms_lua::Value::UserData(ud) => {
            if let Ok(s) = ud.borrow::<Struct>() {
                if s.bytes.len() == n {
                    std::ptr::copy_nonoverlapping(s.bytes.as_ptr(), p, n);
                    return true;
                }
                return false;
            }
            if let Ok(v4) = ud.borrow::<Vec4>() {
                if n == 16 {
                    std::ptr::copy_nonoverlapping(v4.0.as_ptr() as *const u8, p, 16);
                    return true;
                }
            }
            false
        }
        bwms_lua::Value::Table(t) => {
            std::ptr::write_bytes(p, 0, n);
            for par in t.clone().pairs::<String, bwms_lua::Value>() {
                let Ok((k, val)) = par else { return false };
                match campo(ty, &k) {
                    Some((ct, off)) if escrever_valor(p.add(off as usize), ct, &val) => {}
                    _ => return false,
                }
            }
            true
        }
        _ => false,
    }
}

/// Struct nova de `ty` (zerada) com os campos em ordem de offset vindos de `args`, ou de uma tabela `{campo = valor}` como 1º argumento.
pub(crate) unsafe fn nova_struct(ty: *mut c_void, args: &[bwms_lua::Value]) -> Option<Struct> {
    let n = crate::rtti::type_size(ty) as usize;
    if n == 0 {
        return None;
    }
    let mut bytes = vec![0u8; n];
    if let [t @ bwms_lua::Value::Table(_)] = args {
        return escrever_struct(bytes.as_mut_ptr(), ty, n, t).then_some(Struct { tipo: ty, bytes });
    }
    for ((_, ct, off), v) in campos_em_ordem(ty).into_iter().zip(args) {
        if !escrever_valor(bytes.as_mut_ptr().add(off as usize), ct, v) {
            return None;
        }
    }
    Some(Struct { tipo: ty, bytes })
}

/// Argumento de `rf` para o parâmetro `i` a partir do valor Lua, pelo tipo do parâmetro: `String` pelo construtor do motor, o resto nos bytes
/// do tipo. `None` quando o valor não serve para o tipo.
pub(crate) unsafe fn argumento(func: *mut c_void, i: usize, v: &bwms_lua::Value) -> Option<crate::rtti::Arg> {
    let ty = crate::rtti::fn_param_itype(func, i);
    if ty.is_null() {
        return None;
    }
    if nome_do_tipo(ty) == "String" {
        return match v {
            bwms_lua::Value::String(s) => Some(crate::rtti::Arg::Str(s.to_str().ok()?.to_string())),
            _ => None,
        };
    }
    let mut b = vec![0u8; (crate::rtti::type_size(ty) as usize).max(1)];
    escrever_valor(b.as_mut_ptr(), ty, v).then_some(crate::rtti::Arg::Bytes(b))
}

/// Chama `rf` com os valores Lua em `args` (convertidos pelos tipos dos parâmetros) e devolve o retorno pelo tipo de retorno. Parâmetro
/// `ScriptGameInstance` que o mod não passou (os getters `Game.GetXxxSystem()` do CET) entra sozinho, da sessão.
pub(crate) unsafe fn chamar(lua: &bwms_lua::Lua, rf: &crate::rtti::ResolvedFn, ctx: *mut c_void, args: &[bwms_lua::Value]) -> bwms_lua::Result<bwms_lua::Value> {
    let total = crate::rtti::param_count(rf) as usize;
    let mut margs = Vec::with_capacity(total);
    let mut i = 0;
    let mut resto = args.iter();
    while i < total {
        let ty = crate::rtti::fn_param_itype(rf.func, i);
        let falta = total - i > resto.len();
        if falta && nome_do_tipo(ty) == "ScriptGameInstance" {
            let gi = instancia_do_jogo().ok_or_else(|| bwms_lua::Error::RuntimeError("GameInstance fora da sessão".into()))?;
            margs.push(crate::rtti::Arg::Bytes(gi.to_vec()));
            i += 1;
            continue;
        }
        let Some(v) = resto.next() else { break };
        let a = argumento(rf.func, i, v).ok_or_else(|| {
            bwms_lua::Error::RuntimeError(format!("argumento {} não serve para {}", i + 1, nome_do_tipo(ty)))
        })?;
        margs.push(a);
        i += 1;
    }
    // Native sem contexto: o despacho do motor (`0x101099c44`) cai no `vt+0x28` da função, nulo no `GetSystemRequestsHandler`
    // e no `inkMenuScenario.GetSystemRequestsHandler` (lanes L-9 e L-10, null deref em `0x101099d98`). Native estática aceita qualquer objeto RTTI
    // como contexto (DATABASE, ABI de native `static`); com o V no mundo ele é o contexto de reserva.
    let ctx = if ctx.is_null() { crate::current_player() } else { ctx };
    let Some(ret) = crate::rtti::call_func_valor(rf, ctx, &margs) else { return Ok(bwms_lua::Value::Nil) };
    if rf.ret_type.is_null() {
        return Ok(bwms_lua::Value::Nil);
    }
    ler_valor(lua, ret.as_ptr(), rf.ret_type)
}

/// `ScriptGameInstance` (16 B) como o CET monta: `{ framework->gameInstance, 0 }` do motor, no menu e em gameplay.
unsafe fn instancia_do_jogo() -> Option<[u8; 16]> {
    let gi = crate::jogo::motor::game_instance()?;
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&(gi as u64).to_le_bytes());
    Some(b)
}

/// Função `nome` de `ty` (método ou estática, subindo a herança) que os argumentos de `args` alcançam. Nome com assinatura (`Nome;Args`) vale
/// inteiro.
unsafe fn funcao(ty: *mut c_void, nome: &str) -> Option<crate::rtti::ResolvedFn> {
    crate::rtti::resolve_in_class(ty, nome)
}

impl bwms_lua::UserData for Struct {
    fn add_methods<M: bwms_lua::UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(bwms_lua::MetaMethod::Index, |lua, s, chave: String| unsafe {
            if let Some((ct, off)) = campo(s.tipo, &chave) {
                return ler_valor(lua, s.bytes.as_ptr().add(off as usize), ct);
            }
            match funcao(s.tipo, &chave) {
                // Método da struct: `v:Metodo(...)`. Estática recebe a própria struct como 1º argumento; de instância, os bytes são o `this`.
                Some(rf) => {
                    let tipo = s.tipo;
                    let f = lua.create_function(move |lua, (eu, resto): (bwms_lua::AnyUserData, bwms_lua::MultiValue)| {
                        let mut s = eu.borrow_mut::<Struct>()?;
                        let mut args: Vec<bwms_lua::Value> = resto.into_iter().collect();
                        let rf = crate::rtti::ResolvedFn { func: rf.func, ret_type: rf.ret_type, is_static: rf.is_static };
                        if rf.is_static {
                            args.insert(0, bwms_lua::Value::UserData(lua.create_userdata(Struct { tipo, bytes: s.bytes.clone() })?));
                            chamar(lua, &rf, std::ptr::null_mut(), &args)
                        } else {
                            chamar(lua, &rf, s.bytes.as_mut_ptr() as *mut c_void, &args)
                        }
                    })?;
                    Ok(bwms_lua::Value::Function(f))
                }
                None => Ok(bwms_lua::Value::Nil),
            }
        });
        m.add_meta_method_mut(bwms_lua::MetaMethod::NewIndex, |_, s, (chave, v): (String, bwms_lua::Value)| unsafe {
            match campo(s.tipo, &chave) {
                Some((ct, off)) if escrever_valor(s.bytes.as_mut_ptr().add(off as usize), ct, &v) => Ok(()),
                Some(_) => Err(bwms_lua::Error::RuntimeError(format!("valor não serve para o campo {chave}"))),
                None => Err(bwms_lua::Error::RuntimeError(format!("{} não tem o campo {chave}", nome_do_tipo(s.tipo)))),
            }
        });
        m.add_meta_method(bwms_lua::MetaMethod::ToString, |lua, s, ()| unsafe {
            let mut partes = Vec::new();
            for (n, ct, off) in campos_em_ordem(s.tipo) {
                let v = ler_valor(lua, s.bytes.as_ptr().add(off as usize), ct)?;
                partes.push(format!("{}={}", crate::cname::resolve_cname(n), v.to_string().unwrap_or_default()));
            }
            Ok(format!("{}{{{}}}", nome_do_tipo(s.tipo), partes.join(", ")))
        });
    }
}

impl bwms_lua::UserData for Tipo {
    fn add_methods<M: bwms_lua::UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(bwms_lua::MetaMethod::Index, |lua, t, chave: String| unsafe {
            let ty = t.0;
            if chave == "new" {
                let f = lua.create_function(move |lua, args: bwms_lua::MultiValue| {
                    let args: Vec<bwms_lua::Value> = args.into_iter().collect();
                    if crate::rtti::type_kind(ty) == Some(CLASSE) && !super::e_classe_de_objeto(ty) {
                        return match nova_struct(ty, &args) {
                            Some(s) => Ok(bwms_lua::Value::UserData(lua.create_userdata(s)?)),
                            None => Err(bwms_lua::Error::RuntimeError(format!("{}.new: argumentos não servem", nome_do_tipo(ty)))),
                        };
                    }
                    Ok(super::novo_objeto(lua, ty))
                })?;
                return Ok(bwms_lua::Value::Function(f));
            }
            match funcao(ty, &chave) {
                // `Tipo.Estatica(...)` e `GetSingleton('Tipo'):Estatica(...)`: o `:` passa o próprio `Tipo` como 1º argumento, que sai.
                Some(rf) => {
                    let f = lua.create_function(move |lua, args: bwms_lua::MultiValue| {
                        let mut args: Vec<bwms_lua::Value> = args.into_iter().collect();
                        if matches!(args.first(), Some(bwms_lua::Value::UserData(ud)) if ud.is::<Tipo>()) {
                            args.remove(0);
                        }
                        let rf = crate::rtti::ResolvedFn { func: rf.func, ret_type: rf.ret_type, is_static: rf.is_static };
                        // Contexto do `GetSingleton` do CET: o sistema desse tipo no `GameInstance` (`SingletonReference::GetHandle`); tipo que não é
                        // sistema, nenhum.
                        let sistema = crate::jogo::motor::game_instance().map_or(std::ptr::null_mut(), |gi| crate::jogo::sessao::sistema_do_motor(gi, ty));
                        chamar(lua, &rf, sistema, &args)
                    })?;
                    Ok(bwms_lua::Value::Function(f))
                }
                None => Ok(bwms_lua::Value::Nil),
            }
        });
        m.add_meta_method(bwms_lua::MetaMethod::ToString, |_, t, ()| Ok(unsafe { nome_do_tipo(t.0) }));
    }
}

/// `Tipo` de RTTI pelo nome (classe, struct ou enum), se existe.
pub(crate) unsafe fn tipo_por_nome(nome: &str) -> Option<Tipo> {
    let reg = crate::registry()?;
    let c = reg.class_by_name(nome);
    if crate::rtti::sane(c) {
        return Some(Tipo(c));
    }
    // Nome de script do mod (`ActivateTPPRepresentationEvent`) para a classe nativa (`gameActivateTPPRepresentationEvent`), como o CET.
    let c = reg.class_by_script_name(nome);
    if crate::rtti::sane(c) {
        return Some(Tipo(c));
    }
    let t = reg.type_by_name(nome);
    crate::rtti::sane(t).then_some(Tipo(t))
}

/// Função global do jogo (`GetPlayer`, `GetAllBlackboardDefs`) ou estática do `ScriptGameInstance` (`GetSettingsSystem`), o `Game.*` do CET.
pub(crate) unsafe fn funcao_do_jogo(nome: &str) -> Option<crate::rtti::ResolvedFn> {
    let reg = crate::registry()?;
    // Apelido do CET (`RTTIMapper`): `Game.GetSystemRequestsHandler()` é o do `inkMenuScenario`.
    if nome == "GetSystemRequestsHandler" {
        return crate::rtti::resolve_func(reg, "inkMenuScenario", nome);
    }
    let f = crate::rtti::resolve_global_function_robust(reg, nome);
    if !f.is_null() {
        return Some(crate::rtti::ResolvedFn { func: f, ret_type: crate::rtti::ret_type_of(f), is_static: true });
    }
    crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], nome)
}

/// `EnumValueToName(enum, valor)` do CET: `{ value = "Nome" }`, ou `nil` sem o membro.
pub(crate) unsafe fn nome_do_valor_de_enum(lua: &bwms_lua::Lua, en: &str, valor: i64) -> bwms_lua::Result<bwms_lua::Value> {
    let Some(reg) = crate::registry() else { return Ok(bwms_lua::Value::Nil) };
    match crate::rtti::enum_nome(reg.enum_by_name(en) as *const u8, valor as u64) {
        Some(h) => {
            let t = lua.create_table()?;
            t.set("value", crate::cname::resolve_cname(h))?;
            Ok(bwms_lua::Value::Table(t))
        }
        None => Ok(bwms_lua::Value::Nil),
    }
}
