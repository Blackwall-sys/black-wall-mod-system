//! cp77-console — runtime de mods do Black Wall Mod System, 100% Rust (sem JS/Lua),
//! carregado pelo Cyberpunk 2077 do macOS via LC_LOAD_DYLIB.
//!
//! Os hooks usam uma biblioteca de instrumentação chamada de Rust (gum). Cobre o
//! console/RTTI, cheats, TweakDB, NativeSettings e o self-boot nativo.
#![allow(dead_code)] // esqueleto: vários itens só passam a ser usados com os hooks

mod ai;
mod api;
mod cname;
// `capture`: módulo de captura de frame/depth (experimental, uso interno). OFF por
// padrão = não entra na dylib pública. Liga com `--features capture`.
#[cfg(feature = "capture")]
mod capture;
mod camscan;
mod cet_json;
mod console;
mod crashreport;
mod gum;
// hooks (roteador de method-hook CET) e lua = só com a feature `lua`. Sem ela, stubs no-op
// mantêm os call sites do executor/overlay intactos e o core fica 0% Lua (sem luajit).
#[cfg(feature = "lua")]
mod hooks;
#[cfg(not(feature = "lua"))]
#[path = "hooks_stub.rs"]
mod hooks;
#[cfg(feature = "lua")]
mod lua;
#[cfg(not(feature = "lua"))]
#[path = "lua_stub.rs"]
mod lua;
mod overlay;
mod plugins;
mod register;
mod rtti;

mod selfboot;
mod selftest;
mod targets;
mod tweakdb_bake;
mod tweakdb_rt;

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};

// Constructor do macOS: roda quando a .dylib é carregada no processo do jogo.
// Sem a crate `ctor` — usa a seção __mod_init_func direto (zero-dep).
#[link_section = "__DATA,__mod_init_func"]
#[used]
static CTOR: extern "C" fn() = on_load;

extern "C" {
    fn _dyld_image_count() -> u32;
    fn _dyld_get_image_header(image_index: u32) -> *const c_void;
    fn _dyld_get_image_name(image_index: u32) -> *const std::os::raw::c_char;
}

/// Base de link do executável do jogo (__TEXT em 0x1_0000_0000).
const LINK_BASE: u64 = 0x1_0000_0000;

/// Endereço de carga do binário PRINCIPAL do jogo, achado por NOME (robusto —
/// `_dyld_get_image_vmaddr_slide(0)` devolveu o slide da NOSSA dylib, não o do
/// jogo, quando carregada via Module.load/dlopen).
fn game_base() -> usize {
    unsafe {
        let n = _dyld_image_count();
        for i in 0..n {
            let name = _dyld_get_image_name(i);
            if name.is_null() {
                continue;
            }
            if let Ok(s) = std::ffi::CStr::from_ptr(name).to_str() {
                if s.ends_with("/Cyberpunk2077") || s.ends_with("Contents/MacOS/Cyberpunk2077") {
                    return _dyld_get_image_header(i) as usize;
                }
            }
        }
        _dyld_get_image_header(0) as usize // fallback: imagem principal
    }
}

/// VM addr do binário (base 0x1_0000_0000) → endereço real em runtime.
///
/// Os offsets estáticos do projeto são da build STEAM. No GOG (mesma versão, layout
/// deslocado) traduz Steam-vmaddr → GOG-vmaddr via [`steam_to_gog`] ANTES de aplicar o
/// slide. Steam/Unknown = identidade (comportamento histórico, byte-idêntico).
pub(crate) fn rebase(vmaddr: u64) -> *mut c_void {
    let v = match game_build() {
        GameBuild::Gog => steam_to_gog(vmaddr).unwrap_or_else(|| {
            // Não mapeado no GOG: só addr DINÂMICO de probe/sweep dev chega aqui (todos
            // dev-gated + checam prólogo/readable antes de tocar). Loga e passa — nunca
            // crasha por isto; produção GOG só usa consts mapeadas.
            log(&format!("[rebase] vmaddr {vmaddr:#x} sem mapa GOG (dev/probe?) -> passthrough"));
            vmaddr
        }),
        _ => vmaddr, // Steam + Unknown = identidade
    };
    (game_base() + (v - LINK_BASE) as usize) as *mut c_void
}

/// Inverso de `rebase`: ponteiro runtime → VM addr estático do binário (p/ casar com
/// `nm`/símbolos no diagnóstico de vtable). 0 se o ponteiro estiver fora do módulo.
pub(crate) fn un_rebase(ptr: *const c_void) -> u64 {
    let p = ptr as usize;
    let b = game_base();
    if p < b {
        return 0;
    }
    (p - b) as u64 + LINK_BASE
}

/// Qual build do jogo. Steam e GOG = MESMA versão/instruções, layout diferente → os
/// vmaddr estáticos deslocam. Detectado 1x lendo o prólogo do executor; auto-validante.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GameBuild {
    Steam,
    Gog,
    Unknown,
}

/// Prólogo do executor (`stp x28,x27..; stp x26,x25..`). IGUAL nos 2 builds — só o
/// ENDEREÇO muda → serve de assinatura pra identificar o build. (= selfboot::EXEC_PROLOGUE.)
const DETECT_EXEC_PROLOGUE: u64 = 0xa901_67fa_a9ba_6ffc;
const STEAM_EXEC_VM: u64 = 0x1_0217_3120;
const GOG_EXEC_VM: u64 = 0x1_027b_a1b4;

/// Build detectado (cacheado). Lê o prólogo do executor no vmaddr de cada build até casar.
/// Unknown (nenhum casou) → tratado como Steam na tradução; os checks de prólogo por-hook
/// abortam limpos se o endereço não bater (sem crash).
pub(crate) fn game_build() -> GameBuild {
    use std::sync::OnceLock;
    static BUILD: OnceLock<GameBuild> = OnceLock::new();
    *BUILD.get_or_init(|| unsafe {
        let base = game_base();
        let probe = |vm: u64| -> bool {
            let p = (base + (vm - LINK_BASE) as usize) as *const c_void;
            gum::is_readable(p, 8) && (p as *const u64).read_unaligned() == DETECT_EXEC_PROLOGUE
        };
        let b = if probe(STEAM_EXEC_VM) {
            GameBuild::Steam
        } else if probe(GOG_EXEC_VM) {
            GameBuild::Gog
        } else {
            GameBuild::Unknown
        };
        log(&format!("[build] detectado: {b:?} (game_base={base:#x})"));
        b
    })
}

/// Flag: a versão do jogo não é reconhecida (nem Steam nem GOG v2.31). O overlay pode mostrar isto.
pub(crate) static BUILD_UNSUPPORTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// True se o build é reconhecido (Steam ou GOG). `Unknown` = versão do jogo que os nossos offsets
/// NÃO cobrem (auto-atualização além de 2.31, distribuição diferente) → aplicar os endereços
/// Steam-identity num binário desconhecido lê/escreve memória errada e crasha. O gate no `on_load`
/// usa isto pra NÃO instalar nenhum hook de endereço (o jogo boota vanilla), em vez de arriscar.
pub(crate) fn build_supported() -> bool {
    !matches!(game_build(), GameBuild::Unknown)
}

/// Mapa Steam-vmaddr → GOG-vmaddr. Mesma versão, só relayout; delta NÃO uniforme. Cada
/// par verificado por símbolo/pattern-único/disasm (notes: cross-build Steam↔GOG).
/// `None` = não mapeado (só addr dinâmico de probe/sweep DEV).
fn steam_to_gog(s: u64) -> Option<u64> {
    Some(match s {
        0x1_0217_3120 => 0x1_027b_a1b4, // EXEC (executor universal)
        0x1_0218_8e8c => 0x1_027c_ff28, // CRTTISystem::Get
        0x1_0219_5024 => 0x1_027d_bfd0, // GetFunction (vtbl+0x30)
        0x1_021f_cee0 => 0x1_0284_4d48, // bind orchestrator (resolve-log)
        0x1_021e_897c => 0x1_0282_fb34, // bind orch entry
        0x1_021e_8c84 => 0x1_0282_fe3c, // bind resolve-loop
        0x1_0002_2808 => 0x1_0002_59a8, // PoolDefault::AllocateAligned (símbolo)
        0x1_0002_2cb0 => 0x1_0002_5e50, // PoolDefault::Free (símbolo)
        0x1_0345_28e8 => 0x1_026d_608c, // CNamePool::Get
        0x1_02b7_3c7c => 0x1_026a_e764, // TweakDB::Get
        0x1_080c_92d0 => 0x1_07d1_2620, // TweakDB singleton (__bss)
        0x1_026b_8db8 => 0x1_021f_3638, // CreateRecord
        0x1_02b7_63fc => 0x1_026b_0ee4, // RecordExists
        0x1_0001_d9a0 => 0x1_0002_0b40, // PoolRoot::GetHandle (RELOC_TARGET, símbolo)
        0x1_03e2_f17c => 0x1_039d_e198, // PoolArchive::Allocate (símbolo)
        0x1_03ed_96b0 => 0x1_0179_ba38, // InitializeArchives
        0x1_03e2_ebd4 => 0x1_039d_dbf0, // open-archive
        0x1_03ed_a898 => 0x1_0179_cc20, // RequestResource (cand)
        0x1_06f5_01c0 => 0x1_06f5_41c0, // depot vtable (__DATA_CONST)
        0x1_021c_5858 => 0x1_0280_cc38, // RESLINK (ResourcePath→ref)
        0x1_03f7_0740 => 0x1_00a3_d4e0, // boot phase dispatcher (skip-intro)
        0x1_03f5_ec74 => 0x1_00a2_a7b4, // phase getter (GameSessionDesc+0x84)
        0x1_03f5_ec7c => 0x1_00a2_a7bc, // phase getter vizinha (+8, dev measure)
        0x1_0908_b798 => 0x1_08e8_c088, // OPCODE_TABLE (native-com-args; âncora funcOperatorAdd<int>)
        0x1_0900_3000 => 0x1_0910_6a88, // DEPOT_SINGLETON (dev; âncora depot-accessor)
        _ => return None,
    })
}

pub(crate) fn log(msg: &str) {
    // Build PÚBLICO (sem feature `devlog`): silencioso — não escreve /tmp/cp77-console.log nem
    // trace.log (o usuário final não precisa dos diagnósticos; um mod escrevendo em /tmp a cada
    // frame é comportamento desnecessário). Dev liga via `devtools`. (As STRINGS de debug em si
    // seguem no binário — removê-las exige trocar os 324 call-sites por uma macro gateada; próximo
    // passo, precisa boot-test. Aqui já matamos o I/O e deixamos a feature funcional.)
    #[cfg(not(feature = "devlog"))]
    {
        let _ = msg;
        return;
    }
    #[cfg(feature = "devlog")]
    {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/cp77-console.log")
    {
        let _ = writeln!(f, "{msg}");
    }
    // Em dev, ESPELHA no trace.log — sink confiável p/ diagnóstico de 1 ciclo: o console.log é
    // zerado ao abrir o console in-game (perde prints), o trace.log só zera no boot. Junta num
    // lugar só: loads de mod, prints de Lua (cheats/Cron), erros — pra ler tudo de uma vez.
    if dev_mode() {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/cp77-trace.log")
        {
            let _ = writeln!(f, "{msg}");
        }
    }
    } // fim do bloco #[cfg(feature = "devlog")]
}

/// Modo-dev: liga diagnósticos verbosos (trace.log volumoso, logs de registro de hook, o
/// export `cp77-watch.txt` da era frida). Default OFF → jogo LIMPO, registro "debaixo dos
/// panos". Liga com env `BWMS_DEV` ou tocando `/tmp/bwms-dev` (sem mexer no launch da Steam).
/// Checado 1x e cacheado.
pub(crate) fn dev_mode() -> bool {
    use std::sync::OnceLock;
    static DEV: OnceLock<bool> = OnceLock::new();
    *DEV.get_or_init(|| {
        std::env::var_os("BWMS_DEV").is_some() || std::path::Path::new("/tmp/bwms-dev").exists()
    })
}

/// Breadcrumb p/ crash nativo: SEMPRE sobrescreve /tmp/cp77-lasthook.txt com o ÚLTIMO ponto
/// (1 linha, barato — sobrevive a segfault, é o que diagnostica crash nativo). O trace.log
/// VOLUMOSO (sequência inteira, ~100KB/sessão) só é gravado em [`dev_mode`]: em jogo normal
/// não há a escrita nem o churn.
pub(crate) fn trace(msg: &str) {
    use std::io::Write;
    let _ = std::fs::write("/tmp/cp77-lasthook.txt", msg);
    if !dev_mode() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/cp77-trace.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}

extern "C" fn on_load() {
    // Dead-man's switch do lever BwmsFireStart — TEM que rodar antes de qualquer coisa que possa
    // levar o lever a disparar nesta sessão (ver selfboot::check_stale_boot_attempt).
    selfboot::check_stale_boot_attempt();
    // Relatório de crash (best-effort): se o boot ANTERIOR não fechou limpo (dead-man's switch acima)
    // E há um `.ips` recente do Cyberpunk, escreve `red4ext/bwms-crash-report.md` pro usuário colar
    // numa issue — substitui o `bwms-report.command` (executável assusta usuário de mod já marcado por
    // AV). BARATO quando não houve crash (sem marcador stale = retorna na hora). NÃO gateado por
    // build/devlog: o WRITE do .md tem que valer no build PÚBLICO (é o ponto). Roda ANTES dos gates
    // (INERT/versão) de propósito — o relatório é útil justamente quando algo deu errado, inclusive num
    // build não-reconhecido. Toda falha degrada em silêncio (panic=abort → código defensivo). Ver
    // crashreport.rs.
    crashreport::write_crash_report_if_crashed();
    // BISECT (2026-07-15): gate INERTE. Com BWMS_INERT=1 o dylib carrega mas NÃO instala nenhum
    // hook/registro/overlay/probe — isola se o crash SystemsUpdater-NULL (t=38s, determinístico)
    // vem do nosso código ATIVO ou do jogo/reds. Removido após o diagnóstico.
    if std::env::var("BWMS_INERT").is_ok() {
        log("[cp77-console] BWMS_INERT=1 — carregada mas 100% inerte (bisect do crash t=38s)");
        return;
    }
    // GATE GOLDEN DE VERSÃO (2026-07-19): se o build não é reconhecido (nem Steam nem GOG v2.31 —
    // o jogo auto-atualizou além da versão que os offsets cobrem, ou é uma distribuição diferente),
    // NÃO instala NENHUM hook de endereço. Aplicar os offsets Steam-identity num binário desconhecido
    // lê/escreve memória errada → crash antes do menu, em QUALQUER modo. O executor já aborta por
    // prólogo, mas os outros hooks always-on (bind/getfn/register) não têm essa guarda — então
    // desligamos tudo aqui. Degrada pro melhor caso possível: "BWMS não ativou, o jogo bootou vanilla".
    if !build_supported() {
        BUILD_UNSUPPORTED.store(true, Ordering::Relaxed);
        log("[cp77-console] versão do jogo NÃO reconhecida (nem Steam nem GOG v2.31) — BWMS não ativa nenhum hook (jogo boota normal). Atualize o BWMS para esta versão do jogo.");
        return;
    }
    if dev_mode() {
        let _ = std::fs::write("/tmp/cp77-trace.log", ""); // trace fresh por sessão (só dev)
    }
    log(&format!(
        "[cp77-console] carregada (Rust); game_base = {:#x}. Subindo thread do console.",
        game_base()
    ));
    // CPVR (dev): limpa o marcador .cpvr-ingame STALE de uma sessão anterior. Sem isso, o cpvr.js
    // (Frida) lê o marcador preso no startup → gameplayActive=true → captura no menu/boot → o boot
    // TRAVA na tela preta antes do menu. O cp77_tick recria o marcador só quando há player (gameplay).
    #[cfg(feature = "cpvr")]
    cpvr_clear_stale_ingame();
    // F-B: instala a ponte do bind orchestrator JÁ AQUI (topo do on_load), o mais cedo possível —
    // o bind do script (RedScriptsHost::Load → orchestrator @0x1021e897c) roda muito cedo, antes
    // do overlay/selfboot. É só patch de código (sem RTTI). Gated em ~/.bwms-bind-bridge.
    unsafe { selfboot::install_bind_bridge() };
    // F4 (Facade/CallbackSystem/Reflection): sonda OBSERVE-ONLY no validador de classe nativa
    // achado via RE 2026-07-12 (ver selfboot.rs). Gated ~/.bwms-classvalidate-probe, OFF por padrão.
    unsafe { selfboot::install_class_validate_probe() };
    // F4 (Facade): sonda OBSERVE-ONLY no orquestrador do assert "Failed to initialize scripts
    // data!" (baseEngineInit.cpp:1094) — dump de [engine+0x150], achado via RE OFFLINE 2026-07-13
    // (ver selfboot.rs). Gated ~/.bwms-initscripts-probe, OFF por padrão.
    unsafe { selfboot::install_initscripts_orch_probe() };
    // F4 (Facade): sonda OBSERVE-ONLY em 0x103d9622c — a função-wrapper cujo retorno vira
    // [engine+0x54] (o byte final que o orquestrador retorna; bit0==0 = ASSERT dispara, achado
    // via RE offline 2026-07-13). Loga os 4 args (container/flag/engine+0x90/count=10) + retorno.
    // Gated ~/.bwms-countcheck-probe, OFF por padrão.
    unsafe { selfboot::install_count_check_probe() };
    // `bindsig-probe` (2026-07-17): sonda OBSERVE-ONLY no validador "BindFunctionSignature"
    // (0x1021ea1b8) — a mesma janela de tempo (bind do script) dos outros probes F4 acima. Loga
    // o estado do cache `[type_ref+0x18]` pra retorno/params de cada função validada (ver
    // selfboot.rs pra RE completa). Gated ~/.bwms-bindsig-probe, OFF por padrão.
    unsafe { selfboot::install_bindsig_probe() };
    // `dynarraygrowth-probe` (2026-07-18): sonda OBSERVE-ONLY na rotina de crescimento de
    // container (`0x10096ca74`) que o crash-report da sessão bindsig-probe apontou como o SITE
    // REAL do crash de GetService (null-deref num invoke-thunk de alocador embutido em
    // container+0x28) — não o validador de tipo. Loga container ptr + estado do vtable do
    // alocador. Gated ~/.bwms-dynarraygrowth-probe, OFF por padrão.
    unsafe { selfboot::install_dynarraygrowth_probe() };
    // Tentativa 12 (Facade): o dispatcher que chama o validador de classe (0x1021fbf90) despacha
    // por "kind" (0/1/3/4/5); kind==1=classe (0x1021fc61c) já valida 100% das 2843 classes do
    // bundle após os fixes de hoje. A falha residual deve estar em kind==0 (0x1021fc1a4, NUNCA
    // examinado) — sonda OBSERVE-ONLY, loga só as chamadas que retornam 0 (falha). Gated
    // ~/.bwms-kind0-probe, OFF por padrão.
    unsafe { selfboot::install_kind0_probe() };
    // Tentativa 9 (Facade): hook do construtor PRIMÁRIO do CRTTISystem (0x102188634, achado por
    // disasm de CRTTISystem::Get — um call_once clássico). Forja o Codeware NO INSTANTE em que o
    // RTTI passa a existir — mais cedo que initscripts-probe (que já provamos rodar tarde
    // demais). Gated ~/.bwms-rttictor-probe, OFF por padrão.
    unsafe { selfboot::install_rtti_ctor_probe() };
    // Tentativa 10 (Facade): hook em GetOrRegisterType (0x1021885a4) — o helper que TODAS as
    // classes nativas do próprio motor usam pra se auto-registrar no RTTI. Forja o Codeware na
    // 2ª chamada em diante (a partir daí Get() já terminou sua própria construção — sem risco de
    // reentrância, ao contrário da Tentativa 9). Gated ~/.bwms-getorreg-probe, OFF por padrão.
    unsafe { selfboot::install_getorreg_type_probe() };
    // Overlay (janela in-game) — swizzle do present do Metal numa thread própria.
    overlay::start();
    // Self-boot do runtime (hook do executor). O selfboot tem ctor próprio, mas na build
    // `--features lua` ele não roda confiável (luajit muda a ordem do __mod_init_func) →
    // disparamos AQUI também, do `on_load` que SEMPRE roda. É idempotente (guard ACTIVE).
    unsafe { selfboot::selfboot_if_needed() };
    // `axl-pathb-injection-arbitrary`: instala o hook no append de archive AQUI, no `on_load`
    // SÍNCRONO (thread do jogo, mais cedo possível — ANTES do InitializeArchives/LoadGlobs do boot).
    // Instalar hook de CÓDIGO da nossa thread de heartbeat NÃO efetivou a escrita (v2 não capturou
    // nada); o copy-test que funciona instala do cp77_tick = thread do jogo. Ver `install_pathb_capture`.
    if let Ok(hh) = std::env::var("HOME") {
        if std::path::Path::new(&hh).join(".bwms-pathbtest").exists() {
            unsafe { install_pathb_capture() };
        }
    }
    // `axl-factories-apply` E2E: instala o HookAfter em LoadFactoryAsync CEDO (antes do factory-load) +
    // enfileira o NOSSO factory CUSTOM (base\zz_bwms\bwms_factory.csv, num archive injetado via pathb).
    // Prova o e2e: archive custom injetado (pathb) → factory custom re-injetado (LoadFactoryAsync após o
    // sentinel) → carrega do archive injetado. Gated ~/.bwms-facttest (dev). Ver `factory_replacement`.
    if let Ok(hh) = std::env::var("HOME") {
        if std::path::Path::new(&hh).join(".bwms-facttest").exists() {
            unsafe { crate::selftest::install_factory_hook() };
            crate::selftest::factory_add("base\\zz_bwms\\bwms_factory.csv");
        }
    }
    // HEARTBEAT de boot (diagnóstico do early-stick intermitente): thread que loga a cada 2s ONDE o boot
    // está, até chegar na gameplay. Se travar, a ÚLTIMA linha [hb] crava o ponto exato (t + fase). Custo ~0,
    // só loga no build dev (devlog). Para no player (gameplay) ou no cap ~30min.
    std::thread::spawn(|| {
        let t0 = std::time::Instant::now();
        let mut seen_eng = false;
        let mut last = (false, false, -2i32);
        // (hook do append pra pathb agora é instalado no on_load síncrono, thread do jogo.)
        for _ in 0..900u32 {
            // `axl-localization-apply` via a THREAD DO HEARTBEAT (2026-07-17): roda a cada 2s,
            // INDEPENDENTE do executor/cp77_tick/foco do jogo (que ficam intermitentes quando o jogo
            // está sem foco atrás de outras janelas). No menu o LocMgr já está pronto. Ver `prove_loc`.
            if !LOCTEST_DONE.load(Ordering::Relaxed) {
                if let Ok(hh) = std::env::var("HOME") {
                    let m = std::path::Path::new(&hh).join(".bwms-loctest");
                    if m.exists() && unsafe { prove_loc() } {
                        let _ = std::fs::remove_file(&m);
                        LOCTEST_DONE.store(true, Ordering::Relaxed);
                    }
                }
            }
            // (pathb: a injeção agora é automática no replace do InitializeArchives — thread do jogo,
            // depot real, com lock. Ver `init_archives_replacement`. Nada a fazer aqui.)
            let eng = overlay::engagement_active();
            if eng {
                seen_eng = true;
            }
            let player = !current_player().is_null();
            // GAMEPLAY REAL = a engagement JÁ apareceu E o player está vivo. Sem o gate `seen_eng`, a sonda
            // captura um objeto ESPÚRIO na fase-3 (asset-load) e dava falso-positivo de "gameplay" num boot travado.
            if seen_eng && player {
                log(&format!("[hb] t={}s GAMEPLAY (engagement+player) — boot ok", t0.elapsed().as_secs()));
                break;
            }
            // fase da sessão (GAME_SESSION_DESC+0x84, capturado pelo getter): 3=asset-load do boot, 1=engagement,
            // 2=initUser, 5=gameplay. Mostra ONDE travou — preso em phase=3 = thrash do asset-load (ambiente).
            let phase = unsafe {
                let sm = selfboot::GAME_SESSION_DESC.load(Ordering::Relaxed);
                if !sm.is_null() && gum::is_readable(sm as *const c_void, 0x85) {
                    (sm.add(0x84) as *const i8).read() as i32
                } else {
                    -1
                }
            };
            let t = t0.elapsed().as_secs();
            if t < 40 || (eng, seen_eng, phase) != last {
                let spur = if player { " (player-espúrio, sem engagement)" } else { "" };
                log(&format!("[hb] t={t}s eng={eng} seen_eng={seen_eng} phase={phase}{spur} (esperando gameplay)"));
            }
            last = (eng, seen_eng, phase);
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
    // Execução de comandos: NÃO numa thread nossa (instanciar item da thread
    // errada crasha) — a sonda chama `cp77_tick` de dentro do hook do executor,
    // que é a THREAD DO JOGO. (Esse mesmo mecanismo serve pro Observe/Override.)
}

/// Registry global, obtida 1x na thread do jogo. Só-leitura após init → OnceLock
/// (sem Mutex, p/ o Lua poder ler sem deadlock dentro do cp77_tick).
struct SendReg(rtti::Registry);
unsafe impl Send for SendReg {}
unsafe impl Sync for SendReg {}
static REG: std::sync::OnceLock<SendReg> = std::sync::OnceLock::new();
/// player/tx atuais (a sonda captura, o cp77_tick publica; o Lua lê via Game.*).
static CURRENT_PLAYER: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static CURRENT_TX: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) fn registry() -> Option<&'static rtti::Registry> {
    REG.get().map(|s| &s.0)
}
pub(crate) fn current_player() -> *mut c_void {
    CURRENT_PLAYER.load(Ordering::Relaxed)
}
pub(crate) fn set_current_player(p: *mut c_void) {
    CURRENT_PLAYER.store(p, Ordering::Relaxed);
}
pub(crate) fn set_current_tx(t: *mut c_void) {
    CURRENT_TX.store(t, Ordering::Relaxed);
}
pub(crate) fn current_tx() -> *mut c_void {
    CURRENT_TX.load(Ordering::Relaxed)
}

/// Heartbeat do runtime: o cp77_tick incrementa todo tick (na thread do jogo) só
/// quando há player/tx vivos. O badge do overlay lê isso → mostra "ativo" sem spam.
static TICKS: AtomicU64 = AtomicU64::new(0);
/// Quantos mods já foram carregados (loadmod) — exibido no badge.
static MODS_LOADED: AtomicUsize = AtomicUsize::new(0);
/// Auto-load dos mods (BWMS = Black Wall Mod System): dispara UMA vez quando o RTTI
/// fica pronto, pra a aba Mods/cheats vir ativa sem o usuário rodar `loadmods`.
static AUTO_LOADED: AtomicBool = AtomicBool::new(false);
static PLUGINS_LOADED: AtomicBool = AtomicBool::new(false);
static RESLINK_LOADED: AtomicBool = AtomicBool::new(false);
static FACTORIES_LOADED: AtomicBool = AtomicBool::new(false);
static RELOCREAL_DONE: AtomicBool = AtomicBool::new(false);
static ATTACHDETACH_DONE: AtomicBool = AtomicBool::new(false);
static DERIVETEST_DONE: AtomicBool = AtomicBool::new(false);
static LOCTEST_DONE: AtomicBool = AtomicBool::new(false);
// `axl-pathb-injection-arbitrary`: injeta um .archive fora do glob no content-group via um `replace`
// no InitializeArchives (0x103ed96b0). A replacement chama a original (constrói TODOS os archives do
// boot) e DEPOIS injeta — na THREAD DO JOGO, com o depot REAL (x0 do InitializeArchives, ≠ o singleton
// [0x109003000+0x1f8] que tem +0x10=NULL vivo — mesmo erro do cont.78), segurando o lock@depot+0x78,
// pré-streaming. Os offsets da RE (grupos@depot+0x10 stride 0x38, key@g+0x30, count@g+0xc, lock@+0x78)
// são CORRETOS pro `this` real. Ver HISTORICO cont.80/81, notes/RE-archiveinfo-inject-2026-07-17.md.
static INIT_ARCH_ORIG: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static PATHB_HOOK_ON: AtomicBool = AtomicBool::new(false);
static PATHB_INJECTED: AtomicBool = AtomicBool::new(false);
static COPYTEST_DONE: AtomicBool = AtomicBool::new(false);
static COPYTEST_ARMED: AtomicBool = AtomicBool::new(false);
static UPDATEREC_DONE: AtomicBool = AtomicBool::new(false);
/// Pasta padrão de mods. Sobreponível em runtime via `/tmp/cp77-mods-dir.txt`
/// (a sonda pode escrever o caminho certo no boot, p/ portabilidade).
pub(crate) fn mods_dir() -> String {
    // 1) override explícito (a sonda/dev pode fixar).
    if let Ok(s) = std::fs::read_to_string("/tmp/cp77-mods-dir.txt") {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    // 2) PORTÁVEL: <pasta da nossa dylib>/blackwall-mods (a dylib mora em <jogo>/red4ext/ ao lado
    //    de blackwall-mods/). Funciona em qualquer máquina. Se o subdir existir, usa direto; senão
    //    CONFIA na localização mesmo assim (a dylib SEMPRE carrega de red4ext/) — evita embutir um
    //    literal de path do jogo no binário (TRACELESS).
    if let Some(dir) = dylib_dir() {
        return format!("{dir}/blackwall-mods");
    }
    // 3) dladdr falhou (não deveria acontecer): caminho relativo ao cwd, sem embutir path do jogo.
    "red4ext/blackwall-mods".to_string()
}

/// Pasta onde a NOSSA dylib está carregada (via dladdr no próprio código). Base
/// pra resolver caminhos relativos (mods) de forma portável.
pub(crate) fn dylib_dir() -> Option<String> {
    #[repr(C)]
    struct DlInfo {
        dli_fname: *const i8,
        dli_fbase: *mut c_void,
        dli_sname: *const i8,
        dli_saddr: *mut c_void,
    }
    extern "C" {
        fn dladdr(addr: *const c_void, info: *mut DlInfo) -> i32;
    }
    unsafe {
        let mut info: DlInfo = std::mem::zeroed();
        if dladdr(mods_dir as *const c_void, &mut info) == 0 || info.dli_fname.is_null() {
            return None;
        }
        let path = std::ffi::CStr::from_ptr(info.dli_fname).to_string_lossy().into_owned();
        path.rfind('/').map(|i| path[..i].to_string())
    }
}
pub(crate) fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}
pub(crate) fn mods_loaded() -> usize {
    MODS_LOADED.load(Ordering::Relaxed)
}

/// Hotkeys (registerHotkey): char registrado → set p/ a sonda/overlay checar barato;
/// fila de chars pressionados (overlay empurra na main thread, cp77_tick drena na
/// thread do jogo e dispara o callback Lua).
static HOTKEY_CHARS: std::sync::Mutex<Option<std::collections::HashSet<char>>> =
    std::sync::Mutex::new(None);
static HOTKEY_PRESSED: std::sync::Mutex<Vec<char>> = std::sync::Mutex::new(Vec::new());
pub(crate) fn hotkey_register_char(c: char) {
    let mut g = HOTKEY_CHARS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(Default::default).insert(c);
}
pub(crate) fn hotkey_is(c: char) -> bool {
    HOTKEY_CHARS
        .lock()
        .map(|g| g.as_ref().map_or(false, |s| s.contains(&c)))
        .unwrap_or(false)
}
pub(crate) fn hotkey_press(c: char) {
    if let Ok(mut g) = HOTKEY_PRESSED.lock() {
        if g.len() < 32 {
            g.push(c);
        }
    }
}
fn hotkey_drain() -> Vec<char> {
    HOTKEY_PRESSED
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default()
}

/// registerInput: chars com bind de input (down/up) + fila de eventos (char, isDown).
static INPUT_CHARS: std::sync::Mutex<Option<std::collections::HashSet<char>>> =
    std::sync::Mutex::new(None);
static INPUT_EVENTS: std::sync::Mutex<Vec<(char, bool)>> = std::sync::Mutex::new(Vec::new());
/// RawInput do CallbackSystem: TODAS as teclas (JÁ MAPEADAS pro valor real de `EInputKey`, ver
/// `register::map_macos_keycode_to_einputkey`) + modificadores (shift/control/alt) capturados no
/// sendEvent (gameplay), pra emitir o evento "Input/Key" com um `ref<KeyInputEvent>` REAL
/// (2026-07-18, `cw-rawinput-realname` — antes era só o keycode cru como Int32; ≠ INPUT_EVENTS,
/// que é só das teclas registradas no registerInput).
static RAW_KEYS: std::sync::Mutex<Vec<(i32, bool, bool, bool)>> = std::sync::Mutex::new(Vec::new());
pub(crate) fn push_raw_key(key: i32, shift: bool, control: bool, alt: bool) {
    if let Ok(mut q) = RAW_KEYS.lock() {
        if q.len() < 64 {
            q.push((key, shift, control, alt));
        }
    }
}
fn drain_raw_keys() -> Vec<(i32, bool, bool, bool)> {
    RAW_KEYS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}
pub(crate) fn input_register_char(c: char) {
    let mut g = INPUT_CHARS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(Default::default).insert(c);
}
pub(crate) fn input_is(c: char) -> bool {
    INPUT_CHARS
        .lock()
        .map(|g| g.as_ref().map_or(false, |s| s.contains(&c)))
        .unwrap_or(false)
}
pub(crate) fn input_event(c: char, down: bool) {
    if let Ok(mut g) = INPUT_EVENTS.lock() {
        if g.len() < 64 {
            g.push((c, down));
        }
    }
}
fn input_drain() -> Vec<(char, bool)> {
    INPUT_EVENTS
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default()
}

/// Chamado pela sonda DE DENTRO do hook do executor (= thread do jogo), throttled.
/// Publica player/tx, executa o comando pendente E roda o lifecycle dos mods Lua
/// (onUpdate) — tudo na thread certa.
/// Guard de re-entrância do tick (ver cp77_tick). RAII reseta no fim de qualquer caminho.
static TICK_BUSY: AtomicBool = AtomicBool::new(false);
struct TickGuard;
impl Drop for TickGuard {
    fn drop(&mut self) {
        TICK_BUSY.store(false, Ordering::Relaxed);
    }
}

/// Versão do BWMS — escrita no splash de boot. Bumpar a cada release pro Nexus.
pub const BWMS_VERSION: &str = "0.1.3";

/// CPVR (dev): remove o marcador `.cpvr-ingame` stale no boot (1x, no on_load), pra o cpvr.js
/// começar com `gameplayActive=false` — sem capturar no menu/boot. O tick recria só com player vivo.
/// Sem isso, o arquivo persiste em disco entre sessões e todo boot herda o marcador preso.
#[cfg(feature = "cpvr")]
fn cpvr_clear_stale_ingame() {
    if let Some(red4) = std::path::Path::new(&mods_dir()).parent() {
        let _ = std::fs::remove_file(red4.join("logs").join(".cpvr-ingame"));
    }
}

/// CPVR (dev, opt-in): avisa o cpvr.js que estamos EM JOGO (player+tx vivos), pra a captura VR
/// armar só no gameplay — nunca no menu/boot. NÃO muda a captura, só o "quando". Age só na
/// TRANSIÇÃO (não toca disco todo tick) e só com CPVR ligado (~/.bwms-cpvr).
#[cfg(feature = "cpvr")]
fn cpvr_ingame_marker(in_game: bool) {
    static LAST: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);
    let want = in_game as i8;
    if LAST.swap(want, Ordering::Relaxed) == want {
        return; // sem mudança de estado → não mexe no disco
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() || !std::path::Path::new(&format!("{home}/.bwms-cpvr")).exists() {
        return; // CPVR desligado → não sinaliza
    }
    let red4 = match std::path::Path::new(&mods_dir()).parent() {
        Some(p) => p.to_path_buf(),
        None => return,
    };
    let marker = red4.join("logs").join(".cpvr-ingame");
    if in_game {
        let _ = std::fs::File::create(&marker);
    } else {
        let _ = std::fs::remove_file(&marker);
    }
}

#[no_mangle]
pub extern "C" fn cp77_tick() {
    // RE-ENTRÂNCIA: o tick roda DENTRO do hook do executor. Se um callback (Override/Cron/
    // console via call_func) re-entrar o executor e o módulo de tick bater de novo, NÃO
    // re-executa — senão a recursão exec→tick→…→exec→tick estoura a pilha e crasha (foi o
    // crash do `ovcall`). RAII (`_tg`) reseta o flag ao sair, em qualquer return.
    if TICK_BUSY.swap(true, Ordering::Relaxed) {
        return;
    }
    let _tg = TickGuard;
    // WATCHDOG anti-hang de boot (backstop): roda ANTES do gate de registry, todo tick, pra o
    // caso do getter parar de ser chamado mas o tick seguir vivo. Idempotente/barato (ver
    // selfboot::boot_hang_watchdog — só age se skip ligado + phase<=1 após 75s).
    selfboot::boot_hang_watchdog();
    // SKIP-INTRO: força da phase byte DESLIGADA — escrever phase=3 dispara assert do jogo e não
    // fecha o attract screen (camada paralela). Mantido só p/ referência. Ver notes/boot-flow.
    let _ = overlay::engagement_active;
    let _ = selfboot::force_pregame_menu;
    if REG.get().is_none() {
        if let Some(r) = unsafe { rtti::Registry::obtain() } {
            let _ = REG.set(SendReg(r));
        }
    }
    let reg = match registry() {
        Some(r) => r,
        None => return,
    };
    // F-B: re-tenta registrar as nativas do BWMS se o selfboot pegou o RTTI cedo demais
    // (idempotente via REGISTERED). Garante BlackwallPing no RTTI p/ o bind do redscript.
    unsafe { crate::register::register_all() };
    // TweakDB runtime: dump observe-only do singleton (gated ~/.bwms-tdbdump) p/ confirmar
    // o records-map in-vivo antes de registrar record novo (PASSO 0 do clone-runtime).
    unsafe { crate::tweakdb_rt::dump_once_if_marked() };
    // TweakXL clone runtime-reg (PASSO final): registra Items.BwmsCloneTest no TweakDB vivo
    // via a nativa CreateRecord (gated ~/.bwms-tdbcreate). RecordExists antes/depois = prova.
    unsafe { crate::tweakdb_rt::create_once_if_marked() };
    // IA Fase 0: poll non-blocking da resposta do processo externo (throttled). Loga/exibe.
    crate::ai::poll_response();
    // F-B: GetFunction (vtbl+0x30) = vmaddr estático 0x102195024 (descoberto). PARQUEADO — não é
    // o resolvedor do binder do redscript. Resumir = achar/hookar o resolvedor real (~0x2192xxx).
    // Resolve o FromTDBID UMA vez e publica o endereço-alvo: o hook do executor então
    // captura fn/ctx/ret nativamente quando o jogo o chama (substitui a sonda frida).
    if selfboot::active() && selfboot::FROMTD_TGT.load(Ordering::Relaxed).is_null() {
        let rf = unsafe { rtti::resolve_func(reg, "gameItemID", "FromTDBID") }
            .or_else(|| unsafe { rtti::resolve_func(reg, "ItemID", "FromTDBID") });
        if let Some(rf) = rf {
            selfboot::FROMTD_TGT.store(rf.func, Ordering::Relaxed);
        }
    }
    // Self-boot (modo nativo): o hook Rust já setou CURRENT_PLAYER/TX por captura
    // direta (sem /tmp). Trilha dev (injetor externo): lê os ponteiros do arquivo.
    let (player, tx) = if selfboot::active() {
        (current_player(), current_tx())
    } else {
        let (p, t) = read_inst();
        CURRENT_PLAYER.store(p, Ordering::Relaxed);
        CURRENT_TX.store(t, Ordering::Relaxed);
        (p, t)
    };
    // Auto-load dos mods (BWMS) na 1a vez com RTTI pronto — sem `loadmods` manual.
    // O loadmods não usa player/tx (só carrega os Lua + dispara onInit), então roda
    // mesmo no menu (onde player ainda é nulo) — aí o NativeSettings/cheats já
    // registram os hooks e a aba Mods vem ativa.
    if !AUTO_LOADED.swap(true, Ordering::Relaxed) {
        let dir = mods_dir();
        if std::path::Path::new(&dir).is_dir() {
            log(&format!("[bwms] auto-load de mods: {dir}"));
            load_mods_dir(&dir, true); // prod: pula testes/CPVR (carregáveis no manual)
        } else {
            log(&format!("[bwms] pasta de mods não achada p/ auto-load: {dir}"));
        }
    }
    // Carregador de plugin Rust (frida-free, OPT-IN): só age se houver .dylib em
    // red4ext/plugins/. Sem plugin = zero impacto (Lua/jogo do usuário intactos).
    if !PLUGINS_LOADED.swap(true, Ordering::Relaxed) {
        if let Some(red4) = std::path::Path::new(&mods_dir()).parent() {
            plugins::load_plugins(&red4.join("plugins"));
        }
    }
    // resource.link/copy: se um mod instalou pares (red4ext/bwms-reslink.txt, gerado pelo
    // mod-manager a partir do .xl), instala o hook (idempotente) + carrega a tabela. Produção:
    // só age se o arquivo existir (mod de link presente) — sem mod = zero impacto.
    if !RESLINK_LOADED.swap(true, Ordering::Relaxed) {
        if let Some(red4) = std::path::Path::new(&mods_dir()).parent() {
            let f = red4.join("bwms-reslink.txt");
            if f.is_file() {
                unsafe { crate::selftest::install_reslink() };
                crate::selftest::reslink_file(&f.to_string_lossy());
            }
        }
    }
    // `axl-e2e-wire-modmanager`: espelha o auto-load do reslink acima, mas pro `axl-factories-apply`
    // — `bwms-core::apply::apply_report` (chamado pelo `install` do mod-manager) já GERA
    // `red4ext/bwms-factories.txt` automaticamente a partir da seção `factories:` dos `.xl` ativos
    // (`write_factory_table`). Faltava só o RUNTIME carregar esse arquivo sozinho no boot — antes só
    // existia via o marcador de dev `~/.bwms-facttest` (`factory_add` manual). Produção: só age se o
    // arquivo existir (mod com factory presente) — sem mod = zero impacto.
    if !FACTORIES_LOADED.swap(true, Ordering::Relaxed) {
        if let Some(red4) = std::path::Path::new(&mods_dir()).parent() {
            let f = red4.join("bwms-factories.txt");
            if f.is_file() {
                unsafe { crate::selftest::install_factory_hook() };
                crate::selftest::factory_file(&f.to_string_lossy());
            }
        }
    }
    // `red4ext-reloc-universal` one-shot NO MENU: gated por `~/.bwms-relocreal`. Roda AQUI (antes do
    // gate de player) porque o alvo é um dtor de IA de NPC — dormente no menu (mundo não carregado),
    // colisão ~zero; em gameplay ele dispararia. Consome o marcador e roda 1×. Ver `prove_relocreal`.
    if let Ok(h) = std::env::var("HOME") {
        let m = std::path::Path::new(&h).join(".bwms-relocreal");
        if m.exists() && !RELOCREAL_DONE.swap(true, Ordering::Relaxed) {
            let _ = std::fs::remove_file(&m);
            unsafe { prove_relocreal() };
        }
    }
    // `red4ext-attach-detach-contract`: idem, one-shot no menu (2 hooks empilhados no MESMO alvo,
    // LIFO, Detach único). Ver `prove_attach_detach`.
    if let Ok(h) = std::env::var("HOME") {
        let m = std::path::Path::new(&h).join(".bwms-attachdetach");
        if m.exists() && !ATTACHDETACH_DONE.swap(true, Ordering::Relaxed) {
            let _ = std::fs::remove_file(&m);
            unsafe { prove_attach_detach() };
        }
    }
    // `tweakxl-registername`: derive nativo PURO (roda no MENU, não precisa player). Ver `prove_derive`.
    if let Ok(h) = std::env::var("HOME") {
        let m = std::path::Path::new(&h).join(".bwms-derivetest");
        if m.exists() && !DERIVETEST_DONE.swap(true, Ordering::Relaxed) {
            let _ = std::fs::remove_file(&m);
            unsafe { prove_derive() };
        }
    }
    // (loctest movido pra a thread do heartbeat — o cp77_tick/executor é intermitente por foco.)
    // `axl-copy-makeexist`: redirect de path inexistente (roda no MENU). Máquina de estados dirigida
    // pelo tick (captura o depot real → golden) — ver `prove_copy_tick`. Arma no 1º achado do marcador.
    if !COPYTEST_DONE.load(Ordering::Relaxed) {
        if let Ok(h) = std::env::var("HOME") {
            let m = std::path::Path::new(&h).join(".bwms-copytest");
            if m.exists() {
                if !COPYTEST_ARMED.swap(true, Ordering::Relaxed) {
                    let _ = std::fs::remove_file(&m); // consome 1x; a partir daí roda por fase
                }
            }
        }
        if COPYTEST_ARMED.load(Ordering::Relaxed) {
            unsafe { prove_copy_tick() };
            if COPY_PHASE.load(Ordering::Relaxed) >= 3 {
                COPYTEST_DONE.store(true, Ordering::Relaxed); // fase terminal (3=ok, 9=abortado)
            }
        }
    }
    // CPVR (dev): sinaliza gameplay pro cpvr.js gatear a captura VR (só em jogo, nunca menu/boot).
    #[cfg(feature = "cpvr")]
    cpvr_ingame_marker(!player.is_null() && !tx.is_null());
    // CallbackSystem: controller STATE-DRIVEN de sessão do player. Dispara na TRANSIÇÃO real de
    // presença (player+tx): "Player/Spawned" ao entrar no mundo, "Player/Despawned" ao sair
    // (menu/load/troca de save). ≠ Session/Ready (que latcha 1× por PROCESSO): este RE-DISPARA a
    // cada sessão nova (cobre reload de save) e Despawned é sinal NOVO (fim de sessão). Fica ANTES
    // do gate p/ enxergar o despawn (o gate retorna em player-null). Mecanismo = fire_event
    // (provado + GUARDADO: o dispatch valida rtti::sane+class_of → alvo liberado é PULADO, sem o
    // crash de [[cp77-crash-callbacksystem-devreds]]; sem listener = no-op). SEM offset novo.
    {
        static PLAYER_PRESENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        // `redDispatcher-crash-handle-ctor-delay` (2026-07-18, hipótese do coordenador, análise
        // estática dos 2 primeiros .ips + corroborada por leitura de código): no tick EXATO da
        // transição `present` (só acontece de verdade num save-load REAL — nunca exercitada antes
        // desta sessão), 2 `Handle_ctor` (pool alloc) disparavam de volta-a-volta (Session/Start +
        // Entity/Attach), bem no instante em que o motor inunda com realocação de array/world-data
        // do save real. 4/4 crashes desta sessão (`redDispatcher4/7/9`, mesma assinatura EXATA:
        // `cp77_tick→call_func→exec_replacement`, mesmos offsets nativos 35074284/35074676)
        // aconteceram perto dessa janela. FIX: atrasa só os 2 eventos com `make_handle` por N ticks
        // após a transição (mesmo padrão `m_stable>=40`/`TICKS>120` já usado no projeto pra "esperar
        // o mundo assentar" — ver BwmsTppPoller/tweakxl-updaterecord acima). `Player/Spawned` (sem
        // handle) continua disparando na hora — não usa `Handle_ctor`, não é suspeito.
        static PENDING_HANDLE_EVENTS_AT: AtomicU64 = AtomicU64::new(0); // tick-alvo; 0 = nada pendente
        const HANDLE_EVENTS_DELAY_TICKS: u64 = 180; // ~3-6s no ritmo já usado por TICKS neste arquivo
        let present = !player.is_null() && !tx.is_null();
        if present != PLAYER_PRESENT.swap(present, Ordering::Relaxed) {
            let ev = if present { "Player/Spawned" } else { "Player/Despawned" };
            let n = unsafe { register::fire_event(ev) };
            crate::log(&format!("[cbs] {ev} (transição de presença do player) → {n} callback(s)"));
            if present {
                // `1.4` (2026-07-19): sinal MODO-INDEPENDENTE de gameplay alcançada → arma a rede
                // anti-crash do redDispatcher também no modo 0 (onde o getter de skip — a única fonte
                // de PHASE_REACHED_5 — não instala). Latcha.
                selfboot::POST_SAVELOAD.store(true, Ordering::Relaxed);
                // NÃO dispara Session/Start/Entity/Attach aqui — agenda pra N ticks depois (bloco
                // fora do `if`, abaixo), fora da janela de flood do save-load real.
                let target = TICKS.load(Ordering::Relaxed) + HANDLE_EVENTS_DELAY_TICKS;
                PENDING_HANDLE_EVENTS_AT.store(target, Ordering::Relaxed);
                crate::log(&format!(
                    "[cbs] Session/Start+Entity/Attach AGENDADOS pra tick>={target} (delay anti-crash, atual={})",
                    TICKS.load(Ordering::Relaxed)
                ));
            } else {
                // Despawn/End: sem o padrão de flood conhecido (mundo saindo, não entrando) — mantém
                // imediato, e também cancela qualquer disparo pendente de uma sessão anterior.
                PENDING_HANDLE_EVENTS_AT.store(0, Ordering::Relaxed);
                if let Some(arg) = unsafe { register::make_gamesessionevent_arg(false, true) } {
                    let n2 = unsafe { register::fire_event_args("Session/End", &[arg]) };
                    crate::log(&format!("[cbs] Session/End (GameSessionEvent real) → {n2} callback(s)"));
                }
            }
        }
        // Dispara os 2 eventos com `make_handle` (Session/Start + Entity/Attach) quando o tick-alvo
        // agendado acima é atingido — fora do `if` de transição (roda em ticks POSTERIORES ao evento
        // que armou o alvo). Guarda `PLAYER_PRESENT` pra não disparar se o player já saiu de novo.
        let pending = PENDING_HANDLE_EVENTS_AT.load(Ordering::Relaxed);
        if pending != 0 && TICKS.load(Ordering::Relaxed) >= pending {
            PENDING_HANDLE_EVENTS_AT.store(0, Ordering::Relaxed);
            if PLAYER_PRESENT.load(Ordering::Relaxed) && present {
                // `cw-controller-session`: "Session/Start" (nome REAL do Codeware,
                // `CallbackSystem::SessionStartEventName`). `restored=true` (auto-continue SEMPRE
                // carrega save) / `pregame=false` (player só presente pós-char-creation).
                if let Some(arg) = unsafe { register::make_gamesessionevent_arg(false, true) } {
                    let n2 = unsafe { register::fire_event_args("Session/Start", &[arg]) };
                    crate::log(&format!("[cbs] Session/Start (GameSessionEvent real, atrasado {HANDLE_EVENTS_DELAY_TICKS} ticks) → {n2} callback(s)"));
                }
                // `cw-controller-entity`: "Entity/Attach" (nome REAL, `EntityAttachHook.hpp`) com
                // `EntityLifecycleEvent` real (`GetEntity()->ref<Entity>`, "raw"/sem dono).
                if let Some(arg) = unsafe { register::make_entitylifecycleevent_arg(player) } {
                    let n3 = unsafe { register::fire_event_args("Entity/Attach", &[arg]) };
                    crate::log(&format!("[cbs] Entity/Attach (EntityLifecycleEvent real, atrasado {HANDLE_EVENTS_DELAY_TICKS} ticks) → {n3} callback(s)"));
                }
            }
        }
    }
    // `cet-lifecycle-events`: onOverlayOpen/onOverlayClose — a borda é sinalizada na thread do render
    // (overlay.rs::render_imgui, via presentDrawable) e consumida AQUI (thread do jogo, cp77_tick) pra
    // disparar o fire_event com segurança (chamar a VM fora da thread do jogo é arriscado). Roda ANTES
    // do gate de player (o overlay abre/fecha tanto no menu quanto em gameplay).
    if overlay::OVERLAY_OPEN_EDGE.swap(false, Ordering::Relaxed) {
        let n = unsafe { register::fire_event("Overlay/Open") };
        crate::log(&format!("[cbs] Overlay/Open → {n} callback(s)"));
    }
    if overlay::OVERLAY_CLOSE_EDGE.swap(false, Ordering::Relaxed) {
        let n = unsafe { register::fire_event("Overlay/Close") };
        crate::log(&format!("[cbs] Overlay/Close → {n} callback(s)"));
    }
    // `cw-controller-misc`: "Resource/Load" — edge-triggered (MESMO padrão seguro de Player/
    // Spawned/Overlay acima, roda ANTES do gate de player pq streaming de recurso acontece desde
    // o boot). O hook `resource.link` (já instalado, zero-crash provado) marca `WATCH_RES_SEEN`
    // quando o path armado por `watchres <path>` é de fato construído pelo jogo.
    if selftest::WATCH_RES_SEEN.swap(false, Ordering::Relaxed) {
        let h = selftest::WATCH_RES_HASH.load(Ordering::Relaxed);
        if let Some(arg) = unsafe { register::make_resourceevent_arg(h) } {
            let n = unsafe { register::fire_event_args("Resource/Load", &[arg]) };
            crate::log(&format!("[cbs] Resource/Load (ResourceEvent real, path_hash={h:#018x}) → {n} callback(s)"));
        }
    }
    if player.is_null() || tx.is_null() {
        return;
    }
    // runtime vivo (player/tx ok): pulsa o heartbeat que o badge do overlay lê.
    TICKS.fetch_add(1, Ordering::Relaxed);
    // `cet-thirdparty-mod-api`: chama a native registrada por um PLUGIN de 3o ("BwmsPluginOnUpdate",
    // se existir) A CADA TICK — prova lifecycle onUpdate contínuo pós-boot via BwmsApi (não só a
    // chamada 1x de bwms_plugin_main). MESMA via de resolução/chamada do comando `callg`
    // (register::get_function + rtti::call_func) — nenhum offset/ABI novo.
    // GATE (2026-07-16, 3 achados AO VIVO nesta sessão, nessa ordem):
    // 1) sem gate nenhum: um `rtti::call_func` disparado bem na janela da transição de
    //    LoadModdedSave crasha (mesmo padrão já corrigido no full-body).
    // 2) gate `GAME_SESSION_DESC+0x84==5` ANTES do gate de player: lê um objeto que ainda não
    //    estabilizou (phase oscilando 3→0→...) — nunca vira true.
    // 3) o MESMO gate, movido pra DEPOIS do gate de player (mesma posição do full-body): AINDA
    //    não disparou em 2 boots com gameplay REAL confirmada (phasedbg mostrava phase=5 em
    //    OUTROS objetos, mas GAME_SESSION_DESC especificamente não). Conclusão: esse sinal
    //    específico não é confiável fora do caso estreito do full-body (que só o usa como
    //    reforço, gateado TAMBÉM por `TICKS>1800`). Substituído pelo gate coarse já usado (e
    //    provado reliable) pelos comandos de canal (`hasgod`/`callg`) neste mesmo ponto: apenas
    //    "player/tx confirmados não-nulos NESTE tick" (o próprio fato de termos chegado até aqui,
    //    passando o gate acima) + um piso pequeno de ticks (evita a primeira rajada bem na
    //    transição, sem exigir os milhares de ticks que a via antiga precisava).
    // `redDispatcher-crash-any-callfunc-near-phase5` (2026-07-18, v3 — v2 usava
    // `selfboot::GAME_SESSION_DESC` (a MESMA leitura do bloco full-body) e NUNCA armou num boot que
    // crashou de novo (6ª ocorrência, mesma assinatura EXATA): achado real por que — o getter que
    // captura esse ponteiro TRAVA (`ENGAGEMENT_SM_LOCKED`) no objeto TRANSIENTE da tela de
    // engagement e NUNCA re-captura o objeto REAL pós-save-load (confirmado: `[phasedbg] getter#9`
    // e `getter#10`, ambos phase=5, têm `this=` DIFERENTE — um novo objeto substitui o antigo, mas
    // `GAME_SESSION_DESC` fica preso no velho). Ou seja: o próprio sinal que o bloco full-body usa
    // pode estar batendo em phase5 do objeto ERRADO nalguns casos — não invalida o full-body (que
    // parece ter funcionado antes por outra via), mas invalida meu uso aqui. FIX v3: usar
    // `selfboot::PHASE_REACHED_5`/`PHASE5_AT`, setados DIRETO dentro do PRÓPRIO hook do getter
    // (o mesmo que produz os logs `[phasedbg] phase=5` confiáveis, `this` qualquer que seja) —
    // sinal por EVENTO, não por re-derivação de ponteiro cacheado. `PHASE5_AT` é um `Instant` real,
    // então o cooldown é por TEMPO DE PAREDE (imune a incerteza de taxa de tick).
    //
    // Contexto (hipótese ampla, substituindo a estreita do coordenador que a v1 já refutou):
    // `exec_replacement` (frame do meio em TODOS os 6 crashes) tem assinatura de DISPATCHER
    // GENÉRICO de chamada nativa (func/ctx/frame/aOut/a4), não um hook de UMA função — plausível
    // que QUALQUER `rtti::call_func` disparado bem no instante do save-load real completar colida
    // com o motor. Gate: suprime as invocações call_func PERIÓDICAS (este bloco + Session/Ready/
    // Update/Tick abaixo) por uma janela de TEMPO após a 1ª vez que phase5 é observado — sem
    // impedir o uso ANTES (menu/player-espúrio, seguro há centenas de chamadas hoje) nem MUITO
    // DEPOIS (só a janela de risco real).
    // v3 (12s): crashou (7ª ocorrência) ~15-18s pós-`[autocontinue] disparou`, bem quando o
    // cooldown ACABAVA de expirar — o contador do plugin ficou PARADO até ali (gate ENGATOU de
    // verdade, sobreviveu TODA a janela suprimida). v4 (45s): MESMO padrão — sobreviveu os 45s
    // inteiros suprimido (1ª vez que qualquer boot passa da janela clássica de crash), MAS crashou
    // (8ª ocorrência) poucos segundos depois de RETOMAR as chamadas periódicas. **Achado que muda o
    // modelo de novo:** não parece ser "tempo insuficiente desde phase5" — em AMBOS os testes o
    // crash bateu logo após RETOMAR, não importa se retomou aos 12s ou aos 45s. Ou seja, o perigo
    // pode estar ligado ao ATO de retomar/1ª chamada pós-gap, não a uma janela de tempo fixa. v5:
    // suprime PERMANENTEMENTE (nunca retoma nesta sessão) uma vez que phase5 é visto — testa se
    // ficar OFF pra sempre pós-gameplay-real evita o crash de vez (aceitando a perda de feature:
    // plugin onUpdate + Session/Ready/Update/Tick não rodam mais DEPOIS do save-load real, só antes).
    // `1.4`: OR com POST_SAVELOAD pra a supressão valer no modo 0 (getter de skip não instala lá →
    // PHASE_REACHED_5 nunca setava → rede anti-crash inerte no modo 0). POST_SAVELOAD é modo-independente.
    let in_crash_window = selfboot::PHASE_REACHED_5.load(Ordering::Relaxed)
        || selfboot::POST_SAVELOAD.load(Ordering::Relaxed);
    {
        static WAS_IN_WINDOW: AtomicBool = AtomicBool::new(false);
        if in_crash_window != WAS_IN_WINDOW.swap(in_crash_window, Ordering::Relaxed) {
            crate::log(&format!(
                "[cbs] crash-window (call_func periódico suprimido) -> {in_crash_window} (phase5_at.elapsed={:?})",
                selfboot::PHASE5_AT.get().map(|t| t.elapsed())
            ));
        }
    }
    {
        static PLUGIN_ONUPDATE_FN: AtomicU64 = AtomicU64::new(0);
        if PLUGIN_ONUPDATE_FN.load(Ordering::Relaxed) == 0 {
            let f = unsafe { register::get_function(reg, "BwmsPluginOnUpdate") };
            if rtti::sane(f) {
                PLUGIN_ONUPDATE_FN.store(f as u64, Ordering::Relaxed);
            }
        }
        let f = PLUGIN_ONUPDATE_FN.load(Ordering::Relaxed);
        let past_transition_window = TICKS.load(Ordering::Relaxed) > 60 && !in_crash_window;
        if f != 0 && past_transition_window {
            let rf = rtti::ResolvedFn { func: f as *mut c_void, ret_type: std::ptr::null_mut(), is_static: true };
            unsafe { rtti::call_func(&rf, std::ptr::null_mut(), &[]) };
        }
    }
    // `tweakxl-updaterecord`: roda em GAMEPLAY (após o gate de player) — os records instanciam em
    // recordsByID só no world-load (RecordExists=0 no menu, provado). Gated marcador ~/.bwms-updaterec.
    // Ver `tweakdb_rt::prove_updaterecord`. TICKS>120 = deixa a sessão estabilizar como o full-body.
    if !UPDATEREC_DONE.load(Ordering::Relaxed) && TICKS.load(Ordering::Relaxed) > 120 {
        if let Ok(h) = std::env::var("HOME") {
            let m = std::path::Path::new(&h).join(".bwms-updaterec");
            if m.exists() && !UPDATEREC_DONE.swap(true, Ordering::Relaxed) {
                let _ = std::fs::remove_file(&m);
                unsafe { crate::tweakdb_rt::prove_updaterecord() };
            }
        }
    }
    // breadth Reflection: probe de CProperty num objeto VIVO (gated ~/.bwms-reflection-test), 1x.
    unsafe { register::reflection_live_once(player) };
    // breadth RED4ext/CET: valida vtable_hook/unhook na vtable real do player (gated ~/.bwms-vtable-test), 1x.
    unsafe { gum::vtable_selftest_once(player) };
    // FULL-BODY auto-start (2026-07-15): chama o global redscript `BwmsBootFullbody` UMA vez, ~alguns
    // segundos após entrar em gameplay (t>120 ticks). Roda AQUI = game-thread (seguro): cria+agenda os
    // pollers do full-body SEM wrapar nenhuma classe de world-load (o @wrapMethod em PlayerPuppet/HUD
    // corrompia o SystemsUpdater no world-load — provado). Substitui o `callg BwmsBootFullbody` manual.
    //
    // ACHADO 2026-07-15 (mesmo dia, mais tarde): chamar com ctx=null e zero args fazia o
    // `GetGameInstance()` DENTRO do redscript devolver uma instância MORTA (GetDelaySystem/
    // GetPlayerSystem retornavam undefined mesmo em gameplay real confirmada por screenshot —
    // ver proofs/2026-07-15-callg-global-getgameinstance-dead-ACHADO.log). Fix: nunca deixar o
    // redscript conjurar a GameInstance sozinho a partir de uma chamada `call_func` crua — em vez
    // disso, o Rust obtém uma GameInstance de verdade AQUI (mesma receita AUTORITATIVA de
    // console.rs::auth_player: `PlayerPuppet.GetGame` chamado com ctx=player, o ponteiro que
    // JÁ temos vivo neste ponto) e passa por VALOR (Arg::Raw, 16B) pro redscript receber como
    // parâmetro — BwmsBootFullbody(game: GameInstance) em vez de BwmsBootFullbody() + GetGameInstance().
    {
        // TESTE 2026-07-15 (mesmo dia, ainda mais tarde): os pollers agendados logo em TICKS>120
        // (~poucos segundos pós-player-não-nulo, ainda DENTRO da janela do autocontinue/
        // LoadModdedSave) param de vez depois de ~36-37 execuções (~11s), mesmo depois do fix de
        // reagendar `this`. Hipótese: o GameInstance/DelaySystem capturado tão cedo fica órfão
        // quando o mundo termina de carregar de verdade e o motor troca pra uma sessão final.
        // Teste: disparar bem mais tarde (TICKS>1800, ~30-60s pós-player-não-nulo, bem depois da
        // janela onde o poller morria) pra ver se um poller iniciado DEPOIS sobrevive indefinidamente.
        // GATE DE SEGURANÇA (2026-07-15, isolando um crash `SystemsUpdater::Node::LinkJob_NoFence`
        // que reproduziu num boot 100% passivo, SEM tppcam/forcelook ativos — só o auto-trigger +
        // pollers rodando). Até isolar a causa raiz, o auto-start fica OPT-IN (marcador ausente =
        // pollers NUNCA são criados = comportamento idêntico ao usuário que não mexe no full-body).
        let home = std::env::var("HOME").unwrap_or_default();
        let fb_enabled = !home.is_empty() && std::path::Path::new(&format!("{home}/.bwms-fullbody-enable")).exists();
        // OVERRIDE de trigger imediato (2026-07-16): com o jogo OCIOSO (sem input humano p/ andar),
        // TICKS acumula devagar demais (~1 por 2048 chamadas do executor) e o gate TICKS>1800 nunca
        // é atingido em tempo hábil pra testar. Com `~/.bwms-fullbody-now`, dispara assim que
        // phase5 vira true (o phase5 já garante gameplay real = seguro; o TICKS>1800 era só p/ evitar
        // a janela de world-load, que o phase5 também cobre). Lido a cada tick (barato) só quando fb ligado.
        let fb_now = fb_enabled && !home.is_empty()
            && std::path::Path::new(&format!("{home}/.bwms-fullbody-now")).exists();
        // GATE DE GAMEPLAY REAL (2026-07-16): as funções full-body SÓ podem rodar em phase==5 (gameplay).
        // O menu/preview tem um "player-espúrio" — `player != null` NÃO basta (o preview satisfaz).
        // Rodar o full-body no preview deref um objeto STALE quando ele muda → crash na thread
        // redDispatcher (`cp77_tick → call_func → deref de ponteiro-lixo`, provado 2x nesta sessão,
        // inclusive um crash no MENU aos ~309s). phase (GAME_SESSION_DESC+0x84): 1/2/3=menu/boot, 5=gameplay.
        let phase5 = unsafe {
            let sm = selfboot::GAME_SESSION_DESC.load(Ordering::Relaxed);
            !sm.is_null()
                && gum::is_readable(sm as *const c_void, 0x85)
                && (sm.add(0x84) as *const i8).read() as i32 == 5
        };
        static FB_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if phase5 && fb_enabled && !FB_STARTED.load(Ordering::Relaxed) && (fb_now || TICKS.load(Ordering::Relaxed) > 1800) {
            let gi: Option<[u8; 16]> = unsafe {
                crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                    crate::rtti::call_func(&gg, player, &[]).map(|b| {
                        let mut o = [0u8; 16];
                        o.copy_from_slice(&b[..16]);
                        o
                    })
                })
            };
            if let Some(gi) = gi {
                let f = unsafe { register::get_function(reg, "BwmsBootFullbody") };
                if crate::rtti::sane(f) {
                    let rf = crate::rtti::ResolvedFn {
                        func: f,
                        ret_type: std::ptr::null_mut(),
                        is_static: true,
                    };
                    if unsafe { crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) }.is_some() {
                        FB_STARTED.store(true, Ordering::Relaxed);
                        crate::log("[fullbody] BwmsBootFullbody(game) chamado com GameInstance real (PlayerPuppet.GetGame, não GetGameInstance())");
                    }
                }
            } else {
                crate::log("[fullbody] PlayerPuppet.GetGame falhou — adiando auto-start");
            }
        }
        // RE-DISPARO do full-body dirigido pelo TICK LOOP DO RUST (2026-07-15): o ActivateTPPRepresentation
        // é transiente em free-roam (~25s). Em vez de um DelayCallback de redscript que se reagenda (o
        // objeto script cai de escopo no contexto callg e o motor crasha ~1m57s tocando a ref pendente —
        // provado: crasha até com o Call() nunca disparando), o Rust re-chama uma função redscript
        // ONE-SHOT (`BwmsTppRefire`) a cada ~600 ticks (~10s). Cada chamada resolve o player fresco e
        // enfileira 1 evento — zero objeto de vida-longa, zero DelayCallback. Gate próprio (~/.bwms-tpp-refire)
        // pra não interferir no teste limpo do one-shot; só roda depois do FB_STARTED.
        let refire_on = !home.is_empty() && std::path::Path::new(&format!("{home}/.bwms-tpp-refire")).exists();
        if phase5 && refire_on && FB_STARTED.load(Ordering::Relaxed) && TICKS.load(Ordering::Relaxed) % 600 == 0 {
            let gi: Option<[u8; 16]> = unsafe {
                crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                    crate::rtti::call_func(&gg, player, &[]).map(|b| {
                        let mut o = [0u8; 16];
                        o.copy_from_slice(&b[..16]);
                        o
                    })
                })
            };
            if let Some(gi) = gi {
                let f = unsafe { register::get_function(reg, "BwmsTppRefire") };
                if crate::rtti::sane(f) {
                    let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                    unsafe { crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) };
                }
            }
        }
        // SEGURAR AS PERNAS EM PÉ (2026-07-16): as vars `fullbody`/`isTPP` do anim-graph precisam ser
        // reaplicadas RÁPIDO (~1.5s) pra segurar a pose entre os resets do ActivateTPP (a cada ~10s).
        // Cadência ~90 ticks. Mesmo gate (refire marker + FB_STARTED). Ver BwmsLegsHold no reds.
        if phase5 && refire_on && FB_STARTED.load(Ordering::Relaxed) && TICKS.load(Ordering::Relaxed) % 60 == 0 {
            let gi: Option<[u8; 16]> = unsafe {
                crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                    crate::rtti::call_func(&gg, player, &[]).map(|b| {
                        let mut o = [0u8; 16];
                        o.copy_from_slice(&b[..16]);
                        o
                    })
                })
            };
            if let Some(gi) = gi {
                let f = unsafe { register::get_function(reg, "BwmsLegsHold") };
                if crate::rtti::sane(f) {
                    let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                    unsafe { crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) };
                }
            }
        }
    }
    // CallbackSystem (lite): emite "Session/Ready" a cada ~120 ticks até despachar (espera o
    // OnGameAttached registrar). Aqui o "controller" = o tick de gameplay pronto; outros eventos
    // (input/entity) = hooks de função de jogo chamando fire_event. Ver register::fire_event.
    if !in_crash_window {
        static CBS_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let t = TICKS.load(Ordering::Relaxed);
        if !CBS_DONE.load(Ordering::Relaxed) && t % 120 == 0 {
            let n = unsafe { register::fire_event("Session/Ready") };
            if n > 0 {
                CBS_DONE.store(true, Ordering::Relaxed);
            }
        }
        // "Session/Update" = evento PERIÓDICO (onUpdate, callback contínuo de mod). 2º tipo de evento.
        if t % 180 == 0 {
            unsafe { register::fire_event("Session/Update") };
        }
        // "Session/Tick" = evento periódico que PASSA DADO (o nº do tick) pro callback — prova que o
        // evento carrega args (= o que input/entity events precisam). callback = OnBwmsTick(n: Int32).
        if t % 240 == 0 {
            unsafe { register::fire_event_args("Session/Tick", &[rtti::Arg::I32(t as u32)]) };
        }
    }
    // ROTA A: registra os hooks Observe/Override pendentes (resolve a função na
    // thread do jogo + publica a lista de ptrs vigiados pra sonda).
    if hooks::has_pending() {
        unsafe { hooks::drain_pending(reg) };
    }
    // comando pendente (do overlay ou de fora, via /tmp)
    let cmd = std::fs::read_to_string("/tmp/cp77-cmd.txt").unwrap_or_default();
    let cmd = cmd.trim().to_string();
    if !cmd.is_empty() {
        let _ = std::fs::remove_file("/tmp/cp77-cmd.txt");
        run_cmd(reg, player, tx, &cmd);
    }
    // hotkeys pressionados (registerHotkey) → dispara callbacks na thread do jogo.
    for c in hotkey_drain() {
        unsafe { lua::fire_hotkey(c) };
    }
    // registerInput: eventos down/up → dispara callbacks com isDown na thread do jogo.
    for (c, down) in input_drain() {
        unsafe { lua::fire_input(c, down) };
    }
    // CallbackSystem RawInput controller: emite "Input/Key" com um `ref<KeyInputEvent>` REAL
    // pra CADA tecla capturada no sendEvent durante gameplay (2026-07-18, `cw-rawinput-realname`
    // — antes era o keycode cru como Int32; agora constrói+despacha o objeto real via
    // `Arg::Handle`, mesmo nome de evento REAL do Codeware, `Input/Key`).
    for (key, shift, control, alt) in drain_raw_keys() {
        if let Some(arg) = unsafe { register::make_keyinputevent_arg(key, shift, control, alt) } {
            unsafe { register::fire_event_args("Input/Key", &[arg]) };
        }
    }
    // lifecycle: onOverlayOpen/onOverlayClose quando o console abre/fecha (lua) + equivalente
    // não-lua via CallbackSystem ("Overlay/Open"/"Overlay/Close", mesmo padrão de "Input/Key").
    {
        use std::sync::atomic::AtomicBool;
        static LAST_SHOW: AtomicBool = AtomicBool::new(false);
        let now = overlay::is_shown();
        if now != LAST_SHOW.swap(now, Ordering::Relaxed) {
            unsafe { lua::run_event(if now { "onOverlayOpen" } else { "onOverlayClose" }) };
            unsafe { register::fire_event_args(if now { "Overlay/Open" } else { "Overlay/Close" }, &[]) };
        }
    }
    // lifecycle dos mods: onUpdate a cada tick (já throttled pela sonda).
    unsafe { lua::run_event("onUpdate") };
}

/// Chamado pela sonda DE DENTRO do hook do executor, SÓ para métodos vigiados
/// (Observe/Override), ANTES da original. `mcname` = CName do método (func+0x10,
/// lido pela sonda), `ctx` = o `this`. Retorna 1 se um Override pediu p/ SUPRIMIR a
/// original (futuro; hoje 0 = Observe puro).
#[no_mangle]
pub extern "C" fn cp77_obs_before(
    mcname: u64,
    func: *mut c_void,
    ctx: *mut c_void,
    frame: *mut c_void,
) -> u8 {
    // dispatch_before lê os args do frame e roda Observe/Override(wrapped); devolve suppress.
    // Export FFI legado (sonda frida, morta): sem aOut aqui → res=null. Com res null o
    // override-total de retorno POD não grava nada (write_pod_ret=false) → cai no suppress
    // só-void de antes (seguro). O caminho vivo é o `exec_replacement` (passa o aOut real).
    u8::from(unsafe { hooks::dispatch_before(mcname, func, ctx, frame, std::ptr::null_mut()) })
}

/// Idem, mas DEPOIS da original rodar (ObserveAfter) + Override (reescreve `res`).
#[no_mangle]
pub extern "C" fn cp77_obs_after(mcname: u64, ctx: *mut c_void, res: *mut c_void) {
    unsafe {
        hooks::dispatch_after(mcname, ctx);
        hooks::dispatch_override(mcname, ctx, res);
    }
}

/// Carrega os mods de `<dir>/<nome>/init.lua` (estado limpo + onInit de todos).
/// `prod_only`=true (auto-load do primeiro uso) PULA mods de teste/experimentais
/// (nome com "Test" ou começando com "CPVR") — eles continuam vivos via `loadmods`
/// manual, nada é removido. `prod_only`=false carrega tudo.
fn load_mods_dir(dir: &str, prod_only: bool) -> usize {
    unsafe { lua::reset() };
    let mut count = 0usize;
    // Leitura ROBUSTA do diretório: o `entries.flatten()` antigo ENGOLIA em silêncio uma
    // entrada com erro de leitura transiente (comum em volume externo) → o nativeSettings
    // sumia sem aviso e o cheats achava "NativeSettings nao encontrado", quebrando o botão
    // Mods. Agora: coleta TODAS as entradas, LOGA qualquer erro, e tenta de novo (até 3x) se
    // alguma entrada falhar — assim um hiccup transiente não derruba mais um mod inteiro.
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for attempt in 0..3 {
        paths.clear();
        let mut any_err = false;
        match std::fs::read_dir(dir) {
            Ok(rd) => {
                for e in rd {
                    match e {
                        Ok(de) => paths.push(de.path()),
                        Err(er) => {
                            any_err = true;
                            log(&format!("[mods] entrada ilegível (tentativa {attempt}): {er}"));
                        }
                    }
                }
            }
            Err(er) => {
                any_err = true;
                log(&format!("[mods] dir ilegível (tentativa {attempt}): {er}"));
            }
        }
        if !any_err && !paths.is_empty() {
            break; // leitura limpa
        }
    }
    paths.sort(); // ordem determinística (não depende da ordem do FS)
    for path in &paths {
        // nome do mod = nome da pasta (pro GetMod), como no CET.
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if prod_only && (name.contains("Test") || name.starts_with("CPVR")) {
            continue; // fora do default; carregável no `loadmods` manual
        }
        let init = path.join("init.lua");
        if init.exists() {
            match std::fs::read_to_string(&init) {
                Ok(src) => {
                    unsafe { lua::run_mod(&name, &src, path) };
                    count += 1;
                    log(&format!("[mods] carregado: {}", init.display()));
                }
                Err(er) => log(&format!("[mods] erro lendo {}: {er}", init.display())),
            }
        }
    }
    unsafe { lua::run_event("onInit") };
    MODS_LOADED.fetch_add(count, Ordering::Relaxed);
    log(&format!("[mods] {count} mod(s) carregado(s) de {dir} (prod_only={prod_only})"));
    count
}

/// Parseia um arg de console p/ Reflection-Call: `i:5` `f:1.5` `b:true` `n:Name` `s:txt` `e:3`.
/// Sem prefixo conhecido → tenta i32. Marshaling real fica no rtti::Arg/call_func (provado).
/// Int u32 flexível: aceita decimal com sinal (`-1`→0xFFFFFFFF), decimal sem sinal (`4000000000`)
/// e hexa (`0xFF`). O marshalling antigo só lia decimal com sinal → `i:0xFF` virava 0 mudo.
fn parse_u32_flex(v: &str) -> Option<u32> {
    let v = v.trim();
    if let Some(h) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).ok()
    } else {
        v.parse::<i32>().map(|x| x as u32).ok().or_else(|| v.parse::<u32>().ok())
    }
}

/// Int u64 flexível (pra `e:` enum): decimal ou hexa. Idem parse_u32_flex.
fn parse_u64_flex(v: &str) -> Option<u64> {
    let v = v.trim();
    if let Some(h) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else {
        v.parse::<u64>().ok().or_else(|| v.parse::<i64>().map(|x| x as u64).ok())
    }
}

/// Bool tolerante e case-insensitive: `b:TRUE`/`b:Yes`/`b:1` = true (antes só `true`/`1` exatos →
/// `b:TRUE` virava false silencioso numa chamada VIVA de método do jogo).
fn parse_bool_flex(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "t" | "y" | "on" => Some(true),
        "false" | "0" | "no" | "f" | "n" | "off" => Some(false),
        _ => None,
    }
}

/// Faz o marshalling de UM arg do canal (`i:`/`f:`/`b:`/`n:`/`e:`/`s:` ou i32 cru) → `rtti::Arg`,
/// retornando `Err` explícito em vez de `I32(0)` silencioso. Isto é o coração de TODO
/// `call/callf/callg/callon` + do plugin C-ABI: um prefixo com typo (`ii:5`) ou um valor inválido
/// antes contaminava uma chamada de método do jogo VIVA com um zero mudo. Puro/testável offline.
fn parse_cmd_arg_checked(s: &str) -> Result<rtti::Arg, String> {
    if let Some((ty, val)) = s.split_once(':') {
        return match ty {
            "i" => parse_u32_flex(val)
                .map(rtti::Arg::I32)
                .ok_or_else(|| format!("i:{val} não é int (decimal ou 0xHEX)")),
            "f" => val
                .trim()
                .parse::<f32>()
                .map(rtti::Arg::F32)
                .map_err(|_| format!("f:{val} não é float")),
            "b" => parse_bool_flex(val)
                .map(rtti::Arg::Bool)
                .ok_or_else(|| format!("b:{val} não é bool (true/false/1/0/yes/no/on/off)")),
            "n" => Ok(rtti::Arg::CName(crate::cname::cname(val))),
            "e" => parse_u64_flex(val)
                .map(rtti::Arg::Enum)
                .ok_or_else(|| format!("e:{val} não é enum int (decimal ou 0xHEX)")),
            "s" => Ok(rtti::Arg::Str(val.to_string())),
            other => Err(format!(
                "prefixo de arg desconhecido '{other}:' em '{s}' — use i:/f:/b:/n:/e:/s: (ou i32 cru)"
            )),
        };
    }
    // sem prefixo = i32 cru (decimal ou 0xHEX).
    parse_u32_flex(s)
        .map(rtti::Arg::I32)
        .ok_or_else(|| format!("arg '{s}' sem prefixo e não é int — use i:/f:/b:/n:/e:/s:"))
}

/// Wrapper que preserva a assinatura antiga (os 5+ callers de `call*` não mudam): em erro, LOGA o
/// motivo (não é mais silencioso) e cai pra `I32(0)`.
fn parse_cmd_arg(s: &str) -> rtti::Arg {
    match parse_cmd_arg_checked(s) {
        Ok(a) => a,
        Err(e) => {
            log(&format!("[arg] {e} -> usando I32(0)"));
            rtti::Arg::I32(0)
        }
    }
}

/// `red4ext-reloc-universal` (in-game) — prova o relocador arm64 num prólogo REAL do jogo com o
/// caso DIFÍCIL (BL, PC-relativo). Alvo: `game::MuppetStateMachines<MuppetLogicStateMachineState>
/// ::~dtor` @vm 0x104164c9c — prólogo [stp x29,x30,[sp,#-0x10]! | mov x29,sp | BL <helper> | ldp
/// x29,x30]. NO MENU esse dtor de IA de NPC está dormente (mundo não carregado) → hookar+dumpar+
/// reverter é colisão ~zero; por isso o gatilho por marcador roda ANTES do gate de player (menu).
/// Guard nos 16 bytes exatos do prólogo: se o build divergir, aborta sem tocar em nada.
/// PROVA INDEPENDENTE: (A) decodifica o alvo absoluto do BL pelo offset relativo do próprio insn;
/// (B) decodifica o movz/movk x17 que o trampolim materializou; A==B ⇒ o relocador traduziu
/// PC-relativo→absoluto correto. Depois reverte e confere os 16 bytes originais byte-exato.
///
/// # Safety
/// Só toca o alvo se os 16 bytes do prólogo baterem com a assinatura; hook+dump+revert é síncrono
/// (sem yield) e o revert é imediato — nenhuma thread chega a chamar a função hookada.
pub(crate) unsafe fn prove_relocreal() {
    // dummy nunca é chamado (revertemos antes de qualquer thread poder invocar o alvo).
    unsafe extern "C" fn reloc_dummy() {}
    const VM: u64 = 0x1_0416_4c9c;
    const SIG: [u32; 4] = [0xa9bf_7bfd, 0x9100_03fd, 0x9423_6161, 0xa8c1_7bfd];
    let target = crate::rebase(VM);
    if !crate::gum::is_readable(target as *const c_void, 16) {
        log("[relocreal] alvo ilegível (slide/patch?) — abortado sem tocar");
        return;
    }
    let mut orig = [0u8; 16];
    core::ptr::copy_nonoverlapping(target as *const u8, orig.as_mut_ptr(), 16);
    let got: [u32; 4] = core::array::from_fn(|i| {
        u32::from_le_bytes([orig[i * 4], orig[i * 4 + 1], orig[i * 4 + 2], orig[i * 4 + 3]])
    });
    if got != SIG {
        log(&format!(
            "[relocreal] prólogo divergiu do esperado (build diferente?) got={got:08x?} esperado={SIG:08x?} — abortado"
        ));
        return;
    }
    // (A) alvo absoluto do BL (insn[2]), calculado do endereço RUNTIME do próprio BL.
    let t = target as u64;
    let bl = got[2];
    let mut off = (bl & 0x03FF_FFFF) as i64;
    if off & (1 << 25) != 0 {
        off |= !0x03FF_FFFFi64;
    }
    let bl_target_a = ((t + 8) as i64).wrapping_add(off << 2) as u64;

    // hook: relocate_prologue roda no prólogo REAL; devolve o ponteiro do trampolim.
    let it = crate::gum::Interceptor::obtain();
    let tramp = match it.replace(target, reloc_dummy as *mut c_void) {
        Some(tr) => tr as *const u8,
        None => {
            log("[relocreal] replace() RECUSOU — relocador não tratou o prólogo (falha real)");
            return;
        }
    };
    // o site agora começa com um abs-jump (não é mais o prólogo original)?
    let mut site4 = [0u8; 4];
    core::ptr::copy_nonoverlapping(target as *const u8, site4.as_mut_ptr(), 4);
    let site_changed = site4 != [orig[0], orig[1], orig[2], orig[3]];

    // (B) movz/movk x17 no trampolim: offset 8 = após stp,mov copiados verbatim (4B cada).
    let mut mm = [0u8; 20]; // 16B (movz/movk) + 4B (blr x17)
    core::ptr::copy_nonoverlapping(tramp.add(8), mm.as_mut_ptr(), 20);
    let materialized_b = {
        let mut v = 0u64;
        for i in 0..4 {
            let insn = u32::from_le_bytes([mm[i * 4], mm[i * 4 + 1], mm[i * 4 + 2], mm[i * 4 + 3]]);
            let hw = ((insn >> 21) & 0x3) as u64;
            let imm16 = ((insn >> 5) & 0xFFFF) as u64;
            v |= imm16 << (16 * hw);
        }
        v
    };
    let blr = u32::from_le_bytes([mm[16], mm[17], mm[18], mm[19]]);
    let blr_ok = blr == (0xD63F_0000u32 | (17 << 5)); // blr x17
    // o BL original NÃO deve aparecer verbatim no trampolim (foi expandido, não copiado).
    let mut tramp32 = [0u8; 32];
    core::ptr::copy_nonoverlapping(tramp, tramp32.as_mut_ptr(), 32);
    let bl_absent = !tramp32
        .chunks_exact(4)
        .any(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) == bl);

    // revert + confirma byte-exato.
    it.revert(target);
    let mut after = [0u8; 16];
    core::ptr::copy_nonoverlapping(target as *const u8, after.as_mut_ptr(), 16);
    let byte_exact = after == orig;

    let reloc_ok = bl_target_a == materialized_b;
    let verdict = if reloc_ok && byte_exact && site_changed && blr_ok && bl_absent {
        ">>> RELOC-UNIVERSAL OK: BL real relocado (A==B) + blr x17 + BL não-verbatim + site patchado + revert byte-exato <<<"
    } else {
        "FALHA/verificar: alguma condição não bateu"
    };
    log(&format!(
        "[relocreal] alvo@{t:#x} | BL->A={bl_target_a:#x} tramp movz/movk->B={materialized_b:#x} A==B:{reloc_ok} | blr_x17:{blr_ok}({blr:#010x}) BL_expandido:{bl_absent} site_patchado:{site_changed} | revert_byte_exato:{byte_exact} | {verdict}"
    ));
}

/// `tweakxl-registername` — prova a fn nativa PURA de derive de TweakDBID (`CreateTweakDBID` core,
/// achada por RE 2026-07-16 @vmaddr 0x1034535c0): `u64 derive(const char* data, u32 len, u64 base)`
/// = CRC32-IEEE seeded com length telescópico. base=0 → `tweak_db_id(name)` (nome→id, o forward que
/// o gap pede: "resolver 'Items.X' → id no jogo vivo"). base=parent_id → `tweak_db_id_derive`.
/// A fn é PURA (0 escrita de memória, 0 lock) → chamável direto sem risco. Prova = o resultado
/// nativo BATE com `bwms_hashes` (nossa reimplementação, já provada offline vs 3.5M pares) —
/// confirma o endereço sob ASLR E que o hash do jogo == o nosso. Roda no MENU (não precisa player).
///
/// # Safety
/// Só chama a fn pura no endereço rebaseado, com args C-ABI corretos (ptr/len/base); sem efeito colateral.
pub(crate) unsafe fn prove_derive() {
    type DeriveFn = unsafe extern "C" fn(*const u8, u32, u64) -> u64;
    let addr = crate::rebase(0x1_0345_35c0);
    if !crate::gum::is_readable(addr as *const c_void, 4) {
        log("[derivetest] endereço da derive ilegível (slide/versão?) — abortado");
        return;
    }
    let derive: DeriveFn = core::mem::transmute(addr);
    // teste 1 — base=0 (== tweak_db_id): nome cheio → id.
    let n1 = "Items.Preset_Lexington";
    let got1 = derive(n1.as_ptr(), n1.len() as u32, 0);
    let exp1 = bwms_hashes::tweak_db_id(n1);
    let ok1 = got1 == exp1;
    // teste 2 — base=parent (== tweak_db_id_derive): telescoping com seed != 0.
    let suf = ".Cool";
    let got2 = derive(suf.as_ptr(), suf.len() as u32, got1);
    let exp2 = bwms_hashes::tweak_db_id_derive(got1, suf);
    let exp2_full = bwms_hashes::tweak_db_id("Items.Preset_Lexington.Cool");
    let ok2 = got2 == exp2 && got2 == exp2_full;
    let verdict = if ok1 && ok2 {
        ">>> TWEAKXL-REGISTERNAME OK: derive nativo do jogo == bwms_hashes (nome→id forward provado no jogo vivo) <<<"
    } else {
        "FALHA/verificar: derive nativo divergiu do bwms_hashes"
    };
    log(&format!(
        "[derivetest] nativo@{addr:p} | derive('{n1}',22,0)={got1:#x} vs bwms_hashes={exp1:#x} ok={ok1} | derive('{suf}',5,base)={got2:#x} vs derive={exp2:#x}/full={exp2_full:#x} ok={ok2} | {verdict}"
    ));
}

// ===== `axl-copy-makeexist`: fazer um path INEXISTENTE "existir" via redirect (resource.copy) =====
// Achado por RE 2026-07-16 (workflow #2): ResourceDepot::CheckResource/ResourceExists @0x103ed9e9c
// `bool(depot* x0, ResourcePath x1 /*u64 hash*/)`. O ArchiveXL copy usa HookBefore nessa fn: se o
// path pedido é o "copy" (inexistente), reescreve pro path do asset real → o path passa a "existir".
// Depot singleton = [rebase(0x109003000)+0x1f8] (já usado no projeto). PROVA log-only, menu-safe.
type CheckResFn = unsafe extern "C" fn(*mut c_void, u64) -> u8;
static COPY_ORIG: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static COPY_DEPOT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut()); // `this` REAL capturado do jogo
static COPY_X: AtomicU64 = AtomicU64::new(0); // path inexistente (o "copy")
static COPY_REAL_Y: AtomicU64 = AtomicU64::new(0); // path real capturado (orig devolveu true)
static COPY_ARMED: AtomicBool = AtomicBool::new(false); // redirect X→Y ativo?
static COPY_PHASE: AtomicU32 = AtomicU32::new(0);

/// Replacement do CheckResource: (1) CAPTURA o `this` real do depot (x0) e o 1º path real (orig=true)
/// das chamadas naturais do jogo — corrige o crash anterior (o singleton [0x109003000+0x1f8] era o
/// subobjeto ERRADO); (2) quando ARMADO, redireciona o path X (nosso copy inexistente) pro Y real
/// (estilo resource.copy do ArchiveXL) → CheckResource passa a devolver true pra X.
unsafe extern "C" fn check_res_replacement(depot: *mut c_void, path: u64) -> u8 {
    let orig = COPY_ORIG.load(Ordering::Relaxed);
    let f: CheckResFn = core::mem::transmute(orig);
    let x = COPY_X.load(Ordering::Relaxed);
    if COPY_ARMED.load(Ordering::Relaxed) && path == x {
        let y = COPY_REAL_Y.load(Ordering::Relaxed);
        if y != 0 {
            return f(depot, y); // "faz X existir" servindo Y
        }
    }
    let r = f(depot, path);
    if COPY_DEPOT.load(Ordering::Relaxed).is_null() && !depot.is_null() {
        COPY_DEPOT.store(depot, Ordering::Relaxed); // `this` autêntico do jogo
    }
    if r != 0 && COPY_REAL_Y.load(Ordering::Relaxed) == 0 && path != x && path != 0 {
        COPY_REAL_Y.store(path, Ordering::Relaxed); // path real (existe) capturado do jogo
    }
    r
}

/// `axl-copy-makeexist` (menu, log-only) — máquina de estados DIRIGIDA PELO TICK (não one-shot):
/// fase 0 instala o hook (captura o `this` real do depot + um Y real das chamadas do jogo), fase 1
/// espera a captura, fase 2 roda o golden (CheckResource(X) false→true via redirect) e reverte.
/// Corrige o crash da 1ª tentativa (usava o depot-singleton, subobjeto errado → SIGSEGV no field-deref).
///
/// # Safety
/// Só instala hook próprio no CheckResource + chama a fn com o `this` AUTÊNTICO capturado do jogo
/// (mesmo ponteiro que o jogo passa, garantidamente válido); redirect só pro nosso X único.
pub(crate) unsafe fn prove_copy_tick() {
    let check_addr = crate::rebase(0x1_03ed_9e9c);
    match COPY_PHASE.load(Ordering::Relaxed) {
        0 => {
            if !crate::gum::is_readable(check_addr as *const c_void, 4) {
                log("[copytest] CheckResource ilegível — abortado");
                COPY_PHASE.store(9, Ordering::Relaxed);
                return;
            }
            COPY_X.store(
                bwms_hashes::resource_path_hash("base\\bwms\\nonexistent_copy_probe_zzz.mesh"),
                Ordering::Relaxed,
            );
            let it = crate::gum::Interceptor::obtain();
            match it.replace(check_addr, check_res_replacement as *mut c_void) {
                Some(tramp) => {
                    COPY_ORIG.store(tramp, Ordering::Relaxed);
                    std::mem::forget(it);
                    COPY_PHASE.store(1, Ordering::Relaxed);
                    log("[copytest] hook em CheckResource instalado — capturando o depot+Y reais das chamadas do jogo...");
                }
                None => {
                    log("[copytest] replace() de CheckResource RECUSOU — abortado");
                    COPY_PHASE.store(9, Ordering::Relaxed);
                }
            }
        }
        1 => {
            // espera capturar o `this` real + um path real (o jogo chama CheckResource constantemente).
            if !COPY_DEPOT.load(Ordering::Relaxed).is_null() && COPY_REAL_Y.load(Ordering::Relaxed) != 0 {
                COPY_PHASE.store(2, Ordering::Relaxed);
            }
        }
        2 => {
            let depot = COPY_DEPOT.load(Ordering::Relaxed);
            let x = COPY_X.load(Ordering::Relaxed);
            let y = COPY_REAL_Y.load(Ordering::Relaxed);
            let check: CheckResFn = core::mem::transmute(check_addr); // hookado → passa pelo replacement
            COPY_ARMED.store(false, Ordering::Relaxed);
            let x_before = check(depot, x); // esperado 0 (não existe)
            let y_exists = check(depot, y); // esperado != 0 (sanidade do depot+Y)
            COPY_ARMED.store(true, Ordering::Relaxed);
            let x_after = check(depot, x); // redirect X→Y → esperado != 0
            COPY_ARMED.store(false, Ordering::Relaxed);
            crate::gum::Interceptor::obtain().revert(check_addr);
            COPY_PHASE.store(3, Ordering::Relaxed);
            let ok = x_before == 0 && y_exists != 0 && x_after != 0;
            let verdict = if ok {
                ">>> COPY-MAKEEXIST OK: path inexistente passou a EXISTIR (CheckResource false→true) via redirect (resource.copy) <<<"
            } else {
                "FALHA/verificar: esperado X_antes=0, Y_existe!=0, X_depois!=0"
            };
            log(&format!(
                "[copytest] depot@{depot:p}(capturado do jogo) | Y#{y:#018x} existe={y_exists} | X#{x:#018x} existe_ANTES={x_before} existe_DEPOIS_do_redirect={x_after} | {verdict}"
            ));
        }
        _ => {}
    }
}

/// `axl-localization-apply` (menu, log-only) — INJETA um par (primaryKey → CString) no repositório de
/// onscreens da localização (o que o ArchiveXL faz pra adicionar nomes/legendas de item) e LÊ de volta
/// via a GetText nativa, provando que a string custom aparece. Modelo achado por RE (workflow 2026-07-17,
/// HIGH): LocMgr singleton = *(0x108feedb0); repo = singleton+0x28; mapa female = repo+0x38.
/// FindOrInsert nativo 0x102f6d614(map, &key, &CString, &iter); GetText 0x102f6e76c(repo, key, variant,
/// &flag, &CString). CString ctor 0x10091dafc(out@x8, data@x0, len@x1); CString c_str = 0x1000293f8.
///
/// # Safety
/// Todas as fns são nativas do LocalizationManager vivo (carregado no boot); usa os sret (x8) via asm.
/// O grow/rehash/CString são cuidados pelas próprias nativas. Roda 1x no menu.
pub(crate) unsafe fn prove_loc() -> bool {
    use core::arch::asm;
    let sing_pp = crate::rebase(0x1_08fe_edb0) as *const *mut u8;
    if !crate::gum::is_readable(sing_pp as *const c_void, 8) {
        return false; // ainda não pronto — o driver re-tenta
    }
    let sing = *sing_pp;
    if sing.is_null() || !crate::gum::is_readable(sing as *const c_void, 0xa0) {
        // LocMgr ainda não inicializou — re-tenta no próximo tick. Loga 1x + a cada ~600 ticks
        // (visibilidade sem flood) pra distinguir "retry (null)" de "não rodou (tick parado)".
        static NULL_HITS: AtomicU64 = AtomicU64::new(0);
        let n = NULL_HITS.fetch_add(1, Ordering::Relaxed);
        if n == 0 || n % 600 == 0 {
            log(&format!("[loctest] LocMgr singleton ainda null (retry #{n}) — esperando a localização carregar"));
        }
        return false;
    }
    log("[loctest] LocMgr pronto — rodando prova de localização...");
    let repo = sing.add(0x28);
    let female_map = repo.add(0x38);
    let key: u64 = 0xF00D_BEEF_1337_0001;
    let text = b"BWMS_LOC_PROOF_777\0";
    let text_len: u32 = 18; // sem o \0

    // 1) CString do valor: void CString(out@x8, data@x0, len@x1). O node value é 32B (stride 0x30,
    // value@+0x10 → 0x20=32B); red::CString cabe nesse buffer (SSO inline ~30 chars p/ 18-char str).
    let mut val = [0u8; 32];
    asm!("blr {f}", f = in(reg) crate::rebase(0x1_0091_dafc),
        in("x0") text.as_ptr(), in("x1") text_len as u64, in("x8") val.as_mut_ptr(),
        clobber_abi("C"));
    log("[loctest] CString do valor construída");

    // DIAG: os campos do female_map (struct HashMap) devem parecer reais (buckets/nodes = ponteiros
    // de heap; size/cap plausíveis). Se garbage, o offset repo+0x38 está errado. Valida antes de
    // chamar a nativa (evita o crash de deref-lixo que aconteceu).
    let rd_u64 = |p: *const u8| (p as *const u64).read_unaligned();
    let rd_u32 = |p: *const u8| (p as *const u32).read_unaligned();
    let buckets = rd_u64(female_map) as *mut u8;
    let size = rd_u32(female_map.add(0x08));
    let cap = rd_u32(female_map.add(0x0c));
    let nodes = rd_u64(female_map.add(0x10)) as *mut u8;
    let stride = rd_u32(female_map.add(0x1c));
    // O LocMgr existe ANTES da localização carregar os textos — o female_map fica VAZIO (size/ptr 0)
    // por um tempo. NÃO é erro: retorna false pra o heartbeat re-tentar até LoadTexts popular o mapa.
    if buckets.is_null() || nodes.is_null() || size == 0 || cap == 0 {
        static NOTLOADED: AtomicU64 = AtomicU64::new(0);
        let n = NOTLOADED.fetch_add(1, Ordering::Relaxed);
        if n == 0 || n % 10 == 0 {
            log(&format!("[loctest] female_map ainda vazio (loc não populada, retry #{n}) — size={size}"));
        }
        return false; // heartbeat re-tenta em 2s
    }
    log(&format!(
        "[loctest] female_map@{female_map:p} | buckets={buckets:p} size={size} cap={cap} nodes={nodes:p} stride={stride:#x}"
    ));
    let map_sane = crate::gum::is_readable(buckets as *const c_void, 4)
        && crate::gum::is_readable(nodes as *const c_void, 8)
        && size <= cap
        && cap < 10_000_000
        && (0x10..=0x40).contains(&stride);
    if !map_sane {
        log("[loctest] female_map preenchido mas layout estranho (offset repo+0x38 errado?) — abortado");
        return true;
    }

    // APLICO num node EXISTENTE (sem grow, sem FindOrInsert). Pego o 1º node, sobrescrevo a CString
    // dele (node+0x10) pela minha, e verifico LENDO essa CString DIRETO — que é EXATAMENTE o que a
    // GetText nativa resolveria pra essa key (GetText = achar o node pela key → devolver node+0x10).
    // NÃO chamo GetText da thread do heartbeat: ela pega o LOCK do mapa de loc que o game thread
    // segura → DEADLOCK (travou a thread na v2, 0 [hb]). `c_str` é accessor puro (sem lock), safe.
    // Node: {next i32@+0, hash32 u32@+4, key u64@+8, value(CString)@+0x10}.
    let cstr_data: unsafe extern "C" fn(*const u8) -> *const i8 =
        core::mem::transmute(crate::rebase(0x1_0002_93f8));
    let read_cstr = |cs: *const u8| -> String {
        let p = cstr_data(cs);
        if !p.is_null() && crate::gum::is_readable(p as *const c_void, 1) {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        } else {
            "<null>".to_string()
        }
    };

    let node0 = nodes; // 1º node
    if !crate::gum::is_readable(node0 as *const c_void, 0x30) {
        log("[loctest] node0 ilegível — abortado");
        return true;
    }
    let key0 = (node0.add(0x08) as *const u64).read_unaligned();
    let val_slot = node0.add(0x10); // CString do node = o texto que GetText devolve pra key0.
    // O value slot tem 32B (stride do node 0x30 − offset 0x10); red::CString é SSO de ~20 chars
    // inline + length/allocator, tudo dentro desses 32B. Copiar só 16 truncava a string custom
    // (18 chars) em 16 — TEM que copiar os 32B inteiros do value slot.
    let mut saved = [0u8; 32];
    core::ptr::copy_nonoverlapping(val_slot, saved.as_mut_ptr(), 32); // guarda p/ restaurar

    let s_before = read_cstr(val_slot);
    // sobrescreve a CString do node pela minha (val, 32B, construída no passo 1) — só memória.
    core::ptr::copy_nonoverlapping(val.as_ptr(), val_slot, 32);
    let s_after = read_cstr(val_slot);
    // restaura a CString original (não deixa a localização suja pro resto da sessão).
    core::ptr::copy_nonoverlapping(saved.as_ptr(), val_slot, 32);

    let ok = s_after == "BWMS_LOC_PROOF_777" && s_before != s_after;
    let verdict = if ok {
        ">>> LOCALIZATION-APPLY OK: sobrescrevi o texto de uma key de localização existente e a CString que o GetText resolve pra ela virou a string custom (antes != depois) <<<"
    } else {
        "FALHA/verificar: a CString do node não virou a string custom após o overwrite"
    };
    log(&format!(
        "[loctest] key0={key0:#x} | texto ANTES='{s_before}' | DEPOIS do overwrite='{s_after}' (esperado 'BWMS_LOC_PROOF_777') | {verdict}"
    ));
    true
}

/// Replacement do InitializeArchives (0x103ed96b0): `u64(depot@x0)`. Chama a original (constrói TODOS
/// os archives do boot no depot REAL) e DEPOIS injeta o nosso .archive fora do glob — tudo na THREAD DO
/// JOGO, com o depot totalmente construído, segurando o lock, ANTES do streaming do mundo. Uma vez.
unsafe extern "C" fn init_archives_replacement(depot: *mut c_void) -> u64 {
    let orig = INIT_ARCH_ORIG.load(Ordering::Relaxed);
    let f: unsafe extern "C" fn(*mut c_void) -> u64 = core::mem::transmute(orig);
    let ret = f(depot); // constrói os ~N archives do boot no depot
    if !PATHB_INJECTED.swap(true, Ordering::Relaxed)
        && !depot.is_null()
        && crate::gum::is_readable(depot as *const c_void, 0x80)
    {
        pathb_inject(depot as *mut u8);
    }
    ret
}

/// Instala o `replace` no InitializeArchives (uma vez, cedo — do on_load/thread do jogo). Gated pelo
/// chamador (marker). Ver `init_archives_replacement`.
unsafe fn install_pathb_capture() {
    if PATHB_HOOK_ON.swap(true, Ordering::Relaxed) {
        return;
    }
    let addr = crate::rebase(0x1_03ed_96b0);
    if !crate::gum::is_readable(addr as *const c_void, 4) {
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(addr, init_archives_replacement as *mut c_void) {
        Some(tramp) => {
            INIT_ARCH_ORIG.store(tramp, Ordering::Relaxed);
            std::mem::forget(it);
            log("[pathb] replace no InitializeArchives (0x103ed96b0) instalado — injeta pós-build, thread do jogo...");
        }
        None => log("[pathb] replace() do InitializeArchives RECUSOU — pathb não pode injetar"),
    }
}

/// `axl-pathb-injection-arbitrary`: injeta um .archive REAL cujo nome NÃO casa o glob nativo
/// (basegame_*/audio_*/lang_*) no content-group do ResourceDepot via `LoadArchives` @0x103eda488
/// (abre+parseia+APENDA, BYPASSA o filtro do glob). Chamada de dentro de `init_archives_replacement`
/// (thread do jogo, depot REAL x0 já construído, pré-streaming). `depot` é o `this` autêntico — os
/// offsets da RE são corretos aqui (grupos@depot+0x10 inline {ptr@+0x10,cap@+0x18,count@+0x1c,stride
/// 0x38}; key@g+0x30==3; count@g+0xc; lock@depot+0x78). Segura o lock exclusivo durante a injeção.
unsafe fn pathb_inject(depot: *mut u8) {
    use core::arch::asm;
    // grupos: DynArray inline @depot+0x10
    let groups = (depot.add(0x10) as *const *mut u8).read();
    let gcount = (depot.add(0x1c) as *const u32).read();
    if groups.is_null() || gcount == 0 || gcount > 64 {
        log(&format!("[pathb] grupos inválidos (ptr={groups:p} n={gcount}) — abortado"));
        return;
    }
    // Achar o grupo REAL de content: o de MAIOR count. DUMP 2026-07-17 (v5): no Mac o grupo key==3
    // fica VAZIO (count=0, arr=sentinela ESTÁTICO na faixa de imagem 0x109... → LoadArchives nele
    // CRASHA); os archives do boot vão pra key=1 (33) e key=2 (26), arr HEAP, já carregados (síncronos
    // no retorno do InitArchives). Injetar no de maior count = grupo válido/cheio.
    let mut g: *mut u8 = core::ptr::null_mut();
    let mut best = 0u32;
    for i in 0..gcount as usize {
        let gi = groups.add(i * 0x38);
        if !crate::gum::is_readable(gi as *const c_void, 0x38) {
            continue;
        }
        let cnt = (gi.add(0x0c) as *const u32).read();
        let arr = (gi as *const *const u8).read();
        // exige arr em HEAP (não a faixa de imagem 0x1_0000_0000..0x1_1000_0000 = sentinela estático).
        let arr_heap = (arr as usize) > 0x1_1000_0000;
        if cnt > best && cnt < 100_000 && arr_heap {
            best = cnt;
            g = gi;
        }
    }
    // DUMP read-only de TODOS os grupos (key@+0x30, count@+0xc) — safe, sem crash. Revela o layout
    // real do depot pra fechar a injeção numa sessão futura. Achado 2026-07-17 (v5): no retorno do
    // InitializeArchives o content group está VAZIO (count=0) → os archives carregam ASSÍNCRONOS DEPOIS
    // (o append-hook do v4 viu o count crescer até 33 mais tarde). Logo o ponto de injeção certo NÃO é
    // aqui — é após a conclusão do load async (achar o callback), ou de dentro do append-hook (game
    // thread) MAS com o depot capturado à parte pro lock.
    {
        let mut s = format!("[pathb] DUMP {gcount} grupos (depot real={depot:p}):");
        for i in 0..gcount as usize {
            let gi = groups.add(i * 0x38);
            if !crate::gum::is_readable(gi as *const c_void, 0x38) {
                s.push_str(&format!(" [{i}]=ilegível"));
                continue;
            }
            let key = (gi.add(0x30) as *const u32).read();
            let cnt = (gi.add(0x0c) as *const u32).read();
            let arrp = (gi as *const *const u8).read();
            s.push_str(&format!(" [{i}]{{key={key} count={cnt} arr={arrp:p}}}"));
        }
        log(&s);
    }
    if g.is_null() || best == 0 {
        log(&format!("[pathb] nenhum grupo heap com archives entre {gcount} — abortado"));
        return;
    }
    let count0 = (g.add(0x0c) as *const u32).read();

    // INJEÇÃO REAL gated pelo 2º marcador (~/.bwms-pathb-inject). Agora mira o grupo de MAIOR count
    // (heap, cheio) — o crash do v5 era injetar no key==3 vazio (arr sentinela estático). Ver cont.81.
    let do_inject = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-pathb-inject").exists())
        .unwrap_or(false);
    if !do_inject {
        log(&format!(
            "[pathb] grupo de maior count g={g:p} count={count0} — injeção pulada (gated ~/.bwms-pathb-inject)"
        ));
        return;
    }

    // caminho absoluto do nosso archive de teste. A dylib mora em <jogo>/red4ext/libcp77_console.dylib.
    let mut game_dir = String::new();
    let n = _dyld_image_count();
    for i in 0..n {
        let nm = _dyld_get_image_name(i);
        if nm.is_null() {
            continue;
        }
        if let Ok(s) = std::ffi::CStr::from_ptr(nm).to_str() {
            if let Some(p) = s.strip_suffix("/red4ext/libcp77_console.dylib") {
                game_dir = p.to_string();
                break;
            }
        }
    }
    if game_dir.is_empty() {
        log("[pathb] não achei o dir do jogo (via dylib image) — abortado");
        return;
    }
    let apath = format!("{game_dir}/archive/Mac/content/zz_bwms_pathb.archive");
    if !std::path::Path::new(&apath).is_file() {
        log(&format!("[pathb] archive de teste ausente em {apath} — abortado"));
        return;
    }
    let cpath = match std::ffi::CString::new(apath.clone()) {
        Ok(c) => c,
        Err(_) => return,
    };
    let cscope = std::ffi::CString::new("content").unwrap();

    // ctor de CString inline (0x20B): void(out@x0, cstr@x1). O item da fileList É uma CString (stride 0x20).
    let cstr_ctor = crate::rebase(0x1_0002_cdb8);
    let mut s_path = [0u8; 0x20];
    let mut s_scope = [0u8; 0x20];
    asm!("blr {f}", f = in(reg) cstr_ctor,
        in("x0") s_path.as_mut_ptr(), in("x1") cpath.as_ptr(), clobber_abi("C"));
    asm!("blr {f}", f = in(reg) cstr_ctor,
        in("x0") s_scope.as_mut_ptr(), in("x1") cscope.as_ptr(), clobber_abi("C"));

    // fileList = DynArray<CString>{ ptr=&s_path, cap=1, count=1 } (16B). LoadArchives itera stride 0x20.
    #[repr(C)]
    struct FileList {
        ptr: *const u8,
        cap: u32,
        count: u32,
    }
    let flist = FileList {
        ptr: s_path.as_ptr(),
        cap: 1,
        count: 1,
    };

    // SCOPE: o crash report do boot #9 provou que o RDAR-open (0x103e2ebd4) deref o scope como PONTEIRO
    // e a minha CString hand-built de "content" (SSO inline) fazia ele deref os BYTES "content" como
    // endereço (fault @0x...746e65746e6f7b = "content"). A RE oferecia "pass the group's own name": o
    // ArchiveSet tem a RedString de nome/scope PRÓPRIA (válida, construída pelo jogo) em g+0x10. Uso ela.
    let scope_ptr = g.add(0x10) as *const u8; // RedString do próprio grupo (game-built, layout certo)
    let _ = &s_scope; // (mantido só p/ não quebrar o build; scope agora vem do grupo)

    log(&format!("[pathb] injetando no content group g={g:p} (archives ANTES={count0}) scope=g+0x10 segurando o lock..."));
    // lock EXCLUSIVO do depot REAL (SharedSpinLock @depot+0x78 — confirmado no disasm de
    // InitializeArchives: `add x19,x0,#0x78; bl 0x1000020c0`). acquire nativo; release = store-release 0.
    let lock = depot.add(0x78);
    let acquire = crate::rebase(0x1_0000_20c0);
    asm!("blr {f}", f = in(reg) acquire, in("x0") lock, clobber_abi("C"));

    // LoadArchives(x0=0 ignorado, x1=g, x2=&fileList, x3=&scope=g+0x10, w4=0 depsFlag, w5=2 tag)
    let load = crate::rebase(0x1_03ed_a488);
    asm!("blr {f}", f = in(reg) load,
        in("x0") 0usize, in("x1") g, in("x2") &flist as *const FileList,
        in("x3") scope_ptr, in("x4") 0usize, in("x5") 2usize,
        clobber_abi("C"));

    let count1 = (g.add(0x0c) as *const u32).read();
    // release exclusivo: store-release 0 no byte do lock.
    asm!("stlrb wzr, [{p}]", p = in(reg) lock, options(nostack));

    let ok = count1 >= count0 + 1;
    let verdict = if ok {
        ">>> PATH-B OK: .archive fora do glob (zz_bwms_pathb) INJETADO no content-group (depot REAL, thread do jogo, com lock) via LoadArchives — count subiu +1 <<<"
    } else if count1 == count0 {
        "FALHA: count inalterado — o open() rejeitou o archive (magic/versão) ou o append não ocorreu"
    } else {
        "ATENÇÃO: count mudou de forma inesperada (concorrência?)"
    };
    log(&format!(
        "[pathb] g={g:p} | archives ANTES={count0} DEPOIS={count1} | path={apath} | {verdict}"
    ));
}

// ===== `red4ext-attach-detach-contract`: 2 hooks EMPILHADOS no MESMO alvo (LIFO) + Detach único =====
// Alvo: um stub JIT NOSSO de 1 instrução (`ret`), NÃO uma fn Rust compilada no nosso próprio
// dylib — ver `gum::alloc_ret_stub` (achado 2026-07-16: hookar uma fn compilada aqui mesmo
// arriscou auto-modificação da página que o PRÓPRIO `prove_attach_detach` estava executando,
// crash silencioso confirmado ao vivo). Isola o que está sendo provado (o CONTRATO de
// empilhamento attach/detach) do relocador de opcode (já provado em separado, gap
// `red4ext-reloc-universal`) E de qualquer risco de self-modificação.
static AD_ORIG_HITS: AtomicU32 = AtomicU32::new(0);
static AD_H1_HITS: AtomicU32 = AtomicU32::new(0);
static AD_H2_HITS: AtomicU32 = AtomicU32::new(0);
static AD_TRAMP1: AtomicU64 = AtomicU64::new(0);
static AD_TRAMP2: AtomicU64 = AtomicU64::new(0);

type AdFn = unsafe extern "C" fn();

/// Substituto do hook 1 (o mais ANTIGO): chama através de `AD_TRAMP1` (a "orig" que o hook1
/// capturou ao instalar = o stub `ret` verdadeiro, já que foi o 1º) e só incrementa
/// `AD_ORIG_HITS` DEPOIS do call+ret voltar limpo — prova que a chamada através do trampolim
/// realmente alcançou o fim da cadeia e devolveu o controle certinho.
#[inline(never)]
unsafe extern "C" fn ad_repl1() {
    log("[attachdetach] repl1 entrou");
    AD_H1_HITS.fetch_add(1, Ordering::Relaxed);
    let t = AD_TRAMP1.load(Ordering::Relaxed);
    if t != 0 {
        log("[attachdetach] repl1 antes do through-call (tramp1)");
        let f: AdFn = core::mem::transmute(t as *const ());
        f();
        log("[attachdetach] repl1 DEPOIS do through-call (voltou de tramp1)");
        AD_ORIG_HITS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Substituto do hook 2 (o mais NOVO): chama através de `AD_TRAMP2` (a "orig" que o hook2
/// capturou ao instalar = o site JÁ patchado pelo hook1 → encadeia pra `ad_repl1`).
#[inline(never)]
unsafe extern "C" fn ad_repl2() {
    log("[attachdetach] repl2 entrou");
    AD_H2_HITS.fetch_add(1, Ordering::Relaxed);
    let t = AD_TRAMP2.load(Ordering::Relaxed);
    if t != 0 {
        log("[attachdetach] repl2 antes do through-call (tramp2)");
        let f: AdFn = core::mem::transmute(t as *const ());
        f();
        log("[attachdetach] repl2 DEPOIS do through-call (voltou de tramp2)");
    }
}

/// `red4ext-attach-detach-contract` (in-process, menu-oneshot): prova (1) 2 hooks empilhados no
/// MESMO alvo simultaneamente, (2) a cadeia LIFO (repl2 chama repl1 chama o stub — cada um
/// EXATAMENTE 1x), (3) `revert_all` (Detach) remove OS DOIS num call só e restaura os 16 bytes
/// ORIGINAIS byte-exato, (4) depois do Detach só o stub roda (os replacements não disparam
/// mais). `br` (usado no trampolim) não mexe em LR — por isso a cadeia de chamadas aninhadas
/// (`blr tramp2` → `br` pro repl1 → `blr tramp1` → `br` pro stub → `ret` → `ret` → `ret`)
/// devolve o controle certinho em cada nível, sem precisar de contabilidade extra.
///
/// # Safety
/// Só toca no stub JIT alocado aqui (não mexe em nada do motor do jogo, nem em código nosso já
/// compilado); hook+chamada+revert é síncrono, sem outra thread envolvida.
pub(crate) unsafe fn prove_attach_detach() {
    log("[attachdetach] início — alocando stub JIT");
    let target = match crate::gum::alloc_ret_stub() {
        Some(t) => t,
        None => {
            log("[attachdetach] FALHA ao alocar o stub JIT — abortado");
            return;
        }
    };
    log(&format!("[attachdetach] stub alocado @{target:p}"));
    AD_ORIG_HITS.store(0, Ordering::Relaxed);
    AD_H1_HITS.store(0, Ordering::Relaxed);
    AD_H2_HITS.store(0, Ordering::Relaxed);
    AD_TRAMP1.store(0, Ordering::Relaxed);
    AD_TRAMP2.store(0, Ordering::Relaxed);

    let mut orig16 = [0u8; 16];
    core::ptr::copy_nonoverlapping(target as *const u8, orig16.as_mut_ptr(), 16);

    let it = crate::gum::Interceptor::obtain();

    log("[attachdetach] instalando hook1...");
    // attach hook1 (captura o stub `ret` verdadeiro).
    let tramp1 = match it.replace(target, ad_repl1 as *mut c_void) {
        Some(t) => t,
        None => {
            log("[attachdetach] FALHA ao instalar hook1 — abortado (alvo recusado pelo relocador?)");
            return;
        }
    };
    AD_TRAMP1.store(tramp1 as u64, Ordering::Relaxed);
    let hooks_after_1 = it.hooks_on(target);
    log(&format!("[attachdetach] hook1 instalado, tramp1={tramp1:p} hooks_on={hooks_after_1}"));

    log("[attachdetach] instalando hook2 (empilhado)...");
    // attach hook2 EMPILHADO no MESMO alvo (captura o site já patchado pelo hook1).
    let tramp2 = match it.replace(target, ad_repl2 as *mut c_void) {
        Some(t) => t,
        None => {
            log("[attachdetach] FALHA ao instalar hook2 empilhado — desfazendo hook1 e abortando");
            it.revert(target);
            return;
        }
    };
    AD_TRAMP2.store(tramp2 as u64, Ordering::Relaxed);
    let hooks_after_2 = it.hooks_on(target);
    log(&format!("[attachdetach] hook2 instalado, tramp2={tramp2:p} hooks_on={hooks_after_2}"));

    log("[attachdetach] chamando o alvo (deve disparar repl2->repl1->stub)...");
    // chama o alvo: deve disparar repl2 -> repl1 -> original, cada um exatamente 1x (LIFO).
    let f: AdFn = core::mem::transmute(target as *const ());
    f();
    let (h2, h1, o1) = (
        AD_H2_HITS.load(Ordering::Relaxed),
        AD_H1_HITS.load(Ordering::Relaxed),
        AD_ORIG_HITS.load(Ordering::Relaxed),
    );
    let chain_ok = h2 == 1 && h1 == 1 && o1 == 1;
    log(&format!("[attachdetach] chamada voltou: h2={h2} h1={h1} orig={o1} chain_ok={chain_ok}"));

    log("[attachdetach] revert_all (Detach)...");
    // Detach: revert_all remove OS DOIS num call só.
    it.revert_all(target);
    let hooks_after_detach = it.hooks_on(target);
    let mut after16 = [0u8; 16];
    core::ptr::copy_nonoverlapping(target as *const u8, after16.as_mut_ptr(), 16);
    let byte_exact = after16 == orig16;
    log(&format!(
        "[attachdetach] revert_all voltou: hooks_on={hooks_after_detach} byte_exato={byte_exact}"
    ));

    log("[attachdetach] chamando o alvo de novo (pos-detach)...");
    // chama de novo: o site agora é o stub `ret` puro (sem side-effect observável) — a única
    // coisa que dá pra confirmar é que os REPLACEMENTS não disparam mais (contadores parados) E
    // que a chamada não crashou (chegamos até aqui pra checar).
    f();
    log("[attachdetach] 2a chamada voltou (nao crashou)");
    let (h2b, h1b) = (AD_H2_HITS.load(Ordering::Relaxed), AD_H1_HITS.load(Ordering::Relaxed));
    let only_orig_after_detach = h2b == h2 && h1b == h1;

    let ok = hooks_after_1 == 1
        && hooks_after_2 == 2
        && chain_ok
        && hooks_after_detach == 0
        && byte_exact
        && only_orig_after_detach;
    let verdict = if ok {
        ">>> ATTACH-DETACH-CONTRACT OK: 2 hooks empilhados no MESMO alvo (LIFO, cada um chamou o próximo 1x) + Detach removeu OS DOIS num call só + prólogo byte-exato + só a original roda depois <<<"
    } else {
        "FALHA/verificar: alguma condição não bateu"
    };
    log(&format!(
        "[attachdetach] hooks_apos_1={hooks_after_1} hooks_apos_2={hooks_after_2} | cadeia(h2={h2},h1={h1},orig={o1}):{chain_ok} | hooks_apos_detach={hooks_after_detach} byte_exato={byte_exact} | replacements_pararam_depois(h2={h2b},h1={h1b}):{only_orig_after_detach} | {verdict}"
    ));
}

fn run_cmd(reg: &rtti::Registry, player: *mut c_void, tx: *mut c_void, cmd: &str) {
    // ROTA A: `lua <código>` roda Lua no runtime persistente (Game.* bindado).
    if let Some(code) = cmd.strip_prefix("lua ") {
        unsafe { lua::run_code(code) };
        return;
    }
    // `unloadmods` (ou `reset`): descarrega TODOS os mods (limpa o estado Lua).
    if cmd == "unloadmods" || cmd == "reset" {
        unsafe { lua::reset() };
        log("[mods] todos os mods descarregados");
        return;
    }
    // `loadmods <dir>` = mod-manager: estado limpo + varre <dir>/<nome>/init.lua e
    // carrega TODOS (coexistindo num estado só), depois dispara onInit de todos.
    // Estrutura de pasta de mod do CET = mods/<NomeDoMod>/init.lua.
    if let Some(dir) = cmd.strip_prefix("loadmods ") {
        // Manual = carrega TODOS (inclui CPVR/testes).
        load_mods_dir(dir.trim(), false);
        return;
    }
    // `loadmod <arquivo.lua>` carrega um mod (registerForEvent) + dispara onInit.
    if let Some(path) = cmd.strip_prefix("loadmod ") {
        match std::fs::read_to_string(path.trim()) {
            Ok(src) => {
                // nome do mod = stem do arquivo (ex: "meumod.lua" -> "meumod") pro GetMod
                let name = std::path::Path::new(path.trim())
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "mod".into());
                unsafe {
                    lua::reset(); // recarrega LIMPO (sem duplicar onDraw/onUpdate)
                    let d = std::path::Path::new(path.trim())
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."));
                    lua::run_mod(&name, &src, d);
                    lua::run_event("onInit");
                }
                MODS_LOADED.fetch_add(1, Ordering::Relaxed);
                log(&format!("[console] mod carregado: {}", path.trim()));
            }
            Err(e) => log(&format!("[console] loadmod erro: {e}")),
        }
        return;
    }
    // `clear` (ou `cls`): limpa o scrollback do console (trunca o log) — estilo terminal.
    if cmd == "clear" || cmd == "cls" {
        overlay::clear_console_view(); // view-only: limpa a tela, mantém o arquivo p/ debug
        return;
    }
    // `ovcall` — validação do Override-suppress: chama BwmsProbe a partir do RUST (aqui,
    // run_cmd, SEM segurar o lock do Lua) → o override registrado via Lua consegue rodar
    // seu callback (o teste que chamava de dentro de um callback Lua/Cron falhava por
    // re-entrância: o lock do Lua estava seguro → `call_hook_override` dava try_lock Err →
    // override pulado). Na vida real o JOGO (nativo/redscript) chama o método, igual a isto.
    if cmd == "ovcall" {
        unsafe {
            let rp = rtti::resolve_func(reg, "PlayerPuppet", "BwmsProbe");
            let rc = rtti::resolve_func(reg, "PlayerPuppet", "BwmsProbeCalls");
            match (rp, rc) {
                (Some(rp), Some(rc)) => {
                    let i32_of = |o: Option<[u8; 0x20]>| {
                        o.map(|r| i32::from_le_bytes([r[0], r[1], r[2], r[3]])).unwrap_or(-1)
                    };
                    let before = i32_of(rtti::call_func(&rc, player, &[]));
                    // arma o override RUST-nativo de BwmsProbe→42 (sem lua) só p/ esta chamada
                    selfboot::RUST_OV_CNAME
                        .store(crate::cname::cname("BwmsProbe"), Ordering::Relaxed);
                    selfboot::RUST_OV_VAL.store(42, Ordering::Relaxed);
                    let ret = i32_of(rtti::call_func(&rp, player, &[])); // override RUST dispara AQUI
                    selfboot::RUST_OV_CNAME.store(0, Ordering::Relaxed); // desarma
                    let after = i32_of(rtti::call_func(&rc, player, &[]));
                    let verdict = if ret == 42 && after == before {
                        ">>> OVERRIDE-SUPPRESS OK (retorno reescrito + original suprimida) <<<"
                    } else if ret == 42 {
                        "retorno 42 mas a original rodou (rewrite, sem suppress)"
                    } else if after > before {
                        "original rodou (override nao pegou — CName/registro)"
                    } else {
                        "indefinido"
                    };
                    log(&format!(
                        "[ovcall] retorno={ret} (esperado 42) | contador {before}->{after} | {verdict}"
                    ));
                }
                _ => log("[ovcall] BwmsProbe/BwmsProbeCalls nao resolveram (sonda compilada?)"),
            }
        }
        return;
    }
    // `cet-override-suppress-proof` (2026-07-13) — MESMO mecanismo do `ovcall`, mas num método
    // REAL do motor (`gameGodModeSystem::HasGodMode`, read-only, sem efeito colateral) em vez do
    // `BwmsProbe` sintético — fecha o gap "não só ovcall sintético". Arma override->true, chama
    // `HasGodMode` (deve devolver true MESMO com o estado real sendo false), desarma, chama de
    // novo (deve voltar a devolver o estado REAL) — prova rewrite+suppress num alvo genuíno.
    if cmd == "ovcallreal" {
        unsafe {
            match console::hasgod(reg, player) {
                Some(real_before) => {
                    selfboot::RUST_OV_CNAME.store(crate::cname::cname("HasGodMode"), Ordering::Relaxed);
                    selfboot::RUST_OV_VAL.store(1, Ordering::Relaxed); // força Bool=true
                    let overridden = console::hasgod(reg, player);
                    selfboot::RUST_OV_CNAME.store(0, Ordering::Relaxed); // desarma
                    let real_after = console::hasgod(reg, player);
                    let verdict = if overridden == Some(true) && real_after == Some(real_before) {
                        ">>> OVERRIDE-SUPPRESS OK em método REAL (HasGodMode reescrito + estado real preservado) <<<"
                    } else {
                        "verificar: override não pegou ou estado real divergiu"
                    };
                    log(&format!(
                        "[ovcallreal] estado real ANTES={real_before} | overridden={overridden:?} (esperado Some(true)) | estado real DEPOIS={real_after:?} | {verdict}"
                    ));
                }
                None => log("[ovcallreal] hasgod() falhou antes de testar (player/sys indisponível)"),
            }
        }
        return;
    }
    // `cet-hooks-shippable` — os 3 modos (Observe/Override/Suppress) num método REAL do jogo
    // (`HasGodMode`, read-only), caminho NÃO-Lua. Override+Suppress já provados via `ovcallreal`
    // (2026-07-13); aqui fecha a perna OBSERVE que faltava (`RUST_OBS_CNAME`, selfboot.rs): o
    // callback dispara (log + contador) e a original AINDA roda (sem suprimir).
    // `axl-localization-apply` via CANAL (funciona em gameplay onde o executor dispara). Chama
    // prove_loc direto — bypassa a confusão do marcador one-shot no menu ocioso.
    if cmd == "loctest" {
        unsafe { prove_loc() };
        return;
    }
    // `redscript-thirdparty-proof` via CANAL (roda dentro do cp77_tick = thread do jogo, seguro pra
    // chamar a VM/redscript — ao contrário de uma thread nossa separada). Resolve GameInstance via
    // PlayerPuppet.GetGame(ctx=player) — a receita comprovada do BwmsBootFullbody — e chama
    // BwmsTestThirdPartyCheat3p(game) do mod de 3o. Evita o GetGameInstance() global (instância morta
    // quando chamado fora do contexto normal — achado 2026-07-15, cont.57).
    // Full-body: dispara BwmsBootFullbody(game) via CANAL (mesmo padrão do 3ptest) — bypassa o gate
    // automático do cp77_tick (TICKS/fb_now) que não disparou num boot com foco/timing atípico.
    // `redscript-cheat-effects-proof`: chama BwmsCheatEffectsTest(game) — exercita os cheats via a
    // MESMA instância/método (SettingsSelectorControllerBool.BWMSRun) que o clique real aciona.
    if cmd == "cheatfxtest" {
        unsafe {
            if player.is_null() {
                log("[cheatfx] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsCheatEffectsTest");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]).is_some() {
                            log("[cheatfx] BwmsCheatEffectsTest(game) chamado via canal");
                        } else {
                            log("[cheatfx] call_func não completou");
                        }
                    } else {
                        log("[cheatfx] BwmsCheatEffectsTest não resolveu (mod de teste compilado?)");
                    }
                }
                None => log("[cheatfx] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // Diagnóstico do force-look: chama BwmsCamTiltOnce(game) — isola se a escrita da câmera aplica de
    // vez ou é revertida no frame seguinte pela câmera nativa.
    // Torso + tilt NUMA CHAMADA SÓ (menos round-trips de canal = menos superfície pra crash).
    // Teste DEFINITIVO do problema real (pernas dobradas vs em pé) — torso+pernas+câmera numa chamada.
    // `cet-lifecycle-events`: força o toggle do overlay (sem HID) — o cp77_tick do próximo tick
    // dispara onOverlayOpen/Close pela borda real (mesmo caminho que o backtick real usa).
    if cmd == "overlaytoggle" {
        let now = !overlay::is_shown();
        overlay::set_shown(now);
        log(&format!("[overlaytoggle] SHOW agora={now}"));
        return;
    }
    // `cw-player-scheduling-vehicle`: chama a extensão REAL do Codeware `DelaySystem.DelayEvent`.
    if cmd == "delaytest" {
        unsafe {
            if player.is_null() {
                log("[cw-delaytest] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsDelaySystemTest");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
                        log("[cw-delaytest] BwmsDelaySystemTest(game) chamado via canal");
                    } else {
                        log("[cw-delaytest] BwmsDelaySystemTest não resolveu");
                    }
                }
                None => log("[cw-delaytest] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "overlaylctest" {
        unsafe {
            if player.is_null() {
                log("[overlay-lifecycle] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsOverlayLifecycleTest");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
                        log("[overlay-lifecycle] BwmsOverlayLifecycleTest(game) chamado via canal");
                    } else {
                        log("[overlay-lifecycle] BwmsOverlayLifecycleTest não resolveu");
                    }
                }
                None => log("[overlay-lifecycle] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "legstilt" {
        unsafe {
            if player.is_null() {
                log("[legstilt] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsLegsTiltTest");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
                        log("[legstilt] BwmsLegsTiltTest(game) chamado via canal");
                    } else {
                        log("[legstilt] BwmsLegsTiltTest não resolveu");
                    }
                }
                None => log("[legstilt] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "scenetier" {
        // `redDispatcher-crash-any-callfunc-near-phase5` (2026-07-18, achado do coordenador): o
        // screenshot do `legstilt` anterior pousou numa cena/animação roteirizada (V sentado,
        // jornal), não free-roam — confound real. `BwmsGetSceneTier(game)` (bwms-tppcam.reds) lê
        // `PlayerPuppet.GetSceneTier` (gamePSMHighLevel): 0=Default/free-roam real, 1-5=cena,
        // 6=nadando. Checar isto == 0 ANTES de disparar `legstilt` de novo.
        unsafe {
            if player.is_null() {
                log("[scenetier] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsGetSceneTier");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if let Some(ret) = crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            let tier = i32::from_le_bytes([ret[0], ret[1], ret[2], ret[3]]);
                            log(&format!("[scenetier] BwmsGetSceneTier(game) -> tier={tier} (0=free-roam real, 1-5=cena, 6=nadando)"));
                        } else {
                            log("[scenetier] call_func não retornou");
                        }
                    } else {
                        log("[scenetier] BwmsGetSceneTier não resolveu");
                    }
                }
                None => log("[scenetier] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "fbtilt" {
        unsafe {
            if player.is_null() {
                log("[fbtilt] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsFullbodyTiltTest");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
                        log("[fbtilt] BwmsFullbodyTiltTest(game) chamado via canal");
                    } else {
                        log("[fbtilt] BwmsFullbodyTiltTest não resolveu");
                    }
                }
                None => log("[fbtilt] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "camtilt" {
        unsafe {
            if player.is_null() {
                log("[camtilt] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsCamTiltOnce");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
                        log("[camtilt] BwmsCamTiltOnce(game) chamado via canal");
                    } else {
                        log("[camtilt] BwmsCamTiltOnce não resolveu");
                    }
                }
                None => log("[camtilt] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "fbtest" {
        unsafe {
            if player.is_null() {
                log("[fbtest] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsBootFullbody");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]).is_some() {
                            log("[fbtest] BwmsBootFullbody(game) chamado via canal com GameInstance real");
                        } else {
                            log("[fbtest] call_func não completou");
                        }
                    } else {
                        log("[fbtest] BwmsBootFullbody não resolveu");
                    }
                }
                None => log("[fbtest] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "onshutdowntest" {
        unsafe {
            if player.is_null() {
                log("[onshutdown-test] player null — sem save carregado?");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsOnShutdownTest");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]).is_some() {
                            log("[onshutdown-test] BwmsOnShutdownTest(game) chamado — registrado em Session/End");
                        } else {
                            log("[onshutdown-test] call_func de BwmsOnShutdownTest não completou");
                        }
                    } else {
                        log("[onshutdown-test] BwmsOnShutdownTest não resolveu (mod compilado?)");
                    }
                }
                None => log("[onshutdown-test] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "3ptest" {
        unsafe {
            if player.is_null() {
                log("[bwms-3p-test] player null — sem save carregado?");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsTestThirdPartyCheat3p");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]).is_some() {
                            log("[bwms-3p-test] BwmsTestThirdPartyCheat3p(game) chamado com GameInstance real");
                        } else {
                            log("[bwms-3p-test] call_func de BwmsTestThirdPartyCheat3p não completou");
                        }
                    } else {
                        log("[bwms-3p-test] BwmsTestThirdPartyCheat3p não resolveu (mod externo compilado?)");
                    }
                }
                None => log("[bwms-3p-test] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // `axl-factories-apply`: dump read-only do FactoryIndex (aIndex), pra achar o offset REAL do
    // registry pós-async. Ver `selftest::dump_factory_index`.
    if cmd == "factdump" {
        unsafe { crate::selftest::dump_factory_index() };
        return;
    }
    if cmd == "cethooksproof" {
        unsafe {
            match console::hasgod(reg, player) {
                Some(real_before) => {
                    let hits_before = selfboot::RUST_OBS_HITS.load(Ordering::Relaxed);
                    // (a) OBSERVE: arma, chama 1x, desarma — original deve rodar (retorno == estado real).
                    selfboot::RUST_OBS_CNAME.store(crate::cname::cname("HasGodMode"), Ordering::Relaxed);
                    let observed = console::hasgod(reg, player);
                    selfboot::RUST_OBS_CNAME.store(0, Ordering::Relaxed);
                    let hits_after = selfboot::RUST_OBS_HITS.load(Ordering::Relaxed);
                    let observe_ok = hits_after > hits_before && observed == Some(real_before);

                    // (b) OVERRIDE: arma pro valor OPOSTO do estado real (prova válida seja
                    // qual for o baseline do ambiente — forçar pro MESMO valor que já é real não
                    // provaria rewrite nenhum) — chama 1x, retorno deve vir o oposto forçado.
                    let ov_target = !real_before;
                    selfboot::RUST_OV_CNAME.store(crate::cname::cname("HasGodMode"), Ordering::Relaxed);
                    selfboot::RUST_OV_VAL.store(ov_target as i64, Ordering::Relaxed);
                    let overridden = console::hasgod(reg, player);

                    // (c) SUPPRESS: desarma, chama de novo — volta ao estado real (nada ficou mutado).
                    selfboot::RUST_OV_CNAME.store(0, Ordering::Relaxed);
                    let real_after = console::hasgod(reg, player);

                    let override_ok = overridden == Some(ov_target);
                    let suppress_ok = real_after == Some(real_before);
                    let verdict = if observe_ok && override_ok && suppress_ok {
                        ">>> CET-HOOKS-SHIPPABLE OK: Observe (cb disparou + original rodou) + Override (retorno reescrito) + Suppress (estado real preservado), tudo não-Lua em método REAL <<<"
                    } else {
                        "FALHA/verificar: alguma perna não bateu"
                    };
                    log(&format!(
                        "[cethooksproof] real_antes={real_before} | observe: hits {hits_before}->{hits_after} retorno={observed:?} ok={observe_ok} | override(alvo={ov_target}): retorno={overridden:?} ok={override_ok} | suppress: real_depois={real_after:?} ok={suppress_ok} | {verdict}"
                    ));
                }
                None => log("[cethooksproof] hasgod() falhou antes de testar (player/sys indisponível)"),
            }
        }
        return;
    }
    // `red4ext-reloc-universal` (in-game) — ver `prove_relocreal()`. Roda tanto pelo canal (gameplay)
    // quanto por marcador `~/.bwms-relocreal` NO MENU (onde o dtor-alvo de IA está dormente → seguro).
    if cmd == "relocreal" {
        unsafe { prove_relocreal() };
        return;
    }
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    // Pontos de desenvolvimento (PlayerDevelopmentData) — não retornam buffer.
    let pts = |member: &str, n: &str| {
        let q = n.parse().unwrap_or(1);
        let ok = unsafe { console::add_points(reg, player, q, member) };
        log(&format!("[console] '{cmd}' -> {}", if ok { "enviado" } else { "FALHOU" }));
    };
    let act = |label: &str, ok: bool| {
        log(&format!("[console] '{cmd}' -> {}", if ok { label } else { "FALHOU" }));
    };
    match parts.as_slice() {
        ["zzztest123"] => { log("ZZZTEST123-HIT"); return; }
        ["attrs", n] => return pts("Attribute", n),
        ["perks", n] => return pts("Primary", n),
        ["relic", n] => return pts("Espionage", n),
        ["level", n] => {
            return act("enviado", unsafe { console::level(reg, player, n.parse().unwrap_or(1)) })
        }
        ["godmode"] | ["god"] => return act("ON", unsafe { console::godmode(reg, player, true) }),
        ["godmode", "off"] | ["god", "off"] => {
            return act("OFF", unsafe { console::godmode(reg, player, false) })
        }
        // READ-ONLY: `redscript-cheat-effects-proof` — checa HasGodMode sem mutar (ver console::hasgod).
        ["hasgod"] => return log(&format!("[hasgod] {:?}", unsafe { console::hasgod(reg, player) })),
        ["heal"] => return act("curado", unsafe { console::heal(reg, player) }),
        ["summon"] | ["car"] => return act("enviado", unsafe { console::summon(reg, player) }),
        // Codeware/registro nativo (rodar no jogo p/ destravar a fundação):
        // cwprobe = despeja o layout de uma função nativa real → acha o offset do
        // handler; cwreg = smoke-test (registra BlackwallPing global); cwfacade =
        // registra Codeware.Version/Require (precisa do .reds do Codeware).
        ["cwprobe"] => return log(&unsafe { register::probe(reg) }),
        ["cwreg"] => return log(&unsafe { register::register_smoke(reg) }),
        ["cwfacade"] => return log(&unsafe { register::register_codeware_facade(reg) }),
        // RegisterType STEP-1: forja+registra uma CClass mínima (register-sem-instanciar). Confirmar
        // com `rttidump <nome>` depois. Ex.: `cwregtype BwmsTestClass` -> `rttidump BwmsTestClass`.
        ["cwregtype", name] => {
            unsafe { register::register_type_min(reg, name) };
            return;
        }
        // RegisterType STEP-2: forja uma classe INSTANCIÁVEL (alias fiel de <src>: size/vtable reais).
        // Ex.: `cwregalias BwmsAliasV PlayerPuppet` -> `newobj BwmsAliasV` (deve construir sem crash).
        ["cwregalias", name, src] => {
            unsafe { register::register_type_alias(reg, name, src) };
            return;
        }
        // RegisterType STEP-3: forja classe com PROPRIEDADE Float custom. Confirmar com `propdump <novo>`.
        // Ex.: `cwregprop BwmsPropC gameuiInGameMenuGameController myFloat` -> `propdump BwmsPropC`.
        ["cwregprop", name, src, prop] => {
            unsafe { register::register_type_with_prop(reg, name, src, prop) };
            return;
        }
        // ArchiveXL: dumpa o ResourceGameDepot VIVO (singleton *[0x109003000+0x1f8]) — vtable[0..0x20]
        // + campos candidatos. Crava o layout + ajuda a achar o slot de RESOLVE (era gated → agora live).
        ["vt50drain"] => {
            crate::selftest::drain_vt50_ring();
            return;
        }
        ["sweepdrain"] => {
            crate::selftest::drain_sweep();
            return;
        }
        ["reslinkdump"] => {
            crate::selftest::reslink_dump();
            return;
        }
        ["reslink", src, tgt] => {
            let s = u64::from_str_radix(src.trim_start_matches("0x"), 16).unwrap_or(0);
            let t = u64::from_str_radix(tgt.trim_start_matches("0x"), 16).unwrap_or(0);
            crate::selftest::reslink_set(s, t);
            return;
        }
        ["reslinkadd", src, tgt] => {
            let s = u64::from_str_radix(src.trim_start_matches("0x"), 16).unwrap_or(0);
            let t = u64::from_str_radix(tgt.trim_start_matches("0x"), 16).unwrap_or(0);
            crate::selftest::reslink_add(s, t);
            return;
        }
        ["reslinkpath", src, tgt] => {
            crate::selftest::reslink_path(src, tgt);
            return;
        }
        // `cw-controller-misc`: arma o watch de UM resource path — quando o jogo constrói esse
        // `ResourcePath` de verdade (via o hook resource.link já instalado), dispara "Resource/
        // Load" (ResourceEvent real) no próximo tick. Ex.: `watchres base\characters\...\x.mesh`.
        ["watchres", rest @ ..] => {
            crate::selftest::reslink_watch(&rest.join(" "));
            return;
        }
        ["reslinkfile", rest @ ..] => {
            crate::selftest::reslink_file(&rest.join(" "));
            return;
        }
        // axl-factories: adicionar itens. factoryadd <path.csv> enfileira um factory do mod;
        // factoryfile <arquivo> carrega N. O re-inject dispara no sentinel (último factory vanilla),
        // quando o hook estiver instalado (offset Mac de LoadFactoryAsync pendente de RE).
        ["factoryadd", rest @ ..] => {
            crate::selftest::factory_add(&rest.join(" "));
            return;
        }
        ["factoryfile", rest @ ..] => {
            crate::selftest::factory_file(&rest.join(" "));
            return;
        }
        ["reslinkstat"] => {
            crate::selftest::reslink_stat();
            return;
        }
        // `tweakxl-updaterecord` — disparo MANUAL (o auto-trigger por TICKS>120 dispara cedo demais,
        // num player-espúrio pré-load real; usar isto só DEPOIS de confirmar gameplay real, ex. via
        // `callf GetWorldPosition` retornando vec4 não-zero).
        ["updaterectest"] => {
            unsafe { crate::tweakdb_rt::prove_updaterecord() };
            return;
        }
        // `tweakxl-updaterecord` v2 — CreateTDBRecord+Assign (técnica real RED4ext/TweakXL, ver
        // tweakdb_rt::update_record_rt). Roda em THREAD SEPARADA (mesmo motivo do create_record_rt:
        // CreateTDBRecord pode pegar um spinlock; nunca chamar direto do hook do executor).
        ["updaterectest2", class_name, name] => {
            let class_name = class_name.to_string();
            let name = name.to_string();
            std::thread::spawn(move || unsafe {
                crate::tweakdb_rt::update_record_rt(&class_name, &name);
            });
            return;
        }
        // Round-trip completo do proof_needed: setflat (repoint) + UpdateRecord -> le via
        // propriedade RTTI (nao flatDataBuffer cru) sem reload. Ex.:
        // updaterecroundtrip gamedataWeaponItem_Record Items.GrenadeIncendiarySticky deepWaterDepth 7.5
        ["updaterecroundtrip", class_name, name, prop, new_val] => {
            let class_name = class_name.to_string();
            let name = name.to_string();
            let prop = prop.to_string();
            let new_val: f32 = match new_val.parse() {
                Ok(v) => v,
                Err(_) => return log(&format!("[updaterec2-rt] valor inválido '{new_val}'")),
            };
            std::thread::spawn(move || unsafe {
                crate::tweakdb_rt::prove_updaterecord_v2(&class_name, &name, &prop, new_val);
            });
            return;
        }
        // v3 (2026-07-18): observável NOVO — chama o GETTER NATIVO real do record (ex.:
        // Grenade_Record.DeepWaterDepth(), via callf já provado) antes/durante/depois do
        // UpdateRecord, em vez de tentar achar a FlatConnection cacheada por scan de memória
        // (v1/v2, sem sucesso). Ex.:
        // updaterecgetter gamedataGrenade_Record Items.GrenadeIncendiarySticky DeepWaterDepth deepWaterDepth 7.5
        ["updaterecgetter", class_name, name, getter, prop, new_val] => {
            let class_name = class_name.to_string();
            let name = name.to_string();
            let getter = getter.to_string();
            let prop = prop.to_string();
            let new_val: f32 = match new_val.parse() {
                Ok(v) => v,
                Err(_) => return log(&format!("[updaterec3] valor inválido '{new_val}'")),
            };
            std::thread::spawn(move || unsafe {
                crate::tweakdb_rt::prove_updaterecord_v3(&class_name, &name, &getter, &prop, new_val);
            });
            return;
        }
        ["inputlog", "on"] => {
            crate::overlay::input_log_set(true);
            return log("[inputlog] ON -> /tmp/cp77-input.log (tecla/mouse/scroll/mod com ts)");
        }
        ["inputlog", "off"] => {
            crate::overlay::input_log_set(false);
            return log("[inputlog] OFF");
        }
        // peekq <vmaddr-hex> [count] — lê N u64 num vmaddr ESTÁTICO (rebase p/ runtime), read-only.
        // Primitiva de RE: resolver ptr de handler global ([0x1090de530]), func-field de descritor de
        // native, etc. NÃO chama nada (sem crash). count clampado 1..32.
        ["peekq", addr, rest @ ..] => {
            let n = rest.first().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1).clamp(1, 32);
            let va = u64::from_str_radix(addr.trim_start_matches("0x"), 16).unwrap_or(0);
            if va == 0 {
                return log("[peekq] uso: peekq <vmaddr-hex> [count]");
            }
            unsafe {
                let base = crate::rebase(va) as *const u8;
                if !crate::gum::is_readable(base as *const c_void, n * 8) {
                    return log(&format!("[peekq] {va:#x} (rt {base:p}) ILEGÍVEL"));
                }
                let mut out = format!("[peekq] {va:#x} (rt {base:p}):");
                for i in 0..n {
                    let q = (base.add(i * 8) as *const u64).read();
                    out.push_str(&format!(" +{:#x}={q:#018x}", i * 8));
                }
                return log(&out);
            }
        }
        // RE offline 2026-07-13 (Facade/baseEngineInit.cpp): 0x10223a2ac é um getter de SINGLETON
        // clássico (flag lazy-init em 0x107d7e000+0x4c0, ponteiro cacheado em +0x4c8), chamado no
        // TOPO do orquestrador do assert "Failed to initialize scripts data!" (0x103d99e44) — MAS
        // toma ZERO argumentos, então dá pra chamar de QUALQUER lugar (não só de dentro do
        // orquestrador). Dump seguro (vtable + 0x20 qwords) pra descobrir que objeto é esse —
        // pode ser o próprio "engine" ou um sistema relacionado (resource/script). Gated por ser
        // uma sonda NOVA sem teste prévio: roda só sob comando explícito, sempre com is_readable.
        ["enginesys-dump"] => {
            unsafe {
                let getter_vm = crate::rebase(0x1_0223_a2ac);
                if !crate::gum::is_readable(getter_vm as *const c_void, 8) {
                    return log("[enginesys] getter ilegível -> abortado");
                }
                let f: extern "C" fn() -> *mut c_void = std::mem::transmute(getter_vm);
                let obj = f();
                if obj.is_null() || !crate::gum::is_readable(obj as *const c_void, 0x30) {
                    return log(&format!("[enginesys] obj={obj:p} null/ilegível"));
                }
                let vt = (obj as *const *const u64).read();
                let mut s = format!(
                    "[enginesys] obj={obj:p} vtable static {:#x}\n",
                    crate::un_rebase(vt as *const c_void)
                );
                // Estendido 2026-07-13 (8->16 slots): a Tentativa 7/8 da Facade achou que o
                // orquestrador do assert (0x103d99e44) chama `singleton->vtbl[0x48]` como o
                // PRIMEIRO passo, ANTES de checar [engine+0x150] — resultado que eu tinha
                // descartado como "não usado diretamente", mas o log ao vivo mostra a validação
                // de classe (12 chamadas) acontecendo AVANTA de retornar do orquestrador — ou
                // seja, ANINHADA dentro dele. +0x48 (índice 9) estava FORA do range antigo
                // (só ia até +0x38) — nunca tínhamos visto que função é essa.
                if crate::gum::is_readable(vt as *const c_void, 8 * 16) {
                    for i in 0..16usize {
                        let fp = vt.add(i).read();
                        s.push_str(&format!("  vt+{:#04x} = static {:#x}\n", i * 8, crate::un_rebase(fp as *const c_void)));
                    }
                }
                for off in [0x08usize, 0x10, 0x18, 0x20, 0x28, 0x150] {
                    if crate::gum::is_readable((obj as *const u8).add(off) as *const c_void, 8) {
                        let v = ((obj as *const u8).add(off) as *const u64).read();
                        s.push_str(&format!("  +{off:#05x} = {v:#018x}\n"));
                    } else {
                        s.push_str(&format!("  +{off:#05x} = ilegível\n"));
                    }
                }
                // Tentativa 5 (2026-07-13): +0x150 do singleton em si não bateu (0xFFFF...FFFF,
                // não é o "engine"). Segue os 2 ponteiros-filho (+0x08/+0x10) — candidatos a serem
                // o objeto engine de verdade — e dumpa vtable+[+0x150] de CADA UM.
                for child_off in [0x08usize, 0x10] {
                    if !crate::gum::is_readable((obj as *const u8).add(child_off) as *const c_void, 8) {
                        continue;
                    }
                    let child = ((obj as *const u8).add(child_off) as *const *mut u8).read();
                    s.push_str(&format!("  --- filho +{child_off:#04x} = {child:p} ---\n"));
                    if child.is_null() || !crate::gum::is_readable(child as *const c_void, 0x160) {
                        s.push_str("    ilegível/pequeno demais\n");
                        continue;
                    }
                    let cvt = (child as *const *const u64).read();
                    s.push_str(&format!("    vtable static {:#x}\n", crate::un_rebase(cvt as *const c_void)));
                    let v150 = (child.add(0x150) as *const u64).read();
                    let header = (child.add(0x150 + 0x14) as *const u32).read();
                    s.push_str(&format!(
                        "    [+0x150] = {v150:#018x}  header@+0x164={header:#010x} (size={} flag={})\n",
                        header & 0x3FFF_FFFF,
                        header >> 30
                    ));
                }
                log(&s);
            }
            return;
        }
        ["depotdump"] => {
            unsafe {
                let slot = crate::rebase(0x1_0900_3000) as *const u8;
                let depot_pp = slot.add(0x1f8) as *const *const u8;
                if !crate::gum::is_readable(depot_pp as *const c_void, 8) {
                    return log("[depot] slot ilegível");
                }
                let depot = depot_pp.read();
                if depot.is_null() || !crate::gum::is_readable(depot as *const c_void, 0x80) {
                    return log(&format!("[depot] inst={depot:p} null/ilegível (boot incompleto?)"));
                }
                let vt = (depot as *const *const u64).read();
                let mut s = format!("[depot] inst={depot:p} vtable static {:#x}\n", crate::un_rebase(vt as *const c_void));
                if crate::gum::is_readable(vt as *const c_void, 8 * 0x20) {
                    for i in 0..0x20usize {
                        let f = vt.add(i).read();
                        s.push_str(&format!("  vt+{:#04x} = static {:#x}\n", i * 8, crate::un_rebase(f as *const c_void)));
                    }
                }
                for off in [0x10usize, 0x18, 0x1c, 0x20, 0x68, 0x74] {
                    let v = (depot.add(off) as *const u64).read();
                    s.push_str(&format!("  +{:#04x} = {:#018x}\n", off, v));
                }
                log(&s);
            }
            return;
        }
        // DIAGNÓSTICO de construção RED (rodar no MENU PRINCIPAL = save-safe):
        // rttidump = só LÊ a vtable + getters (seguro); newobj = tiro único de
        // CONSTRUÇÃO (arriscado — pode crashar; por isso no menu, sem save aberto).
        ["rttidump", class] => {
            log(&unsafe { crate::rtti::dump_class(reg, class) });
            return;
        }
        ["propdump", class] => {
            log(&unsafe { crate::rtti::dump_props(reg, class) });
            return;
        }
        // TweakXL SetFlat runtime: getflat <nome> (read-only, dumpa o FlatValue vivo);
        // setflat <nome> <0xhex|int|float> (gated ~/.bwms-flatwrite; escreve 4B em +0x08).
        ["getflat", name] => {
            unsafe { crate::tweakdb_rt::probe_flat(name) };
            return;
        }
        // `cet-tweakdb-read-records`: getarr <nome> lê um flat ARRAY (ex.: attacks/statModifiers
        // de uma arma) de um record vivo, read-only. Ver tweakdb_rt::probe_array_flat.
        ["getarr", name] => {
            unsafe { crate::tweakdb_rt::probe_array_flat(name) };
            return;
        }
        ["setflat", name, val] => {
            let v: u32 = val
                .strip_prefix("0x")
                .and_then(|x| u32::from_str_radix(x, 16).ok())
                .or_else(|| val.parse::<i32>().ok().map(|i| i as u32))
                .or_else(|| val.parse::<f32>().ok().map(f32::to_bits))
                .unwrap_or(0);
            unsafe { crate::tweakdb_rt::write_flat(name, v, 0x08) };
            return;
        }
        // SetFlat NÃO-escalar (array/string/etc.): mkflat <field> <donor> <hexbytes> — cria um
        // FlatValue novo (vtable do donor, mesmo tipo) e aponta o field pra ele. Gated .bwms-flatwrite.
        ["mkflat", field, donor, hex] => {
            unsafe { crate::tweakdb_rt::mkflat_cmd(field, donor, hex) };
            return;
        }
        // SetFlat de ARRAY (armas: attacks/statModifiers): mkarr <field> <donor-array> <a,b,c>.
        // Elementos = nomes TweakDBID (ou 0xhex cru). Gated .bwms-flatwrite.
        ["mkarr", field, donor, list] => {
            unsafe { crate::tweakdb_rt::mkarr_cmd(field, donor, list) };
            return;
        }
        // `tweakxl-batch-commit`: batchset <f1>=<hex1> <f2>=<hex2> ... — aplica N sets escalares
        // TUDO-OU-NADA (se qualquer campo não resolver, aborta sem escrever nenhum). Gated
        // .bwms-flatwrite.
        ["batchset", pairs @ ..] => {
            let owned: Vec<String> = pairs.iter().map(|s| s.to_string()).collect();
            unsafe { crate::tweakdb_rt::batchset_cmd(&owned) };
            return;
        }
        // `cet-utils-shippable` (fatia `json`): readjson <path> — lê+parseia um JSON de disco
        // (config de mod típica) sem Lua. Zero risco (I/O puro, sem tocar RTTI/memória do jogo).
        ["readjson", path] => {
            match std::fs::read_to_string(path) {
                Ok(content) => match crate::cet_json::parse(&content) {
                    Ok(v) => log(&format!("[readjson] '{path}' parseado OK: {}", crate::cet_json::stringify(&v))),
                    Err(e) => log(&format!("[readjson] '{path}': erro de parse: {e}")),
                },
                Err(e) => log(&format!("[readjson] '{path}': erro de leitura: {e}")),
            }
            return;
        }
        // `cet-utils-shippable` (fatia `dir`): listdir <path> — lista arquivos de um diretório
        // sem Lua (equivalente ao módulo `dir` do CET-Windows). Zero risco (I/O puro).
        ["listdir", path] => {
            match std::fs::read_dir(path) {
                Ok(entries) => {
                    let names: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().to_string())
                        .collect();
                    log(&format!("[listdir] '{path}' ({} entradas): {:?}", names.len(), names));
                }
                Err(e) => log(&format!("[listdir] '{path}': erro: {e}")),
            }
            return;
        }
        // CLONE probe (READ-ONLY): confirma o layout do array de flats + enumera as props
        // do record-fonte e checa se cada flat "source.<prop>" é achável. NÃO muta. Passo de
        // de-risco ANTES do clone real (insert no flats). Ex:
        //   cloneprobe gamedataWeaponItem_Record Items.Preset_Lexington_Default Items.BwmsCloneTest
        ["cloneprobe", class, source, newname] => {
            unsafe { crate::tweakdb_rt::clone_probe(reg, class, source, newname) };
            return;
        }
        // CLONE USÁVEL (GATED ~/.bwms-flatwrite): herda os flats do source (stats reais via
        // InheritFlats) E registra o record novo no TweakDB vivo. Muta o array de flats sob mutex00,
        // em thread separada. Ex:
        //   clone gamedataWeaponItem_Record Items.Preset_Lexington_Default Items.BwmsLexClone
        ["clone", class, source, newname] => {
            unsafe { crate::tweakdb_rt::clone_cmd(class, source, newname) };
            return;
        }
        // `tweakxl-pipeline-runtime`: clona SEM dizer a classe (detectada automaticamente do
        // `source`) — a peça que faltava pro pipeline .yaml real do TweakXL, que nunca anota a
        // classe do `$base`. Ex.: xlautoclone Items.Preset_Lexington_Default Items.BwmsAutoClone
        ["xlautoclone", source, newname] => {
            unsafe { crate::tweakdb_rt::xlautoclone_cmd(source, newname) };
            return;
        }
        // `tweakxl-pipeline-runtime` completo: lê+parseia um .yaml REAL do TweakXL (o mesmo
        // parser do tweakdb-tool offline) e aplica no TweakDB vivo (clone/create com detecção
        // automática de classe + edits escalares). Ex.: applyxlfile /tmp/meumod.yaml
        ["applyxlfile", path] => {
            unsafe { crate::tweakdb_rt::applyxlfile_cmd(path) };
            return;
        }
        // Reflection USÁVEL (GetValue/SetValue por nome no PLAYER vivo): getf <prop> lê;
        // setf <prop> <0xhex|int|float> escreve. class_of(player) + find_property + prop_get/set.
        ["getf", prop] => {
            unsafe {
                let p = crate::rtti::find_property_in_class(crate::rtti::class_of(player), prop);
                if p.is_null() {
                    log(&format!("[getf] prop '{prop}' não achada na classe do player"));
                } else {
                    let vo = crate::rtti::prop_value_offset(p);
                    let u = crate::rtti::prop_get_u32(p, player);
                    let f = crate::rtti::prop_get_f32(p, player);
                    let b = crate::rtti::prop_get_bool(p, player);
                    log(&format!(
                        "[getf] {prop} (vo={vo:#x}) = u32 {u:#x} / i32 {} / f32 {f} / bool {b}",
                        u as i32
                    ));
                }
            }
            return;
        }
        ["setf", prop, val] => {
            unsafe {
                let p = crate::rtti::find_property_in_class(crate::rtti::class_of(player), prop);
                if p.is_null() {
                    log(&format!("[setf] prop '{prop}' não achada"));
                } else {
                    let v: u32 = val
                        .strip_prefix("0x")
                        .and_then(|x| u32::from_str_radix(x, 16).ok())
                        .or_else(|| val.parse::<i32>().ok().map(|i| i as u32))
                        .or_else(|| val.parse::<f32>().ok().map(f32::to_bits))
                        .unwrap_or(0);
                    let before = crate::rtti::prop_get_u32(p, player);
                    crate::rtti::prop_set_u32(p, player, v);
                    let after = crate::rtti::prop_get_u32(p, player);
                    log(&format!("[setf] {prop}: {before:#x} -> {after:#x} (pediu {v:#x})"));
                }
            }
            return;
        }
        // Reflection CALL com ARGS tipados: chama um método do player por nome, args =
        // `i:5 f:1.5 b:true n:Name s:txt e:3` (I32/F32/Bool/CName/Str/Enum). Sem args = getter.
        // Marshaling = o mesmo call_func dos cheats (provado). Completa get/set/call.
        ["callf", method, raw_args @ ..] => {
            unsafe {
                let args: Vec<crate::rtti::Arg> = raw_args.iter().map(|a| parse_cmd_arg(a)).collect();
                match crate::rtti::resolve_in_class(crate::rtti::class_of(player), method) {
                    Some(rf) => match crate::rtti::call_func(&rf, player, &args) {
                        // call_func devolve [u8;16] (sret). Interpreta como escalar (4B) E como
                        // Vector4/struct-de-16B (4×f32) — assim retornos como GetWorldPosition()→Vector4,
                        // GetWorldForward(), GetWorldOrientation()→Quaternion saem legíveis (Fase 1 do
                        // roadmap IA: posição/orientação do V por nome, sem RE nova).
                        Some(r) => {
                            let f = |i: usize| f32::from_bits(u32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]));
                            log(&format!(
                                "[callf] {method}({}) -> i32 {} / u32 {:#x} / f32 {} / vec4 [{:.3}, {:.3}, {:.3}, {:.3}] / bytes {:02x?}",
                                raw_args.join(" "),
                                i32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                f(0),
                                f(0), f(4), f(8), f(12),
                                &r[..16]
                            ));
                        }
                        None => log(&format!("[callf] {method}({}) não completou (void ou falha)", raw_args.join(" "))),
                    },
                    None => log(&format!("[callf] método '{method}' não achado na classe do player")),
                }
            }
            return;
        }
        // callf num objeto ARBITRÁRIO (não só o player) — fecha o gap `cw-register-class-rtti`:
        // registrar (cwregtype/cwregalias) -> instanciar (newobj, devolve o ponteiro) -> CHAMAR
        // MÉTODO nessa instância. `callon <0xptr> <method> [args]`; class_of(ptr) funciona em
        // QUALQUER objeto válido (lê vtable+8=GetType), inclusive o forjado (vtable clonada).
        ["callon", ptr_hex, method, raw_args @ ..] => {
            unsafe {
                let addr = ptr_hex
                    .strip_prefix("0x")
                    .and_then(|x| u64::from_str_radix(x, 16).ok())
                    .unwrap_or(0);
                let obj = addr as *mut c_void;
                let cls = crate::rtti::class_of(obj);
                if cls.is_null() {
                    log(&format!("[callon] ponteiro {ptr_hex} não é objeto válido (class_of falhou)"));
                    return;
                }
                let args: Vec<crate::rtti::Arg> = raw_args.iter().map(|a| parse_cmd_arg(a)).collect();
                match crate::rtti::resolve_in_class(cls, method) {
                    Some(rf) => match crate::rtti::call_func(&rf, obj, &args) {
                        Some(r) => {
                            let f = |i: usize| f32::from_bits(u32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]));
                            log(&format!(
                                "[callon] {ptr_hex}.{method}({}) -> i32 {} / u32 {:#x} / f32 {} / vec4 [{:.3}, {:.3}, {:.3}, {:.3}]",
                                raw_args.join(" "),
                                i32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                f(0),
                                f(0), f(4), f(8), f(12)
                            ));
                        }
                        None => log(&format!("[callon] {ptr_hex}.{method}({}) não completou (void ou falha)", raw_args.join(" "))),
                    },
                    None => log(&format!("[callon] método '{method}' não achado na classe do objeto {ptr_hex}")),
                }
            }
            return;
        }
        // Reflection CALL de função GLOBAL por nome (com args tipados). Complementa callf (instância).
        // Ex.: `callg Cos f:0.0` -> 1.0 ; `callg SqrtF f:4.0` -> 2.0. get_function + call_func (provados).
        ["callg", name, raw_args @ ..] => {
            unsafe {
                let args: Vec<crate::rtti::Arg> = raw_args.iter().map(|a| parse_cmd_arg(a)).collect();
                let f = register::get_function(reg, name);
                if !crate::rtti::sane(f) {
                    log(&format!("[callg] global '{name}' não resolveu"));
                } else {
                    let rf = crate::rtti::ResolvedFn {
                        func: f,
                        ret_type: std::ptr::null_mut(),
                        is_static: true,
                    };
                    match crate::rtti::call_func(&rf, std::ptr::null_mut(), &args) {
                        Some(r) => log(&format!(
                            "[callg] {name}({}) -> f32 {} / i32 {} / u32 {:#x} / bytes {:02x?}",
                            raw_args.join(" "),
                            f32::from_bits(u32::from_le_bytes([r[0], r[1], r[2], r[3]])),
                            i32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                            u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                            &r[..8]
                        )),
                        None => log(&format!("[callg] {name}({}) não completou", raw_args.join(" "))),
                    }
                }
            }
            return;
        }
        // `cet-game-reflect-bridge` DISPATCH (in-game): cola uma linha CET `Namespace.Method(args)`,
        // parseia via `parse_cet_call` (tokenizer+inferência offline provados), marshalha cada
        // CetArg pro `rtti::Arg` tipado e DISPATCHA — resolve como MÉTODO na classe `namespace`
        // (callf-style, no player) OU como GLOBAL `method` (callg-style). Cobre o subconjunto de
        // linhas CET que mapeiam direto pra função/método do RTTI (não os bindings especiais `Game.*`
        // do Lua, que roteiam pra N pontos distintos — esses seguem no `parse_cet_line` give/money).
        // Ex.: cetcall PlayerPuppet.IsDead()  |  cetcall Vector4.Length(...)
        ["cetcall", rest @ ..] => {
            let line = rest.join(" ");
            match parse_cet_call(&line) {
                None => log(&format!("[cetcall] '{line}' não é Namespace.Method(args)")),
                Some(call) => unsafe {
                    let args: Vec<crate::rtti::Arg> = call
                        .args
                        .iter()
                        .map(|a| match a {
                            CetArg::Str(s) => crate::rtti::Arg::Str(s.clone()),
                            CetArg::Int(i) => crate::rtti::Arg::I32(*i as u32),
                            CetArg::Float(f) => crate::rtti::Arg::F32(*f as f32),
                            CetArg::Bool(b) => crate::rtti::Arg::Bool(*b),
                            CetArg::Ident(s) => crate::rtti::Arg::CName(crate::cname::cname(s)),
                        })
                        .collect();
                    let fmt = |r: [u8; 32]| {
                        let w = u32::from_le_bytes([r[0], r[1], r[2], r[3]]);
                        format!("i32 {} / u32 {:#x} / f32 {}", w as i32, w, f32::from_bits(w))
                    };
                    // 1) tenta como método na classe `namespace` (chama no player)
                    if let Some(rf) = crate::rtti::resolve_func(reg, &call.namespace, &call.method) {
                        match crate::rtti::call_func(&rf, player, &args) {
                            Some(r) => log(&format!("[cetcall] {}.{}(...) [método] -> {}", call.namespace, call.method, fmt(r))),
                            None => log(&format!("[cetcall] {}.{} [método] não completou", call.namespace, call.method)),
                        }
                    } else {
                        // 2) tenta como global `method`
                        let f = register::get_function(reg, &call.method);
                        if crate::rtti::sane(f) {
                            let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                            match crate::rtti::call_func(&rf, std::ptr::null_mut(), &args) {
                                Some(r) => log(&format!("[cetcall] {}(...) [global] -> {}", call.method, fmt(r))),
                                None => log(&format!("[cetcall] {} [global] não completou", call.method)),
                            }
                        } else {
                            log(&format!("[cetcall] {}.{} não resolveu (nem método da classe nem global)", call.namespace, call.method));
                        }
                    }
                }
            }
            return;
        }
        // `cet-call-power-tool`: `sig <Class> <method>` imprime a assinatura (nº params + tipos +
        // tipo de retorno) sem chamar nada — útil pra descobrir a forma antes de arriscar `call`.
        // Reusa param_count/fn_ret_type/fn_param_type (já provados) + resolve_cname (hash->nome).
        ["sig", class, method] => {
            unsafe {
                match crate::rtti::resolve_func(reg, class, method) {
                    Some(rf) => {
                        let n = crate::rtti::param_count(&rf) as usize;
                        let ret_ty = crate::rtti::fn_ret_type(rf.func);
                        let ret_name = if ret_ty.is_null() {
                            "Void".to_string()
                        } else {
                            crate::cname::resolve_cname(crate::rtti::type_name_getname(ret_ty))
                        };
                        let params: Vec<String> = (0..n)
                            .map(|i| crate::cname::resolve_cname(crate::rtti::fn_param_type(rf.func, i)))
                            .collect();
                        log(&format!(
                            "[sig] {class}.{method}({}) -> {ret_name}  (static={}, params={n})",
                            params.join(", "),
                            rf.is_static
                        ));
                    }
                    None => log(&format!("[sig] {class}.{method} não resolveu (classe ou método não achado)")),
                }
            }
            return;
        }
        // `cet-call-power-tool`/`cet-game-reflect-bridge`: `call <Class> <method> [args]` — chama
        // por NOME DE CLASSE (namespace), ctx=null (chamada estática). Cobre o padrão mais comum
        // do `Game.*` do CET (TDBID.*, ItemID.*, GameInstance.Get*System, Cast<>, etc — funções de
        // classe/utilitário, não métodos de INSTÂNCIA arbitrária — pra isso já existem `callf`
        // (objeto capturado) e `callon` (ponteiro explícito), ambos provados). Resolve via
        // resolve_func (reg.class_by_name + resolve_in_class, já provados) + auto-marshalling de
        // args (parse_cmd_arg, o mesmo de callf/callg).
        ["call", class, method, raw_args @ ..] => {
            unsafe {
                match crate::rtti::resolve_func(reg, class, method) {
                    Some(rf) => {
                        let args: Vec<crate::rtti::Arg> = raw_args.iter().map(|a| parse_cmd_arg(a)).collect();
                        // FIX 2026-07-16: método de INSTÂNCIA precisa do `this` (o player); passar
                        // null crashava (deref de null no corpo do método, ex. IsDead). Só static
                        // usa null. `sig` já expõe `static=`; aqui reusamos `rf.is_static`.
                        let ctx = if rf.is_static { std::ptr::null_mut() } else { player };
                        match crate::rtti::call_func(&rf, ctx, &args) {
                            Some(r) => {
                                let f = |i: usize| f32::from_bits(u32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]));
                                log(&format!(
                                    "[call] {class}.{method}({}) -> i32 {} / u32 {:#x} / f32 {} / bytes {:02x?}",
                                    raw_args.join(" "),
                                    i32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                    u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                    f(0),
                                    &r[..8]
                                ));
                            }
                            None => log(&format!("[call] {class}.{method}({}) não completou (void, ctx exigido, ou falha)", raw_args.join(" "))),
                        }
                    }
                    None => log(&format!("[call] método '{class}.{method}' não achado")),
                }
            }
            return;
        }
        // `cet-lut-pixel-proof`: seta o preset de LUT por comando (sem depender de F2/clique
        // ImGui) — `lut 0` (off) / `lut 3` (P&B) / etc. Mesmo write atômico do clique na aba "LUT".
        ["lut", n] => {
            let idx = n.parse::<u32>().unwrap_or(0);
            overlay::set_lut_preset(idx);
            log(&format!("[lut] preset setado via comando -> {idx}"));
            return;
        }
        // `cw-rawinput-realname` (2026-07-18) — dispara UM keyDown+keyUp sintético via CGEvent
        // (mesmo `cg_press` já provado no auto-proceed do skip-intro, `selfboot.rs:663`), pra
        // testar o RawInput controller EM GAMEPLAY (o auto-proceed já para de apertar SPACE
        // assim que o menu é alcançado — não sobrevive até depois do registro do callback de
        // teste). Só existe com `--features autoproceed` (dev); build público não importa
        // CGEventPost/CreateKeyboardEvent. `presskey 49` = SPACE.
        #[cfg(feature = "autoproceed")]
        ["presskey", kc] => {
            let keycode = kc.parse::<u16>().unwrap_or(49);
            unsafe { overlay::cg_press(keycode) };
            log(&format!("[presskey] disparado keyDown+keyUp sintético, keycode={keycode}"));
            return;
        }
        // `cw-real-mod-e2e`/`axl-link-visual-proof` (2026-07-19): navegação de menu por MOUSE via
        // CGEvent DELTA relativo, disparado de DENTRO do processo do jogo (mesma permissão/receita
        // de `presskey`/`cg_press` — evita o gate de Acessibilidade que bloqueia um processo
        // externo). `mousedelta <dx> <dy>` move o cursor RENDERIZADO PELO JOGO (não o do SO).
        #[cfg(feature = "autoproceed")]
        ["mousedelta", dx, dy] => {
            let dx = dx.parse::<i64>().unwrap_or(0);
            let dy = dy.parse::<i64>().unwrap_or(0);
            unsafe { overlay::cg_mouse_delta(dx, dy) };
            log(&format!("[mousedelta] disparado dx={dx} dy={dy}"));
            return;
        }
        // `mouseclick` — mouseDown+mouseUp do botão esquerdo na posição atual (pós-mousedelta).
        #[cfg(feature = "autoproceed")]
        ["mouseclick"] => {
            unsafe { overlay::cg_click() };
            log("[mouseclick] disparado mouseDown+mouseUp");
            return;
        }
        // smoke-test do Pilar 2 (CNamePool::Get): resolve um hash CName -> nome via pool
        // NATIVO. `cname 0x23427ae352f89652` deve dar "GetStatValue"; `cname 0` -> "None".
        ["cname", h] => {
            let hash = h
                .strip_prefix("0x")
                .and_then(|x| u64::from_str_radix(x, 16).ok())
                .or_else(|| h.parse::<u64>().ok())
                .unwrap_or(0);
            let name = crate::cname::resolve_cname(hash);
            log(&format!("[cname] {hash:#018x} -> '{name}'"));
            return;
        }
        ["newobj", class] => {
            log(&format!("[newobj] tentando construir '{class}' ..."));
            let p = unsafe { crate::rtti::new_object(reg, class) };
            log(&format!(
                "[newobj] '{class}' -> {:p} (static {:#x}) {}",
                p,
                un_rebase(p),
                if p.is_null() { "NULL (resolve/size falhou, sem crash)" } else { "OK (nao crashou)" }
            ));
            return;
        }
        _ => {}
    }
    // Tradução de linha CET (QoL): códigos colados da internet funcionam SEM Lua.
    // `Game.AddToInventory("Items.X", 5)` -> give | `Game.AddMoney(7777)` -> money.
    if let Some((name, n)) = parse_cet_line(cmd) {
        let r = unsafe { console::give(reg, player, tx, &name, n) };
        match r {
            Some(res) if res[0] != 0 => log(&format!("[console] CET '{cmd}' -> give {name} x{n} OK")),
            _ => log(&format!("[console] CET '{cmd}' -> give {name} x{n} FALHOU")),
        }
        return;
    }
    let r = unsafe {
        match parts.as_slice() {
            ["money", n] => console::give(reg, player, tx, "Items.money", n.parse().unwrap_or(1)),
            ["give", name] => console::give(reg, player, tx, name, 1),
            ["give", name, n] => console::give(reg, player, tx, name, n.parse().unwrap_or(1)),
            ["remove", name] => console::remove(reg, player, tx, name, 1),
            ["remove", name, n] => console::remove(reg, player, tx, name, n.parse().unwrap_or(1)),
            _ => {
                // CET-style: o console É um REPL Lua. Comando não-reconhecido roda
                // como Lua — digitar `Game.AddMoney(7777)` direto funciona, igual CET.
                log(&format!("[console] (lua) {cmd}"));
                lua::run_code(cmd);
                return;
            }
        }
    };
    match r {
        Some(res) if res[0] != 0 => log(&format!("[console] '{cmd}' -> OK (GiveItem ret={})", res[0])),
        Some(res) => log(&format!(
            "[console] '{cmd}' -> NO-OP (GiveItem retornou {} — owner/tx errado?)",
            res[0]
        )),
        None => log(&format!("[console] '{cmd}' -> FALHOU (resolve/from_tdbid/call)")),
    }
}

/// Um argumento tipado de uma linha CET (`cet-game-reflect-bridge`): a inferência que decide se
/// `42` é int, `1.5` float, `"x"` string, `true` bool, ou um identificador cru (ex.: `TDBID.Create(...)`,
/// nome de enum) a ser resolvido em runtime.
#[derive(Debug, Clone, PartialEq)]
enum CetArg {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Ident(String),
}

/// Uma chamada CET tokenizada: `Namespace.Method(arg, ...)` (também aceita `:` como separador, ex.:
/// `TweakDB:GetFlat`). A DISPATCH pro RTTI do jogo é in-game; a tokenização+inferência é offline.
#[derive(Debug, Clone, PartialEq)]
struct CetCall {
    namespace: String,
    method: String,
    args: Vec<CetArg>,
}

/// Separa os args de nível-TOPO respeitando aspas (`"`/`'`, com escape `\`) e profundidade de
/// parênteses/colchetes — `Game.X(ItemID.FromTDBID(TDBID.Create("a")), 1)` vira 2 args, não 4.
fn split_top_level_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            cur.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                cur.push(c);
            }
            '(' | '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let last = cur.trim();
    if !last.is_empty() {
        out.push(last.to_string());
    }
    out
}

/// Infere o tipo de UM token de argumento CET.
fn infer_cet_arg(tok: &str) -> CetArg {
    let t = tok.trim();
    let quoted = t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')));
    if quoted {
        return CetArg::Str(t[1..t.len() - 1].to_string());
    }
    match t {
        "true" => return CetArg::Bool(true),
        "false" => return CetArg::Bool(false),
        _ => {}
    }
    if let Ok(i) = t.parse::<i64>() {
        return CetArg::Int(i);
    }
    if let Ok(f) = t.parse::<f64>() {
        return CetArg::Float(f);
    }
    CetArg::Ident(t.to_string())
}

/// Tokeniza uma linha CET genérica `Namespace.Method(arg, ...)` → [`CetCall`]. `None` se não parece
/// uma chamada (sem `(`/`)` casados ou sem `Namespace.Method`). Base offline pra colar QUALQUER
/// linha CET no build 0%-Lua (a dispatch pro jogo é o passo in-game).
fn parse_cet_call(line: &str) -> Option<CetCall> {
    let s = line.trim();
    let open = s.find('(')?;
    let head = s[..open].trim();
    let inner = s[open + 1..].trim_end().strip_suffix(')')?;
    let sep = head.rfind(['.', ':'])?;
    let namespace = head[..sep].trim().to_string();
    let method = head[sep + 1..].trim().to_string();
    if namespace.is_empty() || method.is_empty() {
        return None;
    }
    let args = split_top_level_args(inner)
        .iter()
        .map(|a| infer_cet_arg(a))
        .collect();
    Some(CetCall { namespace, method, args })
}

/// Linha CET → (item, qtd), o par que o comando `give` do BWMS entende (build 0%-Lua). Aceita
/// `Game.AddToInventory("Items.X", 5)` (aspas '/"', qtd opcional=1) e `Game.AddMoney(7777)` →
/// ("Items.money", 7777). None = não é uma dessas duas. Agora construído sobre [`parse_cet_call`]
/// (o tokenizer genérico) em vez de string-slicing ad-hoc.
fn parse_cet_line(cmd: &str) -> Option<(String, u32)> {
    let call = parse_cet_call(cmd)?;
    if call.namespace != "Game" {
        return None;
    }
    match (call.method.as_str(), call.args.as_slice()) {
        ("AddMoney", [CetArg::Int(n)]) => Some(("Items.money".to_string(), *n as u32)),
        ("AddToInventory", [CetArg::Str(name)]) if !name.is_empty() => Some((name.clone(), 1)),
        ("AddToInventory", [CetArg::Str(name), CetArg::Int(q)]) if !name.is_empty() => {
            Some((name.clone(), *q as u32))
        }
        _ => None,
    }
}

#[cfg(test)]
mod arg_marshalling_tests {
    use super::{parse_bool_flex, parse_cmd_arg_checked, parse_u32_flex, parse_u64_flex};
    use crate::rtti::Arg;

    #[test]
    fn int_flex_aceita_hex_dec_e_sinal() {
        assert_eq!(parse_u32_flex("255"), Some(255));
        assert_eq!(parse_u32_flex("0xFF"), Some(255));
        assert_eq!(parse_u32_flex("0xff"), Some(255));
        assert_eq!(parse_u32_flex("-1"), Some(0xFFFF_FFFF)); // i32 -1 → u32
        assert_eq!(parse_u32_flex("4000000000"), Some(4_000_000_000)); // > i32::MAX
        assert_eq!(parse_u32_flex("0xFFFFFFFF"), Some(0xFFFF_FFFF));
        assert_eq!(parse_u32_flex("nope"), None);
        assert_eq!(parse_u64_flex("0x1_0000_0000".replace('_', "").as_str()), Some(0x1_0000_0000));
    }

    #[test]
    fn bool_flex_tolerante_case_insensitive() {
        for t in ["true", "TRUE", "True", "1", "yes", "Y", "on"] {
            assert_eq!(parse_bool_flex(t), Some(true), "{t}");
        }
        for f in ["false", "FALSE", "0", "no", "off"] {
            assert_eq!(parse_bool_flex(f), Some(false), "{f}");
        }
        assert_eq!(parse_bool_flex("maybe"), None);
    }

    #[test]
    fn checked_prefixo_desconhecido_erra_nao_zera() {
        // O CERNE do fix: um prefixo com typo NÃO pode virar I32(0) mudo dentro de uma chamada viva.
        assert!(parse_cmd_arg_checked("ii:5").is_err());
        assert!(parse_cmd_arg_checked("x:1").is_err());
        assert!(parse_cmd_arg_checked("i:notanint").is_err());
        assert!(parse_cmd_arg_checked("b:maybe").is_err());
        assert!(parse_cmd_arg_checked("semprefixo_nem_int").is_err());
    }

    #[test]
    fn checked_tipos_corretos() {
        match parse_cmd_arg_checked("i:0x10").unwrap() {
            Arg::I32(v) => assert_eq!(v, 16),
            _ => panic!("esperava I32"),
        }
        match parse_cmd_arg_checked("f:1.5").unwrap() {
            Arg::F32(v) => assert_eq!(v, 1.5),
            _ => panic!("esperava F32"),
        }
        match parse_cmd_arg_checked("b:TRUE").unwrap() {
            Arg::Bool(v) => assert!(v),
            _ => panic!("esperava Bool"),
        }
        match parse_cmd_arg_checked("e:0xFF").unwrap() {
            Arg::Enum(v) => assert_eq!(v, 255),
            _ => panic!("esperava Enum"),
        }
        match parse_cmd_arg_checked("s:hello:world").unwrap() {
            Arg::Str(v) => assert_eq!(v, "hello:world"), // '/' e ':' preservados no valor
            _ => panic!("esperava Str"),
        }
        match parse_cmd_arg_checked("42").unwrap() {
            Arg::I32(v) => assert_eq!(v, 42), // i32 cru sem prefixo
            _ => panic!("esperava I32"),
        }
    }
}

#[cfg(test)]
mod cet_line_tests {
    use super::{infer_cet_arg, parse_cet_call, parse_cet_line, CetArg};
    #[test]
    fn traduz_linhas_cet() {
        assert_eq!(
            parse_cet_line(r#"Game.AddToInventory("Items.Preset_Yasha_Default", 1)"#),
            Some(("Items.Preset_Yasha_Default".into(), 1))
        );
        assert_eq!(
            parse_cet_line("Game.AddToInventory('Items.money', 5000)"),
            Some(("Items.money".into(), 5000))
        );
        assert_eq!(parse_cet_line(r#"Game.AddToInventory("Items.X")"#), Some(("Items.X".into(), 1)));
        assert_eq!(parse_cet_line("Game.AddMoney(7777)"), Some(("Items.money".into(), 7777)));
        assert_eq!(parse_cet_line("give Items.X"), None); // comando nosso ≠ linha CET
        assert_eq!(parse_cet_line(r#"Game.AddToInventory("", 1)"#), None);
    }

    #[test]
    fn tokeniza_chamada_generica() {
        // Namespace.Method + args tipados (int/float/bool/string/ident).
        let c = parse_cet_call(r#"Game.SetLevel("StrengthSkill", 20, 3.5, true, gamedataProficiencyType.Combat)"#).unwrap();
        assert_eq!(c.namespace, "Game");
        assert_eq!(c.method, "SetLevel");
        assert_eq!(
            c.args,
            vec![
                CetArg::Str("StrengthSkill".into()),
                CetArg::Int(20),
                CetArg::Float(3.5),
                CetArg::Bool(true),
                CetArg::Ident("gamedataProficiencyType.Combat".into()),
            ]
        );
    }

    #[test]
    fn args_aninhados_nao_quebram_no_virgula_interno() {
        // vírgula DENTRO de call aninhada + string com vírgula não devem separar no topo.
        let c = parse_cet_call(r#"Game.AddToInventory(ItemID.FromTDBID(TDBID.Create("Items.X")), 1)"#).unwrap();
        assert_eq!(c.args.len(), 2, "2 args no topo, não os internos");
        assert_eq!(c.args[1], CetArg::Int(1));
        let c2 = parse_cet_call(r#"Foo.Bar("a,b,c", 2)"#).unwrap();
        assert_eq!(c2.args, vec![CetArg::Str("a,b,c".into()), CetArg::Int(2)]);
    }

    #[test]
    fn separador_dois_pontos_e_sem_args() {
        // `:` como separador (ex.: TweakDB:GetFlat) + chamada sem argumentos.
        let c = parse_cet_call("TweakDB:GetFlat()").unwrap();
        assert_eq!((c.namespace.as_str(), c.method.as_str()), ("TweakDB", "GetFlat"));
        assert!(c.args.is_empty());
        // namespace ponto-separado com método no fim.
        let c2 = parse_cet_call("Game.GetPlayer()").unwrap();
        assert_eq!((c2.namespace.as_str(), c2.method.as_str()), ("Game", "GetPlayer"));
    }

    #[test]
    fn nao_e_chamada() {
        assert_eq!(parse_cet_call("give Items.X"), None); // sem parênteses
        assert_eq!(parse_cet_call("Foo(1)"), None); // sem Namespace.Method
        assert_eq!(parse_cet_call("Game.Foo(1"), None); // parêntese não fechado
    }

    #[test]
    fn inferencia_de_tipo() {
        assert_eq!(infer_cet_arg("42"), CetArg::Int(42));
        assert_eq!(infer_cet_arg("-7"), CetArg::Int(-7));
        assert_eq!(infer_cet_arg("3.14"), CetArg::Float(3.14));
        assert_eq!(infer_cet_arg("true"), CetArg::Bool(true));
        assert_eq!(infer_cet_arg("false"), CetArg::Bool(false));
        assert_eq!(infer_cet_arg("\"hi\""), CetArg::Str("hi".into()));
        assert_eq!(infer_cet_arg("'hi'"), CetArg::Str("hi".into()));
        assert_eq!(infer_cet_arg("EnumName.Value"), CetArg::Ident("EnumName.Value".into()));
    }
}

/// Lê os ponteiros capturados pela sonda de /tmp/cp77-inst.txt ("player=0x…", "tx=0x…").
fn read_inst() -> (*mut c_void, *mut c_void) {
    let s = std::fs::read_to_string("/tmp/cp77-inst.txt").unwrap_or_default();
    let mut p: *mut c_void = std::ptr::null_mut();
    let mut t: *mut c_void = std::ptr::null_mut();
    for line in s.lines() {
        if let Some(v) = line.trim().strip_prefix("player=0x") {
            p = usize::from_str_radix(v.trim(), 16).unwrap_or(0) as *mut c_void;
        }
        if let Some(v) = line.trim().strip_prefix("tx=0x") {
            t = usize::from_str_radix(v.trim(), 16).unwrap_or(0) as *mut c_void;
        }
    }
    (p, t)
}
