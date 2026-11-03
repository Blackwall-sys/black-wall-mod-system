// ponte-vehiclesystem.reds — `ToggleGarageVehicle` chamável do redscript.
//
// `BwmsVehicleSystemToggleGarageVehicle` chama o `ToggleGarageVehicle` do motor (`VEHICLESYSTEM_TOGGLEGARAGEVEHICLE_VM`) com a ABI
// reconstruída (`tramp_vehiclesystem_toggle_garage_vehicle`, veiculos/mod.rs). Assinatura: em vez de um
// `GarageVehicleID` (`recordID: TweakDBID` + `name: CName`) por referência, vai o `recordID` direto e o `name` sai zerado do lado
// Rust — os usos conhecidos só preenchem o `recordID`.
//
// ⚠️ Muta estado real: ativa e desativa um veículo do garage do player. A chamada ativa não tem prova ao vivo (o endereço foi visto
// disparar só por observação).

native func BwmsVehicleSystemToggleGarageVehicle(sys: ref<VehicleSystem>, recordID: TweakDBID, enable: Bool) -> Bool

@addMethod(VehicleSystem)
public func BwmsToggleGarageVehicle(recordID: TweakDBID, enable: Bool) -> Bool {
    return BwmsVehicleSystemToggleGarageVehicle(this, recordID, enable);
}

// `GarageComponent.LoadSummonSetup()` relê o `Vehicle.summon_setup` do TweakDB para o componente da garagem (o motor só
// relê ao anexar). A classe existe no RTTI (`vehicleGarageComponent`) e o script vanilla da 2.5 não a declara; a native entra na visita do
// validador a ela (`veiculos::registrar_garagem`).
public importonly class GarageComponent extends GameComponent {
    public native func LoadSummonSetup()
}
