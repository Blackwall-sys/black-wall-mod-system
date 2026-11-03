// bwms-district.reds — Codeware `DistrictResolver.GetDistrict`/`GetArea` (item #151): dado puro (tabela estática de 16
// setores reais + algoritmo de "pular 3 bytes, comparar 7"), sem endereço nativo (`mundo/distrito.rs` no Rust, com testes offline).
native func BwmsDistrictGetDistrict(sectorName: CName) -> gamedataDistrict
native func BwmsDistrictGetArea(sectorName: CName) -> gamedataDistrict
