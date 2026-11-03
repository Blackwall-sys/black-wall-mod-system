// Runtime 100% nosso — hook e sonda de memória são Rust nativo (src/ganchos e src/memoria).
// Nenhuma lib de instrumentação de terceiros é linkada. Só os frameworks de
// janela/overlay (Metal/QuartzCore/objc) + Foundation/AppKit.
fn main() {
    // resolv (presente no link original).
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
    println!("cargo:rerun-if-changed=build.rs");
}
