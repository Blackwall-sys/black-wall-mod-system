//! Salvamento: gravar e recriar entidades com o save, serializar objetos, estado de persistência.

#![allow(dead_code)]

pub(crate) mod entidades_persistentes;

use crate::inicio::natives::*;
use std::ffi::c_void;
use crate::rtti::{self, Registry};
use crate::register::*;

// Objeto que não deriva de `IScriptable` (recurso) não vai cru pro script: no Mac o `as`, o `IsA` e os métodos chamados num `ref` leem o tipo no `+0x30` do
// objeto (`IScriptable::GetType` inlinado no handler do cast `0x101202b54`), e num `CResource` esse campo é o hash do caminho. No boot 186 um
// `evt.GetResource() as BwmsSoundBanks` num recurso de verdade caiu no `IsA` (`0x1010c6bf4`) com o hash no lugar da classe. Ele vai dentro de um
// `BwmsSerializable` (`bwms-serializable.reds`), no campo privado `object`; o leitor de parâmetros das natives desembrulha.
pub(crate) const RECURSO_CLASSE_SCRIPT: &str = "BwmsSerializable";

pub(crate) const RECURSO_CAMPO_SCRIPT: &str = "object";

/// `BwmsEntidadesPersistentes() -> Int32` — quantas entidades persistentes a sessão
/// tem agora. Serve ao teste e a quem quiser conferir no jogo.
pub(crate) unsafe extern "C" fn tramp_entidades_persistentes(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, crate::salvamento::entidades_persistentes::quantas() as u64, 4);
}

/// `BwmsGravarEntidadesDoSave(save: String) -> Bool` — grava os specs no arquivo do
/// save dado, sem esperar o vigia. O teste usa pra fechar o ciclo na hora do save.
pub(crate) unsafe extern "C" fn tramp_gravar_entidades(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let ok = crate::salvamento::entidades_persistentes::gravar(&s);
    write_bool_ret(out, ok);
}

/// `BwmsRecriarEntidades() -> Int32` — recria agora as entidades do save corrente.
pub(crate) unsafe extern "C" fn tramp_recriar_entidades(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::salvamento::entidades_persistentes::esquecer_a_geracao();
    write_uint_ret(out, crate::salvamento::entidades_persistentes::recriar() as u64, 4);
}

/// `BwmsPersistentIDHash(entityHash: Uint64, componentHash: Uint64) -> Uint64` — expõe
/// `persistent_id_hash` pro redscript. Usada por `PersistentID.ToHash` (`ponte-casts.reds`)
/// pra corrigir o caso COMPONENT (antes só devolvia `entityHash`, correto só por acidente porque
/// `ForComponent` nunca existia até 2026-08-12 round 4 — nenhum `PersistentID` de componente
/// jamais foi hasheado por este projeto antes de hoje).
pub(crate) unsafe extern "C" fn tramp_persistentid_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let entity_hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let component_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let h = persistent_id_hash(entity_hash, component_hash);
    crate::log_dev!(
        "[persistentid] BwmsPersistentIDHash(entityHash={entity_hash:#x}, componentHash={component_hash:#x}) -> {h:#x}"
    );
    write_uint_ret(out, h, 8);
}

/// `BwmsObjectSave(obj, path) -> Bool` — ver `selftest::c252_gravar_arquivo`.
pub(crate) unsafe extern "C" fn tramp_object_save(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (args, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let obj = recurso_do_script(args.first().map_or(0, |(v, _)| *v));
    let rel = strings.get(1).cloned().flatten().unwrap_or_default();
    let r = crate::salvamento::c252_gravar_arquivo(obj, &rel);
    crate::log_dev!("[c252] BwmsObjectSave({rel}) = {r:?}");
    write_bool_ret(out, r.is_ok());
}

/// `BwmsObjectLoad(path) -> ref<IScriptable>` — a referência forte da instância lida passa pro retorno.
pub(crate) unsafe extern "C" fn tramp_object_load(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_args, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let rel = strings.first().cloned().flatten().unwrap_or_default();
    match crate::salvamento::c252_ler_arquivo(&rel) {
        Ok(mut h) => {
            // Objeto de classe que não deriva de `IScriptable` volta dentro de um `BwmsResource`.
            let mut s = recurso_para_script(h);
            crate::jogo::objetos::soltar_handle(&mut h);
            if out.is_null() {
                crate::jogo::objetos::soltar_handle(&mut s);
            } else {
                core::ptr::write_unaligned(out as *mut u64, s[0]);
                core::ptr::write_unaligned((out as *mut u64).add(1), s[1]);
            }
        }
        Err(e) => {
            crate::log_dev!("[c252] BwmsObjectLoad({rel}): {e}");
            write_handle_ret(out, 0);
        }
    }
}

pub(crate) unsafe fn register_object_serializer_natives(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &["handle:IScriptable", "String"], "BwmsObjectSave", "BwmsObjectSave", tramp_object_save as NativeHandler);
    let ok2 = register_argful_by_types(reg, proto, &["String"], "BwmsObjectLoad", "BwmsObjectLoad", tramp_object_load as NativeHandler);
    crate::log_dev!("[reg] register_all: cw-252 BwmsObjectSave={ok1} BwmsObjectLoad={ok2}");
}

// ---------------------------------------------------------------------------------------------
// Continuação direta do achado do systemMap
// (2026-08-14, bloco `BwmsGetSystemByName` acima): aquele achado destravou o PONTEIRO de
// `gamePersistencySystem`/`gamePopulationSystem`/`gameContainerManager`/`questWorldStateSystem`,
// mas `funclistdump` confirmou (2 dos 4 testados: `questWorldStateSystem`/`gamePopulationSystem`)
// que essas classes não expõem API própria pro redscript — mesma categoria de `MappinSystem`/
// `CommunitySystem` acima, precisam de dispatch por SLOT DE VTABLE (RE por classe), não de
// reflection. Esta rodada ataca o item de MENOR risco/MAIOR valor da lista: `IPersistencySystem`/
// `IPopulationSystem` (nomes de método REAIS, gerados da reflection do jogo — ao contrário de
// `IContainerManager`, cuja reflection NÃO lista nenhum método além de `IGameSystem`, zero offset disponível
// pra essa classe nesta rodada, descartada por falta de dado) — 2 GETTERS PUROS (sem mutação),
// mesmo shift Itanium Mac=Windows+0x08 já estabelecido:
//
//   IPersistencySystem::GetEntityStatus(EntityID) -> EntityPersistenceStatus   // Windows 0x1E8
//   IPopulationSystem::IsSpawning(EntityID) -> bool                            // Windows 0x1C8
//
// `EntityID` é `{hash: u64}` (já confirmado/usado por
// `entity_id_hash`/`#133`) — POD trivial, sem ctor/dtor não-trivial, passado POR VALOR em x1 na
// ABI Itanium (mesma convenção já usada pra `CommunityIdEntryFn` acima). `EntityPersistenceStatus`
// é `enum class : uint64_t` (NotPersisted=0/CanBePersisted=1/Persisted=2) — POD trivial, devolvido
// POR VALOR em x0 (mesmo princípio já usado pro retorno u32 de `MappinGetPoiPhaseFn`, aqui u64,
// nenhum dos dois precisa de sret).
//
// Achado paralelo, registrado honestamente mas NÃO usado nesta implementação: a classe CONCRETA
// `GamePersistencySystem` (script-visível, `redscript-src/orphans.script:16776`) já tem vários
// métodos VANILLA NATIVE reais expostos ao redscript (`EntityProxy_DisablePhysics`/
// `QueueEntityEvent`/`QueuePSEvent`/`GetConstAccessToPSObject`/`ForcePersistentStateChanged`), e
// `GameInstance.GetPersistencySystem(self)`/`GetContainerManager(self)`
// (`orphans.script:11431`/`11569`) já são natives VANILLA que devolvem o ponteiro sem precisar de
// `BwmsGetSystemByName` nem do systemMap — zero RE necessária pra OBTER esses 2 ponteiros
// especificamente (`gamePopulationSystem` não tem equivalente vanilla, segue precisando de
// `BwmsGetSystemByName`). `GetEntityStatus`/`IsSpawning` especificamente NÃO estão entre os
// métodos decompilados de `GamePersistencySystem` (só a interface abstrata `IGamePersistencySystem`,
// vazia) — capacidade genuinamente NOVA, não duplicando nada já vanilla-acessível. `sys` é aceito
// como `ref<IScriptable>` genérico (mesmo padrão já provado de `tramp_callmethod`/`BwmsCallMethod`)
// — o mod passa o handle devolvido por `GameInstance.GetPersistencySystem`/`BwmsGetSystemByName`,
// sem precisar de um tipo `handle:GamePersistencySystem`/`handle:PopulationSystem` nunca testado
// neste projeto.
pub(crate) const PERSISTENCY_GET_ENTITY_STATUS_SLOT: usize = 0x1E8 + 0x08; // = 0x1F0

pub(crate) type PersistencyGetEntityStatusFn = unsafe extern "C" fn(*mut c_void, u64) -> u64;

/// `BwmsPersistencySystemGetEntityStatus(sys: ref<IScriptable>, entityId: EntityID) -> Int32` —
/// chama `IPersistencySystem::GetEntityStatus(EntityID)` via slot de vtable Mac `0x1F0`. SÓ
/// LEITURA (getter puro, sem mutação) — seguro pra smoke test automático mesmo com `EntityID`
/// fabricado (o pior caso é devolver `NotPersisted`/0, nunca muta nada). Retorno: 0=NotPersisted,
/// 1=CanBePersisted, 2=Persisted (valores do enum C++ real, sem equivalente RTTI redscript —
/// mesma divergência já documentada pro `gamedataMappinPhase` acima).
pub(crate) unsafe extern "C" fn tramp_persistency_get_entity_status(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let entity_id = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    // O slot `0x1F0` não é o GetEntityStatus: com o sistema certo e o mesmo id, devolveu valores diferentes em faixa de ponteiro em 2 boots
    // (`npcs/mod.rs`, doc do `BwmsPopulationSystemIsSpawning`). Chamar função virtual não provada com o que o script passar pode mutar estado
    // ou derrubar o jogo; até um disassembly da vtable do `gamePersistencySystem` provar o slot, a resposta é NotPersisted (0).
    let status: u64 = 0;
    crate::log_dev!(
        "[persistencysystem] BwmsPersistencySystemGetEntityStatus(sys={sys:p}, entityId={entity_id:#x}) -> {status} (slot {PERSISTENCY_GET_ENTITY_STATUS_SLOT:#x} não provado: não chamado)"
    );
    write_uint_ret(out, status, 4);
}


pub(crate) unsafe fn register_entidades_persistentes(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &["String"], "BwmsSaveCarregado", "BwmsSaveCarregado", tramp_save_carregado as NativeHandler);
    let ok2 = register_global(reg, proto, "BwmsEntidadesPersistentes", "BwmsEntidadesPersistentes", tramp_entidades_persistentes);
    let ok3 = register_argful_by_types(reg, proto, &["String"], "BwmsGravarEntidadesDoSave", "BwmsGravarEntidadesDoSave", tramp_gravar_entidades as NativeHandler);
    let ok4 = register_global(reg, proto, "BwmsRecriarEntidades", "BwmsRecriarEntidades", tramp_recriar_entidades);
    crate::log_dev!("[reg] register_all: cw-153 SaveCarregado={ok1} Quantas={ok2} Gravar={ok3} Recriar={ok4}");
}

/// Núcleo PURO de `game::PersistentID::GetHash()` (gerado da reflection do jogo — confiança MÁXIMA,
/// mesma categoria já usada pra fechar o `#132`/ComponentTarget): algoritmo REAL de hash do
/// motor, nunca replicado antes neste projeto (item `#31` do catálogo — a nota antiga
/// "não replica o algoritmo interno real de hash do PersistentID, que nunca foi RE'd" fica
/// desatualizada com este achado). Caso ENTITY-only (`componentName` == CName "None", hash==0
/// nesta convenção — mesmo sentinela já usado pra `EInputKey.IK_None`/`#136`): devolve
/// `entityHash` puro, IDÊNTICO ao comportamento já provado. Caso COMPONENT (`componentName!=0`):
/// mistura os 2 hashes via um passo de MurmurHash64A (`magic=0xC6A4A7935BD1E995`), fielmente
/// traduzido byte-a-byte do C++ real (`e ^ (magic * ((magic*c) ^ ((magic*c)>>47)))`, depois
/// `magic * isso`). Zero endereço nativo, zero RTTI/ponteiro — só aritmética de 64 bits
/// (`wrapping_mul`/`wrapping_add` = overflow C++ unsigned, testável offline sem boot).
pub(crate) fn persistent_id_hash(entity_hash: u64, component_hash: u64) -> u64 {
    const MAGIC: u64 = 0xC6A4A7935BD1E995;
    if component_hash == 0 {
        return entity_hash;
    }
    let h1 = MAGIC.wrapping_mul(component_hash);
    let h2 = h1 ^ (h1 >> 47);
    let h3 = MAGIC.wrapping_mul(h2);
    let h4 = entity_hash ^ h3;
    MAGIC.wrapping_mul(h4)
}


// ===== `ObjectSerializer::SaveToFile`/`ReadFromFile` (2026-09-14) =====
// No Mac o par está em `0x1048852d8` (`x0` = handle, `w1` = flags, `w2` = limite, `x8` = `DataBuffer`:
// escritor de pacote `0x101085f28`, prepare `0x101086858` com flags e limite dos argumentos, add `0x101086ae8`, finalize `0x101086ed8`) e `0x10488518c`
// (`x0` = stream, `x1` = classe, `x8` = handle: lê o `DataBuffer` do stream, leitor padrão, cabeçalho, instância pela classe e leitura do objeto). A gravação
// chama o `0x1048852d8` com o limite como argumento, sem hook. A leitura faz o miolo do `0x10488518c` sobre os bytes do arquivo, sem `BaseStream`: leitor
// `0x101074b2c` com a vtable do par (`0x107c76720`), buffer `0x101074b44`, cabeçalho `0x101074b50`, leitor de pacote `0x101089610`/`0x10108986c`, 1 objeto raiz,
// `CreateInstance` `0x1010c36a0` com o tamanho da classe, handle `0x101025c8c`, leitura `0x101089b1c`. O arquivo do BWMS guarda o nome da classe.
pub(crate) const C252_GRAVAR_VM: u64 = 0x1_0488_52d8;

pub(crate) const C252_LEITOR_BASE_VM: u64 = 0x1_0107_4b2c;

pub(crate) const C252_LEITOR_VTABLE_VM: u64 = 0x1_07c7_6720;

pub(crate) const C252_LEITOR_CABECALHO_VM: u64 = 0x1_0107_4b50;

pub(crate) const C252_LEITOR_SOLTAR_VM: u64 = 0x1_0107_4b40;

pub(crate) const C252_PACOTE_CRIAR_VM: u64 = 0x1_0108_9610;

pub(crate) const C252_PACOTE_ABRIR_VM: u64 = 0x1_0108_986c;

pub(crate) const C252_PACOTE_LER_VM: u64 = 0x1_0108_9b1c;

pub(crate) const C252_PACOTE_SOLTAR_VM: u64 = 0x1_0108_972c;

pub(crate) const C252_CLASSE_TAMANHO_VM: u64 = 0x1_010c_31d4;

pub(crate) const C252_DATABUFFER_SOLTAR_VM: u64 = 0x1_0121_2c20;

/// Limite passado ao prepare do escritor; os chamadores do motor passam 64 KB e 256 KB.
pub(crate) const C252_LIMITE: u32 = 0x0100_0000;

pub(crate) const C252_MAGIA: &[u8; 4] = b"BWOB";

pub(crate) const C252_VERSAO: u32 = 1;

pub(crate) const C252_PASTA: &str = "r6/storage/bwms/objetos";

/// Arquivo de objeto do BWMS: `BWOB`, versão (u32), nome da classe (u16 + UTF-8), tamanho do pacote (u32) e os bytes do pacote.
pub(crate) fn c252_montar_arquivo(classe: &str, pacote: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(14 + classe.len() + pacote.len());
    v.extend_from_slice(C252_MAGIA);
    v.extend_from_slice(&C252_VERSAO.to_le_bytes());
    v.extend_from_slice(&(classe.len() as u16).to_le_bytes());
    v.extend_from_slice(classe.as_bytes());
    v.extend_from_slice(&(pacote.len() as u32).to_le_bytes());
    v.extend_from_slice(pacote);
    v
}

/// `(classe, pacote)` de um arquivo do `c252_montar_arquivo`; `None` com magia, versão ou tamanhos que não batem.
pub(crate) fn c252_abrir_arquivo(dados: &[u8]) -> Option<(&str, &[u8])> {
    let resto = dados.strip_prefix(C252_MAGIA)?;
    let (versao, resto) = resto.split_first_chunk::<4>()?;
    if u32::from_le_bytes(*versao) != C252_VERSAO {
        return None;
    }
    let (n, resto) = resto.split_first_chunk::<2>()?;
    let n = u16::from_le_bytes(*n) as usize;
    if resto.len() < n {
        return None;
    }
    let (classe, resto) = resto.split_at(n);
    let classe = std::str::from_utf8(classe).ok().filter(|c| !c.is_empty())?;
    let (tamanho, pacote) = resto.split_first_chunk::<4>()?;
    (u32::from_le_bytes(*tamanho) as usize == pacote.len() && !pacote.is_empty()).then_some((classe, pacote))
}

/// Caminho aceito pelo `BwmsObjectSave`/`BwmsObjectLoad`: partes não vazias de letras, dígitos, `_`, `-` e `.` separadas por `/`, sem `.` nem `..`.
pub(crate) fn c252_caminho_valido(rel: &str) -> bool {
    !rel.is_empty()
        && rel.len() <= 200
        && rel.split('/').all(|p| !p.is_empty() && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
}

pub(crate) unsafe fn c252_caminho(rel: &str) -> Result<std::path::PathBuf, String> {
    if !c252_caminho_valido(rel) {
        return Err(format!("caminho inválido: {rel:?}"));
    }
    let jogo = crate::arquivos_do_jogo::game_dir_from_dylib().ok_or("pasta do jogo não encontrada")?;
    Ok(std::path::Path::new(&jogo).join(C252_PASTA).join(rel))
}

/// Bytes do pacote do motor com `obj` (instância viva); `None` se o motor devolveu buffer vazio.
pub(crate) unsafe fn c252_serializar(obj: u64) -> Option<Vec<u8>> {
    let mut h = [0u64; 2];
    crate::rtti::make_handle(h.as_mut_ptr() as *mut c_void, obj as *mut c_void);
    let mut buf = [0u8; 0x40];
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(C252_GRAVAR_VM) as u64,
        in("x0") h.as_mut_ptr() as u64,
        in("x1") u32::MAX as u64,
        in("x2") C252_LIMITE as u64,
        in("x8") buf.as_mut_ptr() as u64,
        clobber_abi("C"),
    );
    let dados = (buf.as_ptr() as *const u64).read_unaligned();
    let tamanho = (buf.as_ptr().add(8) as *const u32).read_unaligned() as usize;
    let bytes = (dados != 0 && tamanho > 0).then(|| std::slice::from_raw_parts(dados as *const u8, tamanho).to_vec());
    let soltar: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(C252_DATABUFFER_SOLTAR_VM));
    soltar(buf.as_mut_ptr());
    crate::jogo::objetos::soltar_handle(&mut h);
    bytes
}

/// Instância nova de `cls` lida de `pacote`, como handle forte; `None` sem exatamente 1 objeto raiz ou sem instância.
pub(crate) unsafe fn c252_desserializar(cls: *mut c_void, pacote: &[u8]) -> Option<[u64; 2]> {
    if cls.is_null() || pacote.is_empty() || pacote.len() > u32::MAX as usize {
        return None;
    }
    let mut leitor = [0u8; 0x40];
    let mut cabecalho = [0u8; 0x80];
    let mut leitor_pacote = [0u8; 0x100];
    let base: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(C252_LEITOR_BASE_VM));
    let buffer: unsafe extern "C" fn(*mut u8, u64, u64) = std::mem::transmute(crate::rebase(crate::jogo::recursos::LEITOR_BUFFER_VM));
    let soltar_leitor: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(C252_LEITOR_SOLTAR_VM));
    let criar_pacote: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(C252_PACOTE_CRIAR_VM));
    let abrir_pacote: unsafe extern "C" fn(*mut u8, *mut u8) = std::mem::transmute(crate::rebase(C252_PACOTE_ABRIR_VM));
    let soltar_pacote: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(C252_PACOTE_SOLTAR_VM));
    criar_pacote(leitor_pacote.as_mut_ptr());
    base(leitor.as_mut_ptr());
    (leitor.as_mut_ptr() as *mut u64).write_unaligned(crate::rebase(C252_LEITOR_VTABLE_VM) as u64);
    buffer(leitor.as_mut_ptr(), pacote.as_ptr() as u64, pacote.len() as u64);
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(C252_LEITOR_CABECALHO_VM) as u64,
        in("x0") leitor.as_mut_ptr() as u64,
        in("x8") cabecalho.as_mut_ptr() as u64,
        clobber_abi("C"),
    );
    abrir_pacote(leitor_pacote.as_mut_ptr(), cabecalho.as_mut_ptr());
    let ini = (cabecalho.as_ptr().add(0x08) as *const u64).read_unaligned();
    let fim = (cabecalho.as_ptr().add(0x10) as *const u64).read_unaligned();
    let mut h = [0u64; 2];
    if fim.wrapping_sub(ini) & 0x7_ffff_fff8 == 8 {
        let tamanho: unsafe extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(crate::rebase(C252_CLASSE_TAMANHO_VM));
        let criar: unsafe extern "C" fn(*mut c_void, u64, u32) -> *mut c_void = std::mem::transmute(crate::rebase(crate::jogo::objetos::OBJETO_CRIAR_VM));
        let obj = criar(cls, tamanho(cls) as u64, 0);
        if !obj.is_null() {
            crate::rtti::make_handle(h.as_mut_ptr() as *mut c_void, obj);
            let ler: unsafe extern "C" fn(*mut u8, u64, u32) = std::mem::transmute(crate::rebase(C252_PACOTE_LER_VM));
            ler(leitor_pacote.as_mut_ptr(), h[0], 0);
        }
    }
    soltar_leitor(leitor.as_mut_ptr());
    soltar_pacote(leitor_pacote.as_mut_ptr());
    (h[0] != 0).then_some(h)
}

/// `BwmsObjectSave`: grava `obj` em `<jogo>/r6/storage/bwms/objetos/<rel>`; devolve o tamanho do pacote.
pub(crate) unsafe fn c252_gravar_arquivo(obj: u64, rel: &str) -> Result<usize, String> {
    let caminho = c252_caminho(rel)?;
    let cls = crate::rtti::class_of(obj as *mut c_void);
    if cls.is_null() {
        return Err("objeto nulo ou ilegível".into());
    }
    let Some(reg) = crate::rtti::Registry::obtain() else { return Err("sem RTTI".into()) };
    let classe = crate::cname::resolve_cname(crate::rtti::type_name_hash(cls));
    if reg.class_by_name(&classe) != cls {
        return Err(format!("a classe {cls:p} ({classe:?}) não volta pelo nome"));
    }
    let pacote = c252_serializar(obj).ok_or_else(|| format!("o motor não gravou o {classe}"))?;
    if let Some(pasta) = caminho.parent() {
        std::fs::create_dir_all(pasta).map_err(|e| format!("{}: {e}", pasta.display()))?;
    }
    std::fs::write(&caminho, c252_montar_arquivo(&classe, &pacote)).map_err(|e| format!("{}: {e}", caminho.display()))?;
    Ok(pacote.len())
}

/// `BwmsObjectLoad`: lê um arquivo do `BwmsObjectSave` e devolve o handle forte da instância nova.
pub(crate) unsafe fn c252_ler_arquivo(rel: &str) -> Result<[u64; 2], String> {
    let caminho = c252_caminho(rel)?;
    let dados = std::fs::read(&caminho).map_err(|e| format!("{}: {e}", caminho.display()))?;
    let (classe, pacote) = c252_abrir_arquivo(&dados).ok_or("arquivo sem cabeçalho BWOB válido")?;
    let Some(reg) = crate::rtti::Registry::obtain() else { return Err("sem RTTI".into()) };
    let cls = reg.class_by_name(classe);
    if cls.is_null() {
        return Err(format!("a classe {classe} não existe"));
    }
    c252_desserializar(cls, pacote).ok_or_else(|| format!("o motor não leu o {classe}"))
}

