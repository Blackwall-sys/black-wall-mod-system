//! Descompressão Kraken (o compressor dos segmentos `.archive` do jogo).
//!
//! O build público não descomprime: extrai só o que não precisa (segmentos `zsize == size` ou sem header
//! `KARK`) e conta o resto. A feature `kraken` só existe no crate de dev `bwms-dev/bwms-kraken`, que compila
//! este mesmo `main.rs` e linka `bwms_kraken_descomprimir`; ligada aqui, o link falha de propósito.
//!
//! Validação (dev): num archive real, o SHA1 de cada recurso descomprimido bate com o gravado no índice, e a
//! `usedhashes.kark` descomprime byte a byte.

/// Magic `KARK` (bytes "KARK") que prefixa um segmento Kraken.
pub const KARK_MAGIC: u32 = 0x4B52_414B;

#[derive(Debug)]
pub enum KrakenError {
    /// Build sem descompressor (o público).
    #[allow(dead_code)]
    Unavailable,
    /// O descompressor devolveu um tamanho diferente do esperado. Só construído no build de dev.
    #[allow(dead_code)]
    SizeMismatch { got: usize, want: usize },
}

impl std::fmt::Display for KrakenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KrakenError::Unavailable => write!(f, "recurso comprimido: este build não descomprime Kraken"),
            KrakenError::SizeMismatch { got, want } => {
                write!(f, "Kraken descomprimiu {got} bytes, esperado {want}")
            }
        }
    }
}

impl std::error::Error for KrakenError {}

/// `true` se o build descomprime Kraken.
pub fn is_available() -> bool {
    cfg!(feature = "kraken")
}

/// Descomprime `input` (stream Kraken cru, **sem** header KARK) para exatamente
/// `out_size` bytes.
pub fn decompress(input: &[u8], out_size: usize) -> Result<Vec<u8>, KrakenError> {
    #[cfg(feature = "kraken")]
    {
        let mut out = vec![0u8; out_size];
        // SAFETY: ponteiros/comprimentos de buffers Rust vivos; o descompressor só lê `input` e escreve até
        // `out_size` bytes em `out`.
        let written = unsafe {
            ffi::bwms_kraken_descomprimir(input.as_ptr(), input.len(), out.as_mut_ptr(), out.len())
        };
        if written < 0 || written as usize != out_size {
            return Err(KrakenError::SizeMismatch {
                got: written.max(0) as usize,
                want: out_size,
            });
        }
        Ok(out)
    }
    #[cfg(not(feature = "kraken"))]
    {
        let _ = (input, out_size);
        Err(KrakenError::Unavailable)
    }
}

#[cfg(feature = "kraken")]
mod ffi {
    extern "C" {
        /// Definida pelo crate de dev `bwms-kraken`. Retorna o nº de bytes descomprimidos (< 0 ou diferente do
        /// esperado em erro).
        pub fn bwms_kraken_descomprimir(src: *const u8, src_len: usize, dst: *mut u8, dst_len: usize) -> isize;
    }
}

/// Se a extração do segmento exige o descompressor. `first4` são os 4 primeiros bytes do segmento em disco (LE como lidos).
/// Não compara qual tamanho é maior: o cabeçalho `KARK` é quem diz que há fluxo comprimido.
pub fn segment_needs_kraken(zsize: u32, size: u32, first4: Option<u32>) -> bool {
    zsize != size && first4 == Some(KARK_MAGIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sem_kraken_quando_tamanhos_batem() {
        assert!(!segment_needs_kraken(10, 10, Some(KARK_MAGIC)));
    }

    #[test]
    fn sem_kraken_quando_nao_ha_header_kark() {
        assert!(!segment_needs_kraken(20, 5, Some(0x1234_5678)));
        assert!(!segment_needs_kraken(20, 5, None));
    }

    #[test]
    fn precisa_de_kraken_com_kark_e_tamanhos_diferentes() {
        assert!(segment_needs_kraken(20, 5, Some(KARK_MAGIC)));
    }

    #[cfg(not(feature = "kraken"))]
    #[test]
    fn indisponivel_sem_a_feature() {
        assert!(!is_available());
        assert!(matches!(decompress(&[1, 2, 3], 8), Err(KrakenError::Unavailable)));
    }

    #[cfg(feature = "kraken")]
    #[test]
    fn disponivel_com_a_feature() {
        assert!(is_available());
    }
}
