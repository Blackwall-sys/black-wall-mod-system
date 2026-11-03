//! TweakDB RUNTIME — registro de record NOVO no TweakDB VIVO (não no .bin).
//!
//! Descoberta (workflow tweakdb-runtime-record-reg, disasm-verificado): apendar um
//! record no tweakdb.bin NÃO funciona porque o `GiveItem` resolve TweakDBID lendo um
//! **records-HashMap em RAM**, populado 1x no `LoadOptimized`. Record apendado nunca
//! entra nesse mapa. Editar flat de record EXISTENTE funciona (o valor mora no
//! flatDataBuffer que o record já aponta). Para criar record novo: chamar a nativa
//! `CreateRecord(TweakDB*, u32 typeHash, TweakDBID)` no singleton vivo — a MESMA fn
//! que o loader chama por record (insere a Handle no records-map). É "registrar =
//! inverso de resolver", igual ao register.rs (RegisterFunction no RTTI).

use core::ffi::c_void;
use crate::rtti::rd_u64;

pub(crate) mod buffer;
pub(crate) mod escritor;
pub(crate) mod registro;
pub(crate) mod remover;
pub(crate) mod stats;
/// As natives que os scripts chamam para ler e mexer na TweakDB.
pub(crate) mod natives;
#[cfg(feature = "devtools")] // teste ao vivo, só no dev
pub(crate) mod teste;
use std::sync::atomic::{AtomicBool, Ordering};

// Endereços CONFIRMADOS por disasm (vmaddr, base 0x1_0000_0000):
const TDB_SINGLETON_GLOBAL: u64 = 0x1_08c9_c640; // global *TweakDB
// Workflow #2 (disasm-provado): registro de record NOVO em runtime.
const ADDR_CREATE_RECORD: u64 = 0x1_0330_9cc4; // CreateRecord(TweakDB* x0, u32 typeHash w1, TweakDBID x2) — aloca + insere Handle no +0x58
const ADDR_RECORD_EXISTS: u64 = 0x1_037c_eebc; // RecordExists(TweakDB* x0, TweakDBID x1) -> bool w0 (probe do +0x58)

/// `RecordExists` do motor ([`ADDR_RECORD_EXISTS`]): 1 se o record já está no mapa de records.
#[inline(always)]
unsafe fn registro_existe(tdb: *mut c_void, id: u64) -> u8 {
    let f: extern "C" fn(*mut c_void, u64) -> u8 = std::mem::transmute(crate::rebase(ADDR_RECORD_EXISTS));
    f(tdb, id)
}

/// `CreateRecord` do motor ([`ADDR_CREATE_RECORD`]): aloca e insere o handle do record novo no mapa.
#[inline(always)]
unsafe fn criar_registro(tdb: *mut c_void, tipo: u32, id: u64) {
    let f: extern "C" fn(*mut c_void, u32, u64) = std::mem::transmute(crate::rebase(ADDR_CREATE_RECORD));
    f(tdb, tipo, id)
}

// ===== hashes — FONTE ÚNICA no crate `bwms-primitives` (a MESMA impl do tweakdb-tool offline).
// `tweak_db_id` = CRC32(nome)|(len<<32); `record_type_key` =
// murmur3(miolo de gamedata(.*)_Record, seed 0x5EEDBA5E). =====
pub use bwms_primitives::{record_type_key, tweak_db_id, tweak_db_id_chave, tweak_db_id_derive};

static CREATED: AtomicBool = AtomicBool::new(false);

/// M1: registra um record NOVO no TweakDB VIVO chamando a nativa CreateRecord.
/// Valida com RecordExists antes (deve ser false — id novo) e depois (deve virar
/// true) => prova que o jogo passa a RESOLVER o record (o que o bake no .bin NÃO faz).
pub unsafe fn create_record_rt(class_name: &str, new_name: &str) {
    let t = match singleton() {
        Some(s) => s as *mut c_void,
        None => {
            crate::log("[tdb] create: singleton indisponível");
            return;
        }
    };
    // GATE: só prossegue se o TweakDB está REALMENTE carregado (+0x38==1). O cp77_tick
    // dispara cedo (scripts de boot) e chamar as nativas num TweakDB meio-carregado crasha.
    let loaded = crate::memoria::is_readable((t as *const u8).add(0x38) as *const c_void, 1)
        && *(t as *const u8).add(0x38) == 1;
    if !loaded {
        return; // tenta no próximo tick
    }
    let id = tweak_db_id(new_name);
    let type_hash = record_type_key(class_name);
    crate::log(&format!(
        "[tdb] create: entrando — singleton={t:p} loaded=1 id={id:#x} typeHash={type_hash:#010x}"
    ));
    crate::log("[tdb] create: chamando RecordExists(before)...");
    let before = registro_existe(t, id);
    crate::log(&format!("[tdb] create: RecordExists(before)={before}"));
    if before != 0 {
        crate::log(&format!(
            "[tdb] create: '{new_name}' (id={id:#x}) JÁ existe — abortando (CreateRecord aborta se existe)"
        ));
        return;
    }
    crate::log("[tdb] create: chamando CreateRecord...");
    criar_registro(t, type_hash, id);
    crate::log("[tdb] create: CreateRecord retornou; validando...");
    let after = registro_existe(t, id);
    crate::log(&format!(
        "[tdb] CreateRecord('{class_name}' typeHash={type_hash:#010x}, '{new_name}' id={id:#x}) -> RecordExists antes={before} depois={after} {}",
        if after != 0 { "✓✓ RECORD REGISTRADO EM RUNTIME (jogo resolve)" } else { "✗ falhou" }
    ));
}

/// `TweakDBManager.RegisterEnum(id: TweakDBID)` — espelho mínimo do `TweakDBReflection::RegisterEnum`
/// real (fonte vendorizada `enablers/TweakXL/.../ScriptManager.cpp:116-122`: `s_manager->
/// RegisterEnum(aRecordID)`, bookkeeping puro, sem side-effect visível fora da própria reflection).
/// BWMS não tem consumidor downstream desse registro ainda (nenhum sistema de reflection de TweakDB
/// próprio que precise saber "este ID é um enum") — mantemos um espelho simples (HashSet) só pra
/// que a chamada não seja no-op silencioso e para servir de base se um consumidor real aparecer.
static REGISTERED_ENUMS: std::sync::Mutex<Option<std::collections::HashSet<u64>>> = std::sync::Mutex::new(None);

pub fn register_enum_id(id: u64) -> bool {
    if let Ok(mut g) = REGISTERED_ENUMS.lock() {
        g.get_or_insert_with(std::collections::HashSet::new).insert(id);
        true
    } else {
        false
    }
}

/// Núcleo de `CreateRecord` por ID+typeHash JÁ RESOLVIDOS (sem derivar de string) — usado pela
/// API redscript `TweakDBManager.CreateRecord(id: TweakDBID, type: CName)`. Devolve `true` só se
/// `RecordExists` confirmar depois — mesma dupla validação (antes=false/depois=true) de
/// `create_record_rt`, mas SEM logging verboso a cada chamada (uso em produção, não prova pontual)
/// e sem depender de string alguma (o `type_hash` chega pronto, já derivado de um CName resolvido
/// pelo chamador via `crate::cname::resolve_cname` — ver `tramp_tdbm_createrecord`).
pub unsafe fn create_record_by_id(type_hash: u32, id: u64) -> bool {
    let t = match singleton() {
        Some(s) => s as *mut c_void,
        None => return false,
    };
    // Mesmo GATE de `create_record_rt`: só prossegue com o TweakDB REALMENTE carregado.
    let loaded = crate::memoria::is_readable((t as *const u8).add(0x38) as *const c_void, 1)
        && *(t as *const u8).add(0x38) == 1;
    if !loaded {
        return false;
    }
    if registro_existe(t, id) != 0 {
        return false; // já existe — CreateRecord real também aborta nesse caso
    }
    // O `CreateRecord` toma a trava do mapa de records (`+0x21`, `0x1037cd264`) exclusiva e gira até o byte zerar, sem teto. Byte não zero
    // aqui = leitor (1–0x7f) ou escritor (bit 0x80) dentro: é a linha que fica no veredito se a thread de script parar lá dentro.
    #[cfg(feature = "devlog")]
    {
        static TRAVA_OCUPADA_LOGADA: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let trava = (*((t as *const u8).add(0x21) as *const std::sync::atomic::AtomicU8)).load(Ordering::Acquire);
        if trava != 0 && TRAVA_OCUPADA_LOGADA.fetch_add(1, Ordering::Relaxed) < 5 {
            crate::veredito_dev!(
                "[tdbmanager] VEREDITO-TRAVA-RECORDS +0x21={trava:#04x} antes do CreateRecord(id={id:#x}) em t={}s, ordem {:?}",
                crate::inicio::boot::boot_elapsed_secs(),
                crate::inicio::natives::ordem_no_menu()
            );
        }
    }
    criar_registro(t, type_hash, id);
    let ok = registro_existe(t, id) != 0;
    crate::log_dev!("[tdbmanager] CreateRecord(id={id:#x}, typeHash={type_hash:#010x}) -> {ok}");
    ok
}

/// `RecordExists` do motor (mapa por id, `TweakDB+0x58`).
pub(crate) unsafe fn record_existe(id: u64) -> bool {
    let Some(t) = singleton() else { return false };
    registro_existe(t as *mut c_void, id) != 0
}

/// `resolve_getter_flat` por ID (sem precisar do nome-string do record source) — usa
/// `tweak_db_id_derive` (`bwms-primitives`, PROVADO por teste: `derive(id("base"),".suf") ==
/// id("base.suf")`) em vez de `tweak_db_id(format!("{record}.{name}"))`. Mesmo algoritmo de
/// `resolve_getter_flat`, só a fonte do id de busca muda.
unsafe fn resolve_getter_flat_by_id(t: *mut u8, source_id: u64, getter: &str) -> Option<(String, *mut u64)> {
    for name in [lower_first(getter), getter.to_string()] {
        if name.is_empty() {
            continue;
        }
        let flat_id = bwms_primitives::tweak_db_id_derive(source_id, &format!(".{name}"));
        if let Some(ep) = find_flat_entry_by_id(t, flat_id) {
            return Some((name, ep));
        }
    }
    None
}

/// Os sufixos `.prop` dos TweakDBID que o código de script usa (`TweakDBInterface.GetBool(id + t".isInState")`), do pool `tweakdb` do
/// `final.redscripts` do jogo, lidos uma vez: os flats extras que a classe do record não declara e o clone leva junto (o `ExtraFlats` do
/// TweakXL; 524 no blob vanilla da 2.5).
fn nomes_de_flat_dos_scripts() -> &'static [String] {
    static NOMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    NOMES.get_or_init(|| {
        let Some(jogo) = crate::arquivos_do_jogo::game_dir_from_dylib() else { return Vec::new() };
        let Ok(blob) = crate::scripts::blob::ler(&std::path::Path::new(&jogo).join("r6/cache/final.redscripts")) else { return Vec::new() };
        let mut nomes: Vec<String> = blob
            .tweakdb
            .into_iter()
            .filter(|n| n.strip_prefix('.').is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')))
            .collect();
        nomes.sort();
        nomes.dedup();
        crate::log_dev!("[tdbmanager] {} nome(s) de flat no pool tweakdb dos scripts", nomes.len());
        nomes
    })
}

/// `CloneRecord(id: TweakDBID, base: TweakDBID) -> Bool` — item TweakXL #44/#45 (`PENDENCIAS-
/// UNIFICADAS.md`): a via redscript real, só com IDs (sem nome nenhum, ao contrário de
/// `clone_record_api`/`inherit_flats_rt`, que exigem `class_name`/`source`/`clone` como string).
/// Deriva TUDO do `base` já vivo (mesma técnica de `update_record_by_id`: `get_record_instance` +
/// `class_of` + `type_name_hash` + `resolve_cname` pro nome da classe/type_hash) e usa
/// `tweak_db_id_derive` (em vez de concatenar string + rehash) pra computar os novos flat-ids do
/// clone — a peça que faltava pra generalizar `inherit_flats_rt` sem nome. Mesmo algoritmo de
/// merge/grow sob `mutex00` já provado lá, adaptado pra fonte de id.
pub unsafe fn clone_record_by_id(reg: &crate::rtti::Registry, id: u64, base: u64) -> bool {
    let t = match singleton() {
        Some(s) => s,
        None => return false,
    };
    let base_existing = get_record_instance(t, base);
    if base_existing.is_null() || !crate::memoria::is_readable(base_existing as *const c_void, 0x40) {
        crate::log_dev!("[tdbmanager] CloneRecord(id={id:#x}, base={base:#x}): base não achado");
        return false;
    }
    let native_type = crate::rtti::class_of(base_existing as *mut c_void);
    let name_hash = crate::rtti::type_name_hash(native_type);
    let class_name = crate::cname::resolve_cname(name_hash);
    if class_name.is_empty() {
        crate::log_dev!("[tdbmanager] CloneRecord(id={id:#x}, base={base:#x}): não resolveu o nome do tipo");
        return false;
    }
    let type_hash = record_type_key(&class_name);
    // Record que já existe (mod recarregado ou redeclarando o mesmo nome): não recria; os flats herdados voltam a apontar pro base atual.
    let ja_existe = !get_record_instance(t, id).is_null();
    // 1) cria o record NOVO (vazio) do mesmo tipo do base.
    if !ja_existe && !create_record_by_id(type_hash, id) {
        crate::log_dev!("[tdbmanager] CloneRecord(id={id:#x}, base={base:#x}): CreateRecord('{class_name}') falhou");
        return false;
    }
    // 2) herda os flats do base (mesmo algoritmo de `inherit_flats_rt`, ids em vez de nomes).
    let cls = reg.class_by_name(&class_name);
    if cls.is_null() {
        crate::log_dev!("[tdbmanager] CloneRecord(id={id:#x}): classe '{class_name}' não achada no registry (record criado, flats NÃO herdados)");
        return true; // CreateRecord já teve sucesso; herança de flat é best-effort
    }
    let (entries, size0, cap, _sp) = flats_header(t);
    if entries.is_null() || size0 == 0 || size0 > 10_000_000 {
        crate::log_dev!("[tdbmanager] CloneRecord(id={id:#x}): array de flats inválido (record criado, flats NÃO herdados)");
        return true;
    }
    let getters = enum_record_getters(cls);
    let mut news: Vec<u64> = Vec::new();
    let mut herdar = |sufixo: &str, ep: *mut u64| {
        let src_entry = ep.read_unaligned();
        let new_id = tweak_db_id_derive(id, sufixo);
        registro::herdou(id, new_id, tweak_db_id_chave(src_entry));
        if ja_existe && find_flat_entry_by_id(t, new_id).is_some() {
            escritor::reapontar(new_id, entry_tdb_offset(src_entry));
        }
        news.push(tweak_db_id_chave(new_id) | (src_entry & 0xFFFF_FF00_0000_0000));
    };
    let mut declarados = std::collections::HashSet::new();
    for g in &getters {
        if let Some((pname, ep)) = resolve_getter_flat_by_id(t, base, g) {
            let sufixo = format!(".{pname}");
            herdar(&sufixo, ep);
            declarados.insert(sufixo);
        }
    }
    // Flats extras da base (os que a classe não declara e o script lê por nome: `IPrereq.isInState`, `Effector.animFeatureName`).
    for sufixo in nomes_de_flat_dos_scripts().iter().filter(|s| !declarados.contains(*s)) {
        if let Some(ep) = find_flat_entry_by_id(t, tweak_db_id_derive(base, sufixo)) {
            herdar(sufixo, ep);
        }
    }
    if news.is_empty() {
        crate::log_dev!("[tdbmanager] CloneRecord(id={id:#x}, base={base:#x}): nenhum flat do base achado — record criado sem herança");
        return true;
    }
    news.sort_by(|a, b| match (id_key_lt(*a, *b), id_key_lt(*b, *a)) {
        (true, _) => std::cmp::Ordering::Less,
        (_, true) => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    });
    news.dedup_by(|a, b| tweak_db_id_chave(*a) == tweak_db_id_chave(*b));
    news.retain(|&e| !flat_key_present(entries, size0, e));
    let add = news.len();
    if add == 0 {
        crate::log_dev!("[tdbmanager] CloneRecord(id={id:#x}): flats já presentes — nada a herdar");
        return true;
    }
    // Falha na herança não desfaz o record: CreateRecord já teve sucesso, a herança é best-effort.
    match inserir_no_indice(t, (entries, size0, cap), &news) {
        Ok(Some(new_cap)) => crate::log_dev!(
            "[tdbmanager] CloneRecord(id={id:#x}, base={base:#x}, type='{class_name}'): GROW cap {cap}->{new_cap} + herdou {add} flats -> true"
        ),
        Ok(None) => crate::log_dev!("[tdbmanager] CloneRecord(id={id:#x}, base={base:#x}, type='{class_name}'): herdou {add} flats -> true"),
        Err(motivo) => crate::log_dev!("[tdbmanager] CloneRecord(id={id:#x}): {motivo}"),
    }
    true
}

/// Roda o registro UMA vez quando `~/.bwms-tdbcreate` existe (dev). Cria
/// Items.BwmsCloneTest como gamedataWeaponItem_Record (type_key 0x7fdef930, confirmado
/// válido). Chamado do cp77_tick em gameplay.
pub unsafe fn create_once_if_marked() {
    if CREATED.load(Ordering::Relaxed) {
        return;
    }
    let marked = crate::marcador_de_dev(".bwms-tdbcreate");
    if !marked {
        return;
    }
    // GATE (2026-08-01, achado no GOG): o marker+3s-fixos antigo dispara ANTES do que o
    // comentário original assumia — `create_once_if_marked` é chamado desde o 1º tick
    // (`lib.rs`, ANTES do gate de player), não só "em gameplay". No Steam isso nunca deu
    // problema porque 3s após qualquer tick já bastam pra o cache de tipo RTTI do record
    // (`gamedataWeaponItem_Record`) estar populado; no GOG, testado ao vivo, deu
    // `KERN_INVALID_ADDRESS at 0x0` — null-deref FUNDO no construtor RTTI do tipo (RE offline
    // confirmou a função-alvo/offset corretos, não é bug de endereço — ver DATABASE.md). Fix:
    // exige GAMEPLAY REAL (mesmo sinal robusto de "mundo assentado" já usado por
    // `tweakxl-updaterecord`/plugin `onUpdate`) antes de sequer agendar o disparo.
    if !crate::inicio::boot::PHASE_REACHED_5.load(Ordering::Relaxed) {
        return; // tenta de novo no próximo tick
    }
    if CREATED.swap(true, Ordering::Relaxed) {
        return;
    }
    // CRÍTICO: NÃO chamar CreateRecord aqui — cp77_tick roda DENTRO do hook do executor,
    // que pode estar segurando o spinlock do TweakDB (+0x21) → CreateRecord re-entra o lock
    // e DEADLOCKA a thread de script (provado: travou 18s+). O loader do jogo chama
    // CreateRecord de uma JOB THREAD, então é cross-thread-safe. Disparamos numa thread
    // separada (fora do hook) que pega o lock limpo.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(5)); // margem extra pós-phase5
        unsafe { create_record_rt("gamedataWeaponItem_Record", "Items.BwmsCloneTest") };
    });
}

/// nº de records/flats do tweakdb.bin vanilla (de `tweakdb-tool info`) — usado p/
/// flagar qual campo do singleton é a CONTAGEM do records-map / flats.
const N_RECORDS: u32 = 176_198;
const N_FLATS: u32 = 2_880_555;

static DUMPED: AtomicBool = AtomicBool::new(false);

/// O TweakDB carregado visto pelo global já em gameplay: daí em diante é o mesmo objeto (o crescimento do buffer troca `+0x148`/`+0x00`, não o
/// singleton), e as chamadas seguintes pulam as 2 syscalls e a varredura do dyld.
static SINGLETON_FIXO: std::sync::atomic::AtomicPtr<u8> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Ponteiro do singleton TweakDB vivo. Lê o global (populado no load); se null,
/// chama TweakDB::Get() (lazy-init). None se ainda não inicializado/ilegível.
pub unsafe fn singleton() -> Option<*mut u8> {
    let fixo = SINGLETON_FIXO.load(std::sync::atomic::Ordering::Acquire);
    if !fixo.is_null() {
        return Some(fixo);
    }
    // 🔴 GUARDA DE PROCESSO (2026-08-21) — fora do jogo, NADA aqui é válido: `rebase()` calcula o
    // slide a partir do processo atual, então tanto o global quanto o endereço de `Get()` caem
    // dentro do binário hospedeiro (um runner de teste, uma ferramenta que carregue o dylib) e
    // `is_readable` responde TRUE pra eles — a guarda de legibilidade abaixo, sozinha, não
    // protege. Medido: `cargo test` morria com SIGBUS ao chamar o "Get()" rebaseado pra lixo
    // dentro do próprio binário de teste (`api::tests::v4_null_safe`, 15ª chamada). Isso também
    // é robustez da API pública de plugin: `tweakdb_get_flat`/`set_flat` de um plugin carregado
    // fora do jogo agora devolvem false em vez de derrubar o processo.
    if !crate::inicio::boot::in_game() {
        return None;
    }
    // VALIDA que a instância está CARREGADA: o global às vezes aponta pra uma TweakDB
    // staging/vazia (flats size 0, flatDataBuffer null) — usar essa faz o getflat achar nada.
    // Critério de "carregada": flatDataBuffer@+0x148 != 0. Se o global for vazio, cai no Get().
    let loaded = |s: *mut u8| -> bool {
        !s.is_null()
            && crate::memoria::is_readable(s as *const c_void, 0x168)
            && rd_u64(s.add(0x148)) != 0
    };
    let gp = crate::rebase(TDB_SINGLETON_GLOBAL) as *const u8;
    if crate::memoria::is_readable(gp as *const c_void, 8) {
        let s = rd_u64(gp) as *mut u8;
        if loaded(s) {
            // Fixa só em gameplay e só a instância do global: no boot o global pode apontar uma TweakDB de staging.
            if crate::inicio::boot::gameplay_real() {
                SINGLETON_FIXO.store(s, std::sync::atomic::Ordering::Release);
            }
            return Some(s);
        }
    }
    // global vazio/staging → Get() (lazy-init, devolve a TweakDB REAL carregada). Só chama se o
    // endereço de CÓDIGO for legível — fora do processo do jogo (ex. `cargo test`) o rebase()
    // aponta pra lixo, e chamar um ponteiro de função não-validado é SIGSEGV garantido (achado
    // 2026-07-13 ao adicionar os testes null-safe da API de TweakDB — nenhum teste anterior
    // exercitava este caminho, então o gap nunca tinha sido pego).
    let get_addr = crate::rebase(crate::jogo::itens::TWEAKDB_VM);
    if !crate::memoria::is_readable(get_addr as *const c_void, 8) {
        return None;
    }
    let get: extern "C" fn() -> *mut c_void = std::mem::transmute(get_addr);
    let s = get() as *mut u8;
    if loaded(s) {
        Some(s)
    } else {
        // último recurso: o global mesmo que não passe no loaded() (melhor que nada p/ create)
        if crate::memoria::is_readable(gp as *const c_void, 8) {
            let g = rd_u64(gp) as *mut u8;
            if !g.is_null() && crate::memoria::is_readable(g as *const c_void, 0x168) {
                return Some(g);
            }
        }
        None
    }
}

/// Despeja o layout do singleton (0x00..0x168) p/ identificar o records-map e validar
/// os offsets do ctor (FlatPool@+0x30, container@+0x40, container@+0xb8). Marca os
/// campos cujo u32 ≈ N_RECORDS / N_FLATS (a contagem do mapa correspondente).
pub unsafe fn dump_singleton() {
    let s = match singleton() {
        Some(s) => s,
        None => {
            crate::log("[tdb] singleton indisponível (TweakDB não carregada ainda?)");
            return;
        }
    };
    crate::log(&format!("[tdb] singleton @ {s:p} — dump 0x00..0x168:"));
    let mut off = 0usize;
    while off < 0x168 {
        let p = s.add(off);
        if !crate::memoria::is_readable(p as *const c_void, 8) {
            crate::log(&format!("[tdb]   +{off:#05x}: (não mapeado)"));
            off += 8;
            continue;
        }
        let v = rd_u64(p);
        let lo = (v & 0xFFFF_FFFF) as u32;
        let hi = (v >> 32) as u32;
        // flag de contagem aproximada (±64, o mapa pode ter capacidade > size)
        let near = |x: u32, t: u32| x.max(t) - x.min(t) < 256;
        let mut tag = String::new();
        if near(lo, N_RECORDS) || near(hi, N_RECORDS) {
            tag.push_str(" <== ~N_RECORDS (records-map?)");
        }
        if near(lo, N_FLATS) || near(hi, N_FLATS) {
            tag.push_str(" <== ~N_FLATS (flats?)");
        }
        crate::log(&format!(
            "[tdb]   +{off:#05x}: {v:#018x}  (lo={lo} hi={hi}){tag}"
        ));
        off += 8;
    }
    crate::log("[tdb] dump fim L1. L2: scan recursivo (2 níveis) por contagem em [150k,500k]:");
    // O records-map em runtime pode ter contagem != .bin (DLC + records runtime). Em vez de
    // casar 176198 exato, scaneia qualquer u32 numa faixa plausível de contagem de records/flats
    // ([150k, 500k]) nos pointees do singleton e nos pointees DELES (2 níveis). Loga o caminho.
    let plausible = |v: u32| v >= 150_000 && v <= 500_000;
    let scan_region = |base: *const u8, len: usize, path: &str| {
        let mut i = 0usize;
        while i + 4 <= len {
            let v = (base.add(i) as *const u32).read_unaligned();
            if plausible(v) {
                crate::log(&format!("[tdb] L2 {path}+{i:#x} = {v} (contagem? records/flats)"));
            }
            i += 4;
        }
    };
    let mut soff = 0usize;
    while soff < 0x168 {
        let pp = s.add(soff);
        if crate::memoria::is_readable(pp as *const c_void, 8) {
            let p1 = rd_u64(pp) as *const u8;
            // só segue ponteiros plausíveis (heap/imagem), não inteiros pequenos
            if p1 as usize > 0x10000 && crate::memoria::is_readable(p1 as *const c_void, 0x140) {
                scan_region(p1, 0x140, &format!("s+{soff:#05x}->P1"));
                // nível 2: cada ponteiro DENTRO do pointee
                let mut j = 0usize;
                while j < 0x140 {
                    let p2 = (p1.add(j) as *const u64).read_unaligned() as *const u8;
                    if p2 as usize > 0x10000 && crate::memoria::is_readable(p2 as *const c_void, 0x140) {
                        scan_region(p2, 0x140, &format!("s+{soff:#05x}->P1+{j:#x}->P2"));
                    }
                    j += 8;
                }
            }
        }
        soff += 8;
    }
    let _ = N_RECORDS;
    let _ = N_FLATS;
    crate::log("[tdb] dump fim L2.");
}

// ===== SetFlat RUNTIME (2026-06-26) — lê/escreve um flat vivo sem rebakar o .bin =====
// Struct (RED4ext TweakDB.hpp, confirmado pelo dump): flats(SortedUniqueArray<TweakDBID>)@0x40
// {entries, capacity@+0x48, size@+0x4C}, flatDataBuffer@0x148. GetFlatValue: binary-search em flats por
// `tweak_db_id_chave(id)` [40 bits baixos = hash+len], entry.tdbOffset (bytes 5-7 BIG-ENDIAN) →
// FlatValue* = flatDataBuffer + tdbOffset. O valor (data) mora em FlatValue + offsetof(data):
// ~0x08 p/ escalar (Float/Int/Bool), 0x10 p/ 16 bytes (Quaternion/Color).
const FLATS_OFF: usize = 0x40;
/// `SortedArray` do SDK = o DynArray: `capacity@+0x08`, `size@+0x0C` (`RED4EXT_ASSERT_OFFSET`, Containers/SortedArray.hpp:398-399). O motor
/// busca até o size; com os dois campos iguais na carga (3306462), só a declaração diz qual é qual.
const FLATS_CAP_OFF: usize = FLATS_OFF + 0x08;
const FLATS_SIZE_OFF: usize = FLATS_OFF + 0x0C;
const FLAT_DATA_BUFFER_OFF: usize = 0x148; // uintptr_t (RED4ext TweakDB.hpp, ASSERT_OFFSET)
const FLAT_DATA_CAP_OFF: usize = 0x150; //   u32 flatDataBufferCapacity
const FLAT_DATA_END_OFF: usize = 0x158; //   uintptr_t flatDataBufferEnd

/// VIA CANÔNICA de lookup de flat: acha a ENTRY (TweakDBID no SortedUniqueArray) por NOME —
/// binary-search por hash(u32 primário) + length(u8 secundário) (TweakDBID::operator<). Devolve
/// `*mut u64` (a entry, p/ ler OU reescrever o tdbOffset). getflat/setflat/repoint usam ESTA (Q5).
/// Núcleo do binary-search por ID JÁ CALCULADO (sem derivar de string) — usado pela API
/// redscript `TweakDBManager.*` (item TweakXL #44, `PENDENCIAS-UNIFICADAS.md`), cujo `TweakDBID`
/// chega pronto do compilador (`t"..."`/`TDBID.Create` são resolvidos em compile-time pelo scc;
/// só o `u64` atravessa a fronteira nativa). `find_flat_entry(name)` abaixo agora é só um atalho
/// que deriva o id e delega aqui — mesmo algoritmo, sem duplicação.
pub unsafe fn find_flat_entry_by_id(t: *const u8, id: u64) -> Option<*mut u64> {
    let q_hash = (id & 0xFFFF_FFFF) as u32;
    let q_len = ((id >> 32) & 0xFF) as u8;
    let entries = rd_u64(t.add(FLATS_OFF)) as *mut u64;
    let size = (t.add(FLATS_SIZE_OFF) as *const u32).read_unaligned();
    if entries.is_null() || size == 0 || size > 10_000_000 {
        return None;
    }
    // O array é uma alocação contígua só (a do motor, ou a nossa por `Box::leak` no grow, nunca liberada): com as duas pontas legíveis, o
    // meio é. Conferir cada passo custava ~23 syscalls por busca, ~90% do tempo dela, no gancho de spawn sempre ligado.
    if !crate::memoria::is_readable(entries as *const c_void, 8) || !crate::memoria::is_readable(entries.add(size as usize - 1) as *const c_void, 8) {
        return None;
    }
    let (mut lo, mut hi) = (0u32, size);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let ep = entries.add(mid as usize);
        let e = ep.read_unaligned();
        let e_hash = (e & 0xFFFF_FFFF) as u32;
        let e_len = ((e >> 32) & 0xFF) as u8;
        if e_hash < q_hash || (e_hash == q_hash && e_len < q_len) {
            lo = mid + 1;
        } else if e_hash > q_hash || (e_hash == q_hash && e_len > q_len) {
            hi = mid;
        } else {
            return Some(ep);
        }
    }
    None
}

/// Variante por NOME (deriva `tweak_db_id(name)` e delega em `find_flat_entry_by_id`).
#[cfg(feature = "devtools")]
pub unsafe fn find_flat_entry(t: *const u8, name: &str) -> Option<*mut u64> {
    find_flat_entry_by_id(t, tweak_db_id(name))
}

/// tdbOffset de uma entry (bytes 5,6,7 em BIG-ENDIAN).
#[inline]
fn entry_tdb_offset(e: u64) -> u32 {
    ((((e >> 40) & 0xFF) as u32) << 16) | ((((e >> 48) & 0xFF) as u32) << 8) | (((e >> 56) & 0xFF) as u32)
}

/// FlatValue* de um flat por NOME (via `find_flat_entry` + flatDataBuffer + tdbOffset). None se
/// não achar. READ-ONLY multi-thread-safe (ver nota do RED4ext).
pub unsafe fn get_flat_value(t: *const u8, name: &str) -> Option<*mut u8> {
    get_flat_value_by_id(t, tweak_db_id(name))
}

/// Variante por ID já calculado — mesmo mecanismo de `get_flat_value`, sem string.
pub unsafe fn get_flat_value_by_id(t: *const u8, id: u64) -> Option<*mut u8> {
    let ep = find_flat_entry_by_id(t, id)?;
    let off = entry_tdb_offset(ep.read_unaligned());
    let fdb = rd_u64(t.add(FLAT_DATA_BUFFER_OFF)) as *mut u8;
    if fdb.is_null() {
        return None;
    }
    let fv = fdb.add(off as usize);
    if crate::memoria::is_readable(fv as *const c_void, 0x18) { Some(fv) } else { None }
}

/// ArchiveXL `#40` (2026-08-19, continuação — `OnAttachTPP`/`AttachmentExtension::OnAttachTPP`,
/// Extension.cpp:140-185): lê um flat ESCALAR do tipo `TweakDBID` por ID já calculado, mesma
/// técnica de `GetTypeName`@vtable+0xE8 + `GetDataPtr`@vtable+0xF0 já provada ao vivo em
/// `tramp_get_flat_float` (TweakXL `#43`) — mas aceitando o tipo real como `TweakDBID` em vez de
/// `Float`. Usada pra compor `Red::GetFlatValue<Red::TweakDBID>({slotID, ".parentSlot"})` (fonte
/// real do C++, `s_extraSlots`/`s_baseSlots`) sem endereço nativo novo algum — só leitura dinâmica
/// de flat já provada + `bwms_primitives::tweak_db_id_derive` (derivação de sufixo, já provada).
pub unsafe fn get_flat_tdbid(t: *const u8, id: u64) -> Option<u64> {
    get_flat_escalar(t, id, "TweakDBID")
}

/// `Red::GetFlatValue<Red::CName>` por ID (ArchiveXL `#20` 20f, o `<record>.appearanceName` do pedido do ItemFactory): mesma leitura do
/// `get_flat_tdbid`, com o tipo `CName`.
pub unsafe fn get_flat_cname(t: *const u8, id: u64) -> Option<u64> {
    get_flat_escalar(t, id, "CName")
}

/// Texto de um flat `String` por ID (o `CString` do motor que o `GetDataPtr` devolve).
pub(crate) unsafe fn get_flat_texto(t: *const u8, id: u64) -> Option<String> {
    dado_do_flat(t, id, "String").map(|p| crate::jogo::cstring::texto(p))
}

/// Flat escalar de 8 B por ID, só se o tipo do `FlatValue` (`GetTypeName` `vt+0xE8`) é `tipo`.
unsafe fn get_flat_escalar(t: *const u8, id: u64, tipo: &str) -> Option<u64> {
    let p = dado_do_flat(t, id, tipo)?;
    crate::memoria::is_readable(p as *const c_void, 8).then(|| (p as *const u64).read_unaligned())
}

/// Ponteiro pro valor de um flat por ID, só se o tipo do `FlatValue` (`GetTypeName` `vt+0xE8`) é `tipo`; o valor pelo `GetDataPtr` (`vt+0xF0`).
unsafe fn dado_do_flat(t: *const u8, id: u64, tipo: &str) -> Option<*const u8> {
    let fv = get_flat_value_by_id(t, id)?;
    let vt = (fv as *const u64).read_unaligned();
    if !crate::memoria::is_readable(vt as *const c_void, 0xF8) {
        return None;
    }
    let get_type_name = ((vt as *const u8).add(0xE8) as *const u64).read_unaligned();
    let get_data_ptr = ((vt as *const u8).add(0xF0) as *const u64).read_unaligned();
    if !crate::rtti::sane(get_type_name as *mut c_void) || !crate::rtti::sane(get_data_ptr as *mut c_void) {
        return None;
    }
    let mut name_out: u64 = 0;
    let f_gtn: extern "C" fn(*mut c_void, *mut u64) -> *mut u64 = std::mem::transmute(get_type_name);
    let ret = f_gtn(fv as *mut c_void, &mut name_out as *mut u64);
    // O GetTypeName já devolve o hash do nome: compara com o FNV-1a do tipo pedido, sem passar pelo pool de nomes (syscall + String por
    // leitura, no gancho de spawn). Os chamadores passam o nome com a grafia exata do motor.
    if ret as u64 != crate::cname::cname(tipo) {
        return None;
    }
    let f_gdp: extern "C" fn(*mut c_void) -> *mut c_void = std::mem::transmute(get_data_ptr);
    let data_ptr = f_gdp(fv as *mut c_void);
    crate::memoria::is_readable(data_ptr, 8).then_some(data_ptr as *const u8)
}

/// TweakDB com os flats no ar: `+0x38` sozinho não basta (o construtor já o põe em 1, `0x1037ccab8`); o `flatDataBuffer` (`+0x148`) só existe
/// depois do `Load`.
pub(crate) unsafe fn carregado() -> bool {
    singleton().is_some_and(|t| {
        crate::memoria::is_readable(t as *const c_void, 0x150) && *t.add(0x38) == 1 && (t.add(0x148) as *const u64).read_unaligned() != 0
    })
}

/// Flat escalar com o tipo do próprio `FlatValue` (`GetTypeName` `vt+0xE8`): `(tipo, bytes do valor em u64)`. A leitura do `TweakDB:GetFlat` dos
/// mods CET; `None` pra flat que não existe ou que não é escalar (array, `String`, struct).
#[cfg(feature = "lua")]
pub(crate) unsafe fn flat_escalar(id: u64) -> Option<(String, u64)> {
    let t = singleton()? as *const u8;
    let tipo = escritor::tipo_do_valor(t, id)?;
    let n = escritor::largura_do_tipo(&tipo)?;
    let p = dado_do_flat(t, id, &tipo)?;
    let mut b = [0u8; 8];
    std::ptr::copy_nonoverlapping(p, b.as_mut_ptr(), n);
    Some((tipo, u64::from_le_bytes(b)))
}

/// Records do mesmo tipo de `modelo` (o array do mapa por tipo do TweakDB, `+0x88`): `(id, record)`. O id do record está em `+0x40`; a chave
/// do mapa é o hash do tipo que a vtable do record devolve (`vt+0x08`).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn records_do_tipo(t: *mut u8, modelo: *mut u8) -> Vec<(u64, *mut u8)> {
    let Some(tipo) = tipo_do_record(modelo) else { return Vec::new() };
    let Some(no) = crate::jogo::mapas::nos(t.add(0x88)).and_then(|v| v.into_iter().find(|n| (n.add(0x08) as *const u32).read_unaligned() == tipo)) else {
        return Vec::new();
    };
    let arr = no.add(0x10);
    let (entradas, n) = ((arr as *const u64).read_unaligned() as *const [u64; 2], (arr.add(0x0c) as *const u32).read_unaligned() as usize);
    if entradas.is_null() || n > 200_000 || !crate::memoria::is_readable(entradas as *const c_void, n * 16) {
        return Vec::new();
    }
    (0..n)
        .filter_map(|i| {
            let rec = (*entradas.add(i))[0];
            crate::memoria::is_readable((rec + 0x40) as *const c_void, 8).then(|| (tweak_db_id_chave(((rec + 0x40) as *const u64).read_unaligned()), rec as *mut u8))
        })
        .collect()
}

/// Hash do tipo de um record vivo, pela vtable (`vt+0x08`) — a chave do mapa por tipo do TweakDB.
pub(crate) unsafe fn tipo_do_record(rec: *mut u8) -> Option<u32> {
    if rec.is_null() || !crate::memoria::is_readable(rec as *const c_void, 0x48) {
        return None;
    }
    let vt = (rec as *const u64).read_unaligned();
    if !crate::memoria::is_readable((vt + 0x08) as *const c_void, 8) {
        return None;
    }
    let f = ((vt + 0x08) as *const u64).read_unaligned();
    if !crate::rtti::sane(f as *mut c_void) {
        return None;
    }
    let tipo: extern "C" fn(u64) -> u32 = std::mem::transmute(f);
    Some(tipo(rec as u64))
}

/// Repoint: aponta o flat de `name` pro FlatValue em flatDataBuffer+`new_off` (reescreve os bytes
/// 5,6,7 BE da entry, preservando hash+len). O jogo passa a ler o valor novo. É o que permite
/// SetFlat NÃO-escalar (array/string), onde o valor não cabe in-place e precisa de um FlatValue novo.
#[cfg(feature = "devtools")]
pub unsafe fn set_flat_offset(t: *const u8, name: &str, new_off: u32) -> bool {
    set_flat_offset_by_id(t, tweak_db_id(name), new_off)
}

/// Variante por ID já calculado — mesmo mecanismo de `set_flat_offset`, sem derivar de string.
pub unsafe fn set_flat_offset_by_id(t: *const u8, id: u64, new_off: u32) -> bool {
    let ep = match find_flat_entry_by_id(t, id) {
        Some(e) => e,
        None => return false,
    };
    let keep = tweak_db_id_chave(ep.read_unaligned());
    let b5 = ((new_off >> 16) & 0xFF) as u64;
    let b6 = ((new_off >> 8) & 0xFF) as u64;
    let b7 = (new_off & 0xFF) as u64;
    ep.write_unaligned(keep | (b5 << 40) | (b6 << 48) | (b7 << 56));
    true
}

/// SetFlat NÃO-escalar (array/string/etc.): cria um FlatValue novo com `data` (vtable nativa do
/// tipo, copiada de `donor` do MESMO tipo) e aponta `field` pra ele. Combina create_flat_value +
/// set_flat_offset — a via canônica p/ mudar arrays (ex.: attacks/statModifiers da arma). Devolve
/// o tdbOffset novo, ou None.
pub unsafe fn set_flat_nonscalar(t: *mut u8, field: &str, donor: &str, data: &[u8], align: usize) -> Option<i32> {
    set_flat_nonscalar_by_id(t, tweak_db_id(field), tweak_db_id(donor), data, align)
}

/// Variante por ID já calculado — mesmo mecanismo de `set_flat_nonscalar`, sem derivar de string.
pub unsafe fn set_flat_nonscalar_by_id(_t: *mut u8, field_id: u64, donor_id: u64, data: &[u8], align: usize) -> Option<i32> {
    escritor::gravar_valor(field_id, donor_id, data, align).map(|o| o as i32)
}

// ===== ARRAY FLATS (o caso das armas: attacks/statModifiers = array:TweakDBID) =====
// Layout VERIFICADO (workflow 2026-07-01, RED4EXT_ASSERT em Containers/DynArray.hpp:564-566,625 +
// TweakDB.hpp:140 + TweakDB-inl.hpp:348-352): payload do flat array = DynArray BY-VALUE após a
// vtable → entries*@+0x8, capacity(u32)@+0x10, size(u32)@+0x14, FlatValue total 0x18.
// DUAS DEFESAS obrigatórias (veredito adversarial):
// 1. o DynArray lê seu IAllocator do FIM do buffer de elementos (cap>0) ou dos bits de `entries`
//    (cap==0) → nosso buffer leva um TRAILER de 8B com o vft de allocator copiado do donor.
// 2. o offset +0x8 do payload é inferência (assert só existe p/ Quaternion) → probe de sanidade
//    no donor (size<=cap, cap<1M, entries legível) ANTES de escrever.

/// Payload DynArray by-value (0x10): entries@+0, capacity(u32)@+8, size(u32)@+0xC.
fn dynarray_payload(entries: u64, n: u32) -> [u8; 16] {
    let mut p = [0u8; 16];
    p[..8].copy_from_slice(&entries.to_le_bytes());
    p[8..12].copy_from_slice(&n.to_le_bytes());
    p[12..].copy_from_slice(&n.to_le_bytes());
    p
}

/// SetFlat de ARRAY com elemento de 8B (array:TweakDBID / array:CName): monta o buffer de
/// elementos em memória NOSSA (Box::leak — intencional, o flat vive até o fim da sessão) com o
/// trailer de allocator do donor, cria o FlatValue e aponta `field`. Donor DEVE ser um flat
/// array do MESMO tipo. Devolve o tdbOffset novo.
pub unsafe fn set_flat_array_u64(t: *mut u8, field: &str, donor: &str, elems: &[u64]) -> Option<i32> {
    set_flat_array_u64_by_id(t, tweak_db_id(field), tweak_db_id(donor), elems)
}

/// Variante por ID já calculado — mesmo mecanismo de `set_flat_array_u64`, sem derivar de string.
/// `donor_id == field_id` é um uso válido e comum (SELF-DONOR): quando o campo já É um array
/// (caso de `array_append_by_id`/etc., items TweakXL #32/#33), sua PRÓPRIA `FlatValue` atual serve
/// de donor (tem a vtable/alloc certos, lida ANTES de repontar `field` — ordem importa, mas como
/// `create_flat_value_by_id` lê o donor e só DEPOIS `set_flat_offset_by_id` repontam, a leitura do
/// donor sempre vê o valor PRÉ-mutação, mesmo quando donor==field).
pub unsafe fn set_flat_array_u64_by_id(t: *mut u8, field_id: u64, donor_id: u64, elems: &[u64]) -> Option<i32> {
    let payload = carga_de_array(t as *const u8, donor_id, elems)?;
    set_flat_nonscalar_by_id(t, field_id, donor_id, &payload, 8)
}

/// Carga (`{entries, cap, size}`, 16 B) de um `FlatValue` de array de elemento de 8 B com `elems`, num buffer nosso com o trailer do alocador
/// do `FlatValue` do flat `donor_id` (array do mesmo tipo). `None` se o doador não parece array.
pub(crate) unsafe fn carga_de_array(t: *const u8, donor_id: u64, elems: &[u64]) -> Option<[u8; 16]> {
    let dv = get_flat_value_by_id(t, donor_id)?;
    let dados: Vec<u8> = elems.iter().flat_map(|e| e.to_le_bytes()).collect();
    carga_de_array_bytes(dv, 8, 8, &dados, elems.len())
}

/// Carga de um `FlatValue` de array com `n` elementos de `tam` bytes em `dados`, num buffer nosso (alinhado a `alinh`, no mínimo 8) com o
/// trailer do alocador copiado do `FlatValue` de array `dv` (o doador, do mesmo tipo). O `DynArray::Reserve` do motor (`0x100028674` na
/// 2.5) lê e grava o alocador em `entries + alinha8(cap * tam)`, e no próprio campo `entries` quando `cap == 0`.
pub(crate) unsafe fn carga_de_array_bytes(dv: *const u8, tam: usize, alinh: usize, dados: &[u8], n: usize) -> Option<[u8; 16]> {
    // probe de sanidade do donor (defesa 2): o payload em +0x8 parece um DynArray?
    let d_entries = rd_u64(dv.add(0x8));
    let d_cap = (dv.add(0x10) as *const u32).read_unaligned();
    let d_size = (dv.add(0x14) as *const u32).read_unaligned();
    if !crate::register::dynarray_coerente(d_entries as usize, d_size, d_cap) {
        crate::log_dev!("[flat] mkarr: donor {dv:p} não parece array (cap={d_cap} size={d_size})");
        return None;
    }
    if d_size > 0 && !crate::memoria::is_readable(d_entries as *const c_void, tam) {
        crate::log_dev!("[flat] mkarr: entries do donor ilegível ({d_entries:#x})");
        return None;
    }
    let alinha8 = |v: u64| (v + 7) & !7;
    // vft de allocator do donor (defesa 1): cap==0 → está nos bits de entries; cap>0 → trailer.
    let alloc_vft = if d_cap == 0 {
        d_entries
    } else {
        let tr = d_entries + alinha8(d_cap as u64 * tam as u64);
        if crate::memoria::is_readable(tr as *const c_void, 8) { rd_u64(tr as *const u8) } else { 0 }
    };
    if dados.len() != n * tam {
        return None;
    }
    // Array VAZIO (`!remove-all`, TweakXL #30): com `cap==0` o alocador mora no próprio campo `entries`, sem buffer.
    let entries = if n == 0 {
        alloc_vft
    } else {
        // Elementos + o alocador na palavra seguinte, no buffer do próprio alocador do doador: o motor solta por ele ao descarregar o TweakDB.
        let corpo = alinha8((n * tam) as u64) as usize;
        let buf = crate::jogo::listas::alocar_pelo_alocador(alloc_vft, corpo + 8, alinh.max(8));
        if buf.is_null() {
            return None;
        }
        std::ptr::write_bytes(buf, 0, corpo);
        std::ptr::copy_nonoverlapping(dados.as_ptr(), buf, dados.len());
        (buf.add(corpo) as *mut u64).write_unaligned(alloc_vft);
        buf as u64
    };
    Some(dynarray_payload(entries, n as u32))
}

/// Lê os elementos (8 bytes cada) de um flat array (`array:TweakDBID`/`array:CName`/etc.) pelo ID.
/// `None` se o flat não existir/não parecer array; `Some(vec![])` se existir mas estiver vazio.
pub unsafe fn get_flat_array_u64_by_id(t: *const u8, id: u64) -> Option<Vec<u64>> {
    let fv = get_flat_value_by_id(t, id)?;
    let entries = rd_u64(fv.add(0x8));
    let cap = (fv.add(0x10) as *const u32).read_unaligned();
    let size = (fv.add(0x14) as *const u32).read_unaligned();
    if !crate::register::dynarray_coerente(entries as usize, size, cap) {
        return None;
    }
    if size == 0 {
        return Some(Vec::new());
    }
    if !crate::memoria::is_readable(entries as *const c_void, size as usize * 8) {
        return None;
    }
    let mut out = Vec::with_capacity(size as usize);
    for i in 0..size {
        out.push(rd_u64((entries + i as u64 * 8) as *const u8));
    }
    Some(out)
}

// ===== Mutação cirúrgica de array (TweakXL #32/#33: `!append`/`!prepend`/`!append-once`/
// `!prepend-once`/`!remove`/`!append-from`/`!prepend-from`) — insere/remove elemento(s) SEM
// reescrever o array inteiro na intenção do mod (mesmo efeito final observável: o array vivo no
// jogo muda; a IMPLEMENTAÇÃO faz leitura + `set_flat_array_u64_by_id` self-donor por trás, não uma
// via cirúrgica de baixo nível separada — não há necessidade de RE nova, é composição de 2
// primitivas já provadas: `get_flat_array_u64_by_id`(leitura) + `set_flat_array_u64_by_id`
// self-donor(escrita), a MESMA dupla já usada pelo comando `mkarr` provado ao vivo). Array que fica
// VAZIO é gravado vazio pelo `set_flat_array_u64_by_id`, como no `!remove-all`
// (`proofs/2026-08-11-tweakxl-30-removeall-PROVADO.log`).
pub unsafe fn array_append_by_id(t: *mut u8, id: u64, value: u64, unique: bool) -> bool {
    let mut elems = match get_flat_array_u64_by_id(t, id) {
        Some(e) => e,
        None => return false,
    };
    if unique && elems.contains(&value) {
        crate::log_dev!("[xlarr] AppendOnce(id={id:#x}, value={value:#x}): já presente, skip");
        return true;
    }
    elems.push(value);
    let n = elems.len();
    match set_flat_array_u64_by_id(t, id, id, &elems) {
        Some(off) => {
            crate::log_dev!("[xlarr] Append(id={id:#x}, value={value:#x}) -> tdbOffset={off:#x} ({n} elementos)");
            true
        }
        None => {
            crate::log_dev!("[xlarr] Append(id={id:#x}) FALHOU");
            false
        }
    }
}

pub unsafe fn array_prepend_by_id(t: *mut u8, id: u64, value: u64, unique: bool) -> bool {
    let mut elems = match get_flat_array_u64_by_id(t, id) {
        Some(e) => e,
        None => return false,
    };
    if unique && elems.contains(&value) {
        crate::log_dev!("[xlarr] PrependOnce(id={id:#x}, value={value:#x}): já presente, skip");
        return true;
    }
    elems.insert(0, value);
    let n = elems.len();
    match set_flat_array_u64_by_id(t, id, id, &elems) {
        Some(off) => {
            crate::log_dev!("[xlarr] Prepend(id={id:#x}, value={value:#x}) -> tdbOffset={off:#x} ({n} elementos)");
            true
        }
        None => {
            crate::log_dev!("[xlarr] Prepend(id={id:#x}) FALHOU");
            false
        }
    }
}

pub unsafe fn array_remove_by_id(t: *mut u8, id: u64, value: u64) -> bool {
    let mut elems = match get_flat_array_u64_by_id(t, id) {
        Some(e) => e,
        None => return false,
    };
    let before = elems.len();
    elems.retain(|&v| v != value);
    if elems.len() == before {
        crate::log_dev!("[xlarr] Remove(id={id:#x}, value={value:#x}): elemento não estava no array");
        return false;
    }
    let n = elems.len();
    match set_flat_array_u64_by_id(t, id, id, &elems) {
        Some(off) => {
            crate::log_dev!("[xlarr] Remove(id={id:#x}, value={value:#x}) -> tdbOffset={off:#x} ({n} elementos)");
            true
        }
        None => {
            crate::log_dev!("[xlarr] Remove(id={id:#x}) FALHOU");
            false
        }
    }
}

/// `!remove-all` (TweakXL #30, `TweakChangeset::RemoveAllElements` real — `entry.deleteAll = true`
/// na fonte C++, distinta de `!remove`: não recebe VALOR nenhum, limpa o array INTEIRO
/// incondicionalmente). Reusa `set_flat_array_u64_by_id` com `elems=&[]` — agora seguro pra
/// escrever array vazio de verdade (fix acima: `entries` guarda o vft do allocator DIRETO quando
/// `cap==0`, mesma convenção já usada no lado leitura, sem precisar de buffer/trailer nenhum).
pub unsafe fn array_remove_all_by_id(t: *mut u8, id: u64) -> bool {
    let before = match get_flat_array_u64_by_id(t, id) {
        Some(e) => e.len(),
        None => return false,
    };
    match set_flat_array_u64_by_id(t, id, id, &[]) {
        Some(off) => {
            crate::log_dev!("[xlarr] RemoveAll(id={id:#x}) -> tdbOffset={off:#x} ({before} elementos removidos, array vazio)");
            true
        }
        None => {
            crate::log_dev!("[xlarr] RemoveAll(id={id:#x}) FALHOU");
            false
        }
    }
}

/// `!append-from`/`!merge` (append=true) ou `!prepend-from` (append=false): mescla os elementos
/// de OUTRO flat array (`src_id`) no array-alvo (`id`). Ordem do merge segue a semântica real
/// (`writer.rs::compute_value`, `EditOp::AppendFrom|PrependFrom`): append cola a fonte no FIM;
/// prepend cola a fonte na FRENTE, preservando a ordem interna da fonte.
pub unsafe fn array_merge_from_by_id(t: *mut u8, id: u64, src_id: u64, append: bool) -> bool {
    let mut elems = match get_flat_array_u64_by_id(t, id) {
        Some(e) => e,
        None => return false,
    };
    let src = match get_flat_array_u64_by_id(t, src_id) {
        Some(e) => e,
        None => {
            crate::log_dev!("[xlarr] MergeFrom(id={id:#x}, src={src_id:#x}): fonte não achada/não é array");
            return false;
        }
    };
    if append {
        elems.extend(src);
    } else {
        let mut merged = src;
        merged.append(&mut elems);
        elems = merged;
    }
    // Vazio aqui = alvo e fonte já vazios: o array já está como deveria, e gravar criaria outro FlatValue à toa.
    if elems.is_empty() {
        crate::log_dev!("[xlarr] MergeFrom(id={id:#x}, src={src_id:#x}): alvo e fonte vazios, nada a mesclar");
        return true;
    }
    let n = elems.len();
    match set_flat_array_u64_by_id(t, id, id, &elems) {
        Some(off) => {
            crate::log_dev!(
                "[xlarr] MergeFrom(id={id:#x}, src={src_id:#x}, append={append}) -> tdbOffset={off:#x} ({n} elementos)"
            );
            true
        }
        None => {
            crate::log_dev!("[xlarr] MergeFrom(id={id:#x}) FALHOU");
            false
        }
    }
}

/// Um elemento de array u64: hex cru (`0x1234`, serve p/ array:CName com hash conhecido) passa
/// direto; qualquer outra string vira `TweakDBID` (o caso comum: nome de record referenciado,
/// ex. `Attacks.A`). Extraído do `parse_u64_list` (item-a-item) pra reuso pelo `EditOp` do
/// pipeline `.yaml` (TweakXL #32/#33), cujo valor de `!append`/`!remove`/etc. é 1 elemento só.
pub fn parse_u64_elem(p: &str) -> u64 {
    let p = p.trim();
    p.strip_prefix("0x")
        .and_then(|h| u64::from_str_radix(h, 16).ok())
        .unwrap_or_else(|| tweak_db_id(p))
}

/// "Items.A,Items.B,0x1234" → [tweak_db_id, tweak_db_id, 0x1234].
pub fn parse_u64_list(s: &str) -> Vec<u64> {
    s.split(',').map(str::trim).filter(|p| !p.is_empty()).map(parse_u64_elem).collect()
}

/// `mkarr <field> <donor> <a,b,c>`: SetFlat de array — cria um flat
/// array:TweakDBID novo com os elementos dados e aponta `field` pra ele. Ex. (dar um attack a
/// mais pra uma arma clonada):
///   mkarr Items.X.attacks Items.Preset_Yasha_Default.attacks Attacks.A,Attacks.B
pub unsafe fn mkarr_cmd(field: &str, donor: &str, list: &str) {
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log_dev!("[flat] singleton indisponível");
            return;
        }
    };
    let elems = parse_u64_list(list);
    if elems.is_empty() {
        crate::log_dev!("[flat] mkarr: lista vazia (ex.: Attacks.A,Attacks.B)");
        return;
    }
    let before = get_flat_value(t, field).map(|f| f as usize).unwrap_or(0);
    match set_flat_array_u64(t, field, donor, &elems) {
        Some(off) => {
            // read-back: confirma que o field agora aponta pro array novo com size certo
            let fv = get_flat_value(t, field);
            let (after, size) = fv
                .map(|f| (f as usize, (f.add(0x14) as *const u32).read_unaligned()))
                .unwrap_or((0, 0));
            crate::log_dev!(
                "[flat] mkarr '{field}' {} elems -> tdbOffset {off:#x} | FlatValue {before:#x} -> {after:#x} | size lido={size}",
                elems.len()
            );
        }
        None => crate::log_dev!("[flat] mkarr FALHOU (donor '{donor}' é array do mesmo tipo? buffer cheio?)"),
    }
}

/// DEDUP-POOL (porte do TweakXL `TweakDBBuffer::AllocateValue`, Buffer.cpp:56-82): lista de
/// (vtable do tipo, hash FNV-1a64 do valor, tdbOffset). Um valor idêntico do mesmo tipo é criado UMA
/// vez só; os demais reusam o offset. Evita vazar o flatDataBuffer com duplicatas (o que aceleraria o
/// estouro). Vec + scan linear (const-construível em static; o pool é pequeno = flats novos da sessão).
static FLAT_POOL: std::sync::Mutex<Vec<(u64, u64, i32)>> = std::sync::Mutex::new(Vec::new());

/// O motor recarregou o TweakDB do disco ([`instalar_recarga`]): o diário e o offset vivo por tipo do escritor, o registro de herança e o pool
/// de valores falam do TweakDB velho. Esquece tudo sem desfazer — os offsets antigos não valem no índice novo.
pub(crate) fn esquecer_o_tweakdb_velho() {
    escritor::esquecer();
    registro::esquecer_tudo();
    FLAT_POOL.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

// `TweakDB::Reload` (`0x1037cef90`): toma as travas, solta os records, zera o índice de flats e os mapas e chama o `Load` (`0x1037ce05c`) de
// novo. O motor recarrega assim quando um mod do mod.io é instalado com o jogo aberto (F2c, 2026-10-26), e os tweaks da sessão somem junto.
const RECARGA_VM: u64 = 0x1_037c_ef90;
const RECARGA_PROLOGO: u64 = 0xa907_5ff8_d102_c3ff;
static ORIG_RECARGA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

unsafe extern "C" fn recarga(a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64, a7: u64) -> u64 {
    let f: extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64) -> u64 = std::mem::transmute(crate::ganchos::original_publicado(&ORIG_RECARGA));
    let r = f(a0, a1, a2, a3, a4, a5, a6, a7);
    crate::mods_do_usuario::pipeline::depois_da_recarga_do_motor();
    r
}

/// O gancho do `TweakDB::Reload`, no carregamento. Sem o prólogo esperado, a recarga do motor segue sem o BWMS reaplicar os tweaks.
pub(crate) unsafe fn instalar_recarga() {
    let alvo = crate::rebase(RECARGA_VM) as *mut c_void;
    if !crate::memoria::is_readable(alvo, 8) || (alvo as *const u64).read_unaligned() != RECARGA_PROLOGO {
        return crate::log_dev!("[tweakdb] TweakDB::Reload sem o prólogo esperado — a recarga do motor fica sem os tweaks dos mods");
    }
    let posto = crate::ganchos::instalar_com_threads_paradas(alvo, recarga as *mut c_void, Some(&ORIG_RECARGA)).is_some();
    crate::log_dev!("[tweakdb] gancho do TweakDB::Reload: {posto}");
}

/// Consulta o dedup-pool: valor (vtable+hash) já alocado? devolve o tdbOffset. Extraído p/ teste.
fn flat_pool_lookup(vft: u64, data_hash: u64) -> Option<i32> {
    FLAT_POOL
        .lock()
        .ok()?
        .iter()
        .find(|(v, h, _)| *v == vft && *h == data_hash)
        .map(|(_, _, o)| *o)
}
/// Registra (vtable, hash, offset) no dedup-pool.
fn flat_pool_record(vft: u64, data_hash: u64, off: i32) {
    if let Ok(mut p) = FLAT_POOL.lock() {
        p.push((vft, data_hash, off));
    }
}

/// Cria um FlatValue NOVO no flatDataBuffer e devolve o tdbOffset (relativo ao buffer). None com
/// doador inválido ou se o buffer não crescer. Porte da lógica RED4ext `TweakDB::CreateFlatValue`, MAS reusa a
/// vtable NATIVA (copiada de `donor` = um flat existente do MESMO tipo) em vez de instanciar um
/// FlatValueImpl (cuja vtable seria a do SDK C++ → o jogo saltaria pro nosso código). `data` =
/// bytes do valor (f32/i32/DynArray{ptr,size,cap}/...). Chamado da thread do jogo (SetFlat de script) e das threads de
/// aplicação de tweak (uma por arquivo): um escritor por vez, pela trava `ESCRITOR` da função. Sem espaço, o buffer cresce
/// pelo `buffer::crescer` (pool do próprio TweakDB, sob o mutex00).
///
/// Variante por ID já calculado (sem derivar de string) — núcleo real; `create_flat_value(name)`
/// abaixo só deriva o id do donor e delega aqui. Usada pela mutação de array por ID
/// (`array_append_by_id`/etc., items TweakXL #32/#33) — o `flat`/`src` do pipeline `.yaml` já
/// chegam como STRING do parser offline, mas o donor de um array é o PRÓPRIO campo (self-donor),
/// então derivar o id 1x e reusar em toda a cadeia evita 3 `tweak_db_id()` redundantes.
pub unsafe fn create_flat_value_by_id(t: *mut u8, donor_id: u64, data: &[u8], align: usize) -> Option<i32> {
    let dv = get_flat_value_by_id(t as *const u8, donor_id)?; // via canônica (a mesma do getflat)
    criar_valor_com_vft(t, rd_u64(dv), data, align) // FlatValue+0x00 = vtable nativa do tipo (identifica o TIPO)
}

/// O núcleo do [`create_flat_value_by_id`] com a vtable já em mãos (a do valor padrão do tipo, pro flat novo de tipo sem flat doador). O
/// valor vai em `+8`, ou no alinhamento dele quando passa de 8 (o `Quaternion` mora em `+0x10` no criador do próprio motor, `0x1037f65c0`).
pub(crate) unsafe fn criar_valor_com_vft(t: *mut u8, vft: u64, data: &[u8], align: usize) -> Option<i32> {
    // Um escritor por vez: o pipeline de mods abre uma thread por arquivo de tweak e o SetFlat de script roda na thread do jogo.
    // Sem a trava, duas leem o mesmo fim do buffer e gravam por cima uma da outra; o dedup (lookup e registro) também fica dentro.
    // O `buffer::crescer` toma o mutex00 do TweakDB, que é outra trava.
    static ESCRITOR: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _um_por_vez = ESCRITOR.lock().unwrap_or_else(|e| e.into_inner());
    if vft == 0 {
        return None;
    }
    // DEDUP: valor idêntico do mesmo tipo já alocado? reusa o offset (não escreve de novo).
    let data_hash = bwms_primitives::fnv1a64(data);
    if let Some(off) = flat_pool_lookup(vft, data_hash) {
        crate::log_dev!(
            "[flat] dedup-pool HIT: vtable={vft:#x} hash={data_hash:#x} -> offset REUSADO {off} (não alocou de novo)"
        );
        return Some(off);
    }
    let a = align.max(8) as u64;
    let au = |v: u64| (v + a - 1) & !(a - 1);
    let dado_off = a; // 8 = slot da vtable
    let flat_size = au(dado_off + data.len() as u64);
    let mut fdb = rd_u64(t.add(FLAT_DATA_BUFFER_OFF));
    let mut cap = (t.add(FLAT_DATA_CAP_OFF) as *const u32).read_unaligned() as u64;
    let mut end = rd_u64(t.add(FLAT_DATA_END_OFF));
    if fdb == 0 || end < fdb {
        return None;
    }
    if au(end) + flat_size > fdb + cap {
        // Sem espaço: o buffer cresce pro máximo (`buffer::crescer`, pelo pool do próprio TweakDB) e o valor vai no buffer novo.
        if let Err(e) = buffer::crescer() {
            crate::log_dev!("[flat] create: buffer cheio e não cresceu: {e}");
            return None;
        }
        fdb = rd_u64(t.add(FLAT_DATA_BUFFER_OFF));
        cap = (t.add(FLAT_DATA_CAP_OFF) as *const u32).read_unaligned() as u64;
        end = rd_u64(t.add(FLAT_DATA_END_OFF));
    }
    let pos = au(end);
    if pos + flat_size > fdb + cap {
        crate::log_dev!("[flat] create: buffer cheio mesmo no tamanho máximo");
        return None;
    }
    let dst = pos as *mut u8; // flatDataBuffer é heap RW do jogo — escrita direta, sem COW
    (dst as *mut u64).write_unaligned(vft);
    std::ptr::copy_nonoverlapping(data.as_ptr(), dst.add(dado_off as usize), data.len());
    (t.add(FLAT_DATA_END_OFF) as *mut u64).write_unaligned(pos + flat_size); // avança flatDataBufferEnd
    let off = (pos - fdb) as i32; // tdbOffset (relativo ao buffer)
    flat_pool_record(vft, data_hash, off);
    Some(off)
}

/// Variante por NOME (deriva `tweak_db_id(donor)` e delega em `create_flat_value_by_id`).
#[cfg(feature = "devtools")]
pub unsafe fn create_flat_value(t: *mut u8, donor: &str, data: &[u8], align: usize) -> Option<i32> {
    create_flat_value_by_id(t, tweak_db_id(donor), data, align)
}

/// `getflat <nome>` (READ-ONLY): acha o FlatValue e dumpa o cabeçalho (vtable + dados em
/// +0x08/+0x10) pra confirmar o lookup e ver onde o valor mora.
#[cfg(feature = "devtools")]
pub unsafe fn probe_flat(name: &str) {
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log_dev!("[flat] singleton indisponível");
            return;
        }
    };
    // diagnóstico: confirma que o singleton resolvido está CARREGADO (flats_size ~3.3M, fdb != 0)
    let flats_sz = (t.add(FLATS_SIZE_OFF) as *const u32).read_unaligned();
    let fdb = rd_u64(t.add(0x148));
    crate::log_dev!("[flat] singleton={t:p} flats_size={flats_sz} fdb={fdb:#x}");
    match get_flat_value(t, name) {
        None => crate::log_dev!("[flat] '{name}' NÃO achado (id={:#x})", tweak_db_id(name)),
        Some(fv) => {
            let vt = rd_u64(fv);
            let d08 = (fv.add(0x08) as *const u32).read_unaligned();
            let d10 = (fv.add(0x10) as *const u32).read_unaligned();
            crate::log_dev!(
                "[flat] '{name}' @ {fv:p} | vtable={vt:#x} | +0x08: u32 {d08:#x}/f32 {} | +0x10: u32 {d10:#x}/f32 {}",
                f32::from_bits(d08), f32::from_bits(d10)
            );
        }
    }
}

/// `getarr <nome>` (READ-ONLY) — **`cet-tweakdb-read-records` (2026-07-13):** lê um flat ARRAY
/// (ex.: `attacks`/`statModifiers` de uma arma) de um record VIVO e dumpa os elementos (assume
/// `array:TweakDBID`/`array:CName`, elementos de 8B — mesmo layout já usado pelo lado de
/// ESCRITA em `set_flat_array_u64`/`mkarr`, aqui só lendo). Payload DynArray @FlatValue+0x8:
/// entries@+0x8, cap(u32)@+0x10, size(u32)@+0x14 (mesmo layout documentado ali).
#[cfg(feature = "devtools")]
pub unsafe fn probe_array_flat(name: &str) {
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log_dev!("[flat] singleton indisponível");
            return;
        }
    };
    let fv = match get_flat_value(t, name) {
        None => {
            crate::log_dev!("[flat] getarr '{name}' NÃO achado (id={:#x})", tweak_db_id(name));
            return;
        }
        Some(fv) => fv,
    };
    let entries = rd_u64(fv.add(0x8));
    let cap = (fv.add(0x10) as *const u32).read_unaligned();
    let size = (fv.add(0x14) as *const u32).read_unaligned();
    if !crate::register::dynarray_coerente(entries as usize, size, cap) {
        crate::log_dev!(
            "[flat] getarr '{name}': não parece array (cap={cap} size={size}) — é escalar? use getflat"
        );
        return;
    }
    if size > 0 && !crate::memoria::is_readable(entries as *const c_void, size as usize * 8) {
        crate::log_dev!("[flat] getarr '{name}': entries ilegível ({entries:#x})");
        return;
    }
    // A única saída do dump é o log de dev: sem `devlog` o laço não lê os elementos nem monta o texto.
    #[cfg(feature = "devlog")]
    {
        let mut out = format!("[flat] getarr '{name}' @ {fv:p}: cap={cap} size={size}\n");
        for i in 0..size as usize {
            let v = rd_u64((entries as *const u8).add(i * 8));
            out.push_str(&format!("  [{i}] = {v:#018x}\n"));
        }
        crate::log(&out);
    }
}

/// Núcleo UNGATED de get/set de flat escalar (offset +0x08, o caso comum — a maioria dos stats
/// numéricos de item/record). Usado pela API pública (`tweakdb-mod-api`, BwmsApi) — mods de verdade
/// que chamam a API já querem o efeito na hora, sem depender de um marcador de dev; os comandos dev
/// `getflat`/`setflat` vão por `probe_flat`/`write_flat`. `api_set_flat_scalar` grava num FlatValue
/// próprio (`escritor::gravar_escalar`) e re-aponta só a entrada do flat: records que dividiam o
/// valor antigo não mudam.
pub unsafe fn api_get_flat_scalar(name: &str) -> Option<u32> {
    api_get_flat_scalar_by_id(tweak_db_id(name))
}

/// Variante por ID — núcleo por trás de `TweakDBManager`-like `GetFlat` escalar.
pub unsafe fn api_get_flat_scalar_by_id(id: u64) -> Option<u32> {
    let t = singleton()?;
    let fv = get_flat_value_by_id(t, id)?;
    Some((fv.add(0x08) as *const u32).read_unaligned())
}

/// `tweakxl-batch-commit`: aplica N sets de flat escalar ATOMICAMENTE (tudo-ou-nada). Resolve
/// TODOS os campos + guarda o valor ORIGINAL de cada um numa fase 1 (leitura pura, SEM escrever
/// nada ainda); se QUALQUER campo não resolver, aborta sem tocar em NENHUM (nem os que já tinham
/// resolvido) — diferente de aplicar um-por-um, onde um campo ruim no meio da lista deixaria os
/// anteriores já escritos. Devolve os pares (nome, valor_original) escritos com sucesso (pra o
/// caller poder reverter depois se quiser) ou o nome do campo que causou o abort.
pub unsafe fn batch_set_flat_scalar(ops: &[(&str, u32)]) -> Result<Vec<(String, u32)>, String> {
    let t = singleton().ok_or_else(|| "TweakDB singleton indisponível".to_string())?;
    let mut resolved: Vec<(*mut u8, String, u32)> = Vec::with_capacity(ops.len());
    for (name, _new_val) in ops {
        match get_flat_value(t, name) {
            Some(fv) => {
                let orig = (fv.add(0x08) as *const u32).read_unaligned();
                resolved.push((fv, name.to_string(), orig));
            }
            None => return Err(format!("campo '{name}' não resolveu — batch abortado, 0 escritas (nenhum campo anterior foi tocado)")),
        }
    }
    // fase 2: só chega aqui se TODOS os campos resolveram na fase 1 — agora é seguro escrever.
    let mut originals = Vec::with_capacity(resolved.len());
    for ((_, name, orig), (_, new_val)) in resolved.iter().zip(ops.iter()) {
        if escritor::gravar_escalar(tweak_db_id(name), *new_val as u64) {
            originals.push((name.clone(), *orig));
        }
    }
    Ok(originals)
}

/// Parseia o VALOR de um campo do `batchset` na MESMA convenção do `setflat` (não a do
/// `mkflat`): `0xHEX` = valor u32 (big-endian, o número literal), OU decimal int, OU float
/// (vira os bits f32). Ex.: `0xbf000000`/`-0.5` ambos dão o mesmo u32 0xbf000000. Assim
/// `batchset X=0xbf000000` e `setflat X 0xbf000000` escrevem IDÊNTICO — antes divergiam
/// (batchset lia como bytes LE = 0x000000bf, um footgun; a suíte de regressão pegou isso).
fn parse_scalar_val(s: &str) -> Option<u32> {
    s.strip_prefix("0x")
        .and_then(|x| u32::from_str_radix(x, 16).ok())
        .or_else(|| s.parse::<i64>().ok().map(|i| i as u32))
        .or_else(|| s.parse::<f32>().ok().map(f32::to_bits))
}

/// `batchset <f1>=<v1> <f2>=<v2> ...`:
/// prova `tweakxl-batch-commit` — aplica vários sets de uma vez, tudo-ou-nada. Cada valor usa a
/// convenção do `setflat` (0xHEX u32 / decimal / float). Loga o resultado; se ALGUM campo não
/// existir, confirma que NENHUM foi escrito (nem os válidos antes dele na lista).
pub unsafe fn batchset_cmd(pairs: &[String]) {
    let mut ops: Vec<(String, u32)> = Vec::new();
    for p in pairs {
        let Some((name, valstr)) = p.split_once('=') else {
            crate::log_dev!("[batchset] par mal-formado (esperado campo=valor): '{p}'");
            return;
        };
        let Some(val) = parse_scalar_val(valstr) else {
            crate::log_dev!("[batchset] valor inválido em '{p}' (use 0xHEX / decimal / float)");
            return;
        };
        ops.push((name.to_string(), val));
    }
    let ops_ref: Vec<(&str, u32)> = ops.iter().map(|(n, v)| (n.as_str(), *v)).collect();
    match batch_set_flat_scalar(&ops_ref) {
        Ok(originals) => {
            crate::log_dev!(
                "[batchset] TUDO escrito ({} campos) — originais salvos: {:?} — >>> BATCH COMMIT OK <<<",
                originals.len(),
                originals
            );
        }
        Err(msg) => {
            crate::log_dev!("[batchset] ABORTADO: {msg} — >>> ATOMICIDADE OK (nada parcial) <<<");
        }
    }
}

pub unsafe fn api_set_flat_scalar(name: &str, val: u32) -> bool {
    api_set_flat_scalar_by_id(tweak_db_id(name), val)
}

/// Variante por ID — núcleo por trás de `TweakDBManager.SetFlat(id, value: Variant)` pro caso
/// ESCALAR, pelo escritor único ([`escritor::gravar_escalar`]): valor próprio com a largura do tipo, nunca no `FlatValue` dividido.
/// Sem trava: quem chama do redscript já passou pela própria checagem do mod.
pub unsafe fn api_set_flat_scalar_by_id(id: u64, val: u32) -> bool {
    escritor::gravar_escalar(id, val as u64)
}

/// `setflat <nome> <hexval> [off]`: sobrescreve 4 bytes do valor.
/// off = offset do dado no FlatValue (default 0x08 = escalar). ⚠️ afeta records que
/// COMPARTILHAM o valor. Loga before/after.
pub unsafe fn write_flat(name: &str, val: u32, data_off: usize) {
    let t = match singleton() {
        Some(s) => s,
        None => return,
    };
    match get_flat_value(t, name) {
        None => crate::log_dev!("[flat] setflat '{name}': não achado"),
        Some(fv) => {
            let p = fv.add(data_off);
            let before = (p as *const u32).read_unaligned();
            (p as *mut u32).write_unaligned(val);
            let after = (p as *const u32).read_unaligned();
            crate::log_dev!(
                "[flat] setflat '{name}' +{data_off:#x}: {before:#x} -> {after:#x} (pediu {val:#x})"
            );
        }
    }
}

/// "0000803f" → [0x00,0x00,0x80,0x3f]. None se comprimento ímpar ou dígito inválido.
fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.is_empty() || s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// `mkflat <field> <donor> <hexbytes>`: SetFlat NÃO-escalar — cria um
/// FlatValue novo com os `hexbytes` crus (a vtable nativa vem do `donor`, um flat do MESMO tipo) e
/// aponta `field` pra ele. Prova end-to-end de create_flat_value + set_flat_offset: o ponteiro do
/// FlatValue MUDA (repoint) e o valor lido passa a ser o novo. Ex. (round-trip escalar f32=1.0):
///   mkflat Items.GrenadeIncendiarySticky.deepWaterDepth Items.GrenadeIncendiarySticky.deepWaterDepth 0000803f
pub unsafe fn mkflat_cmd(field: &str, donor: &str, hex: &str) {
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log_dev!("[flat] singleton indisponível");
            return;
        }
    };
    let data = match hex_bytes(hex) {
        Some(d) => d,
        None => {
            crate::log_dev!("[flat] hex inválido (ex.: 0000803f = f32 1.0)");
            return;
        }
    };
    let before = get_flat_value(t, field).map(|fv| fv as usize).unwrap_or(0);
    match set_flat_nonscalar(t, field, donor, &data, 8) {
        Some(off) => {
            let after = get_flat_value(t, field).map(|fv| fv as usize).unwrap_or(0);
            crate::log_dev!(
                "[flat] mkflat '{field}' donor='{donor}' {}B -> tdbOffset {off:#x} | FlatValue {before:#x} -> {after:#x}",
                data.len()
            );
        }
        None => crate::log_dev!("[flat] mkflat FALHOU (donor '{donor}' achável? buffer cheio?)"),
    }
}

/// `mkflatnovo <flat> <flat-doador> <hex>` (2026-09-11): cria um flat que o record não tem,
/// com valor de qualquer largura e o tipo do doador — ex.: `AttachmentSlots.Torso.parentSlot` (TweakDBID, 8 B) pro teste
/// do ArchiveXL `#53`. Quando o valor tem 8 B, relê como TweakDBID (que também confere o tipo herdado do doador).
#[cfg(feature = "devtools")]
pub unsafe fn mkflatnovo_cmd(flat: &str, donor: &str, hex: &str) {
    let Some(t) = singleton() else {
        crate::log_veredito("[flatnovo] singleton indisponível");
        return;
    };
    let Some(data) = hex_bytes(hex) else {
        crate::log_veredito("[flatnovo] hex inválido (ex.: 23aa68ee14000000 = TweakDBID AttachmentSlots.Head)");
        return;
    };
    let id = tweak_db_id(flat);
    let ok = add_extra_flat_bytes_by_id(t, id, tweak_db_id(donor), &data);
    let lido = if data.len() == 8 { get_flat_tdbid(t, id) } else { None };
    crate::log_veredito(&format!(
        "[flatnovo] '{flat}' ({id:#x}) doador='{donor}' {}B -> inserido={ok} lido_como_tdbid={lido:x?}",
        data.len()
    ));
}

// ===== CLONE PROBE (READ-ONLY) — confirma o layout vivo ANTES de qualquer mutação =====
// O clone usável precisa INSERIR no array de flats (SortedUniqueArray@+0x40). Inserir errado
// corrompe o TweakDB inteiro → este probe NÃO muta nada: confirma (1) cap vs size do array
// {entries@+0x40, capacity@+0x48, size@+0x4C} — descobre se há slack p/ inserir sem realloc;
// (2) que dá pra enumerar as props da classe do record pelos getters (`enum_record_getters`,
// funcs@cls+0x48 + parents) e reverter CName→nome; (3) que "source.<prop>" existe no flats array (achável
// por get_flat_value) → o tdbOffset que o clone vai REUSAR. Só depois disto se escreve o insert.

#[inline]
unsafe fn rd_u32_at(p: *const u8) -> u32 {
    (p as *const u32).read_unaligned()
}

/// Minúsculo só o 1º char (ASCII), sem tocar no resto. Espelha `ResolvePropertyName` do TweakXL
/// (Reflection.cpp:284-285: `propName[0] = tolower(propName[0])`).
fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_ascii_lowercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

/// Enumera os shortNames dos GETTERS de um record (funcs de instância @cls+0x48 E estáticas @+0x58;
/// count@off+0x08; shortName CName@func+0x10) subindo os parents (cls+0x10). RAIZ DO CLONE 0/0:
/// records NÃO têm CProperties — o array props@cls+0x28 vem VAZIO; as props do TweakDB DERIVAM dos
/// getters (fonte TweakXL Reflection.cpp:94 `for func in aType->funcs`). Mesmo padrão/offsets do
/// `rtti::resolve_in_class` (proxy genérico PROVADO in-game). Devolve shortNames crus, ord+dedup.
unsafe fn enum_record_getters(cls0: *mut c_void) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut cls_it = cls0;
    let mut guard = 0;
    while !cls_it.is_null() && guard < 64 {
        guard += 1;
        if !crate::memoria::is_readable(cls_it as *const c_void, 0x60) {
            break;
        }
        let clsb = cls_it as *const u8;
        for off in [0x48usize, 0x58usize] {
            let fp = rd_u64(clsb.add(off)) as *const u8;
            let n = rd_u32_at(clsb.add(off + 8));
            if fp.is_null() || n >= 20_000 || !crate::memoria::is_readable(fp as *const c_void, 8) {
                continue;
            }
            for i in 0..n as usize {
                if !crate::memoria::is_readable(fp.add(i * 8) as *const c_void, 8) {
                    break;
                }
                let f = rd_u64(fp.add(i * 8)) as *const u8;
                if f.is_null() || !crate::memoria::is_readable(f as *const c_void, 0x20) {
                    continue;
                }
                let sname = crate::cname::resolve_cname(rd_u64(f.add(0x10))); // shortName@+0x10
                if !sname.is_empty() && sname != "None" {
                    names.push(sname);
                }
            }
        }
        cls_it = rd_u64(clsb.add(0x10)) as *mut c_void; // parent
    }
    names.sort();
    names.dedup();
    names
}

/// Do shortName do getter -> o nome-de-prop que RESOLVE a um flat de `record`. Espelha o
/// `ResolvePropertyName` do TweakXL: minúsculo o 1º char; se o flat não existir, tenta o shortName
/// como-está (cobre getters já-minúsculos). Getters-helper (Get*Count/Item/Contains) e type/enum
/// getters não têm flat em nenhuma das duas formas -> None -> PULADOS (sem precisar do funcIndex-skip
/// do TweakXL). Devolve (propName, ptr-da-entry do flat do source).
#[cfg(feature = "devtools")]
unsafe fn resolve_getter_flat(t: *mut u8, record: &str, getter: &str) -> Option<(String, *mut u64)> {
    for name in [lower_first(getter), getter.to_string()] {
        if name.is_empty() {
            continue;
        }
        if let Some(ep) = find_flat_entry(t, &format!("{record}.{name}")) {
            return Some((name, ep));
        }
    }
    None
}

/// `cloneprobe <classe> <source> <novo>` — ex:
/// `cloneprobe gamedataWeaponItem_Record Items.Preset_Lexington_Default Items.BwmsCloneTest`.
/// READ-ONLY. Loga o header do array de flats + quantas props do source têm flat achável.
#[cfg(feature = "devtools")]
pub unsafe fn clone_probe(
    reg: &crate::rtti::Registry,
    class_name: &str,
    source: &str,
    new_name: &str,
) {
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log_dev!("[clone] singleton indisponível");
            return;
        }
    };
    // (1) header do SortedUniqueArray de flats (capacity@+0x48, size@+0x4C, o layout do SDK) e os dois campos crus.
    let (_ent, size, cap, _sp) = flats_header(t);
    let raw48 = rd_u32_at(t.add(FLATS_OFF + 0x08));
    let raw4c = rd_u32_at(t.add(FLATS_OFF + 0x0C));
    let fdb = rd_u64(t.add(FLAT_DATA_BUFFER_OFF));
    crate::log_dev!(
        "[clone] flats: +0x48={raw48} +0x4C={raw4c} -> size={size} cap={cap} slack={} fdb={fdb:#x}",
        cap as i64 - size as i64
    );
    // (2)+(3) enumera props da classe (+ parents) e procura o flat do source.
    let cls = reg.class_by_name(class_name);
    if cls.is_null() {
        crate::log_dev!("[clone] classe '{class_name}' NÃO achada no registry");
        return;
    }
    let getters = enum_record_getters(cls);
    let total = getters.len();
    let mut found = 0usize;
    let mut shown_ok = 0usize;
    let mut shown_no = 0usize;
    crate::log_dev!("[clone] {total} getters enumerados em '{class_name}' (+parents) via funcs@0x48");
    for g in &getters {
        match resolve_getter_flat(t, source, g) {
            Some((pname, _)) => {
                found += 1;
                if shown_ok < 8 {
                    shown_ok += 1;
                    let src_id = tweak_db_id(&format!("{source}.{pname}"));
                    let new_id = tweak_db_id(&format!("{new_name}.{pname}"));
                    crate::log_dev!(
                        "[clone]   getter '{g}' -> '{pname}': flat src ACHADO (src_id={src_id:#x} -> new_id={new_id:#x})"
                    );
                }
            }
            None => {
                if shown_no < 4 {
                    shown_no += 1;
                    crate::log_dev!("[clone]   getter '{g}': sem flat (helper/type getter — pulado)");
                }
            }
        }
    }
    crate::log_dev!(
        "[clone] PROBE '{source}' ({class_name}): {found}/{total} props com flat achável. Inserir {found} flats faria size {size}->{} (cap={cap}, {}). READ-ONLY: nada mutado.",
        size as usize + found,
        if (size as usize + found) <= cap as usize { "CABE no slack — insert SEM realloc" } else { "precisa CRESCER o array (realloc)" }
    );
}

// ===== CLONE COM HERANÇA (2026-07-02) — o INSERT que o clone_probe de-riscou =====
// Faithful port do TweakXL InheritFlats (Manager.cpp:510-532): cada prop-flat do clone
// (clone_id + ".prop") recebe o MESMO tdbOffset do flat do source → COMPARTILHA o valor
// (não copia; override depois faz copy-on-write via create_flat_value/set_flat_offset). Ordem =
// InheritFlats ANTES de CreateRecord (o native lê os flats p/ popular o record), igual ao C++.

/// (hash32, len8) = CHAVE de ordenação do TweakDBID (operator<). O tdbOffset (bits 40-63) NÃO
/// entra na ordenação. Mesma comparação do find_flat_entry.
#[inline]
fn id_key_lt(a: u64, b: u64) -> bool {
    let (ah, al) = ((a & 0xFFFF_FFFF) as u32, ((a >> 32) & 0xFF) as u8);
    let (bh, bl) = ((b & 0xFFFF_FFFF) as u32, ((b >> 32) & 0xFF) as u8);
    ah < bh || (ah == bh && al < bl)
}

/// `(entries, size, cap, &size)` do índice de flats, pelo layout do SDK. O "menor campo é o size" que vivia aqui empatava no índice
/// cheio (size == cap, o caso que pede grow) e o grow gravava o size no campo do capacity: o motor via size = cap novo e buscava
/// numa cauda de entradas zeradas.
unsafe fn flats_header(t: *mut u8) -> (*mut u64, usize, usize, *mut u32) {
    let entries = rd_u64(t.add(FLATS_OFF)) as *mut u64;
    let size_ptr = t.add(FLATS_SIZE_OFF) as *mut u32;
    let cap = (t.add(FLATS_CAP_OFF) as *const u32).read_unaligned() as usize;
    (entries, size_ptr.read_unaligned() as usize, cap, size_ptr)
}

/// A chave (hash+len) já existe no array? (binary-search READ-ONLY; dedup vs o que já está lá.)
unsafe fn flat_key_present(entries: *const u64, size: usize, key: u64) -> bool {
    let (mut lo, mut hi) = (0usize, size);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let e = entries.add(mid).read_unaligned();
        if id_key_lt(e, key) {
            lo = mid + 1;
        } else if id_key_lt(key, e) {
            hi = mid;
        } else {
            return true;
        }
    }
    false
}

/// mutex00 (`SharedSpinLock` @ t+0x20): no Mac a escrita é o bit 7 (`0x80`, `0x1000015f0`) e cada leitor soma 1 (`0x1000015c8`).
/// Exclusivo = CAS 0→0x80 com spin LIMITADO (não trava o jogo se o lock nunca liberar); soltar limpa o bit 7. O 0xFF/store 0 do
/// Windows deixava o +1 de um leitor do motor zerar o byte e entrar junto, e o −1 dele prendia a trava em 0xFF: o `CreateRecord`
/// seguinte girava em `0x1000015f0` (via `0x101732848`) para sempre (trava no menu de 2026-10-25, depois de um GROW do clone).
unsafe fn mutex00_lock(t: *mut u8) -> bool {
    let st = &*(t.add(0x20) as *const std::sync::atomic::AtomicU8);
    for i in 0..4_000_000u32 {
        if st.compare_exchange(0, crate::jogo::travas::TRAVA_ESCRITA, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return true;
        }
        if i & 511 == 511 {
            std::thread::yield_now();
        }
    }
    false
}
unsafe fn mutex00_unlock(t: *mut u8) {
    crate::jogo::travas::soltar_escrita(t.add(0x20));
}

/// RED4ext.SDK `#429` (`SharedSpinLock::LockShared`/`UnlockShared`, 2026-08-10) — a metade
/// LEITURA do mesmo lock já usado por `mutex00_lock`/`mutex00_unlock` (o lado ESCRITA, CAS
/// 0->0x80). CAS `currentState -> currentState+1` (NUNCA um `fetch_add` cru — isso colidiria com um
/// lock exclusivo concorrente no meio do incremento); rejeita com o bit 7 ligado (exclusivo já
/// detido, o mesmo teste do `0x1000015c8`). `UnlockShared` = decremento simples (`InterlockedExchangeAdd8(-1)`),
/// seguro sem CAS porque múltiplos leitores nunca colidem entre si e nenhum exclusivo pode estar
/// ativo enquanto QUALQUER leitor detém o lock. Hoje as leituras de flat (`get_flat_value_by_id`/
/// `find_flat_entry_by_id`) são 100% lock-free (divergência já documentada, item #339) — esta
/// dupla fecha o PRIMITIVO em si (a via segura oficial passa a existir), sem retrofitar os
/// call-sites de leitura já provados/em produção (mudança ampla de escopo maior, fora deste
/// fechamento).
unsafe fn mutex00_lock_shared(t: *mut u8) -> bool {
    let st = &*(t.add(0x20) as *const std::sync::atomic::AtomicU8);
    for i in 0..4_000_000u32 {
        let cur = st.load(Ordering::Relaxed);
        if cur & crate::jogo::travas::TRAVA_ESCRITA == 0 {
            let next = cur.wrapping_add(1);
            if st.compare_exchange(cur, next, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                return true;
            }
        }
        if i & 511 == 511 {
            std::thread::yield_now();
        }
    }
    false
}
unsafe fn mutex00_unlock_shared(t: *mut u8) {
    crate::jogo::travas::soltar_leitura((t.add(0x20)) as *mut u8);
}

/// Comando `mutexsharedtest` (read-only exceto pelo próprio byte do lock, restaurado ao fim):
/// prova o par LockShared/UnlockShared contra o SharedSpinLock real do TweakDB vivo (`t+0x20`).
pub unsafe fn mutex_shared_test_cmd(t: *mut u8) {
    let st = &*(t.add(0x20) as *const std::sync::atomic::AtomicU8);
    let before = st.load(Ordering::Relaxed);
    let s1 = mutex00_lock_shared(t);
    let after1 = st.load(Ordering::Relaxed);
    let s2 = mutex00_lock_shared(t);
    let after2 = st.load(Ordering::Relaxed);
    mutex00_unlock_shared(t);
    let after_rel1 = st.load(Ordering::Relaxed);
    mutex00_unlock_shared(t);
    let after_rel2 = st.load(Ordering::Relaxed);
    let excl = mutex00_lock(t);
    let after_excl = st.load(Ordering::Relaxed);
    let shared_while_excl = mutex00_lock_shared(t); // deve FALHAR (spin limitado, timeout)
    mutex00_unlock(t);
    let after_final = st.load(Ordering::Relaxed);
    crate::log_dev!(
        "[mutexsharedtest] before={before} lock1={s1}(state={after1}) lock2={s2}(state={after2}) unlock1(state={after_rel1}) unlock2(state={after_rel2}) exclusive={excl}(state={after_excl}) shared_enquanto_exclusivo(deve_ser_false)={shared_while_excl} final(deve_ser_0)={after_final}"
    );
}

/// Merge-insert de um lote ORDENADO (`news`, por chave, já sem colisão com o existente) num
/// SortedUniqueArray de `size` entries já ordenado — do FIM pro começo, IN-PLACE. `entries` DEVE ter
/// capacidade p/ `size + news.len()`. Mantém a ordenação por (hash,len). Extraída de inherit_flats_rt
/// p/ ser TESTÁVEL (é a lógica arriscada do clone: escrever no array vivo do TweakDB).
unsafe fn merge_insert_from_end(entries: *mut u64, size: usize, news: &[u64]) {
    let add = news.len();
    let (mut i, mut j, mut k) = (size as isize - 1, add as isize - 1, (size + add) as isize - 1);
    while j >= 0 {
        let nv = news[j as usize];
        // pega o novo se o velho acabou OU o novo >= velho (nunca ==: o chamador já filtra colisões).
        let take_new = i < 0 || !id_key_lt(nv, entries.add(i as usize).read_unaligned());
        if take_new {
            entries.add(k as usize).write_unaligned(nv);
            j -= 1;
        } else {
            entries.add(k as usize).write_unaligned(entries.add(i as usize).read_unaligned());
            i -= 1;
        }
        k -= 1;
    }
}

/// Insere `novas` (ordenadas por chave, únicas, ausentes do índice) no índice de flats (SortedUniqueArray em `FLATS_OFF`) sob
/// `mutex00` exclusivo — o lock que o motor toma para os flats (RED4ext TweakDB.hpp:150). `visto` = `(entries, size, cap)` que o
/// chamador leu ao filtrar `novas`; o grow aborta se o índice mudou desde então.
///
/// Com slack, merge no lugar. Sem slack, GROW do ÍNDICE, não do flatDataBuffer: as entradas apontam para valores que já existem
/// (heranças compartilham o tdbOffset) ou foram criados à parte, nenhum `FlatValue*` muda (o aviso de TweakDB.hpp:28 é sobre o
/// buffer de dados), e o motor acha flat por busca binária, sem guardar ponteiro do índice. Buffer NOSSO com folga de 256 e o
/// trailer do alocador copiado do atual (o padrão do mkarr/set_flat_array_u64); o antigo é vazado, como o motor faz
/// (TweakDB.hpp:30). Publica entries → cap → size: size por último torna as novas visíveis ao leitor sem trava.
///
/// `Ok(Some(cap novo))` se cresceu, `Ok(None)` no merge no lugar; `Err(motivo)` quando abortou sem publicar.
unsafe fn inserir_no_indice(t: *mut u8, visto: (*mut u64, usize, usize), novas: &[u64]) -> Result<Option<usize>, &'static str> {
    let (entries, size0, cap) = visto;
    let add = novas.len();
    if size0 + add <= cap {
        if !mutex00_lock(t) {
            return Err("mutex00 ocupado — abortado SEM mutar");
        }
        let (entries, size, cap, size_ptr) = flats_header(t);
        if size + add > cap {
            mutex00_unlock(t);
            return Err("slack sumiu sob o lock — abortado");
        }
        merge_insert_from_end(entries, size, novas);
        size_ptr.write_unaligned((size + add) as u32);
        mutex00_unlock(t);
        return Ok(None);
    }
    let cur_trailer = ((entries as u64) + cap as u64 * 8 + 7) & !7;
    if !crate::memoria::is_readable(cur_trailer as *const c_void, 8) {
        return Err("grow abortado: trailer de allocator do buffer atual ilegível");
    }
    let alloc_vft = rd_u64(cur_trailer as *const u8);
    let new_cap = size0 + add + 256;
    // new_cap entries (8 B) + o alocador na palavra seguinte, no buffer do próprio alocador do índice (o motor solta por ele ao descarregar o
    // TweakDB); o antigo vaza, porque leitor fora da trava pode estar nele.
    let new_entries = crate::jogo::listas::alocar_pelo_alocador(alloc_vft, (new_cap + 1) * 8, 8) as *mut u64;
    if new_entries.is_null() {
        return Err("grow abortado: o alocador do índice não alocou");
    }
    std::ptr::copy_nonoverlapping(entries as *const u64, new_entries, size0);
    new_entries.add(new_cap).write_unaligned(alloc_vft);
    merge_insert_from_end(new_entries, size0, novas);
    if !mutex00_lock(t) {
        return Err("mutex00 ocupado no grow — abortado (buffer novo vazado, inócuo)");
    }
    let (entries2, size2, cap2, _sp2) = flats_header(t);
    if entries2 != entries || size2 != size0 || cap2 != cap {
        mutex00_unlock(t);
        return Err("flats mudou sob o lock (outro writer?) — abortado SEM publicar");
    }
    (t.add(FLATS_OFF) as *mut u64).write_unaligned(new_entries as u64);
    (t.add(FLATS_CAP_OFF) as *mut u32).write_unaligned(new_cap as u32);
    (t.add(FLATS_SIZE_OFF) as *mut u32).write_unaligned((size0 + add) as u32);
    mutex00_unlock(t);
    Ok(Some(new_cap))
}

/// Classes `_Record` conhecidas + 1 record de amostra REAL cada (confirmado contra o tweakdb.bin
/// via `tweakdb-tool records --class <X>` — armas 1585/roupas 1483/itens 4042/veículos 1036/
/// granadas 102/programas 36 registros encontrados). Cobre os casos de uso mais comuns de mods
/// TweakXL reais. Lista NÃO-exaustiva (840 tipos de record existem no jogo) — candidatos novos só
/// entram aqui depois de confirmados contra o `.bin` real, não por suposição.
const KNOWN_RECORD_CLASSES: &[(&str, &str)] = &[
    ("gamedataWeaponItem_Record", "Items.Preset_Lexington_Default"),
    ("gamedataClothing_Record", "Items.Avg_10_Int_Netrunner_Face_1"),
    ("gamedataItem_Record", "Ammo.HandgunAmmo"),
    ("gamedataVehicle_Record", "Vehicle.PlayerCar"),
    ("gamedataGrenade_Record", "Items.CPO_Flashbang"),
    ("gamedataProgram_Record", "minigame_v2.DefaultItemMinigameHard_inline0"),
];

/// `xlautoclone <base> <novo>`: clona sem dizer a classe, como o `.yaml` do TweakXL (que nunca anota a classe do `$base`). É o mesmo
/// `clone_record_async`, que tira a classe do base vivo.
pub unsafe fn xlautoclone_cmd(base: &str, clone: &str) {
    clone_record_async(base, clone);
}

// ===== `tweakxl-pipeline-runtime`: os arquivos de tweak dos mods no TweakDB VIVO =====
// O parser puro `bwms_primitives::tweakxl` (o mesmo do `bwms tweak` offline) dá a `Vec<Op>` de cada arquivo; os arquivos juntam num
// `Conjunto` (`bwms_primitives::operacoes`) aplicado nas fases do commit do TweakXL (`aplicar_conjunto`): `$base` clona com a classe do
// record-base vivo (`clone_record_by_id`); `$type` usa o modelo curado de `KNOWN_RECORD_CLASSES` ou qualquer record vivo da classe; a
// re-herança pelo mapa de herança do jogo; as atribuições pelo `escritor` (`gravar_texto`, ou `criar_flat_tipado` pro flat que o arquivo
// declara com tipo); as mutações de array de qualquer tipo de elemento, calculadas todas antes de gravar.

/// Flats de um record vivo, pelos getters da classe dele (o mesmo caminho do clone).
pub(crate) unsafe fn flats_do_record(t: *mut u8, id: u64) -> Vec<u64> {
    let rec = get_record_instance(t, id);
    if rec.is_null() {
        return Vec::new();
    }
    let cls = crate::rtti::class_of(rec as *mut c_void);
    enum_record_getters(cls).iter().filter_map(|g| resolve_getter_flat_by_id(t, id, g)).map(|(_, ep)| tweak_db_id_chave(ep.read_unaligned())).collect()
}

/// Threads do `aplicar_conjunto` ainda sem terminar (a versão do TweakDB já subiu quando o contador desce). A recarga do TweakXL
/// espera zerar para o `OnApply` ver os flats reaplicados.
static ARQUIVOS_EM_APLICACAO: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(crate) fn arquivos_em_aplicacao() -> usize {
    ARQUIVOS_EM_APLICACAO.load(Ordering::Acquire)
}

/// A vez de cada conjunto de tweaks no TweakDB. A senha sai na thread que chama `aplicar_conjunto`, na ordem da chamada, e a thread do
/// conjunto espera a vez dela antes de tocar no TweakDB: um conjunto avulso (teste, stat) depois do dos mods vê o que ele gravou. As
/// threads acordam juntas da espera de 500 ms, e sem a vez, quem pega a trava primeiro é o escalonador.
pub(crate) struct FilaDeVez {
    estado: std::sync::Mutex<Vez>,
    liberada: std::sync::Condvar,
}

/// A próxima senha a sair, a senha da vez, e as senhas que saíram de cena antes da vez delas (a thread que não chegou a rodar).
struct Vez {
    proxima: u64,
    atual: u64,
    encerradas: std::collections::BTreeSet<u64>,
}

impl FilaDeVez {
    pub(crate) const fn nova() -> Self {
        Self {
            estado: std::sync::Mutex::new(Vez { proxima: 0, atual: 0, encerradas: std::collections::BTreeSet::new() }),
            liberada: std::sync::Condvar::new(),
        }
    }

    fn travar(&self) -> std::sync::MutexGuard<'_, Vez> {
        self.estado.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A senha seguinte, tirada na thread que chama: a ordem das senhas é a das chamadas.
    pub(crate) fn tirar(&self) -> SenhaDaVez<'_> {
        let mut v = self.travar();
        let numero = v.proxima;
        v.proxima += 1;
        SenhaDaVez { fila: self, numero }
    }
}

/// A vez de um arquivo. Ao sair de cena (aplicado, abortado no meio ou nunca rodado) passa a vez adiante: uma thread que não sobe não
/// trava os arquivos seguintes.
pub(crate) struct SenhaDaVez<'a> {
    fila: &'a FilaDeVez,
    numero: u64,
}

impl SenhaDaVez<'_> {
    /// Bloqueia até ser a vez desta senha.
    pub(crate) fn esperar(&self) {
        let mut v = self.fila.travar();
        while v.atual != self.numero {
            v = self.fila.liberada.wait(v).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

impl Drop for SenhaDaVez<'_> {
    fn drop(&mut self) {
        let mut v = self.fila.travar();
        if v.atual != self.numero {
            v.encerradas.insert(self.numero);
            return;
        }
        v.atual += 1;
        loop {
            let seguinte = v.atual;
            if !v.encerradas.remove(&seguinte) {
                break;
            }
            v.atual += 1;
        }
        drop(v);
        self.fila.liberada.notify_all();
    }
}

static VEZ_DOS_ARQUIVOS: FilaDeVez = FilaDeVez::nova();

/// Mapa de herança dos records do jogo, lido uma vez de `<jogo>/red4ext/bwms-heranca.yaml` (o `bwms tweak heranca` gera das fontes `.tweak`
/// do TweakDB). Sem o arquivo, a mudança num record do jogo não chega aos que herdam dele — o TweakXL sem o `InheritanceMap.dat` também não.
fn mapa_de_heranca() -> Option<&'static bwms_primitives::operacoes::MapaDeHeranca> {
    static MAPA: std::sync::OnceLock<Option<bwms_primitives::operacoes::MapaDeHeranca>> = std::sync::OnceLock::new();
    MAPA.get_or_init(|| {
        let caminho = std::path::Path::new(&crate::dylib_dir()?).join("bwms-heranca.yaml");
        let texto = std::fs::read_to_string(&caminho).ok()?;
        match bwms_primitives::operacoes::MapaDeHeranca::ler(&texto) {
            Ok(m) => {
                crate::log_dev!("[xlyaml] mapa de herança: {} base(s), {} par(es)", m.bases(), m.pares());
                Some(m)
            }
            Err(e) => {
                crate::log_dev!("[xlyaml] mapa de herança {}: {e}", caminho.display());
                None
            }
        }
    })
    .as_ref()
}

/// As perguntas da re-herança respondidas pelo TweakDB vivo, antes de qualquer gravação do conjunto: mesmo valor = mesmo offset no buffer.
struct ConsultaDoTweakDb(*mut u8);

impl bwms_primitives::operacoes::ConsultaDeHeranca for ConsultaDoTweakDb {
    fn record_existe(&self, record: &str) -> bool {
        unsafe { registro_existe(self.0 as *mut c_void, tweak_db_id(record)) != 0 }
    }
    fn flat_existe(&self, flat: &str) -> bool {
        unsafe { find_flat_entry_by_id(self.0, tweak_db_id(flat)).is_some() }
    }
    fn mesmo_valor(&self, a: &str, b: &str) -> bool {
        unsafe {
            match (find_flat_entry_by_id(self.0, tweak_db_id(a)), find_flat_entry_by_id(self.0, tweak_db_id(b))) {
                (Some(x), Some(y)) => entry_tdb_offset(x.read_unaligned()) == entry_tdb_offset(y.read_unaligned()),
                _ => false,
            }
        }
    }
    fn e_array(&self, flat: &str) -> bool {
        unsafe { escritor::tipo_do_valor(self.0, tweak_db_id(flat)).is_some_and(|t| t.starts_with("array:")) }
    }
    fn comeca_com(&self, base: &str, descendente: &str) -> bool {
        unsafe {
            match (escritor::elementos_do_array(self.0, tweak_db_id(base)), escritor::elementos_do_array(self.0, tweak_db_id(descendente))) {
                (Some((eb, b)), Some((ed, d))) => {
                    eb == ed && d.len() >= b.len() && b.iter().zip(&d).all(|(x, y)| escritor::elementos_iguais(&eb, x, y))
                }
                _ => false,
            }
        }
    }
}

/// O valor novo do array `flat` pela cadeia de mutações do conjunto, sem gravar: `(tipo do elemento, elementos)`. O `!remove-all` de cada
/// nível vira os elementos do flat daquele nível; elemento que não serve pro tipo e flat-fonte de `!*-from` ausente ou de outro tipo ficam
/// de fora, com linha no log (o TweakXL loga e segue).
unsafe fn mutacao_calculada(t: *mut u8, conjunto: &bwms_primitives::operacoes::Conjunto, flat: &str) -> Option<(String, Vec<Vec<u8>>)> {
    let (elem, alvo) = escritor::elementos_do_array(t, tweak_db_id(flat))?;
    let conv = |v: &String| {
        let b = escritor::elemento_em_bytes(&elem, v);
        if b.is_none() {
            crate::log_dev!("[xlyaml] {flat}: '{v}' não serve pro elemento '{elem}'");
        }
        b
    };
    let fonte = |src: &String| match escritor::elementos_do_array(t, tweak_db_id(src)) {
        Some((e, l)) if e == elem => l,
        _ => {
            crate::log_dev!("[xlyaml] {flat}: fonte '{src}' ausente ou não é array:{elem}");
            Vec::new()
        }
    };
    let mut niveis = Vec::new();
    for m in conjunto.cadeia(flat) {
        let mut remocoes: Vec<Vec<u8>> = m.remocoes.iter().filter_map(conv).collect();
        if m.remover_tudo {
            remocoes.extend(escritor::elementos_do_array(t, tweak_db_id(&m.flat)).map(|(_, l)| l).unwrap_or_default());
        }
        let pares = |l: &[(String, bool)]| l.iter().filter_map(|(v, u)| conv(v).map(|b| (b, *u))).collect::<Vec<_>>();
        niveis.push(bwms_primitives::operacoes::Nivel {
            remocoes,
            antepor: pares(&m.antepor),
            antepor_de: m.antepor_de.iter().map(fonte).collect(),
            anexar: pares(&m.anexar),
            anexar_de: m.anexar_de.iter().map(fonte).collect(),
        });
    }
    let novo = bwms_primitives::operacoes::mutar(alvo, &niveis, |a, b| escritor::elementos_iguais(&elem, a, b));
    Some((elem, novo))
}

/// Aplica um conjunto de arquivos de tweak no TweakDB VIVO numa thread PRÓPRIA (mesmo motivo do `clone_record_async`: tomar o lock do
/// TweakDB dentro do hook do executor deadlocka), nas fases do commit do TweakXL: records em ordem (cada um visível no índice antes do
/// seguinte), mutação do base nos clones e re-herança pelo mapa (perguntas sobre o TweakDB de antes), atribuições, mutações.
fn aplicar_conjunto(mut conjunto: bwms_primitives::operacoes::Conjunto) {
    use bwms_primitives::operacoes::Op;
    ARQUIVOS_EM_APLICACAO.fetch_add(1, Ordering::AcqRel);
    // Um conjunto por vez e na ordem da chamada (`FilaDeVez`): o clone/herança mexem no índice de flats (GROW sob o mutex00, que o motor
    // também usa), e em paralelo uma thread vê o cabeçalho mudar sob a trava e aborta a herança.
    let senha = VEZ_DOS_ARQUIVOS.tirar();
    std::thread::spawn(move || {
        escritor::com_origem_arquivo(|| {
            std::thread::sleep(std::time::Duration::from_millis(500)); // sai do hot-path do hook
            senha.esperar();
            unsafe {
                let reg = match crate::rtti::Registry::obtain() {
                    Some(r) => r,
                    None => return crate::log_dev!("[xlyaml] Registry indisponível"),
                };
                let t = match singleton() {
                    Some(t) => t,
                    None => return crate::log_dev!("[xlyaml] singleton indisponível"),
                };
                let (mut ok, mut fail) = (0usize, 0usize);
                for op in &conjunto.registros {
                    match op {
                        // `$base`: a classe sai do record-base vivo (`clone_record_by_id`), qualquer que seja — não de uma lista fixa.
                        // Base do `.tweak` sem ponto (`?Base|pacote.Base|…`): tipo de record cria do tipo; senão clona o 1º candidato vivo.
                        Op::Clone { record, base } if base.starts_with('?') => match base_do_tweak(&reg, &base[1..]) {
                            Some(BaseDoTweak::Tipo(classe)) => match criar_record_do_tipo(&reg, t, registro::nomear(record), &classe) {
                                Some((classe, m, _, _)) => {
                                    registro::criado(tweak_db_id(record));
                                    crate::log_dev!("[xlyaml] {record} : {classe} (tipo) pela estrutura de {m:#x} OK");
                                    ok += 1;
                                }
                                None => {
                                    crate::log_dev!("[xlyaml] {record} : {classe} (tipo): sem modelo vivo da classe");
                                    fail += 1;
                                }
                            },
                            Some(BaseDoTweak::Record(b)) if clone_record_by_id(&reg, registro::nomear(record), registro::nomear(&b)) => {
                                registro::clonado(tweak_db_id(record), tweak_db_id(&b));
                                crate::log_dev!("[xlyaml] {record} : {b} (clone) OK");
                                ok += 1;
                            }
                            _ => {
                                crate::log_dev!("[xlyaml] {record} : {base}: nem tipo de record nem record vivo, ou clone falhou");
                                fail += 1;
                            }
                        },
                        Op::Clone { record, base } => {
                            if clone_record_by_id(&reg, registro::nomear(record), registro::nomear(base)) {
                                registro::clonado(tweak_db_id(record), tweak_db_id(base));
                                crate::log_dev!("[xlyaml] $base {record} <- {base} OK");
                                ok += 1;
                            } else {
                                crate::log_dev!("[xlyaml] $base {record} <- {base}: clone falhou");
                                fail += 1;
                            }
                        }
                        // `$type`: modelo curado quando a classe tem um; senão, qualquer record vivo da classe.
                        Op::Create { record, class } => match criar_record_do_tipo(&reg, t, registro::nomear(record), class) {
                            Some((classe, m, padroes, sem)) => {
                                registro::criado(tweak_db_id(record));
                                crate::log_dev!("[xlyaml] $type {record} ({classe}) pela estrutura de {m:#x}, {padroes} flat(s) no padrão do tipo, {sem} sem padrão OK");
                                ok += 1;
                            }
                            None => {
                                crate::log_dev!("[xlyaml] $type {record} ({class}): sem modelo vivo da classe ou clone falhou");
                                fail += 1;
                            }
                        },
                        Op::Edit { .. } | Op::Criar { .. } => {}
                    }
                }
                conjunto.herdar_mutacoes_dos_clones();
                if let Some(mapa) = mapa_de_heranca() {
                    let n = conjunto.herdar_do_mapa(mapa, &ConsultaDoTweakDb(t));
                    crate::log_dev!("[xlyaml] herança do mapa: {n} flat(s) de records do jogo que herdam dos mudados");
                }
                let mut ids_por_classe = std::collections::HashMap::new();
                for a in &conjunto.atribuicoes {
                    let id = registro::nomear(&a.flat);
                    let feito = match &a.tipo {
                        Some(tipo) => escritor::criar_flat_tipado(id, tipo, &a.valor),
                        None => {
                            escritor::gravar_texto(id, &a.valor)
                                || tipo_do_flat_extra(&reg, t, &a.flat, &mut ids_por_classe)
                                    .is_some_and(|tipo| escritor::criar_flat_tipado(id, &tipo, &a.valor))
                        }
                    };
                    if feito {
                        crate::log_dev!("[xlyaml] {} = {} OK", a.flat, a.valor);
                        ok += 1;
                    } else {
                        crate::log_dev!("[xlyaml] {} = {}: não aplicado", a.flat, a.valor);
                        fail += 1;
                    }
                }
                // Mutações: todas calculadas antes da 1ª gravação — o `!remove-all` e a cadeia de herança leem o array de antes delas.
                let mut calculadas = Vec::new();
                for m in &conjunto.mutacoes {
                    let id = registro::nomear(&m.flat);
                    if find_flat_entry_by_id(t, id).is_none() {
                        if let Some(tipo) = m.tipo.as_deref().filter(|x| x.starts_with("array:")) {
                            escritor::criar_flat_tipado(id, tipo, "[]");
                        }
                    }
                    match mutacao_calculada(t, &conjunto, &m.flat) {
                        Some(r) => calculadas.push((m.flat.clone(), id, r)),
                        None => {
                            crate::log_dev!("[xlyaml] {}: mutação de array não aplicada (flat inexistente ou não é array)", m.flat);
                            fail += 1;
                        }
                    }
                }
                for (flat, id, (elem, elementos)) in calculadas {
                    if escritor::gravar_elementos(id, &elem, &elementos) {
                        crate::log_dev!("[xlyaml] {flat}: array:{elem} com {} elemento(s) OK", elementos.len());
                        ok += 1;
                    } else {
                        crate::log_dev!("[xlyaml] {flat}: gravação do array falhou");
                        fail += 1;
                    }
                }
                crate::log_dev!("[xlyaml] pipeline concluído: {ok} ok / {fail} não-aplicadas");
                // Stat novo (`$type: Stat` com `enumName` fora do enum) entra no sistema de stats depois do lote inteiro.
                stats::registrar_tardio();
            }
        });
        // A vez passa depois da versão do TweakDB subir (`com_origem_arquivo`): o conjunto seguinte vê o que este gravou.
        drop(senha);
        ARQUIVOS_EM_APLICACAO.fetch_sub(1, Ordering::AcqRel);
    });
}

/// `$dlc: EP1` (ver `bwms_primitives::tweakxl::Ctx`): chama a native global do jogo `IsEP1() -> Bool`
/// (`orphans.script:39008`, a mesma que o `TweakContext.hpp` do TweakXL usa) via `register::get_function`/`call_func`.
/// `None`/falha = `false`: um mod gateado em EP1 fica desligado em vez de aplicar sem checar.
unsafe fn check_is_ep1() -> bool {
    let Some(reg) = crate::registry() else { return false };
    let f = crate::register::get_function(reg, "IsEP1");
    if !crate::rtti::sane(f) {
        return false;
    }
    // O executor guarda `res` e `ret_type` no CStack (+0x30/+0x38) e os entrega ao handler como `aOut`/tipo; o handler de
    // IsEP1 (0x104d86204) grava o byte com `strb` sem ler o tipo, mas o contrato do executor é o tipo real (`ret_type_of`).
    let rf = crate::rtti::ResolvedFn { func: f, ret_type: crate::rtti::ret_type_of(f), is_static: true };
    match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[]) {
        Some(r) => crate::rtti::bool_do_retorno(&r),
        None => false,
    }
}

/// Versão do jogo instalado pro `$game`: o `CFBundleShortVersionString` do `Cyberpunk2077.app` (a mesma trinca que o RED4ext do Mac dá como
/// versão de produto), lido uma vez; sem o `Info.plist`, a de `bwms_primitives::tweakxl::GAME_VERSION`.
fn versao_do_jogo() -> (u32, u32, u32) {
    static VERSAO: std::sync::OnceLock<(u32, u32, u32)> = std::sync::OnceLock::new();
    *VERSAO.get_or_init(|| {
        crate::dylib_dir()
            .and_then(|d| {
                let plist = std::path::Path::new(&d).parent()?.join("Cyberpunk2077.app/Contents/Info.plist");
                bwms_primitives::tweakxl::versao_do_info_plist(&std::fs::read_to_string(plist).ok()?)
            })
            .unwrap_or(bwms_primitives::tweakxl::GAME_VERSION)
    })
}

/// As operações de um arquivo de tweak, sem tocar no jogo: os dois formatos de autoria viram a mesma lista, aplicada pelo mesmo caminho (e
/// pelo escritor único). O `.tweak` passa pelo `tweak_source`; o `.yaml`/`.yml`, pelo parser do TweakXL, com o `$dlc: EP1` avaliado por
/// `is_ep1` (só chamado no `.yaml`; na varredura do boot, sem o jogo, `|| false` pula os nós de EP1 e só acusa o erro de fora deles, que
/// derruba o arquivo nas duas instalações) e o `$game` pela versão do jogo instalado. `Err` = o erro de leitura ou de parse, com a etapa.
pub(crate) fn ops_do_arquivo(path: &str, is_ep1: impl FnOnce() -> bool) -> Result<Vec<bwms_primitives::operacoes::Op>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("não foi possível ler o arquivo: {e}"))?;
    let formato = std::path::Path::new(path).extension().and_then(|e| e.to_str()).and_then(bwms_primitives::formato_tweak);
    if formato == Some(bwms_primitives::FormatoTweak::Tweak) {
        return bwms_primitives::tweakxl::tweak_source::parse_com_origem(&text, path).map_err(|e| format!("o .tweak não parseou: {e}"));
    }
    let root = bwms_primitives::tweakxl::yaml::parse(&text).map_err(|e| format!("o YAML não parseou: {e}"))?;
    let root = bwms_primitives::tweakxl::template::expand(&root).map_err(|e| format!("o $instances não expandiu: {e}"))?;
    bwms_primitives::tweakxl::interpret_do_jogo(&root, path, is_ep1(), versao_do_jogo()).map_err(|e| format!("o tweak não virou operação: {e}"))
}

/// Os arquivos de tweak num conjunto só, na ordem dada (a de precedência do `mods_do_usuario::pipeline::apply_mod_tweaks`): no mesmo flat
/// vale a atribuição do último, e as mutações de array de todos se aplicam por cima dela, como no TweakXL.
pub(crate) fn aplicar_arquivos(caminhos: &[String]) {
    let mut ep1 = None;
    let mut lidos = Vec::new();
    for path in caminhos {
        match ops_do_arquivo(path, || *ep1.get_or_insert_with(|| unsafe { check_is_ep1() })) {
            Ok(o) if o.is_empty() => crate::log_dev!("[xlauto] '{path}': sem operações"),
            Ok(o) => {
                crate::log_dev!("[xlauto] '{path}': {} ops", o.len());
                lidos.push(o);
            }
            Err(e) => crate::log_dev!("[xlauto] '{path}': {e}"),
        }
    }
    if lidos.is_empty() {
        return;
    }
    crate::log_dev!("[xlauto] {} arquivo(s) num conjunto, aplicando…", lidos.len());
    aplicar_conjunto(bwms_primitives::operacoes::Conjunto::montar(lidos));
}

/// Um arquivo de tweak sozinho (os testes ao vivo e o stat do `stats`).
pub(crate) fn aplicar_arquivo(path: &str) {
    aplicar_arquivos(&[path.to_string()]);
}

/// `applyxlfile <caminho>`: lê+parseia um `.yaml`/`.tweak` e aplica no TweakDB vivo pelo mesmo caminho dos mods ([`aplicar_arquivo`]).
pub unsafe fn applyxlfile_cmd(path: &str) {
    aplicar_arquivo(path);
}

/// Clone fora do hook do executor (assíncrono, sempre em THREAD SEPARADA: o drain do cmd-channel e o handler de plugin rodam dentro do
/// hook, e tomar o lock do TweakDB ali deadlocka). Vai pelo mesmo caminho do `CloneRecord` dos scripts (`clone_record_by_id`): a classe sai
/// do base vivo e cada flat herdado fica no registro, então a mudança do base depois chega ao clone (TweakXL #22, `escritor::propagar`).
/// Usado por `clone_cmd`, `xlautoclone_cmd` (dev) e `clone_record_api` (C-ABI).
unsafe fn clone_record_async(source: &str, clone: &str) {
    let (s, n) = (source.to_string(), clone.to_string());
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        unsafe {
            let reg = match crate::rtti::Registry::obtain() {
                Some(r) => r,
                None => return crate::log_dev!("[clone] Registry indisponível"),
            };
            let (id, base) = (tweak_db_id(&n), tweak_db_id(&s));
            if clone_record_by_id(&reg, id, base) {
                registro::clonado(id, base);
            } else {
                crate::log_dev!("[clone] '{n}' a partir de '{s}': não clonou (motivo acima)");
            }
        }
    });
}

/// `clone <classe> <source> <novo>`: CLONE USÁVEL — herda os flats do
/// source (stats reais) e registra o record novo no TweakDB vivo. Ex.:
///   clone gamedataWeaponItem_Record Items.Preset_Lexington_Default Items.BwmsLexClone
pub unsafe fn clone_cmd(source: &str, clone: &str) {
    clone_record_async(source, clone);
}

/// O `tweakdb_clone_record` da C-ABI: a mesma operação de `clone_cmd` (assíncrona, thread separada).
pub unsafe fn clone_record_api(source: &str, clone: &str) {
    clone_record_async(source, clone);
}

/// Roda o dump UMA vez quando o marcador `~/.bwms-tdbdump` existe (dev). Chamado do
/// cp77_tick (em gameplay, TweakDB já carregada). Idempotente.
pub unsafe fn dump_once_if_marked() {
    if DUMPED.load(Ordering::Relaxed) {
        return;
    }
    let marked = crate::marcador_de_dev(".bwms-tdbdump");
    if !marked {
        return;
    }
    if DUMPED.swap(true, Ordering::Relaxed) {
        return;
    }
    dump_singleton();
}

// ===== `tweakxl-updaterecord` — UpdateRecord (2026-07-17) =====
// Achado por RE #2: NÃO há um TweakDB::UpdateRecord(id) native único. A invalidação é GLOBAL via
// um version-byte @TweakDB+0x160. Cada campo de record cacheia (na sua FlatConnection embutida) o
// tdbOffset resolvido + um version-byte (byte9). O accessor de campo (@0x10165c104) só RE-RESOLVE
// quando field.version != global(+0x160). Logo o "UpdateRecord effect" = bump o +0x160 → força todo
// campo a re-resolver na próxima leitura (endereço no link base 0x100000000).
const VERSION_BYTE_OFF: usize = 0x160; // TweakDB+0x160 = global flat-version byte

/// GetRecord via asm (o retorno Handle vai em x8/sret). Devolve o ponteiro da INSTÂNCIA do record
/// (handle+0x00), ou null.
pub(crate) unsafe fn get_record_instance(t: *mut u8, id: u64) -> *mut u8 {
    let addr = crate::rebase(crate::jogo::itens::TWEAKDB_REGISTRO_VM);
    if !crate::memoria::is_readable(addr as *const c_void, 4) {
        return std::ptr::null_mut();
    }
    let mut handle: [u64; 2] = [0, 0];
    core::arch::asm!(
        "blr {f}",
        f = in(reg) addr,
        in("x0") t,
        in("x1") id,
        in("x8") handle.as_mut_ptr(),
        clobber_abi("C"),
    );
    // O GetRecord devolve um Handle com +1 forte (o próprio motor solta o local depois de chamar, 0x1032c5cb0): só o ponteiro sai daqui, e o mapa
    // do TweakDB mantém o record vivo. Sem soltar, record de mod removido nunca é destruído e cada chamada por frame acumula referência.
    let instancia = handle[0] as *mut u8;
    crate::jogo::objetos::soltar_handle(&mut handle);
    instancia
}

// ===== `tweakxl-updaterecord` v2 (2026-07-17) — técnica REAL do RED4ext/TweakXL: CreateTDBRecord
// numa TweakDB "fake"/scratch (isolada, zerada) + Assign() do record fresco por cima do record JÁ
// VIVO. Achado por RE offline: fonte vendorizada `RED4ext.SDK/include/RED4ext/TweakDB-inl.hpp`
// (`UpdateRecord(gamedataTweakDBRecord*)`) + `HashMap.hpp`/`Containers/DynArray.hpp` (o truque do
// allocator: quando capacity==0, `HashMap::allocator`(u64)@+0x28 ou `DynArray::m_entries`@+0x00 EM
// SI guardam o VTABLE PTR do IAllocator — `GetAllocator()` faz `reinterpret_cast<IAllocator*>(&field)`,
// então virtual-dispatch lê o vtable ptr direto do próprio campo). `IAllocator` (`Memory/Allocators.hpp`)
// tem 7 métodos, SEM destructor virtual → Itanium/macOS não desloca o vtable (mesma ordem do Windows):
// Alloc@0x00, AllocAligned@0x08, Realloc@0x10, ReallocAligned@0x18, Free@0x20, sub_28@0x28,
// GetHandle@0x30. `TweakDB` inteiro = 0x168 bytes (RED4EXT_ASSERT_SIZE); só usamos `recordsByID`
// (HashMap@+0x58) e `recordsByType` (HashMap@+0x88) — resto fica ZERADO (== default-ctor de um
// `TweakDB fakeTweakDB;` real, mesma técnica do `FakeAllocator` da fonte). O crescimento da
// DynArray<Handle> DENTRO de recordsByType (por-tipo) é feito pelo PRÓPRIO MOTOR via rotina de
// realloc interna (achada por RE: símbolo mangled `DynArray<Handle<TweakDBRecord>>::MoveAfterReallocation`
// no literal pool de `0x1037cd264`) — não passa pelo nosso IAllocator fake, então só precisamos
// fornecer o allocator fake pros DOIS HashMaps de topo (`recordsByID`/`recordsByType`), não pros
// DynArrays de valor.
#[repr(C)]
struct AllocResult {
    memory: u64,
    size: u64,
}

unsafe extern "C" fn fake_alloc(this: *const c_void, size: u64) -> AllocResult {
    fake_alloc_aligned(this, size, 8)
}
unsafe extern "C" fn fake_alloc_aligned(_this: *const c_void, size: u64, align: u32) -> AllocResult {
    let align = (align as usize).max(8);
    let size_us = (size as usize).max(1);
    match std::alloc::Layout::from_size_align(size_us, align) {
        Ok(layout) => {
            let p = std::alloc::alloc_zeroed(layout);
            AllocResult { memory: p as u64, size }
        }
        Err(_) => AllocResult { memory: 0, size: 0 },
    }
}
unsafe extern "C" fn fake_realloc(_this: *const c_void, _alloc: *mut AllocResult, _size: u64) -> AllocResult {
    AllocResult { memory: 0, size: 0 } // não usado neste caminho (TweakDB scratch só cresce via AllocAligned)
}
unsafe extern "C" fn fake_realloc_aligned(
    _this: *const c_void,
    _alloc: *mut AllocResult,
    _size: u64,
    _align: u32,
) -> AllocResult {
    AllocResult { memory: 0, size: 0 }
}
unsafe extern "C" fn fake_free(_this: *const c_void, _alloc: *mut AllocResult) {
    // vaza de propósito (scratch descartado no fim da função; mesmo trade-off já aceito em
    // dynarray_push_ptr/mkarr_cmd) — não sabemos o layout exato pra desalocar com segurança aqui.
}
unsafe extern "C" fn fake_sub28(_this: *const c_void, _a1: *const c_void) {}
unsafe extern "C" fn fake_get_handle(_this: *const c_void) -> u32 {
    0
}

// Cast fn-ptr->u64 não é permitido em const-eval de `static` — monta 1x em runtime (OnceLock),
// endereço estável pelo resto do processo (nunca realocado depois do 1º build).
static FAKE_ALLOCATOR_VTABLE: std::sync::OnceLock<[u64; 7]> = std::sync::OnceLock::new();

fn fake_allocator_vtable_ptr() -> u64 {
    FAKE_ALLOCATOR_VTABLE
        .get_or_init(|| {
            [
                fake_alloc as *const () as usize as u64,
                fake_alloc_aligned as *const () as usize as u64,
                fake_realloc as *const () as usize as u64,
                fake_realloc_aligned as *const () as usize as u64,
                fake_free as *const () as usize as u64,
                fake_sub28 as *const () as usize as u64,
                fake_get_handle as *const () as usize as u64,
            ]
        })
        .as_ptr() as u64
}

const ADDR_ASSIGN_TYPE_VTBL_SLOT_OFF: usize = 0x58; // Windows SDK documenta +0x50; shift Itanium 2-dtor já provado no projeto.

/// `UpdateRecord` v2 — cria um record FRESCO (mesmo `class_name`/`name` de um record JÁ VIVO) numa
/// TweakDB isolada (scratch, zerada, com allocator fake só nos 2 HashMaps de topo) e faz
/// `Assign(existing, fresh)` via a vtable do NATIVE TYPE do record — mesma técnica do RED4ext SDK
/// (`TweakDB::UpdateRecord`), sem depender do version-byte (achado não-confiável, ver
/// `proofs/2026-07-17-tweakxl-updaterecord-tentativa-honesta.log`). Log verboso em cada passo (se crashar, o log diz até
/// onde chegou). Devolve `true` só se Assign foi chamado.
///
/// # Safety
/// Chama uma função nativa real (`CreateTDBRecord`) com um ponteiro pra memória NOSSA (não o
/// singleton vivo) — isolado do estado real do jogo até o passo final de Assign, que SIM mexe no
/// record vivo (mutação in-place, mesmo objeto). Rodar de thread separada (mesmo motivo do
/// `create_record_rt`: a native pode pegar um spinlock).
#[cfg(feature = "devtools")]
pub unsafe fn update_record_rt(class_name: &str, name: &str) -> bool {
    crate::log_dev!("[updaterec2] entrando: class='{class_name}' name='{name}'");
    let real_t = match singleton() {
        Some(s) => s,
        None => {
            crate::log_dev!("[updaterec2] singleton REAL indisponível — abortado");
            return false;
        }
    };
    let id = tweak_db_id(name);
    let existing = get_record_instance(real_t, id);
    if existing.is_null() || !crate::memoria::is_readable(existing as *const c_void, 0x40) {
        crate::log_dev!("[updaterec2] record vivo '{name}' (id={id:#x}) não achado — abortado");
        return false;
    }
    crate::log_dev!("[updaterec2] record vivo @{existing:p}");

    // nativeType do record vivo — via GetType virtual (vtbl+8), MESMO getter já usado em class_of.
    let native_type = crate::rtti::class_of(existing as *mut c_void);
    if !crate::memoria::is_readable(native_type as *const c_void, 0x60) {
        crate::log_dev!("[updaterec2] class_of(existing) ilegível — abortado");
        return false;
    }
    crate::log_dev!("[updaterec2] nativeType={native_type:p}");

    // 1) TweakDB scratch — 0x168 bytes zerados + vtable fake nos 2 HashMaps de topo.
    let mut fake: [u8; 0x168] = [0u8; 0x168];
    let vtable_ptr = fake_allocator_vtable_ptr();
    // recordsByID@+0x58, seu campo `allocator`(u64)@+0x28 -> absoluto +0x80.
    // recordsByType@+0x88, idem -> absoluto +0xB0.
    fake[0x80..0x88].copy_from_slice(&vtable_ptr.to_le_bytes());
    fake[0xB0..0xB8].copy_from_slice(&vtable_ptr.to_le_bytes());
    let fake_ptr = fake.as_mut_ptr();
    crate::log_dev!("[updaterec2] scratch TweakDB @{fake_ptr:p} (0x168B, allocator fake @{vtable_ptr:#x})");

    // 2) CreateTDBRecord(scratch, baseHash, id) — MESMO endereço/assinatura de create_record_rt,
    // mas alvo é o SCRATCH, não o singleton real.
    let type_hash = record_type_key(class_name);
    crate::log_dev!("[updaterec2] chamando CreateTDBRecord(scratch, typeHash={type_hash:#010x}, id={id:#x})...");
    criar_registro(fake_ptr as *mut c_void, type_hash, id);
    crate::log_dev!("[updaterec2] CreateTDBRecord retornou");

    // 3) sucesso? recordsByID.size (scratch+0x58+0x08=0x60) != 0.
    let rid_size = (fake_ptr.add(0x60) as *const u32).read_unaligned();
    crate::log_dev!("[updaterec2] scratch.recordsByID.size={rid_size}");
    if rid_size == 0 {
        crate::log_dev!("[updaterec2] CreateTDBRecord não inseriu nada no scratch — abortado (sem Assign)");
        return false;
    }

    // 4) achar o record fresco em scratch.recordsByType (HashMap<IType*,DynArray<Handle<IScriptable>>>
    // @+0x88), chave=native_type (ponteiro), hash=FNV1a32(&key,8) (HashMapHash<T,pointer>).
    let rt_base = fake_ptr.add(0x88);
    let rt_index = (rt_base as *const u64).read_unaligned() as *const u32; // indexTable
    let rt_cap = (rt_base.add(0x0C) as *const u32).read_unaligned();
    let rt_nodes = (rt_base.add(0x10) as *const u64).read_unaligned() as *const u8; // nodeList.nodes
    crate::log_dev!(
        "[updaterec2] scratch.recordsByType: indexTable={rt_index:p} cap={rt_cap} nodes={rt_nodes:p}"
    );
    if rt_index.is_null() || rt_nodes.is_null() || rt_cap == 0 {
        crate::log_dev!("[updaterec2] recordsByType vazio/ilegível após CreateTDBRecord — abortado (sem Assign)");
        return false;
    }
    let hashed_key = crate::jogo::mapas::hash_da_chave(native_type as u64);
    const NODE_STRIDE: usize = 0x20; // next(u32)+hashedKey(u32)+key(ptr8)+value(DynArray<Handle>=16B)
    let mut fresh_record: *mut c_void = std::ptr::null_mut();
    // Varre TODOS os buckets (não só o hash calculado) — o scratch só tem 1 entry (recordsByID.size==1
    // confirmado acima), então basta achar QUALQUER nó populado; robusto a qualquer divergência entre
    // o hash que calculamos e o que o motor usou de verdade (diagnóstico: loga se bateu ou não).
    'buckets: for b in 0..rt_cap {
        let mut idx = (rt_index.add(b as usize)).read_unaligned();
        let mut guard = 0;
        while idx != u32::MAX && guard < 64 {
            guard += 1;
            let node = rt_nodes.add(idx as usize * NODE_STRIDE);
            let node_hash = (node.add(4) as *const u32).read_unaligned();
            let node_key = (node.add(8) as *const u64).read_unaligned();
            let dyn_entries = (node.add(0x10) as *const u64).read_unaligned() as *const u8;
            let dyn_size = (node.add(0x10 + 0x0C) as *const u32).read_unaligned();
            crate::log_dev!(
                "[updaterec2] bucket={b} idx={idx} node_hash={node_hash:#x}(nosso={hashed_key:#x}) node_key={node_key:#x}(nativeType={:#x}) entries={dyn_entries:p} size={dyn_size}",
                native_type as u64
            );
            if !dyn_entries.is_null() && dyn_size > 0 {
                fresh_record = (dyn_entries as *const u64).read_unaligned() as *mut c_void;
                break 'buckets;
            }
            idx = (node.add(0) as *const u32).read_unaligned(); // next
        }
    }
    if fresh_record.is_null() || !crate::memoria::is_readable(fresh_record as *const c_void, 0x40) {
        crate::log_dev!("[updaterec2] record fresco não achado em recordsByType — abortado (sem Assign)");
        return false;
    }
    crate::log_dev!("[updaterec2] record FRESCO @{fresh_record:p}");

    // 5) Assign(existing, fresh) via vtable do nativeType, slot +0x58 (macOS).
    let type_vtable = (native_type as *const u64).read_unaligned();
    if type_vtable == 0 || !crate::memoria::is_readable(type_vtable as *const c_void, ADDR_ASSIGN_TYPE_VTBL_SLOT_OFF + 8) {
        crate::log_dev!("[updaterec2] vtable do nativeType ilegível — abortado (sem Assign)");
        return false;
    }
    let assign_slot = (type_vtable as *const u8).add(ADDR_ASSIGN_TYPE_VTBL_SLOT_OFF) as *const u64;
    let assign_fn = assign_slot.read_unaligned();
    if !crate::memoria::is_readable(assign_fn as *const c_void, 4) {
        crate::log_dev!("[updaterec2] Assign@vtbl+0x58 ilegível — abortado (sem Assign)");
        return false;
    }
    crate::log_dev!(
        "[updaterec2] chamando Assign(existing={existing:p}, fresh={fresh_record:p}) via vtbl+0x58={assign_fn:#x}..."
    );
    let assign: extern "C" fn(*mut c_void, *mut c_void, *mut c_void) =
        std::mem::transmute(assign_fn);
    assign(native_type, existing as *mut c_void, fresh_record);
    crate::log_dev!("[updaterec2] Assign retornou — UpdateRecord completo (sem crash)");
    true
}

/// `UpdateRecord(id: TweakDBID) -> Bool` — variante SEM `class_name` (a API redscript real do
/// TweakXL, `TweakDBManager.UpdateRecord`, só recebe o ID). Deriva o `class_name` que
/// `update_record_rt` precisa (pra `CreateTDBRecord` no scratch) a partir do PRÓPRIO record já
/// vivo: `get_record_instance(id)` -> `class_of` -> `type_name_hash` (CName do nome do tipo,
/// campo `+0x18` da CClass) -> `crate::cname::resolve_cname` (nomes de tipo RTTI reais são sempre
/// pool-registrados, então isso resolve de forma confiável — mesma técnica já usada em
/// `tramp_setfield`/`tramp_callplayer` pro NOME do método/campo, aqui aplicada ao nome da CLASSE).
pub unsafe fn update_record_by_id(id: u64) -> bool {
    let t = match singleton() {
        Some(s) => s,
        None => return false,
    };
    let existing = get_record_instance(t, id);
    if existing.is_null() || !crate::memoria::is_readable(existing as *const c_void, 0x40) {
        crate::log_dev!("[tdbmanager] UpdateRecord(id={id:#x}): record vivo não achado");
        return false;
    }
    let native_type = crate::rtti::class_of(existing as *mut c_void);
    let name_hash = crate::rtti::type_name_hash(native_type);
    let class_name = crate::cname::resolve_cname(name_hash);
    if class_name.is_empty() {
        crate::log_dev!("[tdbmanager] UpdateRecord(id={id:#x}): não resolveu o nome do tipo (hash={name_hash:#x})");
        return false;
    }
    // Só o id, sem o nome do record: `update_record_rt(class_name, name)` deriva o id do nome, então o corpo por id é o `update_record_core`.
    update_record_core(t, id, existing, native_type, &class_name)
}

/// Núcleo compartilhado entre `update_record_rt` (nome+nome) e `update_record_by_id` (só id):
/// CreateTDBRecord num TweakDB *scratch* (mesmo type) + Assign(existing, fresh). Ver
/// `update_record_rt` pro algoritmo comentado passo-a-passo (idêntico, só sem os `crate::log`
/// de cada etapa intermediária — aquele é a versão de prova/diagnóstico, esta é a de produção).
unsafe fn update_record_core(_t: *mut u8, id: u64, existing: *mut u8, native_type: *mut c_void, class_name: &str) -> bool {
    let mut fake: [u8; 0x168] = [0u8; 0x168];
    let vtable_ptr = fake_allocator_vtable_ptr();
    fake[0x80..0x88].copy_from_slice(&vtable_ptr.to_le_bytes());
    fake[0xB0..0xB8].copy_from_slice(&vtable_ptr.to_le_bytes());
    let fake_ptr = fake.as_mut_ptr();

    let type_hash = record_type_key(class_name);
    criar_registro(fake_ptr as *mut c_void, type_hash, id);

    let rid_size = (fake_ptr.add(0x60) as *const u32).read_unaligned();
    if rid_size == 0 {
        crate::log_dev!("[tdbmanager] UpdateRecord(id={id:#x}): CreateTDBRecord não inseriu no scratch");
        return false;
    }

    let rt_base = fake_ptr.add(0x88);
    let rt_index = (rt_base as *const u64).read_unaligned() as *const u32;
    let rt_cap = (rt_base.add(0x0C) as *const u32).read_unaligned();
    let rt_nodes = (rt_base.add(0x10) as *const u64).read_unaligned() as *const u8;
    if rt_index.is_null() || rt_nodes.is_null() || rt_cap == 0 {
        crate::log_dev!("[tdbmanager] UpdateRecord(id={id:#x}): recordsByType vazio após CreateTDBRecord");
        return false;
    }
    const NODE_STRIDE: usize = 0x20;
    let mut fresh_record: *mut c_void = std::ptr::null_mut();
    'buckets: for b in 0..rt_cap {
        let mut idx = (rt_index.add(b as usize)).read_unaligned();
        let mut guard = 0;
        while idx != u32::MAX && guard < 64 {
            guard += 1;
            let node = rt_nodes.add(idx as usize * NODE_STRIDE);
            let dyn_entries = (node.add(0x10) as *const u64).read_unaligned() as *const u8;
            let dyn_size = (node.add(0x10 + 0x0C) as *const u32).read_unaligned();
            if !dyn_entries.is_null() && dyn_size > 0 {
                fresh_record = (dyn_entries as *const u64).read_unaligned() as *mut c_void;
                break 'buckets;
            }
            idx = (node.add(0) as *const u32).read_unaligned();
        }
    }
    if fresh_record.is_null() || !crate::memoria::is_readable(fresh_record as *const c_void, 0x40) {
        crate::log_dev!("[tdbmanager] UpdateRecord(id={id:#x}): record fresco não achado em recordsByType");
        return false;
    }

    let type_vtable = (native_type as *const u64).read_unaligned();
    if type_vtable == 0 || !crate::memoria::is_readable(type_vtable as *const c_void, ADDR_ASSIGN_TYPE_VTBL_SLOT_OFF + 8) {
        crate::log_dev!("[tdbmanager] UpdateRecord(id={id:#x}): vtable do nativeType ilegível");
        return false;
    }
    let assign_slot = (type_vtable as *const u8).add(ADDR_ASSIGN_TYPE_VTBL_SLOT_OFF) as *const u64;
    let assign_fn = assign_slot.read_unaligned();
    if !crate::memoria::is_readable(assign_fn as *const c_void, 4) {
        crate::log_dev!("[tdbmanager] UpdateRecord(id={id:#x}): Assign@vtbl+0x58 ilegível");
        return false;
    }
    let assign: extern "C" fn(*mut c_void, *mut c_void, *mut c_void) = std::mem::transmute(assign_fn);
    assign(native_type, existing as *mut c_void, fresh_record);
    crate::log_dev!("[tdbmanager] UpdateRecord(id={id:#x}, type='{class_name}') -> OK");
    true
}

/// `tweakxl-updaterecord` v3 (2026-07-18) — ACHADO NOVO: as tentativas v1/v2 (2026-07-17) tentaram
/// ler o campo mudado via `find_property_in_class` (falhou: flats não são CProperty) e via scan de
/// FlatConnection por offset fixo dentro da memória do record (falhou: `Assign` reorganiza a área
/// de conexões cacheadas, offset não é estável) — ambas NUNCA tentaram o caminho mais direto: um
/// RECORD do TweakDB expõe GETTERS NATIVOS reais por campo (`Item_Record.FriendlyName()`,
/// `Grenade_Record.DeepWaterDepth()` — confirmado em `redscript-src/orphans.script:27450`, MESMO
/// campo já usado o projeto inteiro pra testar SetFlat escalar). Esses getters são funções RTTI
/// normais, chamáveis via `rtti::call_func` (o MESMO `callf` já provado em dezenas de gaps hoje) —
/// é o "GetRecord lê o campo mudado" do `proof_needed` LITERAL, não uma reinterpretação. Round-trip:
/// chama o getter (baseline) -> repointa o flat (novo FlatValue por `create_flat_value` + `set_flat_offset`)
/// -> chama o getter DE NOVO SEM UpdateRecord (log, não decide nada sozinho) -> chama
/// `update_record_rt` (CreateTDBRecord+Assign, já provado crash-free 3x) -> chama o getter uma 3ª vez.
/// Se a 2ª chamada ainda mostra o valor VELHO e a 3ª mostra o NOVO, é a prova literal do gap.
#[cfg(feature = "devtools")]
pub unsafe fn prove_updaterecord_v3(class_name: &str, record_name: &str, getter_name: &str, prop_name: &str, new_val: f32) {
    crate::log_dev!(
        "[updaterec3] ==== round-trip via GETTER NATIVO: {record_name}.{getter_name}() (flat={prop_name}) -> {new_val} ===="
    );
    let reg = match crate::rtti::Registry::obtain() {
        Some(r) => r,
        None => {
            crate::log_dev!("[updaterec3] Registry indisponível — abortado");
            return;
        }
    };
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log_dev!("[updaterec3] singleton indisponível — abortado");
            return;
        }
    };
    let id = tweak_db_id(record_name);
    let existing = get_record_instance(t, id);
    if existing.is_null() || !crate::memoria::is_readable(existing as *const c_void, 0x40) {
        crate::log_dev!("[updaterec3] record '{record_name}' não achado — abortado");
        return;
    }
    crate::log_dev!("[updaterec3] record vivo @{existing:p}");

    let rf = match crate::rtti::resolve_func(&reg, class_name, getter_name) {
        Some(rf) => rf,
        None => {
            crate::log_dev!("[updaterec3] getter '{class_name}.{getter_name}' não resolveu — abortado");
            return;
        }
    };

    let read_via_getter = |tag: &str| -> Option<f32> {
        let ret = crate::rtti::call_func(&rf, existing as *mut c_void, &[]);
        match ret {
            Some(buf) => {
                let v = f32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                crate::log_dev!("[updaterec3] {tag}: {getter_name}() -> {v}");
                Some(v)
            }
            None => {
                crate::log_dev!("[updaterec3] {tag}: {getter_name}() chamada falhou (call_func None)");
                None
            }
        }
    };

    let baseline = read_via_getter("1) ANTES de qualquer mutação (baseline)");

    let full_flat = format!("{record_name}.{prop_name}");
    let new_off = match create_flat_value(t, &full_flat, &new_val.to_le_bytes(), 8) {
        Some(o) => o as u32,
        None => {
            crate::log_dev!("[updaterec3] create_flat_value falhou — abortado");
            return;
        }
    };
    if !set_flat_offset(t as *const u8, &full_flat, new_off) {
        crate::log_dev!("[updaterec3] set_flat_offset (repoint do índice global) falhou — abortado");
        return;
    }
    crate::log_dev!("[updaterec3] 2) índice global repontado -> offset {new_off:#x} (FlatValue novo = {new_val})");

    let sem_update = read_via_getter("3) getter SEM UpdateRecord ainda (deve ser igual ao baseline se o getter cacheia)");

    let updated = update_record_rt(class_name, record_name);
    crate::log_dev!("[updaterec3] 4) update_record_rt (CreateTDBRecord+Assign) retornou {updated}");

    let com_update = read_via_getter("5) getter COM UpdateRecord (deve ser new_val se o mecanismo funciona)");

    let bateu = updated
        && sem_update.is_some_and(|v| (v - baseline.unwrap_or(f32::NAN)).abs() < 0.001)
        && com_update.is_some_and(|v| (v - new_val).abs() < 0.001);
    crate::log_dev!(
        "[updaterec3] VEREDITO: {}",
        if bateu {
            ">>> UPDATERECORD V3 PROVADO: getter nativo leu o valor VELHO antes do UpdateRecord e o NOVO só depois <<<"
        } else if com_update.is_some_and(|v| (v - new_val).abs() < 0.001) && sem_update.is_some_and(|v| (v - new_val).abs() < 0.001) {
            "getter já mostrava o valor novo MESMO SEM UpdateRecord — o getter não cacheia (lê o índice global direto); UpdateRecord não é observável por este caminho"
        } else {
            "NÃO bateu — ver os valores acima (baseline/sem-update/com-update)"
        }
    );
}

// ===== ArchiveXL #59 (`PENDENCIAS-UNIFICADAS.md`, `TweakDB.hpp`): `GetRecords<R>() -> DynArray
// <Handle<R>>` — lista TODOS os records de um TIPO. Layout `recordsByID: HashMap<TweakDBID,
// Handle<IScriptable>>` @TweakDB+0x58, CONFIRMADO byte-a-byte contra o header REAL vendorizado
// (`RED4ext.SDK/include/RED4ext/HashMap.hpp` + `Memory/SharedPtr.hpp`, lidos nesta sessão, não
// suposição): `indexTable*@+0x00, size@+0x08, capacity@+0x0C, nodeList{nodes*@+0x10,
// cap@+0x18, stride@+0x1C, nextIdx@+0x20, size@+0x24}, allocator@+0x28`; `Node{next(u32)@0,
// hashedKey(u32)@4, key:TweakDBID(8B)@8, value:Handle<IScriptable>{instance*@0x10,
// refCount*@0x18}}` — stride TOTAL = 0x20 (32B). Read-only puro (nunca escreve).

/// Enumera `recordsByID` inteiro, filtrando por CLASSE EXATA (via `rtti::class_of` no ponteiro
/// `instance` de cada Handle, comparado ao `CClass*` resolvido de `class_name`). Devolve pares
/// (TweakDBID, instance ptr). `limit` corta o total de nós visitados (proteção contra loop
/// gigante em capacity absurda); `None` se o singleton/recordsByID não estiver populado.
pub unsafe fn get_records_of_class(t: *mut u8, reg: &crate::rtti::Registry, class_name: &str, limit: usize) -> Option<Vec<(u64, *mut c_void)>> {
    registros_da_classe(t, reg, class_name, limit, usize::MAX)
}

/// O 1º record da classe, na mesma ordem de baldes do `get_records_of_class` (o mesmo id que o `.first()` dele): o `$type` sem classe curada só
/// precisa de um modelo, e a varredura inteira são ~193 mil nós.
pub(crate) unsafe fn primeiro_registro_da_classe(t: *mut u8, reg: &crate::rtti::Registry, class_name: &str) -> Option<u64> {
    registros_da_classe(t, reg, class_name, usize::MAX, 1)?.first().map(|(id, _)| *id)
}

/// Nome da classe de record no RTTI a partir do que o mod escreve: o completo (`gamedataAttachmentSlot_Record`), o curto (`AttachmentSlot`) ou
/// o curto com o sufixo (`AttachmentSlot_Record`, o que o `RegisterOutfitSlots` do Equipment-EX passa ao `CreateRecord`).
pub(crate) fn classe_de_record(nome: &str) -> String {
    let miolo = nome.strip_prefix("gamedata").unwrap_or(nome);
    format!("gamedata{}_Record", miolo.strip_suffix("_Record").unwrap_or(miolo))
}

/// Record `id` novo da classe `classe` (qualquer forma do [`classe_de_record`]) com a estrutura de flats de um record vivo dela (o modelo
/// curado quando a classe tem um) e cada flat no valor padrão do tipo: o `$type` dos arquivos de mod e o `CreateRecord` do script. O
/// `CreateRecord` do motor sozinho põe o record no mapa sem flat nenhum, e o `SetFlat` seguinte não tem onde gravar.
/// `(classe resolvida, modelo, flats no padrão, flats sem padrão)`.
enum BaseDoTweak {
    Tipo(String),
    Record(String),
}

/// A base `Base|pacote.Base|…` de um `X : Base` do `.tweak` como o `RedReader::ResolveGroupState`: o 1º candidato como tipo de record
/// (`Clothing`, `Clothing_Record`, `gamedataClothing_Record`); senão o 1º candidato que é record vivo.
unsafe fn base_do_tweak(reg: &crate::rtti::Registry, candidatos: &str) -> Option<BaseDoTweak> {
    let mut lista = candidatos.split('|');
    let primeiro = lista.next()?;
    let classe = classe_de_record(primeiro);
    if !reg.class_by_name(&classe).is_null() {
        return Some(BaseDoTweak::Tipo(classe));
    }
    std::iter::once(primeiro).chain(lista).find(|c| record_existe(tweak_db_id(c))).map(|c| BaseDoTweak::Record(c.to_string()))
}

/// A classe do record em linha marcado `@Pai.prop` (mapa sem `$type`/`$base` sob a propriedade, `bwms_primitives::tweakxl`): a do record que
/// `Pai.prop` já aponta (o clone herda a referência) ou, sem ela, o tipo de retorno do método de mesmo nome na classe do pai (`vehicleUIData`
/// → `VehicleUIData()` → `whandle:gamedataVehicleUIData_Record`), a mesma reflexão de onde o TweakXL tira o `foreignType`.
unsafe fn classe_estrangeira(reg: &crate::rtti::Registry, t: *mut u8, prop: &str) -> Option<String> {
    let classe_de = |inst: *mut u8| {
        (!inst.is_null()).then(|| crate::cname::resolve_cname(crate::rtti::type_name_hash(crate::rtti::class_of(inst as *mut c_void))))
    };
    let apontado = (escritor::tipo_do_valor(t as *const u8, tweak_db_id(prop)).as_deref() == Some("TweakDBID"))
        .then(|| get_flat_value_by_id(t, tweak_db_id(prop)).map(|fv| (fv.add(0x08) as *const u64).read_unaligned()))
        .flatten()
        .filter(|v| *v != 0);
    if let Some(c) = apontado.and_then(|id| classe_de(get_record_instance(t, id))) {
        return Some(c);
    }
    let (pai, nome) = prop.rsplit_once('.')?;
    let classe_pai = classe_de(get_record_instance(t, tweak_db_id(pai)))?;
    let mut metodo = nome.to_string();
    metodo[..1].make_ascii_uppercase();
    // Propriedade-array devolve a lista por `out` (`ItemStock(out [wref<VendorItem_Record>]) -> Void`): o tipo do elemento sai do
    // `Get<Prop>Item(index)`.
    [metodo.clone(), format!("Get{metodo}Item")].iter().find_map(|m| {
        let f = crate::rtti::resolve_func(reg, &classe_pai, m)?;
        if f.ret_type.is_null() {
            return None;
        }
        let tipo = crate::cname::resolve_cname(crate::rtti::type_name_hash(f.ret_type));
        let c = tipo.trim_start_matches("array:").trim_start_matches("whandle:").trim_start_matches("handle:");
        c.starts_with("gamedata").then(|| c.to_string())
    })
}

/// O tipo do flat extra `<record>.<prop>` que ainda não existe (o que a classe não declara e o script lê por nome: `IPrereq.isInState`,
/// `Effector.animFeatureName`; o `ExtraFlats` do TweakXL): o do mesmo flat na origem vanilla do clone, senão o do 1º record vivo da mesma
/// classe que o tenha. O TweakDB do jogo é a fonte (`tweakdb.bin` 2.5: `isInState` Bool em 421 prereqs, `amplitudeWeight` Float em 15
/// effectors). `ids_por_classe` guarda a varredura de cada classe durante a aplicação.
unsafe fn tipo_do_flat_extra(
    reg: &crate::rtti::Registry,
    t: *mut u8,
    flat: &str,
    ids_por_classe: &mut std::collections::HashMap<String, Vec<u64>>,
) -> Option<String> {
    let (record, prop) = flat.rsplit_once('.')?;
    let sufixo = format!(".{prop}");
    let rid = tweak_db_id(record);
    let no = |base: u64| escritor::tipo_do_valor(t as *const u8, tweak_db_id_derive(tweak_db_id_chave(base), &sufixo));
    if let Some(tipo) = registro::pai_original(rid).and_then(no) {
        return Some(tipo);
    }
    let inst = get_record_instance(t, rid);
    if inst.is_null() {
        return None;
    }
    let classe = crate::cname::resolve_cname(crate::rtti::type_name_hash(crate::rtti::class_of(inst as *mut c_void)));
    if !ids_por_classe.contains_key(&classe) {
        let ids = registros_da_classe(t, reg, &classe, usize::MAX, usize::MAX).unwrap_or_default().into_iter().map(|(id, _)| id).collect();
        ids_por_classe.insert(classe.clone(), ids);
    }
    ids_por_classe[&classe].iter().find_map(|id| no(*id))
}

pub(crate) unsafe fn criar_record_do_tipo(reg: &crate::rtti::Registry, t: *mut u8, id: u64, classe: &str) -> Option<(String, u64, usize, usize)> {
    let estrangeira = match classe.strip_prefix('@') {
        Some(prop) => Some(classe_estrangeira(reg, t, prop)?),
        None => None,
    };
    let classe = estrangeira.as_deref().unwrap_or(classe);
    // O `$type` nomeia um tipo de record: a classe de record primeiro, o nome como veio só se ela não existir (`PhotoModeSticker` também é
    // outra classe no RTTI, e era essa que se procurava).
    let record = classe_de_record(classe);
    let classe = if reg.class_by_name(&record).is_null() { classe.to_string() } else { record };
    let curado = KNOWN_RECORD_CLASSES.iter().find(|(c, _)| *c == classe).map(|(_, s)| tweak_db_id(s));
    let modelo = curado.or_else(|| primeiro_registro_da_classe(t, reg, &classe))?;
    if !clone_record_by_id(reg, id, modelo) {
        return None;
    }
    let (padroes, sem) = escritor::aplicar_padroes(&flats_do_record(t, id));
    Some((classe, modelo, padroes, sem))
}

unsafe fn registros_da_classe(
    t: *mut u8,
    reg: &crate::rtti::Registry,
    class_name: &str,
    limit: usize,
    max_achados: usize,
) -> Option<Vec<(u64, *mut c_void)>> {
    let target_cls = reg.class_by_name(class_name);
    if !crate::rtti::sane(target_cls) {
        crate::log_dev!("[getrecords] classe '{class_name}' não resolveu");
        return None;
    }
    let base = t.add(0x58);
    if !crate::memoria::is_readable(base as *const c_void, 0x30) {
        return None;
    }
    let index_table = rd_u64(base) as *const u32;
    let size = (base.add(0x08) as *const u32).read_unaligned();
    let capacity = (base.add(0x0C) as *const u32).read_unaligned();
    let nodes = rd_u64(base.add(0x10)) as *const u8;
    crate::log_dev!(
        "[getrecords] recordsByID: size={size} capacity={capacity} indexTable={index_table:p} nodes={nodes:p}"
    );
    if index_table.is_null() || nodes.is_null() || capacity == 0 || capacity > 5_000_000 {
        crate::log_dev!("[getrecords] recordsByID vazio/ilegível — abortado");
        return None;
    }
    if !crate::memoria::is_readable(index_table as *const c_void, capacity as usize * 4) {
        crate::log_dev!("[getrecords] indexTable ilegível");
        return None;
    }
    const NODE_STRIDE: usize = 0x20;
    let mut out = Vec::new();
    let mut visited = 0usize;
    'buckets: for b in 0..capacity {
        let mut idx = index_table.add(b as usize).read_unaligned();
        let mut guard = 0;
        while idx != u32::MAX && guard < 10_000 {
            guard += 1;
            visited += 1;
            if visited > limit {
                crate::log_dev!("[getrecords] limite de {limit} nós visitados atingido — parando cedo");
                break 'buckets;
            }
            let node = nodes.add(idx as usize * NODE_STRIDE);
            if !crate::memoria::is_readable(node as *const c_void, NODE_STRIDE) {
                break;
            }
            let key = rd_u64(node.add(0x08)); // TweakDBID
            let instance = rd_u64(node.add(0x10)) as *mut c_void; // Handle.instance
            if !instance.is_null() && crate::memoria::is_readable(instance as *const c_void, 8) {
                let cls = crate::rtti::class_of(instance);
                if cls == target_cls {
                    out.push((key, instance));
                    if out.len() >= max_achados {
                        break 'buckets;
                    }
                }
            }
            idx = (node as *const u32).read_unaligned(); // next
        }
    }
    crate::log_dev!(
        "[getrecords] '{class_name}': {} record(s) encontrados de {visited} nó(s) visitado(s) (size real={size})",
        out.len()
    );
    Some(out)
}

// RED4ext.SDK item #341 (`TweakDB::GetRecordsByType`/`TryGetRecordsByType`). Diferente de
// `get_records_of_class` (varre TODO `recordsByID`@+0x58, O(n) sobre 193354 nós, filtra por
// classe), este usa o índice DEDICADO `recordsByType: HashMap<IType*, DynArray<Handle<
// IScriptable>>>@+0x88` (layout já confirmado ao vivo em `update_record_rt`, contra um scratch
// — aqui aplicado pela 1ª vez contra o SINGLETON REAL) — lookup por bucket único (O(1)
// amortizado), sem varrer o HashMap inteiro. Mesmo NODE_STRIDE=0x20 (chave=`IType*` 8B@+0x08,
// valor=`DynArray<Handle>` 16B@+0x10: entries*@+0x10, cap(u32)@+0x18, size(u32)@+0x1C — cada
// entry do array é um `Handle<IScriptable>` de 16B, instance*@+0). Hash da chave = FNV1a32 dos
// 8 bytes do ponteiro (`HashMapHash<T*>`, item #367, mesma fórmula já usada em `update_record_rt`).
#[cfg(feature = "devtools")]
pub unsafe fn get_records_by_type(t: *mut u8, native_type: *mut c_void, limit: usize) -> Option<Vec<*mut c_void>> {
    if native_type.is_null() {
        return None;
    }
    let base = t.add(0x88);
    if !crate::memoria::is_readable(base as *const c_void, 0x30) {
        return None;
    }
    let index_table = rd_u64(base) as *const u32;
    let size = (base.add(0x08) as *const u32).read_unaligned();
    let capacity = (base.add(0x0C) as *const u32).read_unaligned();
    let nodes = rd_u64(base.add(0x10)) as *const u8;
    crate::log_dev!(
        "[getrecbytype] recordsByType: size={size} capacity={capacity} indexTable={index_table:p} nodes={nodes:p}"
    );
    if index_table.is_null() || nodes.is_null() || capacity == 0 || capacity > 1_000_000 {
        crate::log_dev!("[getrecbytype] recordsByType vazio/ilegível — abortado");
        return None;
    }
    if !crate::memoria::is_readable(index_table as *const c_void, capacity as usize * 4) {
        crate::log_dev!("[getrecbytype] indexTable ilegível");
        return None;
    }
    const NODE_STRIDE: usize = 0x20;
    let hashed_key = crate::jogo::mapas::hash_da_chave(native_type as u64);
    let bucket = (hashed_key % capacity) as usize;
    let mut idx = index_table.add(bucket).read_unaligned();
    let mut guard = 0;
    while idx != u32::MAX && guard < 10_000 {
        guard += 1;
        let node = nodes.add(idx as usize * NODE_STRIDE);
        if !crate::memoria::is_readable(node as *const c_void, NODE_STRIDE) {
            break;
        }
        let node_key = rd_u64(node.add(0x08));
        if node_key == native_type as u64 {
            let entries = rd_u64(node.add(0x10)) as *const u8;
            let dyn_size = (node.add(0x10 + 0x0C) as *const u32).read_unaligned();
            crate::log_dev!(
                "[getrecbytype] bucket={bucket} idx={idx} MATCH: entries={entries:p} size={dyn_size}"
            );
            if entries.is_null() || dyn_size == 0 {
                return Some(Vec::new());
            }
            if !crate::memoria::is_readable(entries as *const c_void, dyn_size as usize * 16) {
                crate::log_dev!("[getrecbytype] entries[] ilegível");
                return None;
            }
            let n = (dyn_size as usize).min(limit);
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let inst = rd_u64(entries.add(i * 16)) as *mut c_void;
                if !inst.is_null() {
                    out.push(inst);
                }
            }
            return Some(out);
        }
        idx = (node as *const u32).read_unaligned();
    }
    crate::log_dev!("[getrecbytype] bucket={bucket}: nenhum node com key={native_type:p} — 0 records");
    Some(Vec::new())
}

/// Comando `getrecbytype <classe> [limite]` — teste read-only de `get_records_by_type`, cross-
/// validado contra `get_records_of_class` (mesma classe deve dar a MESMA contagem por 2 vias
/// independentes: scan completo filtrado vs. lookup direto no índice por-tipo).
#[cfg(feature = "devtools")]
pub unsafe fn getrecbytype_cmd(reg: &crate::rtti::Registry, class_name: &str, limit: usize) {
    let Some(t) = singleton() else {
        return crate::log_dev!("[getrecbytype] singleton indisponível");
    };
    let native_type = reg.class_by_name(class_name);
    if !crate::rtti::sane(native_type) {
        return crate::log_dev!("[getrecbytype] classe '{class_name}' não resolveu");
    }
    match get_records_by_type(t, native_type as *mut c_void, limit) {
        None => {}
        Some(records) => {
            crate::log_dev!(
                "[getrecbytype] '{class_name}' (via recordsByType): {} record(s)",
                records.len()
            );
            for (i, ptr) in records.iter().take(5).enumerate() {
                crate::log_dev!("[getrecbytype]   [{i:02}] instance={ptr:p}");
            }
        }
    }
}

/// Busca o NODE (não só o Handle.instance) de `recordsByID` cuja chave bate `id` — mesmo full-scan
/// já provado de `get_records_of_class`, só com predicado por CHAVE em vez de filtro por classe.
/// Devolve o ponteiro do node inteiro (não só o instance) pra permitir sobrescrever o campo
/// Handle.instance@node+0x10 depois (usado por `create_record_alias_by_id`).
unsafe fn find_record_node_by_id(t: *mut u8, id: u64, limit: usize) -> Option<*mut u8> {
    let base = t.add(0x58);
    if !crate::memoria::is_readable(base as *const c_void, 0x30) {
        return None;
    }
    let index_table = rd_u64(base) as *const u32;
    let capacity = (base.add(0x0C) as *const u32).read_unaligned();
    let nodes = rd_u64(base.add(0x10)) as *const u8;
    if index_table.is_null() || nodes.is_null() || capacity == 0 || capacity > 5_000_000 {
        return None;
    }
    const NODE_STRIDE: usize = 0x20;
    let mut visited = 0usize;
    for b in 0..capacity {
        let mut idx = index_table.add(b as usize).read_unaligned();
        let mut guard = 0;
        while idx != u32::MAX && guard < 10_000 {
            guard += 1;
            visited += 1;
            if visited > limit {
                return None;
            }
            let node = nodes.add(idx as usize * NODE_STRIDE) as *mut u8;
            if !crate::memoria::is_readable(node as *const c_void, NODE_STRIDE) {
                break;
            }
            let key = rd_u64(node.add(0x08));
            if key == id {
                return Some(node);
            }
            idx = (node as *const u32).read_unaligned();
        }
    }
    None
}

// ArchiveXL `#59` (`TweakDB.hpp::CreateRecordAlias(recordID, aliasID)`) — item deprioritizado
// desde 2026-08-10 (cont.161) por causa de `recordsByID` observado em `size==capacity` (zero
// slack, growth manual pareceria arriscado). Achado (2026-08-10, cont.171): NÃO é preciso
// implementar growth/rehash à mão — `create_record_by_id` já usa a native `CreateTDBRecord`
// (`ADDR_CREATE_RECORD`, já PROVADA por `TweakDBManager.CreateRecord`/`CloneRecord`), que faz o
// INSERT com growth-safety do PRÓPRIO motor (o mesmo mecanismo que qualquer record novo do jogo
// usa). Estratégia: cria uma entrada NOVA pro `aliasID` (mesmo tipo do `recordID` fonte, via
// `CreateTDBRecord` — deixa o motor cuidar do growth), depois REPONTA o `Handle.instance` do node
// recém-inserido pro MESMO instance do record fonte (alias de verdade — dado compartilhado, não
// cópia) via 1 escrita de 8 bytes (não mexe em capacity/allocator/growth nenhum). A instância
// alocada pelo `CreateTDBRecord` pro alias fica órfã (vazada) — mesmo padrão já aceito em ~10
// itens do catálogo RED4ext.SDK ("nunca desaloca").
pub unsafe fn create_record_alias_by_id(t: *mut u8, reg: &crate::rtti::Registry, record_id: u64, alias_id: u64) -> bool {
    let Some(source_node) = find_record_node_by_id(t, record_id, 5_000_000) else {
        crate::log_dev!("[createalias] record fonte {record_id:#x} não achado em recordsByID");
        return false;
    };
    let source_instance = rd_u64(source_node.add(0x10)) as *mut c_void;
    if source_instance.is_null() || !crate::memoria::is_readable(source_instance as *const c_void, 8) {
        crate::log_dev!("[createalias] instance fonte nula/ilegível");
        return false;
    }
    let cls = crate::rtti::class_of(source_instance);
    let name_hash = crate::rtti::type_name_getname(cls);
    let class_name = crate::cname::resolve_cname(name_hash);
    if class_name.is_empty() {
        crate::log_dev!("[createalias] classe fonte não resolveu nome");
        return false;
    }
    let type_hash = record_type_key(&class_name);
    if !create_record_by_id(type_hash, alias_id) {
        crate::log_dev!("[createalias] CreateTDBRecord(aliasID={alias_id:#x}, type='{class_name}') falhou");
        return false;
    }
    let Some(alias_node) = find_record_node_by_id(t, alias_id, 5_000_000) else {
        crate::log_dev!("[createalias] node do alias recém-criado não achado — abortado (sem repoint)");
        return false;
    };
    // Repoint: alias.Handle.instance = source.Handle.instance (dado COMPARTILHADO, não copiado).
    (alias_node.add(0x10) as *mut u64).write_unaligned(source_instance as u64);
    let readback = rd_u64(alias_node.add(0x10)) as *mut c_void;
    let ok = readback == source_instance;
    crate::log_dev!(
        "[createalias] aliasID={alias_id:#x} -> recordID={record_id:#x} (type='{class_name}') repoint={ok} (source={source_instance:p} readback={readback:p})"
    );
    let _ = reg;
    ok
}

// TweakXL `#34` (`TweakChangeset::ReinheritFlat`, 2026-08-10) — no C++ real é só uma QUEUE
// (`m_reinheritedProps[aFlatId] = {sourceId, appendix}`, aplicado depois no Commit do changeset,
// `TweakChangeset.cpp:21-31`). Mesma simplificação já documentada+aceita pro resto do
// `TweakDBBatch` (BWMS não faz staging, aplica na hora — ver `ponte-tweakdbbatch.reds`). Repoint em vez
// de insert: `aFlatId` (o flat do CLONE) já tem sua PRÓPRIA entry (herdada via CloneRecord/
// InheritProps) — reponta só os 24 bits de offset (bytes 5-7, mesma máscara já usada em
// `inherit_flats_rt`) pro offset do flat FONTE (`sourceId + "." + appendix`, derivado via
// `tweak_db_id_derive` — mesma telescopagem de CRC já provada), mantendo os 40 bits de chave
// (identidade) do próprio `aFlatId` intactos. 1 escrita de 8 bytes, sem grow/insert.
pub unsafe fn reinherit_flat_by_id(t: *mut u8, flat_id: u64, source_id: u64, appendix: &str) -> bool {
    let Some(dst_entry) = find_flat_entry_by_id(t as *const u8, flat_id) else {
        crate::log_dev!("[reinheritflat] flat alvo {flat_id:#x} não achado (precisa já existir — herdado via CloneRecord)");
        return false;
    };
    let source_flat_id = bwms_primitives::tweak_db_id_derive(source_id, &format!(".{appendix}"));
    let Some(src_entry) = find_flat_entry_by_id(t as *const u8, source_flat_id) else {
        crate::log_dev!(
            "[reinheritflat] flat fonte {source_flat_id:#x} (sourceId={source_id:#x} + '.{appendix}') não achado"
        );
        return false;
    };
    let src_val = src_entry.read_unaligned();
    let dst_val_before = dst_entry.read_unaligned();
    // Pelo escritor único: a mesma troca dos bytes 5-7, mais o diário e a subida da versão (o getter do record re-resolve).
    let escreveu = escritor::reapontar(flat_id, entry_tdb_offset(src_val));
    let readback = dst_entry.read_unaligned();
    let ok = escreveu
        && tweak_db_id_chave(readback) == tweak_db_id_chave(dst_val_before)
        && (readback & 0xFFFF_FF00_0000_0000) == (src_val & 0xFFFF_FF00_0000_0000);
    crate::log_dev!(
        "[reinheritflat] flatId={flat_id:#x} <- sourceId={source_id:#x}+'.{appendix}' (source_flat={source_flat_id:#x}): offset {:#08x} -> {:#08x} ok={ok}",
        (dst_val_before >> 40) & 0xFF_FFFF,
        (readback >> 40) & 0xFF_FFFF
    );
    ok
}

// TweakXL `#23` (`RegisterExtraFlat`, 2026-08-10) — adiciona um flat GENUINAMENTE NOVO (chave
// nunca vista) com storage PRÓPRIO (diferente de CloneRecord/ReinheritFlat, que sempre repontam
// pra storage já EXISTENTE — aqui aloca um `FlatValue` dedicado via `create_flat_value_by_id`,
// vtable clonada de um `donor` do MESMO tipo). Fonte real: só chamado de `MetadataImporter.cpp`
// (import de metadata `extraFlats.json`), sem tag YAML própria nem API redscript exposta —
// mesma filosofia de `#34`: capacidade PRÓPRIA do BWMS. Insert de 1 entry nova por `inserir_no_indice`,
// o mesmo do clone e da herança.
/// Insere um flat NOVO com valor de qualquer largura (4 B nos escalares de 32 bits, 8 no `TweakDBID` — o `.parentSlot` do
/// ArchiveXL `#53` — e no `CName`). O tipo do FlatValue vem do flat doador.
pub unsafe fn add_extra_flat_bytes_by_id(t: *mut u8, flat_id: u64, donor_type_flat_id: u64, data: &[u8]) -> bool {
    if find_flat_entry_by_id(t as *const u8, flat_id).is_some() {
        crate::log_dev!("[addflat] flat {flat_id:#x} já existe — use SetFlat pra sobrescrever, não AddFlat");
        return false;
    }
    let Some(off) = create_flat_value_by_id(t, donor_type_flat_id, data, 8) else {
        crate::log_dev!("[addflat] create_flat_value_by_id falhou (donorTypeFlat={donor_type_flat_id:#x} inválido? buffer cheio?)");
        return false;
    };
    inserir_flat_com_offset(t, flat_id, off as u32)
}

/// Põe no índice de flats a entrada nova `flat_id` → valor em `off` (o `InsertOrAssign` do TweakXL pra flat que não existia).
pub(crate) unsafe fn inserir_flat_com_offset(t: *mut u8, flat_id: u64, off: u32) -> bool {
    let off_u = off;
    let new_entry = tweak_db_id_chave(flat_id)
        | (((off_u >> 16) & 0xFF) as u64) << 40
        | (((off_u >> 8) & 0xFF) as u64) << 48
        | ((off_u & 0xFF) as u64) << 56;
    let (entries, size, cap, _sp) = flats_header(t);
    if entries.is_null() || size > 10_000_000 {
        crate::log_dev!("[addflat] array de flats inválido");
        return false;
    }
    match inserir_no_indice(t, (entries, size, cap), &[new_entry]) {
        Ok(Some(new_cap)) => {
            crate::log_dev!(
                "[addflat] GROW: flat NOVO {flat_id:#x} inserido (cap {cap}->{new_cap}, size {size}->{}) tdbOffset={off} ✓",
                size + 1
            );
            true
        }
        Ok(None) => {
            crate::log_dev!("[addflat] flat NOVO {flat_id:#x} inserido (size {size}->{}, cap {cap}) tdbOffset={off} ✓", size + 1);
            true
        }
        Err(motivo) => {
            crate::log_dev!("[addflat] {motivo}");
            false
        }
    }
}

/// Comando `getrecords <classe> [limite]` — teste read-only de `get_records_of_class`.
#[cfg(feature = "devtools")]
pub unsafe fn getrecords_cmd(reg: &crate::rtti::Registry, class_name: &str, limit: usize) {
    let Some(t) = singleton() else {
        return crate::log_dev!("[getrecords] singleton indisponível");
    };
    match get_records_of_class(t, reg, class_name, limit) {
        None => {}
        Some(records) => {
            for (i, (id, ptr)) in records.iter().take(10).enumerate() {
                crate::log_dev!("[getrecords]   [{i:02}] id={id:#018x} instance={ptr:p}");
            }
            if records.len() > 10 {
                crate::log_dev!("[getrecords]   ... +{} mais (mostrando só os 10 primeiros)", records.len() - 10);
            }
        }
    }
}

#[cfg(test)]
mod array_flat_tests {
    use super::{dynarray_payload, parse_u64_list, tweak_db_id};
    #[test]
    fn payload_layout_confirmado() {
        // entries@+0 LE, capacity(u32)@+8, size(u32)@+0xC (Containers/DynArray.hpp:564-566)
        let p = dynarray_payload(0x1122334455667788, 3);
        assert_eq!(&p[..8], &0x1122334455667788u64.to_le_bytes());
        assert_eq!(&p[8..12], &3u32.to_le_bytes());
        assert_eq!(&p[12..], &3u32.to_le_bytes());
    }
    #[test]
    fn lista_nomes_e_hex() {
        let v = parse_u64_list("Items.money, 0xDEAD, Items.money,");
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], tweak_db_id("Items.money"));
        assert_eq!(v[1], 0xDEAD);
        assert_eq!(v[2], v[0]);
        assert!(parse_u64_list(" , ").is_empty());
    }
}

// Testes da LÓGICA do clone (inherit_flats_rt) — as peças puras/arriscadas, sem o jogo:
// ordenação por (hash,len), busca binária que ignora o tdbOffset, e o merge-insert in-place.
#[cfg(test)]
mod classe_de_record_tests {
    use super::classe_de_record;

    #[test]
    fn as_tres_formas_dao_o_nome_do_rtti() {
        for nome in ["gamedataAttachmentSlot_Record", "AttachmentSlot", "AttachmentSlot_Record"] {
            assert_eq!(classe_de_record(nome), "gamedataAttachmentSlot_Record");
        }
    }
}

#[cfg(test)]
mod clone_logic_tests {
    use super::{flat_key_present, id_key_lt, merge_insert_from_end};

    // helper: monta uma entry = chave (hash|len<<32) com bits de offset opcionais no topo (40-63).
    fn ent(hash: u32, len: u8, off: u32) -> u64 {
        (hash as u64) | ((len as u64) << 32) | ((off as u64 & 0xFF_FFFF) << 40)
    }

    #[test]
    fn ordena_por_hash_depois_len() {
        assert!(id_key_lt(ent(1, 0, 0), ent(2, 0, 0))); // hash primário
        assert!(id_key_lt(ent(5, 1, 0), ent(5, 2, 0))); // mesmo hash → len secundário
        assert!(!id_key_lt(ent(5, 2, 0), ent(5, 2, 0))); // igual não é <
        // o tdbOffset (bits 40-63) NÃO entra na ordenação:
        assert!(!id_key_lt(ent(5, 2, 0xABCDEF), ent(5, 2, 0)));
        assert!(!id_key_lt(ent(5, 2, 0), ent(5, 2, 0xABCDEF)));
    }

    #[test]
    fn busca_binaria_ignora_offset() {
        let arr = [ent(1, 0, 0), ent(3, 0, 0), ent(5, 0, 0), ent(9, 0, 0)];
        unsafe {
            assert!(flat_key_present(arr.as_ptr(), arr.len(), ent(3, 0, 0)));
            assert!(!flat_key_present(arr.as_ptr(), arr.len(), ent(4, 0, 0)));
            // mesma chave, offset diferente → ACHA (o valor herdado tem offset do source):
            assert!(flat_key_present(arr.as_ptr(), arr.len(), ent(5, 0, 0x777)));
        }
    }

    #[test]
    fn merge_insert_mantem_ordenacao_e_offset() {
        // array velho ordenado com slack; lote novo ordenado sem colidir.
        let mut buf = vec![ent(2, 0, 0), ent(6, 0, 0), 0, 0]; // size=2, cap=4
        let news = [ent(1, 0, 0xAAA), ent(4, 0, 0xBBB)]; // herdam offset do source
        unsafe { merge_insert_from_end(buf.as_mut_ptr(), 2, &news) };
        // resultado: 1,2,4,6 ordenado por chave, com os offsets dos novos preservados.
        let keys: Vec<u32> = buf.iter().map(|e| (*e & 0xFFFF_FFFF) as u32).collect();
        assert_eq!(keys, vec![1, 2, 4, 6]);
        assert_eq!((buf[0] >> 40) & 0xFF_FFFF, 0xAAA); // ent(1) manteve o offset herdado
        assert_eq!((buf[2] >> 40) & 0xFF_FFFF, 0xBBB); // ent(4) idem
    }

    #[test]
    fn merge_insert_no_fim() {
        // novos todos MAIORES que os velhos → vão pro fim, velhos intactos.
        let mut buf = vec![ent(1, 0, 0), ent(2, 0, 0), 0, 0];
        unsafe { merge_insert_from_end(buf.as_mut_ptr(), 2, &[ent(5, 0, 0), ent(8, 0, 0)]) };
        let keys: Vec<u32> = buf.iter().map(|e| (*e & 0xFFFF_FFFF) as u32).collect();
        assert_eq!(keys, vec![1, 2, 5, 8]);
    }

    /// TweakDB falso: trava em +0x20 e índice em `FLATS_OFF` como o DynArray ({entries, cap@+0x08, size@+0x0C}); `cap` entries + o
    /// trailer do alocador.
    fn tweakdb_falso(chaves: &[u64], cap: usize) -> (Vec<u8>, Vec<u64>) {
        use super::FLATS_OFF;
        let mut indice = vec![0u64; cap + 1];
        indice[..chaves.len()].copy_from_slice(chaves);
        indice[cap] = crate::jogo::listas::alocador_de_teste();
        let mut t = vec![0u8; FLATS_OFF + 0x10];
        t[FLATS_OFF..FLATS_OFF + 8].copy_from_slice(&(indice.as_mut_ptr() as u64).to_le_bytes());
        t[FLATS_OFF + 8..FLATS_OFF + 12].copy_from_slice(&(cap as u32).to_le_bytes());
        t[FLATS_OFF + 12..FLATS_OFF + 16].copy_from_slice(&(chaves.len() as u32).to_le_bytes());
        (t, indice)
    }

    /// `(entries, size, cap)` do índice falso e se a trava está livre.
    fn estado(t: &[u8]) -> (*const u64, u32, u32, bool) {
        use super::FLATS_OFF;
        let campo = |o: usize| u32::from_le_bytes(t[FLATS_OFF + o..FLATS_OFF + o + 4].try_into().unwrap());
        let entries = u64::from_le_bytes(t[FLATS_OFF..FLATS_OFF + 8].try_into().unwrap()) as *const u64;
        (entries, campo(0x0C), campo(0x08), t[0x20] == 0)
    }

    fn chaves(entries: *const u64, n: usize) -> Vec<u32> {
        (0..n).map(|i| unsafe { entries.add(i).read() } as u32).collect()
    }

    #[test]
    fn insere_no_slack_sem_trocar_o_buffer() {
        let (mut t, indice) = tweakdb_falso(&[ent(2, 0, 0), ent(6, 0, 0)], 4);
        let visto = (indice.as_ptr() as *mut u64, 2, 4);
        let r = unsafe { super::inserir_no_indice(t.as_mut_ptr(), visto, &[ent(1, 0, 0xAAA), ent(4, 0, 0)]) };
        assert_eq!(r, Ok(None));
        let (entries, size, cap, livre) = estado(&t);
        assert_eq!((entries, size, cap, livre), (indice.as_ptr(), 4, 4, true));
        assert_eq!(chaves(entries, 4), vec![1, 2, 4, 6]);
        assert_eq!((indice[0] >> 40) & 0xFF_FFFF, 0xAAA);
    }

    #[test]
    fn grow_publica_buffer_novo_com_trailer() {
        let (mut t, indice) = tweakdb_falso(&[ent(2, 0, 0), ent(6, 0, 0)], 2);
        let visto = (indice.as_ptr() as *mut u64, 2, 2);
        let r = unsafe { super::inserir_no_indice(t.as_mut_ptr(), visto, &[ent(1, 0, 0), ent(4, 0, 0)]) };
        assert_eq!(r, Ok(Some(2 + 2 + 256)));
        let (entries, size, cap, livre) = estado(&t);
        assert_ne!(entries, indice.as_ptr());
        assert_eq!((size, cap, livre), (4, 260, true));
        assert_eq!(chaves(entries, 4), vec![1, 2, 4, 6]);
        assert_eq!(unsafe { entries.add(260).read() }, crate::jogo::listas::alocador_de_teste());
        assert_eq!(chaves(indice.as_ptr(), 2), vec![2, 6]);
    }

    #[test]
    fn grow_aborta_se_o_indice_mudou_desde_o_filtro() {
        let (mut t, indice) = tweakdb_falso(&[ent(2, 0, 0), ent(6, 0, 0)], 2);
        let visto = (indice.as_ptr() as *mut u64, 1, 2);
        let r = unsafe { super::inserir_no_indice(t.as_mut_ptr(), visto, &[ent(1, 0, 0), ent(4, 0, 0)]) };
        assert!(r.is_err());
        assert_eq!(estado(&t), (indice.as_ptr(), 2, 2, true));
    }

    #[test]
    fn dedup_pool_reusa_por_tipo_e_valor() {
        use super::{flat_pool_lookup, flat_pool_record};
        // valores únicos p/ não colidir com o estado global de outros testes.
        let (vt_a, vt_b) = (0xAAAA_0001u64, 0xBBBB_0001u64); // vtables = tipos diferentes
        let (h1, h2) = (0x1111_0001u64, 0x2222_0001u64); // hashes de valor diferentes
        assert_eq!(flat_pool_lookup(vt_a, h1), None); // vazio no começo
        flat_pool_record(vt_a, h1, 500);
        assert_eq!(flat_pool_lookup(vt_a, h1), Some(500)); // mesmo tipo+valor → reusa
        assert_eq!(flat_pool_lookup(vt_a, h2), None); // mesmo tipo, valor DIFERENTE → miss
        assert_eq!(flat_pool_lookup(vt_b, h1), None); // TIPO diferente, mesmo valor → miss (o tipo separa)
        flat_pool_record(vt_b, h1, 900);
        assert_eq!(flat_pool_lookup(vt_b, h1), Some(900)); // pool por-tipo
    }
}

#[cfg(test)]
mod fila_de_vez_tests {
    use super::{ops_do_arquivo, FilaDeVez};
    use std::sync::Mutex;
    use std::time::Duration;

    /// As senhas saem na ordem da chamada e as threads acordam ao contrário (a última dorme menos): a vez segue a senha, não o escalonador.
    #[test]
    fn a_vez_segue_a_ordem_da_chamada() {
        let fila = FilaDeVez::nova();
        let aplicados = Mutex::new(Vec::new());
        std::thread::scope(|s| {
            for i in 0..6u64 {
                let senha = fila.tirar();
                let aplicados = &aplicados;
                s.spawn(move || {
                    std::thread::sleep(Duration::from_millis(10 * (6 - i)));
                    senha.esperar();
                    aplicados.lock().unwrap().push(i);
                });
            }
        });
        assert_eq!(*aplicados.lock().unwrap(), [0, 1, 2, 3, 4, 5]);
    }

    /// A senha que sai de cena sem rodar (a thread do arquivo que não subiu) não trava a seguinte, antes ou depois da vez dela.
    #[test]
    fn senha_que_nao_roda_passa_a_vez() {
        let fila = FilaDeVez::nova();
        let (a, b, c, d) = (fila.tirar(), fila.tirar(), fila.tirar(), fila.tirar());
        drop(b);
        drop(a);
        c.esperar();
        drop(c);
        d.esperar();
    }

    /// O `.yaml` quebrado e o `.tweak` quebrado dão `Err` com a etapa; o bom vira operação; `$dlc: EP1` só pesa com `is_ep1`.
    #[test]
    fn ops_do_arquivo_acusa_o_que_nao_parseia() {
        let dir = std::env::temp_dir().join(format!("bwms-ops-do-arquivo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let caso = |nome: &str, texto: &str| {
            let p = dir.join(nome);
            std::fs::write(&p, texto).unwrap();
            p.to_string_lossy().into_owned()
        };
        let bom = caso("bom.yaml", "Items.X:\n  quality: Quality.Rare\n");
        assert_eq!(ops_do_arquivo(&bom, || false).unwrap().len(), 1);
        let tab = caso("tab.yaml", "Items.X:\n\tquality: Quality.Rare\n");
        assert!(ops_do_arquivo(&tab, || false).err().unwrap().starts_with("o YAML não parseou: linha 2"));
        let lista = caso("lista.yml", "- a\n- b\n");
        assert!(ops_do_arquivo(&lista, || false).err().unwrap().starts_with("o tweak não virou operação: "));
        let tweak = caso("quebrado.tweak", "isto não é tweak (\n");
        assert!(ops_do_arquivo(&tweak, || false).err().unwrap().starts_with("o .tweak não parseou: "));
        let ep1 = caso("ep1.yaml", "Items.X:\n  $dlc: EP1\n  quality: Quality.Rare\n");
        assert_eq!((ops_do_arquivo(&ep1, || false).unwrap().len(), ops_do_arquivo(&ep1, || true).unwrap().len()), (0, 1));
        assert!(ops_do_arquivo(&dir.join("nao-existe.yaml").to_string_lossy(), || false).err().unwrap().starts_with("não foi possível ler"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
