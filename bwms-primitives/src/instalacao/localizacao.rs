//! ArchiveXL `#45` — quais códigos de idioma o `.xl` pode declarar. Reimplementado da semântica de
//! `Localization/Language.cpp`. Nada copiado.
//!
//! Consumidor: `xl::parse_localization`, que tira do `.xl` todo código que não é idioma de jogador
//! antes de a tabela do runtime ser emitida.
//!
//! # 20 códigos, 19 idiomas: o `db-db` fica de fora de propósito
//!
//! Os mapas de path do original têm **20** códigos; o `IsKnown` lista **19**. O `db-db` (locale de
//! debug) resolve a partir de um path mas NÃO é idioma de jogador. Uniformizar em 20 faria o debug
//! ser aceito como idioma de verdade e, na tabela do runtime, virar a reserva do mod quando declarado
//! primeiro. A comparação diferencia caixa, como o `IsKnown` por CName do original: `PT-BR` não é
//! idioma.

/// Os 20 códigos que aparecem nos paths de localização.
pub const CODIGOS: [&str; 20] = [
    "ar-ar", "cz-cz", "db-db", "de-de", "en-us", "es-es", "es-mx", "fr-fr", "hu-hu", "it-it",
    "jp-jp", "kr-kr", "pl-pl", "pt-br", "ru-ru", "th-th", "tr-tr", "ua-ua", "zh-cn", "zh-tw",
];

/// O locale de DEBUG: tem path, mas não é idioma de jogador.
pub const CODIGO_DEBUG: &str = "db-db";

/// É um idioma de JOGADOR? O locale de debug responde `false` de propósito.
pub fn e_idioma_conhecido(codigo: &str) -> bool {
    codigo != CODIGO_DEBUG && CODIGOS.contains(&codigo)
}

#[cfg(test)]
mod testes {
    use super::*;

    /// A assimetria REAL: `db-db` está na lista de locales mas não é idioma de jogador.
    #[test]
    fn o_locale_de_debug_nao_e_idioma_de_jogador() {
        assert!(!e_idioma_conhecido("db-db"), "debug NAO e idioma de jogador");
        assert!(e_idioma_conhecido("pt-br"));
        assert!(!e_idioma_conhecido("xx-xx"));
    }

    #[test]
    fn conhecidos_sao_exatamente_19() {
        let n = CODIGOS.iter().filter(|c| e_idioma_conhecido(c)).count();
        assert_eq!(n, 19, "20 paths, 19 idiomas — a diferença é o db-db");
    }
}
