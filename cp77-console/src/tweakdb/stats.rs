//! Stats novos de mod: um record `Stat` com `enumName` que o enum `gamedataStatType` não tem vira um stat de verdade — o bônus
//! próprio de um cyberware ou perk de mod.

use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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

/// `statRecords`: `DynArray<TweakDBID>` (tamanho em `+0xE4`).
const REGISTROS: usize = 0xD8;
/// `params`: 12 B por stat `{min f32, max f32, flags u32}` (tamanho em `+0xF4`).
const PARAMS: usize = 0xE8;
/// Trava de `params` (byte).
const TRAVA: usize = 0xFC;
/// Valor do enum no `Stat_Record`, lido pelo `GetStatType(TweakDBID)`.
const VALOR_NO_RECORD: usize = 0xD8;
/// Stats do jogo: 0..0x6B0; 0x6B0 = `Count`, 0x6B1 = `Invalid`.
const VANILLA: u32 = 0x6B0;
const PRIMEIRO_NOVO: u32 = 0x6B2;

/// O `StatsDataSystem` que passou pela carga dos records de stat (`INICIALIZAR_REGISTROS_VM`; 0 antes disso). O teste ao vivo lê daqui.
static SISTEMA: AtomicU64 = AtomicU64::new(0);
/// A faixa do stat original (`FAIXA_VM`, a do teto fixo), chamável depois do gancho. O teste ao vivo lê daqui.
static ORIG_FAIXA: AtomicU64 = AtomicU64::new(0);

/// O retorno da faixa do stat (`s0`, `s1`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Faixa {
    min: f32,
    max: f32,
}

static ORIG_CARGA: AtomicU64 = AtomicU64::new(0);
static ORIG_FLAG: AtomicU64 = AtomicU64::new(0);
static ORIG_FLAGS: AtomicU64 = AtomicU64::new(0);
static INSTALADO: AtomicBool = AtomicBool::new(false);
/// Uma passada de registro por vez: a carga dos records de stat e o fim de um lote de tweaks podem coincidir.
static REGISTRANDO: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// `statRecords` maior que isto não é o array medido.
const REGISTROS_MAX: u32 = 0x10_0000;

/// Depois de um lote de tweaks: os stats de mod novos entram no sistema de stats.
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
        let Some(recalcular) = prologo_confere(REINICIALIZAR_PARAMS_VM, REINICIALIZAR_PARAMS_PROLOGO) else {
            return crate::log_dev!("[stats] recálculo dos params sem o prólogo medido: os stats novos ficam sem faixa até a próxima carga");
        };
        // O caminho do próprio motor: trava `params`, zera, recalcula pelo tamanho de `statRecords` e solta.
        let f: extern "C" fn(*mut u8) = std::mem::transmute(recalcular);
        f(sys);
        let total = (sys.add(PARAMS + 0x0C) as *const u32).read_unaligned();
        crate::log_dev!("[stats] {n} stat(s) de mod registrados depois dos tweaks; params recalculados pelo motor ({total})");
    }
}

/// Os ganchos do sistema de stats, no carregamento (antes do motor iniciar os sistemas).
pub(crate) unsafe fn instalar() {
    if INSTALADO.swap(true, Ordering::AcqRel) {
        return;
    }
    let medidos = [
        (CHECAR_FLAG_VM, CHECAR_FLAG_PROLOGO),
        (FLAGS_VM, FLAGS_PROLOGO),
        (FAIXA_VM, FAIXA_PROLOGO),
        (INICIALIZAR_REGISTROS_VM, INICIALIZAR_REGISTROS_PROLOGO),
    ];
    let alvos = medidos.map(|(vm, prologo)| prologo_confere(vm, prologo));
    if alvos.iter().any(Option::is_none) {
        let conferem = alvos.map(|a| a.is_some());
        return crate::log_dev!("[stats] prólogos (flag, flags, faixa, carga) conferem {conferem:?}: sem ganchos, sem stats de mod");
    }
    let novos: [*mut c_void; 4] = [checar_flag as *mut c_void, flags as *mut c_void, faixa as *mut c_void, carga as *mut c_void];
    let originais = [&ORIG_FLAG, &ORIG_FLAGS, &ORIG_FAIXA, &ORIG_CARGA];
    let mut postos = [false; 4];
    // As consultas antes da carga: stat registrado nunca passa pelo teto fixo do original.
    for i in 0..4 {
        if i == 3 && !postos[..3].iter().all(|p| *p) {
            break;
        }
        if let Some(alvo) = alvos[i] {
            postos[i] = crate::ganchos::instalar_com_threads_paradas(alvo, novos[i], Some(originais[i])).is_some();
        }
    }
    crate::log_dev!("[stats] ganchos (flag, flags, faixa, carga): {postos:?}");
}

/// O endereço da função `vm`, só se o prólogo é o medido na 2.5.
unsafe fn prologo_confere(vm: u64, prologo: u64) -> Option<*mut c_void> {
    let alvo = crate::rebase(vm);
    (!alvo.is_null() && crate::memoria::is_readable(alvo, 8) && (alvo as *const u64).read_unaligned() == prologo).then_some(alvo)
}

/// `params[stat]` lido sob a trava de leitura de `params`, como o motor lê; `None` além do tamanho de `params`.
unsafe fn param(sys: *mut u8, stat: u32) -> Option<(Faixa, u32)> {
    let trava = sys.add(TRAVA);
    crate::jogo::travas::travar_leitura(trava);
    let lista = (sys.add(PARAMS) as *const u64).read_unaligned() as *const u8;
    let n = (sys.add(PARAMS + 0x0C) as *const u32).read_unaligned();
    let lido = (stat < n && !lista.is_null()).then(|| {
        let e = lista.add(stat as usize * 12);
        let faixa = Faixa { min: (e as *const f32).read_unaligned(), max: (e.add(4) as *const f32).read_unaligned() };
        (faixa, (e.add(8) as *const u32).read_unaligned())
    });
    crate::jogo::travas::soltar_leitura(trava);
    lido
}

// As três consultas do motor têm o teto fixo `stat <= 0x6af`; o stat de mod (`>= 0x6B2`) é lido de `params` pelo tamanho real.
unsafe extern "C" fn checar_flag(sys: *mut u8, stat: u32, mascara: u32) -> bool {
    if stat < PRIMEIRO_NOVO {
        let f: extern "C" fn(*mut u8, u32, u32) -> bool = std::mem::transmute(crate::ganchos::original_publicado(&ORIG_FLAG));
        return f(sys, stat, mascara);
    }
    param(sys, stat).is_some_and(|(_, flags)| flags & mascara != 0)
}

unsafe extern "C" fn flags(sys: *mut u8, stat: u32) -> u32 {
    if stat < PRIMEIRO_NOVO {
        let f: extern "C" fn(*mut u8, u32) -> u32 = std::mem::transmute(crate::ganchos::original_publicado(&ORIG_FLAGS));
        return f(sys, stat);
    }
    param(sys, stat).map_or(0, |(_, flags)| flags)
}

unsafe extern "C" fn faixa(sys: *mut u8, stat: u32) -> Faixa {
    if stat < PRIMEIRO_NOVO {
        let f: extern "C" fn(*mut u8, u32) -> Faixa = std::mem::transmute(crate::ganchos::original_publicado(&ORIG_FAIXA));
        return f(sys, stat);
    }
    param(sys, stat).map_or(Faixa { min: 0.0, max: 0.0 }, |(faixa, _)| faixa)
}

unsafe extern "C" fn carga(a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64, a7: u64) -> u64 {
    let f: extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64) -> u64 = std::mem::transmute(crate::ganchos::original_publicado(&ORIG_CARGA));
    let r = f(a0, a1, a2, a3, a4, a5, a6, a7);
    SISTEMA.store(a0, Ordering::Release);
    // O motor calcula os params logo depois desta carga, pelo tamanho de `statRecords`: o que entra aqui já sai com faixa.
    registrar(a0 as *mut u8);
    r
}

/// Cópia de `statRecords` (`DynArray<TweakDBID>` em `arr`), só com o tamanho na faixa medida.
unsafe fn ids_registrados(arr: *const u8) -> Option<Vec<u64>> {
    let ids = (arr as *const u64).read_unaligned() as *const u64;
    let n = (arr.add(0x0C) as *const u32).read_unaligned();
    if ids.is_null() || !(VANILLA..REGISTROS_MAX).contains(&n) || !crate::memoria::is_readable(ids as *const c_void, n as usize * 8) {
        return None;
    }
    Some((0..n as usize).map(|i| ids.add(i).read_unaligned()).collect())
}

/// Põe `chave` em `statRecords[v]`, com zeros no que faltar até lá (os `Count`/`Invalid` incluídos). `false` se `v` já é de outro id.
unsafe fn colocar(sys: *mut u8, v: u32, chave: u64) -> bool {
    let arr = sys.add(REGISTROS);
    let trava = sys.add(TRAVA);
    crate::jogo::travas::travar_escrita(trava);
    let mut n = (arr.add(0x0C) as *const u32).read_unaligned();
    while n <= v {
        if crate::register::dynarray_push_ptr(arr as *mut c_void, 0).is_none() {
            break;
        }
        n = (arr.add(0x0C) as *const u32).read_unaligned();
    }
    let posto = n > v && {
        let vaga = ((arr as *const u64).read_unaligned() as *mut u64).add(v as usize);
        let atual = bwms_primitives::tweak_db_id_chave(vaga.read_unaligned());
        (atual == 0 || atual == chave) && {
            vaga.write_unaligned(chave);
            true
        }
    };
    crate::jogo::travas::soltar_escrita(trava);
    posto
}

/// Os records `Stat` criados por mod nesta sessão viram stats do `gamedataStatType`. Devolve quantos entraram agora.
unsafe fn registrar(sys: *mut u8) -> usize {
    let _vez = REGISTRANDO.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let criados = super::registro::criados();
    if criados.is_empty() || sys.is_null() {
        return 0;
    }
    let (Some(t), Some(reg)) = (super::singleton(), crate::rtti::Registry::obtain()) else { return 0 };
    let Some(ids) = ids_registrados(sys.add(REGISTROS)) else {
        crate::log_dev!("[stats] statRecords fora da faixa medida ({VANILLA:#x}..{REGISTROS_MAX:#x}) ou ilegível: nada registrado");
        return 0;
    };
    let chave = bwms_primitives::tweak_db_id_chave;
    let Some(tipo_stat) = super::tipo_do_record(super::get_record_instance(t, chave(ids[0]))) else {
        crate::log_dev!("[stats] statRecords[0] sem record vivo: nada registrado");
        return 0;
    };
    let en = reg.enum_by_name("gamedataStatType");
    if en.is_null() {
        crate::log_dev!("[stats] enum gamedataStatType não achado: nada registrado");
        return 0;
    }
    // Já registrados: o record pode ter sido recriado (recarga de tweak) e perdido o valor no `+0xD8`.
    for (v, id) in ids.iter().enumerate().skip(PRIMEIRO_NOVO as usize) {
        let rec = super::get_record_instance(t, chave(*id));
        if *id != 0 && !rec.is_null() {
            (rec.add(VALOR_NO_RECORD) as *mut u32).write_unaligned(v as u32);
        }
    }
    let presentes: std::collections::HashSet<u64> = ids.iter().map(|id| chave(*id)).filter(|c| *c != 0).collect();
    let mut entraram = 0usize;
    for id in criados.into_iter().map(chave).filter(|c| !presentes.contains(c)) {
        let rec = super::get_record_instance(t, id);
        if rec.is_null() || super::tipo_do_record(rec) != Some(tipo_stat) {
            continue;
        }
        let Some(nome) = super::get_flat_texto(t, bwms_primitives::tweak_db_id_derive(id, ".enumName")).filter(|n| !n.is_empty()) else { continue };
        let no_enum = crate::rtti::enum_valor(en as *const u8, crate::cname::cname(&nome));
        let v = match no_enum {
            Some(v) if v >= PRIMEIRO_NOVO as u64 && v < REGISTROS_MAX as u64 => v as u32,
            Some(v) => {
                crate::log_dev!("[stats] '{nome}' (record {id:#x}) já é o stat {v:#x} do jogo: ignorado");
                continue;
            }
            None => {
                let n = (sys.add(REGISTROS + 0x0C) as *const u32).read_unaligned();
                n.max(PRIMEIRO_NOVO)
            }
        };
        if !colocar(sys, v, id) {
            crate::log_dev!("[stats] '{nome}' (record {id:#x}): statRecords[{v:#x}] já é de outro record, fica de fora");
            continue;
        }
        if no_enum.is_none() && !crate::tipos_do_motor::enum_add_constant(en, &nome, v as i64) {
            crate::log_dev!("[stats] '{nome}' = {v:#x}: a constante não entrou no gamedataStatType");
        }
        (rec.add(VALOR_NO_RECORD) as *mut u32).write_unaligned(v);
        crate::log_dev!("[stats] stat novo '{nome}' = {v:#x} (record {id:#x})");
        entraram += 1;
    }
    entraram
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
