//! Hooking + sonda de memória NATIVOS (Rust puro), sem nenhuma lib de terceiros.
//! Substitui o uso anterior de uma lib de instrumentação externa — o runtime que
//! vai no mod é 100% nosso.
//!
//! - `is_readable`: consulta o mapa de memória do processo via mach_vm_read_overwrite
//!   (não crasha em ponteiro inválido — base da leitura segura de structs RTTI).
//! - `Interceptor::replace`: inline-hook arm64 próprio. O alvo (o executor de nativas
//!   do jogo) tem prólogo só com stp/add/sub — SEM instrução PC-relativa — então o
//!   trampolim copia os 1os 16 bytes verbatim, sem relocação: simples e seguro.

use std::ffi::c_void;
use std::sync::Mutex;

type KernReturn = i32;
type MachPort = u32;
const KERN_SUCCESS: KernReturn = 0;
const VM_PROT_READ: i32 = 1;
const VM_PROT_WRITE: i32 = 2;
const VM_PROT_EXECUTE: i32 = 4;
const VM_PROT_COPY: i32 = 0x10;

const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const PROT_EXEC: i32 = 4;
const MAP_PRIVATE: i32 = 0x0002;
const MAP_FIXED: i32 = 0x0010;
const MAP_ANON: i32 = 0x1000;
const MAP_JIT: i32 = 0x0800;
const MAP_FAILED: isize = -1;

extern "C" {
    static mach_task_self_: MachPort;
    fn mach_vm_read_overwrite(task: MachPort, address: u64, size: u64, data: u64, out_size: *mut u64) -> KernReturn;
    fn mach_vm_protect(task: MachPort, address: u64, size: u64, set_max: i32, new_prot: i32) -> KernReturn;
    fn sys_icache_invalidate(start: *mut c_void, len: usize);
    fn pthread_jit_write_protect_np(enabled: i32);
    fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> i32;
    fn mach_vm_region(target_task: MachPort, address: *mut u64, size: *mut u64, flavor: i32, info: *mut u32, info_cnt: *mut u32, object_name: *mut MachPort) -> KernReturn;
    fn mach_vm_allocate(target_task: MachPort, address: *mut u64, size: u64, flags: i32) -> KernReturn;
    fn mach_vm_remap(
        target_task: MachPort, target_address: *mut u64, size: u64, mask: u64, flags: i32,
        src_task: MachPort, src_address: u64, copy: i32,
        cur_prot: *mut i32, max_prot: *mut i32, inheritance: i32,
    ) -> KernReturn;
    fn __error() -> *mut i32;
}
const VM_FLAGS_FIXED: i32 = 0x0;
const VM_FLAGS_ANYWHERE: i32 = 0x1;
const VM_FLAGS_OVERWRITE: i32 = 0x4000;
const VM_REGION_BASIC_INFO_64: i32 = 9;

/// True se [address, address+len) está mapeado e legível AGORA. Não crasha. Usado
/// pelos resolvedores/diagnósticos que leem memória do jogo a partir de offsets crus.
pub unsafe fn is_readable(address: *const c_void, len: usize) -> bool {
    if address.is_null() || len == 0 {
        return false;
    }
    let n = len.min(512);
    let mut buf = [0u8; 512];
    let mut outsz: u64 = 0;
    let kr = mach_vm_read_overwrite(mach_task_self_, address as u64, n as u64, buf.as_mut_ptr() as u64, &mut outsz);
    kr == KERN_SUCCESS && outsz as usize >= n
}

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
    // esquema de vmaddr é LINK_BASE=0x1_0000_0000 + offset), mas `near_to - MARGIN` frequentemente
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
            crate::log(&format!("[gum] alloc_near_landing_pad: mach_vm_remap(src={src:p} -> free={free:#x}) -> kr={kr_remap} target={target:#x}"));
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
// trampolim pode rodar em qualquer endereço. Instruções PC-relativas ainda não tratadas
// (B/BL/B.cond/CBZ/CBNZ/TBZ/TBNZ/LDR-literal) -> retorna None = recusa o hook (seguro, sem
// corromper). Não-PC-relativas (stp/sub/add/mov/ret/br/...) -> verbatim. O executor (prólogo
// stp/add/sub) cai no verbatim, então o hook existente continua idêntico.
unsafe fn relocate_prologue(orig: u64) -> Option<Vec<u8>> {
    relocate_n(orig, 4)
}

/// Reloca `n_insns` instruções do prólogo. n=4 p/ o abs-jump de 16B (`replace`); n=1 p/ o
/// `replace_near4`, que rouba só a 1ª instrução (cabe num `B` de 4B). Mesma lógica de conserto
/// de PC-relativo (ADR/ADRP/B/cond/CBZ/TBZ/LDR-lit consertados; BL/literais raros = recusa).
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
}
static HOOKS: Mutex<Vec<Hook>> = Mutex::new(Vec::new());

// ---- bookkeeping do contrato attach/detach (puro, unit-testável sem tocar memória) ----
// Vários hooks podem empilhar no MESMO alvo: cada `Hook.orig` guarda os 16 bytes que estavam lá
// QUANDO ELE aplicou (o `replace` copia o prólogo atual antes de patchar). Logo o desfazer correto
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
pub unsafe fn vtable_hook(vtbl: *mut u64, slot_idx: usize, replacement: *const c_void) -> Option<*const c_void> {
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

/// Hooka VÁRIOS slots da MESMA vtable com UM ÚNICO ciclo de COW. O `vtable_hook` repetido faz
/// `VM_PROT_COPY` N vezes na mesma página → corrompe o mapeamento (crash). Aqui: COW a página 1×,
/// escreve todos os slots, restaura RO 1×. Slots com original 0 (gaps) são pulados. Devolve os
/// originais (0 nos pulados), na ordem de `hooks`.
/// # Safety
/// `vtbl` válida; todos os `slot` dentro da mesma página de `vtbl`.
pub unsafe fn vtable_hook_bulk(vtbl: *mut u64, hooks: &[(usize, *const c_void)]) -> Vec<*const c_void> {
    let mut origs = vec![core::ptr::null(); hooks.len()];
    if vtbl.is_null() || hooks.is_empty() {
        return origs;
    }
    let page_addr = vtbl as u64;
    if !set_prot(page_addr, VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY) {
        return origs;
    }
    for (i, &(slot, repl)) in hooks.iter().enumerate() {
        let s = vtbl.add(slot);
        let orig = s.read();
        origs[i] = orig as *const c_void;
        if orig != 0 {
            s.write(repl as u64);
        }
    }
    set_prot(page_addr, VM_PROT_READ);
    origs
}

/// Desfaz um `vtable_hook`, restaurando o ponteiro original no slot.
/// # Safety
/// Mesmos requisitos do `vtable_hook`.
pub unsafe fn vtable_unhook(vtbl: *mut u64, slot_idx: usize, original: *const c_void) {
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
unsafe extern "C" fn vtable_dummy_repl() -> u64 {
    0xBADC_0FFE_E0DD_F00D
}

/// Self-test do `vtable_hook`/`vtable_unhook` (DEV) contra a vtable REAL de um objeto vivo: prova o
/// write em __DATA_CONST via COW + o restore. Acha um slot RARO com ptr de código no módulo (começa
/// alto p/ evitar métodos hot), hooka→confirma slot==dummy→desfaz→confirma slot==orig. Janela
/// síncrona (nanos, sem yield) → o jogo praticamente não chama o slot nesse meio. Retorna relatório.
pub unsafe fn vtable_selftest(obj: *mut c_void) -> String {
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
    let orig = match vtable_hook(vtbl, slot_idx, dummy) {
        Some(o) => o,
        None => return format!("[vtbl-test] vtable_hook FALHOU (set_prot COW recusou) slot={slot_idx}"),
    };
    let hooked = vtbl.add(slot_idx).read();
    vtable_unhook(vtbl, slot_idx, orig);
    let restored = vtbl.add(slot_idx).read();
    format!(
        "[vtbl-test] vtbl static {vtbl_static:#x} in_module(__DATA_CONST)={in_module} slot={slot_idx}: hook {before:#x}->{hooked:#x} (==dummy {}) | unhook ->{restored:#x} (==orig {}) | OK={}",
        hooked == dummy as u64,
        restored == before,
        hooked == dummy as u64 && restored == before
    )
}

static VTBL_TEST_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Roda o vtable_selftest 1x em gameplay quando `~/.bwms-vtable-test` existe (dev).
pub unsafe fn vtable_selftest_once(obj: *mut c_void) {
    let on = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-vtable-test").exists())
        .unwrap_or(false);
    if !on || obj.is_null() {
        return;
    }
    if VTBL_TEST_DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    crate::log(&vtable_selftest(obj));
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- contrato attach/detach: bookkeeping puro (LIFO + original verdadeiro) ----

    fn hk(target: u64, tag: u8) -> Hook {
        Hook { target, orig: [tag; 16] } // `orig` marcado com o tag p/ rastrear qual bytes voltam
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
        // Simula o desfazer que o `revert` faz (remove o latest_index_for a cada passo) e confere
        // que os bytes restaurados saem na ordem LIFO: primeiro o penúltimo estado, depois o original.
        let mut hooks = vec![hk(0xA, 1), hk(0xA, 2), hk(0xA, 3)]; // 3 hooks empilhados no 0xA
        let mut restaurados = Vec::new();
        while let Some(pos) = latest_index_for(&hooks, 0xA) {
            restaurados.push(hooks[pos].orig[0]); // tag dos bytes que o revert escreveria
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
    /// `Interceptor::replace` escreve no alvo) — o cenário de um hook capturando o que OUTRO hook
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
        // endereço com seu PRÓPRIO alvo (b_addr), exatamente como Interceptor::replace faz.
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
}

pub struct Interceptor;

impl Interceptor {
    pub fn obtain() -> Self {
        Interceptor
    }

    /// Substitui `target` por `replacement` (uma `extern "C" fn`) e devolve o
    /// trampolim chamável da função original, ou `None` se falhar.
    ///
    /// # Safety
    /// `target` precisa apontar pra função real já rebaseada com o slide;
    /// `replacement` precisa ter ABI compatível com a original.
    pub unsafe fn replace(&self, target: *mut c_void, replacement: *mut c_void) -> Option<*mut c_void> {
        let t = target as u64;

        // 1) trampolim executável (MAP_JIT): [16 bytes do prólogo original][salto p/ target+16]
        let tramp = mmap(std::ptr::null_mut(), 4096, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANON | MAP_JIT, -1, 0);
        if tramp.is_null() || tramp as isize == -1 {
            return None;
        }
        let mut orig = [0u8; 16];
        std::ptr::copy_nonoverlapping(target as *const u8, orig.as_mut_ptr(), 16);
        // Reloca o prólogo deslocado (conserta adr/adrp); None = prólogo com PC-relativo
        // ainda não tratado -> recusa o hook (seguro, não corrompe).
        let reloc = match relocate_prologue(t) {
            Some(r) => r,
            None => return None,
        };
        let rl = reloc.len();
        let back = abs_jump(t + 16);
        pthread_jit_write_protect_np(0); // JIT -> gravável
        std::ptr::copy_nonoverlapping(reloc.as_ptr(), tramp as *mut u8, rl);
        std::ptr::copy_nonoverlapping(back.as_ptr(), (tramp as *mut u8).add(rl), 16);
        pthread_jit_write_protect_np(1); // JIT -> executável
        sys_icache_invalidate(tramp, rl + 16);

        // 2) patch no alvo: salta pra `replacement`. Torna a página gravável (COW),
        //    escreve os 16 bytes, restaura RX, invalida a i-cache.
        if !set_prot(t, VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY) {
            return None;
        }
        let patch = abs_jump(replacement as u64);
        std::ptr::copy_nonoverlapping(patch.as_ptr(), target as *mut u8, 16);
        set_prot(t, VM_PROT_READ | VM_PROT_EXECUTE);
        sys_icache_invalidate(target, 16);

        HOOKS.lock().unwrap().push(Hook { target: t, orig });
        Some(tramp)
    }

    /// Como `replace`, mas p/ FUNÇÕES PEQUENAS (leaf de 8B etc.): escreve só um `B` de 4 bytes
    /// no alvo (rouba 1 instrução) em vez do abs-jump de 16B → **NÃO transborda a função vizinha**
    /// (a causa do SIGILL @0x3f5ec7c ao hookar o getter da phase-byte). Exige `replacement` em
    /// ±128MB do alvo (alcance do `B`); se longe, tenta um landing-pad JIT perto do alvo (2 saltos,
    /// alcance irrestrito no 2º) antes de recusar — achado 2026-07-12: no GOG o slide do dylib vs. o
    /// jogo empurra o getter da phase byte pra fora de ±128MB (funcionava sempre no Steam, nunca no
    /// GOG até este fallback). Só recusa (`None`) se nem o pad couber em lugar nenhum.
    ///
    /// # Safety
    /// Mesmos requisitos de `replace`. O alvo deve ter ≥4 bytes de prólogo não-BL.
    pub unsafe fn replace_near4(&self, target: *mut c_void, replacement: *mut c_void) -> Option<*mut c_void> {
        let t = target as u64;
        // GUARD: B só alcança ±128MB. Longe → tenta um landing-pad perto do alvo antes de recusar.
        let b = match emit_b_near(t, replacement as u64) {
            Some(b) => b,
            None => {
                let pad = alloc_near_landing_pad(t, replacement as u64)?;
                crate::log(&format!("[gum] replace_near4: fora de alcance direto, landing-pad @ {pad:#x}"));
                emit_b_near(t, pad)?
            }
        };
        // trampolim do original: 1ª instrução roubada (relocada) + abs-jump pro alvo+4.
        let reloc = relocate_n(t, 1)?;
        let tramp = mmap(std::ptr::null_mut(), 4096, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANON | MAP_JIT, -1, 0);
        if tramp.is_null() || tramp as isize == -1 {
            return None;
        }
        let mut orig = [0u8; 16];
        std::ptr::copy_nonoverlapping(target as *const u8, orig.as_mut_ptr(), 16);
        let back = abs_jump(t + 4);
        let rl = reloc.len();
        pthread_jit_write_protect_np(0);
        std::ptr::copy_nonoverlapping(reloc.as_ptr(), tramp as *mut u8, rl);
        std::ptr::copy_nonoverlapping(back.as_ptr(), (tramp as *mut u8).add(rl), 16);
        pthread_jit_write_protect_np(1);
        sys_icache_invalidate(tramp, rl + 16);
        // patch: SÓ 4 bytes no alvo (a vizinha fica intacta).
        if !set_prot(t, VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY) {
            return None;
        }
        std::ptr::copy_nonoverlapping(b.as_ptr(), target as *mut u8, 4);
        set_prot(t, VM_PROT_READ | VM_PROT_EXECUTE);
        sys_icache_invalidate(target, 4);
        HOOKS.lock().unwrap().push(Hook { target: t, orig });
        Some(tramp)
    }

    /// Como `replace_near4`, mas usa `ADRP+BR` (8 bytes, ±4GB de alcance) em vez de `B` (4 bytes,
    /// ±128MB) — rouba as DUAS instruções do getter de 8B (em vez de só a 1ª) mas ainda cabe
    /// exatamente sem transbordar a vizinha. Achado 2026-07-12 (GOG): o landing-pad de
    /// `replace_near4` precisa de um endereço DENTRO de ±128MB do alvo, e o macOS recusa alocar
    /// memória nova nessa vizinhança (reserva de kernel em torno do Mach-O do jogo). Com ADRP
    /// (±4GB) o orçamento é 32x maior — um `mmap` NORMAL (sem `MAP_FIXED`, sempre bem-sucedido,
    /// em QUALQUER endereço que o kernel escolher) cai folgadamente dentro de ±4GB de qualquer
    /// alvo no mesmo processo, sem precisar achar um endereço específico livre.
    ///
    /// # Safety
    /// Mesmos requisitos de `replace`. O alvo deve ter EXATAMENTE 8 bytes de prólogo (2
    /// instruções) não-PC-relativas e não-BL (ambas relocáveis verbatim).
    pub unsafe fn replace_adrp_br8(&self, target: *mut c_void, replacement: *mut c_void) -> Option<*mut c_void> {
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
                crate::log(&format!("[gum] replace_adrp_br8: landing-pad @ {pad_addr:#x} fora de ±4GB do alvo (inesperado)"));
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

        // trampolim do original: as 2 instruções roubadas (relocadas) + abs-jump pro alvo+8.
        let reloc = relocate_n(t, 2)?;
        let tramp = mmap(std::ptr::null_mut(), 4096, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANON | MAP_JIT, -1, 0);
        if tramp.is_null() || tramp as isize == -1 {
            return None;
        }
        let mut orig = [0u8; 16];
        std::ptr::copy_nonoverlapping(target as *const u8, orig.as_mut_ptr(), 16);
        let back = abs_jump(t + 8);
        let rl = reloc.len();
        pthread_jit_write_protect_np(0);
        std::ptr::copy_nonoverlapping(reloc.as_ptr(), tramp as *mut u8, rl);
        std::ptr::copy_nonoverlapping(back.as_ptr(), (tramp as *mut u8).add(rl), 16);
        pthread_jit_write_protect_np(1);
        sys_icache_invalidate(tramp, rl + 16);
        // patch: os 8 bytes inteiros do alvo (é a função inteira -- não sobra nada da vizinha aqui).
        if !set_prot(t, VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY) {
            return None;
        }
        std::ptr::copy_nonoverlapping(b.as_ptr(), target as *mut u8, 8);
        set_prot(t, VM_PROT_READ | VM_PROT_EXECUTE);
        sys_icache_invalidate(target, 8);
        HOOKS.lock().unwrap().push(Hook { target: t, orig });
        Some(tramp)
    }

    /// # Safety
    /// `target` precisa ser um alvo previamente substituído por `replace`.
    pub unsafe fn revert(&self, target: *mut c_void) {
        let t = target as u64;
        let mut hooks = HOOKS.lock().unwrap();
        // LIFO: desfaz o hook mais RECENTE do alvo. Restaura os bytes que estavam lá antes DELE
        // (`orig`), o que reativa o hook anterior no mesmo alvo (ou o original, se era o único).
        // Com 1 hook no alvo (caso comum), é idêntico ao comportamento antigo.
        if let Some(pos) = latest_index_for(&hooks, t) {
            let orig = hooks[pos].orig;
            if set_prot(t, VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY) {
                std::ptr::copy_nonoverlapping(orig.as_ptr(), target as *mut u8, 16);
                set_prot(t, VM_PROT_READ | VM_PROT_EXECUTE);
                sys_icache_invalidate(target, 16);
            }
            hooks.remove(pos);
        }
    }

    /// `red4ext-attach-detach-contract`: desfaz TODOS os hooks empilhados em `target` num call
    /// só (o "Detach(target) -> ambos somem" do contrato) — chama `revert` em loop LIFO até
    /// `target` não ter mais nenhum hook nosso. Idempotente: no-op se já não há hook ali.
    ///
    /// # Safety
    /// Mesmos requisitos de `revert`.
    pub unsafe fn revert_all(&self, target: *mut c_void) {
        while self.hooks_on(target) > 0 {
            self.revert(target);
        }
    }

    /// Quantos hooks estão empilhados em `target` agora (0 = nenhum).
    pub fn hooks_on(&self, target: *mut c_void) -> usize {
        let hooks = HOOKS.lock().unwrap();
        count_for(&hooks, target as u64)
    }
}

/// Aloca 1 página JIT NOVA contendo só `ret` — alvo dummy SEGURO pra hookar quando o teste não
/// precisa de um alvo real do jogo (ex.: `red4ext-attach-detach-contract`, provar o contrato de
/// empilhamento em si). NUNCA hookar uma função compilada no NOSSO PRÓPRIO dylib para isso: o
/// `set_prot`(RW+COPY) do `replace()` te dá a página INTEIRA (4K) do alvo gravável — se essa
/// página for compartilhada com o código que está EXECUTANDO a própria chamada de `replace()`
/// (plausível quando o alvo é uma fn Rust pequena definida perto de quem chama), a auto-
/// modificação em pleno voo pode dar fault de instrução ali mesmo. Uma página `mmap` nova nunca
/// colide com nenhum código nosso em execução.
pub unsafe fn alloc_ret_stub() -> Option<*mut c_void> {
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
