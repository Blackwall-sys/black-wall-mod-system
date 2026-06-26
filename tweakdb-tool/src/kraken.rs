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

/// Magic `KARK` (bytes "KARK") que prefixa um segmento Oodle/Kraken.
pub const KARK_MAGIC: u32 = 0x4B52_414B;

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

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "kraken"))]
    #[test]
    fn indisponivel_sem_a_feature() {
        use super::*;
        assert!(matches!(decompress(&[1, 2, 3], 8), Err(KrakenError::Unavailable)));
    }
}
