//! Representação canônica de um tweak: a lista ordenada de [`Op`] que os dois formatos de autoria produzem
//! (`.yaml`/`.toml` por `tweaks::interpret_from*`, `.tweak` por `tweaks::tweak_source::parse`) e que os dois aplicadores
//! consomem (o runtime no TweakDB vivo, `cp77-console/src/tweakdb/mod.rs::aplicar_conjunto`; o `bwms tweak` no
//! `tweakdb.bin`, `writer::apply_conjunto`). O formato fica no adaptador (`tweaks`): quem aplica não sabe de
//! que formato a operação veio.
//!
//! Os arquivos não se aplicam um por vez: viram um [`Conjunto`]. Um `!append` de um arquivo sobrevive à troca inteira do mesmo array em
//! outro arquivo, em qualquer ordem.

use std::collections::{HashMap, HashSet, VecDeque};

/// Uma operação de alto nível extraída do documento.
pub enum Op {
    /// `$base`: clonar `base` em `record`.
    Clone { record: String, base: String },
    /// `$type`: criar `record` da classe `class` (offline = clona uma amostra).
    Create { record: String, class: String },
    /// Editar um flat que já existe (prop de record: prop desconhecida é recusada sem `$props: AutoFlats`).
    Edit { flat: String, op: EditOp },
    /// Editar um flat que nasce com o tipo `tipo` se não existir (nome do tipo de flat do motor: `Int32`, `array:Float`,
    /// `raRef:CResource`). O flat é criado quando o arquivo declara o tipo (`$type` com `$value`, o tipo do `.tweak`) ou quando o tipo
    /// sai do valor (`$props: AutoFlats`, flat solto no topo do arquivo).
    Criar { flat: String, tipo: String, op: EditOp },
}

/// Operação de edição de um flat. As variantes de array espelham as tags do
/// formato (`!append`, `!prepend`, `!append-once`, `!prepend-once`, `!remove`). Mora aqui (não no
/// `bwms/src/tweak/writer.rs`) porque é o tipo de INTENÇÃO (Op::Edit) — desacoplado do writer offline,
/// o runtime usa o parser SEM arrastar `writer`/`tweakdb`/`kraken` do `bwms tweak`.
pub enum EditOp {
    /// `= valor` (escalar) ou `= [a, b, c]` (array inteiro, no texto de [`lista_em_texto`]).
    Assign(String),
    /// `+= valor` / `!append` — adiciona um elemento ao FIM do array.
    Append(String),
    /// `!append-once` — adiciona ao fim só se ainda não estiver presente.
    AppendOnce(String),
    /// `!prepend` — adiciona um elemento ao INÍCIO do array.
    Prepend(String),
    /// `!prepend-once` — adiciona ao início só se ainda não estiver presente.
    PrependOnce(String),
    /// `-= valor` / `!remove` — remove os elementos iguais.
    Remove(String),
    /// `!remove-all` — limpa o array INTEIRO, incondicionalmente (SEM valor nenhum — distinto de
    /// `!remove`, que só tira elementos IGUAIS a um valor dado).
    RemoveAll,
    /// `!append-from` / `!merge` — anexa ao FIM os elementos de outro flat array
    /// (pelo nome) que o alvo ainda não tem. `!merge` e `!append-from` são o mesmo op.
    AppendFrom(String),
    /// `!prepend-from` — anexa ao INÍCIO os elementos de outro flat array que o alvo ainda não tem.
    PrependFrom(String),
}

/// Precisa de aspas no texto de lista: o elemento vazio, com separador, colchete, aspas, barra ou espaço nas pontas.
fn pede_aspas(e: &str) -> bool {
    e.is_empty() || e.trim() != e || e.contains([',', '[', ']', '"', '\\'])
}

/// O texto `[a, b]` de um array inteiro no `EditOp::Assign`. Elemento que tem vírgula (struct `1,2,3`, texto com vírgula) vai entre aspas
/// duplas com `"` e `\` escapados: o array volta inteiro por [`texto_em_lista`].
pub fn lista_em_texto(itens: &[String]) -> String {
    let partes: Vec<String> = itens
        .iter()
        .map(|e| if pede_aspas(e) { format!("\"{}\"", e.replace('\\', "\\\\").replace('"', "\\\"")) } else { e.clone() })
        .collect();
    format!("[{}]", partes.join(", "))
}

/// Os elementos de um texto `[a, b]`; `None` se o texto não é lista. Elemento sem aspas vem aparado e o vazio sem aspas não conta (`[a, ]`
/// tem 1); entre aspas, vem como escrito, com os escapes desfeitos.
pub fn texto_em_lista(texto: &str) -> Option<Vec<String>> {
    let corpo = texto.trim().strip_prefix('[')?.strip_suffix(']')?;
    let mut itens = Vec::new();
    let mut atual = String::new();
    let mut entre_aspas = false;
    let mut teve_aspas = false;
    let mut chars = corpo.chars();
    let fechar = |atual: &mut String, teve_aspas: &mut bool, itens: &mut Vec<String>| {
        let e = if *teve_aspas { std::mem::take(atual) } else { std::mem::take(atual).trim().to_string() };
        if *teve_aspas || !e.is_empty() {
            itens.push(e);
        }
        *teve_aspas = false;
    };
    while let Some(c) = chars.next() {
        match c {
            '\\' if entre_aspas => atual.extend(chars.next()),
            '"' if entre_aspas => entre_aspas = false,
            '"' if atual.trim().is_empty() => {
                atual.clear();
                entre_aspas = true;
                teve_aspas = true;
            }
            ',' if !entre_aspas => fechar(&mut atual, &mut teve_aspas, &mut itens),
            _ if teve_aspas && !entre_aspas => {}
            _ => atual.push(c),
        }
    }
    fechar(&mut atual, &mut teve_aspas, &mut itens);
    Some(itens)
}

/// O record dono de um flat (`Items.X.tags` → `Items.X`) e o apêndice da prop (`.tags`).
pub fn record_e_apendice(flat: &str) -> Option<(&str, &str)> {
    flat.rfind('.').map(|i| (&flat[..i], &flat[i..]))
}

/// A troca de valor de um flat no conjunto: o texto do valor e o tipo com que o flat nasce se não existir.
#[derive(Clone, Debug, PartialEq)]
pub struct Atribuicao {
    pub flat: String,
    pub tipo: Option<String>,
    pub valor: String,
}

/// As mutações de um array no conjunto, juntas de todos os arquivos.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mutacao {
    pub flat: String,
    /// Tipo com que o flat nasce se não existir.
    pub tipo: Option<String>,
    pub remover_tudo: bool,
    pub remocoes: Vec<String>,
    /// `(valor, só se ausente)`, na ordem dos arquivos.
    pub antepor: Vec<(String, bool)>,
    pub antepor_de: Vec<String>,
    pub anexar: Vec<(String, bool)>,
    pub anexar_de: Vec<String>,
    /// A mutação que vem antes desta na cadeia: a do mesmo array no record de quem este herdou.
    pub base: Option<String>,
}

/// Os arquivos de tweak juntos.
#[derive(Default)]
pub struct Conjunto {
    /// `Clone`/`Create`, na ordem dos arquivos.
    pub registros: Vec<Op>,
    /// Uma por flat, com o último valor; a ordem é a da 1ª atribuição do flat.
    pub atribuicoes: Vec<Atribuicao>,
    /// Uma por flat, na ordem da 1ª mutação do flat.
    pub mutacoes: Vec<Mutacao>,
}

/// O que a re-herança pergunta do TweakDB antes de qualquer gravação do conjunto (o runtime responde pelo TweakDB vivo, o `bwms tweak`
/// pelo `tweakdb.bin`).
pub trait ConsultaDeHeranca {
    fn record_existe(&self, record: &str) -> bool;
    fn flat_existe(&self, flat: &str) -> bool;
    /// Os dois flats apontam pro mesmo valor (o mesmo `FlatValue` no runtime, o mesmo índice de valor no `tweakdb.bin`).
    fn mesmo_valor(&self, a: &str, b: &str) -> bool;
    fn e_array(&self, flat: &str) -> bool;
    /// O array `descendente` começa com os elementos do array `base`.
    fn comeca_com(&self, base: &str, descendente: &str) -> bool;
}

/// O mapa de herança dos records do jogo: record → os que herdam dele diretamente (sai das fontes
/// `.tweak` do jogo por `bwms tweak heranca`). Texto: `Record:` e embaixo `- Filho`, uma chave por record.
#[derive(Default, Debug, PartialEq)]
pub struct MapaDeHeranca {
    filhos: HashMap<String, Vec<String>>,
}

impl MapaDeHeranca {
    pub fn ler(texto: &str) -> Result<Self, String> {
        let raiz = crate::tweaks::yaml::parse(texto).map_err(|e| format!("mapa de herança: {e}"))?;
        let entradas = raiz
            .as_map()
            .ok_or("mapa de herança: a raiz precisa ser `Record:` com os filhos embaixo em `- Filho`; regere o arquivo com `bwms tweak heranca`")?;
        let mut mapa = MapaDeHeranca::default();
        for (base, valor) in entradas {
            let itens = valor
                .as_seq()
                .ok_or_else(|| format!("mapa de herança: '{base}' precisa de uma lista `- Filho` (ou `[]`); regere o arquivo com `bwms tweak heranca`"))?;
            let filhos = mapa.filhos.entry(base.clone()).or_default();
            for item in itens {
                let filho = item
                    .as_str()
                    .ok_or_else(|| format!("mapa de herança: filho de '{base}' precisa ser um nome de record; regere o arquivo com `bwms tweak heranca`"))?;
                filhos.push(filho.to_string());
            }
        }
        Ok(mapa)
    }

    /// De pares `(filho, base)`.
    pub fn de_pares(pares: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut mapa = MapaDeHeranca::default();
        let mut vistos: HashSet<(String, String)> = HashSet::new();
        for (filho, base) in pares {
            if vistos.insert((filho.clone(), base.clone())) {
                mapa.filhos.entry(base).or_default().push(filho);
            }
        }
        mapa
    }

    /// O mapa das fontes `.tweak` do jogo (`tweak_source::grupos_da_fonte` de cada arquivo).
    pub fn das_fontes(fontes: &[crate::tweaks::tweak_source::FonteDeGrupos]) -> Self {
        // Pacote de schema e de consulta descreve tipo, não record: grupo dele não é record nem herda.
        let pacote_de_records = |p: &Option<String>| !matches!(p.as_deref(), Some("RTDB" | "Query"));
        let nome_completo = |p: &Option<String>, curto: &str| match p {
            Some(p) => format!("{p}.{curto}"),
            None => curto.to_string(),
        };
        let mut e_record: HashMap<String, bool> = HashMap::new();
        let mut curtos_por_pacote: HashSet<(Option<String>, &str)> = HashSet::new();
        for fonte in fontes {
            for grupo in &fonte.grupos {
                e_record.insert(nome_completo(&fonte.pacote, &grupo.nome), pacote_de_records(&fonte.pacote) && !grupo.debug);
                curtos_por_pacote.insert((fonte.pacote.clone(), grupo.nome.as_str()));
            }
        }
        let mut pares = Vec::new();
        for fonte in fontes.iter().filter(|f| pacote_de_records(&f.pacote)) {
            for grupo in fonte.grupos.iter().filter(|g| !g.debug) {
                let Some(b) = grupo.base.as_deref() else { continue };
                let resolvida = if e_record.contains_key(b) {
                    Some(b.to_string())
                } else {
                    // O 1º pacote que tem o nome curto decide: se o grupo dele não for record, o seguinte não entra no lugar.
                    std::iter::once(fonte.pacote.clone())
                        .chain(fonte.usings.iter().map(|u| Some(u.clone())))
                        .find(|p| curtos_por_pacote.contains(&(p.clone(), b)))
                        .map(|p| nome_completo(&p, b))
                };
                if let Some(base) = resolvida.filter(|r| e_record.get(r) == Some(&true)) {
                    pares.push((nome_completo(&fonte.pacote, &grupo.nome), base));
                }
            }
        }
        MapaDeHeranca::de_pares(pares)
    }

    /// Fica só com os pares em que o record e a base passam em `existe` (os records do `tweakdb.bin` instalado).
    pub fn filtrar(&mut self, existe: impl Fn(&str) -> bool) {
        self.filhos.retain(|base, filhos| {
            if !existe(base) {
                return false;
            }
            filhos.retain(|f| existe(f));
            !filhos.is_empty()
        });
    }

    /// Pares record → base no mapa.
    pub fn pares(&self) -> usize {
        self.filhos.values().map(Vec::len).sum()
    }

    /// O texto que [`MapaDeHeranca::ler`] lê, com records e filhos em ordem alfabética (arquivo reprodutível).
    pub fn escrever(&self) -> String {
        let mut bases: Vec<&String> = self.filhos.keys().collect();
        bases.sort();
        let mut texto = String::new();
        for base in bases {
            texto.push_str(base);
            texto.push_str(":\n");
            let mut filhos: Vec<&String> = self.filhos[base].iter().collect();
            filhos.sort();
            for filho in filhos {
                texto.push_str("- ");
                texto.push_str(filho);
                texto.push('\n');
            }
        }
        texto
    }

    pub fn filhos(&self, record: &str) -> &[String] {
        self.filhos.get(record).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn e_base(&self, record: &str) -> bool {
        self.filhos.contains_key(record)
    }

    pub fn bases(&self) -> usize {
        self.filhos.len()
    }
}

/// A mutação de `flat`, criada vazia (sem tipo, sem base) se o conjunto ainda não tem; `indice` acompanha `mutacoes` (flat → posição).
fn mutacao_do_flat<'a>(mutacoes: &'a mut Vec<Mutacao>, indice: &mut HashMap<String, usize>, flat: &str) -> &'a mut Mutacao {
    let i = *indice.entry(flat.to_string()).or_insert_with(|| {
        mutacoes.push(Mutacao { flat: flat.to_string(), ..Mutacao::default() });
        mutacoes.len() - 1
    });
    &mut mutacoes[i]
}

fn indice_por_flat<'a>(flats: impl Iterator<Item = &'a String>) -> HashMap<String, usize> {
    flats.enumerate().map(|(i, f)| (f.clone(), i)).collect()
}

impl Conjunto {
    /// Junta as operações dos arquivos, na ordem em que vêm.
    pub fn montar(arquivos: impl IntoIterator<Item = Vec<Op>>) -> Conjunto {
        let mut c = Conjunto::default();
        let mut indice_atrib: HashMap<String, usize> = HashMap::new();
        let mut indice_mut: HashMap<String, usize> = HashMap::new();
        for op in arquivos.into_iter().flatten() {
            let (flat, tipo, edicao) = match op {
                Op::Edit { flat, op } => (flat, None, op),
                Op::Criar { flat, tipo, op } => (flat, Some(tipo), op),
                registro => {
                    c.registros.push(registro);
                    continue;
                }
            };
            if let EditOp::Assign(valor) = edicao {
                match indice_atrib.get(&flat) {
                    Some(&i) => {
                        let a = &mut c.atribuicoes[i];
                        a.valor = valor;
                        // `Edit` depois de `Criar` não desfaz o tipo com que o flat nasce.
                        if tipo.is_some() {
                            a.tipo = tipo;
                        }
                    }
                    None => {
                        indice_atrib.insert(flat.clone(), c.atribuicoes.len());
                        c.atribuicoes.push(Atribuicao { flat, tipo, valor });
                    }
                }
                continue;
            }
            let m = mutacao_do_flat(&mut c.mutacoes, &mut indice_mut, &flat);
            if tipo.is_some() {
                m.tipo = tipo;
            }
            match edicao {
                EditOp::Append(v) => m.anexar.push((v, false)),
                EditOp::AppendOnce(v) => m.anexar.push((v, true)),
                EditOp::Prepend(v) => m.antepor.push((v, false)),
                EditOp::PrependOnce(v) => m.antepor.push((v, true)),
                EditOp::Remove(v) => m.remocoes.push(v),
                EditOp::RemoveAll => m.remover_tudo = true,
                EditOp::AppendFrom(f) => m.anexar_de.push(f),
                EditOp::PrependFrom(f) => m.antepor_de.push(f),
                EditOp::Assign(_) => {}
            }
        }
        c
    }

    pub fn atribuicao(&self, flat: &str) -> Option<&Atribuicao> {
        self.atribuicoes.iter().find(|a| a.flat == flat)
    }

    pub fn mutacao(&self, flat: &str) -> Option<&Mutacao> {
        self.mutacoes.iter().find(|m| m.flat == flat)
    }

    /// A cadeia de mutações de `flat`: da do record mais antigo da herança à do próprio flat (a ordem de aplicação).
    pub fn cadeia(&self, flat: &str) -> Vec<&Mutacao> {
        let mut cadeia: Vec<&Mutacao> = self.mutacao(flat).into_iter().collect();
        while let Some(base) = cadeia.last().and_then(|m| m.base.as_deref()) {
            // Ciclo de `base` termina no 1º flat repetido.
            if cadeia.iter().any(|m| m.flat == base) {
                break;
            }
            let Some(anterior) = self.mutacao(base) else { break };
            cadeia.push(anterior);
        }
        cadeia.reverse();
        cadeia
    }

    /// Record novo de `$base`: a mutação de um array do record-base vale no clone que não reatribui esse array.
    pub fn herdar_mutacoes_dos_clones(&mut self) {
        let clones: Vec<(String, String)> = self
            .registros
            .iter()
            .filter_map(|op| match op {
                Op::Clone { record, base } => Some((record.clone(), base.clone())),
                _ => None,
            })
            .collect();
        let atribuidos: HashSet<&str> = self.atribuicoes.iter().map(|a| a.flat.as_str()).collect();
        let mut indice = indice_por_flat(self.mutacoes.iter().map(|m| &m.flat));
        // Clone de clone declarado antes do clone do qual herda só recebe a mutação na passada seguinte: repete até nada mudar. Cada
        // passada leva a mutação ao menos um degrau, então `clones + 1` passadas cobrem a cadeia mais longa (e param o clone que se aninha
        // no próprio base, `A.b <- A`, que geraria flat novo para sempre).
        for _ in 0..=clones.len() {
            let antes: Vec<Option<String>> = self.mutacoes.iter().map(|m| m.base.clone()).collect();
            for (record, base) in &clones {
                let prefixo = format!("{base}.");
                let alvos: Vec<(String, String)> = self
                    .mutacoes
                    .iter()
                    .filter_map(|m| m.flat.strip_prefix(&prefixo).map(|resto| (m.flat.clone(), format!("{record}.{resto}"))))
                    .collect();
                for (fonte, alvo) in alvos {
                    if alvo != fonte && !atribuidos.contains(alvo.as_str()) {
                        mutacao_do_flat(&mut self.mutacoes, &mut indice, &alvo).base = Some(fonte);
                    }
                }
            }
            if self.mutacoes.iter().map(|m| &m.base).eq(antes.iter()) {
                break;
            }
        }
    }

    /// Leva a mudança de um record do jogo aos records do jogo que herdam dele (pelo `mapa`), com as perguntas respondidas por `tdb` sobre o
    /// TweakDB de antes do conjunto. Devolve quantos flats de descendentes entraram.
    pub fn herdar_do_mapa(&mut self, mapa: &MapaDeHeranca, tdb: &impl ConsultaDeHeranca) -> usize {
        if mapa.bases() == 0 {
            return 0;
        }
        let mut indice_atrib = indice_por_flat(self.atribuicoes.iter().map(|a| &a.flat));
        let mut indice_mut = indice_por_flat(self.mutacoes.iter().map(|m| &m.flat));
        // Mutação que veio dos arquivos (não a que a conversão ou a herança criam aqui).
        let mutados_pelos_arquivos: HashSet<String> = indice_mut.keys().cloned().collect();
        let mut fila: VecDeque<String> = VecDeque::new();
        let mut visto: HashSet<String> = HashSet::new();
        for flat in self.atribuicoes.iter().map(|a| &a.flat).chain(self.mutacoes.iter().map(|m| &m.flat)) {
            if record_e_apendice(flat).is_some_and(|(r, _)| mapa.e_base(r)) && visto.insert(flat.clone()) {
                fila.push_back(flat.clone());
            }
        }
        let mut convertidos: HashSet<String> = HashSet::new();
        let mut entraram = 0;
        while let Some(f) = fila.pop_front() {
            let Some((record, sufixo)) = record_e_apendice(&f) else { continue };
            if !tdb.record_existe(record) || !tdb.flat_existe(&f) {
                continue;
            }
            let atribuicao_de_f = indice_atrib.get(&f).map(|&i| self.atribuicoes[i].clone());
            for filho in mapa.filhos(record) {
                if !tdb.record_existe(filho) {
                    continue;
                }
                let t = format!("{filho}{sufixo}");
                if indice_atrib.contains_key(&t) || !tdb.flat_existe(&t) {
                    continue;
                }
                let entrou = match &atribuicao_de_f {
                    Some(a) => {
                        // Troca inteira + mutação de outro arquivo no mesmo array: a cópia do valor perderia a mutação no filho de mesmo
                        // valor, então ele também vai pela cadeia.
                        let pela_cadeia = mutados_pelos_arquivos.contains(&f) && tdb.e_array(&f);
                        if tdb.mesmo_valor(&t, &f) && !pela_cadeia {
                            indice_atrib.insert(t.clone(), self.atribuicoes.len());
                            self.atribuicoes.push(Atribuicao { flat: t.clone(), tipo: a.tipo.clone(), valor: a.valor.clone() });
                            true
                        } else if tdb.e_array(&f) {
                            // O filho com array próprio fica com os elementos dele: o base vira "tira os originais, antepõe os novos".
                            if convertidos.insert(f.clone()) {
                                let m = mutacao_do_flat(&mut self.mutacoes, &mut indice_mut, &f);
                                m.remover_tudo = true;
                                m.antepor.extend(texto_em_lista(&a.valor).unwrap_or_default().into_iter().map(|e| (e, false)));
                            }
                            mutacao_do_flat(&mut self.mutacoes, &mut indice_mut, &t).base = Some(f.clone());
                            true
                        } else {
                            false
                        }
                    }
                    None if indice_mut.contains_key(&f) && (tdb.mesmo_valor(&t, &f) || tdb.comeca_com(&f, &t)) => {
                        mutacao_do_flat(&mut self.mutacoes, &mut indice_mut, &t).base = Some(f.clone());
                        true
                    }
                    None => false,
                };
                if entrou {
                    entraram += 1;
                    if mapa.e_base(filho) && visto.insert(t.clone()) {
                        fila.push_back(t);
                    }
                }
            }
        }
        self.atribuicoes.retain(|a| !convertidos.contains(&a.flat));
        entraram
    }
}

/// Um nível da cadeia de mutação já em valores de elemento: o `!remove-all` vira a lista dos elementos do próprio flat, os `!*-from`, os
/// elementos do flat-fonte.
pub struct Nivel<T> {
    pub remocoes: Vec<T>,
    pub antepor: Vec<(T, bool)>,
    pub antepor_de: Vec<Vec<T>>,
    pub anexar: Vec<(T, bool)>,
    pub anexar_de: Vec<Vec<T>>,
}

/// Aplica a cadeia ao array `alvo`. `igual` compara dois elementos como o tipo deles.
pub fn mutar<T: Clone>(alvo: Vec<T>, cadeia: &[Nivel<T>], igual: impl Fn(&T, &T) -> bool) -> Vec<T> {
    let removido_em = |niveis: &[Nivel<T>], v: &T| niveis.iter().any(|n| n.remocoes.iter().any(|r| igual(r, v)));
    let tem = |lista: &[T], v: &T| lista.iter().any(|e| igual(e, v));
    let mut meio: Vec<T> = alvo.into_iter().filter(|e| !removido_em(cadeia, e)).collect();

    // O que um descendente removeu não volta pela mutação de um ancestral; a remoção do próprio nível não impede.
    let mut inicio: Vec<T> = Vec::new();
    for (l, nivel) in cadeia.iter().enumerate() {
        let depois = &cadeia[l + 1..];
        for (v, so_ausente) in &nivel.antepor {
            if removido_em(depois, v) || (*so_ausente && (tem(&inicio, v) || tem(&meio, v))) {
                continue;
            }
            inicio.push(v.clone());
        }
        for v in nivel.antepor_de.iter().flatten() {
            if removido_em(depois, v) || tem(&inicio, v) || tem(&meio, v) {
                continue;
            }
            inicio.push(v.clone());
        }
    }
    inicio.append(&mut meio);
    let mut array = inicio;

    for (l, nivel) in cadeia.iter().enumerate() {
        let depois = &cadeia[l + 1..];
        for (v, so_ausente) in &nivel.anexar {
            if removido_em(depois, v) || (*so_ausente && tem(&array, v)) {
                continue;
            }
            array.push(v.clone());
        }
        for v in nivel.anexar_de.iter().flatten() {
            if removido_em(depois, v) || tem(&array, v) {
                continue;
            }
            array.push(v.clone());
        }
    }
    array
}

#[cfg(test)]
mod tests {
    use super::{lista_em_texto, mutar, texto_em_lista, Conjunto, ConsultaDeHeranca, EditOp, MapaDeHeranca, Nivel, Op};
    use crate::tweaks::{interpret_from, template, tweak_source, yaml};
    use std::collections::HashMap;

    fn resumo(ops: &[Op]) -> Vec<String> {
        ops.iter()
            .map(|o| match o {
                Op::Clone { record, base } => format!("clone {record} <- {base}"),
                Op::Edit { flat, op: EditOp::Assign(v) } | Op::Criar { flat, op: EditOp::Assign(v), .. } => format!("{flat} = {v}"),
                Op::Edit { flat, op: EditOp::Append(v) } | Op::Criar { flat, op: EditOp::Append(v), .. } => format!("{flat} += {v}"),
                _ => "fora deste teste".to_string(),
            })
            .collect()
    }

    /// O mesmo tweak escrito nos dois formatos dá a mesma lista de operações: é o que deixa um aplicador só por destino.
    #[test]
    fn yaml_e_tweak_dao_as_mesmas_operacoes() {
        let raiz = yaml::parse("Items.Meu:\n  $base: Items.Katana\n  damage: 10\n  tags:\n    - !append Cool\n").expect("yaml");
        let do_yaml = interpret_from(&template::expand(&raiz).expect("expand"), "meu.yaml").expect("interpret");
        let do_tweak = tweak_source::parse("Items.Meu : Items.Katana {\n  int damage = 10;\n  tags += [\"Cool\"];\n}\n").expect("tweak");
        let esperado = vec!["clone Items.Meu <- Items.Katana", "Items.Meu.damage = 10", "Items.Meu.tags += Cool"];
        assert_eq!(resumo(&do_yaml), esperado);
        assert_eq!(resumo(&do_tweak), esperado);
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn lista_volta_inteira_com_virgula_e_aspas_no_elemento() {
        let itens = s(&["Items.A", "1,2,3", "diz \"oi\"", "", " x ", "a\\b"]);
        let texto = lista_em_texto(&itens);
        assert_eq!(texto_em_lista(&texto), Some(itens));
        assert_eq!(texto_em_lista("[A, B]"), Some(s(&["A", "B"])));
        assert_eq!(texto_em_lista("[]"), Some(Vec::new()));
        assert_eq!(texto_em_lista("[a, ]"), Some(s(&["a"])));
        assert_eq!(texto_em_lista("solto"), None);
    }

    fn arquivo(src: &str, nome: &str) -> Vec<Op> {
        let raiz = yaml::parse(src).expect("yaml");
        interpret_from(&template::expand(&raiz).expect("expand"), nome).expect("interpret")
    }

    fn igual(a: &String, b: &String) -> bool {
        a == b
    }

    /// Diferença 5: o `!append` de um arquivo anterior sobrevive à troca inteira do mesmo array num arquivo posterior (e vice-versa), porque o
    /// conjunto aplica as mutações por cima do valor atribuído.
    #[test]
    fn append_de_um_arquivo_sobrevive_a_troca_inteira_em_outro() {
        let a = arquivo("Items.X:\n  tags:\n    - !append Novo\n", "a.yaml");
        let b = arquivo("Items.X:\n  tags: [Um, Dois]\n", "b.yaml");
        for c in [Conjunto::montar([a, b]), Conjunto::montar([arquivo("Items.X:\n  tags: [Um, Dois]\n", "b.yaml"), arquivo("Items.X:\n  tags:\n    - !append Novo\n", "a.yaml")])] {
            let atrib = c.atribuicao("Items.X.tags").expect("atribuição");
            let base = texto_em_lista(&atrib.valor).unwrap();
            let cadeia = c.cadeia("Items.X.tags");
            let niveis: Vec<Nivel<String>> = cadeia
                .iter()
                .map(|m| Nivel { remocoes: m.remocoes.clone(), antepor: m.antepor.clone(), antepor_de: vec![], anexar: m.anexar.clone(), anexar_de: vec![] })
                .collect();
            assert_eq!(mutar(base, &niveis, igual), s(&["Um", "Dois", "Novo"]));
        }
    }

    #[test]
    fn no_mesmo_flat_vale_a_ultima_atribuicao_e_as_mutacoes_somam() {
        let c = Conjunto::montar([
            arquivo("Items.X:\n  quality: Quality.Rare\n  tags:\n    - !prepend P1\n    - !remove Velho\n", "a.yaml"),
            arquivo("Items.X:\n  quality: Quality.Epic\n  tags:\n    - !prepend P2\n    - !append-once Um\n", "b.yaml"),
        ]);
        assert_eq!(c.atribuicoes.len(), 1);
        assert_eq!(c.atribuicao("Items.X.quality").unwrap().valor, "Quality.Epic");
        let m = c.mutacao("Items.X.tags").unwrap();
        let n = Nivel { remocoes: m.remocoes.clone(), antepor: m.antepor.clone(), antepor_de: vec![], anexar: m.anexar.clone(), anexar_de: vec![] };
        // Os anteposições ficam na ordem dos arquivos (o índice de inserção anda), não invertidos.
        assert_eq!(mutar(s(&["Um", "Velho"]), &[n], igual), s(&["P1", "P2", "Um"]));
    }

    #[test]
    fn mutar_pula_o_que_ja_tem_no_merge_e_o_removido_pelo_descendente() {
        let base = Nivel { remocoes: vec![], antepor: vec![], antepor_de: vec![], anexar: vec![("X".to_string(), false)], anexar_de: vec![s(&["A", "Z"])] };
        let desc = Nivel { remocoes: s(&["X"]), antepor: vec![], antepor_de: vec![], anexar: vec![], anexar_de: vec![] };
        assert_eq!(mutar(s(&["A"]), &[base, desc], igual), s(&["A", "Z"]));
        let tudo = Nivel { remocoes: s(&["A", "B", "A"]), antepor: vec![("N".to_string(), false)], antepor_de: vec![], anexar: vec![], anexar_de: vec![] };
        assert_eq!(mutar(s(&["A", "B", "A"]), &[tudo], igual), s(&["N"]), "!remove-all tira todos, inclusive o repetido");
    }

    #[test]
    fn clone_herda_a_mutacao_do_base_no_conjunto() {
        let mut c = Conjunto::montar([
            arquivo("Items.Novo:\n  $base: Items.Base\n  tags:\n    - !append DoNovo\n", "a.yaml"),
            arquivo("Items.Base:\n  tags:\n    - !append DoBase\n", "b.yaml"),
        ]);
        c.herdar_mutacoes_dos_clones();
        let cadeia: Vec<&str> = c.cadeia("Items.Novo.tags").iter().map(|m| m.flat.as_str()).collect();
        assert_eq!(cadeia, ["Items.Base.tags", "Items.Novo.tags"]);
    }

    /// TweakDB de teste pra re-herança: valor de cada flat por nome.
    struct Tdb(HashMap<&'static str, Vec<&'static str>>);
    impl ConsultaDeHeranca for Tdb {
        fn record_existe(&self, r: &str) -> bool {
            self.0.keys().any(|f| f.starts_with(&format!("{r}.")))
        }
        fn flat_existe(&self, f: &str) -> bool {
            self.0.contains_key(f)
        }
        fn mesmo_valor(&self, a: &str, b: &str) -> bool {
            self.0.get(a) == self.0.get(b)
        }
        fn e_array(&self, f: &str) -> bool {
            self.0.get(f).is_some_and(|v| v.len() != 1)
        }
        fn comeca_com(&self, b: &str, d: &str) -> bool {
            matches!((self.0.get(b), self.0.get(d)), (Some(b), Some(d)) if d.starts_with(b))
        }
    }

    /// O mapa sai das fontes `.tweak` do jogo: nome pelo `package`, base pelo pacote do arquivo e pelos `using`, sem schema nem `[Debug]`.
    #[test]
    fn mapa_das_fontes_tweak() {
        use crate::tweaks::tweak_source::grupos_da_fonte;
        let schema = grupos_da_fonte("package RTDB\nItem { String name; }\n").unwrap();
        let itens = grupos_da_fonte(
            "package Items\nusing RTDB, Base\n\n// comentário { com chave }\nCraftable_Uncommon_X : Item {\n  CName q = \"Quality.Uncommon\";\n  stats = [ { value = 1; } : Stat ];\n}\nCraftable_Rare_X : Craftable_Uncommon_X { q = \"}\"; }\n[ Debug ]\nTeste_X : Craftable_Uncommon_X {}\nOutro : Generico {}\n",
        )
        .unwrap();
        let base = grupos_da_fonte("package Base\nGenerico : Item {}\n").unwrap();
        assert_eq!(itens.grupos.len(), 4);
        let mut mapa = MapaDeHeranca::das_fontes(&[schema, itens, base]);
        assert_eq!(mapa.filhos("Items.Craftable_Uncommon_X"), ["Items.Craftable_Rare_X"]);
        assert_eq!(mapa.filhos("Base.Generico"), ["Items.Outro"]);
        assert!(!mapa.e_base("RTDB.Item"), "herdar do schema não entra");
        assert_eq!(mapa.pares(), 2);
        mapa.filtrar(|r| r != "Items.Outro");
        assert_eq!(mapa.pares(), 1);
        assert!(!mapa.e_base("Base.Generico"));
    }

    /// Diferença 3: a mudança no record base chega aos descendentes do mapa que não a sobrescreveram, em cadeia (neto incluso).
    #[test]
    fn mudanca_no_base_chega_aos_descendentes_do_mapa() {
        let mapa = MapaDeHeranca::ler("Items.Base:\n- Items.Filho\n- Items.Proprio\nItems.Filho:\n- Items.Neto\n").unwrap();
        assert_eq!(MapaDeHeranca::ler(&mapa.escrever()).unwrap(), mapa);
        let tdb = Tdb(HashMap::from([
            ("Items.Base.dano", vec!["10"]),
            ("Items.Filho.dano", vec!["10"]),
            ("Items.Neto.dano", vec!["10"]),
            ("Items.Proprio.dano", vec!["99"]),
            ("Items.Base.tags", vec!["A", "B"]),
            ("Items.Filho.tags", vec!["A", "B", "F"]),
            ("Items.Neto.tags", vec!["A", "B", "F"]),
            ("Items.Proprio.tags", vec!["Q", "R"]),
        ]));
        let mut c = Conjunto::montar([arquivo("Items.Base:\n  dano: 25\n  tags:\n    - !append Nova\n", "a.yaml")]);
        assert_eq!(c.herdar_do_mapa(&mapa, &tdb), 4);
        assert_eq!(c.atribuicao("Items.Filho.dano").unwrap().valor, "25");
        assert_eq!(c.atribuicao("Items.Neto.dano").unwrap().valor, "25");
        assert!(c.atribuicao("Items.Proprio.dano").is_none(), "valor próprio fica");
        let cadeia: Vec<&str> = c.cadeia("Items.Neto.tags").iter().map(|m| m.flat.as_str()).collect();
        assert_eq!(cadeia, ["Items.Base.tags", "Items.Filho.tags", "Items.Neto.tags"]);
        assert!(c.mutacao("Items.Proprio.tags").is_none(), "array que não começa com o do base fica");

        // Troca inteira de array no base: no descendente com valor próprio vira "tira os originais do base, antepõe os novos".
        let mut c = Conjunto::montar([arquivo("Items.Base:\n  tags: [X, Y]\n", "a.yaml")]);
        c.herdar_do_mapa(&mapa, &tdb);
        assert!(c.atribuicao("Items.Base.tags").is_none(), "virou mutação");
        let niveis: Vec<Nivel<String>> = c
            .cadeia("Items.Filho.tags")
            .iter()
            .map(|m| Nivel {
                remocoes: if m.remover_tudo { s(&tdb.0[m.flat.as_str()]) } else { m.remocoes.clone() },
                antepor: m.antepor.clone(),
                antepor_de: vec![],
                anexar: m.anexar.clone(),
                anexar_de: vec![],
            })
            .collect();
        assert_eq!(mutar(s(&tdb.0["Items.Filho.tags"]), &niveis, igual), s(&["X", "Y", "F"]));
    }

    /// Clone de clone declarado antes do clone do qual herda: a mutação do record mais antigo chega a ele do mesmo jeito.
    #[test]
    fn mutacao_chega_ao_clone_de_clone_em_qualquer_ordem() {
        for clones in ["Items.B:\n  $base: Items.A\nItems.C:\n  $base: Items.B\n", "Items.C:\n  $base: Items.B\nItems.B:\n  $base: Items.A\n"] {
            let mut c = Conjunto::montar([arquivo(clones, "a.yaml"), arquivo("Items.A:\n  tags:\n    - !append X\n", "b.yaml")]);
            c.herdar_mutacoes_dos_clones();
            let cadeia: Vec<&str> = c.cadeia("Items.C.tags").iter().map(|m| m.flat.as_str()).collect();
            assert_eq!(cadeia, ["Items.A.tags", "Items.B.tags", "Items.C.tags"], "{clones:?}");
        }
    }

    /// O valor final de `flat` como o aplicador calcula: a atribuição do conjunto (ou o valor do TweakDB) sob a cadeia, com o
    /// `!remove-all` de cada nível expandido nos elementos do flat dele no TweakDB.
    fn valor_final(c: &Conjunto, tdb: &Tdb, flat: &str) -> Vec<String> {
        let alvo = match c.atribuicao(flat) {
            Some(a) => texto_em_lista(&a.valor).expect("lista"),
            None => s(&tdb.0[flat]),
        };
        let niveis: Vec<Nivel<String>> = c
            .cadeia(flat)
            .iter()
            .map(|m| {
                let mut remocoes = m.remocoes.clone();
                if m.remover_tudo {
                    remocoes.extend(s(&tdb.0[m.flat.as_str()]));
                }
                Nivel { remocoes, antepor: m.antepor.clone(), antepor_de: vec![], anexar: m.anexar.clone(), anexar_de: vec![] }
            })
            .collect();
        mutar(alvo, &niveis, igual)
    }

    /// Troca inteira do array do base pelo mesmo valor num arquivo e `!append` em outro: o descendente de mesmo valor recebe o anexo.
    #[test]
    fn filho_de_mesmo_valor_recebe_o_append_de_outro_arquivo() {
        let mapa = MapaDeHeranca::ler("Items.Base:\n- Items.Igual\n").unwrap();
        let tdb = Tdb(HashMap::from([("Items.Base.tags", vec!["A", "B"]), ("Items.Igual.tags", vec!["A", "B"])]));
        let mut c = Conjunto::montar([
            arquivo("Items.Base:\n  tags: [A, B]\n", "a.yaml"),
            arquivo("Items.Base:\n  tags:\n    - !append Z\n", "b.yaml"),
        ]);
        assert_eq!(c.herdar_do_mapa(&mapa, &tdb), 1);
        assert_eq!(valor_final(&c, &tdb, "Items.Base.tags"), s(&["A", "B", "Z"]));
        assert_eq!(valor_final(&c, &tdb, "Items.Igual.tags"), s(&["A", "B", "Z"]));
    }

    /// Atribuição e mutação no mesmo flat do base: cada descendente conta uma vez.
    #[test]
    fn herdar_do_mapa_conta_cada_descendente_uma_vez() {
        let mapa = MapaDeHeranca::ler("Items.Base:\n- Items.Filho\n- Items.Proprio\nItems.Filho:\n- Items.Neto\n").unwrap();
        let tdb = Tdb(HashMap::from([
            ("Items.Base.tags", vec!["A", "B"]),
            ("Items.Filho.tags", vec!["A", "B", "F"]),
            ("Items.Neto.tags", vec!["A", "B", "F"]),
            ("Items.Proprio.tags", vec!["Q", "R"]),
        ]));
        let mut c = Conjunto::montar([
            arquivo("Items.Base:\n  tags: [X]\n", "a.yaml"),
            arquivo("Items.Base:\n  tags:\n    - !append Z\n", "b.yaml"),
        ]);
        assert_eq!(c.herdar_do_mapa(&mapa, &tdb), 3, "Filho, Neto e Proprio");
    }
}
