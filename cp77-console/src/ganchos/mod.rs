//! Ganchos: desviar código do jogo — hook inline arm64 próprio (trampolim com relocação do prólogo), troca de slot de vtable e
//! de entrada de tabela, escrita de instrução com as threads paradas. Nenhuma lib de terceiros. A leitura de memória que não crasha
//! em ponteiro inválido está em `memoria`.

#![allow(dead_code)]

use crate::memoria::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use crate::macos::{mach_task_self_, mach_vm_allocate, mach_vm_protect, KernReturn, MachPort, KERN_SUCCESS, VM_PROT_COPY, VM_PROT_EXECUTE, VM_PROT_READ, VM_PROT_WRITE};

/// O original publicado com as threads paradas, antes do salto, pelo `destino` de [`instalar_com_threads_paradas`], [`instalar_perto`] ou [`instalar_com_ponte`].
pub(crate) unsafe fn original_publicado(orig: &AtomicU64) -> u64 {
    loop {
        let o = orig.load(Ordering::Relaxed);
        if o != 0 {
            return o;
        }
        std::thread::yield_now();
    }
}

/// [`instalar_conferindo_prologo_em`] no endereço do vmaddr `vm`.
pub(crate) unsafe fn instalar_conferindo_prologo(vm: u64, prologo: u32, destino: *mut c_void, orig: &AtomicU64) -> Option<bool> {
    instalar_conferindo_prologo_em(crate::rebase(vm) as *mut c_void, prologo, destino, orig)
}

/// Hook em `alvo` só com o prólogo esperado, com o original publicado em `orig` com as threads paradas, antes do salto. `Some(true)`: posto agora.
/// `None`: `destino` já está no alvo (de antes, ou posto por outra thread entre a leitura do prólogo e a trava de `HOOKS`), ou seja, no ar.
/// `Some(false)`: `destino` não está no alvo: prólogo diferente (outro hook no alvo troca o 1º u32 pelo salto dele), alvo ilegível ou escrita recusada.
pub(crate) unsafe fn instalar_conferindo_prologo_em(alvo: *mut c_void, prologo: u32, destino: *mut c_void, orig: &AtomicU64) -> Option<bool> {
    let lido = if is_readable(alvo as *const c_void, 16) { (alvo as *const u32).read_unaligned() } else { 0 };
    if lido == prologo && instalar_com_threads_paradas(alvo, destino, Some(orig)).is_some() {
        return Some(true);
    }
    (!substituto_no_ar(alvo, destino)).then_some(false)
}

/// Corpo do [`install_hook_probe!`]: hook em `alvo` com o original publicado em `orig` com as threads paradas, antes do salto
/// ([`instalar_com_threads_paradas`]). `None` quando não há nada a fazer: `orig` já publicado (o mesmo hook por outro sítio que divide o `orig`),
/// alvo ilegível, ou a mesma sonda já no alvo (posta por outra thread entre a leitura do `orig` e a trava de `HOOKS`).
pub(crate) unsafe fn instalar_sonda(alvo: *mut c_void, sonda: *mut c_void, orig: &AtomicU64) -> Option<bool> {
    if orig.load(Ordering::Relaxed) != 0 || !is_readable(alvo as *const c_void, 16) {
        return None;
    }
    if instalar_com_threads_paradas(alvo, sonda, Some(orig)).is_some() {
        return Some(true);
    }
    (!substituto_no_ar(alvo, sonda)).then_some(false)
}

/// Instala 1 hook de sonda num vmaddr ([`instalar_sonda`]) e loga o resultado: `$ok`/`$fail` são `&str` ou `&format!(..)`, e não há log quando não
/// há nada a fazer. `$orig` é o `AtomicU64` que a sonda lê com [`original_publicado`]; sítios que instalam o mesmo hook dividem o mesmo `$orig`.
macro_rules! install_hook_probe {
    ($vm:expr, $probe:expr, $orig:expr, $ok:expr, $fail:expr) => {
        match crate::ganchos::instalar_sonda(crate::rebase($vm), $probe as *mut std::ffi::c_void, &$orig) {
            Some(true) => crate::log($ok),
            Some(false) => crate::log($fail),
            None => {}
        }
    };
}
pub(crate) use install_hook_probe;


const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const PROT_EXEC: i32 = 4;
const MAP_PRIVATE: i32 = 0x0002;
const MAP_FIXED: i32 = 0x0010;
const MAP_ANON: i32 = 0x1000;
const MAP_JIT: i32 = 0x0800;
const MAP_FAILED: isize = -1;

extern "C" {
    fn sys_icache_invalidate(start: *mut c_void, len: usize);
    fn pthread_jit_write_protect_np(enabled: i32);
    fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> i32;
    fn mach_vm_region(target_task: MachPort, address: *mut u64, size: *mut u64, flavor: i32, info: *mut u32, info_cnt: *mut u32, object_name: *mut MachPort) -> KernReturn;
    fn mach_vm_remap(
        target_task: MachPort, target_address: *mut u64, size: u64, mask: u64, flags: i32,
        src_task: MachPort, src_address: u64, copy: i32,
        cur_prot: *mut i32, max_prot: *mut i32, inheritance: i32,
    ) -> KernReturn;
    fn __error() -> *mut i32;
    fn mach_thread_self() -> MachPort;
    fn task_threads(task: MachPort, lista: *mut *mut MachPort, quantas: *mut u32) -> KernReturn;
    fn thread_suspend(thread: MachPort) -> KernReturn;
    fn thread_resume(thread: MachPort) -> KernReturn;
    fn thread_get_state(thread: MachPort, flavor: i32, estado: *mut u32, palavras: *mut u32) -> KernReturn;
    fn mach_port_deallocate(task: MachPort, nome: MachPort) -> KernReturn;
    fn mach_vm_deallocate(task: MachPort, endereco: u64, tamanho: u64) -> KernReturn;
}
const VM_FLAGS_FIXED: i32 = 0x0;
const VM_REGION_BASIC_INFO_64: i32 = 9;
/// `ARM_THREAD_STATE64` e o tamanho dele em palavras de 32 bits (x0–x28, fp, lr, sp, pc, cpsr e flags): o `pc` ocupa as palavras 64 e 65.
const ARM_THREAD_STATE64: i32 = 6;
const ARM_THREAD_STATE64_PALAVRAS: u32 = 68;





// salto absoluto de 64 bits (16 bytes): ldr x17,#8 ; br x17 ; .quad alvo
// x17 (IP1) é scratch — clobber seguro na entrada da função.
fn abs_jump(target: u64) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&0x5800_0051u32.to_le_bytes()); // ldr x17, #8
    b[4..8].copy_from_slice(&0xD61F_0220u32.to_le_bytes()); // br  x17
    b[8..16].copy_from_slice(&target.to_le_bytes());
    b
}

// Materializa um imm64 absoluto em Xrd via movz+movk (4 instr, sem PC-relativo, sem pool).
fn emit_load_imm64(out: &mut Vec<u8>, rd: u32, v: u64) {
    let rd = rd & 0x1F;
    let movz = 0xD280_0000u32 | (((v & 0xFFFF) as u32) << 5) | rd; // movz Xrd, #v[15:0]
    out.extend_from_slice(&movz.to_le_bytes());
    let mut hw = 1u32;
    while hw < 4 {
        let half = ((v >> (hw * 16)) & 0xFFFF) as u32;
        let movk = 0xF280_0000u32 | (hw << 21) | (half << 5) | rd; // movk Xrd, #half, lsl 16*hw
        out.extend_from_slice(&movk.to_le_bytes());
        hw += 1;
    }
}

/// `B <to>` de 4 bytes se `to` cai em ±128MB de `from` e está alinhado; senão `None`.
/// (B: opcode 0x14000000 | imm26, com `imm26 = (off>>2)` em complemento-2; alcance ±2^27 bytes.)
/// É o que permite hookar função PEQUENA sem o abs-jump de 16B que transborda a vizinha.
fn emit_b_near(from: u64, to: u64) -> Option<[u8; 4]> {
    let off = to as i64 - from as i64;
    if (off & 0x3) != 0 || off < -(1 << 27) || off >= (1 << 27) {
        return None;
    }
    let imm26 = ((off >> 2) as u32) & 0x03FF_FFFF;
    Some((0x1400_0000u32 | imm26).to_le_bytes())
}

/// `ADRP Xrd, <to>` (calcula o endereço da PÁGINA de `to`, relativo a PC&~0xFFF, alcance ±4GB —
/// 32x maior que o `B` de 4B) seguido de `BR Xrd` — 8 bytes, exatos pro getter de 8B (2 leaf
/// instructions). Exige `to` alinhado a 4096 (ADRP só endereça páginas inteiras). Achado
/// 2026-07-12 (GOG): o `B` de ±128MB não alcança nosso dylib nesse layout, mas QUALQUER
/// `mmap` comum (sem MAP_FIXED, sempre bem-sucedido) cai muito dentro de ±4GB — não precisa
/// mais achar um endereço específico livre, só usar o que o mmap normal já devolve.
fn emit_adrp_br(from: u64, to_page_aligned: u64, reg: u32) -> Option<[u8; 8]> {
    if to_page_aligned & 0xFFF != 0 {
        return None; // ADRP só aponta pra pagina (multiplo de 4096)
    }
    let from_page = (from as i64) & !0xFFF;
    let delta_pages = ((to_page_aligned as i64) - from_page) >> 12;
    if delta_pages < -(1 << 20) || delta_pages >= (1 << 20) {
        return None; // fora de +-4GB (imm21 com sinal, em unidades de 4096)
    }
    let imm21 = (delta_pages as u32) & 0x1F_FFFF;
    let immlo = imm21 & 0x3;
    let immhi = (imm21 >> 2) & 0x7_FFFF;
    let rd = reg & 0x1F;
    let adrp = 0x9000_0000u32 | (immlo << 29) | (immhi << 5) | rd;
    let br = 0xD61F_0000u32 | (rd << 5);
    let mut out = [0u8; 8];
    out[0..4].copy_from_slice(&adrp.to_le_bytes());
    out[4..8].copy_from_slice(&br.to_le_bytes());
    Some(out)
}

/// Acha um endereço LIVRE (não mapeado) de pelo menos `min_size` dentro de [lo, hi), andando
/// pelo mapa de memória real do processo via `mach_vm_region` (nunca adivinha — `MAP_FIXED` numa
/// suposição errada pode SUBSTITUIR silenciosamente uma região já em uso no macOS, corrompendo
/// outra coisa; então SEMPRE confirma o gap antes de mapear ali).
unsafe fn find_free_gap(lo: u64, hi: u64, min_size: u64) -> Option<u64> {
    let page = 16 * 1024u64;
    let mut cursor = lo & !(page - 1);
    while cursor < hi {
        let mut addr = cursor;
        let mut size: u64 = 0;
        let mut info = [0u32; 32];
        let mut info_cnt: u32 = 32;
        let mut obj: MachPort = 0;
        let kr = mach_vm_region(mach_task_self_, &mut addr, &mut size, VM_REGION_BASIC_INFO_64, info.as_mut_ptr(), &mut info_cnt, &mut obj);
        if kr != KERN_SUCCESS {
            // Sem mais regiões mapeadas dali pra frente -> o resto até `hi` está livre.
            return if hi - cursor >= min_size { Some(cursor) } else { None };
        }
        // `addr` agora é o início da PRÓXIMA região mapeada em ou após `cursor`.
        if addr > cursor && (addr - cursor) >= min_size {
            return Some(cursor); // gap [cursor, addr) confirmado livre
        }
        cursor = addr.max(cursor + page) + size.max(page);
    }
    None
}

/// Aloca um "landing pad" JIT DENTRO de ±128MB de `near_to` (achado por `find_free_gap`, nunca
/// adivinhado) contendo um `abs_jump(dest)` (16B, alcance irrestrito). Usado quando `emit_b_near`
/// recusa (alvo longe demais do nosso dylib — visto no GOG: layout de memória diferente do Steam
/// empurra o hook pra fora de ±128MB). O `B` de 4B no alvo aponta pro pad (perto, sempre
/// alcançável); o pad salta pro destino real (longe, sem limite). 2 saltos em vez de 1, custo
/// desprezível (a função só roda ~1x/frame no getter da phase byte).
unsafe fn alloc_near_landing_pad(near_to: u64, dest: u64) -> Option<u64> {
    const PAGE: u64 = 16 * 1024;
    const MARGIN: u64 = 120 * 1024 * 1024; // usa quase todo o orçamento de ±128MB do B
    const FLOOR: u64 = 0x1_0100_0000; // um pouco acima de 4GB — abaixo disso o kernel recusa MAP_FIXED
    // Busca SÓ PRA CIMA de `near_to` primeiro: `near_to` já mora acima da marca de 4GB (nosso
    // esquema de vmaddr é bwms_primitives::enderecos::LINK_BASE=0x1_0000_0000 + offset), mas `near_to - MARGIN` frequentemente
    // cai ABAIXO de 4GB — achado 2026-07-12: essa faixa <4GB é reportada como "livre" pelo
    // `mach_vm_region` mas o kernel RECUSA `mmap(..., MAP_FIXED)` lá mesmo assim (região reservada,
    // não é só "sem mapeamento de processo"). Só cai pra busca pra baixo se a de cima falhar.
    let free = find_free_gap(near_to, near_to.saturating_add(MARGIN), PAGE)
        .or_else(|| find_free_gap(near_to.saturating_sub(MARGIN).max(FLOOR), near_to, PAGE))?;
    if emit_b_near(near_to, free).is_none() {
        return None; // gap achado mas fora do alcance do B (não devia acontecer dentro de MARGIN)
    }
    // TENTATIVA 1: mach_vm_allocate FIXED direto no endereço livre confirmado.
    let mut addr_io = free;
    let kr_alloc = mach_vm_allocate(mach_task_self_, &mut addr_io, PAGE, VM_FLAGS_FIXED);
    let p = if kr_alloc == KERN_SUCCESS && addr_io == free {
        Some(free as *mut c_void)
    } else {
        // TENTATIVA 2: mmap comum FIXED (RW, sem MAP_JIT).
        let p2 = mmap(free as *mut c_void, PAGE as usize, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON | MAP_FIXED, -1, 0);
        if !p2.is_null() && p2 as isize != MAP_FAILED && p2 as u64 == free { Some(p2) } else { None }
    };
    let p = match p {
        Some(p) => p,
        None => {
            // TENTATIVA 3 (achado 2026-07-12: 1 e 2 batem em ENOMEM — o kernel recusa alocação NOVA
            // perto do Mach-O do jogo, mesmo em endereço confirmado livre). `mach_vm_remap` segue um
            // caminho de kernel DIFERENTE (mapeia de novo um objeto de memória JÁ EXISTENTE em vez de
            // criar um novo do zero) — pode não ter a mesma restrição de posicionamento.
            let src = mmap(std::ptr::null_mut(), PAGE as usize, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
            if src.is_null() || src as isize == MAP_FAILED {
                return None;
            }
            let stub = abs_jump(dest);
            std::ptr::copy_nonoverlapping(stub.as_ptr(), src as *mut u8, stub.len());
            if mach_vm_protect(mach_task_self_, src as u64, PAGE, 0, VM_PROT_READ | VM_PROT_EXECUTE) != KERN_SUCCESS {
                let _ = munmap(src, PAGE as usize);
                return None;
            }
            sys_icache_invalidate(src, stub.len());
            let mut target = free;
            let mut cur_prot: i32 = 0;
            let mut max_prot: i32 = 0;
            let kr_remap = mach_vm_remap(
                mach_task_self_, &mut target, PAGE, 0, VM_FLAGS_FIXED,
                mach_task_self_, src as u64, 0, // copy=false: mapeia o MESMO objeto (compartilhado)
                &mut cur_prot, &mut max_prot, 1, // VM_INHERIT_COPY
            );
            crate::log_dev!("[ganchos] alloc_near_landing_pad: mach_vm_remap(src={src:p} -> free={free:#x}) -> kr={kr_remap} target={target:#x}");
            if kr_remap != KERN_SUCCESS || target != free {
                return None;
            }
            return Some(free); // já escrito+executável (é o mesmo objeto do `src`, já preparado acima)
        }
    };
    let stub = abs_jump(dest);
    std::ptr::copy_nonoverlapping(stub.as_ptr(), p as *mut u8, stub.len());
    let kr = mach_vm_protect(mach_task_self_, free, PAGE, 0, VM_PROT_READ | VM_PROT_EXECUTE);
    if kr != KERN_SUCCESS {
        let _ = munmap(p, PAGE as usize);
        return None;
    }
    sys_icache_invalidate(p, stub.len());
    Some(free)
}

// Relocador de prólogo arm64: copia os 16 bytes (4 instr) deslocados consertando os
// PC-relativos. ADR/ADRP -> materializa o resultado ABSOLUTO em Xrd (movz/movk), então o
// trampolim pode rodar em qualquer endereço. B/BL/B.cond/CBZ/CBNZ/TBZ/TBNZ/LDR-literal GP/LDRSW/
// PRFM são relocados (ver `relocate_n`); só o LDR-literal SIMD/FP devolve None = recusa o hook
// (seguro, sem corromper). Não-PC-relativas (stp/sub/add/mov/ret/br/...) -> verbatim. O executor
// (prólogo stp/add/sub) cai no verbatim, então o hook existente continua idêntico.
unsafe fn relocate_prologue(orig: u64) -> Option<Vec<u8>> {
    relocate_n(orig, 4)
}

/// Reloca `n_insns` instruções do prólogo. n=4 p/ o abs-jump de 16B (`instalar`); n=1 p/ o
/// `instalar_perto`, que rouba só a 1ª instrução (cabe num `B` de 4B). Mesma lógica de conserto
/// de PC-relativo (ADR/ADRP/B/B.cond/CBZ/TBZ/BL/LDR-lit GP/LDRSW/PRFM consertados; literal SIMD/FP
/// = recusa).
unsafe fn relocate_n(orig: u64, n_insns: u64) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(96);
    let mut i = 0u64;
    while i < n_insns {
        let ia = orig + i * 4;
        let mut raw = [0u8; 4];
        std::ptr::copy_nonoverlapping(ia as *const u8, raw.as_mut_ptr(), 4);
        let insn = u32::from_le_bytes(raw);

        // ADR / ADRP: bits[28:24] == 0b10000
        if (insn & 0x1F00_0000) == 0x1000_0000 {
            let is_adrp = (insn >> 31) & 1 == 1;
            let rd = insn & 0x1F;
            let immlo = (insn >> 29) & 0x3;
            let immhi = (insn >> 5) & 0x7_FFFF;
            let mut imm = ((immhi << 2) | immlo) as i64;
            if imm & (1 << 20) != 0 {
                imm |= !0x1F_FFFFi64; // sign-extend 21-bit
            }
            let result = if is_adrp {
                ((ia & !0xFFF) as i64).wrapping_add(imm << 12) as u64
            } else {
                (ia as i64).wrapping_add(imm) as u64
            };
            emit_load_imm64(&mut out, rd, result);
            i += 1;
            continue;
        }

        // B (salto incondicional / tail-call) -> salto absoluto pro alvo.
        if (insn & 0xFC00_0000) == 0x1400_0000 {
            let mut off = (insn & 0x03FF_FFFF) as i64;
            if off & (1 << 25) != 0 { off |= !0x03FF_FFFFi64; }
            let target = (ia as i64).wrapping_add(off << 2) as u64;
            out.extend_from_slice(&abs_jump(target));
            i += 1;
            continue;
        }
        // B.cond -> inverte a condição saltando por cima de um abs-jump pro alvo.
        if (insn & 0xFF00_0010) == 0x5400_0000 {
            let cond = insn & 0xF;
            let mut off = ((insn >> 5) & 0x7_FFFF) as i64;
            if off & (1 << 18) != 0 { off |= !0x7_FFFFi64; }
            let target = (ia as i64).wrapping_add(off << 2) as u64;
            let inv = 0x5400_0000u32 | (5u32 << 5) | (cond ^ 1); // b.<inv> #20 (pula o abs-jump de 16B)
            out.extend_from_slice(&inv.to_le_bytes());
            out.extend_from_slice(&abs_jump(target));
            i += 1;
            continue;
        }
        // CBZ/CBNZ -> inverte o op (bit24) + pula por cima do abs-jump.
        if (insn & 0x7E00_0000) == 0x3400_0000 {
            let mut off = ((insn >> 5) & 0x7_FFFF) as i64;
            if off & (1 << 18) != 0 { off |= !0x7_FFFFi64; }
            let target = (ia as i64).wrapping_add(off << 2) as u64;
            let inv = ((insn ^ 0x0100_0000) & !(0x7_FFFFu32 << 5)) | (5u32 << 5);
            out.extend_from_slice(&inv.to_le_bytes());
            out.extend_from_slice(&abs_jump(target));
            i += 1;
            continue;
        }
        // TBZ/TBNZ -> inverte o op (bit24) + pula (imm14).
        if (insn & 0x7E00_0000) == 0x3600_0000 {
            let mut off = ((insn >> 5) & 0x3FFF) as i64;
            if off & (1 << 13) != 0 { off |= !0x3FFFi64; }
            let target = (ia as i64).wrapping_add(off << 2) as u64;
            let inv = ((insn ^ 0x0100_0000) & !(0x3FFFu32 << 5)) | (5u32 << 5);
            out.extend_from_slice(&inv.to_le_bytes());
            out.extend_from_slice(&abs_jump(target));
            i += 1;
            continue;
        }
        // LDR literal 64-bit GP (0x58) -> LÊ o valor UMA VEZ (agora, na relocação) e materializa o
        // VALOR direto em Xt — NÃO materializa o endereço pra reler em runtime. Achado 2026-07-16
        // (ao vivo, empilhando 2 hooks no mesmo alvo pra `red4ext-attach-detach-contract`): o
        // padrão antigo (materializa endereço + `ldr [Xscratch]` em runtime) quebra quando o
        // "prólogo" sendo relocado É UM abs_jump patch de OUTRO hook — o próprio abs_jump usa ESTE
        // exato encoding (`ldr x17,#8`), e reler o endereço em runtime pega o que estiver lá NAQUELE
        // instante, que pode já ter sido SOBRESCRITO por um patch mais novo no MESMO endereço
        // (loop infinito confirmado: tramp2 relia o endereço, um hook3 sobrescrevia com seu próprio
        // alvo, tramp2 saltava pra si mesmo). Ler o valor agora (snapshot) é imune a isso — e é
        // igualmente correto pro caso normal (literal de constante compilada, imutável).
        if (insn & 0xFF00_0000) == 0x5800_0000 {
            let rt = insn & 0x1F;
            let mut off = ((insn >> 5) & 0x7_FFFF) as i64;
            if off & (1 << 18) != 0 { off |= !0x7_FFFFi64; }
            let lit_addr = (ia as i64).wrapping_add(off << 2) as u64;
            let value = std::ptr::read_unaligned(lit_addr as *const u64);
            emit_load_imm64(&mut out, rt, value);
            i += 1;
            continue;
        }
        // BL (chamada) -> materializa o alvo absoluto em X17 (IP1, scratch de convenção — safe pra
        // clobber antes de uma call, mesma regra dos veneers/PLT) + BLR X17. LR fica correto sozinho:
        // BLR seta LR = PC+4 da PRÓPRIA posição no trampolim, que é exatamente onde a relocação
        // continua (resto das instruções + jump-back pro código original) — sem precisar de contas.
        if (insn & 0xFC00_0000) == 0x9400_0000 {
            let mut off = (insn & 0x03FF_FFFF) as i64;
            if off & (1 << 25) != 0 { off |= !0x03FF_FFFFi64; }
            let target = (ia as i64).wrapping_add(off << 2) as u64;
            emit_load_imm64(&mut out, 17, target);
            let blr = 0xD63F_0000u32 | (17u32 << 5); // blr x17
            out.extend_from_slice(&blr.to_le_bytes());
            i += 1;
            continue;
        }
        // LDR literal (32-bit GP, opc=00,V=0, 0x18) -> lê o valor UMA VEZ (u32, zero-estende pra
        // Xt) e materializa direto — mesmo motivo do caso 64-bit acima (snapshot, imune a
        // sobrescrita por um hook mais novo no mesmo endereço).
        if (insn & 0xFF00_0000) == 0x1800_0000 {
            let rt = insn & 0x1F;
            let mut off = ((insn >> 5) & 0x7_FFFF) as i64;
            if off & (1 << 18) != 0 { off |= !0x7_FFFFi64; }
            let lit_addr = (ia as i64).wrapping_add(off << 2) as u64;
            let value = std::ptr::read_unaligned(lit_addr as *const u32) as u64; // ldr Wt zero-estende
            emit_load_imm64(&mut out, rt, value);
            i += 1;
            continue;
        }
        // LDRSW literal (opc=10,V=0, 0x98) -> lê o valor UMA VEZ (i32, SINAL-estende pra Xt) e
        // materializa direto — mesmo motivo dos casos acima (snapshot, imune a sobrescrita).
        if (insn & 0xFF00_0000) == 0x9800_0000 {
            let rt = insn & 0x1F;
            let mut off = ((insn >> 5) & 0x7_FFFF) as i64;
            if off & (1 << 18) != 0 { off |= !0x7_FFFFi64; }
            let lit_addr = (ia as i64).wrapping_add(off << 2) as u64;
            let raw32 = std::ptr::read_unaligned(lit_addr as *const i32);
            let value = raw32 as i64 as u64; // ldrsw sinal-estende 32->64
            emit_load_imm64(&mut out, rt, value);
            i += 1;
            continue;
        }
        // PRFM literal (opc=11,V=0, 0xD8): hint puro de cache, sem efeito observável -> vira NOP.
        // (Prefetch nunca muda o resultado do programa; descartar é seguro e mais simples que
        // materializar o endereço só pra manter um hint que o núcleo pode ignorar de qualquer jeito.)
        if (insn & 0xFF00_0000) == 0xD800_0000 {
            out.extend_from_slice(&0xD503_201Fu32.to_le_bytes()); // nop
            i += 1;
            continue;
        }
        // Literais SIMD/FP (LDR St/Dt/Qt, opc=00/01/10 com V=1: 0x1C/0x5C/0x9C) — registrador de
        // destino é o banco SIMD/FP, não GP; ainda não tratado -> recusa segura (raro em prólogo real).
        let is_other_lit = (insn & 0x3B00_0000) == 0x1800_0000;
        if is_other_lit {
            return None;
        }

        // Não-PC-relativo: verbatim.
        out.extend_from_slice(&raw);
        i += 1;
    }
    Some(out)
}

struct Hook {
    target: u64,
    orig: [u8; 16],
    /// Trampolim devolvido por esta instalação; 0 no patch sem trampolim ([`trocar_instrucao`]). Identifica o hook no `inline_detach` de plugin
    /// ([`remover_parado`] com `so_topo`): o `inline_hook_owned` instala por [`instalar_com_trampolim_novo`], com página nova que nunca é desmapeada
    /// nem entra em [`TRAMPOLINS`], então nenhuma outra instalação devolve o mesmo valor.
    tramp: u64,
    /// Para onde o salto desvia; 0 no patch sem trampolim ([`trocar_instrucao`]). Com `target`, é o que [`escrever_salto`] recusa repetir.
    repl: u64,
}
static HOOKS: Mutex<Vec<Hook>> = Mutex::new(Vec::new());

// ---- bookkeeping do contrato attach/detach (puro, unit-testável sem tocar memória) ----
// Vários hooks podem empilhar no MESMO alvo: cada `Hook.orig` guarda os 16 bytes que estavam lá
// QUANDO ELE aplicou (o `instalar` copia o prólogo atual antes de patchar). Logo o desfazer correto
// é LIFO: reverter o ÚLTIMO hook do alvo restaura o estado anterior a ele (que reativa o penúltimo),
// e assim por diante até o 1º hook restaurar o original verdadeiro. O bug antigo usava o PRIMEIRO
// (`position`), restaurando o original verdadeiro cedo demais e corrompendo os hooks empilhados.

/// Índice do hook mais RECENTE instalado em `target` (LIFO). `None` se não há hook nesse alvo.
fn latest_index_for(hooks: &[Hook], target: u64) -> Option<usize> {
    hooks.iter().rposition(|h| h.target == target)
}

/// Os 16 bytes ORIGINAIS verdadeiros de `target` (o `orig` do PRIMEIRO hook instalado nele) —
/// antes de qualquer patch. `None` se não há hook nesse alvo.
fn true_original_of(hooks: &[Hook], target: u64) -> Option<[u8; 16]> {
    hooks.iter().find(|h| h.target == target).map(|h| h.orig)
}

/// Quantos hooks estão empilhados em `target`.
fn count_for(hooks: &[Hook], target: u64) -> usize {
    hooks.iter().filter(|h| h.target == target).count()
}

/// `repl` já está empilhado em `target`? Um 2º salto pro mesmo substituto vira laço: o trampolim dele, relocado do 1º salto, volta pro substituto.
fn substituto_no_alvo(hooks: &[Hook], target: u64, repl: u64) -> bool {
    hooks.iter().any(|h| h.target == target && h.repl == repl)
}

unsafe fn set_prot(addr: u64, prot: i32) -> bool {
    let page = addr & !0xFFF;
    let span = if (addr + 16) - page > 0x1000 { 0x2000 } else { 0x1000 };
    mach_vm_protect(mach_task_self_, page, span, 0, prot) == KERN_SUCCESS
}

/// Hook de VTABLE (type 2, complementar ao inline hook): troca o slot `slot_idx` (índice
/// em u64) da vtable apontada por `vtbl` pelo ponteiro `replacement`. Devolve o ponteiro
/// ORIGINAL do slot (pra encadear/chamar o método nativo). **NÃO patcha __TEXT** — só a
/// página da vtable (read-only/__DATA_CONST), tornada gravável via mach_vm_protect (COW).
/// Cobre qualquer método C++ VIRTUAL sem mexer em código, sem risco de relocação de prólogo.
///
/// # Safety
/// `vtbl` precisa apontar pra uma vtable válida e `slot_idx` ser um slot existente.
pub unsafe fn trocar_slot_da_vtable(vtbl: *mut u64, slot_idx: usize, replacement: *const c_void) -> Option<*const c_void> {
    if vtbl.is_null() {
        return None;
    }
    let slot = vtbl.add(slot_idx);
    let original = slot.read() as *const c_void;
    let addr = slot as u64;
    if !set_prot(addr, VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY) {
        return None;
    }
    slot.write(replacement as u64);
    set_prot(addr, VM_PROT_READ); // restaura RO (vtable é dado, não código)
    Some(original)
}

/// Desfaz um `trocar_slot_da_vtable`, restaurando o ponteiro original no slot.
/// # Safety
/// Mesmos requisitos do `trocar_slot_da_vtable`.
pub unsafe fn restaurar_slot_da_vtable(vtbl: *mut u64, slot_idx: usize, original: *const c_void) {
    if vtbl.is_null() {
        return;
    }
    let slot = vtbl.add(slot_idx);
    let addr = slot as u64;
    if set_prot(addr, VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY) {
        slot.write(original as u64);
        set_prot(addr, VM_PROT_READ);
    }
}

/// Substituto bobo só p/ o self-test (nunca chamado de fato — a janela hook→unhook é síncrona).
#[cfg(feature = "devtools")]
unsafe extern "C" fn vtable_dummy_repl() -> u64 {
    0xBADC_0FFE_E0DD_F00D
}

/// Self-test do `trocar_slot_da_vtable`/`restaurar_slot_da_vtable` (DEV) contra a vtable REAL de um objeto vivo: prova o
/// write em __DATA_CONST via COW + o restore. Acha um slot RARO com ptr de código no módulo (começa
/// alto p/ evitar métodos hot), hooka→confirma slot==dummy→desfaz→confirma slot==orig. Janela
/// síncrona (nanos, sem yield) → o jogo praticamente não chama o slot nesse meio. Retorna relatório.
#[cfg(feature = "devtools")]
pub unsafe fn testar_troca_de_slot(obj: *mut c_void) -> String {
    if obj.is_null() || !is_readable(obj as *const c_void, 8) {
        return "[vtbl-test] obj inválido".into();
    }
    let vtbl = core::ptr::read_unaligned(obj as *const *mut u64);
    if vtbl.is_null() || !is_readable(vtbl as *const c_void, 8 * 64) {
        return "[vtbl-test] vtable ilegível".into();
    }
    let vtbl_static = crate::un_rebase(vtbl as *const c_void);
    let in_module = (0x1_0000_0000..0x1_0A00_0000).contains(&vtbl_static);
    let mut slot_idx = 0usize;
    for i in 30..64usize {
        let v = vtbl.add(i).read();
        let st = crate::un_rebase(v as *const c_void);
        if (0x1_0000_0000..0x1_0A00_0000).contains(&st) {
            slot_idx = i;
            break;
        }
    }
    if slot_idx == 0 {
        return format!(
            "[vtbl-test] vtbl static {vtbl_static:#x} in_module={in_module} — nenhum slot de código em [30,64)"
        );
    }
    let before = vtbl.add(slot_idx).read();
    let dummy = vtable_dummy_repl as *const c_void;
    let orig = match trocar_slot_da_vtable(vtbl, slot_idx, dummy) {
        Some(o) => o,
        None => return format!("[vtbl-test] trocar_slot_da_vtable FALHOU (set_prot COW recusou) slot={slot_idx}"),
    };
    let hooked = vtbl.add(slot_idx).read();
    restaurar_slot_da_vtable(vtbl, slot_idx, orig);
    let restored = vtbl.add(slot_idx).read();
    format!(
        "[vtbl-test] vtbl static {vtbl_static:#x} in_module(__DATA_CONST)={in_module} slot={slot_idx}: hook {before:#x}->{hooked:#x} (==dummy {}) | unhook ->{restored:#x} (==orig {}) | OK={}",
        hooked == dummy as u64,
        restored == before,
        hooked == dummy as u64 && restored == before
    )
}

#[cfg(feature = "devtools")]
static VTBL_TEST_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Roda o testar_troca_de_slot 1x em gameplay quando `~/.bwms-vtable-test` existe (dev).
#[cfg(feature = "devtools")]
pub unsafe fn testar_troca_de_slot_uma_vez(obj: *mut c_void) {
    let on = crate::marcador_de_dev(".bwms-vtable-test");
    if !on || obj.is_null() {
        return;
    }
    if VTBL_TEST_DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    crate::log(&testar_troca_de_slot(obj));
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- contrato attach/detach: bookkeeping puro (LIFO + original verdadeiro) ----

    fn hk(target: u64, tag: u8) -> Hook {
        Hook { target, orig: [tag; 16], tramp: 0, repl: 0 } // `orig` marcado com o tag p/ rastrear qual bytes voltam
    }

    #[test]
    fn attach_detach_lifo_e_original_verdadeiro() {
        // 2 hooks no MESMO alvo 0xA (tags 1 e 2), 1 num alvo 0xB (tag 9). Ordem de inserção = Vec.
        let hooks = vec![hk(0xA, 1), hk(0xB, 9), hk(0xA, 2)];
        // LIFO no 0xA = o mais recente = índice 2 (tag 2)
        assert_eq!(latest_index_for(&hooks, 0xA), Some(2));
        // no 0xB só há um (índice 1)
        assert_eq!(latest_index_for(&hooks, 0xB), Some(1));
        // alvo sem hook
        assert_eq!(latest_index_for(&hooks, 0xC), None);
        // original verdadeiro do 0xA = o do PRIMEIRO hook (tag 1), não o do último
        assert_eq!(true_original_of(&hooks, 0xA), Some([1u8; 16]));
        assert_eq!(true_original_of(&hooks, 0xB), Some([9u8; 16]));
        assert_eq!(true_original_of(&hooks, 0xC), None);
        // contagem por alvo
        assert_eq!(count_for(&hooks, 0xA), 2);
        assert_eq!(count_for(&hooks, 0xB), 1);
        assert_eq!(count_for(&hooks, 0xC), 0);
    }

    #[test]
    fn attach_detach_desfaz_em_ordem_lifo() {
        // Simula o desfazer que o `remover` faz (remove o latest_index_for a cada passo) e confere
        // que os bytes restaurados saem na ordem LIFO: primeiro o penúltimo estado, depois o original.
        let mut hooks = vec![hk(0xA, 1), hk(0xA, 2), hk(0xA, 3)]; // 3 hooks empilhados no 0xA
        let mut restaurados = Vec::new();
        while let Some(pos) = latest_index_for(&hooks, 0xA) {
            restaurados.push(hooks[pos].orig[0]); // tag dos bytes que o remover escreveria
            hooks.remove(pos);
        }
        // reverte 3→2→1 (LIFO): cada um restaura o estado anterior a ele; o último restaura o original.
        assert_eq!(restaurados, vec![3, 2, 1]);
        assert!(hooks.is_empty());
    }

    #[test]
    fn attach_detach_um_hook_e_identico_ao_antigo() {
        // Caso comum (1 hook no alvo): latest == first, comportamento idêntico ao `position` antigo.
        let hooks = vec![hk(0xA, 7), hk(0xB, 8)];
        assert_eq!(latest_index_for(&hooks, 0xA), Some(0));
        assert_eq!(true_original_of(&hooks, 0xA), Some([7u8; 16]));
        assert_eq!(count_for(&hooks, 0xA), 1);
    }

    // Os alvos dos testes de hook de verdade ficam numa seção própria, cada um no começo de uma página de 16 KB com o resto dela vazio. A escrita tira
    // o `X` da página do alvo; com o alvo na mesma página de `instalar_com_threads_paradas`/`remover_com_threads_paradas` (layout do binário
    // de teste depois do 20c-1), a própria thread que escreve caía com SIGBUS. No jogo o alvo é do binário do jogo e quem escreve está no dylib.
    core::arch::global_asm!(
        ".pushsection __TEXT,__bwms_alvos,regular,pure_instructions",
        ".p2align 14",
        ".globl _bwms_teste_alvo_do_revert",
        "_bwms_teste_alvo_do_revert:",
        "mov x9, #3",
        "mul x0, x0, x9",
        "mov x9, #0x5a5a",
        "eor x0, x0, x9",
        "add x0, x0, #7",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_do_ciclo",
        "_bwms_teste_alvo_do_ciclo:",
        "mov x9, #5",
        "mul x0, x0, x9",
        "mov x9, #0x3c3c",
        "eor x0, x0, x9",
        "add x0, x0, #11",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_do_remover",
        "_bwms_teste_alvo_do_remover:",
        "mov x9, #7",
        "mul x0, x0, x9",
        "mov x9, #0x1e1e",
        "eor x0, x0, x9",
        "add x0, x0, #13",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_da_api",
        "_bwms_teste_alvo_da_api:",
        "mov x9, #9",
        "mul x0, x0, x9",
        "mov x9, #0x0f0f",
        "eor x0, x0, x9",
        "add x0, x0, #17",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_dos_donos",
        "_bwms_teste_alvo_dos_donos:",
        "mov x9, #11",
        "mul x0, x0, x9",
        "mov x9, #0x7878",
        "eor x0, x0, x9",
        "add x0, x0, #19",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_da_sonda",
        "_bwms_teste_alvo_da_sonda:",
        "mov x9, #13",
        "mul x0, x0, x9",
        "mov x9, #0x2d2d",
        "eor x0, x0, x9",
        "add x0, x0, #23",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_do_empilhar",
        "_bwms_teste_alvo_do_empilhar:",
        "mov x9, #15",
        "mul x0, x0, x9",
        "mov x9, #0x4b4b",
        "eor x0, x0, x9",
        "add x0, x0, #29",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_da_corrida",
        "_bwms_teste_alvo_da_corrida:",
        "mov x9, #17",
        "mul x0, x0, x9",
        "mov x9, #0x6969",
        "eor x0, x0, x9",
        "add x0, x0, #31",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_do_prologo",
        "_bwms_teste_alvo_do_prologo:",
        "mov x9, #19",
        "mul x0, x0, x9",
        "mov x9, #0x1e1e",
        "eor x0, x0, x9",
        "add x0, x0, #37",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_do_revert_pelo_instalar",
        "_bwms_teste_alvo_do_revert_pelo_instalar:",
        "mov x9, #21",
        "mul x0, x0, x9",
        "mov x9, #0x5c5c",
        "eor x0, x0, x9",
        "add x0, x0, #41",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_do_ciclo_pelo_instalar",
        "_bwms_teste_alvo_do_ciclo_pelo_instalar:",
        "mov x9, #23",
        "mul x0, x0, x9",
        "mov x9, #0x3a3a",
        "eor x0, x0, x9",
        "add x0, x0, #43",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_do_perto",
        "_bwms_teste_alvo_do_perto:",
        "mov x9, #25",
        "mul x0, x0, x9",
        "mov x9, #0x7171",
        "eor x0, x0, x9",
        "add x0, x0, #47",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_da_ponte",
        "_bwms_teste_alvo_da_ponte:",
        "mov x9, #27",
        "mul x0, x0, x9",
        "mov x9, #0x2e2e",
        "eor x0, x0, x9",
        "add x0, x0, #53",
        "ret",
        ".p2align 14",
        ".globl _bwms_teste_alvo_do_reaproveitar",
        "_bwms_teste_alvo_do_reaproveitar:",
        "mov x9, #29",
        "mul x0, x0, x9",
        "mov x9, #0x6b6b",
        "eor x0, x0, x9",
        "add x0, x0, #59",
        "ret",
        ".p2align 14",
        ".popsection",
    );

    unsafe extern "C" {
        fn bwms_teste_alvo_do_revert(x: u64) -> u64;
        fn bwms_teste_alvo_do_ciclo(x: u64) -> u64;
        fn bwms_teste_alvo_do_remover(x: u64) -> u64;
        fn bwms_teste_alvo_da_api(x: u64) -> u64;
        fn bwms_teste_alvo_dos_donos(x: u64) -> u64;
        fn bwms_teste_alvo_da_sonda(x: u64) -> u64;
        fn bwms_teste_alvo_do_empilhar(x: u64) -> u64;
        fn bwms_teste_alvo_da_corrida(x: u64) -> u64;
        fn bwms_teste_alvo_do_prologo(x: u64) -> u64;
        fn bwms_teste_alvo_do_revert_pelo_instalar(x: u64) -> u64;
        fn bwms_teste_alvo_do_ciclo_pelo_instalar(x: u64) -> u64;
        fn bwms_teste_alvo_do_perto(x: u64) -> u64;
        fn bwms_teste_alvo_da_ponte(x: u64) -> u64;
        fn bwms_teste_alvo_do_reaproveitar(x: u64) -> u64;
    }

    fn seguro(f: unsafe extern "C" fn(u64) -> u64) -> extern "C" fn(u64) -> u64 {
        // SAFETY: os alvos em assembly só fazem aritmética em x0 e x9.
        unsafe { std::mem::transmute(f) }
    }

    extern "C" fn substituto_do_revert(x: u64) -> u64 {
        x.wrapping_add(1_000_000)
    }

    /// Codeware `#118`, boot 178: o jogo travou na remoção dos hooks de evento. Hook de verdade num alvo do próprio binário de teste (`por` devolve o
    /// trampolim ou nulo), 4 threads chamando o alvo sem parar e `tirar` numa thread à parte, com teto de 10 s: travar aqui reproduz o boot sem jogo.
    /// `tirar` devolve se o alvo foi restaurado.
    fn revert_com_chamadas_concorrentes(chamar: extern "C" fn(u64) -> u64, por: fn(*mut c_void) -> *mut c_void, tirar: fn(*mut c_void) -> bool) {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let esperado = chamar(5);
        let alvo = chamar as *mut c_void;
        assert!(!por(alvo).is_null(), "hook não instalou");
        assert_eq!(chamar(5), 1_000_005, "o hook não desviou a chamada");
        let parar = Arc::new(AtomicBool::new(false));
        let trabalhadoras: Vec<_> = (0..4)
            .map(|_| {
                let parar = parar.clone();
                std::thread::spawn(move || {
                    let mut n = 0u64;
                    while !parar.load(Ordering::Relaxed) {
                        n = n.wrapping_add(chamar(std::hint::black_box(n)));
                    }
                    n
                })
            })
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let (tx, rx) = std::sync::mpsc::channel();
        let alvo_end = alvo as usize;
        std::thread::spawn(move || {
            let _ = tx.send(tirar(alvo_end as *mut c_void));
        });
        let resultado = rx.recv_timeout(std::time::Duration::from_secs(10));
        parar.store(true, Ordering::Relaxed);
        for t in trabalhadoras {
            let _ = t.join();
        }
        let restaurado = resultado.expect("a remoção com as threads paradas travou");
        assert!(restaurado, "o alvo não foi restaurado");
        assert_eq!(chamar(5), esperado, "o alvo restaurado não voltou ao corpo original");
    }

    #[test]
    fn revert_com_threads_paradas_com_chamadas_concorrentes() {
        revert_com_chamadas_concorrentes(
            std::hint::black_box(seguro(bwms_teste_alvo_do_revert)),
            // SAFETY: alvo e substituto são funções deste binário com a mesma ABI; o teste restaura o alvo antes de sair.
            |alvo| unsafe { instalar_com_threads_paradas(alvo, substituto_do_revert as *mut c_void, None) }.unwrap_or(std::ptr::null_mut()),
            // SAFETY: o alvo foi substituído por `instalar_com_threads_paradas` acima.
            |alvo| unsafe { remover_com_threads_paradas(alvo) }.0,
        );
    }

    /// O mesmo, pelo caminho dos sítios internos sem `destino`: [`instalar`] e [`remover`].
    #[test]
    fn revert_pelo_instalar_e_remover_com_chamadas_concorrentes() {
        revert_com_chamadas_concorrentes(
            std::hint::black_box(seguro(bwms_teste_alvo_do_revert_pelo_instalar)),
            // SAFETY: alvo e substituto são funções deste binário com a mesma ABI; o teste restaura o alvo antes de sair.
            |alvo| unsafe { instalar(alvo, substituto_do_revert as *mut c_void) }.unwrap_or(std::ptr::null_mut()),
            // SAFETY: o alvo foi substituído por `instalar` acima.
            |alvo| unsafe { remover(alvo) },
        );
    }

    /// O mesmo, pelo caminho que um plugin usa: `inline_hook` e `inline_revert` da C-ABI.
    #[test]
    fn revert_pela_api_de_plugin_com_chamadas_concorrentes() {
        use crate::mods_do_usuario::api::BWMS_API;
        revert_com_chamadas_concorrentes(
            std::hint::black_box(seguro(bwms_teste_alvo_da_api)),
            // SAFETY: alvo e substituto são funções deste binário com a mesma ABI; o teste restaura o alvo antes de sair.
            |alvo| unsafe { (BWMS_API.inline_hook)(alvo, substituto_do_revert as *mut c_void) },
            |alvo| {
                // SAFETY: o alvo foi substituído por `inline_hook` acima.
                unsafe { (BWMS_API.inline_revert)(alvo) };
                ganchos_no_endereco(alvo) == 0
            },
        );
    }

    extern "C" fn substituto_do_dono_a(x: u64) -> u64 {
        x.wrapping_add(2_000_000)
    }

    extern "C" fn substituto_do_dono_b(x: u64) -> u64 {
        x.wrapping_add(3_000_000)
    }

    /// 2 plugins com hook no mesmo alvo pela C-ABI, B por cima de A. O detach de A é recusado sem tirar o hook de B e sem A perder a posse: depois que
    /// B sai, o detach de A funciona. Depois, com o hook de A tirado por `inline_revert` e um novo de B no alvo, o detach de A não tira o de B: o
    /// trampolim é a identidade de cada instalação e não se repete.
    #[test]
    fn detach_de_plugin_so_tira_o_proprio_hook_do_topo() {
        use crate::mods_do_usuario::api::BWMS_API;
        const A: u64 = 0xA0A0_0001;
        const B: u64 = 0xB0B0_0002;
        let chamar = std::hint::black_box(seguro(bwms_teste_alvo_dos_donos));
        let esperado = chamar(5);
        let alvo = chamar as *mut c_void;
        // SAFETY: alvo e substitutos são funções deste binário com a mesma ABI; nenhuma outra thread chama este alvo e o teste termina com ele restaurado.
        unsafe {
            assert!(!(BWMS_API.inline_hook_owned)(A, alvo, substituto_do_dono_a as *mut c_void).is_null(), "hook de A não instalou");
            assert!(!(BWMS_API.inline_hook_owned)(B, alvo, substituto_do_dono_b as *mut c_void).is_null(), "hook de B não instalou");
            assert_eq!(chamar(5), 3_000_005, "o hook de B não ficou no topo");

            assert!(!(BWMS_API.inline_detach)(A, alvo), "detach de A tirou o hook de B, que estava por cima");
            assert_eq!(ganchos_no_endereco(alvo), 2);
            assert_eq!(chamar(5), 3_000_005, "o hook de B saiu do alvo no detach recusado de A");

            assert!((BWMS_API.inline_detach)(B, alvo), "detach de B, no topo, recusado");
            assert_eq!(chamar(5), 2_000_005, "sem B, o hook de A não voltou a responder");
            assert!((BWMS_API.inline_detach)(A, alvo), "A perdeu a posse do próprio hook");
            assert_eq!(ganchos_no_endereco(alvo), 0);
            assert_eq!(chamar(5), esperado, "o alvo não voltou ao corpo original");

            assert!(!(BWMS_API.inline_hook_owned)(A, alvo, substituto_do_dono_a as *mut c_void).is_null(), "hook de A não instalou de novo");
            (BWMS_API.inline_revert)(alvo);
            assert!(!(BWMS_API.inline_hook_owned)(B, alvo, substituto_do_dono_b as *mut c_void).is_null(), "hook de B não instalou de novo");
            assert!((BWMS_API.inline_detach)(A, alvo), "hook de A já tirado: nada a desfazer");
            assert_eq!(chamar(5), 3_000_005, "o detach de A tirou o hook novo de B");
            assert!((BWMS_API.inline_detach)(B, alvo), "detach de B recusado");
            assert_eq!(chamar(5), esperado, "o alvo não voltou ao corpo original");
        }
    }

    /// Nenhum alvo de teste devolve isto: o substituto entrou em laço.
    const LACO: u64 = u64::MAX;

    thread_local! {
        /// Substitutos na pilha desta thread. Um trampolim que volta pro próprio substituto passa de 4 e devolve [`LACO`] em vez de estourar a pilha.
        static ENCADEADOS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    /// Chama o original publicado em `orig` e soma `marca`, como um substituto de hook que chama a original.
    fn pelo_original(orig: &AtomicU64, x: u64, marca: u64) -> u64 {
        let n = ENCADEADOS.with(|c| c.replace(c.get() + 1));
        let r = if n >= 4 {
            LACO
        } else {
            // SAFETY: os `orig` destes testes recebem o trampolim de um alvo em assembly com a ABI `fn(u64) -> u64`.
            let f: extern "C" fn(u64) -> u64 = unsafe { std::mem::transmute(original_publicado(orig)) };
            match f(x) {
                LACO => LACO,
                v => v.wrapping_add(marca),
            }
        };
        ENCADEADOS.with(|c| c.set(n));
        r
    }

    static ORIG_DA_SONDA: AtomicU64 = AtomicU64::new(0);
    static ORIG_DE_OUTRO_SITIO: AtomicU64 = AtomicU64::new(0);

    extern "C" fn sonda_do_teste(x: u64) -> u64 {
        pelo_original(&ORIG_DA_SONDA, x, 4_000_000)
    }

    /// `install_hook_probe!` 2× no mesmo alvo (o corpo dele, [`instalar_sonda`]): a 2ª não faz nada e o alvo fica com 1 hook. Outro sítio com a mesma
    /// sonda que leu o `orig` antes da publicação é recusado sob a trava sem tocar no original publicado: o `orig` apontando pro trampolim de um 2º
    /// salto faria a sonda chamar a si mesma.
    #[test]
    fn sonda_no_mesmo_alvo_entra_uma_vez_e_a_recusa_nao_toca_no_original() {
        let chamar = std::hint::black_box(seguro(bwms_teste_alvo_da_sonda));
        let esperado = chamar(5);
        let alvo = chamar as *mut c_void;
        let sonda = sonda_do_teste as *mut c_void;
        // SAFETY: alvo e sonda são funções deste binário com a mesma ABI; nenhuma outra thread chama este alvo e o teste termina com ele restaurado.
        unsafe {
            assert_eq!(instalar_sonda(alvo, sonda, &ORIG_DA_SONDA), Some(true), "a sonda não instalou");
            let publicado = ORIG_DA_SONDA.load(Ordering::Relaxed);
            assert_ne!(publicado, 0, "o original não foi publicado");
            assert_eq!(chamar(5), esperado + 4_000_000, "a sonda não desviou a chamada");

            assert_eq!(instalar_sonda(alvo, sonda, &ORIG_DA_SONDA), None, "a 2ª chamada, com o original já publicado, fez alguma coisa");
            assert_eq!(ganchos_no_endereco(alvo), 1);

            assert!(instalar_com_threads_paradas(alvo, sonda, Some(&ORIG_DA_SONDA)).is_none(), "a mesma sonda entrou 2× no alvo");
            assert_eq!(ORIG_DA_SONDA.load(Ordering::Relaxed), publicado, "a recusa trocou o original publicado");
            assert_eq!(ganchos_no_endereco(alvo), 1);
            assert_eq!(chamar(5), esperado + 4_000_000, "a sonda não chega mais ao corpo do alvo");

            assert_eq!(instalar_sonda(alvo, sonda, &ORIG_DE_OUTRO_SITIO), None, "a sonda já no alvo deu falha em vez de já no ar");
            assert_eq!(ORIG_DE_OUTRO_SITIO.load(Ordering::Relaxed), 0, "a recusa publicou um original");
            assert_eq!(ganchos_no_endereco(alvo), 1);

            assert!(remover_com_threads_paradas(alvo).0, "o hook não saiu");
        }
        assert_eq!(chamar(5), esperado, "o alvo não voltou ao corpo original");
    }

    static ORIG_DE_BAIXO: AtomicU64 = AtomicU64::new(0);
    static ORIG_DE_CIMA: AtomicU64 = AtomicU64::new(0);

    extern "C" fn substituto_de_baixo(x: u64) -> u64 {
        pelo_original(&ORIG_DE_BAIXO, x, 5_000_000)
    }

    extern "C" fn substituto_de_cima(x: u64) -> u64 {
        pelo_original(&ORIG_DE_CIMA, x, 60_000_000)
    }

    /// Substitutos diferentes no mesmo alvo continuam empilhando (um plugin da `BwmsApi` por cima de uma sonda nossa): a chamada passa pelos dois e
    /// chega ao corpo do alvo.
    #[test]
    fn substituto_diferente_empilha_no_mesmo_alvo() {
        let chamar = std::hint::black_box(seguro(bwms_teste_alvo_do_empilhar));
        let esperado = chamar(5);
        let alvo = chamar as *mut c_void;
        // SAFETY: alvo e substitutos são funções deste binário com a mesma ABI; nenhuma outra thread chama este alvo e o teste termina com ele restaurado.
        unsafe {
            assert_eq!(instalar_sonda(alvo, substituto_de_baixo as *mut c_void, &ORIG_DE_BAIXO), Some(true), "o de baixo não instalou");
            assert_eq!(instalar_sonda(alvo, substituto_de_cima as *mut c_void, &ORIG_DE_CIMA), Some(true), "o de cima não empilhou");
            assert_eq!(ganchos_no_endereco(alvo), 2);
            assert_eq!(chamar(5), esperado + 65_000_000, "a chamada não passou pelos dois substitutos");
            assert!(remover_com_threads_paradas(alvo).0, "o de cima não saiu");
            assert_eq!(chamar(5), esperado + 5_000_000, "sem o de cima, o de baixo não responde");
            assert!(remover_com_threads_paradas(alvo).0, "o de baixo não saiu");
        }
        assert_eq!(ganchos_no_endereco(alvo), 0);
        assert_eq!(chamar(5), esperado, "o alvo não voltou ao corpo original");
    }

    static ORIG_CORRIDA_A: AtomicU64 = AtomicU64::new(0);
    static ORIG_CORRIDA_B: AtomicU64 = AtomicU64::new(0);

    extern "C" fn corrida_a(x: u64) -> u64 {
        pelo_original(&ORIG_CORRIDA_A, x, 7_000_000)
    }

    extern "C" fn corrida_b(x: u64) -> u64 {
        pelo_original(&ORIG_CORRIDA_B, x, 80_000_000)
    }

    /// 2 threads instalando no mesmo alvo ao mesmo tempo (o `RegisterCallback` e a tabela do `resource.patch` no `OnDependenciesReady`; 2 sítios do
    /// macro com o mesmo `orig`). Com o mesmo substituto, 1 só entra e o original publicado é o do hook aceito. Com substitutos diferentes, quem perde
    /// a corrida pelos bytes do alvo é recusado em vez de escrever por cima com um trampolim relocado de bytes velhos. Em toda volta a lista e o alvo
    /// concordam: `ganchos_no_endereco` conta os aceitos, e a chamada passa por todos eles.
    #[test]
    fn duas_threads_instalando_no_mesmo_alvo() {
        use std::sync::{Arc, Barrier};
        let chamar = std::hint::black_box(seguro(bwms_teste_alvo_da_corrida));
        let esperado = chamar(5);
        let alvo = chamar as usize;
        for volta in 0..40 {
            let mesmo = volta % 2 == 0;
            let pares: [(extern "C" fn(u64) -> u64, &'static AtomicU64, u64); 2] = [
                (corrida_a, &ORIG_CORRIDA_A, 7_000_000),
                if mesmo { (corrida_a, &ORIG_CORRIDA_A, 7_000_000) } else { (corrida_b, &ORIG_CORRIDA_B, 80_000_000) },
            ];
            let largada = Arc::new(Barrier::new(2));
            let fios: Vec<_> = pares
                .iter()
                .map(|&(repl, orig, _)| {
                    let largada = largada.clone();
                    std::thread::spawn(move || {
                        largada.wait();
                        // SAFETY: alvo e substitutos são funções deste binário com a mesma ABI; ninguém chama o alvo durante a corrida.
                        unsafe { instalar_com_threads_paradas(alvo as *mut c_void, repl as *mut c_void, Some(orig)) }.map(|t| t as u64)
                    })
                })
                .collect();
            let postos: Vec<Option<u64>> = fios.into_iter().map(|f| f.join().expect("thread da corrida caiu")).collect();
            let aceitos = postos.iter().flatten().count();
            assert_eq!(ganchos_no_endereco(alvo as *mut c_void), aceitos, "volta {volta}: a lista e o alvo discordam");
            if mesmo {
                assert_eq!(aceitos, 1, "volta {volta}: o mesmo substituto entrou {aceitos}×");
                assert_eq!(postos.iter().flatten().next().copied(), Some(ORIG_CORRIDA_A.load(Ordering::Relaxed)), "volta {volta}: o original publicado não é o do hook aceito");
            } else {
                assert!(aceitos >= 1, "volta {volta}: os dois foram recusados");
            }
            let soma: u64 = pares.iter().zip(&postos).filter(|(_, p)| p.is_some()).map(|((_, _, marca), _)| marca).sum();
            assert_eq!(chamar(5), esperado + soma, "volta {volta}: a chamada não passou pelos hooks aceitos");
            for _ in 0..aceitos {
                // SAFETY: os hooks aceitos foram postos por `instalar_com_threads_paradas`.
                assert!(unsafe { remover_com_threads_paradas(alvo as *mut c_void) }.0, "volta {volta}: um hook não saiu");
            }
            assert_eq!(chamar(5), esperado, "volta {volta}: o alvo não voltou ao corpo original");
            ORIG_CORRIDA_A.store(0, Ordering::Relaxed);
            ORIG_CORRIDA_B.store(0, Ordering::Relaxed);
        }
    }

    static ORIG_DO_PROLOGO: AtomicU64 = AtomicU64::new(0);

    extern "C" fn substituto_do_prologo(x: u64) -> u64 {
        pelo_original(&ORIG_DO_PROLOGO, x, 9_000_000)
    }

    /// N5: `garantir_gancho_de_dependencias` e o `C175_ATIVO` gravam o estado pelo resultado de [`instalar_conferindo_prologo`], então `None` tem de
    /// ser só "este substituto está no alvo". Outro hook no alvo é `Some(false)`: o substituto não está no ar. Na corrida do `RegisterCallback` com a
    /// tabela do `resource.patch`, quem perde recebe `None`, nunca `Some(false)`, que gravaria "fora do ar" com o hook no alvo.
    #[test]
    fn prologo_conferido_devolve_none_so_com_o_proprio_substituto_no_alvo() {
        use std::sync::{Arc, Barrier};
        let chamar = std::hint::black_box(seguro(bwms_teste_alvo_do_prologo));
        let esperado = chamar(5);
        let alvo = chamar as usize;
        let substituto = substituto_do_prologo as *mut c_void as usize;
        let (end, subst) = (alvo as *mut c_void, substituto as *mut c_void);
        // SAFETY: 4 bytes do começo de uma função deste binário.
        let prologo = unsafe { (end as *const u32).read_unaligned() };
        // SAFETY: alvo e substitutos são funções deste binário com a mesma ABI; nenhuma outra thread chama este alvo e o teste termina com ele restaurado.
        unsafe {
            assert_eq!(instalar_conferindo_prologo_em(end, !prologo, subst, &ORIG_DO_PROLOGO), Some(false), "prólogo diferente instalou");
            assert_eq!(ganchos_no_endereco(end), 0);

            assert!(instalar_com_threads_paradas(end, substituto_do_revert as *mut c_void, None).is_some(), "o outro hook não instalou");
            assert_eq!(instalar_conferindo_prologo_em(end, prologo, subst, &ORIG_DO_PROLOGO), Some(false), "outro hook no alvo deu já no ar");
            assert_eq!(ORIG_DO_PROLOGO.load(Ordering::Relaxed), 0, "a recusa publicou um original");
            assert_eq!(ganchos_no_endereco(end), 1);
            assert!(remover_com_threads_paradas(end).0, "o outro hook não saiu");

            assert_eq!(instalar_conferindo_prologo_em(end, prologo, subst, &ORIG_DO_PROLOGO), Some(true), "o substituto não instalou");
            let publicado = ORIG_DO_PROLOGO.load(Ordering::Relaxed);
            assert_eq!(chamar(5), esperado + 9_000_000, "o substituto não desviou a chamada");
            assert_eq!(instalar_conferindo_prologo_em(end, prologo, subst, &ORIG_DO_PROLOGO), None, "o substituto já no alvo deu falha em vez de já no ar");
            assert_eq!(ORIG_DO_PROLOGO.load(Ordering::Relaxed), publicado, "a 2ª chamada trocou o original publicado");
            assert_eq!(ganchos_no_endereco(end), 1);
            assert!(remover_com_threads_paradas(end).0, "o substituto não saiu");
        }
        assert_eq!(chamar(5), esperado, "o alvo não voltou ao corpo original");
        ORIG_DO_PROLOGO.store(0, Ordering::Relaxed);

        for volta in 0..20 {
            let largada = Arc::new(Barrier::new(2));
            let fios: Vec<_> = (0..2)
                .map(|_| {
                    let largada = largada.clone();
                    std::thread::spawn(move || {
                        largada.wait();
                        // SAFETY: o mesmo alvo e substituto de cima; ninguém chama o alvo durante a corrida.
                        unsafe { instalar_conferindo_prologo_em(alvo as *mut c_void, prologo, substituto as *mut c_void, &ORIG_DO_PROLOGO) }
                    })
                })
                .collect();
            let mut r: Vec<Option<bool>> = fios.into_iter().map(|f| f.join().expect("thread da corrida caiu")).collect();
            r.sort();
            assert_eq!(r, [None, Some(true)], "volta {volta}: o perdedor não recebeu já no ar");
            assert_eq!(ganchos_no_endereco(end), 1, "volta {volta}");
            assert_eq!(chamar(5), esperado + 9_000_000, "volta {volta}: a chamada não passou pelo substituto");
            // SAFETY: o hook aceito foi posto por `instalar_com_threads_paradas`.
            assert!(unsafe { remover_com_threads_paradas(end) }.0, "volta {volta}: o substituto não saiu");
            assert_eq!(chamar(5), esperado, "volta {volta}: o alvo não voltou ao corpo original");
            ORIG_DO_PROLOGO.store(0, Ordering::Relaxed);
        }
    }

    extern "C" fn substituto_do_ciclo(x: u64) -> u64 {
        x.wrapping_add(1_000_000)
    }

    /// Boot 181: 18 ciclos de instalar e tirar os hooks de evento com o motor chamando os alvos. Aqui, 50 ciclos de `ciclo` (põe e tira 1 hook no
    /// alvo e devolve se os dois deram certo) enquanto 4 threads chamam o alvo sem parar: toda chamada devolve o corpo original ou `desviado`, e o
    /// alvo termina no original.
    fn ciclos_com_chamadas_concorrentes(chamar: extern "C" fn(u64) -> u64, desviado: u64, ciclo: fn(*mut c_void) -> bool) {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::Arc;
        let esperado = chamar(5);
        let alvo_end = chamar as usize;
        let parar = Arc::new(AtomicBool::new(false));
        let estranhos = Arc::new(AtomicU64::new(0));
        let trabalhadoras: Vec<_> = (0..4)
            .map(|_| {
                let (parar, estranhos) = (parar.clone(), estranhos.clone());
                std::thread::spawn(move || {
                    while !parar.load(Ordering::Relaxed) {
                        let r = chamar(std::hint::black_box(5));
                        if r != esperado && r != desviado {
                            estranhos.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })
            })
            .collect();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let certos = (0..50).map(|_| ciclo(alvo_end as *mut c_void) as u32).sum::<u32>();
            let _ = tx.send(certos);
        });
        let certos = rx.recv_timeout(std::time::Duration::from_secs(30));
        parar.store(true, Ordering::Relaxed);
        for t in trabalhadoras {
            let _ = t.join();
        }
        assert_eq!(certos.expect("instalar e tirar com as threads paradas travou"), 50);
        assert_eq!(estranhos.load(Ordering::Relaxed), 0, "chamada devolveu algo que não é o original nem o substituto");
        assert_eq!(chamar(5), esperado, "o alvo não terminou no corpo original");
    }

    #[test]
    fn instalar_e_tirar_com_threads_paradas_em_ciclos() {
        ciclos_com_chamadas_concorrentes(std::hint::black_box(seguro(bwms_teste_alvo_do_ciclo)), 1_000_005, |alvo| {
            let orig = AtomicU64::new(0);
            // SAFETY: alvo e substituto são funções deste binário com a mesma ABI; cada ciclo tira o que pôs.
            let posto = unsafe { instalar_com_threads_paradas(alvo, substituto_do_ciclo as *mut c_void, Some(&orig)) };
            let tirado = unsafe { remover_com_threads_paradas(alvo) }.0;
            posto.is_some() && orig.load(Ordering::Relaxed) != 0 && tirado
        });
    }

    /// O mesmo, pelo caminho dos sítios internos sem `destino`: [`instalar`] e [`remover`].
    #[test]
    fn instalar_e_tirar_pelo_instalar_e_remover_em_ciclos() {
        ciclos_com_chamadas_concorrentes(std::hint::black_box(seguro(bwms_teste_alvo_do_ciclo_pelo_instalar)), 1_000_005, |alvo| {
            // SAFETY: alvo e substituto são funções deste binário com a mesma ABI; cada ciclo tira o que pôs.
            unsafe { instalar(alvo, substituto_do_ciclo as *mut c_void).is_some() && remover(alvo) }
        });
    }

    /// Nenhum alvo de teste devolve isto: o substituto foi chamado com o original ainda não publicado.
    const SEM_ORIGINAL: u64 = u64::MAX - 1;

    /// Chama o original lido de `orig` sem esperar ([`original_publicado`] esperaria) e soma `marca`. Uma chamada desviada antes da publicação lê 0 e
    /// devolve [`SEM_ORIGINAL`], que [`ciclos_com_chamadas_concorrentes`] conta como estranha.
    fn pelo_publicado(orig: &AtomicU64, x: u64, marca: u64) -> u64 {
        match orig.load(Ordering::Relaxed) {
            0 => SEM_ORIGINAL,
            o => {
                // SAFETY: os `orig` destes testes recebem o trampolim de um alvo em assembly com a ABI `fn(u64) -> u64`.
                let f: extern "C" fn(u64) -> u64 = unsafe { std::mem::transmute(o) };
                f(x).wrapping_add(marca)
            }
        }
    }

    static ORIG_DO_PERTO: AtomicU64 = AtomicU64::new(0);
    static ORIG_DA_PONTE: AtomicU64 = AtomicU64::new(0);

    extern "C" fn substituto_do_perto(x: u64) -> u64 {
        pelo_publicado(&ORIG_DO_PERTO, x, 11_000_000)
    }

    extern "C" fn substituto_da_ponte(x: u64) -> u64 {
        pelo_publicado(&ORIG_DA_PONTE, x, 13_000_000)
    }

    /// [`instalar_perto`] pela escrita única: o mesmo substituto não entra 2× e a recusa não toca no `destino`. Depois, os ciclos com chamadas
    /// concorrentes: o substituto lê o original do `destino` sem esperar, então uma chamada desviada antes da publicação apareceria como estranha.
    #[test]
    fn instalar_perto_com_threads_paradas() {
        let chamar = std::hint::black_box(seguro(bwms_teste_alvo_do_perto));
        let esperado = chamar(5);
        let alvo = chamar as *mut c_void;
        let substituto = substituto_do_perto as *mut c_void;
        // SAFETY: alvo e substituto são funções deste binário com a mesma ABI; nenhuma outra thread chama este alvo e o teste termina com ele restaurado.
        unsafe {
            let tramp = instalar_perto(alvo, substituto, Some(&ORIG_DO_PERTO)).expect("o B de 4 bytes não instalou");
            assert_eq!(ORIG_DO_PERTO.load(Ordering::Relaxed), tramp as u64, "o destino não recebeu o trampolim");
            assert_eq!(chamar(5), esperado + 11_000_000, "o B não desviou a chamada");
            assert!(instalar_perto(alvo, substituto, Some(&ORIG_DO_PERTO)).is_none(), "o mesmo substituto entrou 2× no alvo");
            assert_eq!(ORIG_DO_PERTO.load(Ordering::Relaxed), tramp as u64, "a recusa trocou o original publicado");
            assert_eq!(ganchos_no_endereco(alvo), 1);
            assert!(remover(alvo), "o B não saiu");
        }
        assert_eq!(chamar(5), esperado, "o alvo não voltou ao corpo original");
        ORIG_DO_PERTO.store(0, Ordering::Relaxed);
        ciclos_com_chamadas_concorrentes(chamar, esperado + 11_000_000, |alvo| {
            // SAFETY: o mesmo alvo e substituto de cima; cada ciclo tira o que pôs.
            let posto = unsafe { instalar_perto(alvo, substituto_do_perto as *mut c_void, Some(&ORIG_DO_PERTO)) };
            let publicado = posto.is_some_and(|t| ORIG_DO_PERTO.load(Ordering::Relaxed) == t as u64);
            // SAFETY: o hook foi posto por `instalar_perto` acima.
            publicado && unsafe { remover(alvo) }
        });
    }

    /// O mesmo para [`instalar_com_ponte`] (`ADRP+BR` de 8 bytes). A ponte é um `mmap` comum, que tem de cair a ±4 GB do alvo deste binário.
    #[test]
    fn instalar_com_ponte_com_threads_paradas() {
        let chamar = std::hint::black_box(seguro(bwms_teste_alvo_da_ponte));
        let esperado = chamar(5);
        let alvo = chamar as *mut c_void;
        let substituto = substituto_da_ponte as *mut c_void;
        // SAFETY: alvo e substituto são funções deste binário com a mesma ABI; nenhuma outra thread chama este alvo e o teste termina com ele restaurado.
        unsafe {
            let tramp = instalar_com_ponte(alvo, substituto, Some(&ORIG_DA_PONTE)).expect("o ADRP+BR de 8 bytes não instalou");
            assert_eq!(ORIG_DA_PONTE.load(Ordering::Relaxed), tramp as u64, "o destino não recebeu o trampolim");
            assert_eq!(chamar(5), esperado + 13_000_000, "o ADRP+BR não desviou a chamada");
            assert!(instalar_com_ponte(alvo, substituto, Some(&ORIG_DA_PONTE)).is_none(), "o mesmo substituto entrou 2× no alvo");
            assert_eq!(ORIG_DA_PONTE.load(Ordering::Relaxed), tramp as u64, "a recusa trocou o original publicado");
            assert_eq!(ganchos_no_endereco(alvo), 1);
            assert!(remover(alvo), "o ADRP+BR não saiu");
        }
        assert_eq!(chamar(5), esperado, "o alvo não voltou ao corpo original");
        ORIG_DA_PONTE.store(0, Ordering::Relaxed);
        ciclos_com_chamadas_concorrentes(chamar, esperado + 13_000_000, |alvo| {
            // SAFETY: o mesmo alvo e substituto de cima; cada ciclo tira o que pôs.
            let posto = unsafe { instalar_com_ponte(alvo, substituto_da_ponte as *mut c_void, Some(&ORIG_DA_PONTE)) };
            let publicado = posto.is_some_and(|t| ORIG_DA_PONTE.load(Ordering::Relaxed) == t as u64);
            // SAFETY: o hook foi posto por `instalar_com_ponte` acima.
            publicado && unsafe { remover(alvo) }
        });
    }

    static ORIG_DO_REAPROVEITAR: AtomicU64 = AtomicU64::new(0);
    static ORIG_POR_CIMA_DO_REAPROVEITAR: AtomicU64 = AtomicU64::new(0);

    extern "C" fn substituto_do_reaproveitar(x: u64) -> u64 {
        pelo_original(&ORIG_DO_REAPROVEITAR, x, 15_000_000)
    }

    extern "C" fn substituto_por_cima_do_reaproveitar(x: u64) -> u64 {
        pelo_original(&ORIG_POR_CIMA_DO_REAPROVEITAR, x, 170_000_000)
    }

    /// Entrada de [`TRAMPOLINS`] com os bytes do alvo e um conteúdo que não é o relocado.
    static TRAMPOLIM_FALSO: [u8; 4096] = [0; 4096];

    /// Codeware `#118` (F5-11): 8 ciclos de pôr e tirar 2 hooks empilhados no mesmo alvo usam 2 trampolins no total, 1 por bytes no alvo, e em todo
    /// ciclo a chamada passa pelos dois e chega ao corpo do alvo. A entrada da tabela com os mesmos bytes e outro conteúdo não é usada. O
    /// `inline_hook_owned` de plugin ganha página nova em toda instalação, fora da tabela.
    #[test]
    fn reinstalar_com_os_mesmos_bytes_reaproveita_o_trampolim() {
        use crate::mods_do_usuario::api::BWMS_API;
        const DONO: u64 = 0xF511_0001;
        let chamar = std::hint::black_box(seguro(bwms_teste_alvo_do_reaproveitar));
        let esperado = chamar(5);
        let alvo = chamar as *mut c_void;
        // SAFETY: 16 bytes do começo de uma função deste binário.
        let orig = unsafe { (alvo as *const [u8; 16]).read_unaligned() };
        TRAMPOLINS.lock().unwrap().push(Trampolim { alvo: alvo as u64, orig, tramp: TRAMPOLIM_FALSO.as_ptr() as u64 });
        let (mut de_baixo, mut de_cima, mut do_dono) = (Vec::new(), Vec::new(), Vec::new());
        // SAFETY: alvo e substitutos são funções deste binário com a mesma ABI; nenhuma outra thread chama este alvo e o teste termina com ele restaurado.
        unsafe {
            for ciclo in 0..8 {
                let baixo = instalar_com_threads_paradas(alvo, substituto_do_reaproveitar as *mut c_void, Some(&ORIG_DO_REAPROVEITAR))
                    .unwrap_or_else(|| panic!("ciclo {ciclo}: o hook de baixo não instalou"));
                assert_eq!(ORIG_DO_REAPROVEITAR.load(Ordering::Relaxed), baixo as u64, "ciclo {ciclo}: o destino não recebeu o trampolim de baixo");
                let cima = instalar_com_threads_paradas(alvo, substituto_por_cima_do_reaproveitar as *mut c_void, Some(&ORIG_POR_CIMA_DO_REAPROVEITAR))
                    .unwrap_or_else(|| panic!("ciclo {ciclo}: o hook de cima não empilhou"));
                assert_eq!(chamar(5), esperado + 185_000_000, "ciclo {ciclo}: a chamada não passou pelos dois hooks");
                assert!(remover_com_threads_paradas(alvo).0, "ciclo {ciclo}: o de cima não saiu");
                assert_eq!(chamar(5), esperado + 15_000_000, "ciclo {ciclo}: sem o de cima, o de baixo não responde");
                assert!(remover_com_threads_paradas(alvo).0, "ciclo {ciclo}: o de baixo não saiu");
                assert_eq!(chamar(5), esperado, "ciclo {ciclo}: o alvo não voltou ao corpo original");
                de_baixo.push(baixo as u64);
                de_cima.push(cima as u64);
            }
            for ciclo in 0..8 {
                let tramp = (BWMS_API.inline_hook_owned)(DONO, alvo, substituto_do_revert as *mut c_void);
                assert!(!tramp.is_null(), "ciclo {ciclo}: o hook do plugin não instalou");
                assert_eq!(chamar(5), 1_000_005, "ciclo {ciclo}: o hook do plugin não desviou a chamada");
                assert!((BWMS_API.inline_detach)(DONO, alvo), "ciclo {ciclo}: o detach do plugin foi recusado");
                do_dono.push(tramp as u64);
            }
        }
        assert_eq!(chamar(5), esperado, "o alvo não voltou ao corpo original");
        de_baixo.dedup();
        de_cima.dedup();
        assert_eq!(de_baixo.len(), 1, "o hook de baixo usou {} trampolins em 8 ciclos", de_baixo.len());
        assert_eq!(de_cima.len(), 1, "o hook de cima usou {} trampolins em 8 ciclos", de_cima.len());
        assert_ne!(de_baixo[0], de_cima[0], "bytes diferentes no alvo dividiram o trampolim");
        assert_ne!(de_baixo[0], TRAMPOLIM_FALSO.as_ptr() as u64, "a entrada com outro conteúdo foi usada");
        do_dono.sort_unstable();
        do_dono.dedup();
        assert_eq!(do_dono.len(), 8, "o inline_hook_owned repetiu trampolim");
        assert!(!do_dono.iter().any(|t| *t == de_baixo[0] || *t == de_cima[0]), "o inline_hook_owned usou trampolim da tabela");
        assert!(!TRAMPOLINS.lock().unwrap().iter().any(|e| do_dono.contains(&e.tramp)), "trampolim de dono entrou na tabela");
    }

    /// `remover` com a escrita aceita: devolve `true`, tira a entrada e o alvo volta ao corpo original. De novo, sem hook no alvo, devolve `true`
    /// (nada a desfazer). Nenhuma outra thread chama este alvo.
    #[test]
    fn remover_restaura_e_tira_a_entrada() {
        let chamar = std::hint::black_box(seguro(bwms_teste_alvo_do_remover));
        let esperado = chamar(5);
        let alvo = chamar as *mut c_void;
        // SAFETY: alvo e substituto são funções deste binário com a mesma ABI; o teste restaura o alvo antes de sair.
        let tramp = unsafe { instalar(alvo, substituto_do_revert as *mut c_void) };
        assert!(tramp.is_some(), "hook não instalou");
        assert_eq!(ganchos_no_endereco(alvo), 1);
        assert_eq!(chamar(5), 1_000_005, "o hook não desviou a chamada");
        // SAFETY: o alvo foi substituído por `instalar` acima.
        assert!(unsafe { remover(alvo) }, "escrita aceita tem que devolver restaurado");
        assert_eq!(ganchos_no_endereco(alvo), 0, "hook restaurado continua na lista");
        assert_eq!(chamar(5), esperado, "o alvo restaurado não voltou ao corpo original");
        // SAFETY: alvo sem hook; nada é escrito.
        assert!(unsafe { remover(alvo) }, "alvo sem hook tem que devolver nada a desfazer");
    }

    /// Acima de todo endereço de usuário do arm64 (no máximo 52 bits): o `mach_vm_protect` do `set_prot` recusa, que é o caminho da escrita que falha.
    const ALVO_FORA_DO_MAPA: u64 = 0x00FF_0000_0000_0000;

    /// Escrita recusada: o salto continuaria no alvo, então `remover` devolve `false`, a entrada fica e `remover_todos` para em vez de girar.
    #[test]
    fn remover_com_escrita_recusada_mantem_o_hook_e_remover_todos_termina() {
        let alvo = ALVO_FORA_DO_MAPA as *mut c_void;
        HOOKS.lock().unwrap().push(hk(ALVO_FORA_DO_MAPA, 4));
        // SAFETY: o kernel recusa o `set_prot` do alvo antes de qualquer cópia; nenhum byte é escrito.
        let restaurado = unsafe { remover(alvo) };
        let depois_do_remover = ganchos_no_endereco(alvo);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // SAFETY: idem; `remover_todos` só chama `remover`.
            unsafe { remover_todos(ALVO_FORA_DO_MAPA as *mut c_void) };
            let _ = tx.send(());
        });
        let terminou = rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok();
        let depois_do_remover_todos = ganchos_no_endereco(alvo);
        HOOKS.lock().unwrap().retain(|h| h.target != ALVO_FORA_DO_MAPA);
        assert!(!restaurado, "escrita recusada devolveu restaurado");
        assert_eq!(depois_do_remover, 1, "o salto continua no alvo e a entrada saiu da lista");
        assert!(terminou, "remover_todos girou com a escrita recusada");
        assert_eq!(depois_do_remover_todos, 1, "remover_todos tirou a entrada sem restaurar o alvo");
    }

    // Reconstrói o imm64 de uma sequência movz+3×movk.
    fn decode_load_imm64(code: &[u8]) -> u64 {
        let mut v = 0u64;
        for i in 0..4 {
            let insn = u32::from_le_bytes([code[i * 4], code[i * 4 + 1], code[i * 4 + 2], code[i * 4 + 3]]);
            let hw = ((insn >> 21) & 0x3) as u64;
            let imm16 = ((insn >> 5) & 0xFFFF) as u64;
            v |= imm16 << (16 * hw);
        }
        v
    }

    fn mk_buf(insns: [u32; 4]) -> [u8; 16] {
        let mut b = [0u8; 16];
        for i in 0..4 {
            b[i * 4..i * 4 + 4].copy_from_slice(&insns[i].to_le_bytes());
        }
        b
    }

    #[test]
    fn adrp_vira_absoluto() {
        // adrp x0, #0x1000 (imm=1) ; ret ; ret ; ret
        let imm: i64 = 1;
        let immlo = (imm & 0x3) as u32;
        let immhi = ((imm >> 2) & 0x7FFFF) as u32;
        let adrp = 0x9000_0000u32 | (immlo << 29) | (immhi << 5); // Rd=0
        let ret = 0xD65F_03C0u32;
        let buf = mk_buf([adrp, ret, ret, ret]);
        let a = buf.as_ptr() as u64;
        let out = unsafe { relocate_prologue(a) }.expect("adrp deve relocar");
        let expected = (a & !0xFFF).wrapping_add((imm as u64) << 12);
        assert_eq!(decode_load_imm64(&out[0..16]), expected, "adrp -> resultado absoluto em x0");
        assert_eq!(&out[16..28], &buf[4..16], "rets seguem verbatim após o movz/movk");
    }

    #[test]
    fn adr_vira_absoluto() {
        // adr x3, #4 (imm=4) ; nop*3
        let imm: i64 = 4;
        let immlo = (imm & 0x3) as u32;
        let immhi = ((imm >> 2) & 0x7FFFF) as u32;
        let adr = 0x1000_0000u32 | (immlo << 29) | (immhi << 5) | 3; // ADR (op=0), Rd=3
        let nop = 0xD503_201Fu32;
        let buf = mk_buf([adr, nop, nop, nop]);
        let a = buf.as_ptr() as u64;
        let out = unsafe { relocate_prologue(a) }.expect("adr deve relocar");
        assert_eq!(decode_load_imm64(&out[0..16]), (a as i64 + imm) as u64, "adr -> PC+imm absoluto em x3");
    }

    #[test]
    fn prologo_simples_verbatim() {
        let stp = 0xA9BF_7BFDu32; // stp x29,x30,[sp,#-0x10]!
        let sub = 0xD100_43FFu32; // sub sp,sp,#0x10
        let buf = mk_buf([stp, sub, stp, sub]);
        let out = unsafe { relocate_prologue(buf.as_ptr() as u64) }.expect("simples deve passar");
        assert_eq!(&out[..], &buf[..], "prólogo sem PC-relativo = verbatim (caso do executor)");
    }

    #[test]
    fn b_vira_absjump() {
        let b = 0x1400_0001u32; // b #4
        let ret = 0xD65F_03C0u32;
        let buf = mk_buf([b, ret, ret, ret]);
        let a = buf.as_ptr() as u64;
        let out = unsafe { relocate_prologue(a) }.expect("B deve relocar");
        assert_eq!(&out[0..4], &0x5800_0051u32.to_le_bytes(), "ldr x17,#8");
        assert_eq!(&out[4..8], &0xD61F_0220u32.to_le_bytes(), "br x17");
        assert_eq!(&out[8..16], &(a + 4).to_le_bytes(), "alvo absoluto do B");
    }

    #[test]
    fn cbz_inverte_e_absjump() {
        let cbz = 0x3400_0040u32; // cbz x0, #8
        let ret = 0xD65F_03C0u32;
        let buf = mk_buf([cbz, ret, ret, ret]);
        let a = buf.as_ptr() as u64;
        let out = unsafe { relocate_prologue(a) }.expect("CBZ deve relocar");
        assert_eq!(&out[0..4], &0x3500_00A0u32.to_le_bytes(), "cbz->cbnz x0, #20 (pula o abs-jump)");
        assert_eq!(&out[12..20], &(a + 8).to_le_bytes(), "alvo absoluto do abs-jump");
    }

    #[test]
    fn ldr_literal_materializa() {
        // Achado 2026-07-16 (ao vivo, hook empilhado): materializar o ENDEREÇO e reler em runtime
        // quebra quando o "literal" é sobrescrito depois (ex.: outro hook no mesmo alvo) — agora
        // lê o VALOR uma vez, na relocação, e materializa ele direto (snapshot imune a isso).
        let ldrlit = 0x5800_0040u32; // ldr x0, #8
        let ret = 0xD65F_03C0u32;
        let buf = mk_buf([ldrlit, ret, ret, ret]);
        let a = buf.as_ptr() as u64;
        let out = unsafe { relocate_prologue(a) }.expect("ldr-literal deve relocar");
        // valor no "literal" (buf+8..+16) = os 8 bytes de [ret,ret] concatenados (LE) — o mesmo
        // padrão de bytes que o abs_jump embute (2 instruções de dado após ldr+br).
        let mut lit = [0u8; 8];
        lit[0..4].copy_from_slice(&ret.to_le_bytes());
        lit[4..8].copy_from_slice(&ret.to_le_bytes());
        assert_eq!(decode_load_imm64(&out[0..16]), u64::from_le_bytes(lit), "x0 = VALOR do literal (não o endereço)");
    }

    #[test]
    fn bl_vira_blr_absoluto() {
        let bl = 0x9400_0001u32; // bl #4
        let ret = 0xD65F_03C0u32;
        let buf = mk_buf([bl, ret, ret, ret]);
        let a = buf.as_ptr() as u64;
        let out = unsafe { relocate_prologue(a) }.expect("bl deve relocar (materializa+blr)");
        assert_eq!(decode_load_imm64(&out[0..16]), a + 4, "x17 = alvo absoluto do bl");
        assert_eq!(&out[16..20], &0xD63F_0220u32.to_le_bytes(), "blr x17");
    }

    #[test]
    fn ldr_literal_32bit_materializa() {
        let ldrlit = 0x1800_0040u32; // ldr w0, #8
        let ret = 0xD65F_03C0u32;
        let buf = mk_buf([ldrlit, ret, ret, ret]);
        let a = buf.as_ptr() as u64;
        let out = unsafe { relocate_prologue(a) }.expect("ldr-literal 32-bit deve relocar");
        // valor no "literal" = só os 4 bytes em buf+8 (1º `ret`), ZERO-estendido (ldr Wt).
        assert_eq!(decode_load_imm64(&out[0..16]), ret as u64, "w0 zero-estendido = VALOR do literal (não o endereço)");
    }

    #[test]
    fn ldrsw_literal_materializa() {
        let ldrsw = 0x9800_0040u32; // ldrsw x0, #8
        let ret = 0xD65F_03C0u32;
        let buf = mk_buf([ldrsw, ret, ret, ret]);
        let a = buf.as_ptr() as u64;
        let out = unsafe { relocate_prologue(a) }.expect("ldrsw-literal deve relocar");
        // valor no "literal" = 4 bytes em buf+8 (1º `ret`, bit alto setado), SINAL-estendido.
        let expected = (ret as i32) as i64 as u64;
        assert_eq!(decode_load_imm64(&out[0..16]), expected, "x0 sinal-estendido = VALOR do literal (não o endereço)");
    }

    #[test]
    fn prfm_literal_vira_nop() {
        let prfm = 0xD800_0040u32; // prfm pldl1keep, #8
        let ret = 0xD65F_03C0u32;
        let buf = mk_buf([prfm, ret, ret, ret]);
        let out = unsafe { relocate_prologue(buf.as_ptr() as u64) }.expect("prfm-literal deve relocar (vira nop)");
        assert_eq!(&out[0..4], &0xD503_201Fu32.to_le_bytes(), "prfm -> nop");
        assert_eq!(&out[4..16], &mk_buf([0, ret, ret, ret])[4..16], "resto segue verbatim");
    }

    #[test]
    fn ldr_literal_simd_ainda_recusa() {
        // ldr s0, #8 (0x1C, SIMD/FP — registrador de destino não é GP, ainda não tratado)
        let ldrsimd = 0x1C00_0040u32;
        let ret = 0xD65F_03C0u32;
        let buf = mk_buf([ldrsimd, ret, ret, ret]);
        assert!(unsafe { relocate_prologue(buf.as_ptr() as u64) }.is_none(), "LDR SIMD/FP literal -> ainda recusa (fora de escopo)");
    }

    #[test]
    fn adrp_imm_negativo() {
        // adrp x0, #-0x1000 (imm=-1): exercita a extensão de sinal de 21 bits.
        let imm: i64 = -1;
        let immlo = (imm & 0x3) as u32;
        let immhi = ((imm >> 2) & 0x7FFFF) as u32;
        let adrp = 0x9000_0000u32 | (immlo << 29) | (immhi << 5); // Rd=0
        let nop = 0xD503_201Fu32;
        let buf = mk_buf([adrp, nop, nop, nop]);
        let a = buf.as_ptr() as u64;
        let out = unsafe { relocate_prologue(a) }.expect("adrp neg deve relocar");
        let expected = ((a & !0xFFF) as i64 + (imm << 12)) as u64;
        assert_eq!(decode_load_imm64(&out[0..16]), expected, "adrp imm negativo -> absoluto (sign-ext OK)");
    }

    #[test]
    fn b_near_codifica_e_recusa() {
        // pra frente: off=+0x10 -> imm26=4 -> 0x14000004
        assert_eq!(emit_b_near(0x1000, 0x1010), Some(0x1400_0004u32.to_le_bytes()));
        // pra trás: off=-0x10 -> imm26 em c2 -> 0x17FFFFFC
        assert_eq!(emit_b_near(0x1010, 0x1000), Some(0x17FF_FFFCu32.to_le_bytes()));
        // limite: exatamente +128MB (2^27) está FORA -> None
        assert_eq!(emit_b_near(0x1000, 0x1000 + (1 << 27)), None);
        // dentro do alcance perto do limite: 2^27-4 -> Some
        assert!(emit_b_near(0x1000, 0x1000 + (1 << 27) - 4).is_some());
        // desalinhado -> None
        assert_eq!(emit_b_near(0x1000, 0x1003), None);
    }

    #[test]
    fn adrp_br_codifica_e_recusa() {
        // exige `to` alinhado a pagina (4096); nao-alinhado -> None
        assert_eq!(emit_adrp_br(0x1_0000_0000, 0x1_0000_0001, 17), None);
        // gera 8 bytes: ADRP seguido de BR (offset de ~1.5GB, bem dentro de +-4GB)
        let out = emit_adrp_br(0x1_0000_0000, 0x1_6000_0000, 17).expect("dentro de +-4GB deve codificar");
        assert_eq!(out.len(), 8);
        let adrp = u32::from_le_bytes(out[0..4].try_into().unwrap());
        let br = u32::from_le_bytes(out[4..8].try_into().unwrap());
        // decodifica o ADRP (mesmo padrao de bits usado em relocate_prologue/decode_load_imm64)
        assert_eq!(adrp & 0x1F, 17, "Rd do ADRP deve ser X17");
        let is_adrp = (adrp >> 31) & 1 == 1;
        assert!(is_adrp, "op=1 (ADRP, nao ADR)");
        let immlo = (adrp >> 29) & 0x3;
        let immhi = (adrp >> 5) & 0x7_FFFF;
        let mut imm = ((immhi << 2) | immlo) as i64;
        if imm & (1 << 20) != 0 {
            imm |= !0x1F_FFFFi64; // sign-extend 21 bits
        }
        let from_page: i64 = 0x1_0000_0000i64 & !0xFFF;
        let computed = (from_page + (imm << 12)) as u64;
        assert_eq!(computed, 0x1_6000_0000, "ADRP decodificado deve apontar pra pagina certa");
        // BR Xn: 0xD61F0000 | (Rn<<5)
        assert_eq!(br, 0xD61F_0000u32 | (17 << 5), "BR X17");
        // fora de +-4GB -> None
        assert_eq!(emit_adrp_br(0x1_0000_0000, 0x1_0000_0000 + (1u64 << 32), 17), None);
        // dentro, perto do limite -> Some
        assert!(emit_adrp_br(0x1_0000_0000, (0x1_0000_0000i64 + (1i64 << 32) - 0x1000) as u64, 17).is_some());
    }

    // Relocar 1 instrução não-PC-relativa (caso do getter `ldrsb w0,[x0,#0x84]`) = verbatim, 4 bytes.
    #[test]
    fn relocate_n1_verbatim() {
        let ldrsb = 0x39C2_1000u32; // ldrsb w0,[x0,#0x84]
        let ret = 0xD65F_03C0u32;
        let buf = mk_buf([ldrsb, ret, ret, ret]);
        let out = unsafe { relocate_n(buf.as_ptr() as u64, 1) }.expect("1 instr deve relocar");
        assert_eq!(out.len(), 4, "n=1 rouba só 4 bytes");
        assert_eq!(&out[0..4], &ldrsb.to_le_bytes(), "não-PC-relativo = verbatim");
    }

    /// REGRESSÃO (2026-07-16, achado empilhando 2 hooks no mesmo alvo ao vivo p/
    /// `red4ext-attach-detach-contract`): relocar o PRÓPRIO encoding do `abs_jump` (o patch que
    /// `ganchos::instalar` escreve no alvo) — o cenário de um hook capturando o que OUTRO hook
    /// já escreveu ali (hook empilhado). O `abs_jump` é `ldr x17,#8; br x17; <8B: endereço>` — o
    /// mesmo LDR-literal 64-bit que qualquer prólogo real pode ter. Prova que o valor relocado
    /// (materializado em Xt) é um SNAPSHOT: sobrescrever o buffer ORIGINAL depois (simulando um
    /// 2º patch no mesmo endereço) NÃO muda o valor já materializado no trampolim. Antes do fix
    /// (materializava o ENDEREÇO + `ldr [Xscratch]` em RUNTIME), isto quebrava: reler em runtime
    /// pegava o valor NOVO (sobrescrito), não o antigo — causava loop infinito ao vivo (o
    /// trampolim do hook mais novo saltava de volta pra si mesmo).
    #[test]
    fn ldr_literal_sobrevive_a_sobrescrita_do_alvo_original() {
        let a_addr = 0x1_1000_0000u64;
        let b_addr = 0x1_2000_0000u64;
        let mut buf = mk_buf([0, 0, 0, 0]);
        // escreve abs_jump(a_addr) no buffer (mimetiza hook1 patchando o alvo).
        let patch_a = abs_jump(a_addr);
        buf.copy_from_slice(&patch_a);
        let addr = buf.as_ptr() as u64;

        // hook2 captura+reloca o que está lá AGORA (abs_jump pro a_addr).
        let out = unsafe { relocate_prologue(addr) }.expect("abs_jump(a) deve relocar (é so um LDR-lit + BR)");
        assert_eq!(decode_load_imm64(&out[0..16]), a_addr, "materializou o VALOR (a_addr), não o endereço do literal");

        // AGORA sobrescreve o buffer com abs_jump(b_addr) — mimetiza hook2 patchando o MESMO
        // endereço com seu PRÓPRIO alvo (b_addr), exatamente como `ganchos::instalar` faz.
        let patch_b = abs_jump(b_addr);
        buf.copy_from_slice(&patch_b);

        // o trampolim JÁ CONSTRUÍDO (out) não deve ter mudado — é um snapshot, imune à
        // sobrescrita. Se isto falhar (valor virou b_addr), a bug do loop infinito voltou.
        assert_eq!(
            decode_load_imm64(&out[0..16]),
            a_addr,
            "trampolim já construído deve continuar apontando pro valor ORIGINAL (a_addr), mesmo após o buffer real ser sobrescrito com b_addr"
        );
    }

    /// Codeware `#118`: só um PC dentro dos 16 bytes do patch impede a restauração; a instrução seguinte e a anterior não.
    #[test]
    fn pc_no_trecho_do_patch() {
        let alvo = 0x1_019b_95f4;
        assert!(super::pc_no_trecho(alvo, alvo, 16));
        assert!(super::pc_no_trecho(alvo + 12, alvo, 16));
        assert!(!super::pc_no_trecho(alvo + 16, alvo, 16));
        assert!(!super::pc_no_trecho(alvo - 4, alvo, 16));
    }
}

/// O PC de uma thread parada cai nos `tamanho` bytes do patch em `alvo`?
fn pc_no_trecho(pc: u64, alvo: u64, tamanho: u64) -> bool {
    pc >= alvo && pc < alvo + tamanho
}

/// Solta os direitos das threads devolvidas pelo `task_threads` e o vetor delas.
unsafe fn liberar_threads(threads: &[MachPort], lista: *mut MachPort, quantas: u32) {
    for &th in threads {
        mach_port_deallocate(mach_task_self_, th);
    }
    mach_vm_deallocate(mach_task_self_, lista as u64, quantas as u64 * 4);
}

/// Codeware `#118`: roda `escrever` (os 16 bytes de `t` e a lista `HOOKS`) com as outras threads do processo suspensas e nenhum PC dentro dos 16 bytes
/// — uma thread entre as instruções do salto executaria instrução misturada, e a página do alvo fica sem `X` durante a escrita. Com algum PC no trecho,
/// retoma todas e tenta de novo (até 200 vezes, 1 ms entre elas). Com as threads paradas não há alocação, log nem trava nova: a lista de threads, o
/// vetor, a trava de `HOOKS` e 1 posição livre nela vêm antes. `pode` roda com a trava antes de suspender; falso encerra sem escrever. Devolve
/// (escrito, tentativas com PC no trecho).
unsafe fn escrever_com_threads_paradas(t: u64, pode: impl Fn(&[Hook]) -> bool, mut escrever: impl FnMut(&mut Vec<Hook>) -> bool) -> (bool, u32) {
    let eu = mach_thread_self();
    let (mut escrito, mut ocupadas) = (false, 0u32);
    for _ in 0..200 {
        let mut lista: *mut MachPort = std::ptr::null_mut();
        let mut quantas: u32 = 0;
        if task_threads(mach_task_self_, &mut lista, &mut quantas) != KERN_SUCCESS {
            break;
        }
        let threads = std::slice::from_raw_parts(lista, quantas as usize);
        let mut paradas: Vec<MachPort> = Vec::with_capacity(threads.len());
        let mut hooks = HOOKS.lock().unwrap();
        if !pode(&hooks) {
            drop(hooks);
            liberar_threads(threads, lista, quantas);
            break;
        }
        hooks.reserve(1);
        for &th in threads {
            if th != eu && thread_suspend(th) == KERN_SUCCESS {
                paradas.push(th);
            }
        }
        // Thread criada depois da lista não foi suspensa e pode estar rodando código da página do alvo, que fica sem `X` durante a escrita: com a lista
        // mudada, retoma e tenta de novo.
        let estavel = {
            let mut lista2: *mut MachPort = std::ptr::null_mut();
            let mut quantas2: u32 = 0;
            if task_threads(mach_task_self_, &mut lista2, &mut quantas2) == KERN_SUCCESS {
                let threads2 = std::slice::from_raw_parts(lista2, quantas2 as usize);
                let mesma = quantas2 == quantas && threads2.iter().all(|th| threads.contains(th));
                liberar_threads(threads2, lista2, quantas2);
                mesma
            } else {
                false
            }
        };
        let livre = estavel && paradas.iter().all(|&th| {
            let mut estado = [0u32; ARM_THREAD_STATE64_PALAVRAS as usize];
            let mut palavras = ARM_THREAD_STATE64_PALAVRAS;
            // Sem estado não dá pra garantir o PC: conta como ocupada.
            thread_get_state(th, ARM_THREAD_STATE64, estado.as_mut_ptr(), &mut palavras) == KERN_SUCCESS
                && !pc_no_trecho(estado[64] as u64 | (estado[65] as u64) << 32, t, 16)
        });
        if livre {
            escrito = escrever(&mut hooks);
        }
        for &th in &paradas {
            thread_resume(th);
        }
        drop(hooks);
        liberar_threads(threads, lista, quantas);
        if livre {
            break;
        }
        ocupadas += 1;
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    mach_port_deallocate(mach_task_self_, eu);
    (escrito, ocupadas)
}

/// [`remover_parado`] sem conferir de quem é o hook do topo.
///
/// # Safety
/// Mesmos requisitos de [`remover_parado`].
pub unsafe fn remover_com_threads_paradas(target: *mut c_void) -> (bool, u32) {
    remover_parado(target, None)
}

/// Codeware `#118`: desfaz o hook mais recente de `target` com as outras threads suspensas ([`escrever_com_threads_paradas`]). Com `so_topo`, só desfaz
/// se o hook do topo é o que devolveu esse trampolim; a conferência roda com a trava de `HOOKS` na mão, a mesma da escrita, então outro hook empilhado
/// no meio tempo não é tirado no lugar dele. O trampolim continua mapeado, e uma thread que já entrou nele volta pro alvo+16, que a restauração não
/// muda. Devolve (restaurado, tentativas com PC no trecho).
///
/// # Safety
/// `target` precisa ser um alvo com hook posto por este módulo ([`instalar_com_threads_paradas`], [`instalar_perto`], [`instalar_com_ponte`] ou
/// [`trocar_instrucao`]): a restauração escreve os 16 bytes guardados na instalação.
pub(crate) unsafe fn remover_parado(target: *mut c_void, so_topo: Option<u64>) -> (bool, u32) {
    let t = target as u64;
    // Boot 178: o jogo parou logo depois do registro dos ouvintes do teste 118, sem veredito. A linha antes e a depois de cada remoção dizem em qual
    // alvo ela não voltou.
    #[cfg(not(test))]
    let inicio = std::time::Instant::now();
    #[cfg(not(test))]
    crate::veredito_dev!("[ganchos] tirando o hook de {t:#x} com as threads paradas");
    let topo = |hooks: &[Hook]| latest_index_for(hooks, t).filter(|&p| so_topo.is_none_or(|tr| hooks[p].tramp == tr));
    let (restaurado, ocupadas) = escrever_com_threads_paradas(
        t,
        |hooks| topo(hooks).is_some(),
        |hooks| {
            let Some(pos) = topo(hooks) else { return false };
            if !set_prot(t, VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY) {
                return false;
            }
            std::ptr::copy_nonoverlapping(hooks[pos].orig.as_ptr(), target as *mut u8, 16);
            set_prot(t, VM_PROT_READ | VM_PROT_EXECUTE);
            sys_icache_invalidate(target, 16);
            hooks.remove(pos);
            true
        },
    );
    #[cfg(not(test))]
    crate::veredito_dev!(
        "[ganchos] hook de {t:#x}: restaurado {restaurado}, {ocupadas} tentativa(s) com PC no trecho, {} ms",
        inicio.elapsed().as_millis()
    );
    (restaurado, ocupadas)
}

/// Codeware `#118`, boots 178 e 181: com o salto escrito com as outras threads rodando código da página do alvo, o jogo parou 2 vezes na instalação
/// dos hooks de evento (logo depois do 1º registro no 178, antes do 19º no 181), e nenhuma das 180 remoções com as threads paradas travou. A escrita
/// única dos 3 instaladores ([`instalar_com_threads_paradas`], [`instalar_perto`] e [`instalar_com_ponte`]): põe `salto` no começo de `t` como a
/// remoção ([`escrever_com_threads_paradas`]) e empilha o hook. Com a trava de `HOOKS` na mão, que todo escritor de código deste módulo pega, recusa
/// `repl` já empilhado em `t` ([`substituto_no_alvo`]) e alvo com bytes diferentes de `orig`, a cópia que o instalador fez antes de relocar: bytes
/// iguais a ela são os que o trampolim relocou. `destino` recebe o trampolim com as threads paradas, antes do salto: a 1ª chamada desviada já o
/// encontra, e uma recusa não o toca. Devolve se o salto foi escrito.
unsafe fn escrever_salto(t: u64, salto: &[u8], orig: [u8; 16], tramp: u64, repl: u64, destino: Option<&std::sync::atomic::AtomicU64>) -> bool {
    #[cfg(not(test))]
    let inicio = std::time::Instant::now();
    #[cfg(not(test))]
    crate::veredito_dev!("[ganchos] pondo o hook em {t:#x} com as threads paradas");
    let (posto, _ocupadas) = escrever_com_threads_paradas(
        t,
        |hooks| !substituto_no_alvo(hooks, t, repl) && (t as *const [u8; 16]).read_unaligned() == orig,
        |hooks| {
            if !set_prot(t, VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY) {
                return false;
            }
            if let Some(d) = destino {
                d.store(tramp, Ordering::SeqCst);
            }
            std::ptr::copy_nonoverlapping(salto.as_ptr(), t as *mut u8, salto.len());
            set_prot(t, VM_PROT_READ | VM_PROT_EXECUTE);
            sys_icache_invalidate(t as *mut c_void, salto.len());
            hooks.push(Hook { target: t, orig, tramp, repl });
            true
        },
    );
    #[cfg(not(test))]
    crate::veredito_dev!(
        "[ganchos] hook em {t:#x}: posto {posto}, {_ocupadas} tentativa(s) com PC no trecho, {} ms",
        inicio.elapsed().as_millis()
    );
    posto
}

/// Trampolim de [`instalar_com_threads_paradas`]: relocou os 16 bytes `orig` de `alvo` e salta de volta pro `alvo`+16. Nunca sai da tabela nem do
/// mapa: uma thread pode estar dentro dele depois da remoção do hook.
struct Trampolim {
    alvo: u64,
    orig: [u8; 16],
    tramp: u64,
}

/// Codeware `#118`: cada ciclo de registrar e tirar o último ouvinte reinstala até 11 hooks de evento. A reinstalação com os mesmos bytes no alvo e o
/// mesmo conteúdo relocado usa o trampolim da anterior, sem `mmap`: o custo fica em 1 página por alvo no processo, por mais ciclos que haja.
static TRAMPOLINS: Mutex<Vec<Trampolim>> = Mutex::new(Vec::new());

/// Página nova com `reloc` e o salto de volta pra `volta`; `None` com o `mmap` recusado.
unsafe fn trampolim_novo(reloc: &[u8], volta: u64) -> Option<u64> {
    let tramp = mmap(std::ptr::null_mut(), 4096, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANON | MAP_JIT, -1, 0);
    if tramp.is_null() || tramp as isize == -1 {
        return None;
    }
    let rl = reloc.len();
    let back = abs_jump(volta);
    pthread_jit_write_protect_np(0);
    std::ptr::copy_nonoverlapping(reloc.as_ptr(), tramp as *mut u8, rl);
    std::ptr::copy_nonoverlapping(back.as_ptr(), (tramp as *mut u8).add(rl), 16);
    pthread_jit_write_protect_np(1);
    sys_icache_invalidate(tramp, rl + 16);
    Some(tramp as u64)
}

/// O trampolim de `t` em [`TRAMPOLINS`] que relocou `orig` e contém `reloc` seguido da volta pro `t`+16, ou um novo, que entra na tabela. O conteúdo é
/// conferido byte a byte: um literal lido pelo prólogo que mudou de valor dá relocação diferente com os mesmos 16 bytes no alvo.
unsafe fn trampolim_do_alvo(t: u64, orig: [u8; 16], reloc: &[u8]) -> Option<u64> {
    let back = abs_jump(t + 16);
    let mut tabela = TRAMPOLINS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let guardado = tabela.iter().filter(|e| e.alvo == t && e.orig == orig).find(|e| {
        // Toda entrada é uma página de 4096 bytes ou mais, e 4 instruções relocadas mais a volta ocupam no máximo 96.
        let feito = std::slice::from_raw_parts(e.tramp as *const u8, reloc.len() + 16);
        feito[..reloc.len()] == *reloc && feito[reloc.len()..] == back
    });
    if let Some(e) = guardado {
        return Some(e.tramp);
    }
    let tramp = trampolim_novo(reloc, t + 16)?;
    tabela.push(Trampolim { alvo: t, orig, tramp });
    Some(tramp)
}

/// Hook de 16 bytes: usa o trampolim do alvo ([`TRAMPOLINS`]) ou monta um (prólogo relocado e salto pro alvo+16) e escreve o salto pra `replacement`
/// com as threads paradas ([`escrever_salto`], que diz o que é recusado e quando `destino` recebe o trampolim). Devolve o trampolim, ou `None` sem patch.
///
/// # Safety
/// Mesmos requisitos de [`instalar`].
pub unsafe fn instalar_com_threads_paradas(
    target: *mut c_void,
    replacement: *mut c_void,
    destino: Option<&std::sync::atomic::AtomicU64>,
) -> Option<*mut c_void> {
    instalar_salto_de_16(target, replacement, destino, true)
}

/// [`instalar_com_threads_paradas`] sem `destino` e sempre com página nova, fora de [`TRAMPOLINS`]. O `inline_hook_owned` de plugin identifica o hook
/// pelo trampolim ([`remover_parado`] com `so_topo`), e o dono de um hook tirado por `inline_revert` continua com o valor: um trampolim reaproveitado
/// faria o `inline_detach` dele tirar a instalação seguinte no alvo.
///
/// # Safety
/// Mesmos requisitos de [`instalar`].
pub(crate) unsafe fn instalar_com_trampolim_novo(target: *mut c_void, replacement: *mut c_void) -> Option<*mut c_void> {
    instalar_salto_de_16(target, replacement, None, false)
}

/// Corpo de [`instalar_com_threads_paradas`] (`reaproveitar`: o trampolim vem de [`trampolim_do_alvo`]) e de [`instalar_com_trampolim_novo`].
unsafe fn instalar_salto_de_16(
    target: *mut c_void,
    replacement: *mut c_void,
    destino: Option<&std::sync::atomic::AtomicU64>,
    reaproveitar: bool,
) -> Option<*mut c_void> {
    let (t, r) = (target as u64, replacement as u64);
    let orig = (target as *const [u8; 16]).read_unaligned();
    let reloc = relocate_prologue(t)?;
    let tramp = if reaproveitar { trampolim_do_alvo(t, orig, &reloc)? } else { trampolim_novo(&reloc, t + 16)? };
    escrever_salto(t, &abs_jump(r), orig, tramp, r, destino).then_some(tramp as *mut c_void)
}

/// Troca uma instrução (4 bytes) do `__TEXT` com as outras threads paradas, só se a atual é `esperada` — patch de uma instrução (desligar um
/// desvio), não hook. A lista de hooks guarda os 16 bytes de antes, então o [`remover_com_threads_paradas`] desfaz.
///
/// # Safety
/// `alvo` precisa ser uma instrução do código do jogo, já rebaseada.
pub unsafe fn trocar_instrucao(alvo: *mut c_void, esperada: u32, nova: u32) -> bool {
    let t = alvo as u64;
    if !is_readable(alvo, 16) || (alvo as *const u32).read_unaligned() != esperada {
        return false;
    }
    let mut orig = [0u8; 16];
    std::ptr::copy_nonoverlapping(alvo as *const u8, orig.as_mut_ptr(), 16);
    escrever_com_threads_paradas(
        t,
        |_| true,
        |hooks| {
            if !set_prot(t, VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY) {
                return false;
            }
            (alvo as *mut u32).write_unaligned(nova);
            set_prot(t, VM_PROT_READ | VM_PROT_EXECUTE);
            sys_icache_invalidate(alvo, 4);
            hooks.push(Hook { target: t, orig, tramp: 0, repl: 0 });
            true
        },
    )
    .0
}

/// Substitui `target` por `replacement` (uma `extern "C" fn`) com as threads paradas ([`instalar_com_threads_paradas`] sem `destino`) e devolve o
/// trampolim chamável da função original, ou `None` se falhar. Substituto que lê o original de um static passa esse static como `destino` do
/// [`instalar_com_threads_paradas`]: gravado por quem chamou depois deste retorno, uma chamada que chegue antes do `store` lê 0.
///
/// # Safety
/// `target` precisa apontar pra função real já rebaseada com o slide;
/// `replacement` precisa ter ABI compatível com a original.
pub(crate) unsafe fn instalar(target: *mut c_void, replacement: *mut c_void) -> Option<*mut c_void> {
    instalar_com_threads_paradas(target, replacement, None)
}

/// Como `instalar`, mas p/ FUNÇÕES PEQUENAS (leaf de 8B etc.): escreve só um `B` de 4 bytes
/// no alvo (rouba 1 instrução) em vez do abs-jump de 16B → **NÃO transborda a função vizinha**
/// (a causa do SIGILL @0x3f5ec7c ao hookar o getter da phase-byte). Exige `replacement` em
/// ±128MB do alvo (alcance do `B`); se longe, tenta um landing-pad JIT perto do alvo (2 saltos,
/// alcance irrestrito no 2º) antes de recusar — o slide do dylib vs. o jogo pode empurrar o getter
/// da phase byte pra fora de ±128MB (no Steam 2.5.0, 12 de 15 boots; aí o `instalar_com_ponte`
/// assume). Só recusa (`None`) se nem o pad couber em lugar nenhum. O `B` é escrito com as threads paradas ([`escrever_salto`], que
/// diz o que é recusado e quando `destino` recebe o trampolim).
///
/// # Safety
/// Mesmos requisitos de `instalar`. O alvo deve ter ≥4 bytes de prólogo não-BL.
pub(crate) unsafe fn instalar_perto(
    target: *mut c_void,
    replacement: *mut c_void,
    destino: Option<&std::sync::atomic::AtomicU64>,
) -> Option<*mut c_void> {
    let t = target as u64;
    // GUARD: B só alcança ±128MB. Longe → tenta um landing-pad perto do alvo antes de recusar.
    let b = match emit_b_near(t, replacement as u64) {
        Some(b) => b,
        None => {
            let pad = alloc_near_landing_pad(t, replacement as u64)?;
            crate::log_dev!("[ganchos] instalar_perto: fora de alcance direto, landing-pad @ {pad:#x}");
            emit_b_near(t, pad)?
        }
    };
    let orig = (target as *const [u8; 16]).read_unaligned();
    // trampolim do original: 1ª instrução roubada (relocada) + abs-jump pro alvo+4.
    let reloc = relocate_n(t, 1)?;
    let tramp = mmap(std::ptr::null_mut(), 4096, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANON | MAP_JIT, -1, 0);
    if tramp.is_null() || tramp as isize == -1 {
        return None;
    }
    let back = abs_jump(t + 4);
    let rl = reloc.len();
    pthread_jit_write_protect_np(0);
    std::ptr::copy_nonoverlapping(reloc.as_ptr(), tramp as *mut u8, rl);
    std::ptr::copy_nonoverlapping(back.as_ptr(), (tramp as *mut u8).add(rl), 16);
    pthread_jit_write_protect_np(1);
    sys_icache_invalidate(tramp, rl + 16);
    // patch: SÓ 4 bytes no alvo (a vizinha fica intacta).
    escrever_salto(t, &b, orig, tramp as u64, replacement as u64, destino).then_some(tramp)
}

/// Como `instalar_perto`, mas usa `ADRP+BR` (8 bytes, ±4GB de alcance) em vez de `B` (4 bytes,
/// ±128MB) — rouba as DUAS instruções do getter de 8B (em vez de só a 1ª) mas ainda cabe
/// exatamente sem transbordar a vizinha. É o caminho do getter da phase byte na maioria dos boots do
/// Steam 2.5.0 (12 de 15 consoles de 2026-10-18 a 10-23): o landing-pad de
/// `instalar_perto` precisa de um endereço DENTRO de ±128MB do alvo, e o macOS recusa alocar
/// memória nova nessa vizinhança (reserva de kernel em torno do Mach-O do jogo). Com ADRP
/// (±4GB) o orçamento é 32x maior — um `mmap` NORMAL (sem `MAP_FIXED`, sempre bem-sucedido,
/// em QUALQUER endereço que o kernel escolher) cai folgadamente dentro de ±4GB de qualquer
/// alvo no mesmo processo, sem precisar achar um endereço específico livre. O `ADRP+BR` é escrito com as threads paradas ([`escrever_salto`], que
/// diz o que é recusado e quando `destino` recebe o trampolim).
///
/// # Safety
/// Mesmos requisitos de `instalar`. O alvo deve ter EXATAMENTE 8 bytes de prólogo (2
/// instruções) não-PC-relativas e não-BL (ambas relocáveis verbatim).
pub(crate) unsafe fn instalar_com_ponte(
    target: *mut c_void,
    replacement: *mut c_void,
    destino: Option<&std::sync::atomic::AtomicU64>,
) -> Option<*mut c_void> {
    let t = target as u64;
    // landing pad: mmap comum (sem FIXED) -- sempre sucede, endereço page-aligned de graça.
    const PAGE: usize = 16 * 1024;
    let pad = mmap(std::ptr::null_mut(), PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    if pad.is_null() || pad as isize == -1 {
        return None;
    }
    let pad_addr = pad as u64;
    debug_assert_eq!(pad_addr & 0xFFF, 0, "mmap sempre devolve endereço alinhado a página");
    let b = match emit_adrp_br(t, pad_addr, 17) {
        Some(b) => b,
        None => {
            let _ = munmap(pad, PAGE);
            crate::log_dev!("[ganchos] instalar_com_ponte: landing-pad @ {pad_addr:#x} fora de ±4GB do alvo (inesperado)");
            return None;
        }
    };
    let stub = abs_jump(replacement as u64);
    std::ptr::copy_nonoverlapping(stub.as_ptr(), pad as *mut u8, stub.len());
    if mach_vm_protect(mach_task_self_, pad_addr, PAGE as u64, 0, VM_PROT_READ | VM_PROT_EXECUTE) != KERN_SUCCESS {
        let _ = munmap(pad, PAGE);
        return None;
    }
    sys_icache_invalidate(pad, stub.len());

    let orig = (target as *const [u8; 16]).read_unaligned();
    // trampolim do original: as 2 instruções roubadas (relocadas) + abs-jump pro alvo+8.
    let reloc = relocate_n(t, 2)?;
    let tramp = mmap(std::ptr::null_mut(), 4096, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANON | MAP_JIT, -1, 0);
    if tramp.is_null() || tramp as isize == -1 {
        return None;
    }
    let back = abs_jump(t + 8);
    let rl = reloc.len();
    pthread_jit_write_protect_np(0);
    std::ptr::copy_nonoverlapping(reloc.as_ptr(), tramp as *mut u8, rl);
    std::ptr::copy_nonoverlapping(back.as_ptr(), (tramp as *mut u8).add(rl), 16);
    pthread_jit_write_protect_np(1);
    sys_icache_invalidate(tramp, rl + 16);
    // patch: os 8 bytes inteiros do alvo (é a função inteira -- não sobra nada da vizinha aqui).
    escrever_salto(t, &b, orig, tramp as u64, replacement as u64, destino).then_some(tramp)
}

/// Desfaz o hook mais recente de `target` com as threads paradas ([`remover_parado`]). Devolve `false` só quando o alvo segue com o hook (escrita
/// recusada pelo `set_prot`, lista de threads indisponível ou algum PC no trecho em todas as tentativas): a entrada continua na lista e
/// [`ganchos_no_endereco`] segue contando esse hook. Alvo sem hook nosso devolve `true` (nada a desfazer).
///
/// # Safety
/// Mesmos requisitos de [`remover_parado`].
pub(crate) unsafe fn remover(target: *mut c_void) -> bool {
    remover_parado(target, None).0 || ganchos_no_endereco(target) == 0
}

/// `red4ext-attach-detach-contract`: desfaz TODOS os hooks empilhados em `target` num call
/// só (o "Detach(target) -> ambos somem" do contrato) — chama `remover` em loop LIFO até
/// `target` não ter mais nenhum hook nosso. Idempotente: no-op se já não há hook ali.
/// Para na 1ª escrita recusada, com o hook do topo ainda no alvo e na lista.
///
/// # Safety
/// Mesmos requisitos de `remover`.
pub(crate) unsafe fn remover_todos(target: *mut c_void) {
    while ganchos_no_endereco(target) > 0 {
        // A escrita recusada deixa a entrada na lista: repetir não muda nada e o laço não terminaria.
        if !remover(target) {
            break;
        }
    }
}

/// Quantos hooks estão empilhados em `target` agora (0 = nenhum).
pub(crate) fn ganchos_no_endereco(target: *mut c_void) -> usize {
    let hooks = HOOKS.lock().unwrap();
    count_for(&hooks, target as u64)
}

/// O hook que devolveu o trampolim `tramp` continua empilhado em `target`?
pub(crate) fn gancho_no_ar(target: *mut c_void, tramp: u64) -> bool {
    let hooks = HOOKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    hooks.iter().any(|h| h.target == target as u64 && h.tramp == tramp)
}

/// `repl` está empilhado em `target` agora ([`substituto_no_alvo`])?
fn substituto_no_ar(target: *mut c_void, repl: *mut c_void) -> bool {
    let hooks = HOOKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    substituto_no_alvo(&hooks, target as u64, repl as u64)
}

/// Teste da API de plugin: entrada na lista sem patch no alvo, para um alvo em que a escrita da remoção é recusada.
#[cfg(test)]
pub(crate) fn empilhar_sem_patch(target: u64, tramp: u64) {
    HOOKS.lock().unwrap().push(Hook { target, orig: [0; 16], tramp, repl: 0 });
}

/// Teste da API de plugin: tira da lista as entradas de `target` sem escrever nele.
#[cfg(test)]
pub(crate) fn esquecer_sem_restaurar(target: u64) {
    HOOKS.lock().unwrap().retain(|h| h.target != target);
}

/// Aloca 1 página JIT NOVA contendo só `ret` — alvo dummy SEGURO pra hookar quando o teste não
/// precisa de um alvo real do jogo (ex.: `red4ext-attach-detach-contract`, provar o contrato de
/// empilhamento em si). NUNCA hookar uma função compilada no NOSSO PRÓPRIO dylib para isso: o
/// `set_prot`(RW+COPY) do `instalar()` te dá a página INTEIRA (4K) do alvo gravável — se essa
/// página for compartilhada com o código que está EXECUTANDO a própria chamada de `instalar()`
/// (plausível quando o alvo é uma fn Rust pequena definida perto de quem chama), a auto-
/// modificação em pleno voo pode dar fault de instrução ali mesmo. Uma página `mmap` nova nunca
/// colide com nenhum código nosso em execução.
pub unsafe fn stub_que_so_retorna() -> Option<*mut c_void> {
    let m = mmap(std::ptr::null_mut(), 4096, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANON | MAP_JIT, -1, 0);
    if m.is_null() || m as isize == -1 {
        return None;
    }
    pthread_jit_write_protect_np(0);
    (m as *mut u32).write_unaligned(0xD65F_03C0u32); // ret
    pthread_jit_write_protect_np(1);
    sys_icache_invalidate(m, 4);
    Some(m)
}

/// Roda `f` com as outras threads do processo suspensas, sem tocar em proteção de página nem na
/// lista de hooks: serve pra escrever DADO que outra thread pode estar lendo (o ponteiro do
/// bytecode de uma função de script, por exemplo), não código.
///
/// # Safety
/// `f` não pode alocar, travar mutex nem chamar código do jogo — as outras threads estão paradas e
/// qualquer trava que elas estejam segurando não sai.
pub unsafe fn com_threads_paradas<R>(f: impl FnOnce() -> R) -> Option<R> {
    let eu = mach_thread_self();
    let mut lista: *mut MachPort = std::ptr::null_mut();
    let mut quantas: u32 = 0;
    if task_threads(mach_task_self_, &mut lista, &mut quantas) != KERN_SUCCESS {
        mach_port_deallocate(mach_task_self_, eu);
        return None;
    }
    let threads = std::slice::from_raw_parts(lista, quantas as usize);
    let mut paradas: Vec<MachPort> = Vec::with_capacity(threads.len());
    for &th in threads {
        if th != eu && thread_suspend(th) == KERN_SUCCESS {
            paradas.push(th);
        }
    }
    let r = f();
    for &th in &paradas {
        thread_resume(th);
    }
    liberar_threads(threads, lista, quantas);
    mach_port_deallocate(mach_task_self_, eu);
    Some(r)
}
