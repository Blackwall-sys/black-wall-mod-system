//! sobrepor_funcao — sobreposição de função do BWMS: hook nativo (Rust, não-Lua) de QUALQUER
//! função redscript (nativa OU scriptada) por classe+método, com composição Before/After/
//! Replace multi-mod. Prep de 2026-08-11 (`PENDENCIAS-UNIFICADAS.md`).
//!
//! # Mecanismo (sem JIT nenhum)
//!
//! O executor universal do BWMS (`inicio::boot::exec_replacement`, hook JÁ INSTALADO e provado desde
//! 2026-06/2026-07) intercepta o equivalente ARM64/macOS de `CBaseFunction::InternalExecute` — o
//! MESMO ponto de despacho por onde passa TODA chamada de função redscript, nativa OU scriptada,
//! ANTES do motor decidir internamente qual caminho seguir. `exec_replacement` já recebe
//! `(func: CBaseFunction*, ctx, frame, aOut, a4)` — a identidade da função chamada. Como o BWMS
//! intercepta o despachante universal, **"interceptar a função X" aqui é só
//! registrar `X -> callback` numa tabela e consultar essa tabela dentro do hook já existente** —
//! zero swap de struct, zero geração de código de máquina nova. A técnica de tabela-por-ponteiro
//! já está em produção pro caso "nativa FORJADA por nós" (`register::NATIVE_ROUTES`/`route_native`,
//! consultado no MESMO `exec_replacement`); este módulo generaliza o mesmo padrão pra QUALQUER
//! `CBaseFunction*` já EXISTENTE (vanilla ou de outro mod), resolvido via `rtti::resolve_func`
//! (a mesma via já usada pelo CallbackSystem e pela Reflection). Casa por PONTEIRO (`ResolvedFn::func`, exato para "1 classe
//! concreta, 1 método") e usa `register::NativeHandler` como callback, a mesma assinatura que plugins já implementam para
//! `register_native`/`register_method`.
//!
//! # Composição multi-mod (V1)
//!
//! Um `next` por `Override` registrado, que ou chama o PRÓXIMO override ou (no fim da
//! cadeia) a função real, deixaria um mod decidir "delego pro próximo" vs "substituo por
//! completo" caso a caso. Implementar isso aqui exigiria closures Rust capturando o restante da
//! cadeia (viável, mas mais estado + mais superfície de bug em código ainda não testado ao vivo).
//! **V1 usa uma regra mais simples e determinística: `Before` e `After` disparam TODOS os
//! registrados, em ordem de registro; `Replace` só o ÚLTIMO registrado roda** (mod que
//! registrou por último "vence"). Isso cobre o
//! caso prático dominante (1 mod substitui, N mods observam antes/depois) sem a
//! complexidade de continuação. Evoluir pra um `next()`
//! genuíno depois é troca de ESTRUTURA DE DADOS/ORDEM DE CHAMADA, não de PRIMITIVA — pode ser
//! feito sem tocar a parte arriscada (o hook em si já está resolvido).
//!
//! # Status
//!
//! Provado ao vivo em 2026-08-11 (prova de 2026-08-11 da sobreposição de função): Before, After e
//! Replace num método 100% redscript, chamado sem parâmetros. Não provado: método vanilla, 2+ registrantes na mesma fase,
//! e reentrância (um Replace que chama `rtti::call_func` para a original, aninhando o executor). Hoje só os comandos de
//! dev `fnoverridetest`/`fnoverridereplace` registram alvo; a API C de plugin não expõe este módulo.
//!
//! Um `Replace` recebe o frame com os parâmetros ainda no bytecode e sem `set_current_native_func`: o handler tem de
//! consumir os parâmetros com o descritor da função-alvo, senão o bytecode do chamador fica desalinhado.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::register::NativeHandler;
#[cfg(feature = "devtools")]
use crate::rtti::{self, Registry};

/// Fase de disparo pedida pelo registrante — corresponde a `Observe`/`ObserveAfter`/`Override` da API Lua,
/// sem o parâmetro `absolute`/`after` separado (fundidos num enum só).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg(feature = "devtools")]
pub enum Phase {
    /// Roda ANTES da função original (ou do Replace, se houver) — fire-and-forget, não pode
    /// suprimir nem alterar o retorno (como o `ObserveBefore` da API Lua).
    Before = 0,
    /// Roda DEPOIS da função original (ou do Replace) ter retornado — fire-and-forget.
    After = 1,
    /// SUBSTITUI a função original — a original NUNCA roda para esta chamada. O handler é
    /// responsável por escrever o retorno em `aOut` se a função não for `void` (mesma
    /// responsabilidade que um `NativeHandler` normal já tem ao ser chamado via
    /// `register::NATIVE_ROUTES`). V1: só o ÚLTIMO `Replace` registrado pra este alvo roda
    /// (ver nota de composição no cabeçalho do módulo).
    Replace = 2,
}

struct Entry {
    /// Só pra log/diagnóstico (`"ClassName.MethodName"` ou `"(global).Nome"`).
    name: String,
    before: Vec<NativeHandler>,
    after: Vec<NativeHandler>,
    replace: Vec<NativeHandler>,
}

/// Indexado pelo `CBaseFunction*` REAL (o mesmo ponteiro que `rtti::resolve_func` devolve e que
/// chega em `exec_replacement` como `func`). Casar por PONTEIRO (não CName)
/// evita `class_filters`/parent-walk para não confundir overloads: aqui cada `register()` resolve EXATAMENTE 1 método de EXATAMENTE
/// 1 classe (ou 1 global), então o ponteiro já é inequívoco. Trade-off documentado: hookar o MESMO
/// método em N subclasses diferentes exige N chamadas a `register()` (1 por classe concreta) —
/// não há "hookar a família toda de uma vez" nesta V1.
static TARGETS: Mutex<Option<HashMap<usize, Entry>>> = Mutex::new(None);
/// Contagem de alvos: o `exec_replacement` lê uma vez por chamada (via `has_targets`) e, com 0, não toca na tabela —
/// mesmo padrão de `register::NATIVE_ROUTE_COUNT`.
static TARGET_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Resolve `class.method` (ou só `method` como GLOBAL se `class` for vazio) e registra `cb` na
/// fase pedida. Reusa `rtti::resolve_func` (classe+método) ou `register::get_function` (global) —
/// ambos JÁ EM PRODUÇÃO, nenhuma RE nova. Devolve `false` se a resolução falhar (classe/método
/// inexistente) — mesma convenção honesta dos outros `register_*`.
#[cfg(feature = "devtools")]
pub unsafe fn register(reg: &Registry, class: &str, method: &str, phase: Phase, cb: NativeHandler) -> bool {
    let resolved = if class.is_empty() {
        let f = crate::register::get_function(reg, method);
        if !rtti::sane(f) {
            None
        } else {
            Some(f)
        }
    } else {
        rtti::resolve_func(reg, class, method).map(|rf| rf.func)
    };
    let Some(func) = resolved else {
        crate::log_dev!("[fnoverride] register: '{class}.{method}' não resolveu");
        return false;
    };
    let key = func as usize;
    let mut guard = match TARGETS.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    let map = guard.get_or_insert_with(HashMap::new);
    let entry = map.entry(key).or_insert_with(|| Entry {
        name: if class.is_empty() {
            format!("(global).{method}")
        } else {
            format!("{class}.{method}")
        },
        before: Vec::new(),
        after: Vec::new(),
        replace: Vec::new(),
    });
    match phase {
        Phase::Before => entry.before.push(cb),
        Phase::After => entry.after.push(cb),
        Phase::Replace => entry.replace.push(cb),
    }
    // Captura tudo que precisa de `entry` ANTES de tocar `map.len()` — `entry` empresta `map`
    // mutavelmente, e o borrow-checker exige que esse empréstimo termine antes de qualquer uso
    // (mesmo imutável) de `map` que sobreviva além dele (NLL).
    let name = entry.name.clone();
    let (n_before, n_after, n_replace) = (entry.before.len(), entry.after.len(), entry.replace.len());
    TARGET_COUNT.store(map.len(), Ordering::Relaxed);
    crate::log_dev!(
        "[fnoverride] registrado {name} fase={phase:?} func={key:#x} (before={n_before} after={n_after} replace={n_replace})"
    );
    true
}

/// `true` se existe pelo menos 1 alvo registrado. O `exec_replacement` lê uma vez por chamada e usa o mesmo valor para
/// o Before e o After, então os dois ficam pareados; com `false` não trava nem procura na tabela.
#[inline]
pub fn has_targets() -> bool {
    TARGET_COUNT.load(Ordering::Relaxed) != 0
}

/// Chamado de `exec_replacement` ANTES de decidir se cai na via nativa/original do jogo. Roda
/// todos os `Before` registrados pra `func` (fire-and-forget) e, se houver `Replace`, roda SÓ o
/// último registrado e devolve `true` (suprime a chamada original — o chamador NÃO deve invocar
/// `orig(func,ctx,frame,a_out,a4)` neste caso, mesmo contrato que `route_native`/`watched_before`
/// já usam pro mesmo hook).
///
/// Chamado em `inicio::boot::exec_replacement` antes do roteamento das natives nossas.
pub unsafe fn dispatch_before(
    func: *mut c_void,
    ctx: *mut c_void,
    frame: *mut c_void,
    a_out: *mut c_void,
    a4: *mut c_void,
) -> bool {
    if func.is_null() {
        return false;
    }
    let guard = match TARGETS.try_lock() {
        Ok(g) => g,
        Err(_) => return false, // reentrante (um handler disparou outra chamada vigiada) — não bloqueia
    };
    let Some(map) = guard.as_ref() else {
        return false;
    };
    let Some(entry) = map.get(&(func as usize)) else {
        return false;
    };
    for cb in &entry.before {
        cb(ctx, frame, a_out, a4 as i64);
    }
    if let Some(cb) = entry.replace.last() {
        cb(ctx, frame, a_out, a4 as i64);
        return true;
    }
    false
}

/// Chamado de `exec_replacement` DEPOIS da função original (ou do `Replace`) ter retornado —
/// roda todos os `After` registrados pra `func`, em ordem de registro.
///
/// O executor chama em toda saída (Replace, native nossa, original) quando havia alvo
/// no início da chamada.
pub unsafe fn dispatch_after(
    func: *mut c_void,
    ctx: *mut c_void,
    frame: *mut c_void,
    a_out: *mut c_void,
    a4: *mut c_void,
) {
    if func.is_null() {
        return;
    }
    let guard = match TARGETS.try_lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let Some(map) = guard.as_ref() else {
        return;
    };
    let Some(entry) = map.get(&(func as usize)) else {
        return;
    };
    for cb in &entry.after {
        cb(ctx, frame, a_out, a4 as i64);
    }
}

