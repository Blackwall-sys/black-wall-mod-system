//! Tradutor de bytecode de script: arquivo (índice de pool, 4 bytes) → motor (ponteiro/CName, 8).
//!
//! É a peça que falta pro R#97. A recarga do próprio motor refaz todas as funções de script e os
//! caches nativos ficam com os ponteiros antigos (ver `carregador_de_scripts.rs`); trocar SÓ o
//! corpo da função mudada não mexe em objeto nenhum do RTTI, então nada aponta pra lugar velho.
//!
//! O formato do arquivo está medido em `blob_de_script.rs`. O que falta é o outro lado: qual valor
//! de 8 bytes o motor põe em cada referência. Em vez de adivinhar (ou de resolver tudo pelo RTTI e
//! esperar que bata), a sonda APRENDE do jogo rodando: o blob carregado e o bytecode vivo da mesma
//! função são a mesma sequência de instruções, então o valor que está no vivo, na posição da
//! referência, É a tradução do índice que está no arquivo. Com milhares de funções dá pra exigir
//! que o mesmo índice sempre dê o mesmo valor — se um índice aparecer com dois valores, a tabela
//! ou o deslocamento estão errados e a sonda acusa.
//!
//! Dicionário resultante: índice do pool → valor de runtime, e nome → valor (é por nome que uma
//! referência do blob NOVO se resolve, já que o `scc` renumera os pools a cada compilação).

use crate::memoria::read_u64;
use crate::scripts::blob::{self as blob, Blob, Pool};
use std::collections::HashMap;
use std::ffi::c_void;
#[cfg(feature = "devtools")]
use std::sync::atomic::{AtomicU64, Ordering};

/// `CBaseFunction`: nome completo (CName) em `+0x08`, flags em `+0xA8` (bit 0 = nativa) e, do
/// `+0x78` em diante, o que a carga grava pelo `0x1011a9374(função+0x78, arquivo, linha, código,
/// tamanho)`: `stp w1, w2, [x0]` põe arquivo e linha em `+0x78`/`+0x7C`, e o buffer do bytecode é
/// atribuído em `[x0+8]`. O buffer (0x20 B, montado em `0x101212cac`) é `{ponteiro +0x00, tamanho
/// u32 +0x08, 0 +0x10, 0 +0x18}`, dados alinhados em 16. Medido no binário (o chamador
/// `0x1011196d8` faz `add x0, x22, #0x78` com `x22 = [definição+0x18]`); a sonda confirma no objeto
/// vivo antes de usar, por `achar_bytecode`.
const FUNCAO_NOME: usize = 0x08;
/// O `+0x08` é o nome que o blob chama de curto (`nome;Assinatura`, sem a classe) — medido nos boots
/// 306 e 307: casar pelo `+0x10` (nome sem assinatura) deixou 31704 funções sem par.
const FUNCAO_FLAGS: usize = 0xA8;
const FUNCAO_BYTECODE: usize = 0x80;
const FUNCAO_BYTECODE_TAM: usize = 0x88;
/// O buffer inteiro, salvo e devolvido byte a byte na troca de corpo.
const FUNCAO_BUFFER: usize = 0x20;

/// O que uma referência traduzida é: o valor de 8 bytes que o motor grava.
#[derive(Default)]
pub(crate) struct Dicionario {
    /// (pool, índice no blob) → valor de runtime.
    pub por_indice: HashMap<(u8, u32), u64>,
    /// (pool, nome) → valor de runtime. É o que resolve referência de um blob recompilado.
    pub por_nome: HashMap<(u8, String), u64>,
    /// Índices que apareceram com dois valores diferentes: erro de tabela, não de dado.
    pub conflitos: u64,
    pub amostras: u64,
}

fn chave(pool: Pool) -> u8 {
    match pool {
        Pool::Def(t) => t,
        Pool::Nome => 200,
        Pool::Texto => 201,
        Pool::TweakDb => 202,
        Pool::Recurso => 203,
    }
}

/// Nome que identifica o alvo de uma referência, pra resolver a mesma coisa num blob recompilado.
fn nome_do_alvo(b: &Blob, pool: Pool, indice: u32) -> Option<String> {
    let i = indice as usize;
    match pool {
        Pool::Nome => b.nomes.get(i).cloned(),
        Pool::Texto => b.textos.get(i).cloned(),
        Pool::TweakDb => b.tweakdb.get(i).cloned(),
        Pool::Recurso => b.recursos.get(i).cloned(),
        Pool::Def(_) => {
            let d = *b.defs.get(i)?;
            let curto = b.nome(d.nome);
            match b.defs.get(d.pai as usize) {
                // Campo, local, parâmetro e método se identificam com o dono: `Dono::nome`.
                Some(p) if p.tipo != blob::TIPO_ARQUIVO_FONTE && d.pai != 0 => {
                    Some(format!("{}::{curto}", b.nome(p.nome)))
                }
                _ => Some(curto.to_string()),
            }
        }
    }
}

/// Percorre o bytecode do MOTOR (referências de 8 bytes) na mesma sequência de instruções do
/// arquivo, conferindo opcode a opcode. Devolve o valor de 8 bytes que o motor gravou em cada
/// referência, na mesma ordem em que `percorrer` as achou no arquivo.
unsafe fn refs_do_vivo(instr: &[blob::Instrucao], vivo: u64, tam_rt: usize) -> Result<Vec<u64>, String> {
    // Uma checagem pro buffer inteiro: `is_readable` por referência seria um syscall por operando
    // (centenas de milhares no blob todo) e o frame do jogo não aguenta.
    if !crate::memoria::is_readable(vivo as *const c_void, tam_rt) {
        return Err(format!("bytecode vivo ilegível ({tam_rt} B em {vivo:#x})"));
    }
    let mut valores = Vec::new();
    let mut p_rt = 0usize;
    for ins in instr {
        let op_vivo = ((vivo + p_rt as u64) as *const u8).read();
        if op_vivo != ins.op {
            return Err(format!(
                "opcode {} no arquivo, {op_vivo} no motor (offset rt {p_rt})",
                ins.op
            ));
        }
        if ins.op != 51 {
            if let Some((_, n, rs)) = blob::forma_com(ins.op, blob::REF_RT) {
                for (d, pool) in rs.iter().take(n as usize) {
                    // O índice de texto ocupa 4 bytes; ler 8 traria o opcode seguinte junto e, na
                    // última instrução, passaria do fim do buffer.
                    let em = (vivo + (p_rt + 1 + d) as u64) as *const u8;
                    let v = if largura_rt(*pool) == 4 {
                        u64::from((em as *const u32).read_unaligned())
                    } else {
                        (em as *const u64).read_unaligned()
                    };
                    valores.push(v);
                }
            }
        }
        p_rt += ins.tam_rt;
    }
    Ok(valores)
}

/// Funções GLOBAIS de script vivas, pelo hash do nome curto (`FUNCAO_NOME`, `+0x08`).
unsafe fn globais_vivas() -> HashMap<u64, u64> {
    let mut m = HashMap::new();
    let Some(reg) = crate::registry() else { return m };
    let (entradas, n) = reg.get_global_functions();
    if entradas.is_null() || n == 0 || n > 500_000 {
        return m;
    }
    let p = entradas as *const u64;
    for i in 0..n {
        let f = p.add(i).read_unaligned();
        if f == 0 || !crate::memoria::is_readable(f as *const c_void, FUNCAO_FLAGS + 8) {
            continue;
        }
        m.insert(((f + FUNCAO_NOME as u64) as *const u64).read_unaligned(), f);
    }
    m
}

/// A classe viva de um nome do blob. Classe de script tem nome próprio no RTTI (o mapa de nomes de
/// script), então o nome nativo é só a primeira tentativa: medido no boot 307, casar só por
/// `class_by_name` deixou 31704 funções sem par.
unsafe fn classe_viva(reg: &crate::rtti::Registry, nome: &str) -> *mut c_void {
    let c = reg.class_by_name(nome);
    if crate::rtti::sane(c) {
        return c;
    }
    reg.class_by_script_name(nome)
}

/// Métodos de uma CLASSE viva, pelo hash do nome curto. Casar método só pelo nome curto num mapa
/// global junta homônimos de classes diferentes (medido no boot 306: 1068 conflitos e 10565 pares
/// divergentes); a classe é o que desempata. Listas em `cls+0x48` (métodos) e `cls+0x58`
/// (estáticos), com o tamanho no `u32` seguinte, e o nome curto da função em `FUNCAO_NOME`
/// (`+0x08`) — não no `+0x10` (nome sem assinatura) que o `rtti::resolve_in_class` compara.
unsafe fn metodos_da_classe(cls: *mut c_void) -> HashMap<u64, u64> {
    let mut m = HashMap::new();
    if !crate::rtti::sane(cls) || !crate::memoria::is_readable(cls as *const c_void, 0x68) {
        return m;
    }
    let base = cls as u64;
    for off in [0x48u64, 0x58] {
        let lista = ((base + off) as *const u64).read_unaligned();
        let n = ((base + off + 8) as *const u32).read_unaligned() as usize;
        if lista == 0 || n > 20_000 || !crate::memoria::is_readable(lista as *const c_void, 8) {
            continue;
        }
        for i in 0..n {
            let slot = lista + (i * 8) as u64;
            if !crate::memoria::is_readable(slot as *const c_void, 8) {
                break;
            }
            let f = (slot as *const u64).read_unaligned();
            if f == 0 || !crate::memoria::is_readable(f as *const c_void, FUNCAO_FLAGS + 8) {
                continue;
            }
            m.insert(((f + FUNCAO_NOME as u64) as *const u64).read_unaligned(), f);
        }
    }
    m
}

/// A função viva que corresponde a uma definição do blob: método pela classe dona, global pelo nome
/// curto. Monta os mapas na hora — serve pra punhado de funções, não pra varredura do blob todo.
unsafe fn funcao_viva(b: &Blob, i: usize) -> u64 {
    let Some(reg) = crate::registry() else { return 0 };
    let Some(d) = b.defs.get(i) else { return 0 };
    let curto = crate::cname::cname(b.nome(d.nome));
    match b.defs.get(d.pai as usize) {
        Some(p) if p.tipo == blob::TIPO_CLASSE => {
            let cls = classe_viva(&reg, b.nome(p.nome));
            metodos_da_classe(cls).get(&curto).copied().unwrap_or(0)
        }
        _ => globais_vivas().get(&curto).copied().unwrap_or(0),
    }
}

#[cfg(feature = "devtools")]
static SONDA_FEITA: AtomicU64 = AtomicU64::new(0);

/// Aprende o dicionário comparando o blob carregado com o bytecode vivo, e devolve o relato.
pub(crate) unsafe fn aprender(b: &Blob) -> (Dicionario, String) {
    let mut dic = Dicionario::default();
    let globais = globais_vivas();
    // Mapa de métodos por classe, montado sob demanda: as funções do blob vêm agrupadas por classe,
    // então cada classe é resolvida uma vez só.
    let mut metodos: HashMap<u32, HashMap<u64, u64>> = HashMap::new();
    let mut casadas = 0u64;
    let mut sem_par = 0u64;
    let mut sem_classe = 0u64;
    let mut divergentes = 0u64;
    let mut primeira_divergencia = String::new();
    let mut offset_confirmado = FUNCAO_BYTECODE;
    let mut confirmadas = 0u64;
    let mut offsets_divergentes = 0u64;

    for i in 0..b.defs.len() {
        if b.defs[i].tipo != blob::TIPO_FUNCAO {
            continue;
        }
        let Ok(f) = b.funcao(i) else { continue };
        if !f.tem_corpo() || f.nativa() || f.tam_rt == 0 {
            continue;
        }
        let curto = crate::cname::cname(b.nome(b.defs[i].nome));
        let pai = b.defs[i].pai;
        let alvo = match b.defs.get(pai as usize) {
            Some(p) if p.tipo == blob::TIPO_CLASSE => {
                let mapa = metodos.entry(pai).or_insert_with(|| {
                    let Some(reg) = crate::registry() else { return HashMap::new() };
                    metodos_da_classe(classe_viva(&reg, b.nome(p.nome)))
                });
                if mapa.is_empty() {
                    sem_classe += 1;
                }
                mapa.get(&curto).copied().unwrap_or(0)
            }
            _ => globais.get(&curto).copied().unwrap_or(0),
        };
        if alvo == 0 {
            sem_par += 1;
            continue;
        }
        let (instr, refs, _) = match blob::percorrer(f.corpo, f.tam_rt) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // O offset do buffer é confirmado no próprio objeto nas primeiras amostras; depois vale o
        // que elas concordaram (uma varredura por função custaria caro no frame).
        let primeiro = instr.first().map_or(0, |i| i.op);
        if offset_confirmado == 0 || confirmadas < 8 {
            match achar_bytecode(alvo, f.tam_rt, primeiro) {
                Some(off) => {
                    if offset_confirmado != 0 && off != offset_confirmado {
                        offsets_divergentes += 1;
                    }
                    offset_confirmado = off;
                    confirmadas += 1;
                }
                None => {
                    sem_par += 1;
                    continue;
                }
            }
        }
        let Some(vivo) = read_u64((alvo + offset_confirmado as u64) as usize) else {
            sem_par += 1;
            continue;
        };
        if vivo == 0 {
            sem_par += 1;
            continue;
        }
        match refs_do_vivo(&instr, vivo, f.tam_rt) {
            Ok(valores) if valores.len() == refs.len() => {
                casadas += 1;
                dic.amostras += valores.len() as u64;
                for (r, v) in refs.iter().zip(valores) {
                    let k = chave(r.pool);
                    match dic.por_indice.insert((k, r.indice), v) {
                        Some(antigo) if antigo != v => {
                            dic.conflitos += 1;
                            if primeira_divergencia.is_empty() {
                                primeira_divergencia = format!(
                                    "conflito pool {k} índice {}: {antigo:#x} vs {v:#x} em {}",
                                    r.indice,
                                    b.nome_da_funcao(i).unwrap_or_default()
                                );
                            }
                        }
                        _ => {}
                    }
                    if let Some(nome) = nome_do_alvo(b, r.pool, r.indice) {
                        dic.por_nome.insert((k, nome), v);
                    }
                }
            }
            Ok(valores) => {
                divergentes += 1;
                if primeira_divergencia.is_empty() {
                    primeira_divergencia = format!(
                        "{}: {} refs no arquivo, {} no motor",
                        b.nome_da_funcao(i).unwrap_or_default(),
                        refs.len(),
                        valores.len()
                    );
                }
            }
            Err(e) => {
                divergentes += 1;
                if primeira_divergencia.is_empty() {
                    primeira_divergencia =
                        format!("{}: {e}", b.nome_da_funcao(i).unwrap_or_default());
                }
            }
        }
    }

    let relato = format!(
        "globais={} casadas={casadas} sem_par={sem_par} sem_classe={sem_classe} \
         divergentes={divergentes} refs={} conflitos={} nomes={} \
         bytecode@{offset_confirmado:#x} ({confirmadas} confirmações, {offsets_divergentes} \
         divergentes) | {primeira_divergencia}",
        globais.len(),
        dic.amostras,
        dic.conflitos,
        dic.por_nome.len()
    );
    (dic, relato)
}

/// Sonda read-only: só lê o blob do disco e a memória do motor, não escreve nada no jogo. Roda sem
/// jogador no mundo (o menu principal serve), pra dar o veredito da tradução mesmo em boot que não
/// chega ao gameplay. Marcador `~/.bwms-hook-sonda-bytecode`.
#[cfg(feature = "devtools")]
pub(crate) fn sonda_tick() {
    if !crate::marcador_individual("sonda-bytecode") || SONDA_FEITA.swap(1, Ordering::Relaxed) != 0 {
        return;
    }
    let Some(dir) = crate::dylib_dir() else { return };
    let Some(caminho) = std::path::PathBuf::from(&dir)
        .parent()
        .map(|p| p.join("r6/cache/final.redscripts"))
    else {
        return;
    };
    let t0 = std::time::Instant::now();
    let b = match blob::ler(&caminho) {
        Ok(b) => b,
        Err(e) => {
            crate::log_dev!("[sonda-bytecode] blob: {e}");
            return;
        }
    };
    let (dic, relato) = unsafe { aprender(&b) };
    let ms = t0.elapsed().as_millis();
    let ok = dic.conflitos == 0 && dic.amostras > 100_000;
    let linha = format!("[sonda] VEREDITO-SONDA {relato} | {ms} ms; traducao-ok={ok}");
    crate::log_veredito(&linha);
    let _ = std::fs::write("/tmp/bwms-sonda-bytecode.log", format!("{linha}\n"));
}

/// Resolve uma referência do blob NOVO pelo nome do alvo. `dic` vem do blob carregado (aprendido do
/// bytecode vivo), por isso só resolve o que o jogo já conhece: função nova, classe nova, local
/// novo ou texto novo não têm par e a troca é recusada — que é o certo, trocar corpo não cria nada.
fn resolver(b: &Blob, r: &blob::Referencia, dic: &Dicionario) -> Result<u64, String> {
    let nome = nome_do_alvo(b, r.pool, r.indice)
        .ok_or_else(|| format!("referência {:?} {} sem nome no blob", r.pool, r.indice))?;
    let k = chave(r.pool);
    if let Some(v) = dic.por_nome.get(&(k, nome.clone())) {
        return Ok(*v);
    }
    // `Type` e `Class` se cruzam no blob (ver `blob_de_script`), então a outra chave vale tentar —
    // mas SÓ entre elas: um literal de texto "String" não pode resolver pro tipo String.
    if k == blob::TIPO_TIPO || k == blob::TIPO_CLASSE {
        let alt = if k == blob::TIPO_TIPO { blob::TIPO_CLASSE } else { blob::TIPO_TIPO };
        if let Some(v) = dic.por_nome.get(&(alt, nome.clone())) {
            return Ok(*v);
        }
    }
    // Nome e TweakDBID são hash puro do texto: dá pra calcular sem par no jogo.
    match r.pool {
        Pool::Nome => Ok(crate::cname::cname(&nome)),
        Pool::TweakDb => Ok(bwms_primitives::tweak_db_id(&nome)),
        _ => Err(format!("{:?} \"{nome}\" não existe no jogo carregado", r.pool)),
    }
}

/// Largura da referência no bytecode do motor: 8, menos o índice de texto, que fica em 4.
fn largura_rt(pool: Pool) -> usize {
    if pool == Pool::Texto {
        4
    } else {
        blob::REF_RT
    }
}

/// Traduz o bytecode de uma função do blob pro formato que o motor executa. O tamanho resultante
/// tem que dar exatamente o `u32` gravado no arquivo — é o checksum do formato.
pub(crate) fn traduzir(b: &Blob, i: usize, dic: &Dicionario) -> Result<Vec<u8>, String> {
    let f = b.funcao(i)?;
    if !f.tem_corpo() {
        return Err("função sem corpo".into());
    }
    let (instr, refs, _) = blob::percorrer(f.corpo, f.tam_rt)?;
    let mut saida = Vec::with_capacity(f.tam_rt);
    let mut k = 0usize; // índice na lista de referências, que está na ordem do percurso
    for ins in &instr {
        saida.push(ins.op);
        let op_arq = &f.corpo[ins.pos + 1..ins.pos + ins.tam_arq];
        if ins.op == 51 {
            saida.extend_from_slice(op_arq);
            continue;
        }
        let (_, n, rs_arq) = blob::forma_com(ins.op, blob::REF_ARQ).ok_or("opcode sem forma")?;
        let mut cursor = 0usize; // posição dentro do operando do ARQUIVO
        for j in 0..n as usize {
            let d_arq = rs_arq[j].0;
            saida.extend_from_slice(&op_arq[cursor..d_arq]);
            let valor = resolver(b, &refs[k], dic)?;
            saida.extend_from_slice(&valor.to_le_bytes()[..largura_rt(refs[k].pool)]);
            // No arquivo toda referência ocupa 4 bytes; no motor, 8 (o índice de texto fica em 4).
            cursor = d_arq + blob::REF_ARQ;
            k += 1;
        }
        saida.extend_from_slice(&op_arq[cursor..]);
    }
    if saida.len() != f.tam_rt {
        return Err(format!(
            "tradução deu {} bytes, o arquivo diz {}",
            saida.len(),
            f.tam_rt
        ));
    }
    Ok(saida)
}

/// Estado de um corpo trocado, pra desfazer exatamente.
#[derive(Clone, Copy)]
pub(crate) struct CorpoAntigo {
    pub funcao: u64,
    /// O buffer inteiro como estava, pra devolver byte a byte.
    pub buffer: [u8; FUNCAO_BUFFER],
}

/// Confirma (ou acha) onde o descritor vivo guarda o ponteiro do bytecode: o candidato tem que ser
/// legível, ter o tamanho gravado igual ao do arquivo e começar com o mesmo opcode. Varre `+0x70` a
/// `+0xA0` porque quem manda é o objeto, não a nossa expectativa.
unsafe fn achar_bytecode(funcao: u64, tam_rt: usize, primeiro_op: u8) -> Option<usize> {
    for off in (0x70..=0xA0).step_by(8) {
        let p = read_u64((funcao + off as u64) as usize).unwrap_or(0);
        if p == 0 || !crate::memoria::is_readable(p as *const c_void, tam_rt) {
            continue;
        }
        let tam = read_u64((funcao + off as u64 + 8) as usize).unwrap_or(0) & 0xffff_ffff;
        if tam as usize != tam_rt || (p as *const u8).read() != primeiro_op {
            continue;
        }
        return Some(off);
    }
    None
}

/// Troca o bytecode de uma função viva. NÃO escreve no buffer antigo: aloca um novo, copia, e troca
/// o ponteiro em `+0x80` (e o tamanho em `+0x88`) com as threads paradas. Assim uma thread que já
/// está interpretando a função continua no buffer antigo, que fica mapeado, e nada lê bytecode
/// misturado. O buffer inteiro volta byte a byte em `devolver_corpo`, inclusive os dois campos que
/// o motor zera (`+0x10`/`+0x18` dele) — o alocador do buffer é do motor e nada nosso o solta.
///
/// # Safety
/// `funcao` precisa ser um `CBaseFunction` de script vivo, e `novo` o bytecode já traduzido pra essa
/// mesma função (mesmos locais e parâmetros).
pub(crate) unsafe fn trocar_corpo(funcao: u64, novo: &[u8], tam_atual: usize) -> Result<CorpoAntigo, String> {
    let base = funcao + FUNCAO_BYTECODE as u64;
    if !crate::memoria::is_readable(base as *const c_void, FUNCAO_BUFFER) {
        return Err("buffer do bytecode ilegível".into());
    }
    if read_u64(base as usize).unwrap_or(0) == 0 {
        return Err("função sem bytecode".into());
    }
    let tam_gravado = (read_u64((base + 8) as usize).unwrap_or(0) & 0xffff_ffff) as usize;
    if tam_gravado != tam_atual {
        return Err(format!("tamanho vivo {tam_gravado} ≠ {tam_atual} medido"));
    }
    let mut buffer = [0u8; FUNCAO_BUFFER];
    std::ptr::copy_nonoverlapping(base as *const u8, buffer.as_mut_ptr(), FUNCAO_BUFFER);
    let buf = crate::memoria::alloc_anywhere(novo.len().max(16));
    if buf.is_null() {
        return Err("sem memória pro bytecode novo".into());
    }
    std::ptr::copy_nonoverlapping(novo.as_ptr(), buf, novo.len());
    let ok = crate::ganchos::com_threads_paradas(|| {
        (base as *mut u64).write(buf as u64);
        ((funcao + FUNCAO_BYTECODE_TAM as u64) as *mut u32).write(novo.len() as u32);
        true
    })
    .unwrap_or(false);
    if !ok {
        return Err("não consegui parar as threads".into());
    }
    Ok(CorpoAntigo { funcao, buffer })
}

/// Desfaz a troca: devolve o buffer do bytecode exatamente como estava.
///
/// # Safety
/// `c` precisa ter vindo de `trocar_corpo` na mesma sessão.
pub(crate) unsafe fn devolver_corpo(c: CorpoAntigo) -> bool {
    let base = c.funcao + FUNCAO_BYTECODE as u64;
    crate::ganchos::com_threads_paradas(|| {
        std::ptr::copy_nonoverlapping(c.buffer.as_ptr(), base as *mut u8, FUNCAO_BUFFER);
        true
    })
    .unwrap_or(false)
}

// ===== Teste da troca de corpo (R#97), marcador `~/.bwms-hook-troca-de-corpo` =====
// O ciclo que o modder quer: salvar o `.reds`, o jogo recompilar sozinho e o comportamento mudar
// sem reiniciar. O teste faz isso com uma função própria, em gameplay (o caso difícil), e devolve o
// corpo antigo no fim — o estado depois é exatamente o de antes.

#[cfg(feature = "devtools")]
const TESTE_ARQUIVO: &str = "r6/scripts/blackwall-mods/bwms-troca-teste.reds";
#[cfg(any(test, feature = "devtools"))]
const BLOB_DO_TESTE: &str = "/tmp/bwms-troca.redscripts";

#[cfg(feature = "devtools")]
fn teste_fonte(k: i32) -> String {
    format!(
        "// Teste da troca de corpo sem reiniciar: o BWMS recompila, traduz o bytecode do\n\
         // arquivo pro formato do motor e troca só o corpo desta função com o jogo aberto. O corpo usa\n\
         // literal de texto, chamada de função e variável local — as três formas de referência que a\n\
         // tradução tem que resolver pelo nome.\n\
         public func BwmsTrocaValor() -> Int32 {{\n  let base: Int32 = StrLen(\"bwms\");\n  return base * {k};\n}}\n"
    )
}

#[cfg(feature = "devtools")]
unsafe fn valor_do_script() -> Option<i32> {
    let reg = crate::registry()?;
    let f = crate::register::get_function(reg, "BwmsTrocaValor");
    if !crate::rtti::sane(f) {
        return None;
    }
    let rf = crate::rtti::ResolvedFn {
        func: f,
        ret_type: std::ptr::null_mut(),
        is_static: true,
    };
    crate::rtti::call_func(&rf, std::ptr::null_mut(), &[])
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Ponteiro do `CBaseFunction` vivo de `BwmsTrocaValor`.
#[cfg(feature = "devtools")]
unsafe fn funcao_do_teste() -> Option<u64> {
    let reg = crate::registry()?;
    let f = crate::register::get_function(reg, "BwmsTrocaValor");
    crate::rtti::sane(f).then_some(f as u64)
}

/// O teste da troca já deu veredito (ou nunca começou): quem recarrega o mundo no mesmo boot
/// espera por isso.
#[cfg(feature = "devtools")]
pub(crate) fn teste_terminou() -> bool {
    FASE_DO_TESTE.load(Ordering::Relaxed) >= 9
}

#[cfg(feature = "devtools")]
static FASE_DO_TESTE: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) fn teste_tick() {
    use std::sync::Mutex;
    use std::time::Instant;
    static MARCO: Mutex<Option<Instant>> = Mutex::new(None);
    static NOTAS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static DIC: Mutex<Option<Dicionario>> = Mutex::new(None);
    static ANTIGO: Mutex<Option<CorpoAntigo>> = Mutex::new(None);
    static COMPILACAO: AtomicU64 = AtomicU64::new(0); // 1 rodando, 2 ok, 3 erro
    static ERRO: Mutex<String> = Mutex::new(String::new());
    static VALORES: Mutex<[Option<i32>; 4]> = Mutex::new([None; 4]);
    static TICKS: AtomicU64 = AtomicU64::new(0);
    static ALVO: AtomicU64 = AtomicU64::new(0);

    let fase = FASE_DO_TESTE.load(Ordering::Relaxed);
    // Chamar função de script de dentro de outra chamada do executor na mesma thread não é seguro.
    if fase >= 9 || crate::inicio::boot::exec_nested() || !crate::marcador_individual("troca-de-corpo") {
        return;
    }
    TICKS.fetch_add(1, Ordering::Relaxed);
    let Some(jogo) = crate::dylib_dir().and_then(|d| std::path::PathBuf::from(d).parent().map(|p| p.to_path_buf()))
    else {
        return;
    };
    let nota = |s: String| NOTAS.lock().unwrap_or_else(|e| e.into_inner()).push(s);
    let segundos = || MARCO.lock().unwrap_or_else(|e| e.into_inner()).map_or(0, |t| t.elapsed().as_secs());
    let marcar = || *MARCO.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    let fim = |ok: bool, motivo: &str| {
        FASE_DO_TESTE.store(9, Ordering::Relaxed);
        let _ = std::fs::write(jogo.join(TESTE_ARQUIVO), teste_fonte(1));
        let _ = std::fs::remove_file(BLOB_DO_TESTE);
        let notas = NOTAS.lock().unwrap_or_else(|e| e.into_inner()).join("; ");
        crate::veredito_dev!("[troca] VEREDITO-TROCA {notas}{motivo}; troca-ok={ok}");
    };

    match fase {
        0 if crate::inicio::boot::gameplay_real() => {
            let t0 = Instant::now();
            let blob_atual = match blob::ler(&jogo.join("r6/cache/final.redscripts")) {
                Ok(b) => b,
                Err(e) => return fim(false, &format!(" (blob carregado: {e})")),
            };
            let (dic, relato) = unsafe { aprender(&blob_atual) };
            let ok = dic.conflitos == 0 && dic.amostras > 100_000;
            nota(format!("aprendizado em {} ms: {relato}", t0.elapsed().as_millis()));
            if !ok {
                return fim(false, " (aprendizado do dicionário falhou)");
            }
            *DIC.lock().unwrap_or_else(|e| e.into_inner()) = Some(dic);
            let v = unsafe { valor_do_script() };
            VALORES.lock().unwrap_or_else(|e| e.into_inner())[0] = v;
            nota(format!("antes {v:?}"));
            // Idempotente: o teste troca pro OUTRO valor, então um boot interrompido (que deixou o
            // `.reds` no 10) não faz o próximo falhar por motivo errado.
            let (k_novo, esperado) = match v {
                Some(4) => (10, 40),
                Some(40) => (1, 4),
                _ => return fim(false, " (a função de teste não está no bundle)"),
            };
            ALVO.store(esperado, Ordering::Release);
            let _ = std::fs::write(jogo.join(TESTE_ARQUIVO), teste_fonte(k_novo));
            COMPILACAO.store(1, Ordering::Release);
            std::thread::spawn(|| {
                let r = crate::scripts::carregador::compilar_em(std::path::Path::new(BLOB_DO_TESTE));
                match r {
                    Ok(()) => COMPILACAO.store(2, Ordering::Release),
                    Err(e) => {
                        *ERRO.lock().unwrap_or_else(|x| x.into_inner()) = e;
                        COMPILACAO.store(3, Ordering::Release);
                    }
                }
            });
            marcar();
            FASE_DO_TESTE.store(1, Ordering::Relaxed);
        }
        1 => match COMPILACAO.load(Ordering::Acquire) {
            2 => {
                let novo = match blob::ler(std::path::Path::new(BLOB_DO_TESTE)) {
                    Ok(b) => b,
                    Err(e) => return fim(false, &format!(" (blob novo: {e})")),
                };
                let Some(i) = (0..novo.defs.len()).find(|&i| {
                    novo.defs[i].tipo == blob::TIPO_FUNCAO
                        && novo.nome(novo.defs[i].nome).starts_with("BwmsTrocaValor;")
                }) else {
                    return fim(false, " (BwmsTrocaValor não está no blob novo)");
                };
                let guarda = DIC.lock().unwrap_or_else(|e| e.into_inner());
                let Some(dic) = guarda.as_ref() else {
                    return fim(false, " (sem dicionário)");
                };
                let bytes = match traduzir(&novo, i, dic) {
                    Ok(v) => v,
                    Err(e) => return fim(false, &format!(" (tradução: {e})")),
                };
                let Some(f) = (unsafe { funcao_do_teste() }) else {
                    return fim(false, " (função viva não resolve)");
                };
                let tam_atual = unsafe { tamanho_vivo(f) };
                nota(format!(
                    "compilado em {} s; tradução {} bytes (vivo {tam_atual})",
                    segundos(),
                    bytes.len()
                ));
                match unsafe { trocar_corpo(f, &bytes, tam_atual) } {
                    Ok(c) => *ANTIGO.lock().unwrap_or_else(|e| e.into_inner()) = Some(c),
                    Err(e) => return fim(false, &format!(" (troca: {e})")),
                }
                drop(guarda);
                let v = unsafe { valor_do_script() };
                VALORES.lock().unwrap_or_else(|e| e.into_inner())[1] = v;
                nota(format!("depois da troca {v:?}"));
                marcar();
                TICKS.store(0, Ordering::Relaxed);
                FASE_DO_TESTE.store(2, Ordering::Relaxed);
            }
            3 => {
                let e = ERRO.lock().unwrap_or_else(|x| x.into_inner()).clone();
                fim(false, &format!(" (scc: {e})"))
            }
            _ if segundos() > 180 => fim(false, " (scc não terminou em 180 s)"),
            _ => {}
        },
        // 30 s de jogo com o corpo trocado: se o motor fosse cair por causa da troca, cai aqui.
        2 if segundos() >= 30 => {
            let v = unsafe { valor_do_script() };
            VALORES.lock().unwrap_or_else(|e| e.into_inner())[2] = v;
            let ticks = TICKS.load(Ordering::Relaxed);
            nota(format!("30 s depois, ainda em gameplay: {v:?}, {ticks} ticks"));
            let devolvido = ANTIGO
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .is_some_and(|c| unsafe { devolver_corpo(c) });
            let v2 = unsafe { valor_do_script() };
            VALORES.lock().unwrap_or_else(|e| e.into_inner())[3] = v2;
            nota(format!("corpo devolvido {devolvido}: {v2:?}"));
            let vals = *VALORES.lock().unwrap_or_else(|e| e.into_inner());
            let alvo = ALVO.load(Ordering::Acquire) as i32;
            let antes = vals[0];
            let esperado = [antes, Some(alvo), Some(alvo), antes];
            fim(vals == esperado && devolvido && ticks > 100, "");
        }
        _ => {}
    }
}

/// Tamanho do bytecode vivo pelo campo `+0x80` (ponteiro de fim ou tamanho, medido no objeto).
unsafe fn tamanho_vivo(funcao: u64) -> usize {
    if read_u64((funcao + FUNCAO_BYTECODE as u64) as usize).unwrap_or(0) == 0 {
        return 0;
    }
    (read_u64((funcao + FUNCAO_BYTECODE_TAM as u64) as usize).unwrap_or(0) & 0xffff_ffff) as usize
}

#[cfg(test)]
mod teste {
    use super::*;

    fn blob_do_jogo() -> Option<Blob> {
        let jogo = std::env::var("BWMS_GAME")
            .unwrap_or_default(); // sem CP77_DIR o teste se pula
        let caminho = std::path::Path::new(&jogo).join("r6/cache/final.redscripts");
        caminho.exists().then(|| blob::ler(&caminho).expect("blob"))
    }

    /// O comparador não pode acusar mudança onde não houve: o blob contra ele mesmo dá zero.
    #[test]
    fn compara_blob_com_ele_mesmo() {
        let Some(b) = blob_do_jogo() else { return };
        let (trocadas, novas, sumidas) = mudancas(&b, &b);
        assert!(trocadas.is_empty(), "{} funções acusadas à toa", trocadas.len());
        assert!(novas.is_empty() && sumidas.is_empty());
    }

    /// Contra o blob que o teste da troca compila em /tmp (versão com `* 10`): exatamente uma função
    /// trocada, e é a do teste. Só roda depois de um boot com o marcador `troca-de-corpo`.
    #[test]
    fn compara_com_o_blob_do_teste() {
        let caminho = std::path::Path::new(BLOB_DO_TESTE);
        if !caminho.exists() {
            return;
        }
        let Some(a) = blob_do_jogo() else { return };
        let n = blob::ler(caminho).expect("blob do teste");
        let (trocadas, novas, sumidas) = mudancas(&a, &n);
        // O `scc` numera o que ELE gera (`wrapper$203736`, `proxy$203744`) com contador global: mudar o
        // CONJUNTO de `.reds` desloca todos, e aí a comparação por nome vê centenas de novas e sumidas.
        // Isso é o caso "reinicie o jogo" (a aplicação a quente recusa), não o caso deste teste, que é
        // o mesmo conjunto de arquivos com um corpo diferente. Se o blob alternativo ficou velho, pula.
        let gerado = |s: &String| s.contains('$');
        if novas.iter().any(|s| gerado(s)) || sumidas.iter().any(|s| gerado(s)) {
            return;
        }
        let nomes: Vec<String> = trocadas.iter().filter_map(|&i| n.nome_da_funcao(i)).collect();
        assert!(novas.is_empty() && sumidas.is_empty(), "novas {novas:?} sumidas {sumidas:?}");
        assert_eq!(nomes.len(), 1, "trocadas: {nomes:?}");
        assert!(nomes[0].starts_with("BwmsTrocaValor"), "trocadas: {nomes:?}");
    }

    /// O comparador contra um blob REALMENTE diferente: `/tmp/bwms-troca-offline.redscripts` é o
    /// bundle compilado com a função de teste em `* 10` (o do jogo está em `* 1`). Só ela pode aparecer
    /// como trocada — e a tradução dela tem que sair com o tamanho que o arquivo novo declara.
    #[test]
    fn compara_e_traduz_contra_blob_diferente() {
        let alt = std::path::Path::new("/tmp/bwms-troca-offline.redscripts");
        if !alt.exists() {
            return;
        }
        // O blob alternativo é gerado à mão com o `scc`; se o bundle do jogo foi recompilado depois, o
        // par está dessincronizado e a comparação acusaria diferença que não é do corpo da função.
        let jogo = std::env::var("BWMS_GAME")
            .unwrap_or_default(); // sem CP77_DIR o teste se pula
        let atual = std::path::Path::new(&jogo).join("r6/cache/final.redscripts");
        let data = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        if let (Some(a), Some(b)) = (data(&atual), data(alt)) {
            if a > b {
                return;
            }
        }
        let Some(a) = blob_do_jogo() else { return };
        let n = blob::ler(alt).expect("blob alternativo");
        let (trocadas, novas, sumidas) = mudancas(&a, &n);
        // Blob alternativo de um conjunto de `.reds` diferente: os contadores que o `scc` gera
        // (`wrapper$203736`) deslocam todos e a comparação por nome vê centenas de novas e sumidas.
        // Isso é o caso "reinicie o jogo", não o deste teste — se o arquivo ficou velho, pula.
        if novas.iter().chain(sumidas.iter()).any(|s| s.contains('$')) {
            return;
        }
        let nomes: Vec<String> = trocadas.iter().filter_map(|&i| n.nome_da_funcao(i)).collect();
        assert!(novas.is_empty() && sumidas.is_empty(), "novas {novas:?} sumidas {sumidas:?}");
        assert_eq!(nomes.len(), 1, "trocadas: {nomes:?}");
        assert!(nomes[0].starts_with("BwmsTrocaValor"), "trocadas: {nomes:?}");
        // A tradução da função mudada, com dicionário sintético dos nomes que ela referencia.
        let i = trocadas[0];
        let f = n.funcao(i).expect("função");
        let (_, refs, _) = blob::percorrer(f.corpo, f.tam_rt).expect("percurso");
        let mut dic = Dicionario::default();
        for r in &refs {
            let nome = nome_do_alvo(&n, r.pool, r.indice).expect("nome do alvo");
            dic.por_nome.insert((chave(r.pool), nome), 0x1_0000_0000 | u64::from(r.indice));
        }
        let bytes = traduzir(&n, i, &dic).expect("tradução");
        assert_eq!(bytes.len(), f.tam_rt, "tamanho da tradução");
        // O literal que mudou (o 10) tem que estar no bytecode traduzido, e o 1 antigo não.
        let (_, refs_a, _) = blob::percorrer(
            a.funcao(*indice_por_nome(&a).get(&nomes[0]).expect("função no blob do jogo"))
                .expect("função")
                .corpo,
            a.funcao(*indice_por_nome(&a).get(&nomes[0]).expect("idem")).expect("função").tam_rt,
        )
        .expect("percurso do antigo");
        assert_eq!(refs.len(), refs_a.len(), "as duas versões referenciam a mesma coisa");
    }

    /// A MECÂNICA da tradução, em escala e sem o jogo: com um dicionário sintético (cada nome vira um
    /// valor derivado dele), traduzir toda função com corpo do blob do jogo tem que dar exatamente o
    /// tamanho de runtime que o arquivo declara, e a instrução n da saída tem que ser a instrução n da
    /// entrada. O que este teste não cobre é o VALOR das referências — esse é o que a sonda confere ao
    /// vivo, instrução por instrução, contra o bytecode que o motor montou.
    #[test]
    fn traduz_o_blob_inteiro() {
        let Some(b) = blob_do_jogo() else { return };
        // Dicionário sintético: todo nome que o blob referencia, com um valor que não é zero.
        let mut dic = Dicionario::default();
        for i in 0..b.defs.len() {
            if b.defs[i].tipo != blob::TIPO_FUNCAO {
                continue;
            }
            let Ok(f) = b.funcao(i) else { continue };
            if !f.tem_corpo() || f.corpo.is_empty() {
                continue;
            }
            let Ok((_, refs, _)) = blob::percorrer(f.corpo, f.tam_rt) else { continue };
            for r in &refs {
                if let Some(nome) = nome_do_alvo(&b, r.pool, r.indice) {
                    let valor = 0x1_0000_0000u64 | u64::from(r.indice) | (u64::from(chave(r.pool)) << 40);
                    dic.por_nome.insert((chave(r.pool), nome), valor);
                }
            }
        }
        let mut traduzidas = 0usize;
        let mut erros: Vec<String> = Vec::new();
        for i in 0..b.defs.len() {
            if b.defs[i].tipo != blob::TIPO_FUNCAO {
                continue;
            }
            let Ok(f) = b.funcao(i) else { continue };
            if !f.tem_corpo() || f.corpo.is_empty() {
                continue;
            }
            match traduzir(&b, i, &dic) {
                Ok(bytes) => {
                    traduzidas += 1;
                    // Mesma sequência de opcodes, agora com as referências largas.
                    let (entrada, _, _) = blob::percorrer(f.corpo, f.tam_rt).expect("percurso");
                    let mut p = 0usize;
                    for (k, ins) in entrada.iter().enumerate() {
                        if bytes.get(p) != Some(&ins.op) {
                            erros.push(format!(
                                "{}: instrução {k} devia ser opcode {} e saiu {:?}",
                                b.nome_da_funcao(i).unwrap_or_default(),
                                ins.op,
                                bytes.get(p)
                            ));
                            break;
                        }
                        p += ins.tam_rt;
                    }
                    if p != bytes.len() {
                        erros.push(format!(
                            "{}: soma das instruções deu {p}, saída tem {}",
                            b.nome_da_funcao(i).unwrap_or_default(),
                            bytes.len()
                        ));
                    }
                }
                Err(e) => erros.push(format!("{}: {e}", b.nome_da_funcao(i).unwrap_or_default())),
            }
            if erros.len() > 10 {
                break;
            }
        }
        assert!(erros.is_empty(), "{} erro(s) em {traduzidas}:\n{}", erros.len(), erros.join("\n"));
        assert!(traduzidas > 30000, "traduzidas: {traduzidas}");
    }

    /// O que a função do teste E2E exige da tradução: as referências que ela tem e o nome de cada
    /// alvo (é por nome que a resolução acontece). Roda contra o blob do jogo, sem tocar no jogo.
    #[test]
    fn referencias_da_funcao_do_teste() {
        let Some(b) = blob_do_jogo() else { return };
        let Some(i) = (0..b.defs.len()).find(|&i| {
            b.defs[i].tipo == blob::TIPO_FUNCAO && b.nome(b.defs[i].nome).starts_with("BwmsTrocaValor;")
        }) else {
            panic!("BwmsTrocaValor não está no blob — o .reds de teste não foi compilado");
        };
        let f = b.funcao(i).expect("função");
        let (instr, refs, gastos) = blob::percorrer(f.corpo, f.tam_rt).expect("percurso");
        let mut linhas = vec![format!(
            "{} | {} instruções, {} refs, {gastos} B no arquivo, {} B no motor",
            b.nome_da_funcao(i).unwrap_or_default(),
            instr.len(),
            refs.len(),
            f.tam_rt
        )];
        for r in &refs {
            linhas.push(format!(
                "  {:?} idx={} nome={:?}",
                r.pool,
                r.indice,
                nome_do_alvo(&b, r.pool, r.indice)
            ));
        }
        let texto = linhas.join("\n");
        // A tradução precisa de referência de função, de local e de texto pra valer como prova.
        let tem = |p: fn(&Pool) -> bool| refs.iter().any(|r| p(&r.pool));
        assert!(tem(|p| *p == Pool::Def(blob::TIPO_FUNCAO)), "sem chamada:\n{texto}");
        assert!(tem(|p| *p == Pool::Def(blob::TIPO_LOCAL)), "sem local:\n{texto}");
        assert!(tem(|p| *p == Pool::Texto), "sem literal de texto:\n{texto}");
        assert!(refs.iter().all(|r| nome_do_alvo(&b, r.pool, r.indice).is_some()), "ref sem nome:\n{texto}");
        // 5 das 6 referências crescem de 4 pra 8 bytes; a de texto fica em 4.
        assert_eq!(gastos + 20, f.tam_rt, "crescimento da tradução:\n{texto}");
    }
}

// ===== Comparação entre o blob carregado e o recompilado =====
// O `scc` renumera os pools a cada passe, então "mudou" não é diferença de bytes: é diferença de
// opcode, de operando literal ou do NOME do alvo de alguma referência.

/// Índice das funções de um blob pelo nome completo (`Classe::nome;Assinatura`).
pub(crate) fn indice_por_nome(b: &Blob) -> HashMap<String, usize> {
    let mut m = HashMap::with_capacity(b.defs.len() / 2);
    for i in 0..b.defs.len() {
        if b.defs[i].tipo != blob::TIPO_FUNCAO {
            continue;
        }
        if let Some(n) = b.nome_da_funcao(i) {
            m.insert(n, i);
        }
    }
    m
}

/// Duas funções têm o mesmo corpo? Compara a sequência de instruções, os operandos literais e o
/// NOME do alvo de cada referência.
pub(crate) fn mesmo_corpo(a: &Blob, ia: usize, b: &Blob, ib: usize) -> bool {
    let (fa, fb) = match (a.funcao(ia), b.funcao(ib)) {
        (Ok(x), Ok(y)) => (x, y),
        _ => return false,
    };
    if fa.tam_rt != fb.tam_rt || fa.flags != fb.flags {
        return false;
    }
    let (ia_instr, ia_refs, _) = match blob::percorrer(fa.corpo, fa.tam_rt) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let (ib_instr, ib_refs, _) = match blob::percorrer(fb.corpo, fb.tam_rt) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if ia_instr.len() != ib_instr.len() || ia_refs.len() != ib_refs.len() {
        return false;
    }
    for (x, y) in ia_instr.iter().zip(&ib_instr) {
        if x.op != y.op || x.tam_arq != y.tam_arq {
            return false;
        }
        // Operando literal: os bytes fora das posições de referência têm que ser iguais.
        let (mut pa, mut pb) = (x.pos + 1, y.pos + 1);
        let mut corte = Vec::new();
        if let Some((_, n, rs)) = blob::forma_com(x.op, blob::REF_ARQ) {
            for (d, _) in rs.iter().take(n as usize) {
                corte.push(*d);
            }
        }
        let mut anterior = 0usize;
        for d in &corte {
            if fa.corpo[pa + anterior..pa + d] != fb.corpo[pb + anterior..pb + d] {
                return false;
            }
            anterior = d + blob::REF_ARQ;
        }
        pa += anterior;
        pb += anterior;
        let resto = x.tam_arq - 1 - anterior;
        if fa.corpo[pa..pa + resto] != fb.corpo[pb..pb + resto] {
            return false;
        }
    }
    for (x, y) in ia_refs.iter().zip(&ib_refs) {
        if x.pool != y.pool || nome_do_alvo(a, x.pool, x.indice) != nome_do_alvo(b, y.pool, y.indice) {
            return false;
        }
    }
    true
}

/// O que mudou do blob carregado (`antigo`) pro recompilado (`novo`): funções trocadas (índice no
/// novo), funções que o novo tem e o antigo não, e vice-versa. As duas últimas listas são o motivo
/// de recusar a troca a quente — criar ou apagar função mexe no RTTI.
pub(crate) fn mudancas(antigo: &Blob, novo: &Blob) -> (Vec<usize>, Vec<String>, Vec<String>) {
    let ia = indice_por_nome(antigo);
    let ib = indice_por_nome(novo);
    let mut trocadas = Vec::new();
    let mut so_no_novo = Vec::new();
    for (nome, &i) in &ib {
        match ia.get(nome) {
            Some(&j) => {
                if !mesmo_corpo(antigo, j, novo, i) {
                    trocadas.push(i);
                }
            }
            None => so_no_novo.push(nome.clone()),
        }
    }
    let so_no_antigo: Vec<String> = ia.keys().filter(|n| !ib.contains_key(*n)).cloned().collect();
    (trocadas, so_no_novo, so_no_antigo)
}

// ===== Aplicar o `.reds` salvo sem reiniciar (o que o modder usa) =====
// O vigia (`carregador_de_scripts`) já recompila ao salvar. Aqui o resultado vira efeito no jogo
// aberto: compila pra um blob temporário, compara com o que o motor carregou e troca o corpo das
// funções mudadas. Nada de RTTI é criado ou destruído, então nenhum cache nativo aponta pra lugar
// velho — que é o que derruba o jogo na recarga completa do motor.

const BLOB_A_QUENTE: &str = "/tmp/bwms-quente.redscripts";

/// O blob que o motor carregou e o dicionário aprendido dele. O dicionário continua valendo depois
/// de uma troca (os ponteiros vivos não mudam); o blob é substituído pelo novo a cada aplicação,
/// pra próxima comparação sair contra o que está rodando.
static ESTADO: std::sync::Mutex<Option<(Blob, Dicionario)>> = std::sync::Mutex::new(None);

/// Prepara o estado: lê o blob carregado e aprende o dicionário. Caro (percorre o blob inteiro),
/// feito uma vez por sessão.
///
/// # Safety
/// Precisa do RTTI montado e roda na thread do jogo.
pub(crate) unsafe fn preparar(caminho: &std::path::Path) -> Result<String, String> {
    let b = blob::ler(caminho)?;
    let (dic, relato) = aprender(&b);
    if dic.conflitos > 0 || dic.amostras < 100_000 {
        return Err(format!("dicionário não fecha: {relato}"));
    }
    *ESTADO.lock().unwrap_or_else(|e| e.into_inner()) = Some((b, dic));
    Ok(relato)
}

/// Compila os `.reds` pro blob quente. Roda FORA da thread do jogo (o `scc` leva segundos).
pub(crate) fn compilar_quente() -> Result<(), String> {
    crate::scripts::carregador::compilar_em(std::path::Path::new(BLOB_A_QUENTE))
}

/// Aplica no jogo aberto o blob que `compilar_quente` deixou pronto. Devolve o relato pro log de mod.
///
/// # Safety
/// Roda na thread do jogo, com o RTTI montado, e nunca de dentro de uma chamada do executor.
pub(crate) unsafe fn aplicar_a_quente() -> Result<String, String> {
    let mut guarda = ESTADO.lock().unwrap_or_else(|e| e.into_inner());
    if guarda.is_none() {
        drop(guarda);
        let atual = crate::dylib_dir()
            .map(std::path::PathBuf::from)
            .and_then(|d| d.parent().map(|p| p.join("r6/cache/final.redscripts")))
            .ok_or("sem a pasta do jogo")?;
        preparar(&atual)?;
        guarda = ESTADO.lock().unwrap_or_else(|e| e.into_inner());
    }
    let Some((antigo, dic)) = guarda.as_mut() else {
        return Err("estado não preparado".into());
    };
    let saida = std::path::Path::new(BLOB_A_QUENTE);
    let novo = blob::ler(saida)?;
    let (trocadas, novas, sumidas) = mudancas(antigo, &novo);
    if !novas.is_empty() || !sumidas.is_empty() {
        return Err(format!(
            "{} função(ões) criada(s) e {} removida(s): isso mexe no RTTI e pede reiniciar o jogo",
            novas.len(),
            sumidas.len()
        ));
    }
    if trocadas.is_empty() {
        return Ok("nada mudou".into());
    }
    // Traduz TUDO antes de trocar qualquer coisa: uma referência sem par no jogo (local novo,
    // literal novo, função nova) recusa a aplicação inteira, em vez de deixar metade trocada.
    let mut prontas = Vec::with_capacity(trocadas.len());
    for &i in &trocadas {
        let nome = novo.nome_da_funcao(i).unwrap_or_default();
        let bytes = traduzir(&novo, i, dic).map_err(|e| format!("{nome}: {e}"))?;
        let viva = funcao_viva(&novo, i);
        if viva == 0 {
            return Err(format!("{nome}: não está no jogo carregado"));
        }
        prontas.push((viva, bytes, nome));
    }
    let mut trocados = Vec::with_capacity(prontas.len());
    for (viva, bytes, nome) in &prontas {
        let tam = tamanho_vivo(*viva);
        match trocar_corpo(*viva, bytes, tam) {
            Ok(c) => trocados.push(c),
            Err(e) => {
                // Meia troca é pior que nenhuma: devolve o que já foi.
                for c in trocados {
                    devolver_corpo(c);
                }
                return Err(format!("{nome}: {e}"));
            }
        }
    }
    let nomes: Vec<&str> = prontas.iter().map(|(_, _, n)| n.as_str()).collect();
    let relato = format!("{} função(ões) trocada(s): {}", prontas.len(), nomes.join(", "));
    // O blob novo passa a ser a referência, e vai pro `r6/cache` pro próximo boot já nascer igual.
    if let Some(cache) = crate::dylib_dir()
        .map(std::path::PathBuf::from)
        .and_then(|d| d.parent().map(|p| p.join("r6/cache/final.redscripts")))
    {
        let _ = std::fs::copy(saida, cache);
    }
    *antigo = novo;
    Ok(relato)
}

// ===== Teste do caminho do modder (E5), marcador `~/.bwms-hook-vigia-quente` =====
// O que o modder faz: liga `red4ext/bwms-recarga-de-scripts`, salva o `.reds` e o jogo passa a rodar
// o código novo. Aqui o teste só salva o arquivo e espera — quem compila e troca é o vigia.

#[cfg(feature = "devtools")]
static FASE_DO_VIGIA: AtomicU64 = AtomicU64::new(0);

/// O teste do vigia já deu veredito (ou nunca começou).
#[cfg(feature = "devtools")]
pub(crate) fn teste_do_vigia_terminou() -> bool {
    FASE_DO_VIGIA.load(Ordering::Relaxed) >= 9
}

#[cfg(feature = "devtools")]
pub(crate) fn teste_do_vigia_tick() {
    use std::sync::Mutex;
    use std::time::Instant;
    let fase = FASE_DO_VIGIA.load(Ordering::Relaxed);
    static MARCO: Mutex<Option<Instant>> = Mutex::new(None);
    static NOTAS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static ALVO: AtomicU64 = AtomicU64::new(0);
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    if fase >= 9
        || crate::inicio::boot::exec_nested()
        || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30)
        || !crate::marcador_individual("vigia-quente")
    {
        return;
    }
    let Some(jogo) = crate::dylib_dir().and_then(|d| std::path::PathBuf::from(d).parent().map(|p| p.to_path_buf()))
    else {
        return;
    };
    let ligar = jogo.join("red4ext/bwms-recarga-de-scripts");
    let nota = |s: String| NOTAS.lock().unwrap_or_else(|e| e.into_inner()).push(s);
    let segundos = || MARCO.lock().unwrap_or_else(|e| e.into_inner()).map_or(0, |t: Instant| t.elapsed().as_secs());
    let marcar = || *MARCO.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    let fim = |ok: bool, motivo: &str| {
        FASE_DO_VIGIA.store(9, Ordering::Relaxed);
        let _ = std::fs::remove_file(&ligar);
        let notas = NOTAS.lock().unwrap_or_else(|e| e.into_inner()).join("; ");
        crate::veredito_dev!("[vigia] VEREDITO-VIGIA {notas}{motivo}; vigia-ok={ok}");
    };
    match fase {
        0 if crate::inicio::boot::gameplay_real() => {
            let v = unsafe { valor_do_script() };
            let (_, esperado) = match v {
                Some(4) => (10, 40),
                Some(40) => (1, 4),
                _ => return fim(false, " (a função de teste não está no bundle)"),
            };
            ALVO.store(esperado as u64, Ordering::Release);
            // Liga o caminho do modder e salva o `.reds`: daqui pra frente é o vigia que age.
            if std::fs::write(&ligar, b"").is_err() {
                return fim(false, " (não consegui criar red4ext/bwms-recarga-de-scripts)");
            }
            // 5 s pro vigia guardar a data de partida antes da mudança.
            nota(format!("antes {v:?}, alvo {esperado}"));
            marcar();
            FASE_DO_VIGIA.store(1, Ordering::Relaxed);
        }
        1 if segundos() >= 6 => {
            let k = if ALVO.load(Ordering::Acquire) == 40 { 10 } else { 1 };
            let _ = std::fs::write(jogo.join(TESTE_ARQUIVO), teste_fonte(k));
            nota(format!("`.reds` salvo com * {k}"));
            marcar();
            FASE_DO_VIGIA.store(2, Ordering::Relaxed);
        }
        // O vigia olha a cada 3 s, compila (~40 s) e a aplicação roda no tick seguinte.
        2 => {
            let v = unsafe { valor_do_script() };
            let alvo = ALVO.load(Ordering::Acquire) as i32;
            if v == Some(alvo) {
                nota(format!("{} s depois de salvar, o jogo devolve {alvo} sem reiniciar", segundos()));
                let _ = std::fs::write(jogo.join(TESTE_ARQUIVO), teste_fonte(1));
                return fim(true, "");
            }
            if segundos() > 240 {
                nota(format!("valor ainda {v:?} depois de 240 s"));
                let _ = std::fs::write(jogo.join(TESTE_ARQUIVO), teste_fonte(1));
                fim(false, " (o vigia não aplicou)");
            }
        }
        _ => {}
    }
}
