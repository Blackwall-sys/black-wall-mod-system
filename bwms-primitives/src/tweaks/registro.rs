//! As tabelas de metadado dos tweaks.
//!
//! Não dependem do motor: são bookkeeping do gerenciador de mods sobre o que os mods fizeram —
//! derivável inteiramente do fluxo de [`Op`] que [`crate::tweaks::interpret`] já produz. Zero
//! endereço nativo, zero boot, zero TweakDB vivo.
//!
//! | tabela | aqui |
//! |---|---|
//! | enums | [`Registro::registrar_enum`]/[`Registro::enums`] |
//! | chaves estrangeiras | [`Registro::registrar_chave`]/[`Registro::esquecer_chave`]/[`Registro::esquecer_chave_do_flat`] |
//! | caminhos de recurso | [`Registro::registrar_recurso`]/[`Registro::esquecer_recurso`]/[`Registro::esquecer_recurso_do_flat`] |
//! | records afetados | [`Registro::records_afetados`]/[`Registro::vazio`]/[`Registro::checar_problemas`] |
//!
//! **De onde vem a classificação.** Um flat é chave-estrangeira ou caminho-de-recurso pelo TIPO
//! declarado dele, não pelo formato do texto do valor — `Items.Foo` e `base/x.mesh` são ambos
//! strings plausíveis e adivinhar pela aparência erraria. O tipo sai da tabela real do
//! `tweakdb.bin` (índice 2 = `TweakDBID`, 3 = `raRef:CResource`), passada aqui como
//! [`Classificador`] pra este módulo morar no `bwms-primitives` sem arrastar o parser binário do
//! `bwms tweak` (que depende de Kraken nativo).
//!
//! **Escopo honesto:** [`Registro::checar_problemas`] NÃO valida contra o TweakDB vivo. Cobre só o que é decidível olhando o fluxo de
//! ops — cada problema abaixo é uma perda silenciosa real que um mod pode causar sozinho.

use std::collections::{BTreeMap, BTreeSet};

use crate::operacoes::{EditOp, Op};

/// O que um flat guarda, do ponto de vista das tabelas de rastreio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Especie {
    /// Tipo `TweakDBID` (índice 2) — o valor referencia outro record. Alimenta a tabela de chaves.
    ChaveEstrangeira,
    /// Tipo `raRef:CResource` (índice 3) — o valor é um caminho de recurso. Alimenta a tabela de recursos.
    CaminhoDeRecurso,
    /// Qualquer outro tipo (número/string/cor/…) — não entra em nenhuma das duas tabelas.
    Outro,
}

/// Diz a espécie de um flat pelo nome. Na prática vem do `tweakdb.bin` real; nos testes, de um
/// mapa fixo. Devolver [`Especie::Outro`] pra flat desconhecido é o comportamento correto —
/// rastrear a mais é pior que rastrear a menos (gera invalidação em cascata sem motivo).
pub type Classificador<'a> = &'a dyn Fn(&str) -> Especie;

/// Um problema que dá pra detectar só com o fluxo de ops, sem TweakDB vivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problema {
    /// O mesmo record é definido duas vezes (`$base`/`$type`) — a segunda vence em silêncio.
    RecordDefinidoDuasVezes { record: String },
}

/// As quatro tabelas, mais o histórico mínimo que `checar_problemas` precisa.
#[derive(Debug, Default)]
pub struct Registro {
    enums: BTreeSet<String>,
    chaves: BTreeMap<String, BTreeSet<String>>,
    recursos: BTreeMap<String, BTreeSet<String>>,
    afetados: BTreeSet<String>,
    problemas: Vec<Problema>,
}

impl Registro {
    pub fn novo() -> Self {
        Self::default()
    }

    // ── enums ────────────────────────────────────────────────────────────────────────────
    pub fn registrar_enum(&mut self, id: &str) {
        self.enums.insert(id.to_string());
    }
    pub fn enums(&self) -> &BTreeSet<String> {
        &self.enums
    }

    // ── chaves estrangeiras ───────────────────────────────────────────────────────────────────────────
    pub fn registrar_chave(&mut self, chave: &str, flat: &str) {
        self.chaves.entry(chave.to_string()).or_default().insert(flat.to_string());
    }
    /// `ForgetForeignKey(aForeignKey)` — apaga a chave inteira e todos os flats sob ela.
    pub fn esquecer_chave(&mut self, chave: &str) {
        self.chaves.remove(chave);
    }
    /// `ForgetForeignKeys(aForeignKey, aFlatId)` — apaga só o par. A chave some junto quando o
    /// último flat sai: uma chave sem nenhum flat referenciando não é "chave com lista vazia",
    /// é chave que não existe mais (senão a tabela cresce pra sempre num hot-reload).
    pub fn esquecer_chave_do_flat(&mut self, chave: &str, flat: &str) {
        if let Some(flats) = self.chaves.get_mut(chave) {
            flats.remove(flat);
            if flats.is_empty() {
                self.chaves.remove(chave);
            }
        }
    }
    pub fn flats_da_chave(&self, chave: &str) -> Option<&BTreeSet<String>> {
        self.chaves.get(chave)
    }
    pub fn chaves(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.chaves
    }

    // ── caminhos de recurso ───────────────────────────────────────────────────────────────────────────
    pub fn registrar_recurso(&mut self, caminho: &str, flat: &str) {
        self.recursos.entry(caminho.to_string()).or_default().insert(flat.to_string());
    }
    pub fn esquecer_recurso(&mut self, caminho: &str) {
        self.recursos.remove(caminho);
    }
    pub fn esquecer_recurso_do_flat(&mut self, caminho: &str, flat: &str) {
        if let Some(flats) = self.recursos.get_mut(caminho) {
            flats.remove(flat);
            if flats.is_empty() {
                self.recursos.remove(caminho);
            }
        }
    }
    pub fn flats_do_recurso(&self, caminho: &str) -> Option<&BTreeSet<String>> {
        self.recursos.get(caminho)
    }
    pub fn recursos(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.recursos
    }

    // ── records afetados ───────────────────────────────────────────────────────────────────────────
    pub fn records_afetados(&self) -> &BTreeSet<String> {
        &self.afetados
    }
    /// `IsEmpty()` — nenhum record tocado E nenhuma tabela de rastreio preenchida.
    pub fn vazio(&self) -> bool {
        self.afetados.is_empty() && self.chaves.is_empty() && self.recursos.is_empty() && self.enums.is_empty()
    }
    pub fn checar_problemas(&self) -> &[Problema] {
        &self.problemas
    }

    /// Consome um lote de ops já interpretado e preenche as quatro tabelas.
    ///
    /// Chamável várias vezes (vários mods) — as tabelas acumulam, que é o ponto: é assim que
    /// dois mods tocando o mesmo record ficam visíveis.
    pub fn observar(&mut self, ops: &[Op], classificar: Classificador<'_>) {
        // A espécie de um flat de record clonado vem do base, conhecido o lote inteiro (o conjunto cria os records antes das edições).
        let mut base_de: BTreeMap<String, String> = BTreeMap::new();
        for op in ops {
            if let Op::Clone { record, base } = op {
                base_de.entry(record.clone()).or_insert_with(|| base.clone());
            }
        }

        let mut ja_definido: BTreeSet<String> = BTreeSet::new();
        for op in ops {
            match op {
                Op::Clone { record, .. } | Op::Create { record, .. } => {
                    if !ja_definido.insert(record.clone()) {
                        self.problemas.push(Problema::RecordDefinidoDuasVezes { record: record.clone() });
                    }
                    self.afetados.insert(record.clone());
                }
                Op::Edit { flat, op: edit } | Op::Criar { flat, op: edit, .. } => {
                    self.afetados.insert(record_do_flat(flat));
                    let especie = especie_do_flat(flat, classificar, &base_de);
                    for valor in valores_do_edit(edit) {
                        match especie {
                            Especie::ChaveEstrangeira => self.registrar_chave(&valor, flat),
                            Especie::CaminhoDeRecurso => self.registrar_recurso(&valor, flat),
                            Especie::Outro => {}
                        }
                    }
                }
            }
        }
    }
}

/// Profundidade máxima da cadeia de herança seguida por [`especie_do_flat`]. Um `$base` que
/// aponta pra si mesmo (direta ou indiretamente) é um mod quebrado, não um caso a suportar — o
/// teto transforma o laço infinito em "não classificado", que é o fallback seguro.
const MAX_HERANCA: usize = 16;

/// A espécie do flat, seguindo `$base` quando o record é novo NESTE lote.
///
/// **Por que não basta classificar o nome direto.** Um mod quase sempre clona um record e edita
/// o clone: `Items.MeuItem.animSetResource` NÃO existe no `tweakdb.bin`, então o classificador
/// devolveria [`Especie::Outro`] e as tabelas de chaves e de recursos ficariam vazias exatamente no caso
/// mais comum de uso. O tipo certo vem do SCHEMA do record; aqui o equivalente
/// disponível é o record base — `Items.MeuItem` clonado de `Items.Katana` tem os flats de
/// `Items.Katana`, cujos tipos o DB conhece. Cadeia seguida transitivamente (clone de clone).
fn especie_do_flat(
    flat: &str,
    classificar: Classificador<'_>,
    base_de: &BTreeMap<String, String>,
) -> Especie {
    let direto = classificar(flat);
    if direto != Especie::Outro {
        return direto;
    }
    let (mut record, campo) = match flat.rfind('.') {
        Some(i) => (flat[..i].to_string(), &flat[i + 1..]),
        None => return Especie::Outro,
    };
    for _ in 0..MAX_HERANCA {
        let Some(base) = base_de.get(&record) else { return Especie::Outro };
        let e = classificar(&format!("{base}.{campo}"));
        if e != Especie::Outro {
            return e;
        }
        record = base.clone();
    }
    Especie::Outro
}

/// `Items.Foo.damage` → `Items.Foo`. Sem ponto nenhum, o próprio nome é o record.
///
/// Vale pros records INLINE também (`Items.A.stats$3f2a.value` → `Items.A.stats$3f2a`), que é o
/// certo: o record inline é uma entidade própria no TweakDB, não um pedaço do pai.
pub fn record_do_flat(flat: &str) -> String {
    match flat.rfind('.') {
        Some(i) => flat[..i].to_string(),
        None => flat.to_string(),
    }
}

/// Os valores que um `EditOp` introduz. `Assign` de array (`[a, b, c]`) rende N valores;
/// `RemoveAll` não rende nenhum (não tem valor, por definição — ver `EditOp::RemoveAll`).
///
/// `AppendFrom`/`PrependFrom` também não rendem: o operando é o NOME de outro flat de onde
/// copiar, não uma referência que este flat passa a guardar. Registrá-lo como chave estrangeira
/// apontaria a invalidação pro alvo errado.
fn valores_do_edit(edit: &EditOp) -> Vec<String> {
    match edit {
        EditOp::Assign(v) => elementos(v),
        EditOp::Append(v)
        | EditOp::AppendOnce(v)
        | EditOp::Prepend(v)
        | EditOp::PrependOnce(v)
        | EditOp::Remove(v) => vec![normalizar(v)],
        EditOp::RemoveAll | EditOp::AppendFrom(_) | EditOp::PrependFrom(_) => Vec::new(),
    }
}

/// Divide `[a, b, c]` nos elementos; qualquer outra coisa é um valor só. Respeita aspas —
/// caminho de recurso com vírgula dentro existe e cortá-lo geraria dois caminhos inválidos.
fn elementos(v: &str) -> Vec<String> {
    let t = v.trim();
    let Some(interno) = t.strip_prefix('[').and_then(|x| x.strip_suffix(']')) else {
        return vec![normalizar(t)];
    };
    let mut out = Vec::new();
    let mut atual = String::new();
    let mut aspas: Option<char> = None;
    for c in interno.chars() {
        match aspas {
            Some(q) => {
                atual.push(c);
                if c == q {
                    aspas = None;
                }
            }
            None if c == '"' || c == '\'' => {
                atual.push(c);
                aspas = Some(c);
            }
            None if c == ',' => {
                if !atual.trim().is_empty() {
                    out.push(normalizar(&atual));
                }
                atual.clear();
            }
            None => atual.push(c),
        }
    }
    if !atual.trim().is_empty() {
        out.push(normalizar(&atual));
    }
    out
}

/// Tira as decorações aceitas pro MESMO valor (`$Items.Foo`, `"base/x.mesh"`,
/// `t"Items.Foo"`), pra duas grafias do mesmo alvo caírem na mesma entrada da tabela.
fn normalizar(v: &str) -> String {
    let t = v.trim();
    let t = t.strip_prefix("t\"").or_else(|| t.strip_prefix("t'")).map_or(t, |r| {
        r.strip_suffix('"').or_else(|| r.strip_suffix('\'')).unwrap_or(r)
    });
    let t = t
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .or_else(|| t.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')))
        .unwrap_or(t);
    t.strip_prefix('$').unwrap_or(t).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tudo_outro(_: &str) -> Especie {
        Especie::Outro
    }

    #[test]
    fn flat_de_record_clonado_herda_a_especie_do_base() {
        // O caso mais comum de mod: clonar e editar. `Items.Meu.mesh` não existe no DB.
        let classificar = |flat: &str| {
            if flat == "Items.Katana.mesh" { Especie::CaminhoDeRecurso } else { Especie::Outro }
        };
        let ops = vec![
            Op::Clone { record: "Items.Meu".into(), base: "Items.Katana".into() },
            Op::Edit { flat: "Items.Meu.mesh".into(), op: EditOp::Assign("base/x.mesh".into()) },
        ];
        let mut r = Registro::novo();
        r.observar(&ops, &classificar);
        assert_eq!(r.flats_do_recurso("base/x.mesh").map(BTreeSet::len), Some(1));
    }

    #[test]
    fn heranca_e_seguida_em_cadeia() {
        let classificar = |flat: &str| {
            if flat == "Items.Raiz.mesh" { Especie::CaminhoDeRecurso } else { Especie::Outro }
        };
        let ops = vec![
            Op::Clone { record: "Items.Meio".into(), base: "Items.Raiz".into() },
            Op::Clone { record: "Items.Folha".into(), base: "Items.Meio".into() },
            Op::Edit { flat: "Items.Folha.mesh".into(), op: EditOp::Assign("base/y.mesh".into()) },
        ];
        let mut r = Registro::novo();
        r.observar(&ops, &classificar);
        assert_eq!(r.flats_do_recurso("base/y.mesh").map(BTreeSet::len), Some(1));
    }

    #[test]
    fn heranca_circular_para_em_vez_de_travar() {
        // Mod quebrado (A herda de B, B herda de A) não pode virar laço infinito.
        let ops = vec![
            Op::Clone { record: "Items.A".into(), base: "Items.B".into() },
            Op::Clone { record: "Items.B".into(), base: "Items.A".into() },
            Op::Edit { flat: "Items.A.mesh".into(), op: EditOp::Assign("base/z.mesh".into()) },
        ];
        let mut r = Registro::novo();
        r.observar(&ops, &|_| Especie::Outro);
        assert!(r.recursos().is_empty());
    }

    #[test]
    fn classificacao_direta_ganha_da_heranca() {
        // Se o próprio flat já é conhecido, nunca consultar o base (o schema do clone pode
        // divergir do original quando o mod redefine o campo).
        let classificar = |flat: &str| match flat {
            "Items.Meu.x" => Especie::ChaveEstrangeira,
            "Items.Base.x" => Especie::CaminhoDeRecurso,
            _ => Especie::Outro,
        };
        let ops = vec![
            Op::Clone { record: "Items.Meu".into(), base: "Items.Base".into() },
            Op::Edit { flat: "Items.Meu.x".into(), op: EditOp::Assign("Items.Alvo".into()) },
        ];
        let mut r = Registro::novo();
        r.observar(&ops, &classificar);
        assert_eq!(r.chaves().len(), 1);
        assert!(r.recursos().is_empty());
    }

    #[test]
    fn record_do_flat_pega_o_prefixo() {
        assert_eq!(record_do_flat("Items.Foo.damage"), "Items.Foo");
        assert_eq!(record_do_flat("Items.Foo"), "Items");
        assert_eq!(record_do_flat("solto"), "solto");
    }

    #[test]
    fn record_inline_e_entidade_propria() {
        // O `$` do sufixo inline não muda a regra: o record é tudo menos o último segmento.
        assert_eq!(record_do_flat("Items.A.stats$3f2a.value"), "Items.A.stats$3f2a");
    }

    #[test]
    fn afetados_junta_clone_create_e_edit() {
        let ops = vec![
            Op::Clone { record: "Items.A".into(), base: "Items.B".into() },
            Op::Edit { flat: "Items.C.damage".into(), op: EditOp::Assign("10".into()) },
        ];
        let mut r = Registro::novo();
        r.observar(&ops, &tudo_outro);
        assert_eq!(r.records_afetados().iter().cloned().collect::<Vec<_>>(), vec!["Items.A", "Items.C"]);
        assert!(!r.vazio());
    }

    #[test]
    fn registro_novo_e_vazio() {
        assert!(Registro::novo().vazio());
    }

    #[test]
    fn so_flat_do_tipo_certo_entra_em_cada_tabela() {
        let classificar = |flat: &str| match flat {
            "Items.A.parent" => Especie::ChaveEstrangeira,
            "Items.A.mesh" => Especie::CaminhoDeRecurso,
            _ => Especie::Outro,
        };
        let ops = vec![
            Op::Edit { flat: "Items.A.parent".into(), op: EditOp::Assign("Items.Base".into()) },
            Op::Edit { flat: "Items.A.mesh".into(), op: EditOp::Assign("base/x.mesh".into()) },
            Op::Edit { flat: "Items.A.damage".into(), op: EditOp::Assign("10".into()) },
        ];
        let mut r = Registro::novo();
        r.observar(&ops, &classificar);
        assert_eq!(r.flats_da_chave("Items.Base").unwrap().len(), 1);
        assert_eq!(r.flats_do_recurso("base/x.mesh").unwrap().len(), 1);
        // o flat numérico não entrou em NENHUMA das duas
        assert_eq!(r.chaves().len(), 1);
        assert_eq!(r.recursos().len(), 1);
    }

    #[test]
    fn esquecer_o_ultimo_flat_apaga_a_chave() {
        let mut r = Registro::novo();
        r.registrar_chave("Items.Base", "Items.A.parent");
        r.registrar_chave("Items.Base", "Items.B.parent");
        r.esquecer_chave_do_flat("Items.Base", "Items.A.parent");
        assert_eq!(r.flats_da_chave("Items.Base").unwrap().len(), 1);
        r.esquecer_chave_do_flat("Items.Base", "Items.B.parent");
        // some de vez: chave sem flat não é chave com lista vazia
        assert!(r.flats_da_chave("Items.Base").is_none());
        assert!(r.chaves().is_empty());
    }

    #[test]
    fn esquecer_chave_inteira_leva_todos_os_flats() {
        let mut r = Registro::novo();
        r.registrar_chave("Items.Base", "Items.A.parent");
        r.registrar_chave("Items.Base", "Items.B.parent");
        r.esquecer_chave("Items.Base");
        assert!(r.chaves().is_empty());
    }

    #[test]
    fn esquecer_recurso_espelha_a_chave() {
        let mut r = Registro::novo();
        r.registrar_recurso("base/x.mesh", "Items.A.mesh");
        r.esquecer_recurso_do_flat("base/x.mesh", "Items.A.mesh");
        assert!(r.recursos().is_empty());
        r.registrar_recurso("base/y.mesh", "Items.A.mesh");
        r.esquecer_recurso("base/y.mesh");
        assert!(r.recursos().is_empty());
    }

    #[test]
    fn esquecer_o_que_nao_existe_nao_panica() {
        let mut r = Registro::novo();
        r.esquecer_chave("nada");
        r.esquecer_chave_do_flat("nada", "nada");
        r.esquecer_recurso("nada");
        r.esquecer_recurso_do_flat("nada", "nada");
        assert!(r.vazio());
    }

    #[test]
    fn enums_sao_tabela_propria_e_contam_pro_vazio() {
        let mut r = Registro::novo();
        r.registrar_enum("Enums.Foo");
        assert!(!r.vazio());
        assert!(r.enums().contains("Enums.Foo"));
    }

    #[test]
    fn array_de_chaves_rende_uma_entrada_por_elemento() {
        let classificar = |_: &str| Especie::ChaveEstrangeira;
        let ops = vec![Op::Edit {
            flat: "Items.A.tags".into(),
            op: EditOp::Assign("[Items.X, Items.Y, Items.Z]".into()),
        }];
        let mut r = Registro::novo();
        r.observar(&ops, &classificar);
        assert_eq!(r.chaves().len(), 3);
    }

    #[test]
    fn virgula_dentro_de_aspas_nao_divide_o_valor() {
        // Um caminho com vírgula existe; cortá-lo geraria dois caminhos inválidos.
        assert_eq!(elementos("[\"base/a,b.mesh\", base/c.mesh]"), vec!["base/a,b.mesh", "base/c.mesh"]);
    }

    #[test]
    fn grafias_diferentes_do_mesmo_alvo_caem_na_mesma_entrada() {
        assert_eq!(normalizar("$Items.Foo"), "Items.Foo");
        assert_eq!(normalizar("t\"Items.Foo\""), "Items.Foo");
        assert_eq!(normalizar("\"Items.Foo\""), "Items.Foo");
        assert_eq!(normalizar("  Items.Foo  "), "Items.Foo");
    }

    #[test]
    fn append_from_nao_vira_chave_estrangeira() {
        // O operando é de ONDE copiar, não o que o flat passa a referenciar — registrá-lo
        // apontaria a invalidação em cascata pro record errado.
        let ops = vec![Op::Edit {
            flat: "Items.A.tags".into(),
            op: EditOp::AppendFrom("Items.B.tags".into()),
        }];
        let mut r = Registro::novo();
        r.observar(&ops, &|_| Especie::ChaveEstrangeira);
        assert!(r.chaves().is_empty());
        // mas o record segue afetado
        assert!(r.records_afetados().contains("Items.A"));
    }

    #[test]
    fn remove_all_nao_rende_valor() {
        let ops = vec![Op::Edit { flat: "Items.A.tags".into(), op: EditOp::RemoveAll }];
        let mut r = Registro::novo();
        r.observar(&ops, &|_| Especie::ChaveEstrangeira);
        assert!(r.chaves().is_empty());
    }

    #[test]
    fn record_definido_duas_vezes_e_problema() {
        let ops = vec![
            Op::Clone { record: "Items.A".into(), base: "Items.B".into() },
            Op::Clone { record: "Items.A".into(), base: "Items.C".into() },
        ];
        let mut r = Registro::novo();
        r.observar(&ops, &tudo_outro);
        assert_eq!(
            r.checar_problemas(),
            [Problema::RecordDefinidoDuasVezes { record: "Items.A".into() }]
        );
    }

    #[test]
    fn editar_record_criado_antes_nao_e_problema() {
        let ops = vec![
            Op::Clone { record: "Items.A".into(), base: "Items.B".into() },
            Op::Edit { flat: "Items.A.damage".into(), op: EditOp::Assign("10".into()) },
        ];
        let mut r = Registro::novo();
        r.observar(&ops, &tudo_outro);
        assert!(r.checar_problemas().is_empty());
    }

    /// O conjunto aplica records antes das edições e as mutações de array depois das atribuições: edição antes do `$base` no lote e
    /// `!append` antes de `= [...]` não perdem nada.
    #[test]
    fn ordem_no_lote_nao_e_problema() {
        let ops = vec![
            Op::Edit { flat: "Items.A.damage".into(), op: EditOp::Assign("10".into()) },
            Op::Clone { record: "Items.A".into(), base: "Items.B".into() },
            Op::Edit { flat: "Items.A.tags".into(), op: EditOp::Append("X".into()) },
            Op::Edit { flat: "Items.A.tags".into(), op: EditOp::Assign("[Y]".into()) },
        ];
        let mut r = Registro::novo();
        r.observar(&ops, &tudo_outro);
        assert!(r.checar_problemas().is_empty());
    }

    #[test]
    fn append_depois_de_atribuicao_e_ordem_normal() {
        // A ordem inversa é o idioma CORRETO (define a base, depois acrescenta) — nunca avisar.
        let ops = vec![
            Op::Edit { flat: "Items.A.tags".into(), op: EditOp::Assign("[Y]".into()) },
            Op::Edit { flat: "Items.A.tags".into(), op: EditOp::Append("X".into()) },
        ];
        let mut r = Registro::novo();
        r.observar(&ops, &tudo_outro);
        assert!(r.checar_problemas().is_empty());
    }

    #[test]
    fn dois_mods_no_mesmo_record_ficam_visiveis() {
        // O ponto de acumular entre chamadas: é assim que conflito entre mods aparece.
        let mut r = Registro::novo();
        r.observar(
            &[Op::Edit { flat: "Items.A.damage".into(), op: EditOp::Assign("10".into()) }],
            &tudo_outro,
        );
        r.observar(
            &[Op::Edit { flat: "Items.A.damage".into(), op: EditOp::Assign("20".into()) }],
            &tudo_outro,
        );
        assert_eq!(r.records_afetados().len(), 1);
    }
}

#[cfg(test)]
mod grounding {
    use super::*;

    /// Reproduz o formato EXATO do classificador de produção (lookup por hash num mapa), que é
    /// onde a herança falhou ao ser testada contra o `tweakdb.bin` real — os testes com closure
    /// de string casada não pegam a diferença.
    #[test]
    fn heranca_funciona_com_classificador_por_mapa() {
        use std::collections::HashMap;
        let mut mapa: HashMap<String, Especie> = HashMap::new();
        mapa.insert("Items.Base.campo".into(), Especie::ChaveEstrangeira);
        let classificar = |flat: &str| *mapa.get(flat).unwrap_or(&Especie::Outro);

        let base_de: BTreeMap<String, String> =
            [("Items.Novo".to_string(), "Items.Base".to_string())].into_iter().collect();
        assert_eq!(
            especie_do_flat("Items.Novo.campo", &classificar, &base_de),
            Especie::ChaveEstrangeira
        );
    }
}
