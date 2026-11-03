//! NPCs: comunidades (os grupos que o jogo põe na rua), spawners e entidades dinâmicas — criar, ligar, desligar e apagar NPC, com os eventos de quem ouve.

#![allow(dead_code)]

use crate::jogo::sessao::sistema_da_sessao;
use crate::entidades::*;
use crate::mundo::*;
// Só os handlers de prova usam, e o `corpo_de_prova!` público não os cita.
#[cfg(feature = "devtools")]
use crate::diagnostico::{tramp_c82_anotar, tramp_spawner_census_count, tramp_spawner_census_id, COMMUNITY_HASHES_ARQUIVO};
use crate::tipos_do_motor::*;
use crate::salvamento::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
#[cfg(feature = "devtools")]
use crate::cname::cname;
use crate::rtti::{self, Registry};
use crate::register::*;
/// Bloco deste arquivo no `PROVA_NO_PUBLICO` (register.rs).
#[cfg(not(feature = "devtools"))]
const BLOCO_DE_PROVA: u8 = 0b010;
use crate::missoes_e_mapa::*;


/// Registra `func -> handler` (chamado quando o POD é construído).
pub(crate) unsafe fn add_route(func: *mut c_void, handler: NativeHandler) {
    incluir_rota(&ROTA_BITS, &NATIVE_ROUTES, &NATIVE_ROUTE_COUNT, func as usize, handler);
}

/// Só de teste (decisão do dono D5, 2026-10-03): o despacho NÃO usa esta
/// função. A regra viva é `bwms_primitives::targets::bate_por_tags`, chamada pelo `alvo_casa`, e com a
/// entidade fora de `dynent_tags()` ela BLOQUEIA; aqui a falta de dado deixa passar.
/// `criteria` = todas as tags de `DynamicEntityTarget`s associados ao handler (via `AddTarget`).
/// `entity_tags` = o conjunto de tags REAIS da entidade do evento, se conhecido (`None` = entidade
/// não rastreada em `dynent_tags()` — indeterminado, NUNCA "sem tags", ver fail-open abaixo).
#[cfg(test)]
pub(crate) fn dynamic_entity_tag_targets_allow(
    criteria: &[u64],
    entity_tags: Option<&std::collections::HashSet<u64>>,
) -> bool {
    if criteria.is_empty() {
        // Nenhum DynamicEntityTarget associado — este filtro não se aplica (comportamento
        // idêntico ao pré-existente, mesma regra do #134).
        return true;
    }
    match entity_tags {
        // Entidade não rastreada em `dynent_tags()` (nunca teve `AssignTag` chamado pra ela) —
        // FAIL-OPEN, nunca inventa "não bate" sem dado concreto.
        None => true,
        // Semântica OR: `.Tag(a)` + `.Tag(b)` no mesmo handler (2 AddTarget) = "qualquer uma".
        Some(tags) => criteria.iter().any(|c| tags.contains(c)),
    }
}

/// `DynamicEntityTarget.Tag(tag: CName) -> ref<DynamicEntityTarget>` — fábrica estática, 1 arg
/// CName. A plural `Tags(tags: array<CName>)` é `tramp_dyt_tags` (abaixo).
pub(crate) unsafe extern "C" fn tramp_dyt_tag(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    let mut obj: *mut c_void = std::ptr::null_mut();
    if let Some(reg) = rtti::Registry::obtain() {
        obj = rtti::new_object(&reg, "DynamicEntityTarget");
    }
    if !obj.is_null() {
        retain_strong_ref_leaked(obj); // fix preventivo 2026-08-12: mesma retenção do #6/#134, aplicada ANTES de qualquer filtro ler esta tabela de volta (evita repetir o bug de heap corruption)
        if let Ok(mut states) = DYNAMICENTITYTARGET_STATES.lock() {
            states.push((obj as usize, Box::new([tag])));
        }
    } else {
        crate::log_dev!("[dyt] DynamicEntityTarget.Tag: new_object('DynamicEntityTarget') falhou");
    }
    crate::log_dev!(
        "[dyt] DynamicEntityTarget.Tag('{}' hash {tag:#018x}) -> {obj:p} (fábrica estática, mesmo mecanismo PROVADO 2026-08-05 (ver ComponentTarget))",
        crate::cname::resolve_cname(tag)
    );
    rtti::make_handle(out, obj);
}

/// `DynamicEntityTarget.Tags(tags: array<CName>) -> ref<DynamicEntityTarget>` —
/// 2026-08-18: implementa a fábrica PLURAL que `tramp_dyt_tag` (acima) tinha deixado de fora por
/// falta de marshalling de `array<T>` como parâmetro (mecanismo agora existe, ver
/// `rtti::read_params_consuming_with_cnamearray`). Insere CADA tag do array no MESMO conjunto
/// que `.Tag(x)` (fábrica singular) já usa, sem nenhuma diferença de semântica de despacho (o
/// casamento itera as tags com OR — verdadeiro na 1ª tag que bater). Feito
/// aqui com UMA linha `(obj, tags)` em `DYNAMICENTITYTARGET_STATES` (a MESMA tabela do `.Tag(x)`,
/// que grava `[tag]`), array vazio inclusive (`gravar_alvo_por_tags`): alvo com `tags` vazio
/// casa com tudo, e o alvo sem linha sairia dos alvos suportados
/// no despacho. `alvo_casa` (`corpo_e_rosto`) acha a linha do ponteiro e `bate_por_tags` faz OR
/// entre as tags dela; entre alvos `alvos_barram_o_disparo` (`corpo_e_rosto`) faz `any`, então N
/// tags do MESMO objeto dão o mesmo resultado que N objetos `.Tag(x)` separados via `AddTarget`
/// múltiplo.
pub(crate) unsafe extern "C" fn tramp_dyt_tags(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    alvo_por_tags(frame, out, "DynamicEntityTarget", &DYNAMICENTITYTARGET_STATES);
}

/// Forja `DynamicEntityTarget extends CallbackSystemTarget` + registra `Tag`/`Tags`.
pub unsafe fn register_dynamicentitytarget(reg: &Registry) -> String {
    let cst = reg.class_by_name("CallbackSystemTarget");
    if !rtti::sane(cst) {
        return "[reg] register_dynamicentitytarget: 'CallbackSystemTarget' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "DynamicEntityTarget", "gameuiInGameMenuGameController", cst);
    if forged.is_null() {
        return "[reg] register_dynamicentitytarget: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_dynamicentitytarget: classe=OK mas sem protótipo — Tag/Tags NÃO registrados".into(),
    };
    let m1 = register_method(reg, "DynamicEntityTarget", proto, "Tag", "Tag", tramp_dyt_tag, true);
    let m2 = register_method(reg, "DynamicEntityTarget", proto, "Tags", "Tags", tramp_dyt_tags, true);
    format!(
        "[reg] DynamicEntityTarget: classe=OK (INSTANCIÁVEL, parent=CallbackSystemTarget real) Tag={m1} Tags={m2} \
— is_static+construct+handle NUMA CLASSE FORJADA: PROVADO 2026-08-05; Tags(array<CName>) codado 2026-08-18, prova ao vivo pendente"
    )
}

// Layout REAL do motor — não é chute:
//   Community        : id:EntityID@0x08, active@0x10, initialized@0x11,
//                      entries:DynArray<CommunityEntry*>@0x18, template@0x38
//   CommunityEntry   : phases:DynArray<CName>@0x08, name@0x20, spawner:CommunityEntrySpawner*@0x30
//   CommunityEntrySpawner: restoredEntityIDs@0x18, spawnedEntityIDs@0x28, reservedEntityIDs@0x48
// `DynArray<T>` do RED = { entries:ptr@0x00, capacity:u32@0x08, size:u32@0x0C } (`register::dynarray_em`).
pub(crate) const COMMUNITY_ENTRIES_OFF: usize = 0x18;

/// Anda uma `Community` REAL e conta os spawners pendurados nas entries.
///
/// **Por que isto substitui a varredura por `HashMap`.** O boot de 2026-09-04 rodou o minerador de
/// mapa contra o `CommunitySystem` e deu `mapas=0`. Não era filtro apertado demais: pelo layout
/// do motor **não existe mapa de spawners** — um spawner pende de
/// `Community.entries[i].spawner`. Procurar mapa ali era procurar uma estrutura que o motor não tem.
///
/// `GetCommunity(id)` já é PROVADO (2026-08-14) e a sessão de 08-14 deixou 10.254 IDs de comunidade
/// que resolvem — ou seja, a entrada certa existe e é abundante. Isto pega uma comunidade viva e
/// mostra o que ela contém.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn walk_community_spawners(sys: *mut c_void, community_id: u64, max_entries: usize) -> usize {
    let com = community_get_community(sys, community_id);
    if com == 0 || !crate::memoria::is_readable(com as *const c_void, 0x40) {
        crate::log_dev!("[comwalk] communityID={community_id:#x} -> nenhuma Community");
        return 0;
    }
    let (entries, size, cap) = dynarray_em((com as usize + COMMUNITY_ENTRIES_OFF) as *const c_void);
    crate::log_dev!(
        "[comwalk] communityID={community_id:#x} com={com:#x} entries={entries:#x} size={size} cap={cap}"
    );
    if entries == 0 || size == 0 || !dynarray_coerente(entries, size, cap) || size > (1 << 16) {
        return 0;
    }
    let size = size as usize;
    let mut com_spawners = 0usize;
    for i in 0..size.min(max_entries) {
        let slot = entries + i * 8;
        if !crate::memoria::is_readable(slot as *const c_void, 8) {
            break;
        }
        let entry = (slot as *const u64).read_unaligned() as usize;
        if entry == 0 || !crate::memoria::is_readable(entry as *const c_void, 0x40) {
            continue;
        }
        let nome = rd_u64(entry as *const c_void, COMMUNITY_ENTRY_NAME_OFF);
        let spawner = rd_u64(entry as *const c_void, COMMUNITY_ENTRY_SPAWNER_OFF);
        if spawner != 0 {
            com_spawners += 1;
        }
        crate::log_dev!(
            "[comwalk]   entry#{i} name={} spawner={spawner:#x}",
            crate::cname::resolve_cname(nome)
        );
    }
    crate::log_dev!(
        "[comwalk] communityID={community_id:#x}: {com_spawners} de {} entries têm spawner",
        size.min(max_entries)
    );
    com_spawners
}

#[cfg(feature = "devtools")]
pub(crate) unsafe fn mine_community_spawner_keys(sys: *mut c_void, scan_bytes: usize, max_keys: usize) {
    if sys.is_null() || !crate::memoria::is_readable(sys, 0x40) {
        crate::log_dev!("[spawnermine] CommunitySystem nulo/ilegível");
        return;
    }
    crate::log_dev!("[spawnermine] sys={sys:p} varrendo {scan_bytes:#x} bytes por HashMap do RED");
    let mut mapas = 0usize;
    let mut testadas = 0usize;
    let mut achou = false;
    let mut off = 0usize;
    while off + 0x20 <= scan_bytes {
        let base = sys as usize + off;
        if !crate::memoria::is_readable(base as *const c_void, 0x20) {
            off += 8;
            continue;
        }
        let index_table = rd_u64(base as *const c_void, 0x00) as usize;
        let size = rd_u32(base as *const c_void, 0x08);
        let capacity = rd_u32(base as *const c_void, 0x0c) as usize;
        let nodes = rd_u64(base as *const c_void, 0x10) as usize;
        let stride = rd_u32(base as *const c_void, 0x1c) as usize;
        // Header coerente de `HashMap`: ponteiros legíveis, capacity potência-de-2 plausível,
        // `size <= capacity`, stride de nó pequeno. Mesmos critérios que já filtraram os
        // falso-positivos de ASCII no scan do ResourceLoader.
        let plausivel = index_table != 0
            && nodes != 0
            && stride >= 0x10
            && stride <= 0x40
            && capacity >= 4
            && capacity <= (1 << 20)
            && (size as usize) <= capacity
            && size > 0
            && crate::memoria::is_readable(index_table as *const c_void, 4)
            && crate::memoria::is_readable(nodes as *const c_void, stride);
        if !plausivel {
            off += 8;
            continue;
        }
        mapas += 1;
        crate::log_dev!(
            "[spawnermine] mapa#{mapas} em +{off:#x}: size={size} cap={capacity} stride={stride:#x}"
        );
        // Colhe chaves reais deste mapa e testa cada uma contra GetSpawner.
        let mut colhidas = 0usize;
        for i in 0..capacity {
            if colhidas >= max_keys {
                break;
            }
            let slot = index_table + i * 4;
            if !crate::memoria::is_readable(slot as *const c_void, 4) {
                continue;
            }
            let mut cur = (slot as *const u32).read_unaligned();
            let mut guarda = 0;
            while cur != u32::MAX && guarda < 256 && colhidas < max_keys {
                let node = nodes + (cur as usize) * stride;
                if !crate::memoria::is_readable(node as *const c_void, 0x18) {
                    break;
                }
                let chave = rd_u64(node as *const c_void, 0x08);
                if chave != 0 {
                    colhidas += 1;
                    testadas += 1;
                    let inst = community_get_spawner(sys, chave);
                    if inst != 0 {
                        achou = true;
                        crate::veredito_dev!(
                            "[spawnermine] >>> POSITIVO: mapa+{off:#x} chave={chave:#018x} -> spawner instance={inst:#x} <<<"
                        );
                        if crate::memoria::is_readable(inst as *const c_void, 0x40) {
                            let cls = rtti::class_of(inst as *mut c_void);
                            if !cls.is_null() {
                                let nome = crate::cname::resolve_cname(rtti::type_name_getname(cls));
                                crate::log_dev!("[spawnermine]     CLASSE DO SPAWNER: {nome}");
                            }
                        }
                    }
                }
                cur = rd_u32(node as *const c_void, 0x00);
                guarda += 1;
            }
        }
        off += 8;
    }
    crate::veredito_dev!(
        "[spawnermine] VEREDITO: mapas={mapas} chaves_testadas={testadas} positivo={achou}"
    );
}

/// Acha EMPIRICAMENTE o slot de vtable de `GetPoiMappinPhase`, usando a native
/// VANILLA como gabarito.
///
/// **Por que é preciso (medido ao vivo 2026-09-04).** Com hash de POI REAL, o slot `0x3D0`
/// (= Windows `0x3C8` + shift Itanium `0x08`) devolveu `0xa6340d5e→1905696272`,
/// `0x84fa33a8→1905696464`, `0xbf3a1d89→1905696824` — valores crescendo **0xC0 por chamada**, que é
/// progressão de PONTEIRO, não de enum. A regra do shift não vale pra este slot. E os métodos de
/// `game::mappins::IMappinSystem` não têm símbolo no binário (inlined), então não dá pra achar por
/// nome — sobra achar por COMPORTAMENTO.
///
/// **O gabarito.** `MappinSystem.GetPointOfInterestMappinSavedState(hash, out phase, …)` é native
/// vanilla e devolve a fase do MESMO POI por caminho independente. O caller passa `esperado`; aqui
/// varremos a janela de slots e reportamos os que devolvem exatamente esse valor pra TODOS os
/// hashes testados. Um slot que acerta N hashes distintos não acerta por acaso.
///
/// **Guardas.** Só chama slot cujo ponteiro é legível; o valor de retorno é truncado pra `u32` e
/// comparado — nunca é usado como ponteiro. A janela é limitada pelo caller. É a mesma classe de
/// dispatch que o projeto já faz rotineiramente, só que varrida.
/// Conta quantos slots uma vtable REAL tem, andando enquanto os ponteiros forem de código.
///
/// **Por que isto fecha uma dúvida cara (2026-09-04).** O `#142` usa `vtbl+0x3D0` por analogia com
/// o offset Windows (`0x3C8` + shift Itanium) — isso pressupõe uma vtable com **≥123 slots**.
/// Medindo offline com `re-xref vtable-of/vtable-dump` (subcomandos novos que decodificam os
/// *chained fixups* do `__DATA_CONST`), a vtable da classe do subsistema de POI mappin tem **41
/// slots**, terminando em `0x140`. Se a da `MappinSystem` for da mesma ordem, `0x3D0` cai FORA da
/// tabela — o que explica exatamente o retorno tipo-ponteiro crescendo 0xC0 por chamada. Esta
/// função responde a pergunta na instância REAL, sem depender de analogia.
/// Cache de "quantos slots tem esta vtable", indexado pelo PONTEIRO da vtable.
///
/// **Por que existe (regressão real, achada por bissecção em 2026-09-05).** A 1ª versão da guarda
/// `vtbl_slot_fn_bounded` chamava `vtable_slot_count(obj, 512)` a CADA dispatch. Cada slot custa 2
/// chamadas de `ganchos::is_readable`, e cada uma dessas é um syscall `mach_vm_read_overwrite` — ou
/// seja, até **1024 syscalls por dispatch**, em 7 natives, durante o boot. O jogo passou a morrer
/// com `SIGSEGV` aos 46–60s de forma reprodutível; restaurar o dylib anterior fez o boot voltar a
/// chegar na gameplay. Uma "checagem barata de segurança" que era, na verdade, uma tempestade de
/// syscalls.
///
/// O tamanho da vtable é propriedade da CLASSE, não da instância: medir uma vez por ponteiro de
/// vtable é suficiente e transforma o custo em O(1) amortizado.
pub(crate) static VTABLE_SLOTS_CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<u64, usize>>> =
    std::sync::OnceLock::new();

pub(crate) unsafe fn vtable_slot_count(obj: *mut c_void, teto: usize) -> usize {
    if obj.is_null() || !crate::memoria::is_readable(obj, 8) {
        return 0;
    }
    let vtbl = (obj as *const u64).read_unaligned() as usize;
    if vtbl == 0 {
        return 0;
    }
    let mut n = 0usize;
    while n < teto {
        let slot = vtbl + n * 8;
        if !crate::memoria::is_readable(slot as *const c_void, 8) {
            break;
        }
        let fp = (slot as *const u64).read_unaligned();
        // Ponteiro de código: dentro da imagem do jogo e legível como instrução. A faixa é a do binário (VM): o ponteiro vivo
        // vem com o slide do ASLR e, sem converter, um slide alto corta a contagem (boot 227: 39.508 recusas do slot +0x230).
        let fp_vm = crate::vm_do_jogo(fp);
        if fp_vm < 0x1_0000_0000 || fp_vm >= 0x1_0800_0000 || !crate::memoria::is_readable(fp as *const c_void, 4) {
            break;
        }
        n += 1;
    }
    n
}

// ---------------------------------------------------------------------------------------------
// `CommunityWrapper`/`CommunitySystem` (sessão 2026-08-14, 100% offline): 7 dos 9 métodos do
// `CommunitySystem` são slot de VTABLE em bytes (Windows), NÃO
// endereço de função — mesma categoria zero-RE-de-endereço já usada pelo `MappinSystem`
// acima (`#142`/`#232`) e reconfirmada de novo aqui: só o shift Itanium Mac=Windows+0x08 já
// estabelecido neste projeto. `ActivateCommunity`@0x1B0/`DeactivateCommunity`@0x1B8/
// `SetCommunityPhase`@0x1C0/`ResetCommunity`@0x1C8/`ActivateSpawner`@0x1D8/
// `DeactivateSpawner`@0x1E0/`ResetSpawner`@0x1E8 (offsets Windows, bytes) → Mac = +0x08 cada.
// `GetCommunity`@0x228/`GetSpawner`@0x260 devolvem `WeakPtr<T>&` por out-param e entram
// pelo sret (`mundo::CommunityWeakPtrSret`): `community_get_community`/`community_get_spawner`
// (abaixo), só leitura. As 7 funções ficam gated como MUTAÇÃO
// REAL DE MUNDO (ativam/desativam spawn de NPC/crowd) — nunca chamadas automaticamente por
// nenhum smoke test deste projeto (mesma disciplina já usada pro `SetPoiMappinPhase` acima).
//
// `CommunitySystem` já é acessível neste projeto via native VANILLA REAL confirmada em
// `redscript-src/orphans.script:11401` (`GameInstance.GetCommunitySystem(self) -> ref<
// CommunitySystem>`) — as 7 natives abaixo recebem o ponteiro já resolvido pelo mod (mesmo
// `Arg::Handle`/1º-arg-raw-pointer já usado por dezenas de outras natives, ex.
// `tramp_mappinsystem_*` acima), fazem o dispatch de vtable via `vtbl_slot_fn` (generalizado
// nesta mesma rodada, era `mappinsystem_vtbl_fn`) e chamam — sempre validando
// `crate::memoria::is_readable` em cada passo antes de dereferenciar (idêntico ao MappinSystem).
// `aCommunityID`/`aSpawnerID` são `uint64_t` cru (o hash de um `EntityID`/`gameCommunityID`,
// já confirmado ser `{entityId: EntityID}`); `CName` (nomes de entry/fase) é sempre 8 bytes/hash na ABI Itanium,
// mesma convenção já usada neste projeto pra todo CName passado a native do motor.
pub(crate) const COMMUNITY_ACTIVATE_SLOT: usize = 0x1B0 + 0x08;

pub(crate) const COMMUNITY_DEACTIVATE_SLOT: usize = 0x1B8 + 0x08;

pub(crate) const COMMUNITY_SET_PHASE_SLOT: usize = 0x1C0 + 0x08;

pub(crate) const COMMUNITY_RESET_SLOT: usize = 0x1C8 + 0x08;

pub(crate) const COMMUNITY_SPAWNER_ACTIVATE_SLOT: usize = 0x1D8 + 0x08;

pub(crate) const COMMUNITY_SPAWNER_DEACTIVATE_SLOT: usize = 0x1E0 + 0x08;

pub(crate) const COMMUNITY_SPAWNER_RESET_SLOT: usize = 0x1E8 + 0x08;

pub(crate) type CommunityIdEntryFn = unsafe extern "C" fn(*mut c_void, u64, u64);

pub(crate) type CommunitySetPhaseFn = unsafe extern "C" fn(*mut c_void, u64, u64, u64);


/// Núcleo comum das 4 natives `(sys, communityId, entryName)`. `label` só pro log.
pub(crate) unsafe fn community_id_entry_call(sys: *mut c_void, community_id: u64, entry_name: u64, slot: usize, label: &str) {
    match vtbl_slot_fn(sys, slot) {
        Some(fp) => {
            let f: CommunityIdEntryFn = core::mem::transmute(fp);
            f(sys, community_id, entry_name);
            crate::log_dev!(
                "[communitysystem] {label}(sys={sys:p}, communityId={community_id:#x}, entryName={entry_name:#x}) chamado via vtbl+{slot:#x}"
            );
        }
        None => crate::log_dev!(
            "[communitysystem] {label}: sys={sys:p} inválido/vtable+{slot:#x} ilegível — NÃO chamado"
        ),
    }
}

/// Núcleo comum das 3 natives `(sys, spawnerId)`.
pub(crate) unsafe fn community_spawner_call(sys: *mut c_void, spawner_id: u64, slot: usize, label: &str) {
    match vtbl_slot_fn(sys, slot) {
        Some(fp) => {
            let f: CommunitySpawnerFn = core::mem::transmute(fp);
            f(sys, spawner_id);
            crate::log_dev!(
                "[communitysystem] {label}(sys={sys:p}, spawnerId={spawner_id:#x}) chamado via vtbl+{slot:#x}"
            );
        }
        None => crate::log_dev!(
            "[communitysystem] {label}: sys={sys:p} inválido/vtable+{slot:#x} ilegível — NÃO chamado"
        ),
    }
}

pub(crate) unsafe fn read_args3(frame: *mut c_void) -> (u64, u64, u64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let a = args.first().map(|(v, _)| *v).unwrap_or(0);
    let b = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let c = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    (a, b, c)
}

pub(crate) unsafe extern "C" fn tramp_community_activate(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let (sys, cid, entry) = read_args3(frame);
    community_id_entry_call(sys as *mut c_void, cid, entry, COMMUNITY_ACTIVATE_SLOT, "BwmsCommunityActivate");
}

pub(crate) unsafe extern "C" fn tramp_community_deactivate(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let (sys, cid, entry) = read_args3(frame);
    community_id_entry_call(sys as *mut c_void, cid, entry, COMMUNITY_DEACTIVATE_SLOT, "BwmsCommunityDeactivate");
}

pub(crate) unsafe extern "C" fn tramp_community_reset(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let (sys, cid, entry) = read_args3(frame);
    community_id_entry_call(sys as *mut c_void, cid, entry, COMMUNITY_RESET_SLOT, "BwmsCommunityReset");
}

pub(crate) unsafe extern "C" fn tramp_community_set_phase(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let cid = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let entry = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let phase = args.get(3).map(|(v, _)| *v).unwrap_or(0);
    match vtbl_slot_fn_bounded(sys, COMMUNITY_SET_PHASE_SLOT) {
        Some(fp) => {
            let f: CommunitySetPhaseFn = core::mem::transmute(fp);
            f(sys, cid, entry, phase);
            crate::log_dev!(
                "[communitysystem] BwmsCommunitySetPhase(sys={sys:p}, communityId={cid:#x}, entryName={entry:#x}, phaseName={phase:#x}) chamado via vtbl+{COMMUNITY_SET_PHASE_SLOT:#x}"
            );
        }
        None => crate::log_dev!(
            "[communitysystem] BwmsCommunitySetPhase: sys={sys:p} inválido/vtable+{COMMUNITY_SET_PHASE_SLOT:#x} ilegível — NÃO chamado"
        ),
    }
}

pub(crate) unsafe extern "C" fn tramp_community_spawner_activate(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let sid = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    community_spawner_call(sys, sid, COMMUNITY_SPAWNER_ACTIVATE_SLOT, "BwmsCommunitySpawnerActivate");
}

pub(crate) unsafe extern "C" fn tramp_community_spawner_deactivate(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let sid = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    community_spawner_call(sys, sid, COMMUNITY_SPAWNER_DEACTIVATE_SLOT, "BwmsCommunitySpawnerDeactivate");
}

pub(crate) unsafe extern "C" fn tramp_community_spawner_reset(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let sid = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    community_spawner_call(sys, sid, COMMUNITY_SPAWNER_RESET_SLOT, "BwmsCommunitySpawnerReset");
}


pub(crate) type CommunityGetFn = unsafe extern "C" fn(*mut c_void, u64) -> CommunityWeakPtrSret;

/// Núcleo: chama `GetCommunity(sys, communityId)` via vtable (ABI sret real, ver nota acima),
/// lê o `WeakPtr<Community>` cru devolvido (`{instance,refCount}`) e devolve `instance` (0 =
/// não achou/candidato inválido). LEITURA apenas contra o `CommunitySystem` real (não muta
/// nenhum estado de mundo) — mas É dispatch de vtable genuinamente novo, testado com cautela
/// (1 hash real + 1 fabricado, nunca em loop).
pub(crate) unsafe fn community_get_community(sys: *mut c_void, community_id: u64) -> u64 {
    match vtbl_slot_fn_bounded(sys, COMMUNITY_GET_COMMUNITY_SLOT) {
        Some(fp) => {
            let f: CommunityGetFn = core::mem::transmute(fp);
            let result = f(sys, community_id);
            let instance = result.instance;
            let refcount = result.ref_count;
            crate::log_dev!(
                "[communitysystem] BwmsCommunityGetCommunity(sys={sys:p}, communityId={community_id:#x}) -> instance={instance:#x} refCount={refcount:#x} via vtbl+{COMMUNITY_GET_COMMUNITY_SLOT:#x} (sret)"
            );
            instance
        }
        None => {
            crate::log_dev!(
                "[communitysystem] BwmsCommunityGetCommunity: sys={sys:p} inválido/vtable+{COMMUNITY_GET_COMMUNITY_SLOT:#x} ilegível — NÃO chamado"
            );
            0
        }
    }
}

/// `BwmsCommunityGetCommunity(sys: ref<CommunitySystem>, communityId: Uint64) -> Uint64` —
/// devolve o ponteiro `instance` do `WeakPtr<Community>` (0 = indefinido/não achado). `Uint64`
/// cru em vez de `wref<T>` forjado — mesma divergência de escopo já documentada pro resto do
/// item (nenhum tipo `Community`/`Spawner` foi forjado na RTTI, categoria de risco evitada de
/// propósito).
pub(crate) unsafe extern "C" fn tramp_community_get_community(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let cid = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let instance = community_get_community(sys, cid);
    write_uint_ret(out, instance, 8);
}

// `GetSpawner` (2026-08-14, mesma sessão — irmão de `GetCommunity`, 9/9 slots de vtable):
//   GetSpawner@0x260 = void* (ICommunitySystem::*)(WeakPtr<Spawner>& aOut, uint64_t aSpawnerID)
// Mac = Windows 0x260 + 0x08 (mesmo shift Itanium já estabelecido pra esta classe) = 0x268.
// MESMA técnica exata que fechou `GetCommunity` acima (`CommunityWeakPtrSret`, sret real via
// struct >16 bytes — força o LLVM/aarch64 a popular x8 sozinho, do jeito que o C++ real espera
// pro retorno não-trivial de `WeakPtr<T>`, dtor não-trivial via regra Itanium). Diferença
// consciente vs `GetCommunity`: aqui o header já declara `aSpawnerID` como `uint64_t` PURO (não
// `const uint64_t&` como `aCommunityID` tinha) — bate direto com a ABI real confirmada por
// disassembly pro slot irmão (2º argumento por VALOR em x1, nunca por ponteiro), então passamos
// `spawner_id` por valor sem precisar re-confirmar via disassembly nova.
pub(crate) const COMMUNITY_GET_SPAWNER_SLOT: usize = 0x260 + 0x08;

pub(crate) type CommunityGetSpawnerFn = unsafe extern "C" fn(*mut c_void, u64) -> CommunityWeakPtrSret;

/// Núcleo: chama `GetSpawner(sys, spawnerId)` via vtable (ABI sret real, ver nota acima), lê o
/// `WeakPtr<Spawner>` cru devolvido (`{instance,refCount}`) e devolve `instance` (0 = não
/// achado/candidato inválido). LEITURA apenas contra o `CommunitySystem` real (não muta nenhum
/// estado de mundo).
pub(crate) unsafe fn community_get_spawner(sys: *mut c_void, spawner_id: u64) -> u64 {
    match vtbl_slot_fn_bounded(sys, COMMUNITY_GET_SPAWNER_SLOT) {
        Some(fp) => {
            let f: CommunityGetSpawnerFn = core::mem::transmute(fp);
            let result = f(sys, spawner_id);
            let instance = result.instance;
            let refcount = result.ref_count;
            crate::log_dev!(
                "[communitysystem] BwmsCommunityGetSpawner(sys={sys:p}, spawnerId={spawner_id:#x}) -> instance={instance:#x} refCount={refcount:#x} via vtbl+{COMMUNITY_GET_SPAWNER_SLOT:#x} (sret)"
            );
            instance
        }
        None => {
            crate::log_dev!(
                "[communitysystem] BwmsCommunityGetSpawner: sys={sys:p} inválido/vtable+{COMMUNITY_GET_SPAWNER_SLOT:#x} ilegível — NÃO chamado"
            );
            0
        }
    }
}

/// `BwmsCommunityGetSpawner(sys: ref<CommunitySystem>, spawnerId: Uint64) -> Uint64` — devolve
/// o ponteiro `instance` do `WeakPtr<Spawner>` (0 = indefinido/não achado). `Uint64` cru em vez
/// de `wref<T>` forjado — mesma divergência de escopo consciente já usada em `GetCommunity`
/// (nenhum tipo `Spawner` foi forjado na RTTI).
/// `BwmsCommunitySpawnerMine(sys: ref<CommunitySystem>, scanBytes: Int32, maxKeys: Int32) -> Void`
/// — ponta redscript da mineração de `spawnerID` reais.
///
/// **Por que o sistema vem por argumento e não é resolvido aqui.** A 1ª versão resolvia o
/// `CommunitySystem` pelo `systemMap` (via do `BwmsGetSystemByName`) e o boot de 2026-09-04 provou
/// que não funciona pra esta classe: `"CommunitySystem"` nem existe no RTTI por esse nome, e
/// `gameICommunitySystem` existe mas o `systemMap` devolve nulo
/// (`[spawnermine] classe 'gameICommunitySystem' -> sys=0x0`). O jeito certo é o vanilla:
/// `GameInstance.GetCommunitySystem(game)` (`orphans.script:11401`), que o redscript já tem — daí
/// o sistema chegar pronto por argumento, exatamente como nas outras 9 natives deste item.
/// Anda TODAS as comunidades reais do save e conta quantas têm spawner.
///
/// É o probe decisivo que faltava. Os 10.254 hashes do save **todos resolvem como Community**
/// (medido em 2026-08-14) — então são entrada VÁLIDA e abundante; o erro daquela sessão foi
/// alimentá-los ao `GetSpawner`, que espera id de node do mundo. Aqui eles são usados pra o que
/// realmente são: pegar a Community e olhar dentro dela, onde pelo layout do motor o
/// spawner mora (`Community.entries[i].spawner`).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn walk_all_communities(sys: *mut c_void, max_com: usize, max_entries: usize) {
    let texto = match community_hashes_de_arquivo(COMMUNITY_HASHES_ARQUIVO) {
        Ok(l) => l,
        Err(e) => {
            crate::log_dev!("[comwalk] sem lista de hashes: {e}");
            return;
        }
    };
    let lista: Vec<u64> = texto
        .lines()
        .filter_map(|l| u64::from_str_radix(l.trim().trim_start_matches("0x"), 16).ok())
        .collect();
    crate::log_dev!("[comwalk] varrendo {} comunidades (teto {max_com})", lista.len());
    let mut com_ok = 0usize;
    let mut com_com_entries = 0usize;
    let mut total_spawners = 0usize;
    let mut primeiro: u64 = 0;
    for id in lista.iter().take(max_com) {
        let com = community_get_community(sys, *id);
        if com == 0 || !crate::memoria::is_readable(com as *const c_void, 0x40) {
            continue;
        }
        com_ok += 1;
        // `DynArray` = {entries@0x00, capacity@0x08, size@0x0C}. Esta leitura estava TROCADA (size de `+0x08`), e o crivo `size > cap` virava
        // `capacity > size` — verdadeiro pra todo array com folga —, pulando a comunidade e subcontando spawners (achado 2026-09-18: um sítio que
        // escapou da correção de 2026-09-06 registrada em [[cp77-dynarray-ordem-declaracao]]). Agora pelo decodificador canônico.
        let base = com as usize + COMMUNITY_ENTRIES_OFF;
        let (entries, size, cap) = dynarray_em(base as *const c_void);
        if entries == 0 || size == 0 || !dynarray_coerente(entries, size, cap) || size > (1 << 16) {
            continue;
        }
        let size = size as usize;
        com_com_entries += 1;
        for i in 0..size.min(max_entries) {
            let slot = entries + i * 8;
            if !crate::memoria::is_readable(slot as *const c_void, 8) {
                break;
            }
            let entry = (slot as *const u64).read_unaligned() as usize;
            if entry == 0 || !crate::memoria::is_readable(entry as *const c_void, 0x40) {
                continue;
            }
            let spawner = rd_u64(entry as *const c_void, COMMUNITY_ENTRY_SPAWNER_OFF);
            if spawner != 0 {
                total_spawners += 1;
                if primeiro == 0 {
                    primeiro = spawner;
                    let nome = crate::cname::resolve_cname(rd_u64(entry as *const c_void, COMMUNITY_ENTRY_NAME_OFF));
                    crate::veredito_dev!(
                        "[comwalk] >>> 1º SPAWNER VIVO: communityID={id:#x} entry#{i} name='{nome}' spawner={spawner:#x} <<<"
                    );
                }
            }
        }
    }
    crate::veredito_dev!(
        "[comwalk] VEREDITO: communities_resolvidas={com_ok} com_entries={com_com_entries} spawners_achados={total_spawners} primeiro={primeiro:#x}"
    );
}

/// `BwmsCommunityWalkSpawners(sys: ref<CommunitySystem>, communityId: Uint64, maxEntries: Int32) -> Int32`
/// — devolve quantas entries da comunidade têm spawner. Ponta redscript de `walk_community_spawners`.
/// `BwmsCommunityWalkAll(sys: ref<CommunitySystem>, maxCommunities: Int32, maxEntries: Int32) -> Void`
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_community_walk_all(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let maxc = args.get(1).map(|(v, _)| *v as usize).filter(|v| *v > 0).unwrap_or(10254);
    let maxe = args.get(2).map(|(v, _)| *v as usize).filter(|v| *v > 0 && *v <= 4096).unwrap_or(64);
    if !sys.is_null() {
        walk_all_communities(sys, maxc, maxe);
    }
}

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_community_walk_spawners(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let cid = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let maxe = args.get(2).map(|(v, _)| *v as usize).filter(|v| *v > 0 && *v <= 4096).unwrap_or(64);
    let n = if sys.is_null() { 0 } else { walk_community_spawners(sys, cid, maxe) };
    write_uint_ret(out, n as u64, 4);
}

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_community_spawner_mine(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let scan = args.get(1).map(|(v, _)| *v as usize).filter(|v| *v > 0 && *v <= 0x8000).unwrap_or(0x800);
    let maxk = args.get(2).map(|(v, _)| *v as usize).filter(|v| *v > 0 && *v <= 4096).unwrap_or(64);
    // A varredura por `HashMap` fica como 2º passo (barata, e o `mapas=0` de 2026-09-04 já é dado
    // registrado); o caminho PRINCIPAL agora é o do header: `Community.entries[i].spawner`.
    mine_community_spawner_keys(sys, scan, maxk);
}

pub(crate) unsafe extern "C" fn tramp_community_get_spawner(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let sid = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let instance = community_get_spawner(sys, sid);
    write_uint_ret(out, instance, 8);
}

// `BwmsCommunityScanSpawners` (2026-08-14, continuação — retomada dedicada ao teste decisivo
// de `GetSpawner`). Achado da rodada anterior: os 3 hashes usados até agora (`realId1/2/3`,
// primeiros 3 do dump de 10254 do `save-parser`) são todos de COMUNIDADE (confirmados via
// `GetCommunity`), não de spawner. `save-parser/src/community.rs` confirma que o
// node `CommunitySystem` do save é uma ÚNICA lista de hashes sem distinção estrutural (semântica
// completa desconhecida) — não existe uma 2ª lista separada de spawners pra extrair; extender o
// parser não teria o que achar de novo. Em vez disso, varre TODOS os 10254 hashes já
// extraídos contra os 2 métodos SÓ-LEITURA (`GetCommunity`/`GetSpawner`), embutidos aqui via
// `include_str!` (mesmo hash-set já provado seguro pros 3 primeiros, agora testado por
// completo) — procura um hash onde `GetCommunity==0` (não é comunidade) E `GetSpawner!=0`
// (É spawner), o sinal decisivo de um ID de spawner GENUÍNO. Read-only puro (nenhuma das duas
// funções muta estado de mundo), ~20K chamadas de vtable/binary-search triviais — negligível
// em tempo de CPU, mas loga só o RESUMO + até 20 exemplos de hit de spawner (evita inundar o
// log com 20 mil linhas).
// 🔴 2026-08-22 — as duas listas de hash deixaram de ser `include_str!`.
//
// Elas vinham dos SAVES DE UMA PESSOA (10254 e 10232 hashes extraídos de `AutoSave-5` e
// `EndGameSave-2` de uma partida específica) e eram embutidas no binário — inclusive no build
// PÚBLICO, onde somavam ~380 KB de dado que não era de quem instalou. Duas coisas erradas de
// uma vez: dado derivado do jogo viajando no pacote, e a varredura só fazendo sentido contra
// aquela partida.
//
// Agora a lista vem do save de QUEM ESTÁ JOGANDO, lida em runtime de um arquivo de texto
// (um hash `0x...` por linha). Gerar a lista do próprio save é trabalho do `save-parser` do
// projeto, offline e só-leitura. Sem o arquivo, o comando explica o que falta em vez de
// falhar calado.
#[cfg(feature = "devtools")]
pub(crate) fn community_hashes_de_arquivo(nome: &str) -> Result<String, String> {
    let caminho = crate::caminho_de_marcador(nome)
        .ok_or_else(|| "esta varredura é de descoberta e só existe no build de dev do BWMS.".to_string())?;
    std::fs::read_to_string(&caminho).map_err(|e| {
        format!(
            "lista de hashes não encontrada em ~/{nome} ({e}). \
             Esta varredura roda contra os hashes de comunidade do SEU save — gere o arquivo \
             com o save-parser do projeto (um hash 0x... por linha) e rode de novo."
        )
    })
}

#[cfg(feature = "devtools")]
pub(crate) const COMMUNITY_HASHES_ARQUIVO_ALT: &str = ".bwms-community-hashes-2.txt";

#[cfg(feature = "devtools")]
pub(crate) unsafe fn run_community_scan_spawners(sys: *mut c_void) -> String {
    match community_hashes_de_arquivo(COMMUNITY_HASHES_ARQUIVO) {
        Ok(lista) => run_community_scan_spawners_against(sys, &lista, "save-do-usuario"),
        Err(e) => format!("[communityscan] {e}"),
    }
}

/// Mesma varredura contra uma SEGUNDA lista — útil pra comparar dois pontos de progressão do
/// próprio jogador (a diferença entre saves distantes é o que torna a varredura informativa).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn run_community_scan_spawners_endgamesave2(sys: *mut c_void) -> String {
    match community_hashes_de_arquivo(COMMUNITY_HASHES_ARQUIVO_ALT) {
        Ok(lista) => run_community_scan_spawners_against(sys, &lista, "save-do-usuario-2"),
        Err(e) => format!("[communityscan] {e}"),
    }
}

#[cfg(feature = "devtools")]
pub(crate) unsafe fn run_community_scan_spawners_against(sys: *mut c_void, hash_list: &str, label: &str) -> String {
    if sys.is_null() || !rtti::sane(sys) {
        return format!("[communityscan:{label}] sys inválido — abortado");
    }
    let mut community_only = 0usize;
    let mut spawner_only = 0usize;
    let mut both = 0usize;
    let mut neither = 0usize;
    let mut tested = 0usize;
    let mut spawner_examples: Vec<String> = Vec::new();

    for line in hash_list.lines() {
        let hex = line.trim().trim_start_matches("0x");
        if hex.is_empty() {
            continue;
        }
        let h = match u64::from_str_radix(hex, 16) {
            Ok(v) => v,
            Err(_) => continue,
        };
        tested += 1;

        let c = match vtbl_slot_fn_bounded(sys, COMMUNITY_GET_COMMUNITY_SLOT) {
            Some(fp) => {
                let f: CommunityGetFn = core::mem::transmute(fp);
                f(sys, h).instance
            }
            None => 0,
        };
        let s = match vtbl_slot_fn_bounded(sys, COMMUNITY_GET_SPAWNER_SLOT) {
            Some(fp) => {
                let f: CommunityGetSpawnerFn = core::mem::transmute(fp);
                f(sys, h).instance
            }
            None => 0,
        };

        match (c != 0, s != 0) {
            (true, true) => both += 1,
            (true, false) => community_only += 1,
            (false, true) => {
                spawner_only += 1;
                if spawner_examples.len() < 20 {
                    spawner_examples.push(format!("{h:#018x}(instance={s:#x})"));
                }
            }
            (false, false) => neither += 1,
        }
    }

    format!(
        "[communityscan:{label}] sys={sys:p} testados={tested} community_only={community_only} spawner_only={spawner_only} both={both} nenhum={neither} | spawner_examples=[{}]",
        spawner_examples.join(", ")
    )
}

/// `BwmsCommunityScanSpawners(sys: ref<CommunitySystem>) -> Void` — varredura em lote, ver nota
/// grande acima. Loga o resumo (contagens + até 20 hashes de spawner genuíno, se algum existir).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_community_scan_spawners(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let summary = run_community_scan_spawners(sys);
    crate::log(&summary);
}

/// `BwmsCommunityScanSpawnersEndGame(sys: ref<CommunitySystem>) -> Void` — MESMO mecanismo de
/// `BwmsCommunityScanSpawners`, mas contra a 2ª lista de hashes (`COMMUNITY_HASHES_ARQUIVO_ALT`, no
/// HOME). `sys` é o
/// `CommunitySystem` VIVO do save que o autocontinue carregou nesta sessão de boot (não muda —
/// só o conjunto de hashes testados contra ele muda), pedido explícito de testar contra "outros
/// saves" depois do resultado negativo exaustivo contra a lista de `AutoSave-5` sozinha.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_community_scan_spawners_endgamesave2(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let summary = run_community_scan_spawners_endgamesave2(sys);
    crate::log(&summary);
}

pub(crate) const COMMUNITY_VTABLE_VM: u64 = 0x1_07c6_5b88;

pub(crate) const COMMUNITY_ACTIVE_IDS_VM: u64 = 0x1_0470_9d58;

#[cfg(feature = "devtools")]
pub(crate) const COMMUNITY_ACTIVE_COUNT_VM: u64 = 0x1_0470_9e14;

/// Objeto do wrapper → (comunidade, bloco de referência do `WeakPtr`, entry; 0 no `CommunityWrapper`). Endereço reaproveitado sobrescreve a entrada.
pub(crate) static COMUNIDADES_EMBRULHADAS: Mutex<std::collections::BTreeMap<usize, (u64, u64, u64)>> = Mutex::new(std::collections::BTreeMap::new());

/// `GetCommunity(sys, communityId)` sem log: (instância, bloco de referência) do `WeakPtr<Community>` devolvido.
pub(crate) unsafe fn comunidade_fraca(sys: *mut c_void, id: u64) -> Option<(u64, u64)> {
    let fp = vtbl_slot_fn_bounded(sys, COMMUNITY_GET_COMMUNITY_SLOT)?;
    let f: CommunityGetFn = core::mem::transmute(fp);
    let r = f(sys, id);
    (r.instance != 0 && r.ref_count != 0).then_some((r.instance, r.ref_count))
}

/// `CommunityEntry` da comunidade (travada por quem chama).
pub(crate) unsafe fn comunidade_entries(comunidade: u64) -> Vec<u64> {
    dynarray_u64(comunidade + COMMUNITY_ENTRIES_OFF as u64).into_iter().filter(|e| *e != 0).collect()
}

/// Virtual da `game::Community` no slot `slot`, só com a vtable e o alvo do slot medidos.
pub(crate) unsafe fn comunidade_virtual(comunidade: u64, slot: u64, alvo_vm: u64) -> Option<u64> {
    let vtable = eb_u64(comunidade, 0);
    (vtable == crate::rebase(COMMUNITY_VTABLE_VM) as u64 && eb_u64(vtable, slot * 8) == crate::rebase(alvo_vm) as u64).then_some(crate::rebase(alvo_vm) as u64)
}

/// `GetActiveEntityIDs` do motor num `DynArray<EntityID>` novo, alocado por ele (buffer; capacidade e tamanho).
pub(crate) unsafe fn comunidade_ids_ativos(comunidade: u64) -> Option<[u64; 2]> {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(comunidade_virtual(comunidade, 0, COMMUNITY_ACTIVE_IDS_VM)?);
    let mut arr = [0u64; 2];
    f(comunidade as *mut c_void, arr.as_mut_ptr() as *mut c_void);
    Some(arr)
}

/// `GetActiveEntityCount` do motor.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn comunidade_contagem_ativa(comunidade: u64) -> Option<u32> {
    let f: unsafe extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(comunidade_virtual(comunidade, 1, COMMUNITY_ACTIVE_COUNT_VM)?);
    Some(f(comunidade as *mut c_void))
}

/// Roda `f(comunidade, bloco, entry)` com a comunidade travada; `None` se `c` não é wrapper de comunidade, se a comunidade foi descarregada ou se a
/// entry saiu da lista dela.
pub(crate) unsafe fn com_comunidade<T>(c: *mut c_void, f: impl FnOnce(u64, u64, u64) -> T) -> Option<T> {
    let (comunidade, rc, entry) = COMUNIDADES_EMBRULHADAS.lock().ok()?.get(&(c as usize)).copied()?;
    if !crate::jogo::objetos::travar_fraco(rc) {
        return None;
    }
    let r = (entry == 0 || comunidade_entries(comunidade).contains(&entry)).then(|| f(comunidade, rc, entry));
    crate::jogo::objetos::destravar_fraco(rc);
    r
}

/// Wrapper novo de `classe` sobre `(comunidade, bloco, entry)`, com handle forte.
pub(crate) unsafe fn comunidade_wrapper_novo(classe: &str, comunidade: u64, rc: u64, entry: u64) -> [u64; 2] {
    let Some(reg) = rtti::Registry::obtain() else { return [0, 0] };
    let obj = rtti::new_object(&reg, classe);
    if obj.is_null() {
        return [0, 0];
    }
    if let Ok(mut m) = COMUNIDADES_EMBRULHADAS.lock() {
        m.insert(obj as usize, (comunidade, rc, entry));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    [(buf.as_ptr() as *const u64).read_unaligned(), (buf.as_ptr().add(8) as *const u64).read_unaligned()]
}

/// Grava em `out` um `array` novo de elementos de 8 B com `valores`: buffer do pool do jogo e, depois dele, o alocador copiado do trailer de `fonte` —
/// a técnica do [`eb_array_ret`].
pub(crate) unsafe fn array_u64_ret(out: *mut c_void, valores: &[u64], fonte: u64) {
    let (entradas, cap) = (eb_u64(fonte, 0), eb_u64(fonte, 0x08) as u32 as u64);
    let trailer = if entradas != 0 && cap > 0 { eb_u64((entradas + cap * 8 + 7) & !7, 0) } else { 0 };
    let mut buf: *mut u8 = std::ptr::null_mut();
    if !valores.is_empty() {
        buf = rtti::pool_alloc(valores.len() * 8 + 8, 8) as *mut u8;
        if buf.is_null() {
            core::ptr::write_bytes(out as *mut u8, 0, 16);
            return;
        }
        for (i, v) in valores.iter().enumerate() {
            core::ptr::write_unaligned(buf.add(i * 8) as *mut u64, *v);
        }
        core::ptr::write_unaligned(buf.add(valores.len() * 8) as *mut u64, trailer);
    }
    core::ptr::write_unaligned(out as *mut u64, buf as u64);
    core::ptr::write_unaligned((out as *mut u8).add(8) as *mut u32, valores.len() as u32);
    core::ptr::write_unaligned((out as *mut u8).add(12) as *mut u32, valores.len() as u32);
}

/// Grava em `out` um `Variant` do tipo `tipo` com o handle `h`, que já tem referência forte própria; vazio sem objeto.
pub(crate) unsafe fn variante_de_handle_ret(out: *mut c_void, tipo: *mut c_void, h: [u64; 2]) {
    core::ptr::write_bytes(out as *mut u8, 0, 24);
    if h[0] != 0 {
        core::ptr::write_unaligned(out as *mut u64, tipo as u64 | 1);
        core::ptr::write_unaligned((out as *mut u64).add(1), h[0]);
        core::ptr::write_unaligned((out as *mut u64).add(2), h[1]);
    }
}

/// Cópia, com a comunidade travada, do array de 8 B que `fonte(entry)` aponta; vazio sem entry, sem array ou com a comunidade descarregada.
pub(crate) unsafe fn comunidade_lista_ret(c: *mut c_void, out: *mut c_void, fonte: impl FnOnce(u64) -> Option<u64>) {
    let copiado = com_comunidade(c, |_, _, entry| match (entry != 0).then(|| fonte(entry)).flatten() {
        Some(arr) => {
            array_u64_ret(out, &dynarray_u64(arr), arr);
            true
        }
        None => false,
    });
    if copiado != Some(true) {
        array_u64_ret(out, &[], 0);
    }
}

/// Array de ids `k` do spawner da entry (`None` sem spawner).
pub(crate) unsafe fn spawner_ids(entry: u64, k: usize) -> Option<u64> {
    let spawner = eb_u64(entry, COMMUNITY_ENTRY_SPAWNER_OFF as u64);
    (spawner != 0).then(|| spawner + COMMUNITY_SPAWNER_IDS_OFF[k])
}

/// `BwmsCommunityGetWrapper(sys: ref<CommunitySystem>, communityId: Uint64) -> ref<CommunityWrapper>`: nulo sem comunidade viva pro id.
pub(crate) unsafe extern "C" fn tramp_community_get_wrapper(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map_or(std::ptr::null_mut(), |(v, _)| *v as *mut c_void);
    let id = args.get(1).map_or(0, |(v, _)| *v);
    comunidade_wrapper_ret(out, sys, id);
}

/// `CommunityWrapper` novo no retorno pela comunidade `id` do sistema (nulo sem ela).
pub(crate) unsafe fn comunidade_wrapper_ret(out: *mut c_void, sys: *mut c_void, id: u64) {
    let fraca = if sys.is_null() || id == 0 { None } else { comunidade_fraca(sys, id) };
    #[cfg(feature = "devtools")] crate::selftest::c77_comunidade_entregue(fraca.map_or(0, |f| f.0), fraca.map_or(0, |f| f.1)); // anotação do teste 77, só no dev
    let Some((comunidade, rc)) = fraca else {
        write_handle_ret(out, 0);
        return;
    };
    // Uma referência fraca por bloco segura todos os wrappers da mesma comunidade.
    if COMUNIDADES_EMBRULHADAS.lock().is_ok_and(|m| m.values().any(|e| e.1 == rc)) {
        soltar_fraca(rc);
    }
    entregar_handle(out, comunidade_wrapper_novo("CommunityWrapper", comunidade, rc, 0));
}

pub(crate) unsafe extern "C" fn tramp_comunidade_get_entries(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let feito = com_comunidade(c, |comunidade, rc, _| {
        let handles: Vec<[u64; 2]> = comunidade_entries(comunidade).iter().map(|e| comunidade_wrapper_novo("CommunityEntryWrapper", comunidade, rc, *e)).filter(|h| h[0] != 0).collect();
        eb_array_ret(out, &handles, comunidade + COMMUNITY_ENTRIES_OFF as u64, 8);
    });
    if feito.is_none() {
        eb_array_ret(out, &[], 0, 0);
    }
}

pub(crate) unsafe extern "C" fn tramp_comunidade_get_active_entity_ids(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    match com_comunidade(c, |comunidade, _, _| comunidade_ids_ativos(comunidade)).flatten() {
        Some(arr) => core::ptr::write_unaligned(out as *mut [u64; 2], arr),
        None => core::ptr::write_bytes(out as *mut u8, 0, 16),
    }
}

/// `GetTemplateData() -> Variant` (forma própria: `communityCommunityTemplateData` não é declarado no corpus vanilla).
pub(crate) unsafe extern "C" fn tramp_comunidade_get_template_data(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let tipo = rtti::Registry::obtain().map_or(std::ptr::null_mut(), |r| r.type_by_name("handle:communityCommunityTemplateData"));
    let h = com_comunidade(c, |comunidade, _, _| {
        let h = [eb_u64(comunidade, COMMUNITY_TEMPLATE_OFF), eb_u64(comunidade, COMMUNITY_TEMPLATE_OFF + 8)];
        if rtti::sane(tipo) && h[0] != 0 && crate::jogo::objetos::travar_fraco(h[1]) { h } else { [0, 0] }
    });
    variante_de_handle_ret(out, tipo, h.unwrap_or([0, 0]));
}

pub(crate) unsafe extern "C" fn tramp_cew_is_active(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, com_comunidade(c, |_, _, entry| entry != 0 && eb_u64(entry, COMMUNITY_ENTRY_ACTIVE_OFF) as u32 as i32 > 0).unwrap_or(false));
}

pub(crate) unsafe extern "C" fn tramp_cew_get_name(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, com_comunidade(c, |_, _, entry| if entry == 0 { 0 } else { eb_u64(entry, COMMUNITY_ENTRY_NAME_OFF as u64) }).unwrap_or(0), 8);
}

pub(crate) unsafe extern "C" fn tramp_cew_get_phases(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    comunidade_lista_ret(c, out, |entry| Some(entry + COMMUNITY_ENTRY_PHASES_OFF));
}

pub(crate) unsafe extern "C" fn tramp_cew_get_restored_entity_ids(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    comunidade_lista_ret(c, out, |entry| spawner_ids(entry, 0));
}

pub(crate) unsafe extern "C" fn tramp_cew_get_spawned_entity_ids(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    comunidade_lista_ret(c, out, |entry| spawner_ids(entry, 1));
}

pub(crate) unsafe extern "C" fn tramp_cew_get_spawning_entity_ids(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    comunidade_lista_ret(c, out, |entry| spawner_ids(entry, 2));
}

pub(crate) unsafe extern "C" fn tramp_cew_get_reserved_entity_ids(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    comunidade_lista_ret(c, out, |entry| spawner_ids(entry, 3));
}

/// `GetTemplateEntryData() -> Variant` (forma própria: `communitySpawnEntry` não é declarado no corpus vanilla), com o handle do `spawnEntry` da entry.
pub(crate) unsafe extern "C" fn tramp_cew_get_template_entry_data(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let tipo = rtti::Registry::obtain().map_or(std::ptr::null_mut(), |r| r.type_by_name("handle:communitySpawnEntry"));
    let h = com_comunidade(c, |_, _, entry| {
        let objeto = if entry == 0 || !rtti::sane(tipo) { 0 } else { eb_u64(entry, COMMUNITY_ENTRY_SPAWN_ENTRY_OFF) };
        let mut buf = [0u8; 16];
        if objeto != 0 && rtti::sane(objeto as *mut c_void) {
            rtti::make_handle(buf.as_mut_ptr() as *mut c_void, objeto as *mut c_void);
        }
        [(buf.as_ptr() as *const u64).read_unaligned(), (buf.as_ptr().add(8) as *const u64).read_unaligned()]
    });
    variante_de_handle_ret(out, tipo, h.unwrap_or([0, 0]));
}

/// `CommunityWrapper` (pai `IScriptable`).
pub unsafe fn register_communitywrapper(reg: &Registry) -> String {
    let iscriptable = reg.class_by_name("IScriptable");
    if !rtti::sane(iscriptable) || register_type_instantiable_with_parent(reg, "CommunityWrapper", "gameuiInGameMenuGameController", iscriptable).is_null() {
        return "[reg] register_communitywrapper: forja da classe FALHOU".into();
    }
    let Some(proto) = method_donor_proto(reg) else {
        return "[reg] register_communitywrapper: classe=OK mas sem protótipo".into();
    };
    let c = "CommunityWrapper";
    let r = [
        register_method(reg, c, proto, "GetEntries", "GetEntries", tramp_comunidade_get_entries, false),
        register_method(reg, c, proto, "GetActiveEntityIDs", "GetActiveEntityIDs", tramp_comunidade_get_active_entity_ids, false),
        register_method(reg, c, proto, "GetTemplateData", "GetTemplateData", tramp_comunidade_get_template_data, false),
    ];
    format!("[reg] CommunityWrapper: classe=OK métodos {r:?}")
}

/// `CommunityEntryWrapper` (pai `IScriptable`).
pub unsafe fn register_communityentrywrapper(reg: &Registry) -> String {
    let iscriptable = reg.class_by_name("IScriptable");
    if !rtti::sane(iscriptable) || register_type_instantiable_with_parent(reg, "CommunityEntryWrapper", "gameuiInGameMenuGameController", iscriptable).is_null() {
        return "[reg] register_communityentrywrapper: forja da classe FALHOU".into();
    }
    let Some(proto) = method_donor_proto(reg) else {
        return "[reg] register_communityentrywrapper: classe=OK mas sem protótipo".into();
    };
    let c = "CommunityEntryWrapper";
    let r = [
        register_method(reg, c, proto, "IsActive", "IsActive", tramp_cew_is_active, false),
        register_method(reg, c, proto, "GetName", "GetName", tramp_cew_get_name, false),
        register_method(reg, c, proto, "GetPhases", "GetPhases", tramp_cew_get_phases, false),
        register_method(reg, c, proto, "GetRestoredEntityIDs", "GetRestoredEntityIDs", tramp_cew_get_restored_entity_ids, false),
        register_method(reg, c, proto, "GetSpawnedEntityIDs", "GetSpawnedEntityIDs", tramp_cew_get_spawned_entity_ids, false),
        register_method(reg, c, proto, "GetSpawningEntityIDs", "GetSpawningEntityIDs", tramp_cew_get_spawning_entity_ids, false),
        register_method(reg, c, proto, "GetReservedEntityIDs", "GetReservedEntityIDs", tramp_cew_get_reserved_entity_ids, false),
        register_method(reg, c, proto, "GetTemplateEntryData", "GetTemplateEntryData", tramp_cew_get_template_entry_data, false),
    ];
    format!("[reg] CommunityEntryWrapper: classe=OK métodos {r:?}")
}

pub(crate) unsafe fn register_communitysystem_natives(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &["handle:CommunitySystem", "EntityID", "CName"], "BwmsCommunityActivate", "BwmsCommunityActivate", tramp_community_activate);
    let ok2 = register_argful_by_types(reg, proto, &["handle:CommunitySystem", "EntityID", "CName"], "BwmsCommunityDeactivate", "BwmsCommunityDeactivate", tramp_community_deactivate);
    let okmine = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:CommunitySystem", "Int32", "Int32"], "BwmsCommunitySpawnerMine", "BwmsCommunitySpawnerMine", corpo_de_prova!(tramp_community_spawner_mine)));
    let okwalk = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:CommunitySystem", "Uint64", "Int32"], "BwmsCommunityWalkSpawners", "BwmsCommunityWalkSpawners", corpo_de_prova!(tramp_community_walk_spawners)));
    let okall = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:CommunitySystem", "Int32", "Int32"], "BwmsCommunityWalkAll", "BwmsCommunityWalkAll", corpo_de_prova!(tramp_community_walk_all)));
    crate::log_dev!("[reg] register_all: comunidade BwmsCommunityWalkAll={okall}");
    crate::log_dev!("[reg] register_all: comunidade BwmsCommunityWalkSpawners={okwalk} (Community.entries[i].spawner)");
    crate::log_dev!("[reg] register_all: comunidade BwmsCommunitySpawnerMine={okmine} (minera spawnerID reais do próprio sistema)");
    let ok3 = register_argful_by_types(reg, proto, &["handle:CommunitySystem", "EntityID", "CName"], "BwmsCommunityReset", "BwmsCommunityReset", tramp_community_reset);
    let ok4 = register_argful_by_types(reg, proto, &["handle:CommunitySystem", "EntityID", "CName", "CName"], "BwmsCommunitySetPhase", "BwmsCommunitySetPhase", tramp_community_set_phase);
    let ok5 = register_argful_by_types(reg, proto, &["handle:CommunitySystem", "Uint64"], "BwmsCommunitySpawnerActivate", "BwmsCommunitySpawnerActivate", tramp_community_spawner_activate);
    let ok6 = register_argful_by_types(reg, proto, &["handle:CommunitySystem", "Uint64"], "BwmsCommunitySpawnerDeactivate", "BwmsCommunitySpawnerDeactivate", tramp_community_spawner_deactivate);
    let ok7 = register_argful_by_types(reg, proto, &["handle:CommunitySystem", "Uint64"], "BwmsCommunitySpawnerReset", "BwmsCommunitySpawnerReset", tramp_community_spawner_reset);
    let ok8 = register_argful_by_types(reg, proto, &["handle:CommunitySystem", "Uint64"], "BwmsCommunityGetCommunity", "BwmsCommunityGetCommunity", tramp_community_get_community);
    let ok9 = register_argful_by_types(reg, proto, &["handle:CommunitySystem", "Uint64"], "BwmsCommunityGetSpawner", "BwmsCommunityGetSpawner", tramp_community_get_spawner);
    let ok10 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:CommunitySystem"], "BwmsCommunityScanSpawners", "BwmsCommunityScanSpawners", corpo_de_prova!(tramp_community_scan_spawners)));
    let ok11 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:CommunitySystem"], "BwmsCommunityScanSpawnersEndGame", "BwmsCommunityScanSpawnersEndGame", corpo_de_prova!(tramp_community_scan_spawners_endgamesave2)));
    let ok12 = register_argful_by_types(reg, proto, &["handle:CommunitySystem", "Uint64"], "BwmsCommunityGetWrapper", "BwmsCommunityGetWrapper", tramp_community_get_wrapper as NativeHandler);
    crate::log_dev!("[reg] register_all: comunidade BwmsCommunityGetWrapper={ok12}");
    crate::log_dev!(
        "[reg] register_all: communitysystem Activate={ok1} Deactivate={ok2} Reset={ok3} SetPhase={ok4} SpawnerActivate={ok5} SpawnerDeactivate={ok6} SpawnerReset={ok7} GetCommunity={ok8} GetSpawner={ok9} ScanSpawners={ok10} ScanSpawnersEndGame={ok11}"
    );
}

// ---------------------------------------------------------------------------------------------
// `TagSystem::AssignTag`/`UnassignTag` do motor (2026-09-14, desmonte). O `redTagSystem` fica embutido em `GameTagSystem+0x48`: o handler
// vanilla do `GetAnyMatchingEntity` (`0x101709e88`) passa `gts+0x48` pro `0x104008a34`, que lê o `HashMap<CName tag, DynArray<EntityID>>` em `+0x00` (hash
// de 64 B dobrado em 32) sob a trava de 1 B em `+0x60`. `AssignTag` = `0x1040075e8` e `UnassignTag` = `0x104007b40`, as 2 com `(sistema, EntityID, CName)`
// e a trava exclusiva em `+0x60` (o `Assign` também mexe no mapa de `+0x38` sob a trava `+0x61`); os `AssignTags`/`UnassignTags` do motor
// (`0x104007ae8`/`0x104007fd0`) só repetem essas 2 pra cada tag de uma lista.
pub(crate) const TAG_ASSIGN_VM: u64 = 0x1_0400_75e8;

pub(crate) const TAG_UNASSIGN_VM: u64 = 0x1_0400_7b40;

pub(crate) const GAMETAGSYSTEM_TAGSYSTEM_OFF: usize = 0x48;

// Spawner, parte 80c (2026-09-11): leitura do spawner devolvido pelo `BwmsCommunityGetSpawner` (ponteiro `Uint64`). Layout
// (`active@08`, `initialized@09`, `recordID@40`, `appearanceName@48`, `record Handle@60`, `restored@78`, `spawned@88`,
// `reserved@98`), conferido ao vivo pelo cruzamento `recordID@40` == `[record]+0x40` (TweakDBID do `gamedataTweakDBRecord`, `0x48` B).
pub(crate) const SPAWNER_TAMANHO_LIDO: usize = 0xb8;

/// Ponteiro de `game::PopulationSpawner` legível, a partir do `Uint64` do 1º argumento. O valor vem do script: sem o vptr exato, um Uint64
/// errado ou obsoleto levaria o `spawner_pool_ler` a somar e subtrair na trava de memória alheia.
pub(crate) unsafe fn spawner_de(args: &[(u64, u64)]) -> Option<*const u8> {
    let ptr = args.first().map(|(v, _)| *v).unwrap_or(0) as *mut c_void;
    if !rtti::sane(ptr)
        || !crate::memoria::is_readable(ptr as *const c_void, SPAWNER_TAMANHO_LIDO)
        || rd_u64(ptr as *const c_void, 0) != crate::rebase(crate::jogo::mundo::SPAWNER_VPTR_VM) as u64
    {
        return None;
    }
    Some(ptr as *const u8)
}

/// `BwmsSpawnerIsActive(ptr) -> Bool`: `+0x10`. No Mac o id u64 fica em `+0x08` (no Windows o active fica ali, e a 80c lia esse byte até a
/// RE da 80d-2: `Activate` `0x101667480` grava `+0x10`).
pub(crate) unsafe extern "C" fn tramp_spawner_is_active(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    write_bool_ret(out, spawner_de(&args).map_or(false, |b| b.add(0x10).read() != 0));
}

/// `BwmsSpawnerIsInitialized(ptr) -> Bool`: `+0x11` (`Initialize` `0x1016673f8`).
pub(crate) unsafe extern "C" fn tramp_spawner_is_initialized(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    write_bool_ret(out, spawner_de(&args).map_or(false, |b| b.add(0x11).read() != 0));
}

/// `BwmsSpawnerRecordId(ptr, doRecord) -> Uint64`: `recordID@40` do spawner, ou (com `doRecord`) o TweakDBID lido no record do handle `@60`.
pub(crate) unsafe extern "C" fn tramp_spawner_record_id(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let do_record = args.get(1).map_or(false, |(v, _)| rtti::bool_do_script(*v));
    let id = spawner_de(&args).map_or(0, |b| {
        if !do_record {
            return (b.add(0x40) as *const u64).read_unaligned();
        }
        let rec = (b.add(0x60) as *const u64).read_unaligned() as *const c_void;
        if rtti::sane(rec as *mut c_void) && crate::memoria::is_readable(rec, 0x48) {
            (rec as *const u8).add(0x40).cast::<u64>().read_unaligned()
        } else {
            0
        }
    });
    write_uint_ret(out, id, 8);
}

/// `BwmsSpawnerAppearanceName(ptr) -> CName`.
pub(crate) unsafe extern "C" fn tramp_spawner_appearance_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    write_uint_ret(out, spawner_de(&args).map_or(0, |b| (b.add(0x48) as *const u64).read_unaligned()), 8);
}

/// `BwmsSpawnerIdsCount(ptr, qual) -> Int32`: tamanho de `restored` (0), `spawned` (1) ou `reserved` (2); `-1` inválido ou `size > cap`.
pub(crate) unsafe extern "C" fn tramp_spawner_ids_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let qual = args.get(1).map(|(v, _)| *v as u32).unwrap_or(u32::MAX);
    let off = match qual {
        0 => Some(0x78usize),
        1 => Some(0x88),
        2 => Some(0x98),
        _ => None,
    };
    let n = match (spawner_de(&args), off) {
        (Some(b), Some(o)) => {
            let cap = (b.add(o + 0x08) as *const u32).read_unaligned();
            let tam = (b.add(o + 0x0C) as *const u32).read_unaligned();
            if tam <= cap && tam < 1 << 20 { tam as i32 } else { -1 }
        }
        _ => -1,
    };
    write_uint_ret(out, n as u32 as u64, 4);
}

/// Spawner, parte 80d-1 (2026-09-11): `BwmsSpawnerEntityHash(ptr, qual, i) -> Uint64`, hash do `EntityID` `i` do array `qual` (0 restored
/// `@78`, 1 spawned `@88`, 2 reserved `@98`); 0 com índice fora ou array inválido. O script monta o `EntityID` com `BwmsHashToEntityID`.
pub(crate) unsafe extern "C" fn tramp_spawner_entity_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let qual = args.get(1).map(|(v, _)| *v as u32).unwrap_or(u32::MAX);
    let idx = args.get(2).map(|(v, _)| *v as u32).unwrap_or(u32::MAX);
    let off = match qual {
        0 => Some(0x78usize),
        1 => Some(0x88),
        2 => Some(0x98),
        _ => None,
    };
    let h = match (spawner_de(&args), off) {
        (Some(b), Some(o)) => {
            let ents = (b.add(o) as *const u64).read_unaligned() as *const c_void;
            let cap = (b.add(o + 0x08) as *const u32).read_unaligned();
            let tam = (b.add(o + 0x0C) as *const u32).read_unaligned();
            if tam <= cap && idx < tam && rtti::sane(ents as *mut c_void) && crate::memoria::is_readable(ents, (idx as usize + 1) * 8) {
                (ents as *const u64).add(idx as usize).read_unaligned()
            } else {
                0
            }
        }
        _ => 0,
    };
    write_uint_ret(out, h, 8);
}

/// `BwmsSpawnerPosition(ptr) -> Vector4`: `transform@20` é `WorldTransform` (`WorldPosition` com 3 `FixedPoint` de 17 bits de fração em `+0x00`,
/// `Quaternion` em `+0x10`); W = 1. Spawner inválido = vetor zero.
pub(crate) const SPAWNER_FIXEDPOINT_UM: f32 = 131072.0;

pub(crate) unsafe extern "C" fn tramp_spawner_position(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let mut v = [0u8; 16];
    if let Some(b) = spawner_de(&args) {
        for k in 0..3 {
            let bits = (b.add(0x20 + k * 4) as *const i32).read_unaligned();
            v[k * 4..k * 4 + 4].copy_from_slice(&(bits as f32 / SPAWNER_FIXEDPOINT_UM).to_le_bytes());
        }
        v[12..16].copy_from_slice(&1.0f32.to_le_bytes());
    }
    write_raw16_ret(out, &v);
}

/// `BwmsSpawnerOrientation(ptr) -> Quaternion`: os 16 B de `transform.Orientation` (`@30`); spawner inválido = identidade.
pub(crate) unsafe extern "C" fn tramp_spawner_orientation(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let mut q = [0u8; 16];
    q[12..16].copy_from_slice(&1.0f32.to_le_bytes());
    if let Some(b) = spawner_de(&args) {
        std::ptr::copy_nonoverlapping(b.add(0x30), q.as_mut_ptr(), 16);
    }
    write_raw16_ret(out, &q);
}

// Spawner, parte 80d-2 (2026-09-11, RE offline dos ids de entidade ativa): o objeto é
// `game::PopulationSpawner` (vptr `0x107c6d9a8`). Pool de stubs em `[+0xa8]`: trava de leitor num byte em `+0x30` (o motor soma 1 e, se o bit7 do
// escritor estava ligado, espera ele soltar; solta subtraindo 1) e array A de ponteiros em `+0x08` (cap `+0x10`, tamanho `+0x14`), com o id da
// entidade em `[p+0x40]`. Ativos = ids de A seguidos do `spawnedEntityIDs` (`+0x88`), como o `vt[0x00]`, lidos sem alocar no heap do motor.
pub(crate) const SPAWNER_POOL_OFF: usize = 0xa8;

/// Lê o array A do pool sob a trava de leitor do motor: `f(entries, tamanho)`. `None` sem pool legível ou se o escritor não soltar em ~1M voltas.
pub(crate) unsafe fn spawner_pool_ler<R>(b: *const u8, f: impl FnOnce(*const u64, u32) -> R) -> Option<R> {
    use std::sync::atomic::{AtomicU8, Ordering as Ord};
    let pool = (b.add(SPAWNER_POOL_OFF) as *const u64).read_unaligned() as *const u8;
    if !rtti::sane(pool as *mut c_void) || !crate::memoria::is_readable(pool as *const c_void, 0x38) {
        return None;
    }
    let trava = &*(pool.add(0x30) as *const AtomicU8);
    if trava.fetch_add(1, Ord::SeqCst) & crate::jogo::travas::TRAVA_ESCRITA != 0 {
        let mut voltas = 0u32;
        while trava.load(Ord::SeqCst) & crate::jogo::travas::TRAVA_ESCRITA != 0 {
            voltas += 1;
            if voltas > 1_000_000 {
                trava.fetch_sub(1, Ord::SeqCst);
                return None;
            }
            std::hint::spin_loop();
        }
    }
    let entries = (pool.add(0x08) as *const u64).read_unaligned() as *const u64;
    let cap = (pool.add(0x10) as *const u32).read_unaligned();
    let tam = (pool.add(0x14) as *const u32).read_unaligned();
    let valido = tam <= cap && tam < 1 << 20 && (tam == 0 || crate::memoria::is_readable(entries as *const c_void, tam as usize * 8));
    let r = f(entries, if valido { tam } else { 0 });
    trava.fetch_sub(1, Ord::SeqCst);
    Some(r)
}

/// Tamanho válido do `spawnedEntityIDs` (`+0x88`, cap `+0x90`, tamanho `+0x94`); 0 se incoerente.
pub(crate) unsafe fn spawner_spawned_tam(b: *const u8) -> u32 {
    let cap = (b.add(0x90) as *const u32).read_unaligned();
    let tam = (b.add(0x94) as *const u32).read_unaligned();
    if tam <= cap && tam < 1 << 20 { tam } else { 0 }
}

/// `BwmsSpawnerSelfId(ptr) -> Uint64`: o id u64 que o spawner guarda em `+0x08` (controle do layout: igual ao id pedido ao `GetSpawner`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_spawner_self_id(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    write_uint_ret(out, spawner_de(&args).map_or(0, |b| (b.add(0x08) as *const u64).read_unaligned()), 8);
}

/// `BwmsSpawnerActiveCount(ptr) -> Int32`: tamanho do pool A + tamanho do spawned (o que o `vt[0x08]` soma); spawner inválido = 0.
pub(crate) unsafe extern "C" fn tramp_spawner_active_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let n = spawner_de(&args).map_or(0, |b| spawner_pool_ler(b, |_, tam| tam).unwrap_or(0) + spawner_spawned_tam(b));
    write_uint_ret(out, n as u64, 4);
}

/// `BwmsSpawnerActiveEntityHash(ptr, i) -> Uint64`: o `i`-ésimo id da lista de ativos (pool A, depois spawned); 0 fora do intervalo.
pub(crate) unsafe extern "C" fn tramp_spawner_active_entity_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let i = args.get(1).map(|(v, _)| *v as u32).unwrap_or(u32::MAX);
    let h = spawner_de(&args).map_or(0, |b| {
        let (tam_a, h_a) = spawner_pool_ler(b, |entries, tam| {
            let h = if i < tam {
                let p = entries.add(i as usize).read_unaligned() as *const u8;
                if rtti::sane(p as *mut c_void) && crate::memoria::is_readable(p.add(0x40) as *const c_void, 8) {
                    (p.add(0x40) as *const u64).read_unaligned()
                } else {
                    0
                }
            } else {
                0
            };
            (tam, h)
        })
        .unwrap_or((0, 0));
        if i < tam_a {
            return h_a;
        }
        let j = i - tam_a;
        let ents = (b.add(0x88) as *const u64).read_unaligned() as *const u64;
        if j < spawner_spawned_tam(b) && crate::memoria::is_readable(ents as *const c_void, (j as usize + 1) * 8) {
            ents.add(j as usize).read_unaligned()
        } else {
            0
        }
    });
    write_uint_ret(out, h, 8);
}

/// `BwmsSpawnerActiveCountMotor(ptr) -> Int32`: o `vt[0x08]` (`GetActiveEntityCount`) do próprio motor, só pro smoke cruzar com a contagem
/// reproduzida; -1 sem o vptr exato de `game::PopulationSpawner` (conferido no `spawner_de`). O slot só pega a trava de leitor e soma 2 tamanhos.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_spawner_active_count_motor(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let n = spawner_de(&args).map_or(-1i32, |b| {
        let vt = (b as *const u64).read_unaligned();
        let f: unsafe extern "C" fn(*const u8) -> u32 = std::mem::transmute((vt as *const u64).add(1).read_unaligned());
        f(b) as i32
    });
    write_uint_ret(out, n as u32 as u64, 4);
}

// Fase de quest, parte 35b (2026-09-11): grafo de uma fase de quest carregada, só leitura (RE da parte a): `questQuestPhaseResource.graph`
// (`Handle<graphGraphDefinition>`) em `0x40`; `graphGraphDefinition.nodes` (`DynArray<Handle<graphGraphNodeDefinition>>`) em `0x30`, tamanho `0x3c`.
pub(crate) const QUESTPHASE_GRAPH_OFF: usize = 0x40;


// Fase de quest, parte 35c-2a (2026-09-11, RE offline da edição de grafo): leitura coerente do grafo de uma
// fase carregada, só leitura. Nó: `sockets` `@0x30`, `id` u16 `@0x40`; socket: `connections` `@0x38`, dono (WeakHandle) `@0x48`; conexão: `source`
// `@0x30`, `destination` `@0x40` (instância na 1ª palavra do Handle). `desloc` soma um deslocamento nas leituras de dono e das pontas: é o controle
// negativo (com 8 as igualdades têm que quebrar).
pub(crate) const GRAPHNODE_SOCKETS_OFF: usize = 0x30;

pub(crate) const GRAPHSOCKET_CONNECTIONS_OFF: usize = 0x38;

pub(crate) const GRAPHSOCKET_DONO_OFF: usize = 0x48;

pub(crate) const GRAPHCONNECTION_SOURCE_OFF: usize = 0x30;

pub(crate) const GRAPHCONNECTION_DESTINATION_OFF: usize = 0x40;

/// `[nós, sockets, donos certos, conexões (por socket), coerentes, presentes nos dois lados, ids únicos]` do grafo da fase.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questphase_grafo_estatistica(res: *mut c_void, desloc: usize) -> Option<[i32; 7]> {
    let graph = questphase_grafo_de(res)?;
    let nos = handles_legiveis_em(graph, GRAPHDEF_NODES_OFF, 4096);
    let mut e = [0i32; 7];
    e[0] = nos.len() as i32;
    let mut ids = std::collections::BTreeSet::new();
    for no in &nos {
        ids.insert((no.add(QUESTNODE_ID_OFF) as *const u16).read_unaligned());
        for s in handles_legiveis_em(*no, GRAPHNODE_SOCKETS_OFF, 256) {
            e[1] += 1;
            if (s.add(GRAPHSOCKET_DONO_OFF + desloc) as *const u64).read_unaligned() == *no as u64 {
                e[2] += 1;
            }
            for c in handles_legiveis_em(s, GRAPHSOCKET_CONNECTIONS_OFF, 256) {
                e[3] += 1;
                let src = (c.add(GRAPHCONNECTION_SOURCE_OFF + desloc) as *const u64).read_unaligned() as *const u8;
                let dst = (c.add(GRAPHCONNECTION_DESTINATION_OFF + desloc) as *const u64).read_unaligned() as *const u8;
                if src == s || dst == s {
                    e[4] += 1;
                    let outro = if src == s { dst } else { src };
                    if rtti::sane(outro as *mut c_void)
                        && crate::memoria::is_readable(outro as *const c_void, 0x60)
                        && handles_legiveis_em(outro, GRAPHSOCKET_CONNECTIONS_OFF, 256).contains(&c)
                    {
                        e[5] += 1;
                    }
                }
            }
        }
    }
    e[6] = ids.len() as i32;
    Some(e)
}

// Fase de quest, parte 35c-2b (2026-09-12): tirar e recolocar 1 conexão do grafo de uma fase carregada, sem alocar. A conexão (Handle de 16 B) sai dos
// `connections` dos dois sockets, deslocando os seguintes, e as duas cópias do Handle ficam guardadas aqui — o strong não muda, só troca de dono (do array
// pro estado desta native) —; recolocar devolve os 16 B nas mesmas posições (a retirada garante a capacidade). Controle: conexões em comum entre dois
// sockets de nós diferentes que não se ligam = 0. Só na game thread, numa fase que não executa (o smoke usa o epilogue).
#[cfg(feature = "devtools")]
pub(crate) struct ConexaoTirada {
    pub(crate) socket: usize,
    pub(crate) indice: usize,
    pub(crate) handle: [u64; 2],
    pub(crate) outro: usize,
    pub(crate) indice_outro: usize,
    pub(crate) handle_outro: [u64; 2],
}

#[cfg(feature = "devtools")]
pub(crate) static CONEXAO_TIRADA: std::sync::Mutex<Option<ConexaoTirada>> = std::sync::Mutex::new(None);

/// Índice da conexão `c` nos `connections` do socket.
pub(crate) unsafe fn conexao_indice(socket: *const u8, c: *const u8) -> Option<usize> {
    let (ptr, cap, tam) = handles_arr_em(socket, GRAPHSOCKET_CONNECTIONS_OFF);
    if ptr.is_null() || tam > cap || tam > 256 {
        return None;
    }
    (0..tam as usize).find(|i| ptr.add(i * 2).read_unaligned() == c as u64)
}

/// Tira a 1ª conexão achada (ligando dois sockets diferentes) dos dois lados. 1 = ok; -1 recurso inválido; -2 nenhuma; -8 já há uma tirada.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questphase_conexao_tirar(res: *mut c_void) -> i32 {
    let Ok(mut estado) = CONEXAO_TIRADA.lock() else { return -9 };
    if estado.is_some() {
        return -8;
    }
    let Some(graph) = questphase_grafo_de(res) else { return -1 };
    for no in handles_legiveis_em(graph, GRAPHDEF_NODES_OFF, 4096) {
        for s in handles_legiveis_em(no, GRAPHNODE_SOCKETS_OFF, 256) {
            for c in handles_legiveis_em(s, GRAPHSOCKET_CONNECTIONS_OFF, 256) {
                let src = (c.add(GRAPHCONNECTION_SOURCE_OFF) as *const u64).read_unaligned() as *const u8;
                let dst = (c.add(GRAPHCONNECTION_DESTINATION_OFF) as *const u64).read_unaligned() as *const u8;
                let outro = if src == s { dst } else if dst == s { src } else { continue };
                if outro == s || !rtti::sane(outro as *mut c_void) || !crate::memoria::is_readable(outro as *const c_void, 0x60) {
                    continue;
                }
                let (Some(i), Some(j)) = (conexao_indice(s, c), conexao_indice(outro, c)) else { continue };
                let handle = handles_tira(s, GRAPHSOCKET_CONNECTIONS_OFF, i);
                let handle_outro = handles_tira(outro, GRAPHSOCKET_CONNECTIONS_OFF, j);
                let strong = if handle[1] != 0 { (handle[1] as *const u32).read_unaligned() } else { u32::MAX };
                crate::veredito_dev!(
                    "[a35-conexao] tirada conexão {c:p} do socket {s:p} (índice {i}) e do socket {outro:p} (índice {j}); strong do bloco {strong}"
                );
                *estado = Some(ConexaoTirada { socket: s as usize, indice: i, handle, outro: outro as usize, indice_outro: j, handle_outro });
                return 1;
            }
        }
    }
    -2
}

/// Recoloca a conexão tirada nas mesmas posições (ordem inversa). 1 = ok; -1 nada tirado; -3 sem capacidade.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questphase_conexao_recolocar() -> i32 {
    let Ok(mut estado) = CONEXAO_TIRADA.lock() else { return -9 };
    let Some(t) = estado.take() else { return -1 };
    let ok_outro = handles_insere(t.outro as *const u8, GRAPHSOCKET_CONNECTIONS_OFF, t.indice_outro, t.handle_outro);
    let ok = handles_insere(t.socket as *const u8, GRAPHSOCKET_CONNECTIONS_OFF, t.indice, t.handle);
    let strong = if t.handle[1] != 0 { (t.handle[1] as *const u32).read_unaligned() } else { u32::MAX };
    crate::veredito_dev!("[a35-conexao] recolocada ({ok}/{ok_outro}); strong do bloco {strong}");
    if ok && ok_outro { 1 } else { -3 }
}

/// Controle: conexões em comum entre o 1º socket do 1º nó e um socket de outro nó que não se liga a ele (tem que dar 0; -1 inválido, -2 sem par).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questphase_conexao_controle(res: *mut c_void) -> i32 {
    let Some(graph) = questphase_grafo_de(res) else { return -1 };
    let nos = handles_legiveis_em(graph, GRAPHDEF_NODES_OFF, 4096);
    let Some(a) = nos.first().and_then(|n| handles_legiveis_em(*n, GRAPHNODE_SOCKETS_OFF, 256).into_iter().next()) else { return -2 };
    let conexoes_a = handles_legiveis_em(a, GRAPHSOCKET_CONNECTIONS_OFF, 256);
    for no in nos.iter().skip(1) {
        for b in handles_legiveis_em(*no, GRAPHNODE_SOCKETS_OFF, 256) {
            let ligados = conexoes_a.iter().any(|c| {
                let src = (c.add(GRAPHCONNECTION_SOURCE_OFF) as *const u64).read_unaligned() as *const u8;
                let dst = (c.add(GRAPHCONNECTION_DESTINATION_OFF) as *const u64).read_unaligned() as *const u8;
                src == b || dst == b
            });
            if !ligados {
                return conexoes_a.iter().filter(|c| conexao_indice(b, **c).is_some()).count() as i32;
            }
        }
    }
    -2
}

// Fase de quest, parte 35c-2c (2026-09-12): criar 1 conexão nova no grafo de uma fase carregada que não executa (epilogue). Com a AL-2c, o objeto
// `graphGraphConnectionDefinition` (0x50 B) sai do `CreateInstance`, pelo allocator do motor; `source`/`destination` (`@0x30`/`@0x40`) recebem os dois
// sockets pelo tipo real da propriedade no RTTI (Handle ou WeakHandle), e a conexão entra nos `connections` dos dois só se já houver capacidade (sem
// reserve). Desfazer tira dos dois arrays e solta o handle da native: o destrutor do motor solta as referências dos campos, e o hook da destruição
// genérica diz se o objeto foi destruído. O par de sockets é de nós diferentes e sem conexão entre eles.
#[cfg(feature = "devtools")]
pub(crate) struct ConexaoCriada {
    pub(crate) obj: usize,
    pub(crate) dono: [u64; 2],
    pub(crate) socket_a: usize,
    pub(crate) socket_b: usize,
}

#[cfg(feature = "devtools")]
pub(crate) static CONEXAO_CRIADA: std::sync::Mutex<Option<ConexaoCriada>> = std::sync::Mutex::new(None);

/// ERTTI da propriedade `nome` da classe (9 Handle, 10 WeakHandle); `u32::MAX` se não achar.
pub(crate) unsafe fn ertti_da_propriedade(cls: *mut c_void, nome: &str) -> u32 {
    let prop = rtti::find_property_in_class(cls, nome);
    if !rtti::sane(prop) {
        return u32::MAX;
    }
    let ty = rd_u64(prop as *const c_void, 0) as *mut c_void;
    if !rtti::sane(ty) {
        return u32::MAX;
    }
    let vt = rd_u64(ty as *const c_void, 0) as *const u8;
    if vt.is_null() {
        return u32::MAX;
    }
    let gt = (vt.add(0x28) as *const u64).read_unaligned();
    if gt == 0 {
        return u32::MAX;
    }
    let f: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(gt as usize);
    f(ty)
}

/// Mover de `DynArray<THandle<graph::GraphConnectionDefinition>>` (`MoveAfterReallocation`) e o GOT dele (bind do símbolo pelo `dyld_info -fixups`).
/// No boot 61 os `connections` cozidos do epilogue vieram todos com `tam == cap` (a 2c-1 saiu com -3): a conexão nova pede o reserve do motor.
pub(crate) const GRAPHCONNECTION_MOVER_GOT_VM: u64 = 0x1_077a_7400;

pub(crate) const GRAPHCONNECTION_MOVER_VM: u64 = 0x1_0494_e990;

/// Garante 1 posição livre nos `connections` do socket pelo reserve do motor (`tam + 1`, mover conferido no GOT); `false` se não crescer.
pub(crate) unsafe fn conexoes_folga(socket: *const u8) -> bool {
    handles_folga(socket, GRAPHSOCKET_CONNECTIONS_OFF, GRAPHCONNECTION_MOVER_GOT_VM, GRAPHCONNECTION_MOVER_VM)
}

/// Cria e liga 1 conexão. 1 = ok; -1 recurso inválido; -3 sem par de sockets; -4 mover no GOT ou reserve sem folga; -5 tipo dos campos inesperado;
/// -6 classe/objeto; -8 já há uma criada.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questphase_conexao_criar(res: *mut c_void) -> i32 {
    let Ok(mut estado) = CONEXAO_CRIADA.lock() else { return -9 };
    if estado.is_some() {
        return -8;
    }
    let Some(graph) = questphase_grafo_de(res) else { return -1 };
    let Some(reg) = rtti::Registry::obtain() else { return -6 };
    let cls = reg.class_by_name("graphGraphConnectionDefinition");
    if cls.is_null() {
        return -6;
    }
    let (es, ed) = (ertti_da_propriedade(cls, "source"), ertti_da_propriedade(cls, "destination"));
    if es != ed || !(es == 9 || es == 10) {
        crate::veredito_dev!("[a35-conexao] criar recusado: source/destination com ERTTI {es}/{ed}");
        return -5;
    }
    let nos = handles_legiveis_em(graph, GRAPHDEF_NODES_OFF, 4096);
    let mut par: Option<(*const u8, *const u8)> = None;
    'busca: for (ia, na) in nos.iter().enumerate() {
        for a in handles_legiveis_em(*na, GRAPHNODE_SOCKETS_OFF, 256) {
            let conexoes_a = handles_legiveis_em(a, GRAPHSOCKET_CONNECTIONS_OFF, 256);
            for nb in nos.iter().skip(ia + 1) {
                for b in handles_legiveis_em(*nb, GRAPHNODE_SOCKETS_OFF, 256) {
                    let ligados = conexoes_a.iter().any(|c| {
                        let src = (c.add(GRAPHCONNECTION_SOURCE_OFF) as *const u64).read_unaligned() as *const u8;
                        let dst = (c.add(GRAPHCONNECTION_DESTINATION_OFF) as *const u64).read_unaligned() as *const u8;
                        src == b || dst == b
                    });
                    if !ligados {
                        par = Some((a, b));
                        break 'busca;
                    }
                }
            }
        }
    }
    let Some((a, b)) = par else { return -3 };
    let (_, cap_a, tam_a) = handles_arr_em(a, GRAPHSOCKET_CONNECTIONS_OFF);
    let (_, cap_b, tam_b) = handles_arr_em(b, GRAPHSOCKET_CONNECTIONS_OFF);
    if !(conexoes_folga(a) && conexoes_folga(b)) {
        crate::veredito_dev!(
            "[a35-conexao] criar recusado: sem folga nos connections dos sockets {a:p} ({tam_a}/{cap_a}) e {b:p} ({tam_b}/{cap_b}) depois do reserve"
        );
        return -4;
    }
    let cap_a_nova = handles_arr_em(a, GRAPHSOCKET_CONNECTIONS_OFF).1;
    let cap_b_nova = handles_arr_em(b, GRAPHSOCKET_CONNECTIONS_OFF).1;
    crate::veredito_dev!(
        "[a35-conexao] criar: connections {tam_a}/{cap_a} → cap {cap_a_nova} e {tam_b}/{cap_b} → cap {cap_b_nova} (reserve do motor só onde tam == cap)"
    );
    let obj = rtti::new_object(&reg, "graphGraphConnectionDefinition");
    if obj.is_null() {
        return -6;
    }
    let mut dono = [0u64; 2];
    rtti::make_handle(dono.as_mut_ptr() as *mut c_void, obj);
    let o = obj as *mut u8;
    if es == 9 {
        rtti::make_handle(o.add(GRAPHCONNECTION_SOURCE_OFF) as *mut c_void, a as *mut c_void);
        rtti::make_handle(o.add(GRAPHCONNECTION_DESTINATION_OFF) as *mut c_void, b as *mut c_void);
    } else {
        rtti::make_weak_handle(o.add(GRAPHCONNECTION_SOURCE_OFF) as *mut c_void, a as *mut c_void);
        rtti::make_weak_handle(o.add(GRAPHCONNECTION_DESTINATION_OFF) as *mut c_void, b as *mut c_void);
    }
    let (_, _, tam_a) = handles_arr_em(a, GRAPHSOCKET_CONNECTIONS_OFF);
    let (_, _, tam_b) = handles_arr_em(b, GRAPHSOCKET_CONNECTIONS_OFF);
    handle_strong_soma(&dono, 2);
    let ok_a = handles_insere(a, GRAPHSOCKET_CONNECTIONS_OFF, tam_a as usize, dono);
    let ok_b = handles_insere(b, GRAPHSOCKET_CONNECTIONS_OFF, tam_b as usize, dono);
    crate::veredito_dev!(
        "[a35-conexao] criada {obj:p} (campos {}) entre o socket {a:p} e o socket {b:p}; nos arrays {ok_a}/{ok_b}",
        if es == 9 { "Handle" } else { "WeakHandle" }
    );
    if !(ok_a && ok_b) {
        if ok_a {
            let _ = conexao_indice(a, o).map(|i| handles_tira(a, GRAPHSOCKET_CONNECTIONS_OFF, i));
        }
        if ok_b {
            let _ = conexao_indice(b, o).map(|i| handles_tira(b, GRAPHSOCKET_CONNECTIONS_OFF, i));
        }
        handle_strong_soma(&dono, -2);
        let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono);
        return -3;
    }
    *estado = Some(ConexaoCriada { obj: obj as usize, dono, socket_a: a as usize, socket_b: b as usize });
    1
}

/// Desfaz a conexão criada: sai dos dois arrays e o handle da native é solto. 1 = ok; -1 nada criado; -2 não achada num dos arrays.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questphase_conexao_desfazer_criada() -> i32 {
    let Ok(mut estado) = CONEXAO_CRIADA.lock() else { return -9 };
    let Some(mut t) = estado.take() else { return -1 };
    let obj = t.obj as *const u8;
    let mut r = 1;
    for s in [t.socket_a, t.socket_b] {
        match conexao_indice(s as *const u8, obj) {
            Some(i) => {
                let h = handles_tira(s as *const u8, GRAPHSOCKET_CONNECTIONS_OFF, i);
                handle_strong_soma(&h, -1);
            }
            None => r = -2,
        }
    }
    let destruida = crate::jogo::objetos::soltar_handle_contando(&mut t.dono);
    crate::veredito_dev!("[a35-conexao] criada desfeita ({r}); objeto destruído pelo motor={destruida}");
    r
}

pub(crate) const GRAPHSOCKET_MOVER_GOT_VM: u64 = 0x1_077a_73f8;

pub(crate) const GRAPHSOCKET_MOVER_VM: u64 = 0x1_0286_c008;

pub(crate) const SOCKET_SET_OWNER_VM: u64 = 0x1_0494_e068;

#[cfg(feature = "devtools")]
pub(crate) const NO_TESTE_SOCKET: &str = "bwms_teste_out";

#[cfg(feature = "devtools")]
pub(crate) struct NoCriado {
    pub(crate) no: usize,
    pub(crate) dono_no: [u64; 2],
    pub(crate) dono_socket: [u64; 2],
    pub(crate) graph: usize,
}

#[cfg(feature = "devtools")]
pub(crate) static NO_CRIADO: std::sync::Mutex<Option<NoCriado>> = std::sync::Mutex::new(None);

/// Cria o nó de teste com 1 socket e o liga ao grafo. 1 = ok; -1 recurso inválido; -4 mover no GOT ou reserve sem folga; -5 campo do nó ou do socket;
/// -6 classe/objeto; -8 já há um criado; -9 trava.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questphase_no_criar(res: *mut c_void) -> i32 {
    let Ok(mut estado) = NO_CRIADO.lock() else { return -9 };
    if estado.is_some() {
        return -8;
    }
    let Some(graph) = questphase_grafo_de(res) else { return -1 };
    let Some(reg) = rtti::Registry::obtain() else { return -6 };
    let movers = [(GRAPHNODE_MOVER_GOT_VM, GRAPHNODE_MOVER_VM), (GRAPHSOCKET_MOVER_GOT_VM, GRAPHSOCKET_MOVER_VM)];
    if !movers.iter().all(|(g, m)| (crate::rebase(*g) as *const u64).read_unaligned() == crate::rebase(*m) as u64) {
        return -4;
    }
    let Some(saida) = rtti::resolve_enum_value(&reg, "questSocketType", "Output") else { return -5 };
    let nos = handles_legiveis_em(graph, GRAPHDEF_NODES_OFF, 4096);
    let Some(id) = nos.iter().map(|n| (n.add(QUESTNODE_ID_OFF) as *const u16).read_unaligned()).max().unwrap_or(0).checked_add(1) else { return -5 };
    if !handles_folga(graph, GRAPHDEF_NODES_OFF, GRAPHNODE_MOVER_GOT_VM, GRAPHNODE_MOVER_VM) {
        return -4;
    }
    let no = rtti::new_object(&reg, "questInputNodeDefinition") as *mut u8;
    let socket = rtti::new_object(&reg, "questSocketDefinition") as *mut u8;
    if no.is_null() || socket.is_null() {
        return -6;
    }
    let mut dono_no = [0u64; 2];
    let mut dono_socket = [0u64; 2];
    rtti::make_handle(dono_no.as_mut_ptr() as *mut c_void, no as *mut c_void);
    rtti::make_handle(dono_socket.as_mut_ptr() as *mut c_void, socket as *mut c_void);
    let campos = campo_inteiro_grava(no, "id", id as u64)
        && (no.add(QUESTNODE_ID_OFF) as *const u16).read_unaligned() == id
        && campo_inteiro_grava(socket, "name", cname(NO_TESTE_SOCKET))
        && campo_inteiro_grava(socket, "type", saida);
    if !campos || !handles_folga(no, GRAPHNODE_SOCKETS_OFF, GRAPHSOCKET_MOVER_GOT_VM, GRAPHSOCKET_MOVER_VM) {
        let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono_socket);
        let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono_no);
        return -5;
    }
    let set_owner: unsafe extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(crate::rebase(SOCKET_SET_OWNER_VM));
    set_owner(socket as *mut c_void, no as *mut c_void);
    handle_strong_soma(&dono_socket, 1);
    let ok_s = handles_insere(no, GRAPHNODE_SOCKETS_OFF, 0, dono_socket);
    let (_, _, tam_g) = handles_arr_em(graph, GRAPHDEF_NODES_OFF);
    handle_strong_soma(&dono_no, 1);
    let ok_n = handles_insere(graph, GRAPHDEF_NODES_OFF, tam_g as usize, dono_no);
    let dono_certo = (socket.add(GRAPHSOCKET_DONO_OFF) as *const u64).read_unaligned() == no as u64;
    crate::veredito_dev!(
        "[a35-no] criado nó {no:p} (id {id}) com o socket {socket:p} ({NO_TESTE_SOCKET}, Output): no sockets {ok_s}, no nodes {ok_n}, dono do socket certo={dono_certo}"
    );
    if !(ok_s && ok_n) {
        if ok_n {
            if let Some(i) = handle_indice_em(graph, GRAPHDEF_NODES_OFF, no, 4096) {
                let h = handles_tira(graph, GRAPHDEF_NODES_OFF, i);
                handle_strong_soma(&h, -1);
            }
        } else {
            handle_strong_soma(&dono_no, -1);
        }
        if ok_s {
            let h = handles_tira(no, GRAPHNODE_SOCKETS_OFF, 0);
            handle_strong_soma(&h, -1);
        } else {
            handle_strong_soma(&dono_socket, -1);
        }
        let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono_no);
        let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono_socket);
        return -4;
    }
    *estado = Some(NoCriado { no: no as usize, dono_no, dono_socket, graph: graph as usize });
    1
}

/// Desfaz o nó criado: sai do `nodes` e os handles da native são soltos, nó primeiro. 1 = ok; -1 nada criado; -2 não achado no `nodes`.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questphase_no_desfazer() -> i32 {
    let Ok(mut estado) = NO_CRIADO.lock() else { return -9 };
    let Some(mut t) = estado.take() else { return -1 };
    let graph = t.graph as *const u8;
    let r = match handle_indice_em(graph, GRAPHDEF_NODES_OFF, t.no as *const u8, 4096) {
        Some(i) => {
            let h = handles_tira(graph, GRAPHDEF_NODES_OFF, i);
            handle_strong_soma(&h, -1);
            1
        }
        None => -2,
    };
    let no_destruido = crate::jogo::objetos::soltar_handle_contando(&mut t.dono_no);
    let socket_destruido = crate::jogo::objetos::soltar_handle_contando(&mut t.dono_socket);
    crate::veredito_dev!("[a35-no] nó criado desfeito ({r}); destruídos pelo motor: nó={no_destruido} socket={socket_destruido}");
    r
}

// Fase de quest, parte 35c-2d-2 (2026-09-12): o patch de fase em memória, numa fase carregada que não executa (epilogue), desfeito na mesma
// execução. `FindConnectionPoint` pelo caminho de ids (vazio = 1º `questStartNodeDefinition`, desce por `phaseGraph`); `CreatePhaseNode` com id
// `quest_phase_node_id("<fase do mod>:<id do nó de entrada>")` e `phaseResource` = hash do path; `ResolveSocket` acha por tipo e nome ou cria (dono pelo
// set-owner vanilla); `AddConnection` com os dois `WeakHandle` e a conexão nos dois `connections`; com `intercept`, `RemoveConnection` entre o nó de saída e
// o de entrada, guardando os 16 B tirados. Objetos pelo `CreateInstance` (AL-2c), arrays pelo reserve do motor com o mover conferido no GOT. Os `WeakHandle`
// não deixam strong a mais: o `make_weak_handle` passa por um strong temporário, devolvido aqui. Desfazer: conexões criadas saem, as tiradas voltam nas
// mesmas posições, sockets criados em nós que já existiam saem, o nó sai do `nodes` e os donos da native são soltos; o strong dos sockets que já existiam
// tem que voltar ao valor de antes.
#[cfg(feature = "devtools")]
pub(crate) const PATCH_TESTE_FASE: &str = "mod\\bwms\\teste\\fase_35_2d.questphase";

#[cfg(feature = "devtools")]
pub(crate) const PATCH_TESTE_ENTRADA: [u16; 1] = [17];

#[cfg(feature = "devtools")]
pub(crate) const PATCH_TESTE_SAIDA: [u16; 1] = [9];

pub(crate) const QUESTPHASENODE_VPTR_VM: u64 = 0x1_079e_fdb0;

pub(crate) const QUESTSOCKET_VPTR_VM: u64 = 0x1_07a2_fb90;

pub(crate) const GRAPHCONNECTION_VPTR_VM: u64 = 0x1_07c8_5a98;

pub(crate) const QUESTSOCKET_NOME_OFF: usize = 0x30;

pub(crate) const QUESTSOCKET_TIPO_OFF: usize = 0x58;

pub(crate) const QUESTPHASENODE_GRAFO_OFF: usize = 0x60;

pub(crate) const QUESTPHASENODE_RECURSO_OFF: usize = 0x88;

#[cfg(feature = "devtools")]
pub(crate) const PATCH_CARGA_TESTE_FASE: &str = "mod\\bwms\\teste\\fase_35_2d3.questphase";

pub(crate) struct PatchCtx {
    pub(crate) cls_no: usize,
    pub(crate) cls_socket: usize,
    pub(crate) cls_conexao: usize,
    pub(crate) tipo_in: u64,
    pub(crate) tipo_out: u64,
    pub(crate) tipo_largura: usize,
}

pub(crate) unsafe fn inteiro_le(b: *const u8, off: usize, largura: usize) -> u64 {
    match largura {
        1 => b.add(off).read() as u64,
        2 => (b.add(off) as *const u16).read_unaligned() as u64,
        4 => (b.add(off) as *const u32).read_unaligned() as u64,
        _ => (b.add(off) as *const u64).read_unaligned(),
    }
}

pub(crate) unsafe fn inteiro_grava(b: *mut u8, off: usize, largura: usize, v: u64) {
    match largura {
        1 => b.add(off).write(v as u8),
        2 => (b.add(off) as *mut u16).write_unaligned(v as u16),
        4 => (b.add(off) as *mut u32).write_unaligned(v as u32),
        _ => (b.add(off) as *mut u64).write_unaligned(v),
    }
}

/// Objeto novo pelo `CreateInstance` da classe (AL-2c, conferida no contexto), com dono e vptr conferido; o dono é solto se o vptr não bater.
pub(crate) unsafe fn patch_objeto_novo(cls: usize, vptr_vm: u64) -> Result<(*mut u8, [u64; 2]), i32> {
    let obj = rtti::new_object_from_class(cls as *mut c_void) as *mut u8;
    if obj.is_null() {
        return Err(-6);
    }
    let mut dono = [0u64; 2];
    rtti::make_handle(dono.as_mut_ptr() as *mut c_void, obj as *mut c_void);
    if !vptr_e(obj, vptr_vm) {
        let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono);
        return Err(-14);
    }
    Ok((obj, dono))
}

#[cfg(feature = "devtools")]
pub(crate) static PATCH_APLICADO: std::sync::Mutex<Option<PatchAplicado>> = std::sync::Mutex::new(None);

/// vptr de instância de `questStartNodeDefinition` (registro `0x1028d0b80`, construct `0x1028d0ad0` grava `0x1079fbd80` depois do da base).
pub(crate) const QUESTSTARTNODE_VPTR_VM: u64 = 0x1_079f_bd80;

/// `WeakHandle` pra `obj` em `out`; o `make_weak_handle` já devolve o strong temporário. `false` se o bloco não tinha outro dono
/// (strong fica).
pub(crate) unsafe fn weak_sem_strong(out: *mut u8, obj: *const u8) -> bool {
    rtti::make_weak_handle(out as *mut c_void, obj as *mut c_void)
}

/// Conexão nova do socket `nome_out` (saída) de `no_out` ao socket `nome_in` (entrada) de `no_in`; o feito fica em `p`.
pub(crate) unsafe fn fase_conexao_adiciona(ctx: &PatchCtx, no_out: *const u8, nome_out: &str, no_in: *const u8, nome_in: &str, p: &mut PatchAplicado) -> Result<(), i32> {
    let saida = fase_socket_resolve(ctx, no_out, ctx.tipo_out, nome_out, p)?;
    let entrada = fase_socket_resolve(ctx, no_in, ctx.tipo_in, nome_in, p)?;
    if !conexoes_folga(saida) || !conexoes_folga(entrada) {
        return Err(-4);
    }
    let (obj, mut dono) = patch_objeto_novo(ctx.cls_conexao, GRAPHCONNECTION_VPTR_VM)?;
    for (off, socket) in [(GRAPHCONNECTION_SOURCE_OFF, saida), (GRAPHCONNECTION_DESTINATION_OFF, entrada)] {
        if !weak_sem_strong(obj.add(off), socket) {
            p.weak_com_strong += 1;
        }
    }
    // Uma referência forte por array que guarda a conexão.
    handle_strong_soma(&dono, 2);
    let mut postos: Vec<*const u8> = Vec::with_capacity(2);
    for s in [saida, entrada] {
        let (_, _, tam) = handles_arr_em(s, GRAPHSOCKET_CONNECTIONS_OFF);
        if !handles_insere(s, GRAPHSOCKET_CONNECTIONS_OFF, tam as usize, dono) {
            break;
        }
        postos.push(s);
    }
    if postos.len() < 2 {
        for s in postos {
            if let Some(i) = conexao_indice(s, obj) {
                handles_tira(s, GRAPHSOCKET_CONNECTIONS_OFF, i);
            }
        }
        handle_strong_soma(&dono, -2);
        crate::jogo::objetos::soltar_handle(&mut dono);
        return Err(-4);
    }
    p.conexoes.push((saida as usize, entrada as usize, obj as usize, dono));
    Ok(())
}

/// Conexões de `conexoes` de cada socket de quest do tipo `tipo` de `no` cuja ponta em `ponta_off` é socket com dono `outro`, tiradas do fim para o
/// começo e guardadas em `p.tiradas`.
unsafe fn fase_tirar_ligacoes(ctx: &PatchCtx, no: *const u8, tipo: u64, ponta_off: usize, outro: *const u8, p: &mut PatchAplicado) -> usize {
    let mut tiradas = 0;
    for s in handles_legiveis_em(no, GRAPHNODE_SOCKETS_OFF, 256) {
        if !vptr_e(s, QUESTSOCKET_VPTR_VM) || inteiro_le(s, QUESTSOCKET_TIPO_OFF, ctx.tipo_largura) != tipo {
            continue;
        }
        let (ptr, cap, tam) = handles_arr_em(s, GRAPHSOCKET_CONNECTIONS_OFF);
        if ptr.is_null() || tam > cap || tam > 256 || !crate::memoria::is_readable(ptr as *const c_void, tam as usize * 16) {
            continue;
        }
        for i in (0..tam as usize).rev() {
            let c = ptr.add(i * 2).read_unaligned() as *const u8;
            if !rtti::sane(c as *mut c_void) || !crate::memoria::is_readable(c as *const c_void, 0x50) {
                continue;
            }
            let ponta = (c.add(ponta_off) as *const u64).read_unaligned() as *const u8;
            if rtti::sane(ponta as *mut c_void)
                && crate::memoria::is_readable(ponta as *const c_void, 0x60)
                && (ponta.add(GRAPHSOCKET_DONO_OFF) as *const u64).read_unaligned() == outro as u64
            {
                let h = handles_tira(s, GRAPHSOCKET_CONNECTIONS_OFF, i);
                p.tiradas.push((s as usize, i, h));
                tiradas += 1;
            }
        }
    }
    tiradas
}

/// Tira as conexões entre `no_out` e `no_in`; o tirado fica em `p.tiradas`. Devolve quantas.
pub(crate) unsafe fn fase_conexao_remove(ctx: &PatchCtx, no_out: *const u8, no_in: *const u8, p: &mut PatchAplicado) -> usize {
    fase_tirar_ligacoes(ctx, no_in, ctx.tipo_out, GRAPHCONNECTION_DESTINATION_OFF, no_out, p)
        + fase_tirar_ligacoes(ctx, no_out, ctx.tipo_in, GRAPHCONNECTION_SOURCE_OFF, no_in, p)
}

/// Conexões dos sockets `Output` do nó `a` com destino em socket do nó `b`, presentes nos dois arrays.
pub(crate) unsafe fn fase_ligacoes(ctx: &PatchCtx, a: *const u8, b: *const u8) -> usize {
    let mut n = 0;
    for s in handles_legiveis_em(a, GRAPHNODE_SOCKETS_OFF, 256) {
        if !vptr_e(s, QUESTSOCKET_VPTR_VM) || inteiro_le(s, QUESTSOCKET_TIPO_OFF, ctx.tipo_largura) != ctx.tipo_out {
            continue;
        }
        for c in handles_legiveis_em(s, GRAPHSOCKET_CONNECTIONS_OFF, 256) {
            let src = (c.add(GRAPHCONNECTION_SOURCE_OFF) as *const u64).read_unaligned() as *const u8;
            let dst = (c.add(GRAPHCONNECTION_DESTINATION_OFF) as *const u64).read_unaligned() as *const u8;
            if src == s
                && rtti::sane(dst as *mut c_void)
                && crate::memoria::is_readable(dst as *const c_void, 0x60)
                && (dst.add(GRAPHSOCKET_DONO_OFF) as *const u64).read_unaligned() == b as u64
                && conexao_indice(dst, c).is_some()
            {
                n += 1;
            }
        }
    }
    n
}

/// Escrita do patch de fase no grafo `g_in`, depois das checagens; o que foi feito fica em `p` (também no erro, pra desfazer).
pub(crate) unsafe fn fase_patch_passos(
    ctx: &PatchCtx,
    fase: &str,
    id: u16,
    g_in: *const u8,
    entrada: (*const u8, &str),
    saida: Option<(*const u8, Option<&str>)>,
    intercept: bool,
    p: &mut PatchAplicado,
) -> Result<(), i32> {
    if !handles_folga(g_in, GRAPHDEF_NODES_OFF, GRAPHNODE_MOVER_GOT_VM, GRAPHNODE_MOVER_VM) {
        return Err(-4);
    }
    let (no, dono) = patch_objeto_novo(ctx.cls_no, QUESTPHASENODE_VPTR_VM)?;
    p.no = no as usize;
    p.dono_no = dono;
    (no.add(QUESTNODE_ID_OFF) as *mut u16).write_unaligned(id);
    (no.add(QUESTPHASENODE_RECURSO_OFF) as *mut u64).write_unaligned(bwms_primitives::resource_path_hash(fase));
    // O `nodes` do grafo guarda uma referência forte própria.
    handle_strong_soma(&dono, 1);
    let (_, _, tam) = handles_arr_em(g_in, GRAPHDEF_NODES_OFF);
    if !handles_insere(g_in, GRAPHDEF_NODES_OFF, tam as usize, dono) {
        handle_strong_soma(&dono, -1);
        return Err(-4);
    }
    if let Some((no_out, socket)) = saida {
        if intercept {
            fase_conexao_remove(ctx, no_out, entrada.0, p);
        }
        let socket_in = socket.unwrap_or(if vptr_e(no_out, QUESTPHASENODE_VPTR_VM) { "In1" } else { "In" });
        fase_conexao_adiciona(ctx, no, "Out", no_out, socket_in, p)?;
        fase_conexao_adiciona(ctx, no, "Out1", no_out, socket_in, p)?;
    }
    fase_conexao_adiciona(ctx, entrada.0, entrada.1, no, "In1", p)
}

/// Desfaz um patch, também parcial. `(conexões fora, tiradas de volta, nó fora do nodes, destruídos pelo motor)`.
pub(crate) unsafe fn fase_patch_desfaz(mut p: PatchAplicado) -> (usize, usize, bool, usize) {
    let (mut fora, mut de_volta, mut destruidos) = (0, 0, 0);
    for (out, inn, obj, mut dono) in p.conexoes.drain(..).rev() {
        for s in [out, inn] {
            if let Some(i) = conexao_indice(s as *const u8, obj as *const u8) {
                let h = handles_tira(s as *const u8, GRAPHSOCKET_CONNECTIONS_OFF, i);
                handle_strong_soma(&h, -1);
            }
        }
        fora += 1;
        destruidos += crate::jogo::objetos::soltar_handle_contando(&mut dono) as usize;
    }
    for (s, i, h) in p.tiradas.drain(..).rev() {
        de_volta += handles_insere(s as *const u8, GRAPHSOCKET_CONNECTIONS_OFF, i, h) as usize;
    }
    for (no, s, _) in &p.sockets {
        if *no != p.no {
            if let Some(i) = handle_indice_em(*no as *const u8, GRAPHNODE_SOCKETS_OFF, *s as *const u8, 256) {
                let h = handles_tira(*no as *const u8, GRAPHNODE_SOCKETS_OFF, i);
                handle_strong_soma(&h, -1);
            }
        }
    }
    let mut no_fora = false;
    if p.no != 0 {
        if let Some(i) = handle_indice_em(p.graph as *const u8, GRAPHDEF_NODES_OFF, p.no as *const u8, 65536) {
            let h = handles_tira(p.graph as *const u8, GRAPHDEF_NODES_OFF, i);
            handle_strong_soma(&h, -1);
            no_fora = true;
        }
        destruidos += crate::jogo::objetos::soltar_handle_contando(&mut p.dono_no) as usize;
    }
    for (_, _, mut dono) in p.sockets.drain(..) {
        destruidos += crate::jogo::objetos::soltar_handle_contando(&mut dono) as usize;
    }
    (fora, de_volta, no_fora, destruidos)
}

/// Desfaz um patch e confere o strong dos sockets que já existiam: 1 ok, -2 nó não achado no `nodes`, -13 strong diferente do de antes.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn patch_desfazer_conferido(p: PatchAplicado, origem: &str) -> i32 {
    let tocados = p.tocados.clone();
    let (fora, de_volta, no_fora, destruidos) = fase_patch_desfaz(p);
    let depois: Vec<u32> = tocados.iter().map(|(b, _)| (*b as *const u32).read_unaligned()).collect();
    let strong_igual = tocados.iter().zip(&depois).all(|((_, antes), agora)| antes == agora);
    let antes: Vec<u32> = tocados.iter().map(|(_, s)| *s).collect();
    crate::veredito_dev!(
        "[a35-patch] desfeito ({origem}): {fora} conexões fora, {de_volta} tiradas de volta, nó fora {no_fora}, destruídos pelo motor {destruidos}; strong dos sockets que já existiam {antes:?} → {depois:?}"
    );
    if !no_fora {
        -2
    } else if !strong_igual {
        -13
    } else {
        1
    }
}

/// Desfaz o patch de teste da native. 1 = ok; -1 nada aplicado; -2 nó não achado no `nodes`; -13 strong diferente do de antes.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questphase_patch_desfazer() -> i32 {
    let Ok(mut estado) = PATCH_APLICADO.lock() else { return -9 };
    let Some(p) = estado.take() else { return -1 };
    patch_desfazer_conferido(p, "teste")
}

/// Patch de teste pela native (game thread): resolve o contexto e guarda o aplicado em `PATCH_APLICADO`.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questphase_patch_teste(res: *mut c_void, entrada: &[u16], intercept: bool) -> i32 {
    if !patch_ctx_iniciar() {
        return -7;
    }
    let Ok(mut estado) = PATCH_APLICADO.lock() else { return -9 };
    match questphase_patch_aplicar(res, PATCH_TESTE_FASE, (entrada, None), Some((&PATCH_TESTE_SAIDA[..], None)), intercept, estado.is_some()) {
        Ok(p) => {
            *estado = Some(p);
            1
        }
        Err(e) => e,
    }
}

/// Consulta e desfazer dos patches da carga: 12 aplicados, 13 desfaz todos (1 ok, senão o código do que falhou), 14 cargas de fase vistas pelo hook,
/// 15 código da última tentativa, 16 o nó de fase do patch de teste da tabela está no grafo de `res` com id e `phaseResource` certos.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questphase_patch_carga_acao(res: *mut c_void, acao: i32) -> i32 {
    match acao {
        12 => PATCHES_NA_CARGA.lock().map_or(-9, |v| v.len() as i32),
        13 => {
            let Ok(mut v) = PATCHES_NA_CARGA.lock() else { return -9 };
            let mut r = 1;
            for p in v.drain(..).rev() {
                let c = patch_desfazer_conferido(p, "carga");
                if c != 1 {
                    r = c;
                }
            }
            r
        }
        #[cfg(feature = "devtools")] // a contagem vem do hook de teste da carga de fase, só no dev; no público o 14 cai no `_ => 0`
        14 => crate::selftest::questphase_cargas_vistas() as i32,
        15 => PATCH_CARGA_ULTIMO.load(Ordering::Relaxed),
        16 => {
            let Some(graph) = questphase_grafo_de(res) else { return -1 };
            let id = bwms_primitives::quest_phase_node_id(format!("{PATCH_CARGA_TESTE_FASE}:{}", PATCH_TESTE_ENTRADA[0]).as_bytes());
            let hash = bwms_primitives::resource_path_hash(PATCH_CARGA_TESTE_FASE);
            handles_legiveis_em(graph, GRAPHDEF_NODES_OFF, 65536).iter().any(|n| {
                vptr_e(*n, QUESTPHASENODE_VPTR_VM)
                    && (n.add(QUESTNODE_ID_OFF) as *const u16).read_unaligned() == id
                    && (n.add(QUESTPHASENODE_RECURSO_OFF) as *const u64).read_unaligned() == hash
            }) as i32
        }
        _ => 0,
    }
}



pub(crate) unsafe fn register_spawner_census_natives(reg: &Registry, proto: *mut c_void) {
    let ok1 = registro_de_prova!(register_global(reg, proto, "BwmsSpawnerCensusCount", "BwmsSpawnerCensusCount", corpo_de_prova!(tramp_spawner_census_count)));
    let ok2 = {
        let p = compose_params_from_types(reg, &["Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsSpawnerCensusId", "BwmsSpawnerCensusId", corpo_de_prova!(tramp_spawner_census_id)))
    };
    let reg1 = |nome: &str, tipos: &[&str], tramp: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, i64)| {
        let p = compose_params_from_types(reg, tipos);
        params_compostos(p) && register_global_composed(reg, proto, p, nome, nome, tramp)
    };
    let ok3 = reg1("BwmsSpawnerIsActive", &["Uint64"], tramp_spawner_is_active);
    let ok4 = reg1("BwmsSpawnerIsInitialized", &["Uint64"], tramp_spawner_is_initialized);
    let ok5 = reg1("BwmsSpawnerRecordId", &["Uint64", "Bool"], tramp_spawner_record_id);
    let ok6 = reg1("BwmsSpawnerAppearanceName", &["Uint64"], tramp_spawner_appearance_name);
    let ok7 = reg1("BwmsSpawnerIdsCount", &["Uint64", "Int32"], tramp_spawner_ids_count);
    let ok8 = registro_de_prova!(reg1("BwmsQuestPhaseNodeCount", &["handle:IScriptable"], corpo_de_prova!(tramp_questphase_node_count)));
    let ok_grafo = registro_de_prova!(reg1("BwmsQuestPhaseGraphStat", &["handle:IScriptable", "Int32", "Int32"], corpo_de_prova!(tramp_questphase_graph_stat)));
    let ok_conexao = registro_de_prova!(reg1("BwmsQuestPhaseConnectionEdit", &["handle:IScriptable", "Int32"], corpo_de_prova!(tramp_questphase_connection_edit)));
    crate::log_dev!("[reg] register_all: fase-35 35c-2a GraphStat={ok_grafo} 35c-2b ConnectionEdit={ok_conexao}");
    let ok9 = reg1("BwmsSpawnerEntityHash", &["Uint64", "Int32", "Int32"], tramp_spawner_entity_hash);
    let ok10 = reg1("BwmsSpawnerPosition", &["Uint64"], tramp_spawner_position);
    let ok11 = reg1("BwmsSpawnerOrientation", &["Uint64"], tramp_spawner_orientation);
    let ok12 = registro_de_prova!(reg1("BwmsSpawnerSelfId", &["Uint64"], corpo_de_prova!(tramp_spawner_self_id)));
    let ok13 = reg1("BwmsSpawnerActiveCount", &["Uint64"], tramp_spawner_active_count);
    let ok14 = reg1("BwmsSpawnerActiveEntityHash", &["Uint64", "Int32"], tramp_spawner_active_entity_hash);
    let ok15 = registro_de_prova!(reg1("BwmsSpawnerActiveCountMotor", &["Uint64"], corpo_de_prova!(tramp_spawner_active_count_motor)));
    crate::log_dev!(
        "[reg] register_all: spawner-80d wrapper EntityHash={ok9} Position={ok10} Orientation={ok11} SelfId={ok12} ActiveCount={ok13} ActiveEntityHash={ok14} ActiveCountMotor={ok15}"
    );
    crate::log_dev!(
        "[reg] register_all: spawner-80 censo de spawners Count={ok1} Id={ok2} | leitura IsActive={ok3} IsInitialized={ok4} RecordId={ok5} AppearanceName={ok6} IdsCount={ok7} | fase-35 QuestPhaseNodeCount={ok8}"
    );
}




/// `CommunitySystem::Update(sys, true)` do motor: aplica na hora os pedidos que os slots de comunidade enfileiraram. `false` sem o sistema pela vtable
/// medida ou com o prólogo diferente.
pub(crate) unsafe fn community_update(sys: *mut c_void) -> bool {
    let f = crate::rebase(COMMUNITY_UPDATE_VM);
    if !rtti::sane(sys) || rd_u64(sys, 0) != crate::rebase(COMMUNITYSYSTEM_VTBL_VM) as u64 || (f as *const u32).read_unaligned() != COMMUNITY_UPDATE_PROLOGO {
        return false;
    }
    let update: extern "C" fn(*mut c_void, bool) = std::mem::transmute(f);
    update(sys, true);
    true
}

/// `CommunitySystem` da sessão, pela vtable medida; nulo fora do gameplay.
pub(crate) unsafe fn community_da_sessao() -> *mut c_void {
    sistema_da_sessao("gameCommunitySystem", Some(COMMUNITYSYSTEM_VTBL_VM))
}








/// `BwmsCommunityUpdate(sys: ref<CommunitySystem>) -> Bool`
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_community_update(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let sys = a.first().map_or(std::ptr::null_mut(), |(v, _)| *v as *mut c_void);
    write_bool_ret(out, community_update(sys));
}

// ===== world-depot Fase 2: `DynamicEntitySystem` — FECHADO 2026-08-06 cont.23 ================
//
// **HISTÓRICO (corrigido):** cont.18-20 (2026-08-06, mesmo dia) testaram isto ao vivo e CROU 2/2
// vezes, mesma assinatura de `inkWidget` (`EXC_BREAKPOINT`/`brk #1`) — hipótese da época (ERRADA,
// nunca confirmada): "`EntityID` struct-by-value como param novo aciona o assert genérico de
// bind". cont.23 (mesmo dia, mais tarde) achou a causa real, igual ao `inkWidget`: o `.reds`
// tinha uma declaração `module` no mesmo escopo da declaração native — mesmo bug de qualificação
// de `fullName` já documentado. **Fix (remover `module`) testado ao vivo: `DynamicEntitySystem`
// completo (8 métodos, incl. os 4 com `EntityID` — RULED OUT como gatilho por este mesmo teste)
// forjado+registrado+chamado via `newobj`/`callon`, zero crash, GAMEPLAY t=86s em 2 boots.**
// `ponte-dynamicentitysystem.reds` está deployado hoje SEM `module` (fix presente). O código
// abaixo (estruturas + trampolins) é chamado de produção, incondicional, via `inicio/boot.rs::
// class_validate_probe_hook` (`register_dynamicentitysystem`, mais abaixo) — NÃO é código morto.
//
// `public native class DynamicEntitySystem extends IGameSystem`, 21 métodos. `world-depot` já tinha fechado (IMPL,
// 2026-07-25) usando `CompanionSystem.SpawnSubcharacterOnPosition` como substituto — a auditoria
// de 2026-08-05 apontou que isso é "bucket largo, prova estreita" (só prova spawn pontual, não a
// API real de tracking-por-tag/listener que mods reais usam). `register_dynamicentitysystem` (mais
// abaixo) registra as duas partes: o bookkeeping por tag, que é dado nosso no registry Rust-side
// (`AssignTag`/`UnassignTag`/`IsTagged`/`IsPopulated`/`DeleteTagged` e os getters Count+ByIndex), e
// o ciclo de entidade pelo motor (`CreateEntity`/`DeleteEntity`/`Enable*`/`Disable*`, os 3 arrays
// `GetTags`/`GetTagged`/`GetTaggedIDs` e os ouvintes, `#73b`/`#73c`); `IsReady`/`IsRestored`
// consultam os sistemas da sessão.
// `EntityID` é confirmado `{ hash: u64 }` — 8 bytes, mesma
// categoria de "struct escalar" já tratada dezenas de vezes neste projeto (TweakDBID/ItemID/CName).
pub(crate) type DynentMap = std::sync::Mutex<std::collections::HashMap<u64, std::collections::HashSet<u64>>>;

pub(crate) static DYNENT_TAGS_CELL: std::sync::OnceLock<DynentMap> = std::sync::OnceLock::new();

pub(crate) static DYNENT_BY_TAG_CELL: std::sync::OnceLock<DynentMap> = std::sync::OnceLock::new();

pub(crate) fn dynent_tags() -> &'static DynentMap {
    DYNENT_TAGS_CELL.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub(crate) fn dynent_by_tag() -> &'static DynentMap {
    DYNENT_BY_TAG_CELL.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// `IsReady`: os sistemas do motor que o `CreateEntity` usa resolvem na sessão.
/// Sem parâmetro, mas consome o frame: sem pular o `ParamEnd` a native usada inline (`des.IsReady() ? a : b`) devolve lixo (boot 233).
pub(crate) unsafe extern "C" fn tramp_des_is_ready(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    rtti::read_params_consuming(current_native_func(), frame);
    let v = dynent_sistemas().is_some();
    crate::log_dev!("[dynent] IsReady out={out:p} -> {v}");
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_des_is_restored(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    rtti::read_params_consuming(current_native_func(), frame);
    write_bool_ret(out, dynent_restaurado());
}

pub(crate) unsafe extern "C" fn tramp_des_is_managed(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let v = dynent_gerenciada(id) || dynent_tags().lock().map(|m| m.contains_key(&id)).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_des_is_tagged(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let tag = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let v = dynent_tags().lock().map(|m| m.get(&id).is_some_and(|s| s.contains(&tag))).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_des_is_populated(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    let v = dynent_by_tag().lock().map(|m| m.get(&tag).is_some_and(|s| !s.is_empty())).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_des_assign_tag(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let tag = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let motor = tag_do_motor(gametagsystem_da_sessao(), true, id, tag);
    crate::log_dev!("[dynent] AssignTag no GameTagSystem: {motor}");
    if let Ok(mut m) = dynent_tags().lock() {
        m.entry(id).or_default().insert(tag);
    }
    if let Ok(mut m) = dynent_by_tag().lock() {
        m.entry(tag).or_default().insert(id);
    }
    crate::log_dev!("[dynent] AssignTag id={id:#x} tag={tag:#x} ({})", crate::cname::resolve_cname(tag));
}

pub(crate) unsafe extern "C" fn tramp_des_unassign_tag(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let tag = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let motor = tag_do_motor(gametagsystem_da_sessao(), false, id, tag);
    crate::log_dev!("[dynent] UnassignTag no GameTagSystem: {motor}");
    if let Ok(mut m) = dynent_tags().lock() {
        if let Some(s) = m.get_mut(&id) {
            s.remove(&tag);
        }
    }
    if let Ok(mut m) = dynent_by_tag().lock() {
        if let Some(s) = m.get_mut(&tag) {
            s.remove(&id);
        }
    }
    crate::log_dev!("[dynent] UnassignTag id={id:#x} tag={tag:#x}");
}

pub(crate) unsafe extern "C" fn tramp_des_delete_tagged(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    // As entidades do `CreateEntity` com a tag são apagadas; as só marcadas pelo `AssignTag` perdem a tag.
    let (apagar, marcadas): (Vec<u64>, Vec<u64>) = dynent_ids_com_tag(tag).into_iter().partition(|id| dynent_gerenciada(*id));
    for id in &apagar {
        dynent_apagar(*id);
    }
    if let (Ok(mut porid), Ok(mut portag)) = (dynent_tags().lock(), dynent_by_tag().lock()) {
        for id in &marcadas {
            if let Some(s) = porid.get_mut(id) {
                s.remove(&tag);
            }
        }
        if let Some(s) = portag.get_mut(&tag) {
            for id in &marcadas {
                s.remove(id);
            }
        }
    }
    crate::log_dev!("[dynent] DeleteTagged tag={tag:#x} -> {} apagada(s), {} desassociada(s)", apagar.len(), marcadas.len());
}

pub(crate) unsafe extern "C" fn tramp_des_get_tagged_id(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    let id = dynent_by_tag().lock().map(|m| conjunto_menor(&m, tag)).unwrap_or(0);
    write_uint_ret(out, id, 8);
}

pub(crate) unsafe extern "C" fn tramp_des_get_tagged_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    let n = dynent_by_tag().lock().map(|m| conjunto_tamanho(&m, tag)).unwrap_or(0);
    write_uint_ret(out, n as u32 as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_des_get_tagged_id_by_index(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    let idx = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let id = dynent_by_tag().lock().map(|m| conjunto_no_indice(&m, tag, idx)).unwrap_or(0);
    write_uint_ret(out, id, 8);
}

pub(crate) unsafe extern "C" fn tramp_des_get_tag_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let n = dynent_tags().lock().map(|m| conjunto_tamanho(&m, id)).unwrap_or(0);
    write_uint_ret(out, n as u32 as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_des_get_tag_by_index(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let idx = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let tag = dynent_tags().lock().map(|m| conjunto_no_indice(&m, id, idx)).unwrap_or(0);
    write_uint_ret(out, tag, 8);
}

// ===== Entidade dinâmica transiente pelo motor (2026-09-18) =====================================================
// RE offline em `proofs/20260918-*-73b-re-offline.log`. Só id transiente: o persistente iria pro save, e o BWMS não grava dado de mod no save.
pub(crate) const STUBSYS_VTBL_VM: u64 = 0x1_0787_1808;

pub(crate) const STUB_CREATE_VM: u64 = 0x1_0174_835c;

pub(crate) const STUB_DELETE_VM: u64 = 0x1_0174_88d4;

pub(crate) const POPSYS_VTBL_VM: u64 = 0x1_07c6_e3d0;

pub(crate) const POP_FIND_VM: u64 = 0x1_047d_7870;

pub(crate) const POP_IS_SPAWNING_VM: u64 = 0x1_047d_7cb4;

pub(crate) const POP_REGISTER_VM: u64 = 0x1_047e_695c;

pub(crate) const POP_REMOVE_VM: u64 = 0x1_047e_6ea8;

/// `EntityStubCreateRequest` (0x40 B).
#[repr(C, align(16))]
pub(crate) struct PedidoStub {
    pub(crate) id: u64,
    pub(crate) _08: u64,
    pub(crate) posicao: [i32; 3],
    pub(crate) _1c: u32,
    pub(crate) orientacao: [f32; 4],
    pub(crate) record: u64,
    pub(crate) _38: u64,
}

/// `PopulationEntityRegisterRequest` (0x28 B); `+0x11`, `+0x15` e o `WeakHandle` em `+0x18` ficam zerados.
#[repr(C)]
pub(crate) struct PedidoRegistro {
    pub(crate) id: u64,
    pub(crate) aparencia: u64,
    pub(crate) prioridade: u8,
    pub(crate) _11: u8,
    pub(crate) sempre: u8,
    pub(crate) na_vista: u8,
    pub(crate) prefetch: u8,
    pub(crate) _15: [u8; 3],
    pub(crate) fraco: [u64; 2],
}

/// Callback do motor: buffer de 0x80 B (guarda o id) e a tabela `{invoke, copy, move, destruct}` em `+0x80`.
#[repr(C, align(16))]
pub(crate) struct CallbackMotor {
    pub(crate) buffer: [u8; 0x80],
    pub(crate) tabela: *const TabelaCallback,
    pub(crate) _88: u64,
}

pub(crate) static DYNENT_CALLBACK: TabelaCallback = TabelaCallback {
    invoke: dynent_cb_invoke,
    copiar: dynent_cb_copiar,
    mover: dynent_cb_mover,
    destruir: dynent_cb_destruir,
};

pub(crate) static DYNENT_ESTADOS: std::sync::Mutex<std::collections::BTreeMap<u64, EntidadeDinamica>> = std::sync::Mutex::new(std::collections::BTreeMap::new());

pub(crate) static DYNENT_PEDIDOS: AtomicU64 = AtomicU64::new(0);

/// Trilha dos passos do motor por id (pedido, criado com status, registrado, removido, eventos), pro teste conferir a ordem. Limitada a 128.
pub(crate) static DYNENT_TRILHA: std::sync::Mutex<Vec<(u64, &'static str, i64)>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn dynent_trilhar(id: u64, passo: &'static str, valor: i64) {
    crate::log_dev!("[dynent] id={id} {passo} {valor}");
    if let Ok(mut t) = DYNENT_TRILHA.lock() {
        if t.len() < 128 {
            t.push((id, passo, valor));
        }
    }
}

#[cfg(feature = "devtools")]
pub(crate) fn dynent_trilha(id: u64) -> Vec<(&'static str, i64)> {
    DYNENT_TRILHA.lock().map(|t| t.iter().filter(|x| x.0 == id).map(|x| (x.1, x.2)).collect()).unwrap_or_default()
}

pub(crate) unsafe extern "C" fn dynent_cb_copiar(dst: *mut u8, src: *const u8) {
    std::ptr::copy_nonoverlapping(src, dst, 0x80);
}

pub(crate) unsafe extern "C" fn dynent_cb_mover(dst: *mut u8, src: *mut u8) {
    std::ptr::copy_nonoverlapping(src, dst, 0x80);
}

pub(crate) unsafe extern "C" fn dynent_cb_destruir(_buf: *mut u8) {}

/// Invoke do callback do `CreateStub` (buffer: id `@0`, geração do pedido `@8`; token em `token_sp`).
pub(crate) unsafe extern "C" fn dynent_cb_invoke(buffer: *mut u8, token_sp: *mut [u64; 2]) {
    if buffer.is_null() || token_sp.is_null() {
        return;
    }
    let token = (*token_sp)[0] as *mut u8;
    if token.is_null() {
        return;
    }
    let (id, geracao) = ((buffer as *const u64).read_unaligned(), (buffer.add(8) as *const u64).read_unaligned());
    // A trava segura `DeleteEntity`/`DisableEntity` até o fim do registro.
    let Ok(mut estados) = DYNENT_ESTADOS.lock() else { return };
    let status = token.add(0x10).read();
    dynent_trilhar(id, "criado", i64::from(status));
    let Some(e) = estados.get_mut(&id).filter(|e| e.pedido == geracao) else { return };
    e.status = i32::from(status);
    let stub = (token as *const u64).read_unaligned();
    if status != 1 || stub == 0 || (token.add(8) as *const u64).read_unaligned() != id {
        return;
    }
    // O stub passa a ser nosso: o motor apaga o que ficar no token depois do invoke.
    (token as *mut u64).write_unaligned(0);
    e.holder = stub;
    let pedido = PedidoRegistro {
        id,
        aparencia: e.aparencia,
        prioridade: 0,
        _11: 0,
        sempre: e.sempre as u8,
        na_vista: e.na_vista as u8,
        prefetch: (e.aparencia != 0) as u8,
        _15: [0; 3],
        fraco: [0; 2],
    };
    let registrar: unsafe extern "C" fn(usize, *const PedidoRegistro) -> u64 = std::mem::transmute(crate::rebase(POP_REGISTER_VM));
    e.registrada = registrar(e.pop, &pedido) & 0xff != 0;
    dynent_trilhar(id, "registrada", i64::from(e.registrada));
}

/// Solta a referência do BWMS a um `SharedPtr` de token de spawn (`[objeto, bloco de contagem]`).
pub(crate) unsafe fn dynent_soltar_token(sp: [u64; 2]) {
    if sp[1] == 0 {
        return;
    }
    // A última referência fica com o motor, que destrói o token pelo caminho dele.
    let forte = &*(sp[1] as *const std::sync::atomic::AtomicU32);
    let _ = forte.fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| (v > 1).then(|| v - 1));
}

/// Os 3 sistemas do motor da sessão: (stubs, população, ids).
pub(crate) unsafe fn dynent_sistemas() -> Option<(*mut c_void, *mut c_void, *mut c_void)> {
    let (reg, game) = (crate::registry()?, crate::jogo::sessao::game_instance_ptr()?);
    let sistema = |tipo: &str, vtbl: u64| {
        let s = crate::jogo::sessao::sistema_do_motor_por_nome(reg, game, tipo, |s| crate::jogo::sessao::sistema_com_vtable(s, Some(vtbl)));
        (!s.is_null()).then_some(s)
    };
    Some((sistema("gameEntityStubSystem", STUBSYS_VTBL_VM)?, sistema("gamePopulationSystem", POPSYS_VTBL_VM)?, sistema("gameDynamicEntityIDSystem", IDSYS_VTBL_VM)?))
}

/// Handle forte da entidade de `id` no `PopulationSystem` `pop`; `[0, 0]` sem entidade.
pub(crate) unsafe fn dynent_find_entity(pop: *mut c_void, id: u64) -> [u64; 2] {
    let mut h = [0u64; 2];
    if pop.is_null() {
        return h;
    }
    crate::jogo::chamada::sret_chamar(crate::rebase(POP_FIND_VM), [pop as u64, id & 0xffff_ffff, 0, 0], h.as_mut_ptr() as *mut c_void);
    h
}

/// Pedido de stub pro estado `e` (já marcado como pedido); o `CreateStub` roda fora da trava nossa.
pub(crate) struct PedidoDeSpawn {
    pub(crate) stubs: usize,
    pub(crate) pedido: PedidoStub,
    pub(crate) geracao: u64,
}

/// Marca `e` com um pedido novo e devolve o que o `CreateStub` precisa; chamar com a trava dos estados.
pub(crate) fn dynent_marcar_spawn(id: u64, e: &mut EntidadeDinamica) -> PedidoDeSpawn {
    let geracao = DYNENT_PEDIDOS.fetch_add(1, Ordering::AcqRel) + 1;
    e.pedido = geracao;
    e.status = 0;
    let orientacao = if e.orientacao.iter().all(|v| *v == 0.0) { [0.0, 0.0, 0.0, 1.0] } else { e.orientacao };
    PedidoDeSpawn {
        stubs: e.stubs,
        pedido: PedidoStub {
            id,
            _08: 0,
            posicao: [0, 1, 2].map(|k| (e.posicao[k] * FIXEDPOINT_ESCALA) as i32),
            _1c: 0,
            orientacao,
            record: e.record,
            _38: 0,
        },
        geracao,
    }
}

/// Pede o stub ao motor (`STUB_CREATE_VM`) com o callback [`DYNENT_CALLBACK`].
pub(crate) unsafe fn dynent_create_stub(p: PedidoDeSpawn) {
    if p.stubs == 0 {
        return;
    }
    let mut cb = CallbackMotor { buffer: [0; 0x80], tabela: &DYNENT_CALLBACK, _88: 0 };
    cb.buffer[..8].copy_from_slice(&p.pedido.id.to_le_bytes());
    cb.buffer[8..16].copy_from_slice(&p.geracao.to_le_bytes());
    let mut sp = [0u64; 2];
    crate::jogo::chamada::sret_chamar(
        crate::rebase(STUB_CREATE_VM),
        [p.stubs as u64, &p.pedido as *const PedidoStub as u64, &mut cb as *mut CallbackMotor as u64, 0],
        sp.as_mut_ptr() as *mut c_void,
    );
    dynent_trilhar(p.pedido.id, "pedido", sp[0] as i64);
    dynent_soltar_token(sp);
}

/// Tira a entidade do mundo mantendo o estado; chamar com a trava dos estados. Devolve se havia stub ou pedido.
pub(crate) unsafe fn dynent_despawnar(id: u64, e: &mut EntidadeDinamica, mesma_sessao: bool) -> bool {
    let havia = e.holder != 0 || e.pedido != 0;
    if e.holder != 0 && mesma_sessao {
        // `RemoveEntity` e `DeleteStub` desreferenciam o stub sem checar nulo: com ele em mãos, `RemoveEntity` primeiro.
        let remover: unsafe extern "C" fn(usize, u32, u64) = std::mem::transmute(crate::rebase(POP_REMOVE_VM));
        remover(e.pop, id as u32, 0);
        let apagar_stub: unsafe extern "C" fn(usize, *mut u64) = std::mem::transmute(crate::rebase(STUB_DELETE_VM));
        apagar_stub(e.stubs, &mut e.holder);
        dynent_trilhar(id, "removida", 1);
    }
    e.holder = 0;
    e.pedido = 0;
    e.status = -1;
    e.registrada = false;
    havia
}

/// Os sistemas da sessão atual são os do estado `e`.
unsafe fn dynent_mesma_sessao(atuais: Option<(*mut c_void, *mut c_void, *mut c_void)>, e: &EntidadeDinamica) -> bool {
    atuais.is_some_and(|(stubs, pop, _)| stubs as usize == e.stubs && pop as usize == e.pop)
}

/// `CreateEntity` a partir do spec; devolve o id ou 0.
pub(crate) unsafe fn dynent_criar(spec: &DynamicEntitySpecFields) -> u64 {
    if spec.record_id == 0 {
        crate::log_dev!("[dynent] CreateEntity sem record: recusado");
        return 0;
    }
    let Some((stubs, pop, ids)) = dynent_sistemas() else { return 0 };
    dynent_ouvir_motor();
    let proximo: unsafe extern "C" fn(*mut c_void) -> u64 = std::mem::transmute(crate::rebase(ID_NEXT_TRANSIENT_VM));
    let id = proximo(ids) & 0xffff_ffff;
    if id == 0 {
        return 0;
    }
    let mut e = EntidadeDinamica {
        stubs: stubs as usize,
        pop: pop as usize,
        holder: 0,
        status: -1,
        pedido: 0,
        registrada: false,
        ativa: spec.active,
        record: spec.record_id,
        posicao: spec.position,
        orientacao: spec.orientation,
        aparencia: spec.appearance_name,
        sempre: spec.always_spawned,
        na_vista: spec.spawn_in_view,
    };
    let pedido = spec.active.then(|| dynent_marcar_spawn(id, &mut e));
    match DYNENT_ESTADOS.lock() {
        Ok(mut m) => {
            m.insert(id, e);
        }
        Err(_) => return 0,
    }
    if let Ok(mut s) = DYNENT_IDS.lock() {
        s.insert(id);
    }
    if let (Ok(mut porid), Ok(mut portag)) = (dynent_tags().lock(), dynent_by_tag().lock()) {
        for &tag in &spec.tags {
            porid.entry(id).or_default().insert(tag);
            portag.entry(tag).or_default().insert(id);
        }
    }
    dynent_enfileirar(EVENTO_CRIADA, id, spec.tags.clone());
    if let Some(p) = pedido {
        dynent_create_stub(p);
    }
    id
}

/// Entidade transiente por record, sem aparência, tags nem persistência (o teste em `aparencia_do_record`).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn dynent_criar_por_record(record: u64, posicao: [f32; 4]) -> u64 {
    dynent_criar(&DynamicEntitySpecFields {
        record_id: record,
        template_path: 0,
        template_hash: 0,
        appearance_name: 0,
        position: posicao,
        orientation: [0.0, 0.0, 0.0, 1.0],
        persist_state: false,
        persist_spawn: false,
        always_spawned: true,
        spawn_in_view: true,
        tags: Vec::new(),
        active: true,
    })
}

/// Recria uma entidade a partir do spec que o BWMS guardou fora do save.
pub(crate) unsafe fn dynent_criar_do_spec(s: &crate::salvamento::entidades_persistentes::Spec) -> u64 {
    dynent_criar(&DynamicEntitySpecFields {
        record_id: s.record,
        template_path: s.template,
        template_hash: 0,
        appearance_name: s.aparencia,
        position: s.posicao,
        orientation: s.orientacao,
        persist_state: s.estado,
        persist_spawn: true,
        always_spawned: true,
        spawn_in_view: true,
        tags: s.tags.clone(),
        active: true,
    })
}

/// Handle (forte) da entidade de um id do `CreateEntity`; `[0, 0]` enquanto não spawnou.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn dynent_entidade(id: u64) -> [u64; 2] {
    match dynent_sistemas() {
        Some((_, pop, _)) => dynent_find_entity(pop, id),
        None => [0; 2],
    }
}

#[cfg(feature = "devtools")]
pub(crate) unsafe fn dynent_apagar_pub(id: u64) -> bool {
    dynent_apagar(id)
}

/// `DeleteEntity`.
pub(crate) unsafe fn dynent_apagar(id: u64) -> bool {
    if !dynent_gerenciada(id) {
        return false;
    }
    let atuais = dynent_sistemas();
    let Some(mut e) = DYNENT_ESTADOS.lock().ok().and_then(|mut m| m.remove(&id)) else { return false };
    let mesma = dynent_mesma_sessao(atuais, &e);
    dynent_despawnar(id, &mut e, mesma);
    let mut tags: Vec<u64> = Vec::new();
    if let (Ok(mut porid), Ok(mut portag)) = (dynent_tags().lock(), dynent_by_tag().lock()) {
        tags = porid.remove(&id).map(|s| s.into_iter().collect()).unwrap_or_default();
        for tag in &tags {
            if let Some(s) = portag.get_mut(tag) {
                s.remove(&id);
            }
        }
    }
    // O `Despawned` do motor chega depois do `DeleteEntity` e ainda precisa das tags.
    if let Ok(mut a) = DYNENT_TAGS_APAGADAS.lock() {
        a.insert(id, tags.clone());
    }
    if let Ok(mut s) = DYNENT_MORTOS.lock() {
        s.remove(&id);
    }
    dynent_enfileirar(EVENTO_APAGADA, id, tags);
    true
}

/// `EnableEntity`/`DisableEntity`.
pub(crate) unsafe fn dynent_ligar(id: u64, ligar: bool) -> bool {
    let atuais = dynent_sistemas();
    let pedido = {
        let Ok(mut m) = DYNENT_ESTADOS.lock() else { return false };
        let Some(e) = m.get_mut(&id) else { return false };
        if e.ativa == ligar {
            return false;
        }
        e.ativa = ligar;
        let mesma = dynent_mesma_sessao(atuais, e);
        if !ligar {
            return dynent_despawnar(id, e, mesma);
        }
        if !mesma {
            return false;
        }
        dynent_marcar_spawn(id, e)
    };
    if let Ok(mut s) = DYNENT_MORTOS.lock() {
        s.remove(&id);
    }
    dynent_create_stub(pedido);
    true
}

/// Entrada em sessão nova. As entidades da sessão anterior morreram com o mundo dela,
/// mas os estados e as tags ficavam — e o `GetTaggedIDs` contava fantasma (medido no boot 311: tag=2
/// com 1 entidade viva). Esquece tudo que aponta pra sistemas que não são os do jogo agora.
///
/// # Safety
/// Thread do jogo; lê os sistemas de spawn da sessão corrente.
pub(crate) unsafe fn dynent_esquecer_de_outra_sessao() -> usize {
    // Comparar o ponteiro dos sistemas NÃO serve: eles são os mesmos entre sessões (medido no boot
    // 312). Quem diz se a entidade ainda existe é o `PopulationSystem`: registrada, com stub em mãos e
    // sem ser achada pelo `FindEntity` = morreu com o mundo dela.
    let Some((_, pop, _)) = dynent_sistemas() else { return 0 };
    let candidatos: Vec<(u64, bool)> = match DYNENT_ESTADOS.lock() {
        Ok(m) => m.iter().map(|(id, e)| (*id, e.registrada && e.holder != 0)).collect(),
        Err(_) => return 0,
    };
    let mut mortos = Vec::new();
    for (id, registrada) in candidatos {
        if !registrada {
            continue;
        }
        let mut h = dynent_find_entity(pop, id);
        if h[0] == 0 {
            mortos.push(id);
        } else {
            crate::jogo::objetos::soltar_handle(&mut h);
        }
    }
    let Ok(mut m) = DYNENT_ESTADOS.lock() else { return 0 };
    let antes = m.len();
    for id in &mortos {
        m.remove(id);
    }
    drop(m);
    if !mortos.is_empty() {
        if let Ok(mut tags) = dynent_by_tag().lock() {
            for ids in tags.values_mut() {
                for id in &mortos {
                    ids.remove(id);
                }
            }
            tags.retain(|_, ids| !ids.is_empty());
        }
        // Os dois lados do índice: sem isto o id→tags guardava as tags da entidade morta, e `GetTags`/`IsTagged` de um id da
        // sessão anterior respondiam com fantasma.
        if let Ok(mut por_id) = dynent_tags().lock() {
            for id in &mortos {
                por_id.remove(id);
            }
        }
    }
    antes - DYNENT_ESTADOS.lock().map(|m| m.len()).unwrap_or(0)
}

pub(crate) fn dynent_ids_com_tag(tag: u64) -> Vec<u64> {
    dynent_by_tag().lock().ok().and_then(|m| m.get(&tag).map(|s| s.iter().copied().collect())).unwrap_or_default()
}

pub(crate) fn dynent_gerenciada(id: u64) -> bool {
    DYNENT_ESTADOS.lock().map(|m| m.contains_key(&id)).unwrap_or(false)
}

// ----- Eventos ---------------------------------------------------------------------------------------------------------------------
// `Created`/`Deleted` saem do `CreateEntity`/`DeleteEntity`; `Spawned`/`Despawned`/`Dead` vêm do `EntitySpawnerEventsBroadcaster` do motor
// (valores nativos 2/3/4, os mesmos do `DynamicEntityEventType`). O ouvinte do motor roda num job com a trava de escrita do broadcaster presa:
// só enfileira. O tick da thread do jogo despacha pro script.
pub(crate) const EVENTO_CRIADA: u32 = 0;

pub(crate) const EVENTO_APAGADA: u32 = 1;

pub(crate) const BROADCASTER_VTBL_VM: u64 = 0x1_07c6_8a78;

pub(crate) const BROADCASTER_REGISTER_VM: u64 = 0x1_0474_3f20;

/// Callback do `RegisterListener` do broadcaster: buffer de 0x20 (geração do registro `@0`) e a tabela em `+0x20`.
#[repr(C, align(16))]
pub(crate) struct CallbackOuvinte {
    pub(crate) buffer: [u8; 0x20],
    pub(crate) tabela: *const TabelaOuvinte,
    pub(crate) _28: u64,
}

#[repr(C)]
pub(crate) struct TabelaOuvinte {
    pub(crate) invoke: unsafe extern "C" fn(*mut u8, *const u32, *const u64, *const u64, *const u64),
    pub(crate) copiar: unsafe extern "C" fn(*mut u8, *const u8),
    pub(crate) mover: unsafe extern "C" fn(*mut u8, *mut u8),
    pub(crate) destruir: unsafe extern "C" fn(*mut u8),
}

pub(crate) static DYNENT_OUVINTE: TabelaOuvinte = TabelaOuvinte {
    invoke: dynent_ouvinte_invoke,
    copiar: dynent_ouvinte_copiar,
    mover: dynent_ouvinte_mover,
    destruir: dynent_cb_destruir,
};

/// Ids criados nesta sessão (o filtro do ouvinte do motor, que vê toda a população).
pub(crate) static DYNENT_IDS: std::sync::Mutex<std::collections::BTreeSet<u64>> = std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Tags de quem foi apagado, pro `Despawned` que o motor manda depois do `DeleteEntity`.
pub(crate) static DYNENT_TAGS_APAGADAS: std::sync::Mutex<std::collections::BTreeMap<u64, Vec<u64>>> = std::sync::Mutex::new(std::collections::BTreeMap::new());

/// Fila (tipo, id, tags; vazio = as tags atuais) e o tamanho dela pro tick não travar à toa.
pub(crate) static DYNENT_EVENTOS: std::sync::Mutex<Vec<(u32, u64, Vec<u64>)>> = std::sync::Mutex::new(Vec::new());

pub(crate) static DYNENT_EVENTOS_N: AtomicUsize = AtomicUsize::new(0);

/// Registro no broadcaster: (broadcaster, geração). A geração vale no buffer do callback; um registro de sessão anterior que o motor ainda
/// chame não bate e é ignorado.
pub(crate) static DYNENT_REGISTRO: std::sync::Mutex<(usize, u64)> = std::sync::Mutex::new((0, 0));

pub(crate) static DYNENT_GERACAO_OUVINTE: AtomicU64 = AtomicU64::new(0);

/// Ouvintes de script: (tag, handle forte do alvo, função).
pub(crate) static DYNENT_OUVINTES: std::sync::Mutex<Vec<(u64, [u64; 2], u64)>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn dynent_enfileirar(tipo: u32, id: u64, tags: Vec<u64>) {
    if let Ok(mut f) = DYNENT_EVENTOS.lock() {
        f.push((tipo, id, tags));
        DYNENT_EVENTOS_N.store(f.len(), Ordering::Release);
    }
}

pub(crate) unsafe extern "C" fn dynent_ouvinte_copiar(dst: *mut u8, src: *const u8) {
    std::ptr::copy_nonoverlapping(src, dst, 0x20);
}

pub(crate) unsafe extern "C" fn dynent_ouvinte_mover(dst: *mut u8, src: *mut u8) {
    std::ptr::copy_nonoverlapping(src, dst, 0x20);
}

/// `invoke(buffer, &tipo, &id, &id2, &stub)` do broadcaster, com os argumentos por ponteiro.
pub(crate) unsafe extern "C" fn dynent_ouvinte_invoke(buffer: *mut u8, tipo: *const u32, id: *const u64, _id2: *const u64, _stub: *const u64) {
    if tipo.is_null() || id.is_null() || (buffer as *const u64).read_unaligned() != DYNENT_GERACAO_OUVINTE.load(Ordering::Acquire) {
        return;
    }
    let (tipo, id) = (*tipo, *id);
    if !(2..=4).contains(&tipo) || !DYNENT_IDS.lock().map(|s| s.contains(&id)).unwrap_or(false) {
        return;
    }
    dynent_trilhar(id, "evento-motor", i64::from(tipo));
    dynent_enfileirar(tipo, id, Vec::new());
}

/// Registra o ouvinte do BWMS no broadcaster da sessão, uma vez por sessão (refaz se o broadcaster mudou ou depois do fim de sessão).
pub(crate) unsafe fn dynent_ouvir_motor() {
    let b = sistema_da_sessao("gameEntitySpawnerEventsBroadcasterImpl", Some(BROADCASTER_VTBL_VM));
    let Ok(mut r) = DYNENT_REGISTRO.lock() else {
        return;
    };
    if b.is_null() || r.0 == b as usize {
        return;
    }
    let geracao = DYNENT_GERACAO_OUVINTE.load(Ordering::Acquire) + 1;
    let mut cb = CallbackOuvinte { buffer: [0; 0x20], tabela: &DYNENT_OUVINTE, _28: 0 };
    cb.buffer[..8].copy_from_slice(&geracao.to_le_bytes());
    DYNENT_GERACAO_OUVINTE.store(geracao, Ordering::Release);
    let registrar: extern "C" fn(*mut c_void, *mut CallbackOuvinte, *const u8) -> u32 = std::mem::transmute(crate::rebase(BROADCASTER_REGISTER_VM));
    let lid = registrar(b, &mut cb, c"".as_ptr() as *const u8);
    crate::log_dev!("[dynent] ouvinte no broadcaster {b:p}: id={lid} geração={geracao}");
    if lid != 0 {
        *r = (b as usize, geracao);
    }
}

/// Fim de sessão (o mundo saiu): esquece estados, tags, ouvintes e o registro no broadcaster.
pub(crate) unsafe fn dynent_fim_de_sessao() {
    if let Ok(mut r) = DYNENT_REGISTRO.lock() {
        *r = (0, 0);
    }
    DYNENT_GERACAO_OUVINTE.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut m) = DYNENT_ESTADOS.lock() {
        m.clear();
    }
    for c in [&DYNENT_IDS] {
        if let Ok(mut s) = c.lock() {
            s.clear();
        }
    }
    if let Ok(mut a) = DYNENT_TAGS_APAGADAS.lock() {
        a.clear();
    }
    if let Ok(mut s) = DYNENT_MORTOS.lock() {
        s.clear();
    }
    if let (Ok(mut porid), Ok(mut portag)) = (dynent_tags().lock(), dynent_by_tag().lock()) {
        porid.clear();
        portag.clear();
    }
    let ouvintes: Vec<(u64, [u64; 2], u64)> = DYNENT_OUVINTES.lock().map(|mut o| o.drain(..).collect()).unwrap_or_default();
    for (_, mut h, _) in ouvintes {
        crate::jogo::objetos::soltar_handle(&mut h);
    }
    if let Ok(mut f) = DYNENT_EVENTOS.lock() {
        f.clear();
        DYNENT_EVENTOS_N.store(0, Ordering::Release);
    }
}

/// `Dead`: o motor não emite `Death` no broadcaster neste build (os únicos `QueueEvent` do slot `+0x1c8` são `Spawn` em `0x1047e637c` e
/// `0x104c48fb8` e `Despawn` em `0x1047ef538`, nenhum script vanilla emite `Death`), então um `Dead` preso ao `Death` nunca dispararia. O BWMS vigia:
/// a cada ~1 s, com ouvinte registrado, as entidades no mundo que respondem `IsDead()` verdadeiro viram um `Dead`, uma vez por spawn.
pub(crate) static DYNENT_MORTOS: std::sync::Mutex<std::collections::BTreeSet<u64>> = std::sync::Mutex::new(std::collections::BTreeSet::new());

pub(crate) const EVENTO_MORTA: u32 = 4;

pub(crate) unsafe fn dynent_vigiar_mortes() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    if !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || DYNENT_OUVINTES.lock().map(|o| o.is_empty()).unwrap_or(true) {
        return;
    }
    let vivos: Vec<u64> = DYNENT_ESTADOS.lock().map(|m| m.iter().filter(|(_, e)| e.holder != 0 && e.registrada).map(|(id, _)| *id).collect()).unwrap_or_default();
    let ja: std::collections::BTreeSet<u64> = DYNENT_MORTOS.lock().map(|s| s.clone()).unwrap_or_default();
    let alvos: Vec<u64> = vivos.into_iter().filter(|id| !ja.contains(id)).collect();
    if alvos.is_empty() {
        return;
    }
    let Some((_, pop, _)) = dynent_sistemas() else { return };
    for id in alvos {
        let mut h = dynent_find_entity(pop, id);
        let ent = h[0] as *mut c_void;
        let morto = rtti::sane(ent)
            && rtti::resolve_in_class(rtti::class_of(ent), "IsDead").and_then(|rf| rtti::call_func(&rf, ent, &[])).is_some_and(|b| b[0] & 1 != 0);
        crate::jogo::objetos::soltar_handle(&mut h);
        if morto {
            if let Ok(mut s) = DYNENT_MORTOS.lock() {
                s.insert(id);
            }
            dynent_trilhar(id, "morta", 1);
            dynent_enfileirar(EVENTO_MORTA, id, Vec::new());
        }
    }
}

/// Despacha a fila de eventos pros ouvintes de script (thread do jogo): pra cada tag da entidade com ouvinte, `BwmsDynamicEntityEventDispatch`
/// (script) monta o `DynamicEntityEvent` e chama `alvo.função(evento)`.
pub(crate) unsafe fn dynent_despachar() {
    if DYNENT_EVENTOS_N.load(Ordering::Acquire) == 0 {
        return;
    }
    let eventos: Vec<(u32, u64, Vec<u64>)> = match DYNENT_EVENTOS.lock() {
        Ok(mut f) => {
            DYNENT_EVENTOS_N.store(0, Ordering::Release);
            f.drain(..).collect()
        }
        Err(_) => return,
    };
    let Some(reg) = rtti::Registry::obtain() else { return };
    let despachar = get_function(&reg, "BwmsDynamicEntityEventDispatch");
    if despachar.is_null() {
        return;
    }
    let rf = rtti::ResolvedFn { func: despachar, ret_type: std::ptr::null_mut(), is_static: true };
    for (tipo, id, tags) in eventos {
        let tags = if !tags.is_empty() {
            tags
        } else {
            let atuais: Vec<u64> = dynent_tags().lock().ok().and_then(|m| m.get(&id).map(|s| s.iter().copied().collect())).unwrap_or_default();
            if atuais.is_empty() {
                DYNENT_TAGS_APAGADAS.lock().ok().and_then(|a| a.get(&id).cloned()).unwrap_or_default()
            } else {
                atuais
            }
        };
        for tag in tags {
            let ouvintes: Vec<([u64; 2], u64)> =
                DYNENT_OUVINTES.lock().map(|o| o.iter().filter(|x| x.0 == tag).map(|x| (x.1, x.2)).collect()).unwrap_or_default();
            for (alvo, funcao) in ouvintes {
                let mut h = [0u64; 2];
                rtti::make_handle(h.as_mut_ptr() as *mut c_void, alvo[0] as *mut c_void);
                let args = [
                    rtti::Arg::Handle(h[0] as *mut c_void, h[1] as *mut c_void),
                    rtti::Arg::CName(funcao),
                    rtti::Arg::I32(tipo),
                    rtti::Arg::I64(id),
                    rtti::Arg::CName(tag),
                ];
                let ok = rtti::call_func(&rf, std::ptr::null_mut(), &args).is_some();
                // O `call_func` só copia o par: o +1 do `make_handle` é nosso, e sem soltar o ouvinte não chega a strong 0 depois do Unregister.
                crate::jogo::objetos::soltar_handle(&mut h);
                dynent_trilhar(id, "despachado", i64::from(tipo) * 10 + i64::from(ok));
            }
        }
    }
}

pub(crate) fn dynent_id_do_frame(frame: *mut c_void) -> u64 {
    let args = unsafe { rtti::read_params_consuming(current_native_func(), frame) };
    args.first().map_or(0, |(v, _)| *v)
}

/// `CreateEntity(spec: ref<DynamicEntitySpec>) -> EntityID`
pub(crate) unsafe extern "C" fn tramp_des_create_entity(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let spec_obj = dynent_id_do_frame(frame) as usize;
    let spec = des_spec_map().lock().ok().and_then(|m| m.get(&spec_obj).cloned());
    let id = spec.as_ref().map_or(0, |s| dynent_criar(s));
    // Com `persistSpawn`, o spec vai pro arquivo do save (fora dele) e volta na carga.
    if id != 0 {
        if let Some(s) = spec.as_ref().filter(|s| s.persist_spawn) {
            crate::salvamento::entidades_persistentes::registrar(
                id,
                crate::salvamento::entidades_persistentes::Spec {
                    record: s.record_id,
                    template: s.template_path,
                    aparencia: s.appearance_name,
                    posicao: s.position,
                    orientacao: s.orientation,
                    tags: s.tags.clone(),
                    estado: s.persist_state,
                },
            );
        }
    }
    crate::log_dev!("[dynent] CreateEntity(spec={spec_obj:#x}) -> {id}");
    write_uint_ret(out, id, 8);
}

/// `DeleteEntity(id: EntityID) -> Bool`
pub(crate) unsafe extern "C" fn tramp_des_delete_entity(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    crate::salvamento::entidades_persistentes::esquecer(id);
    write_bool_ret(out, dynent_apagar(id));
}

/// `EnableEntity(id: EntityID) -> Bool`
pub(crate) unsafe extern "C" fn tramp_des_enable_entity(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    write_bool_ret(out, dynent_ligar(id, true));
}

/// `DisableEntity(id: EntityID) -> Bool`
pub(crate) unsafe extern "C" fn tramp_des_disable_entity(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    write_bool_ret(out, dynent_ligar(id, false));
}

/// `IsSpawned(id: EntityID) -> Bool`: o `PopulationSystem` tem a entidade.
pub(crate) unsafe extern "C" fn tramp_des_is_spawned(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    let mut h = dynent_sistemas().map_or([0; 2], |(_, pop, _)| dynent_find_entity(pop, id));
    let v = h[0] != 0;
    crate::jogo::objetos::soltar_handle(&mut h);
    write_bool_ret(out, v);
}

/// `IsSpawning(id: EntityID) -> Bool`
pub(crate) unsafe extern "C" fn tramp_des_is_spawning(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    let v = dynent_sistemas().is_some_and(|(_, pop, _)| {
        let f: extern "C" fn(*mut c_void, u32) -> u32 = std::mem::transmute(crate::rebase(POP_IS_SPAWNING_VM));
        f(pop, id as u32) & 0xff != 0
    });
    write_bool_ret(out, v);
}

/// `GetEntity(id: EntityID) -> ref<Entity>`: o handle do `FindEntity` já vem com +1, que passa pro script.
pub(crate) unsafe extern "C" fn tramp_des_get_entity(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    let mut h = dynent_sistemas().map_or([0; 2], |(_, pop, _)| dynent_find_entity(pop, id));
    if out.is_null() {
        crate::jogo::objetos::soltar_handle(&mut h);
        return;
    }
    entregar_handle(out, h);
}

/// `GetTags(id: EntityID) -> array<CName>`
pub(crate) unsafe extern "C" fn tramp_des_get_tags(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    let tags: Vec<u64> = dynent_tags().lock().ok().and_then(|m| m.get(&id).map(|s| s.iter().copied().collect())).unwrap_or_default();
    array_u64_do_motor_ret(out, &tags);
}

/// `GetTaggedIDs(tag: CName) -> array<EntityID>`
pub(crate) unsafe extern "C" fn tramp_des_get_tagged_ids(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let tag = dynent_id_do_frame(frame);
    array_u64_do_motor_ret(out, &dynent_ids_com_tag(tag));
}

/// `GetTagged(tag: CName) -> array<ref<Entity>>`: só as que estão no mundo, cada handle com o +1 do `FindEntity`.
pub(crate) unsafe extern "C" fn tramp_des_get_tagged(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let tag = dynent_id_do_frame(frame);
    let handles: Vec<[u64; 2]> = match dynent_sistemas() {
        Some((_, pop, _)) => dynent_ids_com_tag(tag).into_iter().map(|id| dynent_find_entity(pop, id)).filter(|h| h[0] != 0).collect(),
        None => Vec::new(),
    };
    if out.is_null() {
        for mut h in handles {
            crate::jogo::objetos::soltar_handle(&mut h);
        }
        return;
    }
    array_do_motor_ret(out, handles.len(), 16, |buf| {
        for (i, h) in handles.iter().enumerate() {
            core::ptr::write_unaligned(buf.add(i * 16) as *mut [u64; 2], *h);
        }
    });
}

/// `EnableTagged(tag)`/`DisableTagged(tag)`
pub(crate) unsafe extern "C" fn tramp_des_enable_tagged(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    for id in dynent_ids_com_tag(dynent_id_do_frame(frame)) {
        dynent_ligar(id, true);
    }
}

pub(crate) unsafe extern "C" fn tramp_des_disable_tagged(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    for id in dynent_ids_com_tag(dynent_id_do_frame(frame)) {
        dynent_ligar(id, false);
    }
}

/// `RegisterListener(tag: CName, target: ref<IScriptable>, function: CName)`: guarda um handle forte do alvo até o `Unregister*` ou o fim da
/// sessão.
pub(crate) unsafe extern "C" fn tramp_des_register_listener(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let tag = args.first().map_or(0, |(v, _)| *v);
    let alvo = args.get(1).map_or(std::ptr::null_mut(), |(v, _)| *v as *mut c_void);
    let funcao = args.get(2).map_or(0, |(v, _)| *v);
    if tag == 0 || funcao == 0 || !rtti::sane(alvo) {
        return;
    }
    dynent_ouvir_motor();
    let mut h = [0u64; 2];
    rtti::make_handle(h.as_mut_ptr() as *mut c_void, alvo);
    if let Ok(mut o) = DYNENT_OUVINTES.lock() {
        o.push((tag, h, funcao));
    }
    crate::log_dev!("[dynent] RegisterListener tag={} alvo={alvo:p} função={}", crate::cname::resolve_cname(tag), crate::cname::resolve_cname(funcao));
}

/// `UnregisterListener(tag, target, function)` e `UnregisterListeners(tag)`.
pub(crate) unsafe fn dynent_desregistrar(tag: u64, alvo: Option<(usize, u64)>) {
    let tirados: Vec<[u64; 2]> = match DYNENT_OUVINTES.lock() {
        Ok(mut o) => {
            let (fora, dentro): (Vec<_>, Vec<_>) =
                o.drain(..).partition(|x| x.0 == tag && alvo.is_none_or(|(a, f)| x.1[0] as usize == a && x.2 == f));
            *o = dentro;
            fora.into_iter().map(|x| x.1).collect()
        }
        Err(_) => Vec::new(),
    };
    for mut h in tirados {
        crate::jogo::objetos::soltar_handle(&mut h);
    }
}

pub(crate) unsafe extern "C" fn tramp_des_unregister_listener(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let tag = args.first().map_or(0, |(v, _)| *v);
    let alvo = args.get(1).map_or(0, |(v, _)| *v as usize);
    let funcao = args.get(2).map_or(0, |(v, _)| *v);
    dynent_desregistrar(tag, Some((alvo, funcao)));
}

pub(crate) unsafe extern "C" fn tramp_des_unregister_listeners(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    dynent_desregistrar(dynent_id_do_frame(frame), None);
}

/// `IsRestored`: o mundo veio de um save. O `ScriptableSystem.WasRestored` nativo (`0x102445fbc`) só lê o byte `+0x529`, gravado quando o
/// sistema é restaurado (`0x102445e80`); aqui, o do `EquipmentSystem` vanilla pelo container da sessão.
pub(crate) const SCRIPTABLE_RESTAURADO_OFF: usize = 0x529;

pub(crate) unsafe fn dynent_restaurado() -> bool {
    // Pelo mapa de sistemas; senão pelo `GameInstance.GetScriptableSystemsContainer(game)` estático (handle com +1, solto no fim).
    let mut do_script = [0u64; 2];
    let mut container = sistema_da_sessao("gameScriptableSystemsContainer", None);
    if container.is_null() {
        let player = crate::current_player();
        if let Some(reg) = rtti::Registry::obtain().filter(|_| !player.is_null()) {
            let gi = crate::jogo::sessao::game_instance(&reg, player);
            let h = gi.and_then(|gi| {
                rtti::resolve_any(&reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetScriptableSystemsContainer")
                    .and_then(|f| rtti::call_func(&f, std::ptr::null_mut(), &[rtti::Arg::Raw(gi)]))
            });
            if let Some(h) = h {
                do_script = [u64::from_le_bytes(h[..8].try_into().unwrap_or([0; 8])), u64::from_le_bytes(h[8..16].try_into().unwrap_or([0; 8]))];
                container = do_script[0] as *mut c_void;
            }
        }
    }
    let r = rtti::sane(container).then(|| rtti::resolve_in_class(rtti::class_of(container), "Get")).flatten();
    let b = r.and_then(|rf| rtti::call_func(&rf, container, &[rtti::Arg::CName(crate::cname::cname("EquipmentSystem"))]));
    crate::jogo::objetos::soltar_handle(&mut do_script);
    let Some(b) = b else {
        return false;
    };
    let mut h = [u64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8])), u64::from_le_bytes(b[8..16].try_into().unwrap_or([0; 8]))];
    let obj = h[0] as *const u8;
    let v = !obj.is_null() && crate::memoria::is_readable(obj.add(SCRIPTABLE_RESTAURADO_OFF) as *const c_void, 1) && *obj.add(SCRIPTABLE_RESTAURADO_OFF) != 0;
    crate::jogo::objetos::soltar_handle(&mut h);
    v
}

pub unsafe fn register_dynamicentitysystem(reg: &Registry) -> String {
    let igs = reg.class_by_name("IGameSystem");
    let igs = if rtti::sane(igs) { igs } else { resolve_class_via_validator_getclass("IGameSystem") };
    if !rtti::sane(igs) {
        return "[reg] register_dynamicentitysystem: 'IGameSystem' não resolveu".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "DynamicEntitySystem", "gameuiInGameMenuGameController", igs);
    if forged.is_null() {
        return "[reg] register_dynamicentitysystem: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_dynamicentitysystem: sem protótipo de método".into(),
    };
    let m1 = register_method(reg, "DynamicEntitySystem", proto, "IsReady", "IsReady", tramp_des_is_ready, false);
    let m2 = register_method(reg, "DynamicEntitySystem", proto, "IsRestored", "IsRestored", tramp_des_is_restored, false);
    let m3 = register_method(reg, "DynamicEntitySystem", proto, "IsManaged", "IsManaged", tramp_des_is_managed, false);
    let m4 = register_method(reg, "DynamicEntitySystem", proto, "IsTagged", "IsTagged", tramp_des_is_tagged, false);
    let m5 = register_method(reg, "DynamicEntitySystem", proto, "IsPopulated", "IsPopulated", tramp_des_is_populated, false);
    let m6 = register_method(reg, "DynamicEntitySystem", proto, "AssignTag", "AssignTag", tramp_des_assign_tag, false);
    let m7 = register_method(reg, "DynamicEntitySystem", proto, "UnassignTag", "UnassignTag", tramp_des_unassign_tag, false);
    let m8 = register_method(reg, "DynamicEntitySystem", proto, "DeleteTagged", "DeleteTagged", tramp_des_delete_tagged, false);
    // Round 8 (2026-08-12): 5 getters escalares novos (`GetTaggedID`/`GetTaggedCount`/
    // `GetTaggedIDByIndex`/`GetTagCount`/`GetTagByIndex`) — mesmo idioma Count()+ByIndex(i) já
    // usado por `ReflectionClass`/`Scripting.GetStackTrace` pra evitar array-retorno.
    let m9 = register_method(reg, "DynamicEntitySystem", proto, "GetTaggedID", "GetTaggedID", tramp_des_get_tagged_id, false);
    let m10 = register_method(reg, "DynamicEntitySystem", proto, "GetTaggedCount", "GetTaggedCount", tramp_des_get_tagged_count, false);
    let m11 = register_method(reg, "DynamicEntitySystem", proto, "GetTaggedIDByIndex", "GetTaggedIDByIndex", tramp_des_get_tagged_id_by_index, false);
    let m12 = register_method(reg, "DynamicEntitySystem", proto, "GetTagCount", "GetTagCount", tramp_des_get_tag_count, false);
    let m13 = register_method(reg, "DynamicEntitySystem", proto, "GetTagByIndex", "GetTagByIndex", tramp_des_get_tag_by_index, false);
    // #73b (2026-09-18): ciclo de entidade transiente pelo motor.
    let m14 = register_method(reg, "DynamicEntitySystem", proto, "CreateEntity", "CreateEntity", tramp_des_create_entity, false);
    let m15 = register_method(reg, "DynamicEntitySystem", proto, "DeleteEntity", "DeleteEntity", tramp_des_delete_entity, false);
    let m16 = register_method(reg, "DynamicEntitySystem", proto, "IsSpawned", "IsSpawned", tramp_des_is_spawned, false);
    let m17 = register_method(reg, "DynamicEntitySystem", proto, "IsSpawning", "IsSpawning", tramp_des_is_spawning, false);
    let m18 = register_method(reg, "DynamicEntitySystem", proto, "GetEntity", "GetEntity", tramp_des_get_entity, false);
    // #73c (2026-09-18): ligar e desligar, operações por tag, os 3 arrays e os ouvintes.
    let m19 = register_method(reg, "DynamicEntitySystem", proto, "EnableEntity", "EnableEntity", tramp_des_enable_entity, false);
    let m20 = register_method(reg, "DynamicEntitySystem", proto, "DisableEntity", "DisableEntity", tramp_des_disable_entity, false);
    let m21 = register_method(reg, "DynamicEntitySystem", proto, "GetTags", "GetTags", tramp_des_get_tags, false);
    let m22 = register_method(reg, "DynamicEntitySystem", proto, "GetTagged", "GetTagged", tramp_des_get_tagged, false);
    let m23 = register_method(reg, "DynamicEntitySystem", proto, "GetTaggedIDs", "GetTaggedIDs", tramp_des_get_tagged_ids, false);
    let m24 = register_method(reg, "DynamicEntitySystem", proto, "EnableTagged", "EnableTagged", tramp_des_enable_tagged, false);
    let m25 = register_method(reg, "DynamicEntitySystem", proto, "DisableTagged", "DisableTagged", tramp_des_disable_tagged, false);
    let m26 = register_method(reg, "DynamicEntitySystem", proto, "RegisterListener", "RegisterListener", tramp_des_register_listener, false);
    let m27 = register_method(reg, "DynamicEntitySystem", proto, "UnregisterListener", "UnregisterListener", tramp_des_unregister_listener, false);
    let m28 = register_method(reg, "DynamicEntitySystem", proto, "UnregisterListeners", "UnregisterListeners", tramp_des_unregister_listeners, false);
    format!(
        "[reg] DynamicEntitySystem: classe=OK (INSTANCIÁVEL, parent=IGameSystem real, mesma receita de CallbackSystem) \
IsReady={m1} IsRestored={m2} IsManaged={m3} IsTagged={m4} IsPopulated={m5} AssignTag={m6} UnassignTag={m7} DeleteTagged={m8} \
GetTaggedID={m9} GetTaggedCount={m10} GetTaggedIDByIndex={m11} GetTagCount={m12} GetTagByIndex={m13} \
CreateEntity={m14} DeleteEntity={m15} IsSpawned={m16} IsSpawning={m17} GetEntity={m18} \
EnableEntity={m19} DisableEntity={m20} GetTags={m21} GetTagged={m22} GetTaggedIDs={m23} EnableTagged={m24} DisableTagged={m25} \
RegisterListener={m26} UnregisterListener={m27} UnregisterListeners={m28}"
    )
}



// ===== `DynamicEntitySpec` — implementação NOVA, 2026-08-19 =========================
//
// Companion struct do `DynamicEntitySystem` (investigação offline de 2026-08-19): o IRMÃO
// `StaticEntitySpec` (FECHADO 2026-08-15,
// `register_staticentityspec` acima) é um TEMPLATE quase mecânico — mesmo parent `IScriptable`
// direto, mesmo registry-Rust-por-ponteiro, mesmo `Vector4`/`Quaternion`-como-parâmetro já resolvido
// (`read_params_consuming_with_raw16`/`write_raw16_ret`), mesmo idioma Count+ByIndex pra tags. Esta
// rodada cobre os 12 campos (StaticEntitySpec tinha só 6):
//
//   TweakDBID                 recordID;       // NOVO (não existe em StaticEntitySpec)
//   ResRef                    templatePath;   // igual StaticEntitySpec
//   uint64_t                  templateHash;   // NOVO — companion de templatePath p/ (de)serialização
//   CName                     appearanceName; // igual
//   Vector4                   position;       // igual
//   Quaternion                orientation;    // igual
//   bool                      persistState;   // NOVO
//   bool                      persistSpawn;   // NOVO
//   bool                      alwaysSpawned;  // NOVO
//   bool                      spawnInView;    // NOVO
//   DynArray<CName>           tags;           // igual
//   bool                      active;         // NOVO
//
// `recordID` (TweakDBID) e `templateHash` (Uint64 cru) são ambos escalares de 8 bytes — mesma
// categoria já provada dezenas de vezes (`tramp_identidade_8_bytes`, `write_uint_ret(...,8)`),
// zero técnica nova. Os 5 `bool` novos (persistState/persistSpawn/alwaysSpawned/spawnInView/active)
// usam o MESMO fix de mascaramento `rtti::bool_do_script` já aplicado em `tramp_sespec_set_attached`
// (rodada 11 do #76 — bug real de leitura de parâmetro Bool sem máscara, documentado ali) — aplicado
// AQUI DESDE A ORIGEM, nunca reproduzindo o bug.
//
// Helpers internos (`IsRecord()`/`IsTemplate()`/`PrepareForSaving()`/`RestoreAfterLoading()`) não
// são expostos a script — fora de escopo. Create+Setters/Getters em vez de campos `native let`
// diretos, a mesma decisão de design do `StaticEntitySpec`.
#[derive(Clone, Default)]
pub(crate) struct DynamicEntitySpecFields {
    pub(crate) record_id: u64,
    pub(crate) template_path: u64,
    pub(crate) template_hash: u64,
    pub(crate) appearance_name: u64,
    pub(crate) position: [f32; 4],
    pub(crate) orientation: [f32; 4],
    pub(crate) persist_state: bool,
    pub(crate) persist_spawn: bool,
    pub(crate) always_spawned: bool,
    pub(crate) spawn_in_view: bool,
    pub(crate) tags: Vec<u64>,
    pub(crate) active: bool,
}

pub(crate) type DesSpecMap = std::sync::Mutex<std::collections::HashMap<usize, DynamicEntitySpecFields>>;

pub(crate) static DES_SPEC_CELL: std::sync::OnceLock<DesSpecMap> = std::sync::OnceLock::new();

pub(crate) fn des_spec_map() -> &'static DesSpecMap {
    DES_SPEC_CELL.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// `DynamicEntitySpec.Create() -> ref<DynamicEntitySpec>` — mesma receita de `tramp_sespec_create`
/// (#76): constrói instância NOVA + retenção forte DESDE A ORIGEM + entry vazia no registry.
pub(crate) unsafe extern "C" fn tramp_despec_create(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut obj: *mut c_void = std::ptr::null_mut();
    if let Some(reg) = rtti::Registry::obtain() {
        obj = rtti::new_object(&reg, "DynamicEntitySpec");
    }
    if !obj.is_null() {
        retain_strong_ref_leaked(obj);
        if let Ok(mut m) = des_spec_map().lock() {
            m.insert(obj as usize, DynamicEntitySpecFields::default());
        }
    } else {
        crate::log_dev!("[despec] DynamicEntitySpec.Create: new_object('DynamicEntitySpec') falhou");
    }
    crate::log_dev!("[despec] DynamicEntitySpec.Create() -> {obj:p}");
    rtti::make_handle(out, obj);
}

pub(crate) unsafe extern "C" fn tramp_despec_set_record_id(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let v = args.first().map(|(v, _)| *v).unwrap_or(0);
    if let Ok(mut m) = des_spec_map().lock() {
        m.entry(c as usize).or_default().record_id = v;
    }
    crate::log_dev!("[despec] SetRecordID({v:#018x}) on {c:p}");
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_record_id(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = des_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.record_id)).unwrap_or(0);
    write_uint_ret(out, v, 8);
}

pub(crate) unsafe extern "C" fn tramp_despec_set_template_path(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let path = args.first().map(|(v, _)| *v).unwrap_or(0);
    if let Ok(mut m) = des_spec_map().lock() {
        m.entry(c as usize).or_default().template_path = path;
    }
    crate::log_dev!("[despec] SetTemplatePath({path:#018x}) on {c:p}");
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_template_path(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = des_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.template_path)).unwrap_or(0);
    write_uint_ret(out, v, 8);
}

pub(crate) unsafe extern "C" fn tramp_despec_set_template_hash(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let v = args.first().map(|(v, _)| *v).unwrap_or(0);
    if let Ok(mut m) = des_spec_map().lock() {
        m.entry(c as usize).or_default().template_hash = v;
    }
    crate::log_dev!("[despec] SetTemplateHash({v:#018x}) on {c:p}");
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_template_hash(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = des_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.template_hash)).unwrap_or(0);
    write_uint_ret(out, v, 8);
}

pub(crate) unsafe extern "C" fn tramp_despec_set_appearance_name(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let name = args.first().map(|(v, _)| *v).unwrap_or(0);
    if let Ok(mut m) = des_spec_map().lock() {
        m.entry(c as usize).or_default().appearance_name = name;
    }
    crate::log_dev!("[despec] SetAppearanceName({name:#018x}) on {c:p}");
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_appearance_name(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = des_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.appearance_name)).unwrap_or(0);
    write_uint_ret(out, v, 8);
}

/// `SetPersistState`/`SetPersistSpawn`/`SetAlwaysSpawned`/`SetSpawnInView`/`SetActive` — os 5 `bool`
/// NOVOS deste item (StaticEntitySpec só tinha `attached`). Máscara `rtti::bool_do_script` aplicada DESDE A
/// ORIGEM (fix da rodada 11 do #76, nunca reproduzindo o bug de lixo nos bytes 1-7 do slot de 8
/// bytes lido por `read_params_consuming` pra um tipo `Bool`/1-byte real).
pub(crate) unsafe extern "C" fn tramp_despec_set_persist_state(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let v = args.first().map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    if let Ok(mut m) = des_spec_map().lock() {
        m.entry(c as usize).or_default().persist_state = v;
    }
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_persist_state(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = des_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.persist_state)).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_despec_set_persist_spawn(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let v = args.first().map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    if let Ok(mut m) = des_spec_map().lock() {
        m.entry(c as usize).or_default().persist_spawn = v;
    }
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_persist_spawn(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = des_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.persist_spawn)).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_despec_set_always_spawned(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let v = args.first().map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    if let Ok(mut m) = des_spec_map().lock() {
        m.entry(c as usize).or_default().always_spawned = v;
    }
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_always_spawned(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = des_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.always_spawned)).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_despec_set_spawn_in_view(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let v = args.first().map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    if let Ok(mut m) = des_spec_map().lock() {
        m.entry(c as usize).or_default().spawn_in_view = v;
    }
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_spawn_in_view(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = des_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.spawn_in_view)).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_despec_set_active(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let v = args.first().map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    if let Ok(mut m) = des_spec_map().lock() {
        m.entry(c as usize).or_default().active = v;
    }
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_active(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = des_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.active)).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_despec_add_tag(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    if let Ok(mut m) = des_spec_map().lock() {
        m.entry(c as usize).or_default().tags.push(tag);
    }
    crate::log_dev!("[despec] AddTag({tag:#018x}) on {c:p}");
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_tag_count(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let n = des_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.tags.len())).unwrap_or(0);
    write_uint_ret(out, n as u32 as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_tag_by_index(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let idx = args.first().map(|(v, _)| *v as i32).unwrap_or(-1);
    let v = if idx < 0 {
        0
    } else {
        des_spec_map()
            .lock()
            .ok()
            .and_then(|m| m.get(&(c as usize)).and_then(|f| f.tags.get(idx as usize).copied()))
            .unwrap_or(0)
    };
    write_uint_ret(out, v, 8);
}

/// `SetPosition(v: Vector4) -> Bool` / `SetOrientation(q: Quaternion) -> Bool` — mesma técnica
/// struct-de-16-bytes-como-PARÂMETRO já provada em `tramp_sespec_set_position`/`set_orientation`
/// (#76, rodada 10): `read_params_consuming_with_raw16` entrega os 16 bytes crus de `pinst`.
pub(crate) unsafe extern "C" fn tramp_despec_set_position(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, raw16) = rtti::read_params_consuming_with_raw16(func, frame);
    let bytes = raw16.first().and_then(|r| *r);
    let ok = bytes.is_some();
    if let Some(b) = bytes {
        let mut v = [0f32; 4];
        for i in 0..4 {
            v[i] = f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]));
        }
        if let Ok(mut m) = des_spec_map().lock() {
            m.entry(c as usize).or_default().position = v;
        }
        crate::log_dev!("[despec] SetPosition({v:?}) on {c:p}");
    } else {
        crate::log_dev!("[despec] SetPosition: raw16 vazio (tipo do param não bateu Vector4?) on {c:p}");
    }
    write_bool_ret(out, ok);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_position(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = des_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.position)).unwrap_or([0f32; 4]);
    let mut b = [0u8; 16];
    for i in 0..4 {
        b[i * 4..i * 4 + 4].copy_from_slice(&v[i].to_le_bytes());
    }
    write_raw16_ret(out, &b);
}

pub(crate) unsafe extern "C" fn tramp_despec_set_orientation(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, raw16) = rtti::read_params_consuming_with_raw16(func, frame);
    let bytes = raw16.first().and_then(|r| *r);
    let ok = bytes.is_some();
    if let Some(b) = bytes {
        let mut v = [0f32; 4];
        for i in 0..4 {
            v[i] = f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]));
        }
        if let Ok(mut m) = des_spec_map().lock() {
            m.entry(c as usize).or_default().orientation = v;
        }
        crate::log_dev!("[despec] SetOrientation({v:?}) on {c:p}");
    } else {
        crate::log_dev!("[despec] SetOrientation: raw16 vazio (tipo do param não bateu Quaternion?) on {c:p}");
    }
    write_bool_ret(out, ok);
}

pub(crate) unsafe extern "C" fn tramp_despec_get_orientation(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = des_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.orientation)).unwrap_or([0f32; 4]);
    let mut b = [0u8; 16];
    for i in 0..4 {
        b[i * 4..i * 4 + 4].copy_from_slice(&v[i].to_le_bytes());
    }
    write_raw16_ret(out, &b);
}

pub unsafe fn register_dynamicentityspec(reg: &Registry) -> String {
    let iscriptable = reg.class_by_name("IScriptable");
    if !rtti::sane(iscriptable) {
        return "[reg] register_dynamicentityspec: 'IScriptable' não resolveu".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "DynamicEntitySpec", "gameuiInGameMenuGameController", iscriptable);
    if forged.is_null() {
        return "[reg] register_dynamicentityspec: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_dynamicentityspec: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    let m1 = register_method(reg, "DynamicEntitySpec", proto, "Create", "Create", tramp_despec_create, true);
    let m2 = register_method(reg, "DynamicEntitySpec", proto, "SetRecordID", "SetRecordID", tramp_despec_set_record_id, false);
    let m3 = register_method(reg, "DynamicEntitySpec", proto, "GetRecordID", "GetRecordID", tramp_despec_get_record_id, false);
    let m4 = register_method(reg, "DynamicEntitySpec", proto, "SetTemplatePath", "SetTemplatePath", tramp_despec_set_template_path, false);
    let m5 = register_method(reg, "DynamicEntitySpec", proto, "GetTemplatePath", "GetTemplatePath", tramp_despec_get_template_path, false);
    let m6 = register_method(reg, "DynamicEntitySpec", proto, "SetTemplateHash", "SetTemplateHash", tramp_despec_set_template_hash, false);
    let m7 = register_method(reg, "DynamicEntitySpec", proto, "GetTemplateHash", "GetTemplateHash", tramp_despec_get_template_hash, false);
    let m8 = register_method(reg, "DynamicEntitySpec", proto, "SetAppearanceName", "SetAppearanceName", tramp_despec_set_appearance_name, false);
    let m9 = register_method(reg, "DynamicEntitySpec", proto, "GetAppearanceName", "GetAppearanceName", tramp_despec_get_appearance_name, false);
    let m10 = register_method(reg, "DynamicEntitySpec", proto, "SetPersistState", "SetPersistState", tramp_despec_set_persist_state, false);
    let m11 = register_method(reg, "DynamicEntitySpec", proto, "GetPersistState", "GetPersistState", tramp_despec_get_persist_state, false);
    let m12 = register_method(reg, "DynamicEntitySpec", proto, "SetPersistSpawn", "SetPersistSpawn", tramp_despec_set_persist_spawn, false);
    let m13 = register_method(reg, "DynamicEntitySpec", proto, "GetPersistSpawn", "GetPersistSpawn", tramp_despec_get_persist_spawn, false);
    let m14 = register_method(reg, "DynamicEntitySpec", proto, "SetAlwaysSpawned", "SetAlwaysSpawned", tramp_despec_set_always_spawned, false);
    let m15 = register_method(reg, "DynamicEntitySpec", proto, "GetAlwaysSpawned", "GetAlwaysSpawned", tramp_despec_get_always_spawned, false);
    let m16 = register_method(reg, "DynamicEntitySpec", proto, "SetSpawnInView", "SetSpawnInView", tramp_despec_set_spawn_in_view, false);
    let m17 = register_method(reg, "DynamicEntitySpec", proto, "GetSpawnInView", "GetSpawnInView", tramp_despec_get_spawn_in_view, false);
    let m18 = register_method(reg, "DynamicEntitySpec", proto, "SetActive", "SetActive", tramp_despec_set_active, false);
    let m19 = register_method(reg, "DynamicEntitySpec", proto, "GetActive", "GetActive", tramp_despec_get_active, false);
    let m20 = register_method(reg, "DynamicEntitySpec", proto, "AddTag", "AddTag", tramp_despec_add_tag, false);
    let m21 = register_method(reg, "DynamicEntitySpec", proto, "GetTagCount", "GetTagCount", tramp_despec_get_tag_count, false);
    let m22 = register_method(reg, "DynamicEntitySpec", proto, "GetTagByIndex", "GetTagByIndex", tramp_despec_get_tag_by_index, false);
    let m23 = register_method(reg, "DynamicEntitySpec", proto, "SetPosition", "SetPosition", tramp_despec_set_position, false);
    let m24 = register_method(reg, "DynamicEntitySpec", proto, "GetPosition", "GetPosition", tramp_despec_get_position, false);
    let m25 = register_method(reg, "DynamicEntitySpec", proto, "SetOrientation", "SetOrientation", tramp_despec_set_orientation, false);
    let m26 = register_method(reg, "DynamicEntitySpec", proto, "GetOrientation", "GetOrientation", tramp_despec_get_orientation, false);
    format!(
        "[reg] DynamicEntitySpec: classe=OK (INSTANCIÁVEL, parent=IScriptable real, mesma receita de StaticEntitySpec/#76) \
Create={m1} SetRecordID={m2} GetRecordID={m3} SetTemplatePath={m4} GetTemplatePath={m5} \
SetTemplateHash={m6} GetTemplateHash={m7} SetAppearanceName={m8} GetAppearanceName={m9} \
SetPersistState={m10} GetPersistState={m11} SetPersistSpawn={m12} GetPersistSpawn={m13} \
SetAlwaysSpawned={m14} GetAlwaysSpawned={m15} SetSpawnInView={m16} GetSpawnInView={m17} \
SetActive={m18} GetActive={m19} AddTag={m20} GetTagCount={m21} GetTagByIndex={m22} \
SetPosition={m23} GetPosition={m24} SetOrientation={m25} GetOrientation={m26} \
— 12/12 campos reais cobertos (mask-fix de #76/rodada11 aplicado desde a origem nos 5 bools novos), prova ao vivo pendente"
    )
}


/// `BwmsPopulationSystemIsSpawning(sys: ref<IScriptable>, entityId: EntityID) -> Bool` — `IPopulationSystem::IsSpawning(EntityID)`,
/// `POP_IS_SPAWNING_VM` (slot Mac 0x1D8 da vtable `POPSYS_VTBL_VM`, medido pelos fixups; o 0x1D0 é a busca no mapa de entidades
/// registradas), o mesmo que o `DynamicEntitySystem.IsSpawning` chama. SÓ LEITURA. `sys` vem do script
/// (`BwmsGetSystemByName(n"gamePopulationSystem")`): só chama com a vtable do PopulationSystem conferida.
pub(crate) unsafe extern "C" fn tramp_population_is_spawning(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let entity_id = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let called = rtti::sane(sys)
        && crate::memoria::is_readable(sys, 8)
        && crate::jogo::sessao::sistema_com_vtable(sys, Some(POPSYS_VTBL_VM));
    let spawning = called && {
        let f: extern "C" fn(*mut c_void, u32) -> u32 = std::mem::transmute(crate::rebase(POP_IS_SPAWNING_VM));
        f(sys, entity_id as u32) & 0xff != 0
    };
    crate::log_dev!(
        "[populationsystem] BwmsPopulationSystemIsSpawning(sys={sys:p}, entityId={entity_id:#x}) -> {spawning} (chamado={called})"
    );
    write_bool_ret(out, spawning);
}

// ===== `GetWorkspotInstance` com forma própria do BWMS (2026-09-14) =====
// O `workWorkspotSystem` é o runtime system 31 da `RuntimeScene` (índice e base
// provados em 2026-08-22, `proofs/2026-08-22-runtime-systems-cena-PROVADO.log`; base também em `[CGameFramework+0x18]`, boot 210). No Mac a trava e o
// mapa ficam 4 e 8 B antes do Windows (desmonte de 2026-09-14 das 7 buscas do sistema a partir de `0x1045e3470`, o `IsActorInWorkspot` que o handler
// `0x10266fc28` chama com `[[gamesystem+0x40]+0x78]+0x1f0`): sob a trava compartilhada de 1 B `+0x4C` (`SharedSpinLock`: toma pelo `0x1000015c8`, solta
// com −1), o `InstanceMap` `+0x68` (`HashMap<u64, workWorkspotInstance*>`, chave = `EntityID.hash`, `hashedKey` e bucket pelos 32 B baixos da chave) dá a
// instância. O boot 210 caiu lendo o mapa em `+0x70` (o `indexTable` saiu `{21, 28}`, que são o tamanho e a capacidade). O tipo `workWorkspotInstance` não é visível ao redscript desta build, então o BWMS entrega os campos por natives globais pelo `EntityID` do
// ator: `resource` `+0x70` (o caminho do recurso), `animName` `+0x110`, `idleAnim` `+0x118` e `entryId` `+0x120` (com o `tree` `+0x60`,
// conferidos no teste pela classe dos handles).
pub(crate) const C82_SISTEMA_INDICE: usize = 31;

pub(crate) unsafe fn c82_ator(frame: *mut c_void) -> u64 {
    rtti::read_params_consuming(current_native_func(), frame).first().map_or(0, |(v, _)| *v)
}

/// `BwmsWorkspotHasInstance(actorID: EntityID) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_c82_tem(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let ator = c82_ator(frame);
    write_bool_ret(out, c82_ler(ator, |_| ()).is_some());
}

/// `BwmsWorkspotAnimName(actorID: EntityID) -> CName`.
pub(crate) unsafe extern "C" fn tramp_c82_anim(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let ator = c82_ator(frame);
    write_uint_ret(out, c82_ler(ator, |i| c82_campos(i).1).unwrap_or(0), 8);
}

/// `BwmsWorkspotIdleAnim(actorID: EntityID) -> CName`.
pub(crate) unsafe extern "C" fn tramp_c82_ocioso(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let ator = c82_ator(frame);
    write_uint_ret(out, c82_ler(ator, |i| c82_campos(i).2).unwrap_or(0), 8);
}

/// `BwmsWorkspotEntryId(actorID: EntityID) -> Uint32`.
pub(crate) unsafe extern "C" fn tramp_c82_entrada(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let ator = c82_ator(frame);
    write_uint_ret(out, c82_ler(ator, |i| u64::from(c82_campos(i).3)).unwrap_or(0), 4);
}

/// `BwmsWorkspotResourcePath(actorID: EntityID) -> Uint64`.
pub(crate) unsafe extern "C" fn tramp_c82_recurso(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let ator = c82_ator(frame);
    write_uint_ret(out, c82_ler(ator, |i| c82_campos(i).0).unwrap_or(0), 8);
}


/// Uma linha por `DynamicEntityTarget`: (ponteiro do alvo, tags do `Tag`/`Tags`; vazio = casa todo evento).
pub(crate) static DYNAMICENTITYTARGET_STATES: Mutex<Vec<(usize, Box<[u64]>)>> = Mutex::new(Vec::new());

pub(crate) const COMMUNITY_ENTRY_SPAWNER_OFF: usize = 0x30;

pub(crate) const COMMUNITY_ENTRY_NAME_OFF: usize = 0x20;

pub(crate) const COMMUNITY_GET_COMMUNITY_SLOT: usize = 0x228 + 0x08;

// `CommunityWrapper`/`CommunityEntryWrapper` sobre a `game::Community` viva. Cada wrapper guarda a comunidade pelo `WeakPtr` do
// `GetCommunity` e trava a comunidade a cada leitura (strong +1 por CAS): com a comunidade descarregada, os getters devolvem
// vazio. Layout: `Community` `entries` `+0x18` e `template_` `+0x38`; `CommunityEntry` `phases` `+0x08`, `name` `+0x20`, `active`
// `+0x28`, `spawner` `+0x30`, `spawnEntry` `+0x40`; `CommunityEntrySpawner` com os arrays de ids em `+0x18`/`+0x28`/`+0x38`/`+0x48`. A vtable da
// `game::Community` no Mac (`0x107c65b88`, pelos fixups do `__DATA_CONST`, sem typeinfo) tem 4 virtuais antes dos 2 destrutores — `GetActiveEntityIDs`
// `0x104709d58`, `GetActiveEntityCount` `0x104709e14`, `IsOwnedEntity` `0x104709eb0` e um 4º sem nome conhecido —; o destrutor é chamado direto
// pela soltura do `SharedStorage` (`0x104709894`).
pub(crate) const COMMUNITY_TEMPLATE_OFF: u64 = 0x38;

pub(crate) const COMMUNITY_ENTRY_PHASES_OFF: u64 = 0x08;

pub(crate) const COMMUNITY_ENTRY_ACTIVE_OFF: u64 = 0x28;

pub(crate) const COMMUNITY_ENTRY_SPAWN_ENTRY_OFF: u64 = 0x40;

/// Arrays de ids do `CommunityEntrySpawner`, na ordem dos getters: restaurados, gerados, gerando, reservados.
pub(crate) const COMMUNITY_SPAWNER_IDS_OFF: [u64; 4] = [0x18, 0x28, 0x38, 0x48];

/// **ACHADO AO VIVO 2026-08-14, RECONFIRMADO EM 2º BOOT INDEPENDENTE NA MESMA DATA (CONFIRMADO,
/// NÃO fecha o item)**: `BwmsPopulationSystemIsSpawning` testado em 2 boots — `sys` válido (via
/// `BwmsGetSystemByName`), `entityId=0x1`, `chamado=true`, retorno `false` DETERMINÍSTICO nas 2
/// vezes (inclusive na 2ª rodada, testado explicitamente contra o player REAL pós-autocontinue,
/// não só o espúrio do menu). Esses boots chamavam o slot `0x1D0`, que é a busca no mapa de entidades registradas, não o
/// IsSpawning (slot `0x1D8`, `POP_IS_SPAWNING_VM`, pelos fixups; ver `tramp_population_is_spawning`). `BwmsPersistencySystemGetEntityStatus` deu
/// sinal NEGATIVO CONFIRMADO: em AMBOS os boots, com o MESMO `entityId=0x1`, devolveu valores
/// DIFERENTES entre chamadas (`544817678200`/`544901860088` no 1º boot; `549473325816`≈`0x7fef2992f8`/
/// `549601367800`≈`0x7ff6cb56f8` no 2º boot, completamente independente) — sempre em FAIXA DE
/// PONTEIRO/HEAP, nunca no formato de enum pequeno (0/1/2) esperado. Com `entityId=0xdeadbeef`
/// devolveu `0` limpo e repetível nos 2 boots.
///
/// **Correção de hipótese importante (2º boot)**: `entityId=0x1` NÃO é um placeholder degenerado
/// exclusivo do "player espúrio do menu" — o 2º boot capturou a 2ª firing de `OnGameAttached`
/// DEPOIS de `[autocontinue] disparou`/`phase=5` confirmados (player REAL, save carregado de
/// verdade) e `player.GetEntityID()` retornou `0x1` DE NOVO, idêntico ao da 1ª firing. Ou seja: o
/// EntityID do player parece ser genuinamente fixo/reservado=1 neste engine (não hash), não um
/// artefato de estado transitório do menu — a causa da não-determinência do `GetEntityStatus`
/// está no SLOT/FUNÇÃO chamada, não no input degenerado.
///
/// **Decisão**: NÃO tentei mais nenhum slot vizinho (`0x1E8`/`0x1F8`/`0x200`) às cegas — vários
/// são funções `void` MUTATING conhecidas (`EnableDynamicEntityPersistence`/
/// `DisableDynamicEntityPersistence`), chamar sem disassembly seria adivinhação de alto risco, o
/// oposto da cautela pedida. `BwmsPersistencySystemGetEntityStatus` PERMANECE código ativo (zero
/// crash em 3 chamadas ao total, 2 boots independentes) mas o retorno **NÃO deve ser tratado como
/// confiável sob NENHUMA circunstância** — reprodução dupla eleva isso de "achado honesto,
/// inconclusivo" pra "confirmado negativo". Resolver de vez exige disassembly real do slot `0x1F0`
/// contra o binário Mac, não mais tentativa por offset de header.
pub(crate) unsafe fn register_persistency_population_natives(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(
        reg,
        proto,
        &["handle:IScriptable", "EntityID"],
        "BwmsPersistencySystemGetEntityStatus",
        "BwmsPersistencySystemGetEntityStatus",
        tramp_persistency_get_entity_status,
    );
    let ok2 = register_argful_by_types(
        reg,
        proto,
        &["handle:IScriptable", "EntityID"],
        "BwmsPopulationSystemIsSpawning",
        "BwmsPopulationSystemIsSpawning",
        tramp_population_is_spawning,
    );
    crate::log_dev!(
        "[reg] register_all: vtable (PersistencySystem/PopulationSystem) GetEntityStatus={ok1} IsSpawning={ok2}"
    );
}

/// Estado de uma entidade criada pelo `CreateEntity`. `status`: -1 sem pedido, 0 pedido, e daí o do token (1 criado, 2 record inválido,
/// 3 id duplicado). `pedido` é a geração do pedido de stub em curso (0 sem pedido): o callback de um pedido anterior (desligar e religar antes
/// do motor responder) não toma o stub, e o motor o apaga.
pub(crate) struct EntidadeDinamica {
    pub(crate) stubs: usize,
    pub(crate) pop: usize,
    pub(crate) holder: u64,
    pub(crate) status: i32,
    pub(crate) pedido: u64,
    pub(crate) registrada: bool,
    pub(crate) ativa: bool,
    pub(crate) record: u64,
    pub(crate) posicao: [f32; 4],
    pub(crate) orientacao: [f32; 4],
    pub(crate) aparencia: u64,
    pub(crate) sempre: bool,
    pub(crate) na_vista: bool,
}

pub(crate) const C82_TRAVA_OFF: usize = 0x4c;

pub(crate) const C82_MAPA_OFF: usize = 0x68;

/// `(tree, resource, animName, idleAnim, entryId)` da instância.
pub(crate) const C82_CAMPOS: [usize; 5] = [0x60, 0x70, 0x110, 0x118, 0x120];

/// O `workWorkspotSystem` vivo: o runtime system 31 com a classe conferida.
pub(crate) unsafe fn c82_sistema() -> Option<*mut u8> {
    let (s, nome) = runtime_system_at(C82_SISTEMA_INDICE);
    (s != 0 && nome == "workWorkspotSystem").then_some(s as *mut u8)
}

/// `ler(instância)` com a trava tomada; `None` sem sistema ou sem instância do ator.
pub(crate) unsafe fn c82_ler<T>(hash: u64, ler: impl FnOnce(*const u8) -> T) -> Option<T> {
    let s = c82_sistema()?;
    com_trava_de_leitura(s.add(C82_TRAVA_OFF), || hashmap_get(s.add(C82_MAPA_OFF), hash).filter(|i| *i != 0).map(|i| ler(i as *const u8)))
}

/// Campos da instância: `(caminho do recurso, animName, idleAnim, entryId)`.
pub(crate) unsafe fn c82_campos(inst: *const u8) -> (u64, u64, u64, u32) {
    let ler = |o: usize| (inst.add(o) as *const u64).read_unaligned();
    let recurso = ler(C82_CAMPOS[1]);
    let caminho = if recurso == 0 { 0 } else { ((recurso + 0x30) as *const u64).read_unaligned() };
    (caminho, ler(C82_CAMPOS[2]), ler(C82_CAMPOS[3]), (inst.add(C82_CAMPOS[4]) as *const u32).read_unaligned())
}

/// As 5 natives do workspot (`bwms-workspot.reds`) e a anotação do teste.
pub(crate) unsafe fn register_c82_natives(reg: &Registry, proto: *mut c_void) {
    let por_ator = |nome: &str, tramp: NativeHandler| {
        let p = compose_params_from_types(reg, &["EntityID"]);
        params_compostos(p) && register_global_composed(reg, proto, p, nome, nome, tramp)
    };
    let r = [
        por_ator("BwmsWorkspotHasInstance", tramp_c82_tem),
        por_ator("BwmsWorkspotAnimName", tramp_c82_anim),
        por_ator("BwmsWorkspotIdleAnim", tramp_c82_ocioso),
        por_ator("BwmsWorkspotEntryId", tramp_c82_entrada),
        por_ator("BwmsWorkspotResourcePath", tramp_c82_recurso),
    ];
    let anotar = {
        let p = compose_params_from_types(reg, &["Int32", "Uint64"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsC82Anotar", "BwmsC82Anotar", corpo_de_prova!(tramp_c82_anotar)))
    };
    crate::log_dev!("[reg] register_all: c82 workspot {r:?} BwmsC82Anotar={anotar}");
}

/// Vetores `B5|npcs|*` de `B5-vetores.txt`. Os estados são globais: cada teste usa ids próprios.
#[cfg(test)]
mod vetores_b5 {
    use super::*;

    fn entidade(holder: u64, pedido: u64, ativa: bool) -> EntidadeDinamica {
        EntidadeDinamica {
            stubs: 0x51,
            pop: 0x52,
            holder,
            status: -1,
            pedido,
            registrada: false,
            ativa,
            record: 0xabcd_0000_1234,
            posicao: [0.0; 4],
            orientacao: [0.0; 4],
            aparencia: 0,
            sempre: true,
            na_vista: true,
        }
    }

    /// `(holder, status, pedido, registrada, ativa)`.
    fn resumo(e: &EntidadeDinamica) -> (u64, i32, u64, bool, bool) {
        (e.holder, e.status, e.pedido, e.registrada, e.ativa)
    }

    fn estado(id: u64) -> Option<(u64, i32, u64, bool, bool)> {
        DYNENT_ESTADOS.lock().expect("estados").get(&id).map(resumo)
    }

    #[test]
    fn layouts() {
        assert_eq!((std::mem::size_of::<PedidoStub>(), std::mem::align_of::<PedidoStub>()), (0x40, 16));
        assert_eq!(std::mem::size_of::<PedidoRegistro>(), 0x28);
        assert_eq!((std::mem::size_of::<CallbackMotor>(), std::mem::offset_of!(CallbackMotor, tabela)), (0x90, 0x80));
    }

    #[test]
    fn soltar_token() {
        for (forte, depois) in [(0u32, 0u32), (1, 1), (2, 1), (5, 4)] {
            let bloco = std::sync::atomic::AtomicU32::new(forte);
            unsafe { dynent_soltar_token([0x1000, &bloco as *const _ as u64]) };
            assert_eq!(bloco.load(Ordering::Relaxed), depois, "forte {forte}");
        }
        unsafe { dynent_soltar_token([0x1000, 0]) };
    }

    #[test]
    fn marcar_spawn() {
        let casos = [
            ([1.5, -2.25, 100.0, 1.0], [0.0; 4], [196608, -294912, 13107200], [0.0, 0.0, 0.0, 1.0]),
            ([0.0; 4], [0.1, 0.2, 0.3, 0.9], [0, 0, 0], [0.1, 0.2, 0.3, 0.9]),
            ([-3.8e-6, 16000.0, -16000.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0, 2097152000, -2097152000], [0.0, 0.0, 0.0, 1.0]),
        ];
        for (pos, ori, posicao, orientacao) in casos {
            let mut e = entidade(0, 0, true);
            e.posicao = pos;
            e.orientacao = ori;
            e.status = 9;
            let antes = DYNENT_PEDIDOS.load(Ordering::Relaxed);
            let p = dynent_marcar_spawn(0x42, &mut e);
            assert_eq!((p.stubs, p.pedido.id, p.pedido._08, p.pedido.posicao, p.pedido._1c), (0x51, 0x42, 0, posicao, 0));
            assert_eq!((p.pedido.orientacao, p.pedido.record, p.pedido._38), (orientacao, 0xabcd_0000_1234, 0));
            assert!(p.geracao > antes && e.pedido == p.geracao && e.status == 0);
        }
    }

    #[test]
    fn despawnar() {
        for (holder, pedido, mesma, ret) in [(0, 0, true, false), (0, 7, true, true), (0, 7, false, true), (0x99, 0, false, true)] {
            let mut e = entidade(holder, pedido, true);
            e.registrada = true;
            assert_eq!(unsafe { dynent_despawnar(0xb51001, &mut e, mesma) }, ret);
            assert_eq!(resumo(&e), (0, -1, 0, false, true));
        }
    }

    #[test]
    fn cb_invoke() {
        let id = 0xb51002u64;
        let mut e = entidade(0, 0, true);
        let geracao = dynent_marcar_spawn(id, &mut e).geracao;
        DYNENT_ESTADOS.lock().expect("estados").insert(id, e);
        let mut token = [0x5555u64, id, 2];
        let mut buffer = [0u8; 0x80];
        buffer[..8].copy_from_slice(&id.to_le_bytes());
        buffer[8..16].copy_from_slice(&geracao.to_le_bytes());
        let invocar = |token: &mut [u64; 3], buffer: &mut [u8; 0x80]| unsafe {
            let mut sp = [token.as_mut_ptr() as u64, 0];
            dynent_cb_invoke(buffer.as_mut_ptr(), &mut sp);
        };
        invocar(&mut token, &mut buffer);
        assert_eq!((estado(id), token[0]), (Some((0, 2, geracao, false, true)), 0x5555), "status 2");

        let mut antigo = buffer;
        antigo[8..16].copy_from_slice(&(geracao - 1).to_le_bytes());
        token[2] = 1;
        invocar(&mut token, &mut antigo);
        assert_eq!((estado(id), token[0]), (Some((0, 2, geracao, false, true)), 0x5555), "pedido antigo");

        let mut sem_stub = [0u64, id, 1];
        invocar(&mut sem_stub, &mut buffer);
        assert_eq!((estado(id), sem_stub[0]), (Some((0, 1, geracao, false, true)), 0), "status 1, stub 0");

        let mut outro = [0x5555u64, id + 1, 1];
        invocar(&mut outro, &mut buffer);
        assert_eq!((estado(id), outro[0]), (Some((0, 1, geracao, false, true)), 0x5555), "status 1, id do token diferente");

        unsafe {
            dynent_cb_invoke(buffer.as_mut_ptr(), std::ptr::null_mut());
            dynent_cb_invoke(buffer.as_mut_ptr(), &mut [0, 0]);
        }
        DYNENT_ESTADOS.lock().expect("estados").remove(&id);
    }

    #[test]
    fn ligar() {
        let id = 0xb51004u64;
        DYNENT_ESTADOS.lock().expect("estados").insert(id, entidade(0, 7, true));
        assert!(unsafe { dynent_ligar(id, false) }, "ativa, desligar");
        assert_eq!(estado(id), Some((0, -1, 0, false, false)));
        assert!(!unsafe { dynent_ligar(id, false) }, "inativa, desligar de novo");
        assert_eq!(estado(id), Some((0, -1, 0, false, false)));
        assert!(!unsafe { dynent_ligar(id, true) }, "inativa, ligar fora da sessão");
        assert_eq!(estado(id), Some((0, -1, 0, false, true)));
        assert!(!unsafe { dynent_ligar(0xb5_1fff, true) }, "id desconhecido");
        DYNENT_ESTADOS.lock().expect("estados").remove(&id);
    }

    #[test]
    fn apagar() {
        let id = 0xb51003u64;
        DYNENT_ESTADOS.lock().expect("estados").insert(id, entidade(0, 0, true));
        for tag in [0x71u64, 0x72] {
            dynent_tags().lock().expect("tags").entry(id).or_default().insert(tag);
            dynent_by_tag().lock().expect("tags").entry(tag).or_default().insert(id);
        }
        DYNENT_MORTOS.lock().expect("mortos").insert(id);
        assert!(unsafe { dynent_apagar(id) });
        assert_eq!(estado(id), None);
        assert!(!dynent_tags().lock().expect("tags").contains_key(&id));
        assert_eq!(dynent_by_tag().lock().expect("tags").get(&0x71).map(|s| s.contains(&id)), Some(false));
        let mut apagadas = DYNENT_TAGS_APAGADAS.lock().expect("apagadas").get(&id).cloned().unwrap_or_default();
        apagadas.sort_unstable();
        assert_eq!(apagadas, vec![0x71, 0x72]);
        assert!(!DYNENT_MORTOS.lock().expect("mortos").contains(&id));
        let eventos: Vec<(u32, Vec<u64>)> = DYNENT_EVENTOS.lock().expect("eventos").iter().filter(|e| e.1 == id).map(|e| {
            let mut t = e.2.clone();
            t.sort_unstable();
            (e.0, t)
        }).collect();
        assert_eq!(eventos, vec![(EVENTO_APAGADA, vec![0x71, 0x72])]);
        assert!(!unsafe { dynent_apagar(0xb5_2fff) }, "id desconhecido");
    }

    #[test]
    fn criar_e_sistemas_sem_motor() {
        let spec = |record| DynamicEntitySpecFields { record_id: record, active: true, tags: vec![0x73], ..Default::default() };
        let eventos = || DYNENT_EVENTOS.lock().expect("eventos").iter().filter(|e| e.2 == vec![0x73]).count();
        assert_eq!(unsafe { dynent_criar(&spec(0)) }, 0);
        assert_eq!(unsafe { dynent_criar(&spec(0x1234)) }, 0);
        assert_eq!(eventos(), 0);
        assert!(unsafe { dynent_sistemas() }.is_none());
    }
}

