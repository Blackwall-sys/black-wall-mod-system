// BWMS — TweakDBManager: a API que mods do Nexus chamam (`TweakDBManager.SetFlat/CreateRecord/
// UpdateRecord/RegisterName`) sobre o escritor de
// TweakDB do BWMS (`tweakdb/`).
//
// Classe `native` sem `extends` explícito: o Rust preenche os métodos (`register_facade_methods`) no validador do motor
// (`class_validate_probe_hook`, inicio/boot.rs), quando ele resolve "TweakDBManager".
//
//   SetFlat(id, value)     — escalar (Int32/Uint32/Float/Bool/TweakDBID), CName (o texto `LocKey#…` vira LocKey), ResRef, String e
//                            array de elemento de 8 bytes (`array:TweakDBID`, `array:CName`); array de elemento menor devolve `false`, e
//                            flat que o record não tem, ou de outro tipo, também.
//   CreateRecord(id, type) — `type` completo (`gamedataX_Record`), curto (`X`) ou `X_Record`; o record nasce com os flats da estrutura de
//                            um record vivo da classe, no valor padrão do tipo.
//   CloneRecord(id, base)  — deriva tipo e flats do `base` vivo pelos 2 TweakDBID numéricos (`tweak_db_id_derive`, CRC32
//                            continuado), sem precisar do nome do record.
//   UpdateRecord(id)
//   RegisterEnum(id)
//   RegisterName(name)
//   StartBatch()           — instância nova de `TweakDBBatch` (`rtti::new_object` + `rtti::make_handle`).
public abstract native class TweakDBManager {
  public final static native func SetFlat(id: TweakDBID, value: Variant) -> Bool;
  public final static native func CreateRecord(id: TweakDBID, type: CName) -> Bool;
  public final static native func CloneRecord(id: TweakDBID, base: TweakDBID) -> Bool;
  public final static native func UpdateRecord(id: TweakDBID) -> Bool;
  public final static native func RegisterEnum(id: TweakDBID) -> Bool;
  public final static native func RegisterName(name: CName) -> Bool;
  public final static native func StartBatch() -> ref<TweakDBBatch>;

  // Overloads por NOME (mod author escreve string, não TDBID cru).
  // `TDBID.Create` já é uma função redscript vanilla real (compila o TweakDBID em runtime a partir
  // da string, mesmo algoritmo CRC32|len<<32 que `tweak_db_id` replica no lado Rust) — não precisa
  // de nenhuma native nova pra esses overloads, só delega pro caminho por-ID acima.
  public final static func SetFlat(name: String, value: Variant) -> Bool {
    return TweakDBManager.SetFlat(TDBID.Create(name), value);
  }
  public final static func CreateRecord(name: String, type: CName) -> Bool {
    return TweakDBManager.CreateRecord(TDBID.Create(name), type);
  }
  public final static func CloneRecord(name: String, base: String) -> Bool {
    return TweakDBManager.CloneRecord(TDBID.Create(name), TDBID.Create(base));
  }
  public final static func UpdateRecord(name: String) -> Bool {
    return TweakDBManager.UpdateRecord(TDBID.Create(name));
  }
}
