//! Carregador de plugin NATIVO em Rust — frida-free, 100% nosso.
//!
//! Escaneia `<jogo>/red4ext/plugins/*.dylib`, faz `dlopen` e chama o entry
//! `bwms_plugin_main(*const BwmsApi) -> i32`. Um "mod em Rust" = um cdylib com esse
//! entry, dropado nessa pasta.
//!
//! Filosofia de segurança (pra NÃO afetar o Lua/jogo do usuário comum):
//!  - OPT-IN: se a pasta `plugins/` não existe ou está vazia, não faz NADA.
//!  - ISOLADO: dlopen/entry com guarda — plugin que falha/panica é logado e
//!    ignorado; o core (console/cheats/Lua) segue intacto.
//!
//! O plugin recebe um `*const BwmsApi` (vtable C-ABI, ver `api.rs`): `log` + hook de vtable
//! (motor RED4ext) + reflection (`field_ptr`/`call_method` = Codeware). Inline hook /
//! `register_native` / ImGui = roadmap v2 (campos novos só no FIM da struct, `abi_version` cresce).

use crate::api::BwmsApi;
use std::ffi::{c_void, CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::Path;

extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}
const RTLD_NOW: c_int = 2;

/// Versão da ABI passada pro plugin (cresce quando a struct `BwmsApi` crescer).
/// v4: + register_native_argful (nativa COM ARGS) + fire_event (emitir evento do CallbackSystem).
/// v5: + prop_get_f32/prop_set_f32/prop_get_i32 (Reflection tipada por nome, sem ponteiro cru).
/// v6: + call_method_args (call de método COM ARGS tipados, parity com o callf interno).
/// v7: + register_method (método novo numa classe EXISTENTE, @addMethod-style — fecha
/// `red4ext-register-method-api`, até aqui só register_native/função-global era exposto).
/// v8: + tweakdb_get_flat/tweakdb_set_flat/tweakdb_clone_record (TweakDB — fecha
/// `tweakxl-mod-api`; expõe SetFlat escalar + clone-com-herança, já provados internamente,
/// como API C-ABI pra plugins/mods de 3os).
/// v9: + log_level (logger por-nível) + semver_satisfies (comparação de versão) — parte de
/// `red4ext-sdk-plumbing`. O resto do gap (`PluginInfo` via `bwms_plugin_query`, entry OPCIONAL
/// e ADITIVO — não muda `BwmsApi`/`abi_version`) fechado logo abaixo, mesma versão v9.
/// v10: + register_draw_callback/imgui_begin/imgui_text/imgui_end — plugin NÃO-lua desenha a
/// PRÓPRIA janela ImGui, sem linkar imgui-rs/cimgui (fecha `cet-imgui-thirdparty`). Callback
/// chamado 1x/frame dentro do onDraw (overlay.rs::render_imgui, mesmo ponto/gate que os mods
/// Lua — `overlay::in_draw()`, exige o overlay BWMS aberto).
pub const BWMS_PLUGIN_API: u32 = 11;
type PluginEntry = unsafe extern "C" fn(api: *const BwmsApi) -> i32;

/// Descrição do plugin (nome/autor/versão), preenchida pelo PRÓPRIO plugin via
/// `bwms_plugin_query` (OPCIONAL — completa o resto de `red4ext-sdk-plumbing`, a parte
/// "PluginHandle/PluginInfo" que faltava do v9). Buffers de tamanho fixo (não C-string
/// alocada) — evita qualquer questão de ownership/free cruzando a fronteira do ABI; o
/// plugin escreve os bytes UTF-8 + `\0` final, o loader lê até o primeiro `\0`.
#[repr(C)]
pub struct PluginInfo {
    pub name: [u8; 64],
    pub author: [u8; 64],
    pub version: [u8; 32],
}

impl PluginInfo {
    fn zeroed() -> Self {
        PluginInfo { name: [0; 64], author: [0; 64], version: [0; 32] }
    }
    fn field_str(buf: &[u8]) -> String {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }
}

/// Entry OPCIONAL: se o plugin exportar `bwms_plugin_query`, o loader chama ANTES de
/// `bwms_plugin_main`, preenchendo `info` — puramente informativo (log), não bloqueia o
/// carregamento se ausente ou se devolver false. Plugins v3-v9 SEM essa export continuam
/// carregando exatamente igual (checagem `is_null` antes de chamar).
type PluginQuery = unsafe extern "C" fn(info: *mut PluginInfo) -> bool;

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
        // Query OPCIONAL (PluginInfo — nome/autor/versão), só log, nunca bloqueia o carregamento.
        let qsym = CString::new("bwms_plugin_query").unwrap();
        let qp = dlsym(h, qsym.as_ptr());
        if !qp.is_null() {
            let query: PluginQuery = std::mem::transmute(qp);
            let mut info = PluginInfo::zeroed();
            if query(&mut info) {
                crate::log(&format!(
                    "[plugins] '{name}' info: nome='{}' autor='{}' versao='{}'",
                    PluginInfo::field_str(&info.name),
                    PluginInfo::field_str(&info.author),
                    PluginInfo::field_str(&info.version),
                ));
            }
        }
        let sym = CString::new("bwms_plugin_main").unwrap();
        let p = dlsym(h, sym.as_ptr());
        if p.is_null() {
            // carregou (seus constructors rodaram), mas sem o entry padrão — tudo bem.
            crate::log(&format!("[plugins] '{name}' carregado (sem entry 'bwms_plugin_main')"));
            return true;
        }
        let entry: PluginEntry = std::mem::transmute(p);
        let rc = entry(&crate::api::BWMS_API);
        crate::log(&format!("[plugins] '{name}' bwms_plugin_main(BwmsApi) -> {rc}"));
        true
    })
    .unwrap_or_else(|_| {
        crate::log(&format!("[plugins] '{name}' PANIC no carregamento (isolado — core ok)"));
        false
    })
}

#[cfg(test)]
mod tests {
    use super::PluginInfo;

    #[test]
    fn field_str_para_no_primeiro_nulo() {
        let mut buf = [0u8; 64];
        buf[..11].copy_from_slice(b"example-mod");
        assert_eq!(PluginInfo::field_str(&buf), "example-mod");
    }

    #[test]
    fn field_str_buffer_vazio() {
        let buf = [0u8; 32];
        assert_eq!(PluginInfo::field_str(&buf), "");
    }

    #[test]
    fn field_str_buffer_totalmente_cheio_sem_nulo() {
        // pior caso: string preenche o buffer inteiro, sem terminador — não deve travar/panicar.
        let buf = [b'x'; 16];
        assert_eq!(PluginInfo::field_str(&buf), "x".repeat(16));
    }
}
