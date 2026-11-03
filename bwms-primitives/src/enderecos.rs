//! Endereços do jogo achados em qualquer build pelas instruções em volta (RED4ext `#447`, sobreviver a patch do jogo).
//!
//! Os endereços que o BWMS usa são do build Steam 2.5.0. Num binário com outro layout (o GOG 2.3.1, ou um patch) cada um é
//! achado por uma assinatura: as palavras ARM64 a partir dele com as partes que dependem do layout zeradas (página do `ADRP`, alvo de
//! desvio, offset dentro da página de quem usa o `ADRP`). Endereço de dado (singleton, vtable, tabela) é achado por um lugar do código
//! que o referencia com `ADRP` + `ADD`/`LDR`: acha-se o lugar e decodifica-se o endereço lá.
//!
//! O mesmo código gera a tabela (fora do jogo, lendo o binário Steam do disco) e resolve (no jogo, lendo o `__text` da memória).

use crate::macho::{comandos_de_carga, u32le, LC_DYSYMTAB, LC_SEGMENT_64, LC_SYMTAB, MH_MAGIC_64};

pub const LINK_BASE: u64 = 0x1_0000_0000;
/// Tamanho mínimo e máximo de uma assinatura, em instruções.
pub const MIN_PALAVRAS: usize = 8;
pub const MAX_PALAVRAS: usize = 64;
/// Quantas instruções antes do alvo uma janela pode começar, quando a que começa nele não é única.
/// Acima de 32 a janela nem cobre o alvo (funções idênticas em sequência): o alvo é conferido pelas próprias palavras (`confere`).
const RECUOS: [usize; 8] = [4, 8, 16, 32, 64, 128, 256, 512];
/// Quanto um lugar que referencia um dado pode apontar além dele (campo dentro do objeto).
const DESLOC_MAX: u64 = 0x1000;

fn e_adrp(w: u32) -> bool {
    w & 0x9F00_0000 == 0x9000_0000
}

/// A instrução com os bits que mudam com o layout zerados. `antes` são as até 4 palavras anteriores: um `ADD`/`LDR`/`STR` cujo registro
/// base veio de um `ADRP` ali perto tem o offset da página zerado também.
pub fn mascara(w: u32, antes: &[u32]) -> u32 {
    if w & 0x1F00_0000 == 0x1000_0000 {
        return w & 0x9F00_001F; // ADR/ADRP
    }
    if w & 0x7C00_0000 == 0x1400_0000 {
        return w & 0xFC00_0000; // B/BL
    }
    if w & 0xFF00_0010 == 0x5400_0000 || w & 0x7E00_0000 == 0x3400_0000 || w & 0x3B00_0000 == 0x1800_0000 {
        return w & 0xFF00_001F; // B.cond, CBZ/CBNZ, LDR literal
    }
    if w & 0x7E00_0000 == 0x3600_0000 {
        return w & 0xFFF8_001F; // TBZ/TBNZ
    }
    let rn = (w >> 5) & 0x1F;
    let da_pagina = antes.iter().any(|a| e_adrp(*a) && a & 0x1F == rn);
    if da_pagina && (w & 0x7F80_0000 == 0x1100_0000 || w & 0x3B00_0000 == 0x3900_0000) {
        return w & 0xFFC0_03FF; // ADD imediato / LDR-STR com offset sem sinal
    }
    w
}

/// Palavra `i` do texto.
pub fn palavra(texto: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([texto[i * 4], texto[i * 4 + 1], texto[i * 4 + 2], texto[i * 4 + 3]])
}

/// Palavra `i` mascarada, com o contexto das 4 anteriores.
pub fn mascarada(texto: &[u8], i: usize) -> u32 {
    let ini = i.saturating_sub(4);
    let mut antes = [0u32; 4];
    for (k, j) in (ini..i).enumerate() {
        antes[k] = palavra(texto, j);
    }
    mascara(palavra(texto, i), &antes[..i - ini])
}

/// Endereço que um `ADRP` na posição `i` (texto começando em `texto_vm`) põe no registro: a página.
pub fn pagina_do_adrp(w: u32, pc: u64) -> u64 {
    let imm = (((w >> 5) & 0x7FFFF) << 2 | (w >> 29) & 3) as i64;
    let imm = (imm << 43) >> 43; // 21 bits com sinal
    ((pc & !0xFFF) as i64 + (imm << 12)) as u64
}

/// Offset que um `ADD` imediato ou `LDR`/`STR` com offset sem sinal soma ao registro base, se `w` for um deles com base `rn`.
pub fn offset_de_uso(w: u32, rn: u32) -> Option<u64> {
    if (w >> 5) & 0x1F != rn {
        return None;
    }
    if w & 0x7F80_0000 == 0x1100_0000 {
        let imm = ((w >> 10) & 0xFFF) as u64;
        return Some(if w & (1 << 22) != 0 { imm << 12 } else { imm });
    }
    if w & 0x3B00_0000 == 0x3900_0000 {
        let imm = ((w >> 10) & 0xFFF) as u64;
        let v = w & (1 << 26) != 0;
        let tamanho = w >> 30;
        let escala = if v && tamanho == 0 && w & (1 << 23) != 0 { 4 } else { tamanho };
        return Some(imm << escala);
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tipo {
    /// A janela começa `recuo` instruções antes do endereço.
    Codigo { recuo: u16 },
    /// A janela começa num `ADRP`; o uso (`ADD`/`LDR`) está `uso` instruções depois; o endereço é o do uso mais `desloc`.
    Dado { uso: u8, desloc: i32 },
    /// Função com símbolo de nome único na tabela de símbolos: achada pelo nome (sem `palavras`).
    Simbolo,
    /// Stub de import (`__stubs`): achado pelo nome do símbolo indireto dele.
    Stub,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assinatura {
    pub steam: u64,
    pub tipo: Tipo,
    pub palavras: Vec<u32>,
    /// O nome, no `Tipo::Simbolo` e no `Tipo::Stub`.
    pub simbolo: String,
    /// Palavras mascaradas no próprio alvo quando a janela começa antes dele: conferidas no lugar achado.
    pub confere: Vec<u32>,
}

/// Seção de um Mach-O de 64 bits.
#[derive(Clone, Debug)]
pub struct Secao {
    pub segmento: String,
    pub nome: String,
    pub vm: u64,
    pub tamanho: u64,
    pub arquivo: u64,
    pub flags: u32,
    pub reservado1: u32,
    pub reservado2: u32,
}

/// `SECTION_TYPE` e `S_SYMBOL_STUBS` (`<mach-o/loader.h>`): o tipo da seção mora no byte baixo das flags.
const MASCARA_TIPO_DE_SECAO: u32 = 0xFF;
const SECAO_DE_STUBS: u32 = 0x8;

impl Secao {
    /// Seção de stubs de import (`S_SYMBOL_STUBS`).
    fn de_stubs(&self) -> bool {
        self.flags & MASCARA_TIPO_DE_SECAO == SECAO_DE_STUBS
    }
}

/// `N_STAB` e `N_TYPE`/`N_SECT` (`<mach-o/nlist.h>`) no `n_type` do nlist.
const NLIST_DEPURACAO: u8 = 0xE0;
const NLIST_MASCARA_TIPO: u8 = 0x0E;
const NLIST_NA_SECAO: u8 = 0x0E;

/// O `n_type` é de símbolo definido numa seção (`N_SECT`) e não de depuração (`N_STAB`).
fn simbolo_definido(tipo: u8) -> bool {
    tipo & NLIST_DEPURACAO == 0 && tipo & NLIST_MASCARA_TIPO == NLIST_NA_SECAO
}

fn u64le(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}
fn nome16(b: &[u8], o: usize) -> Option<String> {
    let s = b.get(o..o + 16)?;
    Some(String::from_utf8_lossy(&s[..s.iter().position(|c| *c == 0).unwrap_or(16)]).into_owned())
}

/// A fatia arm64 de um binário universal (ou o próprio binário, se já for um Mach-O de 64 bits).
pub fn fatia_arm64(b: &[u8]) -> Option<&[u8]> {
    match u32::from_be_bytes(b.get(0..4)?.try_into().ok()?) {
        0xCAFE_BABE => {
            let n = u32::from_be_bytes(b.get(4..8)?.try_into().ok()?) as usize;
            (0..n).find_map(|i| {
                let a = 8 + i * 20;
                let cpu = u32::from_be_bytes(b.get(a..a + 4)?.try_into().ok()?);
                let ini = u32::from_be_bytes(b.get(a + 8..a + 12)?.try_into().ok()?) as usize;
                let tam = u32::from_be_bytes(b.get(a + 12..a + 16)?.try_into().ok()?) as usize;
                (cpu == 0x0100_000C).then(|| b.get(ini..ini + tam)).flatten()
            })
        }
        _ if u32le(b, 0)? == MH_MAGIC_64 => Some(b),
        _ => None,
    }
}

/// As seções de um Mach-O de 64 bits (cabeçalho e comandos de carga; serve pro arquivo e pra imagem na memória).
pub fn secoes(macho: &[u8]) -> Option<Vec<Secao>> {
    let mut v = Vec::new();
    for c in comandos_de_carga(macho)?.filter(|c| c.cmd == LC_SEGMENT_64) {
        let nsects = u32le(macho, c.offset + 0x40)? as usize;
        for k in 0..nsects {
            let s = c.offset + 0x48 + k * 0x50;
            v.push(Secao {
                nome: nome16(macho, s)?,
                segmento: nome16(macho, s + 0x10)?,
                vm: u64le(macho, s + 0x20)?,
                tamanho: u64le(macho, s + 0x28)?,
                arquivo: u32le(macho, s + 0x30)? as u64,
                flags: u32le(macho, s + 0x40)?,
                reservado1: u32le(macho, s + 0x44)?,
                reservado2: u32le(macho, s + 0x48)?,
            });
        }
    }
    Some(v)
}

/// Segmento de um Mach-O de 64 bits: nome, endereço, tamanho na memória, offset e tamanho no arquivo.
#[derive(Clone, Debug)]
pub struct Segmento {
    pub nome: String,
    pub vm: u64,
    pub tamanho: u64,
    pub arquivo: u64,
    pub tamanho_arquivo: u64,
}

pub fn segmentos(macho: &[u8]) -> Option<Vec<Segmento>> {
    let mut v = Vec::new();
    for c in comandos_de_carga(macho)?.filter(|c| c.cmd == LC_SEGMENT_64) {
        let o = c.offset;
        v.push(Segmento {
            nome: nome16(macho, o + 8)?,
            vm: u64le(macho, o + 0x18)?,
            tamanho: u64le(macho, o + 0x20)?,
            arquivo: u64le(macho, o + 0x28)?,
            tamanho_arquivo: u64le(macho, o + 0x30)?,
        });
    }
    Some(v)
}

/// `LC_SYMTAB`: `(offset dos nlist no arquivo, quantos, offset das strings, tamanho das strings)`.
pub fn tabela_de_simbolos(macho: &[u8]) -> Option<(u64, usize, u64, usize)> {
    let o = comandos_de_carga(macho)?.find(|c| c.cmd == LC_SYMTAB)?.offset;
    Some((u32le(macho, o + 8)? as u64, u32le(macho, o + 12)? as usize, u32le(macho, o + 16)? as u64, u32le(macho, o + 20)? as usize))
}

/// `LC_DYSYMTAB`: `(offset da tabela de símbolos indiretos no arquivo, quantos)`.
pub fn tabela_indireta(macho: &[u8]) -> Option<(u64, usize)> {
    let o = comandos_de_carga(macho)?.find(|c| c.cmd == LC_DYSYMTAB)?.offset;
    Some((u32le(macho, o + 0x38)? as u64, u32le(macho, o + 0x3C)? as usize))
}

/// Os stubs de import (`S_SYMBOL_STUBS`): `(endereço do stub, nome do símbolo)`, pela tabela indireta e pelos nomes de todos os nlist.
pub fn stubs_de(secoes: &[Secao], indiretos: &[u8], nlist: &[u8], quantos: usize, strings: &[u8]) -> Vec<(u64, String)> {
    let nome = |idx: u32| -> Option<String> {
        if idx as usize >= quantos {
            return None;
        }
        let ini = u32le(nlist, idx as usize * 16)? as usize;
        let resto = strings.get(ini..)?;
        Some(String::from_utf8_lossy(&resto[..resto.iter().position(|c| *c == 0).unwrap_or(resto.len())]).into_owned())
    };
    let mut v = Vec::new();
    for s in secoes.iter().filter(|s| s.de_stubs() && s.reservado2 > 0) {
        for k in 0..(s.tamanho / s.reservado2 as u64) as usize {
            if let Some(n) = u32le(indiretos, (s.reservado1 as usize + k) * 4).and_then(nome) {
                v.push((s.vm + k as u64 * s.reservado2 as u64, n));
            }
        }
    }
    v
}

/// Os símbolos definidos numa seção (`N_SECT`, sem os de depuração): `(endereço, nome)`.
pub fn simbolos_de(nlist: &[u8], quantos: usize, strings: &[u8]) -> Vec<(u64, String)> {
    let mut v = Vec::with_capacity(quantos);
    for k in 0..quantos {
        let e = k * 16;
        let (Some(strx), Some(tipo), Some(valor)) = (u32le(nlist, e), nlist.get(e + 4), u64le(nlist, e + 8)) else { break };
        if !simbolo_definido(*tipo) {
            continue;
        }
        let ini = strx as usize;
        let Some(resto) = strings.get(ini..) else { continue };
        let fim = resto.iter().position(|c| *c == 0).unwrap_or(resto.len());
        v.push((valor, String::from_utf8_lossy(&resto[..fim]).into_owned()));
    }
    v
}

/// O código de um binário: o `__text`, onde ele começa, as seções, os símbolos e os stubs de import.
pub struct Imagem<'a> {
    pub texto: &'a [u8],
    pub texto_vm: u64,
    pub secoes: Vec<Secao>,
    pub simbolos: Vec<(u64, String)>,
    pub stubs: Vec<(u64, String)>,
}

impl<'a> Imagem<'a> {
    /// Do arquivo inteiro (universal ou não).
    pub fn do_arquivo(b: &'a [u8]) -> Option<Self> {
        let m = fatia_arm64(b)?;
        let secoes = secoes(m)?;
        let t = secoes.iter().find(|s| s.segmento == "__TEXT" && s.nome == "__text")?.clone();
        let (so, n, st, sn) = tabela_de_simbolos(m).unwrap_or((0, 0, 0, 0));
        let (nlist, strings) = (m.get(so as usize..so as usize + n * 16).unwrap_or(&[]), m.get(st as usize..st as usize + sn).unwrap_or(&[]));
        let simbolos = simbolos_de(nlist, n, strings);
        let stubs = tabela_indireta(m)
            .and_then(|(io, ni)| m.get(io as usize..io as usize + ni * 4))
            .map(|ind| stubs_de(&secoes, ind, nlist, n, strings))
            .unwrap_or_default();
        Some(Imagem { texto: m.get(t.arquivo as usize..(t.arquivo + t.tamanho) as usize)?, texto_vm: t.vm, secoes, simbolos, stubs })
    }

    fn palavras(&self) -> usize {
        self.texto.len() / 4
    }

    fn no_texto(&self, a: u64) -> Option<usize> {
        (a >= self.texto_vm && a < self.texto_vm + self.texto.len() as u64 && a % 4 == 0).then(|| ((a - self.texto_vm) / 4) as usize)
    }

    fn em_dado(&self, a: u64) -> bool {
        self.secoes.iter().any(|s| !(s.segmento == "__TEXT" && s.nome == "__text") && a >= s.vm && a < s.vm + s.tamanho.max(1))
    }
}

/// Por que um endereço ficou sem assinatura.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemAssinatura {
    ForaDoBinario,
    NaoUnica,
    SemReferencia,
}

/// Maior prefixo comum de `m` a partir de `a` e de `b`, até `max`.
fn prefixo(m: &[u32], a: usize, b: usize, max: usize) -> usize {
    let mut k = 0;
    while k < max && a + k < m.len() && b + k < m.len() && m[a + k] == m[b + k] {
        k += 1;
    }
    k
}

/// Para cada janela (início no fluxo mascarado), o tamanho mínimo que a torna única no texto (`None` se nem `MAX_PALAVRAS` basta).
/// Uma passada: em cada posição cujo primeiro valor bate com o de alguma janela, mede o prefixo comum.
fn tamanhos_unicos(m: &[u32], inicios: &[usize]) -> Vec<Option<usize>> {
    let mut por_primeira: std::collections::HashMap<u32, Vec<usize>> = std::collections::HashMap::new();
    for (k, &i) in inicios.iter().enumerate() {
        if i + MAX_PALAVRAS <= m.len() {
            por_primeira.entry(m[i]).or_default().push(k);
        }
    }
    let mut maior = vec![0usize; inicios.len()];
    for p in 0..m.len() {
        if let Some(ks) = por_primeira.get(&m[p]) {
            for &k in ks {
                let i = inicios[k];
                if p != i {
                    maior[k] = maior[k].max(prefixo(m, p, i, MAX_PALAVRAS + 1));
                }
            }
        }
    }
    inicios
        .iter()
        .zip(maior)
        .map(|(&i, l)| (i + MAX_PALAVRAS <= m.len() && l < MAX_PALAVRAS).then_some((l + 1).max(MIN_PALAVRAS)))
        .collect()
}

/// Lugares do código que referenciam cada endereço de dado pedido: `(posição do ADRP, distância até o uso, endereço referenciado)`.
fn referencias(img: &Imagem, alvos: &[u64]) -> std::collections::HashMap<u64, Vec<(usize, u8, u64)>> {
    let mut refs: std::collections::HashMap<u64, Vec<(usize, u8, u64)>> = std::collections::HashMap::new();
    let mut ordenados: Vec<u64> = alvos.to_vec();
    ordenados.sort_unstable();
    let n = img.palavras();
    for i in 0..n {
        let w = palavra(img.texto, i);
        if !e_adrp(w) {
            continue;
        }
        let pagina = pagina_do_adrp(w, img.texto_vm + i as u64 * 4);
        let rd = w & 0x1F;
        for d in 1..=4usize {
            if i + d >= n {
                break;
            }
            let Some(off) = offset_de_uso(palavra(img.texto, i + d), rd) else { continue };
            let referido = pagina + off;
            // O alvo mais próximo abaixo do referido, dentro de DESLOC_MAX.
            let pos = ordenados.partition_point(|a| *a <= referido);
            if pos > 0 && referido - ordenados[pos - 1] < DESLOC_MAX {
                let l = refs.entry(ordenados[pos - 1]).or_default();
                if l.len() < 32 {
                    l.push((i, d as u8, referido));
                }
            }
            break;
        }
    }
    refs
}

/// Gera as assinaturas dos endereços `alvos` no binário `img` (o Steam). Os que não têm assinatura voltam com o motivo.
pub fn gerar(img: &Imagem, alvos: &[u64]) -> (Vec<Assinatura>, Vec<(u64, SemAssinatura)>) {
    let n = img.palavras();
    let m: Vec<u32> = (0..n).map(|i| mascarada(img.texto, i)).collect();
    let (mut feitas, mut faltam) = (Vec::new(), Vec::new());
    let codigo: Vec<(u64, usize)> = alvos.iter().filter_map(|a| img.no_texto(*a).map(|i| (*a, i))).collect();
    let stubs: std::collections::HashMap<u64, &str> = img.stubs.iter().map(|(a, n)| (*a, n.as_str())).collect();
    let mut nomes_stub: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (_, n) in &img.stubs {
        *nomes_stub.entry(n.as_str()).or_insert(0) += 1;
    }
    for a in alvos {
        if let Some(n) = stubs.get(a).filter(|n| nomes_stub.get(**n) == Some(&1)) {
            feitas.push(Assinatura { steam: *a, tipo: Tipo::Stub, palavras: Vec::new(), simbolo: n.to_string(), confere: Vec::new() });
        }
    }
    let dados: Vec<u64> =
        alvos.iter().copied().filter(|a| img.no_texto(*a).is_none() && img.em_dado(*a) && !stubs.contains_key(a)).collect();
    for a in alvos {
        if img.no_texto(*a).is_none() && !img.em_dado(*a) {
            faltam.push((*a, SemAssinatura::ForaDoBinario));
        }
    }

    // Código: função com símbolo de nome único vai pelo nome; o resto, pela janela que começa no alvo, e as que não ficam únicas tentam
    // começar antes.
    let mut nomes: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (_, n) in &img.simbolos {
        *nomes.entry(n.as_str()).or_insert(0) += 1;
    }
    let por_endereco: std::collections::HashMap<u64, &str> =
        img.simbolos.iter().filter(|(_, n)| nomes.get(n.as_str()) == Some(&1)).map(|(a, n)| (*a, n.as_str())).collect();
    let mut pendentes: Vec<(u64, usize)> = Vec::new();
    for (a, i) in codigo {
        match por_endereco.get(&a) {
            Some(n) => feitas.push(Assinatura { steam: a, tipo: Tipo::Simbolo, palavras: Vec::new(), simbolo: n.to_string(), confere: Vec::new() }),
            None => pendentes.push((a, i)),
        }
    }
    for recuo in std::iter::once(0).chain(RECUOS) {
        let inicios: Vec<usize> = pendentes.iter().map(|(_, i)| i.saturating_sub(recuo)).collect();
        let tamanhos = tamanhos_unicos(&m, &inicios);
        let mut resto = Vec::new();
        for ((a, i), (ini, t)) in pendentes.into_iter().zip(inicios.into_iter().zip(tamanhos)) {
            match t {
                Some(t) if i - ini == recuo => feitas.push(Assinatura {
                    steam: a,
                    tipo: Tipo::Codigo { recuo: recuo as u16 },
                    palavras: m[ini..ini + t].to_vec(),
                    simbolo: String::new(),
                    confere: if recuo == 0 { Vec::new() } else { m[i..(i + MIN_PALAVRAS).min(m.len())].to_vec() },
                }),
                _ => resto.push((a, i)),
            }
        }
        pendentes = resto;
        if pendentes.is_empty() {
            break;
        }
    }
    // Último recurso (funções idênticas em sequência, onde todo recuo fixo cai dentro de outra cópia): a posição única mais perto antes do
    // alvo, até 1024 instruções, com as palavras do alvo conferidas no lugar achado.
    if !pendentes.is_empty() {
        let mut inicios = Vec::new();
        for (_, i) in &pendentes {
            inicios.extend((1..=1024usize).filter(|r| *r <= *i).map(|r| i - r));
        }
        let tamanhos = tamanhos_unicos(&m, &inicios);
        let unicos: std::collections::HashMap<usize, usize> = inicios.iter().zip(tamanhos).filter_map(|(ini, t)| t.map(|t| (*ini, t))).collect();
        let mut resto = Vec::new();
        for (a, i) in pendentes {
            match (1..=1024usize).filter(|r| *r <= i).find_map(|r| unicos.get(&(i - r)).map(|t| (r, *t))) {
                Some((r, t)) => feitas.push(Assinatura {
                    steam: a,
                    tipo: Tipo::Codigo { recuo: r as u16 },
                    palavras: m[i - r..i - r + t].to_vec(),
                    simbolo: String::new(),
                    confere: m[i..(i + MIN_PALAVRAS).min(m.len())].to_vec(),
                }),
                None => resto.push((a, i)),
            }
        }
        pendentes = resto;
    }
    faltam.extend(pendentes.into_iter().map(|(a, _)| (a, SemAssinatura::NaoUnica)));

    // Dado: o primeiro lugar que o referencia com janela única.
    let refs = referencias(img, &dados);
    let mut candidatos: Vec<(u64, usize, u8, u64)> = Vec::new();
    for a in &dados {
        match refs.get(a) {
            Some(l) => candidatos.extend(l.iter().map(|(i, d, r)| (*a, *i, *d, *r))),
            None => faltam.push((*a, SemAssinatura::SemReferencia)),
        }
    }
    // Por alvo, a referência mais perto dele primeiro: um deslocamento grande pode ser outro objeto da mesma página, que num outro layout
    // não fica à mesma distância.
    candidatos.sort_by_key(|(a, i, _, r)| (*a, r.abs_diff(*a), *i));
    let inicios: Vec<usize> = candidatos.iter().map(|c| c.1).collect();
    let tamanhos = tamanhos_unicos(&m, &inicios);
    let mut feitos_dado = std::collections::HashSet::new();
    for ((a, i, d, referido), t) in candidatos.into_iter().zip(tamanhos) {
        if feitos_dado.contains(&a) {
            continue;
        }
        if let Some(t) = t {
            let t = t.max(d as usize + 1);
            feitos_dado.insert(a);
            feitas.push(Assinatura {
                steam: a,
                tipo: Tipo::Dado { uso: d, desloc: (a as i64 - referido as i64) as i32 },
                palavras: m[i..i + t].to_vec(),
                simbolo: String::new(),
                confere: Vec::new(),
            });
        }
    }
    for a in &dados {
        if refs.contains_key(a) && !feitos_dado.contains(a) {
            faltam.push((*a, SemAssinatura::NaoUnica));
        }
    }
    feitas.sort_by_key(|s| s.steam);
    (feitas, faltam)
}

/// Resolve as assinaturas no binário `img` (qualquer build): `(endereço Steam, endereço neste binário)` pra cada uma achada uma vez só.
/// Uma passada pelo texto: cada posição cujas duas primeiras palavras mascaradas batem com as de uma assinatura é conferida inteira.
pub fn resolver(img: &Imagem, tabela: &[Assinatura]) -> Vec<(u64, u64)> {
    let n = img.palavras();
    let mut por_chave: std::collections::HashMap<(u32, u32), Vec<usize>> = std::collections::HashMap::new();
    for (k, s) in tabela.iter().enumerate() {
        if s.palavras.len() >= 2 {
            por_chave.entry((s.palavras[0], s.palavras[1])).or_default().push(k);
        }
    }
    let mut achados: Vec<(usize, u32)> = vec![(usize::MAX, 0); tabela.len()];
    let mut anterior = if n > 0 { mascarada(img.texto, 0) } else { 0 };
    for p in 1..n {
        let atual = mascarada(img.texto, p);
        if let Some(ks) = por_chave.get(&(anterior, atual)) {
            let ini = p - 1;
            for &k in ks {
                let s = &tabela[k];
                if ini + s.palavras.len() <= n && (2..s.palavras.len()).all(|j| mascarada(img.texto, ini + j) == s.palavras[j]) {
                    let (pos, vezes) = &mut achados[k];
                    *pos = ini;
                    *vezes += 1;
                }
            }
        }
        anterior = atual;
    }
    let mut nomes: std::collections::HashMap<&str, Vec<u64>> = std::collections::HashMap::new();
    for (a, n) in &img.simbolos {
        nomes.entry(n.as_str()).or_default().push(*a);
    }
    let mut por_stub: std::collections::HashMap<&str, Vec<u64>> = std::collections::HashMap::new();
    for (a, n) in &img.stubs {
        por_stub.entry(n.as_str()).or_default().push(*a);
    }
    let mut v = Vec::new();
    for (s, (pos, vezes)) in tabela.iter().zip(achados) {
        if s.tipo == Tipo::Stub {
            if let Some([a]) = por_stub.get(s.simbolo.as_str()).map(Vec::as_slice) {
                v.push((s.steam, *a));
            }
            continue;
        }
        if s.tipo == Tipo::Simbolo {
            if let Some([a]) = nomes.get(s.simbolo.as_str()).map(Vec::as_slice) {
                v.push((s.steam, *a));
            }
            continue;
        }
        if vezes != 1 {
            continue;
        }
        let aqui = img.texto_vm + pos as u64 * 4;
        let resolvido = match s.tipo {
            Tipo::Codigo { recuo } => {
                let alvo = pos + recuo as usize;
                let confere = alvo + s.confere.len() <= n && s.confere.iter().enumerate().all(|(j, w)| mascarada(img.texto, alvo + j) == *w);
                confere.then_some(aqui + recuo as u64 * 4)
            }
            Tipo::Simbolo | Tipo::Stub => None,
            Tipo::Dado { uso, desloc } => {
                let w = palavra(img.texto, pos);
                let pagina = pagina_do_adrp(w, aqui);
                offset_de_uso(palavra(img.texto, pos + uso as usize), w & 0x1F).map(|o| (pagina + o) as i64 + desloc as i64).map(|x| x as u64)
            }
        };
        if let Some(r) = resolvido {
            v.push((s.steam, r));
        }
    }
    v
}

const MAGICA: &[u8; 4] = b"BWEN";

/// Formato da tabela: `BWEN`, versão (u32 = 2), quantidade (u32), e por assinatura: endereço Steam (u64), tipo (u8: 0 código, 1 dado,
/// 2 símbolo, 3 stub), recuo (u16), uso (u8) + deslocamento (i32) ou tamanho do nome (u16) + o nome, quantidade de palavras (u8) e as
/// palavras (u32), quantidade de palavras de conferência (u8) e elas. Tudo little-endian.
pub fn serializar(tabela: &[Assinatura]) -> Vec<u8> {
    let mut b = Vec::with_capacity(12 + tabela.len() * 80);
    b.extend_from_slice(MAGICA);
    b.extend_from_slice(&3u32.to_le_bytes());
    b.extend_from_slice(&(tabela.len() as u32).to_le_bytes());
    for s in tabela {
        b.extend_from_slice(&s.steam.to_le_bytes());
        match s.tipo {
            Tipo::Codigo { recuo } => {
                b.push(0);
                b.extend_from_slice(&recuo.to_le_bytes());
            }
            Tipo::Dado { uso, desloc } => {
                b.push(1);
                b.push(uso);
                b.extend_from_slice(&desloc.to_le_bytes());
            }
            Tipo::Simbolo | Tipo::Stub => {
                b.push(if s.tipo == Tipo::Stub { 3 } else { 2 });
                b.extend_from_slice(&(s.simbolo.len() as u16).to_le_bytes());
                b.extend_from_slice(s.simbolo.as_bytes());
            }
        }
        b.push(s.palavras.len() as u8);
        for w in &s.palavras {
            b.extend_from_slice(&w.to_le_bytes());
        }
        b.push(s.confere.len() as u8);
        for w in &s.confere {
            b.extend_from_slice(&w.to_le_bytes());
        }
    }
    b
}

pub fn ler(b: &[u8]) -> Option<Vec<Assinatura>> {
    if b.get(0..4)? != MAGICA || u32le(b, 4)? != 3 {
        return None;
    }
    let n = u32le(b, 8)? as usize;
    let mut o = 12;
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        let steam = u64le(b, o)?;
        o += 8;
        let mut simbolo = String::new();
        let tipo = match *b.get(o)? {
            t @ (2 | 3) => {
                let n = u16::from_le_bytes(b.get(o + 1..o + 3)?.try_into().ok()?) as usize;
                simbolo = String::from_utf8(b.get(o + 3..o + 3 + n)?.to_vec()).ok()?;
                o += 3 + n;
                if t == 3 { Tipo::Stub } else { Tipo::Simbolo }
            }
            0 => {
                o += 3;
                Tipo::Codigo { recuo: u16::from_le_bytes(b.get(o - 2..o)?.try_into().ok()?) }
            }
            1 => {
                let uso = *b.get(o + 1)?;
                o += 6;
                Tipo::Dado { uso, desloc: i32::from_le_bytes(b.get(o - 4..o)?.try_into().ok()?) }
            }
            _ => return None,
        };
        let q = *b.get(o)? as usize;
        o += 1;
        let palavras = (0..q).map(|k| u32le(b, o + k * 4)).collect::<Option<Vec<u32>>>()?;
        o += q * 4;
        let c = *b.get(o)? as usize;
        o += 1;
        let confere = (0..c).map(|k| u32le(b, o + k * 4)).collect::<Option<Vec<u32>>>()?;
        o += c * 4;
        v.push(Assinatura { steam, tipo, palavras, simbolo, confere });
    }
    Some(v)
}

/// Os endereços do binário citados num fonte: literais `0x1_0XXX_XXXX` / `0x10XXXXXXX` (com ou sem `_`) entre a base e 16 MB além do
/// fim típico do binário. Pega também os dos comentários: sobra é inofensiva (a tabela só fica maior).
pub fn enderecos_no_fonte(fonte: &str) -> Vec<u64> {
    let b = fonte.as_bytes();
    let mut v = Vec::new();
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i] == b'0' && b[i + 1] == b'x' && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let mut j = i + 2;
            let mut s = String::new();
            while j < b.len() && (b[j].is_ascii_hexdigit() || b[j] == b'_') {
                if b[j] != b'_' {
                    s.push(b[j] as char);
                }
                j += 1;
            }
            if s.len() == 9 {
                if let Ok(x) = u64::from_str_radix(&s, 16) {
                    if (LINK_BASE..LINK_BASE + 0x1000_0000).contains(&x) {
                        v.push(x);
                    }
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    v.sort_unstable();
    v.dedup();
    v
}

/// Um `mod nome;` de um fonte Rust, com os `#[cfg(...)]` e o `#[path = "..."]` escritos antes dele.
#[derive(Debug, PartialEq)]
pub struct ModuloDeclarado {
    pub nome: String,
    pub caminho: Option<String>,
    pub cfg: Vec<String>,
}

/// Os `mod nome;` de um fonte, cada um com os atributos de uma linha que o precedem (comentário, doc e linha vazia entre eles não
/// separam). `mod nome { … }` mora no próprio arquivo e não entra: o gerador lê o arquivo inteiro de quem o declara.
pub fn modulos_declarados(fonte: &str) -> Vec<ModuloDeclarado> {
    let mut v = Vec::new();
    let (mut cfg, mut caminho) = (Vec::new(), None);
    for linha in fonte.lines() {
        let l = linha.trim();
        if l.is_empty() || l.starts_with("//") {
            continue;
        }
        if let Some(resto) = l.strip_prefix("#[cfg(") {
            let mut fundo = 1usize;
            let fim = resto.char_indices().find_map(|(k, c)| {
                match c {
                    '(' => fundo += 1,
                    ')' => fundo -= 1,
                    _ => {}
                }
                (fundo == 0).then_some(k)
            });
            cfg.push(resto[..fim.unwrap_or(resto.len())].trim().to_string());
            continue;
        }
        if let Some(resto) = l.strip_prefix("#[path") {
            caminho = resto.split('"').nth(1).map(str::to_string);
            continue;
        }
        if l.starts_with("#[") {
            continue;
        }
        let sem_pub = match l.strip_prefix("pub") {
            Some(r) if r.starts_with('(') => r.split_once(')').map_or(r, |(_, d)| d),
            Some(r) => r,
            None => l,
        };
        if let Some(nome) = sem_pub.trim_start().strip_prefix("mod ").and_then(|r| r.trim().strip_suffix(';')) {
            v.push(ModuloDeclarado { nome: nome.trim().to_string(), caminho: caminho.take(), cfg: std::mem::take(&mut cfg) });
        }
        cfg.clear();
        caminho = None;
    }
    v
}

/// Valor de um predicado de `#[cfg(...)]` com `features` ligadas e `test` desligado. `None` = forma que esta avaliação não conhece
/// (`target_os`, `debug_assertions`…): quem chama recusa, em vez de chutar um lado.
pub fn cfg_ativo(predicado: &str, features: &[&str]) -> Option<bool> {
    let p = predicado.trim();
    if p == "test" {
        return Some(false);
    }
    if let Some(f) = p.strip_prefix("feature") {
        let nome = f.trim_start().strip_prefix('=')?.trim().strip_prefix('"')?.strip_suffix('"')?;
        return Some(features.contains(&nome));
    }
    let (op, dentro) = p.split_once('(')?;
    let dentro = dentro.strip_suffix(')')?;
    let mut partes = Vec::new();
    let (mut fundo, mut ini) = (0usize, 0usize);
    for (k, c) in dentro.char_indices() {
        match c {
            '(' => fundo += 1,
            ')' => fundo = fundo.checked_sub(1)?,
            ',' if fundo == 0 => {
                partes.push(&dentro[ini..k]);
                ini = k + 1;
            }
            _ => {}
        }
    }
    partes.push(&dentro[ini..]);
    let valores = partes.iter().filter(|s| !s.trim().is_empty()).map(|s| cfg_ativo(s, features)).collect::<Option<Vec<bool>>>()?;
    match op.trim() {
        "not" if valores.len() == 1 => Some(!valores[0]),
        "any" => Some(valores.iter().any(|&b| b)),
        "all" => Some(valores.iter().all(|&b| b)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mascara_tira_o_que_depende_do_layout() {
        // adrp x8, #page ; add x8, x8, #0x6c5 ; bl alvo ; ldr x9, [x8, #0x10] (base não é de ADRP) ; mov x0, x1
        assert_eq!(mascara(0x9000_0008 | (0x1234 << 5), &[]), 0x9000_0008);
        assert_eq!(mascara(0x9100_0000 | (0x6c5 << 10) | (8 << 5) | 8, &[0x9000_0008]), 0x9100_0000 | (8 << 5) | 8);
        assert_eq!(mascara(0x9400_1234, &[]), 0x9400_0000);
        let ldr = 0xF940_0000 | (2 << 10) | (8 << 5) | 9;
        assert_eq!(mascara(ldr, &[]), ldr);
        assert_eq!(mascara(ldr, &[0x9000_0008]), 0xF940_0000 | (8 << 5) | 9);
        assert_eq!(mascara(0xAA01_03E0, &[]), 0xAA01_03E0);
        assert_eq!(mascara(0x5400_0081, &[]), 0x5400_0001); // b.ne
        assert_eq!(mascara(0xB400_0100, &[]), 0xB400_0000); // cbz x0
    }

    #[test]
    fn adrp_e_uso_decodificados() {
        // adrp x0, 19145 páginas a partir de 0x1021fcc24 -> 0x106cc5000; add x0, x0, #0x5d
        let adrp = 0x9000_0000 | ((19145u32 & 3) << 29) | ((19145u32 >> 2) << 5);
        assert_eq!(pagina_do_adrp(adrp, 0x1_021f_cc24), 0x1_06cc_5000);
        assert_eq!(offset_de_uso(0x9100_0000 | (0x5d << 10), 0), Some(0x5d));
        // ldr x8, [x8, #0xe40] (64 bits: escala 8, imm12 = 0x1c8)
        assert_eq!(offset_de_uso(0xF940_0000 | (0x1c8 << 10) | (8 << 5) | 8, 8), Some(0xe40));
        assert_eq!(offset_de_uso(0xF940_0000 | (0x1c8 << 10) | (8 << 5) | 8, 9), None);
    }

    #[test]
    fn tabela_ida_e_volta() {
        let t = vec![
            Assinatura { steam: 0x1_0217_3120, tipo: Tipo::Codigo { recuo: 4 }, palavras: vec![1, 2, 3], simbolo: String::new(), confere: vec![7, 8] },
            Assinatura { steam: 0x1_080c_92d0, tipo: Tipo::Dado { uso: 2, desloc: -0x1f8 }, palavras: vec![9; 12], simbolo: String::new(), confere: vec![] },
            Assinatura { steam: 0x1_0002_2cb0, tipo: Tipo::Simbolo, palavras: vec![], simbolo: "__ZN3red6memory4FreeEv".into(), confere: vec![] },
            Assinatura { steam: 0x1_04a3_bbfc, tipo: Tipo::Stub, palavras: vec![], simbolo: "__ZN3red6memory6AllocEy".into(), confere: vec![] },
        ];
        assert_eq!(ler(&serializar(&t)), Some(t));
        assert_eq!(ler(b"XXXX"), None);
    }

    /// Corpo de um `LC_SEGMENT_64` (sem `cmd`/`cmdsize`) com as seções `(nome, vm, tamanho, offset no arquivo, flags, reservado1, reservado2)`.
    fn segmento(nome: &str, vm: u64, tamanho: u64, arquivo: u64, secoes: &[(&str, u64, u64, u32, u32, u32, u32)]) -> Vec<u8> {
        let nome16 = |s: &str| {
            let mut b = [0u8; 16];
            b[..s.len()].copy_from_slice(s.as_bytes());
            b
        };
        let mut c = nome16(nome).to_vec();
        for v in [vm, tamanho, arquivo, tamanho] {
            c.extend_from_slice(&v.to_le_bytes());
        }
        for v in [7u32, 5, secoes.len() as u32, 0] {
            c.extend_from_slice(&v.to_le_bytes());
        }
        for &(secao, endereco, bytes, offset, flags, reservado1, reservado2) in secoes {
            c.extend_from_slice(&nome16(secao));
            c.extend_from_slice(&nome16(nome));
            c.extend_from_slice(&endereco.to_le_bytes());
            c.extend_from_slice(&bytes.to_le_bytes());
            for v in [offset, 2, 0, 0, flags, reservado1, reservado2, 0] {
                c.extend_from_slice(&v.to_le_bytes());
            }
        }
        c
    }

    /// Seções e segmentos de cada `LC_SEGMENT_64` na ordem, e o `LC_SYMTAB`/`LC_DYSYMTAB` onde estiverem, pelo andador do `macho`: o
    /// `LC_LOAD_DYLIB` no meio é pulado. O Mach-O de teste termina nos comandos, como o cabeçalho que o runtime lê da memória.
    #[test]
    fn secoes_segmentos_e_tabelas_pelos_comandos_de_carga() {
        let symtab: Vec<u8> = [0x8000u32, 30, 0x8400, 0x200].iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut dysymtab = vec![0u8; 72];
        dysymtab[48..52].copy_from_slice(&0x9000u32.to_le_bytes());
        dysymtab[52..56].copy_from_slice(&12u32.to_le_bytes());
        let texto = [("__text", 0x1_0000_1000, 0x2000, 0x1000, 0x8000_0400, 0, 0), ("__stubs", 0x1_0000_3000, 0x60, 0x3000, 0x8000_0408, 4, 12)];
        let m = crate::macho::macho_de_teste(&[
            (LC_SEGMENT_64, segmento("__TEXT", LINK_BASE, 0x4000, 0, &texto)),
            (crate::macho::LC_LOAD_DYLIB, vec![0; 24]),
            (LC_SYMTAB, symtab),
            (LC_SEGMENT_64, segmento("__LINKEDIT", 0x1_0000_8000, 0x4000, 0x8000, &[])),
            (LC_DYSYMTAB, dysymtab),
        ]);
        let lidas = secoes(&m).expect("seções");
        let lidas: Vec<_> = lidas.iter().map(|s| (s.segmento.as_str(), s.nome.as_str(), s.vm, s.tamanho, s.arquivo, s.flags, s.reservado1, s.reservado2)).collect();
        assert_eq!(
            lidas,
            vec![("__TEXT", "__text", 0x1_0000_1000, 0x2000, 0x1000, 0x8000_0400, 0, 0), ("__TEXT", "__stubs", 0x1_0000_3000, 0x60, 0x3000, 0x8000_0408, 4, 12)]
        );
        let lidos = segmentos(&m).expect("segmentos");
        let lidos: Vec<_> = lidos.iter().map(|g| (g.nome.as_str(), g.vm, g.tamanho, g.arquivo, g.tamanho_arquivo)).collect();
        assert_eq!(lidos, vec![("__TEXT", LINK_BASE, 0x4000, 0, 0x4000), ("__LINKEDIT", 0x1_0000_8000, 0x4000, 0x8000, 0x4000)]);
        assert_eq!(tabela_de_simbolos(&m), Some((0x8000, 30, 0x8400, 0x200)));
        assert_eq!(tabela_indireta(&m), Some((0x9000, 12)));
        let sem_tabelas = crate::macho::macho_de_teste(&[(LC_SEGMENT_64, segmento("__TEXT", LINK_BASE, 0x4000, 0, &[]))]);
        assert_eq!((tabela_de_simbolos(&sem_tabelas), tabela_indireta(&sem_tabelas)), (None, None));
        assert!(secoes(&m[4..]).is_none() && segmentos(&m[4..]).is_none() && tabela_de_simbolos(&m[4..]).is_none() && tabela_indireta(&m[4..]).is_none());
    }

    #[test]
    fn enderecos_do_fonte() {
        let f = "const A: u64 = 0x1_0217_3120; // ver 0x10091dafc e 0x1_0000_0000; 0x12 não; x0x1_0217_3120 não";
        assert_eq!(enderecos_no_fonte(f), vec![0x1_0000_0000, 0x1_0091_dafc, 0x1_0217_3120]);
    }

    #[test]
    fn modulos_e_cfg_do_fonte() {
        let f = "#![allow(dead_code)]\nmod a;\n#[macro_use]\nmod b;\n/// doc\n#[cfg(feature = \"devtools\")] // só no dev\n\npub(crate) mod c;\n\
                 #[cfg(not(feature = \"devtools\"))]\n#[path = \"c_stub.rs\"]\nmod c;\n#[cfg(test)]\nmod testes {\n}\n#[cfg(any(test, feature = \"cpvr\"))]\npub mod d;\n\
                 #[cfg(test)]\nfn f() {}\nmod e;\n";
        let m = modulos_declarados(f);
        let resumo: Vec<(&str, Option<&str>, Vec<&str>)> =
            m.iter().map(|x| (x.nome.as_str(), x.caminho.as_deref(), x.cfg.iter().map(String::as_str).collect())).collect();
        assert_eq!(
            resumo,
            vec![
                ("a", None, vec![]),
                ("b", None, vec![]),
                ("c", None, vec!["feature = \"devtools\""]),
                ("c", Some("c_stub.rs"), vec!["not(feature = \"devtools\")"]),
                ("d", None, vec!["any(test, feature = \"cpvr\")"]),
                ("e", None, vec![]),
            ]
        );
        assert_eq!(cfg_ativo("feature = \"devtools\"", &[]), Some(false));
        assert_eq!(cfg_ativo("feature = \"devtools\"", &["devtools"]), Some(true));
        assert_eq!(cfg_ativo("not(feature = \"devtools\")", &[]), Some(true));
        assert_eq!(cfg_ativo("any(test, feature = \"cpvr\")", &["cpvr"]), Some(true));
        assert_eq!(cfg_ativo("all(feature = \"a\", not(test))", &["a"]), Some(true));
        assert_eq!(cfg_ativo("target_os = \"macos\"", &[]), None);
        assert_eq!(cfg_ativo("any(test, debug_assertions)", &[]), None);
    }

    /// Um texto sintético com uma função repetida em outro layout: a assinatura gerada num acha a cópia no outro.
    #[test]
    fn gera_e_resolve_em_layout_deslocado() {
        let corpo = |base: u32| -> Vec<u32> {
            vec![0xD101_C3FF, 0xA904_57F6, 0x9000_0008 | (base << 5), 0x9100_0108 | (0x40 << 10), 0xF940_0109, 0x9400_0000 | base, 0x7100_053F, 0x5400_0081, 0xAA09_03E0, 0xD65F_03C0]
        };
        let mut a: Vec<u32> = (0..40u32).map(|k| 0x8B00_0000 | k).collect();
        a.extend(corpo(0x100));
        a.extend((0..70u32).map(|k| 0xCB00_0000 | k));
        let mut b: Vec<u32> = (0..13u32).map(|k| 0x8B00_0000 | k).collect();
        b.extend(corpo(0x250));
        b.extend((0..70u32).map(|k| 0xCB00_0000 | k));
        let bytes = |v: &Vec<u32>| v.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>();
        let (ta, tb) = (bytes(&a), bytes(&b));
        let ia = Imagem { texto: &ta, texto_vm: 0x1_0000_4000, secoes: vec![], simbolos: vec![], stubs: vec![] };
        let ib = Imagem { texto: &tb, texto_vm: 0x1_0000_8000, secoes: vec![], simbolos: vec![], stubs: vec![] };
        let alvo = 0x1_0000_4000 + 40 * 4;
        let (tabela, faltam) = gerar(&ia, &[alvo]);
        assert!(faltam.is_empty(), "{faltam:?}");
        assert_eq!(resolver(&ib, &tabela), vec![(alvo, 0x1_0000_8000 + 13 * 4)]);
        assert_eq!(resolver(&ia, &tabela), vec![(alvo, alvo)]);
    }
}

/// Onde a janela da assinatura casa no binário (endereço do começo de cada casamento) — pra conferência e diagnóstico.
pub fn localizar(img: &Imagem, s: &Assinatura) -> Vec<u64> {
    let n = img.palavras();
    (0..n.saturating_sub(s.palavras.len()))
        .filter(|&p| (0..s.palavras.len()).all(|j| mascarada(img.texto, p + j) == s.palavras[j]))
        .map(|p| img.texto_vm + p as u64 * 4)
        .collect()
}
