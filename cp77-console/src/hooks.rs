//! hooks.rs — Observe / ObserveAfter / Override: o CORAÇÃO do CET.
//!
//! Roteia chamadas de método do jogo para callbacks Lua. O insight: o executor
//! universal (@0x2173120) é chamado com `(CBaseFunction*, ctx, frame, res, retType)`,
//! e `rtti::resolve_func` devolve EXATAMENTE esse mesmo `CBaseFunction*` — então
//! casamos por ponteiro de função. A sonda já tem o hook do executor (na thread do
//! jogo); ela chama `cp77_obs_before/after` SÓ para as funcs vigiadas (o teste
//! `watched.has(func)` é feito no hook, barato; só funcs com hook entram aqui).
//!
//! Registro é em 2 tempos (o mod chama Observe em onInit, fora da thread do jogo):
//! `queue()` empilha o pedido; `drain_pending()` (no cp77_tick, thread do jogo, REG
//! vivo) resolve a função e registra. A lista de ptrs vigiados vai pra /tmp/cp77-watch.txt
//! pra sonda sincronizar o `watched` Set.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use mlua::RegistryKey;

use crate::rtti::{self, Registry};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Before,
    After,
    Override,
    Suppress,
}

struct Pending {
    class: String,
    method: String,
    kind: Kind,
    cb: RegistryKey,
}

#[derive(Default)]
struct Entry {
    before: Vec<RegistryKey>,
    after: Vec<RegistryKey>,
    over: Vec<RegistryKey>,
    suppress: bool,
    name: String,
    /// CNames dos NOMES DE TIPO das classes registradas p/ este método (OR). Lidos de
    /// `class_by_name(class).type_name_hash` no drain. Filtra o dispatch: dispara se `class_of(ctx)`
    /// DERIVA de QUALQUER uma (parent-walk casando o CName). VAZIO, ou contendo 0 (uma classe não
    /// resolveu), = casa QUALQUER classe (fallback p/ hooks por nome puro).
    ///
    /// É Vec (não u64) porque o MESMO método pode ser vigiado em classes DIFERENTES (ex.: o
    /// NativeSettings observa `OnMenuItemActivated` em PauseMenuGameController E em
    /// gameuiMenuItemListGameController) — a Entry é indexada pelo CName do método, então as duas
    /// compartilham. Com u64 só a 1ª classe valia e a ativação do menu principal era pulada.
    ///
    /// Por que CName e não o ponteiro CClass*: (a) casar por type-name sobe a cadeia de heranças,
    /// então pega overrides em subclasses (o jogo pode chamar via func ptr diferente numa
    /// derivada — casar só ponteiro PERDE essas chamadas); (b) evita guardar `*mut c_void`
    /// num `static` (não-Send) e qualquer risco de pointer stale.
    class_filters: Vec<u64>,
}

/// True se a Entry deve disparar p/ este ctx: filtro vazio ou com sentinela 0 = casa qualquer;
/// senão `class_of(ctx)` precisa derivar de ALGUMA das classes registradas (OR).
unsafe fn entry_matches_ctx(filters: &[u64], ctx: *mut c_void) -> bool {
    if filters.is_empty() || filters.contains(&0) {
        return true;
    }
    filters.iter().any(|&c| ctx_derives_from(ctx, c))
}

static PENDING: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
/// Indexado pelo CName do MÉTODO (func+0x10), não pelo ponteiro da função: o jogo
/// pode chamar o método numa classe/overload diferente da que resolvemos (ponteiro
/// diferente, MESMO CName). Casar por CName pega todos os overloads.
static WATCHED: Mutex<Option<HashMap<u64, Entry>>> = Mutex::new(None);
static DIRTY: AtomicBool = AtomicBool::new(false);
/// fast-path: se 0, a sonda nem precisa olhar o Set (e drain_pending pode pular).
static WATCH_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Override-WRAPPED flag-based: o callback chama `wrapped()` → seta true → a ORIGINAL roda
/// no FRAME REAL (não suprimimos, sem re-invoke sintético). Se o callback NÃO chamar
/// `wrapped()` (override total), fica false → suprimimos a original.
pub static WRAPPED_CALLED: AtomicBool = AtomicBool::new(false);

/// Chamado pelos bindings Lua (Observe/ObserveAfter/Override) — só empilha.
pub fn queue(class: String, method: String, kind: Kind, cb: RegistryKey) {
    if let Ok(mut p) = PENDING.lock() {
        p.push(Pending { class, method, kind, cb });
    }
}

pub fn has_pending() -> bool {
    PENDING.lock().map(|p| !p.is_empty()).unwrap_or(false)
}

/// Drena os pedidos na THREAD DO JOGO (cp77_tick): resolve cada (classe,método),
/// acha o `CBaseFunction*` (== arg0 do executor) e registra. Publica a lista de
/// ptrs vigiados em /tmp/cp77-watch.txt pra sonda ler.
pub unsafe fn drain_pending(reg: &Registry) {
    let mut pend = match PENDING.lock() {
        Ok(p) => p,
        Err(_) => return,
    };
    if pend.is_empty() {
        return;
    }
    let mut w = WATCHED.lock().unwrap_or_else(|e| e.into_inner());
    let map = w.get_or_insert_with(HashMap::new);
    for req in pend.drain(..) {
        // chave = CName do método (pega qualquer classe/overload que o jogo chame).
        let key = crate::cname::cname(&req.method);
        let e = map.entry(key).or_insert_with(Entry::default);
        if e.name.is_empty() {
            e.name = format!("{}.{}", req.class, req.method);
        }
        // Filtro de classe (OR): resolve a CClass* da classe registrada e ACUMULA o CName do
        // NOME DO TIPO dela no Vec. No dispatch, dispara se class_of(ctx) deriva de ALGUMA.
        // Se class_by_name não achar (nome genérico/errado) OU type_name vier 0, empilha 0
        // (sentinela "casa qualquer") — preserva o fallback de hooks por nome puro. Cada Observe
        // de uma classe distinta do MESMO método soma seu filtro (era o bug do OnMenuItemActivated:
        // PauseMenu travava e a gameui — do menu principal — era pulada).
        {
            let cls = reg.class_by_name(&req.class);
            let tn = if cls.is_null() { 0 } else { rtti::type_name_hash(cls) };
            if !e.class_filters.contains(&tn) {
                e.class_filters.push(tn);
            }
        }
        match req.kind {
            Kind::Before => e.before.push(req.cb),
            Kind::After => e.after.push(req.cb),
            Kind::Override => e.over.push(req.cb),
            // Suppress: o cb roda ANTES (no lugar da original) e a original é PULADA
            // (dispatch_before devolve true → a sonda pula o executor desta chamada).
            Kind::Suppress => {
                e.suppress = true;
                e.before.push(req.cb);
            }
        }
        // resolve_func só pra confirmar/logar (o casamento em runtime é por CName) — só em dev
        // (em jogo normal nem o resolve nem o log rodam: registro silencioso).
        if crate::dev_mode() {
            let resolved = rtti::resolve_func(reg, &req.class, &req.method).is_some();
            crate::log(&format!(
                "[hook] vigiando {} (cname {:#018x}){}",
                e.name,
                key,
                if resolved { "" } else { " [classe não resolvida — casa por nome]" }
            ));
        }
        DIRTY.store(true, Ordering::Relaxed);
    }
    WATCH_COUNT.store(map.len(), Ordering::Relaxed);
    // O `cp77-watch.txt` era export pra SONDA ANTIGA (morta) ler o set vigiado — sem leitor hoje
    // (o dispatch in-process usa WATCH_COUNT + o map direto). Export + log só em dev: em jogo
    // normal o registro dos ~21 hooks do NativeSettings roda CALADO. `swap` roda antes do `&&`
    // (curto-circuito) → DIRTY sempre reseta, sem mudança de semântica.
    if DIRTY.swap(false, Ordering::Relaxed) && crate::dev_mode() {
        let mut s = String::new();
        for k in map.keys() {
            s.push_str(&format!("{:016x}\n", k));
        }
        let _ = std::fs::write("/tmp/cp77-watch.txt", s);
        crate::log(&format!("[hook] {} método(s) vigiado(s) publicado(s)", map.len()));
    }
}

/// Dispara os callbacks BEFORE (Observe) + OVERRIDE de uma função vigiada, na thread
/// do jogo. `this` = ctx (o objeto em que o método foi chamado). Retorna se deve
/// SUPRIMIR a original (override que pediu) — por ora sempre false (Observe puro).
/// Para o SELF-BOOT (sem JS/sonda): lê o CName do método de `func`
/// (shortName@CBaseFunction+0x10) e, se ele está vigiado, roda o `before`,
/// devolvendo `(suppress, mcname)`. Barato no caso comum: sai cedo se ninguém
/// vigia (WATCH_COUNT==0) ou se o método não está no set — só então dispara o
/// dispatch. É o que o cp77-probe.js fazia em JS, agora em Rust direto.
/// CAMINHO A (dev): quando o handler de clique de menu (`OnMenuItemActivated`/
/// `HandleMenuItemActivate`) passa pelo executor, loga a CLASSE do ctx + se está vigiado —
/// MESMO que o watch/filtro de classe o barre depois. Responde de uma vez: o clique dispara?
/// em que classe? o filtro pula? (era o bloqueio do A — ver notes/ui-menu-system.md).
unsafe fn trace_menu_click(mcname: u64, ctx: *mut c_void, is_watched: bool) {
    use std::sync::OnceLock;
    static H: OnceLock<(u64, u64)> = OnceLock::new();
    let (oma, hma) = *H.get_or_init(|| {
        (crate::cname::cname("OnMenuItemActivated"), crate::cname::cname("HandleMenuItemActivate"))
    });
    if mcname != oma && mcname != hma {
        return;
    }
    let cls = rtti::class_of(ctx);
    let cn = if cls.is_null() { 0 } else { rtti::type_name_hash(cls) };
    crate::trace(&format!(
        "[A] {} ctx_class={} watched={}",
        crate::cname::resolve_cname(mcname),
        crate::cname::resolve_cname(cn),
        is_watched
    ));
}

pub unsafe fn watched_before(
    func: *mut c_void,
    ctx: *mut c_void,
    frame: *mut c_void,
    res: *mut c_void,
) -> (bool, u64) {
    if WATCH_COUNT.load(Ordering::Relaxed) == 0 || func.is_null() {
        return (false, 0);
    }
    if !crate::gum::is_readable(func as *const c_void, 0x18) {
        return (false, 0);
    }
    let mcname = core::ptr::read_unaligned((func as *const u8).add(0x10) as *const u64);
    let is_watched = match WATCHED.try_lock() {
        Ok(g) => g.as_ref().is_some_and(|m| m.contains_key(&mcname)),
        Err(_) => return (false, 0),
    };
    if !is_watched {
        return (false, 0);
    }
    (dispatch_before(mcname, func, ctx, frame, res), mcname)
}

/// Espelha o `cp77_obs_after`: roda after + override (para a função vigiada).
pub unsafe fn watched_after(mcname: u64, ctx: *mut c_void, res: *mut c_void) {
    dispatch_after(mcname, ctx);
    dispatch_override(mcname, ctx, res);
}

pub unsafe fn dispatch_before(
    mcname: u64,
    func: *mut c_void,
    ctx: *mut c_void,
    frame: *mut c_void,
    res: *mut c_void,
) -> bool {
    // try_lock: se reentrante (um callback disparou outro método vigiado), pula.
    let w = match WATCHED.try_lock() {
        Ok(w) => w,
        Err(_) => return false,
    };
    if let Some(map) = w.as_ref() {
        if let Some(e) = map.get(&mcname) {
            // FILTRO DE CLASSE: se a classe alvo resolveu (class_cname != 0), só dispara quando
            // class_of(ctx) DERIVA dela. Mata a poluição de overload (AddMenuItem/Setup homônimos
            // de OUTRAS classes, controllers transientes errados). Fallback: class_cname==0 →
            // casa qualquer classe (hooks por nome puro seguem funcionando). Tudo gum-safe.
            // CAMINHO A — relaxação TARGETED do filtro: os métodos de ATIVAÇÃO de item de menu
            // (OnMenuItemActivated/HandleMenuItemActivate) BYPASSAM o filtro de classe. Justificativa:
            // o callback do NativeSettings JÁ checa `data.label=="Mods"` sozinho — over-fire é
            // INOFENSIVO (só liga/desliga fromMods pelo label) — e o filtro pode pular ctx VÁLIDO se
            // `class_of` cair no fallback e devolver a classe BASE (rtti.rs:549). Não toca os demais
            // métodos (a proteção de overload segue onde importa).
            let menu_activate = {
                static M: std::sync::OnceLock<(u64, u64)> = std::sync::OnceLock::new();
                let (a, b) = *M.get_or_init(|| {
                    (
                        crate::cname::cname("OnMenuItemActivated"),
                        crate::cname::cname("HandleMenuItemActivate"),
                    )
                });
                mcname == a || mcname == b
            };
            if !menu_activate && !entry_matches_ctx(&e.class_filters, ctx) {
                crate::trace(&format!("{}:skip (classe de ctx não deriva da alvo)", e.name));
                return false;
            }
            crate::trace(&format!("{}:enter (before={} over={})", e.name, e.before.len(), e.over.len()));
            let params = rtti::read_params(func, frame);
            // DIAGNÓSTICO: tipo (CName do tipo) + valor cru de cada arg lido — pra ver o que
            // o AddMenuItem entrega como spawnEvent (CName? objeto?).
            {
                let desc: Vec<String> = params
                    .iter()
                    .map(|(raw, tc)| format!("(t={tc:#x} v={raw:#x})"))
                    .collect();
                crate::trace(&format!("{}:params[{}] {}", e.name, params.len(), desc.join(" ")));
            }
            for k in &e.before {
                crate::lua::call_hook(k, ctx, &params);
            }
            if e.over.is_empty() {
                crate::trace(&format!("{}:done-observe", e.name));
                return e.suppress;
            }
            // Override (CET) flag-based: `call_hook_override` chama o callback UMA vez com
            // (this, ...args, wrapped), captura se ele chamou `wrapped()` (→ WRAPPED_CALLED) E
            // o valor de retorno. Se for override-TOTAL (sem wrapped) de retorno POD de largura
            // conhecida, JÁ grava o `res` (aOut) com a largura certa e sinaliza suppress; senão
            // não toca `res`. Sem re-invoke sintético frágil; a original roda no frame REAL
            // quando não suprimimos.
            WRAPPED_CALLED.store(false, Ordering::Relaxed);
            let mut any_suppress_value = false;
            for k in &e.over {
                let (_w, sv) = crate::lua::call_hook_override(k, ctx, &params, func, res);
                any_suppress_value |= sv;
            }
            // SUPPRESS — SEGURO porque o nosso `exec_replacement` controla o retorno: no suppress
            // NÃO chama a original e devolve bool, sem mexer em PC/SP/canary.
            //   - VOID: o caller não lê o aOut → suprimir = idêntico a um Return real.
            //   - POD (Bool/Int/Float/...): `call_hook_override` JÁ gravou o aOut com largura
            //     correta (tipo de retorno conferido via GetName+GetSize) → suprimir é seguro.
            //   - value-returning NÃO-POD (classe/handle/string/array): NUNCA suprime → a original
            //     roda e o rewrite pós-original (`call_hook_ret`) cobre getters.
            // Override que chamou `wrapped()` quer a original → nunca suprime.
            let wrapped_called = WRAPPED_CALLED.load(Ordering::Relaxed);
            let void_ret = rtti::fn_returns_void(func);
            let suppress = e.suppress
                || (!wrapped_called && void_ret)
                || (!wrapped_called && any_suppress_value);
            crate::trace(&format!(
                "{}:cb-done (suppress={suppress}, wrapped={wrapped_called}, void={void_ret}, podret={any_suppress_value})",
                e.name
            ));
            return suppress;
        }
    }
    false
}

/// Dispara os callbacks AFTER (ObserveAfter) de um método vigiado (pós-original).
pub unsafe fn dispatch_after(mcname: u64, ctx: *mut c_void) {
    let w = match WATCHED.try_lock() {
        Ok(w) => w,
        Err(_) => return,
    };
    if let Some(map) = w.as_ref() {
        if let Some(e) = map.get(&mcname) {
            if !entry_matches_ctx(&e.class_filters, ctx) {
                return;
            }
            for k in &e.after {
                crate::lua::call_hook(k, ctx, &[]); // ObserveAfter: args = futuro
            }
        }
    }
}

/// True se a classe de `ctx` (via `class_of` = GetType, freeze-safe) DERIVA da classe cujo
/// type-name == `want` (CName). Sobe a cadeia de parents (CClass+0x10) comparando o CName do
/// nome do tipo (CClass+0x18, via `type_name_hash`) — mesmo padrão do `derives_from_iscriptable`
/// do rtti.rs, mas aqui partindo de um OBJETO. Cap de 64 níveis + tudo gum-checked dentro de
/// `class_of`/`type_name_hash` (nunca congela em handle stale). `want==0` nunca chega aqui
/// (o caller já curto-circuita), mas por segurança devolveria-se via comparação trivial.
unsafe fn ctx_derives_from(ctx: *mut c_void, want: u64) -> bool {
    let mut cls = rtti::class_of(ctx);
    let mut guard = 0;
    while !cls.is_null() && guard < 64 {
        guard += 1;
        if rtti::type_name_hash(cls) == want {
            return true;
        }
        // parent @ CClass+0x10. Leitura crua: type_name_hash/class_of já validaram legibilidade
        // do nível atual; o próximo nível é revalidado no topo do loop por type_name_hash.
        if !crate::gum::is_readable(cls as *const c_void, 0x18) {
            break;
        }
        cls = unsafe { *((cls as *const u8).add(0x10) as *const *mut c_void) };
    }
    false
}

/// Override (1ª versão = return-rewrite): roda os callbacks Override pós-original e,
/// se o callback retornar um valor, REESCREVE o buffer de retorno `res` (16B) com
/// ele. Cobre override de getters/queries (sem efeito colateral); suprimir a
/// original (p/ métodos com efeito colateral) precisa de replace-no-executor (futuro).
pub unsafe fn dispatch_override(mcname: u64, ctx: *mut c_void, res: *mut c_void) {
    let w = match WATCHED.try_lock() {
        Ok(w) => w,
        Err(_) => return,
    };
    if let Some(map) = w.as_ref() {
        if let Some(e) = map.get(&mcname) {
            if !entry_matches_ctx(&e.class_filters, ctx) {
                return;
            }
            for k in &e.over {
                crate::lua::call_hook_ret(k, ctx, res);
            }
        }
    }
}
