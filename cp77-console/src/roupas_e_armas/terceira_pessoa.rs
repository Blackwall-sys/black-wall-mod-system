//! ArchiveXL `#53` no Mac (2026-09-11): sub-slots de cabeça e rosto na representação em terceira pessoa.
//!
//! Um slot com o flat `.parentSlot` apontando pra `AttachmentSlots.Head` ou `AttachmentSlots.Eyes` (declarado
//! por mod) passa a ser tratado como o slot-base pelo `TPPRepresentationComponent`. Na fonte de referência isso
//! são dois hooks: depois do `OnAttach` e depois do `IsAffectedSlot(TweakDBID)`. No Mac o `IsAffectedSlot` não
//! existe como função — está inlinado nos handlers de equip e unequip do listener —, então o equivalente é
//! hookar os dois handlers e rodar o ramo "afetado" quando o original não o rodou.
//!
//!     OnAttach              0x101e9ff18  (comp, a2)                    depois: register::onattachtpp_compose
//!     handler de equip      0x101ea5b54  (comp, itemTDBID, slotTDBID)  depois: RegisterAffectedItem do item do slot
//!     handler de unequip    0x101ea50bc  (comp, itemTDBID, slotTDBID)  depois: 0x101ea5f80(comp, itemTDBID)
//!     RegisterAffectedItem  0x101ea0738  (comp, itemTDBID, &Handle<ItemObject>)
//!
//! Endereços e argumentos conferidos ao vivo pelo censo (boots 10 e 11). Diferença de ordem: no original o ramo
//! afetado roda antes do resto do handler; aqui roda depois. O slot-base sai do flat na hora
//! (`get_flat_tdbid`), em vez do cache que a referência monta no `InitializeSlots`.
//!
//! Gate: `~/.bwms-axl-tpp` (desligado por padrão até a prova ao vivo). Relatório no canal de veredito.

use crate::register::rd_u64;
use bwms_primitives::tweak_db_id_chave;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

const EQUIP_VM: u64 = 0x1_01ea_5b54;
const UNEQUIP_VM: u64 = 0x1_01ea_50bc;
const REGISTER_AFFECTED_VM: u64 = 0x1_01ea_0738;
const UNREGISTER_AFFECTED_VM: u64 = 0x1_01ea_5f80;

const COMPONENTE_DONO_OFF: usize = 0x50;
const ITEMOBJECT_ITEMID_OFF: usize = 0x288;
const ATTACHSLOTS_DADOS_OFF: usize = 0xB0;
const SLOTDATA_TAM: usize = 0x90;
const SLOTDATA_ITEMOBJECT_OFF: usize = 0x08;

static ORIG_ONATTACH: AtomicU64 = AtomicU64::new(0);
static ORIG_EQUIP: AtomicU64 = AtomicU64::new(0);
static ORIG_UNEQUIP: AtomicU64 = AtomicU64::new(0);
/// Último componente visto no handler de equip — o do player (o `OnAttach` também passa por um espúrio).
static ULTIMO_COMP: AtomicU64 = AtomicU64::new(0);
static REGISTRADOS: AtomicU64 = AtomicU64::new(0);
static DESREGISTRADOS: AtomicU64 = AtomicU64::new(0);

fn slots_afetados() -> (u64, u64) {
    static S: OnceLock<(u64, u64)> = OnceLock::new();
    *S.get_or_init(|| {
        (
            bwms_primitives::tweak_db_id("AttachmentSlots.Head"),
            bwms_primitives::tweak_db_id("AttachmentSlots.Eyes"),
        )
    })
}

fn eh_afetado_direto(slot: u64, (head, eyes): (u64, u64)) -> bool {
    let s = tweak_db_id_chave(slot);
    s == head || s == eyes
}

/// O ramo "afetado" roda depois do original só quando o original não o rodou (o slot não é Head nem Eyes) e o
/// slot-base declarado é um dos dois.
fn precisa_ramo_afetado(slot: u64, base: Option<u64>, afetados: (u64, u64)) -> bool {
    !eh_afetado_direto(slot, afetados) && matches!(base, Some(b) if eh_afetado_direto(b, afetados))
}

unsafe fn slot_base(slot: u64) -> Option<u64> {
    let t = crate::tweakdb::singleton()?;
    crate::tweakdb::get_flat_tdbid(t, bwms_primitives::tweak_db_id_derive(tweak_db_id_chave(slot), ".parentSlot"))
}

/// Endereço do `Handle<ItemObject>` do `slot` no `AttachmentSlots` do dono do componente.
unsafe fn handle_do_slot(comp: *mut c_void, slot: u64) -> Result<*const c_void, &'static str> {
    if comp.is_null() || !crate::memoria::is_readable(comp as *const c_void, COMPONENTE_DONO_OFF + 8) {
        return Err("componente ilegível");
    }
    let dono = rd_u64(comp as *const c_void, COMPONENTE_DONO_OFF) as *mut c_void;
    let reg = crate::rtti::Registry::obtain().ok_or("registry indisponível")?;
    let slots = crate::register::find_attachslots_component(&reg, dono).ok_or("AttachmentSlots do dono não achado")?;
    let dados = (slots as *const u8).add(ATTACHSLOTS_DADOS_OFF);
    let entradas = (dados as *const u64).read_unaligned() as *const u8;
    let n = (dados.add(0x0C) as *const u32).read_unaligned() as usize;
    if entradas.is_null() {
        return Err("AttachmentSlots sem dados de slot");
    }
    for i in 0..n.min(256) {
        let e = entradas.add(i * SLOTDATA_TAM);
        if crate::memoria::is_readable(e as *const c_void, SLOTDATA_TAM)
            && tweak_db_id_chave((e as *const u64).read_unaligned()) == tweak_db_id_chave(slot)
        {
            return Ok(e.add(SLOTDATA_ITEMOBJECT_OFF) as *const c_void);
        }
    }
    Err("slot fora do AttachmentSlots do dono")
}

/// `RegisterAffectedItem(comp, itemTDBID, &Handle<ItemObject>)` pro item que está no `slot`. Devolve o TweakDBID
/// do item (o `ItemObject::ItemID.tdbid`, como a referência passa).
pub(crate) unsafe fn registrar_item_do_slot(comp: *mut c_void, slot: u64) -> Result<u64, &'static str> {
    let handle = handle_do_slot(comp, slot)?;
    let obj = rd_u64(handle, 0);
    if obj == 0 {
        return Err("slot sem itemObject");
    }
    if !crate::memoria::is_readable(obj as *const c_void, ITEMOBJECT_ITEMID_OFF + 8) {
        return Err("itemObject ilegível");
    }
    let item = rd_u64(obj as *const c_void, ITEMOBJECT_ITEMID_OFF);
    let f: unsafe extern "C" fn(*mut c_void, u64, *const c_void) =
        std::mem::transmute(crate::rebase(REGISTER_AFFECTED_VM));
    f(comp, item, handle);
    REGISTRADOS.fetch_add(1, Ordering::Relaxed);
    Ok(item)
}

unsafe fn desregistrar_item(comp: *mut c_void, item: u64) {
    let f: unsafe extern "C" fn(*mut c_void, u64) = std::mem::transmute(crate::rebase(UNREGISTER_AFFECTED_VM));
    f(comp, item);
    DESREGISTRADOS.fetch_add(1, Ordering::Relaxed);
}

unsafe extern "C" fn onattach(comp: *mut c_void, a2: u64) {
    let orig = ORIG_ONATTACH.load(Ordering::Relaxed);
    if orig != 0 {
        let f: unsafe extern "C" fn(*mut c_void, u64) = std::mem::transmute(orig as usize);
        f(comp, a2);
    }
    if let Some(reg) = crate::rtti::Registry::obtain() {
        crate::log_veredito(&crate::corpo_e_rosto::onattachtpp_compose(&reg, comp));
    }
}

unsafe extern "C" fn equip(comp: *mut c_void, item: u64, slot: u64) {
    let orig = ORIG_EQUIP.load(Ordering::Relaxed);
    if orig != 0 {
        let f: unsafe extern "C" fn(*mut c_void, u64, u64) = std::mem::transmute(orig as usize);
        f(comp, item, slot);
    }
    ULTIMO_COMP.store(comp as u64, Ordering::Relaxed);
    let afetados = slots_afetados();
    if eh_afetado_direto(slot, afetados) {
        return;
    }
    let base = slot_base(slot);
    if !precisa_ramo_afetado(slot, base, afetados) {
        return;
    }
    let r = registrar_item_do_slot(comp, slot);
    crate::veredito_dev!(
        "[axl-tpp] equip slot={slot:#x} base={:#x} item={item:#x} -> RegisterAffectedItem {r:?}",
        base.unwrap_or(0)
    );
}

unsafe extern "C" fn unequip(comp: *mut c_void, item: u64, slot: u64) {
    let orig = ORIG_UNEQUIP.load(Ordering::Relaxed);
    if orig != 0 {
        let f: unsafe extern "C" fn(*mut c_void, u64, u64) = std::mem::transmute(orig as usize);
        f(comp, item, slot);
    }
    let afetados = slots_afetados();
    if eh_afetado_direto(slot, afetados) {
        return;
    }
    let base = slot_base(slot);
    if !precisa_ramo_afetado(slot, base, afetados) {
        return;
    }
    desregistrar_item(comp, item);
    crate::veredito_dev!(
        "[axl-tpp] unequip slot={slot:#x} base={:#x} item={item:#x} -> desregistrado",
        base.unwrap_or(0)
    );
}

/// Instala os 3 hooks se `~/.bwms-axl-tpp` existir. Chamado logo depois do hook do executor (`inicio/boot.rs`).
pub(crate) unsafe fn install() {
    if !crate::marcador_na_home(".bwms-axl-tpp") {
        return;
    }
    // Dois hooks no mesmo alvo entram em loop no relocador: o probe antigo do OnAttach mora no mesmo endereço.
    if crate::marcador_na_home(".bwms-hook-tpponattachc2") || crate::marcador_na_home(".bwms-hook") {
        crate::veredito_dev!("[axl-tpp] probe antigo do OnAttach ligado (~/.bwms-hook-tpponattachc2 ou ~/.bwms-hook): nada instalado");
        return;
    }
    let alvos: [(&str, u64, *mut c_void, &AtomicU64); 3] = [
        ("OnAttach", crate::jogo::anexos::ANEXO_AO_ANEXAR_VM, onattach as unsafe extern "C" fn(*mut c_void, u64) as *mut c_void, &ORIG_ONATTACH),
        ("equip", EQUIP_VM, equip as unsafe extern "C" fn(*mut c_void, u64, u64) as *mut c_void, &ORIG_EQUIP),
        ("unequip", UNEQUIP_VM, unequip as unsafe extern "C" fn(*mut c_void, u64, u64) as *mut c_void, &ORIG_UNEQUIP),
    ];
    let mut ok = 0usize;
    for (nome, vm, repl, orig) in alvos {
        let alvo = crate::rebase(vm);
        if alvo.is_null() || !crate::memoria::is_readable(alvo as *const c_void, 16) {
            crate::veredito_dev!("[axl-tpp] {nome} {vm:#x}: alvo ilegível");
            continue;
        }
        match crate::ganchos::instalar_com_threads_paradas(alvo, repl, Some(orig)) {
            Some(_) => ok += 1,
            None => crate::veredito_dev!("[axl-tpp] {nome} {vm:#x}: replace recusou"),
        }
    }
    crate::veredito_dev!("[axl-tpp] {ok}/3 hooks instalados (gate ~/.bwms-axl-tpp)");
}

/// Comando `tppafetados`: tamanho dos arrays do componente e contadores.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn relatorio() {
    let comp = ULTIMO_COMP.load(Ordering::Relaxed) as *const c_void;
    if comp.is_null() || !crate::memoria::is_readable(comp, 0x150) {
        crate::veredito_dev!("[axl-tpp] nenhum componente visto no handler de equip");
        return;
    }
    let n = |off: usize| ((comp as *const u8).add(off) as *const u32).read_unaligned();
    let q = |off: usize| ((comp as *const u8).add(off) as *const u64).read_unaligned();
    // O RegisterAffectedItem insere {TweakDBID, Handle} no contêiner de +0x128 (via 0x1035a652c, sob o mutex de
    // +0x120..+0x127) — é ali que a ida-e-volta tem que aparecer.
    crate::veredito_dev!(
        "[axl-tpp] comp={comp:p} tamanhos: +0x90={} +0xa0={} affectedAppearanceSlots(+0xb0)={} +0x110={} | +0x128=[{:#x} {:#x} {:#x}] n(+0x134)={} | registrados={} desregistrados={}",
        n(0x9c),
        n(0xac),
        n(0xbc),
        n(0x11c),
        q(0x128),
        q(0x130),
        q(0x138),
        n(0x134),
        REGISTRADOS.load(Ordering::Relaxed),
        DESREGISTRADOS.load(Ordering::Relaxed)
    );
    // Quais slots já estão em `affectedAppearanceSlots` — decide se um sub-slot testado passa pelo push ou pelo dedupe.
    let lista = q(0xb0) as *const u64;
    let quantos = (n(0xbc) as usize).min(32);
    if !lista.is_null() && crate::memoria::is_readable(lista as *const c_void, quantos * 8) {
        let ids: Vec<String> = (0..quantos).map(|i| format!("{:#x}", tweak_db_id_chave(lista.add(i).read_unaligned()))).collect();
        crate::veredito_dev!("[axl-tpp] affectedAppearanceSlots=[{}]", ids.join(" "));
    }
}

#[cfg(feature = "devtools")]
fn comp_para_teste(cmd: &str) -> Option<*mut c_void> {
    if !crate::marcador_na_home(".bwms-axl-tpp") {
        crate::veredito_dev!("[axl-tpp] {cmd}: gate ~/.bwms-axl-tpp desligado");
        return None;
    }
    let comp = ULTIMO_COMP.load(Ordering::Relaxed) as *mut c_void;
    if comp.is_null() {
        crate::veredito_dev!("[axl-tpp] {cmd}: nenhum componente visto no handler de equip");
        return None;
    }
    Some(comp)
}

/// Comando `tppregteste <slot>`: registra como afetado o item de um slot vanilla, direto. Testa a ABI do
/// `RegisterAffectedItem` sem precisar de `.parentSlot`; muda a representação em terceira pessoa até o
/// `tppdesregteste` do mesmo slot.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn registrar_teste(slot_nome: &str) {
    let Some(comp) = comp_para_teste("tppregteste") else { return };
    relatorio();
    let slot = bwms_primitives::tweak_db_id(slot_nome);
    let r = registrar_item_do_slot(comp, slot);
    crate::veredito_dev!("[axl-tpp] tppregteste {slot_nome} ({slot:#x}) -> {r:?}");
    relatorio();
}

/// Comando `tppdesregteste <slot>`: desfaz o `tppregteste` pelo unregister do motor.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn desregistrar_teste(slot_nome: &str) {
    let Some(comp) = comp_para_teste("tppdesregteste") else { return };
    let slot = bwms_primitives::tweak_db_id(slot_nome);
    let item = match handle_do_slot(comp, slot) {
        Ok(h) => {
            let obj = rd_u64(h, 0);
            if obj == 0 || !crate::memoria::is_readable(obj as *const c_void, ITEMOBJECT_ITEMID_OFF + 8) {
                crate::veredito_dev!("[axl-tpp] tppdesregteste {slot_nome}: slot sem item");
                return;
            }
            rd_u64(obj as *const c_void, ITEMOBJECT_ITEMID_OFF)
        }
        Err(motivo) => {
            crate::veredito_dev!("[axl-tpp] tppdesregteste {slot_nome}: {motivo}");
            return;
        }
    };
    desregistrar_item(comp, item);
    crate::veredito_dev!("[axl-tpp] tppdesregteste {slot_nome} item={item:#x} -> desregistrado");
    relatorio();
}

/// Comandos `tppequipteste <slot>` / `tppunequipteste <slot>`: chamam os handlers de equip/unequip do listener pelo
/// endereço hookado, com o item que está no slot. Passa pelo handler original, pela busca do `.parentSlot` e pelo ramo
/// "afetado" do port — o caminho inteiro do `IsAffectedSlot` inlinado, sem precisar re-equipar pelo jogo. Com o flat
/// criado antes (`mkflatnovo AttachmentSlots.X.parentSlot ...`), o contêiner de `+0x128` tem que subir e voltar.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn handler_teste(slot_nome: &str, equipar: bool) {
    let cmd = if equipar { "tppequipteste" } else { "tppunequipteste" };
    let Some(comp) = comp_para_teste(cmd) else { return };
    let slot = bwms_primitives::tweak_db_id(slot_nome);
    let item = match handle_do_slot(comp, slot) {
        Ok(h) => {
            let obj = rd_u64(h, 0);
            if obj == 0 || !crate::memoria::is_readable(obj as *const c_void, ITEMOBJECT_ITEMID_OFF + 8) {
                crate::veredito_dev!("[axl-tpp] {cmd} {slot_nome}: slot sem item");
                return;
            }
            rd_u64(obj as *const c_void, ITEMOBJECT_ITEMID_OFF)
        }
        Err(motivo) => {
            crate::veredito_dev!("[axl-tpp] {cmd} {slot_nome}: {motivo}");
            return;
        }
    };
    let base = slot_base(slot);
    relatorio();
    let alvo = crate::rebase(if equipar { EQUIP_VM } else { UNEQUIP_VM });
    let f: unsafe extern "C" fn(*mut c_void, u64, u64) = std::mem::transmute(alvo);
    f(comp, item, slot);
    crate::veredito_dev!(
        "[axl-tpp] {cmd} {slot_nome} ({slot:#x}) item={item:#x} base={:#x} precisa_ramo={}",
        base.unwrap_or(0),
        precisa_ramo_afetado(slot, base, slots_afetados())
    );
    relatorio();
}

/// Comando `tppanexarteste`: roda a composição do `OnAttach` (a mesma que o hook roda depois do original) no componente
/// do player, agora com o flat `.parentSlot` já criado (`mkflatnovo`). Com sub-slot real, tem que empurrar o sub-slot em
/// `affectedAppearanceSlots` (`+0xb0`) e registrar o item dele (contêiner `+0x128`).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn anexar_teste() {
    let Some(comp) = comp_para_teste("tppanexarteste") else { return };
    relatorio();
    match crate::rtti::Registry::obtain() {
        Some(reg) => crate::log_veredito(&crate::corpo_e_rosto::onattachtpp_compose(&reg, comp)),
        None => crate::veredito_dev!("[axl-tpp] tppanexarteste: registry indisponível"),
    }
    relatorio();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Valores do init estático do binário (`proofs/20260911-archivexl-53-...`, seção 5).
    const HEAD: u64 = 0x14_ee68_aa23;
    const EYES: u64 = 0x14_ee81_6829;
    const SUB: u64 = 0x1c_1234_5678;

    #[test]
    fn slots_afetados_batem_com_o_init_do_binario() {
        assert_eq!(slots_afetados(), (HEAD, EYES));
    }

    #[test]
    fn slot_base_direto_nao_roda_o_ramo_de_novo() {
        assert!(!precisa_ramo_afetado(HEAD, Some(EYES), (HEAD, EYES)));
        assert!(!precisa_ramo_afetado(EYES | (7u64 << 40), Some(HEAD), (HEAD, EYES)), "tdbOffset no slot");
    }

    #[test]
    fn subslot_de_head_ou_eyes_roda_o_ramo() {
        assert!(precisa_ramo_afetado(SUB, Some(HEAD), (HEAD, EYES)));
        assert!(precisa_ramo_afetado(SUB, Some(EYES | (0xABu64 << 48)), (HEAD, EYES)), "tdbOffset no valor do flat");
    }

    #[test]
    fn sem_parent_ou_com_outro_parent_nao_roda() {
        assert!(!precisa_ramo_afetado(SUB, None, (HEAD, EYES)));
        assert!(!precisa_ramo_afetado(SUB, Some(0x15_0000_0001), (HEAD, EYES)));
    }

    #[test]
    fn id_do_flat_parentslot_e_o_do_nome_completo() {
        let torso = bwms_primitives::tweak_db_id("AttachmentSlots.Torso");
        assert_eq!(
            bwms_primitives::tweak_db_id_derive(torso | (0x42u64 << 40), ".parentSlot"),
            bwms_primitives::tweak_db_id("AttachmentSlots.Torso.parentSlot")
        );
    }
}
