//! Conversão de Windows FILETIME (intervalos de 100ns desde 1601-01-01 UTC),
//! como o RDAR grava o timestamp de cada recurso, para uma data ISO-8601 legível.
//! Sem dependências: usa o algoritmo de calendário de Howard Hinnant.

/// 1970-01-01 00:00:00 UTC expresso em FILETIME (100ns desde 1601).
const UNIX_EPOCH_IN_FILETIME: i64 = 116_444_736_000_000_000;

/// Formata um FILETIME como `YYYY-MM-DD HH:MM:SSZ`. Retorna `None` para 0 ou
/// negativo (recursos sem timestamp gravam 0).
pub fn filetime_to_iso(ft: i64) -> Option<String> {
    if ft <= 0 {
        return None;
    }
    let unix_100ns = ft - UNIX_EPOCH_IN_FILETIME;
    let secs = unix_100ns.div_euclid(10_000_000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    Some(format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}Z"))
}

/// Data civil (ano, mês, dia) a partir do número de dias desde 1970-01-01.
/// Algoritmo de Hinnant (`civil_from_days`), válido para datas proléticas.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_unix_bate() {
        assert_eq!(
            filetime_to_iso(UNIX_EPOCH_IN_FILETIME).as_deref(),
            Some("1970-01-01 00:00:00Z")
        );
    }

    #[test]
    fn zero_e_negativo_sem_data() {
        assert_eq!(filetime_to_iso(0), None);
        assert_eq!(filetime_to_iso(-5), None);
    }

    #[test]
    fn data_conhecida() {
        // 2021-12-10 00:00:00 UTC = 1639094400s unix.
        let ft = UNIX_EPOCH_IN_FILETIME + 1_639_094_400 * 10_000_000;
        assert_eq!(filetime_to_iso(ft).as_deref(), Some("2021-12-10 00:00:00Z"));
    }
}
