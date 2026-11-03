//! Teste ao vivo das telas do Equipment-EX e do Virtual Atelier (comando de dev `telasmod`, pelo canal `/tmp/cp77-cmd.txt` depois do
//! GAMEPLAY REAL). Cada tela abre pelo caminho do próprio mod, sem tecla nem mouse: o guarda-roupa do Equipment-EX pelo inventário
//! (`StartHubMenuEvent` com `inventory_screen`) e pelo `ShowWardrobeScreen` que o botão do mod chama, e pelo menu de guarda-roupa do jogo
//! (`inkMenuInstance_SpawnEvent` `OnOpenWardrobeMenu`, o que o guarda-roupa dos apartamentos faz, com o `OnInitialize` que o mod troca); a loja
//! do Virtual Atelier pelo `RequestVendorMenu` com `vendorId = "VirtualVendor"`, o que o clique numa loja faz; a lista de lojas pelo mesmo
//! `SpawnFromExternal` do mod, na raiz do inventário (o navegador de um computador do mundo não abre por script).
//!
//! O script de teste (`blackwall-mods-dev/bwms-telas-de-mod-teste.reds`, `PlayerPuppet.BwmsTelaModPasso`) abre, confere a árvore de widgets
//! pelas APIs do jogo e fecha. O spawn de cada tela sai do despacho de evento do motor (menu aberto pelo motor, ou evento enfileirado no
//! controller do menu), o caminho do mod. Daqui saem o ritmo e a conferência que o script não alcança: o controller que o `Item:Controller`
//! criou (`spawn_de_widget::refazer_com_controller`) é o que o motor inicializou na tela, porque o `OnInitialize` do mod preencheu os campos dele.
//!
//! O passo roda no `Call` de um `DelayCallback` que o script agenda no `DelaySystem` (`BwmsTelaModAgendar`), chamado pelo motor, e não no
//! `call_func` do tick: o que o projeto mediu diferente é esse contexto de chamada. Pelo `call_func` do tick o `GetGameInstance()` global do
//! script volta inválido (boots 247–250) e o `UnequipRequest` do A#11 caiu num invocável nulo (boots 126 e 127); chamados pelo jogo, os dois
//! valem. O passo só enfileira (pedido de menu, evento no controller) e lê a árvore de widgets; o spawn sai do `cb func` ou do `OnInitialize`
//! que o motor despacha, então a thread do `Call` não decide a da troca do controller. Em que thread o `Call` roda é [UNCONFIRMED]: as linhas
//! `[telasmod]` que o passo emite pelo `BwmsVeredito` medem (`linha_do_passo`). Daqui saem só o agendamento de cada passo e a leitura do que ele
//! devolveu; a decisão do passo seguinte fica aqui.
//!
//! Só dev; a lógica pura (pedido, protocolo do passo, espera do `Call`, resumo das trocas) roda nos testes de unidade.

use crate::cname::cname;
use crate::jogo::objetos::{fraco_vivo, gravar_handle_fraco, soltar_fraco};
use crate::rtti;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

extern "C" {
    fn pthread_main_np() -> i32;
}

/// As telas; o valor é o `tela` que o script recebe.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum Tela {
    GuardaRoupaEex = 1,
    ListaVa = 2,
    LojaVa = 3,
    GuardaRoupaDoJogoEex = 4,
}

/// Ordem da série: a lista do Virtual Atelier por último, porque é montada fora do caminho real (raiz do inventário, não o navegador) e é a que
/// mais exercita a ponte do `Codeware.UI` (`inkComponent`, busca); se ela derrubar o jogo, os vereditos das outras já estão no arquivo.
const TODAS: [Tela; 4] = [Tela::GuardaRoupaEex, Tela::GuardaRoupaDoJogoEex, Tela::LojaVa, Tela::ListaVa];

/// Bits de todas as telas (bit `tela - 1`).
const TODAS_AS_TELAS: u8 = 0b1111;

/// O que a conferência do Rust procura numa tela: o nome que o mod pede ao spawn, a classe do controller, os campos que o `OnInitialize` do
/// mod preenche no controller, o vínculo com outro controller trocado pelo `Item:Controller`, os arrays que o mod enche ao abrir (com o tamanho
/// mínimo) e os modelos de grade e lista cujos itens nascem pelo spawn assíncrono (cada grupo precisa de 1 troca assíncrona certa, ao menos).
struct Alvo {
    rotulo: &'static str,
    nome: &'static str,
    item: &'static str,
    classe: &'static str,
    campos: &'static [&'static str],
    vinculo: Option<Vinculo>,
    arrays: &'static [(&'static str, usize)],
    itens: &'static [&'static [&'static str]],
}

/// O controller da tela aponta, no `campo`, o controller trocado do spawn `item`, que o `OnInitialize` da tela cria; esse aponta de volta a tela em
/// `volta` (gravado pela tela depois do spawn) e tem `proprio` gravado pelo `OnInitialize` dele. A volta diz em que instância o `OnInitialize` da
/// tela rodou, sem passar pela leitura dos campos da tela.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Vinculo {
    campo: &'static str,
    item: &'static str,
    volta: &'static str,
    proprio: &'static str,
}

/// `EquipmentEx.reds`: `WardrobeScreenController.OnInitialize` spawna o `OutfitManager` e chama o `Setup` dele (`m_wardrobeScreen`, `:3163`); o
/// `OnInitialize` do `OutfitManagerController` grava o `m_outfitList` (`InitializeList`).
const VINCULO_EEX: Vinculo = Vinculo {
    campo: "m_outfitManager",
    item: "OutfitManager:EquipmentEx.OutfitManagerController",
    volta: "m_wardrobeScreen",
    proprio: "m_outfitList",
};

/// Grade (`InventoryGridTemplateClassifier`: slot ou item) e lista de conjuntos do guarda-roupa do Equipment-EX.
const ITENS_EEX: &[&[&str]] = &[
    &["InventoryGridItem:EquipmentEx.InventoryGridItemController", "InventoryGridSlot:EquipmentEx.InventoryGridSlotController"],
    &["OutfitListEntry:EquipmentEx.OutfitListEntryController"],
];

/// `EquipmentEx.Global.reds` `ShowWardrobeScreen` e o `OnInitialize` trocado do `WardrobeUIGameController`, os dois com o mesmo spawn, e
/// `EquipmentEx.reds` `WardrobeScreenController.OnInitialize`; `VirtualAtelier` `SetupNewTab.reds` `PopulateAtelierView` e
/// `AtelierStoresListController.OnInitialize`; `HijackVendorController.reds` e `VirtualStoreController.OnInitialize` (`InitializeCoreSystems`,
/// `InitializeWidgets`, `InitializeDataSource`). Índice = `tela - 1`.
const ALVOS: [Alvo; 4] = [
    Alvo {
        rotulo: "EEX",
        nome: "guarda-roupa do Equipment-EX",
        item: "Root:EquipmentEx.WardrobeScreenController",
        classe: "EquipmentEx.WardrobeScreenController",
        campos: &[
            "m_player",
            "m_outfitSystem",
            "m_buttonHints",
            "m_outfitManager",
            "m_inventoryGridController",
            "m_inventoryGridDataView",
            "m_searchInput",
            "m_previewWrapper",
        ],
        vinculo: Some(VINCULO_EEX),
        arrays: &[],
        itens: ITENS_EEX,
    },
    // Sem loja registrada a lista não tem item: o `AtelierStoreItem` do log é só a medida do modelo.
    Alvo {
        rotulo: "VA-LISTA",
        nome: "lista de lojas do Virtual Atelier",
        item: "AtelierStores:VirtualAtelier.UI.AtelierStoresListController",
        classe: "VirtualAtelier.UI.AtelierStoresListController",
        campos: &["player", "system", "buttonHints", "config", "storesContainer"],
        vinculo: None,
        arrays: &[],
        itens: &[],
    },
    // `currentPageItemData` enche no `MaterializeStockItem` da página (`VirtualStoreController.reds:1040`), que sai do `DelayCallback` do
    // `ContinuePageRender`: separa "o dado não chegou à grade" de "o item não nasceu". Os itens são `StoreItemSmall`/`StoreItemBig`
    // (`VirtualStoreTemplateClassifier`); o `StoreHeader` só entra na busca.
    Alvo {
        rotulo: "VA-LOJA",
        nome: "loja virtual do Virtual Atelier",
        item: "VirtualStore:VirtualAtelier.UI.VirtualStoreController",
        classe: "VirtualAtelier.UI.VirtualStoreController",
        campos: &["player", "storesManager", "virtualStore", "storeListController", "buttonHintsController", "vendorName", "storeDataView"],
        vinculo: None,
        arrays: &[("virtualStock", 1), ("currentPageItemData", 1)],
        itens: &[&["StoreItemSmall:VirtualAtelier.UI.VirtualStoreItemController", "StoreItemBig:VirtualAtelier.UI.VirtualStoreItemController"]],
    },
    Alvo {
        rotulo: "EEX-GR",
        nome: "guarda-roupa do Equipment-EX pelo menu de guarda-roupa do jogo",
        item: "Root:EquipmentEx.WardrobeScreenController",
        classe: "EquipmentEx.WardrobeScreenController",
        campos: &[
            "m_player",
            "m_outfitSystem",
            "m_buttonHints",
            "m_outfitManager",
            "m_inventoryGridController",
            "m_inventoryGridDataView",
            "m_searchInput",
            "m_previewWrapper",
        ],
        vinculo: Some(VINCULO_EEX),
        arrays: &[],
        itens: ITENS_EEX,
    },
];

fn alvo(tela: Tela) -> &'static Alvo {
    &ALVOS[tela as usize - 1]
}

// Protocolo do retorno de `BwmsTelaModPasso` (o mesmo no `.reds`): 1–199 é o próximo passo (abrir até 99, fechar de 100 em diante);
// 200/201 = estrutura conferida certa/errada, e o Rust confere o controller antes do passo 100; 300 = fechou; 301 = o menu fechou e a saída do
// mod falhou; -2 = fechamento não confirmado; -1 = nada aberto (parou antes de abrir, ou o fechamento não achou o menu da tela). O `call_func`
// entrega o retorno num buffer zerado: 0 é o retorno que o script não escreveu, e fica fora do protocolo.
const ESTRUTURA_CERTA: i32 = 200;
const ESTRUTURA_ERRADA: i32 = 201;
const PASSO_DE_FECHAR: i32 = 100;
const FECHOU: i32 = 300;
const FECHOU_SEM_SAIDA_DO_MOD: i32 = 301;
const NAO_FECHOU: i32 = -2;
const NADA_ABERTO: i32 = -1;

/// Teto de chamadas por fase (abrir, fechar), acima do roteiro mais longo do `.reds` (abrir: 26 chamadas no EEX; fechar: 13): só pega o
/// script que não avança.
const TETO_DE_PASSOS: u32 = 60;

/// Um passo por segundo, o atraso do `DelayCallback` de cada passo: as animações dos mods vão de 0,2 a 0,8 s (`EquipmentEx.Global.reds:445`,
/// `:490`, `:529`) e a grade do Equipment-EX enche 0,5 s depois (`m_inventoryGridUpdateDelay`, `EquipmentEx.reds:3907`).
const CADENCIA_MS: u32 = 1000;

/// Teto da espera pela `vez` do passo agendado em `BwmsTelaModFeito`, além da cadência. Sem ela nesse tempo, ou o `Call` não rodou
/// (`DelaySystem` parado: jogo em pausa, motor dormindo), ou a leitura não volta; o último valor lido separa os dois.
const ESPERA_DO_CALL: Duration = Duration::from_secs(10);

// Métodos do `PlayerPuppet` no `.reds` que o tick chama: agendar o passo, ler a `vez` e o retorno do último que rodou, e desistir do agendado
// (o `Call` que chegar depois não roda o passo).
const AGENDAR: &str = "BwmsTelaModAgendar";
const FEITO: &str = "BwmsTelaModFeito";
const PARAR: &str = "BwmsTelaModParar";

// `BwmsTelaModFeito` = `vez * FEITO_VEZ + retorno + FEITO_DESLOCAMENTO`, numa gravação só de Int32 no `Call`: o tick, noutra thread, lê a
// `vez` e o retorno de uma vez, sem depender da ordem de duas gravações (no ARM64 a 2ª pode aparecer antes da 1ª). Retorno fora de
// `-FEITO_DESLOCAMENTO..FEITO_DESLOCAMENTO` já está fora do protocolo e o script grava `FEITO_DESLOCAMENTO - 1`.
const FEITO_VEZ: i32 = 1024;
const FEITO_DESLOCAMENTO: i32 = 512;

/// `(vez, retorno)` do valor de `BwmsTelaModFeito`.
fn separar_feito(feito: i32) -> (i32, i32) {
    (feito.div_euclid(FEITO_VEZ), feito.rem_euclid(FEITO_VEZ) - FEITO_DESLOCAMENTO)
}

/// O passo agendado na `vez`, visto do tick.
#[derive(Debug, PartialEq)]
enum Espera {
    /// O `Call` rodou o passo, com este retorno.
    Rodou(i32),
    Aguardando,
    /// Passou do teto sem a `vez` do pedido.
    Parado,
}

/// `feito` é `(vez, retorno)` do último passo que o script rodou (`None` com a leitura falha); `decorrido`, o tempo desde o agendamento.
fn esperar(feito: Option<(i32, i32)>, vez: i32, decorrido: Duration) -> Espera {
    match feito {
        Some((v, retorno)) if v == vez => Espera::Rodou(retorno),
        _ if decorrido < Duration::from_millis(u64::from(CADENCIA_MS)) + ESPERA_DO_CALL => Espera::Aguardando,
        _ => Espera::Parado,
    }
}

/// Como a tela terminou.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fechamento {
    /// O menu fechou pela saída do mod.
    Fechou,
    /// O menu fechou (oráculo válido, falso depois), mas a saída do mod falhou: a tela não passa e a série segue.
    MenuFechouSemSaidaDoMod,
    /// Fechamento não confirmado: o menu pode estar aberto.
    NaoFechou,
    /// Nada ficou aberto: a tela parou antes do menu.
    NadaAberto,
}

impl Fechamento {
    fn texto(self) -> &'static str {
        match self {
            Fechamento::Fechou => "true",
            Fechamento::MenuFechouSemSaidaDoMod => "false (menu fechado, saída do mod falhou)",
            Fechamento::NaoFechou => "false",
            Fechamento::NadaAberto => "nada aberto",
        }
    }
}

#[derive(Debug, PartialEq)]
enum Acao {
    Seguir(i32),
    Conferir { estrutura_ok: bool },
    Encerrar(Fechamento),
    Interromper,
}

/// O que fazer com o retorno do passo `passo`, na chamada nº `chamadas` da fase. Estourado o teto, abrindo vai para a conferência e o
/// fechamento; fechando, encerra sem o fechamento confirmado. Valor fora do protocolo (0 inclusive) interrompe a série.
fn decidir(retorno: i32, passo: i32, chamadas: u32) -> Acao {
    match retorno {
        ESTRUTURA_CERTA => Acao::Conferir { estrutura_ok: true },
        ESTRUTURA_ERRADA => Acao::Conferir { estrutura_ok: false },
        FECHOU => Acao::Encerrar(Fechamento::Fechou),
        FECHOU_SEM_SAIDA_DO_MOD => Acao::Encerrar(Fechamento::MenuFechouSemSaidaDoMod),
        NAO_FECHOU => Acao::Encerrar(Fechamento::NaoFechou),
        NADA_ABERTO => Acao::Encerrar(Fechamento::NadaAberto),
        1..=199 if chamadas >= TETO_DE_PASSOS && passo < PASSO_DE_FECHAR => Acao::Conferir { estrutura_ok: false },
        1..=199 if chamadas >= TETO_DE_PASSOS => Acao::Encerrar(Fechamento::NaoFechou),
        1..=199 => Acao::Seguir(retorno),
        _ => Acao::Interromper,
    }
}

/// Passo da tela em curso e chamadas da fase. A contagem zera ao entrar no fechamento: o fechamento tem o teto inteiro, e o pedido de fechar
/// o menu (passos 100 a 103 do script) sai mesmo quando a abertura estourou.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Passos {
    passo: i32,
    chamadas: u32,
}

impl Passos {
    const fn novo() -> Self {
        Passos { passo: 1, chamadas: 0 }
    }

    fn aplicar(&mut self, retorno: i32) -> Acao {
        self.chamadas += 1;
        let acao = decidir(retorno, self.passo, self.chamadas);
        match acao {
            Acao::Seguir(p) => self.passo = p,
            Acao::Conferir { .. } => {
                self.passo = PASSO_DE_FECHAR;
                self.chamadas = 0;
            }
            Acao::Encerrar(_) | Acao::Interromper => {}
        }
        acao
    }

    /// O passo 1 já rodou (é ele que pede o menu): daqui em diante a tela pode estar aberta.
    const fn pode_ter_aberto(&self) -> bool {
        self.passo > 1 || self.chamadas > 0
    }
}

/// A série segue para a tela seguinte só com o menu fechado ou sem nada aberto: com um menu talvez preso, a próxima tela abriria em cima
/// (testes de UI em série; foco e fila de tecla são globais). A saída do mod que falhou com o menu fechado reprova a tela, não a série.
fn serie_segue(fechamento: Fechamento) -> bool {
    fechamento != Fechamento::NaoFechou
}

fn tela_ok(estrutura: Option<bool>, controller: Option<bool>, fechamento: Fechamento) -> bool {
    estrutura == Some(true) && controller == Some(true) && fechamento == Fechamento::Fechou
}

const USO: &str = "uso: telasmod [tudo|eex|eexgr|valista|valoja]... — eex = guarda-roupa do Equipment-EX pelo inventário, eexgr = o mesmo pelo \
                   menu de guarda-roupa do jogo, valista = lista de lojas do Virtual Atelier, valoja = loja virtual do Virtual Atelier (ex.: \
                   telasmod eex valoja)";

/// Bits das telas pedidas (bit `tela - 1`). Sem argumento ou `tudo`: todas.
fn telas_pedidas(args: &[&str]) -> Result<u8, String> {
    if args.is_empty() {
        return Ok(TODAS_AS_TELAS);
    }
    let mut bits = 0u8;
    for a in args {
        bits |= match a.to_ascii_lowercase().as_str() {
            "tudo" => TODAS_AS_TELAS,
            "eex" => 1 << (Tela::GuardaRoupaEex as u8 - 1),
            "eexgr" => 1 << (Tela::GuardaRoupaDoJogoEex as u8 - 1),
            "valista" => 1 << (Tela::ListaVa as u8 - 1),
            "valoja" => 1 << (Tela::LojaVa as u8 - 1),
            _ => return Err(format!("\"{a}\" não é tela do telasmod. {USO}")),
        };
    }
    Ok(bits)
}

fn fila_do_pedido(bits: u8) -> VecDeque<Tela> {
    TODAS.into_iter().filter(|t| bits & (1 << (*t as u8 - 1)) != 0).collect()
}

/// Trocas de um nome de spawn: certas pelo spawn síncrono (a tela, e a medida de cada modelo de grade e lista), certas na entrega do assíncrono
/// (os itens) e falhas.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Contagem {
    sincronas: u32,
    assincronas: u32,
    falhas: u32,
}

/// `(nome, contagem)` por nome de spawn, na ordem da 1ª aparição; cada troca é `(nome, certa, assíncrona)`.
fn resumo_por_nome<'a>(trocas: impl Iterator<Item = (&'a str, bool, bool)>) -> Vec<(&'a str, Contagem)> {
    let mut saida: Vec<(&str, Contagem)> = Vec::new();
    for (nome, ok, assincrona) in trocas {
        let i = match saida.iter().position(|(n, _)| *n == nome) {
            Some(i) => i,
            None => {
                saida.push((nome, Contagem::default()));
                saida.len() - 1
            }
        };
        let c = &mut saida[i].1;
        match (ok, assincrona) {
            (false, _) => c.falhas += 1,
            (true, false) => c.sincronas += 1,
            (true, true) => c.assincronas += 1,
        }
    }
    saida
}

/// Os itens dos modelos de grade e lista nasceram com o controller do mod: cada grupo de `itens` tem 1 troca certa na entrega do assíncrono, ao
/// menos. Devolve também o texto por grupo (`nome N` das trocas assíncronas).
fn itens_ok(resumo: &[(&str, Contagem)], itens: &[&[&str]]) -> (bool, String) {
    let mut ok = true;
    let mut grupos = Vec::new();
    for grupo in itens {
        let contas: Vec<(&str, u32)> =
            grupo.iter().map(|n| (*n, resumo.iter().find(|(r, _)| r == n).map_or(0, |(_, c)| c.assincronas))).collect();
        ok &= contas.iter().any(|(_, k)| *k > 0);
        grupos.push(contas.iter().map(|(n, k)| format!("{n} {k}")).collect::<Vec<_>>().join(" + "));
    }
    let texto = if grupos.is_empty() { "—".to_string() } else { grupos.join("; ") };
    (ok, texto)
}

// ===== Trocas do `Item:Controller` vistas durante a tela =====

/// Uma troca: o nome pedido ao spawn, o controller novo e a raiz do item em handles fracos (o teste não segura nada vivo), se o spawn rodou
/// na thread principal e se a troca foi na entrega do spawn assíncrono. Controller `[0, 0]` = a troca falhou.
struct Troca {
    nome: String,
    controller: [u64; 2],
    raiz: [u64; 2],
    principal: bool,
    assincrona: bool,
}

static TROCAS: Mutex<Vec<Troca>> = Mutex::new(Vec::new());
static TROCAS_FORA_DO_TETO: AtomicU64 = AtomicU64::new(0);

/// Spawns vazios com nome `Item:Controller` que o gancho só repassou à original (fora da thread principal e sem script em execução nela,
/// `spawn_de_widget::na_game_thread`), por nome, na tela em curso. Separa, no `-CONTROLLER`, "o spawn da tela não aconteceu" de "o portão do
/// gancho não trocou".
static SEM_TROCA: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());

/// A grade do Equipment-EX spawna um modelo por slot e por item visível: algumas dezenas por tela.
const TETO_DE_TROCAS: usize = 512;

/// A série está rodando: só então o gancho anota trocas.
static RODANDO: AtomicBool = AtomicBool::new(false);

/// A série está rodando: a entrega do assíncrono só volta o nome do item pra texto com ela.
pub(crate) fn rodando() -> bool {
    RODANDO.load(Ordering::Relaxed)
}

/// Anota a troca do `refazer_com_controller` (síncrona) ou da entrega do spawn assíncrono, com a série rodando. `novo` nulo = a troca falhou; `raiz`
/// é o `rootWidget` do item (handle forte do motor, copiado como fraco).
pub(crate) unsafe fn anotar_troca(nome: &str, novo: *mut c_void, raiz: [u64; 2], assincrona: bool) {
    if !RODANDO.load(Ordering::Relaxed) {
        return;
    }
    let Ok(mut v) = TROCAS.lock() else { return };
    if v.len() >= TETO_DE_TROCAS {
        TROCAS_FORA_DO_TETO.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let mut controller = [0u64; 2];
    if !novo.is_null() {
        rtti::make_weak_handle(controller.as_mut_ptr() as *mut c_void, novo);
    }
    let mut fraca = [0u64; 2];
    if raiz[0] != 0 && raiz[1] != 0 {
        gravar_handle_fraco(fraca.as_mut_ptr(), raiz);
    }
    v.push(Troca { nome: nome.to_string(), controller, raiz: fraca, principal: pthread_main_np() != 0, assincrona });
}

/// Anota o spawn vazio de `item` que o gancho repassou sem troca, com a série rodando e o nome na forma `Item:Controller`.
pub(crate) fn anotar_sem_troca(item: u64) {
    if !RODANDO.load(Ordering::Relaxed) {
        return;
    }
    let Some(nome) = crate::cname::name_of(item).filter(|n| super::spawn_de_widget::item_e_controller(n).is_some()) else { return };
    let Ok(mut v) = SEM_TROCA.lock() else { return };
    match v.iter().position(|(n, _)| *n == nome) {
        Some(i) => v[i].1 += 1,
        None if v.len() < TETO_DE_TROCAS => v.push((nome, 1)),
        None => {}
    }
}

/// Solta os fracos anotados e esvazia as listas (começo de cada tela e fim da série).
unsafe fn esvaziar_trocas() {
    let antigas = TROCAS.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
    TROCAS_FORA_DO_TETO.store(0, Ordering::Relaxed);
    if let Ok(mut v) = SEM_TROCA.lock() {
        v.clear();
    }
    for mut t in antigas {
        for h in [&mut t.controller, &mut t.raiz] {
            if h[1] != 0 {
                soltar_fraco(h);
            }
        }
    }
}

// ===== Conferência do controller =====

/// O que a leitura de um campo `ref`/`wref` (`{instância, bloco}`) achou. Os casos de falha ficam separados: só o caso diz se o `OnInitialize`
/// não gravou ou se a leitura erra. Os campos vão com o nome da fonte do mod (`m_player`); o RTTI guarda sem o `m_` e o
/// `rtti::resolve_prop_in_class` acha pelos dois.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Leitura {
    Vivo(u64),
    /// Nenhuma propriedade com o nome na classe nem nos pais.
    SemPropriedade,
    /// O campo mora no `valueHolder` (`+0x38`) e ele é nulo.
    HolderNulo { off: u32 },
    /// O endereço do campo não é legível.
    Ilegivel { off: u32, no_holder: bool, base: u64 },
    /// `{0, _}` ou `{_, 0}`: nada gravado.
    Nulo { off: u32, no_holder: bool, base: u64 },
    /// Instância com a contagem forte em 0 ou com o bloco ilegível.
    Morto { off: u32, no_holder: bool, base: u64 },
}

impl Leitura {
    fn viva(self) -> Option<u64> {
        match self {
            Leitura::Vivo(p) => Some(p),
            _ => None,
        }
    }

    fn texto(self) -> String {
        let onde = |off: u32, no_holder: bool, base: u64| format!("+{off:#x} {} {base:#x}", if no_holder { "no holder" } else { "no objeto" });
        match self {
            Leitura::Vivo(p) => format!("vivo {p:#x}"),
            Leitura::SemPropriedade => "sem a propriedade".to_string(),
            Leitura::HolderNulo { off } => format!("holder nulo (+{off:#x})"),
            Leitura::Ilegivel { off, no_holder, base } => format!("ilegível ({})", onde(off, no_holder, base)),
            Leitura::Nulo { off, no_holder, base } => format!("nulo ({})", onde(off, no_holder, base)),
            Leitura::Morto { off, no_holder, base } => format!("morto ({})", onde(off, no_holder, base)),
        }
    }
}

/// O campo `nome` do objeto de script, pelo `valueHolder` quando o campo mora lá (`rtti::field_ptr`), com o caso de falha.
unsafe fn ler_campo(obj: *mut c_void, nome: &str) -> Leitura {
    let Some((off, _, no_holder)) = rtti::resolve_prop_in_class(rtti::class_of(obj), nome) else { return Leitura::SemPropriedade };
    let base = if no_holder { (obj as *const u8).add(0x38).cast::<u64>().read_unaligned() } else { obj as u64 };
    if base == 0 {
        return Leitura::HolderNulo { off };
    }
    let p = rtti::field_ptr(obj, off, no_holder) as *const u8;
    if p.is_null() || !crate::memoria::is_readable(p as *const c_void, 16) {
        return Leitura::Ilegivel { off, no_holder, base };
    }
    let (inst, bloco) = ((p as *const u64).read_unaligned(), (p.add(8) as *const u64).read_unaligned());
    if inst == 0 || bloco == 0 {
        return Leitura::Nulo { off, no_holder, base };
    }
    super::weak_vivo(p, 0).map_or(Leitura::Morto { off, no_holder, base }, Leitura::Vivo)
}

/// Tamanho do `array` no campo `nome`; `None` sem o campo ou com outro tipo.
unsafe fn tamanho_do_array(obj: *mut c_void, nome: &str) -> Option<usize> {
    let (off, tipo, no_holder) = rtti::resolve_prop_in_class(rtti::class_of(obj), nome)?;
    if rtti::type_kind(tipo) != Some(rtti::ertti::ARRAY) {
        return None;
    }
    let p = rtti::field_ptr(obj, off, no_holder);
    if p.is_null() || !crate::memoria::is_readable(p as *const c_void, 0x10) {
        return None;
    }
    Some(crate::jogo::listas::dynarray_no_campo(p as *const u8, 0).1)
}

fn nome_da_classe(obj: *mut c_void) -> String {
    let hash = unsafe { rtti::type_name_getname(rtti::class_of(obj)) };
    crate::cname::name_of(hash).unwrap_or_else(|| format!("{hash:#x}"))
}

/// O vínculo da tela com o controller trocado do spawn `vinculo.item` (o último certo), no texto do veredito: o `campo` da tela aponta ele; ele
/// aponta de volta a tela (`volta`, igual ao controller da troca da tela = o `OnInitialize` rodou nele); o `proprio` dele lido. `true` com o campo
/// da tela apontando o controller trocado.
unsafe fn conferir_vinculo(obj: *mut c_void, vinculo: &Vinculo, trocas: &[Troca]) -> (bool, String) {
    let aponta = ler_campo(obj, vinculo.campo);
    let Some(sub) = trocas.iter().rev().find(|s| s.nome == vinculo.item && s.controller[0] != 0) else {
        return (false, format!("{} = controller de {}: false (sem troca certa dele; o campo: {})", vinculo.campo, vinculo.item, aponta.texto()));
    };
    if !fraco_vivo(sub.controller[1]) {
        return (false, format!("{} = controller de {}: false (o controller {:#x} da troca já destruído)", vinculo.campo, vinculo.item, sub.controller[0]));
    }
    let ok = aponta.viva() == Some(sub.controller[0]);
    let s = sub.controller[0] as *mut c_void;
    let volta = match ler_campo(s, vinculo.volta) {
        Leitura::Vivo(p) if p == obj as u64 => "igual ao controller trocado da tela".to_string(),
        Leitura::Vivo(p) => format!("diferente: {p:#x}, outro controller da tela"),
        outra => outra.texto(),
    };
    (
        ok,
        format!(
            "{} = controller de {}: {ok} (o campo: {}); {} dele: {volta}; {} dele: {}",
            vinculo.campo,
            vinculo.item,
            aponta.texto(),
            vinculo.volta,
            vinculo.proprio,
            ler_campo(s, vinculo.proprio).texto()
        ),
    )
}

/// O controller trocado na última troca certa de `alvo.item` está vivo, é da classe pedida e passou pelo `OnInitialize` do mod (campos de
/// `alvo.campos` vivos, vínculo com o outro controller trocado, arrays cheios), a raiz do item está viva e os itens de grade e lista nasceram com
/// o controller do mod (`alvo.itens`). Uma linha de veredito.
unsafe fn conferir_controller(alvo: &Alvo) -> bool {
    let Ok(v) = TROCAS.lock() else {
        crate::veredito_dev!("[telasmod] VEREDITO-{}-CONTROLLER lista de trocas indisponível; controller-ok=false", alvo.rotulo);
        return false;
    };
    let resumo = resumo_por_nome(v.iter().map(|t| (t.nome.as_str(), t.controller[0] != 0, t.assincrona)));
    let resumo_txt =
        resumo.iter().take(12).map(|(n, c)| format!("{n} {}+{}/{}", c.sincronas, c.assincronas, c.falhas)).collect::<Vec<_>>().join(", ");
    let (itens_ok, itens_txt) = itens_ok(&resumo, alvo.itens);
    let pedidas = v.iter().filter(|t| t.nome == alvo.item).count();
    let fora_da_principal = v.iter().filter(|t| !t.principal).count();
    let sem_troca = SEM_TROCA.lock().map(|s| s.clone()).unwrap_or_default();
    let sem_troca_do_alvo = sem_troca.iter().find(|(n, _)| n == alvo.item).map_or(0, |(_, k)| *k);
    let sem_troca_txt = sem_troca.iter().take(12).map(|(n, k)| format!("{n} {k}")).collect::<Vec<_>>().join(", ");
    let ultima = v.iter().rev().find(|t| t.nome == alvo.item && t.controller[0] != 0);
    let classe_pedida = rtti::Registry::obtain().map_or(std::ptr::null_mut(), |r| r.script_class(cname(alvo.classe)));
    let mut ok = false;
    let detalhe = match ultima {
        None => format!(
            "nenhuma troca certa de {} ({pedidas} pedida(s); {sem_troca_do_alvo} spawn(s) dele sem troca, fora da thread principal e sem script em \
             execução)",
            alvo.item
        ),
        Some(t) if !fraco_vivo(t.controller[1]) => format!("controller {:#x} da troca já destruído", t.controller[0]),
        Some(t) => {
            let obj = t.controller[0] as *mut c_void;
            let classe_ok = rtti::sane(classe_pedida) && crate::register::class_is_kind_of(rtti::class_of(obj), classe_pedida);
            let faltando: Vec<(&str, Leitura)> =
                alvo.campos.iter().map(|c| (*c, ler_campo(obj, c))).filter(|(_, l)| l.viva().is_none()).collect();
            let vinculo = alvo.vinculo.map(|vinculo| conferir_vinculo(obj, &vinculo, &v));
            let arrays: Vec<(&str, Option<usize>, usize)> = alvo.arrays.iter().map(|(n, min)| (*n, tamanho_do_array(obj, n), *min)).collect();
            let arrays_ok = arrays.iter().all(|(_, n, min)| n.is_some_and(|n| n >= *min));
            let raiz = fraco_vivo(t.raiz[1]);
            ok = classe_ok && faltando.is_empty() && vinculo.as_ref().is_none_or(|(b, _)| *b) && arrays_ok && raiz && itens_ok;
            format!(
                "controller {:#x} vivo, classe {} (é a pedida: {classe_ok}); campos do OnInitialize {}/{} (faltando: {}); vínculo {}; arrays {}; raiz \
                 viva {raiz}",
                t.controller[0],
                nome_da_classe(obj),
                alvo.campos.len() - faltando.len(),
                alvo.campos.len(),
                if faltando.is_empty() {
                    "nenhum".to_string()
                } else {
                    faltando.iter().map(|(c, l)| format!("{c} [{}]", l.texto())).collect::<Vec<_>>().join(" ")
                },
                vinculo.map_or("—".to_string(), |(_, texto)| texto),
                if arrays.is_empty() {
                    "—".to_string()
                } else {
                    arrays.iter().map(|(n, t, m)| format!("{n}={} (mín {m})", t.map_or("?".to_string(), |t| t.to_string()))).collect::<Vec<_>>().join(" ")
                },
            )
        }
    };
    let injecao = match super::spawn_de_widget::EXTERNAS_OFF.load(Ordering::Relaxed) {
        0 => "não resolvida".to_string(),
        super::spawn_de_widget::EXTERNAS_RECUSADO => "recusada pelo RTTI".to_string(),
        off => format!("em +{off:#x}"),
    };
    let (n_entrega, media_entrega, maior_entrega) = super::spawn_de_widget::custo_da_entrega();
    crate::veredito_dev!(
        "[telasmod] VEREDITO-{}-CONTROLLER {}: {pedidas} troca(s) pedida(s); {detalhe}; itens de grade e lista trocados na entrega do assíncrono: \
         {itens_txt} (itens-ok={itens_ok}); trocas na tela (síncronas+assíncronas/falhas): {}{}; trocas fora da thread principal: \
         {fora_da_principal} de {}; Item:Controller sem troca (fora da principal, sem script em execução): {}; hooks do assíncrono e da entrega no ar: \
         {}; troca na entrega desde o boot: {n_entrega}, média {media_entrega:.1} µs, maior {maior_entrega:.1} µs; injeção de dependência {injecao}; \
         controller-ok={ok}",
        alvo.rotulo,
        alvo.item,
        if resumo_txt.is_empty() { "nenhuma" } else { &resumo_txt },
        match TROCAS_FORA_DO_TETO.load(Ordering::Relaxed) {
            0 => String::new(),
            n => format!(" (+{n} fora do teto)"),
        },
        v.len(),
        if sem_troca_txt.is_empty() { "nenhum" } else { &sem_troca_txt },
        super::spawn_de_widget::assincrono_no_ar(),
    );
    ok
}

/// O controller da última troca certa de `item` já foi solto pelo motor; `None` sem troca.
unsafe fn controller_solto(item: &str) -> Option<bool> {
    let v = TROCAS.lock().ok()?;
    v.iter().rev().find(|t| t.nome == item && t.controller[0] != 0).map(|t| !fraco_vivo(t.controller[1]))
}

/// Pode abrir a tela: ganchos do spawn no ar e a classe do controller do mod no RTTI.
unsafe fn pode_abrir(alvo: &Alvo) -> Result<(), String> {
    use super::spawn_de_widget::{ORIG_SPAWN_FROM_EXTERNAL, ORIG_SPAWN_FROM_LOCAL};
    if ORIG_SPAWN_FROM_LOCAL.load(Ordering::Relaxed) == 0 || ORIG_SPAWN_FROM_EXTERNAL.load(Ordering::Relaxed) == 0 {
        return Err("ganchos do SpawnFromLocal/SpawnFromExternal fora do ar (o on_load não os instalou): sem eles não há Item:Controller".into());
    }
    let classe = rtti::Registry::obtain().map_or(std::ptr::null_mut(), |r| r.script_class(cname(alvo.classe)));
    if !rtti::sane(classe) {
        return Err(format!(
            "classe {} ausente no RTTI: o mod não compilou ou não está em <jogo>/Mods/ (ver a seção Scripts de Mods/BWMS-RELATORIO.txt)",
            alvo.classe
        ));
    }
    Ok(())
}

// ===== Série =====

/// Passo agendado no `DelaySystem`: a `vez` que o script devolveu ao agendar, o passo e quando.
#[derive(Clone, Copy, Debug)]
struct Agendado {
    vez: i32,
    passo: i32,
    em: Instant,
}

struct Rodada {
    fila: VecDeque<Tela>,
    tela: Option<Tela>,
    passos: Passos,
    agendado: Option<Agendado>,
    estrutura: Option<bool>,
    controller: Option<bool>,
    /// `LINHAS_DO_PASSO` no começo da tela em curso.
    linhas_antes: [u64; 2],
}

static RODADA: Mutex<Option<Rodada>> = Mutex::new(None);
static PEDIDO: AtomicU8 = AtomicU8::new(0);

/// Linhas `[telasmod]` que o script emitiu pelo `BwmsVeredito` na série, fora da thread principal e nela. Só o passo emite (os `cb func` e
/// os ganchos dos menus, não), então é a thread em que o `Call` do `DelaySystem` rodou o passo.
static LINHAS_DO_PASSO: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

/// Chamada pelo `BwmsVeredito` (`diagnostico::tramp_veredito`), na thread do script que emitiu a linha.
pub(crate) fn linha_do_passo(tag: &str) {
    if RODANDO.load(Ordering::Relaxed) && tag.starts_with("[telasmod]") {
        let principal = unsafe { pthread_main_np() } != 0;
        LINHAS_DO_PASSO[usize::from(principal)].fetch_add(1, Ordering::Relaxed);
    }
}

fn linhas_do_passo() -> [u64; 2] {
    LINHAS_DO_PASSO.each_ref().map(|c| c.load(Ordering::Relaxed))
}

/// O comando `telasmod [tudo|eex|eexgr|valista|valoja]...`: arma a série; o tick a roda.
pub(crate) fn pedir(args: &[&str]) {
    let bits = match telas_pedidas(args) {
        Ok(b) => b,
        Err(uso) => return crate::resposta!("{uso}"),
    };
    if RODANDO.load(Ordering::Relaxed) || PEDIDO.load(Ordering::Relaxed) != 0 {
        return crate::resposta!("telasmod já está rodando: espere a linha \"[telasmod] série encerrada\" em /tmp/bwms-vereditos.log");
    }
    PEDIDO.store(bits, Ordering::Relaxed);
    let nomes = fila_do_pedido(bits).iter().map(|t| alvo(*t).rotulo).collect::<Vec<_>>().join(", ");
    crate::resposta!("telasmod: {nomes}, sem tecla, um passo por segundo no DelaySystem do jogo; vereditos [telasmod] em /tmp/bwms-vereditos.log");
    crate::veredito_dev!("[telasmod] pedido: {nomes}");
}

/// Tick do `cp77_tick`, em qualquer thread: nada sem pedido nem série, nem em chamada aninhada do executor (o `call_func` do agendamento e
/// da leitura).
pub(crate) fn tick() {
    if PEDIDO.load(Ordering::Relaxed) == 0 && !RODANDO.load(Ordering::Relaxed) {
        return;
    }
    if crate::inicio::boot::exec_nested() || !crate::inicio::boot::gameplay_real() {
        return;
    }
    let player = crate::current_player();
    if player.is_null() {
        return;
    }
    let Ok(mut guarda) = RODADA.try_lock() else { return };
    if guarda.is_none() {
        let bits = PEDIDO.swap(0, Ordering::Relaxed);
        if bits == 0 {
            return;
        }
        for c in &LINHAS_DO_PASSO {
            c.store(0, Ordering::Relaxed);
        }
        RODANDO.store(true, Ordering::Relaxed);
        crate::veredito_dev!(
            "[telasmod] série começou: cada passo roda no Call de um DelayCallback do DelaySystem ({AGENDAR}, {CADENCIA_MS} ms, fora da dilatação \
             de tempo); o tick que agenda está na thread principal: {}",
            unsafe { pthread_main_np() } != 0
        );
        *guarda = Some(Rodada {
            fila: fila_do_pedido(bits),
            tela: None,
            passos: Passos::novo(),
            agendado: None,
            estrutura: None,
            controller: None,
            linhas_antes: [0, 0],
        });
    }
    let Some(r) = guarda.as_mut() else { return };
    if unsafe { avancar(r, player) } {
        *guarda = None;
        unsafe { esvaziar_trocas() };
        RODANDO.store(false, Ordering::Relaxed);
        let [fora, principal] = linhas_do_passo();
        crate::veredito_dev!("[telasmod] série encerrada; linhas do passo na thread principal: {principal} de {}", fora + principal);
    }
}

/// O `Int32` que o método `metodo` do jogador devolve com `args`; `None` sem o método no bundle ou com a chamada sem volta.
unsafe fn chamar(player: *mut c_void, metodo: &str, args: &[rtti::Arg]) -> Option<i32> {
    let b = rtti::resolve_in_class(rtti::class_of(player), metodo).and_then(|rf| rtti::call_func(&rf, player, args))?;
    Some(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Fim da série com um menu talvez aberto: as telas que faltavam não rodam neste boot. Devolve `true` (a série acabou).
fn abortar_serie(r: &mut Rodada, motivo: &str) -> bool {
    let faltaram = r.fila.iter().map(|t| alvo(*t).rotulo).collect::<Vec<_>>().join(", ");
    crate::veredito_dev!(
        "[telasmod] série abortada: {motivo}, o menu pode estar aberto; telas não rodadas: {}; nenhum outro teste de UI neste boot",
        if faltaram.is_empty() { "nenhuma" } else { &faltaram }
    );
    r.fila.clear();
    true
}

/// O passo da tela não rodou: depois do 1º passo a tela pode estar pedida e a série aborta; antes dele nada foi pedido e a série só acaba.
fn sem_o_passo(r: &mut Rodada, motivo: &str) -> bool {
    if r.passos.pode_ter_aberto() {
        return abortar_serie(r, motivo);
    }
    r.fila.clear();
    true
}

/// Um tick da série: com um passo agendado, espera o `Call` dele e aplica o retorno; sem passo agendado, agenda o seguinte (ou o 1º da
/// próxima tela). `true` quando a série acabou.
unsafe fn avancar(r: &mut Rodada, player: *mut c_void) -> bool {
    if let Some(ag) = r.agendado {
        // Antes do atraso do `DelayCallback` não há o que ler: o tick não chama o script à toa.
        if ag.em.elapsed() < Duration::from_millis(u64::from(CADENCIA_MS)) {
            return false;
        }
        let rotulo = r.tela.map_or("?", |t| alvo(t).rotulo);
        let lido = chamar(player, FEITO, &[]).map(separar_feito);
        let retorno = match esperar(lido, ag.vez, ag.em.elapsed()) {
            Espera::Aguardando => return false,
            Espera::Parado => {
                let parar = chamar(player, PARAR, &[]);
                let causa = match lido {
                    None => format!("a leitura do {FEITO} não voltou (call_func sem volta: o Call pode ter rodado)"),
                    Some((vez, retorno)) => format!(
                        "o Call não rodou ({FEITO} lido: vez {vez}, retorno {retorno}; DelaySystem parado: jogo em pausa, ou motor dormindo com a \
                         janela oculta)"
                    ),
                };
                crate::veredito_dev!(
                    "[telasmod] VEREDITO-{rotulo} interrompido: o passo {} (vez {}) não voltou em {} s depois de agendado: {causa}; {PARAR} {parar:?}; \
                     tela-ok=false",
                    ag.passo,
                    ag.vez,
                    ag.em.elapsed().as_secs()
                );
                r.agendado = None;
                // Sem a leitura, o passo pode ter rodado e pedido o menu: a série aborta mesmo no passo 1.
                if lido.is_none() {
                    return abortar_serie(r, "a leitura do passo agendado não voltou");
                }
                return sem_o_passo(r, "o passo agendado não rodou");
            }
            Espera::Rodou(retorno) => retorno,
        };
        r.agendado = None;
        if let Some(fim) = aplicar_retorno(r, retorno, ag.passo) {
            return fim;
        }
    }
    let tela = match r.tela {
        Some(t) => t,
        None => loop {
            let Some(t) = r.fila.pop_front() else { return true };
            let a = alvo(t);
            if let Err(motivo) = pode_abrir(a) {
                crate::veredito_dev!("[telasmod] VEREDITO-{} {}: não abriu — {motivo}; tela-ok=false", a.rotulo, a.nome);
                continue;
            }
            esvaziar_trocas();
            r.tela = Some(t);
            r.passos = Passos::novo();
            r.estrutura = None;
            r.controller = None;
            r.linhas_antes = linhas_do_passo();
            break t;
        },
    };
    let a = alvo(tela);
    let passo = r.passos.passo;
    let args = [rtti::Arg::I32(tela as u8 as u32), rtti::Arg::I32(passo as u32), rtti::Arg::I32(CADENCIA_MS)];
    match chamar(player, AGENDAR, &args) {
        Some(vez) if vez > 0 => {
            r.agendado = Some(Agendado { vez, passo, em: Instant::now() });
            false
        }
        outro => {
            let causa = match outro {
                None => format!("PlayerPuppet.{AGENDAR} não resolveu ou não voltou (bwms-telas-de-mod-teste.reds fora do bundle compilado?)"),
                Some(-1) => format!("{AGENDAR} devolveu -1: GetDelaySystem nulo"),
                Some(v) => format!("{AGENDAR} devolveu {v}, fora do protocolo (0 = o script não escreveu o retorno)"),
            };
            crate::veredito_dev!("[telasmod] VEREDITO-{} interrompido: {causa}, no passo {passo}; tela-ok=false", a.rotulo);
            sem_o_passo(r, "o passo não foi agendado com a tela já pedida")
        }
    }
}

/// Aplica o retorno do passo `passo` da tela em curso. `None`: a série segue (próximo passo, ou próxima tela); `Some(b)`: devolver `b` do
/// tick (`true` = a série acabou).
unsafe fn aplicar_retorno(r: &mut Rodada, retorno: i32, passo: i32) -> Option<bool> {
    let tela = r.tela?;
    let a = alvo(tela);
    match r.passos.aplicar(retorno) {
        Acao::Seguir(_) => None,
        Acao::Conferir { estrutura_ok } => {
            if !estrutura_ok && retorno != ESTRUTURA_ERRADA {
                crate::veredito_dev!("[telasmod] {} estourou {TETO_DE_PASSOS} passos abrindo (último passo {passo}): segue para o fechamento", a.rotulo);
            }
            r.estrutura = Some(estrutura_ok);
            r.controller = Some(conferir_controller(a));
            None
        }
        Acao::Encerrar(fechamento) => {
            if fechamento == Fechamento::NaoFechou && retorno != NAO_FECHOU {
                crate::veredito_dev!("[telasmod] {} estourou {TETO_DE_PASSOS} passos fechando (último passo {passo})", a.rotulo);
            }
            let solto = controller_solto(a.item);
            let [fora, principal] = linhas_do_passo();
            let [fora_antes, principal_antes] = r.linhas_antes;
            let (fora, principal) = (fora - fora_antes, principal - principal_antes);
            crate::veredito_dev!(
                "[telasmod] VEREDITO-{} {}: estrutura-ok={} controller-ok={} fechou-ok={}; controller do mod solto depois de fechar: {}; linhas do \
                 passo na thread principal: {principal} de {}; tela-ok={}",
                a.rotulo,
                a.nome,
                r.estrutura.map_or("não conferida".to_string(), |b| b.to_string()),
                r.controller.map_or("não conferido".to_string(), |b| b.to_string()),
                fechamento.texto(),
                solto.map_or("sem troca".to_string(), |b| b.to_string()),
                fora + principal,
                tela_ok(r.estrutura, r.controller, fechamento)
            );
            r.tela = None;
            if !serie_segue(fechamento) {
                return Some(abortar_serie(r, "fechamento da tela não confirmado"));
            }
            None
        }
        Acao::Interromper => {
            let causa = if retorno == 0 { " (0 = o script não escreveu o retorno: o passo parou no meio)" } else { "" };
            crate::veredito_dev!(
                "[telasmod] VEREDITO-{} interrompido: retorno {retorno} fora do protocolo no passo {passo}{causa}; tela-ok=false",
                a.rotulo
            );
            Some(abortar_serie(r, "passo interrompido"))
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    static EM_SERIE: Mutex<()> = Mutex::new(());

    #[test]
    fn pedido_vira_bits_e_fila_na_ordem_da_serie() {
        assert_eq!(telas_pedidas(&[]), Ok(0b1111));
        assert_eq!(telas_pedidas(&["tudo"]), Ok(0b1111));
        assert_eq!(telas_pedidas(&["eex"]), Ok(0b0001));
        assert_eq!(telas_pedidas(&["eexgr"]), Ok(0b1000));
        assert_eq!(telas_pedidas(&["valoja", "VaLista"]), Ok(0b0110), "ordem e caixa do argumento não contam");
        assert_eq!(telas_pedidas(&["eex", "eex"]), Ok(0b0001));
        assert!(telas_pedidas(&["eex", "bobagem"]).unwrap_err().contains("\"bobagem\" não é tela"));
        assert_eq!(
            fila_do_pedido(0b1111),
            VecDeque::from([Tela::GuardaRoupaEex, Tela::GuardaRoupaDoJogoEex, Tela::LojaVa, Tela::ListaVa])
        );
        assert_eq!(fila_do_pedido(0b0110), VecDeque::from([Tela::LojaVa, Tela::ListaVa]), "a ordem é a da série, não a do pedido");
        assert_eq!(fila_do_pedido(0b1001), VecDeque::from([Tela::GuardaRoupaEex, Tela::GuardaRoupaDoJogoEex]));
        assert!(fila_do_pedido(0).is_empty());
        assert_eq!(TODAS.iter().fold(0u8, |b, t| b | 1 << (*t as u8 - 1)), TODAS_AS_TELAS, "toda tela está na série");
    }

    #[test]
    fn alvo_da_tela_e_o_da_posicao() {
        for t in TODAS {
            let a = alvo(t);
            let (item, controller) = a.item.split_once(':').expect("nome do spawn com Item:Controller");
            assert!(!item.is_empty());
            assert_eq!(controller, a.classe, "{}: o controller pedido ao spawn é a classe conferida", a.rotulo);
        }
        assert_eq!(alvo(Tela::GuardaRoupaEex).rotulo, "EEX");
        assert_eq!(alvo(Tela::ListaVa).rotulo, "VA-LISTA");
        assert_eq!(alvo(Tela::LojaVa).rotulo, "VA-LOJA");
        assert_eq!(alvo(Tela::GuardaRoupaDoJogoEex).rotulo, "EEX-GR");
        // Os dois caminhos do guarda-roupa spawnam o mesmo item: a conferência do controller é a mesma.
        let (pelo_inventario, pelo_menu) = (alvo(Tela::GuardaRoupaEex), alvo(Tela::GuardaRoupaDoJogoEex));
        assert_eq!(pelo_inventario.item, pelo_menu.item);
        assert_eq!(pelo_inventario.campos, pelo_menu.campos);
        assert_eq!(pelo_inventario.vinculo, pelo_menu.vinculo);
    }

    #[test]
    fn protocolo_do_passo() {
        assert_eq!(decidir(2, 1, 1), Acao::Seguir(2));
        assert_eq!(decidir(199, 150, 5), Acao::Seguir(199));
        assert_eq!(decidir(ESTRUTURA_CERTA, 20, 20), Acao::Conferir { estrutura_ok: true });
        assert_eq!(decidir(ESTRUTURA_ERRADA, 15, 15), Acao::Conferir { estrutura_ok: false });
        assert_eq!(decidir(FECHOU, 104, 30), Acao::Encerrar(Fechamento::Fechou));
        assert_eq!(decidir(FECHOU_SEM_SAIDA_DO_MOD, 104, 30), Acao::Encerrar(Fechamento::MenuFechouSemSaidaDoMod));
        assert_eq!(decidir(NAO_FECHOU, 110, 30), Acao::Encerrar(Fechamento::NaoFechou));
        assert_eq!(decidir(NADA_ABERTO, 1, 1), Acao::Encerrar(Fechamento::NadaAberto));
        // A abertura que falhou antes do menu: o fechamento não acha o menu e devolve "nada aberto".
        assert_eq!(decidir(NADA_ABERTO, 104, 5), Acao::Encerrar(Fechamento::NadaAberto));
        // Teto: abrindo, vai para a conferência (e o fechamento); fechando, encerra sem fechar. Retorno de fim não passa pelo teto.
        assert_eq!(decidir(5, 4, TETO_DE_PASSOS), Acao::Conferir { estrutura_ok: false });
        assert_eq!(decidir(105, 104, TETO_DE_PASSOS), Acao::Encerrar(Fechamento::NaoFechou));
        assert_eq!(decidir(FECHOU, 104, TETO_DE_PASSOS + 5), Acao::Encerrar(Fechamento::Fechou));
        // Fora do protocolo; 0 é o buffer de retorno que o script não escreveu, nunca "fechou".
        assert_ne!(FECHOU, 0);
        for r in [0, -99, -3, 202, 299, 302, 1000, i32::MIN] {
            assert_eq!(decidir(r, 1, 1), Acao::Interromper, "retorno {r}");
            assert_eq!(decidir(r, 104, 3), Acao::Interromper, "retorno {r} fechando");
        }
    }

    // A abertura que estoura o teto vai para o fechamento com a contagem zerada: o fechamento roda até o teto dele, e o script chega ao passo
    // que pede o fechamento do menu (101–103).
    #[test]
    fn teto_estourado_abrindo_deixa_o_fechamento_inteiro() {
        let mut p = Passos::novo();
        assert!(!p.pode_ter_aberto(), "antes do passo 1 nada foi pedido");
        assert_eq!(p.aplicar(2), Acao::Seguir(2));
        assert!(p.pode_ter_aberto());
        for _ in 1..TETO_DE_PASSOS - 1 {
            assert_eq!(p.aplicar(2), Acao::Seguir(2), "passo de espera abrindo");
        }
        assert_eq!(p.aplicar(2), Acao::Conferir { estrutura_ok: false }, "a {}ª chamada abrindo estoura", TETO_DE_PASSOS);
        assert_eq!(p, Passos { passo: PASSO_DE_FECHAR, chamadas: 0 });
        assert_eq!(p.aplicar(101), Acao::Seguir(101));
        assert_eq!(p.aplicar(102), Acao::Seguir(102));
        assert_eq!(p.aplicar(103), Acao::Seguir(103));
        for _ in 4..TETO_DE_PASSOS {
            assert_eq!(p.aplicar(104), Acao::Seguir(104), "espera do fechamento");
        }
        assert_eq!(p.aplicar(104), Acao::Encerrar(Fechamento::NaoFechou), "o fechamento estoura na {}ª chamada dele", TETO_DE_PASSOS);

        // Caminho normal: conferência no passo 20, fechamento em 5 chamadas.
        let mut p = Passos::novo();
        for passo in 2..=20 {
            assert_eq!(p.aplicar(passo), Acao::Seguir(passo));
        }
        assert_eq!(p.aplicar(ESTRUTURA_CERTA), Acao::Conferir { estrutura_ok: true });
        assert_eq!(p.passo, PASSO_DE_FECHAR);
        assert_eq!(p.aplicar(101), Acao::Seguir(101));
        assert_eq!(p.aplicar(FECHOU), Acao::Encerrar(Fechamento::Fechou));
    }

    #[test]
    fn serie_so_segue_com_o_menu_fechado_ou_nada_aberto() {
        assert!(serie_segue(Fechamento::Fechou));
        assert!(serie_segue(Fechamento::NadaAberto), "a tela parou antes do menu: a próxima pode rodar");
        assert!(serie_segue(Fechamento::MenuFechouSemSaidaDoMod), "o menu fechou: a falha da saída do mod reprova só a tela");
        assert!(!serie_segue(Fechamento::NaoFechou), "menu possivelmente preso: a próxima abriria em cima");
    }

    #[test]
    fn tela_ok_pede_as_tres() {
        assert!(tela_ok(Some(true), Some(true), Fechamento::Fechou));
        assert!(!tela_ok(Some(true), Some(true), Fechamento::NaoFechou));
        assert!(!tela_ok(Some(true), Some(true), Fechamento::MenuFechouSemSaidaDoMod), "saída do mod falha não passa");
        assert!(!tela_ok(Some(false), Some(true), Fechamento::Fechou));
        assert!(!tela_ok(Some(true), None, Fechamento::Fechou));
        assert!(!tela_ok(None, None, Fechamento::NadaAberto));
    }

    // O retorno só vale com a `vez` do pedido em `BwmsTelaModFeito`: a de um passo anterior, ou a leitura falha, ainda é espera até o teto.
    #[test]
    fn espera_do_call_do_passo() {
        let teto = Duration::from_millis(u64::from(CADENCIA_MS)) + ESPERA_DO_CALL;
        assert_eq!(esperar(Some((7, 3)), 7, Duration::from_millis(1100)), Espera::Rodou(3));
        assert_eq!(esperar(Some((7, FECHOU)), 7, teto * 3), Espera::Rodou(FECHOU), "rodou é rodou, mesmo lido tarde");
        assert_eq!(esperar(Some((6, 3)), 7, Duration::from_millis(1100)), Espera::Aguardando, "a vez anterior não é este passo");
        assert_eq!(esperar(Some(separar_feito(0)), 1, Duration::from_millis(1100)), Espera::Aguardando, "nenhum passo rodou ainda");
        assert_eq!(esperar(None, 7, Duration::from_millis(1100)), Espera::Aguardando);
        assert_eq!(esperar(Some((6, 3)), 7, teto - Duration::from_millis(1)), Espera::Aguardando);
        assert_eq!(esperar(Some((6, 3)), 7, teto), Espera::Parado);
        assert_eq!(esperar(None, 7, teto), Espera::Parado);
    }

    // A `vez` e o retorno voltam do mesmo Int32 que o `.reds` grava (`vez * FEITO_VEZ + retorno + FEITO_DESLOCAMENTO`), para todo código do
    // protocolo, para o 0 do passo que parou no meio e para o valor que o script grava com o retorno fora da faixa, que segue fora do protocolo.
    #[test]
    fn feito_leva_vez_e_retorno_numa_gravacao() {
        let gravar = |vez: i32, retorno: i32| vez * FEITO_VEZ + retorno + FEITO_DESLOCAMENTO;
        for vez in [1, 2, 141, 5000] {
            for retorno in [1, 2, 104, 199, ESTRUTURA_CERTA, ESTRUTURA_ERRADA, FECHOU, FECHOU_SEM_SAIDA_DO_MOD, NAO_FECHOU, NADA_ABERTO, 0] {
                assert_eq!(separar_feito(gravar(vez, retorno)), (vez, retorno), "vez {vez}, retorno {retorno}");
            }
        }
        assert_eq!(separar_feito(0), (0, -FEITO_DESLOCAMENTO), "campo zerado: vez 0, que nenhum pedido usa (a 1ª é 1)");
        assert_eq!(decidir(-FEITO_DESLOCAMENTO, 1, 1), Acao::Interromper);
        assert_eq!(decidir(FEITO_DESLOCAMENTO - 1, 1, 1), Acao::Interromper, "o retorno fora da faixa continua fora do protocolo");
        assert!(FECHOU_SEM_SAIDA_DO_MOD < FEITO_DESLOCAMENTO && NAO_FECHOU >= -FEITO_DESLOCAMENTO, "o protocolo cabe na faixa");
    }

    #[test]
    fn resumo_conta_por_nome_na_ordem_da_primeira_vez() {
        let trocas = [
            ("Root:A", true, false),
            ("Slot:B", true, false),
            ("Slot:B", false, true),
            ("Root:A", true, false),
            ("Slot:B", true, true),
            ("Slot:B", true, true),
        ];
        let c = |sincronas, assincronas, falhas| Contagem { sincronas, assincronas, falhas };
        assert_eq!(resumo_por_nome(trocas.into_iter()), vec![("Root:A", c(2, 0, 0)), ("Slot:B", c(1, 2, 1))]);
        assert!(resumo_por_nome(std::iter::empty()).is_empty());
    }

    // Cada grupo precisa de 1 troca assíncrona certa: a medida do modelo (síncrona) e as falhas não contam; um nome do grupo basta.
    #[test]
    fn itens_ok_pede_troca_assincrona_em_cada_grupo() {
        let c = |sincronas, assincronas, falhas| Contagem { sincronas, assincronas, falhas };
        let so_medida = [("Small:I", c(1, 0, 0)), ("Big:I", c(1, 0, 0)), ("Header:H", c(1, 0, 0))];
        let grupo: &[&[&str]] = &[&["Small:I", "Big:I"]];
        assert_eq!(itens_ok(&so_medida, grupo), (false, "Small:I 0 + Big:I 0".to_string()), "o boot de 2026-10-18: 1/0 por modelo, nenhum item");
        let com_itens = [("Small:I", c(1, 4, 0)), ("Big:I", c(1, 0, 2))];
        assert_eq!(itens_ok(&com_itens, grupo), (true, "Small:I 4 + Big:I 0".to_string()));
        let dois: &[&[&str]] = &[&["Small:I", "Big:I"], &["Lista:L"]];
        assert!(!itens_ok(&com_itens, dois).0, "a lista sem item reprova");
        assert!(itens_ok(&[("Small:I", c(0, 1, 0)), ("Lista:L", c(0, 2, 0))], dois).0);
        assert_eq!(itens_ok(&so_medida, &[]), (true, "—".to_string()), "tela sem grade nem lista");
        assert!(!itens_ok(&[], grupo).0);
    }

    // Os casos de falha da leitura saem separados no veredito.
    #[test]
    fn leitura_do_campo_separa_os_casos() {
        assert_eq!(Leitura::Vivo(0x10).viva(), Some(0x10));
        assert_eq!(Leitura::SemPropriedade.viva(), None);
        assert_eq!(Leitura::HolderNulo { off: 0x40 }.texto(), "holder nulo (+0x40)");
        assert_eq!(Leitura::Nulo { off: 0x48, no_holder: true, base: 0x7000 }.texto(), "nulo (+0x48 no holder 0x7000)");
        assert_eq!(Leitura::Morto { off: 0x50, no_holder: false, base: 0x8000 }.texto(), "morto (+0x50 no objeto 0x8000)");
        assert_eq!(Leitura::Ilegivel { off: 0x58, no_holder: true, base: 0x9000 }.viva(), None);
    }

    // Sem a série rodando, o gancho não anota; rodando, anota até o teto e conta o resto, com a thread do spawn. Controller nulo e raiz zero não
    // chamam o motor (fraco nenhum a tomar nem a soltar), então a lista roda fora do jogo. O spawn repassado sem troca conta por nome, só com a
    // série e só na forma `Item:Controller` (nomes do espelho, `intern`: o pool do motor não existe no teste).
    #[test]
    fn anotacao_so_com_a_serie_e_ate_o_teto() {
        let _serie = EM_SERIE.lock().unwrap_or_else(|e| e.into_inner());
        let antes = RODANDO.swap(false, Ordering::Relaxed);
        let (com_forma, sem_forma) = (crate::cname::intern("TesteSemTroca:TesteController"), crate::cname::intern("TesteSemTrocaSoItem"));
        let principal = unsafe { pthread_main_np() } != 0;
        unsafe {
            esvaziar_trocas();
            anotar_troca("Root:Fora", std::ptr::null_mut(), [0, 0], false);
            anotar_sem_troca(com_forma);
            assert_eq!(TROCAS.lock().unwrap().len(), 0, "sem série, nada anotado");
            assert!(SEM_TROCA.lock().unwrap().is_empty(), "sem série, nada contado");
            RODANDO.store(true, Ordering::Relaxed);
            for i in 0..TETO_DE_TROCAS + 3 {
                anotar_troca("Slot:Teste", std::ptr::null_mut(), [0, 0], i % 2 == 1);
            }
            let v = TROCAS.lock().unwrap();
            assert_eq!(v.len(), TETO_DE_TROCAS);
            assert!(v.iter().all(|t| t.controller == [0, 0] && t.raiz == [0, 0]), "falha anotada sem fraco");
            assert!(v.iter().all(|t| t.principal == principal), "a thread do spawn é a de quem anota");
            assert_eq!(v.iter().filter(|t| t.assincrona).count(), TETO_DE_TROCAS / 2, "síncrona e assíncrona anotadas como vieram");
            drop(v);
            assert_eq!(TROCAS_FORA_DO_TETO.load(Ordering::Relaxed), 3);
            assert_eq!(controller_solto("Slot:Teste"), None, "sem troca certa não há controller a conferir");
            for _ in 0..2 {
                anotar_sem_troca(com_forma);
            }
            anotar_sem_troca(sem_forma);
            assert_eq!(*SEM_TROCA.lock().unwrap(), vec![("TesteSemTroca:TesteController".to_string(), 2)], "só Item:Controller, por nome");
            esvaziar_trocas();
            assert_eq!(TROCAS.lock().unwrap().len(), 0);
            assert_eq!(TROCAS_FORA_DO_TETO.load(Ordering::Relaxed), 0);
            assert!(SEM_TROCA.lock().unwrap().is_empty());
        }
        RODANDO.store(antes, Ordering::Relaxed);
    }

    // A thread do passo sai das linhas `[telasmod]` do script, só com a série rodando; linha de outro teste não conta. O índice
    // esperado vem do `pthread_main_np` da thread deste teste (o libtest põe cada teste numa thread própria).
    #[test]
    fn linha_do_passo_conta_so_telasmod_com_a_serie() {
        let _serie = EM_SERIE.lock().unwrap_or_else(|e| e.into_inner());
        let antes = RODANDO.swap(false, Ordering::Relaxed);
        let i = usize::from(unsafe { pthread_main_np() } != 0);
        let inicio = linhas_do_passo();
        linha_do_passo("[telasmod] EEX inventário aberto no passo 3");
        assert_eq!(linhas_do_passo(), inicio, "sem série, nada contado");
        RODANDO.store(true, Ordering::Relaxed);
        linha_do_passo("[telasmod] VEREDITO-EEX-ESTRUTURA ...");
        linha_do_passo("[aba] passo 1 -> 2");
        linha_do_passo("aba.inicio [telasmod]");
        let depois = linhas_do_passo();
        assert_eq!(depois[i], inicio[i] + 1);
        assert_eq!(depois[1 - i], inicio[1 - i]);
        RODANDO.store(antes, Ordering::Relaxed);
    }

    // O `.reds` do teste usa os mesmos números do protocolo, as mesmas telas e os métodos do condutor que o tick chama. Fora da árvore do
    // projeto (export público) o arquivo não existe e o teste não tem o que comparar.
    #[test]
    fn script_de_teste_usa_o_mesmo_protocolo() {
        let caminho = concat!(env!("CARGO_MANIFEST_DIR"), "/../blackwall-mods-dev/bwms-telas-de-mod-teste.reds");
        let Ok(fonte) = std::fs::read_to_string(caminho) else {
            eprintln!("teste pulado: {caminho} ausente");
            return;
        };
        for n in [ESTRUTURA_CERTA, ESTRUTURA_ERRADA, FECHOU, FECHOU_SEM_SAIDA_DO_MOD, NAO_FECHOU, NADA_ABERTO] {
            assert!(fonte.contains(&format!("return {n};")), "o .reds não devolve {n}");
        }
        assert!(!fonte.contains("return 0;"), "0 é o Int32 padrão de um passo que parou no meio; nenhum passo devolve 0");
        assert!(fonte.contains(&format!("passo == {PASSO_DE_FECHAR}")), "o .reds não fecha no passo {PASSO_DE_FECHAR}");
        // As três esperas de menu (inventário, menu de guarda-roupa, vendedor) refazem o pedido no passo 8.
        assert_eq!(fonte.matches("passo == 8").count(), 3, "2º pedido do menu no passo 8 nas três esperas");
        // Condutor: o tick agenda pelo `BwmsTelaModAgendar` e lê `BwmsTelaModFeito`; o passo roda só no `Call` do `DelayCallback` (pelo
        // `BwmsTelaModRodar`), agendado uma vez, fora da dilatação de tempo, e grava a `vez` e o retorno numa gravação só, na conta do Rust.
        assert!(fonte.contains(&format!("public func {AGENDAR}(tela: Int32, passo: Int32, atrasoMs: Int32) -> Int32")));
        for m in [FEITO, PARAR] {
            assert!(fonte.contains(&format!("public func {m}() -> Int32")), "o .reds não tem {m}");
        }
        let gravacao = format!("this.m_bwmsTelaModFeito = r.vez * {FEITO_VEZ} + retorno + {FEITO_DESLOCAMENTO};");
        let faixa = format!("if retorno < -{FEITO_DESLOCAMENTO} || retorno > {} {{", FEITO_DESLOCAMENTO - 1);
        let gravacoes: Vec<&str> = fonte.lines().map(str::trim).filter(|l| l.starts_with("this.m_bwmsTelaModFeito =")).collect();
        assert_eq!(gravacoes, [gravacao.as_str()], "uma gravação só do feito, com a conta do separar_feito");
        assert!(fonte.contains(&faixa) && fonte.contains(&format!("retorno = {};", FEITO_DESLOCAMENTO - 1)), "o .reds não limita o retorno à faixa");
        assert!(fonte.contains("extends DelayCallback") && fonte.contains("public func Call() -> Void"));
        let agendamentos: Vec<&str> = fonte.lines().map(str::trim).filter(|l| !l.starts_with("//") && l.contains(".DelayCallback(")).collect();
        assert_eq!(agendamentos.len(), 1, "um agendamento só: {agendamentos:?}");
        assert!(agendamentos[0].ends_with(", false);"), "fora da dilatação de tempo: {}", agendamentos[0]);
        // O spawn, a saída da tela do mod e o fechamento do menu rodam no despacho de evento do motor, o caminho do mod: as chamadas ficam em
        // `cb func`, nunca nos métodos do passo. Do passo saem só pedidos enfileirados (`QueueEvent`, `RequestVendorMenu`).
        let mut dono = "";
        let mut passos_chamados = 0;
        for (i, linha) in fonte.lines().enumerate() {
            let l = linha.trim_start();
            if l.starts_with("//") {
                continue;
            }
            if l.contains(" func ") {
                dono = l;
            }
            for chamada in [
                ".SpawnFromExternal(",
                ".SpawnFromLocal(",
                ".ShowWardrobeScreen(",
                ".HideWardrobeScreen(",
                ".CloseWardrobe(",
                ".RemoveChild(",
                ".SpawnEvent(",
            ] {
                if l.contains(chamada) {
                    assert!(dono.contains("cb func"), "linha {}: {chamada} fora de cb func (em \"{dono}\")", i + 1);
                }
            }
            if l.contains(".BwmsTelaModPasso(") {
                passos_chamados += 1;
                assert!(dono.contains("func BwmsTelaModRodar("), "linha {}: o passo chamado fora do Call (em \"{dono}\")", i + 1);
            }
        }
        assert_eq!(passos_chamados, 1, "o passo roda por um caminho só");
        assert!(fonte.contains("public func BwmsTelaModPasso(tela: Int32, passo: Int32) -> Int32"));
        for t in TODAS {
            assert!(fonte.contains(&format!("tela == {}", t as u8)), "o .reds não despacha a tela {t:?}");
            // O script dá as linhas de estrutura e de fechamento da tela; o Rust, as de controller e a final, com o mesmo rótulo. Todo fechamento
            // trata a abertura que parou antes do menu.
            let a = alvo(t);
            for parte in ["ESTRUTURA", "FECHAR", "FECHAR nada aberto:"] {
                assert!(fonte.contains(&format!("VEREDITO-{}-{parte} ", a.rotulo)), "o .reds não dá o VEREDITO-{}-{parte}", a.rotulo);
            }
        }
    }
}
