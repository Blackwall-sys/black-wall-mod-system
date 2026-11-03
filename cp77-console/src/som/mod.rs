//! Som: bancos de som (soundbanks) carregados, caminho e residência.

#![allow(dead_code)]

use std::sync::atomic::AtomicBool;
use crate::tipos_do_motor::*;
use crate::arquivos_do_jogo::*;
use std::ffi::c_void;
use std::sync::Mutex;
use crate::rtti::{self};
use crate::cname::{CName, ResourcePathHash};
use crate::register::*;

/// Índice de script (`Int32`) como posição na edição; `None` negativo.
pub(crate) fn soundbank_indice(v: u64) -> Option<usize> {
    usize::try_from(v as u32 as i32).ok()
}

/// Lê o banco `indice` da edição corrente.
pub(crate) fn soundbank_ler<T>(indice: u64, f: impl Fn(&(u64, bool, u64)) -> T) -> Option<T> {
    let edicao = SOUNDBANKS_EDICAO.lock().ok()?;
    edicao.as_ref()?.get(soundbank_indice(indice)?).map(f)
}

/// Aplica `f` no banco `indice` da edição corrente; `false` fora do despacho ou fora da faixa.
pub(crate) fn soundbank_editar(indice: u64, f: impl FnOnce(&mut (u64, bool, u64))) -> bool {
    let Ok(mut edicao) = SOUNDBANKS_EDICAO.lock() else { return false };
    match edicao.as_mut().and_then(|v| soundbank_indice(indice).and_then(|k| v.get_mut(k))) {
        Some(b) => {
            f(b);
            true
        }
        None => false,
    }
}

/// `BwmsSoundBankCount() -> Int32`.
pub(crate) unsafe extern "C" fn tramp_soundbank_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let _ = rtti::read_params_consuming(current_native_func(), frame);
    let n = SOUNDBANKS_EDICAO.lock().ok().and_then(|e| e.as_ref().map(|v| v.len())).unwrap_or(0);
    write_uint_ret(out, n as u64, 4);
}

/// `BwmsSoundBankName(index) -> CName`.
pub(crate) unsafe extern "C" fn tramp_soundbank_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    write_uint_ret(out, a.first().and_then(|(i, _)| soundbank_ler(*i, |b| b.0)).unwrap_or(0), 8);
}

/// `BwmsSoundBankIsResident(index) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_soundbank_is_resident(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    write_bool_ret(out, a.first().and_then(|(i, _)| soundbank_ler(*i, |b| b.1)).unwrap_or(false));
}

/// `BwmsSoundBankPath(index) -> Uint64` (hash do caminho, a forma do `ResourceEvent.GetPath`).
pub(crate) unsafe extern "C" fn tramp_soundbank_path(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    write_uint_ret(out, a.first().and_then(|(i, _)| soundbank_ler(*i, |b| b.2)).unwrap_or(0), 8);
}

/// `BwmsSoundBankSetResident(index, value) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_soundbank_set_resident(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let valor = a.get(1).is_some_and(|(v, _)| rtti::bool_do_script(*v));
    write_bool_ret(out, a.first().is_some_and(|(i, _)| soundbank_editar(*i, |b| b.1 = valor)));
}

/// `BwmsSoundBankSetPath(index, path) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_soundbank_set_path(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, textos) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let caminho = textos.get(1).and_then(|t| t.as_deref()).map(bwms_primitives::resource_path_hash);
    write_bool_ret(out, matches!((a.first(), caminho), (Some((i, _)), Some(h)) if soundbank_editar(*i, |b| b.2 = h)));
}

/// `BwmsSoundBankAdd(name, isResident, path) -> Int32`: índice do banco (o existente com esse nome recebe os campos, como na escrita de volta da
/// referência); -1 fora do despacho.
pub(crate) unsafe extern "C" fn tramp_soundbank_add(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, textos) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let nome = a.first().map_or(0, |(v, _)| *v);
    let residente = a.get(1).is_some_and(|(v, _)| rtti::bool_do_script(*v));
    let caminho = textos.get(2).and_then(|t| t.as_deref()).map_or(0, bwms_primitives::resource_path_hash);
    let indice = SOUNDBANKS_EDICAO.lock().ok().and_then(|mut e| {
        let v = e.as_mut()?;
        match v.iter().position(|b| b.0 == nome) {
            Some(k) => {
                v[k] = (nome, residente, caminho);
                Some(k)
            }
            None => {
                v.push((nome, residente, caminho));
                Some(v.len() - 1)
            }
        }
    });
    write_uint_ret(out, indice.map_or(u32::MAX as u64, |k| k as u64), 4);
}


/// Áudio: os bancos da edição do `Resource/Load` dos soundbanks — `(nome, residente, hash do caminho)` —, só durante o despacho.
pub(crate) static SOUNDBANKS_EDICAO: Mutex<Option<Vec<(u64, bool, u64)>>> = Mutex::new(None);

/// Áudio: `Resource/Load` dos soundbanks com um `BwmsSoundBanks` (classe de script do `bwms-soundbanks.reds`) no `GetResource`; os
/// ouvintes editam [`SOUNDBANKS_EDICAO`] pelos métodos dele. 0 sem a classe ou sem ouvinte.
pub unsafe fn disparar_soundbanks(caminho: u64) -> usize {
    let Some(reg) = rtti::Registry::obtain() else {
        return 0;
    };
    let obj = rtti::new_object(&reg, "BwmsSoundBanks");
    if obj.is_null() {
        return 0;
    }
    let mut ho = [0u8; 16];
    rtti::make_handle(ho.as_mut_ptr() as *mut c_void, obj);
    let mut objeto = [(ho.as_ptr() as *const u64).read_unaligned(), (ho.as_ptr().add(8) as *const u64).read_unaligned()];
    let Some(arg) = make_resourceevent_arg(caminho) else {
        crate::jogo::objetos::soltar_handle(&mut objeto);
        return 0;
    };
    let (inst, refc) = match &arg {
        rtti::Arg::Handle(i, r) => (*i as u64, *r as u64),
        _ => (0, 0),
    };
    if let Ok(mut r) = RESOURCEEVENT_RECURSOS.lock() {
        r.push((inst as usize, objeto));
    }
    let enviados = fire_event_args("Resource/Load", &[arg]);
    if let Ok(mut s) = RESOURCEEVENT_STATES.lock() {
        s.retain(|(p, _)| *p as u64 != inst);
    }
    let mut hr = RESOURCEEVENT_RECURSOS.lock().ok().and_then(|mut r| r.iter().position(|(p, _)| *p as u64 == inst).map(|i| r.remove(i).1)).unwrap_or([0, 0]);
    crate::jogo::objetos::soltar_handle(&mut hr);
    let mut he = [inst, refc];
    crate::jogo::objetos::soltar_handle(&mut he);
    enviados
}


/// Reenvio dos soundbanks pedido por um `RegisterCallback` de `Resource/Load`.
pub(crate) static BANCOS_DE_SOM_REENVIO_PENDENTE: AtomicBool = AtomicBool::new(false);


// ===== Reenvio dos soundbanks: sonda do `ReadSoundBanksJson` e o tick que despacha `Resource/Load` dos soundbanks. =====
use std::sync::atomic::{AtomicU64, Ordering};
use crate::ganchos::{instalar_conferindo_prologo, original_publicado};
use crate::marcador_individual;

// Sonda de áudio (`~/.bwms-hook-c122som-teste`): `ReadSoundBanksJson` (`0x1051d4f30`) só observando, instalado no `on_load` — quando roda em
// relação à carga da dylib, se já havia ouvinte de `Resource/Load` e quantos bancos o `RegisteredBanks` (`+0x68`, tamanho `+0x70`) tem antes e depois.
// Decide se um ouvinte registrado por script alcança o despacho dos soundbanks.
const READ_SOUNDBANKS_JSON_VM: u64 = 0x1_051d_4f30;
const READ_SOUNDBANKS_JSON_PROLOGO: u32 = 0xd106_c3ff;
static ORIG_READ_SOUNDBANKS_JSON: AtomicU64 = AtomicU64::new(0);
/// (ms desde a carga da dylib, bancos antes, bancos depois, retorno, ouvinte de `Resource/Load`) de cada chamada, até 4.
static BANCOS_DE_SOM_LEITURAS: std::sync::Mutex<Vec<(u128, u32, u32, u64, bool)>> = std::sync::Mutex::new(Vec::new());
static BANCOS_DE_SOM_CARGA_DA_DYLIB: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
/// Manager dos soundbanks visto na leitura (0 antes dela).
pub(crate) static BANCOS_DE_SOM_MANAGER: AtomicU64 = AtomicU64::new(0);
/// Reenvios feitos e bancos novos inseridos no mapa do motor.
pub(crate) static BANCOS_DE_SOM_REENVIOS: AtomicU64 = AtomicU64::new(0);
pub(crate) static BANCOS_DE_SOM_INSERIDOS: AtomicU64 = AtomicU64::new(0);

/// Tamanho do `RegisteredBanks` do manager; `u32::MAX` ilegível.
pub(crate) unsafe fn bancos_registrados(manager: *const u8) -> u32 {
    let campo = manager.wrapping_add(0x70) as *const u32;
    if manager.is_null() || !crate::memoria::is_readable(campo as *const c_void, 4) {
        return u32::MAX;
    }
    campo.read_unaligned()
}

unsafe extern "C" fn read_soundbanks_json_hook(manager: *mut u8) -> u64 {
    let antes = bancos_registrados(manager);
    let f: unsafe extern "C" fn(*mut u8) -> u64 = std::mem::transmute(original_publicado(&ORIG_READ_SOUNDBANKS_JSON));
    let r = f(manager);
    BANCOS_DE_SOM_MANAGER.store(manager as u64, Ordering::Relaxed);
    let ms = BANCOS_DE_SOM_CARGA_DA_DYLIB.get().map_or(0, |t| t.elapsed().as_millis());
    if let Ok(mut v) = BANCOS_DE_SOM_LEITURAS.lock() {
        if v.len() < 4 {
            v.push((ms, antes, bancos_registrados(manager), r & 0xff, crate::tipos_do_motor::tem_ouvinte("Resource/Load")));
        }
    }
    r
}

/// No `on_load`: marca o tempo da carga e instala a sonda do `ReadSoundBanksJson` com o prólogo conferido — com `bwms-soundbanks.reds` no bundle (o
/// reenvio dos soundbanks precisa do manager visto na leitura, que roda antes de qualquer script) ou `~/.bwms-hook-c122som-teste`.
pub(crate) unsafe fn install_read_soundbanks_json_hook() {
    BANCOS_DE_SOM_CARGA_DA_DYLIB.get_or_init(std::time::Instant::now);
    let no_bundle = crate::dylib_dir().is_some_and(|d| std::path::Path::new(&d).join("../r6/scripts/blackwall-mods/bwms-soundbanks.reds").exists());
    if !no_bundle && !marcador_individual("c122som-teste") {
        return;
    }
    let r = instalar_conferindo_prologo(READ_SOUNDBANKS_JSON_VM, READ_SOUNDBANKS_JSON_PROLOGO, read_soundbanks_json_hook as *mut c_void, &ORIG_READ_SOUNDBANKS_JSON);
    crate::veredito_dev!("[c122som] sonda do ReadSoundBanksJson no on_load: {r:?}");
}

#[cfg(feature = "devtools")]
pub(crate) fn c122som_teste_tick() {
    static FEITO: AtomicBool = AtomicBool::new(false);
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    if FEITO.load(Ordering::Relaxed) || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 60) || !marcador_individual("c122som-teste") || !crate::inicio::boot::gameplay_real() {
        return;
    }
    FEITO.store(true, Ordering::Relaxed);
    let chamadas = BANCOS_DE_SOM_LEITURAS.lock().map(|v| v.clone()).unwrap_or_default();
    let gameplay_ms = BANCOS_DE_SOM_CARGA_DA_DYLIB.get().map_or(0, |t| t.elapsed().as_millis());
    let ok = !chamadas.is_empty() && chamadas.iter().all(|c| c.2 != u32::MAX && c.2 >= c.1 && c.2 > 0);
    crate::veredito_dev!(
        "[c122som] VEREDITO-122SOM chamadas (ms desde a carga, bancos antes, depois, retorno, ouvinte de Resource/Load) {chamadas:?}; gameplay em {gameplay_ms} ms; som-ok={ok}"
    );
}

// Áudio: reenvio dos soundbanks. A leitura roda ~3 s depois da carga da dylib, antes de qualquer script (boot 185), então cada
// `RegisterCallback` de `Resource/Load` pede um reenvio. No tick, com o manager visto na leitura e ouvinte, os bancos do `RegisteredBanks` viram a edição
// do `BwmsSoundBanks`, o `Resource/Load` sai com o hash do `soundbanks.json`, e o que os ouvintes mudaram volta pro mapa: banco existente recebe residente e
// caminho; nome novo entra pela inserção do motor (`0x1051d71dc`), com a entrada de 0x18 B no `PoolAudioSystems` e o bloco de contagem no `PoolRefCount`
// (forte 1, fraco 1), como o `ReadSoundBanksJson` faz.
const INSERIR_BANCO_DE_SOM_VM: u64 = 0x1_051d_71dc;
const POOL_AUDIO_ALOCAR_VM: u64 = 0x1_052a_58a0;
pub(crate) const SOUNDBANKS_JSON_CAMINHO: &str = "base\\sound\\event\\soundbanks.json";

/// `(nome, entrada, bloco)` dos bancos do `RegisteredBanks` (`manager+0x68`): tabela de índices `+0x00` (`-1` vazio), buckets `+0x0c`, nós `+0x10` com
/// passo `+0x1c`, nó `{próximo u32, hash u32, chave u64, SharedPtr}`. Vazio com o mapa ilegível.
pub(crate) unsafe fn mapa_de_bancos_de_som(manager: u64) -> Vec<(u64, u64, u64)> {
    let mapa = (manager + 0x68) as *const u8;
    if manager == 0 || !crate::memoria::is_readable(mapa as *const c_void, 0x20) {
        return Vec::new();
    }
    let tabela = (mapa as *const u64).read_unaligned() as *const u32;
    let buckets = (mapa.add(0x0c) as *const u32).read_unaligned() as usize;
    let nos = (mapa.add(0x10) as *const u64).read_unaligned() as *const u8;
    let passo = (mapa.add(0x1c) as *const u32).read_unaligned() as usize;
    if tabela.is_null() || nos.is_null() || buckets == 0 || buckets > 1 << 16 || passo < 0x20 || !crate::memoria::is_readable(tabela as *const c_void, buckets * 4) {
        return Vec::new();
    }
    let mut bancos = Vec::new();
    for b in 0..buckets {
        let mut idx = tabela.add(b).read_unaligned();
        let mut voltas = 0;
        while idx != u32::MAX && voltas < 4096 {
            let no = nos.add(idx as usize * passo);
            if !crate::memoria::is_readable(no as *const c_void, 0x20) {
                break;
            }
            bancos.push(((no.add(8) as *const u64).read_unaligned(), (no.add(0x10) as *const u64).read_unaligned(), (no.add(0x18) as *const u64).read_unaligned()));
            idx = (no as *const u32).read_unaligned();
            voltas += 1;
        }
    }
    bancos
}

/// `(residente, hash do caminho)` de uma `SoundBankEntry`.
pub(crate) unsafe fn campos_do_banco_de_som(entrada: u64) -> Option<(bool, u64)> {
    (entrada != 0 && crate::memoria::is_readable(entrada as *const c_void, 0x18)).then(|| (((entrada + 8) as *const u8).read() != 0, ((entrada + 0x10) as *const u64).read_unaligned()))
}

/// Banco novo no `RegisteredBanks` pela inserção do motor; `true` inserido.
unsafe fn inserir_banco_de_som(manager: u64, nome: CName, residente: bool, caminho: ResourcePathHash) -> bool {
    let (nome, caminho) = (nome.valor(), caminho.valor());
    let alocar_audio: unsafe extern "C" fn(u64) -> *mut u8 = std::mem::transmute(crate::rebase(POOL_AUDIO_ALOCAR_VM));
    let alocar_contagem: unsafe extern "C" fn(u64) -> *mut u8 = std::mem::transmute(crate::rebase(crate::jogo::objetos::CONTAGEM_ALOCAR_VM));
    let entrada = alocar_audio(0x18);
    let bloco = if entrada.is_null() { std::ptr::null_mut() } else { alocar_contagem(8) };
    if bloco.is_null() {
        return false;
    }
    std::ptr::write_bytes(entrada, 0, 0x18);
    (entrada as *mut u64).write_unaligned(nome);
    entrada.add(8).write(residente as u8);
    (entrada.add(0x10) as *mut u64).write_unaligned(caminho);
    (bloco as *mut u64).write_unaligned(0x0000_0001_0000_0001);
    let chave = nome;
    let compartilhado = [entrada as u64, bloco as u64];
    let mut saida = [0u64; 4];
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(INSERIR_BANCO_DE_SOM_VM) as u64,
        in("x0") manager + 0x68,
        in("x1") &chave as *const u64 as u64,
        in("x2") compartilhado.as_ptr() as u64,
        in("x8") saida.as_mut_ptr() as u64,
        clobber_abi("C"),
    );
    // A referência local sai; o mapa ficou com a dele.
    (*(bloco as *const std::sync::atomic::AtomicU32)).fetch_sub(1, Ordering::SeqCst);
    saida[3] & 0xff != 0
}

/// No tick (thread do jogo): com reenvio pedido, manager visto e ouvinte de `Resource/Load`, despacha os soundbanks e escreve de volta.
pub(crate) fn bancos_de_som_tick() {
    if !BANCOS_DE_SOM_REENVIO_PENDENTE.swap(false, Ordering::Relaxed) {
        return;
    }
    let manager = BANCOS_DE_SOM_MANAGER.load(Ordering::Relaxed);
    if manager != 0 && crate::tipos_do_motor::tem_ouvinte("Resource/Load") {
        unsafe { reenviar_bancos_de_som(manager) };
    }
}

/// Despacha os soundbanks do `manager` aos ouvintes de `Resource/Load` e grava de volta o que eles mudaram.
unsafe fn reenviar_bancos_de_som(manager: u64) {
    let mapa = mapa_de_bancos_de_som(manager);
    if mapa.is_empty() {
        return;
    }
    let edicao: Vec<(u64, bool, u64)> =
        mapa.iter().map(|&(nome, entrada, _)| campos_do_banco_de_som(entrada).map_or((nome, false, 0), |(residente, caminho)| (nome, residente, caminho))).collect();
    match SOUNDBANKS_EDICAO.lock() {
        Ok(mut e) => *e = Some(edicao),
        Err(_) => return,
    }
    let enviados = disparar_soundbanks(bwms_primitives::resource_path_hash(SOUNDBANKS_JSON_CAMINHO));
    let editados = SOUNDBANKS_EDICAO.lock().ok().and_then(|mut e| e.take()).unwrap_or_default();
    BANCOS_DE_SOM_REENVIOS.fetch_add(1, Ordering::Relaxed);
    if enviados == 0 {
        return;
    }
    let (mut mudados, mut inseridos) = (0u64, 0u64);
    for (nome, residente, caminho) in editados {
        match mapa.iter().find(|b| b.0 == nome) {
            Some(&(_, entrada, _)) => {
                if campos_do_banco_de_som(entrada).is_some_and(|atual| atual != (residente, caminho)) {
                    ((entrada + 8) as *mut u8).write(residente as u8);
                    ((entrada + 0x10) as *mut u64).write_unaligned(caminho);
                    mudados += 1;
                }
            }
            None => inseridos += inserir_banco_de_som(manager, CName::lido_do_motor(nome), residente, ResourcePathHash::lido_do_motor(caminho)) as u64,
        }
    }
    BANCOS_DE_SOM_INSERIDOS.fetch_add(inseridos, Ordering::Relaxed);
    crate::log_dev!("[c122som] soundbanks reenviados a {enviados} ouvinte(s): {} banco(s) no mapa, {mudados} mudado(s), {inseridos} inserido(s)", mapa.len());
}

