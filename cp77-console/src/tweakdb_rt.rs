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

// ===== hashes (cópia exata de tweakdb-tool/src/hashes.rs) =====
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
/// TweakDBID = crc32(nome) | (len << 32).
pub fn tweak_db_id(name: &str) -> u64 {
    u64::from(crc32(name.as_bytes())) | ((name.len() as u64) << 32)
}
const RECORDS_SEED: u32 = 0x5EED_BA5E;
fn murmur3_32(data: &[u8], seed: u32) -> u32 {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;
    let mut h = seed;
    let nblocks = data.len() / 4;
    for i in 0..nblocks {
        let mut k = u32::from_le_bytes([data[i * 4], data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3]]);
        k = k.wrapping_mul(C1);
        k = k.rotate_left(15);
        k = k.wrapping_mul(C2);
        h ^= k;
        h = h.rotate_left(13);
        h = h.wrapping_mul(5).wrapping_add(0xe654_6b64);
    }
    let tail = &data[nblocks * 4..];
    let mut k1 = 0u32;
    if tail.len() >= 3 {
        k1 ^= u32::from(tail[2]) << 16;
    }
    if tail.len() >= 2 {
        k1 ^= u32::from(tail[1]) << 8;
    }
    if !tail.is_empty() {
        k1 ^= u32::from(tail[0]);
        k1 = k1.wrapping_mul(C1);
        k1 = k1.rotate_left(15);
        k1 = k1.wrapping_mul(C2);
        h ^= k1;
    }
    h ^= data.len() as u32;
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}
/// type_key da classe RED: murmur3(miolo de gamedata(.*)_Record, seed 0x5EEDBA5E).
pub fn record_type_key(class_name: &str) -> u32 {
    let core = class_name
        .strip_prefix("gamedata")
        .and_then(|s| s.strip_suffix("_Record"))
        .unwrap_or(class_name);
    murmur3_32(core.as_bytes(), RECORDS_SEED)
}

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
    // global vazio/staging → Get() (lazy-init, devolve a TweakDB REAL carregada)
    let get: extern "C" fn() -> *mut c_void = std::mem::transmute(crate::rebase(ADDR_TWEAKDB_GET));
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
const FLAT_DATA_BUFFER_OFF: usize = 0x148;

/// FlatValue* de um flat por NOME (binary-search no SortedUniqueArray + tdbOffset BE). None se
/// não achar. NÃO pega lock (read-only é multi-thread-safe; ver nota do RED4ext).
pub unsafe fn get_flat_value(t: *const u8, name: &str) -> Option<*mut u8> {
    let id = tweak_db_id(name);
    let q_hash = (id & 0xFFFF_FFFF) as u32; // CRC32 = chave PRIMÁRIA da ordenação
    let q_len = ((id >> 32) & 0xFF) as u8; //  length = chave SECUNDÁRIA
    let entries = rd_u64(t.add(FLATS_OFF)) as *const u64;
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
        if !crate::gum::is_readable(entries.add(mid as usize) as *const c_void, 8) {
            return None;
        }
        let e = entries.add(mid as usize).read_unaligned();
        // ORDENAÇÃO do jogo (TweakDBID::operator<, NativeTypes-inl.hpp): PRIMÁRIO hash(u32),
        // SECUNDÁRIO length(u8). NÃO é o u64 combinado (lá o length, em bits altos, dominaria).
        let e_hash = (e & 0xFFFF_FFFF) as u32;
        let e_len = ((e >> 32) & 0xFF) as u8;
        let less = e_hash < q_hash || (e_hash == q_hash && e_len < q_len);
        let greater = e_hash > q_hash || (e_hash == q_hash && e_len > q_len);
        if less {
            lo = mid + 1;
        } else if greater {
            hi = mid;
        } else {
            // achou: tdbOffset = bytes 5,6,7 em BIG-ENDIAN
            let b5 = ((e >> 40) & 0xFF) as u32;
            let b6 = ((e >> 48) & 0xFF) as u32;
            let b7 = ((e >> 56) & 0xFF) as u32;
            let off = (b5 << 16) | (b6 << 8) | b7;
            let fdb = rd_u64(t.add(FLAT_DATA_BUFFER_OFF)) as *mut u8;
            if fdb.is_null() {
                return None;
            }
            let fv = fdb.add(off as usize);
            return if crate::gum::is_readable(fv as *const c_void, 0x18) { Some(fv) } else { None };
        }
    }
    None
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
