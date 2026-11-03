// bwms-soundbanks.reds — a parte de áudio do `ResourceLoadHook` no BWMS.
//
// O `Resource/Load` com o `soundbanks.json` teria de sair logo depois do `SoundBankManager::ReadSoundBanksJson`, com o que os ouvintes mudam nos
// bancos voltando pro mapa do motor. No Mac essa leitura roda ~3 s depois da carga da dylib, antes de qualquer script (boot 185), então nenhum ouvinte de
// script chegaria a tempo. Forma do BWMS: cada `RegisterCallback` de `Resource/Load` pede um reenvio, e no tick seguinte os ouvintes recebem um
// `Resource/Load` com o hash de `base\sound\event\soundbanks.json` no `GetPath()` e um `BwmsSoundBanks` no `GetResource()`. Os métodos valem só
// durante o evento: banco com nome existente recebe os campos novos e nome novo entra no mapa do motor. Os
// caminhos saem como hash (`Uint64`), a mesma forma do `ResourceEvent.GetPath`. Bancos residentes já carregados na inicialização não recarregam.
//
// Este arquivo no bundle liga, na carga da dylib, o hook do `ReadSoundBanksJson` que guarda o manager.

public native func BwmsSoundBankCount() -> Int32;

public native func BwmsSoundBankName(index: Int32) -> CName;

public native func BwmsSoundBankIsResident(index: Int32) -> Bool;

public native func BwmsSoundBankPath(index: Int32) -> Uint64;

public native func BwmsSoundBankSetResident(index: Int32, value: Bool) -> Bool;

public native func BwmsSoundBankSetPath(index: Int32, path: String) -> Bool;

public native func BwmsSoundBankAdd(name: CName, isResident: Bool, path: String) -> Int32;

public class BwmsSoundBanks extends IScriptable {
    public func GetCount() -> Int32 {
        let n: Int32 = BwmsSoundBankCount();
        return n;
    }

    public func GetName(index: Int32) -> CName {
        let nome: CName = BwmsSoundBankName(index);
        return nome;
    }

    public func IsResident(index: Int32) -> Bool {
        let residente: Bool = BwmsSoundBankIsResident(index);
        return residente;
    }

    public func GetPathHash(index: Int32) -> Uint64 {
        let caminho: Uint64 = BwmsSoundBankPath(index);
        return caminho;
    }

    public func SetResident(index: Int32, value: Bool) -> Bool {
        let ok: Bool = BwmsSoundBankSetResident(index, value);
        return ok;
    }

    public func SetPath(index: Int32, path: String) -> Bool {
        let ok: Bool = BwmsSoundBankSetPath(index, path);
        return ok;
    }

    public func Add(name: CName, isResident: Bool, path: String) -> Int32 {
        let indice: Int32 = BwmsSoundBankAdd(name, isResident, path);
        return indice;
    }
}
