//! Tabelas de runtime (`red4ext/bwms-*.txt`, formato em [`crate::tabelas`]) emitidas dos `.xl` — camada PURA (offline, testável sem
//! jogo): hash de ResourcePath, plano de redirecionamento (resource.link / resource.copy) e um emissor por seção de
//! [`crate::instalacao::xl::XlFile`]. A camada ENGINE (ganchos do loader e do depot, in-game) mora no
//! runtime (cp77-console), NÃO aqui. RE do loader em `cp77-symbols/notes/archivexl-apply-re.md`.

use crate::instalacao::xl::XlFile;
use crate::tabelas;
use std::collections::{HashMap, HashSet};

/// Hash de ResourcePath do CP2077 = **FNV-1a 64-bit** do path normalizado (lowercase, separador `\`).
/// FONTE ÚNICA no crate `bwms-primitives` (compartilhada com o runtime cp77-console). VALIDADO offline:
/// 5/5 goldens do índice de um .archive real batem exato (`tests::hash_goldens`, abaixo).
pub use crate::resource_path_hash;

/// Plano do `resource.link` e do `resource.copy` de um ou mais `.xl`: os mapas que o runtime aplica e o caminho em texto de cada hash, para
/// `emit_reslink_do_plano` gravar exatamente esses mapas. As outras seções do `.xl` têm emissor próprio (`emit_*`) e tabela no install.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlanoDeRedirecionamento {
    /// `hash(path pedido) → hash(path servido)` de `resource.link` (os pares `ResourceLink` do parse, com a cadeia A→B→C colapsada em A→C) e
    /// `hash(cópia) → hash(original)` de `resource.copy` (sem encadeamento, como a referência). Cada `len()` é o número de linhas desse tipo que
    /// `emit_reslink_do_plano` grava; pedido repetido conta uma vez.
    ///
    /// **Dois mapas, ArchiveXL `#26`:** cada hook da referência (`ResourceLink/Extension.cpp:208-247`) consulta um mapa diferente —
    ///
    ///   `OnLoaderResourceRequest`  -> só `s_links`
    ///   `OnDepotResourceRequest`   -> só `s_copies`
    ///   `OnDepotResourceCheck`     -> `s_mappings` (a união)
    ///   `GetAliases`               -> `s_mappings`, nos DOIS sentidos
    ///
    /// Num mapa só, um `resource.copy` responderia uma requisição de LOADER, coisa que o ArchiveXL nunca faz. A união e o `GetAliases` são do
    /// runtime, sobre as tabelas que ele relê do `bwms-reslink.txt` (`check` e `apelidos_de_copia` em `arquivos_do_jogo/depot.rs`,
    /// `reslink_aliases_of` em `arquivos_do_jogo/mod.rs`).
    pub mapa_links: HashMap<u64, u64>,
    pub mapa_copies: HashMap<u64, u64>,
    /// Caminho pedido de cada link que cai num ciclo (A→B→…→A, chega a um, ou aponta para si mesmo), em ordem: fica fora de `mapa_links`, como o
    /// `Link "…" is cyclic` / `points to itself` da referência (`ResourceLink/Extension.cpp`), que loga e descarta.
    pub cyclic_links: Vec<String>,
    /// Link e cópia que ficam fora de `mapa_links` e `mapa_copies` por regra da referência que não é ciclo, em PT-BR com a ação: a cópia de si mesma
    /// e o link cujo pedido é uma cópia (as chaves dos dois mapas nunca coincidem). A gravação das tabelas leva o texto aos avisos do relatório da `Mods/`.
    pub avisos: Vec<String>,
    /// Caminho em texto de cada hash citado em link/copy, na grafia do primeiro `.xl` que o cita (o alvo de escopo, na de `resource_scope_listas`). O
    /// runtime re-hasheia o texto, então qualquer grafia do mesmo hash dá o mesmo par.
    pub caminhos: HashMap<u64, String>,
}

/// Estado de um nó na busca de `resolver`.
#[derive(Clone, Copy)]
enum Visita {
    /// Na pilha: chegar nele de novo fecha um ciclo.
    EmCurso,
    /// Sucessores todos vistos: as folhas dele já entraram na resolução corrente.
    Feita,
}

/// Resolução canônica de caminho do ArchiveXL, a mesma do `resource.scope` e do `resource.link`: as folhas que `raiz` alcança em `grafo` (nó que não é
/// chave do grafo), em pré-ordem e sem repetição, e se a busca passou por um ciclo. É o fecho do `ResourceMetaExtension::Configure` (escopo dentro de
/// escopo vira os alvos dele) e, com um sucessor por nó, a cadeia do `ResourceLinkExtension::Configure` (A→B→C vira A→C; com ciclo não há folha).
/// Busca em profundidade com estado por nó e pilha explícita: cada nó entra uma vez por raiz, então termina em qualquer grafo, inclusive no ciclo
/// indireto em que o laço `do`/`while` da referência não sai.
fn resolver(grafo: &HashMap<u64, Vec<u64>>, raiz: u64) -> (Vec<u64>, bool) {
    let mut estado: HashMap<u64, Visita> = HashMap::from([(raiz, Visita::EmCurso)]);
    let mut pilha: Vec<(u64, usize)> = vec![(raiz, 0)];
    let mut folhas: Vec<u64> = Vec::new();
    let mut ciclo = false;
    while let Some((no, i)) = pilha.pop() {
        let Some(&prox) = grafo.get(&no).and_then(|s| s.get(i)) else {
            estado.insert(no, Visita::Feita);
            continue;
        };
        pilha.push((no, i + 1));
        if !grafo.contains_key(&prox) {
            if !folhas.contains(&prox) {
                folhas.push(prox);
            }
            continue;
        }
        match estado.get(&prox) {
            Some(Visita::EmCurso) => ciclo = true,
            Some(Visita::Feita) => {}
            None => {
                estado.insert(prox, Visita::EmCurso);
                pilha.push((prox, 0));
            }
        }
    }
    (folhas, ciclo)
}

/// Plano de um `.xl` só (`build_apply_plan_combinado`) com os escopos dele, o mesmo que o install monta quando esse `.xl` é o único ativo: o
/// `bwms xl <f> reslink` emite a tabela que o install grava.
pub fn build_apply_plan(xl: &XlFile) -> PlanoDeRedirecionamento {
    build_apply_plan_combinado(&[xl], &resource_scope_listas(&[xl]).0)
}

/// Constrói o plano de `resource.link` + `resource.copy` de vários `.xl` juntos, como o `Configure` da referência junta as configs antes de resolver
/// cadeia e ciclo: `a: b` num `.xl` e `b: c` noutro dão `a → c`. Pedido repetido: vence o `.xl` que vem depois em `xls` (o `finalLinks[...] =` da
/// referência e o `retain` + `push` do runtime).
///
/// `link`: cada `ResourceLink` dá a aresta `pedido → servido` do grafo que o `resolver` percorre. A direção da forma escalar ou sequência do
/// YAML já vem resolvida do parse (doc de `ResourceLink`). O pedido passa por `caminho` primeiro e o servido depois, porque a grafia que fica em `caminhos` é a do primeiro que cita o hash.
///
/// Link que aponta para si mesmo sai antes da cadeia, como o `points to itself` da referência, e não derruba quem aponta para ele. Depois,
/// `resolver` colapsa cada cadeia no alvo final e tira o link que cai num ciclo (`cyclic_links`).
///
/// `copy`: cada `target` é uma cópia de `source` → sempre `mapa_copies[target] = source`
/// (Config.cpp não tem essa assimetria pra copy — as 2 formas armazenam igual).
///
/// A cópia igual ao original e o link cujo pedido é cópia de qualquer `.xl` saem, com o motivo em `avisos`, como a referência faz com as cópias
/// de todas as configs antes dos links; o link sobre cópia sai antes da cadeia.
///
/// `scopes` (`resource_scope_listas` dos mesmos `.xl`) expande o pedido de cada link (a chave na forma escalar, cada item na sequência) e cada
/// cópia do copy, como o `ExpandList` de `ResourceLink/Extension.cpp:53,87`; o lado servido e o original do copy ficam como estão. O caminho igual
/// ao servido é conferido depois da expansão, como na referência.
pub fn build_apply_plan_combinado(xls: &[&XlFile], scopes: &HashMap<u64, Vec<String>>) -> PlanoDeRedirecionamento {
    let mut caminhos: HashMap<u64, String> = HashMap::new();
    let mut caminho = |s: &str| {
        let h = resource_path_hash(s);
        caminhos.entry(h).or_insert_with(|| s.to_string());
        h
    };
    let mut grafo: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut para_si: Vec<u64> = Vec::new();
    let mut mapa_copies: HashMap<u64, u64> = HashMap::new();
    let mut copias_para_si: Vec<u64> = Vec::new();
    for xl in xls {
        for l in &xl.links {
            for (_, pedido) in expandir_escopo(std::slice::from_ref(&l.pedido), scopes) {
                let (pedido, servido) = (caminho(&pedido), caminho(&l.servido));
                if pedido == servido {
                    para_si.push(pedido);
                } else {
                    grafo.insert(pedido, vec![servido]);
                }
            }
        }
        for c in &xl.copies {
            for (_, copia) in expandir_escopo(&c.targets, scopes) {
                let (copia, original) = (caminho(&copia), caminho(&c.source));
                if copia == original {
                    copias_para_si.push(copia);
                } else {
                    mapa_copies.insert(copia, original);
                }
            }
        }
    }
    // Pedido de link que é cópia sai antes da cadeia, como o `finalCopies.contains(linkPath)` da referência (`ResourceLink/Extension.cpp:99-107`,
    // que junta as cópias de todas as configs antes dos links): com os dois, o loader trocaria o pedido pelo link e o depot o serviria como cópia.
    // Tirar do grafo, e não de `mapa_links` depois do `resolver`, mantém `a → k` quando `k` é cópia e tem link (a referência aceita alvo que é cópia).
    let sobre_copia: Vec<u64> = grafo.keys().copied().filter(|h| mapa_copies.contains_key(h)).collect();
    grafo.retain(|h, _| !mapa_copies.contains_key(h));
    // Cópia de si mesma que nenhuma outra entrada pôs na tabela (o `Copy "…" points to itself` de `Extension.cpp:55-63`).
    copias_para_si.retain(|h| !mapa_copies.contains_key(h));
    let em_texto = |hs: &[u64]| {
        let mut v: Vec<&str> = hs.iter().filter_map(|h| caminhos.get(h).map(String::as_str)).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let avisos: Vec<String> = em_texto(&copias_para_si)
        .into_iter()
        .map(|c| format!("resource.copy {c}: a cópia é o próprio original e fica fora de bwms-reslink.txt; tire esse caminho da lista de cópias no .xl"))
        .chain(em_texto(&sobre_copia).into_iter().map(|c| {
            format!("resource.link {c}: o pedido é um resource.copy e fica fora de bwms-reslink.txt; tire esse caminho do link ou do copy no .xl")
        }))
        .collect();
    let mut p = PlanoDeRedirecionamento { avisos, ..PlanoDeRedirecionamento::default() };
    let mut fora: Vec<u64> = Vec::new();
    for &pedido in grafo.keys() {
        match resolver(&grafo, pedido) {
            (folhas, false) => p.mapa_links.extend(folhas.first().map(|&servido| (pedido, servido))),
            (_, true) => fora.push(pedido),
        }
    }
    fora.extend(para_si.into_iter().filter(|h| !p.mapa_links.contains_key(h)));
    p.cyclic_links = fora.iter().filter_map(|h| caminhos.get(h).cloned()).collect();
    p.cyclic_links.sort();
    p.cyclic_links.dedup();
    p.mapa_copies = mapa_copies;
    p.caminhos = caminhos;
    p
}

/// Emits `bwms-resfixes.txt` content: one line per fix entry (`tabelas::Resfix`).
/// Format: `<resource_path> | names | <old_cname> | <new_cname>`
///         `<resource_path> | paths | <old_path> | <new_path>`
/// The runtime can load this to apply fixes in CMesh/MorphTargetMesh PostLoad hooks.
pub fn emit_resfixes(xl: &XlFile) -> String {
    let mut out = tabelas::RESFIXES.cabecalho();
    for fix in &xl.fixes {
        for (tipo, pares) in [(tabelas::TipoDeFix::Nomes, &fix.names), (tabelas::TipoDeFix::Caminhos, &fix.paths), (tabelas::TipoDeFix::Contexto, &fix.context)] {
            for (antigo, novo) in pares {
                tabelas::Resfix { recurso: &fix.resource, tipo, antigo, novo }.gravar(&mut out);
            }
        }
    }
    out
}

/// Conteúdo de `red4ext/bwms-reslink.txt` de um `.xl` só (`emit_reslink_do_plano` sobre `build_apply_plan`).
pub fn emit_reslink(xl: &XlFile) -> String {
    emit_reslink_do_plano(&build_apply_plan(xl))
}

/// Conteúdo de `red4ext/bwms-reslink.txt` de um plano: uma linha `<path_pedido> | <path_servido>` por entrada de `mapa_links` e
/// `<cópia> | <original> | copy` por entrada de `mapa_copies` (`tabelas::Reslink`), cada grupo em ordem de caminho pedido. O runtime (`cp77-console`,
/// `arquivos_do_jogo::reslink_file`) re-hasheia os dois lados e troca um salto por pedido, então o texto tem de ser o plano já resolvido (cadeia
/// colapsada, ciclo fora) e não o `.xl` cru.
pub fn emit_reslink_do_plano(p: &PlanoDeRedirecionamento) -> String {
    fn em_ordem<'a>(mapa: &HashMap<u64, u64>, caminhos: &'a HashMap<u64, String>) -> Vec<(&'a str, &'a str)> {
        let mut v: Vec<(&str, &str)> = mapa.iter().filter_map(|(de, para)| Some((caminhos.get(de)?.as_str(), caminhos.get(para)?.as_str()))).collect();
        v.sort_unstable();
        v
    }
    let mut out = tabelas::RESLINK.cabecalho();
    for (pedido, servido) in em_ordem(&p.mapa_links, &p.caminhos) {
        tabelas::Reslink { pedido, servido, copia: false }.gravar(&mut out);
    }
    // `copy` vai marcado: o runtime serve a cópia no depot (instância própria, bytes do original), não no loader como o `link`.
    for (copia, original) in em_ordem(&p.mapa_copies, &p.caminhos) {
        tabelas::Reslink { pedido: copia, servido: original, copia: true }.gravar(&mut out);
    }
    out
}

/// Emite o conteúdo de `red4ext/bwms-streaming.txt` a partir do `.xl` (ArchiveXL `#49` parte 4d): uma linha por deleção de node de setor
/// (`streaming.sectors[].nodeDeletions`), no formato `<mod> | <path do setor> | <expectedNodes> | <nodeIndex> | <tipo>`. O runtime
/// (`cp77-console`, `mundo::streaming_file`) agrupa por (mod, setor) e só aplica o grupo se todas as linhas passarem na conferência (contagem de setups,
/// índice e classe do node) — um erro cancela o mod no setor, como no ArchiveXL. Sub-deleções e mutações saem em linhas de 7 colunas
/// (`... | <op> | <valores>`, ver `linhas_de_mutacao`), no formato de `tabelas::Streaming`.
/// `mod_id` identifica o `.xl` de origem.
pub fn emit_streaming_table(xl: &XlFile, mod_id: &str) -> String {
    let mut out = tabelas::STREAMING.cabecalho();
    if let Some(streaming) = &xl.streaming {
        for setor in &streaming.sectors {
            // O parse do `.xl` só deixa `expectedNodes` > 0 e índice em 0..expectedNodes: o `else` não acontece com `.xl` que parseou.
            let linha = |indice: i64, tipo: &str, op: Option<(&str, &str)>, out: &mut String| {
                let (Ok(esperado), Ok(indice)) = (usize::try_from(setor.expected_nodes), usize::try_from(indice)) else { return };
                tabelas::Streaming { mod_id, setor: &setor.path, esperado, indice, tipo, op }.gravar(out);
            };
            for d in &setor.node_deletions {
                if d.sub_node_deletions.is_empty() && d.shape_deletions.is_empty() {
                    linha(d.node_index, &d.node_type, None, &mut out);
                }
                if !d.sub_node_deletions.is_empty() {
                    let indices: Vec<String> = d.sub_node_deletions.iter().map(|i| i.to_string()).collect();
                    let valores = format!("{} {}", d.expected_sub_nodes, indices.join(" "));
                    linha(d.node_index, &d.node_type, Some(("subdel", &valores)), &mut out);
                }
                // `[ator, shape]` (ArchiveXL 1.27.0): `subshape <atores esperados> <ator> <shape> ...`.
                if !d.shape_deletions.is_empty() {
                    let pares: Vec<String> = d.shape_deletions.iter().map(|(a, s)| format!("{a} {s}")).collect();
                    let valores = format!("{} {}", d.expected_sub_nodes, pares.join(" "));
                    linha(d.node_index, &d.node_type, Some(("subshape", &valores)), &mut out);
                }
            }
            for m in &setor.node_mutations {
                for (op, valores) in linhas_de_mutacao(m) {
                    linha(m.node_index, &m.node_type, Some((op, &valores)), &mut out);
                }
            }
        }
    }
    out
}

/// Emite o conteúdo de `red4ext/bwms-questphase.txt` a partir do `.xl` (ArchiveXL `#35` parte 2d-1): uma linha por fase de mod
/// (`quest.phases[]`), no formato `<mod> | <fase pai> | <fase do mod> | <nó de entrada> | <socket de entrada> | <nó de saída> | <socket de saída> |
/// <intercept>`. Caminho de nó = ids separados por espaço (vazio = o `questStartNodeDefinition` do grafo, como na referência); socket ausente = `-`;
/// intercept `1`/`0`. Os caminhos de recurso seguem em texto porque o id do nó novo sai do CRC-16 de `"<fase do mod>:<id do nó de entrada>"`
/// (`crate::quest_phase_node_id`), e o id do nó de entrada só se sabe no grafo quando o caminho é vazio. `mod_id` identifica o `.xl`. Linha de
/// `tabelas::Questphase`.
pub fn emit_questphase_table(xl: &XlFile, mod_id: &str) -> String {
    let mut out = tabelas::QUESTPHASE.cabecalho();
    for p in &xl.quest_phases {
        tabelas::Questphase {
            mod_id,
            pai: &p.parent,
            fase: &p.phase_path,
            entrada: p.input.node_path.clone(),
            socket_entrada: p.input.socket.as_deref(),
            saida: p.output.node_path.clone(),
            socket_saida: p.output.socket.as_deref(),
            intercept: p.intercept,
        }
        .gravar(&mut out);
    }
    out
}

/// Emite o conteúdo de `red4ext/bwms-journal.txt` a partir do `.xl` (ArchiveXL `#44`, fluxo de produção): uma linha por `.journal` de mod (`journal:`), no
/// formato `<mod> | <caminho do .journal>`. O runtime carrega cada um, mescla as entradas na árvore viva e coleta os mappins das entradas novas. `mod_id`
/// identifica o `.xl`. Linha de `tabelas::Journal`.
pub fn emit_journal_table(xl: &XlFile, mod_id: &str) -> String {
    let mut out = tabelas::JOURNAL.cabecalho();
    for caminho in xl.journals.iter().map(|j| j.trim()).filter(|j| !j.is_empty()) {
        tabelas::Journal { mod_id, caminho }.gravar(&mut out);
    }
    out
}

/// Tabela do `localization` (ArchiveXL `#45`) pro runtime: uma linha `<mod> | <tipo> | <idioma> | <caminho>` (`tabelas::Localizacao`) por recurso de
/// texto do mod. O runtime pede ao motor os recursos do idioma em uso (o motor lê o `.archive` do mod, comprimido ou não) e insere as entradas no
/// repositório vivo de textos.
pub fn emit_localization_table(xl: &XlFile, mod_id: &str) -> String {
    let mut out = tabelas::LOCALIZACAO.cabecalho();
    for g in &xl.localization {
        for (idioma, caminhos) in &g.entries {
            for caminho in caminhos.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
                tabelas::Localizacao { mod_id, tipo: &g.kind, idioma: idioma.trim(), caminho }.gravar(&mut out);
            }
        }
    }
    out
}

/// Um aviso por grupo de `localization` cujo tipo o runtime não aplica (`crate::TIPOS_DE_TEXTO_APLICADOS`), para o relatório da `Mods/`
/// (`apply::TabelasDosXl::sem_efeito_no_bwms`) e o classify (`ModReport::avisos_do_xl`) dizerem o mesmo. O grupo continua em `emit_localization_table`, e o único
/// leitor da tabela, o `recursos_do_idioma` do runtime, o deixa de fora antes de escolher a reserva. Por isso a ação é tirar o grupo do `.xl`:
/// o aviso some na abertura seguinte e o jogo fica igual (`apply::tests::localizacao_que_o_runtime_nao_aplica_sai_a_parte_e_some_sem_o_grupo`).
pub fn localizacao_sem_efeito(xl: &XlFile) -> Vec<String> {
    let aplicados = crate::TIPOS_DE_TEXTO_APLICADOS.join(", ");
    xl.localization
        .iter()
        .filter(|g| !crate::TIPOS_DE_TEXTO_APLICADOS.contains(&g.kind.as_str()))
        .map(|g| {
            format!(
                "localization.{k}: não aplicado pelo runtime do BWMS, que só aplica {aplicados}, e o grupo fica sem efeito no jogo; tire o grupo {k} do \
                 localization deste .xl para o aviso sumir (o jogo fica igual)",
                k = g.kind
            )
        })
        .collect()
}

/// Seções do `.xl` (chaves de `XlFile::secoes`) que o parser lê e que nenhum emissor deste módulo grava: o runtime não as vê. O classify
/// separa a nota delas da nota das seções que o runtime aplica; seção que ganhar emissor sai daqui
/// (`tests::secoes_sem_tabela_nao_geram_linha`).
pub const SECOES_SEM_TABELA: [&str; 4] = ["localization.extend", "player.bodyTypes", "animations", "overrides.tags"];

/// As entradas de um `.xl` que o runtime não aplica, uma linha cada: as que o parse leu e o BWMS ainda não aplica
/// (`XlFile::sem_efeito_no_bwms`) e os grupos de `localization` (`localizacao_sem_efeito`). São os avisos de limite que o classify também
/// dá (`ModReport::avisos_do_xl`); a gravação das tabelas os leva a `apply::TabelasDosXl::sem_efeito_no_bwms`.
pub fn entradas_sem_efeito(xl: &XlFile) -> Vec<String> {
    xl.sem_efeito_no_bwms.iter().cloned().chain(localizacao_sem_efeito(xl)).collect()
}

/// As seções de um `.xl` que o runtime não aplica, uma linha cada: as que o parser lê e nenhuma tabela leva (`SECOES_SEM_TABELA`, com o
/// texto da nota do classify) e as que ele não reconhece (`XlFile::other_sections`). O classify as dá nas notas; a gravação das tabelas as leva a
/// `apply::TabelasDosXl::secoes_sem_efeito`, que só o relatório da `Mods/` imprime (lá o classify não roda).
pub fn secoes_sem_efeito(xl: &XlFile) -> Vec<String> {
    let secoes = xl.secoes();
    let sem_tabela = secoes
        .iter()
        .filter(|s| s.entradas > 0 && SECOES_SEM_TABELA.contains(&s.chave))
        .map(|s| format!("{} ×{}: o BWMS ainda não aplica esta seção (sem efeito no jogo); nada a corrigir no arquivo", s.chave, s.entradas));
    let nao_reconhecidas = xl.other_sections.iter().map(|s| format!("{s}: seção que o BWMS não reconhece (sem efeito no jogo); confira o nome da seção, e se o mod roda no Windows não há o que corrigir"));
    sem_tabela.chain(nao_reconhecidas).collect()
}

/// Tabela do `customizations` (ArchiveXL `#47`) pro runtime: uma linha `<male|female> | <caminho>` (`tabelas::Customizacao`) por recurso de opções de
/// customização. O runtime pede cada recurso na carga das tabelas e mescla nos recursos base na abertura do criador ou do espelho.
pub fn emit_customization_table(xl: &XlFile, _mod_id: &str) -> String {
    let mut out = tabelas::CUSTOMIZATIONS.cabecalho();
    if let Some(c) = &xl.customization {
        for (feminino, lista) in [(false, &c.male_options), (true, &c.female_options)] {
            for caminho in lista.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
                tabelas::Customizacao { feminino, caminho }.gravar(&mut out);
            }
        }
    }
    out
}

/// Listas do `resource.scope` de todos os `.xl` juntos, pelo hash do caminho do escopo: os alvos em texto, com o fecho transitivo (escopo dentro de escopo
/// vira os alvos dele, `resolver`). É o que o `ResourceMetaExtension` da referência monta juntando os escopos de todas as configs; como lá, o escopo só
/// existe com ao menos um alvo diferente dele. A grafia de cada caminho é a do primeiro `.xl` que o cita (o runtime re-hasheia o texto). O caminho nulo
/// (vazio, só espaço, só barras) não chega aqui: o parse do `.xl` tira a entrada ou o alvo, com aviso.
///
/// Devolve também, em ordem, o caminho de cada escopo cuja busca passou por um ciclo: ele fica só com os alvos alcançáveis fora do ciclo (A↔B dá A e B
/// vazios, como a referência; no ciclo indireto o laço dela não sai).
pub fn resource_scope_listas(xls: &[&XlFile]) -> (HashMap<u64, Vec<String>>, Vec<String>) {
    let h = |s: &str| crate::resource_path_hash(s.trim());
    let mut grafo: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut caminhos: HashMap<u64, &str> = HashMap::new();
    for xl in xls {
        for sc in &xl.scopes {
            let escopo = h(&sc.resource);
            caminhos.entry(escopo).or_insert(sc.resource.trim());
            for alvo in sc.targets.iter().map(|t| t.trim()) {
                let a = h(alvo);
                if a == escopo {
                    continue;
                }
                caminhos.entry(a).or_insert(alvo);
                let lista = grafo.entry(escopo).or_default();
                if !lista.contains(&a) {
                    lista.push(a);
                }
            }
        }
    }
    let mut listas: HashMap<u64, Vec<String>> = HashMap::new();
    let mut em_ciclo: Vec<String> = Vec::new();
    for &escopo in grafo.keys() {
        let (folhas, ciclo) = resolver(&grafo, escopo);
        if ciclo {
            em_ciclo.extend(caminhos.get(&escopo).map(|c| c.to_string()));
        }
        listas.insert(escopo, folhas.iter().filter_map(|f| caminhos.get(f).map(|c| c.to_string())).collect());
    }
    em_ciclo.sort();
    (listas, em_ciclo)
}

/// `caminhos` com cada escopo trocado pela lista dele em `scopes` (`resource_scope_listas`), o `ResourceMetaExtension::ExpandList` da referência
/// (`ResourceMeta/Extension.cpp:117-135`): escopo com alvos vira os alvos; caminho que não é escopo, ou escopo de lista vazia (o A↔B), fica ele mesmo.
/// Sem repetição pelo hash, porque a referência devolve um `Set`; a ordem é a da primeira ocorrência. Cada caminho sai com o hash dele, calculado
/// uma vez aqui, para quem filtra não re-hashear. O caminho vem sem os espaços das pontas, e o que fica vazio sai: o `resource.patch`
/// não passa pela regra do caminho nulo do parse. Única expansão de escopo do plano: `resource.patch` (includes e excludes), lado pedido do
/// `resource.link` e cópias do `resource.copy`.
fn expandir_escopo(caminhos: &[String], scopes: &HashMap<u64, Vec<String>>) -> Vec<(u64, String)> {
    let h = |s: &str| resource_path_hash(s.trim());
    let mut vistos: HashSet<u64> = HashSet::new();
    let mut v: Vec<(u64, String)> = Vec::new();
    for c in caminhos.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
        let hc = h(c);
        match scopes.get(&hc).filter(|l| !l.is_empty()) {
            Some(lista) => {
                for a in lista {
                    let ha = h(a);
                    if vistos.insert(ha) {
                        v.push((ha, a.clone()));
                    }
                }
            }
            None => {
                if vistos.insert(hc) {
                    v.push((hc, c.to_string()));
                }
            }
        }
    }
    v
}

/// Emite o conteúdo de `red4ext/bwms-resscopes.txt` a partir dos escopos de todos os `.xl` já expandidos (`resource_scope_listas`): uma linha por alvo,
/// `<hash do caminho do escopo> | <caminho do alvo>` (`tabelas::Resscope`), em ordem de escopo. É a tabela do `ResourceMetaExtension::InScope` em
/// runtime (ArchiveXL `#20` 20e-2, o escopo `player_customization.app` do `FixCustomizationAppearance`).
pub fn emit_resscopes(scopes: &HashMap<u64, Vec<String>>) -> String {
    let mut out = tabelas::RESSCOPES.cabecalho();
    let mut escopos: Vec<&u64> = scopes.keys().collect();
    escopos.sort();
    for escopo in escopos {
        for alvo in &scopes[escopo] {
            tabelas::Resscope { escopo: *escopo, alvo }.gravar(&mut out);
        }
    }
    out
}

/// Uma linha da `bwms-resource-patch.txt` antes das regras entre `.xl`, com o texto que vai para o arquivo e o hash desse texto.
struct LinhaDePatch {
    mod_id: String,
    patch: String,
    hash_do_patch: u64,
    alvo: String,
    hash_do_alvo: u64,
    props: u32,
}

/// Emite o conteúdo de `red4ext/bwms-resource-patch.txt` a partir de todos os `.xl` do install juntos, `(id do .xl, .xl)` na ordem do install (ArchiveXL
/// `#17`, `resource.patch`): uma linha por par patch → alvo, no formato `<mod> | <caminho do patch> | <caminho do alvo> | <props>`
/// (`tabelas::ResourcePatch`). O runtime só lê.
///
/// Dentro de cada `.xl`, como o `Configure` da referência: cada include e cada exclude que é escopo vira a lista dele (`scopes`, de todos os `.xl`
/// instalados, por `expandir_escopo`), senão entra o próprio caminho; alvos = includes menos excludes menos o próprio patch, pelo hash do caminho.
/// Cada `streaming.blocks` entra como patch do streaming world principal com a prop `blocks` (o `RegisterPatch` da extensão de streaming da
/// referência; ArchiveXL `#51` 51e).
///
/// Entre os `.xl`, também como o `Configure`: alvo que é patch de qualquer linha sai; o par (patch, alvo) entra uma vez, com o mod da 1ª ocorrência
/// (`knownPatches`); as props do patch são as da última linha dele, mesmo de uma linha que essas regras tiraram (`s_patches[patchPath]` sobrescrito).
/// O hash de patch e alvo nessas regras é o do texto gravado, que é o que o runtime hasheia.
///
/// `props` = máscara em hex (`0x…`, bits `crate::A17_PROP_*`) ou `-` com todas ligadas (props vazio: a referência trata vazio como "tudo"); nome
/// desconhecido não liga bit (`crate::a17_props_mascara`). `|` vira `_` no id do `.xl` e nos caminhos (`tabelas::campo`); linha de patch vazio sai.
pub fn emit_resource_patch_table(xls: &[(String, XlFile)], scopes: &HashMap<u64, Vec<String>>) -> String {
    let mut linhas: Vec<LinhaDePatch> = Vec::new();
    let mut linha = |mod_id: &str, patch: &str, hash_do_patch: u64, alvo: &str, props: u32| {
        let alvo = tabelas::campo(alvo);
        let hash_do_alvo = resource_path_hash(&alvo);
        linhas.push(LinhaDePatch { mod_id: mod_id.to_string(), patch: patch.to_string(), hash_do_patch, alvo, hash_do_alvo, props });
    };
    for (mod_id, xl) in xls {
        for p in xl.patches.iter().filter(|p| !p.patch.trim().is_empty()) {
            let patch = p.patch.trim();
            let props = if p.props.is_empty() { u32::MAX } else { crate::a17_props_mascara(&p.props.join(" ")) };
            let fora: HashSet<u64> = expandir_escopo(&p.excludes, scopes).into_iter().map(|(h, _)| h).chain(std::iter::once(resource_path_hash(patch))).collect();
            let gravado = tabelas::campo(patch);
            let hash_do_patch = resource_path_hash(&gravado);
            for (_, alvo) in expandir_escopo(&p.includes, scopes).into_iter().filter(|(h, _)| !fora.contains(h)) {
                linha(mod_id, &gravado, hash_do_patch, &alvo, props);
            }
        }
        if let Some(streaming) = &xl.streaming {
            for bloco in streaming.blocks.iter().map(|b| tabelas::campo(b.trim())).filter(|b| !b.is_empty()) {
                linha(mod_id, &bloco, resource_path_hash(&bloco), MUNDO_PRINCIPAL, crate::A17_PROP_BLOCKS);
            }
        }
    }
    let patches: HashSet<u64> = linhas.iter().map(|l| l.hash_do_patch).collect();
    let props_do_patch: HashMap<u64, u32> = linhas.iter().map(|l| (l.hash_do_patch, l.props)).collect();
    let mut vistos: HashSet<(u64, u64)> = HashSet::new();
    let mut out = tabelas::RESOURCE_PATCH.cabecalho();
    for l in linhas.iter().filter(|l| !patches.contains(&l.hash_do_alvo) && vistos.insert((l.hash_do_patch, l.hash_do_alvo))) {
        let props = props_do_patch.get(&l.hash_do_patch).copied().unwrap_or(l.props);
        tabelas::ResourcePatch { mod_id: &l.mod_id, patch: &l.patch, alvo: &l.alvo, props }.gravar(&mut out);
    }
    out
}

/// O streaming world em que os `streaming.blocks` entram, o mesmo da referência.
const MUNDO_PRINCIPAL: &str = "base\\worlds\\03_night_city\\_compiled\\default\\03_night_city.streamingworld";

/// `(op, valores)` de cada linha de uma mutação de node (`streaming.sectors[].nodeMutations`): `pos x y z w`, `rot i j k r`, `scale x y z`,
/// `res <hash do path>`, `app <CName>`, `rec <TweakDBID>`, `proxy <diff>` e, por sub-node, `subpos`/`subrot`/`subscale` com `<esperados> <índice>`
/// antes dos valores. Os hashes saem prontos em hex, então o runtime só confere e grava.
fn linhas_de_mutacao(m: &crate::instalacao::xl::WorldNodeMutation) -> Vec<(&'static str, String)> {
    let mut v = Vec::new();
    if let Some(p) = &m.position {
        v.push(("pos", format!("{} {} {} {}", p.x, p.y, p.z, p.w)));
    }
    if let Some(q) = &m.orientation {
        v.push(("rot", format!("{} {} {} {}", q.i, q.j, q.k, q.r)));
    }
    if let Some(s) = &m.scale {
        v.push(("scale", format!("{} {} {}", s.x, s.y, s.z)));
    }
    if let Some(r) = &m.resource_path {
        v.push(("res", format!("{:#018x}", crate::resource_path_hash(r))));
    }
    if let Some(a) = &m.appearance_name {
        v.push(("app", format!("{:#018x}", crate::fnv1a64(a.as_bytes()))));
    }
    if let Some(r) = &m.record_id {
        v.push(("rec", format!("{:#018x}", id_de_record(r))));
    }
    if let Some(d) = m.nb_nodes_under_proxy_diff {
        v.push(("proxy", d.to_string()));
    }
    for s in &m.sub_node_mutations {
        let cab = format!("{} {}", m.expected_sub_nodes, s.sub_node_index);
        if let Some(p) = &s.position {
            v.push(("subpos", format!("{cab} {} {} {} {}", p.x, p.y, p.z, p.w)));
        }
        if let Some(q) = &s.orientation {
            v.push(("subrot", format!("{cab} {} {} {} {}", q.i, q.j, q.k, q.r)));
        }
        if let Some(e) = &s.scale {
            v.push(("subscale", format!("{cab} {} {} {}", e.x, e.y, e.z)));
        }
    }
    v
}

/// TweakDBID de um `recordID` do `.xl`: número (`0x` hex ou decimal) direto; senão, o hash do nome.
fn id_de_record(r: &str) -> u64 {
    let t = r.trim();
    t.strip_prefix("0x")
        .and_then(|h| u64::from_str_radix(h, 16).ok())
        .or_else(|| t.parse::<u64>().ok())
        .unwrap_or_else(|| crate::tweak_db_id(t))
}

#[cfg(test)]
mod streaming_tests {
    use super::{emit_questphase_table, emit_streaming_table};
    use crate::instalacao::xl::parse_xl;

    #[test]
    fn delecao_de_node_vira_linha_da_tabela() {
        let xl = parse_xl(
            "streaming:\n  sectors:\n    - path: base\\worlds\\03_night_city\\_compiled\\default\\teste.streamingsector\n      expectedNodes: 20\n      nodeDeletions:\n        - index: 18\n          type: worldStaticMeshNode\n",
        )
        .expect("parse");
        let t = emit_streaming_table(&xl, "mod|teste");
        let linhas: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            linhas,
            vec!["mod_teste | base\\worlds\\03_night_city\\_compiled\\default\\teste.streamingsector | 20 | 18 | worldStaticMeshNode"]
        );
    }

    #[test]
    fn subdelecao_e_mutacao_viram_linhas_de_7_colunas() {
        let xl = parse_xl(
            "streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeDeletions:\n        - type: worldInstancedMeshNode\n          index: 1\n          expectedInstances: 4\n          instanceDeletions:\n            - 0\n            - 2\n      nodeMutations:\n        - type: worldMeshNode\n          index: 3\n          position: [1.0, 2.0, 3.0]\n          scale: [2.0, 2.0, 2.0]\n          appearance: rusty\n          recordID: Items.X\n          nbNodesUnderProxyDiff: 1\n",
        )
        .expect("parse");
        let t = emit_streaming_table(&xl, "m");
        let linhas: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
        let base = "m | s.streamingsector | 5";
        assert!(linhas.contains(&format!("{base} | 1 | worldInstancedMeshNode | subdel | 4 0 2").as_str()), "{linhas:?}");
        assert!(linhas.contains(&format!("{base} | 3 | worldMeshNode | pos | 1 2 3 0").as_str()), "{linhas:?}");
        assert!(linhas.contains(&format!("{base} | 3 | worldMeshNode | scale | 2 2 2").as_str()), "{linhas:?}");
        let app = format!("{base} | 3 | worldMeshNode | app | {:#018x}", crate::fnv1a64(b"rusty"));
        assert!(linhas.contains(&app.as_str()), "{linhas:?}");
        let rec = format!("{base} | 3 | worldMeshNode | rec | {:#018x}", crate::tweak_db_id("Items.X"));
        assert!(linhas.contains(&rec.as_str()), "{linhas:?}");
        assert!(linhas.contains(&format!("{base} | 3 | worldMeshNode | proxy | 1").as_str()), "{linhas:?}");
        assert_eq!(linhas.len(), 6, "{linhas:?}");
    }

    #[test]
    fn sem_streaming_so_cabecalho() {
        let xl = parse_xl("factories:\n  - base\\teste.csv\n").expect("parse");
        assert_eq!(emit_streaming_table(&xl, "x").lines().filter(|l| !l.starts_with('#')).count(), 0);
    }

    #[test]
    fn fase_de_mod_vira_uma_linha_de_8_colunas() {
        let xl = parse_xl(
            "quest:\n  phases:\n    - path: mod\\fase.questphase\n      parent: base\\quest\\cyberpunk2077.questphase\n      input:\n        node: [9, 17]\n        socket: CutDestination\n      output: [9, 21]\n      intercept: true\n    - path: mod\\outra.questphase\n      parent: base\\quest\\pai.questphase\n",
        )
        .expect("parse");
        let t = emit_questphase_table(&xl, "mod|q");
        let linhas: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            linhas,
            vec![
                "mod_q | base\\quest\\cyberpunk2077.questphase | mod\\fase.questphase | 9 17 | CutDestination | 9 21 | - | 1",
                "mod_q | base\\quest\\pai.questphase | mod\\outra.questphase |  | - |  | - | 0",
            ]
        );
    }

    #[test]
    fn sem_quest_phases_so_cabecalho() {
        let xl = parse_xl("factories:\n  - base\\teste.csv\n").expect("parse");
        assert_eq!(emit_questphase_table(&xl, "x").lines().filter(|l| !l.starts_with('#')).count(), 0);
    }

    #[test]
    fn journal_de_mod_vira_uma_linha_por_arquivo() {
        let xl = parse_xl("journal:\n  - mod\\a.journal\n  - mod\\b.journal\n").expect("parse");
        let t = super::emit_journal_table(&xl, "mod|j");
        let linhas: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(linhas, vec!["mod_j | mod\\a.journal", "mod_j | mod\\b.journal"]);
    }

    #[test]
    fn customizations_viram_uma_linha_por_recurso_e_genero() {
        let xl = parse_xl("customizations:\n  male:\n    - mod\\m1.inkcharcustomization\n    - mod\\m2.inkcharcustomization\n  female: mod\\f.inkcharcustomization\n")
            .expect("parse");
        let t = super::emit_customization_table(&xl, "x");
        let linhas: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(linhas, vec!["male | mod\\m1.inkcharcustomization", "male | mod\\m2.inkcharcustomization", "female | mod\\f.inkcharcustomization"]);
        let vazio = parse_xl("factories:\n  - base\\teste.csv\n").expect("parse");
        assert_eq!(super::emit_customization_table(&vazio, "x").lines().filter(|l| !l.starts_with('#')).count(), 0);
    }

    #[test]
    fn sem_journal_so_cabecalho() {
        let xl = parse_xl("factories:\n  - base\\teste.csv\n").expect("parse");
        assert_eq!(super::emit_journal_table(&xl, "x").lines().filter(|l| !l.starts_with('#')).count(), 0);
    }

    /// Linhas da `bwms-resource-patch.txt` de um `.xl` só, com os escopos dele, sem o cabeçalho.
    fn patch_de_um_xl(xl: &crate::instalacao::xl::XlFile, mod_id: &str) -> Vec<String> {
        let (scopes, _) = super::resource_scope_listas(&[xl]);
        super::emit_resource_patch_table(&[(mod_id.to_string(), xl.clone())], &scopes).lines().filter(|l| !l.starts_with('#')).map(String::from).collect()
    }

    #[test]
    fn resource_patch_vira_uma_linha_por_alvo() {
        let xl = parse_xl(
            "resource:\n  patch:\n    mod\\patch.ent:\n      props: [ components ]\n      targets: [ base\\a.ent, base\\b.ent, mod\\patch.ent, base\\a.ent ]\n    mod\\outro.ent: base\\c.ent\n    mod\\so_exclui.ent: !exclude [ base\\x.ent ]\n",
        )
        .expect("parse");
        assert_eq!(
            patch_de_um_xl(&xl, "mod|p"),
            vec![
                "mod_p | mod\\patch.ent | base\\a.ent | 0x1".to_string(),
                "mod_p | mod\\patch.ent | base\\b.ent | 0x1".to_string(),
                "mod_p | mod\\outro.ent | base\\c.ent | -".to_string(),
            ]
        );
    }

    #[test]
    fn props_do_patch_saem_em_mascara_hex_e_nome_desconhecido_nao_liga_bit() {
        let xl = parse_xl(
            "resource:\n  patch:\n    mod\\a.app:\n      props: [ appearances, partsValues, censorshipMapping, partsOverrides ]\n      targets: [ base\\a.app ]\n    mod\\b.morphtarget:\n      props: [ baseMesh, baseTexture, blob, targets, renderResourceBlob ]\n      targets: [ base\\b.morphtarget ]\n    mod\\c.ent:\n      props: [ visualTags, nada ]\n      targets: [ base\\c.ent ]\n    mod\\d.ent:\n      props: [ nada ]\n      targets: [ base\\d.ent ]\n",
        )
        .expect("parse");
        assert_eq!(
            patch_de_um_xl(&xl, "m"),
            vec![
                "m | mod\\a.app | base\\a.app | 0xe4".to_string(),
                "m | mod\\b.morphtarget | base\\b.morphtarget | 0x1f000".to_string(),
                "m | mod\\c.ent | base\\c.ent | 0x10".to_string(),
                "m | mod\\d.ent | base\\d.ent | 0x0".to_string(),
            ]
        );
    }

    #[test]
    fn alvo_que_e_patch_de_outro_xl_sai_e_par_repetido_fica_uma_vez_com_as_props_do_ultimo() {
        // ModA mira `modb\p.ent`, que é patch do ModB: sai. O par (mod\p.ent, base\x.ent) vem dos dois `.xl`: fica o 1º, com o mod dele e as props
        // da última linha de `mod\p.ent`, a do ModB.
        let a = parse_xl(
            "resource:\n  patch:\n    mod\\p.ent:\n      props: [ components ]\n      targets: [ base\\x.ent, modb\\p.ent ]\n",
        )
        .expect("parse");
        let b = parse_xl(
            "resource:\n  patch:\n    modb\\p.ent: base\\z.ent\n    MOD/P.ENT:\n      props: [ appearances ]\n      targets: [ base\\x.ent, base\\y.ent ]\n",
        )
        .expect("parse");
        let (scopes, _) = super::resource_scope_listas(&[&a, &b]);
        let t = super::emit_resource_patch_table(&[("ModA/a".to_string(), a), ("ModB/b".to_string(), b)], &scopes);
        let linhas: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(linhas, vec!["ModA/a | mod\\p.ent | base\\x.ent | 0x4", "ModB/b | modb\\p.ent | base\\z.ent | -", "ModB/b | MOD/P.ENT | base\\y.ent | 0x4"]);
    }

    #[test]
    fn props_do_patch_vem_da_ultima_linha_mesmo_de_linha_que_saiu() {
        // A última linha de `mod\p.ent` mira `mod\q.ent`, que é patch: sai da tabela, mas as props dela (todas) valem para o patch.
        let xl = parse_xl(
            "resource:\n  patch:\n    mod\\p.ent:\n      props: [ components ]\n      targets: [ base\\a.ent ]\n    mod\\q.ent: base\\b.ent\n",
        )
        .expect("parse");
        let outro = parse_xl("resource:\n  patch:\n    mod\\p.ent: mod\\q.ent\n").expect("parse");
        let t = super::emit_resource_patch_table(&[("m".to_string(), xl), ("n".to_string(), outro)], &Default::default());
        let linhas: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(linhas, vec!["m | mod\\p.ent | base\\a.ent | -", "m | mod\\q.ent | base\\b.ent | -"]);
    }

    #[test]
    fn streaming_blocks_viram_patch_do_mundo_principal() {
        let xl = parse_xl("streaming:\n  blocks:\n    - mod\\bwms\\a.streamingblock\n    - mod\\bwms\\b.streamingblock\n").expect("parse");
        assert_eq!(
            patch_de_um_xl(&xl, "m"),
            vec![
                "m | mod\\bwms\\a.streamingblock | base\\worlds\\03_night_city\\_compiled\\default\\03_night_city.streamingworld | 0x800".to_string(),
                "m | mod\\bwms\\b.streamingblock | base\\worlds\\03_night_city\\_compiled\\default\\03_night_city.streamingworld | 0x800".to_string(),
            ]
        );
    }

    #[test]
    fn sem_resource_patch_so_cabecalho() {
        let xl = parse_xl("factories:\n  - base\\teste.csv\n").expect("parse");
        assert!(patch_de_um_xl(&xl, "x").is_empty());
    }

    #[test]
    fn resource_patch_expande_scope_com_fecho_transitivo_e_sem_repetir_alvo() {
        let xl = parse_xl(
            "resource:\n  scope:\n    mod\\todas.ent:\n      - base\\a.ent\n      - mod\\meio.ent\n    mod\\meio.ent:\n      - base\\c.ent\n  patch:\n    mod\\patch.ent:\n      props: [ components ]\n      targets: [ mod\\todas.ent, base\\d.ent, base\\a.ent ]\n",
        )
        .expect("parse");
        assert_eq!(
            patch_de_um_xl(&xl, "m"),
            vec![
                "m | mod\\patch.ent | base\\a.ent | 0x1".to_string(),
                "m | mod\\patch.ent | base\\c.ent | 0x1".to_string(),
                "m | mod\\patch.ent | base\\d.ent | 0x1".to_string(),
            ]
        );
    }

    #[test]
    fn resource_patch_usa_scope_de_outro_xl() {
        let escopos = parse_xl("resource:\n  scope:\n    mod\\grupo.ent:\n      - base\\x.ent\n      - base\\y.ent\n").expect("parse");
        let mod_xl = parse_xl("resource:\n  patch:\n    mod\\p.ent: mod\\grupo.ent\n").expect("parse");
        let (scopes, _) = super::resource_scope_listas(&[&escopos, &mod_xl]);
        let com: Vec<String> = super::emit_resource_patch_table(&[("m".to_string(), mod_xl.clone())], &scopes).lines().filter(|l| !l.starts_with('#')).map(String::from).collect();
        assert_eq!(com, vec!["m | mod\\p.ent | base\\x.ent | -".to_string(), "m | mod\\p.ent | base\\y.ent | -".to_string()]);
        assert_eq!(patch_de_um_xl(&mod_xl, "m"), vec!["m | mod\\p.ent | mod\\grupo.ent | -".to_string()]);
    }

    #[test]
    fn expandir_escopo_da_cada_hash_uma_vez_na_ordem_da_primeira_grafia() {
        // `base/x.ent` tem o hash de `base\x.ent`, que o escopo já trouxe: fica a primeira grafia, com o hash que o filtro do patch consulta.
        let xl = parse_xl("resource:\n  scope:\n    mod\\g.ent: [base\\x.ent, base\\y.ent]\n").expect("parse");
        let (scopes, _) = super::resource_scope_listas(&[&xl]);
        let entrada: Vec<String> = ["  BASE\\Y.ENT ", " ", "mod\\g.ent", "base/x.ent", "base\\z.ent"].iter().map(|s| s.to_string()).collect();
        let h = crate::resource_path_hash;
        assert_eq!(
            super::expandir_escopo(&entrada, &scopes),
            vec![
                (h("base\\y.ent"), "BASE\\Y.ENT".to_string()),
                (h("base\\x.ent"), "base\\x.ent".to_string()),
                (h("base\\z.ent"), "base\\z.ent".to_string()),
            ]
        );
    }

    #[test]
    fn resource_patch_tira_exclude_e_o_proprio_patch_em_qualquer_grafia() {
        let patch = crate::instalacao::xl::ResourcePatch {
            patch: " mod\\p.ent ".to_string(),
            props: vec![],
            includes: ["base\\a.ent", "MOD/P.ENT", "base\\b.ent", "base\\c.ent", "BASE\\A.ENT"].iter().map(|s| s.to_string()).collect(),
            excludes: vec!["base/B.ent".to_string()],
        };
        let xl = crate::instalacao::xl::XlFile { patches: vec![patch], ..Default::default() };
        assert_eq!(patch_de_um_xl(&xl, "m"), vec!["m | mod\\p.ent | base\\a.ent | -".to_string(), "m | mod\\p.ent | base\\c.ent | -".to_string()]);
    }

    #[test]
    fn resscopes_emite_os_escopos_expandidos_de_todos_os_xl() {
        let a = parse_xl("resource:\n  scope:\n    player_customization.app:\n      - player_ma_beard.app\n").expect("parse");
        let b = parse_xl("resource:\n  scope:\n    player_ma_beard.app:\n      - base\\b1.app\n      - base\\b2.app\n").expect("parse");
        let (scopes, _) = super::resource_scope_listas(&[&a, &b]);
        let mut linhas: Vec<String> = super::emit_resscopes(&scopes).lines().filter(|l| !l.starts_with('#')).map(String::from).collect();
        let (cust, barba) = (crate::resource_path_hash("player_customization.app"), crate::resource_path_hash("player_ma_beard.app"));
        let mut esperado = vec![
            format!("{cust:#018x} | base\\b1.app"),
            format!("{cust:#018x} | base\\b2.app"),
            format!("{barba:#018x} | base\\b1.app"),
            format!("{barba:#018x} | base\\b2.app"),
        ];
        linhas.sort();
        esperado.sort();
        assert_eq!(linhas, esperado);
        assert!(super::emit_resscopes(&Default::default()).lines().all(|l| l.starts_with('#')));
    }

    #[test]
    fn escopo_em_ciclo_indireto_da_o_mesmo_resultado_em_toda_ordem() {
        // `b` e `c` se apontam e `a` alcança o ciclo e a folha `x`. Cada chamada monta HashMap com RandomState novo: 64 voltas cobrem as 3 ordens de chave.
        let xl = parse_xl("resource:\n  scope:\n    a: [b, x]\n    b: c\n    c: b\n").expect("parse");
        let h = crate::resource_path_hash;
        for _ in 0..64 {
            let (listas, em_ciclo) = super::resource_scope_listas(&[&xl]);
            assert_eq!(listas[&h("a")], vec!["x".to_string()], "{listas:?}");
            assert!(listas[&h("b")].is_empty() && listas[&h("c")].is_empty(), "{listas:?}");
            assert!(listas.values().flatten().all(|alvo| !listas.contains_key(&h(alvo))), "alvo que ainda é escopo: {listas:?}");
            assert_eq!(em_ciclo, vec!["a", "b", "c"]);
        }
    }

    #[test]
    fn escopo_a_b_a_fica_vazio_como_na_referencia_e_sai_no_diagnostico() {
        // A referência apaga o alvo antes de ler o conjunto dele e termina com A e B vazios; o patch que mira A cai no próprio caminho de A.
        let xl = parse_xl("resource:\n  scope:\n    a: b\n    b: a\n  patch:\n    p.ent: a\n").expect("parse");
        let (listas, em_ciclo) = super::resource_scope_listas(&[&xl]);
        let h = crate::resource_path_hash;
        assert!(listas[&h("a")].is_empty() && listas[&h("b")].is_empty(), "{listas:?}");
        assert_eq!(em_ciclo, vec!["a", "b"]);
        assert!(super::emit_resscopes(&listas).lines().all(|l| l.starts_with('#')));
        assert_eq!(patch_de_um_xl(&xl, "m"), vec!["m | p.ent | a | -".to_string()]);
    }

    #[test]
    fn escopo_em_diamante_nao_e_ciclo() {
        let xl = parse_xl("resource:\n  scope:\n    a: [b, c]\n    b: d\n    c: d\n    d: [x, y]\n").expect("parse");
        let (listas, em_ciclo) = super::resource_scope_listas(&[&xl]);
        assert_eq!(listas[&crate::resource_path_hash("a")], vec!["x".to_string(), "y".to_string()]);
        assert!(em_ciclo.is_empty(), "{em_ciclo:?}");
    }

    #[test]
    fn escopo_com_mais_de_256_subescopos_expande_todos() {
        let mut y = String::from("resource:\n  scope:\n    a:\n");
        for i in 0..300 {
            y.push_str(&format!("      - s{i}\n"));
        }
        for i in 0..300 {
            y.push_str(&format!("    s{i}: f{i}\n"));
        }
        let xl = parse_xl(&y).expect("parse");
        let (listas, _) = super::resource_scope_listas(&[&xl]);
        let esperado: Vec<String> = (0..300).map(|i| format!("f{i}")).collect();
        assert_eq!(listas[&crate::resource_path_hash("a")], esperado);
    }

    #[test]
    fn escopo_sem_alvo_alem_de_si_mesmo_nao_e_escopo() {
        // O `Configure` da referência só cria o conjunto do escopo no insert de um alvo diferente dele: `b: b` não faz de `b` um escopo, e `a` fica com `b`.
        let xl = parse_xl("resource:\n  scope:\n    a: b\n    b: b\n").expect("parse");
        let (listas, _) = super::resource_scope_listas(&[&xl]);
        let h = crate::resource_path_hash;
        assert_eq!(listas[&h("a")], vec!["b".to_string()], "{listas:?}");
        assert!(!listas.contains_key(&h("b")), "{listas:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instalacao::xl::{ResourceCopy, ResourceLink, XlFile};

    /// Goldens REAIS do índice de `basegame_zzbwms_quadra_e3.archive` (via archive-tool datamap).
    /// Provam que `resource_path_hash` É o hash do CP2077 (5/5 batem).
    #[test]
    fn hash_goldens() {
        let g = [
            ("base\\surfaces\\materials\\default\\white.xbm", 0x3959_590b_0d88_8df1u64),
            ("base\\vehicles\\common\\wheels\\wheel_quadra_turbo_clean_01.mlsetup", 0xc007_6ea3_a1d9_a9edu64),
            ("base\\surfaces\\microblends\\default.xbm", 0xf78f_7031_24ea_fb22u64),
            ("base\\surfaces\\materials\\metal\\metal_generic\\metal_generic_bare_01_300_d.xbm", 0x79c9_5673_c0d4_c73au64),
            ("base\\resource.cooked_mlsetup", 0x3a12_b4fd_1938_d5cau64),
        ];
        for (p, h) in g {
            assert_eq!(resource_path_hash(p), h, "hash de {p}");
        }
    }

    /// `/` vira `\` e case não importa → o path canônico e o "sujo" dão o MESMO hash.
    #[test]
    fn hash_normaliza_sep_e_case() {
        let canon = resource_path_hash("base\\surfaces\\materials\\default\\white.xbm");
        assert_eq!(resource_path_hash("BASE/Surfaces/Materials/Default/White.xbm"), canon);
    }

    /// ArchiveXL `#26`: o loader consulta só os links e o depot só as cópias. Com um mapa só, um `resource.copy` responderia uma requisição de
    /// LOADER — coisa que o ArchiveXL nunca faz. A união (`OnDepotResourceCheck`) é do runtime (`depot.rs`, `check_responde_pela_uniao_e_o_original_nao`).
    #[test]
    fn link_e_copy_ficam_em_mapas_separados() {
        let mut xl = XlFile::default();
        xl.links.push(ResourceLink { pedido: "a.mesh".into(), servido: "b.mesh".into() });
        xl.copies.push(ResourceCopy { source: "c.mesh".into(), targets: vec!["d.mesh".into()] });
        let p = build_apply_plan(&xl);
        let (a, b) = (resource_path_hash("a.mesh"), resource_path_hash("b.mesh"));
        let (c, d) = (resource_path_hash("c.mesh"), resource_path_hash("d.mesh"));

        assert_eq!(p.mapa_links.get(&a), Some(&b));
        assert_eq!(p.mapa_links.get(&d), None, "copy NÃO pode responder no loader");
        assert_eq!(p.mapa_copies.get(&d), Some(&c));
        assert_eq!(p.mapa_copies.get(&a), None, "link NÃO pode responder no depot");
    }

    /// Link cujo pedido é cópia sai do plano (`finalCopies.contains(linkPath)`, `ResourceLink/Extension.cpp:99-107`): o loader e o depot nunca
    /// respondem pelo mesmo caminho, e o motivo vai para `avisos`, não para `cyclic_links`.
    #[test]
    fn link_cujo_pedido_e_copia_sai_do_plano_com_aviso() {
        let mut xl = XlFile::default();
        xl.links.push(ResourceLink { pedido: "x.mesh".into(), servido: "via_link.mesh".into() });
        xl.copies.push(ResourceCopy { source: "via_copy.mesh".into(), targets: vec!["x.mesh".into()] });
        let p = build_apply_plan(&xl);
        let x = resource_path_hash("x.mesh");
        assert_eq!(p.mapa_links.get(&x), None);
        assert_eq!(p.mapa_copies.get(&x), Some(&resource_path_hash("via_copy.mesh")));
        assert_eq!((p.mapa_links.len(), p.mapa_copies.len()), (0, 1));
        assert!(p.cyclic_links.is_empty(), "{:?}", p.cyclic_links);
        assert_eq!(
            p.avisos,
            vec!["resource.link x.mesh: o pedido é um resource.copy e fica fora de bwms-reslink.txt; tire esse caminho do link ou do copy no .xl"]
        );
        assert_eq!(emit_reslink(&xl), "# bwms-reslink (gerado de .xl). Formato: <path_pedido> | <path_servido> [| copy]\nx.mesh | via_copy.mesh | copy\n");
    }

    /// A cópia que tem link sai do grafo antes da cadeia: `a: k` continua `a → k` (a referência aceita link para cópia, `Extension.cpp:177`) e não
    /// atravessa `k: z`. A cópia vem de outro `.xl`, depois do link, e conta igual: a referência junta as cópias de todas as configs primeiro.
    #[test]
    fn link_sobre_copia_sai_antes_da_cadeia_e_copy_de_outro_xl_conta() {
        let links = crate::instalacao::xl::parse_xl("resource:\n  link:\n    mod\\a.mesh: mod\\k.mesh\n    mod\\k.mesh: base\\z.mesh\n").expect("parse");
        let copies = crate::instalacao::xl::parse_xl("resource:\n  copy:\n    base\\o.mesh: mod\\k.mesh\n").expect("parse");
        let p = build_apply_plan_combinado(&[&links, &copies], &HashMap::new());
        assert_eq!(
            emit_reslink_do_plano(&p),
            "# bwms-reslink (gerado de .xl). Formato: <path_pedido> | <path_servido> [| copy]\nmod\\a.mesh | mod\\k.mesh\nmod\\k.mesh | base\\o.mesh | copy\n"
        );
        assert_eq!(p.avisos.len(), 1, "{:?}", p.avisos);
        assert!(p.avisos[0].starts_with("resource.link mod\\k.mesh: o pedido é um resource.copy"), "{}", p.avisos[0]);
        assert!(p.cyclic_links.is_empty(), "{:?}", p.cyclic_links);
        assert_eq!(reslink_relido(&emit_reslink_do_plano(&p)), (p.mapa_links.clone(), p.mapa_copies.clone()));
    }

    /// Cópia igual ao original sai (`Copy "…" points to itself`, `Extension.cpp:55-63`) sem derrubar as outras cópias da mesma entrada, e o
    /// aviso só sai quando nenhuma outra entrada pôs o caminho na tabela. O link para esse caminho continua: ele não é cópia.
    #[test]
    fn copy_para_si_mesmo_sai_com_aviso_e_nao_derruba_o_resto() {
        let xl = crate::instalacao::xl::parse_xl(
            "resource:\n  copy:\n    mod\\s.mesh: [mod\\s.mesh, mod\\c.mesh]\n    base\\o.mesh: [mod\\d.mesh]\n    mod\\d.mesh: [mod\\d.mesh]\n  link:\n    mod\\s.mesh: base\\z.mesh\n",
        )
        .expect("parse");
        let p = build_apply_plan(&xl);
        assert_eq!(
            emit_reslink(&xl),
            "# bwms-reslink (gerado de .xl). Formato: <path_pedido> | <path_servido> [| copy]\nmod\\s.mesh | base\\z.mesh\nmod\\c.mesh | mod\\s.mesh | copy\nmod\\d.mesh | base\\o.mesh | copy\n"
        );
        assert_eq!(
            p.avisos,
            vec!["resource.copy mod\\s.mesh: a cópia é o próprio original e fica fora de bwms-reslink.txt; tire esse caminho da lista de cópias no .xl"]
        );
    }

    #[test]
    fn plan_redirects_link_e_copy() {
        let mut xl = XlFile::default();
        xl.links.push(ResourceLink { pedido: "base\\fake\\a.mesh".into(), servido: "base\\real\\b.mesh".into() });
        xl.copies.push(ResourceCopy {
            source: "base\\real\\c.mesh".into(),
            targets: vec!["base\\new\\d.mesh".into(), "base\\new\\e.mesh".into()],
        });
        let p = build_apply_plan(&xl);
        assert_eq!(p.mapa_links.len(), 1);
        assert_eq!(p.mapa_copies.len(), 2);
        assert_eq!(p.mapa_links[&resource_path_hash("base\\fake\\a.mesh")], resource_path_hash("base\\real\\b.mesh"));
        assert_eq!(p.mapa_copies[&resource_path_hash("base\\new\\d.mesh")], resource_path_hash("base\\real\\c.mesh"));
    }

    #[test]
    fn emit_localization_table_uma_linha_por_recurso() {
        let src = "localization:\n  onscreens:\n    en-us: meumod\\\\loc\\\\en-us.json\n    pt-br: meumod\\\\loc\\\\pt-br.json\n";
        let xl = crate::instalacao::xl::parse_xl(src).expect("parse");
        let t = emit_localization_table(&xl, "meumod");
        let linhas: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(linhas.len(), 2, "{t}");
        assert!(linhas.contains(&"meumod | onscreens | en-us | meumod\\\\loc\\\\en-us.json"), "{t}");
        assert!(linhas.contains(&"meumod | onscreens | pt-br | meumod\\\\loc\\\\pt-br.json"), "{t}");
    }

    #[test]
    fn localizacao_que_o_runtime_nao_aplica_vira_um_aviso_por_grupo_e_continua_na_tabela() {
        let src = "localization:\n  onscreens:\n    en-us: m\\on.json\n  subtitles:\n    pt-br: m\\sub.json\n  lipmaps:\n    en-us: m\\lip.json\n  vomaps:\n    en-us: m\\vo.json\n";
        let xl = crate::instalacao::xl::parse_xl(src).expect("parse");
        assert!(xl.issues.is_empty(), "{:?}", xl.issues);
        let aviso = |k: &str| {
            format!(
                "localization.{k}: não aplicado pelo runtime do BWMS, que só aplica onscreens, e o grupo fica sem efeito no jogo; tire o grupo {k} do localization deste .xl para o aviso sumir (o jogo fica igual)"
            )
        };
        assert_eq!(localizacao_sem_efeito(&xl), vec![aviso("subtitles"), aviso("lipmaps"), aviso("vomaps")]);
        // a tabela continua com os 4 tipos: o filtro é do runtime
        let t = emit_localization_table(&xl, "m");
        let linhas: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(linhas, vec!["m | onscreens | en-us | m\\on.json", "m | subtitles | pt-br | m\\sub.json", "m | lipmaps | en-us | m\\lip.json", "m | vomaps | en-us | m\\vo.json"]);
        let so_onscreens = crate::instalacao::xl::parse_xl("localization:\n  onscreens:\n    en-us: m\\on.json\n").expect("parse");
        assert!(localizacao_sem_efeito(&so_onscreens).is_empty());
    }

    /// O emissor produz linhas que, RE-HASHADAS como o runtime faz (`cp77-console`, `arquivos_do_jogo::reslink_file`),
    /// reproduzem EXATO o mapa de `build_apply_plan` → prova o round-trip emissor↔runtime sem jogo.
    #[test]
    fn emit_reslink_reproduz_o_plano() {
        let mut xl = XlFile::default();
        xl.links.push(ResourceLink { pedido: "base\\fake\\a.mesh".into(), servido: "base\\real\\b.mesh".into() });
        xl.copies.push(ResourceCopy {
            source: "base\\real\\c.mesh".into(),
            targets: vec!["base\\new\\d.mesh".into(), "base\\new\\e.mesh".into()],
        });
        let plan = build_apply_plan(&xl);
        let (links, copies) = reslink_relido(&emit_reslink(&xl));
        assert_eq!(links, plan.mapa_links);
        assert_eq!(copies, plan.mapa_copies);
        assert_eq!(links.len() + copies.len(), 3);
    }

    /// Lê o texto do `bwms-reslink.txt` pelo leitor do runtime (`tabelas::Reslink`, o de `arquivos_do_jogo::reslink_file`), com re-hash dos 2 lados;
    /// a linha `copy` vai para o mapa do depot, e o último par de uma chave vence.
    fn reslink_relido(texto: &str) -> (HashMap<u64, u64>, HashMap<u64, u64>) {
        let (mut links, mut copies) = (HashMap::new(), HashMap::new());
        for l in tabelas::ler(texto, tabelas::Reslink::ler) {
            let l = l.expect("linha que o emissor grava se lê");
            let mapa = if l.copia { &mut copies } else { &mut links };
            mapa.insert(resource_path_hash(l.pedido), resource_path_hash(l.servido));
        }
        (links, copies)
    }

    #[test]
    fn emit_reslink_aplica_a_cadeia_e_tira_o_ciclo_como_o_plano() {
        let mut xl = XlFile::default();
        for (t, s) in [("a", "b"), ("b", "c"), ("x", "y"), ("y", "x")] {
            xl.links.push(ResourceLink { pedido: t.into(), servido: s.into() });
        }
        let plan = build_apply_plan(&xl);
        let h = resource_path_hash;
        let esperado: HashMap<u64, u64> = [(h("a"), h("c")), (h("b"), h("c"))].into_iter().collect();
        assert_eq!(plan.mapa_links, esperado);
        let (links, copies) = reslink_relido(&emit_reslink(&xl));
        assert_eq!(links, esperado);
        assert!(copies.is_empty());
    }

    /// Guarda da direção escalar x sequência no texto emitido: o runtime tem de reconstruir o mesmo `mapa_links` do plano.
    #[test]
    fn emit_reslink_mantem_a_direcao_escalar_e_sequencia() {
        let xl = crate::instalacao::xl::parse_xl("resource:\n  link:\n    fake: real\n    novo: [antigo1, antigo2]\n").expect("parse");
        let plan = build_apply_plan(&xl);
        let h = resource_path_hash;
        let esperado: HashMap<u64, u64> = [(h("fake"), h("real")), (h("antigo1"), h("novo")), (h("antigo2"), h("novo"))].into_iter().collect();
        assert_eq!(plan.mapa_links, esperado);
        assert_eq!(reslink_relido(&emit_reslink(&xl)).0, esperado);
    }

    /// Direção escalar x sequência de `resource.link` contra a referência (`ResourceLink/Config.cpp:25-48` + `finalLinks[linkPath] = targetPath` de
    /// `Extension.cpp:85-113`): o escalar pede a chave e serve o valor; a sequência pede cada item e serve a chave (a migração do `Migration.xl`).
    /// O parse entrega os pares já orientados, na ordem do YAML, e o plano só os aplica.
    #[test]
    fn link_direcao_scalar_vs_sequencia_bate_com_cpp() {
        let xl = crate::instalacao::xl::parse_xl(
            "resource:\n  link:\n    base\\fake\\a.mesh: base\\real\\b.mesh\n    base\\novo\\h1.mesh: [base\\antigo\\base.mesh, base\\antigo\\legacy.mesh]\n",
        )
        .expect("parse");
        let par = |pedido: &str, servido: &str| ResourceLink { pedido: pedido.into(), servido: servido.into() };
        assert_eq!(
            xl.links,
            vec![
                par("base\\fake\\a.mesh", "base\\real\\b.mesh"),
                par("base\\antigo\\base.mesh", "base\\novo\\h1.mesh"),
                par("base\\antigo\\legacy.mesh", "base\\novo\\h1.mesh"),
            ]
        );
        let p = build_apply_plan(&xl);
        assert_eq!(p.mapa_links.len(), 3);
        let h = resource_path_hash;
        for l in &xl.links {
            assert_eq!(p.mapa_links.get(&h(&l.pedido)), Some(&h(&l.servido)), "{l:?}");
        }
        assert_eq!(p.mapa_links.get(&h("base\\novo\\h1.mesh")), None, "o servido da sequência não é pedido");
    }

    /// O trecho do `Migration.xl` real pelo parse e pelo plano, com a tabela emitida inteira: o item (nome legado) é pedido e a chave é servida.
    #[test]
    fn link_migration_real_end_to_end_direcao_correta() {
        let src = "resource:\n  link:\n    archive_xl\\a\\h1.mesh:\n      - archive_xl\\a\\base.mesh\n";
        let xl = crate::instalacao::xl::parse_xl(src).expect("parse");
        assert_eq!(xl.links, vec![ResourceLink { pedido: "archive_xl\\a\\base.mesh".into(), servido: "archive_xl\\a\\h1.mesh".into() }]);
        assert_eq!(
            emit_reslink(&xl),
            "# bwms-reslink (gerado de .xl). Formato: <path_pedido> | <path_servido> [| copy]\narchive_xl\\a\\base.mesh | archive_xl\\a\\h1.mesh\n"
        );
        let p = build_apply_plan(&xl);
        // "base.mesh" (nome antigo, na lista) deve resolver pro "h1.mesh" (o alvo/path novo).
        assert_eq!(
            p.mapa_links[&resource_path_hash("archive_xl\\a\\base.mesh")],
            resource_path_hash("archive_xl\\a\\h1.mesh")
        );
    }

    /// **`axl-link-nton-table` (2026-07-13).** Cadeia A→B→C colapsa pro alvo FINAL (C), igual
    /// `Extension.cpp::Configure()` (persegue `finalLinks[target]` até não achar mais chave,
    /// substitui `link.value()` pelo alvo final) — replicado em `resolver`.
    #[test]
    fn link_cadeia_a_para_b_para_c_colapsa_no_final() {
        let mut xl = XlFile::default();
        xl.links.push(ResourceLink { pedido: "a".into(), servido: "b".into() });
        xl.links.push(ResourceLink { pedido: "b".into(), servido: "c".into() });
        let p = build_apply_plan(&xl);
        assert!(p.cyclic_links.is_empty());
        // a -> b -> c colapsa em a -> c (não em a -> b)
        assert_eq!(p.mapa_links[&resource_path_hash("a")], resource_path_hash("c"));
        // b -> c continua valendo por si só (é o alvo final da própria cadeia de b)
        assert_eq!(p.mapa_links[&resource_path_hash("b")], resource_path_hash("c"));
    }

    /// Ciclo A→B→A é detectado e REMOVIDO (não aplicado) — fiel ao C++ (`LogError` + `erase`).
    #[test]
    fn link_ciclo_e_detectado_e_removido() {
        let mut xl = XlFile::default();
        xl.links.push(ResourceLink { pedido: "a".into(), servido: "b".into() });
        xl.links.push(ResourceLink { pedido: "b".into(), servido: "a".into() });
        let p = build_apply_plan(&xl);
        assert_eq!(p.cyclic_links, vec!["a".to_string(), "b".to_string()]);
        assert!(p.mapa_links.is_empty(), "{:?}", p.mapa_links);
    }

    /// Uma cadeia longa (A→B→C→D) resolve até o fim, sem confundir com ciclo.
    #[test]
    fn link_cadeia_longa_resolve_ate_o_fim_sem_falso_ciclo() {
        let mut xl = XlFile::default();
        for (t, s) in [("a", "b"), ("b", "c"), ("c", "d")] {
            xl.links.push(ResourceLink { pedido: t.into(), servido: s.into() });
        }
        let p = build_apply_plan(&xl);
        assert!(p.cyclic_links.is_empty());
        assert_eq!(p.mapa_links[&resource_path_hash("a")], resource_path_hash("d"));
    }

    /// Quem chega a um ciclo sem estar nele também sai (o C++ marca `a` como cíclico ao revisitar `b`), e a resolução termina.
    #[test]
    fn link_que_chega_a_um_ciclo_sai_junto() {
        let mut xl = XlFile::default();
        for (t, s) in [("a", "b"), ("b", "c"), ("c", "b"), ("x", "y")] {
            xl.links.push(ResourceLink { pedido: t.into(), servido: s.into() });
        }
        let p = build_apply_plan(&xl);
        assert_eq!(p.cyclic_links, vec!["a", "b", "c"]);
        assert_eq!(p.mapa_links, HashMap::from([(resource_path_hash("x"), resource_path_hash("y"))]));
    }

    /// `ResourceLink/Extension.cpp` descarta `b: b` (`points to itself`) antes de montar `finalLinks`, então `a: b` continua valendo.
    #[test]
    fn link_para_si_mesmo_sai_sem_derrubar_quem_aponta_para_ele() {
        let xl = crate::instalacao::xl::parse_xl("resource:\n  link:\n    a: b\n    b: b\n").expect("parse");
        let p = build_apply_plan(&xl);
        assert_eq!(p.mapa_links, HashMap::from([(resource_path_hash("a"), resource_path_hash("b"))]));
        assert_eq!(p.cyclic_links, vec!["b"]);
        assert_eq!(reslink_relido(&emit_reslink(&xl)).0, p.mapa_links);
    }

    /// O pedido passa por `caminho` primeiro nas duas formas: o link para si mesmo escrito com outra caixa sai com a grafia do pedido.
    #[test]
    fn link_para_si_mesmo_sai_com_a_grafia_do_pedido() {
        let xl = crate::instalacao::xl::parse_xl("resource:\n  link:\n    Real.mesh: [real.MESH]\n    a.mesh: A.MESH\n").expect("parse");
        let p = build_apply_plan(&xl);
        assert_eq!(p.cyclic_links, vec!["a.mesh", "real.MESH"]);
        assert!(p.mapa_links.is_empty(), "{:?}", p.mapa_links);
    }

    /// Vários `.xl` num plano só: a cadeia passa de um para o outro, e no pedido repetido vence o `.xl` que vem depois.
    #[test]
    fn plano_combinado_resolve_cadeia_entre_xl_e_o_ultimo_xl_vence() {
        let um = crate::instalacao::xl::parse_xl("resource:\n  link:\n    a: b\n    r: s1\n  copy:\n    o1: [c]\n").expect("parse");
        let dois = crate::instalacao::xl::parse_xl("resource:\n  link:\n    b: c\n    r: s2\n  copy:\n    o2: [c]\n").expect("parse");
        let p = build_apply_plan_combinado(&[&um, &dois], &HashMap::new());
        let h = resource_path_hash;
        assert_eq!(p.mapa_links, HashMap::from([(h("a"), h("c")), (h("b"), h("c")), (h("r"), h("s2"))]));
        assert_eq!(p.mapa_copies, HashMap::from([(h("c"), h("o2"))]));
        let linhas: Vec<String> = emit_reslink_do_plano(&p).lines().filter(|l| !l.starts_with('#')).map(String::from).collect();
        assert_eq!(linhas, vec!["a | c", "b | c", "r | s2", "c | o2 | copy"]);
    }

    /// `ResourceLink/Extension.cpp:53,87`: o escopo vira os alvos dele no lado pedido do link (item da sequência, chave do escalar) e na cópia do copy.
    /// O lado servido (`mod\fora.mesh: mod\seq.mesh`) e o original do copy ficam como estão, e nenhuma linha pede o caminho do escopo.
    #[test]
    fn link_e_copy_expandem_o_escopo_no_lado_pedido() {
        let xl = crate::instalacao::xl::parse_xl(
            "resource:\n  scope:\n    mod\\seq.mesh: [base\\x.mesh, base\\y.mesh]\n    mod\\esc.mesh: [base\\z.mesh]\n    mod\\copias.mesh: [mod\\c1.mesh, mod\\c2.mesh]\n  link:\n    mod\\real.mesh: [mod\\seq.mesh]\n    mod\\esc.mesh: mod\\real2.mesh\n    mod\\fora.mesh: mod\\seq.mesh\n  copy:\n    base\\orig.mesh: [mod\\copias.mesh]\n",
        )
        .expect("parse");
        assert!(xl.issues.is_empty(), "{:?}", xl.issues);
        let linhas: Vec<String> = emit_reslink(&xl).lines().filter(|l| !l.starts_with('#')).map(String::from).collect();
        assert_eq!(
            linhas,
            vec![
                "base\\x.mesh | mod\\real.mesh",
                "base\\y.mesh | mod\\real.mesh",
                "base\\z.mesh | mod\\real2.mesh",
                "mod\\fora.mesh | mod\\seq.mesh",
                "mod\\c1.mesh | base\\orig.mesh | copy",
                "mod\\c2.mesh | base\\orig.mesh | copy",
            ]
        );
        let p = build_apply_plan(&xl);
        assert_eq!((p.mapa_links.len(), p.mapa_copies.len()), (4, 2));
        assert_eq!(reslink_relido(&emit_reslink(&xl)), (p.mapa_links, p.mapa_copies));
    }

    /// Como o `ExpandList`: escopo de lista vazia (o A↔B) e caminho que não é escopo ficam eles mesmos, e o link não some.
    #[test]
    fn escopo_vazio_e_caminho_comum_ficam_como_estao_no_link_e_no_copy() {
        let xl = crate::instalacao::xl::parse_xl("resource:\n  scope:\n    a: b\n    b: a\n  link:\n    a: real\n    comum: real\n  copy:\n    orig: [b]\n").expect("parse");
        assert!(resource_scope_listas(&[&xl]).0.values().all(|l| l.is_empty()));
        let linhas: Vec<String> = emit_reslink(&xl).lines().filter(|l| !l.starts_with('#')).map(String::from).collect();
        assert_eq!(linhas, vec!["a | real", "comum | real", "b | orig | copy"]);
    }

    /// O `points to itself` da referência confere o pedido já expandido: o membro do escopo igual ao servido sai sozinho, os outros ficam.
    #[test]
    fn membro_do_escopo_igual_ao_servido_sai_sem_derrubar_os_outros() {
        let xl = crate::instalacao::xl::parse_xl("resource:\n  scope:\n    g: [x, real]\n  link:\n    real: [g]\n").expect("parse");
        let p = build_apply_plan(&xl);
        assert_eq!(p.mapa_links, HashMap::from([(resource_path_hash("x"), resource_path_hash("real"))]));
        assert_eq!(p.cyclic_links, vec!["real"]);
    }

    /// O escopo de um `.xl` expande o link de outro no plano combinado (os escopos de todas as configs); o `.xl` do link sozinho não conhece o escopo.
    #[test]
    fn escopo_de_outro_xl_expande_o_link_no_plano_combinado() {
        let escopos = crate::instalacao::xl::parse_xl("resource:\n  scope:\n    mod\\grupo.mesh: [base\\x.mesh, base\\y.mesh]\n").expect("parse");
        let mod_xl = crate::instalacao::xl::parse_xl("resource:\n  link:\n    mod\\real.mesh: [mod\\grupo.mesh]\n").expect("parse");
        let p = build_apply_plan_combinado(&[&escopos, &mod_xl], &resource_scope_listas(&[&escopos, &mod_xl]).0);
        let linhas: Vec<String> = emit_reslink_do_plano(&p).lines().filter(|l| !l.starts_with('#')).map(String::from).collect();
        assert_eq!(linhas, vec!["base\\x.mesh | mod\\real.mesh", "base\\y.mesh | mod\\real.mesh"]);
        let so: Vec<String> = emit_reslink(&mod_xl).lines().filter(|l| !l.starts_with('#')).map(String::from).collect();
        assert_eq!(so, vec!["mod\\grupo.mesh | mod\\real.mesh"]);
    }

    // ===== resource.fix: emit_resfixes =====

    #[test]
    fn emit_resfixes_formato_correto() {
        let src = "resource:\n  fix:\n    my.mesh:\n      names:\n        old_mat: new_mat\n      paths:\n        a.mi: b.mi\n";
        let xl = crate::instalacao::xl::parse_xl(src).unwrap();
        let out = emit_resfixes(&xl);
        assert!(out.contains("my.mesh | names | old_mat | new_mat"));
        assert!(out.contains("my.mesh | paths | a.mi | b.mi"));
    }

    /// Um `.xl` só com as seções de `SECOES_SEM_TABELA` popula exatamente essas chaves de `XlFile::secoes` e não dá linha em nenhuma das
    /// tabelas (uma por emissor, as mesmas de `tabelas::TODAS`): a nota "o BWMS ainda não aplica" do classify é verdadeira.
    #[test]
    fn secoes_sem_tabela_nao_geram_linha() {
        let xl = crate::instalacao::xl::parse_xl(
            "overrides:\n  tags:\n    minha_tag:\n      t0_000_pma_base__full: [ 1 ]\nanimations:\n  - entity: player_ma_tpp\n    set: mod\\anims.anims\n\
             player:\n  bodyTypes: [ Corpo ]\nlocalization:\n  extend: mod\\loc.json\n",
        )
        .expect("parse");
        let mut lidas: Vec<&str> = xl.secoes().iter().filter(|s| s.entradas > 0).map(|s| s.chave).collect();
        lidas.sort_unstable();
        let mut sem_tabela = SECOES_SEM_TABELA.to_vec();
        sem_tabela.sort_unstable();
        assert_eq!(lidas, sem_tabela);
        let (scopes, _) = resource_scope_listas(&[&xl]);
        let emitidas = [
            emit_reslink(&xl),
            emit_streaming_table(&xl, "m"),
            emit_questphase_table(&xl, "m"),
            emit_journal_table(&xl, "m"),
            emit_localization_table(&xl, "m"),
            emit_resource_patch_table(&[("m".to_string(), xl.clone())], &scopes),
            emit_resfixes(&xl),
            emit_resscopes(&scopes),
            emit_customization_table(&xl, "m"),
        ];
        assert_eq!(emitidas.len(), crate::tabelas::TODAS.len());
        for t in &emitidas {
            assert!(t.lines().all(|l| l.starts_with('#')), "{t}");
        }
    }

    /// As quatro classes do que o runtime não aplica: o que o parse leu sem aplicar e o grupo de `localization` nas entradas, a seção sem
    /// tabela e a não reconhecida nas seções. Um `.xl` só com o que tem tabela não dá linha.
    #[test]
    fn entradas_e_secoes_sem_efeito_cobrem_as_quatro_classes() {
        let xl = crate::instalacao::xl::parse_xl(
            "resource:\n  fix:\n    a.mesh:\n      names:\n        m1: m2\n      context:\n        p: v\n\
             localization:\n  onscreens:\n    en-us: m\\on.json\n  subtitles:\n    en-us: m\\sub.json\n\
             animations:\n  - entity: player_ma_tpp\n    set: mod\\anims.anims\ncustomNodes: x\n",
        )
        .expect("parse");
        let entradas = entradas_sem_efeito(&xl);
        // O `context` do `resource.fix` vai ao estado de mesh nativo (S12): não entra mais aqui.
        assert_eq!(entradas.len(), 1, "{entradas:?}");
        assert!(entradas[0].starts_with("localization.subtitles: não aplicado pelo runtime do BWMS"), "{}", entradas[0]);
        assert_eq!(
            secoes_sem_efeito(&xl),
            [
                "animations ×1: o BWMS ainda não aplica esta seção (sem efeito no jogo); nada a corrigir no arquivo",
                "customNodes: seção que o BWMS não reconhece (sem efeito no jogo); confira o nome da seção, e se o mod roda no Windows não há o que corrigir",
            ]
        );
        let so_com_tabela = crate::instalacao::xl::parse_xl("localization:\n  onscreens:\n    en-us: m\\on.json\n").expect("parse");
        assert!(entradas_sem_efeito(&so_com_tabela).is_empty());
        assert!(secoes_sem_efeito(&so_com_tabela).is_empty());
    }
}
