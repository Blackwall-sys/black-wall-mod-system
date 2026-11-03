//! Carregador de plugin NATIVO em Rust — 100% nosso, sem injetor de terceiro.
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
//! (motor de hook do BWMS) + reflection (`field_ptr`/`call_method`) + inline hook,
//! `register_native`, janela no terminal e o resto listado em `BWMS_PLUGIN_API` (campos novos só no FIM da
//! struct, `abi_version` cresce).

use crate::mods_do_usuario::api::BwmsApi;
use std::ffi::{CStr, CString};
use std::os::raw::c_int;
use std::path::Path;

use crate::macos::{dlerror, dlopen, dlsym};
const RTLD_NOW: c_int = 2;

/// Versão da ABI passada pro plugin (cresce quando a struct `BwmsApi` crescer).
/// v4: + register_native_argful (nativa COM ARGS) + fire_event (emitir evento do CallbackSystem).
/// v5: + prop_get_f32/prop_set_f32/prop_get_i32 (Reflection tipada por nome, sem ponteiro cru).
/// v6: + call_method_args (call de método COM ARGS tipados, parity com o callf interno).
/// v7: + register_method (método novo numa classe EXISTENTE, @addMethod-style — até aqui só
/// register_native/função-global era exposto).
/// v8: + tweakdb_get_flat/tweakdb_set_flat/tweakdb_clone_record (TweakDB — expõe SetFlat escalar
/// + clone-com-herança, já provados internamente, como API C-ABI pra plugins/mods de 3os).
/// v9: + log_level (logger por-nível) + semver_satisfies (comparação de versão). O resto do gap (`PluginInfo` via `bwms_plugin_query`, entry OPCIONAL
/// e ADITIVO — não muda `BwmsApi`/`abi_version`) fechado logo abaixo, mesma versão v9.
/// v10: + register_draw_callback/janela_abrir/janela_texto/janela_fechar — plugin desenha a
/// PRÓPRIA janela no terminal, sem linkar nada. Callback chamado 1x/quadro com o terminal BWMS
/// aberto (`overlay::desenhar_terminal`); os slots `janela_*` abrem, escrevem e fecham a janela nos
/// widgets do terminal (`api::api_janela_*`).
/// v12: + add_game_state — callbacks de ciclo de vida (Running.OnEnter/Update/Exit + Shutdown.OnEnter)
/// mapeados nos sinais de presença do player já detectados.
/// v13: + `my_handle`/`inline_hook_owned`/`inline_detach`/`vtable_hook_owned`/`vtable_detach` —
/// handle de plugin e posse de hook. Cada plugin recebe uma CÓPIA
/// pessoal da API (`load_one` clona `BWMS_API` com um `my_handle` sequencial único) e as versões
/// `_owned`/`_detach` registram+checam o dono antes de desfazer: um plugin não derruba o hook de
/// outro. `inline_hook`/`inline_revert`/`trocar_slot_da_vtable`/`restaurar_slot_da_vtable` (sem
/// dono) seguem idênticos ao lado, pra compat.
/// v14: + `compare_semver_prerelease` (precedência SemVer 2.0 completa).
/// v15: + `launch_arg` — argumentos de lançamento do processo.
/// v16: + valor de campo de qualquer tipo pelo IType (`prop_type_name`, `prop_to_string`, `prop_get_string`, `prop_set_string`, `prop_copy`,
/// `prop_is_equal`) e genealogia e nome dos records de mod (`tweak_is_original`, `tweak_original_parent`,
/// `tweak_descendants`, `tweak_name`, `tweak_register_name`).
/// v17: + `class_create` e `class_add_property` — classe e propriedade nativas novas no carregamento do plugin, antes do bind; + `take_screenshot`.
pub const BWMS_PLUGIN_API: u32 = 17;
type PluginEntry = unsafe extern "C" fn(api: *const BwmsApi) -> i32;

/// Descarga do plugin — plugin recebe sinal ANTES do processo morrer.
/// Entry OPCIONAL `bwms_plugin_unload() -> Void`: se exportada, o handle+ponteiro são persistidos
/// aqui e `unload_all_plugins()` (chamada de `selfboot::exit_replacement`, ANTES do exit syscall —
/// motor ainda intacto, mesmo timing de `Session/End`/`BwmsOnGameShutdown`) invoca cada um, isolado
/// (`catch_unwind` por plugin, mesma filosofia de `load_one`). Plugins sem essa export carregam e rodam igual
/// (checagem `is_null` antes de registrar).
type PluginUnload = unsafe extern "C" fn();
static PLUGIN_UNLOAD_FNS: std::sync::Mutex<Vec<(String, PluginUnload)>> = std::sync::Mutex::new(Vec::new());

/// Chamado 1x de `exit_replacement`, ANTES do exit syscall. Isolado por plugin (um `unload` que
/// panica não impede os outros nem a saída limpa, que é o objetivo #1 daquele hook).
pub fn unload_all_plugins() {
    let fns = match PLUGIN_UNLOAD_FNS.lock() {
        Ok(g) => g.clone(),
        Err(_) => return,
    };
    for (name, f) in fns {
        let ok = std::panic::catch_unwind(|| unsafe { f() }).is_ok();
        crate::log_dev!("[plugins] '{name}' bwms_plugin_unload() -> {}", if ok { "OK" } else { "panic (isolado)" });
    }
}

/// Descrição do plugin (nome/autor/versão), preenchida pelo PRÓPRIO plugin via
/// `bwms_plugin_query` (OPCIONAL — a parte "PluginInfo" que faltava do v9). Buffers de tamanho fixo (não C-string
/// alocada) — evita qualquer questão de ownership/free cruzando a fronteira do ABI; o
/// plugin escreve os bytes UTF-8 + `\0` final, o loader lê até o primeiro `\0`.
///
/// `runtime` (PENDENCIAS-UNIFICADAS.md,
/// 2026-08-09): versão do JOGO que o plugin foi escrito pra suportar (`[major,minor,build,
/// revision]`) — campo NOVO no FIM da struct (aditivo,
/// mesma convenção já usada em `BwmsApi`/`abi_version`). `[0,0,0,0]` = não declarado (plugins
/// v9-anteriores continuam carregando idêntico, `PluginInfo::zeroed()` já garante isso antes
/// da `query()` escrever só os campos que conhece). Comparação via `compare_file_ver`, log
/// de AVISO em divergência — nunca bloqueia o carregamento (mesma filosofia "isolado" de todo
/// o mecanismo de `PluginInfo`, um mismatch de versão declarada não é motivo suficiente pra
/// negar a um usuário um plugin que pode funcionar mesmo assim).
#[repr(C)]
pub struct PluginInfo {
    pub name: [u8; 64],
    pub author: [u8; 64],
    pub version: [u8; 32],
    pub runtime: [u32; 4],
}

/// Versão do jogo que o BWMS suporta hoje (Steam 2.5.0, o `CFBundleShortVersionString` do `.app`) — a mesma que o gate do `on_load`
/// (`build_supported`) confere pelos bytes do executor. Representada como `[major, minor, build, revision]` só pra esta comparação.
const BWMS_GAME_FILEVER: [u32; 4] = [2, 5, 0, 0];

/// `compare_file_ver(lhs,rhs) -> int32`: comparação lexicográfica major→minor→build→revision.
/// -1 = lhs < rhs, 0 = igual, 1 = lhs > rhs (convenção padrão de comparador 3-way).
fn compare_file_ver(lhs: [u32; 4], rhs: [u32; 4]) -> i32 {
    for i in 0..4 {
        if lhs[i] < rhs[i] {
            return -1;
        }
        if lhs[i] > rhs[i] {
            return 1;
        }
    }
    0
}

impl PluginInfo {
    fn zeroed() -> Self {
        PluginInfo { name: [0; 64], author: [0; 64], version: [0; 32], runtime: [0; 4] }
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

/// `.dylib`s já carregados: a carga (`pipeline::carregar_plugins`) roda antes da validação de classes, no bind dos scripts (pra native de
/// plugin existir quando o `.reds` dela é ligado) e no 1º tick; cada chamada só pega o que ainda não veio.
static CARREGADOS: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Carrega os plugins Rust (.dylib) da pasta que ainda não estão em `CARREGADOS`. Pasta
/// inexistente/vazia = sem efeito.
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
        match CARREGADOS.lock() {
            Ok(mut c) if !c.contains(&path) => c.push(path.clone()),
            _ => continue,
        }
        if load_one(&path) {
            n += 1;
        }
    }
    if n > 0 {
        crate::log_dev!("[plugins] {n} plugin(s) Rust carregado(s) de {}", dir.display());
    }
}

fn load_one(path: &Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string();
    let cpath = match CString::new(path.to_string_lossy().as_bytes()) {
        Ok(c) => c,
        Err(_) => return false,
    };
    // ISOLADO: qualquer falha/panic aqui é contida — o core não cai.
    let result = std::panic::catch_unwind(|| unsafe {
        let h = dlopen(cpath.as_ptr(), RTLD_NOW);
        if h.is_null() {
            let e = dlerror();
            let msg = if e.is_null() {
                "erro desconhecido".to_string()
            } else {
                CStr::from_ptr(e).to_string_lossy().into_owned()
            };
            crate::log_dev!("[plugins] dlopen falhou em '{name}': {msg}");
            return Err(msg);
        }
        // Query OPCIONAL (PluginInfo — nome/autor/versão), só log, nunca bloqueia o carregamento.
        let qsym = CString::new("bwms_plugin_query").unwrap();
        let qp = dlsym(h, qsym.as_ptr());
        if !qp.is_null() {
            let query: PluginQuery = std::mem::transmute(qp);
            let mut info = PluginInfo::zeroed();
            if query(&mut info) {
                crate::log_dev!(
                    "[plugins] '{name}' info: nome='{}' autor='{}' versao='{}'",
                    PluginInfo::field_str(&info.name),
                    PluginInfo::field_str(&info.author),
                    PluginInfo::field_str(&info.version),
                );
                if info.runtime != [0, 0, 0, 0] {
                    let cmp = compare_file_ver(info.runtime, BWMS_GAME_FILEVER);
                    if cmp != 0 {
                        crate::log_dev!(
                            "[plugins] '{name}' AVISO: declara suportar o jogo v{}.{}.{}.{} — BWMS roda contra v{}.{}.{}.{} (não bloqueado, só avisado)",
                            info.runtime[0], info.runtime[1], info.runtime[2], info.runtime[3],
                            BWMS_GAME_FILEVER[0], BWMS_GAME_FILEVER[1], BWMS_GAME_FILEVER[2], BWMS_GAME_FILEVER[3],
                        );
                    }
                }
            }
        }
        // `bwms_plugin_unload` OPCIONAL — resolvido/persistido AQUI (não só localmente) pra
        // `unload_all_plugins()` conseguir chamar depois, no exit real.
        let usym = CString::new("bwms_plugin_unload").unwrap();
        let up = dlsym(h, usym.as_ptr());
        if !up.is_null() {
            let unload: PluginUnload = std::mem::transmute(up);
            if let Ok(mut fns) = PLUGIN_UNLOAD_FNS.lock() {
                fns.push((name.clone(), unload));
            }
            crate::log_dev!("[plugins] '{name}' exporta 'bwms_plugin_unload' — registrado pro exit real");
        }
        let sym = CString::new("bwms_plugin_main").unwrap();
        let p = dlsym(h, sym.as_ptr());
        if p.is_null() {
            crate::log_dev!("[plugins] '{name}' carregado (sem entry 'bwms_plugin_main')");
            return Ok(0i32);
        }
        let entry: PluginEntry = std::mem::transmute(p);
        // Posse de hook: cada plugin recebe uma CÓPIA PESSOAL da API,
        // idêntica em tudo exceto `my_handle` (sequencial, único) — a peça que faltava pra
        // `inline_hook_owned`/`vtable_hook_owned` saberem de quem é cada hook instalado, e
        // `inline_detach`/`vtable_detach` recusarem um plugin tentando soltar o hook de outro.
        // `Box::leak`: vive o processo inteiro (mesmo espírito de `BWMS_API` — ponteiro sempre
        // válido, mesmo se o plugin guardá-lo além desta chamada síncrona).
        let handle = crate::mods_do_usuario::api::next_plugin_handle();
        let my_api: &'static crate::mods_do_usuario::api::BwmsApi =
            Box::leak(Box::new(crate::mods_do_usuario::api::BwmsApi { my_handle: handle, ..crate::mods_do_usuario::api::BWMS_API }));
        // `PENDENCIAS-UNIFICADAS.md`: janela de registro restrita ao load —
        // as funções de registro do BwmsApi (register_native/register_method/
        // register_native_argful/add_game_state/register_draw_callback) só aceitam chamadas
        // DENTRO desta chamada síncrona. RAII (não par manual abre/fecha): fecha na saída do
        // escopo mesmo se `entry()` panicar (o panic é pego pelo `catch_unwind` de fora, mas o
        // Drop já rodou durante o desenrolamento — a janela nunca fica presa aberta).
        let _window = crate::mods_do_usuario::api::open_registration_window();
        let rc = entry(my_api);
        crate::log_dev!("[plugins] '{name}' bwms_plugin_main(BwmsApi, handle={handle}) -> {rc}");
        Ok(rc)
    });
    match result {
        Ok(Ok(rc)) if rc == 0 => {
            crate::mods_do_usuario::register_mod(crate::mods_do_usuario::ModStatus::Ok);
            true
        }
        Ok(Ok(_)) => {
            crate::mods_do_usuario::register_mod(crate::mods_do_usuario::ModStatus::Warning);
            true
        }
        Ok(Err(_)) => {
            crate::mods_do_usuario::register_mod(crate::mods_do_usuario::ModStatus::Error);
            false
        }
        Err(_) => {
            crate::log_dev!("[plugins] '{name}' PANIC no carregamento (isolado — core ok)");
            crate::mods_do_usuario::register_mod(crate::mods_do_usuario::ModStatus::Error);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{compare_file_ver, PluginInfo};

    #[test]
    fn compare_file_ver_igual() {
        assert_eq!(compare_file_ver([2, 31, 0, 0], [2, 31, 0, 0]), 0);
    }

    #[test]
    fn compare_file_ver_menor_no_minor() {
        assert_eq!(compare_file_ver([2, 30, 9, 9], [2, 31, 0, 0]), -1);
    }

    #[test]
    fn compare_file_ver_maior_no_major() {
        assert_eq!(compare_file_ver([3, 0, 0, 0], [2, 31, 0, 0]), 1);
    }

    #[test]
    fn compare_file_ver_desempata_por_build_depois_revision() {
        assert_eq!(compare_file_ver([2, 31, 1, 0], [2, 31, 0, 5]), 1);
        assert_eq!(compare_file_ver([2, 31, 0, 4], [2, 31, 0, 5]), -1);
    }

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
