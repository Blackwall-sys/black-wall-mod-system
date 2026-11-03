// ponte-mappin-cooked-count.reds — quantos mappins, multi-mappins e PoIs o `MappinSystem` já tem nos recursos cozidos em cache
// (`CookedMappinResource@0x58`/`CookedPoiResource@0x68`, offsets do header gerado da reflection do jogo). API do BWMS, só leitura.
//
// Retorno `Int32`: `-1` = recurso ainda não cacheado ou `MappinSystem` inválido; `0` = cacheado e vazio. O `-1` é comum: o cache só
// enche sob demanda.

native func BwmsMappinSystemCookedMappinCount(sys: ref<MappinSystem>) -> Int32;
native func BwmsMappinSystemCookedMultiMappinCount(sys: ref<MappinSystem>) -> Int32;
native func BwmsMappinSystemCookedPoiCount(sys: ref<MappinSystem>) -> Int32;

// Só LEITURA em todos os 3 casos (nunca muta o MappinSystem/os recursos cacheados) — seguro pra
// chamar automaticamente em qualquer momento do gameplay, sem gate/aval manual.
@addMethod(MappinSystem)
public func GetCookedMappinCount() -> Int32 {
    return BwmsMappinSystemCookedMappinCount(this);
}

@addMethod(MappinSystem)
public func GetCookedMultiMappinCount() -> Int32 {
    return BwmsMappinSystemCookedMultiMappinCount(this);
}

@addMethod(MappinSystem)
public func GetCookedPoiCount() -> Int32 {
    return BwmsMappinSystemCookedPoiCount(this);
}
