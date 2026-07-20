//! build.rs — compila o decodificador Kraken (`ooz`) e o linka quando a feature
//! `kraken` está ativa. Sem a feature, é um no-op (o crate continua zero-deps e
//! puro-`std`). Não usa o crate `cc`: invoca o `clang++` direto, então o build
//! com `--features kraken` é autocontido (só precisa de clang + a fonte do ooz).
//!
//! Fonte do ooz: `$OOZ_DIR` ou, por padrão, `../ooz` (clonado de powzix/ooz e
//! adaptado para macOS/clang/arm64 — ver `ooz/stdafx.h` e `ooz/sse2neon.h`).

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // Só faz algo quando a feature `kraken` está ligada.
    if env::var_os("CARGO_FEATURE_KRAKEN").is_none() {
        return;
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let ooz_dir = env::var_os("OOZ_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("..").join("ooz"));

    let sources = ["kraken.cpp", "lzna.cpp", "bitknit.cpp"];
    for src in &sources {
        let p = ooz_dir.join(src);
        if !p.exists() {
            panic!(
                "feature `kraken` ativa mas a fonte do ooz não foi encontrada em {}. \
                 Clone powzix/ooz lá ou aponte OOZ_DIR.",
                p.display()
            );
        }
        println!("cargo:rerun-if-changed={}", p.display());
    }
    let shim = manifest.join("ooz_shim.cpp");
    println!("cargo:rerun-if-changed={}", shim.display());
    println!("cargo:rerun-if-changed={}", ooz_dir.join("stdafx.h").display());
    println!("cargo:rerun-if-env-changed=OOZ_DIR");

    // `arm64` para Apple Silicon, `x86_64` caso contrário.
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok(other) => Box::leak(other.to_string().into_boxed_str()),
        Err(_) => "arm64",
    };
    let cxx = env::var("CXX").unwrap_or_else(|_| "clang++".to_string());

    let mut objects = Vec::new();
    for src in sources.iter().map(|s| ooz_dir.join(s)).chain([shim.clone()]) {
        let obj = out_dir.join(format!(
            "{}.o",
            src.file_stem().unwrap().to_string_lossy()
        ));
        compile(&cxx, arch, &ooz_dir, &src, &obj);
        objects.push(obj);
    }

    // Arquiva os objetos em libooz.a e manda o cargo linkar.
    let lib = out_dir.join("libooz.a");
    let mut ar = Command::new("ar");
    ar.arg("rcs").arg(&lib);
    for obj in &objects {
        ar.arg(obj);
    }
    run(&mut ar, "ar");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    // `-force_load` (não só `-l`/`link-lib=static`): o linker moderno do macOS (ld-prime,
    // Xcode 15+) as vezes não resolve símbolos de uma lib estática dependendo da ORDEM dos
    // argumentos na linha de comando final (que o cargo monta sozinho, variando conforme o
    // grafo de crates — passou a falhar depois de o pacote ganhar um `[lib]` além do `[[bin]]`,
    // 2026-07-15). `-force_load` inclui TODOS os símbolos do `.a` incondicionalmente, sem
    // depender de ordem — fix definitivo, não um workaround frágil.
    println!("cargo:rustc-link-arg=-Wl,-force_load,{}", lib.display());
    // Runtime C++ (libc++) para o que o ooz usa de C++.
    println!("cargo:rustc-link-lib=dylib=c++");
}

fn compile(cxx: &str, arch: &str, include: &Path, src: &Path, obj: &Path) {
    let mut cmd = Command::new(cxx);
    cmd.args(["-arch", arch])
        .args(["-std=c++14", "-O2", "-fPIC", "-w", "-c"])
        .arg("-I")
        .arg(include)
        .arg(src)
        .arg("-o")
        .arg(obj);
    run(&mut cmd, &format!("clang++ {}", src.display()));
}

fn run(cmd: &mut Command, what: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("falha ao executar {what}: {e}"));
    if !status.success() {
        panic!("{what} terminou com {status}");
    }
}
