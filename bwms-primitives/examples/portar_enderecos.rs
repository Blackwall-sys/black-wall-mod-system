//! Porta os endereços que o fonte do BWMS cita de um build do jogo pra outro (Steam 2.3.1 → Steam 2.5.0) e diz por que cada um.
//!
//!     cargo run --release --example portar_enderecos -- <binário origem> <binário destino> <pasta src> <saída .tsv> [manuais .tsv]
//!
//! Saída: `endereco_origem\tendereco_destino\tmetodo\tconfianca\tnota`, uma linha por endereço citado no fonte. Sem prova, o destino
//! fica vazio e a nota diz o motivo. Provas (coluna `nota`, `provas=`):
//! - `simbolo`: a função (ou o dado) que contém o endereço tem o mesmo nome único nos dois binários.
//! - `assinatura`: a janela de `enderecos::gerar` casa uma vez só no destino.
//! - `chamador:<função>`: uma função casada chama o alvo, e a chamada alinhada na imagem dela cai no candidato.
//! - `chamados:k/n`: das n chamadas do alvo a funções casadas, k vão às imagens delas a partir do candidato.
//! - `string`: a função referencia uma string do `__cstring` que só ela referencia, nos dois.
//! - `corpo`: o corpo mascarado da função inteira é único nos dois e igual.
//! - `ref:<função>`: dado tirado do `ADRP`+`ADD`/`LDR` no lugar correspondente de uma função casada.
//! - `tabela:<dado>`: função tirada da mesma entrada de uma tabela de ponteiros (vtable) casada.
//! - `manual`: linha do arquivo de manuais (RE à mão), com a prova na nota.
//! O recuo dentro da função (gancho no meio) é conferido pelas instruções em volta (`corpo igual` ou janela única na função).
//! Confiança `alta` = símbolo exato, ou 2 provas independentes concordando e nenhuma discordando; `media` = 1 prova; `baixa` = discordância.

use bwms_primitives::enderecos::{self as en, Imagem, Secao, LINK_BASE};
use std::collections::{BTreeMap, HashMap, HashSet};

/// `LC_FUNCTION_STARTS` (`<mach-o/loader.h>`).
const LC_FUNCTION_STARTS: u32 = 0x26;
/// Até onde um `ADD`/`LDR` depois do `ADRP` ainda é o uso dele.
const USO_MAX: usize = 4;
/// Quanto a referência pode cair além do dado (campo dentro do objeto) e ainda contar pra ele.
const DESLOC_MAX: i64 = 0x1000;
/// Alinhamento de listas maiores que isto (produto dos tamanhos) é só posicional.
const LCS_MAX: usize = 4_000_000;
/// Entradas lidas de uma tabela de ponteiros casada.
const TABELA_MAX: usize = 1024;

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Alvo {
    Funcao(u32),
    /// Stub de import, pelo hash do nome.
    Stub(u64),
}

struct Bin<'a> {
    arq: &'a [u8],
    img: Imagem<'a>,
    m: Vec<u32>,
    ini: Vec<u64>,
    nome_de: HashMap<u64, String>,
    por_nome: HashMap<String, u64>,
    simbolos_ord: Vec<(u64, String)>,
    stub: HashMap<u64, u64>,
    cham: Vec<Vec<(u32, Alvo)>>,
    refs: Vec<Vec<(u32, u64)>>,
    chamadores: Vec<Vec<u32>>,
    hash: Vec<u64>,
    cstr: Option<Secao>,
    /// Inicializadores estáticos (`__init_offsets`), na ordem do link: índices de função.
    inits: Vec<u32>,
}

fn fnv(h: u64, w: u32) -> u64 {
    let mut h = h;
    for b in w.to_le_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

impl<'a> Bin<'a> {
    fn novo(arq_inteiro: &'a [u8]) -> Bin<'a> {
        let arq = en::fatia_arm64(arq_inteiro).expect("Mach-O arm64");
        let img = Imagem::do_arquivo(arq_inteiro).expect("Mach-O arm64 com __text");
        let n = img.texto.len() / 4;
        let m: Vec<u32> = (0..n).map(|i| en::mascarada(img.texto, i)).collect();
        let fim_texto = img.texto_vm + img.texto.len() as u64;
        let mut ini = Vec::new();
        if let Some(c) = bwms_primitives::macho::comandos_de_carga(arq).and_then(|mut it| it.find(|c| c.cmd == LC_FUNCTION_STARTS)) {
            let (off, tam) = (u32le(arq, c.offset + 8) as usize, u32le(arq, c.offset + 12) as usize);
            let d = &arq[off..off + tam];
            let (mut a, mut i) = (LINK_BASE, 0);
            while i < d.len() {
                let (mut x, mut s) = (0u64, 0);
                loop {
                    let b = d[i];
                    i += 1;
                    x |= ((b & 0x7f) as u64) << s;
                    s += 7;
                    if b & 0x80 == 0 || i >= d.len() {
                        break;
                    }
                }
                if x == 0 {
                    break;
                }
                a += x;
                if a >= img.texto_vm && a < fim_texto {
                    ini.push(a);
                }
            }
        }
        ini.sort_unstable();
        ini.dedup();
        let mut contagem: HashMap<&str, usize> = HashMap::new();
        for (_, s) in &img.simbolos {
            *contagem.entry(s.as_str()).or_insert(0) += 1;
        }
        let mut nome_de = HashMap::new();
        let mut por_nome = HashMap::new();
        for (a, s) in &img.simbolos {
            if contagem[s.as_str()] == 1 && *a >= LINK_BASE {
                nome_de.insert(*a, s.clone());
                por_nome.insert(s.clone(), *a);
            }
        }
        let mut simbolos_ord: Vec<(u64, String)> = img.simbolos.iter().filter(|(a, _)| *a >= LINK_BASE).cloned().collect();
        simbolos_ord.sort();
        let stub: HashMap<u64, u64> = img.stubs.iter().map(|(a, s)| (*a, bwms_primitives::fnv1a64(s.as_bytes()))).collect();
        let cstr = img.secoes.iter().find(|s| s.segmento == "__TEXT" && s.nome == "__cstring").cloned();
        let mut b = Bin {
            arq,
            img,
            m,
            ini,
            nome_de,
            por_nome,
            simbolos_ord,
            stub,
            cham: Vec::new(),
            refs: Vec::new(),
            chamadores: Vec::new(),
            hash: Vec::new(),
            cstr,
            inits: Vec::new(),
        };
        b.varrer();
        if let Some(s) = b.img.secoes.iter().find(|s| s.nome == "__init_offsets").cloned() {
            let d = &b.arq[s.arquivo as usize..(s.arquivo + s.tamanho) as usize];
            b.inits = d.chunks_exact(4).filter_map(|c| b.inicio(LINK_BASE + u32le(c, 0) as u64)).map(|f| f as u32).collect();
        }
        b
    }

    fn pos(&self, a: u64) -> usize {
        ((a - self.img.texto_vm) / 4) as usize
    }

    fn vm(&self, p: usize) -> u64 {
        self.img.texto_vm + p as u64 * 4
    }

    /// Faixa de palavras `[ini, fim)` da função `f`.
    fn faixa(&self, f: usize) -> (usize, usize) {
        let ini = self.pos(self.ini[f]);
        let fim = if f + 1 < self.ini.len() {
            self.pos(self.ini[f + 1])
        } else {
            self.img.texto.len() / 4
        };
        (ini, fim)
    }

    /// A função que contém `a` (no `__text`).
    fn funcao_de(&self, a: u64) -> Option<usize> {
        if a < self.img.texto_vm || a >= self.img.texto_vm + self.img.texto.len() as u64 {
            return None;
        }
        let k = self.ini.partition_point(|x| *x <= a);
        (k > 0).then(|| k - 1)
    }

    fn inicio(&self, a: u64) -> Option<usize> {
        self.ini.binary_search(&a).ok()
    }

    fn no_texto(&self, a: u64) -> bool {
        a >= self.img.texto_vm && a < self.img.texto_vm + self.img.texto.len() as u64
    }

    fn varrer(&mut self) {
        let nf = self.ini.len();
        let mut cham = vec![Vec::new(); nf];
        let mut refs = vec![Vec::new(); nf];
        let mut hash = vec![0xcbf2_9ce4_8422_2325u64; nf];
        let n = self.img.texto.len() / 4;
        let mut f = 0usize;
        let primeira = self.ini.first().map(|a| self.pos(*a)).unwrap_or(n);
        for i in primeira..n {
            while f + 1 < nf && self.pos(self.ini[f + 1]) <= i {
                f += 1;
            }
            let rel = (i - self.pos(self.ini[f])) as u32;
            hash[f] = fnv(hash[f], self.m[i]);
            let w = en::palavra(self.img.texto, i);
            let pc = self.vm(i);
            if w & 0x7C00_0000 == 0x1400_0000 {
                let imm = ((w & 0x03FF_FFFF) as i64) << 38 >> 36;
                let t = (pc as i64 + imm) as u64;
                let bl = w & 0x8000_0000 != 0;
                if let Some(h) = self.stub.get(&t) {
                    cham[f].push((rel, Alvo::Stub(*h)));
                } else if let Some(g) = self.inicio(t) {
                    if bl || g != f {
                        cham[f].push((rel, Alvo::Funcao(g as u32)));
                    }
                }
                continue;
            }
            if w & 0x9F00_0000 == 0x1000_0000 {
                let imm = (((w >> 5) & 0x7FFFF) << 2 | (w >> 29) & 3) as i64;
                let imm = (imm << 43) >> 43;
                refs[f].push((rel, (pc as i64 + imm) as u64));
                continue;
            }
            if w & 0x9F00_0000 == 0x9000_0000 {
                let pagina = en::pagina_do_adrp(w, pc);
                let rd = w & 0x1F;
                for d in 1..=USO_MAX {
                    if i + d >= n {
                        break;
                    }
                    if let Some(off) = en::offset_de_uso(en::palavra(self.img.texto, i + d), rd) {
                        refs[f].push((rel, pagina + off));
                        break;
                    }
                }
            }
        }
        let mut chamadores = vec![Vec::new(); nf];
        for (f, l) in cham.iter().enumerate() {
            for (_, a) in l {
                if let Alvo::Funcao(g) = a {
                    chamadores[*g as usize].push(f as u32);
                }
            }
        }
        for l in &mut chamadores {
            l.sort_unstable();
            l.dedup();
        }
        self.cham = cham;
        self.refs = refs;
        self.chamadores = chamadores;
        self.hash = hash;
    }

    fn string_em(&self, a: u64) -> Option<&'a [u8]> {
        let s = self.cstr.as_ref()?;
        if a < s.vm || a >= s.vm + s.tamanho {
            return None;
        }
        let o = (s.arquivo + (a - s.vm)) as usize;
        let resto = self.arq.get(o..(s.arquivo + s.tamanho) as usize)?;
        Some(&resto[..resto.iter().position(|c| *c == 0).unwrap_or(resto.len())])
    }

    /// O ponteiro gravado em `a` (seção de dado), com o fixup encadeado decodificado: rebase só (bind volta `None`).
    fn ponteiro_em(&self, a: u64) -> Option<u64> {
        let s = self
            .img
            .secoes
            .iter()
            .find(|s| a >= s.vm && a + 8 <= s.vm + s.tamanho && s.segmento != "__TEXT" && s.arquivo != 0)?;
        let o = (s.arquivo + (a - s.vm)) as usize;
        let v = u64::from_le_bytes(self.arq.get(o..o + 8)?.try_into().ok()?);
        if v >> 63 != 0 {
            return None;
        }
        let t = v & 0xF_FFFF_FFFF;
        Some(if t < LINK_BASE { LINK_BASE + t } else { t })
    }

    /// Os 8 bytes gravados em `a` numa seção de dado, sem decodificar.
    fn bruto_em(&self, a: u64) -> Option<u64> {
        let s = self.img.secoes.iter().find(|s| a >= s.vm && a + 8 <= s.vm + s.tamanho && s.segmento != "__TEXT" && s.arquivo != 0)?;
        let o = (s.arquivo + (a - s.vm)) as usize;
        Some(u64::from_le_bytes(self.arq.get(o..o + 8)?.try_into().ok()?))
    }

    fn em_dado(&self, a: u64) -> bool {
        !self.no_texto(a) && self.img.secoes.iter().any(|s| a >= s.vm && a < s.vm + s.tamanho.max(1))
    }

    /// Símbolo que contém `a`: `(início, nome)`, se o nome for único e o próximo símbolo vier depois de `a`.
    fn simbolo_que_contem(&self, a: u64) -> Option<(u64, &str)> {
        let k = self.simbolos_ord.partition_point(|(x, _)| *x <= a);
        if k == 0 {
            return None;
        }
        let (x, nome) = &self.simbolos_ord[k - 1];
        if a - x >= DESLOC_MAX as u64 {
            return None;
        }
        self.nome_de.get(x).map(|_| (*x, nome.as_str()))
    }
}

#[derive(Default)]
struct Casamento {
    ab: HashMap<u32, u32>,
    ba: HashMap<u32, u32>,
    /// Provas do casamento de cada função da origem.
    provas: HashMap<u32, Vec<String>>,
    dado_ab: HashMap<u64, u64>,
    dado_ba: HashMap<u64, u64>,
    dado_provas: HashMap<u64, Vec<String>>,
}

type Votos = HashMap<u32, HashMap<u32, HashSet<String>>>;
type VotosDado = HashMap<u64, HashMap<u64, HashSet<String>>>;

/// Pares de posições `(i, j)` alinhadas entre duas listas de fichas: `Some(ficha)` conhecida, `None` desconhecida. Âncoras = maior
/// subsequência comum das conhecidas; entre duas âncoras, trechos do mesmo tamanho são pareados posição a posição.
fn alinhar<T: Eq + std::hash::Hash>(a: &[Option<T>], b: &[Option<T>]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let mut ancoras: Vec<(usize, usize)> = Vec::new();
    if n * m <= LCS_MAX && n > 0 && m > 0 {
        let mut dp = vec![0u16; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                dp[i * (m + 1) + j] = match (&a[i], &b[j]) {
                    (Some(x), Some(y)) if x == y => dp[(i + 1) * (m + 1) + j + 1] + 1,
                    _ => dp[(i + 1) * (m + 1) + j].max(dp[i * (m + 1) + j + 1]),
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            match (&a[i], &b[j]) {
                (Some(x), Some(y)) if x == y && dp[i * (m + 1) + j] == dp[(i + 1) * (m + 1) + j + 1] + 1 => {
                    ancoras.push((i, j));
                    i += 1;
                    j += 1;
                }
                _ => {
                    if dp[(i + 1) * (m + 1) + j] >= dp[i * (m + 1) + j + 1] {
                        i += 1
                    } else {
                        j += 1
                    }
                }
            }
        }
    } else if n > 0 && m > 0 {
        // Lista grande: âncoras = maior sequência crescente das fichas que aparecem uma vez só em cada lista.
        let mut vezes: HashMap<&T, (usize, usize, usize)> = HashMap::new();
        for (i, x) in a.iter().enumerate() {
            if let Some(x) = x {
                let e = vezes.entry(x).or_insert((0, 0, 0));
                e.0 += 1;
                e.2 = i;
            }
        }
        let mut pares: Vec<(usize, usize)> = Vec::new();
        let mut em_b: HashMap<&T, (usize, usize)> = HashMap::new();
        for (j, y) in b.iter().enumerate() {
            if let Some(y) = y {
                let e = em_b.entry(y).or_insert((0, j));
                e.0 += 1;
            }
        }
        for (t, (na, _, i)) in &vezes {
            if *na == 1 {
                if let Some((1, j)) = em_b.get(t) {
                    pares.push((*i, *j));
                }
            }
        }
        pares.sort_unstable();
        let mut caudas: Vec<usize> = Vec::new();
        let mut antes = vec![usize::MAX; pares.len()];
        for (k, (_, j)) in pares.iter().enumerate() {
            let p = caudas.partition_point(|c| pares[*c].1 < *j);
            if p > 0 {
                antes[k] = caudas[p - 1];
            }
            if p == caudas.len() {
                caudas.push(k);
            } else {
                caudas[p] = k;
            }
        }
        let mut k = caudas.last().copied().unwrap_or(usize::MAX);
        while k != usize::MAX {
            ancoras.push(pares[k]);
            k = antes[k];
        }
        ancoras.reverse();
    }
    let mut v = Vec::new();
    let (mut pi, mut pj) = (0usize, 0usize);
    for (ai, aj) in ancoras.iter().copied().chain(std::iter::once((n, m))) {
        if ai - pi == aj - pj {
            for k in 0..ai - pi {
                v.push((pi + k, pj + k));
            }
        }
        if ai < n {
            v.push((ai, aj));
        }
        pi = ai + 1;
        pj = aj + 1;
    }
    v
}

/// Tamanhos compatíveis (até 1,5×) ou corpos iguais: filtro contra par posicional errado num trecho que mudou.
fn compativeis(a: &Bin, fa: usize, b: &Bin, fb: usize) -> bool {
    if a.hash[fa] == b.hash[fb] {
        return true;
    }
    let (ia, ea) = a.faixa(fa);
    let (ib, eb) = b.faixa(fb);
    let (la, lb) = ((ea - ia) as f64, (eb - ib) as f64);
    la.max(lb) <= la.min(lb) * 1.5 + 4.0
}

/// Segunda fase: aceita o par de uma âncora só mesmo com função pouco parecida, marcado `fraco` (a confiança do endereço vem das provas
/// de vizinhança conferidas depois).
static RELAXADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Tamanhos até `r` vezes um do outro.
fn tamanho_ate(a: &Bin, fa: usize, b: &Bin, fb: usize, r: f64) -> bool {
    let (ia, ea) = a.faixa(fa);
    let (ib, eb) = b.faixa(fb);
    let (la, lb) = ((ea - ia) as f64, (eb - ib) as f64);
    la.max(lb) <= la.min(lb) * r + 4.0
}

/// As 4 primeiras instruções mascaradas iguais.
fn mesmo_prologo(a: &Bin, fa: usize, b: &Bin, fb: usize) -> bool {
    let (ia, ea) = a.faixa(fa);
    let (ib, eb) = b.faixa(fb);
    let k = 4.min(ea - ia).min(eb - ib);
    k == 4 && a.m[ia..ia + k] == b.m[ib..ib + k]
}

/// Função parecida: corpo igual, ou tamanho compatível com as 4 primeiras palavras mascaradas iguais e o nº de chamadas perto.
fn forte(a: &Bin, fa: usize, b: &Bin, fb: usize) -> bool {
    if a.hash[fa] == b.hash[fb] {
        return true;
    }
    let (ia, ea) = a.faixa(fa);
    let (ib, eb) = b.faixa(fb);
    let k = 4.min(ea - ia).min(eb - ib);
    let (na, nb) = (a.cham[fa].len(), b.cham[fb].len());
    compativeis(a, fa, b, fb) && a.m[ia..ia + k] == b.m[ib..ib + k] && na.abs_diff(nb) <= 2.max(na.max(nb) / 4)
}

fn votar(v: &mut Votos, a: u32, b: u32, prova: String) {
    v.entry(a).or_default().entry(b).or_default().insert(prova);
}

/// Aceita os votos sem disputa: o melhor candidato de cada função tem de ser o único, livre no destino, e não disputado por outra.
/// O candidato único, ou o de maioria clara: 3+ âncoras e pelo menos 4× as do segundo (função muito chamada, em que o alinhamento de
/// uma ou outra chamadora erra). A maioria fica nas provas.
fn maioria<T: Copy>(mut cands: Vec<(T, HashSet<String>)>) -> Option<(T, HashSet<String>)> {
    cands.sort_by_key(|(_, p)| std::cmp::Reverse(p.len()));
    let (b, p) = cands.first()?.clone();
    let segundo = cands.get(1).map(|(_, p)| p.len()).unwrap_or(0);
    if segundo == 0 {
        return Some((b, p));
    }
    if p.len() >= 3 && p.len() >= 4 * segundo {
        let total: usize = cands.iter().map(|(_, p)| p.len()).sum();
        let mut p = p;
        p.insert(format!("maioria:{}/{total}", p.len()));
        return Some((b, p));
    }
    None
}

fn aceitar(ba: &Bin, bb: &Bin, c: &mut Casamento, votos: Votos) -> usize {
    let mut escolha: HashMap<u32, (u32, Vec<String>)> = HashMap::new();
    let mut por_b: HashMap<u32, usize> = HashMap::new();
    for (a, cands) in votos {
        if c.ab.contains_key(&a) {
            continue;
        }
        let cands: Vec<(u32, HashSet<String>)> = cands.into_iter().filter(|(b, _)| !c.ba.contains_key(b)).collect();
        let Some((b, p)) = maioria(cands) else { continue };
        // Uma âncora só basta com função parecida (mesmo corpo, ou prólogo e nº de chamadas parecidos); senão, 2 âncoras diferentes.
        let nomeado = p.iter().any(|s| s == "simbolo" || s == "corpo");
        let mut p = p;
        if !nomeado && p.len() < 2 && !forte(ba, a as usize, bb, b as usize) {
            if !RELAXADO.load(std::sync::atomic::Ordering::Relaxed) {
                continue;
            }
            p.insert("fraco".into());
        }
        *por_b.entry(b).or_insert(0) += 1;
        let mut p: Vec<String> = p.into_iter().collect();
        p.sort();
        escolha.insert(a, (b, p));
    }
    let mut n = 0;
    for (a, (b, p)) in escolha {
        if por_b[&b] == 1 {
            c.ab.insert(a, b);
            c.ba.insert(b, a);
            c.provas.insert(a, p);
            n += 1;
        }
    }
    n
}

fn aceitar_dados(c: &mut Casamento, votos: VotosDado) -> Vec<(u64, u64)> {
    let mut escolha: HashMap<u64, (u64, Vec<String>)> = HashMap::new();
    let mut por_b: HashMap<u64, usize> = HashMap::new();
    for (a, cands) in votos {
        if c.dado_ab.contains_key(&a) {
            continue;
        }
        let cands: Vec<(u64, HashSet<String>)> = cands.into_iter().filter(|(b, _)| !c.dado_ba.contains_key(b)).collect();
        let Some((b, p)) = maioria(cands) else { continue };
        *por_b.entry(b).or_insert(0) += 1;
        let mut p: Vec<String> = p.into_iter().collect();
        p.sort();
        escolha.insert(a, (b, p));
    }
    let mut novos = Vec::new();
    for (a, (b, p)) in escolha {
        if por_b[&b] == 1 {
            c.dado_ab.insert(a, b);
            c.dado_ba.insert(b, a);
            c.dado_provas.insert(a, p);
            novos.push((a, b));
        }
    }
    novos
}

/// Sementes: nome único, string única e corpo único.
fn sementes(a: &Bin, b: &Bin, c: &mut Casamento, retidos: &mut Vec<(u32, u32)>) {
    let mut votos: Votos = HashMap::new();
    let validar = std::env::var_os("BWMS_PORTAR_VALIDAR").is_some();
    for (x, nome) in &a.nome_de {
        // Validação: metade dos nomes fica de fora das sementes e serve de gabarito do casamento pelo resto.
        if validar && bwms_primitives::fnv1a64(nome.as_bytes()) & 1 == 1 {
            if let (Some(fa), Some(fb)) = (a.inicio(*x), b.por_nome.get(nome).and_then(|y| b.inicio(*y))) {
                retidos.push((fa as u32, fb as u32));
            }
            continue;
        }
        if let (Some(fa), Some(y)) = (a.inicio(*x), b.por_nome.get(nome)) {
            if let Some(fb) = b.inicio(*y) {
                votar(&mut votos, fa as u32, fb as u32, "simbolo".into());
            }
        }
    }
    let mut so_a: HashMap<&[u8], Vec<u32>> = HashMap::new();
    let mut so_b: HashMap<&[u8], Vec<u32>> = HashMap::new();
    for (bin, mapa) in [(a, &mut so_a), (b, &mut so_b)] {
        for (f, l) in bin.refs.iter().enumerate() {
            for (_, r) in l {
                if let Some(s) = bin.string_em(*r) {
                    if s.len() >= 4 {
                        let e = mapa.entry(s).or_default();
                        if e.last() != Some(&(f as u32)) {
                            e.push(f as u32);
                        }
                    }
                }
            }
        }
    }
    for (s, fa) in &so_a {
        if let (Some(fb), [fa]) = (so_b.get(s), fa.as_slice()) {
            if let [fb] = fb.as_slice() {
                votar(&mut votos, *fa, *fb, format!("string:{:x}", bwms_primitives::fnv1a64(s) & 0xffff_ffff));
            }
        }
    }
    let mut ha: HashMap<u64, Vec<u32>> = HashMap::new();
    let mut hb: HashMap<u64, Vec<u32>> = HashMap::new();
    for (bin, mapa) in [(a, &mut ha), (b, &mut hb)] {
        for f in 0..bin.ini.len() {
            let (i, e) = bin.faixa(f);
            if e - i >= 12 {
                mapa.entry(bin.hash[f]).or_default().push(f as u32);
            }
        }
    }
    for (h, fa) in &ha {
        if let (Some(fb), [fa]) = (hb.get(h), fa.as_slice()) {
            if let [fb] = fb.as_slice() {
                votar(&mut votos, *fa, *fb, "corpo".into());
            }
        }
    }
    // Semente com prova discordante (nome numa, string noutra) fica de fora: `aceitar` só pega candidato único.
    let n = aceitar(a, b, c, votos);
    eprintln!("sementes: {n} funções");
}

fn fichas_chamadas(bin: &Bin, f: usize, c: &Casamento, origem: bool) -> Vec<Option<(u8, u64)>> {
    bin.cham[f]
        .iter()
        .map(|(_, a)| match a {
            Alvo::Stub(h) => Some((0, *h)),
            Alvo::Funcao(g) => {
                if origem {
                    c.ab.get(g).map(|y| (1, *y as u64))
                } else {
                    c.ba.contains_key(g).then_some((1, *g as u64))
                }
            }
        })
        .collect()
}

fn fichas_refs(bin: &Bin, f: usize, c: &Casamento, origem: bool, casa_funcao: &HashMap<u32, u32>) -> Vec<Option<(u8, u64)>> {
    bin.refs[f]
        .iter()
        .map(|(_, r)| {
            if let Some(s) = bin.string_em(*r) {
                return Some((2, bwms_primitives::fnv1a64(s)));
            }
            if let Some(g) = bin.inicio(*r) {
                return if origem {
                    casa_funcao.get(&(g as u32)).map(|y| (1, *y as u64))
                } else {
                    c.ba.contains_key(&(g as u32)).then_some((1, g as u64))
                };
            }
            if origem {
                c.dado_ab.get(r).map(|y| (3, *y))
            } else {
                c.dado_ba.contains_key(r).then_some((3, *r))
            }
        })
        .collect()
}

/// Uma rodada: para cada par casado, alinha chamadas e referências e vota nos pares que saem do alinhamento.
fn rodada(a: &Bin, b: &Bin, c: &mut Casamento) -> (usize, usize) {
    let mut votos: Votos = HashMap::new();
    let mut votos_dado: VotosDado = HashMap::new();
    let pares: Vec<(u32, u32)> = c.ab.iter().map(|(x, y)| (*x, *y)).collect();
    for (fa, fb) in &pares {
        let (fa, fb) = (*fa as usize, *fb as usize);
        let ancora = format!("{:#x}", a.ini[fa]);
        let ta = fichas_chamadas(a, fa, c, true);
        let tb = fichas_chamadas(b, fb, c, false);
        for (i, j) in alinhar(&ta, &tb) {
            if let (Alvo::Funcao(x), Alvo::Funcao(y)) = (a.cham[fa][i].1, b.cham[fb][j].1) {
                if ta[i].is_none() && tb[j].is_none() && compativeis(a, x as usize, b, y as usize) {
                    votar(&mut votos, x, y, format!("chamador:{ancora}"));
                }
            }
        }
        let ra = fichas_refs(a, fa, c, true, &c.ab);
        let rb = fichas_refs(b, fb, c, false, &c.ab);
        for (i, j) in alinhar(&ra, &rb) {
            if ra[i].is_some() || rb[j].is_some() {
                continue;
            }
            let (x, y) = (a.refs[fa][i].1, b.refs[fb][j].1);
            match (a.inicio(x), b.inicio(y)) {
                (Some(gx), Some(gy)) => {
                    if compativeis(a, gx, b, gy) {
                        votar(&mut votos, gx as u32, gy as u32, format!("ponteiro:{ancora}"));
                    }
                }
                (None, None) if a.em_dado(x) && b.em_dado(y) => {
                    votos_dado.entry(x).or_default().entry(y).or_default().insert(format!("ref:{ancora}"));
                }
                _ => {}
            }
        }
    }
    // Inicializadores estáticos: a mesma ordem de link nos dois.
    let ia: Vec<Option<(u8, u64)>> = a.inits.iter().map(|f| c.ab.get(f).map(|y| (1, *y as u64))).collect();
    let ib: Vec<Option<(u8, u64)>> = b.inits.iter().map(|f| c.ba.contains_key(f).then_some((1, *f as u64))).collect();
    for (i, j) in alinhar(&ia, &ib) {
        let (x, y) = (a.inits[i], b.inits[j]);
        if ia[i].is_none() && ib[j].is_none() && compativeis(a, x as usize, b, y as usize) {
            votar(&mut votos, x, y, format!("init:{i}"));
        }
    }
    let nf = aceitar(a, b, c, votos);
    let novos = aceitar_dados(c, votos_dado);
    // Tabela de ponteiros (vtable) casada: as entradas que apontam pra início de função se casam na mesma ordem.
    let mut votos: Votos = HashMap::new();
    for (x, y) in &novos {
        for k in 0..TABELA_MAX as u64 {
            // Entrada de import (bind, ex. `__cxa_pure_virtual`) nos dois: pula e segue; ponteiro pra texto nos dois: segue; o resto acaba a tabela.
            let (Some(rx), Some(ry)) = (a.bruto_em(x + 8 * k), b.bruto_em(y + 8 * k)) else { break };
            if rx >> 63 == 1 && ry >> 63 == 1 {
                continue;
            }
            let (Some(px), Some(py)) = (a.ponteiro_em(x + 8 * k), b.ponteiro_em(y + 8 * k)) else { break };
            if !a.no_texto(px) || !b.no_texto(py) {
                break;
            }
            let (Some(gx), Some(gy)) = (a.inicio(px), b.inicio(py)) else { continue };
            if compativeis(a, gx, b, gy) {
                votar(&mut votos, gx as u32, gy as u32, format!("tabela:{x:#x}"));
            }
        }
    }
    let nt = aceitar(a, b, c, votos);
    (nf + nt, novos.len())
}

/// Função sem chamada direta (só por vtable/tabela): cada vizinho casado numa tabela de ponteiros da origem diz onde a mesma tabela está
/// no destino, e a entrada à mesma distância dele é o candidato. Vale o candidato com 2+ vizinhos diferentes e o dobro do segundo.
fn rodada_vizinhos(a: &Bin, b: &Bin, c: &mut Casamento, pa: &HashMap<u64, Vec<u64>>, pb: &HashMap<u64, Vec<u64>>) -> usize {
    let mut pontos: HashMap<u32, HashMap<u32, HashSet<u64>>> = HashMap::new();
    for (alvo, slots) in pa {
        let Some(fa) = a.inicio(*alvo) else { continue };
        if c.ab.contains_key(&(fa as u32)) {
            continue;
        }
        for s in slots.iter().take(8) {
            for k in -16i64..=16 {
                if k == 0 {
                    continue;
                }
                let n = (*s as i64 + 8 * k) as u64;
                let Some(gn) = a.ponteiro_em(n).and_then(|p| a.inicio(p)) else {
                    continue;
                };
                let Some(gb) = c.ab.get(&(gn as u32)) else { continue };
                let Some(tslots) = pb.get(&b.ini[*gb as usize]) else { continue };
                if tslots.len() > 8 {
                    continue;
                }
                for t in tslots {
                    let m = (*t as i64 - 8 * k) as u64;
                    let Some(fb) = b.ponteiro_em(m).and_then(|p| b.inicio(p)) else {
                        continue;
                    };
                    if c.ba.contains_key(&(fb as u32)) || !compativeis(a, fa, b, fb) {
                        continue;
                    }
                    pontos.entry(fa as u32).or_default().entry(fb as u32).or_default().insert(n);
                }
            }
        }
    }
    let mut votos: Votos = HashMap::new();
    for (fa, cands) in pontos {
        let mut v: Vec<(u32, usize)> = cands.iter().map(|(b, s)| (*b, s.len())).collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        if v[0].1 >= 2 && (v.len() == 1 || v[1].1 * 2 < v[0].1) {
            votar(&mut votos, fa, v[0].0, format!("vizinhos:{}", v[0].1));
        }
    }
    aceitar(a, b, c, votos)
}

/// Recuo do alvo (palavras) dentro da função casada no destino, pelas instruções em volta.
fn recuo_no_destino(a: &Bin, fa: usize, d: usize, b: &Bin, fb: usize) -> Option<(usize, &'static str)> {
    if d == 0 {
        return Some((0, "inicio"));
    }
    let (sa, ea) = a.faixa(fa);
    let (sb, eb) = b.faixa(fb);
    if ea - sa == eb - sb && a.m[sa..ea] == b.m[sb..eb] {
        return Some((d, "corpo igual"));
    }
    let x = sa + d;
    let acha = |ini: usize, fim: usize| -> Vec<usize> {
        let w = &a.m[ini..fim];
        let t = fim - ini;
        if eb - sb < t {
            return Vec::new();
        }
        (sb..=eb - t).filter(|&p| b.m[p..p + t] == *w).collect()
    };
    for t in [8usize, 12, 16, 24, 32, 48, 64] {
        let t = t.min(ea - x);
        if t < 4 {
            break;
        }
        let v = acha(x, x + t);
        if v.len() == 1 {
            return Some((v[0] - sb, "janela unica na funcao"));
        }
        if v.is_empty() || x + t == ea {
            break;
        }
    }
    for t in [8usize, 12, 16, 24, 32, 48, 64] {
        let t = t.min(d + 1);
        if t < 4 {
            break;
        }
        let v = acha(x + 1 - t, x + 1);
        if v.len() == 1 {
            return Some((v[0] + t - 1 - sb, "janela unica antes do alvo"));
        }
        if v.is_empty() || t == d + 1 {
            break;
        }
    }
    // Âncoras: janelas de 6 palavras únicas na função da origem e na do destino, a menos de 256 palavras do alvo. O alvo vale se a âncora
    // de antes e a de depois têm o mesmo deslocamento (trecho sem inserção entre elas) e a palavra do alvo bate.
    let ancora = |p: usize| -> Option<usize> {
        if p < sa || p + 6 > ea {
            return None;
        }
        let w = &a.m[p..p + 6];
        if (sa..=ea - 6).filter(|&q| a.m[q..q + 6] == *w).take(2).count() != 1 {
            return None;
        }
        let v = acha(p, p + 6);
        (v.len() == 1).then(|| v[0])
    };
    let antes = (0..256usize)
        .filter_map(|k| x.checked_sub(k))
        .filter(|p| *p >= sa)
        .find_map(|p| ancora(p).map(|q| (p, q)));
    let depois = (1..256usize)
        .map(|k| x + k)
        .filter(|p| *p + 6 <= ea)
        .find_map(|p| ancora(p).map(|q| (p, q)));
    if let (Some((p1, q1)), Some((p2, q2))) = (antes, depois) {
        if q2 > q1 && p2 - p1 == q2 - q1 && a.m[x] == b.m[q1 + (x - p1)] {
            return Some((q1 + (x - p1) - sb, "ancoras ao redor"));
        }
    }
    None
}

struct Linha {
    origem: u64,
    destino: Option<u64>,
    metodo: String,
    confianca: &'static str,
    nota: String,
}

/// Provas pós-casamento de que `fb` é a imagem de `fa`: chamadores casados que chamam `fb`, e chamados casados que `fb` chama.
/// Chamador que discorda é comum quando o destino inlinou a chamada; só conta contra se a maioria discorda.
fn provas_de_vizinhanca(a: &Bin, fa: usize, b: &Bin, fb: usize, c: &Casamento) -> (Option<(usize, usize, u64)>, Option<String>, usize) {
    let (mut sim, mut nao, mut primeiro) = (0usize, 0usize, 0u64);
    let mut contra = 0;
    for x in &a.chamadores[fa] {
        if let Some(y) = c.ab.get(x) {
            if b.cham[*y as usize].iter().any(|(_, t)| *t == Alvo::Funcao(fb as u32)) {
                if sim == 0 {
                    primeiro = a.ini[*x as usize];
                }
                sim += 1;
            } else {
                nao += 1;
            }
        }
    }
    if nao > sim && nao >= 2 {
        contra += 1;
    }
    let chamadores = (sim > 0).then_some((sim, sim + nao, primeiro));
    let alvos_b: HashSet<Alvo> = b.cham[fb].iter().map(|(_, t)| *t).collect();
    let (mut k, mut n) = (0, 0);
    let mut vistos = HashSet::new();
    for (_, t) in &a.cham[fa] {
        if let Alvo::Funcao(g) = t {
            if let Some(y) = c.ab.get(g) {
                if vistos.insert(*g) {
                    n += 1;
                    k += alvos_b.contains(&Alvo::Funcao(*y)) as usize;
                }
            }
        }
    }
    let chamados = (n >= 2 && k * 10 >= n * 8).then(|| format!("chamados:{k}/{n}"));
    if n >= 3 && k * 2 < n {
        contra += 1;
    }
    (chamadores, chamados, contra)
}

fn confianca(simbolo_exato: bool, provas: usize, contra: usize) -> &'static str {
    if contra > 0 {
        "baixa"
    } else if simbolo_exato || provas >= 2 {
        "alta"
    } else {
        "media"
    }
}

/// Candidatos no destino pra uma função sem par, pela vizinhança: funções que chamam as imagens dos chamados dela (`chamados`), chamadas
/// sem par das imagens dos chamadores dela (`chamadores`) e funções que referenciam as mesmas strings (`strings`). Cada tipo é uma prova.
fn candidatos_por_vizinhanca(a: &Bin, b: &Bin, c: &Casamento, fa: usize, strings_b: &HashMap<&[u8], Vec<u32>>) -> Vec<(usize, [usize; 3])> {
    let mut pontos: HashMap<u32, [usize; 3]> = HashMap::new();
    let mut vistos = HashSet::new();
    for (_, t) in &a.cham[fa] {
        if let Alvo::Funcao(g) = t {
            if !vistos.insert(*g) {
                continue;
            }
            if let Some(y) = c.ab.get(g) {
                let chamadores_b = &b.chamadores[*y as usize];
                if chamadores_b.len() <= 64 {
                    for z in chamadores_b {
                        pontos.entry(*z).or_default()[0] += 1;
                    }
                }
            }
        }
    }
    for x in &a.chamadores[fa] {
        if let Some(y) = c.ab.get(x) {
            let mut ja = HashSet::new();
            for (_, t) in &b.cham[*y as usize] {
                if let Alvo::Funcao(z) = t {
                    if !c.ba.contains_key(z) && ja.insert(*z) {
                        pontos.entry(*z).or_default()[1] += 1;
                    }
                }
            }
        }
    }
    let mut ja = HashSet::new();
    for (_, r) in &a.refs[fa] {
        if let Some(s) = a.string_em(*r) {
            if s.len() >= 4 && ja.insert(s) {
                if let Some(l) = strings_b.get(s) {
                    if l.len() <= 16 {
                        for z in l {
                            pontos.entry(*z).or_default()[2] += 1;
                        }
                    }
                }
            }
        }
    }
    let mut v: Vec<(usize, [usize; 3])> =
        pontos.into_iter().filter(|(z, _)| !c.ba.contains_key(z) && tamanho_ate(a, fa, b, *z as usize, 3.0)).map(|(z, p)| (z as usize, p)).collect();
    v.sort_by_key(|(z, p)| (std::cmp::Reverse(p.iter().sum::<usize>()), *z));
    v.truncate(5);
    v
}

/// O melhor candidato da vizinhança vale se tem 3+ pontos, o dobro do segundo e pelo menos 2 tipos de prova (ou chamados 3+ sozinhos,
/// com as 4 primeiras instruções mascaradas iguais).
fn escolher_por_vizinhanca(a: &Bin, b: &Bin, fa: usize, v: &[(usize, [usize; 3])]) -> Option<(usize, Vec<String>)> {
    let (z, p) = v.first()?;
    let total: usize = p.iter().sum();
    let segundo = v.get(1).map(|(_, p)| p.iter().sum::<usize>()).unwrap_or(0);
    let tipos = p.iter().filter(|n| **n > 0).count();
    if total < 3 || total < 2 * segundo.max(1) || (tipos < 2 && !(p[0] >= 3 && mesmo_prologo(a, fa, b, *z))) {
        return None;
    }
    let nomes = ["chamados", "chamadores", "strings"];
    Some((*z, (0..3).filter(|k| p[*k] > 0).map(|k| format!("vizinhanca-{}:{}", nomes[k], p[k])).collect()))
}

/// Índices do destino pras provas por alvo.
struct Ctx<'x> {
    strings_b: HashMap<&'x [u8], Vec<u32>>,
    pa: HashMap<u64, Vec<u64>>,
    gramas_b: Vec<Vec<u32>>,
}

/// Trigramas (hash) da classe de cada instrução da função (a palavra sem registradores nem imediatos: os 11 bits altos), sem repetição.
fn gramas(bin: &Bin, f: usize) -> Vec<u32> {
    let (i, e) = bin.faixa(f);
    let mut v: Vec<u32> = (i..e.saturating_sub(2))
        .map(|k| {
            let c = |w: u32| w >> 21;
            c(en::palavra(bin.img.texto, k)).wrapping_mul(0x9E37_79B1) ^ c(en::palavra(bin.img.texto, k + 1)).rotate_left(11) ^ c(en::palavra(bin.img.texto, k + 2)).rotate_left(22)
        })
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn jaccard(x: &[u32], y: &[u32]) -> f64 {
    let (mut i, mut j, mut comum) = (0, 0, 0usize);
    while i < x.len() && j < y.len() {
        match x[i].cmp(&y[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                comum += 1;
                i += 1;
                j += 1;
            }
        }
    }
    let u = x.len() + y.len() - comum;
    if u == 0 { 0.0 } else { comum as f64 / u as f64 }
}

/// As 3 funções sem par do destino de corpo mais parecido (trigramas de classe de instrução), com tamanho compatível.
fn parecidas(a: &Bin, fa: usize, b: &Bin, c: &Casamento, gramas_b: &[Vec<u32>]) -> Vec<(usize, f64)> {
    let ga = gramas(a, fa);
    if ga.len() < 6 {
        return Vec::new();
    }
    let mut v: Vec<(usize, f64)> = (0..b.ini.len())
        .filter(|z| !c.ba.contains_key(&(*z as u32)) && compativeis(a, fa, b, *z))
        .map(|z| (z, jaccard(&ga, &gramas_b[z])))
        .collect();
    v.sort_by(|x, y| y.1.total_cmp(&x.1));
    v.truncate(3);
    v
}

/// Dados que a função referencia (strings pelo conteúdo, o resto pelo casamento de dados): `(batem, conhecidos)`.
fn conferir_dados(a: &Bin, fa: usize, b: &Bin, fb: usize, c: &Casamento) -> (usize, usize, Vec<String>) {
    let refs_b: HashSet<u64> = b.refs[fb].iter().map(|(_, r)| *r).collect();
    let strings_b: HashSet<&[u8]> = b.refs[fb].iter().filter_map(|(_, r)| b.string_em(*r)).collect();
    let (mut k, mut n) = (0, 0);
    let mut errados = Vec::new();
    let mut vistos = HashSet::new();
    for (_, r) in &a.refs[fa] {
        if !vistos.insert(*r) {
            continue;
        }
        if let Some(s) = a.string_em(*r) {
            n += 1;
            k += strings_b.contains(s) as usize;
        } else if a.secao_de(*r).is_some_and(|s| s.segmento == "__TEXT") {
            // Constante só de leitura (literal de 16 B, tabela): o compilador junta cópias iguais, então vale o conteúdo, não o endereço.
            if let Some(w) = a.bytes_em(*r, 16) {
                n += 1;
                k += b.refs[fb].iter().any(|(_, rb)| b.bytes_em(*rb, 16) == Some(w)) as usize;
            }
        } else if let Some(y) = c.dado_ab.get(r) {
            n += 1;
            if refs_b.contains(y) {
                k += 1;
            } else if errados.len() < 2 {
                errados.push(format!("{r:#x}->{y:#x}"));
            }
        }
    }
    (k, n, errados)
}

/// Chamadas a funções casadas: `(vão à imagem, casadas)`.
fn conferir_chamados(a: &Bin, fa: usize, b: &Bin, fb: usize, c: &Casamento) -> (usize, usize) {
    let alvos_b: HashSet<Alvo> = b.cham[fb].iter().map(|(_, t)| *t).collect();
    let (mut k, mut n) = (0, 0);
    let mut vistos = HashSet::new();
    for (_, t) in &a.cham[fa] {
        if let Alvo::Funcao(g) = t {
            if let Some(y) = c.ab.get(g) {
                if vistos.insert(*g) {
                    n += 1;
                    k += alvos_b.contains(&Alvo::Funcao(*y)) as usize;
                }
            }
        }
    }
    (k, n)
}

/// Provas e discordâncias das referências e chamadas de `fa` contra `fb` (conferência do candidato, venha de onde vier).
fn conferir(a: &Bin, fa: usize, b: &Bin, fb: usize, c: &Casamento) -> (Vec<String>, usize, usize) {
    // Dado que não bate é fraco contra (constantes duplicadas, tabelas que mudaram): só conta contra junto com chamadas que não batem.
    let (mut provas, mut fortes, mut contra) = (Vec::new(), 0, 0);
    let (kd, nd, errados) = conferir_dados(a, fa, b, fb, c);
    if nd > 0 {
        provas.push(if errados.is_empty() { format!("dados:{kd}/{nd}") } else { format!("dados:{kd}/{nd}(sem: {})", errados.join(" ")) });
        fortes += (kd * 10 >= nd * 8) as usize;
    }
    let (k, n) = conferir_chamados(a, fa, b, fb, c);
    if n > 0 {
        provas.push(format!("chamados:{k}/{n}"));
    }
    let chamados_ok = n >= 2 && k * 10 >= n * 8;
    fortes += chamados_ok as usize;
    if n >= 2 && k * 2 < n || nd >= 2 && kd == 0 && !chamados_ok {
        contra += 1;
    }
    (provas, fortes, contra)
}

fn portar_codigo(a: &Bin, b: &Bin, c: &Casamento, x: u64, assin: Option<u64>, ctx: &Ctx) -> Linha {
    let (strings_b, pa) = (&ctx.strings_b, &ctx.pa);
    let mut l = Linha {
        origem: x,
        destino: None,
        metodo: String::new(),
        confianca: "",
        nota: String::new(),
    };
    if let Some(h) = a.stub.get(&x) {
        let achados: Vec<u64> = b.stub.iter().filter(|(_, g)| *g == h).map(|(e, _)| *e).collect();
        let nome = a.img.stubs.iter().find(|(e, _)| *e == x).map(|(_, n)| n.clone()).unwrap_or_default();
        if let [y] = achados.as_slice() {
            l.destino = Some(*y);
            l.metodo = "simbolo".into();
            l.confianca = "alta";
            l.nota = format!("stub de import {nome}");
        } else {
            l.nota = format!("stub {nome}: {} no destino", achados.len());
        }
        return l;
    }
    let Some(fa) = a.funcao_de(x) else {
        l.nota = "fora das funções do __text".into();
        return l;
    };
    let d = a.pos(x) - a.pos(a.ini[fa]);
    let nome_a = a.nome_de.get(&a.ini[fa]).cloned();
    let mut extra: Vec<String> = Vec::new();
    let vizinhanca = if c.ab.contains_key(&(fa as u32)) { Vec::new() } else { candidatos_por_vizinhanca(a, b, c, fa, strings_b) };
    let mut parecida: Vec<(usize, f64)> = Vec::new();
    let mut escolhido = c.ab.get(&(fa as u32)).map(|y| *y as usize).or_else(|| {
        escolher_por_vizinhanca(a, b, fa, &vizinhanca).map(|(z, p)| {
            extra = p;
            z
        })
    });
    // Corpo parecido: sozinho com 0,9+ e folga de 0,1 sobre o segundo; ou 0,6+ quando é também o primeiro da vizinhança.
    if escolhido.is_none() {
        parecida = parecidas(a, fa, b, c, &ctx.gramas_b);
        if let Some((z, s)) = parecida.first().copied() {
            let folga = s - parecida.get(1).map(|p| p.1).unwrap_or(0.0);
            let viz = vizinhanca.first().filter(|(w, _)| *w == z).map(|(_, p)| *p);
            if s >= 0.9 && folga >= 0.1 || s >= 0.6 && viz.is_some() {
                extra.push(format!("parecida:{s:.2}"));
                if let Some(p) = viz {
                    let nomes = ["chamados", "chamadores", "strings"];
                    extra.extend((0..3).filter(|k| p[*k] > 0).map(|k| format!("vizinhanca-{}:{}", nomes[k], p[k])));
                }
                escolhido = Some(z);
            }
        }
    }
    let Some(fb) = escolhido else {
        let mut lista: Vec<String> = vizinhanca.iter().map(|(z, p)| format!("{:#x}{p:?}", b.ini[*z])).collect();
        lista.extend(parecida.iter().map(|(z, s)| format!("{:#x}~{s:.2}", b.ini[*z])));
        // Função não casada: só a assinatura, se houver.
        match assin {
            Some(r) => {
                // A janela mascarada não vê qual global o ADRP pega nem pra onde o BL vai: a função achada é conferida por isso.
                let (mut provas, mut fortes, mut contra) = (vec!["assinatura".to_string()], 1, 0);
                if let Some(g) = b.funcao_de(r) {
                    let (p, f, k) = conferir(a, fa, b, g, c);
                    provas.extend(p);
                    fortes += f;
                    contra += k;
                }
                l.destino = Some(r);
                l.metodo = "assinatura".into();
                l.confianca = confianca(false, fortes, contra);
                l.nota = format!("função {:#x}+{:#x} não casada; provas={}", a.ini[fa], d * 4, provas.join(","));
            }
            None => {
                l.nota = format!(
                    "função {:#x}+{:#x} sem par ({} chamadores, {} casados){}; candidatos [chamados,chamadores,strings] e ~parecida: {}; slots que apontam pra ela: {:x?}; refs de código: {:x?}",
                    a.ini[fa],
                    d * 4,
                    a.chamadores[fa].len(),
                    a.chamadores[fa].iter().filter(|g| c.ab.contains_key(g)).count(),
                    nome_a.map(|n| format!(" {n}")).unwrap_or_default(),
                    lista.join(" "),
                    pa.get(&a.ini[fa]).map(|v| v.iter().take(4).copied().collect::<Vec<u64>>()).unwrap_or_default(),
                    a.refs.iter().enumerate().filter(|(_, v)| v.iter().any(|(_, r)| *r == a.ini[fa])).map(|(g, _)| a.ini[g]).take(4).collect::<Vec<u64>>()
                );
            }
        }
        return l;
    };
    let Some((db, como)) = recuo_no_destino(a, fa, d, b, fb) else {
        l.nota = format!(
            "função {:#x} ↔ {:#x}, recuo {:#x} sem janela única no destino",
            a.ini[fa],
            b.ini[fb],
            d * 4
        );
        if let Some(r) = assin {
            l.destino = Some(r);
            l.metodo = "assinatura".into();
            l.confianca = "media";
            l.nota.push_str("; provas=assinatura");
        }
        return l;
    };
    let y = b.vm(b.pos(b.ini[fb]) + db);
    // As provas do casamento: as de nome/string/corpo inteiras; as de propagação resumidas por tipo (a âncora vai junto quando é uma só).
    let mut provas: Vec<String> = Vec::new();
    let mut fortes = 0usize;
    let mut por_tipo: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for p in c.provas.get(&(fa as u32)).map(|v| v.as_slice()).unwrap_or(&[]) {
        match p.split_once(':') {
            Some((t, ancora)) => por_tipo.entry(t).or_default().push(ancora),
            None => {
                provas.push(p.clone());
                fortes += (p != "fraco") as usize;
            }
        }
    }
    let (chamadores, chamados, mut contra) = if extra.is_empty() { provas_de_vizinhanca(a, fa, b, fb, c) } else { (None, None, 0) };
    for p in &extra {
        provas.push(p.clone());
        fortes += 1;
    }
    for (t, l) in &por_tipo {
        // Os chamadores da propagação são contados de novo (e conferidos) pela vizinhança.
        if *t == "chamador" && chamadores.is_some() {
            continue;
        }
        provas.push(if l.len() == 1 {
            format!("{t}:{}", l[0])
        } else {
            format!("{t}x{}", l.len())
        });
        if *t != "maioria" {
            fortes += l.len().min(2);
        }
    }
    if let Some((k, n, p)) = chamadores {
        provas.push(if k == 1 {
            format!("chamadores:1/{n}({p:#x})")
        } else {
            format!("chamadores:{k}/{n}")
        });
        fortes += k.min(2);
    }
    if let Some(p) = chamados {
        provas.push(p);
        fortes += 1;
    }
    let (k, n, errados) = conferir_dados(a, fa, b, fb, c);
    if n > 0 {
        provas.push(if errados.is_empty() { format!("dados:{k}/{n}") } else { format!("dados:{k}/{n}(sem: {})", errados.join(" ")) });
        fortes += (k * 10 >= n * 8) as usize;
    }
    let simbolo_exato = nome_a.as_ref().is_some_and(|n| b.nome_de.get(&b.ini[fb]) == Some(n));
    if simbolo_exato && !provas.iter().any(|p| p == "simbolo") {
        provas.push("simbolo".into());
        fortes += 1;
    }
    match assin {
        Some(r) if r == y => {
            provas.push("assinatura".into());
            fortes += 1;
        }
        Some(r) => {
            contra += 1;
            provas.push(format!("assinatura-discorda:{r:#x}"));
        }
        None => {}
    }
    l.destino = Some(y);
    l.metodo = metodo_principal(&provas, d > 0);
    l.confianca = confianca(simbolo_exato && d == 0 || simbolo_exato && como == "corpo igual", fortes, contra);
    l.nota = format!(
        "função {:#x}+{:#x} ↔ {:#x}+{:#x} ({como}){}; provas={}",
        a.ini[fa],
        d * 4,
        b.ini[fb],
        db * 4,
        nome_a.map(|n| format!(" {n}")).unwrap_or_default(),
        provas.join(",")
    );
    l
}

fn metodo_principal(provas: &[String], recuo: bool) -> String {
    let tem = |p: &str| provas.iter().any(|x| x == p || x.starts_with(&format!("{p}:")));
    let m = if tem("simbolo") {
        "simbolo"
    } else if tem("assinatura") {
        "assinatura"
    } else if ["chamador", "chamadores", "chamados", "ponteiro", "tabela"]
        .iter()
        .any(|p| provas.iter().any(|x| x.starts_with(p)))
    {
        "chamada"
    } else if tem("string") {
        "string"
    } else if tem("corpo") {
        "corpo"
    } else {
        "chamada"
    };
    if recuo {
        format!("{m}+recuo")
    } else {
        m.to_string()
    }
}

/// Os ponteiros (rebase) gravados nas seções de dado de `b`: alvo → lugares.
fn indice_de_ponteiros(b: &Bin) -> HashMap<u64, Vec<u64>> {
    let mut v: HashMap<u64, Vec<u64>> = HashMap::new();
    for s in b.img.secoes.iter().filter(|s| s.segmento != "__TEXT" && s.arquivo != 0) {
        let mut a = (s.vm + 7) & !7;
        while a + 8 <= s.vm + s.tamanho {
            if let Some(p) = b.ponteiro_em(a) {
                if p >= LINK_BASE && (b.no_texto(p) || b.em_dado(p)) {
                    v.entry(p).or_default().push(a);
                }
            }
            a += 8;
        }
    }
    v
}

impl Bin<'_> {
    fn secao_de(&self, a: u64) -> Option<&Secao> {
        self.img.secoes.iter().find(|s| a >= s.vm && a < s.vm + s.tamanho)
    }

    /// Bytes do arquivo a partir de `a`, até o fim da seção ou `n`.
    fn bytes_em(&self, a: u64, n: usize) -> Option<&[u8]> {
        let s = self.secao_de(a)?;
        if s.arquivo == 0 {
            return None;
        }
        let o = (s.arquivo + (a - s.vm)) as usize;
        let fim = (s.arquivo + s.tamanho) as usize;
        self.arq.get(o..(o + n).min(fim))
    }
}

/// O que um ponteiro da origem vale no destino: função casada, dado casado ou a mesma string.
fn imagem_do_ponteiro(a: &Bin, b: &Bin, c: &Casamento, p: u64) -> Option<u64> {
    if let Some(f) = a.inicio(p) {
        return c.ab.get(&(f as u32)).map(|g| b.ini[*g as usize]);
    }
    if let Some(y) = c.dado_ab.get(&p) {
        return Some(*y);
    }
    let s = a.string_em(p)?;
    let cs = b.cstr.as_ref()?;
    let todo = b.arq.get(cs.arquivo as usize..(cs.arquivo + cs.tamanho) as usize)?;
    let mut alvo = s.to_vec();
    alvo.push(0);
    let achados: Vec<usize> = todo
        .windows(alvo.len())
        .enumerate()
        .filter(|(i, w)| *w == alvo.as_slice() && (*i == 0 || todo[i - 1] == 0))
        .map(|(i, _)| i)
        .take(2)
        .collect();
    match achados.as_slice() {
        [i] => Some(cs.vm + *i as u64),
        _ => None,
    }
}

/// Endereço na mesma seção (pelo nome) do destino onde os bytes de `x` na origem aparecem uma vez só: string inteira (com o recuo dentro
/// dela) no `__cstring`, ou a menor janela única de 16 a 128 bytes noutra seção. Bytes que são ponteiros não servem (mudam com o layout).
fn pelo_conteudo(a: &Bin, b: &Bin, x: u64) -> Option<(u64, String)> {
    let sa = a.secao_de(x)?;
    let sb = b.img.secoes.iter().find(|s| s.segmento == sa.segmento && s.nome == sa.nome)?;
    let todo = b.arq.get(sb.arquivo as usize..(sb.arquivo + sb.tamanho) as usize)?;
    if sb.arquivo == 0 {
        return None;
    }
    if sa.nome == "__cstring" {
        let mut ini = x;
        while ini > sa.vm && a.bytes_em(ini - 1, 1)?[0] != 0 {
            ini -= 1;
        }
        let s = a.string_em(ini)?;
        let mut alvo = s.to_vec();
        alvo.push(0);
        let achados: Vec<usize> = todo
            .windows(alvo.len())
            .enumerate()
            .filter(|(i, w)| *w == alvo.as_slice() && (*i == 0 || todo[i - 1] == 0))
            .map(|(i, _)| i)
            .take(2)
            .collect();
        return match achados.as_slice() {
            [i] => Some((
                sb.vm + *i as u64 + (x - ini),
                format!("string \"{}\"+{:#x}", String::from_utf8_lossy(&s[..s.len().min(40)]), x - ini),
            )),
            _ => None,
        };
    }
    if sa.segmento != "__TEXT" {
        return None;
    }
    for n in [16usize, 32, 64, 128] {
        let w = a.bytes_em(x, n)?;
        if w.len() < n || w.iter().all(|b| *b == 0) {
            return None;
        }
        let achados: Vec<usize> = todo.windows(n).enumerate().filter(|(_, v)| *v == w).map(|(i, _)| i).take(2).collect();
        match achados.as_slice() {
            [i] => return Some((sb.vm + *i as u64, format!("bytes{n}"))),
            [] => return None,
            _ => {}
        }
    }
    None
}

fn portar_dado(a: &Bin, b: &Bin, c: &Casamento, x: u64, assin: Option<u64>, ptrs_b: &HashMap<u64, Vec<u64>>) -> Linha {
    let mut l = Linha {
        origem: x,
        destino: None,
        metodo: String::new(),
        confianca: "",
        nota: String::new(),
    };
    // candidato -> provas; `campo` = a referência cai num campo do dado, e não nele (prova fraca: o layout do objeto pode ter mudado)
    let mut cands: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    let mut simbolo_exato = None;
    if let Some((s, nome)) = a.simbolo_que_contem(x) {
        if let Some(y) = b.por_nome.get(nome) {
            cands.entry(y + (x - s)).or_default().push(format!("simbolo:{nome}+{:#x}", x - s));
            if x == s {
                simbolo_exato = Some(*y);
            }
        }
    }
    if let Some(y) = c.dado_ab.get(&x) {
        let p = c.dado_provas.get(&x).cloned().unwrap_or_default();
        cands.entry(*y).or_default().push(if p.len() == 1 {
            format!("alinhado:{}", p[0])
        } else {
            format!("alinhadox{}", p.len())
        });
    }
    // Ponteiro gravado no dado (GOT, entrada de vtable): o lugar no destino que guarda a imagem do mesmo alvo, se for um só.
    let conteudo = a.ponteiro_em(x).and_then(|p| imagem_do_ponteiro(a, b, c, p).map(|q| (p, q)));
    if let Some((p, q)) = conteudo {
        if let Some([y]) = ptrs_b.get(&q).map(|v| v.as_slice()) {
            cands.entry(*y).or_default().push(format!("conteudo:{p:#x}"));
        }
    }
    if let Some((y, como)) = pelo_conteudo(a, b, x) {
        cands.entry(y).or_default().push(format!("conteudo:{como}"));
    }
    // Lugares que referenciam o dado (ou um campo dele) em funções casadas: a janela em volta do ADRP, única na imagem da função.
    let mut campo: BTreeMap<u64, Vec<(i64, String)>> = BTreeMap::new();
    for (fa, l_refs) in a.refs.iter().enumerate() {
        let Some(fb) = c.ab.get(&(fa as u32)).map(|y| *y as usize) else {
            continue;
        };
        for (rel, r) in l_refs {
            let delta = *r as i64 - x as i64;
            if !(-8..DESLOC_MAX).contains(&delta) {
                continue;
            }
            let p = a.pos(a.ini[fa]) + *rel as usize;
            let (sb, eb) = b.faixa(fb);
            let (sa, ea) = a.faixa(fa);
            let mut achado = None;
            for t in [6usize, 8, 12, 16, 24] {
                let ini = p.saturating_sub(2).max(sa);
                let fim = (ini + t).min(ea);
                if fim - ini < 4 || eb - sb < fim - ini {
                    break;
                }
                let w = &a.m[ini..fim];
                let v: Vec<usize> = (sb..=eb - (fim - ini)).filter(|&q| b.m[q..q + (fim - ini)] == *w).collect();
                if v.len() == 1 {
                    achado = Some(v[0] + (p - ini));
                    break;
                }
                if v.is_empty() {
                    break;
                }
            }
            let Some(q) = achado else { continue };
            let rq = (q - b.pos(b.ini[fb])) as u32;
            if let Some((_, rb)) = b.refs[fb].iter().find(|(o, _)| *o == rq) {
                let y = (*rb as i64 - delta) as u64;
                if delta == 0 {
                    let marca = format!("ref:{:#x}", a.ini[fa]);
                    let e = cands.entry(y).or_default();
                    if !e.contains(&marca) {
                        e.push(marca);
                    }
                } else {
                    campo
                        .entry(y)
                        .or_default()
                        .push((delta, format!("ref:{:#x}(campo {delta:+#x})", a.ini[fa])));
                }
            }
        }
    }
    // Referência a campo só conta sem referência exata, e só a(s) do menor deslocamento: as de longe costumam ser outro objeto.
    let tem_exata = cands.values().flatten().any(|p| p.starts_with("ref:"));
    if !tem_exata {
        if let Some(menor) = campo.values().flatten().map(|(d, _)| d.abs()).min() {
            for (y, l) in &campo {
                for (d, m) in l {
                    if d.abs() == menor {
                        cands.entry(*y).or_default().push(m.clone());
                    }
                }
            }
        }
    }
    if let Some(r) = assin {
        cands.entry(r).or_default().push("assinatura".into());
    }
    if cands.is_empty() {
        l.nota = format!(
            "nenhuma prova ({} refs exatas em funções da origem, {} delas casadas)",
            a.refs.iter().flatten().filter(|(_, r)| *r == x).count(),
            a.refs
                .iter()
                .enumerate()
                .filter(|(f, v)| c.ab.contains_key(&(*f as u32)) && v.iter().any(|(_, r)| *r == x))
                .count()
        );
        return l;
    }
    // Conteúdo que confere: o candidato guarda o ponteiro equivalente ao da origem.
    if let Some((p, q)) = conteudo {
        for (y, pr) in cands.iter_mut() {
            if b.ponteiro_em(*y) == Some(q) && !pr.iter().any(|s| s.starts_with("conteudo:")) {
                pr.push(format!("conteudo-confere:{p:#x}"));
            }
        }
    }
    let peso = |p: &[String]| p.iter().map(|s| if s.contains("campo") { 1 } else { 2 }).sum::<usize>();
    let mut v: Vec<(u64, Vec<String>)> = cands.into_iter().collect();
    v.sort_by_key(|(_, p)| std::cmp::Reverse(peso(p)));
    let (y, mut provas) = v[0].clone();
    // Só discorda quem tem prova que não é de campo.
    let mut contra = v[1..].iter().filter(|(_, p)| p.iter().any(|s| !s.contains("campo"))).count();
    let mut independentes = provas.iter().filter(|s| !s.contains("campo")).count() + provas.iter().any(|s| s.contains("campo")) as usize;
    // Tabela de ponteiros (vtable, GOT): as 16 primeiras entradas que apontam pra função ou dado casado têm de apontar pra imagem no destino.
    let (mut k, mut n) = (0, 0);
    for e in 0..16u64 {
        let Some(p) = a.ponteiro_em(x + 8 * e) else { break };
        let Some(q) = imagem_do_ponteiro(a, b, c, p) else { continue };
        n += 1;
        k += (b.ponteiro_em(y + 8 * e) == Some(q)) as usize;
    }
    if n > 0 {
        provas.push(format!("entradas:{k}/{n}"));
        if n >= 2 && k * 10 >= n * 8 {
            independentes += 1;
        } else if n >= 2 && k * 2 < n {
            contra += 1;
        }
    }
    l.destino = Some(y);
    l.metodo = if provas.iter().any(|p| p.starts_with("simbolo")) {
        "simbolo".into()
    } else if provas.iter().any(|p| p == "assinatura") {
        "assinatura".into()
    } else {
        "dado".into()
    };
    l.confianca = confianca(simbolo_exato == Some(y), independentes, contra);
    let resumo = |p: &[String]| {
        if p.len() > 4 {
            format!("{},…(+{})", p[..3].join(","), p.len() - 3)
        } else {
            p.join(",")
        }
    };
    let outros: Vec<String> = v[1..].iter().map(|(z, p)| format!("{z:#x}<-{}", resumo(p))).collect();
    l.nota = format!(
        "provas={}{}",
        resumo(&provas),
        if outros.is_empty() {
            String::new()
        } else {
            format!("; outros: {}", outros.join(" "))
        }
    );
    l
}

/// RE à mão (`--xrefs <binário> <endereço>...`): a função que contém cada endereço e quem o cita: `BL`/`B`, `ADRP` + uso até 16 instruções
/// depois, `ADR`, `LDR` literal e ponteiros gravados nas seções de dado.
fn xrefs(bin: &Bin, alvos: &[u64]) {
    let n = bin.img.texto.len() / 4;
    let slots: HashMap<u64, Vec<u64>> = indice_de_ponteiros(bin);
    for x in alvos {
        let f = bin.funcao_de(*x);
        println!(
            "{x:#x}: função {} {}",
            f.map(|f| format!("{:#x}+{:#x} ({} palavras)", bin.ini[f], x - bin.ini[f], bin.faixa(f).1 - bin.faixa(f).0)).unwrap_or_else(|| "-".into()),
            f.and_then(|f| bin.nome_de.get(&bin.ini[f])).map(String::as_str).unwrap_or("")
        );
        let mut linhas = Vec::new();
        for i in 0..n {
            let w = en::palavra(bin.img.texto, i);
            let pc = bin.vm(i);
            let quem = |pc: u64| bin.funcao_de(pc).map(|g| format!("{pc:#x} (fn {:#x})", bin.ini[g])).unwrap_or_default();
            if w & 0x7C00_0000 == 0x1400_0000 {
                let imm = ((w & 0x03FF_FFFF) as i64) << 38 >> 36;
                if (pc as i64 + imm) as u64 == *x {
                    linhas.push(format!("  {} {}", if w & 0x8000_0000 != 0 { "bl" } else { "b " }, quem(pc)));
                }
            } else if w & 0x9F00_0000 == 0x1000_0000 {
                let imm = (((w >> 5) & 0x7FFFF) << 2 | (w >> 29) & 3) as i64;
                if (pc as i64 + ((imm << 43) >> 43)) as u64 == *x {
                    linhas.push(format!("  adr {}", quem(pc)));
                }
            } else if w & 0x3B00_0000 == 0x1800_0000 {
                let imm = ((((w >> 5) & 0x7FFFF) as i64) << 45) >> 43;
                let alvo = (pc as i64 + imm) as u64;
                if alvo == *x {
                    linhas.push(format!("  ldr-literal {}", quem(pc)));
                }
            } else if w & 0x9F00_0000 == 0x9000_0000 && en::pagina_do_adrp(w, pc) == x & !0xFFF {
                let rd = w & 0x1F;
                for d in 1..=16usize {
                    if i + d >= n {
                        break;
                    }
                    if let Some(off) = en::offset_de_uso(en::palavra(bin.img.texto, i + d), rd) {
                        if en::pagina_do_adrp(w, pc) + off == *x {
                            linhas.push(format!("  adrp+{d} {}", quem(pc)));
                        }
                        break;
                    }
                }
            }
            if linhas.len() > 40 {
                break;
            }
        }
        for s in slots.get(x).map(|v| v.as_slice()).unwrap_or(&[]).iter().take(20) {
            linhas.push(format!("  ponteiro em {s:#x} ({})", bin.secao_de(*s).map(|s| format!("{},{}", s.segmento, s.nome)).unwrap_or_default()));
        }
        for l in linhas {
            println!("{l}");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `--bytes <origem> <destino> <mapa.tsv>`: os 8 primeiros bytes de cada par de código do mapa, iguais ou não (as guardas de prólogo do
    // BWMS comparam esses bytes antes de pôr o gancho).
    if args.get(1).map(String::as_str) == Some("--bytes") && args.len() == 5 {
        let (ba, bb) = (std::fs::read(&args[2]).expect("origem"), std::fs::read(&args[3]).expect("destino"));
        let (ia, ib) = (Imagem::do_arquivo(&ba).expect("origem"), Imagem::do_arquivo(&bb).expect("destino"));
        let ler = |img: &Imagem, a: u64| -> Option<u64> {
            let o = a.checked_sub(img.texto_vm)? as usize;
            Some(u64::from_le_bytes(img.texto.get(o..o + 8)?.try_into().ok()?))
        };
        for l in std::fs::read_to_string(&args[4]).expect("mapa").lines() {
            let c: Vec<&str> = l.split('\t').collect();
            let (Some(x), Some(y)) = (c.first().and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok()), c.get(1).and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())) else { continue };
            if let (Some(p), Some(q)) = (ler(&ia, x), ler(&ib, y)) {
                println!("{x:#x}\t{y:#x}\t{}\t{p:#018x}\t{q:#018x}", if p == q { "igual" } else if p as u32 == q as u32 { "1a-igual" } else { "mudou" });
            }
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("--xrefs") && args.len() >= 4 {
        let arq = std::fs::read(&args[2]).expect("binário");
        let bin = Bin::novo(&arq);
        let alvos: Vec<u64> = args[3..].iter().filter_map(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok()).collect();
        xrefs(&bin, &alvos);
        return;
    }
    if args.len() < 5 {
        eprintln!("uso: portar_enderecos <binário origem> <binário destino> <pasta src> <saída .tsv> [manuais .tsv]");
        std::process::exit(2);
    }
    let (ba, bb) = (
        std::fs::read(&args[1]).expect("binário origem"),
        std::fs::read(&args[2]).expect("binário destino"),
    );
    let mut alvos = Vec::new();
    let mut pilha = vec![std::path::PathBuf::from(&args[3])];
    while let Some(d) = pilha.pop() {
        for e in std::fs::read_dir(&d).expect("pasta src").flatten() {
            let p = e.path();
            if p.is_dir() {
                pilha.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                alvos.extend(en::enderecos_no_fonte(&std::fs::read_to_string(&p).unwrap_or_default()));
            }
        }
    }
    alvos.sort_unstable();
    alvos.dedup();
    let t0 = std::time::Instant::now();
    let a = Bin::novo(&ba);
    let b = Bin::novo(&bb);
    eprintln!(
        "origem: {} funções, {} nomes; destino: {} funções, {} nomes ({} ms)",
        a.ini.len(),
        a.nome_de.len(),
        b.ini.len(),
        b.nome_de.len(),
        t0.elapsed().as_millis()
    );
    let (tabela, _) = en::gerar(&a.img, &alvos);
    let assin: HashMap<u64, u64> = en::resolver(&b.img, &tabela).into_iter().collect();
    eprintln!("assinatura: {} de {} resolvem no destino", assin.len(), alvos.len());
    let mut c = Casamento::default();
    let mut retidos = Vec::new();
    sementes(&a, &b, &mut c, &mut retidos);
    let (pa, pb) = (indice_de_ponteiros(&a), indice_de_ponteiros(&b));
    for fase in 0..2 {
        RELAXADO.store(fase == 1, std::sync::atomic::Ordering::Relaxed);
        for k in 0..400 {
            let (nf, nd) = rodada(&a, &b, &mut c);
            let nv = if nf == 0 && nd == 0 {
                rodada_vizinhos(&a, &b, &mut c, &pa, &pb)
            } else {
                0
            };
            eprintln!(
                "rodada {k}: +{nf} funções, +{nd} dados, +{nv} por vizinhos em tabela (total {} funções, {} dados)",
                c.ab.len(),
                c.dado_ab.len()
            );
            if nf == 0 && nd == 0 && nv == 0 {
                break;
            }
        }
    }
    if !retidos.is_empty() {
        let certos = retidos.iter().filter(|(x, y)| c.ab.get(x) == Some(y)).count();
        let errados: Vec<&(u32, u32)> = retidos.iter().filter(|(x, y)| c.ab.get(x).is_some_and(|z| z != y)).collect();
        println!(
            "VALIDAÇÃO: {} nomes retidos; casados certo {certos}, errado {}, sem par {}",
            retidos.len(),
            errados.len(),
            retidos.len() - certos - errados.len()
        );
        for (x, y) in errados.iter().take(40) {
            println!(
                "  errado {:#x} {}: esperado {:#x}, casou {:#x}",
                a.ini[*x as usize],
                a.nome_de.get(&a.ini[*x as usize]).map(String::as_str).unwrap_or(""),
                b.ini[*y as usize],
                b.ini[c.ab[x] as usize]
            );
        }
        // Conferência independente: janela das 16 primeiras palavras mascaradas única nos dois binários, contra o par da propagação.
        let janela = |bin: &Bin, f: usize| -> Option<Vec<u32>> {
            let (i, e) = bin.faixa(f);
            (e - i >= 16).then(|| bin.m[i..i + 16].to_vec())
        };
        let mut conta_a: HashMap<Vec<u32>, (usize, usize)> = HashMap::new();
        for f in 0..a.ini.len() {
            if let Some(w) = janela(&a, f) {
                let e = conta_a.entry(w).or_insert((0, f));
                e.0 += 1;
            }
        }
        let mut conta_b: HashMap<Vec<u32>, (usize, usize)> = HashMap::new();
        for f in 0..b.ini.len() {
            if let Some(w) = janela(&b, f) {
                if conta_a.get(&w).is_some_and(|(n, _)| *n == 1) {
                    let e = conta_b.entry(w).or_insert((0, f));
                    e.0 += 1;
                }
            }
        }
        let (mut sim, mut nao, mut sem) = (0, 0, 0);
        for (w, (n, fb)) in &conta_b {
            if *n != 1 {
                continue;
            }
            let fa = conta_a[w].1 as u32;
            if c.provas.get(&fa).is_some_and(|p| p.iter().any(|s| s == "corpo")) {
                continue;
            }
            match c.ab.get(&fa) {
                Some(y) if *y as usize == *fb => sim += 1,
                Some(y) => {
                    nao += 1;
                    if nao <= 25 {
                        println!(
                            "  discorda {:#x}: janela {:#x}, propagação {:#x} ({:?})",
                            a.ini[fa as usize],
                            b.ini[*fb],
                            b.ini[*y as usize],
                            c.provas.get(&fa)
                        );
                    }
                }
                None => sem += 1,
            }
        }
        println!("VALIDAÇÃO por janela de 16 palavras única nos dois: concorda {sim}, discorda {nao}, sem par na propagação {sem}");
        return;
    }
    // O casamento inteiro (função origem → destino, provas), pra RE à mão: `BWMS_PORTAR_CASAMENTO=<arquivo>`.
    if let Some(p) = std::env::var_os("BWMS_PORTAR_CASAMENTO") {
        let mut v: Vec<(u64, u64, String)> = c.ab.iter().map(|(x, y)| (a.ini[*x as usize], b.ini[*y as usize], c.provas.get(x).map(|p| p.iter().take(3).cloned().collect::<Vec<_>>().join(",")).unwrap_or_default())).collect();
        v.extend(c.dado_ab.iter().map(|(x, y)| (*x, *y, "dado".to_string())));
        v.sort();
        let t: String = v.iter().map(|(x, y, p)| format!("{x:#x}\t{y:#x}\t{p}\n")).collect();
        std::fs::write(p, t).expect("casamento");
    }
    let manuais: HashMap<u64, (Option<u64>, String, String, String)> = args
        .get(5)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| {
            t.lines()
                .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
                .filter_map(|l| {
                    let c: Vec<&str> = l.split('\t').collect();
                    let x = u64::from_str_radix(c.first()?.trim_start_matches("0x"), 16).ok()?;
                    let y = u64::from_str_radix(c.get(1)?.trim_start_matches("0x"), 16).ok();
                    Some((x, (y, c.get(2).unwrap_or(&"manual").to_string(), c.get(3).unwrap_or(&"media").to_string(), c.get(4).unwrap_or(&"").to_string())))
                })
                .collect()
        })
        .unwrap_or_default();
    let ptrs_b = pb;
    let mut strings_b: HashMap<&[u8], Vec<u32>> = HashMap::new();
    for (f, l) in b.refs.iter().enumerate() {
        for (_, r) in l {
            if let Some(s) = b.string_em(*r) {
                let e = strings_b.entry(s).or_default();
                if e.last() != Some(&(f as u32)) {
                    e.push(f as u32);
                }
            }
        }
    }
    let ctx = Ctx { strings_b, pa, gramas_b: (0..b.ini.len()).map(|f| gramas(&b, f)).collect() };
    let mut linhas = Vec::new();
    for x in &alvos {
        let mut l = if *x == LINK_BASE {
            Linha {
                origem: *x,
                destino: Some(*x),
                metodo: "base".into(),
                confianca: "alta",
                nota: "base de link da imagem".into(),
            }
        } else if a.no_texto(*x) || a.stub.contains_key(x) {
            portar_codigo(&a, &b, &c, *x, assin.get(x).copied(), &ctx)
        } else if a.em_dado(*x) {
            portar_dado(&a, &b, &c, *x, assin.get(x).copied(), &ptrs_b)
        } else {
            Linha {
                origem: *x,
                destino: None,
                metodo: String::new(),
                confianca: "",
                nota: "fora das seções da origem".into(),
            }
        };
        if let Some((y, metodo, conf, nota)) = manuais.get(x) {
            let auto = l.destino.map(|d| format!("{d:#x}")).unwrap_or_else(|| "-".into());
            let concorda = l.destino.is_some() && l.destino == *y;
            l.nota = format!("{nota}; automático={auto} ({}){}", l.nota, if concorda { " concorda" } else { "" });
            l.destino = *y;
            l.metodo = if metodo == "nao-endereco" { "nao-endereco".into() } else { "manual".into() };
            l.confianca = if concorda && conf == "media" {
                "alta"
            } else {
                Box::leak(conf.clone().into_boxed_str())
            };
        }
        linhas.push(l);
    }
    let mut saida = String::from("endereco_231\tendereco_25\tmetodo\tconfianca\tnota\n");
    let mut por_metodo: BTreeMap<String, usize> = BTreeMap::new();
    let mut por_conf: BTreeMap<&str, usize> = BTreeMap::new();
    let mut destinos: HashMap<u64, Vec<u64>> = HashMap::new();
    for l in &linhas {
        saida.push_str(&format!(
            "{:#x}\t{}\t{}\t{}\t{}\n",
            l.origem,
            l.destino.map(|d| format!("{d:#x}")).unwrap_or_default(),
            if l.destino.is_some() || l.metodo == "nao-endereco" { l.metodo.as_str() } else { "-" },
            if l.destino.is_some() { l.confianca } else { "-" },
            l.nota
        ));
        if let Some(d) = l.destino {
            *por_metodo.entry(l.metodo.split('+').next().unwrap_or("").to_string()).or_insert(0) += 1;
            *por_conf.entry(l.confianca).or_insert(0) += 1;
            destinos.entry(d).or_default().push(l.origem);
        }
    }
    std::fs::write(&args[4], saida).expect("saída");
    let feitos = linhas.iter().filter(|l| l.destino.is_some()).count();
    println!(
        "{feitos} de {} endereços portados; por método {por_metodo:?}; por confiança {por_conf:?}",
        alvos.len()
    );
    for (d, o) in destinos.iter().filter(|(_, o)| o.len() > 1) {
        println!("destino repetido {d:#x} <- {o:x?}");
    }
    let nao_end: Vec<String> = linhas.iter().filter(|l| l.metodo == "nao-endereco").map(|l| format!("{:#x}", l.origem)).collect();
    println!("não são endereço do 2.3.1 ({}): {}", nao_end.len(), nao_end.join(" "));
    for l in linhas.iter().filter(|l| l.destino.is_none() && l.metodo != "nao-endereco") {
        println!("sem par {:#x}: {}", l.origem, l.nota);
    }
}
