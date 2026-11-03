//! `ResourceTarget`: filtro de path por WILDCARD ou REGEX explícito.
//!
//! A fábrica `Path()` decide UMA vez, nesta ordem:
//!
//! 1. padrão começa com `"regex:"` → o resto é regex crua (ECMAScript);
//! 2. senão, padrão contém `*` → glob ancorado nas duas pontas, `*` casa qualquer trecho;
//! 3. senão → sem regex; o `Matches` compara o path.
//!
//! O `Matches` só executa o que a fábrica montou, casando o texto inteiro.
//! [`CompiledResourceTarget`] é essa decisão compilada: quem guarda o alvo compila uma vez e casa
//! por evento sem reinterpretar o texto.
//!
//! O caso (2) não passa pelo motor de regex: depois da transformação o padrão é um GLOB — literais
//! separados por `.*`, ancorado nas duas pontas — e casar glob é exato e linear. O caso (1) usa
//! `crate::regex`. `None` em [`casa`] é "não sei" (padrão `regex:` recusado, com o motivo em
//! [`CompiledResourceTarget::Recusado`]), nunca "não casou".
//!
//! [`wildcard_para_regex`] dá a regex equivalente ao glob do caso (2), para conferir que os dois
//! aceitam o mesmo conjunto de strings.

use crate::regex::{ErroDeRegex, Regex};

/// Regex equivalente a um padrão com `*` (usada pela native de devtools e pelo teste de equivalência com o glob).
/// `^` + o padrão com `*` → `.*` e cada metacaractere de regex escapado + `$`: ancorada nas duas pontas, como o glob.
pub fn wildcard_para_regex(padrao: &str) -> String {
    let mut rx = String::with_capacity(padrao.len() * 2 + 2);
    rx.push('^');
    for c in padrao.chars() {
        match c {
            '*' => rx.push_str(".*"),
            '.' | '^' | '$' | '-' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '?' | '\\' => {
                rx.push('\\');
                rx.push(c);
            }
            _ => rx.push(c),
        }
    }
    rx.push('$');
    rx
}

/// Padrão de `ResourceTarget` interpretado e compilado uma vez, na fábrica.
#[derive(Debug)]
pub enum CompiledResourceTarget {
    /// Tinha `*`: glob ancorado, o mesmo conjunto da regex equivalente.
    Glob(Box<str>),
    /// Prefixo `regex:`.
    Regex(Regex),
    /// Prefixo `regex:` que o motor recusou; o motivo vai para o diagnóstico.
    Recusado(ErroDeRegex),
}

impl CompiledResourceTarget {
    /// `None` = o texto não é padrão (sem `regex:` e sem `*`): o path é comparado, sem regex.
    pub fn compilar(padrao: &str) -> Option<Self> {
        if let Some(resto) = padrao.strip_prefix("regex:") {
            return Some(match Regex::compilar(resto) {
                Ok(r) => Self::Regex(r),
                Err(e) => Self::Recusado(e),
            });
        }
        padrao.contains('*').then(|| Self::Glob(padrao.into()))
    }

    /// `None` = padrão recusado ("não sei"), diferente de `Some(false)` = avaliado e NÃO casou.
    pub fn casa(&self, path: &str) -> Option<bool> {
        match self {
            Self::Glob(g) => Some(casa_glob(g, path)),
            Self::Regex(r) => Some(r.casa(path)),
            Self::Recusado(_) => None,
        }
    }
}

/// Casa um glob ancorado (`*` = qualquer sequência, inclusive vazia) contra um texto.
///
/// Iterativo com backtracking de uma estrela só — o algoritmo clássico, O(n·m) no pior caso e sem
/// recursão. Sobre bytes; com UTF-8 válido dos dois
/// lados dá o mesmo que por caractere, porque byte líder nunca coincide com byte de continuação.
// TODO: `*` aqui casa `\n`/`\r`; o `.*` da regex talvez não (ver o TODO do `.` em `crate::regex`). Path de recurso não tem quebra de linha.
fn casa_glob(padrao: &str, texto: &str) -> bool {
    let (p, t) = (padrao.as_bytes(), texto.as_bytes());
    let (mut i, mut j) = (0usize, 0usize);
    let (mut estrela, mut retomada) = (usize::MAX, 0usize);
    while j < t.len() {
        if i < p.len() && p[i] == b'*' {
            estrela = i;
            retomada = j;
            i += 1;
        } else if i < p.len() && p[i] == t[j] {
            i += 1;
            j += 1;
        } else if estrela != usize::MAX {
            // volta pra depois da última estrela e deixa ela consumir mais um byte
            i = estrela + 1;
            retomada += 1;
            j = retomada;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == b'*' {
        i += 1;
    }
    i == p.len()
}

/// Decide se um path casa com o padrão, compilando na hora. Casar o mesmo padrão várias vezes sem
/// recompilar é o [`CompiledResourceTarget`].
pub fn casa(padrao: &str, path: &str) -> Option<bool> {
    match CompiledResourceTarget::compilar(padrao) {
        Some(alvo) => alvo.casa(path),
        None => Some(padrao == path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_vira_regex_ancorada_com_metacaracteres_escapados() {
        // Entrada (string real): `base\*.inkatlas`
        //   `\` é metacaractere -> vira `\\`;  `*` -> `.*`;  `.` é metacaractere -> vira `\.`
        // Saída esperada (string real): `^base\\.*\.inkatlas$`
        assert_eq!(wildcard_para_regex("base\\*.inkatlas"), "^base\\\\.*\\.inkatlas$");
        // metacaractere real é escapado
        assert_eq!(wildcard_para_regex("a+b"), "^a\\+b$");
        assert_eq!(wildcard_para_regex("a(b)"), "^a\\(b\\)$");
        assert_eq!(wildcard_para_regex("a.b"), "^a\\.b$");
    }

    #[test]
    fn literal_sem_estrela() {
        assert!(CompiledResourceTarget::compilar("base\\x.mesh").is_none());
        assert_eq!(casa("base\\x.mesh", "base\\x.mesh"), Some(true));
        assert_eq!(casa("base\\x.mesh", "base\\y.mesh"), Some(false));
    }

    #[test]
    fn glob_prefixo_sufixo_meio() {
        assert_eq!(casa("base\\*", "base\\gameplay\\a.mesh"), Some(true));
        assert_eq!(casa("*.inkatlas", "base\\ui\\a.inkatlas"), Some(true));
        assert_eq!(casa("*.inkatlas", "base\\ui\\a.mesh"), Some(false));
        assert_eq!(casa("base\\*\\a.mesh", "base\\ui\\a.mesh"), Some(true));
        assert_eq!(casa("base\\*\\a.mesh", "base\\ui\\b.mesh"), Some(false));
    }

    #[test]
    fn glob_por_byte_igual_ao_glob_por_caractere() {
        assert_eq!(casa("*é*", "aéb"), Some(true));
        assert_eq!(casa("a*ç", "aç"), Some(true));
        assert_eq!(casa("*é", "ae"), Some(false));
        assert_eq!(casa("a*é", "aèé"), Some(true));
    }

    #[test]
    fn estrela_casa_vazio() {
        // `.*` aceita sequência vazia — o glob tem de aceitar também.
        assert_eq!(casa("ab*", "ab"), Some(true));
        assert_eq!(casa("*ab", "ab"), Some(true));
        assert_eq!(casa("a*b", "ab"), Some(true));
    }

    #[test]
    fn ancoragem_nas_duas_pontas() {
        // A regex ancora com ^...$: um padrão sem estrela nas pontas NÃO casa substring.
        assert_eq!(casa("b*c", "abcd"), Some(false));
        assert_eq!(casa("*b*c*", "abcd"), Some(true));
    }

    #[test]
    fn regex_explicita_resolve_e_nao_suportado_e_none() {
        assert_eq!(casa("regex:^base.*$", "base\\x"), Some(true));
        assert_eq!(casa("regex:^base.*$", "outro"), Some(false), "e sabe dizer que NÃO casa");
        // Fora do subconjunto: "não sei", nunca "não casou".
        assert_eq!(casa("regex:a{2,3}", "aa"), None);
        assert!(matches!(CompiledResourceTarget::compilar("regex:abc"), Some(CompiledResourceTarget::Regex(_))));
    }

    #[test]
    fn compila_uma_vez_e_casa_varias() {
        assert!(matches!(CompiledResourceTarget::compilar("base\\*"), Some(CompiledResourceTarget::Glob(_))));
        assert!(matches!(CompiledResourceTarget::compilar("regex:^a$"), Some(CompiledResourceTarget::Regex(_))));
        assert!(matches!(
            CompiledResourceTarget::compilar("regex:a{2}"),
            Some(CompiledResourceTarget::Recusado(ErroDeRegex::NaoSuportado))
        ));
        assert!(matches!(
            CompiledResourceTarget::compilar("regex:(abc"),
            Some(CompiledResourceTarget::Recusado(ErroDeRegex::Invalido))
        ));
        // O prefixo é sensível a caixa.
        assert!(CompiledResourceTarget::compilar("REGEX:abc").is_none());
        let c = CompiledResourceTarget::compilar("regex:^base.*$").expect("é padrão");
        assert_eq!(c.casa("base\\x"), Some(true));
        assert_eq!(c.casa("outro"), Some(false));
        let r = CompiledResourceTarget::compilar(r"regex:^\d$").expect("é padrão");
        assert_eq!(r.casa("5"), None, "recusado responde 'não sei' em todo path");
        assert_eq!(r.casa("d"), None);
    }

    #[test]
    fn multiplas_estrelas_nao_explodem() {
        let padrao = "*a*a*a*a*a*b";
        assert_eq!(casa(padrao, &("a".repeat(64) + "b")), Some(true));
        assert_eq!(casa(padrao, &"a".repeat(64)), Some(false));
    }

    /// `cargo test --release -- --ignored --nocapture bench_restarget_match`. Padrão compilado uma
    /// vez contra `casa` livre, que recompila a cada chamada como o `caminho_casa` do cp77-console
    /// faz por evento `Resource/Load`.
    #[test]
    #[ignore]
    fn bench_restarget_match() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};
        let deco = r"base\environment\decoration\furniture\chairs\chair_a\chair_a_01.mesh";
        let a40 = "a".repeat(40);
        let casos: [(&str, &str, u32); 6] = [
            (r"base\environment\decoration\*", deco, 200_000),
            ("*.inkatlas", r"base\ui\a.inkatlas", 200_000),
            (r"base\ui\a.mesh", r"base\ui\a.mesh", 200_000),
            (r"regex:^base\\gameplay\\.*\.ent$", r"base\gameplay\npc\joe.ent", 200_000),
            (r"regex:^base\\environment\\decoration\\.*$", deco, 200_000),
            (r"regex:^(a*)*b$", &a40, 200),
        ];
        let mut total = Duration::ZERO;
        let mut total_livre = Duration::ZERO;
        for (padrao, path, n) in casos {
            let compilado = CompiledResourceTarget::compilar(padrao);
            let t0 = Instant::now();
            let mut casou = 0u32;
            for _ in 0..n {
                let r = match &compilado {
                    Some(alvo) => alvo.casa(black_box(path)),
                    None => Some(black_box(padrao) == black_box(path)),
                };
                if r == Some(true) {
                    casou += 1;
                }
            }
            let dt = t0.elapsed();
            total += dt;
            let t1 = Instant::now();
            for _ in 0..n {
                black_box(casa(black_box(padrao), black_box(path)));
            }
            let dt_livre = t1.elapsed();
            total_livre += dt_livre;
            println!(
                "bench_restarget_match {padrao:?} n={n}: compilado {:.1} ns/iter | casa livre {:.1} ns/iter casou={casou}",
                dt.as_nanos() as f64 / f64::from(n),
                dt_livre.as_nanos() as f64 / f64::from(n)
            );
        }
        println!(
            "bench_restarget_match total: compilado {:.1} ms | casa livre {:.1} ms",
            total.as_secs_f64() * 1e3,
            total_livre.as_secs_f64() * 1e3
        );
    }

    #[test]
    fn vazios() {
        assert_eq!(casa("", ""), Some(true));
        assert_eq!(casa("*", ""), Some(true));
        assert_eq!(casa("", "x"), Some(false));
    }
}

#[cfg(test)]
mod tests_regex_explicita {
    use super::*;

    #[test]
    fn regex_explicita_casa_o_path_inteiro() {
        assert_eq!(casa(r"regex:^base\\gameplay\\.*\.ent$", r"base\gameplay\v.ent"), Some(true));
        assert_eq!(casa(r"regex:^base\\gameplay\\.*\.ent$", r"base\outro\v.ent"), Some(false));
        // Casamento do texto inteiro: sem âncora, o padrão ainda cobre o path inteiro.
        // Pendência: a prova do alvo por regex de 2026-09-05 registrou a busca por substring (esperado=1); o smoke de dev
        // `regex-alternancia` já espera 0 e só vale como prova num boot novo.
        assert_eq!(casa("regex:(foo|bar)", "xxbarxx"), Some(false));
        assert_eq!(casa("regex:(foo|bar)", "bar"), Some(true));
        assert_eq!(casa("regex:", r"base\x.ent"), Some(false), "regex vazia não casa path nenhum");
        assert_eq!(casa("regex:abc", "abc"), Some(true));
        assert_eq!(casa("regex:^[a-z]+$", "Path"), Some(false));
    }

    #[test]
    fn fora_do_subconjunto_continua_indefinido() {
        // A distinção que a API existe pra preservar: "não sei" != "não casou".
        assert_eq!(casa("regex:a{2,3}", "aa"), None);
        assert_eq!(casa("regex:(abc", "abc"), None);
    }

    #[test]
    fn as_tres_rotas_do_item_convivem() {
        assert_eq!(casa("exato.ent", "exato.ent"), Some(true));      // literal
        assert_eq!(casa("*.ent", "v.ent"), Some(true));              // wildcard
        assert_eq!(casa("regex:^v\\.ent$", "v.ent"), Some(true));    // regex explícita
    }
}
