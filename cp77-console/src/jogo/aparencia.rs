//! Aparência: achar aparência e sufixos de roupa.


#[cfg(feature = "devtools")]
pub(crate) const APARENCIA_ACHAR_VM: u64 = 0x1_017d_d6c0;

#[cfg(feature = "devtools")]
pub(crate) const ROUPA_SUFIXOS_VM: u64 = 0x1_0201_dcc8;

/// `ent::EntityTemplate::FindAppearance(template, nome) -> TemplateAppearance*` (`Red::TemplateAppearance* FindAppearance(EntityTemplate*, CName)`):
/// x0 = `EntityTemplate*`, x1 = hash do CName. RE estática de 2026-07-24, confiança média-alta: função de ~0xd8 B, laço de CName com passo 0x18,
/// sorteio de aparência aleatória com 2 RNG, 9 call-sites.
pub(crate) const ENTITY_TEMPLATE_FIND_APP_VM: u64 = 0x1_019c_5e2c;

// ===== `OnAppearanceResourceLoad`: definições e `censorshipMapping` do `.app` de patch (2026-09-12) =====
// Hook do `PostLoad` do `appearanceAppearanceResource` (`0x1017df10c`, disparando ao vivo em 2026-08-15/17), instalado pela tabela quando algum alvo `.app`
// tem `appearances` ou `censorshipMapping`, antes da original. Com `appearances`, cada definição do patch (`DynArray<Handle<AppearanceDefinition>>`, `+0x98`)
// vai por nome (`+0x30`): nome `None` modifica todas as definições do alvo; nome existente modifica a 1ª com ele (menos nome que um patch anterior deste
// recurso anexou); nome novo vai pro fim como cópia do handle (strong +1, a definição fica compartilhada com o patch, como o `EmplaceBack` da referência).
// Modificar = juntar `partsValues` (8 B, `+0x50`), `resolvedDependencies` (8 B, `+0x1B8`) e `visualTags` (`+0x88`) que a definição ainda não tem, conforme
// as props. `censorshipMapping` (0x18 B, `+0xA8`): anexa todas. Ficam pra depois: `partsOverrides` (cópia funda do array interno), `components` (com o
// `OnEntityPackageLoad`) e a checagem de depot da referência. Offsets conferidos pelo RTTI na carga da tabela. Thread de job: memória, reserve e o
// incremento atômico do strong.
pub(crate) const APPRESOURCE_APPEARANCES_OFF: usize = 0x98;

pub(crate) const APPDEF_NAME_OFF: usize = 0x30;

pub(crate) const APPDEF_PARTSVALUES_OFF: usize = 0x50;

/// 51c-4: `DynArray<AppearancePartOverrides>` (0x18 B: `partResource` RaRef `+0x00`, `DynArray<PartComponentOverrides>` `+0x08`, item de 0x50 B com
/// alinhamento 0x10 e sem dono).
pub(crate) const APPDEF_PARTSOVERRIDES_OFF: usize = 0x78;

pub(crate) const APPDEF_VISUALTAGS_OFF: usize = 0x88;

