// Codeware #39 — `localizationPersistenceOnScreenEntry`, a declaração do vendorizado do Codeware, verbatim, em ESCOPO GLOBAL.
//
// Layout conferido no `RED4ext.SDK/.../localization/PersistenceOnScreenEntry.hpp`: `{primaryKey:u64@0x00, secondaryKey:CString@0x08,
// femaleVariant:CString@0x28, maleVariant:CString@0x48}`, `ASSERT_SIZE 0x68` (`CString` = 0x20). Sem `module` de propósito: declaração
// NATIVA dentro de `module X.Y` ganha o prefixo no `fullName`, o bind procura `Codeware.Localization.localizationPersistenceOnScreenEntry`,
// nome que a RTTI do jogo não conhece, e o jogo trava no boot.
public native struct localizationPersistenceOnScreenEntry {
  public native let primaryKey: Uint64;
  public native let secondaryKey: String;
  public native let femaleVariant: String;
  public native let maleVariant: String;
}
