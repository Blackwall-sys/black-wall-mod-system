//! Leitor do `GarmentProcessingContext` — ArchiveXL `#4` e `#5`.
//!
//! **O que os itens pedem.** `#4` descreve o contexto COMPLETO que `ProcessGarment` e
//! `ExtractComponentsJob` usam (componentes, meshes, templates, recursos, definição de aparência,
//! request, job), e `#5` os PARÂMETROS de entrada dessas funções — o objeto central do pipeline
//! de montagem de garment. Os dois fecharam ao vivo em 2026-09-06 (critério em `resumo()`).
//!
//! **Por que a decodificação mora aqui, fora do runtime.** Ler struct por offset é onde este
//! projeto mais se machucou: offset errado não dá erro, dá lixo plausível. Separar a aritmética
//! (aqui, com teste) do acesso à memória viva (no `register.rs`) é o que permite provar a primeira
//! sem o jogo. Este módulo NÃO desreferencia nada: recebe bytes já lidos e diz o que eles
//! significam.
//!
//! **Offsets vêm do header vendorizado**, não de chute — estão transcritos no item `#4` e
//! repetidos abaixo em constantes com nome, pra um erro de digitação virar teste vermelho e não
//! um campo lido do lugar errado. Os 4 arrays saem pelo decodificador único de `DynArray`
//! (`crate::dynarray`), o mesmo cujo crivo o runtime usa.

use crate::dynarray::DynArray;

/// Offsets do `GarmentProcessingContext`, conforme o header do ArchiveXL (item `#4`).
pub mod off {
    pub const UNK00: usize = 0x00; // SharedPtr<void>
    pub const COMPONENTS: usize = 0x10; // DynArray<Handle<IComponent>>
    pub const MESHES: usize = 0x20; // DynArray<Handle<CMesh>>
    pub const TEMPLATES: usize = 0x30; // DynArray<Handle<EntityTemplate>>
    pub const RESOURCES: usize = 0x40; // DynArray<ResourcePath>
    pub const DEFINITION: usize = 0x58; // Handle<AppearanceDefinition>
    pub const REQUEST: usize = 0x68; // EntityInitializeRequest
    pub const JOB: usize = 0x138; // JobHandle
    /// Menor tamanho que ainda contém todos os campos nomeados.
    pub const TAMANHO_MINIMO: usize = 0x140;
}

/// O contexto decodificado. Todo campo é cru — quem quiser desreferenciar decide depois, e com
/// as guardas do runtime.
///
/// Sem `Default`: `decodificar` devolve `None` para "não decodificou" nunca virar "contexto
/// vazio", e um `GarmentContext::default()` reabriria esse caminho.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GarmentContext {
    pub components: DynArray,
    pub meshes: DynArray,
    pub templates: DynArray,
    pub resources: DynArray,
    pub definition: u64,
    pub job: u64,
}

fn u64_em(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(off..off + 8)?.try_into().ok()?))
}

/// Decodifica um contexto a partir dos bytes já lidos. `None` se o buffer for curto demais —
/// nunca lê parcial e completa com zero, que transformaria buffer truncado em "contexto vazio".
pub fn decodificar(bytes: &[u8]) -> Option<GarmentContext> {
    if bytes.len() < off::TAMANHO_MINIMO {
        return None;
    }
    Some(GarmentContext {
        components: DynArray::decodificar(bytes, off::COMPONENTS)?,
        meshes: DynArray::decodificar(bytes, off::MESHES)?,
        templates: DynArray::decodificar(bytes, off::TEMPLATES)?,
        resources: DynArray::decodificar(bytes, off::RESOURCES)?,
        definition: u64_em(bytes, off::DEFINITION)?,
        job: u64_em(bytes, off::JOB)?,
    })
}

impl GarmentContext {
    fn arrays(&self) -> [DynArray; 4] {
        [self.components, self.meshes, self.templates, self.resources]
    }

    /// Os 4 arrays respeitam as invariantes de `DynArray`. `false` refuta o layout: algum offset
    /// caiu fora de um `DynArray`. `true` NÃO prova que o bloco é um `GarmentProcessingContext` —
    /// 0x140 bytes zerados passam, e uma constante de offset apontando para o `DynArray` vizinho do
    /// mesmo bloco também.
    pub fn arrays_com_estrutura_valida(&self) -> bool {
        self.arrays().iter().all(DynArray::estrutura_valida)
    }

    /// Os 4 arrays passam na heurística de capacidade. Mesmo alcance do crivo de estrutura: `false`
    /// aponta offset errado, `true` não identifica o tipo.
    pub fn arrays_com_capacidade_plausivel(&self) -> bool {
        self.arrays().iter().all(DynArray::capacidade_plausivel)
    }

    /// Critério POSITIVO do layout, o que confirmou o item em 2026-09-06: os 4 arrays com o mesmo
    /// `size`, maior que zero. Os crivos acima só refutam (zeros passam neles); quatro campos
    /// independentes concordando num número não nulo é o que identifica o bloco. A prova tem 6
    /// disparos (`resumo`); [UNCONFIRMED] se a igualdade vale em todo disparo, então quem gateia
    /// escrita por aqui deixa de escrever quando ela falha, nunca o contrário.
    pub fn layout_confirmado(&self) -> bool {
        let n = self.resources.size;
        n > 0 && self.arrays().iter().all(|a| a.size == n)
    }

    /// Resumo de uma linha para log — a forma em que o veredito ao vivo vai aparecer. Os dois
    /// crivos saem separados para o log dizer qual caiu; nenhum deles confirma o layout. O que
    /// confirmou em 2026-09-06 foi critério externo: os 4 `size` iguais entre si em 6 disparos de
    /// tamanhos distintos (`cp77-symbols/notes/proofs/20260906-archivexl-4-5-garment-contexto-PROVADO.log`).
    pub fn resumo(&self) -> String {
        format!(
            "components={} meshes={} templates={} resources={} definition={:#x} job={:#x} estrutura_valida={} capacidade_plausivel={}",
            self.components.size,
            self.meshes.size,
            self.templates.size,
            self.resources.size,
            self.definition,
            self.job,
            self.arrays_com_estrutura_valida(),
            self.arrays_com_capacidade_plausivel()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monta um buffer com valores plantados nos offsets do header.
    fn buffer() -> Vec<u8> {
        let mut b = vec![0u8; off::TAMANHO_MINIMO];
        // ORDEM DE MEMÓRIA: entries@00 · capacity@08 · size@0C. A fixture antiga plantava
        // size em +8 e capacity em +0x0C — codificava o mesmo bug que o decodificador tinha, então
        // os testes passavam com os DOIS errados. Fixture e código errados na mesma direção é o
        // pior estado possível: cria confiança sem cobertura.
        let mut por = |off: usize, ptr: u64, size: u32, cap: u32| {
            b[off..off + 8].copy_from_slice(&ptr.to_le_bytes());
            b[off + 8..off + 12].copy_from_slice(&cap.to_le_bytes());
            b[off + 12..off + 16].copy_from_slice(&size.to_le_bytes());
        };
        por(off::COMPONENTS, 0x1111_0000, 3, 4);
        por(off::MESHES, 0x2222_0000, 2, 2);
        por(off::TEMPLATES, 0x3333_0000, 1, 8);
        por(off::RESOURCES, 0x4444_0000, 5, 8);
        b[off::DEFINITION..off::DEFINITION + 8].copy_from_slice(&0xAAAA_0000u64.to_le_bytes());
        b[off::JOB..off::JOB + 8].copy_from_slice(&0xBBBB_0000u64.to_le_bytes());
        b
    }

    #[test]
    fn le_cada_campo_do_seu_offset() {
        let c = decodificar(&buffer()).expect("decodifica");
        assert_eq!(c.components.size, 3);
        assert_eq!(c.meshes.size, 2);
        assert_eq!(c.templates.size, 1);
        assert_eq!(c.resources.size, 5);
        assert_eq!(c.definition, 0xAAAA_0000);
        assert_eq!(c.job, 0xBBBB_0000);
    }

    #[test]
    fn buffer_curto_e_none_nao_contexto_vazio() {
        // Completar com zero transformaria "buffer truncado" em "contexto vazio", que é um
        // achado falso — e achado falso aqui vira offset errado escrito no catálogo.
        assert!(decodificar(&vec![0u8; off::TAMANHO_MINIMO - 1]).is_none());
        assert!(decodificar(&[]).is_none());
    }

    #[test]
    fn contexto_zerado_passa_nos_crivos_mas_vazio() {
        // Os crivos só refutam: zeros passam nos dois sem provar que o bloco é o contexto.
        let c = decodificar(&vec![0u8; off::TAMANHO_MINIMO]).expect("decodifica");
        assert!(c.arrays_com_estrutura_valida(), "tudo zero é array vazio legítimo, não lixo");
        assert!(c.arrays_com_capacidade_plausivel());
        assert_eq!(c.components.size, 0);
    }

    #[test]
    fn size_maior_que_capacity_e_lixo() {
        let mut b = buffer();
        // size vive em +0x0C (não em +8, que é capacity) — a fixture antiga escrevia no campo errado.
        b[off::MESHES + 0x0C..off::MESHES + 0x10].copy_from_slice(&99u32.to_le_bytes()); // size=99, cap=2
        let c = decodificar(&b).expect("decodifica");
        assert!(!c.meshes.estrutura_valida());
        assert!(!c.arrays_com_estrutura_valida(), "um array com estrutura inválida derruba o crivo dos 4");
    }

    #[test]
    fn ponteiro_nulo_com_tamanho_e_lixo() {
        let mut b = buffer();
        b[off::TEMPLATES..off::TEMPLATES + 8].copy_from_slice(&0u64.to_le_bytes());
        let c = decodificar(&b).expect("decodifica");
        assert!(!c.templates.estrutura_valida(), "ponteiro nulo com size>0 não existe");
    }

    #[test]
    fn capacidade_absurda_e_lixo() {
        // O sintoma típico de offset errado: um campo qualquer lido como capacity.
        let mut b = buffer();
        b[off::RESOURCES + 8..off::RESOURCES + 12].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        let c = decodificar(&b).expect("decodifica");
        assert_eq!(c.resources.capacity, 0x7FFF_FFFF);
        assert_eq!(c.resources.size, 5);
        // size <= capacity: a estrutura não acusa nada, só a heurística pega.
        assert!(c.resources.estrutura_valida());
        assert!(!c.resources.capacidade_plausivel());
        assert!(!c.arrays_com_capacidade_plausivel());
        let r = c.resumo();
        for esperado in ["estrutura_valida=true", "capacidade_plausivel=false"] {
            assert!(r.contains(esperado), "o log tem de dizer qual crivo caiu; faltou {esperado} em: {r}");
        }
    }

    #[test]
    fn layout_confirmado_exige_os_4_size_iguais_e_nao_nulos() {
        // A fixture tem sizes 3/2/1/5: passa nos crivos e não confirma nada.
        let c = decodificar(&buffer()).expect("decodifica");
        assert!(c.arrays_com_estrutura_valida() && c.arrays_com_capacidade_plausivel());
        assert!(!c.layout_confirmado(), "crivo que só refuta não vira confirmação");

        let mut b = buffer();
        for o in [off::COMPONENTS, off::MESHES, off::TEMPLATES, off::RESOURCES] {
            b[o + 8..o + 12].copy_from_slice(&8u32.to_le_bytes());
            b[o + 0x0C..o + 0x10].copy_from_slice(&4u32.to_le_bytes());
        }
        assert!(decodificar(&b).expect("decodifica").layout_confirmado());

        let zerado = decodificar(&vec![0u8; off::TAMANHO_MINIMO]).expect("decodifica");
        assert!(!zerado.layout_confirmado(), "4 zeros concordam sem identificar o bloco");
    }

    #[test]
    fn tipos_decodificados_nao_tem_default() {
        // Busca de método por autoref: o `tem_default` de `ComDefault` (receptor `&Sonda<T>`) só
        // existe com `T: Default`; sem ele a busca desce para o de `SemDefault` (`&&Sonda<T>`).
        // Falha só pela presença de `Default`, não por qualquer erro de compilação.
        use std::marker::PhantomData;
        struct Sonda<T>(PhantomData<T>);
        trait ComDefault {
            fn tem_default(&self) -> bool {
                true
            }
        }
        impl<T: Default> ComDefault for Sonda<T> {}
        trait SemDefault {
            fn tem_default(&self) -> bool {
                false
            }
        }
        impl<T> SemDefault for &Sonda<T> {}

        assert!((&Sonda::<u32>(PhantomData)).tem_default(), "controle: a sonda enxerga Default");
        assert!(!(&Sonda::<GarmentContext>(PhantomData)).tem_default());
        assert!(!(&Sonda::<DynArray>(PhantomData)).tem_default());
    }

    #[test]
    fn resumo_traz_os_numeros_que_o_veredito_precisa() {
        let r = decodificar(&buffer()).unwrap().resumo();
        for esperado in [
            "components=3",
            "meshes=2",
            "templates=1",
            "resources=5",
            "estrutura_valida=true",
            "capacidade_plausivel=true",
        ] {
            assert!(r.contains(esperado), "faltou {esperado} em: {r}");
        }
    }

    #[test]
    fn offsets_batem_com_o_header_do_item() {
        // Transcrição literal do item `#4`. Um erro de digitação aqui vira teste vermelho, não
        // um campo lido do lugar errado.
        assert_eq!(off::COMPONENTS, 0x10);
        assert_eq!(off::MESHES, 0x20);
        assert_eq!(off::TEMPLATES, 0x30);
        assert_eq!(off::RESOURCES, 0x40);
        assert_eq!(off::DEFINITION, 0x58);
        assert_eq!(off::REQUEST, 0x68);
        assert_eq!(off::JOB, 0x138);
    }
}
