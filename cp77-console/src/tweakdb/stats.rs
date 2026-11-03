//! Stats novos de mod (TweakXL `#7`): um record `Stat` com `enumName` que o enum `gamedataStatType` não tem vira um stat de verdade — o bônus
//! próprio de um cyberware ou perk de mod. O `StatsDataSystem` do motor guarda, por valor do enum, o id do record (`statRecords`) e a faixa e as
//! flags (`params`). O BWMS põe a constante no enum, o id em `statRecords` e o valor no record; os params o motor calcula do próprio record.
//!
//! RE (Steam 2.5.0):
//! - `InitializeRecords` (`0x1023b2f68`): reserva 0x6B0 e empurra `TweakDBID("BaseStats." + nome do valor i)`, i em 0..0x6B0, em `statRecords`
//!   (`DynArray<TweakDBID>` em `+0xD8`, tamanho em `+0xE4`).
//! - `InitializeParams` (`0x1023b29a0`): pra cada id de `statRecords`, pelo tamanho do array e não pelo 0x6B0, `{min, max, flags}` do record
//!   (`+0xC0`, `+0xCC` e os `CName` de `+0x90`; faixa aberta quando min >= max), empurrado em `params` (12 B, `+0xE8`, tamanho em `+0xF4`). Os
//!   dois rodam em sequência (`0x1023b4bec`/`0x1023b4bf4`).
//! - `ReinitializeParams` (`0x1023b2960`): trava de escrita no byte `+0xFC`, zera `params`, chama o `InitializeParams`, solta a trava e relê 4
//!   floats do TweakDB — o caminho do próprio motor pra recalcular depois de mudar o TweakDB.
//! - `CheckStatFlag` (`0x1023b3438`), `GetStatFlags` (`0x1023b34b4`) e `GetStatRange` (`0x1023b3cb0`), chamados só pela vtable: teto fixo
//!   `cmp w1, #0x6af` antes de ler `params` sob a trava de leitura (`0x1000015c8`, solta com -1 no byte). Acima do teto saem flags 0 e faixa
//!   (0, 0), que prende o valor de um stat novo em 0.
//! - 0x6B0 e 0x6B1 são `Count` e `Invalid` (0x6B1 é o "sem record" do `GetStatType(TweakDBID)`, `0x1023b3110`): o primeiro stat novo é 0x6B2,
//!   com as 2 posições de antes vazias em `statRecords`. O valor do enum mora no record em `+0xD8`, lido direto pelo `GetStatType`.
//!
//! Quando: os tweaks de mod entram no começo da sessão, depois do `InitializeRecords`. O registro roda no fim do `InitializeRecords` (stat que já
//! existia) e no fim de cada lote de tweaks (`registrar_tardio`, que recalcula os params pelo `ReinitializeParams` do motor).

use core::ffi::c_void;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const INICIALIZAR_REGISTROS_VM: u64 = 0x1_023b_2f68;
const INICIALIZAR_REGISTROS_PROLOGO: u64 = 0xa906_6ffc_d103_03ff;
const REINICIALIZAR_PARAMS_VM: u64 = 0x1_023b_2960;
const REINICIALIZAR_PARAMS_PROLOGO: u64 = 0xa901_7bfd_a9be_4ff4;
const CHECAR_FLAG_VM: u64 = 0x1_023b_3438;
const CHECAR_FLAG_PROLOGO: u64 = 0xa901_4ff4_a9bd_57f6;
const FLAGS_VM: u64 = 0x1_023b_34b4;
const FLAGS_PROLOGO: u64 = 0x5400_0288_711a_bc3f;
const FAIXA_VM: u64 = 0x1_023b_3cb0;
const FAIXA_PROLOGO: u64 = 0x6f00_e400_6f00_e401;

const REGISTROS: usize = 0xD8;
const PARAMS: usize = 0xE8;
const TRAVA: usize = 0xFC;
const VALOR_NO_RECORD: usize = 0xD8;
const VANILLA: u32 = 0x6B0;
const PRIMEIRO_NOVO: u32 = 0x6B2;

static SISTEMA: AtomicU64 = AtomicU64::new(0);
static ORIG_REGISTROS: AtomicU64 = AtomicU64::new(0);
static ORIG_CHECAR: AtomicU64 = AtomicU64::new(0);
static ORIG_FLAGS: AtomicU64 = AtomicU64::new(0);
static ORIG_FAIXA: AtomicU64 = AtomicU64::new(0);
static REGISTRANDO: Mutex<()> = Mutex::new(());

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Faixa {
    min: f32,
    max: f32,
}

/// `{min, max, flags}` de `stat` em `params`, sob a trava de leitura do motor; `None` além do fim.
unsafe fn param(sys: *mut u8, stat: u32) -> Option<(f32, f32, u32)> {
    let trava = sys.add(TRAVA);
    crate::jogo::travas::travar_leitura((trava) as *mut u8);
    let p = (sys.add(PARAMS) as *const u64).read_unaligned() as *const u8;
    let n = (sys.add(PARAMS + 0x0C) as *const u32).read_unaligned();
    let r = (stat < n && !p.is_null()).then(|| {
        let e = p.add(stat as usize * 12);
        ((e as *const f32).read_unaligned(), (e.add(4) as *const f32).read_unaligned(), (e.add(8) as *const u32).read_unaligned())
    });
    crate::jogo::travas::soltar_leitura((trava) as *mut u8);
    r
}

unsafe extern "C" fn checar_flag(sys: *mut u8, stat: u32, mascara: u32) -> bool {
    if stat < PRIMEIRO_NOVO {
        let f: extern "C" fn(*mut u8, u32, u32) -> bool = std::mem::transmute(ORIG_CHECAR.load(Ordering::Acquire));
        return f(sys, stat, mascara);
    }
    param(sys, stat).is_some_and(|(_, _, flags)| flags & mascara != 0)
}

unsafe extern "C" fn flags(sys: *mut u8, stat: u32) -> u32 {
    if stat < PRIMEIRO_NOVO {
        let f: extern "C" fn(*mut u8, u32) -> u32 = std::mem::transmute(ORIG_FLAGS.load(Ordering::Acquire));
        return f(sys, stat);
    }
    param(sys, stat).map_or(0, |(_, _, flags)| flags)
}

unsafe extern "C" fn faixa(sys: *mut u8, stat: u32) -> Faixa {
    if stat < PRIMEIRO_NOVO {
        let f: extern "C" fn(*mut u8, u32) -> Faixa = std::mem::transmute(ORIG_FAIXA.load(Ordering::Acquire));
        return f(sys, stat);
    }
    param(sys, stat).map_or(Faixa { min: 0.0, max: 0.0 }, |(min, max, _)| Faixa { min, max })
}

unsafe extern "C" fn inicializar_registros(sys: *mut u8) {
    let f: extern "C" fn(*mut u8) = std::mem::transmute(ORIG_REGISTROS.load(Ordering::Acquire));
    f(sys);
    SISTEMA.store(sys as u64, Ordering::Release);
    let n = registrar(sys);
    crate::log_dev!("[stats] InitializeRecords do motor em {sys:p}; {n} stat(s) de mod registrados");
}

/// Registra em `statRecords` os records `Stat` de mod com `enumName` fora dos stats do jogo. Devolve quantos entraram agora.
unsafe fn registrar(sys: *mut u8) -> usize {
    let _g = REGISTRANDO.lock().unwrap_or_else(|e| e.into_inner());
    // Sem record de mod (o boot comum, e sempre no `InitializeRecords` do começo), nada a registrar: nem TweakDB nem RTTI são consultados.
    let criados = super::registro::criados();
    if criados.is_empty() {
        return 0;
    }
    let Some(t) = super::singleton() else { return 0 };
    let Some(reg) = crate::rtti::Registry::obtain() else { return 0 };
    let en = reg.enum_by_name("gamedataStatType");
    let arr = sys.add(REGISTROS);
    let ids = (arr as *const u64).read_unaligned() as *const u64;
    let n = (arr.add(0x0C) as *const u32).read_unaligned();
    if en.is_null() || ids.is_null() || !(VANILLA..0x10_0000).contains(&n) || !crate::memoria::is_readable(ids as *const c_void, n as usize * 8) {
        crate::log_dev!("[stats] statRecords fora do esperado (tamanho {n}, enum {en:p}) — nada registrado");
        return 0;
    }
    let presentes: HashSet<u64> = (0..n as usize).map(|i| bwms_primitives::tweak_db_id_chave(ids.add(i).read_unaligned())).collect();
    // Os já registrados de antes: o record pode ter sido recriado (recarga de tweak) e perdido o valor.
    for v in PRIMEIRO_NOVO..n {
        marcar(super::get_record_instance(t, ids.add(v as usize).read_unaligned()), v);
    }
    // Só records criados por mod: um `Stat` do jogo fora do enum (stat aposentado) continua fora, como no jogo sem mods.
    let Some(tipo) = super::tipo_do_record(super::get_record_instance(t, ids.read_unaligned())) else { return 0 };
    let candidatos: Vec<(u64, *mut u8)> = criados
        .into_iter()
        .map(|id| (id, super::get_record_instance(t, id)))
        .filter(|(_, rec)| super::tipo_do_record(*rec) == Some(tipo))
        .collect();
    let mut nossos: BTreeMap<u32, (u64, *mut u8)> = BTreeMap::new();
    let mut novos: Vec<(String, u64, *mut u8)> = Vec::new();
    for (id, rec) in candidatos {
        if presentes.contains(&id) {
            continue;
        }
        let nome = super::get_flat_texto(t as *const u8, bwms_primitives::tweak_db_id_derive(id, ".enumName")).unwrap_or_default();
        if nome.is_empty() {
            continue;
        }
        match crate::rtti::enum_valor(en as *const u8, crate::cname::cname(&nome)) {
            // Do BWMS, de um registro anterior (outro sistema de stats, ou o record foi recriado).
            Some(v) if v >= PRIMEIRO_NOVO as u64 && v < 0x10_0000 => {
                nossos.insert(v as u32, (id, rec));
            }
            // Nome de stat do jogo: clone de um stat vanilla que não trocou o `enumName`.
            Some(_) => {}
            None => novos.push((nome, id, rec)),
        }
    }
    if nossos.is_empty() && novos.is_empty() {
        return 0;
    }
    let mut tamanho = n;
    let mut entraram = 0;
    let empurrar = |id: u64, tamanho: &mut u32| -> bool {
        let ok = crate::register::dynarray_push_ptr(arr as *mut c_void, id).is_some();
        if ok {
            *tamanho += 1;
        }
        ok
    };
    // `Count` e `Invalid` ficam vazios.
    while tamanho < PRIMEIRO_NOVO {
        if !empurrar(0, &mut tamanho) {
            return 0;
        }
    }
    for (v, (id, rec)) in nossos {
        while tamanho < v && empurrar(0, &mut tamanho) {}
        if tamanho == v && empurrar(id, &mut tamanho) {
            marcar(rec, v);
            entraram += 1;
        } else {
            crate::log_dev!("[stats] valor {v:#x} do record {id:#x} já ocupado — fica de fora");
        }
    }
    for (nome, id, rec) in novos {
        let v = tamanho;
        if crate::tipos_do_motor::enum_add_constant(en, &nome, v as i64) && empurrar(id, &mut tamanho) {
            marcar(rec, v);
            entraram += 1;
            crate::log_dev!("[stats] stat novo '{nome}' = {v:#x} (record {id:#x})");
        } else {
            crate::log_dev!("[stats] stat novo '{nome}' (record {id:#x}) não entrou no enum");
        }
    }
    entraram
}

/// Grava o valor do enum no record (`+0xD8`), o que o `GetStatType(TweakDBID)` do motor lê.
unsafe fn marcar(rec: *mut u8, v: u32) {
    if !rec.is_null() && crate::memoria::is_readable(rec.add(VALOR_NO_RECORD) as *const c_void, 4) {
        (rec.add(VALOR_NO_RECORD) as *mut u32).write_unaligned(v);
    }
}

/// Depois de um lote de tweaks: registra os stats novos e, se algum entrou, recalcula os params pelo `ReinitializeParams` do motor.
pub(crate) fn registrar_tardio() {
    let sys = SISTEMA.load(Ordering::Acquire) as *mut u8;
    if sys.is_null() {
        return;
    }
    unsafe {
        let n = registrar(sys);
        if n == 0 {
            return;
        }
        let alvo = crate::rebase(REINICIALIZAR_PARAMS_VM);
        if !prologo_confere(alvo, REINICIALIZAR_PARAMS_PROLOGO) {
            return crate::log("[stats] ReinitializeParams sem o prólogo esperado — params dos stats novos não calculados");
        }
        let f: extern "C" fn(*mut u8) = std::mem::transmute(alvo);
        f(sys);
        let total = (sys.add(PARAMS + 0x0C) as *const u32).read_unaligned();
        crate::log_dev!("[stats] {n} stat(s) de mod registrados depois dos tweaks; params recalculados pelo motor ({total})");
    }
}

unsafe fn prologo_confere(alvo: *mut c_void, prologo: u64) -> bool {
    crate::memoria::is_readable(alvo as *const c_void, 8) && (alvo as *const u64).read_unaligned() == prologo
}

/// Os 4 ganchos, no carregamento (antes do motor iniciar os sistemas). Sem o prólogo esperado em algum, nenhum entra.
pub(crate) unsafe fn instalar() {
    if ORIG_REGISTROS.load(Ordering::Relaxed) != 0 {
        return;
    }
    let pontos: [(u64, u64, *mut c_void, &AtomicU64); 4] = [
        (INICIALIZAR_REGISTROS_VM, INICIALIZAR_REGISTROS_PROLOGO, inicializar_registros as *mut c_void, &ORIG_REGISTROS),
        (CHECAR_FLAG_VM, CHECAR_FLAG_PROLOGO, checar_flag as *mut c_void, &ORIG_CHECAR),
        (FLAGS_VM, FLAGS_PROLOGO, flags as *mut c_void, &ORIG_FLAGS),
        (FAIXA_VM, FAIXA_PROLOGO, faixa as *mut c_void, &ORIG_FAIXA),
    ];
    if let Some((vm, _, _, _)) = pontos.iter().find(|(vm, p, _, _)| !prologo_confere(crate::rebase(*vm), *p)) {
        return crate::log_dev!("[stats] {vm:#x} sem o prólogo esperado — sem stats de mod");
    }
    // Os getters antes do InitializeRecords: stat registrado nunca é lido pelo teto antigo.
    let postos: Vec<bool> = pontos
        .iter()
        .rev()
        .map(|(vm, _, novo, orig)| crate::ganchos::instalar_com_threads_paradas(crate::rebase(*vm), *novo, Some(orig)).is_some())
        .collect();
    crate::log_dev!("[stats] ganchos (faixa, flags, checar flag, InitializeRecords): {postos:?}");
}

// ===== Teste (`~/.bwms-hook-stats-teste`) ================================================================================================
// Como um mod: um YAML com `$type: gamedataStat_Record`, `enumName` novo e faixa 0..1000, aplicado pelo mesmo caminho dos tweaks de mod. O lado
// do motor confere a constante no enum, o id em `statRecords` na posição do valor, o valor no record, a faixa pelo getter com gancho (e, de
// controle, (0, 0) pelo original com o teto) e um stat do jogo igual pelos dois. O lado do script (`bwms-stats-teste.reds`) põe +1234 no V pelo
// `StatsSystem` e tem de ler 1000 (a faixa do record), e o valor de antes depois de tirar o modificador.

#[cfg(feature = "devtools")]
const TESTE_YAML: &str = "BaseStats.BwmsTesteStat:\n  $type: gamedataStat_Record\n  enumName: BwmsTesteStat\n  min: 0\n  max: 1000\n";

#[cfg(feature = "devtools")]
pub(crate) fn teste_tick() {
    static FASE: AtomicU64 = AtomicU64::new(0);
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static DESDE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    static PASSOS: crate::selftest::PassosDeScript = crate::selftest::PassosDeScript::novo();
    if !crate::marcador_individual("stats-teste") {
        return;
    }
    PASSOS.tick("stats-teste", "BwmsStatsTestePasso", "stats", 45);
    let fase = FASE.load(Ordering::Relaxed);
    if fase >= 2 || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 60) || !crate::inicio::boot::gameplay_real() {
        return;
    }
    let desde = DESDE.get_or_init(std::time::Instant::now).elapsed().as_secs();
    if fase == 0 && desde >= 20 {
        FASE.store(1, Ordering::Relaxed);
        let caminho = "/tmp/bwms-stats-teste.yaml";
        if let Err(e) = std::fs::write(caminho, TESTE_YAML) {
            FASE.store(2, Ordering::Relaxed);
            return crate::log_veredito(&format!("[stats] VEREDITO-STATS-MOTOR não gravou o YAML do teste: {e}; stats-motor-ok=false"));
        }
        super::aplicar_arquivo(caminho);
    } else if fase == 1 && desde >= 30 {
        FASE.store(2, Ordering::Relaxed);
        unsafe { conferir() };
    }
}

#[cfg(feature = "devtools")]
unsafe fn conferir() {
    let sys = SISTEMA.load(Ordering::Acquire) as *mut u8;
    let (Some(t), Some(reg)) = (super::singleton(), crate::rtti::Registry::obtain()) else {
        return crate::log_veredito("[stats] VEREDITO-STATS-MOTOR sem TweakDB ou RTTI; stats-motor-ok=false");
    };
    if sys.is_null() {
        return crate::log_veredito("[stats] VEREDITO-STATS-MOTOR o InitializeRecords do motor não passou pelo gancho; stats-motor-ok=false");
    }
    let en = reg.enum_by_name("gamedataStatType") as *const u8;
    let valor = crate::rtti::enum_valor(en, crate::cname::cname("BwmsTesteStat")).map(|v| v as u32);
    let id = bwms_primitives::tweak_db_id("BaseStats.BwmsTesteStat");
    let arr = sys.add(REGISTROS);
    let (ids, n) = ((arr as *const u64).read_unaligned() as *const u64, (arr.add(0x0C) as *const u32).read_unaligned());
    let no_array = valor.is_some_and(|v| v < n && bwms_primitives::tweak_db_id_chave(ids.add(v as usize).read_unaligned()) == id);
    let rec = super::get_record_instance(t, id);
    let no_record = if rec.is_null() { None } else { Some((rec.add(VALOR_NO_RECORD) as *const u32).read_unaligned()) };
    let pelo_motor: extern "C" fn(*mut u8, u32) -> Faixa = std::mem::transmute(crate::rebase(FAIXA_VM));
    let original: extern "C" fn(*mut u8, u32) -> Faixa = std::mem::transmute(ORIG_FAIXA.load(Ordering::Acquire));
    let flags_pelo_motor: extern "C" fn(*mut u8, u32) -> u32 = std::mem::transmute(crate::rebase(FLAGS_VM));
    let v = valor.unwrap_or(0);
    let (faixa_nova, faixa_teto) = (pelo_motor(sys, v), original(sys, v));
    let (vanilla_gancho, vanilla_orig) = (pelo_motor(sys, 0), original(sys, 0));
    let ok = v >= PRIMEIRO_NOVO
        && no_array
        && no_record == Some(v)
        && faixa_nova == Faixa { min: 0.0, max: 1000.0 }
        && faixa_teto == Faixa { min: 0.0, max: 0.0 }
        && vanilla_gancho == vanilla_orig;
    crate::log_veredito(&format!(
        "[stats] VEREDITO-STATS-MOTOR stat do YAML: valor no enum {valor:?}, statRecords {n} com o record na posição {no_array}, valor no record {no_record:?}, \
         faixa pelo getter {faixa_nova:?} (original com teto {faixa_teto:?}), flags {:#x}, stat 0 igual pelos dois {} ({vanilla_gancho:?}); stats-motor-ok={ok}",
        flags_pelo_motor(sys, v),
        vanilla_gancho == vanilla_orig
    ));
}
