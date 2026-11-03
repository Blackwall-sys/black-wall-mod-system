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

// Curvas `curveData:*` por propriedade de objeto (`bwms-curve.reds`). O núcleo (`c21_curva`..`c21_redimensionar`) mora neste
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


// ===== Curvas de dados por propriedade de objeto (2026-09-13) =====
// Os tipos `CurveDataFloat`/`CurvePointFloat` não existem neste build; as curvas existem como propriedades `curveData:Float|Vector2|Vector3|
// Vector4|HDRColor` (kind `LegacySingleChannelCurve`) em 215 classes nativas. A API do BWMS recebe o objeto e o nome da propriedade. Layout da curva, o mesmo
// da entrada de `CurveSet` a partir de `+0x08`: nome `+0x00`; `RawBuffer` `+0x08` (dados `+0x08`, tamanho `+0x10`, alinhamento `+0x14`, alocador
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

/// Muda a curva para `n` pontos.
pub(crate) unsafe fn c21_redimensionar(curva: *mut u8, tipo: usize, n: u32) -> bool {
    c21_redimensionar_com(curva, tipo, n, |obj| vtable_do_alocador_rawbuffer(obj).is_some(), |obj, dados, tam, alin| realocar_rawbuffer(obj, dados, tam, alin))
}

/// Offset dos valores e tamanho total do buffer de `n` pontos com valores de `tam_valor` B; `None` se não cabe em `u32`.
fn c21_layout(n: u32, tam_valor: usize) -> Option<(u32, u32)> {
    let valores = 0x10u32.checked_add(n.checked_mul(4)?)?.checked_add(3)? & !3;
    let total = valores.checked_add(n.checked_mul(u32::try_from(tam_valor).ok()?)?)?;
    Some((valores, total))
}

/// Núcleo de [`c21_redimensionar`] com o alocador por parâmetro: `conferido(obj)` diz se o par em `obj` é o alocador do jogo e `realocar` é o slot 0
/// dele. O teste passa um alocador próprio porque o `rebase` do harness não acha a vtable do jogo.
unsafe fn c21_redimensionar_com(
    curva: *mut u8,
    tipo: usize,
    n: u32,
    conferido: impl Fn(*const u8) -> bool,
    realocar: impl Fn(*mut u8, u64, u32, u32) -> u64,
) -> bool {
    let Some(&(_, tam_valor)) = C21_TIPOS.get(tipo) else { return false };
    if n == 0 || curva.is_null() {
        return false;
    }
    let Some((off_valores, total)) = c21_layout(n, tam_valor) else { return false };
    let alocador = curva.add(0x18);
    let dados = c21_dados(curva);
    if dados.is_null() {
        if !conferido(alocador) {
            let Some(modelo) = C21_ALOCADOR_MODELO.lock().ok().and_then(|m| *m) else { return false };
            (alocador as *mut [u64; 2]).write_unaligned(modelo);
        }
        let novo = realocar(alocador, 0, total, C21_ALINHAMENTO) as *mut u8;
        if novo.is_null() {
            return false;
        }
        std::ptr::write_bytes(novo, 0, total as usize);
        (novo as *mut [u32; 4]).write_unaligned([n, C21_ALINHAMENTO, 0x10, off_valores]);
        (curva.add(0x08) as *mut u64).write_unaligned(novo as u64);
        (curva.add(0x10) as *mut u32).write_unaligned(total);
        (curva.add(0x14) as *mut u32).write_unaligned(C21_ALINHAMENTO);
        return true;
    }
    let cabecalho = (dados as *const [u32; 4]).read_unaligned();
    let antigo = cabecalho[0];
    if antigo == n {
        return true;
    }
    if cabecalho[2] != 0x10 || !conferido(alocador) {
        return false;
    }
    // A realocação preserva só o começo do buffer: os valores antigos saem antes, porque ao encolher o offset deles passa do tamanho novo.
    let mantidos = antigo.min(n) as usize;
    let valores_antigos = std::slice::from_raw_parts(dados.add(cabecalho[3] as usize), mantidos * tam_valor).to_vec();
    let alinhamento = (curva.add(0x14) as *const u32).read_unaligned();
    let novo = realocar(alocador, dados as u64, total, alinhamento) as *mut u8;
    if novo.is_null() {
        return false;
    }
    let inicio_zerado = 0x10 + mantidos * 4;
    std::ptr::write_bytes(novo.add(inicio_zerado), 0, total as usize - inicio_zerado);
    std::ptr::copy_nonoverlapping(valores_antigos.as_ptr(), novo.add(off_valores as usize), valores_antigos.len());
    (novo as *mut [u32; 4]).write_unaligned([n, cabecalho[1], 0x10, off_valores]);
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
    /// Mesmo nome com `valueType` diferente na entrada `i` do alvo.
    TipoDiferente(usize),
    /// Nome que o alvo não tem.
    Nova,
}

/// `alvo[i]` e `patch[j]` = `(nome, valueType)` das curvas.
pub(crate) fn a51g_plano(alvo: &[(u64, u64)], patch: &[(u64, u64)]) -> Vec<A51gPasso> {
    patch
        .iter()
        .map(|&(nome, tipo)| match alvo.iter().position(|&(n, _)| n == nome) {
            Some(i) if alvo[i].1 == tipo => A51gPasso::Copia(i),
            Some(i) => A51gPasso::TipoDiferente(i),
            None => A51gPasso::Nova,
        })
        .collect()
}

/// Entrada de `curves`: 0x40 B; o `RawBuffer` da curva começa em `+0x10` (dados), tamanho `+0x18`, alinhamento `+0x1c`, alocador `+0x20`/`+0x28`.
const A51G_ENTRADA: usize = 0x40;

/// Teto de entradas lidas de um `CurveSet`: acima disso o array é tratado como lixo.
const A51G_MAX_ENTRADAS: usize = 4096;

/// `(nome, valueType)` das curvas de um `CurveSet`; `None` com o array ilegível.
pub(crate) unsafe fn a51g_entradas(res: u64) -> Option<Vec<(u64, u64)>> {
    if res == 0 || !crate::memoria::is_readable(res as *const c_void, 0x50) {
        return None;
    }
    let (b, n, _) = dynarray_no_campo(res as *const u8, A51G_CURVAS_OFF.load(Ordering::Relaxed) as usize);
    if n > A51G_MAX_ENTRADAS {
        return None;
    }
    if n == 0 {
        return Some(Vec::new());
    }
    if b.is_null() || !crate::memoria::is_readable(b as *const c_void, n * A51G_ENTRADA) {
        return None;
    }
    Some((0..n).map(|i| ((b.add(i * A51G_ENTRADA) as *const u64).read_unaligned(), (b.add(i * A51G_ENTRADA + 0x30) as *const u64).read_unaligned())).collect())
}

/// `(dados, tamanho)` do `RawBuffer` de uma entrada de `curves`.
unsafe fn a51g_buffer(entrada: *const u8) -> (u64, u32) {
    ((entrada.add(0x10) as *const u64).read_unaligned(), (entrada.add(0x18) as *const u32).read_unaligned())
}

/// Contagem de uma linha da tabela, somada nos globais no fim dela.
#[derive(Default)]
struct A51gContagem {
    copiadas: u64,
    sem_capacidade: u64,
    tipo_diferente: u64,
    anexadas: u64,
    nao_anexadas: u64,
}

fn a51g_amostra(alvo: u64, patch: u64, nome: u64, marca: u64) {
    if let Ok(mut a) = A51G_AMOSTRAS.lock() {
        if a.len() < 32 {
            a.push([alvo, patch, nome, marca]);
        }
    }
}

/// Copia a curva `pe` do patch por cima da entrada `te` do alvo (mesmo nome e tipo). `Some(bytes já iguais)` copiada; `None` sem capacidade.
unsafe fn a51g_copiar(te: *mut u8, pe: *const u8) -> Option<bool> {
    let (pd, ps) = a51g_buffer(pe);
    let (td, ts) = a51g_buffer(te);
    if pd != 0 && td != 0 && ps > ts {
        let novo = realocar_rawbuffer(te.add(0x20), td, ps, (te.add(0x1c) as *const u32).read_unaligned());
        if novo != 0 {
            (te.add(0x10) as *mut u64).write_unaligned(novo);
            (te.add(0x18) as *mut u32).write_unaligned(ps);
            A51G_REALOCADAS.fetch_add(1, Ordering::Relaxed);
        }
    }
    let (td, ts) = a51g_buffer(te);
    if td == 0 || ts == 0 || pd == 0 || ps > ts || !crate::memoria::is_readable(pd as *const c_void, ps as usize) || !crate::memoria::is_readable(td as *const c_void, ps as usize) {
        return None;
    }
    let iguais = std::slice::from_raw_parts(td as *const u8, ps as usize) == std::slice::from_raw_parts(pd as *const u8, ps as usize);
    std::ptr::copy_nonoverlapping(pd as *const u8, td as *mut u8, ps as usize);
    (te.add(0x18) as *mut u32).write_unaligned(ps);
    te.add(0x38).write(pe.add(0x38).read());
    te.add(0x39).write(pe.add(0x39).read());
    Some(iguais)
}

/// Anexa a curva `pe` do patch no fim de `curves` do alvo, com buffer próprio do mesmo alocador. `false` não anexada.
unsafe fn a51g_anexar(res: u64, pe: *const u8) -> bool {
    let (pd, ps) = a51g_buffer(pe);
    if pd == 0 || ps == 0 || !crate::memoria::is_readable(pd as *const c_void, ps as usize) || vtable_do_alocador_rawbuffer(pe.add(0x20)).is_none() {
        return false;
    }
    let off = A51G_CURVAS_OFF.load(Ordering::Relaxed) as usize;
    let lista = (res as *mut u8).add(off);
    let (_, n, cap) = dynarray_no_campo(res as *const u8, off);
    if n >= cap {
        crate::jogo::listas::reservar(lista as *mut c_void, (n + 1) as u32, A51G_ENTRADA as u32, 8, std::ptr::null());
    }
    let (b, n, cap) = dynarray_no_campo(res as *const u8, off);
    if b.is_null() || n >= cap {
        return false;
    }
    let nova = b.add(n * A51G_ENTRADA);
    std::ptr::write_bytes(nova, 0, A51G_ENTRADA);
    std::ptr::copy_nonoverlapping(pe, nova, 0x10);
    std::ptr::copy_nonoverlapping(pe.add(0x1c), nova.add(0x1c), 4);
    std::ptr::copy_nonoverlapping(pe.add(0x20), nova.add(0x20), 0x10);
    std::ptr::copy_nonoverlapping(pe.add(0x30), nova.add(0x30), 0x0a);
    let alinhamento = (pe.add(0x1c) as *const u32).read_unaligned();
    let dados = a51g_alocar_copia(nova.add(0x20), pd, ps, alinhamento);
    if dados == 0 {
        std::ptr::write_bytes(nova, 0, A51G_ENTRADA);
        return false;
    }
    (nova.add(0x10) as *mut u64).write_unaligned(dados);
    (nova.add(0x18) as *mut u32).write_unaligned(ps);
    (lista.add(0x0c) as *mut u32).write_unaligned(n as u32 + 1);
    true
}

/// Buffer novo de `ps` B pelo alocador em `alocador`, com os bytes de `pd`. 0 na falha.
unsafe fn a51g_alocar_copia(alocador: *mut u8, pd: u64, ps: u32, alinhamento: u32) -> u64 {
    // Teste do crescimento (`~/.bwms-hook-a51g-teste`): aloca a metade e cresce pelo mesmo alocador, conferindo que a metade copiada ficou.
    #[cfg(feature = "devtools")]
    if A51G_TESTE.load(Ordering::Relaxed) && ps >= 2 {
        let metade = ps / 2;
        let d = realocar_rawbuffer(alocador, 0, metade, alinhamento);
        if d == 0 {
            return 0;
        }
        std::ptr::copy_nonoverlapping(pd as *const u8, d as *mut u8, metade as usize);
        let cresceu = realocar_rawbuffer(alocador, d, ps, alinhamento);
        if cresceu == 0 {
            return 0;
        }
        A51G_CRESCIMENTOS.fetch_add(1, Ordering::Relaxed);
        if std::slice::from_raw_parts(cresceu as *const u8, metade as usize) == std::slice::from_raw_parts(pd as *const u8, metade as usize) {
            A51G_CRESC_PRESERVOU.fetch_add(1, Ordering::Relaxed);
        }
        std::ptr::copy_nonoverlapping(pd as *const u8, cresceu as *mut u8, ps as usize);
        return cresceu;
    }
    let d = realocar_rawbuffer(alocador, 0, ps, alinhamento);
    if d != 0 {
        std::ptr::copy_nonoverlapping(pd as *const u8, d as *mut u8, ps as usize);
    }
    d
}

/// No `OnDependenciesReady` (thread de job): curvas de cada patch da tabela cujo alvo é este `CurveSet`.
pub(crate) unsafe fn a51g_mesclar_curvas(alvo: u64) {
    A51G_CURVESETS.fetch_add(1, Ordering::Relaxed);
    let caminho = hash_do_caminho_do_recurso(alvo);
    if let Ok(mut v) = A51G_VISTOS.lock() {
        if v.len() < 1024 && !v.contains(&caminho) {
            v.push(caminho);
        }
    }
    // Leitura sem esperar: o gancho roda em thread de job e a tabela só troca na carga de mods.
    let linhas: Vec<(u64, [u64; 2])> = match PATCHES_DE_RECURSO.try_read() {
        Ok(t) => t.do_alvo(caminho).map(|p| (p.patch, p.token)).collect(),
        Err(_) => return,
    };
    let classe = A51G_CLASSE_CURVESET.load(Ordering::Relaxed);
    let off = A51G_CURVAS_OFF.load(Ordering::Relaxed) as usize;
    for (patch, token) in linhas {
        let rp = if token[0] == 0 { std::ptr::null() } else { recurso_do_patch_da_classe(token, 0x50, classe) };
        if rp.is_null() {
            A17_PATCHES_NAO_PRONTOS.fetch_add(1, Ordering::Relaxed);
            if crate::primeiras(&A51G_PATCH_NAO_PRONTO, 8) {
                crate::veredito_dev!("[a51g] patch {patch:#018x} do curveset {caminho:#018x} não pronto; fica de fora");
            }
            continue;
        }
        let (Some(ea), Some(ep)) = (a51g_entradas(alvo), a51g_entradas(rp as u64)) else { continue };
        let (pb, _, _) = dynarray_no_campo(rp, off);
        let mut c = A51gContagem::default();
        for (j, passo) in a51g_plano(&ea, &ep).into_iter().enumerate() {
            let pe = pb.add(j * A51G_ENTRADA) as *const u8;
            match passo {
                A51gPasso::Copia(i) => {
                    let (tb, _, _) = dynarray_no_campo(alvo as *const u8, off);
                    match a51g_copiar(tb.add(i * A51G_ENTRADA), pe) {
                        Some(iguais) => {
                            c.copiadas += 1;
                            a51g_amostra(alvo, patch, ep[j].0, iguais as u64);
                        }
                        None => c.sem_capacidade += 1,
                    }
                }
                A51gPasso::TipoDiferente(_) => c.tipo_diferente += 1,
                A51gPasso::Nova => {
                    if a51g_anexar(alvo, pe) {
                        c.anexadas += 1;
                        // 2 marca a curva anexada para o teste 51g2, que confere o buffer próprio.
                        a51g_amostra(alvo, patch, ep[j].0, 2);
                    } else {
                        c.nao_anexadas += 1;
                    }
                }
            }
        }
        A51G_ANEXADAS.fetch_add(c.anexadas, Ordering::Relaxed);
        A51G_COPIADAS.fetch_add(c.copiadas, Ordering::Relaxed);
        A51G_SEM_CAPACIDADE.fetch_add(c.sem_capacidade, Ordering::Relaxed);
        A51G_TIPO_DIFERENTE.fetch_add(c.tipo_diferente, Ordering::Relaxed);
        A51G_NOVAS_PENDENTES.fetch_add(c.nao_anexadas, Ordering::Relaxed);
        crate::veredito_dev!(
            "[a51g] curveset {caminho:#018x} patch {patch:#018x}: {} copiada(s), {} sem capacidade, {} de tipo diferente, {} anexada(s), {} nova(s) não anexada(s)",
            c.copiadas,
            c.sem_capacidade,
            c.tipo_diferente,
            c.anexadas,
            c.nao_anexadas
        );
    }
}

/// Vetores `B5|curvas|*` de `B5-vetores.txt`.
#[cfg(test)]
mod vetores_b5 {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Marca do alocador conferido no teste (o `rebase` do harness não acha a vtable do jogo).
    const ALOCADOR_TESTE: u64 = 0xA10C_A10C;

    /// Alocador de teste: guarda `(dados não nulos, tamanho, alinhamento)` de cada chamada e preserva o começo como o slot 0 do jogo.
    struct Alocador {
        chamadas: RefCell<Vec<(bool, u32, u32)>>,
        vivos: RefCell<HashMap<u64, Vec<u8>>>,
    }

    impl Alocador {
        fn novo() -> Self {
            Alocador { chamadas: RefCell::new(Vec::new()), vivos: RefCell::new(HashMap::new()) }
        }

        fn realocar(&self, dados: u64, tam: u32, alin: u32) -> u64 {
            self.chamadas.borrow_mut().push((dados != 0, tam, alin));
            let mut novo = vec![0xEEu8; tam as usize];
            if let Some(antigo) = self.vivos.borrow_mut().remove(&dados) {
                let n = antigo.len().min(novo.len());
                novo[..n].copy_from_slice(&antigo[..n]);
            }
            let p = novo.as_ptr() as u64;
            self.vivos.borrow_mut().insert(p, novo);
            p
        }

        /// Buffer de curva já existente, com o cabeçalho e os bytes dados.
        fn buffer(&self, bytes: Vec<u8>) -> u64 {
            let p = bytes.as_ptr() as u64;
            self.vivos.borrow_mut().insert(p, bytes);
            p
        }
    }

    fn redimensionar(curva: &mut [u8; 0x38], tipo: usize, n: u32, a: &Alocador) -> bool {
        unsafe {
            c21_redimensionar_com(
                curva.as_mut_ptr(),
                tipo,
                n,
                |obj| (obj as *const u64).read_unaligned() == ALOCADOR_TESTE,
                |_, dados, tam, alin| a.realocar(dados, tam, alin),
            )
        }
    }

    /// Curva com buffer: pontos e valores (cada valor com `C21_TIPOS[tipo].1 / 4` floats), alinhamento do campo `alin`.
    fn curva_com(a: &Alocador, tipo: usize, pontos: &[f32], valores: &[f32], alin: u32) -> [u8; 0x38] {
        let n = pontos.len() as u32;
        let (off, total) = c21_layout(n, C21_TIPOS[tipo].1).expect("layout");
        let mut b = vec![0u8; total as usize];
        b[..16].copy_from_slice(&[n, 4, 0x10, off].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
        for (i, p) in pontos.iter().enumerate() {
            b[0x10 + 4 * i..0x14 + 4 * i].copy_from_slice(&p.to_le_bytes());
        }
        for (i, v) in valores.iter().enumerate() {
            b[off as usize + 4 * i..off as usize + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        let mut c = [0u8; 0x38];
        c[0x08..0x10].copy_from_slice(&a.buffer(b).to_le_bytes());
        c[0x10..0x14].copy_from_slice(&total.to_le_bytes());
        c[0x14..0x18].copy_from_slice(&alin.to_le_bytes());
        c[0x18..0x20].copy_from_slice(&ALOCADOR_TESTE.to_le_bytes());
        c
    }

    /// `(tamanho do campo, alinhamento do campo, bytes do buffer em hex)`.
    fn estado(c: &[u8; 0x38]) -> (u32, u32, String) {
        let tam = u32::from_le_bytes(c[0x10..0x14].try_into().expect("4 B"));
        let alin = u32::from_le_bytes(c[0x14..0x18].try_into().expect("4 B"));
        let dados = u64::from_le_bytes(c[0x08..0x10].try_into().expect("8 B"));
        let hex = if dados == 0 { String::new() } else { unsafe { std::slice::from_raw_parts(dados as *const u8, tam as usize) }.iter().map(|b| format!("{b:02x}")).collect() };
        (tam, alin, hex)
    }

    #[test]
    fn plano() {
        use A51gPasso::*;
        assert_eq!(a51g_plano(&[(5, 1), (5, 2)], &[(5, 2), (5, 1)]), vec![TipoDiferente(0), Copia(0)]);
        assert_eq!(a51g_plano(&[], &[(7, 1), (7, 1)]), vec![Nova, Nova]);
        assert_eq!(a51g_plano(&[(1, 1)], &[]), vec![]);
        assert_eq!(
            a51g_plano(&[(1, 10), (2, 20), (3, 30)], &[(2, 20), (3, 99), (4, 40), (1, 10), (2, 21)]),
            vec![Copia(1), TipoDiferente(2), Nova, Copia(0), TipoDiferente(1)]
        );
    }

    /// Curva vazia: com e sem o par do alocador, com e sem o modelo (o modelo é global; um teste só mexe nele).
    #[test]
    fn redimensionar_vazia() {
        let a = Alocador::novo();
        *C21_ALOCADOR_MODELO.lock().expect("modelo") = None;
        let mut c = [0u8; 0x38];
        assert!(!redimensionar(&mut c, 0, 3, &a));
        assert_eq!((estado(&c), a.chamadas.borrow().clone()), ((0, 0, String::new()), vec![]));

        *C21_ALOCADOR_MODELO.lock().expect("modelo") = Some([ALOCADOR_TESTE, 0x77]);
        let mut c = [0u8; 0x38];
        assert!(redimensionar(&mut c, 0, 3, &a));
        *C21_ALOCADOR_MODELO.lock().expect("modelo") = None;
        assert_eq!(&c[0x18..0x28], &[ALOCADOR_TESTE.to_le_bytes(), 0x77u64.to_le_bytes()].concat()[..]);
        assert_eq!(estado(&c), (40, 4, "0300000004000000100000001c000000000000000000000000000000000000000000000000000000".into()));
        assert_eq!(a.chamadas.borrow().last(), Some(&(false, 40, 4)));

        for (tipo, n, tam, cabecalho) in [(0, 1, 24, [1u32, 4, 16, 20]), (1, 2, 40, [2, 4, 16, 24]), (2, 3, 64, [3, 4, 16, 28]), (3, 1, 36, [1, 4, 16, 20]), (4, 2, 56, [2, 4, 16, 24])] {
            let mut c = [0u8; 0x38];
            c[0x18..0x20].copy_from_slice(&ALOCADOR_TESTE.to_le_bytes());
            assert!(redimensionar(&mut c, tipo, n, &a));
            let esperado: String = cabecalho.iter().flat_map(|v| v.to_le_bytes()).map(|b| format!("{b:02x}")).collect::<String>() + &"0".repeat(tam as usize * 2 - 32);
            assert_eq!(estado(&c), (tam, 4, esperado), "{}", C21_TIPOS[tipo].0);
            assert_eq!(a.chamadas.borrow().last(), Some(&(false, tam, 4)));
        }
    }

    #[test]
    fn redimensionar_com_buffer() {
        let a = Alocador::novo();
        let antes = "020000000400000010000000180000000000803f00000040000020410000a041";
        let mut c = curva_com(&a, 0, &[1.0, 2.0], &[10.0, 20.0], 4);
        assert_eq!(estado(&c).2, antes);
        assert!(!redimensionar(&mut c, 0, 0, &a));
        assert!(redimensionar(&mut c, 0, 2, &a));
        assert_eq!((estado(&c).2.as_str(), a.chamadas.borrow().len()), (antes, 0));

        assert!(redimensionar(&mut c, 0, 5, &a));
        assert_eq!(estado(&c), (56, 4, "050000000400000010000000240000000000803f00000040000000000000000000000000000020410000a041000000000000000000000000".into()));

        let mut c = curva_com(&a, 2, &[0.5], &[1.0, 2.0, 3.0], 4);
        assert!(redimensionar(&mut c, 2, 3, &a));
        assert_eq!(estado(&c), (64, 4, "0300000004000000100000001c0000000000003f00000000000000000000803f0000004000004040000000000000000000000000000000000000000000000000".into()));

        let mut c = curva_com(&a, 1, &[0.0, 1.0, 2.0, 3.0], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], 4);
        assert!(redimensionar(&mut c, 1, 1, &a));
        assert_eq!(estado(&c), (28, 4, "01000000040000001000000014000000000000000000803f00000040".into()));

        let mut c = curva_com(&a, 0, &[0.0, 1.0, 2.0, 3.0, 4.0], &[10.0, 11.0, 12.0, 13.0, 14.0], 4);
        assert!(redimensionar(&mut c, 0, 3, &a));
        assert_eq!(estado(&c), (40, 4, "0300000004000000100000001c000000000000000000803f00000040000020410000304100004041".into()));

        let mut c = curva_com(&a, 0, &[1.0], &[10.0], 16);
        assert!(redimensionar(&mut c, 0, 2, &a));
        assert_eq!(estado(&c), (32, 16, "020000000400000010000000180000000000803f000000000000204100000000".into()));
        assert_eq!(*a.chamadas.borrow(), vec![(true, 56, 4), (true, 64, 4), (true, 28, 4), (true, 40, 4), (true, 32, 16)]);

        let mut c = curva_com(&a, 3, &[0.25, 0.75], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], 4);
        assert!(redimensionar(&mut c, 3, 3, &a));
        unsafe {
            let p = c.as_ptr();
            assert_eq!((0..4).map(|i| c21_ponto(p, i)).collect::<Vec<_>>(), vec![Some(0.25), Some(0.75), Some(0.0), None]);
            let v: Vec<Option<f32>> = (0..3).flat_map(|i| (0..5).map(move |k| c21_valor(p, 3, i, k))).collect();
            let s = |x: f32| Some(x);
            assert_eq!(v, vec![s(1.0), s(2.0), s(3.0), s(4.0), None, s(5.0), s(6.0), s(7.0), s(8.0), None, s(0.0), s(0.0), s(0.0), s(0.0), None]);
        }
    }

    #[test]
    fn redimensionar_recusa_sem_tocar() {
        let a = Alocador::novo();
        let mut c = curva_com(&a, 0, &[1.0, 2.0], &[10.0, 20.0], 4);
        let antes = estado(&c);
        let dados = u64::from_le_bytes(c[0x08..0x10].try_into().expect("8 B"));
        unsafe { ((dados + 8) as *mut u32).write_unaligned(0x14) };
        assert!(!redimensionar(&mut c, 0, 3, &a), "pontos fora de 0x10");
        unsafe { ((dados + 8) as *mut u32).write_unaligned(0x10) };
        c[0x18..0x20].copy_from_slice(&0u64.to_le_bytes());
        assert!(!redimensionar(&mut c, 0, 3, &a), "buffer presente, alocador desconhecido");
        assert_eq!((estado(&c), a.chamadas.borrow().len()), (antes, 0));
    }
}

