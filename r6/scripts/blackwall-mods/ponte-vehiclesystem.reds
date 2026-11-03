// ponte-vehiclesystem.reds — Codeware `Vehicle/VehicleSystem.reds` (item #72): `ToggleGarageVehicle` chamável do redscript e o
// `EnablePlayerVehicleID` do Codeware em cima dele.
//
// `BwmsVehicleSystemToggleGarageVehicle` chama o `ToggleGarageVehicle` do motor (`VEHICLESYSTEM_TOGGLEGARAGEVEHICLE_VM`) com a ABI
// reconstruída (`tramp_vehiclesystem_toggle_garage_vehicle`, veiculos/mod.rs). Diferença de assinatura: o Codeware passa um
// `GarageVehicleID` (`recordID: TweakDBID` + `name: CName`) por referência; aqui vai o `recordID` direto e o `name` sai zerado do lado
// Rust — o Codeware também só preenche o `recordID` nos usos conhecidos.
//
// ⚠️ Muta estado real: ativa e desativa um veículo do garage do player. A chamada ativa não tem prova ao vivo (o endereço foi visto
// disparar só por observação).

native func BwmsVehicleSystemToggleGarageVehicle(sys: ref<VehicleSystem>, recordID: TweakDBID, enable: Bool) -> Bool

@addMethod(VehicleSystem)
public func BwmsToggleGarageVehicle(recordID: TweakDBID, enable: Bool) -> Bool {
    return BwmsVehicleSystemToggleGarageVehicle(this, recordID, enable);
}

// Cópia quase verbatim de `enablers/Codeware/scripts/Vehicle/VehicleSystem.reds::
// EnablePlayerVehicleID` — mesma lógica (checa posse via `Vehicle.vehicle_list.list`, ativa/
// desativa, despawna se pedido), só trocando a chamada nativa pela nossa (`BwmsToggleGarageVehicle`
// em vez do `ToggleGarageVehicle(GarageVehicleID,...)` literal do Codeware).
@addMethod(VehicleSystem)
public func EnablePlayerVehicleID(vehicleID: TweakDBID, enable: Bool, opt despawnIfDisabling: Bool) -> Bool {
    let playerVehicles: array<TweakDBID> = TweakDBInterface.GetForeignKeyArray(t"Vehicle.vehicle_list.list");

    if !ArrayContains(playerVehicles, vehicleID) {
        return false;
    }

    let success: Bool = this.BwmsToggleGarageVehicle(vehicleID, enable);

    if success && !enable && despawnIfDisabling {
        let garageID: GarageVehicleID;
        garageID.recordID = vehicleID;
        this.DespawnPlayerVehicle(garageID);
    }

    return success;
}

// Codeware 1.21.1: `GarageComponent.LoadSummonSetup()` relê o `Vehicle.summon_setup` do TweakDB para o componente da garagem (o motor só
// relê ao anexar). A classe existe no RTTI (`vehicleGarageComponent`) e o script vanilla da 2.5 não a declara; a native entra na visita do
// validador a ela (`veiculos::registrar_garagem`).
public importonly class GarageComponent extends GameComponent {
    public native func LoadSummonSetup()
}
