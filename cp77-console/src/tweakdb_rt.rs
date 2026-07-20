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
//!
//! ESTE arquivo, fase 1 = PROBE observe-only: confirma o singleton in-vivo e revela
//! o offset do records-map (campo de contagem ≈ nº de records). Zero escrita.

use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

// Endereços CONFIRMADOS por disasm (vmaddr, base 0x1_0000_0000):
const ADDR_TWEAKDB_GET: u64 = 0x1_02b7_3c7c; // TweakDB::Get() lazy-init, ptr em x0
const TDB_SINGLETON_GLOBAL: u64 = 0x1_080c_92d0; // global *TweakDB
// Workflow #2 (disasm-provado): registro de record NOVO em runtime.
const ADDR_CREATE_RECORD: u64 = 0x1_026b_8db8; // CreateRecord(TweakDB* x0, u32 typeHash w1, TweakDBID x2) — aloca + insere Handle no +0x58
const ADDR_RECORD_EXISTS: u64 = 0x1_02b7_63fc; // RecordExists(TweakDB* x0, TweakDBID x1) -> bool w0 (probe do +0x58)

// ===== hashes — FONTE ÚNICA no crate `bwms-hashes` (a MESMA impl do tweakdb-tool offline;
// antes era cópia byte-a-byte aqui). `tweak_db_id` = CRC32(nome)|(len<<32); `record_type_key` =
// murmur3(miolo de gamedata(.*)_Record, seed 0x5EEDBA5E). =====
pub use bwms_hashes::{record_type_key, tweak_db_id};

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
    let loaded = crate::gum::is_readable((t as *const u8).add(0x38) as *const c_void, 1)
        && *(t as *const u8).add(0x38) == 1;
    if !loaded {
        return; // tenta no próximo tick
    }
    let id = tweak_db_id(new_name);
    let type_hash = record_type_key(class_name);
    crate::log(&format!(
        "[tdb] create: entrando — singleton={t:p} loaded=1 id={id:#x} typeHash={type_hash:#010x}"
    ));
    let exists: extern "C" fn(*mut c_void, u64) -> u8 =
        std::mem::transmute(crate::rebase(ADDR_RECORD_EXISTS));
    crate::log("[tdb] create: chamando RecordExists(before)...");
    let before = exists(t, id);
    crate::log(&format!("[tdb] create: RecordExists(before)={before}"));
    if before != 0 {
        crate::log(&format!(
            "[tdb] create: '{new_name}' (id={id:#x}) JÁ existe — abortando (CreateRecord aborta se existe)"
        ));
        return;
    }
    let create: extern "C" fn(*mut c_void, u32, u64) =
        std::mem::transmute(crate::rebase(ADDR_CREATE_RECORD));
    crate::log("[tdb] create: chamando CreateRecord...");
    create(t, type_hash, id);
    crate::log("[tdb] create: CreateRecord retornou; validando...");
    let after = exists(t, id);
    crate::log(&format!(
        "[tdb] CreateRecord('{class_name}' typeHash={type_hash:#010x}, '{new_name}' id={id:#x}) -> RecordExists antes={before} depois={after} {}",
        if after != 0 { "✓✓ RECORD REGISTRADO EM RUNTIME (jogo resolve)" } else { "✗ falhou" }
    ));
}

/// Roda o registro UMA vez quando `~/.bwms-tdbcreate` existe (dev). Cria
/// Items.BwmsCloneTest como gamedataWeaponItem_Record (type_key 0x7fdef930, confirmado
/// válido). Chamado do cp77_tick em gameplay.
pub unsafe fn create_once_if_marked() {
    if CREATED.load(Ordering::Relaxed) {
        return;
    }
    let marked = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-tdbcreate").exists())
        .unwrap_or(false);
    if !marked {
        return;
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
        std::thread::sleep(std::time::Duration::from_secs(3));
        unsafe { create_record_rt("gamedataWeaponItem_Record", "Items.BwmsCloneTest") };
    });
}

/// nº de records/flats do tweakdb.bin vanilla (de `tweakdb-tool info`) — usado p/
/// flagar qual campo do singleton é a CONTAGEM do records-map / flats.
const N_RECORDS: u32 = 176_198;
const N_FLATS: u32 = 2_880_555;

static DUMPED: AtomicBool = AtomicBool::new(false);

#[inline]
unsafe fn rd_u64(p: *const u8) -> u64 {
    (p as *const u64).read_unaligned()
}

/// Ponteiro do singleton TweakDB vivo. Lê o global (populado no load); se null,
/// chama TweakDB::Get() (lazy-init). None se ainda não inicializado/ilegível.
pub unsafe fn singleton() -> Option<*mut u8> {
    // VALIDA que a instância está CARREGADA: o global às vezes aponta pra uma TweakDB
    // staging/vazia (flats size 0, flatDataBuffer null) — usar essa faz o getflat achar nada.
    // Critério de "carregada": flatDataBuffer@+0x148 != 0. Se o global for vazio, cai no Get().
    let loaded = |s: *mut u8| -> bool {
        !s.is_null()
            && crate::gum::is_readable(s as *const c_void, 0x168)
            && rd_u64(s.add(0x148)) != 0
    };
    let gp = crate::rebase(TDB_SINGLETON_GLOBAL) as *const u8;
    if crate::gum::is_readable(gp as *const c_void, 8) {
        let s = rd_u64(gp) as *mut u8;
        if loaded(s) {
            return Some(s);
        }
    }
    // global vazio/staging → Get() (lazy-init, devolve a TweakDB REAL carregada). Só chama se o
    // endereço de CÓDIGO for legível — fora do processo do jogo (ex. `cargo test`) o rebase()
    // aponta pra lixo, e chamar um ponteiro de função não-validado é SIGSEGV garantido (achado
    // 2026-07-13 ao adicionar os testes null-safe da API de TweakDB — nenhum teste anterior
    // exercitava este caminho, então o gap nunca tinha sido pego).
    let get_addr = crate::rebase(ADDR_TWEAKDB_GET);
    if !crate::gum::is_readable(get_addr as *const c_void, 8) {
        return None;
    }
    let get: extern "C" fn() -> *mut c_void = std::mem::transmute(get_addr);
    let s = get() as *mut u8;
    if loaded(s) {
        Some(s)
    } else {
        // último recurso: o global mesmo que não passe no loaded() (melhor que nada p/ create)
        if crate::gum::is_readable(gp as *const c_void, 8) {
            let g = rd_u64(gp) as *mut u8;
            if !g.is_null() && crate::gum::is_readable(g as *const c_void, 0x168) {
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
        if !crate::gum::is_readable(p as *const c_void, 8) {
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
        if crate::gum::is_readable(pp as *const c_void, 8) {
            let p1 = rd_u64(pp) as *const u8;
            // só segue ponteiros plausíveis (heap/imagem), não inteiros pequenos
            if p1 as usize > 0x10000 && crate::gum::is_readable(p1 as *const c_void, 0x140) {
                scan_region(p1, 0x140, &format!("s+{soff:#05x}->P1"));
                // nível 2: cada ponteiro DENTRO do pointee
                let mut j = 0usize;
                while j < 0x140 {
                    let p2 = (p1.add(j) as *const u64).read_unaligned() as *const u8;
                    if p2 as usize > 0x10000 && crate::gum::is_readable(p2 as *const c_void, 0x140) {
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
// {entries, size@+0x48}, flatDataBuffer@0x148. GetFlatValue: binary-search em flats por
// (id & 0xFF_FFFF_FFFF) [40 bits baixos = hash+len], entry.tdbOffset (bytes 5-7 BIG-ENDIAN) →
// FlatValue* = flatDataBuffer + tdbOffset. O valor (data) mora em FlatValue + offsetof(data):
// ~0x08 p/ escalar (Float/Int/Bool), 0x10 p/ 16 bytes (Quaternion/Color).
const FLATS_OFF: usize = 0x40;
const FLAT_DATA_BUFFER_OFF: usize = 0x148; // uintptr_t (RED4ext TweakDB.hpp, ASSERT_OFFSET)
const FLAT_DATA_CAP_OFF: usize = 0x150; //   u32 flatDataBufferCapacity
const FLAT_DATA_END_OFF: usize = 0x158; //   uintptr_t flatDataBufferEnd

/// VIA CANÔNICA de lookup de flat: acha a ENTRY (TweakDBID no SortedUniqueArray) por NOME —
/// binary-search por hash(u32 primário) + length(u8 secundário) (TweakDBID::operator<). Devolve
/// `*mut u64` (a entry, p/ ler OU reescrever o tdbOffset). getflat/setflat/repoint usam ESTA (Q5).
pub unsafe fn find_flat_entry(t: *const u8, name: &str) -> Option<*mut u64> {
    let id = tweak_db_id(name);
    let q_hash = (id & 0xFFFF_FFFF) as u32;
    let q_len = ((id >> 32) & 0xFF) as u8;
    let entries = rd_u64(t.add(FLATS_OFF)) as *mut u64;
    let size = (t.add(FLATS_OFF + 8) as *const u32).read_unaligned();
    if entries.is_null() || size == 0 || size > 10_000_000 {
        return None;
    }
    if !crate::gum::is_readable(entries as *const c_void, (size as usize).min(64) * 8) {
        return None;
    }
    let (mut lo, mut hi) = (0u32, size);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let ep = entries.add(mid as usize);
        if !crate::gum::is_readable(ep as *const c_void, 8) {
            return None;
        }
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

/// tdbOffset de uma entry (bytes 5,6,7 em BIG-ENDIAN).
#[inline]
fn entry_tdb_offset(e: u64) -> u32 {
    ((((e >> 40) & 0xFF) as u32) << 16) | ((((e >> 48) & 0xFF) as u32) << 8) | (((e >> 56) & 0xFF) as u32)
}

/// FlatValue* de um flat por NOME (via `find_flat_entry` + flatDataBuffer + tdbOffset). None se
/// não achar. READ-ONLY multi-thread-safe (ver nota do RED4ext).
pub unsafe fn get_flat_value(t: *const u8, name: &str) -> Option<*mut u8> {
    let ep = find_flat_entry(t, name)?;
    let off = entry_tdb_offset(ep.read_unaligned());
    let fdb = rd_u64(t.add(FLAT_DATA_BUFFER_OFF)) as *mut u8;
    if fdb.is_null() {
        return None;
    }
    let fv = fdb.add(off as usize);
    if crate::gum::is_readable(fv as *const c_void, 0x18) { Some(fv) } else { None }
}

/// Repoint: aponta o flat de `name` pro FlatValue em flatDataBuffer+`new_off` (reescreve os bytes
/// 5,6,7 BE da entry, preservando hash+len). O jogo passa a ler o valor novo. É o que permite
/// SetFlat NÃO-escalar (array/string), onde o valor não cabe in-place e precisa de um FlatValue novo.
pub unsafe fn set_flat_offset(t: *const u8, name: &str, new_off: u32) -> bool {
    let ep = match find_flat_entry(t, name) {
        Some(e) => e,
        None => return false,
    };
    let keep = ep.read_unaligned() & 0x0000_00FF_FFFF_FFFF; // bits 0..39 = hash(0..3)+len(4)
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
    let off = create_flat_value(t, donor, data, align)?;
    if set_flat_offset(t as *const u8, field, off as u32) {
        Some(off)
    } else {
        None
    }
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
    let dv = get_flat_value(t as *const u8, donor)?;
    // probe de sanidade do donor (defesa 2): o payload em +0x8 parece um DynArray?
    let d_entries = rd_u64(dv.add(0x8));
    let d_cap = (dv.add(0x10) as *const u32).read_unaligned();
    let d_size = (dv.add(0x14) as *const u32).read_unaligned();
    if d_size > d_cap || d_cap > 1_000_000 {
        crate::log(&format!("[flat] mkarr: donor '{donor}' não parece array (cap={d_cap} size={d_size})"));
        return None;
    }
    if d_size > 0 && !crate::gum::is_readable(d_entries as *const c_void, 8) {
        crate::log(&format!("[flat] mkarr: entries do donor ilegível ({d_entries:#x})"));
        return None;
    }
    // vft de allocator do donor (defesa 1): cap==0 → está nos bits de entries; cap>0 → trailer.
    let alloc_vft = if d_cap == 0 {
        d_entries
    } else {
        let tr = (d_entries + d_cap as u64 * 8 + 7) & !7;
        if crate::gum::is_readable(tr as *const c_void, 8) { rd_u64(tr as *const u8) } else { 0 }
    };
    // buffer nosso: elems + trailer (leak intencional — flats nunca são destruídos; ver workflow)
    let mut buf = Vec::with_capacity(elems.len() + 1);
    buf.extend_from_slice(elems);
    buf.push(alloc_vft);
    let entries = Box::leak(buf.into_boxed_slice()).as_ptr() as u64;
    let payload = dynarray_payload(entries, elems.len() as u32);
    set_flat_nonscalar(t, field, donor, &payload, 8)
}

/// "Items.A,Items.B,0x1234" → [tweak_db_id, tweak_db_id, 0x1234]. Hex cru passa direto
/// (serve p/ array:CName com hash conhecido); nome vira TweakDBID.
pub fn parse_u64_list(s: &str) -> Vec<u64> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            p.strip_prefix("0x")
                .and_then(|h| u64::from_str_radix(h, 16).ok())
                .unwrap_or_else(|| tweak_db_id(p))
        })
        .collect()
}

/// `mkarr <field> <donor> <a,b,c>` (GATED ~/.bwms-flatwrite): SetFlat de array — cria um flat
/// array:TweakDBID novo com os elementos dados e aponta `field` pra ele. Ex. (dar um attack a
/// mais pra uma arma clonada):
///   mkarr Items.X.attacks Items.Preset_Yasha_Default.attacks Attacks.A,Attacks.B
pub unsafe fn mkarr_cmd(field: &str, donor: &str, list: &str) {
    let on = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-flatwrite").exists())
        .unwrap_or(false);
    if !on {
        crate::log("[flat] mkarr BLOQUEADO: crie ~/.bwms-flatwrite p/ habilitar");
        return;
    }
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log("[flat] singleton indisponível");
            return;
        }
    };
    let elems = parse_u64_list(list);
    if elems.is_empty() {
        crate::log("[flat] mkarr: lista vazia (ex.: Attacks.A,Attacks.B)");
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
            crate::log(&format!(
                "[flat] mkarr '{field}' {} elems -> tdbOffset {off:#x} | FlatValue {before:#x} -> {after:#x} | size lido={size}",
                elems.len()
            ));
        }
        None => crate::log(&format!("[flat] mkarr FALHOU (donor '{donor}' é array do mesmo tipo? buffer cheio?)")),
    }
}

/// Cria um FlatValue NOVO no flatDataBuffer e devolve o tdbOffset (relativo ao buffer). None se
/// não couber / doador inválido. Porte da lógica RED4ext `TweakDB::CreateFlatValue`, MAS reusa a
/// vtable NATIVA (copiada de `donor` = um flat existente do MESMO tipo) em vez de instanciar um
/// FlatValueImpl (cuja vtable seria a do SDK C++ → o jogo saltaria pro nosso código). `data` =
/// bytes do valor (f32/i32/DynArray{ptr,size,cap}/...). Escrita na MESMA thread do jogo (drain do
/// cmd-channel, ponto limpo); mutex00@0x20 NÃO travado (v1 — hardening TODO). `grow` do buffer NÃO
/// implementado (realoc+memcpy com risco de race — ver PORT-MAP-MEMAXO): só o caminho que CABE.
/// DEDUP-POOL (porte do TweakXL `TweakDBBuffer::AllocateValue`, Buffer.cpp:56-82): lista de
/// (vtable do tipo, hash FNV-1a64 do valor, tdbOffset). Um valor idêntico do mesmo tipo é criado UMA
/// vez só; os demais reusam o offset. Evita vazar o flatDataBuffer com duplicatas (o que aceleraria o
/// estouro). Vec + scan linear (const-construível em static; o pool é pequeno = flats novos da sessão).
static FLAT_POOL: std::sync::Mutex<Vec<(u64, u64, i32)>> = std::sync::Mutex::new(Vec::new());

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

pub unsafe fn create_flat_value(t: *mut u8, donor: &str, data: &[u8], align: usize) -> Option<i32> {
    let dv = get_flat_value(t as *const u8, donor)?; // via canônica (a mesma do getflat)
    let vft = rd_u64(dv); // FlatValue+0x00 = vtable nativa do tipo (identifica o TIPO)
    if vft == 0 {
        return None;
    }
    // DEDUP: valor idêntico do mesmo tipo já alocado? reusa o offset (não escreve de novo).
    let data_hash = bwms_hashes::fnv1a64(data);
    if let Some(off) = flat_pool_lookup(vft, data_hash) {
        crate::log(&format!(
            "[flat] dedup-pool HIT: vtable={vft:#x} hash={data_hash:#x} -> offset REUSADO {off} (não alocou de novo)"
        ));
        return Some(off);
    }
    let a = align.max(8) as u64;
    let au = |v: u64| (v + a - 1) & !(a - 1);
    let flat_size = au(8 + data.len() as u64); // 8 = slot da vtable
    let fdb = rd_u64(t.add(FLAT_DATA_BUFFER_OFF));
    let cap = (t.add(FLAT_DATA_CAP_OFF) as *const u32).read_unaligned() as u64;
    let end = rd_u64(t.add(FLAT_DATA_END_OFF));
    if fdb == 0 || end < fdb {
        return None;
    }
    let pos = au(end);
    if pos + flat_size > fdb + cap {
        // grow do buffer NÃO implementado: move os 26MB + invalida FlatValue* absolutos + os
        // defaultValues (TweakDB-inl.hpp:390) — risco alto, não boot-testável. O dedup acima usa o
        // slack com eficiência; setflat/mkflat já provaram que o buffer vanilla tem folga.
        crate::log("[flat] create: buffer cheio (grow não implementado — ver PORT-MAP-MEMAXO)");
        return None;
    }
    let dst = pos as *mut u8; // flatDataBuffer é heap RW do jogo — escrita direta, sem COW
    (dst as *mut u64).write_unaligned(vft);
    std::ptr::copy_nonoverlapping(data.as_ptr(), dst.add(8), data.len());
    (t.add(FLAT_DATA_END_OFF) as *mut u64).write_unaligned(pos + flat_size); // avança flatDataBufferEnd
    let off = (pos - fdb) as i32; // tdbOffset (relativo ao buffer)
    flat_pool_record(vft, data_hash, off);
    Some(off)
}

/// `getflat <nome>` (READ-ONLY): acha o FlatValue e dumpa o cabeçalho (vtable + dados em
/// +0x08/+0x10) pra confirmar o lookup e ver onde o valor mora.
pub unsafe fn probe_flat(name: &str) {
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log("[flat] singleton indisponível");
            return;
        }
    };
    // diagnóstico: confirma que o singleton resolvido está CARREGADO (flats_size ~3.3M, fdb != 0)
    let flats_sz = (t.add(0x48) as *const u32).read_unaligned();
    let fdb = rd_u64(t.add(0x148));
    crate::log(&format!("[flat] singleton={t:p} flats_size={flats_sz} fdb={fdb:#x}"));
    match get_flat_value(t, name) {
        None => crate::log(&format!("[flat] '{name}' NÃO achado (id={:#x})", tweak_db_id(name))),
        Some(fv) => {
            let vt = rd_u64(fv);
            let d08 = (fv.add(0x08) as *const u32).read_unaligned();
            let d10 = (fv.add(0x10) as *const u32).read_unaligned();
            crate::log(&format!(
                "[flat] '{name}' @ {fv:p} | vtable={vt:#x} | +0x08: u32 {d08:#x}/f32 {} | +0x10: u32 {d10:#x}/f32 {}",
                f32::from_bits(d08), f32::from_bits(d10)
            ));
        }
    }
}

/// `getarr <nome>` (READ-ONLY) — **`cet-tweakdb-read-records` (2026-07-13):** lê um flat ARRAY
/// (ex.: `attacks`/`statModifiers` de uma arma) de um record VIVO e dumpa os elementos (assume
/// `array:TweakDBID`/`array:CName`, elementos de 8B — mesmo layout já usado pelo lado de
/// ESCRITA em `set_flat_array_u64`/`mkarr`, aqui só lendo). Payload DynArray @FlatValue+0x8:
/// entries@+0x8, cap(u32)@+0x10, size(u32)@+0x14 (mesmo layout documentado ali).
pub unsafe fn probe_array_flat(name: &str) {
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log("[flat] singleton indisponível");
            return;
        }
    };
    let fv = match get_flat_value(t, name) {
        None => {
            crate::log(&format!("[flat] getarr '{name}' NÃO achado (id={:#x})", tweak_db_id(name)));
            return;
        }
        Some(fv) => fv,
    };
    let entries = rd_u64(fv.add(0x8));
    let cap = (fv.add(0x10) as *const u32).read_unaligned();
    let size = (fv.add(0x14) as *const u32).read_unaligned();
    if size > cap || cap > 1_000_000 {
        crate::log(&format!(
            "[flat] getarr '{name}': não parece array (cap={cap} size={size}) — é escalar? use getflat"
        ));
        return;
    }
    if size > 0 && !crate::gum::is_readable(entries as *const c_void, size as usize * 8) {
        crate::log(&format!("[flat] getarr '{name}': entries ilegível ({entries:#x})"));
        return;
    }
    let mut out = format!("[flat] getarr '{name}' @ {fv:p}: cap={cap} size={size}\n");
    for i in 0..size as usize {
        let v = rd_u64((entries as *const u8).add(i * 8));
        out.push_str(&format!("  [{i}] = {v:#018x}\n"));
    }
    crate::log(&out);
}

/// Núcleo UNGATED de get/set de flat escalar (offset +0x08, o caso comum — a maioria dos stats
/// numéricos de item/record). Usado pelo comando dev `getflat`/`setflat` (que tem seu próprio
/// gate/log) E pela API pública (`tweakdb-mod-api`, BwmsApi) — mods de verdade que chamam a API
/// já querem o efeito na hora, sem depender de um marcador de dev. ⚠️ `api_set_flat_scalar`
/// afeta TODOS os records que compartilham o mesmo FlatValue (mesma ressalva do `setflat`).
pub unsafe fn api_get_flat_scalar(name: &str) -> Option<u32> {
    let t = singleton()?;
    let fv = get_flat_value(t, name)?;
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
    for ((fv, name, orig), (_, new_val)) in resolved.iter().zip(ops.iter()) {
        (fv.add(0x08) as *mut u32).write_unaligned(*new_val);
        originals.push((name.clone(), *orig));
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

/// `batchset <f1>=<v1> <f2>=<v2> ...` (GATED ~/.bwms-flatwrite, mesmo gate de setflat/mkflat):
/// prova `tweakxl-batch-commit` — aplica vários sets de uma vez, tudo-ou-nada. Cada valor usa a
/// convenção do `setflat` (0xHEX u32 / decimal / float). Loga o resultado; se ALGUM campo não
/// existir, confirma que NENHUM foi escrito (nem os válidos antes dele na lista).
pub unsafe fn batchset_cmd(pairs: &[String]) {
    let on = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-flatwrite").exists())
        .unwrap_or(false);
    if !on {
        crate::log("[batchset] BLOQUEADO: crie ~/.bwms-flatwrite p/ habilitar escrita");
        return;
    }
    let mut ops: Vec<(String, u32)> = Vec::new();
    for p in pairs {
        let Some((name, valstr)) = p.split_once('=') else {
            crate::log(&format!("[batchset] par mal-formado (esperado campo=valor): '{p}'"));
            return;
        };
        let Some(val) = parse_scalar_val(valstr) else {
            crate::log(&format!("[batchset] valor inválido em '{p}' (use 0xHEX / decimal / float)"));
            return;
        };
        ops.push((name.to_string(), val));
    }
    let ops_ref: Vec<(&str, u32)> = ops.iter().map(|(n, v)| (n.as_str(), *v)).collect();
    match batch_set_flat_scalar(&ops_ref) {
        Ok(originals) => {
            crate::log(&format!(
                "[batchset] TUDO escrito ({} campos) — originais salvos: {:?} — >>> BATCH COMMIT OK <<<",
                originals.len(),
                originals
            ));
        }
        Err(msg) => {
            crate::log(&format!("[batchset] ABORTADO: {msg} — >>> ATOMICIDADE OK (nada parcial) <<<"));
        }
    }
}

pub unsafe fn api_set_flat_scalar(name: &str, val: u32) -> bool {
    let t = match singleton() {
        Some(s) => s,
        None => return false,
    };
    match get_flat_value(t, name) {
        None => false,
        Some(fv) => {
            (fv.add(0x08) as *mut u32).write_unaligned(val);
            true
        }
    }
}

/// `setflat <nome> <hexval> [off]` (GATED ~/.bwms-flatwrite): sobrescreve 4 bytes do valor.
/// off = offset do dado no FlatValue (default 0x08 = escalar). ⚠️ afeta records que
/// COMPARTILHAM o valor. Loga before/after.
pub unsafe fn write_flat(name: &str, val: u32, data_off: usize) {
    let on = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-flatwrite").exists())
        .unwrap_or(false);
    if !on {
        crate::log("[flat] setflat BLOQUEADO: crie ~/.bwms-flatwrite p/ habilitar escrita");
        return;
    }
    let t = match singleton() {
        Some(s) => s,
        None => return,
    };
    match get_flat_value(t, name) {
        None => crate::log(&format!("[flat] setflat '{name}': não achado")),
        Some(fv) => {
            let p = fv.add(data_off);
            let before = (p as *const u32).read_unaligned();
            (p as *mut u32).write_unaligned(val);
            let after = (p as *const u32).read_unaligned();
            crate::log(&format!(
                "[flat] setflat '{name}' +{data_off:#x}: {before:#x} -> {after:#x} (pediu {val:#x})"
            ));
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

/// `mkflat <field> <donor> <hexbytes>` (GATED ~/.bwms-flatwrite): SetFlat NÃO-escalar — cria um
/// FlatValue novo com os `hexbytes` crus (a vtable nativa vem do `donor`, um flat do MESMO tipo) e
/// aponta `field` pra ele. Prova end-to-end de create_flat_value + set_flat_offset: o ponteiro do
/// FlatValue MUDA (repoint) e o valor lido passa a ser o novo. Ex. (round-trip escalar f32=1.0):
///   mkflat Items.GrenadeIncendiarySticky.deepWaterDepth Items.GrenadeIncendiarySticky.deepWaterDepth 0000803f
pub unsafe fn mkflat_cmd(field: &str, donor: &str, hex: &str) {
    let on = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-flatwrite").exists())
        .unwrap_or(false);
    if !on {
        crate::log("[flat] mkflat BLOQUEADO: crie ~/.bwms-flatwrite p/ habilitar");
        return;
    }
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log("[flat] singleton indisponível");
            return;
        }
    };
    let data = match hex_bytes(hex) {
        Some(d) => d,
        None => {
            crate::log("[flat] hex inválido (ex.: 0000803f = f32 1.0)");
            return;
        }
    };
    let before = get_flat_value(t, field).map(|fv| fv as usize).unwrap_or(0);
    match set_flat_nonscalar(t, field, donor, &data, 8) {
        Some(off) => {
            let after = get_flat_value(t, field).map(|fv| fv as usize).unwrap_or(0);
            crate::log(&format!(
                "[flat] mkflat '{field}' donor='{donor}' {}B -> tdbOffset {off:#x} | FlatValue {before:#x} -> {after:#x}",
                data.len()
            ));
        }
        None => crate::log(&format!("[flat] mkflat FALHOU (donor '{donor}' achável? buffer cheio?)")),
    }
}

// ===== CLONE PROBE (READ-ONLY) — confirma o layout vivo ANTES de qualquer mutação =====
// O clone usável precisa INSERIR no array de flats (SortedUniqueArray@+0x40). Inserir errado
// corrompe o TweakDB inteiro → este probe NÃO muta nada: confirma (1) cap vs size do array
// {entries@+0x40, capacity@+0x48, size@+0x4C} — descobre se há slack p/ inserir sem realloc;
// (2) que dá pra enumerar as props da classe do record (cls+0x28, count@+0x34, name@+0x08,
// parent@+0x10) e reverter CName→nome; (3) que "source.<prop>" existe no flats array (achável
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
        if !crate::gum::is_readable(cls_it as *const c_void, 0x60) {
            break;
        }
        let clsb = cls_it as *const u8;
        for off in [0x48usize, 0x58usize] {
            let fp = rd_u64(clsb.add(off)) as *const u8;
            let n = rd_u32_at(clsb.add(off + 8));
            if fp.is_null() || n >= 20_000 || !crate::gum::is_readable(fp as *const c_void, 8) {
                continue;
            }
            for i in 0..n as usize {
                if !crate::gum::is_readable(fp.add(i * 8) as *const c_void, 8) {
                    break;
                }
                let f = rd_u64(fp.add(i * 8)) as *const u8;
                if f.is_null() || !crate::gum::is_readable(f as *const c_void, 0x20) {
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
pub unsafe fn clone_probe(
    reg: &crate::rtti::Registry,
    class_name: &str,
    source: &str,
    new_name: &str,
) {
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log("[clone] singleton indisponível");
            return;
        }
    };
    // (1) header do SortedUniqueArray de flats. AMBIGUIDADE de layout: o SDK declara
    // capacity@+0x08, size@+0x0C, MAS o find_flat_entry PROVADO usa +0x08 como bound. Resolvo por
    // INVARIANTE (capacity>=size): size=MENOR campo, cap=MAIOR (flats_header). Reporto os dois campos
    // CRUS p/ o próximo boot revelar a verdade (há slack? qual offset é size?).
    let (_ent, size, cap, _sp) = flats_header(t);
    let raw48 = rd_u32_at(t.add(FLATS_OFF + 0x08));
    let raw4c = rd_u32_at(t.add(FLATS_OFF + 0x0C));
    let fdb = rd_u64(t.add(FLAT_DATA_BUFFER_OFF));
    crate::log(&format!(
        "[clone] flats: +0x48={raw48} +0x4C={raw4c} -> size={size} cap={cap} slack={} fdb={fdb:#x}",
        cap as i64 - size as i64
    ));
    // (2)+(3) enumera props da classe (+ parents) e procura o flat do source.
    let cls = reg.class_by_name(class_name);
    if cls.is_null() {
        crate::log(&format!("[clone] classe '{class_name}' NÃO achada no registry"));
        return;
    }
    let getters = enum_record_getters(cls);
    let total = getters.len();
    let mut found = 0usize;
    let mut shown_ok = 0usize;
    let mut shown_no = 0usize;
    crate::log(&format!("[clone] {total} getters enumerados em '{class_name}' (+parents) via funcs@0x48"));
    for g in &getters {
        match resolve_getter_flat(t, source, g) {
            Some((pname, _)) => {
                found += 1;
                if shown_ok < 8 {
                    shown_ok += 1;
                    let src_id = tweak_db_id(&format!("{source}.{pname}"));
                    let new_id = tweak_db_id(&format!("{new_name}.{pname}"));
                    crate::log(&format!(
                        "[clone]   getter '{g}' -> '{pname}': flat src ACHADO (src_id={src_id:#x} -> new_id={new_id:#x})"
                    ));
                }
            }
            None => {
                if shown_no < 4 {
                    shown_no += 1;
                    crate::log(&format!("[clone]   getter '{g}': sem flat (helper/type getter — pulado)"));
                }
            }
        }
    }
    crate::log(&format!(
        "[clone] PROBE '{source}' ({class_name}): {found}/{total} props com flat achável. Inserir {found} flats faria size {size}->{} (cap={cap}, {}). READ-ONLY: nada mutado.",
        size as usize + found,
        if (size as usize + found) <= cap as usize { "CABE no slack — insert SEM realloc" } else { "precisa CRESCER o array (realloc)" }
    ));
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
#[inline]
fn id_key40(a: u64) -> u64 {
    a & 0x0000_00FF_FFFF_FFFF
}

/// Header do SortedUniqueArray de flats, ROBUSTO à ambiguidade de layout (SDK: capacity@+0x08,
/// size@+0x0C; find_flat_entry provado lê +0x08 como bound). Pela invariante capacity>=size:
/// size = MENOR campo, cap = MAIOR, e `size_ptr` = o campo que guarda o menor (é o que cresce no
/// insert). Sob mutex00 não há mutação concorrente → a invariante vale no instante da leitura.
unsafe fn flats_header(t: *mut u8) -> (*mut u64, usize, usize, *mut u32) {
    let entries = rd_u64(t.add(FLATS_OFF)) as *mut u64;
    let f_a = t.add(FLATS_OFF + 0x08) as *mut u32; // +0x48
    let f_b = t.add(FLATS_OFF + 0x0C) as *mut u32; // +0x4C
    let (a, b) = (f_a.read_unaligned() as usize, f_b.read_unaligned() as usize);
    if a <= b { (entries, a, b, f_a) } else { (entries, b, a, f_b) }
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

/// mutex00 (SharedSpinLock @ t+0x20; RED4ext SharedSpinLock-inl.hpp): int8 0=livre, -1(0xFF)=excl.
/// Lock exclusivo = CAS 0->0xFF com spin LIMITADO (não trava o jogo se o lock nunca liberar).
unsafe fn mutex00_lock(t: *mut u8) -> bool {
    let st = &*(t.add(0x20) as *const std::sync::atomic::AtomicU8);
    for i in 0..4_000_000u32 {
        if st.compare_exchange(0, 0xFF, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return true;
        }
        if i & 511 == 511 {
            std::thread::yield_now();
        }
    }
    false
}
unsafe fn mutex00_unlock(t: *mut u8) {
    (*(t.add(0x20) as *const std::sync::atomic::AtomicU8)).store(0, Ordering::Release);
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

/// HERANÇA DE FLATS (o insert): cada prop-flat do `clone` recebe o MESMO tdbOffset do flat do
/// `source` (valor compartilhado). Enumera as props da classe (cls+0x28 + parents, CName@+0x08),
/// monta as entries do clone, e faz MERGE-INSERT (lote ordenado, do fim pro começo) no
/// SortedUniqueArray flats@+0x40 SÓ dentro do slack (cap-size), sob mutex00 exclusivo. Devolve o nº
/// herdado, ou None (classe/array inválidos, nada a herdar, ou precisa crescer o array — grow NÃO
/// implementado: invalidaria FlatValue*, TweakDB.hpp:28). NÃO cria o record (chame create_record_rt
/// DEPOIS). ⚠️ in-game NÃO provado — rode `cloneprobe` antes p/ confirmar que há slack.
pub unsafe fn inherit_flats_rt(
    reg: &crate::rtti::Registry,
    t: *mut u8,
    class_name: &str,
    source: &str,
    clone: &str,
) -> Option<usize> {
    let cls = reg.class_by_name(class_name);
    if cls.is_null() {
        crate::log(&format!("[clone] classe '{class_name}' não achada no registry"));
        return None;
    }
    let (entries, size0, cap, _sp) = flats_header(t);
    if entries.is_null() || size0 == 0 || size0 > 10_000_000 {
        crate::log("[clone] array de flats inválido");
        return None;
    }
    // 1. enumera os GETTERS da classe (funcs@0x48, não props@0x28 — record não tem CProperty) ->
    //    entries novas do clone: cada prop herda o MESMO tdbOffset do flat do source (pula ausentes).
    let getters = enum_record_getters(cls);
    let mut news: Vec<u64> = Vec::new();
    for g in &getters {
        if let Some((pname, ep)) = resolve_getter_flat(t, source, g) {
            let src_entry = ep.read_unaligned();
            let new_id = tweak_db_id(&format!("{clone}.{pname}"));
            // clone_entry = chave nova (40 bits baixos) | bits de offset do source (40-63).
            news.push(id_key40(new_id) | (src_entry & 0xFFFF_FF00_0000_0000));
        }
    }
    if news.is_empty() {
        crate::log(&format!("[clone] nenhum flat de '{source}' achado — nada a herdar (o source existe?)"));
        return None;
    }
    // ordena por chave + dedup (SortedUniqueArray é ÚNICO) + tira os que JÁ existem no array.
    news.sort_by(|a, b| match (id_key_lt(*a, *b), id_key_lt(*b, *a)) {
        (true, _) => std::cmp::Ordering::Less,
        (_, true) => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    });
    news.dedup_by(|a, b| id_key40(*a) == id_key40(*b));
    news.retain(|&e| !flat_key_present(entries, size0, e));
    let add = news.len();
    if add == 0 {
        crate::log(&format!("[clone] todos os flats de '{clone}' já existem — nada a inserir"));
        return None;
    }
    if size0 + add > cap {
        // GROW do ÍNDICE flats (SortedUniqueArray @+0x40, entries de 8B). NÃO cresce o flatDataBuffer:
        // as heranças COMPARTILHAM o tdbOffset do source (nenhum FlatValue novo) → o aviso de
        // invalidação (TweakDB.hpp:28) é sobre o fdb, não sobre este índice; o engine faz lookup por
        // binary-search (não cacheia ptr aqui). Realoca num buffer NOSSO (maior) com o trailer de
        // allocator copiado do buffer atual (mesmo padrão PROVADO do mkarr/set_flat_array_u64). O
        // buffer antigo é VAZADO, não liberado — a própria estratégia do engine (TweakDB.hpp:30).
        let cur_trailer = ((entries as u64) + cap as u64 * 8 + 7) & !7;
        if !crate::gum::is_readable(cur_trailer as *const c_void, 8) {
            crate::log("[clone] grow abortado: trailer de allocator do buffer atual ilegível");
            return None;
        }
        let alloc_vft = rd_u64(cur_trailer as *const u8);
        let new_cap = size0 + add + 256; // margem p/ próximos clones sem re-grow
        // buffer NOSSO: new_cap entries (8B) + 1 trailer (em new_cap*8, alinhado). leak intencional.
        let mut buf: Vec<u64> = vec![0u64; new_cap + 1];
        std::ptr::copy_nonoverlapping(entries as *const u64, buf.as_mut_ptr(), size0);
        buf[new_cap] = alloc_vft;
        let new_entries = Box::leak(buf.into_boxed_slice()).as_mut_ptr();
        // merge das `add` novas em [0..size0] -> [0..size0+add] ordenado (não toca o trailer).
        merge_insert_from_end(new_entries, size0, &news);
        // publica sob mutex00 exclusivo (o engine toma mutex00 p/ flats — RED4ext TweakDB.hpp:150).
        if !mutex00_lock(t) {
            crate::log("[clone] mutex00 ocupado no grow — abortado (buffer novo vazado, inócuo)");
            return None;
        }
        let (entries2, size2, cap2, _sp2) = flats_header(t); // reconfirma sob o lock
        if entries2 != entries || size2 != size0 || cap2 != cap {
            mutex00_unlock(t);
            crate::log("[clone] flats mudou sob o lock (outro writer?) — abortado SEM publicar");
            return None;
        }
        let fa = t.add(FLATS_OFF + 0x08) as *mut u32; // +0x48
        let fb = t.add(FLATS_OFF + 0x0C) as *mut u32; // +0x4C
        let (size_ptr, cap_ptr) = if (fa.read_unaligned() as usize) <= (fb.read_unaligned() as usize) {
            (fa, fb)
        } else {
            (fb, fa)
        };
        // ordem segura p/ leitor lock-free: entries ptr -> cap -> size (size por último torna as novas visíveis).
        (t.add(FLATS_OFF) as *mut u64).write_unaligned(new_entries as u64);
        cap_ptr.write_unaligned(new_cap as u32);
        size_ptr.write_unaligned((size0 + add) as u32);
        mutex00_unlock(t);
        crate::log(&format!(
            "[clone] GROW: índice flats realocado cap {cap}->{new_cap} + herdou {add} '{source}'->'{clone}' (size {size0}->{}) ✓",
            size0 + add
        ));
        return Some(add);
    }
    // 2. merge-insert do lote (do fim pro começo) sob mutex00 exclusivo.
    if !mutex00_lock(t) {
        crate::log("[clone] mutex00 ocupado — abortado SEM mutar");
        return None;
    }
    let (entries, size, cap, size_ptr) = flats_header(t); // reconfirma sob o lock
    if size + add > cap {
        mutex00_unlock(t);
        crate::log("[clone] slack sumiu sob o lock — abortado");
        return None;
    }
    merge_insert_from_end(entries, size, &news);
    size_ptr.write_unaligned((size + add) as u32);
    mutex00_unlock(t);
    crate::log(&format!(
        "[clone] herdou {add} flats '{source}' -> '{clone}' (size {size}->{}, cap {cap}) ✓",
        size + add
    ));
    Some(add)
}

// ===== `tweakxl-pipeline-runtime` (2026-07-15): detecção de CLASSE de um record por NOME =====
// O formato real do TweakXL (`$base: Items.X`) NUNCA anota a classe do record clonado — só o
// `$type` (create-from-classe) tem isso. O parser OFFLINE (`tweakdb-tool`) resolve isso varrendo
// o `.bin` inteiro carregado (tem um índice id->type_key pronto); em RUNTIME não há esse índice
// vivo (só RTTI por getters). Fix: tabela de classes conhecidas + amostra REAL extraída offline do
// tweakdb.bin do jogo (`tweakdb-tool records --class <X>`, 2026-07-15) + detecção por CONTAGEM de
// getters que resolvem no record-alvo (a classe CERTA resolve MUITOS; uma errada só os poucos
// nomes de prop que colidem por acaso, ex. "price"/"tags").

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

/// Detecta a classe de um record EXISTENTE tentando cada candidato de `KNOWN_RECORD_CLASSES`: pra
/// cada um, enumera os getters da classe e conta quantos resolvem um flat em `record`. Exige
/// mínimo de 3 matches (evita falso-positivo de colisão de nome) e fica com o candidato de MAIOR
/// contagem (a classe certa deve bater MUITO mais que uma errada por coincidência).
pub unsafe fn detect_record_class(reg: &crate::rtti::Registry, t: *mut u8, record: &str) -> Option<&'static str> {
    let mut best: Option<(&'static str, usize)> = None;
    for (class_name, _sample) in KNOWN_RECORD_CLASSES {
        let cls = reg.class_by_name(class_name);
        if cls.is_null() {
            continue;
        }
        let getters = enum_record_getters(cls);
        let hits = getters.iter().filter(|g| resolve_getter_flat(t, record, g).is_some()).count();
        if hits >= 3 && best.is_none_or(|(_, n)| hits > n) {
            best = Some((class_name, hits));
        }
    }
    if let Some((name, hits)) = best {
        crate::log(&format!("[xlauto] classe detectada p/ '{record}': '{name}' ({hits} getters resolvidos)"));
    } else {
        crate::log(&format!("[xlauto] classe NÃO detectada p/ '{record}' (nenhum candidato bateu ≥3 getters — fora da KNOWN_RECORD_CLASSES ou record inexistente)"));
    }
    best.map(|(name, _)| name)
}

/// `xlautoclone <base> <novo>` (GATED ~/.bwms-flatwrite, mesmo gate de clone/mkflat) — a peça que
/// faltava pro `tweakxl-pipeline-runtime`: clona SEM precisar dizer a classe manualmente (a
/// classe é DETECTADA de `base` via `detect_record_class`). Reaproveita 100% o mecanismo de clone
/// já provado (`clone_record_async` = `inherit_flats_rt`+`create_record_rt`) — só adiciona a
/// detecção automática na frente. É o passo que faltava pra rodar um `.yaml` real do TweakXL
/// (que nunca anota a classe do `$base`) sem intervenção manual.
pub unsafe fn xlautoclone_cmd(base: &str, clone: &str) {
    let on = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-flatwrite").exists())
        .unwrap_or(false);
    if !on {
        crate::log("[xlauto] BLOQUEADO: crie ~/.bwms-flatwrite p/ habilitar (muta o TweakDB vivo)");
        return;
    }
    let reg = match crate::rtti::Registry::obtain() {
        Some(r) => r,
        None => return crate::log("[xlauto] Registry indisponível"),
    };
    let t = match singleton() {
        Some(t) => t,
        None => return crate::log("[xlauto] singleton indisponível"),
    };
    match detect_record_class(&reg, t, base) {
        Some(class_name) => clone_record_async(class_name, base, clone),
        None => crate::log(&format!("[xlauto] abortado: classe de '{base}' não detectada")),
    }
}

// ===== `tweakxl-pipeline-runtime` (2026-07-15): PONTE com o parser .yaml REAL do TweakXL =====
// Usa o parser puro do `tweakdb-tool` (reexposto como lib, ver Cargo.toml + tweakdb-tool/src/
// lib.rs) — mesmo `interpret_from` que já produz `Vec<Op>` pro caminho OFFLINE, agora aplicado
// direto no TweakDB VIVO. Escopo desta v1 (documentado, não escondido): `$base`/`$type` completos
// (via `detect_record_class`/`KNOWN_RECORD_CLASSES`); `Op::Edit` só cobre `EditOp::Assign`
// ESCALAR (o caso mais comum de mods reais — `damage: 500` etc.) — arrays/`!append`/`!remove`/
// Assign-de-array ficam fora desta v1 (logados como "não suportado", não crasham).

/// Aplica uma sequência de `Op` (do parser `tweakdb_tool::tweakxl`) no TweakDB VIVO. Roda numa
/// thread PRÓPRIA (mesmo motivo do `clone_record_async`: tomar o lock do TweakDB dentro do hook
/// do executor deadlocka) — SEQUENCIAL: cada Clone/Create precisa terminar (e ficar visível no
/// índice de flats) antes dos Edits que o seguem no `.yaml`, porque referem ao record recém-criado.
/// Por isso roda tudo numa ÚNICA spawn, em ordem, sem re-spawnar por op (diferente de
/// `xlautoclone_cmd`, que é 1 op isolada).
fn apply_ops_runtime(ops: Vec<tweakdb_tool::tweakxl::Op>) {
    use tweakdb_tool::tweakxl::{EditOp, Op};
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500)); // sai do hot-path do hook
        unsafe {
            let reg = match crate::rtti::Registry::obtain() {
                Some(r) => r,
                None => return crate::log("[xlyaml] Registry indisponível"),
            };
            let t = match singleton() {
                Some(t) => t,
                None => return crate::log("[xlyaml] singleton indisponível"),
            };
            let (mut ok, mut fail) = (0usize, 0usize);
            for op in ops {
                match op {
                    Op::Clone { record, base } => match detect_record_class(&reg, t, &base) {
                        Some(class_name) => {
                            match inherit_flats_rt(&reg, t, class_name, &base, &record) {
                                Some(n) => {
                                    create_record_rt(class_name, &record);
                                    crate::log(&format!("[xlyaml] $base {record} <- {base} ({class_name}, {n} flats) OK"));
                                    ok += 1;
                                }
                                None => {
                                    crate::log(&format!("[xlyaml] $base {record} <- {base}: inherit_flats_rt falhou"));
                                    fail += 1;
                                }
                            }
                        }
                        None => {
                            crate::log(&format!("[xlyaml] $base {record} <- {base}: classe não detectada — abortado"));
                            fail += 1;
                        }
                    },
                    Op::Create { record, class } => {
                        match KNOWN_RECORD_CLASSES.iter().find(|(c, _)| *c == class) {
                            Some((class_name, sample)) => {
                                match inherit_flats_rt(&reg, t, class_name, sample, &record) {
                                    Some(n) => {
                                        create_record_rt(class_name, &record);
                                        crate::log(&format!("[xlyaml] $type {record} ({class}) de amostra {sample}, {n} flats OK"));
                                        ok += 1;
                                    }
                                    None => {
                                        crate::log(&format!("[xlyaml] $type {record} ({class}): inherit_flats_rt falhou"));
                                        fail += 1;
                                    }
                                }
                            }
                            None => {
                                crate::log(&format!("[xlyaml] $type {record} ({class}): classe fora da KNOWN_RECORD_CLASSES (v1) — abortado"));
                                fail += 1;
                            }
                        }
                    }
                    Op::Edit { flat, op } => match op {
                        EditOp::Assign(val) => match parse_scalar_val(&val) {
                            Some(bits) => {
                                if api_set_flat_scalar(&flat, bits) {
                                    crate::log(&format!("[xlyaml] {flat} = {val} OK"));
                                    ok += 1;
                                } else {
                                    crate::log(&format!("[xlyaml] {flat} = {val}: flat não achado"));
                                    fail += 1;
                                }
                            }
                            None => {
                                crate::log(&format!("[xlyaml] {flat} = '{val}': valor não-escalar (array/string) — fora do escopo desta v1, não aplicado"));
                                fail += 1;
                            }
                        },
                        _ => {
                            crate::log(&format!("[xlyaml] {flat}: operação de array (!append/!remove/etc.) — fora do escopo desta v1, não aplicada"));
                            fail += 1;
                        }
                    },
                }
            }
            crate::log(&format!("[xlyaml] pipeline concluído: {ok} ok / {fail} não-aplicadas"));
        }
    });
}

/// `applyxlfile <caminho.yaml>` (GATED ~/.bwms-flatwrite): lê+parseia um `.yaml` REAL do
/// TweakXL e aplica no TweakDB vivo (ver `apply_ops_runtime` pro escopo exato desta v1). É o
/// pipeline completo `tweakxl-pipeline-runtime`: parser real (`tweakdb_tool::tweakxl`) + detecção
/// automática de classe (`detect_record_class`) + aplicação sequencial em runtime.
pub unsafe fn applyxlfile_cmd(path: &str) {
    let on = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-flatwrite").exists())
        .unwrap_or(false);
    if !on {
        crate::log("[xlyaml] BLOQUEADO: crie ~/.bwms-flatwrite p/ habilitar (muta o TweakDB vivo)");
        return;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return crate::log(&format!("[xlyaml] lendo '{path}': {e}")),
    };
    let root = match tweakdb_tool::yaml::parse(&text) {
        Ok(r) => r,
        Err(e) => return crate::log(&format!("[xlyaml] parse YAML: {e}")),
    };
    let root = match tweakdb_tool::template::expand(&root) {
        Ok(r) => r,
        Err(e) => return crate::log(&format!("[xlyaml] expand $instances: {e}")),
    };
    let ops = match tweakdb_tool::tweakxl::interpret_from(&root, path) {
        Ok(o) => o,
        Err(e) => return crate::log(&format!("[xlyaml] interpret: {e}")),
    };
    if ops.is_empty() {
        return crate::log(&format!("[xlyaml] '{path}' não produziu nenhuma operação"));
    }
    crate::log(&format!("[xlyaml] '{path}': {} operações extraídas, aplicando...", ops.len()));
    apply_ops_runtime(ops);
}

/// Núcleo compartilhado (assíncrono, sempre em THREAD SEPARADA — o drain do cmd-channel/handler de
/// plugin roda dentro do hook do executor; tomar o lock do TweakDB ali deadlocka) do clone com
/// herança de flats + registro do record novo. Usado por `clone_cmd` (dev, gated) e
/// `clone_record_api` (BwmsApi, ungated — mods de verdade chamando a API já querem o efeito).
unsafe fn clone_record_async(class_name: &str, source: &str, clone: &str) {
    let (c, s, n) = (class_name.to_string(), source.to_string(), clone.to_string());
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        unsafe {
            let reg = match crate::rtti::Registry::obtain() {
                Some(r) => r,
                None => return crate::log("[clone] Registry indisponível"),
            };
            let t = match singleton() {
                Some(t) => t,
                None => return crate::log("[clone] singleton indisponível"),
            };
            match inherit_flats_rt(&reg, t, &c, &s, &n) {
                Some(k) => {
                    crate::log(&format!("[clone] {k} flats herdados; criando record '{n}'..."));
                    create_record_rt(&c, &n);
                }
                None => crate::log("[clone] herança não aplicada — record NÃO criado (ver motivo acima)"),
            }
        }
    });
}

/// `clone <classe> <source> <novo>` (GATED ~/.bwms-flatwrite): CLONE USÁVEL — herda os flats do
/// source (stats reais) e registra o record novo no TweakDB vivo. Ex.:
///   clone gamedataWeaponItem_Record Items.Preset_Lexington_Default Items.BwmsLexClone
pub unsafe fn clone_cmd(class_name: &str, source: &str, clone: &str) {
    let on = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-flatwrite").exists())
        .unwrap_or(false);
    if !on {
        crate::log("[clone] BLOQUEADO: crie ~/.bwms-flatwrite p/ habilitar (muta o TweakDB vivo)");
        return;
    }
    clone_record_async(class_name, source, clone);
}

/// Versão UNGATED de `clone_cmd`, exposta via `BwmsApi::tweakdb_clone_record` — mesma operação
/// (assíncrona, thread separada), sem depender do marcador de dev `~/.bwms-flatwrite`.
pub unsafe fn clone_record_api(class_name: &str, source: &str, clone: &str) {
    clone_record_async(class_name, source, clone);
}

/// Roda o dump UMA vez quando o marcador `~/.bwms-tdbdump` existe (dev). Chamado do
/// cp77_tick (em gameplay, TweakDB já carregada). Idempotente.
pub unsafe fn dump_once_if_marked() {
    if DUMPED.load(Ordering::Relaxed) {
        return;
    }
    let marked = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-tdbdump").exists())
        .unwrap_or(false);
    if !marked {
        return;
    }
    if DUMPED.swap(true, Ordering::Relaxed) {
        return;
    }
    dump_singleton();
}

// `tweakxl-updaterecord` — TENTADO e REVERTIDO (2026-07-13): a ideia (chamar
// `TweakDBInterface.GetFloat(TweakDBID, default) -> Float`, a API OFICIAL de mods redscript,
// como consumidor INDEPENDENTE do nosso próprio `get_flat_value`, pra provar que o record re-lê
// o flat mutado sem reload) CRASHOU (`EXC_BAD_ACCESS`/SIGSEGV, null-deref dentro de
// `rtti::call_func`, chamado a partir desta função — confirmado via crash-report parseado,
// stack: `updaterecord_test` → `call_func` → `exec_replacement`). A classe resolveu (por
// `class_by_name` OU pelo fallback `resolve_class_via_validator_getclass`), e `resolve_in_class`
// achou algo chamado "GetFloat" — mas a chamada em si crashou, indicando que ou (a) o ponteiro
// de classe resolvido não é genuinamente válido pra esta classe `importonly` específica (walk
// de `funcs`/`staticFuncs` sobre memória que não é o CClass real), ou (b) `call_func` com
// `ctx=null` não é a convenção certa pra um método static de uma classe `importonly` (pode
// esperar um contexto diferente do que `callg` usa pra funções verdadeiramente globais). Não
// isolado — precisa de RE dedicada (mesmo nível das tentativas 1-11 da Facade) antes de
// retomar. Revertido por completo; `resolve_class_via_validator_getclass` ficou `pub(crate)`
// (mudança inócua, sem uso agora) caso sirva de novo depois.

// ===== `tweakxl-updaterecord` — UpdateRecord (2026-07-17) =====
// Achado por RE #2: NÃO há um TweakDB::UpdateRecord(id) native único. A invalidação é GLOBAL via
// um version-byte @TweakDB+0x160. Cada campo de record cacheia (na sua FlatConnection embutida) o
// tdbOffset resolvido + um version-byte (byte9). O accessor de campo (@0x100a25f4c) só RE-RESOLVE
// quando field.version != global(+0x160). Logo o "UpdateRecord effect" = bump o +0x160 → força todo
// campo a re-resolver na próxima leitura. Endereços (link base 0x100000000):
const ADDR_GET_RECORD: u64 = 0x1_02b7_45d0; // void GetRecord(TweakDB* x0, TweakDBID x1, Handle<Record>* out x8)
const ADDR_FLAT_ACCESSOR: u64 = 0x1_00a2_5f4c; // re-resolve lazy do campo (x0 = FlatConnection*)
const VERSION_BYTE_OFF: usize = 0x160; // TweakDB+0x160 = global flat-version byte

/// GetRecord via asm (o retorno Handle vai em x8/sret). Devolve o ponteiro da INSTÂNCIA do record
/// (handle+0x00), ou null.
unsafe fn get_record_instance(t: *mut u8, id: u64) -> *mut u8 {
    let addr = crate::rebase(ADDR_GET_RECORD);
    if !crate::gum::is_readable(addr as *const c_void, 4) {
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
    handle[0] as *mut u8
}

/// Acha a FlatConnection (10 bytes {hash[0..3],len[4],offsetBE[5..7],lock[8],version[9]}) de um flat
/// DENTRO da instância do record, casando os 5 bytes baixos (hash+len) do TweakDBID do flat E
/// validando que o offset cacheado (bytes 5-7 BE) == `expected_off` (o offset do flat índice ANTES
/// do repoint — a FlatConnection REAL cacheia o MESMO offset que o índice). O prefixo-só dava FALSO
/// positivo (qualquer u64 com os 5 bytes baixos batendo); a validação do offset desambigua. Varre
/// uma janela limitada (byte-granular). Devolve o ptr da FlatConnection, ou null.
unsafe fn find_flat_connection(record: *mut u8, flat_id: u64, expected_off: u32) -> *mut u8 {
    let prefix = flat_id & 0x0000_00FF_FFFF_FFFF; // hash(0..3)+len(4)
    const WINDOW: usize = 0x1000;
    if !crate::gum::is_readable(record as *const c_void, WINDOW) {
        return std::ptr::null_mut();
    }
    let mut off = 0usize;
    while off + 10 <= WINDOW {
        let v = (record.add(off) as *const u64).read_unaligned();
        if v & 0x0000_00FF_FFFF_FFFF == prefix && fc_offset(record.add(off)) == expected_off {
            return record.add(off);
        }
        off += 1;
    }
    std::ptr::null_mut()
}

/// Como `find_flat_connection`, mas SEM exigir offset — coleta TODOS os candidatos (só prefixo
/// hash+len batendo) numa janela do record. Diagnóstico (2026-07-17): usado quando a validação por
/// offset falha, pra ver o que REALMENTE está cacheado sem assumir uma hipótese específica.
unsafe fn find_flat_connections_by_prefix(record: *mut u8, flat_id: u64) -> Vec<(usize, u32)> {
    let prefix = flat_id & 0x0000_00FF_FFFF_FFFF;
    const WINDOW: usize = 0x1000;
    let mut out = Vec::new();
    if !crate::gum::is_readable(record as *const c_void, WINDOW) {
        return out;
    }
    let mut off = 0usize;
    while off + 10 <= WINDOW {
        let v = (record.add(off) as *const u64).read_unaligned();
        if v & 0x0000_00FF_FFFF_FFFF == prefix {
            out.push((off, fc_offset(record.add(off))));
        }
        off += 1;
    }
    out
}

#[inline]
unsafe fn fc_offset(fc: *const u8) -> u32 {
    // bytes 5,6,7 (BE) da FlatConnection = tdbOffset cacheado.
    let b5 = *fc.add(5) as u32;
    let b6 = *fc.add(6) as u32;
    let b7 = *fc.add(7) as u32;
    (b5 << 16) | (b6 << 8) | b7
}

/// `tweakxl-updaterecord` (menu, log-only): prova que um setflat REPOINT num record existente só
/// aparece no CAMPO DO RECORD após o "UpdateRecord" (bump do version-byte + re-resolve). Escalar
/// in-place já é auto-visível (mesmo offset), então usa REPOINT (novo FlatValue, offset diferente)
/// pra testar de verdade a re-resolução.
///
/// # Safety
/// GetRecord/accessor são fns nativas do TweakDB vivo; a FlatConnection é achada dentro da própria
/// instância do record (ponteiro válido do jogo); o repoint reusa set_flat_offset já provado.
pub(crate) unsafe fn prove_updaterecord() {
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log("[updaterec] TweakDB singleton indisponível (cedo demais?) — abortado");
            return;
        }
    };
    let record_name = "Items.GrenadeIncendiarySticky";
    let flat_short = "deepWaterDepth";
    let full_flat = format!("{record_name}.{flat_short}");
    let rec_id = tweak_db_id(record_name);
    let flat_id = tweak_db_id(&full_flat);

    // DIAG: RecordExists no menu? (records instanciam em recordsByID só no world-load? — se false
    // aqui mas o record existe no .bin, precisa de gameplay).
    let exists: extern "C" fn(*mut c_void, u64) -> u8 =
        core::mem::transmute(crate::rebase(ADDR_RECORD_EXISTS));
    let rec_exists = exists(t as *mut c_void, rec_id);
    crate::log(&format!("[updaterec] RecordExists('{record_name}' id={rec_id:#x})={rec_exists}"));

    // 1) instância do record.
    let record = get_record_instance(t, rec_id);
    if record.is_null() || !crate::gum::is_readable(record as *const c_void, 0x40) {
        crate::log(&format!(
            "[updaterec] GetRecord('{record_name}') null/ilegível (RecordExists={rec_exists}) — se RecordExists=0, recordsByID não populado no menu (precisa gameplay); abortado"
        ));
        return;
    }
    crate::log(&format!("[updaterec] record '{record_name}' @{record:p}"));

    // Accessor: acc(FlatConnection*) -> value_addr (re-resolve lazy se version(byte9)!=global). É a
    // via que o jogo usa pra ler o valor — evita computar fdb/offset na mão (RE 2026-07-17: fdb é
    // *(u64*)(*(0x1080c92d0)) NÃO t+0x148, e o FC cacheia idx_off+8, não idx_off).
    let acc: unsafe extern "C" fn(*mut u8) -> *mut u8 = core::mem::transmute(crate::rebase(ADDR_FLAT_ACCESSOR));
    // O caller real do accessor faz `bl accessor; ldr x0,[x0]` (um deref a MAIS) — então o valor
    // pode ser *(acc(fc)) e não acc(fc) direto. read_field faz o deref extra; a 1ª leitura loga
    // vários candidatos pra cravar (esperado o inicial = -0.5 do deepWaterDepth).
    let read_val_at = |va: *const u8| -> f32 {
        if va.is_null() || !crate::gum::is_readable(va as *const c_void, 4) {
            f32::NAN
        } else {
            (va as *const f32).read_unaligned()
        }
    };
    let read_field = |fc: *mut u8, diag: bool| -> f32 {
        let p = acc(fc); // ptr devolvido pelo accessor
        if p.is_null() || !crate::gum::is_readable(p as *const c_void, 8) {
            return f32::NAN;
        }
        let deref1 = *(p as *const *const u8); // *(acc(fc)) — o `ldr x0,[x0]` do caller
        if diag {
            crate::log(&format!(
                "[updaterec-diag] acc(fc)={p:p} | direto f32@p={} | *(acc)={deref1:p} f32@*(acc)={} | f32@(*(acc)+8)={}",
                read_val_at(p),
                read_val_at(deref1),
                read_val_at(unsafe { deref1.add(8) })
            ));
        }
        // hipótese principal: valor = *(f32*)(*(acc(fc)))
        read_val_at(deref1)
    };

    // 2) FlatConnection do flat no record — validada pelo offset cacheado == (offset do índice + 8)
    // (o FC aponta pro VALOR, pulando o vtable de 8B do FlatValue; RE 2026-07-17). Case os 5 bytes
    // inteiros (hash u32 + len u8) — o scan só-prefixo dava falso positivo.
    let idx_ep = match find_flat_entry(t as *const u8, &full_flat) {
        Some(e) => e,
        None => {
            crate::log(&format!("[updaterec] flat '{full_flat}' não achado no índice — abortado"));
            return;
        }
    };
    let idx_off = entry_tdb_offset(idx_ep.read_unaligned());
    let fc = find_flat_connection(record, flat_id, idx_off + 8);
    if fc.is_null() {
        crate::log(&format!(
            "[updaterec] FlatConnection de '{full_flat}' (5-byte id {:#014x}, offset esperado idx+8={:#x}) não achada — abortado",
            flat_id & 0xFF_FFFF_FFFF,
            idx_off + 8
        ));
        return;
    }
    let v_inicial = read_field(fc, true); // trigger resolve + cacheia (version=global). Valor OLD (-0.5).
    let off_ini = fc_offset(fc); // OBSERVÁVEL ROBUSTO: offset cacheado (bytes 5-7 BE) — leitura de memória.
    crate::log(&format!(
        "[updaterec] FlatConnection @{fc:p} offset_cacheado_inicial={off_ini:#x} (esperado idx+8={:#x}) valor_inicial={v_inicial}",
        idx_off + 8
    ));

    // 3) cria um FlatValue NOVO (valor distinto) e REPOINTA o índice do flat (não o record).
    let new_val: f32 = 424242.0;
    let new_off = match create_flat_value(t, &full_flat, &new_val.to_le_bytes(), 8) {
        Some(o) => o as u32,
        None => {
            crate::log("[updaterec] create_flat_value falhou (buffer cheio / donor não Float?) — abortado");
            return;
        }
    };
    if !set_flat_offset(t as *const u8, &full_flat, new_off) {
        crate::log("[updaterec] set_flat_offset (repoint) falhou — abortado");
        return;
    }
    crate::log(&format!("[updaterec] repoint: índice do flat agora -> offset {new_off:#x} (FlatValue novo)"));

    // 4) SEM UpdateRecord: o accessor não re-resolve (version bate) → offset cacheado da FC INALTERADO.
    let v_sem = read_field(fc, false); // trigger (não re-resolve pq version==global)
    let off_sem = fc_offset(fc); // deve continuar == off_ini (idx antigo+8)

    // 5) UpdateRecord = bump do version-byte global. Próximo acesso re-resolve (version != global).
    let ver_antes = *t.add(VERSION_BYTE_OFF);
    *t.add(VERSION_BYTE_OFF) = ver_antes.wrapping_add(1);
    let ver_depois = *t.add(VERSION_BYTE_OFF);
    let v_com = read_field(fc, false); // trigger re-resolve → FC re-lê o índice → offset cacheado vira new+8.
    let off_com = fc_offset(fc); // deve == new_off+8

    // PROVA robusta pelo OFFSET CACHEADO da FC (o valor via double-deref é frágil e fica só como diag):
    // (a) o repoint NÃO muda o offset cacheado ANTES do UpdateRecord; (b) DEPOIS do bump+re-resolve,
    // o offset cacheado vira o do FlatValue NOVO (new_off+8). Isso É o efeito do UpdateRecord.
    let esperado_new = new_off + 8;
    let sem_inalterado = off_sem == off_ini && off_ini == idx_off + 8;
    let com_reresolveu = off_com == esperado_new;
    let ok = sem_inalterado && com_reresolveu;
    // check secundário (diag): o valor lido também deve virar o novo (se o accessor value-read funcionar).
    let val_virou = (v_com - new_val).abs() < 0.1;
    let verdict = if ok {
        ">>> UPDATERECORD OK: a FlatConnection do RECORD só re-resolveu pro FlatValue novo APÓS o UpdateRecord (bump +0x160): offset cacheado inalterado antes, = new_off+8 depois <<<"
    } else {
        "verificar: off_sem deve = idx+8 (inalterado) e off_com deve = new_off+8 (re-resolvido)"
    };
    crate::log(&format!(
        "[updaterec] version {ver_antes}->{ver_depois} | OFFSET cacheado: ini={off_ini:#x} sem_update={off_sem:#x} com_update={off_com:#x} (esperado new+8={esperado_new:#x}) | sem_inalterado={sem_inalterado} com_reresolveu={com_reresolveu} | (diag valor: ini={v_inicial} sem={v_sem} com={v_com} virou={val_virou}) | {verdict}"
    ));
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
// no literal pool de `0x102b74408`) — não passa pelo nosso IAllocator fake, então só precisamos
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
                fake_alloc as usize as u64,
                fake_alloc_aligned as usize as u64,
                fake_realloc as usize as u64,
                fake_realloc_aligned as usize as u64,
                fake_free as usize as u64,
                fake_sub28 as usize as u64,
                fake_get_handle as usize as u64,
            ]
        })
        .as_ptr() as u64
}

const ADDR_ASSIGN_TYPE_VTBL_SLOT_OFF: usize = 0x58; // Windows SDK documenta +0x50; shift Itanium 2-dtor já provado no projeto.

/// `UpdateRecord` v2 — cria um record FRESCO (mesmo `class_name`/`name` de um record JÁ VIVO) numa
/// TweakDB isolada (scratch, zerada, com allocator fake só nos 2 HashMaps de topo) e faz
/// `Assign(existing, fresh)` via a vtable do NATIVE TYPE do record — mesma técnica do RED4ext SDK
/// (`TweakDB::UpdateRecord`), sem depender do version-byte (achado não-confiável, ver
/// `prove_updaterecord`/proof 2026-07-17). Log verboso em cada passo (se crashar, o log diz até
/// onde chegou). Devolve `true` só se Assign foi chamado.
///
/// # Safety
/// Chama uma função nativa real (`CreateTDBRecord`) com um ponteiro pra memória NOSSA (não o
/// singleton vivo) — isolado do estado real do jogo até o passo final de Assign, que SIM mexe no
/// record vivo (mutação in-place, mesmo objeto). Rodar de thread separada (mesmo motivo do
/// `create_record_rt`: a native pode pegar um spinlock).
pub unsafe fn update_record_rt(class_name: &str, name: &str) -> bool {
    crate::log(&format!("[updaterec2] entrando: class='{class_name}' name='{name}'"));
    let real_t = match singleton() {
        Some(s) => s,
        None => {
            crate::log("[updaterec2] singleton REAL indisponível — abortado");
            return false;
        }
    };
    let id = tweak_db_id(name);
    let existing = get_record_instance(real_t, id);
    if existing.is_null() || !crate::gum::is_readable(existing as *const c_void, 0x40) {
        crate::log(&format!("[updaterec2] record vivo '{name}' (id={id:#x}) não achado — abortado"));
        return false;
    }
    crate::log(&format!("[updaterec2] record vivo @{existing:p}"));

    // nativeType do record vivo — via GetType virtual (vtbl+8), MESMO getter já usado em class_of.
    let native_type = crate::rtti::class_of(existing as *mut c_void);
    if !crate::gum::is_readable(native_type as *const c_void, 0x60) {
        crate::log("[updaterec2] class_of(existing) ilegível — abortado");
        return false;
    }
    crate::log(&format!("[updaterec2] nativeType={native_type:p}"));

    // 1) TweakDB scratch — 0x168 bytes zerados + vtable fake nos 2 HashMaps de topo.
    let mut fake: [u8; 0x168] = [0u8; 0x168];
    let vtable_ptr = fake_allocator_vtable_ptr();
    // recordsByID@+0x58, seu campo `allocator`(u64)@+0x28 -> absoluto +0x80.
    // recordsByType@+0x88, idem -> absoluto +0xB0.
    fake[0x80..0x88].copy_from_slice(&vtable_ptr.to_le_bytes());
    fake[0xB0..0xB8].copy_from_slice(&vtable_ptr.to_le_bytes());
    let fake_ptr = fake.as_mut_ptr();
    crate::log(&format!("[updaterec2] scratch TweakDB @{fake_ptr:p} (0x168B, allocator fake @{vtable_ptr:#x})"));

    // 2) CreateTDBRecord(scratch, baseHash, id) — MESMO endereço/assinatura de create_record_rt,
    // mas alvo é o SCRATCH, não o singleton real.
    let type_hash = record_type_key(class_name);
    let create: extern "C" fn(*mut c_void, u32, u64) = std::mem::transmute(crate::rebase(ADDR_CREATE_RECORD));
    crate::log(&format!("[updaterec2] chamando CreateTDBRecord(scratch, typeHash={type_hash:#010x}, id={id:#x})..."));
    create(fake_ptr as *mut c_void, type_hash, id);
    crate::log("[updaterec2] CreateTDBRecord retornou");

    // 3) sucesso? recordsByID.size (scratch+0x58+0x08=0x60) != 0.
    let rid_size = (fake_ptr.add(0x60) as *const u32).read_unaligned();
    crate::log(&format!("[updaterec2] scratch.recordsByID.size={rid_size}"));
    if rid_size == 0 {
        crate::log("[updaterec2] CreateTDBRecord não inseriu nada no scratch — abortado (sem Assign)");
        return false;
    }

    // 4) achar o record fresco em scratch.recordsByType (HashMap<IType*,DynArray<Handle<IScriptable>>>
    // @+0x88), chave=native_type (ponteiro), hash=FNV1a32(&key,8) (HashMapHash<T,pointer>).
    let rt_base = fake_ptr.add(0x88);
    let rt_index = (rt_base as *const u64).read_unaligned() as *const u32; // indexTable
    let rt_cap = (rt_base.add(0x0C) as *const u32).read_unaligned();
    let rt_nodes = (rt_base.add(0x10) as *const u64).read_unaligned() as *const u8; // nodeList.nodes
    crate::log(&format!(
        "[updaterec2] scratch.recordsByType: indexTable={rt_index:p} cap={rt_cap} nodes={rt_nodes:p}"
    ));
    if rt_index.is_null() || rt_nodes.is_null() || rt_cap == 0 {
        crate::log("[updaterec2] recordsByType vazio/ilegível após CreateTDBRecord — abortado (sem Assign)");
        return false;
    }
    let key_bytes = (native_type as u64).to_le_bytes();
    let hashed_key = bwms_hashes::fnv1a32(&key_bytes);
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
            crate::log(&format!(
                "[updaterec2] bucket={b} idx={idx} node_hash={node_hash:#x}(nosso={hashed_key:#x}) node_key={node_key:#x}(nativeType={:#x}) entries={dyn_entries:p} size={dyn_size}",
                native_type as u64
            ));
            if !dyn_entries.is_null() && dyn_size > 0 {
                fresh_record = (dyn_entries as *const u64).read_unaligned() as *mut c_void;
                break 'buckets;
            }
            idx = (node.add(0) as *const u32).read_unaligned(); // next
        }
    }
    if fresh_record.is_null() || !crate::gum::is_readable(fresh_record as *const c_void, 0x40) {
        crate::log("[updaterec2] record fresco não achado em recordsByType — abortado (sem Assign)");
        return false;
    }
    crate::log(&format!("[updaterec2] record FRESCO @{fresh_record:p}"));

    // 5) Assign(existing, fresh) via vtable do nativeType, slot +0x58 (macOS).
    let type_vtable = (native_type as *const u64).read_unaligned();
    if type_vtable == 0 || !crate::gum::is_readable(type_vtable as *const c_void, ADDR_ASSIGN_TYPE_VTBL_SLOT_OFF + 8) {
        crate::log("[updaterec2] vtable do nativeType ilegível — abortado (sem Assign)");
        return false;
    }
    let assign_slot = (type_vtable as *const u8).add(ADDR_ASSIGN_TYPE_VTBL_SLOT_OFF) as *const u64;
    let assign_fn = assign_slot.read_unaligned();
    if !crate::gum::is_readable(assign_fn as *const c_void, 4) {
        crate::log("[updaterec2] Assign@vtbl+0x58 ilegível — abortado (sem Assign)");
        return false;
    }
    crate::log(&format!(
        "[updaterec2] chamando Assign(existing={existing:p}, fresh={fresh_record:p}) via vtbl+0x58={assign_fn:#x}..."
    ));
    let assign: extern "C" fn(*mut c_void, *mut c_void, *mut c_void) =
        std::mem::transmute(assign_fn);
    assign(native_type, existing as *mut c_void, fresh_record);
    crate::log("[updaterec2] Assign retornou — UpdateRecord completo (sem crash)");
    true
}

/// Round-trip COMPLETO (canal `updaterecroundtrip`): repointa o índice do flat pra um FlatValue
/// NOVO (mesma técnica de `prove_updaterecord`), varre a memória do record por TODAS as
/// FlatConnections cujo prefixo (hash+len) bate com o flat (sem exigir offset — acha o que
/// REALMENTE está lá), chama `update_record_rt` (CreateTDBRecord+Assign) e varre DE NOVO — a prova
/// é qualquer entrada cujo offset cacheado MUDOU pra `new_off+8` (ou sumiu/reapareceu diferente),
/// confirmando que Assign escreveu algo real no record. Nota: `find_property_in_class` NÃO serve
/// aqui (achado 2026-07-17: flats de TweakDB record não são CProperty comuns — "prop não achada"
/// mesmo numa classe válida — são expostos só via TweakDBInterface/FlatConnection, não RTTI genérica).
/// `tweakxl-updaterecord` v3 (2026-07-18) — ACHADO NOVO: as tentativas v1/v2 (2026-07-17) tentaram
/// ler o campo mudado via `find_property_in_class` (falhou: flats não são CProperty) e via scan de
/// FlatConnection por offset fixo dentro da memória do record (falhou: `Assign` reorganiza a área
/// de conexões cacheadas, offset não é estável) — ambas NUNCA tentaram o caminho mais direto: um
/// RECORD do TweakDB expõe GETTERS NATIVOS reais por campo (`Item_Record.FriendlyName()`,
/// `Grenade_Record.DeepWaterDepth()` — confirmado em `redscript-src/orphans.script:27450`, MESMO
/// campo já usado o projeto inteiro pra testar SetFlat escalar). Esses getters são funções RTTI
/// normais, chamáveis via `rtti::call_func` (o MESMO `callf` já provado em dezenas de gaps hoje) —
/// é o "GetRecord lê o campo mudado" do `proof_needed` LITERAL, não uma reinterpretação. Round-trip:
/// chama o getter (baseline) -> repointa o flat (novo FlatValue, mesma técnica de `prove_updaterecord_v2`)
/// -> chama o getter DE NOVO SEM UpdateRecord (log, não decide nada sozinho) -> chama
/// `update_record_rt` (CreateTDBRecord+Assign, já provado crash-free 3x) -> chama o getter uma 3ª vez.
/// Se a 2ª chamada ainda mostra o valor VELHO e a 3ª mostra o NOVO, é a prova literal do gap.
pub unsafe fn prove_updaterecord_v3(class_name: &str, record_name: &str, getter_name: &str, prop_name: &str, new_val: f32) {
    crate::log(&format!(
        "[updaterec3] ==== round-trip via GETTER NATIVO: {record_name}.{getter_name}() (flat={prop_name}) -> {new_val} ===="
    ));
    let reg = match crate::rtti::Registry::obtain() {
        Some(r) => r,
        None => {
            crate::log("[updaterec3] Registry indisponível — abortado");
            return;
        }
    };
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log("[updaterec3] singleton indisponível — abortado");
            return;
        }
    };
    let id = tweak_db_id(record_name);
    let existing = get_record_instance(t, id);
    if existing.is_null() || !crate::gum::is_readable(existing as *const c_void, 0x40) {
        crate::log(&format!("[updaterec3] record '{record_name}' não achado — abortado"));
        return;
    }
    crate::log(&format!("[updaterec3] record vivo @{existing:p}"));

    let rf = match crate::rtti::resolve_func(&reg, class_name, getter_name) {
        Some(rf) => rf,
        None => {
            crate::log(&format!("[updaterec3] getter '{class_name}.{getter_name}' não resolveu — abortado"));
            return;
        }
    };

    let read_via_getter = |tag: &str| -> Option<f32> {
        let ret = crate::rtti::call_func(&rf, existing as *mut c_void, &[]);
        match ret {
            Some(buf) => {
                let v = f32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                crate::log(&format!("[updaterec3] {tag}: {getter_name}() -> {v}"));
                Some(v)
            }
            None => {
                crate::log(&format!("[updaterec3] {tag}: {getter_name}() chamada falhou (call_func None)"));
                None
            }
        }
    };

    let baseline = read_via_getter("1) ANTES de qualquer mutação (baseline)");

    let full_flat = format!("{record_name}.{prop_name}");
    let new_off = match create_flat_value(t, &full_flat, &new_val.to_le_bytes(), 8) {
        Some(o) => o as u32,
        None => {
            crate::log("[updaterec3] create_flat_value falhou — abortado");
            return;
        }
    };
    if !set_flat_offset(t as *const u8, &full_flat, new_off) {
        crate::log("[updaterec3] set_flat_offset (repoint do índice global) falhou — abortado");
        return;
    }
    crate::log(&format!("[updaterec3] 2) índice global repontado -> offset {new_off:#x} (FlatValue novo = {new_val})"));

    let sem_update = read_via_getter("3) getter SEM UpdateRecord ainda (deve ser igual ao baseline se o getter cacheia)");

    let updated = update_record_rt(class_name, record_name);
    crate::log(&format!("[updaterec3] 4) update_record_rt (CreateTDBRecord+Assign) retornou {updated}"));

    let com_update = read_via_getter("5) getter COM UpdateRecord (deve ser new_val se o mecanismo funciona)");

    let bateu = updated
        && sem_update.is_some_and(|v| (v - baseline.unwrap_or(f32::NAN)).abs() < 0.001)
        && com_update.is_some_and(|v| (v - new_val).abs() < 0.001);
    crate::log(&format!(
        "[updaterec3] VEREDITO: {}",
        if bateu {
            ">>> UPDATERECORD V3 PROVADO: getter nativo leu o valor VELHO antes do UpdateRecord e o NOVO só depois <<<"
        } else if com_update.is_some_and(|v| (v - new_val).abs() < 0.001) && sem_update.is_some_and(|v| (v - new_val).abs() < 0.001) {
            "getter já mostrava o valor novo MESMO SEM UpdateRecord — o getter não cacheia (lê o índice global direto); UpdateRecord não é observável por este caminho"
        } else {
            "NÃO bateu — ver os valores acima (baseline/sem-update/com-update)"
        }
    ));
}

pub unsafe fn prove_updaterecord_v2(class_name: &str, record_name: &str, prop_name: &str, new_val: f32) {
    crate::log(&format!("[updaterec2-rt] ==== round-trip: {record_name}.{prop_name} -> {new_val} ===="));
    let t = match singleton() {
        Some(s) => s,
        None => {
            crate::log("[updaterec2-rt] singleton indisponível — abortado");
            return;
        }
    };
    let id = tweak_db_id(record_name);
    let record = get_record_instance(t, id);
    if record.is_null() || !crate::gum::is_readable(record as *const c_void, 0x40) {
        crate::log(&format!("[updaterec2-rt] record '{record_name}' não achado — abortado"));
        return;
    }
    let full_flat = format!("{record_name}.{prop_name}");
    let flat_id = tweak_db_id(&full_flat);

    let before = find_flat_connections_by_prefix(record, flat_id);
    crate::log(&format!("[updaterec2-rt] 1) FlatConnections ANTES (prefixo bate): {before:?}"));

    let new_off = match create_flat_value(t, &full_flat, &new_val.to_le_bytes(), 8) {
        Some(o) => o as u32,
        None => {
            crate::log("[updaterec2-rt] create_flat_value falhou — abortado");
            return;
        }
    };
    if !set_flat_offset(t as *const u8, &full_flat, new_off) {
        crate::log("[updaterec2-rt] set_flat_offset (repoint) falhou — abortado");
        return;
    }
    let expected_new = new_off + 8;
    crate::log(&format!(
        "[updaterec2-rt] 2) repoint feito: índice -> offset {new_off:#x} (esperado cache pós-update = {expected_new:#x}, FlatValue novo = {new_val})"
    ));

    let sem_update = find_flat_connections_by_prefix(record, flat_id);
    crate::log(&format!("[updaterec2-rt] 3) FlatConnections SEM UpdateRecord ainda: {sem_update:?}"));

    let updated = update_record_rt(class_name, record_name);
    crate::log(&format!("[updaterec2-rt] 4) update_record_rt retornou {updated}"));

    let com_update = find_flat_connections_by_prefix(record, flat_id);
    crate::log(&format!("[updaterec2-rt] 5) FlatConnections COM UpdateRecord: {com_update:?}"));

    let mudou = com_update.iter().any(|&(_, off)| off == expected_new)
        && !sem_update.iter().any(|&(_, off)| off == expected_new);
    crate::log(&format!(
        "[updaterec2-rt] VEREDITO: {}",
        if updated && mudou {
            ">>> UPDATERECORD V2 OK: apareceu uma FlatConnection com offset=new_off+8 SÓ DEPOIS do UpdateRecord (não antes) <<<"
        } else {
            "NÃO bateu — ver as listas de FlatConnections acima (antes/sem/com update)"
        }
    ));
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
mod clone_logic_tests {
    use super::{flat_key_present, id_key40, id_key_lt, merge_insert_from_end};

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
    fn key40_tira_o_offset() {
        assert_eq!(id_key40(ent(0x1234, 7, 0xFFFFFF)), ent(0x1234, 7, 0));
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
