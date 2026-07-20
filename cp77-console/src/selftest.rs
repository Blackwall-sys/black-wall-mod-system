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
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};

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

/// `red4ext-reloc-prove-ingame` (2026-07-13): `AlignedFree(void*, void*)` @ 0x100fa6f00 —
/// achada offline via scan dos símbolos definidos (`cp77-symbols/symbols-mangled.txt`) filtrando
/// pelo 1º instr = CBZ/CBNZ (script `find_cbz_tbz_bcond_prologues.py`, scratchpad). Função MÍNIMA
/// (4 instr, tail-call): `cbz x1,+0xc / ldur x0,[x1,#-8] / b <free-real> / ret`. O `cbz` no 1º
/// slot exercita a relocação de CBZ (gum.rs inverte pra `cbnz` saltando um abs-jump de 16B —
/// caminho só testado OFFLINE até agora). NUNCA é chamada pelo self-test (install→verifica→
/// revert síncrono, igual ao teste do ADRP acima).
const CBZ_TARGET_VM: u64 = 0x1_00fa_6f00;
/// Prólogo esperado (4 instr, LE u32): cbz / ldur / b / ret.
const CBZ_PROLOGUE: [u32; 4] = [0xb4000061, 0xf85f8020, 0x14ea5ca0, 0xd65f03c0];

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
        // axl-factories-apply: HookAfter LoadFactoryAsync (re-inject dos factories do mod = adicionar
        // itens). NO-OP até o offset Mac ser confirmado (RE em curso) — não instala hook torto.
        install_factory_hook();
        // ArchiveXL: HookBefore open-archive → loga o PATH de cada .archive carregado (prova
        // que os nossos carregam + mapeia a função do Path B).
        install_openarchive_diag();
        // resource.link: hook NAKED em 0x1021c5858 (ResourcePath->ref) — swap de path = link. Comandos
        // reslinkdump/reslink/reslinkstat provam in-game.
        install_reslink();
        let _ = install_sweep;
        let _ = install_depot_probes;
        let _ = install_reqres_probe;
        // Relocador (type 1) — já provado in-game.
        test_relocator();
        // `red4ext-reloc-prove-ingame`: mesma técnica, alvo real com prólogo CBZ (não ADRP).
        test_relocator_cbz();
        // `test_vtable_pool()` — TENTADO e REVERTIDO (2026-07-13): CRASHOU ao vivo (EXC_BAD_ACCESS
        // null-deref dentro de `PoolStorageProxy<PoolDefault>::AllocateAligned`). Causa: `run_dev_selftests`
        // roda MUITO cedo (dentro de `on_load`/`selfboot_if_needed`, ~ctor-time), ANTES do pool
        // `PoolDefault` do próprio motor estar inicializado — `rtti::pool_alloc` só é seguro
        // mais tarde (confirmado: todo forge de classe usa pool_alloc só de dentro de
        // `class_validate_probe_hook`, que roda durante o bind do script, bem depois). MESMA
        // categoria de armadilha de timing já documentada pra RTTI/GetOrRegisterType — reusar o
        // "mais cedo tecnicamente seguro" já mapeado (2ª chamada de GetOrRegisterType em diante),
        // não `on_load` direto. Função mantida (não chamada) pra retomada futura no hook certo.
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
/// 2º replacement-dummy — corpo distinto do 1º (evita code-folding pro MESMO endereço),
/// usado no 2º ciclo de install+revert (mesmo alvo) do teste `red4ext-attach-detach-contract`.
static RELOC_DUMMY2_TOUCHED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
extern "C" fn reloc_dummy_repl2() {
    RELOC_DUMMY2_TOUCHED.store(1, std::sync::atomic::Ordering::Relaxed);
}

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

    // `red4ext-attach-detach-contract` (2026-07-13): guarda os 16 bytes ORIGINAIS ANTES de
    // qualquer hook, pra comparar byte-a-byte depois do revert (não só "revert() foi chamado" —
    // prova que o conteúdo REALMENTE voltou idêntico ao que era).
    let mut original = [0u8; 16];
    std::ptr::copy_nonoverlapping(target as *const u8, original.as_mut_ptr(), 16);

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

            // Byte-exato: os 16 bytes do alvo DEPOIS do revert precisam bater 1:1 com os
            // capturados ANTES de qualquer hook — prova que o detach restaura o prólogo
            // original de verdade, não uma aproximação.
            let mut after = [0u8; 16];
            std::ptr::copy_nonoverlapping(target as *const u8, after.as_mut_ptr(), 16);
            let bytes_match = after == original;

            if is_movz {
                crate::log(&format!(
                    "[selftest] relocador OK em PoolStorageProxy<PoolRoot>::GetHandle (adrp relocado -> movz {insn:#010x})"
                ));
            } else {
                crate::log(&format!(
                    "[selftest] relocador FALHOU: tramp[0..4]={insn:#010x} não é movz (adrp não relocado?)"
                ));
            }
            crate::log(&format!(
                "[selftest] attach-detach: prólogo pós-revert {} do original ({} bytes) -> {}",
                if bytes_match { "BATE byte-a-byte" } else { "DIVERGE" },
                original.len(),
                if bytes_match { ">>> BYTE-EXATO OK <<<" } else { ">>> FALHOU <<<" }
            ));

            // `red4ext-attach-detach-contract` (hooks múltiplos por target, sequencial): re-hooka
            // o MESMO alvo com um replacement DIFERENTE, confirma que o 2º install também produz
            // um trampolim relocado corretamente, e reverte de novo — prova que o alvo não fica
            // "marcado"/corrompido por um ciclo anterior de install+revert.
            match it.replace(target, reloc_dummy_repl2 as *mut c_void) {
                Some(tramp2) => {
                    let mut first2 = [0u8; 4];
                    std::ptr::copy_nonoverlapping(tramp2 as *const u8, first2.as_mut_ptr(), 4);
                    let insn2 = u32::from_le_bytes(first2);
                    let is_movz2 = (insn2 & 0xFF80_0000) == 0xD280_0000;
                    it.revert(target);
                    let mut after2 = [0u8; 16];
                    std::ptr::copy_nonoverlapping(target as *const u8, after2.as_mut_ptr(), 16);
                    let bytes_match2 = after2 == original;
                    crate::log(&format!(
                        "[selftest] 2º ciclo (replacement diferente, mesmo alvo): relocado={is_movz2} byte-exato-pós-revert={bytes_match2} -> {}",
                        if is_movz2 && bytes_match2 { ">>> MÚLTIPLOS HOOKS POR TARGET OK <<<" } else { ">>> FALHOU <<<" }
                    ));
                }
                None => crate::log("[selftest] 2º ciclo FALHOU: Interceptor::replace (2ª vez) devolveu None"),
            }
        }
        None => {
            crate::log("[selftest] relocador FALHOU: Interceptor::replace devolveu None");
        }
    }
    std::mem::forget(it); // Interceptor é ZST; evita qualquer Drop implícito
}

/// `red4ext-reloc-prove-ingame` — MESMO padrão de `test_relocator`, mas no alvo CBZ
/// (`AlignedFree`, ver `CBZ_TARGET_VM`). Prova que o relocador lida com CBZ/CBNZ num alvo REAL
/// (não só o teste sintético offline `gum::tests`) — a inversão pra `cbnz` + abs-jump de 16B
/// (gum.rs:293-303) precisa materializar corretamente no trampolim.
unsafe fn test_relocator_cbz() {
    let target = crate::rebase(CBZ_TARGET_VM);
    if !gum::is_readable(target as *const c_void, 16) {
        crate::log("[selftest] relocador(CBZ): alvo ilegível -> pulado (sem hook)");
        return;
    }
    if !prologue_matches(target, &CBZ_PROLOGUE) {
        crate::log("[selftest] relocador(CBZ): prólogo não casou (patch?) -> abortado (sem hook)");
        return;
    }
    let mut original = [0u8; 16];
    std::ptr::copy_nonoverlapping(target as *const u8, original.as_mut_ptr(), 16);

    let it = Interceptor::obtain();
    match it.replace(target, reloc_dummy_repl as *mut c_void) {
        Some(tramp) => {
            // O 1º slot do alvo é um `cbz` → o relocador deve materializar um `cbnz` invertido
            // saltando por cima de um abs-jump (gum.rs:293-303). Confirma tramp[0..4] = cbnz
            // (mesma família CBZ/CBNZ, bit24 setado): máscara 0x7E000000 == 0x34000000, bit24=1.
            let mut first = [0u8; 4];
            std::ptr::copy_nonoverlapping(tramp as *const u8, first.as_mut_ptr(), 4);
            let insn = u32::from_le_bytes(first);
            let is_cbnz_family = (insn & 0x7E000000) == 0x34000000 && (insn & 0x01000000) != 0;

            it.revert(target);

            let mut after = [0u8; 16];
            std::ptr::copy_nonoverlapping(target as *const u8, after.as_mut_ptr(), 16);
            let bytes_match = after == original;

            crate::log(&format!(
                "[selftest] relocador(CBZ) em AlignedFree: 1ª instr do tramp={insn:#010x} cbnz-invertido={is_cbnz_family} | pós-revert byte-exato={bytes_match} -> {}",
                if is_cbnz_family && bytes_match { ">>> RELOC-EM-CBZ-REAL OK <<<" } else { ">>> FALHOU <<<" }
            ));
        }
        None => crate::log("[selftest] relocador(CBZ): Interceptor::replace devolveu None"),
    }
    std::mem::forget(it);
}

// ===================== 2) vtable (type 2) =====================
// Teste-vivo ANTIGO removido: protegia memória do STACK (mach_vm_protect numa array
// local) — inválido, travava o self-test. `test_vtable_pool` (2026-07-13) resolve isso
// usando `rtti::pool_alloc` (HEAP do pool do próprio jogo, mesma alocação que os forges de
// classe usam a sessão inteira) — protection-flip funciona igual em heap, sem o problema
// de stack. NÃO mexe em nenhuma vtable REAL do jogo (buffer 100% nosso, nunca consultado
// por nenhum sistema do motor) — zero risco a sistemas vivos, mas exercita a MESMA função
// `vtable_hook`/`vtable_unhook` que a `BwmsApi` expõe pra plugins.

/// `red4ext-api-prove-hooks-extplugin` (fatia vtable_hook, 2026-07-13): aloca um buffer de 2
/// slots no POOL do jogo (`rtti::pool_alloc`, mesma via que os forges de classe usam),
/// escreve um ponteiro conhecido no slot 0, chama `gum::vtable_hook` (a MESMA função por trás
/// de `BwmsApi.vtable_hook`) pra trocar por outro ponteiro, confirma a troca, e
/// `vtable_unhook` pra restaurar — tudo num buffer que NENHUM sistema do motor consulta.
unsafe fn test_vtable_pool() {
    let buf = crate::rtti::pool_alloc(16, 8) as *mut u64;
    if buf.is_null() {
        crate::log("[selftest] vtable(pool): pool_alloc devolveu null -> pulado");
        return;
    }
    let original_fn = reloc_dummy_repl as *const c_void;
    let replacement_fn = reloc_dummy_repl2 as *const c_void;
    buf.write(original_fn as u64);
    buf.add(1).write(0xDEAD_BEEF_0000_0000u64); // slot vizinho, só pra confirmar que não vaza

    let hooked = gum::vtable_hook(buf, 0, replacement_fn);
    let slot0_after_hook = buf.read() as *const c_void;
    let hook_ok = hooked == Some(original_fn) && slot0_after_hook == replacement_fn;

    gum::vtable_unhook(buf, 0, original_fn);
    let slot0_after_unhook = buf.read() as *const c_void;
    let unhook_ok = slot0_after_unhook == original_fn;
    let neighbor_intact = buf.add(1).read() == 0xDEAD_BEEF_0000_0000u64;

    crate::log(&format!(
        "[selftest] vtable(pool): hook devolveu original certo={hook_ok} | pós-unhook restaurou certo={unhook_ok} | slot vizinho intacto={neighbor_intact} -> {}",
        if hook_ok && unhook_ok && neighbor_intact { ">>> VTABLE_HOOK/UNHOOK OK <<<" } else { ">>> FALHOU <<<" }
    ));
}

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

// ===== axl-factories-apply — HookAfter FactoryIndex::LoadFactoryAsync (ADICIONAR ITENS, uso #1) =====
// Mecanismo (cp2077-archive-xl/src/App/Extensions/FactoryIndex/Extension.cpp): HookAfter
// LoadFactoryAsync(aIndex x0, ResourcePath aPath x1 /*u64*/, aContext x2). Quando aPath == o ÚLTIMO
// factory vanilla (SENTINEL "base\gameplay\factories\vehicles\vehicles.csv"), re-chama LoadFactoryAsync
// p/ CADA factory do mod → injeta os itens do mod DEPOIS do vanilla terminar. O aPath é um ResourcePath
// = FNV-1a64 do path normalizado = o nosso resource_path_hash (mesma fn do resource.link, agora em
// bwms-hashes). **Offset Mac de LoadFactoryAsync = [UNCONFIRMED]** (RE em curso, workflow
// bwms-6mods-attack): o install só liga quando FACTORY_LOADFACTORY_VM for válido (!=0 e legível).
// vmaddr Mac de FactoryIndex::LoadFactoryAsync (link base 0x100000000) — achado por RE semântica
// ARM64 2026-07-16 (workflow de disassembly, ALTA confiança; ver notes/native-addrs-found-2026-07-16.md).
// Assinatura `void(uintptr_t aIndex /*x0*/, ResourcePath aPath /*x1,u64*/, uintptr_t aContext /*x2*/)`.
// Único caller = 0x100cc11c8 (iterador 0x100cc10dc), 1 chamada por factory csv. VERIFY: os goldens
// no factory_replacement (vehicles.csv=0xf94faab4ff97393a sentinel, object_pool_budgets=0x433d78092c642133).
const FACTORY_LOADFACTORY_VM: u64 = 0x1_00cc_0710;
/// Goldens de verificação (bwms_hashes::resource_path_hash dos paths .csv de factory conhecidos).
const FACTORY_GOLDEN_SENTINEL: u64 = 0xf94f_aab4_ff97_393a; // base\gameplay\factories\vehicles\vehicles.csv (ÚLTIMA)
const FACTORY_GOLDEN_OBJPOOL: u64 = 0x433d_7809_2c64_2133; // base\gameplay\factories\items\object_pool_budgets.csv
/// Contador de chamadas observadas de LoadFactoryAsync (limita log a N).
static FACTORY_CALLS: AtomicU64 = AtomicU64::new(0);
const FACTORY_SENTINEL: &str = "base\\gameplay\\factories\\vehicles\\vehicles.csv";
static ORIG_LOADFACTORY: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// Último `aIndex` (FactoryIndex*) visto pelo hook — exposto pro comando de canal `factdump` (dump
/// read-only, pós-async, pra achar o offset REAL do count do registry). Ver `dump_factory_index`.
pub(crate) static FACTORY_LAST_INDEX: AtomicUsize = AtomicUsize::new(0);
/// aIndex ESPECÍFICO capturado no momento do sentinel/re-inject (não sobrescrito por outras chamadas).
pub(crate) static FACTORY_REINJECT_INDEX: AtomicUsize = AtomicUsize::new(0);
/// Hashes (ResourcePath) dos .csv de factory dos mods — o mod-manager popula do `.xl` (secção
/// `factories`), o runtime re-injeta no sentinel. Análogo ao RESLINK_MAP do resource.link.
static FACTORY_PATHS: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

/// Adiciona UM .csv de factory (por path) à lista de re-inject. Dedup por hash.
pub(crate) fn factory_add(path: &str) {
    let h = resource_path_hash(path);
    if let Ok(mut v) = FACTORY_PATHS.lock() {
        if !v.contains(&h) {
            v.push(h);
        }
        crate::log(&format!("[factory] +'{path}' (#{h:#018x}); {} factories na fila", v.len()));
    }
}
/// Carrega N paths de factory de um arquivo (1 por linha; `#`=comentário) — o que o mod-manager gera
/// do `.xl`. Como o reslink_file.
pub(crate) fn factory_file(path: &str) {
    match std::fs::read_to_string(path) {
        Ok(c) => {
            let mut n = 0;
            for line in c.lines() {
                let l = line.trim();
                if !l.is_empty() && !l.starts_with('#') {
                    factory_add(l);
                    n += 1;
                }
            }
            crate::log(&format!("[factory] {n} factories carregados de '{path}'"));
        }
        Err(e) => crate::log(&format!("[factory] não leu '{path}': {e}")),
    }
}

/// HookAfter LoadFactoryAsync: roda o vanilla; se o path é o sentinel (último factory), re-injeta os
/// factories do mod com a MESMA fn nativa (aIndex/aContext preservados).
unsafe extern "C" fn factory_replacement(index: usize, path: u64, context: usize) {
    let orig = ORIG_LOADFACTORY.load(Ordering::Relaxed);
    if orig.is_null() {
        return;
    }
    // VERIFICAÇÃO do endereço (observe-only, achado por RE 2026-07-16): loga os N primeiros callers.
    // Se x1 (path) casar os goldens (object_pool_budgets no meio da rajada + vehicles.csv como a
    // ÚLTIMA/sentinel) e index (aIndex) for constante, o endereço 0x100cc0710 está CONFIRMADO.
    {
        FACTORY_LAST_INDEX.store(index, Ordering::Relaxed);
        let n = FACTORY_CALLS.fetch_add(1, Ordering::Relaxed);
        let sentinel = path == FACTORY_GOLDEN_SENTINEL;
        if n < 32 || sentinel {
            let tag = if sentinel {
                " <== SENTINEL (vehicles.csv, última)"
            } else if path == FACTORY_GOLDEN_OBJPOOL {
                " <== object_pool_budgets.csv (golden)"
            } else {
                ""
            };
            crate::log(&format!(
                "[factory-diag] LoadFactoryAsync #{n} aIndex={index:#x} aPath={path:#018x} aContext={context:#x}{tag}"
            ));
        }
    }
    let f: unsafe extern "C" fn(usize, u64, usize) = std::mem::transmute(orig);
    f(index, path, context); // HookAfter: vanilla primeiro
    if path == resource_path_hash(FACTORY_SENTINEL) {
        // CAPTURA o aIndex ESPECÍFICO deste sentinel (achado 2026-07-17: FACTORY_LAST_INDEX é
        // sobrescrito por OUTRAS chamadas de LoadFactoryAsync com aIndex DIFERENTE — várias factory
        // tables coexistem/streamam durante gameplay — então "o último visto" não é confiável pra
        // rastrear ESTE re-inject especificamente ao longo do tempo). `factdump` usa este ponteiro fixo.
        FACTORY_REINJECT_INDEX.store(index, Ordering::Relaxed);
        // PROVA (log, sem visual): o FactoryIndex (aIndex) tem um registry cujo COUNT cresce quando um
        // factory novo carrega. Snapshot dos u32 de [aIndex+0 .. +0x80] ANTES e DEPOIS da re-injeção;
        // qualquer offset que CRESCEU = o count do registry → prova que o meu factory ENTROU no índice.
        let idx = index as *const u8;
        let readable = gum::is_readable(idx as *const c_void, 0x80);
        let mut before = [0u32; 32];
        if readable {
            for (i, b) in before.iter_mut().enumerate() {
                *b = (idx.add(i * 4) as *const u32).read_unaligned();
            }
        }
        if let Ok(v) = FACTORY_PATHS.lock() {
            for &h in v.iter() {
                f(index, h, context); // re-injeta o factory do mod DEPOIS do sentinel
                crate::log(&format!("[factory] re-injetado #{h:#018x} (após o sentinel)"));
            }
        }
        if readable {
            let mut cresceu = String::new();
            for i in 0..32usize {
                let after = (idx.add(i * 4) as *const u32).read_unaligned();
                if after > before[i] && after.wrapping_sub(before[i]) < 100_000 {
                    cresceu.push_str(&format!(" [+{:#x}]{}→{}", i * 4, before[i], after));
                }
            }
            if cresceu.is_empty() {
                crate::log("[factory] aIndex: NENHUM u32 [0..0x80] cresceu após a re-inject (o factory não entrou no registry, ou o count é noutro offset/estrutura)");
            } else {
                crate::log(&format!(
                    ">>> FACTORY-APPLY OK: o registry do FactoryIndex CRESCEU após injetar o factory custom (entrada ativa):{cresceu} <<<"
                ));
            }
        }
    }
}

/// Instala o HookAfter em LoadFactoryAsync. NO-OP enquanto FACTORY_LOADFACTORY_VM==0 (offset pendente
/// de RE) — não instala hook torto. Quando o offset for confirmado, liga sozinho.
pub(crate) unsafe fn install_factory_hook() {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if FACTORY_LOADFACTORY_VM == 0 {
        return; // offset ainda não confirmado (RE em curso)
    }
    let target = crate::rebase(FACTORY_LOADFACTORY_VM);
    if !gum::is_readable(target as *const c_void, 16) || INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }
    let it = Interceptor::obtain();
    match it.replace(target, factory_replacement as *mut c_void) {
        Some(tramp) => {
            ORIG_LOADFACTORY.store(tramp, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log("[factory] HookAfter LoadFactoryAsync instalado (re-inject dos factories do mod)");
        }
        None => {
            INSTALLED.store(false, Ordering::Relaxed);
            crate::log("[factory] FALHA ao hookar LoadFactoryAsync");
        }
    }
}

/// `axl-factories-apply` (achado 2026-07-17): LoadFactoryAsync é ASSÍNCRONO — a re-inject só ENFILEIRA
/// o load, o registry do FactoryIndex cresce DEPOIS. Snapshot imediato (0 boots atrás) não achou nada
/// em [aIndex+0..0x80]. Este dump é READ-ONLY e ROBUSTO: varre uma janela BEM mais ampla
/// [0x00..0x400) procurando por padrões `{ptr_heap, u32 cap, u32 count}` (DynArray genérico do engine,
/// mesmo layout achado no depot do pathb) — candidatos plausíveis (ptr em heap real, cap>=count,
/// count pequeno) são logados pra eu comparar antes/depois manualmente (2 chamadas, com espera entre).
pub(crate) unsafe fn dump_factory_index() {
    // Prefere o aIndex FIXADO no momento do sentinel/re-inject (estável) — FACTORY_LAST_INDEX é
    // sobrescrito por outras chamadas concorrentes de LoadFactoryAsync (várias tables coexistem).
    let idx = match FACTORY_REINJECT_INDEX.load(Ordering::Relaxed) {
        0 => FACTORY_LAST_INDEX.load(Ordering::Relaxed),
        v => v,
    };
    if idx == 0 {
        crate::log("[factdump] nenhum aIndex capturado ainda (hook não rodou?)");
        return;
    }
    let base = idx as *const u8;
    const WIN: usize = 0x400;
    if !gum::is_readable(base as *const c_void, WIN) {
        crate::log(&format!("[factdump] aIndex={idx:#x} ilegível na janela {WIN:#x}"));
        return;
    }
    let mut out = format!("[factdump] aIndex={idx:#x} candidatos DynArray-like em [0..{WIN:#x}):");
    let mut n = 0;
    let mut off = 0usize;
    while off + 16 <= WIN {
        let ptr = (base.add(off) as *const u64).read_unaligned();
        let cap = (base.add(off + 8) as *const u32).read_unaligned();
        let cnt = (base.add(off + 12) as *const u32).read_unaligned();
        // heap real (não null, não a faixa de imagem estática 0x1_0000_0000..0x1_1000_0000)
        let ptr_heap = ptr != 0 && !(0x1_0000_0000..0x1_1000_0000).contains(&ptr);
        if ptr_heap && cnt > 0 && cnt <= cap && cap < 100_000 && gum::is_readable(ptr as *const c_void, 8) {
            out.push_str(&format!(" [+{off:#x}]{{ptr={ptr:#x} cap={cap} count={cnt}}}"));
            n += 1;
            if n >= 24 {
                break;
            }
        }
        off += 4; // granularidade de 4B (structs não necessariamente 8-alinhadas aqui)
    }
    if n == 0 {
        out.push_str(" (nenhum candidato — talvez a estrutura seja HashMap/outro layout, ou o count ainda não cresceu)");
    }
    crate::log(&out);
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

// ===== resource.link/copy: live-confirm de RequestResource (observe-only) =====
// 0x103eda898 = cand a ResourceDepot::RequestResource(x0=depot, x1=outHandle, x2=path, x3=archiveHandle).
// Disasm: prólogo limpo `sub sp,#0x70`, 4-arg, chama o resolve-helper 0x103eda360, lê campos do depot.
// (0x103eda894 era o ramo `bl __stack_chk_fail` da função ANTERIOR — entrada real é +4.)
// HookBefore: loga (x0, x2) dos 1os calls + confirma x0==singleton do depot, chama a orig. Só observa.
const REQRES_VM: u64 = 0x1_03ed_a898;
static ORIG_REQRES: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static REQRES_N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
type ReqResFn = unsafe extern "C" fn(*mut c_void, *mut c_void, u64, *mut c_void) -> *mut c_void;

unsafe extern "C" fn reqres_replacement(
    x0: *mut c_void,
    x1: *mut c_void,
    x2: u64,
    x3: *mut c_void,
) -> *mut c_void {
    let n = REQRES_N.fetch_add(1, Ordering::Relaxed);
    if n < 12 {
        // confirma x0 == singleton do depot ([0x109003000+0x1f8] deref)
        let depot_pp = (crate::rebase(0x1_0900_3000) as *const u8).add(0x1f8) as *const *const u8;
        let singleton = if gum::is_readable(depot_pp as *const c_void, 8) {
            depot_pp.read()
        } else {
            std::ptr::null()
        };
        let is_depot = x0 as *const u8 == singleton;
        crate::log(&format!(
            "[reqres-probe] #{n} x0={x0:p} (==depot? {is_depot}) path={x2:#018x} outH={x1:p} arch={x3:p}"
        ));
    }
    let orig = ORIG_REQRES.load(Ordering::Relaxed);
    if orig.is_null() {
        return std::ptr::null_mut();
    }
    let f: ReqResFn = std::mem::transmute(orig);
    f(x0, x1, x2, x3)
}

unsafe fn install_reqres_probe() {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    let target = crate::rebase(REQRES_VM);
    if !gum::is_readable(target as *const c_void, 16) {
        crate::log("[reqres-probe] alvo RequestResource ilegível -> sem hook");
        return;
    }
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }
    let it = Interceptor::obtain();
    match it.replace(target, reqres_replacement as *mut c_void) {
        Some(tramp) => {
            ORIG_REQRES.store(tramp, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log("[reqres-probe] hook instalado em RequestResource cand 0x103eda894 (observe-only)");
        }
        None => {
            INSTALLED.store(false, Ordering::Relaxed);
            crate::log("[reqres-probe] FALHA ao hookar RequestResource (replace None)");
        }
    }
}

// ===== Batch-probe da seção do depot: identifica RequestResource/CheckResource por ARGS AO VIVO =====
// Static signature-matching falhou (regs viram scratch). Aqui hookamos candidatos observe-only e
// deixamos os args reais decidirem: quem é chamado com x0==depot + um arg com cara de hash de
// ResourcePath (alta entropia, não-ponteiro) é o alvo.
unsafe fn dprobe_depot_ptr() -> u64 {
    let pp = (crate::rebase(0x1_0900_3000) as *const u8).add(0x1f8) as *const u64;
    if gum::is_readable(pp as *const c_void, 8) {
        pp.read()
    } else {
        0
    }
}
/// Heurística de hash de ResourcePath: >32 bits, não-zero no topo, e NÃO num range de ponteiro
/// do macOS (code 0x10–0x16, heap/stack 0x60–0x7f).
fn dprobe_pathlike(v: u64) -> bool {
    let hi = v >> 40;
    v > 0xffff_ffff && hi != 0 && !(0x10..=0x16).contains(&hi) && !(0x60..=0x7f).contains(&hi)
}
/// Loga (slot, args) quando o método VIRTUAL é chamado com x0==depot. RequestResource sai com
/// path-hash em x2; CheckResource com path em x1.
unsafe fn dprobe_log_vt(slot: usize, x1: u64, x2: u64, x3: u64) {
    crate::log(&format!(
        "[vprobe vt+{:#04x}] DEPOT x1={x1:#018x}{} x2={x2:#018x}{} x3={x3:#x}",
        slot * 8,
        if dprobe_pathlike(x1) { " <PATH" } else { "" },
        if dprobe_pathlike(x2) { " <PATH" } else { "" },
    ));
}

/// Vtable do `res::ResourceGameDepot` (static, do depotdump). `vtable_hook` troca o PONTEIRO do slot
/// (COW em __DATA_CONST) — NÃO patcha __TEXT → sem overlap, sem relocação, sem o crash do inline-hook.
const DEPOT_VTABLE_VM: u64 = 0x1_06f5_01c0;

macro_rules! vt_probe {
    ($name:ident, $slot:literal) => {
        mod $name {
            use crate::gum;
            use std::ffi::c_void;
            use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
            pub static ORIG: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
            static N: AtomicU64 = AtomicU64::new(0);
            pub const SLOT: usize = $slot;
            pub unsafe extern "C" fn repl(x0: u64, x1: u64, x2: u64, x3: u64, x4: u64, x5: u64) -> u64 {
                let d = super::dprobe_depot_ptr();
                if x0 == d && d != 0 && N.fetch_add(1, Ordering::Relaxed) < 5 {
                    super::dprobe_log_vt($slot, x1, x2, x3);
                }
                let o = ORIG.load(Ordering::Relaxed);
                if o.is_null() {
                    return 0; // slot pulado (gap) nunca instalado → nunca chega aqui
                }
                let f: unsafe extern "C" fn(u64, u64, u64, u64, u64, u64) -> u64 =
                    std::mem::transmute(o);
                f(x0, x1, x2, x3, x4, x5)
            }
            pub fn set_orig(o: *const c_void) {
                ORIG.store(o as *mut c_void, Ordering::Relaxed);
            }
        }
    };
}

vt_probe!(vp00, 0); vt_probe!(vp01, 1); vt_probe!(vp02, 2); vt_probe!(vp03, 3);
vt_probe!(vp04, 4); vt_probe!(vp05, 5); vt_probe!(vp06, 6); vt_probe!(vp07, 7);
vt_probe!(vp08, 8); vt_probe!(vp09, 9); vt_probe!(vp10, 10); vt_probe!(vp11, 11);
vt_probe!(vp12, 12); vt_probe!(vp13, 13); vt_probe!(vp14, 14); vt_probe!(vp15, 15);
vt_probe!(vp16, 16); vt_probe!(vp17, 17); vt_probe!(vp18, 18); vt_probe!(vp19, 19);
vt_probe!(vp20, 20); vt_probe!(vp21, 21); vt_probe!(vp22, 22); vt_probe!(vp23, 23);
vt_probe!(vp24, 24); vt_probe!(vp25, 25); vt_probe!(vp26, 26); vt_probe!(vp27, 27);
vt_probe!(vp28, 28); vt_probe!(vp29, 29); vt_probe!(vp30, 30); vt_probe!(vp31, 31);

// vt+0x50 (slot 10) = "itera archives por índice" do depot. O RequestResource, ao buscar um path,
// ITERA os archives → CHAMA vt+0x50. Hookamos esse slot com um SHIM NAKED que preserva x0-x8+x30 (ABI
// intacta — conserta o crash do passthrough tipado): grava o x30 (caller) num ring buffer e faz `br`
// (tail-call) ao original. Em GAMEPLAY os callers distintos = RequestResource (não-virtual, sem símbolo).
#[repr(C)]
struct Vt50Ring {
    idx: AtomicU64,
    buf: [AtomicU64; 256],
}
pub(crate) static VT50_RING: Vt50Ring = Vt50Ring {
    idx: AtomicU64::new(0),
    buf: [const { AtomicU64::new(0) }; 256],
};
static ORIG_VT50N: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// NAKED: usa só x9-x13 (scratch, livres na entrada). NÃO toca x0-x8 nem x30. Grava o caller (x30) em
// VT50_RING.buf[idx&255] e dá `br` no original (que retorna a x30 = caller real, intacto).
#[unsafe(naked)]
unsafe extern "C" fn vt50_naked() {
    core::arch::naked_asm!(
        "adrp x9, {ring}@PAGE",
        "add  x9, x9, {ring}@PAGEOFF",   // x9 = &VT50_RING (idx@0, buf@8)
        "ldr  x10, [x9]",                 // idx
        "add  x11, x10, #1",
        "str  x11, [x9]",                 // idx++ (relaxed, ok p/ diag)
        "and  x10, x10, #0xff",
        "add  x12, x9, #8",               // &buf[0]
        "str  x30, [x12, x10, lsl #3]",   // buf[idx&255] = caller
        "adrp x13, {orig}@PAGE",
        "add  x13, x13, {orig}@PAGEOFF",
        "ldr  x13, [x13]",                // x13 = original
        "br   x13",                       // tail-call (x30 intacto → original retorna ao caller)
        ring = sym VT50_RING,
        orig = sym ORIG_VT50N,
    )
}

unsafe fn install_depot_probes() {
    let vtbl = crate::rebase(DEPOT_VTABLE_VM) as *mut u64;
    if !gum::is_readable(vtbl as *const c_void, 8 * 16) {
        crate::log("[vt50] vtable do depot ilegível -> sem probe");
        return;
    }
    match gum::vtable_hook(vtbl, 10, vt50_naked as *const c_void) {
        Some(orig) => {
            ORIG_VT50N.store(orig as *mut c_void, Ordering::Relaxed);
            crate::log("[vt50] hook NAKED em vt+0x50 instalado — drene com 'vt50drain' em gameplay");
        }
        None => crate::log("[vt50] vtable_hook falhou"),
    }
}

/// Drena o ring de callers do vt+0x50 (comando 'vt50drain' em gameplay). Loga os endereços distintos
/// (un-rebased) por frequência = candidatos ao RequestResource.
pub(crate) fn drain_vt50_ring() {
    let total = VT50_RING.idx.load(Ordering::Relaxed);
    let mut seen: std::collections::BTreeMap<u64, u32> = std::collections::BTreeMap::new();
    for slot in VT50_RING.buf.iter() {
        let v = slot.load(Ordering::Relaxed);
        if v != 0 {
            *seen.entry(v).or_insert(0) += 1;
        }
    }
    crate::log(&format!("[vt50drain] {total} chamadas, {} callers distintos:", seen.len()));
    let mut v: Vec<(u64, u32)> = seen.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (caller, cnt) in v.into_iter().take(24) {
        let st = unsafe { crate::un_rebase(caller as *const c_void) };
        crate::log(&format!("[vt50drain]   x30={st:#x} (static)  x{cnt}"));
    }
}

// ===== SWEEP NAKED: grava (idx,x0,x2) sobre candidatos não-virtuais da seção do depot, em GAMEPLAY =====
// Cada shim naked (preserva ABI) grava no ring e tail-calla seu trampolim. O candidato chamado com
// x0==depot + x2==hash de path = RequestResource(depot, out, path, arch). SEGURO + gameplay (auto-continue).
// Slot POR-CANDIDATO (sem ring/flooding): [count, last_x0, last_x2, pad] × 24.
static SWEEP_STATE: [AtomicU64; 96] = [const { AtomicU64::new(0) }; 96];

const SWEEP_ADDRS: [u64; 24] = [
    // LOADER batch 2 (0x1021c). 0x1021c26fc = anchor (path-hasher, 246×). LoadAsync = x2=request, path em [x2].
    0x1_021c_26fc, 0x1_021c_4358, 0x1_021c_44ec, 0x1_021c_4730,
    0x1_021c_4828, 0x1_021c_488c, 0x1_021c_4950, 0x1_021c_4a14,
    0x1_021c_4c0c, 0x1_021c_4d4c, 0x1_021c_51b8, 0x1_021c_52bc,
    0x1_021c_53d4, 0x1_021c_5658, 0x1_021c_56fc, 0x1_021c_5858,
    0x1_021c_5a1c, 0x1_021c_5b64, 0x1_021c_5c8c, 0x1_021c_5e9c,
    0x1_021c_6020, 0x1_021c_6138, 0x1_021c_63a4, 0x1_021c_6438,
];

macro_rules! sweep_shim {
    ($m:ident, $idx:literal) => {
        mod $m {
            use std::ffi::c_void;
            use std::sync::atomic::{AtomicPtr, Ordering};
            pub static ORIG: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
            #[unsafe(naked)]
            pub unsafe extern "C" fn shim() {
                core::arch::naked_asm!(
                    "adrp x9, {st}@PAGE",
                    "add  x9, x9, {st}@PAGEOFF",
                    "add  x9, x9, #{off}",   // &SWEEP_STATE[idx*4] (off = idx*32 bytes)
                    "ldr  x10, [x9]",
                    "add  x10, x10, #1",
                    "str  x10, [x9]",        // count++
                    "str  x0, [x9, #8]",     // last x0
                    "str  x2, [x9, #16]",    // last x2
                    "str  x1, [x9, #24]",    // last x1 (path-hasher usa [x1])
                    "adrp x13, {orig}@PAGE",
                    "add  x13, x13, {orig}@PAGEOFF",
                    "ldr  x13, [x13]",
                    "br   x13",
                    st = sym super::SWEEP_STATE,
                    orig = sym ORIG,
                    off = const ($idx * 32),
                )
            }
            pub unsafe fn install(addr: u64) {
                let t = crate::rebase(addr);
                if !crate::gum::is_readable(t as *const c_void, 16) {
                    return;
                }
                let it = crate::gum::Interceptor::obtain();
                if let Some(tr) = it.replace(t, shim as *mut c_void) {
                    ORIG.store(tr, Ordering::Relaxed);
                    std::mem::forget(it);
                }
            }
        }
    };
}

sweep_shim!(sw00, 0); sweep_shim!(sw01, 1); sweep_shim!(sw02, 2); sweep_shim!(sw03, 3);
sweep_shim!(sw04, 4); sweep_shim!(sw05, 5); sweep_shim!(sw06, 6); sweep_shim!(sw07, 7);
sweep_shim!(sw08, 8); sweep_shim!(sw09, 9); sweep_shim!(sw10, 10); sweep_shim!(sw11, 11);
sweep_shim!(sw12, 12); sweep_shim!(sw13, 13); sweep_shim!(sw14, 14); sweep_shim!(sw15, 15);
sweep_shim!(sw16, 16); sweep_shim!(sw17, 17); sweep_shim!(sw18, 18); sweep_shim!(sw19, 19);
sweep_shim!(sw20, 20); sweep_shim!(sw21, 21); sweep_shim!(sw22, 22); sweep_shim!(sw23, 23);

unsafe fn install_sweep() {
    crate::log("[sweep] 24 shims NAKED na seção do depot (record x0/x2; drene 'sweepdrain' em gameplay)...");
    sw00::install(SWEEP_ADDRS[0]); sw01::install(SWEEP_ADDRS[1]);
    sw02::install(SWEEP_ADDRS[2]); sw03::install(SWEEP_ADDRS[3]);
    sw04::install(SWEEP_ADDRS[4]); sw05::install(SWEEP_ADDRS[5]);
    sw06::install(SWEEP_ADDRS[6]); sw07::install(SWEEP_ADDRS[7]);
    sw08::install(SWEEP_ADDRS[8]); sw09::install(SWEEP_ADDRS[9]);
    sw10::install(SWEEP_ADDRS[10]); sw11::install(SWEEP_ADDRS[11]);
    sw12::install(SWEEP_ADDRS[12]); sw13::install(SWEEP_ADDRS[13]);
    sw14::install(SWEEP_ADDRS[14]); sw15::install(SWEEP_ADDRS[15]);
    sw16::install(SWEEP_ADDRS[16]); sw17::install(SWEEP_ADDRS[17]);
    sw18::install(SWEEP_ADDRS[18]); sw19::install(SWEEP_ADDRS[19]);
    sw20::install(SWEEP_ADDRS[20]); sw21::install(SWEEP_ADDRS[21]);
    sw22::install(SWEEP_ADDRS[22]); sw23::install(SWEEP_ADDRS[23]);
    crate::log("[sweep] instalado");
}

/// Drena os slots por-candidato, ordenado por frequência. Pra LoadAsync: x2=request → deref [x2]=path.
pub(crate) fn drain_sweep() {
    let depot = unsafe { dprobe_depot_ptr() };
    crate::log(&format!("[sweepdrain] depot={depot:#x} (ordenado por freq; LoadAsync=alta+[x2]=path):"));
    let mut rows: Vec<(u64, u64, u64, u64)> = Vec::new(); // (cnt, addr, x0, x2)
    for n in 0..24usize {
        let cnt = SWEEP_STATE[n * 4].load(Ordering::Relaxed);
        if cnt == 0 {
            continue;
        }
        rows.push((
            cnt,
            SWEEP_ADDRS[n],
            SWEEP_STATE[n * 4 + 1].load(Ordering::Relaxed),
            SWEEP_STATE[n * 4 + 2].load(Ordering::Relaxed),
        ));
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    for (cnt, addr, x0, x2) in rows {
        let n = SWEEP_ADDRS.iter().position(|&a| a == addr).unwrap_or(0);
        let x1 = SWEEP_STATE[n * 4 + 3].load(Ordering::Relaxed);
        // procura um path (FNV) em: x2 valor, [x2] (request.path), x1 valor, [x1] (path-hasher).
        let chk = |v: u64| -> Option<u64> {
            if dprobe_pathlike(v) {
                return Some(v);
            }
            if v > 0x1000 {
                let p = v as *const u64;
                if unsafe { gum::is_readable(p as *const c_void, 8) } {
                    let d = unsafe { p.read() };
                    if dprobe_pathlike(d) {
                        return Some(d);
                    }
                }
            }
            None
        };
        let mut tail = String::new();
        if let Some(p) = chk(x2) {
            tail = format!(" PATH(x2)={p:#x}");
        } else if let Some(p) = chk(x1) {
            tail = format!(" PATH(x1)={p:#x}");
        }
        crate::log(&format!(
            "[sweepdrain]   {addr:#x} x{cnt} x0={x0:#x}{}{}",
            if depot != 0 && x0 == depot { " ==DEPOT" } else { "" },
            tail,
        ));
    }
}

// ===== RESOURCE.LINK: hook de swap em 0x1021c5858 (ResourcePath->ResourceReference constructor) =====
// Shim NAKED: grava o path (x0) num ring (dump) e, se x0==swapsrc, troca x0=swaptgt (= resource.link).
// Preserva x8 (indirect-result do constructor) — só usa x9-x14 (scratch). Tail-call ao original.
#[repr(C)]
struct ResLink {
    idx: AtomicU64,        // @0  contador de construções (ring)
    count: AtomicU64,      // @8  # de pares no mapa (gate do asm: 0 = pula a chamada Rust)
    swapcnt: AtomicU64,    // @16 swaps disparados (hits)
    _pad: AtomicU64,       // @24
    ring: [AtomicU64; 64], // @32 últimos paths construídos (pro dump)
}
static RESLINK: ResLink = ResLink {
    idx: AtomicU64::new(0),
    count: AtomicU64::new(0),
    swapcnt: AtomicU64::new(0),
    _pad: AtomicU64::new(0),
    ring: [const { AtomicU64::new(0) }; 64],
};
/// Tabela de redirects (source_hash -> target_hash). Const-init (Vec::new/Mutex::new são const).
static RESLINK_MAP: std::sync::Mutex<Vec<(u64, u64)>> = std::sync::Mutex::new(Vec::new());
static ORIG_RESLINK: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
const RESLINK_VM: u64 = 0x1_021c_5858;

/// `cw-controller-misc` (2026-07-19): hash observado por `watchres <path>` + flag "visto" — o
/// hook `reslink_lookup` (já disparando em TODA construção real de `ResourcePath`, mecanismo
/// zero-crash provado dezenas de vezes) marca `WATCH_RES_SEEN` quando o hash observado bate. O
/// `cp77_tick` (thread do jogo, edge-triggered, MESMO padrão seguro de `Player/Spawned`) drena a
/// flag e dispara `Resource/Load` (`ResourceEvent` real) com o hash — sem hookar
/// `ResourceSerializer::SchedulePostLoadJobs` (RE nova, fora de escopo desta sessão). O disparo em
/// si é 100% real: só acontece quando o jogo de fato constrói aquele `ResourcePath` específico.
pub(crate) static WATCH_RES_HASH: AtomicU64 = AtomicU64::new(0);
pub(crate) static WATCH_RES_SEEN: AtomicBool = AtomicBool::new(false);

/// Arma o watch: registra um self-map (src==tgt, no-op de swap, só força o gate asm `count>0`) +
/// grava o hash-alvo. Reseta `WATCH_RES_SEEN` (novo braço). Loga o hash esperado (cross-check
/// contra o `[re] ResourceEvent.GetPath()` que vai disparar, quando disparar).
pub(crate) fn reslink_watch(path: &str) {
    let h = resource_path_hash(path);
    WATCH_RES_HASH.store(h, Ordering::Relaxed);
    WATCH_RES_SEEN.store(false, Ordering::Relaxed);
    crate::log(&format!("[reslink] watchres armado: '{path}' (hash={h:#018x}) — aguardando construção real do ResourcePath"));
    reslink_add(h, h); // self-map: abre o gate do hook, zero efeito de swap
}

/// Chamado pelo shim SÓ quando count>0 (gate no asm). Varre a tabela; se achar o path, devolve o
/// alvo (swap = resource.link) e conta o hit; senão devolve o path intacto.
unsafe extern "C" fn reslink_lookup(path: u64) -> u64 {
    if let Ok(m) = RESLINK_MAP.lock() {
        for &(s, t) in m.iter() {
            if s == path {
                RESLINK.swapcnt.fetch_add(1, Ordering::Relaxed);
                // DIAG (2026-07-16): loga quando redireciona o .ent do PLAYER (não o animgraph) —
                // confirma se o template do player passa pelo nosso hook no spawn. Player .ent hashes:
                if s == 0x1ab9_05fe_c596_cb22
                    || s == 0x1bcd_09f6_f70a_7818
                    || s == 0x58b1_6007_a8cd_0f7c
                    || s == 0x5ea3_4357_4414_752e
                {
                    crate::log(&format!("[reslink] >>> PLAYER .ent redirecionado: {s:#x} -> {t:#x}"));
                }
                if s == WATCH_RES_HASH.load(Ordering::Relaxed) && s != 0 {
                    WATCH_RES_SEEN.store(true, Ordering::Relaxed);
                }
                return t;
            }
        }
    }
    path
}

#[unsafe(naked)]
unsafe extern "C" fn reslink_shim() {
    core::arch::naked_asm!(
        "adrp x9, {r}@PAGE",
        "add  x9, x9, {r}@PAGEOFF",   // x9 = &RESLINK
        "ldr  x10, [x9]",
        "add  x11, x10, #1",
        "str  x11, [x9]",             // idx++
        "and  x10, x10, #0x3f",
        "add  x12, x9, #32",
        "str  x0, [x12, x10, lsl #3]", // ring[idx&63] = x0 (path)
        "ldr  x13, [x9, #8]",          // count (# de pares)
        "cbz  x13, 2f",                // sem links -> tail-call direto (caminho comum, lock-free)
        "stp  x1, x2, [sp, #-0x60]!",  // salva args + indirect-result (x8) do constructor
        "stp  x3, x4, [sp, #0x10]",
        "stp  x5, x6, [sp, #0x20]",
        "stp  x7, x8, [sp, #0x30]",
        "str  x30, [sp, #0x40]",
        "bl   {lookup}",               // x0 = reslink_lookup(x0)  (swap se hit = resource.link!)
        "ldr  x30, [sp, #0x40]",
        "ldp  x7, x8, [sp, #0x30]",
        "ldp  x5, x6, [sp, #0x20]",
        "ldp  x3, x4, [sp, #0x10]",
        "ldp  x1, x2, [sp], #0x60",
        "2:",
        "adrp x13, {o}@PAGE",
        "add  x13, x13, {o}@PAGEOFF",
        "ldr  x13, [x13]",
        "br   x13",                    // tail-call original (x8/x1..x7/x30 intactos)
        r = sym RESLINK,
        o = sym ORIG_RESLINK,
        lookup = sym reslink_lookup,
    )
}

pub(crate) unsafe fn install_reslink() {
    if !ORIG_RESLINK.load(Ordering::Relaxed).is_null() {
        return; // já instalado (idempotente: dev-selftest + auto-load de mod)
    }
    let t = crate::rebase(RESLINK_VM);
    if !gum::is_readable(t as *const c_void, 16) {
        crate::log("[reslink] alvo 0x1021c5858 ilegível");
        return;
    }
    let it = Interceptor::obtain();
    match it.replace(t, reslink_shim as *mut c_void) {
        Some(tr) => {
            ORIG_RESLINK.store(tr, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log("[reslink] hook NAKED em 0x1021c5858 (ResourcePath->ref) — reslinkdump / reslink <src> <tgt> / reslinkstat");
        }
        None => crate::log("[reslink] replace None"),
    }
}

pub(crate) fn reslink_dump() {
    let total = RESLINK.idx.load(Ordering::Relaxed);
    let mut seen: std::collections::BTreeMap<u64, u32> = std::collections::BTreeMap::new();
    for s in RESLINK.ring.iter() {
        let v = s.load(Ordering::Relaxed);
        if dprobe_pathlike(v) {
            *seen.entry(v).or_insert(0) += 1;
        }
    }
    crate::log(&format!("[reslinkdump] {total} construções, {} paths distintos (use um como <src>):", seen.len()));
    let mut v: Vec<(u64, u32)> = seen.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (p, c) in v.into_iter().take(64) {
        crate::log(&format!("[reslinkdump]   path={p:#018x} x{c}"));
    }
}
/// FNV-1a64 do ResourcePath (lowercase + '/'->'\\'). FONTE ÚNICA no crate `bwms-hashes` (a MESMA
/// impl provada 5/5 goldens em bwms-core/apply_xl.rs; antes era cópia byte-a-byte aqui).
pub(crate) use bwms_hashes::resource_path_hash;
/// Adiciona UM par (source_hash -> target_hash) à tabela. Dedup por source.
pub(crate) fn reslink_add(src: u64, tgt: u64) {
    if let Ok(mut m) = RESLINK_MAP.lock() {
        m.retain(|&(s, _)| s != src);
        m.push((src, tgt));
        RESLINK.count.store(m.len() as u64, Ordering::Relaxed);
        crate::log(&format!("[reslink] +par {src:#018x} -> {tgt:#018x} (tabela: {} pares)", m.len()));
    }
}
/// Adiciona um par a partir dos PATHS (strings) — hasheia com FNV-1a64.
pub(crate) fn reslink_path(src: &str, tgt: &str) {
    let (s, t) = (resource_path_hash(src), resource_path_hash(tgt));
    crate::log(&format!("[reslink] path '{src}' (#{s:#018x}) -> '{tgt}'"));
    reslink_add(s, t);
}
/// Carrega N pares de um arquivo (linhas `srcpath|tgtpath`, `#`=comentário). É o que um mod
/// ArchiveXL real usa: o mod-manager gera esse arquivo do `.xl` (resource.link/copy) e o runtime
/// popula a tabela. Caminho default = `<red4ext>/bwms-reslink.txt`.
pub(crate) fn reslink_file(path: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return crate::log(&format!("[reslink] não leu '{path}': {e}")),
    };
    let mut n = 0;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((s, t)) = line.split_once('|') {
            reslink_add(resource_path_hash(s.trim()), resource_path_hash(t.trim()));
            n += 1;
        }
    }
    crate::log(&format!("[reslink] {n} pares carregados de '{path}'"));
}
/// Compat: limpa a tabela e registra UM par (hashes).
pub(crate) fn reslink_set(src: u64, tgt: u64) {
    if let Ok(mut m) = RESLINK_MAP.lock() {
        m.clear();
    }
    RESLINK.swapcnt.store(0, Ordering::Relaxed);
    reslink_add(src, tgt);
}
pub(crate) fn reslink_stat() {
    let n = RESLINK.swapcnt.load(Ordering::Relaxed);
    let pairs = RESLINK_MAP.lock().map(|m| m.len()).unwrap_or(0);
    crate::log(&format!(
        "[reslink] {pairs} pares na tabela; swaps em path REAL: {n}  {}",
        if n > 0 { ">>> RESOURCE.LINK FUNCIONANDO <<<" } else { "(ainda 0 — src não reconstruído)" }
    ));
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

#[cfg(test)]
mod reslink_tests {
    use super::resource_path_hash;
    #[test]
    fn fnv_golden_bate_com_apply_xl() {
        // golden provado em bwms-core/src/apply_xl.rs::tests::hash_goldens
        assert_eq!(
            resource_path_hash("base\\resource.cooked_mlsetup"),
            0x3a12_b4fd_1938_d5ca
        );
        // normalização: '/' vira '\\' e case não importa → mesmo hash
        assert_eq!(
            resource_path_hash("BASE/resource.cooked_mlsetup"),
            0x3a12_b4fd_1938_d5ca
        );
    }
}
