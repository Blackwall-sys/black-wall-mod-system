//! Registro de FUNÇÃO/TIPO NATIVO no RTTI do jogo.
//!
//! Descoberta-chave: **registrar é o inverso de
//! resolver**. Nosso `rtti::resolve_in_class` já LÊ `funcs@CClass+0x48` /
//! `staticFuncs@+0x58`; registrar um método = dar PushBack nesses mesmos
//! `DynArray`. Registrar um global = chamar `CRTTISystem::RegisterFunction`
//! (vtbl+0xA0). O objeto-função (`CGlobalFunction`/`CClassFunction`) é um POD que
//! a gente constrói à mão, clonando a **vtable** de uma função nativa existente.
//!
//! O handler NÃO vai no objeto-função: `+0xF0`/`+0xF8` são parent/regIndex, e o motor tira o handler de uma
//! tabela global por regIndex. O executor hookado (`inicio::boot::exec_replacement`) consulta
//! `NATIVE_ROUTES` (`route_native`) e chama o handler Rust (`extern "C" fn(ctx, frame, out, retType)`)
//! direto. Slots de vtbl do CRTTISystem (workflow):
//!   +0x30 GetFunction · +0x80 RegisterType · +0xA0 RegisterFunction
//!   +0xC8 AddPostRegisterCallback · +0x100 RegisterScriptName

#[cfg(feature = "cpvr")]
use crate::vr::*;
use crate::salvamento::*;
use crate::som::*;
use crate::roupas_e_armas::*;
use crate::tarefas::*;
use crate::configuracao::*;
use crate::inicio::natives::*;
use crate::veiculos::*;
use crate::interface::*;
use crate::npcs::*;
use crate::arquivos_do_jogo::*;
use crate::corpo_e_rosto::*;
use crate::tweakdb::natives::*;
use crate::missoes_e_mapa::*;
use crate::mundo::*;
use crate::texto_e_numeros::*;
use crate::entidades::*;
use crate::tipos_do_motor::*;
use crate::diagnostico::{
    register_a11_natives, register_c118_natives, register_c122_natives, register_c122sb_natives, register_c130b_natives, register_c130c_natives,
    register_c130d_natives, register_c130e_natives, register_c133_natives, register_c134_natives, register_c142_natives, register_c6_natives,
    register_c71_natives, register_c73b_natives, register_c77_natives, register_c81t_natives, register_scriptref_proof, register_variant_natives,
    regmethod_selftest, FUNC_POD_SIZE,
};
// Só os handlers de prova usam, e o `corpo_de_prova!` público não os cita.
#[cfg(feature = "devtools")]
use crate::diagnostico::{
    tramp_a47_ciclo, tramp_bwms_emit, tramp_c115_guardar, tramp_c115c_montado, tramp_c115c_veredito, tramp_c115e_estado, tramp_c115e_montado,
    tramp_c116_estado, tramp_c116_montado, tramp_c129_anotar, tramp_c175_anotar, tramp_c176_recebido, tramp_c176e_montado, tramp_c176e_recebido, tramp_c20_anotar, tramp_c239_anotar,
    tramp_c252_anotar, tramp_c73c_anotar, tramp_c75_anotar, tramp_camscan, tramp_dbg_hasf, tramp_dbg_hast, tramp_dbg_meta0, tramp_dbg_skip,
    tramp_dump_runtime_systems, tramp_echo, tramp_engdump, tramp_make_test_axisinputevent, tramp_make_test_entitybuilderwrapper,
    tramp_make_test_entitycomponentevent, tramp_make_test_inkwidgetbrush, tramp_make_test_inkwidgetspawnevent, tramp_make_test_keyinputevent,
    tramp_make_test_vehiclelightcontrolevent, tramp_ping, tramp_trava_anotar, tramp_vector4_roundtrip,
};
use crate::aparencia::*;
use crate::curvas::*;
use crate::camera::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::cname::cname;
use crate::rtti::{self, Registry};

/// Flags de CBaseFunction, offset `+0xA8` — bit0=isNative, bit1=isStatic (CONFIRMADO
/// ao vivo 2026-08-10 via `funcflagsglobal`, corrigindo o "palpite" antigo de bit2: uma native
/// global nossa lia `flags=0x5 -> isNative|isFinal` — bit2 é `isFinal` no ground-truth de
/// `decode_function_flags`/`rtti.rs`, não `isStatic`. Toda native/função
/// global forjada por este projeto vinha marcando `isFinal` em vez de `isStatic` desde sempre —
/// bug de METADADO (não afeta despacho de chamada, que nunca dependeu deste bit — só
/// introspecção/Reflection, ex. `ReflectionFunc.IsStatic()`, lia errado).
pub(crate) const FLAG_NATIVE: u32 = 1 << 0;
pub(crate) const FLAG_STATIC: u32 = 1 << 1;

/// Assinatura que o executor RED chama para uma função nativa.
pub type NativeHandler = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, i64);

/// Handler de native de prova: o corpo real no dev, o stub no público. No público o stub só entra no RTTI se o
/// bloco do arquivo estiver ligado em [`PROVA_NO_PUBLICO`] (ver [`registro_de_prova!`]).
/// A versão pública não cita `$h`: o handler pode morar em código que só compila com `devtools`.
/// O nome registrado fica na mesma linha do `corpo_de_prova!(h)`: o grupo 9 do `bwms-audit-alcance.sh` e o
/// `dist/derivar-reds-publicos.sh` montam a lista de natives de prova lendo o fonte linha a linha.
#[cfg(feature = "devtools")]
macro_rules! corpo_de_prova {
    ($h:path) => {
        $h as NativeHandler
    };
}
#[cfg(not(feature = "devtools"))]
macro_rules! corpo_de_prova {
    ($h:path) => {
        tramp_corpo_de_prova as NativeHandler
    };
}
pub(crate) use corpo_de_prova;

/// Blocos de natives de prova que o build PÚBLICO ainda registra, com o corpo [`tramp_corpo_de_prova`]. Cada arquivo de registro
/// declara o seu `BLOCO_DE_PROVA`: `0b001` = register.rs, interface, entidades, arquivos_do_jogo; `0b010` = diagnostico, aparencia,
/// npcs; `0b100` = mundo, missoes_e_mapa. Recuo se o boot público cair no bind: `0b111` devolve o registro das 213 como era;
/// um bit por boot bissecta. Risco medido: tirar natives do `register_all` derrubou o bind na 2.31 (2/2 boots com `EXC_BREAKPOINT`
/// em `baseEngineInit.cpp:1094`, `HISTORICO.md` cont.43/44); na 2.5, tirar 12 do registro público passou em 2 boots
/// (`proofs/20261024-rodada1-*`).
pub(crate) const PROVA_NO_PUBLICO: u8 = 0;

/// Registro de native de prova. No dev, a chamada. No público, a chamada só existe se o bloco do arquivo estiver em
/// [`PROVA_NO_PUBLICO`]: com o bit desligado o ramo é constante falso e some do dylib com o nome, e o `bwms-helper natives` passa a
/// acusar qualquer `.reds` público que ainda declare a native (SIGTRAP no bind).
#[cfg(feature = "devtools")]
macro_rules! registro_de_prova {
    ($registro:expr) => {
        $registro
    };
}
#[cfg(not(feature = "devtools"))]
macro_rules! registro_de_prova {
    ($registro:expr) => {
        if ($crate::register::PROVA_NO_PUBLICO & BLOCO_DE_PROVA) != 0 {
            $registro
        } else {
            false
        }
    };
}
pub(crate) use registro_de_prova;

#[cfg(not(feature = "devtools"))]
const BLOCO_DE_PROVA: u8 = 0b001;

#[cfg(all(test, not(feature = "devtools")))]
mod testes_do_corpo_de_prova {
    use super::{tramp_corpo_de_prova, NativeHandler};

    #[test]
    fn publico_nao_cita_o_handler() {
        // O caminho não existe: só compila porque a versão pública não cita `$h`, e é isso que deixa o handler morar em código só do dev.
        let h = corpo_de_prova!(so_no_dev::handler_inexistente);
        assert_eq!(h as usize, tramp_corpo_de_prova as NativeHandler as usize);
    }
}

/// ROTEAMENTO de nativas registradas: ponteiro do POD da func -> handler Rust.
///
/// O despacho NÃO usa o handler no objeto (`+0xF0`/`+0xF8` = parent/
/// regIndex; o engine pega o handler de uma tabela GLOBAL por regIndex). Em vez de
/// achar essa tabela, o executor (`inicio::boot::exec_replacement`, hook que já temos)
/// consulta ESTE mapa ANTES de cair na via nativa do jogo: se `func` é nossa, chama
/// o handler direto e retorna. Escrito no register e lido pelo executor em TODAS as threads que rodam script (a do jogo e as de job):
/// leitura compartilhada, que não falha por concorrência. Era `Mutex` + `try_lock` com a nota "sem concorrência real": com handlers de
/// script em thread de job, o `try_lock` falhava, a chamada caía na via nativa do jogo sem o handler e o getter voltava vazio — no boot 169,
/// leituras em 0 no teste do `Entity/Extract` e anotações perdidas no dos eventos `Entity/*`, poucas em centenas de despachos.
// PERF (2026-08-16, achado ao vivo via lldb num boot travado, ~890% CPU sem progresso):
// era `Vec<(usize, NativeHandler)>` com busca LINEAR (`v.iter().find`) — a cada chamada de
// `exec_replacement` (TODA chamada de script do jogo inteiro, hot-path extremo). Com centenas
// de natives registradas ao longo de muitas sessões, isso vira O(n) por chamada, compondo
// catastroficamente durante o streaming pesado do boot/load de mundo (milhões de chamadas).
// `BTreeMap` dá O(log n) sem precisar de inicializador não-const (HashMap::new() não é const
// por causa do RandomState). Backtrace decisivo em
// `cp77-symbols/notes/proofs/2026-08-16-boot-stall-exec-replacement-route-native-linear-scan-lldb-backtrace.log`.
pub(crate) static NATIVE_ROUTES: std::sync::RwLock<std::collections::BTreeMap<usize, NativeHandler>> =
    std::sync::RwLock::new(std::collections::BTreeMap::new());
pub(crate) static NATIVE_ROUTE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Filtro sem trava na frente de `NATIVE_ROUTES`: 1 bit por `(func >> 4) & 0xFFFF`. Quase toda chamada do executor é
/// função do jogo, e o bit desligado responde sem tocar a palavra do `RwLock` (a leitura dela custa 2 RMW por chamada, de
/// todas as threads de script). Bit ligado só diz "pode ser rota": colisão cai no `BTreeMap`, que decide. Nenhuma rota sai
/// da tabela, então bit ligado nunca fica errado. Alinhado em 128 B (a linha de cache do Apple Silicon) para não dividir
/// linha com a trava, que a chamada roteada ainda grava.
#[repr(align(128))]
pub(crate) struct FiltroDeRotas([AtomicU64; 1024]);

impl FiltroDeRotas {
    const fn novo() -> Self {
        Self([const { AtomicU64::new(0) }; 1024])
    }

    fn posicao(func: usize) -> (usize, u64) {
        let i = (func >> 4) & 0xFFFF;
        (i >> 6, 1 << (i & 63))
    }

    fn ligar(&self, func: usize) {
        let (palavra, bit) = Self::posicao(func);
        self.0[palavra].fetch_or(bit, Ordering::Relaxed);
    }

    /// `Relaxed` basta: o bit liga antes de a func entrar no RTTI (`build_native_func`), e quem chama a func a achou pelo
    /// RTTI, que publica sob trava do motor.
    fn pode_ter(&self, func: usize) -> bool {
        let (palavra, bit) = Self::posicao(func);
        self.0[palavra].load(Ordering::Relaxed) & bit != 0
    }
}

pub(crate) static ROTA_BITS: FiltroDeRotas = FiltroDeRotas::novo();

/// Corpo do `add_route` sobre tabelas recebidas: o teste usa tabelas próprias sem mexer na contagem global, que invalida o
/// cache de `rtti::resolve_in_class` dos outros testes. O bit e a contagem mudam sob a trava de escrita, depois do insert.
pub(crate) fn incluir_rota(
    filtro: &FiltroDeRotas,
    rotas: &std::sync::RwLock<std::collections::BTreeMap<usize, NativeHandler>>,
    total: &AtomicUsize,
    func: usize,
    handler: NativeHandler,
) {
    if let Ok(mut m) = rotas.write() {
        m.insert(func, handler);
        filtro.ligar(func);
        total.store(m.len(), Ordering::Relaxed);
    }
}

/// Corpo do `route_native` sobre tabelas recebidas (mesmo motivo do `incluir_rota`).
#[inline]
fn rota_de(
    filtro: &FiltroDeRotas,
    rotas: &std::sync::RwLock<std::collections::BTreeMap<usize, NativeHandler>>,
    func: usize,
) -> Option<NativeHandler> {
    if !filtro.pode_ter(func) {
        return None;
    }
    let m = rotas.read().ok()?;
    m.get(&func).copied()
}

#[cfg(test)]
mod testes_do_filtro_de_rotas {
    use super::{incluir_rota, rota_de, FiltroDeRotas, NativeHandler};
    use std::collections::BTreeMap;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::RwLock;

    unsafe extern "C" fn handler_a(_: *mut c_void, _: *mut c_void, out: *mut c_void, _: i64) {
        *(out as *mut u8) = b'a';
    }
    unsafe extern "C" fn handler_b(_: *mut c_void, _: *mut c_void, out: *mut c_void, _: i64) {
        *(out as *mut u8) = b'b';
    }

    /// Qual handler veio: roda e lê a marca que ele grava no `out`.
    fn marca(h: Option<NativeHandler>) -> Option<u8> {
        h.map(|h| {
            let mut out = 0u8;
            unsafe { h(std::ptr::null_mut(), std::ptr::null_mut(), (&mut out as *mut u8).cast(), 0) };
            out
        })
    }

    #[test]
    fn rota_incluida_acha_o_handler_e_colisao_no_bit_cai_no_mapa() {
        let filtro = FiltroDeRotas::novo();
        let rotas = RwLock::new(BTreeMap::new());
        let total = AtomicUsize::new(0);
        let p = 0x1_2345_6780usize;
        let fora = p + 0x10;
        // Mesmo `(func >> 4) & 0xFFFF`: 1 MiB adiante.
        let colide = p + 0x10_0000;
        assert_eq!(marca(rota_de(&filtro, &rotas, p)), None, "tabela vazia");

        incluir_rota(&filtro, &rotas, &total, p, handler_a);
        assert_eq!(marca(rota_de(&filtro, &rotas, p)), Some(b'a'));
        assert_eq!(total.load(Ordering::Relaxed), 1);
        assert!(!filtro.pode_ter(fora));
        assert_eq!(marca(rota_de(&filtro, &rotas, fora)), None);
        assert!(filtro.pode_ter(colide), "a colisão passa no filtro");
        assert_eq!(marca(rota_de(&filtro, &rotas, colide)), None, "o BTreeMap decide a colisão");

        incluir_rota(&filtro, &rotas, &total, colide, handler_b);
        assert_eq!(marca(rota_de(&filtro, &rotas, colide)), Some(b'b'));
        assert_eq!(marca(rota_de(&filtro, &rotas, p)), Some(b'a'));
        assert_eq!(total.load(Ordering::Relaxed), 2);
    }
}

// `func` da native EM EXECUÇÃO, publicado pelo executor (exec_replacement) ANTES de chamar o
// handler → o handler lê seus args do frame via `read_params_consuming(func, frame)`. Por THREAD:
// era um global com a nota "game-thread, sem corrida real", e deixou de ser verdade quando handlers
// de script passaram a rodar em thread de job (fila de jobs de script, eventos `Entity/*`). No boot 167 o
// handler de `Entity/Initialize` leu parâmetros com o descritor de outra native (7 anotações perdidas
// e 4 entidades erradas em 260 despachos). Re-entrância (native→native) na MESMA thread ainda
// sobrescreve, mas nossos handlers são leaf e leem os args no começo.
thread_local! {
    static CURRENT_NATIVE_FUNC: std::cell::Cell<*mut c_void> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}
pub fn set_current_native_func(f: *mut c_void) {
    CURRENT_NATIVE_FUNC.with(|c| c.set(f));
}
pub fn current_native_func() -> *mut c_void {
    CURRENT_NATIVE_FUNC.with(|c| c.get())
}

/// Quantas funções o BWMS já registrou no RTTI. Muda a cada native nova (inclusive de plugin), e o cache de
/// `rtti::resolve_in_class` usa isso para saber que uma classe pode ter ganhado método.
pub(crate) fn rotas_registradas() -> usize {
    NATIVE_ROUTE_COUNT.load(Ordering::Relaxed)
}

/// O executor consulta isto a CADA chamada de script, de toda thread. Bit desligado em `ROTA_BITS` (quase toda chamada,
/// e todas antes do 1º registro) volta com 1 load, sem trava; bit ligado lê `NATIVE_ROUTES`. Se `func` é nossa, devolve o
/// handler. A leitura solta a trava antes de o executor chamar o handler, então native que chama script que chama native
/// não reentra na trava.
pub unsafe fn route_native(func: *mut c_void) -> Option<NativeHandler> {
    rota_de(&ROTA_BITS, &NATIVE_ROUTES, func as usize)
}

#[inline]
pub(crate) unsafe fn wr_u64(base: *mut c_void, off: usize, v: u64) {
    core::ptr::write_unaligned((base as *mut u8).add(off) as *mut u64, v);
}
#[inline]
pub(crate) unsafe fn wr_u32(base: *mut c_void, off: usize, v: u32) {
    core::ptr::write_unaligned((base as *mut u8).add(off) as *mut u32, v);
}
/// `u64` em `base + off` sem conferir legibilidade: quem chama já provou o endereço. `rtti::rd_u64` e `ler_u64` são
/// as formas de 1 argumento; a leitura que confere é `memoria::read_u64`.
#[inline]
pub(crate) unsafe fn rd_u64(base: *const c_void, off: usize) -> u64 {
    core::ptr::read_unaligned((base as *const u8).add(off) as *const u64)
}
/// Variante de 4 bytes de `rd_u64`, mesmo idioma (`read_unaligned`).
#[inline]
pub(crate) unsafe fn rd_u32(base: *const c_void, off: usize) -> u32 {
    core::ptr::read_unaligned((base as *const u8).add(off) as *const u32)
}

#[cfg(test)]
mod testes_da_leitura_u64 {
    use super::{ler_u64, eb_u64, rd_u64, read_u64_field};
    use std::ffi::c_void;

    // 0x1000 cai no __PAGEZERO do executável de teste: uma forma que deixasse de conferir derrubaria o processo aqui.
    const FORA_DO_MAPA: u64 = 0x1000;

    #[test]
    fn as_formas_leem_o_mesmo_u64_desalinhado() {
        let bytes: [u8; 24] = std::array::from_fn(|i| 0x10 + i as u8);
        let base = bytes.as_ptr();
        for off in [0usize, 1, 3, 7, 13, 16] {
            let mut le = [0u8; 8];
            le.copy_from_slice(&bytes[off..off + 8]);
            let esperado = u64::from_le_bytes(le);
            unsafe {
                assert_eq!(rd_u64(base as *const c_void, off), esperado, "rd_u64 +{off}");
                assert_eq!(crate::rtti::rd_u64(base.add(off)), esperado, "rtti::rd_u64 +{off}");
                assert_eq!(ler_u64(base as u64 + off as u64), esperado, "ler_u64 +{off}");
                assert_eq!(read_u64_field(base as *mut c_void, off), Some(esperado), "read_u64_field +{off}");
                assert_eq!(eb_u64(base as u64, off as u64), esperado, "eb_u64 +{off}");
            }
        }
    }

    #[test]
    fn as_formas_que_conferem_recusam_nulo_e_fora_do_mapa() {
        unsafe {
            assert_eq!(read_u64_field(std::ptr::null_mut(), 0), None);
            assert_eq!(read_u64_field(std::ptr::null_mut(), 0x40), None);
            assert_eq!(read_u64_field(FORA_DO_MAPA as *mut c_void, 8), None);
            assert_eq!(eb_u64(0, 0x40), 0);
            assert_eq!(eb_u64(FORA_DO_MAPA, 8), 0);
        }
    }
}
/// Escreve o retorno Bool (u8) de uma native no `out` do frame, se não-nulo. VIA CANÔNICA dos
/// trampolines — antes o `if !out.is_null() { write_unaligned(out as *mut u8, v as u8); }` estava
/// repetido ~16× (a auditoria de convergência apontou o boilerplate).
#[inline]
pub(crate) unsafe fn write_bool_ret(out: *mut c_void, v: bool) {
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, v as u8);
    }
}
/// Escreve um Int32 no slot de retorno. Nasceu pro seletor de modo CPVR (`#[cfg(feature =
/// "cpvr")]`), mas hoje é usada por várias natives de `BwmsAparencia` (`GetVisualTagOverrideCount`/
/// `ApplyVisualTagOverride`/`GetOffsetOverride`, nenhuma delas gateada por `cpvr`) — achado real
/// 2026-08-16: o gate antigo quebrava o build PÚBLICO (`build-core.sh` sem features, o único que
/// NÃO liga `cpvr` transitivamente via `devtools`) com `E0425 cannot find function write_i32_ret`
/// nesses 2+ call-sites pré-existentes, nunca pego porque `deploy.sh`/todo teste de dev sempre usa
/// `--features devtools`. Removido o gate — função trivial (4 bytes, sem I/O, sem side-effect),
/// sem motivo real pra ficar atrás de feature nenhuma.
pub(crate) unsafe fn write_i32_ret(out: *mut c_void, v: i32) {
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut i32, v);
    }
}

/// Escreve um `Float` no slot de retorno. Mesma largura do `i32`, mas com nome próprio: passar
/// `to_bits() as i32` pro `write_i32_ret` funciona e esconde a intenção — e retorno de float
/// errado neste projeto vira coordenada plausível, não erro.
pub(crate) unsafe fn write_f32_ret(out: *mut c_void, v: f32) {
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut f32, v);
    }
}

/// Escreve um inteiro sem sinal no slot de retorno, na LARGURA certa (1/2/4/8 bytes) — usado
/// pelos `BitSetN`/`BitShiftLN`/`BitShiftRN`, cujo retorno é
/// `UintN` (N variando por função). VIA CANÔNICA, mesmo espírito do `write_bool_ret`.
#[inline]
pub(crate) unsafe fn write_uint_ret(out: *mut c_void, v: u64, width_bytes: u8) {
    if out.is_null() {
        return;
    }
    match width_bytes {
        1 => core::ptr::write_unaligned(out as *mut u8, v as u8),
        2 => core::ptr::write_unaligned(out as *mut u16, v as u16),
        4 => core::ptr::write_unaligned(out as *mut u32, v as u32),
        _ => core::ptr::write_unaligned(out as *mut u64, v),
    }
}

/// Escreve um `ref<T>` (handle) NOVO no slot de retorno — 2026-07-18, achado ao vivo (crash-report
/// `Cyberpunk2077-2026-07-18-144828.ips`, `SIGBUS EXC_ARM_DA_ALIGN`): um `ref<T>` LOCAL em bytecode
/// compilado NÃO é um ponteiro cru de 8 bytes — é uma estrutura de **16 bytes**: `[+0x00]`=ponteiro
/// do objeto, `[+0x08]`=ponteiro pro BLOCO DE REFCOUNT (ou `0` = sem refcount/"não-dono", igual ao
/// padrão de `weak`/raw). Confirmado por disasm da rotina de RELEASE que o compilador injeta no
/// teardown de TODO frame com locais `ref<>` (`0x101025dc8`, achada ao vivo no crash): lê
/// `[slot+0x08]`, se não-nulo faz `ldaddal w9,w8,[refcount_block+4]` (decremento ATÔMICO, exige
/// alinhamento de 4 bytes — o crash: `write_uint_ret` só escrevia os 8 bytes de `[+0x00]`, deixando
/// `[+0x08]` com LIXO da stack; a rotina lia esse lixo como ponteiro-de-refcount-block e o `ldaddal`
/// numa base desalinhada faturava SIGBUS). Fix: escrever os 16 bytes — `[+0x08]=0` EXPLÍCITO faz a
/// rotina de release tomar o ramo `cbz x8,...` (sem refcount = no-op seguro), igual a um handle
/// "raw"/sem dono. Usado por TODA função nossa que devolve `ref<classe-forjada>`.
/// Objeto do motor que não deriva de `IScriptable` (nó de quest, recurso) sobe dentro de um `BwmsSerializable` (`recurso_para_script`): o `as` do
/// script lê o tipo no `+0x30` e cairia com ele cru (boot 186). Objeto `IScriptable` e ponteiro que não é objeto saem como antes.
#[inline]
pub(crate) unsafe fn write_handle_ret(out: *mut c_void, ptr: u64) {
    if out.is_null() {
        return;
    }
    let h = if ptr != 0 && !rtti::class_of(ptr as *mut c_void).is_null() { recurso_para_script([ptr, 0]) } else { [ptr, 0] };
    core::ptr::write_unaligned(out as *mut u64, h[0]);
    core::ptr::write_unaligned((out as *mut u64).add(1), h[1]);
}

/// Escreve os 16 bytes crus de um `Vector4`/`Quaternion` NOVO no slot de retorno —
/// `StaticEntitySpec.GetPosition`/`GetOrientation` (2026-08-15, rodada 10). Ambos são POD
/// puro (4 floats, sem allocator/ownership — ao contrário de `CString` acima), então é um memcpy
/// direto: `out` já vem alocado pelo TAMANHO do tipo de retorno DECLARADO pelo executor real do
/// jogo (mesma garantia documentada em `write_cstring_inline_ret`, que escreve 0x20 bytes no
/// mesmo `out`) — 16 bytes é estritamente MENOR que o maior caso já provado nesse slot.
#[inline]
pub(crate) unsafe fn write_raw16_ret(out: *mut c_void, bytes: &[u8; 16]) {
    if out.is_null() {
        return;
    }
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, 16);
}

/// Retorno `String` de native, de qualquer tamanho, pelo construtor de texto do motor ([`crate::jogo::cstring::nova`]). Até o boot 268 escrevia
/// só o buffer inline (até 19 bytes) e devolvia vazio pro resto; o `bwmsLongo` de 60 caracteres do teste de argumentos provou o caminho.
pub(crate) unsafe fn write_cstring_inline_ret(out: *mut c_void, s: &str) -> bool {
    if out.is_null() {
        return false;
    }
    crate::jogo::cstring::nova(out as *mut u8, s);
    true
}

/// `CRTTISystem::GetFunction(CName)` (vtbl+0x30) — acha um GLOBAL existente. Usado
/// p/ (a) clonar a vtable de CGlobalFunction e (b) confirmar que nosso registro
/// entrou (re-resolve por nome).
pub unsafe fn get_function(reg: &Registry, name: &str) -> *mut c_void {
    let slot = reg.vtbl_slot(0x30);
    if !rtti::sane(slot) {
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(slot);
    let fast = f(reg.raw(), cname(name));
    if !fast.is_null() {
        return fast;
    }
    // FALLBACK MODULE-SCOPING (item #55, 2026-08-21) — o nativo acima resolve por HASH de
    // `CName`, então uma global declarada dentro de `module X.Y` **não é achável pelo nome BARE**:
    // o RTTI guarda a forma QUALIFICADA, cujo hash é outro. Confirmado ao vivo em 2026-08-17
    // ("nome bare falha, nome qualificado resolve e devolve o valor exato esperado") — é a razão de
    // o item ter voltado de INCERTO pra REAL_GAP.
    //
    // `resolve_global_function_robust` varre `GetGlobalFunctions` e compara por STRING, casando o
    // último segmento (`MinhaFunc` acha `MeuModulo.MinhaFunc`). Fica aqui, na via de PRODUÇÃO, e
    // não só no comando de diagnóstico — senão a correção não seria alcançável por ninguém: este é
    // o resolvedor que a API C-ABI dos plugins (`api.rs`), o `fnoverride` e dezenas de call-sites
    // usam. Custo zero no caminho comum (só roda quando o nativo já devolveu null, isto é, quando
    // hoje o resultado seria "não achou").
    rtti::resolve_global_function_robust(reg, name)
}

/// `CRTTISystem::RegisterFunction(CGlobalFunction*)` (vtbl+0xA0).
pub(crate) unsafe fn call_register_function(reg: &Registry, func: *mut c_void) -> bool {
    let slot = reg.vtbl_slot(0xA0);
    if !rtti::sane(slot) {
        crate::log_dev!("[reg] RegisterFunction (vtbl+0xA0) ilegível");
        return false;
    }
    let f: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(slot);
    f(reg.raw(), func);
    true
}

/// Constrói um objeto-função nativo (POD) à mão: clona a `vtable` de `proto`
/// (uma função nativa existente do mesmo tipo) e preenche os campos conhecidos.
/// Devolve o ponteiro ou null.
pub(crate) unsafe fn build_native_func(
    proto: *mut c_void,
    full: &str,
    short: &str,
    handler: NativeHandler,
    is_static: bool,
) -> *mut c_void {
    if !rtti::sane(proto) {
        return std::ptr::null_mut();
    }
    let vtable = rd_u64(proto as *const c_void, 0x00);
    if vtable == 0 {
        return std::ptr::null_mut();
    }
    let mem = rtti::pool_alloc(FUNC_POD_SIZE, 8);
    if mem.is_null() {
        return std::ptr::null_mut();
    }
    std::ptr::write_bytes(mem as *mut u8, 0, FUNC_POD_SIZE);
    wr_u64(mem, 0x00, vtable); // vtable clonada
    wr_u64(mem, 0x08, cname(full)); // fullName (CName)
    wr_u64(mem, 0x10, cname(short)); // shortName (CName)
    let mut flags = FLAG_NATIVE;
    if is_static {
        flags |= FLAG_STATIC;
    }
    wr_u32(mem, 0xA8, flags); // flags
    // O handler NÃO vai no objeto: 0xF0/0xF8 são parent/regIndex, e
    // escrever um ponteiro de 64 bits lá corromperia o regIndex (se o engine algum dia
    // lesse). Deixamos 0xF0/0xF8 zerados e roteamos func->handler pelo executor (add_route).
    add_route(mem, handler);
    crate::log_dev!(
        "[reg] build {full} -> {mem:p} (vtable={vtable:#x}, rota func->handler registrada)"
    );
    mem
}

/// Registra um GLOBAL nativo. Clona a vtable de `proto_global` (ex.: GetFunction
/// de uma global conhecida). Retorna sucesso.
pub unsafe fn register_global(
    reg: &Registry,
    proto_global: *mut c_void,
    full: &str,
    short: &str,
    handler: NativeHandler,
) -> bool {
    let func = build_native_func(proto_global, full, short, handler, true);
    if func.is_null() {
        return false;
    }
    if !call_register_function(reg, func) {
        return false;
    }
    // Confirma: re-resolve por nome.
    let back = get_function(reg, full);
    let ok = rtti::sane(back);
    crate::log_dev!("[reg] register_global {full}: re-resolve -> {} ({back:p})", if ok { "OK" } else { "FALHOU" });
    ok
}

/// FIX (2026-07-18, sessão `dynarraygrowth-probe`): todo forge de CClass abaixo aloca `CCLASS_SIZE`
/// (0x2D0) bytes e faz `write_bytes(mem, 0, CCLASS_SIZE)` antes de escrever só um punhado de campos
/// conhecidos (vtable/name/size/flags/parent/...). O RESTO fica zerado — inclusive vários hashmaps
/// EMBUTIDOS (bucket array + entries + capacity + stride + um "alocador" sub-objeto cujo 1º campo
/// é um vtable pointer, formato confirmado via a rotina de crescimento `0x100ff5f84`, ver
/// `HISTORICO.md` 2026-07-18 cont. 99). Rastreado AO VIVO (crash-report + hook 2026-07-18):
/// quando o engine insere a 1ª entrada num desses hashmaps (achado disparando ao registrar
/// `GetService` numa classe forjada — happens no 1º `register_method` de uma classe), um invoke-
/// thunk genérico desreferencia o vtable NULO do alocador -> SIGSEGV. Achados 2 slots ao vivo
/// (`CClass+0x78` e `CClass+0xA8`, exatamente 0x30 bytes = tamanho de 1 slot, um atrás do outro).
///
/// Esta função varre candidatos espaçados de 0x30 a partir de `0x78` (dentro de `CCLASS_SIZE`) e,
/// pra CADA um, só copia o vtable do alocador (`slot+0x28`, 8 bytes) do DONOR real se: (a) os
/// primeiros 0x28 bytes do slot no donor forem TODOS "vazio" (cada palavra de 32 bits é `0x0` OU
/// `0xFFFFFFFF` — o MESMO esquema de sentinela -1 que a própria rotina de crescimento usa pros
/// slots de bucket vazios, achado ao vivo: `container+0x20` do slot @0xA8 é `0xFFFFFFFF`, não
/// zero puro — "vazio" não é só zero neste formato) — o MESMO estado que o nosso forge já produz
/// (zero) OU um estado "vazio" equivalente, provando que não há dado per-instância ali, só a
/// vtable estática do tipo alocador — E (b) o valor em `slot+0x28` passar `rtti::sane()` (parece
/// mesmo um ponteiro, não lixo). Auto-guardado: nunca copia um slot que não bata as 2 condições —
/// pior caso é deixar aquele slot específico com o mesmo risco de antes (nunca corrompe nada, só
/// não conserta um slot que a heurística não reconheceu). Loga 1x por slot corrigido.
pub(crate) unsafe fn fix_embedded_allocator_vtables(mem: *mut c_void, src: *mut c_void, cclass_size: usize) {
    const SLOT_STRIDE: usize = 0x30;
    const FIRST_SLOT: usize = 0x78;
    let mut off = FIRST_SLOT;
    let mut fixed = 0usize;
    let mut checked = 0usize;
    while off + SLOT_STRIDE <= cclass_size {
        checked += 1;
        if crate::memoria::is_readable((src as *const u8).add(off) as *const c_void, SLOT_STRIDE) {
            let mut prefix_empty = true;
            for i in (0..0x28usize).step_by(4) {
                let w = ((src as *const u8).add(off + i) as *const u32).read_unaligned();
                if w != 0 && w != 0xFFFF_FFFF {
                    prefix_empty = false;
                    break;
                }
            }
            if prefix_empty {
                let alloc_vt = rd_u64(src as *const c_void, off + 0x28);
                if rtti::sane(alloc_vt as *mut c_void) {
                    wr_u64(mem, off + 0x28, alloc_vt);
                    fixed += 1;
                }
            }
        }
        off += SLOT_STRIDE;
    }
    // Os mapas da CClass fora da grade de 0x30: `propsByName` em `0xE8` (o motor o refaz quando a classe tem propriedade, em `0x1010c481c`,
    // e com o alocador nulo lia `[0]+8` — boot 278) e o de `0x180`. Só a vtable do alocador (estática do tipo) vem do doador.
    for off in [0xE8usize, 0x180] {
        if off + 0x30 <= cclass_size && rd_u64(mem as *const c_void, off + 0x28) == 0 {
            let alloc_vt = rd_u64(src as *const c_void, off + 0x28);
            if rtti::sane(alloc_vt as *mut c_void) {
                wr_u64(mem, off + 0x28, alloc_vt);
                fixed += 1;
            }
        }
    }
    if fixed > 0 {
        crate::log_dev!(
            "[reg] fix_embedded_allocator_vtables: {fixed}/{checked} slot(s) corrigido(s) (hashmap embutido, vtable copiado do donor)"
        );
    }
}

/// Classes nativas de onde a forja mínima tira a vtable, na ordem de preferência: a 1ª que resolver é a doadora.
pub(crate) const DOADORAS_DA_FORJA_MINIMA: [&str; 6] = ["gameuiInGameMenuGameController", "ScriptableSystem", "GameObject", "IScriptable", "PlayerPuppet", "gameObject"];

/// RegisterType STEP-1: forja uma CClass MÍNIMA e registra via `RegisterType@vtbl+0x80`, sem instanciar
/// (mapa do CClass em proofs/map-cclass-forge). A vtable é CLONADA de uma classe nativa (getters reais GetName@0x10/GetSize@0x18/
/// GetType@0x28), e `isAbstract=1` blinda contra CreateInstance. Fora do zero do memset: parent `IScriptable`, size/holderSize/align
/// da doadora (AL-2b) e a vtable do alocador dos mapas embutidos (`fix_embedded_allocator_vtables`). NÃO popula prop/func/default.
/// Layout CClass (0x2D0): name(CName)@0x18, size(u32)@0x68, flags@0x70, alignment@0x74. Confirmar com `rttidump <nome>` depois.
/// Retorna a forja ou null.
pub unsafe fn register_type_min(reg: &Registry, new_name: &str) -> *mut c_void {
    const CCLASS_SIZE: usize = 0x2D0;
    // dedup DURO: sobrescrever um tipo existente corromperia o RTTI global → aborta se já existe.
    if rtti::sane(reg.class_by_name(new_name)) {
        crate::log_dev!("[reg] register_type: '{new_name}' JÁ existe — abortado (não sobrescrevo o RTTI)");
        return std::ptr::null_mut();
    }
    crate::log_dev!("[reg] register_type: entrou p/ '{new_name}'");
    // fonte da vtable: 1ª classe nativa concreta que resolver (só usamos os getters dela).
    let mut src = std::ptr::null_mut();
    let mut src_name = "";
    for n in DOADORAS_DA_FORJA_MINIMA {
        let c = reg.class_by_name(n);
        if rtti::sane(c) {
            src = c;
            src_name = n;
            break;
        }
    }
    if !rtti::sane(src) {
        crate::log_dev!("[reg] register_type: nenhuma classe-fonte p/ clonar a vtable");
        return std::ptr::null_mut();
    }
    let vtable = rd_u64(src as *const c_void, 0x00);
    crate::log_dev!("[reg] register_type: fonte='{src_name}' cls={src:p} vtable={vtable:#x}");
    if vtable == 0 {
        crate::log_dev!("[reg] register_type: vtable da fonte = 0 (abortado)");
        return std::ptr::null_mut();
    }
    let mem = rtti::pool_alloc(CCLASS_SIZE, 8);
    if mem.is_null() {
        crate::log_dev!("[reg] register_type: pool_alloc(0x2D0) devolveu null (abortado)");
        return std::ptr::null_mut();
    }
    crate::log_dev!("[reg] register_type: forja alocada {mem:p}");
    std::ptr::write_bytes(mem as *mut u8, 0, CCLASS_SIZE); // memset 0 ANTES de qualquer campo
    wr_u64(mem, 0x00, vtable); // vtable CLONADA (getters reais)
    wr_u64(mem, 0x18, cname(new_name)); // name (CName) — a chave do RegisterType
    wr_u32(mem, 0x68, 0x40); // size = sizeof(IScriptable)
    wr_u32(mem, 0x70, 0x03); // flags: isAbstract(bit0)|isNative(bit1) — blinda instanciar
    wr_u32(mem, 0x74, 0x08); // alignment = 8 (NUNCA 0: AllocMemory faria AlignUp(size,0)=crash)
    // AL-2b (2026-09-12, RE `proofs/20260911-allocator-al2-createinstance-offline.log`): a vtable acima é da doadora, e o ConstructCls dela
    // (`gameuiInGameMenuGameController`, ctor `0x104accb88`) escreve até `+0x188`. Uma subclasse reds desta forja (ex.: de `ScriptableTweak`)
    // herda o size daqui e estourava o bloco de 0x40 B na criação. Por isso size, holderSize e align saem da doadora, como na forja
    // instanciável — nos dois builds: um mod de terceiro que estende `ScriptableTweak`/`ScriptableService` cai aqui no jogo do usuário.
    if crate::memoria::is_readable(src as *const c_void, 0x78) {
        let src_size = ((src as *const u8).add(0x68) as *const u32).read_unaligned();
        let src_holder = ((src as *const u8).add(0x6c) as *const u32).read_unaligned();
        let src_align = ((src as *const u8).add(0x74) as *const u32).read_unaligned();
        if src_size >= 0x40 && src_size <= 0x10000 {
            wr_u32(mem, 0x68, src_size);
            wr_u32(mem, 0x6c, src_holder.max(src_size));
            wr_u32(mem, 0x74, src_align.max(8));
            crate::veredito_dev!(
                "[al2b] forja mínima '{new_name}' com o layout da doadora '{src_name}': size {src_size:#x} holder {:#x} align {:#x}",
                src_holder.max(src_size),
                src_align.max(8)
            );
        }
    }
    // PARENT (CClass+0x10) — achado 2026-07-13 via RE offline (disasm de 0x1011a84b4, o
    // validador de classe nativa do bundle): ele chama um walker `IsKindOf(cls, base)` que segue
    // ESTE ponteiro em loop (até ~16 níveis) comparando contra um ponteiro-base fixo (cache
    // global, quase certeza IScriptable — todo `native class` sem `extends` explícito herda
    // implicitamente dele). O memset-zero do forge deixava esse campo NULL, então o walker
    // falhava na 1ª iteração (cbz) — a classe "existia" e os métodos resolviam por nome (nossa
    // reflection não olha o parent), mas o VALIDADOR NATIVO do motor rejeitava (retorno 0 pra
    // fachada de versão, confirmado ao vivo mesmo com Version/Require=true). Setar o parent de verdade é
    // o candidato mais forte pro fix real da Facade.
    // CONTROLE FEITO (2026-07-13, mesma sessão): desliguei isto por 1 boot pra isolar a causa de
    // 4 travamentos seguidos em t=38s/phase=-1 — o boot de CONTROLE (sem este write) travou no
    // MESMO ponto exato, provando que a causa é degradação ambiental da máquina (sessão de
    // muitas horas, uptime 1d+14h), NÃO este código. Reativado.
    let parent = reg.class_by_name("IScriptable");
    if rtti::sane(parent) {
        wr_u64(mem, 0x10, parent as u64);
        crate::log_dev!("[reg] register_type: parent (CClass+0x10) = IScriptable @ {parent:p}");
    } else {
        crate::log_dev!("[reg] register_type: IScriptable não resolveu — parent fica NULL (achado 2026-07-13 sugere que isso quebra a validação nativa)");
    }
    // FIX (2026-07-18, sessão `dynarraygrowth-probe`): mesmo fix de `register_type_instantiable_
    // with_parent` — conserta os hashmaps embutidos ANTES do RegisterType (ver
    // `fix_embedded_allocator_vtables`). Classes forjadas por ESTA função hoje têm 0 métodos
    // (`ScriptableService`/`CallbackSystemTarget`), mas aplicar aqui tb blinda qualquer
    // `register_method` futuro nelas contra o mesmo crash.
    fix_embedded_allocator_vtables(mem, src, CCLASS_SIZE);
    // RegisterType@vtbl+0x80 = IRTTISystem::RegisterType(IType* type, u32 hash=0).
    let slot = reg.vtbl_slot(0x80);
    if !rtti::sane(slot) {
        crate::log_dev!("[reg] RegisterType (vtbl+0x80) ilegível");
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void, *mut c_void, u32) = std::mem::transmute(slot);
    f(reg.raw(), mem, 0);
    // confirma SEM instanciar: re-resolve por nome.
    let back = reg.class_by_name(new_name);
    let ok = rtti::sane(back);
    crate::log_dev!(
        "[reg] register_type '{new_name}' -> forja {mem:p} (vtable {vtable:#x} de fonte {src:p}); re-resolve {} ({back:p})",
        if ok { "OK ✓ (RTTI aceitou a classe forjada)" } else { "FALHOU" }
    );
    if ok {
        mem
    } else {
        std::ptr::null_mut()
    }
}

/// Protótipo doador GENÉRICO pra `register_method` (só serve de molde de `CClassFunction`/
/// convenção de chamada C-ABI — nunca é chamado de verdade, o handler real é o que importa).
/// Sempre `gameGodModeSystem::AddGodMode`, qualquer método estático nativo simples serviria.
/// Extraído em 2026-08-06 (cont.26): as ~22 funções `register_*` desta família
/// (CallbackSystem/DynamicEntitySystem/ScriptableServiceContainer/ScriptableTweak/etc., cada
/// uma forjada numa sessão diferente ao longo de semanas) todas reimplementavam o MESMO bloco
/// de 4 linhas (`rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode")` + match) —
/// redundância acumulada do processo "6 mods tratados separado, depois unificados" que o
/// projeto vinha carregando. Consolidado aqui; cada call-site mantém sua própria mensagem de
/// erro (qual `register_*` falhou), só a resolução em si deixou de ser duplicada.
pub unsafe fn method_donor_proto(reg: &Registry) -> Option<*mut c_void> {
    rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode").map(|rf| rf.func)
}

pub(crate) unsafe fn dynarray_push_ptr(arr: *mut c_void, val: u64) -> Option<usize> {
    dynarray_push_bytes(arr, &val.to_le_bytes())
}

/// Append de `elem` (passo = `elem.len()`) num `DynArray` do motor, pelo `es22_vaga_no_fim`: array vazio cresce pelo reserve do motor,
/// que grava o alocador; cheio, pelo `dynarray_crescer`, que copia o do buffer antigo. Buffer sem alocador derrubou o jogo nos boots
/// 168 e 184 (o motor solta o array pela palavra depois dos `cap` elementos). O tamanho é publicado por último.
pub(crate) unsafe fn dynarray_push_bytes(arr: *mut c_void, elem: &[u8]) -> Option<usize> {
    if !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return None;
    }
    let a = arr as *mut u8;
    let tam = (a.add(0x0c) as *const u32).read_unaligned();
    let vaga = es22_vaga_no_fim(a, elem.len())?;
    std::ptr::copy_nonoverlapping(elem.as_ptr(), vaga, elem.len());
    (a.add(0x0c) as *mut u32).write_unaligned(tam + 1);
    Some(tam as usize)
}

/// construtor de entidade (2026-07-24): PushBack num `DynArray<Handle<T>>` — elemento de **16 bytes**
/// (`{instance: T*, refCount: RefCnt*}`, o MESMO layout de `rtti::make_handle`/`SharedPtrBase<T>`:
/// `T* instance; RefCnt* refCount;`, 2 ponteiros). Diferente de `dynarray_push_ptr` (stride 8, usado pra `DynArray<T*>`
/// cru — props/funcs/staticFuncs), este é o stride certo pra arrays de HANDLE reais do motor (ex.:
/// `ent::ComponentsStorage::components`). Cresce pelo `dynarray_push_bytes`.
pub(crate) unsafe fn dynarray_push_handle16(arr: *mut c_void, instance: u64, refcount: u64) -> Option<usize> {
    let mut elem = [0u8; 16];
    elem[..8].copy_from_slice(&instance.to_le_bytes());
    elem[8..].copy_from_slice(&refcount.to_le_bytes());
    dynarray_push_bytes(arr, &elem)
}

/// `ComputePlayerGarment` (2026-08-16, rodada de escrita): variante de **4 bytes**
/// (stride de `int32_t`, não 8/16) dos 2 irmãos acima — pra `DynArray<int32_t>` real do motor
/// (`aOffsets` de `ComputePlayerGarment`). Mesmo layout de header (`entries@0x00 u64, cap@0x08 u32,
/// size@0x0C u32`) e o mesmo crescimento (`dynarray_push_bytes`).
#[cfg(any(test, feature = "devtools"))]
pub(crate) unsafe fn dynarray_push_i32(arr: *mut c_void, val: i32) -> Option<usize> {
    dynarray_push_bytes(arr, &val.to_le_bytes())
}

/// `DynArray<T>::Clear()` — só reseta `size` (+0x0C) pra `0`, preserva `entries`/`cap` intactos
/// (o buffer fica alocado pra reuso — mesma semântica do `Clear()` real do motor, nunca desaloca;
/// mesma disciplina "nunca libera, só reseta o header" já aceita em todo `dynarray_push_*` acima).
/// Genérico por stride (só toca o header de 16 bytes, comum a QUALQUER `DynArray<T>`).
#[cfg(any(test, feature = "devtools"))]
pub(crate) unsafe fn dynarray_clear(arr: *mut c_void) -> bool {
    if !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return false;
    }
    core::ptr::write_unaligned((arr as *mut u8).add(0x0C) as *mut u32, 0u32);
    true
}

/// Registra um MÉTODO (instância/estático) numa CClass existente: PushBack no
/// `DynArray` `funcs@cls+0x48` (instância) ou `staticFuncs@cls+0x58` (estático).
/// V2: REALOCA se o array estiver cheio (via `dynarray_push_ptr`) — não falha mais
/// por falta de capacidade. É o caminho que mods com vários métodos na mesma classe exigem.
pub unsafe fn register_method(
    reg: &Registry,
    class: &str,
    proto_method: *mut c_void,
    full: &str,
    short: &str,
    handler: NativeHandler,
    is_static: bool,
) -> bool {
    let cls = reg.class_by_name(class);
    if !rtti::sane(cls) {
        crate::log_dev!("[reg] classe '{class}' não existe");
        return false;
    }
    let func = build_native_func(proto_method, full, short, handler, is_static);
    if func.is_null() {
        return false;
    }
    let arr = (cls as *mut u8).add(if is_static { 0x58 } else { 0x48 }) as *mut c_void;
    match dynarray_push_ptr(arr, func as u64) {
        Some(slot) => {
            crate::log_dev!("[reg] register_method {class}.{short} -> slot {slot}");
            true
        }
        None => {
            crate::log_dev!("[reg] register_method {class}.{short} FALHOU (array ilegível ou alloc null)");
            false
        }
    }
}

// ===== Trampolins de smoke-test =====================================================

/// Como `register_global`, mas o POD HERDA a assinatura (params) de `proto_params` (clona
/// `params@0x28` + `count@0x30`) → o bind redscript de `native func X(args)` casa, e `read_params`
/// lê os args. `proto_params` deve ter a MESMA assinatura (ex.: AbsF/Cos = `(Float)->Float`).
pub unsafe fn register_global_argful(
    reg: &Registry,
    proto_vtable: *mut c_void,
    proto_params: *mut c_void,
    full: &str,
    short: &str,
    handler: NativeHandler,
) -> bool {
    let func = build_native_func(proto_vtable, full, short, handler, true);
    if func.is_null() {
        return false;
    }
    if rtti::sane(proto_params) {
        let pe = rd_u64(proto_params as *const c_void, 0x28); // params (ptr)
        let pc = core::ptr::read_unaligned((proto_params as *const u8).add(0x30) as *const u32); // count
        wr_u64(func, 0x28, pe);
        wr_u32(func, 0x30, pc);
    }
    if !call_register_function(reg, func) {
        return false;
    }
    let back = get_function(reg, full);
    let ok = rtti::sane(back);
    crate::log_dev!(
        "[reg] register_global_argful {full}: re-resolve -> {} ({back:p})",
        if ok { "OK" } else { "FALHOU" }
    );
    ok
}

/// Compõe um array de params EMPRESTANDO `CProperty[idx]` de cada proto → (entries_ptr, count).
/// Permite assinaturas multi-arg de tipos arbitrários SEM GetType: ex. (CName,Float) =
/// `[(NameToString,0),(AbsF,0)]`. Buffer no pool do jogo (vaza pequeno; o RTTI só LÊ os params).
pub(crate) unsafe fn compose_params(specs: &[(*mut c_void, usize)]) -> (u64, u32) {
    let n = specs.len();
    let entries = rtti::pool_alloc(n * 8, 8) as *mut u64;
    if entries.is_null() {
        return (0, 0);
    }
    for (i, (proto, idx)) in specs.iter().enumerate() {
        let pe = rd_u64(*proto as *const c_void, 0x28) as *const u8; // params entries do proto
        let cprop = if pe.is_null() { 0 } else { rd_u64(pe.add(idx * 8) as *const c_void, 0) };
        entries.add(i).write_unaligned(cprop);
    }
    (entries as u64, n as u32)
}

/// Como `register_global_argful`, mas com params JÁ compostos (de `compose_params`).
pub unsafe fn register_global_composed(
    reg: &Registry,
    proto_vtable: *mut c_void,
    params: (u64, u32),
    full: &str,
    short: &str,
    handler: NativeHandler,
) -> bool {
    // Param sem tipo só vira veredito, não recusa: native declarada no .reds e não registrada dá SIGTRAP no bind
    // (boot 40, boot 267).
    if cfg!(feature = "devlog") && params_compostos(params) {
        let cprops: Vec<u64> = (0..params.1 as usize).map(|i| (params.0 as *const u64).add(i).read_unaligned()).collect();
        if let Some(i) = primeiro_param_sem_tipo(&cprops) {
            crate::veredito_dev!("[reg] {full}: parâmetro {i} sem tipo no RTTI — registrada mesmo assim (sem registro o bind cai)");
        }
    }
    let func = build_native_func(proto_vtable, full, short, handler, true);
    if func.is_null() {
        return false;
    }
    wr_u64(func, 0x28, params.0);
    wr_u32(func, 0x30, params.1);
    if !call_register_function(reg, func) {
        return false;
    }
    let back = get_function(reg, full);
    let ok = rtti::sane(back);
    crate::log_dev!(
        "[reg] register_global_composed {full}: re-resolve -> {} ({back:p})",
        if ok { "OK" } else { "FALHOU" }
    );
    ok
}

/// Registra uma native global COM ARGS a partir de NOMES DE TIPO (ex.: `["Float","CName"]`) —
/// compõe os params (`compose_params_from_types`) e registra (`register_global_composed`). É a via
/// que a API C-ABI expõe pros plugins (register_native_argful): o plugin não precisa de proto/params
/// crus, só dos nomes dos tipos. `proto_vtable` = vtable de uma global existente (Cos/Sin/...).
pub unsafe fn register_argful_by_types(
    reg: &Registry,
    proto_vtable: *mut c_void,
    type_names: &[&str],
    full: &str,
    short: &str,
    handler: NativeHandler,
) -> bool {
    let params = compose_params_from_types(reg, type_names);
    if !params_compostos(params) && !type_names.is_empty() {
        return false; // falha do pool
    }
    register_global_composed(reg, proto_vtable, params, full, short, handler)
}

/// GetType @ CRTTISystem vtbl+0x00 — resolve QUALQUER IType por nome (CName/Float/`handle:IScriptable`
/// ...). PROVADO seguro in-game (não é dtor, jogo sobrevive). Destrava params de native de QUALQUER
/// assinatura → arg Handle p/ dispatch arbitrário do CallbackSystem.
pub unsafe fn get_type(reg: &Registry, name: &str) -> *mut c_void {
    let slot = reg.vtbl_slot(0x00);
    if !rtti::sane(slot) {
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(slot);
    f(reg.raw(), cname(name))
}

/// Constrói um CProperty MÍNIMO (só `type@0x00`) p/ um IType — suficiente p/ read_params + bind do param.
pub(crate) unsafe fn build_min_cprop(itype: *mut c_void) -> *mut c_void {
    if itype.is_null() {
        return std::ptr::null_mut();
    }
    let cp = rtti::pool_alloc(0x30, 8);
    if cp.is_null() {
        return cp;
    }
    std::ptr::write_bytes(cp as *mut u8, 0, 0x30);
    wr_u64(cp, 0x00, itype as u64); // CProperty+0 = IType
    cp
}

/// Compõe params a partir de NOMES DE TIPO (via GetType + build_min_cprop) → (entries_ptr, count).
/// Permite QUALQUER assinatura sem precisar de proto pra clonar. Ex.: `["handle:IScriptable","CName"]`.
pub(crate) unsafe fn compose_params_from_types(reg: &Registry, type_names: &[&str]) -> (u64, u32) {
    compose_params_de_cprops(&type_names.iter().map(|tn| build_min_cprop(get_type(reg, tn)) as u64).collect::<Vec<_>>())
}

/// Parâmetros montados de `CProperty`s já prontas: as mínimas de [`build_min_cprop`] e as tiradas de uma função do jogo (o `Variant`, ver
/// [`cprop_variant`]).
pub(crate) unsafe fn compose_params_de_cprops(cprops: &[u64]) -> (u64, u32) {
    let entries = rtti::pool_alloc(cprops.len() * 8, 8) as *mut u64;
    if entries.is_null() {
        return (0, 0);
    }
    for (i, cp) in cprops.iter().enumerate() {
        entries.add(i).write_unaligned(*cp);
    }
    (entries as u64, cprops.len() as u32)
}

/// A montagem dos params deu certo: `compose_params`, `compose_params_from_types` e `compose_params_de_cprops` devolvem `(0, 0)` quando o pool
/// falha e `(entries, n)` senão. Conferir o `n` contra a contagem escrita à mão repete o array de tipos, e os dois divergindo a native deixa
/// de ser registrada calada. Com a lista de tipos vazia o nulo não prova falha (o pedido é de 0 bytes): quem decide é a lista.
pub(crate) fn params_compostos(params: (u64, u32)) -> bool {
    params.0 != 0
}

/// Índice do primeiro parâmetro sem tipo (IType ou `CProperty` nula). A contagem dos params não pega esse caso: o slot
/// nulo entra no `+0x30` como param válido.
pub(crate) fn primeiro_param_sem_tipo(tipos: &[u64]) -> Option<usize> {
    tipos.iter().position(|&t| t == 0)
}

#[cfg(test)]
mod testes_params_sem_tipo {
    use super::{build_min_cprop, primeiro_param_sem_tipo};

    /// `GetType` que não resolve vira `CProperty` nula, e a contagem de params a conta como param válido.
    #[test]
    fn tipo_nao_resolvido_vira_cprop_nula_e_e_apontado() {
        let nula = unsafe { build_min_cprop(std::ptr::null_mut()) } as u64;
        assert_eq!(nula, 0);
        assert_eq!(primeiro_param_sem_tipo(&[0x1000, nula, 0x2000]), Some(1));
    }

    #[test]
    fn aponta_o_primeiro_sem_tipo() {
        assert_eq!(primeiro_param_sem_tipo(&[0, 0x1000, 0]), Some(0));
    }

    #[test]
    fn todos_com_tipo_ou_sem_param_passa() {
        assert_eq!(primeiro_param_sem_tipo(&[0x1000, 0x2000]), None);
        assert_eq!(primeiro_param_sem_tipo(&[]), None);
    }
}

#[cfg(test)]
mod testes_params_compostos {
    use super::params_compostos;

    /// A falha do pool volta `(0, 0)`; o sucesso traz o buffer, seja qual for a contagem.
    #[test]
    fn falha_do_pool_nao_compoe() {
        assert!(!params_compostos((0, 0)));
        assert!(params_compostos((0x1000, 1)));
        assert!(params_compostos((0x1000, 5)));
    }
}

// ===== `CallbackSystemTarget`/`EntityTarget` — FILTRAGEM EM TEMPO DE
// DESPACHO (2026-08-12) ===========================================================================
// Achado-raiz (2026-08-11, 2 sessões consecutivas): as 12 factories static de
// Target (`ComponentTarget.ID/Name`, `EntityTarget.Type/RecordID/Template/Appearance/Definition`,
// etc.) já construíam+registravam com sucesso, e `AddTarget` já guardava a lista em
// `HandlerState.targets` (item `#4`, ✅ desde 2026-07-18) — mas `fire_event_args_by_hash` NUNCA lia
// essa lista: despachava pra TODO handler do mesmo nome de evento, ignorando o Target por completo.
// "a factory funciona" ≠ "o gap fecha" — a capacidade que faltava é ESTA seção.
//
// Escopo desta implementação (aditiva, ver regra de ouro no topo do arquivo/CLAUDE.md): só
// `EntityTarget` construído via `.RecordID(...)` (item `#134`, o mais rico dos 7 Targets, mas só
// esta 1 das 6 factories). Fica de fora, DE PROPÓSITO, por falta de offset/mecanismo confirmado com
// a mesma barra "zero risco de chute" já exigida no resto do projeto:
//   - `EntityTarget.Type/Template/Appearance` — comparariam contra `entity->GetType()`/
//     `templatePath`/`appearanceName`, campos de `Entity` nunca RE'ados neste projeto.
//   - `ComponentTarget`(`#132`) — compara `component->id`/
//     `component->name`; o offset de `id`(CRUID) dentro de `IComponent` nunca foi confirmado aqui
//     (catálogo item `#132`: "usa campo `component->id` nunca antes cruzado").
//   - `DynamicEntityTarget`/`StaticEntityTarget`(`#133`) — delegam pra `IsTagged(entityID, tag)`;
//     o registry Rust de tags (`dynent_tags()`) é indexado por `EntityID`(hash), mas o offset do
//     campo `Entity::entityID` (pra derivar o hash a partir do ponteiro vivo da entidade que a gente
//     TEM no evento) nunca foi confirmado — mesma categoria de risco recusada.
//   - `inkWidgetTarget`/`InputTarget`/`ResourceTarget`(`#135`/`#136`/`#137`) — eventos que não
//     carregam conceito de "entidade" no formato que esta seção resolve (`Input/Key`,
//     `Resource/Load`) — fora do escopo semântico desta rodada (targets desses tipos nunca são
//     verificados aqui — mesmo efeito de "sem filtro" que já existia, ver `entity_recordid_
//     targets_allow` abaixo: só bloqueia se houver >=1 `EntityTarget(RecordID)`).
//
// `Puppet::RecordID@+0x25C` no Mac (o header diz +0x260; setter 0x1020a0954 grava em +0x25C, 2026-09-13)/`Vehicle::RecordID@+0x638` (`Red::TweakDBID`, campo simples, NÃO
// injetado por mod), já cross-referenciados no catálogo (`#222`, ✅🔑). Nomes de classe RTTI resolvidos
// por leitura de `redscript-src` real (não chutados): `gamePuppetBase` (`core/gameplay/puppet.
// script:2`, ancestral confirmado de `PlayerPuppet`, NÃO `gamePuppet`). Pro veículo, o candidato é
// `vehicleBaseObject` — esse
// nome tem ZERO ocorrência em todo o corpus `redscript-src` (1764 arquivos) e nos símbolos
// demangled, então NÃO está confirmado que resolve neste build Mac (mesma categoria de achado já
// documentada no projeto pra `ToggleAuxLights`/`cpPlayerSystem`: nome do header Windows pode não
// bater 1:1 com o nome interno do binário). `categoria_e_record` (`entidades`) tenta `vehicleBaseObject`
// primeiro e cai pra `VehicleObject` (nome CONFIRMADO real via `core/gameplay/vehicles.script:
// 2`, ancestral de `WheeledObject`/`TankObject`); o `RecordID` de veículo passou pelo despacho no boot 182
// (prova do boot 182 em `proofs/`, linha `134e`).
//
// FAIL-OPEN é a regra central de segurança de toda esta seção: qualquer incerteza (target não é
// `EntityTarget`/critério não é RecordID/entidade não é Puppet nem Vehicle/leitura de memória fora
// de faixa) resolve pra "despacha mesmo assim" — o comportamento ORIGINAL (sem filtro nenhum) nunca
// piora, só um MATCH POSITIVO confirmado é que pode reduzir a entrega. Zero regressão possível pro
// caso já provado (handler sem target — `#1`, 7/7 fechado).

/// O `IsA` por ponteiro do produto: anda `cls+0x10` (campo `parent` de `CClass`, mesmo offset já usado em `register_type`/
/// `register_type_instantiable_with_parent`) até achar `target` ou esgotar a cadeia. Só lê memória, com `is_readable` e teto de profundidade
/// pra nunca travar num handle torto/cíclico, então serve em hook e thread de job. Classe que chega como nome do script resolve antes pelo
/// `Registry::script_class`; ponteiro fora de faixa (`CLASSE_NAO_RESOLVIDA`, nulo) nunca casa.
pub(crate) unsafe fn class_is_kind_of(candidate: *mut c_void, target: *mut c_void) -> bool {
    if !rtti::sane(candidate) || !rtti::sane(target) {
        return false;
    }
    let mut cur = candidate;
    let mut guard = 0;
    while !cur.is_null() && guard < 64 {
        if cur == target {
            return true;
        }
        if !crate::memoria::is_readable(cur as *const c_void, 0x18) {
            return false;
        }
        cur = rd_u64(cur, 0x10) as *mut c_void;
        guard += 1;
    }
    false
}

/// Lê um `u64` (TweakDBID/qualquer escalar de 8 bytes) num offset de instância, com guarda de
/// leitura — nunca crasha em offset/ponteiro fora de faixa (devolve `None`).
pub(crate) unsafe fn read_u64_field(ptr: *mut c_void, offset: usize) -> Option<u64> {
    crate::memoria::read_u64((ptr as usize).wrapping_add(offset))
}

/// `ComponentTarget::Matches` (2026-08-12) — lê `IComponent::name`(CName)@`+0x40`
/// e `IComponent::id`(CRUID)@`+0x60`, offsets da reflection REAL do jogo. Campos de DADO, sem shift Itanium.
/// `Red::CRUID{unk00: i64}`/`Red::CName{hash: u64}` são ambos scalars de 8 bytes (categoria já
/// tratada dezenas de vezes neste projeto). Devolve `None` se o ponteiro for nulo/ilegível.
pub(crate) unsafe fn component_id_name(component_ptr: *mut c_void) -> Option<(u64, u64)> {
    if component_ptr.is_null() || !crate::memoria::is_readable(component_ptr as *const c_void, 8) {
        return None;
    }
    let id = read_u64_field(component_ptr, 0x60)?;
    let name = read_u64_field(component_ptr, 0x40)?;
    Some((id, name))
}

// ===== Alvos do `CallbackSystemHandler` =====
// Cada família de `CallbackSystemTarget` suporta certos tipos de evento (`Supports`) e decide pelo dado do evento (`Matches`). O dado sai das
// tabelas de estado dos eventos do BWMS (entidade, builder travado, componente, tecla, spawn de widget); o alvo é reconhecido pela tabela da fábrica
// que o criou, nunca dereferenciando o objeto alvo.

/// `CallbackSystem.RegisterCallback(eventName, target, function, opt sticky) -> ref<CallbackSystemHandler>`
/// — FUNCIONAL (registry) + devolve **null** (ver ACHADO 2026-07-18 abaixo — NÃO é mais o mesmo
/// "deferido" de antes, é um bloqueador NOVO e mais fundo, isolado nesta mesma sessão).
///
/// **TENTATIVA 2026-07-18 (handler de callback, mesma sessão que fechou `GetService`):**
/// construir uma instância REAL via `rtti::new_object("CallbackSystemHandler")` e devolvê-la
/// (permitindo `RegisterCallback(...).AddTarget(...).SetLifetime(...)` encadeado de verdade) —
/// TESTADO AO VIVO, 2 tentativas, **AMBAS CRASHARAM** num ponto NOVO, diferente do crash de
/// 2026-07-15 (que já está fechado — forjar a classe e registrar os 6 métodos NÃO crasha mais,
/// confirmado nos 2 boots desta tentativa). O crash agora é DEPOIS: quando o REDSCRIPT COMPILADO
/// tenta LIBERAR (refcount release) o `ref<CallbackSystemHandler>` devolvido, ao sair de escopo.
/// RE ao vivo (crash-report `.ips`, 2 boots): a rotina de release que o compilador injeta no
/// teardown de todo frame com locais `ref<>` (achada em `0x101025dc8`, link vmaddr) lê
/// `[slot+0x08]` como um PONTEIRO PRO BLOCO DE REFCOUNT do handle e, se não-nulo, faz um
/// DECREMENTO ATÔMICO nele (`ldaddal`) — ou seja, **um valor `ref<T>` em bytecode compilado é uma
/// ESTRUTURA DE 16 BYTES** (`[+0x00]`=ponteiro do objeto, `[+0x08]`=ponteiro pro refcount block),
/// não um ponteiro cru de 8 bytes. 1ª tentativa (`write_uint_ret`, só 8 bytes) crashou com SIGBUS
/// (endereço desalinhado, lixo da stack em `[+0x08]`). 2ª tentativa (`write_handle_ret`, novo
/// helper que escreve `[+0x08]=0` EXPLICITAMENTE, esperando que o `cbz` inicial da rotina tratasse
/// como "sem refcount = no-op seguro") **AINDA CRASHOU**, mesma instrução, endereço de fault
/// ligeiramente diferente — ou seja, zerar `[+0x08]` NÃO foi suficiente; o real problema é mais
/// fundo: `rtti::new_object` provavelmente só constrói o OBJETO cru (`Construct()`/`CreateInstance`
/// via vtable), sem alocar+ligar um BLOCO DE REFCOUNT de verdade (o que uma API real tipo
/// `MakeHandle<T>()` faria) — então mesmo escrevendo 0 em `[+0x08]`, ALGO no caminho de
/// atribuição/cópia do valor de retorno (entre o `aOut` da native e o LOCAL redscript de fato)
/// deve estar lendo/copiando de outro lugar que ainda contém lixo, não investigado a fundo.
///
/// **Por que `CallbackSystem` (`BwmsGetCallbackSystem`) NUNCA bateu nisto:** é `extends IGameSystem` —
/// singleton do motor, hipótese forte (não confirmada) de que esse tipo de referência é
/// "weak"/não-contada (o motor é dono, script só empresta), então o compilador nem EMITE a
/// rotina de release pra locais desse tipo — diferente de `CallbackSystemHandler`
/// (`IScriptable` puro, refcount real). Continua via `write_handle_ret` (seguro, já provado).
///
/// **CRASH FECHADO 2026-07-18 (sessão `handle-ctor-re`, continuação direta):** achado o
/// construtor REAL de `Handle<T>` do motor — `rtti::make_handle`/`ADDR_HANDLE_CTOR`
/// (`0x101025c8c`, nota completa em `rtti.rs`), achado por vizinhança da rotina de release já
/// conhecida (`0x101025dc8`, a 0x13c bytes DEPOIS no mesmo objeto) e confirmado por 4045
/// call-sites reais no binário inteiro com o padrão exato esperado (`<constrói objeto> -> mov
/// x1,raw_ptr -> bl 0x101025c8c`). `RegisterCallback` agora constrói a instância de verdade
/// (`rtti::new_object`), registra o `HandlerState` (pra `IsRegistered`/`AddTarget`/etc.
/// funcionarem) e escreve um HANDLE REAL (refcount-block genuíno, não zerado) via
/// `rtti::make_handle` — o release emitido pelo compilador no teardown do `ref<>` agora acha um
/// bloco válido pra decrementar, em vez de lixo/null. PROVADO ao vivo, múltiplos boots até
/// GAMEPLAY REAL (t=84s), zero crash novo (mesma degradação/timing do controle sem o mod de
/// teste).
///
/// **Achado #2, SEPARADO — encadeamento numa ÚNICA expressão (`a().b().c()`) tem um limite do
/// COMPILADOR/VM, não da nossa fix:** testado ao vivo (mesmo dia): a forma `RegisterCallback(...)
/// .AddTarget(...).SetLifetime(...)` numa expressão só faz o VM PULAR `AddTarget`/`SetLifetime`
/// inteiramente (nenhum trampolim `tramp_csh_*` é sequer chamado — confirmado por log
/// incondicional ausente), como se o `Context` opcode (null-safe navigation — ver
/// `redscript/compiler/src/assembler.rs::assemble`, `Expr::MethodCall`) julgasse o retorno de
/// `RegisterCallback` como null. **Refutado que seja realmente null:** diagnóstico logo após
/// `make_handle` (abaixo) leu de volta os 16 bytes de `out` e confirmou AMBOS os campos
/// não-nulos e plausíveis (`[+0x00]`=ponteiro do objeto, `[+0x08]`=bloco de refcount real). A
/// forma SEPARADA (cada retorno guardado num `let` próprio, com `IsDefined` checado entre cada
/// passo — `AddTarget`/`SetLifetime`/`IsRegistered`/`HasTarget`/`Unregister` como STATEMENTS
/// distintos, não uma expressão encadeada) FUNCIONA 100%: todos os 6 métodos disparam de
/// verdade, com efeito colateral real e observável (`IsRegistered()->true`, `HasTarget()->true`,
/// `Unregister()` remove a entrada certa de `CALLBACKS`). Ou seja: o problema é ESPECÍFICO de
/// usar o retorno de uma função NATIVA diretamente como receiver de outra chamada NA MESMA
/// expressão (sem passar por um `assign` pra um local primeiro) — hipótese não confirmada:
/// o slot de stack que o `Context` lê pro receiver nesse caso pode não ser o MESMO `out` que a
/// nativa recebeu (overlap/timing entre a call-convention nativa e a starting-stack do próximo
/// `Context`). Não é mais RE necessária pro objetivo desta sessão (o crash — o motivo original de
/// handler de callback estar bloqueado — está genuinamente fechado); é um gap NOVO, mais
/// estreito, documentado honestamente em vez de escondido. Ver
/// `cp77-symbols/notes/proofs/2026-07-18-handle-ctor-re-*.log` pros logs completos dos 4 boots
/// desta sessão (1 anomalia não-reproduzida + 3 limpos, incl. o boot da forma separada).
// ===== ATIVAÇÃO PREGUIÇOSA DE HOOK.
//
// O mecanismo de um evento só precisa estar ativo quando algum mod se
// registra nele. O BWMS fazia o oposto em todo lugar: os call-sites de `lib.rs` CONSTROEM o
// argumento do evento — `make_entitylifecycleevent_arg`, `make_entitycomponentevent_arg`, objetos
// RTTI de verdade — e só então chamam `fire_event_args`, que descobre lá dentro que ninguém
// escuta e volta. O trabalho caro acontecia antes da pergunta barata.
//
// `tem_ouvinte()` é a pergunta barata, e vem da mesma tabela que conta registro/desregistro real
// do `CallbackSystem` (`CALLBACKS` e `STATIC_CALLBACKS`, por `has_listener_for`). A mesma pergunta
// decide os hooks: o registro sobe o grupo do evento em `ativar_hooks_do_evento`, e
// `desativar_hooks_sem_ouvinte` tira, com as outras threads paradas, o grupo que ficou sem ouvinte.
// Estado de instalação só existe na camada física (`ganchos_no_endereco` e, onde o grupo tem, o
// atômico dele: `HOOKS_DO_CICLO_NO_AR`, `C130C_HOOK_OK`, `GANCHO_DEPENDENCIAS_NO_AR`), gravado depois do
// resultado do patch.

/// Dono de um componente (`owner`, `+0x50` do `entIComponent`; o `vehicleController` herda o `gameComponent`).
pub(crate) unsafe fn dono_do_componente(componente: *mut c_void) -> usize {
    read_u64_field(componente, 0x50).unwrap_or(0) as usize
}

/// Forja a classe `CallbackSystem` (extends IGameSystem) + registra os 7 métodos nativos + o
/// getter estático `GameInstance.GetCallbackSystem()`. Mesmo fix de parent+fullName-bare que
/// fechou a Facade (`registrar_fachada_de_versao`), generalizado: aqui o parent é IGameSystem (não
/// IScriptable) porque o `.reds` real declara `extends IGameSystem` explícito, e a classe é
/// INSTANCIÁVEL (`register_type_instantiable_with_parent`, não `register_type_min`) porque
/// `GetCallbackSystem()` precisa devolver um objeto de verdade. Ver módulo `ponte-callbacksystem.reds`
/// pro escopo exato (funcional vs. stub) de cada método.
/// Resolve uma classe pelo MESMO caminho que o VALIDADOR de classe nativa usa internamente pra
/// checar "base declarada" (singleton em `0x1010b3288` + `vtbl+0x108`) — **diferente** de
/// `Registry::class_by_name` (que usa `vtbl+0x10` do singleton obtido por `Registry::obtain()`).
/// ACHADO AO VIVO (2026-07-13, CallbackSystem no RTTI): `IGameSystem` É uma classe nativa REAL,
/// já registrada pelo próprio motor (endereço estático, ex. `0x10c7c5260`) — mas
/// `class_by_name("IGameSystem")` retornava null (vtbl+0x10 não a via, por razão ainda não
/// mapeada — objeto/slot diferente, não falta de registro). Esta função usa o MESMO caminho que
/// o validador usa de verdade, evitando forjar (arriscado e desnecessário) uma classe que já existe.
pub(crate) unsafe fn resolve_class_via_validator_getclass(name: &str) -> *mut c_void {
    let getter: extern "C" fn() -> *mut c_void = std::mem::transmute(crate::rebase(0x1_010b_3288));
    let singleton = getter();
    if singleton.is_null() || !crate::memoria::is_readable(singleton as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    let vt = core::ptr::read_unaligned(singleton as *const *mut u8);
    if vt.is_null() || !crate::memoria::is_readable(vt.add(0x108) as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    let slot = core::ptr::read_unaligned(vt.add(0x108) as *const *mut c_void);
    if !rtti::sane(slot) {
        return std::ptr::null_mut();
    }
    let getclass: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(slot);
    getclass(singleton, cname(name))
}

// Classes de evento-alvo — TESTE ISOLADO enum-return, 2026-07-18, 4 rodadas, TESTADO E
// REVERTIDO. Conclusão final (ver nota completa em `ponte-callbacksystem.reds`): 3 de 4 boots
// com a mesma configuração essencial NÃO crasharam (incluindo uma repetição quase exata do
// boot que crashou) — a hipótese "retorno enum quebra o bind-pass" NÃO é confirmada de forma
// confiável; o crash isolado do Round 1 é provavelmente um flake ambiental, não determinístico.


/// Retém uma referência FORTE adicional em `obj`, permanentemente vazada (nunca liberada) — mesma
/// filosofia já usada pra `make_weak_handle`/`DecWeakRef` neste projeto ("nunca desaloca handle
/// forjado/emprestado"). Usar sempre que `obj` for guardado como `usize` cru num estado de vida
/// LONGA (ex. `ENTITYTARGET_STATES`/`HANDLER_STATES.targets`) — sem isso, o único dono forte é o
/// `ref<>` TEMPORÁRIO do lado redscript que construiu/passou o objeto, liberado logo após a
/// expressão retornar (idioma comum: `handler.AddTarget(EntityTarget.RecordID(id))`), deixando o
/// ponteiro guardado dangling. Causa-raiz EXATA do crash de heap corruption documentado
/// 2026-08-12 (`EntityTarget.RecordID`/`AddTarget`, ver `ENTITYTARGET_RECORDID_FILTER_ENABLED`).
/// `make_handle` (ADDR_HANDLE_CTOR) já é seguro de chamar 2x no mesmo objeto — 1ª chamada cria o
/// bloco de refcount (força=1), chamadas seguintes acham o bloco existente e só incrementam
/// (nunca duplicam) — não importa se esta função roda antes ou depois do `make_handle(out, obj)`
/// que constrói o handle de RETORNO.
pub(crate) unsafe fn retain_strong_ref_leaked(obj: *mut c_void) {
    if obj.is_null() {
        return;
    }
    let leaked: &'static mut [u8; 16] = Box::leak(Box::new([0u8; 16]));
    rtti::make_handle(leaked.as_mut_ptr() as *mut c_void, obj);
}

/// Classe `BwmsResource` vista na 1ª criação: o leitor de parâmetros compara o `+0x30` (tipo do `IScriptable`) de cada handle com ela antes de
/// desembrulhar. Num objeto que não é `IScriptable` o `+0x30` é outro campo e não bate.
pub(crate) static RECURSO_CLASSE: AtomicU64 = AtomicU64::new(0);

/// Parâmetro `handle` de native: dentro de um `BwmsResource`, o objeto do campo privado; senão, o próprio.
pub(crate) unsafe fn recurso_do_parametro(obj: u64) -> u64 {
    let cls = RECURSO_CLASSE.load(Ordering::Relaxed);
    if cls == 0 || !crate::memoria::is_readable((obj + 0x30) as *const c_void, 8) || ((obj + 0x30) as *const u64).read_unaligned() != cls {
        return obj;
    }
    recurso_do_script(obj)
}

/// Entrega `obj` ao script no retorno de uma native, como handle forte novo: `IScriptable` direto, o resto dentro de um `BwmsResource`.
pub(crate) unsafe fn entregar_recurso(out: *mut c_void, obj: *mut c_void) {
    let mut h = [0u64; 2];
    if !obj.is_null() {
        rtti::make_handle(h.as_mut_ptr() as *mut c_void, obj);
    }
    let mut s = recurso_para_script(h);
    crate::jogo::objetos::soltar_handle(&mut h);
    if out.is_null() {
        crate::jogo::objetos::soltar_handle(&mut s);
    } else {
        entregar_handle(out, s);
    }
}

/// `true` se `cls` é a classe de nome nativo `nome` ou filha dela: compara o nome em `CClass+0x18` a cada nível, sem chamar o RTTI do motor.
/// É a guarda de nome nativo fixo do código (`IScriptable`, `CResource`, `questNodeDefinition`...); nome que chega do script passa pelo
/// `Registry::script_class` e pelo `class_is_kind_of`, que aceitam o apelido de script.
pub(crate) unsafe fn classe_deriva_de_nome(mut cls: *mut c_void, nome: u64) -> bool {
    for _ in 0..64 {
        if cls.is_null() || !crate::memoria::is_readable(cls as *const c_void, 0x20) {
            return false;
        }
        if rtti::type_name_hash(cls) == nome {
            return true;
        }
        cls = core::ptr::read_unaligned((cls as *const u8).add(0x10) as *const *mut c_void);
    }
    false
}

/// Handle forte novo pro script a partir de `h` (que continua do chamador): objeto `IScriptable` vai direto; o resto vai dentro de um `BwmsResource` novo, com
/// uma cópia forte de `h` no campo privado. `[0, 0]` com objeto nulo ou sem a classe `BwmsResource`.
pub(crate) unsafe fn recurso_para_script(h: [u64; 2]) -> [u64; 2] {
    if h[0] == 0 {
        return [0, 0];
    }
    if rtti::derives_from_iscriptable(rtti::class_of(h[0] as *mut c_void)) {
        handle_strong_soma(&h, 1);
        return h;
    }
    let Some(reg) = rtti::Registry::obtain() else { return [0, 0] };
    let caixa = rtti::new_object(&reg, RECURSO_CLASSE_SCRIPT);
    if caixa.is_null() {
        return [0, 0];
    }
    RECURSO_CLASSE.store(rtti::class_of(caixa) as u64, Ordering::Relaxed);
    let mut saida = [0u64; 2];
    rtti::make_handle(saida.as_mut_ptr() as *mut c_void, caixa);
    let campo = rtti::resolve_prop_in_class(rtti::class_of(caixa), RECURSO_CAMPO_SCRIPT).map_or(std::ptr::null_mut(), |(voff, _, holder)| rtti::field_ptr(caixa, voff, holder));
    if campo.is_null() {
        crate::jogo::objetos::soltar_handle(&mut saida);
        return [0, 0];
    }
    core::ptr::write_unaligned(campo as *mut u64, h[0]);
    core::ptr::write_unaligned((campo as *mut u64).add(1), h[1]);
    handle_strong_soma(&h, 1);
    saida
}

/// Objeto que o script passou a uma native: dentro de um `BwmsResource`, o recurso do campo privado; senão, o próprio.
pub(crate) unsafe fn recurso_do_script(obj: u64) -> u64 {
    let cls = rtti::class_of(obj as *mut c_void);
    if !classe_deriva_de_nome(cls, crate::cname::cname(RECURSO_CLASSE_SCRIPT)) {
        return obj;
    }
    rtti::resolve_prop_in_class(cls, RECURSO_CAMPO_SCRIPT).map_or(0, |(voff, _, holder)| {
        let p = rtti::field_ptr(obj as *mut c_void, voff, holder) as *const u64;
        if p.is_null() { 0 } else { p.read_unaligned() }
    })
}

// ===== Wrappers sobre um `EntityBuilder` vivo (`Entity/Extract`) =====
// O `Entity/Extract` sai do hook do corpo do job de componentes do builder (`0x1019b74b0`, `selftest::job_do_builder_hook`), antes da original, com o builder
// travado pelo `WeakPtr`. Cada wrapper guarda (objeto, builder, bloco de referência do `WeakPtr`, aparência) e trava o builder de novo a cada leitura:
// com o builder morto, devolve vazio. Layout do header conferido no desmonte do corpo do job no Mac.
pub(crate) const EB_EXTRATOR: u64 = 0x10;
pub(crate) const EB_APARENCIA: u64 = 0x20;
pub(crate) const EB_APARENCIAS: u64 = 0x80;
pub(crate) const EB_TEMPLATE: u64 = 0x90;
pub(crate) const EB_REQUEST: u64 = 0xa0;
/// `EntityBuilderAppearance` (0x30 B): definição, recurso, extrator.
pub(crate) const EBA_DEFINICAO: u64 = 0x00;
pub(crate) const EBA_RECURSO: u64 = 0x10;
pub(crate) const EBA_EXTRATOR: u64 = 0x20;
pub(crate) const EBA_TAM: u64 = 0x30;
/// `EntityBuilderRequest` (herda o `EntityInitializeRequest`).
pub(crate) const EBR_ENTITY_ID: u64 = 0x20;
pub(crate) const EBR_APARENCIA: u64 = 0x28;
pub(crate) const EBR_PARAMS: u64 = 0x70;
pub(crate) const EBR_RECORD: u64 = 0xc0;
/// `ObjectPackageExtractor::results` (`DynArray<Handle<ISerializable>>`).
pub(crate) const EXT_RESULTADOS: u64 = 0xa8;
/// `CResource::path` e o `rootIndex` (i16) do cabeçalho de objetos do `EntityTemplate`.
pub(crate) const RES_CAMINHO: u64 = 0x30;
/// Nome da `AppearanceDefinition`.
pub(crate) const DEF_NOME: u64 = 0x30;

/// Roda `f(builder, aparência)` com o builder travado; `None` se o objeto não é wrapper de builder vivo ou o builder já morreu.
pub(crate) unsafe fn ebw_com_builder<T>(c: *mut c_void, f: impl FnOnce(u64, u64) -> T) -> Option<T> {
    let (builder, rc, aparencia) = EBW_VIVOS.lock().ok()?.iter().rev().find(|e| e.0 == c as usize).map(|e| (e.1, e.2, e.3))?;
    if !crate::jogo::objetos::travar_fraco(rc) {
        return None;
    }
    let r = f(builder, aparencia);
    crate::jogo::objetos::destravar_fraco(rc);
    Some(r)
}

/// `u64` em `base + off`, 0 se `base` é nulo ou ilegível.
pub(crate) unsafe fn eb_u64(base: u64, off: u64) -> u64 {
    if base == 0 {
        return 0;
    }
    crate::memoria::read_u64((base + off) as usize).unwrap_or(0)
}

/// Handles de um `DynArray<Handle<T>>` em `arr`, a partir do índice `de`.
pub(crate) unsafe fn eb_handles(arr: u64, de: usize) -> Vec<[u64; 2]> {
    let (entradas, tam) = (eb_u64(arr, 0), (eb_u64(arr, 0x08) >> 32) as usize);
    if entradas == 0 || tam > 100_000 || !crate::memoria::is_readable(entradas as *const c_void, tam * 16) {
        return Vec::new();
    }
    (de..tam).map(|i| [eb_u64(entradas, (i * 16) as u64), eb_u64(entradas, (i * 16 + 8) as u64)]).collect()
}

/// Raiz e componentes da extração do builder: o objeto do resultado no `rootIndex` do template e os handles depois dele.
/// `(0, [])` sem extrator, sem template, com `rootIndex` ilegível ou fora dos resultados (template sem entidade raiz grava -1).
pub(crate) unsafe fn eb_raiz_e_componentes(builder: u64) -> (u64, Vec<[u64; 2]>) {
    let (extrator, template) = (eb_u64(builder, EB_EXTRATOR), eb_u64(builder, EB_TEMPLATE));
    if extrator == 0 || template == 0 {
        return (0, Vec::new());
    }
    let mut bytes = [0u8; 2];
    if !crate::memoria::read_chunk((template + crate::corpo_e_rosto::TPL_ROOT_INDEX) as usize, &mut bytes) {
        return (0, Vec::new());
    }
    let resultados = extrator + EXT_RESULTADOS;
    let tam = (eb_u64(resultados, 0x08) >> 32) as usize;
    let raiz = match usize::try_from(i16::from_le_bytes(bytes)) {
        Ok(i) if i < tam => i,
        _ => return (0, Vec::new()),
    };
    // A ordem dos resultados é a do pacote do template: o que vem depois da raiz são os componentes dela.
    let entradas = eb_u64(resultados, 0);
    (eb_u64(entradas, (raiz * 16) as u64), eb_handles(resultados, raiz + 1))
}

/// Grava em `out` um `array<ref<T>>` novo com `handles`: buffer do pool do jogo e, depois dele, o alocador copiado do trailer de `fonte` (um `DynArray` do
/// motor com elemento de `passo` B) — a técnica do `BwmsEntityGetComponents`.
pub(crate) unsafe fn eb_array_ret(out: *mut c_void, handles: &[[u64; 2]], fonte: u64, passo: u64) {
    let (entradas, cap) = (eb_u64(fonte, 0), eb_u64(fonte, 0x08) as u32 as u64);
    let trailer = if entradas != 0 && cap > 0 { eb_u64((entradas + cap * passo + 7) & !7, 0) } else { 0 };
    let mut buf: *mut u8 = std::ptr::null_mut();
    if !handles.is_empty() {
        buf = rtti::pool_alloc(handles.len() * 16 + 8, 8) as *mut u8;
        if buf.is_null() {
            core::ptr::write_bytes(out as *mut u8, 0, 16);
            return;
        }
        for (i, h) in handles.iter().enumerate() {
            core::ptr::write_unaligned(buf.add(i * 16) as *mut u64, h[0]);
            core::ptr::write_unaligned(buf.add(i * 16 + 8) as *mut u64, h[1]);
        }
        core::ptr::write_unaligned(buf.add(handles.len() * 16) as *mut u64, trailer);
    }
    core::ptr::write_unaligned(out as *mut u64, buf as u64);
    core::ptr::write_unaligned((out as *mut u8).add(8) as *mut u32, handles.len() as u32);
    core::ptr::write_unaligned((out as *mut u8).add(12) as *mut u32, handles.len() as u32);
}

/// Handle forte de um componente recebido pelo script, empurrado nos resultados de um extrator.
pub(crate) unsafe fn eb_adicionar_componente(frame: *mut c_void, extrator: u64) -> bool {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let componente = args.first().map_or(std::ptr::null_mut(), |(v, _)| *v as *mut c_void);
    if extrator == 0 || !rtti::sane(componente) {
        return false;
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, componente);
    let (inst, refc) = ((buf.as_ptr() as *const u64).read_unaligned(), (buf.as_ptr().add(8) as *const u64).read_unaligned());
    dynarray_push_handle16((extrator + EXT_RESULTADOS) as *mut c_void, inst, refc).is_some()
}

/// Componentes como `array<ref<IComponent>>` sem referência nossa (os objetos são do motor), com o trailer do array de resultados.
pub(crate) unsafe fn eb_componentes_ret(out: *mut c_void, handles: &[[u64; 2]], resultados: u64) {
    let crus: Vec<[u64; 2]> = handles.iter().filter(|h| rtti::sane(h[0] as *mut c_void)).map(|h| [h[0], 0]).collect();
    eb_array_ret(out, &crus, resultados, 16);
}

/// Guard de registro único (idempotente). Resetado p/ retry se o RTTI ainda não está pronto.
static REGISTERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Registra TODAS as nativas do BWMS no RTTI, 1x. Chamado CEDO (ponte do bind em `inicio::boot` + cp77_tick) p/ a
/// nativa existir ANTES do bind do script (o redscript resolve `native func` por nome no load). Se o RTTI
/// ainda não estiver pronto, reseta o guard e tenta de novo na próxima chamada.
pub unsafe fn register_all() {
    use std::sync::atomic::Ordering;
    // Fast-path BARATO p/ o hot-path do executor: já registrado → 1 load e sai.
    if REGISTERED.load(Ordering::Relaxed) {
        return;
    }
    if REGISTERED.swap(true, Ordering::Relaxed) {
        return;
    }
    let reg = match rtti::Registry::obtain() {
        Some(r) => r,
        None => {
            REGISTERED.store(false, Ordering::Relaxed);
            return;
        }
    };
    let mut proto = std::ptr::null_mut();
    for n in ["Cos", "Sin", "AbsF", "SqrtF", "LogF"] {
        let p = get_function(&reg, n);
        if rtti::sane(p) {
            proto = p;
            break;
        }
    }
    if !rtti::sane(proto) {
        crate::log_dev!("[reg] register_all: sem protótipo global (Cos/Sin/...) — adiado");
        REGISTERED.store(false, Ordering::Relaxed);
        return;
    }
    let ok = registro_de_prova!(register_global(&reg, proto, "BlackwallPing", "BlackwallPing", corpo_de_prova!(tramp_ping)));
    crate::log_dev!(
        "[reg] register_all: BlackwallPing registrado={ok} (ponte redscript→native, F-B)"
    );
    // SEGUNDA native real — prova que register_all escala (N funções no RTTI) e que o
    // retorno Bool native→redscript é consumido numa condicional do .reds.
    let ok2 = register_global(&reg, proto, "BwmsAutoContinue", "BwmsAutoContinue", tramp_autocontinue);
    let _ = register_global(&reg, proto, "BwmsAutoContinueDoBoot", "BwmsAutoContinueDoBoot", tramp_autocontinue_do_boot);
    // Runtime systems da cena, chamáveis do redscript — sem depender do canal (que exige
    // `dev_mode`, o modo com histórico de derrubar boot).
    let _ = registro_de_prova!(register_global(&reg, proto, "BwmsDumpRuntimeSystems", "BwmsDumpRuntimeSystems", corpo_de_prova!(tramp_dump_runtime_systems)));
    let oka1 = register_global(&reg, proto, "BwmsAutoContinueOn", "BwmsAutoContinueOn", tramp_autocontinue_on);
    let oka2 = register_global(&reg, proto, "BwmsAutoContinueOff", "BwmsAutoContinueOff", tramp_autocontinue_off);
    let _ = register_global(&reg, proto, "BwmsAutoContinueSaveIndex", "BwmsAutoContinueSaveIndex", tramp_autocontinue_saveindex);
    let _ = register_global(&reg, proto, "BwmsFireStartOn", "BwmsFireStartOn", tramp_fire_start_on);
    let _ = register_global(&reg, proto, "BwmsFireStartOff", "BwmsFireStartOff", tramp_fire_start_off);
    let okfss = register_global(&reg, proto, "BwmsFireStartState", "BwmsFireStartState", tramp_fire_start_state);
    crate::log_dev!("[reg] register_all: BwmsFireStartState={okfss} (níveis 1+2 do seletor usam o lever)");
    let _ = register_global(&reg, proto, "BwmsAcFired", "BwmsAcFired", tramp_ac_fired);
    let _ = register_global(&reg, proto, "BwmsMenuReadySplashOff", "BwmsMenuReadySplashOff", tramp_menu_ready_splash_off);
    // Estado do puppet: gatilho sem world-nav (ver comentário da trampolim). Só o `bwms-puppetstate-mirror.reds` do dev declara.
    #[cfg(feature = "devtools")]
    {
        let _ = register_global(&reg, proto, "BwmsPuppetStateFireOnce", "BwmsPuppetStateFireOnce", tramp_puppetstate_fire_once);
        let _ = register_global(&reg, proto, "BwmsPuppetStateShouldRecover", "BwmsPuppetStateShouldRecover", tramp_puppetstate_should_recover);
    }
    // Skill 1 (VER-O-V, câmera 3ª pessoa autônoma) — lê ~/.bwms-tppcam. Ver bwms-tppcam.reds.
    let oktpp = registro_de_prova!(register_global(&reg, proto, "BwmsTppState", "BwmsTppState", corpo_de_prova!(tramp_tpp_state)));
    // Mod FULL BODY (ver o próprio corpo em 1ª pessoa, offset da câmera tunável) — eixos X/Y/Z em cm. Só o `bwms-tppcam.reds` do dev declara, e o
    // marcador `~/.bwms-cam*` que elas leem não existe no público (`caminho_de_marcador`).
    #[cfg(feature = "devtools")]
    {
        let _okcb = register_global(&reg, proto, "BwmsCamBack", "BwmsCamBack", tramp_cam_back);
        let _okcx = register_global(&reg, proto, "BwmsCamX", "BwmsCamX", tramp_cam_x);
        let _okcz = register_global(&reg, proto, "BwmsCamZ", "BwmsCamZ", tramp_cam_z);
    }
    let _okfl = registro_de_prova!(register_global(&reg, proto, "BwmsForceLook", "BwmsForceLook", corpo_de_prova!(tramp_force_look)));
    // Skill 2 (EQUIPAR roupa por código, fazer aparecer no V) — lê ~/.bwms-equip. Ver bwms-tppcam.reds.
    let okeq = registro_de_prova!(register_global(&reg, proto, "BwmsEquipState", "BwmsEquipState", corpo_de_prova!(tramp_equip_state)));
    let _okel = registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32"], "BwmsEquipLog", "BwmsEquipLog", corpo_de_prova!(tramp_equip_log)));
    let okinv = registro_de_prova!(register_global(&reg, proto, "BwmsInvState", "BwmsInvState", corpo_de_prova!(tramp_inv_state)));
    let _okec = registro_de_prova!(register_global(&reg, proto, "BwmsEquipCheck", "BwmsEquipCheck", corpo_de_prova!(tramp_equip_check)));
    let _okrb = registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32", "TweakDBID"], "BwmsEquipReadback", "BwmsEquipReadback", corpo_de_prova!(tramp_equip_readback)));
    crate::log_dev!("[reg] register_all: BwmsTppState={oktpp} BwmsEquipState={okeq} BwmsInvState={okinv} (Skills 1+2)");
    let _ = registro_de_prova!(register_global(&reg, proto, "BwmsDbgMeta0", "BwmsDbgMeta0", corpo_de_prova!(tramp_dbg_meta0)));
    let _ = registro_de_prova!(register_global(&reg, proto, "BwmsDbgHasT", "BwmsDbgHasT", corpo_de_prova!(tramp_dbg_hast)));
    let _ = registro_de_prova!(register_global(&reg, proto, "BwmsDbgHasF", "BwmsDbgHasF", corpo_de_prova!(tramp_dbg_hasf)));
    let _okds = registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32"], "BwmsDbgSkip", "BwmsDbgSkip", corpo_de_prova!(tramp_dbg_skip)));
    crate::log_dev!(
        "[reg] register_all: BwmsAutoContinue read={ok2} On={oka1} Off={oka2} (toggle pular-até-o-jogo)"
    );
    // IA Fase 0: ponte game->processo-externo (enfileira evento, non-blocking). Corpo só no dev: ele escreve em
    // `/tmp/bwms-ai` e nenhum `.reds` do pacote declara a native.
    let ok3 = registro_de_prova!(register_global(&reg, proto, "BwmsEmit", "BwmsEmit", corpo_de_prova!(tramp_bwms_emit)));
    crate::log_dev!("[reg] register_all: BwmsEmit registrado={ok3} (IA Fase 0)");
    // Skip-intro: sinal preciso da engagement do boot vindo do redscript (leitores em
    // `terminal::overlay::engagement_active`).
    let oke1 = register_global(&reg, proto, "BwmsEngagementOn", "BwmsEngagementOn", tramp_engagement_on);
    let oke2 = register_global(&reg, proto, "BwmsEngagementOff", "BwmsEngagementOff", tramp_engagement_off);
    // LEVER da ação 'Start' (gate `pular_intro`, ligado por padrão). Escreve [SM+0xd4]=2 (`tramp_fire_start`). 0-arg -> Bool.
    let okfs = register_global(&reg, proto, "BwmsFireStart", "BwmsFireStart", tramp_fire_start);
    crate::log_dev!("[reg] register_all: BwmsFireStart registrado={okfs} (injeta a acao Start via ctx+0x572, gated)");
    crate::log_dev!("[reg] register_all: BwmsEngagementOn={oke1} Off={oke2} (skip-intro)");
    // Toggle do skip-intro pela UI (persiste no marcador ~/.bwms-skipintro).
    let oks1 = register_global(&reg, proto, "BwmsSkipIntroOn", "BwmsSkipIntroOn", tramp_skipintro_on);
    let oks2 = register_global(&reg, proto, "BwmsSkipIntroOff", "BwmsSkipIntroOff", tramp_skipintro_off);
    let oks3 = register_global(&reg, proto, "BwmsSkipIntroState", "BwmsSkipIntroState", tramp_skipintro_state);
    crate::log_dev!("[reg] register_all: BwmsSkipIntro On={oks1} Off={oks2} State={oks3} (toggle UI)");
    // Toggle do CPVR (VR) — DEV-ONLY (feature `cpvr`); marcador ~/.bwms-cpvr lido pelos reds do CPVR.
    #[cfg(feature = "cpvr")]
    {
        let okv1 = register_global(&reg, proto, "BwmsCpvrOn", "BwmsCpvrOn", tramp_cpvr_on);
        let okv2 = register_global(&reg, proto, "BwmsCpvrOff", "BwmsCpvrOff", tramp_cpvr_off);
        let okv3 = register_global(&reg, proto, "BwmsCpvrState", "BwmsCpvrState", tramp_cpvr_state);
        crate::log_dev!("[reg] register_all: BwmsCpvr On={okv1} Off={okv2} State={okv3} (toggle VR, dev)");
        let okm1 = register_global(&reg, proto, "BwmsCpvrMode", "BwmsCpvrMode", tramp_cpvr_mode);
        let okm2 = register_global(&reg, proto, "BwmsCpvrModeNext", "BwmsCpvrModeNext", tramp_cpvr_mode_next);
        let okm3 = register_global(&reg, proto, "BwmsCpvrModePrev", "BwmsCpvrModePrev", tramp_cpvr_mode_prev);
        crate::log_dev!("[reg] register_all: BwmsCpvrMode={okm1} Next={okm2} Prev={okm3} (seletor VR, dev)");
        // As mãos do headset, pro redscript mirar e atirar. Uma native por número: forjar struct
        // nova só pra isto custaria mais do que quatro chamadas baratas.
        let okh1 = register_argful_by_types(&reg, proto, &["Bool"], "BwmsVrMaoPresente", "BwmsVrMaoPresente", tramp_vr_mao_presente);
        let okh2 = register_argful_by_types(&reg, proto, &["Bool", "Int32"], "BwmsVrMaoEixo", "BwmsVrMaoEixo", tramp_vr_mao_eixo);
        let okh3 = register_argful_by_types(&reg, proto, &["Bool"], "BwmsVrMaoBotoes", "BwmsVrMaoBotoes", tramp_vr_mao_botoes);
        let okh4 = register_global(&reg, proto, "BwmsVrLeituras", "BwmsVrLeituras", tramp_vr_leituras);
        crate::log_dev!("[reg] register_all: mãos do VR Presente={okh1} Eixo={okh2} Botoes={okh3} Leituras={okh4}");
        // A cabeça, pelo mesmo caminho: o script gira a câmera com `TargetingSystem.LookAt`, que é a
        // única rota de script que o jogo trata como input de câmera.
        let okc1 = register_argful_by_types(&reg, proto, &["Int32"], "BwmsVrCabecaEixo", "BwmsVrCabecaEixo", tramp_vr_cabeca_eixo);
        let okc2 = register_global(&reg, proto, "BwmsVrCabecaSeq", "BwmsVrCabecaSeq", tramp_vr_cabeca_seq);
        let okg1 = register_argful_by_types(&reg, proto, &["Bool", "Int32"], "BwmsVrGatilho", "BwmsVrGatilho", tramp_vr_gatilho);
        crate::log_dev!("[reg] register_all: cabeça do VR Eixo={okc1} Seq={okc2} · gatilho={okg1}");
        let okmp = register_global(&reg, proto, "BwmsCpvrStereoPing", "BwmsCpvrStereoPing", tramp_cpvr_stereo_ping);
        let okap = register_global(&reg, proto, "BwmsCpvrApplyPing", "BwmsCpvrApplyPing", tramp_cpvr_apply_ping);
        let okv2 = register_global(&reg, proto, "BwmsCpvrIsV2", "BwmsCpvrIsV2", tramp_cpvr_is_v2);
        crate::log_dev!("[reg] register_all: BwmsCpvrStereoPing={okmp} ApplyPing={okap} IsV2={okv2} (diag v2.0)");
        // MECANISMO 7: BwmsCamScan(x,y,z: Float) -> Bool. Scanner nativo de memoria, nosso.
        let okcs = registro_de_prova!(register_argful_by_types(&reg, proto, &["Float", "Float", "Float"], "BwmsCamScan", "BwmsCamScan", corpo_de_prova!(tramp_camscan)));
        crate::log_dev!("[reg] register_all: BwmsCamScan registrado={okcs} (scanner nativo da camera, mec 7)");
    }
    // FOUNDATIONAL: native que LÊ arg do redscript. proto (Cos/AbsF/...) é (Float)->Float → herda
    // a assinatura (1 Float). Destrava arg-natives (CallbackSystem dispatch, Reflection p/ redscript).
    let ok4 = registro_de_prova!(register_global_argful(&reg, proto, proto, "BwmsEchoF", "BwmsEchoF", corpo_de_prova!(tramp_echo)));
    crate::log_dev!("[reg] register_all: BwmsEchoF registrado={ok4} (arg-reading foundational)");
    // ISOLADO (2026-08-15, rodada 10): BwmsVector4RoundTrip(v: Vector4) -> Vector4 — teste dedicado
    // de struct-de-16-bytes como PARÂMETRO+RETORNO, zero dependência de #76/StaticEntitySpec (ver
    // tramp_vector4_roundtrip acima).
    let params_v4 = compose_params_from_types(&reg, &["Vector4"]);
    if params_compostos(params_v4) {
        let okv4 = registro_de_prova!(register_global_composed(&reg, proto, params_v4, "BwmsVector4RoundTrip", "BwmsVector4RoundTrip", corpo_de_prova!(tramp_vector4_roundtrip)));
        crate::log_dev!("[reg] register_all: BwmsVector4RoundTrip registrado={okv4} (struct-16B-como-parametro, teste ISOLADO pro #76)");
    } else {
        crate::log_dev!("[reg] register_all: BwmsVector4RoundTrip — compose_params_from_types('Vector4') FALHOU (tipo não resolveu)");
    }
    // Reflection GETF pro redscript: BwmsGetPlayerField(CName)->Float. proto de params = um global
    // com 1 param CName (NameToString/IsNameValid), disponível cedo.
    let proto_cn = ["NameToString", "IsNameValid", "StringToName"]
        .iter()
        .find_map(|n| {
            let p = get_function(&reg, n);
            if rtti::sane(p) { Some(p) } else { None }
        });
    if let Some(pc) = proto_cn {
        let ok5 = register_global_argful(&reg, proto, pc, "BwmsGetPlayerField", "BwmsGetPlayerField", tramp_getfield);
        crate::log_dev!("[reg] register_all: BwmsGetPlayerField registrado={ok5} (Reflection getf pro redscript)");
        // setf pro redscript: BwmsSetPlayerField(CName, Float) -> Bool. params compostos (CName + Float).
        let params2 = compose_params(&[(pc, 0), (proto, 0)]);
        if params_compostos(params2) {
            let ok6 = register_global_composed(&reg, proto, params2, "BwmsSetPlayerField", "BwmsSetPlayerField", tramp_setfield);
            crate::log_dev!("[reg] register_all: BwmsSetPlayerField registrado={ok6} (Reflection setf pro redscript, params compostos)");
        }
        // callf pro redscript: BwmsCallPlayerMethod(CName)->Bool. Completa get/set/CALL pro redscript.
        let ok7 = register_global_argful(&reg, proto, pc, "BwmsCallPlayerMethod", "BwmsCallPlayerMethod", tramp_callplayer);
        crate::log_dev!("[reg] register_all: BwmsCallPlayerMethod registrado={ok7} (Reflection callf pro redscript)");
    } else {
        crate::log_dev!("[reg] register_all: sem proto CName p/ BwmsGetPlayerField");
    }
    // CallbackSystem DISPATCH (núcleo): BwmsCallMethod(ref<IScriptable>, CName)->Bool. Params via
    // GetType (provado) → assinatura (Handle, CName) sem precisar clonar proto. Despacha p/ qualquer alvo.
    let params_hc = compose_params_from_types(&reg, &["handle:IScriptable", "CName"]);
    if params_compostos(params_hc) {
        let ok8 = register_global_composed(&reg, proto, params_hc, "BwmsCallMethod", "BwmsCallMethod", tramp_callmethod);
        crate::log_dev!("[reg] register_all: BwmsCallMethod registrado={ok8} (dispatch arbitrário via GetType — CallbackSystem core)");
    }
    // Garment/transmog (2026-08-02, /goal): BwmsQueueRequestRaw(sys, req)->Bool —
    // chama EquipmentSystem::QueueRequest (0x102445e8c, RE dedicada desta sessão) DIRETO via transmute,
    // bypass total do RTTI/GetInvokable() quebrado. Mesma composição de params de BwmsCallMethod.
    let params_qr = compose_params_from_types(&reg, &["handle:IScriptable", "handle:IScriptable"]);
    if params_compostos(params_qr) {
        let ok10 = registro_de_prova!(register_global_composed(&reg, proto, params_qr, "BwmsQueueRequestRaw", "BwmsQueueRequestRaw", corpo_de_prova!(tramp_queue_request_raw)));
        crate::log_dev!("[reg] register_all: BwmsQueueRequestRaw registrado={ok10} (QueueRequest raw @ 0x102445e8c, bypass GetInvokable)");
    }
    // Garment/transmog (2026-08-02, /goal): BwmsQueueRequestCtx(sys, req)->Bool —
    // 3ª via, chama QueueRequest via call_func NOSSO com ctx explícito (mesmo padrão do `give`), em vez
    // do endereço cru. Mesma composição de params das 2 anteriores.
    let params_qc = compose_params_from_types(&reg, &["handle:IScriptable", "handle:IScriptable"]);
    if params_compostos(params_qc) {
        let ok11 = registro_de_prova!(register_global_composed(&reg, proto, params_qc, "BwmsQueueRequestCtx", "BwmsQueueRequestCtx", corpo_de_prova!(tramp_queue_request_ctx)));
        crate::log_dev!("[reg] register_all: BwmsQueueRequestCtx registrado={ok11} (QueueRequest via call_func, ctx explícito)");
    }
    // Item 213 (2026-08-18): auto-instala o fix cirúrgico de `GetInvokable()` pra
    // `EquipmentSystem::QueueRequest` JÁ NO BOOT — mesmo mecanismo provado extensivamente desde
    // 2026-08-03 (`install_getinvokable_fix`, `selftest.rs`), até agora só disparado sob-demanda
    // pelos comandos de canal (`equiprawv7`/`equipcyber`/`equipglasses`/`equipoutfit`/`equiprawv6`/
    // `transmogviaquery`). Instalar cedo (aqui, register_all, muito antes de qualquer
    // OnGameAttached) permite que `BwmsEquipViaQuery`/`BwmsForceEquipRaw`/etc. sejam chamados
    // DIRETO de dentro de redscript (ex. num @wrapMethod automático), sem precisar de nenhuma
    // interação manual via canal — necessário pro item #213 (VisualController) forçar um equip
    // antes de ler os campos VCC, já que o canal interativo é documentado como não-confiável
    // pós-autocontinue. Fix ESCOPADO (só redireciona o descritor exato de QueueRequest — "zero
    // regressão pros outros ~3 usos dessa vtable slot", já confirmado nos meses de uso via
    // comando) — instalar mais cedo não muda o comportamento, só o timing.
    if let Some(rf_qr_boot) = rtti::resolve_func(&reg, "EquipmentSystem", "QueueRequest") {
        let fix_ok = crate::roupas_e_armas::install_getinvokable_fix(rf_qr_boot.func, 0x1_0244_5e8cu64);
        crate::log_dev!("[reg] register_all: fix de GetInvokable p/ QueueRequest auto-instalado no boot={fix_ok} (cw-213)");
    } else {
        crate::log_dev!("[reg] register_all: resolve_func(EquipmentSystem, QueueRequest) falhou no boot — fix de GetInvokable NAO instalado (cw-213)");
    }
    // Garment/transmog (2026-08-03, /goal): 4ª via — BwmsCaptureEs/Req(ptr)->Void,
    // 1 arg handle cada (mesmo shape seguro de BwmsCallMethod), só escrevem num atomic. Usadas
    // ANINHADAS de dentro de 1 única call_func top-level (BwmsPrepAndCapture), evitando os 2 bugs
    // já diagnosticados. Ver tramp_capture_es/req acima.
    let params_1h_es = compose_params_from_types(&reg, &["handle:IScriptable"]);
    if params_compostos(params_1h_es) {
        let okc1 = registro_de_prova!(register_global_composed(&reg, proto, params_1h_es, "BwmsCaptureEs", "BwmsCaptureEs", corpo_de_prova!(tramp_capture_es)));
        crate::log_dev!("[reg] register_all: BwmsCaptureEs registrado={okc1}");
    }
    let params_1h_req = compose_params_from_types(&reg, &["handle:IScriptable"]);
    if params_compostos(params_1h_req) {
        let okc2 = registro_de_prova!(register_global_composed(&reg, proto, params_1h_req, "BwmsCaptureReq", "BwmsCaptureReq", corpo_de_prova!(tramp_capture_req)));
        crate::log_dev!("[reg] register_all: BwmsCaptureReq registrado={okc2}");
    }
    // READ-ONLY: dump da state-machine da engagement (validação da LEVER C, sem escrita). 1-arg handle.
    let params_h = compose_params_from_types(&reg, &["handle:IScriptable"]);
    if params_compostos(params_h) {
        let oke = registro_de_prova!(register_global_composed(&reg, proto, params_h, "BwmsEngDump", "BwmsEngDump", corpo_de_prova!(tramp_engdump)));
        crate::log_dev!("[reg] register_all: BwmsEngDump registrado={oke} (dump read-only do SM da engagement)");
    }
    // CallbackSystem RegisterCallback(CName, ref<IScriptable>, CName)->Bool (3-arg via GetType).
    let params_ccc = compose_params_from_types(&reg, &["CName", "handle:IScriptable", "CName"]);
    if params_compostos(params_ccc) {
        let ok9 = register_global_composed(&reg, proto, params_ccc, "BwmsRegisterCallback", "BwmsRegisterCallback", tramp_register_callback);
        crate::log_dev!("[reg] register_all: BwmsRegisterCallback registrado={ok9} (CallbackSystem RegisterCallback)");
    }
    // `CustomizationExtension` (2026-08-14): BwmsCustomizationEdge(kind: Int32)
    // -> Void — chamado pelos 3 wraps em `characterCreationBodyMorphMenu`, só marca um edge flag
    // (drenado por `drain_customization_edges()`, chamado de `cp77_tick`).
    let params_i32 = compose_params_from_types(&reg, &["Int32"]);
    if params_compostos(params_i32) {
        let ok12 = register_global_composed(&reg, proto, params_i32, "BwmsCustomizationEdge", "BwmsCustomizationEdge", tramp_customization_edge as NativeHandler);
        crate::log_dev!("[reg] register_all: BwmsCustomizationEdge registrado={ok12} (CustomizationExtension, edge flag)");
    }
    // Prova do register_method com REALLOC (gated ~/.bwms-regmethod-test).
    regmethod_selftest(&reg);
    // Utilitários de script (Bits/Hash/Number/String/CName/CRUID/NodeRef/Logging, ~30+ entradas).
    register_bits_utils(&reg, proto);
    register_hash_utils(&reg, proto);
    register_number_utils(&reg, proto);
    register_string_utils(&reg, proto);
    register_entidades_persistentes(&reg, proto);
    register_hash_wrapper_utils(&reg, proto);
    register_compatibility_utils(&reg, proto);
    crate::log_dev!("[reg] register_all: cw-utils registrado");
    // `inkSystem.GetClipboardText`/`SetClipboardText` (2026-08-11, 4ª rodada de
    // mineração).
    register_ink_clipboard_utils(&reg, proto);
    // Partes 7a/7b (2026-09-12): natives do teste da busca de mappin cozido.
    register_mappin_teste_natives(&reg, proto);
    // Parte 100-4 (2026-09-12): controllers dos world widgets lidos do `inkWorldLayer`.
    register_ink_world_widget_natives(&reg, proto);
    register_quest_accessor_natives(&reg, proto);
    // 2026-09-18: `WorldStateSystem` — `IsReady`, `GetStreamingWorld`, `ToggleNode`/`ToggleVariant` e o `Update` da comunidade.
    register_worldstate_natives(&reg, proto);
    // Parte 100-2 (2026-09-12): foco do inkSystem lido em `InkSystem+0x2E8`.
    let ok = registro_de_prova!(register_argful_by_types(&reg, proto, &["handle:IScriptable"], "BwmsInkSystemFocusIs", "BwmsInkSystemFocusIs", corpo_de_prova!(tramp_inksystem_focus_is)));
    crate::log_dev!("[reg] register_all: cw-100 100-2 FocusIs={ok}");
    // `InkSystem` GetLayers/GetLayer/GetWorldWidgets (2026-08-14);
    // usa `ink_system()` (lib.rs, cacheado).
    register_inksystem_layers_utils(&reg, proto);
    // `GetWorldWidgets` diagnóstico observe-only (2026-08-16).
    register_inksystem_worldwidgets_diag(&reg, proto);
    // `inkLayerWrapper` (2026-08-14) — bloqueio original resolvido pelo
    // singleton `Red::InkSystem::Get()` confirmado mais cedo hoje; composição de campo sobre o
    // handle de layer já devolvido por `BwmsInkSystemGetLayer`.
    register_inklayer_field_utils(&reg, proto);
    // `BwmsGetSystemByName` (mapa de sistemas da prova de 2026-08-14 em
    // `proofs/`): generaliza o `sysbymap` (já provado)
    // pro redscript. Ver nota grande em `register_system_by_name_utils`.
    register_system_by_name_utils(&reg, proto);
    // `LocalizationString` (candidato 6ª rodada 2026-08-11).
    let r = register_localizationstring_utils(&reg, proto);
    crate::log_dev!("[reg] register_all: cw-66 LocalizationString {r}");
    // O `inkCharacterEvent` de teste (`ponte-inkcharacterevent.reds` do dev); `GetAction`/`GetType` são do jogo na 2.5.
    let r = register_inkcharacterevent_utils(&reg, proto);
    crate::log_dev!("[reg] register_all: cw-92 inkCharacterEvent {r}");
    // `WidgetInputService` (catálogo exaustivo round 10, 2026-08-12).
    let r = register_widgetinputservice_utils(&reg, proto);
    crate::log_dev!("[reg] register_all: cw-176 WidgetInputService {r}");
    // 176c — native do ouvinte de teste (`bwms-c176-teste.reds`).
    let ok = registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32", "Int32", "Bool", "Bool"], "BwmsC176Recebido", "BwmsC176Recebido", corpo_de_prova!(tramp_c176_recebido)));
    crate::log_dev!("[reg] register_all: cw-176 BwmsC176Recebido registrado={ok}");
    // 176e — natives do controller de teste (`bwms-c176e-teste.reds`): o controller montado e cada tecla que o handler dele recebeu.
    let m = registro_de_prova!(register_argful_by_types(&reg, proto, &["Bool", "handle:IScriptable"], "BwmsC176eMontado", "BwmsC176eMontado", corpo_de_prova!(tramp_c176e_montado)));
    crate::log_dev!("[reg] register_all: cw-176 BwmsC176eMontado={m}");
    #[cfg(feature = "devtools")]
    {
        let r = register_argful_by_types(
            &reg,
            proto,
            &["handle:IScriptable", "Int32", "Int32", "Bool", "Bool"],
            "BwmsC176eRecebido",
            "BwmsC176eRecebido",
            tramp_c176e_recebido,
        );
        crate::log_dev!("[reg] register_all: cw-176 BwmsC176eRecebido={r}");
    }
    // Native do teste de medida de texto (`bwms-c115-medida-teste.reds`): 1 handle e 1 inteiro, o formato já seguro em chamada aninhada.
    let ok = registro_de_prova!(register_argful_by_types(&reg, proto, &["handle:inkText", "Int32"], "BwmsC115Guardar", "BwmsC115Guardar", corpo_de_prova!(tramp_c115_guardar)));
    crate::log_dev!("[reg] register_all: cw-115 BwmsC115Guardar registrado={ok}");
    // 115c — natives do teste do medidor (`bwms-c115c-medidor-teste.reds`): escalares e o handle do medidor.
    let m = registro_de_prova!(register_argful_by_types(&reg, proto, &["Bool", "Int32", "handle:IScriptable"], "BwmsC115cMontado", "BwmsC115cMontado", corpo_de_prova!(tramp_c115c_montado)));
    let v = registro_de_prova!(register_argful_by_types(&reg, proto, &["Float", "Float", "Float", "Float", "Bool"], "BwmsC115cVeredito", "BwmsC115cVeredito", corpo_de_prova!(tramp_c115c_veredito)));
    crate::log_dev!("[reg] register_all: cw-115 BwmsC115cMontado={m} BwmsC115cVeredito={v}");
    // 115e — natives do teste do campo de texto (`bwms-c115e-campo-teste.reds`): o campo montado e o estado dele depois de cada lote.
    let m = registro_de_prova!(register_argful_by_types(&reg, proto, &["Bool", "handle:IScriptable"], "BwmsC115eMontado", "BwmsC115eMontado", corpo_de_prova!(tramp_c115e_montado)));
    let e = registro_de_prova!(register_argful_by_types(&reg, proto, &["String", "Int32", "Int32", "Int32", "Float", "Float"], "BwmsC115eEstado", "BwmsC115eEstado", corpo_de_prova!(tramp_c115e_estado)));
    crate::log_dev!("[reg] register_all: cw-115 BwmsC115eMontado={m} BwmsC115eEstado={e}");
    // Natives do teste do campo com tema (`bwms-c116-hub-teste.reds`): o campo montado e o estado das camadas.
    let m = registro_de_prova!(register_argful_by_types(&reg, proto, &["Bool", "handle:IScriptable"], "BwmsC116Montado", "BwmsC116Montado", corpo_de_prova!(tramp_c116_montado)));
    let e = registro_de_prova!(register_argful_by_types(&reg, proto, &["String", "Float", "Float", "Float", "Float", "Int32"], "BwmsC116Estado", "BwmsC116Estado", corpo_de_prova!(tramp_c116_estado)));
    crate::log_dev!("[reg] register_all: cw-116 BwmsC116Montado={m} BwmsC116Estado={e}");
    // Nine-slice do `inkImage` pelos campos do RTTI (`bwms-hub-text-input.reds`).
    let ok = register_argful_by_types(&reg, proto, &["handle:inkImage", "Float", "Float", "Float", "Float"], "BwmsImageNineSlice", "BwmsImageNineSlice", tramp_image_nine_slice as NativeHandler);
    crate::log_dev!("[reg] register_all: cw-116 BwmsImageNineSlice={ok}");
    // Curvas `curveData:*` por propriedade de objeto (`bwms-curve.reds`).
    let r = [
        register_argful_by_types(&reg, proto, &["handle:IScriptable", "CName"], "BwmsCurveKind", "BwmsCurveKind", tramp_curve_kind as NativeHandler),
        register_argful_by_types(&reg, proto, &["handle:IScriptable", "CName"], "BwmsCurveSize", "BwmsCurveSize", tramp_curve_size as NativeHandler),
        register_argful_by_types(&reg, proto, &["handle:IScriptable", "CName", "Int32"], "BwmsCurveResize", "BwmsCurveResize", tramp_curve_resize as NativeHandler),
        register_argful_by_types(&reg, proto, &["handle:IScriptable", "CName", "Bool"], "BwmsCurveMode", "BwmsCurveMode", tramp_curve_mode as NativeHandler),
        register_argful_by_types(&reg, proto, &["handle:IScriptable", "CName", "Bool", "Int32"], "BwmsCurveSetMode", "BwmsCurveSetMode", tramp_curve_set_mode as NativeHandler),
        register_argful_by_types(&reg, proto, &["handle:IScriptable", "CName", "Int32"], "BwmsCurvePoint", "BwmsCurvePoint", tramp_curve_point as NativeHandler),
        register_argful_by_types(&reg, proto, &["handle:IScriptable", "CName", "Int32", "Int32"], "BwmsCurveValue", "BwmsCurveValue", tramp_curve_value as NativeHandler),
        register_argful_by_types(&reg, proto, &["handle:IScriptable", "CName", "Int32", "Float"], "BwmsCurveSetPoint", "BwmsCurveSetPoint", tramp_curve_set_point as NativeHandler),
        register_argful_by_types(
            &reg,
            proto,
            &["handle:IScriptable", "CName", "Int32", "Float", "Float", "Float", "Float"],
            "BwmsCurveSetValue",
            "BwmsCurveSetValue",
            tramp_curve_set_value,
        ),
    ];
    crate::log_dev!("[reg] register_all: cw-21 curvas {}/9 registradas", r.iter().filter(|x| **x).count());
    // Clone de objeto de script (`bwms-clone.reds`).
    let ok = register_argful_by_types(&reg, proto, &["handle:IScriptable"], "BwmsCloneObject", "BwmsCloneObject", tramp_clone_object as NativeHandler);
    crate::log_dev!("[reg] register_all: cw-57 BwmsCloneObject={ok}");
    // Consultas ao espelho do `ScriptBundle` (`bwms-script-bundle.reds`).
    let r = [
        registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32"], "BwmsScriptDefinitionCount", "BwmsScriptDefinitionCount", corpo_de_prova!(tramp_script_definition_count))),
        registro_de_prova!(register_argful_by_types(&reg, proto, &["CName", "CName"], "BwmsScriptSourceLine", "BwmsScriptSourceLine", corpo_de_prova!(tramp_script_source_line))),
        register_argful_by_types(
            &reg,
            proto,
            &["CName", "CName", "String"],
            "BwmsScriptSourceFileMatches",
            "BwmsScriptSourceFileMatches",
            tramp_script_source_file_matches,
        ),
    ];
    crate::log_dev!("[reg] register_all: cw-251 bundle {}/3 registradas", r.iter().filter(|x| **x).count());
    // Jobs de script no pool do motor (`bwms-job-queue.reds`).
    let r = [
        register_argful_by_types(&reg, proto, &["Uint64"], "BwmsJobQueueCreate", "BwmsJobQueueCreate", tramp_job_queue_create as NativeHandler),
        register_argful_by_types(
            &reg,
            proto,
            &["Uint64", "handle:IScriptable", "Int32", "Int32"],
            "BwmsJobQueueDispatch",
            "BwmsJobQueueDispatch",
            tramp_job_queue_dispatch,
        ),
        register_argful_by_types(&reg, proto, &["Uint64", "Uint64"], "BwmsJobQueueWait", "BwmsJobQueueWait", tramp_job_queue_wait as NativeHandler),
        register_argful_by_types(&reg, proto, &["Uint64"], "BwmsJobQueueFinalize", "BwmsJobQueueFinalize", tramp_job_queue_finalize as NativeHandler),
        register_argful_by_types(&reg, proto, &["Uint64", "Uint32"], "BwmsJobQueueSyncWait", "BwmsJobQueueSyncWait", tramp_job_queue_sync_wait as NativeHandler),
    ];
    crate::log_dev!("[reg] register_all: cw-204 jobs {}/5 registradas", r.iter().filter(|x| **x).count());
    // `ScriptInterface.GetFlat->Variant` (`PENDENCIAS-UNIFICADAS.md`). `BwmsGetFlatFloat` entrega a MESMA capacidade prática (leitura dinâmica de
    // flat por TweakDBID) usando `GetTypeName`/`GetDataPtr` já provados nesta sessão.
    registrar_leitura_de_flats_e_records(&reg, proto);
    // `ResourceDepot.ResourceExists` (`PENDENCIAS-UNIFICADAS.md`). Mecanismo interno (`crate::resource_exists`) já provado ao vivo (2026-08-10)
    // via comando de canal; isto registra a PONTA redscript-callable.
    register_resourcedepot_utils(&reg, proto);
    // CallbackSystem no RTTI: getter do singleton `CallbackSystem`, exposto como GLOBAL (não
    // `@addMethod(GameInstance)` — essa via CRASHA o boot, ver nota longa em
    // `register_callbacksystem`: "GameInstance" é validado cedo demais, antes de QUALQUER hook
    // nosso rodar). `register_all()` roda uma vez só (guard acima) — seguro registrar direto aqui,
    // mesmo padrão de BlackwallPing/BwmsAutoContinue etc.
    let okcbs = register_global(&reg, proto, "BwmsGetCallbackSystem", "BwmsGetCallbackSystem", tramp_get_callback_system);
    crate::log_dev!("[reg] register_all: BwmsGetCallbackSystem registrado={okcbs}");
    // As formas anteriores à 2.5 que a declaração do jogo trocou (`ponte-callbacksystem.reds`): o mesmo handler da native da classe,
    // que lê o tipo de cada parâmetro.
    let ok = register_argful_by_types(
        &reg, proto, &["CName", "CName", "CName", "Bool"],
        "BwmsRegisterStaticCallback", "BwmsRegisterStaticCallback", tramp_cbs_register_static_callback as NativeHandler,
    );
    crate::log_dev!("[reg] register_all: BwmsRegisterStaticCallback registrado={ok}");
    let ok = register_argful_by_types(
        &reg, proto, &["EInputKey", "EInputAction"],
        "BwmsInputTargetKey", "BwmsInputTargetKey", crate::entrada::tramp_it_key as NativeHandler,
    );
    crate::log_dev!("[reg] register_all: BwmsInputTargetKey registrado={ok}");
    // Depot do mundo, Fase 2 — mesmo padrão (getter global em vez de @addMethod(GameInstance)).
    let okdes = register_global(&reg, proto, "BwmsGetDynamicEntitySystem", "BwmsGetDynamicEntitySystem", tramp_get_dynamicentitysystem);
    crate::log_dev!("[reg] register_all: BwmsGetDynamicEntitySystem registrado={okdes}");
    // `StaticEntitySystem` — implementação NOVA 2026-08-15, mesmo padrão do `DynamicEntitySystem`
    // acima (getter global em vez de @addMethod(GameInstance)).
    let okses = register_global(&reg, proto, "BwmsGetStaticEntitySystem", "BwmsGetStaticEntitySystem", tramp_get_staticentitysystem);
    crate::log_dev!("[reg] register_all: BwmsGetStaticEntitySystem registrado={okses}");
    // Classes de evento-alvo — globais de TESTE (fixtures, nunca usadas por mod real).
    let okkie = registro_de_prova!(register_global(&reg, proto, "BwmsMakeTestKeyInputEvent", "BwmsMakeTestKeyInputEvent", corpo_de_prova!(tramp_make_test_keyinputevent)));
    crate::log_dev!("[reg] register_all: BwmsMakeTestKeyInputEvent registrado={okkie}");
    let okaie = registro_de_prova!(register_global(&reg, proto, "BwmsMakeTestAxisInputEvent", "BwmsMakeTestAxisInputEvent", corpo_de_prova!(tramp_make_test_axisinputevent)));
    let okvlce = registro_de_prova!(register_global(&reg, proto, "BwmsMakeTestVehicleLightControlEvent", "BwmsMakeTestVehicleLightControlEvent", corpo_de_prova!(tramp_make_test_vehiclelightcontrolevent)));
    let okece = registro_de_prova!(register_global(&reg, proto, "BwmsMakeTestEntityComponentEvent", "BwmsMakeTestEntityComponentEvent", corpo_de_prova!(tramp_make_test_entitycomponentevent)));
    crate::log_dev!("[reg] register_all: BwmsMakeTestAxisInputEvent={okaie} BwmsMakeTestVehicleLightControlEvent={okvlce} BwmsMakeTestEntityComponentEvent={okece}");
    // `inkWidgetSpawnEvent` (candidato 6ª rodada 2026-08-11) — mesma fixture.
    let okiwse = registro_de_prova!(register_global(&reg, proto, "BwmsMakeTestInkWidgetSpawnEvent", "BwmsMakeTestInkWidgetSpawnEvent", corpo_de_prova!(tramp_make_test_inkwidgetspawnevent)));
    crate::log_dev!("[reg] register_all: BwmsMakeTestInkWidgetSpawnEvent registrado={okiwse}");
    // `EntityBuilderWrapper` (item nunca tocado, 2026-08-18 madrugada cont.20) —
    // mesma fixture (ver nota grande em `register_entitybuilderwrapper`/
    // `tramp_make_test_entitybuilderwrapper`: 100% sintética, sem trigger real conhecido ainda).
    let okebw = registro_de_prova!(register_global(&reg, proto, "BwmsMakeTestEntityBuilderWrapper", "BwmsMakeTestEntityBuilderWrapper", corpo_de_prova!(tramp_make_test_entitybuilderwrapper)));
    crate::log_dev!("[reg] register_all: BwmsMakeTestEntityBuilderWrapper registrado={okebw}");
    // `ResourceToken` (2026-08-18 madrugada, continuação dedicada) — mesma fixture,
    // cross-referenciando o `inkWidgetBrush.textureAtlas` achado hoje pelo `#199`
    // (`scan11`/`res11probe`, ver `tramp_make_test_inkwidgetbrush`): permite ao smoke test do `#18`
    // exercitar o branch `type_kind==11` através da CLASSE WRAPPER `ResourceToken.reds` pela 1ª
    // vez (antes só `mesh`/`rig`, ambos guard-only, nunca alcançavam esse branch).
    let okiwb = registro_de_prova!(register_global(&reg, proto, "BwmsMakeTestInkWidgetBrush", "BwmsMakeTestInkWidgetBrush", corpo_de_prova!(tramp_make_test_inkwidgetbrush)));
    crate::log_dev!("[reg] register_all: BwmsMakeTestInkWidgetBrush registrado={okiwb}");
    // Fachada de versão (Require/Version): NÃO chamar aqui — register_all() roda EM TODO
    // TICK (idempotente pras outras natives via checagem de re-resolve, mas register_method NÃO
    // dedupa: chamar de novo aqui empilharia Version/Require DUPLICADOS a cada tick). A represa de
    // timing (classe precisa existir ANTES do bind eager, ~6s, bem antes do 1º tick) é resolvida
    // em `inicio/boot.rs::class_validate_probe_hook` (2026-07-13): forja a classe (register_type_min)
    // + registra os métodos NA HORA CERTA, 1x só (guard próprio em `inicio/boot.rs`), gated
    // ~/.bwms-classvalidate-probe.
    // `redscript-mod-persistence` (2026-07-13): config EXTERNO ao save (regra-mãe do projeto —
    // "nada escreve dado de mod dentro do save"), pra estado de cheat/mod sobreviver entre
    // sessões/saves diferentes sem tocar o save-file. Só 2 globais (mesmo padrão de
    // register_argful_by_types já provado 20+ vezes esta sessão, zero risco novo).
    let okcg = register_argful_by_types(&reg, proto, &["String"], "BwmsConfigGet", "BwmsConfigGet", tramp_config_get);
    let okcs = register_argful_by_types(&reg, proto, &["String", "String"], "BwmsConfigSet", "BwmsConfigSet", tramp_config_set);
    crate::log_dev!("[reg] register_all: BwmsConfigGet={okcg} BwmsConfigSet={okcs} (persistência externa ao save)");
    let okla = register_argful_by_types(&reg, proto, &["String"], "BwmsGetLaunchArg", "BwmsGetLaunchArg", tramp_launch_arg as NativeHandler);
    let okha = register_argful_by_types(&reg, proto, &["String"], "BwmsHasLaunchArg", "BwmsHasLaunchArg", tramp_has_launch_arg as NativeHandler);
    crate::log_dev!("[reg] register_all: BwmsGetLaunchArg={okla} BwmsHasLaunchArg={okha} (argumentos de lançamento)");
    // Screenshot pelo motor.
    let okss = register_argful_by_types(&reg, proto, &["String"], "BwmsTakeScreenshot", "BwmsTakeScreenshot", tramp_take_screenshot as NativeHandler);
    crate::log_dev!("[reg] register_all: BwmsTakeScreenshot={okss}");
    // Bisecção do crash full-body (2026-07-15): contador de ciclos EXATO por poller.
    let okpt = registro_de_prova!(register_argful_by_types(&reg, proto, &["String", "Int32"], "BwmsPollerTick", "BwmsPollerTick", corpo_de_prova!(tramp_poller_tick)));
    crate::log_dev!("[reg] register_all: BwmsPollerTick={okpt} (contador de ciclos, bisecção do crash full-body)");
    // API de reflexão de classe (2026-07-24): globais de reflection generalizadas (ver nota longa
    // em `register_reflection_natives`) — reclassifica o gap de XL pra "repackaging" dos
    // primitivos já provados.
    register_reflection_natives(&reg, proto);
    register_registro_de_records(&reg, proto);
    // Propriedades e classes novas antes do bind dos scripts.
    registrar_tipos_de_mod(&reg);
    // construtor de entidade (2026-07-24): `BwmsEntityAddComponent` (ver nota longa em
    // `register_entity_builder_natives`) — atalho que evita forjar `Red::EntityBuilder`.
    register_entity_builder_natives(&reg, proto);
    // 130b: a native de anotação do teste de despacho dos eventos `Entity/*` (`bwms-c130b-entidade-teste.reds`).
    register_c130b_natives(&reg, proto);
    // 130d: as natives de anotação do teste do `Entity/Extract` (`bwms-c130d-extract-teste.reds`).
    register_c130d_natives(&reg, proto);
    // 130e: a native de anotação do teste do `Entity/Reassemble` (`bwms-c130e-reassemble-teste.reds`).
    register_c130e_natives(&reg, proto);
    // As natives de anotação do teste dos alvos (`bwms-c134-alvos-teste.reds`).
    register_c134_natives(&reg, proto);
    // 130c: a native de anotação do teste do `Entity/Assemble` (`bwms-c130c-assemble-teste.reds`).
    register_c130c_natives(&reg, proto);
    // As natives de anotação do teste do `CommunityWrapper` (`bwms-c77-comunidade-teste.reds`).
    register_c77_natives(&reg, proto);
    // O ciclo do `OpenWorldTracker` num PoI real (`bwms-c142-teste.reds`).
    register_c142_natives(&reg, proto);
    // O ciclo de entidade transiente (`bwms-c73b-teste.reds`).
    register_c73b_natives(&reg, proto);
    // Anotação do handler estático do teste (`bwms-c129-teste.reds`).
    let a = registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32", "Uint64"], "BwmsC129Anotar", "BwmsC129Anotar", corpo_de_prova!(tramp_c129_anotar)));
    crate::log_dev!("[reg] register_all: c129 Anotar={a}");
    // Trava de save dos testes que mexem em quest ou mundo (`bwms-trava-save-teste.reds`).
    let a = registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32", "Uint64"], "BwmsTravaAnotar", "BwmsTravaAnotar", corpo_de_prova!(tramp_trava_anotar)));
    crate::log_dev!("[reg] register_all: trava Anotar={a}");
    // Reinício de rede de segurança real (`bwms-c20-teste.reds`).
    let a = registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32", "Uint64"], "BwmsC20Anotar", "BwmsC20Anotar", corpo_de_prova!(tramp_c20_anotar)));
    crate::log_dev!("[reg] register_all: c20 Anotar={a}");
    // 47b: o ciclo do fluxo de edição sem boneco (`bwms-a47b-teste.reds`).
    let a = registro_de_prova!(register_argful_by_types(&reg, proto, &["handle:IScriptable"], "BwmsA47Ciclo", "BwmsA47Ciclo", corpo_de_prova!(tramp_a47_ciclo)));
    crate::log_dev!("[reg] register_all: a47 Ciclo={a}");
    // Entidade por template (`bwms-c75-teste.reds`).
    let a = registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32", "Uint64"], "BwmsC75Anotar", "BwmsC75Anotar", corpo_de_prova!(tramp_c75_anotar)));
    crate::log_dev!("[reg] register_all: c75 Anotar={a}");
    // O resto do `DynamicEntitySystem` (`bwms-c73c-teste.reds`).
    let a = registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32", "Uint64"], "BwmsC73cAnotar", "BwmsC73cAnotar", corpo_de_prova!(tramp_c73c_anotar)));
    crate::log_dev!("[reg] register_all: c73c Anotar={a}");
    // `GetWorkspotInstance` pelo `EntityID` do ator (`bwms-workspot.reds`) e a anotação do teste (`bwms-c82-workspot-teste.reds`).
    register_c82_natives(&reg, proto);
    // A native de etapa do teste dos overrides de aparência (`bwms-a11-teste.reds`).
    register_a11_natives(&reg, proto);
    // A native de anotação do teste da ativação e desativação dos hooks de evento (`bwms-c118-ativacao-teste.reds`).
    register_c118_natives(&reg, proto);
    // Árvore de widget e controller anexados à camada da tela (`bwms-widget-building.reds`).
    let r = [
        register_argful_by_types(&reg, proto, &["handle:inkWidget"], "BwmsWidgetAttachTree", "BwmsWidgetAttachTree", tramp_widget_attach_tree as NativeHandler),
        register_argful_by_types(
            &reg,
            proto,
            &["handle:inkWidget", "handle:IScriptable", "Bool"],
            "BwmsWidgetAttachController",
            "BwmsWidgetAttachController",
            tramp_widget_attach_controller,
        ),
    ];
    crate::log_dev!("[reg] register_all: cw-175 widget building {}/2 registradas", r.iter().filter(|x| **x).count());
    // Teste de cobertura (`bwms-c121-teste.reds`): o estado do controller depois do `AttachController` vanilla.
    #[cfg(feature = "devtools")]
    {
        let m = register_argful_by_types(
            &reg,
            proto,
            &["handle:inkWidget", "handle:IScriptable"],
            "BwmsC121Medir",
            "BwmsC121Medir",
            crate::interface::tramp_c121_medir,
        );
        crate::log_dev!("[reg] register_all: BwmsC121Medir={m}");
    }
    // Áudio: os bancos do `Resource/Load` dos soundbanks (`bwms-soundbanks.reds`).
    let r = [
        register_global(&reg, proto, "BwmsSoundBankCount", "BwmsSoundBankCount", tramp_soundbank_count),
        register_argful_by_types(&reg, proto, &["Int32"], "BwmsSoundBankName", "BwmsSoundBankName", tramp_soundbank_name as NativeHandler),
        register_argful_by_types(&reg, proto, &["Int32"], "BwmsSoundBankIsResident", "BwmsSoundBankIsResident", tramp_soundbank_is_resident as NativeHandler),
        register_argful_by_types(&reg, proto, &["Int32"], "BwmsSoundBankPath", "BwmsSoundBankPath", tramp_soundbank_path as NativeHandler),
        register_argful_by_types(&reg, proto, &["Int32", "Bool"], "BwmsSoundBankSetResident", "BwmsSoundBankSetResident", tramp_soundbank_set_resident as NativeHandler),
        register_argful_by_types(&reg, proto, &["Int32", "String"], "BwmsSoundBankSetPath", "BwmsSoundBankSetPath", tramp_soundbank_set_path as NativeHandler),
        register_argful_by_types(&reg, proto, &["CName", "Bool", "String"], "BwmsSoundBankAdd", "BwmsSoundBankAdd", tramp_soundbank_add as NativeHandler),
    ];
    crate::log_dev!("[reg] register_all: cw-122 soundbanks {}/7 registradas", r.iter().filter(|x| **x).count());
    // A native de anotação do teste do `Resource/Load` pelo serializer (`bwms-c122-recurso-teste.reds`).
    register_c122_natives(&reg, proto);
    // Áudio: a native de anotação do teste do reenvio dos soundbanks (`bwms-c122sb-teste.reds`).
    register_c122sb_natives(&reg, proto);
    // Objeto do motor que não é `IScriptable` entregue ao script dentro de um `BwmsSerializable` (`bwms-serializable.reds`): classe, herança e hash do recurso.
    register_resource_natives(&reg, proto);
    // Gravar e ler objetos em arquivo (`bwms-object-serializer.reds`) e a anotação do teste (`bwms-c252-teste.reds`).
    register_object_serializer_natives(&reg, proto);
    // A native de anotação do teste do ciclo de vida do `ScriptableService` (`bwms-c239-teste.reds`).
    let ok = registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32", "handle:IScriptable"], "BwmsC239Anotar", "BwmsC239Anotar", corpo_de_prova!(tramp_c239_anotar)));
    crate::log_dev!("[reg] register_all: c239 BwmsC239Anotar={ok}");
    let ok = registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32", "handle:IScriptable"], "BwmsC252Anotar", "BwmsC252Anotar", corpo_de_prova!(tramp_c252_anotar)));
    crate::log_dev!("[reg] register_all: c252 BwmsC252Anotar={ok}");
    // A native de anotação do teste do `WidgetBuildingService` (`bwms-c175-widgets-teste.reds`).
    let ok = registro_de_prova!(register_argful_by_types(&reg, proto, &["Int32", "handle:IScriptable"], "BwmsC175Anotar", "BwmsC175Anotar", corpo_de_prova!(tramp_c175_anotar)));
    crate::log_dev!("[reg] register_all: c175 BwmsC175Anotar={ok}");
    // Grafo de quest/`#160` (2026-08-15): `BwmsQuestPhaseGetRootGraph`/`BwmsQuestGraphNodeCount`/
    // `BwmsQuestGraphNodeAt` (ver nota longa em `register_quest_graph_natives`)
    register_quest_graph_natives(&reg, proto);
    // Só o `bwms-puppetstate-mirror.reds` do dev declara.
    #[cfg(feature = "devtools")]
    register_puppetstate_recovery_native(&reg, proto);
    // 2026-08-12: `MappinSystem.SetPoiMappinPhase`/`GetPoiMappinPhase`
    // via slot de vtable Mac (mecanismo confirmado, ver nota grande em
    // `register_mappinsystem_poi_phase_natives`).
    register_mappinsystem_poi_phase_natives(&reg, proto);
    // Offsets de IGameSystem (2026-08-16): dump
    // read-only dos 11 slots que o CallbackSystem real override, contra MappinSystem vivo (ver
    // nota grande em `register_mappinsystem_igamesystem_vtdump`).
    register_mappinsystem_igamesystem_vtdump(&reg, proto);
    // Continuação — `trocar_slot_da_vtable` real no slot ÚNICO `OnGameRestored` (ver nota
    // grande em `register_mappinsystem_hook_ongamerestored`).
    register_mappinsystem_hook_ongamerestored(&reg, proto);
    // Continuação 2 (2026-08-18) — `trocar_slot_da_vtable` real no slot `OnWorldAttached`
    // (ver nota grande em `register_mappinsystem_hook_onworldattached`).
    register_mappinsystem_hook_onworldattached(&reg, proto);
    // `JournalExtension` (2026-08-14): contagem de mappins/POIs já cacheados
    // (campos de DADO `CookedMappinResource@0x58`/`CookedPoiResource@0x68`, mecanismo confirmado,
    // ver nota grande em `register_mappinsystem_cooked_count_natives`).
    register_mappinsystem_cooked_count_natives(&reg, proto);
    // `TraceResult.GetHitObject`/`GetHitEntity` (2026-09-11): endereços
    // achados offline (ver nota em `register_traceresult_hit_natives`).
    register_traceresult_hit_natives(&reg, proto);
    // `CommunityWrapper`/`CommunitySystem` (2026-08-14): 7 slots de vtable
    // (mecanismo confirmado, ver nota grande em `register_communitysystem_natives`).
    register_communitysystem_natives(&reg, proto);
    // Continuação (2026-08-14): 2 getters puros
    // via slot de vtable Mac (`IPersistencySystem::GetEntityStatus`/`IPopulationSystem::IsSpawning`,
    // ver nota grande em `register_persistency_population_natives`).
    register_persistency_population_natives(&reg, proto);
    // `WeatherSystem` (2026-08-12, agente dedicado offline): leitura pura
    // de campo do `worldRuntimeSystemWeather` real (ver nota grande em
    // `register_weathersystem_state_natives`).
    register_weathersystem_state_natives(&reg, proto);
    // `WeatherSystem.SetWeather`/`ResetWeather` (2026-09-11): endereços achados
    // offline (ver nota em `register_weathersystem_set_natives`). MUTA o clima do mundo.
    register_weathersystem_set_natives(&reg, proto);
    // `GetEnvironmentDefinition`/`GetWeatherState` (2026-09-14): só leitura, com a anotação do teste (`bwms-c81-clima-teste.reds`).
    register_weathersystem_read_natives(&reg, proto);
    // `TagSystem::AssignTag`/`UnassignTag` do motor (2026-09-14) e a anotação do teste (`bwms-c220-tag-teste.reds`).
    register_tagsystem_natives(&reg, proto);
    // Marcação e anotação do teste dos alvos por tag (`bwms-c133-alvos-teste.reds`).
    register_c133_natives(&reg, proto);
    // Anotação dos testes do `ResourceTarget` e da troca de clima (`bwms-c6-recurso-teste.reds`, `bwms-c81-troca-teste.reds`).
    register_c6_natives(&reg, proto);
    register_c81t_natives(&reg, proto);
    // Medição dos offsets de telemetria de veículo no Mac (`bwms-c71-veiculo-teste.reds`).
    register_c71_natives(&reg, proto);
    // `ComponentWrapper.GetChunkMask`/`SetChunkMask`: campo cru `chunkMask` do componente (offset pelo RTTI em
    // `roupas_e_armas::garment_chunk_mask_offset`, com a correção `+0x240`/`+0x248`; round-trip provado em 2026-08-15, `proofs/`).
    register_componentwrapper_chunkmask_natives(&reg, proto);
    // Round 14 (2026-08-16): `SetResourcePath`/`GetResourcePath`
    // + `SetAppearanceName`/`GetMeshAppearanceName` (o real `GetAppearanceName` colide com a
    // native vanilla `IComponent.GetAppearanceName()`, que lê um campo DIFERENTE — achado desta
    // rodada, ver nota grande em `register_componentwrapper_extra_natives`) + `IsMeshComponent`.
    register_componentwrapper_extra_natives(&reg, proto);
    // `RenderProxy` (2026-09-11): proxy de render do componente + 4 funções do motor.
    register_renderproxy_natives(&reg, proto);
    // Parte 3 (2026-09-11): leitura da árvore do journal pelo layout nativo.
    register_journal_arvore_natives(&reg, proto);
    register_spawner_census_natives(&reg, proto);
    // `VisualController` (round 12 — 2026-08-12): leitura pura de
    // campo via `entVisualControllerComponent+0xD8` já obtido pelo redscript (ver nota grande em
    // `register_visualcontroller_213_natives`).
    register_visualcontroller_213_natives(&reg, proto);
    // `entVisualControllerComponent` (correção de risco 2026-08-18): leitura
    // pura dos campos de TOPO do componente (0x90-0xF3), distinta da anterior (sub-struct C++
    // `VisualController` via `Controller@0xD8`).
    register_vcc_33_natives(&reg, proto);
    // `CMaterialInstance`/`CMeshMaterialEntry`/`meshMeshAppearance` (correção de risco
    // 2026-08-18): observação PASSIVA do `CMesh*`
    // vivo por trás de `entVisualControllerComponent.meshProxy`, só quando o
    // resource-token já estiver carregado por outra parte do motor — nunca dispara load.
    register_cmesh_via_vcc_natives(&reg, proto);
    // `VehicleObject` (round 3 — 2026-08-12): leitura pura de
    // campo (ver nota grande em `register_vehicleobject_telemetry_natives`).
    register_vehicleobject_telemetry_natives(&reg, proto);
    // `VehicleSystem.ToggleGarageVehicle` (round 3 —
    // 2026-08-12): reusa endereço já RE'd em 2026-07-29 (ver nota grande em
    // `register_vehiclesystem_toggle_garage_native`).
    register_vehiclesystem_toggle_garage_native(&reg, proto);
    // `inkWidgetRef.Set` (round 5 — 2026-08-12): campo puro (ver nota
    // grande em `register_inkwidgetref_set`).
    register_inkwidgetref_set(&reg, proto);
    // `WidgetSpawningService::ToggleWidgetSpawnEvent`: grava a chave `WIDGET_SPAWN_EVENT_ENABLED`, que os hooks de
    // `interface::spawn_de_widget` leem junto com o ouvinte registrado antes de despachar "InkWidget/Spawn".
    let ok = register_toggle_widget_spawn_event(&reg, proto);
    crate::log_dev!("[reg] register_all: cw-widgetspawnevent-toggle (#123) BwmsToggleWidgetSpawnEvent={ok}");
    // 2026-08-10: prova do mecanismo de `script_ref<T>` real.
    register_scriptref_proof(&reg, proto);
    // `ISerializable.ProcessPostLoad` (round 9 — 2026-08-12): dispatch virtual
    // genérico via vtable slot 0x30 (ver nota grande em `register_iscriptable_postload_native`).
    register_iscriptable_postload_native(&reg, proto);
    // Marshalling de Variant (2026-07-25): BwmsVariantLog(a: Variant, b: Variant) -> Void.
    // Proto Variant = param[1] de gameIBlackboard::SetVariant. Prova boxing/unboxing de tipos mistos.
    register_variant_natives(&reg, proto);
    // Módulo de player, 100% redscript puro (@addField/@addMethod/@wrapMethod sobre
    // PlayerSystem/inkPuppetPreviewGameController/PhotoModePlayerEntityComponent, todas vanilla —
    // zero forge de RTTI, zero native func). Teste (`BwmsPlayerModuleTest`, também pura redscript)
    // é chamado sob demanda pelo comando de canal `playertest` (lib.rs), não daqui.
    // 2026-08-12: probes observe-only do `__invoke` de RegisterEventConnector
    // de `ent::VisualControllerComponent` (ver nota grande em `install_visualcontroller_event_probes`).
    // Chamado daqui (dentro do corpo "roda 1x" de `register_all`, gated ~/.bwms-hook-* individual,
    // nunca ligado por padrão) porque `register.rs` não tem hook installer próprio existente e não
    // deve tocar `selftest.rs` (outro agente trabalha lá em paralelo nesta sessão).
    unsafe { install_visualcontroller_event_probes() };
}

/// Retorno `String` de qualquer tamanho, pelo construtor do `CString` do motor (texto maior que o buffer inline vai pro alocador do jogo).
pub(crate) unsafe fn write_string_ret(out: *mut c_void, s: &str) {
    if !out.is_null() {
        crate::jogo::cstring::nova(out as *mut u8, s);
    }
}

/// `ResourceToken<T>`: `resource` `+0x28`, `path` `+0x48`, `job` `+0x50` (`JobHandle`, 8 B), `finished` `+0x58`, `error` `+0x5c`.
pub(crate) unsafe fn status_do_token(t: u64) -> i32 {
    resource_token_status(((t + 0x58) as *const i32).read_unaligned(), ((t + 0x5c) as *const u8).read())
}

/// Devolve o recurso JÁ CARREGADO cujo caminho tem o hash dado, lendo o mapa de recursos vivos do bank do loader (`[loader+0x48]`).
///
/// **Como isto ficou possível (2026-08-21):** o mapa (em `+0x00` do bank `[loader+0x48]`) foi identificado por
/// CONTEÚDO, não por forma — procurei nele o hash de um caminho real que o PRÓPRIO MOTOR gerou
/// (`base\gameplay\gui\common\icons\atlas_common.inkatlas` = `0xcb91e1aeaf64dd2c`, capturado por
/// `reslinkdump`), achei a chave, e o RTTI nomeou o valor como **`inkTextureAtlas`** — o tipo exato
/// de um `.inkatlas`. Ou seja: `HashMap<ResourcePath, CResource*>`, o cache de recursos vivos.
///
/// Zero endereço nativo novo: é leitura de dado sobre o loader de `CARREGADOR_VM` com o mesmo
/// idioma de `HashMap` do RED já provado no `WardrobeSystem` (246 entradas) e agora exercitado em
/// escala (~55 mil).
///
/// **Não inicia carga:** este devolve o recurso se ele JÁ estiver
/// carregado, e `null` se não — não inicia carga. Quem precisa carregar usa o `LoadAsync` do motor
/// (`0x1010f0d34`, `jogo/recursos.rs::token_do_path`).
pub(crate) unsafe fn depot_loaded_resource(path_hash: u64) -> *mut c_void {
    crate::arquivos_do_jogo::depot::depot_cache_buscar(path_hash)
}

#[cfg(test)]
mod testes_dynarray {
    /// Só teste (regra do dono de 2026-10-03, D4): a API de tupla `(entries, size, capacity)` sobre o
    /// decodificador único `bwms_primitives::dynarray::DynArray::decodificar`.
    fn dynarray_decode(bytes: &[u8]) -> Option<(u64, u32, u32)> {
        let d = bwms_primitives::dynarray::DynArray::decodificar(bytes, 0)?;
        Some((d.entries, d.size, d.capacity))
    }

    /// A ordem é `entries · capacity · size`. Este teste existe porque a ordem CONTRÁRIA
    /// (`entries · size · capacity`) estava viva em 5 sítios deste arquivo até 2026-09-06,
    /// induzida pela lista de inicialização do construtor no header do SDK.
    #[test]
    fn capacidade_vem_antes_do_tamanho() {
        let mut b = [0u8; 16];
        b[0..8].copy_from_slice(&0xdead_beef_u64.to_le_bytes());
        b[8..12].copy_from_slice(&64u32.to_le_bytes()); // capacity
        b[12..16].copy_from_slice(&3u32.to_le_bytes()); // size
        let (entries, size, cap) = dynarray_decode(&b).unwrap();
        assert_eq!(entries, 0xdead_beef);
        assert_eq!(size, 3, "size mora em +0x0C");
        assert_eq!(cap, 64, "capacity mora em +0x08");
    }

    /// O caso que escondeu o bug por meses: com `capacity == size` as duas leituras coincidem.
    #[test]
    fn capacidade_igual_ao_tamanho_esconde_a_troca() {
        let mut b = [0u8; 16];
        b[0..8].copy_from_slice(&1u64.to_le_bytes());
        b[8..12].copy_from_slice(&7u32.to_le_bytes());
        b[12..16].copy_from_slice(&7u32.to_le_bytes());
        let (_, size, cap) = dynarray_decode(&b).unwrap();
        assert_eq!(size, cap);
    }

    #[test]
    fn buffer_curto_recusa_em_vez_de_ler_lixo() {
        assert!(dynarray_decode(&[0u8; 15]).is_none());
        assert!(dynarray_decode(&[]).is_none());
    }

    /// Crivo único de `dynarray_coerente` (caminhada de `Entity+0xA0` das natives de tag, setor do mundo, flats de TweakDB,
    /// `rtti::dynarray_remove_at`): `size > capacity`, ponteiro nulo com elemento e capacidade acima de `1<<20` são offset
    /// errado lido como dado; zeros e folga são `DynArray` legítimos. Cada trio confere também os dois crivos de
    /// `bwms_primitives::dynarray::DynArray`, que dizem a mesma coisa.
    #[test]
    fn crivo_de_coerencia_por_trio() {
        use super::dynarray_coerente;
        use bwms_primitives::dynarray::DynArray;
        // (entries, size, capacity, coerente)
        let trios: [(usize, u32, u32, bool); 8] = [
            (0x1000, 5, 4, false),
            (0x1000, 3, 0, false),
            (0, 1, 4, false),
            (0x1000, 1, (1 << 20) + 1, false),
            (0x1000, 1 << 20, 1 << 20, true),
            (0, 0, 0, true),
            (0x1000, 3, 64, true),
            (0x1000, 7, 7, true),
        ];
        for (e, size, cap, coerente) in trios {
            assert_eq!(dynarray_coerente(e, size, cap), coerente, "dynarray_coerente({e:#x}, {size}, {cap})");
            let d = DynArray { entries: e as u64, capacity: cap, size };
            assert_eq!(d.estrutura_valida() && d.capacidade_plausivel(), coerente, "dynarray ({e:#x}, {size}, {cap})");
        }
    }

    /// As caminhadas leem o trio por `dynarray_em`, que devolve `(entries, size, capacity)` com `size` de +0x0C.
    #[test]
    fn dynarray_em_devolve_size_de_0x0c() {
        let mut b = [0u8; 16];
        b[0..8].copy_from_slice(&0xdead_beef_u64.to_le_bytes());
        b[8..12].copy_from_slice(&64u32.to_le_bytes());
        b[12..16].copy_from_slice(&3u32.to_le_bytes());
        let trio = unsafe { super::dynarray_em(b.as_ptr().cast()) };
        assert_eq!(trio, (0xdead_beef, 3, 64));
    }
}

#[cfg(test)]
mod resource_token_status_tests {
    use super::resource_token_status;
    #[test]
    fn ainda_carregando() {
        assert_eq!(resource_token_status(0, 0), 0);
    }
    #[test]
    fn carregado_com_sucesso() {
        assert_eq!(resource_token_status(1, 0), 1);
        assert_eq!(resource_token_status(42, 0), 1);
    }
    #[test]
    fn falhou_prevalece_sobre_finished() {
        // `error` sozinho já é falha, mesmo com `finished` ainda não setado por alguma race de leitura.
        assert_eq!(resource_token_status(0, 1), 2);
        assert_eq!(resource_token_status(1, 1), 2);
    }
}

// `BwmsHashToResRef(hash) -> ResRef` usa o `tramp_identidade_8_bytes` (identidade de 8 B).

/// Variante por HASH do `depot_loaded_resource_class` (que recebe path). Existe porque `ResourceRef`
/// É um hash — pedir o path de volta seria reverter FNV-1a64, que não dá.
pub(crate) unsafe fn depot_loaded_resource_class_by_hash(hash: u64) -> String {
    let res = depot_loaded_resource(hash);
    if res.is_null() || !crate::memoria::is_readable(res, 0x40) {
        return String::new();
    }
    let cls = rtti::class_of(res);
    if cls.is_null() {
        return String::new();
    }
    crate::cname::resolve_cname(rtti::type_name_getname(cls))
}

// ===== Modelo de edição de texto (`TextInput`), núcleo ORIGINAL.
//
// Esta camada cobre `Caret`/`Selection`/`TextFlow` com um modelo próprio, cuja lógica inteira mora em
// `bwms_primitives::editor_texto` — puro, sem RTTI, testado offline em `editor_texto::tests`. Aqui só há plumbing.
//
// O estado vive no Rust, não em campos de redscript, por um motivo concreto: o modelo é indexado
// em codepoint (`char`) e o redscript só oferece `String` (bytes). Manter o buffer do lado que sabe contar
// codepoints é o que impede a classe de bug de fatiar um codepoint ao meio — que este projeto já
// cometeu. A `.reds` só manda comandos e lê o resultado renderizável.
//
// A medição fica no `BwmsTextMeasurer` (`bwms-text-measurer.reds`): `GetDesiredSize` só sai de (0,0) com família de fonte, a 1ª
// medida espera a fonte carregar (boot 151), e anda na mesma unidade do modelo (`BwmsUTF8Split`, um codepoint por vez).

// ===== `worldStreamingSector` alcançado SEM forjar o tipo.
//
// **A objeção que travava o item, e por que ela não se aplica aqui.** O item estava marcado
// CMesh-trap: `worldStreamingSector` tem ZERO ocorrências em `redscript-src`, e declarar
// `native class` de um tipo cujo pai não é script-visível crasha o bind do RTTI (item `#30`).
// Verdade — mas isso vale pra DECLARAR o tipo em redscript. Ler o objeto pelo Rust não declara
// nada, e a via já existe inteira, montada de peças provadas:
//
//   1. `archive-resources.txt` (o dump de 100% do install) tem **26.356 paths `.streamingsector`
//      reais**, incluindo os `always_loaded_*`, que por definição estão carregados em gameplay;
//   2. `bwms_primitives::resource_path_hash` converte path → hash (PROVADO, FNV-1a64);
//   3. `depot_loaded_resource(hash)` devolve o `CResource*` vivo do mapa no bank `[loader+0x48]`
//      (PROVADO 2026-09-04, ~55k entradas);
//   4. `worldStreamingSector.nodes` é um `DynArray` em **+0x48** — offset já RE'd e testado R/W
//      em 2026-07-26 (aplicação no streaming, ciclo `sz 21→22→21` completo, zero crash).
//
// Nada aqui é endereço novo. É a composição que ninguém tinha feito porque o item foi arquivado
// como "não tentar" sem separar "declarar em redscript" de "ler pelo Rust".
//
// SÓ LEITURA. O ciclo de escrita de 2026-07-26 crashou por refcount (raw copy sem IncRef, e
// `worldNode` é memory-mapped), então esta camada não escreve — e nem precisa, porque o que os
// métodos `GetNodes`/`GetNodeCount`/`GetNode` do item pedem é leitura.

// ===== Os 8 acessores do `worldStreamingSector`, sobre os 3 arrays já IDENTIFICADOS.
//
// Offsets confirmados ao vivo em 2026-09-05, cada um por mecanismo independente do que o achou:
//   +0x68 `nodes`      — elemento 0 resolveu classe RTTI `worldStaticMarkerNode`
//   +0x78 `nodeRefs`   — stride 8, 5/5 amostras com cara de hash (NodeRef é hash de 8 bytes)
//   +0x88 `nodeSetups` — stride > 8, 0/5 com cara de hash (struct por valor)
//
// SÓ LEITURA, de propósito: o ciclo de escrita tentado em 2026-07-26 crashou por refcount
// (raw copy sem IncRef, e `worldNode` é memory-mapped). Os 8 métodos que o item pede são
// todos de leitura.

/// `Handle<T>` do RED ocupa 2 ponteiros (`instance` + `refcount`). Confirmado ao vivo em
/// 2026-09-05: com passo 8 só o índice 0 resolvia classe; com 16, todos.
pub(crate) const HANDLE_STRIDE: usize = 16;

/// `(entries, size, capacity)` lidos da memória viva, na ordem de memória `entries · capacity · size`
/// (a prova da ordem está em `bwms_primitives::dynarray`). Sem crivo — quem chama decide.
pub(crate) unsafe fn dynarray_em(b: *const c_void) -> (usize, u32, u32) {
    let entries = rd_u64(b, 0) as usize;
    let capacity = rd_u32(b, 8);
    let size = rd_u32(b, 0x0C);
    (entries, size, capacity)
}

/// `true` se o trio pode ser um `DynArray` e não offset errado lido como dado: os dois crivos de
/// `bwms_primitives::dynarray::DynArray`, a única definição do que é coerente. Só refuta — zeros passam.
pub(crate) fn dynarray_coerente(entries: usize, size: u32, cap: u32) -> bool {
    let d = bwms_primitives::dynarray::DynArray { entries: entries as u64, capacity: cap, size };
    d.estrutura_valida() && d.capacidade_plausivel()
}

// ===== Consultas ao espelho do `ScriptBundle` (`bwms-script-bundle.reds`) =====
// O índice sai do hook do `ScriptValidator::Validate` (`inicio/boot.rs`), na carga dos scripts.

// ===== `inkWorldWidgetInfos::GetComponent` (UI 3D no mundo).
//
// São exatamente 2 saltos de ponteiro: o `SharedPtr<worlduiWorldLayerWidget>` em `+0x40` (vazio = nada)
// e, dentro do widget, o `WeakHandle<IComponent>` em `+0x08`.
//
// O outro método do item (`worlduiIGameController::GetWorldWidgetComponent`) é redscript PURO —
// sobe a cadeia `parentWidget` procurando o userData — e ambos os tipos que ele toca
// (`worlduiIGameController`, `inkUserData`) existem no corpus. O que faltava era só este acesso.
//
// **Por que não declarar `inkWorldWidgetInfos` em redscript:** é `abstract native class` com ZERO
// ocorrência no corpus (só existe registrada em runtime). Declarar é a armadilha do
// CMesh. Mesma saída do `worldStreamingSector`: o redscript passa o `ref<inkUserData>` que `GetUserData` já
// devolve, e a leitura acontece aqui — ler pelo Rust não declara tipo nenhum.

// ===== `MeshComponent.meshResource` por COMPOSIÇÃO.
//
// O item foi TENTADO+REVERTIDO em 2026-08-10: tipar `ref<CMesh>` exige declarar
// `native class CMesh`, e `CMesh` tem zero ocorrência no corpus — o bind de RTTI crasha
// (`EXC_BREAKPOINT`). O bloqueio é real pra DECLARAR. Não é pra OBTER.
//
// Há dois campos pro mesmo recurso: `meshResource: ref<CMesh>` (o objeto já
// carregado) e `mesh: ResourceAsyncRef` (a referência assíncrona). O segundo é nativo do
// componente — `RaRef<CMesh> mesh` em `+0x150` — e
// `ResourceAsyncReference` guarda **só um `ResourcePath`**,
// isto é, um `u64` de hash. Daí em diante a cadeia já é provada:
//
//   +0x150 (hash) → mapa no bank `[loader+0x48]` → `CResource*`  (`#18`, PROVADO 2026-09-05)
//
// e as natives `BwmsResRefIsLoaded`/`BwmsResRefResourceClass` já tomam um hash direto. Ou seja:
// a capacidade que `meshResource` entrega sai sem declarar tipo nenhum.

pub(crate) unsafe fn prop_value_offset(cls: *mut c_void, nome: &str) -> Option<usize> {
    if !rtti::sane(cls) {
        return None;
    }
    let p = rtti::find_property_in_class(cls, nome);
    if p.is_null() || !crate::memoria::is_readable(p, CPROPERTY_VALUE_OFFSET + 4) {
        return None;
    }
    let off = rd_u32(p as *const c_void, CPROPERTY_VALUE_OFFSET) as usize;
    // offset absurdo = leitura errada do CProperty, não propriedade exótica
    (off > 0 && off < (1 << 16)).then_some(off)
}

/// `(classe, offset, endereço do campo)` de uma propriedade de um objeto vivo — o núcleo compartilhado dos leitores e escritores por
/// nome. O endereço respeita o `inValueHolder` (campo de classe scripted mora no `valueHolder`, não no objeto); 0 quando ilegível.
pub(crate) unsafe fn prop_em(obj: *mut c_void, nome: &str) -> Option<(String, usize, usize)> {
    if obj.is_null() || !rtti::sane(obj) {
        return None;
    }
    let cls = rtti::class_of(obj);
    let off = prop_value_offset(cls, nome)?;
    let prop = rtti::find_property_in_class(cls, nome);
    let alvo = rtti::field_ptr(obj, off as u32, rtti::prop_in_holder(prop)) as usize;
    Some((crate::cname::resolve_cname(rtti::type_name_getname(cls)), off, alvo))
}

// ===== Materiais e aparências de um `CMesh`.
//
// O item pede 3 grupos de campos: `CMaterialInstance`(5), `CMeshMaterialEntry`(4) e
// `meshMeshAppearance`(3). A ficha registra CMesh-trap nos três (zero ocorrência no corpus) e a
// nota diz "não tentar `@addField`". Correto — pra DECLARAR. Aqui nada é declarado: o `CMesh*`
// vem do depot (cadeia do `#18`/`#30`) e os campos saem do RTTI desta build.
//
// **Nenhum offset de header é usado.** Depois do `#30` — onde o header do SDK errou por 8 bytes
// justamente num campo precedido de `unkNN[...]` — a única fonte confiável é `CProperty::
// valueOffset`. Os headers ficam como referência de NOME de campo, não de posição.

#[cfg(test)]
mod widgetinputservice_tests {
    use crate::entrada::{is_character_input_key, to_character_key};

    #[test]
    fn letters_digits_space_are_character_input() {
        assert!(is_character_input_key(32)); // Space
        assert!(is_character_input_key(48)); // '0'
        assert!(is_character_input_key(57)); // '9'
        assert!(is_character_input_key(65)); // 'A'
        assert!(is_character_input_key(90)); // 'Z'
        assert!(!is_character_input_key(0)); // IK_None
        assert!(!is_character_input_key(27)); // IK_Escape
        assert!(!is_character_input_key(136)); // IK_Pad_A_CROSS (gamepad, não char)
    }

    #[test]
    fn punctuation_and_numpad_ranges_are_character_input() {
        assert!(is_character_input_key(186)); // Semicolon
        assert!(is_character_input_key(192)); // Tilde
        assert!(is_character_input_key(219)); // LeftBracket
        assert!(is_character_input_key(222)); // SingleQuote
        assert!(is_character_input_key(96)); // NumPad0
        assert!(is_character_input_key(111)); // NumSlash
        assert!(!is_character_input_key(193)); // Mouse4, fora dos 2 ranges
    }

    #[test]
    fn to_character_matches_ascii_for_letters_digits_space() {
        assert_eq!(to_character_key(32, false), " ");
        assert_eq!(to_character_key(48, false), "0");
        assert_eq!(to_character_key(57, false), "9");
        assert_eq!(to_character_key(65, false), "a");
        assert_eq!(to_character_key(90, false), "z");
        assert_eq!(to_character_key(65, true), "A");
        assert_eq!(to_character_key(90, true), "Z");
    }

    #[test]
    fn to_character_shifted_digits_give_symbols() {
        assert_eq!(to_character_key(49, true), "!"); // Shift+1
        assert_eq!(to_character_key(48, true), ")"); // Shift+0
        assert_eq!(to_character_key(56, true), "*"); // Shift+8
    }

    #[test]
    fn to_character_punctuation_shifted_and_unshifted() {
        assert_eq!(to_character_key(186, false), ";");
        assert_eq!(to_character_key(186, true), ":");
        assert_eq!(to_character_key(191, false), "/");
        assert_eq!(to_character_key(191, true), "?");
        assert_eq!(to_character_key(192, false), "`");
        assert_eq!(to_character_key(192, true), "~");
    }

    #[test]
    fn to_character_non_character_key_is_empty() {
        assert_eq!(to_character_key(27, false), ""); // Escape
        assert_eq!(to_character_key(0, false), ""); // None
    }
}

// RTTI #168/#169/#232/#236: valor de campo de qualquer tipo pelo próprio tipo do campo. `Assign` copia (String, handle, array, struct),
// `IsEqual` compara e `ToString` formata, pela vtable do IType — o que o motor faz no `=`, no `==` e no `ToString` dos scripts. Os acessores
// por tipo acima (Float, Int32, Bool, CName) ficam pros casos simples.

// ===== Marshalling de Variant: BwmsVariantLog(a: Variant, b: Variant) -> Void =====
// Proto dos params: param[1] de gameIBlackboard::SetVariant (único lugar no motor onde um
// método nativo aceita Variant como argumento — dá o CProperty correto sem GetType("Variant")).
// Handler captura 24 bytes brutos de cada Variant ANTES do type_inst_free (inline-safe p/ Int32/
// Float/Bool/CName/TweakDBID), extrai tipo via GetName() do IType e o qword de dados inline.

/// Igual a `vtbl_slot_fn`, mas RECUSA slot além do fim REAL da vtable do objeto.
///
/// **Defeito real que isto corrige (achado 2026-09-04, presente na 0.1.4 empacotada).** As natives
/// `BwmsMappinSystemSetPoiPhase`/`GetPoiPhase` usam offsets derivados por analogia com Windows
/// (`0x1D0`/`0x3C8` + shift Itanium `0x08`) = slots **59** e **122**. Medindo a vtable REAL da
/// `MappinSystem` no objeto vivo: **35 slots**, fim em `0x118`. Os dois estão FORA.
/// `GetPoiPhase` só devolvia lixo (medido: valores crescendo `0xC0` por chamada); mas
/// `SetPoiPhase` **chamava um ponteiro arbitrário com 2 argumentos** — um mod que usasse a API
/// exposta no pacote público quebraria o jogo do usuário.
///
/// `vtbl_slot_fn` sozinho não protege: ele só checa se o alvo é LEGÍVEL, e qualquer endereço dentro
/// do binário passa nessa checagem. A proteção tem de vir do TAMANHO da tabela.
pub(crate) unsafe fn vtbl_slot_fn_bounded(obj: *mut c_void, slot_off: usize) -> Option<*const c_void> {
    let n = vtable_slot_count_cached(obj);
    if n == 0 || slot_off + 8 > n * 8 {
        crate::log_dev!(
            "[vtbl] RECUSADO: slot +{slot_off:#x} está fora da vtable de {n} slots (fim {:#x}) — chamada abortada",
            n * 8
        );
        return None;
    }
    vtbl_slot_fn(obj, slot_off)
}

/// Lê `vtbl_ptr+slot_off` como ponteiro de função, validando legibilidade em cada passo
/// (objeto -> vtable -> slot -> function pointer). `None` em qualquer falha — nunca
/// dereferencia um ponteiro não confirmado legível. Serve a qualquer objeto C++ com vtable no
/// offset 0; o `slot_off` é medido na vtable do Mac, não derivado do Windows (ver `vtbl_slot_fn_bounded`).
pub(crate) unsafe fn vtbl_slot_fn(obj: *mut c_void, slot_off: usize) -> Option<*const c_void> {
    if !rtti::sane(obj) || !crate::memoria::is_readable(obj as *const c_void, 8) {
        return None;
    }
    let vtbl = (obj as *const u64).read_unaligned() as *const u8;
    if vtbl.is_null() || !crate::memoria::is_readable(vtbl as *const c_void, slot_off + 8) {
        return None;
    }
    let slot_ptr = vtbl.add(slot_off) as *const u64;
    if !crate::memoria::is_readable(slot_ptr as *const c_void, 8) {
        return None;
    }
    let fp = slot_ptr.read_unaligned() as *const c_void;
    if fp.is_null() || !crate::memoria::is_readable(fp, 4) {
        return None;
    }
    Some(fp)
}

// ---------------------------------------------------------------------------------------------
// Continuação — tentativa de `trocar_slot_da_vtable` REAL no slot ÚNICO `OnGameRestored`
// (mac=0x158, evento real 'Session/Ready'), seguindo a recomendação exata deixada pela rodada
// anterior no catálogo (só slots com endereço único, nunca ICF-compartilhado; probe
// observe-only, sempre repassa pro original, nunca suprime). Ver a nota grande no `.reds` de
// teste do hook de `OnGameRestored` em `blackwall-mods-dev/`.
//
// ABI: `bool OnGameRestored()` — só `this`, sem outros argumentos, retorna bool. Escolhido
// em vez de `OnGameLoad` (mac=0x150) porque aquele tem ABI mais arriscada: `void
// OnGameLoad(const Red::JobGroup&, bool& aSuccess, void* aStream)`, com um out-param que
// controla se o load do jogo é considerado bem-sucedido — escrever esse campo errado poderia
// falhar o carregamento do save de verdade. `OnGameRestored` não tem esse risco.
//
// A vtable patcheada é a vtable de CLASSE de `MappinSystem` (compartilhada por TODAS as
// instâncias da mesma classe C++, jamais por outras classes — cada classe C++ distinta tem sua
// própria vtable; a única exceção seria ICF de VTABLE INTEIRA entre 2 classes byte-idênticas,
// que não é o caso aqui pois os 5 slots "únicos" têm corpo de função real e não-trivial,
// confirmado no proof da rodada anterior). Hookar UMA VEZ, cedo (1ª firing de OnGameAttached,
// ainda no player-espúrio do menu, mesmo timing já provado seguro pelo dump read-only acima) já
// cobre qualquer instância futura de `MappinSystem` — inclusive a que processar o
// `OnGameRestored` real durante o autocontinue, que é o evento que este probe quer capturar.
// ---------------------------------------------------------------------------------------------

// ---------------------------------------------------------------------------------------------
// Continuação 2 (2026-08-18, madrugada) — `trocar_slot_da_vtable` no slot `OnWorldAttached`
// (mac=0x118, evento real 'Session/BeforeStart'), seguindo a recomendação exata deixada pela
// rodada de 2026-08-16 (`HISTORICO.md`/catálogo, item `#117`): "testar um slot que dispare mais
// tarde no ciclo de vida (`OnWorldAttached`, que semanticamente deveria disparar quando o mundo é
// montado pós-load, mais perto da janela em que nosso `register_all()` já rodou)" — via NUNCA
// tentada até agora (só `OnGameRestored` foi testado, confirmado NUNCA disparar mesmo com
// breakpoint real via lldb, 2026-08-16 rodada 54).
//
// ABI: `virtual void OnWorldAttached(world::RuntimeScene* aScene);` — `this` + 1 ponteiro extra
// (a cena), retorna void. `OnWorldAttached` é 1 dos 5 slots com endereço ÚNICO/não-ICF-colapsado
// confirmados na rodada de 2026-08-16 (prólogo real `SUB SP,SP,#0xB0`, fn=0x106398e80 naquele
// boot) — categoria estruturalmente mais segura que os 6 slots ICF-compartilhados (nunca
// escolhidos pra hook, mesmo raciocínio já documentado pro `OnGameRestored`).
//
// 100% OBSERVE-ONLY no núcleo (chama original com os MESMOS args, nunca suprime/muda retorno,
// mesmo idioma já provado 7800+ vezes pelo `AnimationSystem_FrameBeginReset`) + o mesmo padrão
// bounded/gated (`has_listener_for` antes de `fire_event_args`) já DECISIVO pro fechamento do
// item `#127` (`IComponent::Toggle`) nesta mesma madrugada — só dispara "Session/BeforeStart" (o
// evento que corresponde a este slot) depois de confirmar um listener de verdade, bounded a 3 chamadas.
// ---------------------------------------------------------------------------------------------

// ---------------------------------------------------------------------------------------------
// `JournalExtension` (2026-08-14) — varredura sistemática dos 86 acessores
// `GameInstance.GetXSystem`/`GetXManager` (extraídos de `orphans.script`) cruzada contra os itens
// REAL_GAP do catálogo. Achado real: 3 dos hooks nunca tentados do item (`OnMappinDataLoaded`/
// `OnGetMappinData`/`OnGetPoiData`) recebem `aMappinSystem` como parâmetro — a MESMA classe
// `MappinSystem` já acessível neste projeto via `GameInstance.GetMappinSystem(game) ->
// ref<MappinSystem>` (native vanilla real, provado ao vivo desde 2026-08-11, reusada pelo
// `SetPoiMappinPhase` acima).
//
// A resolução de mappin cozido (`ResolveCookedMappin`) usa 2 campos de DADO crus do
// `MappinSystem` (NÃO `RawFunc`/`AddressLib`): `CookedMappinResource` em `+0x58`
// (`Handle<gameMappinResource>`) e `CookedPoiResource` em `+0x68`
// (`Handle<gamePointOfInterestMappinResource>`). `Handle<T>` tem
// 16 bytes, `instance: T*`@+0x0/`refCount`@+0x8 — ler só os
// 8 bytes de `instance` dá o ponteiro cru do recurso cacheado. Os campos DENTRO do recurso
// (`cookedData`/`cookedMultiData`, `DynArray<T>` = 16 bytes: `entries`@+0x0/`capacity`@+0x8/
// `size`@+0xC) têm offset da reflection real do jogo; o total do objeto (`MappinResource`=0x70,
// `PointOfInterestMappinResource`=0x50) bate com onde os campos cabem.
//
// Zero endereço `AddressLib` novo, zero shift Itanium (campos de DADO puros, não vtable — regra
// já estabelecida repetidas vezes neste projeto). Escopo HONESTO: isto expõe só LEITURA de
// contagem (quantos mappins/multi-mappins/POIs o jogo já tem cacheados agora) — não implementa a
// capacidade de ESCRITA/injeção que é o coração real do item (precisaria hookar
// `MappinSystem::GetMappinData`/`GetPoiData`, ambos `RawFunc`/`AddressLib`, categoria bloqueada —
// mesma classe do WeatherSystem, RE de endereço Mac nunca feita; e o MERGE de
// journal/`FindEntry`/`MergeEntries` é lógica sobre `JournalEntry*`, sem
// dependência de sistema nenhum). O item PERMANECE REAL_GAP (regra de ouro) — isto é uma
// sub-capacidade nova, nunca antes lida por este projeto, genuinamente ligada ao tema "resolução
// de mappin" do item, alcançada via composição pura sem nenhuma RE nova.
// ---------------------------------------------------------------------------------------------

pub(crate) unsafe fn entregar_handle(out: *mut c_void, h: [u64; 2]) {
    if out.is_null() {
        return;
    }
    core::ptr::write_unaligned(out as *mut u64, h[0]);
    core::ptr::write_unaligned((out as *mut u64).add(1), h[1]);
}

// Parte 7d-2b-3b (2026-09-12): volume calculado do nó de área contra o cozido. Instância pelo `FindNode`; nó IsA `worldAreaShapeNode`
// (subclasses de área herdam o campo) → `outline` Handle `+0x40` → `points` DynArray<Vector3> `+0x30`, `height` `+0x40`; cada ponto escalado e girado pela
// orientação da instância, guardando X e Y. Altura: a forma escalada multiplica pela escala Z, mas no boot 76 essa forma divergiu do cozido em 98 de 221 nós, só
// na altura (pontos iguais com 2e-6); o cozido vanilla guarda a altura do outline sem escala, que é a forma usada aqui, com a escalada como controle.

#[cfg(test)]
mod mappin_cooked_count_tests {
    use super::mappin_cooked_count_from_lens;

    #[test]
    fn none_means_uncached_sentinel_minus_one() {
        assert_eq!(mappin_cooked_count_from_lens(None), -1);
    }

    #[test]
    fn cached_but_empty_is_zero_not_sentinel() {
        assert_eq!(mappin_cooked_count_from_lens(Some((0, 0))), 0);
    }

    #[test]
    fn cached_with_entries_returns_size_not_capacity() {
        // size != capacity é o caso normal (array com folga reservada) — a função deve devolver
        // `size` (entradas REAIS), nunca `capacity` (espaço alocado).
        assert_eq!(mappin_cooked_count_from_lens(Some((7, 32))), 7);
    }

    #[test]
    fn large_size_fits_i32_without_overflow() {
        // `size` é u32; valores plausíveis de contagem de mappins nunca chegam perto de
        // i32::MAX, mas a conversão deve ser exata (sem saturar/errar) na faixa normal.
        assert_eq!(mappin_cooked_count_from_lens(Some((100_000, 100_000))), 100_000);
    }
}

/// Solta uma referência fraca sem levá-la a zero (liberar o bloco é do motor).
pub(crate) unsafe fn soltar_fraca(rc: u64) {
    let fraca = &*((rc + 4) as *const std::sync::atomic::AtomicU32);
    let mut v = fraca.load(Ordering::SeqCst);
    while v > 1 {
        match fraca.compare_exchange_weak(v, v - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return,
            Err(x) => v = x,
        }
    }
}

/// Valores de um `DynArray` de elementos de 8 B (`CName`, `EntityID`, ponteiro) em `arr`.
pub(crate) unsafe fn dynarray_u64(arr: u64) -> Vec<u64> {
    let (entradas, tam) = (eb_u64(arr, 0), (eb_u64(arr, 0x08) >> 32) as usize);
    if entradas == 0 || tam > 100_000 || !crate::memoria::is_readable(entradas as *const c_void, tam * 8) {
        return Vec::new();
    }
    (0..tam).map(|i| eb_u64(entradas, (i * 8) as u64)).collect()
}

#[cfg(test)]
mod weathersystem_state_tests {
    use super::*;

    fn mkbuf(idx: u32, src: u64, prev: u32) -> [u8; WEATHER_RUNTIME_CHUNK_LEN] {
        let mut b = [0u8; WEATHER_RUNTIME_CHUNK_LEN];
        b[WEATHER_CURRENT_STATE_INDEX_OFF..WEATHER_CURRENT_STATE_INDEX_OFF + 4].copy_from_slice(&idx.to_le_bytes());
        b[WEATHER_CURRENT_SOURCE_OFF..WEATHER_CURRENT_SOURCE_OFF + 8].copy_from_slice(&src.to_le_bytes());
        b[WEATHER_PREVIOUS_STATE_INDEX_OFF..WEATHER_PREVIOUS_STATE_INDEX_OFF + 4].copy_from_slice(&prev.to_le_bytes());
        b
    }

    #[test]
    fn decodes_all_three_fields_at_real_offsets() {
        let buf = mkbuf(3, 0xDEAD_BEEF_1234_5678, 2);
        let (idx, src, prev) = decode_weather_runtime_state(&buf).expect("deve decodificar");
        assert_eq!(idx, 3);
        assert_eq!(src, 0xDEAD_BEEF_1234_5678);
        assert_eq!(prev, 2);
    }

    #[test]
    fn zeroed_buffer_decodes_to_zero_not_garbage() {
        let buf = [0u8; WEATHER_RUNTIME_CHUNK_LEN];
        let (idx, src, prev) = decode_weather_runtime_state(&buf).expect("deve decodificar");
        assert_eq!((idx, src, prev), (0, 0, 0));
    }

    #[test]
    fn short_buffer_rejected_never_panics() {
        let short = [0u8; 4];
        assert_eq!(decode_weather_runtime_state(&short), None);
    }

    #[test]
    fn offsets_do_not_overlap_and_fit_chunk_len() {
        // Confirma que os 3 campos (com seus tamanhos) cabem no chunk e não se sobrepõem —
        // trava de regressão caso alguém edite um offset sem atualizar o comprimento do chunk.
        assert!(WEATHER_CURRENT_STATE_INDEX_OFF + 4 <= WEATHER_CURRENT_SOURCE_OFF);
        assert!(WEATHER_CURRENT_SOURCE_OFF + 8 <= WEATHER_PREVIOUS_STATE_INDEX_OFF);
        assert!(WEATHER_PREVIOUS_STATE_INDEX_OFF + 4 <= WEATHER_RUNTIME_CHUNK_LEN);
    }
}
// ---------------------------------------------------------------------------------------------

// -------------------------------------------------------------------------------------------
// `QuestPhaseGraphAccessor` — 2026-08-15, sessão dedicada de varredura de
// itens "composição pura sobre native vanilla real, zero endereço nativo necessário" ainda
// sem código. A API é ~15 métodos de filtro
// sobre `questGraphDefinition.nodes` (cast de tipo + iteração de
// `DynArray`) — nenhum precisa de endereço nativo. Generalizo aqui o construto-base que a maioria
// dos 15 métodos reusa (filtro de nós por tipo), em vez de 1 native por método — cobre `FindInputNode`/
// `FindManagerEvents`/`FindJournalEntries` (filtro direto, sem Cast aninhado em `.actions`/
// `.condition`) e serve de base pra completar o resto numa sessão futura.
//
// Offsets, TODOS campo-de-dado (zero shift Itanium, "Mac=Windows sem shift" — regra já
// estabelecida no projeto pra `Core::OffsetPtr`, ao contrário de `RawVFunc`):
//   - `QuestsSystem` raiz da fase em `+0x68` (`Handle<questPhaseInstance>`) — MESMO offset já
//     usado com sucesso pelas investigações de `#77`/`#84`/`#157` desta mesma sessão.
//   - `QuestPhaseInstance` grafo em `+0x40` (`Handle<questGraphDefinition>`), mesma vizinhança.
//   - `graphGraphDefinition.nodes @ +0x30` — `DynArray<Handle<graph::GraphNodeDefinition>>`,
//     da reflection real do jogo.
//
// Disciplina anti-CMesh-trap: os ~11 tipos concretos de nó (`questInputNodeDefinition`/
// `questEventManagerNodeDefinition`/`questJournalNodeDefinition`/etc.) têm ZERO ocorrência em
// `redscript-src` — declarar a cadeia de ancestralidade inteira (`questNodeDefinition` →
// `graphGraphNodeDefinition` → `graphIGraphObjectDefinition` → `ISerializable`, mais os 2
// ramos intermediários pros 3 tipos-folha) às cegas seria a MESMA categoria de risco que já
// crashou o `#30`/CMesh (bind RTTI recusa destrutivamente se a ancestralidade declarada não
// bate com a real). Em vez disso, TODO retorno de handle de nó é `ref<IScriptable>` (base
// vanilla real, sempre segura) — mesma disciplina já usada pro `#100`/`GetLayer` — o
// mod-author confirma o tipo concreto via `GetClassName()` (mesmo padrão que fechou aquele
// item). Filtro por classe usa `type_hash: CName` passado pelo REDSCRIPT (o mod-author escreve
// `n"questInputNodeDefinition"` etc.) — zero hardcode nosso do nome, zero declaração de tipo.

/// Aplicação de attachment / itens #40, #41, #54 (2026-08-19, continuação direta da manhã): dump
/// SOB DEMANDA (não gated a um firing do hook `EquipStart`) do array `unk90`+0xB0 do componente
/// `gameAttachmentSlots` do player — permite reler o estado ATUAL do array (ex. pós-`give`+
/// `equiprawv7`) sem precisar esperar um novo evento nativo. Mesma travessia de
/// `run_enum_skinnedmesh` (componentes reais em `player+0xA0`), procurando a classe
/// `gameAttachmentSlots` (nome RTTI real do header oficial, `game::AttachmentSlots::NAME`).
/// Confirma identidade por 2 sinais independentes: `class_by_name` bater E `IComponent::name`
/// (CName@+0x40) == `fnv1a64("AttachmentSlots")` — mesmo golden já usado no probe do hook.
/// Acha a instância REAL `gameAttachmentSlots` do `player` (mesma travessia de componentes já
/// usada por `run_enum_skinnedmesh`/`scan_attachslots_now`) — extraído como helper reusável pro
/// item #40/#41/#54 (composição pura de `IsSlotEmpty`/`IsSlotSpawning`, ver
/// [`attachslot_check_by_id`], em `diagnostico`). Confirma identidade por 2 sinais: `class_by_name` bater
/// E `IComponent::name`(CName@+0x40) == `fnv1a64("AttachmentSlots")`.
pub(crate) unsafe fn find_attachslots_component(reg: &crate::rtti::Registry, player: *mut c_void) -> Option<*mut c_void> {
    if player.is_null() || !crate::memoria::is_readable(player as *const c_void, 0x10) {
        return None;
    }
    let mut target_cls = reg.class_by_name("gameAttachmentSlots");
    if target_cls.is_null() {
        target_cls = reg.class_by_name("AttachmentSlots");
    }
    if target_cls.is_null() {
        return None;
    }
    let arr = (player as *mut u8).add(0xA0) as *mut c_void;
    if !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return None;
    }
    let entries = rd_u64(arr, 0x00) as *const u8;
    let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    if entries.is_null() || size == 0 || size > 4096 {
        return None;
    }
    for i in 0..size as usize {
        let slot = entries.add(i * 16);
        if !crate::memoria::is_readable(slot as *const c_void, 16) {
            continue;
        }
        let inst = core::ptr::read_unaligned(slot as *const u64) as *mut c_void;
        if inst.is_null() {
            continue;
        }
        if !class_is_kind_of(crate::rtti::class_of(inst), target_cls) || !crate::memoria::is_readable(inst as *const c_void, 0x120) {
            continue;
        }
        let name_hash = (inst as *const u8).add(0x40).cast::<u64>().read_unaligned();
        if name_hash != bwms_primitives::fnv1a64(b"AttachmentSlots") {
            continue;
        }
        return Some(inst);
    }
    None
}



#[cfg(test)]
mod componentwrapper_offset_tests {
    use super::*;

    /// O offset vem do RTTI da classe real; o que é fixo é o NOME da property por tipo.
    #[test]
    fn nomes_de_property_por_tipo() {
        assert_eq!(nome_prop_path(GarmentComponentKind::MeshComponent), Some("mesh"));
        assert_eq!(nome_prop_path(GarmentComponentKind::SkinnedFamily), Some("mesh"));
        assert_eq!(nome_prop_path(GarmentComponentKind::MorphTarget), Some("morphResource"));
        for k in [GarmentComponentKind::MeshComponent, GarmentComponentKind::SkinnedFamily, GarmentComponentKind::MorphTarget] {
            assert_eq!(nome_prop_aparencia(k), Some("meshAppearance"));
            assert_eq!(nome_prop_chunk_mask(k), Some("chunkMask"));
        }
    }

    #[test]
    fn unsupported_nao_tem_property() {
        assert_eq!(nome_prop_path(GarmentComponentKind::Unsupported), None);
        assert_eq!(nome_prop_aparencia(GarmentComponentKind::Unsupported), None);
        assert_eq!(nome_prop_chunk_mask(GarmentComponentKind::Unsupported), None);
    }

    const D: u64 = CNAME_DEFAULT;

    #[test]
    fn find_appearance_casa_nome_exato_e_pula_entrada_vazia() {
        let e = [(0, 7), (0x10, 5), (0x20, 7)];
        assert_eq!(escolher_aparencia(&e, 7, D), Some(2));
        assert_eq!(escolher_aparencia(&e, 5, D), Some(1));
    }

    #[test]
    fn find_appearance_padrao_e_none_caem_na_primeira() {
        let e = [(0x10, 5), (0x20, 6)];
        assert_eq!(escolher_aparencia(&e, D, D), Some(0));
        assert_eq!(escolher_aparencia(&e, 0, D), Some(0));
        assert_eq!(escolher_aparencia(&e, 99, D), None);
        assert_eq!(escolher_aparencia(&[], D, D), None);
    }

    #[test]
    fn find_appearance_padrao_com_aparencia_propria_casa_exato() {
        let e = [(0x10, 5), (0x20, D)];
        assert_eq!(escolher_aparencia(&e, D, D), Some(1));
    }

    #[test]
    fn resolver_cai_no_padrao_quando_nao_acha() {
        let e = [(0x10, 5), (0x20, 6)];
        assert_eq!(resolver_aparencia(&e, 99, D), Some(0));
        assert_eq!(resolver_aparencia(&e, 6, D), Some(1));
        assert_eq!(resolver_aparencia(&[], 99, D), None);
    }
}

// ===== Leitores de `AppearanceDefinition` e helpers de `DynArray`/trava de `AppearanceResource` =====
// Overrides de aparência e de máscara ficam no `ComponentState` de `es20_componentes` e a parte
// ligada em `ES22_COMPONENT_APPEARANCE` (`aparencia`), com chave dupla `(entity_ptr, nome base)`.

/// Caminhos de `partsValues` da definição.
pub(crate) unsafe fn es20_partes(definicao: *const u8) -> Vec<u64> {
    let arr = definicao.add(crate::jogo::aparencia::APPDEF_PARTSVALUES_OFF);
    let b = rd_u64(arr as *const c_void, 0) as *const u64;
    let n = if b.is_null() { 0 } else { (arr.add(0x0c) as *const u32).read_unaligned() as usize };
    (0..n.min(4096)).map(|k| b.add(k).read_unaligned()).collect()
}

/// Offsets de `componentName`, `meshAppearance` e `chunkMask` em `appearancePartComponentOverrides` (0x50 B), pelo RTTI.
pub(crate) unsafe fn es20_campos_de_componente(reg: &rtti::Registry) -> Option<[usize; 3]> {
    let c = reg.class_by_name("appearancePartComponentOverrides");
    if c.is_null() {
        return None;
    }
    let campo = |n: &str| rtti::resolve_prop_in_class(c, n).map(|(o, _, _)| o as usize).filter(|o| *o + 8 <= 0x50);
    Some([campo("componentName")?, campo("meshAppearance")?, campo("chunkMask")?])
}

/// Vaga zerada no índice `tamanho` de um `DynArray` (cabeçalho em `arr`, elementos de `passo` B), crescendo o buffer se está cheio: array vazio pelo reserve
/// do motor, com elementos pelo `dynarray_crescer` (o buffer antigo fica, pra quem ainda lê). O tamanho não muda; quem grava a vaga publica o tamanho depois.
pub(crate) unsafe fn es22_vaga_no_fim(arr: *mut u8, passo: usize) -> Option<*mut u8> {
    let tamanho = |a: *mut u8| ((a.add(0x08) as *const u32).read_unaligned(), (a.add(0x0c) as *const u32).read_unaligned());
    let (cap, tam) = tamanho(arr);
    let entradas = rd_u64(arr as *const c_void, 0) as *mut u8;
    if entradas.is_null() || cap == 0 {
        crate::jogo::listas::reservar(arr as *mut c_void, 4, passo as u32, 8, std::ptr::null());
    } else if tam >= cap {
        let (novo, nova_cap) = crate::jogo::listas::dynarray_crescer(entradas, cap, tam, passo)?;
        (arr as *mut u64).write_unaligned(novo as u64);
        (arr.add(0x08) as *mut u32).write_unaligned(nova_cap);
    }
    let (cap, tam) = tamanho(arr);
    let buffer = rd_u64(arr as *const c_void, 0) as *mut u8;
    if buffer.is_null() || tam >= cap {
        return None;
    }
    let vaga = buffer.add(tam as usize * passo);
    std::ptr::write_bytes(vaga, 0, passo);
    Some(vaga)
}

/// Trava exclusiva `+0xF0` do recurso durante `f`.
pub(crate) unsafe fn es22_com_trava<T>(recurso: *mut u8, f: impl FnOnce() -> T) -> T {
    let trava = recurso.add(ES22_RECURSO_TRAVA);
    crate::jogo::travas::travar_escrita((trava as *mut c_void) as *mut u8);
    let r = f();
    crate::jogo::travas::soltar_escrita((trava) as *mut u8);
    r
}

/// `appearances.PushBack(h)` com a trava exclusiva já tomada.
pub(crate) unsafe fn es22_anexar_definicao_travado(recurso: *mut u8, h: [u64; 2]) -> bool {
    let arr = recurso.add(crate::jogo::aparencia::APPRESOURCE_APPEARANCES_OFF);
    let Some(vaga) = es22_vaga_no_fim(arr, 16) else { return false };
    handle_strong_soma(&h, 1);
    (vaga as *mut u64).write_unaligned(h[0]);
    (vaga.add(8) as *mut u64).write_unaligned(h[1]);
    let tam = (arr.add(0x0c) as *const u32).read_unaligned();
    (arr.add(0x0c) as *mut u32).write_unaligned(tam + 1);
    true
}

/// Nova instância da classe de `modelo` com o nome `nome` em `+0x30`, com cada propriedade copiada de `modelo` quando `copiar`.
pub(crate) unsafe fn es22_definicao_nova(modelo: *mut c_void, nome: u64, copiar: bool) -> Option<[u64; 2]> {
    let cls = rtti::class_of(modelo);
    let novo = if rtti::sane(cls) { rtti::create_instance(cls) } else { std::ptr::null_mut() };
    if novo.is_null() {
        return None;
    }
    let mut h = [0u64; 2];
    rtti::make_handle(h.as_mut_ptr() as *mut c_void, novo);
    if copiar {
        for (tipo, off, no_holder) in rtti::propriedades_com_holder(cls) {
            let (origem, destino) = (rtti::field_ptr(modelo, off, no_holder), rtti::field_ptr(novo, off, no_holder));
            if !origem.is_null() && !destino.is_null() {
                rtti::type_assign(tipo, destino, origem);
            }
        }
    }
    ((novo as usize + crate::jogo::aparencia::APPDEF_NAME_OFF) as *mut u64).write_unaligned(nome);
    Some(h)
}


#[cfg(test)]
mod iserializable_postload_tests {
    use super::*;

    /// Guard de segurança: ponteiro obviamente inválido (null / abaixo do teto `sane()`) tem
    /// que voltar `false` SEM tocar memória nenhuma — testável offline, sem processo do jogo,
    /// porque `sane()` rejeita antes de qualquer leitura de vtable.
    #[test]
    fn rejects_null_and_low_pointers_without_touching_memory() {
        unsafe {
            assert!(!rtti::iserializable_process_post_load(std::ptr::null_mut(), false));
            assert!(!rtti::iserializable_process_post_load(0x1 as *mut c_void, false));
            assert!(!rtti::iserializable_process_post_load(0x1000 as *mut c_void, true));
        }
    }
}

#[cfg(test)]
mod vehicleobject_telemetry_tests {
    use super::*;

    fn mkbuf(g: bool, a: f32, d: f32, r: bool, b: f32) -> [u8; VEHICLEOBJECT_CHUNK_LEN] {
        let mut buf = [0u8; VEHICLEOBJECT_CHUNK_LEN];
        buf[VEHICLEOBJECT_IS_ON_GROUND_OFF] = g as u8;
        buf[VEHICLEOBJECT_ACCELERATION_OFF..VEHICLEOBJECT_ACCELERATION_OFF + 4].copy_from_slice(&a.to_le_bytes());
        buf[VEHICLEOBJECT_DECELERATION_OFF..VEHICLEOBJECT_DECELERATION_OFF + 4].copy_from_slice(&d.to_le_bytes());
        buf[VEHICLEOBJECT_IS_REVERSING_OFF] = r as u8;
        buf[VEHICLEOBJECT_BURNOUT_OFF..VEHICLEOBJECT_BURNOUT_OFF + 4].copy_from_slice(&b.to_le_bytes());
        buf
    }

    #[test]
    fn decodes_all_five_fields_at_real_offsets() {
        let buf = mkbuf(true, 3.5, 9.0, false, 42.25);
        let (g, a, d, r, b) = decode_vehicleobject_telemetry(&buf).expect("deve decodificar");
        assert_eq!(g, true);
        assert_eq!(a, 3.5);
        assert_eq!(d, 9.0);
        assert_eq!(r, false);
        assert_eq!(b, 42.25);
    }

    #[test]
    fn zeroed_buffer_decodes_to_false_and_zero_not_garbage() {
        let buf = [0u8; VEHICLEOBJECT_CHUNK_LEN];
        let (g, a, d, r, b) = decode_vehicleobject_telemetry(&buf).expect("deve decodificar");
        assert_eq!((g, a, d, r, b), (false, 0.0, 0.0, false, 0.0));
    }

    #[test]
    fn short_buffer_rejected_never_panics() {
        let short = [0u8; 16];
        assert_eq!(decode_vehicleobject_telemetry(&short), None);
    }

    #[test]
    fn offsets_batem_com_o_layout_e_nao_se_sobrepoem() {
        // Offsets do Mac 2.5: 4 iguais aos do Windows e o `burnout` +8, medido no construtor do veículo.
        assert_eq!(VEHICLEOBJECT_IS_ON_GROUND_OFF, 0x25C);
        assert_eq!(VEHICLEOBJECT_ACCELERATION_OFF, 0x264);
        assert_eq!(VEHICLEOBJECT_DECELERATION_OFF, 0x268);
        assert_eq!(VEHICLEOBJECT_IS_REVERSING_OFF, 0x2A3);
        assert_eq!(VEHICLEOBJECT_BURNOUT_OFF, 0x2C4);
        assert!(VEHICLEOBJECT_IS_ON_GROUND_OFF + 1 <= VEHICLEOBJECT_ACCELERATION_OFF);
        assert!(VEHICLEOBJECT_ACCELERATION_OFF + 4 <= VEHICLEOBJECT_DECELERATION_OFF);
        assert!(VEHICLEOBJECT_DECELERATION_OFF + 4 <= VEHICLEOBJECT_IS_REVERSING_OFF);
        assert!(VEHICLEOBJECT_IS_REVERSING_OFF + 1 <= VEHICLEOBJECT_BURNOUT_OFF);
        assert!(VEHICLEOBJECT_BURNOUT_OFF + 4 <= VEHICLEOBJECT_CHUNK_LEN);
    }
}

/// `(entries, cap, tamanho)` do `DynArray` em `base+off`.
pub(crate) unsafe fn handles_arr_em(base: *const u8, off: usize) -> (*mut u64, u32, u32) {
    let arr = base.add(off);
    ((arr as *const u64).read_unaligned() as *mut u64, (arr.add(0x08) as *const u32).read_unaligned(), (arr.add(0x0C) as *const u32).read_unaligned())
}

/// Soma `delta` no strong do bloco de um handle `{instância, bloco}`: só pra cópias que a própria native cria e solta (nunca chega a 0 aqui).
pub(crate) unsafe fn handle_strong_soma(h: &[u64; 2], delta: i32) {
    if h[1] == 0 {
        return;
    }
    let a = &*(h[1] as *const std::sync::atomic::AtomicU32);
    if delta >= 0 {
        a.fetch_add(delta as u32, std::sync::atomic::Ordering::SeqCst);
    } else {
        a.fetch_sub(delta.unsigned_abs(), std::sync::atomic::Ordering::SeqCst);
    }
}

/// Hash do GlobalNodeRef que o `ResolveNodeRef` vanilla devolve pro NodeRef na raiz global (`GlobalNodeID.GetRoot()`), chamado pela VM na game thread.
pub(crate) unsafe fn resolver_noderef(reg: &Registry, noderef: u64) -> Option<u64> {
    let raiz_f = rtti::resolve_any(reg, &["GlobalNodeID", "worldGlobalNodeID"], "GetRoot")?;
    let raiz = u64::from_le_bytes(rtti::call_func(&raiz_f, std::ptr::null_mut(), &[])?[..8].try_into().ok()?);
    let f = rtti::resolve_global_function_robust(reg, "ResolveNodeRef");
    if f.is_null() {
        return None;
    }
    let rf = rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
    let b = rtti::call_func(&rf, std::ptr::null_mut(), &[rtti::Arg::I64(noderef), rtti::Arg::I64(raiz)])?;
    Some(u64::from_le_bytes(b[..8].try_into().ok()?))
}

pub(crate) unsafe fn f32x3(b: *const u8, off: usize) -> [f32; 3] {
    [0, 4, 8].map(|d| (b.add(off + d) as *const f32).read_unaligned())
}

// Parte 7d-2b-2a (2026-09-12): acesso por índice aos pins da coleta, pro script conferir a posição cozida contra a do nó do NodeRef
// (`ResolveNodeRef` + `GameInstance.GetNodeTransform`, natives de script vanilla) com o mundo carregado. A RE offline de 2026-09-12
// (coleta de mappins do journal, em `proofs/`) diz que o cozido guarda a posição do nó sem o `offset`.

// Parte 7d-2b-2b (2026-09-12): pin de mod na posição do nó. O hash de mod é o Murmur3 do caminho de um pin da coleta com o último id
// trocado (`<id>_bwms`), fora do cozido; o registro vai pela tabela de mappins de mod da 7d-2a, agora com a posição dada (a do nó resolvido pelo script).

// 2026-09-12: cobertura sintética das 2 buscas sem dado positivo nas fases cozidas (`CompletedPointOfInterestMappin` em nenhuma das 460+
// fases varridas; `LootConditions` só com `isPlayer` falso). No grafo de uma fase carregada que não executa, 1 nó montado pelo `CreateInstance` com o dado
// que o filtro pede entra no fim do `nodes` (reserve do motor, mover conferido), a busca roda com as fases aninhadas e o nó sai; o handle da native é
// solto e o motor destrói a cadeia (nó → condição → tipo, ou nó → tipo → path). Controle: com `certo` 0 o campo do filtro fica no valor errado.

#[cfg(test)]
mod modconfig_tests {
    use super::{config_get_em, config_get_from, config_sanitize_key, config_sanitize_value, config_upsert};

    fn config_temporario(nome: &str, conteudo: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("bwms-teste-config-{}-{nome}", std::process::id()));
        std::fs::write(&p, conteudo).expect("temp_dir gravável");
        p
    }

    #[test]
    fn leitura_guardada_ve_edicao_externa() {
        let p = config_temporario("externa", "legs=0\n");
        assert_eq!(config_get_em(&p, "legs"), "0");
        assert_eq!(config_get_em(&p, "legs"), "0");
        std::fs::write(&p, "legs=1\nstand=1\n").expect("regravar");
        assert_eq!(config_get_em(&p, "legs"), "1");
        assert_eq!(config_get_em(&p, "stand"), "1");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn leitura_guardada_ve_edicao_de_mesmo_tamanho() {
        let p = config_temporario("mesmo-tamanho", "look=0\n");
        assert_eq!(config_get_em(&p, "look"), "0");
        std::fs::write(&p, "look=1\n").expect("regravar");
        // Força um `mtime` diferente: o teste não pode depender da granularidade do disco.
        let f = std::fs::File::options().write(true).open(&p).expect("abrir");
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)).expect("mtime");
        assert_eq!(config_get_em(&p, "look"), "1");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn upsert_atualiza_e_preserva() {
        let existing = "godmode=1\nlegs=0\n";
        let out = config_upsert(existing, "legs", "1");
        assert_eq!(config_get_from(&out, "legs"), "1");
        assert_eq!(config_get_from(&out, "godmode"), "1"); // outras chaves preservadas
    }

    #[test]
    fn upsert_insere_chave_nova() {
        let out = config_upsert("a=1\n", "b", "2");
        assert_eq!(config_get_from(&out, "a"), "1");
        assert_eq!(config_get_from(&out, "b"), "2");
    }

    #[test]
    fn upsert_colapsa_duplicatas() {
        // arquivo corrompido com a chave repetida → set colapsa numa linha só (o writer antigo
        // reescrevia CADA ocorrência, mantendo as duplicatas).
        let existing = "k=1\nk=2\nother=9\nk=3\n";
        let out = config_upsert(existing, "k", "5");
        assert_eq!(out.lines().filter(|l| l.starts_with("k=")).count(), 1);
        assert_eq!(config_get_from(&out, "k"), "5");
        assert_eq!(config_get_from(&out, "other"), "9");
    }

    #[test]
    fn sanitiza_quebra_de_linha_e_igual() {
        // valor com '\n' não pode injetar uma linha/chave falsa.
        let v = config_sanitize_value("mau\nfake=999");
        assert!(!v.contains('\n'));
        let out = config_upsert("", "k", &v);
        assert_eq!(out.lines().count(), 1, "1 linha só, sem chave 'fake' injetada");
        assert_eq!(config_get_from(&out, "fake"), ""); // não vazou
        // chave com '=' é saneada (não quebra o parsing).
        assert_eq!(config_sanitize_key("a=b\nc"), "abc");
    }

    #[test]
    fn get_primeiro_match_e_vazio_ausente() {
        assert_eq!(config_get_from("k=1\nk=2\n", "k"), "1");
        assert_eq!(config_get_from("k=1\n", "ausente"), "");
    }
}

#[cfg(test)]
mod string_utils_tests {
    use super::{
        utf8_split, utf8_split_char, utf8_str_left, utf8_str_len, utf8_str_lower, utf8_str_mid, utf8_str_right, utf8_str_upper,
        write_cstring_inline_ret,
    };

    // O medidor de texto troca `BwmsUTF8StrLen` + `BwmsUTF8StrMid(text, i, 1)` por `BwmsUTF8Split` + `BwmsUTF8SplitChar(i)`: contagem e
    // caracteres têm de ser os mesmos, senão a largura somada muda.
    #[test]
    fn quebra_igual_a_mid_de_um_caractere() {
        let longo: String = "aé 👍x\u{301}".chars().cycle().take(1000).collect();
        for s in ["", "a", "aA a", "café com leite", "👍👍", "e\u{301}x", "🇧🇷", "👨\u{200D}👩", longo.as_str()] {
            let n = utf8_split(s);
            assert_eq!(n, utf8_str_len(s), "contagem de {s:?}");
            for i in 0..n {
                let c = utf8_split_char(i).map(String::from).unwrap_or_default();
                assert_eq!(c, utf8_str_mid(s, i, 1), "caractere {i} de {s:?}");
            }
        }
    }

    #[test]
    fn quebra_fora_do_alcance_e_texto_menor_depois() {
        assert_eq!(utf8_split("abc"), 3);
        assert_eq!(utf8_split_char(-1), None);
        assert_eq!(utf8_split_char(3), None);
        assert_eq!(utf8_split("z"), 1);
        assert_eq!(utf8_split_char(0), Some('z'));
        assert_eq!(utf8_split_char(1), None, "sobra da quebra anterior não pode aparecer");
        assert_eq!(utf8_split(""), 0);
        assert_eq!(utf8_split_char(0), None);
    }

    #[test]
    fn quebra_e_por_thread() {
        assert_eq!(utf8_split("ab"), 2);
        let outra = std::thread::spawn(|| {
            let n = utf8_split("xyz");
            (n, utf8_split_char(2))
        })
        .join()
        .expect("thread do teste");
        assert_eq!(outra, (3, Some('z')));
        assert_eq!(utf8_split_char(1), Some('b'));
        assert_eq!(utf8_split_char(2), None);
    }

    #[test]
    fn conta_ascii_simples() {
        assert_eq!(utf8_str_len("hello"), 5);
    }

    #[test]
    fn conta_codepoints_multibyte_nao_bytes() {
        // "café" = 4 codepoints, mas 5 bytes em UTF-8 (é 2 bytes) — prova que conta CODEPOINT.
        assert_eq!(utf8_str_len("café"), 4);
        assert_eq!("café".len(), 5); // bytes, pra deixar a diferença explícita
    }

    #[test]
    fn string_vazia() {
        assert_eq!(utf8_str_len(""), 0);
    }

    #[test]
    fn emoji_multibyte() {
        // emoji comuns são 4 bytes UTF-8, 1 codepoint cada.
        assert_eq!(utf8_str_len("👍👍"), 2);
    }

    #[test]
    fn left_right_mid_em_codepoints() {
        assert_eq!(utf8_str_left("hello world", 5), "hello");
        assert_eq!(utf8_str_right("hello world", 5), "world");
        assert_eq!(utf8_str_mid("hello world", 6, 5), "world");
        // multibyte: "café com leite" — Left(4) tem que pegar "café" (4 codepoints, 5 bytes),
        // não truncar no meio do 'é' como um corte por-byte faria.
        assert_eq!(utf8_str_left("café com leite", 4), "café");
    }

    #[test]
    fn left_right_mid_clampam_fora_do_alcance() {
        assert_eq!(utf8_str_left("hi", 100), "hi"); // pede mais que existe -> tudo
        assert_eq!(utf8_str_left("hi", 0), "");
        assert_eq!(utf8_str_left("hi", -5), "");
        assert_eq!(utf8_str_right("hi", 100), "hi");
        assert_eq!(utf8_str_mid("hi", 50, 5), ""); // offset além do fim -> vazio
    }

    #[test]
    fn lower_upper_ascii_e_acentuado() {
        assert_eq!(utf8_str_lower("HELLO"), "hello");
        assert_eq!(utf8_str_upper("hello"), "HELLO");
        assert_eq!(utf8_str_lower("CAFÉ"), "café");
        assert_eq!(utf8_str_upper("café"), "CAFÉ");
    }

    // O texto de qualquer tamanho é montado pelo construtor do motor (provado em jogo no boot 268); fora do jogo só dá pra testar a recusa sem
    // slot de retorno.
    #[test]
    fn retorno_de_texto_sem_slot_recusa() {
        unsafe {
            assert!(!write_cstring_inline_ret(std::ptr::null_mut(), &"a".repeat(60)));
        }
    }
}

#[cfg(test)]
mod number_utils_tests {
    use super::*;

    #[test]
    fn parseint_decimal_simples() {
        assert_eq!(strtoll_like("42", 10), Some(42));
        assert_eq!(strtoll_like("-42", 10), Some(-42));
        assert_eq!(strtoll_like("  7", 10), Some(7));
    }

    #[test]
    fn parseint_hex_com_prefixo() {
        assert_eq!(strtoll_like("0x2A", 16), Some(42));
        assert_eq!(strtoll_like("2A", 16), Some(42));
        assert_eq!(strtoll_like("0x2A", 0), Some(42)); // base 0 auto-detecta 0x
    }

    #[test]
    fn parseint_octal_base0() {
        assert_eq!(strtoll_like("010", 0), Some(8));
    }

    #[test]
    fn parseint_lixo_sobrando_falha() {
        assert_eq!(strtoll_like("42abc", 10), None);
        assert_eq!(strtoll_like("42 ", 10), None); // espaço à direita também conta como lixo
        assert_eq!(strtoll_like("", 10), None);
        assert_eq!(strtoll_like("abc", 10), None);
    }

    #[test]
    fn parseint_overflow_clampa() {
        assert_eq!(strtoll_like("99999999999999999999", 10), Some(i64::MAX));
        assert_eq!(strtoll_like("-99999999999999999999", 10), Some(i64::MIN));
    }

    #[test]
    fn parseuint_aceita_minus_e_nega_mod2_64() {
        // strtoull("-1", ...) == UINT64_MAX, comportamento real do glibc/libc++
        assert_eq!(strtoull_like("-1", 10), Some(u64::MAX));
    }

    #[test]
    fn parseuint_overflow_clampa() {
        assert_eq!(strtoull_like("99999999999999999999", 10), Some(u64::MAX));
    }

    #[test]
    fn trampoline_trunca_pela_largura_do_tipo_alvo() {
        // -1 em Int8 deve ter o MESMO bit-pattern que ParseInt8("-1") == 0xFF (truncado)
        let v = strtoll_like("-1", 10).unwrap();
        assert_eq!((v as u64) & 0xFF, 0xFF);
    }
}

// `DynamicEntitySystem`, sub-família "bulk-por-tag" — 5 métodos NOVOS, round 8
// (2026-08-12). O bloco de comentário original listava `GetTagged`/`GetTaggedID`/
// `GetTaggedIDs`/`RegisterListener`/etc. como "FORA — precisam de array<T> de RETORNO (ABI mais
// complexa) ou spawn real". Isso é só PARCIALMENTE verdade: os métodos que devolvem `array<T>`
// (`GetTagged`/`GetTaggedIDs`/`GetTags`) genuinamente precisam da técnica de array-retorno-com-
// trailer (categoria que já crashou 2x neste projeto — `GetRecordsArray`/2026-08-10 e o patch
// de recursos — e só foi resolvida com um donor real emprestado; sem boot pra confirmar, não seguro
// implementar às cegas aqui). MAS `GetTaggedID(tag)` (SINGULAR, devolve só 1 `EntityID`) é um
// retorno ESCALAR trivial — mesma categoria seguríssima de `IsTagged`/`IsPopulated` já provados.
// E o padrão "Count()+ByIndex(i)" já é o IDIOMA ESTABELECIDO deste projeto pra evitar o risco de
// array-retorno em QUALQUER outro item (`ReflectionClass.GetPropertyCount`/`GetPropertyByIndex`,
// `Scripting.GetStackDepth`/`GetStackFrameFunction(i)`) — aplicado aqui pra dar a MESMA capacidade
// prática de `GetTags(id)`/`GetTaggedIDs(tag)` (enumerar) sem o risco de array-retorno.
//
// Deliberadamente NÃO implementado nesta rodada (mantidos fora, mesma linha de raciocínio de
// `#79`/`#138`/`#139` — não repetir "classe forjada aceita chamada, mas nada real por trás"):
// `RegisterListener`/`UnregisterListener`/`UnregisterListeners` dispatcham um `DynamicEntityEvent`
// com 5 tipos de EVENTO DE CICLO DE VIDA REAL (`Created`/`Deleted`/`Spawned`/`Despawned`/`Dead`) —
// mas `CreateEntity`/`DeleteEntity`/`EnableEntity`/`DisableEntity` (as ÚNICAS fontes honestas desses
// eventos) não existem nesta implementação (só bookkeeping de tag, zero spawn real) — implementar
// os listeners disparraria eventos FALSOS sem nenhuma ação real por trás, o mesmo anti-padrão já
// corrigido (`OpenWorldSystem`/`#79`, `DynamicEntitySystem`/`#138`). `EnableTagged`/
// `DisableTagged` (bulk) ficam de fora pelo mesmo motivo — `EnableEntity`/`DisableEntity`
// individuais nem sequer estão registrados nesta build.

pub(crate) const IDSYS_VTBL_VM: u64 = 0x1_0792_bc28;
pub(crate) const ID_NEXT_TRANSIENT_VM: u64 = 0x1_01ef_0a2c;
/// `FixedPoint` da `WorldPosition`: metros × 2^17.
pub(crate) const FIXEDPOINT_ESCALA: f32 = 131072.0;

#[repr(C)]
pub(crate) struct TabelaCallback {
    pub(crate) invoke: unsafe extern "C" fn(*mut u8, *mut [u64; 2]),
    pub(crate) copiar: unsafe extern "C" fn(*mut u8, *const u8),
    pub(crate) mover: unsafe extern "C" fn(*mut u8, *mut u8),
    pub(crate) destruir: unsafe extern "C" fn(*mut u8),
}

// ----- Natives ------------------------------------------------------------------------------------------------------------------------------

/// Grava em `out` um `array` novo de `n` elementos de `passo` B pelo reserve do motor sobre um `DynArray` zerado (o reserve põe o alocador
/// padrão no fim do buffer, `0x10002216c`), e `escrever` preenche os elementos.
pub(crate) unsafe fn array_do_motor_ret(out: *mut c_void, n: usize, passo: u32, escrever: impl FnOnce(*mut u8)) {
    // `out` nulo: a native foi usada inline como argumento (`ArraySize(des.GetTaggedIDs(t))`), e no boot 234 a escrita nele derrubou o jogo.
    if out.is_null() {
        crate::log_dev!("[dynent] array de retorno com out nulo (native usada inline): nada devolvido");
        return;
    }
    let mut arr = [0u64; 2];
    if n > 0 {
        crate::jogo::listas::reservar(arr.as_mut_ptr() as *mut c_void, n as u32, passo, 8, std::ptr::null_mut());
        if arr[0] != 0 {
            escrever(arr[0] as *mut u8);
            core::ptr::write_unaligned((arr.as_mut_ptr() as *mut u8).add(0x0c) as *mut u32, n as u32);
        }
    }
    core::ptr::write_unaligned(out as *mut [u64; 2], arr);
}

pub(crate) unsafe fn array_u64_do_motor_ret(out: *mut c_void, valores: &[u64]) {
    array_do_motor_ret(out, valores.len(), 8, |buf| {
        for (i, v) in valores.iter().enumerate() {
            core::ptr::write_unaligned(buf.add(i * 8) as *mut u64, *v);
        }
    });
}

// ===== Probe + smoke-test (comandos de console) =====================================

/// 2026-08-05 (unificação BWMS): `registrar_fachada_de_versao`/`registrar_fachada_de_tweaks` eram
/// copy-paste byte-a-byte (mesmo protótipo doador, mesmo loop de `register_method`, mesma
/// ressalva de nome bare da Tentativa 11) — a ÚNICA diferença real é a lista de (nome, trampolim)
/// por classe. Um 3º facade futuro (se algum mod real precisar) usa isto
/// direto, sem reimplementar o padrão pela 3ª vez. `proto_donor` é sempre `gameGodModeSystem::
/// AddGodMode` (qualquer método estático nativo simples serve de molde de `CClassFunction`, não
/// é específico de nenhum facade).
pub(crate) unsafe fn register_facade_methods(reg: &Registry, class: &str, methods: &[(&str, NativeHandler)]) -> String {
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return format!("[reg] sem protótipo de método (AddGodMode não resolveu) — {class} não registrado"),
    };
    // Tentativa 11 (2026-07-13, achado na fachada de versão, vale pra QUALQUER facade forjado assim):
    // disasm do 2º validador de classe mostrou que ele compara [func+0x08] (fullName) contra o
    // CName do DESCRITOR do compilador — o nome BARE ("Version"/"Require"), sem prefixo de
    // classe. `full`=`short` (nome bare nos dois campos) é obrigatório aqui, não opcional.
    let parts: Vec<String> = methods
        .iter()
        .map(|(name, handler)| {
            let ok = register_method(reg, class, proto, name, name, *handler, true);
            format!("{name}={ok}")
        })
        .collect();
    format!("[reg] {class}.{} (precisa do .reds da Facade carregado p/ a classe existir)", parts.join(" "))
}

// ===== `TweakDBManager` (`PENDENCIAS-UNIFICADAS.md`): a API redscript real que mods do Nexus importam pra CRUD de TweakDB em runtime.
// 1º incremento (2026-08-08): SetFlat (só escalar) / CreateRecord / UpdateRecord / RegisterName —
// todos delegando pro backend NATIVO já provado em `tweakdb/mod.rs` (comandos de console `setflat`/
// `xlautoclone`/`updaterecroundtrip`), agora acessível direto do redscript sem passar por comando.
// 🔶 CODADO — mesmo padrão de forja já funcional da fachada de tweaks
// (`registrar_fachada_de_tweaks` acima), nunca testado ao vivo ainda (ver `.reds` em
// `blackwall-mods-dev/ponte-tweakdbmanager.reds` pro status/cobertura exatos).




#[cfg(test)]
mod bits_utils_tests {
    use super::{bit_set, bit_shl, bit_shr, bit_test, mask_arg};

    #[test]
    fn bit_test_le_e_fora_do_range() {
        assert!(bit_test(0b1010, 1));
        assert!(!bit_test(0b1010, 0));
        assert!(!bit_test(0xFF, -1));
        assert!(!bit_test(0xFF, 64));
    }

    #[test]
    fn bit_set_liga_e_desliga() {
        assert_eq!(bit_set(0b0000, 2, true), 0b0100);
        assert_eq!(bit_set(0b1111, 2, false), 0b1011);
        // fora do range: devolve o valor original, sem tocar
        assert_eq!(bit_set(0xAB, -1, true), 0xAB);
        assert_eq!(bit_set(0xAB, 64, true), 0xAB);
    }

    #[test]
    fn bit_shl_mascara_pela_largura() {
        // 8 bits: 0xFF << 4 estoura o byte, deve truncar em 8 bits (mask 0xFF)
        assert_eq!(bit_shl(0xFF, 4, 8), 0xF0);
        // 32 bits: 1 << 31 cabe exatamente
        assert_eq!(bit_shl(1, 31, 32), 1u64 << 31);
        // 64 bits: sem mask (largura total)
        assert_eq!(bit_shl(1, 63, 64), 1u64 << 63);
    }

    #[test]
    fn bit_shr_simples() {
        assert_eq!(bit_shr(0b1000, 3), 0b1);
        assert_eq!(bit_shr(0xFF, -1), 0xFF); // n negativo: devolve como veio
    }

    #[test]
    fn mask_arg_trunca_pela_largura_certa() {
        assert_eq!(mask_arg(0x1234_5678_9ABC_DEF0, 1), 0xF0);
        assert_eq!(mask_arg(0x1234_5678_9ABC_DEF0, 2), 0xDEF0);
        assert_eq!(mask_arg(0x1234_5678_9ABC_DEF0, 4), 0x9ABC_DEF0);
        assert_eq!(mask_arg(0x1234_5678_9ABC_DEF0, 8), 0x1234_5678_9ABC_DEF0);
    }
}

/// `CallbackSystemTarget`/`EntityTarget` dispatch-time filtering
/// (2026-08-12) — testa o NÚCLEO PURO da decisão de despacho (`entity_recordid_targets_allow`),
/// isolado de RTTI/ponteiros/boot. A regra de ouro da implementação (nunca regredir o caso
/// sem-target) e o fail-open (nunca bloquear por incerteza) são os 2 invariantes mais críticos —
/// cobertos explicitamente abaixo, sem depender do jogo rodando.
#[cfg(test)]
mod entitytarget_recordid_filter_tests {
    use crate::diagnostico::entity_recordid_targets_allow;

    #[test]
    fn sem_criterios_sempre_despacha() {
        // Handler sem NENHUM EntityTarget(RecordID) associado (lista vazia) — este filtro não se
        // aplica, nunca bloqueia. Cobre tanto "handler sem target nenhum" (#1, 7/7 já fechado,
        // nunca pode regredir) quanto "handler só tem Target de outra família" (ComponentTarget
        // etc., fora de escopo desta rodada).
        assert!(entity_recordid_targets_allow(&[], None));
        assert!(entity_recordid_targets_allow(&[], Some(0xDEAD)));
    }

    #[test]
    fn entidade_indeterminada_fail_open() {
        // Handler TEM critério(s) RecordID, mas a entidade do evento não é Puppet/Vehicle (ou a
        // leitura de memória falhou) — `None`. FAIL-OPEN: nunca inventa "não bate" sem dado real.
        assert!(entity_recordid_targets_allow(&[0x111], None));
        assert!(entity_recordid_targets_allow(&[0x111, 0x222], None));
    }

    #[test]
    fn recordid_bate_despacha() {
        assert!(entity_recordid_targets_allow(&[0xAAAA], Some(0xAAAA)));
    }

    #[test]
    fn recordid_nao_bate_bloqueia() {
        assert!(!entity_recordid_targets_allow(&[0xAAAA], Some(0xBBBB)));
    }

    #[test]
    fn multiplos_criterios_semantica_or() {
        // `.RecordID(a)` + `.RecordID(b)` no mesmo handler (2 AddTarget) = "qualquer um dos dois".
        let criteria = [0x111u64, 0x222, 0x333];
        assert!(entity_recordid_targets_allow(&criteria, Some(0x222))); // bate o do meio
        assert!(entity_recordid_targets_allow(&criteria, Some(0x333))); // bate o último
        assert!(!entity_recordid_targets_allow(&criteria, Some(0x999))); // nenhum bate
    }

    #[test]
    fn recordid_zero_e_um_criterio_valido_normal() {
        // TweakDBID(0) é um valor real e possível (registro "vazio"/sentinela) — não é caso
        // especial nem "sem filtro"; segue a MESMA regra de comparação exata que qualquer outro.
        assert!(entity_recordid_targets_allow(&[0], Some(0)));
        assert!(!entity_recordid_targets_allow(&[0], Some(1)));
        assert!(!entity_recordid_targets_allow(&[1], Some(0)));
    }
}

/// `DynamicEntityTarget` dispatch-time filtering (2026-08-12) — testa o NÚCLEO
/// PURO da decisão de despacho por TAG (`dynamic_entity_tag_targets_allow`), mesmo formato de
/// `entitytarget_recordid_filter_tests` acima (fail-open/sem-critério/OR/semântica de conjunto).
#[cfg(test)]
mod dynamicentitytarget_tag_filter_tests {
    use super::dynamic_entity_tag_targets_allow;
    use std::collections::HashSet;

    #[test]
    fn sem_criterios_sempre_despacha() {
        let tags: HashSet<u64> = [0xDEAD].into_iter().collect();
        assert!(dynamic_entity_tag_targets_allow(&[], None));
        assert!(dynamic_entity_tag_targets_allow(&[], Some(&tags)));
    }

    #[test]
    fn entidade_nao_rastreada_fail_open() {
        // Handler TEM critério(s) de tag, mas a entidade nunca teve `AssignTag` chamado — `None`.
        // FAIL-OPEN: nunca inventa "não bate" sem dado real (mesma regra do `#134`).
        assert!(dynamic_entity_tag_targets_allow(&[0x111], None));
        assert!(dynamic_entity_tag_targets_allow(&[0x111, 0x222], None));
    }

    #[test]
    fn tag_bate_despacha() {
        let tags: HashSet<u64> = [0xAAAA, 0xBBBB].into_iter().collect();
        assert!(dynamic_entity_tag_targets_allow(&[0xAAAA], Some(&tags)));
    }

    #[test]
    fn tag_nao_bate_bloqueia() {
        let tags: HashSet<u64> = [0xBBBB].into_iter().collect();
        assert!(!dynamic_entity_tag_targets_allow(&[0xAAAA], Some(&tags)));
    }

    #[test]
    fn entidade_com_tags_vazias_bloqueia_criterio_nao_vazio() {
        // Diferente de `None` (indeterminado): a entidade É rastreada mas não tem NENHUMA tag
        // atribuída — `Some(HashSet::new())`. Não é fail-open (temos dado real, só que negativo).
        let tags: HashSet<u64> = HashSet::new();
        assert!(!dynamic_entity_tag_targets_allow(&[0xAAAA], Some(&tags)));
    }

    #[test]
    fn multiplos_criterios_semantica_or() {
        // `.Tag(a)` + `.Tag(b)` no mesmo handler (2 AddTarget) = "qualquer uma das duas".
        let criteria = [0x111u64, 0x222, 0x333];
        let tags_a: HashSet<u64> = [0x222].into_iter().collect();
        let tags_b: HashSet<u64> = [0x999].into_iter().collect();
        assert!(dynamic_entity_tag_targets_allow(&criteria, Some(&tags_a)));
        assert!(!dynamic_entity_tag_targets_allow(&criteria, Some(&tags_b)));
    }

    #[test]
    fn tags_plural_mesmo_objeto_semantica_or_dentro_do_target() {
        // `.Tags([a,b,c])` (2026-08-18, 1 único DynamicEntityTarget, 3 tags no mesmo Set<CName>)
        // — `tramp_dyt_tags` grava UMA linha `(ptr, [tag_a, tag_b, tag_c])` em
        // `DYNAMICENTITYTARGET_STATES` (`gravar_alvo_por_tags`). No despacho vivo, "N tags de
        // 1 target" e `.Tag(a)+.Tag(b)+.Tag(c)` (3 objetos via AddTarget) dão o mesmo resultado
        // porque há OR nos dois níveis: `bate_por_tags` dentro do alvo (`alvo_casa`,
        // `corpo_e_rosto`) e `any` entre os alvos (`alvos_barram_o_disparo`, `corpo_e_rosto`),
        // fielmente à semântica OR do `Matches()` real (`for tag in tags: if IsTagged(...)
        // return true`). Aqui as 3 tags do único target vão numa lista só e basta QUALQUER uma bater.
        let criteria_de_um_unico_tags_target = [0xAAAAu64, 0xBBBB, 0xCCCC];
        let tags_com_b: std::collections::HashSet<u64> = [0xBBBB].into_iter().collect();
        let tags_sem_nenhuma: std::collections::HashSet<u64> = [0xDDDD].into_iter().collect();
        assert!(dynamic_entity_tag_targets_allow(&criteria_de_um_unico_tags_target, Some(&tags_com_b)));
        assert!(!dynamic_entity_tag_targets_allow(&criteria_de_um_unico_tags_target, Some(&tags_sem_nenhuma)));
    }
}

/// Consultas de tag dos dois sistemas de entidade (`#73` dinâmico e `#75` estático usam as mesmas funções) — testa
/// os núcleos dos 5 getters (`GetTaggedID`/`GetTaggedCount`/`GetTaggedIDByIndex`/
/// `GetTagCount`/`GetTagByIndex`) contra um `HashMap` fabricado, sem depender do registry global
/// (`dynent_tags()`/`dynent_by_tag()`, protegido por `Mutex` — não seguro chamar em paralelo com
/// outros testes do crate que também tocam esse estado global).
#[cfg(test)]
mod dynent_query_tests {
    use crate::entidades::{conjunto_menor, conjunto_no_indice, conjunto_tamanho};
    use std::collections::{HashMap, HashSet};

    fn map(pairs: &[(u64, &[u64])]) -> HashMap<u64, HashSet<u64>> {
        pairs.iter().map(|(k, vs)| (*k, vs.iter().copied().collect())).collect()
    }

    #[test]
    fn tag_desconhecida_devolve_zero_ou_vazio() {
        let m = map(&[]);
        assert_eq!(conjunto_menor(&m, 0x999), 0);
        assert_eq!(conjunto_tamanho(&m, 0x999), 0);
        assert_eq!(conjunto_no_indice(&m, 0x999, 0), 0);
        assert_eq!(conjunto_tamanho(&m, 0x999), 0);
        assert_eq!(conjunto_no_indice(&m, 0x999, 0), 0);
    }

    #[test]
    fn get_tagged_id_devolve_algum_membro_real() {
        let m = map(&[(0xAAAA_1111, &[0x10, 0x20, 0x30])]);
        let id = conjunto_menor(&m, 0xAAAA_1111);
        assert!([0x10, 0x20, 0x30].contains(&id));
    }

    #[test]
    fn get_tagged_count_conta_certo() {
        let m = map(&[(0xAAAA, &[0x10, 0x20, 0x30]), (0xBBBB, &[0x40])]);
        assert_eq!(conjunto_tamanho(&m, 0xAAAA), 3);
        assert_eq!(conjunto_tamanho(&m, 0xBBBB), 1);
        assert_eq!(conjunto_tamanho(&m, 0xCCCC), 0);
    }

    #[test]
    fn get_tagged_id_by_index_ordem_deterministica() {
        let m = map(&[(0xAAAA, &[0x30, 0x10, 0x20])]);
        // Ordenado ascendente (`sort_unstable`), independente da ordem de inserção no HashSet.
        assert_eq!(conjunto_no_indice(&m, 0xAAAA, 0), 0x10);
        assert_eq!(conjunto_no_indice(&m, 0xAAAA, 1), 0x20);
        assert_eq!(conjunto_no_indice(&m, 0xAAAA, 2), 0x30);
    }

    #[test]
    fn get_tagged_id_by_index_fora_de_faixa_devolve_zero() {
        let m = map(&[(0xAAAA, &[0x10])]);
        assert_eq!(conjunto_no_indice(&m, 0xAAAA, 1), 0);
        assert_eq!(conjunto_no_indice(&m, 0xAAAA, -1), 0);
    }

    #[test]
    fn get_tag_count_e_by_index_espelham_a_mesma_logica_do_lado_id() {
        let m = map(&[(0x111, &[0xAAAA, 0xBBBB])]);
        assert_eq!(conjunto_tamanho(&m, 0x111), 2);
        let mut got = [conjunto_no_indice(&m, 0x111, 0), conjunto_no_indice(&m, 0x111, 1)];
        got.sort_unstable();
        assert_eq!(got, [0xAAAA, 0xBBBB]);
        assert_eq!(conjunto_no_indice(&m, 0x111, 2), 0);
    }
}

/// `ComponentTarget` dispatch-time filtering (2026-08-12) — testa o NÚCLEO PURO
/// da decisão de despacho por `ID`(CRUID) OU `Name`(CName) (`component_target_targets_allow`),
/// mesmo formato dos núcleos puros já testados acima.
#[cfg(test)]
mod componenttarget_filter_tests {
    use crate::diagnostico::component_target_targets_allow;

    #[test]
    fn sem_criterios_sempre_despacha() {
        assert!(component_target_targets_allow(&[], &[], None));
        assert!(component_target_targets_allow(&[], &[], Some((0x111, 0x222))));
    }

    #[test]
    fn componente_indeterminado_fail_open() {
        // Handler TEM critério(s), mas o componente do evento não foi resolvido (`None`) —
        // FAIL-OPEN, mesma regra do `#134`/`#133`.
        assert!(component_target_targets_allow(&[0xAAAA], &[], None));
        assert!(component_target_targets_allow(&[], &[0xBBBB], None));
    }

    #[test]
    fn id_bate_despacha() {
        assert!(component_target_targets_allow(&[0xAAAA], &[], Some((0xAAAA, 0x999))));
    }

    #[test]
    fn id_nao_bate_bloqueia() {
        assert!(!component_target_targets_allow(&[0xAAAA], &[], Some((0xBBBB, 0x999))));
    }

    #[test]
    fn name_bate_despacha() {
        assert!(component_target_targets_allow(&[], &[0xCCCC], Some((0x111, 0xCCCC))));
    }

    #[test]
    fn name_nao_bate_bloqueia() {
        assert!(!component_target_targets_allow(&[], &[0xCCCC], Some((0x111, 0xDDDD))));
    }

    #[test]
    fn id_e_name_juntos_semantica_or_entre_tipos() {
        // `.ID(a)` + `.Name(b)` no mesmo handler (2 AddTarget) — qualquer um dos dois campos bate
        // já é suficiente (semântica OR entre todos os alvos do handler, independente de tipo).
        let id_c = [0xAAAA];
        let name_c = [0xCCCC];
        // ID bate, Name não bate -> ainda despacha (achou 1 target que bate).
        assert!(component_target_targets_allow(&id_c, &name_c, Some((0xAAAA, 0xDDDD))));
        // ID não bate, Name bate -> ainda despacha.
        assert!(component_target_targets_allow(&id_c, &name_c, Some((0xBBBB, 0xCCCC))));
        // Nenhum bate -> bloqueia.
        assert!(!component_target_targets_allow(&id_c, &name_c, Some((0xBBBB, 0xDDDD))));
    }

    #[test]
    fn multiplos_criterios_do_mesmo_tipo_semantica_or() {
        let ids = [0x111u64, 0x222, 0x333];
        assert!(component_target_targets_allow(&ids, &[], Some((0x222, 0))));
        assert!(!component_target_targets_allow(&ids, &[], Some((0x999, 0))));
    }
}

/// `InputTarget` dispatch-time filtering (2026-08-12) — testa o NÚCLEO PURO da
/// decisão de despacho por `key` E `action` (`input_target_targets_allow`), incl. os 2 sentinelas
/// (`IK_None=0`/`IACT_None=0`) como wildcard — as 2 guardas
/// (`key != IK_None && key != event->key`, `action != IACT_None && action != event->action`).
#[cfg(test)]
mod inputtarget_filter_tests {
    use crate::diagnostico::input_target_targets_allow;
    use crate::entrada::{IACT_NONE, IACT_PRESS, IACT_RELEASE, IK_NONE};

    /// Critério de um `.Key(key, action)` — o par cru de `INPUTTARGET_STATES`, com o sentinela, que esta cópia de teste lê.
    fn key_target(key: i32, action: i32) -> (i64, i64) {
        (key as i64, action as i64)
    }
    /// Critério de um `.Axis(axis, threshold)`: `action` entra SEMPRE como wildcard, porque a
    /// fonte C++ nunca escreve o campo `action` em `Axis()` (ver `InputTargetCriterion`).
    fn axis_target(axis: i64) -> (i64, i64) {
        (axis, IACT_NONE as i64)
    }

    #[test]
    fn sem_criterios_sempre_despacha() {
        assert!(input_target_targets_allow(&[], None, None));
        assert!(input_target_targets_allow(&[], Some(49), Some(IACT_PRESS as i64)));
    }

    #[test]
    fn evento_indeterminado_fail_open() {
        assert!(input_target_targets_allow(&[key_target(49, IACT_PRESS)], None, None));
        // Só uma das 2 dimensões indeterminada: a outra ainda decide.
        assert!(input_target_targets_allow(&[key_target(49, IACT_PRESS)], None, Some(IACT_PRESS as i64)));
        assert!(!input_target_targets_allow(&[key_target(49, IACT_PRESS)], None, Some(IACT_RELEASE as i64)));
    }

    #[test]
    fn key_bate_despacha() {
        assert!(input_target_targets_allow(
            &[key_target(49, IACT_NONE)],
            Some(49),
            Some(IACT_PRESS as i64)
        ));
    }

    #[test]
    fn key_nao_bate_bloqueia() {
        assert!(!input_target_targets_allow(
            &[key_target(49, IACT_NONE)],
            Some(50),
            Some(IACT_PRESS as i64)
        ));
    }

    #[test]
    fn ik_none_e_wildcard_sempre_bate() {
        // `InputTarget.Key(IK_None, ...)` (edge case — factories reais sempre exigem key
        // concreta, mas a fonte C++ trata 0 como "sem filtro") — deve bater qualquer key real.
        assert!(input_target_targets_allow(
            &[key_target(IK_NONE, IACT_NONE)],
            Some(49),
            Some(IACT_PRESS as i64)
        ));
        assert!(input_target_targets_allow(
            &[key_target(IK_NONE, IACT_NONE)],
            Some(999),
            Some(IACT_RELEASE as i64)
        ));
    }

    #[test]
    fn multiplos_criterios_semantica_or() {
        let criteria = [key_target(49, IACT_NONE), key_target(50, IACT_NONE), key_target(51, IACT_NONE)];
        assert!(input_target_targets_allow(&criteria, Some(50), Some(IACT_PRESS as i64)));
        assert!(!input_target_targets_allow(&criteria, Some(999), Some(IACT_PRESS as i64)));
    }

    /// O CASO QUE O BUG DO `action` FIXO QUEBRAVA: um mod que pede só a DESCIDA da tecla.
    /// Antes de 2026-08-21 todo evento carregava `action=2`(`IACT_Release`), então este alvo
    /// nunca disparava na descida e disparava na subida — exatamente ao contrário do pedido.
    #[test]
    fn action_press_so_bate_na_descida() {
        let alvo = [key_target(49, IACT_PRESS)];
        assert!(input_target_targets_allow(&alvo, Some(49), Some(IACT_PRESS as i64)));
        assert!(!input_target_targets_allow(&alvo, Some(49), Some(IACT_RELEASE as i64)));
    }

    #[test]
    fn action_release_so_bate_na_subida() {
        let alvo = [key_target(49, IACT_RELEASE)];
        assert!(!input_target_targets_allow(&alvo, Some(49), Some(IACT_PRESS as i64)));
        assert!(input_target_targets_allow(&alvo, Some(49), Some(IACT_RELEASE as i64)));
    }

    /// `action` omitido no `.reds` = `IACT_None` = wildcard: bate nas 2 fases (comportamento
    /// esperado, e o que o filtro só-por-key de 2026-08-12 fazia pra TODO alvo).
    #[test]
    fn action_none_bate_nas_duas_fases() {
        let alvo = [key_target(49, IACT_NONE)];
        assert!(input_target_targets_allow(&alvo, Some(49), Some(IACT_PRESS as i64)));
        assert!(input_target_targets_allow(&alvo, Some(49), Some(IACT_RELEASE as i64)));
    }

    /// As 2 dimensões são AND dentro do MESMO alvo (não OR): key certa + action errada bloqueia.
    #[test]
    fn key_e_action_sao_and_dentro_do_alvo() {
        let alvo = [key_target(49, IACT_PRESS)];
        assert!(!input_target_targets_allow(&alvo, Some(50), Some(IACT_PRESS as i64)));
        assert!(!input_target_targets_allow(&alvo, Some(49), Some(IACT_RELEASE as i64)));
        assert!(!input_target_targets_allow(&alvo, Some(50), Some(IACT_RELEASE as i64)));
        assert!(input_target_targets_allow(&alvo, Some(49), Some(IACT_PRESS as i64)));
    }

    /// REGRESSÃO da colisão de slot: um `.Axis(axis, 0.5)` guarda o threshold em BITS no mesmo
    /// 4º campo onde `.Key` guarda `action` — `0.5f32` = `0x3F000000` = 1056964608. Sem a tag
    /// `InputTargetCriterion`, esse número seria lido como `action` e, por não ser `IACT_None`
    /// nem casar com nenhum `EInputAction` real (0-3), bloquearia TODO evento do alvo.
    #[test]
    fn axis_nao_confunde_threshold_com_action() {
        let bits = 0.5f32.to_bits() as i64;
        assert_eq!(bits, 1_056_964_608, "fixture do threshold mudou");
        // Como `inputtarget_criteria` monta o alvo de Axis (action = wildcard): despacha normal.
        let alvo = [axis_target(49)];
        assert!(input_target_targets_allow(&alvo, Some(49), Some(IACT_PRESS as i64)));
        assert!(input_target_targets_allow(&alvo, Some(49), Some(IACT_RELEASE as i64)));
        // Se o threshold vazasse pro slot de `action` (o bug que a tag previne), bloquearia tudo:
        let alvo_bugado = [(49i64, bits)];
        assert!(!input_target_targets_allow(&alvo_bugado, Some(49), Some(IACT_PRESS as i64)));
        assert!(!input_target_targets_allow(&alvo_bugado, Some(49), Some(IACT_RELEASE as i64)));
    }
}

/// `ResourceTarget` dispatch-time filtering (2026-08-12) — testa o NÚCLEO PURO
/// da decisão por `Path` (`resource_target_targets_allow`), mesmo formato dos outros 5 núcleos.
#[cfg(test)]
mod resourcetarget_filter_tests {
    use crate::diagnostico::resource_target_targets_allow;

    #[test]
    fn sem_criterios_sempre_despacha() {
        assert!(resource_target_targets_allow(&[], None));
        assert!(resource_target_targets_allow(&[], Some(0xAAAA)));
    }

    #[test]
    fn path_indeterminado_fail_open() {
        assert!(resource_target_targets_allow(&[0xAAAA], None));
    }

    #[test]
    fn path_bate_despacha() {
        assert!(resource_target_targets_allow(&[0xAAAA], Some(0xAAAA)));
    }

    #[test]
    fn path_nao_bate_bloqueia() {
        assert!(!resource_target_targets_allow(&[0xAAAA], Some(0xBBBB)));
    }

    #[test]
    fn multiplos_criterios_semantica_or() {
        let criteria = [0x111u64, 0x222, 0x333];
        assert!(resource_target_targets_allow(&criteria, Some(0x222)));
        assert!(!resource_target_targets_allow(&criteria, Some(0x999)));
    }
}

/// `inkWidgetTarget::Matches` — a regra de um alvo (`ink_widget_alvo_casa`): biblioteca, item e controller, cada um só quando setado.
#[cfg(test)]
mod inkwidgettarget_filter_tests {
    use super::ink_widget_alvo_casa;

    const LIB: u64 = 0xAAAA;
    const ITEM: u64 = 0x1111;

    #[test]
    fn biblioteca_e_item_iguais_casam() {
        assert!(ink_widget_alvo_casa((Some(LIB), Some(ITEM), None), LIB, ITEM, |_| false));
    }

    #[test]
    fn item_nao_setado_e_curinga() {
        assert!(ink_widget_alvo_casa((Some(LIB), None, None), LIB, 0x2222, |_| false));
    }

    #[test]
    fn biblioteca_diferente_nao_casa() {
        assert!(!ink_widget_alvo_casa((Some(LIB), None, None), 0xBBBB, ITEM, |_| true));
    }

    #[test]
    fn item_diferente_nao_casa() {
        assert!(!ink_widget_alvo_casa((Some(LIB), Some(ITEM), None), LIB, 0x2222, |_| true));
    }

    #[test]
    fn controller_decide_quando_setado() {
        assert!(ink_widget_alvo_casa((None, None, Some(0x77)), LIB, ITEM, |t| t == 0x77));
        assert!(!ink_widget_alvo_casa((None, None, Some(0x77)), LIB, ITEM, |_| false));
    }

    #[test]
    fn controller_nao_consultado_sem_tipo() {
        assert!(ink_widget_alvo_casa((Some(LIB), Some(ITEM), None), LIB, ITEM, |_| panic!("controller consultado sem tipo")));
    }
}

/// `ResourceHelper.IsReferenceLoaded` (2026-08-12) — núcleo puro
/// `resource_token_is_loaded`: carregado = terminou E sem erro. Testável offline sem RTTI/ponteiro nenhum.
#[cfg(test)]
mod resource_token_is_loaded_tests {
    use super::resource_token_is_loaded;

    #[test]
    fn nao_finalizado_nunca_carregado() {
        // finished==0 (ainda carregando ou nunca iniciado) — sempre false, com ou sem erro.
        assert!(!resource_token_is_loaded(0, 0));
        assert!(!resource_token_is_loaded(0, 1));
    }

    #[test]
    fn finalizado_sem_erro_carregado() {
        assert!(resource_token_is_loaded(1, 0));
        // qualquer valor não-zero de `finished` conta (é um contador atômico de job, não um bool
        // estrito) — mesma semântica do `bool` C++ (`finished &&`).
        assert!(resource_token_is_loaded(42, 0));
        assert!(resource_token_is_loaded(-1, 0));
    }

    #[test]
    fn finalizado_com_erro_nao_carregado() {
        // finished!=0 mas error!=0 = terminou em FALHA, não em sucesso — `IsFailed()`, não
        // `IsLoaded()`. Distinção central do método (não é só "terminou").
        assert!(!resource_token_is_loaded(1, 1));
        assert!(!resource_token_is_loaded(1, 255));
    }
}

/// `PersistentID.ToHash` (2026-08-14) — núcleo puro `persistent_id_hash`, o hash do
/// `PersistentID` do jogo. Testável offline sem RTTI/ponteiro/boot.
#[cfg(test)]
mod persistent_id_hash_tests {
    use super::persistent_id_hash;

    #[test]
    fn entity_only_devolve_o_proprio_entity_hash() {
        // componentName "None" (hash==0) — mesmo comportamento JÁ PROVADO ao vivo (`ToHash`
        // original, antes desta rodada, só cobria este caso) — não pode regredir.
        assert_eq!(persistent_id_hash(0xDEADBEEF, 0), 0xDEADBEEF);
        assert_eq!(persistent_id_hash(0, 0), 0);
        assert_eq!(persistent_id_hash(u64::MAX, 0), u64::MAX);
    }

    #[test]
    fn component_case_nunca_degenera_pro_entity_hash_puro() {
        // Com componentName != None, o resultado tem que vir do mix MurmurHash64A-like, não do
        // entityHash cru — refuta um bug óbvio de "esqueceu de ramificar".
        let e = 0x1234_5678_9abc_def0u64;
        let c = 0x0fed_cba9_8765_4321u64;
        let h = persistent_id_hash(e, c);
        assert_ne!(h, e);
        assert_ne!(h, c);
        assert_ne!(h, 0);
    }

    #[test]
    fn component_case_e_deterministico_e_sensivel_aos_2_hashes() {
        // Mesma entrada -> mesma saída (função pura); mudar QUALQUER um dos 2 hashes muda o
        // resultado (confirma que ambos entram na mistura, não só um deles).
        let e = 0x1111_1111_1111_1111u64;
        let c = 0x2222_2222_2222_2222u64;
        assert_eq!(persistent_id_hash(e, c), persistent_id_hash(e, c));
        assert_ne!(persistent_id_hash(e, c), persistent_id_hash(e + 1, c));
        assert_ne!(persistent_id_hash(e, c), persistent_id_hash(e, c + 1));
    }

    #[test]
    fn valores_extremos_nao_estouram_sem_wrap() {
        // overflow de u64 tem que dar wrap (== comportamento unsigned do C++ real), nunca panic
        // em debug/release — exercita os 4 `wrapping_mul`/xor da tradução do algoritmo real.
        let _ = persistent_id_hash(u64::MAX, u64::MAX);
        let _ = persistent_id_hash(u64::MAX, 1);
        let _ = persistent_id_hash(1, u64::MAX);
    }
}

#[cfg(test)]
mod wrapped_array_offsets_tests {
    use crate::diagnostico::wrapped_array_elem_offsets;

    #[test]
    fn caso_normal_respeita_stride_e_tamanho() {
        assert_eq!(wrapped_array_elem_offsets(4, 3, 0x20, 8), vec![0, 0x20, 0x40]);
    }

    #[test]
    fn teto_de_amostra_corta_array_grande() {
        assert_eq!(wrapped_array_elem_offsets(1000, 1000, 16, 3), vec![0, 16, 32]);
    }

    // `size > cap` = header incoerente (não é DynArray, ou memória em transição). Deref aqui seria
    // leitura fora dos limites em memória VIVA do jogo — recusar é o comportamento certo.
    #[test]
    fn header_incoerente_size_maior_que_cap_recusa() {
        assert!(wrapped_array_elem_offsets(2, 9, 16, 8).is_empty());
    }

    // stride 0 = `GetSize` não resolveu o tipo interno; stride absurdo = tipo errado. Nos dois
    // casos a aritmética de ponteiro seria lixo.
    #[test]
    fn stride_invalido_recusa() {
        assert!(wrapped_array_elem_offsets(4, 4, 0, 8).is_empty());
        assert!(wrapped_array_elem_offsets(4, 4, 99_999, 8).is_empty());
    }

    #[test]
    fn array_vazio_nao_sonda_nada() {
        assert!(wrapped_array_elem_offsets(8, 0, 16, 8).is_empty());
    }
}

#[cfg(test)]
mod res_loader_signature_tests {
    use crate::diagnostico::res_loader_signature;

    fn win(index_table: u64, size: u32, cap: u32, nodes: u64, stride: u32) -> Vec<u8> {
        let mut w = vec![0u8; 0x40];
        w[0x20..0x28].copy_from_slice(&index_table.to_le_bytes());
        w[0x28..0x2c].copy_from_slice(&size.to_le_bytes());
        w[0x2c..0x30].copy_from_slice(&cap.to_le_bytes());
        w[0x30..0x38].copy_from_slice(&nodes.to_le_bytes());
        w[0x3c..0x40].copy_from_slice(&stride.to_le_bytes());
        w
    }

    #[test]
    fn objeto_plausivel_e_aceito() {
        let w = win(0x7b_1157_c4d0, 1200, 2048, 0x7b_1157_e000, 0x20);
        assert_eq!(res_loader_signature(&w, 1), Some((1200, 2048, 0x20)));
    }

    // `size > capacity` é impossível num HashMap coerente — é a checagem que mais separa sinal de
    // lixo, porque memória aleatória viola isso quase sempre.
    #[test]
    fn size_maior_que_capacity_rejeitado() {
        assert!(res_loader_signature(&win(0x7b_0000_0000, 9000, 64, 0x7b_0001_0000, 0x20), 1).is_none());
    }

    // Mapa vazio não distingue "loader sem tokens" de "memória zerada" — rejeita pra não virar ruído.
    // O piso de tamanho é o que separa "mapa de tokens" de "mapinha qualquer": a 1ª varredura
    // achou 4 mapas de 3-40 entradas com `min_size=1`, nenhum plausível como loader.
    #[test]
    fn abaixo_do_piso_de_tamanho_rejeitado() {
        assert!(res_loader_signature(&win(0x7b_0000_0000, 40, 64, 0x7b_0001_0000, 0x20), 64).is_none());
        assert!(res_loader_signature(&win(0x7b_0000_0000, 900, 2048, 0x7b_0001_0000, 0x20), 64).is_some());
    }

    #[test]
    fn mapa_vazio_rejeitado() {
        assert!(res_loader_signature(&win(0x7b_0000_0000, 0, 64, 0x7b_0001_0000, 0x20), 1).is_none());
    }

    #[test]
    fn ponteiro_pequeno_demais_nao_e_ponteiro() {
        assert!(res_loader_signature(&win(0x1234, 10, 64, 0x7b_0001_0000, 0x20), 1).is_none());
        assert!(res_loader_signature(&win(0x7b_0000_0000, 10, 64, 0x40, 0x20), 1).is_none());
    }

    #[test]
    fn stride_implausivel_rejeitado() {
        assert!(res_loader_signature(&win(0x7b_0000_0000, 10, 64, 0x7b_0001_0000, 0), 1).is_none());
        assert!(res_loader_signature(&win(0x7b_0000_0000, 10, 64, 0x7b_0001_0000, 4096), 1).is_none());
        assert!(res_loader_signature(&win(0x7b_0000_0000, 10, 64, 0x7b_0001_0000, 0x21), 1).is_none());
    }

    #[test]
    fn janela_curta_demais_rejeitada() {
        assert!(res_loader_signature(&[0u8; 0x20], 1).is_none());
    }
}

#[cfg(test)]
mod res_loader_ascii_falso_positivo_tests {
    use crate::diagnostico::res_loader_signature;

    /// O caso real que a varredura de 2026-08-21 produziu: uma janela de TEXTO ASCII passa em todos
    /// os invariantes numéricos. Documenta por que a assinatura numérica sozinha é insuficiente: só a
    /// leitura de memória (ponteiros legíveis) separa o falso-positivo.
    #[test]
    fn texto_ascii_passa_na_assinatura_numerica_e_por_isso_ela_nao_basta() {
        let mut w = vec![0u8; 0x40];
        w[0x20..0x28].copy_from_slice(b"6_shard_"); // virou "indexTable"
        w[0x28..0x2c].copy_from_slice(&112u32.to_le_bytes());
        w[0x2c..0x30].copy_from_slice(&696842u32.to_le_bytes());
        w[0x30..0x38].copy_from_slice(b"quickmel"); // virou "nodes"
        w[0x3c..0x40].copy_from_slice(&40u32.to_le_bytes());
        // Passa: é exatamente o falso-positivo observado ao vivo. Só a leitura de memória o mata.
        assert!(res_loader_signature(&w, 64).is_some());
    }
}

#[cfg(test)]
mod res_loader_sdk_signature_tests {
    use crate::diagnostico::res_loader_signature_sdk;

    fn win_sdk(index_table: u64, size: u32, cap: u32, nodes: u64, stride: u32) -> Vec<u8> {
        let mut w = vec![0u8; 0x40];
        w[0x00..0x08].copy_from_slice(&index_table.to_le_bytes());
        w[0x08..0x0c].copy_from_slice(&size.to_le_bytes());
        w[0x0c..0x10].copy_from_slice(&cap.to_le_bytes());
        w[0x10..0x18].copy_from_slice(&nodes.to_le_bytes());
        w[0x1c..0x20].copy_from_slice(&stride.to_le_bytes());
        w
    }

    #[test]
    fn layout_do_sdk_aceito() {
        assert_eq!(
            res_loader_signature_sdk(&win_sdk(0x7b_1157_c4d0, 2000, 4096, 0x7b_1157_e000, 32), 64),
            Some((2000, 4096, 32))
        );
    }

    // Mesmos invariantes da outra variante — o que muda são só os offsets.
    #[test]
    fn invariantes_valem_igual_no_layout_do_sdk() {
        assert!(res_loader_signature_sdk(&win_sdk(0x7b_0000_0000, 9000, 64, 0x7b_0001_0000, 32), 1).is_none());
        assert!(res_loader_signature_sdk(&win_sdk(0x1234, 100, 128, 0x7b_0001_0000, 32), 1).is_none());
        assert!(res_loader_signature_sdk(&win_sdk(0x7b_0000_0000, 100, 128, 0x7b_0001_0000, 0), 1).is_none());
    }
}

/// Nós de um `red::HashMap` do motor (índice `+0`, capacidade `+0xc`, nós `+0x10`, passo `+0x1c`), pelas cadeias de todos os baldes.
pub(crate) unsafe fn mapa_do_motor_nos(mapa: usize) -> Vec<usize> {
    let mut v = Vec::new();
    if mapa == 0 || !crate::memoria::is_readable(mapa as *const c_void, 0x20) {
        return v;
    }
    let indice = ler_u64(mapa as u64) as usize;
    let cap = ((mapa + 0xc) as *const u32).read_unaligned() as usize;
    let nos = ler_u64(mapa as u64 + 0x10) as usize;
    let passo = ((mapa + 0x1c) as *const u32).read_unaligned() as usize;
    if indice == 0 || nos == 0 || cap == 0 || cap > 1 << 22 || !(0x10..=0x100).contains(&passo) || !crate::memoria::is_readable(indice as *const c_void, cap * 4) {
        return v;
    }
    for b in 0..cap {
        let mut i = ((indice + b * 4) as *const u32).read_unaligned();
        let mut guarda = 0;
        while i != u32::MAX && guarda <= cap {
            guarda += 1;
            let no = nos + i as usize * passo;
            if !crate::memoria::is_readable(no as *const c_void, passo) {
                break;
            }
            v.push(no);
            i = (no as *const u32).read_unaligned();
        }
    }
    v
}

/// Cabeçalho do `red::HashMap<u64, T*>`: `(indexTable, nodes, size, capacity, passo)`; `None` com tabela ou nós nulos ou passo menor que o nó.
pub(crate) unsafe fn hashmap_cabecalho(mapa: *const u8) -> Option<(*const u32, *const u8, u32, u32, usize)> {
    let indices = rd_u64(mapa as *const c_void, 0) as *const u32;
    let nos = rd_u64(mapa as *const c_void, 0x10) as *const u8;
    let (tamanho, capacidade) = ((mapa.add(0x08) as *const u32).read_unaligned(), (mapa.add(0x0c) as *const u32).read_unaligned());
    let passo = (mapa.add(0x1c) as *const u32).read_unaligned() as usize;
    (!indices.is_null() && !nos.is_null() && capacidade > 0 && passo >= 0x18).then_some((indices, nos, tamanho, capacidade, passo))
}

/// `HashMap<u64, T*>::Get(chave)` sobre o layout do `red::HashMap` (nó: `next` `+0x00`, `hashedKey` `+0x04`, chave `+0x08`, valor `+0x10`), com o hash
/// dos 32 B baixos da chave e o bucket `hash % capacity`, como a busca do motor (`0x1045e3470`).
pub(crate) unsafe fn hashmap_get(mapa: *const u8, chave: u64) -> Option<u64> {
    let (indices, nos, tamanho, capacidade, passo) = hashmap_cabecalho(mapa)?;
    let hash = chave as u32;
    let mut i = indices.add((hash % capacidade) as usize).read_unaligned();
    for _ in 0..=tamanho {
        if i == u32::MAX {
            return None;
        }
        let no = nos.add(i as usize * passo);
        if (no.add(4) as *const u32).read_unaligned() == hash && (no.add(8) as *const u64).read_unaligned() == chave {
            return Some((no.add(0x10) as *const u64).read_unaligned());
        }
        i = (no as *const u32).read_unaligned();
    }
    None
}

/// `f` com a trava compartilhada do `SharedSpinLock` tomada pelo motor (`0x1000015c8`), solta com −1.
pub(crate) unsafe fn com_trava_de_leitura<T>(trava: *mut u8, f: impl FnOnce() -> T) -> T {
    crate::jogo::travas::travar_leitura((trava as *mut c_void) as *mut u8);
    let r = f();
    crate::jogo::travas::soltar_leitura((trava) as *mut u8);
    r
}

#[cfg(test)]
mod hashmap_do_motor_tests {
    use super::hashmap_get;
    use crate::diagnostico::c82_mapa_entradas;

    /// Mapa montado como o `red::HashMap<u64, T*>`, com 5 chaves em 4 buckets (há cadeia).
    #[test]
    fn mapa_do_motor_acha_pela_chave_e_enumera_as_cadeias() {
        let chaves = [0x1111u64, 0x2222, 0x3333, 0x4444, 0x5555];
        let capacidade = 4u32;
        let hash = |k: u64| k as u32;
        let mut indices = vec![u32::MAX; capacidade as usize];
        let mut nos = vec![0u8; chaves.len() * 0x18];
        for (i, k) in chaves.iter().enumerate() {
            let b = (hash(*k) % capacidade) as usize;
            let no = &mut nos[i * 0x18..(i + 1) * 0x18];
            no[0..4].copy_from_slice(&indices[b].to_le_bytes());
            no[4..8].copy_from_slice(&hash(*k).to_le_bytes());
            no[8..16].copy_from_slice(&k.to_le_bytes());
            no[16..24].copy_from_slice(&(0xA000 + i as u64).to_le_bytes());
            indices[b] = i as u32;
        }
        assert!(indices.iter().filter(|i| **i != u32::MAX).count() < chaves.len());
        let mut mapa = [0u8; 0x30];
        mapa[0..8].copy_from_slice(&(indices.as_ptr() as u64).to_le_bytes());
        mapa[8..12].copy_from_slice(&(chaves.len() as u32).to_le_bytes());
        mapa[12..16].copy_from_slice(&capacidade.to_le_bytes());
        mapa[16..24].copy_from_slice(&(nos.as_ptr() as u64).to_le_bytes());
        mapa[0x1c..0x20].copy_from_slice(&0x18u32.to_le_bytes());
        unsafe {
            for (i, k) in chaves.iter().enumerate() {
                assert_eq!(hashmap_get(mapa.as_ptr(), *k), Some(0xA000 + i as u64));
            }
            assert_eq!(hashmap_get(mapa.as_ptr(), 0x9999), None);
            let mut e = c82_mapa_entradas(mapa.as_ptr());
            e.sort();
            assert_eq!(e, chaves.iter().enumerate().map(|(i, k)| (*k, 0xA000 + i as u64)).collect::<Vec<_>>());
        }
    }
}

// SAFETY: só dados e endereços do motor, usados na game thread.
unsafe impl Send for CustMescla {}



/// Base da `RuntimeScene` e de onde veio: a capturada no `OnWorldAttached` (só com o smoke que instala o hook) ou `[CGameFramework+0x18]`
/// (o montador `0x101577ba8` grava ali o objeto de 0x4b8 B) com o framework do job de inicialização.
/// A do `OnWorldAttached` é zerada pelo getter da phase (`inicio::boot::phase_skip_getter`) quando a fase sai de 5, que é o que acontece na
/// volta ao menu (5 -> 3, `proofs/20260919-boot311-console-completo.log:2767-2854`; volta ao menu pedida em `:3464`). O getter só é hookado com
/// `pular_intro` e se `install_phase_skip_near4` aceitar o alvo. Sem o gancho, ou numa troca de sessão que não passe por fase fora de 5
/// (carregar save pelo menu de pausa), ela não é zerada [não medido]. Na 1ª sessão
/// do boot o hook veio depois do `phase=5` (prova do `OnWorldAttached` de 2026-08-18 em `proofs/`, linhas 1339-1340); numa sessão
/// seguinte o momento dele é [não medido]. A do framework não fica guardada: o campo é lido a cada chamada.
pub(crate) unsafe fn cena_base() -> (usize, &'static str) {
    let owa = IGS_OWA_SCENE_PTR.load(Ordering::Relaxed) as usize;
    if owa != 0 {
        return (owa, "OnWorldAttached");
    }
    let campo = crate::inicio::boot::CGAMEFRAMEWORK_ATUAL.load(Ordering::Relaxed) as usize + 0x18;
    if campo == 0x18 || !crate::memoria::is_readable(campo as *const c_void, 8) {
        return (0, "");
    }
    ((campo as *const u64).read_unaligned() as usize, "framework+0x18")
}

// Índices da tabela "O mapa" de `cp77-symbols/notes/MAPA-RUNTIMESCENE.md` (levantada ao vivo em 2026-08-22, nomes pela RTTI).
pub(crate) const RUNTIME_SYSTEMS_DO_MAPA: &[usize] = &[
    0, 1, 2, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 18, 19, 20, 23, 26, 27, 29, 30, 31, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 47, 48, 51, 52,
    59, 62, 63, 64,
];

pub(crate) fn runtime_system_no_mapa(index: usize) -> bool {
    RUNTIME_SYSTEMS_DO_MAPA.contains(&index)
}

/// Runtime system por ÍNDICE, a partir da `RuntimeScene` de `cena_base`.
///
/// A ARITMÉTICA:
/// `offset = índice * sizeof(Handle<T>)` = `índice * 16`, e o valor ali é um `Handle` cujo
/// primeiro campo é a instância. O único offset que o projeto já conhecia — `Weather@0x280`
/// — é **exatamente** `40 * 16`, e 40 é o índice mapeado
/// para `worldRuntimeSystemWeather`. Bate na conta, o que sustenta que a
/// cena capturada é a base certa.
///
/// Índices que interessam: 31 = `workWorkspotSystem` (item `#82`),
/// 35 = `worldNodeInstanceRegistry` (destrava `#80`/`#84` — é a rota nunca tentada para obter um
/// NodeRef/spawner real), 40 = `worldRuntimeSystemWeather` (o controle).
///
/// Só lê índice do mapa (`RUNTIME_SYSTEMS_DO_MAPA`): fora dele o slot não tem runtime system conhecido, e a guarda abaixo não prova que o
/// ponteiro é objeto RTTI — a varredura cega da cena passava cada ponteiro por uma guarda do mesmo tipo (ponteiro e vtable plausíveis) e
/// derrubou o jogo em 2026-09-30 (`proofs/20260930-c39-crash-no-comando-scenedump.ips`).
///
/// Devolve `(instância, nome-da-classe)`. Nome vazio = não resolveu como objeto RTTI.
pub(crate) unsafe fn runtime_system_at(index: usize) -> (u64, String) {
    if !runtime_system_no_mapa(index) {
        return (0, String::new());
    }
    let (base, _) = cena_base();
    if base == 0 {
        return (0, String::new());
    }
    let slot = base + index * 16;
    if !crate::memoria::is_readable(slot as *const c_void, 16) {
        return (0, String::new());
    }
    let inst = (slot as *const u64).read_unaligned();
    // Guarda antes do `class_of`: ponteiro plausível E com vtable na imagem, senão `class_of`
    // dereferencia lixo e derruba o jogo (categoria que já custou 2 crashes aqui).
    if inst < 0x1_0000_0000 || inst >= 0x8000_0000_0000 || inst & 7 != 0
        || !crate::memoria::is_readable(inst as *const c_void, 8)
    {
        return (0, String::new());
    }
    let vt = (inst as *const u64).read_unaligned() as usize;
    if vt < 0x1_0000_0000 || vt >= 0x1_1000_0000 || vt & 7 != 0 || !crate::memoria::is_readable(vt as *const c_void, 8) {
        return (inst, String::new());
    }
    let cls = crate::rtti::class_of(inst as *mut c_void);
    if cls.is_null() {
        return (inst, String::new());
    }
    let h = crate::rtti::type_name_getname(cls);
    if h == 0 {
        return (inst, String::new());
    }
    (inst, crate::cname::resolve_cname(h))
}

#[cfg(test)]
mod mapa_da_runtimescene_tests {
    use super::{runtime_system_no_mapa, RUNTIME_SYSTEMS_DO_MAPA};

    // Os 42 do `MAPA-RUNTIMESCENE.md`, em ordem, com os índices de quem chama dentro.
    #[test]
    fn indices_de_quem_chama_estao_no_mapa() {
        assert_eq!(RUNTIME_SYSTEMS_DO_MAPA.len(), 42);
        assert!(RUNTIME_SYSTEMS_DO_MAPA.windows(2).all(|w| w[0] < w[1]));
        for i in [0, crate::mundo::WS_STREAMING_IDX, crate::npcs::C82_SISTEMA_INDICE, 35, 40, 42, 64] {
            assert!(runtime_system_no_mapa(i), "índice {i}");
        }
    }

    // Buracos da tabela e a faixa depois do último índice do mapa: a varredura cega logou até +0x400
    // (índice 64) e morreu depois; o ponto exato é [não medido].
    #[test]
    fn fora_do_mapa_recusado() {
        for i in [3, 4, 15, 21, 22, 24, 25, 28, 32, 43, 46, 49, 50, 53, 58, 60, 61, 65, 66, 100, usize::MAX] {
            assert!(!runtime_system_no_mapa(i), "índice {i}");
        }
    }
}

#[cfg(test)]
mod auto_referencia_tests {
    use super::auto_referencia_confere;

    #[test]
    fn handle_de_serializavel_confere() {
        assert!(auto_referencia_confere(0x70_1695_17f0, 0x70_2000_0010, 0x70_1695_17f0, 0x70_2000_0010));
    }

    // Objeto C++ comum: as palavras em +0x08/+0x10 são dados quaisquer, e o vt+0x08 dele é o destrutor com delete (boot 134).
    #[test]
    fn objeto_sem_auto_referencia_recusado() {
        assert!(!auto_referencia_confere(0x70_1695_17f0, 0x70_2000_0010, 0x70_1695_1800, 0x70_2000_0010));
        assert!(!auto_referencia_confere(0x70_1695_17f0, 0x70_2000_0010, 0x70_1695_17f0, 0x70_2000_0020));
        assert!(!auto_referencia_confere(0x70_1695_17f0, 0, 0x70_1695_17f0, 0));
        assert!(!auto_referencia_confere(0, 0x70_2000_0010, 0, 0x70_2000_0010));
    }
}

/// Corpo das natives de PROVA no build público. Consome o frame pelos metadados da própria função
/// (sem isso uma native usada inline devolve lixo — boot 233) e **não escreve o retorno**: sem o tipo
/// em mão, escrever 8 bytes num slot de 1 (Bool) corromperia o frame do chamador, e nenhum `.reds` do
/// pacote declara estas natives, então ninguém lê esse retorno.
pub(crate) unsafe extern "C" fn tramp_corpo_de_prova(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    rtti::read_params_consuming(current_native_func(), frame);
}

/// `rd_u64` com o endereço já somado, como `u64`.
pub(crate) unsafe fn ler_u64(a: u64) -> u64 {
    rd_u64(a as *const c_void, 0)
}

// TODO(F2-04, D7 em tmp/decisoes-do-dono-20261003.md): o estado por entidade das natives de `BwmsAparencia` não é solto
// quando a entidade ou o mundo saem, e uma entidade nova no mesmo endereço herda o que a anterior deixou. O lugar de esvaziar tudo é
// antes do `GarmentAssembler::OnGameDetach` do motor (o estado dinâmico vai junto com o da entidade). Aqui o `OnGameDetach` (`0x102006b0c`) só existe
// como sonda dev atrás do marcador `ongamedetach`, com 1 disparo natural na prova de 2026-07-29 (`OnGameDetach` isolado) e sem
// registro do momento. Os hooks do ciclo de vida (`entidades::ciclo_detach_hook`/`ciclo_uninitialize_hook`/`ciclo_dispose_hook`) só entram com ouvinte
// de `Entity/*`, e o `Detach` não é a posição: o estado de entidade que sai do mundo e volta tem de ser mantido. Falta, antes de ligar: o hook no
// produto, prova de que ele dispara em toda saída de sessão (carregar save em jogo, voltar ao menu) e só nela, e a thread em que roda. A soltura
// esvazia de uma vez `es20_componentes`, `es20_originais`, `es20_componentes_da_parte`, `ES22_COMPONENT_APPEARANCE`, `ES22_RESOURCE_STATE`,
// `ES67_RESOURCE_OFFSET`, `es20_estados_dinamicos` e `ES20_FILA_DE_ESTADO`. O teste abaixo trava o vazamento de hoje; com a soltura ligada, os
// asserts de herança viram "nada herdado depois da soltura".
#[cfg(test)]
mod estado_sem_gatilho_de_saida_tests {
    use super::{
        es20_adicionar_mascara, es20_decidir_mascara, es20_esquecer, es20_id_do_componente,
        es20_ligar_componente, es20_nome_base, es20_nomes_da_parte, es20_original, es22_adicionar_aparencia_do_componente_vivo,
        es22_decidir_aparencia, es22_ligar_componente_vivo, es22_ligar_parte, ES22_RESOURCE_STATE,
    };
    use crate::cname::CName;
    use crate::diagnostico::{es20_esquecer_originais, es20_esquecer_partes, es22_esquecer};
    use std::ffi::c_void;

    const NOME: &str = "bwms_f204_componente";

    /// Componente em memória: o nome em `+0x40` e `appearanceName`/`appearancePath` em `+0x48`/`+0x68`, o `GetUniqueId`. Vazado: a loja é do
    /// processo e o endereço tem de ser único.
    fn componente() -> *mut u8 {
        let c: &'static mut [u64; 32] = Box::leak(Box::new([0u64; 32]));
        c[0x40 / 8] = crate::cname::intern(NOME);
        c[0x48 / 8] = 0xF204_00A1;
        c[0x68 / 8] = 0xF204_00B1;
        c.as_mut_ptr().cast()
    }

    #[test]
    fn entidade_nova_no_mesmo_endereco_herda_override_parte_e_originais_da_que_saiu() {
        let e = Box::leak(Box::new([0u64; 32])).as_mut_ptr() as u64;
        let parte = 0xF204_0030u64;
        // A 1ª entidade pelos caminhos das natives: `AddAppearanceOverrideForComponent`, `LinkComponentToPart`, `LinkPartToAppearanceName`, a
        // lista da parte do #20, as originais de aparência (0x999) e de máscara (0b1111) lidas na 1ª aplicação e uma tag que esconde o chunk 0.
        let velho = componente();
        let base = unsafe { es20_nome_base(velho) };
        assert!(unsafe { es22_adicionar_aparencia_do_componente_vivo(e as *mut c_void, velho.cast(), 0xAA, 0x111) });
        assert!(unsafe { es22_ligar_componente_vivo(e as *mut c_void, velho.cast(), parte) });
        es22_ligar_parte(e, parte, "bwms_f204_aparencia");
        unsafe { es20_ligar_componente(e, velho, parte) };
        let id = unsafe { es20_id_do_componente((0x48, 0x68), velho) };
        assert_eq!(es20_original(e, id, 1, || 0x999), 0x999);
        es20_adicionar_mascara(e, 0xAA, CName::lido_do_motor(base), !0b1, false);
        assert_eq!(unsafe { es20_decidir_mascara(e, velho, || es20_original(e, id, 0, || 0b1111)) }, Some(0b1110));
        // A entidade sai e nada no produto solta o estado. Entra outra no mesmo endereço, com componente de mesmo nome e mesmo `GetUniqueId`,
        // aparência 0x555 e máscara 0b0011, que nenhum mod tocou. Sem herança: nada a aplicar, sem parte nem `ResourceState`, originais dela.
        let novo = componente();
        let d = unsafe { es22_decidir_aparencia(e, novo, || 0x555) };
        assert_eq!((d.fontes, d.aparencia, d.parte), ((Some(0x111), None), Some(0x111), Some(parte)), "override e parte da que saiu");
        assert!(ES22_RESOURCE_STATE.lock().map(|g| g.iter().any(|(k, _)| *k == (e, parte))).unwrap_or(false), "ResourceState da que saiu");
        assert_eq!(es20_nomes_da_parte(e, parte), vec![crate::cname::intern(NOME)], "lista da parte da que saiu");
        let id_novo = unsafe { es20_id_do_componente((0x48, 0x68), novo) };
        assert_eq!(id_novo, id);
        assert_eq!(es20_original(e, id_novo, 1, || 0x555), 0x999, "original de aparência da que saiu");
        let mascara = unsafe { es20_decidir_mascara(e, novo, || es20_original(e, id_novo, 0, || 0b0011)) };
        assert_eq!(mascara, Some(0b1110), "tag e original de máscara da que saiu: sem herança daria None; só com a original nova, 0b0010");
        // Limpeza do teste; no produto só o selftest chama estas funções.
        es20_esquecer(e);
        es20_esquecer_originais(e, id);
        es20_esquecer_partes(e);
        es22_esquecer(e, &[base], &[parte]);
    }
}

#[cfg(test)]
mod testes_heranca_de_classe {
    use super::{class_is_kind_of, classe_deriva_de_nome};
    use std::ffi::c_void;

    /// `CClass` de teste: `+0x10` = mãe, `+0x18` = hash do nome. O `+0x08` aponta a isca, uma classe fora da cadeia, para que andar pelo campo
    /// errado dê outro resultado.
    fn classe(mae: *const [u64; 4], nome: u64, isca: *const [u64; 4]) -> Box<[u64; 4]> {
        Box::new([0, isca as u64, mae as u64, nome])
    }

    /// As duas caminhadas de pais do produto sobem por `CClass+0x10`: a filha casa a mãe e a avó pelo ponteiro (`class_is_kind_of`) e pelo nome
    /// nativo (`classe_deriva_de_nome`), não casa a isca do `+0x08`, a avó não casa a filha, e ponteiro fora de faixa (a `CLASSE_NAO_RESOLVIDA`
    /// da fábrica de alvo), nulo ou ciclo não casa nem trava.
    #[test]
    fn caminhadas_de_pais_sobem_por_0x10() {
        const NOME_ISCA: u64 = 0x15CA;
        const NOME_AVO: u64 = 0xA0A0;
        const NOME_MAE: u64 = 0xBEBE;
        const NOME_FILHA: u64 = 0xF1F1;
        let isca = classe(std::ptr::null(), NOME_ISCA, std::ptr::null());
        let avo = classe(std::ptr::null(), NOME_AVO, std::ptr::null());
        let mae = classe(&*avo, NOME_MAE, &*isca);
        let filha = classe(&*mae, NOME_FILHA, &*isca);
        let mut ciclo = classe(std::ptr::null(), 0xC1C1, std::ptr::null());
        ciclo[2] = &*ciclo as *const [u64; 4] as u64;
        let p = |c: &[u64; 4]| c as *const [u64; 4] as *mut c_void;
        unsafe {
            assert!(class_is_kind_of(p(&filha), p(&mae)));
            assert!(class_is_kind_of(p(&filha), p(&avo)));
            assert!(class_is_kind_of(p(&filha), p(&filha)));
            assert!(!class_is_kind_of(p(&avo), p(&filha)));
            assert!(!class_is_kind_of(p(&filha), p(&isca)));
            assert!(!class_is_kind_of(p(&filha), crate::corpo_e_rosto::CLASSE_NAO_RESOLVIDA as *mut c_void));
            assert!(!class_is_kind_of(std::ptr::null_mut(), p(&avo)));
            assert!(!class_is_kind_of(p(&ciclo), p(&avo)));
            assert!(classe_deriva_de_nome(p(&filha), NOME_MAE));
            assert!(classe_deriva_de_nome(p(&filha), NOME_AVO));
            assert!(classe_deriva_de_nome(p(&filha), NOME_FILHA));
            assert!(!classe_deriva_de_nome(p(&filha), NOME_ISCA));
            assert!(!classe_deriva_de_nome(p(&mae), NOME_FILHA));
            assert!(!classe_deriva_de_nome(std::ptr::null_mut(), NOME_AVO));
            assert!(!classe_deriva_de_nome(p(&ciclo), NOME_AVO));
        }
    }
}

#[cfg(test)]
mod raiz_do_builder_tests {
    use super::{eb_raiz_e_componentes, EB_EXTRATOR, EB_TEMPLATE, EXT_RESULTADOS};

    /// Vetores `[B13]`: resultados [A, B, C, D] (handles com o contador `0x11..0x14`), `rootIndex` (i16) em `template+0x1d8`.
    #[test]
    fn raiz_e_o_resultado_do_root_index_e_os_componentes_vem_depois() {
        let resultados = [0x1000u64, 0x11, 0x2000, 0x12, 0x3000, 0x13, 0x4000, 0x14];
        let rodar = |raiz: i16, com_extrator: bool, com_template: bool| {
            let mut extrator = [0u64; 0x18];
            extrator[EXT_RESULTADOS as usize / 8] = resultados.as_ptr() as u64;
            extrator[EXT_RESULTADOS as usize / 8 + 1] = 4 | 4 << 32;
            let mut template = [0u64; 0x3e];
            template[0x1d8 / 8] = u64::from(raiz as u16);
            let mut builder = [0u64; 0x20];
            if com_extrator {
                builder[EB_EXTRATOR as usize / 8] = extrator.as_ptr() as u64;
            }
            if com_template {
                builder[EB_TEMPLATE as usize / 8] = template.as_ptr() as u64;
            }
            let (r, comps) = unsafe { eb_raiz_e_componentes(builder.as_ptr() as u64) };
            (r, comps.iter().map(|h| h[0]).collect::<Vec<_>>())
        };
        assert_eq!(rodar(1, true, true), (0x2000, vec![0x3000, 0x4000]));
        assert_eq!(rodar(0, true, true), (0x1000, vec![0x2000, 0x3000, 0x4000]));
        assert_eq!(rodar(3, true, true), (0x4000, vec![]));
        assert_eq!(rodar(4, true, true), (0, vec![]));
        assert_eq!(rodar(-1, true, true), (0, vec![]));
        assert_eq!(rodar(1, false, true), (0, vec![]));
        assert_eq!(rodar(1, true, false), (0, vec![]));
    }

    /// Os componentes saem com o handle inteiro (objeto e contador), como o `array<ref<IComponent>>` que o script recebe.
    #[test]
    fn componentes_levam_o_contador_do_handle() {
        let resultados = [0x1000u64, 0x11, 0x2000, 0x12];
        let mut extrator = [0u64; 0x18];
        extrator[EXT_RESULTADOS as usize / 8] = resultados.as_ptr() as u64;
        extrator[EXT_RESULTADOS as usize / 8 + 1] = 2 | 2 << 32;
        let template = [0u64; 0x3e];
        let mut builder = [0u64; 0x20];
        builder[EB_EXTRATOR as usize / 8] = extrator.as_ptr() as u64;
        builder[EB_TEMPLATE as usize / 8] = template.as_ptr() as u64;
        assert_eq!(unsafe { eb_raiz_e_componentes(builder.as_ptr() as u64) }, (0x1000, vec![[0x2000, 0x12]]));
    }
}
