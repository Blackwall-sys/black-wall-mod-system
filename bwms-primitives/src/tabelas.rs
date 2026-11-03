//! As tabelas `red4ext/bwms-*.txt` entre o install e o runtime: o nome do arquivo, o cabeçalho e a linha de cada uma, definidos uma vez para
//! quem grava (`instalacao::tabelas_de_runtime`, `instalacao::apply`) e para quem lê (`cp77-console`, na carga das tabelas do boot e do
//! `ArchiveXL.Reload`).
//!
//! Regra das 9: uma linha por registro, colunas separadas por ` | `, e o cabeçalho do gerador começa com `#`. Na leitura, linha vazia e linha
//! que começa com `#` (depois dos espaços) ficam de fora, e cada coluna sai sem os espaços das pontas; linha com outro número de colunas, ou com
//! coluna que não lê, é recusada inteira. Na gravação, `|` dentro de uma coluna vira `_` (`campo`), porque a coluna não tem escape.

/// Uma tabela de `red4ext/` (`crate::PASTA_DO_RUNTIME`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tabela {
    /// Nome do arquivo dentro de `red4ext/`.
    pub arquivo: &'static str,
    /// As colunas da linha, como o cabeçalho dos emissores de `.xl` as descreve.
    pub formato: &'static str,
}

impl Tabela {
    /// O nome do arquivo sem `.txt`, que abre o cabeçalho.
    pub fn nome(&self) -> &'static str {
        self.arquivo.strip_suffix(".txt").unwrap_or(self.arquivo)
    }

    /// Começo do cabeçalho dos emissores de `.xl`. O install só apaga a tabela que ficou sem linha quando ela começa com a marca do gerador:
    /// arquivo posto à mão fica.
    pub fn marca_do_emissor(&self) -> String {
        format!("# {} (gerado de .xl)", self.nome())
    }

    /// Cabeçalho dos emissores de `.xl`, com o fim de linha.
    pub fn cabecalho(&self) -> String {
        format!("{}. Formato: {}\n", self.marca_do_emissor(), self.formato)
    }

    /// Começo do cabeçalho que o install grava no `bwms-reslink.txt`. O do emissor fica para a saída do `bwms xl`,
    /// que vai para `red4ext/` à mão.
    pub fn marca_do_install(&self) -> String {
        format!("# {} (gerado automaticamente pelo install", self.nome())
    }

    /// Cabeçalho do install com a contagem (`"2 par(es)"`), com o fim de linha.
    pub fn cabecalho_do_install(&self, contagem: &str) -> String {
        format!("{} — {contagem})\n", self.marca_do_install())
    }
}

pub const RESLINK: Tabela = Tabela { arquivo: "bwms-reslink.txt", formato: "<path_pedido> | <path_servido> [| copy]" };
pub const STREAMING: Tabela = Tabela { arquivo: "bwms-streaming.txt", formato: "<mod> | <setor> | <expectedNodes> | <nodeIndex> | <tipo>" };
pub const QUESTPHASE: Tabela = Tabela {
    arquivo: "bwms-questphase.txt",
    formato: "<mod> | <fase pai> | <fase do mod> | <nó de entrada> | <socket de entrada> | <nó de saída> | <socket de saída> | <intercept>",
};
pub const JOURNAL: Tabela = Tabela { arquivo: "bwms-journal.txt", formato: "<mod> | <caminho do .journal>" };
pub const LOCALIZACAO: Tabela = Tabela { arquivo: "bwms-localizacao.txt", formato: "<mod> | <tipo> | <idioma> | <caminho do recurso>" };
pub const RESOURCE_PATCH: Tabela = Tabela {
    arquivo: "bwms-resource-patch.txt",
    formato: "<mod> | <caminho do patch> | <caminho do alvo> | <props em máscara hex; - = todas>",
};
pub const RESFIXES: Tabela = Tabela { arquivo: "bwms-resfixes.txt", formato: "<resource> | names|paths|context | <old ou parâmetro> | <new ou valor>" };
pub const RESSCOPES: Tabela = Tabela { arquivo: "bwms-resscopes.txt", formato: "<hash do caminho do escopo> | <caminho do alvo>" };
pub const CUSTOMIZATIONS: Tabela = Tabela { arquivo: "bwms-customizations.txt", formato: "<male|female> | <caminho do recurso>" };

/// As 9, na ordem em que o install as grava.
pub const TODAS: [Tabela; 9] = [RESLINK, STREAMING, QUESTPHASE, JOURNAL, LOCALIZACAO, RESOURCE_PATCH, RESFIXES, RESSCOPES, CUSTOMIZATIONS];

/// Uma coluna de texto como a tabela a grava: `|` vira `_`. Quem hasheia o texto gravado (o `resource.patch` do install) hasheia isto.
pub fn campo(texto: &str) -> String {
    texto.replace('|', "_")
}

/// Cada linha com registro de `texto` lida por `linha` sobre as colunas dela. `Err` traz a linha recusada, para o log de quem lê.
pub fn ler<'a, T: 'a>(texto: &'a str, linha: fn(&[&'a str]) -> Option<T>) -> impl Iterator<Item = Result<T, &'a str>> + 'a {
    texto.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(move |l| {
        let colunas: Vec<&str> = l.split('|').map(str::trim).collect();
        linha(&colunas).ok_or(l)
    })
}

fn gravar_colunas(out: &mut String, colunas: &[&str]) {
    for (i, c) in colunas.iter().enumerate() {
        if i > 0 {
            out.push_str(" | ");
        }
        out.push_str(&campo(c));
    }
    out.push('\n');
}

/// Linha de `bwms-reslink.txt`: `resource.link` (`<pedido> | <servido>`, servido no loader) ou `resource.copy` (`<cópia> | <original> | copy`,
/// servida no depot; aqui `pedido` é a cópia e `servido` o original).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reslink<'a> {
    pub pedido: &'a str,
    pub servido: &'a str,
    pub copia: bool,
}

impl<'a> Reslink<'a> {
    pub fn ler(c: &[&'a str]) -> Option<Self> {
        match *c {
            [pedido, servido] => Some(Reslink { pedido, servido, copia: false }),
            [pedido, servido, "copy"] => Some(Reslink { pedido, servido, copia: true }),
            _ => None,
        }
    }

    pub fn gravar(&self, out: &mut String) {
        if self.copia {
            gravar_colunas(out, &[self.pedido, self.servido, "copy"]);
        } else {
            gravar_colunas(out, &[self.pedido, self.servido]);
        }
    }
}

/// Linha de `bwms-streaming.txt` (`streaming.sectors`, ArchiveXL `#49`): deleção ou mutação de um node de setor. O runtime agrupa por
/// (`mod_id`, setor) e um grupo com uma linha ruim é recusado inteiro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Streaming<'a> {
    pub mod_id: &'a str,
    pub setor: &'a str,
    pub esperado: usize,
    pub indice: usize,
    pub tipo: &'a str,
    /// Linha de 7 colunas: `(op, valores)` de sub-deleção (`subdel`) ou de mutação (`pos`, `rot`, `scale`, `res`, `app`, `rec`, `proxy`,
    /// `subpos`, `subrot`, `subscale`), com os valores em texto, que o runtime confere no setor vivo. `None` (5 colunas): o node inteiro sai.
    pub op: Option<(&'a str, &'a str)>,
}

impl<'a> Streaming<'a> {
    pub fn ler(c: &[&'a str]) -> Option<Self> {
        let (mod_id, setor, esperado, indice, tipo, op) = match *c {
            [m, s, e, i, t] => (m, s, e, i, t, None),
            [m, s, e, i, t, op, valores] => (m, s, e, i, t, Some((op, valores))),
            _ => return None,
        };
        Some(Streaming { mod_id, setor, esperado: esperado.parse().ok()?, indice: indice.parse().ok()?, tipo, op })
    }

    pub fn gravar(&self, out: &mut String) {
        let (esperado, indice) = (self.esperado.to_string(), self.indice.to_string());
        let base = [self.mod_id, self.setor, esperado.as_str(), indice.as_str(), self.tipo];
        match self.op {
            None => gravar_colunas(out, &base),
            Some((op, valores)) => gravar_colunas(out, &[&base[..], &[op, valores][..]].concat()),
        }
    }
}

/// Linha de `bwms-questphase.txt` (`quest.phases`, ArchiveXL `#35`): uma fase de mod pendurada numa fase do jogo. Os caminhos de recurso vão
/// em texto: o id do nó novo sai do CRC-16 de `"<fase do mod>:<id do nó de entrada>"`, e o id do nó de entrada só se sabe no grafo quando o
/// caminho é vazio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Questphase<'a> {
    pub mod_id: &'a str,
    pub pai: &'a str,
    pub fase: &'a str,
    /// Ids do caminho do nó, separados por espaço na coluna; vazio (ou `-`) = o `questStartNodeDefinition` do grafo, como na referência.
    pub entrada: Vec<u16>,
    /// `None` grava `-`; `-` e a coluna vazia leem `None`.
    pub socket_entrada: Option<&'a str>,
    pub saida: Vec<u16>,
    pub socket_saida: Option<&'a str>,
    pub intercept: bool,
}

impl<'a> Questphase<'a> {
    pub fn ler(c: &[&'a str]) -> Option<Self> {
        let [mod_id, pai, fase, entrada, socket_entrada, saida, socket_saida, intercept] = *c else { return None };
        let ids = |s: &str| -> Option<Vec<u16>> {
            if s == "-" {
                Some(Vec::new())
            } else {
                s.split_whitespace().map(|x| x.parse::<u16>().ok()).collect()
            }
        };
        let socket = |s: &'a str| (!s.is_empty() && s != "-").then_some(s);
        Some(Questphase {
            mod_id,
            pai,
            fase,
            entrada: ids(entrada)?,
            socket_entrada: socket(socket_entrada),
            saida: ids(saida)?,
            socket_saida: socket(socket_saida),
            intercept: match intercept {
                "1" => true,
                "0" => false,
                _ => return None,
            },
        })
    }

    pub fn gravar(&self, out: &mut String) {
        let ids = |v: &[u16]| v.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(" ");
        let socket = |s: Option<&'a str>| s.filter(|s| !s.is_empty()).unwrap_or("-");
        let (entrada, saida) = (ids(&self.entrada), ids(&self.saida));
        let intercept = if self.intercept { "1" } else { "0" };
        gravar_colunas(
            out,
            &[self.mod_id, self.pai, self.fase, entrada.as_str(), socket(self.socket_entrada), saida.as_str(), socket(self.socket_saida), intercept],
        );
    }
}

/// Linha de `bwms-journal.txt` (`journal`, ArchiveXL `#44`): um `.journal` de mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Journal<'a> {
    pub mod_id: &'a str,
    pub caminho: &'a str,
}

impl<'a> Journal<'a> {
    pub fn ler(c: &[&'a str]) -> Option<Self> {
        match *c {
            [mod_id, caminho] if !caminho.is_empty() => Some(Journal { mod_id, caminho }),
            _ => None,
        }
    }

    pub fn gravar(&self, out: &mut String) {
        gravar_colunas(out, &[self.mod_id, self.caminho]);
    }
}

/// Linha de `bwms-localizacao.txt` (`localization`, ArchiveXL `#45`): um recurso de texto do mod num idioma, na ordem do `.xl` (o 1º idioma de
/// cada mod é a reserva no runtime).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Localizacao<'a> {
    pub mod_id: &'a str,
    pub tipo: &'a str,
    pub idioma: &'a str,
    pub caminho: &'a str,
}

impl<'a> Localizacao<'a> {
    pub fn ler(c: &[&'a str]) -> Option<Self> {
        match *c {
            [mod_id, tipo, idioma, caminho] => Some(Localizacao { mod_id, tipo, idioma, caminho }),
            _ => None,
        }
    }

    pub fn gravar(&self, out: &mut String) {
        gravar_colunas(out, &[self.mod_id, self.tipo, self.idioma, self.caminho]);
    }
}

/// Linha de `bwms-resource-patch.txt` (`resource.patch`, ArchiveXL `#17`): um par patch → alvo, já resolvido entre os mods pelo install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourcePatch<'a> {
    pub mod_id: &'a str,
    pub patch: &'a str,
    pub alvo: &'a str,
    /// Bits `crate::A17_PROP_*`; `u32::MAX` (todas) grava `-`, o props vazio da referência. Na leitura, a coluna em hex (`0x…`) ou, numa
    /// tabela posta à mão, os nomes (`crate::a17_props_mascara`).
    pub props: u32,
}

impl<'a> ResourcePatch<'a> {
    pub fn ler(c: &[&'a str]) -> Option<Self> {
        match *c {
            [mod_id, patch, alvo, props] if !patch.is_empty() && !alvo.is_empty() => Some(ResourcePatch {
                mod_id,
                patch,
                alvo,
                props: props.strip_prefix("0x").and_then(|h| u32::from_str_radix(h, 16).ok()).unwrap_or_else(|| crate::a17_props_mascara(props)),
            }),
            _ => None,
        }
    }

    pub fn gravar(&self, out: &mut String) {
        let props = if self.props == u32::MAX { "-".to_string() } else { format!("{:#x}", self.props) };
        gravar_colunas(out, &[self.mod_id, self.patch, self.alvo, props.as_str()]);
    }
}

/// Coluna 2 de `bwms-resfixes.txt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoDeFix {
    /// `names`: nome de material do mesh, trocado no `PostLoad` do `CMesh`.
    Nomes,
    /// `paths`: recurso de aparência de uma opção de customização (`FixCustomizationOptions`).
    Caminhos,
    /// `context`: parâmetro de contexto do mesh dinâmico (`PrepareMeshState(mesh, fix.GetContext())` da referência); antigo = nome, novo = valor.
    Contexto,
}

/// Nome do atributo de contexto do mesh a partir da chave do `context` (`Str::SnakeCase` da referência, que o `PrefillContext` aplica e o da 2.5
/// não): maiúscula vira `_` + minúscula, menos no começo e logo depois de `_`.
pub fn nome_de_atributo(chave: &str) -> String {
    let mut nome = String::with_capacity(chave.len() + 4);
    let mut separa = false;
    for c in chave.chars() {
        if c.is_ascii_uppercase() {
            if separa {
                nome.push('_');
                separa = false;
            }
            nome.push(c.to_ascii_lowercase());
        } else {
            nome.push(c);
            separa = c != '_';
        }
    }
    nome
}

/// Linha de `bwms-resfixes.txt` (`resource.fix`, ArchiveXL `#51` 51i): no recurso, o antigo vira o novo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resfix<'a> {
    pub recurso: &'a str,
    pub tipo: TipoDeFix,
    pub antigo: &'a str,
    pub novo: &'a str,
}

impl<'a> Resfix<'a> {
    pub fn ler(c: &[&'a str]) -> Option<Self> {
        let [recurso, tipo, antigo, novo] = *c else { return None };
        let tipo = match tipo {
            "names" => TipoDeFix::Nomes,
            "paths" => TipoDeFix::Caminhos,
            "context" => TipoDeFix::Contexto,
            _ => return None,
        };
        (!recurso.is_empty() && !antigo.is_empty()).then_some(Resfix { recurso, tipo, antigo, novo })
    }

    pub fn gravar(&self, out: &mut String) {
        let tipo = match self.tipo {
            TipoDeFix::Nomes => "names",
            TipoDeFix::Caminhos => "paths",
            TipoDeFix::Contexto => "context",
        };
        gravar_colunas(out, &[self.recurso, tipo, self.antigo, self.novo]);
    }
}

/// Linha de `bwms-resscopes.txt` (`resource.scope`, ArchiveXL `#20` 20e-2): um alvo de um escopo já expandido, com o escopo pelo hash do
/// caminho (`crate::resource_path_hash`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resscope<'a> {
    pub escopo: u64,
    pub alvo: &'a str,
}

impl<'a> Resscope<'a> {
    pub fn ler(c: &[&'a str]) -> Option<Self> {
        match *c {
            [escopo, alvo] if !alvo.is_empty() => Some(Resscope { escopo: u64::from_str_radix(escopo.trim_start_matches("0x"), 16).ok()?, alvo }),
            _ => None,
        }
    }

    pub fn gravar(&self, out: &mut String) {
        let escopo = format!("{:#018x}", self.escopo);
        gravar_colunas(out, &[escopo.as_str(), self.alvo]);
    }
}

/// Linha de `bwms-customizations.txt` (`customizations`, ArchiveXL `#47`): um recurso de opções de customização do corpo masculino ou feminino.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Customizacao<'a> {
    pub feminino: bool,
    pub caminho: &'a str,
}

impl<'a> Customizacao<'a> {
    pub fn ler(c: &[&'a str]) -> Option<Self> {
        let [genero, caminho] = *c else { return None };
        let feminino = match genero {
            "male" => false,
            "female" => true,
            _ => return None,
        };
        (!caminho.is_empty()).then_some(Customizacao { feminino, caminho })
    }

    pub fn gravar(&self, out: &mut String) {
        gravar_colunas(out, &[if self.feminino { "female" } else { "male" }, self.caminho]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Snake_case da referência e o hash dos nomes que o mesh nativo da 2.5 usa (constantes dos inicializadores `0x101ce97b8` e `0x101ce9644`).
    #[test]
    fn nome_de_atributo_e_hashes_do_mesh_nativo() {
        assert_eq!(nome_de_atributo("appearanceExpansionSource"), "appearance_expansion_source");
        assert_eq!(nome_de_atributo("DreadBaseMaterial"), "dread_base_material");
        assert_eq!(nome_de_atributo("ja_snake"), "ja_snake");
        assert_eq!(nome_de_atributo("a_B"), "a_b");
        assert_eq!(crate::fnv1a64(b"appearance_expansion_source"), 0x1af8_3fd9_60c9_0ff3);
        assert_eq!(crate::fnv1a64(b"@context"), 0xd0e6_8e7f_5139_6a06);
    }

    /// Grava cada linha e lê o texto de volta: as mesmas linhas, e o texto é o `esperado`.
    fn ida_e_volta<'a, T: std::fmt::Debug + PartialEq + 'a>(linhas: &[T], gravar: fn(&T, &mut String), ler_linha: fn(&[&'a str]) -> Option<T>, esperado: &'a str) {
        let mut texto = String::new();
        for l in linhas {
            gravar(l, &mut texto);
        }
        assert_eq!(texto, esperado);
        let lidas: Vec<T> = ler(esperado, ler_linha).collect::<Result<_, _>>().expect("toda linha gravada se lê");
        assert_eq!(&lidas[..], linhas);
    }

    #[test]
    fn ida_e_volta_das_9_tabelas() {
        ida_e_volta(
            &[Reslink { pedido: "base\\fake\\a.mesh", servido: "base\\real\\a.mesh", copia: false }, Reslink { pedido: "mod\\copia.mesh", servido: "base\\orig.mesh", copia: true }],
            Reslink::gravar,
            Reslink::ler,
            "base\\fake\\a.mesh | base\\real\\a.mesh\nmod\\copia.mesh | base\\orig.mesh | copy\n",
        );
        ida_e_volta(
            &[
                Streaming { mod_id: "ModB/beta.archive", setor: "s.streamingsector", esperado: 5, indice: 1, tipo: "worldStaticMeshNode", op: None },
                Streaming { mod_id: "m", setor: "s.streamingsector", esperado: 5, indice: 2, tipo: "worldInstancedMeshNode", op: Some(("subdel", "4 0 3")) },
                Streaming { mod_id: "m", setor: "s.streamingsector", esperado: 5, indice: 3, tipo: "worldStaticMeshNode", op: Some(("pos", "1 2 3 1")) },
            ],
            Streaming::gravar,
            Streaming::ler,
            "ModB/beta.archive | s.streamingsector | 5 | 1 | worldStaticMeshNode\n\
             m | s.streamingsector | 5 | 2 | worldInstancedMeshNode | subdel | 4 0 3\n\
             m | s.streamingsector | 5 | 3 | worldStaticMeshNode | pos | 1 2 3 1\n",
        );
        ida_e_volta(
            &[
                Questphase {
                    mod_id: "ModB/beta.archive",
                    pai: "base\\quest\\pai.questphase",
                    fase: "modb\\fase.questphase",
                    entrada: Vec::new(),
                    socket_entrada: None,
                    saida: Vec::new(),
                    socket_saida: None,
                    intercept: false,
                },
                Questphase {
                    mod_id: "m",
                    pai: "base\\pai.questphase",
                    fase: "m\\f.questphase",
                    entrada: vec![17, 3],
                    socket_entrada: Some("CutDestination"),
                    saida: vec![9],
                    socket_saida: Some("In"),
                    intercept: true,
                },
            ],
            Questphase::gravar,
            Questphase::ler,
            "ModB/beta.archive | base\\quest\\pai.questphase | modb\\fase.questphase |  | - |  | - | 0\n\
             m | base\\pai.questphase | m\\f.questphase | 17 3 | CutDestination | 9 | In | 1\n",
        );
        ida_e_volta(&[Journal { mod_id: "ModA/alfa", caminho: "moda\\q.journal" }], Journal::gravar, Journal::ler, "ModA/alfa | moda\\q.journal\n");
        ida_e_volta(
            &[Localizacao { mod_id: "ModA/alfa", tipo: "onscreens", idioma: "en-us", caminho: "moda\\\\loc\\\\en-us.json" }],
            Localizacao::gravar,
            Localizacao::ler,
            "ModA/alfa | onscreens | en-us | moda\\\\loc\\\\en-us.json\n",
        );
        ida_e_volta(
            &[
                ResourcePatch { mod_id: "ModB/beta.archive", patch: "modb\\p.ent", alvo: "base\\x.ent", props: crate::A17_PROP_COMPONENTS },
                ResourcePatch { mod_id: "ModB/beta.archive", patch: "modb\\p.ent", alvo: "base\\y.ent", props: u32::MAX },
                ResourcePatch { mod_id: "m", patch: "m\\p.ent", alvo: "base\\z.ent", props: 0 },
            ],
            ResourcePatch::gravar,
            ResourcePatch::ler,
            "ModB/beta.archive | modb\\p.ent | base\\x.ent | 0x1\nModB/beta.archive | modb\\p.ent | base\\y.ent | -\nm | m\\p.ent | base\\z.ent | 0x0\n",
        );
        ida_e_volta(
            &[
                Resfix { recurso: "my.mesh", tipo: TipoDeFix::Nomes, antigo: "old_mat", novo: "new_mat" },
                Resfix { recurso: "base.app", tipo: TipoDeFix::Caminhos, antigo: "antigo.app", novo: "" },
            ],
            Resfix::gravar,
            Resfix::ler,
            "my.mesh | names | old_mat | new_mat\nbase.app | paths | antigo.app | \n",
        );
        ida_e_volta(
            &[Resscope { escopo: 0x1100_0209_e259_5c59, alvo: "base\\x.ent" }, Resscope { escopo: 7, alvo: "base\\y.ent" }],
            Resscope::gravar,
            Resscope::ler,
            "0x11000209e2595c59 | base\\x.ent\n0x0000000000000007 | base\\y.ent\n",
        );
        ida_e_volta(
            &[Customizacao { feminino: false, caminho: "m\\m.inkcharcustomization" }, Customizacao { feminino: true, caminho: "moda\\f.inkcharcustomization" }],
            Customizacao::gravar,
            Customizacao::ler,
            "male | m\\m.inkcharcustomization\nfemale | moda\\f.inkcharcustomization\n",
        );
    }

    /// `|` numa coluna sai como `_`, e a linha continua com o número de colunas do formato.
    #[test]
    fn barra_numa_coluna_vira_sublinhado_e_a_linha_continua_legivel() {
        let mut texto = String::new();
        Journal { mod_id: "Mod|A/x", caminho: "a|b.journal" }.gravar(&mut texto);
        Reslink { pedido: "a|b.mesh", servido: "c.mesh", copia: true }.gravar(&mut texto);
        assert_eq!(texto, "Mod_A/x | a_b.journal\na_b.mesh | c.mesh | copy\n");
        assert_eq!(ler(&texto, Journal::ler).next(), Some(Ok(Journal { mod_id: "Mod_A/x", caminho: "a_b.journal" })));
        assert_eq!(campo("x|y|z"), "x_y_z");
    }

    /// Cabeçalho, comentário com espaço antes do `#` e linha vazia ficam de fora; linha com coluna a mais, a menos ou que não lê volta em `Err` com
    /// o texto dela, sem os espaços das pontas.
    #[test]
    fn leitura_pula_comentario_e_recusa_a_linha_fora_do_formato() {
        let texto = "# bwms-journal (gerado de .xl)\n\n   # comentário | x\n m | a.journal \nm | a | b\nm |  \nsó uma coluna\n";
        let lidas: Vec<Result<Journal, &str>> = ler(texto, Journal::ler).collect();
        assert_eq!(lidas, vec![Ok(Journal { mod_id: "m", caminho: "a.journal" }), Err("m | a | b"), Err("m |"), Err("só uma coluna")]);

        let reslink: Vec<Result<Reslink, &str>> = ler("a | b | x\na | b | copy | x\na\n", Reslink::ler).collect();
        assert_eq!(reslink, vec![Err("a | b | x"), Err("a | b | copy | x"), Err("a")]);

        let streaming: Vec<Result<Streaming, &str>> = ler("m | s | 5 | -1 | t\nm | s | x | 1 | t\nm | s | 5 | 1 | t | subdel\n", Streaming::ler).collect();
        assert!(streaming.iter().all(Result::is_err), "{streaming:?}");

        let fases: Vec<Result<Questphase, &str>> =
            ler("m | p | f | 1 x | - | | - | 0\nm | p | f | 70000 | - | | - | 0\nm | p | f | | - | | - | 2\nm | p | f | - | - | - | - | 1\n", Questphase::ler).collect();
        assert_eq!(fases[..3].iter().filter(|f| f.is_err()).count(), 3, "{fases:?}");
        assert_eq!(fases[3].as_ref().map(|f| (f.entrada.is_empty(), f.saida.is_empty(), f.intercept)), Ok((true, true, true)));

        let patches: Vec<Result<ResourcePatch, &str>> = ler("m |  | b.ent | -\nm | p.ent | b.ent | entity components\nm | p.ent | b.ent | 0xzz\n", ResourcePatch::ler).collect();
        assert_eq!(patches[0], Err("m |  | b.ent | -"));
        assert_eq!(patches[1].map(|p| p.props), Ok(crate::A17_PROP_ENTITY | crate::A17_PROP_COMPONENTS));
        assert_eq!(patches[2].map(|p| p.props), Ok(0), "hex que não lê cai na leitura por nome, e nenhum nome liga bit");

        let fixes: Vec<Result<Resfix, &str>> = ler(" | names | a | b\nr | names |  | b\nr | outro | a | b\nr | names | a | \n", Resfix::ler).collect();
        assert_eq!(fixes[..3].iter().filter(|f| f.is_err()).count(), 3, "{fixes:?}");
        assert_eq!(fixes[3], Ok(Resfix { recurso: "r", tipo: TipoDeFix::Nomes, antigo: "a", novo: "" }), "o novo vazio é do leitor decidir");

        let escopos: Vec<Result<Resscope, &str>> = ler("0x10 | a.app\n10 | b.app\nzz | c.app\n0x10 | \n", Resscope::ler).collect();
        assert_eq!(escopos[0], Ok(Resscope { escopo: 0x10, alvo: "a.app" }));
        assert_eq!(escopos[1], Ok(Resscope { escopo: 0x10, alvo: "b.app" }), "sem o 0x também é hex");
        assert!(escopos[2].is_err() && escopos[3].is_err());

        let custom: Vec<Result<Customizacao, &str>> = ler("neutro | a\nfemale | \nmale | a | b\n", Customizacao::ler).collect();
        assert!(custom.iter().all(Result::is_err), "{custom:?}");
    }

    /// O cabeçalho de cada tabela começa pela marca do emissor, que é o que o install confere antes de apagar; os 9 arquivos são distintos e
    /// todos `bwms-*.txt`.
    #[test]
    fn cabecalho_de_cada_tabela_comeca_pela_marca_e_os_arquivos_sao_distintos() {
        let mut arquivos: Vec<&str> = TODAS.iter().map(|t| t.arquivo).collect();
        arquivos.sort_unstable();
        arquivos.dedup();
        assert_eq!(arquivos.len(), 9);
        for t in TODAS {
            assert!(t.arquivo.starts_with("bwms-") && t.arquivo.ends_with(".txt"), "{}", t.arquivo);
            assert!(t.cabecalho().starts_with(&t.marca_do_emissor()) && t.cabecalho().ends_with('\n'), "{}", t.arquivo);
            assert_eq!(ler(&t.cabecalho(), Journal::ler).count(), 0, "o cabeçalho não é linha de registro");
        }
        assert_eq!(STREAMING.cabecalho(), "# bwms-streaming (gerado de .xl). Formato: <mod> | <setor> | <expectedNodes> | <nodeIndex> | <tipo>\n");
        assert_eq!(RESLINK.cabecalho_do_install("2 par(es)"), "# bwms-reslink (gerado automaticamente pelo install — 2 par(es))\n");
        assert!(RESLINK.cabecalho_do_install("0 par(es)").starts_with(&RESLINK.marca_do_install()));
    }
}
