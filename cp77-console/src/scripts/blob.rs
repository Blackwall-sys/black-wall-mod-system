//! Leitor do `final.redscripts`: o blob que o `scc` grava e o motor carrega.
//!
//! Serve ao R#97 (recarga de script sem reiniciar). A recarga do motor refaz TODAS as funções de
//! script e os caches nativos guardam os ponteiros antigos, então ela derruba o jogo (ver
//! `carregador_de_scripts.rs`). O caminho que sobra é trocar só o CORPO da função mudada: pra isso
//! é preciso ler o bytecode do arquivo e traduzir pro formato que o motor executa, sem recriar
//! objeto do RTTI.
//!
//! Formato conferido contra o compilador que acompanha o projeto (`redscript/core/src`:
//! `bundle.rs` cabeçalho e pools, `definition.rs` corpo das definições, `bytecode.rs` opcodes) e
//! MEDIDO contra o blob do jogo em `calibra_larguras`.
//!
//! **A medida que importa:** no arquivo uma referência ocupa 4 bytes (índice de pool); no bytecode
//! que o motor executa ela ocupa 8 (ponteiro/CName). O `u32` gravado antes do bytecode é o tamanho
//! DEPOIS da tradução, não o do arquivo — é isso que conciliava `Instr::size()` (conta 8) com o
//! `Decode` do compilador (lê 4). Consequências, verificadas no blob real:
//!   - o tamanho do bytecode no arquivo sai do fim do registro da definição (`offset+tamanho`);
//!   - a soma dos tamanhos de runtime tem que dar exatamente o `u32` gravado — checksum de graça
//!     pra qualquer tradução que a gente produza;
//!   - os desvios (`Jump`, `Switch`, `Conditional`...) já estão gravados em unidades de RUNTIME,
//!     então expandir 4→8 não mexe em salto nenhum.

use std::path::Path;

pub(crate) const MAGIC: u32 = 0x5344_4552;
pub(crate) const VERSAO: u32 = 14;
/// Cabeçalho com as 6 tabelas: o que o `scc` grava, e o do jogo até a 2.3.1.
const CABECALHO_LEGADO: usize = 104;
/// O jogo 2.5 liga o bit 0x2 das `flags` (`+8`) e põe um 7º cabeçalho de tabela depois dos 6, com um registro de 12 B por definição
/// (`tmp/cp77-25/scc-0532/diff.txt`). Os offsets das 6 tabelas e das definições são absolutos e já contam os 12 B a mais.
const FLAG_REGISTROS_POR_DEF: u32 = 0x2;
const CABECALHO_25: usize = CABECALHO_LEGADO + 12;
const REGISTRO_POR_DEF: usize = 12;

/// Largura da referência no arquivo (índice de pool) e no bytecode do motor (ponteiro/CName).
pub(crate) const REF_ARQ: usize = 4;
pub(crate) const REF_RT: usize = 8;

/// Tipo de definição, valores do próprio blob (`DefinitionType`).
pub(crate) const TIPO_TIPO: u8 = 0;
pub(crate) const TIPO_CLASSE: u8 = 1;
pub(crate) const TIPO_VALOR_DE_ENUM: u8 = 2;
pub(crate) const TIPO_ENUM: u8 = 3;
pub(crate) const TIPO_FUNCAO: u8 = 5;
pub(crate) const TIPO_PARAMETRO: u8 = 6;
pub(crate) const TIPO_LOCAL: u8 = 7;
pub(crate) const TIPO_CAMPO: u8 = 8;
pub(crate) const TIPO_ARQUIVO_FONTE: u8 = 9;

/// Pool em que um operando de referência indexa.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Pool {
    /// Índice no vetor de definições (função, classe, tipo, campo, local, enum...).
    Def(u8),
    Nome,
    Texto,
    TweakDb,
    Recurso,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Def {
    pub nome: u32,
    pub pai: u32,
    pub offset: u32,
    pub tamanho: u32,
    pub tipo: u8,
}

pub(crate) struct Blob {
    pub bytes: Vec<u8>,
    /// `CABECALHO_LEGADO` (o que o `scc` grava) ou `CABECALHO_25` (o `final.redscripts` vanilla da 2.5).
    pub cabecalho: usize,
    pub nomes: Vec<String>,
    pub tweakdb: Vec<String>,
    pub recursos: Vec<String>,
    pub textos: Vec<String>,
    pub defs: Vec<Def>,
}

/// Bits do `flags` da função no blob (`FunctionFlags` do compilador): `FUNCAO_NATIVA` tira o `source` do registro e cada `FUNCAO_COM_*` põe o
/// trecho dele.
const FUNCAO_NATIVA: u32 = 1 << 4;
const FUNCAO_COM_OPERADOR: u32 = 1 << 7;
const FUNCAO_COM_RETORNO: u32 = 1 << 8;
const FUNCAO_COM_METODO_BASE: u32 = 1 << 9;
const FUNCAO_COM_PARAMETROS: u32 = 1 << 10;
const FUNCAO_COM_LOCAIS: u32 = 1 << 11;
const FUNCAO_COM_CORPO: u32 = 1 << 12;
const FUNCAO_COM_CAST: u32 = 1 << 13;

pub(crate) struct Funcao<'a> {
    pub flags: u32,
    /// Bytecode como está no arquivo (referências de 4 bytes), do primeiro opcode até o fim do
    /// registro da definição. Pode incluir o `unk2` no fim quando a flag 23 está ligada; quem
    /// precisa do limite exato usa `percorrer`, que para pelo tamanho de runtime.
    pub corpo: &'a [u8],
    /// O `u32` gravado antes do bytecode: tamanho que ele terá no motor.
    pub tam_rt: usize,
}

impl Funcao<'_> {
    pub fn nativa(&self) -> bool {
        self.flags & FUNCAO_NATIVA != 0
    }
    pub fn tem_corpo(&self) -> bool {
        self.flags & FUNCAO_COM_CORPO != 0
    }
}

fn u32_em(b: &[u8], p: usize) -> Result<u32, String> {
    b.get(p..p + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| format!("u32 fora do arquivo em {p}"))
}

/// Cabeçalho de tabela: `offset`, `count`, `hash`.
fn tabela(b: &[u8], p: usize) -> Result<(usize, usize), String> {
    Ok((u32_em(b, p)? as usize, u32_em(b, p + 4)? as usize))
}

/// Lê uma C-string no arquivo a partir de `p`.
fn texto_em(b: &[u8], p: usize) -> Result<String, String> {
    let fim = b
        .get(p..)
        .and_then(|s| s.iter().position(|c| *c == 0))
        .ok_or_else(|| format!("texto sem terminador em {p}"))?;
    String::from_utf8(b[p..p + fim].to_vec()).map_err(|e| format!("texto não-UTF8 em {p}: {e}"))
}

fn pool_de_textos(b: &[u8], base: usize, tab: (usize, usize)) -> Result<Vec<String>, String> {
    let (off, n) = tab;
    let mut v = Vec::with_capacity(n);
    for i in 0..n {
        let rel = u32_em(b, off + i * 4)? as usize;
        v.push(texto_em(b, base + rel)?);
    }
    Ok(v)
}

pub(crate) fn ler(caminho: &Path) -> Result<Blob, String> {
    let bytes = std::fs::read(caminho).map_err(|e| format!("{}: {e}", caminho.display()))?;
    ler_bytes(bytes)
}

pub(crate) fn ler_bytes(bytes: Vec<u8>) -> Result<Blob, String> {
    if bytes.len() < CABECALHO_LEGADO {
        return Err("arquivo menor que o cabeçalho".into());
    }
    let magic = u32_em(&bytes, 0)?;
    if magic != MAGIC {
        return Err(format!("magic {magic:#x} não é REDS"));
    }
    let versao = u32_em(&bytes, 4)?;
    if versao != VERSAO {
        return Err(format!("versão {versao} não suportada"));
    }
    let cabecalho = if u32_em(&bytes, 8)? & FLAG_REGISTROS_POR_DEF != 0 { CABECALHO_25 } else { CABECALHO_LEGADO };
    if bytes.len() < cabecalho {
        return Err(format!("arquivo menor que o cabeçalho de {cabecalho} B"));
    }
    let dados = tabela(&bytes, 32)?;
    let t_nomes = tabela(&bytes, 44)?;
    let t_tweak = tabela(&bytes, 56)?;
    let t_recursos = tabela(&bytes, 68)?;
    let t_defs = tabela(&bytes, 80)?;
    let t_textos = tabela(&bytes, 92)?;
    // O bloco de dados começa logo depois do cabeçalho nos dois layouts: flag que não bate com o arquivo vira erro, não leitura deslocada.
    if dados.0 != cabecalho {
        return Err(format!("dados em {} com cabeçalho de {cabecalho} B", dados.0));
    }
    if cabecalho == CABECALHO_25 {
        // A 7ª tabela não é lida (as definições têm offset próprio); conferi-la prova que o layout é o da 2.5.
        let (off, n) = tabela(&bytes, CABECALHO_LEGADO)?;
        if n != t_defs.1 || off.checked_add(n * REGISTRO_POR_DEF).is_none_or(|fim| fim > bytes.len()) {
            return Err(format!("7ª tabela ({off}, {n}) não bate com {} definições em {} B", t_defs.1, bytes.len()));
        }
    }

    // Os offsets dos pools de texto são relativos ao início do bloco de dados.
    let base = dados.0;
    let nomes = pool_de_textos(&bytes, base, t_nomes)?;
    let tweakdb = pool_de_textos(&bytes, base, t_tweak)?;
    let recursos = pool_de_textos(&bytes, base, t_recursos)?;
    let textos = pool_de_textos(&bytes, base, t_textos)?;

    let mut defs = Vec::with_capacity(t_defs.1);
    for i in 0..t_defs.1 {
        let p = t_defs.0 + i * 20;
        defs.push(Def {
            nome: u32_em(&bytes, p)?,
            pai: u32_em(&bytes, p + 4)?,
            offset: u32_em(&bytes, p + 8)?,
            tamanho: u32_em(&bytes, p + 12)?,
            tipo: *bytes.get(p + 16).ok_or("tipo fora do arquivo")?,
        });
    }

    Ok(Blob {
        bytes,
        cabecalho,
        nomes,
        tweakdb,
        recursos,
        textos,
        defs,
    })
}

impl Blob {
    pub fn nome(&self, i: u32) -> &str {
        self.nomes.get(i as usize).map(|s| s.as_str()).unwrap_or("")
    }

    /// Nome como o blob guarda: `nome;TipoDoArg1TipoDoArg2`. O RTTI usa o mesmo formato.
    pub fn nome_da_funcao(&self, i: usize) -> Option<String> {
        let d = self.defs.get(i)?;
        if d.tipo != TIPO_FUNCAO {
            return None;
        }
        let curto = self.nome(d.nome);
        match self.defs.get(d.pai as usize) {
            Some(p) if p.tipo == TIPO_CLASSE => Some(format!("{}::{curto}", self.nome(p.nome))),
            _ => Some(curto.to_string()),
        }
    }

    pub fn funcao(&self, i: usize) -> Result<Funcao<'_>, String> {
        self.funcao_com(i, REF_ARQ)
    }

    /// `lr` = largura da referência no cabeçalho da função. Parâmetro só pra `calibra_larguras`
    /// poder medir de novo se o formato do blob mudar; em produção é sempre `REF_ARQ`.
    pub fn funcao_com(&self, i: usize, lr: usize) -> Result<Funcao<'_>, String> {
        let d = *self
            .defs
            .get(i)
            .ok_or_else(|| format!("definição {i} fora da tabela"))?;
        if d.tipo != TIPO_FUNCAO {
            return Err(format!("definição {i} é tipo {}", d.tipo));
        }
        let b = &self.bytes;
        let fim = (d.offset + d.tamanho) as usize;
        // Pula a visibilidade (u8): o `u32_em` das flags já recusa registro fora do arquivo.
        let mut p = d.offset as usize + 1;
        let flags = u32_em(b, p)?;
        p += 4;
        // `source` só existe em função não-nativa: índice do arquivo (u32) + linha (u32).
        if flags & FUNCAO_NATIVA == 0 {
            p += 8;
        }
        // Retorno, método base, params e locais ninguém usa: os trechos só avançam `p`. O `u32_em`
        // descartado recusa blob truncado também em função sem corpo (flag 12), que não lê mais nada.
        if flags & FUNCAO_COM_RETORNO != 0 {
            u32_em(b, p)?;
            p += lr + 1; // referência + o `unk1` (bool) que só vem com retorno
        }
        if flags & FUNCAO_COM_METODO_BASE != 0 {
            u32_em(b, p)?;
            p += lr;
        }
        if flags & FUNCAO_COM_PARAMETROS != 0 {
            let n = u32_em(b, p)? as usize;
            p += 4;
            for _ in 0..n {
                u32_em(b, p)?;
                p += lr;
            }
        }
        if flags & FUNCAO_COM_LOCAIS != 0 {
            let n = u32_em(b, p)? as usize;
            p += 4;
            for _ in 0..n {
                u32_em(b, p)?;
                p += lr;
            }
        }
        if flags & FUNCAO_COM_OPERADOR != 0 {
            p += 4; // operador
        }
        if flags & FUNCAO_COM_CAST != 0 {
            p += 1; // cast
        }
        let (corpo, tam_rt): (&[u8], usize) = if flags & FUNCAO_COM_CORPO != 0 {
            let n = u32_em(b, p)? as usize;
            p += 4;
            if p > fim {
                return Err(format!("cabeçalho de {i} passou do registro ({p} > {fim})"));
            }
            (
                b.get(p..fim)
                    .ok_or_else(|| format!("corpo de {i} fora do arquivo"))?,
                n,
            )
        } else {
            (&[], 0)
        };
        Ok(Funcao {
            flags,
            corpo,
            tam_rt,
        })
    }
}

/// Uma instrução do bytecode: onde começa no arquivo, opcode e os dois tamanhos.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Instrucao {
    pub pos: usize,
    pub op: u8,
    pub tam_arq: usize,
    pub tam_rt: usize,
}

/// Uma referência achada no bytecode do arquivo.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Referencia {
    /// Posição do índice dentro do bytecode do ARQUIVO.
    pub pos: usize,
    pub pool: Pool,
    pub indice: u32,
}

/// Tamanho do operando + as referências dele (deslocamento dentro do operando + pool).
/// `r` = largura da referência. `None` = opcode desconhecido ou de tamanho variável.
pub(crate) fn forma_com(op: u8, r: usize) -> Option<(usize, u8, [(usize, Pool); 2])> {
    const NADA: (usize, Pool) = (0, Pool::Nome);
    let uma = |t: usize, d: usize, pool: Pool| Some((t, 1u8, [(d, pool), NADA]));
    let nenhuma = |t: usize| Some((t, 0u8, [NADA, NADA]));
    let tipo = |t: usize| uma(t, 0, Pool::Def(TIPO_TIPO));
    match op {
        0..=3 => nenhuma(0),
        4 | 8 => nenhuma(1),
        5 | 9 => nenhuma(2),
        6 | 10 | 12 => nenhuma(4),
        7 | 11 | 13 => nenhuma(8),
        14 => uma(r, 0, Pool::Nome),
        15 => Some((
            2 * r,
            2,
            [(0, Pool::Def(TIPO_ENUM)), (r, Pool::Def(TIPO_VALOR_DE_ENUM))],
        )),
        16 => uma(4, 0, Pool::Texto),
        17 => uma(r, 0, Pool::TweakDb),
        18 => uma(r, 0, Pool::Recurso),
        19 | 20 => nenhuma(0),
        21 => nenhuma(19),
        22 => nenhuma(0),
        23 => nenhuma(0), // Target: não existe no bytecode gravado
        24 => uma(r, 0, Pool::Def(TIPO_LOCAL)),
        25 => uma(r, 0, Pool::Def(TIPO_PARAMETRO)),
        26 => uma(r, 0, Pool::Def(TIPO_CAMPO)),
        27 => nenhuma(0),
        28 => tipo(r + 2),
        29 => nenhuma(4),
        30 => nenhuma(0),
        31..=33 => nenhuma(2),
        34 => nenhuma(4),
        35 => uma(1 + r, 1, Pool::Def(TIPO_CLASSE)),
        36 => uma(4 + r + 2, 4, Pool::Def(TIPO_FUNCAO)),
        37 => uma(4 + r + 2, 4, Pool::Nome),
        38 | 39 => nenhuma(0),
        40 => uma(r, 0, Pool::Def(TIPO_CAMPO)),
        41 => nenhuma(2),
        42..=47 => tipo(r),
        48 => uma(r, 0, Pool::Def(TIPO_CLASSE)),
        49 | 50 => nenhuma(0),
        51 => None, // StartProfiling: tamanho variável, tratado em `percorrer`
        52..=85 => tipo(r),
        86 | 87 => nenhuma(0),
        88 | 89 => tipo(r + 1),
        90 => uma(r + 1, 0, Pool::Def(TIPO_CLASSE)),
        91..=93 => tipo(r),
        94..=101 => nenhuma(0),
        102 | 103 => tipo(r),
        _ => None,
    }
}

/// Percorre o bytecode do arquivo até completar `tam_rt` bytes de runtime (o valor gravado antes do
/// bytecode). Devolve as instruções, as referências e quantos bytes do ARQUIVO foram gastos.
pub(crate) fn percorrer(
    corpo: &[u8],
    tam_rt: usize,
) -> Result<(Vec<Instrucao>, Vec<Referencia>, usize), String> {
    percorrer_com(corpo, tam_rt, REF_ARQ)
}

/// `lb` = largura da referência no arquivo. Em produção é `REF_ARQ`; o parâmetro existe pra
/// `calibra_larguras` medir em vez de assumir.
pub(crate) fn percorrer_com(
    corpo: &[u8],
    tam_rt: usize,
    lb: usize,
) -> Result<(Vec<Instrucao>, Vec<Referencia>, usize), String> {
    let mut instr = Vec::new();
    let mut refs = Vec::new();
    let mut p = 0usize;
    let mut rt = 0usize;
    while rt < tam_rt {
        let op = *corpo.get(p).ok_or_else(|| format!("bytecode acaba em {p}"))?;
        let (tam_arq, tam_rt_instr) = if op == 51 {
            // StartProfiling: u32 de tamanho + texto + bool. Igual nos dois formatos.
            let n = u32_em(corpo, p + 1)? as usize;
            (1 + 4 + n + 1, 1 + 4 + n + 1)
        } else {
            let (ta, n, rs) =
                forma_com(op, lb).ok_or_else(|| format!("opcode {op} desconhecido em {p}"))?;
            let (tr, _, _) = forma_com(op, REF_RT).ok_or("opcode sem forma de runtime")?;
            for (d, pool) in rs.iter().take(n as usize) {
                let pos = p + 1 + d;
                refs.push(Referencia {
                    pos,
                    pool: *pool,
                    indice: u32_em(corpo, pos)?,
                });
            }
            (1 + ta, 1 + tr)
        };
        instr.push(Instrucao {
            pos: p,
            op,
            tam_arq,
            tam_rt: tam_rt_instr,
        });
        p += tam_arq;
        rt += tam_rt_instr;
    }
    if rt != tam_rt {
        return Err(format!("runtime fechou em {rt}, gravado {tam_rt}"));
    }
    Ok((instr, refs, p))
}

#[cfg(test)]
mod teste {
    use super::*;

    fn blob_do_jogo() -> Option<Blob> {
        let jogo = std::env::var("BWMS_GAME")
            .or_else(|_| std::env::var("CP77_DIR"))
            .unwrap_or_else(|_| {
                String::new() // sem CP77_DIR o teste se pula; caminho de máquina não entra no repositório
            });
        let caminho = Path::new(&jogo).join("r6/cache/final.redscripts");
        if !caminho.exists() {
            return None;
        }
        Some(ler(&caminho).expect("blob do jogo"))
    }

    #[test]
    fn le_blob_do_jogo() {
        let Some(b) = blob_do_jogo() else { return };
        assert!(b.defs.len() > 20000, "definições: {}", b.defs.len());
        assert!(b.nomes.len() > 20000, "nomes: {}", b.nomes.len());
        assert!(b.nomes.iter().any(|n| n == "PlayerPuppet"), "sem PlayerPuppet no pool");
        let funcoes = b.defs.iter().filter(|d| d.tipo == TIPO_FUNCAO).count();
        assert!(funcoes > 20000, "funções: {funcoes}");
        for d in &b.defs {
            assert!(d.tipo <= TIPO_ARQUIVO_FONTE, "tipo {}", d.tipo);
            assert!((d.offset as usize) < b.bytes.len(), "offset {} fora do arquivo", d.offset);
        }
    }

    /// Consistência de uma função: o bytecode fecha o tamanho de runtime gravado, gasta exatamente
    /// os bytes que sobram no registro (fora o `unk2` da flag 23) e TODA referência aponta uma
    /// definição do tipo que o opcode pede. Errar largura ou deslocamento quebra os três.
    fn consistente(b: &Blob, i: usize, lr: usize, lb: usize) -> Result<(), String> {
        let f = b.funcao_com(i, lr)?;
        if !f.tem_corpo() {
            return Ok(());
        }
        let (_, refs, gastos) = percorrer_com(f.corpo, f.tam_rt, lb)?;
        if f.flags & (1 << 23) == 0 {
            if gastos != f.corpo.len() {
                return Err(format!("gastou {gastos} de {} bytes do registro", f.corpo.len()));
            }
        } else if gastos > f.corpo.len() {
            return Err(format!("gastou {gastos} > {} do registro", f.corpo.len()));
        }
        for r in &refs {
            let limite = match r.pool {
                Pool::Def(_) => b.defs.len(),
                Pool::Nome => b.nomes.len(),
                Pool::Texto => b.textos.len(),
                Pool::TweakDb => b.tweakdb.len(),
                Pool::Recurso => b.recursos.len(),
            };
            if (r.indice as usize) >= limite {
                return Err(format!("índice {} fora de {:?} ({limite})", r.indice, r.pool));
            }
            if let Pool::Def(esperado) = r.pool {
                let achado = b.defs[r.indice as usize].tipo;
                // `Type` e `Class` se cruzam: `array<X>`/`ref<X>` são Type, e o compilador aponta
                // Type onde a instrução fala de classe.
                let ok = achado == esperado
                    || (esperado == TIPO_TIPO && achado == TIPO_CLASSE)
                    || (esperado == TIPO_CLASSE && achado == TIPO_TIPO);
                if !ok {
                    return Err(format!("ref em {} pedia tipo {esperado}, achou {achado}", r.pos));
                }
            }
        }
        Ok(())
    }

    /// Mede as larguras em vez de assumir: roda as 4 combinações contra o blob do jogo e exige que
    /// a melhor seja a que as constantes declaram, com TODAS as funções consistentes.
    #[test]
    fn calibra_larguras() {
        let Some(b) = blob_do_jogo() else { return };
        let mut relato = Vec::new();
        let mut melhor = (0usize, 0usize, 0usize);
        let mut exemplo = String::new();
        for lr in [4usize, 8] {
            for lb in [4usize, 8] {
                let (mut ok, mut falhas) = (0usize, 0usize);
                let mut primeiro = String::new();
                for i in 0..b.defs.len() {
                    if b.defs[i].tipo != TIPO_FUNCAO {
                        continue;
                    }
                    match consistente(&b, i, lr, lb) {
                        Ok(()) => ok += 1,
                        Err(e) => {
                            falhas += 1;
                            if primeiro.is_empty() {
                                primeiro = format!(
                                    "  1ª falha: {} — {e}",
                                    b.nome_da_funcao(i).unwrap_or_default()
                                );
                            }
                        }
                    }
                }
                relato.push(format!("cab={lr} bc={lb}: ok={ok} falhas={falhas}"));
                if ok > melhor.0 {
                    melhor = (ok, lr, lb);
                    exemplo = primeiro;
                }
            }
        }
        let texto = format!("{}\n{exemplo}", relato.join("\n"));
        assert_eq!(
            (melhor.1, melhor.2),
            (REF_ARQ, REF_ARQ),
            "largura medida diferente da declarada:\n{texto}"
        );
        let funcoes = b.defs.iter().filter(|d| d.tipo == TIPO_FUNCAO).count();
        assert_eq!(melhor.0, funcoes, "não fechou tudo:\n{texto}");
    }

    fn por_u32(b: &mut [u8], p: usize, v: u32) {
        b[p..p + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// Blob mínimo no layout que o `scc` grava: 2 nomes e a função `BwmsTeste;` com corpo `Name(0)` (opcode 14: 5 B no arquivo, 9 no motor).
    fn blob_legado() -> Vec<u8> {
        let mut b = vec![0u8; CABECALHO_LEGADO];
        por_u32(&mut b, 0, MAGIC);
        por_u32(&mut b, 4, VERSAO);
        let dados = b"\0PlayerPuppet\0BwmsTeste;\0";
        b.extend_from_slice(dados);
        let nomes = b.len();
        for rel in [1u32, 14] {
            b.extend_from_slice(&rel.to_le_bytes());
        }
        let corpo = b.len();
        b.push(0); // visibilidade
        for v in [FUNCAO_COM_CORPO, 0, 0, 9] {
            b.extend_from_slice(&v.to_le_bytes()); // flags, arquivo, linha, tamanho no motor
        }
        b.push(14);
        b.extend_from_slice(&0u32.to_le_bytes());
        let tamanho = b.len() - corpo;
        let defs = b.len();
        b.extend_from_slice(&[0u8; 20]);
        for v in [1u32, 0, corpo as u32, tamanho as u32] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[TIPO_FUNCAO, 0, 0, 0]);
        for (p, off, n) in [(32, CABECALHO_LEGADO, dados.len()), (44, nomes, 2), (80, defs, 2)] {
            por_u32(&mut b, p, off as u32);
            por_u32(&mut b, p + 4, n as u32);
        }
        b
    }

    /// O mesmo blob no layout do jogo 2.5: flag 0x2, 7º cabeçalho de tabela nos bytes 104..116, os offsets absolutos (tabelas e corpo das
    /// definições) 12 B adiante e um registro de 12 B por definição no fim.
    fn para_layout_25(legado: &[u8]) -> Vec<u8> {
        let ler = |b: &[u8], p: usize| u32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]]);
        let desvio = (CABECALHO_25 - CABECALHO_LEGADO) as u32;
        let mut b = legado[..CABECALHO_LEGADO].to_vec();
        por_u32(&mut b, 8, ler(legado, 8) | FLAG_REGISTROS_POR_DEF);
        for t in 0..6 {
            let p = 32 + t * 12;
            if ler(legado, p + 4) != 0 {
                por_u32(&mut b, p, ler(legado, p) + desvio);
            }
        }
        let (defs, n) = (ler(legado, 80) as usize, ler(legado, 84));
        for v in [legado.len() as u32 + desvio, n, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&legado[CABECALHO_LEGADO..]);
        for i in 0..n as usize {
            let p = defs + desvio as usize + i * 20 + 8;
            let off = ler(&b, p);
            if off != 0 {
                por_u32(&mut b, p, off + desvio);
            }
        }
        b.resize(b.len() + n as usize * REGISTRO_POR_DEF, 0);
        b
    }

    /// Os dois layouts dão o mesmo blob: nomes, definições (o offset do corpo anda 12 B), corpo da função e percurso do bytecode.
    #[test]
    fn le_os_dois_layouts() {
        let legado = blob_legado();
        let novo = para_layout_25(&legado);
        let (a, b) = (ler_bytes(legado).expect("legado"), ler_bytes(novo).expect("2.5"));
        assert_eq!((a.cabecalho, b.cabecalho), (CABECALHO_LEGADO, CABECALHO_25));
        assert_eq!(a.nomes, ["PlayerPuppet", "BwmsTeste;"]);
        assert_eq!(a.nomes, b.nomes);
        assert_eq!(a.defs.len(), b.defs.len());
        assert_eq!(b.defs[1].offset, a.defs[1].offset + 12);
        assert_eq!(b.nome_da_funcao(1).as_deref(), Some("BwmsTeste;"));
        let (fa, fb) = (a.funcao(1).expect("função legado"), b.funcao(1).expect("função 2.5"));
        assert_eq!((fa.corpo, fa.tam_rt), (fb.corpo, fb.tam_rt));
        assert_eq!((fb.corpo, fb.tam_rt), (&[14u8, 0, 0, 0, 0][..], 9));
        let (_, refs, gastos) = percorrer(fb.corpo, fb.tam_rt).expect("percurso");
        assert_eq!((refs.len(), refs[0].pool, refs[0].indice, gastos), (1, Pool::Nome, 0, 5));
    }

    /// Flag que não bate com o arquivo vira erro: 0x2 num blob de 104 B, 0x2 sem a 7ª tabela coerente, layout 2.5 sem a flag.
    #[test]
    fn layout_trocado_e_recusado() {
        let mut so_flag = blob_legado();
        so_flag[8] |= FLAG_REGISTROS_POR_DEF as u8;
        assert!(ler_bytes(so_flag).is_err());
        let mut sem_registros = para_layout_25(&blob_legado());
        por_u32(&mut sem_registros, CABECALHO_LEGADO + 4, 1);
        assert!(ler_bytes(sem_registros).is_err());
        let mut sem_flag = para_layout_25(&blob_legado());
        sem_flag[8] &= !(FLAG_REGISTROS_POR_DEF as u8);
        assert!(ler_bytes(sem_flag).is_err());
        assert!(ler_bytes(blob_legado()[..CABECALHO_LEGADO - 1].to_vec()).is_err());
    }

    /// Blob real por variável de ambiente (cópia fora da pasta do jogo): `BWMS_BLOB_25` = `final.redscripts` vanilla 2.5, `BWMS_BLOB_SCC` =
    /// o que o `scc` grava. Cada função fecha o percurso e aponta definições do tipo certo (`consistente`).
    fn confere_blob_real(var: &str, cabecalho: usize) {
        let Ok(caminho) = std::env::var(var) else { return };
        let b = ler(Path::new(&caminho)).unwrap_or_else(|e| panic!("{var}: {e}"));
        assert_eq!(b.cabecalho, cabecalho, "{caminho}");
        assert!(b.nomes.iter().any(|n| n == "PlayerPuppet"), "sem PlayerPuppet no pool");
        let funcoes: Vec<usize> = (0..b.defs.len()).filter(|&i| b.defs[i].tipo == TIPO_FUNCAO).collect();
        let falhas: Vec<String> = funcoes
            .iter()
            .filter_map(|&i| consistente(&b, i, REF_ARQ, REF_ARQ).err().map(|e| format!("{}: {e}", b.nome_da_funcao(i).unwrap_or_default())))
            .collect();
        assert!(falhas.is_empty(), "{} de {} funções falharam; 1ª: {}", falhas.len(), funcoes.len(), falhas[0]);
        eprintln!("{var}: cabeçalho {} B, {} definições, {} funções consistentes, {} nomes", b.cabecalho, b.defs.len(), funcoes.len(), b.nomes.len());
    }

    #[test]
    fn le_blob_vanilla_25() {
        confere_blob_real("BWMS_BLOB_25", CABECALHO_25);
    }

    #[test]
    fn le_blob_do_scc() {
        confere_blob_real("BWMS_BLOB_SCC", CABECALHO_LEGADO);
    }
}
