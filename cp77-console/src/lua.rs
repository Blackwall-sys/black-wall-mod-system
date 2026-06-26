//! lua.rs — ROTA A: runtime Lua PERSISTENTE (LuaJIT) = plataforma de mods estilo
//! CET. Um estado global único (serializado por Mutex), `Game.*` ligado ao nosso
//! motor RTTI, `registerForEvent(onInit/onUpdate/...)` e o lifecycle dirigido pelo
//! `cp77_tick` (thread do jogo). Um mod persistente com `onUpdate` rodando =
//! a diferença real entre CET-plataforma e console-de-comando.

use std::ffi::c_void;
use std::sync::Mutex;

use crate::console;
use crate::rtti::Registry;

struct SendLua(mlua::Lua);
unsafe impl Send for SendLua {}
static LUA: Mutex<Option<SendLua>> = Mutex::new(None);
/// Callbacks de hotkey (registerHotkey), keyed pela string da tecla.
static HOTKEYS: Mutex<Option<std::collections::HashMap<String, mlua::RegistryKey>>> =
    Mutex::new(None);

/// Última msg "[proxy] metodo nao achado" — pra throttle (colapsa runs idênticos no console).
static LAST_PROXY_MISS: Mutex<Option<String>> = Mutex::new(None);

/// Callbacks de registerInput (down/up), keyed pelo char da tecla. O cb recebe um
/// bool `isDown` — dispara tanto no key-down quanto no key-up (como o CET).
static INPUTS: Mutex<Option<std::collections::HashMap<char, mlua::RegistryKey>>> =
    Mutex::new(None);

/// Dispara o callback de hotkey da tecla `c` (chamado pelo cp77_tick, thread do jogo).
pub unsafe fn fire_hotkey(c: char) {
    let g = match LUA.try_lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if let Some(s) = g.as_ref() {
        let lua = &s.0;
        let hk = HOTKEYS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = hk.as_ref() {
            if let Some(rk) = map.get(&c.to_string()) {
                if let Ok(f) = lua.registry_value::<mlua::Function>(rk) {
                    let _: mlua::Result<()> = f.call(());
                }
            }
        }
    }
}

/// Dispara o callback de registerInput da tecla `c` com `down` (key-down/up),
/// chamado pelo cp77_tick na thread do jogo. O cb recebe `isDown: bool`.
pub unsafe fn fire_input(c: char, down: bool) {
    let g = match LUA.try_lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if let Some(s) = g.as_ref() {
        let lua = &s.0;
        let inp = INPUTS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = inp.as_ref() {
            if let Some(rk) = map.get(&c) {
                if let Ok(f) = lua.registry_value::<mlua::Function>(rk) {
                    let _: mlua::Result<()> = f.call(down);
                }
            }
        }
    }
}

/// Roda `f` com (reg, player, tx) atuais — globais que o cp77_tick seta a cada
/// tick (na thread do jogo). É como o Lua alcança o motor sem capturar ponteiros.
unsafe fn with_engine<F: FnOnce(&Registry, *mut c_void, *mut c_void)>(f: F) {
    if let Some(reg) = crate::registry() {
        let p = crate::current_player();
        let t = crate::current_tx();
        if !p.is_null() && !t.is_null() {
            f(reg, p, t);
        }
    }
}

/// Proxy de um objeto/handle RED exposto ao Lua. `h:Method(args)` resolve o método
/// na classe do objeto (vtable→GetType→CClass) e chama pelo executor; o retorno
/// vira Handle (se ponteiro são, encadeável) ou inteiro. É o que torna o `this` do
/// Observe e o `Game.GetPlayer()` ÚTEIS — sem isso o handle é opaco.
/// `.0` = ponteiro do objeto. `.1` = CLASSE conhecida (de quando NÓS construímos via
/// new_object) ou null. Pra objeto construído, `class_of` (GetType) devolve a base
/// (IScriptable) p/ classes scripted → usamos a classe guardada p/ resolver campos/métodos.
#[derive(Clone, Copy)]
pub struct Handle(pub *mut c_void, pub *mut c_void);

impl mlua::UserData for Handle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // __index: PRIMEIRO tenta PROPRIEDADE (campo por nome via RTTI) — `obj.label`,
        // `spawnEvent.value`, etc.; se não for campo, devolve proxy de MÉTODO `obj:Metodo()`.
        // Espelha o CET (campo > método).
        methods.add_meta_method(mlua::MetaMethod::Index, |lua, this, key: String| {
            let ptr = this.0;
            let known = this.1;
            unsafe {
                // classe conhecida (objeto construído) tem prioridade sobre class_of (que
                // devolve a base p/ scripted).
                let cls = if !known.is_null() { known } else { crate::rtti::class_of(ptr) };
                if !cls.is_null() {
                    if let Some((off, ty, in_holder)) =
                        crate::rtti::resolve_prop_in_class(cls, &key)
                    {
                        let v = read_field(lua, ptr, off, ty, in_holder);
                        let tn = crate::cname::resolve_cname(crate::rtti::type_name_getname(ty));
                        crate::trace(&format!(
                            "field .{key} @{off:#x} (type '{tn}' holder={in_holder}) -> {}",
                            lua_dbg(&v)
                        ));
                        return Ok(v);
                    }
                }
            }
            let f = lua.create_function(move |lua, args: mlua::Variadic<mlua::Value>| {
                // `h:Method(a,b)` => args = [h, a, b]; pula o self.
                let real: Vec<mlua::Value> = args.into_iter().skip(1).collect();
                unsafe { call_method(lua, ptr, known, &key, &real) }
            })?;
            Ok(mlua::Value::Function(f))
        });
        // __newindex: escreve um CAMPO por nome — `data.label = "Mods"`, `.action = ...`.
        methods.add_meta_method(
            mlua::MetaMethod::NewIndex,
            |_, this, (key, val): (String, mlua::Value)| {
                let ptr = this.0;
                let known = this.1;
                unsafe {
                    let cls = if !known.is_null() { known } else { crate::rtti::class_of(ptr) };
                    if !cls.is_null() {
                        if let Some((off, ty, in_holder)) =
                            crate::rtti::resolve_prop_in_class(cls, &key)
                        {
                            let fp = crate::rtti::field_ptr(ptr, off, in_holder);
                            crate::trace(&format!(
                                "write .{key} @{off:#x} holder={in_holder} fp={fp:p}"
                            ));
                            write_field(ptr, off, ty, in_holder, &val);
                        } else {
                            let cn = crate::cname::resolve_cname(crate::rtti::type_name_hash(cls));
                            crate::log(&format!(
                                "[lua] campo '.{key}' desconhecido (classe '{cn}' cls {cls:p})"
                            ));
                        }
                    } else {
                        crate::log(&format!("[lua] campo '.{key}' sem classe (cls null)"));
                    }
                }
                Ok(())
            },
        );
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!("Handle(0x{:x})", this.0 as usize))
        });
    }
}

/// Debug curto de um Value pro trace.
fn lua_dbg(v: &mlua::Value) -> String {
    match v {
        mlua::Value::Nil => "nil".into(),
        mlua::Value::Boolean(b) => format!("{b}"),
        mlua::Value::Integer(i) => format!("{i}"),
        mlua::Value::Number(n) => format!("{n}"),
        mlua::Value::String(s) => {
            format!("'{}'", s.to_str().map(|x| x.to_string()).unwrap_or_default())
        }
        mlua::Value::UserData(ud) => {
            if let Ok(h) = ud.borrow::<Handle>() {
                unsafe {
                    let p = h.0;
                    let readable = crate::gum::is_readable(p as *const c_void, 8);
                    let vt = if readable { (p as *const usize).read_unaligned() } else { 0 };
                    let gt = if vt != 0 && crate::gum::is_readable(vt as *const c_void, 0x10) {
                        ((vt + 8) as *const usize).read_unaligned()
                    } else {
                        0
                    };
                    format!(
                        "h:{p:p} vt_s:{:#x} gt_s:{:#x} cls:{:p}",
                        crate::un_rebase(vt as *mut c_void),
                        crate::un_rebase(gt as *mut c_void),
                        crate::rtti::class_of(p)
                    )
                }
            } else {
                "<ud>".into()
            }
        }
        _ => "<other>".into(),
    }
}

/// Lê o campo (obj+off) decodificando pelo NOME do tipo (IType+0x18). gum-checked → Nil se torto.
/// Slot de (Weak)Handle {instance@+0, refCount(RefCnt*)@+8}; RefCnt.strongRefs u32@+0.
/// Devolve a instância VIVA (strongRefs>0, instance e refCount não-null) ou null. É o que
/// faltava no PushData/GetToggledIndex: líamos o ponteiro cru sem checar se o handle vive.
unsafe fn handle_alive(slot: *const u8) -> *mut c_void {
    if !crate::gum::is_readable(slot as *const c_void, 0x10) {
        return std::ptr::null_mut();
    }
    let instance = (slot as *const *mut c_void).read_unaligned();
    let refcnt = (slot.add(0x08) as *const *mut c_void).read_unaligned();
    if instance.is_null() || refcnt.is_null() {
        return std::ptr::null_mut();
    }
    if !crate::gum::is_readable(refcnt as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    let strong = (refcnt as *const u32).read_unaligned(); // RefCnt.strongRefs @ +0
    if strong == 0 || !crate::rtti::sane(instance) {
        return std::ptr::null_mut();
    }
    instance
}

unsafe fn read_field(
    lua: &mlua::Lua,
    obj: *mut c_void,
    off: u32,
    ty: *mut c_void,
    in_holder: bool,
) -> mlua::Value {
    // ponteiro final respeitando inValueHolder (props de scripted moram no valueHolder@0x38).
    let f = crate::rtti::field_ptr(obj, off, in_holder) as *mut u8;
    if f.is_null() || !crate::gum::is_readable(f as *const c_void, 8) {
        return mlua::Value::Nil;
    }
    let tn = crate::rtti::type_name_getname(ty);
    // (Weak)Handle: o slot é {instance, refCount}; resolve a instância VIVA e carrega a
    // classe INTERNA (inner_type) p/ achar os métodos (PushData/etc.). Espelha o unwrap do CET.
    let tname = crate::cname::resolve_cname(tn);
    if tname.starts_with("handle:") || tname.starts_with("whandle:") {
        let live = handle_alive(f);
        if live.is_null() {
            return mlua::Value::Nil; // dangling/morto → nil seguro (sem crash)
        }
        let inner = crate::rtti::inner_type(ty);
        return lua
            .create_userdata(Handle(live, inner))
            .map(mlua::Value::UserData)
            .unwrap_or(mlua::Value::Nil);
    }
    let cn = crate::cname::cname;
    if tn == cn("Bool") {
        return mlua::Value::Boolean(*f != 0);
    }
    if tn == cn("Int8") {
        return mlua::Value::Integer(*(f as *const i8) as i64);
    }
    if tn == cn("Uint8") {
        return mlua::Value::Integer(*f as i64);
    }
    if tn == cn("Int16") {
        return mlua::Value::Integer((f as *const i16).read_unaligned() as i64);
    }
    if tn == cn("Uint16") {
        return mlua::Value::Integer((f as *const u16).read_unaligned() as i64);
    }
    if tn == cn("Int32") {
        return mlua::Value::Integer((f as *const i32).read_unaligned() as i64);
    }
    if tn == cn("Uint32") {
        return mlua::Value::Integer((f as *const u32).read_unaligned() as i64);
    }
    if tn == cn("Int64") {
        return mlua::Value::Integer((f as *const i64).read_unaligned());
    }
    if tn == cn("Uint64") {
        return mlua::Value::Integer((f as *const u64).read_unaligned() as i64);
    }
    if tn == cn("Float") {
        return mlua::Value::Number((f as *const f32).read_unaligned() as f64);
    }
    if tn == cn("Double") {
        return mlua::Value::Number((f as *const f64).read_unaligned());
    }
    if tn == cn("CName") {
        let h = (f as *const u64).read_unaligned();
        return lua
            .create_userdata(CName(h))
            .map(mlua::Value::UserData)
            .unwrap_or(mlua::Value::Nil);
    }
    if tn == cn("String") {
        let s = crate::rtti::red_string_read(f);
        return lua
            .create_string(s.as_bytes())
            .map(mlua::Value::String)
            .unwrap_or(mlua::Value::Nil);
    }
    // enum/objeto/ref: discrimina pelo TAMANHO do tipo.
    match crate::rtti::type_size(ty) {
        1 => mlua::Value::Integer(*f as i64),
        2 => mlua::Value::Integer((f as *const u16).read_unaligned() as i64),
        4 => mlua::Value::Integer((f as *const u32).read_unaligned() as i64),
        _ => {
            // 8+: provável ponteiro de objeto (Handle/ref) → embrulha p/ chamar métodos.
            let p = (f as *const *mut c_void).read_unaligned();
            if crate::rtti::sane(p) {
                handle_val(lua, p)
            } else {
                mlua::Value::Integer((f as *const i64).read_unaligned())
            }
        }
    }
}

/// Escreve o campo (obj+off) marshalando o Value pelo tipo. Só POD/CName/enum/String-inline
/// e ponteiros — campos de obj recém-Construct (zerados). gum-checked.
unsafe fn write_field(
    obj: *mut c_void,
    off: u32,
    ty: *mut c_void,
    in_holder: bool,
    val: &mlua::Value,
) {
    let f = crate::rtti::field_ptr(obj, off, in_holder) as *mut u8;
    if f.is_null() || !crate::gum::is_readable(f as *const c_void, 8) {
        return;
    }
    let tn = crate::rtti::type_name_getname(ty);
    match val {
        mlua::Value::Boolean(b) => *f = u8::from(*b),
        mlua::Value::Integer(i) => match crate::rtti::type_size(ty) {
            1 => *f = *i as u8,
            2 => (f as *mut u16).write_unaligned(*i as u16),
            8 => (f as *mut u64).write_unaligned(*i as u64),
            _ => (f as *mut u32).write_unaligned(*i as u32),
        },
        mlua::Value::Number(n) => (f as *mut f32).write_unaligned(*n as f32),
        mlua::Value::String(s) => {
            let st = s.to_str().map(|x| x.to_string()).unwrap_or_default();
            if tn == crate::cname::cname("String") {
                if !crate::rtti::red_string_write_inline(f, &st) {
                    crate::log(&format!("[lua] String >19 nao suportada inline: '{st}'"));
                }
            } else {
                // CName (e fallback p/ campos nomeados por string)
                (f as *mut u64).write_unaligned(crate::cname::cname(&st));
            }
        }
        mlua::Value::UserData(ud) => {
            if let Ok(c) = ud.borrow::<CName>() {
                (f as *mut u64).write_unaligned(c.0);
            } else if let Ok(h) = ud.borrow::<Handle>() {
                (f as *mut *mut c_void).write_unaligned(h.0);
            }
        }
        _ => {}
    }
}

/// Constrói um objeto RED por nome e embrulha em Handle (Nil se a classe não resolver).
fn new_object_handle(lua: &mlua::Lua, class_name: &str) -> mlua::Value {
    let mut out: *mut c_void = std::ptr::null_mut();
    let mut cls: *mut c_void = std::ptr::null_mut();
    unsafe {
        if let Some(r) = crate::registry() {
            cls = r.class_by_name(class_name); // CLASSE conhecida (GetType daria a base)
            out = crate::rtti::new_object(r, class_name);
        }
    }
    if out.is_null() {
        mlua::Value::Nil
    } else {
        lua.create_userdata(Handle(out, cls))
            .map(mlua::Value::UserData)
            .unwrap_or(mlua::Value::Nil)
    }
}

/// Tipos do CET expostos ao Lua como userdata, p/ marshalling correto nos métodos:
/// CName(hash u64), TweakDBID(8B), ItemID(16B). Mods passam esses como args.
#[derive(Clone, Copy)]
pub struct CName(pub u64);
impl mlua::UserData for CName {
    fn add_methods<M: mlua::UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(mlua::MetaMethod::ToString, |_, t, ()| {
            Ok(format!("CName(0x{:016x})", t.0))
        });
        // CET: `cname.value` é a STRING do nome (NativeSettings faz `.value == "..."`,
        // `.value:gsub(...)`, `.value:find(...)`). Resolve via pool interno (name_of); se
        // desconhecido, devolve o hex do hash — SEMPRE string, pra os métodos de string não
        // quebrarem. `.hash` = o número. (Resolver CName→nome geral = CNamePool, em RE.)
        m.add_meta_method(mlua::MetaMethod::Index, |lua, t, k: String| {
            Ok(match k.as_str() {
                "value" => {
                    let s = crate::cname::name_of(t.0)
                        .unwrap_or_else(|| format!("{:016x}", t.0));
                    crate::trace(&format!("CName.value -> '{s}' (hash {:#018x})", t.0));
                    mlua::Value::String(lua.create_string(s.as_bytes())?)
                }
                "hash" => mlua::Value::Integer(t.0 as i64),
                _ => mlua::Value::Nil,
            })
        });
    }
}
#[derive(Clone, Copy)]
pub struct TweakDBID(pub [u8; 8]);
impl mlua::UserData for TweakDBID {
    fn add_methods<M: mlua::UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(mlua::MetaMethod::ToString, |_, t, ()| {
            Ok(format!("TweakDBID({:02x?})", t.0))
        });
    }
}
#[derive(Clone, Copy)]
pub struct ItemId(pub [u8; 16]);
impl mlua::UserData for ItemId {
    fn add_methods<M: mlua::UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(mlua::MetaMethod::ToString, |_, t, ()| {
            Ok(format!("ItemID({:02x?})", &t.0[..8]))
        });
    }
}
/// Vetor de 4 floats (Vector4/Quaternion/EulerAngles do CET) = 16B → Arg::Raw.
/// Campos x/y/z/w (Euler usa roll/pitch/yaw nos 3 primeiros).
#[derive(Clone, Copy)]
pub struct Vec4(pub [f32; 4]);
impl Vec4 {
    fn bytes(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        for (i, f) in self.0.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
        }
        b
    }
}
impl mlua::UserData for Vec4 {
    fn add_fields<F: mlua::UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("x", |_, t| Ok(t.0[0]));
        f.add_field_method_get("y", |_, t| Ok(t.0[1]));
        f.add_field_method_get("z", |_, t| Ok(t.0[2]));
        f.add_field_method_get("w", |_, t| Ok(t.0[3]));
        // CET permite `v.x = 5` — setters por componente.
        f.add_field_method_set("x", |_, t, v: f32| { t.0[0] = v; Ok(()) });
        f.add_field_method_set("y", |_, t, v: f32| { t.0[1] = v; Ok(()) });
        f.add_field_method_set("z", |_, t, v: f32| { t.0[2] = v; Ok(()) });
        f.add_field_method_set("w", |_, t, v: f32| { t.0[3] = v; Ok(()) });
    }
    fn add_methods<M: mlua::UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(mlua::MetaMethod::ToString, |_, t, ()| {
            Ok(format!("Vec4({},{},{},{})", t.0[0], t.0[1], t.0[2], t.0[3]))
        });
        // math 3D (xyz) — como o Vector4 do CET (teleport/distância)
        m.add_method("Length", |_, t, ()| {
            Ok((t.0[0] * t.0[0] + t.0[1] * t.0[1] + t.0[2] * t.0[2]).sqrt())
        });
        m.add_method("LengthSquared", |_, t, ()| {
            Ok(t.0[0] * t.0[0] + t.0[1] * t.0[1] + t.0[2] * t.0[2])
        });
        m.add_method("Distance", |_, t, o: mlua::AnyUserData| {
            let b = o.borrow::<Vec4>()?;
            let (dx, dy, dz) = (t.0[0] - b.0[0], t.0[1] - b.0[1], t.0[2] - b.0[2]);
            Ok((dx * dx + dy * dy + dz * dz).sqrt())
        });
        m.add_method("Dot", |_, t, o: mlua::AnyUserData| {
            let b = o.borrow::<Vec4>()?;
            Ok(t.0[0] * b.0[0] + t.0[1] * b.0[1] + t.0[2] * b.0[2] + t.0[3] * b.0[3])
        });
        m.add_method("Normalize", |lua, t, ()| {
            let len = (t.0[0] * t.0[0] + t.0[1] * t.0[1] + t.0[2] * t.0[2]).sqrt();
            let n = if len > 1e-8 {
                [t.0[0] / len, t.0[1] / len, t.0[2] / len, t.0[3]]
            } else {
                t.0
            };
            lua.create_userdata(Vec4(n))
        });
        // aritmética componente-a-componente (CET soma/subtrai Vector4)
        m.add_meta_method(mlua::MetaMethod::Add, |lua, t, o: mlua::AnyUserData| {
            let b = o.borrow::<Vec4>()?;
            lua.create_userdata(Vec4([
                t.0[0] + b.0[0], t.0[1] + b.0[1], t.0[2] + b.0[2], t.0[3] + b.0[3],
            ]))
        });
        m.add_meta_method(mlua::MetaMethod::Sub, |lua, t, o: mlua::AnyUserData| {
            let b = o.borrow::<Vec4>()?;
            lua.create_userdata(Vec4([
                t.0[0] - b.0[0], t.0[1] - b.0[1], t.0[2] - b.0[2], t.0[3] - b.0[3],
            ]))
        });
    }
}

/// Chama `method` em `obj` via RTTI, marshalando args escalares do Lua → Arg. `known` =
/// classe conhecida (objeto construído) ou null → usa class_of.
unsafe fn call_method(
    lua: &mlua::Lua,
    obj: *mut c_void,
    known: *mut c_void,
    method: &str,
    args: &[mlua::Value],
) -> mlua::Result<mlua::Value> {
    if obj.is_null() || crate::registry().is_none() {
        return Ok(mlua::Value::Nil);
    }
    let cls = if !known.is_null() {
        known
    } else {
        crate::rtti::class_of(obj)
    };
    let rf = match crate::rtti::resolve_in_class(cls, method) {
        Some(r) => r,
        None => {
            let cn = if cls.is_null() {
                "<null>".to_string()
            } else {
                crate::cname::resolve_cname(crate::rtti::type_name_getname(cls))
            };
            // THROTTLE: a UI do jogo chama o mesmo método em loop (ex.: GetControllerByType na
            // medição de aba do NativeSettings) — logar cada um inunda o console. Só loga quando
            // a mensagem MUDA (colapsa runs idênticos).
            let msg = format!("[proxy] metodo '{method}' nao achado (classe '{cn}' cls {cls:p})");
            let dup = LAST_PROXY_MISS
                .lock()
                .map(|mut l| {
                    let same = l.as_deref() == Some(msg.as_str());
                    if !same {
                        *l = Some(msg.clone());
                    }
                    same
                })
                .unwrap_or(false);
            if !dup {
                crate::log(&msg);
            }
            return Ok(mlua::Value::Nil);
        }
    };
    let mut margs: Vec<crate::rtti::Arg> = Vec::with_capacity(args.len());
    for (ai, v) in args.iter().enumerate() {
        match v {
            mlua::Value::Integer(i) => margs.push(crate::rtti::Arg::I32(*i as u32)),
            mlua::Value::Number(n) => margs.push(crate::rtti::Arg::F32(*n as f32)),
            mlua::Value::Boolean(b) => margs.push(crate::rtti::Arg::Bool(*b)),
            mlua::Value::UserData(ud) => {
                if let Ok(h) = ud.borrow::<Handle>() {
                    margs.push(crate::rtti::Arg::Handle(h.0, crate::console::refcnt()));
                } else if let Ok(c) = ud.borrow::<CName>() {
                    margs.push(crate::rtti::Arg::CName(c.0));
                } else if let Ok(t) = ud.borrow::<TweakDBID>() {
                    margs.push(crate::rtti::Arg::Tdb(t.0));
                } else if let Ok(it) = ud.borrow::<ItemId>() {
                    margs.push(crate::rtti::Arg::Item16(it.0));
                } else if let Ok(v) = ud.borrow::<Vec4>() {
                    margs.push(crate::rtti::Arg::Raw(v.bytes()));
                }
            }
            mlua::Value::String(s) => {
                // DIRIGIDO POR TIPO: se o param é `String` → red::String (SetText); senão → CName
                // (a maioria dos métodos pega CName). Antes ia SEMPRE CName → SetText não pintava.
                if let Ok(st) = s.to_str() {
                    let ptn = crate::rtti::fn_param_type(rf.func, ai);
                    let is_str = ptn == crate::cname::cname("String");
                    // PROBE (dev): mostra como cada string é roteada (Str=red::String vs CName).
                    // Pra cravar o bug da aba Mods: se "Modo Imortal"/"Saude Infinita"/"Definir Nivel"
                    // for pra CName, o SetText recebe hash e o rótulo não pinta.
                    if crate::dev_mode() {
                        crate::trace(&format!(
                            "[arg] str '{st}' -> {} (ptn {ptn:#018x})",
                            if is_str { "Str" } else { "CName" }
                        ));
                    }
                    if is_str {
                        margs.push(crate::rtti::Arg::Str(st.to_string()));
                    } else {
                        margs.push(crate::rtti::Arg::CName(crate::cname::cname(&st)));
                    }
                }
            }
            mlua::Value::Table(t) => {
                // table (sequência de strings/CName) → DynArray<CName> = inkWidgetPath, p/
                // `widget:GetWidgetByPath(BuildWidgetPath({...}))`. Sem isto o path ia descartado
                // → GetWidgetByPath devolvia wref null → toda a cadeia de widget falhava.
                let mut cns: Vec<u64> = Vec::new();
                for v in t.clone().sequence_values::<mlua::Value>().flatten() {
                    match v {
                        mlua::Value::String(s) => {
                            if let Ok(st) = s.to_str() {
                                cns.push(crate::cname::cname(&st));
                            }
                        }
                        mlua::Value::UserData(ud) => {
                            if let Ok(c) = ud.borrow::<CName>() {
                                cns.push(c.0);
                            }
                        }
                        _ => {}
                    }
                }
                if !cns.is_empty() {
                    if let Some(b) = crate::rtti::build_cname_dynarray(&cns) {
                        margs.push(crate::rtti::Arg::Array(b));
                    }
                }
            }
            _ => {}
        }
    }
    match crate::rtti::call_func(&rf, obj, &margs) {
        Some(b) => {
            let ret_ty = rf.ret_type;
            // GetName (vale p/ fundamentais) — type_name_hash (IType+0x18) dava lixo p/ String/
            // CName/Int de RETORNO (ex.: GetText/GetToggledIndex), caindo na heurística errada.
            let ret_tn = crate::rtti::type_name_getname(ret_ty);
            let rname = crate::cname::resolve_cname(ret_tn);
            // (Weak)Handle de RETORNO: b[0..16] é o slot {instance, refCount}. Resolve a
            // instância viva + a classe interna → encadeamento `obj:GetController():Setup()`
            // funciona, e os métodos resolvem na classe REAL. (Destrava o PushData E a aba.)
            if rname.starts_with("handle:") || rname.starts_with("whandle:") {
                let inst = {
                    let a = handle_alive(b.as_ptr());
                    if !a.is_null() {
                        a
                    } else {
                        // risco #5: o executor pode escrever só o ptr do objeto (sem refCount).
                        let p = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
                            as *mut c_void;
                        if crate::rtti::sane(p) && crate::gum::is_readable(p as *const c_void, 8) {
                            p
                        } else {
                            std::ptr::null_mut()
                        }
                    }
                };
                if inst.is_null() {
                    return Ok(mlua::Value::Nil);
                }
                let inner = crate::rtti::inner_type(ret_ty);
                return Ok(mlua::Value::UserData(lua.create_userdata(Handle(inst, inner))?));
            }
            let cn = crate::cname::cname;
            let v = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
            if ret_tn == cn("Float") {
                return Ok(mlua::Value::Number(f32::from_bits(v as u32) as f64));
            }
            if ret_tn == cn("Double") {
                return Ok(mlua::Value::Number(f64::from_bits(v)));
            }
            if ret_tn == cn("Bool") {
                return Ok(mlua::Value::Boolean(v & 0xff != 0));
            }
            if ret_tn == cn("CName") {
                return Ok(mlua::Value::UserData(lua.create_userdata(CName(v))?));
            }
            if [
                "Int32", "Uint32", "Int64", "Uint64", "Int8", "Uint8", "Int16", "Uint16",
            ]
            .iter()
            .any(|t| ret_tn == cn(t))
            {
                return Ok(mlua::Value::Integer(v as i64));
            }
            // tipo desconhecido: heurística antiga (objeto encadeável se ptr são > 4GB).
            if v >= 0x1_0000_0000 && crate::rtti::sane(v as *mut c_void) {
                Ok(mlua::Value::UserData(
                    lua.create_userdata(Handle(v as *mut c_void, std::ptr::null_mut()))?,
                ))
            } else {
                Ok(mlua::Value::Integer(v as i64))
            }
        }
        None => Ok(mlua::Value::Nil),
    }
}

/// DrawList do ImGui (paridade CET: `ImGui.GetWindowDrawList():AddLine(...)`). Desenho
/// custom no overlay. Cores = u32 empacotado (IM_COL32: (A<<24)|(B<<16)|(G<<8)|R).
struct DrawList(*mut imgui::sys::ImDrawList);
impl mlua::UserData for DrawList {
    fn add_methods<M: mlua::UserDataMethods<Self>>(m: &mut M) {
        fn iv(x: f32, y: f32) -> imgui::sys::ImVec2 { imgui::sys::ImVec2 { x, y } }
        m.add_method("AddLine", |_, t, (x1, y1, x2, y2, col, th): (f32, f32, f32, f32, i64, Option<f32>)| {
            if crate::overlay::in_draw() && !t.0.is_null() {
                unsafe { imgui::sys::ImDrawList_AddLine(t.0, iv(x1, y1), iv(x2, y2), col as u32, th.unwrap_or(1.0)) };
            }
            Ok(())
        });
        m.add_method("AddRect", |_, t, (x1, y1, x2, y2, col, r, th): (f32, f32, f32, f32, i64, Option<f32>, Option<f32>)| {
            if crate::overlay::in_draw() && !t.0.is_null() {
                unsafe { imgui::sys::ImDrawList_AddRect(t.0, iv(x1, y1), iv(x2, y2), col as u32, r.unwrap_or(0.0), 0, th.unwrap_or(1.0)) };
            }
            Ok(())
        });
        m.add_method("AddRectFilled", |_, t, (x1, y1, x2, y2, col, r): (f32, f32, f32, f32, i64, Option<f32>)| {
            if crate::overlay::in_draw() && !t.0.is_null() {
                unsafe { imgui::sys::ImDrawList_AddRectFilled(t.0, iv(x1, y1), iv(x2, y2), col as u32, r.unwrap_or(0.0), 0) };
            }
            Ok(())
        });
        m.add_method("AddCircle", |_, t, (cx, cy, rad, col, seg, th): (f32, f32, f32, i64, Option<i32>, Option<f32>)| {
            if crate::overlay::in_draw() && !t.0.is_null() {
                unsafe { imgui::sys::ImDrawList_AddCircle(t.0, iv(cx, cy), rad, col as u32, seg.unwrap_or(0), th.unwrap_or(1.0)) };
            }
            Ok(())
        });
        m.add_method("AddCircleFilled", |_, t, (cx, cy, rad, col, seg): (f32, f32, f32, i64, Option<i32>)| {
            if crate::overlay::in_draw() && !t.0.is_null() {
                unsafe { imgui::sys::ImDrawList_AddCircleFilled(t.0, iv(cx, cy), rad, col as u32, seg.unwrap_or(0)) };
            }
            Ok(())
        });
        m.add_method("AddTriangleFilled", |_, t, (x1, y1, x2, y2, x3, y3, col): (f32, f32, f32, f32, f32, f32, i64)| {
            if crate::overlay::in_draw() && !t.0.is_null() {
                unsafe { imgui::sys::ImDrawList_AddTriangleFilled(t.0, iv(x1, y1), iv(x2, y2), iv(x3, y3), col as u32) };
            }
            Ok(())
        });
        m.add_method("AddText", |_, t, (x, y, col, text): (f32, f32, i64, String)| {
            if crate::overlay::in_draw() && !t.0.is_null() {
                let c = std::ffi::CString::new(text).unwrap_or_default();
                unsafe { imgui::sys::ImDrawList_AddText_Vec2(t.0, iv(x, y), col as u32, c.as_ptr(), std::ptr::null()) };
            }
            Ok(())
        });
    }
}

fn setup_imgui(lua: &mlua::Lua) -> mlua::Result<()> {
    // ---- ImGui-pro-Lua: mods desenham a própria janela (a assinatura do CET) ----
    // Usa a API CRUA do Dear ImGui (imgui::sys) — stateful (Begin/End), ideal p/ Lua.
    // Só tem efeito DENTRO do onDraw (overlay::in_draw()), na thread de render.
    let ig = lua.create_table()?;
    fn cstr(s: &str) -> std::ffi::CString {
        std::ffi::CString::new(s).unwrap_or_default()
    }
    ig.set(
        "Begin",
        lua.create_function(|_, title: String| {
            if !crate::overlay::in_draw() {
                return Ok(true);
            }
            let c = cstr(&title);
            Ok(unsafe { imgui::sys::igBegin(c.as_ptr(), std::ptr::null_mut(), 0) })
        })?,
    )?;
    ig.set(
        "End",
        lua.create_function(|_, ()| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igEnd() };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "Text",
        lua.create_function(|_, s: String| {
            if crate::overlay::in_draw() {
                let c = cstr(&s);
                unsafe { imgui::sys::igTextUnformatted(c.as_ptr(), std::ptr::null()) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "Button",
        lua.create_function(|_, s: String| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            let c = cstr(&s);
            Ok(unsafe { imgui::sys::igButton(c.as_ptr(), imgui::sys::ImVec2 { x: 0.0, y: 0.0 }) })
        })?,
    )?;
    ig.set(
        "Separator",
        lua.create_function(|_, ()| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igSeparator() };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "SameLine",
        lua.create_function(|_, ()| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igSameLine(0.0, -1.0) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "Spacing",
        lua.create_function(|_, ()| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igSpacing() };
            }
            Ok(())
        })?,
    )?;
    // posicionamento de janela (ImGuiCond_FirstUseEver=4: posição inicial, user arrasta).
    ig.set(
        "SetNextWindowPos",
        lua.create_function(|_, (x, y): (f32, f32)| {
            if crate::overlay::in_draw() {
                unsafe {
                    imgui::sys::igSetNextWindowPos(
                        imgui::sys::ImVec2 { x, y },
                        4,
                        imgui::sys::ImVec2 { x: 0.0, y: 0.0 },
                    )
                };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "SetNextWindowSize",
        lua.create_function(|_, (w, h): (f32, f32)| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igSetNextWindowSize(imgui::sys::ImVec2 { x: w, y: h }, 4) };
            }
            Ok(())
        })?,
    )?;
    // Checkbox: retorna (novoValor, mudou) — padrão CET.
    ig.set(
        "Checkbox",
        lua.create_function(|_, (label, val): (String, bool)| {
            if !crate::overlay::in_draw() {
                return Ok((val, false));
            }
            let c = cstr(&label);
            let mut b = val;
            let changed = unsafe { imgui::sys::igCheckbox(c.as_ptr(), &mut b as *mut bool) };
            Ok((b, changed))
        })?,
    )?;
    // SliderInt: retorna (novoValor, mudou).
    ig.set(
        "SliderInt",
        lua.create_function(|_, (label, val, min, max): (String, i32, i32, i32)| {
            if !crate::overlay::in_draw() {
                return Ok((val, false));
            }
            let c = cstr(&label);
            let mut v = val;
            let changed = unsafe {
                imgui::sys::igSliderInt(c.as_ptr(), &mut v, min, max, std::ptr::null(), 0)
            };
            Ok((v, changed))
        })?,
    )?;
    // SliderFloat: retorna (novoValor, mudou).
    ig.set(
        "SliderFloat",
        lua.create_function(|_, (label, val, min, max): (String, f32, f32, f32)| {
            if !crate::overlay::in_draw() {
                return Ok((val, false));
            }
            let c = cstr(&label);
            let fmt = cstr("%.3f");
            let mut v = val;
            let changed = unsafe {
                imgui::sys::igSliderFloat(c.as_ptr(), &mut v, min, max, fmt.as_ptr(), 0)
            };
            Ok((v, changed))
        })?,
    )?;
    // ---- lote grande de widgets ImGui (paridade CET) ----
    use std::os::raw::c_char;
    let v4 = |r: f32, g: f32, b: f32, a: f32| imgui::sys::ImVec4 { x: r, y: g, z: b, w: a };
    let v2 = |x: f32, y: f32| imgui::sys::ImVec2 { x, y };
    ig.set(
        "TextColored",
        lua.create_function(move |_, (r, g, b, a, s): (f32, f32, f32, f32, String)| {
            if crate::overlay::in_draw() {
                let c = cstr(&s);
                unsafe {
                    imgui::sys::igPushStyleColor_Vec4(0, v4(r, g, b, a)); // 0 = ImGuiCol_Text
                    imgui::sys::igTextUnformatted(c.as_ptr(), std::ptr::null());
                    imgui::sys::igPopStyleColor(1);
                }
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "TextWrapped",
        lua.create_function(|_, s: String| {
            if crate::overlay::in_draw() {
                let c = cstr(&s);
                unsafe {
                    imgui::sys::igPushTextWrapPos(0.0);
                    imgui::sys::igTextUnformatted(c.as_ptr(), std::ptr::null());
                    imgui::sys::igPopTextWrapPos();
                }
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "InputText",
        lua.create_function(|_, (label, val, maxlen): (String, String, Option<usize>)| {
            if !crate::overlay::in_draw() {
                return Ok((val, false));
            }
            let cap = maxlen.unwrap_or(256).max(2);
            let mut buf = vec![0u8; cap];
            let bytes = val.as_bytes();
            let n = bytes.len().min(cap - 1);
            buf[..n].copy_from_slice(&bytes[..n]);
            let clabel = cstr(&label);
            let changed = unsafe {
                imgui::sys::igInputText(
                    clabel.as_ptr(),
                    buf.as_mut_ptr() as *mut c_char,
                    cap,
                    0,
                    None,
                    std::ptr::null_mut(),
                )
            };
            let end = buf.iter().position(|&b| b == 0).unwrap_or(cap);
            Ok((String::from_utf8_lossy(&buf[..end]).into_owned(), changed))
        })?,
    )?;
    ig.set(
        "InputInt",
        lua.create_function(|_, (label, val): (String, i32)| {
            if !crate::overlay::in_draw() {
                return Ok((val, false));
            }
            let c = cstr(&label);
            let mut v = val;
            let changed = unsafe { imgui::sys::igInputInt(c.as_ptr(), &mut v, 1, 100, 0) };
            Ok((v, changed))
        })?,
    )?;
    ig.set(
        "BeginChild",
        lua.create_function(move |_, (id, w, h): (String, Option<f32>, Option<f32>)| {
            if !crate::overlay::in_draw() {
                return Ok(true);
            }
            let c = cstr(&id);
            Ok(unsafe {
                imgui::sys::igBeginChild_Str(
                    c.as_ptr(),
                    v2(w.unwrap_or(0.0), h.unwrap_or(0.0)),
                    false,
                    0,
                )
            })
        })?,
    )?;
    ig.set(
        "EndChild",
        lua.create_function(|_, ()| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igEndChild() };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "CollapsingHeader",
        lua.create_function(|_, label: String| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            let c = cstr(&label);
            Ok(unsafe { imgui::sys::igCollapsingHeader_TreeNodeFlags(c.as_ptr(), 0) })
        })?,
    )?;
    ig.set(
        "BeginCombo",
        lua.create_function(|_, (label, preview): (String, String)| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            let cl = cstr(&label);
            let cp = cstr(&preview);
            Ok(unsafe { imgui::sys::igBeginCombo(cl.as_ptr(), cp.as_ptr(), 0) })
        })?,
    )?;
    ig.set(
        "EndCombo",
        lua.create_function(|_, ()| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igEndCombo() };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "Selectable",
        lua.create_function(move |_, (label, selected): (String, Option<bool>)| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            let c = cstr(&label);
            Ok(unsafe {
                imgui::sys::igSelectable_Bool(c.as_ptr(), selected.unwrap_or(false), 0, v2(0.0, 0.0))
            })
        })?,
    )?;
    ig.set(
        "RadioButton",
        lua.create_function(|_, (label, active): (String, bool)| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            let c = cstr(&label);
            Ok(unsafe { imgui::sys::igRadioButton_Bool(c.as_ptr(), active) })
        })?,
    )?;
    ig.set(
        "BeginTabBar",
        lua.create_function(|_, id: String| {
            if !crate::overlay::in_draw() {
                return Ok(true);
            }
            let c = cstr(&id);
            Ok(unsafe { imgui::sys::igBeginTabBar(c.as_ptr(), 0) })
        })?,
    )?;
    ig.set(
        "EndTabBar",
        lua.create_function(|_, ()| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igEndTabBar() };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "BeginTabItem",
        lua.create_function(|_, label: String| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            let c = cstr(&label);
            Ok(unsafe { imgui::sys::igBeginTabItem(c.as_ptr(), std::ptr::null_mut(), 0) })
        })?,
    )?;
    ig.set(
        "EndTabItem",
        lua.create_function(|_, ()| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igEndTabItem() };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "PushStyleColor",
        lua.create_function(move |_, (idx, r, g, b, a): (i32, f32, f32, f32, f32)| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igPushStyleColor_Vec4(idx, v4(r, g, b, a)) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "PopStyleColor",
        lua.create_function(|_, count: Option<i32>| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igPopStyleColor(count.unwrap_or(1)) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "SetTooltip",
        lua.create_function(|_, s: String| {
            if crate::overlay::in_draw() {
                let c = cstr(&s);
                unsafe {
                    imgui::sys::igBeginTooltip();
                    imgui::sys::igTextUnformatted(c.as_ptr(), std::ptr::null());
                    imgui::sys::igEndTooltip();
                }
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "Dummy",
        lua.create_function(move |_, (w, h): (f32, f32)| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igDummy(v2(w, h)) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "ProgressBar",
        lua.create_function(move |_, frac: f32| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igProgressBar(frac, v2(-1.0, 0.0), std::ptr::null()) };
            }
            Ok(())
        })?,
    )?;
    // ---- cauda do ImGui (mais widgets de layout/árvore/cor/itens) ----
    macro_rules! ig_void0 {
        ($name:literal, $f:path) => {
            ig.set(
                $name,
                lua.create_function(|_, ()| {
                    if crate::overlay::in_draw() {
                        unsafe { $f() };
                    }
                    Ok(())
                })?,
            )?;
        };
    }
    ig_void0!("BeginGroup", imgui::sys::igBeginGroup);
    ig_void0!("EndGroup", imgui::sys::igEndGroup);
    ig_void0!("TreePop", imgui::sys::igTreePop);
    ig_void0!("PopItemWidth", imgui::sys::igPopItemWidth);
    ig_void0!("PopID", imgui::sys::igPopID);
    ig_void0!("NewLine", imgui::sys::igNewLine);
    ig_void0!("Bullet", imgui::sys::igBullet);
    ig.set(
        "TreeNode",
        lua.create_function(|_, label: String| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            let c = cstr(&label);
            Ok(unsafe { imgui::sys::igTreeNode_Str(c.as_ptr()) })
        })?,
    )?;
    ig.set(
        "SmallButton",
        lua.create_function(|_, label: String| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            let c = cstr(&label);
            Ok(unsafe { imgui::sys::igSmallButton(c.as_ptr()) })
        })?,
    )?;
    ig.set(
        "BulletText",
        lua.create_function(|_, s: String| {
            if crate::overlay::in_draw() {
                let c = cstr(&s);
                unsafe {
                    imgui::sys::igBullet();
                    imgui::sys::igSameLine(0.0, -1.0);
                    imgui::sys::igTextUnformatted(c.as_ptr(), std::ptr::null());
                }
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "PushItemWidth",
        lua.create_function(|_, w: f32| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igPushItemWidth(w) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "PushID",
        lua.create_function(|_, id: String| {
            if crate::overlay::in_draw() {
                let c = cstr(&id);
                unsafe { imgui::sys::igPushID_Str(c.as_ptr()) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "Indent",
        lua.create_function(|_, w: Option<f32>| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igIndent(w.unwrap_or(0.0)) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "Unindent",
        lua.create_function(|_, w: Option<f32>| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igUnindent(w.unwrap_or(0.0)) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "InputFloat",
        lua.create_function(|_, (label, val): (String, f32)| {
            if !crate::overlay::in_draw() {
                return Ok((val, false));
            }
            let c = cstr(&label);
            let fmt = cstr("%.3f");
            let mut v = val;
            let changed =
                unsafe { imgui::sys::igInputFloat(c.as_ptr(), &mut v, 0.0, 0.0, fmt.as_ptr(), 0) };
            Ok((v, changed))
        })?,
    )?;
    ig.set(
        "ColorEdit3",
        lua.create_function(|_, (label, r, g, b): (String, f32, f32, f32)| {
            if !crate::overlay::in_draw() {
                return Ok((r, g, b, false));
            }
            let c = cstr(&label);
            let mut col = [r, g, b];
            let changed =
                unsafe { imgui::sys::igColorEdit3(c.as_ptr(), col.as_mut_ptr(), 0) };
            Ok((col[0], col[1], col[2], changed))
        })?,
    )?;
    ig.set(
        "IsItemHovered",
        lua.create_function(|_, ()| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            Ok(unsafe { imgui::sys::igIsItemHovered(0) })
        })?,
    )?;
    ig.set(
        "IsItemClicked",
        lua.create_function(|_, ()| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            Ok(unsafe { imgui::sys::igIsItemClicked(0) })
        })?,
    )?;
    ig.set(
        "GetWindowWidth",
        lua.create_function(|_, ()| Ok(unsafe { imgui::sys::igGetWindowWidth() }))?,
    )?;
    ig.set(
        "GetWindowHeight",
        lua.create_function(|_, ()| Ok(unsafe { imgui::sys::igGetWindowHeight() }))?,
    )?;
    // ---- lote 3: menus, popups, drag, style-var, cursor, disabled ----
    ig_void0!("EndMenuBar", imgui::sys::igEndMenuBar);
    ig_void0!("EndMenu", imgui::sys::igEndMenu);
    ig_void0!("EndPopup", imgui::sys::igEndPopup);
    ig_void0!("EndDisabled", imgui::sys::igEndDisabled);
    ig_void0!("AlignTextToFramePadding", imgui::sys::igAlignTextToFramePadding);
    ig.set(
        "BeginMenuBar",
        lua.create_function(|_, ()| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            Ok(unsafe { imgui::sys::igBeginMenuBar() })
        })?,
    )?;
    ig.set(
        "BeginMenu",
        lua.create_function(|_, label: String| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            let c = cstr(&label);
            Ok(unsafe { imgui::sys::igBeginMenu(c.as_ptr(), true) })
        })?,
    )?;
    ig.set(
        "MenuItem",
        lua.create_function(|_, label: String| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            let c = cstr(&label);
            Ok(unsafe { imgui::sys::igMenuItem_Bool(c.as_ptr(), std::ptr::null(), false, true) })
        })?,
    )?;
    ig.set(
        "BeginPopup",
        lua.create_function(|_, id: String| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            let c = cstr(&id);
            Ok(unsafe { imgui::sys::igBeginPopup(c.as_ptr(), 0) })
        })?,
    )?;
    ig.set(
        "OpenPopup",
        lua.create_function(|_, id: String| {
            if crate::overlay::in_draw() {
                let c = cstr(&id);
                unsafe { imgui::sys::igOpenPopup_Str(c.as_ptr(), 0) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "BeginDisabled",
        lua.create_function(|_, d: Option<bool>| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igBeginDisabled(d.unwrap_or(true)) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "PopStyleVar",
        lua.create_function(|_, count: Option<i32>| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igPopStyleVar(count.unwrap_or(1)) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "PushStyleVar",
        lua.create_function(|_, (idx, val): (i32, f32)| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igPushStyleVar_Float(idx, val) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "DragFloat",
        lua.create_function(|_, (label, val, speed, min, max): (String, f32, Option<f32>, Option<f32>, Option<f32>)| {
            if !crate::overlay::in_draw() {
                return Ok((val, false));
            }
            let c = cstr(&label);
            let fmt = cstr("%.3f");
            let mut v = val;
            let changed = unsafe {
                imgui::sys::igDragFloat(c.as_ptr(), &mut v, speed.unwrap_or(1.0), min.unwrap_or(0.0), max.unwrap_or(0.0), fmt.as_ptr(), 0)
            };
            Ok((v, changed))
        })?,
    )?;
    ig.set(
        "DragInt",
        lua.create_function(|_, (label, val, speed, min, max): (String, i32, Option<f32>, Option<i32>, Option<i32>)| {
            if !crate::overlay::in_draw() {
                return Ok((val, false));
            }
            let c = cstr(&label);
            let fmt = cstr("%d");
            let mut v = val;
            let changed = unsafe {
                imgui::sys::igDragInt(c.as_ptr(), &mut v, speed.unwrap_or(1.0), min.unwrap_or(0), max.unwrap_or(0), fmt.as_ptr(), 0)
            };
            Ok((v, changed))
        })?,
    )?;
    ig.set(
        "ColorEdit4",
        lua.create_function(|_, (label, r, g, b, a): (String, f32, f32, f32, f32)| {
            if !crate::overlay::in_draw() {
                return Ok((r, g, b, a, false));
            }
            let c = cstr(&label);
            let mut col = [r, g, b, a];
            let changed = unsafe { imgui::sys::igColorEdit4(c.as_ptr(), col.as_mut_ptr(), 0) };
            Ok((col[0], col[1], col[2], col[3], changed))
        })?,
    )?;
    ig.set(
        "SetCursorPosX",
        lua.create_function(|_, x: f32| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igSetCursorPosX(x) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "SetCursorPosY",
        lua.create_function(|_, y: f32| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igSetCursorPosY(y) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "GetCursorPosX",
        lua.create_function(|_, ()| Ok(unsafe { imgui::sys::igGetCursorPosX() }))?,
    )?;
    ig.set(
        "GetCursorPosY",
        lua.create_function(|_, ()| Ok(unsafe { imgui::sys::igGetCursorPosY() }))?,
    )?;
    ig.set(
        "SetKeyboardFocusHere",
        lua.create_function(|_, ()| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igSetKeyboardFocusHere(0) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "GetContentRegionAvail",
        lua.create_function(|_, ()| {
            let mut v = imgui::sys::ImVec2 { x: 0.0, y: 0.0 };
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igGetContentRegionAvail(&mut v) };
            }
            Ok((v.x, v.y))
        })?,
    )?;
    // ---- lote 4: tables, item-state, scroll, mouse (macros p/ os padrões simples) ----
    macro_rules! ig_b0 {
        ($n:literal, $f:path) => {
            ig.set(
                $n,
                lua.create_function(|_, ()| {
                    Ok(if crate::overlay::in_draw() {
                        unsafe { $f() }
                    } else {
                        false
                    })
                })?,
            )?;
        };
    }
    macro_rules! ig_f0 {
        ($n:literal, $f:path) => {
            ig.set($n, lua.create_function(|_, ()| Ok(unsafe { $f() }))?)?;
        };
    }
    ig_b0!("IsItemActive", imgui::sys::igIsItemActive);
    ig_b0!("IsItemFocused", imgui::sys::igIsItemFocused);
    ig_b0!("IsItemEdited", imgui::sys::igIsItemEdited);
    ig_b0!("IsItemActivated", imgui::sys::igIsItemActivated);
    ig_b0!("IsItemDeactivatedAfterEdit", imgui::sys::igIsItemDeactivatedAfterEdit);
    ig_b0!("IsAnyItemHovered", imgui::sys::igIsAnyItemHovered);
    ig.set(
        "IsWindowFocused",
        lua.create_function(|_, ()| Ok(unsafe { imgui::sys::igIsWindowFocused(0) }))?,
    )?;
    ig.set(
        "IsWindowHovered",
        lua.create_function(|_, ()| Ok(unsafe { imgui::sys::igIsWindowHovered(0) }))?,
    )?;
    ig_b0!("TableNextColumn", imgui::sys::igTableNextColumn);
    ig_void0!("SetItemDefaultFocus", imgui::sys::igSetItemDefaultFocus);
    ig_void0!("CloseCurrentPopup", imgui::sys::igCloseCurrentPopup);
    ig_void0!("EndTable", imgui::sys::igEndTable);
    ig_void0!("TableHeadersRow", imgui::sys::igTableHeadersRow);
    ig_f0!("GetScrollY", imgui::sys::igGetScrollY);
    ig_f0!("GetScrollMaxY", imgui::sys::igGetScrollMaxY);
    ig_f0!("GetFrameHeight", imgui::sys::igGetFrameHeight);
    ig_f0!("GetFrameHeightWithSpacing", imgui::sys::igGetFrameHeightWithSpacing);
    ig_f0!("GetTextLineHeight", imgui::sys::igGetTextLineHeight);
    ig_f0!("GetTextLineHeightWithSpacing", imgui::sys::igGetTextLineHeightWithSpacing);
    // ---- Fase 1b (paridade CET): mais funções ImGui (assinaturas verificadas em imgui-sys-0.12) ----
    ig_void0!("EndTooltip", imgui::sys::igEndTooltip);
    ig_b0!("IsWindowAppearing", imgui::sys::igIsWindowAppearing);
    ig_b0!("IsAnyItemActive", imgui::sys::igIsAnyItemActive);
    ig_b0!("IsAnyItemFocused", imgui::sys::igIsAnyItemFocused);
    ig_f0!("GetScrollX", imgui::sys::igGetScrollX);
    ig.set("BeginTooltip", lua.create_function(|_, ()| {
        if crate::overlay::in_draw() { unsafe { imgui::sys::igBeginTooltip() }; }
        Ok(true)
    })?)?;
    ig.set("SetNextItemWidth", lua.create_function(|_, w: f32| {
        if crate::overlay::in_draw() { unsafe { imgui::sys::igSetNextItemWidth(w) }; }
        Ok(())
    })?)?;
    ig.set("SetWindowFontScale", lua.create_function(|_, s: f32| {
        if crate::overlay::in_draw() { unsafe { imgui::sys::igSetWindowFontScale(s) }; }
        Ok(())
    })?)?;
    ig.set("ArrowButton", lua.create_function(|_, (id, dir): (String, i32)| {
        if !crate::overlay::in_draw() { return Ok(false); }
        let c = cstr(&id);
        Ok(unsafe { imgui::sys::igArrowButton(c.as_ptr(), dir) })
    })?)?;
    ig.set("TextDisabled", lua.create_function(|_, s: String| {
        if crate::overlay::in_draw() {
            let f = cstr("%s"); let c = cstr(&s);
            unsafe { imgui::sys::igTextDisabled(f.as_ptr(), c.as_ptr()) };
        }
        Ok(())
    })?)?;
    ig.set("LabelText", lua.create_function(|_, (label, text): (String, String)| {
        if crate::overlay::in_draw() {
            let l = cstr(&label); let f = cstr("%s"); let t = cstr(&text);
            unsafe { imgui::sys::igLabelText(l.as_ptr(), f.as_ptr(), t.as_ptr()) };
        }
        Ok(())
    })?)?;
    ig.set("GetCursorScreenPos", lua.create_function(|_, ()| {
        let mut o = imgui::sys::ImVec2 { x: 0.0, y: 0.0 };
        if crate::overlay::in_draw() { unsafe { imgui::sys::igGetCursorScreenPos(&mut o) }; }
        Ok((o.x, o.y))
    })?)?;
    // multi-componente: recebem (label, x,y,z[,w], min,max) → retornam (changed, x,y,z[,w]).
    ig.set("SliderFloat3", lua.create_function(|_, (label, x, y, z, mn, mx): (String, f32, f32, f32, f32, f32)| {
        if !crate::overlay::in_draw() { return Ok((false, x, y, z)); }
        let l = cstr(&label); let f = cstr("%.3f"); let mut v = [x, y, z];
        let ch = unsafe { imgui::sys::igSliderFloat3(l.as_ptr(), v.as_mut_ptr(), mn, mx, f.as_ptr(), 0) };
        Ok((ch, v[0], v[1], v[2]))
    })?)?;
    ig.set("SliderFloat4", lua.create_function(|_, (label, x, y, z, w, mn, mx): (String, f32, f32, f32, f32, f32, f32)| {
        if !crate::overlay::in_draw() { return Ok((false, x, y, z, w)); }
        let l = cstr(&label); let f = cstr("%.3f"); let mut v = [x, y, z, w];
        let ch = unsafe { imgui::sys::igSliderFloat4(l.as_ptr(), v.as_mut_ptr(), mn, mx, f.as_ptr(), 0) };
        Ok((ch, v[0], v[1], v[2], v[3]))
    })?)?;
    ig.set("DragFloat3", lua.create_function(|_, (label, x, y, z, speed): (String, f32, f32, f32, Option<f32>)| {
        if !crate::overlay::in_draw() { return Ok((false, x, y, z)); }
        let l = cstr(&label); let f = cstr("%.3f"); let mut v = [x, y, z];
        let ch = unsafe { imgui::sys::igDragFloat3(l.as_ptr(), v.as_mut_ptr(), speed.unwrap_or(1.0), 0.0, 0.0, f.as_ptr(), 0) };
        Ok((ch, v[0], v[1], v[2]))
    })?)?;
    ig.set("InputFloat2", lua.create_function(|_, (label, x, y): (String, f32, f32)| {
        if !crate::overlay::in_draw() { return Ok((false, x, y)); }
        let l = cstr(&label); let f = cstr("%.3f"); let mut v = [x, y];
        let ch = unsafe { imgui::sys::igInputFloat2(l.as_ptr(), v.as_mut_ptr(), f.as_ptr(), 0) };
        Ok((ch, v[0], v[1]))
    })?)?;
    ig.set("InputFloat3", lua.create_function(|_, (label, x, y, z): (String, f32, f32, f32)| {
        if !crate::overlay::in_draw() { return Ok((false, x, y, z)); }
        let l = cstr(&label); let f = cstr("%.3f"); let mut v = [x, y, z];
        let ch = unsafe { imgui::sys::igInputFloat3(l.as_ptr(), v.as_mut_ptr(), f.as_ptr(), 0) };
        Ok((ch, v[0], v[1], v[2]))
    })?)?;
    // cauda ImGui (assinaturas verificadas em imgui-sys-0.12)
    ig_void0!("EndListBox", imgui::sys::igEndListBox);
    ig_void0!("NextColumn", imgui::sys::igNextColumn);
    ig_void0!("PopTextWrapPos", imgui::sys::igPopTextWrapPos);
    ig_void0!("EndMainMenuBar", imgui::sys::igEndMainMenuBar);
    ig_b0!("BeginMainMenuBar", imgui::sys::igBeginMainMenuBar);
    ig.set("BeginListBox", lua.create_function(|_, label: String| {
        if !crate::overlay::in_draw() { return Ok(false); }
        let l = cstr(&label);
        Ok(unsafe { imgui::sys::igBeginListBox(l.as_ptr(), imgui::sys::ImVec2 { x: 0.0, y: 0.0 }) })
    })?)?;
    ig.set("TreeNodeEx", lua.create_function(|_, (label, flags): (String, Option<i32>)| {
        if !crate::overlay::in_draw() { return Ok(false); }
        let l = cstr(&label);
        Ok(unsafe { imgui::sys::igTreeNodeEx_Str(l.as_ptr(), flags.unwrap_or(0)) })
    })?)?;
    ig.set("SetNextItemOpen", lua.create_function(|_, (open, cond): (bool, Option<i32>)| {
        if crate::overlay::in_draw() { unsafe { imgui::sys::igSetNextItemOpen(open, cond.unwrap_or(0)) }; }
        Ok(())
    })?)?;
    ig.set("Columns", lua.create_function(|_, (count, border): (Option<i32>, Option<bool>)| {
        if crate::overlay::in_draw() { unsafe { imgui::sys::igColumns(count.unwrap_or(1), std::ptr::null(), border.unwrap_or(true)) }; }
        Ok(())
    })?)?;
    ig.set("GetColumnWidth", lua.create_function(|_, idx: Option<i32>| {
        Ok(if crate::overlay::in_draw() { unsafe { imgui::sys::igGetColumnWidth(idx.unwrap_or(-1)) } } else { 0.0 })
    })?)?;
    ig.set("GetFrameCount", lua.create_function(|_, ()| Ok(unsafe { imgui::sys::igGetFrameCount() } as i64))?)?;
    ig.set("SetMouseCursor", lua.create_function(|_, c: i32| {
        if crate::overlay::in_draw() { unsafe { imgui::sys::igSetMouseCursor(c) }; }
        Ok(())
    })?)?;
    ig.set("PushTextWrapPos", lua.create_function(|_, pos: Option<f32>| {
        if crate::overlay::in_draw() { unsafe { imgui::sys::igPushTextWrapPos(pos.unwrap_or(0.0)) }; }
        Ok(())
    })?)?;
    ig.set("InvisibleButton", lua.create_function(|_, (id, w, h): (String, f32, f32)| {
        if !crate::overlay::in_draw() { return Ok(false); }
        let c = cstr(&id);
        Ok(unsafe { imgui::sys::igInvisibleButton(c.as_ptr(), imgui::sys::ImVec2 { x: w, y: h }, 0) })
    })?)?;
    ig.set("BeginPopupModal", lua.create_function(|_, name: String| {
        if !crate::overlay::in_draw() { return Ok(false); }
        let c = cstr(&name);
        Ok(unsafe { imgui::sys::igBeginPopupModal(c.as_ptr(), std::ptr::null_mut(), 0) })
    })?)?;
    ig.set("BeginPopupContextItem", lua.create_function(|_, ()| {
        if !crate::overlay::in_draw() { return Ok(false); }
        Ok(unsafe { imgui::sys::igBeginPopupContextItem(std::ptr::null(), 1) })
    })?)?;
    // draw-list: desenho custom (linhas/retângulos/círculos/texto) — paridade CET.
    ig.set("GetWindowDrawList", lua.create_function(|lua, ()| {
        let dl = if crate::overlay::in_draw() {
            unsafe { imgui::sys::igGetWindowDrawList() }
        } else {
            std::ptr::null_mut()
        };
        lua.create_userdata(DrawList(dl))
    })?)?;
    // ImGui.ColorU32(r,g,b[,a]) (0-255) → u32 empacotado (IM_COL32) p/ as cores do draw-list.
    ig.set("ColorU32", lua.create_function(|_, (r, g, b, a): (i64, i64, i64, Option<i64>)| {
        let r = (r & 0xff) as u32; let g = (g & 0xff) as u32; let b = (b & 0xff) as u32;
        let a = (a.unwrap_or(255) & 0xff) as u32;
        Ok(((a << 24) | (b << 16) | (g << 8) | r) as i64)
    })?)?;
    // Combo/ListBox com tabela de itens (0-based, como o ImGui). Retornam (changed, current).
    ig.set("Combo", lua.create_function(|_, (label, current, items): (String, i32, mlua::Table)| {
        if !crate::overlay::in_draw() { return Ok((false, current)); }
        let strs: Vec<String> = items.sequence_values::<String>().flatten().collect();
        let mut joined = String::new();
        for s in &strs { joined.push_str(s); joined.push('\0'); }
        joined.push('\0');
        let l = cstr(&label);
        let mut cur = current;
        let ch = unsafe { imgui::sys::igCombo_Str(l.as_ptr(), &mut cur, joined.as_ptr() as *const std::os::raw::c_char, -1) };
        Ok((ch, cur))
    })?)?;
    ig.set("ListBox", lua.create_function(|_, (label, current, items): (String, i32, mlua::Table)| {
        if !crate::overlay::in_draw() { return Ok((false, current)); }
        let strs: Vec<String> = items.sequence_values::<String>().flatten().collect();
        let citems: Vec<std::ffi::CString> = strs.iter().map(|s| std::ffi::CString::new(s.as_str()).unwrap_or_default()).collect();
        let ptrs: Vec<*const std::os::raw::c_char> = citems.iter().map(|c| c.as_ptr()).collect();
        let l = cstr(&label);
        let mut cur = current;
        let ch = unsafe { imgui::sys::igListBox_Str_arr(l.as_ptr(), &mut cur, ptrs.as_ptr(), ptrs.len() as i32, -1) };
        Ok((ch, cur))
    })?)?;
    ig.set(
        "BeginTable",
        lua.create_function(move |_, (id, cols): (String, i32)| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            let c = cstr(&id);
            Ok(unsafe { imgui::sys::igBeginTable(c.as_ptr(), cols, 0, v2(0.0, 0.0), 0.0) })
        })?,
    )?;
    ig.set(
        "TableNextRow",
        lua.create_function(|_, ()| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igTableNextRow(0, 0.0) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "TableSetupColumn",
        lua.create_function(|_, label: String| {
            if crate::overlay::in_draw() {
                let c = cstr(&label);
                unsafe { imgui::sys::igTableSetupColumn(c.as_ptr(), 0, 0.0, 0) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "TableSetColumnIndex",
        lua.create_function(|_, n: i32| {
            if !crate::overlay::in_draw() {
                return Ok(false);
            }
            Ok(unsafe { imgui::sys::igTableSetColumnIndex(n) })
        })?,
    )?;
    ig.set(
        "SetScrollHereY",
        lua.create_function(|_, r: Option<f32>| {
            if crate::overlay::in_draw() {
                unsafe { imgui::sys::igSetScrollHereY(r.unwrap_or(0.5)) };
            }
            Ok(())
        })?,
    )?;
    ig.set(
        "GetMousePos",
        lua.create_function(|_, ()| {
            let mut v = imgui::sys::ImVec2 { x: 0.0, y: 0.0 };
            unsafe { imgui::sys::igGetMousePos(&mut v) };
            Ok((v.x, v.y))
        })?,
    )?;
    ig.set(
        "IsMouseClicked",
        lua.create_function(|_, btn: Option<i32>| {
            Ok(unsafe { imgui::sys::igIsMouseClicked(btn.unwrap_or(0), false) })
        })?,
    )?;
    ig.set(
        "IsMouseDown",
        lua.create_function(|_, btn: Option<i32>| {
            Ok(unsafe { imgui::sys::igIsMouseDown(btn.unwrap_or(0)) })
        })?,
    )?;
    lua.globals().set("ImGui", ig)?;
    setup_imgui_enums(lua)?; // tabelas de enum nomeadas (ImGuiWindowFlags.NoResize, ImGuiCol.Text, ...)
    Ok(())
}

fn setup_game(lua: &mlua::Lua) -> mlua::Result<()> {
    let g = lua.create_table()?;
    g.set(
        "AddToInventory",
        lua.create_function(|_, (name, qty): (String, Option<i32>)| {
            let q = qty.unwrap_or(1).max(1) as u32;
            unsafe { with_engine(|r, p, t| { console::give(r, p, t, &name, q); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "AddMoney",
        lua.create_function(|_, n: i32| {
            unsafe { with_engine(|r, p, t| { console::give(r, p, t, "Items.money", n.max(1) as u32); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "AddPerkPoints",
        lua.create_function(|_, n: i32| {
            unsafe { with_engine(|r, p, _| { console::add_points(r, p, n.max(1) as u32, "Primary"); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "AddAttributePoints",
        lua.create_function(|_, n: i32| {
            unsafe { with_engine(|r, p, _| { console::add_points(r, p, n.max(1) as u32, "Attribute"); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "AddRelicPoints",
        lua.create_function(|_, n: i32| {
            unsafe { with_engine(|r, p, _| { console::add_points(r, p, n.max(1) as u32, "Espionage"); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "SetLevel",
        lua.create_function(|_, n: i32| {
            unsafe { with_engine(|r, p, _| { console::level(r, p, n.max(1) as u32); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "Heal",
        lua.create_function(|_, ()| {
            unsafe { with_engine(|r, p, _| { console::heal(r, p); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "GodMode",
        lua.create_function(|_, on: Option<bool>| {
            let on = on.unwrap_or(true);
            unsafe { with_engine(|r, p, _| { console::godmode(r, p, on); }) };
            Ok(())
        })?,
    )?;
    // Proxy genérico: Game.GetPlayer() → Handle do V vivo; Game.Wrap(addr) → Handle
    // de um ponteiro cru (p/ encadear retornos de getter). handle:Method(...) resolve.
    g.set(
        "GetPlayer",
        lua.create_function(|lua, ()| {
            let p = crate::current_player();
            if p.is_null() {
                return Ok(mlua::Value::Nil);
            }
            Ok(mlua::Value::UserData(lua.create_userdata(Handle(p, std::ptr::null_mut()))?))
        })?,
    )?;
    g.set(
        "Wrap",
        lua.create_function(|lua, addr: i64| {
            Ok(mlua::Value::UserData(
                lua.create_userdata(Handle(addr as usize as *mut c_void, std::ptr::null_mut()))?,
            ))
        })?,
    )?;
    // Game.GetSingleton(nome) → Handle de um sistema scriptável (PlayerDevelopmentSystem,
    // etc.). Muitos mods começam pegando um sistema e chamando métodos nele.
    g.set(
        "GetSingleton",
        lua.create_function(|lua, name: String| {
            let mut sysptr: *mut c_void = std::ptr::null_mut();
            unsafe {
                with_engine(|r, p, _| sysptr = crate::console::get_singleton(r, p, &name));
            }
            if sysptr.is_null() {
                Ok(mlua::Value::Nil)
            } else {
                Ok(mlua::Value::UserData(lua.create_userdata(Handle(sysptr, std::ptr::null_mut()))?))
            }
        })?,
    )?;

    // Aliases naturais: o usuário pensa em "money/heal/god" (os atalhos do console),
    // então `Game.money(7777)` funciona igual `Game.AddMoney(7777)`.
    if let Ok(f) = g.get::<mlua::Function>("AddMoney") {
        g.set("money", f.clone())?;
        g.set("Money", f)?;
    }
    if let Ok(f) = g.get::<mlua::Function>("Heal") {
        g.set("heal", f)?;
    }
    if let Ok(f) = g.get::<mlua::Function>("GodMode") {
        g.set("god", f.clone())?;
        g.set("godmode", f)?;
    }
    // Metatable: acessar um campo inexistente de Game.* loga uma DICA clara (com as
    // funções válidas) em vez de só estourar "attempt to call a nil value" cru.
    let mt = lua.create_table()?;
    mt.set(
        "__index",
        lua.create_function(|lua, (_t, k): (mlua::Table, String)| {
            // overloads mangled (ex.: "OperatorEqual;IScriptableIScriptable;Bool") — o CET
            // resolve como função estática. Minimamente: identidade de ponteiro entre Handles
            // (== isSameInstance do NativeSettings / Ref.Equals). Cobre getOptionTable.
            if k.starts_with("OperatorEqual") || k.starts_with("OperatorNotEqual") {
                let neg = k.starts_with("OperatorNotEqual");
                return Ok(mlua::Value::Function(lua.create_function(
                    move |_, (a, b): (mlua::Value, mlua::Value)| {
                        let eq = handle_ptr(&a) == handle_ptr(&b);
                        Ok(if neg { !eq } else { eq })
                    },
                )?));
            }
            crate::log(&format!(
                "[lua] Game.{k} nao existe. Funcoes: AddMoney/AddToInventory/AddPerkPoints/\
                 AddAttributePoints/AddRelicPoints/SetLevel/Heal/GodMode (atalhos: money N, heal, godmode)"
            ));
            Ok(mlua::Value::Nil)
        })?,
    )?;
    g.set_metatable(Some(mt));
    lua.globals().set("Game", g)?;
    Ok(())
}

fn setup_hooks(lua: &mlua::Lua) -> mlua::Result<()> {
    // ---- Observe / ObserveAfter / Override: o coração do CET ----
    // Registram um callback p/ quando o jogo chamar <classe>.<método>. O registro
    // é em 2 tempos (queue agora, resolve no cp77_tick na thread do jogo).
    use crate::hooks::{queue, Kind};
    lua.globals().set(
        "Observe",
        lua.create_function(|lua, (cls, m, cb): (String, String, mlua::Function)| {
            let key = lua.create_registry_value(cb)?;
            queue(cls, m, Kind::Before, key);
            Ok(())
        })?,
    )?;
    lua.globals().set(
        "ObserveAfter",
        lua.create_function(|lua, (cls, m, cb): (String, String, mlua::Function)| {
            let key = lua.create_registry_value(cb)?;
            queue(cls, m, Kind::After, key);
            Ok(())
        })?,
    )?;
    lua.globals().set(
        "Override",
        lua.create_function(|lua, (cls, m, cb): (String, String, mlua::Function)| {
            let key = lua.create_registry_value(cb)?;
            queue(cls, m, Kind::Override, key);
            Ok(())
        })?,
    )?;
    // Suppress(classe, método, cb): roda o cb NO LUGAR da original (a original é PULADA).
    // É o Override-suppress do CET — pra métodos com efeito colateral (não só getter).
    lua.globals().set(
        "Suppress",
        lua.create_function(|lua, (cls, m, cb): (String, String, mlua::Function)| {
            let key = lua.create_registry_value(cb)?;
            queue(cls, m, Kind::Suppress, key);
            Ok(())
        })?,
    )?;

    // registerHotkey(tecla, cb): dispara o cb quando a tecla é pressionada em gameplay
    // (overlay fechado). Tecla = string de 1 caractere (ex: "k").
    lua.globals().set(
        "registerHotkey",
        lua.create_function(|lua, (key, cb): (String, mlua::Function)| {
            let rk = lua.create_registry_value(cb)?;
            if let Some(c) = key.chars().next() {
                crate::hotkey_register_char(c);
            }
            let mut g = HOTKEYS.lock().unwrap_or_else(|e| e.into_inner());
            g.get_or_insert_with(Default::default).insert(key, rk);
            Ok(())
        })?,
    )?;
    // registerInput(tecla, [descrição,] cb): o cb(isDown) dispara no KEY-DOWN e no
    // KEY-UP em gameplay (overlay fechado). Igual ao registerInput do CET, mas a
    // tecla É o bind (não há UI de bindings). Aceita (tecla, cb) ou (tecla, desc, cb).
    lua.globals().set(
        "registerInput",
        lua.create_function(|lua, (key, a, b): (String, mlua::Value, mlua::Value)| {
            let cb = match (a, b) {
                (_, mlua::Value::Function(f)) => f,   // (tecla, descrição, cb)
                (mlua::Value::Function(f), _) => f,   // (tecla, cb)
                _ => {
                    return Err(mlua::Error::RuntimeError(
                        "registerInput(tecla, [descrição,] callback): callback ausente".into(),
                    ))
                }
            };
            if let Some(c) = key.chars().next() {
                crate::input_register_char(c);
                let rk = lua.create_registry_value(cb)?;
                let mut g = INPUTS.lock().unwrap_or_else(|e| e.into_inner());
                g.get_or_insert_with(Default::default).insert(c, rk);
            }
            Ok(())
        })?,
    )?;
    // SetTheme(idx): troca o tema do console Blackwall.sys (0=Entropism..3=Neokitsch).
    lua.globals().set(
        "SetTheme",
        lua.create_function(|_, idx: i32| {
            crate::overlay::request_theme(idx);
            Ok(())
        })?,
    )?;

    Ok(())
}

fn setup_api(lua: &mlua::Lua) -> mlua::Result<()> {
    setup_game(lua)?; // proxy Game.* + cheats — extraído

    // registerForEvent("onInit"/"onUpdate"/..., fn) → guarda numa tabela global.
    lua.globals().set("__events", lua.create_table()?)?;
    lua.globals().set(
        "registerForEvent",
        lua.create_function(|lua, (ev, cb): (String, mlua::Function)| {
            let events: mlua::Table = lua.globals().get("__events")?;
            let list: mlua::Table = match events.get::<Option<mlua::Table>>(ev.as_str())? {
                Some(t) => t,
                None => {
                    let t = lua.create_table()?;
                    events.set(ev.as_str(), &t)?;
                    t
                }
            };
            list.push(cb)?;
            Ok(())
        })?,
    )?;
    lua.globals().set(
        "print",
        lua.create_function(|_, msg: String| {
            crate::log(&format!("[lua] {msg}"));
            Ok(())
        })?,
    )?;

    // ---- Fase 1 (paridade CET): utils + construtores faltantes ----
    // IsDefined(h): true se h é Handle não-null (null-check essencial dos mods CET).
    lua.globals().set(
        "IsDefined",
        lua.create_function(|_, v: mlua::Value| {
            Ok(match v {
                mlua::Value::UserData(ud) => ud.borrow::<Handle>().map(|h| !h.0.is_null()).unwrap_or(true),
                mlua::Value::Nil => false,
                _ => true,
            })
        })?,
    )?;
    // Vector3(x,y,z) — usa Vec4 com w=0 (cobre x/y/z; marshalling 12B = refino futuro).
    lua.globals().set(
        "Vector3",
        lua.create_function(|lua, (x, y, z): (Option<f32>, Option<f32>, Option<f32>)| {
            lua.create_userdata(Vec4([x.unwrap_or(0.0), y.unwrap_or(0.0), z.unwrap_or(0.0), 0.0]))
        })?,
    )?;
    // To* construtores table-style (CET): ToVector4{x,y,z,w} etc.
    fn tf(t: &mlua::Table, k: &str) -> f32 { t.get::<f32>(k).unwrap_or(0.0) }
    lua.globals().set("ToVector4", lua.create_function(|lua, t: mlua::Table| {
        lua.create_userdata(Vec4([tf(&t,"x"),tf(&t,"y"),tf(&t,"z"),tf(&t,"w")]))
    })?)?;
    lua.globals().set("ToVector3", lua.create_function(|lua, t: mlua::Table| {
        lua.create_userdata(Vec4([tf(&t,"x"),tf(&t,"y"),tf(&t,"z"),0.0]))
    })?)?;
    lua.globals().set("ToEulerAngles", lua.create_function(|lua, t: mlua::Table| {
        lua.create_userdata(Vec4([tf(&t,"roll"),tf(&t,"pitch"),tf(&t,"yaw"),0.0]))
    })?)?;
    lua.globals().set("ToQuaternion", lua.create_function(|lua, t: mlua::Table| {
        lua.create_userdata(Vec4([tf(&t,"i"),tf(&t,"j"),tf(&t,"k"),tf(&t,"r")]))
    })?)?;
    // spdlog.* : logging (mesmo sink do print). Nota: 'warning', não 'warn'.
    {
        let sp = lua.create_table()?;
        for lvl in ["info", "debug", "trace", "warning", "error", "critical"] {
            let l = lvl.to_string();
            sp.set(lvl, lua.create_function(move |_, msg: String| {
                crate::log(&format!("[{l}] {msg}"));
                Ok(())
            })?)?;
        }
        lua.globals().set("spdlog", sp)?;
    }
    // Enum(tipo, membro) → valor inteiro do enum (resolve_enum_value já existe). EnumInt(e) → int.
    lua.globals().set("Enum", lua.create_function(|_, (ty, member): (String, String)| {
        let mut out: Option<i64> = None;
        unsafe {
            if let Some(reg) = crate::registry() {
                out = crate::rtti::resolve_enum_value(reg, &ty, &member).map(|v| v as i64);
            }
        }
        Ok(out)
    })?)?;
    lua.globals().set("EnumInt", lua.create_function(|_, v: mlua::Value| {
        Ok(match v {
            mlua::Value::Integer(i) => Some(i),
            mlua::Value::Number(n) => Some(n as i64),
            _ => None,
        })
    })?)?;
    // Proxies de enum NOMEADOS (CET): `PauseMenuAction.OpenSubMenu` → valor int via RTTI.
    // Membro/enum não-resolvido → nil. Usados nos corpos dos hooks de menu (Stage B).
    for ty in [
        "PauseMenuAction",
        "textLetterCase",
        "inkEAnchor",
        "textHorizontalAlignment",
        "textVerticalAlignment",
    ] {
        let t = lua.create_table()?;
        let emt = lua.create_table()?;
        emt.set(
            "__index",
            lua.create_function(move |_, (_t, member): (mlua::Table, String)| {
                let mut out: Option<i64> = None;
                unsafe {
                    if let Some(reg) = crate::registry() {
                        out = crate::rtti::resolve_enum_value(reg, ty, &member).map(|v| v as i64);
                    }
                }
                Ok(out)
            })?,
        )?;
        t.set_metatable(Some(emt));
        lua.globals().set(ty, t)?;
    }
    // GetDisplayResolution() → (w, h) do frame do jogo (o overlay captura no render).
    lua.globals().set("GetDisplayResolution", lua.create_function(|_, ()| {
        let (w, h) = crate::overlay::frame_size();
        Ok((w as i64, h as i64))
    })?)?;
    // NewObject(className) → Handle de instância nova. DESGATEADO: Construct@0x40 (offsets
    // macOS) validado in-game (`newobj Vector4`→ptr OK) + alloc do POOL DO RED (não corrompe
    // no free do engine). Nil se a classe não resolver.
    lua.globals().set("NewObject", lua.create_function(|lua, class_name: String| {
        Ok(new_object_handle(lua, &class_name))
    })?)?;
    // _G metatable (estilo CET): um GLOBAL indefinido que SEJA uma classe RTTI registrada
    // vira um proxy chamável com `.new()` → constrói a instância. Ex.: `PauseMenuListItemData
    // .new()`, `inkTextWidget.new()`. Só dispara p/ chave faltante E classe existente → não
    // muda o resto (nomes desconhecidos seguem nil). É o que destrava os widgets do NativeSettings.
    {
        let g_mt = lua.create_table()?;
        g_mt.set(
            "__index",
            lua.create_function(|lua, (_g, key): (mlua::Table, String)| {
                let exists = unsafe {
                    crate::registry()
                        .map(|r| !r.class_by_name(&key).is_null())
                        .unwrap_or(false)
                };
                if !exists {
                    return Ok(mlua::Value::Nil);
                }
                let cls = key.clone();
                let proxy = lua.create_table()?;
                proxy.set(
                    "new",
                    lua.create_function(move |lua, _a: mlua::Variadic<mlua::Value>| {
                        Ok(new_object_handle(lua, &cls))
                    })?,
                )?;
                Ok(mlua::Value::Table(proxy))
            })?,
        )?;
        lua.globals().set_metatable(Some(g_mt));
    }

    setup_hooks(lua)?; // Observe/ObserveAfter/Override/registerHotkey/registerInput — extraído
    // ---- construtores de tipo (mods passam como args pros métodos do jogo) ----
    lua.globals().set(
        "ToCName",
        lua.create_function(|lua, s: String| lua.create_userdata(CName(crate::cname::intern(&s))))?,
    )?;
    // CName no CET é uma TABELA CHAMÁVEL com .new/.add — NÃO só função. NativeSettings
    // faz `CName.add(label)` (registra o nome); mods fazem `CName.new("x")`/`CName("x")`.
    // Todas INTERNAM (string→hash) no nosso espelho pra NameToString reverter; a estrutura/
    // aba monta com o hash. (Registro no CNamePool nativo do jogo é refino futuro.)
    {
        let cname_tbl = lua.create_table()?;
        let ctor = lua.create_function(|lua, s: String| {
            lua.create_userdata(CName(crate::cname::intern(&s)))
        })?;
        cname_tbl.set("new", ctor.clone())?;
        cname_tbl.set("add", ctor.clone())?;
        let mt = lua.create_table()?;
        // CName("x") — __call recebe a própria tabela como 1º arg, descarta.
        mt.set("__call", lua.create_function(|lua, (_t, s): (mlua::Value, String)| {
            lua.create_userdata(CName(crate::cname::intern(&s)))
        })?)?;
        cname_tbl.set_metatable(Some(mt));
        lua.globals().set("CName", cname_tbl)?;
    }
    // Pré-interna strings que mods comparam contra `CName.value` (o engine passa só o hash;
    // o FNV1a64 é o mesmo, então name_of reverte). Crítico: AddMenuItem do NativeSettings faz
    // `spawnEvent.value == "OnSwitchToSettings"` → sem isso o botão "Mods" nunca é criado.
    for s in [
        "OnSwitchToSettings", "OnSwitchToCredits", "OnSwitchToGameplay", "Mods",
        "hold_input", "None", "click", "OnPressBack",
    ] {
        let _ = crate::cname::intern(s);
    }
    lua.globals().set(
        "ToTweakDBID",
        lua.create_function(|lua, s: String| {
            lua.create_userdata(TweakDBID(crate::cname::tweak_db_id(&s).to_le_bytes()))
        })?,
    )?;
    lua.globals().set("TweakDBID", lua.globals().get::<mlua::Function>("ToTweakDBID")?)?;
    lua.globals().set(
        "ItemID",
        lua.create_function(|lua, s: String| {
            let mut bytes = [0u8; 16];
            unsafe {
                with_engine(|r, _, _| {
                    if let Some(b) = crate::rtti::from_tdbid(r, &s) {
                        bytes = b;
                    }
                });
            }
            lua.create_userdata(ItemId(bytes))
        })?,
    )?;
    lua.globals().set(
        "Vector4",
        lua.create_function(|lua, (x, y, z, w): (f32, f32, Option<f32>, Option<f32>)| {
            lua.create_userdata(Vec4([x, y, z.unwrap_or(0.0), w.unwrap_or(1.0)]))
        })?,
    )?;
    lua.globals().set("Quaternion", lua.globals().get::<mlua::Function>("Vector4")?)?;
    lua.globals().set("EulerAngles", lua.globals().get::<mlua::Function>("Vector4")?)?;

    // ---- globals utilitários do CET ----
    // GetVersion DEVE casar o regex `^v(%d+)%.(%d+)%.(%d+)` do CET (psiberx) — mods fazem
    // gate de versão (ex.: NativeSettings: `tonumber(GetVersion():gsub(...)) < 1.25`). Uma
    // string fora do formato vira nil → `nil < 1.25` ESTOURA. Reportamos vX.Y.Z >= 1.25.
    lua.globals().set(
        "GetVersion",
        lua.create_function(|_, ()| Ok("v1.35.0".to_string()))?,
    )?;
    // StringToName(s) = ToCName (interna + devolve CName). NativeSettings usa em SetName/GetWidget.
    lua.globals().set("StringToName", lua.globals().get::<mlua::Function>("ToCName")?)?;
    // NameToString(n) → nome internado do CName/hash (espelho do CNamePool); "" se nunca visto.
    lua.globals().set(
        "NameToString",
        lua.create_function(|_, v: mlua::Value| {
            let h = match v {
                mlua::Value::UserData(ud) => ud.borrow::<CName>().map(|c| c.0).unwrap_or(0),
                mlua::Value::Integer(i) => i as u64,
                mlua::Value::Number(n) => n as u64,
                mlua::Value::String(s) => {
                    return Ok(s.to_str().map(|st| st.to_string()).unwrap_or_default())
                }
                _ => 0,
            };
            Ok(crate::cname::name_of(h).unwrap_or_default())
        })?,
    )?;
    // GetLocalizedText[ByKey](key) → STUB "" (sem bridge de localização). Os mods tratam
    // ""/len==0 com fallback (NativeSettings usa o label cru). NUNCA nil (nil:len() estoura).
    {
        let loc = lua.create_function(|_, _v: mlua::Value| Ok(String::new()))?;
        lua.globals().set("GetLocalizedTextByKey", loc.clone())?;
        lua.globals().set("GetLocalizedText", loc)?;
    }
    // BuildWidgetPath(segs) → devolve a própria lista de segmentos (consumida por
    // handle:GetWidgetByPath nos corpos dos hooks/Stage B). Não estoura no registro.
    lua.globals().set(
        "BuildWidgetPath",
        lua.create_function(|_, segs: mlua::Value| Ok(segs))?,
    )?;
    // CalcSeed(o) → identidade do ponteiro do Handle como int (o que o CET hasheia).
    lua.globals().set(
        "CalcSeed",
        lua.create_function(|_, v: mlua::Value| {
            Ok(match v {
                mlua::Value::UserData(ud) => ud.borrow::<Handle>().map(|h| h.0 as i64).unwrap_or(0),
                _ => 0,
            })
        })?,
    )?;
    lua.globals().set(
        "GetMod",
        lua.create_function(|lua, name: String| {
            // devolve a tabela que o mod retornou do init.lua (capturada por run_mod)
            match lua.globals().get::<mlua::Value>("__mods")? {
                mlua::Value::Table(t) => t.get::<mlua::Value>(name),
                _ => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    lua.globals().set(
        "ModArchiveExists",
        lua.create_function(|_, _name: String| Ok(false))?,
    )?;
    // TweakDB(): READ de flats via getters tipados (sem Variant). GetFloat/GetInt/GetBool.
    // Aceita o path do flat como string (ex: "Items.Preset_X.damage").
    lua.globals().set(
        "TweakDB",
        {
            // TweakDB é uma TABELA (CET-compat): `TweakDB:GetFlat(path)` / `:GetFloat(path)` direto.
            let t = lua.create_table()?;
            // Lê do bake offline (crate::tweakdb_bake) — valores reais, SEM chamar o jogo
            // (o getter in-game trava). tag: 1=Float 2=Int 3=Bool 4=CName 5=TweakDBID.
            t.set(
                "GetFloat",
                lua.create_function(|_, (_s, path): (mlua::Value, String)| {
                    let id = crate::cname::tweak_db_id(&path);
                    Ok(match crate::tweakdb_bake::lookup(id) {
                        Some((1, v)) => Some(f32::from_bits(v as u32)),
                        Some((2, v)) => Some(v as i64 as f32), // int onde o mod espera float
                        _ => None,
                    })
                })?,
            )?;
            t.set(
                "GetInt",
                lua.create_function(|_, (_s, path): (mlua::Value, String)| {
                    let id = crate::cname::tweak_db_id(&path);
                    Ok(match crate::tweakdb_bake::lookup(id) {
                        Some((2, v)) => Some(v as i64),
                        Some((1, v)) => Some(f32::from_bits(v as u32) as i64),
                        _ => None,
                    })
                })?,
            )?;
            t.set(
                "GetBool",
                lua.create_function(|_, (_s, path): (mlua::Value, String)| {
                    let id = crate::cname::tweak_db_id(&path);
                    Ok(match crate::tweakdb_bake::lookup(id) {
                        Some((3, v)) => Some(v != 0),
                        _ => None,
                    })
                })?,
            )?;
            t.set(
                "GetCName",
                lua.create_function(|lua, (_s, path): (mlua::Value, String)| {
                    let id = crate::cname::tweak_db_id(&path);
                    match crate::tweakdb_bake::lookup(id) {
                        Some((4, v)) => Ok(mlua::Value::UserData(lua.create_userdata(CName(v))?)),
                        _ => Ok(mlua::Value::Nil),
                    }
                })?,
            )?;
            t.set(
                "GetTweakDBID",
                lua.create_function(|lua, (_s, path): (mlua::Value, String)| {
                    let id = crate::cname::tweak_db_id(&path);
                    match crate::tweakdb_bake::lookup(id) {
                        Some((5, v)) => Ok(mlua::Value::UserData(
                            lua.create_userdata(TweakDBID(v.to_le_bytes()))?,
                        )),
                        _ => Ok(mlua::Value::Nil),
                    }
                })?,
            )?;
            // GetFlat: getter genérico estilo CET (auto-tipado) — TweakDB:GetFlat(path).
            t.set(
                "GetFlat",
                lua.create_function(|lua, (_s, path): (mlua::Value, String)| {
                    let id = crate::cname::tweak_db_id(&path);
                    Ok::<mlua::Value, mlua::Error>(match crate::tweakdb_bake::lookup(id) {
                        Some((1, v)) => mlua::Value::Number(f32::from_bits(v as u32) as f64),
                        Some((2, v)) => mlua::Value::Integer(v as i64),
                        Some((3, v)) => mlua::Value::Boolean(v != 0),
                        Some((4, v)) => mlua::Value::UserData(lua.create_userdata(CName(v))?),
                        Some((5, v)) => {
                            mlua::Value::UserData(lua.create_userdata(TweakDBID(v.to_le_bytes()))?)
                        }
                        _ => mlua::Value::Nil,
                    })
                })?,
            )?;
            t
        },
    )?;

    setup_imgui(lua)?; // ImGui (tabela + ~120 widgets + 467 enums) — extraído p/ clean code
    Ok(())
}

/// Chama um callback de hook (Observe/Override) passando `this` (o objeto em que o
/// método foi chamado) como lightuserdata. `try_lock`: se o Lua já está travado
/// (reentrância — um callback disparou outro método vigiado), PULA sem deadlock.
pub unsafe fn call_hook(key: &mlua::RegistryKey, this: *mut c_void, params: &[(u64, u64)]) {
    let g = match LUA.try_lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if let Some(s) = g.as_ref() {
        let lua = &s.0;
        if let Ok(f) = lua.registry_value::<mlua::Function>(key) {
            // monta (this, arg1, arg2, ...): this como Handle; args decodificados por tipo.
            let mut args: Vec<mlua::Value> = Vec::with_capacity(params.len() + 1);
            args.push(handle_val(lua, this));
            for &(raw, tc) in params {
                args.push(decode_arg(lua, raw, tc));
            }
            let mv: mlua::Variadic<mlua::Value> = args.into_iter().collect();
            let _: mlua::Result<()> = f.call(mv);
        }
    }
}

/// Override-WRAPPED (CET) FLAG-BASED: chama o callback com `(this, ...args, wrapped)`
/// (wrapped por ÚLTIMO, como o CET). A closure `wrapped` NÃO re-invoca a original (frame
/// sintético crashava métodos de UI); ela só seta `WRAPPED_CALLED` → o dispatch deixa a
/// ORIGINAL rodar no FRAME REAL. Se o callback não chamar wrapped (override total), a
/// original é suprimida. (Limitação: código DEPOIS de wrapped() roda antes da original e
/// wrapped() não devolve valor — ok p/ NativeSettings, que só faz `wrapped(idx)` no fim.)
fn handle_val(lua: &mlua::Lua, p: *mut c_void) -> mlua::Value {
    match lua.create_userdata(Handle(p, std::ptr::null_mut())) {
        Ok(ud) => mlua::Value::UserData(ud),
        Err(_) => mlua::Value::Nil,
    }
}

/// Ponteiro do Handle como usize p/ identidade (OperatorEqual): dois Handles comparam
/// o ponteiro real; nil↔nil = 0 (iguais); não-Handle = 1 (≠ qualquer ponteiro real).
fn handle_ptr(v: &mlua::Value) -> usize {
    match v {
        mlua::Value::UserData(ud) => ud.borrow::<Handle>().map(|h| h.0 as usize).unwrap_or(1),
        _ => 0,
    }
}

/// Decodifica um arg pelo CName do TIPO do param (float/bool/int/CName corretos);
/// se o tipo for desconhecido, cai na heurística (Handle se ponteiro são, senão int).
fn decode_arg(lua: &mlua::Lua, raw: u64, tcname: u64) -> mlua::Value {
    use crate::cname::cname;
    if tcname != 0 {
        if tcname == cname("Float") {
            return mlua::Value::Number(f32::from_bits(raw as u32) as f64);
        }
        if tcname == cname("Double") {
            return mlua::Value::Number(f64::from_bits(raw));
        }
        if tcname == cname("Bool") {
            return mlua::Value::Boolean(raw & 0xff != 0);
        }
        if tcname == cname("CName") {
            if let Ok(ud) = lua.create_userdata(CName(raw)) {
                return mlua::Value::UserData(ud);
            }
        }
        if tcname == cname("Int32")
            || tcname == cname("Int64")
            || tcname == cname("Uint32")
            || tcname == cname("Uint64")
            || tcname == cname("Int16")
            || tcname == cname("Uint8")
        {
            return mlua::Value::Integer(raw as i64);
        }
    }
    // heurística (Handle/inteiro) p/ tipos ricos (handle/struct) ou tipo desconhecido.
    if raw >= 0x1_0000_0000 && crate::rtti::sane(raw as *mut c_void) {
        handle_val(lua, raw as *mut c_void)
    } else {
        mlua::Value::Integer(raw as i64)
    }
}

/// Escreve um valor POD no buffer de retorno `res` com a LARGURA CORRETA, gateado pelo
/// TIPO DE RETORNO declarado da função (GetName + GetSize do `IType*` em func+0x18). Só
/// os fundamentais cuja largura sabemos casar exatamente; qualquer outra coisa
/// (classe/handle/string/array, ou um valor Lua que não bate com o tipo) → `false` =
/// NÃO é seguro suprimir a original. É isto que torna o Override-TOTAL seguro p/ funções
/// que RETORNAM valor: nunca deixa lixo no aOut (na dúvida, a original roda).
unsafe fn write_pod_ret(func: *mut c_void, res: *mut c_void, v: &mlua::Value) -> bool {
    if res.is_null() {
        return false;
    }
    let ty = crate::rtti::fn_ret_type(func);
    if ty.is_null() {
        return false; // void → tratado no caller (suppress sem escrever)
    }
    use crate::cname::cname;
    let tn = crate::rtti::type_name_getname(ty);
    let sz = crate::rtti::type_size(ty);
    let p = res as *mut u8;
    if tn == cname("Bool") && sz == 1 {
        if let mlua::Value::Boolean(b) = v {
            *p = u8::from(*b);
            return true;
        }
    } else if (tn == cname("Int8") || tn == cname("Uint8")) && sz == 1 {
        if let mlua::Value::Integer(i) = v {
            *p = *i as u8;
            return true;
        }
    } else if (tn == cname("Int16") || tn == cname("Uint16")) && sz == 2 {
        if let mlua::Value::Integer(i) = v {
            (p as *mut i16).write_unaligned(*i as i16);
            return true;
        }
    } else if (tn == cname("Int32") || tn == cname("Uint32")) && sz == 4 {
        if let mlua::Value::Integer(i) = v {
            (p as *mut i32).write_unaligned(*i as i32);
            return true;
        }
    } else if (tn == cname("Int64") || tn == cname("Uint64")) && sz == 8 {
        if let mlua::Value::Integer(i) = v {
            (p as *mut i64).write_unaligned(*i);
            return true;
        }
    } else if tn == cname("Float") && sz == 4 {
        if let mlua::Value::Number(n) = v {
            (p as *mut f32).write_unaligned(*n as f32);
            return true;
        }
    } else if tn == cname("Double") && sz == 8 {
        if let mlua::Value::Number(n) = v {
            (p as *mut f64).write_unaligned(*n);
            return true;
        }
    }
    false
}

/// Override unificado (semântica CET): roda o callback UMA vez com `(this, ...args,
/// wrapped)`, capturando se ele chamou `wrapped()` (→ `WRAPPED_CALLED`) E o valor de
/// retorno. Se for override-TOTAL (não chamou `wrapped`) e o retorno couber num POD de
/// largura conhecida, ESCREVE o `res` (aOut) e devolve `suppress_value=true` → o
/// executor pula a original (sem efeito colateral dela). Senão NÃO toca o `res` e
/// devolve `false` → a original roda (e o rewrite pós-original via `call_hook_ret`
/// segue cobrindo getters). Retorna `(wrapped, suppress_value)`.
pub unsafe fn call_hook_override(
    key: &mlua::RegistryKey,
    this: *mut c_void,
    params: &[(u64, u64)],
    func: *mut c_void,
    res: *mut c_void,
) -> (bool, bool) {
    let g = match LUA.try_lock() {
        Ok(g) => g,
        Err(_) => return (false, false),
    };
    if let Some(s) = g.as_ref() {
        let lua = &s.0;
        if let Ok(cb) = lua.registry_value::<mlua::Function>(key) {
            let wrapped = match lua.create_function(move |_, _a: mlua::Variadic<mlua::Value>| {
                crate::hooks::WRAPPED_CALLED.store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }) {
                Ok(w) => w,
                Err(_) => return (false, false),
            };
            // ordem CET: (this, ...args, wrapped).
            let mut args: Vec<mlua::Value> = Vec::with_capacity(params.len() + 2);
            args.push(handle_val(lua, this));
            for &(raw, tc) in params {
                args.push(decode_arg(lua, raw, tc));
            }
            args.push(mlua::Value::Function(wrapped));
            let mv: mlua::Variadic<mlua::Value> = args.into_iter().collect();
            let ret: mlua::Value = cb.call(mv).unwrap_or(mlua::Value::Nil);
            let wrapped_called =
                crate::hooks::WRAPPED_CALLED.load(std::sync::atomic::Ordering::Relaxed);
            // Só tenta suprimir-com-valor em override-TOTAL (sem wrapped) que devolveu algo.
            let suppress_value = if !wrapped_called && !matches!(ret, mlua::Value::Nil) {
                write_pod_ret(func, res, &ret)
            } else {
                false
            };
            return (wrapped_called, suppress_value);
        }
    }
    (false, false)
}

/// Override (return-rewrite): roda o callback passando `this` e, se ele retornar um
/// valor não-nil, ENCODA esse valor no buffer de retorno `res` (16B) — o caller do
/// método passa a ver o valor do mod. nil = não reescreve (mantém o original).
pub unsafe fn call_hook_ret(key: &mlua::RegistryKey, this: *mut c_void, res: *mut c_void) -> bool {
    let g = match LUA.try_lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    if let Some(s) = g.as_ref() {
        let lua = &s.0;
        if let Ok(f) = lua.registry_value::<mlua::Function>(key) {
            let this_val = match lua.create_userdata(Handle(this, std::ptr::null_mut())) {
                Ok(ud) => mlua::Value::UserData(ud),
                Err(_) => mlua::Value::Nil,
            };
            if let Ok(ret) = f.call::<mlua::Value>(this_val) {
                return encode_ret(res, &ret);
            }
        }
    }
    false
}

/// Encoda um valor Lua no buffer de retorno (16B) conforme o tipo do valor: bool→1B,
/// int→i64, float→f32, Handle→ptr. nil/outros = não reescreve.
unsafe fn encode_ret(res: *mut c_void, v: &mlua::Value) -> bool {
    if res.is_null() {
        return false;
    }
    let p = res as *mut u8;
    match v {
        mlua::Value::Boolean(b) => {
            *p = u8::from(*b);
            true
        }
        mlua::Value::Integer(i) => {
            (p as *mut i64).write_unaligned(*i);
            true
        }
        mlua::Value::Number(n) => {
            (p as *mut f32).write_unaligned(*n as f32);
            true
        }
        mlua::Value::UserData(ud) => {
            if let Ok(h) = ud.borrow::<Handle>() {
                (p as *mut *mut c_void).write_unaligned(h.0);
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Prelúdio Lua injetado em todo runtime: Cron (agendador) + json (encode/decode),
/// implementados em Lua puro (como o CET faz). Cron é tickado num onUpdate interno.
const PRELUDE_LUA: &str = r#"
Cron = { _t = {}, _id = 0 }
function Cron.After(d, fn) Cron._id=Cron._id+1; Cron._t[Cron._id]={at=os.clock()+d,fn=fn}; return Cron._id end
function Cron.Every(i, fn) Cron._id=Cron._id+1; Cron._t[Cron._id]={at=os.clock()+i,fn=fn,every=true,iv=i}; return Cron._id end
function Cron.NextTick(fn) Cron._id=Cron._id+1; Cron._t[Cron._id]={at=0,fn=fn}; return Cron._id end
function Cron.Halt(id) Cron._t[id]=nil end
function Cron.Pause(id) if Cron._t[id] then Cron._t[id].paused=true end end
function Cron.Resume(id) if Cron._t[id] then Cron._t[id].paused=false; Cron._t[id].at=os.clock()+(Cron._t[id].iv or 0) end end
function Cron.IsActive(id) return Cron._t[id]~=nil end
registerForEvent("onUpdate", function()
  local now=os.clock()
  for id,t in pairs(Cron._t) do
    if now>=t.at and not t.paused then local _ok,_err=pcall(t.fn); if not _ok then print("[Cron] erro: "..tostring(_err)) end; if t.every then t.at=now+t.iv else Cron._t[id]=nil end end
  end
end)

json = {}
local function enc(v)
  local tp=type(v)
  if v==nil then return 'null'
  elseif tp=='boolean' then return v and 'true' or 'false'
  elseif tp=='number' then return tostring(v)
  elseif tp=='string' then return '"'..v:gsub('["\\]','\\%0'):gsub('\n','\\n')..'"'
  elseif tp=='table' then
    local isarr=true; local n=0
    for k,_ in pairs(v) do n=n+1; if type(k)~='number' then isarr=false end end
    if isarr and n>0 then
      local p={}; for i=1,#v do p[i]=enc(v[i]) end; return '['..table.concat(p,',')..']'
    else
      local p={}; for k,val in pairs(v) do p[#p+1]='"'..tostring(k)..'":'..enc(val) end
      return '{'..table.concat(p,',')..'}'
    end
  end
  return 'null'
end
function json.encode(v) return enc(v) end
function json.decode(s)
  local i=1
  local function skip() while i<=#s and s:sub(i,i):match('%s') do i=i+1 end end
  local parse
  local function pstr() local r=''; i=i+1; while i<=#s do local c=s:sub(i,i); if c=='\\' then r=r..s:sub(i+1,i+1); i=i+2 elseif c=='"' then i=i+1; break else r=r..c; i=i+1 end end; return r end
  parse=function()
    skip(); local c=s:sub(i,i)
    if c=='"' then return pstr()
    elseif c=='{' then i=i+1; local o={}; skip(); if s:sub(i,i)=='}' then i=i+1; return o end
      while true do skip(); local k=pstr(); skip(); i=i+1; o[k]=parse(); skip(); local d=s:sub(i,i); i=i+1; if d=='}' then break end end; return o
    elseif c=='[' then i=i+1; local a={}; skip(); if s:sub(i,i)==']' then i=i+1; return a end
      while true do a[#a+1]=parse(); skip(); local d=s:sub(i,i); i=i+1; if d==']' then break end end; return a
    elseif c=='t' then i=i+4; return true
    elseif c=='f' then i=i+5; return false
    elseif c=='n' then i=i+4; return nil
    else local j=i; while i<=#s and s:sub(i,i):match('[%d%.%-eE+]') do i=i+1 end; return tonumber(s:sub(j,i-1)) end
  end
  local ok,r=pcall(parse); if ok then return r else return nil end
end
"#;

/// Auto-teste do Override-suppress (build de TESTE, gateado por `/tmp/bwms-ovtest`).
/// Espera um V vivo, arma Override→42 + ObserveAfter(sentinela), drena ~2s, chama 1× e
/// loga PASS/FAIL sozinho. Em uso normal (sem o marcador) nunca é injetado — zero efeito.
const OVTEST_LUA: &str = r#"
local done, armed, arm_at = false, false, 0
Cron.Every(1.0, function()
  if done then return end
  local p = Game.GetPlayer()
  if not p then return end
  if not armed then
    _G.__ovt_ran = false
    ObserveAfter("PlayerPuppet", "BwmsProbe", function(self) _G.__ovt_ran = true end)
    Override("PlayerPuppet", "BwmsProbe", function(self) return 42 end)
    _G.__ovt_before = p:BwmsProbeCalls()
    armed, arm_at = true, os.clock()
    print("[ov-test] V vivo: override armado, drenando ~2s...")
    return
  end
  if os.clock() - arm_at < 2.0 then return end
  local r = p:BwmsProbe()
  local after = p:BwmsProbeCalls()
  local marshaling = (r == 42)
  local suppressed = (after == _G.__ovt_before) and (_G.__ovt_ran == false)
  print(string.format("[ov-test] retorno=%s (esperado 42) | contador %s->%d | original_rodou=%s",
    tostring(r), tostring(_G.__ovt_before), after, tostring(_G.__ovt_ran)))
  if marshaling and suppressed then
    print("[ov-test] >>> OVERRIDE-SUPPRESS POD: OK <<<  (retorno reescrito + original suprimida)")
  elseif marshaling and not suppressed then
    print("[ov-test] PARCIAL: retorno 42 mas a ORIGINAL rodou (suppress falhou)")
  else
    print("[ov-test] FALHOU: retorno=" .. tostring(r))
  end
  done = true
end)
"#;

unsafe fn lock_init<'a>(
    g: &'a mut std::sync::MutexGuard<'_, Option<SendLua>>,
) -> Option<&'a mlua::Lua> {
    if g.is_none() {
        let lua = mlua::Lua::new();
        if let Err(e) = setup_api(&lua) {
            crate::log(&format!("[lua] setup ERRO: {e}"));
        }
        if let Err(e) = lua.load(PRELUDE_LUA).exec() {
            crate::log(&format!("[lua] prelude ERRO: {e}"));
        }
        // Build de TESTE: se o marcador existe, injeta o auto-teste do Override-suppress
        // (espera V vivo e roda sozinho). Sem o marcador, NADA disto roda.
        if std::path::Path::new("/tmp/bwms-ovtest").exists() {
            if let Err(e) = lua.load(OVTEST_LUA).exec() {
                crate::log(&format!("[lua] ovtest ERRO: {e}"));
            } else {
                crate::log("[ov-test] armado — carregue um save (espera um V vivo)");
            }
        }
        **g = Some(SendLua(lua));
        crate::log("[lua] runtime LuaJIT inicializado (Game.*, registerForEvent, ImGui, Cron, json)");
    }
    g.as_ref().map(|s| &s.0)
}

/// Descarta o estado Lua (próximo run_code re-inicializa limpo via setup_api). O
/// loadmod usa isto p/ recarregar um mod SEM duplicar callbacks (onDraw/onUpdate).
/// Não desliga cheats já aplicados no jogo (godmode/itens vivem na engine, não no Lua).
/// TODO 100%: mod-manager com isolamento por-mod (mods/<nome>/init.lua coexistindo).
pub unsafe fn reset() {
    let mut g = LUA.lock().unwrap_or_else(|e| e.into_inner());
    // dispara onShutdown ANTES de descartar (mods salvam estado/limpam) — paridade CET.
    if let Some(s) = g.as_ref() {
        let lua = &s.0;
        let _: mlua::Result<()> = (|| {
            let events: mlua::Table = lua.globals().get("__events")?;
            if let Some(list) = events.get::<Option<mlua::Table>>("onShutdown")? {
                for cb in list.sequence_values::<mlua::Function>().flatten() {
                    let _: mlua::Result<()> = cb.call(());
                }
            }
            Ok(())
        })();
    }
    *g = None;
}

/// Executa um trecho de código Lua no estado persistente.
pub unsafe fn run_code(code: &str) {
    let mut g = LUA.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(lua) = lock_init(&mut g) {
        if let Err(e) = lua.load(code).exec() {
            crate::log(&format!("[lua] ERRO: {e}"));
        }
    }
}

/// Roda o init.lua de um mod CAPTURANDO seu valor de retorno em `__mods[name]`
/// (pro `GetMod(name)` — interop entre mods, como no CET: o mod faz `return M`).
/// O chunk Lua já é uma função; chamá-la executa o init.lua e devolve o `return`.
pub unsafe fn run_mod(name: &str, code: &str, dir: &std::path::Path) {
    let mut g = LUA.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(lua) = lock_init(&mut g) {
        let r: mlua::Result<()> = (|| {
            // package.path → submódulos do mod: `require("Cron")` acha <dir>/Cron.lua
            // (como o CET resolve require na pasta do mod). Prepende a pasta deste mod.
            if let Ok(pkg) = lua.globals().get::<mlua::Table>("package") {
                let d = dir.to_string_lossy();
                let cur: String = pkg.get("path").unwrap_or_default();
                let _ = pkg.set("path", format!("{d}/?.lua;{d}/?/init.lua;{cur}"));
            }
            let f = lua.load(code).into_function()?;
            let ret: mlua::Value = f.call(())?; // executa + pega o return do init.lua
            let mods: mlua::Table = match lua.globals().get::<mlua::Value>("__mods")? {
                mlua::Value::Table(t) => t,
                _ => {
                    let t = lua.create_table()?;
                    lua.globals().set("__mods", &t)?;
                    t
                }
            };
            mods.set(name, ret)?;
            Ok(())
        })();
        if let Err(e) = r {
            crate::log(&format!("[lua] mod '{name}' ERRO: {e}"));
        }
    }
}

/// Último instante em que o onUpdate rodou — p/ computar o `delta` (segundos) que o CET passa
/// pro callback. Sem isso, mods que fazem `Cron.Update(delta)` (NativeSettings) erram todo frame
/// (`arithmetic on nil`). Wall-clock entre ticks = tempo real decorrido (o que o Cron espera).
static LAST_UPDATE: Mutex<Option<std::time::Instant>> = Mutex::new(None);

fn next_update_delta() -> f64 {
    let mut lu = LAST_UPDATE.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    let d = lu.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(1.0 / 60.0);
    *lu = Some(now);
    // clampa: após um load/pausa longo o delta seria gigante e dispararia todos os timers do
    // Cron de uma vez (e estouraria contas). 0.25s é teto generoso (~4 fps).
    d.clamp(0.0, 0.25)
}

/// Dispara um evento (onInit/onUpdate/...) → chama todos os callbacks registrados. Pro `onUpdate`
/// passa o `delta` (segundos desde o último), como o CET — alguns mods dependem dele.
pub unsafe fn run_event(name: &str) {
    let mut g = LUA.lock().unwrap_or_else(|e| e.into_inner());
    // não inicializa o runtime só pra rodar evento (evita custo se não há mod).
    if g.is_none() {
        return;
    }
    let delta = if name == "onUpdate" { next_update_delta() } else { 0.0 };
    if let Some(lua) = lock_init(&mut g) {
        let r: mlua::Result<()> = (|| {
            let events: mlua::Table = lua.globals().get("__events")?;
            if let Some(list) = events.get::<Option<mlua::Table>>(name)? {
                for cb in list.sequence_values::<mlua::Function>().flatten() {
                    // erro de UM callback NÃO pode abortar os outros mods — loga e segue.
                    let res = if name == "onUpdate" {
                        cb.call::<()>(delta)
                    } else {
                        cb.call::<()>(())
                    };
                    if let Err(e) = res {
                        crate::log(&format!("[lua] evento '{name}' callback ERRO: {e}"));
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            crate::log(&format!("[lua] evento '{name}' ERRO: {e}"));
        }
    }
}

/// Como run_event, mas TRY_LOCK — roda na thread de RENDER (present hook). Se o Lua
/// estiver travado pela thread do jogo, PULA o frame em vez de travar o render.
/// Só p/ "onDraw" (mods desenhando ImGui).
pub unsafe fn run_event_draw() {
    let mut g = match LUA.try_lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if g.is_none() {
        return;
    }
    if let Some(lua) = lock_init(&mut g) {
        let r: mlua::Result<()> = (|| {
            let events: mlua::Table = lua.globals().get("__events")?;
            if let Some(list) = events.get::<Option<mlua::Table>>("onDraw")? {
                for cb in list.sequence_values::<mlua::Function>().flatten() {
                    if let Err(e) = cb.call::<()>(()) {
                        crate::log(&format!("[lua] onDraw callback ERRO: {e}"));
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            crate::log(&format!("[lua] onDraw ERRO: {e}"));
        }
    }
}

// GERADO de imgui-sys-0.12 bindings.rs — tabelas de enum p/ o Lua (paridade CET).
fn setup_imgui_enums(lua: &mlua::Lua) -> mlua::Result<()> {
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("NoTitleBar", 1i64)?;
        t.set("NoResize", 2i64)?;
        t.set("NoMove", 4i64)?;
        t.set("NoScrollbar", 8i64)?;
        t.set("NoScrollWithMouse", 16i64)?;
        t.set("NoCollapse", 32i64)?;
        t.set("AlwaysAutoResize", 64i64)?;
        t.set("NoBackground", 128i64)?;
        t.set("NoSavedSettings", 256i64)?;
        t.set("NoMouseInputs", 512i64)?;
        t.set("MenuBar", 1024i64)?;
        t.set("HorizontalScrollbar", 2048i64)?;
        t.set("NoFocusOnAppearing", 4096i64)?;
        t.set("NoBringToFrontOnFocus", 8192i64)?;
        t.set("AlwaysVerticalScrollbar", 16384i64)?;
        t.set("AlwaysHorizontalScrollbar", 32768i64)?;
        t.set("NoNavInputs", 262144i64)?;
        t.set("NoNavFocus", 524288i64)?;
        t.set("UnsavedDocument", 1048576i64)?;
        t.set("NoNav", 786432i64)?;
        t.set("NoDecoration", 43i64)?;
        t.set("NoInputs", 786944i64)?;
        t.set("ChildWindow", 16777216i64)?;
        t.set("Tooltip", 33554432i64)?;
        t.set("Popup", 67108864i64)?;
        t.set("Modal", 134217728i64)?;
        t.set("ChildMenu", 268435456i64)?;
        lua.globals().set("ImGuiWindowFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("Text", 0i64)?;
        t.set("TextDisabled", 1i64)?;
        t.set("WindowBg", 2i64)?;
        t.set("ChildBg", 3i64)?;
        t.set("PopupBg", 4i64)?;
        t.set("Border", 5i64)?;
        t.set("BorderShadow", 6i64)?;
        t.set("FrameBg", 7i64)?;
        t.set("FrameBgHovered", 8i64)?;
        t.set("FrameBgActive", 9i64)?;
        t.set("TitleBg", 10i64)?;
        t.set("TitleBgActive", 11i64)?;
        t.set("TitleBgCollapsed", 12i64)?;
        t.set("MenuBarBg", 13i64)?;
        t.set("ScrollbarBg", 14i64)?;
        t.set("ScrollbarGrab", 15i64)?;
        t.set("ScrollbarGrabHovered", 16i64)?;
        t.set("ScrollbarGrabActive", 17i64)?;
        t.set("CheckMark", 18i64)?;
        t.set("SliderGrab", 19i64)?;
        t.set("SliderGrabActive", 20i64)?;
        t.set("Button", 21i64)?;
        t.set("ButtonHovered", 22i64)?;
        t.set("ButtonActive", 23i64)?;
        t.set("Header", 24i64)?;
        t.set("HeaderHovered", 25i64)?;
        t.set("HeaderActive", 26i64)?;
        t.set("Separator", 27i64)?;
        t.set("SeparatorHovered", 28i64)?;
        t.set("SeparatorActive", 29i64)?;
        t.set("ResizeGrip", 30i64)?;
        t.set("ResizeGripHovered", 31i64)?;
        t.set("ResizeGripActive", 32i64)?;
        t.set("Tab", 33i64)?;
        t.set("TabHovered", 34i64)?;
        t.set("TabActive", 35i64)?;
        t.set("TabUnfocused", 36i64)?;
        t.set("TabUnfocusedActive", 37i64)?;
        t.set("PlotLines", 38i64)?;
        t.set("PlotLinesHovered", 39i64)?;
        t.set("PlotHistogram", 40i64)?;
        t.set("PlotHistogramHovered", 41i64)?;
        t.set("TableHeaderBg", 42i64)?;
        t.set("TableBorderStrong", 43i64)?;
        t.set("TableBorderLight", 44i64)?;
        t.set("TableRowBg", 45i64)?;
        t.set("TableRowBgAlt", 46i64)?;
        t.set("TextSelectedBg", 47i64)?;
        t.set("DragDropTarget", 48i64)?;
        t.set("NavHighlight", 49i64)?;
        t.set("NavWindowingHighlight", 50i64)?;
        t.set("NavWindowingDimBg", 51i64)?;
        t.set("ModalWindowDimBg", 52i64)?;
        lua.globals().set("ImGuiCol", t)?; }
    { let t = lua.create_table()?;
        t.set("Alpha", 0i64)?;
        t.set("DisabledAlpha", 1i64)?;
        t.set("WindowRounding", 3i64)?;
        t.set("WindowBorderSize", 4i64)?;
        t.set("WindowMinSize", 5i64)?;
        t.set("WindowTitleAlign", 6i64)?;
        t.set("ChildRounding", 7i64)?;
        t.set("ChildBorderSize", 8i64)?;
        t.set("PopupRounding", 9i64)?;
        t.set("PopupBorderSize", 10i64)?;
        t.set("FrameRounding", 12i64)?;
        t.set("FrameBorderSize", 13i64)?;
        t.set("ItemSpacing", 14i64)?;
        t.set("ItemInnerSpacing", 15i64)?;
        t.set("IndentSpacing", 16i64)?;
        t.set("ScrollbarSize", 18i64)?;
        t.set("ScrollbarRounding", 19i64)?;
        t.set("GrabMinSize", 20i64)?;
        t.set("GrabRounding", 21i64)?;
        t.set("TabRounding", 22i64)?;
        t.set("ButtonTextAlign", 23i64)?;
        t.set("SelectableTextAlign", 24i64)?;
        lua.globals().set("ImGuiStyleVar", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("Always", 1i64)?;
        t.set("Once", 2i64)?;
        t.set("FirstUseEver", 4i64)?;
        t.set("Appearing", 8i64)?;
        lua.globals().set("ImGuiCond", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("Selected", 1i64)?;
        t.set("Framed", 2i64)?;
        t.set("AllowItemOverlap", 4i64)?;
        t.set("NoTreePushOnOpen", 8i64)?;
        t.set("NoAutoOpenOnLog", 16i64)?;
        t.set("DefaultOpen", 32i64)?;
        t.set("OpenOnDoubleClick", 64i64)?;
        t.set("OpenOnArrow", 128i64)?;
        t.set("Leaf", 256i64)?;
        t.set("Bullet", 512i64)?;
        t.set("SpanAvailWidth", 2048i64)?;
        t.set("SpanFullWidth", 4096i64)?;
        t.set("NavLeftJumpsBackHere", 8192i64)?;
        t.set("CollapsingHeader", 26i64)?;
        lua.globals().set("ImGuiTreeNodeFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("CharsDecimal", 1i64)?;
        t.set("CharsHexadecimal", 2i64)?;
        t.set("CharsUppercase", 4i64)?;
        t.set("CharsNoBlank", 8i64)?;
        t.set("AutoSelectAll", 16i64)?;
        t.set("EnterReturnsTrue", 32i64)?;
        t.set("CallbackCompletion", 64i64)?;
        t.set("CallbackHistory", 128i64)?;
        t.set("CallbackAlways", 256i64)?;
        t.set("CallbackCharFilter", 512i64)?;
        t.set("AllowTabInput", 1024i64)?;
        t.set("CtrlEnterForNewLine", 2048i64)?;
        t.set("NoHorizontalScroll", 4096i64)?;
        t.set("AlwaysOverwrite", 8192i64)?;
        t.set("ReadOnly", 16384i64)?;
        t.set("Password", 32768i64)?;
        t.set("NoUndoRedo", 65536i64)?;
        t.set("CharsScientific", 131072i64)?;
        t.set("CallbackResize", 262144i64)?;
        t.set("CallbackEdit", 524288i64)?;
        t.set("EscapeClearsAll", 1048576i64)?;
        lua.globals().set("ImGuiInputTextFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("Resizable", 1i64)?;
        t.set("Reorderable", 2i64)?;
        t.set("Hideable", 4i64)?;
        t.set("Sortable", 8i64)?;
        t.set("NoSavedSettings", 16i64)?;
        t.set("ContextMenuInBody", 32i64)?;
        t.set("RowBg", 64i64)?;
        t.set("BordersInnerH", 128i64)?;
        t.set("BordersOuterH", 256i64)?;
        t.set("BordersInnerV", 512i64)?;
        t.set("BordersOuterV", 1024i64)?;
        t.set("BordersH", 384i64)?;
        t.set("BordersV", 1536i64)?;
        t.set("BordersInner", 640i64)?;
        t.set("BordersOuter", 1280i64)?;
        t.set("Borders", 1920i64)?;
        t.set("NoBordersInBody", 2048i64)?;
        t.set("NoBordersInBodyUntilResize", 4096i64)?;
        t.set("SizingFixedFit", 8192i64)?;
        t.set("SizingFixedSame", 16384i64)?;
        t.set("SizingStretchProp", 24576i64)?;
        t.set("SizingStretchSame", 32768i64)?;
        t.set("NoHostExtendX", 65536i64)?;
        t.set("NoHostExtendY", 131072i64)?;
        t.set("NoKeepColumnsVisible", 262144i64)?;
        t.set("PreciseWidths", 524288i64)?;
        t.set("NoClip", 1048576i64)?;
        t.set("PadOuterX", 2097152i64)?;
        t.set("NoPadOuterX", 4194304i64)?;
        t.set("NoPadInnerX", 8388608i64)?;
        t.set("ScrollX", 16777216i64)?;
        t.set("ScrollY", 33554432i64)?;
        t.set("SortMulti", 67108864i64)?;
        t.set("SortTristate", 134217728i64)?;
        lua.globals().set("ImGuiTableFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("Disabled", 1i64)?;
        t.set("DefaultHide", 2i64)?;
        t.set("DefaultSort", 4i64)?;
        t.set("WidthStretch", 8i64)?;
        t.set("WidthFixed", 16i64)?;
        t.set("NoResize", 32i64)?;
        t.set("NoReorder", 64i64)?;
        t.set("NoHide", 128i64)?;
        t.set("NoClip", 256i64)?;
        t.set("NoSort", 512i64)?;
        t.set("NoSortAscending", 1024i64)?;
        t.set("NoSortDescending", 2048i64)?;
        t.set("NoHeaderLabel", 4096i64)?;
        t.set("NoHeaderWidth", 8192i64)?;
        t.set("PreferSortAscending", 16384i64)?;
        t.set("PreferSortDescending", 32768i64)?;
        t.set("IndentEnable", 65536i64)?;
        t.set("IndentDisable", 131072i64)?;
        t.set("IsEnabled", 16777216i64)?;
        t.set("IsVisible", 33554432i64)?;
        t.set("IsSorted", 67108864i64)?;
        t.set("IsHovered", 134217728i64)?;
        lua.globals().set("ImGuiTableColumnFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("Headers", 1i64)?;
        lua.globals().set("ImGuiTableRowFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("RowBg0", 1i64)?;
        t.set("RowBg1", 2i64)?;
        t.set("CellBg", 3i64)?;
        lua.globals().set("ImGuiTableBgTarget", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("DontClosePopups", 1i64)?;
        t.set("SpanAllColumns", 2i64)?;
        t.set("AllowDoubleClick", 4i64)?;
        t.set("Disabled", 8i64)?;
        t.set("AllowItemOverlap", 16i64)?;
        lua.globals().set("ImGuiSelectableFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("Reorderable", 1i64)?;
        t.set("AutoSelectNewTabs", 2i64)?;
        t.set("TabListPopupButton", 4i64)?;
        t.set("NoCloseWithMiddleMouseButton", 8i64)?;
        t.set("NoTabListScrollingButtons", 16i64)?;
        t.set("NoTooltip", 32i64)?;
        t.set("FittingPolicyResizeDown", 64i64)?;
        t.set("FittingPolicyScroll", 128i64)?;
        lua.globals().set("ImGuiTabBarFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("UnsavedDocument", 1i64)?;
        t.set("SetSelected", 2i64)?;
        t.set("NoCloseWithMiddleMouseButton", 4i64)?;
        t.set("NoPushId", 8i64)?;
        t.set("NoTooltip", 16i64)?;
        t.set("NoReorder", 32i64)?;
        t.set("Leading", 64i64)?;
        t.set("Trailing", 128i64)?;
        lua.globals().set("ImGuiTabItemFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("PopupAlignLeft", 1i64)?;
        t.set("HeightSmall", 2i64)?;
        t.set("HeightRegular", 4i64)?;
        t.set("HeightLarge", 8i64)?;
        t.set("HeightLargest", 16i64)?;
        t.set("NoArrowButton", 32i64)?;
        t.set("NoPreview", 64i64)?;
        lua.globals().set("ImGuiComboFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("ChildWindows", 1i64)?;
        t.set("RootWindow", 2i64)?;
        t.set("AnyWindow", 4i64)?;
        t.set("NoPopupHierarchy", 8i64)?;
        t.set("AllowWhenBlockedByPopup", 32i64)?;
        t.set("AllowWhenBlockedByActiveItem", 128i64)?;
        t.set("AllowWhenOverlapped", 256i64)?;
        t.set("AllowWhenDisabled", 512i64)?;
        t.set("NoNavOverride", 1024i64)?;
        t.set("RectOnly", 416i64)?;
        t.set("RootAndChildWindows", 3i64)?;
        t.set("DelayNormal", 2048i64)?;
        t.set("DelayShort", 4096i64)?;
        t.set("NoSharedDelay", 8192i64)?;
        lua.globals().set("ImGuiHoveredFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("ChildWindows", 1i64)?;
        t.set("RootWindow", 2i64)?;
        t.set("AnyWindow", 4i64)?;
        t.set("NoPopupHierarchy", 8i64)?;
        t.set("RootAndChildWindows", 3i64)?;
        lua.globals().set("ImGuiFocusedFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", -1i64)?;
        t.set("Left", 0i64)?;
        t.set("Right", 1i64)?;
        t.set("Up", 2i64)?;
        t.set("Down", 3i64)?;
        lua.globals().set("ImGuiDir", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("AlwaysClamp", 16i64)?;
        t.set("Logarithmic", 32i64)?;
        t.set("NoRoundToFormat", 64i64)?;
        t.set("NoInput", 128i64)?;
        lua.globals().set("ImGuiSliderFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("NoAlpha", 2i64)?;
        t.set("NoPicker", 4i64)?;
        t.set("NoOptions", 8i64)?;
        t.set("NoSmallPreview", 16i64)?;
        t.set("NoInputs", 32i64)?;
        t.set("NoTooltip", 64i64)?;
        t.set("NoLabel", 128i64)?;
        t.set("NoSidePreview", 256i64)?;
        t.set("NoDragDrop", 512i64)?;
        t.set("NoBorder", 1024i64)?;
        t.set("AlphaBar", 65536i64)?;
        t.set("AlphaPreview", 131072i64)?;
        t.set("AlphaPreviewHalf", 262144i64)?;
        t.set("HDR", 524288i64)?;
        t.set("DisplayRGB", 1048576i64)?;
        t.set("DisplayHSV", 2097152i64)?;
        t.set("DisplayHex", 4194304i64)?;
        t.set("Uint8", 8388608i64)?;
        t.set("Float", 16777216i64)?;
        t.set("PickerHueBar", 33554432i64)?;
        t.set("PickerHueWheel", 67108864i64)?;
        t.set("InputRGB", 134217728i64)?;
        t.set("InputHSV", 268435456i64)?;
        lua.globals().set("ImGuiColorEditFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("Left", 0i64)?;
        t.set("Right", 1i64)?;
        t.set("Middle", 2i64)?;
        lua.globals().set("ImGuiMouseButton", t)?; }
    { let t = lua.create_table()?;
        t.set("None", -1i64)?;
        t.set("Arrow", 0i64)?;
        t.set("TextInput", 1i64)?;
        t.set("ResizeAll", 2i64)?;
        t.set("ResizeNS", 3i64)?;
        t.set("ResizeEW", 4i64)?;
        t.set("ResizeNESW", 5i64)?;
        t.set("ResizeNWSE", 6i64)?;
        t.set("Hand", 7i64)?;
        t.set("NotAllowed", 8i64)?;
        lua.globals().set("ImGuiMouseCursor", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("MouseButtonLeft", 0i64)?;
        t.set("MouseButtonRight", 1i64)?;
        t.set("MouseButtonMiddle", 2i64)?;
        t.set("NoOpenOverExistingPopup", 32i64)?;
        t.set("NoOpenOverItems", 64i64)?;
        t.set("AnyPopupId", 128i64)?;
        t.set("AnyPopupLevel", 256i64)?;
        t.set("AnyPopup", 384i64)?;
        lua.globals().set("ImGuiPopupFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("Closed", 1i64)?;
        t.set("RoundCornersTopLeft", 16i64)?;
        t.set("RoundCornersTopRight", 32i64)?;
        t.set("RoundCornersBottomLeft", 64i64)?;
        t.set("RoundCornersBottomRight", 128i64)?;
        t.set("RoundCornersNone", 256i64)?;
        t.set("RoundCornersTop", 48i64)?;
        t.set("RoundCornersBottom", 192i64)?;
        t.set("RoundCornersLeft", 80i64)?;
        t.set("RoundCornersRight", 160i64)?;
        t.set("RoundCornersAll", 240i64)?;
        lua.globals().set("ImDrawFlags", t)?; }
    { let t = lua.create_table()?;
        t.set("None", 0i64)?;
        t.set("Tab", 512i64)?;
        t.set("LeftArrow", 513i64)?;
        t.set("RightArrow", 514i64)?;
        t.set("UpArrow", 515i64)?;
        t.set("DownArrow", 516i64)?;
        t.set("PageUp", 517i64)?;
        t.set("PageDown", 518i64)?;
        t.set("Home", 519i64)?;
        t.set("End", 520i64)?;
        t.set("Insert", 521i64)?;
        t.set("Delete", 522i64)?;
        t.set("Backspace", 523i64)?;
        t.set("Space", 524i64)?;
        t.set("Enter", 525i64)?;
        t.set("Escape", 526i64)?;
        t.set("LeftCtrl", 527i64)?;
        t.set("LeftShift", 528i64)?;
        t.set("LeftAlt", 529i64)?;
        t.set("LeftSuper", 530i64)?;
        t.set("RightCtrl", 531i64)?;
        t.set("RightShift", 532i64)?;
        t.set("RightAlt", 533i64)?;
        t.set("RightSuper", 534i64)?;
        t.set("Menu", 535i64)?;
        t.set("0", 536i64)?;
        t.set("1", 537i64)?;
        t.set("2", 538i64)?;
        t.set("3", 539i64)?;
        t.set("4", 540i64)?;
        t.set("5", 541i64)?;
        t.set("6", 542i64)?;
        t.set("7", 543i64)?;
        t.set("8", 544i64)?;
        t.set("9", 545i64)?;
        t.set("A", 546i64)?;
        t.set("B", 547i64)?;
        t.set("C", 548i64)?;
        t.set("D", 549i64)?;
        t.set("E", 550i64)?;
        t.set("F", 551i64)?;
        t.set("G", 552i64)?;
        t.set("H", 553i64)?;
        t.set("I", 554i64)?;
        t.set("J", 555i64)?;
        t.set("K", 556i64)?;
        t.set("L", 557i64)?;
        t.set("M", 558i64)?;
        t.set("N", 559i64)?;
        t.set("O", 560i64)?;
        t.set("P", 561i64)?;
        t.set("Q", 562i64)?;
        t.set("R", 563i64)?;
        t.set("S", 564i64)?;
        t.set("T", 565i64)?;
        t.set("U", 566i64)?;
        t.set("V", 567i64)?;
        t.set("W", 568i64)?;
        t.set("X", 569i64)?;
        t.set("Y", 570i64)?;
        t.set("Z", 571i64)?;
        t.set("F1", 572i64)?;
        t.set("F2", 573i64)?;
        t.set("F3", 574i64)?;
        t.set("F4", 575i64)?;
        t.set("F5", 576i64)?;
        t.set("F6", 577i64)?;
        t.set("F7", 578i64)?;
        t.set("F8", 579i64)?;
        t.set("F9", 580i64)?;
        t.set("F10", 581i64)?;
        t.set("F11", 582i64)?;
        t.set("F12", 583i64)?;
        t.set("Apostrophe", 584i64)?;
        t.set("Comma", 585i64)?;
        t.set("Minus", 586i64)?;
        t.set("Period", 587i64)?;
        t.set("Slash", 588i64)?;
        t.set("Semicolon", 589i64)?;
        t.set("Equal", 590i64)?;
        t.set("LeftBracket", 591i64)?;
        t.set("Backslash", 592i64)?;
        t.set("RightBracket", 593i64)?;
        t.set("GraveAccent", 594i64)?;
        t.set("CapsLock", 595i64)?;
        t.set("ScrollLock", 596i64)?;
        t.set("NumLock", 597i64)?;
        t.set("PrintScreen", 598i64)?;
        t.set("Pause", 599i64)?;
        t.set("Keypad0", 600i64)?;
        t.set("Keypad1", 601i64)?;
        t.set("Keypad2", 602i64)?;
        t.set("Keypad3", 603i64)?;
        t.set("Keypad4", 604i64)?;
        t.set("Keypad5", 605i64)?;
        t.set("Keypad6", 606i64)?;
        t.set("Keypad7", 607i64)?;
        t.set("Keypad8", 608i64)?;
        t.set("Keypad9", 609i64)?;
        t.set("KeypadDecimal", 610i64)?;
        t.set("KeypadDivide", 611i64)?;
        t.set("KeypadMultiply", 612i64)?;
        t.set("KeypadSubtract", 613i64)?;
        t.set("KeypadAdd", 614i64)?;
        t.set("KeypadEnter", 615i64)?;
        t.set("KeypadEqual", 616i64)?;
        t.set("GamepadStart", 617i64)?;
        t.set("GamepadBack", 618i64)?;
        t.set("GamepadFaceLeft", 619i64)?;
        t.set("GamepadFaceRight", 620i64)?;
        t.set("GamepadFaceUp", 621i64)?;
        t.set("GamepadFaceDown", 622i64)?;
        t.set("GamepadDpadLeft", 623i64)?;
        t.set("GamepadDpadRight", 624i64)?;
        t.set("GamepadDpadUp", 625i64)?;
        t.set("GamepadDpadDown", 626i64)?;
        t.set("GamepadL1", 627i64)?;
        t.set("GamepadR1", 628i64)?;
        t.set("GamepadL2", 629i64)?;
        t.set("GamepadR2", 630i64)?;
        t.set("GamepadL3", 631i64)?;
        t.set("GamepadR3", 632i64)?;
        t.set("GamepadLStickLeft", 633i64)?;
        t.set("GamepadLStickRight", 634i64)?;
        t.set("GamepadLStickUp", 635i64)?;
        t.set("GamepadLStickDown", 636i64)?;
        t.set("GamepadRStickLeft", 637i64)?;
        t.set("GamepadRStickRight", 638i64)?;
        t.set("GamepadRStickUp", 639i64)?;
        t.set("GamepadRStickDown", 640i64)?;
        t.set("MouseLeft", 641i64)?;
        t.set("MouseRight", 642i64)?;
        t.set("MouseMiddle", 643i64)?;
        t.set("MouseX1", 644i64)?;
        t.set("MouseX2", 645i64)?;
        t.set("MouseWheelX", 646i64)?;
        t.set("MouseWheelY", 647i64)?;
        lua.globals().set("ImGuiKey", t)?; }
    Ok(())
}

/// Testes OFFLINE do marshaling de retorno do Override-suppress (`write_pod_ret`).
/// Não tocam no jogo: montam um `IType` + vtable SINTÉTICOS (GetName@+0x10,
/// GetSize@+0x18) e um descritor de função (`func+0x18` = ret-type), e provam que
/// `write_pod_ret` (a) escreve a LARGURA CORRETA por tipo POD e (b) RECUSA (não
/// suprime) em qualquer dúvida (não-POD, tipo/valor incompatível, largura errada,
/// res null, void). `cname` é FNV puro e `is_readable` lê o próprio processo → tudo
/// roda em `cargo test --features lua`, sem o jogo.
#[cfg(test)]
mod ret_marshal_tests {
    use super::*;
    use std::ffi::c_void;

    // GetSize@vtable+0x18 (por largura).
    extern "C" fn gs1(_t: *mut c_void) -> u32 { 1 }
    extern "C" fn gs2(_t: *mut c_void) -> u32 { 2 }
    extern "C" fn gs4(_t: *mut c_void) -> u32 { 4 }
    extern "C" fn gs8(_t: *mut c_void) -> u32 { 8 }
    // GetName@vtable+0x10 → CName do tipo (FNV puro, offline).
    extern "C" fn gn_bool(_t: *mut c_void) -> u64 { crate::cname::cname("Bool") }
    extern "C" fn gn_i32(_t: *mut c_void) -> u64 { crate::cname::cname("Int32") }
    extern "C" fn gn_i64(_t: *mut c_void) -> u64 { crate::cname::cname("Int64") }
    extern "C" fn gn_f32(_t: *mut c_void) -> u64 { crate::cname::cname("Float") }
    extern "C" fn gn_f64(_t: *mut c_void) -> u64 { crate::cname::cname("Double") }
    extern "C" fn gn_class(_t: *mut c_void) -> u64 { crate::cname::cname("PlayerPuppet") } // não-POD

    /// Monta `func(+0x18) -> IType(->vtable[GetName@0x10, GetSize@0x18])` e chama write_pod_ret.
    unsafe fn run(
        gn: extern "C" fn(*mut c_void) -> u64,
        gs: extern "C" fn(*mut c_void) -> u32,
        v: &mlua::Value,
    ) -> (bool, [u8; 16]) {
        // vtable: 4 slots; +0x10 (idx2)=GetName, +0x18 (idx3)=GetSize.
        let vtable: [usize; 4] = [0, 0, gn as usize, gs as usize];
        // IType: 3 ponteiros (>=0x18 legível); [0] = &vtable.
        let ity: [usize; 3] = [vtable.as_ptr() as usize, 0, 0];
        // func: 0x20 bytes; +0x18 = &ity.
        let mut func = [0u8; 0x20];
        (func.as_mut_ptr().add(0x18) as *mut usize).write_unaligned(ity.as_ptr() as usize);
        let mut res = [0u8; 16];
        let ok = write_pod_ret(
            func.as_mut_ptr() as *mut c_void,
            res.as_mut_ptr() as *mut c_void,
            v,
        );
        (ok, res)
    }

    // ---- POD: escreve a largura correta, sem tocar nos bytes acima ----

    #[test]
    fn int32_4bytes() {
        unsafe {
            let (ok, r) = run(gn_i32, gs4, &mlua::Value::Integer(0x1234_5678_i64));
            assert!(ok);
            assert_eq!(u32::from_le_bytes([r[0], r[1], r[2], r[3]]), 0x1234_5678);
            assert_eq!(&r[4..], &[0u8; 12], "bytes acima da largura NÃO tocados");
        }
    }
    #[test]
    fn bool_1byte() {
        unsafe {
            let (ok, r) = run(gn_bool, gs1, &mlua::Value::Boolean(true));
            assert!(ok);
            assert_eq!(r[0], 1);
            assert_eq!(&r[1..], &[0u8; 15]);
        }
    }
    #[test]
    fn float_4bytes() {
        unsafe {
            let (ok, r) = run(gn_f32, gs4, &mlua::Value::Number(1.5));
            assert!(ok);
            assert_eq!(f32::from_le_bytes([r[0], r[1], r[2], r[3]]), 1.5);
            assert_eq!(&r[4..], &[0u8; 12]);
        }
    }
    #[test]
    fn int64_8bytes() {
        unsafe {
            let (ok, r) = run(gn_i64, gs8, &mlua::Value::Integer(0x0102_0304_0506_0708_i64));
            assert!(ok);
            assert_eq!(u64::from_le_bytes(r[0..8].try_into().unwrap()), 0x0102_0304_0506_0708);
            assert_eq!(&r[8..], &[0u8; 8]);
        }
    }
    #[test]
    fn double_8bytes() {
        unsafe {
            let (ok, r) = run(gn_f64, gs8, &mlua::Value::Number(2.5));
            assert!(ok);
            assert_eq!(f64::from_le_bytes(r[0..8].try_into().unwrap()), 2.5);
        }
    }

    // ---- SEGURANÇA: qualquer dúvida → false (não suprime → original roda) ----

    #[test]
    fn nonpod_class_refused() {
        unsafe {
            let (ok, r) = run(gn_class, gs8, &mlua::Value::Integer(5));
            assert!(!ok, "tipo NÃO-POD nunca suprime");
            assert_eq!(&r[..], &[0u8; 16], "nada escrito no res");
        }
    }
    #[test]
    fn type_value_mismatch_refused() {
        unsafe {
            // tipo Int32 mas valor Boolean → não casa → false
            let (ok, _) = run(gn_i32, gs4, &mlua::Value::Boolean(true));
            assert!(!ok);
        }
    }
    #[test]
    fn size_mismatch_refused() {
        unsafe {
            // nome Int32 mas size 8 (largura não bate) → false
            let (ok, _) = run(gn_i32, gs8, &mlua::Value::Integer(1));
            assert!(!ok);
        }
    }
    #[test]
    fn null_res_refused() {
        unsafe {
            let mut func = [0u8; 0x20];
            let ity: [usize; 3] = [0, 0, 0];
            (func.as_mut_ptr().add(0x18) as *mut usize).write_unaligned(ity.as_ptr() as usize);
            let ok = write_pod_ret(
                func.as_mut_ptr() as *mut c_void,
                std::ptr::null_mut(),
                &mlua::Value::Integer(1),
            );
            assert!(!ok, "res null nunca suprime");
        }
    }
    #[test]
    fn void_ret_refused() {
        unsafe {
            // func+0x18 = 0 (void) → fn_ret_type null → false (suppress de void é tratado à parte)
            let mut func = [0u8; 0x20];
            let mut res = [0u8; 16];
            let ok = write_pod_ret(
                func.as_mut_ptr() as *mut c_void,
                res.as_mut_ptr() as *mut c_void,
                &mlua::Value::Integer(1),
            );
            assert!(!ok);
        }
    }
}
