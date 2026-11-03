//! Tarefas (jobs) do motor: criar fila, despachar, esperar e finalizar.

#![allow(dead_code)]

// Contagem dos corpos e das threads de job que só o selftest lê.
#[cfg(feature = "devtools")]
use crate::diagnostico::{JOB_CORPOS, JOB_THREADS};
#[cfg(feature = "devtools")]
use crate::aparencia::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use crate::rtti::{self, Registry};
use crate::register::*;

// ===== Codeware `#204` — `BwmsJobQueue` (`bwms-job-queue.reds`) =====
//
// O `Handle` de um objeto de script vira job no pool do motor pela fila (`JobQueue`) do 48b: descritor {corpo, alvo, família} no despachante
// `0x104ddf514`, e o corpo recebe o grupo do job. Sem tamanho, um job; com tamanho, um job por lote cobrindo [0, size) e, depois do `SyncWait` da fila,
// o `Finish`. A referência embrulha o mesmo `Red::JobQueue` e chama o método pelo `CallFunction`. As filas ficam aqui por id: `Finalize` captura o job
// da fila e fecha; `SyncWaitUntilFinished` fecha e espera o contador de jobs pendentes dela até o tempo acabar.
pub(crate) struct JobDeScript {
    pub(crate) objeto: [u64; 2],
    pub(crate) funcao: *mut c_void,
    pub(crate) args: [i32; 2],
    pub(crate) quantos: usize,
    pub(crate) com_grupo: bool,
    pub(crate) pendentes: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

pub(crate) struct FilaDeScript {
    pub(crate) fila: [u64; 16],
    pub(crate) pendentes: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

pub(crate) static FILAS_DE_SCRIPT: Mutex<Vec<(u64, Box<FilaDeScript>)>> = Mutex::new(Vec::new());

/// Jobs capturados por `Finalize`, com endereço estável pro `Wait` do motor. Não são soltos: o destrutor de `JobHandle` não tem endereço.
pub(crate) static JOBS_CAPTURADOS: Mutex<Vec<(u64, Box<u64>)>> = Mutex::new(Vec::new());

pub(crate) static JOB_PROXIMO_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) unsafe extern "C" fn job_de_script_corpo(alvo: *mut c_void, grupo: *const c_void) {
    crate::jogo::fila::gravar_param_de_thread_do_job(grupo, false);
    let mut job = Box::from_raw(alvo as *mut JobDeScript);
    let mut args: Vec<rtti::Arg> = job.args[..job.quantos].iter().map(|v| rtti::Arg::I32(*v as u32)).collect();
    if job.com_grupo {
        args.push(rtti::Arg::I64(grupo as u64));
    }
    let rf = rtti::ResolvedFn { func: job.funcao, ret_type: std::ptr::null_mut(), is_static: false };
    rtti::call_func(&rf, job.objeto[0] as *mut c_void, &args);
    #[cfg(feature = "devtools")]
    {
        JOB_CORPOS.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut t) = JOB_THREADS.lock() {
            let id = std::thread::current().id();
            if t.len() < 64 && !t.contains(&id) {
                t.push(id);
            }
        }
    }
    crate::jogo::objetos::soltar_handle(&mut job.objeto);
    job.pendentes.fetch_sub(1, Ordering::SeqCst);
    crate::jogo::fila::gravar_param_de_thread_do_job(grupo, true);
}

/// Método `nome` do objeto com `int32` parâmetros `Int32` e, opcional no fim, o `Uint64` do grupo: `(função, com grupo)`; `None` sem o método ou com
/// outra assinatura.
pub(crate) unsafe fn job_metodo(obj: *mut c_void, nome: &str, int32: usize) -> Option<(*mut c_void, bool)> {
    let rf = rtti::resolve_in_class(rtti::class_of(obj), nome)?;
    let n = rtti::param_count(&rf) as usize;
    if n != int32 && n != int32 + 1 {
        return None;
    }
    let (t_int32, t_uint64) = (crate::cname::cname("Int32"), crate::cname::cname("Uint64"));
    if (0..int32).any(|i| rtti::fn_param_type(rf.func, i) != t_int32) || (n == int32 + 1 && rtti::fn_param_type(rf.func, int32) != t_uint64) {
        return None;
    }
    Some((rf.func, n == int32 + 1))
}

pub(crate) unsafe fn fila_de_script<R>(id: u64, f: impl FnOnce(&mut FilaDeScript) -> R) -> Option<R> {
    let mut filas = FILAS_DE_SCRIPT.lock().ok()?;
    let (_, fila) = filas.iter_mut().find(|(i, _)| *i == id)?;
    Some(f(fila))
}

pub(crate) fn fila_de_script_tirar(id: u64) -> Option<Box<FilaDeScript>> {
    let mut filas = FILAS_DE_SCRIPT.lock().ok()?;
    let k = filas.iter().position(|(i, _)| *i == id)?;
    Some(filas.swap_remove(k).1)
}

/// Um job de `funcao` no objeto, com um Handle forte dele até o corpo rodar.
pub(crate) unsafe fn job_despachar(fila: &mut FilaDeScript, obj: *mut c_void, funcao: *mut c_void, args: [i32; 2], quantos: usize, com_grupo: bool) {
    let mut objeto = [0u64; 2];
    rtti::make_handle(objeto.as_mut_ptr() as *mut c_void, obj);
    fila.pendentes.fetch_add(1, Ordering::SeqCst);
    let job = Box::new(JobDeScript { objeto, funcao, args, quantos, com_grupo, pendentes: fila.pendentes.clone() });
    crate::jogo::fila::fila_despachar_solto(&mut fila.fila, job_de_script_corpo, Box::into_raw(job) as *mut c_void);
}

/// `BwmsJobQueueCreate(group) -> Uint64`: fila nova (grupo 0) ou no grupo do job atual; devolve o id.
pub(crate) unsafe extern "C" fn tramp_job_queue_create(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let grupo = a.first().map_or(0, |(v, _)| *v);
    let fila = Box::new(FilaDeScript { fila: crate::jogo::fila::fila_de_jobs(grupo as *const c_void), pendentes: Default::default() });
    let id = JOB_PROXIMO_ID.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut filas) = FILAS_DE_SCRIPT.lock() {
        filas.push((id, fila));
    }
    write_uint_ret(out, id, 8);
}

/// `BwmsJobQueueDispatch(queue, job, size, batch) -> Bool`: sem tamanho, `Handle()` ou `Handle(group)`; com tamanho, `Handle(from, to[, group])` por lote
/// (lote 0 = até 16 jobs) e `Finish(size[, group])` ou `Finish()` depois. Falso sem fila, sem objeto ou com assinatura que não bate.
pub(crate) unsafe extern "C" fn tramp_job_queue_dispatch(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let g = |k: usize| a.get(k).map_or(0, |(v, _)| *v);
    let (id, obj, tamanho, lote) = (g(0), g(1) as *mut c_void, g(2) as u32 as i32, g(3) as u32 as i32);
    let ok = !obj.is_null()
        && rtti::sane(obj)
        && fila_de_script(id, |fila| {
            if tamanho <= 0 {
                let Some((handle, grupo)) = job_metodo(obj, "Handle", 0) else { return false };
                job_despachar(fila, obj, handle, [0, 0], 0, grupo);
                return true;
            }
            let Some((handle, grupo)) = job_metodo(obj, "Handle", 2) else { return false };
            let fim = job_metodo(obj, "Finish", 1).map(|(f, g)| (f, 1, g)).or_else(|| job_metodo(obj, "Finish", 0).map(|(f, g)| (f, 0, g)));
            let passo = if lote > 0 { lote } else { (tamanho + 15) / 16 }.max(1);
            let mut de = 0;
            while de < tamanho {
                let ate = (de + passo).min(tamanho);
                job_despachar(fila, obj, handle, [de, ate], 2, grupo);
                de = ate;
            }
            if let Some((finish, quantos, grupo_fim)) = fim {
                crate::jogo::fila::fila_sincronizar(&mut fila.fila);
                job_despachar(fila, obj, finish, [tamanho, 0], quantos, grupo_fim);
            }
            true
        })
        .unwrap_or(false);
    write_bool_ret(out, ok);
}

/// `BwmsJobQueueWait(queue, job) -> Bool`: os próximos jobs da fila esperam o job capturado por `Finalize`.
pub(crate) unsafe extern "C" fn tramp_job_queue_wait(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let g = |k: usize| a.get(k).map_or(0, |(v, _)| *v);
    let (id, job) = (g(0), g(1));
    let ok = JOBS_CAPTURADOS.lock().is_ok_and(|jobs| {
        jobs.iter()
            .find(|(i, _)| *i == job)
            .is_some_and(|(_, h)| fila_de_script(id, |fila| crate::jogo::fila::fila_esperar(&mut fila.fila, &**h as *const u64)).is_some())
    });
    write_bool_ret(out, ok);
}

/// `BwmsJobQueueFinalize(queue) -> Uint64`: captura o job da fila, fecha a fila e devolve o id do job (0 sem fila).
pub(crate) unsafe extern "C" fn tramp_job_queue_finalize(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let id = a.first().map_or(0, |(v, _)| *v);
    let job = match fila_de_script_tirar(id) {
        Some(mut fila) => {
            let h = crate::jogo::fila::fila_capturar_e_fechar(&mut fila.fila);
            let jid = JOB_PROXIMO_ID.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut jobs) = JOBS_CAPTURADOS.lock() {
                jobs.push((jid, Box::new(h)));
            }
            jid
        }
        None => 0,
    };
    write_uint_ret(out, job, 8);
}

/// `BwmsJobQueueSyncWait(queue, timeout) -> Bool`: fecha a fila e espera os jobs dela terminarem (timeout 0 = 60 s); verdadeiro se terminaram.
pub(crate) unsafe extern "C" fn tramp_job_queue_sync_wait(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let g = |k: usize| a.get(k).map_or(0, |(v, _)| *v);
    let (id, tempo) = (g(0), g(1) as u32);
    let ok = match fila_de_script_tirar(id) {
        Some(mut fila) => {
            let pendentes = fila.pendentes.clone();
            crate::jogo::fila::fila_fechar(&mut fila.fila);
            let limite = std::time::Duration::from_millis(if tempo == 0 { 60_000 } else { tempo as u64 });
            let inicio = std::time::Instant::now();
            while pendentes.load(Ordering::SeqCst) != 0 && inicio.elapsed() < limite {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            pendentes.load(Ordering::SeqCst) == 0
        }
        None => false,
    };
    write_bool_ret(out, ok);
}


/// `IsDone` do JobHandle de 8 B (0 = sem job).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn jobhandle_concluido(job: u64) -> bool {
    if job == 0 {
        return true;
    }
    let concluido: unsafe extern "C" fn(*const u64) -> bool = std::mem::transmute(crate::rebase(crate::jogo::fila::TAREFA_TERMINOU_VM));
    concluido(&job)
}

/// Espera o job por até `ms`; devolve se concluiu.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn jobhandle_esperar(job: u64, ms: u64) -> bool {
    let fim = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    while !jobhandle_concluido(job) {
        if std::time::Instant::now() >= fim {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    true
}

/// Destrói o JobHandle (solta a referência ao job) e zera.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn jobhandle_soltar(job: &mut u64) {
    if *job != 0 {
        let destruir: unsafe extern "C" fn(*mut u64) = std::mem::transmute(crate::rebase(JOBHANDLE_DESTRUIR_VM));
        destruir(job);
        *job = 0;
    }
}

// ===== `JobQueue` — Codeware `#23`: fachada estática, atalho SÍNCRONO =====
//
// `JobQueue.DispatchJob(job)` resolve `Handle` no objeto e chama sem argumento, na hora, dentro da própria chamada (`rtti::call_func`); `true` se achou
// e chamou. A fila real do motor é o `BwmsJobQueue` acima: pool de threads, lotes `Handle(de, até)` + `Finish`, `Wait`, `Finalize` com o JobHandle e
// `SyncWait`. `SyncWaitUntilFinished` aqui é no-op porque o trabalho já terminou quando `DispatchJob` devolve. Classe facade 100% estática
// (`register_type_min` + `register_facade_methods`): sem instância, sem forja com parent.
pub(crate) unsafe extern "C" fn tramp_jobqueue_dispatchjob(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let job = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut ok = false;
    if !job.is_null() && rtti::sane(job) {
        if let Some(rf) = rtti::resolve_in_class(rtti::class_of(job), "Handle") {
            if rtti::call_func(&rf, job, &[]).is_some() {
                ok = true;
            }
        }
    }
    crate::log_dev!(
        "[jobqueue] DispatchJob(job={job:p}) -> ok={ok} — SÍNCRONO (chama Handle() na hora, sem thread pool real; ver nota de divergência)"
    );
    write_bool_ret(out, ok);
}

/// `SyncWaitUntilFinished(opt timeout)` — NO-OP documentado: `DispatchJob` já roda `Handle()`
/// sincronamente antes de devolver, então quando um mod chega a chamar `SyncWaitUntilFinished` o
/// trabalho já terminou (ou nunca foi despachado — nesse caso não há nada a esperar mesmo).
pub(crate) unsafe extern "C" fn tramp_jobqueue_syncwait(_c: *mut c_void, _f: *mut c_void, _out: *mut c_void, _rt: i64) {
    crate::log_dev!("[jobqueue] SyncWaitUntilFinished() -> (no-op, DispatchJob já é síncrono)");
}

pub unsafe fn register_jobqueue_facade(reg: &Registry) -> String {
    let _forged = register_type_min(reg, "JobQueue");
    register_facade_methods(
        reg,
        "JobQueue",
        &[
            ("DispatchJob", tramp_jobqueue_dispatchjob),
            ("SyncWaitUntilFinished", tramp_jobqueue_syncwait),
        ],
    )
}


