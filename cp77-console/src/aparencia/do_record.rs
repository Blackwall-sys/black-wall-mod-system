//! Aparência do record no spawn (ArchiveXL `#63`/`#101`, o `EntitySpawnerPatch`). Os três caminhos da interface do `EntitySpawner` (slots
//! `+0x10`, `+0x18` e `+0x20` da vtable `0x1078c9dd0`) montam um pedido interno e convergem em `0x1019f8d60` (caminho do template `x0`,
//! pedido `x1`, saída por sret `x8`), o spawn por template. Nesse pedido o record vai em `+0xE0` e a aparência em `+0xC0`; no caminho com
//! record (slot `+0x18`) a aparência nasce vazia, e um record de mod com `appearanceName` saía com a aparência padrão do template. Com record e
//! aparência vazia ou "default" (`pedida_livre`), entra a do flat `<record>.appearanceName` quando ele tem nome (`nome_lido_do_record`), a
//! regra do `EntitySpawnerPatch.cpp:17-23` da referência.

use core::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

const SPAWN_POR_TEMPLATE_VM: u64 = 0x1_019f_8d60;
/// `sub sp, sp, #0x30` + `stp x20, x19, [sp, #0x10]`.
const SPAWN_POR_TEMPLATE_PROLOGO: u64 = 0xa901_4ff4_d100_c3ff;
const PEDIDO_APARENCIA: usize = 0xC0;
const PEDIDO_RECORD: usize = 0xE0;

static ORIG: AtomicU64 = AtomicU64::new(0);
/// Pedidos vistos, com record, e com a aparência trocada pela do record.
#[cfg(feature = "devtools")]
pub(crate) static VISTOS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
pub(crate) static COM_RECORD: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
pub(crate) static TROCADOS: AtomicU64 = AtomicU64::new(0);
/// Pedidos com record e aparência vazia que chegaram sem o TweakDB carregado, e o segundo do processo do primeiro e do último com record.
#[cfg(feature = "devtools")]
static SEM_TWEAKDB: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
static PRIMEIRO_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
static ULTIMO_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
fn ms_do_processo() -> u64 {
    static INICIO: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    INICIO.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}
/// Últimas trocas: (record, aparência do record).
#[cfg(feature = "devtools")]
static TROCAS: std::sync::Mutex<Vec<(u64, u64)>> = std::sync::Mutex::new(Vec::new());
/// Teste do gancho (`aparencia-teste`): os records da lista do boot anterior e o que recebeu o flat dentro do gancho.
#[cfg(feature = "devtools")]
static TESTE_LISTA: std::sync::OnceLock<Vec<u64>> = std::sync::OnceLock::new();
#[cfg(feature = "devtools")]
static FIXTURE: AtomicU64 = AtomicU64::new(0);
/// Records que chegaram com a aparência vazia (os que o patch atende), até 64 distintos.
#[cfg(feature = "devtools")]
static RECORDS_VAZIOS: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

fn cname_padrao() -> u64 {
    static C: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *C.get_or_init(|| crate::cname::cname("default"))
}

/// Aparência pedida que deixa o record mandar: vazia ou "default". Quem passa "default" de propósito deixa o padrão, não escolhe; tratá-lo
/// como escolha faria o gancho não disparar no caso mais comum, e a aparência do record entraria só às vezes.
fn pedida_livre(pedida: u64) -> bool {
    pedida == 0 || pedida == cname_padrao()
}

/// O `appearanceName` lido do flat do record, só quando tem nome: record sem nome não troca um vazio por outro.
fn nome_lido_do_record(lido: Option<u64>) -> Option<u64> {
    lido.filter(|a| *a != 0)
}

unsafe extern "C" fn decidir(pedido: *mut u8) {
    #[cfg(feature = "devtools")]
    VISTOS.fetch_add(1, Ordering::Relaxed);
    if pedido.is_null() || !crate::memoria::is_readable(pedido.add(PEDIDO_APARENCIA) as *const c_void, 0x28) {
        return;
    }
    let record = bwms_primitives::tweak_db_id_chave((pedido.add(PEDIDO_RECORD) as *const u64).read_unaligned());
    if record == 0 {
        return;
    }
    // A contagem, o tempo e as listas abaixo só alimentam o teste do gancho (`teste_tick`).
    #[cfg(feature = "devtools")]
    {
        COM_RECORD.fetch_add(1, Ordering::Relaxed);
        let agora = ms_do_processo();
        let _ = PRIMEIRO_MS.compare_exchange(0, agora.max(1), Ordering::Relaxed, Ordering::Relaxed);
        ULTIMO_MS.store(agora, Ordering::Relaxed);
    }
    let pedida = (pedido.add(PEDIDO_APARENCIA) as *const u64).read_unaligned();
    if !pedida_livre(pedida) {
        return;
    }
    #[cfg(feature = "devtools")]
    if let Ok(mut v) = RECORDS_VAZIOS.try_lock() {
        if v.len() < 64 && !v.contains(&record) {
            v.push(record);
        }
    }
    let Some(t) = crate::tweakdb::singleton() else {
        #[cfg(feature = "devtools")]
        SEM_TWEAKDB.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let flat = bwms_primitives::tweak_db_id_derive(record, ".appearanceName");
    // Teste: o primeiro record da lista que chega vazio ganha o flat "default" aqui (o mod que o põe no setor com `appearanceName`).
    #[cfg(feature = "devtools")]
    if FIXTURE.load(Ordering::Relaxed) == 0
        && TESTE_LISTA.get().is_some_and(|l| l.contains(&record))
        && crate::tweakdb::get_flat_cname(t, flat).is_none()
        && FIXTURE.compare_exchange(0, record, Ordering::Relaxed, Ordering::Relaxed).is_ok()
    {
        let doador = bwms_primitives::tweak_db_id_derive(bwms_primitives::tweak_db_id(TESTE_TIPO_DE), ".appearanceName");
        crate::tweakdb::add_extra_flat_bytes_by_id(t, flat, doador, &cname_padrao().to_le_bytes());
    }
    if let Some(a) = nome_lido_do_record(crate::tweakdb::get_flat_cname(t, flat)) {
        (pedido.add(PEDIDO_APARENCIA) as *mut u64).write_unaligned(a);
        #[cfg(feature = "devtools")]
        {
            TROCADOS.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut v) = TROCAS.lock() {
                if v.len() >= 32 {
                    v.remove(0);
                }
                v.push((record, a));
            }
        }
    }
}

#[unsafe(naked)]
unsafe extern "C" fn shim() {
    core::arch::naked_asm!(
        "stp  x0, x1, [sp, #-0x60]!",
        "stp  x2, x3, [sp, #0x10]",
        "stp  x4, x5, [sp, #0x20]",
        "stp  x6, x7, [sp, #0x30]",
        "stp  x8, x30, [sp, #0x40]",   // x8 = sret da saída
        "mov  x0, x1",                 // o pedido interno
        "bl   {decidir}",
        "ldp  x8, x30, [sp, #0x40]",
        "ldp  x6, x7, [sp, #0x30]",
        "ldp  x4, x5, [sp, #0x20]",
        "ldp  x2, x3, [sp, #0x10]",
        "ldp  x0, x1, [sp], #0x60",
        "adrp x9, {o}@PAGE",
        "add  x9, x9, {o}@PAGEOFF",
        "ldr  x9, [x9]",
        "br   x9",
        o = sym ORIG,
        decidir = sym decidir,
    )
}

/// Gancho sempre ligado (como o patch da referência), posto no `on_load`.
pub(crate) unsafe fn instalar() {
    if ORIG.load(Ordering::Relaxed) != 0 {
        return;
    }
    #[cfg(feature = "devtools")]
    ms_do_processo();
    #[cfg(feature = "devtools")]
    if crate::marcador_individual("aparencia-teste") {
        let lidos: Vec<u64> = std::fs::read_to_string(arquivo_de_records())
            .unwrap_or_default()
            .lines()
            .filter_map(|l| u64::from_str_radix(l.trim().trim_start_matches("0x"), 16).ok())
            .collect();
        let _ = TESTE_LISTA.set(lidos);
    }
    let alvo = crate::rebase(SPAWN_POR_TEMPLATE_VM);
    if !crate::memoria::is_readable(alvo as *const c_void, 8) || (alvo as *const u64).read_unaligned() != SPAWN_POR_TEMPLATE_PROLOGO {
        return crate::log_dev!("[aparencia] spawn por template sem o prólogo esperado -> sem gancho");
    }
    let ok = crate::ganchos::instalar_com_threads_paradas(alvo, shim as *mut c_void, Some(&ORIG)).is_some();
    crate::log_dev!("[aparencia] gancho no spawn por template {ok}");
}

// ===== Teste (`~/.bwms-hook-aparencia-teste`) ============================================================================================
// Como um mod: um record de personagem clonado com `appearanceName` apontando pra uma aparência que o template dele tem; a entidade criada
// pelo `DynamicEntitySystem` com o clone, sem aparência no spec, tem de nascer com ela. O record base é o primeiro personagem do TweakDB com 2
// ou mais `crowdAppearanceNames` (aparências válidas do próprio template); o alvo é a última da lista. Controle: uma entidade do record base,
// que não tem `appearanceName` e fica com a que o motor escolher.

#[cfg(feature = "devtools")]
const TESTE_TIPO_DE: &str = "Character.afterlife_merc_generic_medium_range_friendly";
#[cfg(feature = "devtools")]
const TESTE_CLONE: &str = "Character.bwms_aparencia_teste";

#[cfg(feature = "devtools")]
struct Teste {
    fase: u32,
    base: u64,
    alvo: u64,
    lista: usize,
    clone_ent: u64,
    base_ent: u64,
    desde: Option<std::time::Instant>,
}
#[cfg(feature = "devtools")]
static TESTE: std::sync::Mutex<Teste> = std::sync::Mutex::new(Teste {
    fase: 0,
    base: 0,
    alvo: 0,
    lista: 0,
    clone_ent: 0,
    base_ent: 0,
    desde: None,
});

#[cfg(feature = "devtools")]
fn veredito(t: &mut Teste, texto: &str, ok: bool) {
    t.fase = 99;
    let (v, r, tr) = (VISTOS.load(Ordering::Relaxed), COM_RECORD.load(Ordering::Relaxed), TROCADOS.load(Ordering::Relaxed));
    crate::veredito_dev!(
        "[aparencia] VEREDITO-APARENCIA {texto}; gancho: {v} pedidos, {r} com record (do ms {} ao {} do processo), {} vazios sem TweakDB carregado, {tr} trocados; aparencia-ok={ok}",
        PRIMEIRO_MS.load(Ordering::Relaxed),
        ULTIMO_MS.load(Ordering::Relaxed),
        SEM_TWEAKDB.load(Ordering::Relaxed)
    );
}

#[cfg(feature = "devtools")]
unsafe fn aparencia_de(id: u64) -> Option<u64> {
    let mut h = crate::npcs::dynent_entidade(id);
    if h[0] == 0 {
        return None;
    }
    let ent = h[0] as *mut c_void;
    let a = crate::rtti::resolve_in_class(crate::rtti::class_of(ent), "GetCurrentAppearanceName")
        .and_then(|rf| crate::rtti::call_func(&rf, ent, &[]))
        .map(|b| u64::from_le_bytes(b[..8].try_into().unwrap()))
        .unwrap_or(0);
    crate::jogo::objetos::soltar_handle(&mut h);
    Some(a)
}

/// Arquivo com os records que chegaram vazios num boot anterior (os do streaming do save do teste).
#[cfg(feature = "devtools")]
fn arquivo_de_records() -> std::path::PathBuf {
    std::path::Path::new(&std::env::var("HOME").unwrap_or_default()).join(".bwms-aparencia-records.txt")
}

#[cfg(feature = "devtools")]
pub(crate) fn teste_tick() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    if !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("aparencia-teste") {
        return;
    }
    if !crate::selftest::trava_de_save_confirmada() || !crate::inicio::boot::gameplay_real() {
        return;
    }
    let Ok(mut t) = TESTE.lock() else { return };
    let desde = *t.desde.get_or_insert_with(std::time::Instant::now);
    if desde.elapsed().as_secs() < 20 {
        return;
    }
    unsafe {
        match t.fase {
            0 => {
                let Some(tdb) = crate::tweakdb::singleton() else { return };
                let modelo_id = bwms_primitives::tweak_db_id(TESTE_TIPO_DE);
                let modelo = crate::tweakdb::get_record_instance(tdb, modelo_id);
                let id_no_record = if modelo.is_null() { 0 } else { bwms_primitives::tweak_db_id_chave(((modelo as u64 + 0x40) as *const u64).read_unaligned()) };
                if id_no_record != modelo_id {
                    return veredito(&mut t, &format!("o id no record não está em +0x40 ({id_no_record:#x})"), false);
                }
                let ids: Vec<u64> = crate::tweakdb::records_do_tipo(tdb, modelo).into_iter().map(|(id, _)| id).collect();
                let achado = ids.iter().find_map(|id| {
                    let lista = crate::tweakdb::get_flat_array_u64_by_id(tdb, bwms_primitives::tweak_db_id_derive(*id, ".crowdAppearanceNames"))?;
                    let modelo = crate::tweakdb::get_flat_value_by_id(tdb, bwms_primitives::tweak_db_id_derive(*id, ".entityTemplatePath"))
                        .map(|p| (p as *const u64).read_unaligned())
                        .unwrap_or(0);
                    (lista.len() >= 2 && modelo != 0 && lista.iter().all(|a| *a != 0)).then_some((*id, lista))
                });
                let Some((base, lista)) = achado else {
                    return veredito(&mut t, &format!("nenhum dos {} personagens tem 2 ou mais crowdAppearanceNames", ids.len()), false);
                };
                t.base = base;
                t.lista = lista.len();
                t.alvo = *lista.last().unwrap();
                let clone = bwms_primitives::tweak_db_id(TESTE_CLONE);
                let Some(reg) = crate::rtti::Registry::obtain() else { return };
                let clonou = crate::tweakdb::clone_record_by_id(&reg, clone, base);
                let flat = bwms_primitives::tweak_db_id_derive(clone, ".appearanceName");
                let gravou = clonou && crate::tweakdb::escritor::gravar_escalar(flat, t.alvo);
                let lida = crate::tweakdb::get_flat_cname(tdb, flat);
                if !gravou || lida != Some(t.alvo) {
                    return veredito(&mut t, &format!("clone {clonou}, appearanceName gravado {gravou}, lido {lida:?}"), false);
                }
                let player = crate::current_player();
                let pos = crate::rtti::resolve_in_class(crate::rtti::class_of(player), "GetWorldPosition")
                    .and_then(|rf| crate::rtti::call_func(&rf, player, &[]))
                    .map(|b| [0, 1, 2, 3].map(|k| f32::from_le_bytes([b[k * 4], b[k * 4 + 1], b[k * 4 + 2], b[k * 4 + 3]])))
                    .unwrap_or([0.0; 4]);
                t.clone_ent = crate::npcs::dynent_criar_por_record(clone, [pos[0] + 3.0, pos[1] + 3.0, pos[2], 1.0]);
                t.base_ent = crate::npcs::dynent_criar_por_record(base, [pos[0] - 3.0, pos[1] + 3.0, pos[2], 1.0]);
                if t.clone_ent == 0 {
                    return veredito(&mut t, "o DynamicEntitySystem não criou a entidade do clone", false);
                }
                t.desde = Some(std::time::Instant::now() - std::time::Duration::from_secs(20));
                t.fase = 1;
            }
            1 => {
                let (a, b) = (aparencia_de(t.clone_ent), aparencia_de(t.base_ent));
                if (a.is_none() || b.is_none()) && desde.elapsed().as_secs() < 80 {
                    return;
                }
                let (c1, c2) = (crate::npcs::dynent_apagar_pub(t.clone_ent), crate::npcs::dynent_apagar_pub(t.base_ent));
                let a = a.unwrap_or(0);
                let texto = format!(
                    "record base {:#x} com {} aparências de multidão, alvo a última; entidade do clone (spec sem aparência) nasceu com {}, a do base com {} (controle), apagadas {c1}/{c2}",
                    t.base,
                    t.lista,
                    crate::cname::resolve_cname(a),
                    b.map(crate::cname::resolve_cname).unwrap_or_else(|| "nada".into())
                );
                // Parte do gancho: o record da lista que ganhou o flat dentro do gancho saiu dele com a aparência do flat.
                let fixture = FIXTURE.load(Ordering::Relaxed);
                let candidatos: Vec<u64> = if fixture != 0 { vec![fixture] } else { Vec::new() };
                let trocado = TROCAS.lock().ok().and_then(|v| v.iter().find(|(rec, a)| candidatos.contains(rec) && *a == cname_padrao()).map(|x| x.0));
                let vistos = RECORDS_VAZIOS.lock().map(|v| v.clone()).unwrap_or_default();
                let restantes = crate::tweakdb::singleton().map_or(0, |tdb| {
                    candidatos
                        .iter()
                        .filter(|id| crate::tweakdb::get_flat_cname(tdb, bwms_primitives::tweak_db_id_derive(**id, ".appearanceName")) == Some(cname_padrao()))
                        .count()
                });
                let _ = std::fs::write(arquivo_de_records(), vistos.iter().map(|r| format!("{r:#x}\n")).collect::<String>());
                let texto = format!(
                    "{texto}; gancho: record do streaming {fixture:#x} (lista do boot anterior, {} com o flat) ganhou o flat appearanceName default ao chegar vazio, pedido trocado {trocado:x?}, flat no TweakDB no fim {restantes}; {} records chegaram vazios nesta sessão",
                    candidatos.len(),
                    vistos.len()
                );
                // Critério: o record de mod com `appearanceName` nasce com ela (ponta a ponta) e o gancho está vivo no spawn por template, vendo os
                // pedidos com record no layout conferido. A troca no streaming (`trocado`) é informativa: os records desse caminho não têm o flat.
                let ok = a == t.alvo && VISTOS.load(Ordering::Relaxed) > 0 && COM_RECORD.load(Ordering::Relaxed) > 0;
                veredito(&mut t, &texto, ok);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    /// `decidir` sai sem record, sai quando a pedida não é livre e só troca pelo `nome_lido_do_record`; a saída sem record é código do
    /// gancho e fica fora deste teste.
    #[test]
    fn regra_do_patch() {
        let padrao = cname_padrao();
        assert!(pedida_livre(0), "pedida vazia deixa o record mandar");
        assert!(pedida_livre(padrao), "\"default\" explícito conta como não escolhido");
        assert!(!pedida_livre(9), "aparência escolhida é respeitada");
        assert_eq!(nome_lido_do_record(Some(7)), Some(7));
        assert_eq!(nome_lido_do_record(Some(0)), None, "record sem nome não sobrescreve");
        assert_eq!(nome_lido_do_record(None), None, "record sem o flat não sobrescreve");
        // "default" pedido com record sem nome: passa pela pedida e para no nome lido.
        assert!(pedida_livre(padrao) && nome_lido_do_record(Some(0)).is_none());
    }
}
