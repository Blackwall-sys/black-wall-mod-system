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
const MAP_ANON: i32 = 0x1000;
const MAP_JIT: i32 = 0x0800;

extern "C" {
    static mach_task_self_: MachPort;
    fn mach_vm_read_overwrite(task: MachPort, address: u64, size: u64, data: u64, out_size: *mut u64) -> KernReturn;
    fn mach_vm_protect(task: MachPort, address: u64, size: u64, set_max: i32, new_prot: i32) -> KernReturn;
    fn sys_icache_invalidate(start: *mut c_void, len: usize);
    fn pthread_jit_write_protect_np(enabled: i32);
    fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut c_void;
}

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
        // LDR literal 64-bit GP (0x58) -> materializa o endereço e faz ldr [Xscratch].
        if (insn & 0xFF00_0000) == 0x5800_0000 {
            let rt = insn & 0x1F;
            let mut off = ((insn >> 5) & 0x7_FFFF) as i64;
            if off & (1 << 18) != 0 { off |= !0x7_FFFFi64; }
            let lit_addr = (ia as i64).wrapping_add(off << 2) as u64;
            let scratch = if rt == 16 { 17u32 } else { 16u32 };
            emit_load_imm64(&mut out, scratch, lit_addr);
            let ldr = 0xF940_0000u32 | (scratch << 5) | rt; // ldr Xt, [Xscratch]
            out.extend_from_slice(&ldr.to_le_bytes());
            i += 1;
            continue;
        }
        // BL (chamada) e outros literais (32-bit/SIMD/LDRSW/PRFM): ainda não tratados -> recusa segura.
        let is_bl = (insn & 0xFC00_0000) == 0x9400_0000;
        let is_other_lit = (insn & 0x3B00_0000) == 0x1800_0000;
        if is_bl || is_other_lit {
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
        let ldrlit = 0x5800_0040u32; // ldr x0, #8
        let ret = 0xD65F_03C0u32;
        let buf = mk_buf([ldrlit, ret, ret, ret]);
        let a = buf.as_ptr() as u64;
        let out = unsafe { relocate_prologue(a) }.expect("ldr-literal deve relocar");
        assert_eq!(decode_load_imm64(&out[0..16]), a + 8, "x16 = endereço do literal");
        assert_eq!(&out[16..20], &0xF940_0200u32.to_le_bytes(), "ldr x0, [x16]");
    }

    #[test]
    fn bl_recusa() {
        let bl = 0x9400_0001u32; // bl #4 (chamada — precisa preservar x30)
        let ret = 0xD65F_03C0u32;
        let buf = mk_buf([bl, ret, ret, ret]);
        assert!(unsafe { relocate_prologue(buf.as_ptr() as u64) }.is_none(), "BL no prólogo -> recusa (preservar x30)");
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
    /// ±128MB do alvo (alcance do `B`); se longe, RECUSA limpo (`None`) — guard anti-transbordo,
    /// nunca corrompe. O trampolim devolvido roda a 1ª instrução (relocada) + volta pro alvo+4.
    ///
    /// # Safety
    /// Mesmos requisitos de `replace`. O alvo deve ter ≥4 bytes de prólogo não-BL.
    pub unsafe fn replace_near4(&self, target: *mut c_void, replacement: *mut c_void) -> Option<*mut c_void> {
        let t = target as u64;
        // GUARD: B só alcança ±128MB. Longe → recusa (nunca escreve 16B numa função pequena).
        let b = emit_b_near(t, replacement as u64)?;
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

    /// # Safety
    /// `target` precisa ser um alvo previamente substituído por `replace`.
    pub unsafe fn revert(&self, target: *mut c_void) {
        let t = target as u64;
        let mut hooks = HOOKS.lock().unwrap();
        if let Some(pos) = hooks.iter().position(|h| h.target == t) {
            let orig = hooks[pos].orig;
            if set_prot(t, VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY) {
                std::ptr::copy_nonoverlapping(orig.as_ptr(), target as *mut u8, 16);
                set_prot(t, VM_PROT_READ | VM_PROT_EXECUTE);
                sys_icache_invalidate(target, 16);
            }
            hooks.remove(pos);
        }
    }
}
