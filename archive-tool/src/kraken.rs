//! Descompressão Kraken/Oodle.
//!
//! O payload comprimido dos `.archive` usa o compressor **Kraken** da Oodle. No
//! macOS isso é resolvido recompilando o open-source `ooz` (decode-only) para
//! arm64 — o `libkraken.dylib` shipado pelo WolvenKit é x86_64-only, e **não** é
//! preciso a Oodle DLL proprietária da Windows.
//!
//! Com a feature `kraken` (default), o `build.rs` compila o `ooz` de `../ooz`
//! (adaptado p/ clang/arm64 via `ooz/stdafx.h` + `sse2neon.h`) num `libooz.a` e
//! o linka; o FFI chama `ooz_kraken_decompress` (ponte de `ooz_shim.cpp`). Com
//! `--no-default-features`, esta função reporta indisponível e o tool extrai só
//! o que não precisa de Kraken (segmentos `zsize == size` ou sem header `KARK`).
//!
//! Validação: num archive real, o SHA1 de cada recurso descomprimido bate com o
//! gravado no índice, e a `usedhashes.kark` descomprime byte-a-byte.

use crate::archive::KARK_MAGIC;

#[derive(Debug)]
pub enum KrakenError {
    /// Nenhum backend Kraken compilado (feature `kraken` desligada).
    /// Só construído sem a feature `kraken`.
    #[allow(dead_code)]
    Unavailable,
    /// O backend rodou mas devolveu um tamanho diferente do esperado.
    /// Só construído com a feature `kraken` ativa.
    #[allow(dead_code)]
    SizeMismatch { got: usize, want: usize },
}

impl std::fmt::Display for KrakenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KrakenError::Unavailable => write!(
                f,
                "descompressor Kraken indisponível (recompile o ooz para arm64 e ative a feature `kraken`)"
            ),
            KrakenError::SizeMismatch { got, want } => {
                write!(f, "Kraken descomprimiu {got} bytes, esperado {want}")
            }
        }
    }
}

impl std::error::Error for KrakenError {}

/// `true` se há um backend Kraken funcional compilado.
pub fn is_available() -> bool {
    cfg!(feature = "kraken")
}

/// Descomprime `input` (stream Kraken cru, **sem** header KARK) para exatamente
/// `out_size` bytes.
pub fn decompress(input: &[u8], out_size: usize) -> Result<Vec<u8>, KrakenError> {
    #[cfg(feature = "kraken")]
    {
        let mut out = vec![0u8; out_size];
        // SAFETY: passamos ponteiros/comprimentos válidos de buffers Rust vivos;
        // o ooz só lê `input` e escreve até `out_size` bytes em `out`.
        let written = unsafe {
            ffi::ooz_kraken_decompress(
                input.as_ptr(),
                input.len(),
                out.as_mut_ptr(),
                out.len(),
            )
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
        /// Ponte de `ooz_shim.cpp` para o `Kraken_Decompress` do ooz. Retorna o
        /// nº de bytes descomprimidos (ou < 0 / != esperado em erro).
        /// (O `powzix/ooz` é decode-only — não há compressor.)
        pub fn ooz_kraken_decompress(
            src: *const u8,
            src_len: usize,
            dst: *mut u8,
            dst_len: usize,
        ) -> isize;
    }
}

/// Decide, dado um segmento (já mapeado em `FileSegment`), se a sua extração
/// exige Kraken. Replica a lógica de `Oodle.DecompressAndCopySegment` do
/// WolvenKit: só precisa de Kraken quando `zsize != size` **e** os primeiros 4
/// bytes em disco são o magic `KARK`. Caso contrário a saída é cópia crua.
///
/// `first4` são os 4 primeiros bytes do segmento em disco (LE como lidos).
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
