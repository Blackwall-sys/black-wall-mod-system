//! Texto e números: strings UTF-8, leitura de inteiros, bits e os hashes do jogo (FNV-1a, Murmur3, CRUID, EntityID, NodeRef).

#![allow(dead_code)]

use crate::mods_do_usuario::tramp_modlog;
use crate::mods_do_usuario::tramp_print;
use crate::salvamento::tramp_persistentid_hash;
use crate::aparencia::write_cstring_chunk_ret;
use crate::mundo::*;
use crate::tipos_do_motor::*;
use crate::arquivos_do_jogo::*;
use std::ffi::c_void;
use crate::rtti::{self, Registry};
use crate::register::*;

// ===== Hash de texto (2026-07-13) — fecha a represa de leitura de
// `String` nativa via `read_cstring`. Assinatura:
// `FNV1a64(data: script_ref<String>, opt seed: Uint64=0xCBF29CE484222325) -> Uint64` (e
// variantes 32-bit/Murmur3). Registrado com `String` puro (não `script_ref<String>` — o nome
// EXATO do tipo wrapper no RTTI é incerto sem teste ao vivo; `String` reduz risco de bind
// falhar, ao custo de possível incompatibilidade estrita com mods reais que dependam do
// wrapper — ajustar se a 1ª prova ao vivo mostrar que não bindou). CODADO, SEM confirmação
// in-game ainda (ambiente bloqueado no momento da implementação).
pub(crate) unsafe extern "C" fn tramp_fnv1a64(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let data = strings.first().cloned().flatten().unwrap_or_default();
    let seed = args.get(1).map(|(v, _)| *v).unwrap_or(bwms_primitives::FNV1A64_OFFSET);
    let hash = bwms_primitives::fnv1a64_seeded(data.as_bytes(), seed);
    crate::log_dev!("[hashutils] FNV1a64('{data}', seed={seed:#x}) = {hash:#x}");
    write_uint_ret(out, hash, 8);
}

pub(crate) unsafe extern "C" fn tramp_fnv1a32(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let data = strings.first().cloned().flatten().unwrap_or_default();
    let seed = args.get(1).map(|(v, _)| *v as u32).unwrap_or(bwms_primitives::FNV1A32_OFFSET);
    let hash = bwms_primitives::fnv1a32_seeded(data.as_bytes(), seed);
    crate::log_dev!("[hashutils] FNV1a32('{data}', seed={seed:#x}) = {hash:#x}");
    write_uint_ret(out, hash as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_murmur3(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let data = strings.first().cloned().flatten().unwrap_or_default();
    let seed = args.get(1).map(|(v, _)| *v as u32).unwrap_or(0x5eed_ba5e);
    let hash = bwms_primitives::murmur3_32(data.as_bytes(), seed);
    crate::log_dev!("[hashutils] Murmur3('{data}', seed={seed:#x}) = {hash:#x}");
    write_uint_ret(out, hash as u64, 4);
}

/// Registra as 3 funções de hash com nome próprio `Bwms*`. Ver nota acima sobre o tipo do
/// 1º param (`String` em vez de `script_ref<String>`).
pub(crate) unsafe fn register_hash_utils(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &["String", "Uint64"], "BwmsFNV1a64", "BwmsFNV1a64", tramp_fnv1a64);
    let ok2 = register_argful_by_types(reg, proto, &["String", "Uint32"], "BwmsFNV1a32", "BwmsFNV1a32", tramp_fnv1a32);
    let ok3 = register_argful_by_types(reg, proto, &["String", "Uint32"], "BwmsMurmur3", "BwmsMurmur3", tramp_murmur3);
    crate::log_dev!("[reg] register_all: hash BwmsFNV1a64={ok1} BwmsFNV1a32={ok2} BwmsMurmur3={ok3}");
}

pub(crate) unsafe extern "C" fn tramp_utf8strlen(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let len = utf8_str_len(&s);
    crate::log_dev!("[strutils] UTF8StrLen('{s}') = {len}");
    write_uint_ret(out, len as u32 as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_utf8strleft(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let length = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
    let r = utf8_str_left(&s, length);
    let ok = write_cstring_inline_ret(out, &r);
    crate::log_dev!("[strutils] UTF8StrLeft('{s}',{length}) = '{r}' (escrito={ok})");
}

pub(crate) unsafe extern "C" fn tramp_utf8strright(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let length = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
    let r = utf8_str_right(&s, length);
    let ok = write_cstring_inline_ret(out, &r);
    crate::log_dev!("[strutils] UTF8StrRight('{s}',{length}) = '{r}' (escrito={ok})");
}

pub(crate) unsafe extern "C" fn tramp_utf8strmid(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let offset = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
    let length = args.get(2).map(|(v, _)| *v as i32).unwrap_or(0);
    let r = utf8_str_mid(&s, offset, length);
    let ok = write_cstring_inline_ret(out, &r);
    crate::log_dev!("[strutils] UTF8StrMid('{s}',{offset},{length}) = '{r}' (escrito={ok})");
}

pub(crate) unsafe extern "C" fn tramp_utf8strlower(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let r = utf8_str_lower(&s);
    let ok = write_cstring_inline_ret(out, &r);
    crate::log_dev!("[strutils] UTF8StrLower('{s}') = '{r}' (escrito={ok})");
}

pub(crate) unsafe extern "C" fn tramp_utf8strupper(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let r = utf8_str_upper(&s);
    let ok = write_cstring_inline_ret(out, &r);
    crate::log_dev!("[strutils] UTF8StrUpper('{s}') = '{r}' (escrito={ok})");
}

/// `BwmsUTF8Split(s) -> Int32`: quebra o texto em codepoints, guarda nesta thread e devolve quantos. O texto é copiado uma vez por quebra; o
/// `BwmsUTF8SplitChar` lê cada caractere sem receber o texto.
pub(crate) unsafe extern "C" fn tramp_utf8split(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.into_iter().next().flatten().unwrap_or_default();
    let n = utf8_split(&s);
    crate::log_dev!("[strutils] UTF8Split(bytes={}) = {n}", s.len());
    write_uint_ret(out, n as u32 as u64, 4);
}

/// `BwmsUTF8SplitChar(index) -> String`: caractere `index` do último `BwmsUTF8Split` desta thread; vazio fora do alcance. Sem log: roda uma vez
/// por caractere.
pub(crate) unsafe extern "C" fn tramp_utf8splitchar(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let indice = args.first().map(|(v, _)| *v as i32).unwrap_or(-1);
    let mut buf = [0u8; 4];
    let s: &str = match utf8_split_char(indice) {
        Some(c) => c.encode_utf8(&mut buf),
        None => "",
    };
    write_cstring_inline_ret(out, s);
}

/// Registra as 6 funções de texto com nome próprio `Bwms*` e a quebra em caracteres do medidor de texto.
pub(crate) unsafe fn register_string_utils(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &["String"], "BwmsUTF8StrLen", "BwmsUTF8StrLen", tramp_utf8strlen);
    let ok2 = register_argful_by_types(reg, proto, &["String", "Int32"], "BwmsUTF8StrLeft", "BwmsUTF8StrLeft", tramp_utf8strleft);
    let ok3 = register_argful_by_types(reg, proto, &["String", "Int32"], "BwmsUTF8StrRight", "BwmsUTF8StrRight", tramp_utf8strright);
    let ok4 = register_argful_by_types(reg, proto, &["String", "Int32", "Int32"], "BwmsUTF8StrMid", "BwmsUTF8StrMid", tramp_utf8strmid);
    let ok5 = register_argful_by_types(reg, proto, &["String"], "BwmsUTF8StrLower", "BwmsUTF8StrLower", tramp_utf8strlower);
    let ok6 = register_argful_by_types(reg, proto, &["String"], "BwmsUTF8StrUpper", "BwmsUTF8StrUpper", tramp_utf8strupper);
    let ok7 = register_argful_by_types(reg, proto, &["String"], "BwmsUTF8Split", "BwmsUTF8Split", tramp_utf8split);
    let ok8 = register_argful_by_types(reg, proto, &["Int32"], "BwmsUTF8SplitChar", "BwmsUTF8SplitChar", tramp_utf8splitchar);
    crate::log_dev!(
        "[reg] register_all: texto Len={ok1} Left={ok2} Right={ok3} Mid={ok4} Lower={ok5} Upper={ok6} Split={ok7} SplitChar={ok8}"
    );
}

/// `BwmsLongStr(fonte: String, arg: String, i: Int32) -> String` — string longa em PEDAÇOS.
///
/// UMA native pra todos os produtores de string longa, em vez de uma variante `...Chunk` por
/// native (que seriam ~38 cópias da mesma coisa — exatamente o espaguete que a regra de estilo do
/// projeto manda matar). O redscript junta com `BwmsLongStrJoin`.
///
/// `fonte` é validada contra um `match` FIXO: o redscript nunca escolhe um caminho de código
/// arbitrário, só um dos nomes listados aqui.
pub(crate) unsafe extern "C" fn tramp_long_str(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let fonte = strings.first().cloned().flatten().unwrap_or_default();
    let arg = strings.get(1).cloned().flatten().unwrap_or_default();
    let i = a.get(2).map(|(v, _)| *v as usize).unwrap_or(0);
    let inteiro: String = match fonte.as_str() {
        // classe RTTI do recurso carregado — o caso que fez `#86` e `A#49` lerem `''` como
        // "não está no cache" quando o nome só era comprido demais.
        "sectorclass" | "depotclass" => {
            depot_loaded_resource_class_by_hash(bwms_primitives::resource_path_hash(&arg))
        }
        // classe do recurso por HASH em hex (quando quem chama já tem o hash, não o path).
        "depotclasshash" => {
            let h = u64::from_str_radix(arg.trim_start_matches("0x"), 16).unwrap_or(0);
            if h == 0 { String::new() } else { depot_loaded_resource_class_by_hash(h) }
        }
        _ => String::new(),
    };
    let _ = write_cstring_chunk_ret(out, &inteiro, i);
}

/// Identidade de 8 bytes das conversões Hash<->Tipo: CName, CRUID, NodeRef, EntityID, TweakDBID, ResRef e EngineTime são um
/// `uint64_t` por baixo, então a native copia os 8 bytes que o descritor da native
/// corrente leu do frame. Só serve para retorno de 8 bytes: uma native com retorno menor sobrescreveria memória além dele.
/// `EngineTime.GetFrequency` não passa por aqui: lê um `double` num endereço do motor ainda sem RE.
pub(crate) unsafe extern "C" fn tramp_identidade_8_bytes(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let v = args.first().map(|(v, _)| *v).unwrap_or(0);
    write_uint_ret(out, v, 8);
}

// ===== Bits (2026-07-13) — só argumentos numéricos. Nome próprio
// `Bwms*` (`BwmsBitTest32` etc.): um `.reds` de Windows que chame `BitTest32`
// não compila contra o bundle.
#[inline]
pub(crate) fn bit_test(value: u64, n: i32) -> bool {
    if !(0..64).contains(&n) {
        return false;
    }
    (value >> n) & 1 != 0
}

#[inline]
pub(crate) fn bit_set(value: u64, n: i32, state: bool) -> u64 {
    if !(0..64).contains(&n) {
        return value;
    }
    if state { value | (1u64 << n) } else { value & !(1u64 << n) }
}

#[inline]
pub(crate) fn bit_shl(value: u64, n: i32, width_bits: u32) -> u64 {
    if n < 0 {
        return value;
    }
    let shifted = value.wrapping_shl(n as u32);
    let mask = if width_bits >= 64 { u64::MAX } else { (1u64 << width_bits) - 1 };
    shifted & mask
}

#[inline]
pub(crate) fn bit_shr(value: u64, n: i32) -> u64 {
    if n < 0 { value } else { value.wrapping_shr(n as u32) }
}

/// Gera os 4 trampolines (`Test`/`Set`/`ShiftL`/`ShiftR`) de UMA largura de bits. `read_width` =
/// bytes do arg `value` (1/2/4/8); `width_bits` = bits totais (pro mask do shift-left).
macro_rules! bits_width_trampolines {
    ($test_fn:ident, $set_fn:ident, $shl_fn:ident, $shr_fn:ident, $read_width:literal, $width_bits:literal) => {
        unsafe extern "C" fn $test_fn(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
            let args = rtti::read_params_consuming(current_native_func(), frame);
            let value = mask_arg(args.first().map(|(v, _)| *v).unwrap_or(0), $read_width);
            let n = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
            write_bool_ret(out, bit_test(value, n));
        }
        unsafe extern "C" fn $set_fn(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
            let args = rtti::read_params_consuming(current_native_func(), frame);
            let value = mask_arg(args.first().map(|(v, _)| *v).unwrap_or(0), $read_width);
            let n = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
            let state = args.get(2).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
            write_uint_ret(out, bit_set(value, n, state), $read_width);
        }
        unsafe extern "C" fn $shl_fn(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
            let args = rtti::read_params_consuming(current_native_func(), frame);
            let value = mask_arg(args.first().map(|(v, _)| *v).unwrap_or(0), $read_width);
            let n = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
            write_uint_ret(out, bit_shl(value, n, $width_bits), $read_width);
        }
        unsafe extern "C" fn $shr_fn(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
            let args = rtti::read_params_consuming(current_native_func(), frame);
            let value = mask_arg(args.first().map(|(v, _)| *v).unwrap_or(0), $read_width);
            let n = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
            write_uint_ret(out, bit_shr(value, n), $read_width);
        }
    };
}

#[inline]
pub(crate) fn mask_arg(raw: u64, width_bytes: u8) -> u64 {
    match width_bytes {
        1 => raw as u8 as u64,
        2 => raw as u16 as u64,
        4 => raw as u32 as u64,
        _ => raw,
    }
}

bits_width_trampolines!(tramp_bittest8, tramp_bitset8, tramp_bitshl8, tramp_bitshr8, 1, 8);

bits_width_trampolines!(tramp_bittest16, tramp_bitset16, tramp_bitshl16, tramp_bitshr16, 2, 16);

bits_width_trampolines!(tramp_bittest32, tramp_bitset32, tramp_bitshl32, tramp_bitshr32, 4, 32);

bits_width_trampolines!(tramp_bittest64, tramp_bitset64, tramp_bitshl64, tramp_bitshr64, 8, 64);

/// Registra as 16 funções de bits com nome próprio `Bwms*` (`BwmsBitTest32` etc.). `proto` = doador de vtable
/// já resolvido em `register_all`.
pub(crate) unsafe fn register_bits_utils(reg: &Registry, proto: *mut c_void) {
    let uint_t = |bits: u32| if bits == 8 { "Uint8" } else if bits == 16 { "Uint16" } else if bits == 32 { "Uint32" } else { "Uint64" };
    let mut n_ok = 0usize;
    macro_rules! reg1 {
        ($bits:literal, $test_fn:ident, $set_fn:ident, $shl_fn:ident, $shr_fn:ident) => {
            let u = uint_t($bits);
            if register_argful_by_types(reg, proto, &[u, "Int32"], concat!("BwmsBitTest", $bits), concat!("BwmsBitTest", $bits), $test_fn) { n_ok += 1; }
            if register_argful_by_types(reg, proto, &[u, "Int32", "Bool"], concat!("BwmsBitSet", $bits), concat!("BwmsBitSet", $bits), $set_fn) { n_ok += 1; }
            if register_argful_by_types(reg, proto, &[u, "Int32"], concat!("BwmsBitShiftL", $bits), concat!("BwmsBitShiftL", $bits), $shl_fn) { n_ok += 1; }
            if register_argful_by_types(reg, proto, &[u, "Int32"], concat!("BwmsBitShiftR", $bits), concat!("BwmsBitShiftR", $bits), $shr_fn) { n_ok += 1; }
        };
    }
    reg1!(8, tramp_bittest8, tramp_bitset8, tramp_bitshl8, tramp_bitshr8);
    reg1!(16, tramp_bittest16, tramp_bitset16, tramp_bitshl16, tramp_bitshr16);
    reg1!(32, tramp_bittest32, tramp_bitset32, tramp_bitshl32, tramp_bitshr32);
    reg1!(64, tramp_bittest64, tramp_bitset64, tramp_bitshl64, tramp_bitshr64);
    crate::log_dev!("[reg] register_all: bits {n_ok}/16 registradas (nome próprio BWMS — ver dist/MODDER-API.md)");
}

// ===== Números (2026-07-13) — ParseInt8/16/32/64 + ParseUint8/16/
// 32/64, mesma represa de String que o hash acabou de fechar (`read_cstring`). Contrato:
// `T ParseInt<T>(str, opt base=10)` — semântica de
// `strtoll`/`strtoull` (int64 p/ signed, uint64 p/ unsigned), e SE `end != str+len` (sobrou
// lixo não consumido) retorna 0; senão trunca pro tipo alvo. Semântica:
// espaço em branco à esquerda ignorado, sinal opcional (`strtoull` aceita `-` também — nega mod
// 2^64, igual ao glibc), base 0 = auto-detecta `0x`/`0X`->16 ou `0`->8 senão 10; overflow CLAMPA em
// i64::MIN/MAX ou u64::MAX (mimetiza o comportamento real do strtoll/strtoull sem checar errno) em
// vez de dar panic/wrap arbitrário. Exige consumir a string INTEIRA (equivalente ao `end` check) —
// qualquer caractere sobrando (mesmo espaço à direita) já falha e retorna 0.
pub(crate) fn parse_number_core(s: &str, base_in: i32, accept_leading_minus_unsigned: bool) -> Option<(usize, bool, u128)> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
        i += 1;
    }
    let mut neg = false;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        if bytes[i] == b'-' && !accept_leading_minus_unsigned {
            neg = true;
        } else if bytes[i] == b'-' {
            neg = true; // strtoull aceita '-' também (nega mod 2^64 no fim)
        }
        i += 1;
    }
    let mut base = base_in;
    if base == 0 {
        if i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] | 0x20) == b'x' {
            base = 16;
            i += 2;
        } else if i < bytes.len() && bytes[i] == b'0' {
            base = 8;
            i += 1;
        } else {
            base = 10;
        }
    } else if base == 16 && i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] | 0x20) == b'x' {
        i += 2;
    }
    if !(2..=36).contains(&base) {
        return None;
    }
    let digit_start = i;
    let mut acc: u128 = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let d = match c {
            b'0'..=b'9' => (c - b'0') as u32,
            b'a'..=b'z' => (c - b'a' + 10) as u32,
            b'A'..=b'Z' => (c - b'A' + 10) as u32,
            _ => break,
        };
        if d as i32 >= base {
            break;
        }
        acc = acc.saturating_mul(base as u128).saturating_add(d as u128);
        acc = acc.min(u64::MAX as u128);
        i += 1;
    }
    if i == digit_start {
        return None; // nenhum dígito consumido (strtoll deixa end==nptr, sempre falha o whole-string check)
    }
    Some((i, neg, acc))
}

/// `strtoll`-like: retorna `None` se não consumir a string inteira ou faltar dígito; clampa em
/// i64::MIN/MAX no overflow (mimetiza strtoll sem checar errno).
pub(crate) fn strtoll_like(s: &str, base: i32) -> Option<i64> {
    let (consumed, neg, acc) = parse_number_core(s, base, false)?;
    if consumed != s.len() {
        return None;
    }
    Some(if neg {
        if acc > (i64::MAX as u128) + 1 { i64::MIN } else { (acc as i64).wrapping_neg() }
    } else if acc > i64::MAX as u128 {
        i64::MAX
    } else {
        acc as i64
    })
}

/// `strtoull`-like: aceita `-` líder (nega mod 2^64, igual ao glibc); clampa em u64::MAX no overflow.
pub(crate) fn strtoull_like(s: &str, base: i32) -> Option<u64> {
    let (consumed, neg, acc) = parse_number_core(s, base, true)?;
    if consumed != s.len() {
        return None;
    }
    let v = acc.min(u64::MAX as u128) as u64;
    Some(if neg { v.wrapping_neg() } else { v })
}

macro_rules! parse_int_trampoline {
    ($name:ident, $width:literal) => {
        unsafe extern "C" fn $name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
            let func = current_native_func();
            let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
            let s = strings.first().cloned().flatten().unwrap_or_default();
            let base = args.get(1).map(|(v, _)| *v as i32).unwrap_or(10);
            let val = strtoll_like(&s, base).unwrap_or(0);
            crate::log_dev!("[numutils] ParseInt{}('{s}', base={base}) = {val}", $width);
            write_uint_ret(out, val as u64, $width / 8);
        }
    };
}

macro_rules! parse_uint_trampoline {
    ($name:ident, $width:literal) => {
        unsafe extern "C" fn $name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
            let func = current_native_func();
            let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
            let s = strings.first().cloned().flatten().unwrap_or_default();
            let base = args.get(1).map(|(v, _)| *v as i32).unwrap_or(10);
            let val = strtoull_like(&s, base).unwrap_or(0);
            crate::log_dev!("[numutils] ParseUint{}('{s}', base={base}) = {val}", $width);
            write_uint_ret(out, val, $width / 8);
        }
    };
}

parse_int_trampoline!(tramp_parseint8, 8);

parse_int_trampoline!(tramp_parseint16, 16);

parse_int_trampoline!(tramp_parseint32, 32);

parse_int_trampoline!(tramp_parseint64, 64);

parse_uint_trampoline!(tramp_parseuint8, 8);

parse_uint_trampoline!(tramp_parseuint16, 16);

parse_uint_trampoline!(tramp_parseuint32, 32);

parse_uint_trampoline!(tramp_parseuint64, 64);

/// Registra as 8 funções de número com nome próprio `Bwms*`. Mesma incerteza do hash sobre
/// `String` vs `script_ref<String>` — ver comentário lá.
pub(crate) unsafe fn register_number_utils(reg: &Registry, proto: *mut c_void) {
    let mut n_ok = 0usize;
    macro_rules! reg1 {
        ($ret_t:literal, $name:literal, $fn:ident) => {
            if register_argful_by_types(reg, proto, &["String", "Int32"], $name, $name, $fn) {
                n_ok += 1;
            }
            let _ = $ret_t;
        };
    }
    reg1!("Int8", "BwmsParseInt8", tramp_parseint8);
    reg1!("Int16", "BwmsParseInt16", tramp_parseint16);
    reg1!("Int32", "BwmsParseInt32", tramp_parseint32);
    reg1!("Int64", "BwmsParseInt64", tramp_parseint64);
    reg1!("Uint8", "BwmsParseUint8", tramp_parseuint8);
    reg1!("Uint16", "BwmsParseUint16", tramp_parseuint16);
    reg1!("Uint32", "BwmsParseUint32", tramp_parseuint32);
    reg1!("Uint64", "BwmsParseUint64", tramp_parseuint64);
    crate::log_dev!("[reg] register_all: números {n_ok}/8 registradas (nome próprio BWMS)");
}

// ===== Texto (2026-07-13) — as 6 funções reais. `UTF8StrLen`
// é retorno escalar (Int32), mesmo padrão de hash/número. As outras 5 retornam `String` —
// precisavam de um mecanismo de ESCREVER uma CString NOVA de volta pro `out` (represa distinta
// da de LER, que já tinha caído com `read_cstring`). Fechada na MESMA sessão: `call_func::res`
// era `[u8;16]` (dimensionado pro maior retorno já testado, Vector4) — alargar pra 0x20 (ver
// nota lá) tornou seguro escrever uma CString inteira sem transbordar. `write_cstring_inline_ret`
// só cobre o caminho SSO (<=19 bytes) — strings mais longas exigiriam heap-alloc no allocator do
// PRÓPRIO jogo (fora de escopo, risco real de corrupção se malfeito).
// Semântica: Left/Right/Mid operam em
// CODEPOINTS UTF-8 (não bytes, mesmo padrão de UTF8StrLen); Lower/Upper via
// `char::to_lowercase/to_uppercase` do Rust (cobre expansões tipo alemão ß→"ss").
#[inline]
pub(crate) fn utf8_str_len(s: &str) -> i32 {
    s.chars().count() as i32
}

#[inline]
pub(crate) fn utf8_str_left(s: &str, length: i32) -> String {
    if length <= 0 {
        return String::new();
    }
    s.chars().take(length as usize).collect()
}

#[inline]
pub(crate) fn utf8_str_right(s: &str, length: i32) -> String {
    if length <= 0 {
        return String::new();
    }
    let total = s.chars().count() as i64;
    let skip = (total - length as i64).max(0) as usize;
    s.chars().skip(skip).collect()
}

#[inline]
pub(crate) fn utf8_str_mid(s: &str, offset: i32, length: i32) -> String {
    if length <= 0 {
        return String::new();
    }
    s.chars().skip(offset.max(0) as usize).take(length as usize).collect()
}

#[inline]
pub(crate) fn utf8_str_lower(s: &str) -> String {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}

#[inline]
pub(crate) fn utf8_str_upper(s: &str) -> String {
    s.chars().flat_map(|c| c.to_uppercase()).collect()
}

// Codepoints do último `utf8_split` desta thread. O medidor de texto quebra o texto e lê os caracteres pelo índice logo em seguida, na mesma
// função de script e sem chamar script no meio, então nenhuma outra quebra entra entre as duas. Por thread porque script também roda em thread de
// job. Sem caminho de pânico no dylib: `Cell` com `take`/`set` em vez de `RefCell`, e `try_with` em vez de `with` (a thread em encerramento já
// destruiu o `Vec`; aí a quebra dá 0 e o caractere sai vazio). O `Vec` fica com a capacidade da maior quebra da thread.
thread_local! {
    static QUEBRA_UTF8: std::cell::Cell<Vec<char>> = const { std::cell::Cell::new(Vec::new()) };
}

/// Quebra `s` em codepoints, a mesma divisão de `utf8_str_mid(s, i, 1)`, e guarda nesta thread. Devolve quantos.
pub(crate) fn utf8_split(s: &str) -> i32 {
    QUEBRA_UTF8
        .try_with(|q| {
            let mut chars = q.take();
            chars.clear();
            chars.extend(s.chars());
            let n = chars.len() as i32;
            q.set(chars);
            n
        })
        .unwrap_or(0)
}

/// Codepoint `indice` do último `utf8_split` desta thread, ou `None` fora do alcance.
pub(crate) fn utf8_split_char(indice: i32) -> Option<char> {
    let i = usize::try_from(indice).ok()?;
    QUEBRA_UTF8
        .try_with(|q| {
            let chars = q.take();
            let c = chars.get(i).copied();
            q.set(chars);
            c
        })
        .ok()
        .flatten()
}

/// Registra `Utils/CName.reds` (2/2), `Utils/CRUID.reds` (2/2), `Utils/NodeRef.reds` (3/3 —
/// `CreateNodeRef` FECHADO 2026-08-15, rodada 20, ver nota acima) e `Utils/Logging.reds` (2/2,
/// loga no dev-log).
/// Utilitários de hash, nome, tag, distrito e pilha que o script usa: `(nome, tipos dos
/// argumentos, trampolim)`. Tabela em vez de uma chamada inline por native — os dados vão pro
/// `.rodata` e o registro é um laço só. Mesmo efeito, binário menor, e a lista se lê de cima a
/// baixo em vez de espalhada em 20 `let ok`.
pub(crate) static UTILS_DE_HASH_E_NOME: &[(&str, &[&str], NativeHandler)] = &[
    ("BwmsHashToName", &["Uint64"], tramp_identidade_8_bytes),
    ("BwmsNameToHash", &["CName"], tramp_identidade_8_bytes),
    ("BwmsHashToCRUID", &["Uint64"], tramp_identidade_8_bytes),
    ("BwmsCRUIDToHash", &["CRUID"], tramp_identidade_8_bytes),
    ("BwmsHashToNodeRef", &["Uint64"], tramp_identidade_8_bytes),
    ("BwmsNodeRefToHash", &["NodeRef"], tramp_identidade_8_bytes),
    ("BwmsCreateNodeRef", &["String"], tramp_string_to_noderef),
    ("BwmsEntityIDToHash", &["EntityID"], tramp_identidade_8_bytes),
    ("BwmsHashToEntityID", &["Uint64"], tramp_identidade_8_bytes),
    ("BwmsHashToTweakDBID", &["Uint64"], tramp_identidade_8_bytes),
    ("BwmsHashToResRef", &["Uint64"], tramp_identidade_8_bytes),
    ("BwmsResRefToHash", &["ResRef"], tramp_identidade_8_bytes),
    ("Print", &["String"], tramp_print),
    ("ModLog", &["CName", "String"], tramp_modlog),
    ("BwmsTicksToEngineTime", &["Uint64"], tramp_identidade_8_bytes),
    ("BwmsEngineTimeToTicks", &["EngineTime"], tramp_identidade_8_bytes),
    ("BwmsGetStackDepth", &[], tramp_stack_get_depth),
    ("BwmsGetStackFrameFunction", &["Int32"], tramp_stack_get_frame_function),
    ("BwmsGetStackFrameClass", &["Int32"], tramp_stack_get_frame_class),
    ("BwmsPersistentIDHash", &["Uint64", "Uint64"], tramp_persistentid_hash),
];


/// Parseia um arg de console p/ Reflection-Call: `i:5` `f:1.5` `b:true` `n:Name` `s:txt` `e:3`.
/// Sem prefixo conhecido → tenta i32. Marshaling real fica no rtti::Arg/call_func (provado).
/// Int u32 flexível: aceita decimal com sinal (`-1`→0xFFFFFFFF), decimal sem sinal (`4000000000`)
/// e hexa (`0xFF`). O marshalling antigo só lia decimal com sinal → `i:0xFF` virava 0 mudo.
pub(crate) fn parse_u32_flex(v: &str) -> Option<u32> {
    let v = v.trim();
    if let Some(h) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).ok()
    } else {
        v.parse::<i32>().map(|x| x as u32).ok().or_else(|| v.parse::<u32>().ok())
    }
}

/// Int u64 flexível (pra `e:` enum): decimal ou hexa. Idem parse_u32_flex.
pub(crate) fn parse_u64_flex(v: &str) -> Option<u64> {
    let v = v.trim();
    if let Some(h) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else {
        v.parse::<u64>().ok().or_else(|| v.parse::<i64>().map(|x| x as u64).ok())
    }
}

/// Bool tolerante e case-insensitive: `b:TRUE`/`b:Yes`/`b:1` = true (antes só `true`/`1` exatos →
/// `b:TRUE` virava false silencioso numa chamada VIVA de método do jogo).
pub(crate) fn parse_bool_flex(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "t" | "y" | "on" => Some(true),
        "false" | "0" | "no" | "f" | "n" | "off" => Some(false),
        _ => None,
    }
}


/// `CName` pelo texto (FNV-1a 64), `None` = 0.
pub(crate) fn cname_ou_zero(s: &str) -> u64 {
    if s.is_empty() || s == "None" {
        0
    } else {
        bwms_primitives::fnv1a64(s.as_bytes())
    }
}

/// Texto de um CName sem chamar o motor: o espelho do BWMS, depois o pool lido direto — balde `[pool+0x18+(hash&0x7ffff)·8]`, nó `{hash, próximo, índice<<8
/// em +0x10}`, entrada `[[pool+0x08]+índice·8]` com o texto terminado em zero em +0x14.
pub(crate) unsafe fn texto_do_cname(hash: u64) -> Option<String> {
    if hash == 0 {
        return None;
    }
    if let Some(s) = crate::cname::espelhado(hash) {
        return Some(s);
    }
    if (crate::rebase(CNAMEPOOL_PRONTO_VM) as *const u8).read() & 1 == 0 {
        return None;
    }
    let pool = crate::rebase(CNAMEPOOL_VM) as usize;
    let mut no = ((pool + 0x18 + (hash & 0x7ffff) as usize * 8) as *const u64).read();
    for _ in 0..64 {
        if !crate::rtti::sane(no as *mut c_void) {
            return None;
        }
        if (no as *const u64).read() == hash {
            let indice = ((no as usize + 0x10) as *const u32).read() >> 8;
            let entradas = ((pool + 0x08) as *const u64).read();
            if indice >= ((pool + 0x14) as *const u32).read() || !crate::rtti::sane(entradas as *mut c_void) {
                return None;
            }
            let e = ((entradas as usize + indice as usize * 8) as *const u64).read();
            if !crate::rtti::sane(e as *mut c_void) {
                return None;
            }
            let texto = (e as usize + 0x14) as *const u8;
            let tam = (0..256).take_while(|i| texto.add(*i).read() != 0).count();
            return Some(String::from_utf8_lossy(std::slice::from_raw_parts(texto, tam)).into_owned());
        }
        no = ((no as usize + 8) as *const u64).read();
    }
    None
}

/// `Str::SnakeCase`: maiúscula depois de minúscula/dígito (não `_`) abre `_`; maiúsculas seguidas não separam. Só ASCII é classificado (o `isupper` da libc sobre byte não-ASCII depende de locale): o resto passa intacto e abre divisão.
pub(crate) fn para_snake_case(entrada: &str) -> String {
    let mut saida = String::with_capacity(entrada.len() + 4);
    let mut pode = false;
    for c in entrada.chars() {
        if c.is_ascii_uppercase() {
            if pode {
                saida.push('_');
                pode = false;
            }
            saida.push(c.to_ascii_lowercase());
        } else {
            saida.push(c);
            pode = c != '_';
        }
    }
    saida
}

