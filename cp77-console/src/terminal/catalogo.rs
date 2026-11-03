//! O que cada comando do terminal faz, como dado.
//!
//! Por que existe: o terminal tem **206 comandos** num `match` de milhares de linhas, misturando o
//! que um jogador usa (`give`, `godmode`, `heal`) com o instrumental de engenharia reversa
//! (`inkgetbaseline`). Em 2026-09-20 eu mesmo procurei os comandos
//! de arma e conclui que não existiam — estavam ali, a dez linhas de distância, num braço do `match`
//! que a minha busca não pegou. Se quem escreveu o código não acha, quem baixou o mod também não.
//!
//! Aqui o catálogo é uma tabela: nome, categoria e uma linha do que faz. O `ajuda` do terminal é
//! gerado dela, começando pelo que interessa a quem joga. E o teste no fim do arquivo lê o
//! `lib.rs` e reprova se um comando catalogado aqui faltar no código — a tabela não pode mentir.

/// Para quem o comando serve. É o que separa "o BWMS do usuário" do "BWMS nosso": a ajuda mostra primeiro
/// o que se usa jogando. O comando cuja única saída é o log de dev nem vai ao build público (decisão de 2026-10-13).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Publico {
    /// Quem está jogando: dar item, curar, virar invencível, chamar carro.
    Jogador,
    /// Quem instala e gerencia mods.
    Mods,
    /// Fora do catálogo: nenhum responde na aba Console. No dev é o instrumental com que ACHAMOS o que vira
    /// função (dump, sonda, varredura, prova de gap); no público sobram as ferramentas que mexem no jogo
    /// (TweakDB, reflexão, links de recurso, LUT).
    Engenharia,
}

/// O que a ajuda diz que os comandos fora do catálogo são, neste build.
const O_QUE_SAO_OS_DE_RE: &str =
    if cfg!(feature = "devtools") { "dump, sonda, varredura, prova de gap" } else { "TweakDB, reflexão, links de recurso, LUT" };

impl Publico {
    pub(crate) fn titulo(self) -> &'static str {
        match self {
            Publico::Jogador => "JOGANDO",
            Publico::Mods => "MODS",
            Publico::Engenharia => "ENGENHARIA REVERSA",
        }
    }
}

/// Os comandos que alguém usa jogando ou administrando mods, com a forma de uso. Os de engenharia
/// reversa não entram um a um: no dev são mais de cem, mudam a cada sessão de RE e ninguém os procura pelo
/// nome — a ajuda diz quantos são e como listá-los. Só entra aqui comando que responde na aba
/// Console do build público. Um que fala só pelo log de dev (`log_dev!`, `log_veredito`) ou que é
/// teste de RE com nome de ação (`equipcyber`, `camtilt`, `toggle <cname>`) fica atrás de
/// `#[cfg(feature = "devtools")]` no `run_cmd` e não existe no público.
pub(crate) static CATALOGO: &[(&str, Publico, &str)] = &[
    // --- jogando ---
    ("give", Publico::Jogador, "give <item> [n] — põe um item no inventário (ex.: give Items.Preset_Yukimura_Kang_Tao)"),
    ("remove", Publico::Jogador, "remove <item> [n] — tira um item do inventário"),
    ("money", Publico::Jogador, "money <n> — eddies"),
    ("godmode", Publico::Jogador, "godmode [off] — invencível"),
    ("god", Publico::Jogador, "god [off] — o mesmo que godmode"),
    ("hasgod", Publico::Jogador, "hasgod — diz se o invencível está ligado, sem mudar nada"),
    ("heal", Publico::Jogador, "heal — vida cheia"),
    ("level", Publico::Jogador, "level <n> — nível do V"),
    ("attrs", Publico::Jogador, "attrs <n> — pontos de atributo"),
    ("perks", Publico::Jogador, "perks <n> — pontos de perk"),
    ("relic", Publico::Jogador, "relic <n> — pontos da árvore da Relíquia"),
    ("summon", Publico::Jogador, "summon — chama o veículo"),
    ("car", Publico::Jogador, "car — o mesmo que summon"),
    ("spawnsub", Publico::Jogador, "spawnsub <id> [x y z] — põe um NPC companheiro ao lado do V (ou em x y z)"),
    // --- mods ---
    ("clear", Publico::Mods, "clear — limpa a tela do terminal"),
    ("cls", Publico::Mods, "cls — o mesmo que clear"),
];

/// Texto da ajuda, agrupado por para-quem-serve. `assunto` vazio traz o resumo; com um nome de
/// categoria (`jogando`, `mods`, `re`) traz a lista daquele grupo.
pub(crate) fn ajuda(assunto: &str, total_no_terminal: usize) -> String {
    let alvo = match assunto.trim().to_lowercase().as_str() {
        "" => None,
        "jogando" | "jogador" | "jogo" => Some(Publico::Jogador),
        "mods" | "mod" => Some(Publico::Mods),
        "re" | "engenharia" | "dev" => Some(Publico::Engenharia),
        outro => return format!("ajuda: não conheço '{outro}'. Use: ajuda | ajuda jogando | ajuda mods | ajuda re"),
    };
    if alvo == Some(Publico::Engenharia) {
        let catalogados = CATALOGO.len();
        return format!(
            "ENGENHARIA REVERSA: {} comandos ({O_QUE_SAO_OS_DE_RE}).\n\
             Eles não são listados um a um de propósito — não respondem na aba e não servem\n\
             pra jogar. Estão no código, em `run_cmd`; o que serve pro jogo está em `ajuda jogando`.",
            total_no_terminal.saturating_sub(catalogados)
        );
    }
    let mut s = String::new();
    for grupo in [Publico::Jogador, Publico::Mods] {
        if alvo.is_some() && alvo != Some(grupo) {
            continue;
        }
        s.push_str(&format!("\n-- {} --\n", grupo.titulo()));
        for (_, g, uso) in CATALOGO.iter().filter(|(_, g, _)| *g == grupo) {
            let _ = g;
            s.push_str("  ");
            s.push_str(uso);
            s.push('\n');
        }
    }
    if alvo.is_none() {
        s.push_str(&format!(
            "\n-- {} --\n  {} comandos ({O_QUE_SAO_OS_DE_RE}): `ajuda re`\n",
            Publico::Engenharia.titulo(),
            total_no_terminal.saturating_sub(CATALOGO.len())
        ));
    }
    s
}

#[cfg(test)]
mod teste {
    use super::*;

    /// Todo comando do catálogo tem que existir no terminal. Sem isto a ajuda promete o que o
    /// terminal não faz — pior que não ter ajuda.
    #[test]
    fn o_catalogo_nao_promete_comando_que_nao_existe() {
        let fonte = include_str!("../lib.rs");
        let faltando: Vec<&str> = CATALOGO
            .iter()
            .map(|(n, _, _)| *n)
            .filter(|n| {
                !fonte.contains(&format!("[\"{n}\"]"))
                    && !fonte.contains(&format!("[\"{n}\","))
                    && !fonte.contains(&format!("cmd == \"{n}\""))
                    && !fonte.contains(&format!("strip_prefix(\"{n} \")"))
                    && !fonte.contains(&format!("strip_prefix(\"{n}\")"))
            })
            .collect();
        assert!(faltando.is_empty(), "catalogados mas ausentes do terminal: {faltando:?}");
    }

    /// Os nomes que o `run_cmd` despacha no build público: `if` ou braço sem `#[cfg(feature = ...)]` na linha de cima.
    /// Lê as duas formas de despacho do corpo (`cmd == "x"`/`strip_prefix("x ")` nos `if` do topo e `["x", ..]` nos dois
    /// `match`); um `match` dentro de um braço fica mais fundo e não entra.
    #[cfg(not(any(feature = "devtools", feature = "autoproceed")))]
    fn nomes_do_publico() -> std::collections::BTreeSet<&'static str> {
        let fonte = include_str!("../lib.rs");
        let (_, corpo) = fonte.split_once("\nfn run_cmd(").expect("lib.rs sem run_cmd");
        let corpo = &corpo[..corpo.find("\n}\n").expect("run_cmd sem fim")];
        let mut nomes = std::collections::BTreeSet::new();
        let mut anterior = "";
        for linha in corpo.lines() {
            let fundo = linha.len() - linha.trim_start().len();
            if !["#[cfg(feature", "#[cfg(all(feature"].iter().any(|c| anterior.trim_start().starts_with(c)) {
                if fundo == 4 && linha.trim_start().starts_with("if ") {
                    for (marca, fim) in [("cmd == \"", '"'), ("strip_prefix(\"", ' ')] {
                        for pedaco in linha.split(marca).skip(1) {
                            if let Some(i) = pedaco.find(fim) {
                                nomes.insert(&pedaco[..i]);
                            }
                        }
                    }
                } else if (fundo == 8 || fundo == 12) && linha.trim_start().starts_with("[\"") {
                    for alternativa in linha.split(" =>").next().unwrap_or("").split('|') {
                        if let Some(nome) = alternativa.split('"').nth(1) {
                            nomes.insert(nome);
                        }
                    }
                }
            }
            if !linha.trim().is_empty() {
                anterior = linha;
            }
        }
        nomes
    }

    /// O mesmo comando com outro nome: fica fora da conta do `TOTAL_DE_COMANDOS`.
    #[cfg(not(any(feature = "devtools", feature = "autoproceed")))]
    const APELIDOS: [&str; 5] = ["help", "?", "god", "car", "cls"];

    /// O número que a ajuda usa no público é o do fonte: os nomes sem `cfg` no `run_cmd`, menos os apelidos.
    #[cfg(not(any(feature = "devtools", feature = "autoproceed")))]
    #[test]
    fn o_total_do_publico_e_o_do_fonte() {
        let nomes = nomes_do_publico();
        let sem_apelido = nomes.iter().filter(|n| !APELIDOS.contains(n)).count();
        assert_eq!(sem_apelido, crate::TOTAL_DE_COMANDOS, "nomes do público: {nomes:?}");
    }

    /// O catálogo é o que o público promete: comando catalogado atrás de `cfg(feature = "devtools")` some do jogo do
    /// usuário e a ajuda continua mostrando.
    #[cfg(not(any(feature = "devtools", feature = "autoproceed")))]
    #[test]
    fn o_catalogo_so_promete_comando_do_publico() {
        let nomes = nomes_do_publico();
        let so_no_dev: Vec<&str> = CATALOGO.iter().map(|(n, _, _)| *n).filter(|n| !nomes.contains(n)).collect();
        assert!(so_no_dev.is_empty(), "catalogados que o público não tem: {so_no_dev:?}");
    }

    /// Imprime a ajuda como o usuário a vê — roda com `-- --nocapture` pra conferir o texto.
    #[test]
    fn mostra_no_terminal() {
        println!("{}", ajuda("", 206));
    }

    #[test]
    fn a_ajuda_comeca_pelo_que_se_usa_jogando() {
        let t = ajuda("", 206);
        let pos_jogando = t.find("JOGANDO").expect("sem seção de jogando");
        let pos_re = t.find("ENGENHARIA").expect("sem seção de RE");
        assert!(pos_jogando < pos_re, "a ajuda tem que abrir pelo que serve pra jogar");
        assert!(t.contains("give <item>"), "o comando de dar item tem que estar no resumo");
        assert!(t.contains("godmode"), "o invencível tem que estar no resumo");
    }

    #[test]
    fn cada_grupo_responde_sozinho() {
        assert!(ajuda("jogando", 206).contains("give <item>"));
        assert!(!ajuda("jogando", 206).contains("clear"));
        assert!(ajuda("mods", 206).contains("clear"));
        assert!(ajuda("re", 206).contains("ENGENHARIA"));
        assert!(ajuda("bobagem", 206).contains("não conheço"));
    }

    /// A conta de "quantos são de RE" sai do total menos os catalogados — se alguém catalogar mais,
    /// o número tem que cair sozinho.
    #[test]
    fn a_conta_de_re_acompanha_o_catalogo() {
        let t = ajuda("re", 206);
        let esperado = 206 - CATALOGO.len();
        assert!(t.contains(&esperado.to_string()), "esperava {esperado} em: {t}");
    }
}
