//! Mundo: spawner, fases de missão, nós de malha instanciada e a garagem de veículos.

pub(crate) const SPAWNER_VPTR_VM: u64 = 0x1_07c6_d9a8;

pub(crate) const FASE_DE_MISSAO_VPTR_VM: u64 = 0x1_07a2_dde0;

pub(crate) const NO_MALHA_INSTANCIADA_VPTR_VM: u64 = 0x1_078e_5130;

/// `VehicleSystem::ToggleGarageVehicle(this, &GarageVehicleID, enable) -> bool`: x0 = `VehicleSystem*`, x1 = `&GarageVehicleID{TweakDBID recordID @0,
/// CName name @8}` (16 B por referência), w2 = `enable`, w0 = retorno. ABI simples, sem sret, prólogo real (não é thunk). RE
/// offline de 2026-07-29, confiança alta: 3 callers no binário (`EnablePlayerVehicle`/`EnableAllPlayerVehicles`), cadeia validada 2 vezes.
pub(crate) const VEHICLESYSTEM_TOGGLEGARAGEVEHICLE_VM: u64 = 0x1_02aa_9a7c;
