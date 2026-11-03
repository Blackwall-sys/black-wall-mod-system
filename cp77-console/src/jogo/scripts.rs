//! Executor de scripts do motor.

/// Executor universal (vmaddr; base de link 0x100000000). Mesmo do probe.js.
pub(crate) const EXECUTOR_VM: u64 = 0x1_0109_9fbc;

/// Assinatura dos 8 primeiros bytes do executor (`stp x28,x27,[sp,#-0x60]!` +
/// `stp x26,x25,[sp,#0x10]`). O selfboot só hooka se bater — nunca chuta no endereço, e
/// se um patch do jogo mover/mudar a função ele ABORTA limpo (sem crash). Os bytes são
/// iguais no Steam e no GOG, só o endereço muda: o `game_build` identifica o build por ela.
pub(crate) const EXECUTOR_PROLOGO: u64 = 0xa901_67fa_a9ba_6ffc;

#[cfg(test)]
mod testes {
    /// Codifica `stp Xt, Xt2, [sp, #imm]` de 64 bits; `pre` = forma pré-indexada `[sp, #imm]!`.
    fn stp_sp(rt: u32, rt2: u32, imm: i32, pre: bool) -> u32 {
        let base = if pre { 0xa980_0000 } else { 0xa900_0000 };
        base | ((((imm / 8) as u32) & 0x7f) << 15) | (rt2 << 10) | (31 << 5) | rt
    }

    #[test]
    fn prologo_do_executor_sao_os_dois_stp_em_little_endian() {
        let primeiro = stp_sp(28, 27, -0x60, true);
        let segundo = stp_sp(26, 25, 0x10, false);
        assert_eq!(primeiro, 0xa9ba_6ffc);
        assert_eq!(segundo, 0xa901_67fa);
        // A leitura de 8 bytes é `read_unaligned::<u64>`: a 1ª instrução cai na metade baixa.
        assert_eq!(super::EXECUTOR_PROLOGO, (segundo as u64) << 32 | primeiro as u64);
    }
}
