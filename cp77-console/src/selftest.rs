//! Self-test DEV-GATED do hooking + diagnóstico do ArchiveXL.
//!
//! TUDO aqui só roda sob `crate::dev_mode()` (env `BWMS_DEV` ou `/tmp/bwms-dev`).
//! No build/boot de PRODUÇÃO nada disto instala hook, lê memória do jogo ou loga —
//! o jogo do usuário fica intacto.
//!
//! Três peças:
//!   1. **Relocador (inline hook, type 1):** instala em um getter-folha do pool
//!      (`PoolStorageProxy<PoolRoot>::GetHandle`, 4 instr `adrp/add/ldr/ret`),
//!      confirma que o relocador converteu o `adrp` do prólogo em `movz/movk`
//!      no trampolim, e DESINSTALA na hora. A função-alvo NUNCA é chamada.
//!   2. **vtable (type 2):** se houver uma vtable RTTI viva e segura, troca um slot,
//!      confirma a troca e restaura. Conservador: sem alvo 100% seguro → só loga
//!      que precisa de runtime com objeto vivo (não arrisca).
//!   3. **Diagnóstico ArchiveXL:** instala em `PoolStorageProxy<PoolArchive>::Allocate`
//!      um replacement OBSERVE-ONLY (shim naked p/ capturar x30 = ret-addr do caller),
//!      loga os ~8 primeiros callers (rebaseados p/ vmaddr) e SEMPRE chama a original.
//!
//! Segurança transversal: guarda de legibilidade + guarda de prólogo (compara os 16
//! bytes com o esperado) ANTES de hookar; aborta limpo se o binário mudou (patch).
//! Hook reversível (`Interceptor::revert`). Patch via VM_PROT_COPY (COW) só na cópia
//! do processo.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};

use crate::gum::{self, Interceptor};

// ===================== Alvos estáticos (vmaddr; base de link 0x100000000) =====================

/// `red::memory::PoolStorageProxy<red::memory::PoolRoot>::GetHandle()` @ 0x10001d9a0.
/// Folha pura `adrp x8 / add x8 / ldr w0,[x8] / ret`. O `adrp` no 1º slot exercita o
/// relocador (vira `movz/movk`). NUNCA é chamada pelo self-test.
const RELOC_TARGET_VM: u64 = 0x1_0001_d9a0;
/// Prólogo esperado (4 instr, LE u32): adrp / add / ldr / ret.
const RELOC_PROLOGUE: [u32; 4] = [0x90046d48, 0x910f6108, 0xb9402100, 0xd65f03c0];

/// `red::memory::PoolStorageProxy<archive::PoolArchive>::Allocate(unsigned long long)`
/// @ 0x103e2f17c. Prólogo `sub sp / stp x20,x19 / stp x29,x30 / add x29` — 16 bytes
/// 100% não-PC-relativos → relocador copia verbatim. O `adrp` PC-relativo está em +0x14
/// (5ª instr), FORA da janela de 16 bytes.
const ARCHIVE_ALLOC_VM: u64 = 0x1_03e2_f17c;
/// Prólogo esperado (4 instr, LE u32): sub / stp / stp / add.
const ARCHIVE_ALLOC_PROLOGUE: [u32; 4] = [0xd100c3ff, 0xa9014ff4, 0xa9027bfd, 0x910083fd];

// ===================== ABI do Allocate =====================

/// ABI nativa do `Allocate`: `(size: u64) -> *mut c_void`. x0 = size, retorno em x0.
type AllocFn = unsafe extern "C" fn(u64) -> *mut c_void;

/// Trampolim do original (devolvido por `Interceptor::replace`), tipado como `AllocFn`.
static ORIG_ALLOC: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// Contador de chamadas observadas (limita o log a N callers → sem flood).
static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
/// Quantos primeiros callers logar.
const ALLOC_LOG_N: u64 = 8;

// ===================== Entrada pública =====================

/// Roda os 3 self-tests, SÓ se `crate::dev_mode()`. No-op em produção.
/// Deve ser chamada DEPOIS do hook do executor estar instalado (do selfboot).
pub fn run_dev_selftests() {
    if !crate::dev_mode() {
        return; // produção: nada roda, jogo intacto
    }
    crate::log("[selftest] === início (DEV) ===");
    unsafe {
        // ArchiveXL diag PRIMEIRO (o valioso: instala o hook observe-only; não pode ser
        // bloqueado por um teste posterior).
        install_archivexl_diag();
        // Path B (ArchiveXL): HookAfter InitializeArchives → dump do ResourceGameDepot.
        install_initarchives_diag();
        // ArchiveXL: HookBefore open-archive → loga o PATH de cada .archive carregado (prova
        // que os nossos carregam + mapeia a função do Path B).
        install_openarchive_diag();
        // Relocador (type 1) — já provado in-game.
        test_relocator();
        // vtable (type 2): vtable_hook/unhook está pronto e compila. O teste-vivo antigo
        // (test_vtable) protegia memória do STACK (inválido -> travava o fluxo). Valida-se
        // contra uma vtable real __DATA_CONST do jogo na fase do Codeware UI, não numa array falsa.
        crate::log("[selftest] vtable: função pronta (vtable_hook/unhook), valida-se contra vtable real do jogo (Codeware UI)");
    }
    crate::log("[selftest] === fim ===");
}

// ===================== 1) Relocador (inline hook, type 1) =====================

/// Replacement-dummy do teste do relocador. NUNCA é chamado (o alvo é hookado e
/// revertido sem nunca executar a função). Existe só pra ter um ponteiro válido.
extern "C" fn reloc_dummy_repl() {}

unsafe fn test_relocator() {
    let target = crate::rebase(RELOC_TARGET_VM);

    // GUARD 1: legibilidade dos 16 bytes.
    if !gum::is_readable(target as *const c_void, 16) {
        crate::log("[selftest] relocador: alvo ilegível -> pulado (sem hook)");
        return;
    }
    // GUARD 2: prólogo bate com o esperado? Se um patch moveu/mudou a função, ABORTA
    // sem hookar (não corrompe nada).
    if !prologue_matches(target, &RELOC_PROLOGUE) {
        crate::log("[selftest] relocador: prólogo não casou (patch?) -> abortado (sem hook)");
        return;
    }

    let it = Interceptor::obtain();
    match it.replace(target, reloc_dummy_repl as *mut c_void) {
        Some(tramp) => {
            // O 1º slot do alvo é um `adrp` → o relocador deve materializá-lo como
            // movz/movk no início do trampolim. Confirma que tramp[0..4] é um `movz`.
            // movz (64-bit): 0xD28xxxxx (bits[31:23] = 0b110100101). Máscara 0xFF80_0000.
            let mut first = [0u8; 4];
            std::ptr::copy_nonoverlapping(tramp as *const u8, first.as_mut_ptr(), 4);
            let insn = u32::from_le_bytes(first);
            let is_movz = (insn & 0xFF80_0000) == 0xD280_0000;

            // DESINSTALA IMEDIATAMENTE — nunca deixa o alvo hookado, nunca chama a fn.
            it.revert(target);

            if is_movz {
                crate::log(&format!(
                    "[selftest] relocador OK em PoolStorageProxy<PoolRoot>::GetHandle (adrp relocado -> movz {insn:#010x})"
                ));
            } else {
                crate::log(&format!(
                    "[selftest] relocador FALHOU: tramp[0..4]={insn:#010x} não é movz (adrp não relocado?)"
                ));
            }
        }
        None => {
            crate::log("[selftest] relocador FALHOU: Interceptor::replace devolveu None");
        }
    }
    std::mem::forget(it); // Interceptor é ZST; evita qualquer Drop implícito
}

// ===================== 2) vtable (type 2) =====================
// Teste-vivo REMOVIDO: protegia memória do STACK (mach_vm_protect numa array local) —
// inválido, e travava o fluxo do self-test antes do diagnóstico. vtable_hook/unhook
// (gum.rs) é p/ vtable real __DATA_CONST; valida-se contra uma vtable do jogo na fase
// do Codeware UI (slot ocioso comprovado), não numa array falsa.

// ===================== 3) Diagnóstico ArchiveXL (observe-only) =====================

// ===== Path B (ArchiveXL): HookAfter InitializeArchives — dump do ResourceGameDepot =====
// InitializeArchives(ResourceGameDepot* this) @ vmaddr 0x103ed96b0 (cadeia mapeada em
// six-core-mods-status.md). this+0x68 = DynArray<ArchiveSet> (entries@0x68, size@0x74, stride 0x140).
// 1º passo do Path B: provar que hookamos o LOADER + acessamos o depot — base p/ carregar nossos
// .archive de pasta própria (com ordem controlada), em vez de depender do glob basegame_*.
// Observe-only: chama a original (HookAfter) e só LÊ o depot.
const INITARCH_VM: u64 = 0x1_03ed_96b0;
static ORIG_INITARCH: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

unsafe extern "C" fn initarch_replacement(this: *mut c_void) {
    // HookAfter: roda a original PRIMEIRO (x0 = this chega cru; o abs-jump usa x17).
    let orig = ORIG_INITARCH.load(Ordering::Relaxed);
    if !orig.is_null() {
        let f: unsafe extern "C" fn(*mut c_void) = std::mem::transmute(orig);
        f(this);
    }
    if this.is_null() || !gum::is_readable(this as *const c_void, 0x80) {
        crate::log("[initarch-diag] depot ilegível");
        return;
    }
    let entries = core::ptr::read_unaligned((this as *const u8).add(0x68) as *const u64) as *const u8;
    let count = core::ptr::read_unaligned((this as *const u8).add(0x74) as *const u32);
    crate::log(&format!(
        "[initarch-diag] InitializeArchives HOOKADO ✓ depot={this:p} (static {:#x}) ArchiveSets entries={entries:p} count={count}",
        crate::un_rebase(this)
    ));
    if !entries.is_null() && gum::is_readable(entries as *const c_void, 0x10) {
        for i in 0..count.min(3) as usize {
            let set = entries.add(i * 0x140);
            if !gum::is_readable(set as *const c_void, 0x10) {
                break;
            }
            let q0 = core::ptr::read_unaligned(set as *const u64);
            let q1 = core::ptr::read_unaligned(set.add(8) as *const u64);
            crate::log(&format!("[initarch-diag]   set[{i}] @ {set:p}: q0={q0:#x} q1={q1:#x}"));
        }
    }
}

unsafe fn install_initarchives_diag() {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    let target = crate::rebase(INITARCH_VM);
    if !gum::is_readable(target as *const c_void, 16) {
        crate::log("[initarch-diag] alvo InitializeArchives ilegível -> sem hook");
        return;
    }
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }
    let it = Interceptor::obtain();
    match it.replace(target, initarch_replacement as *mut c_void) {
        Some(tramp) => {
            ORIG_INITARCH.store(tramp, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log("[initarch-diag] hook instalado em InitializeArchives (HookAfter, observe-only)");
        }
        None => {
            INSTALLED.store(false, Ordering::Relaxed);
            crate::log("[initarch-diag] FALHA ao hookar InitializeArchives (replace None)");
        }
    }
}

// ===== ArchiveXL: HookBefore open-archive — loga o PATH de cada .archive =====
// open archive @ vmaddr 0x103e2ebd4 (x0=ArchiveInfo*, x1=path, x2=group). HookBefore observe-only:
// loga x1 (path) e chama a original. Prova quais .archive o engine carrega (inclui os nossos
// basegame_zzbwms_*) + é a função que o Path B chamaria por .archive nosso.
const OPENARCH_VM: u64 = 0x1_03e2_ebd4;
static ORIG_OPENARCH: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static OPENARCH_N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Tenta ler o path em `x1` como string: 1º como C-string em x1, depois x1 como ptr-p/-string.
unsafe fn read_path_arg(x1: *mut c_void) -> String {
    let try_cstr = |p: *const u8| -> String {
        let mut s = String::new();
        for i in 0..256 {
            if !gum::is_readable(p.add(i) as *const c_void, 1) {
                break;
            }
            let b = *p.add(i);
            if b == 0 {
                break;
            }
            if b.is_ascii_graphic() || b == b' ' {
                s.push(b as char);
            } else {
                break;
            }
        }
        s
    };
    if x1.is_null() || !gum::is_readable(x1 as *const c_void, 8) {
        return "<null>".into();
    }
    let s = try_cstr(x1 as *const u8);
    if s.len() >= 3 {
        return s;
    }
    let inner = (x1 as *const *const u8).read_unaligned();
    if !inner.is_null() && gum::is_readable(inner as *const c_void, 1) {
        let s2 = try_cstr(inner);
        if s2.len() >= 3 {
            return format!("(via ptr) {s2}");
        }
    }
    "<não-string>".into()
}

type OpenArchFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, u64, u64) -> *mut c_void;

unsafe extern "C" fn openarch_replacement(
    x0: *mut c_void,
    x1: *mut c_void,
    x2: *mut c_void,
    x3: u64,
    x4: u64,
) -> *mut c_void {
    let n = OPENARCH_N.fetch_add(1, Ordering::Relaxed);
    if n < 30 {
        // o glob (pattern+dir) vive em x0+0x78. Loga p/ ver todos os prefixos/dirs varridos
        // (incl. basegame_*.archive em content/ = onde nossos mods são pegos).
        let glob = read_path_arg((x0 as *mut u8).add(0x78) as *mut c_void);
        if glob.len() >= 6 {
            let ours = glob.contains("content") || glob.contains("zzbwms");
            crate::log(&format!(
                "[openarch-diag] glob#{n} '{glob}'{}",
                if ours { "  <<< dir/prefixo dos NOSSOS mods" } else { "" }
            ));
        }
    }
    if n < 6 {
        // DUMP RAW + scan de x0/x2 pra achar onde mora o path (string inline OU via ptr).
        crate::log(&format!(
            "[openarch-diag] #{n} RAW x0={x0:p} x1={x1:p} x2={x2:p} x3={x3:#x} x4={x4:#x}"
        ));
        for (name, base) in [("x0", x0), ("x1", x1), ("x2", x2)] {
            if base.is_null() || !gum::is_readable(base as *const c_void, 0x80) {
                continue;
            }
            for off in (0..0x80usize).step_by(8) {
                let s = read_path_arg((base as *mut u8).add(off) as *mut c_void);
                if s.len() >= 6 && (s.contains(".archive") || s.contains('/') || s.contains('_')) {
                    crate::log(&format!("[openarch-diag]   {name}+{off:#04x}: '{s}'"));
                }
            }
        }
    }
    let orig = ORIG_OPENARCH.load(Ordering::Relaxed);
    if orig.is_null() {
        return std::ptr::null_mut();
    }
    let f: OpenArchFn = std::mem::transmute(orig);
    f(x0, x1, x2, x3, x4)
}

unsafe fn install_openarchive_diag() {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    let target = crate::rebase(OPENARCH_VM);
    if !gum::is_readable(target as *const c_void, 16) {
        crate::log("[openarch-diag] alvo open-archive ilegível -> sem hook");
        return;
    }
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }
    let it = Interceptor::obtain();
    match it.replace(target, openarch_replacement as *mut c_void) {
        Some(tramp) => {
            ORIG_OPENARCH.store(tramp, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log("[openarch-diag] hook instalado em open-archive (HookBefore, loga paths)");
        }
        None => {
            INSTALLED.store(false, Ordering::Relaxed);
            crate::log("[openarch-diag] FALHA ao hookar open-archive (replace None)");
        }
    }
}

unsafe fn install_archivexl_diag() {
    let target = crate::rebase(ARCHIVE_ALLOC_VM);

    // GUARD 1: legibilidade.
    if !gum::is_readable(target as *const c_void, 16) {
        crate::log("[archivexl-diag] alvo Allocate ilegível -> abortado (sem hook)");
        return;
    }
    // GUARD 2: prólogo (sub/stp/stp/add). Se mudou (patch), aborta sem hookar.
    if !prologue_matches(target, &ARCHIVE_ALLOC_PROLOGUE) {
        crate::log("[archivexl-diag] prólogo de Allocate não casou (patch?) -> abortado (sem hook)");
        return;
    }
    // Idempotência: não instala 2x (run_dev_selftests poderia ser chamado de novo).
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }

    let it = Interceptor::obtain();
    match it.replace(target, alloc_replacement as *mut c_void) {
        Some(tramp) => {
            ORIG_ALLOC.store(tramp, Ordering::Relaxed);
            std::mem::forget(it); // mantém o hook vivo pela sessão
            crate::log(&format!(
                "[archivexl-diag] hook instalado em PoolArchive::Allocate (observe-only, vai logar {ALLOC_LOG_N} callers)"
            ));
        }
        None => {
            INSTALLED.store(false, Ordering::Relaxed);
            crate::log("[archivexl-diag] FALHA ao hookar Allocate (replace devolveu None)");
        }
    }
}

/// Shim NAKED: a 1ª instrução é literalmente a nossa — o compilador NÃO emite prólogo,
/// então x30 chega CRU do caller (o abs-jump do hook usa x17, não toca x30/sp). Movemos
/// x30 → x1 (2º arg) ANTES de qualquer branch que o clobbe e usamos `b` (tail-call, não
/// `bl`) pro body → x0 (size) intacto, x1 = caller_ret, frame preservado.
#[unsafe(naked)]
unsafe extern "C" fn alloc_replacement() {
    core::arch::naked_asm!(
        "mov x1, x30",     // x1 = ret-addr do caller (x30 ainda é o do caller real)
        "b   {body}",      // tail-call: o `b` não escreve x30
        body = sym alloc_body,
    )
}

/// Corpo Rust do replacement. ABI: `x0 = size` (arg original), `x1 = caller_ret`.
/// OBSERVE-ONLY: loga os N primeiros callers (rebaseados p/ vmaddr) e SEMPRE chama a
/// original via trampolim, devolvendo o ptr dela intacto. Nunca altera size, nunca pula
/// a alocação, nunca forja ptr → memória idêntica ao vanilla.
unsafe extern "C" fn alloc_body(size: u64, caller_ret: u64) -> *mut c_void {
    let n = ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
    if n < ALLOC_LOG_N {
        // runtime ret-addr → vmaddr estático (casa offline contra symbols-demangled.txt).
        // un_rebase devolve 0 se o ptr estiver fora do módulo principal (ex.: caller numa
        // dylib injetada como o ArchiveXL-loader) — útil já como sinal.
        let vmaddr = crate::un_rebase(caller_ret as *const c_void);
        crate::log(&format!(
            "[archivexl-diag] alloc caller #{n}: size={size} ret={caller_ret:#x} vmaddr={vmaddr:#x}"
        ));
    }
    // SEMPRE chama a original. Se (improvável) o trampolim for null, fallback seguro = null.
    let orig = ORIG_ALLOC.load(Ordering::Relaxed);
    if orig.is_null() {
        return std::ptr::null_mut();
    }
    let f: AllocFn = std::mem::transmute(orig);
    f(size)
}

// ===================== Util =====================

/// Compara os 4 primeiros u32 (16 bytes) em `addr` com `expected`. Pré-condição: já
/// validado legível pelo caller. Protege contra hookar um alvo que um patch moveu/mudou.
unsafe fn prologue_matches(addr: *mut c_void, expected: &[u32; 4]) -> bool {
    let mut buf = [0u8; 16];
    std::ptr::copy_nonoverlapping(addr as *const u8, buf.as_mut_ptr(), 16);
    for (i, &exp) in expected.iter().enumerate() {
        let got = u32::from_le_bytes([buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]]);
        if got != exp {
            return false;
        }
    }
    true
}
