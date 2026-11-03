//! Tempo do motor: duração do quadro, dilatação do tempo e conversão entre tempo e ticks.

#![allow(dead_code)]

use std::sync::atomic::AtomicBool;
use crate::diagnostico::{marcar_estagio, ANIM_FRAMEBEGIN_HOOK_CNT, UPDATE_REGISTRAR_CAPTURED_PTR};
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use crate::register::*;

/// Cadência por contagem: verdadeiro na volta 0 e a cada `periodo` voltas depois. `periodo` 0 entra em pânico, como o `%`.
#[inline]
pub(crate) fn a_cada(volta: u64, periodo: u64) -> bool {
    volta % periodo == 0
}

/// `intervalo` cumprido de `desde` até `agora` (ticks ou segundos, a mesma unidade nos três); `agora` antes de `desde` conta como não cumprido.
#[inline]
pub(crate) fn ja_passou(desde: u64, agora: u64, intervalo: u64) -> bool {
    agora.saturating_sub(desde) >= intervalo
}

/// `BwmsFrameDeltaTime() -> Float` e `BwmsFrameTimeDilation() -> Float`, do `FrameInfo` do frame corrente (guardado pelos
/// ganchos dos estágios do pipeline). Consomem o frame mesmo sem parâmetro: chamadas inline (`BwmsFrameDeltaTime() * BwmsFrameTimeDilation()`)
/// deixariam o ParamEnd para a próxima native (boot 233).
pub(crate) unsafe extern "C" fn tramp_frame_delta(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::rtti::read_params_consuming(current_native_func(), frame);
    write_f32_ret(out, f32::from_bits(crate::tempo::FRAME_DELTA.load(std::sync::atomic::Ordering::Relaxed)));
}

pub(crate) unsafe extern "C" fn tramp_frame_dilatacao(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::rtti::read_params_consuming(current_native_func(), frame);
    write_f32_ret(out, f32::from_bits(crate::tempo::FRAME_DILATACAO.load(std::sync::atomic::Ordering::Relaxed)));
}


// `UpdateRegistrar::RegisterUpdate`, overload GROUP — RE offline 2026-08-11
// (agente dedicado): string-xref+ABI cruzada contra 5 call-sites reais cujo argumento de enum bate
// semanticamente com a string de debug-name daquele call-site (ex. "CameraSystem/PlayerAimUpdateTick"
// passa W1=7=PlayerAimUpdate). Assinatura real: `RegisterUpdate(UpdateTickGroup
// aGroup, IScriptable* aSystem, const char* aName, GroupUpdateCallback&& aCallback)` — this=X0,
// group(u8)=X1, system=X2, name(cstr)=X3, callback(ponteiro, é referência)=X4. Hook OBSERVE-ONLY
// (captura só o `this`/UpdateRegistrar* na 1ª chamada vanilla real, nunca chama RegisterUpdate de
// volta ainda — categoria de risco BAIXA, mesmo padrão de passthrough já provado dezenas de vezes
// nesta sessão). `UpdateRegistrar` não tem layout próprio pra ler,
// só serve pra REUSAR o ponteiro capturado numa chamada futura de `RegisterUpdate`.
pub(crate) const UPDATE_REGISTRAR_GROUP_VM: u64 = 0x104d74ab0;

pub(crate) static UPDATE_REGISTRAR_CAPTURE_CNT: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_UPDATE_REGISTRAR_GROUP: AtomicU64 = AtomicU64::new(0);

// ===== UpdateRegistrar::RegisterUpdate(group) OBSERVE-ONLY =====
// Captura o `this`(UpdateRegistrar*) real na 1ª chamada vanilla (acontece naturalmente no boot,
// confirmado por 5 call-sites reais na RE offline) — NÃO chama RegisterUpdate de volta ainda, só
// loga+guarda o ponteiro pra uma etapa futura (construir Callback<> + registrar nosso próprio
// update). Passthrough incondicional pro original em toda chamada (nunca muda comportamento real).
// São 6 argumentos (`0x104d74acc mov x19, x5`; o `w5` vai ao nó de update em `0x104d74b98`, e os 128 chamadores passam `#0xa`): com 5, o
// original recebia no `w5` o que o Rust deixasse no registrador (`z034/relatorios/GANCHOS-ARGS.md`).
pub(crate) type RegisterUpdateGroupFn = unsafe extern "C" fn(*mut c_void, u8, *mut c_void, *const u8, *mut c_void, u32);

pub(crate) unsafe extern "C" fn update_registrar_group_probe(
    this: *mut c_void,
    group: u8,
    system: *mut c_void,
    name: *const u8,
    callback: *mut c_void,
    ordem: u32,
) {
    let n = UPDATE_REGISTRAR_CAPTURE_CNT.fetch_add(1, Ordering::Relaxed);
    if n == 0 {
        UPDATE_REGISTRAR_CAPTURED_PTR.store(this as u64, Ordering::Relaxed);
    }
    if n < 8 {
        let name_str = if crate::memoria::is_readable(name as *const c_void, 1) {
            std::ffi::CStr::from_ptr(name as *const i8).to_string_lossy().into_owned()
        } else {
            String::from("(ilegível)")
        };
        // Opção A do plano de 2026-08-11: dereferencia o `Callback<>`
        // real (`callback` já chega como ponteiro — tipo não-trivial >16B, passado por referência
        // invisível no AAPCS64) pra descobrir qual dos 3 formatos (`UnboundFunctionTarget`/
        // `ClosureTarget`/`MemberFunctionTarget`) o jogo usa de verdade nesta build. Leitura pura,
        // zero escrita — mesma categoria de risco (zero) do resto deste probe observe-only.
        let (buf0, handler_ptr, h_invoke, h_copy, h_move, h_destruct) =
            // 2.5: buffer de 0x40 B e o handler em `+0x40` (`0x104d74b38`, `z036/relatorios/P.md`).
            if crate::memoria::is_readable(callback as *const c_void, 0x48) {
                let buf0 = (callback as *const u64).read_unaligned();
                let handler_ptr = ((callback as *const u8).add(0x40) as *const u64).read_unaligned();
                if handler_ptr != 0 && crate::memoria::is_readable(handler_ptr as *const c_void, 0x20) {
                    let hp = handler_ptr as *const u64;
                    (
                        buf0,
                        handler_ptr,
                        hp.read_unaligned(),
                        hp.add(1).read_unaligned(),
                        hp.add(2).read_unaligned(),
                        hp.add(3).read_unaligned(),
                    )
                } else {
                    (buf0, handler_ptr, 0, 0, 0, 0)
                }
            } else {
                (0, 0, 0, 0, 0, 0)
            };
        let matches_system = buf0 == (system as u64);
        crate::log_dev!(
            "[registro-de-update] RegisterUpdate(group)#{n} this={this:p} group={group} system={system:p} name='{name_str}' | callback={callback:p} buf0={buf0:#x} matches_system={matches_system} handler={handler_ptr:#x} invoke={h_invoke:#x} copy={h_copy:#x} move={h_move:#x} destruct={h_destruct:#x}"
        );
    }
    let f: RegisterUpdateGroupFn = std::mem::transmute(crate::ganchos::original_publicado(&ORIG_UPDATE_REGISTRAR_GROUP));
    f(this, group, system, name, callback, ordem);
}

// ===== Opção A: hook no `invoke` REAL do callback já registrado pelo motor =====
// Em vez de tocar em qualquer estrutura do
// `UpdateRegistrar` (categoria que já CROU, ver `register_bwms_update_tick` acima), instala um
// inline-hook comum direto no CÓDIGO do callback que `AnimationSystem_FrameBeginReset` registra — execução ordenada num ponto real do
// pipeline (grupo FrameBegin). O alvo é estático no binário 2.5 (`z036/relatorios/P.md`: o `Callback` da 2.5 tem o handler em `+0x40` e o
// nome chega como `"[U]/AnimationSystem_FrameBeginReset"`; o `invoke` do handler `0x107da5798` é `0x101a4ebac`). Não é capturado no
// `RegisterUpdate`: a captura de dentro do registro de updates do motor (dyld, `is_readable` a cada chamada) travou 3 de 5 boots antes da
// engagement em 2026-10-25, e o probe ficou só com o log.
const FRAMEBEGIN_INVOKE_VM: u64 = 0x1_01a4_ebac;

pub(crate) static ANIM_FRAMEBEGIN_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Trampolim do `invoke` original, publicado antes do salto (`instalar_com_threads_paradas`).
pub(crate) static ORIG_ANIM_FRAMEBEGIN_INVOKE: AtomicU64 = AtomicU64::new(0);

pub(crate) type AnimInvokeFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);

// Mais dois estágios do frame pelo mesmo caminho do `FrameBegin` — o `invoke` do 1º callback vanilla registrado em
// cada grupo (`UpdateTickGroup`: 3 = `PreBuckets`, antes dos buckets de entidades; 11 = `PreRenderUpdate`, antes do render) — e o
// `FrameInfo` (`deltaTime` em `+0x00`, `details` em `+0x08` com a `timeDilation` em `+0x04`) guardado a cada frame pras natives de leitura.
pub(crate) const ESTAGIOS: [(u32, &str); 2] = [(3, "Pipeline/PreBuckets"), (11, "Pipeline/PreRenderUpdate")];

/// Eventos dos 3 ganchos na ordem dos estágios (0 = `FrameBegin`, `I + 1` = `ESTAGIOS[I]`), a mesma de `marcar_estagio` e de [`DETALHES_PROVADOS`].
pub(crate) const EVENTOS_DO_PIPELINE: [&str; 3] = ["Pipeline/FrameBegin", ESTAGIOS[0].1, ESTAGIOS[1].1];

/// Bit `k` ligado = alguém ouve `EVENTOS_DO_PIPELINE[k]`. Os ganchos rodam todo frame em thread de job e só leem este atômico; quem trava
/// `CALLBACKS` para montá-lo são o registro e a remoção de ouvinte, em `atualizar_eventos_de_entidade`. Quem recebe continua decidido por
/// `fire_event_args`, que lê a tabela.
pub(crate) static PIPELINE_ARMADOS: AtomicU8 = AtomicU8::new(0);

/// Alguém ouve o evento do estágio (índice de [`EVENTOS_DO_PIPELINE`]).
#[inline]
pub(crate) fn estagio_armado(estagio: usize) -> bool {
    PIPELINE_ARMADOS.load(Ordering::Relaxed) & (1 << estagio) != 0
}

/// `invoke` do handler do 1º callback de cada grupo de [`ESTAGIOS`] no binário 2.5 (grupo 3 = `0x1050df7f0`, grupo 11 = `0x101bbe6b0`; pelos
/// fixups e chamadores do `RegisterUpdate`, `z036/relatorios/P.md`), estáticos pelo mesmo motivo do [`FRAMEBEGIN_INVOKE_VM`].
const ESTAGIO_INVOKE_VM: [u64; 2] = [0x1_050d_f7f0, 0x1_01bb_e6b0];

pub(crate) static ESTAGIO_ORIG: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

pub(crate) static ESTAGIO_INSTALADO: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

pub(crate) static FRAME_DELTA: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub(crate) static FRAME_DILATACAO: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Último `details` provado legível em cada estágio (0 = `FrameBegin`, 1 e 2 = `ESTAGIOS`), comparado por valor: o `is_readable` (syscall)
/// só roda quando o ponteiro muda. Um slot por estágio porque não está medido se os 3 estágios recebem o mesmo `FrameInfo` no frame;
/// com um slot só, ponteiros diferentes entre estágios provariam de novo a cada chamada.
static DETALHES_PROVADOS: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

/// `fi` é a `FrameInfo&` que o motor entrega ao `invoke` original nesta mesma chamada (referência C++ viva por contrato), então é lido
/// sem `is_readable`. O `details` é ponteiro guardado dentro dele: só ele passa pela prova.
pub(crate) unsafe fn guardar_frame_info(fi: *mut c_void, estagio: usize) {
    if fi.is_null() {
        return;
    }
    FRAME_DELTA.store((fi as *const u32).read_unaligned(), Ordering::Relaxed);
    let detalhes = ((fi as *const u8).add(8) as *const u64).read_unaligned();
    if detalhes == 0 {
        return;
    }
    let provado = &DETALHES_PROVADOS[estagio];
    if provado.load(Ordering::Relaxed) != detalhes {
        if !crate::memoria::is_readable(detalhes as *const c_void, 8) {
            return;
        }
        provado.store(detalhes, Ordering::Relaxed);
    }
    FRAME_DILATACAO.store(((detalhes + 4) as *const u32).read_unaligned(), Ordering::Relaxed);
}

pub(crate) unsafe extern "C" fn hook_estagio<const I: usize>(target: *mut c_void, frame_info: *mut c_void, job_queue: *mut c_void) {
    guardar_frame_info(frame_info, I + 1);
    marcar_estagio(I as u8 + 1);
    if estagio_armado(I + 1) {
        crate::tipos_do_motor::fire_event_args(EVENTOS_DO_PIPELINE[I + 1], &[]);
    }
    let f: AnimInvokeFn = std::mem::transmute(crate::ganchos::original_publicado(&ESTAGIO_ORIG[I]));
    f(target, frame_info, job_queue);
}

/// Instala os ganchos dos estágios nos `invoke` estáticos (idempotente por estágio).
pub(crate) unsafe fn instalar_estagios() {
    let ganchos: [*mut c_void; 2] = [hook_estagio::<0> as *mut c_void, hook_estagio::<1> as *mut c_void];
    for i in 0..ESTAGIOS.len() {
        let alvo = crate::rebase(ESTAGIO_INVOKE_VM[i]);
        if alvo.is_null() {
            crate::veredito_dev!("[pipeline] RECUSADO: invoke de {} (grupo {}) sem endereço neste build; nenhum salto escrito", ESTAGIOS[i].1, ESTAGIOS[i].0);
            continue;
        }
        if ESTAGIO_INSTALADO[i].swap(true, Ordering::Relaxed) {
            continue;
        }
        // Roda todo quadro (thread de job): escrita com as threads paradas e o trampolim publicado antes do salto (boots 178/181).
        match crate::ganchos::instalar_com_threads_paradas(alvo, ganchos[i], Some(&ESTAGIO_ORIG[i])) {
            Some(_) => {
                crate::veredito_dev!(
                    "[pipeline] gancho no invoke de {} (grupo {}) @ {alvo:p} (estático {:#x}, __text)",
                    ESTAGIOS[i].1,
                    ESTAGIOS[i].0,
                    ESTAGIO_INVOKE_VM[i]
                );
            }
            None => ESTAGIO_INSTALADO[i].store(false, Ordering::Relaxed),
        }
    }
}

/// Mesma assinatura ABI de `CallbackHandler::invoke` (`bwms_callback_invoke` acima, já provada):
/// `(target_buffer_ptr, FrameInfo*, JobQueue*)`. NUNCA suprime o original — só observa+repassa
/// (mesmo idioma de todo hook observe-only deste projeto), pra nunca regredir a animação real do jogo.
pub(crate) unsafe extern "C" fn bwms_anim_framebegin_hook(target: *mut c_void, frame_info: *mut c_void, job_queue: *mut c_void) {
    guardar_frame_info(frame_info, 0);
    marcar_estagio(0);
    let n = ANIM_FRAMEBEGIN_HOOK_CNT.fetch_add(1, Ordering::Relaxed);
    if n < 5 || a_cada(n, 600) {
        crate::log_dev!(
            "[registro-de-update-invoke] AnimationSystem_FrameBeginReset invoke interceptado #{n} target={target:p} frame_info={frame_info:p} job_queue={job_queue:p}"
        );
    }
    // O despacho (`resolve_in_class` + `call_func`, RTTI numa thread de job, já provado) só roda nos frames com ouvinte; a pergunta é o
    // bit do estágio, sem trava nem varredura de `CALLBACKS` a cada frame.
    if estagio_armado(0) {
        let sent = crate::tipos_do_motor::fire_event_args(EVENTOS_DO_PIPELINE[0], &[]);
        if n < 5 || a_cada(n, 600) {
            crate::log_dev!("[registro-de-update-invoke] fire_event_args('Pipeline/FrameBegin') dentro do hook #{n} -> despachado pra {sent} listener(s)");
        }
    }
    let f: AnimInvokeFn = std::mem::transmute(crate::ganchos::original_publicado(&ORIG_ANIM_FRAMEBEGIN_INVOKE));
    f(target, frame_info, job_queue);
}

/// Chamado do `cp77_tick` quando há ouvinte de `Pipeline/*` (`lib.rs`, `PIPELINE_ARMADOS`) até o gancho estar no `invoke` estático do
/// FrameBegin — idempotente, barato de rechecar todo tick.
pub(crate) unsafe fn install_anim_framebegin_hook_if_ready() -> bool {
    if ANIM_FRAMEBEGIN_HOOK_INSTALLED.load(Ordering::Relaxed) {
        return true;
    }
    let target = crate::rebase(FRAMEBEGIN_INVOKE_VM);
    if target.is_null() || !crate::memoria::is_readable(target as *const c_void, 16) {
        return false;
    }
    if ANIM_FRAMEBEGIN_HOOK_INSTALLED.swap(true, Ordering::Relaxed) {
        return true;
    }
    // Roda todo quadro (thread de job): threads paradas e trampolim publicado antes do salto, como os hooks de evento (boots 178/181).
    match crate::ganchos::instalar_com_threads_paradas(target, bwms_anim_framebegin_hook as *mut c_void, Some(&ORIG_ANIM_FRAMEBEGIN_INVOKE)) {
        Some(_) => {
            crate::veredito_dev!(
                "[pipeline] gancho no invoke de {} (grupo 0) @ {target:p} (estático {FRAMEBEGIN_INVOKE_VM:#x}, __text)",
                EVENTOS_DO_PIPELINE[0]
            );
            instalar_estagios();
            true
        }
        None => {
            ANIM_FRAMEBEGIN_HOOK_INSTALLED.store(false, Ordering::Relaxed);
            crate::log_dev!("[registro-de-update-invoke] FALHA ao instalar hook no invoke do FrameBegin");
            false
        }
    }
}


/// Opção A: latch local barato pra parar de rechecar o marcador/estado a cada tick
/// depois que `install_anim_framebegin_hook_if_ready()` (selftest.rs) já confirmou instalado.
pub(crate) static ANIM_FRAMEBEGIN_HOOK_INSTALLED_TICK: AtomicBool = AtomicBool::new(false);


#[cfg(test)]
mod frame_info_provado {
    use super::{guardar_frame_info, DETALHES_PROVADOS, FRAME_DELTA, FRAME_DILATACAO};
    use std::ffi::c_void;
    use std::ptr::addr_of_mut;
    use std::sync::atomic::Ordering;

    /// `FrameDetailedInfo` do SDK (0x18): `deltaTime` em `+0x00`, `timeDilation` em `+0x04`.
    #[repr(C)]
    struct Detalhes {
        delta: f32,
        dilatacao: f32,
        resto: [u64; 2],
    }

    /// `FrameInfo` do SDK (0x18): `deltaTime` em `+0x00`, `details` em `+0x08`.
    #[repr(C)]
    struct Quadro {
        delta: f32,
        pad: u32,
        detalhes: u64,
        resto: u64,
    }

    fn idas_ao_kernel() -> u64 {
        crate::memoria::IS_READABLE_CHAMADAS.with(|c| c.get())
    }

    /// O que `BwmsFrameDeltaTime` e `BwmsFrameTimeDilation` devolvem: os dois atômicos lidos como `f32`.
    fn lido_pelas_natives() -> (f32, f32) {
        (f32::from_bits(FRAME_DELTA.load(Ordering::Relaxed)), f32::from_bits(FRAME_DILATACAO.load(Ordering::Relaxed)))
    }

    /// Os 3 ganchos de um frame: `FrameBegin` (0), `PreBuckets` (1), `PreRenderUpdate` (2).
    unsafe fn um_frame(fi: *mut c_void) {
        for estagio in 0..3 {
            guardar_frame_info(fi, estagio);
        }
    }

    #[test]
    fn detalhes_provado_uma_vez_por_ponteiro_e_estagio() {
        for slot in &DETALHES_PROVADOS {
            slot.store(0, Ordering::Relaxed);
        }
        let mut a = Detalhes { delta: 0.0, dilatacao: 0.5, resto: [0; 2] };
        let mut b = Detalhes { delta: 0.0, dilatacao: 0.25, resto: [0; 2] };
        let pa = addr_of_mut!(a);
        let pb = addr_of_mut!(b);
        let mut q = Quadro { delta: 0.016, pad: 0, detalhes: pa as u64, resto: 0 };
        let pq = addr_of_mut!(q);
        let fi = pq as *mut c_void;
        unsafe {
            let antes = idas_ao_kernel();
            um_frame(fi);
            assert_eq!(idas_ao_kernel() - antes, 3, "1º frame: uma prova do details por estágio, nenhuma do FrameInfo");
            assert_eq!(lido_pelas_natives(), (0.016, 0.5));

            // Mesmo details em todo frame: zero syscall, e os valores continuam lidos a cada chamada (não ficam no cache).
            let antes = idas_ao_kernel();
            for i in 1..=10u32 {
                (*pq).delta = 0.016 + i as f32 * 0.001;
                (*pa).dilatacao = 0.5 + i as f32 * 0.01;
                um_frame(fi);
                assert_eq!(lido_pelas_natives(), (0.016 + i as f32 * 0.001, 0.5 + i as f32 * 0.01));
            }
            assert_eq!(idas_ao_kernel() - antes, 0, "details já provado não volta ao kernel");

            // details novo num estágio: prova de novo só nele.
            (*pq).detalhes = pb as u64;
            let antes = idas_ao_kernel();
            guardar_frame_info(fi, 1);
            assert_eq!(idas_ao_kernel() - antes, 1);
            assert_eq!(lido_pelas_natives().1, 0.25);
            guardar_frame_info(fi, 1);
            assert_eq!(idas_ao_kernel() - antes, 1, "o ponteiro novo ficou provado");

            // details nulo: grava o deltaTime, mantém a dilatação, sem syscall.
            (*pq).detalhes = 0;
            (*pq).delta = 0.033;
            let antes = idas_ao_kernel();
            um_frame(fi);
            assert_eq!(idas_ao_kernel() - antes, 0);
            assert_eq!(lido_pelas_natives(), (0.033, 0.25));

            // details ilegível (__PAGEZERO): não grava a dilatação e não fica provado, então a próxima chamada prova de novo.
            (*pq).detalhes = 0x1000;
            let antes = idas_ao_kernel();
            guardar_frame_info(fi, 2);
            guardar_frame_info(fi, 2);
            assert_eq!(idas_ao_kernel() - antes, 2);
            assert_eq!(lido_pelas_natives(), (0.033, 0.25));
            assert_eq!(DETALHES_PROVADOS[2].load(Ordering::Relaxed), pa as u64, "o ilegível não substitui o último provado");

            // FrameInfo nulo: não toca em nada.
            (*pq).detalhes = pa as u64;
            let antes = idas_ao_kernel();
            guardar_frame_info(std::ptr::null_mut(), 0);
            assert_eq!(idas_ao_kernel() - antes, 0);
            assert_eq!(lido_pelas_natives(), (0.033, 0.25));
        }
    }
}

#[cfg(test)]
mod ganchos_do_pipeline {
    use super::{bwms_anim_framebegin_hook, hook_estagio, AnimInvokeFn, ESTAGIO_ORIG, ORIG_ANIM_FRAMEBEGIN_INVOKE, PIPELINE_ARMADOS};
    use crate::tipos_do_motor::{CALLBACKS, TESTES_DE_CALLBACKS_EM_SERIE};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{mpsc, PoisonError};
    use std::time::Duration;

    static ORIGINAIS: AtomicU64 = AtomicU64::new(0);

    unsafe extern "C" fn original(_: *mut c_void, _: *mut c_void, _: *mut c_void) {
        ORIGINAIS.fetch_add(1, Ordering::Relaxed);
    }

    // `perfil:PT-08`: com o bit do próprio estágio desligado (e os dos outros ligados), os 3 ganchos passam pelo frame sem travar `CALLBACKS`: o teste
    // segura a trava e eles voltam. Com o bit ligado, o gancho despacha, e o despacho trava `CALLBACKS`: fica esperando até o teste soltar. O original
    // roda uma vez nos dois casos. `FrameInfo` nulo deixa `FRAME_DELTA` e `FRAME_DILATACAO` intocados.
    #[test]
    fn gancho_so_trava_callbacks_com_o_bit_do_estagio() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(PoisonError::into_inner);
        ORIG_ANIM_FRAMEBEGIN_INVOKE.store(original as *const () as u64, Ordering::Relaxed);
        for orig in &ESTAGIO_ORIG {
            orig.store(original as *const () as u64, Ordering::Relaxed);
        }
        let ganchos: [AnimInvokeFn; 3] = [bwms_anim_framebegin_hook, hook_estagio::<0>, hook_estagio::<1>];
        let mut lidos = Vec::new();
        for (estagio, gancho) in ganchos.into_iter().enumerate() {
            for armado in [false, true] {
                let bit = 1u8 << estagio;
                PIPELINE_ARMADOS.store(if armado { bit } else { 0b111 & !bit }, Ordering::Relaxed);
                let antes = ORIGINAIS.load(Ordering::Relaxed);
                let trava = CALLBACKS.lock().unwrap_or_else(PoisonError::into_inner);
                let (avisa, voltou) = mpsc::channel();
                let thread = std::thread::spawn(move || {
                    unsafe { gancho(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
                    let _ = avisa.send(());
                });
                let voltou_com_a_trava = voltou.recv_timeout(Duration::from_millis(if armado { 200 } else { 5000 })).is_ok();
                drop(trava);
                thread.join().expect("thread do gancho");
                lidos.push((voltou_com_a_trava, ORIGINAIS.load(Ordering::Relaxed) - antes));
            }
        }
        crate::tipos_do_motor::atualizar_eventos_de_entidade();
        assert_eq!(lidos, [(true, 1), (false, 1)].repeat(3));
    }
}
