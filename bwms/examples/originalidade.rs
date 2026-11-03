//! Gate de originalidade da árvore pública: aborta o empacotamento se algum `.reds` publicado tiver um BLOCO CONTÍGUO de CORPO copiado de
//! uma fonte de terceiro vendorizada.
//!
//! Os dois critérios, e por que cada um:
//!
//!  · BLOCO CONTÍGUO, não linha solta. `return true;`, `wrappedMethod();`, `let i: Int32 = 0;` aparecem em qualquer redscript — casar essas
//!    linhas não prova nada. Uma sequência de linhas idênticas, na mesma ordem, contra o mesmo arquivo de origem, prova.
//!
//!  · CORPO, não assinatura. `public native func GetKey() -> EInputKey` sem corpo é a FORMA DO PLUGUE: um mod escrito para o ecossistema de
//!    PC chama esse nome, e qualquer implementação compatível — independente de como foi escrita — declara exatamente isso. O mesmo vale para
//!    enum e campo que existem na RTTI do próprio jogo. O que não pode viajar é implementação alheia: lógica, valores, layout, composição.
//!
//! Uso:   cargo run --release --example originalidade -- <dir-publicado> <dir-fontes-de-terceiro> [limite-bloco]
//! Saída: 0 = limpo; 1 = achou corpo copiado (lista o quê e de onde); 2 = uso errado.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

fn palavra(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `s` começa com `prefixo` e o prefixo termina numa fronteira de palavra (`\b` do regex).
fn comeca_com_palavra(s: &str, prefixo: &str) -> bool {
    s.strip_prefix(prefixo).is_some_and(|r| !r.starts_with(palavra))
}

/// `func` como palavra inteira em qualquer ponto da linha.
fn tem_palavra_func(s: &str) -> bool {
    s.match_indices("func").any(|(i, _)| {
        let antes = s[..i].chars().next_back().is_none_or(|c| !palavra(c));
        let depois = s[i + 4..].chars().next().is_none_or(|c| !palavra(c));
        antes && depois
    })
}

fn tira_modificadores<'a>(mut s: &'a str, modificadores: &[&str]) -> &'a str {
    while let Some(r) = modificadores.iter().find_map(|m| s.strip_prefix(m)) {
        s = r;
    }
    s
}

/// Linha que qualquer implementação compatível escreveria igual.
fn eh_assinatura_ou_fato(s: &str) -> bool {
    if comeca_com_palavra(s, "module") || comeca_com_palavra(s, "import") {
        return true;
    }
    if ["@addField", "@addMethod", "@wrapMethod", "@replaceMethod"].iter().any(|p| comeca_com_palavra(s, p)) {
        return true;
    }
    // assinatura sem corpo
    if tem_palavra_func(s) && !s.contains('{') && !s.contains('=') {
        return true;
    }
    // declaração de campo: `[public |protected |private ][native ]let nome:`
    let campo = ["public ", "protected ", "private "].iter().find_map(|v| s.strip_prefix(v)).unwrap_or(s);
    let campo = campo.strip_prefix("native ").unwrap_or(campo);
    if let Some(r) = campo.strip_prefix("let ") {
        let nome: String = r.chars().take_while(|c| palavra(*c)).collect();
        if !nome.is_empty() && r[nome.len()..].trim_start().starts_with(':') {
            return true;
        }
    }
    // `[visibilidade ][abstract |final |native ]*(class|struct|enum) Nome`
    let tipo = ["public ", "protected ", "private "].iter().find_map(|v| s.strip_prefix(v)).unwrap_or(s);
    let tipo = tira_modificadores(tipo, &["abstract ", "final ", "native "]);
    for kw in ["class", "struct", "enum"] {
        if let Some(r) = tipo.strip_prefix(kw) {
            let sem_espaco = r.trim_start();
            if sem_espaco.len() < r.len() && sem_espaco.starts_with(palavra) {
                return true;
            }
        }
    }
    // membro de enum: `Nome = -?123,?`
    if let Some((nome, valor)) = s.split_once(" = ") {
        let valor = valor.strip_suffix(',').unwrap_or(valor);
        let valor = valor.strip_prefix('-').unwrap_or(valor);
        if !nome.is_empty() && nome.chars().all(palavra) && !valor.is_empty() && valor.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    matches!(s, "}" | "{" | "};" | ")" | "),")
}

/// Só linhas de CORPO — o que carrega autoria. Espaço em branco junto vira um espaço só.
fn corpo(p: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(p) else { return Vec::new() };
    let txt = String::from_utf8_lossy(&bytes);
    let mut out = Vec::new();
    for ln in txt.split('\n') {
        let s = ln.trim();
        if s.is_empty() || s.starts_with("//") || s.starts_with('*') || s.starts_with("/*") {
            continue;
        }
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if eh_assinatura_ou_fato(&s) {
            continue;
        }
        out.push(s);
    }
    out
}

fn arquivos(raiz: &Path, saida: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(raiz) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            arquivos(&p, saida);
        } else if p.is_file() {
            saida.push(p);
        }
    }
}

fn extensao(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// O maior bloco de linhas iguais na mesma ordem, como `(tamanho, início em a)`.
fn maior_bloco(a: &[String], b: &[String]) -> (usize, usize) {
    let mut melhor = (0, 0);
    let mut anterior = vec![0usize; b.len() + 1];
    for (i, la) in a.iter().enumerate() {
        let mut atual = vec![0usize; b.len() + 1];
        for (j, lb) in b.iter().enumerate() {
            if la == lb {
                atual[j + 1] = anterior[j] + 1;
                if atual[j + 1] > melhor.0 {
                    melhor = (atual[j + 1], i + 1 - atual[j + 1]);
                }
            }
        }
        anterior = atual;
    }
    melhor
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("uso: originalidade <dir-publicado> <dir-fontes-de-terceiro> [limite-bloco]");
        std::process::exit(2);
    }
    let publicado = Path::new(&args[1]);
    let terceiros = Path::new(&args[2]);
    let limite: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(5);

    let mut todos = Vec::new();
    arquivos(terceiros, &mut todos);
    let mut fontes: HashMap<PathBuf, Vec<String>> = HashMap::new();
    for q in todos {
        if matches!(extensao(&q).as_str(), "reds" | "script") {
            let linhas = corpo(&q);
            if linhas.len() >= limite {
                fontes.insert(q, linhas);
            }
        }
    }
    // Linha longa (≥ 25 caracteres) → arquivos de terceiro que a têm: só esses viram candidatos à comparação de bloco.
    let mut raro: HashMap<&str, HashSet<&PathBuf>> = HashMap::new();
    for (q, linhas) in &fontes {
        for s in linhas.iter().collect::<HashSet<_>>() {
            if s.chars().count() >= 25 {
                raro.entry(s.as_str()).or_default().insert(q);
            }
        }
    }

    let mut publicados = Vec::new();
    arquivos(publicado, &mut publicados);
    publicados.sort();
    let mut achados: Vec<(usize, String, String, Vec<String>)> = Vec::new();
    for p in publicados.iter().filter(|p| extensao(p) == "reds") {
        let meu = corpo(p);
        if meu.len() < limite {
            continue;
        }
        let mut candidatos: HashSet<&PathBuf> = HashSet::new();
        for s in meu.iter().collect::<HashSet<_>>() {
            if s.chars().count() >= 25 {
                if let Some(qs) = raro.get(s.as_str()) {
                    candidatos.extend(qs.iter().copied());
                }
            }
        }
        let mut candidatos: Vec<&PathBuf> = candidatos.into_iter().collect();
        candidatos.sort();
        for q in candidatos {
            let (tamanho, inicio) = maior_bloco(&meu, &fontes[q]);
            if tamanho >= limite {
                let amostra = meu[inicio..inicio + tamanho].iter().take(3).cloned().collect();
                let nome = |x: &Path| x.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                achados.push((tamanho, nome(p), nome(q), amostra));
                break;
            }
        }
    }

    if achados.is_empty() {
        println!("✅ gate de originalidade: nenhum corpo copiado (limite: bloco de {limite} linhas)");
        return;
    }
    println!("❌ GATE DE ORIGINALIDADE: {} arquivo(s) com corpo copiado (bloco >= {limite} linhas seguidas)", achados.len());
    achados.sort();
    achados.reverse();
    for (tamanho, nome, fonte, amostra) in &achados {
        println!("   {tamanho:4} linhas  {nome}  <-  {fonte}");
        for a in amostra {
            println!("              {}", a.chars().take(76).collect::<String>());
        }
    }
    std::process::exit(1);
}
