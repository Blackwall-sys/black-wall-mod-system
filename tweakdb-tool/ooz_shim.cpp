// Ponte C estável para o decodificador do `ooz` (Kraken/Mermaid/Selkie/
// Leviathan + LZNA/BitKnit). O `Kraken_Decompress` do ooz é C++ (símbolo
// mangled); declaramo-lo com a mesma assinatura para casar no link e o
// reexpomos como `extern "C"` para o FFI do Rust.
#include <cstddef>
#include <cstdint>

// Definido em ooz/kraken.cpp (byte == unsigned char). Retorna o nº de bytes
// descomprimidos, ou um valor < 0 / != esperado em erro.
int Kraken_Decompress(const unsigned char *src, size_t src_len,
                      unsigned char *dst, size_t dst_len);

extern "C" long ooz_kraken_decompress(const uint8_t *src, size_t src_len,
                                      uint8_t *dst, size_t dst_len) {
    return Kraken_Decompress(src, src_len, dst, dst_len);
}
