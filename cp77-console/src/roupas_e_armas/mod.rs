//! Roupas e armas: componentes de roupa (offsets de malha, aparência, máscara de partes), equipar e conferir o equipado, guarda-roupa.

#![allow(dead_code)]

pub(crate) mod terceira_pessoa;

use crate::rtti::refcnt;
use crate::jogo::sessao::auth_or;
use crate::jogo::sessao::auth_player;
use crate::rtti::Arg;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use crate::rtti::{self, Registry};
use crate::register::*;
use crate::aparencia::*;

/// `BwmsEquipState() -> Int32` — Skill 2 (EQUIPAR item por código, "fazer aparecer no V" sem HID).
/// Lê `~/.bwms-equip`: um inteiro que seleciona qual roupa de teste equipar (0=nenhuma, 1/2/3 =
/// itens do jogo base pré-mapeados no `bwms-tppcam.reds`; mais tarde vira o ID de um mod). O poller
/// redscript, na transição, monta um `EquipRequest`+`ItemID.FromTDBID` e faz
/// `EquipmentSystem.GetInstance(player).QueueRequest(req)` com `addToInventory=true` (dá E equipa de
/// uma vez). O `new` do redscript cuida do Handle/refcount — o lado Rust só passa o número.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_equip_state(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = crate::caminho_de_marcador(".bwms-equip")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    write_uint_ret(out, v, 4);
}

/// `BwmsEquipLog(step: Int32)` — traça onde o poller de equipar chega (diagnóstico da Skill 2):
/// 1=disparou c/ sel>0 · 2=player ok · 3=EquipmentSystem ok · 4=QueueRequest chamado · 10+=readback.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_equip_log(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let step = args.first().map(|(v, _)| *v as i32).unwrap_or(0);
    crate::log_dev!("[skill2] equip step {step}");
    write_bool_ret(out, true);
}

/// `BwmsEquipCheck() -> Int32` — gatilho de READBACK do equip (prova definitiva por código): lê
/// `~/.bwms-equipcheck`. Quando vira 1, o poller lê o item ativo de cada slot de roupa e loga.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_equip_check(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = crate::caminho_de_marcador(".bwms-equipcheck")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    write_uint_ret(out, v, 4);
}

/// `BwmsEquipReadback(area: Int32, id: TweakDBID) -> Bool` — o poller passa o TweakDBID do item
/// equipado em cada slot; aqui a gente loga o hash + o nome se conhecido. Prova que a roupa
/// equipou de verdade (sem depender de câmera). Compara com os hashes dos itens de teste.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_equip_readback(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let area = args.first().map(|(v, _)| *v as i32).unwrap_or(-1);
    let id = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    // hashes dos itens de teste (bwms_primitives::tweak_db_id) pra reconhecer no log.
    let known = [
        ("Items.GOG_DLC_Jacket_Legendary", bwms_primitives::tweak_db_id("Items.GOG_DLC_Jacket_Legendary")),
        ("Items.Fixer_01_Set_TShirt", bwms_primitives::tweak_db_id("Items.Fixer_01_Set_TShirt")),
        ("Items.Coat_04_rich_02_Crafting", bwms_primitives::tweak_db_id("Items.Coat_04_rich_02_Crafting")),
    ];
    let name = known.iter().find(|(_, h)| *h == id).map(|(n, _)| *n).unwrap_or("(outro/vazio)");
    // area = código NOSSO passado pelo reds (1=OuterChest,2=InnerChest,3=Legs,4=Feet,5=Head,6=Face)
    let aname = match area {
        1 => "OuterChest",
        2 => "InnerChest",
        3 => "Legs",
        4 => "Feet",
        5 => "Head",
        6 => "Face",
        _ => "?",
    };
    crate::log_dev!("[skill2] slot {aname} = item {id:#018x} {name}");
    write_bool_ret(out, true);
}

// Codeware `WardrobeSystem.ForgetItemID` (catálogo item #46/#208, candidato barato, 2026-08-11,
// 3ª rodada) — ver `selftest::wardrobe_forget_by_tdbid` pro algoritmo/offsets completos (scan
// linear por `ItemID.tdbid`, mesma mecânica já provada de `wardrobe_forget_item`/2026-07-31,
// generalizada pra varrer todos os buckets em vez de 1 só). Registrado como GLOBAL (não
// `@addMethod`), mesmo padrão já usado por `BwmsGameObjectAddTag`/etc. — o `.reds` embrulha com
// `@addMethod(WardrobeSystem) public func ForgetItemID(itemID: ItemID) -> Bool` chamando
// `ItemID.GetTDBID(itemID).ToNumber()` (2 natives VANILLA reais já em uso no projeto).
pub(crate) unsafe extern "C" fn tramp_wardrobe_forget_by_tdbid(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut u8).unwrap_or(std::ptr::null_mut());
    let tdbid = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let mut removed: u32 = 0;
    if rtti::sane(sys as *mut c_void) && tdbid != 0 {
        removed = crate::roupas_e_armas::wardrobe_forget_by_tdbid(sys, tdbid);
    } else {
        crate::log_dev!("[wardrobe] ForgetItemID: sys={sys:p} ou tdbid={tdbid:#018x} inválido, abortado sem tocar memória");
    }
    // Contagem nunca é negativa: `write_uint_ret(out, v, 4)` grava o mesmo bit-pattern do Int32.
    write_uint_ret(out, removed as u64, 4);
}

/// Offset REAL de uma propriedade dentro do objeto, perguntado ao RTTI do jogo.
///
/// **Isto substituiu uma tabela de offsets por classe, e o motivo importa.** A tabela vinha dos
/// headers gerados do RED4ext.SDK e já tinha uma armadilha conhecida (o mesmo campo `mesh` fica em
/// `0x150` no `entMeshComponent` e noutro lugar no `entSkinnedMeshComponent`, que é outro ramo).
/// Aí a medição ao vivo mostrou uma segunda camada do problema: **o header diz `0x228` e nesta
/// build o campo está em `0x220`** — o próprio header declara `uint8_t unk1E0[0x228-0x1E0]`, ou
/// seja, o autor do SDK não conhecia essa faixa e chutou a borda.
///
/// Provado por conteúdo, não por forma: o `u64` em `0x220` reverte para
/// `base\characters\common\base_bodies\man_average\_old_to_delete\t0_001_ma_base_hq__torso.mesh`
/// nos 663.364 paths do install — o mesh do próprio componente `torso`. O de `0x228` não reverte
/// pra path nenhum.
///
/// O RTTI carrega o offset certo desta build em `CProperty::valueOffset@0x20`
/// (`RED4EXT_ASSERT_OFFSET` no SDK). Perguntar elimina a classe inteira do erro — e vale pra
/// QUALQUER propriedade, não só as 7 que eu tinha tabelado.
pub(crate) const CPROPERTY_VALUE_OFFSET: usize = 0x20;

/// Classifica um componente na MESMA ordem de `App::ComponentWrapper::ComponentWrapper`
/// (Wrapper.cpp:83-106): `MeshComponent` primeiro, depois `MorphTargetSkinnedMeshComponent`,
/// depois `GarmentSkinnedMeshComponent` (tem que vir ANTES de `SkinnedMeshComponent` puro —
/// Garment `IsA` Skinned também, checar na ordem errada classificaria Garment como Skinned
/// genérico, o que na prática não muda offset mas quebraria a fidelidade à fonte), por
/// último `SkinnedMeshComponent`.
pub(crate) unsafe fn detect_garment_component_kind(reg: &crate::rtti::Registry, inst: *mut c_void) -> GarmentComponentKind {
    if inst.is_null() {
        return GarmentComponentKind::Unsupported;
    }
    let cls = crate::rtti::class_of(inst);
    if cls.is_null() {
        return GarmentComponentKind::Unsupported;
    }
    if class_is_kind_of(cls, reg.class_by_name("entMeshComponent")) {
        return GarmentComponentKind::MeshComponent;
    }
    if class_is_kind_of(cls, reg.class_by_name("entMorphTargetSkinnedMeshComponent")) {
        return GarmentComponentKind::MorphTarget;
    }
    if class_is_kind_of(cls, reg.class_by_name("entGarmentSkinnedMeshComponent")) {
        return GarmentComponentKind::SkinnedFamily;
    }
    if class_is_kind_of(cls, reg.class_by_name("entSkinnedMeshComponent")) {
        return GarmentComponentKind::SkinnedFamily;
    }
    GarmentComponentKind::Unsupported
}

pub(crate) unsafe fn offset_por_rtti(inst: *mut c_void, nome: Option<&str>) -> Option<usize> {
    prop_value_offset(crate::rtti::class_of(inst), nome?)
}

/// Offset de `chunkMask` (`uint64_t`) no componente. `None` = tipo fora do escopo ou property não resolvida.
pub(crate) unsafe fn garment_chunk_mask_offset(inst: *mut c_void, kind: GarmentComponentKind) -> Option<usize> {
    offset_por_rtti(inst, nome_prop_chunk_mask(kind))
}

/// Offset de `meshAppearance` (`CName`) no componente.
pub(crate) unsafe fn garment_mesh_appearance_offset(inst: *mut c_void, kind: GarmentComponentKind) -> Option<usize> {
    offset_por_rtti(inst, nome_prop_aparencia(kind))
}

/// Offset do `RaRef<CMesh> mesh` (`morphResource` no MorphTarget) — `ComponentWrapper::GetResourcePath`/`SetResourcePath`.
/// `RaRef<T>` é só o `ResourcePath` de 8 bytes, sem indireção.
pub(crate) unsafe fn garment_mesh_path_offset(inst: *mut c_void, kind: GarmentComponentKind) -> Option<usize> {
    offset_por_rtti(inst, nome_prop_path(kind))
}

// ===== Codeware `#168` (`ComponentWrapper::ResetMaterialCache`) — 2026-08-15, RE AO VIVO
// dedicada: `CMesh::FindAppearance` FECHADO SEM ENDEREÇO NATIVO NENHUM =====
//
// A rodada 18 (2026-08-15, 100% offline) tentou 5 ângulos de RE de ENDEREÇO nativo pra
// `Raw::CMesh::FindAppearance(CMesh*, CName) -> Handle<mesh::MeshAppearance>&` (símbolo, header
// gerado, disassembly de `RefreshAppearance`, mineração de log-strings do handler de
// `AppearanceMeshLoadedEvent`, xref do array-reallocator) — TODOS negativos, concluindo que só
// RE ao vivo (breakpoint/memory-watch sobre o ptr-to-member de `SkinnedMeshComponent::
// OnMeshLoaded`) destravaria isso.
//
// **Esta rodada achou que não precisa de endereço nenhum — `FindAppearance` é um scan-linear
// TRIVIAL sobre dado 100% documentado pelo próprio RED4ext.SDK, implementável em Rust puro:**
//
//  - `CMesh::appearances: DynArray<Handle<mesh::MeshAppearance>> @ 0x1E0` — header HAND-WRITTEN
//    oficial (`RED4ext.SDK/include/RED4ext/Scripting/Natives/CMesh.hpp`), validado pelo próprio
//    SDK via `RED4EXT_ASSERT_SIZE(CMesh, 0x230)` contra o jogo real (não é palpite).
//  - `mesh::MeshAppearance::name: CName @ 0x30` — header GERADO da reflection real
//    (`Generated/mesh/MeshAppearance.hpp`, `RED4EXT_ASSERT_SIZE(MeshAppearance, 0xA8)`).
//  - `Handle<T>` = 16 bytes `{instance: T* @0x00, refCount: RefCnt* @0x08}`
//    (`RED4ext.SDK/include/RED4ext/Memory/SharedPtr.hpp`, `RED4EXT_ASSERT_OFFSET` explícito).
//  - `DynArray<T>` = 16 bytes header `{entries@0x00, capacity(u32)@0x08, size(u32)@0x0C}`
//    (`RED4ext.SDK/include/RED4ext/Containers/DynArray.hpp`, membros com offset em comentário).
//
// `FindAppearance` (a variante SEGURA, devolve vazio se não achar — em oposição a `GetAppearance`,
// que provavelmente afirma/crasha se não achar, por isso NÃO reimplementada aqui) é: pegar
// `CMesh::appearances`, andar as `size` entradas, comparar `entries[i].instance->name` contra o
// CName pedido, devolver o 1º match. Zero função nativa chamada, zero endereço de motor.
//
// **Entrada da função (`meshHandle`) — offset DIFERENTE do `mesh`/`RaRef<CMesh>` já usado por
// `GetResourcePath`** (esse é o caminho de CARREGAR; `meshHandle` é o `CMesh*` já RESOLVIDO,
// exatamente o que `ComponentWrapper::ResetMaterialCache` usa, `ComponentWrapper.cpp:369/374`):
//  - `MeshComponent`: fonte real faz `reinterpret_cast<Handle<CMesh>*>(&component->unk158)` —
//    o header GERADO não nomeia esse campo (bloco opaco `unk158[0x158-0x178]`), mas confirma
//    que o Handle começa exatamente em `0x158` (onde o bloco opaco começa).
//  - `SkinnedFamily` (`entSkinnedMeshComponent`/`entGarmentSkinnedMeshComponent`, ex. 'torso'):
//    `meshHandle: Handle<CMesh> @ 0x200` — header GERADO nomeia EXPLICITAMENTE (maior confiança
//    que o caso `MeshComponent` acima).
//  - `MorphTarget`: FORA de escopo desta rodada — a fonte real dereferencia mais 1 nível
//    (`meshHandle->baseMesh.Get()`, `MorphTargetMesh::baseMesh: Ref<CMesh> @ 0x60`,
//    `RED4ext.SDK/.../Scripting/Natives/MorphTargetMesh.hpp`) e `Ref<T>` (≠ `Handle<T>`) NUNCA
//    teve seu layout confirmado neste projeto — decisão consciente de não adivinhar.
//
// ⚠️ REGRA DE OURO: só a LEITURA (`FindAppearance`) foi implementada e é 100% observe-only
// (zero escrita). O RESTO de `ResetMaterialCache` (`MeshAppearanceEx::ResetMaterialCache`,
// `MeshAppearanceEx.hpp:19-42`) — `Red::WaitForJob(materialJob, 5000ms)` + lock exclusivo +
// `Raw::RenderData::Release(materialPtr.data)` + `materialMap.Clear()` + `materialCached=false`
// — precisa de PELO MENOS 2 endereços nativos novos (`WaitForJob`/`RenderData::Release`, nunca
// resolvidos em lugar nenhum deste projeto) + o layout de `Map<CName,Handle<IMaterial>>` (nunca
// confirmado) pra um `Clear()` seguro (decrementar refcount de cada entrada corretamente) — RE
// de endereço nativo genuína, deliberadamente NÃO tentada nesta rodada (risco de corromper
// dado de render live sem entender o `Clear()` por completo). Item `#168` continua REAL_GAP,
// mas o bloqueador que a rodada 18 achou insuperável sem RE ao vivo está removido — o resto
// do item é RE de endereço puro e conhecido, sem incerteza de mecanismo.
/// Família skinned: o `Handle<CMesh>` fica em `0x200` (2.5) — onde a closure de conclusão do `LoadResource` (`0x10199264c`) grava o
/// mesh carregado; `0x200` é o bloco de contagem desse handle.
pub(crate) fn garment_mesh_handle_offset(kind: GarmentComponentKind) -> Option<usize> {
    match kind {
        GarmentComponentKind::MeshComponent => Some(0x158),
        // 2.5: o `ISkinTarget` ganhou um campo em +0x180 e tudo da família skinned a partir dali andou +8 (LODMode 0x24A → 0x252, mesh
        // 0x220 → 0x228 pelo registro de propriedades); o handle carregado (+0x1F8 na 2.31) vai a +0x200 por essa conta, sem leitura direta.
        GarmentComponentKind::SkinnedFamily => Some(0x200),
        GarmentComponentKind::MorphTarget => None,
        GarmentComponentKind::Unsupported => None,
    }
}

/// ArchiveXL #97 (2026-08-11): `Facade::EnableGarmentOffsets()`/`DisableGarmentOffsets()` —
/// fonte real (`App/Extensions/Garment/Extension.cpp:883-890`) é um `static inline bool
/// s_garmentOffsetsEnabled` puro, sem chamada nativa nenhuma — só seta um flag global lido no
/// hook do `ComputePlayerGarment` ([`install_offsets_de_roupa`]).
pub(crate) static GARMENT_OFFSETS_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) unsafe extern "C" fn tramp_offsets_de_roupa_ligar(_c: *mut c_void, _f: *mut c_void, _out: *mut c_void, _rt: i64) {
    GARMENT_OFFSETS_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
    crate::log_dev!("[reg] >>> ArchiveXL.EnableGarmentOffsets chamado — flag=true");
}

pub(crate) unsafe extern "C" fn tramp_offsets_de_roupa_desligar(_c: *mut c_void, _f: *mut c_void, _out: *mut c_void, _rt: i64) {
    GARMENT_OFFSETS_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
    crate::log_dev!("[reg] >>> ArchiveXL.DisableGarmentOffsets chamado — flag=false");
}

/// Diagnóstico (não faz parte da API real do ArchiveXL — o C++ real não tem getter) pro comando
/// de canal `garmentoffsets` conseguir provar o round-trip Enable/Disable ao vivo.
pub fn garment_offsets_enabled() -> bool {
    GARMENT_OFFSETS_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}


/// axl-garment-apply / axl-transmog-apply (2026-08-02, /goal): endereço C++ RAW de
/// `ScriptableSystem::QueueRequest` (= `EquipmentSystem::QueueRequest`, herdado — `IEquipmentSystem`
/// não sobrescreve, confirmado por RE dedicada). Achado via a MESMA técnica que fechou
/// `IComponent::Toggle`/`vehicleController::ToggleLights` (bloco de nomes nativos no `__cstring` +
/// registrador de classe de `ScriptableSystem` + cadeia lazy-init por índice — `QueueRequest` é o
/// índice 0 dos 3 métodos nativos declarados em `core/systems/scriptableSystem.script`). Prólogo de
/// função real (`sub sp,#0x30; stp x20,x19...`), zero BL-callers diretos no binário (esperado — só é
/// invocado via o ponteiro guardado no `CClassFunction`/executor RTTI, nunca de outro call-site
/// compilado), cross-validado 2x (extração do registrador == boundary por disassembly a partir do
/// `ret` de QueueRequest). Corpo: dispatch genérico por tabela de função-ponteiro indexada por um
/// byte de tipo em `request+0x62` — bate com `ref<ScriptableSystemRequest>` desreferenciado DIRETO
/// (sem indireção de Handle), não um `Handle<T>` de 16 bytes.
///
/// Motivo de existir: chamar `QueueRequest` via RTTI normal (nosso `rtti::call_func`, ou o
/// interpretador de bytecode redscript chamando `es.QueueRequest(req)`) crasha SEMPRE — RE offline
/// completa (mesma sessão, ver HISTORICO.md 2026-08-02 cont.3-6) achou que `CClassFunction::GetInvokable()`
/// retorna NULL pro descritor RTTI ESPECÍFICO desse método (bit4 de `unkAC`/+0xAC, hipótese forte =
/// "requer despacho via job/fila do motor, não invocável do interpretador"). Chamar o endereço RAW
/// direto via `transmute`, TOTALMENTE fora do sistema RTTI/executor, contorna isso por completo —
/// mesmo padrão já provado com sucesso pra `CreateRecord`/`RecordExists` (`tweakdb/mod.rs`).
///
/// AINDA NÃO TESTADO AO VIVO. `sys`/`req` chegam como ponteiros RAW já resolvidos por
/// `read_params_consuming` (mesma receita de `tramp_callmethod` acima — `handle:IScriptable` em
/// ambos os params, `EquipmentSystem`/`EquipRequest` são IScriptable-derived, confirmado em
/// `redscript-src`). `sys` deve ser a instância de `EquipmentSystem` (`EquipmentSystem.GetInstance(player)`
/// em redscript); `req` a instância de `EquipRequest` já montada (mesmos campos de sempre: itemID/owner/
/// addToInventory/slotIndex). Risco honesto: o corpo real
/// da função (dispatch por tabela) nunca foi exercitado por nós antes — pode ter seu próprio crash
/// (ex. se a tabela de handler por tipo tiver o mesmo problema de bind que QueueRequest tinha via
/// GetInvokable); mas é uma categoria estruturalmente diferente (não passa pelo RTTI executor nem
/// por GetInvokable() nenhuma vez), então não é o MESMO beco-sem-saída já confirmado.
#[cfg(feature = "devtools")]
pub(crate) const ADDR_QUEUE_REQUEST_RAW: u64 = 0x1_0244_5e8c;

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_queue_request_raw(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    // 2026-08-03: log de ENTRADA — 2 crashes ao vivo anteriores não mostraram NENHUM log deste
    // trampolim (nem este nem o de antes do transmute), sugerindo que o crash acontece ANTES do
    // dispatch chegar aqui. Este log, se aparecer, prova que o trampolim FOI alcançado.
    crate::log_dev!("[queuerequestraw] ENTROU no trampolim (dispatch chegou aqui)");
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let req = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut ok = false;
    if !sys.is_null() && !req.is_null() && rtti::sane(sys) && rtti::sane(req) {
        crate::log_dev!(
            "[queuerequestraw] chamando 0x{:x} direto (sys={sys:p} req={req:p}) — bypass RTTI/GetInvokable",
            ADDR_QUEUE_REQUEST_RAW
        );
        let f: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(crate::rebase(ADDR_QUEUE_REQUEST_RAW));
        f(sys, req);
        ok = true;
        crate::log_dev!("[queuerequestraw] retornou, zero crash até aqui");
    } else {
        crate::log_dev!(
            "[queuerequestraw] args inválidos: sys={sys:p} req={req:p}"
        );
    }
    write_bool_ret(out, ok);
}

/// axl-garment-apply / axl-transmog-apply (2026-08-02, /goal): 3ª via, achado pelo agente de RE que
/// desmontou `GetInvokable()` em si (`0x1008c5c9c` — stub trivial, SEMPRE retorna null, ignora `this`
/// por completo; refuta a hipótese antiga do bit4/`unkAC` como mecanismo interno). A lógica condicional
/// REAL fica um nível acima, no caller (`Func C`/`0x101099c44` → `0x10109d20c`): se `struct+0x18`
/// (que é literalmente o `ctx` passado pro dispatcher) for não-nulo, um FAST-PATH devolve ele direto e
/// `GetInvokable()` NUNCA é chamado. `struct+0x18 = ctx` de `rtti::call_func(rf, ctx, args)` — ou seja,
/// **chamar `QueueRequest` via `rtti::call_func` NOSSO, com `ctx` explícito = ponteiro da instância
/// `EquipmentSystem`**, nunca foi tentado (toda tentativa anterior deixava o `es.QueueRequest(req)`
/// rodar DENTRO de bytecode redscript interpretado, onde o `ctx` não é controlado por nós — é aí que
/// aparentemente chega null). Mesmo padrão já PROVADO do comando `give` (`console.rs:126`,
/// `TransactionSystem::GiveItem` com `ctx=tx` explícito + `Arg::Handle(owner, refcnt())`).
///
/// Mais simples que `BwmsQueueRequestRaw` (não precisa do endereço cru nem de bypassar o RTTI inteiro),
/// mas MENOS robusto (ainda depende do RTTI/`GetInvokable()`/fast-path funcionarem como entendido) —
/// serve de 3º ponto de dados independente pro mesmo boot de teste.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_queue_request_ctx(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let req = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut ok = false;
    if !sys.is_null() && !req.is_null() && rtti::sane(sys) && rtti::sane(req) {
        if let Some(reg) = rtti::Registry::obtain() {
            if let Some(rf) = rtti::resolve_func(&reg, "EquipmentSystem", "QueueRequest") {
                crate::log_dev!(
                    "[queuerequestctx] call_func(QueueRequest, ctx=sys={sys:p}, req={req:p}) — ctx explícito, mesmo padrão do `give`"
                );
                let r = rtti::call_func(&rf, sys, &[rtti::Arg::Handle(req, crate::rtti::refcnt())]);
                ok = r.is_some();
                crate::log_dev!("[queuerequestctx] call_func -> {:?}, zero crash até aqui", r.is_some());
            } else {
                crate::log_dev!("[queuerequestctx] resolve_func(EquipmentSystem,QueueRequest) falhou");
            }
        } else {
            crate::log_dev!("[queuerequestctx] REGISTRY indisponível");
        }
    } else {
        crate::log_dev!("[queuerequestctx] args inválidos: sys={sys:p} req={req:p}");
    }
    write_bool_ret(out, ok);
}


/// `BwmsInvState() -> Int32` — Skill 1b (VER o V no preview 3D do inventário). Lê `~/.bwms-invpreview`:
/// 1 = abrir o inventário (mostra o modelo de corpo inteiro do V com a roupa), 0 = fechar. Rota
/// no-HID via `GameInstance.GetUISystem(game).QueueEvent(inkMenuInstance_SpawnEvent)`.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_inv_state(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = crate::caminho_de_marcador(".bwms-invpreview")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    write_uint_ret(out, v, 4);
}


/// `RegisterOffsetOverrides(estado, hash, definição, deslocamento)`: devolve quantas partes.
pub(crate) unsafe fn es20_registrar_offsets(entidade: u64, hash: u64, definicao: *const u8, deslocamento: i32) -> usize {
    let partes = es20_partes(definicao);
    for parte in &partes {
        add_offset_override(entidade, hash, *parte, deslocamento);
    }
    partes.len()
}

/// `ApplyOffsetOverrides(estado, offsets, resources)`: com tamanhos diferentes, `Reserve` do tamanho dos recursos (reserve do motor); ligados
/// (`EnableGarmentOffsets`), a lista refeita com o deslocamento de cada recurso (0 sem estado); desligados, o padrão, a lista vazia. `false` se o
/// `Reserve` não deu lugar.
pub(crate) unsafe fn es20_aplicar_deslocamentos(entidade: u64, deslocamentos: *mut u8, recursos: &[u64], ligados: bool) -> bool {
    let tamanho = deslocamentos.add(0x0c) as *mut u32;
    if tamanho.read_unaligned() as usize != recursos.len() {
        crate::jogo::listas::reservar(deslocamentos as *mut c_void, recursos.len() as u32, 4, 4, std::ptr::null());
    }
    tamanho.write_unaligned(0);
    if !ligados || recursos.is_empty() {
        return true;
    }
    let valores = (deslocamentos as *const u64).read_unaligned() as *mut i32;
    if valores.is_null() || ((deslocamentos.add(0x08) as *const u32).read_unaligned() as usize) < recursos.len() {
        return false;
    }
    for (k, recurso) in recursos.iter().enumerate() {
        valores.add(k).write_unaligned(get_offset_override(entidade, *recurso));
    }
    tamanho.write_unaligned(recursos.len() as u32);
    true
}

/// `AppearanceChanger::ComputePlayerGarment`, 8 parâmetros, com `aOffsets` em x2 e `SharedPtr<GarmentProcessingContext>&` em x3.
const COMPUTAR_ROUPA_VM: u64 = 0x1_0202_0a80;
const COMPUTAR_ROUPA_PROLOGO: u32 = 0xd104_83ff;
/// `resources` do contexto: `DynArray<ResourcePath>`.
const CONTEXTO_RECURSOS: u64 = 0x40;
static ORIG_COMPUTAR_ROUPA: AtomicU64 = AtomicU64::new(0);

/// Desligados, o padrão, a lista de offsets vai vazia ao motor, como o `OnComputeGarment` da referência. Sem isso o `garmentOffset` que o Equipment-EX grava em
/// todo `Clothing_Record` (100000+) chega ao montador e o V perde pedaços da roupa na terceira pessoa (lanes L-28 a L-34). Ligados, a lista do motor segue.
#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn computar_roupa_hook(a1: u64, entidade: *const u64, deslocamentos: *mut u8, dados: *const u64, a5: u64, a6: u64, a7: u64, a8: u64) {
    let contexto = if dados.is_null() { 0 } else { dados.read_unaligned() };
    if contexto != 0 && !deslocamentos.is_null() && !garment_offsets_enabled() {
        let recursos = ((contexto + CONTEXTO_RECURSOS) as *const u64).read_unaligned() as *const u64;
        let n = if recursos.is_null() { 0 } else { ((contexto + CONTEXTO_RECURSOS + 0x0c) as *const u32).read_unaligned() as usize };
        let lista: &[u64] = if n == 0 { &[] } else { std::slice::from_raw_parts(recursos, n) };
        es20_aplicar_deslocamentos(0, deslocamentos, lista, false);
    }
    let f: unsafe extern "C" fn(u64, *const u64, *mut u8, *const u64, u64, u64, u64, u64) =
        std::mem::transmute(crate::ganchos::original_publicado(&ORIG_COMPUTAR_ROUPA));
    f(a1, entidade, deslocamentos, dados, a5, a6, a7, a8);
}

/// No `on_load`, sempre, depois dos hooks de teste do mesmo alvo, que fazem esta conta por conta própria.
pub(crate) unsafe fn install_offsets_de_roupa() {
    let r = crate::ganchos::instalar_conferindo_prologo(COMPUTAR_ROUPA_VM, COMPUTAR_ROUPA_PROLOGO, computar_roupa_hook as *mut c_void, &ORIG_COMPUTAR_ROUPA);
    crate::veredito_dev!("[roupa] hook do ComputePlayerGarment: {r:?}");
}

/// `EnableGarmentOffsets`/`DisableGarmentOffsets` (a flag do `ApplyOffsetOverrides`).
pub(crate) fn es20_deslocamentos_ligados() -> bool {
    garment_offsets_enabled()
}


/// Codeware `WardrobeSystemEx::ForgetItemID` (catálogo item #46/#208, agente de candidatos
/// baratos 2026-08-11, 3ª rodada da sessão) — fallback de SCAN LINEAR por `ItemID.tdbid`, a
/// mesma técnica que a fonte real usa quando a remoção direta por CName-derivado-de-flat falha
/// (`WardrobeSystemEx.hpp`: calcula `appearanceName` via `GetFlatValue<CName>({itemID.tdbid,
/// ".appearanceName"})`, tenta remover por essa chave, e se isso falhar varre `store.ForEach`
/// comparando `tdbid` e remove TODAS as entradas que baterem). **Divergência de escopo
/// consciente**: em vez de replicar as DUAS etapas (derivar CName via flat + fallback), uso só
/// o SCAN por tdbid como via ÚNICA e PRIMÁRIA — mais robusto (não depende do item ter um flat
/// `.appearanceName` populado no TweakDB) e reusa 100% a mecânica já provada em
/// `wardrobe_forget_item_locked` (mesmo `SharedSpinLock@+0xF4`, mesmo layout de `HashMap<CName,
/// ItemID>` confirmado ao vivo contra o save real do usuário, 246 itens, 2026-07-31). A única
/// peça nova é o CRITÉRIO de match: em vez de comparar a CHAVE (`node.key`, um CName que o
/// caller já precisa saber), compara o campo `tdbid` do VALOR (`node+0x10`) — `ItemID{tdbid:
/// TweakDBID@0x00(8 bytes), rngSeed@0x08, uniqueCounter@0x0C, structure@0x0E, flags@0x0F}`,
/// `RED4EXT_ASSERT_SIZE(ItemID, 0x10)` confirmado em `RED4ext.SDK/include/RED4ext/
/// NativeTypes.hpp:128` (ground-truth oficial vendorizado, não chute) — como `tdbid` é o
/// PRIMEIRO campo do struct, fica exatamente em `node+0x10` sem offset adicional. Varre TODOS
/// os buckets (não só o bucket do hash de 1 chave) e remove TODAS as entradas que baterem
/// (mesma semântica "remove todas" da fonte real) — devolve a CONTAGEM removida.
pub(crate) unsafe fn wardrobe_forget_by_tdbid(wardrobe_sys: *mut u8, target_tdbid: u64) -> u32 {
    if !crate::memoria::is_readable(wardrobe_sys as *const c_void, 0x100) {
        crate::log_dev!("[wardrobe-forget-tdbid] instância ilegível");
        return 0;
    }
    let hm = wardrobe_sys.add(0x48);
    let lock_atomic = &*(wardrobe_sys.add(0xF4) as *const std::sync::atomic::AtomicU8);
    let mut locked = false;
    // Protocolo do Mac (0x1000015f0): exclusiva = CAS 0→0x80, solta limpando o bit 7. O 0xFF/store 0 do Windows deixa o +1 de um
    // leitor do motor zerar o byte e entrar junto, e o −1 dele prende a trava em 0xFF.
    for i in 0..4_000_000u32 {
        if lock_atomic.compare_exchange(0, 0x80, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            locked = true;
            break;
        }
        if i & 511 == 511 {
            std::thread::yield_now();
        }
    }
    if !locked {
        crate::log_dev!("[wardrobe-forget-tdbid] lock ocupado (spin esgotado), abortado sem mudar nada");
        return 0;
    }
    let result = wardrobe_forget_by_tdbid_locked(hm, target_tdbid);
    crate::jogo::travas::soltar_escrita(wardrobe_sys.add(0xF4));
    result
}

pub(crate) unsafe fn wardrobe_forget_by_tdbid_locked(hm: *mut u8, target_tdbid: u64) -> u32 {
    const STRIDE: usize = 0x20;
    const INVALID: u32 = 0xFFFF_FFFF;
    let index_table = (hm as *const u64).read_unaligned() as *mut u32;
    let size_cap = (hm.add(0x08) as *const u64).read_unaligned();
    let capacity = (size_cap >> 32) as u32;
    let nodes = (hm.add(0x10) as *const u64).read_unaligned() as *mut u8;
    if capacity == 0 || capacity > 4096 || index_table.is_null() || nodes.is_null() {
        crate::log_dev!("[wardrobe-forget-tdbid] capacity/ponteiros fora de faixa plausível, abortado");
        return 0;
    }
    let mut removed = 0u32;
    for bucket in 0..(capacity as usize) {
        let mut prev_next_field: *mut u32 = index_table.add(bucket);
        let mut idx = prev_next_field.read_unaligned();
        let mut guard = 0u32;
        while idx != INVALID && guard < capacity + 1 {
            guard += 1;
            let node = nodes.add(idx as usize * STRIDE);
            if !crate::memoria::is_readable(node as *const c_void, STRIDE) {
                crate::log_dev!("[wardrobe-forget-tdbid] node ilegível no meio da cadeia, parando com segurança");
                return removed;
            }
            let next = (node as *const u32).read_unaligned();
            let value_tdbid = (node.add(0x10) as *const u64).read_unaligned();
            if value_tdbid == target_tdbid {
                // desengancha da cadeia (mesma técnica de `wardrobe_forget_item_locked`)
                prev_next_field.write_unaligned(next);
                let nextidx_ptr = hm.add(0x20) as *mut u32;
                let old_freelist_head = nextidx_ptr.read_unaligned();
                (node as *mut u32).write_unaligned(old_freelist_head);
                nextidx_ptr.write_unaligned(idx);
                let size_ptr = hm.add(0x08) as *mut u32;
                let cur_size = size_ptr.read_unaligned();
                size_ptr.write_unaligned(cur_size.saturating_sub(1));
                removed += 1;
                crate::log_dev!("[wardrobe-forget-tdbid] removido: bucket={bucket} idx={idx} tdbid={target_tdbid:#018x} size {cur_size}->{}", cur_size.saturating_sub(1));
                // `prev_next_field` já foi atualizado pra pular `node` — segue a cadeia a
                // partir do `next` capturado ANTES da escrita, sem tocar `prev_next_field`.
                idx = next;
                continue;
            }
            prev_next_field = node as *mut u32; // node.next fica em +0x00, vira o "prev" da próxima iteração
            idx = next;
        }
    }
    if removed == 0 {
        crate::log_dev!("[wardrobe-forget-tdbid] tdbid={target_tdbid:#018x} não encontrado em nenhum bucket");
    }
    removed
}

// ===== axl-garment-apply/axl-transmog-apply (2026-08-03, /goal, cont.30): FIX CIRÚRGICO de
// `GetInvokable()` =====
// Achado desta madrugada (equiprawv5): chamar `0x102445e8c` DIRETO com um `CScriptStackFrame`
// à mão (mesma receita de `call_func`) roda SEM CRASH mas NÃO FAZ NADA — o bytecode sintético
// (`LocalVar`+`ParamEnd`) só faz o EXECUTOR interpretar "declarar 1 argumento", e como não há
// mais nenhuma instrução real depois disso (nosso buffer é só padding-zero = opcode 0 = ret),
// a função devolve sem nunca invocar a lógica nativa de enfileiramento — `0x102445e8c` só roda
// de verdade quando despachado via `crate::jogo::scripts::EXECUTOR_VM`, que primeiro chama `this->GetInvokable()` (RE
// já feita: `0x1008c5c9c`, stub que SEMPRE retorna null pra QueueRequest — bug confirmado desde
// cont.14 desta mesma madrugada). Fix cirúrgico: hookar `GetInvokable()` (função normal C-ABI,
// sem naked asm necessário) e, SÓ quando `this` for o descritor exato de `QueueRequest`,
// devolver o endereço nativo real (`0x102445e8c`) em vez de null — qualquer outro `this` cai
// no comportamento ORIGINAL (stub null), zero regressão pros outros ~3 usos dessa vtable slot.
pub(crate) static ORIG_GET_INVOKABLE: AtomicU64 = AtomicU64::new(0);

pub(crate) static GETINVOKABLE_TARGET_DESC: AtomicU64 = AtomicU64::new(0);

pub(crate) static GETINVOKABLE_NATIVE_STATIC: AtomicU64 = AtomicU64::new(0);

pub(crate) const ADDR_GET_INVOKABLE_VM: u64 = 0x1_008c_5c9c;

pub(crate) extern "C" fn get_invokable_shim(this: *mut c_void) -> *mut c_void {
    if this as u64 == GETINVOKABLE_TARGET_DESC.load(Ordering::Relaxed) {
        let na = GETINVOKABLE_NATIVE_STATIC.load(Ordering::Relaxed);
        if na != 0 {
            crate::log_dev!("[getinvokable-fix] this==QueueRequest descriptor — devolvendo endereço nativo real");
            return crate::rebase(na);
        }
    }
    let orig = ORIG_GET_INVOKABLE.load(Ordering::Relaxed) as *mut c_void;
    if orig.is_null() {
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void) -> *mut c_void = unsafe { std::mem::transmute(orig) };
    f(this)
}

/// Instala o fix (idempotente) e ARMA o alvo (`descriptor`=CClassFunction* de QueueRequest,
/// `native_static`=0x102445e8c). Chamar ANTES de invocar `rtti::call_func` normal sobre esse rf.
pub(crate) unsafe fn install_getinvokable_fix(descriptor: *mut c_void, native_static: u64) -> bool {
    GETINVOKABLE_TARGET_DESC.store(descriptor as u64, Ordering::Relaxed);
    GETINVOKABLE_NATIVE_STATIC.store(native_static, Ordering::Relaxed);
    if ORIG_GET_INVOKABLE.load(Ordering::Relaxed) != 0 {
        return true; // já instalado, só reapontamos o alvo acima
    }
    let t = crate::rebase(ADDR_GET_INVOKABLE_VM);
    if !crate::memoria::is_readable(t as *const c_void, 4) {
        crate::log_dev!("[getinvokable-fix] alvo ilegível");
        return false;
    }
    // O alvo tem 8 B (`mov x0,#0; ret`): o salto de 16 B do `instalar` cobria as 2 primeiras instruções da vizinha `0x1008c5ca4`, que tem
    // chamador direto (`0x1007d4e14`). `instalar_perto` grava 4 B; fora de ±128MB, `instalar_com_ponte` grava os 8 B da função inteira.
    let modo = if crate::ganchos::instalar_perto(t, get_invokable_shim as *mut c_void, Some(&ORIG_GET_INVOKABLE)).is_some() {
        "instalar_perto"
    } else if crate::ganchos::instalar_com_ponte(t, get_invokable_shim as *mut c_void, Some(&ORIG_GET_INVOKABLE)).is_some() {
        "instalar_com_ponte"
    } else {
        crate::log_dev!("[getinvokable-fix] recusado nos 2 métodos (near4 e adrp_br8)");
        return false;
    };
    crate::log_dev!("[getinvokable-fix] hook em 0x1008c5c9c instalado via {modo} (fix cirúrgico p/ QueueRequest)");
    true
}


/// ArchiveXL `#9`: o valor de 1 sufixo de aparência pro item e o dono, como o `GetSuffixValue` do motor (game thread).
pub(crate) unsafe fn a9_valor_do_sufixo(reg: &crate::rtti::Registry, dono_h: &[u64; 2], item: &[u8; 16], sufixo: u64) -> String {
    let s = bwms_primitives::tweak_db_id_chave(sufixo);
    let id = |n: &str| bwms_primitives::tweak_db_id(&format!("itemsFactoryAppearanceSuffix.{n}"));
    if s == id("Gender") {
        sufixo_gender(dono_h[0])
    } else if s == id("Partial") {
        sufixo_partial(dono_h, item)
    } else if s == id("Camera") {
        sufixo_camera(dono_h[0])
    } else {
        sufixo_generico(reg, dono_h, item, sufixo)
    }
}


/// Dá `qty` do item `name` no inventário do player, via `GiveItem`.
/// Retorna o buffer de resultado (res[0] = Bool de sucesso do GiveItem).
pub unsafe fn give(
    reg: &Registry,
    captured_player: *mut c_void,
    tx: *mut c_void,
    name: &str,
    qty: u32,
) -> Option<[u8; 0x20]> {
    let owner = {
        let a = auth_player(reg, captured_player);
        if a.is_null() {
            crate::log_dev!("[give] auth_player falhou; fallback pro player capturado");
            captured_player
        } else {
            a
        }
    };
    let gi = match rtti::resolve_func(reg, "gameTransactionSystem", "GiveItem") {
        Some(g) => g,
        None => {
            crate::log_dev!("[give] GiveItem NÃO resolvido");
            return None;
        }
    };
    crate::log_dev!(
        "[give] GiveItem func={:p} params={} static={}",
        gi.func,
        rtti::param_count(&gi),
        gi.is_static
    );
    let item = rtti::from_tdbid(reg, name)?;
    crate::log_dev!(
        "[give] ctx(tx)={tx:p} owner={owner:p} (capturado={captured_player:p}) qty={qty}"
    );
    let r = rtti::call_func(
        &gi,
        tx,
        &[Arg::Handle(owner, refcnt()), Arg::Item16(item), Arg::I32(qty)],
    );
    crate::log_dev!("[give] call_func -> {:02x?}", r);
    r
}

/// Remove `qty` do item `name` do inventário, via `RemoveItem` (espelho do give).
pub unsafe fn remove(
    reg: &Registry,
    captured_player: *mut c_void,
    tx: *mut c_void,
    name: &str,
    qty: u32,
) -> Option<[u8; 0x20]> {
    let owner = auth_or(reg, captured_player);
    let rf = rtti::resolve_func(reg, "gameTransactionSystem", "RemoveItem")?;
    let item = rtti::from_tdbid(reg, name)?;
    let r = rtti::call_func(
        &rf,
        tx,
        &[Arg::Handle(owner, refcnt()), Arg::Item16(item), Arg::I32(qty)],
    );
    crate::log_dev!("[remove] {name} x{qty} -> {:02x?}", r);
    r
}

