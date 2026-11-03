//! Mundo: clima, setores de streaming, nós da cena, traçado de raio e estado do mundo. Missões, journal e mapa estão em `missoes_e_mapa`.

#![allow(dead_code)]

use crate::jogo::sessao::*;
use crate::jogo::sessao::sistema_da_sessao;
use crate::jogo::recursos::PostLoadFn;
use crate::jogo::recursos::hash_do_caminho_do_recurso;
use crate::jogo::listas::dynarray_no_campo;
use bwms_primitives::{resource_path_hash, LOCKEY_PREFIXO};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicPtr;
use crate::missoes_e_mapa::*;
use crate::corpo_e_rosto::*;
use crate::npcs::*;
use crate::inicio::natives::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use crate::cname::{cname, CName};
use crate::rtti::{self, Registry};
use crate::register::*;
/// Bloco deste arquivo no `PROVA_NO_PUBLICO` (register.rs).
#[cfg(not(feature = "devtools"))]
const BLOCO_DE_PROVA: u8 = 0b100;
use crate::ganchos::*;
use crate::diagnostico::{
    bwms_hook_enabled_global, buffer_com_preset, indice_do_preset, listas_de_colisao, node_instance_da_memoria, planejar_delecao, PlanoDelecao, ATOR_TAM,
    ATOR_Z_DELETADO, ATOR_Z_OFF, COLISAO_ATORES_OFF, COLISAO_DADOS_OFF, COLISAO_NUM_ATORES_OFF, DESTRUTIVEL_TAM, PRESET_SEM_COLISAO, SHAPE_PRESET_OFF,
    DESTRUTIVEL_Z_DELETADO, DESTRUTIVEL_Z_OFF, INSTANCIA_SCALE_OFF, INSTANCIA_TAM, POSTLOAD_CNT_WSSECTOR_C2, SETOR_ALWAYS_LOADED_0_HASH, WSSECTOR_ALVO_LOGS,
};
// A releitura das deleções gravadas, que só o veredito de dev usa.
#[cfg(feature = "devlog")]
use crate::diagnostico::{ator_z, destrutivel_z, instancia_scale};
// Só os handlers de prova e os ramos de teste usam, e o público não cita nenhum dos dois.
#[cfg(feature = "devtools")]
use crate::diagnostico::{
    a51h_valor_igual, c237_contar, c237_resumo, setor_censo_spawners, setor_censo_static_mesh, setor_delecao_teste, setor_teste_anuncio, setor_teste_campo,
    setor_teste_colisao, setor_teste_destrutiveis, setor_teste_generica, setor_teste_instancias, setor_teste_colisao_cozida, setor_teste_mutacao, setor_teste_no_cozido, setor_teste_subno,
    tramp_c157_contexto,
    tramp_c157_executar, tramp_c48_medir, tramp_c81_anotar, tramp_c84_fase, tramp_c84_mundo, tramp_journal_mod_merge_undo, tramp_node_area_volume_check,
    tramp_rebase, tramp_veredito, tramp_vtable_slot, A51hInsercao, C237Conta, A51H_AMOSTRAS, A51H_ATRIBUIDAS, A51H_CRESCIMENTOS, A51H_DEVICES, A51H_INSERIDAS,
    A51H_NOVAS_PENDENTES, A51H_PATCH_NAO_PRONTO, A51H_TESTE, A51H_TESTE_FEITO, A51H_V2_AMOSTRAS, WSSECTOR_DELETE_TESTE, WSSECTOR_INST_TESTE, WS_ULTIMO,
};
use crate::tipos_do_motor::*;
use crate::texto_e_numeros::tramp_long_str;
use crate::arquivos_do_jogo::*;
use crate::entidades::*;
#[cfg(feature = "devtools")] use crate::aparencia::*; // aqui só os handlers de prova usam, e o `corpo_de_prova!` público não os cita
use crate::interface::*;

/// Pedaço `i` de `s`: caracteres inteiros até 19 bytes por pedaço, contados desde o início (vazio depois do fim). A versão anterior cortava 16
/// caracteres e encurtava o pedaço multibyte que passasse de 19 bytes, mas o seguinte começava no caractere 16·(i+1) e o que sobrou se perdia
/// (achado da 100-3, 2026-09-12; os textos em pedaços até então eram ASCII).
pub(crate) fn pedaco_utf8(s: &str, i: usize) -> String {
    const MAX_BYTES: usize = 19;
    let mut parte = 0usize;
    let mut atual = String::new();
    for c in s.chars() {
        if atual.len() + c.len_utf8() > MAX_BYTES {
            if parte == i {
                return atual;
            }
            parte += 1;
            atual.clear();
        }
        atual.push(c);
    }
    if parte == i { atual } else { String::new() }
}




/// Coleta SEGURA do endereço da vtable da `MappinSystem`, pelo CANAL.
///
/// **Por que pelo canal e não por smoke (medido 2026-09-05).** Com os gates de smoke desligados o
/// boot chega à gameplay de forma confiável; com QUALQUER smoke ligado em `OnGameAttached` ele
/// morre entre 45 e 60s — 3 medições seguidas, inclusive com o gate mais leve. O canal de comando
/// dispara DEPOIS da gameplay confirmada, que é exatamente por que `depotres`/`mapfind` renderam
/// dado ontem e os smokes não. Instrumento que derruba o processo não é instrumento.
///
/// Só LÊ: pega o ponteiro da vtable, conta os slots (com cache) e reporta o endereço ESTÁTICO, pra
/// a análise de qual slot é qual ser feita offline com `re-xref vtable-dump`. Zero chamada.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn run_poi_vtable() {
    let Some(reg) = rtti::Registry::obtain() else {
        crate::log_dev!("[poivtable] RTTI indisponível");
        return;
    };
    let player = crate::current_player();
    if player.is_null() {
        crate::log_dev!("[poivtable] sem player — precisa de GAMEPLAY");
        return;
    }
    let Some(b) = crate::jogo::sessao::game_instance(&reg, player) else {
        crate::log_dev!("[poivtable] GetGame falhou");
        return;
    };
    let game = usize::from_le_bytes(b[..8].try_into().unwrap_or([0; 8])) as *mut c_void;
    // Nome de classe no RTTI não é o nome que o redscript usa: a chave do systemMap é `gamemappinsMappinSystem`
    // (`proofs/20260926-sistema-do-motor-igual-ao-andador-antigo-PROVADO.log`); `IMappinSystem`, `MappinSystem`, `gameIMappinSystem` e
    // `gamemappinsIMappinSystem` não resolvem (boot de 2026-09-26 04:04).
    let sys = crate::jogo::sessao::sistema_do_motor_por_nome(&reg, game, "gamemappinsMappinSystem", |_| true);
    if sys.is_null() || !crate::memoria::is_readable(sys, 8) {
        crate::veredito_dev!("[poivtable] gamemappinsMappinSystem não resolveu no systemMap");
        return;
    }
    let vt_runtime = (sys as *const u64).read_unaligned();
    let base_runtime = crate::rebase(0x1_0000_0000) as u64;
    let vt_estatico = vt_runtime.wrapping_sub(base_runtime).wrapping_add(0x1_0000_0000);
    let n = vtable_slot_count_cached(sys);
    crate::veredito_dev!(
        "[poivtable] MappinSystem sys={sys:p} vtable_runtime={vt_runtime:#x} vtable_estatico={vt_estatico:#x} slots={n} (fim {:#x})",
        n * 8
    );
    crate::veredito_dev!(
        "[poivtable] o `0x3D0` é o slot {} — {} numa tabela de {n}",
        0x3D0usize / 8,
        if 0x3D0usize / 8 < n { "DENTRO" } else { "FORA" }
    );
    crate::veredito_dev!(
        "[poivtable] analisar offline: re-xref <bin> vtable-dump {vt_estatico:#x} {n}"
    );
}

/// Última resolução de `setor_por_path`: `(quando, tick, hash do path, recurso)`.
pub(crate) static SETOR_POR_PATH_MEMO: std::sync::Mutex<Option<(std::time::Instant, u64, u64, usize)>> = std::sync::Mutex::new(None);

/// Validade da resolução guardada, dentro do mesmo tick.
pub(crate) const SETOR_POR_PATH_MEMO_MS: u128 = 100;

/// Resolve um path de `.streamingsector` até o `CResource*` vivo, ou nulo se não estiver no cache. O `depot_loaded_resource` varre o cache inteiro (dezenas
/// de milhares de entradas, com checagem de legibilidade por slot), e os leitores da API da 85d chamam isto milhares de vezes num frame: no boot 84 o smoke
/// da API prendeu a thread do jogo até o harness fechar. A resolução fica guardada por até 100 ms no mesmo tick — o mesmo tempo em que o censo da 85b já
/// segura o ponteiro numa chamada só —; tick novo ou prazo vencido refazem a varredura.
pub(crate) unsafe fn setor_por_path(path: &str) -> *mut c_void {
    let hash = bwms_primitives::resource_path_hash(path);
    let tick = crate::ticks();
    if let Ok(m) = SETOR_POR_PATH_MEMO.lock() {
        if let Some((quando, t, h, p)) = *m {
            if t == tick && h == hash && quando.elapsed().as_millis() < SETOR_POR_PATH_MEMO_MS {
                return p as *mut c_void;
            }
        }
    }
    let r = depot_loaded_resource(hash);
    if let Ok(mut m) = SETOR_POR_PATH_MEMO.lock() {
        *m = Some((std::time::Instant::now(), tick, hash, r as usize));
    }
    r
}

/// `nodes: DynArray<Handle<worldNode>>` na entrada do CACHE do depot.
///
/// **CORREÇÃO 2026-09-06:** havia DUAS constantes pro mesmo campo — esta valia `0x48` e a
/// `SECTOR_NODES` (usada pelos acessores novos) valia `0x68`. `0x48` veio da entrada
/// aplicação no streaming de 2026-07-26, que descreve o objeto recebido pelo hook de **PostLoad**,
/// não o do cache; na verdade `0x48` é `0x40+0x08`, o campo **size do `DataBuffer` de
/// `nodeSetups`` — o log ao vivo mostrou `entries=0x1000211e00`, exatamente o slot do DataBuffer.
/// O crivo de coerência salvou (devolvia `-2`, nunca número falso), mas o acessor ficava inútil:
/// 0 de 4 setores "resolvidos" enquanto o irmão lia 5.689 nós no mesmo setor.
///
/// Ficou UMA constante. Duas grafias do mesmo offset é exatamente como a divergência nasceu.
///
/// 2.5: o `NodeBuffer` ganhou a versão u32 em `+0x28` (vale `0x44`), e `nodes`/`nodeRefs` foram para `+0x30`/`+0x40` dele. Ler o `+0x68` antigo
/// cai na versão e o crivo devolve `-2`.
pub(crate) const SECTOR_NODES: usize = 0x70;

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_sector_node_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let sec = setor_por_path(&path);
    // -1 = não está no cache (nunca 0, que se confundiria com "setor sem nó").
    if sec.is_null() || !crate::memoria::is_readable(sec, SECTOR_NODES + 0x10) {
        write_i32_ret(out, -1);
        return;
    }
    let base = sec as usize + SECTOR_NODES;
    let (entries_u, size, cap) = dynarray_em(base as *const c_void);
    let entries = entries_u as u64;
    // Um trio incoerente (`size > capacity`, ponteiro nulo com tamanho, capacidade absurda) é offset
    // errado lido como dado, não setor estranho. -2 diz isso sem mentir "0".
    if !dynarray_coerente(entries_u, size, cap) {
        crate::log_dev!(
            "[cw86] '{path}' INCOERENTE em +{SECTOR_NODES:#x}: entries={entries:#x} size={size} cap={cap}"
        );
        write_i32_ret(out, -2);
        return;
    }
    crate::log_dev!("[cw86] '{path}' nodes: size={size} cap={cap} entries={entries:#x}");
    write_i32_ret(out, size as i32);
}

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_sector_class(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let s = depot_loaded_resource_class_by_hash(bwms_primitives::resource_path_hash(&path));
    let _ = write_cstring_inline_ret(out, &s);
}

/// Varre os primeiros `teto` bytes de um setor procurando QUALQUER `DynArray` coerente.
///
/// Existe porque a medição de 2026-09-05 mostrou que `+0x48` — o offset que o `DATABASE.md`
/// registra desde 2026-07-26 — **não** tem DynArray na entrada do cache: lá há um ponteiro em
/// `+0x48` e OUTRO em `+0x50` (o "size=153679888 cap=1" era a metade baixa e alta de um u64).
/// O offset do DATABASE foi medido no objeto que o hook de PostLoad recebe, que não é
/// necessariamente o mesmo endereço que o cache guarda.
///
/// A varredura é o passo honesto: em vez de escolher outro offset por palpite, mede TODOS e
/// deixa o crivo de coerência decidir quais são candidatos. Um DynArray coerente tem ponteiro
/// legível, `size <= capacity` e capacidade sã — três condições que lixo raramente satisfaz de
/// uma vez, e por isso a lista que sai é curta o bastante pra um humano conferir.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_sector_scan(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let teto = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0x200).min(0x800);
    let sec = setor_por_path(&path);
    if sec.is_null() {
        crate::log_dev!("[cw86-scan] '{path}' não está no cache");
        write_i32_ret(out, -1);
        return;
    }
    if !crate::memoria::is_readable(sec, teto + 0x10) {
        crate::log_dev!("[cw86-scan] '{path}' ilegível até +{teto:#x}");
        write_i32_ret(out, -2);
        return;
    }
    let mut achados = 0i32;
    // Passo de 8: DynArray sempre começa alinhado a ponteiro.
    for off in (0..teto).step_by(8) {
        let b = (sec as usize + off) as *const c_void;
        let (entries_u, size, cap) = dynarray_em(b);
        let entries = entries_u as u64;
        let coerente = entries != 0
            && cap > 0
            && dynarray_coerente(entries_u, size, cap)
            && crate::memoria::is_readable(entries as *const c_void, 8);
        if coerente {
            achados += 1;
            crate::log_dev!(
                "[cw86-scan] +{off:#05x}: DynArray coerente entries={entries:#x} size={size} cap={cap}"
            );
        }
    }
    crate::log_dev!("[cw86-scan] '{path}': {achados} candidato(s) até +{teto:#x}");
    write_i32_ret(out, achados);
}

/// Classifica o CONTEÚDO de um DynArray candidato do setor, para dizer qual dos três ele é.
///
/// Os 8 métodos que o item `#86` pede se distribuem em 3 arrays — `nodes`, `nodeSetups`,
/// `nodeRefs` — e a varredura anterior achou 3 candidatos com a assinatura certa. Chutar qual é
/// qual seria inventar; o conteúdo distingue sem ambiguidade:
///
/// - **`nodes`** é `DynArray<Handle<worldNode>>`: cada elemento é um ponteiro, e o objeto
///   apontado tem vtable RTTI — logo `rtti::class_of` devolve um NOME (`worldMeshNode`, etc.).
/// - **`nodeRefs`** é `DynArray<NodeRef>`: hash de 8 bytes, não ponteiro. Ilegível como endereço.
/// - **`nodeSetups`** é `DynArray<CompiledNodeInstanceSetupInfo>`: struct POR VALOR, com stride
///   grande (transform+scale+ids). Elemento 0 e 1 ficam a mais de 8 bytes um do outro.
///
/// O stride sai da própria varredura: `(entries[1] - entries[0])` não serve porque não temos os
/// endereços; então a heurística é ler os 2 primeiros qwords e ver o que eles PARECEM.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_sector_classify(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let off = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let sec = setor_por_path(&path);
    if sec.is_null() || !crate::memoria::is_readable(sec, off + 0x10) {
        crate::log_dev!("[cw86-cls] +{off:#x}: setor não resolvido");
        write_i32_ret(out, -1);
        return;
    }
    let b = (sec as usize + off) as *const c_void;
    let (entries_u, size, _cap) = dynarray_em(b);
    let entries = entries_u as *const c_void;
    if entries.is_null() || size == 0 || !crate::memoria::is_readable(entries, 0x20) {
        crate::log_dev!("[cw86-cls] +{off:#x}: entries ilegível");
        write_i32_ret(out, -2);
        return;
    }
    // Os 4 primeiros qwords do array, crus. É deles que sai a classificação.
    let q: Vec<u64> = (0..4).map(|i| rd_u64(entries, i * 8)).collect();
    // Elemento 0 é ponteiro pra objeto RTTI? Então é `nodes`.
    let mut classe = String::new();
    if q[0] > 0x1000 && crate::memoria::is_readable(q[0] as *const c_void, 8) {
        // Mesmo idioma provado no resto do projeto: `class_of` (vtable+0x08 = GetType) e
        // `type_name_getname` (CClass vtable+0x10 = GetName), resolvido pelo dicionário de CName.
        let cls = rtti::class_of(q[0] as *mut c_void);
        if !cls.is_null() {
            classe = crate::cname::resolve_cname(rtti::type_name_getname(cls));
        }
    }
    let veredito = if !classe.is_empty() {
        "nodes (Handle<worldNode> — classe RTTI resolvida)"
    } else if q[0] > 0x1000 && q[1] > 0x1000 && crate::memoria::is_readable(q[0] as *const c_void, 8) {
        "ponteiros, mas sem classe RTTI — talvez nodes ainda sem vtable"
    } else {
        "valores por VALOR (hash ou struct) — nodeRefs ou nodeSetups"
    };
    crate::log_dev!(
        "[cw86-cls] +{off:#x} size={size} q0={:#x} q1={:#x} q2={:#x} q3={:#x} classe='{classe}' -> {veredito}",
        q[0], q[1], q[2], q[3]
    );
    write_i32_ret(out, if classe.is_empty() { 0 } else { 1 });
}

/// Mede o STRIDE de um array do setor — o que separa `nodeRefs` de `nodeSetups`.
///
/// `nodeRefs` é `DynArray<NodeRef>`: elemento de **8 bytes**, então o último elemento vive em
/// `entries + (size-1)*8` e essa região inteira é legível. `nodeSetups` é
/// `DynArray<CompiledNodeInstanceSetupInfo>`: struct grande (transform+scale+ids), então com
/// stride 8 o "último elemento" cairia bem antes do fim real do buffer — e, ao contrário,
/// assumir stride 8 num array de structs faz a leitura passar por CIMA de campos internos.
///
/// O teste é direto: se `entries + size*8` é legível E os elementos amostrados ao longo desse
/// intervalo têm cara de hash (valor grande, sem byte alto zerado, não-ponteiro), stride é 8 e o
/// array é `nodeRefs`. Se a região de `size*8` NÃO for legível, o array não pode ter stride 8 —
/// e aí é `nodeSetups`, cujo stride é maior.
///
/// Devolve o stride inferido (8) ou 0 quando a evidência aponta struct maior.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_sector_stride(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let off = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let sec = setor_por_path(&path);
    if sec.is_null() || !crate::memoria::is_readable(sec, off + 0x10) {
        write_i32_ret(out, -1);
        return;
    }
    let b = (sec as usize + off) as *const c_void;
    let (entries, size_u32, _cap) = dynarray_em(b);
    let size = size_u32 as usize;
    if entries == 0 || size == 0 {
        write_i32_ret(out, -2);
        return;
    }
    let bytes_se_8 = size * 8;
    let cabe_8 = crate::memoria::is_readable(entries as *const c_void, bytes_se_8);
    // Amostra ao longo do intervalo, não só no começo: um array de structs lido de 8 em 8
    // mostraria ponteiros e floats no meio, um array de hash mostra hash em todo lugar.
    let mut hashes = 0;
    let mut amostras = 0;
    for i in [0usize, size / 4, size / 2, size * 3 / 4, size - 1] {
        let p = entries + i * 8;
        if !crate::memoria::is_readable(p as *const c_void, 8) {
            continue;
        }
        amostras += 1;
        let v = rd_u64(p as *const c_void, 0);
        // Hash de NodeRef: valor grande, não alinhado como ponteiro, e fora da faixa de
        // endereço do processo (ponteiros aqui começam em 0x7x/0x1xx).
        let parece_hash = v > 0xFFFF_FFFF && (v >> 40) != 0 && !crate::memoria::is_readable(v as *const c_void, 8);
        if parece_hash {
            hashes += 1;
        }
    }
    let stride8 = cabe_8 && amostras > 0 && hashes * 2 >= amostras;
    crate::log_dev!(
        "[cw86-str] +{off:#x} size={size} cabe_como_8={cabe_8} amostras={amostras} parecem_hash={hashes} -> {}",
        if stride8 { "stride=8 => nodeRefs" } else { "stride>8 => nodeSetups (ou indefinido)" }
    );
    write_i32_ret(out, if stride8 { 8 } else { 0 });
}

pub(crate) const SECTOR_NODEREFS: usize = 0x80;

/// `(entries, size)` de um DynArray do setor, já com o crivo de coerência aplicado.
pub(crate) unsafe fn sector_array(sec: *mut c_void, off: usize) -> Option<(usize, u32)> {
    if sec.is_null() || !crate::memoria::is_readable(sec, off + 0x10) {
        return None;
    }
    let (entries, size, cap) = dynarray_em((sec as usize + off) as *const c_void);
    if !dynarray_coerente(entries, size, cap) {
        return None;
    }
    Some((entries, size))
}

/// Só pros dois que SÃO `DynArray`. `nodeSetups` (qual=2) não entra aqui de propósito: é
/// `DataBuffer`, decodificado por [`sector_setups`] — misturar os dois foi o bug de 2026-09-05.
pub(crate) fn off_por_indice(qual: i32) -> usize {
    match qual {
        1 => SECTOR_NODEREFS,
        _ => SECTOR_NODES,
    }
}

/// `GetNodeCount` / `GetNodeRefCount` / `GetNodeSetupCount` — `qual`: 0=nodes 1=refs 2=setups.
/// `-1` = setor fora do cache; `-2` = array incoerente. Nunca 0 pros dois, porque 0 é uma
/// contagem legítima e confundir isso é como um achado negativo nasce falso.
pub(crate) unsafe extern "C" fn tramp_sector_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let qual = a.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
    let sec = setor_por_path(&path);
    if sec.is_null() {
        write_i32_ret(out, -1);
        return;
    }
    let medido = if qual == 2 {
        sector_setups(sec)
    } else {
        sector_array(sec, off_por_indice(qual))
    };
    match medido {
        Some((_, size)) => write_i32_ret(out, size as i32),
        None => write_i32_ret(out, -2),
    }
}

/// `GetNode(i)` como HASH do nome de classe — sem limite de tamanho.
///
/// Devolver o hash resolve sem truncar nem mentir: o redscript compara contra o hash do nome
/// que ele espera, e o dicionário de CName reverte quando precisa.
pub(crate) unsafe extern "C" fn tramp_sector_node_class_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let i = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let sec = setor_por_path(&path);
    let mut h = 0u64;
    if let Some((entries, size)) = sector_array(sec, SECTOR_NODES) {
        if i < size as usize {
            // `Handle<T>` do RED é `{instance*, refcount*}` = **16 bytes**, não 8. Lendo com
            // passo 8, o índice 7 caía na metade de refcount do elemento 3 e devolvia 0 —
            // medido ao vivo em 2026-09-05 (índice 0 resolvia, 7 não). O `instance` é o 1º
            // qword de cada par.
            let p = entries + i * HANDLE_STRIDE;
            if crate::memoria::is_readable(p as *const c_void, 8) {
                let cls = rtti::class_of(rd_u64(p as *const c_void, 0) as *mut c_void);
                if !cls.is_null() {
                    h = rtti::type_name_getname(cls);
                }
            }
        }
    }
    write_uint_ret(out, h, 8);
}

/// `GetNodeRef(i)` — o hash do NodeRef `i`. Stride 8 confirmado ao vivo. `0` = fora de faixa,
/// que é distinguível de um hash real (nenhum NodeRef válido é zero).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_sector_noderef(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let i = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let sec = setor_por_path(&path);
    let mut h = 0u64;
    if let Some((entries, size)) = sector_array(sec, SECTOR_NODEREFS) {
        if i < size as usize {
            let p = entries + i * 8;
            if crate::memoria::is_readable(p as *const c_void, 8) {
                h = rd_u64(p as *const c_void, 0);
            }
        }
    }
    write_uint_ret(out, h, 8);
}

/// `(base, quantidade)` de `nodeSetups`. Layout do `DataBuffer` **MEDIDO ao vivo 2026-09-05**,
/// não suposto:
///
/// ```text
/// buffer@+0x40 slots=[0x8d2a58c000, 0x1000211e00, 0x10b233810, 0x10b5597c8, 0x0]
///                     └ data @+0x00  └ size u32 @+0x08 (0x211e00 / 0x90 = 15072 setups)
/// ```
///
/// O `+0x0C` trazia `0x10` (alinhamento/flag, não tamanho) — por isso o tamanho se lê como
/// **u32**, nunca como o u64 inteiro do slot.
///
/// A versão anterior varria os 5 slots atrás de um par coerente. Serviu pra DESCOBRIR o layout
/// (foi o log dela que deu os números acima) e sai agora que ele está confirmado: heurística que
/// já cumpriu o papel vira risco de casar com o par errado noutro setor.
///
/// Confere o resultado antes de devolver — `size` múltiplo de `0x90` e região inteira legível.
/// `None` quando não fecha, nunca um número plausível: foi exatamente assim que o `1` falso
/// passou por prova em 2026-09-05.
pub(crate) unsafe fn sector_setups(sec: *mut c_void) -> Option<(usize, u32)> {
    if sec.is_null() || !crate::memoria::is_readable(sec, SECTOR_NODEBUFFER + 0x10) {
        return None;
    }
    let b = (sec as usize + SECTOR_NODEBUFFER) as *const c_void;
    let data = rd_u64(b, 0) as usize;
    let bytes = rd_u32(b, 8) as usize;
    if data == 0 || bytes == 0 || bytes % SETUP_SIZE != 0 || bytes > (1 << 26) {
        return None;
    }
    if !crate::memoria::is_readable(data as *const c_void, bytes) {
        return None;
    }
    Some((data, (bytes / SETUP_SIZE) as u32))
}

/// `GetNodeSetup(i, offset)` — lê um `u32` DENTRO do setup `i`.
///
/// Leitura arbitrária DENTRO do elemento, com limite duplo — índice contra `size` e offset contra
/// `0x90` —, pra ninguém ler fora do elemento por engano. O mapa dos offsets dos campos está no
/// doc do `tramp_sector_setup_f32`.
///
/// `-1` = setor fora do cache · `-2` = array incoerente · `-3` = índice ou offset fora de faixa.
/// Três negativos distintos porque confundi-los é como um achado falso nasce.
pub(crate) unsafe extern "C" fn tramp_sector_setup(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let i = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let off = a.get(2).map(|(v, _)| *v as usize).unwrap_or(0);
    let sec = setor_por_path(&path);
    if sec.is_null() {
        write_i32_ret(out, -1);
        return;
    }
    let Some((entries, size)) = sector_setups(sec) else {
        write_i32_ret(out, -2);
        return;
    };
    if i >= size as usize || off + 4 > SETUP_SIZE {
        write_i32_ret(out, -3);
        return;
    }
    let p = entries + i * SETUP_SIZE + off;
    if !crate::memoria::is_readable(p as *const c_void, 4) {
        write_i32_ret(out, -3);
        return;
    }
    write_i32_ret(out, rd_u32(p as *const c_void, 0) as i32);
}

/// `WorldNodeSetupWrapper` — leitura TIPADA dentro do setup `i`.
///
/// O item pede 13 getters. Em vez de 13 nativas (uma por campo), 2 leitores tipados + os offsets
/// do layout, e os getters viram composição do lado redscript — os offsets já são conhecidos e
/// fixos, então o que faltava era só ler no tipo certo. `BwmsSectorSetup` (u32) já cobria os
/// inteiros; faltavam float (transform/scale/distâncias) e u64 (globalNodeID/proxyNodeID).
///
/// Mapa dos campos dentro dos `0x90` (`CompiledNodeInstanceSetupInfo`):
/// `transform@0x00` (`position:Vector4@0x00`, `orientation:Quaternion@0x10`), `scale@0x20`,
/// `secondaryRefPointPosition@0x2C`, `streamingRefPoint@0x38`, `node*@0x50`, `globalNodeID@0x58`,
/// `proxyNodeID@0x60`, `nodeIndex` nos 19 bits de baixo do u64 de `0x70`, `secondaryRefPointDistance@0x78`,
/// `streamingDistance@0x7C`. É o layout da versão 68 do `NodeBuffer` (`+0x28`); setor de versão 62 (o da 2.31: distâncias em `0x70`/`0x74`,
/// índice em `0x78`) a 2.5 converte para ele no carregamento, antes do bind (`0x101a8e9f8`, Steam 2.5).
///
/// Os 12 setters do item compõem sobre os 3 escritores logo abaixo (`BwmsSectorSetupSet*`).
pub(crate) unsafe extern "C" fn tramp_sector_setup_f32(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let i = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let off = a.get(2).map(|(v, _)| *v as usize).unwrap_or(0);
    let mut val = f32::NAN; // NaN = "não li", distinto de 0.0 que é valor legítimo de coordenada
    if let Some(p) = setup_field_ptr(&path, i, off, 4) {
        val = f32::from_bits(rd_u32(p as *const c_void, 0));
    }
    write_f32_ret(out, val);
}

pub(crate) unsafe extern "C" fn tramp_sector_setup_u64(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let i = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let off = a.get(2).map(|(v, _)| *v as usize).unwrap_or(0);
    let val = setup_field_ptr(&path, i, off, 8).map_or(0u64, |p| rd_u64(p as *const c_void, 0));
    write_uint_ret(out, val, 8);
}

/// Endereço de um campo de `largura` bytes dentro do setup `i`, ou `None`. Fonte ÚNICA do
/// limite duplo (índice contra a contagem, offset+largura contra `0x90`) pros 3 leitores —
/// duplicar essa checagem por leitor é como um deles acaba lendo fora do elemento.
pub(crate) unsafe fn setup_field_ptr(path: &str, i: usize, off: usize, largura: usize) -> Option<usize> {
    let sec = setor_por_path(path);
    let (entries, size) = sector_setups(sec)?;
    if i >= size as usize || off + largura > SETUP_SIZE {
        return None;
    }
    let p = entries + i * SETUP_SIZE + off;
    crate::memoria::is_readable(p as *const c_void, largura).then_some(p)
}

// ===== Os 12 setters do `WorldNodeSetupWrapper` =====
//
// Compostos no redscript (`bwms-world-node-setup.reds`) sobre 3 escritores tipados, como os getters sobre os leitores. Os escritores de campo recusam o
// ponteiro do nó (`+0x50`), que só troca pelo `SetNodeIndex`, junto do índice, com o nó do array `nodes` do setor.

// 2.5: índice do nó nos 19 bits de baixo do u64 de `+0x70` do setup (na 2.31, em `+0x78`); os 45 de cima são outros campos e ficam.
pub(crate) const SETUP_NO_INDICE: usize = 0x70;
pub(crate) const SETUP_NO_INDICE_MASCARA: u64 = 0x7ffff;

/// Como [`setup_field_ptr`], recusando qualquer faixa que encoste no ponteiro do nó ou no u64 do índice: os dois só trocam juntos.
pub(crate) unsafe fn setup_escrita_ptr(path: &str, i: usize, off: usize, largura: usize) -> Option<usize> {
    let encosta = |ini: usize| off < ini + 8 && off + largura > ini;
    if encosta(NODESETUP_NO_OFF) || encosta(SETUP_NO_INDICE) {
        return None;
    }
    setup_field_ptr(path, i, off, largura)
}

/// `(path, i, offset, valor)` dos escritores; índice e offset são `Int32`, então negativo vira fora de faixa.
pub(crate) unsafe fn setup_escrita_args(frame: *mut c_void) -> (String, usize, usize, u64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let g = |k: usize| a.get(k).map_or(0, |(v, _)| *v);
    (strings.first().cloned().flatten().unwrap_or_default(), g(1) as u32 as usize, g(2) as u32 as usize, g(3))
}

/// `BwmsSectorSetupSetF32(path, i, offset, value) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_sector_setup_set_f32(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (path, i, off, valor) = setup_escrita_args(frame);
    let p = setup_escrita_ptr(&path, i, off, 4);
    if let Some(p) = p {
        (p as *mut u32).write_unaligned(valor as u32);
    }
    write_bool_ret(out, p.is_some());
}

/// `BwmsSectorSetupSetU64(path, i, offset, value) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_sector_setup_set_u64(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (path, i, off, valor) = setup_escrita_args(frame);
    let p = setup_escrita_ptr(&path, i, off, 8);
    if let Some(p) = p {
        (p as *mut u64).write_unaligned(valor);
    }
    write_bool_ret(out, p.is_some());
}

/// `BwmsSectorSetupSetNodeIndex(path, i, index) -> Bool`: índice (os 19 bits do campo) e ponteiro do nó do array `nodes`. O índice é conferido contra a
/// quantidade de nós; conferir contra a de setups, que é maior (15072 contra 5689 no `always_loaded_0`), deixaria ler fora do array.
pub(crate) unsafe extern "C" fn tramp_sector_setup_set_node_index(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (path, i, indice, _) = setup_escrita_args(frame);
    let ok = match setup_e_no(&path, i, indice) {
        Some((setup, no, nos)) if indice < nos as usize && indice as u64 <= SETUP_NO_INDICE_MASCARA && no != 0 => {
            let campo = setup.add(SETUP_NO_INDICE) as *mut u64;
            campo.write_unaligned(campo.read_unaligned() & !SETUP_NO_INDICE_MASCARA | indice as u64);
            (setup.add(NODESETUP_NO_OFF) as *mut u64).write_unaligned(no);
            true
        }
        _ => false,
    };
    write_bool_ret(out, ok);
}

/// Devolve a QUANTIDADE de setups medida por [`sector_setups`], e loga os 5 slots crus do
/// `DataBuffer` — é o log que fixa o layout de uma vez, em vez de deixá-lo suposto.
///
/// Legibilidade não confirma o stride `0x90`: 1 elemento de 144 bytes é legível em quase
/// qualquer endereço válido. Por isso a quantidade vem do `sector_setups`, que confere o `size`.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_sector_setup_stride(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let sec = setor_por_path(&path);
    let mut r = 0i32;
    if !sec.is_null() && crate::memoria::is_readable(sec, SECTOR_NODEBUFFER + 0x28) {
        let base = (sec as usize + SECTOR_NODEBUFFER) as *const c_void;
        let slots: [u64; 5] = core::array::from_fn(|i| rd_u64(base, i * 8));
        let medido = sector_setups(sec);
        r = medido.map_or(0, |(_, n)| n as i32);
        crate::log_dev!(
            "[cw86-setup] buffer@+{SECTOR_NODEBUFFER:#x} data={:#x} size={:#x} slots=[{}] medido={medido:x?} qtd={r}",
            slots[0], slots[1] & 0xffff_ffff,
            slots.iter().map(|v| format!("{v:#x}")).collect::<Vec<_>>().join(", ")
        );
    }
    write_i32_ret(out, r);
}

/// `BwmsSectorVtableSlot(path, slot) -> Uint64` — o valor de um slot de vtable do
/// `worldStreamingSector` resolvido por PATH. `0` se o setor não estiver no cache ou o slot for
/// ilegível. **Leitura pura: nunca chama o slot.**
///
/// **Por que uma variante por path.** `BwmsVtableSlot` recebe `ref<IScriptable>`, e um setor não é
/// alcançável como handle a partir do redscript — ele vem do cache do depot por hash de path
/// (`#86`, provado 2026-09-05). Sem isto, a família inteira de itens que depende de vtable de
/// SETOR fica fora do alcance da verificação read-only que o `#58` estabeleceu.
///
/// **Primeiro uso — pós-carga de setor:** o item precisa hookar
/// `worldStreamingSector::PostLoad`, endereço `0x101c05360` já resolvido em 2026-07-25 e
/// RECONFIRMADO offline em 2026-09-06 por `re-xref vtable-of` (índice 6 = offset `0x30`, o mesmo
/// slot de PostLoad de outras 3 classes). Comparar `BwmsSectorVtableSlot(path, 0x30)` contra
/// `BwmsRebase(0x101c05360)` prova o alvo do hook **sem instalar hook nenhum** — que é exatamente
/// a disciplina que tirou o `#58` do impasse "só dá pra provar chamando".
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_sector_vtable_slot(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let slot = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let mut v = 0u64;
    if slot <= 0x400 && !path.is_empty() {
        let sec = setor_por_path(&path);
        if rtti::sane(sec) && crate::memoria::is_readable(sec as *const c_void, 8) {
            let vt = rd_u64(sec as *const c_void, 0) as *mut c_void;
            if rtti::sane(vt) && crate::memoria::is_readable(vt as *const c_void, slot + 8) {
                v = rd_u64(vt as *const c_void, slot);
            }
        }
    }
    write_uint_ret(out, v, 8);
}

pub(crate) unsafe fn register_sector_86_utils(reg: &Registry, proto: *mut c_void) {
    let a = registro_de_prova!(register_argful_by_types(reg, proto, &["String"], "BwmsSectorNodeCount", "BwmsSectorNodeCount", corpo_de_prova!(tramp_sector_node_count)));
    let b = registro_de_prova!(register_argful_by_types(reg, proto, &["String"], "BwmsSectorClass", "BwmsSectorClass", corpo_de_prova!(tramp_sector_class)));
    // `BwmsLongStr` — a saída pro limite de 20 bytes do retorno de String (ver
    // `write_cstring_chunk_ret`). Registrada aqui, junto do `BwmsSectorClass`, porque é ele o
    // caso de uso que motivou.
    let _bls = register_argful_by_types(reg, proto, &["String", "String", "Int32"], "BwmsLongStr", "BwmsLongStr", tramp_long_str as NativeHandler);
    let c = registro_de_prova!(register_argful_by_types(reg, proto, &["String", "Int32"], "BwmsSectorScan", "BwmsSectorScan", corpo_de_prova!(tramp_sector_scan)));
    let d = registro_de_prova!(register_argful_by_types(reg, proto, &["String", "Int32"], "BwmsSectorClassify", "BwmsSectorClassify", corpo_de_prova!(tramp_sector_classify)));
    let e = registro_de_prova!(register_argful_by_types(reg, proto, &["String", "Int32"], "BwmsSectorStride", "BwmsSectorStride", corpo_de_prova!(tramp_sector_stride)));
    let f = register_argful_by_types(reg, proto, &["String", "Int32"], "BwmsSectorCount", "BwmsSectorCount", tramp_sector_count as NativeHandler);
    let h = registro_de_prova!(register_argful_by_types(reg, proto, &["String", "Int32"], "BwmsSectorNodeRef", "BwmsSectorNodeRef", corpo_de_prova!(tramp_sector_noderef)));
    let i = register_argful_by_types(reg, proto, &["String", "Int32"], "BwmsSectorNodeClassHash", "BwmsSectorNodeClassHash", tramp_sector_node_class_hash as NativeHandler);
    let j = register_argful_by_types(reg, proto, &["String", "Int32", "Int32"], "BwmsSectorSetup", "BwmsSectorSetup", tramp_sector_setup as NativeHandler);
    let k = registro_de_prova!(register_argful_by_types(reg, proto, &["String"], "BwmsSectorSetupStride", "BwmsSectorSetupStride", corpo_de_prova!(tramp_sector_setup_stride)));
    // Leitores tipados sobre o MESMO setup — os 13 getters compõem daqui.
    let sf = register_argful_by_types(reg, proto, &["String", "Int32", "Int32"], "BwmsSectorSetupF32", "BwmsSectorSetupF32", tramp_sector_setup_f32 as NativeHandler);
    let su = register_argful_by_types(reg, proto, &["String", "Int32", "Int32"], "BwmsSectorSetupU64", "BwmsSectorSetupU64", tramp_sector_setup_u64 as NativeHandler);
    // Os 3 escritores dos 12 setters (`bwms-world-node-setup.reds`).
    let escritores = [
        register_argful_by_types(reg, proto, &["String", "Int32", "Int32", "Float"], "BwmsSectorSetupSetF32", "BwmsSectorSetupSetF32", tramp_sector_setup_set_f32 as NativeHandler),
        register_argful_by_types(reg, proto, &["String", "Int32", "Int32", "Uint64"], "BwmsSectorSetupSetU64", "BwmsSectorSetupSetU64", tramp_sector_setup_set_u64 as NativeHandler),
        register_argful_by_types(
            reg,
            proto,
            &["String", "Int32", "Int32"],
            "BwmsSectorSetupSetNodeIndex",
            "BwmsSectorSetupSetNodeIndex",
            tramp_sector_setup_set_node_index,
        ),
    ];
    crate::log_dev!("[reg] register_all: setup-83 escritores do setup {}/3 registrados", escritores.iter().filter(|x| **x).count());
    let l = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsFirstComponentId", "BwmsFirstComponentId", corpo_de_prova!(tramp_first_component_id)));
    // Widget de mundo: o único pedaço NATIVO do item — o resto (`GetWorldWidgetComponent`) é
    // redscript puro sobre tipos que já existem no corpus.
    let w96 = register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsInkWorldWidgetComponent", "BwmsInkWorldWidgetComponent", tramp_ink_world_widget_component as NativeHandler);
    let wp = register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsWidgetParent", "BwmsWidgetParent", tramp_widget_parent as NativeHandler);
    // O `.mesh` do componente como HASH — a cadeia hash→depot→CResource* já existe.
    // `BwmsMeshRefHash`/`BwmsMeshRefAppearance` foram REMOVIDAS: `BwmsPropU64(obj, "mesh")` e
    // `BwmsPropU64(obj, "meshAppearance")` fazem o mesmo e valem pra qualquer propriedade. Manter
    // as três seria manter três grafias da mesma coisa — foi assim que o offset duplicado do
    // `#86` sobreviveu semanas.
    let s30 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32"], "BwmsMeshRefScan", "BwmsMeshRefScan", corpo_de_prova!(tramp_mesh_ref_scan)));
    // Via GENÉRICA: o offset vem do RTTI desta build, não de tabela. Cobre os ~1.107 campos que mods
    // leem por `@addField` e que o Mac não tem como property script-visível.
    let po = register_argful_by_types(reg, proto, &["handle:IScriptable", "String"], "BwmsPropOffset", "BwmsPropOffset", tramp_prop_offset as NativeHandler);
    let pu = register_argful_by_types(reg, proto, &["handle:IScriptable", "String", "Int32"], "BwmsPropU64", "BwmsPropU64", tramp_prop_u64 as NativeHandler);
    let ps = register_argful_by_types(reg, proto, &["handle:IScriptable", "String", "Int32", "Uint64"], "BwmsPropSetU64", "BwmsPropSetU64", tramp_prop_set_u64 as NativeHandler);
    crate::veredito_dev!("[reg] campo-91 escrita de campo: BwmsPropSetU64={ps}");
    // leitura pura de slot de vtable — fecha o loop com `re-xref vtable-of` (ver o tramp).
    let vs = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32"], "BwmsVtableSlot", "BwmsVtableSlot", corpo_de_prova!(tramp_vtable_slot)));
    // estático -> runtime (slide de ASLR). Sem isso, comparar endereço de catálogo com
    // endereço lido de objeto vivo dá falso negativo garantido.
    let rb = registro_de_prova!(register_argful_by_types(reg, proto, &["Uint64"], "BwmsRebase", "BwmsRebase", corpo_de_prova!(tramp_rebase)));
    // variante por PATH: setor não é alcançável como handle a partir do redscript.
    let svs = registro_de_prova!(register_argful_by_types(reg, proto, &["String", "Int32"], "BwmsSectorVtableSlot", "BwmsSectorVtableSlot", corpo_de_prova!(tramp_sector_vtable_slot)));
    // Materiais/aparências de um CMesh, tudo com offset vindo do RTTI.
    let mc = registro_de_prova!(register_argful_by_types(reg, proto, &["Uint64"], "BwmsMeshMaterialCount", "BwmsMeshMaterialCount", corpo_de_prova!(tramp_mesh_material_count)));
    let mi = registro_de_prova!(register_argful_by_types(reg, proto, &["Uint64", "Int32", "Int32"], "BwmsMeshMaterialInfo", "BwmsMeshMaterialInfo", corpo_de_prova!(tramp_mesh_material_info)));
    let ac = registro_de_prova!(register_argful_by_types(reg, proto, &["Uint64"], "BwmsMeshAppearanceCount", "BwmsMeshAppearanceCount", corpo_de_prova!(tramp_mesh_appearance_count)));
    let an = registro_de_prova!(register_argful_by_types(reg, proto, &["Uint64", "Int32"], "BwmsMeshAppearanceName", "BwmsMeshAppearanceName", corpo_de_prova!(tramp_mesh_appearance_name)));
    let cc = registro_de_prova!(register_argful_by_types(reg, proto, &["Uint64", "Int32"], "BwmsMeshAppearanceChunkCount", "BwmsMeshAppearanceChunkCount", corpo_de_prova!(tramp_mesh_appearance_chunk_count)));
    let tc = registro_de_prova!(register_argful_by_types(reg, proto, &["Uint64", "Int32"], "BwmsMeshAppearanceTagCount", "BwmsMeshAppearanceTagCount", corpo_de_prova!(tramp_mesh_appearance_tag_count)));
    let mp = registro_de_prova!(register_argful_by_types(reg, proto, &["Uint64", "Int32", "Int32"], "BwmsMeshMaterialParam", "BwmsMeshMaterialParam", corpo_de_prova!(tramp_mesh_material_param)));
    crate::log_dev!("[reg] register_all: BwmsVtableSlot={vs} BwmsRebase={rb} BwmsSectorVtableSlot={svs}");
    crate::log_dev!("[reg] register_all: setor-86 SectorNodeCount={a} SectorClass={b} SectorScan={c} SectorClassify={d} SectorStride={e} Count={f} NodeRef={h} NodeClassHash={i} Setup={j} SetupStride={k} SetupF32={sf} SetupU64={su} FirstComponentId={l} InkWorldWidgetComponent={w96} WidgetParent={wp} MeshRefScan={s30} PropOffset={po} PropU64={pu} MeshMaterialCount={mc} MeshMaterialInfo={mi} MeshAppearanceCount={ac} MeshAppearanceName={an} MeshAppearanceChunkCount={cc} MeshAppearanceTagCount={tc} MeshMaterialParam={mp}");
}



/// Igual a `vtable_slot_count`, mas mede UMA vez por vtable e reusa depois.
pub(crate) unsafe fn vtable_slot_count_cached(obj: *mut c_void) -> usize {
    if obj.is_null() || !crate::memoria::is_readable(obj, 8) {
        return 0;
    }
    let vt = (obj as *const u64).read_unaligned();
    if vt == 0 {
        return 0;
    }
    let cache = VTABLE_SLOTS_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Ok(m) = cache.lock() {
        if let Some(n) = m.get(&vt) {
            return *n;
        }
    }
    // Teto de 256: as vtables medidas neste binário têm dezenas de slots (MappinSystem=35), então
    // 256 é folga larga e evita varrer meio mapa de memória se o heurístico falhar.
    let n = vtable_slot_count(obj, 256);
    if let Ok(mut m) = cache.lock() {
        m.insert(vt, n);
    }
    n
}




pub(crate) type MappinGetPoiPhaseFn = unsafe extern "C" fn(*mut c_void, u32) -> u32;



pub(crate) unsafe fn register_mappinsystem_poi_phase_natives(reg: &Registry, proto: *mut c_void) {
    let params_set = compose_params_from_types(reg, &["handle:MappinSystem", "Uint32", "Int32"]);
    let ok_set = if params_compostos(params_set) {
        register_global_composed(
            reg,
            proto,
            params_set,
            "BwmsMappinSystemSetPoiPhase",
            "BwmsMappinSystemSetPoiPhase",
            tramp_mappinsystem_set_poi_phase,
        )
    } else {
        crate::log_dev!("[reg] register_mappinsystem_poi_phase_natives: compose_params_from_types falhou (handle:MappinSystem/Uint32/Int32)");
        false
    };
    let params_get = compose_params_from_types(reg, &["handle:MappinSystem", "Uint32"]);
    let ok_get = if params_compostos(params_get) {
        register_global_composed(
            reg,
            proto,
            params_get,
            "BwmsMappinSystemGetPoiPhase",
            "BwmsMappinSystemGetPoiPhase",
            tramp_mappinsystem_get_poi_phase,
        )
    } else {
        crate::log_dev!("[reg] register_mappinsystem_poi_phase_natives: compose_params_from_types falhou (handle:MappinSystem/Uint32)");
        false
    };
    crate::log_dev!(
        "[reg] register_all: mappinsystem-poi-phase BwmsMappinSystemSetPoiPhase={ok_set} BwmsMappinSystemGetPoiPhase={ok_get}"
    );
    let okslot = registro_de_prova!(register_argful_by_types(
        reg,
        proto,
        &["handle:MappinSystem", "Uint32", "Int32", "Uint32", "Int32", "Uint32", "Int32", "Int32", "Int32"],
        "BwmsMappinScanPoiSlot",
        "BwmsMappinScanPoiSlot", corpo_de_prova!(tramp_mappin_scan_poi_slot),
    ));
    crate::log_dev!(
        "[reg] register_all: poi-142 BwmsMappinScanPoiSlot={okslot} (acha o slot por comportamento, gabarito vanilla)"
    );
    let okver = registro_de_prova!(register_argful_by_types(reg, proto, &["String", "Int32"], "BwmsVeredito", "BwmsVeredito", corpo_de_prova!(tramp_veredito)));
    crate::log_dev!("[reg] register_all: BwmsVeredito={okver} (canal de veredito imune ao throttle)");
}


























/// Núcleo comum dos 3 trampolines abaixo: `sys` (`MappinSystem*`) -> `Handle`@`handle_off` ->
/// recurso cacheado -> `DynArray`@`array_off` -> tamanho. `-1` = recurso ainda não cacheado ou
/// ponteiro ilegível (sentinela, distinto de `0` = cacheado mas vazio) — testado offline em
/// `mappin_cooked_count_tests` abaixo.
pub(crate) fn mappin_cooked_count_from_lens(resource: Option<(u32, u32)>) -> i32 {
    match resource {
        Some((size, _capacity)) => size as i32,
        None => -1,
    }
}

pub(crate) unsafe fn mappin_cooked_count(sys: *mut c_void, handle_off: usize, array_off: usize) -> i32 {
    let lens = read_handle_instance(sys, handle_off).and_then(|res| read_dynarray_len(res, array_off));
    mappin_cooked_count_from_lens(lens)
}





// `FindNode` do registro de instâncias de nó do mundo (prova de 2026-09-12, RE offline do volume de mappin da parte 7d-2b-3, seção 3): `GameInstance+0x78` →
// `+0x230` (vptr `0x1078e4610`), `vt[0x198]` `0x101b191c8(registro, id)` com o Handle da instância por sret — a mesma chamada do `GetNodeTransform` vanilla,
// só na game thread (trava de leitura do registro). O Handle volta com referência e é solto pela sequência do motor. Usado pela instância de nó pelo id e
// pelo volume do nó de área (7d-2b-3b).
pub(crate) const GAMEINSTANCE_RUNTIME_OFF: usize = 0x78;

pub(crate) const RUNTIME_NODEREGISTRY_OFF: usize = 0x230;

pub(crate) const NODEREGISTRY_VPTR_VM: u64 = 0x1_078e_4610;

pub(crate) const NODEREGISTRY_FINDNODE: (usize, u64) = (0x198, 0x1_01b1_91c8);


/// Registro de instâncias (vptr conferido) e o endereço do `FindNode` (slot conferido contra a RE).
pub(crate) unsafe fn node_registry() -> Option<(*mut c_void, usize)> {
    let g = game_instance_ptr()?;
    if !crate::memoria::is_readable(g as *const c_void, GAMEINSTANCE_RUNTIME_OFF + 8) {
        return None;
    }
    let rt = rd_u64(g as *const c_void, GAMEINSTANCE_RUNTIME_OFF) as *mut c_void;
    if !rtti::sane(rt) || !crate::memoria::is_readable(rt as *const c_void, RUNTIME_NODEREGISTRY_OFF + 8) {
        return None;
    }
    let r = rd_u64(rt as *const c_void, RUNTIME_NODEREGISTRY_OFF) as *mut c_void;
    if !rtti::sane(r) || !crate::memoria::is_readable(r as *const c_void, 8) || rd_u64(r as *const c_void, 0) != crate::rebase(NODEREGISTRY_VPTR_VM) as u64 {
        return None;
    }
    // Mesmo conferidor de slot do loader de quest: endereço lido do vtable igual ao da RE.
    let f = questloader_slot(r, NODEREGISTRY_FINDNODE)?;
    Some((r, f))
}

// Instância de nó, parte 85b (2026-09-12, RE offline do layout da instância de nó, seção 4): campos do `worldInstancedMeshNode` num
// setor do cache do depot, só leitura, contra o CR2W do mesmo setor. O nó sai da lista `nodes` do setor (`SECTOR_NODES`) pelo vptr `0x1078e5130`; cada campo
// é lido no offset do registro RTTI desta build, conferido com o da RE. O cozido omite o campo igual ao padrão da classe, então o censo separa os valores
// iguais ao padrão de uma instância nova (pelo `CreateInstance`, AL-2c), que o CR2W não grava, dos diferentes, que são os gravados.
#[cfg(feature = "devtools")]
pub(crate) const INSTANCEDMESHNODE_TAMANHO: usize = 0xa0;

/// `(propriedade RTTI, offset na RE, largura em bytes, deslocamento dentro da propriedade)`, na ordem dos campos do `BwmsSectorInstancedMeshCensus`.
#[cfg(feature = "devtools")]
pub(crate) const INSTANCEDMESHNODE_CAMPOS: [(&str, usize, usize, usize); 10] = [
    ("castShadows", 0x97, 1, 0),
    ("castLocalShadows", 0x98, 1, 0),
    ("occluderType", 0x94, 1, 0),
    ("meshLODScales", 0x90, 4, 0),
    ("occluderAutohideDistanceScale", 0x95, 1, 0),
    ("version", 0x99, 1, 0),
    ("mesh", 0x80, 8, 0),
    ("meshAppearance", 0x88, 8, 0),
    ("worldTransformsBuffer", 0x38, 4, 0x10),
    ("worldTransformsBuffer", 0x38, 4, 0x14),
];

#[cfg(feature = "devtools")]
pub(crate) struct CensoInstanced {
    pub(crate) nos: u64,
    pub(crate) offsets_iguais: u64,
    pub(crate) padrao: Option<[u64; 10]>,
    pub(crate) valores: Vec<[u64; 10]>,
    pub(crate) castshadows_1b_antes: u64,
    pub(crate) mesh_1b_depois_xor: u64,
    /// Faixas `startIndex + numElements` ordenadas que não emendam na seguinte (no CR2W do setor de teste elas são contíguas).
    pub(crate) faixas_quebradas: u64,
}

/// Último censo, pelo endereço do setor.
#[cfg(feature = "devtools")]
pub(crate) static CENSO_INSTANCED: std::sync::Mutex<Option<(usize, CensoInstanced)>> = std::sync::Mutex::new(None);

pub(crate) unsafe fn campo_de_largura(obj: *const u8, off: usize, largura: usize) -> u64 {
    match largura {
        1 => *obj.add(off) as u64,
        4 => (obj.add(off) as *const u32).read_unaligned() as u64,
        _ => (obj.add(off) as *const u64).read_unaligned(),
    }
}

/// Censo dos `worldInstancedMeshNode` do setor; `Err` -1 sem registro, -2 `nodes` incoerente.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn censo_instanced(sec: *mut c_void) -> Result<CensoInstanced, i32> {
    let reg = crate::registry().ok_or(-1)?;
    let cls = reg.class_by_name("worldInstancedMeshNode");
    let mut offs = [0usize; 10];
    let mut iguais = 0u64;
    for (i, (nome, off_re, largura, dentro)) in INSTANCEDMESHNODE_CAMPOS.iter().enumerate() {
        let prop = if rtti::sane(cls) { rtti::find_property_in_class(cls, nome) } else { std::ptr::null_mut() };
        let off_rtti = if rtti::sane(prop) { Some(rd_u32(prop as *const c_void, 0x20) as usize) } else { None };
        iguais += (off_rtti == Some(*off_re)) as u64;
        offs[i] = off_rtti.filter(|o| o + dentro + largura <= INSTANCEDMESHNODE_TAMANHO).unwrap_or(*off_re) + dentro;
    }
    if !crate::memoria::is_readable(sec as *const c_void, SECTOR_NODES + 0x10) {
        return Err(-2);
    }
    let (entries, size, cap) = dynarray_em((sec as usize + SECTOR_NODES) as *const c_void);
    if entries == 0 || !dynarray_coerente(entries, size, cap) || !crate::memoria::is_readable(entries as *const c_void, size as usize * 16) {
        return Err(-2);
    }
    let le = |obj: *const u8| {
        let mut v = [0u64; 10];
        for (i, (_, _, largura, _)) in INSTANCEDMESHNODE_CAMPOS.iter().enumerate() {
            v[i] = campo_de_largura(obj, offs[i], *largura);
        }
        v
    };
    let vptr = crate::rebase(crate::jogo::mundo::NO_MALHA_INSTANCIADA_VPTR_VM) as u64;
    let mut c = CensoInstanced {
        nos: size as u64,
        offsets_iguais: iguais,
        padrao: None,
        valores: Vec::new(),
        castshadows_1b_antes: 0,
        mesh_1b_depois_xor: 0,
        faixas_quebradas: 0,
    };
    for i in 0..size as usize {
        let no = rd_u64(entries as *const c_void, i * 16) as *const u8;
        if !rtti::sane(no as *mut c_void) || !crate::memoria::is_readable(no as *const c_void, INSTANCEDMESHNODE_TAMANHO) || (no as *const u64).read_unaligned() != vptr {
            continue;
        }
        c.valores.push(le(no));
        c.castshadows_1b_antes += (*no.add(0x96) == 1) as u64;
        c.mesh_1b_depois_xor ^= (no.add(0x81) as *const u64).read_unaligned();
    }
    let mut faixas: Vec<(u64, u64)> = c.valores.iter().map(|v| (v[8], v[9])).collect();
    faixas.sort_unstable();
    let quebras: Vec<String> = faixas.windows(2).filter(|w| w[0].0 + w[0].1 != w[1].0).map(|w| format!("{}+{}→{}", w[0].0, w[0].1, w[1].0)).collect();
    c.faixas_quebradas = quebras.len() as u64;
    crate::veredito_dev!("[cw85-setor] faixas startIndex+numElements ordenadas: {} quebra(s) {:?}", quebras.len(), &quebras[..quebras.len().min(8)]);
    // Boot 82: sem quebras e com a mesma contagem, a soma dos inícios ainda veio 3 abaixo do CR2W — tuplas por nó pra comparar com o arquivo offline.
    let mut por_inicio: Vec<[u64; 4]> = c.valores.iter().map(|v| [v[8], v[9], v[6], v[7]]).collect();
    por_inicio.sort_unstable();
    for (k, bloco) in por_inicio.chunks(24).enumerate() {
        let texto: Vec<String> = bloco.iter().map(|t| format!("{}+{}:{:x}:{:x}", t[0], t[1], t[2], t[3])).collect();
        crate::veredito_dev!("[cw85-setor] nós por início, bloco {k}: {}", texto.join(" "));
    }
    c.padrao = {
        let obj = rtti::new_object(reg, "worldInstancedMeshNode") as *const u8;
        if obj.is_null() || !crate::memoria::is_readable(obj as *const c_void, INSTANCEDMESHNODE_TAMANHO) {
            None
        } else {
            let v = le(obj);
            let mut dono = [0u64; 2];
            rtti::make_handle(dono.as_mut_ptr() as *mut c_void, obj as *mut c_void);
            let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono);
            Some(v)
        }
    };
    crate::veredito_dev!(
        "[cw85-setor] setor {sec:p}: {size} nós, {} worldInstancedMeshNode, offsets RTTI iguais aos da RE {iguais}/10, padrão da instância nova {:?}",
        c.valores.len(),
        c.padrao
    );
    Ok(c)
}

/// Valor `what` do censo (ver `BwmsSectorInstancedMeshCensus`); `u64::MAX` sem padrão ou código desconhecido.
#[cfg(feature = "devtools")]
pub(crate) fn censo_instanced_valor(c: &CensoInstanced, what: i32) -> u64 {
    match what {
        0 => c.valores.len() as u64,
        1 => c.nos,
        2 => c.offsets_iguais,
        3 => c.padrao.is_some() as u64,
        900 => c.castshadows_1b_antes,
        901 => c.mesh_1b_depois_xor,
        902 => c.faixas_quebradas,
        w if (1000..11000).contains(&w) => {
            let f = (w / 1000 - 1) as usize;
            let Some(p) = c.padrao else { return u64::MAX };
            let dif: Vec<u64> = c.valores.iter().map(|v| v[f]).filter(|x| *x != p[f]).collect();
            let combinado = if f == 6 || f == 7 { dif.iter().fold(0u64, |a, x| a ^ *x) } else { dif.iter().fold(0u64, |a, x| a.wrapping_add(*x)) };
            match w % 1000 {
                0 => p[f],
                1 => c.valores.iter().filter(|v| v[f] == p[f]).count() as u64,
                2 => combinado & 0xffff_ffff,
                3 => dif.len() as u64,
                4 => combinado >> 32,
                999 => c.valores.iter().filter(|v| v[f] == 0xffff_0000).count() as u64,
                m if m >= 10 => c.valores.iter().filter(|v| v[f] == (m - 10) as u64).count() as u64,
                _ => u64::MAX,
            }
        }
        _ => u64::MAX,
    }
}

/// `BwmsSectorInstancedMeshCensus(path, what) -> Uint64`: censo dos `worldInstancedMeshNode` do setor no cache do depot — 0 nós da classe (vptr), 1 nós do
/// setor, 2 offsets RTTI iguais aos da RE (de 10), 3 padrão lido de uma instância nova (1/0); por campo f (0 castShadows, 1 castLocalShadows, 2 occluderType,
/// 3 meshLODScales, 4 occluderAutohideDistanceScale, 5 version, 6 mesh, 7 meshAppearance, 8 startIndex e 9 numElements do `worldTransformsBuffer`), o código
/// `1000·(f+1) + m`: m 0 padrão, 1 nós com o padrão, 2/4 32 bits baixos/altos da soma dos valores diferentes do padrão (XOR em mesh e meshAppearance), 3 nós
/// diferentes do padrão, 10+v nós com o valor v, 999 nós com `0xffff0000`; controles 900 nós com o byte antes do castShadows igual a 1 e 901 XOR do u64 1 B
/// depois do mesh. `u64::MAX` sem padrão ou código desconhecido, `u64::MAX - 1` setor fora do cache, `u64::MAX - 2` `nodes` incoerente ou sem registro.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_sector_instanced_mesh_census(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let what = a.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let sec = setor_por_path(&path);
    let v = if sec.is_null() {
        u64::MAX - 1
    } else if let Ok(mut cache) = CENSO_INSTANCED.lock() {
        if cache.as_ref().map_or(true, |(s, _)| *s != sec as usize) {
            *cache = censo_instanced(sec).ok().map(|c| (sec as usize, c));
        }
        cache.as_ref().map_or(u64::MAX - 2, |(_, c)| censo_instanced_valor(c, what))
    } else {
        u64::MAX - 2
    };
    write_uint_ret(out, v, 8);
}

/// Nó `indice` da lista `nodes` do setor no cache do depot.
pub(crate) unsafe fn setor_no(path: &str, indice: i32) -> Option<*const u8> {
    let sec = setor_por_path(path);
    if sec.is_null() || !crate::memoria::is_readable(sec as *const c_void, SECTOR_NODES + 0x10) {
        return None;
    }
    let (entries, size, cap) = dynarray_em((sec as usize + SECTOR_NODES) as *const c_void);
    let i = usize::try_from(indice).ok()?;
    if entries == 0 || !dynarray_coerente(entries, size, cap) || i >= size as usize || !crate::memoria::is_readable((entries + i * 16) as *const c_void, 16) {
        return None;
    }
    let no = rd_u64(entries as *const c_void, i * 16) as *const u8;
    (rtti::sane(no as *mut c_void) && crate::memoria::is_readable(no as *const c_void, 0x38)).then_some(no)
}

/// Tamanho da instância da classe do nó, com o nó legível até lá; `None` sem classe.
pub(crate) unsafe fn setor_no_tamanho(no: *const u8) -> Option<usize> {
    let cls = rtti::class_of(no as *mut c_void);
    if !rtti::sane(cls) {
        return None;
    }
    let tamanho = rd_u32(cls as *const c_void, CCLASS_TAMANHO_OFF) as usize;
    (tamanho >= 0x38 && crate::memoria::is_readable(no as *const c_void, tamanho)).then_some(tamanho)
}

/// Campo `nome` do nó pelo offset da propriedade RTTI (+ `dentro`), só se couber no tamanho da classe.
pub(crate) unsafe fn setor_no_campo(no: *const u8, nome: &str, dentro: i32, largura: usize) -> Option<u64> {
    let tamanho = setor_no_tamanho(no)?;
    let prop = rtti::find_property_in_class(rtti::class_of(no as *mut c_void), nome);
    if !rtti::sane(prop) {
        return None;
    }
    let off = rd_u32(prop as *const c_void, 0x20) as i32 + dentro;
    ler_largura_ate(no, off, largura, tamanho)
}

/// Largura em bytes pedida pelo reds: 1, 4 ou 8.
pub(crate) fn largura_valida(w: i32) -> Option<usize> {
    matches!(w, 1 | 4 | 8).then_some(w as usize)
}

/// Tipo do header mais próximo na hierarquia de `cls` (a própria classe, depois os pais): classe derivada de um dos 5 tipos usa o offset dele.
pub(crate) unsafe fn nodeinstance_tipo(mut cls: *mut c_void) -> Option<usize> {
    for _ in 0..16 {
        if !rtti::sane(cls) || !crate::memoria::is_readable(cls as *const c_void, 0x20) {
            return None;
        }
        let nome = rtti::type_name_hash(cls);
        if let Some(k) = NODEINSTANCE_CARGAS.iter().position(|(c, _, _)| crate::cname::cname(c) == nome) {
            return Some(k);
        }
        cls = rd_u64(cls as *const c_void, 0x10) as *mut c_void;
    }
    None
}

/// Instância de nó lida pela memória.
pub(crate) struct NodeInstanceLida {
    pub(crate) obj: *mut c_void,
    pub(crate) classe: *mut c_void,
    pub(crate) nome: u64,
    pub(crate) estado: u16,
    /// Índice em `NODEINSTANCE_CARGAS` pela hierarquia da classe.
    pub(crate) tipo: Option<usize>,
    /// Classe do objeto do Handle no offset lido; nula sem Handle de `ISerializable` ali.
    pub(crate) carga: *mut c_void,
}

/// Objeto do Handle guardado em `slot`, só se for `ISerializable` com `GetType` (vt+0x08) na imagem. `class_of` é chamada virtual: num objeto C++ comum
/// o vt+0x08 é o destrutor com delete, e a sonda do boot 134 abortou o jogo assim (`free` de ponteiro que não veio do malloc).
pub(crate) unsafe fn handle_serializavel(slot: *const c_void) -> Option<*mut c_void> {
    if !crate::memoria::is_readable(slot, 16) {
        return None;
    }
    let (p, contagem) = (rd_u64(slot, 0), rd_u64(slot, 8));
    let obj = p as *mut c_void;
    if !rtti::sane(obj) || !crate::memoria::is_readable(obj as *const c_void, 0x18) {
        return None;
    }
    if !auto_referencia_confere(p, contagem, rd_u64(obj as *const c_void, 0x08), rd_u64(obj as *const c_void, 0x10)) {
        return None;
    }
    let vt = rd_u64(obj as *const c_void, 0);
    let get_type = if vt != 0 && crate::memoria::is_readable(vt as *const c_void, 16) { rd_u64(vt as *const c_void, 8) } else { 0 };
    (get_type != 0 && (0x1_0000_0000..0x1_0A00_0000).contains(&(crate::un_rebase(get_type as *mut c_void) as u64))).then_some(obj)
}

/// Em cada offset de 0x90 a 0x128 da instância em `obj` (dentro do tamanho da classe), se há um Handle cujo objeto é de cada uma das 3 bases.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn node_instance_sondar(obj: *mut c_void, bases: [*mut c_void; 3]) -> Option<[[bool; 3]; SLOTS_SONDADOS]> {
    if !rtti::sane(obj) {
        return None;
    }
    let cls = rtti::class_of(obj);
    let tamanho = if rtti::sane(cls) { rtti::type_size(cls) as usize } else { 0 };
    if tamanho < NODEINSTANCE_LEITURA_FIM || !crate::memoria::is_readable(obj as *const c_void, tamanho.min(0x90 + SLOTS_SONDADOS * 8)) {
        return None;
    }
    let mut achados = [[false; 3]; SLOTS_SONDADOS];
    for (i, por_base) in achados.iter_mut().enumerate() {
        let off = 0x90 + i * 8;
        if off + 16 > tamanho {
            break;
        }
        let Some(p) = handle_serializavel((obj as *const u8).add(off) as *const c_void) else {
            continue;
        };
        let classe = rtti::class_of(p);
        for (j, base) in bases.iter().enumerate() {
            por_base[j] = rtti::sane(classe) && !base.is_null() && class_is_kind_of(classe, *base);
        }
    }
    Some(achados)
}

/// Por que `node_instance_objeto` não devolveu nada: 0 sem registro, 1 id sem instância no registro, 2 instância ou classe inválida, 3 classe menor que a
/// leitura ou ilegível.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn node_instance_motivo(id: u64) -> usize {
    let Some(mut h) = find_node(id) else { return 0 };
    let obj = h[0] as *mut c_void;
    let motivo = if obj.is_null() {
        1
    } else if !rtti::sane(obj) || !rtti::sane(rtti::class_of(obj)) {
        2
    } else {
        3
    };
    let _ = crate::jogo::objetos::soltar_handle_contando(&mut h);
    motivo
}

#[cfg(feature = "devtools")]
pub(crate) unsafe fn varrer_instancias_do_setor(reg: &rtti::Registry, path: &str) -> (String, bool) {
    let setor = setor_por_path(path);
    if sector_setups(setor).is_none() {
        return ("setor fora do cache".into(), false);
    }
    varrer_instancias_dos_setores(reg, &[setor], u32::MAX)
}

/// Instâncias de uma classe e, nas primeiras `AMOSTRA_POR_CLASSE`, quantas têm em cada offset sondado um Handle cujo objeto é de cada base de
/// `BASES_SONDADAS`; a classe (pros pais no resumo) e o tipo do header pela hierarquia.
#[derive(Default)]
#[cfg(feature = "devtools")]
pub(crate) struct InstanciasDaClasse {
    pub(crate) n: u32,
    pub(crate) hits: [[u32; 3]; SLOTS_SONDADOS],
    pub(crate) classe: usize,
    pub(crate) tipo: Option<usize>,
}

#[cfg(feature = "devtools")]
pub(crate) const AMOSTRA_POR_CLASSE: u32 = 40;

/// Offsets sondados: 0x90 + 8·i.
#[cfg(feature = "devtools")]
pub(crate) const SLOTS_SONDADOS: usize = 20;

#[cfg(feature = "devtools")]
pub(crate) const BASES_SONDADAS: [&str; 3] = ["CMesh", "IMaterial", "entEntity"];

/// Nomes dos pais da classe, até 4 níveis.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn pais_da_classe(mut cls: usize) -> String {
    let mut nomes = Vec::new();
    for _ in 0..4 {
        if cls == 0 || !crate::memoria::is_readable(cls as *const c_void, 0x20) {
            break;
        }
        cls = rd_u64(cls as *const c_void, 0x10) as usize;
        if cls == 0 || !crate::memoria::is_readable(cls as *const c_void, 0x20) {
            break;
        }
        nomes.push(crate::cname::resolve_cname(rtti::type_name_hash(cls as *mut c_void)));
    }
    nomes.join("<")
}

/// Teste do `#237`: pra cada setup dos `setores` com GlobalNodeID (até `teto_setups`), a instância pela busca do motor; por tipo do header conta instâncias,
/// as que carregam objeto da classe base esperada, as inicializadas, anexadas e visíveis, e anexada sem o bit de inicializada (incoerência do bitfield).
/// Controle negativo: o objeto lido no offset de OUTRO tipo (mesh em 0xA0, entidade em 0x98) não pode ser da classe esperada. Devolve o resumo e se passou.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn varrer_instancias_dos_setores(reg: &rtti::Registry, setores: &[*mut c_void], teto_setups: u32) -> (String, bool) {
    let bases: Vec<*mut c_void> = NODEINSTANCE_CARGAS.iter().map(|(_, _, b)| reg.class_by_name(b)).collect();
    let mut conta = C237Conta::default();
    for &setor in setores {
        if conta.setups >= teto_setups {
            break;
        }
        let Some((entries, size)) = sector_setups(setor) else { continue };
        conta.varridos += 1;
        conta.setups = conta.setups.saturating_add(size);
        for i in 0..size as usize {
            let id = rd_u64((entries + i * SETUP_SIZE + NODESETUP_GLOBALID_OFF) as *const c_void, 0);
            if id != 0 {
                c237_contar(&bases, id, &mut conta);
            }
        }
    }
    c237_resumo(&conta)
}

/// `BwmsSectorNodeField(path, index, name, inner, width) -> Uint64`: campo `name` do nó `index` do setor pelo offset do registro RTTI desta build, mais
/// `inner` (campo dentro de uma struct), com `width` 1, 4 ou 8 bytes; 0 sem nó, sem propriedade ou fora do tamanho da classe.
pub(crate) unsafe extern "C" fn tramp_sector_node_field(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let nome = strings.get(2).cloned().flatten().unwrap_or_default();
    let indice = a.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let dentro = a.get(3).map(|(v, _)| *v as u32 as i32).unwrap_or(0);
    let largura = a.get(4).and_then(|(v, _)| largura_valida(*v as u32 as i32));
    let v = match (setor_no(&path, indice), largura) {
        (Some(no), Some(w)) if !nome.is_empty() => setor_no_campo(no, &nome, dentro, w).unwrap_or(0),
        _ => 0,
    };
    write_uint_ret(out, v, 8);
}

/// `BwmsSectorNodeArraySize(path, index, offset) -> Int32`: tamanho do `DynArray` em `offset` do nó `index` do setor (os arrays de runtime do
/// `worldInstancedMeshNode` ficam em `0x50`, `0x60` e `0x70`, fora do RTTI); -1 sem nó, fora do tamanho da classe ou array incoerente.
pub(crate) unsafe extern "C" fn tramp_sector_node_array_size(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let indice = a.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let off = a.get(2).and_then(|(v, _)| usize::try_from(*v as u32 as i32).ok());
    let n = match (setor_no(&path, indice), off) {
        (Some(no), Some(off)) => match setor_no_tamanho(no) {
            Some(tamanho) if off + 0x10 <= tamanho => {
                let (entries, size, cap) = dynarray_em((no as usize + off) as *const c_void);
                if dynarray_coerente(entries, size, cap) { size as i32 } else { -1 }
            }
            _ => -1,
        },
        _ => -1,
    };
    write_uint_ret(out, n as u32 as u64, 4);
}

/// Endereço de `largura` bytes em `dentro` do elemento `elemento` (passo `passo`) do `DynArray` em `off` do nó `indice` do setor; `None` fora do array, do
/// tamanho da classe, do passo ou com array incoerente.
pub(crate) unsafe fn setor_no_elemento(path: &str, indice: i32, off: i32, elemento: i32, passo: i32, dentro: i32, largura: usize) -> Option<*const u8> {
    let no = setor_no(path, indice)?;
    let tamanho = setor_no_tamanho(no)?;
    let (off, elemento, passo, dentro) = (usize::try_from(off).ok()?, usize::try_from(elemento).ok()?, usize::try_from(passo).ok()?, usize::try_from(dentro).ok()?);
    if off + 0x10 > tamanho || passo == 0 || dentro + largura > passo {
        return None;
    }
    let (entries, size, cap) = dynarray_em((no as usize + off) as *const c_void);
    if !dynarray_coerente(entries, size, cap) || elemento >= size as usize {
        return None;
    }
    let p = (entries + elemento * passo + dentro) as *const u8;
    crate::memoria::is_readable(p as *const c_void, largura).then_some(p)
}

/// `BwmsSectorNodeArrayF32(path, index, offset, element, stride, inner) -> Float`: f32 em `inner` do elemento `element` (passo `stride`) do `DynArray` em
/// `offset` do nó `index` do setor (85c: caixas em `0x50` com passo 32, distâncias em `0x60` com passo 4); NaN fora do array.
pub(crate) unsafe extern "C" fn tramp_sector_node_array_f32(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let arg = |i: usize| a.get(i).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let v = setor_no_elemento(&path, arg(1), arg(2), arg(3), arg(4), arg(5), 4).map_or(f32::NAN, |p| (p as *const f32).read_unaligned());
    write_uint_ret(out, v.to_bits() as u64, 4);
}

/// `BwmsSectorNodeArrayU32(path, index, offset, element, stride, inner) -> Uint32`: u32 em `inner` do elemento `element` do `DynArray` em `offset` do nó
/// `index` do setor (85c: lotes de instância em `0x70` com passo 4); `0xFFFFFFFF` fora do array.
pub(crate) unsafe extern "C" fn tramp_sector_node_array_u32(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let arg = |i: usize| a.get(i).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let v = setor_no_elemento(&path, arg(1), arg(2), arg(3), arg(4), arg(5), 4).map_or(u32::MAX, |p| (p as *const u32).read_unaligned());
    write_uint_ret(out, v as u64, 4);
}

// ---------------------------------------------------------------------------------------------
// `WeatherSystem` (sessão 2026-08-12, 100% offline) — leitura pura de campo, zero endereço de função:
//
//   WeatherSystem+0x40        = worldRuntimeSystemWeather*
//   RuntimeSystemWeather+0x8C = CycleWeather (bool)
//   RuntimeSystemWeather+0x90 = CurrentStateIndex (u32)
//   RuntimeSystemWeather+0x98 = CurrentSource (CName)
//   RuntimeSystemWeather+0xB8 = PreviousStateIndex (u32)
//
// SetWeatherByName/SetWeatherByIndex/GetEnvironmentDefinition/GetWeatherState são funções do motor:
// os endereços Mac saíram de desmonte offline, e os 4 métodos de script (SetWeather/ResetWeather/
// GetEnvironmentDefinition/GetWeatherState) estão mais abaixo (`BwmsWeatherSystemSetWeather` e seguintes).
//
// O ponteiro do sistema e os 4 campos são TODOS
// campo de dado cru (não função/vtable) — dois ponteiros encadeados, zero
// chamada de função em qualquer etapa (mais seguro até que o padrão de vtable-dispatch já usado
// pra `MappinSystem`/`#142` acima, que ao menos CHAMA um function pointer; aqui só lemos bytes).
// `WeatherSystem` confirmado vanilla real e redscript-visível (`orphans.script:43653`,
// `importonly class WeatherSystem extends IScriptable`; getter `GameInstance.GetWeatherSystem`
// já existe, `orphans.script:11547`) — validado por scratch-compile (`@addMethod(WeatherSystem)`
// resolve limpo contra o bundle real, zero UNRESOLVED_REF).
//
// Capacidade nova exposta (fora do escopo original dos 4 métodos, mas prática): ler
// o estado de clima ATUAL sem precisar de nenhuma das 4 funções — índice do estado de clima
// corrente/fonte(CName)/índice do estado anterior. Suficiente pra um mod CONSULTAR o clima
// (ex.: "está chovendo?"/"qual clima tocou antes?"). TROCAR passa pelas funções do motor
// (`BwmsWeatherSystemSetWeather`/`ResetWeather`, abaixo).
pub(crate) const WEATHER_RUNTIME_PTR_OFF: usize = 0x40; // worldRuntimeSystemWeather* (dentro de WeatherSystem)

/// Núcleo PURO (testável offline, sem memória real): decodifica os 3 campos de estado de clima
/// a partir de um buffer que espelha os primeiros `WEATHER_RUNTIME_CHUNK_LEN` bytes do
/// `worldRuntimeSystemWeather*` real, nos offsets acima.
pub(crate) fn decode_weather_runtime_state(buf: &[u8]) -> Option<(u32, u64, u32)> {
    if buf.len() < WEATHER_RUNTIME_CHUNK_LEN {
        return None;
    }
    let idx = u32::from_le_bytes(buf[WEATHER_CURRENT_STATE_INDEX_OFF..WEATHER_CURRENT_STATE_INDEX_OFF + 4].try_into().ok()?);
    let src = u64::from_le_bytes(buf[WEATHER_CURRENT_SOURCE_OFF..WEATHER_CURRENT_SOURCE_OFF + 8].try_into().ok()?);
    let prev = u32::from_le_bytes(buf[WEATHER_PREVIOUS_STATE_INDEX_OFF..WEATHER_PREVIOUS_STATE_INDEX_OFF + 4].try_into().ok()?);
    Some((idx, src, prev))
}

/// Segue `sys(WeatherSystem*) -> +0x40 -> worldRuntimeSystemWeather*` e lê os 3 campos via
/// `ganchos::read_chunk` (1 syscall só) + `decode_weather_runtime_state` (núcleo puro acima).
/// `None` em qualquer falha de leitura/ponteiro nulo — nunca deferencia sem checar.
pub(crate) unsafe fn weathersystem_read_state(sys: *mut c_void) -> Option<(u32, u64, u32)> {
    if sys.is_null() || !rtti::sane(sys) || !crate::memoria::is_readable(sys as *const c_void, WEATHER_RUNTIME_PTR_OFF + 8) {
        return None;
    }
    let rt_ptr = ((sys as *const u8).add(WEATHER_RUNTIME_PTR_OFF) as *const u64).read_unaligned() as usize;
    if rt_ptr == 0 {
        return None;
    }
    let mut buf = [0u8; WEATHER_RUNTIME_CHUNK_LEN];
    if !crate::memoria::read_chunk(rt_ptr, &mut buf) {
        return None;
    }
    decode_weather_runtime_state(&buf)
}

/// `BwmsWeatherSystemGetCurrentStateIndex(sys: ref<WeatherSystem>) -> Uint32`
pub(crate) unsafe extern "C" fn tramp_weathersystem_current_state_index(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let val = weathersystem_read_state(sys).map(|(idx, _, _)| idx).unwrap_or(0);
    crate::log_dev!("[weathersystem] BwmsWeatherSystemGetCurrentStateIndex(sys={sys:p}) -> {val}");
    write_uint_ret(out, val as u64, 4);
}

/// `BwmsWeatherSystemGetCurrentSource(sys: ref<WeatherSystem>) -> CName`
pub(crate) unsafe extern "C" fn tramp_weathersystem_current_source(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let val = weathersystem_read_state(sys).map(|(_, src, _)| src).unwrap_or(0);
    crate::log_dev!("[weathersystem] BwmsWeatherSystemGetCurrentSource(sys={sys:p}) -> {val:#x}");
    write_uint_ret(out, val, 8);
}

/// `BwmsWeatherSystemGetPreviousStateIndex(sys: ref<WeatherSystem>) -> Uint32`
pub(crate) unsafe extern "C" fn tramp_weathersystem_previous_state_index(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let val = weathersystem_read_state(sys).map(|(_, _, prev)| prev).unwrap_or(0);
    crate::log_dev!("[weathersystem] BwmsWeatherSystemGetPreviousStateIndex(sys={sys:p}) -> {val}");
    write_uint_ret(out, val as u64, 4);
}

pub(crate) unsafe fn register_weathersystem_state_natives(reg: &Registry, proto: *mut c_void) {
    let p1 = compose_params_from_types(reg, &["handle:WeatherSystem"]);
    let ok1 = if params_compostos(p1) {
        register_global_composed(reg, proto, p1, "BwmsWeatherSystemGetCurrentStateIndex", "BwmsWeatherSystemGetCurrentStateIndex", tramp_weathersystem_current_state_index)
    } else {
        crate::log_dev!("[reg] register_weathersystem_state_natives: compose_params_from_types falhou (handle:WeatherSystem) [1/3]");
        false
    };
    let p2 = compose_params_from_types(reg, &["handle:WeatherSystem"]);
    let ok2 = if params_compostos(p2) {
        register_global_composed(reg, proto, p2, "BwmsWeatherSystemGetCurrentSource", "BwmsWeatherSystemGetCurrentSource", tramp_weathersystem_current_source)
    } else {
        crate::log_dev!("[reg] register_weathersystem_state_natives: compose_params_from_types falhou (handle:WeatherSystem) [2/3]");
        false
    };
    let p3 = compose_params_from_types(reg, &["handle:WeatherSystem"]);
    let ok3 = if params_compostos(p3) {
        register_global_composed(reg, proto, p3, "BwmsWeatherSystemGetPreviousStateIndex", "BwmsWeatherSystemGetPreviousStateIndex", tramp_weathersystem_previous_state_index)
    } else {
        crate::log_dev!("[reg] register_weathersystem_state_natives: compose_params_from_types falhou (handle:WeatherSystem) [3/3]");
        false
    };
    crate::log_dev!(
        "[reg] register_all: weathersystem-state BwmsWeatherSystemGetCurrentStateIndex={ok1} BwmsWeatherSystemGetCurrentSource={ok2} BwmsWeatherSystemGetPreviousStateIndex={ok3}"
    );
}

// ---------------------------------------------------------------------------------------------
// `WeatherSystem.SetWeather`/`ResetWeather` (2026-09-11). Endereços achados offline:
// `SetWeatherByName` = `0x101a730fc` (x0 sistema, x1 clima, x2 fonte, s0 blend, w3, w4 prioridade; lê
// `CurrentSource@0x98`, procura o estado pelo nome e chama o `SetWeatherByIndex` = `0x101a70298`), e
// `0x101a719ec` = `strb w1, [x0, #0x8c]` (o `CycleWeather`). O motor já tem o `SetWeather`
// pronto em `0x102921138`: ByName(sistema, clima, fonte, blend, 0, prioridade) e, se deu certo,
// ciclo = false. O nó de quest `0x102921180` usa a mesma sequência e, no ramo de reset, ciclo = true.
//
// O `ResetWeather` chama o `SetWeatherByIndex` com 0 em tudo. No Mac o 5º e o 6º argumentos saem de
// x3/x4 (o ByName repassa assim); o 5º também pode ser lido como float. A assinatura abaixo zera s1 E
// x3/x4, então as duas leituras recebem 0.
pub(crate) const WEATHER_SETWEATHER_HELPER_VM: u64 = 0x1_0292_1138;

pub(crate) const WEATHER_SETBYINDEX_VM: u64 = 0x1_01a7_0298;

pub(crate) const WEATHER_SETCYCLE_VM: u64 = 0x1_01a7_19ec;

/// `WeatherSystem*` (objeto de script) -> `worldRuntimeSystemWeather*` em `+0x40`; nulo se ilegível.
pub(crate) unsafe fn weathersystem_runtime(sys: *mut c_void) -> *mut c_void {
    if sys.is_null() || !rtti::sane(sys) || !crate::memoria::is_readable(sys as *const c_void, WEATHER_RUNTIME_PTR_OFF + 8) {
        return std::ptr::null_mut();
    }
    ((sys as *const u8).add(WEATHER_RUNTIME_PTR_OFF) as *const u64).read_unaligned() as *mut c_void
}

/// `BwmsWeatherSystemSetWeather(sys: ref<WeatherSystem>, weather: CName, blendTime: Float, priority: Int32) -> Bool`
pub(crate) unsafe extern "C" fn tramp_weathersystem_set_weather(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let clima = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let blend = f32::from_bits(args.get(2).map(|(v, _)| *v as u32).unwrap_or(0));
    let prioridade = args.get(3).map(|(v, _)| *v as u32).unwrap_or(0);
    let rt = weathersystem_runtime(sys);
    let mut ok = false;
    if !rt.is_null() && clima != 0 {
        let set_weather: extern "C" fn(*mut c_void, u64, u64, f32, u64) -> bool =
            std::mem::transmute(crate::rebase(WEATHER_SETWEATHER_HELPER_VM));
        ok = set_weather(rt, clima, 0, blend, prioridade as u64);
    }
    crate::log_dev!(
        "[weathersystem] BwmsWeatherSystemSetWeather(clima={clima:#x}, blend={blend}, prioridade={prioridade}) rt={rt:p} -> {ok}"
    );
    write_bool_ret(out, ok);
}

/// `BwmsWeatherSystemResetWeather(sys: ref<WeatherSystem>, forceRestore: Bool, blendTime: Float) -> Bool`
pub(crate) unsafe extern "C" fn tramp_weathersystem_reset_weather(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let forcar = args.get(1).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let blend = f32::from_bits(args.get(2).map(|(v, _)| *v as u32).unwrap_or(0));
    let rt = weathersystem_runtime(sys);
    let mut ok = false;
    if !rt.is_null() {
        let set_cycle: extern "C" fn(*mut c_void, u32) = std::mem::transmute(crate::rebase(WEATHER_SETCYCLE_VM));
        set_cycle(rt, 1);
        ok = true;
        if forcar {
            let by_index: extern "C" fn(*mut c_void, u32, u64, f32, f32, u64, u64) -> bool =
                std::mem::transmute(crate::rebase(WEATHER_SETBYINDEX_VM));
            ok = by_index(rt, 0, 0, blend, 0.0, 0, 0);
        }
    }
    crate::log_dev!(
        "[weathersystem] BwmsWeatherSystemResetWeather(forcar={forcar}, blend={blend}) rt={rt:p} -> {ok}"
    );
    write_bool_ret(out, ok);
}

pub(crate) unsafe fn register_weathersystem_set_natives(reg: &Registry, proto: *mut c_void) {
    let p_set = compose_params_from_types(reg, &["handle:WeatherSystem", "CName", "Float", "Int32"]);
    let ok_set = if params_compostos(p_set) {
        register_global_composed(reg, proto, p_set, "BwmsWeatherSystemSetWeather", "BwmsWeatherSystemSetWeather", tramp_weathersystem_set_weather as NativeHandler)
    } else {
        crate::log_dev!("[reg] register_weathersystem_set_natives: compose_params_from_types falhou (SetWeather)");
        false
    };
    let p_reset = compose_params_from_types(reg, &["handle:WeatherSystem", "Bool", "Float"]);
    let ok_reset = if params_compostos(p_reset) {
        register_global_composed(reg, proto, p_reset, "BwmsWeatherSystemResetWeather", "BwmsWeatherSystemResetWeather", tramp_weathersystem_reset_weather as NativeHandler)
    } else {
        crate::log_dev!("[reg] register_weathersystem_set_natives: compose_params_from_types falhou (ResetWeather)");
        false
    };
    crate::log_dev!(
        "[reg] register_all: weathersystem-set BwmsWeatherSystemSetWeather={ok_set} BwmsWeatherSystemResetWeather={ok_reset}"
    );
}

// ---------------------------------------------------------------------------------------------
// `GetEnvironmentDefinition`/`GetWeatherState` (2026-09-14). Desmonte: `0x101a71abc` pega a definição pelo token em `[sys+0x60]`
// (`0x101b695cc`) e devolve o `Handle` por `x8` com +1 forte; `0x101a71548` escolhe o índice (`[sys+0x88]` com o byte `[sys+0x84]` ligado, senão o
// `CurrentStateIndex` `[sys+0x90]`) e salta pro `0x101a72ccc`, que devolve por `x8` o `Handle` da entrada do índice em `[def+0x188]` (16 B, tamanho em
// `+0x194`). Os 2 objetos não derivam de `IScriptable`: sobem pro script dentro de um `BwmsSerializable`.
pub(crate) const WEATHER_GET_ENVDEF_VM: u64 = 0x1_01a7_1abc;

pub(crate) const WEATHER_GET_STATE_VM: u64 = 0x1_01a7_1548;

/// Chama a leitura do motor (`x0` = sistema, `Handle` forte por `x8`) e devolve pro script dentro de um `BwmsSerializable`; `parte` anota o objeto pro teste.
pub(crate) unsafe fn weathersystem_ler_handle(frame: *mut c_void, out: *mut c_void, vm: u64, #[cfg_attr(not(feature = "devtools"), allow(unused_variables))] parte: usize) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let rt = weathersystem_runtime(args.first().map_or(std::ptr::null_mut(), |(v, _)| *v as *mut c_void));
    let mut h = [0u64; 2];
    if !rt.is_null() {
        crate::jogo::chamada::sret_chamar(crate::rebase(vm) as *mut c_void, [rt as u64, 0, 0, 0], h.as_mut_ptr() as *mut c_void);
    }
    #[cfg(feature = "devtools")] crate::selftest::c81_objeto(parte, h[0]); // anotação do teste 81, só no dev
    let script = recurso_para_script(h);
    crate::jogo::objetos::soltar_handle(&mut h);
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u64, script[0]);
        core::ptr::write_unaligned((out as *mut u64).add(1), script[1]);
    }
}

/// `BwmsWeatherSystemGetEnvironmentDefinition(sys: ref<WeatherSystem>) -> ref<BwmsSerializable>`
pub(crate) unsafe extern "C" fn tramp_weathersystem_get_envdef(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    weathersystem_ler_handle(frame, out, WEATHER_GET_ENVDEF_VM, 0);
}

/// `BwmsWeatherSystemGetWeatherState(sys: ref<WeatherSystem>) -> ref<BwmsSerializable>`
pub(crate) unsafe extern "C" fn tramp_weathersystem_get_state(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    weathersystem_ler_handle(frame, out, WEATHER_GET_STATE_VM, 1);
}

/// As 2 leituras de clima (`ponte-weathersystem.reds`) e a anotação do teste.
pub(crate) unsafe fn register_weathersystem_read_natives(reg: &Registry, proto: *mut c_void) {
    let ler = |nome: &str, tramp: NativeHandler| {
        let p = compose_params_from_types(reg, &["handle:WeatherSystem"]);
        params_compostos(p) && register_global_composed(reg, proto, p, nome, nome, tramp)
    };
    let definicao = ler("BwmsWeatherSystemGetEnvironmentDefinition", tramp_weathersystem_get_envdef);
    let estado = ler("BwmsWeatherSystemGetWeatherState", tramp_weathersystem_get_state);
    let anotar = {
        let p = compose_params_from_types(reg, &["Int32", "Uint64"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsC81Anotar", "BwmsC81Anotar", corpo_de_prova!(tramp_c81_anotar)))
    };
    crate::log_dev!(
        "[reg] register_all: c81 leituras de clima GetEnvironmentDefinition={definicao} GetWeatherState={estado} BwmsC81Anotar={anotar}"
    );
}

// Clima (teste da troca, 2026-09-15): leituras do `worldRuntimeSystemWeather` pelas mesmas funções que o `SetWeatherByName` usa
// (`0x101a730fc`: quantos estados, estado por índice por sret e nome do estado).
#[cfg(feature = "devtools")]
pub(crate) const WEATHER_STATE_COUNT_VM: u64 = 0x1_01a7_1058;

#[cfg(feature = "devtools")]
pub(crate) const WEATHER_STATE_BY_INDEX_VM: u64 = 0x1_01a7_2ccc;

#[cfg(feature = "devtools")]
pub(crate) const WEATHER_STATE_NAME_VM: u64 = 0x1_01ae_0ae4;

/// Quantos estados a definição de clima ativa tem.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn clima_quantos_estados(rt: *mut c_void) -> u32 {
    let f: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(crate::rebase(WEATHER_STATE_COUNT_VM));
    f(rt)
}

/// Objeto e nome do estado `indice` da definição ativa (`(0, 0)` sem estado); o handle local é solto, a definição segura o objeto.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn clima_estado(rt: *mut c_void, indice: u32) -> (u64, u64) {
    let mut h = [0u64; 2];
    crate::jogo::chamada::sret_chamar(crate::rebase(WEATHER_STATE_BY_INDEX_VM) as *mut c_void, [rt as u64, u64::from(indice), 0, 0], h.as_mut_ptr() as *mut c_void);
    let obj = h[0];
    let nome = if obj == 0 {
        0
    } else {
        let f: extern "C" fn(*mut c_void) -> u64 = std::mem::transmute(crate::rebase(WEATHER_STATE_NAME_VM));
        f(obj as *mut c_void)
    };
    crate::jogo::objetos::soltar_handle(&mut h);
    (obj, nome)
}

/// Objeto do estado que o `GetWeatherState` do motor entrega agora (0 sem estado).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn clima_estado_atual(rt: *mut c_void) -> u64 {
    let mut h = [0u64; 2];
    crate::jogo::chamada::sret_chamar(crate::rebase(WEATHER_GET_STATE_VM) as *mut c_void, [rt as u64, 0, 0, 0], h.as_mut_ptr() as *mut c_void);
    let obj = h[0];
    crate::jogo::objetos::soltar_handle(&mut h);
    obj
}

/// `SetWeatherByIndex` do motor com os campos de um bloco (fonte, 5º argumento e prioridade): devolve o bloco anterior quando ele era igual ao atual,
/// o que o `SetWeatherByName` recusa.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn clima_por_indice(rt: *mut c_void, indice: u32, fonte: u64, quinto: u32, prioridade: u32) -> bool {
    let f: extern "C" fn(*mut c_void, u32, u64, f32, f32, u64, u64) -> bool = std::mem::transmute(crate::rebase(WEATHER_SETBYINDEX_VM));
    f(rt, indice, fonte, 0.0, 0.0, u64::from(quinto), u64::from(prioridade))
}

/// `CycleWeather` do motor (`strb w1, [x0, #0x8c]`).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn clima_ciclo(rt: *mut c_void, ligado: bool) {
    let f: extern "C" fn(*mut c_void, u32) = std::mem::transmute(crate::rebase(WEATHER_SETCYCLE_VM));
    f(rt, u32::from(ligado));
}












/// Instância do 1º parâmetro, se for um `gameJournalContainerEntry` (ou derivado) legível.
pub(crate) unsafe fn journal_container_de(inst: *mut c_void) -> Option<*mut c_void> {
    if !rtti::sane(inst) || !crate::memoria::is_readable(inst as *const c_void, JOURNAL_CONTAINER_ENTRIES_OFF + 0x10) {
        return None;
    }
    let reg = crate::registry()?;
    let alvo = reg.class_by_name("gameJournalContainerEntry");
    (!alvo.is_null() && class_is_kind_of(rtti::class_of(inst), alvo)).then_some(inst)
}

/// Os filhos (instâncias) do container, na ordem do array.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_filhos(container: *mut c_void) -> Vec<*mut c_void> {
    let base = (container as *const u8).add(JOURNAL_CONTAINER_ENTRIES_OFF);
    let ptr = (base as *const u64).read_unaligned() as *const u8;
    let n = (base.add(0x0C) as *const u32).read_unaligned() as usize;
    if ptr.is_null() || n > 4096 || !crate::memoria::is_readable(ptr as *const c_void, n * 16) {
        return Vec::new();
    }
    (0..n).map(|i| (ptr.add(i * 16) as *const u64).read_unaligned() as *mut c_void).collect()
}




// Journal de mod, parte 4 (2026-09-11): root de um `.journal` carregado por caminho. `gameJournalResource` (`0x58`) guarda o root em
// `entry` (`Handle<gameJournalEntry>`, `0x40`), pelo registro RTTI (`mov w2,#0x40` antes de `bl 0x1010a04ec`, nome "entry").
pub(crate) const JOURNAL_RESOURCE_ENTRY_OFF: usize = 0x40;

/// Root do `.journal`, se `res` for um `gameJournalResource` legível com root válido.
pub(crate) unsafe fn journal_resource_root(res: *mut c_void) -> Option<*mut c_void> {
    if !rtti::sane(res) || !crate::memoria::is_readable(res as *const c_void, JOURNAL_RESOURCE_ENTRY_OFF + 0x10) {
        return None;
    }
    let reg = crate::registry()?;
    let alvo = reg.class_by_name("gameJournalResource");
    if alvo.is_null() || !class_is_kind_of(rtti::class_of(res), alvo) {
        return None;
    }
    let root = rd_u64(res as *const c_void, JOURNAL_RESOURCE_ENTRY_OFF) as *mut c_void;
    rtti::sane(root).then_some(root)
}





// `gameJournalTree.rootEntries`: +0x78 na 2.5 (registro de propriedade com `w2 = 0x78`; +0x58 na 2.31).
pub(crate) const JOURNALTREE_RAIZES_OFF: usize = 0x78;

/// `gameJournalTree*` de um `gameJournalManager` legível.
pub(crate) unsafe fn journal_tree_de(jm: *mut c_void) -> Option<*mut c_void> {
    if !rtti::sane(jm) || !crate::memoria::is_readable(jm as *const c_void, JOURNALMANAGER_TREE_OFF + 8) {
        return None;
    }
    let reg = crate::registry()?;
    let alvo = reg.class_by_name("gameJournalManager");
    if alvo.is_null() || !class_is_kind_of(rtti::class_of(jm), alvo) {
        return None;
    }
    let tree = rd_u64(jm as *const c_void, JOURNALMANAGER_TREE_OFF) as *mut c_void;
    (rtti::sane(tree) && crate::memoria::is_readable(tree as *const c_void, 0xc8)).then_some(tree)
}

/// Instâncias das raízes (`rootEntries`) do tree.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_tree_raizes(tree: *mut c_void) -> Vec<*mut c_void> {
    let base = (tree as *const u8).add(JOURNALTREE_RAIZES_OFF);
    let ptr = (base as *const u64).read_unaligned() as *const u8;
    let n = (base.add(0x0C) as *const u32).read_unaligned() as usize;
    if ptr.is_null() || n > 256 || !crate::memoria::is_readable(ptr as *const c_void, n * 16) {
        return Vec::new();
    }
    (0..n).map(|i| (ptr.add(i * 16) as *const u64).read_unaligned() as *mut c_void).collect()
}






/// `graphGraphDefinition*` de um `questQuestPhaseResource` legível.
pub(crate) unsafe fn questphase_grafo_de(res: *mut c_void) -> Option<*const u8> {
    if !rtti::sane(res) || !crate::memoria::is_readable(res as *const c_void, 0x70) {
        return None;
    }
    let reg = crate::registry()?;
    let alvo = reg.class_by_name("questQuestPhaseResource");
    if alvo.is_null() || !class_is_kind_of(rtti::class_of(res), alvo) {
        return None;
    }
    let graph = rd_u64(res as *const c_void, QUESTPHASE_GRAPH_OFF) as *const c_void;
    (rtti::sane(graph as *mut c_void) && crate::memoria::is_readable(graph, GRAPHDEF_NODES_OFF + 0x10)).then_some(graph as *const u8)
}

// Fase de quest, parte 35c-2d-3 (2026-09-12): o mesmo patch roda no hook da rota de carga de fase (`0x1029e8c48`), em thread de job, onde RTTI já
// travou o load (boot 55). O que o patch pedia ao RTTI sai de um contexto resolvido na game thread — na leitura da tabela (1ª iteração do tick) ou na
// native —: classes pro `CreateInstance`, offsets conferidos contra a RE da 35c-2, largura e valores do `questSocketType`, tipo dos campos da conexão e
// movers. No caminho do patch sobram leitura de memória, vptr contra constante e funções que o próprio loader usa (`CreateInstance`, Handle, reserve,
// set-owner); todo objeto criado tem o vptr conferido antes de entrar no grafo. Caminho vazio = 1º `questStartNodeDefinition`, achado pelo vptr de
// instância (`0x1079fbd80`).
/// vptr de instância do `questQuestResource` (o `.quest`, filho do `questQuestPhaseResource` sem campos novos; construtor `0x1029e611c`).
pub(crate) const QUESTRES_VPTR_VM: u64 = 0x1_07a2_e050;

pub(crate) static PATCH_CTX: std::sync::Mutex<Option<PatchCtx>> = std::sync::Mutex::new(None);

/// `FindConnectionPoint`: `(grafo, nó)` pelo caminho de ids a partir do grafo, descendo por `phaseGraph` dos nós de fase; caminho vazio = 1º
/// `questStartNodeDefinition` do grafo (pelo vptr).
pub(crate) unsafe fn fase_ponto_de_conexao(graph: *const u8, caminho: &[u16]) -> Option<(*const u8, *const u8)> {
    if caminho.is_empty() {
        return handles_legiveis_em(graph, GRAPHDEF_NODES_OFF, 65536).into_iter().find(|n| vptr_e(*n, QUESTSTARTNODE_VPTR_VM)).map(|n| (graph, n));
    }
    let mut g = graph;
    for (passo, id) in caminho.iter().enumerate() {
        let no = handles_legiveis_em(g, GRAPHDEF_NODES_OFF, 65536).into_iter().find(|n| (n.add(QUESTNODE_ID_OFF) as *const u16).read_unaligned() == *id)?;
        if passo + 1 == caminho.len() {
            return Some((g, no));
        }
        if !vptr_e(no, QUESTPHASENODE_VPTR_VM) {
            return None;
        }
        let pg = (no.add(QUESTPHASENODE_GRAFO_OFF) as *const u64).read_unaligned() as *const u8;
        if !rtti::sane(pg as *mut c_void) || !crate::memoria::is_readable(pg as *const c_void, GRAPHDEF_NODES_OFF + 0x10) {
            return None;
        }
        g = pg;
    }
    None
}

/// `PatchPhase` no grafo da fase `obj` (instância de `questQuestPhaseResource`, conferida pelo vptr). `Err`: -1 fase inválida, -4 mover ou folga, -6 objeto,
/// -7 sem contexto, -8 já há um aplicado (teste da native), -9 trava, -10 id do nó de fase já no grafo de entrada, -11 ponto de conexão não achado, -12
/// ligações diferentes do esperado, -14 vptr de objeto criado diferente da RE. Erro depois do 1º efeito desfaz antes de voltar.
pub(crate) unsafe fn questphase_patch_aplicar(
    obj: *mut c_void,
    fase: &str,
    entrada: (&[u16], Option<&str>),
    saida: Option<(&[u16], Option<&str>)>,
    intercept: bool,
    ocupado: bool,
) -> Result<PatchAplicado, i32> {
    let recurso = obj as *const u8;
    if !(vptr_e(recurso, crate::jogo::mundo::FASE_DE_MISSAO_VPTR_VM) || vptr_e(recurso, QUESTRES_VPTR_VM)) || !crate::memoria::is_readable(obj as *const c_void, QUESTPHASE_GRAPH_OFF + 8) {
        return Err(-1);
    }
    let graph = (recurso.add(QUESTPHASE_GRAPH_OFF) as *const u64).read_unaligned() as *const u8;
    if !rtti::sane(graph as *mut c_void) || !crate::memoria::is_readable(graph as *const c_void, GRAPHDEF_NODES_OFF + 0x10) {
        return Err(-1);
    }
    let Some((g_in, no_in)) = fase_ponto_de_conexao(graph, entrada.0) else { return Err(-11) };
    let no_out = match saida {
        Some((caminho, socket)) => match fase_ponto_de_conexao(graph, caminho) {
            Some((_, n)) => Some((n, socket)),
            None => return Err(-11),
        },
        None => None,
    };
    let id_in = (no_in.add(QUESTNODE_ID_OFF) as *const u16).read_unaligned();
    let id = bwms_primitives::quest_phase_node_id(format!("{fase}:{id_in}").as_bytes());
    if handles_legiveis_em(g_in, GRAPHDEF_NODES_OFF, 65536).iter().any(|n| (n.add(QUESTNODE_ID_OFF) as *const u16).read_unaligned() == id) {
        return Err(-10);
    }
    if ocupado {
        return Err(-8);
    }
    let Ok(guarda) = PATCH_CTX.lock() else { return Err(-9) };
    let Some(ctx) = guarda.as_ref() else { return Err(-7) };
    let direto_antes = no_out.map_or(0, |(o, _)| fase_ligacoes(ctx, no_in, o));
    let mut p = PatchAplicado { graph: g_in as usize, no: 0, dono_no: [0; 2], sockets: Vec::new(), conexoes: Vec::new(), tiradas: Vec::new(), tocados: Vec::new(), weak_com_strong: 0 };
    let passos = fase_patch_passos(ctx, fase, id, g_in, (no_in, entrada.1.unwrap_or("Out")), no_out, intercept, &mut p);
    let no = p.no as *const u8;
    let (direto, entrada_no, no_saida) = match no_out {
        Some((o, _)) if passos.is_ok() => (fase_ligacoes(ctx, no_in, o), fase_ligacoes(ctx, no_in, no), fase_ligacoes(ctx, no, o)),
        _ if passos.is_ok() => (0, fase_ligacoes(ctx, no_in, no), 0),
        _ => (0, 0, 0),
    };
    let esperado = match no_out {
        Some(_) => direto == if intercept { 0 } else { direto_antes } && entrada_no == 1 && no_saida == 2,
        None => entrada_no == 1,
    };
    let resumo = format!(
        "nó {id} ({fase}) no grafo {g_in:p}, entrada {id_in}, intercept {intercept}: sockets criados {}, conexões criadas {}, tiradas {}; entrada→saída {direto_antes} → {direto}, entrada→nó {entrada_no}, nó→saída {no_saida}; weak com strong {}",
        p.sockets.len(),
        p.conexoes.len(),
        p.tiradas.len(),
        p.weak_com_strong
    );
    let r = match passos {
        Err(e) => e,
        Ok(()) if !esperado || p.weak_com_strong > 0 => -12,
        Ok(()) => 1,
    };
    if r != 1 {
        let (fora, de_volta, no_fora, destruidos) = fase_patch_desfaz(p);
        crate::veredito_dev!("[a35-patch] recusado ({r}) e desfeito: {resumo}; desfeito {fora} conexões, {de_volta} tiradas de volta, nó fora {no_fora}, destruídos {destruidos}");
        return Err(r);
    }
    crate::veredito_dev!("[a35-patch] aplicado: {resumo}");
    Ok(p)
}

#[derive(Clone)]
pub(crate) struct PatchDaTabela {
    pub(crate) mod_id: String,
    pub(crate) pai: u64,
    pub(crate) fase: String,
    pub(crate) entrada: Vec<u16>,
    pub(crate) socket_entrada: Option<String>,
    pub(crate) saida: Vec<u16>,
    pub(crate) socket_saida: Option<String>,
    pub(crate) intercept: bool,
}

pub(crate) static PATCHES_DA_TABELA: std::sync::Mutex<Vec<PatchDaTabela>> = std::sync::Mutex::new(Vec::new());

// Nós de fase de mod forçados (no início da quest e no `OnGameRestored`), 2026-09-18.
//
// Pra cada `.quest` que recebeu um nó de fase de mod no patch de fase, guardam-se os ids desses nós (`FASES_FORCADAS`). No início de uma quest
// (`QuestRootInstance::Start`, hook antes) marca o fact de cada nó forçado: a quest começa do zero e o grafo já passa pelo nó de mod. Na carga de save
// (`QuestsSystem::OnGameRestored`) o save pode ser de antes do mod: pra cada nó forçado sem fact, põe o caminho `{questId}` no `NodeHashMap`, força o
// início do nó pelo `ForceStartNode` com o socket `In1` e marca o fact. No Mac o FactManager fica em `[QuestsSystem+0xf8]` (`GetFact` vt+0x18,
// `SetFact` vt+0x20, `RemoveFacts` vt+0x48, store 1 = nomeado); FactID do nó = FNV-1a 32 dos 8 bytes LE de `(pathHash << 32) + nodeId`, com o
// `pathHash` = FNV-1a 32 dos `u16` do caminho; questId = índice do caminho na QuestList (`sys+0xa8`, tamanho em `+0xb4`).
pub(crate) const QUESTLIST_OFF: usize = 0xa8;

pub(crate) const FACTMANAGER_OFF: usize = 0xf8;

pub(crate) const FACT_GET_VM: u64 = 0x1_027f_23e4;

pub(crate) const FACT_SET_VM: u64 = 0x1_027f_23f8;

/// `pathHash` de um caminho de nós de quest (`MakeQuestNodePathHash`).
/// O FNV-1a encadeia pelo seed: hashear id a id dá o mesmo que hashear os `u16` LE concatenados, sem buffer.
pub(crate) fn quest_caminho_hash(caminho: &[u16]) -> u32 {
    caminho.iter().fold(bwms_primitives::FNV1A32_OFFSET, |h, id| bwms_primitives::fnv1a32_seeded(&id.to_le_bytes(), h))
}

/// FactID do nó `no` da fase de caminho `caminho_hash` (`FactID(QuestNodeKey)`).
pub(crate) fn quest_fact_do_no(caminho_hash: u32, no: u16) -> u32 {
    bwms_primitives::fnv1a32(&(((caminho_hash as u64) << 32) + no as u64).to_le_bytes())
}

/// Índice do caminho `recurso` na QuestList do sistema (o questId).
pub(crate) unsafe fn quest_id_na_lista(sys: *const c_void, recurso: u64) -> Option<u16> {
    let (e, n, cap) = dynarray_em((sys as usize + QUESTLIST_OFF) as *const c_void);
    if e == 0 || n > cap || n > 0xffff || !crate::memoria::is_readable(e as *const c_void, n as usize * 8) {
        return None;
    }
    (0..n as usize).find(|i| ((e + i * 8) as *const u64).read_unaligned() == recurso).map(|i| i as u16)
}

/// `SetFact` do store nomeado.
pub(crate) unsafe fn fact_gravar(sys: *const c_void, fact: u32, valor: i32) -> bool {
    let fm = factmanager_de(sys);
    if fm.is_null() {
        return false;
    }
    let f: extern "C" fn(*mut c_void, u32, u32, i32) = std::mem::transmute(crate::rebase(FACT_SET_VM));
    f(fm, FACTS_STORE_NOMEADO, fact, valor);
    true
}

/// Início de quest, com o QuestsSystem do contexto: marca o fact de cada nó forçado das quests forçadas que estão na QuestList. Devolve
/// quantos facts gravou. Roda na thread do Tick do sistema, dentro das travas `+0x128`/`+0x60`: só lê memória e chama o FactManager.
pub(crate) unsafe fn questphase_forcados_no_inicio(sys: *const c_void) -> usize {
    let forcadas: Vec<(u64, Vec<u16>)> = match FASES_FORCADAS.lock() {
        Ok(f) if !f.is_empty() => f.iter().map(|(k, v)| (*k, v.iter().copied().collect())).collect(),
        _ => return 0,
    };
    let mut gravados = 0;
    for (quest, nos) in forcadas {
        let Some(id) = quest_id_na_lista(sys, quest) else { continue };
        let h = quest_caminho_hash(&[id]);
        for no in nos {
            if fact_gravar(sys, quest_fact_do_no(h, no), 1) {
                gravados += 1;
            }
        }
    }
    gravados
}

// `OnGameRestored` (e o início forçado do teste de fase). NodeHashMap em `sys+0x78` (`HashMap<u32, DynArray<u16>>`, nó `{próximo, hash +4, chave +8,
// caminho +0x10}`); a inserção do motor (`0x102810994`, a mesma da `Initialize` da fase) copia o caminho e roda sob a trava `sys+0x60`. O
// `ForceStartNode` (`0x102a0d6a0`) toma a trava sozinho, acha o caminho pela chave (chave ausente derruba o jogo), pega `root.children[caminho[0]]` sem
// conferir, acha o nó por varredura linear do `nodes` do grafo da fase e o executa na hora com um contexto de modo 6. O hash da chave é o
// FNV padrão do caminho (igual ao motor só no questId 0: o motor semeia pela tabela `0x1073cc600`), então a chave pode faltar
// e é inserida.
pub(crate) const NODEHASHMAP_OFF: usize = 0x78;

pub(crate) const NODEHASHMAP_INSERIR_VM: u64 = 0x1_0281_0994;

pub(crate) const FORCESTARTNODE: (u64, u32) = (0x1_02a0_d6a0, 0xa9ba_6ffc);

/// `chave → caminho` no NodeHashMap pela inserção do motor (sret de 0x20 B), com a trava `sys+0x60`; `false` com a trava ocupada ou sem a chave depois.
pub(crate) unsafe fn nodehashmap_inserir(sys: *mut c_void, chave: u32, caminho: &[u16]) -> bool {
    #[repr(C)]
    struct Sret([u64; 4]);
    let buf = caminho.to_vec();
    let n = buf.len() as u64;
    let arr: [u64; 2] = [buf.as_ptr() as u64, (n << 32) | n];
    let f: extern "C" fn(*mut c_void, *const u32, *const [u64; 2]) -> Sret = std::mem::transmute(crate::rebase(NODEHASHMAP_INSERIR_VM));
    if !quests_travar(sys) {
        return false;
    }
    let _ = f((sys as usize + NODEHASHMAP_OFF) as *mut c_void, &chave, &arr);
    quests_soltar(sys);
    nodehashmap_caminho(sys, chave).as_deref() == Some(caminho)
}

/// Fase filha da raiz no índice `id` (a raiz indexa os filhos pelo questId sem conferir), só se ela existe.
pub(crate) unsafe fn quest_fase_no_indice(sys: *const c_void, id: u16) -> Option<usize> {
    let root = rd_u64(sys, 0x68) as usize;
    if root == 0 || !crate::memoria::is_readable((root + 0x88) as *const c_void, 0x10) {
        return None;
    }
    let (e, n, _) = cust_arr(root + 0x88);
    if e == 0 || id as usize >= n || !crate::memoria::is_readable((e + id as usize * 16) as *const c_void, 16) {
        return None;
    }
    let inst = ((e + id as usize * 16) as *const u64).read_unaligned() as usize;
    (inst != 0 && crate::memoria::is_readable(inst as *const c_void, 0x90)).then_some(inst)
}

/// Caminho do recurso da fase (`[instância+0x30]` → `+0x30`), 0 sem ele.
pub(crate) unsafe fn quest_fase_recurso(inst: usize) -> u64 {
    let res = rd_u64(inst as *const c_void, 0x30) as usize;
    if res == 0 || !crate::memoria::is_readable((res + 0x30) as *const c_void, 8) { 0 } else { rd_u64(res as *const c_void, 0x30) }
}

/// `ForceStartNode(sys, &{chave, nó}, &["In1"])` do motor. Só com o caminho da chave no NodeHashMap e a fase filha do 1º id dele viva: sem isso o motor
/// derruba o jogo.
pub(crate) unsafe fn quest_forcar_no(sys: *mut c_void, chave: u32, no: u16) -> bool {
    let f = crate::rebase(FORCESTARTNODE.0);
    let caminho = nodehashmap_caminho(sys, chave).unwrap_or_default();
    if (f as *const u32).read_unaligned() != FORCESTARTNODE.1 || caminho.first().is_none_or(|id| quest_fase_no_indice(sys, *id).is_none()) {
        return false;
    }
    let socket = [crate::cname::cname("In1")];
    let sockets: [u64; 2] = [socket.as_ptr() as u64, (1 << 32) | 1];
    let chave_no: [u32; 2] = [chave, no as u32];
    let g: extern "C" fn(*mut c_void, *const [u32; 2], *const [u64; 2]) = std::mem::transmute(f);
    g(sys, &chave_no, &sockets);
    true
}

/// `OnGameRestored`: pra cada quest forçada na QuestList (com a fase dela no índice), a chave `{questId}` no NodeHashMap e, pra cada nó
/// forçado sem fact, o `ForceStartNode` e o fact. Devolve quantos nós forçou.
pub(crate) unsafe fn questphase_forcar_na_restauracao(sys: *mut c_void) -> usize {
    let forcadas: Vec<(u64, Vec<u16>)> = match FASES_FORCADAS.lock() {
        Ok(f) if !f.is_empty() => f.iter().map(|(k, v)| (*k, v.iter().copied().collect())).collect(),
        _ => return 0,
    };
    let mut forcados = 0;
    for (quest, nos) in forcadas {
        let Some(id) = quest_id_na_lista(sys, quest) else { continue };
        if quest_fase_no_indice(sys, id).is_none_or(|inst| quest_fase_recurso(inst) != quest) {
            continue;
        }
        let h = quest_caminho_hash(&[id]);
        match nodehashmap_caminho(sys, h) {
            Some(c) if c == [id] => {}
            Some(_) => continue,
            None if nodehashmap_inserir(sys, h, &[id]) => {}
            None => continue,
        }
        for no in nos {
            let fact = quest_fact_do_no(h, no);
            if fact_ler(sys, fact) == Some(0) && quest_forcar_no(sys, h, no) {
                fact_gravar(sys, fact, 1);
                forcados += 1;
            }
        }
    }
    forcados
}

/// Teste 55: nó `questFactsDBManagerNodeDefinition` com um `questSetVar_NodeType` que grava `fact = 1` (valor exato), id `id`, sem sockets, dentro de um
/// `Handle` forte que fica com quem chama.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn quest_no_de_fact(fact: &str, id: u16) -> Option<[u64; 2]> {
    let reg = Registry::obtain()?;
    let tipo = rtti::new_object(&reg, "questSetVar_NodeType");
    let no = rtti::new_object(&reg, "questFactsDBManagerNodeDefinition");
    if !rtti::sane(tipo) || !rtti::sane(no) {
        return None;
    }
    let mut dono = [0u64; 2];
    rtti::make_handle(dono.as_mut_ptr() as *mut c_void, no);
    let mut solto = [0u64; 2];
    rtti::make_handle(solto.as_mut_ptr() as *mut c_void, tipo);
    let ct = rtti::class_of(tipo);
    let campos = (ws_campo(tipo as *mut u8, ct, "factName"), ws_campo(tipo as *mut u8, ct, "value"), ws_campo(tipo as *mut u8, ct, "setExactValue"));
    let id_no = ws_campo(no as *mut u8, rtti::class_of(no), "id");
    let ok = match (campos, id_no) {
        ((Some(nome), Some(valor), Some(exato)), Some(i)) => {
            let escrito = rtti::red_string_write_inline(nome as *mut u8, fact);
            (valor as *mut i32).write_unaligned(1);
            *(exato as *mut u8) = 1;
            (i as *mut u16).write_unaligned(id);
            escrito && campo_handle_grava(no as *mut u8, "type", tipo as *mut u8)
        }
        _ => false,
    };
    crate::jogo::objetos::soltar_handle(&mut solto);
    if ok {
        Some(dono)
    } else {
        crate::jogo::objetos::soltar_handle(&mut dono);
        None
    }
}

/// Teste 55: sockets `In1` e `Out` no nó, pelo mesmo criador do `PatchPhase` (dono ligado). Sem o `Out`, a saída que o `Execute` empurra no contexto
/// de modo 6 vai pra lista pendente e a retomada do Tick (`0x102a1077c` → `0x1028122f4` → `0x102804dc8`) derruba o jogo procurando o socket (boot
/// 248, nulo + 0x58). Devolve os donos dos sockets, pra soltar na volta.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn quest_no_sockets(no: usize) -> Option<Vec<[u64; 2]>> {
    if !patch_ctx_iniciar() {
        return None;
    }
    let guarda = PATCH_CTX.lock().ok()?;
    let ctx = guarda.as_ref()?;
    let mut p = PatchAplicado { graph: 0, no: 0, dono_no: [0; 2], sockets: Vec::new(), conexoes: Vec::new(), tiradas: Vec::new(), tocados: Vec::new(), weak_com_strong: 0 };
    let ok = fase_socket_resolve(ctx, no as *const u8, ctx.tipo_in, "In1", &mut p).is_ok() && fase_socket_resolve(ctx, no as *const u8, ctx.tipo_out, "Out", &mut p).is_ok();
    let donos: Vec<[u64; 2]> = p.sockets.into_iter().map(|(_, _, d)| d).collect();
    if ok {
        Some(donos)
    } else {
        for mut d in donos {
            crate::jogo::objetos::soltar_handle(&mut d);
        }
        None
    }
}

/// Teste 55: acrescenta o nó (`Handle`, forte +1) no fim do `nodes` (`+0x30`) do grafo, com a trava `sys+0x60`.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn quest_grafo_anexar(sys: *mut c_void, grafo: usize, dono: [u64; 2]) -> bool {
    if !quests_travar(sys) {
        return false;
    }
    let ok = cust_anexar_handle(grafo + 0x30, dono);
    quests_soltar(sys);
    ok
}

/// Teste 55: devolve o `nodes` do grafo ao `tamanho` de antes (soltando o que passa dele), com a trava `sys+0x60`.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn quest_grafo_truncar(sys: *mut c_void, grafo: usize, tamanho: usize) -> bool {
    if !quests_travar(sys) {
        return false;
    }
    cust_truncar(grafo + 0x30, tamanho, CustElemento::Handle);
    quests_soltar(sys);
    cust_arr(grafo + 0x30).1 == tamanho
}

/// Lê `bwms-questphase.txt` (`<mod> | <fase pai> | <fase do mod> | <nó de entrada> | <socket de entrada> | <nó de saída> | <socket de saída> |
/// <intercept>`, ids separados por espaço, `-` ou vazio = sem valor; `bwms_primitives::tabelas::Questphase`) e resolve o contexto (game thread). Linha
/// malformada é ignorada com log. O arquivo ausente esvazia a tabela sem resolver contexto: o `bwms install` o apaga quando nenhum mod gera patch, e a
/// recarga do `Reload` da fachada de recursos tira os patches anteriores.
pub(crate) unsafe fn questphase_tabela(caminho: &str) -> usize {
    let conteudo = match std::fs::read_to_string(caminho) {
        Ok(c) => c,
        Err(e) => {
            crate::log_dev!("[a35-carga-patch] não leu '{caminho}': {e}");
            if e.kind() == std::io::ErrorKind::NotFound {
                if let Ok(mut t) = PATCHES_DA_TABELA.lock() {
                    t.clear();
                }
            }
            return 0;
        }
    };
    let mut tabela = Vec::new();
    for lida in bwms_primitives::tabelas::ler(&conteudo, bwms_primitives::tabelas::Questphase::ler) {
        match lida {
            Ok(l) => tabela.push(PatchDaTabela {
                mod_id: l.mod_id.to_string(),
                pai: bwms_primitives::resource_path_hash(l.pai),
                fase: l.fase.to_string(),
                entrada: l.entrada,
                socket_entrada: l.socket_entrada.map(str::to_string),
                saida: l.saida,
                socket_saida: l.socket_saida.map(str::to_string),
                intercept: l.intercept,
            }),
            Err(linha) => crate::log_dev!("[a35-carga-patch] linha ignorada: '{linha}'"),
        }
    }
    let n = tabela.len();
    let pais: Vec<String> = tabela.iter().map(|p| format!("{:#018x}", p.pai)).collect();
    if let Ok(mut t) = PATCHES_DA_TABELA.lock() {
        *t = tabela;
    }
    let ctx = patch_ctx_iniciar();
    crate::veredito_dev!("[a35-carga-patch] {n} patch(es) de '{caminho}', pais {pais:?}; contexto pronto {ctx}");
    n
}

/// Hook de carga (thread de job): aplica os patches da tabela cujo pai é o path carregado. Sem contexto pronto, nada é aplicado (o código fica no log).
pub(crate) unsafe fn questphase_patches_na_carga(path: u64, fase: *mut c_void) {
    let patches: Vec<PatchDaTabela> = match PATCHES_DA_TABELA.lock() {
        Ok(t) => t.iter().filter(|p| p.pai == path).cloned().collect(),
        Err(_) => return,
    };
    if patches.is_empty() || fase.is_null() || !crate::memoria::is_readable(fase as *const c_void, 8) {
        return;
    }
    let obj = (fase as *const u64).read_unaligned() as *mut c_void;
    for p in patches {
        let saida = (!p.saida.is_empty()).then(|| (&p.saida[..], p.socket_saida.as_deref()));
        let codigo = match questphase_patch_aplicar(obj, &p.fase, (&p.entrada[..], p.socket_entrada.as_deref()), saida, p.intercept, false) {
            Ok(a) => {
                // Nó de fase de mod num `.quest`: forçado na restauração e marcado no início da quest (`FASES_FORCADAS`).
                if vptr_e(obj as *const u8, QUESTRES_VPTR_VM) && a.no != 0 {
                    let id = ((a.no + QUESTNODE_ID_OFF) as *const u16).read_unaligned();
                    if let Ok(mut f) = FASES_FORCADAS.lock() {
                        f.entry(path).or_default().insert(id);
                    }
                }
                if let Ok(mut v) = PATCHES_NA_CARGA.lock() {
                    v.push(a);
                }
                1
            }
            Err(e) => e,
        };
        PATCH_CARGA_ULTIMO.store(codigo, Ordering::Relaxed);
        crate::veredito_dev!("[a35-carga-patch] {}: {} na carga de {path:#018x} -> {codigo}", p.mod_id, p.fase);
    }
}

/// Patches da tabela aplicados na carga (rota de fase, recriação e início de quest).
#[cfg(feature = "devtools")]
pub(crate) fn questphase_patches_aplicados() -> usize {
    PATCHES_NA_CARGA.lock().map_or(0, |v| v.len())
}











/// Endereço do slot do loader, só se for a função da RE.
pub(crate) unsafe fn questloader_slot(l: *mut c_void, (slot, vm): (usize, u64)) -> Option<usize> {
    let vt = rd_u64(l as *const c_void, 0) as *const u64;
    if vt.is_null() || !crate::memoria::is_readable(vt as *const c_void, slot + 8) {
        return None;
    }
    let f = vt.add(slot / 8).read_unaligned() as usize;
    (f == crate::rebase(vm) as usize).then_some(f)
}







// Journal de mod, parte 5c-1 (2026-09-11, RE offline do registro de entrada da parte 5c): pré-requisitos do registro
// de entrada, só leitura. O hash de uma entrada é MurmurHash3_x86_32 com a seed `0x5EEDBA5E` (`bwms_primitives::RECORDS_SEED`) do path
// "pasta/.../id" montado subindo pelos pais (o motor lê a cauda com sinal; igual pra path ASCII). Pastas e filhos saem pelo layout da parte 3; a
// classe, pelo nome do tipo (`+0x18` do CClass), porque o `GetEntryByString` vanilla pede a classe; o mover do `DynArray<Handle<JournalEntry>>`
// (símbolo `MoveAfterReallocation` em `0x1051fe924`) é conferido no GOT antes de qualquer append da 5c-2. O GOT é `0x1077a7088` (bind do
// símbolo pelo `dyld_info -fixups`); o `0x1077a71d8` da RE era o de `THandle<JournalEntryOverrideData>` e falhou no boot 53.
pub(crate) const JOURNAL_ENTRY_MOVER_GOT_VM: u64 = 0x1_077a_7088;

pub(crate) const JOURNAL_ENTRY_MOVER_VM: u64 = 0x1_051f_e924;







// Journal de mod, parte 5c-2 (2026-09-11, RE offline do registro de entrada da parte 5c): 1 entrada de teste registrada na
// árvore viva e desfeita na mesma execução de script. Inserir = append nos `entries` da pasta `onscreens` da raiz 0 (trava de byte `+0x70`, reserve do
// motor com o mover conferido no GOT), vínculo filho→pai no tree (`0x105232f70`, que MOVE os handles) e `jm->vt[0x1e0]` (os índices A/B copiam +1).
// Desfazer = `jm->vt[0x1e8]`, `0x105232fcc(tree, &filho)` e remoção do `entries`. O objeto (`gameJournalOnscreen`, id `bwms_teste_5c`) tem dono
// vazado: nenhuma soltura chega a destruí-lo (a destruição pelo motor usaria outro allocator, AL-1). Toda checagem vem antes do 1º efeito.
pub(crate) const JOURNALMANAGER_REGISTER_VM: u64 = 0x1_0524_3f0c;

pub(crate) const JOURNALMANAGER_UNREGISTER_VM: u64 = 0x1_0524_3fc8;

pub(crate) const JOURNALTREE_PAI_INSERE_VM: u64 = 0x1_0523_2f70;










/// Append de `dono` nos `entries` da pasta, sob a trava dela, com 1 strong a mais pro array. 1 = ok; -7 reserve sem capacidade (nada anexado). É o passo
/// (a) do `journal_anexar` e a mescla inteira na raiz da 7e-6b-2, onde o `Initialize` de cada entrada, que roda depois, faz o vínculo e o índice.
pub(crate) unsafe fn journal_anexar_so_entries(pasta: *mut c_void, dono: [u64; 2]) -> i32 {
    let mover = (crate::rebase(JOURNAL_ENTRY_MOVER_GOT_VM) as *const u64).read_unaligned();
    let arr = (pasta as *mut u8).add(JOURNAL_CONTAINER_ENTRIES_OFF);
    journal_trava(pasta);
    let cap = (arr.add(0x08) as *const u32).read_unaligned();
    let tam = (arr.add(0x0C) as *const u32).read_unaligned();
    if tam == cap {
        crate::jogo::listas::reservar(arr as *mut c_void, tam + 1, 0x10, 8, if tam == 0 { std::ptr::null() } else { mover as *const c_void });
    }
    if (arr.add(0x08) as *const u32).read_unaligned() <= tam {
        journal_solta(pasta);
        return -7;
    }
    let ptr = (arr as *const u64).read_unaligned() as *mut u64;
    handle_strong_soma(&dono, 1);
    ptr.add(tam as usize * 2).write_unaligned(dono[0]);
    ptr.add(tam as usize * 2 + 1).write_unaligned(dono[1]);
    (arr.add(0x0C) as *mut u32).write_unaligned(tam + 1);
    journal_solta(pasta);
    1
}

/// Anexa `dono` (handle com 1 strong de quem chama) à árvore viva: (a) append nos `entries` da pasta, sob a trava dela; (b) vínculo filho→pai; (c)
/// índices A/B pelo slot `0x1e0`. 1 = ok; -7 reserve sem capacidade (nada anexado). Mover e slots são conferidos por quem chama.
pub(crate) unsafe fn journal_anexar(jm: *mut c_void, tree: *mut c_void, pasta: *mut c_void, pasta_h: [u64; 2], dono: *mut [u64; 2]) -> i32 {
    let vt = (jm as *const u64).read_unaligned();
    let slot = |off: u64| ((vt + off) as *const u64).read_unaligned();
    // (a) append nos `entries` da pasta, sob a trava dela
    if journal_anexar_so_entries(pasta, *dono) != 1 {
        return -7;
    }
    // (b) vínculo filho→pai
    journal_pai_inserir(tree, *dono, pasta_h);
    // (c) índices A/B
    let register: unsafe extern "C" fn(*mut c_void, *mut [u64; 2]) = std::mem::transmute(slot(0x1e0) as usize);
    register(jm, dono);
    1
}












// Journal de mod, parte 7e-2 (2026-09-12): fluxo de produção do journal de mod. A instalação grava `red4ext/bwms-journal.txt` (7e-1, `bwms_primitives::instalacao`); a
// carga das tabelas lê as linhas `<mod> | <caminho do .journal>` e, em gameplay, no ponto assentado do `OnApply` (sem executor aninhado), cada `.journal`
// é pedido ao `LoadAsync` quando o caminho existe no depot e, carregado, mesclado na raiz viva 0: entrada com id que já existe e é container nos dois lados
// desce; entrada nova entra com a subárvore inteira, na ordem do registro do motor na carga de um `.journal` (`0x105202418`: mapa filho→pai e `Register`
// no container, depois nos filhos) — o topo pelo `journal_anexar` e cada descendente pelo mapa de pais e pelos índices, de cima pra baixo, porque o hash do
// caminho sobe pelo mapa. As entradas novas passam pela conversão de LocKeys (6b). O `JournalManager` sai do mapa de sistemas do `GameInstance` (só
// leitura). Boot 79: tabela lida, carga e mescla em produção (28465 casadas, 1 anexada com 9 descendentes). Com o marcador
// `~/.bwms-hook-journal-7e-teste`, um objetivo da cópia ganha o sufixo `_bwms` antes da mescla (a cópia do `cooked_journal` não tem entrada nova) e a mescla
// pode ser desfeita pela native de teste; desde a 7e-3c o objetivo é o pai do 1º pin de quest cujo nó é de área, pra existir pin com volume.
// Parte 7e-3 (2026-09-12): os pins das entradas novas viram pendentes com o hash Murmur3 do caminho, e o tick os registra na
// tabela de mappins de mod (7d-2a) na posição do nó do NodeRef, resolvido pelo `ResolveNodeRef` vanilla na raiz global e achado pelo `FindNode` — o cozido
// vanilla guarda a posição do nó, sem o offset (boot 72) —; pin sem NodeRef fica no offset e nó fora do registro espera o streaming.
#[cfg(feature = "devtools")]
pub(crate) const JOURNAL_7E_TESTE_SUFIXO: &str = "_bwms";

/// Pins de quest examinados, no máximo, na busca do objetivo de teste.
#[cfg(feature = "devtools")]
pub(crate) const JOURNAL_7E_TESTE_BUSCA_MAX: usize = 4000;

/// Pins do objetivo de teste renomeado: `(caminho novo, caminho original)`, pro smoke comparar posição e volume de cada um com o cozido vanilla.
#[cfg(feature = "devtools")]
pub(crate) static JOURNAL_7E_TESTE_PINS: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Pins resolvidos por tick, pra um journal grande não pesar num frame só.
pub(crate) const PINS_POR_TICK: usize = 16;

// Journal de mod, parte 7e-4 (2026-09-12): edição de entrada existente. O último segmento do id de uma entrada do journal de mod terminado
// em `*` marca edição: com a entrada viva achada e o mesmo tipo, os LocKeys da do mod são convertidos e cada propriedade dela, menos o `id`, vai pra viva
// pelo `Assign` do tipo; depois a descida nos filhos segue como na mescla. Id com `/` anda pelas pastas vivas a partir da pasta; caminho que não existe é
// recusado sem anexar. Duas escolhas deliberadas: os `entries` do container não são copiados (trocariam os filhos vivos por fora dos índices do journal —
// a descida mescla os filhos), e a entrada nova com `/` no id fica com o último segmento como id (com o id inteiro, o caminho montado pelo motor não bate
// com o hash do pin). Cada valor substituído fica numa cópia do tipo pro desfazer do teste. O `Assign` (`vt+0x58`) é conferido uma vez numa String e num
// Vector3 temporários antes da 1ª edição; sem a conferência, nada é editado.
pub(crate) const JOURNAL_EDICAO_MARCA: char = '*';

/// Id do pin de controle do teste: marcado pra edição, com um caminho que não existe na árvore viva.
#[cfg(feature = "devtools")]
pub(crate) const JOURNAL_7E4_CONTROLE_ID: &str = "bwms_sem_caminho*";

/// Deslocamento do offset X do pin editado no teste.
#[cfg(feature = "devtools")]
pub(crate) const JOURNAL_7E4_DELTA_X: f32 = 1.5;

/// Tamanho da instância no `CClass`.
pub(crate) const CCLASS_TAMANHO_OFF: usize = 0x68;

/// `Assign` dos tipos: 0 não conferido, 1 conferido, 2 falhou.
pub(crate) static ASSIGN_CONFERIDO: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Entrada viva editada: objeto, caminho e, por campo, `(tipo, offset, cópia do valor anterior, tamanho da cópia)`.
pub(crate) struct EditadaDeMod {
    pub(crate) alvo: usize,
    pub(crate) caminho: String,
    pub(crate) campos: Vec<(usize, usize, usize, usize)>,
}

/// Teste da 7e-4: pin editado e pin de controle na cópia com os ids originais, o pin vivo do editado e os offsets original e editado.
#[cfg(feature = "devtools")]
pub(crate) struct TesteEdicao {
    pub(crate) editado: usize,
    pub(crate) editado_id: String,
    pub(crate) controle: usize,
    pub(crate) controle_id: String,
    pub(crate) vivo: usize,
    pub(crate) original: [f32; 3],
    pub(crate) editado_offset: [f32; 3],
}

/// Entrada nova anexada à árvore viva: topo, pasta viva e os descendentes na ordem em que entraram.
pub(crate) struct AnexadaDeMod {
    pub(crate) topo: [u64; 2],
    pub(crate) pasta: usize,
    pub(crate) caminho: String,
    pub(crate) descendentes: Vec<[u64; 2]>,
}

pub(crate) struct JournalDeMod {
    pub(crate) mod_id: String,
    pub(crate) caminho: String,
    pub(crate) token: u64,
    /// 0 pendente, 1 carregando, 2 mesclado; negativo = falha: -1 sem loader, -2 token sumiu, -3 carga falhou, -4 sem tree ou raiz, -5 recurso sem
    /// root, -6 root é a árvore viva, -9 mover no GOT, -10 slots do `JournalManager`.
    pub(crate) estado: i32,
    pub(crate) casadas: usize,
    pub(crate) anexadas: Vec<AnexadaDeMod>,
    pub(crate) entradas_novas: usize,
    pub(crate) pins_novos: usize,
    pub(crate) lockeys: usize,
    #[cfg(feature = "devtools")]
    pub(crate) teste_renomeado: Option<(usize, String)>,
    pub(crate) editadas: Vec<EditadaDeMod>,
    pub(crate) edicoes_recusadas: usize,
    #[cfg(feature = "devtools")]
    pub(crate) teste_edicao: Option<TesteEdicao>,
    /// 7e-6b-2: mesclado no `Initialize` da raiz, antes do índice.
    #[cfg(feature = "devtools")]
    pub(crate) na_raiz: bool,
}

/// Pin de mod à espera de posição: hash do caminho, busca (POI ou mappin), NodeRef e offset.
pub(crate) struct PinDeModPendente {
    pub(crate) hash: u32,
    pub(crate) poi: bool,
    pub(crate) noderef: u64,
    pub(crate) offset: [f32; 3],
    pub(crate) caminho: String,
    pub(crate) tentativas: u32,
}

pub(crate) static JOURNAL_MODS: std::sync::Mutex<Vec<JournalDeMod>> = std::sync::Mutex::new(Vec::new());

pub(crate) static JOURNAL_MODS_JM: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(crate) static PINS_DE_MOD: std::sync::Mutex<Vec<PinDeModPendente>> = std::sync::Mutex::new(Vec::new());

/// Pins já registrados na tabela de mappins de mod: `(hash, poi)`, pra o desfazer do teste retirar.
pub(crate) static PINS_DE_MOD_REGISTRADOS: std::sync::Mutex<Vec<(u32, bool)>> = std::sync::Mutex::new(Vec::new());

/// Lê `bwms-journal.txt` (`bwms_primitives::tabelas::Journal`); devolve as linhas aceitas. Só texto (roda na carga das tabelas); journal já na lista não é
/// trocado. Com `substituir` (o `Reload` da fachada de recursos), o arquivo ausente vale como vazio e a lista vira a do arquivo mais o journal já mesclado (estado 2),
/// que está na árvore viva e só sai com o jogo recarregado (o desfazer só mexe no que a mescla anexou ou editou). O que o arquivo não lista sai mesmo com
/// a carga já pedida (estado 1): ficando, o `journal_mods_tick` o mesclaria na árvore viva na volta seguinte.
pub(crate) fn journal_tabela(arquivo: &str, substituir: bool) -> usize {
    let texto = match std::fs::read_to_string(arquivo) {
        Ok(t) => t,
        Err(e) if substituir && e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return 0,
    };
    let linhas: Vec<(String, String)> = bwms_primitives::tabelas::ler(&texto, bwms_primitives::tabelas::Journal::ler)
        .flatten()
        .map(|j| (j.mod_id.to_string(), j.caminho.to_string()))
        .collect();
    let Ok(mut g) = JOURNAL_MODS.lock() else { return 0 };
    if substituir {
        g.retain(|j| j.estado == 2 || linhas.iter().any(|(_, c)| *c == j.caminho));
    }
    for (mod_id, caminho) in &linhas {
        if !g.iter().any(|j| j.caminho == *caminho) {
            g.push(JournalDeMod {
                mod_id: mod_id.clone(),
                caminho: caminho.clone(),
                token: 0,
                estado: 0,
                casadas: 0,
                anexadas: Vec::new(),
                entradas_novas: 0,
                pins_novos: 0,
                lockeys: 0,
                #[cfg(feature = "devtools")]
                teste_renomeado: None,
                editadas: Vec::new(),
                edicoes_recusadas: 0,
                #[cfg(feature = "devtools")]
                teste_edicao: None,
                #[cfg(feature = "devtools")]
                na_raiz: false,
            });
        }
    }
    crate::veredito_dev!("[a44-7e] tabela {arquivo}: {} linha(s), {} journal(s) de mod na lista", linhas.len(), g.len());
    linhas.len()
}

/// `JournalManager` pelo mapa de sistemas do `GameInstance` do player (só leitura), pela chave `gameJournalManager`.
pub(crate) unsafe fn journal_manager_do_jogo(reg: &Registry, player: *mut c_void) -> Option<*mut c_void> {
    if player.is_null() {
        return None;
    }
    let b = crate::jogo::sessao::game_instance(reg, player)?;
    let game = u64::from_le_bytes(b[..8].try_into().ok()?) as *mut c_void;
    let jm = crate::jogo::sessao::sistema_do_motor_por_nome(reg, game, "gameJournalManager", |jm| journal_tree_de(jm).is_some());
    (!jm.is_null()).then_some(jm)
}

/// Handle `{instância, bloco}` da raiz viva `indice` (`rootEntries` do tree).
pub(crate) unsafe fn journal_tree_raiz_handle(tree: *mut c_void, indice: usize) -> Option<[u64; 2]> {
    let base = (tree as *const u8).add(JOURNALTREE_RAIZES_OFF);
    let ptr = (base as *const u64).read_unaligned() as *const u64;
    let n = (base.add(0x0C) as *const u32).read_unaligned() as usize;
    if ptr.is_null() || indice >= n || n > 256 || !crate::memoria::is_readable(ptr as *const c_void, n * 16) {
        return None;
    }
    Some([ptr.add(indice * 2).read_unaligned(), ptr.add(indice * 2 + 1).read_unaligned()])
}

/// Handles dos filhos do container, na ordem do array.
pub(crate) unsafe fn journal_filhos_handles(container: *mut c_void) -> Vec<[u64; 2]> {
    let base = (container as *const u8).add(JOURNAL_CONTAINER_ENTRIES_OFF);
    let ptr = (base as *const u64).read_unaligned() as *const u64;
    let n = (base.add(0x0C) as *const u32).read_unaligned() as usize;
    if ptr.is_null() || n > 4096 || !crate::memoria::is_readable(ptr as *const c_void, n * 16) {
        return Vec::new();
    }
    (0..n).map(|i| [ptr.add(i * 2).read_unaligned(), ptr.add(i * 2 + 1).read_unaligned()]).collect()
}

/// `id` da entrada (vazio se ilegível).
pub(crate) unsafe fn journal_id_de(entrada: *mut c_void) -> String {
    if !rtti::sane(entrada) || !crate::memoria::is_readable(entrada as *const c_void, JOURNAL_ENTRY_ID_OFF + 0x20) {
        return String::new();
    }
    rtti::read_cstring((entrada as *const u8).add(JOURNAL_ENTRY_ID_OFF)).unwrap_or_default()
}

/// Endereço do slot `off` do vtable do `JournalManager`.
pub(crate) unsafe fn journal_slot(jm: *mut c_void, off: u64) -> usize {
    let vt = (jm as *const u64).read_unaligned();
    ((vt + off) as *const u64).read_unaligned() as usize
}

/// Mescla de `modc` em `vivo` que separa o que falta: casadas somam em `casadas`; cada entrada da cópia sem id igual na pasta viva vira
/// `(pasta viva, handle da pasta viva, handle da entrada, caminho)`. Id com `/` anda pelas pastas vivas antes do último segmento; último segmento com `*`
/// (7e-4) marca edição: achada, a entrada entra em `editar` como `(entrada do mod, entrada viva, caminho)`; não achada, ou caminho que não existe, vai pra
/// `recusadas`.
pub(crate) unsafe fn journal_mod_descer(
    modc: *mut c_void,
    vivo: *mut c_void,
    vivo_h: [u64; 2],
    caminho: &str,
    prof: u32,
    casadas: &mut usize,
    faltam: &mut Vec<(*mut c_void, [u64; 2], [u64; 2], String)>,
    editar: &mut Vec<(*mut c_void, *mut c_void, String)>,
    recusadas: &mut Vec<String>,
) {
    let vivos = journal_ids_mapa(vivo);
    for h in journal_filhos_handles(modc) {
        let id = journal_id_de(h[0] as *mut c_void);
        if id.is_empty() {
            continue;
        }
        let marcado = id.ends_with(JOURNAL_EDICAO_MARCA);
        let limpo = id.strip_suffix(JOURNAL_EDICAO_MARCA).unwrap_or(&id);
        let mut segmentos: Vec<&str> = limpo.split('/').collect();
        let ultimo = segmentos.pop().unwrap_or_default();
        let (mut pasta, mut pasta_h, mut pc) = (vivo, vivo_h, caminho.to_string());
        let mut andou = !ultimo.is_empty();
        for s in &segmentos {
            let prox = journal_ids_mapa(pasta).get(*s).copied().and_then(|hv| journal_container_guardado(hv[0] as *mut c_void).map(|c| (c, hv)));
            let Some((c, hv)) = prox else {
                andou = false;
                break;
            };
            pasta = c;
            pasta_h = hv;
            pc = if pc.is_empty() { s.to_string() } else { format!("{pc}/{s}") };
        }
        let p = if pc.is_empty() { ultimo.to_string() } else { format!("{pc}/{ultimo}") };
        if !andou {
            recusadas.push(p);
            continue;
        }
        let achada = if segmentos.is_empty() { vivos.get(ultimo).copied() } else { journal_ids_mapa(pasta).get(ultimo).copied() };
        match achada {
            None if marcado => recusadas.push(p),
            None => {
                if !segmentos.is_empty() && !journal_id_trocar(h[0] as *mut c_void, ultimo) {
                    recusadas.push(p);
                    continue;
                }
                faltam.push((pasta, pasta_h, h, p));
            }
            Some(hv) => {
                *casadas += 1;
                if hv[0] != h[0] {
                    if marcado {
                        editar.push((h[0] as *mut c_void, hv[0] as *mut c_void, p.clone()));
                    }
                    if prof < 16 {
                        if let (Some(mc), Some(vc)) = (journal_container_guardado(h[0] as *mut c_void), journal_container_guardado(hv[0] as *mut c_void)) {
                            journal_mod_descer(mc, vc, hv, &p, prof + 1, casadas, faltam, editar, recusadas);
                        }
                    }
                }
            }
        }
    }
}

/// Descendentes de `topo` (caminho `caminho`), de cima pra baixo: `(filho, pai, caminho do filho)`; id vazio não entra no caminho, como no motor.
pub(crate) unsafe fn journal_descendentes(topo: [u64; 2], caminho: &str) -> Vec<([u64; 2], [u64; 2], String)> {
    let mut v = Vec::new();
    let mut fila = std::collections::VecDeque::from([(topo, caminho.to_string())]);
    while let Some((pai, pc)) = fila.pop_front() {
        if v.len() > 100_000 {
            break;
        }
        let Some(cont) = journal_container_guardado(pai[0] as *mut c_void) else { continue };
        for f in journal_filhos_handles(cont) {
            if rtti::sane(f[0] as *mut c_void) {
                let id = journal_id_de(f[0] as *mut c_void);
                let c = if id.is_empty() { pc.clone() } else { format!("{pc}/{id}") };
                v.push((f, pai, c.clone()));
                fila.push_back((f, c));
            }
        }
    }
    v
}

/// Vínculo filho→pai no tree com cópias dos dois handles: o motor move as cópias; o que sobra (chave já existente) volta com -1.
pub(crate) unsafe fn journal_pai_inserir(tree: *mut c_void, filho: [u64; 2], pai: [u64; 2]) {
    let (mut f, mut p) = (filho, pai);
    handle_strong_soma(&f, 1);
    handle_strong_soma(&p, 1);
    let insere: unsafe extern "C" fn(*mut c_void, *mut [u64; 2], *mut [u64; 2]) = std::mem::transmute(crate::rebase(JOURNALTREE_PAI_INSERE_VM));
    insere(tree, &mut f, &mut p);
    if f[0] != 0 {
        handle_strong_soma(&f, -1);
    }
    if p[0] != 0 {
        handle_strong_soma(&p, -1);
    }
}

/// O nó da instância do id é `worldAreaShapeNode` (ou derivado).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn no_de_area(id: u64) -> bool {
    let Some(mut h) = find_node(id) else { return false };
    let inst = h[0] as *const u8;
    if inst.is_null() {
        return false;
    }
    let area = crate::memoria::is_readable(inst as *const c_void, 0x90) && {
        let no = (inst.add(NODEINSTANCE_NO_OFF) as *const u64).read_unaligned() as *const u8;
        rtti::sane(no as *mut c_void) && crate::memoria::is_readable(no as *const c_void, 0x60) && acessor_e_da_classe(no, "worldAreaShapeNode")
    };
    let _ = crate::jogo::objetos::soltar_handle_contando(&mut h);
    area
}

/// Com o marcador de teste: na cópia, o objetivo (pai) do 1º `gameJournalQuestMapPin` cujo NodeRef resolve pra um nó de área no registro de instâncias
/// ganha o id `<id>_bwms` (inline, até 19 bytes), pra existir uma subárvore nova com pin de volume; os caminhos novo e original dos pins do objetivo ficam
/// em `JOURNAL_7E_TESTE_PINS`. Devolve `(objetivo, id original)`.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_7e_renomear(reg: Option<&Registry>, modc: *mut c_void, exigir_area: bool) -> Option<(usize, String)> {
    let pino = cname("gameJournalQuestMapPin");
    let familias = [pino, cname("gameJournalQuestMultiMapPin"), cname("gameJournalQuestPointOfInterestMapPin"), cname("gameJournalPointOfInterestMappin")];
    let mut pilha: Vec<(*mut c_void, String)> = vec![(modc, String::new())];
    let mut examinados = 0usize;
    while let Some((cont, caminho)) = pilha.pop() {
        let filhos = journal_filhos(cont);
        let id_cont = journal_id_de(cont);
        let cabe = !caminho.is_empty() && !id_cont.is_empty() && id_cont.len() + JOURNAL_7E_TESTE_SUFIXO.len() <= 19;
        for f in &filhos {
            if !cabe || examinados >= JOURNAL_7E_TESTE_BUSCA_MAX || !rtti::sane(*f) || rtti::type_name_hash(rtti::class_of(*f)) != pino {
                continue;
            }
            if !crate::memoria::is_readable(*f as *const c_void, JOURNAL_QUESTMAPPIN_REFERENCE_OFF + 0x20) {
                continue;
            }
            examinados += 1;
            let noderef = rd_u64(*f as *const c_void, JOURNAL_QUESTMAPPIN_REFERENCE_OFF);
            // Na raiz (7e-6b-2) o registro de nós ainda não existe: o objetivo de teste é o mesmo que a busca por nó de área achou nos boots 83–89.
            let escolhido = if exigir_area {
                reg.and_then(|r| resolver_noderef(r, noderef)).map_or(false, |id| no_de_area(id))
            } else {
                caminho == JOURNAL_7E6_TESTE_OBJETIVO
            };
            if noderef == 0 || !escolhido {
                continue;
            }
            let novo_id = format!("{id_cont}{JOURNAL_7E_TESTE_SUFIXO}");
            let base_novo = match caminho.rsplit_once('/') {
                Some((pai, _)) => format!("{pai}/{novo_id}"),
                None => novo_id.clone(),
            };
            let pins: Vec<(String, String)> = filhos
                .iter()
                .filter(|g| rtti::sane(**g) && familias.contains(&rtti::type_name_hash(rtti::class_of(**g))))
                .map(|g| {
                    let gid = journal_id_de(*g);
                    (format!("{base_novo}/{gid}"), format!("{caminho}/{gid}"))
                })
                .collect();
            if !rtti::red_string_write_inline((cont as *mut u8).add(JOURNAL_ENTRY_ID_OFF), &novo_id) {
                return None;
            }
            crate::veredito_dev!("[a44-7e3] objetivo de teste {caminho} → {novo_id}: {} pin(s), {examinados} pin(s) de quest examinado(s)", pins.len());
            if let Ok(mut t) = JOURNAL_7E_TESTE_PINS.lock() {
                *t = pins;
            }
            return Some((cont as usize, id_cont));
        }
        for f in filhos {
            if let Some(c) = journal_container_guardado(f) {
                let id = journal_id_de(f);
                if !id.is_empty() {
                    pilha.push((c, if caminho.is_empty() { id } else { format!("{caminho}/{id}") }));
                }
            }
        }
    }
    crate::veredito_dev!("[a44-7e3] nenhum objetivo de teste com pin de área ({examinados} pin(s) de quest examinado(s))");
    None
}

/// Troca o id da entrada: o antigo com mais de 19 bytes (heap) é destruído pelo tipo `String`; o novo fica inline até 19 bytes e, acima disso, sai do
/// construtor do motor.
pub(crate) unsafe fn journal_id_trocar(entrada: *mut c_void, novo: &str) -> bool {
    let slot = (entrada as *mut u8).add(JOURNAL_ENTRY_ID_OFF);
    let tipo = tipo_string_da_entrada(entrada);
    if !rtti::sane(tipo) {
        return false;
    }
    let construida = if novo.len() > 19 {
        match cstring_do_motor(novo) {
            Some(s) => Some(s),
            None => return false,
        }
    } else {
        None
    };
    if rtti::read_cstring(slot).map_or(false, |s| s.len() > 19) {
        destruir_string(tipo, slot);
        std::ptr::write_bytes(slot, 0, 0x20);
    }
    match construida {
        Some(s) => {
            std::ptr::copy_nonoverlapping(s.as_ptr(), slot, 0x20);
            true
        }
        None => rtti::red_string_write_inline(slot, novo),
    }
}

/// Propriedades da classe e dos pais que a edição copia, `(tipo, offset)`: todas menos o `id` e os `entries` do container.
pub(crate) unsafe fn journal_campos_editaveis(cls0: *mut c_void) -> Vec<(*mut c_void, usize)> {
    let fora = [cname("id"), cname("entries")];
    let mut v = Vec::new();
    let mut cls = cls0;
    let mut niveis = 0;
    while rtti::sane(cls) && niveis < 16 {
        for prop in rtti::class_own_properties(cls) {
            if !crate::memoria::is_readable(prop as *const c_void, 0x28) || fora.contains(&rd_u64(prop as *const c_void, 0x08)) {
                continue;
            }
            let tipo = rd_u64(prop as *const c_void, 0x00) as *mut c_void;
            if rtti::sane(tipo) {
                v.push((tipo, rd_u32(prop as *const c_void, 0x20) as usize));
            }
        }
        cls = rd_u64(cls as *const c_void, 0x10) as *mut c_void;
        niveis += 1;
    }
    v
}

/// Edição 7e-4 da entrada viva `vivo` pela `modo` do journal de mod: mesmo tipo, LocKeys da do mod convertidos, cada campo guardado numa cópia e copiado
/// pelo `Assign`. Devolve os campos copiados; `Err` -1 `Assign` não conferido, -2 tipos diferentes, -3 objeto ilegível, -4 cópia do valor anterior falhou.
pub(crate) unsafe fn journal_editar(modo: *mut c_void, vivo: *mut c_void, caminho: &str, j: &mut JournalDeMod) -> Result<usize, i32> {
    if !assign_confere(vivo) {
        return Err(-1);
    }
    let cls = rtti::class_of(vivo);
    if cls != rtti::class_of(modo) {
        return Err(-2);
    }
    let tamanho = rd_u32(cls as *const c_void, CCLASS_TAMANHO_OFF) as usize;
    if tamanho == 0 || !crate::memoria::is_readable(vivo as *const c_void, tamanho) || !crate::memoria::is_readable(modo as *const c_void, tamanho) {
        return Err(-3);
    }
    if let Ok((_, n)) = journal_converter_lockeys(modo) {
        j.lockeys += n;
    }
    let lista = journal_campos_editaveis(cls);
    let mut campos = Vec::with_capacity(lista.len());
    for (tipo, off) in &lista {
        let (copia, n) = rtti::type_inst_alloc(*tipo);
        if copia.is_null() || !rtti::type_assign(*tipo, copia, (vivo as *const u8).add(*off) as *const c_void) {
            rtti::type_inst_free(*tipo, copia, n);
            for (t, _, c, m) in campos.drain(..) {
                rtti::type_inst_free(t as *mut c_void, c as *mut c_void, m);
            }
            return Err(-4);
        }
        campos.push((*tipo as usize, *off, copia as usize, n));
    }
    for (tipo, off) in &lista {
        rtti::type_assign(*tipo, (vivo as *mut u8).add(*off) as *mut c_void, (modo as *const u8).add(*off) as *const c_void);
    }
    let n = campos.len();
    j.editadas.push(EditadaDeMod { alvo: vivo as usize, caminho: caminho.to_string(), campos });
    Ok(n)
}

/// Com o marcador de teste (7e-4): na cópia, fora do objetivo renomeado, o 1º `gameJournalQuestMapPin` com id inline (até 18 bytes) ganha o id `<id>*` e o
/// offset X + 1,5 (edição da entrada viva), e o 2º ganha `bwms_sem_caminho*` (controle: recusado sem anexar).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_7e4_marcar(modc: *mut c_void, evitar: Option<usize>) -> Option<TesteEdicao> {
    let pino = cname("gameJournalQuestMapPin");
    let mut pilha: Vec<(*mut c_void, String)> = vec![(modc, String::new())];
    let mut achados: Vec<(*mut c_void, String, String)> = Vec::new();
    'busca: while let Some((cont, caminho)) = pilha.pop() {
        if evitar == Some(cont as usize) {
            continue;
        }
        for f in journal_filhos(cont) {
            if !rtti::sane(f) {
                continue;
            }
            let id = journal_id_de(f);
            if id.is_empty() {
                continue;
            }
            let c = if caminho.is_empty() { id.clone() } else { format!("{caminho}/{id}") };
            if rtti::type_name_hash(rtti::class_of(f)) == pino && crate::memoria::is_readable(f as *const c_void, JOURNAL_QUESTMAPPIN_REFERENCE_OFF + 0x20) {
                if id.len() < 19 {
                    achados.push((f, id, c));
                }
                if achados.len() == 2 {
                    break 'busca;
                }
            } else if let Some(sub) = journal_container_guardado(f) {
                pilha.push((sub, c));
            }
        }
    }
    if achados.len() < 2 {
        crate::veredito_dev!("[a44-7e4] teste: menos de 2 pins de quest fora do objetivo renomeado");
        return None;
    }
    let (e, e_id, e_caminho) = achados.remove(0);
    let (c, c_id, c_caminho) = achados.remove(0);
    let original = f32x3(e as *const u8, JOURNAL_PINBASE_OFFSET_OFF);
    let editado_offset = [original[0] + JOURNAL_7E4_DELTA_X, original[1], original[2]];
    let slot_e = (e as *mut u8).add(JOURNAL_ENTRY_ID_OFF);
    if !rtti::red_string_write_inline(slot_e, &format!("{e_id}{JOURNAL_EDICAO_MARCA}")) {
        return None;
    }
    if !rtti::red_string_write_inline((c as *mut u8).add(JOURNAL_ENTRY_ID_OFF), JOURNAL_7E4_CONTROLE_ID) {
        let _ = rtti::red_string_write_inline(slot_e, &e_id);
        return None;
    }
    ((e as *mut u8).add(JOURNAL_PINBASE_OFFSET_OFF) as *mut f32).write_unaligned(editado_offset[0]);
    crate::veredito_dev!(
        "[a44-7e4] teste: {e_caminho} → {e_id}{JOURNAL_EDICAO_MARCA} com offset X {} → {}; controle {c_caminho} → {JOURNAL_7E4_CONTROLE_ID}",
        original[0], editado_offset[0]
    );
    Some(TesteEdicao { editado: e as usize, editado_id: e_id, controle: c as usize, controle_id: c_id, vivo: 0, original, editado_offset })
}


/// Pin da entrada nova: `gameJournalQuestMapPin` com NodeRef (entidade dinâmica fica de fora), `gameJournalQuestMultiMapPin` e
/// `gameJournalQuestPointOfInterestMapPin` só com offset, e `gameJournalPointOfInterestMappin` com o `staticNodeRef` na busca de POI.
pub(crate) unsafe fn journal_pin_de_mod(entrada: *mut c_void, caminho: &str) -> Option<PinDeModPendente> {
    let b = entrada as *const u8;
    let legivel = |n: usize| crate::memoria::is_readable(b as *const c_void, n);
    let classe = rtti::type_name_hash(rtti::class_of(entrada));
    let (poi, noderef, offset) = if classe == cname("gameJournalQuestMapPin") && legivel(JOURNAL_QUESTMAPPIN_REFERENCE_OFF + 0x20) {
        if rd_u64(b as *const c_void, JOURNAL_QUESTMAPPIN_REFERENCE_OFF + ENTITYREFERENCE_DINAMICA_OFF) != 0 {
            return None;
        }
        (false, rd_u64(b as *const c_void, JOURNAL_QUESTMAPPIN_REFERENCE_OFF), f32x3(b, JOURNAL_PINBASE_OFFSET_OFF))
    } else if (classe == cname("gameJournalQuestMultiMapPin") || classe == cname("gameJournalQuestPointOfInterestMapPin")) && legivel(JOURNAL_PINBASE_OFFSET_OFF + 12) {
        (false, 0, f32x3(b, JOURNAL_PINBASE_OFFSET_OFF))
    } else if classe == cname("gameJournalPointOfInterestMappin") && legivel(JOURNAL_POI_OFFSET_OFF + 12) {
        (true, rd_u64(b as *const c_void, JOURNAL_POI_NODEREF_OFF), f32x3(b, JOURNAL_POI_OFFSET_OFF))
    } else {
        return None;
    };
    Some(PinDeModPendente {
        hash: bwms_primitives::murmur3_32(caminho.as_bytes(), bwms_primitives::RECORDS_SEED),
        poi,
        noderef,
        offset,
        caminho: caminho.to_string(),
        tentativas: 0,
    })
}

/// `.journal` carregado: renomeia a entrada de teste (com o marcador), mescla na raiz viva 0, converte os LocKeys das novas e deixa os pins delas
/// pendentes. Devolve o estado novo.
pub(crate) unsafe fn journal_mod_mesclar(reg: &Registry, jm: *mut c_void, res: *mut c_void, j: &mut JournalDeMod) -> i32 {
    let Some(tree) = journal_tree_de(jm) else { return -4 };
    let Some(raiz_h) = journal_tree_raiz_handle(tree, 0) else { return -4 };
    let Some(vivo) = journal_container_de(raiz_h[0] as *mut c_void) else { return -4 };
    let Some(modc) = journal_resource_root(res).and_then(|r| journal_container_de(r)) else { return -5 };
    if modc == vivo {
        return -6;
    }
    if (crate::rebase(JOURNAL_ENTRY_MOVER_GOT_VM) as *const u64).read_unaligned() != crate::rebase(JOURNAL_ENTRY_MOVER_VM) as u64 {
        return -9;
    }
    if journal_slot(jm, 0x1e0) != crate::rebase(JOURNALMANAGER_REGISTER_VM) as usize || journal_slot(jm, 0x1e8) != crate::rebase(JOURNALMANAGER_UNREGISTER_VM) as usize {
        return -10;
    }
    #[cfg(feature = "devtools")]
    if crate::marcador_individual("journal-7e-teste") {
        j.teste_renomeado = journal_7e_renomear(Some(reg), modc, true);
        j.teste_edicao = journal_7e4_marcar(modc, j.teste_renomeado.as_ref().map(|t| t.0));
    }
    // O `reg` só serve à busca do objetivo do teste 7e.
    #[cfg(not(feature = "devtools"))]
    let _ = reg;
    let mut faltam = Vec::new();
    let mut editar = Vec::new();
    let mut recusadas = Vec::new();
    journal_mod_descer(modc, vivo, raiz_h, "", 0, &mut j.casadas, &mut faltam, &mut editar, &mut recusadas);
    for caminho in &recusadas {
        crate::veredito_dev!("[a44-7e4] {}: {caminho}: caminho não existe na árvore viva, entrada fora da mescla", j.mod_id);
    }
    j.edicoes_recusadas += recusadas.len();
    for (modo, vivo_e, caminho) in editar {
        match journal_editar(modo, vivo_e, &caminho, j) {
            Ok(n) => {
                #[cfg(feature = "devtools")]
                if let Some(t) = j.teste_edicao.as_mut().filter(|t| t.editado == modo as usize) {
                    t.vivo = vivo_e as usize;
                }
                crate::veredito_dev!("[a44-7e4] {}: editada {caminho} ({n} campo(s))", j.mod_id);
            }
            Err(e) => {
                j.edicoes_recusadas += 1;
                crate::veredito_dev!("[a44-7e4] {}: edição de {caminho} recusada -> {e}", j.mod_id);
            }
        }
    }
    let register: unsafe extern "C" fn(*mut c_void, *mut [u64; 2]) = std::mem::transmute(journal_slot(jm, 0x1e0));
    let mut pins = Vec::new();
    for (pasta, pasta_h, topo, caminho) in faltam {
        let mut dono = topo;
        let r = journal_anexar(jm, tree, pasta, pasta_h, &mut dono);
        if r != 1 {
            crate::veredito_dev!("[a44-7e] {}: anexar {caminho} -> {r}", j.mod_id);
            continue;
        }
        let desc = journal_descendentes(topo, &caminho);
        for (filho, pai, _) in &desc {
            journal_pai_inserir(tree, *filho, *pai);
            let mut d = *filho;
            register(jm, &mut d);
        }
        for (e, c) in std::iter::once((topo, caminho.clone())).chain(desc.iter().map(|(f, _, c)| (*f, c.clone()))) {
            let obj = e[0] as *mut c_void;
            if let Ok((_, n)) = journal_converter_lockeys(obj) {
                j.lockeys += n;
            }
            if let Some(p) = journal_pin_de_mod(obj, &c) {
                pins.push(p);
            }
            j.entradas_novas += 1;
        }
        crate::veredito_dev!("[a44-7e] {}: anexada {caminho} com {} descendente(s)", j.mod_id, desc.len());
        j.anexadas.push(AnexadaDeMod { topo, pasta: pasta as usize, caminho, descendentes: desc.iter().map(|(f, _, _)| *f).collect() });
    }
    j.pins_novos += pins.len();
    if let Ok(mut p) = PINS_DE_MOD.lock() {
        p.extend(pins);
    }
    #[cfg(feature = "devtools")]
    let teste_renomeado = j.teste_renomeado.is_some();
    #[cfg(not(feature = "devtools"))]
    let teste_renomeado = false;
    crate::veredito_dev!(
        "[a44-7e] {} mesclado: {} casada(s), {} anexada(s), {} entrada(s) nova(s), {} pin(s), {} LocKey(s) convertido(s), {} editada(s), {} recusada(s), teste renomeado={}",
        j.caminho,
        j.casadas,
        j.anexadas.len(),
        j.entradas_novas,
        j.pins_novos,
        j.lockeys,
        j.editadas.len(),
        j.edicoes_recusadas,
        teste_renomeado
    );
    2
}

/// Avança os journals de mod da tabela (game thread, gameplay assentado), a cada 30 ticks: pede a carga quando o caminho existe no depot (um archive
/// registrado em runtime entra depois do load), espera o token e mescla; depois registra os pins pendentes.
pub(crate) unsafe fn journal_mods_tick(reg: &Registry, player: *mut c_void) {
    static VOLTAS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if !crate::tempo::a_cada(u64::from(VOLTAS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)), 30) {
        return;
    }
    pins_de_mod_tick(reg);
    let Ok(mut g) = JOURNAL_MODS.try_lock() else { return };
    if !g.iter().any(|j| j.estado == 0 || j.estado == 1) {
        return;
    }
    let guardado = JOURNAL_MODS_JM.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void;
    let jm = if journal_tree_de(guardado).is_some() {
        guardado
    } else {
        let Some(jm) = journal_manager_do_jogo(reg, player) else { return };
        JOURNAL_MODS_JM.store(jm as usize, std::sync::atomic::Ordering::Relaxed);
        jm
    };
    for j in g.iter_mut() {
        let antes = j.estado;
        match j.estado {
            0 => {
                if !crate::resource_exists(&j.caminho) {
                    continue;
                }
                j.token = crate::arquivos_do_jogo::carregar_e_guardar(bwms_primitives::resource_path_hash(&j.caminho));
                j.estado = if j.token == 0 { -1 } else { 1 };
            }
            1 => match crate::arquivos_do_jogo::token_guardado(j.token) {
                None => j.estado = -2,
                Some(t) => match status_do_token(t) {
                    0 => {}
                    1 => {
                        let res = ((t + 0x28) as *const u64).read_unaligned() as *mut c_void;
                        j.estado = journal_mod_mesclar(reg, jm, res, j);
                    }
                    _ => j.estado = -3,
                },
            },
            _ => {}
        }
        if j.estado != antes {
            crate::veredito_dev!("[a44-7e] {} {}: estado {antes} -> {}", j.mod_id, j.caminho, j.estado);
        }
    }
}

// Journal de mod, parte 7e-6b-1 (2026-09-12): mescla antecipada, só observando. Com `~/.bwms-hook-journal-menu`, o tick do menu (sem player) pede a carga dos
// journals da tabela assim que o caminho existe, e cada journal carregado vira um plano: id da raiz do mod, ids de topo e total de entradas. O hook do
// `Initialize` da raiz (7e-6a, thread de job) compara o plano com as entradas de topo da raiz viva só lendo memória e loga quantas casariam e quantas seriam
// novas. A observação não escreve nada. Com `~/.bwms-hook-journal-mescla-na-raiz` o mesmo hook mescla na raiz (7e-6b-2, `journal_mescla_na_raiz`);
// sem ele a mescla fica no tick de gameplay (7e-2), a partir do token pedido no menu.
/// Plano de um journal de mod carregado no menu. A chave é o `caminho` do `.journal`: as linhas de um `.xl` com 2 journals levam o mesmo `mod_id`
/// (`emit_journal_table`, bwms-primitives/src/instalacao/tabelas_de_runtime.rs, faz um laço em `xl.journals`), e `journal_tabela` também separa os journals pelo caminho.
#[cfg(any(test, feature = "devtools"))]
pub(crate) struct PlanoDeRaiz {
    pub(crate) mod_id: String,
    pub(crate) caminho: String,
    pub(crate) raiz_id: String,
    pub(crate) topo: Vec<String>,
    pub(crate) entradas: usize,
}

#[cfg(feature = "devtools")]
pub(crate) static JOURNAL_PLANOS: std::sync::Mutex<Vec<PlanoDeRaiz>> = std::sync::Mutex::new(Vec::new());

/// O journal `caminho` já tem plano.
#[cfg(any(test, feature = "devtools"))]
pub(crate) fn plano_montado(planos: &[PlanoDeRaiz], caminho: &str) -> bool {
    planos.iter().any(|p| p.caminho == caminho)
}

/// Caminhos dos journals com plano da raiz `raiz_id`: só estes entram na mescla dessa raiz.
#[cfg(any(test, feature = "devtools"))]
pub(crate) fn caminhos_com_plano_da_raiz(planos: &[PlanoDeRaiz], raiz_id: &str) -> Vec<String> {
    planos.iter().filter(|p| p.raiz_id == raiz_id).map(|p| p.caminho.clone()).collect()
}

/// Classes do journal resolvidas na game thread (carga das tabelas): o `Initialize` da raiz roda em thread de job e não pode perguntar ao RTTI pelo nome.
pub(crate) static JOURNAL_CLASSE_RECURSO: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(crate) static JOURNAL_CLASSE_CONTAINER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Teto da espera pela carga de um journal de mod dentro do `Initialize` da raiz.
#[cfg(feature = "devtools")]
pub(crate) const JOURNAL_PLANO_ESPERA_MS: u64 = 3000;

/// Guarda as classes `gameJournalResource` e `gameJournalContainerEntry` (game thread).
pub(crate) unsafe fn journal_classes_guardar() {
    if let Some(reg) = crate::registry() {
        JOURNAL_CLASSE_RECURSO.store(reg.class_by_name("gameJournalResource") as usize, std::sync::atomic::Ordering::Relaxed);
        JOURNAL_CLASSE_CONTAINER.store(reg.class_by_name("gameJournalContainerEntry") as usize, std::sync::atomic::Ordering::Relaxed);
    }
}

/// `inst` se for container, pela classe guardada: só memória e o `GetType` do objeto, então vale em thread de job.
pub(crate) unsafe fn journal_container_guardado(inst: *mut c_void) -> Option<*mut c_void> {
    let alvo = JOURNAL_CLASSE_CONTAINER.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void;
    if alvo.is_null() || !rtti::sane(inst) || !crate::memoria::is_readable(inst as *const c_void, JOURNAL_CONTAINER_ENTRIES_OFF + 0x10) {
        return None;
    }
    class_is_kind_of(rtti::class_of(inst), alvo).then_some(inst)
}

/// Plano do journal `j`, carregado em `res`: raiz, ids de topo e total de entradas da árvore. Só memória e as classes guardadas (game thread ou thread de
/// job).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_plano(j: &JournalDeMod, res: *mut c_void) -> Option<PlanoDeRaiz> {
    let alvo = JOURNAL_CLASSE_RECURSO.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void;
    if alvo.is_null() || !rtti::sane(res) || !crate::memoria::is_readable(res as *const c_void, JOURNAL_RESOURCE_ENTRY_OFF + 0x10) || !class_is_kind_of(rtti::class_of(res), alvo) {
        return None;
    }
    let raiz = journal_container_guardado(rd_u64(res as *const c_void, JOURNAL_RESOURCE_ENTRY_OFF) as *mut c_void)?;
    let topo: Vec<String> = journal_filhos(raiz).into_iter().filter(|f| rtti::sane(*f)).map(|f| journal_id_de(f)).collect();
    let mut entradas = 0usize;
    let mut pilha = vec![raiz];
    while let Some(c) = pilha.pop() {
        for f in journal_filhos(c) {
            if !rtti::sane(f) {
                continue;
            }
            entradas += 1;
            if let Some(sub) = journal_container_guardado(f) {
                pilha.push(sub);
            }
        }
        if entradas > 200_000 {
            break;
        }
    }
    Some(PlanoDeRaiz { mod_id: j.mod_id.clone(), caminho: j.caminho.clone(), raiz_id: journal_id_de(raiz), topo, entradas })
}

/// No hook do `Initialize` da raiz (thread de job, só memória): espera, com teto, a carga já pedida dos journals da tabela e monta o plano dos carregados.
/// Devolve `(ms esperados, planos montados)`. No boot 88 o plano saiu depois das 2 chamadas do `Initialize`, que roda na carga inicial, antes do player do
/// menu: a carga pedida na 1ª volta do tick ainda não tinha terminado.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_planos_na_hora() -> (u128, usize) {
    let Ok(g) = JOURNAL_MODS.try_lock() else { return (0, 0) };
    let inicio = std::time::Instant::now();
    let mut montados = 0;
    for j in g.iter().filter(|j| j.estado == 1) {
        let Some(t) = crate::arquivos_do_jogo::token_guardado(j.token) else { continue };
        while status_do_token(t) == 0 && inicio.elapsed() < std::time::Duration::from_millis(JOURNAL_PLANO_ESPERA_MS) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if status_do_token(t) != 1 {
            continue;
        }
        let Ok(mut planos) = JOURNAL_PLANOS.try_lock() else { continue };
        if plano_montado(&planos, &j.caminho) {
            continue;
        }
        let res = ((t + 0x28) as *const u64).read_unaligned() as *mut c_void;
        if let Some(p) = journal_plano(j, res) {
            planos.push(p);
            montados += 1;
        }
    }
    (inicio.elapsed().as_millis(), montados)
}

/// Tick de carga antecipada (todo tick, desde o menu), com `~/.bwms-hook-journal-menu`: pede a carga dos journals da tabela e monta o plano de cada carregado.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_mods_menu_tick() {
    static VOLTAS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if !crate::tempo::a_cada(u64::from(VOLTAS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)), 30) || !crate::marcador_individual("journal-menu") {
        return;
    }
    let Ok(mut g) = JOURNAL_MODS.try_lock() else { return };
    for j in g.iter_mut() {
        if j.estado == 0 && crate::resource_exists(&j.caminho) {
            j.token = crate::arquivos_do_jogo::carregar_e_guardar(bwms_primitives::resource_path_hash(&j.caminho));
            j.estado = if j.token == 0 { -1 } else { 1 };
            crate::veredito_dev!("[a44-7e6] {} {}: carga pedida no menu -> estado {}", j.mod_id, j.caminho, j.estado);
            continue;
        }
        if j.estado != 1 {
            continue;
        }
        let Some(t) = crate::arquivos_do_jogo::token_guardado(j.token) else { continue };
        if status_do_token(t) != 1 {
            continue;
        }
        let Ok(mut planos) = JOURNAL_PLANOS.try_lock() else { continue };
        if plano_montado(&planos, &j.caminho) {
            continue;
        }
        let res = ((t + 0x28) as *const u64).read_unaligned() as *mut c_void;
        if let Some(p) = journal_plano(j, res) {
            crate::veredito_dev!(
                "[a44-7e6] plano de {} {}: raiz '{}', {} entrada(s) de topo, {} entrada(s) na árvore; phase 5 já visto={}, player={}",
                p.mod_id,
                p.caminho,
                p.raiz_id,
                p.topo.len(),
                p.entradas,
                crate::inicio::boot::PHASE5_AT.get().is_some(),
                !crate::current_player().is_null()
            );
            planos.push(p);
        }
    }
}

/// Objetivo de teste da mescla na raiz (7e-6b-2): o que a busca pelo 1º pin de quest com nó de área escolheu nos boots 83–89, fixado porque o registro de
/// nós ainda não existe no `Initialize`.
#[cfg(feature = "devtools")]
pub(crate) const JOURNAL_7E6_TESTE_OBJETIVO: &str = "quests/street_stories/wesbrook/japan_town/sts_wbr_jpn_01/sts_wbr_jpn_01/meet";

/// 7e-6b-2: mescla do journal de mod carregado `res` na raiz viva `raiz`, dentro do `Initialize` dela (thread de job, antes da original). Com o marcador,
/// marca os testes na cópia; desce casando por id, edita as marcadas com `*` e anexa as novas só nos `entries` da pasta viva — o `Initialize` de cada
/// entrada, que roda depois, faz o vínculo com o pai e o índice (RE do boot 88). LocKeys e pins das novas como na mescla do tick. Devolve o estado novo:
/// 2, ou -5 sem raiz no recurso, -6 raiz do recurso é a viva, -9 mover no GOT.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_mod_mesclar_na_raiz(raiz: *mut c_void, res: *mut c_void, j: &mut JournalDeMod) -> i32 {
    let alvo = JOURNAL_CLASSE_RECURSO.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void;
    if alvo.is_null() || !rtti::sane(res) || !crate::memoria::is_readable(res as *const c_void, JOURNAL_RESOURCE_ENTRY_OFF + 0x10) || !class_is_kind_of(rtti::class_of(res), alvo) {
        return -5;
    }
    let Some(modc) = journal_container_guardado(rd_u64(res as *const c_void, JOURNAL_RESOURCE_ENTRY_OFF) as *mut c_void) else { return -5 };
    if modc == raiz {
        return -6;
    }
    if (crate::rebase(JOURNAL_ENTRY_MOVER_GOT_VM) as *const u64).read_unaligned() != crate::rebase(JOURNAL_ENTRY_MOVER_VM) as u64 {
        return -9;
    }
    if crate::marcador_individual("journal-7e-teste") {
        j.teste_renomeado = journal_7e_renomear(None, modc, false);
        j.teste_edicao = journal_7e4_marcar(modc, j.teste_renomeado.as_ref().map(|t| t.0));
    }
    let mut faltam = Vec::new();
    let mut editar = Vec::new();
    let mut recusadas = Vec::new();
    journal_mod_descer(modc, raiz, [raiz as u64, 0], "", 0, &mut j.casadas, &mut faltam, &mut editar, &mut recusadas);
    j.edicoes_recusadas += recusadas.len();
    for (modo, vivo_e, caminho) in editar {
        match journal_editar(modo, vivo_e, &caminho, j) {
            Ok(_) => {
                if let Some(t) = j.teste_edicao.as_mut().filter(|t| t.editado == modo as usize) {
                    t.vivo = vivo_e as usize;
                }
            }
            Err(e) => {
                j.edicoes_recusadas += 1;
                crate::veredito_dev!("[a44-7e6] {}: edição de {caminho} recusada na raiz -> {e}", j.mod_id);
            }
        }
    }
    let mut pins = Vec::new();
    for (pasta, _, topo, caminho) in faltam {
        let r = journal_anexar_so_entries(pasta, topo);
        if r != 1 {
            crate::veredito_dev!("[a44-7e6] {}: anexar {caminho} na raiz -> {r}", j.mod_id);
            continue;
        }
        let desc = journal_descendentes(topo, &caminho);
        for (e, c) in std::iter::once((topo, caminho.clone())).chain(desc.iter().map(|(f, _, c)| (*f, c.clone()))) {
            let obj = e[0] as *mut c_void;
            if let Ok((_, n)) = journal_converter_lockeys(obj) {
                j.lockeys += n;
            }
            if let Some(p) = journal_pin_de_mod(obj, &c) {
                pins.push(p);
            }
            j.entradas_novas += 1;
        }
        j.anexadas.push(AnexadaDeMod { topo, pasta: pasta as usize, caminho, descendentes: desc.iter().map(|(f, _, _)| *f).collect() });
    }
    j.pins_novos += pins.len();
    if let Ok(mut p) = PINS_DE_MOD.lock() {
        p.extend(pins);
    }
    j.na_raiz = true;
    2
}

/// 7e-6b-2, do hook do `Initialize` da raiz (thread de job): mescla na raiz `raiz_id` os journals da tabela com plano desta raiz e token carregado. Texto pro
/// log.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_mescla_na_raiz(raiz: *mut c_void, raiz_id: &str) -> String {
    let caminhos = match JOURNAL_PLANOS.try_lock() {
        Ok(p) => caminhos_com_plano_da_raiz(&p, raiz_id),
        Err(_) => return "mescla: planos ocupados".to_string(),
    };
    let Ok(mut g) = JOURNAL_MODS.try_lock() else { return "mescla: journals ocupados".to_string() };
    let mut textos = Vec::new();
    for j in g.iter_mut().filter(|j| j.estado == 1 && caminhos.contains(&j.caminho)) {
        let Some(t) = crate::arquivos_do_jogo::token_guardado(j.token) else { continue };
        if status_do_token(t) != 1 {
            continue;
        }
        let res = ((t + 0x28) as *const u64).read_unaligned() as *mut c_void;
        let antes = j.estado;
        j.estado = journal_mod_mesclar_na_raiz(raiz, res, j);
        textos.push(format!(
            "mescla {} {}: estado {antes} -> {}, {} casada(s), {} anexada(s), {} entrada(s) nova(s), {} pin(s), {} editada(s), {} recusada(s), teste renomeado={}",
            j.mod_id,
            j.caminho,
            j.estado,
            j.casadas,
            j.anexadas.len(),
            j.entradas_novas,
            j.pins_novos,
            j.editadas.len(),
            j.edicoes_recusadas,
            j.teste_renomeado.is_some()
        ));
    }
    if textos.is_empty() {
        "mescla: nada pra esta raiz".to_string()
    } else {
        textos.join("; ")
    }
}

/// Do hook do `Initialize` da raiz (thread de job, só memória): compara os planos da raiz `raiz_id` com as entradas de topo da raiz viva. Texto pro log.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_plano_na_raiz(raiz: *mut c_void, raiz_id: &str) -> String {
    let (espera_ms, na_hora) = if crate::marcador_individual("journal-menu") { journal_planos_na_hora() } else { (0, 0) };
    let espera = format!("esperou {espera_ms} ms, {na_hora} plano(s) montado(s) na hora");
    let Ok(planos) = JOURNAL_PLANOS.try_lock() else { return format!("planos ocupados; {espera}") };
    let doesta: Vec<&PlanoDeRaiz> = planos.iter().filter(|p| p.raiz_id == raiz_id).collect();
    if doesta.is_empty() {
        return format!("nenhum plano desta raiz ({} no total); {espera}", planos.len());
    }
    let vivos = journal_ids_mapa(raiz);
    let comparacao = doesta
        .iter()
        .map(|p| {
            let casadas = p.topo.iter().filter(|i| vivos.contains_key(*i)).count();
            format!("{} {}: {} de topo casariam, {} seriam novas, {} entradas no mod", p.mod_id, p.caminho, casadas, p.topo.len() - casadas, p.entradas)
        })
        .collect::<Vec<_>>()
        .join("; ");
    drop(planos);
    let mescla = if crate::marcador_individual("journal-mescla-na-raiz") { format!("; {}", journal_mescla_na_raiz(raiz, raiz_id)) } else { String::new() };
    format!("{comparacao}; {espera}{mescla}")
}

/// Posição da instância do nó do id (`FindNode`); `None` fora do registro.
pub(crate) unsafe fn posicao_do_no(id: u64) -> Option<[f32; 3]> {
    let mut h = find_node(id)?;
    let obj = h[0] as *const u8;
    if obj.is_null() {
        return None;
    }
    let p = crate::memoria::is_readable(obj as *const c_void, 0x90).then(|| f32x3(obj, NODEINSTANCE_POSICAO_OFF));
    let _ = crate::jogo::objetos::soltar_handle_contando(&mut h);
    p
}

/// Volume do pin de mod a partir do nó de área da instância do id, como o cozido vanilla (7d-2b-3): altura do outline sem escala, pontos escalados e
/// girados só em XY. Objeto pelo `CreateInstance` real (o motor solta pelo allocator da classe), `DynArray<Vector2>` pelo reserve do motor, Handle com 1
/// strong que passa pra tabela de mappins de mod. `[0, 0]` sem instância, sem nó de área ou com a classe/vptr diferente da RE.
pub(crate) unsafe fn volume_de_mod(reg: &Registry, id: u64) -> [u64; 2] {
    let Some(mut h) = find_node(id) else { return [0, 0] };
    let inst = h[0] as *const u8;
    if inst.is_null() {
        return [0, 0];
    }
    let legivel = |q: *const u8, n: usize| rtti::sane(q as *mut c_void) && crate::memoria::is_readable(q as *const c_void, n);
    let mut resultado = [0u64; 2];
    'monta: {
        if !legivel(inst, 0x90) {
            break 'monta;
        }
        let no = (inst.add(NODEINSTANCE_NO_OFF) as *const u64).read_unaligned() as *const u8;
        if !legivel(no, 0x60) || !acessor_e_da_classe(no, "worldAreaShapeNode") {
            break 'monta;
        }
        let outline = (no.add(AREASHAPENODE_OUTLINE_OFF) as *const u64).read_unaligned() as *const u8;
        if !legivel(outline, 0x48) {
            break 'monta;
        }
        let (ptr, cap, n) = handles_arr_em(outline, AREASHAPEOUTLINE_PONTOS_OFF);
        if ptr.is_null() || n == 0 || n > cap || n > 4096 || !crate::memoria::is_readable(ptr as *const c_void, n as usize * 12) {
            break 'monta;
        }
        let vol = rtti::create_instance(reg.class_by_name("gamemappinsOutlineMappinVolume")) as *mut u8;
        if vol.is_null() || !legivel(vol, 0x48) || (vol as *const u64).read_unaligned() != crate::rebase(OUTLINEVOLUME_VPTR_VM) as u64 {
            break 'monta;
        }
        let mut dono = [0u64; 2];
        rtti::make_handle(dono.as_mut_ptr() as *mut c_void, vol as *mut c_void);
        let arr = vol.add(OUTLINEVOLUME_PONTOS_OFF);
        crate::jogo::listas::reservar(arr as *mut c_void, n, 8, 4, std::ptr::null());
        let dados = (arr as *const u64).read_unaligned() as *mut f32;
        if dados.is_null() || (arr.add(0x08) as *const u32).read_unaligned() < n {
            let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono);
            break 'monta;
        }
        let escala = f32x3(inst, NODEINSTANCE_ESCALA_OFF);
        let quat = [0, 4, 8, 12].map(|d| (inst.add(NODEINSTANCE_QUAT_OFF + d) as *const f32).read_unaligned());
        (vol.add(OUTLINEVOLUME_ALTURA_OFF) as *mut f32).write_unaligned((outline.add(AREASHAPEOUTLINE_ALTURA_OFF) as *const f32).read_unaligned());
        for i in 0..n as usize {
            let p = f32x3(ptr as *const u8, i * 12);
            let q = crate::mundo::quat_rot(quat, [p[0] * escala[0], p[1] * escala[1], p[2] * escala[2]]);
            dados.add(i * 2).write_unaligned(q[0]);
            dados.add(i * 2 + 1).write_unaligned(q[1]);
        }
        (arr.add(0x0C) as *mut u32).write_unaligned(n);
        resultado = dono;
    }
    let _ = crate::jogo::objetos::soltar_handle_contando(&mut h);
    resultado
}

/// Registra até `PINS_POR_TICK` pins pendentes na tabela de mappins de mod do `MappinSystem` que o hook do `OnStreamingWorldLoaded` guardou: posição do
/// nó do NodeRef ou o offset sem NodeRef. Nó fora do registro, sistema ausente ou busca sem hook (-1/-2) ficam pra depois; hash já cozido (-3) sai.
pub(crate) unsafe fn pins_de_mod_tick(reg: &Registry) {
    let sys = crate::missoes_e_mapa::MAPPIN_SISTEMA.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void;
    if sys.is_null() {
        return;
    }
    let Ok(mut g) = PINS_DE_MOD.try_lock() else { return };
    if g.is_empty() {
        return;
    }
    let mut feitos = 0;
    g.retain_mut(|p| {
        if feitos >= PINS_POR_TICK {
            return true;
        }
        feitos += 1;
        let id = if p.noderef == 0 { None } else { resolver_noderef(reg, p.noderef) };
        let posicao = if p.noderef == 0 { Some(p.offset) } else { id.and_then(|id| posicao_do_no(id)) };
        let Some(pos) = posicao else {
            p.tentativas += 1;
            return true;
        };
        let volume = match (p.poi, id) {
            (false, Some(id)) => volume_de_mod(reg, id),
            _ => [0, 0],
        };
        let com_volume = volume[0] != 0;
        let r = crate::missoes_e_mapa::mappin_de_mod_registrar(busca_do_codigo(if p.poi { 10 } else { 0 }), sys, p.hash, Some(pos), volume);
        crate::veredito_dev!(
            "[a44-7e3] pin {} ({:#x}, {}) na posição {pos:?}, volume {com_volume} -> {r}",
            p.caminho,
            p.hash,
            if p.poi { "POI" } else { "mappin" }
        );
        if r == 1 {
            if let Ok(mut reg_pins) = PINS_DE_MOD_REGISTRADOS.lock() {
                reg_pins.push((p.hash, p.poi));
            }
        }
        r == -1 || r == -2
    });
}






// Journal de mod, partes 7a/7b (2026-09-12): natives do teste da busca de mappin cozido (o hook e a tabela ficam no `selftest.rs`).
/// A busca de um código de ação/eixo: de 10 em diante é a de POI (7c); abaixo, a de mappin (7b).
pub(crate) fn busca_do_codigo(codigo: i32) -> &'static crate::missoes_e_mapa::BuscaCozida {
    if codigo >= 10 { &crate::missoes_e_mapa::BUSCA_POI } else { &crate::missoes_e_mapa::BUSCA_MAPPIN }
}




// Journal de mod, parte 7d-2b-1 (2026-09-12, RE offline da coleta de mappins do journal): coleta dos pins de
// um `.journal` carregado por caminho, só leitura. A árvore do recurso é percorrida montando o caminho de ids (sem o id da raiz); nas famílias de
// pin — `gameJournalQuestMapPin`, `gameJournalQuestMultiMapPin` e `gameJournalQuestPointOfInterestMapPin` (base `QuestMapPinBase`) e
// `gameJournalPointOfInterestMappin` — guarda o hash Murmur3 do caminho (seed `0x5EEDBA5E`, o que o motor consulta), o NodeRef, a entidade dinâmica e o
// `offset`, pelos offsets do registro RTTI. Critério: o censo offline do `cooked_journal`; a busca cozida vanilla confere o hash de 1 pin e 1 POI.
pub(crate) const JOURNAL_PINBASE_OFFSET_OFF: usize = 0x108;

pub(crate) const JOURNAL_QUESTMAPPIN_REFERENCE_OFF: usize = 0x120;

pub(crate) const ENTITYREFERENCE_DINAMICA_OFF: usize = 0x18;

pub(crate) const JOURNAL_POI_NODEREF_OFF: usize = 0x70;

pub(crate) const JOURNAL_POI_OFFSET_OFF: usize = 0x100;



















pub(crate) const OUTLINEVOLUME_VPTR_VM: u64 = 0x1_07a9_d098;

pub(crate) const OUTLINEVOLUME_ALTURA_OFF: usize = 0x30;

pub(crate) const OUTLINEVOLUME_PONTOS_OFF: usize = 0x38;

























/// Monta um `QuestContext` do `QuestsSystem` vivo em `ctx` (buffer de `QUESTCTX_TAM` do chamador) com os campos que o próprio sistema passa.
pub(crate) unsafe fn questctx_montar(ctx: *mut c_void, sys: *mut c_void) {
    let s = sys as u64;
    let ctor: extern "C" fn(*mut c_void, u32, u32, i32, i32, u64, u64, u64, u64, u64, u64, u64) = std::mem::transmute(crate::rebase(QUESTCTX_CTOR_VM));
    ctor(ctx, 1, 0, -1, -1, rd_u64(sys, 0xe8), s + 0xa8, s + 0xb8, s + 0x78, s + 0x1d0, rd_u64(sys, 0x2c8), rd_u64(sys, 0x40));
}

/// Destrói o `QuestContext` montado por [`questctx_montar`].
pub(crate) unsafe fn questctx_destruir(ctx: *mut c_void) {
    let dtor: extern "C" fn(*mut c_void) = std::mem::transmute(crate::rebase(QUESTCTX_DTOR_VM));
    dtor(ctx);
}





pub(crate) unsafe fn quests_trava(sys: *mut c_void) -> &'static std::sync::atomic::AtomicU8 {
    &*((sys as *mut u8).add(QUESTS_TRAVA_OFF) as *const std::sync::atomic::AtomicU8)
}

/// Trava de escrita do `QuestsSystem` como a `0x1000015f0` do motor, mas com teto; `false` = não conseguiu.
pub(crate) unsafe fn quests_travar(sys: *mut c_void) -> bool {
    let t = quests_trava(sys);
    for i in 0..QUESTS_TRAVA_TENTATIVAS {
        if t.compare_exchange(0, QUESTS_TRAVA_ESCRITA, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            return true;
        }
        std::hint::spin_loop();
        if i % 256 == 255 {
            std::thread::yield_now();
        }
    }
    false
}

pub(crate) unsafe fn quests_soltar(sys: *mut c_void) {
    quests_trava(sys).fetch_and(!QUESTS_TRAVA_ESCRITA, Ordering::Release);
}


/// `ExecuteNode` do motor num nó qualquer, na fase raiz e com a trava do sistema. `Err`: -1 sistema, -2 fase raiz, -3 não é nó de quest, -5 trava ocupada.
pub(crate) unsafe fn quests_execute_node(sys: *mut c_void, no: *mut c_void, socket: u64) -> Result<QuestsExecucao, i32> {
    quests_execute_node_com(sys, no, socket, |_| true)
}

/// `quests_execute_node` com `guarda(ctx)` conferindo o `QuestContext` montado antes da execução; `Err(-6)` quando ela recusa.
pub(crate) unsafe fn quests_execute_node_com(sys: *mut c_void, no: *mut c_void, socket: u64, guarda: fn(*const u8) -> bool) -> Result<QuestsExecucao, i32> {
    if !rtti::sane(sys) || !crate::memoria::is_readable(sys as *const c_void, 0x2d0) {
        return Err(-1);
    }
    let fase = rd_u64(sys, 0x68) as *mut c_void;
    if !rtti::sane(fase) {
        return Err(-2);
    }
    if !rtti::sane(no) || !classe_deriva_de_nome(rtti::class_of(no), crate::cname::cname("questNodeDefinition")) {
        return Err(-3);
    }
    let mut buf = BufQuestCtx([0u8; QUESTCTX_TAM]);
    let ctx = buf.0.as_mut_ptr() as *mut c_void;
    let empilha: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(crate::rebase(QUESTCTX_EMPILHA_VM));
    let desempilha: extern "C" fn(*mut c_void) -> *mut c_void = std::mem::transmute(crate::rebase(QUESTCTX_DESEMPILHA_VM));
    questctx_montar(ctx, sys);
    empilha(ctx, fase);
    let pilha_empilhada = rd_u32(ctx, 0x104);
    if !guarda(ctx as *const u8) {
        desempilha(ctx);
        questctx_destruir(ctx);
        return Err(-6);
    }
    if !quests_travar(sys) {
        desempilha(ctx);
        questctx_destruir(ctx);
        return Err(-5);
    }
    let mut sock = [0u8; 16];
    sock[..8].copy_from_slice(&socket.to_le_bytes());
    // `DynArray` vazio: {entries, capacity, size}.
    let mut saida = [0u64; 2];
    let execute_node: extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> u32 =
        std::mem::transmute(crate::rebase(EXECUTENODE_VM));
    let resultado = execute_node(fase, no, ctx, sock.as_mut_ptr() as *mut c_void, saida.as_mut_ptr() as *mut c_void) & 0xff;
    quests_soltar(sys);
    desempilha(ctx);
    let pilha_depois = rd_u32(ctx, 0x104);
    questctx_destruir(ctx);
    let (entries, size, cap) = dynarray_em(saida.as_ptr() as *const c_void);
    let mut saidas = Vec::new();
    if entries != 0 && dynarray_coerente(entries, size, cap) {
        for i in 0..size.min(64) as usize {
            saidas.push(((entries + i * 0x10) as *const u64).read_unaligned());
        }
    }
    if cap != 0 {
        // Elemento é `{CName, u8}`, sem destrutor: zera o tamanho e devolve o buffer pelo `realloc` do motor com capacidade 0.
        core::ptr::write_unaligned((saida.as_mut_ptr() as *mut u8).add(0x0c) as *mut u32, 0);
        crate::jogo::listas::reservar(saida.as_mut_ptr() as *mut c_void, 0, 0x10, 8, std::ptr::null_mut());
    }
    Ok(QuestsExecucao { resultado, saidas, pilha_empilhada, pilha_depois })
}



pub(crate) unsafe fn register_quest_accessor_natives(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsQuestPhaseResourceGraph", "BwmsQuestPhaseResourceGraph", tramp_questphase_resource_graph as NativeHandler);
    let ok2 = register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "CName"], "BwmsQuestAccessorCount", "BwmsQuestAccessorCount", tramp_quest_accessor_count as NativeHandler);
    let ok3 = register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "CName", "Int32"], "BwmsQuestAccessorAt", "BwmsQuestAccessorAt", tramp_quest_accessor_at as NativeHandler);
    let ok4 = register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "CName"], "BwmsQuestAccessorFindCount", "BwmsQuestAccessorFindCount", tramp_quest_accessor_find_count as NativeHandler);
    let ok5 = register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "CName", "Int32"], "BwmsQuestAccessorFindAt", "BwmsQuestAccessorFindAt", tramp_quest_accessor_find_at as NativeHandler);
    let ok6 = register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsQuestAccessorPathCount", "BwmsQuestAccessorPathCount", tramp_quest_accessor_path_count as NativeHandler);
    let ok7 = register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32"], "BwmsQuestAccessorPathDepth", "BwmsQuestAccessorPathDepth", tramp_quest_accessor_path_depth as NativeHandler);
    let ok8 = register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "Int32"], "BwmsQuestAccessorPathId", "BwmsQuestAccessorPathId", tramp_quest_accessor_path_id as NativeHandler);
    let ok9 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "CName", "Int32"], "BwmsQuestAccessorSyntheticTest", "BwmsQuestAccessorSyntheticTest", corpo_de_prova!(tramp_quest_accessor_synthetic_test)));
    // Fase de quest, 35c-2d-3b: loader de quest do QuestsSystem (fila da rota de carga de fase).
    let ok_fila_op = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "Int32"], "BwmsQuestLoaderOp", "BwmsQuestLoaderOp", corpo_de_prova!(tramp_questloader_op)));
    let ok_fila_res = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32"], "BwmsQuestLoaderResource", "BwmsQuestLoaderResource", corpo_de_prova!(tramp_questloader_resource)));
    let ok_inicio = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsQuestGraphStartNode", "BwmsQuestGraphStartNode", corpo_de_prova!(tramp_quest_graph_start_node)));
    // O `QuestContext` montado e destruído, sem executar nó (o `ExecuteNode` de verdade mexe em progresso).
    let ok_ctx = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsC157Contexto", "BwmsC157Contexto", corpo_de_prova!(tramp_c157_contexto)));
    let ok_exec = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsC157Executar", "BwmsC157Executar", corpo_de_prova!(tramp_c157_executar)));
    crate::log_dev!("[reg] register_all: questctx-157 QuestContexto={ok_ctx} ExecuteNodeSintetico={ok_exec}");
    // `QuestsSystem.ExecuteNode` com a trava, o construtor de nó e o teste.
    let ok_en = register_argful_by_types(reg, proto, &["handle:QuestsSystem", "handle:IScriptable", "CName"], "BwmsQuestsSystemExecuteNode", "BwmsQuestsSystemExecuteNode", tramp_quests_execute_node as NativeHandler);
    let ok_nn = registro_de_prova!(register_argful_by_types(reg, proto, &["CName"], "BwmsQuestNodeNew", "BwmsQuestNodeNew", corpo_de_prova!(tramp_quest_node_new)));
    let ok_48 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:QuestsSystem", "handle:IScriptable", "Int32"], "BwmsC48Medir", "BwmsC48Medir", corpo_de_prova!(tramp_c48_medir)));
    crate::log_dev!("[reg] register_all: executenode-48 QuestsSystemExecuteNode={ok_en} QuestNodeNew={ok_nn} C48Medir={ok_48}");
    crate::log_dev!("[reg] register_all: fase-35 2d-3b QuestLoaderOp={ok_fila_op} QuestLoaderResource={ok_fila_res} QuestGraphStartNode={ok_inicio}");
    crate::log_dev!(
        "[reg] register_all: grafo-160 QuestPhaseResourceGraph={ok1} AccessorCount={ok2} At={ok3} FindCount={ok4} FindAt={ok5} PathCount={ok6} PathDepth={ok7} PathId={ok8} SyntheticTest={ok9}"
    );
}

// O `WorldStateSystem` sem classe de sistema forjada, 2026-09-18.
//
// O sistema tem 13 métodos. No Mac os 9 de comunidade e spawner já são o `CommunitySystem` do motor por vtable (`#77`) e
// aqui ganham o `CommunitySystem::Update(sys, true)` (`0x10472c880`) depois de cada mutação: ele esvazia na hora a fila de pedidos
// em `+0x48` que os slots de comunidade enchem (sem ele o pedido espera o próximo tick; os slots de spawner repassam pra `[sys+0xd8]`, que o `Update` não
// processa, como no Windows). Os outros 4:
// - `IsReady`: "mundo anexado" e "fases registradas"; aqui é o `QuestsSystem` da sessão com a fase raiz (`[sys+0x68]`) e o streaming
//   world carregado.
// - `GetStreamingWorld`: o `Handle<worldStreamingWorld>` em `+0x270` do runtime system 27, `worldRuntimeSystemWorldStreaming` (o getter do motor
//   `0x101bfe958` é `add x0, x0, #0x270`).
// - `ToggleNode`/`ToggleVariant`: um `questWorldDataManagerNodeDefinition` com o tipo `questShowWorldNode_NodeType` (`objectRef`,
//   `show`) ou `questTogglePrefabVariant_NodeType` (`params[0]` com o prefab e 1 estado de variante), executado pelo `ExecuteNode` do motor na fase raiz
//   (`quests_execute_node_com`, o do `#48`). O `Execute` do tipo (`0x1029c6274` e `0x1029c80cc`) resolve o NodeRef na raiz, aplica no streaming (mapa de
//   visibilidade do `NodeStreaming`, runtime system 42, em `+0x150`, ou o gerenciador de variantes em `[streaming+0x288]`) e grava no
//   `questWorldStateSystem` (mapas `+0x70` e `+0xe8`), que vai pro save. O `questWorldStateSystem` é lido do contexto (`[phaseCtx+0x50]`) sem teste de
//   nulo: a execução só segue com ele lá, pela vtable medida.
pub(crate) const WS_STREAMING_IDX: usize = 27;

pub(crate) const WS_STREAMING_WORLD_OFF: usize = 0x270;

/// `QuestPhaseContext` dentro do `QuestContext`: o `phaseContext` que o `Execute` recebe.
pub(crate) const QUESTCTX_FASE_OFF: usize = 0x110;

/// `QuestsSystem` da sessão; nulo fora do gameplay.
pub(crate) unsafe fn quests_da_sessao() -> *mut c_void {
    let s = sistema_da_sessao("questQuestsSystem", None);
    if rtti::sane(s) && classe_deriva_de_nome(rtti::class_of(s), crate::cname::cname("questQuestsSystem")) { s } else { std::ptr::null_mut() }
}

/// O contexto montado tem o `questWorldStateSystem` (vtable medida), o `GameInstance` e o registro de nós que o `Execute` dos tipos de mundo usa.
pub(crate) fn ws_contexto_ok(ctx: *const u8) -> bool {
    unsafe {
        let fase = ctx.add(QUESTCTX_FASE_OFF) as *const c_void;
        let wss = rd_u64(fase, 0x50);
        wss != 0
            && crate::memoria::is_readable(wss as *const c_void, 8)
            && rd_u64(wss as *const c_void, 0) == crate::rebase(WSS_VTBL_VM) as u64
            && rd_u64(fase, 0x08) != 0
            && rd_u64(fase, 0x1e0) != 0
    }
}

/// `worldRuntimeSystemWorldStreaming` da cena; 0 fora do mundo.
pub(crate) unsafe fn ws_streaming() -> u64 {
    let (inst, nome) = runtime_system_at(WS_STREAMING_IDX);
    if nome == "worldRuntimeSystemWorldStreaming" { inst } else { 0 }
}

/// `Handle<worldStreamingWorld>` do streaming, sem somar referência; `[0, 0]` sem mundo carregado.
pub(crate) unsafe fn ws_streaming_world() -> [u64; 2] {
    let s = ws_streaming();
    if s == 0 || !crate::memoria::is_readable((s as usize + WS_STREAMING_WORLD_OFF) as *const c_void, 16) {
        return [0, 0];
    }
    let h = [rd_u64(s as *const c_void, WS_STREAMING_WORLD_OFF), rd_u64(s as *const c_void, WS_STREAMING_WORLD_OFF + 8)];
    let ok = rtti::sane(h[0] as *mut c_void) && classe_deriva_de_nome(rtti::class_of(h[0] as *mut c_void), crate::cname::cname("worldStreamingWorld"));
    if ok { h } else { [0, 0] }
}

/// `WorldStateSystem.IsReady`.
pub(crate) unsafe fn ws_pronto() -> bool {
    let q = quests_da_sessao();
    !q.is_null() && rtti::sane(rd_u64(q, 0x68) as *mut c_void) && ws_streaming_world()[0] != 0
}

/// Endereço do campo `nome` de uma instância de `cls` (ou de um struct em `base`) pelo RTTI.
pub(crate) unsafe fn ws_campo(base: *mut u8, cls: *mut c_void, nome: &str) -> Option<usize> {
    rtti::resolve_prop_in_class(cls, nome).map(|(o, _, _)| base as usize + o as usize)
}

/// `questWorldDataManagerNodeDefinition` com um tipo novo de `classe_tipo` preenchido por `preencher`, executado na fase raiz. `Err`: -10 sem sessão, -11
/// classe, -12 campo, e os de `quests_execute_node_com` (-6 = contexto sem o `questWorldStateSystem`).
pub(crate) unsafe fn ws_executar(classe_tipo: &str, preencher: impl FnOnce(*mut u8) -> bool) -> Result<u32, i32> {
    let (Some(reg), sys) = (Registry::obtain(), quests_da_sessao()) else { return Err(-10) };
    if sys.is_null() {
        return Err(-10);
    }
    let no = rtti::new_object(&reg, "questWorldDataManagerNodeDefinition");
    if !rtti::sane(no) {
        return Err(-11);
    }
    let mut dono = [0u64; 2];
    rtti::make_handle(dono.as_mut_ptr() as *mut c_void, no);
    let tipo = rtti::new_object(&reg, classe_tipo);
    let r = if !rtti::sane(tipo) {
        Err(-11)
    } else if !campo_handle_grava(no as *mut u8, "type", tipo as *mut u8) {
        // Sem o campo o tipo não tem dono: o handle solto aqui o destrói.
        let mut solto = [0u64; 2];
        rtti::make_handle(solto.as_mut_ptr() as *mut c_void, tipo);
        crate::jogo::objetos::soltar_handle(&mut solto);
        Err(-12)
    } else if !preencher(tipo as *mut u8) {
        Err(-12)
    } else {
        quests_execute_node_com(sys, no, 0, ws_contexto_ok).map(|e| e.resultado)
    };
    let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono);
    #[cfg(feature = "devtools")]
    WS_ULTIMO.store(r.map_or_else(|e| e as i64, |v| v as i64), Ordering::Relaxed);
    r
}

/// `WorldStateSystem.ToggleNode(node, state)`.
pub(crate) unsafe fn ws_toggle_node(noderef: u64, ligar: bool) -> Result<u32, i32> {
    ws_executar("questShowWorldNode_NodeType", |t| {
        let c = rtti::class_of(t as *mut c_void);
        match (ws_campo(t, c, "objectRef"), ws_campo(t, c, "show"), ws_campo(t, c, "isPlayer")) {
            (Some(r), Some(s), Some(p)) => {
                (r as *mut u64).write_unaligned(noderef);
                *(s as *mut u8) = ligar as u8;
                *(p as *mut u8) = 0;
                true
            }
            _ => false,
        }
    })
}

/// `WorldStateSystem.ToggleVariant(prefab, variant, state)`: 1 parâmetro com o prefab e 1 estado de variante, anexados pelo reserve do motor.
pub(crate) unsafe fn ws_toggle_variant(noderef: u64, variante: CName, ligar: bool) -> Result<u32, i32> {
    ws_executar("questTogglePrefabVariant_NodeType", |t| {
        let Some((off, tipo_arr, _)) = rtti::resolve_prop_in_class(rtti::class_of(t as *mut c_void), "params") else { return false };
        let Some(tp) = rtti::get_inner_type(tipo_arr) else { return false };
        let (Some((o_ref, _, _)), Some((o_estados, tipo_estados, _))) =
            (rtti::resolve_prop_in_class(tp, "prefabNodeRef"), rtti::resolve_prop_in_class(tp, "variantStates"))
        else {
            return false;
        };
        let Some(te) = rtti::get_inner_type(tipo_estados) else { return false };
        let (Some((o_nome, _, _)), Some((o_show, _, _))) = (rtti::resolve_prop_in_class(te, "name"), rtti::resolve_prop_in_class(te, "show")) else {
            return false;
        };
        let (passo, passo_e) = (rtti::type_size(tp) as usize, rtti::type_size(te) as usize);
        if passo == 0 || passo > 0x100 || passo_e == 0 || passo_e > 0x100 {
            return false;
        }
        let Some(p) = cust_anexar_valor(t as usize + off as usize, tp as usize, passo, 0) else { return false };
        ((p + o_ref as usize) as *mut u64).write_unaligned(noderef);
        let Some(e) = cust_anexar_valor(p + o_estados as usize, te as usize, passo_e, 0) else { return false };
        ((e + o_nome as usize) as *mut u64).write_unaligned(variante.valor());
        *((e + o_show as usize) as *mut u8) = ligar as u8;
        true
    })
}

/// `BwmsWorldStateIsReady() -> Bool`
pub(crate) unsafe extern "C" fn tramp_worldstate_is_ready(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let _ = rtti::read_params_consuming(current_native_func(), frame);
    write_bool_ret(out, ws_pronto());
}

/// `BwmsWorldStateStreamingWorld() -> ref<IScriptable>`: o `worldStreamingWorld` dentro de um `BwmsResource`, com referência forte nova.
pub(crate) unsafe extern "C" fn tramp_worldstate_streaming_world(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let _ = rtti::read_params_consuming(current_native_func(), frame);
    let mut s = recurso_para_script(ws_streaming_world());
    if out.is_null() {
        crate::jogo::objetos::soltar_handle(&mut s);
    } else {
        entregar_handle(out, s);
    }
}

/// `BwmsWorldStateToggleNode(node: NodeRef, state: Bool) -> Int32`: o resultado do `ExecuteNode` ou o erro negativo.
pub(crate) unsafe extern "C" fn tramp_worldstate_toggle_node(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let no = a.first().map_or(0, |(v, _)| *v);
    let ligar = a.get(1).is_some_and(|(v, _)| rtti::bool_do_script(*v));
    let r = if no == 0 { Err(-7) } else { ws_toggle_node(no, ligar) };
    write_i32_ret(out, r.map_or_else(|e| e, |v| v as i32));
}

/// `BwmsWorldStateToggleVariant(ref: NodeRef, name: CName, state: Bool) -> Int32`
pub(crate) unsafe extern "C" fn tramp_worldstate_toggle_variant(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let no = a.first().map_or(0, |(v, _)| *v);
    let variante = a.get(1).map_or(0, |(v, _)| *v);
    let ligar = a.get(2).is_some_and(|(v, _)| rtti::bool_do_script(*v));
    let r = if no == 0 || variante == 0 { Err(-7) } else { ws_toggle_variant(no, CName::lido_do_motor(variante), ligar) };
    write_i32_ret(out, r.map_or_else(|e| e, |v| v as i32));
}

pub(crate) unsafe fn register_worldstate_natives(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &[], "BwmsWorldStateIsReady", "BwmsWorldStateIsReady", tramp_worldstate_is_ready as NativeHandler);
    let ok2 = register_argful_by_types(reg, proto, &[], "BwmsWorldStateStreamingWorld", "BwmsWorldStateStreamingWorld", tramp_worldstate_streaming_world as NativeHandler);
    let ok3 = register_argful_by_types(reg, proto, &["NodeRef", "Bool"], "BwmsWorldStateToggleNode", "BwmsWorldStateToggleNode", tramp_worldstate_toggle_node as NativeHandler);
    let ok4 = register_argful_by_types(reg, proto, &["NodeRef", "CName", "Bool"], "BwmsWorldStateToggleVariant", "BwmsWorldStateToggleVariant", tramp_worldstate_toggle_variant as NativeHandler);
    let ok5 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:CommunitySystem"], "BwmsCommunityUpdate", "BwmsCommunityUpdate", corpo_de_prova!(tramp_community_update)));
    let ok6 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsC84Mundo", "BwmsC84Mundo", corpo_de_prova!(tramp_c84_mundo)));
    let ok7 = register_argful_by_types(reg, proto, &["Int32", "NodeRef", "CName", "CName"], "BwmsWorldStateCommunity", "BwmsWorldStateCommunity", tramp_worldstate_community as NativeHandler);
    let ok8 = register_argful_by_types(reg, proto, &["Int32", "NodeRef"], "BwmsWorldStateSpawner", "BwmsWorldStateSpawner", tramp_worldstate_spawner as NativeHandler);
    let ok9 = register_argful_by_types(reg, proto, &["NodeRef"], "BwmsWorldStateGetCommunity", "BwmsWorldStateGetCommunity", tramp_worldstate_get_community as NativeHandler);
    let ok10 = register_argful_by_types(reg, proto, &["NodeRef"], "BwmsWorldStateSpawnerId", "BwmsWorldStateSpawnerId", tramp_worldstate_spawner_id as NativeHandler);
    let ok11 = register_argful_by_types(reg, proto, &[], "BwmsSessionGame", "BwmsSessionGame", tramp_session_game as NativeHandler);
    let ok12 = registro_de_prova!(register_argful_by_types(reg, proto, &[], "BwmsC84Fase", "BwmsC84Fase", corpo_de_prova!(tramp_c84_fase)));
    crate::log_dev!("[reg] register_all: worldstate-84 C84Fase={ok12}");
    crate::log_dev!(
        "[reg] register_all: worldstate-84 WorldStateIsReady={ok1} StreamingWorld={ok2} ToggleNode={ok3} ToggleVariant={ok4} CommunityUpdate={ok5} C84Mundo={ok6} \
         Community={ok7} Spawner={ok8} GetCommunity={ok9} SpawnerId={ok10} SessionGame={ok11}"
    );
}

/// Grava em `obj.nome` um Handle novo pra `alvo` (o campo começa zerado); `false` se o campo não existe.
pub(crate) unsafe fn campo_handle_grava(obj: *mut u8, nome: &str, alvo: *mut u8) -> bool {
    let prop = rtti::find_property_in_class(rtti::class_of(obj as *mut c_void), nome);
    if !rtti::sane(prop) {
        return false;
    }
    rtti::make_handle(obj.add(rd_u32(prop as *const c_void, 0x20) as usize) as *mut c_void, alvo as *mut c_void);
    true
}





/// `id → handle` dos filhos do container.
pub(crate) unsafe fn journal_ids_mapa(container: *mut c_void) -> std::collections::HashMap<String, [u64; 2]> {
    let base = (container as *const u8).add(JOURNAL_CONTAINER_ENTRIES_OFF);
    let ptr = (base as *const u64).read_unaligned() as *const u64;
    let n = (base.add(0x0C) as *const u32).read_unaligned() as usize;
    let mut m = std::collections::HashMap::new();
    if ptr.is_null() || n > 65536 || !crate::memoria::is_readable(ptr as *const c_void, n * 16) {
        return m;
    }
    for i in 0..n {
        let f = ptr.add(i * 2).read_unaligned() as *mut c_void;
        if rtti::sane(f) && crate::memoria::is_readable(f as *const c_void, JOURNAL_ENTRY_ID_OFF + 0x20) {
            if let Some(id) = rtti::read_cstring((f as *const u8).add(JOURNAL_ENTRY_ID_OFF)) {
                m.insert(id, [f as u64, ptr.add(i * 2 + 1).read_unaligned()]);
            }
        }
    }
    m
}








/// Tipo `String` do RTTI, tirado do campo `id` da entrada de journal.
pub(crate) unsafe fn tipo_string_da_entrada(entrada: *mut c_void) -> *mut c_void {
    let prop = rtti::find_property_in_class(rtti::class_of(entrada), "id");
    if rtti::sane(prop) { rd_u64(prop as *const c_void, 0x00) as *mut c_void } else { std::ptr::null_mut() }
}

/// `CString` montada pelo construtor do motor (inline até 19 bytes, heap pelo allocator do motor acima disso).
pub(crate) unsafe fn cstring_do_motor(texto: &str) -> Option<[u8; 0x20]> {
    use core::arch::asm;
    let c = std::ffi::CString::new(texto).ok()?;
    let mut s = [0u8; 0x20];
    let ctor = crate::rebase(CSTRING_CTOR_VM);
    asm!("blr {f}", f = in(reg) ctor, in("x0") s.as_mut_ptr(), in("x1") c.as_ptr(), clobber_abi("C"));
    Some(s)
}

/// Destrói a `CString` do slot pelo tipo `String` (`vt+0x48`).
pub(crate) unsafe fn destruir_string(tipo: *mut c_void, slot: *mut u8) {
    let vt = rd_u64(tipo as *const c_void, 0) as *const u8;
    if vt.is_null() {
        return;
    }
    let d = (vt.add(0x48) as *const u64).read_unaligned();
    if d != 0 {
        let f: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(d as usize);
        f(tipo, slot as *mut c_void);
    }
}

/// Converte os campos sem `LocKey#`: `(campos, convertidos)`; `Err(-2)` sem tipo `String`.
pub(crate) unsafe fn journal_converter_lockeys(entrada: *mut c_void) -> Result<(usize, usize), i32> {
    let tipo = tipo_string_da_entrada(entrada);
    if !rtti::sane(tipo) {
        return Err(-2);
    }
    let slots = journal_lockey_slots(entrada);
    let mut convertidos = 0;
    for slot in &slots {
        let texto = rtti::read_cstring(*slot).unwrap_or_default();
        if texto.starts_with(LOCKEY_PREFIXO) {
            continue;
        }
        let novo_texto = format!("{LOCKEY_PREFIXO}{}", bwms_primitives::fnv1a64(texto.as_bytes()));
        let Some(novo) = cstring_do_motor(&novo_texto) else { continue };
        destruir_string(tipo, *slot);
        std::ptr::copy_nonoverlapping(novo.as_ptr(), *slot, 0x20);
        convertidos += 1;
    }
    Ok((slots.len(), convertidos))
}











pub(crate) const PHYSICS_GETHITENTITY_VM: u64 = 0x1_0156_e184;

pub(crate) unsafe extern "C" fn tramp_traceresult_gethitobject(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_args, srefs) = rtti::read_params_consuming_with_scriptrefs(current_native_func(), frame);
    let id = srefs.first().copied().and_then(|p| traceresult_result_id(p));
    let mut h = [0u64; 2];
    if let Some(id) = id {
        // O resultado guarda até 2 objetos, e o 1º pode vir vazio.
        for indice in 0..2u32 {
            h = chamar_handle_sret(PHYSICS_GETHITOBJECT_VM, id, indice);
            if h[0] != 0 {
                break;
            }
        }
    }
    // O retorno do lado redscript é `ref<IScriptable>` (o compilador não conhece `ISerializable`).
    // Objeto que não seja IScriptable não sobe: vira null, e a referência forte do motor é solta aqui (o chão, um
    // `worldCollisionNodeInstance`, cai neste caminho; segurá-lo prenderia o nó do setor depois do streaming descarregar).
    let cls = if h[0] != 0 { rtti::class_of(h[0] as *mut c_void) } else { std::ptr::null_mut() };
    let scriptable = !cls.is_null() && rtti::derives_from_iscriptable(cls);
    crate::log_dev!(
        "[cw44-hit] BwmsTraceResultGetHitObject(resultID={id:?}) -> {:#x} classe={} scriptable={scriptable}",
        h[0],
        if cls.is_null() { String::from("-") } else { format!("{:#x}", rtti::type_name_hash(cls)) }
    );
    if h[0] != 0 && !scriptable {
        crate::jogo::objetos::soltar_handle(&mut h);
        h = [0u64; 2];
    }
    entregar_handle(out, h);
}

pub(crate) unsafe extern "C" fn tramp_traceresult_gethitentity(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_args, srefs) = rtti::read_params_consuming_with_scriptrefs(current_native_func(), frame);
    let id = srefs.first().copied().and_then(|p| traceresult_result_id(p));
    let h = match id {
        Some(id) => chamar_handle_sret(PHYSICS_GETHITENTITY_VM, id, 0),
        None => [0u64; 2],
    };
    crate::log_dev!("[cw44-hit] BwmsTraceResultGetHitEntity(resultID={id:?}) -> {:#x}", h[0]);
    entregar_handle(out, h);
}

/// `BwmsTraceResultGetHitObjectClass(result) -> CName` — a classe do 1º objeto atingido, QUALQUER classe.
/// No boot 9 o chão devolveu um `worldCollisionNodeInstance`, que não é IScriptable e por isso não sobe como
/// `ref<>` — este native deixa o script saber o que foi atingido mesmo assim. O Handle devolvido pelo motor
/// traz referência forte, que é solta depois de ler a classe.
pub(crate) unsafe extern "C" fn tramp_traceresult_gethitobject_class(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_args, srefs) = rtti::read_params_consuming_with_scriptrefs(current_native_func(), frame);
    let id = srefs.first().copied().and_then(|p| traceresult_result_id(p));
    let mut classe: u64 = 0;
    if let Some(id) = id {
        for indice in 0..2u32 {
            let mut h = chamar_handle_sret(PHYSICS_GETHITOBJECT_VM, id, indice);
            if h[0] != 0 {
                let cls = rtti::class_of(h[0] as *mut c_void);
                if !cls.is_null() {
                    classe = rtti::type_name_hash(cls);
                }
                // Só a classe sai daqui: a referência forte que o motor deu volta.
                crate::jogo::objetos::soltar_handle(&mut h);
                break;
            }
        }
    }
    crate::log_dev!("[cw44-hit] BwmsTraceResultGetHitObjectClass(resultID={id:?}) -> {classe:#x}");
    write_uint_ret(out, classe, 8);
}

/// Valor `what` da instância achada pelo id (ver `BwmsNodeInstanceById`).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn node_instance_por_id(id: u64, what: i32) -> f32 {
    if what == 9 {
        return match find_node(0) {
            None => -2.0,
            Some(h) => (h[0] == 0) as i32 as f32,
        };
    }
    let Some(mut h) = find_node(id) else { return -2.0 };
    let obj = h[0] as *const u8;
    if obj.is_null() {
        return if what == 0 { 0.0 } else { f32::NAN };
    }
    let legivel = rtti::sane(obj as *mut c_void) && crate::memoria::is_readable(obj as *const c_void, 0x90);
    let rd = |off: usize| (obj.add(off) as *const u64).read_unaligned();
    let r = if !legivel {
        f32::NAN
    } else {
        let no = rd(NODEINSTANCE_NO_OFF) as *const u8;
        let (duplo, simples) = node_instance_setups(obj);
        let setup = duplo.or(simples);
        let quat = [0, 4, 8, 12].map(|d| (obj.add(NODEINSTANCE_QUAT_OFF + d) as *const f32).read_unaligned());
        if what == 0 {
            crate::veredito_dev!(
                "[cw85] FindNode({id:#x}) -> instância {obj:p}, nó {no:p}, GlobalNodeID {:#x}, setup duplo {} / simples {} (globalNodeID {:#x}), posição {:?}",
                rd(NODEINSTANCE_GLOBALID_OFF),
                duplo.is_some(),
                simples.is_some(),
                setup.map_or(0, |s| (s.add(NODESETUP_GLOBALID_OFF) as *const u64).read_unaligned()),
                f32x3(obj, NODEINSTANCE_POSICAO_OFF)
            );
        }
        match what {
            0 => 1.0,
            1 => (rd(NODEINSTANCE_GLOBALID_OFF) == id) as i32 as f32,
            2 => setup.map_or(0.0, |s| ((s.add(NODESETUP_GLOBALID_OFF) as *const u64).read_unaligned() == id) as i32 as f32),
            3 => (rtti::sane(no as *mut c_void) && crate::memoria::is_readable(no as *const c_void, 0x38) && acessor_e_da_classe(no, "worldNode")) as i32 as f32,
            4..=6 => f32x3(obj, NODEINSTANCE_POSICAO_OFF)[(what - 4) as usize],
            7 => ((quat.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 0.01) as i32 as f32,
            8 => (rd(NODEINSTANCE_GLOBALID_OFF + 8) == id) as i32 as f32,
            _ => f32::NAN,
        }
    };
    let _ = crate::jogo::objetos::soltar_handle_contando(&mut h);
    r
}

/// `BwmsNodeInstanceById(id, what) -> Float`: instância que o `FindNode` do registro devolve pro id (hash do GlobalNodeRef) — 0 achou (1/0), 1 GlobalNodeID
/// `+0x80` igual ao id, 2 setup (pelo nó) com globalNodeID igual ao id, 3 nó é `worldNode`, 4..6 posição x/y/z, 7 quaternion com norma ≈ 1, 8 controle:
/// GlobalNodeID lido 8 B deslocado igual ao id, 9 controle: `FindNode(0)` devolve vazio (1/0); -2 sem registro ou com o slot diferente da RE.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_node_instance_by_id(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let what = args.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    write_uint_ret(out, node_instance_por_id(id, what).to_bits() as u64, 4);
}

/// Valor da instância do id (`alvo` 0 a instância, 1 o setup) em `off` com `largura`; `None` sem instância, sem setup ou fora da faixa.
pub(crate) unsafe fn node_instance_ler(id: u64, alvo: i32, off: i32, largura: usize) -> Option<u64> {
    let mut h = find_node(id)?;
    let obj = h[0] as *const u8;
    if obj.is_null() {
        return None;
    }
    let v = if !rtti::sane(obj as *mut c_void) || !crate::memoria::is_readable(obj as *const c_void, NODEINSTANCE_LEITURA_FIM) {
        None
    } else if alvo == 0 {
        ler_largura_ate(obj, off, largura, NODEINSTANCE_LEITURA_FIM)
    } else {
        let (duplo, simples) = node_instance_setups(obj);
        duplo.or(simples).and_then(|s| ler_largura_ate(s, off, largura, NODEINSTANCE_LEITURA_FIM))
    };
    let _ = crate::jogo::objetos::soltar_handle_contando(&mut h);
    v
}

/// Hash do nome da classe do nó da instância do id; 0 sem instância.
pub(crate) unsafe fn node_instance_classe_do_no(id: u64) -> u64 {
    let Some(mut h) = find_node(id) else { return 0 };
    let obj = h[0] as *const u8;
    if obj.is_null() {
        return 0;
    }
    let no = if crate::memoria::is_readable(obj as *const c_void, NODEINSTANCE_LEITURA_FIM) {
        (obj.add(NODEINSTANCE_NO_OFF) as *const u64).read_unaligned() as *mut c_void
    } else {
        std::ptr::null_mut()
    };
    let classe = if rtti::sane(no) && crate::memoria::is_readable(no as *const c_void, 8) { rtti::type_name_hash(rtti::class_of(no)) } else { 0 };
    let _ = crate::jogo::objetos::soltar_handle_contando(&mut h);
    classe
}

/// `BwmsNodeInstanceF32(id, target, offset) -> Float`: f32 da instância do id (`target` 0) ou do setup dela (1) em `offset`; NaN sem instância ou fora
/// dos 0x90 bytes.
pub(crate) unsafe extern "C" fn tramp_node_instance_f32(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let id = a.first().map(|(v, _)| *v).unwrap_or(0);
    let alvo = a.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let off = a.get(2).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let v = node_instance_ler(id, alvo, off, 4).map_or(f32::NAN, |b| f32::from_bits(b as u32));
    write_uint_ret(out, v.to_bits() as u64, 4);
}

/// `BwmsNodeInstanceU64(id, target, offset) -> Uint64`: u64 da instância do id (`target` 0) ou do setup (1) em `offset`; 0 sem instância ou fora da faixa.
pub(crate) unsafe extern "C" fn tramp_node_instance_u64(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let id = a.first().map(|(v, _)| *v).unwrap_or(0);
    let alvo = a.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let off = a.get(2).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    write_uint_ret(out, node_instance_ler(id, alvo, off, 8).unwrap_or(0), 8);
}

/// `BwmsNodeInstanceNodeClass(id) -> CName`: classe do nó da instância do id; `None` sem instância.
pub(crate) unsafe extern "C" fn tramp_node_instance_node_class(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let id = a.first().map(|(v, _)| *v).unwrap_or(0);
    write_uint_ret(out, node_instance_classe_do_no(id), 8);
}

/// `BwmsNodeInstanceInfo(id, what) -> Uint64`: `what` 0 = hash da classe da instância, 1 = `WorldNodeInstanceNodeState`, 2 = hash da classe do objeto que
/// ela carrega (mesh, material ou entidade, conforme o tipo); 0 sem instância.
pub(crate) unsafe extern "C" fn tramp_node_instance_info(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let id = a.first().map(|(v, _)| *v).unwrap_or(0);
    let what = a.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let v = node_instance_objeto(id, None).map_or(0, |l| match what {
        0 => l.nome,
        1 => l.estado as u64,
        2 if rtti::sane(l.carga) => rtti::type_name_hash(l.carga),
        _ => 0,
    });
    write_uint_ret(out, v, 8);
}


pub(crate) type CommunitySpawnerFn = unsafe extern "C" fn(*mut c_void, u64);

// `GetCommunity` (2026-08-14, mesma sessão — slot que faltava nas 7 já
// implementadas acima):
//   GetCommunity@0x228 = void* (ICommunitySystem::*)(WeakPtr<Community>& aOut,
//                                                     const uint64_t& aCommunityID)
// Mac = Windows 0x228 + 0x08 (shift Itanium já estabelecido) = 0x230. **A SLOT em si está
// correta** — o crash real (2026-08-14 14:05, `Cyberpunk2077-2026-08-14-140515.ips`, thread
// `redDispatcher4`, `EXC_BAD_ACCESS`/`KERN_INVALID_ADDRESS at 0x4`) foi diagnosticado por
// disassembly REAL do binário Mac na região da falha (não suposição): a função que mora nesse
// slot (vmaddr estático `0x10472bf60`, faulting insn em `0x10472c0dc`) NÃO usa a ABI que a
// assinatura acima sugere (2 parâmetros por referência). O corpo real:
//   - `this+0x68`/`this+0x74` = ponteiro+contagem de um array ORDENADO de IDs (busca binária via
//     `0x101667c74`, um comparador TRIVIAL `cmp x0,x1; cset w0,lo; ret` — confirma que os 2
//     valores comparados são ESCALARES `uint64_t`, nunca deferenciados);
//   - `x1` (2º argumento) é usado DIRETO nessa comparação, sem NENHUM `ldr` — ou seja,
//     `aCommunityID` chega **por VALOR em x1**, não por ponteiro (a assinatura C++ é "de intenção",
//     não necessariamente a ABI exata desta build);
//   - o "não achado" cai no branch `mov x8,#0; b <store>` → `stp x8, xzr, [x20]`, onde `x20` foi
//     copiado de **x8 NA ENTRADA DA FUNÇÃO** (`mov x20, x8`, a 1ª instrução real do corpo) — x8
//     é o REGISTRADOR DE RESULTADO INDIRETO do ARM64 (AAPCS64/Itanium), usado quando o retorno é
//     um tipo C++ NÃO-TRIVIAL devolvido POR VALOR (`WeakPtr<Community>` tem dtor não-trivial,
//     `~WeakPtr(){DecWeakRef();}` — pela regra Itanium isso força sret INDEPENDENTE do tamanho).
//   O bug real: a implementação ANTERIOR chamava a função como `(this, *mut u8, *const u64)`
//   (x0=this, x1=ponteiro-pro-out-buf, x2=ponteiro-pro-id) — **nunca populava x8** (Rust/LLVM só
//   usa x8 automaticamente quando o TIPO DE RETORNO Rust em si precisa de sret — não é o caso
//   de um simples `*mut c_void`). x8 ficava com LIXO de chamadas anteriores (calhou em `4` nesse
//   boot) → a função salvava esse lixo em `x20` → `stp x8,xzr,[x20]` escrevia em `[0x4]` →
//   crash EXATO em `0x0000000000000004`, o mesmo endereço do relatório.
//
// **Fix (mesmo padrão `SretPad32` já provado ao vivo neste projeto pra exatamente este tipo de
// bug — `garment-apply`/`inkspawner-apply`, 2026-07-29, ver `selftest.rs`):**
// declarar o retorno Rust como uma struct >16 bytes força o LLVM/aarch64 a usar sret de verdade
// (populando x8 sozinho, do jeito que o C++ real espera) — e passar `community_id` POR VALOR
// (não mais um ponteiro), batendo com o que o comparador provou. Isso elimina o out-buf de 16
// bytes por completo (não é mais necessário — o valor chega pronto no sret).
#[repr(C)]
pub(crate) struct CommunityWeakPtrSret {
    pub(crate) instance: u64,
    pub(crate) ref_count: u64,
    pub(crate) _pad: [u64; 2],
}

pub(crate) const COMMUNITY_UPDATE_VM: u64 = 0x1_0472_c880;

pub(crate) const COMMUNITY_UPDATE_PROLOGO: u32 = 0xd102_03ff;

pub(crate) const COMMUNITYSYSTEM_VTBL_VM: u64 = 0x1_07c6_7818;

/// Id global do NodeRef pelo `ResolveNodeRef` vanilla na raiz; 0 sem ele.
pub(crate) unsafe fn ws_resolver(noderef: u64) -> u64 {
    if noderef == 0 {
        return 0;
    }
    Registry::obtain().and_then(|reg| resolver_noderef(&reg, noderef)).unwrap_or(0)
}

/// `ActivateCommunity` (0), `DeactivateCommunity` (1), `SetCommunityPhase` (2) e `ResetCommunity` (3, sem entry) pelo `CommunitySystem`
/// da sessão, seguidos do `Update` do motor. Sem passar pelo `GameInstance` do script: pelo tick do BWMS o `GetCommunitySystem(GetGameInstance())`
/// voltou nulo (boots 247 e 248). `false` com NodeRef que não resolve ou sem o sistema.
pub(crate) unsafe fn ws_comunidade(op: i32, noderef: u64, entry: CName, fase: CName) -> bool {
    let (id, sys) = (ws_resolver(noderef), community_da_sessao());
    if id == 0 || sys.is_null() {
        return false;
    }
    let (entry, fase) = (entry.valor(), fase.valor());
    match op {
        0 => community_id_entry_call(sys, id, entry, COMMUNITY_ACTIVATE_SLOT, "WorldStateSystem.ActivateCommunity"),
        1 => community_id_entry_call(sys, id, entry, COMMUNITY_DEACTIVATE_SLOT, "WorldStateSystem.DeactivateCommunity"),
        2 => {
            let Some(fp) = vtbl_slot_fn_bounded(sys, COMMUNITY_SET_PHASE_SLOT) else { return false };
            let f: CommunitySetPhaseFn = core::mem::transmute(fp);
            f(sys, id, entry, fase);
        }
        3 => community_id_entry_call(sys, id, 0, COMMUNITY_RESET_SLOT, "WorldStateSystem.ResetCommunity"),
        _ => return false,
    }
    community_update(sys)
}

/// `ActivatePopulationSpawner` (0), `DeactivatePopulationSpawner` (1) e `ResetPopulationSpawner` (2), com o `Update` depois (o
/// `Update` não processa a fila dos spawners, `[sys+0xd8]`).
pub(crate) unsafe fn ws_spawner(op: i32, noderef: u64) -> bool {
    let (id, sys) = (ws_resolver(noderef), community_da_sessao());
    let slot = match op {
        0 => COMMUNITY_SPAWNER_ACTIVATE_SLOT,
        1 => COMMUNITY_SPAWNER_DEACTIVATE_SLOT,
        2 => COMMUNITY_SPAWNER_RESET_SLOT,
        _ => return false,
    };
    if id == 0 || sys.is_null() {
        return false;
    }
    community_spawner_call(sys, id, slot, "WorldStateSystem.PopulationSpawner");
    community_update(sys)
}

/// `BwmsWorldStateCommunity(op: Int32, community: NodeRef, entry: CName, phase: CName) -> Bool`
pub(crate) unsafe extern "C" fn tramp_worldstate_community(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let v = |i: usize| a.get(i).map_or(0, |(v, _)| *v);
    write_bool_ret(out, ws_comunidade(v(0) as u32 as i32, v(1), CName::lido_do_motor(v(2)), CName::lido_do_motor(v(3))));
}

/// `BwmsWorldStateSpawner(op: Int32, spawner: NodeRef) -> Bool`
pub(crate) unsafe extern "C" fn tramp_worldstate_spawner(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let v = |i: usize| a.get(i).map_or(0, |(v, _)| *v);
    write_bool_ret(out, ws_spawner(v(0) as u32 as i32, v(1)));
}

/// `BwmsWorldStateGetCommunity(community: NodeRef) -> ref<CommunityWrapper>`: o wrapper do `#77` pela comunidade do id resolvido.
pub(crate) unsafe extern "C" fn tramp_worldstate_get_community(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let (id, sys) = (ws_resolver(a.first().map_or(0, |(v, _)| *v)), community_da_sessao());
    comunidade_wrapper_ret(out, sys, id);
}

/// `BwmsWorldStateSpawnerId(spawner: NodeRef) -> Uint64`: o id resolvido quando o `GetSpawner` do motor o acha, 0 senão.
pub(crate) unsafe extern "C" fn tramp_worldstate_spawner_id(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let (id, sys) = (ws_resolver(a.first().map_or(0, |(v, _)| *v)), community_da_sessao());
    let achado = id != 0 && !sys.is_null() && community_get_spawner(sys, id) != 0;
    write_uint_ret(out, if achado { id } else { 0 }, 8);
}


// `BwmsCreateNodeRef(path: String) -> NodeRef` — fechada 2026-08-15 (rodada 20). Criar um NodeRef
// a partir de texto NUNCA precisou de endereço Mac — o
// `NodeRef` é um wrapper de 1 `uint64_t hash` e o hash (FNV-1a64 com 2 regras de skip) está
// em `bwms_primitives::node_ref_hash` desde a sessão de RE de 2026-07-16. Composição pura: lê a `String` (mesmo marshalling já usado
// dezenas de vezes, ex. `BwmsConfigGet`/`BwmsConfigSet`), computa o hash em Rust, devolve como
// escalar de 8 bytes — MESMA saída do `tramp_identidade_8_bytes`. Log de diagnóstico
// (string+hash) inclusos pra facilitar a leitura do proof ao vivo sem precisar de outro comando.
pub(crate) unsafe extern "C" fn tramp_string_to_noderef(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let hash = bwms_primitives::node_ref_hash(&path);
    crate::log_dev!("[noderefdbg] BwmsCreateNodeRef('{path}') -> {hash:#018x}");
    write_uint_ret(out, hash, 8);
}

#[cfg(feature = "devtools")]
pub(crate) const NODEINSTANCE_ANEXADO_BIT: u16 = 1 << 5;

pub(crate) const NODESETUP_NO_OFF: usize = 0x50;

#[cfg(feature = "devtools")]
pub(crate) const NODESETUP_PROXYID_OFF: usize = 0x60;

#[cfg(feature = "devtools")]
pub(crate) const NODESETUP_DISTANCIA_OFF: usize = 0x7C;

/// Checagem `what` (ver `BwmsTraceNodeInstanceCheck`) na instância `obj`; -1 ilegível, -2 `what` desconhecido.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn node_instance_checagem(obj: *const u8, what: i32) -> i32 {
    let legivel = |q: *const u8, n: usize| rtti::sane(q as *mut c_void) && crate::memoria::is_readable(q as *const c_void, n);
    if !legivel(obj, 0x90) {
        return -1;
    }
    let rd = |p: *const u8, off: usize| (p.add(off) as *const u64).read_unaligned();
    let no = rd(obj, NODEINSTANCE_NO_OFF) as *const u8;
    let gid = rd(obj, NODEINSTANCE_GLOBALID_OFF);
    let (duplo, simples) = node_instance_setups(obj);
    let setup = duplo.or(simples);
    let quat = [0, 4, 8, 12].map(|d| (obj.add(NODEINSTANCE_QUAT_OFF + d) as *const f32).read_unaligned());
    let estado = (obj.add(NODEINSTANCE_ESTADO_OFF) as *const u16).read_unaligned();
    if what == 0 {
        let f3 = |o: usize| [0, 4, 8].map(|d| (obj.add(o + d) as *const f32).read_unaligned());
        crate::veredito_dev!(
            "[cw85] instância {obj:p} (vptr worldCollisionNodeInstance {}): nó {no:p} (vptr worldCollisionNode {}), GlobalNodeID {gid:#x}, posição {:?}, quaternion {quat:?}, escala {:?}, estado {estado:#06x}; setup pelo nó: duplo {} / simples {}; setup globalNodeID {:#x}, proxy {:#x}, streamingDistance {}",
            rd(obj, 0) == crate::rebase(0x1_078e_11f0) as u64,
            legivel(no, 8) && rd(no, 0) == crate::rebase(0x1_078e_0d08) as u64,
            f3(NODEINSTANCE_POSICAO_OFF),
            f3(NODEINSTANCE_ESCALA_OFF),
            duplo.is_some(),
            simples.is_some(),
            setup.map_or(0, |s| rd(s, NODESETUP_GLOBALID_OFF)),
            setup.map_or(0, |s| rd(s, NODESETUP_PROXYID_OFF)),
            setup.map_or(f32::NAN, |s| (s.add(NODESETUP_DISTANCIA_OFF) as *const f32).read_unaligned())
        );
    }
    match what {
        0 => acessor_e_da_classe(obj, "worldINodeInstance") as i32,
        1 => (legivel(no, 0x38) && acessor_e_da_classe(no, "worldNode")) as i32,
        2 => duplo.is_some() as i32,
        3 => simples.is_some() as i32,
        4 => setup.map_or(0, |s| (rd(s, NODESETUP_GLOBALID_OFF) == gid) as i32),
        5 => ((quat.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 0.01) as i32,
        6 => (estado & NODEINSTANCE_ANEXADO_BIT != 0) as i32,
        7 => setup.map_or(0, |s| ((s.add(NODESETUP_DISTANCIA_OFF) as *const f32).read_unaligned() > 0.0) as i32),
        8 => setup.map_or(0, |s| (rd(obj, NODEINSTANCE_GLOBALID_OFF + 8) == rd(s, NODESETUP_GLOBALID_OFF)) as i32),
        9 => setup.map_or(0, |s| (rd(obj, NODEINSTANCE_NO_OFF + 8) == rd(s, NODESETUP_NO_OFF)) as i32),
        _ => -2,
    }
}

/// `BwmsTraceNodeInstanceCheck(result, what) -> Int32`: checagem `what` no objeto atingido pelo trace — 0 é `worldINodeInstance`, 1 o nó é `worldNode`,
/// 2/3 setup em `[[+0x70]]` / `[+0x70]` com `node*` igual ao nó da instância, 4 GlobalNodeID da instância igual ao do setup, 5 quaternion com norma ≈ 1,
/// 6 bit isAttached, 7 streamingDistance do setup > 0, 8/9 controles (GlobalNodeID e nó lidos 8 B deslocados contra o setup); 1 sim, 0 não, -1 sem objeto.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_trace_node_instance_check(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (args, srefs) = rtti::read_params_consuming_with_scriptrefs(current_native_func(), frame);
    let what = args.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(-2);
    let id = srefs.first().copied().and_then(|p| traceresult_result_id(p));
    let mut h = [0u64; 2];
    if let Some(id) = id {
        for indice in 0..2u32 {
            h = chamar_handle_sret(PHYSICS_GETHITOBJECT_VM, id, indice);
            if h[0] != 0 {
                break;
            }
        }
    }
    let r = if h[0] == 0 { -1 } else { node_instance_checagem(h[0] as *const u8, what) };
    write_uint_ret(out, r as u32 as u64, 4);
}


/// Início do `StreamingSectorNodeBuffer` (`+0x40` do setor).
/// `nodes`/`nodeRefs` acima são este mesmo bloco + `0x30`/`0x40` na 2.5, e batem — foi o
/// que confirmou o `+0x40`.
pub(crate) const SECTOR_NODEBUFFER: usize = 0x40;

/// Tamanho de `CompiledNodeInstanceSetupInfo` (0x90).
pub(crate) const SETUP_SIZE: usize = 0x90;

/// Setup `i` do setor (ponteiro dos 0x90 B), o `instance` do nó `k` do array `nodes` (0 fora da faixa) e a quantidade de nós; `None` sem o setup.
pub(crate) unsafe fn setup_e_no(path: &str, i: usize, k: usize) -> Option<(*mut u8, u64, u32)> {
    let sec = setor_por_path(path);
    let (entries, size) = sector_setups(sec)?;
    if i >= size as usize || !crate::memoria::is_readable(sec, SECTOR_NODES + 0x10) {
        return None;
    }
    let (nos, total, capacidade) = dynarray_em((sec as usize + SECTOR_NODES) as *const c_void);
    let entrada = nos + k * 0x10;
    let no = if dynarray_coerente(nos, total, capacidade) && k < total as usize && crate::memoria::is_readable(entrada as *const c_void, 8) {
        rd_u64(entrada as *const c_void, 0)
    } else {
        0
    };
    Some(((entries + i * SETUP_SIZE) as *mut u8, no, total))
}

/// Ponteiro da `RuntimeScene` capturado no `OnWorldAttached`. Só escrita
/// atômica no hook; quem lê é `register::cena_base`, fora do caminho crítico, e os runtime systems
/// saem por índice (`register::runtime_system_at`, comando `rtsys`). O getter da phase
/// (`inicio::boot::phase_skip_getter`) zera quando a fase sai de 5 (volta ao menu); ele só é
/// hookado com `pular_intro` e se o instalador aceitar o alvo (`install_phase_skip_near4`).
/// Sem o gancho, ou em troca de sessão sem fase fora de 5, ninguém zera.
pub(crate) static IGS_OWA_SCENE_PTR: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------------------------
// `TraceResult`: `GetHitObject`/`GetHitEntity` — 2026-09-11.
//
// Os dois endereços saíram offline (prova de 2026-09-11, varredura sequencial que identificou o `GetHitObject`):
// `GetHitObject(out, resultID, índice)` = `0x104583398`, e o `GetHitEntity(out, resultID)` do próprio
// motor = `0x10156e184` (laço de índice 0 e 1 com `IsA`). Os dois
// devolvem Handle de 16 B por SRET, que no ARM64 vai em `x8`. Chamada por `asm!` e não por
// `extern "C" fn -> Struct`: o `GetHitObject` sai sem escrever o `out` quando o resultID não tem
// registro, e o slot de sret que o Rust passaria não é zerado — seria lixo lido como Handle. Aqui o
// slot é zerado antes da chamada. O resultID é campo escondido do `TraceResult` em `+0x48` (o motor
// lê `[sp,#0x48]` de um TraceResult local antes de chamar a irmã `0x104582ff8`).
//
// O Handle que volta traz a referência forte e vai inteiro pro slot de retorno: a VM passa a ser
// dona e solta no teardown. Sem slot (retorno descartado) o Handle vaza de propósito — soltar à mão
// exige a sequência decremento/destrói/decremento-fraco do motor, e errar ali derruba o jogo.
pub(crate) const PHYSICS_GETHITOBJECT_VM: u64 = 0x1_0458_3398;

/// `resultID` do `TraceResult` apontado por um `script_ref<TraceResult>`; `None` se ilegível.
pub(crate) unsafe fn traceresult_result_id(traceresult: u64) -> Option<u32> {
    let campo = (traceresult as *const u8).wrapping_add(TRACERESULT_RESULTID_OFF);
    if traceresult == 0 || !crate::memoria::is_readable(campo as *const c_void, 4) {
        return None;
    }
    Some((campo as *const u32).read_unaligned())
}

/// Chama uma função do motor que devolve Handle por sret (`x8`), com `w0`/`w1` de argumento.
pub(crate) unsafe fn chamar_handle_sret(vmaddr: u64, w0: u32, w1: u32) -> [u64; 2] {
    let mut out = [0u64; 2];
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(vmaddr),
        in("x0") w0 as u64,
        in("x1") w1 as u64,
        in("x8") out.as_mut_ptr(),
        clobber_abi("C"),
    );
    out
}

pub(crate) unsafe fn register_traceresult_hit_natives(reg: &Registry, proto: *mut c_void) {
    let params = compose_params_from_types(reg, &["script_ref:TraceResult"]);
    let ok = params_compostos(params);
    if !ok {
        crate::log_dev!("[reg] register_traceresult_hit_natives: compose_params_from_types falhou (script_ref:TraceResult)");
    }
    let ok_obj = ok
        && register_global_composed(
            reg,
            proto,
            params,
            "BwmsTraceResultGetHitObject",
            "BwmsTraceResultGetHitObject",
            tramp_traceresult_gethitobject,
        );
    let ok_ent = ok
        && register_global_composed(
            reg,
            proto,
            params,
            "BwmsTraceResultGetHitEntity",
            "BwmsTraceResultGetHitEntity",
            tramp_traceresult_gethitentity,
        );
    let ok_cls = ok
        && register_global_composed(
            reg,
            proto,
            params,
            "BwmsTraceResultGetHitObjectClass",
            "BwmsTraceResultGetHitObjectClass",
            tramp_traceresult_gethitobject_class,
        );
    crate::log_dev!(
        "[reg] register_all: traceresult-44 BwmsTraceResultGetHitObject={ok_obj} BwmsTraceResultGetHitEntity={ok_ent} BwmsTraceResultGetHitObjectClass={ok_cls}"
    );
    let ok_trace_node = registro_de_prova!(register_argful_by_types(reg, proto, &["script_ref:TraceResult", "Int32"], "BwmsTraceNodeInstanceCheck", "BwmsTraceNodeInstanceCheck", corpo_de_prova!(tramp_trace_node_instance_check)));
    let ok_node_por_id = registro_de_prova!(register_argful_by_types(reg, proto, &["Uint64", "Int32"], "BwmsNodeInstanceById", "BwmsNodeInstanceById", corpo_de_prova!(tramp_node_instance_by_id)));
    let ok_vol = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Uint64", "Int32", "Int32"], "BwmsNodeAreaVolumeCheck", "BwmsNodeAreaVolumeCheck", corpo_de_prova!(tramp_node_area_volume_check)));
    let ok_censo_do_setor = registro_de_prova!(register_argful_by_types(reg, proto, &["String", "Int32"], "BwmsSectorInstancedMeshCensus", "BwmsSectorInstancedMeshCensus", corpo_de_prova!(tramp_sector_instanced_mesh_census)));
    let ok_7e_stat = registro_de_prova!(register_argful_by_types(reg, proto, &["Int32"], "BwmsJournalModStat", "BwmsJournalModStat", corpo_de_prova!(tramp_journal_mod_stat)));
    let ok_7e_undo = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsJournalModMergeUndo", "BwmsJournalModMergeUndo", corpo_de_prova!(tramp_journal_mod_merge_undo)));
    let ok_7e_pin = registro_de_prova!(register_argful_by_types(reg, proto, &["Int32", "Int32"], "BwmsJournalModTestPinHash", "BwmsJournalModTestPinHash", corpo_de_prova!(tramp_journal_mod_test_pin_hash)));
    let ok_85d_f = register_argful_by_types(reg, proto, &["Uint64", "Int32", "Int32"], "BwmsNodeInstanceF32", "BwmsNodeInstanceF32", tramp_node_instance_f32 as NativeHandler);
    let ok_85d_u = register_argful_by_types(reg, proto, &["Uint64", "Int32", "Int32"], "BwmsNodeInstanceU64", "BwmsNodeInstanceU64", tramp_node_instance_u64 as NativeHandler);
    let ok_85d_c = register_argful_by_types(reg, proto, &["Uint64"], "BwmsNodeInstanceNodeClass", "BwmsNodeInstanceNodeClass", tramp_node_instance_node_class as NativeHandler);
    let ok_237 = register_argful_by_types(reg, proto, &["Uint64", "Int32"], "BwmsNodeInstanceInfo", "BwmsNodeInstanceInfo", tramp_node_instance_info as NativeHandler);
    crate::log_dev!("[reg] register_all: nodeinstance-237 NodeInstanceInfo={ok_237}");
    let ok_85d_campo = register_argful_by_types(reg, proto, &["String", "Int32", "String", "Int32", "Int32"], "BwmsSectorNodeField", "BwmsSectorNodeField", tramp_sector_node_field as NativeHandler);
    let ok_85d_arr = register_argful_by_types(reg, proto, &["String", "Int32", "Int32"], "BwmsSectorNodeArraySize", "BwmsSectorNodeArraySize", tramp_sector_node_array_size as NativeHandler);
    let ok_85c_f = register_argful_by_types(reg, proto, &["String", "Int32", "Int32", "Int32", "Int32", "Int32"], "BwmsSectorNodeArrayF32", "BwmsSectorNodeArrayF32", tramp_sector_node_array_f32 as NativeHandler);
    let ok_85c_u = register_argful_by_types(reg, proto, &["String", "Int32", "Int32", "Int32", "Int32", "Int32"], "BwmsSectorNodeArrayU32", "BwmsSectorNodeArrayU32", tramp_sector_node_array_u32 as NativeHandler);
    crate::log_dev!(
        "[reg] register_all: nodeinstance-85 TraceNodeInstanceCheck={ok_trace_node} NodeInstanceById={ok_node_por_id} SectorInstancedMeshCensus={ok_censo_do_setor} | journal-44 7d-2b-3 NodeAreaVolumeCheck={ok_vol} 7e-2 JournalModStat={ok_7e_stat} JournalModMergeUndo={ok_7e_undo} 7e-3c JournalModTestPinHash={ok_7e_pin} | nodeinstance-85d NodeInstanceF32={ok_85d_f} NodeInstanceU64={ok_85d_u} NodeInstanceNodeClass={ok_85d_c} SectorNodeField={ok_85d_campo} SectorNodeArraySize={ok_85d_arr} SectorNodeArrayF32={ok_85c_f} SectorNodeArrayU32={ok_85c_u}"
    );
}

// Instância de nó, parte 85a (2026-09-12, RE offline do layout da instância de nó): getters de `worldINodeInstance` numa instância
// viva vinda do trace de física (rota do `GetHitObject`), sem declarar o tipo em redscript. Layout: transform `+0x30` (quaternion `+0x40`), scale `+0x50`, nó Handle
// `+0x60`, setup `+0x70` (ponteiro duplo: no boot 75 `[+0x70]` é uma entrada com a própria instância em `+0x8` e o setup em `+0x0`), GlobalNodeID `+0x80`, estado `+0x88`; no setup, `node*` `+0x50`,
// globalNodeID `+0x58`, proxyNodeID `+0x60`, streamingDistance `+0x7C` (2.5). Nó de colisão não tem GlobalNodeID próprio (0 na instância e no setup), então a forma
// do setup é decidida pelo nó, e o GlobalNodeID positivo sai de uma instância achada pelo `FindNode` do registro. O Handle do trace vaza, como no `GetHitObject`.
pub(crate) const NODEINSTANCE_POSICAO_OFF: usize = 0x30;

pub(crate) const NODEINSTANCE_QUAT_OFF: usize = 0x40;

pub(crate) const NODEINSTANCE_ESCALA_OFF: usize = 0x50;

pub(crate) const NODEINSTANCE_NO_OFF: usize = 0x60;

#[cfg(feature = "devtools")]
pub(crate) const NODEINSTANCE_GLOBALID_OFF: usize = 0x80;

pub(crate) const NODEINSTANCE_ESTADO_OFF: usize = 0x88;

#[cfg(feature = "devtools")]
pub(crate) const NODESETUP_GLOBALID_OFF: usize = 0x58;

/// Setup da instância lendo `+0x70` como ponteiro duplo ou simples, cada forma só se o `node*` do setup for o nó da instância.
pub(crate) unsafe fn node_instance_setups(obj: *const u8) -> (Option<*const u8>, Option<*const u8>) {
    let legivel = |q: *const u8, n: usize| rtti::sane(q as *mut c_void) && crate::memoria::is_readable(q as *const c_void, n);
    let rd = |p: *const u8, off: usize| (p.add(off) as *const u64).read_unaligned();
    let no = rd(obj, NODEINSTANCE_NO_OFF);
    let p = rd(obj, NODEINSTANCE_SETUP_OFF) as *const u8;
    let bate_no = |s: &*const u8| no != 0 && rd(*s, NODESETUP_NO_OFF) == no;
    let duplo = if legivel(p, 8) { Some(rd(p, 0) as *const u8).filter(|s| legivel(*s, 0x90)).filter(bate_no) } else { None };
    let simples = Some(p).filter(|s| legivel(*s, 0x90)).filter(bate_no);
    (duplo, simples)
}

pub(crate) const AREASHAPENODE_OUTLINE_OFF: usize = 0x40;

pub(crate) const AREASHAPEOUTLINE_PONTOS_OFF: usize = 0x30;

pub(crate) const AREASHAPEOUTLINE_ALTURA_OFF: usize = 0x40;

/// `FindNode(id)`: Handle forte da instância (`[0, 0]` sem instância); `None` sem registro ou com o slot diferente da RE. Quem chama solta o Handle.
pub(crate) unsafe fn find_node(id: u64) -> Option<[u64; 2]> {
    let (r, f) = node_registry()?;
    let mut out = [0u64; 2];
    core::arch::asm!(
        "blr {f}",
        f = in(reg) f,
        in("x0") r as u64,
        in("x1") id,
        in("x8") out.as_mut_ptr(),
        clobber_abi("C"),
    );
    Some(out)
}

// Instância de nó, partes 85c/85d (2026-09-12): API de leitura pra mods, sem declarar `worldINodeInstance` nem `worldInstancedMeshNode` (categoria CMesh).
// Como no `WorldNodeSetupWrapper`, leitores tipados com os getters compostos em reds (`bwms-world-node.reds`): a instância sai do `FindNode` pelo id (layout provado na
// 85a), o setup pela forma escolhida pelo nó, e o nó de setor pela lista `nodes` do setor no cache do depot, com cada campo no offset do registro RTTI desta
// build (provado na 85b). Os 3 arrays de runtime do `worldInstancedMeshNode` (`+0x50`, `+0x60`, `+0x70`, fora do RTTI; o construtor `0x101b1d840` inicia os
// três) saem pelo tamanho (85c). Só leitura; o Handle da instância é solto em toda chamada.
pub(crate) const NODEINSTANCE_LEITURA_FIM: usize = 0x90;

/// `largura` bytes em `obj + off` se couberem antes de `fim`.
pub(crate) unsafe fn ler_largura_ate(obj: *const u8, off: i32, largura: usize, fim: usize) -> Option<u64> {
    let off = usize::try_from(off).ok()?;
    (off + largura <= fim).then(|| campo_de_largura(obj, off, largura))
}

// ===== Estado e objeto carregado da instância de nó (2026-09-13) =====
/// `WorldNodeInstanceNodeState` (u16 em `NODEINSTANCE_ESTADO_OFF`): bit 0 inicializando, 1 inicializado, 2 anexando, 3 desanexando, 5 anexado, 9 visível.
/// Objeto que cada tipo de instância carrega num `Handle` e a classe base esperada dele. Offsets do layout do Windows, com o do `worldBendedMeshNodeInstance`
/// medido no Mac: 0xC0 em vez de 0xA0 (boot 136, 13/13 com `CMesh` em 0xC0 e nenhum em 0xA0).
pub(crate) const NODEINSTANCE_CARGAS: [(&str, usize, &str); 5] = [
    ("worldMeshNodeInstance", 0x98, "CMesh"),
    ("worldBendedMeshNodeInstance", 0xC0, "CMesh"),
    ("worldStaticDecalNodeInstance", 0xA0, "IMaterial"),
    ("worldFoliageNodeInstance", 0xB8, "CMesh"),
    ("worldEntityNodeInstance", 0xA0, "entEntity"),
];

/// A instância do id pela busca do motor, lida com `node_instance_da_memoria`; o Handle da busca é solto antes de voltar (a instância anexada segue viva no mundo).
pub(crate) unsafe fn node_instance_objeto(id: u64, off: Option<usize>) -> Option<NodeInstanceLida> {
    let mut h = find_node(id)?;
    let r = node_instance_da_memoria(h[0] as *mut c_void, off);
    let _ = crate::jogo::objetos::soltar_handle_contando(&mut h);
    r
}

/// Handle `(instância, bloco de contagem)` de um `ISerializable`: a auto-referência fraca da base (`ref`, +0x08) aponta pro próprio objeto e divide o bloco
/// de contagem com o Handle.
pub(crate) fn auto_referencia_confere(p: u64, contagem: u64, ref_instancia: u64, ref_contagem: u64) -> bool {
    p != 0 && contagem != 0 && ref_instancia == p && ref_contagem == contagem
}

pub(crate) const WEATHER_CURRENT_STATE_INDEX_OFF: usize = 0x90; // dentro de worldRuntimeSystemWeather

pub(crate) const WEATHER_CURRENT_SOURCE_OFF: usize = 0x98;

pub(crate) const WEATHER_PREVIOUS_STATE_INDEX_OFF: usize = 0xB8;

pub(crate) const WEATHER_RUNTIME_CHUNK_LEN: usize = 0xC0; // cobre até PreviouseStateIndex+4 (0xBC) com folga

pub(crate) const WSS_VTBL_VM: u64 = 0x1_07a3_1448;


pub(crate) static WORLD_BLOCKREFS_OFF: AtomicU64 = AtomicU64::new(0);

pub(crate) static WORLD_OFFSETS_OK: AtomicBool = AtomicBool::new(false);

pub(crate) static BLOCOS_APLICADOS: AtomicU64 = AtomicU64::new(0);

// ===== Patch de devices, 51h — pelo `OnDependenciesReady` do serializer (2026-09-12) =====
// Pega o `HashMap<u64, gameCookedDeviceData>` em `data+0x30` de cada `gameDeviceResource` e faz `InsertOrAssign`
// de cada entrada do mapa de cada patch do caminho. Aqui o hook é o da 51g. Layout: `data` `Handle<gameDeviceResourceData>` em `+0x40` do recurso;
// mapa: `indexTable` `+0x00`, `size` `+0x08`, `capacity` `+0x0c`, nós `+0x10` (capacidade `+0x18`, stride `+0x1c`), alocador `+0x28`; nó = `{next u32,
// hashedKey u32, chave u64 +0x08, gameCookedDeviceData +0x10}`, hash da chave = FNV-1a 32 dos 8 bytes. `gameCookedDeviceData` (0x40 B): `unk00` `+0x00`,
// `className` `+0x08`, `children` `DynArray<u64>` `+0x10`, `parents` `+0x20`, `nodePosition` + `unk3C` `+0x30`. A chave que o alvo já tem recebe o valor do
// patch (as listas pelo reserve de `DynArray` do motor; provado no boot 115). v2: chave nova entra por `Emplace`, com o `Reserve` pelo alocador inline
// do mapa (`PoolStorageProxy<game::PoolGMPL_Devices_Data_HMap>`, vtable lida no boot 115) quando não cabe — bloco único `nós + tabela`, como o do motor.
pub(crate) static MESCLA_DE_DEVICES_ATIVA: AtomicBool = AtomicBool::new(false);

pub(crate) static CLASSE_GAMEDEVICERESOURCE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) static DEVICERESOURCE_DATA_OFF: AtomicU64 = AtomicU64::new(0x40);

/// Vtable do alocador inline do mapa (`mapa+0x28`): `0x08` `AllocAligned(tamanho, alinhamento)` → `{memória, tamanho}`, `0x20` `Free(&bloco)`.
pub(crate) const ALOCADOR_DO_MAPA_DE_DEVICES_VT: u64 = 0x1_0786_4bb8;

/// Retorno de `AllocAligned` (`x0`/`x1`).
#[repr(C)]
pub(crate) struct BlocoAlocado {
    pub(crate) memoria: u64,
    pub(crate) tamanho: u64,
}

/// Mapa de devices de um `gameDeviceResource` (`data` → `+0x30`), `None` sem dado legível.
pub(crate) unsafe fn mapa_de_devices(res: u64) -> Option<*mut u8> {
    let off = DEVICERESOURCE_DATA_OFF.load(Ordering::Relaxed);
    if res == 0 || !crate::memoria::is_readable(res as *const c_void, (off + 0x10) as usize) {
        return None;
    }
    let dado = ((res + off) as *const u64).read_unaligned();
    (dado != 0 && crate::memoria::is_readable(dado as *const c_void, 0x68)).then_some((dado + 0x30) as *mut u8)
}

/// Nós do mapa: `(tabela, capacidade, nós, stride, capacidade dos nós)`, conferidos legíveis.
pub(crate) unsafe fn partes_do_mapa(mapa: *const u8) -> Option<(*const u32, usize, *mut u8, usize)> {
    let tabela = (mapa as *const u64).read_unaligned() as *const u32;
    let cap = (mapa.add(0x0c) as *const u32).read_unaligned() as usize;
    let nos = (mapa.add(0x10) as *const u64).read_unaligned() as *mut u8;
    let cap_nos = (mapa.add(0x18) as *const u32).read_unaligned() as usize;
    let stride = (mapa.add(0x1c) as *const u32).read_unaligned() as usize;
    let ok = cap > 0 && cap <= 0x10_0000 && !tabela.is_null() && !nos.is_null() && stride >= 0x50 && cap_nos >= cap
        && crate::memoria::is_readable(tabela as *const c_void, cap * 4) && crate::memoria::is_readable(nos as *const c_void, cap_nos * stride);
    ok.then_some((tabela, cap, nos, stride))
}

/// `DynArray<u64>` (cabeçalho em `arr`) com o conteúdo de `origem`; `false` sem capacidade.
pub(crate) unsafe fn copiar_lista_u64(arr: *mut u8, origem: *const u8) -> bool {
    let (ob, on, _) = dynarray_no_campo(origem, 0);
    if on > 0 && !crate::memoria::is_readable(ob as *const c_void, on * 8) {
        return false;
    }
    let (_, _, cap) = dynarray_no_campo(arr, 0);
    if on > cap {
        crate::jogo::listas::reservar(arr as *mut c_void, on as u32, 8, 8, std::ptr::null());
        if dynarray_no_campo(arr, 0).2 < on {
            return false;
        }
    }
    if on > 0 {
        std::ptr::copy(ob, dynarray_no_campo(arr, 0).0, on * 8);
    }
    (arr.add(0x0c) as *mut u32).write_unaligned(on as u32);
    true
}

/// `HashMap::Reserve` pelo alocador inline do mapa (vtable conferida): bloco único `nós + tabela`, nós rehasheados em ordem, bloco antigo liberado.
/// Só troca o mapa depois de mover todos os nós; `false` sem mexer nele.
pub(crate) unsafe fn reservar_mapa(mapa: *mut u8, nova_cap: usize) -> bool {
    let vt = (mapa.add(0x28) as *const u64).read_unaligned();
    if vt != crate::rebase(ALOCADOR_DO_MAPA_DE_DEVICES_VT) as u64 || !crate::memoria::is_readable(vt as *const c_void, 0x28) {
        return false;
    }
    let tam = (mapa.add(0x08) as *const u32).read_unaligned() as usize;
    let cap = (mapa.add(0x0c) as *const u32).read_unaligned() as usize;
    let stride = (mapa.add(0x1c) as *const u32).read_unaligned() as usize;
    let nos = (mapa.add(0x10) as *const u64).read_unaligned();
    let tabela = (mapa as *const u64).read_unaligned();
    if stride != 0x50 || nova_cap <= cap || nova_cap > 0x40_0000 {
        return false;
    }
    // O bloco antigo tem a forma do `Reserve` (tabela logo depois dos nós): só assim o `Free` do início dos nós libera o bloco inteiro.
    let cap_nos = (mapa.add(0x18) as *const u32).read_unaligned() as usize;
    if cap > 0 && (nos == 0 || tabela != nos + (cap_nos * stride) as u64 || partes_do_mapa(mapa).is_none()) {
        return false;
    }
    let bytes = nova_cap * (stride + 4);
    let alocar: unsafe extern "C" fn(*const u8, u32, u32) -> BlocoAlocado = std::mem::transmute(((vt + 0x08) as *const u64).read_unaligned());
    let liberar: unsafe extern "C" fn(*const u8, *const [u64; 2]) = std::mem::transmute(((vt + 0x20) as *const u64).read_unaligned());
    let bloco = alocar(mapa.add(0x28), bytes as u32, 8);
    if bloco.memoria == 0 {
        return false;
    }
    if (bloco.tamanho as usize) < bytes {
        liberar(mapa.add(0x28), &[bloco.memoria, 0]);
        return false;
    }
    std::ptr::write_bytes(bloco.memoria as *mut u8, 0, bytes);
    let novos = bloco.memoria as *mut u8;
    let nova_tabela = novos.add(nova_cap * stride) as *mut u32;
    for i in 0..nova_cap {
        nova_tabela.add(i).write_unaligned(u32::MAX);
    }
    let mut k = 0usize;
    for b in 0..cap {
        let mut idx = (tabela as *const u32).add(b).read_unaligned();
        while idx != u32::MAX && k < nova_cap && (idx as usize) < cap_nos {
            let velho = (nos as *const u8).add(idx as usize * stride);
            let h = (velho.add(4) as *const u32).read_unaligned();
            let novo = novos.add(k * stride);
            std::ptr::copy_nonoverlapping(velho, novo, stride);
            let balde = nova_tabela.add(h as usize % nova_cap);
            (novo as *mut u32).write_unaligned(balde.read_unaligned());
            balde.write_unaligned(k as u32);
            idx = (velho as *const u32).read_unaligned();
            k += 1;
        }
    }
    if k != tam {
        liberar(mapa.add(0x28), &[bloco.memoria, 0]);
        return false;
    }
    if cap > 0 {
        liberar(mapa.add(0x28), &[nos, 0]);
    }
    (mapa as *mut u64).write_unaligned(nova_tabela as u64);
    (mapa.add(0x0c) as *mut u32).write_unaligned(nova_cap as u32);
    (mapa.add(0x10) as *mut u64).write_unaligned(bloco.memoria);
    (mapa.add(0x18) as *mut u32).write_unaligned(nova_cap as u32);
    // `k` nós tirados em sequência de uma lista nova (`k < nova_cap`): próximo livre e usados = `k`, como o `GetNextAvailNode`.
    (mapa.add(0x20) as *mut u32).write_unaligned(k as u32);
    (mapa.add(0x24) as *mut u32).write_unaligned(k as u32);
    #[cfg(feature = "devtools")]
    A51H_CRESCIMENTOS.fetch_add(1, Ordering::Relaxed);
    true
}

/// `Emplace` com chave que o mapa não tem (`valor` = `gameCookedDeviceData` do patch, listas pelo reserve do motor). `None` sem nó livre ou sem
/// capacidade (o `Reserve` vem antes); `Some(listas copiadas)` com a chave inserida.
pub(crate) unsafe fn inserir_no_mapa(mapa: *mut u8, chave: u64, valor: *const u8) -> Option<bool> {
    let (tabela, cap, nos, stride) = partes_do_mapa(mapa)?;
    let tam = (mapa.add(0x08) as *const u32).read_unaligned() as usize;
    let cap_nos = (mapa.add(0x18) as *const u32).read_unaligned();
    let (prox, usados) = ((mapa.add(0x20) as *const u32).read_unaligned(), (mapa.add(0x24) as *const u32).read_unaligned());
    if tam + 1 > cap || stride != 0x50 || (prox != u32::MAX && prox >= cap_nos) {
        return None;
    }
    let (idx, novo_prox, novos_usados) = crate::jogo::mapas::proximo_livre(prox, usados, cap_nos, |i| (nos.add(i as usize * stride) as *const u32).read_unaligned())?;
    let no = nos.add(idx as usize * stride);
    let h = crate::jogo::mapas::hash_da_chave(chave);
    std::ptr::write_bytes(no, 0, stride);
    (no.add(4) as *mut u32).write_unaligned(h);
    (no.add(8) as *mut u64).write_unaligned(chave);
    let dst = no.add(0x10);
    std::ptr::copy_nonoverlapping(valor, dst, 0x10);
    std::ptr::copy_nonoverlapping(valor.add(0x30), dst.add(0x30), 0x10);
    let listas = copiar_lista_u64(dst.add(0x10), valor.add(0x10)) && copiar_lista_u64(dst.add(0x20), valor.add(0x20));
    let balde = (tabela as *mut u32).add(h as usize % cap);
    (no as *mut u32).write_unaligned(balde.read_unaligned());
    balde.write_unaligned(idx);
    (mapa.add(0x20) as *mut u32).write_unaligned(novo_prox);
    (mapa.add(0x24) as *mut u32).write_unaligned(novos_usados);
    (mapa.add(0x08) as *mut u32).write_unaligned((tam + 1) as u32);
    Some(listas)
}

/// 51h, no `OnDependenciesReady` (thread de job): entradas de cada patch da tabela cujo alvo é este `gameDeviceResource`.
pub(crate) unsafe fn mesclar_devices(alvo: u64) {
    #[cfg(feature = "devtools")]
    A51H_DEVICES.fetch_add(1, Ordering::Relaxed);
    let caminho = hash_do_caminho_do_recurso(alvo);
    let patches: Vec<(u64, [u64; 2])> = match PATCHES_DE_RECURSO.try_read() {
        Ok(g) => g.do_alvo(caminho).map(|p| (p.patch, p.token)).collect(),
        Err(_) => return,
    };
    let Some(mapa) = (if patches.is_empty() { None } else { mapa_de_devices(alvo) }) else { return };
    for (patch, pt) in patches {
        let pres = if pt[0] == 0 { 0 } else { recurso_do_patch_da_classe(pt, 0x50, CLASSE_GAMEDEVICERESOURCE.load(Ordering::Relaxed)) as u64 };
        let Some(pmapa) = (if pres == 0 { None } else { mapa_de_devices(pres) }) else {
            #[cfg(feature = "devtools")]
            if crate::primeiras(&A51H_PATCH_NAO_PRONTO, 8) {
                crate::veredito_dev!("[a51h] patch {patch:#018x} do device {caminho:#018x} ({alvo:#x}) sem mapa pronto (token {:#x})", pt[0]);
            }
            continue;
        };
        let (mut atribuidas, mut falhas) = (0usize, 0usize);
        let entradas = entradas_do_mapa(pmapa);
        let mut novas: Vec<(u64, *const u8)> = Vec::new();
        for (chave, pno) in &entradas {
            let Some(no) = no_do_mapa(mapa, *chave) else {
                novas.push((*chave, (*pno).add(0x10) as *const u8));
                continue;
            };
            let (dst, src) = (no.add(0x10), (*pno).add(0x10) as *const u8);
            #[cfg(feature = "devtools")]
            let iguais_antes = a51h_valor_igual(dst, src);
            std::ptr::copy(src, dst, 0x10);
            std::ptr::copy(src.add(0x30), dst.add(0x30), 0x10);
            if !copiar_lista_u64(dst.add(0x10), src.add(0x10)) || !copiar_lista_u64(dst.add(0x20), src.add(0x20)) {
                falhas += 1;
                continue;
            }
            atribuidas += 1;
            #[cfg(feature = "devtools")]
            if let Ok(mut g) = A51H_AMOSTRAS.try_lock() {
                if g.len() < 32 && (!iguais_antes || g.len() < 8) {
                    g.push([alvo, patch, *chave, iguais_antes as u64]);
                }
            }
        }
        let tam_de = |m: *const u8| (m.add(0x08) as *const u32).read_unaligned() as usize;
        let cap_de = |m: *const u8| (m.add(0x0c) as *const u32).read_unaligned() as usize;
        // Teste (`~/.bwms-hook-a51h-teste`), uma vez: chave sintética com o valor da 1ª entrada do patch — o mapa do mundo vem cheio, então força o `Reserve`.
        #[cfg(feature = "devtools")]
        let mut teste: Option<A51hInsercao> = None;
        #[cfg(feature = "devtools")]
        if A51H_TESTE.load(Ordering::Relaxed) && !entradas.is_empty() && A51H_TESTE_FEITO.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            let origem = entradas[0].0;
            let mut chave = origem ^ 0xb3a5_5eed_0000_0001;
            while no_do_mapa(mapa, chave).is_some() {
                chave = chave.wrapping_add(1);
            }
            let antigas: Vec<u64> = entradas_do_mapa(mapa).iter().step_by(509).take(64).map(|(k, _)| *k).collect();
            teste = Some(A51hInsercao { alvo, chave, origem, tam_antes: tam_de(mapa), cap_antes: cap_de(mapa), inseridas: 0, antigas });
            novas.push((chave, entradas[0].1.add(0x10) as *const u8));
        }
        let (mut inseridas, mut nao_inseridas) = (0usize, 0usize);
        if !novas.is_empty() {
            let (tam, cap) = (tam_de(mapa), cap_de(mapa));
            if tam + novas.len() > cap && !reservar_mapa(mapa, crate::jogo::mapas::capacidade_para(tam, cap, novas.len())) {
                nao_inseridas = novas.len();
            } else {
                for (chave, valor) in &novas {
                    match inserir_no_mapa(mapa, *chave, *valor) {
                        Some(listas) => {
                            inseridas += 1;
                            falhas += !listas as usize;
                        }
                        None => nao_inseridas += 1,
                    }
                }
            }
        }
        #[cfg(feature = "devtools")]
        if let Some(mut t) = teste {
            t.inseridas = inseridas;
            if let Ok(mut g) = A51H_V2_AMOSTRAS.lock() {
                g.push(t);
            }
        }
        #[cfg(feature = "devtools")]
        A51H_ATRIBUIDAS.fetch_add(atribuidas as u64, Ordering::Relaxed);
        #[cfg(feature = "devtools")]
        A51H_INSERIDAS.fetch_add(inseridas as u64, Ordering::Relaxed);
        #[cfg(feature = "devtools")]
        A51H_NOVAS_PENDENTES.fetch_add(nao_inseridas as u64, Ordering::Relaxed);
        crate::veredito_dev!(
            "[a51h] patch {patch:#018x} no device {caminho:#018x} ({alvo:#x}): {} entrada(s) no patch; {atribuidas} atribuída(s), {} chave(s) nova(s) ({inseridas} inserida(s), {nao_inseridas} não), {falhas} falha(s) de lista; mapa {}/{}",
            entradas.len(),
            novas.len(),
            tam_de(mapa),
            cap_de(mapa)
        );
    }
}

// Pós-carga de setor de streaming — candidato #2 de `worldStreamingSector::PostLoad`,
// achado pelo MESMO `postloadprobe` na mesma sessão de 2026-08-12, DIVERGENTE do endereço já
// PROVADO ao vivo sob o rótulo antigo `streaming-apply` (`STREAMING_SL26_VM`/`0x1010c5f74`,
// achado por técnica DIFERENTE — scan manual de slot de vtable, não `new_object`+vtable+0x30).
// RE offline dedicada (2026-08-12, disassembly + varredura completa de
// `__DATA_CONST` por chained-fixups) reconciliou parcialmente a divergência, sem prova ao vivo:
//   - Este candidato (0x101c05360) é referenciado por **1 ÚNICA** entrada em TODO `__DATA_CONST`
//     (vtable-slot exclusivo, não-compartilhado) — sentado em **LOCAL SLOT 6** do seu próprio vtable
//     (delimitado por um par (0,0) de offset-to-top/typeinfo imediatamente antes) — o MESMO slot
//     local já confirmado (por 2 vias independentes) pra `0x1010b0854` (a base
//     "PostLoad" compartilhada por `gameObject`/`entPuppet`) E pra CMesh/MorphTargetMesh via a
//     técnica live já provada — 3 classes convergindo no MESMO slot-local-6, forte evidência de que
//     é genuinamente o slot Itanium-shiftado de `ISerializable::PostLoad` (Windows +0x28 -> Mac +0x30).
//   - O disassembly do corpo (700 bytes, boundary de função confirmada via LC_FUNCTION_STARTS)
//     chama `bl 0x1010b0854` (a "PostLoad" base) como a PRIMEIRA instrução
//     após o prólogo — padrão clássico de override chamando `Super::PostLoad()` antes da lógica
//     própria — e, logo depois, opera sobre `this+0x40`, EXATAMENTE o offset do `NodeBuffer`
//     desta MESMA classe (`SECTOR_NODEBUFFER`).
//   - `STREAMING_SL26_VM`/0x1010c5f74, em contraste, tem **8958 referências** em `__DATA_CONST`
//     (função amplamente COMPARTILHADA por milhares de vtables — não é override específico de
//     classe) e, na instância examinada, senta no LOCAL SLOT 26 de um vtable de 31 métodos
//     DIFERENTE/ADJACENTE (não o mesmo vtable deste candidato) — corpo do disassembly (240 bytes)
//     tem formato de GETTER/agregador (loop com dispatch virtual computando um máximo via `csel`,
//     termina com `ret` devolvendo escalar em w0), não o padrão void/2-args de PostLoad.
//   - **Conclusão**: `0x101c05360` é `worldStreamingSector::PostLoad` — reconfirmado offline em 2026-09-06 e provado ao
//     vivo no boot 44 (prova de 2026-09-11 do PostLoad de setor, parte 4b);
//     é o hook de produção das deleções do `.xl` (`aplicar_delecoes_do_setor`). `SL26`/0x1010c5f74 provavelmente NÃO é PostLoad — é um método
//     amplamente herdado/compartilhado que só COINCIDE em disparar perto da janela de load do
//     setor (por isso o teste de array R/W antigo funcionou — o OBJETO `this` legitimamente tem
//     arrays de node por perto, não porque SL26 EM SI seja quem os popula). Isso NÃO invalida a
//     técnica/prova de R/W do SL26 (mecanismo genérico continua real) — só a alegação de IDENTIDADE
//     ("isto é PostLoad") fica em dúvida.
//   - Gate PRÓPRIO E SEPARADO do SL26: nunca ligar os 2 no mesmo boot (hookam candidatos no mesmo
//     objeto; ver `install_wssector_postload_probe`).
pub(crate) const WORLDSTREAMINGSECTOR_POSTLOAD_VM: u64 = 0x1_01c0_5360;

pub(crate) static ORIG_WSSECTOR_POSTLOAD: AtomicU64 = AtomicU64::new(0);

// Pós-carga de setor, parte 4d (2026-09-11): deleções de node vindas do `.xl`. O `bwms install` (ou o example `emitir_streaming` do `bwms-primitives`) escreve
// `red4ext/bwms-streaming.txt` com `emit_streaming_table`: `<mod> | <setor> | <expectedNodes> | <nodeIndex> | <tipo>` (`bwms_primitives::tabelas::Streaming`).
// Aqui a tabela vira grupos por (mod, hash do setor); no `PostLoad`, cada grupo é conferido inteiro (contagem de setups, índice e vptr da classe) e só
// então aplicado — uma linha ruim cancela o grupo. Tipos com vptr conhecido: `worldStaticMeshNode` (zera o `scale`).
// Parte 4e-1 (2026-09-11): linhas de 7 colunas (`... | <op> | <valores>`, emissor v2) e `worldInstancedMeshNode` — node inteiro zera o scale de
// todas as instâncias do buffer; `subdel <esperadas> <i>...` zera só as pedidas. Op sem suporte recusa o grupo.
#[derive(Clone)]
pub(crate) struct DelecaoDeNode {
    pub(crate) modulo: String,
    pub(crate) setor: u64,
    pub(crate) esperado: usize,
    pub(crate) indice: usize,
    pub(crate) tipo: String,
    /// Vazio na linha de 5 colunas (node inteiro); `subdel` etc. nas de 7.
    pub(crate) op: String,
    pub(crate) valores: String,
}

/// A linha da tabela como a deleção que o `PostLoad` confere, com o setor pelo hash do caminho.
pub(crate) fn linha_de_delecao(l: &bwms_primitives::tabelas::Streaming) -> DelecaoDeNode {
    let (op, valores) = l.op.unwrap_or_default();
    DelecaoDeNode {
        modulo: l.mod_id.to_string(),
        setor: resource_path_hash(l.setor),
        esperado: l.esperado,
        indice: l.indice,
        tipo: l.tipo.to_string(),
        op: op.to_string(),
        valores: valores.to_string(),
    }
}

pub(crate) static DELECOES_DE_SETOR: std::sync::Mutex<Vec<DelecaoDeNode>> = std::sync::Mutex::new(Vec::new());

/// Carrega `bwms-streaming.txt` (substitui a tabela; o arquivo ausente a esvazia, porque o `bwms install` o apaga quando nenhum mod gera deleção e a
/// recarga do `Reload` da fachada de recursos tira a tabela anterior). Linha fora do formato (`bwms_primitives::tabelas::Streaming`) é ignorada com log.
pub(crate) fn streaming_file(path: &str) {
    let conteudo = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return crate::log_dev!("[axl-streaming-xl] não leu '{path}': {e}"),
    };
    let mut tabela = Vec::new();
    for lida in bwms_primitives::tabelas::ler(&conteudo, bwms_primitives::tabelas::Streaming::ler) {
        match lida {
            Ok(l) => tabela.push(linha_de_delecao(&l)),
            Err(linha) => crate::log_dev!("[axl-streaming-xl] linha ignorada: '{linha}'"),
        }
    }
    let n = tabela.len();
    if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
        *t = tabela;
    }
    crate::veredito_dev!("[axl-streaming-xl] {n} deleção(ões) carregada(s) de '{path}'");
}

/// Transform do motor: posição (x, y, z) e orientação (i, j, k, r), com o produto e a inversa do `Transform`: `a * b` tem orientação
/// `(b.q * a.q)` normalizada e posição `b.q` aplicada em `a.p` mais `b.p`; a inversa é o conjugado, com posição `conj` aplicado em `-p`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Xf {
    pub(crate) p: [f32; 3],
    pub(crate) q: [f32; 4],
}

pub(crate) fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ai, aj, ak, ar] = a;
    let [bi, bj, bk, br] = b;
    [
        ar * bi + ai * br + aj * bk - ak * bj,
        ar * bj + aj * br + ak * bi - ai * bk,
        ar * bk + ak * br + ai * bj - aj * bi,
        ar * br - ai * bi - aj * bj - ak * bk,
    ]
}

pub(crate) fn quat_norm(q: [f32; 4]) -> [f32; 4] {
    let m = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    if m != 0.0 { q.map(|v| v / m) } else { q }
}

/// `q` (normalizado) aplicado em `v`: `v + w·t + qv × t`, com `t = 2·(qv × v)`.
pub(crate) fn quat_rot(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [qi, qj, qk, qw] = quat_norm(q);
    let cruz = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let qv = [qi, qj, qk];
    let t = cruz(qv, v).map(|x| x * 2.0);
    let c = cruz(qv, t);
    [v[0] + t[0] * qw + c[0], v[1] + t[1] * qw + c[1], v[2] + t[2] * qw + c[2]]
}

impl Xf {
    pub(crate) fn mul(self, b: Xf) -> Xf {
        let r = quat_rot(b.q, self.p);
        Xf { p: [r[0] + b.p[0], r[1] + b.p[1], r[2] + b.p[2]], q: quat_norm(quat_mul(b.q, self.q)) }
    }

    pub(crate) fn inversa(self) -> Xf {
        let c = [-self.q[0], -self.q[1], -self.q[2], self.q[3]];
        Xf { p: quat_rot(c, self.p.map(|v| -v)), q: c }
    }
}

/// Aplica os grupos (mod, setor) da tabela que casam com o setor. Confere tudo antes de gravar. A releitura do que foi gravado e dos vizinhos só
/// alimenta o veredito de dev: no público cada plano só grava.
#[cfg_attr(not(feature = "devlog"), allow(unused_variables))]
pub(crate) unsafe fn aplicar_delecoes_do_setor(this: *mut c_void, hash: u64) {
    let grupos: Vec<(String, Vec<DelecaoDeNode>)> = match DELECOES_DE_SETOR.lock() {
        Ok(t) => {
            let mut g: Vec<(String, Vec<DelecaoDeNode>)> = Vec::new();
            for d in t.iter().filter(|d| d.setor == hash) {
                match g.iter_mut().find(|(m, _)| *m == d.modulo) {
                    Some((_, v)) => v.push(d.clone()),
                    None => g.push((d.modulo.clone(), vec![d.clone()])),
                }
            }
            g
        }
        Err(_) => return,
    };
    if grupos.is_empty() {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(SECTOR_NODEBUFFER) as *const u64).read_unaligned() as *mut u8;
    let setups = ((b.add(SECTOR_NODEBUFFER + 0x08) as *const u32).read_unaligned() / SETUP_SIZE as u32) as usize;
    let legivel = !setups_ptr.is_null() && setups > 0 && crate::memoria::is_readable(setups_ptr as *const c_void, setups * SETUP_SIZE);
    for (modulo, linhas) in grupos {
        let planos: Result<Vec<PlanoDelecao>, String> = linhas.iter().map(|d| planejar_delecao(d, setups_ptr, setups, legivel)).collect();
        let planos = match planos {
            Ok(planos) => planos,
            Err(m) => {
                crate::veredito_dev!("[axl-streaming-xl] mod={modulo} setor={hash:#018x} linhas={} veredito=grupo recusado ({m}), nada gravado", linhas.len());
                continue;
            }
        };
        #[cfg(feature = "devlog")]
        let (mut zerados, mut relidos, mut intactos) = (0usize, 0usize, true);
        for p in &planos {
            match p {
                PlanoDelecao::ScaleSetup(i) => {
                    let e = setups_ptr.add(i * SETUP_SIZE + 0x20) as *mut f32;
                    for k in 0..3 {
                        e.add(k).write_unaligned(0.0);
                    }
                    #[cfg(feature = "devlog")]
                    {
                        zerados += 1;
                        if (0..3).all(|k| e.add(k).read_unaligned() == 0.0) {
                            relidos += 1;
                        }
                    }
                }
                PlanoDelecao::Instancias { dados, inicio, n, faixa, vizinho } => {
                    #[cfg(feature = "devlog")]
                    let antes: Vec<[f32; 3]> = (*inicio..inicio + n).map(|j| instancia_scale(*dados, j)).collect();
                    #[cfg(feature = "devlog")]
                    let vizinho_antes = vizinho.map(|j| instancia_scale(*dados, j));
                    for j in faixa {
                        let s = dados.add(j * INSTANCIA_TAM + INSTANCIA_SCALE_OFF) as *mut f32;
                        for k in 0..3 {
                            s.add(k).write_unaligned(0.0);
                        }
                    }
                    #[cfg(feature = "devlog")]
                    {
                        zerados += faixa.len();
                        relidos += faixa.iter().filter(|j| instancia_scale(*dados, **j) == [0.0; 3]).count();
                        for (k, j) in (*inicio..inicio + n).enumerate() {
                            if !faixa.contains(&j) && instancia_scale(*dados, j) != antes[k] {
                                intactos = false;
                            }
                        }
                        if vizinho.map(|j| instancia_scale(*dados, j)) != vizinho_antes {
                            intactos = false;
                        }
                    }
                }
                PlanoDelecao::Destrutiveis { dados, inicio, n, faixa } => {
                    #[cfg(feature = "devlog")]
                    let antes: Vec<f32> = (*inicio..inicio + n).map(|j| destrutivel_z(*dados, j)).collect();
                    for j in faixa {
                        (dados.add(j * DESTRUTIVEL_TAM + DESTRUTIVEL_Z_OFF) as *mut f32).write_unaligned(DESTRUTIVEL_Z_DELETADO);
                    }
                    #[cfg(feature = "devlog")]
                    {
                        zerados += faixa.len();
                        relidos += faixa.iter().filter(|j| destrutivel_z(*dados, **j) == DESTRUTIVEL_Z_DELETADO).count();
                        for (k, j) in (*inicio..inicio + n).enumerate() {
                            if !faixa.contains(&j) && destrutivel_z(*dados, j) != antes[k] {
                                intactos = false;
                            }
                        }
                    }
                }
                PlanoDelecao::Atores { atores, n, faixa } => {
                    #[cfg(feature = "devlog")]
                    let antes: Vec<i32> = (0..*n).map(|j| ator_z(*atores, j)).collect();
                    for j in faixa {
                        (atores.add(j * ATOR_TAM + ATOR_Z_OFF) as *mut i32).write_unaligned(ATOR_Z_DELETADO);
                    }
                    #[cfg(feature = "devlog")]
                    {
                        zerados += faixa.len();
                        relidos += faixa.iter().filter(|j| ator_z(*atores, **j) == ATOR_Z_DELETADO).count();
                        for (j, z) in antes.iter().enumerate() {
                            if !faixa.contains(&j) && ator_z(*atores, j) != *z {
                                intactos = false;
                            }
                        }
                    }
                }
                PlanoDelecao::ShapesSemColisao { no, shapes } => {
                    let Ok((dados, bytes, l)) = listas_de_colisao(*no) else {
                        #[cfg(feature = "devlog")]
                        {
                            intactos = false;
                        }
                        continue;
                    };
                    let nome = crate::cname::cname(PRESET_SEM_COLISAO);
                    let atual = std::slice::from_raw_parts(dados as *const u8, bytes);
                    let (dados, listas, preset) = match indice_do_preset(atual, &l, nome) {
                        Some(i) => (dados, l, i),
                        None => {
                            let Some((novo, listas, i)) = buffer_com_preset(atual, &l, nome) else { continue };
                            // Buffer novo pelo alocador do próprio `RawBuffer`; o antigo fica: soltar pode deixar pendurado quem guardou o ponteiro, e o
                            // custo é o tamanho de um nó.
                            let raw = no.add(COLISAO_DADOS_OFF);
                            let alinhamento = (raw.add(0x0c) as *const u32).read_unaligned();
                            let p = crate::jogo::buffers::realocar_rawbuffer(raw.add(0x10), 0, novo.len() as u32, alinhamento) as *mut u8;
                            if p.is_null() {
                                continue;
                            }
                            std::ptr::copy_nonoverlapping(novo.as_ptr(), p, novo.len());
                            (raw as *mut u64).write_unaligned(p as u64);
                            (raw.add(8) as *mut u32).write_unaligned(novo.len() as u32);
                            for k in 0..9 {
                                (no.add(COLISAO_ATORES_OFF + 4 * k) as *mut u32).write_unaligned(listas.offsets[k]);
                                (no.add(COLISAO_NUM_ATORES_OFF + 2 * k) as *mut u16).write_unaligned(listas.contagens[k]);
                            }
                            (p, listas, i)
                        }
                    };
                    for s in shapes {
                        dados.add(listas.offsets[1] as usize + s * 0x20 + SHAPE_PRESET_OFF).write(preset);
                    }
                    #[cfg(feature = "devlog")]
                    {
                        zerados += shapes.len();
                        relidos += shapes.iter().filter(|s| dados.add(listas.offsets[1] as usize + **s * 0x20 + SHAPE_PRESET_OFF).read() == preset).count();
                    }
                }
                PlanoDelecao::Doador { indice, doador, vizinho } => {
                    let e = setups_ptr.add(indice * SETUP_SIZE);
                    #[cfg(feature = "devlog")]
                    let setup_viz = |v: usize| std::slice::from_raw_parts(setups_ptr.add(v * SETUP_SIZE) as *const u8, SETUP_SIZE).to_vec();
                    #[cfg(feature = "devlog")]
                    let viz_antes = vizinho.map(setup_viz);
                    (e.add(NODESETUP_NO_OFF) as *mut u64).write_unaligned(*doador);
                    #[cfg(feature = "devlog")]
                    {
                        zerados += 1;
                        if (e.add(NODESETUP_NO_OFF) as *const u64).read_unaligned() == *doador {
                            relidos += 1;
                        }
                        if vizinho.map(setup_viz) != viz_antes {
                            intactos = false;
                        }
                    }
                }
                PlanoDelecao::Generica { indice, doador, vizinho } => {
                    let e = setups_ptr.add(indice * SETUP_SIZE);
                    #[cfg(feature = "devlog")]
                    let setup_viz = |v: usize| std::slice::from_raw_parts(setups_ptr.add(v * SETUP_SIZE) as *const u8, SETUP_SIZE).to_vec();
                    #[cfg(feature = "devlog")]
                    let viz_antes = vizinho.map(setup_viz);
                    (e.add(0x08) as *mut f32).write_unaligned(-2000.0);
                    (e.add(0x58) as *mut u64).write_unaligned(0);
                    // -1 desde a versão 68 de setor: nó fantasma não tem item cozido.
                    (e.add(crate::diagnostico::SETUP_INDICE_DE_INSTANCIA) as *mut i32).write_unaligned(-1);
                    (e.add(NODESETUP_NO_OFF) as *mut u64).write_unaligned(*doador);
                    #[cfg(feature = "devlog")]
                    {
                        zerados += 1;
                        let relido = (e.add(0x08) as *const f32).read_unaligned() == -2000.0
                            && (e.add(0x58) as *const u64).read_unaligned() == 0
                            && (e.add(crate::diagnostico::SETUP_INDICE_DE_INSTANCIA) as *const i32).read_unaligned() == -1
                            && (e.add(NODESETUP_NO_OFF) as *const u64).read_unaligned() == *doador;
                        if relido {
                            relidos += 1;
                        }
                        if vizinho.map(setup_viz) != viz_antes {
                            intactos = false;
                        }
                    }
                }
                PlanoDelecao::Campo { no, off, largura, valor, soma } => {
                    let e = no.add(*off);
                    let novo = if *largura == 4 {
                        let atual = (e as *const u32).read_unaligned();
                        let v = if *soma { atual.wrapping_add(*valor as u32) } else { *valor as u32 };
                        (e as *mut u32).write_unaligned(v);
                        v as u64
                    } else {
                        (e as *mut u64).write_unaligned(*valor);
                        *valor
                    };
                    #[cfg(feature = "devlog")]
                    {
                        zerados += 1;
                        let lido = if *largura == 4 { (e as *const u32).read_unaligned() as u64 } else { (e as *const u64).read_unaligned() };
                        if lido == novo {
                            relidos += 1;
                        }
                    }
                }
                PlanoDelecao::SubNoI32 { dados, tam, j, off, valores, inicio, n } => {
                    #[cfg(feature = "devlog")]
                    let copia = |k: usize| std::slice::from_raw_parts(dados.add(k * tam) as *const u8, *tam).to_vec();
                    #[cfg(feature = "devlog")]
                    let outros_antes: Vec<Vec<u8>> = (*inicio..inicio + n).filter(|k| k != j).map(copia).collect();
                    let e = dados.add(j * tam + off);
                    for (k, v) in valores.iter().enumerate() {
                        (e.add(k * 4) as *mut i32).write_unaligned(*v);
                    }
                    #[cfg(feature = "devlog")]
                    {
                        zerados += 1;
                        if valores.iter().enumerate().all(|(k, v)| (e.add(k * 4) as *const i32).read_unaligned() == *v) {
                            relidos += 1;
                        }
                        let outros_depois: Vec<Vec<u8>> = (*inicio..inicio + n).filter(|k| k != j).map(copia).collect();
                        if outros_depois != outros_antes {
                            intactos = false;
                        }
                    }
                }
                PlanoDelecao::SubNo { dados, tam, j, off, valores, inicio, n } => {
                    #[cfg(feature = "devlog")]
                    let copia = |k: usize| std::slice::from_raw_parts(dados.add(k * tam) as *const u8, *tam).to_vec();
                    #[cfg(feature = "devlog")]
                    let outros_antes: Vec<Vec<u8>> = (*inicio..inicio + n).filter(|k| k != j).map(copia).collect();
                    let e = dados.add(j * tam + off);
                    for (k, v) in valores.iter().enumerate() {
                        (e.add(k * 4) as *mut f32).write_unaligned(*v);
                    }
                    #[cfg(feature = "devlog")]
                    {
                        zerados += 1;
                        if valores.iter().enumerate().all(|(k, v)| (e.add(k * 4) as *const f32).read_unaligned() == *v) {
                            relidos += 1;
                        }
                        let outros_depois: Vec<Vec<u8>> = (*inicio..inicio + n).filter(|k| k != j).map(copia).collect();
                        if outros_depois != outros_antes {
                            intactos = false;
                        }
                    }
                }
                PlanoDelecao::SubNoLocal { dados, j, inicio, n, setup, pos, rot } => {
                    let tam = DESTRUTIVEL_TAM;
                    #[cfg(feature = "devlog")]
                    let copia = |k: usize| std::slice::from_raw_parts(dados.add(k * tam) as *const u8, tam).to_vec();
                    #[cfg(feature = "devlog")]
                    let outros_antes: Vec<Vec<u8>> = (*inicio..inicio + n).filter(|k| k != j).map(copia).collect();
                    let e = dados.add(j * tam) as *mut f32;
                    let local = Xf {
                        p: [e.read_unaligned(), e.add(1).read_unaligned(), e.add(2).read_unaligned()],
                        q: [e.add(4).read_unaligned(), e.add(5).read_unaligned(), e.add(6).read_unaligned(), e.add(7).read_unaligned()],
                    };
                    let mut mundo = local.mul(*setup);
                    if let Some(p) = pos {
                        mundo.p = *p;
                    }
                    if let Some(q) = rot {
                        mundo.q = *q;
                    }
                    let novo = mundo.mul(setup.inversa());
                    let valores = [novo.p[0], novo.p[1], novo.p[2], 0.0, novo.q[0], novo.q[1], novo.q[2], novo.q[3]];
                    for (k, v) in valores.iter().enumerate() {
                        e.add(k).write_unaligned(*v);
                    }
                    #[cfg(feature = "devlog")]
                    {
                        zerados += 1;
                        if valores.iter().enumerate().all(|(k, v)| e.add(k).read_unaligned() == *v) {
                            relidos += 1;
                        }
                        let outros_depois: Vec<Vec<u8>> = (*inicio..inicio + n).filter(|k| k != j).map(copia).collect();
                        if outros_depois != outros_antes {
                            intactos = false;
                        }
                    }
                }
                PlanoDelecao::Setup { indice, off, valores, vizinho } => {
                    let e = setups_ptr.add(indice * SETUP_SIZE + off);
                    #[cfg(feature = "devlog")]
                    let setup_viz = |v: usize| std::slice::from_raw_parts(setups_ptr.add(v * SETUP_SIZE) as *const u8, SETUP_SIZE).to_vec();
                    #[cfg(feature = "devlog")]
                    let viz_antes = vizinho.map(setup_viz);
                    for (k, v) in valores.iter().enumerate() {
                        (e.add(k * 4) as *mut f32).write_unaligned(*v);
                    }
                    #[cfg(feature = "devlog")]
                    {
                        zerados += 1;
                        if valores.iter().enumerate().all(|(k, v)| (e.add(k * 4) as *const f32).read_unaligned() == *v) {
                            relidos += 1;
                        }
                        if vizinho.map(setup_viz) != viz_antes {
                            intactos = false;
                        }
                    }
                }
            }
        }
        #[cfg(feature = "devlog")]
        crate::veredito_dev!(
            "[axl-streaming-xl] mod={modulo} setor={hash:#018x} linhas={} veredito={} linha(s) aplicada(s): {zerados} valor(es) gravado(s) (scale 0, Z -2000, troca de nó ou transform do setup), {relidos} relido(s), vizinhos intactos={intactos}",
            linhas.len(),
            linhas.len()
        );
    }
}

/// `(hash do path, nodes.size, setups, setup[0].node == nodes[idx], idx)` de um setor já passado pelo bind.
pub(crate) unsafe fn setor_resumo(this: *mut c_void) -> Option<(u64, u32, u32, bool, u32)> {
    if !crate::memoria::is_readable(this, 0xe0) {
        return None;
    }
    let b = this as *const u8;
    let hash = (b.add(0x30) as *const u64).read_unaligned();
    let setups = (b.add(SECTOR_NODEBUFFER + 0x08) as *const u32).read_unaligned() / SETUP_SIZE as u32;
    let nos = (b.add(SECTOR_NODES + 0x0C) as *const u32).read_unaligned();
    // `bate`/`idx` só alimentam o veredito de dev: no público o PostLoad paga só a 1ª syscall e o hash.
    #[cfg(feature = "devlog")]
    let (bate, idx) = {
        let setups_ptr = (b.add(SECTOR_NODEBUFFER) as *const u64).read_unaligned() as *const u8;
        let nodes_ptr = (b.add(SECTOR_NODES) as *const u64).read_unaligned() as *const u8;
        let (mut bate, mut idx) = (false, u32::MAX);
        if setups > 0 && !setups_ptr.is_null() && crate::memoria::is_readable(setups_ptr as *const c_void, SETUP_SIZE) {
            idx = ((setups_ptr.add(SETUP_NO_INDICE) as *const u64).read_unaligned() & SETUP_NO_INDICE_MASCARA) as u32;
            let no_setup = (setups_ptr.add(NODESETUP_NO_OFF) as *const u64).read_unaligned();
            let slot = if nodes_ptr.is_null() { std::ptr::null() } else { nodes_ptr.wrapping_add(idx as usize * 0x10) };
            if idx < nos && !slot.is_null() && crate::memoria::is_readable(slot as *const c_void, 0x10) {
                let no_arr = (slot as *const u64).read_unaligned();
                bate = no_arr != 0 && no_arr == no_setup;
            }
        }
        (bate, idx)
    };
    #[cfg(not(feature = "devlog"))]
    let (bate, idx) = (false, u32::MAX);
    Some((hash, nos, setups, bate, idx))
}

pub(crate) unsafe extern "C" fn wssector_postload_replacement(this: *mut c_void, a1: u64, a2: u64) {
    let f: PostLoadFn = std::mem::transmute(crate::ganchos::original_publicado(&ORIG_WSSECTOR_POSTLOAD));
    f(this, a1, a2);
    let n = POSTLOAD_CNT_WSSECTOR_C2.fetch_add(1, Ordering::Relaxed) + 1;
    if let Some((hash, nos, setups, bate, idx)) = setor_resumo(this) {
        // Censos e testes de setor são descoberta: só no dev, na mesma ordem em volta da aplicação das deleções de mod (produto).
        #[cfg(feature = "devtools")]
        {
            setor_censo_static_mesh(this, hash);
            setor_censo_spawners(this, hash);
            setor_delecao_teste(this, hash);
        }
        aplicar_delecoes_do_setor(this, hash);
        #[cfg(feature = "devtools")]
        {
            setor_teste_instancias(this, hash);
            setor_teste_no_cozido(this, hash);
            setor_teste_colisao_cozida(this, hash);
            setor_teste_destrutiveis(this, hash);
            setor_teste_colisao(this, hash);
            setor_teste_anuncio(this, hash);
            setor_teste_generica(this, hash);
            setor_teste_mutacao(this, hash);
            setor_teste_subno(this, hash);
            setor_teste_campo(this, hash);
        }
        let alvo = hash == SETOR_ALWAYS_LOADED_0_HASH;
        if (alvo && crate::primeiras(&WSSECTOR_ALVO_LOGS, 4)) || (!alvo && n <= 3) {
            let rotulo = if alvo { " (always_loaded_0)" } else { "" };
            crate::veredito_dev!(
                "[axl-streaming-c2] PostLoad#{n} setor={hash:#018x}{rotulo} nodes={nos} setups={setups} setup0.idx={idx} setup0.node==nodes[idx]={bate}"
            );
        }
    }
    if n == 100 || n == 1000 || n == 10000 {
        crate::veredito_dev!("[axl-streaming-c2] {n} PostLoad de setor");
    }
}

/// Instala o hook da parte 4b (no `on_load`, fora do `dev_mode`) com `~/.bwms-hook-wssector-postload-c2` ou com a tabela
/// `red4ext/bwms-streaming.txt` (deleções do `.xl`). Nunca junto do gate
/// `streaming-sl26`, que hooka outro candidato no mesmo objeto.
pub(crate) unsafe fn install_wssector_postload_probe() {
    let tem_tabela_de_streaming = std::path::Path::new(&crate::mods_do_usuario::mods_dir()).parent().map_or(false, |r| r.join(bwms_primitives::tabelas::STREAMING.arquivo).is_file());
    if !bwms_hook_enabled_global("wssector-postload-c2") && !tem_tabela_de_streaming {
        return;
    }
    // Os testes de setor que leem estas flags (`setor_delecao_teste`, `setor_teste_*`) só rodam no dev.
    #[cfg(feature = "devtools")]
    {
        WSSECTOR_DELETE_TESTE.store(bwms_hook_enabled_global("wssector-delete-teste"), Ordering::Relaxed);
        WSSECTOR_INST_TESTE.store(bwms_hook_enabled_global("wssector-inst-teste"), Ordering::Relaxed);
    }
    install_hook_probe!(
        WORLDSTREAMINGSECTOR_POSTLOAD_VM, wssector_postload_replacement, ORIG_WSSECTOR_POSTLOAD,
        &format!("[axl-streaming-c2] worldStreamingSector::PostLoad hook instalado @ {WORLDSTREAMINGSECTOR_POSTLOAD_VM:#012x}"),
        "[axl-streaming-c2] FALHA ao hookar worldStreamingSector::PostLoad"
    );
}


/// Nó da chave (pelo balde `hash % capacidade` e a cadeia `next`), ou `None`.
pub(crate) unsafe fn no_do_mapa(mapa: *const u8, chave: u64) -> Option<*mut u8> {
    let (tabela, cap, nos, stride) = partes_do_mapa(mapa)?;
    let h = crate::jogo::mapas::hash_da_chave(chave);
    let mut idx = tabela.add(h as usize % cap).read_unaligned();
    for _ in 0..4096 {
        if idx == u32::MAX {
            return None;
        }
        let no = nos.add(idx as usize * stride);
        if (no.add(4) as *const u32).read_unaligned() == h && (no.add(8) as *const u64).read_unaligned() == chave {
            return Some(no);
        }
        idx = (no as *const u32).read_unaligned();
    }
    None
}

/// Chaves e nós de um mapa, percorrendo todos os baldes (até 0x10_0000 nós).
pub(crate) unsafe fn entradas_do_mapa(mapa: *const u8) -> Vec<(u64, *mut u8)> {
    let Some((tabela, cap, nos, stride)) = partes_do_mapa(mapa) else { return Vec::new() };
    let mut v = Vec::new();
    for b in 0..cap {
        let mut idx = tabela.add(b).read_unaligned();
        let mut passos = 0;
        while idx != u32::MAX && passos < 4096 && v.len() < 0x10_0000 {
            let no = nos.add(idx as usize * stride);
            v.push(((no.add(8) as *const u64).read_unaligned(), no));
            idx = (no as *const u32).read_unaligned();
            passos += 1;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `apply-N5`: as 2 linhas de um `.xl` com 2 journals levam o mesmo `mod_id` (`emit_journal_table` faz um laço em `xl.journals`). As voltas do tick
    /// montam o plano dos dois, sem repetir o plano de um journal já planejado, e a mescla de cada raiz pega só os journals com plano dela.
    #[test]
    fn xl_com_2_journals_monta_os_2_planos_e_cada_raiz_pega_os_seus() {
        let voltas = [
            ("ModA/quests", "mod\\a\\um.journal", "raiz"),
            ("ModA/quests", "mod\\a\\dois.journal", "raiz"),
            ("ModA/quests", "mod\\a\\tres.journal", "outra"),
            ("ModA/quests", "mod\\a\\um.journal", "raiz"),
        ];
        let mut planos: Vec<PlanoDeRaiz> = Vec::new();
        for (mod_id, caminho, raiz_id) in voltas {
            if !plano_montado(&planos, caminho) {
                planos.push(PlanoDeRaiz { mod_id: mod_id.into(), caminho: caminho.into(), raiz_id: raiz_id.into(), topo: Vec::new(), entradas: 0 });
            }
        }
        let caminhos: Vec<&str> = planos.iter().map(|p| p.caminho.as_str()).collect();
        assert_eq!(caminhos, ["mod\\a\\um.journal", "mod\\a\\dois.journal", "mod\\a\\tres.journal"]);
        assert_eq!(caminhos_com_plano_da_raiz(&planos, "raiz"), ["mod\\a\\um.journal", "mod\\a\\dois.journal"]);
        assert_eq!(caminhos_com_plano_da_raiz(&planos, "outra"), ["mod\\a\\tres.journal"]);
        assert!(caminhos_com_plano_da_raiz(&planos, "sem plano").is_empty());
    }

    /// `hash:HL-N5`: o `pathHash` encadeado id a id bate com o FNV-1a 32 dos `u16` LE concatenados; caminho vazio = offset basis.
    #[test]
    fn quest_caminho_hash_encadeado_bate_com_os_bytes_le_concatenados() {
        assert_eq!(quest_caminho_hash(&[1, 2, 0xffff]), bwms_primitives::fnv1a32(&[1, 0, 2, 0, 0xff, 0xff]));
        assert_eq!(quest_caminho_hash(&[0x1234]), bwms_primitives::fnv1a32(&[0x34, 0x12]));
        assert_eq!(quest_caminho_hash(&[]), bwms_primitives::FNV1A32_OFFSET);
        assert_eq!(quest_caminho_hash(&[]), bwms_primitives::fnv1a32(&[]));
    }
}
