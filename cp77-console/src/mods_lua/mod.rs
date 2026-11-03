//! Mods Lua no Mac: os `.lua` rodam no `bwms-lua`. Um estado global único (serializado por Mutex), `Game.*` ligado ao nosso motor RTTI,
//! `registerForEvent(onInit/onUpdate/...)` e o lifecycle dirigido pelo `cp77_tick` (thread do jogo). Mod que depende de `.dll` não roda aqui.

use std::ffi::c_void;
use std::sync::Mutex;

use crate::rtti::Registry;

pub(crate) mod banco;
pub(crate) mod interface;
pub(crate) mod ponte;
pub(crate) mod observar;

/// Resposta do `GetVersion()`: o nível da API Lua que os mods conferem, não a versão do BWMS. Mod compara com esse texto para saber o que
/// pode chamar.
const NIVEL_DA_API_LUA: &str = "v1.38.1";

struct SendLua(bwms_lua::Lua);
unsafe impl Send for SendLua {}
static LUA: Mutex<Option<SendLua>> = Mutex::new(None);
/// Chave do `io` completo no metatable dos globais (protegido por `__metatable`): só `ambiente_do_mod` o lê, para montar o `io` preso à
/// pasta de cada mod. Fica no próprio estado, então cada estado tem o seu.
const CHAVE_DO_IO_COMPLETO: &str = "__io_completo";
/// Callbacks de hotkey (registerHotkey), keyed pela string da tecla.
static HOTKEYS: Mutex<Option<std::collections::HashMap<String, bwms_lua::RegistryKey>>> =
    Mutex::new(None);

/// Callbacks de registerInput (down/up), keyed pelo char da tecla. O cb recebe um
/// bool `isDown` — dispara tanto no key-down quanto no key-up (o que os mods esperam).
static INPUTS: Mutex<Option<std::collections::HashMap<char, bwms_lua::RegistryKey>>> =
    Mutex::new(None);

/// Dispara o callback de hotkey da tecla `c` (chamado pelo cp77_tick, thread do jogo).
pub unsafe fn fire_hotkey(c: char) {
    let g = match LUA.try_lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if let Some(s) = g.as_ref() {
        let lua = &s.0;
        let ids: Vec<String> =
            TECLAS_DE_HOTKEY.lock().map(|g| g.iter().filter(|(t, _)| *t == c).map(|(_, id)| id.clone()).collect()).unwrap_or_default();
        for id in ids {
            chamar_hotkey(lua, &id);
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
                if let Ok(f) = lua.registry_value::<bwms_lua::Function>(rk) {
                    if let Err(e) = f.call::<()>(down) {
                        erro_de_mod("registerInput", &e);
                    }
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

impl bwms_lua::UserData for Handle {
    fn add_methods<M: bwms_lua::UserDataMethods<Self>>(methods: &mut M) {
        // __index: PRIMEIRO tenta PROPRIEDADE (campo por nome via RTTI) — `obj.label`,
        // `spawnEvent.value`, etc.; se não for campo, devolve proxy de MÉTODO `obj:Metodo()`.
        // Campo vence método, a ordem que os mods esperam.
        methods.add_meta_method(bwms_lua::MetaMethod::Index, |lua, this, key: String| {
            let ptr = this.0;
            let known = this.1;
            unsafe {
                let cls = classe_do_objeto(ptr, known);
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
            let f = lua.create_function(move |lua, args: bwms_lua::Variadic<bwms_lua::Value>| {
                // `h:Method(a,b)` => args = [h, a, b]; pula o self.
                let real: Vec<bwms_lua::Value> = args.into_iter().skip(1).collect();
                unsafe { call_method(lua, ptr, known, &key, &real) }
            })?;
            Ok(bwms_lua::Value::Function(f))
        });
        // __newindex: escreve um CAMPO por nome — `data.label = "Mods"`, `.action = ...`.
        methods.add_meta_method(
            bwms_lua::MetaMethod::NewIndex,
            |_, this, (key, val): (String, bwms_lua::Value)| {
                let ptr = this.0;
                let known = this.1;
                unsafe {
                    let cls = classe_do_objeto(ptr, known);
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
        methods.add_meta_method(bwms_lua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!("Handle(0x{:x})", this.0 as usize))
        });
    }
}

/// Debug curto de um Value pro trace.
fn lua_dbg(v: &bwms_lua::Value) -> String {
    match v {
        bwms_lua::Value::Nil => "nil".into(),
        bwms_lua::Value::Boolean(b) => format!("{b}"),
        bwms_lua::Value::Integer(i) => format!("{i}"),
        bwms_lua::Value::Number(n) => format!("{n}"),
        bwms_lua::Value::String(s) => {
            format!("'{}'", s.to_str().map(|x| x.to_string()).unwrap_or_default())
        }
        bwms_lua::Value::UserData(ud) => {
            if let Ok(h) = ud.borrow::<Handle>() {
                unsafe {
                    let p = h.0;
                    let readable = crate::memoria::is_readable(p as *const c_void, 8);
                    let vt = if readable { (p as *const usize).read_unaligned() } else { 0 };
                    let gt = if vt != 0 && crate::memoria::is_readable(vt as *const c_void, 0x10) {
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
    if !crate::memoria::is_readable(slot as *const c_void, 0x10) {
        return std::ptr::null_mut();
    }
    let instance = (slot as *const *mut c_void).read_unaligned();
    let refcnt = (slot.add(0x08) as *const *mut c_void).read_unaligned();
    if instance.is_null() || refcnt.is_null() {
        return std::ptr::null_mut();
    }
    if !crate::memoria::is_readable(refcnt as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    let strong = (refcnt as *const u32).read_unaligned(); // RefCnt.strongRefs @ +0
    if strong == 0 || !crate::rtti::sane(instance) {
        return std::ptr::null_mut();
    }
    instance
}

unsafe fn read_field(
    lua: &bwms_lua::Lua,
    obj: *mut c_void,
    off: u32,
    ty: *mut c_void,
    in_holder: bool,
) -> bwms_lua::Value {
    // ponteiro final respeitando inValueHolder (props de scripted moram no valueHolder@0x38).
    let f = crate::rtti::field_ptr(obj, off, in_holder) as *mut u8;
    if f.is_null() || !crate::memoria::is_readable(f as *const c_void, 8) {
        return bwms_lua::Value::Nil;
    }
    // Pelo tipo do campo, como o retorno de método: struct (`gamebbScriptID_Bool` do blackboard) volta inteira.
    ponte::ler_valor(lua, f, ty).unwrap_or(bwms_lua::Value::Nil)
}

/// Escreve o campo (obj+off) marshalando o Value pelo tipo. Só POD/CName/enum/String-inline
/// e ponteiros — campos de obj recém-Construct (zerados). gum-checked.
unsafe fn write_field(
    obj: *mut c_void,
    off: u32,
    ty: *mut c_void,
    in_holder: bool,
    val: &bwms_lua::Value,
) {
    let f = crate::rtti::field_ptr(obj, off, in_holder) as *mut u8;
    if f.is_null() || !crate::memoria::is_readable(f as *const c_void, 8) {
        return;
    }
    let tn = crate::rtti::type_name_getname(ty);
    match val {
        bwms_lua::Value::Boolean(b) => *f = u8::from(*b),
        bwms_lua::Value::Integer(i) => match crate::rtti::type_size(ty) {
            1 => *f = *i as u8,
            2 => (f as *mut u16).write_unaligned(*i as u16),
            8 => (f as *mut u64).write_unaligned(*i as u64),
            _ => (f as *mut u32).write_unaligned(*i as u32),
        },
        bwms_lua::Value::Number(n) => (f as *mut f32).write_unaligned(*n as f32),
        bwms_lua::Value::String(s) => {
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
        bwms_lua::Value::UserData(ud) => {
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
/// `GetClasses` sem abstratas inclui a própria classe quando ela é concreta. A resposta fica guardada por classe: a lista vem do motor inteira.
unsafe fn classe_abstrata(r: &Registry, cls: *mut c_void) -> bool {
    static CONHECIDAS: Mutex<Option<std::collections::HashMap<usize, bool>>> = Mutex::new(None);
    if cls.is_null() {
        return false;
    }
    let mut g = CONHECIDAS.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(Default::default);
    *m.entry(cls as usize).or_insert_with(|| {
        let (lista, n) = r.get_classes(cls, false);
        !(0..n).any(|i| (lista as *const *mut c_void).add(i).read() == cls)
    })
}

fn new_object_handle(lua: &bwms_lua::Lua, class_name: &str) -> bwms_lua::Value {
    let mut out: *mut c_void = std::ptr::null_mut();
    let mut cls: *mut c_void = std::ptr::null_mut();
    unsafe {
        if let Some(r) = crate::registry() {
            cls = r.class_by_name(class_name); // CLASSE conhecida (GetType daria a base)
            // API 1.38: classe abstrata não constrói, `.new()` devolve nil.
            if !classe_abstrata(r, cls) {
                out = crate::rtti::new_object(r, class_name);
            }
        }
    }
    if out.is_null() {
        bwms_lua::Value::Nil
    } else {
        lua.create_userdata(Handle(out, cls))
            .map(bwms_lua::Value::UserData)
            .unwrap_or(bwms_lua::Value::Nil)
    }
}

/// Tipos do jogo expostos ao Lua como userdata, p/ marshalling correto nos métodos:
/// CName(hash u64), TweakDBID(8B), ItemID(16B). Mods passam esses como args.
#[derive(Clone, Copy)]
pub struct CName(pub u64);
impl bwms_lua::UserData for CName {
    fn add_methods<M: bwms_lua::UserDataMethods<Self>>(m: &mut M) {
        // O texto que os mods esperam: eles recortam o nome entre `--[[ ` e ` --]]`.
        m.add_meta_method(bwms_lua::MetaMethod::ToString, |_, t, ()| {
            let (lo, hi) = (t.0 as u32, (t.0 >> 32) as u32);
            Ok(match nome_do_cname(t.0) {
                Some(n) => format!("ToCName{{ hash_lo = 0x{lo:08X}, hash_hi = 0x{hi:08X} --[[ {n} --]] }}"),
                None => format!("ToCName{{ hash_lo = 0x{lo:08X}, hash_hi = 0x{hi:08X} }}"),
            })
        });
        // `cname.value` é a STRING do nome (há mod que faz `.value == "..."`,
        // `.value:gsub(...)`, `.value:find(...)`). Resolve via pool interno (name_of); se
        // desconhecido, devolve o hex do hash — SEMPRE string, pra os métodos de string não
        // quebrarem. `.hash` = o número. (Resolver CName→nome geral = CNamePool, em RE.)
        m.add_meta_method(bwms_lua::MetaMethod::Index, |lua, t, k: String| {
            Ok(match k.as_str() {
                "value" => {
                    let s = nome_do_cname(t.0).unwrap_or_else(|| format!("{:016x}", t.0));
                    crate::trace(&format!("CName.value -> '{s}' (hash {:#018x})", t.0));
                    bwms_lua::Value::String(lua.create_string(s.as_bytes())?)
                }
                "hash" => bwms_lua::Value::Integer(t.0 as i64),
                _ => bwms_lua::Value::Nil,
            })
        });
    }
}
#[derive(Clone, Copy)]
pub struct TweakDBID(pub [u8; 8]);
impl bwms_lua::UserData for TweakDBID {
    fn add_methods<M: bwms_lua::UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(bwms_lua::MetaMethod::ToString, |_, t, ()| {
            let id = u64::from_le_bytes(t.0);
            Ok(format!("ToTweakDBID{{ hash = 0x{:08X}, length = {} }}", id as u32, (id >> 32) as u8))
        });
    }
}
#[derive(Clone, Copy)]
pub struct ItemId(pub [u8; 16]);
impl bwms_lua::UserData for ItemId {
    fn add_methods<M: bwms_lua::UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(bwms_lua::MetaMethod::ToString, |_, t, ()| {
            Ok(format!("ItemID({:02x?})", &t.0[..8]))
        });
    }
}
/// Vetor de 4 floats (Vector4/Quaternion/EulerAngles) = 16B → Arg::Raw.
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
impl bwms_lua::UserData for Vec4 {
    fn add_fields<F: bwms_lua::UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("x", |_, t| Ok(t.0[0]));
        f.add_field_method_get("y", |_, t| Ok(t.0[1]));
        f.add_field_method_get("z", |_, t| Ok(t.0[2]));
        f.add_field_method_get("w", |_, t| Ok(t.0[3]));
        // Mods fazem `v.x = 5` — setters por componente.
        f.add_field_method_set("x", |_, t, v: f32| { t.0[0] = v; Ok(()) });
        f.add_field_method_set("y", |_, t, v: f32| { t.0[1] = v; Ok(()) });
        f.add_field_method_set("z", |_, t, v: f32| { t.0[2] = v; Ok(()) });
        f.add_field_method_set("w", |_, t, v: f32| { t.0[3] = v; Ok(()) });
    }
    fn add_methods<M: bwms_lua::UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(bwms_lua::MetaMethod::ToString, |_, t, ()| {
            Ok(format!("Vec4({},{},{},{})", t.0[0], t.0[1], t.0[2], t.0[3]))
        });
        // math 3D (xyz) do Vector4 (teleport/distância)
        m.add_method("Length", |_, t, ()| {
            Ok((t.0[0] * t.0[0] + t.0[1] * t.0[1] + t.0[2] * t.0[2]).sqrt())
        });
        m.add_method("LengthSquared", |_, t, ()| {
            Ok(t.0[0] * t.0[0] + t.0[1] * t.0[1] + t.0[2] * t.0[2])
        });
        m.add_method("Distance", |_, t, o: bwms_lua::AnyUserData| {
            let b = o.borrow::<Vec4>()?;
            let (dx, dy, dz) = (t.0[0] - b.0[0], t.0[1] - b.0[1], t.0[2] - b.0[2]);
            Ok((dx * dx + dy * dy + dz * dz).sqrt())
        });
        m.add_method("Dot", |_, t, o: bwms_lua::AnyUserData| {
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
        // aritmética componente-a-componente (mods somam/subtraem Vector4)
        m.add_meta_method(bwms_lua::MetaMethod::Add, |lua, t, o: bwms_lua::AnyUserData| {
            let b = o.borrow::<Vec4>()?;
            lua.create_userdata(Vec4([
                t.0[0] + b.0[0], t.0[1] + b.0[1], t.0[2] + b.0[2], t.0[3] + b.0[3],
            ]))
        });
        m.add_meta_method(bwms_lua::MetaMethod::Sub, |lua, t, o: bwms_lua::AnyUserData| {
            let b = o.borrow::<Vec4>()?;
            lua.create_userdata(Vec4([
                t.0[0] - b.0[0], t.0[1] - b.0[1], t.0[2] - b.0[2], t.0[3] - b.0[3],
            ]))
        });
    }
}

/// Classe para achar método e campo de `obj`: a real quando ela deriva de `known` (retorno `ref<entIComponent>` que é uma câmera: `Activate`
/// só existe na classe real); senão `known` (objeto construído: o `class_of` devolve a base para classe de script).
unsafe fn classe_do_objeto(obj: *mut c_void, known: *mut c_void) -> *mut c_void {
    let real = crate::rtti::class_of(obj);
    if known.is_null() {
        return real;
    }
    let mut c = real;
    for _ in 0..64 {
        if !crate::rtti::sane(c) || !crate::memoria::is_readable(c as *const c_void, 0x18) {
            break;
        }
        if c == known {
            return real;
        }
        c = ((c as *const u8).add(0x10) as *const *mut c_void).read_unaligned(); // CClass+0x10 = pai
    }
    known
}

/// Chama `method` em `obj` via RTTI, marshalando args escalares do Lua → Arg. `known` =
/// classe conhecida (declarada no retorno, ou de objeto construído) ou null → usa class_of.
unsafe fn call_method(
    lua: &bwms_lua::Lua,
    obj: *mut c_void,
    known: *mut c_void,
    method: &str,
    args: &[bwms_lua::Value],
) -> bwms_lua::Result<bwms_lua::Value> {
    if obj.is_null() || crate::registry().is_none() {
        return Ok(bwms_lua::Value::Nil);
    }
    let cls = classe_do_objeto(obj, known);
    let rf = match crate::rtti::resolve_in_class(cls, method) {
        Some(r) => r,
        None => {
            let cn = if cls.is_null() {
                "<null>".to_string()
            } else {
                crate::cname::resolve_cname(crate::rtti::type_name_getname(cls))
            };
            // Método que não existe devolve `nil` sem derrubar o callback, mas conta como erro do mod: mudo, ele esconde a ação que não aconteceu.
            erro_de_mod(&format!("método {method}"), &format!("não existe na classe {cn}"));
            return Ok(bwms_lua::Value::Nil);
        }
    };
    let mut margs: Vec<crate::rtti::Arg> = Vec::with_capacity(args.len());
    for (ai, v) in args.iter().enumerate() {
        // O tipo do parâmetro decide (struct por valor, Float com número inteiro, enum por nome). Valor que não serve para o tipo é erro do mod:
        // mandar outra largura ao motor é lixo no parâmetro.
        if let Some(a) = ponte::argumento(rf.func, ai, v) {
            margs.push(a);
            continue;
        }
        match v {
            bwms_lua::Value::Table(t) => {
                // table (sequência de strings/CName) → DynArray<CName> = inkWidgetPath, p/
                // `widget:GetWidgetByPath(BuildWidgetPath({...}))`. Sem isto o path ia descartado
                // → GetWidgetByPath devolvia wref null → toda a cadeia de widget falhava.
                let mut cns: Vec<u64> = Vec::new();
                for v in t.clone().sequence_values::<bwms_lua::Value>().flatten() {
                    match v {
                        bwms_lua::Value::String(s) => {
                            if let Ok(st) = s.to_str() {
                                cns.push(crate::cname::cname(&st));
                            }
                        }
                        bwms_lua::Value::UserData(ud) => {
                            if let Ok(c) = ud.borrow::<CName>() {
                                cns.push(c.0);
                            }
                        }
                        _ => {}
                    }
                }
                if !cns.is_empty() {
                    margs.push(crate::rtti::Arg::Raw(lista_de_cnames(&cns)));
                }
            }
            _ => return Err(bwms_lua::Error::RuntimeError(format!("{method}: argumento {} não serve para o tipo do parâmetro", ai + 1))),
        }
    }
    match crate::rtti::call_func_valor(&rf, obj, &margs) {
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
                        if crate::rtti::sane(p) && crate::memoria::is_readable(p as *const c_void, 8) {
                            p
                        } else {
                            std::ptr::null_mut()
                        }
                    }
                };
                if inst.is_null() {
                    return Ok(bwms_lua::Value::Nil);
                }
                let inner = crate::rtti::inner_type(ret_ty);
                return Ok(bwms_lua::Value::UserData(lua.create_userdata(Handle(inst, inner))?));
            }
            // Pelo tipo de retorno: struct por valor (`ItemID`, `Vector4`, `Matrix`) volta inteira; `nil` para void e para o que a ponte não converte.
            ponte::ler_valor(lua, b.as_ptr(), ret_ty)
        }
        None => Ok(bwms_lua::Value::Nil),
    }
}


fn setup_game(lua: &bwms_lua::Lua) -> bwms_lua::Result<()> {
    let g = lua.create_table()?;
    g.set(
        "AddToInventory",
        lua.create_function(|_, (name, qty): (String, Option<i32>)| {
            let q = qty.unwrap_or(1).max(1) as u32;
            unsafe { with_engine(|r, p, t| { crate::roupas_e_armas::give(r, p, t, &name, q); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "AddMoney",
        lua.create_function(|_, n: i32| {
            unsafe { with_engine(|r, p, t| { crate::roupas_e_armas::give(r, p, t, "Items.money", n.max(1) as u32); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "AddPerkPoints",
        lua.create_function(|_, n: i32| {
            unsafe { with_engine(|r, p, _| { crate::status_do_personagem::add_points(r, p, n.max(1) as u32, "Primary"); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "AddAttributePoints",
        lua.create_function(|_, n: i32| {
            unsafe { with_engine(|r, p, _| { crate::status_do_personagem::add_points(r, p, n.max(1) as u32, "Attribute"); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "AddRelicPoints",
        lua.create_function(|_, n: i32| {
            unsafe { with_engine(|r, p, _| { crate::status_do_personagem::add_points(r, p, n.max(1) as u32, "Espionage"); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "SetLevel",
        lua.create_function(|_, n: i32| {
            unsafe { with_engine(|r, p, _| { crate::status_do_personagem::level(r, p, n.max(1) as u32); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "Heal",
        lua.create_function(|_, ()| {
            unsafe { with_engine(|r, p, _| { crate::status_do_personagem::heal(r, p); }) };
            Ok(())
        })?,
    )?;
    g.set(
        "GodMode",
        lua.create_function(|_, on: Option<bool>| {
            let on = on.unwrap_or(true);
            unsafe { with_engine(|r, p, _| { crate::status_do_personagem::godmode(r, p, on); }) };
            Ok(())
        })?,
    )?;
    // Game.GetPlayer(): o V capturado; sem ele, o `GetPlayer(GameInstance)` do jogo.
    // Game.Wrap(addr) → Handle de um ponteiro cru (p/ encadear retornos de getter). handle:Method(...) resolve.
    g.set(
        "GetPlayer",
        lua.create_function(|lua, ()| {
            // O V capturado não chama o motor e vale em qualquer thread (o `onDraw` roda na de render); sem ele, o do jogo.
            let p = crate::current_player();
            if p.is_null() {
                return match unsafe { ponte::funcao_do_jogo("GetPlayer") } {
                    Some(rf) => unsafe { ponte::chamar(lua, &rf, std::ptr::null_mut(), &[]) },
                    None => Ok(bwms_lua::Value::Nil),
                };
            }
            Ok(bwms_lua::Value::UserData(lua.create_userdata(Handle(p, std::ptr::null_mut()))?))
        })?,
    )?;
    g.set(
        "Wrap",
        lua.create_function(|lua, addr: i64| {
            Ok(bwms_lua::Value::UserData(
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
                with_engine(|r, p, _| sysptr = crate::jogo::sessao::sistema_de_script_do_jogador(r, p, &name));
            }
            if sysptr.is_null() {
                Ok(bwms_lua::Value::Nil)
            } else {
                Ok(bwms_lua::Value::UserData(lua.create_userdata(Handle(sysptr, std::ptr::null_mut()))?))
            }
        })?,
    )?;

    // Aliases naturais: o usuário pensa em "money/heal/god" (os atalhos do console),
    // então `Game.money(7777)` funciona igual `Game.AddMoney(7777)`.
    if let Ok(f) = g.get::<bwms_lua::Function>("AddMoney") {
        g.set("money", f.clone())?;
        g.set("Money", f)?;
    }
    if let Ok(f) = g.get::<bwms_lua::Function>("Heal") {
        g.set("heal", f)?;
    }
    if let Ok(f) = g.get::<bwms_lua::Function>("GodMode") {
        g.set("god", f.clone())?;
        g.set("godmode", f)?;
    }
    // Metatable: acessar um campo inexistente de Game.* loga uma DICA clara (com as
    // funções válidas) em vez de só estourar "attempt to call a nil value" cru.
    let mt = lua.create_table()?;
    mt.set(
        "__index",
        lua.create_function(|lua, (_t, k): (bwms_lua::Table, String)| {
            // overloads mangled (ex.: "OperatorEqual;IScriptableIScriptable;Bool") — os mods
            // chamam como função estática. Minimamente: identidade de ponteiro entre Handles
            // (== isSameInstance / Ref.Equals). Cobre getOptionTable.
            if k.starts_with("OperatorEqual") || k.starts_with("OperatorNotEqual") {
                let neg = k.starts_with("OperatorNotEqual");
                return Ok(bwms_lua::Value::Function(lua.create_function(
                    move |_, (a, b): (bwms_lua::Value, bwms_lua::Value)| {
                        let eq = handle_ptr(&a) == handle_ptr(&b);
                        Ok(if neg { !eq } else { eq })
                    },
                )?));
            }
            // Função global do jogo ou getter estático do `ScriptGameInstance` (`Game.GetSettingsSystem()`), com o `GameInstance` posto sozinho.
            match unsafe { ponte::funcao_do_jogo(&k) } {
                Some(rf) => Ok(bwms_lua::Value::Function(lua.create_function(move |lua, args: bwms_lua::MultiValue| {
                    let args: Vec<bwms_lua::Value> = args.into_iter().collect();
                    unsafe { ponte::chamar(lua, &rf, std::ptr::null_mut(), &args) }
                })?)),
                None => {
                    crate::log(&format!("[lua] Game.{k} não existe no jogo"));
                    Ok(bwms_lua::Value::Nil)
                }
            }
        })?,
    )?;
    g.set_metatable(Some(mt));
    lua.globals().set("Game", g)?;
    Ok(())
}

fn setup_hooks(lua: &bwms_lua::Lua) -> bwms_lua::Result<()> {
    // ---- Observe / ObserveAfter / Override: o coração da camada Lua ----
    // Registram um callback p/ quando o jogo chamar <classe>.<método>. O registro
    // é em 2 tempos (queue agora, resolve no cp77_tick na thread do jogo).
    use crate::mods_lua::observar::{queue, Kind};
    lua.globals().set(
        "Observe",
        lua.create_function(|lua, (cls, m, cb): (String, String, bwms_lua::Function)| {
            let key = lua.create_registry_value(cb)?;
            queue(cls, m, Kind::Before, key);
            Ok(())
        })?,
    )?;
    lua.globals().set(
        "ObserveAfter",
        lua.create_function(|lua, (cls, m, cb): (String, String, bwms_lua::Function)| {
            let key = lua.create_registry_value(cb)?;
            queue(cls, m, Kind::After, key);
            Ok(())
        })?,
    )?;
    lua.globals().set(
        "Override",
        lua.create_function(|lua, (cls, m, cb): (String, String, bwms_lua::Function)| {
            let key = lua.create_registry_value(cb)?;
            queue(cls, m, Kind::Override, key);
            Ok(())
        })?,
    )?;
    // Suppress(classe, método, cb): roda o cb NO LUGAR da original (a original é PULADA).
    // É o Override-suppress — pra métodos com efeito colateral (não só getter).
    lua.globals().set(
        "Suppress",
        lua.create_function(|lua, (cls, m, cb): (String, String, bwms_lua::Function)| {
            let key = lua.create_registry_value(cb)?;
            queue(cls, m, Kind::Suppress, key);
            Ok(())
        })?,
    )?;

    // registerHotkey(id, rótulo, cb) e registerInput(id, rótulo, cb(isDown)), a assinatura que os mods usam. O bind
    // vem do `bwms.toml` (`tecla_<id> = "j"`); sem bind o mod carrega e a tecla fica muda. A forma curta `(tecla, cb)` usa o id como tecla.
    lua.globals().set(
        "registerHotkey",
        lua.create_function(|lua, (id, a, b): (String, bwms_lua::Value, bwms_lua::Value)| {
            let (tecla, cb) = bind_e_callback(&id, a, b)?;
            if let Some(c) = tecla {
                hotkey_register_char(c, &id);
            }
            let rk = lua.create_registry_value(cb)?;
            HOTKEYS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(id, rk);
            Ok(())
        })?,
    )?;
    lua.globals().set(
        "registerInput",
        lua.create_function(|lua, (id, a, b): (String, bwms_lua::Value, bwms_lua::Value)| {
            let (tecla, cb) = bind_e_callback(&id, a, b)?;
            let Some(c) = tecla else { return Ok(()) };
            input_register_char(c);
            let rk = lua.create_registry_value(cb)?;
            INPUTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(c, rk);
            Ok(())
        })?,
    )?;

    Ok(())
}

/// Tecla e callback de um `registerHotkey`/`registerInput`: `(id, rótulo, cb)` com o bind do `bwms.toml`, ou `(tecla, cb)`; tecla `None` sem bind
/// (o hotkey fica guardado pelo id e só não tem tecla).
fn bind_e_callback(id: &str, a: bwms_lua::Value, b: bwms_lua::Value) -> bwms_lua::Result<(Option<char>, bwms_lua::Function)> {
    let (cb, curta) = match (a, b) {
        (_, bwms_lua::Value::Function(f)) => (f, false),
        (bwms_lua::Value::Function(f), _) => (f, true),
        _ => return Err(bwms_lua::Error::RuntimeError(format!("{id}: callback ausente"))),
    };
    let bind = crate::configuracao::do_mod::ler_texto(&format!("tecla_{id}")).or_else(|| curta.then(|| id.to_string()));
    let tecla = bind.and_then(|t| t.chars().next());
    if tecla.is_none() {
        crate::log(&format!("[lua] {id} sem tecla: ponha tecla_{id} = \"<tecla>\" no bwms.toml"));
    }
    Ok((tecla, cb))
}

fn setup_api(lua: &bwms_lua::Lua) -> bwms_lua::Result<()> {
    setup_game(lua)?; // proxy Game.* + cheats — extraído

    // registerForEvent("onInit"/"onUpdate"/..., fn) → guarda numa tabela global.
    lua.globals().set("__events", lua.create_table()?)?;
    lua.globals().set(
        "registerForEvent",
        lua.create_function(|lua, (ev, cb): (String, bwms_lua::Function)| {
            let events: bwms_lua::Table = lua.globals().get("__events")?;
            let list: bwms_lua::Table = match events.get::<Option<bwms_lua::Table>>(ev.as_str())? {
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

    // ---- Fase 1: utils + construtores faltantes ----
    // IsDefined(h): true se h é Handle não-null (null-check essencial dos mods Lua).
    lua.globals().set(
        "IsDefined",
        lua.create_function(|_, v: bwms_lua::Value| {
            Ok(match v {
                bwms_lua::Value::UserData(ud) => ud.borrow::<Handle>().map(|h| !h.0.is_null()).unwrap_or(true),
                bwms_lua::Value::Nil => false,
                _ => true,
            })
        })?,
    )?;
    // To* (`ToVector4{x=…}`, `ToEulerAngles{roll=…}`, `ToQuaternion{i=…}`, `ToVector3`): a struct do RTTI montada pelos campos. Os
    // tipos em si (`Vector4.new(…)`) vêm do global por tipo do RTTI (`setup_api`).
    for (global, tipo) in [("ToVector4", "Vector4"), ("ToVector3", "Vector3"), ("ToEulerAngles", "EulerAngles"), ("ToQuaternion", "Quaternion")] {
        lua.globals().set(
            global,
            lua.create_function(move |lua, t: bwms_lua::Table| unsafe {
                match ponte::tipo_por_nome(tipo).and_then(|ty| ponte::nova_struct(ty.0, &[bwms_lua::Value::Table(t)])) {
                    Some(s) => Ok(bwms_lua::Value::UserData(lua.create_userdata(s)?)),
                    None => Ok(bwms_lua::Value::Nil),
                }
            })?,
        )?;
    }
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
    lua.globals().set("EnumInt", lua.create_function(|_, v: bwms_lua::Value| {
        Ok(match v {
            bwms_lua::Value::Integer(i) => Some(i),
            bwms_lua::Value::Number(n) => Some(n as i64),
            _ => None,
        })
    })?)?;
    // Proxies de enum NOMEADOS: `PauseMenuAction.OpenSubMenu` → valor int via RTTI.
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
            lua.create_function(move |_, (_t, member): (bwms_lua::Table, String)| {
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
        let (w, h) = crate::terminal::overlay::tamanho_do_quadro();
        Ok((w as i64, h as i64))
    })?)?;
    // NewObject(className) → Handle de instância nova. DESGATEADO: Construct@0x40 (offsets
    // macOS) validado in-game (`newobj Vector4`→ptr OK) + alloc do POOL DO RED (não corrompe
    // no free do engine). Nil se a classe não resolver.
    lua.globals().set("NewObject", lua.create_function(|lua, class_name: String| {
        Ok(new_object_handle(lua, &class_name))
    })?)?;
    // _G metatable: um GLOBAL indefinido que SEJA uma classe RTTI registrada
    // vira um proxy chamável com `.new()` → constrói a instância. Ex.: `PauseMenuListItemData
    // .new()`, `inkTextWidget.new()`. Só dispara p/ chave faltante E classe existente → não
    // muda o resto (nomes desconhecidos seguem nil). É o que destrava os widgets dos mods de menu de opções.
    {
        let g_mt = lua.create_table()?;
        g_mt.set(
            "__index",
            lua.create_function(|lua, (_g, key): (bwms_lua::Table, String)| {
                let exists = unsafe {
                    crate::registry()
                        .map(|r| !r.class_by_name(&key).is_null())
                        .unwrap_or(false)
                };
                if !exists {
                    return Ok(bwms_lua::Value::Nil);
                }
                let cls = key.clone();
                let proxy = lua.create_table()?;
                proxy.set(
                    "new",
                    lua.create_function(move |lua, _a: bwms_lua::Variadic<bwms_lua::Value>| {
                        Ok(new_object_handle(lua, &cls))
                    })?,
                )?;
                Ok(bwms_lua::Value::Table(proxy))
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
    // CName é uma TABELA CHAMÁVEL com .new/.add — NÃO só função. Há mod que
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
        mt.set("__call", lua.create_function(|lua, (_t, s): (bwms_lua::Value, String)| {
            lua.create_userdata(CName(crate::cname::intern(&s)))
        })?)?;
        cname_tbl.set_metatable(Some(mt));
        lua.globals().set("CName", cname_tbl)?;
    }
    // Pré-interna strings que mods comparam contra `CName.value` (o engine passa só o hash;
    // o FNV1a64 é o mesmo, então name_of reverte). Crítico: o AddMenuItem de um mod de menu de opções faz
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
            lua.create_userdata(TweakDBID(bwms_primitives::tweak_db_id(&s).to_le_bytes()))
        })?,
    )?;
    // TweakDBID: `TweakDBID.new(path)`, `TweakDBID.new(base, ".sufixo")` (base = path ou TweakDBID) e `TweakDBID(path)`.
    {
        fn id_de(v: &bwms_lua::Value) -> Option<u64> {
            match v {
                bwms_lua::Value::String(s) => s.to_str().ok().map(|s| bwms_primitives::tweak_db_id(&s)),
                bwms_lua::Value::UserData(ud) => ud.borrow::<TweakDBID>().ok().map(|t| u64::from_le_bytes(t.0)),
                _ => None,
            }
        }
        let novo = lua.create_function(|lua, (base, sufixo): (bwms_lua::Value, Option<String>)| {
            let id = id_de(&base).ok_or_else(|| bwms_lua::Error::RuntimeError("TweakDBID.new: path ou TweakDBID".into()))?;
            let id = sufixo.map_or(id, |s| bwms_primitives::tweak_db_id_derive(id, &s));
            lua.create_userdata(TweakDBID(id.to_le_bytes()))
        })?;
        let t = lua.create_table()?;
        t.set("new", novo.clone())?;
        let mt = lua.create_table()?;
        let chamar = novo.clone();
        mt.set("__call", lua.create_function(move |_, (_t, base, sufixo): (bwms_lua::Value, bwms_lua::Value, Option<String>)| chamar.call::<bwms_lua::Value>((base, sufixo)))?)?;
        t.set_metatable(Some(mt));
        lua.globals().set("TweakDBID", t)?;
    }
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

    // ---- globals utilitários ----
    // GetVersion DEVE casar o regex `^v(%d+)%.(%d+)%.(%d+)` — mods fazem
    // gate de versão (ex.: `tonumber(GetVersion():gsub(...)) < 1.25`). Uma
    // string fora do formato vira nil → `nil < 1.25` ESTOURA. Reportamos vX.Y.Z >= 1.25.
    lua.globals().set(
        "GetVersion",
        lua.create_function(|_, ()| Ok(NIVEL_DA_API_LUA.to_string()))?,
    )?;
    setup_globais_de_tipo(lua)?;
    // StringToName(s) = ToCName (interna + devolve CName). Mods usam em SetName/GetWidget.
    lua.globals().set("StringToName", lua.globals().get::<bwms_lua::Function>("ToCName")?)?;
    // NameToString(n) → nome internado do CName/hash (espelho do CNamePool); "" se nunca visto.
    lua.globals().set(
        "NameToString",
        lua.create_function(|_, v: bwms_lua::Value| {
            let h = match v {
                bwms_lua::Value::UserData(ud) => ud.borrow::<CName>().map(|c| c.0).unwrap_or(0),
                bwms_lua::Value::Integer(i) => i as u64,
                bwms_lua::Value::Number(n) => n as u64,
                bwms_lua::Value::String(s) => {
                    return Ok(s.to_str().map(|st| st.to_string()).unwrap_or_default())
                }
                _ => 0,
            };
            Ok(crate::cname::name_of(h).unwrap_or_default())
        })?,
    )?;
    // GetLocalizedText[ByKey](key) → STUB "" (sem bridge de localização). Os mods tratam
    // ""/len==0 com fallback (usam o label cru). NUNCA nil (nil:len() estoura).
    {
        let loc = lua.create_function(|_, _v: bwms_lua::Value| Ok(String::new()))?;
        lua.globals().set("GetLocalizedTextByKey", loc.clone())?;
        lua.globals().set("GetLocalizedText", loc)?;
    }
    // BuildWidgetPath(segs) → devolve a própria lista de segmentos (consumida por
    // handle:GetWidgetByPath nos corpos dos hooks/Stage B). Não estoura no registro.
    lua.globals().set(
        "BuildWidgetPath",
        lua.create_function(|_, segs: bwms_lua::Value| Ok(segs))?,
    )?;
    // CalcSeed(o) → identidade do ponteiro do Handle como int (o que os mods hasheiam).
    lua.globals().set(
        "CalcSeed",
        lua.create_function(|_, v: bwms_lua::Value| {
            Ok(match v {
                bwms_lua::Value::UserData(ud) => ud.borrow::<Handle>().map(|h| h.0 as i64).unwrap_or(0),
                _ => 0,
            })
        })?,
    )?;
    lua.globals().set(
        "GetMod",
        lua.create_function(|lua, name: String| {
            // devolve a tabela que o mod retornou do init.lua (capturada por run_mod)
            match lua.globals().get::<bwms_lua::Value>("__mods")? {
                bwms_lua::Value::Table(t) => t.get::<bwms_lua::Value>(name),
                _ => Ok(bwms_lua::Value::Nil),
            }
        })?,
    )?;
    lua.globals().set(
        "ModArchiveExists",
        lua.create_function(|_, _name: String| Ok(false))?,
    )?;
    // TweakDB: tabela com os getters e o `SetFlat` sobre o TweakDB vivo do jogo. O flat sai pelo tipo do próprio `FlatValue`; o
    // `SetFlat` grava pelo escritor único (o tipo do flat converte o valor) e atualiza o record dono do flat.
    lua.globals().set("TweakDB", {
        let t = lua.create_table()?;
        for nome in ["GetFlat", "GetFloat", "GetInt", "GetBool", "GetCName", "GetTweakDBID"] {
            t.set(nome, lua.create_function(|lua, (_s, path): (bwms_lua::Value, String)| valor_do_flat(lua, &path))?)?;
        }
        for (nome, atualiza) in [("SetFlat", true), ("SetFlatNoUpdate", false)] {
            t.set(
                nome,
                lua.create_function(move |_, (_s, path, v): (bwms_lua::Value, String, bwms_lua::Value)| Ok(unsafe { gravar_flat(&path, &v, atualiza) }))?,
            )?;
        }
        t.set(
            "Update",
            lua.create_function(|_, (_s, record): (bwms_lua::Value, String)| {
                Ok(unsafe { crate::tweakdb::update_record_by_id(bwms_primitives::tweak_db_id(&record)) })
            })?,
        )?;
        t
    })?;

    // Tipos do jogo no global: nome que não é global do Lua e é tipo do RTTI vira o `Tipo` (`Vector4.new(…)`,
    // `Quaternion.Estatica(…)`); função global do jogo (`GetPlayer()`) vira função. Consulta só no global ausente.
    let meta = lua.create_table()?;
    meta.set(
        "__index",
        lua.create_function(|lua, (_g, nome): (bwms_lua::Table, String)| unsafe {
            if let Some(t) = ponte::tipo_por_nome(&nome) {
                return Ok(bwms_lua::Value::UserData(lua.create_userdata(t)?));
            }
            match crate::registry().map(|r| crate::rtti::resolve_global_function_robust(r, &nome)).filter(|f| !f.is_null()) {
                Some(f) => {
                    let rf = crate::rtti::ResolvedFn { func: f, ret_type: crate::rtti::ret_type_of(f), is_static: true };
                    Ok(bwms_lua::Value::Function(lua.create_function(move |lua, args: bwms_lua::MultiValue| {
                        let args: Vec<bwms_lua::Value> = args.into_iter().collect();
                        ponte::chamar(lua, &rf, std::ptr::null_mut(), &args)
                    })?))
                }
                None => Ok(bwms_lua::Value::Nil),
            }
        })?,
    )?;
    lua.globals().set_metatable(Some(meta));
    // GetSingleton(tipo): as estáticas do tipo (`GetSingleton('Quaternion'):ToEulerAngles(q)`).
    lua.globals().set(
        "GetSingleton",
        lua.create_function(|lua, nome: String| unsafe {
            match ponte::tipo_por_nome(&nome) {
                Some(t) => Ok(bwms_lua::Value::UserData(lua.create_userdata(t)?)),
                None => Ok(bwms_lua::Value::Nil),
            }
        })?,
    )?;
    lua.globals().set(
        "EnumValueToName",
        lua.create_function(|lua, (en, valor): (String, i64)| unsafe { ponte::nome_do_valor_de_enum(lua, &en, valor) })?,
    )?;

    interface::instalar(lua)?;
    setup_imgui_enums(lua)?;
    fechar_globais(lua)
}

/// Os globais são o que um mod alcança por `_G`, `getfenv()` e `package.loaded`. Sai o `io` (vai para o metatable protegido dos
/// globais), o `require` que lê a partir da pasta do jogo e, do `os`, tudo que não é relógio e data: arquivo, só pelo ambiente do mod,
/// preso à pasta dele.
fn fechar_globais(lua: &bwms_lua::Lua) -> bwms_lua::Result<()> {
    let g = lua.globals();
    let meta = g.metatable().ok_or_else(|| bwms_lua::Error::RuntimeError("globais sem metatable".into()))?;
    meta.set(CHAVE_DO_IO_COMPLETO, g.get::<bwms_lua::Table>("io")?)?;
    meta.set("__metatable", false)?;
    g.set("io", bwms_lua::Value::Nil)?;
    g.set("require", bwms_lua::Value::Nil)?;
    let os: bwms_lua::Table = g.get("os")?;
    let os_seguro = lua.create_table()?;
    for nome in ["clock", "time", "date", "difftime"] {
        os_seguro.set(nome, os.get::<bwms_lua::Value>(nome)?)?;
    }
    g.set("os", os_seguro.clone())?;
    let carregados: bwms_lua::Table = g.get::<bwms_lua::Table>("package")?.get("loaded")?;
    carregados.set("io", bwms_lua::Value::Nil)?;
    carregados.set("os", os_seguro)?;
    Ok(())
}

/// Chama um callback de hook (Observe/Override) passando `this` (o objeto em que o
/// método foi chamado) como lightuserdata. `try_lock`: se o Lua já está travado
/// (reentrância — um callback disparou outro método vigiado), PULA sem deadlock.
pub unsafe fn call_hook(key: &bwms_lua::RegistryKey, this: *mut c_void, params: &[(u64, u64)]) {
    let g = match LUA.try_lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if let Some(s) = g.as_ref() {
        let lua = &s.0;
        if let Ok(f) = lua.registry_value::<bwms_lua::Function>(key) {
            // monta (this, arg1, arg2, ...): this como Handle; args decodificados por tipo.
            let mut args: Vec<bwms_lua::Value> = Vec::with_capacity(params.len() + 1);
            args.push(handle_val(lua, this));
            for &(raw, tc) in params {
                args.push(decode_arg(lua, raw, tc));
            }
            let mv: bwms_lua::Variadic<bwms_lua::Value> = args.into_iter().collect();
            if let Err(e) = f.call::<()>(mv) {
                erro_de_mod("Observe", &e);
            }
        }
    }
}

/// Override-WRAPPED FLAG-BASED: chama o callback com `(this, ...args, wrapped)`
/// (wrapped por ÚLTIMO, a ordem que os mods esperam). A closure `wrapped` NÃO re-invoca a original (frame
/// sintético crashava métodos de UI); ela só seta `WRAPPED_CALLED` → o dispatch deixa a
/// ORIGINAL rodar no FRAME REAL. Se o callback não chamar wrapped (override total), a
/// original é suprimida. (Limitação: código DEPOIS de wrapped() roda antes da original e
/// wrapped() não devolve valor — ok p/ mod que só faz `wrapped(idx)` no fim.)
fn handle_val(lua: &bwms_lua::Lua, p: *mut c_void) -> bwms_lua::Value {
    match lua.create_userdata(Handle(p, std::ptr::null_mut())) {
        Ok(ud) => bwms_lua::Value::UserData(ud),
        Err(_) => bwms_lua::Value::Nil,
    }
}

/// Ponteiro do Handle como usize p/ identidade (OperatorEqual): dois Handles comparam
/// o ponteiro real; nil↔nil = 0 (iguais); não-Handle = 1 (≠ qualquer ponteiro real).
fn handle_ptr(v: &bwms_lua::Value) -> usize {
    match v {
        bwms_lua::Value::UserData(ud) => ud.borrow::<Handle>().map(|h| h.0 as usize).unwrap_or(1),
        _ => 0,
    }
}

/// Decodifica um arg pelo CName do TIPO do param (float/bool/int/CName corretos);
/// se o tipo for desconhecido, cai na heurística (Handle se ponteiro são, senão int).
fn decode_arg(lua: &bwms_lua::Lua, raw: u64, tcname: u64) -> bwms_lua::Value {
    use crate::cname::cname;
    if tcname != 0 {
        if tcname == cname("Float") {
            return bwms_lua::Value::Number(f32::from_bits(raw as u32) as f64);
        }
        if tcname == cname("Double") {
            return bwms_lua::Value::Number(f64::from_bits(raw));
        }
        if tcname == cname("Bool") {
            return bwms_lua::Value::Boolean(raw & 0xff != 0);
        }
        if tcname == cname("CName") {
            if let Ok(ud) = lua.create_userdata(CName(raw)) {
                return bwms_lua::Value::UserData(ud);
            }
        }
        if tcname == cname("Int32")
            || tcname == cname("Int64")
            || tcname == cname("Uint32")
            || tcname == cname("Uint64")
            || tcname == cname("Int16")
            || tcname == cname("Uint8")
        {
            return bwms_lua::Value::Integer(raw as i64);
        }
    }
    // heurística (Handle/inteiro) p/ tipos ricos (handle/struct) ou tipo desconhecido.
    if raw >= 0x1_0000_0000 && crate::rtti::sane(raw as *mut c_void) {
        handle_val(lua, raw as *mut c_void)
    } else {
        bwms_lua::Value::Integer(raw as i64)
    }
}

/// Escreve um valor POD no buffer de retorno `res` com a LARGURA CORRETA, gateado pelo
/// TIPO DE RETORNO declarado da função (GetName + GetSize do `IType*` em `**(func+0x18)`). Só
/// os fundamentais cuja largura sabemos casar exatamente; qualquer outra coisa
/// (classe/handle/string/array, ou um valor Lua que não bate com o tipo) → `false` =
/// NÃO é seguro suprimir a original. É isto que torna o Override-TOTAL seguro p/ funções
/// que RETORNAM valor: nunca deixa lixo no aOut (na dúvida, a original roda).
unsafe fn write_pod_ret(func: *mut c_void, res: *mut c_void, v: &bwms_lua::Value) -> bool {
    if res.is_null() {
        return false;
    }
    // `func+0x18` é o `CProperty*` do retorno; o `IType*` é o 1º campo dele.
    let ty = crate::rtti::ret_type_of(func);
    if ty.is_null() {
        return false; // void → tratado no caller (suppress sem escrever)
    }
    use crate::cname::cname;
    let tn = crate::rtti::type_name_getname(ty);
    let sz = crate::rtti::type_size(ty);
    let p = res as *mut u8;
    if tn == cname("Bool") && sz == 1 {
        if let bwms_lua::Value::Boolean(b) = v {
            *p = u8::from(*b);
            return true;
        }
    } else if (tn == cname("Int8") || tn == cname("Uint8")) && sz == 1 {
        if let bwms_lua::Value::Integer(i) = v {
            *p = *i as u8;
            return true;
        }
    } else if (tn == cname("Int16") || tn == cname("Uint16")) && sz == 2 {
        if let bwms_lua::Value::Integer(i) = v {
            (p as *mut i16).write_unaligned(*i as i16);
            return true;
        }
    } else if (tn == cname("Int32") || tn == cname("Uint32")) && sz == 4 {
        if let bwms_lua::Value::Integer(i) = v {
            (p as *mut i32).write_unaligned(*i as i32);
            return true;
        }
    } else if (tn == cname("Int64") || tn == cname("Uint64")) && sz == 8 {
        if let bwms_lua::Value::Integer(i) = v {
            (p as *mut i64).write_unaligned(*i);
            return true;
        }
    } else if tn == cname("Float") && sz == 4 {
        if let bwms_lua::Value::Number(n) = v {
            (p as *mut f32).write_unaligned(*n as f32);
            return true;
        }
    } else if tn == cname("Double") && sz == 8 {
        if let bwms_lua::Value::Number(n) = v {
            (p as *mut f64).write_unaligned(*n);
            return true;
        }
    }
    false
}

/// Override unificado: roda o callback UMA vez com `(this, ...args,
/// wrapped)`, capturando se ele chamou `wrapped()` (→ `WRAPPED_CALLED`) E o valor de
/// retorno. Se for override-TOTAL (não chamou `wrapped`) e o retorno couber num POD de
/// largura conhecida, ESCREVE o `res` (aOut) e devolve `suppress_value=true` → o
/// executor pula a original (sem efeito colateral dela). Senão NÃO toca o `res` e
/// devolve `false` → a original roda (e o rewrite pós-original via `call_hook_ret`
/// segue cobrindo getters). Retorna `(wrapped, suppress_value)`.
pub unsafe fn call_hook_override(
    key: &bwms_lua::RegistryKey,
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
        if let Ok(cb) = lua.registry_value::<bwms_lua::Function>(key) {
            let wrapped = match lua.create_function(move |_, _a: bwms_lua::Variadic<bwms_lua::Value>| {
                crate::mods_lua::observar::WRAPPED_CALLED.store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }) {
                Ok(w) => w,
                Err(_) => return (false, false),
            };
            // ordem: (this, ...args, wrapped).
            let mut args: Vec<bwms_lua::Value> = Vec::with_capacity(params.len() + 2);
            args.push(handle_val(lua, this));
            for &(raw, tc) in params {
                args.push(decode_arg(lua, raw, tc));
            }
            args.push(bwms_lua::Value::Function(wrapped));
            let mv: bwms_lua::Variadic<bwms_lua::Value> = args.into_iter().collect();
            let ret: bwms_lua::Value = cb.call(mv).unwrap_or(bwms_lua::Value::Nil);
            let wrapped_called =
                crate::mods_lua::observar::WRAPPED_CALLED.load(std::sync::atomic::Ordering::Relaxed);
            // Só tenta suprimir-com-valor em override-TOTAL (sem wrapped) que devolveu algo.
            let suppress_value = if !wrapped_called && !matches!(ret, bwms_lua::Value::Nil) {
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
pub unsafe fn call_hook_ret(key: &bwms_lua::RegistryKey, this: *mut c_void, res: *mut c_void) -> bool {
    let g = match LUA.try_lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    if let Some(s) = g.as_ref() {
        let lua = &s.0;
        if let Ok(f) = lua.registry_value::<bwms_lua::Function>(key) {
            let this_val = match lua.create_userdata(Handle(this, std::ptr::null_mut())) {
                Ok(ud) => bwms_lua::Value::UserData(ud),
                Err(_) => bwms_lua::Value::Nil,
            };
            if let Ok(ret) = f.call::<bwms_lua::Value>(this_val) {
                return encode_ret(res, &ret);
            }
        }
    }
    false
}

/// Encoda um valor Lua no buffer de retorno (16B) conforme o tipo do valor: bool→1B,
/// int→i64, float→f32, Handle→ptr. nil/outros = não reescreve.
unsafe fn encode_ret(res: *mut c_void, v: &bwms_lua::Value) -> bool {
    if res.is_null() {
        return false;
    }
    let p = res as *mut u8;
    match v {
        bwms_lua::Value::Boolean(b) => {
            *p = u8::from(*b);
            true
        }
        bwms_lua::Value::Integer(i) => {
            (p as *mut i64).write_unaligned(*i);
            true
        }
        bwms_lua::Value::Number(n) => {
            (p as *mut f32).write_unaligned(*n as f32);
            true
        }
        bwms_lua::Value::UserData(ud) => {
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

unsafe fn lock_init<'a>(
    g: &'a mut std::sync::MutexGuard<'_, Option<SendLua>>,
) -> Option<&'a bwms_lua::Lua> {
    if g.is_none() {
        let lua = bwms_lua::Lua::new();
        if let Err(e) = setup_api(&lua) {
            crate::log(&format!("[lua] setup ERRO: {e}"));
        }
        **g = Some(SendLua(lua));
        crate::log("[lua] runtime inicializado (Game.*, registerForEvent)");
    }
    g.as_ref().map(|s| &s.0)
}

/// Descarta o estado Lua (próximo run_code re-inicializa limpo via setup_api). O
/// loadmod usa isto p/ recarregar um mod SEM duplicar callbacks (onDraw/onUpdate).
/// Não desliga cheats já aplicados no jogo (godmode/itens vivem na engine, não no Lua).
/// TODO 100%: mod-manager com isolamento por-mod (mods/<nome>/init.lua coexistindo).
pub unsafe fn reset() {
    let mut g = LUA.lock().unwrap_or_else(|e| e.into_inner());
    // dispara onShutdown ANTES de descartar (mods salvam estado/limpam).
    if let Some(s) = g.as_ref() {
        let lua = &s.0;
        let _: bwms_lua::Result<()> = (|| {
            let events: bwms_lua::Table = lua.globals().get("__events")?;
            if let Some(list) = events.get::<Option<bwms_lua::Table>>("onShutdown")? {
                for cb in list.sequence_values::<bwms_lua::Function>().flatten() {
                    let _: bwms_lua::Result<()> = cb.call(());
                }
            }
            Ok(())
        })();
    }
    *g = None;
}

/// Executa um trecho de código Lua no estado persistente. Quem chama é o terminal de quem joga (`luarun`), não um mod: roda num
/// ambiente com o `io` completo, que lê e grava nos globais.
pub unsafe fn run_code(code: &str) {
    let mut g = LUA.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(lua) = lock_init(&mut g) {
        let r: bwms_lua::Result<()> = (|| {
            let env = lua.create_table()?;
            let meta = lua.create_table()?;
            meta.set("__index", lua.globals())?;
            meta.set("__newindex", lua.globals())?;
            env.set_metatable(Some(meta));
            env.set("io", io_completo(lua)?)?;
            lua.load(code).set_environment(env).exec()
        })();
        if let Err(e) = r {
            crate::log(&format!("[lua] ERRO: {e}"));
        }
    }
}

/// O `io` completo guardado por `fechar_globais` no metatable protegido dos globais.
fn io_completo(lua: &bwms_lua::Lua) -> bwms_lua::Result<bwms_lua::Table> {
    lua.globals()
        .metatable()
        .map(|m| m.get::<Option<bwms_lua::Table>>(CHAVE_DO_IO_COMPLETO))
        .transpose()?
        .flatten()
        .ok_or_else(|| bwms_lua::Error::RuntimeError("io do estado Lua ausente: o setup não rodou".into()))
}

/// Roda o `init.lua` de um mod no ambiente dele ([`ambiente_do_mod`]) e guarda o `return` em `__mods[name]` (o que o `GetMod(name)` devolve).
pub unsafe fn run_mod(name: &str, code: &str, dir: &std::path::Path) {
    let mut g = LUA.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(lua) = lock_init(&mut g) {
        let r: bwms_lua::Result<()> = (|| {
            let env = ambiente_do_mod(lua, dir)?;
            let ret: bwms_lua::Value = lua.load(code).set_name(format!("@{name}/init.lua")).set_environment(env).call(())?;
            let mods: bwms_lua::Table = match lua.globals().get::<bwms_lua::Value>("__mods")? {
                bwms_lua::Value::Table(t) => t,
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
            erro_de_mod(&format!("mod '{name}'"), &e);
        }
    }
}

/// Caminho de arquivo do mod: relativo à pasta dele, sem sair dela (absoluto e `..` recusados): o `io` de cada mod fica restrito.
fn caminho_no_mod(dir: &std::path::Path, rel: &str) -> Option<std::path::PathBuf> {
    // Mod de Windows escreve o caminho com `\`.
    let rel = rel.replace('\\', "/");
    let p = std::path::Path::new(&rel);
    p.components().all(|c| matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir)).then(|| dir.join(p))
}

/// Arquivo de script do mod por `rel`, como o `loadfile` que os mods chamam: o caminho como veio ou com `.lua`.
fn script_no_mod(dir: &std::path::Path, rel: &str) -> Option<std::path::PathBuf> {
    [rel.to_string(), format!("{rel}.lua")].iter().filter_map(|r| caminho_no_mod(dir, r)).find(|p| p.is_file())
}

/// Globais pequenos do sandbox dos mods: `IconGlyphs` (ícone sem fonte no terminal vira texto vazio), `ImVec2`/`ImVec4`, `CRUID`,
/// `LocKey`, `Dump`/`DumpType`/`DumpAllTypeNames`. `GameOptions` responde `nil` e avisa: as opções do motor ainda não têm ponte.
fn setup_globais_de_tipo(lua: &bwms_lua::Lua) -> bwms_lua::Result<()> {
    use bwms_lua::Value as V;
    let g = lua.globals();

    let icones = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.set("__index", lua.create_function(|_, (_t, _k): (V, V)| Ok(""))?)?;
    icones.set_metatable(Some(meta));
    g.set("IconGlyphs", icones)?;

    for (nome, campos) in [("ImVec2", &["x", "y"][..]), ("ImVec4", &["x", "y", "z", "w"][..])] {
        let t = lua.create_table()?;
        t.set(
            "new",
            lua.create_function(move |lua, a: bwms_lua::MultiValue| {
                let v = lua.create_table()?;
                for (k, x) in campos.iter().zip(a) {
                    v.set(*k, x)?;
                }
                Ok(v)
            })?,
        )?;
        g.set(nome, t)?;
    }

    g.set("CRUID", lua.create_function(|_, n: f64| Ok(n))?)?;
    g.set("LocKey", lua.create_function(|_, v: V| Ok(v))?)?;

    g.set(
        "GameOptions",
        {
            let t = lua.create_table()?;
            let meta = lua.create_table()?;
            meta.set(
                "__index",
                lua.create_function(|lua, (_t, k): (V, String)| {
                    crate::log(&format!("[lua] GameOptions.{k}: sem ponte com as opções do motor, responde nil"));
                    lua.create_function(|_, _: bwms_lua::MultiValue| Ok(V::Nil))
                })?,
            )?;
            t.set_metatable(Some(meta));
            t
        },
    )?;

    let dump_tipo = lua.create_function(|_, nome: String| {
        let props = unsafe {
            let cls = crate::registry().map_or(std::ptr::null_mut(), |r| r.class_by_name(&nome));
            if cls.is_null() { Vec::new() } else { crate::rtti::propriedades(cls) }
        };
        crate::log(&format!("[lua] DumpType {nome}: {} propriedade(s)", props.len()));
        for (n, ty, off) in props {
            crate::log(&format!("[lua]   +0x{off:x} {}: {}", crate::cname::resolve_cname(n), unsafe {
                crate::cname::resolve_cname(crate::rtti::type_name_getname(ty))
            }));
        }
        Ok(())
    })?;
    g.set("DumpType", dump_tipo.clone())?;
    g.set(
        "Dump",
        lua.create_function(move |_, v: V| {
            let V::UserData(u) = v else { return Ok(()) };
            let Ok(h) = u.borrow::<Handle>() else { return Ok(()) };
            let nome = unsafe { crate::cname::resolve_cname(crate::rtti::type_name_getname(h.1)) };
            dump_tipo.call::<()>(nome)
        })?,
    )?;
    g.set(
        "DumpAllTypeNames",
        lua.create_function(|_, ()| {
            let n = unsafe {
                crate::registry().map_or(0, |r| {
                    let base = r.class_by_name("IScriptable");
                    let (lista, n) = r.get_classes(base, true);
                    for i in 0..n {
                        let cls = (lista as *const *mut c_void).add(i).read();
                        crate::log(&format!("[lua] {}", crate::cname::resolve_cname(crate::rtti::type_name_getname(cls))));
                    }
                    n
                })
            };
            crate::log(&format!("[lua] DumpAllTypeNames: {n} tipo(s)"));
            Ok(())
        })?,
    )?;
    Ok(())
}

/// Compila `codigo` no ambiente `env`: `(função, nil)` ou `(nil, mensagem)`, como o `loadstring` do sandbox dos mods.
fn carregar_no_ambiente(lua: &bwms_lua::Lua, codigo: Vec<u8>, nome: String, env: &bwms_lua::Table) -> bwms_lua::Result<(bwms_lua::Value, bwms_lua::Value)> {
    Ok(match lua.load(codigo).set_name(nome).set_environment(env.clone()).into_function() {
        Ok(f) => (bwms_lua::Value::Function(f), bwms_lua::Value::Nil),
        Err(e) => (bwms_lua::Value::Nil, bwms_lua::Value::String(lua.create_string(e.to_string())?)),
    })
}

/// Ambiente de um mod: lê os globais por `__index` e guarda os globais do mod nele, com `require`, `io.open`/`io.lines`, `os.remove`/
/// `os.rename`, `dofile` e `dir` presos à pasta do mod (o sandbox de cada mod).
fn ambiente_do_mod(lua: &bwms_lua::Lua, dir: &std::path::Path) -> bwms_lua::Result<bwms_lua::Table> {
    let env = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.set("__index", lua.globals())?;
    env.set_metatable(Some(meta));
    let carregados = lua.create_table()?;

    // `require` devolve `(módulo, erro)`, sem levantar erro: mod testa o `nil`. Módulo em carga responde `nil` (corta `require` circular).
    let em_carga = lua.create_table()?;
    let (d, e, c) = (dir.to_path_buf(), env.clone(), carregados.clone());
    env.set(
        "require",
        lua.create_function(move |lua, nome: String| {
            use bwms_lua::Value as V;
            let rel = nome.replace('.', "/");
            let candidatos = [nome.clone(), format!("{nome}.lua"), format!("{rel}.lua"), format!("{nome}/init.lua")];
            let Some(arq) = candidatos.iter().filter_map(|r| caminho_no_mod(&d, r)).find(|p| p.is_file()) else {
                let msg = lua.create_string(format!("módulo '{nome}' não existe dentro da pasta do mod"))?;
                return Ok((V::Nil, V::String(msg)));
            };
            let chave = arq.to_string_lossy().into_owned();
            if let Some(v) = c.get::<Option<V>>(chave.as_str())? {
                return Ok((v, V::Nil));
            }
            if em_carga.get::<Option<bool>>(chave.as_str())?.is_some() {
                return Ok((V::Nil, V::Nil));
            }
            let codigo = std::fs::read(&arq).map_err(bwms_lua::Error::external)?;
            let (f, erro) = carregar_no_ambiente(lua, codigo, format!("@{chave}"), &e)?;
            let V::Function(f) = f else {
                erro_de_mod(&format!("módulo '{nome}'"), &bwms_lua::Error::RuntimeError(erro.to_string()?));
                return Ok((V::Nil, erro));
            };
            em_carga.set(chave.as_str(), true)?;
            let r = f.call::<V>(());
            em_carga.set(chave.as_str(), V::Nil)?;
            match r {
                Ok(v) => {
                    let v = if v.is_nil() { V::Boolean(true) } else { v };
                    c.set(chave.as_str(), v.clone())?;
                    Ok((v, V::Nil))
                }
                Err(falha) => {
                    erro_de_mod(&format!("módulo '{nome}'"), &falha);
                    Ok((V::Nil, V::String(lua.create_string(falha.to_string())?)))
                }
            }
        })?,
    )?;

    let e2 = env.clone();
    env.set(
        "loadstring",
        lua.create_function(move |lua, (s, nome): (bwms_lua::LuaString, Option<String>)| {
            carregar_no_ambiente(lua, s.as_bytes().to_vec(), nome.unwrap_or_else(|| "=loadstring".into()), &e2)
        })?,
    )?;
    let (d, e2) = (dir.to_path_buf(), env.clone());
    env.set(
        "loadfile",
        lua.create_function(move |lua, rel: String| {
            let Some(p) = script_no_mod(&d, &rel) else {
                let msg = lua.create_string(format!("'{rel}' não existe dentro da pasta do mod"))?;
                return Ok((bwms_lua::Value::Nil, bwms_lua::Value::String(msg)));
            };
            let codigo = std::fs::read(&p).map_err(bwms_lua::Error::external)?;
            carregar_no_ambiente(lua, codigo, format!("@{}", p.display()), &e2)
        })?,
    )?;

    let io = io_completo(lua)?;
    let io_mod = lua.create_table()?;
    for par in io.pairs::<bwms_lua::Value, bwms_lua::Value>() {
        let (k, v) = par?;
        io_mod.set(k, v)?;
    }
    for nome in ["open", "lines"] {
        let (d, f) = (dir.to_path_buf(), io.get::<bwms_lua::Function>(nome)?);
        io_mod.set(
            nome,
            lua.create_function(move |_, (rel, resto): (String, bwms_lua::MultiValue)| match caminho_no_mod(&d, &rel) {
                Some(p) => f.call::<bwms_lua::MultiValue>((p.to_string_lossy().into_owned(), resto)),
                None => Ok(bwms_lua::MultiValue::from_vec(vec![bwms_lua::Value::Nil])),
            })?,
        )?;
    }
    env.set("io", io_mod)?;

    let os: bwms_lua::Table = lua.globals().get("os")?;
    let os_mod = lua.create_table()?;
    for par in os.pairs::<bwms_lua::Value, bwms_lua::Value>() {
        let (k, v) = par?;
        os_mod.set(k, v)?;
    }
    let d = dir.to_path_buf();
    os_mod.set(
        "remove",
        lua.create_function(move |_, rel: String| Ok(caminho_no_mod(&d, &rel).is_some_and(|p| std::fs::remove_file(p).is_ok())))?,
    )?;
    let d = dir.to_path_buf();
    os_mod.set(
        "rename",
        lua.create_function(move |_, (de, para): (String, String)| {
            Ok(match (caminho_no_mod(&d, &de), caminho_no_mod(&d, &para)) {
                (Some(a), Some(b)) => std::fs::rename(a, b).is_ok(),
                _ => false,
            })
        })?,
    )?;
    env.set("os", os_mod)?;

    let (d, e) = (dir.to_path_buf(), env.clone());
    env.set(
        "dofile",
        lua.create_function(move |lua, rel: String| {
            let p = script_no_mod(&d, &rel).ok_or_else(|| bwms_lua::Error::RuntimeError(format!("'{rel}' não existe dentro da pasta do mod")))?;
            let src = std::fs::read(&p).map_err(bwms_lua::Error::external)?;
            lua.load(src).set_name(format!("@{}", p.display())).set_environment(e.clone()).call::<bwms_lua::MultiValue>(())
        })?,
    )?;

    let nome_do_mod = dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    env.set("db", BancoDoMod(std::sync::Arc::new(Mutex::new(banco::Banco::do_mod(dir))), nome_do_mod))?;

    let d = dir.to_path_buf();
    env.set(
        "dir",
        lua.create_function(move |lua, rel: String| {
            let t = lua.create_table()?;
            let Some(p) = caminho_no_mod(&d, &rel) else { return Ok(t) };
            let Ok(rd) = std::fs::read_dir(p) else { return Ok(t) };
            for (i, ent) in rd.flatten().enumerate() {
                let item = lua.create_table()?;
                item.set("name", ent.file_name().to_string_lossy().into_owned())?;
                item.set("type", if ent.path().is_dir() { "directory" } else { "file" })?;
                t.set(i + 1, item)?;
            }
            Ok(t)
        })?,
    )?;
    Ok(env)
}

/// Último instante em que o onUpdate rodou — p/ computar o `delta` (segundos) que os mods esperam
/// no callback. Sem isso, mods que fazem `Cron.Update(delta)` erram todo frame
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
/// passa o `delta` (segundos desde o último) — alguns mods dependem dele.
pub unsafe fn run_event(name: &str) {
    let mut g = LUA.lock().unwrap_or_else(|e| e.into_inner());
    // não inicializa o runtime só pra rodar evento (evita custo se não há mod).
    if g.is_none() {
        return;
    }
    let delta = if name == "onUpdate" { next_update_delta() } else { 0.0 };
    if let Some(lua) = lock_init(&mut g) {
        let r: bwms_lua::Result<()> = (|| {
            let events: bwms_lua::Table = lua.globals().get("__events")?;
            if let Some(list) = events.get::<Option<bwms_lua::Table>>(name)? {
                for cb in list.sequence_values::<bwms_lua::Function>().flatten() {
                    // erro de UM callback NÃO pode abortar os outros mods — loga e segue.
                    let res = if name == "onUpdate" {
                        cb.call::<()>(delta)
                    } else {
                        cb.call::<()>(())
                    };
                    if let Err(e) = res {
                        erro_de_mod(&format!("evento '{name}'"), &e);
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            erro_de_mod(&format!("evento '{name}'"), &e);
        }
    }
}

/// Como run_event, mas TRY_LOCK — roda na thread de RENDER (present hook). Se o Lua
/// estiver travado pela thread do jogo, PULA o frame em vez de travar o render.
/// Só p/ "onDraw" (mods desenhando ImGui).
pub unsafe fn run_event_draw() {
    // Até 3 ms pela trava (o `onUpdate` do tick a segura): sem espera ilimitada no `onDraw`, o render não fica preso ao jogo.
    let inicio = std::time::Instant::now();
    let mut g = loop {
        match LUA.try_lock() {
            Ok(g) => break g,
            Err(_) if inicio.elapsed() < std::time::Duration::from_millis(3) => std::thread::yield_now(),
            Err(_) => {
                QUADROS_DO_ONDRAW[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        }
    };
    QUADROS_DO_ONDRAW[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if g.is_none() {
        return;
    }
    if let Some(lua) = lock_init(&mut g) {
        let r: bwms_lua::Result<()> = (|| {
            let events: bwms_lua::Table = lua.globals().get("__events")?;
            if let Some(list) = events.get::<Option<bwms_lua::Table>>("onDraw")? {
                for cb in list.sequence_values::<bwms_lua::Function>().flatten() {
                    if let Err(e) = cb.call::<()>(()) {
                        erro_de_mod("evento 'onDraw'", &e);
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            erro_de_mod("evento 'onDraw'", &e);
        }
    }
}

// Os nomes são o contrato (o mod escreve `ImGuiWindowFlags.NoDocking`); os valores são do BWMS e nada os lê: a janela Lua aceita e
// ignora flags. Membros em ordem alfabética: `None` vale 0, enum `*Flags*` dá um bit por membro, os outros contam 0, 1, 2...
const IMGUI_ENUMS: &[(&str, &[&str])] = &[
    ("ImGuiWindowFlags", &[
        "AlwaysAutoResize", "AlwaysHorizontalScrollbar", "AlwaysVerticalScrollbar", "ChildMenu", "ChildWindow", "HorizontalScrollbar",
        "MenuBar", "Modal", "NoBackground", "NoBringToFrontOnFocus", "NoCollapse", "NoDecoration", "NoDocking", "NoFocusOnAppearing",
        "NoInputs", "NoMouseInputs", "NoMove", "NoNav", "NoNavFocus", "NoNavInputs", "NoResize", "NoSavedSettings", "NoScrollWithMouse",
        "NoScrollbar", "NoTitleBar", "None", "Popup", "Tooltip", "UnsavedDocument",
    ]),
    ("ImGuiCol", &[
        "Border", "BorderShadow", "Button", "ButtonActive", "ButtonHovered", "CheckMark", "ChildBg", "DragDropTarget", "FrameBg",
        "FrameBgActive", "FrameBgHovered", "Header", "HeaderActive", "HeaderHovered", "MenuBarBg", "ModalWindowDimBg", "NavHighlight",
        "NavWindowingDimBg", "NavWindowingHighlight", "PlotHistogram", "PlotHistogramHovered", "PlotLines", "PlotLinesHovered", "PopupBg",
        "ResizeGrip", "ResizeGripActive", "ResizeGripHovered", "ScrollbarBg", "ScrollbarGrab", "ScrollbarGrabActive",
        "ScrollbarGrabHovered", "Separator", "SeparatorActive", "SeparatorHovered", "SliderGrab", "SliderGrabActive", "Tab", "TabActive",
        "TabHovered", "TabUnfocused", "TabUnfocusedActive", "TableBorderLight", "TableBorderStrong", "TableHeaderBg", "TableRowBg",
        "TableRowBgAlt", "Text", "TextDisabled", "TextSelectedBg", "TitleBg", "TitleBgActive", "TitleBgCollapsed", "WindowBg",
    ]),
    ("ImGuiStyleVar", &[
        "Alpha", "ButtonTextAlign", "ChildBorderSize", "ChildRounding", "DisabledAlpha", "FrameBorderSize", "FrameRounding", "GrabMinSize",
        "GrabRounding", "IndentSpacing", "ItemInnerSpacing", "ItemSpacing", "PopupBorderSize", "PopupRounding", "ScrollbarRounding",
        "ScrollbarSize", "SelectableTextAlign", "TabRounding", "WindowBorderSize", "WindowMinSize", "WindowRounding", "WindowTitleAlign",
    ]),
    ("ImGuiCond", &[
        "Always", "Appearing", "FirstUseEver", "None", "Once",
    ]),
    ("ImGuiTreeNodeFlags", &[
        "AllowItemOverlap", "Bullet", "CollapsingHeader", "DefaultOpen", "Framed", "Leaf", "NavLeftJumpsBackHere", "NoAutoOpenOnLog",
        "NoTreePushOnOpen", "None", "OpenOnArrow", "OpenOnDoubleClick", "Selected", "SpanAvailWidth", "SpanFullWidth",
    ]),
    ("ImGuiInputTextFlags", &[
        "AllowTabInput", "AlwaysOverwrite", "AutoSelectAll", "CallbackAlways", "CallbackCharFilter", "CallbackCompletion", "CallbackEdit",
        "CallbackHistory", "CallbackResize", "CharsDecimal", "CharsHexadecimal", "CharsNoBlank", "CharsScientific", "CharsUppercase",
        "CtrlEnterForNewLine", "EnterReturnsTrue", "EscapeClearsAll", "NoHorizontalScroll", "NoUndoRedo", "None", "Password", "ReadOnly",
    ]),
    ("ImGuiTableFlags", &[
        "Borders", "BordersH", "BordersInner", "BordersInnerH", "BordersInnerV", "BordersOuter", "BordersOuterH", "BordersOuterV",
        "BordersV", "ContextMenuInBody", "Hideable", "NoBordersInBody", "NoBordersInBodyUntilResize", "NoClip", "NoHostExtendX",
        "NoHostExtendY", "NoKeepColumnsVisible", "NoPadInnerX", "NoPadOuterX", "NoSavedSettings", "None", "PadOuterX", "PreciseWidths",
        "Reorderable", "Resizable", "RowBg", "ScrollX", "ScrollY", "SizingFixedFit", "SizingFixedSame", "SizingStretchProp",
        "SizingStretchSame", "SortMulti", "SortTristate", "Sortable",
    ]),
    ("ImGuiTableColumnFlags", &[
        "DefaultHide", "DefaultSort", "Disabled", "IndentDisable", "IndentEnable", "IsEnabled", "IsHovered", "IsSorted", "IsVisible",
        "NoClip", "NoHeaderLabel", "NoHeaderWidth", "NoHide", "NoReorder", "NoResize", "NoSort", "NoSortAscending", "NoSortDescending",
        "None", "PreferSortAscending", "PreferSortDescending", "WidthFixed", "WidthStretch",
    ]),
    ("ImGuiTableRowFlags", &[
        "Headers", "None",
    ]),
    ("ImGuiTableBgTarget", &[
        "CellBg", "None", "RowBg0", "RowBg1",
    ]),
    ("ImGuiSelectableFlags", &[
        "AllowDoubleClick", "AllowItemOverlap", "Disabled", "DontClosePopups", "None", "SpanAllColumns",
    ]),
    ("ImGuiTabBarFlags", &[
        "AutoSelectNewTabs", "FittingPolicyResizeDown", "FittingPolicyScroll", "NoCloseWithMiddleMouseButton", "NoTabListScrollingButtons",
        "NoTooltip", "None", "Reorderable", "TabListPopupButton",
    ]),
    ("ImGuiTabItemFlags", &[
        "Leading", "NoCloseWithMiddleMouseButton", "NoPushId", "NoReorder", "NoTooltip", "None", "SetSelected", "Trailing",
        "UnsavedDocument",
    ]),
    ("ImGuiComboFlags", &[
        "HeightLarge", "HeightLargest", "HeightRegular", "HeightSmall", "NoArrowButton", "NoPreview", "None", "PopupAlignLeft",
    ]),
    ("ImGuiHoveredFlags", &[
        "AllowWhenBlockedByActiveItem", "AllowWhenBlockedByPopup", "AllowWhenDisabled", "AllowWhenOverlapped", "AnyWindow", "ChildWindows",
        "DelayNormal", "DelayShort", "NoNavOverride", "NoPopupHierarchy", "NoSharedDelay", "None", "RectOnly", "RootAndChildWindows",
        "RootWindow",
    ]),
    ("ImGuiFocusedFlags", &[
        "AnyWindow", "ChildWindows", "NoPopupHierarchy", "None", "RootAndChildWindows", "RootWindow",
    ]),
    ("ImGuiDir", &[
        "Down", "Left", "None", "Right", "Up",
    ]),
    ("ImGuiSliderFlags", &[
        "AlwaysClamp", "Logarithmic", "NoInput", "NoRoundToFormat", "None",
    ]),
    ("ImGuiColorEditFlags", &[
        "AlphaBar", "AlphaPreview", "AlphaPreviewHalf", "DisplayHSV", "DisplayHex", "DisplayRGB", "Float", "HDR", "InputHSV", "InputRGB",
        "NoAlpha", "NoBorder", "NoDragDrop", "NoInputs", "NoLabel", "NoOptions", "NoPicker", "NoSidePreview", "NoSmallPreview", "NoTooltip",
        "None", "PickerHueBar", "PickerHueWheel", "Uint8",
    ]),
    ("ImGuiMouseButton", &[
        "Left", "Middle", "Right",
    ]),
    ("ImGuiMouseCursor", &[
        "Arrow", "Hand", "None", "NotAllowed", "ResizeAll", "ResizeEW", "ResizeNESW", "ResizeNS", "ResizeNWSE", "TextInput",
    ]),
    ("ImGuiPopupFlags", &[
        "AnyPopup", "AnyPopupId", "AnyPopupLevel", "MouseButtonLeft", "MouseButtonMiddle", "MouseButtonRight", "NoOpenOverExistingPopup",
        "NoOpenOverItems", "None",
    ]),
    ("ImDrawFlags", &[
        "Closed", "None", "RoundCornersAll", "RoundCornersBottom", "RoundCornersBottomLeft", "RoundCornersBottomRight", "RoundCornersLeft",
        "RoundCornersNone", "RoundCornersRight", "RoundCornersTop", "RoundCornersTopLeft", "RoundCornersTopRight",
    ]),
    ("ImGuiKey", &[
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "A", "Apostrophe", "B", "Backslash", "Backspace", "C", "CapsLock", "Comma", "D",
        "Delete", "DownArrow", "E", "End", "Enter", "Equal", "Escape", "F", "F1", "F10", "F11", "F12", "F2", "F3", "F4", "F5", "F6", "F7",
        "F8", "F9", "G", "GamepadBack", "GamepadDpadDown", "GamepadDpadLeft", "GamepadDpadRight", "GamepadDpadUp", "GamepadFaceDown",
        "GamepadFaceLeft", "GamepadFaceRight", "GamepadFaceUp", "GamepadL1", "GamepadL2", "GamepadL3", "GamepadLStickDown",
        "GamepadLStickLeft", "GamepadLStickRight", "GamepadLStickUp", "GamepadR1", "GamepadR2", "GamepadR3", "GamepadRStickDown",
        "GamepadRStickLeft", "GamepadRStickRight", "GamepadRStickUp", "GamepadStart", "GraveAccent", "H", "Home", "I", "Insert", "J", "K",
        "Keypad0", "Keypad1", "Keypad2", "Keypad3", "Keypad4", "Keypad5", "Keypad6", "Keypad7", "Keypad8", "Keypad9", "KeypadAdd",
        "KeypadDecimal", "KeypadDivide", "KeypadEnter", "KeypadEqual", "KeypadMultiply", "KeypadSubtract", "L", "LeftAlt", "LeftArrow",
        "LeftBracket", "LeftCtrl", "LeftShift", "LeftSuper", "M", "Menu", "Minus", "MouseLeft", "MouseMiddle", "MouseRight", "MouseWheelX",
        "MouseWheelY", "MouseX1", "MouseX2", "N", "None", "NumLock", "O", "P", "PageDown", "PageUp", "Pause", "Period", "PrintScreen", "Q",
        "R", "RightAlt", "RightArrow", "RightBracket", "RightCtrl", "RightShift", "RightSuper", "S", "ScrollLock", "Semicolon", "Slash",
        "Space", "T", "Tab", "U", "UpArrow", "V", "W", "X", "Y", "Z",
    ]),
];

fn setup_imgui_enums(lua: &bwms_lua::Lua) -> bwms_lua::Result<()> {
    for (nome, membros) in IMGUI_ENUMS {
        let t = lua.create_table()?;
        let bits = nome.contains("Flags");
        let base = i64::from(membros.contains(&"None"));
        let mut i = 0i64;
        for m in *membros {
            if *m == "None" {
                t.set("None", 0i64)?;
                continue;
            }
            t.set(*m, if bits { 1i64 << i } else { base + i })?;
            i += 1;
        }
        lua.globals().set(*nome, t)?;
    }
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

    /// Monta `func(+0x18) -> CProperty(+0) -> IType(->vtable[GetName@0x10, GetSize@0x18])` e chama write_pod_ret.
    unsafe fn run(
        gn: extern "C" fn(*mut c_void) -> u64,
        gs: extern "C" fn(*mut c_void) -> u32,
        v: &bwms_lua::Value,
    ) -> (bool, [u8; 16]) {
        // vtable: 4 slots; +0x10 (idx2)=GetName, +0x18 (idx3)=GetSize.
        let vtable: [usize; 4] = [0, 0, gn as usize, gs as usize];
        // IType: 3 ponteiros (>=0x18 legível); [0] = &vtable.
        let ity: [usize; 3] = [vtable.as_ptr() as usize, 0, 0];
        // CProperty do retorno: [0] = &ity. func: 0x20 bytes; +0x18 = &cprop.
        let cprop: [usize; 6] = [ity.as_ptr() as usize, 0, 0, 0, 0, 0];
        let mut func = [0u8; 0x20];
        (func.as_mut_ptr().add(0x18) as *mut usize).write_unaligned(cprop.as_ptr() as usize);
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
            let (ok, r) = run(gn_i32, gs4, &bwms_lua::Value::Integer(0x1234_5678_i64));
            assert!(ok);
            assert_eq!(u32::from_le_bytes([r[0], r[1], r[2], r[3]]), 0x1234_5678);
            assert_eq!(&r[4..], &[0u8; 12], "bytes acima da largura NÃO tocados");
        }
    }
    #[test]
    fn bool_1byte() {
        unsafe {
            let (ok, r) = run(gn_bool, gs1, &bwms_lua::Value::Boolean(true));
            assert!(ok);
            assert_eq!(r[0], 1);
            assert_eq!(&r[1..], &[0u8; 15]);
        }
    }
    #[test]
    fn float_4bytes() {
        unsafe {
            let (ok, r) = run(gn_f32, gs4, &bwms_lua::Value::Number(1.5));
            assert!(ok);
            assert_eq!(f32::from_le_bytes([r[0], r[1], r[2], r[3]]), 1.5);
            assert_eq!(&r[4..], &[0u8; 12]);
        }
    }
    #[test]
    fn int64_8bytes() {
        unsafe {
            let (ok, r) = run(gn_i64, gs8, &bwms_lua::Value::Integer(0x0102_0304_0506_0708_i64));
            assert!(ok);
            assert_eq!(u64::from_le_bytes(r[0..8].try_into().unwrap()), 0x0102_0304_0506_0708);
            assert_eq!(&r[8..], &[0u8; 8]);
        }
    }
    #[test]
    fn double_8bytes() {
        unsafe {
            let (ok, r) = run(gn_f64, gs8, &bwms_lua::Value::Number(2.5));
            assert!(ok);
            assert_eq!(f64::from_le_bytes(r[0..8].try_into().unwrap()), 2.5);
        }
    }

    // ---- SEGURANÇA: qualquer dúvida → false (não suprime → original roda) ----

    #[test]
    fn nonpod_class_refused() {
        unsafe {
            let (ok, r) = run(gn_class, gs8, &bwms_lua::Value::Integer(5));
            assert!(!ok, "tipo NÃO-POD nunca suprime");
            assert_eq!(&r[..], &[0u8; 16], "nada escrito no res");
        }
    }
    #[test]
    fn type_value_mismatch_refused() {
        unsafe {
            // tipo Int32 mas valor Boolean → não casa → false
            let (ok, _) = run(gn_i32, gs4, &bwms_lua::Value::Boolean(true));
            assert!(!ok);
        }
    }
    #[test]
    fn size_mismatch_refused() {
        unsafe {
            // nome Int32 mas size 8 (largura não bate) → false
            let (ok, _) = run(gn_i32, gs8, &bwms_lua::Value::Integer(1));
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
                &bwms_lua::Value::Integer(1),
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
                &bwms_lua::Value::Integer(1),
            );
            assert!(!ok);
        }
    }
}

/// `array<CName>` de argumento: o buffer vem do reserve do motor, que guarda o alocador do array no fim, e a VM solta por ele.
unsafe fn lista_de_cnames(cns: &[u64]) -> [u8; 16] {
    let mut lista = [0u8; 16];
    if cns.is_empty() {
        return lista;
    }
    crate::jogo::listas::reservar(lista.as_mut_ptr() as *mut c_void, cns.len() as u32, 8, 8, std::ptr::null());
    let entries = (lista.as_ptr() as *const u64).read_unaligned() as *mut u64;
    if entries.is_null() {
        return [0u8; 16];
    }
    std::ptr::copy_nonoverlapping(cns.as_ptr(), entries, cns.len());
    (lista.as_mut_ptr().add(0xc) as *mut u32).write_unaligned(cns.len() as u32);
    lista
}

/// Teclas com `registerHotkey` e com `registerInput`, e o que o `sendEvent` pegou delas até o próximo tick.
static TECLAS_DE_HOTKEY: Mutex<Vec<(char, String)>> = Mutex::new(Vec::new());
static TECLAS_DE_INPUT: Mutex<Vec<char>> = Mutex::new(Vec::new());
static TECLAS_APERTADAS: Mutex<Vec<(char, bool)>> = Mutex::new(Vec::new());

fn hotkey_register_char(c: char, id: &str) {
    let mut g = TECLAS_DE_HOTKEY.lock().unwrap_or_else(|e| e.into_inner());
    if !g.iter().any(|(t, i)| *t == c && i == id) {
        g.push((c, id.to_string()));
    }
}

fn input_register_char(c: char) {
    let mut g = TECLAS_DE_INPUT.lock().unwrap_or_else(|e| e.into_inner());
    if !g.contains(&c) {
        g.push(c);
    }
}

/// Tecla do `sendEvent` (main thread): guarda só as que algum mod registrou; o tick despacha. Fila com teto pra quadro sem tick.
pub(crate) fn tecla(c: char, apertada: bool) {
    let registrada = TECLAS_DE_INPUT.lock().is_ok_and(|g| g.contains(&c))
        || (apertada && TECLAS_DE_HOTKEY.lock().is_ok_and(|g| g.iter().any(|(t, _)| *t == c)));
    if !registrada {
        return;
    }
    if let Ok(mut g) = TECLAS_APERTADAS.lock() {
        if g.len() < 64 {
            g.push((c, apertada));
        }
    }
}

/// No tick (thread do jogo): dispara `registerHotkey` no aperto e `registerInput` no aperto e na soltura.
pub(crate) unsafe fn despachar_teclas() {
    let fila = TECLAS_APERTADAS.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    for (c, apertada) in fila {
        if apertada && TECLAS_DE_HOTKEY.lock().is_ok_and(|g| g.iter().any(|(t, _)| *t == c)) {
            fire_hotkey(c);
        }
        if TECLAS_DE_INPUT.lock().is_ok_and(|g| g.contains(&c)) {
            fire_input(c, apertada);
        }
    }
}

/// Flat escalar do TweakDB vivo como valor Lua, pelo tipo do flat; `nil` pra flat ausente ou não escalar.
fn valor_do_flat(lua: &bwms_lua::Lua, path: &str) -> bwms_lua::Result<bwms_lua::Value> {
    let Some((tipo, v)) = (unsafe { crate::tweakdb::flat_escalar(bwms_primitives::tweak_db_id(path)) }) else {
        return Ok(bwms_lua::Value::Nil);
    };
    Ok(match tipo.as_str() {
        "Float" => bwms_lua::Value::Number(f64::from(f32::from_bits(v as u32))),
        "Int32" => bwms_lua::Value::Integer(i64::from(v as u32 as i32)),
        "Bool" => bwms_lua::Value::Boolean(v & 0xff != 0),
        "CName" => bwms_lua::Value::UserData(lua.create_userdata(CName(v))?),
        "TweakDBID" => bwms_lua::Value::UserData(lua.create_userdata(TweakDBID(v.to_le_bytes()))?),
        _ => bwms_lua::Value::Integer(v as i64),
    })
}

/// `TweakDB:SetFlat(path, valor)`: o valor vira o texto de mod que o escritor converte pelo tipo do flat; com `atualiza`, o record dono do
/// flat (o path sem o último campo) é atualizado.
unsafe fn gravar_flat(path: &str, v: &bwms_lua::Value, atualiza: bool) -> bool {
    let Some(texto) = texto_de_flat(v) else { return false };
    let ok = crate::tweakdb::escritor::gravar_texto(bwms_primitives::tweak_db_id(path), &texto);
    crate::veredito_dev!("[lua] SetFlat {path} = {texto}: ok={ok}");
    if ok && atualiza {
        if let Some((record, _)) = path.rsplit_once('.') {
            crate::tweakdb::update_record_by_id(bwms_primitives::tweak_db_id(record));
        }
    }
    ok
}

/// Valor Lua no texto de mod do escritor de flat: tabela-sequência vira lista `[a, b]`.
fn texto_de_flat(v: &bwms_lua::Value) -> Option<String> {
    Some(match v {
        bwms_lua::Value::Boolean(b) => b.to_string(),
        bwms_lua::Value::Integer(i) => i.to_string(),
        bwms_lua::Value::Number(n) => n.to_string(),
        bwms_lua::Value::String(s) => s.to_str().ok()?.to_string(),
        bwms_lua::Value::UserData(ud) => {
            if let Ok(c) = ud.borrow::<CName>() {
                crate::cname::resolve_cname(c.0)
            } else if let Ok(t) = ud.borrow::<TweakDBID>() {
                format!("{:#x}", u64::from_le_bytes(t.0))
            } else {
                return None;
            }
        }
        bwms_lua::Value::Table(t) => {
            let itens = t.sequence_values::<bwms_lua::Value>().map(|e| e.ok().and_then(|e| texto_de_flat(&e))).collect::<Option<Vec<_>>>()?;
            bwms_primitives::operacoes::lista_em_texto(&itens)
        }
        _ => return None,
    })
}

const PASTA_DOS_MODS_LUA: &str = "bin/x64/plugins/cyber_engine_tweaks/mods";

/// Pastas dos mods Lua (a que tem `init.lua`): as da árvore do jogo e as de `Mods/<mod>/`, onde o usuário solta o zip do mod. Nome repetido carrega
/// uma vez, a da árvore do jogo primeiro.
fn pastas_de_mods() -> Vec<std::path::PathBuf> {
    let Some(g) = crate::arquivos_do_jogo::game_dir_from_dylib() else { return Vec::new() };
    let jogo = std::path::Path::new(&g);
    let mut raizes = vec![jogo.join(PASTA_DOS_MODS_LUA)];
    if let Ok(rd) = std::fs::read_dir(jogo.join("Mods")) {
        let mut mods: Vec<_> = rd.flatten().map(|e| e.path().join(PASTA_DOS_MODS_LUA)).collect();
        mods.sort();
        raizes.extend(mods);
    }
    let mut pastas: Vec<std::path::PathBuf> = Vec::new();
    for raiz in raizes {
        let Ok(rd) = std::fs::read_dir(raiz) else { continue };
        let mut daqui: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.join("init.lua").is_file()).collect();
        daqui.sort();
        for p in daqui {
            if !pastas.iter().any(|q| q.file_name() == p.file_name()) {
                pastas.push(p);
            }
        }
    }
    pastas
}

/// Carrega o `init.lua` de cada pasta de [`pastas_de_mods`] e dispara `onTweak` (os mods o esperam no load do TweakDB). Devolve quantos carregaram.
unsafe fn carregar_mods() -> usize {
    let pastas = pastas_de_mods();
    let mut n = 0;
    for p in &pastas {
        let nome = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::read_to_string(p.join("init.lua")) {
            Ok(src) => {
                run_mod(&nome, &src, p);
                n += 1;
                crate::veredito_dev!("[lua] mod carregado: {nome}");
            }
            Err(e) => crate::log(&format!("[lua] {nome}: init.lua ilegível ({e})")),
        }
    }
    if n > 0 {
        run_event("onTweak");
    }
    n
}

/// Ciclo dos mods Lua no tick (thread do jogo): carrega os mods na 1ª vez com os flats do TweakDB no ar ([`crate::tweakdb::carregado`]; o `onTweak` grava flats),
/// registra os `Observe` pedidos, despacha as teclas e, com o V no mundo, dispara `onInit` (uma vez) e `onUpdate`.
pub(crate) unsafe fn tick(reg: &Registry, em_gameplay: bool) {
    static CARREGADOS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !CARREGADOS.load(std::sync::atomic::Ordering::Relaxed) {
        if !crate::tweakdb::carregado() {
            return;
        }
        CARREGADOS.store(true, std::sync::atomic::Ordering::Relaxed);
        let n = carregar_mods();
        crate::veredito_dev!("[lua] {n} mod(s) Lua carregado(s)");
    }
    // `onInit` no 1º player nascido, não na carga; o `GameSession` dos mods conta com o V.
    static INICIADOS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !crate::current_player().is_null() && !INICIADOS.swap(true, std::sync::atomic::Ordering::Relaxed) {
        run_event("onInit");
        crate::veredito_dev!("[lua] onInit no 1º V no mundo");
    }
    if observar::has_pending() {
        observar::drain_pending(reg);
    }
    despachar_teclas();
    #[cfg(feature = "devtools")]
    if em_gameplay {
        medir_framework_no_motor();
    }
    // `onUpdate` no ritmo de quadro (no máximo a cada 16 ms): o tick roda por contagem de chamadas do executor (`TICK_EVERY`), não por
    // quadro, e cada `onUpdate` segura a trava do Lua que o `onDraw` precisa.
    static PROXIMO_UPDATE: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    let agora = std::time::Instant::now();
    let na_hora = PROXIMO_UPDATE.lock().map(|mut p| {
        let sim = p.is_none_or(|t| agora >= t);
        if sim {
            *p = Some(agora + std::time::Duration::from_millis(16));
        }
        sim
    });
    if em_gameplay && na_hora.unwrap_or(false) {
        run_event("onUpdate");
    }
}

#[cfg(all(test, feature = "devtools"))]
mod testes_com_mods_reais;

#[cfg(test)]
mod testes_da_api_lua {
    use super::*;

    /// Os globais que os mods chamam em runtime existem depois do `setup_api` (o `ImGui` é a etapa R2). Os tipos do jogo (`Vector4`,
    /// `Quaternion`, `EulerAngles`) e as funções globais vêm do RTTI pelo metatable dos globais: só existem com o jogo.
    #[test]
    fn globais_que_os_mods_chamam_existem() {
        let lua = bwms_lua::Lua::new();
        setup_api(&lua).expect("setup_api");
        let faltam: Vec<&str> = [
            "CName", "TweakDBID", "Game", "GetMod", "GetSingleton", "GetVersion",
            "Enum", "EnumValueToName", "IsDefined", "ModArchiveExists", "Observe", "Override", "NewObject", "TweakDB", "spdlog",
            "registerForEvent", "registerHotkey", "registerInput", "ImGuiCond",
        ]
        .into_iter()
        .filter(|n| lua.globals().get::<bwms_lua::Value>(*n).map_or(true, |v| v.is_nil()))
        .collect();
        assert!(faltam.is_empty(), "faltam: {faltam:?}");
    }

    /// Um mod não sai da própria pasta: pelos globais (`_G`, `getfenv()`, `package.loaded`) não há `io`, `require` de arquivo nem `os`
    /// além de relógio e data, o metatable dos globais não se entrega, e o `io.open` do ambiente recusa caminho fora da pasta.
    #[test]
    fn mod_nao_alcanca_arquivo_fora_da_pasta() {
        let lua = bwms_lua::Lua::new();
        setup_api(&lua).expect("setup_api");
        let dir = std::env::temp_dir().join(format!("bwms-sandbox-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("pasta do mod");
        let env = ambiente_do_mod(&lua, &dir).expect("ambiente");
        let codigo = br#"
            local g = getfenv()
            return _G.io == nil, g.io == nil, package.loaded.io == nil, g.require == nil,
                   os.getenv == nil, os.execute == nil, _G.os.remove == nil, getmetatable(_G) == false,
                   io.open("/etc/hosts") == nil, io.open("dentro.txt", "w") ~= nil
        "#
        .to_vec();
        let (f, _) = carregar_no_ambiente(&lua, codigo, "=teste".into(), &env).expect("carregar");
        let bwms_lua::Value::Function(f) = f else { panic!("o trecho de teste não compilou") };
        let r: bwms_lua::MultiValue = f.call(()).expect("rodar");
        let r: Vec<bool> = r.into_iter().map(|v| matches!(v, bwms_lua::Value::Boolean(true))).collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(r, vec![true; 10]);
    }

    /// `TweakDBID.new(path)`, `TweakDBID.new(base, sufixo)` e `TweakDBID(path)` dão o id que os mods esperam (há mod que monta slot e item assim).
    #[test]
    fn tweakdbid_na_forma_dos_mods() {
        let lua = bwms_lua::Lua::new();
        setup_api(&lua).expect("setup_api");
        let t: bwms_lua::Table = lua.globals().get("TweakDBID").expect("TweakDBID");
        let novo: bwms_lua::Function = t.get("new").expect("new");
        let id = |ud: bwms_lua::AnyUserData| u64::from_le_bytes(ud.borrow::<TweakDBID>().expect("TweakDBID").0);
        let base = bwms_primitives::tweak_db_id("AttachmentSlots.Head");
        assert_eq!(id(novo.call("AttachmentSlots.Head").expect("new(path)")), base);
        let chamada: bwms_lua::Function = t.metatable().expect("metatable").get("__call").expect("__call");
        assert_eq!(id(chamada.call((t.clone(), "AttachmentSlots.Head")).expect("TweakDBID(path)")), base);
        let derivado = bwms_primitives::tweak_db_id_derive(bwms_primitives::tweak_db_id("AttachmentSlots"), ".Head");
        assert_eq!(id(novo.call(("AttachmentSlots", ".Head")).expect("new(base, sufixo)")), derivado);
    }

    /// `tostring` de `CName` e `TweakDBID` no texto que os mods esperam (há mod que recorta o nome do `CName` entre `--[[ ` e ` --]]`).
    #[test]
    fn tostring_na_forma_dos_mods() {
        let lua = bwms_lua::Lua::new();
        let tostring: bwms_lua::Function = lua.globals().get("tostring").expect("tostring");
        let h = crate::cname::intern("TPP_Body");
        assert_eq!(
            tostring.call::<String>(lua.create_userdata(CName(h)).expect("ud")).expect("tostring"),
            format!("ToCName{{ hash_lo = 0x{:08X}, hash_hi = 0x{:08X} --[[ TPP_Body --]] }}", h as u32, (h >> 32) as u32)
        );
        let id = bwms_primitives::tweak_db_id("AttachmentSlots.Head");
        assert_eq!(
            tostring.call::<String>(lua.create_userdata(TweakDBID(id.to_le_bytes())).expect("ud")).expect("tostring"),
            format!("ToTweakDBID{{ hash = 0x{:08X}, length = 20 }}", id as u32)
        );
    }

    #[test]
    fn caminho_do_mod_nao_sai_da_pasta() {
        let d = std::path::Path::new("/m");
        assert_eq!(caminho_no_mod(d, "sessions/a.lua"), Some(std::path::PathBuf::from("/m/sessions/a.lua")));
        assert_eq!(caminho_no_mod(d, "../x"), None);
        assert_eq!(caminho_no_mod(d, "/etc/passwd"), None);
    }
}

/// O `db` de um mod ([`banco::Banco`]) com a API de banco que os mods chamam: `exec` devolve 0 (ok) ou 1 (erro, logado); `rows` itera
/// linhas como array, `nrows` como tabela por nome de coluna, `urows` com os valores soltos.
struct BancoDoMod(std::sync::Arc<Mutex<banco::Banco>>, String);

fn valor_do_banco(lua: &bwms_lua::Lua, v: &banco::Valor) -> bwms_lua::Result<bwms_lua::Value> {
    Ok(match v {
        banco::Valor::Nulo => bwms_lua::Value::Nil,
        banco::Valor::Inteiro(n) => bwms_lua::Value::Integer(*n),
        banco::Valor::Real(r) => bwms_lua::Value::Number(*r),
        banco::Valor::Texto(s) => bwms_lua::Value::String(lua.create_string(s)?),
    })
}

impl BancoDoMod {
    fn consultar(&self, sql: &str) -> banco::Linhas {
        let mut b = self.0.lock().unwrap_or_else(|e| e.into_inner());
        b.consultar(sql).unwrap_or_else(|e| {
            crate::log(&format!("[lua] {} db: {e} ({sql})", self.1));
            banco::Linhas { colunas: Vec::new(), linhas: Vec::new() }
        })
    }

    /// Iterador Lua sobre as linhas, cada uma montada por `montar`.
    fn iterador(
        lua: &bwms_lua::Lua,
        l: banco::Linhas,
        montar: fn(&bwms_lua::Lua, &[String], &[banco::Valor]) -> bwms_lua::Result<bwms_lua::MultiValue>,
    ) -> bwms_lua::Result<bwms_lua::Function> {
        let proxima = std::cell::Cell::new(0usize);
        lua.create_function(move |lua, _: bwms_lua::MultiValue| {
            let i = proxima.get();
            proxima.set(i + 1);
            match l.linhas.get(i) {
                Some(linha) => montar(lua, &l.colunas, linha),
                None => Ok(bwms_lua::MultiValue::from_vec(vec![bwms_lua::Value::Nil])),
            }
        })
    }
}

impl bwms_lua::UserData for BancoDoMod {
    fn add_methods<M: bwms_lua::UserDataMethods<Self>>(m: &mut M) {
        m.add_method("exec", |_, b, sql: String| {
            let mut g = b.0.lock().unwrap_or_else(|e| e.into_inner());
            Ok(match g.exec(&sql) {
                Ok(()) => 0,
                Err(e) => {
                    crate::log(&format!("[lua] {} db: {e}", b.1));
                    1
                }
            })
        });
        m.add_method("rows", |lua, b, sql: String| {
            BancoDoMod::iterador(lua, b.consultar(&sql), |lua, _, linha| {
                let t = lua.create_table()?;
                for (i, v) in linha.iter().enumerate() {
                    t.set(i + 1, valor_do_banco(lua, v)?)?;
                }
                Ok(bwms_lua::MultiValue::from_vec(vec![bwms_lua::Value::Table(t)]))
            })
        });
        m.add_method("nrows", |lua, b, sql: String| {
            BancoDoMod::iterador(lua, b.consultar(&sql), |lua, colunas, linha| {
                let t = lua.create_table()?;
                for (c, v) in colunas.iter().zip(linha) {
                    t.set(c.as_str(), valor_do_banco(lua, v)?)?;
                }
                Ok(bwms_lua::MultiValue::from_vec(vec![bwms_lua::Value::Table(t)]))
            })
        });
        m.add_method("urows", |lua, b, sql: String| {
            BancoDoMod::iterador(lua, b.consultar(&sql), |lua, _, linha| {
                linha.iter().map(|v| valor_do_banco(lua, v)).collect::<bwms_lua::Result<Vec<_>>>().map(bwms_lua::MultiValue::from_vec)
            })
        });
        m.add_method("close", |_, _, ()| Ok(0));
    }
}

/// Erro de mod Lua: logado até 5 vezes por origem (o `onUpdate` e o `onDraw` de um mod quebrado erram todo quadro) e o 1º de cada origem
/// no canal de veredito.
fn erro_de_mod(origem: &str, e: &dyn std::fmt::Display) {
    static ERROS: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());
    let mut g = ERROS.lock().unwrap_or_else(|e| e.into_inner());
    let n = match g.iter_mut().find(|(o, _)| o == origem) {
        Some((_, n)) => {
            *n += 1;
            *n
        }
        None => {
            g.push((origem.to_string(), 1));
            1
        }
    };
    if n <= 5 {
        crate::log(&format!("[lua] {origem} ERRO: {e}"));
    }
    if n == 1 {
        crate::veredito_dev!("[lua] erro em {origem}: {e}");
    }
}

/// Classe de objeto (deriva de `IScriptable`), que `.new` instancia como handle; struct é valor.
unsafe fn e_classe_de_objeto(ty: *mut c_void) -> bool {
    let alvo = crate::cname::cname("IScriptable");
    let mut c = ty;
    for _ in 0..64 {
        if !crate::rtti::sane(c) {
            return false;
        }
        if crate::rtti::type_name_getname(c) == alvo {
            return true;
        }
        if !crate::memoria::is_readable(c as *const c_void, 0x18) {
            return false;
        }
        c = ((c as *const u8).add(0x10) as *const *mut c_void).read_unaligned(); // CClass+0x10 = pai
    }
    false
}

/// Objeto novo da classe `ty` como [`Handle`] (o `NewObject` com o tipo já resolvido).
unsafe fn novo_objeto(lua: &bwms_lua::Lua, ty: *mut c_void) -> bwms_lua::Value {
    new_object_handle(lua, &crate::cname::resolve_cname(crate::rtti::type_name_getname(ty)))
}

/// Callback do `registerHotkey` de id `id`, com o erro do mod no canal.
fn chamar_hotkey(lua: &bwms_lua::Lua, id: &str) -> bool {
    let hk = HOTKEYS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(rk) = hk.as_ref().and_then(|m| m.get(id)) else { return false };
    let Ok(f) = lua.registry_value::<bwms_lua::Function>(rk) else { return false };
    if let Err(e) = f.call::<()>(()) {
        erro_de_mod("registerHotkey", &e);
    }
    true
}

/// `registerHotkey` pelo id, na thread do jogo (o comando de dev `luatecla <id>`: prova sem tecla). `false` sem o id ou com o Lua ocupado.
pub(crate) unsafe fn disparar_hotkey(id: &str) -> bool {
    let Ok(g) = LUA.try_lock() else { return false };
    g.as_ref().is_some_and(|s| chamar_hotkey(&s.0, id))
}

/// Medida do `CGameFramework` no `CBaseEngine` do Mac (no Windows, `engine+0x308`, com o `GameInstance*` em `+0x10`): com o V no
/// mundo, o offset do motor cujo ponteiro tem em `+0x10` o `GameInstance` do V. Uma vez por processo, no canal de veredito.
#[cfg(feature = "devtools")]
unsafe fn medir_framework_no_motor() {
    static FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if FEITO.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let (Some(gi), m) = (crate::jogo::sessao::game_instance_ptr(), crate::jogo::motor::motor()) else { return };
    if m.is_null() {
        return;
    }
    FEITO.store(true, std::sync::atomic::Ordering::Relaxed);
    let achados: Vec<String> = (0..0x600usize)
        .step_by(8)
        .filter(|&k| {
            let p = ((m as *const u8).add(k) as *const u64).read_unaligned() as *const u8;
            crate::rtti::sane(p as *mut c_void)
                && crate::memoria::is_readable(p.add(0x10) as *const c_void, 8)
                && (p.add(0x10) as *const u64).read_unaligned() == gi as u64
        })
        .map(|k| format!("+{k:#x}"))
        .collect();
    crate::veredito_dev!(
        "[lua] VEREDITO-FRAMEWORK GameInstance {gi:p}; framework no motor em: {}; pelo motor {:?}, igual={}",
        if achados.is_empty() { "nenhum".into() } else { achados.join(" ") },
        crate::jogo::motor::game_instance(),
        crate::jogo::motor::game_instance() == Some(gi)
    );
}

/// Nome de um `CName`: o pool interno do BWMS, senão o do motor com o jogo no ar; `None` sem nome.
fn nome_do_cname(h: u64) -> Option<String> {
    crate::cname::name_of(h).or_else(|| crate::registry().is_some().then(|| crate::cname::resolve_cname(h)).filter(|s| !s.is_empty()))
}

/// Quadros em que o `onDraw` rodou e em que pulou por não pegar a trava do Lua em 3 ms (o `luadraw` de dev mostra).
pub(crate) static QUADROS_DO_ONDRAW: [std::sync::atomic::AtomicU64; 2] = [std::sync::atomic::AtomicU64::new(0), std::sync::atomic::AtomicU64::new(0)];

/// `Observe` de prova (o comando de dev `luaobserva <classe> <método>`): callback vazio feito no Rust; o 1º disparo sai no canal de veredito.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn observar_de_prova(classe: &str, metodo: &str) -> bool {
    let Ok(g) = LUA.try_lock() else { return false };
    let Some(s) = g.as_ref() else { return false };
    let lua = &s.0;
    let chave = lua.create_function(|_, _: bwms_lua::MultiValue| Ok(())).and_then(|f| lua.create_registry_value(f));
    match chave {
        Ok(k) => {
            observar::queue(classe.to_string(), metodo.to_string(), observar::Kind::Before, k);
            true
        }
        Err(_) => false,
    }
}
