//! Curvas de dados do jogo: tipo, modo, pontos e valores.

#![allow(dead_code)]

use crate::jogo::recursos::hash_do_caminho_do_recurso;
use crate::jogo::listas::dynarray_no_campo;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicPtr;
use crate::jogo::buffers::realocar_rawbuffer;
use crate::jogo::buffers::vtable_do_alocador_rawbuffer;
use crate::diagnostico::{
    A17_PATCHES_NAO_PRONTOS, A51G_AMOSTRAS, A51G_ANEXADAS, A51G_COPIADAS, A51G_CURVESETS, A51G_NOVAS_PENDENTES, A51G_PATCH_NAO_PRONTO,
    A51G_REALOCADAS, A51G_SEM_CAPACIDADE, A51G_TIPO_DIFERENTE, A51G_VISTOS,
};
// Só o ramo de teste da 51g usa, e o público não o cita.
#[cfg(feature = "devtools")]
use crate::diagnostico::{A51G_CRESCIMENTOS, A51G_CRESC_PRESERVOU, A51G_TESTE};
use crate::arquivos_do_jogo::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use crate::rtti::{self};
use crate::register::*;

// Codeware `#21` — curvas `curveData:*` por propriedade de objeto (`bwms-curve.reds`). O núcleo (`c21_curva`..`c21_redimensionar`) mora neste
// arquivo; no `selftest.rs` fica só o teste (`c21_esperado`, `c21_teste_tick`).
pub(crate) unsafe fn c21_args(frame: *mut c_void) -> (rtti::ArgsLidos, Option<(*mut u8, usize, *mut c_void)>) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let curva = crate::curvas::c21_curva(obj, prop);
    (args, curva)
}

pub(crate) unsafe fn c21_i32(args: &[(u64, u64)], i: usize) -> i32 {
    args.get(i).map(|(v, _)| *v as u32 as i32).unwrap_or(-1)
}

pub(crate) unsafe fn c21_f32(args: &[(u64, u64)], i: usize) -> f32 {
    f32::from_bits(args.get(i).map(|(v, _)| *v as u32).unwrap_or(0))
}

/// `BwmsCurveKind(owner, property) -> Int32` — 0 Float, 1 Vector2, 2 Vector3, 3 Vector4, 4 HDRColor; -1 sem curva.
pub(crate) unsafe extern "C" fn tramp_curve_kind(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_, curva) = c21_args(frame);
    write_i32_ret(out, curva.map_or(-1, |(_, tipo, _)| tipo as i32));
}

/// `BwmsCurveSize(owner, property) -> Int32` — pontos da curva; -1 sem curva.
pub(crate) unsafe extern "C" fn tramp_curve_size(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_, curva) = c21_args(frame);
    write_i32_ret(out, curva.map_or(-1, |(c, _, _)| crate::curvas::c21_tamanho(c) as i32));
}

/// `BwmsCurveResize(owner, property, size) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_curve_resize(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (args, curva) = c21_args(frame);
    let n = c21_i32(&args, 2);
    write_bool_ret(out, n > 0 && curva.is_some_and(|(c, tipo, _)| crate::curvas::c21_redimensionar(c, tipo, n as u32)));
}

/// `BwmsCurveMode(owner, property, link) -> Int32` — `interpolationType` ou, com `link`, `linkType`; -1 sem curva.
pub(crate) unsafe extern "C" fn tramp_curve_mode(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (args, curva) = c21_args(frame);
    let ligacao = args.get(2).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    write_i32_ret(out, curva.map_or(-1, |(c, _, _)| crate::curvas::c21_modo(c, ligacao)));
}

/// `BwmsCurveSetMode(owner, property, link, value) -> Bool` — fora da faixa (0–4 e 0–2) é recusado.
pub(crate) unsafe extern "C" fn tramp_curve_set_mode(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (args, curva) = c21_args(frame);
    let ligacao = args.get(2).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let valor = c21_i32(&args, 3);
    write_bool_ret(out, curva.is_some_and(|(c, _, _)| crate::curvas::c21_definir_modo(c, ligacao, valor)));
}

/// `BwmsCurvePoint(owner, property, index) -> Float` — tempo do ponto; 1e30 fora da faixa.
pub(crate) unsafe extern "C" fn tramp_curve_point(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (args, curva) = c21_args(frame);
    let i = c21_i32(&args, 2);
    let v = curva.filter(|_| i >= 0).and_then(|(c, _, _)| crate::curvas::c21_ponto(c, i as u32));
    write_f32_ret(out, v.unwrap_or(1e30));
}

/// `BwmsCurveValue(owner, property, index, component) -> Float` — componente do valor; 1e30 fora da faixa.
pub(crate) unsafe extern "C" fn tramp_curve_value(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (args, curva) = c21_args(frame);
    let (i, k) = (c21_i32(&args, 2), c21_i32(&args, 3));
    let v = curva.filter(|_| i >= 0 && k >= 0).and_then(|(c, tipo, _)| crate::curvas::c21_valor(c, tipo, i as u32, k as u32));
    write_f32_ret(out, v.unwrap_or(1e30));
}

/// `BwmsCurveSetPoint(owner, property, index, point) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_curve_set_point(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (args, curva) = c21_args(frame);
    let i = c21_i32(&args, 2);
    let tempo = c21_f32(&args, 3);
    write_bool_ret(out, i >= 0 && curva.is_some_and(|(c, _, _)| crate::curvas::c21_definir_ponto(c, i as u32, tempo)));
}

/// `BwmsCurveSetValue(owner, property, index, x, y, z, w) -> Bool` — só os componentes que o tipo tem.
pub(crate) unsafe extern "C" fn tramp_curve_set_value(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (args, curva) = c21_args(frame);
    let i = c21_i32(&args, 2);
    let v = [c21_f32(&args, 3), c21_f32(&args, 4), c21_f32(&args, 5), c21_f32(&args, 6)];
    write_bool_ret(out, i >= 0 && curva.is_some_and(|(c, tipo, _)| crate::curvas::c21_definir_valor(c, tipo, i as u32, v)));
}


// ===== Codeware `#21` — curvas de dados por propriedade de objeto (2026-09-13) =====
// Os tipos `CurveDataFloat`/`CurvePointFloat` do Codeware não existem neste build; as curvas existem como propriedades `curveData:Float|Vector2|Vector3|
// Vector4|HDRColor` (kind `LegacySingleChannelCurve`) em 215 classes nativas. A API do BWMS recebe o objeto e o nome da propriedade. Layout da curva, o mesmo
// da entrada de `CurveSet` da 51g a partir de `+0x08`: nome `+0x00`; `RawBuffer` `+0x08` (dados `+0x08`, tamanho `+0x10`, alinhamento `+0x14`, alocador
// `+0x18`/`+0x20`); `valueType` `+0x28`; `interpolationType` `+0x30`; `linkType` `+0x31`. Buffer: tamanho `u32` `+0`, alinhamento `+4`, offset dos pontos `+8`
// (0x10), offset dos valores `+0xC`; pontos `f32` e valores alinhados em 4. Redimensionar usa o `RawBufferAllocator` da curva (51g v2); curva vazia recebe o
// par do alocador de uma curva do jogo já vista (`C21_ALOCADOR_MODELO`).
pub(crate) const C21_TIPOS: [(&str, usize); 5] = [("Float", 4), ("Vector2", 8), ("Vector3", 12), ("Vector4", 16), ("HDRColor", 16)];

pub(crate) const C21_ALINHAMENTO: u32 = 4;

pub(crate) static C21_ALOCADOR_MODELO: std::sync::Mutex<Option<[u64; 2]>> = std::sync::Mutex::new(None);

/// Curva `prop` em `obj`: ponteiro da curva, índice do tipo em `C21_TIPOS` e o tipo RTTI da propriedade; `None` sem a propriedade ou fora dos 5 tipos.
pub(crate) unsafe fn c21_curva(obj: *mut c_void, prop: u64) -> Option<(*mut u8, usize, *mut c_void)> {
    if obj.is_null() || !crate::rtti::sane(obj) {
        return None;
    }
    let (tipo, off) = crate::rtti::propriedades(crate::rtti::class_of(obj)).into_iter().find(|(n, _, _)| *n == prop).map(|(_, t, o)| (t, o))?;
    if tipo.is_null() {
        return None;
    }
    let nome_tipo = crate::cname::name_of(crate::rtti::type_name_getname(tipo))?;
    let indice = C21_TIPOS.iter().position(|(n, _)| Some(*n) == nome_tipo.strip_prefix("curveData:"))?;
    let curva = (obj as *mut u8).add(off as usize);
    crate::memoria::is_readable(curva as *const c_void, 0x38).then_some((curva, indice, tipo))
}

pub(crate) unsafe fn c21_dados(curva: *const u8) -> *mut u8 {
    (curva.add(0x08) as *const u64).read_unaligned() as *mut u8
}

pub(crate) unsafe fn c21_tamanho(curva: *const u8) -> u32 {
    let d = c21_dados(curva);
    if d.is_null() {
        0
    } else {
        (d as *const u32).read_unaligned()
    }
}

/// Tempo do ponto `i`, ou `None` fora da faixa.
pub(crate) unsafe fn c21_ponto(curva: *const u8, i: u32) -> Option<f32> {
    let d = c21_dados(curva);
    (i < c21_tamanho(curva)).then(|| (d.add((d.add(8) as *const u32).read_unaligned() as usize + i as usize * 4) as *const f32).read_unaligned())
}

/// Componente `comp` do valor `i` (Float tem 1, Vector2 2, Vector3 3, Vector4 e HDRColor 4), ou `None`.
pub(crate) unsafe fn c21_valor(curva: *const u8, tipo: usize, i: u32, comp: u32) -> Option<f32> {
    let d = c21_dados(curva);
    let tam = C21_TIPOS[tipo].1;
    if i >= c21_tamanho(curva) || comp as usize * 4 >= tam {
        return None;
    }
    let valores = (d.add(0x0c) as *const u32).read_unaligned() as usize;
    Some((d.add(valores + i as usize * tam + comp as usize * 4) as *const f32).read_unaligned())
}

pub(crate) unsafe fn c21_definir_ponto(curva: *mut u8, i: u32, tempo: f32) -> bool {
    if i >= c21_tamanho(curva) {
        return false;
    }
    let d = c21_dados(curva);
    (d.add((d.add(8) as *const u32).read_unaligned() as usize + i as usize * 4) as *mut f32).write_unaligned(tempo);
    true
}

/// Grava os componentes do valor `i` que o tipo tem; os demais de `v` ficam de fora.
pub(crate) unsafe fn c21_definir_valor(curva: *mut u8, tipo: usize, i: u32, v: [f32; 4]) -> bool {
    if i >= c21_tamanho(curva) {
        return false;
    }
    let d = c21_dados(curva);
    let tam = C21_TIPOS[tipo].1;
    let base = (d.add(0x0c) as *const u32).read_unaligned() as usize + i as usize * tam;
    for (k, x) in v.iter().take(tam / 4).enumerate() {
        (d.add(base + k * 4) as *mut f32).write_unaligned(*x);
    }
    true
}

/// `interpolationType` (0–4) e `linkType` (0–2), `u8` em `+0x30`/`+0x31`.
pub(crate) unsafe fn c21_modo(curva: *const u8, ligacao: bool) -> i32 {
    curva.add(if ligacao { 0x31 } else { 0x30 }).read() as i32
}

pub(crate) unsafe fn c21_definir_modo(curva: *mut u8, ligacao: bool, valor: i32) -> bool {
    let maximo = if ligacao { 2 } else { 4 };
    if !(0..=maximo).contains(&valor) {
        return false;
    }
    curva.add(if ligacao { 0x31 } else { 0x30 }).write(valor as u8);
    true
}

/// `Resize` da referência: valores reposicionados pro novo offset, pontos e valores novos zerados, buffer realocado pelo alocador da curva. Curva vazia aloca
/// pelo modelo de alocador; sem modelo, `false`.
pub(crate) unsafe fn c21_redimensionar(curva: *mut u8, tipo: usize, n: u32) -> bool {
    if n < 1 {
        return false;
    }
    let tam = C21_TIPOS[tipo].1;
    let novos_valores = (0x10 + n * 4 + C21_ALINHAMENTO - 1) & !(C21_ALINHAMENTO - 1);
    let total = novos_valores + n * tam as u32;
    let alocador = curva.add(0x18);
    let dados = c21_dados(curva);
    if dados.is_null() {
        if vtable_do_alocador_rawbuffer(alocador).is_none() {
            let Some(modelo) = C21_ALOCADOR_MODELO.lock().ok().and_then(|g| *g) else { return false };
            (alocador as *mut u64).write_unaligned(modelo[0]);
            (alocador.add(8) as *mut u64).write_unaligned(modelo[1]);
        }
        let novo = realocar_rawbuffer(alocador, 0, total, C21_ALINHAMENTO) as *mut u8;
        if novo.is_null() {
            return false;
        }
        std::ptr::write_bytes(novo, 0, total as usize);
        for (k, v) in [n, C21_ALINHAMENTO, 0x10, novos_valores].into_iter().enumerate() {
            (novo.add(k * 4) as *mut u32).write_unaligned(v);
        }
        (curva.add(0x08) as *mut u64).write_unaligned(novo as u64);
        (curva.add(0x10) as *mut u32).write_unaligned(total);
        (curva.add(0x14) as *mut u32).write_unaligned(C21_ALINHAMENTO);
        return true;
    }
    let antigo = (dados as *const u32).read_unaligned();
    if n == antigo {
        return true;
    }
    if (dados.add(8) as *const u32).read_unaligned() != 0x10 || vtable_do_alocador_rawbuffer(alocador).is_none() {
        return false;
    }
    let antigos_valores = (dados.add(0x0c) as *const u32).read_unaligned() as usize;
    if n < antigo {
        std::ptr::copy(dados.add(antigos_valores), dados.add(novos_valores as usize), n as usize * tam);
    }
    let alinhamento = (curva.add(0x14) as *const u32).read_unaligned();
    let novo = realocar_rawbuffer(alocador, dados as u64, total, alinhamento) as *mut u8;
    if novo.is_null() {
        return false;
    }
    if n > antigo {
        std::ptr::copy(novo.add(antigos_valores), novo.add(novos_valores as usize), antigo as usize * tam);
        std::ptr::write_bytes(novo.add(0x10 + antigo as usize * 4), 0, (n - antigo) as usize * 4);
        std::ptr::write_bytes(novo.add(novos_valores as usize + antigo as usize * tam), 0, (n - antigo) as usize * tam);
    }
    (novo as *mut u32).write_unaligned(n);
    (novo.add(0x0c) as *mut u32).write_unaligned(novos_valores);
    (curva.add(0x08) as *mut u64).write_unaligned(novo as u64);
    (curva.add(0x10) as *mut u32).write_unaligned(total);
    true
}


pub(crate) static A51G_ATIVO: AtomicBool = AtomicBool::new(false);

pub(crate) static A51G_CLASSE_CURVESET: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) static A51G_CURVAS_OFF: AtomicU64 = AtomicU64::new(0x40);

/// 51g v2: vtable do `RawBufferAllocator` (lida ao vivo no boot 114): slot 0 `(this, dados, tamanho, alinhamento)` realoca pelo `ReallocateAligned` do
/// alocador interno e devolve os dados novos; slot 1 `(this, dados)` libera.
pub(crate) const A51G_RAWBUFFER_ALOCADOR_VT: u64 = 0x1_0781_6278;

/// Libera `dados` pelo slot 1 do mesmo alocador.
#[cfg(any(test, feature = "devtools"))]
pub(crate) unsafe fn a51g_liberar(obj: *mut u8, dados: u64) {
    if let Some(vt) = vtable_do_alocador_rawbuffer(obj) {
        let f: unsafe extern "C" fn(*mut u8, u64) = std::mem::transmute(((vt + 8) as *const u64).read_unaligned());
        f(obj, dados);
    }
}

/// Passo da mescla de uma curva do patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum A51gPasso {
    /// Mesmo nome e mesmo `valueType` na entrada `i` do alvo.
    Copia(usize),
    /// Mesmo nome com `valueType` diferente: a referência não mexe.
    TipoDiferente(usize),
    /// Nome que o alvo não tem: a referência anexa.
    Nova,
}

/// Núcleo puro do `OnCurveSetResourceLoad`: `alvo[i]` e `patch[j]` = `(nome, valueType)` das curvas.
pub(crate) fn a51g_plano(alvo: &[(u64, u64)], patch: &[(u64, u64)]) -> Vec<A51gPasso> {
    patch
        .iter()
        .map(|(nome, tipo)| match alvo.iter().position(|(n, _)| n == nome) {
            Some(i) if alvo[i].1 == *tipo => A51gPasso::Copia(i),
            Some(i) => A51gPasso::TipoDiferente(i),
            None => A51gPasso::Nova,
        })
        .collect()
}

/// `(nome, valueType)` das curvas de um `CurveSet`; `None` com o array ilegível.
pub(crate) unsafe fn a51g_entradas(res: u64) -> Option<Vec<(u64, u64)>> {
    if res == 0 || !crate::memoria::is_readable(res as *const c_void, 0x50) {
        return None;
    }
    let (b, n, _) = dynarray_no_campo(res as *const u8, A51G_CURVAS_OFF.load(Ordering::Relaxed) as usize);
    if n > 4096 || (n > 0 && !crate::memoria::is_readable(b as *const c_void, n * 0x40)) {
        return None;
    }
    Some((0..n).map(|i| ((b.add(i * 0x40) as *const u64).read_unaligned(), (b.add(i * 0x40 + 0x30) as *const u64).read_unaligned())).collect())
}

/// 51g, no `OnDependenciesReady` (thread de job): curvas de cada patch da tabela cujo alvo é este `CurveSet`.
pub(crate) unsafe fn a51g_mesclar_curvas(alvo: u64) {
    A51G_CURVESETS.fetch_add(1, Ordering::Relaxed);
    let caminho = hash_do_caminho_do_recurso(alvo);
    if let Ok(mut g) = A51G_VISTOS.try_lock() {
        if g.len() < 1024 {
            g.push(caminho);
        }
    }
    let patches: Vec<(u64, [u64; 2])> = match PATCHES_DE_RECURSO.try_read() {
        Ok(g) => g.do_alvo(caminho).map(|p| (p.patch, p.token)).collect(),
        Err(_) => return,
    };
    let off = A51G_CURVAS_OFF.load(Ordering::Relaxed) as usize;
    for (patch, pt) in patches {
        // A referência junta o job do token do patch ao do alvo (`OnResourceDeserialize`); aqui a espera é a da 51a, até 500 ms pelo status do token.
        let pres = if pt[0] == 0 { 0 } else { recurso_do_patch_da_classe(pt, 0x50, A51G_CLASSE_CURVESET.load(Ordering::Relaxed)) as u64 };
        if pres == 0 {
            A17_PATCHES_NAO_PRONTOS.fetch_add(1, Ordering::Relaxed);
            if crate::primeiras(&A51G_PATCH_NAO_PRONTO, 8) {
                crate::veredito_dev!("[a51g] patch {patch:#018x} do curveset {caminho:#018x} ({alvo:#x}) não ficou pronto na espera (token {:#x})", pt[0]);
            }
            continue;
        }
        let (Some(ea), Some(ep)) = (a51g_entradas(alvo), a51g_entradas(pres)) else { continue };
        let plano = a51g_plano(&ea, &ep);
        let (tb, _, _) = dynarray_no_campo(alvo as *const u8, off);
        let (pb, _, _) = dynarray_no_campo(pres as *const u8, off);
        let (mut copiadas, mut sem_cap, mut tipo, mut novas) = (0usize, 0usize, 0usize, 0usize);
        let mut a_anexar: Vec<usize> = Vec::new();
        for (j, passo) in plano.iter().enumerate() {
            let pe = pb.add(j * 0x40);
            match *passo {
                A51gPasso::Copia(i) => {
                    let te = tb.add(i * 0x40);
                    let (pd, ps) = ((pe.add(0x10) as *const u64).read_unaligned(), (pe.add(0x18) as *const u32).read_unaligned() as usize);
                    let (mut td, mut ts) = ((te.add(0x10) as *const u64).read_unaligned(), (te.add(0x18) as *const u32).read_unaligned() as usize);
                    // 51g v2: buffer do patch maior que o do alvo → realoca pelo alocador do próprio buffer (o `Resize` da referência).
                    if pd != 0 && td != 0 && ps > ts {
                        let novo = realocar_rawbuffer(te.add(0x20), td, ps as u32, (te.add(0x1c) as *const u32).read_unaligned());
                        if novo != 0 {
                            (te.add(0x10) as *mut u64).write_unaligned(novo);
                            (te.add(0x18) as *mut u32).write_unaligned(ps as u32);
                            (td, ts) = (novo, ps);
                            A51G_REALOCADAS.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    if pd == 0 || td == 0 || ps == 0 || ps > ts || !crate::memoria::is_readable(pd as *const c_void, ps) || !crate::memoria::is_readable(td as *const c_void, ts) {
                        sem_cap += 1;
                        continue;
                    }
                    let iguais_antes = std::slice::from_raw_parts(td as *const u8, ps) == std::slice::from_raw_parts(pd as *const u8, ps);
                    std::ptr::copy(pd as *const u8, td as *mut u8, ps);
                    (te.add(0x18) as *mut u32).write_unaligned(ps as u32);
                    te.add(0x38).write(pe.add(0x38).read());
                    te.add(0x39).write(pe.add(0x39).read());
                    copiadas += 1;
                    if let Ok(mut g) = A51G_AMOSTRAS.try_lock() {
                        if g.len() < 32 {
                            g.push([alvo, patch, ep[j].0, iguais_antes as u64]);
                        }
                    }
                }
                A51gPasso::TipoDiferente(_) => tipo += 1,
                A51gPasso::Nova => a_anexar.push(j),
            }
        }
        // 51g v2: curvas novas no fim do `curves` do alvo, com buffer próprio alocado pelo alocador copiado da entrada do patch (o `EmplaceBack` + `Resize`).
        let mut anexadas = 0usize;
        for j in a_anexar {
            let pe = pb.add(j * 0x40);
            let (pd, ps) = ((pe.add(0x10) as *const u64).read_unaligned(), (pe.add(0x18) as *const u32).read_unaligned() as usize);
            let arr = (alvo as *mut u8).add(off);
            let (_, n, cap) = dynarray_no_campo(arr, 0);
            if pd == 0 || ps == 0 || !crate::memoria::is_readable(pd as *const c_void, ps) || (pe.add(0x20) as *const u64).read_unaligned() != crate::rebase(A51G_RAWBUFFER_ALOCADOR_VT) as u64 {
                novas += 1;
                continue;
            }
            if n + 1 > cap {
                crate::jogo::listas::reservar(arr as *mut c_void, (n + 1) as u32, 0x40, 8, std::ptr::null());
                if dynarray_no_campo(arr, 0).2 < n + 1 {
                    novas += 1;
                    continue;
                }
            }
            let dst = dynarray_no_campo(arr, 0).0.add(n * 0x40);
            std::ptr::write_bytes(dst, 0, 0x40);
            std::ptr::copy_nonoverlapping(pe, dst, 0x10);
            (dst.add(0x1c) as *mut u32).write_unaligned((pe.add(0x1c) as *const u32).read_unaligned());
            std::ptr::copy_nonoverlapping(pe.add(0x20), dst.add(0x20), 0x10);
            std::ptr::copy_nonoverlapping(pe.add(0x30), dst.add(0x30), 0x0a);
            let alin = (pe.add(0x1c) as *const u32).read_unaligned();
            // O teste do crescimento (`~/.bwms-hook-a51g-teste`) só existe no dev: no público a curva nova pede o tamanho todo de uma vez.
            #[cfg(not(feature = "devtools"))]
            let novo = realocar_rawbuffer(dst.add(0x20), 0, ps as u32, alin);
            #[cfg(feature = "devtools")]
            let novo = if A51G_TESTE.load(Ordering::Relaxed) && ps >= 2 {
                // Teste: aloca a metade, copia essa metade e cresce pelo mesmo slot — exercita a realocação de um buffer com dados, a da curva comum maior.
                let metade = ps / 2;
                let pequeno = realocar_rawbuffer(dst.add(0x20), 0, metade as u32, alin);
                if pequeno == 0 {
                    0
                } else {
                    std::ptr::copy_nonoverlapping(pd as *const u8, pequeno as *mut u8, metade);
                    let cresceu = realocar_rawbuffer(dst.add(0x20), pequeno, ps as u32, alin);
                    if cresceu == 0 {
                        a51g_liberar(dst.add(0x20), pequeno);
                    } else {
                        A51G_CRESCIMENTOS.fetch_add(1, Ordering::Relaxed);
                        if std::slice::from_raw_parts(cresceu as *const u8, metade) == std::slice::from_raw_parts(pd as *const u8, metade) {
                            A51G_CRESC_PRESERVOU.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    cresceu
                }
            } else {
                realocar_rawbuffer(dst.add(0x20), 0, ps as u32, alin)
            };
            if novo == 0 {
                std::ptr::write_bytes(dst, 0, 0x40);
                novas += 1;
                continue;
            }
            std::ptr::copy_nonoverlapping(pd as *const u8, novo as *mut u8, ps);
            (dst.add(0x10) as *mut u64).write_unaligned(novo);
            (dst.add(0x18) as *mut u32).write_unaligned(ps as u32);
            (arr.add(0x0c) as *mut u32).write_unaligned((n + 1) as u32);
            anexadas += 1;
            if let Ok(mut g) = A51G_AMOSTRAS.try_lock() {
                if g.len() < 32 {
                    g.push([alvo, patch, ep[j].0, 2]);
                }
            }
        }
        A51G_ANEXADAS.fetch_add(anexadas as u64, Ordering::Relaxed);
        A51G_COPIADAS.fetch_add(copiadas as u64, Ordering::Relaxed);
        A51G_SEM_CAPACIDADE.fetch_add(sem_cap as u64, Ordering::Relaxed);
        A51G_TIPO_DIFERENTE.fetch_add(tipo as u64, Ordering::Relaxed);
        A51G_NOVAS_PENDENTES.fetch_add(novas as u64, Ordering::Relaxed);
        crate::veredito_dev!(
            "[a51g] patch {patch:#018x} no curveset {caminho:#018x} ({alvo:#x}): {} curva(s) no patch, {} no alvo; {copiadas} copiada(s), {sem_cap} sem capacidade, {tipo} com tipo diferente, {anexadas} anexada(s), {novas} nova(s) não anexada(s)",
            ep.len(),
            ea.len()
        );
    }
}

