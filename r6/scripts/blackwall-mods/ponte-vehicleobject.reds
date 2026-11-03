// ponte-vehicleobject.reds — os 5 campos de telemetria do veículo.
//
// `@addField(VehicleObject) public native let X` exigiria uma property RTTI registrada em C++.
// No Mac ninguém registra essa property, e declará-la como `native let` é a categoria do `CMesh`, que derruba o bind. Aqui são GETTERS
// por função nativa que lê o campo direto do objeto. Offsets do header gerado (`Scripting/Natives/
// vehicleBaseObject.hpp`), campo de dado sem deslocamento de vtable; o `isOnGround` foi conferido no
// registrador de propriedade do RTTI do Mac e ao vivo, em carros no chão:
//   isOnGround   : bool  @ +0x25C
//   acceleration : float @ +0x264
//   deceleration : float @ +0x268
//   isReversing  : bool  @ +0x2A3
//   burnout      : float @ +0x2BC
//
// `VehicleObject` é vanilla e visível ao script: é o retorno da native vanilla `GetMountedVehicle(object: ref<GameObject>) ->
// wref<VehicleObject>` (`core/gameplay/vehicles.script:1490`).
native func BwmsVehicleObjectGetIsOnGround(veh: ref<VehicleObject>) -> Bool
native func BwmsVehicleObjectGetAcceleration(veh: ref<VehicleObject>) -> Float
native func BwmsVehicleObjectGetDeceleration(veh: ref<VehicleObject>) -> Float
native func BwmsVehicleObjectGetIsReversing(veh: ref<VehicleObject>) -> Bool
native func BwmsVehicleObjectGetBurnout(veh: ref<VehicleObject>) -> Float

@addMethod(VehicleObject)
public func GetIsOnGround() -> Bool {
    return BwmsVehicleObjectGetIsOnGround(this);
}

@addMethod(VehicleObject)
public func GetAcceleration() -> Float {
    return BwmsVehicleObjectGetAcceleration(this);
}

@addMethod(VehicleObject)
public func GetDeceleration() -> Float {
    return BwmsVehicleObjectGetDeceleration(this);
}

@addMethod(VehicleObject)
public func GetIsReversing() -> Bool {
    return BwmsVehicleObjectGetIsReversing(this);
}

@addMethod(VehicleObject)
public func GetBurnout() -> Float {
    return BwmsVehicleObjectGetBurnout(this);
}
