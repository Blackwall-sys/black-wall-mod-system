//! Codeware `#132`/`#133`/`#134`/`#135`/`#136` — semântica de match dos `CallbackSystemTarget`.
//!
//! Os 7 Targets do Codeware (`App/Callback/Targets/`) compartilham **uma** regra de decisão, e é
//! ela que este módulo porta:
//!
//! > para cada campo do filtro: se o campo está NÃO-SETADO, ele não filtra nada; se está setado,
//! > o evento precisa bater exatamente naquele campo. Todos os campos setados precisam bater.
//!
//! O que conta como "não-setado" é por tipo e vem da fonte, não de convenção minha:
//! `CName`/`ResourcePath`/`TweakDBID` = `0`, `EInputKey` = `IK_None`, `EInputAction` = `IACT_None`,
//! lista de tags = vazia. Aqui o campo do filtro chega como `Option`, com `None` = não-setado: quem lê
//! o alvo converte o valor da referência na entrada, e nenhuma regra deste módulo depende de valor
//! sentinela. É por isso que existe [`campo_bate`] em vez de um `==` solto — "não-setado é curinga" é
//! a parte que um porte descuidado erra, e ela inverte o resultado do filtro inteiro.
//!
//! **Escopo honesto:** os Targets de ENTIDADE (`DynamicEntityTarget`/`StaticEntityTarget`) usam a
//! mesma regra de decisão, mas o predicado por tag (`entitySystem->IsTagged`) é nativo. Aqui a
//! decisão é portada e o predicado entra como argumento ([`bate_por_tags`]), o que deixa a parte
//! pura testável agora e isola exatamente o que ainda depende de RE.

/// Um campo de filtro bate quando está não-setado (`None`, curinga) OU quando é igual ao do evento.
pub fn campo_bate<T: PartialEq>(filtro: Option<T>, evento: T) -> bool {
    // rust-version 1.74 (Cargo.toml): `Option::is_none_or` só existe a partir do 1.82.
    filtro.map_or(true, |f| f == evento)
}

/// `InkWidgetTarget` (`#135`): filtra por `libraryPath` e `itemName`, ambos curinga quando `None`.
pub fn inkwidget_bate(
    filtro_library_path: Option<u64>,
    filtro_item_name: Option<u64>,
    evento_library_path: u64,
    evento_item_name: u64,
) -> bool {
    campo_bate(filtro_library_path, evento_library_path) && campo_bate(filtro_item_name, evento_item_name)
}

/// `ComponentTarget` (`#132`): filtra por `componentName` e pelo `componentID.unk00`.
pub fn component_bate(
    filtro_nome: Option<u64>,
    filtro_id: Option<u64>,
    evento_nome: u64,
    evento_id: u64,
) -> bool {
    campo_bate(filtro_nome, evento_nome) && campo_bate(filtro_id, evento_id)
}

/// `InputTarget` (`#136`): filtra por tecla e ação; `IK_None`/`IACT_None` da referência chegam como `None`.
pub fn input_bate(
    filtro_key: Option<u32>,
    filtro_action: Option<u32>,
    evento_key: u32,
    evento_action: u32,
) -> bool {
    campo_bate(filtro_key, evento_key) && campo_bate(filtro_action, evento_action)
}

/// `DynamicEntityTarget`/`StaticEntityTarget` (`#133`): lista vazia casa tudo; senão casa se o
/// evento tem QUALQUER uma das tags.
///
/// `tem_tag` é o predicado nativo (`entitySystem->IsTagged`), passado de fora — a decisão é o que
/// esta função porta.
pub fn bate_por_tags<F: FnMut(u64) -> bool>(tags: &[u64], mut tem_tag: F) -> bool {
    if tags.is_empty() {
        return true;
    }
    tags.iter().any(|t| tem_tag(*t))
}

/// Uma aparência candidata do `EntityBuilder`: caminho do recurso + nome da definição.
#[derive(Debug, Clone, Copy)]
pub struct AparenciaCandidata {
    pub resource_path: u64,
    pub definition_name: u64,
}

/// `EntityTarget` (`#134`) — casamento de aparência do ramo `EntityBuilderEvent`.
///
/// Três regras do original que um porte apressado erra:
/// 1. se `appearancePath` não está setado, o filtro de aparência **não se aplica** (passa);
/// 2. com `resource` único presente, ele é o ÚNICO candidato — se não bater, falha, e a lista
///    nem é consultada;
/// 3. sem `resource` e com a lista VAZIA, o resultado é **falha** — não "passa por omissão".
///    É o `else { return false; }` final, e é o ponto mais fácil de perder de vista.
pub fn entity_builder_aparencia_bate(
    filtro_appearance_path: Option<u64>,
    filtro_definition_name: Option<u64>,
    resource_unico: Option<AparenciaCandidata>,
    lista: &[AparenciaCandidata],
) -> bool {
    let Some(appearance_path) = filtro_appearance_path else {
        return true;
    };
    if let Some(r) = resource_unico {
        if appearance_path != r.resource_path {
            return false;
        }
        return campo_bate(filtro_definition_name, r.definition_name);
    }
    if lista.is_empty() {
        return false;
    }
    lista.iter().any(|a| {
        appearance_path == a.resource_path && campo_bate(filtro_definition_name, a.definition_name)
    })
}

/// Categoria da entidade, para o despacho de `recordID` do ramo de eventos de entidade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoriaEntidade {
    Puppet,
    Vehicle,
    Device,
    /// Nenhuma das três — o original devolve `false` neste caso.
    Outra,
}

/// `EntityTarget` (`#134`) — despacho de `recordID` por categoria de entidade.
///
/// O original resolve o record por 3 caminhos DIFERENTES (puppet/vehicle/device) e, se a entidade
/// não é nenhum dos três, **falha**. `record_da_entidade` é `None` quando o caminho nativo não
/// conseguiu resolver (ex.: device sem `controller` ou sem `persistentState`) — e `None` também é
/// falha, não "ignora o filtro".
pub fn entity_record_bate(
    filtro_record_id: Option<u64>,
    categoria: CategoriaEntidade,
    record_da_entidade: Option<u64>,
) -> bool {
    let Some(record_id) = filtro_record_id else {
        return true;
    };
    if categoria == CategoriaEntidade::Outra {
        return false;
    }
    match record_da_entidade {
        Some(r) => record_id == r,
        None => false,
    }
}

/// Os 7 campos do `EntityTarget` da referência; cada fábrica preenche o seu (`Definition` preenche
/// 2). `None` = não-setado.
#[derive(Clone, Copy, Default, Debug)]
pub struct CamposDoAlvoDeEntidade {
    pub entity_id: Option<u64>,
    /// A classe que a fábrica resolveu, como ponteiro: o `IsA` é nativo e fica com quem chama.
    pub tipo: Option<u64>,
    pub record_id: Option<u64>,
    pub template_path: Option<u64>,
    pub appearance_name: Option<u64>,
    pub appearance_path: Option<u64>,
    pub definition_name: Option<u64>,
}

/// Os 3 campos da entidade do evento que o ramo de entidade compara. `template_path` e
/// `appearance_name` são hashes do mesmo tamanho: o nome do campo é o que impede a troca.
#[derive(Clone, Copy, Debug)]
pub struct CamposDoEventoDeEntidade {
    pub entity_id: u64,
    pub template_path: u64,
    pub appearance_name: u64,
}

/// `EntityTarget` (`#134`) — o ramo dos 3 eventos de ENTIDADE
/// (`EntityLifecycle`/`EntityComponent`/`VehicleLightControl`).
///
/// **A regra que mais surpreende:** se `appearancePath` OU `definitionName` estiverem setados, este
/// ramo falha de imediato — esses 2 filtros só existem para o `EntityBuilderEvent`. Quem porta sem
/// notar isso faz um filtro de aparência "funcionar" onde o original recusa.
pub fn entity_evento_bate(
    filtro: &CamposDoAlvoDeEntidade,
    evento: &CamposDoEventoDeEntidade,
) -> bool {
    if filtro.appearance_path.is_some() || filtro.definition_name.is_some() {
        return false;
    }
    campo_bate(filtro.entity_id, evento.entity_id)
        && campo_bate(filtro.template_path, evento.template_path)
        && campo_bate(filtro.appearance_name, evento.appearance_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campo_nao_setado_e_curinga() {
        // A regra que um porte descuidado erra: não-setado NÃO quer dizer "só casa com o vazio".
        assert!(campo_bate(None, 12345u64));
        assert!(campo_bate(None, 0u64));
    }

    #[test]
    fn campo_setado_exige_igualdade() {
        assert!(campo_bate(Some(7u64), 7));
        assert!(!campo_bate(Some(7u64), 8));
        // um filtro setado nunca casa com o "vazio" do evento
        assert!(!campo_bate(Some(7u64), 0));
        // o núcleo não conhece sentinela: `Some(0)` é campo setado, e quem converte o 0 da referência é o adaptador
        assert!(campo_bate(Some(0u64), 0));
        assert!(!campo_bate(Some(0u64), 7));
    }

    #[test]
    fn inkwidget_todos_os_quadrantes() {
        // filtro vazio casa qualquer evento
        assert!(inkwidget_bate(None, None, 111, 222));
        // um campo setado e batendo, outro curinga
        assert!(inkwidget_bate(Some(111), None, 111, 222));
        assert!(inkwidget_bate(None, Some(222), 111, 222));
        // um campo setado e NÃO batendo derruba o conjunto
        assert!(!inkwidget_bate(Some(999), None, 111, 222));
        assert!(!inkwidget_bate(Some(111), Some(999), 111, 222));
    }

    #[test]
    fn component_mesma_regra() {
        assert!(component_bate(None, None, 5, 6));
        assert!(component_bate(Some(5), Some(6), 5, 6));
        assert!(!component_bate(Some(5), Some(7), 5, 6));
    }

    #[test]
    fn input_mesma_regra() {
        assert!(input_bate(None, None, 9, 2));
        assert!(input_bate(Some(9), Some(2), 9, 2));
        assert!(!input_bate(Some(9), Some(1), 9, 2));
        // ação curinga: só a tecla decide
        assert!(input_bate(Some(9), None, 9, 2));
        assert!(!input_bate(Some(8), None, 9, 2));
    }

    #[test]
    fn tags_vazias_casam_tudo() {
        assert!(bate_por_tags(&[], |_| false));
    }

    #[test]
    fn tags_casam_por_qualquer_uma() {
        assert!(bate_por_tags(&[1, 2, 3], |t| t == 3));
        assert!(!bate_por_tags(&[1, 2, 3], |_| false));
    }

    #[test]
    fn tags_param_no_primeiro_acerto() {
        // `IsTagged` é nativo: depois do primeiro acerto, consultar o resto é custo sem efeito.
        let mut consultas = 0;
        assert!(bate_por_tags(&[1, 2, 3], |t| {
            consultas += 1;
            t == 1
        }));
        assert_eq!(consultas, 1);
    }

    #[test]
    fn builder_lista_vazia_e_falha_nao_omissao() {
        // O `else { return false; }` do original: sem resource único e sem lista, NÃO passa.
        assert!(!entity_builder_aparencia_bate(Some(77), None, None, &[]));
        // mas se o filtro de path nem está setado, o bloco inteiro não se aplica
        assert!(entity_builder_aparencia_bate(None, None, None, &[]));
    }

    #[test]
    fn builder_resource_unico_e_exclusivo() {
        let r = AparenciaCandidata { resource_path: 77, definition_name: 5 };
        // A lista contém exatamente o que o filtro pede: só a exclusividade do resource único
        // explica o `false`. Uma lista que também não batesse deixaria passar quem cai nela.
        let outro = AparenciaCandidata { resource_path: 88, definition_name: 5 };
        // com resource único presente, a LISTA não é consultada nem em caso de falha
        assert!(!entity_builder_aparencia_bate(Some(88), None, Some(r), &[outro]));
        assert!(entity_builder_aparencia_bate(Some(77), None, Some(r), &[]));
        // definitionName setado tem de bater junto
        assert!(entity_builder_aparencia_bate(Some(77), Some(5), Some(r), &[]));
        assert!(!entity_builder_aparencia_bate(Some(77), Some(6), Some(r), &[]));
    }

    #[test]
    fn builder_lista_casa_por_qualquer_uma() {
        let lista = [
            AparenciaCandidata { resource_path: 1, definition_name: 10 },
            AparenciaCandidata { resource_path: 2, definition_name: 20 },
        ];
        assert!(entity_builder_aparencia_bate(Some(2), None, None, &lista));
        assert!(entity_builder_aparencia_bate(Some(2), Some(20), None, &lista));
        // path bate mas definition não: o original faz `continue`, então NÃO casa
        assert!(!entity_builder_aparencia_bate(Some(2), Some(99), None, &lista));
        assert!(!entity_builder_aparencia_bate(Some(3), None, None, &lista));
    }

    #[test]
    fn record_entidade_fora_das_3_categorias_falha() {
        assert!(!entity_record_bate(Some(5), CategoriaEntidade::Outra, Some(5)));
        // sem filtro, categoria nem é consultada
        assert!(entity_record_bate(None, CategoriaEntidade::Outra, None));
    }

    #[test]
    fn record_nao_resolvido_e_falha_nao_curinga() {
        // device sem controller/persistentState -> None -> falha
        assert!(!entity_record_bate(Some(5), CategoriaEntidade::Device, None));
        assert!(entity_record_bate(Some(5), CategoriaEntidade::Device, Some(5)));
        assert!(!entity_record_bate(Some(5), CategoriaEntidade::Puppet, Some(6)));
    }

    #[test]
    fn ramo_de_entidade_recusa_filtro_de_aparencia() {
        let vazio = CamposDoEventoDeEntidade { entity_id: 0, template_path: 0, appearance_name: 0 };
        let evento = CamposDoEventoDeEntidade { entity_id: 9, template_path: 8, appearance_name: 7 };
        // A regra fácil de perder: appearancePath/definitionName são exclusivos do EntityBuilder.
        assert!(!entity_evento_bate(&CamposDoAlvoDeEntidade { appearance_path: Some(1), ..Default::default() }, &vazio));
        assert!(!entity_evento_bate(&CamposDoAlvoDeEntidade { definition_name: Some(1), ..Default::default() }, &vazio));
        // sem eles, os 3 campos normais valem a regra de curinga de sempre
        let por_id = CamposDoAlvoDeEntidade { entity_id: Some(9), ..Default::default() };
        assert!(entity_evento_bate(&CamposDoAlvoDeEntidade::default(), &evento));
        assert!(entity_evento_bate(&por_id, &evento));
        assert!(!entity_evento_bate(&por_id, &CamposDoEventoDeEntidade { entity_id: 1, ..evento }));
    }

    #[test]
    fn ramo_de_entidade_compara_cada_campo_com_o_seu() {
        // templatePath e appearanceName são hashes do mesmo tamanho: cada filtro só pode olhar o
        // próprio campo do evento (evento: template 0xA, aparência 0xB).
        let evento = CamposDoEventoDeEntidade { entity_id: 0, template_path: 0xA, appearance_name: 0xB };
        let certo = CamposDoAlvoDeEntidade { template_path: Some(0xA), appearance_name: Some(0xB), ..Default::default() };
        let trocado = CamposDoAlvoDeEntidade { template_path: Some(0xB), appearance_name: Some(0xA), ..Default::default() };
        assert!(entity_evento_bate(&certo, &evento));
        assert!(!entity_evento_bate(&trocado, &evento));
    }

    #[test]
    fn tags_nao_chamam_predicado_quando_vazio() {
        // Importa: com lista vazia o original nem consulta o sistema (retorna true antes).
        let mut chamou = false;
        let r = bate_por_tags(&[], |_| {
            chamou = true;
            true
        });
        assert!(r && !chamou);
    }
}
