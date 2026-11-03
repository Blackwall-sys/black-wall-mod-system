//! Codeware `#176`: a tecla chega ao widget com foco. O hook do `inkSystem::ProcessCharacterEvent` entrega, antes da original, um `BwmsInkKeyEvent`
//! ao `inputWidget` (o `WidgetInputService` do Codeware) quando o `.reds` da classe está no bundle. Os testes 176a/176c/diag usam o mesmo hook e
//! leem os estáticos daqui; os ticks de teste ficam no selftest.

use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crate::ganchos::install_hook_probe;
use crate::marcador_individual;

// ===== Codeware `#176` 176a — `inkSystem::ProcessCharacterEvent` só observando (2026-09-13) =====
// `0x10427028c(InkSystem*, EInputKey, EInputAction) -> bool`. O laço de despacho do `ProcessInputEvents` chama pra cada evento da fila (`0x10426d328`:
// `ldp w1,w2,[evento]; bl`) e testa o `w0` inteiro (`cbnz w0` em `0x10426d334`): `true` tira o evento da fila. A função trava o `inputWidget` (`+0x2E8`) e sem widget
// devolve `false`; com widget, tecla 8..46 vira evento de navegação e as outras, caractere (letra com e sem shift, dígito, numpad, pontuação), num
// `inkCharacterEvent` de 0xA0 B (ação `@0x90`, caractere `@0x94`, navegação `@0x95`; a base `inkInputEvent` tem 0x90 B e guarda o ponteiro do
// `keyboardState` em `@0x68`) entregue ao widget pelo slot 50 da vtable (`OnCharacterKey`), e devolve `true`. Prólogo de 16 B sem instrução relativa
// ao PC, conferido antes do hook. O probe repassa o `x0` inteiro da original.
const C176_PCE_VM: u64 = 0x1_0427_028c;
const C176_PCE_PROLOGO: [u32; 4] = [0xd101_c3ff, 0xa902_67fa, 0xa903_5ff8, 0xa904_57f6];
#[cfg(feature = "devtools")]
pub(crate) const C176_IK_A: u32 = 65;
pub(crate) static ORIG_C176_PCE: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
pub(crate) static C176_CHAMADAS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
pub(crate) static C176_GRAVANDO: AtomicBool = AtomicBool::new(false);
// (this, tecla, ação, retorno, inputWidget lido antes da original), até 64 chamadas depois do pedido do teste.
#[cfg(feature = "devtools")]
pub(crate) static C176_EVENTOS: std::sync::Mutex<Vec<(u64, u32, u32, u64, u64)>> = std::sync::Mutex::new(Vec::new());
// O `0x1042700d0` do 2.5 (`(InkSystem*, const evento*)`, só eventos do tipo 3 num laço à parte, ação 9 fixa) é outra função; o `ProcessCharacterEvent`
// com `(tecla, ação)` continua no `0x10427028c`.
type C176PceFn = unsafe extern "C" fn(*mut c_void, u32, u32) -> u64;

// ===== Codeware `#176` 176c — `TriggerEvent` com ouvinte de script, na thread do ink (2026-09-13) =====
// `TriggerEvent` = `0x1043ac60c(EventManager* = widget+0x50, CName, const Handle<Event>*) -> bool` (true com ouvinte). O teste 176c prepara na thread do
// jogo um `inkCanvas` solto, o ouvinte de script `BwmsC176Ouvinte` registrado com `RegisterToCallback(n"OnBwmsInputKey", ouvinte, n"OnTecla")` e um
// `BwmsInkKeyEvent` de IK_A, e injeta IK_A pela fila do `#120`. No probe (thread do ink), a 1ª chegada de IK_A com ação 1 dispara o `TriggerEvent` com o
// nome registrado e com um nome sem ouvinte (controle); o ouvinte chama `BwmsC176Recebido`.
#[cfg(feature = "devtools")]
pub(crate) const C176C_EVENTO_NOME: &str = "OnBwmsInputKey";
#[cfg(feature = "devtools")]
const C176C_CONTROLE_NOME: &str = "OnBwmsInputKeySemOuvinte";
#[cfg(feature = "devtools")]
pub(crate) static C176C_WIDGET: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
pub(crate) static C176C_EVENTO: std::sync::Mutex<[u64; 2]> = std::sync::Mutex::new([0; 2]);
#[cfg(feature = "devtools")]
pub(crate) static C176C_DISPARO: std::sync::Mutex<Option<(bool, bool, String)>> = std::sync::Mutex::new(None);
#[cfg(feature = "devtools")]
pub(crate) static C176C_RECEBIDOS: std::sync::Mutex<Vec<(i32, i32, bool, bool, String)>> = std::sync::Mutex::new(Vec::new());
type C176TriggerFn = unsafe extern "C" fn(*mut c_void, u64, *const u64) -> u64;

/// Chamado pelo `BwmsC176Recebido` do ouvinte de script: tecla, ação, caractere `a`/`A` e `IsCharacter`, com a thread que rodou o ouvinte.
#[cfg(feature = "devtools")]
pub(crate) fn c176c_recebido(tecla: i32, acao: i32, caractere_a: bool, e_caractere: bool) {
    if let Ok(mut v) = C176C_RECEBIDOS.lock() {
        if v.len() < 8 {
            v.push((tecla, acao, caractere_a, e_caractere, format!("{:?}", std::thread::current().id())));
        }
    }
}

#[cfg(feature = "devtools")]
unsafe fn c176c_disparar(widget: u64) {
    let Ok(h) = C176C_EVENTO.try_lock().map(|g| *g) else {
        C176C_WIDGET.store(widget, Ordering::Relaxed);
        return;
    };
    let trigger: C176TriggerFn = std::mem::transmute(crate::rebase(crate::jogo::interface::WIDGET_DISPARAR_EVENTO_VM));
    let manager = (widget as *mut u8).add(0x50) as *mut c_void;
    let com = trigger(manager, crate::cname::cname(C176C_EVENTO_NOME), h.as_ptr()) & 1 != 0;
    let sem = trigger(manager, crate::cname::cname(C176C_CONTROLE_NOME), h.as_ptr()) & 1 != 0;
    if let Ok(mut d) = C176C_DISPARO.lock() {
        *d = Some((com, sem, format!("{:?}", std::thread::current().id())));
    }
}

// ===== Codeware `#176` 176d — entrega de produção: a tecla ao widget com foco (2026-09-13) =====
// Como o `WidgetInputService` do Codeware, antes da original: press e release com `inputWidget` vivo viram um `BwmsInkKeyEvent` entregue por
// `TriggerEvent(inputWidget+0x50, n"OnBwmsInputKey", evento)`. Roda na thread do ink, onde o motor cria o `inkCharacterEvent` pelo pool de eventos do
// ink. O widget é travado como o motor trava (+1 no contador forte por CAS) e solto no fim; o evento é solto depois do `TriggerEvent`. Ligada quando o
// `.reds` da classe está no bundle ou com `~/.bwms-hook-c176d-teste`; sem a classe forjada (`bwms_ink_key_event_novo` nulo) não faz nada.
pub(crate) const C176_EVENTO_TECLA: &str = "OnBwmsInputKey";
pub(crate) static C176_ENTREGA_ATIVA: AtomicBool = AtomicBool::new(false);
static C176_ENTREGAS: AtomicU64 = AtomicU64::new(0);
pub(crate) static C176_ENTREGAS_COM_OUVINTE: AtomicU64 = AtomicU64::new(0);
static C176_ENTREGAS_SEM_OUVINTE: AtomicU64 = AtomicU64::new(0);
// Diagnóstico dos boots 152/154 (sem engagement com a entrega ligada): chamadas da entrega e o modo sem efeito do `~/.bwms-hook-c176diag-teste`.
#[cfg(feature = "devtools")]
static C176_DIAG: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
static C176_SEM_EFEITO: AtomicBool = AtomicBool::new(false);
// `~/.bwms-hook-c176diag-trava`: a entrega trava o contador forte do widget e solta logo depois, sem buscar ouvintes nem criar evento.
#[cfg(feature = "devtools")]
static C176_SO_TRAVA: AtomicBool = AtomicBool::new(false);

/// Resumo do diagnóstico, uma vez em gameplay: chegar aqui com a entrega ligada já é o resultado.
#[cfg(feature = "devtools")]
pub(crate) fn c176diag_teste_tick() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FEITO: AtomicBool = AtomicBool::new(false);
    if FEITO.load(Ordering::Relaxed)
        || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 60)
        || !(marcador_individual("c176diag-teste") || marcador_individual("c176diag-trava"))
        || !crate::inicio::boot::gameplay_real()
    {
        return;
    }
    FEITO.store(true, Ordering::Relaxed);
    crate::veredito_dev!(
        "[c176diag] VEREDITO-176DIAG gameplay alcançado com a entrega ligada={}; {} chamada(s) da entrega; sem efeito={}; só trava={}; com ouvinte {}, sem ouvinte {}; diag-ok=true",
        C176_ENTREGA_ATIVA.load(Ordering::Relaxed),
        C176_DIAG.load(Ordering::Relaxed),
        C176_SEM_EFEITO.load(Ordering::Relaxed),
        C176_SO_TRAVA.load(Ordering::Relaxed),
        C176_ENTREGAS_COM_OUVINTE.load(Ordering::Relaxed),
        C176_ENTREGAS_SEM_OUVINTE.load(Ordering::Relaxed)
    );
}
// Busca dos ouvintes de um nome no `EventManager` (a 1ª chamada do `TriggerEvent`): devolve a lista ou nulo.
const C176_OUVINTES_VM: u64 = 0x1_0424_ecdc;
type C176OuvintesFn = unsafe extern "C" fn(*mut c_void, u64) -> *mut c_void;

unsafe fn c176_entregar(ink: *mut c_void, tecla: u32, acao: u32) {
    let base = ink as *const u8;
    if ink.is_null() || !crate::memoria::is_readable(base.add(0x2e8) as *const c_void, 0x18) {
        return;
    }
    let widget = (base.add(0x2e8) as *const u64).read_unaligned();
    let contador = (base.add(0x2f0) as *const u64).read_unaligned() as *const std::sync::atomic::AtomicU32;
    // Diagnóstico do boot 155 (engagement não chegou nos boots 152 e 154 com a entrega ligada): com marcador de teste, as 40 primeiras chamadas vão pro
    // canal de veredito e, com `~/.bwms-hook-c176diag-teste`, a entrega sai aqui, antes de travar o widget, buscar ouvintes ou soltar.
    #[cfg(feature = "devtools")]
    {
        let n = C176_DIAG.fetch_add(1, Ordering::Relaxed);
        let sem_efeito = C176_SEM_EFEITO.load(Ordering::Relaxed);
        if n < 40 && C176_REGISTRO.load(Ordering::Relaxed) {
            let forte_antes =
                if !contador.is_null() && crate::memoria::is_readable(contador as *const c_void, 4) { (*contador).load(Ordering::Relaxed) } else { u32::MAX };
            crate::veredito_dev!("[c176-diag] entrega #{n} tecla {tecla} ação {acao} inputWidget {widget:#x} forte {forte_antes} sem-efeito {sem_efeito}");
        }
        if sem_efeito {
            return;
        }
    }
    if widget == 0 || contador.is_null() || !crate::memoria::is_readable(contador as *const c_void, 4) {
        return;
    }
    let forte = &*contador;
    let mut s = forte.load(Ordering::Relaxed);
    loop {
        if s == 0 {
            return;
        }
        match forte.compare_exchange_weak(s, s + 1, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => break,
            Err(atual) => s = atual,
        }
    }
    let mut hw = [widget, contador as u64];
    #[cfg(feature = "devtools")]
    if C176_SO_TRAVA.load(Ordering::Relaxed) {
        crate::jogo::objetos::soltar_handle(&mut hw);
        return;
    }
    // Sem ouvinte do nome no widget, nada a entregar: consulta a mesma busca que o `TriggerEvent` faz (`0x10424ecdc(manager, nome)`, trava de leitura em
    // `manager+0x30`) antes de criar o evento, e uma tela vanilla com foco não paga alocação nem soltura por tecla.
    let ouvintes: C176OuvintesFn = std::mem::transmute(crate::rebase(C176_OUVINTES_VM));
    if ouvintes((widget as *mut u8).add(0x50) as *mut c_void, crate::cname::cname(C176_EVENTO_TECLA)).is_null() {
        C176_ENTREGAS_SEM_OUVINTE.fetch_add(1, Ordering::Relaxed);
        crate::jogo::objetos::soltar_handle(&mut hw);
        return;
    }
    if let Some(reg) = crate::rtti::Registry::obtain() {
        let evento = crate::interface::bwms_ink_key_event_novo(&reg, base.add(0x2f8), tecla, acao);
        if !evento.is_null() {
            let mut b = [0u8; 16];
            crate::rtti::make_handle(b.as_mut_ptr() as *mut c_void, evento);
            let mut he = [(b.as_ptr() as *const u64).read_unaligned(), (b.as_ptr().add(8) as *const u64).read_unaligned()];
            let trigger: C176TriggerFn = std::mem::transmute(crate::rebase(crate::jogo::interface::WIDGET_DISPARAR_EVENTO_VM));
            let com_ouvinte = trigger((widget as *mut u8).add(0x50) as *mut c_void, crate::cname::cname(C176_EVENTO_TECLA), he.as_ptr()) & 1 != 0;
            C176_ENTREGAS.fetch_add(1, Ordering::Relaxed);
            if com_ouvinte {
                C176_ENTREGAS_COM_OUVINTE.fetch_add(1, Ordering::Relaxed);
            }
            crate::jogo::objetos::soltar_handle(&mut he);
        }
    }
    crate::jogo::objetos::soltar_handle(&mut hw);
}

unsafe extern "C" fn c176_pce_probe(ink: *mut c_void, tecla: u32, acao: u32) -> u64 {
    #[cfg(feature = "devtools")]
    {
        let teste = C176C_WIDGET.load(Ordering::Relaxed);
        if teste != 0 && tecla == C176_IK_A && acao == 1 && C176C_WIDGET.compare_exchange(teste, 0, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            c176c_disparar(teste);
        }
    }
    if (acao == 1 || acao == 2) && C176_ENTREGA_ATIVA.load(Ordering::Relaxed) {
        c176_entregar(ink, tecla, acao);
    }
    // O `inputWidget` antes da original e a contagem só servem à gravação dos testes 176a/176c.
    #[cfg(feature = "devtools")]
    let widget = {
        let campo = (ink as *const u8).wrapping_add(0x2e8);
        if !ink.is_null() && crate::memoria::is_readable(campo as *const c_void, 8) { (campo as *const u64).read_unaligned() } else { 0 }
    };
    let r = std::mem::transmute::<u64, C176PceFn>(crate::ganchos::original_publicado(&ORIG_C176_PCE))(ink, tecla, acao);
    #[cfg(feature = "devtools")]
    {
        C176_CHAMADAS.fetch_add(1, Ordering::Relaxed);
        if C176_GRAVANDO.load(Ordering::Relaxed) {
            if let Ok(mut v) = C176_EVENTOS.try_lock() {
                if v.len() < 64 {
                    v.push((ink as u64, tecla, acao, r, widget));
                }
            }
        }
    }
    r
}

/// `bwms-ink-key-event.reds` no bundle (`<jogo>/r6/scripts/blackwall-mods`, a partir da pasta da dylib em `red4ext/`).
fn c176_reds_no_bundle() -> bool {
    crate::dylib_dir().is_some_and(|d| std::path::Path::new(&d).join("../r6/scripts/blackwall-mods/bwms-ink-key-event.reds").exists())
}

/// Instala o hook do `ProcessCharacterEvent` (no `on_load`) com o prólogo conferido: pra entrega de produção (176d: o `.reds` da classe no bundle ou
/// `~/.bwms-hook-c176d-teste`) e pros testes 176a/176c.
pub(crate) unsafe fn install_c176_pce_probe() {
    // Produção: o `.reds` da classe no bundle liga a entrega. Os travamentos dos boots 152/154 antes da engagement ficaram fora dela (boot 155: todas as
    // chamadas até o gameplay com `inputWidget` nulo, nada tocado no motor) e o boot 156 chegou ao gameplay com a entrega ligada.
    let no_bundle = c176_reds_no_bundle();
    // `~/.bwms-hook-c176diag-teste` liga a entrega em modo sem efeito (só registra as chamadas); `c176diag-trava` só trava e solta o widget em foco.
    #[cfg(feature = "devtools")]
    let por_marcador = {
        let diag = marcador_individual("c176diag-teste");
        let so_trava = marcador_individual("c176diag-trava");
        C176_SEM_EFEITO.store(diag, Ordering::Relaxed);
        C176_SO_TRAVA.store(so_trava, Ordering::Relaxed);
        let por_marcador = marcador_individual("c176d-teste") || marcador_individual("c176e-teste") || diag || so_trava;
        C176_REGISTRO.store(por_marcador, Ordering::Relaxed);
        por_marcador
    };
    #[cfg(not(feature = "devtools"))]
    let por_marcador = false;
    let entrega = no_bundle || por_marcador;
    crate::log_dev!("[c176] reds da classe no bundle={no_bundle}; marcador de teste da entrega={por_marcador}");
    C176_ENTREGA_PEDIDA.store(entrega, Ordering::Relaxed);
    if !entrega && !marcador_individual("c176a-teste") && !marcador_individual("c176c-teste") {
        return;
    }
    let alvo = crate::rebase(C176_PCE_VM) as *const u32;
    if !crate::memoria::is_readable(alvo as *const c_void, 16) {
        return;
    }
    let lido: Vec<u32> = (0..4).map(|i| alvo.add(i).read_unaligned()).collect();
    if lido != C176_PCE_PROLOGO {
        crate::veredito_dev!("[c176a] prólogo do ProcessCharacterEvent diferente do esperado ({lido:08x?}); hook não instalado");
        return;
    }
    install_hook_probe!(
        C176_PCE_VM, c176_pce_probe, ORIG_C176_PCE,
        &format!("[c176a] inkSystem::ProcessCharacterEvent hook @ {C176_PCE_VM:#012x}"),
        "[c176a] FALHA ao hookar inkSystem::ProcessCharacterEvent"
    );
    let ativa = C176_ENTREGA_PEDIDA.load(Ordering::Relaxed) && ORIG_C176_PCE.load(Ordering::Relaxed) != 0;
    C176_ENTREGA_ATIVA.store(ativa, Ordering::Relaxed);
    crate::log_dev!("[c176] entrega da tecla ao widget com foco ligada={ativa}");
}

// Pedido de entrega (o `.reds` da classe no bundle ou o marcador do 176d), lido no `install_c176_pce_probe`; a entrega só liga com o hook instalado.
static C176_ENTREGA_PEDIDA: AtomicBool = AtomicBool::new(false);
// Marcador de teste da entrega (176d ou diagnóstico): só com ele as 40 primeiras chamadas vão pro canal de veredito.
#[cfg(feature = "devtools")]
static C176_REGISTRO: AtomicBool = AtomicBool::new(false);
