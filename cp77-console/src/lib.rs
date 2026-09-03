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
mod district;
mod crashreport;
// RASCUNHO (2026-08-11, prep CET `FunctionOverride`/`PENDENCIAS-UNIFICADAS.md`): hook nativo
// (não-Lua) de função redscript por classe+método, reusando o executor `exec_replacement` já
// hookado — ver doc-comment no topo de `fnoverride.rs`. Compilado offline, NUNCA integrado nos
// pontos de chamada (`exec_replacement`/`api.rs`) nem testado ao vivo — módulo isolado, zero
// efeito em runtime até alguém ligar os 3 pontos de integração documentados no arquivo.
mod fnoverride;
mod gum;
mod i18n;
// RASCUNHO (2026-08-11, prep CET `DumpVTablesTask`/`PENDENCIAS-UNIFICADAS.md` item `#44`): dump
// de vtable→nome-de-classe por construção real de instância, versão BOUNDED/filtrada (não o
// "tudo de uma vez" do CET original) — ver doc-comment no topo de `vtabledump.rs`. Compilado
// offline, comando `vtabledump` wired no match de comandos (gated `dev_mode()`), NUNCA testado
// ao vivo.
mod vtabledump;
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
mod mod_pipeline;
mod mod_scan;
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
pub(crate) fn game_base() -> usize {
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
/// Os offsets estáticos do projeto são da build STEAM. No GOG e no Epic (mesma versão,
/// layout deslocado) traduz Steam-vmaddr → vmaddr do build via [`steam_to_gog`]/
/// [`steam_to_epic`] ANTES de aplicar o slide. Steam/Unknown = identidade (comportamento
/// histórico, byte-idêntico).
/// True na PRIMEIRA vez que este vmaddr aparece; false depois.
///
/// Sem isto, um endereço não-mapeado que é consultado TODO TICK (o budget do watchdog do motor
/// é o caso real) enche o log com a mesma linha milhares de vezes e — pior — inunda a view
/// K-LOG do console in-game, que passa a ser inútil. O log de "sem mapa" é diagnóstico: a
/// informação está na PRIMEIRA ocorrência, as repetições não acrescentam nada.
fn first_time_unmapped(vmaddr: u64) -> bool {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map(|mut s| s.insert(vmaddr))
        .unwrap_or(false)
}

pub(crate) fn rebase(vmaddr: u64) -> *mut c_void {
    let v = match game_build() {
        // O mapa do Epic é PARCIAL de propósito (ver [`steam_to_epic`]): o `None` aqui é o
        // caminho ESPERADO pra toda feature ainda não mapeada, não um erro.
        GameBuild::Epic => match steam_to_epic(vmaddr) {
            Some(v) => v,
            None => {
                if first_time_unmapped(vmaddr) {
                    log(&format!(
                        "[rebase] vmaddr {vmaddr:#x} sem mapa Epic -> SKIP (null; feature inerte)"
                    ));
                }
                return core::ptr::null_mut();
            }
        },
        GameBuild::Gog => match steam_to_gog(vmaddr) {
            Some(v) => v,
            None => {
                // CORRIGIDO 2026-07-31 (crash real confirmado): a suposição antiga era que
                // "passthrough" (devolver o vmaddr Steam cru) era seguro porque os call-sites
                // checam prólogo/readable antes de tocar. FALSO — um vmaddr Steam interpretado
                // como offset de arquivo no binário GOG (layout DIFERENTE) pode calhar numa
                // sequência de bytes que PARECE um prólogo ARM64 válido (padrão comum,
                // `stp`/`sub sp`) sem SER o alvo certo; um `Interceptor::replace/attach` ali
                // corrompe código real do executável (confirmado: crash em
                // `dyld4::Loader::runInitializersBottomUp`, dentro do binário principal —
                // assinatura de patch aplicado em local errado, não de null-deref comum).
                // Fix: nunca mais devolver o vmaddr cru — retorna null, que todo call-site já
                // trata como "não instala" via `gum::is_readable` (retorna false pra null).
                // 1x por vmaddr (ver `first_time_unmapped`): um endereço consultado todo tick
                // repetiria esta linha milhares de vezes e inundaria o K-LOG do console.
                if first_time_unmapped(vmaddr) {
                    log(&format!("[rebase] vmaddr {vmaddr:#x} sem mapa GOG -> SKIP (null; nunca mais passthrough)"));
                }
                return core::ptr::null_mut();
            }
        },
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

/// Qual build do jogo. Steam, GOG e Epic = MESMA versão/instruções, layout diferente → os
/// vmaddr estáticos deslocam. Detectado 1x lendo o prólogo do executor; auto-validante.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GameBuild {
    Steam,
    Gog,
    Epic,
    Unknown,
}

/// Prólogo do executor (`stp x28,x27..; stp x26,x25..`). IGUAL nos 3 builds — só o
/// ENDEREÇO muda → serve de assinatura pra identificar o build. (= selfboot::EXEC_PROLOGUE.)
const DETECT_EXEC_PROLOGUE: u64 = 0xa901_67fa_a9ba_6ffc;
const STEAM_EXEC_VM: u64 = 0x1_0217_3120;
const GOG_EXEC_VM: u64 = 0x1_027b_a1b4;
/// Epic (`com.cdprojektred.cyberpunk.egs`, 2.3.1 build 5314028). Achado por backtrace no
/// `redDispatcher2` (breakpoint em `funcOperatorAdd<int>`): é o ÚNICO frame de 5 args da
/// cadeia (`x0..x4` salvos no prólogo = func, ctx, frame, res, retType). O `DETECT_EXEC_PROLOGUE`
/// acima casa byte-a-byte aqui — confirmação independente, já que o endereço NÃO foi achado
/// por essa assinatura.
const EPIC_EXEC_VM: u64 = 0x1_0422_5588;

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
        } else if probe(EPIC_EXEC_VM) {
            GameBuild::Epic
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

/// True se o build é reconhecido (Steam, GOG ou Epic). `Unknown` = versão do jogo que os nossos offsets
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

/// Mapa Steam-vmaddr → Epic-vmaddr (`com.cdprojektred.cyberpunk.egs`, 2.3.1 build 5314028).
/// Mesma versão/instruções; o layout desloca porque o Epic linka `libGameServicesEpic`/EOS
/// no lugar do Galaxy. O delta NÃO é uniforme — medido: `+0x29A8478`, `+0x271C804` e um
/// NEGATIVO (`-0x1ADDCE4`) → é reordenação real por objeto, nenhum rebase único resolve.
///
/// PARCIAL (12 de 26 pares): só entram endereços VERIFICADOS. Todo o resto cai no `None` do
/// [`rebase`], que devolve null → o call-site não instala o hook. Isso é seguro por desenho
/// (mesma garantia que o GOG já usa desde o fix de 2026-07-31); o custo é que as features
/// não-mapeadas (TweakDB, archives, depot, boot-phase) ficam INERTES no Epic, não que
/// crashem.
///
/// Como cada par foi obtido (notes: `bwms-epic-re/findings.md`):
///   - EXEC: backtrace do `redDispatcher2`; único frame de 5 args + `DETECT_EXEC_PROLOGUE`.
///   - CRTTISystem::Get: previsto pela distância EXEC↔CRTTI (Steam `0x15d6c`, GOG `0x15d74`
///     → mesmo objeto) e CONFIRMADO em runtime — chamado via lldb, a vtable do objeto
///     devolvido tem 10/10 slots com código real (o candidato descartado tinha 1/10, o resto
///     eram stubs `ret`).
///   - GetFunction: lido direto do slot +0x30 dessa vtable viva.
///   - bind*: `BINDSIG` achado por assinatura de prólogo em janela graduada; os outros por
///     distância relativa a ele — `BIND_ORCH − CLASS_VALIDATE` bate EXATO (`0x8C4`) e as duas
///     previsões caíram em entrada de função (prólogo logo depois de um `ret`).
///   - Pool*/PoolArchive: símbolo (`nm` — o binário Epic NÃO é stripped, 68654 símbolos).
///   - open-archive: vizinho de `PoolArchive::Allocate` (`0x5a8` de distância), hit exato.
///   - OPCODE_TABLE: disasm de `funcOperatorAdd<int>`; as 7417 referências à página batem
///     todas nesse mesmo endereço.
fn steam_to_epic(s: u64) -> Option<u64> {
    Some(match s {
        0x1_0217_3120 => 0x1_0422_5588, // EXEC (executor universal)
        0x1_0218_8e8c => 0x1_0423_b2c8, // CRTTISystem::Get
        0x1_0219_5024 => 0x1_0424_7370, // GetFunction (vtbl+0x30)
        0x1_021f_cee0 => 0x1_042a_e748, // bind orchestrator (resolve-log)
        0x1_021e_897c => 0x1_0429_a1ec, // bind orch entry
        0x1_021e_8c84 => 0x1_0429_a4f4, // bind resolve-loop
        0x1_021f_c61c => 0x1_042a_de84, // class-validate (mesma âncora BINDSIG; a distância
        // `BIND_ORCH − CLASS_VALIDATE` bate EXATA entre Steam e Epic: 0x8C4)
        0x1_0002_2808 => 0x1_0002_21e8, // PoolDefault::AllocateAligned (símbolo)
        0x1_0002_2cb0 => 0x1_0002_2690, // PoolDefault::Free (símbolo)
        0x1_0001_d9a0 => 0x1_0001_d380, // PoolRoot::GetHandle (símbolo)
        0x1_03e2_f17c => 0x1_0360_2724, // PoolArchive::Allocate (símbolo)
        0x1_03e2_ebd4 => 0x1_0360_217c, // open-archive
        0x1_0908_b798 => 0x1_090c_a9c8, // OPCODE_TABLE (âncora funcOperatorAdd<int>)
        // CNamePool::Get — de longe o mais pedido em runtime (10371x num boot de 75s; o
        // segundo colocado pede 2x). Achado pelo DADO, não por padrão de código: a string
        // "gameStatsSystem" existe UMA vez no __cstring (0x106c6dca2); quem a referencia por
        // adrp+add chama `CNamePool::Add(hash, name)` @ 0x100a5ccd4 com o hash literal em x0
        // (0x761774a571cc8913 = cname("gameStatsSystem"), confere com o teste em cname.rs).
        // Dentro do Add: `and x21,x19,#0x7ffff` + `adrp 0x107520000 + #0xac0` → a BASE do pool
        // é 0x107520ac0. Só 7 funções referenciam essa base; 3 recebem hash em x0, e das 3 uma
        // devolve bool (`cset w0,lo` = Exists) e duas devolvem `add x0,x8,#0x14` (ponteiro pra
        // string DENTRO do nó — não pro __cstring, porque o Add copia). CONFIRMADO chamando a
        // função no jogo vivo via lldb: os 3 vetores de teste de `cname.rs` devolvem a string
        // certa. A outra sobrevivente (0x100a5cb10) devolve ponteiros IDÊNTICOS — são dois
        // pontos de entrada da mesma busca; qualquer uma serve pro contrato `fn(u64)->*const i8`.
        0x1_0345_28e8 => 0x1_00a5_c820, // CNamePool::Get
        // NÃO mapeados ainda (ficam inertes, ver doc acima): CNamePool::Get, TweakDB::Get,
        // TweakDB singleton, CreateRecord, RecordExists, InitializeArchives, RequestResource,
        // depot vtable, DEPOT_SINGLETON, boot phase dispatcher, os 2 phase getters e RESLINK
        // (candidato `0x1_0427_7c48` PROVÁVEL mas não confirmado — fica de fora de propósito).
        _ => return None,
    })
}

/// `~/.bwms-log-full` presente = AUMENTA o orçamento verbatim do `log()` de 800 pra 20000 linhas
/// (não desliga o throttle — ver a nota dentro de `log()`).
///
/// **Por que existe (achado 2026-08-21):** o rate-limit global (1 em 8 depois das ~800 primeiras
/// chamadas) foi criado em 2026-08-20 pra reduzir I/O na janela de boot, e resolve isso — mas
/// torna TODA leitura de veredito de smoke test não-confiável: num boot com 1375 linhas, ~87% das
/// linhas depois das 800 primeiras são descartadas em SILÊNCIO, então "0 linhas" pra um smoke não
/// distingue "não rodou" de "a linha foi jogada fora". Isso levou a conclusões erradas — cheguei a
/// registrar que 3 itens "precisavam de estado de jogo" quando o que faltava era a linha no log.
///
/// Gate próprio (não `dev_mode()`): boots de DIAGNÓSTICO ligam o marcador e recebem log COMPLETO;
/// boots normais de dev seguem com o throttle e sua proteção. Resolvido 1x (`OnceLock`) — o
/// caminho quente não paga syscall por chamada.
fn log_full_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("HOME")
            .map(|h| std::path::Path::new(&h).join(".bwms-log-full").exists())
            .unwrap_or(false)
    })
}

/// Uma anomalia é DIGNA DE LOG quando o objeto é POLIMÓRFICO e mesmo assim saiu do `Construct`
/// com ponteiro de vtable NULO.
///
/// A distinção importa e é a razão de isto ser função pura testável em vez de um `if` solto:
/// para quem deriva de `IScriptable` o offset 0 é o ponteiro de vtable (obrigatoriamente
/// não-nulo — é objeto C++ com métodos virtuais), mas para um struct PURO (`Vector4` etc.) o
/// offset 0 é DADO comum, e `x = 0.0` é um valor perfeitamente legítimo. Uma checagem cega em
/// `*obj == 0` acusaria todo `Vector4` zerado como defeito.
pub(crate) fn null_vtable_is_anomalous(derives_iscriptable: bool, vtable_word: usize) -> bool {
    derives_iscriptable && vtable_word == 0
}

/// Log de ANOMALIA — não passa pelo rate-limit de [`log`].
///
/// [`log`] mantém as primeiras ~800 chamadas e depois grava só 1 em cada 8 (corte deliberado de
/// I/O, 2026-08-20). Isso é correto para diagnóstico de sequência, e errado para um evento raro:
/// a linha que decide uma hipótese teria 1/8 de chance de sobreviver. Como anomalia é rara por
/// definição, gravar todas custa I/O desprezível — nada a ver com o firehose que o rate-limit
/// existe para conter. Arquivo próprio para não se perder no meio das milhares de linhas normais.
pub(crate) fn log_anomaly(msg: &str) {
    #[cfg(not(feature = "devlog"))]
    {
        let _ = msg;
    }
    #[cfg(feature = "devlog")]
    {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/bwms-anomalias.log")
        {
            let _ = writeln!(f, "{msg}");
        }
        log(msg); // também na trilha normal, para aparecer na sequência do boot
    }
}

/// Carimbo do build (epoch em segundos, injetado pelo build.rs). Muda a cada compilação, então
/// serve pra confirmar QUAL dylib o jogo carregou — o arquivo em disco pode já ser o novo.
pub(crate) fn build_stamp() -> &'static str {
    env!("BWMS_BUILD_STAMP")
}

pub(crate) fn log(msg: &str) {
    // A saída de COMANDO precisa aparecer na aba Console mesmo no build público, onde o resto
    // desta função é no-op. Sem isto, digitar `help`/`give`/`money` não devolve NADA na tela e a
    // ferramenta parece travada — o resultado só existia pra quem rodasse um build `devlog`.
    // O log em si (arquivo/K-LOG) continua gateado por `devlog`, como era.
    // Prefixos que o USUÁRIO age em cima (resposta de comando + diagnóstico dos cheats). Vão pra
    // aba Console mesmo no build público, onde o resto desta função é no-op. Sem isto, um cheat
    // que não funciona não tem NENHUM sinal na tela — foi exatamente o que aconteceu com o
    // `cloak`: o comando dizia "ON" e o motivo real ficava num log que só existe em build dev.
    const USER_FACING: [&str; 13] = [
        "[console]", "[cloak]", "[ammo]", "[ram]", "[god]", "[heal]", "[give]", "[sig]",
        "[level]", "[build]", "[se]", "[stat]", "[funcs]",
    ];
    if USER_FACING.iter().any(|p| msg.starts_with(p)) {
        crate::overlay::console_out(msg);
    }
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
    // PERF (2026-08-17, achado do dia — mesma classe de bug já corrigida em `route_native`/
    // `capture()`): `log()` é chamado de ~324 call-sites, incl. o HOT-PATH de `exec_replacement`
    // e seus vizinhos (`AnimationSystem_FrameBeginReset`, ticks de GameState de plugin) — sob
    // carga pesada (streaming de boot, rajada de eventos) isso dispara MILHARES de vezes/segundo.
    // A versão antiga reabria o arquivo (`open+append+close`, syscalls de verdade) TODA CHAMADA —
    // custo que composto nessa escala pode facilmente virar minutos de I/O bloqueante, mesmo
    // padrão que já explicou o travamento do `route_native` O(n) e o `capture()` sem rate-limit.
    // Fix: handle cacheado (aberto 1x, `O_APPEND` garante posicionamento correto mesmo se algo
    // externo truncar o arquivo entre boots — cada processo novo abre um handle novo de qualquer
    // forma). `try_lock`: se contendido (log reentrante/concorrente), pula silenciosamente em vez
    // de bloquear — perder uma linha de log é aceitável, travar o hot-path não é.
    //
    // RATE-LIMIT GLOBAL (2026-08-20, madrugada, achado desta sessão): build de teste inteira desta
    // madrugada rodou com `devtools`(=`devlog` ligado) e reproduziu SIGSEGV+travamento de memória
    // consistentemente (≥8 tentativas); um boot idêntico com `devlog` OFF sobreviveu 570s+ sem
    // crash nem travamento severo, mesma memória crítica do sistema — forte indício de que o
    // VOLUME de I/O deste log (2 escritas por chamada em dev_mode: console.log+trace.log, cada
    // `writeln!` é 1 syscall real mesmo com handle cacheado) contribui pra fragilidade sob pressão,
    // mesmo sem ser a causa raiz da pressão em si. Fix cirúrgico: mantém verbatim as primeiras
    // ~800 chamadas do processo (cobre o boot inicial, onde o diagnóstico de sequência importa
    // mais), depois passa a escrever só 1 em cada 8 — corta ~87% do volume em regime de alta
    // atividade (exatamente a janela onde o crash/travamento historicamente acontece) sem perder
    // a capacidade de diagnóstico (amostragem ainda mostra a sequência, só mais esparsa).
    static LOG_CALL_COUNT: AtomicU64 = AtomicU64::new(0);
    let n = LOG_CALL_COUNT.fetch_add(1, Ordering::Relaxed);
    // Orçamento de linhas VERBATIM antes do 1-em-8 começar: 800 no boot normal, 20000 quando o
    // marcador de diagnóstico está armado. **É um TETO, não "sem limite"** — desligar o throttle
    // por completo devolve o firehose de I/O que correlacionou com fragilidade de boot em
    // 2026-08-20 (e um boot de 2026-08-21 com log irrestrito ficou visivelmente mais lento pra
    // chegar na engagement). 20000 cobre com folga a janela de um veredito de smoke (um boot
    // inteiro com throttle deu ~1375 linhas) sem virar firehose.
    let budget: u64 = if log_full_enabled() { 20_000 } else { 800 };
    if n >= budget && n % 8 != 0 {
        return;
    }
    static LOG_FILE: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);
    if let Ok(mut guard) = LOG_FILE.try_lock() {
        if guard.is_none() {
            *guard = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/cp77-console.log")
                .ok();
        }
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{msg}");
        }
    }
    // Em dev, ESPELHA no trace.log — sink confiável p/ diagnóstico de 1 ciclo: o console.log é
    // zerado ao abrir o console in-game (perde prints), o trace.log só zera no boot. Junta num
    // lugar só: loads de mod, prints de Lua (cheats/Cron), erros — pra ler tudo de uma vez.
    // Mesmo fix de handle cacheado (era `open+append+close` por chamada).
    if dev_mode() {
        static TRACE_FILE: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);
        if let Ok(mut guard) = TRACE_FILE.try_lock() {
            if guard.is_none() {
                *guard = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("/tmp/cp77-trace.log")
                    .ok();
            }
            if let Some(f) = guard.as_mut() {
                let _ = writeln!(f, "{msg}");
            }
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

/// Atraso fixo no topo do `on_load()` — fix real (não workaround de dev) do crash determinístico
/// `brk #1`/`baseEngineInit.cpp:1094` ("Failed to initialize scripts data!") que acontecia em TODO
/// boot com o dylib presente, independente de conteúdo/registro/comportamento (isolado por
/// eliminação em 2026-08-19: mesmo crash com `BWMS_INERT=1`, com `register_all()` desligado, com
/// dylib antigo pré-sessão — é corrida de timing no boot, não bug de conteúdo). Achado: rodar sob
/// `lldb -- <exe>` (stop-at-entry) evita o crash por completo — a pausa do lldb entre dyld carregar
/// o dylib (roda este `on_load` como parte da carga) e o processo seguir pra `main()` desloca o
/// timing o suficiente pra sair da corrida. Um atraso artificial aqui reproduz o MESMO efeito sem
/// precisar de debugger: confirmado em boot normal (sem lldb), GAMEPLAY real alcançada, zero crash,
/// input funcional. `BWMS_BOOT_DELAY_MS` sobrescreve o valor (ms) pra tuning/teste.
const BOOT_RACE_DELAY_MS: u64 = 800;

extern "C" fn on_load() {
    let delay_ms = std::env::var("BWMS_BOOT_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(BOOT_RACE_DELAY_MS);
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }
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
    // Registro condicional (2026-08-07): `register_all()` lê o conteúdo dos `.reds` deployados
    // pra só registrar no RTTI o que algum mod de fato usa (reduz a injeção no motor). Achado #1
    // (corrigido): a 1ª tentativa lia da pasta ERRADA (`mods_dir()`=`red4ext/blackwall-mods/`,
    // vazia/vestigial — ver `game_scripts_dir()`), causando gate-off de natives realmente usadas
    // → crash de bind no Steam. Com o path certo (`r6/scripts`), o crash de bind AINDA
    // reapareceu 1x — hipótese em aberto: I/O síncrono na janela crítica do hook, não mais falta
    // de dado. Pré-popula aqui, em `on_load()` (FS comprovadamente confiável — crashreport/
    // check_stale_boot_attempt acima leem/escrevem com sucesso), ANTES do hook nem existir — log
    // do tamanho lido confirma se o preload aqui realmente evita I/O na hot-path mais tarde.
    let preload_bytes = unsafe { crate::register::preload_reds_scan() };
    log(&format!("[cp77-console] preload_reds_scan (on_load, pré-hook): {preload_bytes} bytes"));
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
    // `axl-pathb-injection-arbitrary` + `ArchiveXL.RegisterArchive`/`RegisterDir`: instala o hook no
    // InitializeArchives AQUI, no `on_load` SÍNCRONO (thread do jogo, mais cedo possível — ANTES do
    // InitializeArchives/LoadGlobs do boot). Instalar hook de CÓDIGO da nossa thread de heartbeat NÃO
    // efetivou a escrita (v2 não capturou nada); o copy-test que funciona instala do cp77_tick = thread
    // do jogo. **Sempre instalado** (antes só sob `~/.bwms-pathbtest`) — base da API Facade shipada,
    // não mais só probe de dev. Ver `install_pathb_capture`.
    unsafe { install_pathb_capture() };
    // RED4ext.SDK `#461` (`CallbackSystem.RegisterCallback(n"Pipeline/FrameBegin",...)`, ✅ FECHADO
    // 2026-08-11): **Sempre instalado** (antes só sob `dev_mode()`+`~/.bwms-updateregistrar-probe`
    // dentro de `install_postload_hooks()`) — mesma promoção de "probe de dev" pra "base shipada"
    // já dada ao `install_pathb_capture()` acima. Passthrough observe-only puro, zero mudança de
    // comportamento do jogo. Ver `install_pipeline_framebegin_probe` (selftest.rs).
    unsafe { crate::selftest::install_pipeline_framebegin_probe() };
    // `axl-streaming-apply` + `axl-resource-patch-apply`: instala hooks PostLoad CEDO (on_load,
    // ANTES do carregamento de recursos). worldStreamingSector::PostLoad dispara ANTES do player
    // spawnar — hook via postload-hook manual chega tarde demais. Gated por dev_mode().
    if crate::dev_mode() {
        unsafe { crate::selftest::install_postload_hooks() };
    }
    // `axl-factories-apply` E2E: instala o HookAfter em LoadFactoryAsync CEDO (antes do factory-load) +
    // enfileira o NOSSO factory CUSTOM (base\zz_bwms\bwms_factory.csv, num archive injetado via pathb).
    // Prova o e2e: archive custom injetado (pathb) → factory custom re-injetado (LoadFactoryAsync após o
    // sentinel) → carrega do archive injetado. Gated ~/.bwms-facttest (dev). Ver `factory_replacement`.
    if let Ok(hh) = std::env::var("HOME") {
        if std::path::Path::new(&hh).join(".bwms-facttest").exists() {
            unsafe { crate::selftest::install_factory_hook() };
            unsafe { crate::selftest::install_resolveresource_probe() };
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
                // Fallback: o getter viu real==5 na maioria dos boots, mas às vezes a transição
                // 1→5 não passa pelo getter hookado (multi-SM, race). O heartbeat é o gate final.
                if !selfboot::PHASE_REACHED_5.swap(true, Ordering::Relaxed) {
                    log("[hb] PHASE_REACHED_5 setado via heartbeat (getter não viu phase=5)");
                }
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
            // 2026-08-14 (ângulo novo desta rodada, `#198`/`#199`/checkres): captura BEM CEDO no
            // boot, ANTES de `seen_eng && player` (gameplay) — ideia nunca tentada antes nesta
            // investigação. Toda tentativa anterior só mandava `checkreshook` depois de confirmar
            // GAMEPLAY (o canal completo do executor/heartbeat só existe no loop PÓS-gameplay,
            // abaixo). Mas o comentário do próprio `install_checkres_probe` já dizia que
            // `CheckResource` "dispara continuamente durante loading real, mesmo antes de
            // GAMEPLAY" — e o BOOT em si (fase 1-3, streaming de textura/shader/mundo) é onde essa
            // atividade deveria ser mais densa, não depois. Whitelist mínima (só os 3 comandos
            // desta investigação + `ping` de diagnóstico) reaproveitando as MESMAS funções
            // thread-safe já usadas no loop pós-gameplay — `checkresbaseline`/`checkrespost` são
            // puro drain de ring/Mutex, zero risco novo. `checkreshook` (instala
            // `Interceptor::replace`, escreve código) AQUI roda da THREAD DO HEARTBEAT (não da
            // game thread/executor) — categoria de risco já documentada como "nunca testada" pro
            // `checkreshook` especificamente; testando agora de propósito, opt-in via gate próprio
            // do probe, prefixo `[hb-early]` pra nunca confundir com o canal pós-gameplay.
            if let Ok(cmd) = std::fs::read_to_string("/tmp/cp77-cmd.txt") {
                let cmd = cmd.trim().to_string();
                let consumed = match cmd.as_str() {
                    "ping" => { log(&format!("[hb-early] pong (t={t}s, pré-gameplay)")); true }
                    "checkreshook" => {
                        unsafe { crate::selftest::install_checkres_probe() };
                        log(&format!("[hb-early] checkreshook (t={t}s, pré-gameplay)"));
                        true
                    }
                    "checkresbaseline" => {
                        crate::selftest::checkres_baseline();
                        log(&format!("[hb-early] checkresbaseline (t={t}s, pré-gameplay)"));
                        true
                    }
                    "checkrespost" => {
                        crate::selftest::checkres_post();
                        log(&format!("[hb-early] checkrespost (t={t}s, pré-gameplay)"));
                        true
                    }
                    // `findresloader` PRÉ-GAMEPLAY (2026-08-21): metade dos boots desta sessão
                    // morreu ANTES da gameplay, e como o scan só rodava pelo canal pós-gameplay,
                    // esses boots eram DESPERDIÇADOS por completo. Aqui ele roda mesmo num boot que
                    // nunca chega ao jogo. É a adição mais SEGURA possível a esta whitelist: só
                    // `mach_vm_read_overwrite` (leitura pura, thread-safe, nunca falha em página
                    // inválida) — bem menos arriscado que o `checkreshook` que já está aqui, que
                    // ESCREVE código via `Interceptor::replace`.
                    c if c.starts_with("findresloader") => {
                        let a: Vec<&str> = c.split_whitespace().collect();
                        let start = a.get(1)
                            .and_then(|x| u64::from_str_radix(x.trim_start_matches("0x"), 16).ok())
                            .unwrap_or(0x1_06e0_0000);
                        let mb = a.get(2).and_then(|x| x.parse::<u64>().ok()).unwrap_or(16);
                        let min_size = a.get(3).and_then(|x| x.parse::<u32>().ok()).unwrap_or(64);
                        log(&format!("[hb-early] findresloader (t={t}s, pré-gameplay)"));
                        unsafe { crate::register::find_res_loader(start, mb, min_size) };
                        true
                    }
                    _ => false,
                };
                if consumed {
                    let _ = std::fs::write("/tmp/cp77-cmd.txt", "");
                }
            }
            // RED4ext.SDK `#37` (`GameStates.OnUpdate`, Initialization) — MOVIDO pra ESTE loop
            // (2026-08-11, fix desta sessão): antes vivia só no loop PÓS-gameplay (abaixo), que só
            // começa a rodar DEPOIS que `seen_eng && player` já é true — mas com o fix de `#38`
            // (Init.OnExit agora disparando de forma confiável na MESMA transição de presença que
            // termina este loop, via `GS1_EXITED` em cp77_tick), `GS1_EXITED` já está `true` no
            // instante em que o loop pós-gameplay começa — a janela de `OnUpdate(1)` tinha
            // colapsado pra ZERO disparos (regressão que o fix de `#38` teria introduzido em `#37`
            // se não corrigida junto). Este loop (ANTES de `seen_eng && player`) é a janela REAL de
            // `Initialization` — dispara aqui, a cada ~2s, enquanto ainda não transicionou.
            if !GS1_EXITED.load(Ordering::Relaxed) {
                crate::api::call_game_state_update(1);
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        // Pós-boot: game state management + canal drain, independente do executor.
        // O executor para quando o jogo fica idle após save-load; esta thread continua.
        loop {
            // Canal para comandos thread-safe (sem callf/nativo que exige a game thread).
            if let Ok(cmd) = std::fs::read_to_string("/tmp/cp77-cmd.txt") {
                let cmd = cmd.trim().to_string();
                if !cmd.is_empty() {
                    let parts: Vec<&str> = cmd.split_whitespace().collect();
                    let consumed = match parts.as_slice() {
                        ["ping"] => { log("[hb-canal] pong"); true }
                        ["gsenter", n] => {
                            if let Ok(state) = n.parse::<u32>() {
                                crate::api::call_game_state_enter(state);
                                log(&format!("[hb-canal] call_game_state_enter({state}) ok"));
                            }
                            true
                        }
                        ["gsexit", n] => {
                            if let Ok(state) = n.parse::<u32>() {
                                crate::api::call_game_state_exit(state);
                                log(&format!("[hb-canal] call_game_state_exit({state}) ok"));
                            }
                            true
                        }
                        // 2026-08-14 (achado desta rodada, `#198`/`#199`/checkres): `checkresbaseline`/
                        // `checkrespost` são 100% thread-safe (só drenam um ring de AtomicU64 + logam —
                        // ZERO chamada de VM/callf, ZERO instalação de hook) mas ficavam presos no canal
                        // gated-por-executor (`cp77_tick`, lib.rs ~1466) — que SÓ roda quando
                        // `exec_replacement` dispara. Achado ao vivo: o executor para de disparar assim
                        // que a rajada inicial de smoke-tests do `OnGameAttached` termina e o jogo fica
                        // "quieto" (nesta sessão, save preso numa cutscene de diálogo) — o canal fica
                        // MORTO pro resto do boot, mesmo com `PHASE_REACHED_5=true`. Adicionadas aqui
                        // (thread do heartbeat, sempre viva) pra nunca dependerem do executor estar
                        // ativo. `checkreshook` (instala o `Interceptor::replace`, escreve código
                        // executável) FICA DE FORA de propósito — patchear a partir de uma thread
                        // diferente da que pode estar executando o alvo concorrentemente é uma categoria
                        // de risco nova, nunca testada (e `install_pathb_capture` já documentou 1 caso
                        // real de hook-da-thread-de-heartbeat "não efetivar a escrita" pra um alvo
                        // diferente) — precisa ser instalado cedo, DURANTE a rajada (executor ainda
                        // ativo), não corrigido movendo pra esta thread.
                        ["checkresbaseline"] => {
                            crate::selftest::checkres_baseline();
                            log("[hb-canal] checkresbaseline (via heartbeat, sem depender do executor)");
                            true
                        }
                        ["checkrespost"] => {
                            crate::selftest::checkres_post();
                            log("[hb-canal] checkrespost (via heartbeat, sem depender do executor)");
                            true
                        }
                        // 2026-08-14 (mesma sessão, mesmo fix aplicado a `inkgetbaseline`/`inkgetpost`
                        // — item Codeware `#120`): MESMO mecanismo/MESMA causa do `checkresbaseline`/
                        // `checkrespost` acima — a 5ª tentativa (mesmo dia) confirmou que o canal
                        // gated-por-executor morre ~15-20s pós-GAMEPLAY (a rajada de smoke-tests do
                        // `OnGameAttached` termina e `exec_replacement` para de disparar), deixando
                        // `presskey`+`inkgetpost` presos em `/tmp/cp77-cmd.txt` intocados pelo resto do
                        // boot. `inkget_baseline`/`inkget_post` são 100% thread-safe (só drenam o ring
                        // `INKGET_RING` de `AtomicU64` + um `Mutex<BTreeSet>` de baseline + logam — ZERO
                        // chamada de VM/RTTI/callf, ZERO instalação de hook) — movidas aqui pelo mesmo
                        // motivo. `inkgethook` (instala o `Interceptor::replace_adrp_br8`, escreve
                        // código executável) fica DE FORA deste braço de propósito (não precisa: desde
                        // `cw-inkget-autoretry`, 2026-08-16, a thread dedicada spawnada em `on_load`
                        // já tenta instalar sozinha, repetidamente, independente deste canal — ver o
                        // spawn logo após a thread de heartbeat, e o braço `["inkgethook"]` manual do
                        // executor mais abaixo pra reinstalar sob demanda).
                        ["inkgetbaseline"] => {
                            crate::selftest::inkget_baseline();
                            log("[hb-canal] inkgetbaseline (via heartbeat, sem depender do executor)");
                            true
                        }
                        ["inkgetpost"] => {
                            crate::selftest::inkget_post();
                            log("[hb-canal] inkgetpost (via heartbeat, sem depender do executor)");
                            true
                        }
                        // 2026-08-16/17 (rodada 41, infra pendente desde a rodada 39/40):
                        // `archivegroupdump` é 100% read-only (zero mutação, só lê `PATHB_DEPOT`,
                        // já capturado pelo hook `InitializeArchives` bem mais cedo no boot,
                        // ~t=15s — MUITO antes de `PHASE_REACHED_5`/GAMEPLAY) — mesma categoria
                        // segura de `checkresbaseline`/`inkgetbaseline` acima. Migrado pro canal
                        // do heartbeat (sempre vivo, independente do executor) pra que diagnósticos
                        // futuros do depot não precisem competir com a janela de risco do lock
                        // nativo do motor (`SharedSpinLock::Lock()`, confirmado nas rodadas
                        // 26/32/35/36, perto/depois de gameplay real). `archivegroupcreate` (muta
                        // memória real do engine) fica DE FORA de propósito — mesma cautela que já
                        // manteve `checkreshook`/`presskey` fora desta migração (categoria "escreve
                        // em memória/código vivo" precisa do executor, nunca da thread do
                        // heartbeat). Núcleo em `run_archivegroupdump()` (lib.rs, junto de
                        // `archive_scope_name`), reusado pelo fallback do canal do executor acima.
                        ["archivegroupdump"] => {
                            unsafe { run_archivegroupdump() };
                            log("[hb-canal] archivegroupdump (via heartbeat, sem depender do executor)");
                            true
                        }
                        _ => false // callf/spawnsub/outros precisam da game thread — NÃO apaga
                    };
                    if consumed {
                        let _ = std::fs::write("/tmp/cp77-cmd.txt", "");
                    }
                }
            }
            // (nota: `OnUpdate(1)`/Initialization foi movido pro loop DE CIMA — este loop só
            // começa a rodar DEPOIS que `seen_eng && player` já é true, ponto em que `GS1_EXITED`
            // já sempre disparou via `cp77_tick`; a janela real de Initialization é ANTES disso.)
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
    // `cw-inkget-autoretry` (2026-08-16, engenharia de robustez pro item Codeware `#120`): 9
    // tentativas MANUAIS seguidas (ver HISTORICO.md) confirmaram que a janela em que o canal do
    // EXECUTOR fica vivo pós-gameplay é curta e IMPREVISÍVEL (às vezes fecha em 1-2s, às vezes
    // nem isso, dependendo de quando a 2ª rajada de `OnGameAttached` do autocontinue domina o
    // executor) — mandar `inkgethook` "na hora certa" por fora (agente/humano cronometrando)
    // nunca convergiu de forma confiável. Em vez de continuar apostando no timing manual, o
    // PRÓPRIO dylib agora tenta instalar o probe sozinho, repetidamente, numa THREAD DEDICADA
    // (sempre viva desde o `on_load`, independente do executor/foco — mesma categoria da thread
    // de heartbeat acima, só que com cadência mais apertada, 750ms, só pra esta tarefa).
    //
    // Por que é seguro chamar `install_inkget_probe()` em loop, de uma thread que não é a do
    // executor: (1) `install_inkget_probe()` JÁ é idempotente por design — o primeiro `swap`
    // em `INKGET_INSTALLED` funciona como uma trava: só 1 chamada por vez chega a de fato tentar
    // o patch; se falhar (alvo ilegível/prólogo mudou), a flag volta pra `false` e a PRÓXIMA
    // tentativa (deste mesmo loop) tenta de novo do zero — nunca instala 2x, nunca deixa o hook
    // pela metade. (2) O mecanismo de escrita (`Interceptor::replace_adrp_br8`, ver `gum.rs`) é
    // `mach_vm_protect`(COW)+`memcpy`+`sys_icache_invalidate` sobre memória do MESMO processo —
    // opera no espaço de endereço da TASK inteira, não é uma operação por-thread; e
    // `pthread_jit_write_protect_np` (usado só pro trampolim JIT, não pro patch em si) já é
    // corretamente per-thread e setado/resetado dentro da própria chamada, então funciona igual
    // não importa qual thread chama. (3) Já existe precedente direto no próprio projeto: o
    // `checkreshook` (mesmíssima categoria — instala hook de código, ver bloco `[hb-early]`
    // acima) foi testado ao vivo rodando desta MESMA thread de heartbeat e o hook instalou e
    // capturou dado real (`HISTORICO.md`, 2026-08-14). O único precedente NEGATIVO conhecido
    // (`install_pathb_capture`, comentário ~linha 393) foi pra um alvo que dispara 1 VEZ SÓ, bem
    // cedo no boot (`InitializeArchives`) — plausivelmente um problema de JANELA (o heartbeat só
    // chegou a instalar DEPOIS que a única chamada já tinha acontecido), não de escrita
    // cross-thread não-visível; `Red::InkSystem::Get()` é chamado continuamente (562+ sites, o
    // boot inteiro) — não tem essa janela de "1 chance só", então esse risco específico não se
    // aplica aqui. Risco residual, PRÉ-EXISTENTE (não introduzido por esta mudança): qualquer
    // inline hook nesta base de código pode colidir com outra thread lendo o mesmo prólogo no
    // instante exato do patch (nenhum stop-the-world) — já era verdade quando `inkgethook` só
    // rodava via comando manual do executor; mover PRA ONDE roda não muda ESSE risco específico.
    //
    // Gate `~/.bwms-hook-inkget-lr` (o mesmo marcador manual de sempre) checado 1x aqui, na
    // decisão de nascer a thread — ausente = zero overhead (thread nem chega a existir) pra
    // qualquer boot/usuário sem o marcador. `install_inkget_probe()` internamente RE-checa o
    // mesmo gate a cada chamada (barato, mesmo padrão de outros probes do projeto) — redundante
    // de propósito, não uma otimização perdida.
    if let Ok(hh) = std::env::var("HOME") {
        if std::path::Path::new(&hh).join(".bwms-hook-inkget-lr").exists() {
            std::thread::spawn(|| {
                // ~7,5min a 750ms — generoso (feature dev-only, gate fechado por padrão pra
                // qualquer usuário final; não precisa ser econômico), mas não infinito: se o
                // alvo nunca ficar legível (módulo nunca mapeado, boot travado antes disso),
                // a thread desiste e para de gastar ciclos sozinha.
                const MAX_ATTEMPTS: u32 = 600;
                let mut n = 0u32;
                loop {
                    n += 1;
                    log(&format!("[inkget-retry] tentativa {n}"));
                    unsafe { crate::selftest::install_inkget_probe() };
                    if crate::selftest::inkget_is_installed() {
                        log(&format!(
                            "[inkget-retry] SUCESSO na tentativa {n} — hook instalado, parando de tentar"
                        ));
                        // Captura de baseline AUTOMÁTICA (sem precisar do comando manual
                        // `inkgetbaseline`): espera a instalação assentar (~10s de idle real)
                        // e drena o ring sozinho, sempre pela mesma thread — `inkget_baseline()`
                        // é 100% thread-safe (só drena `INKGET_RING`/`AtomicU64` + grava
                        // `INKGET_BASELINE`/`Mutex<BTreeSet>`, zero VM/RTTI/callf).
                        std::thread::sleep(std::time::Duration::from_secs(10));
                        crate::selftest::inkget_baseline();
                        log("[inkget-retry] baseline automático capturado (10s pós-instalação) — presskey/inkgetpost seguem manuais via canal (precisam de ação externa)");
                        break;
                    }
                    if n >= MAX_ATTEMPTS {
                        log(&format!(
                            "[inkget-retry] desistindo após {n} tentativas (~{}min) — alvo nunca ficou instalável nesta sessão",
                            (n as u64 * 750) / 60_000
                        ));
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(750));
                }
            });
        }
    }
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

/// RAM (pool `Memory`) infinita: reaplicada no tick enquanto ligada.
///
/// O jogo não expõe "custo zero" — o que existe é o POOL. Então "ilimitado" aqui é encher o pool
/// de volta periodicamente, que é o que o jogador percebe como RAM que não acaba. Reaplicar a
/// cada tick seria uma chamada RTTI por frame sem ganho nenhum (o pool não cai tão rápido), então
/// vai a cada `RAM_EVERY_TICKS` — barato e imperceptível em jogo.
static RAM_INFINITE: AtomicBool = AtomicBool::new(false);
const RAM_EVERY_TICKS: u64 = 15;
pub(crate) fn set_ram_infinite(on: bool) {
    RAM_INFINITE.store(on, Ordering::Relaxed);
}
pub(crate) fn ram_infinite() -> bool {
    RAM_INFINITE.load(Ordering::Relaxed)
}

/// Munição infinita: mesma forma da RAM. `MagazineCapacity` na arma já mata a recarga (medido em
/// jogo), mas a RESERVA continua caindo, porque é dela que a recarga puxa. O jogo não expõe "não
/// consumir", então o que o jogador percebe como munição infinita é a reserva reposta no tick.
static AMMO_INFINITE: AtomicBool = AtomicBool::new(false);
/// A cada ~2s a 60fps. O custo real deste caminho não é a matemática, é o `GiveItem`: cada um
/// levanta os eventos de inventário do jogo. Quatro por disparo a cada meio segundo era gasto sem
/// retorno — a reserva não cai nem perto dessa velocidade.
const AMMO_EVERY_TICKS: u64 = 120;
pub(crate) fn set_ammo_infinite(on: bool) {
    AMMO_INFINITE.store(on, Ordering::Relaxed);
}
pub(crate) fn ammo_infinite() -> bool {
    AMMO_INFINITE.load(Ordering::Relaxed)
}
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
/// Quantos mods já foram carregados (loadmod) — mantido por compat interna.
static MODS_LOADED: AtomicUsize = AtomicUsize::new(0);
/// Cache do ponteiro `Red::InkSystem*` já cross-validado nesta sessão de boot (2026-08-14,
/// Codeware `#100`/`#120`) — `get_inksystem_singleton()` grava aqui na 1ª descoberta bem
/// sucedida pra nunca re-varrer a BSS a cada chamada de `GetLayers`/`GetLayer`/etc. 0 = ainda
/// não resolvido nesta sessão.
static INKSYSTEM_CACHED: AtomicU64 = AtomicU64::new(0);

// ---- registry de status por mod (badge colorido) --------------------------------

#[derive(Clone, Debug)]
pub(crate) enum ModStatus {
    Ok,
    Warning(String),
    Error(String),
    Inactive,
}

pub(crate) struct ModRecord {
    pub name: String,
    pub status: ModStatus,
}

use std::sync::OnceLock;
static MOD_REGISTRY: OnceLock<std::sync::Mutex<Vec<ModRecord>>> = OnceLock::new();

fn mod_registry() -> &'static std::sync::Mutex<Vec<ModRecord>> {
    MOD_REGISTRY.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

pub(crate) fn register_mod(name: String, status: ModStatus) {
    if let Ok(mut r) = mod_registry().lock() {
        r.push(ModRecord { name, status });
    }
}

/// Retorna (ok, warn, err, inactive) — lido pelo badge do overlay.
pub(crate) fn mod_counts() -> (usize, usize, usize, usize) {
    let Ok(r) = mod_registry().lock() else { return (0, 0, 0, 0); };
    r.iter().fold((0, 0, 0, 0), |acc, m| match &m.status {
        ModStatus::Ok => (acc.0 + 1, acc.1, acc.2, acc.3),
        ModStatus::Warning(_) => (acc.0, acc.1 + 1, acc.2, acc.3),
        ModStatus::Error(_) => (acc.0, acc.1, acc.2 + 1, acc.3),
        ModStatus::Inactive => (acc.0, acc.1, acc.2, acc.3 + 1),
    })
}
/// Auto-load dos mods (BWMS = Black Wall Mod System): dispara UMA vez quando o RTTI
/// fica pronto, pra a aba Mods/cheats vir ativa sem o usuário rodar `loadmods`.
static AUTO_LOADED: AtomicBool = AtomicBool::new(false);
static GS01_FIRED: AtomicBool = AtomicBool::new(false);
/// RED4ext.SDK `#32`/`#36`/`#37`/`#38` (2026-08-11, fix desta sessão — GameStates cluster):
/// latch de `Initialization.OnExit` (type=1), ESCOPO DE ARQUIVO (antes era uma `static` LOCAL
/// dentro da closure da thread de heartbeat, disparada por leitura do byte de fase — achado de
/// cont.192 (`proofs/2026-08-11-red4ext-37-gamestates-onupdate-PARCIAL.log`): essa leitura é
/// FLAKY, às vezes o byte nunca chega a `5` de forma confiável naquela thread, mesmo quando a
/// transição real pra Running já aconteceu). Movido pra disparar no MESMO ponto/mecanismo já
/// PROVADO confiável pra `Running.OnEnter`/`OnExit` — a transição de presença do player dentro
/// de `cp77_tick` (ver bloco `[cbs]` abaixo). Latch único: `Initialization` só transiciona pra
/// `Running` 1x por processo (mesmo racional já usado pra `GS01_FIRED`).
static GS1_EXITED: AtomicBool = AtomicBool::new(false);
static RELOCREAL_DONE: AtomicBool = AtomicBool::new(false);
static ATTACHDETACH_DONE: AtomicBool = AtomicBool::new(false);
static DERIVETEST_DONE: AtomicBool = AtomicBool::new(false);
static LOCTEST_DONE: AtomicBool = AtomicBool::new(false);
/// `red4ext-461` Opção A: latch local barato pra parar de rechecar o marcador/estado a cada tick
/// depois que `install_anim_framebegin_hook_if_ready()` (selftest.rs) já confirmou instalado.
static ANIM_FRAMEBEGIN_HOOK_INSTALLED_TICK: AtomicBool = AtomicBool::new(false);
// `axl-pathb-injection-arbitrary`: injeta um .archive fora do glob no content-group via um `replace`
// no InitializeArchives (0x103ed96b0). A replacement chama a original (constrói TODOS os archives do
// boot) e DEPOIS injeta — na THREAD DO JOGO, com o depot REAL (x0 do InitializeArchives, ≠ o singleton
// [0x109003000+0x1f8] que tem +0x10=NULL vivo — mesmo erro do cont.78), segurando o lock@depot+0x78,
// pré-streaming. Os offsets da RE (grupos@depot+0x10 stride 0x38, key@g+0x30, count@g+0xc, lock@+0x78)
// são CORRETOS pro `this` real. Ver HISTORICO cont.80/81, notes/RE-archiveinfo-inject-2026-07-17.md.
static INIT_ARCH_ORIG: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static PATHB_HOOK_ON: AtomicBool = AtomicBool::new(false);
static PATHB_INJECTED: AtomicBool = AtomicBool::new(false);
/// `ArchiveXL.RegisterArchive`/`RegisterDir` (`PENDENCIAS-UNIFICADAS.md`, Facade.hpp — achado já
/// desde 2026-07-13 nunca implementado): o `depot` REAL capturado 1x por `init_archives_replacement`
/// (sempre instalado agora, não mais só sob `~/.bwms-pathbtest`) fica cacheado aqui pra qualquer
/// chamada FUTURA de `RegisterArchive` (redscript, tempo de execução do mod) reusar sem precisar
/// re-hookar nada — `InitializeArchives` só roda 1x no boot inteiro.
static PATHB_DEPOT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static COPYTEST_DONE: AtomicBool = AtomicBool::new(false);
static COPYTEST_ARMED: AtomicBool = AtomicBool::new(false);
static UPDATEREC_DONE: AtomicBool = AtomicBool::new(false);
static EQUIP_DEFERRED_DONE: AtomicBool = AtomicBool::new(false);
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

/// `<jogo>/r6/scripts` — onde o `scc` compila TODO `.reds` de verdade (BWMS +
/// mods de 3os), a partir de `dylib_dir()` (`<jogo>/red4ext`). Achado 2026-08-07:
/// `mods_dir()` (acima) NÃO é isso — aponta pra `red4ext/blackwall-mods/`, uma
/// pasta vestigial/vazia desde 2026-07-11, sem relação com o bundle redscript real.
pub(crate) fn game_scripts_dir() -> Option<String> {
    dylib_dir().map(|d| format!("{d}/../r6/scripts"))
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
/// CET item `VKBindings` (`PENDENCIAS-UNIFICADAS.md`, 2026-08-11) — fila IRMÃ de `RAW_KEYS`,
/// mesmo formato, mas pro evento SOLTA-tecla (KeyUp). Antes só existia captura de key-DOWN via
/// CallbackSystem — mods conseguiam detectar "tecla pressionada" mas nunca "tecla solta"/"tecla
/// mantida", bloqueando detecção de combo/input-contínuo (o núcleo prático de `VKBindings`, que
/// mods redscript podem construir em cima disto). Mesmo mecanismo já provado (`cw-rawinput-
/// realname`), só o evento nome/timing divergem — zero RE nova.
static RAW_KEYS_UP: std::sync::Mutex<Vec<(i32, bool, bool, bool)>> = std::sync::Mutex::new(Vec::new());
pub(crate) fn push_raw_key_up(key: i32, shift: bool, control: bool, alt: bool) {
    if let Ok(mut q) = RAW_KEYS_UP.lock() {
        if q.len() < 64 {
            q.push((key, shift, control, alt));
        }
    }
}
fn drain_raw_keys_up() -> Vec<(i32, bool, bool, bool)> {
    RAW_KEYS_UP.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
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
pub const BWMS_VERSION: &str = "0.1.4";

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
    // [execpace] rodada 31 (2026-08-17): mede o ritmo de exec_replacement, throttled a 1
    // log/segundo, investigando o achado da rodada 30 (cadeia de ~108 `@wrapMethod(PlayerPuppet)
    // OnGameAttached` empilhados pode nunca devolver controle dentro da janela testada). Roda
    // ANTES de qualquer gate (registry/RTTI) de propósito — queremos ver o ritmo mesmo se o
    // resto do tick ainda não puder agir. Puramente observacional. Ver selfboot::log_exec_pace.
    selfboot::log_exec_pace();
    // WATCHDOG anti-hang de boot (backstop): roda ANTES do gate de registry, todo tick, pra o
    // caso do getter parar de ser chamado mas o tick seguir vivo. Idempotente/barato (ver
    // selfboot::boot_hang_watchdog — só age se skip ligado + phase<=1 após 75s).
    selfboot::boot_hang_watchdog();
    // Proteção contra o watchdog PRÓPRIO do motor (CDPR) matando o processo sob macOS Low Power
    // Mode (CPU/GPU throttled -> acumulado do watchdog estoura o budget default de 120s). Roda
    // SEMPRE, todo tick, incondicional (não é diagnóstico) — ver selfboot::neutralize_engine_watchdog.
    selfboot::neutralize_engine_watchdog();
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
    // RAM infinita (comando `ram on`): reenche o pool `Memory` de tempos em tempos. Fica AQUI,
    // depois do gate de registry, porque precisa do `reg` e do player vivos.
    if ram_infinite() && ticks() % RAM_EVERY_TICKS == 0 {
        let p = current_player();
        if !p.is_null() {
            unsafe { crate::console::ram(reg, p) };
        }
    }
    // Munição infinita (comando `ammo on`): repõe a reserva. Precisa do tx (transaction system)
    // além do player, então só roda quando os dois estão publicados.
    if ammo_infinite() && ticks() % AMMO_EVERY_TICKS == 0 {
        let p = current_player();
        let tx = current_tx();
        if !p.is_null() && !tx.is_null() {
            unsafe { crate::console::refill_ammo(reg, p, tx) };
        }
    }
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
    // Pipeline unificado de carga de mods (6 serem 1): ArchiveXL resource.link + factory,
    // RED4ext plugins e scan inicial da aba Mods. Idempotente — roda só na 1ª iteração.
    mod_pipeline::boot_phase();
    // RED4ext.SDK #32/#36/#37/#38 (2026-08-11): cobertura completa de `EGameStateType` —
    // `BaseInitialization`(0)/`Initialization`(1) NUNCA disparavam antes (só `Running`(2, via
    // heartbeat phase-tracking) e `Shutdown`(3, via exit hook)). `boot_phase()` (acima) é o
    // 1º ponto idempotente ONDE plugins já tiveram chance de `add_game_state` (carregados
    // dentro dele, via `mod_pipeline::load_plugins`) — dispara enter(0)→exit(0)→enter(1) aqui,
    // 1x. `exit(1)` dispara mais abaixo, na thread de heartbeat, na 1ª transição real pra
    // Running (mesmo sinal já usado pro `enter(2)`). Mapeamento é uma decisão pragmática (não
    // verificada contra o binário real) — residual: callback pode disparar num momento
    // ligeiramente diferente do original Windows, não um risco de crash.
    if !GS01_FIRED.swap(true, Ordering::Relaxed) {
        crate::api::call_game_state_enter(0);
        crate::api::call_game_state_exit(0);
        crate::api::call_game_state_enter(1);
        log("[gs] call_game_state_enter(0)+exit(0)+enter(1) — cobertura BaseInitialization/Initialization");
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
    // `red4ext-461` (✅ FECHADO 2026-08-11, promovido pra sempre-ativo 2026-08-17): hookar o
    // `invoke` REAL do callback já registrado pelo `AnimationSystem_FrameBeginReset` vanilla,
    // capturado pelo probe observe-only de `UpdateRegistrar::RegisterUpdate`
    // (`install_pipeline_framebegin_probe`, sempre instalado no `on_load` agora — nenhum marcador
    // necessário). Rechecado todo tick até o ponteiro ter sido capturado — idempotente, barato.
    if !ANIM_FRAMEBEGIN_HOOK_INSTALLED_TICK.load(Ordering::Relaxed)
        && unsafe { crate::selftest::install_anim_framebegin_hook_if_ready() }
    {
        ANIM_FRAMEBEGIN_HOOK_INSTALLED_TICK.store(true, Ordering::Relaxed);
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
        // ptr do player salvo antes da transição null — Entity/Detach precisa dele quando player já é null
        static LAST_ENTITY_PTR: AtomicUsize = AtomicUsize::new(0);
        const HANDLE_EVENTS_DELAY_TICKS: u64 = 180; // ~3-6s no ritmo já usado por TICKS neste arquivo
        // 2026-08-20 (madrugada, sessão 2, retomada pós-compactação): 4/4 crashes SIGSEGV reproduzidos
        // nesta sessão (mesma assinatura de sempre, `EXC_BAD_ACCESS`/zero frame nosso na pilha) sempre
        // IMEDIATAMENTE APÓS `mod_pipeline::session_phase` (scan RTTI de ~27800 tipos procurando
        // `ScriptableTweak`, `[tweakxl] GetAllTypes`) terminar de rodar no MESMO tick dos 4
        // `fire_event_args` acima — nunca durante os 4 eventos em si, sempre logo depois do scan+
        // dispatch do TweakXL completar. Candidato de fix já listado em sessões anteriores, testado
        // agora pela 1ª vez: dar ao scan TweakXL um tick-alvo PRÓPRIO, mais tarde que os 4 eventos
        // (não junto no mesmo tick) — reduz a chance de o scan pesado colidir com a mesma janela de
        // fila-de-eventos do motor que os 4 `fire_event_args` já ocupam. Ver memória
        // `bwms-t82s-crash-vs-887pct-hang` (Atualização 10).
        static PENDING_SESSION_PHASE_AT: AtomicU64 = AtomicU64::new(0); // tick-alvo p/ o scan TweakXL; 0 = nada pendente
        const SESSION_PHASE_EXTRA_DELAY_TICKS: u64 = 120; // ~2-4s DEPOIS dos 4 eventos (soma a HANDLE_EVENTS_DELAY_TICKS)
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
                LAST_ENTITY_PTR.store(player as usize, Ordering::Relaxed);
                // RED4ext.SDK `#32`/`#36`/`#37`/`#38` (2026-08-11, fix desta sessão): `Initialization.
                // OnExit` (type=1) TEM que disparar ANTES de `Running.OnEnter` — mesma transição, mesmo
                // instante, mesmo mecanismo (presença do player, já provado confiável pro Running) em
                // vez do byte de fase lido pela thread de heartbeat (flaky, ver `GS1_EXITED` acima).
                if !GS1_EXITED.swap(true, Ordering::Relaxed) {
                    crate::api::call_game_state_exit(1);
                    crate::log("[gs] call_game_state_exit(1) — saiu de Initialization, entrando em Running (via presença do player)");
                }
                // red4ext-gamestates-add: Running.OnEnter (type=2) — player spawnando = estado Running
                crate::api::call_game_state_enter(2);
                // NÃO dispara Session/Start/Entity/Attach aqui — agenda pra N ticks depois (bloco
                // fora do `if`, abaixo), fora da janela de flood do save-load real.
                let target = TICKS.load(Ordering::Relaxed) + HANDLE_EVENTS_DELAY_TICKS;
                PENDING_HANDLE_EVENTS_AT.store(target, Ordering::Relaxed);
                crate::log(&format!(
                    "[cbs] Session/Start+Entity/Attach AGENDADOS pra tick>={target} (delay anti-crash, atual={})",
                    TICKS.load(Ordering::Relaxed)
                ));
                // session_phase (scan TweakXL) agendado num tick DEPOIS dos 4 eventos acima —
                // ver comentário de `SESSION_PHASE_EXTRA_DELAY_TICKS` acima.
                let sp_target = target + SESSION_PHASE_EXTRA_DELAY_TICKS;
                PENDING_SESSION_PHASE_AT.store(sp_target, Ordering::Relaxed);
                crate::log(&format!(
                    "[cbs] session_phase (scan TweakXL) AGENDADO pra tick>={sp_target} (separado dos 4 eventos, +{SESSION_PHASE_EXTRA_DELAY_TICKS} ticks)"
                ));
            } else {
                // Despawn/End: sem o padrão de flood conhecido (mundo saindo, não entrando) — mantém
                // imediato, e também cancela qualquer disparo pendente de uma sessão anterior.
                PENDING_HANDLE_EVENTS_AT.store(0, Ordering::Relaxed);
                PENDING_SESSION_PHASE_AT.store(0, Ordering::Relaxed);
                // red4ext-gamestates-add: Running.OnExit (type=2) — player despawnando = saindo do Running
                crate::api::call_game_state_exit(2);
                if let Some(arg) = unsafe { register::make_gamesessionevent_arg(false, true) } {
                    let n2 = unsafe { register::fire_event_args("Session/End", &[arg]) };
                    crate::log(&format!("[cbs] Session/End (GameSessionEvent real) → {n2} callback(s)"));
                }
                // `cw-controller-entity` sub-evento: "Entity/Detach" (EntityDetachHook.hpp) —
                // usa LAST_ENTITY_PTR pq player já é null aqui na transição de despawn.
                let last = LAST_ENTITY_PTR.load(Ordering::Relaxed) as *mut c_void;
                if !last.is_null() {
                    if let Some(arg) = unsafe { register::make_entitylifecycleevent_arg(last) } {
                        let nd = unsafe { register::fire_event_args("Entity/Detach", &[arg]) };
                        crate::log(&format!("[cbs] Entity/Detach (EntityLifecycleEvent real) → {nd} callback(s)"));
                    }
                }
            }
        }
        // Dispara os 2 eventos com `make_handle` (Session/Start + Entity/Attach) quando o tick-alvo
        // agendado acima é atingido — fora do `if` de transição (roda em ticks POSTERIORES ao evento
        // que armou o alvo). Guarda `PLAYER_PRESENT` pra não disparar se o player já saiu de novo.
        let pending = PENDING_HANDLE_EVENTS_AT.load(Ordering::Relaxed);
        if pending != 0 && TICKS.load(Ordering::Relaxed) >= pending {
            PENDING_HANDLE_EVENTS_AT.store(0, Ordering::Relaxed);
            // DIAGNÓSTICO TEMPORÁRIO (2026-08-20, investigação do crash SIGSEGV redDispatcher da
            // madrugada, ver memória `bwms-t82s-crash-vs-887pct-hang` Atualização 3b/4): teste de
            // eliminação — se `~/.bwms-diag-skip-delayed-dispatch` existir, pula TODO este bloco
            // (Session/Start+Entity/Assemble+Entity/Attach+Component/Toggle; `session_phase`/scan
            // TweakXL agora tem tick-alvo PRÓPRIO, separado, ver bloco abaixo)
            // pra ver se o crash intermitente na fila de eventos do motor (`redDispatcher*`) some.
            // Remover este gate depois que a causa for isolada (não é fix permanente).
            let skip_diag = std::env::var("HOME")
                .ok()
                .map(|h| std::path::Path::new(&h).join(".bwms-diag-skip-delayed-dispatch").exists())
                .unwrap_or(false);
            if skip_diag {
                crate::log("[diag] ~/.bwms-diag-skip-delayed-dispatch presente — PULANDO Session/Start+Entity/Assemble+Entity/Attach+Component/Toggle (teste de eliminação)");
            } else if PLAYER_PRESENT.load(Ordering::Relaxed) && present {
                // `cw-controller-session`: "Session/Start" (nome REAL do Codeware,
                // `CallbackSystem::SessionStartEventName`). `restored=true` (auto-continue SEMPRE
                // carrega save) / `pregame=false` (player só presente pós-char-creation).
                if let Some(arg) = unsafe { register::make_gamesessionevent_arg(false, true) } {
                    let n2 = unsafe { register::fire_event_args("Session/Start", &[arg]) };
                    crate::log(&format!("[cbs] Session/Start (GameSessionEvent real, atrasado {HANDLE_EVENTS_DELAY_TICKS} ticks) → {n2} callback(s)"));
                }
                // `cw-entity-builder` (2026-07-24): "Entity/Assemble" (nome REAL,
                // `EntityAssembleHook.hpp`) — achado da investigação paralela: carrega o MESMO
                // `EntityLifecycleEvent` que "Entity/Attach" (não `EntityBuilderEvent`, esse é
                // específico de "Entity/Extract"), então dispara pelo MESMO mecanismo já provado,
                // zero RE nova. Ordem real do motor: Assemble roda ANTES de Attach — disparado
                // primeiro aqui pelo mesmo motivo (um `Entity.AddComponent` feito no handler de
                // Assemble deveria já estar no array quando o Attach nativo roda de verdade).
                if let Some(arg) = unsafe { register::make_entitylifecycleevent_arg(player) } {
                    let n0 = unsafe { register::fire_event_args("Entity/Assemble", &[arg]) };
                    crate::log(&format!("[cbs] Entity/Assemble (EntityLifecycleEvent real, atrasado {HANDLE_EVENTS_DELAY_TICKS} ticks) → {n0} callback(s)"));
                }
                // `cw-controller-entity`: "Entity/Attach" (nome REAL, `EntityAttachHook.hpp`) com
                // `EntityLifecycleEvent` real (`GetEntity()->ref<Entity>`, "raw"/sem dono).
                if let Some(arg) = unsafe { register::make_entitylifecycleevent_arg(player) } {
                    let n3 = unsafe { register::fire_event_args("Entity/Attach", &[arg]) };
                    crate::log(&format!("[cbs] Entity/Attach (EntityLifecycleEvent real, atrasado {HANDLE_EVENTS_DELAY_TICKS} ticks) → {n3} callback(s)"));
                }
                // Codeware `#9` (2026-08-11): "Component/Toggle" (nome REAL, `ComponentTarget.hpp`
                // wiki) com `EntityComponentEvent` real e um COMPONENTE GENUÍNO do player (1º slot
                // vivo de `entity+0xA0`, não fixture) — testa `GetComponent()->wref<IComponent>`
                // com dado real, mesmo padrão de disparo diagnóstico atrasado já usado acima.
                let comp = unsafe { register::first_component(player) };
                if !comp.is_null() {
                    if let Some(arg) = unsafe { register::make_entitycomponentevent_arg(comp) } {
                        let n4 = unsafe { register::fire_event_args("Component/Toggle", &[arg]) };
                        crate::log(&format!("[cbs] Component/Toggle (EntityComponentEvent real, componente={comp:p}, atrasado {HANDLE_EVENTS_DELAY_TICKS} ticks) → {n4} callback(s)"));
                    }
                }
            }
        }
        // session_phase (scan TweakXL/GetAllTypes) — tick-alvo PRÓPRIO, separado dos 4 eventos
        // acima (ver `SESSION_PHASE_EXTRA_DELAY_TICKS`). Mesmo gate de diagnóstico (consistência:
        // se o usuário pediu pra pular tudo, pula os dois blocos).
        let sp_pending = PENDING_SESSION_PHASE_AT.load(Ordering::Relaxed);
        if sp_pending != 0 && TICKS.load(Ordering::Relaxed) >= sp_pending {
            PENDING_SESSION_PHASE_AT.store(0, Ordering::Relaxed);
            let skip_diag = std::env::var("HOME")
                .ok()
                .map(|h| std::path::Path::new(&h).join(".bwms-diag-skip-delayed-dispatch").exists())
                .unwrap_or(false);
            if skip_diag {
                crate::log("[diag] ~/.bwms-diag-skip-delayed-dispatch presente — PULANDO session_phase (teste de eliminação)");
            } else if PLAYER_PRESENT.load(Ordering::Relaxed) && present {
                // Pipeline TweakXL Session/Start: ScriptableTweak dispatch + YAML auto-apply dos mods.
                if let Some(reg) = unsafe { rtti::Registry::obtain() } {
                    unsafe { mod_pipeline::session_phase(&reg) };
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
    // ArchiveXL `#47` (`CustomizationExtension`, 2026-08-14): drena os 3 edge flags marcados por
    // `BwmsCustomizationEdge` (chamado sincronamente pelos wraps em `characterCreationBodyMorphMenu`)
    // — MESMO padrão seguro de "Resource/Load" acima (edge setado por native chamado por bytecode
    // → dispatch de verdade só aqui, fora de qualquer nesting de call_func).
    unsafe { register::drain_customization_edges() };
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
    // `axl-puppet-state-apply` (2026-08-02, iteração 2): dispara a recuperação ATRASADA do
    // soft-lock do espelho, se `BwmsPuppetStateArmRecovery` armou uma (ver register.rs). Roda
    // todo tick, é um no-op de load quando nada está pendente (checa 1 AtomicU64).
    unsafe { register::puppetstate_recovery_tick(reg) };
    // `axl-garment-apply` (2026-08-02): teste da hipótese do agente de RE do crash de `equiponce`
    // (`EquipmentSystem::QueueRequest` crasha porque `CClassFunction::GetInvokable()` retorna null —
    // possível dependência de contexto/thread, não de cache-warmup). Em vez de disparar na hora que o
    // comando chega (thread arbitrária, pode ser reentrante numa chamada nativa qualquer do motor),
    // dispara UMA VEZ desta MESMA posição periódica onde `BwmsPluginOnUpdate` já fez centenas de
    // chamadas sem incidente — mesmo TICKS>N de "deixar a sessão assentar", SEM o gate de
    // `in_crash_window` (esse gate é sobre proximidade de save-load, não é a preocupação aqui).
    // Gated marcador ~/.bwms-equipdeferred + ~/.bwms-equip=1/2/3 (mesmo seletor de item de sempre).
    if !EQUIP_DEFERRED_DONE.load(Ordering::Relaxed) && TICKS.load(Ordering::Relaxed) > 120 {
        if let Ok(h) = std::env::var("HOME") {
            let m = std::path::Path::new(&h).join(".bwms-equipdeferred");
            if m.exists() && !EQUIP_DEFERRED_DONE.swap(true, Ordering::Relaxed) {
                let _ = std::fs::remove_file(&m);
                unsafe {
                    if !player.is_null() {
                        let gi: Option<[u8; 16]> = rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                            rtti::call_func(&gg, player, &[]).map(|b| {
                                let mut o = [0u8; 16];
                                o.copy_from_slice(&b[..16]);
                                o
                            })
                        });
                        match gi {
                            Some(gi) => {
                                let f = register::get_function(reg, "BwmsForceEquipOnce");
                                if rtti::sane(f) {
                                    let rf = rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                                    if rtti::call_func(&rf, std::ptr::null_mut(), &[rtti::Arg::Raw(gi)]).is_some() {
                                        log("[equipdeferred] BwmsForceEquipOnce(game) chamado do tick periódico (rota alternativa, teste de contexto/thread)");
                                    } else {
                                        log("[equipdeferred] call_func não completou");
                                    }
                                } else {
                                    log("[equipdeferred] BwmsForceEquipOnce não resolveu");
                                }
                            }
                            None => log("[equipdeferred] PlayerPuppet.GetGame falhou"),
                        }
                    } else {
                        log("[equipdeferred] player null");
                    }
                }
            }
        }
    }
    // `tweakxl-hot-reload`: poll de mtime a cada ~10s; re-aplica yamls se algum mudou.
    if !in_crash_window {
        unsafe { mod_pipeline::hot_reload_tick(reg) };
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
            // red4ext-gamestates-add: Running.OnUpdate (type=2)
            crate::api::call_game_state_update(2);
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
    // comando(s) pendente(s) (do overlay ou de fora, via /tmp) — suporta múltiplas linhas.
    // Gate: só processa em gameplay (phase==5). Durante boot/menu, comandos como `postloadprobe`
    // constroem objetos complexos antes do engine estar pronto → crash (2026-07-27).
    // O arquivo fica intacto até phase5 chegar — sem perda de comandos.
    // CORREÇÃO 2026-07-27: gate original usava GAME_SESSION_DESC+0x84 que trava no objeto TRANSIENTE
    // da engagement e NUNCA re-captura o objeto real pós-save-load (bug documentado em selfboot.rs:1039-1048).
    // Substituído por PHASE_REACHED_5 (event-based, set na 1ª vez que QUALQUER getter vê phase=5).
    // Player é garantido não-nulo aqui (gate player/tx em cima já retornou).
    // CORREÇÃO 2026-07-31 (RE do crash do `equiponce`): + `!exec_nested()` — não roda a fila se a
    // chamada ATUAL do executor está aninhada dentro de uma chamada do motor ainda em voo nesta
    // thread (mesmo mecanismo do crash: `call_func` síncrono disparado no meio de uma execução já
    // em andamento). Arquivo fica intacto (não é consumido) — tenta de novo no próximo tick não-aninhado.
    {
        let canal_phase5 = selfboot::PHASE_REACHED_5.load(Ordering::Relaxed);
        if canal_phase5 && !selfboot::exec_nested() {
            let content = std::fs::read_to_string("/tmp/cp77-cmd.txt").unwrap_or_default();
            let cmds: Vec<&str> = content.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            if !cmds.is_empty() {
                let _ = std::fs::remove_file("/tmp/cp77-cmd.txt");
                for cmd in cmds {
                    run_cmd(reg, player, tx, cmd);
                }
            }
        }
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
        // `IACT_PRESS`: tecla DESCENDO (2026-08-21 — antes ia `2`/`IACT_Release` fixo aqui e no
        // KeyUp abaixo, ver `make_keyinputevent_arg`).
        let arg = unsafe { register::make_keyinputevent_arg(register::IACT_PRESS, key, shift, control, alt) };
        if let Some(arg) = arg {
            unsafe { register::fire_event_args("Input/Key", &[arg]) };
        }
    }
    // CET `VKBindings` (2026-08-11): metade que faltava — emite "Input/KeyUp" com o MESMO
    // `ref<KeyInputEvent>` real (reuso puro do construtor já provado), pra CADA tecla solta
    // capturada no sendEvent durante gameplay. Nome de evento distinto (não reaproveita
    // "Input/Key") segue o padrão já estabelecido no projeto de sinalizar fases por NOME
    // (`Session/Start`/`Session/End`, `Overlay/Open`/`Overlay/Close`) em vez de um campo
    // extra no evento.
    for (key, shift, control, alt) in drain_raw_keys_up() {
        // `IACT_RELEASE`: tecla SUBINDO — o único caso em que o `2` antigo estava certo por acaso.
        let arg = unsafe { register::make_keyinputevent_arg(register::IACT_RELEASE, key, shift, control, alt) };
        if let Some(arg) = arg {
            unsafe { register::fire_event_args("Input/KeyUp", &[arg]) };
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
            register_mod(name.clone(), ModStatus::Inactive);
            continue; // fora do default; carregável no `loadmods` manual
        }
        let init = path.join("init.lua");
        if init.exists() {
            match std::fs::read_to_string(&init) {
                Ok(src) => {
                    unsafe { lua::run_mod(&name, &src, path) };
                    count += 1;
                    register_mod(name.clone(), ModStatus::Ok);
                    log(&format!("[mods] carregado: {}", init.display()));
                }
                Err(er) => {
                    register_mod(name.clone(), ModStatus::Error(er.to_string()));
                    log(&format!("[mods] erro lendo {}: {er}", init.display()));
                }
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
    // guard de null: sem mapa pro build, `rebase` devolve null e chamar isso seria blr xzr.
    // `<null>` já é a resposta que o closure dá pra ponteiro inválido, então degradar é coerente.
    let cstr_p = crate::rebase(0x1_0002_93f8);
    let read_cstr = |cs: *const u8| -> String {
        if cstr_p.is_null() {
            return "<null>".to_string();
        }
        let cstr_data: unsafe extern "C" fn(*const u8) -> *const i8 = core::mem::transmute(cstr_p);
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
    if !depot.is_null() && crate::gum::is_readable(depot as *const c_void, 0x80) {
        // Cacheia SEMPRE — base de `facade_register_archive`/`facade_register_dir`, chamáveis a
        // qualquer momento depois (InitializeArchives só roda esta 1x por boot).
        PATHB_DEPOT.store(depot, Ordering::Relaxed);
        // Regressão do achado original (`axl-pathb-injection-arbitrary`), só sob marker de dev —
        // injeta um .archive de teste fixo, comportamento preservado 1:1.
        if !PATHB_INJECTED.swap(true, Ordering::Relaxed) {
            pathb_inject(depot as *mut u8);
        }
    }
    ret
}

/// Instala o `replace` no InitializeArchives (uma vez, cedo — do on_load/thread do jogo). Ver
/// `init_archives_replacement`. **Sempre instalado agora** (não mais só sob `~/.bwms-pathbtest`) —
/// vira base de uma capacidade shipada (`ArchiveXL.RegisterArchive`), não mais só probe de dev.
/// Risco: baixo — a replacement SEMPRE chama a original primeiro (constrói os archives do boot
/// normalmente); o único efeito extra incondicional é guardar 1 ponteiro num `AtomicPtr`.
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

/// Acha o grupo REAL de content do depot: o de MAIOR count, HEAP (não sentinela estático de
/// imagem). Extraído de `axl-pathb-injection-arbitrary` (achado 2026-07-17: key==3 fica vazio,
/// arr sentinela; os archives do boot vão pra key=1/2, arr HEAP — injetar no de maior count).
/// `groups@depot+0x10` inline {ptr@+0x10,cap@+0x18,count@+0x1c}, stride 0x38; count@g+0x0c.
unsafe fn find_pathb_content_group(depot: *mut u8) -> Option<*mut u8> {
    let groups = (depot.add(0x10) as *const *mut u8).read();
    let gcount = (depot.add(0x1c) as *const u32).read();
    if groups.is_null() || gcount == 0 || gcount > 64 {
        log(&format!("[pathb] grupos inválidos (ptr={groups:p} n={gcount}) — abortado"));
        return None;
    }
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
    if g.is_null() || best == 0 {
        log(&format!("[pathb] nenhum grupo heap com archives entre {gcount} — abortado"));
        return None;
    }
    Some(g)
}

/// ArchiveXL `#37`/`ResolveArchiveGroup(depot, basePath) -> ArchiveGroup&` (achado 2026-08-17,
/// auditoria de consistência de documentação — item catalogado como "ACHADO ACIONÁVEL"). Função
/// 100% PRÓPRIA do ArchiveXL (`App/Archives/ArchiveService.cpp`) — `std::find_if` puro sobre
/// `aDepot->groups`, ZERO `RawFunc`/`AddressLib`, ZERO endereço nativo a resolver. Layout do
/// grupo confirmado pelo header OFICIAL vendorizado (`RED4ext.SDK/include/RED4ext/ResourceDepot.hpp`,
/// `RED4EXT_ASSERT_SIZE`/`RED4EXT_ASSERT_OFFSET`, não é palpite):
/// `ArchiveGroup{archives:DynArray<Archive>@0x00, basePath:CString@0x10, scope:ArchiveScope(u32)@0x30}`
/// (stride 0x38). `ArchiveGroup`/`Archive` NÃO são polimórficos (zero vtable) — os offsets batem
/// Windows=Mac SEM o shift Itanium de +0x08 que outras structs deste projeto precisam; confirmado
/// pela porção `archives` (offsets 0x00/0x0c) já EMPIRICAMENTE validada ao vivo há semanas por
/// `find_pathb_content_group`/`load_archive_into_group` (contagem sobe corretamente ao injetar).
///
/// Implementa só a METADE de LEITURA do algoritmo real (passo 1 de `ResolveArchiveGroup`: acha
/// grupo JÁ EXISTENTE cujo `basePath` bate exato) — substituiria o heurístico frágil de
/// `find_pathb_content_group` ("grupo de maior contagem heap", documentado no próprio código como
/// não sendo a resolução real) por resolução por BASE PATH de verdade, quando o grupo já existe.
///
/// A METADE de ESCRITA do algoritmo oficial (criar grupo NOVO via `DynArray<ArchiveGroup>::Emplace`
/// quando nenhum bate, inserindo antes do 1º grupo `scope != Mod`) fica DE FORA de propósito —
/// mutar/crescer o array `depot->groups` ao vivo é categoria de risco alta (mesma classe de bug
/// que já causou heap-corruption em outros arrays deste projeto ao crescer sem o cuidado certo,
/// ex. saga do TweakXL flat-array/`CreateFlatValue`) e precisa de RE do padrão de grow (allocator-vft
/// trailer, mesma técnica já usada em `mkarr`/`inherit_flats_rt`) + teste ao vivo dedicado antes de
/// entrar em qualquer caminho de boot padrão. **Nunca chamada de nenhum hook/caminho de produção** —
/// só o comando de canal `archivegroupdump`/`archivegroupresolve` abaixo, 100% observe-only.
unsafe fn resolve_archive_group_by_path(depot: *mut u8, base_path: &str) -> Option<*mut u8> {
    let groups = (depot.add(0x10) as *const *mut u8).read();
    let gcount = (depot.add(0x1c) as *const u32).read();
    if groups.is_null() || gcount == 0 || gcount > 64 {
        return None;
    }
    for i in 0..gcount as usize {
        let gi = groups.add(i * 0x38);
        if !crate::gum::is_readable(gi as *const c_void, 0x38) {
            continue;
        }
        if crate::rtti::red_string_read(gi.add(0x10)) == base_path {
            return Some(gi);
        }
    }
    None
}

/// Decodifica `ArchiveScope` (`RED4ext.SDK/include/RED4ext/ResourceDepot.hpp`, enum u32 real):
/// `0=Invalid 1=Content(archive\pc\content) 2=DLC 3=Patch 4=Mod(archive\pc\mod + mods\*\archives)`.
fn archive_scope_name(v: u32) -> &'static str {
    match v {
        0 => "Invalid",
        1 => "Content",
        2 => "DLC",
        3 => "Patch",
        4 => "Mod",
        _ => "?",
    }
}

/// Núcleo do comando `archivegroupdump` (2026-08-17, ArchiveXL `#37`; migrado pro `[hb-canal]`
/// em 2026-08-16/17, rodada 41 — infra pendente desde a rodada 39/40) — extraído do braço de
/// `run_cmd` pra ser chamável tanto dali (canal gated-por-executor, só drena perto/depois de
/// `PHASE_REACHED_5`) quanto da thread do heartbeat (`[hb-canal]`, sempre viva, independente do
/// executor). 100% read-only, zero mutação — só depende de `PATHB_DEPOT`, capturado pelo hook
/// `InitializeArchives` bem mais cedo no boot (~t=15s) do que a janela de gameplay real, o que é
/// o ganho real da migração: diagnósticos de depot deixam de competir com a janela de risco do
/// lock nativo do motor (`SharedSpinLock::Lock()`, confirmado nas rodadas 26/32/35/36).
unsafe fn run_archivegroupdump() {
    let depot = PATHB_DEPOT.load(Ordering::Relaxed);
    if depot.is_null() {
        log("[archivegroupdump] PATHB_DEPOT ainda não capturado (boot incompleto?)");
        return;
    }
    let depot = depot as *mut u8;
    let groups = (depot.add(0x10) as *const *mut u8).read();
    let gcount = (depot.add(0x1c) as *const u32).read();
    if groups.is_null() || gcount == 0 || gcount > 64 {
        log(&format!("[archivegroupdump] grupos inválidos (ptr={groups:p} n={gcount})"));
        return;
    }
    log(&format!("[archivegroupdump] depot={depot:p} gcount={gcount}"));
    for i in 0..gcount as usize {
        let gi = groups.add(i * 0x38);
        if !crate::gum::is_readable(gi as *const c_void, 0x38) {
            log(&format!("[archivegroupdump]   [{i:02}] @{gi:p} ILEGÍVEL"));
            continue;
        }
        let cnt = (gi.add(0x0c) as *const u32).read();
        let base_path = crate::rtti::red_string_read(gi.add(0x10));
        let scope_raw = (gi.add(0x30) as *const u32).read();
        log(&format!(
            "[archivegroupdump]   [{i:02}] @{gi:p} archives={cnt} basePath='{base_path}' scope={scope_raw}({})",
            archive_scope_name(scope_raw)
        ));
    }
}

/// ArchiveXL `#37`/`ResolveArchiveGroup` — METADE DE ESCRITA (2026-08-17, rodada offline seguinte,
/// avançando o que a auditoria anterior tinha deixado de fora deliberadamente). Implementa o
/// passo 2 do algoritmo real (`ArchiveService.cpp::ResolveArchiveGroup`): quando nenhum grupo
/// existente bate o `basePath` pedido, cria um `ArchiveGroup` NOVO (zerado, `basePath` setado,
/// `scope=Mod`) inserido ANTES do 1º grupo cujo `scope != Mod` — replica
/// `aDepot->groups.Emplace(firstNonModGroup)` (`DynArray<T>::Emplace`,
/// `RED4ext.SDK/Containers/DynArray.hpp`) byte-a-byte, incluindo o fallback pra "insere no fim"
/// quando TODOS os grupos são `Mod` (ou o depot está vazio).
///
/// **Correção de uma suposição da rodada anterior desta mesma sessão** (o comentário de
/// `resolve_archive_group_by_path` acima apontava pro padrão "allocator-vft-trailer" do TweakDB
/// como pré-requisito pra crescer este array). Lendo o header REAL do `DynArray<T>` genérico
/// (não só `ArchiveGroup`/`ResourceDepot`): `GetAllocator()` tem 2 casos — com `capacity==0` o
/// allocator mora no PRÓPRIO campo `entries` (union); com `capacity>0` ele mora "no fim do
/// buffer de entries, alinhado" (1 slot extra logo após `entries[capacity-1]`). **Isso confirma
/// que o "allocator-vft-trailer" é uma propriedade do `DynArray<T>` GENÉRICO, não algo exclusivo
/// da `SortedUniqueArray` do TweakDB** — mas essa trilha só importa se o MOTOR chamar
/// `GetAllocator()` de volta (só acontece em `~DynArray()` ou em `Reserve()`→`SetCapacity()`,
/// nunca em leitura/iteração normal). A técnica de grow JÁ PROVADA AO VIVO deste projeto
/// (`dynarray_push_ptr` em `register.rs`, usada em produção por `GameObject.AddTag`/
/// `CClass.props`/`CClass.funcs`/`ReflectionEnum.AddConstant`) já realoca via `rtti::pool_alloc`
/// SEM NUNCA escrever esse trailer — e funciona porque nenhum desses 4 arrays é
/// destruído/redimensionado pelo PRÓPRIO motor depois que o BWMS mexe neles (configurados 1x,
/// lidos o resto da vida do processo). `depot->groups` é inicializado 1x no boot
/// (`InitializeArchives`) e nunca mais tocado pela engine pelo resto da sessão — mesma categoria,
/// mesmo risco aceito pelos 4 casos já em produção, agora só explicitado por escrito.
///
/// Escopo consciente: só suporta `basePath` < 20 bytes (limite do `CString` inline/SSO, mesmo
/// limite documentado no RED4ext `#351`/`write_cstring_inline_ret`) — paths mais longos
/// exigiriam alocação heap de `CString` (não implementada neste projeto, fora de escopo; a
/// função aborta com log explícito nesse caso, nunca escreve parcial). **Nunca chamada de
/// nenhum hook/caminho de produção** — só o comando de canal `archivegroupcreate` abaixo, e
/// GATED atrás de `~/.bwms-flatwrite` (mesma trava de `mkarr`/`mkflat`/`clone` — categoria
/// "muta memória viva do motor", não só TweakDB).
unsafe fn create_archive_group_by_path(depot: *mut u8, base_path: &str) -> Option<*mut u8> {
    // passo 1 do algoritmo real: se já existe, devolve ele (nunca duplica).
    if let Some(existing) = resolve_archive_group_by_path(depot, base_path) {
        return Some(existing);
    }
    if base_path.as_bytes().len() >= 20 {
        log("[archivegroupcreate] basePath >= 20 bytes — CString heap não implementado neste projeto, abortado (fora de escopo)");
        return None;
    }
    const STRIDE: usize = 0x38;
    let groups = (depot.add(0x10) as *const *mut u8).read();
    let cap = (depot.add(0x18) as *const u32).read();
    let size = (depot.add(0x1c) as *const u32).read();
    if size > 64 || cap > 128 {
        log(&format!("[archivegroupcreate] cap/size implausível (cap={cap} size={size}) — abortado"));
        return None;
    }
    // valida a leitura sobre `cap` (não só `size`) — cobre também o caso raro `cap>0 && size==0`
    // (array reservado mas nunca preenchido: `groups` já é um ponteiro de dado real nesse caso,
    // per `DynArray<T>::GetAllocator()`, não o union "allocator no lugar de entries" que só vale
    // com `cap==0`).
    if cap > 0 && (groups.is_null() || !crate::gum::is_readable(groups as *const c_void, (cap as usize) * STRIDE)) {
        log("[archivegroupcreate] buffer de groups ilegível — abortado");
        return None;
    }
    // acha o índice de inserção: 1º grupo com scope != Mod(4); se nenhum bater (ou depot vazio),
    // insere no fim — mesmo fallback do `std::find_if`/`end()` real.
    let mut insert_idx = size as usize;
    for i in 0..size as usize {
        let gi = groups.add(i * STRIDE);
        if !crate::gum::is_readable(gi as *const c_void, STRIDE) {
            log(&format!("[archivegroupcreate] grupo [{i}] ilegível — abortado"));
            return None;
        }
        let scope = (gi.add(0x30) as *const u32).read();
        if scope != 4 {
            insert_idx = i;
            break;
        }
    }
    let new_group: *mut u8 = if size < cap {
        // slack já disponível no buffer atual — shift em memória, sem realocar (mesmo idioma de
        // `DynArray<T>::ShiftEntries`: desloca a cauda 1 slot pra frente antes de zerar o novo).
        let tail = size as usize - insert_idx;
        if tail > 0 {
            core::ptr::copy(
                groups.add(insert_idx * STRIDE),
                groups.add((insert_idx + 1) * STRIDE),
                tail * STRIDE,
            );
        }
        let slot = groups.add(insert_idx * STRIDE);
        core::ptr::write_bytes(slot, 0, STRIDE);
        // size escrito por último — mesma disciplina de `dynarray_push_ptr` (engine nunca vê
        // um size > que o buffer publicado permite).
        core::ptr::write_unaligned(depot.add(0x1c) as *mut u32, size + 1);
        slot
    } else {
        // cheio: realoca via `rtti::pool_alloc` (mesma técnica já provada em produção pros 4
        // outros arrays citados na doc acima — SEM allocator-trailer, risco aceito explícito).
        let new_cap = cap.saturating_mul(2).max(size + 4).max(4);
        let new_buf = crate::rtti::pool_alloc(new_cap as usize * STRIDE, 8) as *mut u8;
        if new_buf.is_null() {
            log("[archivegroupcreate] pool_alloc falhou — abortado");
            return None;
        }
        if insert_idx > 0 {
            core::ptr::copy_nonoverlapping(groups, new_buf, insert_idx * STRIDE);
        }
        let slot = new_buf.add(insert_idx * STRIDE);
        core::ptr::write_bytes(slot, 0, STRIDE);
        let tail = size as usize - insert_idx;
        if tail > 0 {
            core::ptr::copy_nonoverlapping(
                groups.add(insert_idx * STRIDE),
                new_buf.add((insert_idx + 1) * STRIDE),
                tail * STRIDE,
            );
        }
        // republica entries -> cap -> size (size por último, mesma disciplina de dynarray_push_ptr).
        core::ptr::write_unaligned(depot.add(0x10) as *mut u64, new_buf as u64);
        core::ptr::write_unaligned(depot.add(0x18) as *mut u32, new_cap);
        core::ptr::write_unaligned(depot.add(0x1c) as *mut u32, size + 1);
        log(&format!(
            "[archivegroupcreate] groups realocou: cap {cap}->{new_cap} entries {groups:p}->{new_buf:p} size {size}->{}",
            size + 1
        ));
        slot
    };
    // basePath (CString inline SSO, mesmo layout/técnica de `register::write_cstring_inline_ret`,
    // já provado ao vivo — RED4ext `#351`) + scope=Mod(4).
    const CSTRING_SIZE: usize = 0x20;
    let bp_bytes = base_path.as_bytes();
    core::ptr::write_bytes(new_group.add(0x10), 0, CSTRING_SIZE);
    core::ptr::copy_nonoverlapping(bp_bytes.as_ptr(), new_group.add(0x10), bp_bytes.len());
    core::ptr::write_unaligned(new_group.add(0x10 + 0x14) as *mut u32, bp_bytes.len() as u32);
    core::ptr::write_unaligned(new_group.add(0x30) as *mut u32, 4u32);
    log(&format!(
        "[archivegroupcreate] '{base_path}' -> grupo NOVO @{new_group:p} (idx={insert_idx}, scope=Mod)"
    ));
    Some(new_group)
}

/// Codeware `#16`/`#198` (`ResourceDepot.ArchiveExists`). Fonte real (`App/Depot/ResourceDepot.hpp`):
/// varre `depot->groups`, filtra `scope==Mod`, compara `std::filesystem::path(archive.path).filename()`
/// contra o nome pedido. Layout de `ArchiveInfo` (name RedString@+0x10, stride 0x50) CONFIRMADO
/// ao vivo em 2026-08-10 (`archivenamedump`, refutando a incerteza registrada em
/// `notes/RE-archiveinfo-inject-2026-07-17.md` sobre a RE estática original) — paths reais lidos
/// corretos (`basegame_1_engine.archive` etc.). **Divergência documentada**: em vez de filtrar por
/// `scope==Mod` (campo de scope do GRUPO ainda não mapeado com certeza — `+0x30` do grupo é
/// "key" 3=content/1=memoryresident por uma nota antiga, não confirmadamente "Mod"), varre TODOS
/// os grupos — superset seguro (nunca dá falso-negativo pra archive de mod real; risco de
/// falso-positivo só em colisão de nome de arquivo entre mod e vanilla, praticamente nulo).
pub unsafe fn archive_exists_by_name(name: &str) -> bool {
    let depot = PATHB_DEPOT.load(Ordering::Relaxed) as *mut u8;
    if depot.is_null() {
        log("[archiveexists] depot ainda não capturado (InitializeArchives não rodou) — abortado");
        return false;
    }
    let groups = (depot.add(0x10) as *const *mut u8).read();
    let gcount = (depot.add(0x1c) as *const u32).read();
    if groups.is_null() || gcount == 0 || gcount > 64 {
        return false;
    }
    for gi_idx in 0..gcount as usize {
        let gi = groups.add(gi_idx * 0x38);
        if !crate::gum::is_readable(gi as *const c_void, 0x38) {
            continue;
        }
        let cnt = (gi.add(0x0c) as *const u32).read();
        let arr = (gi as *const *const u8).read();
        if arr.is_null() || cnt == 0 || cnt > 100_000 {
            continue;
        }
        for i in 0..cnt as usize {
            let entry = arr.add(i * 0x50);
            if !crate::gum::is_readable(entry as *const c_void, 0x50) {
                continue;
            }
            let path = crate::rtti::red_string_read(entry.add(0x10));
            let filename = path.rsplit(['/', '\\']).next().unwrap_or(&path);
            if filename == name {
                return true;
            }
        }
    }
    false
}

/// `<jogo>/` a partir do path da PRÓPRIA dylib carregada (`<jogo>/red4ext/libcp77_console.dylib`,
/// sempre presente — é o próprio processo rodando). Reusado por `pathb_inject`/`facade_register_*`/
/// `register::tramp_game_file_exists` (Codeware `Utils.Compatibility.GameFileExists`).
pub(crate) unsafe fn game_dir_from_dylib() -> Option<String> {
    let n = _dyld_image_count();
    for i in 0..n {
        let nm = _dyld_get_image_name(i);
        if nm.is_null() {
            continue;
        }
        if let Ok(s) = std::ffi::CStr::from_ptr(nm).to_str() {
            if let Some(p) = s.strip_suffix("/red4ext/libcp77_console.dylib") {
                return Some(p.to_string());
            }
        }
    }
    None
}

/// `axl-pathb-injection-arbitrary`: injeta um .archive REAL cujo nome NÃO casa o glob nativo
/// (basegame_*/audio_*/lang_*) no grupo `g` do ResourceDepot via `LoadArchives` @0x103eda488
/// (abre+parseia+APENDA, BYPASSA o filtro do glob). `depot`/`g` já resolvidos pelo chamador
/// (`find_pathb_content_group`); `abs_path` é o `.archive` a carregar (path absoluto real).
/// Segura o lock exclusivo do depot durante a injeção. Devolve `true` se `count` subiu.
unsafe fn load_archive_into_group(depot: *mut u8, g: *mut u8, abs_path: &str) -> bool {
    use core::arch::asm;
    if !std::path::Path::new(abs_path).is_file() {
        log(&format!("[pathb] arquivo ausente em {abs_path} — abortado"));
        return false;
    }
    let cpath = match std::ffi::CString::new(abs_path) {
        Ok(c) => c,
        Err(_) => {
            log(&format!("[pathb] path '{abs_path}' tem byte nulo interno — abortado"));
            return false;
        }
    };
    let count0 = (g.add(0x0c) as *const u32).read();

    // ctor de CString inline (0x20B): void(out@x0, cstr@x1). O item da fileList É uma CString (stride 0x20).
    let cstr_ctor = crate::rebase(0x1_0002_cdb8);
    let mut s_path = [0u8; 0x20];
    asm!("blr {f}", f = in(reg) cstr_ctor,
        in("x0") s_path.as_mut_ptr(), in("x1") cpath.as_ptr(), clobber_abi("C"));

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

    // SCOPE: o crash report do boot #9 (2026-07-17) provou que o RDAR-open (0x103e2ebd4) deref o
    // scope como PONTEIRO e uma CString hand-built de "content" (SSO inline) fazia ele deref os
    // BYTES "content" como endereço. Fix: usar a RedString de nome/scope PRÓPRIA do grupo (já
    // construída pelo jogo, layout certo) em `g+0x10`.
    let scope_ptr = g.add(0x10) as *const u8;

    log(&format!(
        "[pathb] injetando '{abs_path}' no content group g={g:p} (archives ANTES={count0}) scope=g+0x10 segurando o lock..."
    ));
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
        ">>> INJETADO no content-group (depot REAL, thread do jogo, com lock) via LoadArchives — count subiu <<<"
    } else if count1 == count0 {
        "FALHA: count inalterado — o open() rejeitou o archive (magic/versão) ou o append não ocorreu"
    } else {
        "ATENÇÃO: count mudou de forma inesperada (concorrência?)"
    };
    log(&format!(
        "[pathb] g={g:p} | archives ANTES={count0} DEPOIS={count1} | path={abs_path} | {verdict}"
    ));
    ok
}

/// Regressão do achado original `axl-pathb-injection-arbitrary`: injeta um .archive de TESTE fixo
/// (`zz_bwms_pathb.archive`), gated pelo marcador de dev de sempre (`~/.bwms-pathb-inject`) — só
/// mantido pra não perder a prova ao vivo já feita (10 boots, 2026-07-17). A capacidade REAL pra
/// mods é `facade_register_archive`/`facade_register_dir` (abaixo), sem marker, path arbitrário.
unsafe fn pathb_inject(depot: *mut u8) {
    let do_inject = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-pathb-inject").exists())
        .unwrap_or(false);
    if !do_inject {
        return;
    }
    let Some(g) = find_pathb_content_group(depot) else { return };
    let Some(game_dir) = game_dir_from_dylib() else {
        log("[pathb] não achei o dir do jogo (via dylib image) — abortado");
        return;
    };
    let apath = format!("{game_dir}/archive/Mac/content/zz_bwms_pathb.archive");
    load_archive_into_group(depot, g, &apath);
}

/// Prefixos vanilla (`basegame_`/`audio_`/`lang_`/`dlc_`, o próprio glob nativo que
/// `RegisterArchive`/Path-B existe pra CONTORNAR): um archive com um desses nomes quase certamente
/// já está carregado pela via normal. Achado 2026-08-09: re-injetar `basegame_4_gamedata.archive`
/// (produção, grande) via este bypass CROU (`EXC_BAD_ACCESS`, thread `redDispatcher5`, dentro de
/// `load_archive_into_group`) — causa raiz não isolada, mas o caso de uso é sempre ilegítimo (um
/// mod nunca precisa re-registrar conteúdo vanilla), então recusar aqui é estritamente mais seguro
/// sem perder capacidade real nenhuma.
fn is_vanilla_archive_name(path: &str) -> bool {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    ["basegame_", "audio_", "lang_", "dlc_"]
        .iter()
        .any(|pfx| name.starts_with(pfx))
}

/// `ArchiveXL.RegisterArchive(path: String) -> Bool` (`PENDENCIAS-UNIFICADAS.md`, Facade.hpp) —
/// capacidade REAL pra mods, generalização do `axl-pathb-injection-arbitrary` já provado: qualquer
/// mod pode registrar um `.archive` seu em runtime (chamável de `OnAttach`/bootstrap redscript),
/// sem precisar que o nome bata o glob nativo (`basegame_*`/`audio_*`/`lang_*`). `path` absoluto
/// USA como está; relativo resolve contra o diretório do jogo (`<jogo>/<path>`).
/// Codeware `#16`/`#198` (`PENDENCIAS-UNIFICADAS.md`) — `ResourceDepot.ResourceExists(path)`,
/// a API redscript-facing que um mod chamaria via `import Codeware.Depot.*`. Composição de 2
/// peças JÁ PROVADAS, zero RE nova: `PATHB_DEPOT` (o depot real, capturado 1x em
/// `InitializeArchives`, sempre instalado desde o Facade — mesma fonte que `RegisterArchive`
/// já usa) + `ResourceDepot::CheckResource@0x103ed9e9c` (achado 2026-07-16, PROVADO via
/// `axl-copy-makeexist`: `bool(depot*, ResourcePath hash)`). Chamada DIRETA (sem hook/replace —
/// só invoca o endereço como função pura), zero mutação, zero side-effect.
pub unsafe fn resource_exists(path: &str) -> bool {
    let depot = PATHB_DEPOT.load(Ordering::Relaxed);
    if depot.is_null() {
        log("[depot] ResourceExists: depot ainda não capturado (InitializeArchives não rodou) — abortado");
        return false;
    }
    let check_addr = crate::rebase(0x1_03ed_9e9c);
    if !crate::gum::is_readable(check_addr as *const c_void, 4) {
        log("[depot] ResourceExists: CheckResource ilegível — abortado");
        return false;
    }
    let hash = bwms_hashes::resource_path_hash(path);
    let f: unsafe extern "C" fn(*mut c_void, u64) -> u8 = std::mem::transmute(check_addr);
    let r = f(depot, hash) != 0;
    log(&format!("[depot] ResourceExists('{path}', hash={hash:#018x}) = {r}"));
    r
}

pub unsafe fn facade_register_archive(path: &str) -> bool {
    if is_vanilla_archive_name(path) {
        log(&format!(
            "[facade] RegisterArchive: '{path}' tem nome de prefixo VANILLA (basegame_/audio_/lang_/dlc_) — \
             recusado (já carregado pela via normal; ver achado 2026-08-09 de crash ao re-injetar)"
        ));
        return false;
    }
    let depot = PATHB_DEPOT.load(Ordering::Relaxed);
    if depot.is_null() {
        log("[facade] RegisterArchive: depot ainda não capturado (InitializeArchives não rodou, ou hook recusou) — abortado");
        return false;
    }
    let abs = if std::path::Path::new(path).is_absolute() {
        path.to_string()
    } else {
        match game_dir_from_dylib() {
            Some(gd) => format!("{gd}/{path}"),
            None => {
                log("[facade] RegisterArchive: path relativo mas não achei o dir do jogo — abortado");
                return false;
            }
        }
    };
    let Some(g) = find_pathb_content_group(depot as *mut u8) else { return false };
    load_archive_into_group(depot as *mut u8, g, &abs)
}

/// `ArchiveXL.RegisterDir(path: String) -> Bool` — registra TODOS os `.archive` (não-recursivo)
/// de um diretório via `facade_register_archive` (mesma regra path absoluto/relativo-ao-jogo).
/// Devolve `true` se o diretório foi lido e PELO MENOS 1 arquivo foi injetado com sucesso.
pub unsafe fn facade_register_dir(path: &str) -> bool {
    let abs_dir = if std::path::Path::new(path).is_absolute() {
        path.to_string()
    } else {
        match game_dir_from_dylib() {
            Some(gd) => format!("{gd}/{path}"),
            None => {
                log("[facade] RegisterDir: path relativo mas não achei o dir do jogo — abortado");
                return false;
            }
        }
    };
    let entries = match std::fs::read_dir(&abs_dir) {
        Ok(e) => e,
        Err(err) => {
            log(&format!("[facade] RegisterDir: não consegui ler '{abs_dir}': {err}"));
            return false;
        }
    };
    let mut any_ok = false;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("archive") {
            if let Some(s) = p.to_str() {
                if facade_register_archive(s) {
                    any_ok = true;
                }
            }
        }
    }
    any_ok
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

// CET `FunctionOverride` (não-Lua, `fnoverride.rs`, 2026-08-11) — handlers de teste pro comando
// `fnoverridetest`/`fnoverridereplace`. `NativeHandler = unsafe extern "C" fn(ctx, frame, aOut, a4)`.
unsafe extern "C" fn fnoverride_test_before(_ctx: *mut c_void, _frame: *mut c_void, _out: *mut c_void, _a4: i64) {
    log("[fnoverride-test] BEFORE disparou");
}
unsafe extern "C" fn fnoverride_test_after(_ctx: *mut c_void, _frame: *mut c_void, _out: *mut c_void, _a4: i64) {
    log("[fnoverride-test] AFTER disparou");
}
unsafe extern "C" fn fnoverride_test_replace(_ctx: *mut c_void, _frame: *mut c_void, out: *mut c_void, _a4: i64) {
    log("[fnoverride-test] REPLACE disparou — escrevendo 999 no lugar do retorno original");
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut i32, 999);
    }
}

/// `findinksystembss`/`walkinksystem`/`inksystemvalidate` (2026-08-14) — teste estrutural de
/// "isto PARECE um `Red::InkSystem*` vivo?", read-only, sem crash possível (só
/// `gum::is_readable` + reads).
///
/// Layout confirmado no header vendorizado REAL (`enablers/Codeware/src/Red/InkSystem.hpp`,
/// não chutado): `inputWidget: WeakHandle<inkWidget>@0x2E8` (16B: {instance*,rc_block*}),
/// `keyboardState: KeyboardState@0x2F8` (u16 bitfield, só 6 bits reais + 10 bits SEMPRE
/// reservados/zero, `enablers/Codeware/src/Red/Input.hpp`), `requestsHandler:
/// WeakHandle<ISystemRequestsHandler>@0x370` (16B), `layerManagers:
/// DynArray<SharedPtr<InkLayerManager>>@0x380` (16B: {entries*,cap:u32,size:u32}).
/// `InkSystem::Instance` é `Core::RawPtr` (ponteiro GLOBAL cru, MESMA categoria de
/// `ResourceGameDepot`/`JournalManager` — não vtable/singleton-Get()) — por isso NÃO dá pra
/// usar a técnica de `findcgameengine` (vtable-match): não há vtable conhecida pra comparar.
///
/// **RODADA 2026-08-14 (apertando o filtro — antes ruidoso demais).** A versão anterior deste
/// filtro (só "ponteiro plausível-ou-null" por campo, faixa+alinhamento, SEM dereferenciar +
/// `size<=cap<=64` aceitando até `cap==0`) classificou **987 de 1020 objetos visitados** como
/// candidato num BFS real (`walkinksystem <root> 3 4000`) — ruído demais pra ser um sinal útil,
/// e a validação cruzada independente (`requestsHandler` vs `BwmsGetSystemRequestsHandler()`)
/// testou os 4 candidatos mais fortes dessa rodada e NENHUM bateu (ver `HISTORICO.md`
/// 2026-08-14, "inksystemvalidate"). Reescrito pra exigir MÚLTIPLOS sinais convergindo
/// SIMULTANEAMENTE (o mesmo padrão de rigor que já funcionou nesta base noutros achados —
/// "vários sinais fracos juntos, nenhum sozinho decisivo" é exatamente o que faltava aqui):
///
/// 1. **Ponteiro "mapeado", não só "plausível"** (`is_mapped_ptr_or_null`): além de
///    faixa+alinhamento, agora DEREFERENCIA o valor (`gum::is_readable(v, 8)`) — um qword que
///    parece ponteiro mas não aponta pra memória mapeada é descartado na hora, não só aceito
///    como "ponteiro plausível" (o critério antigo, que nunca lia o que o ponteiro apontava).
/// 2. **Invariante de nulidade PAREADA nos 2 `WeakHandle`** (`inputWidget`/`requestsHandler`):
///    por `SharedPtrBase<T>` (`RED4ext.SDK/Memory/SharedPtr.hpp`), TODO construtor real seta
///    `instance`/`refCount` JUNTOS — nunca um null e o outro não (ctor default: os 2 null;
///    copy/move/swap: os 2 sempre movidos/copiados como par). Exigir `(instance==0) ==
///    (refCount==0)` elimina candidatos onde só 1 dos 2 slots vira ponteiro por coincidência.
/// 3. **`RefCnt` plausível quando presente**: se `refCount!=0`, lê os 2 `u32`
///    (`strongRefs`/`weakRefs`, `RED4ext.SDK/Memory/SharedPtr.hpp::RefCnt`) e exige valores
///    pequenos e sãos (`<=10_000` cada) — um refcount real nunca tem bilhões de referências.
/// 4. **`keyboardState` — os 10 bits reservados TÊM que ser 0**: `Red::KeyboardState`
///    (`Red/Input.hpp`) só define 6 bits reais (`shiftLeft/Right`,`controlLeft/Right`,
///    `altLeft/Right`); os outros 10 (`b10`) são padding NUNCA escrito pelo motor — exigir
///    `kb & 0xFC00 == 0` é invariante estrutural rígido, não heurística frouxa.
/// 5. **`layerManagers` NÃO-VAZIO** (`cap>=1`, ERA `cap>=0`): a lista de layer managers
///    (HUD/Popup/Menu/etc.) é povoada no BOOT — um `InkSystem` vivo NUNCA tem `cap==0`. A
///    versão antiga aceitava `cap=0/size=0` como caso trivial-válido — exatamente o ruído
///    estrutural que dominava os 987 falsos-candidatos (a maioria de um heap grande tem VÁRIOS
///    arrays vazios por coincidência, então "cap==size==0" é um sinal quase inútil sozinho).
///    Mantido o teto `<=64` (nº plausível de layer managers nomeados).
/// 6. **1º elemento do array de fato inspecionado** (NOVO — a versão antiga nunca olhava
///    DENTRO do array, só os 2 campos de tamanho): lê `layerManagers.entries[0]` como
///    `SharedPtr<InkLayerManager>` (16B, mesmo layout `SharedPtrBase`) e aplica os MESMOS
///    testes 1+2 (mapeado + nulidade pareada) + exige `instance!=0` — com `size>=1` já
///    garantido pelo sinal 5, o slot 0 tem que conter um layer manager de verdade, não lixo
///    herdado de outro array.
///
/// Resultado esperado: de "987 candidatos" pra "poucas dezenas ou menos" no mesmo BFS. Devolve
/// `(keyboardState, layerManagers.cap, layerManagers.size)` se bater, `None` senão — MESMA
/// assinatura de antes, os 3 call-sites (`findinksystembss`/`walkinksystem`/
/// `inksystemvalidate`) não mudam.
unsafe fn inksystem_shape_matches(candidate: u64) -> Option<(u16, u32, u32)> {
    if candidate < 0x10000 || candidate % 8 != 0 {
        return None;
    }
    let is_plausible_ptr_or_null = |v: u64| v == 0 || (v > 0x10000 && v % 8 == 0);
    // Sinal 1: não só "parece ponteiro" — DEREFERENCIA pra confirmar que aponta pra memória
    // mapeada de verdade. Um qword 8-alinhado>0x10000 aleatório raramente sobrevive a isso.
    let is_mapped_ptr_or_null =
        |v: u64| v == 0 || (is_plausible_ptr_or_null(v) && gum::is_readable(v as *const c_void, 8));

    let f_inputwidget = candidate + 0x2E8;
    let f_kbstate = candidate + 0x2F8;
    let f_reqhandler = candidate + 0x370;
    let f_layermanagers = candidate + 0x380;
    if !gum::is_readable(f_inputwidget as *const c_void, 16)
        || !gum::is_readable(f_kbstate as *const c_void, 2)
        || !gum::is_readable(f_reqhandler as *const c_void, 16)
        || !gum::is_readable(f_layermanagers as *const c_void, 16)
    {
        return None;
    }

    // Sinais 1+2 — inputWidget: WeakHandle{instance,refCount}, mapeados + nulidade pareada.
    let iw0 = (f_inputwidget as *const u64).read_unaligned(); // instance
    let iw1 = (f_inputwidget as *const u64).add(1).read_unaligned(); // refCount
    if !is_mapped_ptr_or_null(iw0) || !is_mapped_ptr_or_null(iw1) || (iw0 == 0) != (iw1 == 0) {
        return None;
    }

    // Sinal 4 — keyboardState: os 10 bits reservados (b10) TÊM que ser 0 (nunca escritos pelo
    // motor real; `Red::KeyboardState` só define os 6 bits baixos).
    let kb = (f_kbstate as *const u16).read_unaligned();
    if kb & 0xFC00 != 0 {
        return None;
    }

    // Sinais 1+2+3 — requestsHandler: mesma checagem de inputWidget + sanidade do RefCnt
    // apontado (se presente).
    let rh0 = (f_reqhandler as *const u64).read_unaligned(); // instance
    let rh1 = (f_reqhandler as *const u64).add(1).read_unaligned(); // refCount (RefCnt*)
    if !is_mapped_ptr_or_null(rh0) || !is_mapped_ptr_or_null(rh1) || (rh0 == 0) != (rh1 == 0) {
        return None;
    }
    if rh1 != 0 {
        // RefCnt{strongRefs:u32, weakRefs:u32} — contadores reais são sempre pequenos.
        let strong = (rh1 as *const u32).read_unaligned();
        let weak = (rh1 as *const u32).add(1).read_unaligned();
        if strong > 10_000 || weak > 10_000 {
            return None;
        }
    }

    // Sinal 5 — layerManagers: NÃO-VAZIO (cap>=1, era >=0) + size dentro do cap + invariante
    // cap==0<=>entries==0 (defesa em profundidade, redundante com cap>=1 mas barata de manter).
    let lm_entries = (f_layermanagers as *const u64).read_unaligned();
    let lm_cap = (f_layermanagers.wrapping_add(8) as *const u32).read_unaligned();
    let lm_size = (f_layermanagers.wrapping_add(12) as *const u32).read_unaligned();
    if !is_mapped_ptr_or_null(lm_entries) {
        return None;
    }
    if lm_cap < 1 || lm_cap > 64 || lm_size < 1 || lm_size > lm_cap {
        return None;
    }
    if (lm_cap == 0) != (lm_entries == 0) {
        return None;
    }

    // Sinal 6 — 1º elemento do array de fato inspecionado (a versão antiga nunca olhava DENTRO
    // do array): SharedPtr<InkLayerManager>{instance,refCount}, mesmos testes de mapeamento +
    // nulidade pareada + `instance!=0` (com size>=1 já garantido, o slot 0 tem que ser um layer
    // manager real, não lixo herdado de um array de outro tipo).
    if !gum::is_readable(lm_entries as *const c_void, 16) {
        return None;
    }
    let e0_instance = (lm_entries as *const u64).read_unaligned();
    let e0_refcount = (lm_entries as *const u64).add(1).read_unaligned();
    if e0_instance == 0
        || !is_mapped_ptr_or_null(e0_instance)
        || !is_mapped_ptr_or_null(e0_refcount)
        || (e0_instance == 0) != (e0_refcount == 0)
    {
        return None;
    }

    Some((kb, lm_cap, lm_size))
}

/// `get_inksystem_singleton(reg)` (2026-08-14, Codeware `#100`/`#120`, continuação da sessão que
/// CONFIRMOU `Red::InkSystem::Get()` via `findinksystembss`+`inksystemvalidate`) — versão
/// CHAMÁVEL/CACHEADA do mesmo mecanismo de descoberta decisivo (varredura de forma na BSS +
/// cross-validação contra `BwmsGetSystemRequestsHandler()` no MESMO instante), pra qualquer
/// native que precise do ponteiro `Red::InkSystem*` sem repetir a varredura manual toda vez
/// (`GetLayers`/`GetLayer`/`GetWorldWidgets` chamam isto internamente). Reusa
/// `inksystem_shape_matches` (filtro estrutural já endurecido) + o MESMO padrão de
/// cross-validação já provado (`candidato+0x370` == `BwmsGetSystemRequestsHandler()`, 2
/// mecanismos independentes convergindo).
///
/// Cache (`INKSYSTEM_CACHED`): uma vez confirmado nesta sessão de boot, reusa o ponteiro sem
/// re-varrer — só re-valida a FORMA (barato, sem I/O de rede/RTTI) antes de reusar, pra pegar o
/// caso raro de heap realocado embaixo do candidato antigo.
pub(crate) unsafe fn get_inksystem_singleton(reg: &rtti::Registry) -> Option<u64> {
    // 1) cache já validado nesta sessão — revalida só a FORMA (barato) antes de reusar.
    let cached = INKSYSTEM_CACHED.load(Ordering::SeqCst);
    if cached != 0 && inksystem_shape_matches(cached).is_some() {
        return Some(cached);
    }

    // 2) resolve o 2º mecanismo (despacho RTTI real) UMA VEZ — reusado pra cada candidato.
    let f = register::get_function(reg, "BwmsGetSystemRequestsHandler");
    if !rtti::sane(f) {
        log("[get_inksystem_singleton] BwmsGetSystemRequestsHandler não resolveu (declaração ausente do bundle?) — abortando");
        return None;
    }
    let rf = rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
    let res = rtti::call_func(&rf, std::ptr::null_mut(), &[])?;
    let handler_instance = u64::from_le_bytes(res[0..8].try_into().unwrap());
    if handler_instance == 0 {
        log("[get_inksystem_singleton] BwmsGetSystemRequestsHandler()->instance==0 — abortando (sem handler vivo ainda)");
        return None;
    }

    // 3) varre os offsets do cluster BSS já confirmados nesta investigação (default +
    // o offset alternativo que achou o candidato decisivo em 2026-08-14) — cross-valida CADA
    // candidato de forma contra o handler já resolvido no passo 2, no MESMO instante.
    const SCAN_BASES: [u64; 2] = [0x1_0900_0000, 0x108d00000];
    const CHUNK: usize = 65536;
    const SIZE_MB: u64 = 4;
    const MAX_SHAPE_CHECKS: u32 = 20000;
    let mut buf = vec![0u8; CHUNK];
    for &start_static in &SCAN_BASES {
        let scan_start = crate::rebase(start_static) as usize;
        let scan_end = scan_start + (SIZE_MB * 1024 * 1024) as usize;
        let mut p = scan_start;
        let mut checks = 0u32;
        'scan: while p < scan_end {
            let this_len = CHUNK.min(scan_end - p);
            if gum::read_chunk(p, &mut buf[..this_len]) {
                let mut i = 0usize;
                while i + 8 <= this_len {
                    let candidate = u64::from_le_bytes(buf[i..i + 8].try_into().unwrap());
                    if candidate > 0x10000 && candidate % 8 == 0 {
                        checks += 1;
                        if checks > MAX_SHAPE_CHECKS {
                            break 'scan;
                        }
                        if inksystem_shape_matches(candidate).is_some() {
                            let f_reqhandler = candidate + 0x370;
                            if gum::is_readable(f_reqhandler as *const c_void, 16) {
                                let cand_instance = (f_reqhandler as *const u64).read_unaligned();
                                if cand_instance != 0 && cand_instance == handler_instance {
                                    INKSYSTEM_CACHED.store(candidate, Ordering::SeqCst);
                                    log(&format!(
                                        "[get_inksystem_singleton] CONFIRMADO {candidate:#x} (cross-validado contra BwmsGetSystemRequestsHandler(), cacheado)"
                                    ));
                                    return Some(candidate);
                                }
                            }
                        }
                    }
                    i += 8;
                }
            }
            p += this_len;
        }
    }
    log("[get_inksystem_singleton] nenhum candidato bateu a cross-validação nesta varredura — None");
    None
}

/// Validação OFFLINE (zero jogo, zero boot) do filtro `inksystem_shape_matches` acima —
/// possível porque `gum::is_readable` lê via `mach_vm_read_overwrite(mach_task_self_, ...)`,
/// ou seja, SEMPRE a memória do PRÓPRIO processo chamador. Rodando dentro de `cargo test`, esse
/// "próprio processo" é o binário de teste — então dá pra montar um buffer LOCAL com o layout
/// EXATO de `Red::InkSystem` (endereços reais desta memória, não fabricados) e confirmar que o
/// filtro aceita/rejeita exatamente como projetado, sem precisar do jogo rodando. Isso responde
/// ao pedido da rodada (2026-08-14): "se identificar um jeito de validar o filtro OFFLINE, use
/// isso" — não havia dump de memória salvo em `cp77-symbols/` pra reusar, mas construir um
/// candidato sintético em memória real do processo de teste é uma alternativa igualmente válida
/// (e mais forte: cobre também os casos NEGATIVOS, um dump gravado só cobriria 1 estado).
#[cfg(test)]
mod inksystem_shape_tests {
    use super::inksystem_shape_matches;
    use std::ptr::write_unaligned;

    /// Monta um buffer de 0x400 bytes (mesmo `SPAN` usado por `walkinksystem`) com um candidato
    /// BEM-FORMADO no layout exato que `inksystem_shape_matches` espera:
    /// - `inputWidget@0x2E8`/`requestsHandler@0x370`: `WeakHandle{instance,refCount}`, ambos
    ///   auto-referenciando o início do buffer (`base`) — sempre ponteiro mapeado válido nesta
    ///   memória de teste.
    /// - `requestsHandler.refCount` (`base`) aponta pro `storage[0]`, que guarda um `RefCnt`
    ///   pequeno e são de propósito (`strongRefs=1, weakRefs=1`) — sem isso, o valor cru de
    ///   `base` (um endereço de heap de verdade, quase sempre >10_000 nos bits baixos) seria
    ///   mal-interpretado como contador implausível e reprovaria por engano.
    /// - `layerManagers@0x380`: `entries` aponta pra `base+0x40` (região SEPARADA de
    ///   `storage[0]`, sem colisão), onde vive o elemento `[0]` `{instance=base,
    ///   refCount=base}` — não-nulo, mapeado, par consistente. `cap=4, size=3`.
    /// - `keyboardState@0x2F8` = `0x2A` (só bits baixos, nenhum bit reservado `b10`).
    fn build_valid_candidate() -> (Box<[u64; 128]>, u64) {
        let mut storage = Box::new([0u64; 128]); // 128*8 = 0x400 bytes, 8-alinhado
        let base = storage.as_ptr() as u64;
        assert!(base > 0x10000 && base % 8 == 0, "endereço de heap do teste fora do esperado: {base:#x}");
        unsafe {
            let p = storage.as_mut_ptr() as *mut u8;
            // storage[0] (byte 0x00): RefCnt{strongRefs=1,weakRefs=1} — alvo de requestsHandler.refCount.
            write_unaligned(p as *mut u32, 1u32);
            write_unaligned(p.add(4) as *mut u32, 1u32);
            // entries[0] (byte 0x40, região separada de storage[0]) — {instance,refCount} = base.
            write_unaligned(p.add(0x40) as *mut u64, base);
            write_unaligned(p.add(0x48) as *mut u64, base);
            // inputWidget@0x2E8
            write_unaligned(p.add(0x2E8) as *mut u64, base);
            write_unaligned(p.add(0x2E8 + 8) as *mut u64, base);
            // keyboardState@0x2F8 — só bits baixos (shiftLeft+controlLeft+altLeft, ex.), 0 nos reservados.
            write_unaligned(p.add(0x2F8) as *mut u16, 0x2Au16);
            // requestsHandler@0x370 — refCount aponta pro RefCnt são em storage[0].
            write_unaligned(p.add(0x370) as *mut u64, base);
            write_unaligned(p.add(0x370 + 8) as *mut u64, base);
            // layerManagers@0x380 — entries aponta pro elemento[0] em base+0x40, cap=4 size=3.
            write_unaligned(p.add(0x380) as *mut u64, base + 0x40);
            write_unaligned(p.add(0x388) as *mut u32, 4u32);
            write_unaligned(p.add(0x38C) as *mut u32, 3u32);
        }
        (storage, base)
    }

    #[test]
    fn candidato_bem_formado_passa() {
        let (_storage, base) = build_valid_candidate();
        let got = unsafe { inksystem_shape_matches(base) };
        let (kb, cap, size) = got.expect("candidato sintético bem-formado deveria bater");
        assert_eq!(cap, 4);
        assert_eq!(size, 3);
        assert_eq!(kb & 0xFC00, 0, "keyboardState do candidato não devia ter bit reservado setado");
    }

    /// O ACHADO-CHAVE desta rodada: este é o padrão que dominava os 987/1020 "candidatos" do
    /// filtro antigo — um bloco de heap zerado batia `cap==0<=>entries==0` (trivialmente
    /// satisfeito) e os 2 ponteiros null-ou-null passavam o teste frouxo de "plausível". O
    /// filtro novo EXIGE `layerManagers.cap>=1`, matando essa classe inteira de falso-positivo.
    #[test]
    fn zerado_por_completo_e_rejeitado_era_o_ruido_dominante() {
        let storage = Box::new([0u64; 128]);
        let base = storage.as_ptr() as u64;
        let got = unsafe { inksystem_shape_matches(base) };
        assert!(got.is_none(), "bloco 100% zerado não pode mais bater — cap=0 agora é rejeitado");
    }

    #[test]
    fn keyboardstate_com_bit_reservado_e_rejeitado() {
        let (mut storage, base) = build_valid_candidate();
        unsafe { write_unaligned((storage.as_mut_ptr() as *mut u8).add(0x2F8) as *mut u16, 0xFFFFu16) };
        assert!(unsafe { inksystem_shape_matches(base) }.is_none(), "bit reservado (b10) setado tinha que reprovar");
    }

    #[test]
    fn nulidade_despareada_em_inputwidget_e_rejeitada() {
        let (mut storage, base) = build_valid_candidate();
        unsafe {
            // instance != 0 mas refCount == 0 — nunca acontece num WeakHandle/SharedPtrBase real
            // (todo construtor seta os 2 juntos, `RED4ext.SDK/Memory/SharedPtr.hpp`).
            write_unaligned((storage.as_mut_ptr() as *mut u8).add(0x2E8 + 8) as *mut u64, 0);
        }
        assert!(
            unsafe { inksystem_shape_matches(base) }.is_none(),
            "nulidade despareada (instance≠0, refCount=0) tinha que reprovar"
        );
    }

    #[test]
    fn layermanagers_vazio_e_rejeitado_era_aceito_pelo_filtro_antigo() {
        let (mut storage, base) = build_valid_candidate();
        unsafe {
            let p = storage.as_mut_ptr() as *mut u8;
            write_unaligned(p.add(0x380) as *mut u64, 0);
            write_unaligned(p.add(0x388) as *mut u32, 0);
            write_unaligned(p.add(0x38C) as *mut u32, 0);
        }
        assert!(
            unsafe { inksystem_shape_matches(base) }.is_none(),
            "layerManagers vazio (cap=0) tinha que reprovar — o filtro antigo aceitava isso"
        );
    }

    #[test]
    fn size_maior_que_cap_e_rejeitado() {
        let (mut storage, base) = build_valid_candidate();
        unsafe { write_unaligned((storage.as_mut_ptr() as *mut u8).add(0x38C) as *mut u32, 999u32) };
        assert!(unsafe { inksystem_shape_matches(base) }.is_none(), "size>cap tinha que reprovar");
    }

    #[test]
    fn primeiro_elemento_do_array_nulo_e_rejeitado() {
        let (mut storage, base) = build_valid_candidate();
        unsafe { write_unaligned((storage.as_mut_ptr() as *mut u8).add(0x40) as *mut u64, 0) };
        assert!(
            unsafe { inksystem_shape_matches(base) }.is_none(),
            "1º elemento do array nulo (size>=1 mas slot0 vazio) tinha que reprovar — sinal novo, o filtro antigo nunca olhava dentro do array"
        );
    }

    #[test]
    fn refcount_implausivel_e_rejeitado() {
        let (mut storage, base) = build_valid_candidate();
        unsafe {
            // requestsHandler.refCount continua apontando pro storage[0], mas o strongRefs
            // gravado lá vira um número absurdo (nenhum RefCnt real tem bilhões de referências).
            write_unaligned(storage.as_mut_ptr() as *mut u32, 0xFFFF_FFFFu32);
        }
        assert!(
            unsafe { inksystem_shape_matches(base) }.is_none(),
            "RefCnt com contador implausível (bilhões) tinha que reprovar"
        );
    }
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
                log(&format!("[console] mod loaded: {}", path.trim()));
            }
            Err(e) => log(&format!("[console] loadmod error: {e}")),
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
    // equiprawv5 (2026-08-03): verificação DEFINITIVA — chama `BwmsCheckItemQty(game)` (redscript,
    // TransactionSystem.GetItemQuantity real) com GameInstance resolvido em Rust, mesmo padrão do
    // `fbtest` acima. Confirma se `QueueRequest` via frame real (equiprawv5) de fato deu o item.
    if cmd == "itemqty" {
        unsafe {
            if player.is_null() {
                log("[itemqty] player null");
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
                    let f = register::get_function(reg, "BwmsCheckItemQty");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: crate::rtti::ret_type_of(f), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(r) => log(&format!("[itemqty] BwmsCheckItemQty(game) -> qty={}", i32::from_le_bytes([r[0],r[1],r[2],r[3]]))),
                            None => log("[itemqty] call_func não completou"),
                        }
                    } else {
                        log("[itemqty] BwmsCheckItemQty não resolveu");
                    }
                }
                None => log("[itemqty] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "equipslotcheck" {
        unsafe {
            if player.is_null() {
                log("[equipslotcheck] player null");
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
                    let f = register::get_function(reg, "BwmsCheckEquipSlot");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: crate::rtti::ret_type_of(f), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(r) => log(&format!("[equipslotcheck] BwmsCheckEquipSlot(game) -> equipped={}", r[0] != 0)),
                            None => log("[equipslotcheck] call_func não completou"),
                        }
                    } else {
                        log("[equipslotcheck] BwmsCheckEquipSlot não resolveu");
                    }
                }
                None => log("[equipslotcheck] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // enumcomp: axl-fullbody-torso (cont.41) — enumera DIRETO os componentes reais do player
    // (Entity+0xA0, offset ground-truth já usado em cw-entity-builder) e conta quantos são
    // SkinnedMeshComponent, logando enabled/res_ptr de cada um. Observe-only, zero mutação.
    if cmd == "enumcomp" {
        unsafe {
            log(&register::run_enum_skinnedmesh(reg, player));
        }
        return;
    }
    // scanattach2: axl-attachment-apply / itens #40, #41, #54 (2026-08-19) — dump SOB DEMANDA do
    // array `unk90`+0xB0 (candidato `DynArray<AttachmentSlotData>`, cap~23) do componente REAL
    // `gameAttachmentSlots` do player, sem depender de um firing do hook `EquipStart`. Útil pra
    // reler o estado ATUAL pós-`give`+`equiprawv7` (equip real). Observe-only, zero mutação.
    if cmd == "scanattach2" {
        unsafe {
            log(&register::scan_attachslots_now(reg, player));
        }
        return;
    }
    // attachslotcheck <SlotSuffix>: ArchiveXL #40/#41/#54 (2026-08-19) — teste ao vivo da
    // composição pura de IsSlotEmpty/IsSlotSpawning. `SlotSuffix` é o sufixo depois de
    // "AttachmentSlots." (ex. `attachslotcheck Torso` -> `AttachmentSlots.Torso`).
    if let Some(suffix) = cmd.strip_prefix("attachslotcheck ") {
        unsafe {
            let full_name = format!("AttachmentSlots.{suffix}");
            let tdbid = bwms_hashes::tweak_db_id(&full_name);
            match register::attachslot_check_by_id(reg, player, tdbid) {
                Some((is_empty, is_spawning, resolved_name)) => log(&format!(
                    "[attachslotcheck] '{full_name}' tdbid={tdbid:#018x} resolved_name={resolved_name} \
                     IsSlotEmpty={is_empty} IsSlotSpawning={is_spawning}"
                )),
                None => log(&format!(
                    "[attachslotcheck] '{full_name}' tdbid={tdbid:#018x} — slot NÃO achado no array (componente ausente ou slotID sem match)"
                )),
            }
        }
        return;
    }
    // toggle <cname>: axl-fullbody-torso (cont.41) — ACHADO: 'torso'/'legs'/'shoes' existem como
    // entSkinnedMeshComponent reais do player, mesh carregado (res_ptr válido), mas enabled=0.
    // Este comando escreve enabled=1 no componente com o CName pedido (offset +0x8b, já usado
    // com segurança em IComponent::Toggle desde 2026-07-28). ESCRITA REAL — não gated por
    // padrão além de precisar de dev_mode (mesmo canal de todo comando de console).
    if let Some(rest) = cmd.strip_prefix("toggle ") {
        let name = rest.trim();
        unsafe {
            log(&register::run_toggle_component_by_cname(reg, player, name));
        }
        return;
    }
    // togglerebuild <cname>: axl-fullbody-torso (cont.41) — `toggle` sozinho persiste mas não
    // muda o visual (confirmado por screenshot); esta variante TAMBÉM chama o método virtual
    // "rebuild render proxy" (vtable+0x288, já observado seguro) na mesma instância, testando
    // se a reconstrução do proxy é o passo que falta pro mesh aparecer.
    if let Some(rest) = cmd.strip_prefix("togglerebuild ") {
        let name = rest.trim();
        unsafe {
            log(&register::run_toggle_and_rebuild(reg, player, name));
        }
        return;
    }
    // togglefull <cname>: axl-fullbody-torso (cont.42) — RE offline achou 2 leitores reais de
    // `enabled` (vtable+0x2d0/0x2d8, não o wrapper já testado); notificação de render exige
    // +0x24a==1 E +0x253 bit2, ambos falsos no torso por padrão. Seta os 3 campos + rebuild.
    if let Some(rest) = cmd.strip_prefix("togglefull ") {
        let name = rest.trim();
        unsafe {
            log(&register::run_toggle_full_and_rebuild(reg, player, name));
        }
        return;
    }
    // dissolve <cname>: axl-fullbody-torso (cont.42) — `+0x24a` é enum de estado (0/1/2), não
    // bool; simula a progressão 1→dispatch→2→dispatch que o sistema de dissolve/fade faria.
    if let Some(rest) = cmd.strip_prefix("dissolve ") {
        let name = rest.trim();
        unsafe {
            log(&register::run_toggle_dissolve_sequence(reg, player, name));
        }
        return;
    }
    // forceopacity <cname>: axl-fullbody-torso (cont.42, 4ª rodada) — bypassa o gate de dissolve
    // inteiro: lê o proxy real (componente+0x1d8), força os 2 bits de +0x23 + SetOpacity flag
    // em +0x21, chama a função de commit (`0x1016a7344`) DIRETO.
    if let Some(rest) = cmd.strip_prefix("forceopacity ") {
        let name = rest.trim();
        unsafe {
            log(&register::run_force_proxy_opacity(reg, player, name));
        }
        return;
    }
    // dumpcomp <cname>: axl-fullbody-torso (cont.41) — dump hex bruto (0x300 bytes) do componente
    // achado por CName, pra comparação offline contra um componente sabidamente visível (ex.
    // pescoço FPP) e achar o offset real do gate de visibilidade (chunkMask ou equivalente).
    if let Some(rest) = cmd.strip_prefix("dumpcomp ") {
        let name = rest.trim();
        unsafe {
            log(&register::run_dump_component_by_cname(reg, player, name));
        }
        return;
    }
    // renderproxyvis <cname>: Codeware #254 (Rendering.hpp, Raw::RenderProxy::IsVisible) —
    // leitura pura de campo, zero endereço nativo. renderProxy@+0x1e0 (SharedPtr.instance,
    // offset do header GERADO entSkinnedMeshComponent.hpp, mesma família já provada pelo
    // chunkmaskget/#22) -> Flags@proxy+0x23, IsVisible=(flags&6)==6. Observe-only.
    if let Some(rest) = cmd.strip_prefix("renderproxyvis ") {
        let name = rest.trim();
        unsafe {
            log(&register::run_render_proxy_visible(reg, player, name));
        }
        return;
    }
    // dumpat <0xhex>: variante de dumpcomp que aceita ponteiro cru (já achado via enumcomp),
    // pra comparar um componente habilitado sem precisar adivinhar o CName exato.
    if let Some(rest) = cmd.strip_prefix("dumpat ") {
        let hexstr = rest.trim().trim_start_matches("0x");
        if let Ok(addr) = u64::from_str_radix(hexstr, 16) {
            unsafe {
                log(&register::run_dump_component_at(addr as *mut std::os::raw::c_void));
            }
        } else {
            log(&format!("[dumpat] hex inválido: '{rest}'"));
        }
        return;
    }
    // dumpres <0xptr_componente>: dump do RESOURCE apontado por res_ptr@+0x1f8 do componente
    // (não o componente em si). Hipótese: gate de visibilidade dentro do resource, não no comp.
    if let Some(rest) = cmd.strip_prefix("dumpres ") {
        let hexstr = rest.trim().trim_start_matches("0x");
        if let Ok(addr) = u64::from_str_radix(hexstr, 16) {
            unsafe {
                log(&register::run_dump_resource_at(addr as *mut std::os::raw::c_void));
            }
        } else {
            log(&format!("[dumpres] hex inválido: '{rest}'"));
        }
        return;
    }
    // xformcap: Codeware `#24` (2026-08-19, RE ao vivo dedicada) — captura observe-only do
    // ponteiro `IPlacedComponent*` real do player via `Entity+0xB0` (`transformComponent`,
    // offset confirmado por leitura de fonte `Red/Entity.hpp:54` + item #60 do catálogo
    // ArchiveXL — NUNCA lido ao vivo antes de hoje). Loga o ponteiro do player, o ponteiro do
    // componente, e os 2 endereços-alvo candidatos pro watchpoint de hardware (`+0xC0`
    // localTransform / `+0xE0` worldTransform) — prontos pra colar num `lldb watchpoint set
    // expression`. Zero escrita, zero call, só leitura crua de memória já mapeada.
    if cmd == "xformcap" {
        unsafe {
            if player.is_null() {
                return log("[xformcap] player null");
            }
            let player_addr = player as u64;
            let field_addr = player_addr + 0xB0;
            if !crate::gum::is_readable(field_addr as *const c_void, 8) {
                return log(&format!("[xformcap] player={player_addr:#x} +0xB0={field_addr:#x} ILEGÍVEL"));
            }
            let comp_ptr = (field_addr as *const u64).read();
            if comp_ptr == 0 {
                return log(&format!("[xformcap] player={player_addr:#x} transformComponent@+0xB0={field_addr:#x} -> NULO"));
            }
            let watch_local = comp_ptr + 0xC0;
            let watch_world = comp_ptr + 0xE0;
            let local_ok = crate::gum::is_readable(watch_local as *const c_void, 0x20);
            let world_ok = crate::gum::is_readable(watch_world as *const c_void, 0x20);
            log(&format!(
                "[xformcap] player={player_addr:#x} transformComponent(Entity+0xB0)={comp_ptr:#x} | localTransform(+0xC0)={watch_local:#x} legivel={local_ok} | worldTransform(+0xE0)={watch_world:#x} legivel={world_ok}"
            ));
        }
        return;
    }
    // chunkmaskget <cname>: ArchiveXL `#22` (2026-08-12) — leitura PURA de chunkMask/isEnabled/
    // kind do componente achado por CName (offsets confirmados contra o header RTTI oficial,
    // ver register.rs). Observe-only, zero mutação — 1º passo seguro antes de `chunkmaskover`.
    if let Some(rest) = cmd.strip_prefix("chunkmaskget ") {
        let name = rest.trim();
        unsafe {
            log(&register::run_get_chunk_mask_by_cname(reg, player, name));
        }
        return;
    }
    // chunkmaskover <cname> <hexmask>: ArchiveXL `#22` — composição PURA de
    // `EntityState::ApplyChunkMaskOverride` (campo cru `chunkMask`, offset por tipo: 0x198
    // Mesh / 0x248 Skinned+Garment / 0x240 MorphTarget, header RTTI oficial). ESCRITA REAL —
    // mesma disciplina de `toggle`/`mkflat` (canal de dev, sem gate extra além do próprio
    // dev_mode). ⚠️ NUNCA TESTADO AO VIVO (ver register.rs pro raciocínio de confiança).
    if let Some(rest) = cmd.strip_prefix("chunkmaskover ") {
        let mut it = rest.trim().splitn(2, ' ');
        let name = it.next().unwrap_or("").trim();
        let hexstr = it.next().unwrap_or("").trim().trim_start_matches("0x");
        match u64::from_str_radix(hexstr, 16) {
            Ok(mask) => unsafe {
                log(&register::run_apply_chunk_mask_override_by_cname(reg, player, name, mask));
            },
            Err(_) => log(&format!("[chunkmaskover] uso: chunkmaskover <cname> <hexmask> (hex inválido: '{hexstr}')")),
        }
        return;
    }
    // appearancenameget/appearancenameover <cname> [hash-hex]: ArchiveXL `#22` (round 2,
    // 2026-08-12) — wire do `component_get_appearance_name`/`component_set_appearance_name`
    // já codados na rodada anterior mas nunca ligados a comando nenhum. Mesma disciplina
    // observe-only-primeiro de chunkmaskget/chunkmaskover.
    if let Some(rest) = cmd.strip_prefix("appearancenameget ") {
        let name = rest.trim();
        unsafe {
            log(&register::run_get_appearance_name_by_cname(reg, player, name));
        }
        return;
    }
    if let Some(rest) = cmd.strip_prefix("appearancenameover ") {
        let mut it = rest.trim().splitn(2, ' ');
        let name = it.next().unwrap_or("").trim();
        let hexstr = it.next().unwrap_or("").trim().trim_start_matches("0x");
        match u64::from_str_radix(hexstr, 16) {
            Ok(cname_hash) => unsafe {
                log(&register::run_set_appearance_name_by_cname(reg, player, name, cname_hash));
            },
            Err(_) => log(&format!("[appearancenameover] uso: appearancenameover <cname> <hash-hex> (hex inválido: '{hexstr}')")),
        }
        return;
    }
    // vtagcount/vtagapply <tag>: ArchiveXL `#21`/`#22` (round 2, 2026-08-12) — a tabela de 14
    // tags built-in (`GetTagManager`, item `#21`, já fechada offline) composta com a escrita
    // de chunkMask (`#22`) numa capacidade genuinamente nova: aplicar uma tag conhecida
    // ("hide_Torso"/"HighHeels"/etc.) em TODOS os componentes do player que baterem por nome.
    // `vtagcount` é dry-run (zero mutação), sempre testar antes de `vtagapply`.
    if let Some(rest) = cmd.strip_prefix("vtagcount ") {
        let tag = rest.trim();
        unsafe {
            log(&register::run_visual_tag_override_count(reg, player, tag));
        }
        return;
    }
    if let Some(rest) = cmd.strip_prefix("vtagapply ") {
        let tag = rest.trim();
        unsafe {
            log(&register::run_visual_tag_override_apply(reg, player, tag));
        }
        return;
    }
    // vtagapplymod/vtagremovemod <tag> <modhash-hex>: item `#22` round 6 (2026-08-12) —
    // variante MULTI-MOD de vtagapply: 2 chamadas com `modhash` DIFERENTE na MESMA
    // tag/componente agora COMBINAM (AND-hiding/OR-showing, fiel ao `ComponentState` real) em
    // vez de "o último a chamar vence". `vtagremovemod` desfaz só a contribuição daquele
    // modhash (uninstall-safety) — se era o último override do componente, volta pro baseline
    // cacheado no 1º apply.
    if let Some(rest) = cmd.strip_prefix("vtagapplymod ") {
        let mut it = rest.trim().splitn(2, ' ');
        let tag = it.next().unwrap_or("").trim();
        let hexstr = it.next().unwrap_or("").trim().trim_start_matches("0x");
        match u64::from_str_radix(hexstr, 16) {
            Ok(mod_hash) => unsafe {
                log(&register::run_visual_tag_override_apply_for_mod(reg, player, tag, mod_hash));
            },
            Err(_) => log(&format!("[vtagapplymod] uso: vtagapplymod <tag> <modhash-hex> (hex inválido: '{hexstr}')")),
        }
        return;
    }
    if let Some(rest) = cmd.strip_prefix("vtagremovemod ") {
        let mut it = rest.trim().splitn(2, ' ');
        let tag = it.next().unwrap_or("").trim();
        let hexstr = it.next().unwrap_or("").trim().trim_start_matches("0x");
        match u64::from_str_radix(hexstr, 16) {
            Ok(mod_hash) => unsafe {
                log(&register::run_visual_tag_override_remove_for_mod(reg, player, tag, mod_hash));
            },
            Err(_) => log(&format!("[vtagremovemod] uso: vtagremovemod <tag> <modhash-hex> (hex inválido: '{hexstr}')")),
        }
        return;
    }
    // vtagapplymodrefresh/vtagremovemodrefresh: igual acima, mas TAMBÉM chama
    // `RefreshAppearance` (já provado ao vivo via `togglerebuild`) nos componentes
    // `SkinnedFamily` tocados — fecha "campo escrito" -> "efeito visual real". Categoria de
    // risco separada de propósito (dispatch de função nativa, não só campo cru).
    if let Some(rest) = cmd.strip_prefix("vtagapplymodrefresh ") {
        let mut it = rest.trim().splitn(2, ' ');
        let tag = it.next().unwrap_or("").trim();
        let hexstr = it.next().unwrap_or("").trim().trim_start_matches("0x");
        match u64::from_str_radix(hexstr, 16) {
            Ok(mod_hash) => unsafe {
                log(&register::run_visual_tag_override_apply_for_mod_refresh(reg, player, tag, mod_hash));
            },
            Err(_) => log(&format!("[vtagapplymodrefresh] uso: vtagapplymodrefresh <tag> <modhash-hex> (hex inválido: '{hexstr}')")),
        }
        return;
    }
    if let Some(rest) = cmd.strip_prefix("vtagremovemodrefresh ") {
        let mut it = rest.trim().splitn(2, ' ');
        let tag = it.next().unwrap_or("").trim();
        let hexstr = it.next().unwrap_or("").trim().trim_start_matches("0x");
        match u64::from_str_radix(hexstr, 16) {
            Ok(mod_hash) => unsafe {
                log(&register::run_visual_tag_override_remove_for_mod_refresh(reg, player, tag, mod_hash));
            },
            Err(_) => log(&format!("[vtagremovemodrefresh] uso: vtagremovemodrefresh <tag> <modhash-hex> (hex inválido: '{hexstr}')")),
        }
        return;
    }
    if cmd == "equiprawv7" {
        unsafe {
            if player.is_null() {
                log("[equiprawv7] player null");
                return;
            }
            // Mesmo fix cirúrgico do equiprawv6 (idempotente — se já instalado nesta boot, só
            // reaponta o alvo). Precisa rodar ANTES da chamada em bytecode pra QueueRequest não
            // crashar em GetInvokable()==null.
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { log("[equiprawv7] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            if !crate::selftest::install_getinvokable_fix(rf_qr.func, 0x1_03b1_f624u64) {
                log("[equiprawv7] install_getinvokable_fix falhou — abortando");
                return;
            }
            log("[equiprawv7] fix de GetInvokable instalado/reapontado — chamando BwmsEquipViaQuery...");
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsEquipViaQuery");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => log("[equiprawv7] BwmsEquipViaQuery(game) chamado — ZERO CRASH, ver [equiprawv7] no log do reds"),
                            None => log("[equiprawv7] call_func não completou"),
                        }
                    } else {
                        log("[equiprawv7] BwmsEquipViaQuery não resolveu");
                    }
                }
                None => log("[equiprawv7] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // axl-29-equipcyber (2026-08-14): mesmo padrão de equiprawv7, mas equipa `Items.MantisBlades`
    // (cyberware de braço REAL) em vez da T-shirt — teste DECISIVO do item ArchiveXL #29
    // (`ComputePuppetArmsState`). Precisa de `give Items.MantisBlades` antes (posse real).
    if cmd == "equipcyber" {
        unsafe {
            if player.is_null() {
                log("[equipcyber] player null");
                return;
            }
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { log("[equipcyber] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            if !crate::selftest::install_getinvokable_fix(rf_qr.func, 0x1_03b1_f624u64) {
                log("[equipcyber] install_getinvokable_fix falhou — abortando");
                return;
            }
            log("[equipcyber] fix de GetInvokable instalado/reapontado — chamando BwmsEquipCyberwareViaQuery...");
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsEquipCyberwareViaQuery");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => log("[equipcyber] BwmsEquipCyberwareViaQuery(game) chamado — ZERO CRASH, ver [equipcyber] no log do reds"),
                            None => log("[equipcyber] call_func não completou"),
                        }
                    } else {
                        log("[equipcyber] BwmsEquipCyberwareViaQuery não resolveu");
                    }
                }
                None => log("[equipcyber] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // axl-29-drawcyber (2026-08-18): teste DECISIVO #2 do item ArchiveXL #29 — achado honesto de
    // 2026-08-14 (proofs/2026-08-14-archivexl-29-armsdetect-DECISIVO-tentado-INCONCLUSIVO.log)
    // confirmou que `equipcyber` move o item pra posse real (RightArm) mas não popula
    // `AttachmentSlots.WeaponRight` (a arma precisa estar DESEMBAINHADA/em uso). Mesmo padrão de
    // `equipcyber`, mas chama `BwmsDrawCyberwareAndCheckArms` (bwms-tppcam.reds) — despacha
    // `DrawItemRequest{itemID=CreateQuery(Items.MantisBlades)}` via QueueRequest (mesmo fix de
    // GetInvokable, já instalado se `equiprawv7`/`equipcyber` já rodaram nesta boot) e, na
    // sequência, chama `ComputePuppetArmsState` de novo e loga o ordinal (marcador 9613 +
    // EnumInt). Rodar depois de `give Items.MantisBlades` + `equipcyber`.
    if cmd == "drawcyber" {
        unsafe {
            if player.is_null() {
                log("[drawcyber] player null");
                return;
            }
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { log("[drawcyber] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            if !crate::selftest::install_getinvokable_fix(rf_qr.func, 0x1_03b1_f624u64) {
                log("[drawcyber] install_getinvokable_fix falhou — abortando");
                return;
            }
            log("[drawcyber] fix de GetInvokable instalado/reapontado — chamando BwmsDrawCyberwareAndCheckArms...");
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            match gi {
                Some(gi) => {
                    let f = register::get_function(reg, "BwmsDrawCyberwareAndCheckArms");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => log("[drawcyber] BwmsDrawCyberwareAndCheckArms(game) chamado — ZERO CRASH, ver [drawcyber]/[variant_log] no log do reds"),
                            None => log("[drawcyber] call_func não completou"),
                        }
                    } else {
                        log("[drawcyber] BwmsDrawCyberwareAndCheckArms não resolveu");
                    }
                }
                None => log("[drawcyber] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // axl-29-armscheck (2026-08-14): re-dispara o teste do item #29 SOB DEMANDA (não só no
    // OnGameAttached original, que roda ANTES de qualquer equip manual via canal) — chama
    // `BwmsPuppetArmsStateSmoke.Run(player)` direto via resolve_func (mesmo padrão de
    // `EquipmentSystem::QueueRequest`, classe redscript comum, não native), passando o player
    // real como `ref<PlayerPuppet>` (Arg::Handle, mesmo idioma já provado em `custsys call`).
    if cmd == "armscheck" {
        unsafe {
            if player.is_null() {
                log("[armscheck] player null");
                return;
            }
            // DIAGNÓSTICO 2026-08-17: `resolve_func` (== class_by_name + resolve_in_class) sempre
            // falhava aqui, sem dar pra saber SE era a classe que não resolvia ou o método. Separado
            // em 2 passos pra isolar. Fallback via `resolve_class_via_validator_getclass` — MESMO
            // padrão já confirmado necessário pra `IGameSystem` (register.rs, cw-callbacksystem-rtti,
            // 2026-07-13): `class_by_name`/GetClass@vtbl+0x10 do singleton de `Registry::obtain()`
            // já provou NÃO resolver toda classe genuinamente registrada no RTTI por "razão ainda não
            // mapeada" — candidato de baixo risco pra uma classe scc-compilada pura (sem native/forge),
            // categoria NUNCA antes testada via resolve_func neste projeto (todo outro call-site é
            // classe vanilla ou native-forjada pelo Rust).
            let cls_direct = reg.class_by_name("BwmsPuppetArmsStateSmoke");
            let (cls, via) = if crate::rtti::sane(cls_direct) {
                (cls_direct, "class_by_name")
            } else {
                let cls_fb = crate::register::resolve_class_via_validator_getclass("BwmsPuppetArmsStateSmoke");
                (cls_fb, "resolve_class_via_validator_getclass(fallback)")
            };
            if !crate::rtti::sane(cls) {
                log("[armscheck] classe 'BwmsPuppetArmsStateSmoke' NÃO resolveu por NENHUMA via (class_by_name nem o fallback do validador) — não é problema de método, é a classe em si");
                return;
            }
            match crate::rtti::resolve_in_class(cls, "Run") {
                Some(rf) => {
                    log(&format!("[armscheck] classe resolvida via {via}; método 'Run' achado (is_static={})", rf.is_static));
                    match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Handle(player, crate::console::refcnt())]) {
                        Some(_) => log("[armscheck] BwmsPuppetArmsStateSmoke.Run(player) chamado — ver marcadores 9611/9612 no log"),
                        None => log("[armscheck] call_func não completou"),
                    }
                }
                None => log(&format!("[armscheck] classe resolvida via {via}, mas resolve_in_class('Run') falhou — método não achado nas tabelas cls+0x48/cls+0x58")),
            }
        }
        return;
    }
    // axl-transmog-apply (2026-08-05): mesmo padrão de equiprawv7, com BwmsVisualEquipViaQuery
    // (EquipVisualsRequest em vez de EquipRequest) — testa se ChangeAppearanceToItem dispara sem
    // precisar da UI de Wardrobe/espelho. O fix de GetInvokable já é global pra QUALQUER request
    // que passe por QueueRequest, então não precisa reinstalar se equiprawv7 já rodou nesta boot,
    // mas reaponta de qualquer forma (idempotente, barato).
    if cmd == "transmogviaquery" {
        unsafe {
            if player.is_null() {
                log("[transmogviaquery] player null");
                return;
            }
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { log("[transmogviaquery] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            if !crate::selftest::install_getinvokable_fix(rf_qr.func, 0x1_03b1_f624u64) {
                log("[transmogviaquery] install_getinvokable_fix falhou — abortando");
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
                    let f = register::get_function(reg, "BwmsVisualEquipViaQuery");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => log("[transmogviaquery] BwmsVisualEquipViaQuery(game) chamado — ZERO CRASH, ver [transmogviaquery]/[axl-transmog] no log"),
                            None => log("[transmogviaquery] call_func não completou"),
                        }
                    } else {
                        log("[transmogviaquery] BwmsVisualEquipViaQuery não resolveu");
                    }
                }
                None => log("[transmogviaquery] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // 2026-08-05 (auditoria de blind spots): testa persistent/@replaceMethod/TweakXL.Facade
    // numa chamada só — mesmo padrão de transmogviaquery (resolve global, passa GameInstance
    // real via PlayerPuppet.GetGame, não GetGameInstance() interno).
    if cmd == "blindspottest" {
        unsafe {
            if player.is_null() {
                log("[blindspot] player null");
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
                    let f = register::get_function(reg, "BwmsRunBlindspotTests");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => log("[blindspot] BwmsRunBlindspotTests(game) chamado — ZERO CRASH, ver [blindspot] no log"),
                            None => log("[blindspot] call_func não completou"),
                        }
                    } else {
                        log("[blindspot] BwmsRunBlindspotTests não resolveu");
                    }
                }
                None => log("[blindspot] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "playertest" {
        unsafe {
            if player.is_null() {
                log("[cw-player] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            // NOTA (2026-08-07): `register::get_function` (CRTTISystem::GetFunction, vtbl+0x30)
            // NÃO é confiável pra achar função de SCRIPT pura (só acha o que a gente registrou
            // via Rust — já documentado no projeto como "PARQUEADO, não é o resolvedor real do
            // binder"). Pra chamar `PlayerSystem.GetPlayer()` (redscript puro, via @addMethod),
            // usa `resolve_func`/`call_func` — o mesmo mecanismo já provado dezenas de vezes
            // neste projeto (getf/setf/callf), que caminha o array de métodos real da classe.
            match gi {
                Some(gi) => {
                    let ps_handle = crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetPlayerSystem")
                        .and_then(|f| crate::rtti::call_func(&f, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]));
                    match ps_handle {
                        Some(h) => {
                            let ps_ptr = usize::from_le_bytes(h[..8].try_into().unwrap()) as *mut c_void;
                            log(&format!("[cw-player] GameInstance.GetPlayerSystem(game) -> {ps_ptr:p}"));
                            if !ps_ptr.is_null() {
                                // Codeware `#45` (2026-08-11, achado do agente de pesquisa): "PlayerSystem"
                                // é só um ALIAS script-time que o `scc` resolve em compile-time — o CNAME
                                // REAL na RTTI é `gamePlayerSystem` (mesma categoria de divergência já
                                // documentada #81-85, script-name vs native-name). Bug latente desde que
                                // esta linha foi escrita, nunca pego porque o bypass `callon`/`class_of`
                                // (2026-08-10) contorna resolução por nome.
                                match crate::rtti::resolve_func(reg, "gamePlayerSystem", "GetPlayer") {
                                    Some(gp) => match crate::rtti::call_func(&gp, ps_ptr, &[]) {
                                        Some(pb) => {
                                            let p_ptr = usize::from_le_bytes(pb[..8].try_into().unwrap()) as *mut c_void;
                                            log(&format!(
                                                "[cw-player] PlayerSystem.GetPlayer() -> {p_ptr:p} (igual ao player capturado={})",
                                                p_ptr == player
                                            ));
                                        }
                                        None => log("[cw-player] call_func(GetPlayer) não completou"),
                                    },
                                    None => log("[cw-player] PlayerSystem.GetPlayer não resolveu (o @addMethod não bindou?)"),
                                }
                                match crate::rtti::resolve_func(reg, "gamePlayerSystem", "GetInventoryPuppet") {
                                    Some(gip) => match crate::rtti::call_func(&gip, ps_ptr, &[]) {
                                        Some(pb) => {
                                            let inv_ptr = usize::from_le_bytes(pb[..8].try_into().unwrap()) as *mut c_void;
                                            log(&format!(
                                                "[cw-player] PlayerSystem.GetInventoryPuppet() -> {inv_ptr:p} (defined={})",
                                                !inv_ptr.is_null()
                                            ));
                                        }
                                        None => log("[cw-player] call_func(GetInventoryPuppet) não completou"),
                                    },
                                    None => log("[cw-player] PlayerSystem.GetInventoryPuppet não resolveu"),
                                }
                            }
                        }
                        None => log("[cw-player] GameInstance.GetPlayerSystem falhou"),
                    }
                }
                None => log("[cw-player] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // `sysvtdump` (2026-08-11, RED4ext.SDK #472) — diagnóstico READ-ONLY (zero chamada) dos
    // primeiros slots da vtable do `GameInstance*` real. A 1ª tentativa de chamar +0x08 direto
    // (offset do header vendorizado Windows) CROU — hipótese: mesma convenção Itanium dual-dtor
    // já estabelecida neste projeto (Mac tem 2 dtor slots vs 1 no MSVC, desloca tudo +0x08).
    // Dump os slots 0x00-0x50 pra julgar plausibilidade (padrão de prólogo ARM64) antes de
    // qualquer retry de chamada.
    if cmd == "sysvtdump" {
        unsafe {
            if player.is_null() {
                log("[sysvtdump] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            let game_ptr = match gi {
                Some(g) => usize::from_le_bytes(g[..8].try_into().unwrap()) as *mut c_void,
                None => {
                    log("[sysvtdump] PlayerPuppet.GetGame falhou");
                    return;
                }
            };
            log(&format!("[sysvtdump] game_ptr={game_ptr:p}"));
            let base = crate::game_base();
            for (off, slot, bytes) in crate::rtti::game_instance_vtable_dump(game_ptr, 11) {
                let vmaddr = if (slot as usize) > base { slot as usize - base + 0x1_0000_0000 } else { slot as usize };
                let op0 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                let plausible = slot != 0 && op0 != 0 && op0 != 0xFFFFFFFF;
                log(&format!(
                    "[sysvtdump] +{off:#x} slot={slot:#x} vmaddr={vmaddr:#010x} bytes={bytes:02x?} plausible={plausible}"
                ));
            }
        }
        return;
    }
    // `sysbytype <classname> <slot_off_hex>` (2026-08-11, RED4ext.SDK #472) — resolve um system
    // genérico via `GameInstance::GetSystem(IType*)`, agora com offset EXPLÍCITO (achado via
    // `sysvtdump` primeiro) em vez do +0x08 hardcoded que crashou. Cross-valida contra
    // `GameInstance.GetPlayerSystem` (native já provado) quando classname=="gamePlayerSystem".
    if let Some(rest) = cmd.strip_prefix("sysbytype ") {
        let mut parts = rest.split_whitespace();
        let class_name = parts.next().unwrap_or("");
        let slot_off = parts
            .next()
            .and_then(|s| usize::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0x08);
        unsafe {
            if player.is_null() {
                log("[sysbytype] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            let game_ptr = match gi {
                Some(g) => usize::from_le_bytes(g[..8].try_into().unwrap()) as *mut c_void,
                None => {
                    log("[sysbytype] PlayerPuppet.GetGame falhou");
                    return;
                }
            };
            let itype = reg.class_by_name(class_name);
            if itype.is_null() {
                log(&format!("[sysbytype] class_by_name('{class_name}') não resolveu"));
                return;
            }
            let sys_ptr = crate::rtti::game_instance_get_system(game_ptr, itype, slot_off);
            log(&format!("[sysbytype] GetSystem('{class_name}', slot_off={slot_off:#x}) -> {sys_ptr:p} game={game_ptr:p} itype={itype:p}"));
            if class_name == "gamePlayerSystem" {
                let ps_handle = crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetPlayerSystem")
                    .and_then(|f| crate::rtti::call_func(&f, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi.unwrap())]));
                if let Some(h) = ps_handle {
                    let known_ptr = usize::from_le_bytes(h[..8].try_into().unwrap()) as *mut c_void;
                    log(&format!(
                        "[sysbytype] cross-check: GameInstance.GetPlayerSystem(game) -> {known_ptr:p} (match={})",
                        known_ptr == sys_ptr
                    ));
                }
            }
        }
        return;
    }
    // `updateregistrar` (2026-08-11, mesma sessão, RED4ext.SDK #461) — read-only: reporta o
    // ponteiro `UpdateRegistrar*` capturado pelo probe `update_registrar_group_probe` (se já
    // armado via `~/.bwms-updateregistrar-probe` e se o motor já chamou `RegisterUpdate` pra
    // algum sistema vanilla, o que acontece naturalmente durante o boot). Zero mutação.
    if cmd == "updateregistrar" {
        let ptr = crate::selftest::UPDATE_REGISTRAR_CAPTURED_PTR.load(std::sync::atomic::Ordering::Relaxed);
        if ptr == 0 {
            log("[updateregistrar] nenhum ponteiro capturado ainda (probe armado? RegisterUpdate já disparou?)");
        } else {
            log(&format!("[updateregistrar] UpdateRegistrar* capturado = {:#018x}", ptr));
        }
        return;
    }
    // `registerupdate` (2026-08-11, mesma sessão, RED4ext.SDK #461) — a ÚLTIMA peça: constrói o
    // `Callback<>` real (ABI mapeada) e chama `RegisterUpdate` no `UpdateRegistrar*` já capturado,
    // registrando `bwms_update_tick_callback` (função MÍNIMA, só loga, nunca lê FrameInfo/JobQueue)
    // pra confirmar que a chamada de registro NÃO crasha e que o callback DISPARA de verdade nos
    // frames seguintes. Toca maquinaria de registro do motor ATIVAMENTE (não mais só observar) —
    // gate PRÓPRIO (`~/.bwms-registerupdate-confirm`) além do probe já precisar estar armado.
    if cmd == "registerupdate" {
        let confirm = std::env::var("HOME")
            .ok()
            .map(|h| std::path::Path::new(&h).join(".bwms-registerupdate-confirm").exists())
            .unwrap_or(false);
        if !confirm {
            log("[registerupdate] BLOQUEADO: crie ~/.bwms-registerupdate-confirm p/ habilitar (toca registro ativo do motor)");
            return;
        }
        if player.is_null() {
            log("[registerupdate] player null");
            return;
        }
        unsafe {
            crate::selftest::register_bwms_update_tick(player);
        }
        return;
    }
    // `gisysmapdump [n]` (2026-08-11, mesma sessão, RED4ext.SDK #472) — diagnóstico depois do
    // 1º teste ao vivo de `sysbymap` ter dado NEGATIVO (itype não achado). Dump de TODAS as
    // chaves reais do systemMap (não busca por 1 nome) — resolve cada chave de volta pra nome
    // via `type_name_getname`, deixa comparar visualmente contra o que `class_by_name` resolve.
    if let Some(rest) = cmd.strip_prefix("gisysmapdump") {
        let n: usize = rest.trim().parse().unwrap_or(30);
        unsafe {
            if player.is_null() {
                log("[gisysmapdump] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            let game_ptr = match gi {
                Some(g) => usize::from_le_bytes(g[..8].try_into().unwrap()) as *mut c_void,
                None => {
                    log("[gisysmapdump] PlayerPuppet.GetGame falhou");
                    return;
                }
            };
            crate::rtti::game_instance_dump_system_map(game_ptr, n);
            let itype = reg.class_by_name("gamePlayerSystem");
            log(&format!("[gisysmapdump] class_by_name('gamePlayerSystem') -> {itype:p} (comparar contra as chaves acima)"));
        }
        return;
    }
    // `sysbymap <classname>` (2026-08-11, sessão nova, RED4ext.SDK #472) — via SEGURA (zero
    // execução de código do motor): lê `systemMap` @ `GameInstance+0x08` DIRETO em vez de chamar
    // slot de vtable (mesmo campo-de-objeto, imune ao shift Itanium que já causou o crash da
    // tentativa anterior por vtable-call). Cross-valida contra `GetPlayerSystem` (native já
    // provado) quando classname=="gamePlayerSystem" — mesmo padrão do `sysbytype`.
    if let Some(rest) = cmd.strip_prefix("sysbymap ") {
        let class_name = rest.trim();
        unsafe {
            if player.is_null() {
                log("[sysbymap] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            let game_ptr = match gi {
                Some(g) => usize::from_le_bytes(g[..8].try_into().unwrap()) as *mut c_void,
                None => {
                    log("[sysbymap] PlayerPuppet.GetGame falhou");
                    return;
                }
            };
            let itype = reg.class_by_name(class_name);
            if itype.is_null() {
                log(&format!("[sysbymap] class_by_name('{class_name}') não resolveu"));
                return;
            }
            let sys_ptr = crate::rtti::game_instance_get_system_by_map(game_ptr, itype);
            log(&format!("[sysbymap] GetSystem('{class_name}') -> {sys_ptr:p} game={game_ptr:p} itype={itype:p}"));
            if class_name == "gamePlayerSystem" {
                let ps_handle = crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetPlayerSystem")
                    .and_then(|f| crate::rtti::call_func(&f, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi.unwrap())]));
                if let Some(h) = ps_handle {
                    let known_ptr = usize::from_le_bytes(h[..8].try_into().unwrap()) as *mut c_void;
                    log(&format!(
                        "[sysbymap] cross-check: GameInstance.GetPlayerSystem(game) -> {known_ptr:p} (match={})",
                        known_ptr == sys_ptr
                    ));
                }
            }
        }
        return;
    }
    // `workspotdump` — ArchiveXL `#49` (2026-08-12, sessão dedicada, catálogo exaustivo):
    // `AIWorkspotManager::RegisterSpots` continua SEM endereço achado (ver nota completa em
    // `register::run_workspot_manager_probe`) — este comando NÃO hooka nada, só confirma que o
    // singleton é alcançável via `sysbymap`/systemMap (já provado, RED4ext.SDK #472) e lê o campo
    // `Spots@+0x48` (offset confirmado no header vendorizado + `SortedArray<T>` do SDK real).
    // Read-only, zero mutação, zero chamada de código do motor — mesmo padrão de `sysbymap`.
    if cmd == "workspotdump" {
        unsafe {
            if player.is_null() {
                log("[workspotdump] player null");
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
                Some(g) => {
                    let game_ptr = usize::from_le_bytes(g[..8].try_into().unwrap()) as *mut c_void;
                    log(&register::run_workspot_manager_probe(reg, game_ptr));
                }
                None => log("[workspotdump] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "transmogtryv2" {
        unsafe {
            if player.is_null() {
                log("[transmogtryv2] player null");
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
                    let f = register::get_function(reg, "BwmsTransmogTryV2");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => log("[transmogtryv2] BwmsTransmogTryV2(game) chamado — ZERO CRASH, ver [transmogtryv2] no log do reds"),
                            None => log("[transmogtryv2] call_func não completou"),
                        }
                    } else {
                        log("[transmogtryv2] BwmsTransmogTryV2 não resolveu");
                    }
                }
                None => log("[transmogtryv2] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "equipglasses" {
        unsafe {
            if player.is_null() {
                log("[equipglasses] player null");
                return;
            }
            // mesmo fix cirúrgico de GetInvokable (idempotente).
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { log("[equipglasses] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            if !crate::selftest::install_getinvokable_fix(rf_qr.func, 0x1_03b1_f624u64) {
                log("[equipglasses] install_getinvokable_fix falhou — abortando");
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
                    let f = register::get_function(reg, "BwmsEquipGlassesViaQuery");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => log("[equipglasses] BwmsEquipGlassesViaQuery(game) chamado — ver [equipglasses] no log do reds"),
                            None => log("[equipglasses] call_func não completou"),
                        }
                    } else {
                        log("[equipglasses] BwmsEquipGlassesViaQuery não resolveu");
                    }
                }
                None => log("[equipglasses] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "equipoutfit" {
        unsafe {
            if player.is_null() {
                log("[equipoutfit] player null");
                return;
            }
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { log("[equipoutfit] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            if !crate::selftest::install_getinvokable_fix(rf_qr.func, 0x1_03b1_f624u64) {
                log("[equipoutfit] install_getinvokable_fix falhou — abortando");
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
                    let f = register::get_function(reg, "BwmsEquipOutfitViaQuery");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => log("[equipoutfit] BwmsEquipOutfitViaQuery(game) chamado — ver [equipoutfit] no log do reds"),
                            None => log("[equipoutfit] call_func não completou"),
                        }
                    } else {
                        log("[equipoutfit] BwmsEquipOutfitViaQuery não resolveu");
                    }
                }
                None => log("[equipoutfit] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    if cmd == "scanslots" {
        unsafe {
            if player.is_null() {
                log("[scanslots] player null");
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
                    let f = register::get_function(reg, "BwmsScanEmptySlots");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => log("[scanslots] BwmsScanEmptySlots(game) chamado — ver [scanslots] no log do reds"),
                            None => log("[scanslots] call_func não completou"),
                        }
                    } else {
                        log("[scanslots] BwmsScanEmptySlots não resolveu");
                    }
                }
                None => log("[scanslots] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // `scanspawn` — ArchiveXL `#54` (2026-08-12, sessão dedicada, catálogo exaustivo): irmão de
    // `scanslots` (acima), mesma estrutura de dispatch, chamando `BwmsScanSpawningSlots` (native
    // VANILLA `TransactionSystem.IsSlotSpawningAnyItem`, zero endereço nativo/hook — ver nota
    // completa em `blackwall-mods-dev/bwms-tppcam.reds`). Read-only.
    if cmd == "scanspawn" {
        unsafe {
            if player.is_null() {
                log("[scanspawn] player null");
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
                    let f = register::get_function(reg, "BwmsScanSpawningSlots");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => log("[scanspawn] BwmsScanSpawningSlots(game) chamado — ver [scanspawn] no log do reds"),
                            None => log("[scanspawn] call_func não completou"),
                        }
                    } else {
                        log("[scanspawn] BwmsScanSpawningSlots não resolveu");
                    }
                }
                None => log("[scanspawn] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // `axl-garment-apply` (2026-08-02): diagnóstico OBSERVE-ONLY (zero call_func, só leitura de
    // memória) pra comparar o `CClassFunction` QUEBRADO de `EquipmentSystem::QueueRequest`
    // (`GetInvokable()@+0x28` confirmado NULL pela RE) contra um `CClassFunction` que SABEMOS
    // funcionar (`PlayerPuppet::GetGame`, chamado com sucesso centenas de vezes nesta sessão) —
    // dump lado a lado dos primeiros 0xC0 bytes de cada descritor, pra achar a diferença
    // estrutural real em vez de só teorizar. Zero risco: nenhuma chamada acontece, só leitura.
    if cmd == "qrdiag" {
        unsafe {
            let qr = crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest");
            let gg = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame");
            for (label, rf) in [("QueueRequest(QUEBRADO)", qr), ("GetGame(FUNCIONA)", gg)] {
                match rf {
                    Some(rf) => {
                        let p = rf.func as *const u8;
                        if !gum::is_readable(p as *const c_void, 0xc0) {
                            log(&format!("[qrdiag] {label}: descritor @ {p:p} ILEGÍVEL"));
                            continue;
                        }
                        log(&format!("[qrdiag] {label}: descritor @ {p:p} is_static={}", rf.is_static));
                        for off in (0x00..0xc0).step_by(8) {
                            let v = (p.add(off) as *const u64).read_unaligned();
                            let tag = if off == 0x18 { " <- ret_type" }
                                else if off == 0x28 { " <- GetInvokable()" }
                                else if off == 0x30 { " <- param_count(u32 low)" }
                                else if off == 0xa8 { " <- flags" }
                                else { "" };
                            log(&format!("[qrdiag]   +{off:#04x}: {v:#018x}{tag}"));
                        }
                    }
                    None => log(&format!("[qrdiag] {label}: resolve_func falhou")),
                }
            }
        }
        return;
    }
    // `axl-garment-apply` state-hooks (AddItem/AddCustomItem/ChangeItem/ChangeCustomItem/RemoveItem):
    // provavelmente só disparam com uma ação real de EQUIPAR — NÃO usar `BwmsEquipPoller` agendado
    // via `ds.DelayCallback` (esse mecanismo tem crash PRÓPRIO documentado, ~1min55s de execução
    // sustentada, sessão 2026-07-15). Em vez disso: 1 disparo SÍNCRONO, ÚNICO, sem agendamento —
    // mesmo padrão seguro do `fbtest` acima (resolve `game` via `PlayerPuppet.GetGame`, chama um
    // global redscript UMA VEZ). `BwmsForceEquipOnce(game)` constrói um `BwmsEquipPoller` e chama
    // `.Call()` DIRETAMENTE (não agenda), reusando 100% da lógica de EquipRequest já provada
    // end-to-end em 2026-07-15 ("Skill 2"), sem o padrão de poller recorrente que causou o crash.
    if cmd == "equiponce" {
        unsafe {
            if player.is_null() {
                log("[equiponce] player null");
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
                    let f = register::get_function(reg, "BwmsForceEquipOnce");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]).is_some() {
                            log("[equiponce] BwmsForceEquipOnce(game) chamado via canal com GameInstance real");
                        } else {
                            log("[equiponce] call_func não completou");
                        }
                    } else {
                        log("[equiponce] BwmsForceEquipOnce não resolveu (mod compilado?)");
                    }
                }
                None => log("[equiponce] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // axl-transmog-apply / axl-garment-apply (2026-08-02, /goal): via alternativa que NÃO passa por
    // `EquipmentSystem::QueueRequest` (beco-sem-saída RE-esgotado, GetInvokable() null pro descritor
    // dessa função específica — ver HISTORICO.md cont.3-6). Chama `TransactionSystem.ChangeItemAppearanceByItemID`
    // direto (`BwmsTransmogTryOnce`, bwms-tppcam.reds) — mesma categoria "consumidora" já provada
    // segura (GetVisualTags/RegisterPart/LoadAppearance), descriptor RTTI SEPARADO de QueueRequest.
    if cmd == "transmogtry" {
        unsafe {
            if player.is_null() {
                log("[transmogtry] player null");
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
                    let f = register::get_function(reg, "BwmsTransmogTryOnce");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]).is_some() {
                            log("[transmogtry] BwmsTransmogTryOnce(game) chamado via canal com GameInstance real");
                        } else {
                            log("[transmogtry] call_func não completou");
                        }
                    } else {
                        log("[transmogtry] BwmsTransmogTryOnce não resolveu (mod compilado?)");
                    }
                }
                None => log("[transmogtry] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // axl-garment-apply / axl-transmog-apply (2026-08-02, /goal): via mais direta que `transmogtry` —
    // chama `EquipmentSystem::QueueRequest` real (0x103b1f624, RE dedicada desta sessão) via transmute,
    // TOTALMENTE fora do RTTI/executor (mesmo padrão de CreateRecord/RecordExists), contornando
    // `GetInvokable()` (a causa raiz exata do crash do `equiponce`/`es.QueueRequest(req)` normal).
    if cmd == "equiprawonce" {
        unsafe {
            if player.is_null() {
                log("[equiprawonce] player null");
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
                    let f = register::get_function(reg, "BwmsForceEquipRaw");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]).is_some() {
                            log("[equiprawonce] BwmsForceEquipRaw(game) chamado via canal com GameInstance real");
                        } else {
                            log("[equiprawonce] call_func não completou");
                        }
                    } else {
                        log("[equiprawonce] BwmsForceEquipRaw não resolveu (mod compilado?)");
                    }
                }
                None => log("[equiprawonce] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // axl-garment-apply / axl-transmog-apply (2026-08-02, /goal): 3ª via — QueueRequest via call_func
    // NOSSO com ctx explícito (achado do agente que desmontou GetInvokable(), ver register.rs).
    if cmd == "equipctxonce" {
        unsafe {
            if player.is_null() {
                log("[equipctxonce] player null");
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
                    let f = register::get_function(reg, "BwmsForceEquipCtx");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]).is_some() {
                            log("[equipctxonce] BwmsForceEquipCtx(game) chamado via canal com GameInstance real");
                        } else {
                            log("[equipctxonce] call_func não completou");
                        }
                    } else {
                        log("[equipctxonce] BwmsForceEquipCtx não resolveu (mod compilado?)");
                    }
                }
                None => log("[equipctxonce] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // axl-garment-apply / axl-transmog-apply (2026-08-03, /goal): achado ao vivo — `equipctxonce` E
    // `equiprawonce` (que chamam um native de 2 params `handle` de DENTRO do bytecode de
    // `BwmsForceEquipCtx`/`Raw`) crasham no MESMO 0x1021730ec de sempre, ANTES de qualquer log nosso
    // aparecer (nem o Print imediatamente antes, nem o log de entrada do trampolim Rust). Isso aponta
    // pro DESPACHO do native aninhado como o problema, não a lógica interna dele. Via nova: ZERO
    // native aninhado. `BwmsPrepEquipSys`/`BwmsPrepEquipReq` só CONSTROEM e RETORNAM objetos (sem
    // chamar nenhum native de 2-handle-params) — 2 chamadas TOP-LEVEL separadas (mesmo padrão
    // provado de `getcustsys`: ponteiro do objeto = primeiros 8 bytes do retorno). O `transmute` pro
    // endereço cru roda 100% aqui em `run_cmd`, sem NENHUM native aninhado no meio.
    // 2026-08-03 (/goal): diagnóstico MÍNIMO/incremental — isola qual PEÇA exata da cadeia
    // GetPlayerSystem/EquipmentSystem.GetInstance introduz o crash quando a função externa (chamada
    // via nosso `call_func`) retorna `ref<T>` em vez de `Void`. `rettest` (só Print+null) já passou.
    let rettest_fn: Option<&str> = match cmd {
        "rettest2" => Some("BwmsTestRef2"),
        "rettest3" => Some("BwmsTestRef3"),
        "rettest4" => Some("BwmsTestRef4"),
        "rettest5" => Some("BwmsTestRef5"),
        "rettest6" => Some("BwmsPrepEquipSys"),
        "rettest7" => Some("BwmsPrepEquipReq"),
        _ => None,
    };
    if cmd == "rettest" || rettest_fn.is_some() {
        let fname = rettest_fn.unwrap_or("BwmsTestReturnsRef");
        unsafe {
            if player.is_null() {
                log(&format!("[{cmd}] player null"));
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            let gi = match gi {
                Some(gi) => gi,
                None => { log(&format!("[{cmd}] PlayerPuppet.GetGame falhou")); return; }
            };
            let f = register::get_function(reg, fname);
            if !crate::rtti::sane(f) {
                log(&format!("[{cmd}] {fname} não resolveu (mod compilado?)"));
                return;
            }
            log(&format!("[{cmd}] resolvido f={f:p}, chamando {fname} agora..."));
            let rf = crate::rtti::ResolvedFn { func: f, ret_type: crate::rtti::ret_type_of(f), is_static: true };
            let r = crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            log(&format!("[{cmd}] call_func retornou: {:?} — zero crash até aqui!", r.is_some()));
        }
        return;
    }
    // axl-garment-apply/axl-transmog-apply (2026-08-03, /goal): 4ª via — 1 SÓ call_func top-level
    // (Void), 2 natives aninhadas de 1-handle-arg cada capturam es/req via atomic; Rust lê depois e
    // faz o transmute pro endereço cru. Evita os 2 bugs diagnosticados nesta madrugada (cont.17/19).
    // axl-garment-apply/axl-transmog-apply (2026-08-03, /goal): diff de memória — compara um
    // EquipRequest construído via bytecode REAL (BwmsPrepAndCapture, `new` sintético, mas como
    // ÚNICA/1ª chamada substancial da sessão = sabidamente seguro) contra um via rtti::new_object,
    // byte a byte, pra achar o campo que falta em +0x6b (achado cont.22). `dumpreqbc` = bytecode;
    // `dumpreqru` = rtti::new_object puro.
    // axl-garment-apply/axl-transmog-apply (2026-08-03, /goal): instala a sonda naked observe-only
    // em 0x103b1f678 (dentro de QueueRequest, ANTES do dispatch por tabela runtime). Rodar ESTE
    // comando ANTES de `equiprawv4` — loga x0/x1/x2/x3/x8 + buf16 antes/depois, sem mudar
    // comportamento nenhum (a chamada real acontece igual).
    if cmd == "qrprobeon" {
        unsafe { crate::selftest::install_qr_dispatch_probe() };
        return;
    }
    if cmd == "dumpreqbc" || cmd == "dumpreqru" {
        unsafe {
            if player.is_null() {
                log(&format!("[{cmd}] player null"));
                return;
            }
            let req_ptr: *mut c_void;
            if cmd == "dumpreqbc" {
                let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                    crate::rtti::call_func(&gg, player, &[]).map(|b| { let mut o=[0u8;16]; o.copy_from_slice(&b[..16]); o })
                });
                let gi = match gi { Some(g) => g, None => { log("[dumpreqbc] GetGame falhou"); return; } };
                register::CAPTURED_ES_PTR.store(0, std::sync::atomic::Ordering::Relaxed);
                register::CAPTURED_REQ_PTR.store(0, std::sync::atomic::Ordering::Relaxed);
                let f = register::get_function(reg, "BwmsPrepAndCapture");
                if !crate::rtti::sane(f) { log("[dumpreqbc] BwmsPrepAndCapture não resolveu"); return; }
                let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
                req_ptr = register::CAPTURED_REQ_PTR.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void;
                log(&format!("[dumpreqbc] req via BYTECODE new: {req_ptr:p}"));
            } else {
                req_ptr = crate::rtti::new_object(reg, "gameEquipRequest");
                log(&format!("[dumpreqru] req via rtti::new_object: {req_ptr:p}"));
            }
            if req_ptr.is_null() || !crate::rtti::sane(req_ptr) {
                log(&format!("[{cmd}] req_ptr inválido"));
                return;
            }
            let mut hex = String::new();
            for i in 0..0x80usize {
                let b = *(req_ptr as *const u8).add(i);
                hex.push_str(&format!("{b:02x}"));
                if i % 8 == 7 { hex.push(' '); }
            }
            log(&format!("[{cmd}] dump 0x80 bytes @ {req_ptr:p}: {hex}"));
        }
        return;
    }
    if cmd == "equiprawv3" {
        unsafe {
            if player.is_null() {
                log("[equiprawv3] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            let gi = match gi {
                Some(gi) => gi,
                None => { log("[equiprawv3] PlayerPuppet.GetGame falhou"); return; }
            };
            register::CAPTURED_ES_PTR.store(0, std::sync::atomic::Ordering::Relaxed);
            register::CAPTURED_REQ_PTR.store(0, std::sync::atomic::Ordering::Relaxed);
            let f = register::get_function(reg, "BwmsPrepAndCapture");
            if !crate::rtti::sane(f) {
                log("[equiprawv3] BwmsPrepAndCapture não resolveu (mod compilado?)");
                return;
            }
            log("[equiprawv3] chamando BwmsPrepAndCapture (1 única call_func top-level, Void)...");
            let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
            let r = crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            log(&format!("[equiprawv3] call_func retornou: {:?} — zero crash até aqui", r.is_some()));
            let es_ptr = register::CAPTURED_ES_PTR.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void;
            let req_ptr = register::CAPTURED_REQ_PTR.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void;
            log(&format!("[equiprawv3] es_ptr={es_ptr:p} req_ptr={req_ptr:p} (via atomics, sem 2ª call_func)"));
            if es_ptr.is_null() || req_ptr.is_null() || !crate::rtti::sane(es_ptr) || !crate::rtti::sane(req_ptr) {
                log("[equiprawv3] es_ptr/req_ptr inválido — abortando antes do transmute");
                return;
            }
            log("[equiprawv3] chamando 0x103b1f624 direto via transmute...");
            let tp = crate::rebase(0x1_03b1_f624u64);
            if tp.is_null() {
                log("[equiprawv3] sem mapa pra 0x103b1f624 neste build -> pulando (inerte)");
            } else {
                let tf: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(tp);
                tf(es_ptr, req_ptr);
                log("[equiprawv3] transmute retornou — ZERO CRASH, via completa!");
            }
        }
        return;
    }
    // axl-garment-apply/axl-transmog-apply (2026-08-03, /goal): 5ª via — achado decisivo (cont.21):
    // o crash é especificamente `new T()` via `call_func` (bytecode) quando não é uma das 1ªs
    // alocações da sessão. `rtti::new_object` JÁ EXISTE (replica CClass::CreateInstance 100% em
    // Rust, usado pra outras coisas há sessões) — constrói `req` SEM bytecode nenhum, evitando o
    // mecanismo problemático por completo. `es` continua via `BwmsPrepEquipSys` (call_func, mas SEM
    // `new`, já provado seguro repetidas vezes hoje). Campos escritos via `resolve_prop_in_class`
    // (offset real, não chute) — `itemID`/`slotIndex`/`addToInventory` (EquipRequest) +
    // `owner` (PlayerScriptableSystemRequest, herdado).
    if cmd == "equiprawv4" {
        unsafe {
            if player.is_null() {
                log("[equiprawv4] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            let gi = match gi {
                Some(gi) => gi,
                None => { log("[equiprawv4] PlayerPuppet.GetGame falhou"); return; }
            };
            let es_fn = register::get_function(reg, "BwmsPrepEquipSys");
            if !crate::rtti::sane(es_fn) {
                log("[equiprawv4] BwmsPrepEquipSys não resolveu");
                return;
            }
            let es_rf = crate::rtti::ResolvedFn { func: es_fn, ret_type: crate::rtti::ret_type_of(es_fn), is_static: true };
            let es_ret = crate::rtti::call_func(&es_rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            let es_ptr = es_ret.map(|r| u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]) as *mut c_void).unwrap_or(std::ptr::null_mut());
            log(&format!("[equiprawv4] es_ptr={es_ptr:p} (via BwmsPrepEquipSys, call_func mas sem `new`)"));
            if es_ptr.is_null() || !crate::rtti::sane(es_ptr) {
                log("[equiprawv4] es_ptr inválido — abortando");
                return;
            }
            // req: construído 100% em Rust via rtti::new_object (zero bytecode `new`).
            let req_ptr = crate::rtti::new_object(reg, "gameEquipRequest");
            log(&format!("[equiprawv4] req_ptr={req_ptr:p} (via rtti::new_object, ZERO bytecode new)"));
            if req_ptr.is_null() || !crate::rtti::sane(req_ptr) {
                log("[equiprawv4] req_ptr inválido — abortando");
                return;
            }
            // FIX DECISIVO #3 v2 (2026-08-03, achado por 2ª RE offline dedicada, corrige a v1 de hoje):
            // `rtti::new_object` chama `Construct()` via vtable mas NUNCA constrói o `WeakHandle<ISerializable>`
            // auto-referente que mora DENTRO do próprio objeto, em `req+0x08..+0x17` (`ISerializable::ref`).
            // A v1 do fix (chamar `make_handle` numa saída SEPARADA/throwaway) não tinha efeito nenhum —
            // Handle_ctor escreve no `out` que você dá a ele, e uma saída throwaway nunca toca a memória
            // real do objeto. Confirmado por disasm do handler de dispatch real (índice 24, 0x1022502f4):
            // ele lê `req+0x10` (metade "refcount-block" do par) esperando um bloco real; com {0,0} o
            // valor colapsa pra um inteiro pequeno lido de outro lugar, que acaba virando o ponteiro cru
            // 0x2b deref'd no crash (slot 34 da vtable de EquipmentSystem, `ldr x8,[x0]`). Fix certo:
            // Handle_ctor com a saída APONTANDO DIRETO pra req_ptr+0x08 (auto-referência real).
            crate::rtti::make_handle((req_ptr as *mut u8).add(0x08) as *mut c_void, req_ptr);
            log("[equiprawv4] req+0x08 (ISerializable::ref) construído via make_handle DIRETO no objeto (fix v2)");
            let cls = reg.class_by_name("gameEquipRequest");
            // itemID: ItemID (16 bytes), via from_tdbid (já existe, mesma via de Skill 2/equiponce).
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "itemID") {
                if let Some(item16) = crate::rtti::from_tdbid(reg, "Items.Fixer_01_Set_TShirt") {
                    std::ptr::copy_nonoverlapping(item16.as_ptr(), (req_ptr as *mut u8).add(voff as usize), 16);
                }
            }
            // owner: wref<GameObject> — FIX #2 (mesma RE): {player, NULL} cru não é um WeakHandle
            // válido pro motor (falta o refcount-block real); constrói via make_handle (Handle_ctor)
            // igual ao req_ptr acima, mesma técnica.
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "owner") {
                let mut owner_pair = [0u8; 16];
                crate::rtti::make_handle(owner_pair.as_mut_ptr() as *mut c_void, player);
                std::ptr::copy_nonoverlapping(owner_pair.as_ptr(), (req_ptr as *mut u8).add(voff as usize), 16);
            }
            // addToInventory: Bool (1 byte) = true.
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "addToInventory") {
                *(req_ptr as *mut u8).add(voff as usize) = 1u8;
            }
            // slotIndex: Int32 = -1.
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "slotIndex") {
                ((req_ptr as *mut u8).add(voff as usize) as *mut i32).write_unaligned(-1);
            }
            // FIX DECISIVO (2026-08-03, achado por RE dedicada offline): `ScriptableSystemRequest+0x40`
            // é um campo C++ interno NÃO-refletido (confirmado pelo header vendorizado real do
            // RED4ext.SDK: `uint8_t unk40[0x48-0x40]` em ScriptableSystemRequest.hpp) — QueueRequest LÊ
            // esse campo (nunca escreve) esperando um ponteiro cru de volta pro `ScriptableSystem`/
            // `EquipmentSystem` dono (o mesmo `this`), usado internamente pra enfileirar no ring buffer
            // lock-free do próprio sistema (`CircularBuffer<THandle<ScriptableSystemRequest>>`). Nem
            // `new EquipRequest()` em redscript nem `Construct()` genérico via vtable populam isso —
            // fica null, e QueueRequest crasha lendo através dele (`+0x40..+0x5c` relativo). Fix: 1
            // escrita de 8 bytes, offset fixo (não resolvível via RTTI — não é propriedade refletida).
            (req_ptr as *mut u8).add(0x40).cast::<*mut c_void>().write_unaligned(es_ptr);
            log(&format!("[equiprawv4] req+0x40={es_ptr:p} (fix do campo interno não-refletido) — req montado, chamando 0x103b1f624 direto..."));
            let tp = crate::rebase(0x1_03b1_f624u64);
            if tp.is_null() {
                log("[equiprawv4] sem mapa pra 0x103b1f624 neste build -> pulando (inerte)");
            } else {
                let tf: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(tp);
                tf(es_ptr, req_ptr);
                log("[equiprawv4] transmute retornou — ZERO CRASH, via completa!");
            }
        }
        return;
    }
    if cmd == "equiprawv5" {
        unsafe {
            if player.is_null() {
                log("[equiprawv5] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            let gi = match gi {
                Some(gi) => gi,
                None => { log("[equiprawv5] PlayerPuppet.GetGame falhou"); return; }
            };
            let es_fn = register::get_function(reg, "BwmsPrepEquipSys");
            if !crate::rtti::sane(es_fn) {
                log("[equiprawv5] BwmsPrepEquipSys não resolveu");
                return;
            }
            let es_rf = crate::rtti::ResolvedFn { func: es_fn, ret_type: crate::rtti::ret_type_of(es_fn), is_static: true };
            let es_ret = crate::rtti::call_func(&es_rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            let es_ptr = es_ret.map(|r| u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]) as *mut c_void).unwrap_or(std::ptr::null_mut());
            log(&format!("[equiprawv5] es_ptr={es_ptr:p}"));
            if es_ptr.is_null() || !crate::rtti::sane(es_ptr) {
                log("[equiprawv5] es_ptr inválido — abortando");
                return;
            }
            let req_ptr = crate::rtti::new_object(reg, "gameEquipRequest");
            log(&format!("[equiprawv5] req_ptr={req_ptr:p}"));
            if req_ptr.is_null() || !crate::rtti::sane(req_ptr) {
                log("[equiprawv5] req_ptr inválido — abortando");
                return;
            }
            crate::rtti::make_handle((req_ptr as *mut u8).add(0x08) as *mut c_void, req_ptr);
            let cls = reg.class_by_name("gameEquipRequest");
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "itemID") {
                if let Some(item16) = crate::rtti::from_tdbid(reg, "Items.Fixer_01_Set_TShirt") {
                    std::ptr::copy_nonoverlapping(item16.as_ptr(), (req_ptr as *mut u8).add(voff as usize), 16);
                }
            }
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "owner") {
                let mut owner_pair = [0u8; 16];
                crate::rtti::make_handle(owner_pair.as_mut_ptr() as *mut c_void, player);
                std::ptr::copy_nonoverlapping(owner_pair.as_ptr(), (req_ptr as *mut u8).add(voff as usize), 16);
            }
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "addToInventory") {
                *(req_ptr as *mut u8).add(voff as usize) = 1u8;
            }
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "slotIndex") {
                ((req_ptr as *mut u8).add(voff as usize) as *mut i32).write_unaligned(-1);
            }
            (req_ptr as *mut u8).add(0x40).cast::<*mut c_void>().write_unaligned(es_ptr);
            log("[equiprawv5] req montado (mesmo setup do v4) — construindo CScriptStackFrame de verdade...");

            // FIX DECISIVO #4 (2026-08-03, achado por RE dedicada + sonda ao vivo `qrprobeon`):
            // `0x103b1f624` (QueueRequest) NÃO recebe `req` cru como 2º argumento — ele lê
            // `x1[0]` (frame->code) como CURSOR DE OPCODE da VM redscript, indexando a MESMA
            // OPCODE_TABLE (0x10908b798) que `rtti::call_func` já usa pra ler argumentos. Os
            // crashes anteriores (0x6b/0x2b/0xEB, variando por boot) eram sempre bytes CRUS do
            // vtable/heap de `req` reinterpretados como opcode — nunca foi um campo faltando em
            // `req`, é o TIPO ERRADO de 2º argumento. `rtti::call_func` normal não serve aqui:
            // ela despacha via `ADDR_EXEC`, que SEMPRE resolve a função via `GetInvokable()`
            // antes de rodar — e `GetInvokable()` é um stub que retorna null incondicional pra
            // QueueRequest (RE-esgotado, ver HISTORICO cont.14/16 desta mesma madrugada). Fix:
            // construir o frame NÓS MESMOS (mesma receita exata de `call_func`: LocalVar+ParamEnd,
            // CProperty sintética via GetType("handle:IScriptable"), Handle no local) e chamar
            // `0x103b1f624` DIRETO com esse frame como x1 — bypassa ADDR_EXEC/GetInvokable por
            // completo, mas dá a QueueRequest o formato de argumento que ela realmente espera.
            let itype = crate::register::get_type(reg, "handle:IScriptable");
            if itype.is_null() {
                log("[equiprawv5] GetType(handle:IScriptable) falhou — abortando");
                return;
            }
            let mut cprop = vec![0u8; 0x30];
            (cprop.as_mut_ptr() as *mut u64).write_unaligned(itype as u64); // CProperty+0 = IType*

            let mut locals = vec![0u8; 0x40 + 0x20]; // n=1 arg, mesma fórmula de call_func
            let dst = locals.as_mut_ptr().add(0x20);
            (dst as *mut *mut c_void).write_unaligned(req_ptr);
            (dst.add(8) as *mut *mut c_void).write_unaligned(crate::console::refcnt());

            let mut bc = vec![0u8; 16 + 1 * 9]; // mesma fórmula de call_func (n=1)
            bc[0] = 0x18; // LocalVar
            (bc.as_mut_ptr().add(1) as *mut *mut c_void).write_unaligned(cprop.as_mut_ptr() as *mut c_void);
            bc[9] = 0x26; // ParamEnd

            let mut fr = vec![0u8; 0x90];
            (fr.as_mut_ptr() as *mut *mut c_void).write_unaligned(bc.as_mut_ptr() as *mut c_void);
            (fr.as_mut_ptr().add(0x10) as *mut *mut c_void).write_unaligned(locals.as_mut_ptr() as *mut c_void);
            (fr.as_mut_ptr().add(0x18) as *mut *mut c_void).write_unaligned(locals.as_mut_ptr() as *mut c_void);
            (fr.as_mut_ptr().add(0x40) as *mut *mut c_void).write_unaligned(es_ptr);

            log(&format!(
                "[equiprawv5] frame montado (fr={:p} bc={:p} locals={:p} cprop={:p} itype={:p}) — chamando 0x103b1f624(es_ptr, &frame)...",
                fr.as_ptr(), bc.as_ptr(), locals.as_ptr(), cprop.as_ptr(), itype
            ));
            let tp = crate::rebase(0x1_03b1_f624u64);
            if tp.is_null() {
                log("[equiprawv5] sem mapa pra 0x103b1f624 neste build -> pulando (inerte)");
            } else {
                let tf: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(tp);
                tf(es_ptr, fr.as_mut_ptr() as *mut c_void);
                log("[equiprawv5] transmute retornou — ZERO CRASH, via completa (frame real)!");
            }
        }
        return;
    }
    if cmd == "equiprawv6" {
        unsafe {
            if player.is_null() {
                log("[equiprawv6] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            let gi = match gi {
                Some(gi) => gi,
                None => { log("[equiprawv6] PlayerPuppet.GetGame falhou"); return; }
            };
            let es_fn = register::get_function(reg, "BwmsPrepEquipSys");
            if !crate::rtti::sane(es_fn) {
                log("[equiprawv6] BwmsPrepEquipSys não resolveu");
                return;
            }
            let es_rf = crate::rtti::ResolvedFn { func: es_fn, ret_type: crate::rtti::ret_type_of(es_fn), is_static: true };
            let es_ret = crate::rtti::call_func(&es_rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            let es_ptr = es_ret.map(|r| u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]) as *mut c_void).unwrap_or(std::ptr::null_mut());
            log(&format!("[equiprawv6] es_ptr={es_ptr:p}"));
            if es_ptr.is_null() || !crate::rtti::sane(es_ptr) {
                log("[equiprawv6] es_ptr inválido — abortando");
                return;
            }
            let req_ptr = crate::rtti::new_object(reg, "gameEquipRequest");
            log(&format!("[equiprawv6] req_ptr={req_ptr:p}"));
            if req_ptr.is_null() || !crate::rtti::sane(req_ptr) {
                log("[equiprawv6] req_ptr inválido — abortando");
                return;
            }
            crate::rtti::make_handle((req_ptr as *mut u8).add(0x08) as *mut c_void, req_ptr);
            let cls = reg.class_by_name("gameEquipRequest");
            if let Some((voff, _, in_holder)) = crate::rtti::resolve_prop_in_class(cls, "itemID") {
                log(&format!("[equiprawv6-diag] itemID voff={voff:#x} in_holder={in_holder}"));
                if let Some(item16) = crate::rtti::from_tdbid(reg, "Items.Fixer_01_Set_TShirt") {
                    std::ptr::copy_nonoverlapping(item16.as_ptr(), (req_ptr as *mut u8).add(voff as usize), 16);
                }
            }
            if let Some((voff, _, in_holder)) = crate::rtti::resolve_prop_in_class(cls, "owner") {
                log(&format!("[equiprawv6-diag] owner voff={voff:#x} in_holder={in_holder}"));
                let mut owner_pair = [0u8; 16];
                crate::rtti::make_handle(owner_pair.as_mut_ptr() as *mut c_void, player);
                std::ptr::copy_nonoverlapping(owner_pair.as_ptr(), (req_ptr as *mut u8).add(voff as usize), 16);
            }
            if let Some((voff, _, in_holder)) = crate::rtti::resolve_prop_in_class(cls, "addToInventory") {
                log(&format!("[equiprawv6-diag] addToInventory voff={voff:#x} in_holder={in_holder}"));
                *(req_ptr as *mut u8).add(voff as usize) = 1u8;
            }
            if let Some((voff, _, in_holder)) = crate::rtti::resolve_prop_in_class(cls, "slotIndex") {
                log(&format!("[equiprawv6-diag] slotIndex voff={voff:#x} in_holder={in_holder}"));
                ((req_ptr as *mut u8).add(voff as usize) as *mut i32).write_unaligned(-1);
            }
            (req_ptr as *mut u8).add(0x40).cast::<*mut c_void>().write_unaligned(es_ptr);
            // DIAGNÓSTICO: dump dos 0x50 primeiros bytes de req_ptr logo antes da chamada, pra
            // conferir visualmente se os campos foram escritos no lugar certo (comparado ao layout
            // do header: vtable@0, ISerializable::ref@0x08, unk18@0x18, unk28@0x28(4B), nativeType
            // @0x30, valueHolder@0x38, unk40(es_ptr)@0x40).
            let dump: &[u8] = std::slice::from_raw_parts(req_ptr as *const u8, 0x80);
            log(&format!("[equiprawv6-diag] req bytes[0..0x80]={:02x?}", dump));
            log("[equiprawv6] req montado (mesmo setup do v4/v5) — fix cirúrgico de GetInvokable...");

            // FIX DECISIVO #5 (2026-08-03): equiprawv5 provou que chamar `0x103b1f624` DIRETO com
            // um frame à mão roda sem crash mas NÃO faz nada (nosso bytecode sintético só declara
            // 1 arg via LocalVar+ParamEnd — não há instrução real de corpo depois disso, então o
            // executor retorna sem invocar a lógica nativa de enfileiramento). A via CERTA é deixar
            // o `ADDR_EXEC` de sempre (o mesmo que todo `rtti::call_func` já usa com sucesso pra
            // dezenas de outras nativas) fazer TUDO — arg-parsing E dispatch pro código nativo real
            // — mas ele SÓ despacha depois de chamar `this->GetInvokable()`, que é um stub que
            // SEMPRE retorna null pra QueueRequest (RE já feita, `0x100339b2c`, cont.14 desta
            // madrugada — causa-raiz original do crash antes de qualquer workaround). Fix: hookar
            // `GetInvokable()` cirurgicamente (só pro descritor exato de QueueRequest, zero
            // regressão nos outros usos) pra devolver `0x103b1f624` em vez de null, e então usar
            // `rtti::call_func` NORMAL (a via já provada, não mais um transmute cru nosso).
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { log("[equiprawv6] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            log(&format!("[equiprawv6] rf_qr.func={:p} — instalando fix de GetInvokable...", rf_qr.func));
            if !crate::selftest::install_getinvokable_fix(rf_qr.func, 0x1_03b1_f624u64) {
                log("[equiprawv6] install_getinvokable_fix falhou — abortando");
                return;
            }
            log("[equiprawv6] fix instalado — chamando via rtti::call_func NORMAL (ADDR_EXEC)...");
            let ret = crate::rtti::call_func(&rf_qr, es_ptr, &[crate::rtti::Arg::Handle(req_ptr, crate::console::refcnt())]);
            log(&format!("[equiprawv6] call_func retornou={:?} — ZERO CRASH, via ADDR_EXEC completa!", ret.is_some()));
        }
        return;
    }
    if cmd == "equiprawv2" {
        unsafe {
            if player.is_null() {
                log("[equiprawv2] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame").and_then(|gg| {
                crate::rtti::call_func(&gg, player, &[]).map(|b| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[..16]);
                    o
                })
            });
            let gi = match gi {
                Some(gi) => gi,
                None => { log("[equiprawv2] PlayerPuppet.GetGame falhou"); return; }
            };
            let es_fn = register::get_function(reg, "BwmsPrepEquipSys");
            let req_fn = register::get_function(reg, "BwmsPrepEquipReq");
            if !crate::rtti::sane(es_fn) || !crate::rtti::sane(req_fn) {
                log("[equiprawv2] BwmsPrepEquipSys/Req não resolveram (mod compilado?)");
                return;
            }
            // FIX 2026-08-03 (achado ao vivo, 3 crashes): ret_type NÃO pode ser hardcoded null pra
            // funções que retornam ref<T> (só é correto pra Void) — ADDR_EXEC usa esse valor
            // DIRETO, um ret_type errado corrompe o processamento da chamada inteira, não só o
            // retorno. Ver rtti::ret_type_of.
            let es_rf = crate::rtti::ResolvedFn { func: es_fn, ret_type: crate::rtti::ret_type_of(es_fn), is_static: true };
            let req_rf = crate::rtti::ResolvedFn { func: req_fn, ret_type: crate::rtti::ret_type_of(req_fn), is_static: true };
            log("[equiprawv2] pré: chamando es_rf (1ª call_func desta invocação)...");
            let es_ret = crate::rtti::call_func(&es_rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            log(&format!("[equiprawv2] es_ret={:?} — chamando req_rf agora (2ª call_func)...", es_ret.is_some()));
            let req_ret = crate::rtti::call_func(&req_rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            log(&format!("[equiprawv2] req_ret={:?}", req_ret.is_some()));
            let es_ptr = es_ret.map(|r| u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]) as *mut c_void).unwrap_or(std::ptr::null_mut());
            let req_ptr = req_ret.map(|r| u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]) as *mut c_void).unwrap_or(std::ptr::null_mut());
            log(&format!("[equiprawv2] es_ptr={es_ptr:p} req_ptr={req_ptr:p} — 2 chamadas top-level, zero native aninhado"));
            if es_ptr.is_null() || req_ptr.is_null() || !crate::rtti::sane(es_ptr) || !crate::rtti::sane(req_ptr) {
                log("[equiprawv2] es_ptr/req_ptr inválido — abortando antes do transmute");
                return;
            }
            log("[equiprawv2] chamando 0x103b1f624 direto via transmute, DIRETO de run_cmd (zero aninhamento)");
            let fp = crate::rebase(0x1_03b1_f624u64);
            if fp.is_null() {
                log("[equiprawv2] sem mapa pra 0x103b1f624 neste build -> pulando (inerte)");
            } else {
                let f: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(fp);
                f(es_ptr, req_ptr);
                log("[equiprawv2] transmute retornou — zero crash até aqui!");
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
    if let Some(rest) = cmd.strip_prefix("factlookup") {
        let name = rest.trim();
        let name = if name.is_empty() { "bwms_test_weapon" } else { name };
        unsafe { crate::selftest::factlookup_test(name) };
        return;
    }
    if cmd == "factread" {
        unsafe { crate::selftest::factread() };
        return;
    }
    if cmd == "factread2" {
        unsafe { crate::selftest::factread2() };
        return;
    }
    if cmd == "factread3" {
        unsafe { crate::selftest::factread3() };
        return;
    }
    if cmd == "postload-probe" {
        unsafe { crate::selftest::postload_probe(&reg) };
        return;
    }
    if cmd == "factinject" {
        unsafe { crate::selftest::factinject() };
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
        ["staminacheck"] => return act("lido (ver log)", unsafe { console::staminacheck(reg, player) }),
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
        // Codeware `#120` — captura DINÂMICA de LR em `Red::InkSystem::Get()` (2026-08-14).
        // `inkgethook` instala o probe observe-only (gate `~/.bwms-hook-inkget-lr`); rode
        // `inkgetbaseline` em idle (~10-15s após o hook), depois `presskey <kc>` 3-5x, depois
        // `inkgetpost` — loga os call-sites que só aparecem depois do keypress.
        //
        // TIMING (fix desta sessão, mesma lição corrigida do `checkreshook`/`#198`/`#199`
        // acima): mande `inkgethook` IMEDIATAMENTE ao confirmar GAMEPLAY, durante a própria
        // rajada de smoke-tests do `OnGameAttached` (enquanto o executor ainda dispara) — a 5ª
        // tentativa desta linha confirmou `inkgethook`/`inkgetbaseline` funcionando quando
        // enviados assim, mas `presskey`+`inkgetpost` (mandados ~20-27s depois) nunca foram
        // processados: o executor já tinha parado de disparar (mesmo mecanismo do
        // `checkresbaseline`/`checkrespost`, canal MORTO fora da rajada mesmo com
        // `PHASE_REACHED_5=true`). `inkgetbaseline`/`inkgetpost` são 100% thread-safe (só
        // drenam `INKGET_RING`/`AtomicU64` + comparam contra `INKGET_BASELINE`/`Mutex<BTreeSet>`
        // + logam — ZERO chamada de VM/RTTI/callf, ZERO instalação de hook) — MOVIDAS pra
        // também rodar pela THREAD DO HEARTBEAT (sempre viva, independente do executor — ver
        // `lib.rs` ~526, bloco `["inkgetbaseline"]`/`["inkgetpost"]` no loop pós-boot). Ficam
        // AQUI TAMBÉM (fallback: se o executor ainda estiver ativo quando o comando chegar,
        // processa por aqui primeiro — resultado idêntico, ambos chamam a mesma função pura).
        // `inkgethook` (este braço) continua existindo como comando MANUAL do canal do executor
        // (útil pra reinstalar sob demanda/depurar), mas desde `cw-inkget-autoretry` (2026-08-16,
        // `on_load`, thread dedicada logo após a thread de heartbeat) NÃO é mais o único jeito de
        // instalar o probe: o dylib agora tenta sozinho, repetidamente (750ms, gate
        // `~/.bwms-hook-inkget-lr`), independente do executor estar vivo ou não — resolve
        // exatamente a fragilidade de timing descrita no parágrafo acima (janela do executor
        // curta e imprevisível). Este braço manual fica como fallback/atalho, nunca conflita com
        // o retry automático (mesma trava `INKGET_INSTALLED`, idempotente dos dois lados).
        //
        // `presskey`/`focusgame` (ver mais abaixo, ~7600+) DELIBERADAMENTE NÃO migram pro canal
        // do heartbeat: ao contrário de inkgetbaseline/inkgetpost (leitura pura), `presskey`
        // INJETA um evento de input REAL (CGEvent keyDown/keyUp) que tem efeito genuíno no jogo
        // vivo — categoria de risco diferente (mutação, não leitura). Rodar via heartbeat
        // tiraria a garantia do gate `PHASE_REACHED_5 && !exec_nested()` (o jogo poderia não
        // estar pronto pra receber input ainda) e chamaria `NSApplication.
        // activateIgnoringOtherApps:`/`CGEventPost` de uma thread Rust própria que nunca foi
        // confirmada como sendo a main-thread do processo (AppKit não garante thread-safety fora
        // dela) — risco novo, nunca testado, mesmo padrão de cautela do `checkreshook`.
        //
        // LIÇÃO DE TESTE (2026-08-14, achado ao testar esta migração ao vivo): o canal do
        // heartbeat (`lib.rs` ~490) lê o arquivo INTEIRO e faz `split_whitespace()` sobre o
        // CONTEÚDO TODO como se fosse UM comando só (`parts.as_slice()` == token array) — ao
        // contrário do canal do executor (`lib.rs` ~1494), que faz `content.lines()` e despacha
        // CADA linha separada via `run_cmd`. Um arquivo com múltiplos comandos por linha (ex.
        // `focusgame\ninkgethook\ninkgetbaseline\n`) NUNCA bate em nenhum braço de match do
        // heartbeat (vira um array de 3 tokens, não `["focusgame"]`) — fica só esperando o
        // executor (que pode já estar morto). **Pra confiar no canal do heartbeat, sempre
        // escrever/enviar UM comando por vez** (uma escrita = um comando, sem `\n` interno) —
        // nunca bater múltiplos comandos na mesma escrita achando que "funciona igual ao canal
        // do executor". Boot de verificação desta migração confirmou o `inkgetpost` sendo
        // processado com sucesso via heartbeat (`[hb-canal] inkgetpost...` no log) quando enviado
        // sozinho, mas um lote de 3 comandos (`focusgame`+`inkgethook`+`inkgetbaseline`) mandado
        // junto se perdeu sem nenhum dos dois canais processá-lo.
        ["inkgethook"] => {
            unsafe { crate::selftest::install_inkget_probe() };
            return;
        }
        ["inkgetbaseline"] => {
            crate::selftest::inkget_baseline();
            return;
        }
        ["inkgetpost"] => {
            crate::selftest::inkget_post();
            return;
        }
        // Codeware `#120` candidato `0x104a13088` (2026-08-17, achado por LR-tracing em `Get()` +
        // disassembly offline — ver `HISTORICO.md`/`CATALOGO-EXAUSTIVO-CODEWARE.md` item `#120`).
        // `ink120hook` instala sob demanda (independente do gate de boot `~/.bwms-hook-
        // ink120cand`); `ink120dump` lê o ring (x0/x1/x2/x3 por chamada); `ink120reset` zera pra
        // uma janela nova. Mesmo idioma do trio `inkgethook`/`inkgetbaseline`/`inkgetpost` acima.
        ["ink120hook"] => {
            unsafe { crate::selftest::install_ink120cand_probe() };
            return;
        }
        ["ink120dump"] => {
            crate::selftest::ink120cand_dump();
            return;
        }
        ["ink120reset"] => {
            crate::selftest::ink120cand_reset();
            return;
        }
        // Codeware `#198`/`#199` (`ResourceLoader::LoadAsync`/`LoadResource`) — captura DINÂMICA
        // de LR em `ResourceDepot::CheckResource` (2026-08-14, ancoragem alternativa ao
        // `InkSystem::Get()` do `#120`). `checkreshook` instala o probe observe-only (gate
        // `~/.bwms-hook-checkres-lr`); `CheckResource` dispara sozinho durante loading real —
        // não precisa de `presskey`. NUNCA rodar junto de `copytest` (mesmo endereço, hooks
        // conflitam).
        //
        // TIMING (CORRIGIDO 2026-08-14, 2ª rodada — invalida a lição da 1ª rodada, que dizia
        // "espere uma janela de silêncio pós-gameplay antes de mandar"): esse conselho estava
        // ERRADO. Achado ao vivo: o canal deste bloco só drena dentro de `cp77_tick()`, chamado
        // só como efeito colateral de `exec_replacement` (o hook do executor de script) disparar.
        // O executor PARA de disparar assim que a rajada inicial de smoke-tests do
        // `OnGameAttached` termina e o jogo fica "quieto" — nesse ponto o canal fica MORTO pro
        // resto do boot (`checkreshook` ficou intocado em `/tmp/cp77-cmd.txt` por 35s+ até o
        // fim do boot, mesmo com `PHASE_REACHED_5=true`). A janela de "silêncio" NÃO é o momento
        // certo pra mandar — é exatamente quando o canal para de ser servido. **Mande
        // `checkreshook` IMEDIATAMENTE ao confirmar GAMEPLAY (durante a própria rajada, enquanto
        // o executor ainda está disparando), sem ficar esperando quietude.** `checkresbaseline`/
        // `checkrespost` já foram movidos pra rodar também pela THREAD DO HEARTBEAT (sempre viva,
        // independente do executor — ver `lib.rs` ~495, bloco `["checkresbaseline"]`/
        // `["checkrespost"]` no loop pós-boot) — só `checkreshook` continua exigindo o executor
        // ativo (instala um `Interceptor::replace`, escreve código; mover isso pra outra thread é
        // categoria de risco não testada).
        ["checkreshook"] => {
            unsafe { crate::selftest::install_checkres_probe() };
            return;
        }
        ["checkresbaseline"] => {
            crate::selftest::checkres_baseline();
            return;
        }
        ["checkrespost"] => {
            crate::selftest::checkres_post();
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
        // RED4ext #475 (`CGameEngine::Get()`): a vtable REAL de `CGameEngine` foi identificada
        // com alta confiança offline (`0x10728d218`, cross-validada por `dyld_info -fixups` contra
        // o símbolo mangled `GetNativeTypeHash<CGameEngine>`) — mas o STORAGE do singleton nunca
        // foi achado (2 rodadas de RE offline esgotadas, ver `RED4EXT-461-VIA-CARA-PLANO.md`-style
        // achados no catálogo). Técnica NOVA (não tentada ainda): outros singletons do motor
        // (`ResourceGameDepot`@[0x109003000+0x1f8], `JournalManager`@0x10900b580) vivem NUM
        // CLUSTER DE PÁGINAS BSS conhecido (~0x109000000-0x109020000) — varre esse range procurando
        // um qword cujo PRÓPRIO deref bate com a vtable confirmada de `CGameEngine` (candidato a
        // singleton: `[slot]` = ponteiro pro objeto, `*[slot]` = vtable). 100% leitura, zero escrita,
        // zero risco novo — mesma primitiva `gum::is_readable` já usada em toda parte do projeto.
        // v2 (2026-08-11, mesma sessão): a 1ª versão (range fixo 128KB, `is_readable` por qword)
        // rodou limpo (zero crash) mas deu zero candidatos — precisa varrer mais memória, MAS sem
        // 1 syscall (`mach_vm_read_overwrite`) por qword (custo proibitivo em MB). Fix: lê em
        // BLOCOS de 64KB (`gum::read_chunk`, sem o cap de 512B do `is_readable`) — reduz de
        // milhões pra centenas de syscalls — e só faz o segundo `is_readable` (deref do
        // candidato) pra qwords que PARECEM ponteiro (não-zero, 8-alinhado, acima de 0x10000) —
        // ainda assim capado num teto absoluto de checagens pra nunca travar um frame.
        // RED4ext `#406`/`#408`/`#409` — caça o singleton do `ResourceLoader` pela assinatura
        // estrutural achada por string-xref em 2026-08-21 (ver `DATABASE.md`, seção ResourceLoader).
        // Read-only: só `mach_vm_read_overwrite`, zero escrita, zero hook.
        // Anda o mapa de tokens de um candidato achado pelo `findresloader`. Offset do mapa é
        // argumento porque a RE deixou 2 hipóteses em aberto (`+0x00` pelo SDK, `+0x20` pela página
        // de debug) — o dado decide qual.
        ["restokens", ptr, off, n] => {
            unsafe {
                if let (Ok(p), Ok(o), Ok(m)) = (
                    u64::from_str_radix(ptr.trim_start_matches("0x"), 16),
                    usize::from_str_radix(off.trim_start_matches("0x"), 16),
                    n.parse::<usize>(),
                ) {
                    crate::register::dump_res_loader_tokens(p, o, m);
                } else {
                    log("[restokens] uso: restokens <0xobj> <0xoffset-do-mapa> <max>");
                }
            }
            return;
        }
        // Busca DIRIGIDA: procura o ponteiro do loader dentro de um objeto conhecido (o scan de
        // globais foi eliminado por evidência; `#483` mostra que structs de recurso carregam
        // `ResourceLoader*` como CAMPO).
        ["loaderfrom", ptr] | ["loaderfrom", ptr, ..] => {
            unsafe {
                if let Ok(p) = u64::from_str_radix(ptr.trim_start_matches("0x"), 16) {
                    crate::register::find_loader_from(p, 0x400);
                } else {
                    log("[loaderfrom] uso: loaderfrom <0xobj>");
                }
            }
            return;
        }
        ["findresloader", rest @ ..] => {
            unsafe {
                let start = rest
                    .first()
                    .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                    .unwrap_or(0x1_06e0_0000);
                let mb = rest.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(16);
                let min_size = rest.get(2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(64);
                crate::register::find_res_loader(start, mb, min_size);
            }
            return;
        }
        ["findcgameengine", rest @ ..] => {
            unsafe {
                let target_vtbl = crate::rebase(0x10728d218) as u64;
                if !crate::gum::is_readable(target_vtbl as *const c_void, 8) {
                    log(&format!("[findcgameengine] vtable alvo {target_vtbl:#x} ILEGÍVEL — rebase pode estar errado, abortando"));
                    return;
                }
                let start_static: u64 = rest
                    .first()
                    .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                    .unwrap_or(0x1_0800_0000);
                let size_mb: u64 = rest.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(8).clamp(1, 64);
                let scan_start = crate::rebase(start_static) as usize;
                let scan_len = (size_mb * 1024 * 1024) as usize;
                let scan_end = scan_start + scan_len;
                log(&format!(
                    "[findcgameengine] v2 vtable alvo (rebased) = {target_vtbl:#x}; varrendo {scan_start:#x}..{scan_end:#x} ({size_mb}MB, chunked+heurístico)"
                ));
                const CHUNK: usize = 65536;
                const MAX_DEREF_CHECKS: u32 = 20000; // teto absoluto, nunca trava um frame
                let mut buf = vec![0u8; CHUNK];
                let mut found = 0u32;
                let mut readable_chunks = 0u32;
                let mut total_chunks = 0u32;
                let mut deref_checks = 0u32;
                let mut p = scan_start;
                'outer: while p < scan_end {
                    let this_len = CHUNK.min(scan_end - p);
                    total_chunks += 1;
                    if crate::gum::read_chunk(p, &mut buf[..this_len]) {
                        readable_chunks += 1;
                        let mut i = 0usize;
                        while i + 8 <= this_len {
                            let candidate = u64::from_le_bytes(buf[i..i + 8].try_into().unwrap());
                            if candidate > 0x10000 && candidate % 8 == 0 {
                                if deref_checks >= MAX_DEREF_CHECKS {
                                    log(&format!("[findcgameengine] teto de {MAX_DEREF_CHECKS} derefs atingido em {:#x} — parando cedo", p + i));
                                    break 'outer;
                                }
                                deref_checks += 1;
                                if crate::gum::is_readable(candidate as *const c_void, 8) {
                                    let vtbl = (candidate as *const u64).read_unaligned();
                                    if vtbl == target_vtbl {
                                        found += 1;
                                        log(&format!(
                                            "[findcgameengine] CANDIDATO ACHADO: slot={:#x} -> CGameEngine*={candidate:#x} vtbl={vtbl:#x}",
                                            p + i
                                        ));
                                        // Validação cruzada v2 (offset exato +0x308/+0x10 já REFUTADO numa
                                        // rodada anterior — `systemMap.size` deu 115 milhões, implausível).
                                        // Em vez de confiar num offset único, varre os PRÓPRIOS campos do
                                        // candidato (`0x00..0x350`, tamanho documentado de `CGameEngine`) por
                                        // QUALQUER ponteiro plausível ("framework candidato"), e pra cada um,
                                        // varre os primeiros campos DELE por outro ponteiro plausível
                                        // ("gameInstance candidato") cujo próprio +0x10 pareça o `size` do
                                        // `systemMap` (HashMap já confirmado pelo #472, size fica no offset
                                        // +0x08 dentro do HashMap, que começa em GameInstance+0x08 → +0x10
                                        // total) — plausível = inteiro pequeno (1..5000), não lixo/zero.
                                        let mut cand_off = 0usize;
                                        while cand_off < 0x350 {
                                            let f_addr = candidate + cand_off as u64;
                                            if crate::gum::is_readable(f_addr as *const c_void, 8) {
                                                let f_val = (f_addr as *const u64).read_unaligned();
                                                if f_val > 0x1000 && f_val % 8 == 0 && crate::gum::is_readable(f_val as *const c_void, 0x40) {
                                                    let mut sub_off = 0usize;
                                                    while sub_off < 0x40 {
                                                        let g_addr = f_val + sub_off as u64;
                                                        let g_val = (g_addr as *const u64).read_unaligned();
                                                        if g_val > 0x1000 && g_val % 8 == 0 && crate::gum::is_readable(g_val as *const c_void, 8) {
                                                            let sm_addr = g_val + 0x10;
                                                            if crate::gum::is_readable(sm_addr as *const c_void, 4) {
                                                                let sm_size = (sm_addr as *const u32).read_unaligned();
                                                                if sm_size >= 1 && sm_size <= 5000 {
                                                                    log(&format!(
                                                                        "[findcgameengine] validação v2: candidato+{cand_off:#x}={f_val:#x} (framework?) -> +{sub_off:#x}={g_val:#x} (gameInstance?) -> systemMap.size@+0x10={sm_size} PLAUSÍVEL"
                                                                    ));
                                                                }
                                                            }
                                                        }
                                                        sub_off += 8;
                                                    }
                                                }
                                            }
                                            cand_off += 8;
                                        }
                                        log("[findcgameengine] validação v2 completa (varredura de campos do candidato)");
                                    }
                                }
                            }
                            i += 8;
                        }
                    }
                    p += this_len;
                }
                log(&format!(
                    "[findcgameengine] fim: {total_chunks} chunks ({readable_chunks} legíveis), {deref_checks} derefs testados, {found} candidato(s)"
                ));
            }
            return;
        }
        // `findinksystembss` (2026-08-14, RED4ext #409/Codeware #100/#120) — varredura DIRETA
        // e LIMITADA (read-only) de uma região de memória procurando qualquer qword cujo deref
        // bate a "forma" estrutural de `Red::InkSystem` (ver `inksystem_shape_matches`).
        // Diferente do `findcgameengine` (vtable-match contra vtable JÁ CONHECIDA), aqui NÃO HÁ
        // vtable conhecida — `InkSystem::Instance` é `Core::RawPtr` (ponteiro GLOBAL cru, mesma
        // categoria de `ResourceGameDepot`/`JournalManager`), então o filtro é MULTI-CAMPO em vez
        // de vtable única. Região default = o mesmo cluster BSS já usado pra outros singletons
        // (`ResourceGameDepot`@[0x109003000+0x1f8]/`JournalManager`@0x10900b580), size default 4MB
        // (teto de segurança: clamp 1..16MB, nunca "o processo inteiro").
        ["findinksystembss", rest @ ..] => {
            unsafe {
                let start_static: u64 = rest
                    .first()
                    .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                    .unwrap_or(0x1_0900_0000);
                let size_mb: u64 = rest.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(4).clamp(1, 16);
                let scan_start = crate::rebase(start_static) as usize;
                let scan_len = (size_mb * 1024 * 1024) as usize;
                let scan_end = scan_start + scan_len;
                log(&format!(
                    "[findinksystembss] varrendo {scan_start:#x}..{scan_end:#x} ({size_mb}MB) por forma-InkSystem (sem vtable conhecida, filtro multi-campo)"
                ));
                const CHUNK: usize = 65536;
                const MAX_SHAPE_CHECKS: u32 = 20000; // teto absoluto, nunca trava um frame
                let mut buf = vec![0u8; CHUNK];
                let mut found = 0u32;
                let mut readable_chunks = 0u32;
                let mut total_chunks = 0u32;
                let mut shape_checks = 0u32;
                let mut p = scan_start;
                'outer: while p < scan_end {
                    let this_len = CHUNK.min(scan_end - p);
                    total_chunks += 1;
                    if crate::gum::read_chunk(p, &mut buf[..this_len]) {
                        readable_chunks += 1;
                        let mut i = 0usize;
                        while i + 8 <= this_len {
                            let candidate = u64::from_le_bytes(buf[i..i + 8].try_into().unwrap());
                            if candidate > 0x10000 && candidate % 8 == 0 {
                                if shape_checks >= MAX_SHAPE_CHECKS {
                                    log(&format!("[findinksystembss] teto de {MAX_SHAPE_CHECKS} checagens atingido em {:#x} — parando cedo", p + i));
                                    break 'outer;
                                }
                                shape_checks += 1;
                                if let Some((kb, cap, size)) = inksystem_shape_matches(candidate) {
                                    found += 1;
                                    log(&format!(
                                        "[findinksystembss] CANDIDATO: slot={:#x} -> InkSystem*={candidate:#x} keyboardState={kb:#06x} layerManagers.cap={cap} .size={size}",
                                        p + i
                                    ));
                                }
                            }
                            i += 8;
                        }
                    }
                    p += this_len;
                }
                log(&format!(
                    "[findinksystembss] fim: {total_chunks} chunks ({readable_chunks} legíveis), {shape_checks} formas testadas, {found} candidato(s)"
                ));
            }
            return;
        }
        // `walkinksystem <root_hex_runtime> [maxdepth] [maxvisit]` (2026-08-14) — 2º mecanismo,
        // INDEPENDENTE do scan de BSS acima: ANCORADO num ponteiro runtime JÁ CONHECIDO/vivo
        // (ex.: o `CGameEngine*` que `findcgameengine` já confirma achar, colado do log dele —
        // InkSystem é dono de subsistema de UI/render, plausivelmente alcançável por poucos
        // saltos de ponteiro a partir do engine). BFS bounded: lê os campos [0..span) de cada
        // objeto visitado, testa CADA qword pointer-shaped contra `inksystem_shape_matches`;
        // qwords que passam o pré-filtro (mas não a forma) viram nós do próximo nível, até
        // `maxdepth`. Teto de visitas total (`maxvisit`, default 4000) — nunca varredura
        // ilimitada. 100% leitura.
        ["walkinksystem", root, rest @ ..] => {
            let root_va = u64::from_str_radix(root.trim_start_matches("0x"), 16).unwrap_or(0);
            if root_va == 0 {
                return log("[walkinksystem] uso: walkinksystem <ptr-hex-runtime> [maxdepth] [maxvisit]");
            }
            let maxdepth: u32 = rest.first().and_then(|s| s.parse::<u32>().ok()).unwrap_or(2).clamp(1, 3);
            let maxvisit: u32 = rest.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(4000).clamp(1, 20000);
            const SPAN: usize = 0x400; // bytes por objeto visitado (cobre InkSystem+folga, CGameEngine 0x350)
            unsafe {
                log(&format!(
                    "[walkinksystem] raiz={root_va:#x} maxdepth={maxdepth} maxvisit={maxvisit} span={SPAN:#x}"
                ));
                let mut frontier: Vec<u64> = vec![root_va];
                let mut visited = std::collections::HashSet::new();
                let mut found = 0u32;
                let mut total_visits = 0u32;
                for depth in 0..maxdepth {
                    let mut next_frontier: Vec<u64> = Vec::new();
                    for &obj in &frontier {
                        if total_visits >= maxvisit {
                            break;
                        }
                        if !visited.insert(obj) {
                            continue;
                        }
                        total_visits += 1;
                        if !crate::gum::is_readable(obj as *const c_void, 8) {
                            continue;
                        }
                        let mut i = 0usize;
                        while i + 8 <= SPAN {
                            let addr = obj + i as u64;
                            if crate::gum::is_readable(addr as *const c_void, 8) {
                                let candidate = (addr as *const u64).read_unaligned();
                                if candidate > 0x10000 && candidate % 8 == 0 && candidate != obj {
                                    if let Some((kb, cap, size)) = inksystem_shape_matches(candidate) {
                                        found += 1;
                                        log(&format!(
                                            "[walkinksystem] CANDIDATO (depth={depth}): {obj:#x}+{i:#x} -> InkSystem*={candidate:#x} keyboardState={kb:#06x} layerManagers.cap={cap} .size={size}"
                                        ));
                                    } else if depth + 1 < maxdepth {
                                        next_frontier.push(candidate);
                                    }
                                }
                            }
                            i += 8;
                        }
                    }
                    frontier = next_frontier;
                    if frontier.is_empty() || total_visits >= maxvisit {
                        break;
                    }
                }
                log(&format!(
                    "[walkinksystem] fim: {total_visits} objetos visitados, {found} candidato(s)"
                ));
            }
            return;
        }
        // `inksystemvalidate <candidato-hex-runtime>` (2026-08-14, validação cruzada do
        // candidato achado por `walkinksystem`) — 2ª fonte INDEPENDENTE pra confirmar
        // `Red::InkSystem::Get()` (RED4ext #409/Codeware #100/#120), mesmo padrão que fechou
        // `CGameEngine::Get()` (2 mecanismos diferentes batendo exato não é coincidência).
        //
        // Mecanismo: `InkSystem::requestsHandler@0x370` é um `WeakHandle<ink::ISystemRequestsHandler>`
        // (16B: {instance*@+0, refCount*@+8} — layout confirmado em `RED4ext.SDK/Handle.hpp`,
        // `SharedPtrBase<T>{instance@0x00,refCount@0x08}`, MESMO layout que `WeakHandle` herda via
        // `WeakPtrWithAccess`). `BwmsGetSystemRequestsHandler()` (global 100% redscript, já
        // deployada desde 2026-08-10 — `codeware-casts.reds`, item #101 — `new
        // inkMenuScenario().GetSystemRequestsHandler()`, native VANILLA real) devolve um
        // `wref<inkISystemRequestsHandler>` — MESMO tipo de WeakHandle, MESMA representação.
        // Se `candidato+0x370` bate byte-a-byte contra o retorno dessa chamada, feita no MESMO
        // instante do MESMO boot, são 2 fontes independentes (leitura crua de campo vs. despacho
        // RTTI/bytecode real através do motor) confirmando a MESMA identidade de objeto.
        ["inksystemvalidate", candidate_hex] => {
            let candidate = u64::from_str_radix(candidate_hex.trim_start_matches("0x"), 16).unwrap_or(0);
            if candidate == 0 {
                return log("[inksystemvalidate] uso: inksystemvalidate <InkSystem*-hex-runtime>");
            }
            unsafe {
                // 1) a forma básica ainda bate (mesmo filtro multi-campo do walkinksystem)?
                let shape = inksystem_shape_matches(candidate);
                let (kb, cap, size) = match shape {
                    Some(t) => t,
                    None => {
                        return log(&format!(
                            "[inksystemvalidate] candidato {candidate:#x} NÃO bate a forma InkSystem — abortando"
                        ));
                    }
                };
                // 2) lê candidato+0x370 (WeakHandle<ink::ISystemRequestsHandler>) DIRETO da memória.
                let f_reqhandler = candidate + 0x370;
                if !crate::gum::is_readable(f_reqhandler as *const c_void, 16) {
                    return log(&format!("[inksystemvalidate] candidato={candidate:#x}+0x370 ilegível"));
                }
                let cand_instance = (f_reqhandler as *const u64).read_unaligned();
                let cand_refcount = (f_reqhandler as *const u64).add(1).read_unaligned();

                // 3) chama BwmsGetSystemRequestsHandler() — 2ª fonte, MESMO instante, MESMO boot.
                let f = register::get_function(reg, "BwmsGetSystemRequestsHandler");
                if !crate::rtti::sane(f) {
                    return log(
                        "[inksystemvalidate] BwmsGetSystemRequestsHandler não resolveu (declaração ausente do bundle deployado?)",
                    );
                }
                let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                let res = match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[]) {
                    Some(r) => r,
                    None => {
                        return log("[inksystemvalidate] BwmsGetSystemRequestsHandler() não completou");
                    }
                };
                let handler_instance = u64::from_le_bytes(res[0..8].try_into().unwrap());
                let handler_refcount = u64::from_le_bytes(res[8..16].try_into().unwrap());

                let inst_match = cand_instance != 0 && cand_instance == handler_instance;
                let rc_match = cand_refcount != 0 && cand_refcount == handler_refcount;
                log(&format!(
                    "[inksystemvalidate] candidato={candidate:#x} (keyboardState={kb:#06x} layerManagers.cap={cap} .size={size}) | +0x370.instance={cand_instance:#x} +0x370.refCount={cand_refcount:#x} | BwmsGetSystemRequestsHandler()->instance={handler_instance:#x} .refCount={handler_refcount:#x} | instance_match={inst_match} refcount_match={rc_match}"
                ));
                if inst_match {
                    log("[inksystemvalidate] >>> MATCH EXATO (instance) — candidato CONFIRMADO como Red::InkSystem::Get() <<<");
                } else {
                    log("[inksystemvalidate] >>> SEM MATCH — candidato NÃO confirmado por esta via <<<");
                }
            }
            return;
        }
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
        // vtdump <hex_ptr> [n] — irmão do `peekq`, MAS pra ponteiro RUNTIME já resolvido (ex.:
        // vtable de um objeto vivo, lido via `rd_u64(obj)`) — NÃO faz `rebase()` (que espera um
        // vmaddr ESTÁTICO de arquivo; aplicar rebase de novo num ponteiro já pós-ASLR dava
        // endereço errado). Read-only, nunca chama nada. count clampado 1..48 (cobre vtables
        // maiores, ex. `TweakDB::FlatValue` tem 31 slots — ver `RED4ext.SDK/TweakDB.hpp`).
        ["vtdump", addr, rest @ ..] => {
            let n = rest.first().and_then(|s| s.parse::<usize>().ok()).unwrap_or(8).clamp(1, 48);
            let va = u64::from_str_radix(addr.trim_start_matches("0x"), 16).unwrap_or(0);
            if va == 0 {
                return log("[vtdump] uso: vtdump <ptr-hex-runtime> [count]");
            }
            unsafe {
                let base = va as *const u8;
                if !crate::gum::is_readable(base as *const c_void, n * 8) {
                    return log(&format!("[vtdump] {va:#x} ILEGÍVEL"));
                }
                let mut out = format!("[vtdump] {va:#x}:");
                let mut prev: Option<u64> = None;
                let mut run_start = 0usize;
                for i in 0..n {
                    let q = (base.add(i * 8) as *const u64).read_unaligned();
                    let same = prev == Some(q);
                    out.push_str(&format!(" +{:#x}={q:#018x}{}", i * 8, if same { "=" } else { "" }));
                    if !same {
                        run_start = i;
                    }
                    prev = Some(q);
                    let _ = run_start;
                }
                return log(&out);
            }
        }
        // `flatvtable <nome>` — resolve um FlatValue REAL do TweakDB vivo (mesma via de `getflat`)
        // e dumpa os 31 primeiros qwords da SUA vtable (não do FlatValue em si — do que
        // `rd_u64(fv)` aponta). Serve pra confirmar/refutar ao vivo a hipótese do layout de
        // `TweakDB::FlatValue` mapeado por leitura de `TweakDB.hpp` (2 dtors Itanium + 26
        // `GetValueOffset_*` com corpo idêntico, prováveis alvo de ICF do compilador -> mesmo
        // ponteiro repetido — + `GetValue`/`GetTypeName`/`GetDataPtr` nos últimos 3 slots,
        // offsets 0x00-0xF0). TweakXL `ScriptInterface`/`GetFlat->Variant` (#43, PENDENCIAS-
        // UNIFICADAS.md) depende de confirmar isso ANTES de chamar `GetTypeName` às cegas.
        ["flatvtable", name] => {
            unsafe {
                let t = match crate::tweakdb_rt::singleton() {
                    Some(s) => s,
                    None => return log("[flatvtable] singleton indisponível"),
                };
                match crate::tweakdb_rt::get_flat_value(t, name) {
                    None => log(&format!("[flatvtable] '{name}' não achado")),
                    Some(fv) => {
                        let vt = (fv as *const u64).read_unaligned();
                        log(&format!("[flatvtable] '{name}' FlatValue={fv:p} vtable={vt:#x}"));
                        if !crate::gum::is_readable(vt as *const c_void, 31 * 8) {
                            return log("[flatvtable] vtable ILEGÍVEL");
                        }
                        let mut out = format!("[flatvtable] vtable dump ({vt:#x}):");
                        for i in 0..31usize {
                            let q = (vt as *const u8).add(i * 8) as *const u64;
                            out.push_str(&format!(" +{:#x}={:#018x}", i * 8, q.read_unaligned()));
                        }
                        log(&out);
                    }
                }
            }
            return;
        }
        // `flatgettype <nome> <gtn|gdp>` (2026-08-10) — TweakXL `ScriptInterface.GetFlat->Variant`
        // (#43). **ACHADO DE RISCO REAL (1ª tentativa, bundlando as 2 chamadas numa função só):
        // TRAVOU o game thread (sem crash — zero .ips, zero ProblemReporter — command channel e
        // heartbeat pararam de vez, só recuperou via SIGTERM). NÃO chamar as 2 juntas de novo.**
        // Refatorado pra isolar QUAL das 2 trava (mesma técnica que resolveu o mistério de crash
        // do garment em 2026-07-29 — 1 hook por boot, nunca bundlar 2 chamadas não-confirmadas):
        // `gtn` chama SÓ `GetTypeName(CName*)` (vtable+0xE8); `gdp` chama SÓ `GetDataPtr()`
        // (vtable+0xF0). Gated atrás de `~/.bwms-flatgettype-danger` — sem o marcador, só loga o
        // endereço resolvido (read-only) sem chamar nada. O layout da vtable em si (dump
        // comparativo, `flatvtable`) está CONFIRMADO e é seguro — o risco é especificamente
        // CHAMAR o ponteiro resolvido.
        ["flatgettype", name, which] => {
            unsafe {
                let home = std::env::var("HOME").unwrap_or_default();
                let danger = std::path::Path::new(&format!("{home}/.bwms-flatgettype-danger")).exists();
                let t = match crate::tweakdb_rt::singleton() {
                    Some(s) => s,
                    None => return log("[flatgettype] singleton indisponível"),
                };
                let fv = match crate::tweakdb_rt::get_flat_value(t, name) {
                    Some(fv) => fv,
                    None => return log(&format!("[flatgettype] '{name}' não achado")),
                };
                let vt = (fv as *const u64).read_unaligned();
                if !crate::gum::is_readable(vt as *const c_void, 0xF8) {
                    return log("[flatgettype] vtable ILEGÍVEL");
                }
                let off: usize = match *which {
                    "gtn" => 0xE8,
                    "gdp" => 0xF0,
                    _ => return log("[flatgettype] uso: flatgettype <nome> <gtn|gdp>"),
                };
                let func = ((vt as *const u8).add(off) as *const u64).read_unaligned();
                if !crate::rtti::sane(func as *mut c_void) {
                    return log("[flatgettype] endereço de método implausível -> abortado");
                }
                if !danger {
                    return log(&format!(
                        "[flatgettype] '{name}' {which}={func:#x} (READ-ONLY — sem ~/.bwms-flatgettype-danger, NÃO chama; ver risco de hang documentado acima)"
                    ));
                }
                log(&format!("[flatgettype] '{name}' {which}={func:#x} CHAMANDO (danger armado)..."));
                match *which {
                    "gtn" => {
                        let mut name_out: u64 = 0;
                        let f: extern "C" fn(*mut c_void, *mut u64) -> *mut u64 = std::mem::transmute(func);
                        let ret = f(fv as *mut c_void, &mut name_out as *mut u64);
                        let type_name = crate::cname::resolve_cname(name_out);
                        log(&format!(
                            "[flatgettype] gtn OK -> CName {name_out:#x} ('{type_name}') ret_ptr={ret:p}"
                        ));
                    }
                    "gdp" => {
                        let f: extern "C" fn(*mut c_void) -> *mut c_void = std::mem::transmute(func);
                        let data_ptr = f(fv as *mut c_void);
                        log(&format!("[flatgettype] gdp OK -> {data_ptr:p} (esperado fv+0x08={:p})", fv.add(0x08)));
                    }
                    _ => unreachable!(),
                }
            }
            return;
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
        // `getrecords <classe> [limite]` (2026-08-10) — ArchiveXL #59, `GetRecords<R>`: lista
        // TODOS os records de um TIPO enumerando `recordsByID` (read-only).
        ["getrecords", class] => {
            unsafe { crate::tweakdb_rt::getrecords_cmd(reg, class, 500_000) };
            return;
        }
        ["getrecords", class, limit] => {
            let n: usize = limit.parse().unwrap_or(500_000);
            unsafe { crate::tweakdb_rt::getrecords_cmd(reg, class, n) };
            return;
        }
        // `getrecbytype <classe> [limite]` (2026-08-10) — RED4ext.SDK #341, `GetRecordsByType`:
        // lookup DIRETO no índice `recordsByType`@+0x88 (O(1) amortizado), cross-validado contra
        // `getrecords` (scan completo filtrado) — mesma classe deve dar a MESMA contagem.
        ["getrecbytype", class] => {
            unsafe { crate::tweakdb_rt::getrecbytype_cmd(reg, class, 500_000) };
            return;
        }
        // `mutexsharedtest` (2026-08-10) — RED4ext.SDK #429, `SharedSpinLock::LockShared`/
        // `UnlockShared`: prova o par contra o lock real do TweakDB vivo (t+0x20). Restaura o
        // byte do lock a 0 (livre) ao final — nenhum efeito colateral persistente.
        ["mutexsharedtest"] => {
            unsafe {
                match crate::tweakdb_rt::singleton() {
                    Some(t) => crate::tweakdb_rt::mutex_shared_test_cmd(t),
                    None => log("[mutexsharedtest] singleton TweakDB indisponível"),
                }
            }
            return;
        }
        // `dynarrtest` (2026-08-11) — RED4ext.SDK #284/#280, `DynArray<T>::Erase`/`RemoveAt`:
        // buffer 100% NOSSO (nunca exposto ao motor), zero risco de crash de teardown.
        ["dynarrtest"] => {
            unsafe { crate::rtti::dynarray_removeat_selftest(); }
            return;
        }
        // `garmentoffsets` (2026-08-11) — ArchiveXL #97, `Facade.EnableGarmentOffsets`/
        // `DisableGarmentOffsets`: diagnóstico read-only do flag (a real API do ArchiveXL não
        // tem getter — só existe pra provar o round-trip Enable/Disable ao vivo).
        ["garmentoffsets"] => {
            log(&format!("[garmentoffsets] enabled={}", crate::register::garment_offsets_enabled()));
            return;
        }
        // `innertype <typeName>` (2026-08-10) — Codeware #197, `ReflectionType.GetInnerType()`:
        // resolve um IType composto (array/handle/weak-handle) por nome e mostra o tipo interno.
        ["innertype", type_name] => {
            unsafe { crate::rtti::probe_inner_type(reg, type_name) };
            return;
        }
        ["getrecbytype", class, limit] => {
            let n: usize = limit.parse().unwrap_or(500_000);
            unsafe { crate::tweakdb_rt::getrecbytype_cmd(reg, class, n) };
            return;
        }
        // `createalias <record_id_hex> <alias_id_hex>` (2026-08-10) — ArchiveXL #59,
        // `CreateRecordAlias`: cria uma entrada nova em `recordsByID` (via `CreateTDBRecord`,
        // growth-safe) apontando pro MESMO instance do record fonte.
        ["createalias", rid, aid] => {
            unsafe {
                let Some(t) = crate::tweakdb_rt::singleton() else {
                    return log("[createalias] singleton indisponível");
                };
                let record_id = u64::from_str_radix(rid.trim_start_matches("0x"), 16).unwrap_or(0);
                let alias_id = u64::from_str_radix(aid.trim_start_matches("0x"), 16).unwrap_or(0);
                let ok = crate::tweakdb_rt::create_record_alias_by_id(t, reg, record_id, alias_id);
                log(&format!("[createalias] record={record_id:#x} alias={alias_id:#x} -> {ok}"));
            }
            return;
        }
        // `mkvariant <tipo> <hex_value>` (2026-08-10) — RED4ext.SDK #316: constrói um Variant
        // NOVO (`build_variant_inline`) e lê de volta com o decoder JÁ PROVADO
        // (`variant_type_cname`+`variant_inline_u64`) — round-trip auto-contido, nunca sai do
        // nosso processo (não é passado ao motor/redscript nesta prova).
        ["mkvariant", ty, hex_val] => {
            unsafe {
                let v = u64::from_str_radix(hex_val.trim_start_matches("0x"), 16).unwrap_or(0);
                match crate::rtti::build_variant_inline(reg, ty, v) {
                    None => log(&format!("[mkvariant] tipo '{ty}' não resolveu (get_type falhou)")),
                    Some(buf) => {
                        let type_hash = crate::rtti::variant_type_cname(&buf);
                        let type_name = crate::cname::resolve_cname(type_hash);
                        let readback = crate::rtti::variant_inline_u64(&buf);
                        log(&format!(
                            "[mkvariant] construído tipo='{ty}' valor={v:#x} -> lido de volta: tipo='{type_name}' ({type_hash:#x}) valor={readback:#x} (round-trip {})",
                            if type_name == *ty && readback == v { "OK" } else { "DIVERGIU" }
                        ));
                    }
                }
            }
            return;
        }
        // `resexists <path>` (2026-08-10) — Codeware `#16`/`#198`: testa `resource_exists`
        // contra um path CONHECIDO (deve existir) e um INVENTADO (não deve).
        ["resexists", path] => {
            unsafe {
                let r = resource_exists(path);
                log(&format!("[resexists] '{path}' -> {r}"));
            }
            return;
        }
        // `archiveexists <nome>` (2026-08-10) — Codeware `#16`/`#198`: testa `archive_exists_by_name`
        // contra um nome CONHECIDO (deve existir, ex. basegame_1_engine.archive) e um INVENTADO.
        ["archiveexists", name] => {
            unsafe {
                let r = archive_exists_by_name(name);
                log(&format!("[archiveexists] '{name}' -> {r}"));
            }
            return;
        }
        // `testunload` (2026-08-10) — RED4ext.SDK #45: dispara `unload_all_plugins()` DIRETO
        // (sem passar por `exit_replacement`/exit real — achado ao vivo: SIGTERM não roda o
        // `exit()` hookado do libc, só um quit real via UI faria; testar a via de PROCESSO real
        // exigiria fechar o jogo pela UI, fora do alcance automatizado). Prova a peça central do
        // mecanismo (o dlsym+chamada do `bwms_plugin_unload` de cada plugin) sem depender do
        // caminho de exit — a integração em `exit_replacement` é 1 linha trivial já adicionada,
        // mesma categoria de risco de qualquer outra chamada já feita ali (Session/End etc.).
        ["testunload"] => {
            crate::plugins::unload_all_plugins();
            log("[testunload] unload_all_plugins() chamado diretamente (fora do exit real)");
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
        // `archivenamedump [n]` (2026-08-10) — Codeware #16/#198 (`ArchiveExists`): DIAGNÓSTICO
        // read-only, NÃO implementa a native ainda. Uma nota de RE histórica
        // (`notes/RE-archiveinfo-inject-2026-07-17.md`) flagou que o layout ESTÁTICO original de
        // `ArchiveInfo` (name/path RedString@+0x10..+0x30, stride 0x50) NÃO bateu contra o depot
        // VIVO numa tentativa anterior (2026-07-17) — só os offsets de TOPO do grupo
        // (`depot+0x10`/`+0x1c`, usados por `find_pathb_content_group`) foram reconfirmados desde
        // então (via `RegisterArchive`/`RegisterDir`, fechados 2026-08-09, que só manipulam
        // CONTAGEM, nunca leem nome). Este comando reusa `find_pathb_content_group` (confirmado) +
        // tenta ler `red_string_read` no offset suspeito de cada ArchiveInfo — sem escrever nada,
        // só pra CONFIRMAR ou REFUTAR o layout de nome antes de implementar `ArchiveExists` de verdade.
        ["archivenamedump"] | ["archivenamedump", _] => {
            let n: usize = match parts.get(1) {
                Some(s) => s.parse().unwrap_or(5),
                None => 5,
            };
            unsafe {
                let depot = PATHB_DEPOT.load(Ordering::Relaxed);
                if depot.is_null() {
                    return log("[archivenamedump] PATHB_DEPOT ainda não capturado (boot incompleto?)");
                }
                let Some(g) = find_pathb_content_group(depot as *mut u8) else {
                    return log("[archivenamedump] find_pathb_content_group falhou");
                };
                let cnt = (g.add(0x0c) as *const u32).read();
                let arr = (g as *const *const u8).read();
                log(&format!("[archivenamedump] grupo @{g:p}: cnt={cnt} arr={arr:p} — dumpando {n} entries (stride 0x50 suposto)"));
                if arr.is_null() || cnt == 0 {
                    return log("[archivenamedump] array vazio/nulo");
                }
                for i in 0..(n.min(cnt as usize)) {
                    let entry = arr.add(i * 0x50);
                    if !crate::gum::is_readable(entry as *const c_void, 0x50) {
                        log(&format!("[archivenamedump]   [{i:02}] entry@{entry:p} ILEGÍVEL"));
                        continue;
                    }
                    let raw16: Vec<u8> = (0..0x10).map(|o| *entry.add(o)).collect();
                    let s = crate::rtti::red_string_read(entry.add(0x10));
                    log(&format!(
                        "[archivenamedump]   [{i:02}] entry@{entry:p} bytes[0..0x10]={raw16:02x?} red_string@+0x10='{s}'"
                    ));
                }
            }
            return;
        }
        // `archivegroupdump` (2026-08-17, auditoria de consistência — ArchiveXL `#37`,
        // `ResolveArchiveGroup`) — DIAGNÓSTICO read-only: lista TODOS os grupos do depot com
        // basePath (offset +0x10) e scope (offset +0x30), decodificados via o header oficial
        // vendorizado. Confirma (ou refuta) os 2 offsets novos antes de usar `resolve_archive_
        // _group_by_path` de verdade em qualquer lugar. Zero escrita, zero mutação.
        // 2026-08-16/17 (rodada 41): núcleo movido pra `run_archivegroupdump()` (lib.rs, junto de
        // `archive_scope_name`) — este braço fica como fallback do canal gated-por-executor (útil
        // se o executor ainda estiver ativo quando o comando chegar); o braço PRINCIPAL agora é o
        // do `[hb-canal]` (loop pós-boot, thread do heartbeat, sempre viva) — ver logo abaixo.
        ["archivegroupdump"] => {
            unsafe { run_archivegroupdump() };
            return;
        }
        // `archivegroupresolve <basePath>` (2026-08-17, ArchiveXL `#37`) — testa
        // `resolve_archive_group_by_path` contra um basePath real (achado via `archivegroupdump`
        // primeiro) e um inventado. Read-only, zero mutação.
        ["archivegroupresolve", base_path] => {
            unsafe {
                let depot = PATHB_DEPOT.load(Ordering::Relaxed);
                if depot.is_null() {
                    return log("[archivegroupresolve] PATHB_DEPOT ainda não capturado (boot incompleto?)");
                }
                match resolve_archive_group_by_path(depot as *mut u8, base_path) {
                    Some(g) => log(&format!("[archivegroupresolve] '{base_path}' -> grupo @{g:p}")),
                    None => log(&format!("[archivegroupresolve] '{base_path}' -> nenhum grupo bate (esperado se for path novo)")),
                }
            }
            return;
        }
        // `archivegroupcreate <basePath>` (2026-08-17, ArchiveXL `#37`, metade de ESCRITA) —
        // testa `create_archive_group_by_path` (grow+insert real do `depot->groups`, mutação de
        // memória viva do motor). GATED ~/.bwms-flatwrite (mesma trava de `mkarr`/`mkflat`/`clone`
        // — categoria de risco "muta array vivo do engine", não exclusiva de TweakDB). Rode
        // `archivegroupdump` ANTES e DEPOIS pra confirmar visualmente: grupo novo no índice certo,
        // scope=Mod, count total +1, e os grupos vizinhos intactos (mesmos basePath/scope de antes).
        ["archivegroupcreate", base_path] => {
            let on = std::env::var("HOME")
                .ok()
                .map(|h| std::path::Path::new(&h).join(".bwms-flatwrite").exists())
                .unwrap_or(false);
            if !on {
                return log("[archivegroupcreate] BLOQUEADO: crie ~/.bwms-flatwrite p/ habilitar");
            }
            unsafe {
                let depot = PATHB_DEPOT.load(Ordering::Relaxed);
                if depot.is_null() {
                    return log("[archivegroupcreate] PATHB_DEPOT ainda não capturado (boot incompleto?)");
                }
                match create_archive_group_by_path(depot as *mut u8, base_path) {
                    Some(g) => log(&format!("[archivegroupcreate] '{base_path}' -> grupo @{g:p} (ok, ver archivegroupdump p/ confirmar)")),
                    None => log(&format!("[archivegroupcreate] '{base_path}' -> falhou (ver log anterior pro motivo)")),
                }
            }
            return;
        }
        // axl-journal-apply: dump JournalManager entries array at runtime.
        // jmdump — reads JM singleton + entries DynArray; safe (read-only).
        ["jmdump"] => {
            unsafe {
                use crate::selftest::JOURNAL_MANAGER_GLOBAL_VM;
                let jm_global = crate::rebase(JOURNAL_MANAGER_GLOBAL_VM) as *const *mut c_void;
                if !crate::gum::is_readable(jm_global as *const c_void, 8) {
                    return log("[jmdump] JM global not readable");
                }
                let jm_ptr = jm_global.read_unaligned();
                if jm_ptr.is_null() {
                    return log("[jmdump] JM global is null — not loaded yet");
                }
                log(&format!("[jmdump] JM = {jm_ptr:p}"));
                if !crate::gum::is_readable(jm_ptr as *const c_void, 0x40) {
                    return log("[jmdump] JM not readable");
                }
                let r64 = |off: usize| (jm_ptr.cast::<u8>().add(off) as *const u64).read_unaligned();
                let r32 = |off: usize| (jm_ptr.cast::<u8>().add(off) as *const u32).read_unaligned();
                // DynArray at +0x30: ptr(8)+cap(4)+sz(4)
                let arr_buf = r64(0x30) as *const u64;
                let arr_cap = r32(0x38);
                let arr_sz  = r32(0x3c);
                log(&format!("[jmdump] entries[+0x30]: ptr={arr_buf:p} cap={arr_cap} sz={arr_sz}"));
                if arr_buf.is_null() || arr_sz == 0 { return; }
                // Read first 8 elements as 8-byte-stride pointers, log vtable of each
                let n = (arr_sz as usize).min(8);
                for i in 0..n {
                    let slot = arr_buf.add(i);
                    if !crate::gum::is_readable(slot as *const c_void, 8) { break; }
                    let elem = slot.read_unaligned() as *const u64;
                    let vtbl = if !elem.is_null() && crate::gum::is_readable(elem as *const c_void, 16) {
                        elem.read_unaligned()
                    } else { 0 };
                    let vtbl2 = if !elem.is_null() && crate::gum::is_readable(elem as *const c_void, 16) {
                        elem.add(1).read_unaligned()
                    } else { 0 };
                    log(&format!("[jmdump] [{i}] obj={elem:p} +0={vtbl:#018x} +8={vtbl2:#018x}"));
                }
                // Also try 16-byte stride (Handle<T>)
                log("[jmdump] -- 16-byte stride (Handle<T>) --");
                let arr_buf16 = arr_buf as *const u8;
                for i in 0..n {
                    let slot_ptr = arr_buf16.add(i * 16) as *const u64;
                    if !crate::gum::is_readable(slot_ptr as *const c_void, 16) { break; }
                    let ptr_val = slot_ptr.read_unaligned() as *const u64;
                    let rc_val  = slot_ptr.add(1).read_unaligned();
                    let vtbl = if !ptr_val.is_null() && crate::gum::is_readable(ptr_val as *const c_void, 8) {
                        ptr_val.read_unaligned()
                    } else { 0 };
                    log(&format!("[jmdump] [{i}] ptr={ptr_val:p} rc={rc_val:#018x} vtbl={vtbl:#018x}"));
                }
            }
            return;
        }
        // axl-journal-apply WRITE PROOF: duplicate elem[0] at pos sz, increment sz.
        // jmprove — proves DynArray is R/W and sz can grow; safe at gameplay (no concurrent load).
        ["jmprove"] => {
            unsafe {
                use crate::selftest::JOURNAL_MANAGER_GLOBAL_VM;
                let jm_global = crate::rebase(JOURNAL_MANAGER_GLOBAL_VM) as *const *mut c_void;
                if !crate::gum::is_readable(jm_global as *const c_void, 8) {
                    return log("[jmprove] JM global not readable");
                }
                let jm_ptr = jm_global.read_unaligned() as *mut u8;
                if jm_ptr.is_null() { return log("[jmprove] JM null"); }
                if !crate::gum::is_readable(jm_ptr as *const c_void, 0x40) {
                    return log("[jmprove] JM not readable");
                }
                let r64 = |off: usize| (jm_ptr.add(off) as *const u64).read_unaligned();
                let r32 = |off: usize| (jm_ptr.add(off) as *const u32).read_unaligned();
                let arr_buf = r64(0x30) as *mut u8;
                let cap = r32(0x38);
                let sz  = r32(0x3c);
                log(&format!("[jmprove] before: ptr={arr_buf:p} cap={cap} sz={sz}"));
                if arr_buf.is_null() || sz == 0 || sz >= cap {
                    return log(&format!("[jmprove] abort: null={} sz={sz} cap={cap}", arr_buf.is_null()));
                }
                if !crate::gum::is_readable(arr_buf as *const c_void, 16) {
                    return log("[jmprove] arr[0] not readable");
                }
                // Element stride = 16 (Handle<T>: obj_ptr + rc_block_ptr)
                let elem0_ptr = (arr_buf as *const u64).read_unaligned();
                let elem0_rc  = (arr_buf.add(8) as *const u64).read_unaligned();
                log(&format!("[jmprove] elem[0]: ptr=0x{elem0_ptr:016x} rc=0x{elem0_rc:016x}"));
                // Write duplicate of elem[0] at slot sz
                let inject = arr_buf.add((sz as usize) * 16);
                if !crate::gum::is_readable(inject as *const c_void, 16) {
                    return log("[jmprove] inject slot not readable");
                }
                (inject as *mut u64).write_unaligned(elem0_ptr);
                (inject.add(8) as *mut u64).write_unaligned(elem0_rc);
                // Increment sz in DynArray
                (jm_ptr.add(0x3c) as *mut u32).write_unaligned(sz + 1);
                let sz_after = r32(0x3c);
                log(&format!("[jmprove] INJECT PROOF: sz {sz}→{sz_after} ✓"));
            }
            return;
        }
        // ArchiveXL #77 (JournalManager.hpp, 2026-08-11): dump read-only dos 6 slots de vtable
        // nunca explorados (GetTrackedQuest/GetTrackedPointOfInterest/GetEntryByHash/GetEntryHash/
        // TrackQuestByPath/TrackPointOfInterest). Windows offsets do header vendorizado (RawVFunc):
        // 0x1F8/0x208/0x220/0x230/0x298/0x2A0. Convenção +0x08 Itanium já confirmada NESTA MESMA
        // família de classe (gameIJournalManager é irmã de questIQuestsSystem, onde OnGameRestored
        // Windows 0x150->Mac 0x158 já foi confirmado via getquestsys) aplicada aqui como candidato.
        // Zero chamada — só lê o ponteiro de função + os primeiros 16 bytes crus (prólogo) pra
        // julgar plausibilidade, mesmo padrão do diagnóstico ForceStartNode/#55.
        ["journalvtdump"] => {
            unsafe {
                use crate::selftest::JOURNAL_MANAGER_GLOBAL_VM;
                let jm_global = crate::rebase(JOURNAL_MANAGER_GLOBAL_VM) as *const *mut c_void;
                if !crate::gum::is_readable(jm_global as *const c_void, 8) {
                    return log("[journalvtdump] JM global not readable");
                }
                let jm_ptr = jm_global.read_unaligned();
                if jm_ptr.is_null() {
                    return log("[journalvtdump] JM global is null — not loaded yet");
                }
                log(&format!("[journalvtdump] JM = {jm_ptr:p}"));
                if !crate::gum::is_readable(jm_ptr as *const c_void, 8) {
                    return log("[journalvtdump] JM not readable");
                }
                let vt = (jm_ptr as *const u64).read_unaligned() as *const u8;
                log(&format!("[journalvtdump] vtable = {vt:p}"));
                let base = crate::game_base();
                for (label, win_off) in [
                    ("GetTrackedQuest", 0x1F8usize),
                    ("GetTrackedPointOfInterest", 0x208usize),
                    ("GetEntryByHash", 0x220usize),
                    ("GetEntryHash", 0x230usize),
                    ("TrackQuestByPath", 0x298usize),
                    ("TrackPointOfInterest", 0x2A0usize),
                ] {
                    let mac_off = win_off + 0x08;
                    let slot_ptr = vt.add(mac_off) as *const u64;
                    if !crate::gum::is_readable(slot_ptr as *const c_void, 8) {
                        log(&format!("[journalvtdump] vtbl+{mac_off:#x} ({label}, win={win_off:#x}) slot ilegível"));
                        continue;
                    }
                    let fn_rt = slot_ptr.read_unaligned() as usize;
                    let vmaddr = if fn_rt > base { fn_rt - base + 0x1_0000_0000 } else { fn_rt };
                    let mut bytes16 = [0u8; 16];
                    let mut prologue_ok = false;
                    if crate::gum::is_readable(fn_rt as *const c_void, 16) {
                        std::ptr::copy_nonoverlapping(fn_rt as *const u8, bytes16.as_mut_ptr(), 16);
                        // Prólogo ARM64 comum: stp x29,x30,[sp,#-N]! (0xa9 no 4º byte de um par
                        // stp de 32-bit, forma comum 0xa9bf7bfd/0xa9bd... ) ou sub sp,sp,#N
                        // (0xd10x...). Checagem frouxa: 1º opcode não-zero e não-0xFFFFFFFF.
                        let op0 = u32::from_le_bytes([bytes16[0],bytes16[1],bytes16[2],bytes16[3]]);
                        prologue_ok = op0 != 0 && op0 != 0xFFFFFFFF;
                    }
                    log(&format!(
                        "[journalvtdump] vtbl+{mac_off:#x} ({label}, win={win_off:#x}) rt={fn_rt:#x} vmaddr={vmaddr:#010x} bytes={bytes16:02x?} plausible={prologue_ok}"
                    ));
                }
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
        // vtbl <cls-name> <n_slots> — lista N slots da vtable de uma classe RTTI
        // Usa o punteiro de instância da classe para achar a vtable e converte para vmaddr.
        // Ex: vtbl gameuiCharacterCustomizationSystem 30
        // 2026-08-05 (auditoria de blind spots): leitura PURA (zero chamada, zero risco — mesmo
        // mecanismo já provado 4x: GetClass/GetEnum/RegisterType/RegisterFunction) dos slots da
        // vtable do CRTTISystem que o SDK real documenta mas este projeto nunca usou —
        // AddRegisterCallback(+0xC0)/AddPostRegisterCallback(+0xC8)/CreateScriptedClass(+0xE0)/
        // CreateScriptedEnum(+0xE8). RE offline não achou NENHUM caller desses 4 no binário
        // inteiro (o jogo vanilla nunca registra enum/classe scriptada em runtime) — sem
        // endereço concreto não dá pra confirmar o ABI real, e chamar às cegas é a MESMA
        // categoria de erro que já crashou este projeto antes. Só lê+loga os endereços aqui;
        // uma rodada de RE offline dedicada disassembla o corpo real depois.
        // 2026-08-05: teste ISOLADO de CreateScriptedEnum (ABI confirmada por RE de disassembly,
        // ver register.rs::create_scripted_enum) com um enum de TESTE inofensivo, antes de tentar
        // EInputAction de verdade — mesma disciplina de sempre (nunca testar a hipótese arriscada
        // já indo pro alvo real).
        ["mkenumtest"] => {
            unsafe {
                match crate::rtti::Registry::obtain() {
                    None => log("[mkenumtest] Registry::obtain() falhou"),
                    Some(reg) => {
                        let ok = crate::register::create_scripted_enum(
                            &reg,
                            "BwmsTestEnum",
                            1,
                            &[("BWMS_A", 0), ("BWMS_B", 1), ("BWMS_C", 2)],
                        );
                        log(&format!("[mkenumtest] create_scripted_enum -> {ok}"));
                        if ok {
                            let e = reg.enum_by_name("BwmsTestEnum");
                            log(&format!("[mkenumtest] enum_by_name('BwmsTestEnum') -> {e:p} (não-null = achou de volta no RTTI)"));
                        }
                    }
                }
            }
            return;
        }
        ["crttivtbl"] => {
            unsafe {
                match crate::rtti::Registry::obtain() {
                    None => log("[crttivtbl] Registry::obtain() falhou"),
                    Some(reg) => {
                        let base = crate::game_base();
                        for (label, off) in [
                            ("AddRegisterCallback", 0xC0usize),
                            ("AddPostRegisterCallback", 0xC8usize),
                            ("CreateScriptedClass", 0xE0usize),
                            ("CreateScriptedEnum", 0xE8usize),
                        ] {
                            let fp = reg.vtbl_slot(off) as usize;
                            if fp == 0 {
                                log(&format!("[crttivtbl] +{off:#x} ({label}) ilegível/null"));
                                continue;
                            }
                            let vmaddr = if fp > base { fp - base + 0x1_0000_0000 } else { fp };
                            log(&format!("[crttivtbl] +{off:#x} ({label}) rt={fp:#x} vmaddr={vmaddr:#010x}"));
                        }
                    }
                }
            }
            return;
        }
        ["vtbl", class, n_str] => {
            let n: usize = n_str.parse().unwrap_or(20).min(64);
            unsafe {
                let obj = crate::rtti::new_object(&reg, class);
                if obj.is_null() {
                    log(&format!("[vtbl] new_object falhou para '{class}'"));
                    return;
                }
                if !crate::gum::is_readable(obj, 8) {
                    log(&format!("[vtbl] instância {obj:p} ilegível"));
                    return;
                }
                let vtbl_ptr = (obj as *const u64).read_unaligned() as *const u64;
                if vtbl_ptr.is_null() || !crate::gum::is_readable(vtbl_ptr as *const c_void, (n * 8) as usize) {
                    log(&format!("[vtbl] vtable {vtbl_ptr:p} ilegível"));
                    return;
                }
                log(&format!("[vtbl] class={class} vtbl={vtbl_ptr:p}"));
                let base = crate::game_base();
                for i in 0..n {
                    let slot_ptr = vtbl_ptr.add(i) as *const u64;
                    if !crate::gum::is_readable(slot_ptr as *const c_void, 8) { break; }
                    let fn_rt = slot_ptr.read_unaligned() as usize;
                    let vmaddr = if fn_rt > base { fn_rt - base + 0x1_0000_0000 } else { fn_rt };
                    log(&format!("[vtbl] [{i:02}] rt={fn_rt:#x} vmaddr={vmaddr:#010x}"));
                }
            }
            return;
        }
        ["propdump", class] => {
            log(&unsafe { crate::rtti::dump_props(reg, class) });
            return;
        }
        // RED4ext.SDK — introspecção fina de flags (`PENDENCIAS-UNIFICADAS.md`, "CClass::Flags
        // 11 bits / CProperty.flags 13 bits / CBaseFunction.flags 13 bits — maioria nunca
        // lida"). 3 comandos read-only, decodificam os bits nomeados dos 3 headers vendorizados.
        ["classflags", name] => {
            unsafe { crate::rtti::probe_class_flags(reg, name) };
            return;
        }
        ["propflags", class, prop] => {
            unsafe { crate::rtti::probe_property_flags(reg, class, prop) };
            return;
        }
        ["funcflags", class, method] => {
            unsafe { crate::rtti::probe_function_flags(reg, class, method) };
            return;
        }
        // `funcflagsglobal <nome>` (2026-08-10) — RED4ext.SDK #186 (lado escrita): diagnóstico
        // pra confirmar/refutar o "palpite" documentado em `register.rs::FLAG_STATIC` (bit2,
        // nunca verificado contra o ground-truth de `decode_function_flags`, que diz bit1=
        // isStatic/bit2=isFinal do header vendorizado). Testa contra uma native GLOBAL nossa
        // (sempre estática) já registrada.
        ["funcflagsglobal", name] => {
            unsafe {
                let f = register::get_function(reg, name);
                if !crate::rtti::sane(f) {
                    return log(&format!("[funcflagsglobal] '{name}' não resolveu"));
                }
                match crate::rtti::function_flags(f) {
                    Some(raw) => log(&format!(
                        "[funcflagsglobal] '{name}' flags={raw:#010x} -> {}",
                        crate::rtti::decode_function_flags(raw)
                    )),
                    None => log(&format!("[funcflagsglobal] '{name}' flags ilegível")),
                }
            }
            return;
        }
        // `funcflagswrite <nome-global> <hex-mask>` (2026-08-11) — RED4ext.SDK #186, fecha o
        // lado ESCRITA por completo (13/13 bits nomeados). OR o mask nos flags já existentes
        // (preserva isNative/isStatic) + lê de volta via o mesmo decode_function_flags já usado
        // por funcflagsglobal. Campo é metadado puro — despacho nunca depende destes bits (ver
        // nota em rtti::function_flags_write).
        ["funcflagswrite", name, mask_hex] => {
            unsafe {
                let f = register::get_function(reg, name);
                if !crate::rtti::sane(f) {
                    return log(&format!("[funcflagswrite] '{name}' não resolveu"));
                }
                let mask = u32::from_str_radix(mask_hex.trim_start_matches("0x"), 16).unwrap_or(0);
                let before = crate::rtti::function_flags(f).unwrap_or(0);
                let new_flags = before | mask;
                let ok = crate::rtti::function_flags_write(f, new_flags);
                log(&format!("[funcflagswrite] '{name}' before={before:#010x} mask={mask:#010x} write_ok={ok}"));
                match crate::rtti::function_flags(f) {
                    Some(raw) => log(&format!(
                        "[funcflagswrite] '{name}' readback flags={raw:#010x} -> {}",
                        crate::rtti::decode_function_flags(raw)
                    )),
                    None => log(&format!("[funcflagswrite] '{name}' readback ilegível")),
                }
            }
            return;
        }
        // Codeware `Reflection.ReflectionFunc` — "introspecção de assinatura" (nome, retorno,
        // params, is_native/is_static), junta `resolve_func`+`type_kind`+`function_flags`.
        ["funcsig", class, method] => {
            unsafe { crate::rtti::probe_function_signature(reg, class, method) };
            return;
        }
        // Codeware `Reflection.ReflectionEnum`/`ReflectionBitfield.IsNative()`.
        ["enumisnative", name] => {
            unsafe { crate::rtti::probe_type_is_native(reg, name) };
            return;
        }
        // Codeware `ReflectionClass.GetFunctions()`/`GetStaticFunctions()` (item #51).
        ["funclistdump", class] => {
            unsafe { crate::rtti::probe_class_own_functions(reg, class, 15) };
            return;
        }
        ["funclistdump", class, n] => {
            let n: usize = n.parse().unwrap_or(15).min(200);
            unsafe { crate::rtti::probe_class_own_functions(reg, class, n) };
            return;
        }
        // RED4ext.SDK — `CClass.defaults` (Map<CName,Variant*>@+0x158, PENDENCIAS-UNIFICADAS.md
        // prosa "Restante"): valor-default de uma propriedade, nunca lido antes.
        ["classdefault", class, prop] => {
            unsafe { crate::rtti::probe_class_default(reg, class, prop) };
            return;
        }
        // `newobjget <classe> <prop>` (2026-08-10) — RED4ext.SDK #102 (`CClass.defaults`): a via
        // OFICIAL (ler o Map interno `CClass+0x158`) deu vazio nos 5 casos testados antes. Via
        // alternativa: construir uma instância NOVA (`rtti::new_object`, já provado) e ler a prop
        // (`find_property_in_class`+`prop_get_*`, já provado) — o valor que sai do `Construct()`
        // real do motor É o default de fato, na prática mais confiável que o Map (que parece
        // raramente populado). Read-only sobre um objeto DESCARTÁVEL (nunca registrado em lugar
        // nenhum, sem side-effect fora de si mesmo).
        ["newobjget", class, prop] => {
            unsafe {
                let cls = reg.class_by_name(class);
                if cls.is_null() {
                    return log(&format!("[newobjget] classe '{class}' não encontrada"));
                }
                let obj = crate::rtti::new_object(reg, class);
                if obj.is_null() {
                    return log(&format!("[newobjget] new_object('{class}') falhou"));
                }
                let p = crate::rtti::find_property_in_class(cls, prop);
                if p.is_null() {
                    return log(&format!("[newobjget] prop '{prop}' não achada em '{class}'"));
                }
                let vo = crate::rtti::prop_value_offset(p);
                let u = crate::rtti::prop_get_u32(p, obj);
                let f = crate::rtti::prop_get_f32(p, obj);
                let b = crate::rtti::prop_get_bool(p, obj);
                log(&format!(
                    "[newobjget] {class}.{prop} (vo={vo:#x}, instância NOVA descartável @{obj:p}) = u32 {u:#x} / i32 {} / f32 {f} / bool {b}",
                    u as i32
                ));
            }
            return;
        }
        // RED4ext.SDK — "família de tipos-wrapper" (itens #129-158): introspecção genérica de
        // categoria de tipo via `rtti::IType::GetType()` (macOS vtbl+0x28, shift de +0x08 vs
        // Windows — 2 dtors Itanium, achado já documentado no código, nunca exposto como comando).
        ["proptypekind", class, prop] => {
            unsafe { crate::rtti::probe_property_type_kind(reg, class, prop) };
            return;
        }
        // Codeware `Reflection.ReflectionEnum.AddConstant` (PENDENCIAS-UNIFICADAS.md, Codeware
        // "Reflection... AddConstant"): cria enum de teste, adiciona constante, lê de volta.
        ["enumaddconstprobe"] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::register::probe_enum_add_constant(&reg) };
            } else {
                log("[enumaddconst] Registry indisponível");
            }
            return;
        }
        ["proptypekindall", class] => {
            unsafe { crate::rtti::probe_property_type_kind_all(reg, class, 20) };
            return;
        }
        ["proptypekindall", class, n] => {
            let n: usize = n.parse().unwrap_or(20).min(200);
            unsafe { crate::rtti::probe_property_type_kind_all(reg, class, n) };
            return;
        }
        // Codeware `#199`: varredura BFS pela hierarquia de `âncora` procurando um
        // `type_kind()==11` (ResourceReference/Ref<T>) real, pra finalmente testar
        // `IsReferenceLoaded`/`GetReferenceResource` contra um alvo positivo genuíno.
        ["scan11", anchor] => {
            unsafe { crate::rtti::scan_type_kind_across_hierarchy(reg, anchor, 3000) };
            return;
        }
        ["scan11", anchor, max] => {
            let max: usize = max.parse().unwrap_or(3000).min(20_000);
            unsafe { crate::rtti::scan_type_kind_across_hierarchy(reg, anchor, max) };
            return;
        }
        // Codeware `#199`: constrói uma instância sintética de `classe` e roda a MESMA lógica
        // de `BwmsIsResourceReferenceLoaded`/`BwmsGetResourceReferenceResource` direto (bypass do
        // marshalling redscript), sobre um campo `type_kind==11` REAL achado via `scan11`.
        ["res11probe", class, prop] => {
            unsafe { crate::register::probe_resource_helper_type11(reg, class, prop) };
            return;
        }
        // Codeware `#199` (2026-08-18, continuação da continuação): `res11rec <classe> <prop>
        // [cap]` — mesma sonda do `res11probe`, mas contra records REAIS do TweakDB vivo (via
        // `get_records_of_class`, já provado pelo ArchiveXL #59) em vez de instância sintética.
        ["res11rec", class, prop] => {
            unsafe { crate::register::probe_resource_helper_type11_records(reg, class, prop, 5) };
            return;
        }
        ["res11rec", class, prop, cap] => {
            let cap: usize = cap.parse().unwrap_or(5).min(50);
            unsafe { crate::register::probe_resource_helper_type11_records(reg, class, prop, cap) };
            return;
        }
        // Codeware `#199` (2026-08-18, pivô final): `res11walk [max_layers] [max_nodes]` — anda a
        // árvore real de widgets a partir de cada InkLayer->gameController procurando um campo
        // `inkWidgetBrush` VIVO (não sintético), via natives vanilla (GetRootWidget/
        // GetNumChildren/GetWidgetByIndex) chamadas direto por Rust.
        ["res11walk"] => {
            unsafe { crate::register::probe_widget_tree_for_brush(reg, 20, 500) };
            return;
        }
        ["res11walk", max_layers, max_nodes] => {
            let ml: usize = max_layers.parse().unwrap_or(20).min(200);
            let mn: usize = max_nodes.parse().unwrap_or(500).min(20_000);
            unsafe { crate::register::probe_widget_tree_for_brush(reg, ml, mn) };
            return;
        }
        // `axl-customization-apply`: lista métodos RTTI (instância+estáticos) de uma classe.
        // Ex: rttifuncs gameuiCharacterCustomizationSystem → GetHeadOptions/GetBodyOptions etc.
        ["rttifuncs", class] => {
            log(&unsafe { crate::rtti::dump_class_funcs(reg, class) });
            return;
        }
        // `cw-controller-misc`: dado CClass+método, acha o CClassFunction* e despeja qwords
        // em múltiplos offsets — identifica qual offset tem o ponteiro nativo C++ (no TEXT).
        // Ex: nativefunc IComponent Toggle → despeja CFunc*+0x00..+0x60; candidato = vmaddr TEXT.
        // `resolvefuncrobust <nome>` (2026-08-10) — RED4ext.SDK #55: compara `register::get_function`
        // (vtbl+0x30, só acha o que foi RegisterFunction-ado pelo Rust) contra
        // `rtti::resolve_global_function_robust` (varre GetGlobalFunctions, casa por nome/fullName)
        // — prova se o fallback resolve uma global PURAMENTE ESCRIPTADA que a via rápida não acha.
        ["resolvefuncrobust", name] => {
            unsafe {
                let fast = crate::register::get_function(reg, name);
                let robust = crate::rtti::resolve_global_function_robust(reg, name);
                log(&format!(
                    "[resolvefuncrobust] '{name}': get_function(vtbl+0x30)={fast:p} | resolve_global_function_robust(varredura)={robust:p}"
                ));
            }
            return;
        }
        ["nativefunc", class, method] => {
            unsafe {
                let cls = reg.class_by_name(class);
                if cls.is_null() {
                    log(&format!("[nativefunc] classe '{class}' não encontrada"));
                    return;
                }
                match crate::rtti::resolve_in_class(cls, method) {
                    None => log(&format!("[nativefunc] '{method}' não achado em '{class}'")),
                    Some(rf) => {
                        let f = rf.func as *const u8;
                        log(&format!("[nativefunc] '{class}::{method}' func={f:p}"));
                        let base = crate::game_base() as u64;
                        for off in (0usize..=0x80).step_by(8) {
                            if !crate::gum::is_readable(f.add(off) as *const c_void, 8) { break; }
                            let v = (f.add(off) as *const u64).read_unaligned();
                            let is_text = v > base && v < base + 0x800_0000;
                            let vm = if is_text { format!("vmaddr={:#010x}", v - base + 0x1_0000_0000u64) } else { String::new() };
                            log(&format!("[nativefunc] +{off:#04x}: {v:#018x} {vm}"));
                        }
                    }
                }
            }
            return;
        }
        // `getcustsys`: obtém o ptr do gameuiCharacterCustomizationSystem via GameInstance
        // (player.GetGame() → GetCharacterCustomizationSystem()), atualiza CHAR_CUSTOM_SYS_PTR,
        // e loga o ptr para uso em callon/custsys.
        ["getcustsys"] => {
            unsafe {
                // BUG CORRIGIDO 2026-07-28: `class_of(player)` derivava a classe da INSTÂNCIA
                // capturada — falha (game_cls=0x0) logo após save-load/transições de cena, mesmo
                // com `player` não-nulo (mesmo padrão documentado em console.rs::auth_player,
                // "player capturado costuma ser puppet transiente"). Fix: resolver "GetGame"
                // DIRETO da classe estática "PlayerPuppet" (sempre registrada), sem depender do
                // estado da instância — mesma receita robusta já usada em auth_player.
                log(&format!("[getcustsys] player={player:p}"));
                match crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame") {
                    None => log("[getcustsys] PlayerPuppet.GetGame não resolvido"),
                    Some(rf) => match crate::rtti::call_func(&rf, player, &[]) {
                        None => log("[getcustsys] GetGame() não completou"),
                        Some(r) => {
                            let game_ptr = u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]);
                            log(&format!("[getcustsys] GetGame()={game_ptr:#018x}"));
                            if game_ptr == 0 { return; }
                            // CAUSA RAIZ ACHADA 2026-07-28 (via `nativefunc ScriptGameInstance
                            // GetQuestsSystem`, mesmo shape): a struct do CClassFunction tem
                            // p_count=1 (campo +0x30, lido por `call_func::p_count`) — este native
                            // espera 1 ARG explícito, não 0. `GetCharacterCustomizationSystem(self:
                            // GameInstance)` é o padrão redscript "static com self explícito"
                            // (chamado como `GameInstance.GetX(game)` no .script real, não
                            // `game.GetX()`) — o GameInstance tem que ir no ARRAY DE ARGS
                            // (`Arg::Raw`, o próprio comentário do enum já dizia "ex.: GameInstance")
                            // com `ctx` sendo QUALQUER objeto válido (não o GameInstance) — exatamente
                            // como `console.rs::auth_player` já fazia pra `GetPlayerSystem` (mesmo
                            // padrão de assinatura). O crash de antes (EXC_BAD_ACCESS, endereço com
                            // cara de hash) era ler PARÂMETRO FALTANTE como se fosse dado — `call_func`
                            // só rejeita `args.len() > p_count`, nunca `args.len() < p_count`, então
                            // a chamada com 0 args "passava" sem erro só pra crashar dentro do native.
                            let mut game_arg = [0u8; 16];
                            game_arg[..8].copy_from_slice(&game_ptr.to_le_bytes());
                            match crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetCharacterCustomizationSystem") {
                                None => log("[getcustsys] GetCharacterCustomizationSystem não achado em GameInstance"),
                                Some(rf2) => match crate::rtti::call_func(&rf2, player, &[crate::rtti::Arg::Raw(game_arg)]) {
                                    None => log("[getcustsys] GetCharacterCustomizationSystem() não completou"),
                                    Some(r2) => {
                                        let sys_ptr = u64::from_le_bytes([r2[0],r2[1],r2[2],r2[3],r2[4],r2[5],r2[6],r2[7]]);
                                        log(&format!("[getcustsys] gameuiCharacterCustomizationSystem={sys_ptr:#018x}"));
                                        if sys_ptr != 0 {
                                            crate::selftest::CHAR_CUSTOM_SYS_PTR.store(sys_ptr, std::sync::atomic::Ordering::Relaxed);
                                            log("[getcustsys] CHAR_CUSTOM_SYS_PTR actualizado — custsys probe vai funcionar");
                                        }
                                    }
                                },
                            }
                        }
                    },
                }
            }
            return;
        }
        // `axl-questphase-apply`: mesma ABI corrigida de `getcustsys` (GameInstance vai em
        // Arg::Raw, não em ctx) aplicada a `GameInstance.GetQuestsSystem(game)`. Dado o ptr do
        // questQuestsSystem vivo, dumpa a vtable nos 2 slots que a RE offline (agente dedicado,
        // 2026-07-28) já apontou como candidatos: 0x158 (OnGameRestored, Windows 0x150 +0x08
        // Itanium) e 0x368 (candidato pro caminho de Start, achado dentro de QuestsSystem::Tick).
        ["getquestsys"] => {
            unsafe {
                match crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame") {
                    None => log("[getquestsys] PlayerPuppet.GetGame não resolvido"),
                    Some(rf) => match crate::rtti::call_func(&rf, player, &[]) {
                        None => log("[getquestsys] GetGame() não completou"),
                        Some(r) => {
                            let game_ptr = u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]);
                            if game_ptr == 0 { log("[getquestsys] GetGame()=0"); return; }
                            let mut game_arg = [0u8; 16];
                            game_arg[..8].copy_from_slice(&game_ptr.to_le_bytes());
                            match crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetQuestsSystem") {
                                None => log("[getquestsys] GetQuestsSystem não achado em GameInstance"),
                                Some(rf2) => match crate::rtti::call_func(&rf2, player, &[crate::rtti::Arg::Raw(game_arg)]) {
                                    None => log("[getquestsys] GetQuestsSystem() não completou"),
                                    Some(r2) => {
                                        let sys_ptr = u64::from_le_bytes([r2[0],r2[1],r2[2],r2[3],r2[4],r2[5],r2[6],r2[7]]);
                                        log(&format!("[getquestsys] questQuestsSystem={sys_ptr:#018x}"));
                                        if sys_ptr == 0 { return; }
                                        let obj = sys_ptr as *const u8;
                                        if !crate::gum::is_readable(obj as *const c_void, 8) {
                                            log("[getquestsys] instância ilegível");
                                            return;
                                        }
                                        let vt = (obj as *const u64).read_unaligned() as *const u8;
                                        let base = crate::game_base();
                                        // ForceStartNode-cand (2026-08-10, cont.192): item ArchiveXL #55.
                                        // `Raw::QuestsSystem::ForceStartNode = Core::RawVFunc<0x240,
                                        // void(questIQuestsSystem::*)(const QuestNodeKey&, const
                                        // DynArray<CName>&)>` no header vendorizado (Windows offset).
                                        // Mesma convenção já confirmada NESTA classe (OnGameRestored:
                                        // Windows 0x150 -> Mac 0x158, +0x08 Itanium) aplicada: candidato
                                        // Mac = 0x240+0x08 = 0x248. Leitura pura, zero chamada.
                                        for (label, slot_off) in [("OnGameRestored-cand", 0x158usize), ("Start-cand", 0x368usize), ("ForceStartNode-cand", 0x248usize)] {
                                            let slot_ptr = vt.add(slot_off) as *const u64;
                                            if !crate::gum::is_readable(slot_ptr as *const c_void, 8) {
                                                log(&format!("[getquestsys] vtbl+{slot_off:#x} ({label}) ilegível"));
                                                continue;
                                            }
                                            let fn_rt = slot_ptr.read_unaligned() as usize;
                                            let vmaddr = if fn_rt > base { fn_rt - base + 0x1_0000_0000 } else { fn_rt };
                                            log(&format!("[getquestsys] vtbl+{slot_off:#x} ({label}) rt={fn_rt:#x} vmaddr={vmaddr:#010x}"));
                                        }
                                    }
                                },
                            }
                        }
                    },
                }
            }
            return;
        }
        // `axl-questphase-apply` (2026-08-05, via RE — passo 2): `questdump` (abaixo) achou os
        // vtables ESTÁTICOS reais de `questRootInstance` (0x1_0070fd348 — confirmado pela string
        // RTTI "questRootInstance" no site de registro do tipo) e da base `questPhaseInstance`
        // (0x1_0070fd088, string "questPhaseInstance"). Só os 4 primeiros slots foram capturados
        // (boilerplate comum ISerializable: GetNativeType/GetType/helper-compartilhado/destrutor)
        // — `Start`/`ProcessPhaseResource`, se forem virtuais, ficam no slot 4+. Leitura direta e
        // estática da região __DATA_CONST (sempre mapeada, não depende de instância viva nenhuma).
        ["questvtabledump"] => {
            unsafe {
                for (label, vt_vmaddr) in [
                    ("questRootInstance", 0x1070fd348u64),
                    ("questPhaseInstance(base)", 0x1070fd088u64),
                ] {
                    let vt = crate::rebase(vt_vmaddr) as *const u8;
                    if !crate::gum::is_readable(vt as *const c_void, 8) {
                        log(&format!("[questvtabledump] {label} vtable={vt_vmaddr:#010x} ilegível"));
                        continue;
                    }
                    log(&format!("[questvtabledump] === {label} vtable estático={vt_vmaddr:#010x} ==="));
                    for slot in 0..24usize {
                        let sp = vt.add(slot * 8) as *const u64;
                        if !crate::gum::is_readable(sp as *const c_void, 8) {
                            log(&format!("[questvtabledump] {label} slot{slot} ilegível"));
                            break;
                        }
                        let fn_rt = sp.read_unaligned();
                        let fn_vm = crate::un_rebase(fn_rt as *const c_void);
                        log(&format!("[questvtabledump] {label} slot{slot} vmaddr={fn_vm:#010x}"));
                    }
                }
            }
            return;
        }
        // `axl-questphase-apply` (2026-08-05, via RE): `Start`/`ProcessPhaseResource` nunca
        // aparecem no RTTI (confirmado exaustivamente, ver HISTORICO.md cont.24) — não dá pra
        // achar por nome/registro de classe. Em vez disso, varredura CRUA de dados (não vtable)
        // do `questQuestsSystem` vivo, mesmo padrão observe-only já provado em `wardrobesys`:
        // pra cada qword que parece um ponteiro de heap plausível, lê o 1º qword DO ALVO — se
        // cair dentro do `__TEXT` do binário (`un_rebase()!=0`), é candidato a VTABLE de um
        // objeto C++ real (ex. `questRootInstance`, que o sistema provavelmente rastreia como
        // membro) — dá uma âncora estática nova pra RE offline dedicada. Zero mutação, zero hook.
        ["questdump"] => {
            unsafe {
                match crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame") {
                    None => log("[questdump] PlayerPuppet.GetGame não resolvido"),
                    Some(rf) => match crate::rtti::call_func(&rf, player, &[]) {
                        None => log("[questdump] GetGame() não completou"),
                        Some(r) => {
                            let game_ptr = u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]);
                            if game_ptr == 0 { log("[questdump] GetGame()=0"); return; }
                            let mut game_arg = [0u8; 16];
                            game_arg[..8].copy_from_slice(&game_ptr.to_le_bytes());
                            match crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetQuestsSystem") {
                                None => log("[questdump] GetQuestsSystem não achado em GameInstance"),
                                Some(rf2) => match crate::rtti::call_func(&rf2, player, &[crate::rtti::Arg::Raw(game_arg)]) {
                                    None => log("[questdump] GetQuestsSystem() não completou"),
                                    Some(r2) => {
                                        let sys_ptr = u64::from_le_bytes([r2[0],r2[1],r2[2],r2[3],r2[4],r2[5],r2[6],r2[7]]);
                                        log(&format!("[questdump] questQuestsSystem={sys_ptr:#018x}"));
                                        if sys_ptr == 0 { return; }
                                        let obj = sys_ptr as *const u8;
                                        if !crate::gum::is_readable(obj as *const c_void, 0x400) {
                                            log("[questdump] instância ilegível (menos de 0x400 bytes)");
                                            return;
                                        }
                                        // 2026-08-05 (refinado por RE): a 1ª passada (só `un_rebase()!=0`)
                                        // deu 100% falso-positivo — pegava qualquer coisa perto da base
                                        // (funções em __TEXT,__text, plumbing de alocador). Vtables REAIS
                                        // deste binário ficam em __DATA_CONST,__const (achado por RE,
                                        // range estático 0x106e4a8d8..0x107392948) — filtro agora exige
                                        // isso, e valida o candidato lendo os 4 slots seguintes: um vtable
                                        // de verdade tem uma SEQUÊNCIA de ponteiros pra dentro de
                                        // __TEXT,__text (0x100002070..0x104a396a0), não lixo aleatório.
                                        const DATA_CONST_LO: u64 = 0x1_06e4_a8d8;
                                        const DATA_CONST_HI: u64 = 0x1_0739_2948;
                                        const TEXT_LO: u64 = 0x1_0000_2070;
                                        const TEXT_HI: u64 = 0x1_04a3_96a0;
                                        for off in (0x08usize..0x400).step_by(8) {
                                            let v = (obj.add(off) as *const u64).read_unaligned();
                                            if v == 0 { continue; }
                                            let candidate = v as *const c_void;
                                            if !crate::gum::is_readable(candidate, 8) { continue; }
                                            let first_qword = (candidate as *const u64).read_unaligned();
                                            let vmaddr = crate::un_rebase(first_qword as *const c_void);
                                            if vmaddr < DATA_CONST_LO || vmaddr > DATA_CONST_HI { continue; }
                                            let vt = first_qword as *const u8;
                                            let mut slots_valid = 0u32;
                                            let mut slot_log = String::new();
                                            for slot in 0..4usize {
                                                let sp = vt.add(slot * 8) as *const u64;
                                                if !crate::gum::is_readable(sp as *const c_void, 8) { continue; }
                                                let fn_rt = sp.read_unaligned();
                                                let fn_vm = crate::un_rebase(fn_rt as *const c_void);
                                                if fn_vm >= TEXT_LO && fn_vm <= TEXT_HI { slots_valid += 1; }
                                                slot_log.push_str(&format!(" slot{slot}={fn_vm:#010x}"));
                                            }
                                            log(&format!("[questdump] +{off:#x} = {v:#018x} -> vtable-candidato={vmaddr:#010x} slots_validos={slots_valid}/4{slot_log}"));
                                        }
                                    }
                                },
                            }
                        }
                    },
                }
            }
            return;
        }
        // `cw-player-scheduling-vehicle` (WardrobeSystem::ForgetItemID): a fonte real do Codeware
        // (`WardrobeSystemEx.hpp`) já dá os offsets do Windows — `HashMap<CName,ItemID> ItemStore`
        // em +0x48, `SharedSpinLock ItemStoreMutex` em +0xF4 — NÃO são endereços de função (que
        // mudam entre plataformas), são offsets de DADOS/struct, que este projeto já confirmou
        // repetidas vezes portarem 1:1 macOS↔Windows (`Core::OffsetPtr<0xNN>`). Em vez de tentar
        // achar uma função nativa que NÃO EXISTE (RE já confirmou isso — ver DATABASE.md), lê os
        // bytes crus nesses offsets de uma instância VIVA (mesma ABI de "static com self explícito"
        // já provada em getquestsys/getcustsys) — observe-only, zero mutação, zero hook — só pra
        // VALIDAR se os offsets batem (heurística: HashMap real tem cara de {ptr,count,cap} ou
        // similar; SharedSpinLock real é 0 quando destravado).
        ["wardrobesys"] => {
            unsafe {
                match crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame") {
                    None => log("[wardrobesys] PlayerPuppet.GetGame não resolvido"),
                    Some(rf) => match crate::rtti::call_func(&rf, player, &[]) {
                        None => log("[wardrobesys] GetGame() não completou"),
                        Some(r) => {
                            let game_ptr = u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]);
                            if game_ptr == 0 { log("[wardrobesys] GetGame()=0"); return; }
                            let mut game_arg = [0u8; 16];
                            game_arg[..8].copy_from_slice(&game_ptr.to_le_bytes());
                            match crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetWardrobeSystem") {
                                None => log("[wardrobesys] GetWardrobeSystem não achado em GameInstance"),
                                Some(rf2) => match crate::rtti::call_func(&rf2, player, &[crate::rtti::Arg::Raw(game_arg)]) {
                                    None => log("[wardrobesys] GetWardrobeSystem() não completou"),
                                    Some(r2) => {
                                        let sys_ptr = u64::from_le_bytes([r2[0],r2[1],r2[2],r2[3],r2[4],r2[5],r2[6],r2[7]]);
                                        log(&format!("[wardrobesys] WardrobeSystem={sys_ptr:#018x}"));
                                        if sys_ptr == 0 { return; }
                                        let obj = sys_ptr as *const u8;
                                        if !crate::gum::is_readable(obj as *const c_void, 0x100) {
                                            log("[wardrobesys] instância ilegível (menos de 0x100 bytes)");
                                            return;
                                        }
                                        // dump bruto de +0x40..+0x60 (janela ao redor do candidato +0x48
                                        // ItemStore) + +0xF0..+0x100 (janela ao redor do +0xF4 mutex)
                                        for off in (0x40usize..0x60).step_by(8) {
                                            let v = (obj.add(off) as *const u64).read_unaligned();
                                            log(&format!("[wardrobesys] +{off:#x} = {v:#018x}"));
                                        }
                                        for off in (0xf0usize..0x100).step_by(8) {
                                            let v = (obj.add(off) as *const u64).read_unaligned();
                                            log(&format!("[wardrobesys] +{off:#x} = {v:#018x}"));
                                        }
                                        // 2026-07-31: enumeração COMPLETA, 100% observe-only (zero
                                        // escrita), do HashMap<CName,ItemID> real em WardrobeSystem+0x48.
                                        // Layout confirmado por agente de RE contra RED4ext.SDK/HashMap.hpp
                                        // (ground-truth, ver DATABASE.md): indexTable@+0x00(rel ao HashMap,
                                        // =WardrobeSystem+0x48), size@+0x08(u32), capacity@+0x0C(u32),
                                        // nodes@+0x10(Node*), Node{next:u32@0,hashedKey:u32@4,key:CName
                                        // u64@8,value:ItemID 16B@0x10}, stride=0x20. Caminha TODOS os
                                        // buckets + cadeias — nunca escreve nada, só lê e loga. Serve pra
                                        // validar o algoritmo de hash/bucket/cadeia ANTES de confiar numa
                                        // implementação de Remove (que precisaria escrever).
                                        let hm = obj.add(0x48);
                                        let index_table = (hm as *const u64).read_unaligned() as *const u32;
                                        let size_cap = (hm.add(0x08) as *const u64).read_unaligned();
                                        let map_size = (size_cap & 0xFFFF_FFFF) as u32;
                                        let capacity = (size_cap >> 32) as u32;
                                        let nodes = (hm.add(0x10) as *const u64).read_unaligned() as *const u8;
                                        log(&format!("[wardrobesys] HashMap: indexTable={index_table:p} size={map_size} capacity={capacity} nodes={nodes:p}"));
                                        if capacity == 0 || capacity > 4096 || index_table.is_null() || nodes.is_null() {
                                            log("[wardrobesys] capacity/ponteiros fora de faixa plausível, abortando enumeração");
                                        } else if !crate::gum::is_readable(index_table as *const c_void, (capacity as usize) * 4) {
                                            log("[wardrobesys] indexTable ilegível");
                                        } else {
                                            const STRIDE: usize = 0x20;
                                            const INVALID: u32 = 0xFFFF_FFFF;
                                            let mut found: u32 = 0;
                                            for bucket in 0..capacity {
                                                let mut idx = index_table.add(bucket as usize).read_unaligned();
                                                let mut guard = 0;
                                                while idx != INVALID && guard < capacity + 1 {
                                                    guard += 1;
                                                    let node = nodes.add(idx as usize * STRIDE);
                                                    if !crate::gum::is_readable(node as *const c_void, STRIDE) {
                                                        log(&format!("[wardrobesys] bucket={bucket} idx={idx} node ilegível, parando cadeia"));
                                                        break;
                                                    }
                                                    let next = (node as *const u32).read_unaligned();
                                                    let hashed_key = (node.add(4) as *const u32).read_unaligned();
                                                    let key = (node.add(8) as *const u64).read_unaligned();
                                                    let value_lo = (node.add(0x10) as *const u64).read_unaligned();
                                                    let value_hi = (node.add(0x18) as *const u64).read_unaligned();
                                                    found += 1;
                                                    log(&format!("[wardrobesys] item#{found} bucket={bucket} idx={idx} hashedKey={hashed_key:#010x} CName={key:#018x} ItemID=({value_lo:#018x},{value_hi:#018x})"));
                                                    idx = next;
                                                }
                                            }
                                            log(&format!("[wardrobesys] enumeração completa: {found} entradas encontradas (map_size reportado={map_size})"));
                                        }
                                    }
                                },
                            }
                        }
                    },
                }
            }
            return;
        }
        // `cw-player-scheduling-vehicle`: `WardrobeSystem::ForgetItemID` — implementação REAL de
        // `HashMap<CName,ItemID>::Remove`, layout confirmado por agente de RE dedicado contra
        // `RED4ext.SDK/include/RED4ext/HashMap.hpp` (ground-truth, 2 corroborações independentes:
        // fonte + leitura ao vivo no Mac batendo exata — ver DATABASE.md). Algoritmo (RED4ext.SDK
        // HashMap.hpp linhas ~196-239): hashedKey = XOR-fold do CName (já é hash FNV1a64) —
        // (u32)hash ^ (u32)(hash>>32); bucket = hashedKey % capacity; caminha a cadeia por `.next`
        // rastreando o ponteiro ANTERIOR (indexTable[bucket] ou node.next de quem veio antes);
        // ao achar, desengancha (*prev = node.next), empurra o slot liberado na freelist
        // (node.next = nodeList.nextIdx; nodeList.nextIdx = idx), decrementa `size`. Sob
        // SharedSpinLock @+0xF4 (CAS 0->0xFF, mesmo padrão já provado em `tweakdb_rt::mutex00_lock`).
        // **CODADO+build-verificado, mas DELIBERADAMENTE NÃO testado ao vivo contra o save real
        // do usuário** — ao contrário de leituras (reversíveis por definição), uma ESCRITA errada
        // num HashMap do motor pode corromper heap ou (pior) persistir uma mutação indesejada no
        // save se `ItemStore` for serializado. `wardrobesys` (comando acima) já confirmou o
        // read-path 100% (enumeração bateu EXATO com `size` reportado, incl. uma cadeia de colisão
        // real). Faltaria: confirmar que remover não quebra nada visível (idealmente com o usuário
        // testando/confirmando, ou numa save descartável) antes de considerar isso "provado".
        ["wardrobeforget", cname_hex] => {
            unsafe {
                let target_cname: u64 = match u64::from_str_radix(cname_hex.trim_start_matches("0x"), 16) {
                    Ok(v) => v,
                    Err(_) => { log("[wardrobeforget] CName inválido (espera hex, ex: 8548a2140bb48d1c)"); return; }
                };
                match crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame") {
                    None => log("[wardrobeforget] PlayerPuppet.GetGame não resolvido"),
                    Some(rf) => match crate::rtti::call_func(&rf, player, &[]) {
                        None => log("[wardrobeforget] GetGame() não completou"),
                        Some(r) => {
                            let game_ptr = u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]);
                            if game_ptr == 0 { log("[wardrobeforget] GetGame()=0"); return; }
                            let mut game_arg = [0u8; 16];
                            game_arg[..8].copy_from_slice(&game_ptr.to_le_bytes());
                            match crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetWardrobeSystem") {
                                None => log("[wardrobeforget] GetWardrobeSystem não achado"),
                                Some(rf2) => match crate::rtti::call_func(&rf2, player, &[crate::rtti::Arg::Raw(game_arg)]) {
                                    None => log("[wardrobeforget] GetWardrobeSystem() não completou"),
                                    Some(r2) => {
                                        let sys_ptr = u64::from_le_bytes([r2[0],r2[1],r2[2],r2[3],r2[4],r2[5],r2[6],r2[7]]);
                                        if sys_ptr == 0 { log("[wardrobeforget] WardrobeSystem=0"); return; }
                                        let ok = crate::selftest::wardrobe_forget_item(sys_ptr as *mut u8, target_cname);
                                        log(&format!("[wardrobeforget] CName={target_cname:#018x} removido={ok}"));
                                    }
                                },
                            }
                        }
                    },
                }
            }
            return;
        }
        // Codeware `WardrobeSystem.ForgetItemID` (item #46/#208, 2026-08-11, 3ª rodada de
        // candidatos baratos) — mesma cadeia GetGame/GetWardrobeSystem de `wardrobeforget` acima,
        // mas remove por TDBID (scan-linear por `ItemID.tdbid`, ver
        // `selftest::wardrobe_forget_by_tdbid`) em vez de CName exato — não precisa saber a
        // CHAVE (appearanceName) de antemão, só o TweakDBID do item. Devolve a CONTAGEM removida
        // (pode ser >1 se o mesmo tdbid tiver mais de uma entrada, mesma semântica da fonte real).
        ["wardrobeforgettdbid", tdbid_hex] => {
            unsafe {
                let target_tdbid: u64 = match u64::from_str_radix(tdbid_hex.trim_start_matches("0x"), 16) {
                    Ok(v) => v,
                    Err(_) => { log("[wardrobeforgettdbid] TweakDBID inválido (espera hex, ex: 2c00011165)"); return; }
                };
                match crate::rtti::resolve_func(reg, "PlayerPuppet", "GetGame") {
                    None => log("[wardrobeforgettdbid] PlayerPuppet.GetGame não resolvido"),
                    Some(rf) => match crate::rtti::call_func(&rf, player, &[]) {
                        None => log("[wardrobeforgettdbid] GetGame() não completou"),
                        Some(r) => {
                            let game_ptr = u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]);
                            if game_ptr == 0 { log("[wardrobeforgettdbid] GetGame()=0"); return; }
                            let mut game_arg = [0u8; 16];
                            game_arg[..8].copy_from_slice(&game_ptr.to_le_bytes());
                            match crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetWardrobeSystem") {
                                None => log("[wardrobeforgettdbid] GetWardrobeSystem não achado"),
                                Some(rf2) => match crate::rtti::call_func(&rf2, player, &[crate::rtti::Arg::Raw(game_arg)]) {
                                    None => log("[wardrobeforgettdbid] GetWardrobeSystem() não completou"),
                                    Some(r2) => {
                                        let sys_ptr = u64::from_le_bytes([r2[0],r2[1],r2[2],r2[3],r2[4],r2[5],r2[6],r2[7]]);
                                        if sys_ptr == 0 { log("[wardrobeforgettdbid] WardrobeSystem=0"); return; }
                                        let removed = crate::selftest::wardrobe_forget_by_tdbid(sys_ptr as *mut u8, target_tdbid);
                                        log(&format!("[wardrobeforgettdbid] TDBID={target_tdbid:#018x} removidos={removed}"));
                                    }
                                },
                            }
                        }
                    },
                }
            }
            return;
        }
        // `axl-customization-apply`: usa o ptr salvo por getcustsys/CharCustomA probe para
        // inspecionar e chamar métodos do gameuiCharacterCustomizationSystem.
        // custsys probe → loga ptr + valida via class_of
        // custsys call <method> → callon <ptr_salvo> <method> via RTTI
        ["custsys", action @ ..] => {
            unsafe {
                let ptr = crate::selftest::CHAR_CUSTOM_SYS_PTR.load(std::sync::atomic::Ordering::Relaxed);
                if ptr == 0 {
                    log("[custsys] ptr não capturado — use getcustsys primeiro");
                    return;
                }
                log(&format!("[custsys] ptr={ptr:#018x} action={}", action.join(" ")));
                let obj = ptr as *mut std::ffi::c_void;
                match action {
                    ["probe"] | [] => {
                        let cls = crate::rtti::class_of(obj);
                        log(&format!("[custsys] probe cls={cls:p} valid={}",
                            !cls.is_null()));
                    }
                    ["call", method, raw_args @ ..] => {
                        let cls = crate::rtti::class_of(obj);
                        if cls.is_null() {
                            log(&format!("[custsys] ptr={ptr:#018x} não é objeto RED válido"));
                            return;
                        }
                        // axl-puppet-state-apply (2026-08-03, /goal): suporte a "player" como arg —
                        // passa o handle real do player capturado (Arg::Handle, mesmo padrão de
                        // `give`/console.rs), pra chamar métodos como `HasCharacterCustomizationComponent
                        // (entity: ref<Entity>)` que precisam de um handle de verdade, não um enum/int.
                        let args: Vec<crate::rtti::Arg> = raw_args.iter().map(|a| {
                            if *a == "player" && !player.is_null() {
                                crate::rtti::Arg::Handle(player, crate::console::refcnt())
                            } else {
                                parse_cmd_arg(a)
                            }
                        }).collect();
                        match crate::rtti::resolve_in_class(cls, method) {
                            Some(rf) => match crate::rtti::call_func(&rf, obj, &args) {
                                Some(r) => {
                                    let f = |i: usize| f32::from_bits(u32::from_le_bytes([r[i], r[i+1], r[i+2], r[i+3]]));
                                    log(&format!("[custsys] {method}({}) -> i32 {} / u32 {:#x} / f32 {}",
                                        raw_args.join(" "),
                                        i32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                        u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                        f(0)));
                                    // axl-47/66 (2026-08-18): decodifica também como possível
                                    // DynArray<T> sret {entries(ptr)@0x00, cap(u32)@0x08, size(u32)@0x0C}
                                    // — layout já batalha-testado em todo o resto do projeto (rtti.rs).
                                    // GetHeadOptions()/etc. devolvem array<T>, não escalar; útil pra ler
                                    // ArraySize sem precisar de comando novo dedicado.
                                    let entries = u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]);
                                    let cap = u32::from_le_bytes([r[8],r[9],r[10],r[11]]);
                                    let size = u32::from_le_bytes([r[12],r[13],r[14],r[15]]);
                                    log(&format!("[custsys] {method}(...) como DynArray sret: entries={entries:#018x} cap={cap} size={size}"));
                                }
                                None => log(&format!("[custsys] {method}({}) → void/falha", raw_args.join(" "))),
                            },
                            None => log(&format!("[custsys] método '{method}' não achado na classe")),
                        }
                    }
                    _ => log("[custsys] uso: custsys probe | custsys call <method> [args]"),
                }
            }
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
        // `axl-resource-patch-apply` (2026-07-24): postloadprobe constrói uma instância REAL de
        // EntityTemplate/AppearanceResource/CMesh/MorphTargetMesh (rtti::new_object) e lê o slot
        // PostLoad (+0x30) da vtable própria do objeto — resolve os 4 endereços sem RE estática.
        // Read-only após construir (não chama PostLoad). Rodar no MENU (sem save aberto).
        ["postloadprobe"] => {
            unsafe { crate::selftest::probe_postload_addresses() };
            return;
        }
        // Codeware `#58` (`ISerializable.ProcessPostLoad`/`RefreshResource`): `postloadprobe`
        // acima só LÊ o endereço do slot PostLoad (+0x30), nunca chama. Este comando vai além:
        // constrói uma instância fresca/descartável (mesmo `new_object`, mesma classe já usada
        // com segurança por `postloadprobe` há sessões) e CHAMA `ProcessPostLoad` de verdade
        // (o mesmo código-path exposto ao redscript via `BwmsProcessPostLoad`), dumpando bytes
        // antes/depois pra confirmar mutação real, não só "não crashou". Ex.: `postloadcall CMesh`.
        ["postloadcall", cls] => {
            unsafe { crate::selftest::postload_call_probe(cls) };
            return;
        }
        // RED4ext.SDK #81-85 (`PENDENCIAS-UNIFICADAS.md`): cluster de mapeamento nome-nativo↔
        // nome-script do IRTTISystem (vtbl 0x100/0x108/0x110/0x118/0x120). Read-only exceto o
        // passo 4 (RegisterScriptName sobre a classe forjada "TweakXL", nunca vanilla).
        ["scriptnameprobe"] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::rtti::probe_script_name_mapping(&reg) };
            } else {
                log("[rttiname] Registry indisponível");
            }
            return;
        }
        // RED4ext.SDK #57/#58/#60/#61/#62/#63/#64: cluster de introspecção RTTI em massa
        // (GetNativeTypes/GetGlobalFunctions/GetClassFunctions/GetEnums/GetBitfields/GetClasses/
        // GetDerivedClasses). Só CONTA — read-only.
        ["rttimassprobe"] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::rtti::probe_mass_enumeration(&reg) };
            } else {
                log("[rttimass] Registry indisponível");
            }
            return;
        }
        // RED4ext.SDK #68 (UnregisterType): cria+desregistra um enum de TESTE descartável,
        // autocontido — não toca em nada vanilla nem forjado por outro código.
        ["unregtypeprobe"] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::rtti::probe_unregister_type(&reg) };
            } else {
                log("[rttiunreg] Registry indisponível");
            }
            return;
        }
        // RED4ext.SDK #68 — versão COMPLETA (2026-08-11, disassembly ARM64 achou a causa raiz:
        // UnregisterType nativo só limpa `typesByAsyncId`@+0x40, nunca `types`@+0x10, a estrutura
        // que GetClass/GetEnum checam PRIMEIRO). `unregtypeprobe2` roda o mesmo teste autocontido
        // mas via `unregister_type_complete` (nativo + fastpath manual), num enum de teste
        // SEPARADO (`BwmsUnregisterTestEnum2`) pra não colidir com `unregtypeprobe`.
        ["unregtypeprobe2"] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::rtti::probe_unregister_type_complete(&reg) };
            } else {
                log("[rttiunregfix] Registry indisponível");
            }
            return;
        }
        // RED4ext.SDK #70 (UnregisterFunction): registra+desregistra uma global de TESTE
        // descartável, mesmo padrão autocontido de unregtypeprobe.
        ["unregfnprobe"] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::rtti::probe_unregister_function(&reg) };
            } else {
                log("[rttiunregfn] Registry indisponível");
            }
            return;
        }
        // `CBitfield` (12 métodos, PENDENCIAS-UNIFICADAS.md): lista os bits nomeados de um
        // bitfield já resolvido (vanilla ou forjado via bitfieldprobe).
        ["bitfielddump", name] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::rtti::probe_bitfield_dump(&reg, name) };
            } else {
                log("[bitfielddump] Registry indisponível");
            }
            return;
        }
        // RED4ext.SDK #79 (CreateScriptedBitfield): via oficial, irmã de CreateScriptedEnum.
        ["bitfieldprobe"] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::rtti::probe_create_scripted_bitfield(&reg) };
            } else {
                log("[rttibitfield] Registry indisponível");
            }
            return;
        }
        // RED4ext.SDK #73/#74 (AddRegisterCallback/AddPostRegisterCallback): registra callback
        // void() nosso via Callback<void(*)()> construído à mão. `regcallbackstatus` consulta
        // depois se disparou (pode não disparar nunca — fase de registro do RTTI já passou faz
        // tempo quando um plugin carrega tão tarde quanto o BWMS; ver nota em rtti.rs).
        ["regcallbackprobe"] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::rtti::probe_register_callbacks(&reg) };
            } else {
                log("[rttiregcb] Registry indisponível");
            }
            return;
        }
        ["regcallbackstatus"] => {
            crate::rtti::probe_register_callback_status();
            return;
        }
        // Codeware item #50 (Reflection.GetClasses/GetEnums/GetBitfields — universo filtrado por
        // categoria, composição de get_native_types+type_kind, já provados).
        ["reflenum", kind] => {
            unsafe { crate::rtti::probe_count_and_list_by_kind(reg, kind, 10) };
            return;
        }
        ["reflenum", kind, n] => {
            let n: usize = n.parse().unwrap_or(10).min(100);
            unsafe { crate::rtti::probe_count_and_list_by_kind(reg, kind, n) };
            return;
        }
        // Codeware item #50 (Reflection.GetDerivedClasses — via get_derived_classes já provado,
        // evita a anomalia de GetClasses/#63).
        ["reflderived", base] => {
            unsafe { crate::rtti::probe_reflect_derived(reg, base) };
            return;
        }
        // CET item #44 (DumpVTablesTask, RASCUNHO — ver vtabledump.rs): fatia BOUNDED do
        // universo RTTI, construindo instâncias reais e lendo o ponteiro de vtable. Gated
        // internamente atrás de dev_mode(). `count` sempre limitado (nunca o universo inteiro
        // numa chamada só) — ver doc-comment do módulo pro porquê.
        ["vtabledump", start, count] => {
            let start: usize = start.parse().unwrap_or(0);
            let count: usize = count.parse().unwrap_or(50).min(500);
            unsafe { crate::vtabledump::probe_vtabledump(reg, start, count) };
            return;
        }
        // Codeware `#176` (2026-08-19 tarde/noite): constrói 1 instância NOMEADA (mesmo perfil
        // de risco de `newobj`, endurecido com filtro isAbstract) e dumpa os slots do vtable
        // dela como vmaddr estático, flagando os 2 candidatos já catalogados
        // (0x1049b1c64/0x1049bded0). Ver vtabledump.rs::probe_vtslots.
        ["vtslots", class_name, count] => {
            let count: usize = count.parse().unwrap_or(48).min(200);
            unsafe { crate::vtabledump::probe_vtslots(reg, class_name, count) };
            return;
        }
        // RED4ext.SDK #63 (GetClasses anomalia, cont.134): testa GetClasses/GetDerivedClasses
        // com âncoras MENORES (classe folha) pra ver se a anomalia (universo inteiro em vez de
        // filtrar por ancestralidade) é específica de IScriptable ou geral.
        ["getclassesanomprobe"] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::rtti::probe_getclasses_anomaly(&reg) };
            } else {
                log("[rttiganom] Registry indisponível");
            }
            return;
        }
        // RED4ext.SDK #63 (2026-08-11): compara os PONTEIROS (endereços) dos slots 0x40
        // (GetNativeTypes) vs 0x70 (GetClasses) — testa a hipótese de ICF (Identical Code
        // Folding) do compilador colapsando os 2 métodos na MESMA função, o que explicaria a
        // anomalia sem culpa da nossa marshalling. Read-only, nunca chama através do ponteiro.
        ["getclassesvtid"] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::rtti::probe_getclasses_vtable_identity(&reg) };
            } else {
                log("[rttiganom2] Registry indisponível");
            }
            return;
        }
        // RED4ext.SDK #63 (2026-08-11, sessão de disassembly ARM64 real via rttislot+Capstone):
        // disassembly manual do corpo inteiro de GetClasses/GetDerivedClasses/GetNativeTypes
        // achou a causa raiz — BWMS lê `buf[8..12]` (capacity do DynArray) em vez de `buf[12..16]`
        // (size real). Pra GetClasses (que pré-reserva capacity pro universo INTEIRO antes do
        // filtro de ancestralidade rodar), isso faz o "count" reportado ser sempre o universo,
        // mascarando o filtro real (que a RE confirma existir e funcionar via
        // `IsKindOf@0x10219ac60`). `getclassessize <nome_classe_ancora>` chama os 3 métodos e
        // loga capacity E size lado a lado pra confirmar antes de aplicar o fix.
        ["getclassessize", anchor] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::rtti::probe_getclasses_capacity_vs_size(&reg, anchor) };
            } else {
                log("[rttisize] Registry indisponível");
            }
            return;
        }
        // `rttislot <hex_off>` — irmão GENÉRICO de `crttivtbl` (que só cobre 4 offsets fixos):
        // lê o ponteiro CRU do slot `off` da vtable do IRTTISystem (nunca chama através dele) +
        // computa o vmaddr ESTÁTICO (mesma fórmula de `crttivtbl`). Usado pra alimentar `vtdump`
        // em qualquer offset novo sem precisar de um comando dedicado por item — ex. investigação
        // de disassembly do #63 (`GetClasses`@0x70)/#68 (`UnregisterType`@0x98).
        ["rttislot", off_hex] => {
            let off = usize::from_str_radix(off_hex.trim_start_matches("0x"), 16).unwrap_or(usize::MAX);
            if off == usize::MAX {
                return log("[rttislot] uso: rttislot <hex_off> (ex.: rttislot 98)");
            }
            unsafe {
                match crate::rtti::Registry::obtain() {
                    None => log("[rttislot] Registry::obtain() falhou"),
                    Some(reg) => {
                        let fp = reg.vtbl_slot(off) as usize;
                        if fp == 0 {
                            return log(&format!("[rttislot] +{off:#x} ilegível/null"));
                        }
                        let base = crate::game_base();
                        let vmaddr = if fp > base { fp - base + 0x1_0000_0000 } else { fp };
                        log(&format!("[rttislot] +{off:#x} rt={fp:#x} vmaddr={vmaddr:#010x}"));
                    }
                }
            }
            return;
        }
        // `axl-resource-patch-apply` (2026-07-25): instala hooks observe-only em CMesh::PostLoad
        // (0x100e16b28) + MorphTargetMesh::PostLoad (0x100e467bc). Log primeiros 8 calls de cada
        // tipo com this + campos +0x08..+0x38. Objetivo: RE de layout do CResource (onde fica o
        // resource-path hash) + confirmar que os vmaddrs são realmente PostLoad. Após instalar,
        // esperar meshes carregarem em gameplay (rodar "reslinkdump" pra comparar hashes).
        ["postload-hook"] => {
            unsafe { crate::selftest::install_postload_hooks() };
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
        // TweakXL #30 (`!remove-all`): rmall <field> — limpa o array INTEIRO (não recebe valor,
        // distinto de `!remove`). Gated ~/.bwms-flatwrite (mesma trava de `mkarr`/`mkflat`).
        ["rmall", field] => {
            let on = std::env::var("HOME")
                .ok()
                .map(|h| std::path::Path::new(&h).join(".bwms-flatwrite").exists())
                .unwrap_or(false);
            if !on {
                log("[rmall] BLOQUEADO: crie ~/.bwms-flatwrite p/ habilitar");
                return;
            }
            let id = crate::tweakdb_rt::tweak_db_id(field);
            let ok = match unsafe { crate::tweakdb_rt::singleton() } {
                Some(t) => unsafe { crate::tweakdb_rt::array_remove_all_by_id(t, id) },
                None => {
                    log("[rmall] singleton indisponível");
                    return;
                }
            };
            log(&format!("[rmall] '{field}' -> {ok}"));
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
        // Codeware `#120` (captura DINÂMICA de LR em InkSystem::Get()): traz a janela do jogo pra
        // FRENTE via `[NSApp activateIgnoringOtherApps:YES]` (mesma receita/permissão de
        // `presskey`/`mousedelta` — DENTRO do processo do jogo, não externo). Lição já documentada
        // (2026-08-11, `cw-45-getinventorypuppet`): `presskey` sem foco de janela é NO-OP silencioso
        // (o `CGEventPost` GLOBAL só é postado quando `game_is_frontmost()==true`) — chamar ISTO
        // antes de qualquer `presskey` nesta investigação evita repetir esse bug.
        ["focusgame"] => {
            unsafe { overlay::force_game_frontmost() };
            log("[focusgame] force_game_frontmost() chamado");
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
        //
        // NÃO MIGRADO pro canal do heartbeat (2026-08-14, decisão desta rodada — ao contrário
        // de `inkgetbaseline`/`inkgetpost`/`checkresbaseline`/`checkrespost`, que SÃO leitura
        // pura e já rodam também na thread do heartbeat, ver `lib.rs` ~526/~548): `cg_press`
        // (mesmo sendo "thread-safe" no sentido de não tocar RTTI/VM do jogo, ver comentário em
        // `overlay.rs`) INJETA um evento de input REAL — mutação com efeito genuíno no jogo
        // vivo, categoria de risco diferente de um drain de ring-buffer. Rodar via heartbeat
        // (a) perderia o gate `PHASE_REACHED_5 && !exec_nested()` deste canal — o jogo poderia
        // não estar pronto pra receber input com segurança ainda; (b) chamaria
        // `CGEventSourceCreate`/`CGEventPost` (e `focusgame`/`force_game_frontmost` chamaria
        // `NSApplication.activateIgnoringOtherApps:`) de uma thread Rust própria cuja relação com
        // a main-thread AppKit do processo nunca foi verificada — AppKit não garante
        // thread-safety fora da main thread, risco novo nunca testado neste projeto. Mesma
        // categoria de cautela já aplicada ao `checkreshook` (patch de código executável também
        // ficou de fora da migração). `presskey`/`focusgame`/`mousedelta`/`mouseclick` continuam
        // só no canal do executor (game thread).
        #[cfg(feature = "autoproceed")]
        ["presskey", kc] => {
            let keycode = kc.parse::<u16>().unwrap_or(49);
            unsafe { overlay::cg_press(keycode) };
            log(&format!("[presskey] disparado keyDown+keyUp sintético, keycode={keycode}"));
            return;
        }
        // `keydown`/`keyup` — metades ISOLADAS de `presskey` (ver `overlay::cg_key_event`),
        // pra permitir HOLD sustentado (ex. andar pra frente) controlado por um processo
        // EXTERNO (bridge de IA por visão) via 2 comandos + sleep do lado de FORA, sem bloquear
        // a thread do jogo. `automacao-mundo` (2026-08-17).
        #[cfg(feature = "autoproceed")]
        ["keydown", kc] => {
            let keycode = kc.parse::<u16>().unwrap_or(49);
            unsafe { overlay::cg_key_event(keycode, true) };
            log(&format!("[keydown] disparado, keycode={keycode}"));
            return;
        }
        #[cfg(feature = "autoproceed")]
        ["keyup", kc] => {
            let keycode = kc.parse::<u16>().unwrap_or(49);
            unsafe { overlay::cg_key_event(keycode, false) };
            log(&format!("[keyup] disparado, keycode={keycode}"));
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
        // `CallbackLifetime.Session` (2026-08-11): dispara "Session/End" manualmente (mesmo
        // GameSessionEvent real que o despawn de player já usa em lib.rs) — testa o sweep sem
        // precisar navegar até o menu de verdade. Diagnóstico dev-only, seguro (mesma função já
        // provada 2x em produção: despawn + clean-exit).
        ["firesessionend"] => {
            let n = if let Some(arg) = unsafe { register::make_gamesessionevent_arg(false, true) } {
                unsafe { register::fire_event_args("Session/End", &[arg]) }
            } else {
                0
            };
            log(&format!("[firesessionend] Session/End disparado manualmente -> {n} callback(s)"));
            return;
        }
        // CET `FunctionOverride` (não-Lua, `fnoverride.rs`, 2026-08-11): registra Before+After
        // no método de teste `PlayerPuppet.OnFnOverrideTestTarget` (declarado no .reds de teste
        // temporário). Rodar `callon <ptr> OnFnOverrideTestTarget` antes e depois pra comparar.
        ["fnoverridetest"] => {
            let ok = unsafe {
                crate::fnoverride::register(
                    reg,
                    "PlayerPuppet",
                    "OnFnOverrideTestTarget",
                    crate::fnoverride::Phase::Before,
                    fnoverride_test_before,
                ) && crate::fnoverride::register(
                    reg,
                    "PlayerPuppet",
                    "OnFnOverrideTestTarget",
                    crate::fnoverride::Phase::After,
                    fnoverride_test_after,
                )
            };
            log(&format!("[fnoverridetest] registro Before+After -> {ok}"));
            return;
        }
        // Adiciona um Replace por cima do Before/After já registrado — só o ÚLTIMO Replace
        // registrado roda (regra V1 documentada em fnoverride.rs), escreve 999 no aOut.
        ["fnoverridereplace"] => {
            let ok = unsafe {
                crate::fnoverride::register(
                    reg,
                    "PlayerPuppet",
                    "OnFnOverrideTestTarget",
                    crate::fnoverride::Phase::Replace,
                    fnoverride_test_replace,
                )
            };
            log(&format!("[fnoverridereplace] registro Replace -> {ok}"));
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
        // RED4ext.SDK #43 (`CompareSemVerPrerelease`): prova ao vivo do C-ABI v14 exposto em
        // BwmsApi (mesma via que um plugin de 3os chamaria) + do Require() real dos 3 enablers.
        // `semvercmp 1.2.3-rc1 1.2.3` deve dar cmp=-1 (release > pre-release, o bug original).
        ["semvercmp", lhs, rhs] => {
            let cmp = unsafe { (crate::api::BWMS_API.compare_semver_prerelease)(
                std::ffi::CString::new(*lhs).unwrap_or_default().as_ptr(),
                std::ffi::CString::new(*rhs).unwrap_or_default().as_ptr(),
            ) };
            let sat = unsafe { (crate::api::BWMS_API.semver_satisfies)(
                std::ffi::CString::new(*rhs).unwrap_or_default().as_ptr(),
                std::ffi::CString::new(*lhs).unwrap_or_default().as_ptr(),
            ) };
            log(&format!(
                "[semvercmp] compare_semver_prerelease('{lhs}','{rhs}')={cmp} | semver_satisfies(required='{rhs}',actual='{lhs}')={sat}"
            ));
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
        // cw-reflection-class-api proof: Reflection.GetClass + probe de props sem file gate
        ["refltest"] => {
            unsafe {
                let cls = crate::rtti::class_of(player);
                log(&crate::rtti::reflection_probe_cls(cls, "refltest:player-class", player));
            }
            return;
        }
        // cw-event-target-classes proof: KeyInputEvent.GetKey/GetAction round-trip
        ["keytest"] => {
            unsafe { register::run_keyinput_test(&reg); }
            return;
        }
        // cw-entity-builder proof: BwmsEntityAddComponent via dynarray_push_handle16
        ["entitytest"] => {
            unsafe { register::run_entity_builder_test(player); }
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
            _ => log(&format!("[console] CET '{cmd}' -> give {name} x{n} FAILED")),
        }
        return;
    }
    let r = unsafe {
        match parts.as_slice() {
            ["money", n] => console::give(reg, player, tx, "Items.money", n.parse().unwrap_or(1)),
            // RAM do cyberdeck. `ram` enche uma vez; `ram on|off` mantém cheia (ver RAM_INFINITE).
            // "NPC'ler görmesin": camuflagem óptica do jogo (BaseStatusEffect.Cloaked).
            ["cloak"] | ["cloak", "on"] => {
                let ok = console::cloak(reg, player, true);
                log(&format!("[console] 'cloak' -> {}", if ok { "ON" } else { "FAILED" }));
                return;
            }
            ["cloak", "off"] => {
                let ok = console::cloak(reg, player, false);
                log(&format!("[console] 'cloak off' -> {}", if ok { "OFF" } else { "FAILED" }));
                return;
            }
            // Munição infinita (sem recarregar): GameplayRestriction.InfiniteAmmo.
            ["ammo"] | ["ammo", "on"] => {
                let ok = console::infinite_ammo(reg, player, true);
                log(&format!("[console] 'ammo' -> {}", if ok { "ON" } else { "FAILED" }));
                return;
            }
            ["ammo", "off"] => {
                let ok = console::infinite_ammo(reg, player, false);
                log(&format!("[console] 'ammo off' -> {}", if ok { "OFF" } else { "FAILED" }));
                return;
            }
            // `enumv <Enum>`: membros + VALORES. Um valor errado vira uma chamada silenciosa
            // que não faz nada — foi exatamente a suspeita sobre `gameGodModeType`.
            ["enumv", name] => {
                unsafe {
                    let v = crate::rtti::list_enum(reg, name);
                    if v.is_empty() {
                        log(&format!("[funcs] enum '{name}' não existe ou está vazio"));
                    } else {
                        log(&format!("[funcs] {name}: {} membro(s)", v.len()));
                        for (n, val) in v.iter().take(40) {
                            log(&format!("[funcs]   {n} = {val}"));
                        }
                    }
                }
                return;
            }
            // `findfunc <filtro>`: procura no registro GLOBAL de funções. Par de `funcs`, que
            // olha dentro de uma classe — juntos cobrem os dois lugares onde um método pode estar.
            ["findfunc", filter] => { list_global_funcs(reg, filter); return; }
            // `funcs <Classe> [filtro]`: lista os nomes REAIS das funções de uma classe na
            // RTTI. Existe porque "não resolveu" custava uma recompilação pra descobrir como o
            // método se chama — as funções de redscript entram com a assinatura colada no nome.
            ["funcs", class] | ["funcs", class, ""] => { list_class_funcs(reg, class, ""); return; }
            ["funcs", class, filter] => { list_class_funcs(reg, class, filter); return; }
            // `stat`/`statw <Nome> [valor]`: modificador de stat no JOGADOR ou na ARMA ativa,
            // com leitura antes/depois. É o mecanismo por trás de `cloak` e `ammo` — exposto
            // cru pra testar um stat candidato sem recompilar.
            ["stat", name] => { console::stat_mod(reg, player, name, 1.0, console::StatTarget::Player, "stat"); return; }
            ["stat", name, v] => {
                let val: f32 = v.parse().unwrap_or(1.0);
                console::stat_mod(reg, player, name, val, console::StatTarget::Player, "stat");
                return;
            }
            ["statw", name] => { console::stat_mod(reg, player, name, 1.0, console::StatTarget::Weapon, "stat"); return; }
            ["statw", name, v] => {
                let val: f32 = v.parse().unwrap_or(1.0);
                console::stat_mod(reg, player, name, val, console::StatTarget::Weapon, "stat");
                return;
            }
            ["stat?", name] => { console::stat_read(reg, player, name, console::StatTarget::Player); return; }
            ["statw?", name] => { console::stat_read(reg, player, name, console::StatTarget::Weapon); return; }
            ["stat-", name] => { console::stat_unmod(reg, player, name, "stat"); return; }
            // `se <record> [on|off]` / `se? <record>`: status effect ARBITRÁRIO por nome.
            // Testar um candidato passa a custar uma linha digitada em vez de recompilar +
            // reiniciar o jogo, que é o ciclo caro deste port.
            ["se", name] | ["se", name, "on"] => {
                console::status_effect_named(reg, player, name, true);
                return;
            }
            ["se", name, "off"] => {
                console::status_effect_named(reg, player, name, false);
                return;
            }
            ["se?", name] | ["sehas", name] => {
                console::status_effect_query(reg, player, name);
                return;
            }
            ["ver"] | ["version"] => {
                log(&format!("[build] stamp={} (compare com o do instalador)", build_stamp()));
                return;
            }
            ["ram"] => {
                let ok = console::ram(reg, player);
                log(&format!("[console] 'ram' -> {}", if ok { "OK (Memory=100)" } else { "FAILED" }));
                return;
            }
            ["ram", sw @ ("on" | "off")] => {
                let on = *sw == "on";
                set_ram_infinite(on);
                if on {
                    console::ram(reg, player);
                }
                log(&format!("[console] 'ram {sw}' -> unlimited RAM {}", if on { "ON" } else { "OFF" }));
                return;
            }
            ["give", name] => console::give(reg, player, tx, name, 1),
            ["give", name, n] => console::give(reg, player, tx, name, n.parse().unwrap_or(1)),
            ["remove", name] => console::remove(reg, player, tx, name, 1),
            ["remove", name, n] => console::remove(reg, player, tx, name, n.parse().unwrap_or(1)),
            // `cw-world-depot` (2026-07-24): `spawnsub <tdbid-or-path> [x y z]` — atalho pragmático
            // via `CompanionSystem.SpawnSubcharacterOnPosition` real (ver `console::spawn_subcharacter`).
            // Não testado em boot ainda.
            ["spawnsub", name] => console::spawn_subcharacter(reg, player, name, (0.0, 0.0, 0.0)),
            ["spawnsub", name, x, y, z] => console::spawn_subcharacter(
                reg,
                player,
                name,
                (x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0), z.parse().unwrap_or(0.0)),
            ),
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
        Some(res) => {
            // `GiveItem` devolvendo 0 quase sempre é o ID e não o contexto. O caso mais comum é o
            // usuário escrever o nome sem o namespace (`give money`, `give katana`) — TweakDBID é
            // o hash do nome INTEIRO, então `money` simplesmente não existe e o engine devolve 0
            // sem erro. Culpar "owner/tx" mandava o usuário procurar no lugar errado; dizer o que
            // provavelmente está errado e como escrever certo resolve na primeira leitura.
            let name = parts.get(1).copied().unwrap_or("");
            let hint = if !name.is_empty() && !name.contains('.') {
                format!(" - '{name}' has no namespace; try 'Items.{name}'")
            } else if !name.is_empty() {
                format!(" - is '{name}' a record in this build's TweakDB?")
            } else {
                String::new()
            };
            log(&format!(
                "[console] '{cmd}' -> NO-OP (GiveItem returned {}){hint}",
                res[0]
            ))
        }
        None => log(&format!("[console] '{cmd}' -> FAILED (resolve/from_tdbid/call)")),
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

#[cfg(test)]
mod null_vtable_guard_tests {
    use super::null_vtable_is_anomalous;

    // O caso que a assinatura de crash desta sessão descreve: objeto polimórfico com o ponteiro
    // de vtable zerado. Qualquer despacho virtual nele lê `[0 + slot]` e falta no offset do slot.
    #[test]
    fn objeto_polimorfico_com_vtable_zerada_e_anomalia() {
        assert!(null_vtable_is_anomalous(true, 0));
    }

    #[test]
    fn objeto_polimorfico_com_vtable_valida_nao_e_anomalia() {
        assert!(!null_vtable_is_anomalous(true, 0x1_0472_d218));
    }

    // A razão de a checagem NÃO poder ser um `*obj == 0` cego: num struct puro o offset 0 é dado.
    // `Vector4 { x: 0.0, .. }` tem a primeira word zerada e é perfeitamente válido — acusá-lo
    // encheria o log de falso-positivo em cima do caminho mais comum de construção de struct.
    #[test]
    fn struct_puro_com_primeira_word_zerada_nao_e_anomalia() {
        assert!(!null_vtable_is_anomalous(false, 0));
    }

    #[test]
    fn struct_puro_com_primeira_word_nao_zerada_tambem_nao_e_anomalia() {
        assert!(!null_vtable_is_anomalous(false, 0x3f80_0000));
    }
}


/// Imprime no console os nomes das funções de uma classe (filtro opcional por substring).
/// Suporte de `funcs <Classe> [filtro]`; corta em 60 linhas pra não afogar o buffer.
fn list_class_funcs(reg: &crate::rtti::Registry, class: &str, filter: &str) {
    unsafe {
        let cls = reg.class_by_name(class);
        if cls.is_null() {
            log(&format!("[funcs] classe '{class}' não existe na RTTI"));
            return;
        }
        let mut v = crate::rtti::list_functions(cls, filter);
        v.sort();
        v.dedup();
        log(&format!("[funcs] {class}: {} função(ões){}", v.len(),
            if filter.is_empty() { String::new() } else { format!(" com '{filter}'") }));
        for n in v.iter().take(60) {
            log(&format!("[funcs]   {n}"));
        }
        if v.len() > 60 {
            log(&format!("[funcs]   ... e mais {} (use um filtro)", v.len() - 60));
        }
    }
}


/// Imprime funções GLOBAIS cujo nome contém `filter`. Suporte de `findfunc <filtro>`.
fn list_global_funcs(reg: &crate::rtti::Registry, filter: &str) {
    unsafe {
        let mut v = crate::rtti::list_global_functions(reg, filter);
        v.sort();
        v.dedup();
        log(&format!("[funcs] globais com '{filter}': {}", v.len()));
        for n in v.iter().take(60) {
            log(&format!("[funcs]   {n}"));
        }
        if v.len() > 60 {
            log(&format!("[funcs]   ... e mais {} (refine o filtro)", v.len() - 60));
        }
    }
}
