//! Carregador de plugin NATIVO em Rust — frida-free, 100% nosso.
//!
//! Escaneia `<jogo>/red4ext/plugins/*.dylib`, faz `dlopen` e chama o entry
//! `bwms_plugin_main(api_version) -> i32`. Um "mod em Rust" = um cdylib com esse
//! entry, dropado nessa pasta.
//!
//! Filosofia de segurança (pra NÃO afetar o Lua/jogo do usuário comum):
//!  - OPT-IN: se a pasta `plugins/` não existe ou está vazia, não faz NADA.
//!  - ISOLADO: dlopen/entry com guarda — plugin que falha/panica é logado e
//!    ignorado; o core (console/cheats/Lua) segue intacto.
//!
//! ALFA: a API que o plugin recebe ainda é mínima (só roda o entry). Expor RTTI/
//! register/ImGui pro plugin é roadmap dos próximos patches.

use std::ffi::{c_void, CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::Path;

extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}
const RTLD_NOW: c_int = 2;

/// Versão da ABI passada pro plugin (cresce quando a API crescer).
pub const BWMS_PLUGIN_API: u32 = 1;
type PluginEntry = unsafe extern "C" fn(api_version: u32) -> i32;

/// Carrega todos os plugins Rust (.dylib) da pasta. Chamar 1x no boot.
/// Retorna quantos foram carregados. Pasta inexistente/vazia = 0 (sem efeito).
pub fn load_plugins(dir: &Path) {
    if !dir.is_dir() {
        return; // opt-in: sem pasta = nada a fazer (zero impacto)
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut n = 0usize;
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("dylib") {
            continue;
        }
        if load_one(&path) {
            n += 1;
        }
    }
    if n > 0 {
        crate::log(&format!("[plugins] {n} plugin(s) Rust carregado(s) de {}", dir.display()));
    }
}

fn load_one(path: &Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string();
    let cpath = match CString::new(path.to_string_lossy().as_bytes()) {
        Ok(c) => c,
        Err(_) => return false,
    };
    // ISOLADO: qualquer falha/panic aqui é contida — o core não cai.
    std::panic::catch_unwind(|| unsafe {
        let h = dlopen(cpath.as_ptr(), RTLD_NOW);
        if h.is_null() {
            let e = dlerror();
            let msg = if e.is_null() {
                "erro desconhecido".to_string()
            } else {
                CStr::from_ptr(e).to_string_lossy().into_owned()
            };
            crate::log(&format!("[plugins] dlopen falhou em '{name}': {msg}"));
            return false;
        }
        let sym = CString::new("bwms_plugin_main").unwrap();
        let p = dlsym(h, sym.as_ptr());
        if p.is_null() {
            // carregou (seus constructors rodaram), mas sem o entry padrão — tudo bem.
            crate::log(&format!("[plugins] '{name}' carregado (sem entry 'bwms_plugin_main')"));
            return true;
        }
        let entry: PluginEntry = std::mem::transmute(p);
        let rc = entry(BWMS_PLUGIN_API);
        crate::log(&format!("[plugins] '{name}' bwms_plugin_main -> {rc}"));
        true
    })
    .unwrap_or_else(|_| {
        crate::log(&format!("[plugins] '{name}' PANIC no carregamento (isolado — core ok)"));
        false
    })
}
