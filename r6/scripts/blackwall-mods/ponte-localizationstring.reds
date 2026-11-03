// ponte-localizationstring.reds — `LocalizationString`.
//
// Trivial: `CreateLocalizationString(value) -> {0, value}` /
// `ExtractLocalizationString(value) -> value.unk08`, sobre o struct nativo `LocalizationString{int64 unk00; CString unk08}`
// (0x28 bytes) — o mesmo tipo que a native vanilla `LocalizationStringComponent.GetString` devolve
// (`orphans.script:21668`), então `LocalizationString` é visível ao redscript neste build. Sem endereço nativo: leitura e escrita do
// `CString` em +0x08, de qualquer tamanho. Nomes do BWMS: `BwmsCreateLocalizationString`/`BwmsExtractLocalizationString`.
native func BwmsCreateLocalizationString(value: String) -> LocalizationString
native func BwmsExtractLocalizationString(value: LocalizationString) -> String

// Sugar opcional, nome que os mods chamam (só pra `String`->`LocalizationString`; sem `ToString`
// genérico pra evitar ambiguidade de overload com o `ToString` já usado em todo o projeto).
public func ToLocalizationString(value: String) -> LocalizationString = BwmsCreateLocalizationString(value)
