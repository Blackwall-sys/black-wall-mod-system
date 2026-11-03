//! sobrepor_funcao — "FunctionOverride" do BWMS: hook nativo (Rust, não-Lua) de QUALQUER
//! função redscript (nativa OU scriptada) por classe+método, com composição Before/After/
//! Replace multi-mod. Prep de 2026-08-11 (`PENDENCIAS-UNIFICADAS.md`, CET item `#39/FunctionOverride`).
//!
//! # Como o CET real faz isto (C++, `enablers/CET/src/scripting/FunctionOverride.cpp`)
//!
//! CET só hooka 2 pontos de entrada GLOBAIS e BAIXO NÍVEL via MinHook:
//! `CScript_RunPureScript` (despacho de função NATIVA) e `CScript_AllocateFunction` (alocação
//! do POD de função, só pra garantir espaço extra pro "trampolim"). Para interceptar uma função
//! redscript ESPECÍFICA sem recompilar `.reds`, ele precisa de 3 truques encadeados, porque esses
//! 2 pontos de hook não sabem NADA sobre qual `CBaseFunction*` está sendo chamado além do que já
//! recebem — e uma função SCRIPTADA (não-nativa) nem passa pelo hook de `RunPureScript`:
//!
//! 1. **Clona os METADADOS da função real** (`CopyFunctionDescription`: fullName/params/bytecode)
//!    num objeto NOVO ("trampolim"), com o ponteiro de código nativo apontando pra um STUB DE
//!    MÁQUINA gerado via `Xbyak::CodeGenerator` em runtime (`OverrideCodegen`, 7 instruções x64:
//!    `sub rsp,56; mov rax,funcReal; mov [rsp+32],rax; mov rax,HandleOverridenFunction; call rax;
//!    add rsp,56; ret`). O stub existe só porque a convenção de chamada nativa
//!    `(ctx,frame,out,a4)` NÃO carrega "qual CBaseFunction sou eu" — o stub materializa esse 5º
//!    argumento (o ponteiro real) como CONSTANTE embutida no código, então é 1 stub por função
//!    hookada (mini-JIT de "closure com constante presa").
//! 2. **Swap de conteúdo por PONTEIRO ESTÁVEL**: `memcpy` trocando os BYTES INTEIROS entre o
//!    objeto da função REAL (cujo ponteiro está espalhado pela RTTI inteira — `CClass::funcs` etc,
//!    nunca muda) e o objeto trampolim novo. Depois do swap, qualquer código que já tinha o
//!    ponteiro ORIGINAL passa a executar o STUB (sem saber que mudou), e a implementação original
//!    passa a viver no objeto "trampolim" (ainda 100% chamável, é o "next" do fim da cadeia).
//!    Isso é necessário porque CET força uma função SCRIPTADA a "virar nativa" pra cair no hook
//!    de `RunPureScript`/`AllocateFunction` — sem trocar a IDENTIDADE do objeto function, outros
//!    lugares da engine que já resolveram o ponteiro original nunca veriam a interceptação.
//! 3. **`ExecuteChain`/`WrapNextOverride`**: cadeia de middleware com continuação `next()`
//!    (Before → Overrides recursivos com `next` → After), usando `RTTIHelper::ExecuteFunction`
//!    (bytecode montado à mão) pra invocar a função real no fim da cadeia.
//!
//! # Por que o BWMS NÃO precisa do passo 1 nem do passo 2 (nem de JIT nenhum)
//!
//! O executor universal do BWMS (`inicio::boot::exec_replacement`, hook JÁ INSTALADO e provado desde
//! 2026-06/2026-07) intercepta o equivalente ARM64/macOS de `CBaseFunction::InternalExecute` — o
//! MESMO ponto de despacho por onde passa TODA chamada de função redscript, nativa OU scriptada,
//! ANTES do motor decidir internamente qual caminho seguir. `exec_replacement` já recebe
//! `(func: CBaseFunction*, ctx, frame, aOut, a4)` — exatamente a identidade que CET precisa
//! "injetar" via stub. Como o BWMS intercepta um nível ACIMA (o despachante universal, não os
//! 2 pontos específicos de native-execute que CET usa), **"interceptar a função X" aqui é só
//! registrar `X -> callback` numa tabela e consultar essa tabela dentro do hook já existente** —
//! zero swap de struct, zero geração de código de máquina nova. A técnica de tabela-por-ponteiro
//! já está em produção pro caso "nativa FORJADA por nós" (`register::NATIVE_ROUTES`/`route_native`,
//! consultado no MESMO `exec_replacement`); este módulo generaliza o mesmo padrão pra QUALQUER
//! `CBaseFunction*` já EXISTENTE (vanilla ou de outro mod), resolvido via `rtti::resolve_func`
//! (a mesma via já usada pelo CallbackSystem e pela Reflection). Casa por PONTEIRO (`ResolvedFn::func`, exato para "1 classe
//! concreta, 1 método") e usa `register::NativeHandler` como callback, a mesma assinatura que plugins já implementam para
//! `register_native`/`register_method`.
//!
//! # Composição multi-mod: divergência CONSCIENTE do `next()` do CET (V1)
//!
//! CET dá a cada `Override` registrado um `next` que ou chama o PRÓXIMO override ou (no fim da
//! cadeia) a função real — permite que um mod decida "delego pro próximo" vs "substituo por
//! completo" caso a caso. Implementar isso aqui exigiria closures Rust capturando o restante da
//! cadeia (viável, mas mais estado + mais superfície de bug em código ainda não testado ao vivo).
//! **V1 usa uma regra mais simples e determinística: `Before` e `After` disparam TODOS os
//! registrados, em ordem de registro; `Replace` só o ÚLTIMO registrado roda** (mod que
//! registrou por último "vence"). Isso cobre o
//! caso prático dominante (1 mod substitui, N mods observam antes/depois) sem reintroduzir a
//! complexidade de continuação que o CET só paga por causa do stub JIT. Evoluir pra um `next()`
//! genuíno depois é troca de ESTRUTURA DE DADOS/ORDEM DE CHAMADA, não de PRIMITIVA — pode ser
//! feito sem tocar a parte arriscada (o hook em si já está resolvido).
//!
//! # Status
//!
//! Provado ao vivo em 2026-08-11 (`cp77-symbols/notes/proofs/2026-08-11-cet-fnoverride-PROVADO.log`): Before, After e
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

/// Fase de disparo pedida pelo registrante — espelha `Observe`/`ObserveAfter`/`Override` do CET,
/// sem o parâmetro `absolute`/`after` separado (fundidos num enum só).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg(feature = "devtools")]
pub enum Phase {
    /// Roda ANTES da função original (ou do Replace, se houver) — fire-and-forget, não pode
    /// suprimir nem alterar o retorno (mesmo espírito do `Before`/`ObserveBefore` do CET).
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

