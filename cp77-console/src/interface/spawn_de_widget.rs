//! Spawn de widget de biblioteca (`InkWidgetLibrary::SpawnFromLocal`/`SpawnFromExternal`): o controller pedido no nome do item (`Item:Controller`, do
//! `InkSpawner` do ArchiveXL e do `WidgetSpawningService` do Codeware, que o Virtual Atelier e o Equipment-EX chamam) e o evento `InkWidget/Spawn` do
//! Codeware (`#123`, com o filtro do `inkWidgetTarget` do `#135`). Os hooks ficam no ar desde o `on_load`: quem spawna com `Item:Controller` não
//! registra nada antes. O `SpawnFromExternal` põe antes a biblioteca externa na lista de dependências de quem chama (`InjectDependency`).
//!
//! O spawn assíncrono (`AsyncSpawnFromLocal`/`AsyncSpawnFromExternal`) é o dos itens das grades e listas virtuais (`inkVirtualListController`,
//! `inkVirtualGridController`): o nome `Item:Controller` vai inteiro no pedido e o motor não acha item com ele. O hook do `AsyncSpawnFrom*` refaz o
//! pedido pelo `Item` e guarda o nome inteiro; o hook da entrega (o `FinishAsyncSpawn` da referência) troca o controller da instância antes de o motor
//! anexá-la e inicializá-la.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use crate::cname::cname;
use crate::ganchos::{instalar_conferindo_prologo, original_publicado};
use crate::jogo::chamada::sret_chamar;
use crate::jogo::interface::{
    ASYNC_SPAWN_FROM_EXTERNAL_PROLOGO, ASYNC_SPAWN_FROM_EXTERNAL_VM, ASYNC_SPAWN_FROM_LOCAL_PROLOGO, ASYNC_SPAWN_FROM_LOCAL_VM,
    ENTREGA_DO_SPAWN_ASSINCRONO_PROLOGO, ENTREGA_DO_SPAWN_ASSINCRONO_VM, SPAWN_FROM_EXTERNAL_PROLOGO, SPAWN_FROM_EXTERNAL_VM, SPAWN_FROM_LOCAL_PROLOGO,
    SPAWN_FROM_LOCAL_VM,
};
use crate::jogo::listas::dynarray_no_campo;
use crate::jogo::objetos::soltar_handle;
use crate::register::class_is_kind_of;
use crate::rtti;

pub(crate) static ORIG_SPAWN_FROM_LOCAL: AtomicU64 = AtomicU64::new(0);
pub(crate) static ORIG_SPAWN_FROM_EXTERNAL: AtomicU64 = AtomicU64::new(0);
pub(crate) static ORIG_ASYNC_SPAWN_FROM_LOCAL: AtomicU64 = AtomicU64::new(0);
pub(crate) static ORIG_ASYNC_SPAWN_FROM_EXTERNAL: AtomicU64 = AtomicU64::new(0);
pub(crate) static ORIG_ENTREGA_DO_SPAWN_ASSINCRONO: AtomicU64 = AtomicU64::new(0);

/// `inkWidgetLibraryItemInstance`: `rootWidget` e `gameController`, Handles de 16 B. O `+0x48` é o que o filtro de controller do `inkWidgetTarget` lê.
const ITEM_RAIZ: usize = 0x38;
const ITEM_CONTROLLER: usize = 0x48;

/// `CResource::path` da biblioteca.
const BIBLIOTECA_CAMINHO: usize = 0x30;

/// Elemento do `externalLibraries` (`Ref<inkWidgetLibraryResource>`): caminho em `+0x00` e `SharedPtr` do token em `+0x08`/`+0x10`, o mesmo layout do
/// `blockRefs` que `arquivos_do_jogo::mesclar_blocos_no_mundo` anexa no Mac.
const EXTERNA_PASSO: usize = 0x18;

/// Offset do `externalLibraries` no `inkWidgetLibraryResource`, pelo RTTI do motor (o caminho do `blockRefs` em
/// `arquivos_do_jogo::guardar_offsets_do_mundo`, que deu o número do desmonte no Mac). 0 = ainda não resolvido; [`EXTERNAS_RECUSADO`] = layout fora do
/// esperado, injeção desligada.
pub(crate) static EXTERNAS_OFF: AtomicU64 = AtomicU64::new(0);
pub(crate) const EXTERNAS_RECUSADO: u64 = u64::MAX;

/// O `s_mutex` da referência: leitura pra conferir a lista, escrita pra crescer, anexar e esperar a carga. Dois spawns na mesma biblioteca não
/// realocam o array um embaixo do outro.
static TRAVA_DAS_EXTERNAS: RwLock<()> = RwLock::new(());

/// Teto da espera pela carga da biblioteca injetada: o `WaitForResource(token, 1000 ms)` da referência.
const ESPERA_DA_DEPENDENCIA: Duration = Duration::from_millis(1000);

/// Chamadas do `SpawnFromLocal`, do `SpawnFromExternal` e controllers trocados, pro log do dev.
#[cfg(feature = "devtools")]
pub(crate) static CONTAGEM: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

extern "C" {
    fn pthread_main_np() -> i32;
}

/// O hook troca o controller (`create_instance`), dispara o evento (`new_object`, `fire_event_args`) e injeta a dependência na thread principal e
/// em thread com script em execução (o executor do BWMS aberto nela): é onde o script do mod faz o próprio `new`, e na gameplay o script roda nos
/// workers com a GameThread esperando (230 de 230 amostras do `cp77_tick` fora da principal, e nenhum `SpawnFrom*` da gameplay na principal:
/// `cp77-symbols/notes/proofs/20261018-provas-015-anexos/U.txt` e `console-do-boot.log`). Fora dos dois (a carga assíncrona do próprio jogo, vista nos boots) o hook só chama a original.
/// Vale só pros dois síncronos; o `AsyncSpawnFrom*` e a entrega não passam por ele. O `AsyncSpawnFrom*` roda no job do processador de pedidos
/// (`0x104368b58`, despachado pelo `0x104ddf514`); a entrega roda no job que acabou de instanciar o item: o do processador com o pacote
/// carregado, ou a closure `0x10431ea84` sem ele.
#[cfg(not(test))]
fn na_game_thread() -> bool {
    let principal = unsafe { pthread_main_np() != 0 };
    principal || crate::inicio::boot::exec_depth() > 0
}

/// Nos testes o hook roda em threads do cargo: vale a marca da thread, ligada por padrão (o teste do desvio a desliga), ou o executor aberto.
#[cfg(test)]
fn na_game_thread() -> bool {
    testes::GAME_THREAD_NO_TESTE.with(|g| g.get()) || crate::inicio::boot::exec_depth() > 0
}

/// Chamadas de cada hook que só repassaram à original, pro veredito da 1ª do processo.
#[cfg(feature = "devtools")]
static FORA_DA_GAME_THREAD: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

/// Veredito da 1ª chamada repassada de cada hook e, com o `telasmod` rodando, a conta do spawn `Item:Controller` vazio que ficou sem troca.
#[cfg(feature = "devtools")]
unsafe fn anotar_repasse(hook: usize, item: u64, saida: *mut [u64; 2]) {
    if crate::primeiras(&FORA_DA_GAME_THREAD[hook], 1) {
        let t = std::thread::current();
        crate::veredito_dev!(
            "[spawn] {} fora da thread principal e sem script em execução nela: o hook só chama a original; thread {:?} {:?}",
            ["SpawnFromLocal", "SpawnFromExternal"][hook],
            t.id(),
            t.name()
        );
    }
    if (*saida)[0] == 0 {
        super::teste_telas_de_mod::anotar_sem_troca(item);
    }
}

thread_local! {
    /// Dentro de um `SpawnFromExternal` hookado: o evento sai dele, com a biblioteca externa, depois da troca de controller; um `SpawnFromLocal` aninhado
    /// não despacha.
    static DENTRO_DO_EXTERNO: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Hooks do `SpawnFromLocal`, do `SpawnFromExternal`, dos dois `AsyncSpawnFrom*` e da entrega do spawn assíncrono, com o prólogo conferido: no `on_load`
/// e de novo no `RegisterCallback` de `InkWidget/Spawn` (linha da tabela de grupos de evento). Sem remoção: o `Item:Controller` não tem ouvinte que avise
/// antes do spawn.
pub(crate) unsafe fn instalar() {
    let local = instalar_conferindo_prologo(SPAWN_FROM_LOCAL_VM, SPAWN_FROM_LOCAL_PROLOGO, spawn_local_tramp as *mut c_void, &ORIG_SPAWN_FROM_LOCAL);
    let externo = instalar_conferindo_prologo(SPAWN_FROM_EXTERNAL_VM, SPAWN_FROM_EXTERNAL_PROLOGO, spawn_externo_tramp as *mut c_void, &ORIG_SPAWN_FROM_EXTERNAL);
    let async_local =
        instalar_conferindo_prologo(ASYNC_SPAWN_FROM_LOCAL_VM, ASYNC_SPAWN_FROM_LOCAL_PROLOGO, async_spawn_local as *mut c_void, &ORIG_ASYNC_SPAWN_FROM_LOCAL);
    let async_externo = instalar_conferindo_prologo(
        ASYNC_SPAWN_FROM_EXTERNAL_VM,
        ASYNC_SPAWN_FROM_EXTERNAL_PROLOGO,
        async_spawn_externo as *mut c_void,
        &ORIG_ASYNC_SPAWN_FROM_EXTERNAL,
    );
    let entrega = instalar_conferindo_prologo(
        ENTREGA_DO_SPAWN_ASSINCRONO_VM,
        ENTREGA_DO_SPAWN_ASSINCRONO_PROLOGO,
        antes_da_entrega as *mut c_void,
        &ORIG_ENTREGA_DO_SPAWN_ASSINCRONO,
    );
    if [local, externo, async_local, async_externo, entrega].iter().any(Option::is_some) {
        crate::veredito_dev!(
            "[spawn] hooks do SpawnFromLocal {local:?}, do SpawnFromExternal {externo:?}, do AsyncSpawnFromLocal {async_local:?}, do AsyncSpawnFromExternal \
             {async_externo:?} e da entrega do spawn assíncrono {entrega:?}"
        );
    }
}

/// Os hooks do assíncrono e da entrega no ar: sem eles o item `Item:Controller` de grade ou lista virtual não nasce.
pub(crate) fn assincrono_no_ar() -> bool {
    [&ORIG_ASYNC_SPAWN_FROM_LOCAL, &ORIG_ASYNC_SPAWN_FROM_EXTERNAL, &ORIG_ENTREGA_DO_SPAWN_ASSINCRONO].iter().all(|o| o.load(Ordering::Relaxed) != 0)
}

// ===== Spawn assíncrono =====

/// Nomes `Item:Controller` que um `AsyncSpawnFrom*` refez pelo `Item`: o hash do nome inteiro, que fica no pedido, e o do controller. A entrega lê o
/// nome do pedido e acha o controller aqui, sem voltar o nome pra texto na thread do job. Só cresce: um par por nome com controller que os mods spawnam.
pub(crate) struct NomesDaEntrega<const N: usize> {
    pares: [[AtomicU64; 2]; N],
    usados: AtomicUsize,
    escrita: Mutex<()>,
}

impl<const N: usize> NomesDaEntrega<N> {
    pub(crate) const fn nova() -> Self {
        NomesDaEntrega { pares: [const { [AtomicU64::new(0), AtomicU64::new(0)] }; N], usados: AtomicUsize::new(0), escrita: Mutex::new(()) }
    }

    /// Nenhum nome: a entrega de todo spawn do jogo custa esta leitura.
    fn vazia(&self) -> bool {
        self.usados.load(Ordering::Acquire) == 0
    }

    /// O controller do nome inteiro `nome`. Sem trava: o par é gravado antes de o `usados` (`Release`) publicá-lo.
    fn controller(&self, nome: u64) -> Option<u64> {
        let n = self.usados.load(Ordering::Acquire);
        self.pares[..n].iter().find(|p| p[0].load(Ordering::Relaxed) == nome).map(|p| p[1].load(Ordering::Relaxed))
    }

    /// `nome` → `controller`, uma vez por nome (o nome inteiro determina o controller). `false` com a tabela cheia.
    fn registrar(&self, nome: u64, controller: u64) -> bool {
        let _escrita = self.escrita.lock().unwrap_or_else(PoisonError::into_inner);
        if self.controller(nome).is_some() {
            return true;
        }
        let n = self.usados.load(Ordering::Relaxed);
        let Some(par) = self.pares.get(n) else { return false };
        par[1].store(controller, Ordering::Relaxed);
        par[0].store(nome, Ordering::Relaxed);
        self.usados.store(n + 1, Ordering::Release);
        true
    }
}

/// Teto da tabela, folgado sobre os nomes `Item:Controller` de modelo de grade e lista que o boot de 2026-10-18 mostrou no Virtual Atelier e no
/// Equipment-EX (7, `proofs/20261018-telas-de-mod-eex-va-vereditos.log`).
const TETO_DE_NOMES_DA_ENTREGA: usize = 256;
static NOMES_DA_ENTREGA: NomesDaEntrega<TETO_DE_NOMES_DA_ENTREGA> = NomesDaEntrega::nova();

type AsyncLocal = unsafe extern "C" fn(*mut c_void, *mut c_void, u64, u64) -> u32;
type AsyncExterno = unsafe extern "C" fn(*mut c_void, *mut c_void, u64, u64, u64) -> u32;
type Entrega = unsafe extern "C" fn(*mut c_void, *mut [u64; 2]);

/// `AsyncSpawnFromLocal(biblioteca, info, item, flag)` (`OnAsyncSpawnLocal` da referência): o pedido pelo nome inteiro primeiro, como o motor pediu;
/// sem item com esse nome e com a forma `Item:Controller`, o nome entra em [`NOMES_DA_ENTREGA`] e o pedido vai de novo pelo `Item`. O nome entra antes
/// da 2ª chamada porque, com o pacote do item já carregado, a entrega roda dentro dela. A original sem o item devolve 0 sem efeito.
unsafe extern "C" fn async_spawn_local(lib: *mut c_void, info: *mut c_void, item: u64, flag: u64) -> u32 {
    let original: AsyncLocal = std::mem::transmute(original_publicado(&ORIG_ASYNC_SPAWN_FROM_LOCAL));
    let achou = original(lib, info, item, flag);
    if achou & 1 != 0 {
        return achou;
    }
    match item_da_entrega(item, &NOMES_DA_ENTREGA, ORIG_ENTREGA_DO_SPAWN_ASSINCRONO.load(Ordering::Relaxed) != 0) {
        Some(base) => original(lib, info, base, flag),
        None => achou,
    }
}

/// `AsyncSpawnFromExternal(biblioteca, info, caminho da externa, item, flag)`: a biblioteca externa entra antes nas dependências de quem chama
/// ([`injetar_dependencia`], no job que a procura logo depois, na original) e o nome segue como no [`async_spawn_local`].
unsafe extern "C" fn async_spawn_externo(lib: *mut c_void, info: *mut c_void, caminho: u64, item: u64, flag: u64) -> u32 {
    let original: AsyncExterno = std::mem::transmute(original_publicado(&ORIG_ASYNC_SPAWN_FROM_EXTERNAL));
    injetar_dependencia(lib, caminho);
    let achou = original(lib, info, caminho, item, flag);
    if achou & 1 != 0 {
        return achou;
    }
    match item_da_entrega(item, &NOMES_DA_ENTREGA, ORIG_ENTREGA_DO_SPAWN_ASSINCRONO.load(Ordering::Relaxed) != 0) {
        Some(base) => original(lib, info, caminho, base, flag),
        None => achou,
    }
}

/// O `Item` de um nome `Item:Controller`, com o nome inteiro posto em `tabela` quando o controller não é vazio. `None` sem a forma, e também com
/// controller e sem a troca garantida (o hook da entrega fora do ar, ou a tabela cheia): o item refeito nasceria com o controller do pacote, que o
/// script do mod não espera; sem o refazer, o pedido fica vazio como no motor sem o BWMS.
fn item_da_entrega<const N: usize>(item: u64, tabela: &NomesDaEntrega<N>, entrega_no_ar: bool) -> Option<u64> {
    let nome = crate::cname::name_of(item)?;
    let (base, controller) = item_e_controller(&nome)?;
    if controller.is_empty() {
        return Some(base);
    }
    if !entrega_no_ar {
        #[cfg(feature = "devtools")]
        if crate::primeiras(&AVISOS_DO_ASSINCRONO[0], 1) {
            crate::veredito_dev!("[spawn] '{nome}': hook da entrega do spawn assíncrono fora do ar; o pedido fica vazio");
        }
        return None;
    }
    if !tabela.registrar(item, cname(controller)) {
        #[cfg(feature = "devtools")]
        if crate::primeiras(&AVISOS_DO_ASSINCRONO[1], 1) {
            crate::veredito_dev!("[spawn] '{nome}': tabela da entrega cheia ({N} nomes); o pedido fica vazio");
        }
        return None;
    }
    Some(base)
}

/// Pedidos `Item:Controller` do assíncrono deixados vazios, sem o hook da entrega e com a tabela cheia, pro veredito do 1º de cada.
#[cfg(feature = "devtools")]
static AVISOS_DO_ASSINCRONO: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

/// Callable da entrega: o `SharedPtr` do pedido em `+0x10`/`+0x18` (o destrutor `0x10436929c` solta os dois). Pedido: o nome inteiro do item em
/// `+0x48`, que o processador grava (`0x1043643fc`) e passa ao `AsyncSpawnFrom*`.
const ENTREGA_PEDIDO: usize = 0x10;
const PEDIDO_NOME: usize = 0x48;

/// Entrega do spawn assíncrono (o `HookBefore` do `FinishAsyncSpawn` da referência): o controller do `Item:Controller` entra na instância antes de a
/// original copiar a raiz e o `gameController` pro pedido; o motor anexa a instância e inicializa os controllers depois, pelo caminho de sempre. Roda no
/// job que acabou de instanciar o item, onde o `Instantiate` cria os objetos do pacote pelo mesmo `CreateInstance` que o [`trocar_controller`] chama.
/// Sem nome na tabela, uma leitura atômica.
unsafe extern "C" fn antes_da_entrega(entrega: *mut c_void, instancia: *mut [u64; 2]) {
    if !NOMES_DA_ENTREGA.vazia() {
        if let Some((inst, nome, controller)) = troca_da_entrega(entrega as *const u8, instancia, &NOMES_DA_ENTREGA) {
            #[cfg(feature = "devtools")]
            let inicio = Instant::now();
            let trocou = trocar_controller(inst, controller);
            #[cfg(feature = "devtools")]
            anotar_troca_da_entrega(nome, inst, trocou, inicio.elapsed());
            #[cfg(not(feature = "devtools"))]
            let _ = (nome, trocou);
        }
    }
    let original: Entrega = std::mem::transmute(original_publicado(&ORIG_ENTREGA_DO_SPAWN_ASSINCRONO));
    original(entrega, instancia);
}

/// `(instância, nome inteiro, controller)` da entrega: o nome do pedido em `tabela`. `None` com callable, pedido ou instância nulos, ou com o nome fora
/// da tabela.
unsafe fn troca_da_entrega<const N: usize>(entrega: *const u8, instancia: *const [u64; 2], tabela: &NomesDaEntrega<N>) -> Option<(*mut u8, u64, u64)> {
    if entrega.is_null() || instancia.is_null() {
        return None;
    }
    let pedido = (entrega.add(ENTREGA_PEDIDO) as *const u64).read_unaligned() as *const u8;
    if pedido.is_null() {
        return None;
    }
    let nome = (pedido.add(PEDIDO_NOME) as *const u64).read_unaligned();
    let controller = tabela.controller(nome)?;
    let inst = (*instancia)[0] as *mut u8;
    (!inst.is_null()).then_some((inst, nome, controller))
}

/// Trocas na entrega, pro veredito da 1ª do processo.
#[cfg(feature = "devtools")]
static ENTREGAS_TROCADAS: AtomicU64 = AtomicU64::new(0);

/// Tempo do `trocar_controller` na entrega: trocas, ns somados e o maior, pro veredito do `telasmod`.
#[cfg(any(test, feature = "devtools"))]
static CUSTO_DA_ENTREGA: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

/// `(trocas, média em µs, maior em µs)` do `trocar_controller` na entrega desde o começo do processo.
#[cfg(any(test, feature = "devtools"))]
pub(crate) fn custo_da_entrega() -> (u64, f64, f64) {
    let [n, soma, maior] = CUSTO_DA_ENTREGA.each_ref().map(|c| c.load(Ordering::Relaxed));
    (n, if n == 0 { 0.0 } else { soma as f64 / n as f64 / 1000.0 }, maior as f64 / 1000.0)
}

/// Veredito da 1ª troca na entrega do processo, com a thread em que ela rodou e o tempo da troca, e a troca pro teste das telas de mod
/// (`telasmod`).
#[cfg(feature = "devtools")]
unsafe fn anotar_troca_da_entrega(nome: u64, instancia: *mut u8, trocou: Option<*mut c_void>, tempo: Duration) {
    let ns = tempo.as_nanos().min(u128::from(u64::MAX)) as u64;
    CUSTO_DA_ENTREGA[0].fetch_add(1, Ordering::Relaxed);
    CUSTO_DA_ENTREGA[1].fetch_add(ns, Ordering::Relaxed);
    CUSTO_DA_ENTREGA[2].fetch_max(ns, Ordering::Relaxed);
    let primeira = crate::primeiras(&ENTREGAS_TROCADAS, 1);
    if !primeira && !super::teste_telas_de_mod::rodando() {
        return;
    }
    let texto = crate::cname::name_of(nome).unwrap_or_else(|| format!("{nome:#018x}"));
    if primeira {
        let t = std::thread::current();
        crate::veredito_dev!(
            "[spawn] entrega assíncrona '{texto}': controller trocado={} em {} µs; thread {:?} {:?}, principal {}, script em execução {}",
            trocou.is_some(),
            ns / 1000,
            t.id(),
            t.name(),
            pthread_main_np() != 0,
            crate::inicio::boot::exec_depth() > 0
        );
    }
    let raiz = [(instancia.add(ITEM_RAIZ) as *const u64).read_unaligned(), (instancia.add(ITEM_RAIZ + 8) as *const u64).read_unaligned()];
    super::teste_telas_de_mod::anotar_troca(&texto, trocou.unwrap_or(std::ptr::null_mut()), raiz, true);
}

/// O despacho de `InkWidget/Spawn` está ligado: há ouvinte registrado (`WIDGET_SPAWN_ARMADO`, o `OnActivateEvent` do `InkSpawningController` da
/// referência) e a chave do `BwmsToggleWidgetSpawnEvent` (`WIDGET_SPAWN_EVENT_ENABLED`) está ligada.
fn evento_ligado() -> bool {
    super::WIDGET_SPAWN_ARMADO.load(Ordering::Relaxed) && super::WIDGET_SPAWN_EVENT_ENABLED.load(Ordering::Relaxed)
}

/// Trampolim nu: o `x8` do chamador (a saída de 16 B) vira o 3º argumento.
#[unsafe(naked)]
unsafe extern "C" fn spawn_local_tramp() {
    core::arch::naked_asm!("mov x2, x8", "b {hook}", hook = sym depois_do_spawn_local)
}

unsafe extern "C" fn depois_do_spawn_local(lib: *mut c_void, item: u64, saida: *mut [u64; 2]) {
    let original = original_publicado(&ORIG_SPAWN_FROM_LOCAL) as *mut c_void;
    if !na_game_thread() {
        sret_chamar(original, [lib as u64, item, 0, 0], saida as *mut c_void);
        #[cfg(feature = "devtools")]
        anotar_repasse(0, item, saida);
        return;
    }
    sret_chamar(original, [lib as u64, item, 0, 0], saida as *mut c_void);
    let item = refazer_com_controller(item, saida, |base| sret_chamar(original, [lib as u64, base, 0, 0], saida as *mut c_void));
    let instancia = (*saida)[0] as usize;
    #[cfg(feature = "devtools")]
    {
        let n = CONTAGEM[0].fetch_add(1, Ordering::Relaxed);
        if n < 8 || n.is_power_of_two() {
            crate::log_dev!("[spawn] SpawnFromLocal #{n} lib={lib:p} item={item:#018x} instância={instancia:#x}");
        }
    }
    if instancia != 0 && evento_ligado() && !DENTRO_DO_EXTERNO.with(|d| d.get()) {
        disparar_spawn(caminho_da_biblioteca(lib), item, instancia);
    }
}

#[unsafe(naked)]
unsafe extern "C" fn spawn_externo_tramp() {
    core::arch::naked_asm!("mov x3, x8", "b {hook}", hook = sym depois_do_spawn_externo)
}

unsafe extern "C" fn depois_do_spawn_externo(lib: *mut c_void, caminho: u64, item: u64, saida: *mut [u64; 2]) {
    let original = original_publicado(&ORIG_SPAWN_FROM_EXTERNAL) as *mut c_void;
    if !na_game_thread() {
        sret_chamar(original, [lib as u64, caminho, item, 0], saida as *mut c_void);
        #[cfg(feature = "devtools")]
        anotar_repasse(1, item, saida);
        return;
    }
    injetar_dependencia(lib, caminho);
    let antes = DENTRO_DO_EXTERNO.with(|d| d.replace(true));
    sret_chamar(original, [lib as u64, caminho, item, 0], saida as *mut c_void);
    let item = refazer_com_controller(item, saida, |base| sret_chamar(original, [lib as u64, caminho, base, 0], saida as *mut c_void));
    DENTRO_DO_EXTERNO.with(|d| d.set(antes));
    let instancia = (*saida)[0] as usize;
    #[cfg(feature = "devtools")]
    {
        let n = CONTAGEM[1].fetch_add(1, Ordering::Relaxed);
        if n < 8 || n.is_power_of_two() {
            crate::log_dev!("[spawn] SpawnFromExternal #{n} lib={lib:p} externa={caminho:#018x} item={item:#018x} instância={instancia:#x}");
        }
        crate::selftest::c135_ver_spawn_externo(lib, caminho, item, instancia);
    }
    if instancia != 0 && evento_ligado() && !antes {
        disparar_spawn(caminho, item_do_evento(item), instancia);
    }
}

/// O item que o evento leva, como no `OnSpawnLocal` da referência: o `Item` de um nome `Item:Controller`, senão o próprio. No externo o nome inteiro
/// fica quando foi um `SpawnFromLocal` aninhado que refez o spawn.
fn item_do_evento(item: u64) -> u64 {
    crate::cname::name_of(item).and_then(|nome| item_e_controller(&nome).map(|(base, _)| base)).unwrap_or(item)
}

/// `Item:Controller` (`OnSpawnLocal` da referência): com o spawn pelo nome inteiro vazio, `spawnar` refaz com o `Item` e o controller do item vira o
/// pedido. Devolve o item que o evento leva: o `Item` quando o nome tem a forma, senão o de entrada.
unsafe fn refazer_com_controller(item: u64, saida: *mut [u64; 2], spawnar: impl Fn(u64)) -> u64 {
    if (*saida)[0] != 0 {
        return item;
    }
    let Some(nome) = crate::cname::name_of(item) else { return item };
    let Some((base, controller)) = item_e_controller(&nome) else { return item };
    spawnar(base);
    let instancia = (*saida)[0] as *mut u8;
    if !instancia.is_null() && !controller.is_empty() {
        let trocou = trocar_controller(instancia, cname(controller));
        #[cfg(feature = "devtools")]
        {
            CONTAGEM[2].fetch_add(1, Ordering::Relaxed);
            crate::veredito_dev!("[spawn] '{nome}': controller {controller} trocado={}", trocou.is_some());
            // O teste das telas de mod (`telasmod`) confere depois que o controller trocado aqui é o que o motor inicializou.
            let raiz = [(instancia.add(ITEM_RAIZ) as *const u64).read_unaligned(), (instancia.add(ITEM_RAIZ + 8) as *const u64).read_unaligned()];
            super::teste_telas_de_mod::anotar_troca(&nome, trocou.unwrap_or(std::ptr::null_mut()), raiz, false);
        }
        #[cfg(not(feature = "devtools"))]
        let _ = trocou;
    }
    base
}

/// `Item:Controller` partido no 1º `:` (o `strchr` da referência): o hash do `Item` e o nome do controller, que pode vir vazio. `None` sem `:` ou com o
/// `Item` vazio, que não nomeia item da biblioteca.
pub(crate) fn item_e_controller(nome: &str) -> Option<(u64, &str)> {
    let (item, controller) = nome.split_once(':')?;
    (!item.is_empty()).then(|| (cname(item), controller))
}

/// `InjectController` da referência: um controller novo da classe de nome `controller` (hash do `CName`; nativa ou de script) entra no lugar do
/// `gameController` do item (classe de `inkIWidgetController`) ou do `logicController` da raiz (classe de `inkWidgetLogicController`) e herda do antigo
/// os campos de mesmo nome e tipo. O antigo é solto. Devolve o controller novo; `None` com classe desconhecida, de outra família, ou sem raiz.
unsafe fn trocar_controller(instancia: *mut u8, controller: u64) -> Option<*mut c_void> {
    let reg = rtti::Registry::obtain()?;
    let cls = reg.script_class(controller);
    if !rtti::sane(cls) {
        return None;
    }
    let campo = campo_do_controller(cls, reg.class_by_name("inkIWidgetController"), reg.class_by_name("inkWidgetLogicController"), instancia)?;
    let novo = rtti::create_instance(cls);
    if novo.is_null() {
        return None;
    }
    let mut dono = [0u64; 2];
    rtti::make_handle(dono.as_mut_ptr() as *mut c_void, novo);
    let mut antigo = [campo.read_unaligned(), campo.add(1).read_unaligned()];
    campo.write_unaligned(dono[0]);
    campo.add(1).write_unaligned(dono[1]);
    if antigo[0] != 0 {
        let velho = antigo[0] as *mut c_void;
        herdar_campos(novo, cls, velho, rtti::class_of(velho));
    }
    soltar_handle(&mut antigo);
    Some(novo)
}

/// O campo que o controller `cls` ocupa na instância spawnada: o `gameController` do item (`+0x48`) pra filha de `cls_game` (`inkIWidgetController`), o
/// `logicController` da raiz (`rootWidget +0x38` → `+0x128`) pra filha de `cls_logica` (`inkWidgetLogicController`). `None` com classe de outra família ou
/// raiz nula/ilegível.
unsafe fn campo_do_controller(cls: *mut c_void, cls_game: *mut c_void, cls_logica: *mut c_void, instancia: *mut u8) -> Option<*mut u64> {
    if class_is_kind_of(cls, cls_game) {
        return Some(instancia.add(ITEM_CONTROLLER) as *mut u64);
    }
    if !class_is_kind_of(cls, cls_logica) {
        return None;
    }
    let raiz = (instancia.add(ITEM_RAIZ) as *const u64).read_unaligned() as *mut u8;
    let campo = raiz.wrapping_add(super::C175_WIDGET_CONTROLLER);
    (!raiz.is_null() && crate::memoria::is_readable(campo as *const c_void, 16)).then_some(campo as *mut u64)
}

/// `InjectDependency` da referência: o `SpawnFromExternal` do motor só acha a biblioteca externa que está no `externalLibraries` da biblioteca de quem
/// chama, e a de um mod (Virtual Atelier, Equipment-EX) não está no `.inkwidget` vanilla do controller. Sem o caminho na lista, a referência entra no fim
/// com o token da carga do motor, e a espera vai até 1 s, tudo sob a trava de escrita. Devolve se entrou.
unsafe fn injetar_dependencia(lib: *mut c_void, caminho: u64) -> bool {
    if caminho == 0 || !rtti::sane(lib) {
        return false;
    }
    let Some(off) = offset_das_externas() else { return false };
    let arr = (lib as *mut u8).add(off);
    // Sem `is_readable`: a original lê a mesma lista logo depois, e o ponteiro é do motor.
    let presente = || {
        let (entradas, n, _) = dynarray_no_campo(arr, 0);
        dependencia_presente(entradas, n, caminho)
    };
    {
        let _leitura = TRAVA_DAS_EXTERNAS.read().unwrap_or_else(PoisonError::into_inner);
        if presente() {
            return false;
        }
    }
    let _escrita = TRAVA_DAS_EXTERNAS.write().unwrap_or_else(PoisonError::into_inner);
    if presente() {
        return false;
    }
    let Some(vaga) = vaga_nas_externas(arr) else { return false };
    let Some(token) = crate::jogo::recursos::token_do_path(caminho).filter(|t| t[0] != 0) else { return false };
    let inicio = Instant::now();
    anexar_externa(arr, vaga, caminho, token);
    esperar_dependencia(token[0]);
    crate::veredito_dev!(
        "[spawn] dependência {caminho:#018x} injetada no externalLibraries (+{off:#x}) da biblioteca {:#018x}: {} referência(s), token {} em {} ms",
        caminho_da_biblioteca(lib),
        dynarray_no_campo(arr, 0).1,
        ["carregando", "carregado", "falhou"].get(crate::register::status_do_token(token[0]) as usize).copied().unwrap_or("?"),
        inicio.elapsed().as_millis()
    );
    true
}

/// O offset guardado do `externalLibraries`, resolvido pelo RTTI na 1ª chamada. Sem registro de RTTI a próxima tenta de novo; layout recusado fica
/// recusado.
unsafe fn offset_das_externas() -> Option<usize> {
    match EXTERNAS_OFF.load(Ordering::Relaxed) {
        EXTERNAS_RECUSADO => None,
        0 => {
            let reg = rtti::Registry::obtain()?;
            let cls = reg.class_by_name("inkWidgetLibraryResource");
            let prop = if cls.is_null() { None } else { rtti::resolve_prop_in_class(cls, "externalLibraries") };
            let off = prop.and_then(|(off, tipo, no_holder)| {
                let elemento = rtti::get_inner_type(tipo).unwrap_or(std::ptr::null_mut());
                layout_das_externas(off, no_holder, rtti::type_kind(tipo), rtti::type_kind(elemento), rtti::type_size(elemento))
            });
            EXTERNAS_OFF.store(off.map_or(EXTERNAS_RECUSADO, |o| o as u64), Ordering::Relaxed);
            crate::veredito_dev!(
                "[spawn] externalLibraries do inkWidgetLibraryResource pelo RTTI: {prop:?} (SDK do Windows +0x58) -> injeção de dependência {}",
                off.map_or("desligada".to_string(), |o| format!("em +{o:#x}"))
            );
            off
        }
        off => Some(off as usize),
    }
}

/// O offset do `externalLibraries` quando o RTTI descreve o layout que a injeção grava: `array` fora do value holder, de `ResourceReference` de 0x18 B,
/// depois do `CResource::path`. Outro layout desliga a injeção em vez de gravar no lugar errado.
fn layout_das_externas(off: u32, no_holder: bool, tipo: Option<u8>, elemento: Option<u8>, tamanho_do_elemento: u32) -> Option<usize> {
    let ok = !no_holder
        && tipo == Some(rtti::ertti::ARRAY)
        && elemento == Some(rtti::ertti::RESOURCE_REFERENCE)
        && tamanho_do_elemento as usize == EXTERNA_PASSO
        && (BIBLIOTECA_CAMINHO as u32 + 8..0x100).contains(&off);
    ok.then_some(off as usize)
}

/// O caminho já está entre as `n` referências de `entradas`, como o laço da referência.
unsafe fn dependencia_presente(entradas: *const u8, n: usize, caminho: u64) -> bool {
    !entradas.is_null() && (0..n).any(|i| (entradas.add(i * EXTERNA_PASSO) as *const u64).read_unaligned() == caminho)
}

/// Vaga no fim do `externalLibraries` (cabeçalho em `arr`), crescendo pelo reserve do motor com mover nulo: o elemento é caminho + `SharedPtr`, que a
/// realocação move pelos bytes. `None` se o motor não deu capacidade.
unsafe fn vaga_nas_externas(arr: *mut u8) -> Option<*mut u8> {
    let (_, n, cap) = dynarray_no_campo(arr, 0);
    if n >= cap {
        crate::jogo::listas::reservar(arr as *mut c_void, (n + 1) as u32, EXTERNA_PASSO as u32, 8, std::ptr::null());
    }
    let (entradas, n, cap) = dynarray_no_campo(arr, 0);
    (!entradas.is_null() && n < cap).then(|| entradas.add(n * EXTERNA_PASSO))
}

/// Grava a referência (caminho e o `SharedPtr` do token, que passa a ser do array) na vaga e publica o tamanho novo.
unsafe fn anexar_externa(arr: *mut u8, vaga: *mut u8, caminho: u64, token: [u64; 2]) {
    (vaga as *mut u64).write_unaligned(caminho);
    (vaga.add(0x08) as *mut u64).write_unaligned(token[0]);
    (vaga.add(0x10) as *mut u64).write_unaligned(token[1]);
    let n = (arr.add(0x0c) as *const u32).read_unaligned();
    (arr.add(0x0c) as *mut u32).write_unaligned(n + 1);
}

/// Espera a carga da biblioteca injetada terminar (carregada ou falha) até [`ESPERA_DA_DEPENDENCIA`]. A fila com `Wait` no job não bloqueia neste binário,
/// então a espera confere o `finished` do token a cada 1 ms, como `arquivos_do_jogo::esperar_terminar`.
unsafe fn esperar_dependencia(token: u64) {
    crate::arquivos_do_jogo::esperar_token(token);
    let inicio = Instant::now();
    while crate::register::status_do_token(token) == 0 && inicio.elapsed() < ESPERA_DA_DEPENDENCIA {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// `InheritProperties` da referência: cada campo de `origem` (classe `cls_origem` e pais) vai pelo `Assign` do tipo pro campo de mesmo nome e tipo de
/// `destino`. Devolve quantos copiou.
pub(crate) unsafe fn herdar_campos(destino: *mut c_void, cls_destino: *mut c_void, origem: *mut c_void, cls_origem: *mut c_void) -> usize {
    let de = rtti::props_crus(cls_origem);
    let para = rtti::props_crus(cls_destino);
    let chaves = |props: &[*const u8]| -> Vec<(u64, usize)> {
        props.iter().map(|p| ((p.add(0x08) as *const u64).read_unaligned(), (*p as *const u64).read_unaligned() as usize)).collect()
    };
    let mut copiados = 0;
    for (i, j) in campos_herdados(&chaves(&de), &chaves(&para)) {
        if let (Some((valor, tipo)), Some((campo, _))) = (rtti::campo_e_tipo(origem, de[i] as *const c_void), rtti::campo_e_tipo(destino, para[j] as *const c_void)) {
            copiados += rtti::type_assign(tipo, campo, valor) as usize;
        }
    }
    copiados
}

/// Pares `(índice na origem, índice no destino)` da herança de campos, por `(nome, tipo)`: o 1º campo do destino com o nome (a classe antes dos pais,
/// como o `GetProperty`), e só quando o tipo é o mesmo.
pub(crate) fn campos_herdados(origem: &[(u64, usize)], destino: &[(u64, usize)]) -> Vec<(usize, usize)> {
    origem
        .iter()
        .enumerate()
        .filter_map(|(i, (nome, tipo))| {
            let j = destino.iter().position(|(n, _)| n == nome)?;
            (destino[j].1 == *tipo).then_some((i, j))
        })
        .collect()
}

/// `CResource::path` da biblioteca; 0 ilegível.
unsafe fn caminho_da_biblioteca(lib: *mut c_void) -> u64 {
    let campo = (lib as *const u8).wrapping_add(BIBLIOTECA_CAMINHO);
    if lib.is_null() || !crate::memoria::is_readable(campo as *const c_void, 8) {
        return 0;
    }
    (campo as *const u64).read_unaligned()
}

/// `InkWidget/Spawn` com a biblioteca, o item e a instância (`inkWidgetSpawnEvent`), como o `DispatchNativeEvent` da referência. Depois dos ouvintes o
/// evento sai da tabela e é solto.
unsafe fn disparar_spawn(biblioteca: u64, item: u64, instancia: usize) -> usize {
    #[cfg(feature = "devtools")]
    crate::selftest::c135_anotar(biblioteca, item, instancia);
    let Some(arg) = super::make_inkwidgetspawnevent_arg(biblioteca, item, instancia) else { return 0 };
    let (evento, bloco) = match &arg {
        rtti::Arg::Handle(i, r) => (*i as u64, *r as u64),
        _ => (0, 0),
    };
    let enviados = crate::tipos_do_motor::fire_event_args("InkWidget/Spawn", &[arg]);
    if let Ok(mut estados) = super::INKWIDGETSPAWN_STATES.lock() {
        estados.retain(|e| e.0 as u64 != evento);
    }
    let mut h = [evento, bloco];
    soltar_handle(&mut h);
    enviados
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::tipos_do_motor::TESTES_DE_CALLBACKS_EM_SERIE;
    use std::sync::Mutex;

    thread_local! {
        /// Se o hook vê a thread do teste como a game thread (`na_game_thread`).
        pub(super) static GAME_THREAD_NO_TESTE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    }

    /// Itens que a original falsa recebeu, na ordem.
    static PEDIDOS: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    const INSTANCIA_FALSA: u64 = 0x7e57_5000_0000;

    /// Original falsa dos dois spawns: acha só o item `Root`, como a biblioteca que não tem `Root:Controller`.
    unsafe extern "C" fn spawn_falso(item: u64, saida: *mut [u64; 2]) {
        PEDIDOS.lock().unwrap_or_else(|e| e.into_inner()).push(item);
        *saida = if item == cname("Root") { [INSTANCIA_FALSA, 0] } else { [0, 0] };
    }

    unsafe extern "C" fn local_falso(_lib: *mut c_void, item: u64, saida: *mut [u64; 2]) {
        spawn_falso(item, saida);
    }

    unsafe extern "C" fn externo_falso(_lib: *mut c_void, _caminho: u64, item: u64, saida: *mut [u64; 2]) {
        spawn_falso(item, saida);
    }

    #[unsafe(naked)]
    unsafe extern "C" fn local_falso_tramp() {
        core::arch::naked_asm!("mov x2, x8", "b {f}", f = sym local_falso)
    }

    #[unsafe(naked)]
    unsafe extern "C" fn externo_falso_tramp() {
        core::arch::naked_asm!("mov x3, x8", "b {f}", f = sym externo_falso)
    }

    /// Chama o hook pela entrada, como o motor: argumentos em `x0`–`x2` e a saída no `x8`. Devolve a saída e os itens que a original recebeu.
    unsafe fn pelo_hook(tramp: unsafe extern "C" fn(), args: [u64; 4]) -> ([u64; 2], Vec<u64>) {
        PEDIDOS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let mut saida = [0xdead_u64, 0xdead];
        sret_chamar(tramp as *mut c_void, args, saida.as_mut_ptr() as *mut c_void);
        (saida, PEDIDOS.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    fn publicar(orig: &AtomicU64, f: unsafe extern "C" fn()) -> u64 {
        orig.swap(f as usize as u64, Ordering::Relaxed)
    }

    #[test]
    fn item_e_controller_parte_no_primeiro_dois_pontos() {
        assert_eq!(item_e_controller("Root:EquipmentEx.WardrobeScreenController"), Some((cname("Root"), "EquipmentEx.WardrobeScreenController")));
        assert_eq!(item_e_controller("AtelierStores:VirtualAtelier.UI.AtelierStoresListController"), Some((cname("AtelierStores"), "VirtualAtelier.UI.AtelierStoresListController")));
        assert_eq!(item_e_controller("a:b:c"), Some((cname("a"), "b:c")), "o controller fica com o resto, como o strchr");
        assert_eq!(item_e_controller("Root:"), Some((cname("Root"), "")), "controller vazio: spawna o item sem trocar");
        assert_eq!(item_e_controller("Root"), None);
        assert_eq!(item_e_controller(":Controller"), None, "item vazio não nomeia item da biblioteca");
        assert_eq!(item_e_controller(""), None);
    }

    // Nomes do espelho (`intern`): o pool do motor não existe no teste.
    #[test]
    fn item_do_evento_tira_o_controller() {
        assert_eq!(item_do_evento(crate::cname::intern("TesteItem:TesteController")), cname("TesteItem"));
        let so_item = crate::cname::intern("TesteItem");
        assert_eq!(item_do_evento(so_item), so_item);
    }

    #[test]
    fn campos_herdados_pelo_primeiro_nome_e_mesmo_tipo() {
        let origem = [(1, 0x10), (2, 0x20), (3, 0x30), (4, 0x40)];
        // O nome 2 aparece 2 vezes no destino (classe e pai): vale o 1º, de tipo diferente, e o campo não passa — o `GetProperty` não procura o 2º.
        let destino = [(3, 0x30), (2, 0x99), (2, 0x20), (1, 0x10)];
        assert_eq!(campos_herdados(&origem, &destino), vec![(0, 3), (2, 0)]);
        assert_eq!(campos_herdados(&[], &destino), vec![]);
        assert_eq!(campos_herdados(&origem, &[]), vec![]);
    }

    /// Classe falsa com o layout que `rtti::props_crus` lê: props em `+0x28` (tamanho em `+0x34`), pai em `+0x10`; cada prop com o tipo em `+0x00`, o
    /// CName em `+0x08`, o offset do valor em `+0x20` e as flags (sem `inValueHolder`) em `+0x28`.
    struct ClasseFalsa {
        classe: Box<[u64; 16]>,
        lista: Box<[u64; 4]>,
        props: Box<[[u64; 6]; 4]>,
    }

    impl ClasseFalsa {
        fn nova(campos: &[(&str, *const [u64; 12], u32)], pai: *mut c_void) -> Self {
            let mut c = ClasseFalsa { classe: Box::new([0; 16]), lista: Box::new([0; 4]), props: Box::new([[0; 6]; 4]) };
            for (i, (nome, tipo, off)) in campos.iter().enumerate() {
                c.props[i][0] = *tipo as u64;
                c.props[i][1] = cname(nome);
                c.props[i][4] = *off as u64;
                c.lista[i] = c.props[i].as_ptr() as u64;
            }
            c.classe[0x10 / 8] = pai as u64;
            c.classe[0x28 / 8] = c.lista.as_ptr() as u64;
            c.classe[0x30 / 8] = ((campos.len() as u64) << 32) | 4;
            c
        }

        fn cls(&mut self) -> *mut c_void {
            self.classe.as_mut_ptr() as *mut c_void
        }
    }

    /// `Assign` dos tipos falsos (slot `+0x58` da vtable): copia 8 B.
    extern "C" fn assign_de_8(_tipo: *mut c_void, destino: *mut c_void, origem: *const c_void) {
        unsafe { (destino as *mut u64).write_unaligned((origem as *const u64).read_unaligned()) };
    }

    /// Tipo falso: `+0x00` aponta a vtable, que é o próprio array com o `Assign` em `+0x58`.
    fn tipo_falso() -> Box<[u64; 12]> {
        let mut t = Box::new([0u64; 12]);
        t[0x58 / 8] = assign_de_8 as *const () as usize as u64;
        let base = t.as_ptr() as u64;
        t[0] = base;
        t
    }

    // Destino com a classe filha (o controller pedido herda do antigo): os campos do pai entram pela cadeia, o de mesmo nome e outro tipo fica, e o que
    // só a origem tem não vai pra lugar nenhum.
    #[test]
    fn herdar_campos_copia_pelo_assign_so_nome_e_tipo_iguais() {
        let (int, texto) = (tipo_falso(), tipo_falso());
        let (int, texto): (*const [u64; 12], *const [u64; 12]) = (&*int, &*texto);
        let mut origem_cls = ClasseFalsa::nova(&[("widgetA", int, 0x40), ("contador", int, 0x48), ("rotulo", texto, 0x50), ("soNaOrigem", int, 0x58)], std::ptr::null_mut());
        let mut pai = ClasseFalsa::nova(&[("widgetA", int, 0x40), ("contador", int, 0x48)], std::ptr::null_mut());
        let mut filha = ClasseFalsa::nova(&[("rotulo", int, 0x50), ("extra", int, 0x60)], pai.cls());
        let mut origem = [0u64; 16];
        origem[0x40 / 8] = 11;
        origem[0x48 / 8] = 22;
        origem[0x50 / 8] = 33;
        origem[0x58 / 8] = 44;
        let mut destino = [0u64; 16];
        destino[0x50 / 8] = 7;
        let n = unsafe { herdar_campos(destino.as_mut_ptr() as *mut c_void, filha.cls(), origem.as_mut_ptr() as *mut c_void, origem_cls.cls()) };
        assert_eq!(n, 2, "widgetA e contador");
        assert_eq!((destino[0x40 / 8], destino[0x48 / 8], destino[0x50 / 8], destino[0x58 / 8], destino[0x60 / 8]), (11, 22, 7, 0, 0));
    }

    // O hook com a original falsa, pela entrada como o motor chama: spawn achado passa sem pedir de novo; `Item:` vazio refaz com o `Item`; nome sem a
    // forma ou com o item vazio fica vazio com 1 pedido só. Os nomes vêm do espelho (`intern`): o pool do motor não existe no teste. Sem ouvinte, nada
    // de evento; com o controller vazio, nada de RTTI; com a injeção recusada, a biblioteca falsa (fora da memória) não é lida. A trava dos testes de
    // callbacks segura o ouvinte desligado: o `atualizar_eventos_de_entidade` de outro teste o liga junto com o ouvinte de `InkWidget/Spawn`.
    #[test]
    fn hook_refaz_o_spawn_vazio_pelo_item_do_nome() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(|e| e.into_inner());
        let ligado_antes = crate::interface::WIDGET_SPAWN_ARMADO.swap(false, Ordering::Relaxed);
        let externas_antes = EXTERNAS_OFF.swap(EXTERNAS_RECUSADO, Ordering::Relaxed);
        let (root, com_vazio, sem_forma, item_vazio) =
            (cname("Root"), crate::cname::intern("Root:"), crate::cname::intern("TesteSemForma"), crate::cname::intern(":TesteController"));
        let lib = 0x7e57_6000_0000u64;
        let antes = (publicar(&ORIG_SPAWN_FROM_LOCAL, local_falso_tramp), publicar(&ORIG_SPAWN_FROM_EXTERNAL, externo_falso_tramp));
        let lidos = unsafe {
            [
                pelo_hook(spawn_local_tramp, [lib, root, 0, 0]),
                pelo_hook(spawn_local_tramp, [lib, com_vazio, 0, 0]),
                pelo_hook(spawn_local_tramp, [lib, sem_forma, 0, 0]),
                pelo_hook(spawn_local_tramp, [lib, item_vazio, 0, 0]),
                pelo_hook(spawn_externo_tramp, [lib, 0x1234, root, 0]),
                pelo_hook(spawn_externo_tramp, [lib, 0x1234, com_vazio, 0]),
                pelo_hook(spawn_externo_tramp, [lib, 0x1234, sem_forma, 0]),
            ]
        };
        ORIG_SPAWN_FROM_LOCAL.store(antes.0, Ordering::Relaxed);
        ORIG_SPAWN_FROM_EXTERNAL.store(antes.1, Ordering::Relaxed);
        crate::interface::WIDGET_SPAWN_ARMADO.store(ligado_antes, Ordering::Relaxed);
        EXTERNAS_OFF.store(externas_antes, Ordering::Relaxed);
        let achado = [INSTANCIA_FALSA, 0];
        assert_eq!(lidos[0], (achado, vec![root]));
        assert_eq!(lidos[1], (achado, vec![com_vazio, root]));
        assert_eq!(lidos[2], ([0, 0], vec![sem_forma]));
        assert_eq!(lidos[3], ([0, 0], vec![item_vazio]));
        assert_eq!(lidos[4], (achado, vec![root]));
        assert_eq!(lidos[5], (achado, vec![com_vazio, root]));
        assert_eq!(lidos[6], ([0, 0], vec![sem_forma]));
        assert!(!DENTRO_DO_EXTERNO.with(|d| d.get()), "o externo devolve a marca");
    }

    // Fora da thread principal e sem script em execução nela, o hook só chama a original, nos dois: o `Item:` vazio fica vazio, sem o 2º pedido pelo
    // `Item`.
    #[test]
    fn fora_da_game_thread_o_hook_so_chama_a_original() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(|e| e.into_inner());
        let ligado_antes = crate::interface::WIDGET_SPAWN_ARMADO.swap(false, Ordering::Relaxed);
        let externas_antes = EXTERNAS_OFF.swap(EXTERNAS_RECUSADO, Ordering::Relaxed);
        let com_vazio = crate::cname::intern("Root:");
        let lib = 0x7e57_6000_0000u64;
        let antes = (publicar(&ORIG_SPAWN_FROM_LOCAL, local_falso_tramp), publicar(&ORIG_SPAWN_FROM_EXTERNAL, externo_falso_tramp));
        GAME_THREAD_NO_TESTE.with(|g| g.set(false));
        let lidos = unsafe { [pelo_hook(spawn_local_tramp, [lib, com_vazio, 0, 0]), pelo_hook(spawn_externo_tramp, [lib, 0x1234, com_vazio, 0])] };
        GAME_THREAD_NO_TESTE.with(|g| g.set(true));
        ORIG_SPAWN_FROM_LOCAL.store(antes.0, Ordering::Relaxed);
        ORIG_SPAWN_FROM_EXTERNAL.store(antes.1, Ordering::Relaxed);
        crate::interface::WIDGET_SPAWN_ARMADO.store(ligado_antes, Ordering::Relaxed);
        EXTERNAS_OFF.store(externas_antes, Ordering::Relaxed);
        assert_eq!(lidos[0], ([0, 0], vec![com_vazio]));
        assert_eq!(lidos[1], ([0, 0], vec![com_vazio]));
    }

    // Fora da thread principal com script em execução nesta thread (o executor aberto, como num worker da gameplay), o hook refaz o `Item:` vazio
    // pelo `Item`, nos dois.
    #[test]
    fn fora_da_principal_com_script_em_execucao_o_hook_refaz() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(|e| e.into_inner());
        let ligado_antes = crate::interface::WIDGET_SPAWN_ARMADO.swap(false, Ordering::Relaxed);
        let externas_antes = EXTERNAS_OFF.swap(EXTERNAS_RECUSADO, Ordering::Relaxed);
        let (root, com_vazio) = (cname("Root"), crate::cname::intern("Root:"));
        let lib = 0x7e57_6000_0000u64;
        let antes = (publicar(&ORIG_SPAWN_FROM_LOCAL, local_falso_tramp), publicar(&ORIG_SPAWN_FROM_EXTERNAL, externo_falso_tramp));
        GAME_THREAD_NO_TESTE.with(|g| g.set(false));
        let lidos = crate::inicio::boot::com_o_executor_aberto(|| unsafe {
            [pelo_hook(spawn_local_tramp, [lib, com_vazio, 0, 0]), pelo_hook(spawn_externo_tramp, [lib, 0x1234, com_vazio, 0])]
        });
        GAME_THREAD_NO_TESTE.with(|g| g.set(true));
        ORIG_SPAWN_FROM_LOCAL.store(antes.0, Ordering::Relaxed);
        ORIG_SPAWN_FROM_EXTERNAL.store(antes.1, Ordering::Relaxed);
        crate::interface::WIDGET_SPAWN_ARMADO.store(ligado_antes, Ordering::Relaxed);
        EXTERNAS_OFF.store(externas_antes, Ordering::Relaxed);
        assert_eq!(lidos[0], ([INSTANCIA_FALSA, 0], vec![com_vazio, root]));
        assert_eq!(lidos[1], ([INSTANCIA_FALSA, 0], vec![com_vazio, root]));
    }

    /// Offset do `externalLibraries` na biblioteca falsa.
    const EXTERNAS_FALSO: u64 = 0x48;

    /// Biblioteca falsa com o `externalLibraries` em [`EXTERNAS_FALSO`]: cabeçalho `{entradas, capacidade u32, tamanho u32}` apontando pra `entradas`.
    fn biblioteca_falsa(entradas: &mut [u64], capacidade: u32, tamanho: u32) -> Box<[u64; 16]> {
        let mut lib = Box::new([0u64; 16]);
        lib[EXTERNAS_FALSO as usize / 8] = entradas.as_mut_ptr() as u64;
        lib[EXTERNAS_FALSO as usize / 8 + 1] = capacidade as u64 | (tamanho as u64) << 32;
        lib
    }

    // A escolha do campo do `InjectController`: classe da família do `gameController` (ela mesma ou filha) vai pro `+0x48` do item; da família do
    // `logicController`, pro `+0x128` da raiz (`+0x38` do item), subindo mais de um pai; raiz nula ou classe de outra família não dão campo. As classes
    // falsas têm o pai em `+0x10`, que é o que o `class_is_kind_of` sobe.
    #[test]
    fn campo_do_controller_pela_familia_da_classe() {
        let mut game = ClasseFalsa::nova(&[], std::ptr::null_mut());
        let mut logica = ClasseFalsa::nova(&[], std::ptr::null_mut());
        let mut filha_game = ClasseFalsa::nova(&[], game.cls());
        let mut filha_logica = ClasseFalsa::nova(&[], logica.cls());
        let mut neta_logica = ClasseFalsa::nova(&[], filha_logica.cls());
        let mut outra = ClasseFalsa::nova(&[], std::ptr::null_mut());
        let mut raiz = Box::new([0u64; 0x140 / 8]);
        let mut item = [0u64; 16];
        item[ITEM_RAIZ / 8] = raiz.as_mut_ptr() as u64;
        let base = item.as_mut_ptr() as *mut u8;
        let (g, l) = (game.cls(), logica.cls());
        unsafe {
            let campo = |cls| campo_do_controller(cls, g, l, base);
            let no_item = Some(base.add(ITEM_CONTROLLER) as *mut u64);
            let na_raiz = Some((raiz.as_mut_ptr() as *mut u8).add(0x128) as *mut u64);
            assert_eq!(campo(filha_game.cls()), no_item);
            assert_eq!(campo(g), no_item, "a própria classe base");
            assert_eq!(campo(filha_logica.cls()), na_raiz);
            assert_eq!(campo(neta_logica.cls()), na_raiz, "sobe mais de um pai");
            assert_eq!(campo(outra.cls()), None, "outra família");
            (base.add(ITEM_RAIZ) as *mut u64).write_unaligned(0);
            assert_eq!(campo(filha_logica.cls()), None, "raiz nula");
            assert_eq!(campo(filha_game.cls()), no_item, "o gameController não depende da raiz");
        }
    }

    #[test]
    fn caminho_da_biblioteca_le_o_path_do_recurso() {
        let mut lib = [0u64; 8];
        lib[BIBLIOTECA_CAMINHO / 8] = 0xd6b2_a2bc_eeee_5160;
        unsafe {
            assert_eq!(caminho_da_biblioteca(std::ptr::null_mut()), 0, "nula");
            assert_eq!(caminho_da_biblioteca(0x10 as *mut c_void), 0, "ilegível");
            assert_eq!(caminho_da_biblioteca(lib.as_mut_ptr() as *mut c_void), 0xd6b2_a2bc_eeee_5160);
        }
    }

    #[test]
    fn layout_das_externas_so_aceita_o_que_a_injecao_grava() {
        use rtti::ertti::{ARRAY, RESOURCE_ASYNC_REFERENCE, RESOURCE_REFERENCE, STATIC_ARRAY};
        let certo = |off| layout_das_externas(off, false, Some(ARRAY), Some(RESOURCE_REFERENCE), 0x18);
        assert_eq!(certo(0x50), Some(0x50));
        assert_eq!(certo(0x38), Some(0x38), "logo depois do CResource::path");
        assert_eq!(certo(0x30), None, "em cima do path");
        assert_eq!(certo(0x100), None);
        assert_eq!(layout_das_externas(0x50, true, Some(ARRAY), Some(RESOURCE_REFERENCE), 0x18), None, "no value holder");
        assert_eq!(layout_das_externas(0x50, false, Some(STATIC_ARRAY), Some(RESOURCE_REFERENCE), 0x18), None);
        assert_eq!(layout_das_externas(0x50, false, Some(ARRAY), Some(RESOURCE_ASYNC_REFERENCE), 0x08), None, "raRef: só o caminho, sem token");
        assert_eq!(layout_das_externas(0x50, false, Some(ARRAY), Some(RESOURCE_REFERENCE), 0x10), None, "outro tamanho");
        assert_eq!(layout_das_externas(0x50, false, None, None, 0), None, "RTTI sem resposta");
    }

    // Lista com capacidade: a busca acha pelo caminho, a vaga é o fim, o anexo grava caminho e token e publica o tamanho. O reserve do motor só entra
    // com a lista cheia, e esse caminho não roda fora do jogo.
    #[test]
    fn externas_com_capacidade_anexam_sem_o_motor() {
        let (a, b, c) = (0xaaaa_u64, 0xbbbb_u64, 0xcccc_u64);
        let mut entradas = [a, 1, 2, b, 3, 4, 0, 0, 0, 0, 0, 0];
        let mut lib = biblioteca_falsa(&mut entradas, 4, 2);
        unsafe {
            let arr = (lib.as_mut_ptr() as *mut u8).add(EXTERNAS_FALSO as usize);
            let (e, n, _) = dynarray_no_campo(arr, 0);
            assert!(dependencia_presente(e, n, a) && dependencia_presente(e, n, b));
            assert!(!dependencia_presente(e, n, c));
            assert!(!dependencia_presente(std::ptr::null(), 3, a), "lista sem buffer");
            let vaga = vaga_nas_externas(arr).expect("lista com capacidade tem vaga");
            assert_eq!(vaga, e.add(2 * EXTERNA_PASSO));
            anexar_externa(arr, vaga, c, [0x70ce_0000, 0x70ce_1000]);
            let (e, n, cap) = dynarray_no_campo(arr, 0);
            assert_eq!((n, cap), (3, 4));
            assert!(dependencia_presente(e, n, c));
        }
        assert_eq!(entradas[6..9], [c, 0x70ce_0000, 0x70ce_1000]);
    }

    // O `SpawnFromExternal` pela entrada hookada com a biblioteca externa já na lista de quem chama (o caso de todo spawn vanilla) e com caminho 0, as
    // duas com a lista cheia: a lista não muda e a original recebe o pedido uma vez. Se a injeção seguisse, o reserve do motor rodaria fora do jogo.
    #[test]
    fn hook_externo_nao_mexe_na_lista_que_ja_tem_a_dependencia() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(|e| e.into_inner());
        let ligado_antes = crate::interface::WIDGET_SPAWN_ARMADO.swap(false, Ordering::Relaxed);
        let externas_antes = EXTERNAS_OFF.swap(EXTERNAS_FALSO, Ordering::Relaxed);
        let (caminho, root) = (0xbbbb_u64, cname("Root"));
        let mut entradas = [0xaaaa_u64, 1, 2, caminho, 3, 4];
        let mut lib = biblioteca_falsa(&mut entradas, 2, 2);
        let lib_ptr = lib.as_mut_ptr() as u64;
        let antes = publicar(&ORIG_SPAWN_FROM_EXTERNAL, externo_falso_tramp);
        let lidos = unsafe { [pelo_hook(spawn_externo_tramp, [lib_ptr, caminho, root, 0]), pelo_hook(spawn_externo_tramp, [lib_ptr, 0, root, 0])] };
        ORIG_SPAWN_FROM_EXTERNAL.store(antes, Ordering::Relaxed);
        EXTERNAS_OFF.store(externas_antes, Ordering::Relaxed);
        crate::interface::WIDGET_SPAWN_ARMADO.store(ligado_antes, Ordering::Relaxed);
        for lido in lidos {
            assert_eq!(lido, ([INSTANCIA_FALSA, 0], vec![root]));
        }
        assert_eq!(lib[EXTERNAS_FALSO as usize / 8 + 1], 2 | 2 << 32, "capacidade e tamanho intactos");
        assert_eq!(entradas, [0xaaaa, 1, 2, caminho, 3, 4]);
    }

    // A chave do `BwmsToggleWidgetSpawnEvent` e o ouvinte são atômicos separados, e o registro de qualquer evento regrava só o do ouvinte: a chave
    // nasce ligada, desligada segura o despacho com ouvinte e não volta sozinha no registro de outro evento, e ligada sem ouvinte não despacha.
    #[test]
    fn chave_do_toggle_sobrevive_ao_registro_de_outro_evento() {
        use crate::tipos_do_motor::{atualizar_eventos_de_entidade, CALLBACKS};
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(|e| e.into_inner());
        let (armado_antes, chave_antes) =
            (crate::interface::WIDGET_SPAWN_ARMADO.load(Ordering::Relaxed), crate::interface::WIDGET_SPAWN_EVENT_ENABLED.load(Ordering::Relaxed));
        let (alvo, outro) = (0x7e57_0013_0000usize, 0x7e57_0014_0000usize);
        let ouvinte = |evento: &str, quem: usize, entra: bool| {
            if let Ok(mut c) = CALLBACKS.lock() {
                if entra {
                    c.push((cname(evento), quem, 1, 0));
                } else {
                    c.retain(|e| e.1 != quem);
                }
            }
            atualizar_eventos_de_entidade();
        };
        let chave = |v: bool| crate::interface::WIDGET_SPAWN_EVENT_ENABLED.store(v, Ordering::Relaxed);
        let estado = || (evento_ligado(), crate::interface::WIDGET_SPAWN_EVENT_ENABLED.load(Ordering::Relaxed));
        ouvinte("InkWidget/Spawn", alvo, true);
        let com_ouvinte = estado();
        chave(false);
        ouvinte("Entity/Assemble", outro, true);
        let desligada_e_outro_registro = estado();
        chave(true);
        let religada = estado();
        ouvinte("InkWidget/Spawn", alvo, false);
        let sem_ouvinte = estado();
        ouvinte("Entity/Assemble", outro, false);
        let sem_ouvinte_e_outra_remocao = estado();
        crate::interface::WIDGET_SPAWN_ARMADO.store(armado_antes, Ordering::Relaxed);
        chave(chave_antes);
        assert!(chave_antes, "a chave nasce ligada");
        assert_eq!(com_ouvinte, (true, true), "o registro sozinho liga");
        assert_eq!(desligada_e_outro_registro, (false, false));
        assert_eq!(religada, (true, true));
        assert_eq!(sem_ouvinte, (false, true));
        assert_eq!(sem_ouvinte_e_outra_remocao, (false, true));
    }

    // A tabela da entrega: vazia até o 1º nome; o nome acha o controller; o nome repetido fica com o controller gravado; cheia, recusa o nome novo e
    // guarda os que tem.
    #[test]
    fn nomes_da_entrega_registram_acham_e_param_no_teto() {
        let t: NomesDaEntrega<2> = NomesDaEntrega::nova();
        assert!(t.vazia());
        assert_eq!(t.controller(1), None);
        assert!(t.registrar(1, 0x10));
        assert!(!t.vazia());
        assert_eq!(t.controller(1), Some(0x10));
        assert!(t.registrar(1, 0x99), "nome repetido");
        assert_eq!(t.controller(1), Some(0x10));
        assert!(t.registrar(2, 0x20));
        assert!(!t.registrar(3, 0x30), "cheia");
        assert_eq!((t.controller(1), t.controller(2), t.controller(3)), (Some(0x10), Some(0x20), None));
    }

    /// `(caminho, item, flag, nome inteiro já na tabela da entrega)` que a original falsa do assíncrono recebeu, na ordem.
    static PEDIDOS_ASSINCRONOS: Mutex<Vec<(u64, u64, u64, bool)>> = Mutex::new(Vec::new());
    const NOME_ASSINCRONO: &str = "Root:TesteControllerAssincrono";

    /// Original falsa dos dois `AsyncSpawnFrom*`: acha só o item `Root` (devolve 1), como a biblioteca sem `Root:Controller`.
    fn assincrono_falso(caminho: u64, item: u64, flag: u64) -> u32 {
        let na_tabela = NOMES_DA_ENTREGA.controller(cname(NOME_ASSINCRONO)).is_some();
        PEDIDOS_ASSINCRONOS.lock().unwrap_or_else(|e| e.into_inner()).push((caminho, item, flag, na_tabela));
        u32::from(item == cname("Root"))
    }

    unsafe extern "C" fn async_local_falso(_lib: *mut c_void, _info: *mut c_void, item: u64, flag: u64) -> u32 {
        assincrono_falso(0, item, flag)
    }

    unsafe extern "C" fn async_externo_falso(_lib: *mut c_void, _info: *mut c_void, caminho: u64, item: u64, flag: u64) -> u32 {
        assincrono_falso(caminho, item, flag)
    }

    // O assíncrono pela entrada: o achado pelo nome inteiro passa com 1 pedido; sem o hook da entrega, `Item:Controller` sem item fica com o 0 da
    // original (refeito, nasceria com o controller do pacote); com ele, refaz pelo `Item` com o nome inteiro já na tabela da entrega no 2º pedido (a
    // entrega roda dentro dele); `Item:` refaz sem pôr nome; nome sem a forma e `:Controller` ficam com o 0. Caminho e flag passam inteiros. Com a
    // injeção recusada, a biblioteca falsa (fora da memória) não é lida. Nomes do espelho (`intern`): o pool do motor não existe no teste.
    #[test]
    fn hook_assincrono_refaz_pelo_item_e_guarda_o_nome() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(|e| e.into_inner());
        let externas_antes = EXTERNAS_OFF.swap(EXTERNAS_RECUSADO, Ordering::Relaxed);
        let root = cname("Root");
        let (com_controller, vazio, sem_forma, item_vazio) = (
            crate::cname::intern(NOME_ASSINCRONO),
            crate::cname::intern("Root:"),
            crate::cname::intern("TesteSemFormaAssincrono"),
            crate::cname::intern(":TesteControllerAssincrono"),
        );
        let (lib, info) = (0x7e57_6000_0000usize as *mut c_void, 0x7e57_7000_0000usize as *mut c_void);
        let antes = (
            ORIG_ASYNC_SPAWN_FROM_LOCAL.swap(async_local_falso as *const () as usize as u64, Ordering::Relaxed),
            ORIG_ASYNC_SPAWN_FROM_EXTERNAL.swap(async_externo_falso as *const () as usize as u64, Ordering::Relaxed),
        );
        let pelo = |f: &dyn Fn() -> u32| {
            PEDIDOS_ASSINCRONOS.lock().unwrap_or_else(|e| e.into_inner()).clear();
            let r = f();
            (r, PEDIDOS_ASSINCRONOS.lock().unwrap_or_else(|e| e.into_inner()).clone())
        };
        let entrega_antes = ORIG_ENTREGA_DO_SPAWN_ASSINCRONO.swap(0, Ordering::Relaxed);
        let sem_entrega = unsafe {
            [pelo(&|| async_spawn_local(lib, info, com_controller, 0x5a)), pelo(&|| async_spawn_externo(lib, info, 0x1234, com_controller, 0x5a))]
        };
        ORIG_ENTREGA_DO_SPAWN_ASSINCRONO.store(entrega_falsa as *const () as usize as u64, Ordering::Relaxed);
        let lidos = unsafe {
            [
                pelo(&|| async_spawn_local(lib, info, root, 0x5a)),
                pelo(&|| async_spawn_local(lib, info, vazio, 0x5a)),
                pelo(&|| async_spawn_local(lib, info, sem_forma, 0x5a)),
                pelo(&|| async_spawn_local(lib, info, item_vazio, 0x5a)),
                pelo(&|| async_spawn_externo(lib, info, 0x1234, com_controller, 0x5a)),
                pelo(&|| async_spawn_local(lib, info, com_controller, 0x5a)),
                pelo(&|| async_spawn_externo(lib, info, 0x1234, sem_forma, 0x5a)),
            ]
        };
        ORIG_ASYNC_SPAWN_FROM_LOCAL.store(antes.0, Ordering::Relaxed);
        ORIG_ASYNC_SPAWN_FROM_EXTERNAL.store(antes.1, Ordering::Relaxed);
        ORIG_ENTREGA_DO_SPAWN_ASSINCRONO.store(entrega_antes, Ordering::Relaxed);
        EXTERNAS_OFF.store(externas_antes, Ordering::Relaxed);
        assert_eq!(sem_entrega[0], (0, vec![(0, com_controller, 0x5a, false)]), "sem o hook da entrega: não refaz");
        assert_eq!(sem_entrega[1], (0, vec![(0x1234, com_controller, 0x5a, false)]));
        assert_eq!(lidos[0], (1, vec![(0, root, 0x5a, false)]));
        assert_eq!(lidos[1], (1, vec![(0, vazio, 0x5a, false), (0, root, 0x5a, false)]), "controller vazio: refaz sem pôr nome");
        assert_eq!(lidos[2], (0, vec![(0, sem_forma, 0x5a, false)]));
        assert_eq!(lidos[3], (0, vec![(0, item_vazio, 0x5a, false)]));
        assert_eq!(lidos[4], (1, vec![(0x1234, com_controller, 0x5a, false), (0x1234, root, 0x5a, true)]), "o nome entra antes do 2º pedido");
        assert_eq!(lidos[5], (1, vec![(0, com_controller, 0x5a, true), (0, root, 0x5a, true)]), "nome já na tabela");
        assert_eq!(lidos[6], (0, vec![(0x1234, sem_forma, 0x5a, true)]));
        assert_eq!(NOMES_DA_ENTREGA.controller(com_controller), Some(cname("TesteControllerAssincrono")));
        assert_eq!(NOMES_DA_ENTREGA.controller(vazio), None);
    }

    // A decisão da entrega pelo nome do pedido (com a tabela do teste: a troca em si precisa do RTTI do jogo): o nome da tabela dá a instância e o
    // controller; nome fora da tabela, instância nula, callable ou pedido nulos não dão troca.
    #[test]
    fn entrega_troca_so_o_nome_da_tabela() {
        let t: NomesDaEntrega<4> = NomesDaEntrega::nova();
        let (nome, outro) = (0xaaaa_u64, 0xbbbb_u64);
        assert!(t.registrar(nome, 0xc0c0));
        let mut pedido = [0u64; 12];
        let mut entrega = [0u64; 5];
        entrega[ENTREGA_PEDIDO / 8] = pedido.as_mut_ptr() as u64;
        let inst = [0x7e57_8000_0000u64, 0x7e57_9000_0000];
        let e = entrega.as_mut_ptr() as *const u8;
        unsafe {
            (pedido.as_mut_ptr() as *mut u8).add(PEDIDO_NOME).cast::<u64>().write(nome);
            assert_eq!(troca_da_entrega(e, &inst, &t), Some((inst[0] as *mut u8, nome, 0xc0c0)));
            assert_eq!(troca_da_entrega(e, &[0, 0], &t), None, "instância nula");
            assert_eq!(troca_da_entrega(e, std::ptr::null(), &t), None);
            assert_eq!(troca_da_entrega(std::ptr::null(), &inst, &t), None);
            (pedido.as_mut_ptr() as *mut u8).add(PEDIDO_NOME).cast::<u64>().write(outro);
            assert_eq!(troca_da_entrega(e, &inst, &t), None, "nome fora da tabela");
            (entrega.as_mut_ptr() as *mut u8).add(ENTREGA_PEDIDO).cast::<u64>().write(0);
            assert_eq!(troca_da_entrega(e, &inst, &t), None, "pedido nulo");
        }
    }

    /// `(callable, instância)` que a entrega falsa recebeu.
    static ENTREGAS: Mutex<Vec<(u64, [u64; 2])>> = Mutex::new(Vec::new());

    unsafe extern "C" fn entrega_falsa(entrega: *mut c_void, instancia: *mut [u64; 2]) {
        ENTREGAS.lock().unwrap_or_else(|e| e.into_inner()).push((entrega as u64, *instancia));
    }

    // A entrega pela entrada, com a tabela global não vazia e o nome do pedido fora dela (todo spawn do jogo depois que um mod pediu
    // `Item:Controller`): a original recebe os mesmos argumentos, uma vez, e a instância não muda.
    #[test]
    fn entrega_com_o_nome_fora_da_tabela_so_chama_a_original() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(|e| e.into_inner());
        assert!(NOMES_DA_ENTREGA.registrar(cname("TesteEntregaNaTabela:Controller"), cname("Controller")));
        let mut pedido = [0u64; 12];
        pedido[PEDIDO_NOME / 8] = cname("TesteEntregaForaDaTabela:Controller");
        let mut entrega = [0u64; 5];
        entrega[ENTREGA_PEDIDO / 8] = pedido.as_mut_ptr() as u64;
        let mut inst = [0x7e57_8000_0000u64, 0x7e57_9000_0000];
        let antes = ORIG_ENTREGA_DO_SPAWN_ASSINCRONO.swap(entrega_falsa as *const () as usize as u64, Ordering::Relaxed);
        ENTREGAS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        unsafe { antes_da_entrega(entrega.as_mut_ptr() as *mut c_void, &mut inst) };
        ORIG_ENTREGA_DO_SPAWN_ASSINCRONO.store(antes, Ordering::Relaxed);
        assert!(!NOMES_DA_ENTREGA.vazia());
        assert_eq!(*ENTREGAS.lock().unwrap_or_else(|e| e.into_inner()), vec![(entrega.as_ptr() as u64, [0x7e57_8000_0000, 0x7e57_9000_0000])]);
        assert_eq!(inst, [0x7e57_8000_0000, 0x7e57_9000_0000]);
    }

    /// O que os hooks do assíncrono somam a cada spawn achado pelo nome (o caso de todo spawn do jogo) e o que a entrega soma sem troca, com a tabela
    /// vazia e com 8 nomes (o nome do pedido fora dela): ns por chamada pelo hook menos a original falsa chamada direto. No `AsyncSpawnFromExternal`, a
    /// biblioteca externa na 4ª de 4 referências (a trava de leitura e a busca da injeção). Mediana de 5 rodadas de 10M. Microbenchmark offline: fica
    /// de fora o salto do patch e o trampolim com o prólogo relocado. `cargo test --release --lib -- --ignored custo_do_assincrono_por_spawn --nocapture`.
    #[test]
    #[ignore]
    fn custo_do_assincrono_por_spawn() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(|e| e.into_inner());
        const CHAMADAS: usize = 10_000_000;
        unsafe extern "C" fn achado_local(_lib: *mut c_void, _info: *mut c_void, _item: u64, _flag: u64) -> u32 {
            1
        }
        unsafe extern "C" fn achado_externo(_lib: *mut c_void, _info: *mut c_void, _caminho: u64, _item: u64, _flag: u64) -> u32 {
            1
        }
        unsafe extern "C" fn entrega_vazia(_entrega: *mut c_void, _instancia: *mut [u64; 2]) {}
        let caminho = 0xdddd_u64;
        let mut entradas = [0xaaaa_u64, 0, 0, 0xbbbb, 0, 0, 0xcccc, 0, 0, caminho, 0, 0];
        let mut lib = biblioteca_falsa(&mut entradas, 4, 4);
        let lib_ptr = lib.as_mut_ptr() as *mut c_void;
        let mut pedido = [0u64; 12];
        pedido[PEDIDO_NOME / 8] = cname("TesteCustoForaDaTabela:Controller");
        let mut callable = [0u64; 5];
        callable[ENTREGA_PEDIDO / 8] = pedido.as_mut_ptr() as u64;
        let callable_ptr = callable.as_mut_ptr() as *mut c_void;
        let antes = (
            ORIG_ASYNC_SPAWN_FROM_LOCAL.swap(achado_local as *const () as usize as u64, Ordering::Relaxed),
            ORIG_ASYNC_SPAWN_FROM_EXTERNAL.swap(achado_externo as *const () as usize as u64, Ordering::Relaxed),
            ORIG_ENTREGA_DO_SPAWN_ASSINCRONO.swap(entrega_vazia as *const () as usize as u64, Ordering::Relaxed),
        );
        let externas_antes = EXTERNAS_OFF.swap(EXTERNAS_FALSO, Ordering::Relaxed);
        let medir = |f: &dyn Fn()| {
            let mut rodadas = [0f64; 5];
            for r in rodadas.iter_mut() {
                let inicio = std::time::Instant::now();
                for _ in 0..CHAMADAS {
                    f();
                }
                *r = inicio.elapsed().as_nanos() as f64 / CHAMADAS as f64;
            }
            rodadas.sort_by(f64::total_cmp);
            rodadas
        };
        let (l, e, f): (AsyncLocal, AsyncExterno, Entrega) = (achado_local, achado_externo, entrega_vazia);
        let (hl, he, hf): (AsyncLocal, AsyncExterno, Entrega) = (async_spawn_local, async_spawn_externo, antes_da_entrega);
        let mut inst = [INSTANCIA_FALSA, 0];
        let ip = &mut inst as *mut [u64; 2];
        let local = unsafe {
            (
                medir(&|| { let _ = std::hint::black_box(std::hint::black_box(l)(lib_ptr, lib_ptr, 2, 0)); }),
                medir(&|| { let _ = std::hint::black_box(std::hint::black_box(hl)(lib_ptr, lib_ptr, 2, 0)); }),
            )
        };
        let externo = unsafe {
            (
                medir(&|| { let _ = std::hint::black_box(std::hint::black_box(e)(lib_ptr, lib_ptr, caminho, 2, 0)); }),
                medir(&|| { let _ = std::hint::black_box(std::hint::black_box(he)(lib_ptr, lib_ptr, caminho, 2, 0)); }),
            )
        };
        let nomes_antes = NOMES_DA_ENTREGA.usados.load(Ordering::Relaxed);
        let entrega_vazia_ns =
            unsafe { (medir(&|| std::hint::black_box(f)(callable_ptr, ip)), medir(&|| std::hint::black_box(hf)(callable_ptr, ip))) };
        for i in 0..8u64 {
            NOMES_DA_ENTREGA.registrar(cname("TesteCusto:Controller") ^ (i + 1), i + 1);
        }
        let entrega_com_nomes =
            unsafe { (medir(&|| std::hint::black_box(f)(callable_ptr, ip)), medir(&|| std::hint::black_box(hf)(callable_ptr, ip))) };
        ORIG_ASYNC_SPAWN_FROM_LOCAL.store(antes.0, Ordering::Relaxed);
        ORIG_ASYNC_SPAWN_FROM_EXTERNAL.store(antes.1, Ordering::Relaxed);
        ORIG_ENTREGA_DO_SPAWN_ASSINCRONO.store(antes.2, Ordering::Relaxed);
        EXTERNAS_OFF.store(externas_antes, Ordering::Relaxed);
        let rotulo_vazia = format!("entrega, tabela com {nomes_antes} nome(s)");
        let rotulo_cheia = format!("entrega, tabela com {} nome(s)", NOMES_DA_ENTREGA.usados.load(Ordering::Relaxed));
        for (nome, (direto, pelo_hook)) in [
            ("AsyncSpawnFromLocal", local),
            ("AsyncSpawnFromExternal", externo),
            (rotulo_vazia.as_str(), entrega_vazia_ns),
            (rotulo_cheia.as_str(), entrega_com_nomes),
        ] {
            println!(
                "[spawn] {nome}: original direta {:.2} ns, pelo hook {:.2} ns: +{:.2} ns por spawn (medianas de 5; hook min {:.2} max {:.2})",
                direto[2],
                pelo_hook[2],
                pelo_hook[2] - direto[2],
                pelo_hook[0],
                pelo_hook[4]
            );
        }
        assert_eq!(inst, [INSTANCIA_FALSA, 0], "nome fora da tabela: a instância não muda");
        assert_eq!(lib[EXTERNAS_FALSO as usize / 8 + 1], 4 | 4 << 32, "a lista com a dependência não cresce");
    }

    /// O que o hook soma a cada spawn achado, sem ouvinte e sem `Item:Controller` (o caso de todo spawn do jogo): ns por chamada pela entrada hookada
    /// menos a original falsa chamada direto. No `SpawnFromExternal`, com a biblioteca externa na 4ª de 4 referências da lista de quem chama (a trava de
    /// leitura e a busca da injeção de dependência). Mediana de 5 rodadas de 10M. Microbenchmark offline: fica de fora o salto do patch (`ldr x17` +
    /// `br x17`) e o trampolim com o prólogo relocado. `cargo test --release --lib -- --ignored custo_do_hook_por_spawn --nocapture`.
    #[test]
    #[ignore]
    fn custo_do_hook_por_spawn() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(|e| e.into_inner());
        const CHAMADAS: usize = 10_000_000;
        unsafe extern "C" fn achado_local_alvo(_lib: *mut c_void, _item: u64, saida: *mut [u64; 2]) {
            *saida = [INSTANCIA_FALSA, 0];
        }
        #[unsafe(naked)]
        unsafe extern "C" fn achado_local_tramp() {
            core::arch::naked_asm!("mov x2, x8", "b {f}", f = sym achado_local_alvo)
        }
        unsafe extern "C" fn achado_externo_alvo(_lib: *mut c_void, _caminho: u64, _item: u64, saida: *mut [u64; 2]) {
            *saida = [INSTANCIA_FALSA, 0];
        }
        #[unsafe(naked)]
        unsafe extern "C" fn achado_externo_tramp() {
            core::arch::naked_asm!("mov x3, x8", "b {f}", f = sym achado_externo_alvo)
        }
        let caminho = 0xdddd_u64;
        let mut entradas = [0xaaaa_u64, 0, 0, 0xbbbb, 0, 0, 0xcccc, 0, 0, caminho, 0, 0];
        let mut lib = biblioteca_falsa(&mut entradas, 4, 4);
        let lib_ptr = lib.as_mut_ptr() as u64;
        let antes = (publicar(&ORIG_SPAWN_FROM_LOCAL, achado_local_tramp), publicar(&ORIG_SPAWN_FROM_EXTERNAL, achado_externo_tramp));
        let ligado_antes = crate::interface::WIDGET_SPAWN_ARMADO.swap(false, Ordering::Relaxed);
        let externas_antes = EXTERNAS_OFF.swap(EXTERNAS_FALSO, Ordering::Relaxed);
        let medir = |alvo: unsafe extern "C" fn(), args: [u64; 4]| {
            let mut rodadas = [0f64; 5];
            for r in rodadas.iter_mut() {
                let mut saida = [0u64; 2];
                let inicio = std::time::Instant::now();
                for _ in 0..CHAMADAS {
                    unsafe { sret_chamar(std::hint::black_box(alvo as *mut c_void), args, saida.as_mut_ptr() as *mut c_void) };
                }
                std::hint::black_box(saida);
                *r = inicio.elapsed().as_nanos() as f64 / CHAMADAS as f64;
            }
            rodadas.sort_by(f64::total_cmp);
            rodadas
        };
        let local = (medir(achado_local_tramp, [1, 2, 0, 0]), medir(spawn_local_tramp, [1, 2, 0, 0]));
        let externo = (medir(achado_externo_tramp, [lib_ptr, caminho, 2, 0]), medir(spawn_externo_tramp, [lib_ptr, caminho, 2, 0]));
        ORIG_SPAWN_FROM_LOCAL.store(antes.0, Ordering::Relaxed);
        ORIG_SPAWN_FROM_EXTERNAL.store(antes.1, Ordering::Relaxed);
        crate::interface::WIDGET_SPAWN_ARMADO.store(ligado_antes, Ordering::Relaxed);
        EXTERNAS_OFF.store(externas_antes, Ordering::Relaxed);
        for (nome, (direto, pelo_hook)) in [("SpawnFromLocal", local), ("SpawnFromExternal", externo)] {
            println!(
                "[spawn] {nome}: original direta {:.2} ns, pelo hook {:.2} ns: +{:.2} ns por spawn (medianas de 5; hook min {:.2} max {:.2})",
                direto[2],
                pelo_hook[2],
                pelo_hook[2] - direto[2],
                pelo_hook[0],
                pelo_hook[4]
            );
        }
        assert_eq!(lib[EXTERNAS_FALSO as usize / 8 + 1], 4 | 4 << 32, "a lista com a dependência não cresce");
    }
}
