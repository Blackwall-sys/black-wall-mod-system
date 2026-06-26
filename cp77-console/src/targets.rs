//! Alvos com endereço ESTÁTICO no binário (thunks script-bound `::funcXxx`),
//! verificados por `nm`/`c++filt` (read-only) — hookáveis HOJE, SEM precisar
//! resolver a RTTI em runtime. São 172 no total; aqui os relevantes pro começo.
//!
//! Os endereços são VM addrs do Mach-O (base [`IMAGE_BASE`]); em runtime soma-se
//! o slide de ASLR do módulo principal (ver `rebase` em lib.rs).

pub const IMAGE_BASE: u64 = 0x1_0000_0000;

pub struct Thunk {
    pub name: &'static str,
    pub vmaddr: u64,
}

/// Leads pro NOCLIP / movimento (LocomotionParameters) + leitura de var
/// persistente/blackboard — todos com thunk estático confirmado.
pub const STATIC_THUNKS: &[Thunk] = &[
    // movimento — candidatos a noclip (zerar gravidade, ignorar relevo, pular)
    Thunk { name: "ActionLocomotionParameters::funcSetUpwardsGravity", vmaddr: 0x1_03bb_804c },
    Thunk { name: "ActionLocomotionParameters::funcSetDownwardsGravity", vmaddr: 0x1_03bb_80d0 },
    Thunk { name: "ActionLocomotionParameters::funcSetIgnoreSlope", vmaddr: 0x1_03bb_8574 },
    Thunk { name: "ActionLocomotionParameters::funcSetDoJump", vmaddr: 0x1_03bb_847c },
    Thunk { name: "ActionLocomotionParameters::funcSetCapsuleHeight", vmaddr: 0x1_03bb_8364 },
    Thunk { name: "ActionLocomotionParameters::funcSetCapsuleRadius", vmaddr: 0x1_03bb_83f0 },
    // leitura de estado (úteis pra console/debug)
    Thunk { name: "PersistencySystem::funcGetPersistentVar<bool>", vmaddr: 0x1_03ff_5014 },
    Thunk { name: "IBlackboard::funcGet<int>", vmaddr: 0x1_0118_f2cc },
];
