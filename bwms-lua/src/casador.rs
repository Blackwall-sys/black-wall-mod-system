//! Casador de padrões do Lua 5.1 (Manual §5.4.1), sobre bytes e com as classes do locale "C".
//!
//! Desenho: o padrão é compilado uma vez numa lista plana de `Item` — toda malformação (`%` no fim, conjunto sem `]`, `%b` incompleto,
//! `%f` sem `[`, parênteses desbalanceados, retroreferência a captura inexistente ou ainda aberta, capturas demais) vira erro aqui, antes
//! de olhar o texto. O casamento é uma máquina de volta atrás com pilha EXPLÍCITA no heap: nada recorre na pilha nativa, então padrão
//! nenhum estoura a pilha da thread do jogo. Cada item quantificado deixa no máximo um ponto de volta vivo, então a pilha cresce no máximo
//! até o número de itens do padrão; `LIMITE_DE_PONTOS_DE_VOLTA` é o teto de segurança que faz o padrão gigante virar erro em vez de
//! consumir memória sem fim — é o "limite de profundidade" que a spec pede, na forma que este desenho tem.
//! As capturas mudam por uma trilha de desfazer: cada ponto de volta guarda o tamanho da trilha e, ao voltar, as capturas voltam ao valor
//! daquele instante.

/// 64 e não 32: mod que monta padrão por `string.rep` passa de 32 sem ser abuso, e cada captura custa só 16 B no casamento.
pub(crate) const LIMITE_DE_CAPTURAS: usize = 64;

/// Teto da pilha de volta atrás (ver o comentário do módulo); padrão com mais itens quantificados que isso é recusado.
pub(crate) const LIMITE_DE_PONTOS_DE_VOLTA: usize = 100_000;

/// Conjunto de bytes como mapa de 256 bits: classe, `[conjunto]` e complemento viram a mesma coisa, e testar um byte é um acesso.
#[derive(Clone, Debug, PartialEq)]
struct Conjunto([u64; 4]);

impl Conjunto {
    fn vazio() -> Self {
        Conjunto([0; 4])
    }

    fn todos() -> Self {
        Conjunto([u64::MAX; 4])
    }

    fn de_byte(b: u8) -> Self {
        let mut c = Conjunto::vazio();
        c.por(b);
        c
    }

    fn de_predicado(p: impl Fn(u8) -> bool) -> Self {
        let mut c = Conjunto::vazio();
        for b in 0..=255u8 {
            if p(b) {
                c.por(b);
            }
        }
        c
    }

    fn por(&mut self, b: u8) {
        self.0[usize::from(b >> 6)] |= 1 << (b & 63);
    }

    fn tem(&self, b: u8) -> bool {
        self.0[usize::from(b >> 6)] & (1 << (b & 63)) != 0
    }

    fn inverter(&mut self) {
        for p in &mut self.0 {
            *p = !*p;
        }
    }

    fn unir(&mut self, o: &Conjunto) {
        for (a, b) in self.0.iter_mut().zip(o.0.iter()) {
            *a |= b;
        }
    }
}

/// `%a`, `%d`… no locale "C": só ASCII conta; byte ≥ 128 cai sempre no complemento. Letra maiúscula = complemento da minúscula.
/// `None` = a letra não nomeia classe e vale como ela mesma.
fn classe_nomeada(letra: u8) -> Option<Conjunto> {
    let base: fn(u8) -> bool = match letra.to_ascii_lowercase() {
        b'a' => |b| b.is_ascii_alphabetic(),
        b'c' => |b| b.is_ascii_control(),
        b'd' => |b| b.is_ascii_digit(),
        b'l' => |b| b.is_ascii_lowercase(),
        b'p' => |b| b.is_ascii_punctuation(),
        // `is_ascii_whitespace` do Rust deixa o \v (0x0B) de fora; o `isspace` do C o inclui.
        b's' => |b| b == b' ' || (0x09..=0x0D).contains(&b),
        b'u' => |b| b.is_ascii_uppercase(),
        b'w' => |b| b.is_ascii_alphanumeric(),
        b'x' => |b| b.is_ascii_hexdigit(),
        b'z' => |b| b == 0,
        _ => return None,
    };
    let mut c = Conjunto::de_predicado(base);
    if letra.is_ascii_uppercase() {
        c.inverter();
    }
    Some(c)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Quantidade {
    Um,
    /// `*`
    ZeroOuMais,
    /// `+`
    UmOuMais,
    /// `-`
    ZeroOuMaisCurto,
    /// `?`
    ZeroOuUm,
}

#[derive(Debug)]
enum Item {
    Classe(Conjunto, Quantidade),
    AbreCaptura(usize),
    FechaCaptura(usize),
    /// `()`: guarda a posição corrente.
    Posicao(usize),
    /// `%1`…`%9`.
    Retro(usize),
    /// `%bxy`.
    Balanceado(u8, u8),
    /// `%f[conjunto]`.
    Fronteira(Conjunto),
    /// `$` no fim do padrão.
    FimDoTexto,
}

/// O que uma captura devolve: o trecho do texto ou, para `()`, a posição 1-based.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Trecho {
    Texto(usize, usize),
    Posicao(usize),
}

#[derive(Debug)]
pub(crate) struct Casamento {
    /// Índices 0-based, `fim` exclusivo.
    pub(crate) inicio: usize,
    pub(crate) fim: usize,
    pub(crate) capturas: Vec<Trecho>,
}

#[derive(Debug)]
pub(crate) struct Padrao {
    itens: Vec<Item>,
    ancorado: bool,
    de_posicao: Vec<bool>,
}

/// Captura ainda aberta no meio do casamento.
const ABERTA: usize = usize::MAX;

enum Volta {
    /// `*`/`+`: o item já comeu `n + 1` bytes a partir de `base`; ao falhar o resto, tenta com `n`.
    Recuar { seguinte: usize, base: usize, n: usize, minimo: usize },
    /// `-`: tentou parar em `s`; ao falhar o resto, come mais um byte se a classe deixar.
    Estender { classe: usize, s: usize },
    /// `?`: comeu o byte; ao falhar o resto, segue sem ele.
    Pular { seguinte: usize, s: usize },
}

struct PontoDeVolta {
    volta: Volta,
    trilha: usize,
}

/// `[` em `p[i]`: devolve o conjunto e o índice depois do `]`.
fn ler_conjunto(p: &[u8], i: usize) -> Result<(Conjunto, usize), String> {
    let mut j = i + 1;
    let negado = p.get(j) == Some(&b'^');
    if negado {
        j += 1;
    }
    let mut c = Conjunto::vazio();
    let mut primeiro = true;
    loop {
        let Some(&b) = p.get(j) else { return Err("padrão malformado: conjunto sem ']'".into()) };
        // `]` logo depois de `[` ou `[^` é byte do conjunto, não o fecho.
        if b == b']' && !primeiro {
            break;
        }
        primeiro = false;
        if b == b'%' {
            let Some(&esc) = p.get(j + 1) else { return Err("padrão malformado: conjunto sem ']'".into()) };
            match classe_nomeada(esc) {
                Some(cl) => c.unir(&cl),
                None => c.por(esc),
            }
            j += 2;
        } else if p.get(j + 1) == Some(&b'-') && p.get(j + 2).is_some_and(|&f| f != b']') {
            // Intervalo pelo valor do byte; ponta de cima menor que a de baixo dá intervalo vazio.
            for x in b..=p[j + 2] {
                c.por(x);
            }
            j += 3;
        } else {
            c.por(b);
            j += 1;
        }
    }
    if negado {
        c.inverter();
    }
    Ok((c, j + 1))
}

/// Classe de um byte em `p[i]` (literal, `.`, `%x`, `[conjunto]`): devolve o conjunto e o índice seguinte.
fn ler_classe(p: &[u8], i: usize) -> Result<(Conjunto, usize), String> {
    match p[i] {
        b'.' => Ok((Conjunto::todos(), i + 1)),
        b'[' => ler_conjunto(p, i),
        b'%' => {
            let Some(&esc) = p.get(i + 1) else { return Err("padrão malformado: termina em '%'".into()) };
            Ok((classe_nomeada(esc).unwrap_or_else(|| Conjunto::de_byte(esc)), i + 2))
        }
        b => Ok((Conjunto::de_byte(b), i + 1)),
    }
}

impl Padrao {
    /// `ancora_permitida = false` é o `gmatch`: lá o `^` inicial é byte literal.
    pub(crate) fn compilar(p: &[u8], ancora_permitida: bool) -> Result<Padrao, String> {
        let mut itens = Vec::new();
        let mut i = 0;
        let ancorado = ancora_permitida && p.first() == Some(&b'^');
        if ancorado {
            i = 1;
        }
        let mut abertas: Vec<usize> = Vec::new();
        let mut fechada: Vec<bool> = Vec::new();
        let mut de_posicao: Vec<bool> = Vec::new();
        while i < p.len() {
            match p[i] {
                b'(' => {
                    let n = fechada.len();
                    if n >= LIMITE_DE_CAPTURAS {
                        return Err(format!("padrão com capturas demais (máximo {LIMITE_DE_CAPTURAS})"));
                    }
                    if p.get(i + 1) == Some(&b')') {
                        fechada.push(true);
                        de_posicao.push(true);
                        itens.push(Item::Posicao(n));
                        i += 2;
                    } else {
                        fechada.push(false);
                        de_posicao.push(false);
                        abertas.push(n);
                        itens.push(Item::AbreCaptura(n));
                        i += 1;
                    }
                    continue;
                }
                b')' => {
                    let n = abertas.pop().ok_or("padrão inválido: ')' sem '(' correspondente")?;
                    fechada[n] = true;
                    itens.push(Item::FechaCaptura(n));
                    i += 1;
                    continue;
                }
                b'$' if i + 1 == p.len() => {
                    itens.push(Item::FimDoTexto);
                    i += 1;
                    continue;
                }
                b'%' => match p.get(i + 1) {
                    None => return Err("padrão malformado: termina em '%'".into()),
                    Some(b'b') => {
                        let (Some(&x), Some(&y)) = (p.get(i + 2), p.get(i + 3)) else {
                            return Err("padrão malformado: '%b' pede dois bytes".into());
                        };
                        itens.push(Item::Balanceado(x, y));
                        i += 4;
                        continue;
                    }
                    Some(b'f') => {
                        if p.get(i + 2) != Some(&b'[') {
                            return Err("padrão malformado: '%f' pede um '[conjunto]'".into());
                        }
                        let (c, prox) = ler_conjunto(p, i + 2)?;
                        itens.push(Item::Fronteira(c));
                        i = prox;
                        continue;
                    }
                    Some(&d) if d.is_ascii_digit() => {
                        // `%0` não referencia captura; `()` não tem texto para repetir.
                        let n = usize::from(d - b'0');
                        let valida = n >= 1 && fechada.get(n - 1) == Some(&true) && de_posicao.get(n - 1) == Some(&false);
                        if !valida {
                            return Err(format!("padrão inválido: '%{n}' não é captura fechada"));
                        }
                        itens.push(Item::Retro(n - 1));
                        i += 2;
                        continue;
                    }
                    Some(_) => {}
                },
                _ => {}
            }
            // Quantificador sem classe antes (início do padrão, depois de captura, de `%b`, `%f`, `%1`) cai aqui e vale como byte literal.
            let (c, prox) = ler_classe(p, i)?;
            let q = match p.get(prox) {
                Some(b'*') => Quantidade::ZeroOuMais,
                Some(b'+') => Quantidade::UmOuMais,
                Some(b'-') => Quantidade::ZeroOuMaisCurto,
                Some(b'?') => Quantidade::ZeroOuUm,
                _ => Quantidade::Um,
            };
            i = if q == Quantidade::Um { prox } else { prox + 1 };
            itens.push(Item::Classe(c, q));
        }
        if !abertas.is_empty() {
            return Err("padrão inválido: '(' sem ')'".into());
        }
        Ok(Padrao { itens, ancorado, de_posicao })
    }

    pub(crate) fn ancorado(&self) -> bool {
        self.ancorado
    }

    /// Primeiro casamento que começa em `de` ou depois (só em `de` se ancorado). `de` pode ser `t.len()`: padrão vazio casa no fim.
    pub(crate) fn procurar(&self, t: &[u8], de: usize) -> Result<Option<Casamento>, String> {
        let mut ini = de;
        while ini <= t.len() {
            if let Some(c) = self.casar_em(t, ini)? {
                return Ok(Some(c));
            }
            if self.ancorado {
                break;
            }
            ini += 1;
        }
        Ok(None)
    }

    /// Casamento que começa exatamente em `ini`.
    pub(crate) fn casar_em(&self, t: &[u8], ini: usize) -> Result<Option<Casamento>, String> {
        let mut caps: Vec<(usize, usize)> = vec![(0, ABERTA); self.de_posicao.len()];
        let mut trilha: Vec<(usize, (usize, usize))> = Vec::new();
        let mut pilha: Vec<PontoDeVolta> = Vec::new();
        let (mut item, mut s) = (0usize, ini);
        let empilhar = |pilha: &mut Vec<PontoDeVolta>, volta: Volta, trilha: usize| -> Result<(), String> {
            if pilha.len() >= LIMITE_DE_PONTOS_DE_VOLTA {
                return Err("padrão complexo demais: limite de volta atrás".into());
            }
            pilha.push(PontoDeVolta { volta, trilha });
            Ok(())
        };
        let gravar = |caps: &mut Vec<(usize, usize)>, trilha: &mut Vec<(usize, (usize, usize))>, k: usize, v: (usize, usize)| {
            trilha.push((k, caps[k]));
            caps[k] = v;
        };
        loop {
            let Some(it) = self.itens.get(item) else {
                let capturas = caps
                    .iter()
                    .zip(&self.de_posicao)
                    .map(|(&(a, b), &pos)| if pos { Trecho::Posicao(a + 1) } else { Trecho::Texto(a, b) })
                    .collect();
                return Ok(Some(Casamento { inicio: ini, fim: s, capturas }));
            };
            let deu = match it {
                Item::Classe(c, q) => match q {
                    Quantidade::Um => {
                        let ok = t.get(s).is_some_and(|&b| c.tem(b));
                        if ok {
                            s += 1;
                        }
                        ok
                    }
                    Quantidade::ZeroOuUm => {
                        if t.get(s).is_some_and(|&b| c.tem(b)) {
                            empilhar(&mut pilha, Volta::Pular { seguinte: item + 1, s }, trilha.len())?;
                            s += 1;
                        }
                        true
                    }
                    Quantidade::ZeroOuMais | Quantidade::UmOuMais => {
                        let minimo = usize::from(*q == Quantidade::UmOuMais);
                        let n = t[s..].iter().take_while(|&&b| c.tem(b)).count();
                        if n < minimo {
                            false
                        } else {
                            if n > minimo {
                                empilhar(&mut pilha, Volta::Recuar { seguinte: item + 1, base: s, n: n - 1, minimo }, trilha.len())?;
                            }
                            s += n;
                            true
                        }
                    }
                    Quantidade::ZeroOuMaisCurto => {
                        empilhar(&mut pilha, Volta::Estender { classe: item, s }, trilha.len())?;
                        true
                    }
                },
                Item::AbreCaptura(k) => {
                    gravar(&mut caps, &mut trilha, *k, (s, ABERTA));
                    true
                }
                Item::FechaCaptura(k) => {
                    let a = caps[*k].0;
                    gravar(&mut caps, &mut trilha, *k, (a, s));
                    true
                }
                Item::Posicao(k) => {
                    gravar(&mut caps, &mut trilha, *k, (s, s));
                    true
                }
                Item::Retro(k) => {
                    let (a, b) = caps[*k];
                    let ok = t[s..].starts_with(&t[a..b]);
                    if ok {
                        s += b - a;
                    }
                    ok
                }
                Item::Balanceado(x, y) => match fim_balanceado(t, s, *x, *y) {
                    Some(f) => {
                        s = f;
                        true
                    }
                    None => false,
                },
                Item::Fronteira(c) => {
                    // Fora do texto conta o byte 0: `%f[%z]` casa o fim.
                    let antes = if s == 0 { 0 } else { t[s - 1] };
                    let agora = t.get(s).copied().unwrap_or(0);
                    !c.tem(antes) && c.tem(agora)
                }
                Item::FimDoTexto => s == t.len(),
            };
            if deu {
                item += 1;
                continue;
            }
            loop {
                let Some(PontoDeVolta { volta, trilha: marca }) = pilha.pop() else { return Ok(None) };
                while trilha.len() > marca {
                    if let Some((k, v)) = trilha.pop() {
                        caps[k] = v;
                    }
                }
                match volta {
                    Volta::Pular { seguinte, s: sv } => {
                        item = seguinte;
                        s = sv;
                        break;
                    }
                    Volta::Recuar { seguinte, base, n, minimo } => {
                        if n > minimo {
                            empilhar(&mut pilha, Volta::Recuar { seguinte, base, n: n - 1, minimo }, marca)?;
                        }
                        item = seguinte;
                        s = base + n;
                        break;
                    }
                    Volta::Estender { classe, s: sv } => {
                        let Item::Classe(c, _) = &self.itens[classe] else { continue };
                        if t.get(sv).is_some_and(|&b| c.tem(b)) {
                            empilhar(&mut pilha, Volta::Estender { classe, s: sv + 1 }, marca)?;
                            item = classe + 1;
                            s = sv + 1;
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// `%bxy` a partir de `s`: índice depois do `y` que zera a contagem. Com `x == y` o fecho é testado antes da abertura, então `%b""` casa
/// um texto entre aspas.
fn fim_balanceado(t: &[u8], s: usize, x: u8, y: u8) -> Option<usize> {
    if t.get(s) != Some(&x) {
        return None;
    }
    let mut nivel = 1usize;
    for (j, &b) in t.iter().enumerate().skip(s + 1) {
        if b == y {
            nivel -= 1;
            if nivel == 0 {
                return Some(j + 1);
            }
        } else if b == x {
            nivel += 1;
        }
    }
    None
}

#[cfg(test)]
mod testes {
    use super::*;

    fn casa(t: &str, p: &str) -> Option<(usize, usize, Vec<Trecho>)> {
        Padrao::compilar(p.as_bytes(), true).expect("padrão válido").procurar(t.as_bytes(), 0).expect("sem erro").map(|c| (c.inicio, c.fim, c.capturas))
    }

    fn trecho<'a>(t: &'a str, p: &str) -> Option<&'a str> {
        casa(t, p).map(|(a, b, _)| &t[a..b])
    }

    #[test]
    fn classes_e_conjuntos() {
        assert_eq!(trecho("Hello42", "[a-z]+"), Some("ello"));
        assert_eq!(trecho("x-y", "[a%-]+"), Some("-"));
        assert_eq!(trecho("abc^", "[^^a]+"), Some("bc"));
        assert_eq!(trecho("a]b", "[]]"), Some("]"));
        assert_eq!(trecho("a-b", "[a-]+"), Some("a-"));
        assert_eq!(trecho("f00 bar", "[%d%s]+"), Some("00 "));
        assert_eq!(trecho("..ab12CD", "%W+"), Some(".."));
        assert_eq!(trecho("a1!B ", "%A+"), Some("1!"));
        assert_eq!(trecho("tab\x0bx", "%s"), Some("\x0b"));
        assert_eq!(trecho("ação", "%a+"), Some("a"));
        assert_eq!(casa("a\0b", "%z").map(|c| c.0), Some(1));
    }

    #[test]
    fn quantificadores() {
        assert_eq!(trecho("aaa", "a*"), Some("aaa"));
        assert_eq!(trecho("aaa", "a-"), Some(""));
        assert_eq!(trecho("<a><b>", "<(.-)>"), Some("<a>"));
        assert_eq!(trecho("<a><b>", "<(.*)>"), Some("<a><b>"));
        assert_eq!(trecho("color colour", "colou?r"), Some("color"));
        assert_eq!(trecho("b", "a+b"), None);
        assert_eq!(trecho("abc", "a.-c$"), Some("abc"));
        // Quantificador sem classe antes é literal.
        assert_eq!(trecho("a+b", "+"), Some("+"));
        assert_eq!(trecho("(*)", "(*)"), Some("*"));
    }

    #[test]
    fn ancoras() {
        assert_eq!(trecho("abc", "^b"), None);
        assert_eq!(trecho("a$b", "$b"), Some("$b"));
        assert_eq!(trecho("x^y", "x^y"), Some("x^y"));
        let p = Padrao::compilar(b"^a", false).expect("válido");
        assert_eq!(p.procurar(b"a^a", 0).expect("ok").map(|c| (c.inicio, c.fim)), Some((1, 3)));
    }

    #[test]
    fn capturas_retro_balanceado_fronteira() {
        assert_eq!(casa("abc", "b()").map(|c| c.2), Some(vec![Trecho::Posicao(3)]));
        assert_eq!(trecho("abcabc", "(abc)%1"), Some("abcabc"));
        assert_eq!(trecho("xyyx", "(.)(.)%2%1"), Some("xyyx"));
        assert_eq!(trecho("f(a(b)c) d", "%b()"), Some("(a(b)c)"));
        assert_eq!(trecho("[x[y]", "%b[]"), Some("[y]"));
        assert_eq!(trecho("(open", "%b()"), None);
        assert_eq!(trecho("say \"hi\" now", "%b\"\""), Some("\"hi\""));
        assert_eq!(trecho("THE (quick) fox", "%f[%a]%a+"), Some("THE"));
        assert_eq!(casa("foo", "%f[%z]").map(|c| (c.0, c.1)), Some((3, 3)));
    }

    #[test]
    fn volta_atras_restaura_capturas() {
        // A 1ª tentativa fecha a captura em "aa"; a volta tem de desfazer antes de fechar em "a".
        let (_, _, c) = casa("aab", "(a*)ab").expect("casa");
        assert_eq!(c, vec![Trecho::Texto(0, 1)]);
    }

    #[test]
    fn erros_de_padrao() {
        for p in ["[a", "%", "(a", "a)", "%b", "%ba", "%f", "%fa", "%1", "(a%1)", "%0", "()%1", "[%"] {
            assert!(Padrao::compilar(p.as_bytes(), true).is_err(), "{p} deveria falhar");
        }
        assert!(Padrao::compilar("(a)".repeat(LIMITE_DE_CAPTURAS).as_bytes(), true).is_ok());
        assert!(Padrao::compilar("(a)".repeat(LIMITE_DE_CAPTURAS + 1).as_bytes(), true).is_err());
    }

    #[test]
    fn padrao_longo_nao_estoura_a_pilha() {
        let t = "a".repeat(200_000);
        assert_eq!(trecho(&t, "a-$").map(str::len), Some(200_000));
        assert_eq!(trecho(&t, ".*$").map(str::len), Some(200_000));
        let p = "a?".repeat(LIMITE_DE_PONTOS_DE_VOLTA + 1);
        let r = Padrao::compilar(p.as_bytes(), true).expect("válido").procurar(t.as_bytes(), 0);
        assert!(r.is_err());
    }
}
