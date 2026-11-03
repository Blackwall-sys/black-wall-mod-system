//! Mods do usuário: achar, carregar, versão e log dos mods instalados, API de plugin e compatibilidade de script.

#![allow(dead_code)]

pub(crate) mod log_de_mod;

pub(crate) mod api;

pub(crate) mod plugins;

pub(crate) mod pipeline;

pub(crate) mod pasta_mods;

pub(crate) mod mod_io;

pub(crate) mod lista_dos_mods;

pub(crate) mod pasta_nativa;


use crate::dylib_dir;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::ffi::c_void;
use crate::rtti::{self};
use crate::register::*;

/// Os plugues de framework que o BWMS atende pela porta `<classe>.Version()`/`<classe>.Require(v)`, cada um com a versão da
/// referência contra a qual o catálogo dele fechou (`_referencia-de-terceiro/enablers/<classe>`). `Version()` devolve a versão do
/// BWMS, que é quem responde; `Require(v)` aceita v ≤ a versão coberta, e acima disso o mod pede mais do que o BWMS cobre e o
/// BWMS precisa ser atualizado (decisão do dono de 2026-10-03, `proofs/20261003-refino-r33-plugues-respondem-pelo-bwms.log`).
/// TweakXL 1.12.0 pelo delta de 1.11.3 (2026-10-27): os flats extras novos da 2.5 estão no pool de scripts que o BWMS lê, o hash de tipo de
/// record já sai do nome, e o mapa de herança da 2.5 vai no pacote (`red4ext/bwms-heranca.yaml`). Codeware 1.21.1 e ArchiveXL 1.28.1 pelo delta
/// dos commits até elas, provado nas lanes H, I e G de 2026-10-27/28 (`proofs/20261028-lane-I-*`, `proofs/20261028-lane-G-2-*`).
pub(crate) const VERSOES_COBERTAS: [(&str, &str); 3] = [("Codeware", "1.21.1"), ("ArchiveXL", "1.28.6"), ("TweakXL", "1.12.0")];

/// A chave de cada plugue em `VERSOES_COBERTAS`. O registro da fachada a passa para `tramp_version`/`tramp_require`, porque o
/// handler de native recebe só o frame e não sabe de qual classe foi chamado.
pub(crate) const PLUGUE_CODEWARE: usize = 0;
pub(crate) const PLUGUE_ARCHIVEXL: usize = 1;
pub(crate) const PLUGUE_TWEAKXL: usize = 2;

/// Resposta do `Require`: a versão coberta alcança a pedida pela precedência SemVer 2.0 completa (`api::semver_precedence`).
pub(crate) fn cobertura_atende(coberta: &str, pedida: &str) -> bool {
    crate::mods_do_usuario::api::semver_precedence(coberta, pedida) != std::cmp::Ordering::Less
}

/// `<classe>.Version() -> String` do plugue `PLUGUE`: a versão do BWMS.
pub(crate) unsafe extern "C" fn tramp_version<const PLUGUE: usize>(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    // `const`: uma chave fora da tabela para o build (E0080 na monomorfização; o `cargo check` não pega), em vez de virar
    // pânico dentro de um handler `extern "C"`.
    let (classe, _) = const { VERSOES_COBERTAS[PLUGUE] };
    crate::log_dev!("[reg] >>> {classe}.Version chamado");
    if !out.is_null() {
        std::ptr::write_bytes(out as *mut u8, 0, 0x20);
        rtti::red_string_write_inline(out as *mut u8, crate::BWMS_VERSION);
    }
}

/// `<classe>.Require(version: String) -> Bool` do plugue `PLUGUE`: lê o argumento por `read_params_consuming_with_strings` e
/// responde `cobertura_atende` contra a versão coberta. Sem argumento legível, a pedida é vazia e conta como 0.0.0.
pub(crate) unsafe extern "C" fn tramp_require<const PLUGUE: usize>(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (classe, coberta) = const { VERSOES_COBERTAS[PLUGUE] };
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let requested = strings.first().cloned().flatten().unwrap_or_default();
    let ok = cobertura_atende(coberta, &requested);
    crate::log_dev!("[reg] >>> {classe}.Require('{requested}') vs nosso '{coberta}' -> {ok}");
    write_bool_ret(out, ok);
}

pub(crate) unsafe extern "C" fn tramp_print(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let text = strings.first().cloned().flatten().unwrap_or_default();
    crate::mods_do_usuario::log_de_mod::gravar("DEBUG", &text);
}

pub(crate) unsafe extern "C" fn tramp_modlog(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let mod_hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let mod_name = crate::cname::resolve_cname(mod_hash);
    let text = strings.get(1).cloned().flatten().unwrap_or_default();
    crate::mods_do_usuario::log_de_mod::gravar(&mod_name, &text);
}


/// A luz de um mod no selo do canto, na aba Mods e no relatório: verde entrou inteiro, amarelo entrou em parte, vermelho não funciona, cinza
/// fora do jogo (plugin inativo, mod do mod.io desligado no menu ou sem estado legível). O discriminante é o índice em `MODS_POR_ESTADO`, a
/// posição na tupla de `mod_counts` e a da cor no selo (`inicio::splash::CORES_DAS_LUZES`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModStatus {
    Ok = 0,
    Warning = 1,
    Error = 2,
    Inactive = 3,
}

impl ModStatus {
    /// O nome da luz no relatório, em PT-BR e inglês.
    pub(crate) fn nome_da_luz(self) -> &'static str {
        match self {
            ModStatus::Ok => "verde / green",
            ModStatus::Warning => "amarelo / yellow (entrou em parte / partly loaded)",
            ModStatus::Error => "vermelho / red (não funciona / does not work)",
            ModStatus::Inactive => "cinza / gray (fora do jogo / not loaded)",
        }
    }
}

impl From<pasta_mods::CorDoMod> for ModStatus {
    fn from(c: pasta_mods::CorDoMod) -> Self {
        match c {
            pasta_mods::CorDoMod::Verde => ModStatus::Ok,
            pasta_mods::CorDoMod::Amarelo => ModStatus::Warning,
            pasta_mods::CorDoMod::Vermelho => ModStatus::Error,
        }
    }
}

/// Plugins `.dylib` carregados por estado. `register_mod` é o único escritor (carga dos plugins, `plugins.rs`) e o selo do canto lê a cada
/// quadro da gameplay: contador atômico lido sem trava e sem varrer lista.
static MODS_POR_ESTADO: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];

pub(crate) fn register_mod(status: ModStatus) {
    MODS_POR_ESTADO[status as usize].fetch_add(1, Ordering::Relaxed);
}

/// Os plugins `.dylib` por luz, `[ok, aviso, erro, inativo]`.
pub(crate) fn contagem_dos_plugins() -> [usize; 4] {
    MODS_POR_ESTADO.each_ref().map(|n| n.load(Ordering::Relaxed))
}

/// Põe os contadores dos plugins em `n` e devolve os de antes: a prova visual do terminal desenha a aba Mods ("plugins N") com um número
/// fixo, e o teste do `register_mod` soma no mesmo processo. Quem chama segura `vr::painel::UM_POR_VEZ`.
#[cfg(test)]
pub(crate) fn trocar_contagem_dos_plugins(n: [usize; 4]) -> [usize; 4] {
    [0, 1, 2, 3].map(|i| MODS_POR_ESTADO[i].swap(n[i], Ordering::Relaxed))
}

/// `(ok, aviso, erro, inativo)`, lido pelo selo do canto a cada quadro: os plugins `.dylib` mais os mods dos 2 caminhos (a `Mods/` e o
/// mod.io, `lista_dos_mods::contagem`), só contadores atômicos. `Relaxed`: cada contador é independente, e uma leitura no meio da carga só
/// adianta ou atrasa um quadro do selo.
pub(crate) fn mod_counts() -> (usize, usize, usize, usize) {
    let p = contagem_dos_plugins();
    let c = lista_dos_mods::contagem();
    (p[0] + c[0], p[1] + c[1], p[2] + c[2], p[3] + c[3])
}

/// Pasta de mods.
pub(crate) fn mods_dir() -> String {
    // 1) PORTÁVEL: <pasta da nossa dylib>/blackwall-mods (a dylib mora em <jogo>/red4ext/ ao lado
    //    de blackwall-mods/). Funciona em qualquer máquina. Se o subdir existir, usa direto; senão
    //    CONFIA na localização mesmo assim (a dylib SEMPRE carrega de red4ext/) — evita embutir um
    //    literal de path do jogo no binário (TRACELESS).
    if let Some(dir) = dylib_dir() {
        return format!("{dir}/blackwall-mods");
    }
    // 2) dladdr falhou (não deveria acontecer): caminho relativo ao cwd, sem embutir path do jogo.
    "red4ext/blackwall-mods".to_string()
}

#[cfg(test)]
mod testes_dos_plugues {
    use super::*;

    #[test]
    fn chave_de_cada_plugue_aponta_a_classe_e_a_versao_decididas() {
        assert_eq!(VERSOES_COBERTAS[PLUGUE_CODEWARE], ("Codeware", "1.21.1"));
        assert_eq!(VERSOES_COBERTAS[PLUGUE_ARCHIVEXL], ("ArchiveXL", "1.28.6"));
        assert_eq!(VERSOES_COBERTAS[PLUGUE_TWEAKXL], ("TweakXL", "1.12.0"));
    }

    /// Os pares que o R33 mediu ao vivo (`proofs/20261003-refino-r33-plugues-respondem-pelo-bwms.log`); os do TweakXL andaram com a cobertura.
    #[test]
    fn require_responde_pela_cobertura_como_no_r33() {
        let casos = [
            (PLUGUE_ARCHIVEXL, "1.26.0", true),
            (PLUGUE_ARCHIVEXL, "1.28.6", true),
            (PLUGUE_ARCHIVEXL, "1.28.7", false),
            (PLUGUE_TWEAKXL, "1.12.0", true),
            (PLUGUE_TWEAKXL, "1.12.1", false),
            (PLUGUE_CODEWARE, "1.20.2", true),
            (PLUGUE_CODEWARE, "1.21.1", true),
            (PLUGUE_CODEWARE, "1.22.0", false),
        ];
        for (plugue, pedida, esperado) in casos {
            let (classe, coberta) = VERSOES_COBERTAS[plugue];
            assert_eq!(cobertura_atende(coberta, pedida), esperado, "{classe}.Require('{pedida}') contra '{coberta}'");
        }
    }

    /// Único teste que chama `register_mod`: os contadores são do processo, então ele mede pela diferença, com a trava da prova visual do
    /// terminal, que os zera durante o desenho.
    #[test]
    fn register_mod_conta_cada_estado_na_sua_posicao() {
        let _trava = crate::vr::painel::UM_POR_VEZ.lock().unwrap_or_else(|e| e.into_inner());
        let antes = contagem_dos_plugins();
        for status in [ModStatus::Ok, ModStatus::Warning, ModStatus::Warning, ModStatus::Error, ModStatus::Inactive, ModStatus::Inactive, ModStatus::Inactive] {
            register_mod(status);
        }
        let depois = contagem_dos_plugins();
        assert_eq!([depois[0] - antes[0], depois[1] - antes[1], depois[2] - antes[2], depois[3] - antes[3]], [1, 2, 1, 3]);
    }

    #[test]
    fn require_segue_a_precedencia_semver_completa() {
        assert!(cobertura_atende("1.20.2", "1.20.2-rc.1"), "release alcança o próprio pre-release");
        assert!(!cobertura_atende("1.20.2-rc.1", "1.20.2"));
        assert!(cobertura_atende("1.20.2", "v1.20.2+build.7"), "prefixo v e metadado de build não pesam");
        assert!(cobertura_atende("1.20.2", ""), "pedida vazia conta como 0.0.0");
    }

    // Fora do `devlog`: lá o handler grava `[reg] >>> <classe>.Version chamado` no /tmp/cp77-console.log, o mesmo arquivo do
    // jogo, e um `cargo test` durante um boot poria depois do marcador uma linha que o jogo não emitiu.
    #[cfg(not(feature = "devlog"))]
    #[test]
    fn version_dos_tres_plugues_devolve_a_versao_do_bwms() {
        unsafe fn versao_por(handler: crate::register::NativeHandler) -> String {
            let mut slot = [0xAAu8; 0x20];
            handler(std::ptr::null_mut(), std::ptr::null_mut(), slot.as_mut_ptr() as *mut c_void, 0);
            rtti::red_string_read(slot.as_ptr())
        }
        unsafe {
            assert_eq!(versao_por(tramp_version::<PLUGUE_CODEWARE>), crate::BWMS_VERSION);
            assert_eq!(versao_por(tramp_version::<PLUGUE_ARCHIVEXL>), crate::BWMS_VERSION);
            assert_eq!(versao_por(tramp_version::<PLUGUE_TWEAKXL>), crate::BWMS_VERSION);
            tramp_version::<PLUGUE_CODEWARE>(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0);
        }
    }
}


