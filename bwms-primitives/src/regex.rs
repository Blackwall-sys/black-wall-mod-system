//! Motor de regex do `regex:` do Codeware `#137` (`ResourceTarget`).
//!
//! **Contrato do original.** O Codeware monta `std::regex` com as flags padrão (ECMAScript) uma vez,
//! na fábrica `Path()` (`App/Callback/Targets/ResourceTarget.hpp:72-74`), e casa com
//! `std::regex_match` (`hpp:36`): o texto INTEIRO, sobre `std::string`, byte a byte. Este motor
//! segue as três coisas: compila uma vez, casa o texto inteiro, a unidade é o byte.
//!
//! O projeto é zero-dependência, e `std::regex` sobre path de recurso usa um vocabulário pequeno.
//! **O que suporta**, cada um porque um padrão de path o escreve:
//!   `.` qualquer byte · `*` `+` `?` quantificadores · `[abc]` `[^abc]` `[a-z]` classes ·
//!   `(a|b)` grupo · `|` em qualquer nível · `^` `$` asserções em qualquer posição ·
//!   escape de não alfanumérico (`\.` `\\` `\$` `\-`).
//!
//! **O que recusa**, com [`ErroDeRegex`] em vez de uma resposta: escape alfanumérico (`\d`, `\w`,
//! `\b`, backreference `\1`), `{n,m}`, grupo `(?…)`, quantificador preguiçoso, `[]`/`[^]`, classe
//! POSIX `[[:digit:]]`, faixa com byte não-ASCII. Recusar é dizer "não sei"; casar como literal
//! transformaria "não implementei" em "não casou".
//!
//! **Como casa.** parse → árvore (`No`) → programa de Thompson (`Inst`) → simulação com o
//! conjunto de estados ativos (Pike VM sem captura). Custo O(texto × programa), sem recursão e sem
//! backtracking no casamento: o padrão vem do mod e roda no despacho de `Resource/Load`. O teto de
//! complexidade fica na compilação (`PADRAO_MAX`, `ANINHAMENTO_MAX`), nunca no casamento.

use std::fmt;

/// Teto do padrão em bytes. Cada byte gera no máximo 2 instruções (`|` gera bifurcação e salto),
/// então o programa fica abaixo de 2·`PADRAO_MAX`+1 e o casamento abaixo de texto × isso.
const PADRAO_MAX: usize = 4096;

/// Teto de grupos aninhados. O parser e a compilação recursam por `(`; sem teto, `((((…` vindo do
/// mod estoura a pilha da thread do jogo.
const ANINHAMENTO_MAX: u32 = 256;

/// Por que um padrão não compilou. Nenhuma das três é "não casou".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErroDeRegex {
    /// Sintaxe quebrada: grupo ou classe aberta, `)` sobrando, quantificador sem alvo ou sobre
    /// asserção, faixa invertida, `\` no fim.
    Invalido,
    /// Sintaxe do ECMAScript fora do subconjunto implementado.
    NaoSuportado,
    /// Acima de `PADRAO_MAX` bytes ou de `ANINHAMENTO_MAX` grupos aninhados.
    Complexo,
}

impl fmt::Display for ErroDeRegex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErroDeRegex::Invalido => f.write_str("sintaxe inválida: confira parênteses, colchetes, faixas e quantificadores"),
            ErroDeRegex::NaoSuportado => f.write_str(
                "recurso fora do subconjunto (\\d, {n,m}, (?…), *?, [[:x:]]): reescreva com . [0-9] * + ? (a|b)",
            ),
            ErroDeRegex::Complexo => write!(
                f,
                "padrão grande demais (mais de {PADRAO_MAX} bytes ou {ANINHAMENTO_MAX} grupos aninhados): divida em padrões menores"
            ),
        }
    }
}

/// Conjunto de bytes de uma classe: um bit por valor de byte.
type ConjuntoDeBytes = [u64; 4];

fn conjunto_contem(c: &ConjuntoDeBytes, b: u8) -> bool {
    c[usize::from(b >> 6)] >> (b & 63) & 1 != 0
}

fn conjunto_marcar(c: &mut ConjuntoDeBytes, b: u8) {
    c[usize::from(b >> 6)] |= 1 << (b & 63);
}

enum Quantificador {
    Estrela,
    Mais,
    Opcional,
}

/// Árvore do padrão, saída do parser e entrada da compilação.
enum No {
    Byte(u8),
    /// `.`
    Qualquer,
    Classe(ConjuntoDeBytes),
    /// `^`: largura zero, só na posição 0 (o original não liga `multiline`).
    Ini,
    /// `$`: largura zero, só no fim do texto.
    Fim,
    /// `(a|b)`; o nível de topo também é uma alternância.
    Alt(Vec<Vec<No>>),
    Rep(Box<No>, Quantificador),
}

/// Instrução do programa de Thompson. Alvos em `u32`: o programa tem menos de 2·[`PADRAO_MAX`]+1
/// instruções.
#[derive(Debug, Clone, Copy)]
enum Inst {
    Byte(u8),
    Qualquer,
    /// Índice em [`Regex::classes`].
    Classe(u32),
    Ini,
    Fim,
    /// Segue os dois alvos sem consumir byte.
    Bifurca(u32, u32),
    Salta(u32),
    Aceita,
}

/// Uma regex compilada.
#[derive(Debug)]
pub struct Regex {
    programa: Vec<Inst>,
    classes: Vec<ConjuntoDeBytes>,
}

struct Parser<'a> {
    p: &'a [u8],
    i: usize,
    aninhamento: u32,
}

impl Parser<'_> {
    fn olhar(&self) -> Option<u8> {
        self.p.get(self.i).copied()
    }

    fn comer(&mut self) -> Option<u8> {
        let c = self.olhar()?;
        self.i += 1;
        Some(c)
    }

    fn alternancia(&mut self) -> Result<Vec<Vec<No>>, ErroDeRegex> {
        let mut ramos = vec![self.sequencia()?];
        while self.olhar() == Some(b'|') {
            self.i += 1;
            ramos.push(self.sequencia()?);
        }
        Ok(ramos)
    }

    /// Sequência até `|` ou `)`.
    fn sequencia(&mut self) -> Result<Vec<No>, ErroDeRegex> {
        let mut out = Vec::new();
        while let Some(c) = self.olhar() {
            if c == b'|' || c == b')' {
                break;
            }
            self.i += 1;
            let no = match c {
                b'^' => No::Ini,
                b'$' => No::Fim,
                b'.' => No::Qualquer,
                b'(' => self.grupo()?,
                b'[' => self.classe()?,
                b'\\' => No::Byte(self.escape()?),
                b'*' | b'+' | b'?' => return Err(ErroDeRegex::Invalido), // quantificador sem alvo
                b'{' => return Err(ErroDeRegex::NaoSuportado),
                outro => No::Byte(outro),
            };
            let q = match self.olhar() {
                Some(b'*') => Quantificador::Estrela,
                Some(b'+') => Quantificador::Mais,
                Some(b'?') => Quantificador::Opcional,
                _ => {
                    out.push(no);
                    continue;
                }
            };
            // ECMAScript não quantifica asserção: `^*` é erro de sintaxe.
            if matches!(no, No::Ini | No::Fim) {
                return Err(ErroDeRegex::Invalido);
            }
            self.i += 1;
            // `*?` `+?` `??`: quantificador preguiçoso.
            if self.olhar() == Some(b'?') {
                return Err(ErroDeRegex::NaoSuportado);
            }
            out.push(No::Rep(Box::new(no), q));
        }
        Ok(out)
    }

    /// Depois do `(`.
    fn grupo(&mut self) -> Result<No, ErroDeRegex> {
        // `(?:` `(?=` `(?!`
        if self.olhar() == Some(b'?') {
            return Err(ErroDeRegex::NaoSuportado);
        }
        if self.aninhamento == ANINHAMENTO_MAX {
            return Err(ErroDeRegex::Complexo);
        }
        self.aninhamento += 1;
        let alt = self.alternancia()?;
        self.aninhamento -= 1;
        if self.comer() != Some(b')') {
            return Err(ErroDeRegex::Invalido);
        }
        Ok(No::Alt(alt))
    }

    /// Depois do `\`.
    fn escape(&mut self) -> Result<u8, ErroDeRegex> {
        let e = self.comer().ok_or(ErroDeRegex::Invalido)?;
        // No ECMAScript, `\` + alfanumérico nunca é a própria letra: é classe (`\d`), asserção
        // (`\b`), controle (`\n`) ou backreference (`\1`). Não alfanumérico é o próprio byte.
        if e.is_ascii_alphanumeric() {
            return Err(ErroDeRegex::NaoSuportado);
        }
        Ok(e)
    }

    /// Depois do `[`.
    fn classe(&mut self) -> Result<No, ErroDeRegex> {
        let negado = self.olhar() == Some(b'^');
        if negado {
            self.i += 1;
        }
        // `[]` e `[^]`: no ECMAScript, classe vazia e "qualquer byte"; no POSIX, `]` literal. Qual
        // dos dois o `std::regex` do MSVC aplica não está comprovado, e recusar não depende disso.
        if self.olhar() == Some(b']') {
            return Err(ErroDeRegex::NaoSuportado);
        }
        let mut bytes: ConjuntoDeBytes = [0; 4];
        loop {
            let c = self.comer().ok_or(ErroDeRegex::Invalido)?;
            if c == b']' {
                break;
            }
            // `[:digit:]` `[.a.]` `[=a=]`: classes POSIX que o `std::regex` aceita entre colchetes.
            if c == b'[' && matches!(self.olhar(), Some(b':' | b'.' | b'=')) {
                return Err(ErroDeRegex::NaoSuportado);
            }
            let inicio = if c == b'\\' { self.escape()? } else { c };
            // `a-z`; `-` logo antes do `]` é literal
            if self.olhar() == Some(b'-') && self.p.get(self.i + 1).is_some_and(|&n| n != b']') {
                self.i += 1;
                let f = self.comer().ok_or(ErroDeRegex::Invalido)?;
                let fim = if f == b'\\' { self.escape()? } else { f };
                // A ordem de byte acima de 0x7f depende de `char` ter sinal no compilador do original.
                if !inicio.is_ascii() || !fim.is_ascii() {
                    return Err(ErroDeRegex::NaoSuportado);
                }
                if inicio > fim {
                    return Err(ErroDeRegex::Invalido);
                }
                for b in inicio..=fim {
                    conjunto_marcar(&mut bytes, b);
                }
            } else {
                conjunto_marcar(&mut bytes, inicio);
            }
        }
        if negado {
            for palavra in &mut bytes {
                *palavra = !*palavra;
            }
        }
        Ok(No::Classe(bytes))
    }
}

struct Compilador {
    programa: Vec<Inst>,
    classes: Vec<ConjuntoDeBytes>,
}

impl Compilador {
    fn pc(&self) -> u32 {
        self.programa.len() as u32
    }

    fn emitir(&mut self, i: Inst) -> u32 {
        let pc = self.pc();
        self.programa.push(i);
        pc
    }

    fn sequencia(&mut self, seq: &[No]) {
        for no in seq {
            self.no(no);
        }
    }

    fn alternancia(&mut self, ramos: &[Vec<No>]) {
        let Some((ultimo, anteriores)) = ramos.split_last() else { return };
        let mut saltos = Vec::with_capacity(anteriores.len());
        for ramo in anteriores {
            let bifurca = self.emitir(Inst::Bifurca(0, 0));
            self.sequencia(ramo);
            saltos.push(self.emitir(Inst::Salta(0)));
            self.programa[bifurca as usize] = Inst::Bifurca(bifurca + 1, self.pc());
        }
        self.sequencia(ultimo);
        let fim = self.pc();
        for s in saltos {
            self.programa[s as usize] = Inst::Salta(fim);
        }
    }

    fn no(&mut self, no: &No) {
        match no {
            No::Byte(b) => {
                self.emitir(Inst::Byte(*b));
            }
            No::Qualquer => {
                self.emitir(Inst::Qualquer);
            }
            No::Classe(c) => {
                let k = self.classes.len() as u32;
                self.classes.push(*c);
                self.emitir(Inst::Classe(k));
            }
            No::Ini => {
                self.emitir(Inst::Ini);
            }
            No::Fim => {
                self.emitir(Inst::Fim);
            }
            No::Alt(ramos) => self.alternancia(ramos),
            No::Rep(corpo, Quantificador::Estrela) => {
                let bifurca = self.emitir(Inst::Bifurca(0, 0));
                self.no(corpo);
                self.emitir(Inst::Salta(bifurca));
                self.programa[bifurca as usize] = Inst::Bifurca(bifurca + 1, self.pc());
            }
            No::Rep(corpo, Quantificador::Mais) => {
                let inicio = self.pc();
                self.no(corpo);
                let bifurca = self.emitir(Inst::Bifurca(inicio, 0));
                self.programa[bifurca as usize] = Inst::Bifurca(inicio, bifurca + 1);
            }
            No::Rep(corpo, Quantificador::Opcional) => {
                let bifurca = self.emitir(Inst::Bifurca(0, 0));
                self.no(corpo);
                self.programa[bifurca as usize] = Inst::Bifurca(bifurca + 1, self.pc());
            }
        }
    }
}

impl Regex {
    /// Compila. O erro diz por que o padrão ficou sem resposta; os consumidores do `#137` recebem
    /// `None` ("não sei") via [`crate::restarget::CompiledResourceTarget::casa`].
    pub fn compilar(padrao: &str) -> Result<Regex, ErroDeRegex> {
        if padrao.len() > PADRAO_MAX {
            return Err(ErroDeRegex::Complexo);
        }
        let mut p = Parser { p: padrao.as_bytes(), i: 0, aninhamento: 0 };
        let alt = p.alternancia()?;
        if p.i != p.p.len() {
            return Err(ErroDeRegex::Invalido); // `)` sem `(`
        }
        let mut c = Compilador { programa: Vec::new(), classes: Vec::new() };
        c.alternancia(&alt);
        c.emitir(Inst::Aceita);
        Ok(Regex { programa: c.programa, classes: c.classes })
    }

    /// `true` se o texto INTEIRO casa, como `std::regex_match` (`hpp:36`).
    pub fn casa(&self, texto: &str) -> bool {
        let t = texto.as_bytes();
        let m = self.programa.len();
        let mut atuais: Vec<u32> = Vec::with_capacity(m);
        let mut proximos: Vec<u32> = Vec::with_capacity(m);
        let mut pilha: Vec<u32> = Vec::with_capacity(m);
        // Posição + 1 em que o pc entrou num conjunto (0 = nunca): uma geração por posição dispensa
        // limpar entre passos.
        let mut marca: Vec<usize> = vec![0; m];
        self.fecho(0, 0, t.len(), &mut atuais, &mut marca, &mut pilha);
        for pos in 0..=t.len() {
            if atuais.is_empty() {
                return false;
            }
            let byte = t.get(pos).copied();
            for &pc in &atuais {
                let avanca = match (self.programa[pc as usize], byte) {
                    (Inst::Aceita, None) => return true,
                    (Inst::Byte(b), Some(c)) => b == c,
                    // TODO: no ECMAScript `.` não casa `\n`/`\r`; o que o `std::regex` do MSVC faz não
                    // está comprovado, e path de recurso não tem quebra de linha.
                    (Inst::Qualquer, Some(_)) => true,
                    (Inst::Classe(k), Some(c)) => conjunto_contem(&self.classes[k as usize], c),
                    _ => false,
                };
                if avanca {
                    self.fecho(pc + 1, pos + 1, t.len(), &mut proximos, &mut marca, &mut pilha);
                }
            }
            std::mem::swap(&mut atuais, &mut proximos);
            proximos.clear();
        }
        false
    }

    /// Fecho-ε a partir de `pc` na posição `pos`: segue `Bifurca`/`Salta` e as asserções que valem
    /// em `pos`, e guarda em `lista` só o que consome byte ou aceita. Cada pc entra uma vez por
    /// posição, o que também encerra laço vazio (`(a*)*`).
    fn fecho(&self, pc: u32, pos: usize, fim: usize, lista: &mut Vec<u32>, marca: &mut [usize], pilha: &mut Vec<u32>) {
        pilha.push(pc);
        while let Some(pc) = pilha.pop() {
            let k = pc as usize;
            if marca[k] == pos + 1 {
                continue;
            }
            marca[k] = pos + 1;
            match self.programa[k] {
                Inst::Salta(a) => pilha.push(a),
                Inst::Bifurca(a, b) => {
                    pilha.push(b);
                    pilha.push(a);
                }
                Inst::Ini => {
                    if pos == 0 {
                        pilha.push(pc + 1);
                    }
                }
                Inst::Fim => {
                    if pos == fim {
                        pilha.push(pc + 1);
                    }
                }
                Inst::Byte(_) | Inst::Qualquer | Inst::Classe(_) | Inst::Aceita => lista.push(pc),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::restarget;

    fn casa(p: &str, t: &str) -> bool {
        Regex::compilar(p).unwrap_or_else(|e| panic!("não compilou: {p}: {e}")).casa(t)
    }

    #[test]
    fn casa_o_texto_inteiro() {
        assert!(casa("abc", "abc"));
        assert!(!casa("abc", "xxabcxx"), "`std::regex_match` casa o texto inteiro, não substring");
        assert!(!casa("abc", "abd"));
    }

    #[test]
    fn ancoras() {
        assert!(casa("^abc$", "abc"));
        assert!(!casa("^abc$", "xabc"));
        assert!(!casa("^abc$", "abcx"));
        assert!(!casa("^abc", "abcdef"));
        assert!(!casa("def$", "abcdef"));
    }

    #[test]
    fn ponto_e_qualquer_caractere() {
        assert!(casa("^a.c$", "abc"));
        assert!(casa("^a.c$", "a_c"));
        assert!(!casa("^a.c$", "ac"), "`.` consome exatamente 1");
    }

    #[test]
    fn quantificadores() {
        assert!(casa("^ab*c$", "ac"));
        assert!(casa("^ab*c$", "abbbc"));
        assert!(!casa("^ab+c$", "ac"), "`+` exige pelo menos 1");
        assert!(casa("^ab+c$", "abc"));
        assert!(casa("^ab?c$", "ac"));
        assert!(casa("^ab?c$", "abc"));
        assert!(!casa("^ab?c$", "abbc"), "`?` aceita no máximo 1");
    }

    #[test]
    fn estrela_gulosa_com_cauda() {
        // o caso que exige considerar TODAS as contagens, não só a maior
        assert!(casa("^a.*c$", "abbbc"));
        assert!(casa("^.*$", ""));
        assert!(casa("^a.*a$", "aba"));
    }

    #[test]
    fn classes() {
        assert!(casa("^[abc]$", "b"));
        assert!(!casa("^[abc]$", "d"));
        assert!(casa("^[a-z]+$", "path"));
        assert!(!casa("^[a-z]+$", "Path"));
        assert!(casa("^[^0-9]+$", "abc"));
        assert!(!casa("^[^0-9]+$", "ab1"));
    }

    #[test]
    fn traco_no_fim_da_classe_e_literal() {
        assert!(casa("^[a-]$", "-"));
        assert!(casa("^[a-]$", "a"));
    }

    #[test]
    fn classe_vazia_e_recusada() {
        // `[]]` é `]` literal no POSIX e classe vazia + `]` no ECMAScript; o MSVC não está comprovado.
        assert_eq!(Regex::compilar("^[]]$").err(), Some(ErroDeRegex::NaoSuportado));
        assert_eq!(Regex::compilar("[^]").err(), Some(ErroDeRegex::NaoSuportado));
        assert!(casa(r"^[\]]$", "]"), "`]` escapado continua literal");
    }

    #[test]
    fn alternancia() {
        assert!(casa("^(foo|bar)$", "foo"));
        assert!(casa("^(foo|bar)$", "bar"));
        assert!(!casa("^(foo|bar)$", "baz"));
        assert!(casa("^a(b|c)d$", "acd"));
        assert!(casa("a|ab", "ab"), "casamento total tenta o ramo seguinte quando o primeiro sobra texto");
    }

    #[test]
    fn escapes() {
        assert!(casa(r"^a\.c$", "a.c"));
        assert!(!casa(r"^a\.c$", "abc"), "`\\.` é ponto literal");
        assert!(casa(r"^a\\c$", r"a\c"));
        assert!(casa(r"^\$$", "$"), "`\\$` no fim é cifrão literal, não âncora");
    }

    #[test]
    fn padrao_de_path_realista() {
        let p = r"^base\\gameplay\\.*\.ent$";
        assert!(casa(p, r"base\gameplay\v.ent"));
        assert!(casa(p, r"base\gameplay\npc\joe.ent"));
        assert!(!casa(p, r"base\outro\v.ent"));
        assert!(!casa(p, r"base\gameplay\v.mesh"));
    }

    #[test]
    fn fora_do_subconjunto_devolve_erro_em_vez_de_casar_errado() {
        use ErroDeRegex::*;
        for (p, esperado) in [
            ("a{2,3}", NaoSuportado),
            ("(abc", Invalido),
            ("abc)", Invalido),
            ("*abc", Invalido),
            ("[abc", Invalido),
            ("a**", Invalido),
            ("a\\", Invalido),
            ("a*?", NaoSuportado),
            ("(?:a)", NaoSuportado),
            ("(?=a)", NaoSuportado),
            ("[[:digit:]]", NaoSuportado),
        ] {
            assert_eq!(Regex::compilar(p).err(), Some(esperado), "padrão {p:?}");
        }
    }

    #[test]
    fn escape_alfanumerico_e_recusado() {
        for p in [r"^\d+$", r"\w", r"\s", r"\b", r"(a)\1", r"\n", r"[\w]", r"[a-\d]", r"[\d-z]"] {
            assert_eq!(Regex::compilar(p).err(), Some(ErroDeRegex::NaoSuportado), "padrão {p:?}");
        }
        assert_eq!(restarget::casa(r"regex:^\d+$", "123"), None);
        assert_eq!(restarget::casa(r"regex:^\d+$", "ddd"), None);
        // Não alfanumérico continua literal, inclusive `\_`.
        assert!(Regex::compilar(r"\_").is_ok());
        assert_eq!(restarget::casa(r"regex:^a\.b\-c$", "a.b-c"), Some(true));
    }

    #[test]
    fn ancora_e_assercao_no_ramo_e_no_grupo() {
        assert_eq!(restarget::casa(r"regex:^base\\.*\.ent$|^ep1\\.*\.ent$", r"ep1\v.ent"), Some(true));
        assert_eq!(restarget::casa(r"regex:^base\\.*\.ent$|^ep1\\.*\.ent$", r"base\v.ent"), Some(true));
        assert_eq!(restarget::casa("regex:(^a)", "a"), Some(true));
        assert_eq!(restarget::casa("regex:a$|b", "a"), Some(true));
        assert_eq!(restarget::casa("regex:a$b", "a$b"), Some(false), "`$` no meio é asserção, não cifrão");
        assert_eq!(Regex::compilar("^*a").err(), Some(ErroDeRegex::Invalido));
        assert_eq!(Regex::compilar("a$+").err(), Some(ErroDeRegex::Invalido));
    }

    #[test]
    fn cifrao_depois_de_barra_escapada_e_ancora() {
        assert_eq!(restarget::casa(r"regex:^a\\$", r"a\"), Some(true));
        assert_eq!(restarget::casa(r"regex:^a\\$", r"a\$"), Some(false));
        assert_eq!(restarget::casa(r"regex:^a\\\$$", r"a\$"), Some(true), "3 barras: `\\\\` e `\\$`");
    }

    #[test]
    fn faixa_invertida_e_invalida() {
        assert_eq!(Regex::compilar("[z-a]").err(), Some(ErroDeRegex::Invalido));
        assert_eq!(restarget::casa("regex:^[z-a]$", "m"), None);
        assert!(casa("^[a-a]$", "a"));
    }

    #[test]
    fn complexidade_e_limitada_na_compilacao() {
        let a2100 = "a".repeat(2100);
        assert_eq!(restarget::casa(&format!("regex:^{a2100}$"), &a2100), Some(true), "literal longo não é complexo");
        assert_eq!(Regex::compilar(&"a".repeat(PADRAO_MAX + 1)).err(), Some(ErroDeRegex::Complexo));
        let fundo = "(".repeat(ANINHAMENTO_MAX as usize + 1) + "a" + &")".repeat(ANINHAMENTO_MAX as usize + 1);
        assert_eq!(Regex::compilar(&fundo).err(), Some(ErroDeRegex::Complexo));
        let raso = "(".repeat(ANINHAMENTO_MAX as usize) + "a" + &")".repeat(ANINHAMENTO_MAX as usize);
        assert!(Regex::compilar(&raso).expect("no teto compila").casa("a"));
        // Padrão do mod que estouraria a pilha do parser recursivo.
        assert_eq!(Regex::compilar(&("(".repeat(100_000) + &")".repeat(100_000))).err(), Some(ErroDeRegex::Complexo));
    }

    #[test]
    fn teto_de_aninhamento_cabe_na_pilha_de_thread_secundaria() {
        // 512 KiB é a pilha padrão de thread secundária no macOS; o despacho de `Resource/Load` roda fora da thread principal.
        let n = ANINHAMENTO_MAX as usize;
        let p = "(".repeat(n) + "a" + &")*".repeat(n);
        let r = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || Regex::compilar(&p).map(|r| (r.casa("aaa"), r.casa("b"))))
            .expect("thread")
            .join()
            .expect("compilar no teto não pode estourar a pilha");
        assert_eq!(r, Ok((true, false)));
    }

    #[test]
    fn padrao_patologico_termina() {
        let r = Regex::compilar("^(a*)*b$").expect("compila");
        assert!(!r.casa(&"a".repeat(40)));
        assert!(r.casa(&format!("{}b", "a".repeat(40))));
    }

    #[test]
    fn iteracao_obrigatoria_de_largura_zero() {
        // ECMAScript só corta iteração vazia depois do mínimo: `(b*)+` aceita zero `b`.
        assert_eq!(restarget::casa("regex:^a(b*)+c$", "ac"), Some(true));
        assert_eq!(restarget::casa("regex:^a(b*)+c$", "abbc"), Some(true));
    }

    #[test]
    fn quantificadores_encadeados_terminam_em_tempo_previsivel() {
        // Sob backtracking, 8 `.*` adjacentes contra 100 bytes são ~3,5·10¹¹ caminhos.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r1 = restarget::casa("regex:^.*.*.*.*.*.*.*.*x$", &"a".repeat(100));
            let r2 = restarget::casa("regex:.*.*.*x", &"a".repeat(200));
            let _ = tx.send((r1, r2));
        });
        let r = rx.recv_timeout(std::time::Duration::from_secs(2)).expect("casamento não terminou em 2 s");
        assert_eq!(r, (Some(false), Some(false)));
    }

    #[test]
    fn unidade_e_o_byte_como_std_string() {
        // `std::regex` sobre `std::string`: `é` são 2 bytes em UTF-8, e `.` consome 1.
        assert!(!casa("^a.b$", "aéb"));
        assert!(casa("^a..b$", "aéb"));
        assert!(casa("^aéb$", "aéb"));
        assert_eq!(Regex::compilar("[à-é]").err(), Some(ErroDeRegex::NaoSuportado));
    }

    /// `cargo test --release -- --ignored --nocapture bench_regex_match`. Padrões dos consumidores
    /// reais (c6, smoke `#137`) mais dois patológicos; mede só o casamento, regex já compilada.
    #[test]
    #[ignore]
    fn bench_regex_match() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};
        let joe = r"base\gameplay\npc\joe.ent";
        let deco = r"base\environment\decoration\furniture\chairs\chair_a\chair_a_01.mesh";
        let a40 = "a".repeat(40);
        let a24 = "a".repeat(24);
        let casos: [(&str, &str, u32); 7] = [
            (r"^base\\gameplay\\.*\.ent$", joe, 200_000),
            (r"^base\\gameplay\\.*\.ent$", r"base\outro\v.ent", 200_000),
            (r"^base\\environment\\decoration\\.*$", deco, 200_000),
            (r"^(foo|bar)$", "bar", 200_000),
            (r"^[a-z_\\]+\.mesh$", deco, 200_000),
            (r"^(a*)*b$", &a40, 200),
            (r"^.*.*.*.*.*x$", &a24, 20),
        ];
        let mut total = Duration::ZERO;
        for (padrao, texto, n) in casos {
            let r = Regex::compilar(padrao).expect("compila");
            let t0 = Instant::now();
            let mut casou = 0u32;
            for _ in 0..n {
                if r.casa(black_box(texto)) {
                    casou += 1;
                }
            }
            let dt = t0.elapsed();
            total += dt;
            println!("bench_regex_match {padrao:?} len={} n={n}: {:.1} ns/iter casou={casou}", texto.len(), dt.as_nanos() as f64 / f64::from(n));
        }
        println!("bench_regex_match total: {:.1} ms", total.as_secs_f64() * 1e3);
    }

    #[test]
    fn equivalente_ao_glob_para_o_mesmo_padrao() {
        // A regex que `wildcard_para_regex` produz tem que aceitar o mesmo conjunto que o glob —
        // é o que garante que as duas rotas do `#137` concordam.
        for (padrao, alvos) in [
            ("base\\gameplay\\*.ent", vec![(r"base\gameplay\v.ent", true), (r"base\outro\v.ent", false)]),
            ("*.mesh", vec![("a.mesh", true), ("a.ent", false)]),
            ("exato.ent", vec![("exato.ent", true), ("xexato.ent", false)]),
            ("*é*", vec![("aéb", true), ("ae", false)]),
        ] {
            let rx = restarget::wildcard_para_regex(padrao);
            let r = Regex::compilar(&rx).unwrap_or_else(|e| panic!("não compilou: {rx}: {e}"));
            for (alvo, esperado) in alvos {
                assert_eq!(r.casa(alvo), esperado, "padrao={padrao} rx={rx} alvo={alvo}");
                assert_eq!(
                    restarget::casa(padrao, alvo),
                    Some(esperado),
                    "glob e regex divergiram em padrao={padrao} alvo={alvo}"
                );
            }
        }
    }
}
