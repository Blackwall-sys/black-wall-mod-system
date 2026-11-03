// ponte-mappinsystem.reds — `MappinSystem.GetAllMappins()` e a fase de ponto de
// interesse.
//
// `GetAllMappins()`: os campos C++ privados `MappinsLock@0x198`/`MappinsData@0x1A0` (`SortedArray<MappinData>`) não são lidos, porque o
// layout do elemento do `SortedArray` no Mac nunca foi conferido. Aqui são 3 chamadas ao `MappinSystem.GetMappins(targetType, out
// mappins)` vanilla (`cp77-symbols/redscript-src/orphans.script`), uma por valor de `gamemappinsMappinTargetType` (`World`/`Minimap`/
// `Map`), fundidas sem repetir handle (`ArrayContains`): todos os mappins ativos, sem campo cru nem endereço nativo.

@addMethod(MappinSystem)
private final func BwmsCollectMappins(targetType: gamemappinsMappinTargetType, all: array<ref<IMappin>>) -> array<ref<IMappin>> {
    let batch: array<ref<IMappin>>;
    this.GetMappins(targetType, batch);

    let i: Int32 = 0;
    while i < ArraySize(batch) {
        if !ArrayContains(all, batch[i]) {
            ArrayPush(all, batch[i]);
        }
        i += 1;
    }

    return all;
}

@addMethod(MappinSystem)
public func GetAllMappins() -> array<ref<IMappin>> {
    let all: array<ref<IMappin>>;
    all = this.BwmsCollectMappins(gamemappinsMappinTargetType.World, all);
    all = this.BwmsCollectMappins(gamemappinsMappinTargetType.Minimap, all);
    all = this.BwmsCollectMappins(gamemappinsMappinTargetType.Map, all);
    return all;
}

// ---------------------------------------------------------------------------------------------
// Fase de ponto de interesse no `MappinSystem`:
//
//   SetPoiMappinPhase  slot 0x1D0  void(uint32_t journalPath, gamedataMappinPhase phase)
//   GetPoiMappinPhase  slot 0x3C8  gamedataMappinPhase(uint32_t journalPath)
//
// São slots de VTABLE (offset Windows em bytes). No Mac os slots NÃO saem de "Windows + 0x08": foram medidos na vtable real do
// `MappinSystem` (`MAPPIN_SET/GET_POI_PHASE_SLOT`, missoes_e_mapa/mod.rs) — `0x3D0` é o setter e `0x3D8` o getter, os dois sobre o
// mesmo byte de flags da entrada do PoI. A native lê a vtable do objeto e chama o slot, conferindo cada ponteiro com
// `memoria::is_readable` antes de seguir (objeto/vtable/slot/função).
// ---------------------------------------------------------------------------------------------

native func BwmsMappinSystemSetPoiPhase(sys: ref<MappinSystem>, journalPath: Uint32, phase: Int32) -> Void;
native func BwmsMappinSystemGetPoiPhase(sys: ref<MappinSystem>, journalPath: Uint32) -> Int32;

// MUDA estado real de mappin de quest. O 3º argumento chega ao motor como FLAG, não como fase: diferente de 0 liga o bit 2 do byte
// de flags da entrada, 0 desliga.
@addMethod(MappinSystem)
public func SetPoiMappinPhase(journalPath: Uint32, phase: gamedataMappinPhase) -> Void {
    BwmsMappinSystemSetPoiPhase(this, journalPath, EnumInt(phase));
}

// Só leitura: o byte cru de flags da entrada, convertido pro enum — não é uma fase de verdade. 0 quando a entrada não existe.
@addMethod(MappinSystem)
public func GetPoiMappinPhase(journalPath: Uint32) -> gamedataMappinPhase {
    return IntEnum<gamedataMappinPhase>(BwmsMappinSystemGetPoiPhase(this, journalPath));
}
