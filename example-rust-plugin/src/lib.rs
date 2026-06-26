// Exemplo de PLUGIN do BWMS em Rust nativo (100% nosso, sem libs externas).
//
// Como funciona: o runtime do BWMS escaneia <jogo>/red4ext/plugins/*.dylib no boot,
// faz dlopen e chama `bwms_plugin_main(api_version)`. Seu plugin roda DENTRO do
// processo do jogo, junto do runtime.
//
// Build:
//     cd example-rust-plugin
//     cargo build --release
//     cp target/release/libbwms_plugin_example.dylib  "<jogo>/red4ext/plugins/"
// Reabra o jogo: o BWMS carrega e chama o entry (veja /tmp/cp77-console.log).
//
// ALFA: a API que o plugin recebe ainda é mínima (só o entry + o que você mesmo
// fizer em Rust dentro do processo). RTTI/register/ImGui expostos pro plugin = roadmap.

#[no_mangle]
pub extern "C" fn bwms_plugin_main(api_version: u32) -> i32 {
    // Prova de vida: aparece no log do BWMS. Troque por sua lógica em Rust.
    eprintln!("[meu-plugin-rust] ola do BWMS! api_version={api_version}");
    0 // 0 = ok
}
