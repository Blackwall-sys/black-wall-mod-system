//! O que o BWMS faz no boot, lido de `<jogo>/red4ext/bwms.toml`.
//!
//! Por que existe: até 2026-09-20 o splash, o skip dos logos e o auto-continue dependiam de arquivos
//! OCULTOS no home do usuário (`~/.bwms-skipintro` e companhia) que **o instalador nunca criava** —
//! ele só os apagava no `--restore`. O resultado é que quem instalava o mod via os logos da CDPR e a
//! tela preta de loading, exatamente como sem mod, e o splash que é a cara do projeto não aparecia
//! pra ninguém. O relatório de crash de um usuário mostrava isso preto no branco:
//! `Skip mode markers: none`.
//!
//! Agora é um TOML só, ao lado do dylib, escrito pelo instalador com os padrões e um comentário por
//! opção. No build de dev os marcadores do home valem POR CIMA, que é como o ciclo de teste liga e
//! desliga cada peça sem reinstalar nada; o público lê só o TOML.

use std::sync::OnceLock;

/// O que o usuário pediu pro boot. Lido uma vez por processo.
#[derive(Clone, Copy)]
pub(crate) struct ConfigDeBoot {
    /// Pula os logos de abertura e a tela "APERTE ESPAÇO". Ligado por padrão: ninguém instala um mod pra ver logo de publisher.
    pub pular_intro: bool,
    /// Desenha a tela do BWMS por cima da tela preta de loading. Ligado por padrão — é a cara do mod.
    pub splash: bool,
    /// Carrega o último save sozinho, sem passar pelo menu. Ligado por padrão (decisão do dono, 2026-10-05): o mod instalado
    /// abre o jogo e vai até a gameplay. Quem quiser parar no menu escolhe "Até o menu" no seletor "Pular boot" ou grava
    /// `false` aqui. Sem save nenhum, o boot para no menu.
    pub continuar_automatico: bool,
}

impl Default for ConfigDeBoot {
    fn default() -> Self {
        Self { pular_intro: true, splash: true, continuar_automatico: true }
    }
}

/// O texto que o instalador grava. Também é o que documenta as opções pro usuário — não existe outro
/// lugar onde ele descubra que elas existem.
pub(crate) const MODELO: &str = "\
# BWMS — o que o mod faz quando o jogo abre.
# Apague uma linha pra voltar ao padrão; apague o arquivo inteiro pra voltar a todos eles.

# Pula os logos de abertura e a tela \"APERTE ESPAÇO\" (a tela preta de loading continua, é streaming de disco).
pular_intro = true

# Desenha a tela do BWMS por cima da tela preta de loading, com a barra de progresso.
splash = true

# Carrega o último save sozinho, sem passar pelo menu principal.
continuar_automatico = true
";

fn arquivo() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(crate::dylib_dir()?).join("bwms.toml"))
}

/// Lê um `chave = true/false` do TOML. Formato deliberadamente pequeno: este arquivo é escrito por
/// um humano num editor de texto, e um parser maior seria mais superfície pra erro do que serventia.
fn bool_do_toml(txt: &str, chave: &str) -> Option<bool> {
    for linha in txt.lines() {
        let l = linha.trim();
        if l.starts_with('#') {
            continue;
        }
        let Some((nome, valor)) = l.split_once('=') else { continue };
        if nome.trim() != chave {
            continue;
        }
        return match valor.split('#').next().unwrap_or("").trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
    }
    None
}

/// Lê um `chave = "texto"` do TOML (entre aspas duplas).
pub(crate) fn texto_do_toml(txt: &str, chave: &str) -> Option<String> {
    for linha in txt.lines() {
        let l = linha.trim();
        if l.starts_with('#') {
            continue;
        }
        let Some((nome, valor)) = l.split_once('=') else { continue };
        if nome.trim() != chave {
            continue;
        }
        let v = valor.trim();
        let v = v.strip_prefix('"')?;
        return v.split_once('"').map(|(dentro, _)| dentro.to_string());
    }
    None
}

/// O TOML com `chave = <valor_toml>` (o valor já formatado: `"texto"` ou `true`): troca a linha da chave se ela existe, senão acrescenta `comentario` e a linha no fim. O resto do arquivo
/// (as outras chaves e os comentários do usuário) fica como estava.
pub(crate) fn com_chave(txt: &str, chave: &str, valor_toml: &str, comentario: &str) -> String {
    let nova = format!("{chave} = {valor_toml}");
    let mut trocou = false;
    let mut linhas: Vec<String> = txt
        .lines()
        .map(|linha| {
            let l = linha.trim();
            let e_a_chave = !l.starts_with('#') && l.split_once('=').is_some_and(|(nome, _)| nome.trim() == chave);
            if e_a_chave && !trocou {
                trocou = true;
                nova.clone()
            } else {
                linha.to_string()
            }
        })
        .collect();
    if !trocou {
        if linhas.last().is_some_and(|l| !l.trim().is_empty()) {
            linhas.push(String::new());
        }
        linhas.push(format!("# {comentario}"));
        linhas.push(nova);
    }
    let mut saida = linhas.join("\n");
    saida.push('\n');
    saida
}

/// Texto de uma chave do `bwms.toml` do jogo, lido agora (sem o cache do `config`).
pub(crate) fn ler_texto(chave: &str) -> Option<String> {
    texto_do_toml(&std::fs::read_to_string(arquivo()?).ok()?, chave)
}

/// `chave = true/false` do `bwms.toml` do jogo, lido agora (sem o cache do `config`).
pub(crate) fn ler_bool(chave: &str) -> Option<bool> {
    bool_do_toml(&std::fs::read_to_string(arquivo()?).ok()?, chave)
}

/// Grava `chave = <valor_toml>` no `bwms.toml` do jogo (cria o arquivo se não existe). Escreve num temporário e troca, para um erro no
/// meio não deixar o arquivo do usuário pela metade.
fn gravar(chave: &str, valor_toml: &str, comentario: &str) -> std::io::Result<()> {
    let caminho = arquivo().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "pasta do BWMS não encontrada"))?;
    let atual = std::fs::read_to_string(&caminho).unwrap_or_default();
    let temporario = caminho.with_extension("toml.gravando");
    std::fs::write(&temporario, com_chave(&atual, chave, valor_toml, comentario))?;
    std::fs::rename(&temporario, &caminho)
}

/// Grava `chave = "valor"` (texto entre aspas).
pub(crate) fn gravar_texto(chave: &str, valor: &str, comentario: &str) -> std::io::Result<()> {
    gravar(chave, &format!("\"{valor}\""), comentario)
}

/// Grava `chave = true/false`.
pub(crate) fn gravar_bool(chave: &str, valor: bool, comentario: &str) -> std::io::Result<()> {
    gravar(chave, if valor { "true" } else { "false" }, comentario)
}

/// Marcador do home (`~/.bwms-<nome>`): no dev vale por cima do TOML, pra ligar uma peça num boot de teste sem editar arquivo no
/// jogo. No público é sempre `false`.
fn marcador(nome: &str) -> bool {
    if !cfg!(feature = "devtools") {
        return false;
    }
    std::env::var("HOME")
        .map(|h| std::path::Path::new(&h).join(format!(".bwms-{nome}")).exists())
        .unwrap_or(false)
}

/// O que o `bwms.toml` pede agora, lido do disco sem cache: o seletor "Pular boot" mostra a escolha que acabou de gravar.
pub(crate) fn config_lida_agora() -> ConfigDeBoot {
    let txt = arquivo().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let padrao = ConfigDeBoot::default();
    ConfigDeBoot {
        pular_intro: bool_do_toml(&txt, "pular_intro").unwrap_or(padrao.pular_intro) || marcador("skipintro"),
        splash: bool_do_toml(&txt, "splash").unwrap_or(padrao.splash) && !marcador("nosplash"),
        continuar_automatico: bool_do_toml(&txt, "continuar_automatico").unwrap_or(padrao.continuar_automatico)
            || marcador("autocontinue"),
    }
}

/// A configuração do boot, lida uma vez por processo: o boot inteiro obedece a mesma escolha.
pub(crate) fn config() -> ConfigDeBoot {
    static CACHE: OnceLock<ConfigDeBoot> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let c = config_lida_agora();
        crate::log_dev!(
            "[config] pular_intro={} splash={} continuar_automatico={}",
            c.pular_intro,
            c.splash,
            c.continuar_automatico
        );
        c
    })
}

#[cfg(test)]
mod teste {
    use super::*;

    #[test]
    fn le_os_tres_campos_e_ignora_comentario() {
        let t = "# comentário\npular_intro = false\nsplash = true  # com comentário no fim\ncontinuar_automatico = true\n";
        assert_eq!(bool_do_toml(t, "pular_intro"), Some(false));
        assert_eq!(bool_do_toml(t, "splash"), Some(true));
        assert_eq!(bool_do_toml(t, "continuar_automatico"), Some(true));
        assert_eq!(bool_do_toml(t, "inexistente"), None);
    }

    #[test]
    fn linha_comentada_nao_vale() {
        assert_eq!(bool_do_toml("# splash = false\n", "splash"), None);
        assert_eq!(bool_do_toml("  #pular_intro = false\n", "pular_intro"), None);
    }

    #[test]
    fn texto_entre_aspas_e_comentario_no_fim() {
        let t = "# atalho = \"x\"\natalho_terminal = \"ctrl+17\"  # gravado pela aba Mods\n";
        assert_eq!(texto_do_toml(t, "atalho_terminal").as_deref(), Some("ctrl+17"));
        assert_eq!(texto_do_toml(t, "atalho"), None);
        assert_eq!(texto_do_toml("atalho_terminal = ctrl+17\n", "atalho_terminal"), None);
    }

    #[test]
    fn com_chave_troca_a_linha_e_preserva_o_resto() {
        let t = "# meu comentário\nsplash = true\natalho_terminal = \"50\"\ncontinuar_automatico = false\n";
        let novo = com_chave(t, "atalho_terminal", "\"ctrl+17\"", "x");
        assert_eq!(novo, "# meu comentário\nsplash = true\natalho_terminal = \"ctrl+17\"\ncontinuar_automatico = false\n");
    }

    #[test]
    fn com_chave_acrescenta_com_comentario_quando_falta() {
        let novo = com_chave("splash = true\n", "atalho_terminal", "\"ctrl+17\"", "Teclas do terminal.");
        assert_eq!(novo, "splash = true\n\n# Teclas do terminal.\natalho_terminal = \"ctrl+17\"\n");
        assert_eq!(com_chave("", "a", "true", "c"), "# c\na = true\n");
        assert_eq!(bool_do_toml(&com_chave("splash = true\n", "splash", "false", "x"), "splash"), Some(false));
        assert_eq!(texto_do_toml(&novo, "atalho_terminal").as_deref(), Some("ctrl+17"));
    }

    #[test]
    fn valor_invalido_cai_no_padrao() {
        assert_eq!(bool_do_toml("splash = talvez\n", "splash"), None);
        assert_eq!(bool_do_toml("splash =\n", "splash"), None);
    }

    /// O modelo que o instalador grava tem que dizer a mesma coisa que o código assume por padrão —
    /// senão o usuário lê uma coisa no arquivo e o jogo faz outra.
    #[test]
    fn o_modelo_bate_com_os_padroes() {
        let p = ConfigDeBoot::default();
        assert_eq!(bool_do_toml(MODELO, "pular_intro"), Some(p.pular_intro));
        assert_eq!(bool_do_toml(MODELO, "splash"), Some(p.splash));
        assert_eq!(bool_do_toml(MODELO, "continuar_automatico"), Some(p.continuar_automatico));
    }
}
