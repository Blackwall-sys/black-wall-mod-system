//! Codeware `App::DistrictResolver` (`PENDENCIAS-UNIFICADAS.md` item `#151`, RE 2026-08-11) —
//! resolve o distrito/área de Night City a partir do nome de uma cena/setor. Algoritmo 100% puro
//! da fonte real (`enablers/Codeware/src/App/World/DistrictResolver.hpp`): pula os 3 primeiros
//! bytes do nome (prefixo de categoria, ex. `"q1_"`), compara os 7 bytes seguintes como string
//! (o C++ compara o FNV1a64 deles; a chave é a mesma) com as chaves de uma tabela ESTÁTICA
//! hardcoded de 16 setores reais (a mesma nos dois mapas — `s_districtMap`/`s_areaMap` diferem só
//! no distrito de DESTINO, não nas 16 chaves). Zero endereço nativo — dado puro, portado
//! byte-a-byte. Valores de enum (`gamedataDistrict`) confirmados via `redscript-src/orphans.script`
//! (declaração real do enum, com os ordinais explícitos do jogo).

/// `gamedataDistrict` — só os ordinais realmente usados por esta tabela (`orphans.script:4749`).
mod district_ord {
    pub const ARROYO: u8 = 3;
    pub const BADLANDS: u8 = 12;
    pub const CHARTER_HILL: u8 = 30;
    pub const CITY_CENTER: u8 = 33;
    pub const COASTVIEW: u8 = 34;
    pub const CORPO_PLAZA: u8 = 42;
    pub const DOWNTOWN: u8 = 62;
    pub const GLEN: u8 = 65;
    pub const HEYWOOD: u8 = 71;
    pub const JAPAN_TOWN: u8 = 72;
    pub const KABUKI: u8 = 82;
    pub const LITTLE_CHINA: u8 = 88;
    pub const NORTH_OAKS: u8 = 99;
    pub const NORTHSIDE: u8 = 104;
    pub const PACIFICA: u8 = 110;
    pub const RANCHO_CORONADO: u8 = 111;
    pub const SANTO_DOMINGO: u8 = 117;
    pub const VISTA_DEL_REY: u8 = 123;
    pub const WATSON: u8 = 128;
    pub const WELLSPRINGS: u8 = 129;
    pub const WEST_WIND_ESTATE: u8 = 130;
    pub const WESTBROOK: u8 = 131;
    pub const INVALID: u8 = 133;
}

/// `s_districtMap`, `DistrictResolver.hpp:35-52` — setor→distrito (o bairro específico).
const DISTRICT_TABLE: [(&str, u8); 16] = {
    use district_ord::*;
    [
        ("bls_ina", BADLANDS),
        ("cct_cpz", CORPO_PLAZA),
        ("cct_dtn", DOWNTOWN),
        ("hey_gle", GLEN),
        ("hey_rey", VISTA_DEL_REY),
        ("hey_spr", WELLSPRINGS),
        ("pac_cvi", COASTVIEW),
        ("pac_wwd", WEST_WIND_ESTATE),
        ("std_arr", ARROYO),
        ("std_rcr", RANCHO_CORONADO),
        ("wat_kab", KABUKI),
        ("wat_lch", LITTLE_CHINA),
        ("wat_nid", NORTHSIDE),
        ("wbr_hil", CHARTER_HILL),
        ("wbr_jpn", JAPAN_TOWN),
        ("wbr_nok", NORTH_OAKS),
    ]
};

/// `s_areaMap`, `DistrictResolver.hpp:54-71` — setor→área (a região maior, "megazona").
const AREA_TABLE: [(&str, u8); 16] = {
    use district_ord::*;
    [
        ("bls_ina", BADLANDS),
        ("cct_cpz", CITY_CENTER),
        ("cct_dtn", CITY_CENTER),
        ("hey_gle", HEYWOOD),
        ("hey_rey", HEYWOOD),
        ("hey_spr", HEYWOOD),
        ("pac_cvi", PACIFICA),
        ("pac_wwd", PACIFICA),
        ("std_arr", SANTO_DOMINGO),
        ("std_rcr", SANTO_DOMINGO),
        ("wat_kab", WATSON),
        ("wat_lch", WATSON),
        ("wat_nid", WATSON),
        ("wbr_hil", WESTBROOK),
        ("wbr_jpn", WESTBROOK),
        ("wbr_nok", WESTBROOK),
    ]
};

/// Extrai a chave de 7 bytes (pula os 3 primeiros) — `None` se a string for curta demais
/// (`aName.ToString() + 3` num nome curto seria leitura fora dos limites em C++; aqui é seguro,
/// só devolve `None`, equivalente ao `Invalid` do C++ real quando `!aName`).
fn sector_key(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    std::str::from_utf8(&bytes[3..10]).ok()
}

fn resolve(name: &str, table: &[(&str, u8)]) -> u8 {
    match sector_key(name) {
        Some(key) => table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v).unwrap_or(district_ord::INVALID),
        None => district_ord::INVALID,
    }
}

/// `DistrictResolver::GetDistrict(CName) -> gamedataDistrict` — devolve o ordinal do enum (u8),
/// ou `INVALID` (133) se o nome não bater com nenhum dos 16 setores conhecidos.
pub fn get_district(name: &str) -> u8 {
    resolve(name, &DISTRICT_TABLE)
}

/// `DistrictResolver::GetArea(CName) -> gamedataDistrict` — mesma mecânica, mapa de destino diferente.
pub fn get_area(name: &str) -> u8 {
    resolve(name, &AREA_TABLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sector_key_pula_3_bytes_e_pega_7() {
        assert_eq!(sector_key("q1_wat_kab"), Some("wat_kab"));
        assert_eq!(sector_key("xyzbls_ina_resto_ignorado"), Some("bls_ina"));
    }

    #[test]
    fn sector_key_string_curta_devolve_none() {
        assert_eq!(sector_key("abc"), None);
        assert_eq!(sector_key(""), None);
    }

    #[test]
    fn get_district_todos_os_16_setores_reais() {
        assert_eq!(get_district("q1_bls_ina"), district_ord::BADLANDS);
        assert_eq!(get_district("q1_cct_cpz"), district_ord::CORPO_PLAZA);
        assert_eq!(get_district("q1_cct_dtn"), district_ord::DOWNTOWN);
        assert_eq!(get_district("q1_hey_gle"), district_ord::GLEN);
        assert_eq!(get_district("q1_hey_rey"), district_ord::VISTA_DEL_REY);
        assert_eq!(get_district("q1_hey_spr"), district_ord::WELLSPRINGS);
        assert_eq!(get_district("q1_pac_cvi"), district_ord::COASTVIEW);
        assert_eq!(get_district("q1_pac_wwd"), district_ord::WEST_WIND_ESTATE);
        assert_eq!(get_district("q1_std_arr"), district_ord::ARROYO);
        assert_eq!(get_district("q1_std_rcr"), district_ord::RANCHO_CORONADO);
        assert_eq!(get_district("q1_wat_kab"), district_ord::KABUKI);
        assert_eq!(get_district("q1_wat_lch"), district_ord::LITTLE_CHINA);
        assert_eq!(get_district("q1_wat_nid"), district_ord::NORTHSIDE);
        assert_eq!(get_district("q1_wbr_hil"), district_ord::CHARTER_HILL);
        assert_eq!(get_district("q1_wbr_jpn"), district_ord::JAPAN_TOWN);
        assert_eq!(get_district("q1_wbr_nok"), district_ord::NORTH_OAKS);
    }

    #[test]
    fn get_area_agrupa_varios_distritos_na_mesma_area() {
        // hey_gle/hey_rey/hey_spr -> 3 distritos DIFERENTES, mas a MESMA área (Heywood)
        assert_eq!(get_area("q1_hey_gle"), district_ord::HEYWOOD);
        assert_eq!(get_area("q1_hey_rey"), district_ord::HEYWOOD);
        assert_eq!(get_area("q1_hey_spr"), district_ord::HEYWOOD);
        // cct_cpz/cct_dtn -> CityCenter
        assert_eq!(get_area("q1_cct_cpz"), district_ord::CITY_CENTER);
        assert_eq!(get_area("q1_cct_dtn"), district_ord::CITY_CENTER);
        // bls_ina é o ÚNICO setor onde district==area (Badlands nos dois mapas)
        assert_eq!(get_area("q1_bls_ina"), district_ord::BADLANDS);
        assert_eq!(get_district("q1_bls_ina"), district_ord::BADLANDS);
    }

    #[test]
    fn nome_desconhecido_devolve_invalid() {
        assert_eq!(get_district("q1_xxx_yyy"), district_ord::INVALID);
        assert_eq!(get_area("q1_xxx_yyy"), district_ord::INVALID);
    }

    #[test]
    fn prefixo_de_3_bytes_e_ignorado_de_verdade() {
        // 2 prefixos DIFERENTES de 3 bytes, mesmo setor -> mesmo resultado (confirma que só os
        // bytes [3..10] importam, o prefixo em si é irrelevante pro algoritmo real)
        assert_eq!(get_district("abcwat_kab"), get_district("xyzwat_kab"));
        assert_eq!(get_district("abcwat_kab"), district_ord::KABUKI);
    }
}
