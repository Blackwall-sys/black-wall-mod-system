// Runtime 100% nosso — hook e sonda de memória são Rust nativo (src/gum.rs).
// Nenhuma lib de instrumentação de terceiros é linkada. Só os frameworks de
// janela/overlay (Metal/QuartzCore/objc) + Foundation/AppKit.
fn main() {
    // libc++ (imgui é C++) + resolv (presentes no link original).
    println!("cargo:rustc-link-lib=c++");
    println!("cargo:rustc-link-lib=resolv");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=AppKit");
    // overlay: Metal/QuartzCore (render no frame) + objc runtime (swizzle).
    println!("cargo:rustc-link-lib=framework=Metal");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
    println!("cargo:rustc-link-lib=objc");
    // LINKEDIT string pool desalinhado (ld-prime do Xcode 16) → dyld novo recusa
    // ('mis-aligned LINKEDIT string pool'). Remove as sub-tabelas que descolam o
    // alinhamento (não são necessárias pra carregar).
    println!("cargo:rustc-link-arg=-Wl,-no_function_starts");
    println!("cargo:rustc-link-arg=-Wl,-no_data_in_code_info");
    // Carimbo de build: identifica QUAL dylib está rodando. Sem isto, "o jogo pegou a versão
    // nova?" só dá pra responder comparando timestamps por fora — e errar isso já custou várias
    // rodadas de diagnóstico em cima de um binário velho. `rerun-if-changed` nos fontes garante
    // que o carimbo muda sempre que o código muda.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=BWMS_BUILD_STAMP={stamp}");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");
}
