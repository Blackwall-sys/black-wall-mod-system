//! Os eventos `Session/*` que o próprio jogo dispara no `game::CallbackSystem` dele, repassados ao despacho do BWMS. Com a família do
//! `CallbackSystem` religada ao BWMS (o `RegisterCallback` de script cai no registro do BWMS), o mapa de handlers do jogo fica sem os ouvintes
//! dos mods e o evento do jogo não chegava a ninguém.
//!
//! RE estática do Steam 2.5.0 (`relatorios/A.md`): cada virtual de `IGameSystem` da vtable `0x107cfbcb8` do `game::CallbackSystem` chama o
//! despachante único `0x105046c90(sys, CName do evento, &sys+0x49 pregame, &sys+0x48 restored)`; o binário inteiro tem só esses 9 `b`/`bl` para
//! ele. O despachante procura o nome no mapa `sys+0x50` (contagem em `sys+0x58`) e, sem handler, volta antes de criar o evento; com handler,
//! cria o `GameSessionEvent` de 0x50 B (`eventName` em +0x40, `IsPreGame` em +0x48 lido de `*x2`, `IsRestored` em +0x49 lido de `*x3`). O gancho
//! fica na entrada, então vê os 9 eventos mesmo com o mapa do jogo vazio, e repassa ao BWMS pelo mesmo `GameSessionEvent` do despacho dele
//! (`natives::disparar_evento_de_sessao`).
//!
//! Uma fonte por evento e por sessão. `Session/BeforeStart`, `Session/Start`, `Session/Ready` e `Session/End` têm fonte própria do BWMS (as
//! reservas: sonda de dev do `MappinSystem`, `Session/Start` atrasado no `cp77_tick`, `Session/Ready` da carga de save no spawn do jogador,
//! `Session/End` no despawn do jogador, no spawn do jogador em outro mundo e no `exit()`; o `Session/Ready` do jogo novo e do menu sai do
//! `natives::session_ready_tick`). Como na referência, cada mundo é uma sessão: o do menu (`IsPreGame`) tem `Session/Ready` e `Session/End`
//! próprios antes dos da carga de save. Dois registros decidem:
//! - `provado`: o despachante do jogo já entregou aquele evento neste processo. A reserva dele desliga só aí, não na instalação do gancho: em 15
//!   boots Steam 2.5 com o gancho posto o despachante não foi chamado nenhuma vez (`relatorios/L12.md`).
//! - `entregue`: o evento já saiu nesta sessão, por qualquer fonte. Cada fonte o reivindica com um `fetch_or` atômico antes de despachar: só a
//!   1ª recebe o bit livre, a outra não despacha, venha de que thread vier. A sessão abre no `Session/BeforeStart` do jogo, no spawn do jogador
//!   quando o jogo não abre (`abrir_sessao_no_spawn`) ou, depois de um `Session/End`, no próximo `Session/Start`/presença do jogador.
//!   Os outros 5 (`BeforeEnd`, `BeforeSave`, `AfterSave`, `Pause`, `Resume`) só o jogo dispara e não passam por eles.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// `0x105046c90`: o despachante dos `Session/*` do `game::CallbackSystem`, com o 1º u32 do prólogo (`sub sp, sp, #0x70`).
pub(crate) const DESPACHANTE: (u64, u32) = (0x1_0504_6c90, 0xd101_c3ff);

/// Os nomes que o despachante recebe, na ordem dos slots da vtable (`+0x118` `OnWorldAttached` … `+0x170` `OnGameResumed`). O CName de cada um é
/// o imediato do `mov`/`movk` do chamador (`0x105046ad8`, `0x105046f34`, `0x105046f50`, `0x1050472d8`, `0x105047308`, `0x105047330`,
/// `0x105047360`, `0x10504737c`, `0x105047398`), conferido no teste contra o FNV-1a 64 do nome.
pub(crate) const EVENTOS: [&str; 9] = [
    INICIO_ANTES,
    "Session/BeforeEnd",
    FIM,
    "Session/BeforeSave",
    "Session/AfterSave",
    PRONTO,
    INICIO,
    "Session/Pause",
    "Session/Resume",
];

const INICIO_ANTES: &str = "Session/BeforeStart";
const INICIO: &str = "Session/Start";
const PRONTO: &str = "Session/Ready";
const FIM: &str = "Session/End";

/// Os 4 eventos com reserva do BWMS, como bits de `provado`/`entregue`.
pub(crate) const ANTES_DO_INICIO: u32 = 1 << 0;
pub(crate) const INICIO_DA_SESSAO: u32 = 1 << 1;
pub(crate) const SESSAO_PRONTA: u32 = 1 << 2;
pub(crate) const FIM_DA_SESSAO: u32 = 1 << 3;
/// O `Session/BeforeStart` desta sessão saiu pela reserva: o do jogo no mesmo mundo é repetição. Só a sonda de dev do `MappinSystem` liga.
const ANTES_PELA_RESERVA: u32 = 1 << 4;
/// O objeto principal do jogador já nasceu nesta sessão: o próximo spawn ou `OnWorldAttached` é de outro mundo. Só as reservas ligam.
const JOGADOR_NA_SESSAO: u32 = 1 << 5;

/// O trampolim do despachante.
static ORIGINAL: AtomicU64 = AtomicU64::new(0);

/// Estado das fontes dos `Session/*`. Um por processo (`SESSAO`); os testes montam o próprio.
pub(crate) struct FontesDaSessao {
    /// Eventos que o despachante do jogo já entregou neste processo: a reserva deles fica desligada.
    provado: AtomicU32,
    /// Eventos já entregues nesta sessão, por qualquer fonte.
    entregue: AtomicU32,
}

static SESSAO: FontesDaSessao = FontesDaSessao::new();

impl FontesDaSessao {
    const fn new() -> Self {
        Self { provado: AtomicU32::new(0), entregue: AtomicU32::new(0) }
    }

    fn reserva_ligada(&self, evento: u32) -> bool {
        self.provado.load(Ordering::Acquire) & evento == 0
    }

    /// Verdadeiro só para a 1ª fonte que entrega `evento` nesta sessão.
    fn reivindicar(&self, evento: u32) -> bool {
        self.entregue.fetch_or(evento, Ordering::AcqRel) & evento == 0
    }

    /// Sessão fechada (o `Session/End` dela já saiu) vira sessão nova.
    fn abrir_se_fechada(&self) {
        let _ = self.entregue.fetch_update(Ordering::AcqRel, Ordering::Acquire, |e| (e & FIM_DA_SESSAO != 0).then_some(0));
    }

    /// A sessão `e` é de um mundo que já teve o jogador e saiu sem `Session/End`: o `End` dela fica com quem abre a seguinte.
    fn fim_pendente(&self, e: u32) -> bool {
        e & JOGADOR_NA_SESSAO != 0 && e & FIM_DA_SESSAO == 0 && self.reserva_ligada(FIM_DA_SESSAO)
    }

    /// Spawn do jogador: abre a sessão quando o jogo não abre (o `Session/BeforeStart` dele nunca chegou) ou quando ela já fechou. Sem o
    /// jogo, o spawn num mundo cuja sessão já teve jogador é outro mundo (menu → carga de save, sem despawn visto no tick): verdadeiro quando o
    /// `Session/End` da anterior não saiu e quem chama o entrega antes do `Session/Ready` desta. O `BeforeStart` da reserva deste mundo fica.
    fn abrir_no_spawn(&self) -> bool {
        if !self.reserva_ligada(ANTES_DO_INICIO) {
            self.abrir_se_fechada();
            return false;
        }
        let anterior = self
            .entregue
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |e| {
                Some(if e & (JOGADOR_NA_SESSAO | FIM_DA_SESSAO) != 0 { JOGADOR_NA_SESSAO } else { e | JOGADOR_NA_SESSAO })
            })
            .unwrap_or(0);
        self.fim_pendente(anterior)
    }

    /// `Session/BeforeStart` da reserva: abre a sessão e entrega, salvo com o deste mundo já entregue (aberta, sem jogador e sem
    /// `Session/End`). `Some(true)`: a sessão anterior teve jogador e saiu sem `End`, que quem chama entrega antes do `BeforeStart`.
    fn antes_do_inicio_pela_reserva(&self) -> Option<bool> {
        if !self.reserva_ligada(ANTES_DO_INICIO) {
            return None;
        }
        self.entregue
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |e| {
                let aberta = e & ANTES_DO_INICIO != 0 && e & (FIM_DA_SESSAO | JOGADOR_NA_SESSAO) == 0;
                (!aberta).then_some(ANTES_DO_INICIO | ANTES_PELA_RESERVA)
            })
            .ok()
            .map(|anterior| self.fim_pendente(anterior))
    }

    /// O despachante do jogo entregou `nome`: liga `provado` e diz se esta é a fonte da sessão. O `Session/BeforeStart` do jogo abre a sessão
    /// sempre (é o `OnWorldAttached` do mundo novo) e só cede ao da reserva no mesmo mundo; o `Session/Start` abre a sessão já fechada.
    fn do_jogo(&self, nome: &str) -> bool {
        let evento = evento_com_reserva(nome);
        if evento == 0 {
            return true;
        }
        self.provado.fetch_or(evento, Ordering::AcqRel);
        match nome {
            INICIO_ANTES => self.entregue.swap(ANTES_DO_INICIO, Ordering::AcqRel) & ANTES_PELA_RESERVA == 0,
            INICIO => {
                self.abrir_se_fechada();
                self.reivindicar(evento)
            }
            _ => self.reivindicar(evento),
        }
    }
}

fn evento_com_reserva(nome: &str) -> u32 {
    match nome {
        INICIO_ANTES => ANTES_DO_INICIO,
        INICIO => INICIO_DA_SESSAO,
        PRONTO => SESSAO_PRONTA,
        FIM => FIM_DA_SESSAO,
        _ => 0,
    }
}

/// A reserva do BWMS para `evento` segue ligada: o despachante do jogo ainda não o entregou neste processo.
pub(crate) fn reserva_ligada(evento: u32) -> bool {
    SESSAO.reserva_ligada(evento)
}

/// Verdadeiro só para a 1ª fonte que entrega `evento` nesta sessão (jogo, reserva ou `session_ready_tick`).
pub(crate) fn reivindicar(evento: u32) -> bool {
    SESSAO.reivindicar(evento)
}

/// Reserva de `evento` (`nome`): entrega só com ela ligada, com ouvinte e sem outra fonte nesta sessão. Sem ouvinte não reivindica.
pub(crate) fn pela_reserva(evento: u32, nome: &str) -> bool {
    SESSAO.reserva_ligada(evento) && crate::tipos_do_motor::tem_ouvinte(nome) && SESSAO.reivindicar(evento)
}

/// `Session/BeforeStart` da reserva (sonda de dev do `MappinSystem`): `Some` quando ela é a fonte deste mundo, com `true` quando o
/// `Session/End` do mundo anterior sai antes.
pub(crate) fn antes_do_inicio_pela_reserva() -> Option<bool> {
    SESSAO.antes_do_inicio_pela_reserva()
}

/// O `Session/End` desta sessão: verdadeiro só na 1ª entrega, de qualquer fonte.
pub(crate) fn marcar_fim() -> bool {
    SESSAO.reivindicar(FIM_DA_SESSAO)
}

/// Spawn do objeto principal do jogador (`boot::player_main_object_spawned_hook`), antes do `session_ready_tick` consumir o spawn. Verdadeiro
/// quando o `Session/End` da sessão anterior não saiu e fica com o `session_ready_tick`, antes do `Session/Ready` deste spawn.
pub(crate) fn abrir_sessao_no_spawn() -> bool {
    SESSAO.abrir_no_spawn()
}

/// Presença do jogador no `cp77_tick`: abre a sessão só se a anterior já fechou (sem o hook do spawn no ar, é o que abre).
pub(crate) fn abrir_sessao_na_presenca() {
    SESSAO.abrir_se_fechada();
}

/// O nome do evento pelo CName que o jogo passa em `x1`.
fn nome_do_evento(hash: u64) -> Option<&'static str> {
    EVENTOS.iter().copied().find(|n| crate::cname::cname(n) == hash)
}

/// Gancho no despachante, no `on_load`. Só com o prólogo esperado; fora do Steam 2.5.0 o `rebase` só traduz o endereço com ele na tabela de
/// assinaturas. Instalado não liga nada: as reservas desligam evento a evento na 1ª entrega do jogo (`FontesDaSessao::do_jogo`).
pub(crate) unsafe fn instalar() {
    let r = crate::ganchos::instalar_conferindo_prologo(DESPACHANTE.0, DESPACHANTE.1, despacho_do_jogo as *mut c_void, &ORIGINAL);
    crate::log_dev!("[sessao-jogo] gancho no despachante do CallbackSystem do jogo ({:#x}): {r:?}", DESPACHANTE.0);
    #[cfg(feature = "devtools")]
    sonda::instalar();
}

/// `0x105046c90(sys, evento, pregame, restored) -> w0`. As flags são lidas antes da original (ela só as lê); o retorno dela volta intacto: os
/// slots `+0x138` (`OnBeforeGameSave`) e `+0x158` (`OnGameRestored`) devolvem bool e os outros fazem `b` direto para cá.
unsafe extern "C" fn despacho_do_jogo(sys: *mut c_void, evento: u64, pregame: *const u8, restored: *const u8) -> u64 {
    #[cfg(feature = "devtools")]
    sonda::ENTRADAS.fetch_add(1, Ordering::Relaxed);
    let ler = |p: *const u8| !p.is_null() && p.read() != 0;
    let (pg, rs) = (ler(pregame), ler(restored));
    let original: unsafe extern "C" fn(*mut c_void, u64, *const u8, *const u8) -> u64 = std::mem::transmute(ORIGINAL.load(Ordering::Relaxed));
    let ret = original(sys, evento, pregame, restored);
    match nome_do_evento(evento) {
        Some(nome) if SESSAO.do_jogo(nome) => {
            let n = repassar(nome, pg, rs);
            crate::veredito_dev!("[sessao-jogo] {nome} pregame={pg} restored={rs} → {n} callback(s) ({})", retrato(sys));
        }
        Some(nome) => crate::veredito_dev!(
            "[sessao-jogo] {nome} pregame={pg} restored={rs} → já entregue nesta sessão por outra fonte, não repassado ({})",
            retrato(sys)
        ),
        None => crate::veredito_dev!("[sessao-jogo] evento {evento:#x} fora dos 9 Session/* — não repassado"),
    }
    ret
}

/// Entrega `nome` aos ouvintes do BWMS com um `GameSessionEvent` (`pregame`, `restored`); devolve quantos receberam. Sem ouvinte não constrói o
/// evento, salvo o `Session/End`, que sempre passa pelo despacho: a varredura do `CallbackLifetime.Session` roda nele mesmo sem
/// ouvinte do próprio `End`.
pub(crate) unsafe fn repassar(nome: &'static str, pregame: bool, restored: bool) -> usize {
    if crate::tipos_do_motor::tem_ouvinte(nome) {
        crate::inicio::natives::disparar_evento_de_sessao(nome, pregame, restored)
    } else if nome == FIM {
        crate::tipos_do_motor::fire_event_args(FIM, &[])
    } else {
        0
    }
}

/// `game::CallbackSystem::s_self` (`Handle`, instância em +0): o singleton que o `GameInstance.GetCallbackSystem` do jogo devolve
/// (`0x1016a4f00` lê o `GetInstance` `0x105046acc`). Nasce em `0x105046150` pela classe estática (`[0x109d23378]`, `TTypedClass` em
/// `0x109890ab0`), nunca pelo nome de script.
const S_SELF: u64 = 0x1_07d9_a490;
/// vtable do `game::CallbackSystem` (ctor `0x105045ce4`).
#[cfg(feature = "devtools")]
const VTABLE: u64 = 0x1_07cf_bcb8;

unsafe fn ler_u64(p: *const u8) -> Option<u64> {
    crate::memoria::is_readable(p as *const c_void, 8).then(|| (p as *const u64).read_unaligned())
}

/// Instância do `s_self` do jogo, 0 sem ela.
unsafe fn instancia_do_s_self() -> u64 {
    let h = crate::rebase(S_SELF) as *const u8;
    if h.is_null() { 0 } else { ler_u64(h).unwrap_or(0) }
}

/// Texto do veredito: o sistema que chamou é o `s_self` e quantos handlers o mapa dele tem (`sys+0x58`, o `cbz` que o despachante testa).
unsafe fn retrato(sys: *mut c_void) -> String {
    let s = sys as *const u8;
    let mapa = if crate::memoria::is_readable(s.add(0x58) as *const c_void, 4) { (s.add(0x58) as *const u32).read_unaligned() as i64 } else { -1 };
    format!("sys={:#x} s_self={} mapa={mapa}", sys as u64, sys as u64 == instancia_do_s_self())
}

/// Sondas de dev que separam as causas de o despachante não rodar, num boot:
/// - H1: o `GameInstance` cria os sistemas pelas linhas do `base\systems\game_systems_startup.csv` (`0x101aebc3c`, coluna 0 = nome) com
///   `GetClass(nome)` (`0x101ae87f8`, slot `+0x10`, busca direta no mapa de tipos antes dos resolvedores) e `IsA(IGameSystem)`. Se a linha for
///   `CallbackSystem`, a classe nativa homônima que o BWMS forja (pai `IGameSystem`, doador `gameuiInGameMenuGameController`) vence a busca e o
///   sistema criado é o forjado: as virtuais `+0x118..+0x170` são as do doador.
/// - H2: o `game::CallbackSystem` está no `GameInstance` e as virtuais não chamam (H2c) ou chamam e o gancho do despachante não vê (H2b).
/// - H3: nenhuma linha do CSV cria `game::CallbackSystem` nem a forjada: o jogo não dispara `Session/*` por esse caminho.
#[cfg(feature = "devtools")]
pub(crate) mod sonda {
    use super::*;

    /// `0x101ae89c4(GameInstance, &Handle<IGameSystem>)`: registra cada sistema criado do CSV nos mapas do `GameInstance` (`+0x08`, `+0x48`);
    /// único chamador `0x101ae88ac`, dentro do laço das linhas.
    const REGISTRO_DE_SISTEMA: (u64, u32) = (0x1_01ae_89c4, 0xd101_83ff);
    /// `0x105046ad8`: `OnWorldAttached` (slot `+0x118`) do `game::CallbackSystem`, testemunha independente do despachante (`bl` em `0x105046b6c`).
    const MUNDO_LIGADO: (u64, u32) = (0x1_0504_6ad8, 0xd101_03ff);

    static ORIG_REGISTRO: AtomicU64 = AtomicU64::new(0);
    static ORIG_MUNDO: AtomicU64 = AtomicU64::new(0);
    /// Entradas no despachante (todas, antes de filtrar o nome).
    pub(crate) static ENTRADAS: AtomicU64 = AtomicU64::new(0);
    static MUNDOS: AtomicU64 = AtomicU64::new(0);
    static SISTEMAS: AtomicU64 = AtomicU64::new(0);
    /// Sistemas com a vtable do `game::CallbackSystem`; o último deles.
    static DO_JOGO: AtomicU64 = AtomicU64::new(0);
    static ULTIMO_DO_JOGO: AtomicU64 = AtomicU64::new(0);
    /// Sistemas cuja classe é a do doador da forja ou se chama `CallbackSystem` sem ser a do jogo.
    static FORJADOS: AtomicU64 = AtomicU64::new(0);

    pub(super) unsafe fn instalar() {
        let registro = crate::ganchos::instalar_conferindo_prologo(REGISTRO_DE_SISTEMA.0, REGISTRO_DE_SISTEMA.1, sistema_registrado as *mut c_void, &ORIG_REGISTRO);
        let mundo = crate::ganchos::instalar_conferindo_prologo(MUNDO_LIGADO.0, MUNDO_LIGADO.1, mundo_ligado as *mut c_void, &ORIG_MUNDO);
        crate::veredito_dev!("[sessao-sonda] ganchos: registro de sistema do GameInstance {registro:?}, OnWorldAttached do game::CallbackSystem {mundo:?}");
    }

    unsafe extern "C" fn sistema_registrado(gi: u64, handle: *const u64) -> u64 {
        let f: unsafe extern "C" fn(u64, *const u64) -> u64 = std::mem::transmute(crate::ganchos::original_publicado(&ORIG_REGISTRO));
        let r = f(gi, handle);
        SISTEMAS.fetch_add(1, Ordering::Relaxed);
        let inst = ler_u64(handle as *const u8).unwrap_or(0);
        let vt = ler_u64(inst as *const u8).map_or(0, |v| crate::un_rebase(v as *const c_void));
        let nome = crate::rtti::type_name_getname(crate::rtti::class_of(inst as *mut c_void));
        let do_jogo = vt == VTABLE;
        let forjado = !do_jogo && (nome == crate::cname::cname("CallbackSystem") || nome == crate::cname::cname("gameuiInGameMenuGameController"));
        if do_jogo {
            DO_JOGO.fetch_add(1, Ordering::Relaxed);
            ULTIMO_DO_JOGO.store(inst, Ordering::Relaxed);
        }
        if forjado {
            FORJADOS.fetch_add(1, Ordering::Relaxed);
        }
        if do_jogo || forjado || nome == crate::cname::cname("gameCallbackSystem") {
            crate::veredito_dev!(
                "[sessao-sonda] GameInstance {gi:#x} registrou sistema {nome:#x} inst={inst:#x} vtable={vt:#x} game::CallbackSystem={do_jogo} forjado={forjado} s_self={}",
                inst == instancia_do_s_self()
            );
        }
        r
    }

    unsafe extern "C" fn mundo_ligado(sys: *mut c_void, cena: u64) -> u64 {
        let f: unsafe extern "C" fn(*mut c_void, u64) -> u64 = std::mem::transmute(crate::ganchos::original_publicado(&ORIG_MUNDO));
        let n = MUNDOS.fetch_add(1, Ordering::Relaxed);
        if n < 3 {
            crate::veredito_dev!("[sessao-sonda] OnWorldAttached do game::CallbackSystem nº{} ({})", n + 1, retrato(sys));
        }
        f(sys, cena)
    }

    /// Na 1ª presença do jogador: o estado que separa H1/H2/H3 numa linha.
    pub(crate) unsafe fn na_gameplay() {
        static FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if FEITO.swap(true, Ordering::Relaxed) {
            return;
        }
        let s_self = instancia_do_s_self();
        let vt = ler_u64(s_self as *const u8).map_or(0, |v| crate::un_rebase(v as *const c_void));
        let (cls_jogo, cls_nome, cls_script) = match crate::rtti::Registry::obtain() {
            Some(reg) => (
                reg.class_by_name("gameCallbackSystem") as u64,
                reg.class_by_name("CallbackSystem") as u64,
                reg.class_by_script_name("CallbackSystem") as u64,
            ),
            None => (0, 0, 0),
        };
        let (sistemas, do_jogo, forjados) = (SISTEMAS.load(Ordering::Relaxed), DO_JOGO.load(Ordering::Relaxed), FORJADOS.load(Ordering::Relaxed));
        let (mundos, entradas) = (MUNDOS.load(Ordering::Relaxed), ENTRADAS.load(Ordering::Relaxed));
        let hipotese = if forjados > 0 {
            "H1 (o GameInstance criou a classe forjada)"
        } else if sistemas == 0 {
            "sem medida (o gancho do registro de sistema não viu nenhum)"
        } else if do_jogo == 0 {
            "H3 (GameInstance sem game::CallbackSystem)"
        } else if mundos == 0 {
            "H2c (game::CallbackSystem no GameInstance, OnWorldAttached não chamado)"
        } else if entradas == 0 {
            "H2b (OnWorldAttached chamado, gancho do despachante não viu)"
        } else {
            "despachante vivo"
        };
        crate::veredito_dev!(
            "[sessao-sonda] gameplay: {hipotese}; GameInstance registrou {sistemas} sistema(s), game::CallbackSystem {do_jogo} (último={:#x}, s_self={}), forjado(s) {forjados}; OnWorldAttached {mundos}; despachante {entradas}; s_self={s_self:#x} vtable={vt:#x} ({}); GetClass(gameCallbackSystem)={cls_jogo:#x} GetClass(CallbackSystem)={cls_nome:#x} (homônima do BWMS={}) GetClassByScriptName(CallbackSystem)={cls_script:#x}; reservas ligadas={:#06b}",
            ULTIMO_DO_JOGO.load(Ordering::Relaxed),
            ULTIMO_DO_JOGO.load(Ordering::Relaxed) == s_self && s_self != 0,
            if s_self == 0 { "s_self vazio".to_string() } else { retrato(s_self as *mut c_void) },
            cls_nome != 0 && cls_nome != cls_jogo,
            !SESSAO.provado.load(Ordering::Relaxed) & 0xf,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O CName que cada chamador do despachante põe em `x1` (`mov`/`movk`, endereços em `EVENTOS`) é o FNV-1a 64 do nome que o BWMS despacha.
    #[test]
    fn cname_dos_chamadores_e_o_do_nome() {
        let do_binario: [(u64, &str); 9] = [
            (0xc31c_51d3_5650_d441, "Session/BeforeStart"),
            (0xeb3e_fb15_656f_4220, "Session/BeforeEnd"),
            (0x393f_50ba_0a5e_7bcd, "Session/End"),
            (0xf0b9_08a2_8aff_9f2e, "Session/BeforeSave"),
            (0x19ba_1467_4244_f95f, "Session/AfterSave"),
            (0xe647_8c9f_c2f0_363f, "Session/Ready"),
            (0xaade_8761_5909_b814, "Session/Start"),
            (0x4fdd_8c59_699c_60ea, "Session/Pause"),
            (0x74b0_6601_3edf_db8b, "Session/Resume"),
        ];
        for (hash, nome) in do_binario {
            assert_eq!(nome_do_evento(hash), Some(nome), "{nome}");
        }
        assert_eq!(nome_do_evento(0x8e61_375f_26a8_871c), None, "Resource/PostLoad não passa por este despachante");
    }

    /// Gancho posto e despachante mudo (os 15 boots do L12): as reservas seguem como fonte, uma vez por sessão, sessão a sessão.
    #[test]
    fn despachante_mudo_mantem_as_reservas() {
        let s = FontesDaSessao::new();
        for _ in 0..2 {
            assert!(!s.abrir_no_spawn(), "sessão anterior fechada pelo End: nada pendente");
            assert!(s.reserva_ligada(INICIO_DA_SESSAO) && s.reivindicar(INICIO_DA_SESSAO), "Start da reserva");
            assert!(!s.reivindicar(INICIO_DA_SESSAO), "Start repetido na sessão");
            assert!(s.reivindicar(SESSAO_PRONTA), "Ready da reserva");
            assert!(s.reivindicar(FIM_DA_SESSAO), "End do despawn");
            assert!(!s.reivindicar(FIM_DA_SESSAO), "End do exit() depois do despawn");
        }
    }

    /// A 1ª entrega do jogo desliga só a reserva daquele evento.
    #[test]
    fn primeira_entrega_desliga_so_aquele_evento() {
        let s = FontesDaSessao::new();
        s.abrir_no_spawn();
        assert!(s.do_jogo(INICIO));
        assert!(!s.reserva_ligada(INICIO_DA_SESSAO));
        assert!(s.reserva_ligada(SESSAO_PRONTA) && s.reserva_ligada(FIM_DA_SESSAO) && s.reserva_ligada(ANTES_DO_INICIO));
        assert!(s.do_jogo("Session/Pause") && s.do_jogo("Session/Pause"), "sem reserva, o jogo sempre entrega");
    }

    /// Corrida: quem chega primeiro na sessão entrega; o outro não, nas duas ordens.
    #[test]
    fn uma_fonte_por_evento_nas_duas_ordens() {
        let s = FontesDaSessao::new();
        s.abrir_no_spawn();
        assert!(s.reivindicar(INICIO_DA_SESSAO), "reserva antes");
        assert!(!s.do_jogo(INICIO), "o Start do jogo depois da reserva não sai");
        assert!(!s.reserva_ligada(INICIO_DA_SESSAO), "mas prova o jogo");
        assert!(s.do_jogo(FIM), "End do jogo");
        assert!(!s.reivindicar(FIM_DA_SESSAO), "o despawn depois do End do jogo não sai");
        assert!(s.do_jogo(INICIO_ANTES), "BeforeStart do mundo seguinte abre a sessão");
        assert!(s.do_jogo(INICIO), "Start do jogo na sessão nova");
        assert!(s.do_jogo(PRONTO), "Ready do jogo antes da reserva");
        assert!(!s.reivindicar(SESSAO_PRONTA), "Ready da reserva ou do session_ready_tick depois do jogo não sai");
    }

    /// O `Session/BeforeStart` da reserva e o do jogo no mesmo mundo: um só, e o mundo seguinte é do jogo.
    #[test]
    fn antes_do_inicio_um_por_mundo() {
        let s = FontesDaSessao::new();
        assert_eq!(s.antes_do_inicio_pela_reserva(), Some(false), "reserva no 1º mundo");
        assert_eq!(s.antes_do_inicio_pela_reserva(), None, "reserva repetida no mesmo mundo");
        assert!(!s.do_jogo(INICIO_ANTES), "o do jogo no mesmo mundo cede");
        assert_eq!(s.antes_do_inicio_pela_reserva(), None, "reserva desligada depois da 1ª entrega do jogo");
        assert!(s.do_jogo(FIM));
        assert!(s.do_jogo(INICIO_ANTES), "mundo seguinte: o do jogo");
    }

    /// Com o jogo abrindo as sessões, o spawn não apaga o que já saiu nela; sem `BeforeStart` depois de um `End`, o spawn abre.
    #[test]
    fn spawn_so_abre_sem_o_jogo_ou_com_a_sessao_fechada() {
        let s = FontesDaSessao::new();
        assert!(s.do_jogo(INICIO_ANTES));
        assert!(s.reivindicar(SESSAO_PRONTA), "Ready do jogo novo pelo session_ready_tick");
        s.abrir_no_spawn();
        assert!(!s.reivindicar(SESSAO_PRONTA), "o spawn depois do BeforeStart do jogo não reabre");
        assert!(s.do_jogo(FIM));
        s.abrir_no_spawn();
        assert!(s.reivindicar(SESSAO_PRONTA), "sessão fechada: o spawn abre");
        s.abrir_se_fechada();
        assert!(!s.reivindicar(SESSAO_PRONTA), "presença com a sessão aberta não reabre");
    }

    /// Despachante mudo, menu → carga de save sem despawn no tick (boot de 2026-10-24, `proofs/20261024-rodada2-sessao-sonda-H3/`): o spawn
    /// do 2º mundo deixa o `Session/End` do menu com quem chama e abre a sessão onde o `Session/Ready` da carga sai de novo.
    #[test]
    fn spawn_em_outro_mundo_fecha_a_sessao_do_menu() {
        let s = FontesDaSessao::new();
        assert!(!s.abrir_no_spawn(), "1º spawn do processo: sem sessão anterior");
        assert!(s.reivindicar(SESSAO_PRONTA), "Ready do menu (pregame)");
        assert!(s.abrir_no_spawn(), "spawn da carga de save: End do menu pendente");
        assert!(s.reivindicar(SESSAO_PRONTA), "Ready da carga de save na sessão nova");
        assert!(s.reivindicar(FIM_DA_SESSAO), "o End desta sessão segue livre para o despawn/exit()");
        assert!(!s.abrir_no_spawn(), "sessão já fechada pelo End: nada pendente");
    }

    /// Com o `Session/BeforeStart` da reserva: no mesmo mundo o spawn não fecha nada; no mundo seguinte o `BeforeStart` leva o `End` antes.
    #[test]
    fn antes_do_inicio_da_reserva_e_spawn_no_mesmo_mundo() {
        let s = FontesDaSessao::new();
        assert_eq!(s.antes_do_inicio_pela_reserva(), Some(false), "BeforeStart do menu");
        assert!(!s.abrir_no_spawn(), "spawn no mesmo mundo do BeforeStart");
        assert!(!s.reivindicar(ANTES_DO_INICIO), "o BeforeStart deste mundo ficou");
        assert!(s.reivindicar(SESSAO_PRONTA));
        assert_eq!(s.antes_do_inicio_pela_reserva(), Some(true), "mundo seguinte: End do menu antes do BeforeStart");
        assert!(!s.abrir_no_spawn(), "spawn depois do BeforeStart do mundo novo");
        assert!(s.reivindicar(SESSAO_PRONTA), "Ready da sessão nova");
    }

    /// Com o jogo entregando o `BeforeStart`, o spawn nunca deixa `End` pendente: o `OnWorldDetached` do jogo é a fonte.
    #[test]
    fn com_o_jogo_o_spawn_nao_fecha_sessao() {
        let s = FontesDaSessao::new();
        assert!(s.do_jogo(INICIO_ANTES));
        assert!(!s.abrir_no_spawn());
        assert!(s.reivindicar(SESSAO_PRONTA));
        assert!(!s.abrir_no_spawn(), "2º spawn com o jogo no ar");
    }

    /// Duas threads reivindicam o mesmo evento: exatamente uma ganha.
    #[test]
    fn corrida_entre_threads_tem_um_vencedor() {
        for _ in 0..200 {
            let s = std::sync::Arc::new(FontesDaSessao::new());
            let (a, b) = (s.clone(), s.clone());
            let t1 = std::thread::spawn(move || a.do_jogo(INICIO));
            let t2 = std::thread::spawn(move || b.reivindicar(INICIO_DA_SESSAO));
            let (j, r) = (t1.join().unwrap(), t2.join().unwrap());
            assert!(j ^ r, "jogo={j} reserva={r}");
        }
    }
}
