//! Representação canônica de um tweak: a lista ordenada de [`Op`] que os dois formatos de autoria produzem
//! (`.yaml`/`.toml` por `tweakxl::interpret_from*`, `.tweak` por `tweakxl::tweak_source::parse`) e que os dois aplicadores
//! consomem (o runtime no TweakDB vivo, `cp77-console/src/tweakdb/mod.rs::aplicar_conjunto`; o `bwms tweak` no
//! `tweakdb.bin`, `writer::apply_conjunto`). O nome do formato de terceiro fica no adaptador (`tweakxl`): quem aplica não sabe de
//! que formato a operação veio.
//!
//! Os arquivos não se aplicam um por vez: viram um [`Conjunto`], aplicado nas fases do commit do TweakXL (`TweakChangeset::Commit`):
//! records, re-herança pelo mapa de herança do jogo, atribuições (no mesmo flat vale a última), mutações de array por cima do valor
//! atribuído. Um `!append` de um arquivo sobrevive à troca inteira do mesmo array em outro arquivo, em qualquer ordem.

use std::collections::{HashMap, HashSet};

/// Uma operação de alto nível extraída do documento.
pub enum Op {
    /// `$base`: clonar `base` em `record`.
    Clone { record: String, base: String },
    /// `$type`: criar `record` da classe `class` (offline = clona uma amostra).
    Create { record: String, class: String },
    /// Editar um flat que já existe (prop de record: o TweakXL recusa prop desconhecida sem `$props: AutoFlats`).
    Edit { flat: String, op: EditOp },
    /// Editar um flat que nasce com o tipo `tipo` se não existir (nome do tipo de flat do motor: `Int32`, `array:Float`,
    /// `raRef:CResource`). O TweakXL cria o flat quando o arquivo declara o tipo (`$type` com `$value`, o tipo do `.tweak`) ou quando o tipo
    /// sai do valor (`$props: AutoFlats`, flat solto no topo do arquivo).
    Criar { flat: String, tipo: String, op: EditOp },
}

/// Operação de edição de um flat. As variantes de array espelham as tags do
/// TweakXL (`!append`, `!prepend`, `!append-once`, `!prepend-once`, `!remove`). Mora aqui (não no
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
    /// `!remove-all` — limpa o array INTEIRO, incondicionalmente (`TweakChangeset::
    /// RemoveAllElements`, fonte real: `entry.deleteAll = true`, SEM valor nenhum — distinto de
    /// `!remove`, que só tira elementos IGUAIS a um valor dado).
    RemoveAll,
    /// `!append-from` / `!merge` — anexa ao FIM os elementos de outro flat array
    /// (pelo nome) que o alvo ainda não tem. O TweakXL trata `!merge` e `!append-from` como o mesmo op.
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

/// As mutações de um array no conjunto, juntas de todos os arquivos (a `MutationEntry` do TweakXL).
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
    /// A mutação que vem antes desta na cadeia (o `baseId` do TweakXL): a do mesmo array no record de quem este herdou.
    pub base: Option<String>,
}

/// Os arquivos de tweak juntos, nas fases do commit do TweakXL.
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

/// O mapa de herança dos records do jogo: record → os que herdam dele diretamente (o `InheritanceMap` do TweakXL; o do BWMS sai das fontes
/// `.tweak` do jogo por `bwms tweak heranca`). Texto: `Record:` e embaixo `- Filho`, uma chave por record.
#[derive(Default, Debug, PartialEq)]
pub struct MapaDeHeranca {
    filhos: HashMap<String, Vec<String>>,
}

impl MapaDeHeranca {
    pub fn ler(texto: &str) -> Result<Self, String> {
        let raiz = crate::tweakxl::yaml::parse(texto)?;
        let entradas = raiz.as_map().ok_or("o mapa de herança precisa ser um mapa record → filhos")?;
        let mut filhos: HashMap<String, Vec<String>> = HashMap::new();
        for (record, lista) in entradas {
            let itens = lista.as_seq().ok_or_else(|| format!("'{record}': os filhos vão numa lista `- Nome`"))?;
            let nomes: Option<Vec<String>> = itens.iter().map(|n| n.as_str().map(str::to_string)).collect();
            let nomes = nomes.ok_or_else(|| format!("'{record}': filho que não é nome de record"))?;
            filhos.entry(record.clone()).or_default().extend(nomes);
        }
        Ok(MapaDeHeranca { filhos })
    }

    /// De pares `(filho, base)`.
    pub fn de_pares(pares: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut filhos: HashMap<String, Vec<String>> = HashMap::new();
        for (filho, base) in pares {
            let l = filhos.entry(base).or_default();
            if !l.contains(&filho) {
                l.push(filho);
            }
        }
        MapaDeHeranca { filhos }
    }

    /// O mapa das fontes `.tweak` do jogo, pela regra do exportador do TweakXL (`MetadataExporter::ResolveGroups`): o grupo de um arquivo com
    /// `package P` chama `P.Nome`; a base sem pacote acha o grupo pelo nome inteiro, depois no pacote do arquivo, depois nos `using`. Entra o
    /// par record → base quando os dois são records (fora dos pacotes `RTDB`, de schema, e `Query`, e sem `[Debug]`); herdar direto do
    /// schema não é herança de record. Records inline (`{ }` dentro de flat) ficam de fora: o nome deles (`_inlineN`) só se acha pelo TweakDB.
    pub fn das_fontes(fontes: &[crate::tweakxl::tweak_source::FonteDeGrupos]) -> Self {
        let nome_de = |pacote: &Option<String>, nome: &str| match pacote {
            Some(p) => format!("{p}.{nome}"),
            None => nome.to_string(),
        };
        let e_record = |pacote: &Option<String>| !matches!(pacote.as_deref(), Some("RTDB" | "Query"));
        let mut todos: HashMap<String, bool> = HashMap::new();
        let mut do_pacote: HashMap<String, HashSet<String>> = HashMap::new();
        for f in fontes {
            for g in &f.grupos {
                todos.insert(nome_de(&f.pacote, &g.nome), e_record(&f.pacote) && !g.debug);
                do_pacote.entry(f.pacote.clone().unwrap_or_default()).or_default().insert(g.nome.clone());
            }
        }
        let mut pares = Vec::new();
        for f in fontes {
            for g in &f.grupos {
                let (Some(base), true, false) = (&g.base, e_record(&f.pacote), g.debug) else { continue };
                let achada = if todos.contains_key(base) {
                    Some(base.clone())
                } else {
                    std::iter::once(f.pacote.clone().unwrap_or_default())
                        .chain(f.usings.iter().cloned())
                        .find(|p| do_pacote.get(p).is_some_and(|s| s.contains(base)))
                        .map(|p| if p.is_empty() { base.clone() } else { format!("{p}.{base}") })
                };
                if let Some(b) = achada.filter(|b| todos.get(b) == Some(&true)) {
                    pares.push((nome_de(&f.pacote, &g.nome), b));
                }
            }
        }
        Self::de_pares(pares)
    }

    /// Fica só com os pares em que o record e a base passam em `existe` (os records do `tweakdb.bin` instalado).
    pub fn filtrar(&mut self, existe: impl Fn(&str) -> bool) {
        self.filhos.retain(|b, _| existe(b));
        for f in self.filhos.values_mut() {
            f.retain(|x| existe(x));
        }
        self.filhos.retain(|_, f| !f.is_empty());
    }

    /// Pares record → base no mapa.
    pub fn pares(&self) -> usize {
        self.filhos.values().map(Vec::len).sum()
    }

    /// O texto que [`MapaDeHeranca::ler`] lê, com records e filhos em ordem alfabética (arquivo reprodutível).
    pub fn escrever(&self) -> String {
        let mut bases: Vec<&String> = self.filhos.keys().collect();
        bases.sort();
        let mut s = String::new();
        for b in bases {
            let mut f: Vec<&String> = self.filhos[b].iter().collect();
            f.sort();
            s.push_str(b);
            s.push_str(":\n");
            for x in f {
                s.push_str("- ");
                s.push_str(x);
                s.push('\n');
            }
        }
        s
    }

    pub fn filhos(&self, record: &str) -> &[String] {
        self.filhos.get(record).map_or(&[], Vec::as_slice)
    }

    pub fn e_base(&self, record: &str) -> bool {
        self.filhos.contains_key(record)
    }

    pub fn bases(&self) -> usize {
        self.filhos.len()
    }
}

impl Conjunto {
    /// Junta as operações dos arquivos, na ordem em que vêm.
    pub fn montar(arquivos: impl IntoIterator<Item = Vec<Op>>) -> Conjunto {
        let mut c = Conjunto::default();
        let mut pos_atrib: HashMap<String, usize> = HashMap::new();
        for ops in arquivos {
            for op in ops {
                let (flat, tipo, edit) = match op {
                    Op::Clone { .. } | Op::Create { .. } => {
                        c.registros.push(op);
                        continue;
                    }
                    Op::Edit { flat, op } => (flat, None, op),
                    Op::Criar { flat, tipo, op } => (flat, Some(tipo), op),
                };
                if let EditOp::Assign(valor) = edit {
                    match pos_atrib.get(&flat) {
                        Some(&i) => {
                            let a = &mut c.atribuicoes[i];
                            a.valor = valor;
                            if tipo.is_some() {
                                a.tipo = tipo;
                            }
                        }
                        None => {
                            pos_atrib.insert(flat.clone(), c.atribuicoes.len());
                            c.atribuicoes.push(Atribuicao { flat, tipo, valor });
                        }
                    }
                    continue;
                }
                let m = c.mutacao_mut(&flat);
                if tipo.is_some() {
                    m.tipo = tipo;
                }
                match edit {
                    EditOp::Append(v) => m.anexar.push((v, false)),
                    EditOp::AppendOnce(v) => m.anexar.push((v, true)),
                    EditOp::Prepend(v) => m.antepor.push((v, false)),
                    EditOp::PrependOnce(v) => m.antepor.push((v, true)),
                    EditOp::Remove(v) => m.remocoes.push(v),
                    EditOp::RemoveAll => m.remover_tudo = true,
                    EditOp::AppendFrom(f) => m.anexar_de.push(f),
                    EditOp::PrependFrom(f) => m.antepor_de.push(f),
                    EditOp::Assign(_) => unreachable!("atribuição tratada acima"),
                }
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

    fn mutacao_mut(&mut self, flat: &str) -> &mut Mutacao {
        let i = match self.mutacoes.iter().position(|m| m.flat == flat) {
            Some(i) => i,
            None => {
                self.mutacoes.push(Mutacao { flat: flat.to_string(), ..Mutacao::default() });
                self.mutacoes.len() - 1
            }
        };
        &mut self.mutacoes[i]
    }

    /// A cadeia de mutações de `flat`: da do record mais antigo da herança à do próprio flat (a ordem em que o TweakXL aplica).
    pub fn cadeia(&self, flat: &str) -> Vec<&Mutacao> {
        let mut cadeia = Vec::new();
        let mut vistos = HashSet::new();
        let mut atual = self.mutacao(flat);
        while let Some(m) = atual {
            if !vistos.insert(m.flat.as_str()) {
                break;
            }
            cadeia.push(m);
            atual = m.base.as_deref().and_then(|b| self.mutacao(b));
        }
        cadeia.reverse();
        cadeia
    }

    /// Record novo de `$base`: cada array do record-base que o conjunto muta e o clone não reatribui herda a mutação (o clone nasce do valor
    /// de antes; a mutação do base entra na cadeia dele, como no TweakXL).
    pub fn herdar_mutacoes_dos_clones(&mut self) {
        let clones: Vec<(String, String)> = self
            .registros
            .iter()
            .filter_map(|op| match op {
                Op::Clone { record, base } => Some((record.clone(), base.clone())),
                _ => None,
            })
            .collect();
        for (record, base) in clones {
            let prefixo = format!("{base}.");
            let do_base: Vec<String> = self.mutacoes.iter().filter(|m| m.flat.starts_with(&prefixo)).map(|m| m.flat.clone()).collect();
            for flat_base in do_base {
                let alvo = format!("{record}.{}", &flat_base[prefixo.len()..]);
                if self.atribuicao(&alvo).is_none() {
                    self.mutacao_mut(&alvo).base = Some(flat_base);
                }
            }
        }
    }

    /// Leva a mudança de um record do jogo aos records do jogo que herdam dele (o `ReinheritFlat` + a resolução do `Commit` do TweakXL):
    /// - atribuição: o descendente que ainda tem o mesmo valor do base ganha a mesma atribuição; num array com valor próprio, a atribuição
    ///   do base vira "tira os elementos originais, antepõe os novos" e o descendente entra na cadeia dela;
    /// - mutação: o descendente com o mesmo valor, ou cujo array começa com o do base, entra na cadeia;
    /// - o descendente que o conjunto já reatribui fica com o valor dele; e o que também é base leva a mudança adiante.
    ///
    /// Roda antes de qualquer gravação: as perguntas são sobre o TweakDB de antes do conjunto. Devolve quantos flats de descendentes
    /// entraram.
    pub fn herdar_do_mapa(&mut self, mapa: &MapaDeHeranca, tdb: &impl ConsultaDeHeranca) -> usize {
        if mapa.bases() == 0 {
            return 0;
        }
        let mut fila: Vec<String> = self
            .atribuicoes
            .iter()
            .map(|a| a.flat.clone())
            .chain(self.mutacoes.iter().map(|m| m.flat.clone()))
            .filter(|f| record_e_apendice(f).is_some_and(|(r, _)| mapa.e_base(r)))
            .collect();
        let mut vistos: HashSet<String> = fila.iter().cloned().collect();
        let mut convertidas: HashSet<String> = HashSet::new();
        let mut entraram = 0;
        while let Some(flat) = fila.pop() {
            let Some((record, apendice)) = record_e_apendice(&flat) else { continue };
            let (record, apendice) = (record.to_string(), apendice.to_string());
            if !tdb.record_existe(&record) || !tdb.flat_existe(&flat) {
                continue;
            }
            let atribuicao = self.atribuicao(&flat).cloned().filter(|_| !convertidas.contains(&flat));
            let e_mutacao = self.mutacao(&flat).is_some();
            for filho in mapa.filhos(&record) {
                if !tdb.record_existe(filho) {
                    continue;
                }
                let alvo = format!("{filho}{apendice}");
                if self.atribuicao(&alvo).is_some() || !tdb.flat_existe(&alvo) {
                    continue;
                }
                if let Some(a) = &atribuicao {
                    if tdb.mesmo_valor(&alvo, &flat) {
                        self.atribuicoes.push(Atribuicao { flat: alvo.clone(), tipo: a.tipo.clone(), valor: a.valor.clone() });
                    } else if tdb.e_array(&flat) {
                        if convertidas.insert(flat.clone()) {
                            let novos = texto_em_lista(&a.valor).unwrap_or_default();
                            let m = self.mutacao_mut(&flat);
                            m.remover_tudo = true;
                            m.antepor.extend(novos.into_iter().map(|v| (v, false)));
                        }
                        self.mutacao_mut(&alvo).base = Some(flat.clone());
                    } else {
                        continue;
                    }
                } else if e_mutacao || convertidas.contains(&flat) {
                    if !tdb.mesmo_valor(&alvo, &flat) && !tdb.comeca_com(&flat, &alvo) {
                        continue;
                    }
                    self.mutacao_mut(&alvo).base = Some(flat.clone());
                } else {
                    continue;
                }
                entraram += 1;
                if mapa.e_base(filho) && vistos.insert(alvo.clone()) {
                    fila.push(alvo);
                }
            }
        }
        self.atribuicoes.retain(|a| !convertidas.contains(&a.flat));
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

/// Aplica a cadeia ao array `alvo` como o `Commit` do TweakXL: primeiro as remoções de todos os níveis, depois os anteposições (na ordem
/// dos arquivos, base antes), depois os anexos. `!*-once` e os `!*-from` pulam elemento que o array já tem; um elemento removido num nível
/// de cima (descendente) não volta por inserção de nível de baixo (base). `igual` compara dois elementos como o tipo deles.
pub fn mutar<T: Clone>(mut alvo: Vec<T>, cadeia: &[Nivel<T>], igual: impl Fn(&T, &T) -> bool) -> Vec<T> {
    alvo.retain(|e| !cadeia.iter().any(|n| n.remocoes.iter().any(|d| igual(e, d))));
    let pula = |v: &T, nivel: usize| cadeia.iter().enumerate().any(|(l, n)| l > nivel && n.remocoes.iter().any(|d| igual(d, v)));
    let tem = |a: &[T], v: &T| a.iter().any(|e| igual(e, v));
    let mut pos = 0;
    for (l, n) in cadeia.iter().enumerate() {
        for (v, unico) in &n.antepor {
            if (*unico && tem(&alvo, v)) || pula(v, l) {
                continue;
            }
            alvo.insert(pos, v.clone());
            pos += 1;
        }
        for fonte in &n.antepor_de {
            for v in fonte {
                if tem(&alvo, v) || pula(v, l) {
                    continue;
                }
                alvo.insert(pos, v.clone());
                pos += 1;
            }
        }
    }
    for (l, n) in cadeia.iter().enumerate() {
        for (v, unico) in &n.anexar {
            if (*unico && tem(&alvo, v)) || pula(v, l) {
                continue;
            }
            alvo.push(v.clone());
        }
        for fonte in &n.anexar_de {
            for v in fonte {
                if tem(&alvo, v) || pula(v, l) {
                    continue;
                }
                alvo.push(v.clone());
            }
        }
    }
    alvo
}

#[cfg(test)]
mod tests {
    use super::{lista_em_texto, mutar, texto_em_lista, Conjunto, ConsultaDeHeranca, EditOp, MapaDeHeranca, Nivel, Op};
    use crate::tweakxl::{interpret_from, template, tweak_source, yaml};
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
        // Os anteposições ficam na ordem dos arquivos (o TweakXL anda o índice de inserção), não invertidos.
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
        use crate::tweakxl::tweak_source::grupos_da_fonte;
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
}
