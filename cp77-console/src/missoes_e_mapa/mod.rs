//! Missões e mapa: diário (journal), fases e grafos de missão, pinos do mapa (mappins) e pontos de interesse.

#![allow(dead_code)]

use crate::marcador_individual;
use crate::corpo_e_rosto::*;
use crate::npcs::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use crate::cname::cname;
use crate::rtti::{self, Registry};
use crate::register::*;
/// Bloco deste arquivo no `PROVA_NO_PUBLICO` (register.rs).
#[cfg(not(feature = "devtools"))]
const BLOCO_DE_PROVA: u8 = 0b100;
#[cfg(feature = "devtools")]
use crate::aparencia::*;
use crate::mundo::*;
use crate::ganchos::*;
use crate::diagnostico::{
    bwms_hook_enabled_global, A34_CHAMADAS, A34_FACTS, A35_RECRIADAS, A35_RESTAURACOES_ADIADAS, A55_FORCADOS_NA_RESTAURACAO, FACT_REMOVER_VM,
    MAPPIN_EMPURRADOS, QUESTPHASE_CARGAS, QUESTPHASE_OBSERVAR, QUESTPHASE_RAIZ_PATH, QUESTSYS_ONGAMERESTORED_CNT, QUESTS_ULTIMA,
};
// Só os handlers de prova e o teste do `QuestRootInstance::Start` usam, e o público não cita nenhum dos dois.
#[cfg(feature = "devtools")]
use crate::diagnostico::{
    tramp_journal_mappin_census, tramp_mappin_cooked_position, tramp_mappin_cooked_volume, tramp_mappin_test_inject, tramp_mappin_test_position,
    A34_FILHO_DO_RECURSO_VM, A34_RECURSO_TESTE, A34_RECUSADAS,
};
use crate::tipos_do_motor::*;
#[cfg(feature = "devtools")]
use bwms_primitives::LOCKEY_PREFIXO;

// ===== `OpenWorldTracker`: fase de mappin de POI =====
//
// `SetPoiMappinPhase(uint32_t, gamedataMappinPhase)` e `GetPoiMappinPhase(uint32_t)` são slots
// de VTABLE do `MappinSystem`/`IMappinSystem` — NÃO endereço de função. Os slots Mac NÃO saem de
// "Windows + 0x08": foram medidos na vtable real (nota abaixo, `MAPPIN_SET/GET_POI_PHASE_SLOT`).
//
// `MappinSystem` já é instanciável/acessível neste projeto (item `#78`/`GetAllMappins`,
// provado ao vivo 2026-08-11, via `GameInstance.GetMappinSystem(game) -> ref<MappinSystem>`,
// native VANILLA real) — reusado aqui: as 2 natives abaixo recebem o ponteiro de
// `MappinSystem` (mesmo `Arg::Handle`/1º-arg-raw-pointer já usado por dezenas de outras
// natives, ex. `tramp_entity_find_component_by_type` acima), lêem o 1º qword do objeto
// (vtable), aplicam o offset Mac, lêem o slot como ponteiro de função e chamam — sempre
// validando `crate::memoria::is_readable` ANTES de dereferenciar qualquer ponteiro (o objeto em
// si via `rtti::sane`+leitura da vtable, a vtable antes de indexar o slot, o slot antes de
// ler o function pointer, e o function pointer antes de chamar). `gamedataMappinPhase`
// (`orphans.script:1328`, enum RTTI-visível real: CompletedPhase=0/DefaultPhase=1/
// DiscoveredPhase=2/UndiscoveredPhase=3/Count=4/Invalid=5) é passado/devolvido por VALOR
// como inteiro de 32 bits na ABI Itanium (mesma convenção já usada neste projeto pra todo
// enum nativo passado a native do motor — nenhuma indireção/ponteiro), então os 2 params
// nativos usam `u32` cru (o lado redscript converte `gamedataMappinPhase<->Int32` via
// `EnumInt`/cast, ver `.reds`).
// Slots medidos em 2026-09-06 por disassembly da vtable REAL do `MappinSystem` (`0x107a9b098`,
// achada pelo `slot[0]` do objeto vivo via `BwmsVtableSlot` + `BwmsRebase` + `re-xref vtable-of`):
//
//   slot 122 = 0x3D0 -> `0x10310370c` (228 B). É o **SETTER**: `f(this, key, flag)`, VOID.
//              `cbz x1` aborta com chave 0; faz lookup por hash em `this+0x138/0x148/0x154`;
//              `ldrb w8,[entry+0x10]` e então `orr w8,w8,#2` (flag!=0) ou `and w8,w8,#~2`
//              (flag==0), e `strb` de volta. Nenhum `mov w0` antes do `ret`.
//   slot 123 = 0x3D8 -> `0x1031037f0` (196 B). É o **GETTER**: `f(this, key)`, lê o MESMO byte
//              (`ldrb w0,[entry+0x10]`) e devolve em `w0`; `mov w0,#0` no caminho não-encontrado.
//
// **`0x3D0` não é getter.** Chamado com 1 argumento, devolve lixo (`1245820864`, "+0xC0 por
// chamada"): é função VOID de 3 argumentos e o "retorno" é o que sobra em `x0`. E cada chamada
// MUTA o byte, porque `0x3D0` escreve.
//
// **`0x1D8` (slot 59) não é o setter**: é `0x1030fe028`, 432 bytes, que mexe em `this+0x2c0`
// (outro campo, não o mapa em `0x138`) e ALOCA (`bl 0x1052a4fb8` com `w0=0x20`). Não tem
// relação com fase de POI.
//
// **Por que "Windows + 0x08" não serve aqui.** Só vale quando o offset Windows de partida está
// certo, e os de partida conhecidos (`0x1D0`/`0x3C8`) não batem com esta build — regra do projeto
// ([[cp77-vtable-slot-mac-nao-e-windows-mais-8]]): MEDIR antes.
//
// O SETTER é PROIBIDO em chamada automática (muta estado real de mappin de quest); só a native
// `BwmsMappinSystemSetPoiPhase`, chamada à mão, usa a constante.
pub(crate) const MAPPIN_SET_POI_PHASE_SLOT: usize = 0x3D0; // MEDIDO (slot 122), void, 3 args

pub(crate) const MAPPIN_GET_POI_PHASE_SLOT: usize = 0x3D8; // MEDIDO (slot 123), devolve u8 em w0

#[cfg(feature = "devtools")]
pub(crate) unsafe fn scan_poi_phase_slot(
    sys: *mut c_void,
    amostras: &[(u32, u32)],
    slot_ini: usize,
    slot_fim: usize,
) {
    if sys.is_null() || amostras.is_empty() {
        crate::log_dev!("[poislot] sys nulo ou sem amostras");
        return;
    }
    // PISO ESTRUTURAL — não é conselho, é o mínimo aplicado à variável.
    //
    // **Custou um crash real (2026-09-04, `Bus error: 10` aos 47s).** Mandei varrer a partir do
    // offset 0 e os 2 primeiros slots de qualquer classe C++ com ABI Itanium são os DESTRUTORES
    // (`~T()` e `~T() deleting`). Chamá-los no `MappinSystem` VIVO destrói o objeto do jogo — o
    // processo morreu antes de a varredura terminar, e nenhum veredito saiu. Os slots seguintes
    // ainda são os virtuais herdados de `IGameSystem` (Init/Shutdown/…), igualmente destrutivos.
    // Por isso o piso mora aqui dentro: quem chamar com `slot_ini` menor simplesmente não
    // consegue tocar nessa faixa.
    const PISO_SEGURO: usize = 0x40; // pula dtors + os virtuais básicos de IGameSystem
    let slot_ini = slot_ini.max(PISO_SEGURO);
    // TETO ESTRUTURAL — a outra metade do mesmo crash. Passar do fim da vtable significa ler um
    // u64 que não é ponteiro de função e CHAMÁ-LO. `vtbl_slot_fn` só checa se o alvo é legível, e
    // um endereço qualquer dentro do binário passa nessa checagem — então a proteção tem de vir do
    // TAMANHO REAL da tabela, medido antes de varrer. Na `MappinSystem` isso dá 35 slots
    // (fim em 0x118): varrer até 0x200, como a chamada pedia, seria chamar 29 endereços que não
    // são métodos dela.
    let nslots = vtable_slot_count_cached(sys);
    let fim_real = nslots.saturating_mul(8);
    let slot_fim = if fim_real > slot_ini { slot_fim.min(fim_real - 8) } else { slot_ini };

    // ── COLETA SEGURA, SEMPRE ────────────────────────────────────────────────────────────────
    // Reporta o ENDEREÇO ESTÁTICO da vtable antes de qualquer chamada. Com ele, todo o resto da
    // análise (quais slots são getters minúsculos, qual devolve enum) é feita OFFLINE com
    // `re-xref vtable-dump` + disassembly — sem chamar nada, sem arriscar o processo.
    //
    // **Por que isto passou a vir antes da varredura (2026-09-05):** a varredura CHAMA virtuais com
    // assinatura `(this, u32) -> u64`. Se o slot real tiver outra assinatura, a chamada é UB — e o
    // boot morreu logo depois de a varredura começar, sem chegar ao veredito. Um dado colhido em
    // segurança vale mais que um dado que talvez não sobreviva ao próprio método de coleta.
    let vt_runtime = (sys as *const u64).read_unaligned();
    let base_runtime = crate::rebase(0x1_0000_0000) as u64;
    let vt_estatico = vt_runtime.wrapping_sub(base_runtime).wrapping_add(0x1_0000_0000);
    crate::veredito_dev!(
        "[poislot] VTABLE da MappinSystem: runtime={vt_runtime:#x} estático={vt_estatico:#x} slots={nslots} — analisar offline com `re-xref vtable-dump {vt_estatico:#x} {nslots}`"
    );

    // A varredura por chamada fica atrás de marcador explícito: é a parte arriscada.
    let marcador_scan = crate::marcador_de_dev(".bwms-poislot-scan");
    if !marcador_scan {
        crate::log_veredito(
            "[poislot] varredura por CHAMADA desativada (arme ~/.bwms-poislot-scan pra ligar) — só a coleta segura rodou",
        );
        return;
    }
    crate::veredito_dev!(
        "[poislot] vtable tem {nslots} slots (fim {fim_real:#x}); varrendo {slot_ini:#x}..{slot_fim:#x} — piso {PISO_SEGURO:#x} (dtors/IGameSystem) e teto no fim REAL da tabela"
    );
    type Fn32 = unsafe extern "C" fn(*mut c_void, u32) -> u64;
    let mut candidatos = 0usize;
    let mut plausiveis = 0usize;
    let mut slot = slot_ini & !7;
    while slot <= slot_fim {
        if let Some(fp) = vtbl_slot_fn(sys, slot) {
            let f: Fn32 = core::mem::transmute(fp);
            let mut bate = true;
            let mut primeiro = 0u32;
            let mut vistos: Vec<u32> = Vec::new();
            for (i, (hash, esperado)) in amostras.iter().enumerate() {
                let r = (f(sys, *hash) & 0xffff_ffff) as u32;
                vistos.push(r);
                if i == 0 {
                    primeiro = r;
                }
                if r != *esperado {
                    bate = false;
                }
            }
            // "Cabe num enum de fase" = todos os retornos pequenos. Filtro grosseiro de propósito.
            let plausivel_enum = !vistos.is_empty() && vistos.iter().all(|v| *v < 8);
            if bate {
                candidatos += 1;
                crate::veredito_dev!(
                    "[poislot] >>> CANDIDATO: vtbl+{slot:#x} bateu com o oráculo em {}/{} amostras (1º={primeiro}) <<<",
                    amostras.len(), amostras.len()
                );
            } else if plausivel_enum {
                // 2º critério, usado quando NÃO há gabarito do oráculo (o save pode não ter estado
                // salvo pra POI nenhum — foi o caso em 2026-09-04). Um `gamedataMappinPhase` válido
                // é um número PEQUENO; o slot errado `0x3D0` devolve ~1,4 bilhão. Então "todos os
                // retornos < 8" já separa drasticamente. Não é prova sozinho — é a lista curta que
                // vale inspecionar.
                plausiveis += 1;
                crate::veredito_dev!(
                    "[poislot] plausível: vtbl+{slot:#x} -> [{}]",
                    vistos.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ")
                );
            }
        }
        slot += 8;
    }
    crate::veredito_dev!(
        "[poislot] VEREDITO: {candidatos} slot(s) batem com o oráculo; {plausiveis} devolvem valor de enum plausível (<8)"
    );
}

/// Ponta redscript: `BwmsMappinScanPoiSlot(sys, h1,p1, h2,p2, h3,p3, slotIni, slotFim)`.
/// Os pares `(hash, fase)` vêm do oráculo vanilla, colhidos no próprio redscript.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mappin_scan_poi_slot(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let g = |i: usize| args.get(i).map(|(v, _)| *v as u32).unwrap_or(0);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut amostras = Vec::new();
    for k in 0..3 {
        let h = g(1 + k * 2);
        let p = g(2 + k * 2);
        if h != 0 {
            amostras.push((h, p));
        }
    }
    let ini = args.get(7).map(|(v, _)| *v as usize).filter(|v| *v < 0x2000).unwrap_or(0x300);
    let fim = args.get(8).map(|(v, _)| *v as usize).filter(|v| *v < 0x2000).unwrap_or(0x480);
    // Pelo canal de veredito: esta chamada acontece na 2ª firing, além do orçamento do `log()`.
    crate::veredito_dev!(
        "[poislot] chamado com {} amostra(s) (o `0x3D0` seria o slot {})",
        amostras.len(), 0x3D0usize / 8
    );
    scan_poi_phase_slot(sys, &amostras, ini, fim.max(ini));
}

pub(crate) type MappinSetPoiPhaseFn = unsafe extern "C" fn(*mut c_void, u32, u32);

/// `BwmsMappinSystemSetPoiPhase(sys: ref<MappinSystem>, journalPath: Uint32, phase: Int32) -> Void`
/// — chama o setter em `MAPPIN_SET_POI_PHASE_SLOT` (ver nota grande acima). O 3º argumento é
/// FLAG, não fase: `!= 0` liga o bit 2 do byte de flags da entrada, `0` desliga. MUTA estado
/// real de mappin de quest — nunca chamado automaticamente por nenhum smoke test deste projeto.
pub(crate) unsafe extern "C" fn tramp_mappinsystem_set_poi_phase(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let journal_path = args.get(1).map(|(v, _)| *v as u32).unwrap_or(0);
    let phase = args.get(2).map(|(v, _)| *v as u32).unwrap_or(0);
    match vtbl_slot_fn_bounded(sys, MAPPIN_SET_POI_PHASE_SLOT) {
        Some(fp) => {
            let f: MappinSetPoiPhaseFn = core::mem::transmute(fp);
            f(sys, journal_path, phase);
            crate::log_dev!(
                "[mappinsystem] BwmsMappinSystemSetPoiPhase(sys={sys:p}, journalPath={journal_path:#x}, phase={phase}) chamado via vtbl+{MAPPIN_SET_POI_PHASE_SLOT:#x}"
            );
        }
        None => crate::log_dev!(
            "[mappinsystem] BwmsMappinSystemSetPoiPhase: sys={sys:p} inválido/vtable+{MAPPIN_SET_POI_PHASE_SLOT:#x} ilegível — NÃO chamado"
        ),
    }
}

/// `BwmsMappinSystemGetPoiPhase(sys: ref<MappinSystem>, journalPath: Uint32) -> Int32` —
/// chama o getter em `MAPPIN_GET_POI_PHASE_SLOT` (ver nota grande acima). Devolve o byte CRU de
/// flags da entrada (`ldrb`), não uma fase; chave não encontrada devolve `0`. Só LEITURA — seguro
/// pra smoke test automático, mesmo com `journalPath` fabricado.
pub(crate) unsafe extern "C" fn tramp_mappinsystem_get_poi_phase(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let journal_path = args.get(1).map(|(v, _)| *v as u32).unwrap_or(0);
    let mut phase: u32 = 5; // sentinela = gamedataMappinPhase.Invalid, só usado se a chamada não rolar
    let mut called = false;
    if let Some(fp) = vtbl_slot_fn_bounded(sys, MAPPIN_GET_POI_PHASE_SLOT) {
        let f: MappinGetPoiPhaseFn = core::mem::transmute(fp);
        phase = f(sys, journal_path);
        called = true;
    }
    crate::log_dev!(
        "[mappinsystem] BwmsMappinSystemGetPoiPhase(sys={sys:p}, journalPath={journal_path:#x}) -> {phase} (chamado={called}, vtbl+{MAPPIN_GET_POI_PHASE_SLOT:#x})"
    );
    write_uint_ret(out, phase as u64, 4);
}

// ---------------------------------------------------------------------------------------------
// Os eventos de sessão via override de vtable de `IGameSystem` (RODADA 2026-08-16, 100% OFFLINE):
// os 20 VIRTUAIS de `IGameSystem` têm offset Windows conhecido por slot (`OnWorldAttached` `0x110`
// ... `OnUninitialize` `0x1A8`). 11 dos 20 slots mapeiam pra um `CName` de evento de sessão
// (`OnWorldAttached`→`"Session/BeforeStart"`, `OnGamePrepared`→`"Session/Start"`, etc.). Ver tabela
// `IGAMESYSTEM_SLOTS` abaixo.
//
// `IGameSystem` estende `IUpdatableSystem` (1 virtual próprio, `OnRegisterUpdates`@Windows
// `0x108`) que estende `IScriptable` — os 20 virtuais NOVOS de `IGameSystem` começam em `0x110`
// (imediatamente após), confirmando que não há slot "escondido" entre os dois. `MappinSystem`
// (já acessível AO VIVO via `GameInstance.GetMappinSystem(game)`, itens `#78`/`#142`/`#232` já
// fechados) estende `IMappinSystem`→`IReplicatedGameSystem`→`IGameSystem` — `IReplicatedGameSystem`
// NÃO adiciona nenhum virtual novo (só +0x10 de dado cru, confirmado no header GERADO da
// reflection real do jogo) — então os slots `0x110`-`0x1A8` de `MappinSystem` SÃO literalmente
// os 20 slots de `IGameSystem`, sem deslocamento. Os offsets Windows `0x1D0`/`0x3C8` da fase de POI
// não servem de âncora: não batem com esta build (nota do `MAPPIN_SET_POI_PHASE_SLOT`).
//
// **O que isto NÃO resolve, honestamente**: mesmo com os 20 offsets agora conhecidos com certeza
// (não mais "índice desconhecido"), a metade FALTANTE do bloqueador original é DIFERENTE — não é
// "onde fica o slot", é "como fazer o MOTOR chamar o NOSSO código nesse slot". Duas vias
// possíveis, NENHUMA testada ainda: (a) nosso próprio `CallbackSystem` forjado (já
// registrado como `IGameSystem`-derivado) receber essas chamadas de verdade — MAS o próprio
// projeto já optou deliberadamente por NÃO depender disso pro dispatch real de
// `Session/Start`/`Session/End` (usa polling de presença do player, `controller-session`,
// 2026-07-18) porque nunca foi confirmado se o motor de fato invoca lifecycle callbacks numa
// instância forjada não descoberta pelo mecanismo de registro real de sistemas; (b)
// `trocar_slot_da_vtable` (técnica já em produção no projeto) num slot de um sistema VANILLA real (ex.
// `MappinSystem`, já com ponteiro vivo) — MUTaria/intercepta o lifecycle de um sistema de
// produção real, categoria de risco alto que o projeto evita de propósito (mesma razão que
// bloqueou esta linha desde o início). Este achado troca "índice desconhecido, categoria XL" por
// "índice conhecido, falta confirmar a via de wiring segura" — reduz a incerteza mas NÃO fecha o
// item.
//
// **Probe desta rodada — 100% OBSERVE-ONLY, ZERO chamada de função nativa/vtable**: dump
// read-only dos 11 slots de evento de sessão, contra uma instância VIVA e já
// acessível de `MappinSystem` (mesma cadeia de herança, offsets idênticos) — só lê o ponteiro de
// função + os 4 primeiros bytes (checagem frouxa de prólogo ARM64, mesmo idioma já usado em
// `journalvtdump`/`lib.rs`), NUNCA invoca. Objetivo: confirmar (ou refutar) que o shift
// Mac=Windows+0x08 também bate pra ESTA família de vtable antes de qualquer sessão futura decidir
// arriscar um `trocar_slot_da_vtable` real. Zero mutação, zero risco de crash, seguro pra rodar automático
// (mais conservador ainda que os probes de LEITURA já existentes, que ao menos CHAMAM o slot —
// este nem chama).
// ---------------------------------------------------------------------------------------------
#[cfg(feature = "devtools")]
pub(crate) const IGAMESYSTEM_SLOTS: &[(&str, usize, &str)] = &[
    // (nome do virtual C++, offset Windows em bytes, evento CName de sessão)
    ("OnWorldAttached", 0x110, "Session/BeforeStart"),
    ("OnBeforeWorldDetach", 0x118, "Session/BeforeEnd"),
    ("OnWorldDetached", 0x120, "Session/End"),
    ("OnAfterWorldDetach", 0x128, "(interno — sweep de handlers Session, sem evento)"),
    ("OnBeforeGameSave", 0x130, "Session/BeforeSave"),
    ("OnAfterGameSave", 0x140, "Session/AfterSave"),
    ("OnGameLoad", 0x148, "(interno — seta m_restored=true, sem evento)"),
    ("OnGameRestored", 0x150, "Session/Ready"),
    ("OnGamePrepared", 0x158, "Session/Start"),
    ("OnGamePaused", 0x160, "Session/Pause"),
    ("OnGameResumed", 0x168, "Session/Resume"),
];

/// `BwmsMappinSystemDumpIGameSystemSlots(sys: ref<MappinSystem>) -> Void` — ver
/// nota grande acima. 100% OBSERVE-ONLY: só lê o ponteiro de função em cada slot de
/// `IGameSystem` (Mac = Windows+0x08) + os 4 primeiros bytes do prólogo, NUNCA chama nada.
/// `sys` é qualquer instância viva de uma classe que estenda `IGameSystem` sem virtuais extras
/// ANTES do slot `0x110` (confirmado pra `MappinSystem`, ver nota) — zero mutação, seguro pra
/// smoke test automático.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mappinsystem_dump_igamesystem_slots(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    if sys.is_null() || !rtti::sane(sys) || !crate::memoria::is_readable(sys, 8) {
        crate::log_dev!("[igamesystem] BwmsMappinSystemDumpIGameSystemSlots: sys inválido/ilegível — abortado");
        return;
    }
    let vtbl = (sys as *const u64).read_unaligned() as *const u8;
    if vtbl.is_null() {
        crate::log_dev!("[igamesystem] BwmsMappinSystemDumpIGameSystemSlots: vtable nula");
        return;
    }
    crate::log_dev!(
        "[igamesystem] sys={sys:p} vtable={vtbl:p} — dump read-only dos 11 slots de IGameSystem que o CallbackSystem real override (Mac=Windows+0x08, NUNCA chamado, só leitura de ponteiro):"
    );
    for (name, win_off, event) in IGAMESYSTEM_SLOTS {
        let mac_off = win_off + 0x08;
        let slot_ptr = vtbl.add(mac_off) as *const u64;
        if !crate::memoria::is_readable(slot_ptr as *const c_void, 8) {
            crate::log_dev!("[igamesystem] {name} (win={win_off:#x} mac={mac_off:#x}) slot ilegível");
            continue;
        }
        let fp = slot_ptr.read_unaligned() as *const u8;
        if fp.is_null() {
            crate::log_dev!("[igamesystem] {name} (win={win_off:#x} mac={mac_off:#x}) -> ponteiro nulo");
            continue;
        }
        let mut prologue_ok = false;
        let mut bytes4 = [0u8; 4];
        if crate::memoria::is_readable(fp as *const c_void, 4) {
            std::ptr::copy_nonoverlapping(fp, bytes4.as_mut_ptr(), 4);
            let op0 = u32::from_le_bytes(bytes4);
            prologue_ok = op0 != 0 && op0 != 0xFFFF_FFFF;
        }
        crate::log_dev!(
            "[igamesystem] {name} (win={win_off:#x} mac={mac_off:#x}, evento='{event}') fn={fp:p} bytes={bytes4:02x?} plausible={prologue_ok}"
        );
    }
}

pub(crate) unsafe fn register_mappinsystem_igamesystem_vtdump(reg: &Registry, proto: *mut c_void) {
    let ok = registro_de_prova!(register_argful_by_types(
        reg,
        proto,
        &["handle:MappinSystem"],
        "BwmsMappinSystemDumpIGameSystemSlots",
        "BwmsMappinSystemDumpIGameSystemSlots", corpo_de_prova!(tramp_mappinsystem_dump_igamesystem_slots),
    ));
    crate::log_dev!(
        "[reg] register_all: igamesystem-vtdump BwmsMappinSystemDumpIGameSystemSlots={ok}"
    );
}

#[cfg(feature = "devtools")]
pub(crate) const IGAMESYSTEM_ONGAMERESTORED_SLOT_IDX: usize = 0x158 / 8; // 43 — mac offset confirmado ao vivo (proof 2026-08-16)

#[cfg(feature = "devtools")]
pub(crate) static IGS_OGR_INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "devtools")]
pub(crate) static IGS_OGR_FIRE_CNT: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static ORIG_IGS_ONGAMERESTORED: std::sync::atomic::AtomicPtr<c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

#[cfg(feature = "devtools")]
pub(crate) type OnGameRestoredFn = unsafe extern "C" fn(*mut c_void) -> bool;

/// Hook OBSERVE-ONLY: só loga que disparou + SEMPRE repassa pro original (nunca suprime, nunca
/// muda o retorno) — mesmo idioma já provado 7800+ vezes pelo hook do
/// `AnimationSystem_FrameBeginReset` (`selftest.rs`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn bwms_igamesystem_ongamerestored_hook(this: *mut c_void) -> bool {
    let n = IGS_OGR_FIRE_CNT.fetch_add(1, Ordering::Relaxed);
    crate::log_dev!(
        "[igamesystem-vtablehook] OnGameRestored INTERCEPTADO #{n} this={this:p} — evento real 'Session/Ready'"
    );
    let orig = ORIG_IGS_ONGAMERESTORED.load(Ordering::Relaxed);
    if orig.is_null() {
        // Nunca deveria acontecer (só instalamos depois de capturar o original com sucesso),
        // mas por segurança nunca inventa um retorno — false é o "não-sucesso" mais conservador
        // possível, nunca crasha nem lê memória adicional.
        crate::log_dev!("[igamesystem-vtablehook] ORIGINAL NULO — retorno de segurança (false), NÃO deveria acontecer");
        return false;
    }
    let f: OnGameRestoredFn = std::mem::transmute(orig);
    f(this)
}

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mappinsystem_hook_ongamerestored(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    if sys.is_null() || !rtti::sane(sys) || !crate::memoria::is_readable(sys, 8) {
        crate::log_dev!("[igamesystem-vtablehook] BwmsMappinSystemHookOnGameRestored: sys inválido/ilegível — abortado");
        write_bool_ret(out, false);
        return;
    }
    if IGS_OGR_INSTALLED.load(Ordering::Relaxed) {
        // idempotente — a smoke test dispara em toda firing de OnGameAttached (menu + real), só
        // a 1ª deve instalar de verdade (instalar 2x reescreveria o slot com o PRÓPRIO hook como
        // "original", quebrando o passthrough).
        write_bool_ret(out, true);
        return;
    }
    let vtbl = (sys as *const u64).read_unaligned() as *mut u64;
    if vtbl.is_null()
        || !crate::memoria::is_readable(vtbl as *const c_void, (IGAMESYSTEM_ONGAMERESTORED_SLOT_IDX + 1) * 8)
    {
        crate::log_dev!("[igamesystem-vtablehook] vtable nula/ilegível — abortado");
        write_bool_ret(out, false);
        return;
    }
    match crate::ganchos::trocar_slot_da_vtable(
        vtbl,
        IGAMESYSTEM_ONGAMERESTORED_SLOT_IDX,
        bwms_igamesystem_ongamerestored_hook as *const c_void,
    ) {
        Some(orig) => {
            ORIG_IGS_ONGAMERESTORED.store(orig as *mut c_void, Ordering::Relaxed);
            IGS_OGR_INSTALLED.store(true, Ordering::Relaxed);
            crate::log_dev!(
                "[igamesystem-vtablehook] trocar_slot_da_vtable instalado no slot OnGameRestored (idx={IGAMESYSTEM_ONGAMERESTORED_SLOT_IDX}, mac_off=0x158) vtbl={vtbl:p} original={orig:p}"
            );
            write_bool_ret(out, true);
        }
        None => {
            crate::log_dev!("[igamesystem-vtablehook] trocar_slot_da_vtable FALHOU (set_prot recusou)");
            write_bool_ret(out, false);
        }
    }
}

pub(crate) unsafe fn register_mappinsystem_hook_ongamerestored(reg: &Registry, proto: *mut c_void) {
    let ok = registro_de_prova!(register_argful_by_types(
        reg,
        proto,
        &["handle:MappinSystem"],
        "BwmsMappinSystemHookOnGameRestored",
        "BwmsMappinSystemHookOnGameRestored", corpo_de_prova!(tramp_mappinsystem_hook_ongamerestored),
    ));
    crate::log_dev!(
        "[reg] register_all: igamesystem-vtablehook-ongamerestored BwmsMappinSystemHookOnGameRestored={ok}"
    );
}

#[cfg(feature = "devtools")]
pub(crate) const IGAMESYSTEM_ONWORLDATTACHED_SLOT_IDX: usize = 0x118 / 8; // 35 — mac offset (Windows 0x110 +0x08, confirmado ao vivo 2026-08-16)

#[cfg(feature = "devtools")]
pub(crate) static IGS_OWA_INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "devtools")]
pub(crate) static IGS_OWA_FIRE_CNT: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static ORIG_IGS_ONWORLDATTACHED: std::sync::atomic::AtomicPtr<c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

#[cfg(feature = "devtools")]
pub(crate) type OnWorldAttachedFn = unsafe extern "C" fn(*mut c_void, *mut c_void);

/// Hook OBSERVE-ONLY: loga que disparou, SEMPRE repassa pro original com os mesmos args (nunca
/// suprime), e — só depois de confirmar um listener REAL registrado pra "Session/BeforeStart" —
/// despacha o evento real (bounded a 3 chamadas, mesmo padrão já decisivo do `#127`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn bwms_igamesystem_onworldattached_hook(this: *mut c_void, scene: *mut c_void) {
    let n = IGS_OWA_FIRE_CNT.fetch_add(1, Ordering::Relaxed);
    if n < 8 {
        crate::log_dev!(
            "[igamesystem-vtablehook-owa] OnWorldAttached INTERCEPTADO #{n} this={this:p} scene={scene:p} — evento real 'Session/BeforeStart'"
        );
    }
    let orig = ORIG_IGS_ONWORLDATTACHED.load(Ordering::Relaxed);
    if orig.is_null() {
        crate::log_dev!("[igamesystem-vtablehook-owa] ORIGINAL NULO — não deveria acontecer, abortando repasse com segurança");
        return;
    }
    let f: OnWorldAttachedFn = std::mem::transmute(orig);
    f(this, scene);

    // Ponteiro da cena — 2026-08-22, 2ª tentativa. A 1ª varria a `RuntimeScene` AQUI DENTRO e
    // chamava `rtti::class_of` em cada ponteiro plausível. Isso crashou o jogo: `class_of`
    // dereferencia a vtable e despacha — num ponteiro que só "parece" heap, é null-deref na hora.
    // É a mesma categoria que já custou um crash real neste projeto (`custsys probe`, 2026-07-28).
    //
    // Forma segura: o hook faz UMA escrita atômica com o ponteiro da cena e mais nada. A cena é lida
    // fora do hook (`register::cena_base`), e os runtime systems saem por índice
    // (`register::runtime_system_at`, comando `rtsys`), sem varrer ponteiro: a varredura derrubou o
    // jogo também fora do hook, em gameplay (`proofs/20260930-c39-crash-no-comando-scenedump.ips`).
    IGS_OWA_SCENE_PTR.store(scene as u64, Ordering::Relaxed);

    // FECHADO E PROVADO AO VIVO 2026-08-18 (4º boot, prova do despacho estático do
    // `OnWorldAttached`): `OnWorldAttached` disparou 1 vez num boot de
    // sessão única (não repete como `IComponent::Toggle`; numa 2ª sessão do boot [não medido]), durante o
    // world-swap real do autocontinue — zero crash em 4 boots. Achados de correção ao longo do
    // caminho: (1) listener de INSTÂNCIA (`RegisterCallback`, `ref<PlayerPuppet>` do menu
    // espúrio) já está MORTO nesse instante — `rtti::sane(target)` recusa corretamente, mecanismo
    // SEGURO, não bug — então `RegisterStaticCallback` (função GLOBAL, sem instância, mesmo
    // padrão já provado pro `CallbackSystem`) é a via que sobrevive ao world-swap; (2) `has_listener_for`
    // só olha `CALLBACKS` (registry de instância), por isso checamos `STATIC_CALLBACKS` também.
    // Promovido a SEMPRE-ATIVO (sem marcador de opt-in): gate de dev que nunca liga por padrão
    // deixa a capacidade FECHADA mas inacessível a mod real (rodada 60, 2026-08-17) — o núcleo é 100% observe-only
    // (sempre repassa pro original ANTES desta seção) e o dispatch é bounded a 3 fires totais.
    static IGS_OWA_FIRES_DONE: AtomicU64 = AtomicU64::new(0);
    let has_static = STATIC_CALLBACKS
        .lock()
        .map(|sc| sc.iter().any(|(ev, _, _)| *ev == cname("Session/BeforeStart")))
        .unwrap_or(false);
    // Reserva do `Session/BeforeStart`: desliga na 1ª entrega do `OnWorldAttached` do `game::CallbackSystem` e cede ao dele no mesmo mundo
    // (`inicio::sessao_do_jogo`).
    if crate::inicio::sessao_do_jogo::reserva_ligada(crate::inicio::sessao_do_jogo::ANTES_DO_INICIO)
        && (crate::tipos_do_motor::has_listener_for("Session/BeforeStart") || has_static)
    {
        let done = IGS_OWA_FIRES_DONE.fetch_add(1, Ordering::Relaxed);
        let fonte = if done < 3 { crate::inicio::sessao_do_jogo::antes_do_inicio_pela_reserva() } else { None };
        if let Some(fim_pendente) = fonte {
            // Mundo novo depois de um com jogador e sem `Session/End`: o `End` dele antes do `BeforeStart` deste
            // (`OnWorldDetached` do mundo velho antes do `OnWorldAttached` do novo), na mesma forma sem argumento do `BeforeStart` daqui.
            if fim_pendente {
                let fim = fire_event_args("Session/End", &[]);
                crate::veredito_dev!("[sessao-reserva] Session/End (OnWorldAttached do MappinSystem #{n}: mundo anterior sem End) → {fim} callback(s)");
            }
            let sent = fire_event_args("Session/BeforeStart", &[]);
            crate::veredito_dev!("[sessao-reserva] Session/BeforeStart (OnWorldAttached do MappinSystem #{n}, fire #{done}) → {sent} callback(s)");
        }
    }
}

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mappinsystem_hook_onworldattached(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    if sys.is_null() || !rtti::sane(sys) || !crate::memoria::is_readable(sys, 8) {
        crate::log_dev!("[igamesystem-vtablehook-owa] BwmsMappinSystemHookOnWorldAttached: sys inválido/ilegível — abortado");
        write_bool_ret(out, false);
        return;
    }
    if IGS_OWA_INSTALLED.load(Ordering::Relaxed) {
        // idempotente — a smoke test dispara em toda firing de OnGameAttached (menu + real), só a
        // 1ª deve instalar de verdade.
        write_bool_ret(out, true);
        return;
    }
    let vtbl = (sys as *const u64).read_unaligned() as *mut u64;
    if vtbl.is_null()
        || !crate::memoria::is_readable(vtbl as *const c_void, (IGAMESYSTEM_ONWORLDATTACHED_SLOT_IDX + 1) * 8)
    {
        crate::log_dev!("[igamesystem-vtablehook-owa] vtable nula/ilegível — abortado");
        write_bool_ret(out, false);
        return;
    }
    match crate::ganchos::trocar_slot_da_vtable(
        vtbl,
        IGAMESYSTEM_ONWORLDATTACHED_SLOT_IDX,
        bwms_igamesystem_onworldattached_hook as *const c_void,
    ) {
        Some(orig) => {
            ORIG_IGS_ONWORLDATTACHED.store(orig as *mut c_void, Ordering::Relaxed);
            IGS_OWA_INSTALLED.store(true, Ordering::Relaxed);
            crate::log_dev!(
                "[igamesystem-vtablehook-owa] trocar_slot_da_vtable instalado no slot OnWorldAttached (idx={IGAMESYSTEM_ONWORLDATTACHED_SLOT_IDX}, mac_off=0x118) vtbl={vtbl:p} original={orig:p}"
            );
            write_bool_ret(out, true);
        }
        None => {
            crate::log_dev!("[igamesystem-vtablehook-owa] trocar_slot_da_vtable FALHOU (set_prot recusou)");
            write_bool_ret(out, false);
        }
    }
}

pub(crate) unsafe fn register_mappinsystem_hook_onworldattached(reg: &Registry, proto: *mut c_void) {
    let ok = registro_de_prova!(register_argful_by_types(
        reg,
        proto,
        &["handle:MappinSystem"],
        "BwmsMappinSystemHookOnWorldAttached",
        "BwmsMappinSystemHookOnWorldAttached", corpo_de_prova!(tramp_mappinsystem_hook_onworldattached),
    ));
    crate::log_dev!(
        "[reg] register_all: igamesystem-vtablehook-onworldattached BwmsMappinSystemHookOnWorldAttached={ok}"
    );
}

pub(crate) const MAPPINSYS_COOKED_MAPPIN_RESOURCE_HANDLE_OFF: usize = 0x58; // Handle<gameMappinResource>

pub(crate) const MAPPINSYS_COOKED_POI_RESOURCE_HANDLE_OFF: usize = 0x68; // Handle<gamePointOfInterestMappinResource>

pub(crate) const MAPPINRES_COOKED_DATA_OFF: usize = 0x40; // DynArray<game::CookedMappinData>

pub(crate) const MAPPINRES_COOKED_MULTI_DATA_OFF: usize = 0x50; // DynArray<game::CookedMultiMappinData>

pub(crate) const POIRES_COOKED_DATA_OFF: usize = 0x40; // DynArray<game::CookedPointOfInterestMappinData>

/// Lê um campo `Handle<T>` (16 bytes: `instance`@+0x0/`refCount`@+0x8) num offset de INSTÂNCIA C++ real (não `ref<T>` redscript) — devolve só
/// o ponteiro `instance`. Guarda de leitura antes de qualquer dereferência; `None` cobre tanto
/// "ilegível" quanto "handle vazio" (`instance==null`, caso NORMAL antes do recurso ser cacheado
/// — não é erro).
pub(crate) unsafe fn read_handle_instance(ptr: *mut c_void, offset: usize) -> Option<*mut c_void> {
    if ptr.is_null() || !crate::memoria::is_readable((ptr as *const u8).add(offset) as *const c_void, 16) {
        return None;
    }
    let instance = rd_u64(ptr, offset) as *mut c_void;
    if instance.is_null() {
        return None;
    }
    Some(instance)
}

pub(crate) unsafe extern "C" fn tramp_mappinsystem_cooked_mappin_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let result = mappin_cooked_count(sys, MAPPINSYS_COOKED_MAPPIN_RESOURCE_HANDLE_OFF, MAPPINRES_COOKED_DATA_OFF);
    crate::log_dev!("[mappinsystem] BwmsMappinSystemCookedMappinCount(sys={sys:p}) -> {result}");
    write_uint_ret(out, (result as u32) as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_mappinsystem_cooked_multi_mappin_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let result = mappin_cooked_count(sys, MAPPINSYS_COOKED_MAPPIN_RESOURCE_HANDLE_OFF, MAPPINRES_COOKED_MULTI_DATA_OFF);
    crate::log_dev!("[mappinsystem] BwmsMappinSystemCookedMultiMappinCount(sys={sys:p}) -> {result}");
    write_uint_ret(out, (result as u32) as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_mappinsystem_cooked_poi_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let result = mappin_cooked_count(sys, MAPPINSYS_COOKED_POI_RESOURCE_HANDLE_OFF, POIRES_COOKED_DATA_OFF);
    crate::log_dev!("[mappinsystem] BwmsMappinSystemCookedPoiCount(sys={sys:p}) -> {result}");
    write_uint_ret(out, (result as u32) as u64, 4);
}

pub(crate) unsafe fn register_mappinsystem_cooked_count_natives(reg: &Registry, proto: *mut c_void) {
    let params = compose_params_from_types(reg, &["handle:MappinSystem"]);
    let ok = params_compostos(params);
    if !ok {
        crate::log_dev!("[reg] register_mappinsystem_cooked_count_natives: compose_params_from_types falhou (handle:MappinSystem)");
    }
    let ok_mappin = ok
        && register_global_composed(
            reg,
            proto,
            params,
            "BwmsMappinSystemCookedMappinCount",
            "BwmsMappinSystemCookedMappinCount",
            tramp_mappinsystem_cooked_mappin_count,
        );
    let ok_multi = ok
        && register_global_composed(
            reg,
            proto,
            params,
            "BwmsMappinSystemCookedMultiMappinCount",
            "BwmsMappinSystemCookedMultiMappinCount",
            tramp_mappinsystem_cooked_multi_mappin_count,
        );
    let ok_poi = ok
        && register_global_composed(
            reg,
            proto,
            params,
            "BwmsMappinSystemCookedPoiCount",
            "BwmsMappinSystemCookedPoiCount",
            tramp_mappinsystem_cooked_poi_count,
        );
    crate::log_dev!(
        "[reg] register_all: mappin-cooked-count BwmsMappinSystemCookedMappinCount={ok_mappin} BwmsMappinSystemCookedMultiMappinCount={ok_multi} BwmsMappinSystemCookedPoiCount={ok_poi}"
    );
}

/// `QuestsSystem.RootPhase@+0x68` → `QuestPhaseInstance.Graph@+0x40` = o grafo de fase raiz.
pub(crate) unsafe fn quest_root_graph(sys: *mut c_void) -> *mut c_void {
    if !rtti::sane(sys) || !crate::memoria::is_readable(sys as *const c_void, 0x70) {
        return std::ptr::null_mut();
    }
    let phase = rd_u64(sys, 0x68) as *mut c_void;
    if !rtti::sane(phase) || !crate::memoria::is_readable(phase as *const c_void, 0x48) {
        return std::ptr::null_mut();
    }
    let graph = rd_u64(phase, 0x40) as *mut c_void;
    if !rtti::sane(graph) {
        return std::ptr::null_mut();
    }
    graph
}

pub(crate) unsafe extern "C" fn tramp_quest_root_graph(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let graph = quest_root_graph(sys);
    crate::log_dev!("[questgraph] BwmsQuestPhaseGetRootGraph: sys={sys:p} -> graph={graph:p}");
    write_handle_ret(out, graph as u64);
}

/// Percorre `graph.nodes@+0x30` contando/devolvendo o N-ésimo nó cuja CLASSE (via
/// `class_of`+`type_name_getname`, já provado dezenas de vezes neste projeto) bate `type_hash`.
/// `want_index<0` só conta; `want_index>=0` devolve o handle daquele índice dentre os matches
/// (mesma disciplina "Count+At(i)" já usada pra evitar risco de array-retorno noutros itens).
///
/// 2026-08-18: vira wrapper fino sobre `quest_array_nodes_of_type` (offset `0x30` fixo) — a
/// lógica de varredura foi generalizada pra offset ARBITRÁRIO (ver função abaixo), reusada
/// tanto pra `graph.nodes` quanto pra `SpawnManagerNodeDefinition.actions@+0x48` (item #160).
pub(crate) unsafe fn quest_graph_nodes_of_type(graph: *mut c_void, type_hash: u64, want_index: i32) -> (i32, *mut c_void) {
    quest_array_nodes_of_type(graph, 0x30, type_hash, want_index)
}

/// Generaliza `quest_graph_nodes_of_type` pra um offset de array ARBITRÁRIO (não mais fixo em
/// `+0x30`) — mesma disciplina (`DynArray<Handle<T>>`, stride 16 bytes: instância no 1º qword,
/// bloco de refcount no 2º; header padrão `DynArray` — `entries@+0x00`/`size@+0x0C`, mesmo
/// idioma já usado em toda leitura de `DynArray` deste projeto).
///
/// 2026-08-18 — acesso ao grafo de fase de quest, avanço offline sem endereço nativo
/// novo: busca com cast ANINHADO dentro de `node->actions`/`node->condition`/`node->type` (não só
/// os nós de um tipo direto, já coberto por `quest_graph_nodes_of_type`). RE offline
/// (layout da reflection real do jogo):
///
///   - `SpawnManagerNodeDefinition`: `actions:
///     DynArray<SpawnManagerNodeActionEntry>@+0x48`.
///   - `SpawnManagerNodeActionEntry`: struct de 0x10 bytes
///     que É o `Handle<SpawnManagerNodeType>` inteiro (offset 0 do entry) — MESMO shape de
///     `DynArray<Handle<T>>` já provado (stride 16, instância no 1º qword) — zero struct nova
///     de leitura necessária, só o offset do array muda.
///   - `FactsDBManagerNodeDefinition`: `type:
///     Handle<IFactsDBManagerNodeType>@+0x48` (campo ÚNICO, não array — usa
///     `quest_node_handle_field`, não este helper).
///   - `PauseConditionNodeDefinition`: `condition:
///     Handle<IBaseCondition>@+0x48` (idem, campo único).
///
/// Isso destrava, em conjunto com `quest_node_handle_field` (abaixo), a composição de
/// `FindCommunities`/`FindSpawnSets`/`FindSpawners` (via `node.actions@+0x48`, filtrando o
/// `.type` de cada entry pelo `type_hash` de `questCommunityTemplate_NodeType`/
/// `questSpawnSet_NodeType`/`questSpawner_NodeType`) e `FindFactChanges` (via
/// `node.type@+0x48` de `questFactsDBManagerNodeDefinition`, filtrando por
/// `questSetVar_NodeType`) — sem declarar NENHUM desses ~6 tipos concretos em redscript (mesma
/// disciplina anti-CMesh-trap já em uso: retorno sempre `ref<IScriptable>`, filtro por
/// `type_hash: CName` passado pelo mod-author). Um 2º nível de filtro
/// (`nodeType->action == Activate/Reactivate` em `questCommunityTemplate_NodeType`/etc.)
/// fica de fora — precisaria de mais 1 offset de campo por tipo concreto, não achado nesta
/// rodada; o mod-author pode reconstruir esse filtro em redscript se precisar, usando o handle
/// genérico devolvido aqui. Zero código Rust novo pros métodos que fazem Cast ANINHADO em 2
/// NÍVEIS (`FindFactConditions`/`FindJournalConditions`/`FindLootConditions`/
/// `FindLootContainerCondition`/`FindCharacterKillCondition`, que primeiro leem `.condition`
/// depois fazem Cast DE NOVO no resultado) — ficam pro próximo passo, mas `quest_node_handle_field`
/// composto 2x (uma vez pra achar `.condition`, outra pro Cast interno, ambos via o MESMO
/// generic reader, offset diferente por tipo) já reduz a distância pra "achar mais 1-2 offsets".
pub(crate) unsafe fn quest_array_nodes_of_type(base: *mut c_void, array_offset: i32, type_hash: u64, want_index: i32) -> (i32, *mut c_void) {
    let mut count: i32 = 0;
    let mut found: *mut c_void = std::ptr::null_mut();
    if !rtti::sane(base) || type_hash == 0 || array_offset < 0 || array_offset as usize > 0x2000 {
        return (0, std::ptr::null_mut());
    }
    let arr = (base as *mut u8).add(array_offset as usize) as *const c_void;
    if !crate::memoria::is_readable(arr, 0x10) {
        return (0, std::ptr::null_mut());
    }
    let entries = rd_u64(arr, 0x00) as *const u8;
    let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    if entries.is_null() || size > 100_000 {
        return (0, std::ptr::null_mut());
    }
    for i in 0..size as usize {
        let slot = entries.add(i * 16);
        if !crate::memoria::is_readable(slot as *const c_void, 8) {
            continue;
        }
        let inst = core::ptr::read_unaligned(slot as *const u64) as *mut c_void;
        if !rtti::sane(inst) {
            continue;
        }
        let cls = rtti::class_of(inst);
        if cls.is_null() {
            continue;
        }
        if rtti::type_name_getname(cls) == type_hash {
            if want_index >= 0 && count == want_index {
                found = inst;
            }
            count += 1;
        }
    }
    (count, found)
}

/// Lê um `Handle<T>` (instância no primeiro qword do par de 16 bytes) num offset ARBITRÁRIO de
/// um objeto native já resolvido (`base+offset`) — generaliza o padrão já usado em
/// `quest_root_graph` (offsets fixos `0x68`/`0x40`, hardcoded em 2 lugares) pra qualquer campo
/// `Handle<T>` documentado num header GERADO da reflection real, sem precisar de 1 função Rust
/// nova por campo. Cobre `questFactsDBManagerNodeDefinition.type@0x48`/
/// `questPauseConditionNodeDefinition.condition@0x48` (item #160, 2026-08-18, ver nota longa em
/// `quest_array_nodes_of_type`) e qualquer campo `Handle<T>` futuro do mesmo formato — mesma
/// disciplina anti-CMesh-trap (devolve sempre `ref<IScriptable>` genérico, filtro de classe fica
/// por conta do redscript via `GetClassName()`/`type_hash`).
pub(crate) unsafe fn quest_node_handle_field(base: *mut c_void, offset: i32) -> *mut c_void {
    if !rtti::sane(base) || offset < 0 || offset as usize > 0x2000 {
        return std::ptr::null_mut();
    }
    let field_ptr = (base as *mut u8).add(offset as usize) as *const c_void;
    if !crate::memoria::is_readable(field_ptr, 8) {
        return std::ptr::null_mut();
    }
    let inst = rd_u64(field_ptr, 0) as *mut c_void;
    if !rtti::sane(inst) {
        return std::ptr::null_mut();
    }
    inst
}

pub(crate) unsafe extern "C" fn tramp_quest_graph_node_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let graph = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let type_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let (count, _) = quest_graph_nodes_of_type(graph, type_hash, -1);
    crate::log_dev!(
        "[questgraph] BwmsQuestGraphNodeCount: graph={graph:p} type_hash={type_hash:#x} -> {count}"
    );
    write_i32_ret(out, count);
}

pub(crate) unsafe extern "C" fn tramp_quest_graph_node_at(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let graph = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let type_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let index = args.get(2).map(|(v, _)| *v as i32).unwrap_or(-1);
    let (_, found) = quest_graph_nodes_of_type(graph, type_hash, index);
    crate::log_dev!(
        "[questgraph] BwmsQuestGraphNodeAt: graph={graph:p} type_hash={type_hash:#x} idx={index} -> {found:p}"
    );
    write_handle_ret(out, found as u64);
}

/// `BwmsQuestNodeHandleField(node, offset) -> ref<IScriptable>` — wrapper redscript-callable de
/// `quest_node_handle_field` (2026-08-18, item `#160`). Genérico: o mod-author passa o offset
/// (ex. `0x48` pra `.condition`/`.type` de vários node-types já documentados na nota da função
/// Rust) — nenhuma declaração de tipo concreto necessária.
pub(crate) unsafe extern "C" fn tramp_quest_node_handle_field(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let base = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let offset = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let field = quest_node_handle_field(base, offset);
    crate::log_dev!(
        "[questgraph] BwmsQuestNodeHandleField: base={base:p} offset={offset:#x} -> {field:p}"
    );
    write_handle_ret(out, field as u64);
}

/// `BwmsQuestNodeArrayCount(node, offset, typeHash) -> Int32` — wrapper redscript-callable de
/// `quest_array_nodes_of_type` (offset ARBITRÁRIO, não mais fixo em `graph.nodes@+0x30`) —
/// cobre `SpawnManagerNodeDefinition.actions@+0x48` pra `FindCommunities`/`FindSpawnSets`/
/// `FindSpawners` (item `#160`, 2026-08-18).
pub(crate) unsafe extern "C" fn tramp_quest_node_array_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let base = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let offset = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let type_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let (count, _) = quest_array_nodes_of_type(base, offset, type_hash, -1);
    crate::log_dev!(
        "[questgraph] BwmsQuestNodeArrayCount: base={base:p} offset={offset:#x} type_hash={type_hash:#x} -> {count}"
    );
    write_i32_ret(out, count);
}

/// `BwmsQuestNodeArrayAt(node, offset, typeHash, index) -> ref<IScriptable>` — irmão do `Count`
/// acima, devolve o handle do índice N dentre os matches (mesma disciplina "Count+At(i)" já
/// usada pra evitar risco de array-retorno, item `#160`, 2026-08-18).
pub(crate) unsafe extern "C" fn tramp_quest_node_array_at(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let base = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let offset = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let type_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let index = args.get(3).map(|(v, _)| *v as i32).unwrap_or(-1);
    let (_, found) = quest_array_nodes_of_type(base, offset, type_hash, index);
    crate::log_dev!(
        "[questgraph] BwmsQuestNodeArrayAt: base={base:p} offset={offset:#x} type_hash={type_hash:#x} idx={index} -> {found:p}"
    );
    write_handle_ret(out, found as u64);
}

pub(crate) unsafe fn register_quest_graph_natives(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(
        reg,
        proto,
        &["handle:QuestsSystem"],
        "BwmsQuestPhaseGetRootGraph",
        "BwmsQuestPhaseGetRootGraph",
        tramp_quest_root_graph,
    );
    crate::log_dev!("[reg] register_all: questgraph BwmsQuestPhaseGetRootGraph={ok1}");
    let ok2 = register_argful_by_types(
        reg,
        proto,
        &["handle:IScriptable", "CName"],
        "BwmsQuestGraphNodeCount",
        "BwmsQuestGraphNodeCount",
        tramp_quest_graph_node_count,
    );
    crate::log_dev!("[reg] register_all: questgraph BwmsQuestGraphNodeCount={ok2}");
    let ok3 = register_argful_by_types(
        reg,
        proto,
        &["handle:IScriptable", "CName", "Int32"],
        "BwmsQuestGraphNodeAt",
        "BwmsQuestGraphNodeAt",
        tramp_quest_graph_node_at,
    );
    crate::log_dev!("[reg] register_all: questgraph BwmsQuestGraphNodeAt={ok3}");
    // 2026-08-18: 3 natives genéricas novas (ver nota longa em `quest_array_nodes_of_type`) —
    // destravam `FindCommunities`/`FindSpawnSets`/`FindSpawners`/`FindFactChanges` sem endereço
    // nativo novo, offset passado pelo redscript em vez de hardcoded por-campo.
    let ok4 = register_argful_by_types(
        reg,
        proto,
        &["handle:IScriptable", "Int32"],
        "BwmsQuestNodeHandleField",
        "BwmsQuestNodeHandleField",
        tramp_quest_node_handle_field,
    );
    crate::log_dev!("[reg] register_all: questgraph BwmsQuestNodeHandleField={ok4}");
    let ok5 = register_argful_by_types(
        reg,
        proto,
        &["handle:IScriptable", "Int32", "CName"],
        "BwmsQuestNodeArrayCount",
        "BwmsQuestNodeArrayCount",
        tramp_quest_node_array_count,
    );
    crate::log_dev!("[reg] register_all: questgraph BwmsQuestNodeArrayCount={ok5}");
    let ok6 = register_argful_by_types(
        reg,
        proto,
        &["handle:IScriptable", "Int32", "CName", "Int32"],
        "BwmsQuestNodeArrayAt",
        "BwmsQuestNodeArrayAt",
        tramp_quest_node_array_at,
    );
    crate::log_dev!("[reg] register_all: questgraph BwmsQuestNodeArrayAt={ok6}");
}

/// `id` do filho `indice` do container, lido em `+0x40` (vazio se não houver).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_filho_id(container: *mut c_void, indice: usize) -> String {
    journal_filhos(container)
        .get(indice)
        .copied()
        .filter(|f| rtti::sane(*f) && crate::memoria::is_readable(*f as *const c_void, JOURNAL_ENTRY_ID_OFF + 0x20))
        .and_then(|f| rtti::read_cstring((f as *const u8).add(JOURNAL_ENTRY_ID_OFF)))
        .unwrap_or_default()
}

/// `BwmsJournalChildCount(entry) -> Int32`: filhos no `entries` nativo; `-1` se não for container.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_child_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let n = journal_container_de(inst).map_or(-1, |c| journal_filhos(c).len() as i32);
    write_uint_ret(out, n as u32 as u64, 4);
}

/// `BwmsJournalChildId(entry, index) -> String`: `id` do filho `index` lido em `+0x40` (até 19 bytes; mais longo volta vazio).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_child_id(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let indice = args.get(1).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let id = journal_container_de(inst).map(|c| journal_filho_id(c, indice)).unwrap_or_default();
    write_cstring_inline_ret(out, &id);
}

/// `BwmsJournalResourceRootIsFolder(resource) -> Bool`: o root do `.journal` é um `gameJournalRootFolderEntry`.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_resource_root_is_folder(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let res = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let pasta = journal_resource_root(res).map_or(false, |root| {
        let alvo = crate::registry().map_or(std::ptr::null_mut(), |r| r.class_by_name("gameJournalRootFolderEntry"));
        !alvo.is_null() && class_is_kind_of(rtti::class_of(root), alvo)
    });
    write_bool_ret(out, pasta);
}

/// `BwmsJournalResourceRootChildCount(resource) -> Int32`: filhos do root do `.journal`; `-1` sem root container.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_resource_root_child_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let res = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let n = journal_resource_root(res).and_then(|r| journal_container_de(r)).map_or(-1, |c| journal_filhos(c).len() as i32);
    write_uint_ret(out, n as u32 as u64, 4);
}

/// `BwmsJournalResourceRootChildIdPart(resource, index, part) -> String`: pedaço `part` (16 caracteres) do `id` do filho `index` do root.
/// Id de 20+ bytes não cabe num retorno só; o reds junta os pedaços até vir vazio.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_resource_root_child_id_part(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let res = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let indice = args.get(1).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let parte = args.get(2).map(|(v, _)| *v as u32 as usize).unwrap_or(0).min(64);
    let id = journal_resource_root(res)
        .and_then(|r| journal_container_de(r))
        .map(|c| journal_filho_id(c, indice))
        .unwrap_or_default();
    write_cstring_chunk_ret(out, &id, parte);
}

// Journal de mod, parte 5b (2026-09-11): árvore VIVA do `JournalManager`, só leitura (RE da 5a, DATABASE): `JournalManager+0x58` = `gameJournalTree*`;
// no tree, `rootEntries` (`DynArray<Handle<gameJournalEntry>>`) em `+0x78` (2.5). Os índices moram no PRÓPRIO `JournalManager` (boot 48: lidos no tree
// deram 0 e lixo; o handler do `GetEntry` `0x1052547c0` chama `this->vt[0x220]` com `x0 = this`, e o `0x10524495c` usa `+0x68` lock, `+0x78` count):
// índice por hash (A) com count em `jm+0x78` e por entrada (B) em `jm+0xa8`. Critério: a raiz do `cooked_journal` e A == B > 0.
pub(crate) const JOURNALMANAGER_TREE_OFF: usize = 0x58;

/// Raiz `indice` como container, se existir.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_tree_raiz(jm: *mut c_void, indice: usize) -> Option<*mut c_void> {
    let tree = journal_tree_de(jm)?;
    journal_tree_raizes(tree).get(indice).copied().and_then(|r| journal_container_de(r))
}

/// `BwmsJournalTreeRootCount(jm) -> Int32`: raízes vivas; `-1` sem tree.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_tree_root_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let jm = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let n = journal_tree_de(jm).map_or(-1, |t| journal_tree_raizes(t).len() as i32);
    write_uint_ret(out, n as u32 as u64, 4);
}

/// `BwmsJournalTreeRootChildCount(jm, raiz) -> Int32`: filhos da raiz viva; `-1` se não houver.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_tree_root_child_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let jm = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let raiz = args.get(1).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let n = journal_tree_raiz(jm, raiz).map_or(-1, |c| journal_filhos(c).len() as i32);
    write_uint_ret(out, n as u32 as u64, 4);
}

/// `BwmsJournalTreeRootChildIdPart(jm, raiz, filho, parte) -> String`: pedaço de 16 caracteres do `id` do filho.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_tree_root_child_id_part(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let jm = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let raiz = args.get(1).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let filho = args.get(2).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let parte = args.get(3).map(|(v, _)| *v as u32 as usize).unwrap_or(0).min(64);
    let id = journal_tree_raiz(jm, raiz).map(|c| journal_filho_id(c, filho)).unwrap_or_default();
    write_cstring_chunk_ret(out, &id, parte);
}

/// `BwmsJournalTreeIndexCount(jm, qual) -> Int32`: entradas no índice por hash (`qual` 0) ou por entrada (`qual` 1); `-1` inválido.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_tree_index_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let jm = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let qual = args.get(1).map(|(v, _)| *v as u32).unwrap_or(u32::MAX);
    let off = match qual {
        0 => Some(JOURNALTREE_INDICE_HASH_COUNT_OFF),
        1 => Some(JOURNALTREE_INDICE_ENTRADA_COUNT_OFF),
        _ => None,
    };
    let n = match (journal_tree_de(jm), off) {
        (Some(_), Some(o)) if crate::memoria::is_readable(jm as *const c_void, o + 4) => ((jm as *const u8).add(o) as *const u32).read_unaligned() as i32,
        _ => -1,
    };
    write_uint_ret(out, n as u32 as u64, 4);
}

// Fase de quest, parte 35c-2d-3b (2026-09-12, RE offline da fila do `QuestResourceLoader`): o boot 71 mostrou
// que a rota de carga só roda pra fase pedida ao `QuestResourceLoader`. Aqui a fase vai pro loader do QuestsSystem (`+0x198`, vptr `0x107a2e338`) pelo
// `LoadOne` com functor vazio: carga pelo depot, conclusão em job, rota (onde o hook aplica o patch da tabela) e entrada no mapa do loader com contagem 1 —
// sem mexer nas raízes, sem ligar `phaseGraph`, sem callback do sistema e sem save. Todo slot chamado tem o endereço conferido contra a RE antes.
// Desfazer: o patch primeiro (ação 13), depois `Release`.
#[cfg(feature = "devtools")]
pub(crate) const QUESTSSYSTEM_LOADER_OFF: usize = 0x198;

#[cfg(feature = "devtools")]
pub(crate) const QUESTLOADER_VPTR_VM: u64 = 0x1_07a2_e338;

// O slot guarda `0x1029e88ac` (lido do vtable no arquivo); a RE deu `0x1029e88b0`, 4 B depois da entrada — a guarda recusou o `LoadOne` no boot 73.
#[cfg(feature = "devtools")]
pub(crate) const QUESTLOADER_LOADONE: (usize, u64) = (0x38, 0x1_029e_88ac);

#[cfg(feature = "devtools")]
pub(crate) const QUESTLOADER_RELEASE: (usize, u64) = (0x40, 0x1_029e_8a98);

#[cfg(feature = "devtools")]
pub(crate) const QUESTLOADER_GETRESOURCE: (usize, u64) = (0x48, 0x1_029e_8e18);

#[cfg(feature = "devtools")]
pub(crate) const QUESTLOADER_ISFAILED: (usize, u64) = (0x50, 0x1_029e_8ef4);

#[cfg(feature = "devtools")]
pub(crate) const QUESTPHASE_RAIZ_HASH: u64 = 0x8464_2c97_f5c7_1116;

#[cfg(feature = "devtools")]
pub(crate) const QUESTPHASE_EPILOGUE_HASH: u64 = 0x9e3d_6e99_8707_ecaa;

#[cfg(feature = "devtools")]
pub(crate) const QUESTPHASE_INEXISTENTE: &str = "mod\\bwms\\teste\\fase_35_nao_existe.questphase";

/// Loader do QuestsSystem conferido pelo vptr (lido em `+desloc`, pro controle).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questloader_de(qs: *mut c_void, desloc: usize) -> Option<*mut c_void> {
    if !rtti::sane(qs) || !crate::memoria::is_readable(qs as *const c_void, QUESTSSYSTEM_LOADER_OFF + 8) {
        return None;
    }
    let l = rd_u64(qs as *const c_void, QUESTSSYSTEM_LOADER_OFF) as *mut c_void;
    if !rtti::sane(l) || !crate::memoria::is_readable(l as *const c_void, 0x88) {
        return None;
    }
    (rd_u64(l as *const c_void, desloc) == crate::rebase(QUESTLOADER_VPTR_VM) as u64).then_some(l)
}

/// Path do teste: 0 epilogue, 1 fase raiz, 2 path que não existe.
#[cfg(feature = "devtools")]
pub(crate) fn questloader_path(kind: i32) -> Option<u64> {
    match kind {
        0 => Some(QUESTPHASE_EPILOGUE_HASH),
        1 => Some(QUESTPHASE_RAIZ_HASH),
        2 => Some(bwms_primitives::resource_path_hash(QUESTPHASE_INEXISTENTE)),
        _ => None,
    }
}

/// `GetResource(path)` do loader: Handle forte por sret (`x8`); `[0, 0]` sem entrada ou com o slot diferente da RE.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questloader_recurso(l: *mut c_void, path: u64) -> [u64; 2] {
    let mut out = [0u64; 2];
    let Some(f) = questloader_slot(l, QUESTLOADER_GETRESOURCE) else { return out };
    core::arch::asm!(
        "blr {f}",
        f = in(reg) f,
        in("x0") l as u64,
        in("x1") path,
        in("x8") out.as_mut_ptr(),
        clobber_abi("C"),
    );
    out
}

/// `BwmsQuestLoaderOp(questsSystem, op, kind) -> Int32`: 0 loader com o vptr da RE (1/0), 1 o mesmo lendo o vptr 8 B deslocado (controle), 2 `GetResource`
/// não nulo (1/0), 3 `LoadOne` com functor vazio (1 pedido; -3 já no mapa), 4 `IsFailed` (1/0), 5 `Release` (1 chamado); -1 sem loader, -2 slot diferente
/// da RE ou `kind`/`op` inválido.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn questloader_op(qs: *mut c_void, op: i32, kind: i32) -> i32 {
    if op == 1 {
        return questloader_de(qs, 8).is_some() as i32;
    }
    let Some(l) = questloader_de(qs, 0) else { return if op == 0 { 0 } else { -1 } };
    let Some(path) = questloader_path(kind) else { return -2 };
    match op {
        0 => 1,
        2 => {
            let mut h = questloader_recurso(l, path);
            let achou = h[0] != 0;
            if achou {
                let _ = crate::jogo::objetos::soltar_handle_contando(&mut h);
            }
            achou as i32
        }
        3 => {
            let mut h = questloader_recurso(l, path);
            if h[0] != 0 {
                let _ = crate::jogo::objetos::soltar_handle_contando(&mut h);
                return -3;
            }
            let Some(f) = questloader_slot(l, QUESTLOADER_LOADONE) else { return -2 };
            let functor = [0u8; 0x28];
            let load_one: unsafe extern "C" fn(*mut c_void, u64, *const u8) = std::mem::transmute(f);
            load_one(l, path, functor.as_ptr());
            crate::veredito_dev!("[a35-fila] LoadOne({path:#018x}) pedido ao loader {l:p}");
            1
        }
        4 => {
            let Some(f) = questloader_slot(l, QUESTLOADER_ISFAILED) else { return -2 };
            let is_failed: unsafe extern "C" fn(*mut c_void, u64) -> u8 = std::mem::transmute(f);
            (is_failed(l, path) != 0) as i32
        }
        5 => {
            let Some(f) = questloader_slot(l, QUESTLOADER_RELEASE) else { return -2 };
            let release: unsafe extern "C" fn(*mut c_void, u64) = std::mem::transmute(f);
            release(l, path);
            crate::veredito_dev!("[a35-fila] Release({path:#018x}) no loader {l:p}");
            1
        }
        _ => -2,
    }
}

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_questloader_op(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let qs = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let op = args.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let kind = args.get(2).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    write_uint_ret(out, questloader_op(qs, op, kind) as u32 as u64, 4);
}

/// `BwmsQuestLoaderResource(questsSystem, kind) -> ref<IScriptable>`: o recurso da fase no mapa do loader (Handle forte entregue ao script); nulo sem
/// entrada.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_questloader_resource(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let qs = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let kind = args.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let mut h = match (questloader_de(qs, 0), questloader_path(kind)) {
        (Some(l), Some(p)) => questloader_recurso(l, p),
        _ => [0u64; 2],
    };
    // O recurso sobe dentro de um `BwmsSerializable`; a referência forte que veio do loader é solta depois da cópia.
    let mut s = recurso_para_script(h);
    crate::jogo::objetos::soltar_handle(&mut h);
    if out.is_null() {
        crate::jogo::objetos::soltar_handle(&mut s);
    } else {
        entregar_handle(out, s);
    }
}

/// `BwmsQuestGraphStartNode(resource) -> Int32`: id do nó que o `FindConnectionPoint` acha com caminho vazio (1º `questStartNodeDefinition` do grafo,
/// pelo vptr); -11 sem nó de início, -1 sem grafo. O grafo sai do campo `graph` pelo RTTI (game thread), então serve pra `.quest` e `.questphase`.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_quest_graph_start_node(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let res = args.first().map(|(v, _)| *v as *const u8).unwrap_or(std::ptr::null());
    let grafo = if rtti::sane(res as *mut c_void) { acessor_handle(res, "graph") } else { None };
    let r = match grafo {
        None => -1,
        Some(g) => fase_ponto_de_conexao(g, &[]).map_or(-11, |(_, n)| (n.add(QUESTNODE_ID_OFF) as *const u16).read_unaligned() as i32),
    };
    write_uint_ret(out, r as u32 as u64, 4);
}

/// `BwmsJournalPathHash(path) -> Int32`: o hash que o `GetEntryHash` vanilla devolveria pra esse path.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_path_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_args, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    write_uint_ret(out, bwms_primitives::murmur3_32(path.as_bytes(), bwms_primitives::RECORDS_SEED) as u64, 4);
}

/// Entrada `pasta` da raiz viva `raiz` e, com `filho >= 0`, o filho dela (instâncias cruas).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_tree_entrada(jm: *mut c_void, raiz: usize, pasta: usize, filho: i32) -> Option<*mut c_void> {
    let p = journal_tree_raiz(jm, raiz).and_then(|r| journal_filhos(r).get(pasta).copied()).filter(|p| rtti::sane(*p))?;
    if filho < 0 {
        return Some(p);
    }
    journal_container_de(p).and_then(|c| journal_filhos(c).get(filho as usize).copied()).filter(|f| rtti::sane(*f))
}

/// `BwmsJournalTreeFolderChildCount(jm, raiz, pasta) -> Int32`: filhos da pasta `pasta` da raiz viva; `-1` se não for container.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_tree_folder_child_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let jm = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let raiz = args.get(1).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let pasta = args.get(2).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let n = journal_tree_entrada(jm, raiz, pasta, -1).and_then(|p| journal_container_de(p)).map_or(-1, |c| journal_filhos(c).len() as i32);
    write_uint_ret(out, n as u32 as u64, 4);
}

/// `BwmsJournalTreeFolderChildIdPart(jm, raiz, pasta, filho, parte) -> String`: pedaço `parte` (19 bytes) do id do filho; vazio no fim.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_tree_folder_child_id_part(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let jm = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let raiz = args.get(1).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let pasta = args.get(2).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let filho = args.get(3).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let parte = args.get(4).map(|(v, _)| *v as u32 as usize).unwrap_or(0).min(64);
    let id = journal_tree_entrada(jm, raiz, pasta, -1)
        .and_then(|p| journal_container_de(p))
        .map(|c| journal_filho_id(c, filho))
        .unwrap_or_default();
    write_cstring_chunk_ret(out, &id, parte);
}

/// `BwmsJournalTreeClassName(jm, raiz, pasta, filho) -> CName`: classe da pasta (`filho` = -1) ou do filho; `None` se não houver.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_tree_class_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let jm = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let raiz = args.get(1).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let pasta = args.get(2).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let filho = args.get(3).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let nome = journal_tree_entrada(jm, raiz, pasta, filho).map_or(0, |e| rtti::type_name_hash(rtti::class_of(e)));
    write_uint_ret(out, nome, 8);
}

/// `BwmsJournalMoverGotOk() -> Bool`: o GOT do mover aponta pro `MoveAfterReallocation` de `DynArray<Handle<JournalEntry>>`.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_mover_got_ok(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let _ = rtti::read_params_consuming(current_native_func(), frame);
    let got = crate::rebase(JOURNAL_ENTRY_MOVER_GOT_VM) as *const u64;
    let ok = crate::memoria::is_readable(got as *const c_void, 8) && got.read_unaligned() == crate::rebase(JOURNAL_ENTRY_MOVER_VM) as u64;
    write_bool_ret(out, ok);
}

#[cfg(feature = "devtools")]
pub(crate) const JOURNALTREE_PAI_REMOVE_VM: u64 = 0x1_0523_2fcc;

pub(crate) const JOURNAL_CONTAINER_TRAVA_OFF: usize = 0x70;

#[cfg(feature = "devtools")]
pub(crate) const JOURNAL_TESTE_PASTA: &str = "onscreens";

#[cfg(feature = "devtools")]
pub(crate) const JOURNAL_TESTE_ID: &str = "bwms_teste_5c";

/// Entre inserir e desfazer: `(ponteiro do dono vazado, pasta)`.
#[cfg(feature = "devtools")]
pub(crate) static JOURNAL_ENTRADA_TESTE: std::sync::Mutex<Option<(usize, usize)>> = std::sync::Mutex::new(None);

/// Trava de byte do container (o motor troca o byte por 1 e cede a vez depois de 16 voltas).
pub(crate) unsafe fn journal_trava(container: *mut c_void) {
    let t = &*((container as *const u8).add(JOURNAL_CONTAINER_TRAVA_OFF) as *const std::sync::atomic::AtomicU8);
    let mut voltas = 0u32;
    while t.swap(1, std::sync::atomic::Ordering::SeqCst) != 0 {
        voltas += 1;
        if voltas >= 16 {
            std::thread::yield_now();
        }
    }
}

pub(crate) unsafe fn journal_solta(container: *mut c_void) {
    (*((container as *const u8).add(JOURNAL_CONTAINER_TRAVA_OFF) as *const std::sync::atomic::AtomicU8)).store(0, std::sync::atomic::Ordering::SeqCst);
}

/// `(índice, handle)` do filho com esse id nos `entries` do container.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_filho_por_id(container: *mut c_void, id: &str) -> Option<(usize, [u64; 2])> {
    let base = (container as *const u8).add(JOURNAL_CONTAINER_ENTRIES_OFF);
    let ptr = (base as *const u64).read_unaligned() as *const u64;
    let n = (base.add(0x0C) as *const u32).read_unaligned() as usize;
    if ptr.is_null() || n > 4096 || !crate::memoria::is_readable(ptr as *const c_void, n * 16) {
        return None;
    }
    (0..n).find(|i| journal_filho_id(container, *i) == id).map(|i| (i, [ptr.add(i * 2).read_unaligned(), ptr.add(i * 2 + 1).read_unaligned()]))
}

/// Insere a entrada de teste. 1 = ok; recusas antes de qualquer efeito: -1 jm/tree, -2 pasta, -3 id já existe, -4 mover no GOT, -5 slots do
/// `JournalManager`, -6 objeto, -7 reserve; -8 já há entrada de teste inserida.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_teste_inserir(jm: *mut c_void) -> i32 {
    let Ok(mut estado) = JOURNAL_ENTRADA_TESTE.lock() else { return -8 };
    if estado.is_some() {
        return -8;
    }
    let Some(tree) = journal_tree_de(jm) else { return -1 };
    let Some(raiz) = journal_tree_raiz(jm, 0) else { return -1 };
    let Some((_, pasta_h)) = journal_filho_por_id(raiz, JOURNAL_TESTE_PASTA) else { return -2 };
    let Some(pasta) = journal_container_de(pasta_h[0] as *mut c_void) else { return -2 };
    if journal_filho_por_id(pasta, JOURNAL_TESTE_ID).is_some() {
        return -3;
    }
    let mover = (crate::rebase(JOURNAL_ENTRY_MOVER_GOT_VM) as *const u64).read_unaligned();
    if mover != crate::rebase(JOURNAL_ENTRY_MOVER_VM) as u64 {
        return -4;
    }
    let vt = (jm as *const u64).read_unaligned();
    let slot = |off: u64| ((vt + off) as *const u64).read_unaligned();
    if slot(0x1e0) != crate::rebase(JOURNALMANAGER_REGISTER_VM) as u64 || slot(0x1e8) != crate::rebase(JOURNALMANAGER_UNREGISTER_VM) as u64 {
        return -5;
    }
    let Some(reg) = rtti::Registry::obtain() else { return -6 };
    let obj = rtti::new_object(&reg, "gameJournalOnscreen");
    if obj.is_null() || !rtti::red_string_write_inline((obj as *mut u8).add(JOURNAL_ENTRY_ID_OFF), JOURNAL_TESTE_ID) {
        return -6;
    }
    let dono: *mut [u64; 2] = Box::into_raw(Box::new([0u64; 2]));
    rtti::make_handle(dono as *mut c_void, obj);
    let r = journal_anexar(jm, tree, pasta, pasta_h, dono);
    if r != 1 {
        return r;
    }
    *estado = Some((dono as usize, pasta as usize));
    1
}

/// Desfaz a entrada de teste: 1 = ok; -1 nada inserido; -2 jm/tree inválido; -3 entrada não achada nos `entries` da pasta.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_teste_desfazer(jm: *mut c_void) -> i32 {
    let Ok(mut estado) = JOURNAL_ENTRADA_TESTE.lock() else { return -1 };
    let Some((dono_u, pasta_u)) = *estado else { return -1 };
    let Some(tree) = journal_tree_de(jm) else { return -2 };
    let dono = dono_u as *mut [u64; 2];
    let pasta = pasta_u as *mut c_void;
    let r = journal_desanexar(jm, tree, pasta, dono);
    *estado = None;
    r
}

/// Desfaz o `journal_anexar`: índices A/B pelo slot `0x1e8`, vínculo filho→pai e o elemento dos `entries` da pasta (solta a cópia do array). 1 = ok;
/// -3 entrada não achada nos `entries`. O strong de `dono` fica com quem chama.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_desanexar(jm: *mut c_void, tree: *mut c_void, pasta: *mut c_void, dono: *mut [u64; 2]) -> i32 {
    let vt = (jm as *const u64).read_unaligned();
    let unregister: unsafe extern "C" fn(*mut c_void, *mut [u64; 2]) = std::mem::transmute(((vt + 0x1e8) as *const u64).read_unaligned() as usize);
    unregister(jm, dono);
    let remove: unsafe extern "C" fn(*mut c_void, *mut [u64; 2]) = std::mem::transmute(crate::rebase(JOURNALTREE_PAI_REMOVE_VM));
    remove(tree, dono);
    let arr = (pasta as *mut u8).add(JOURNAL_CONTAINER_ENTRIES_OFF);
    journal_trava(pasta);
    let ptr = (arr as *const u64).read_unaligned() as *mut u64;
    let tam = (arr.add(0x0C) as *const u32).read_unaligned() as usize;
    let obj = (*dono)[0];
    let r = match (0..tam).find(|i| ptr.add(i * 2).read_unaligned() == obj) {
        Some(i) => {
            let copia = [ptr.add(i * 2).read_unaligned(), ptr.add(i * 2 + 1).read_unaligned()];
            std::ptr::copy(ptr.add((i + 1) * 2), ptr.add(i * 2), (tam - i - 1) * 2);
            (arr.add(0x0C) as *mut u32).write_unaligned((tam - 1) as u32);
            handle_strong_soma(&copia, -1);
            1
        }
        None => -3,
    };
    journal_solta(pasta);
    r
}

// Journal de mod, parte 5d-2 (2026-09-12): 1 entrada vinda de um `.journal` carregado entra na árvore viva. A 5d-1 mostrou que a cópia carregada do
// `cooked_journal` não divide objeto com a árvore viva, então a entrada de teste sai dela: num grupo de `onscreens` da cópia que também existe na árvore
// viva, a 1ª folha com id de até 19 bytes ganha o id `bwms_teste_5d` (inline; o original volta no desfazer), a mescla a seco do grupo confirma que só
// ela falta, e ela passa pelo mesmo `journal_anexar` da 5c-2. Nada notifica e nada vai pro save.
#[cfg(feature = "devtools")]
pub(crate) const JOURNAL_MESCLA_TESTE_ID: &str = "bwms_teste_5d";

#[cfg(feature = "devtools")]
pub(crate) struct MesclaTeste {
    pub(crate) dono: usize,
    pub(crate) pasta: usize,
    pub(crate) obj: usize,
    pub(crate) id_original: String,
    pub(crate) caminho: String,
}

#[cfg(feature = "devtools")]
pub(crate) static JOURNAL_MESCLA_TESTE: std::sync::Mutex<Option<MesclaTeste>> = std::sync::Mutex::new(None);

/// Na pasta `JOURNAL_TESTE_PASTA` da cópia, o 1º grupo que existe na árvore viva com uma folha de id até 19 bytes:
/// `(id do grupo, grupo vivo, handle do grupo vivo, grupo da cópia, handle da folha, id da folha)`.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_mescla_escolher(mod_raiz: *mut c_void, raiz: *mut c_void) -> Option<(String, *mut c_void, [u64; 2], *mut c_void, [u64; 2], String)> {
    let (_, viva_h) = journal_filho_por_id(raiz, JOURNAL_TESTE_PASTA)?;
    let viva = journal_container_de(viva_h[0] as *mut c_void)?;
    let (_, copia_h) = journal_filho_por_id(mod_raiz, JOURNAL_TESTE_PASTA)?;
    let copia = journal_container_de(copia_h[0] as *mut c_void)?;
    let mut grupos: Vec<(String, [u64; 2])> = journal_ids_mapa(copia).into_iter().collect();
    grupos.sort();
    for (gid, gh) in grupos {
        let Some(mgrupo) = journal_container_de(gh[0] as *mut c_void) else { continue };
        let Some((_, grupo_h)) = journal_filho_por_id(viva, &gid) else { continue };
        let Some(grupo) = journal_container_de(grupo_h[0] as *mut c_void) else { continue };
        let mut folhas: Vec<(String, [u64; 2])> = journal_ids_mapa(mgrupo).into_iter().collect();
        folhas.sort();
        if let Some((fid, fh)) = folhas.into_iter().find(|(fid, fh)| !fid.is_empty() && fid.len() <= 19 && journal_container_de(fh[0] as *mut c_void).is_none()) {
            return Some((gid, grupo, grupo_h, mgrupo, fh, fid));
        }
    }
    None
}

/// Insere a entrada vinda da cópia. 1 = ok; recusas antes de qualquer efeito: -1 jm/tree, -3 recurso sem root, -4 cópia é a árvore viva, -5 sem
/// folha, -6 id novo já existe, -9 mover no GOT, -10 slots do `JournalManager`, -11 troca do id, -12 mescla a seco não deu 1 faltando; -7 reserve; -8
/// já há entrada inserida.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_mescla_inserir(jm: *mut c_void, res: *mut c_void) -> i32 {
    let Ok(mut estado) = JOURNAL_MESCLA_TESTE.lock() else { return -8 };
    if estado.is_some() {
        return -8;
    }
    let Some(tree) = journal_tree_de(jm) else { return -1 };
    let Some(raiz) = journal_tree_raiz(jm, 0) else { return -1 };
    let Some(mod_raiz) = journal_resource_root(res).and_then(|r| journal_container_de(r)) else { return -3 };
    if mod_raiz == raiz {
        return -4;
    }
    let Some((gid, pasta, pasta_h, mpasta, folha, id_original)) = journal_mescla_escolher(mod_raiz, raiz) else { return -5 };
    if journal_filho_por_id(pasta, JOURNAL_MESCLA_TESTE_ID).is_some() {
        return -6;
    }
    let mover = (crate::rebase(JOURNAL_ENTRY_MOVER_GOT_VM) as *const u64).read_unaligned();
    if mover != crate::rebase(JOURNAL_ENTRY_MOVER_VM) as u64 {
        return -9;
    }
    let vt = (jm as *const u64).read_unaligned();
    let slot = |off: u64| ((vt + off) as *const u64).read_unaligned();
    if slot(0x1e0) != crate::rebase(JOURNALMANAGER_REGISTER_VM) as u64 || slot(0x1e8) != crate::rebase(JOURNALMANAGER_UNREGISTER_VM) as u64 {
        return -10;
    }
    let obj = folha[0] as *mut u8;
    if !rtti::red_string_write_inline(obj.add(JOURNAL_ENTRY_ID_OFF), JOURNAL_MESCLA_TESTE_ID) {
        return -11;
    }
    let mut acc = [0i32; 4];
    journal_mescla_seco(mpasta, pasta, &format!("{JOURNAL_TESTE_PASTA}/{gid}"), 15, &mut acc);
    if acc[1] != 1 {
        let _ = rtti::red_string_write_inline(obj.add(JOURNAL_ENTRY_ID_OFF), &id_original);
        return -12;
    }
    let dono: *mut [u64; 2] = Box::into_raw(Box::new(folha));
    handle_strong_soma(&*dono, 1);
    let r = journal_anexar(jm, tree, pasta, pasta_h, dono);
    if r != 1 {
        handle_strong_soma(&*dono, -1);
        drop(Box::from_raw(dono));
        let _ = rtti::red_string_write_inline(obj.add(JOURNAL_ENTRY_ID_OFF), &id_original);
        return r;
    }
    let caminho = format!("{JOURNAL_TESTE_PASTA}/{gid}/{JOURNAL_MESCLA_TESTE_ID}");
    crate::veredito_dev!("[a44-mescla2] inserida {caminho} (id original '{id_original}', {} irmãos casados no grupo)", acc[0]);
    *estado = Some(MesclaTeste { dono: dono as usize, pasta: pasta as usize, obj: obj as usize, id_original, caminho });
    1
}

/// Desfaz a 5d-2: tira a entrada da árvore viva, solta o strong da native e devolve o id original na cópia. 1 = ok; -1 nada inserido; -2 jm/tree;
/// -3 não achada nos `entries`; -13 id original não voltou.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_mescla_desfazer(jm: *mut c_void) -> i32 {
    let Ok(mut estado) = JOURNAL_MESCLA_TESTE.lock() else { return -1 };
    let Some(t) = estado.take() else { return -1 };
    let Some(tree) = journal_tree_de(jm) else {
        *estado = Some(t);
        return -2;
    };
    let dono = t.dono as *mut [u64; 2];
    let r = journal_desanexar(jm, tree, t.pasta as *mut c_void, dono);
    handle_strong_soma(&*dono, -1);
    drop(Box::from_raw(dono));
    let obj = t.obj as *mut u8;
    let volta = rtti::red_string_write_inline(obj.add(JOURNAL_ENTRY_ID_OFF), &t.id_original)
        && rtti::read_cstring(obj.add(JOURNAL_ENTRY_ID_OFF)).as_deref() == Some(t.id_original.as_str());
    crate::veredito_dev!("[a44-mescla2] desfeita {} -> {r}, id original de volta={volta}", t.caminho);
    if r == 1 && !volta { -13 } else { r }
}

/// `BwmsJournalMergeTestEntry(jm, resource, action) -> Int32`: `action` 1 insere a entrada vinda da cópia carregada, 2 desfaz.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_merge_test_entry(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let jm = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let res = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let acao = args.get(2).map(|(v, _)| *v as u32 as i32).unwrap_or(0);
    let r = match acao {
        1 => journal_mescla_inserir(jm, res),
        2 => journal_mescla_desfazer(jm),
        _ => 0,
    };
    crate::veredito_dev!("[a44-mescla2] BwmsJournalMergeTestEntry ação {acao} -> {r}");
    write_uint_ret(out, r as u32 as u64, 4);
}

/// `BwmsJournalMergeTestPathPart(part) -> String`: pedaço do caminho da entrada inserida pela 5d-2; vazio sem entrada.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_merge_test_path_part(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let parte = args.first().map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let caminho = JOURNAL_MESCLA_TESTE.lock().ok().and_then(|g| g.as_ref().map(|t| t.caminho.clone())).unwrap_or_default();
    let _ = write_cstring_chunk_ret(out, &caminho, parte);
}

/// `BwmsJournalMergeTestClass() -> CName`: classe da entrada inserida pela 5d-2; `None` sem entrada.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_merge_test_class(_c: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let obj = JOURNAL_MESCLA_TESTE.lock().ok().and_then(|g| g.as_ref().map(|t| t.obj)).unwrap_or(0);
    let h = if obj == 0 { 0 } else { rtti::type_name_hash(rtti::class_of(obj as *mut c_void)) };
    write_uint_ret(out, h, 8);
}

/// Códigos 12/13/15 do teste da 7e-4 no pin vivo: offset igual ao editado, offset igual ao original, id igual ao original sem a marca (1/0; -1 sem vivo).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn teste_edicao_codigo(t: &TesteEdicao, what: i32) -> i32 {
    let vivo = t.vivo as *const u8;
    if vivo.is_null() || !crate::memoria::is_readable(vivo as *const c_void, JOURNAL_QUESTMAPPIN_REFERENCE_OFF + 0x20) {
        return -1;
    }
    let perto = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 0.001);
    let offset = f32x3(vivo, JOURNAL_PINBASE_OFFSET_OFF);
    let v = match what {
        12 => perto(offset, t.editado_offset),
        13 => perto(offset, t.original),
        _ => rtti::read_cstring(vivo.add(JOURNAL_ENTRY_ID_OFF)).as_deref() == Some(t.editado_id.as_str()),
    };
    v as i32
}

/// `BwmsJournalModStat(what) -> Int32`: 0 journals na tabela, 1 menor estado (2 = todos mesclados), 2 casadas, 3 anexadas, 4 entradas novas, 5 pins
/// novos, 6 LocKeys convertidos, 7 entrada de teste renomeada (1/0), 8 pins ainda pendentes, 9 pins registrados; 7e-4: 10 entradas editadas, 11 edições
/// recusadas, 12/13/15 teste da edição no pin vivo (offset igual ao editado, offset igual ao original, id igual ao original sem a marca; 1/0, -1 sem
/// teste), 14 `Assign` dos tipos (0 não conferido, 1 conferido, 2 falhou); 16 journal mesclado no `Initialize` da raiz (7e-6b-2, 1/0).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_mod_stat(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let what = args.first().map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let v = match what {
        8 => PINS_DE_MOD.lock().map_or(-9, |p| p.len() as i32),
        9 => PINS_DE_MOD_REGISTRADOS.lock().map_or(-9, |p| p.len() as i32),
        14 => ASSIGN_CONFERIDO.load(std::sync::atomic::Ordering::Relaxed) as i32,
        _ => JOURNAL_MODS.lock().map_or(-9, |g| {
            let soma = |f: &dyn Fn(&JournalDeMod) -> usize| g.iter().map(f).sum::<usize>() as i32;
            match what {
                0 => g.len() as i32,
                1 => g.iter().map(|j| j.estado).min().unwrap_or(-99),
                2 => soma(&|j| j.casadas),
                3 => soma(&|j| j.anexadas.len()),
                4 => soma(&|j| j.entradas_novas),
                5 => soma(&|j| j.pins_novos),
                6 => soma(&|j| j.lockeys),
                7 => g.iter().any(|j| j.teste_renomeado.is_some()) as i32,
                10 => soma(&|j| j.editadas.len()),
                11 => soma(&|j| j.edicoes_recusadas),
                12 | 13 | 15 => g.iter().find_map(|j| j.teste_edicao.as_ref()).map_or(-1, |t| teste_edicao_codigo(t, what)),
                16 => g.iter().any(|j| j.na_raiz) as i32,
                _ => -1,
            }
        }),
    };
    write_uint_ret(out, v as u32 as u64, 4);
}

/// `BwmsJournalModTestPinHash(index, novo) -> Int32`: Murmur3 do caminho do pin `index` do objetivo de teste com o id renomeado (`novo` 1) ou original (0);
/// com `index` -1, quantos pins o objetivo tem; 0 fora do alcance.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_mod_test_pin_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let indice = args.first().map(|(v, _)| *v as u32 as i32).unwrap_or(-2);
    let novo = args.get(1).map_or(false, |(v, _)| *v as u32 != 0);
    let v = JOURNAL_7E_TESTE_PINS.lock().map_or(0, |t| {
        if indice == -1 {
            return t.len() as u32;
        }
        usize::try_from(indice)
            .ok()
            .and_then(|i| t.get(i))
            .map_or(0, |(n, o)| bwms_primitives::murmur3_32(if novo { n } else { o }.as_bytes(), bwms_primitives::RECORDS_SEED))
    });
    write_uint_ret(out, v as u64, 4);
}

/// `BwmsJournalTestEntry(jm, acao) -> Int32`: `acao` 1 insere a entrada de teste, 2 desfaz (códigos em `journal_teste_inserir`/`_desfazer`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_test_entry(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let jm = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let acao = args.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(0);
    let r = match acao {
        1 => journal_teste_inserir(jm),
        2 => journal_teste_desfazer(jm),
        _ => 0,
    };
    crate::veredito_dev!("[a44-registro] BwmsJournalTestEntry ação {acao} -> {r}");
    write_uint_ret(out, r as u32 as u64, 4);
}

#[cfg(feature = "devtools")]
pub(crate) const PIN_TESTE_OFFSET: &str = "quests/minor_quest/mq018_writer/mq018_writer/13d_really_interact/fortuneteller";

#[cfg(feature = "devtools")]
pub(crate) const PIN_TESTE_MAPPIN: (&str, u32) = ("quests/main_quest/act_01/q115_rogues_last_flight/05_atrium/00_get_down/down", 0x8d70_666e);

#[cfg(feature = "devtools")]
pub(crate) const PIN_TESTE_POI: (&str, u32) = ("points_of_interest/safehouses/v_room", 0xae94_6549);

#[derive(Clone, Copy, PartialEq)]
#[cfg(feature = "devtools")]
pub(crate) enum FamiliaPin {
    Pin,
    Multi,
    QuestPoi,
    Poi,
}

#[cfg(feature = "devtools")]
pub(crate) struct PinDeJournal {
    pub(crate) caminho: String,
    pub(crate) hash: u32,
    pub(crate) familia: FamiliaPin,
    pub(crate) entrada: usize,
    pub(crate) noderef: u64,
    pub(crate) dinamica: u64,
    pub(crate) offset: [f32; 3],
}

#[cfg(feature = "devtools")]
pub(crate) struct ColetaPins {
    pub(crate) pins: Vec<PinDeJournal>,
    pub(crate) entradas: usize,
    pub(crate) hashes: usize,
    pub(crate) colisoes: usize,
    /// `offset` de uma instância nova de `gameJournalQuestMapPin` (padrão da classe).
    pub(crate) padrao_offset: Option<[f32; 3]>,
}

/// Última coleta, pelo endereço do recurso.
#[cfg(feature = "devtools")]
pub(crate) static COLETA_PINS: std::sync::Mutex<Option<(usize, ColetaPins)>> = std::sync::Mutex::new(None);

/// Percorre a árvore a partir da raiz e coleta os pins das 4 classes.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_coleta_pins(raiz: *mut c_void) -> ColetaPins {
    let familias = [
        (cname("gameJournalQuestMapPin"), FamiliaPin::Pin),
        (cname("gameJournalQuestMultiMapPin"), FamiliaPin::Multi),
        (cname("gameJournalQuestPointOfInterestMapPin"), FamiliaPin::QuestPoi),
        (cname("gameJournalPointOfInterestMappin"), FamiliaPin::Poi),
    ];
    let mut c = ColetaPins { pins: Vec::new(), entradas: 0, hashes: 0, colisoes: 0, padrao_offset: None };
    let mut pilha: Vec<(*mut c_void, String)> = journal_container_de(raiz).into_iter().map(|r| (r, String::new())).collect();
    while let Some((cont, prefixo)) = pilha.pop() {
        for f in journal_filhos(cont) {
            if c.entradas >= 200_000 || !rtti::sane(f) || !crate::memoria::is_readable(f as *const c_void, JOURNAL_ENTRY_ID_OFF + 0x20) {
                continue;
            }
            c.entradas += 1;
            let id = rtti::read_cstring((f as *const u8).add(JOURNAL_ENTRY_ID_OFF)).unwrap_or_default();
            let caminho = if prefixo.is_empty() { id } else { format!("{prefixo}/{id}") };
            let classe = rtti::type_name_hash(rtti::class_of(f));
            if let Some((_, familia)) = familias.iter().find(|(h, _)| *h == classe) {
                let b = f as *const u8;
                let legivel = |n: usize| crate::memoria::is_readable(b as *const c_void, n);
                let (noderef, dinamica, offset) = match familia {
                    FamiliaPin::Poi if legivel(JOURNAL_POI_OFFSET_OFF + 12) => (rd_u64(b as *const c_void, JOURNAL_POI_NODEREF_OFF), 0, f32x3(b, JOURNAL_POI_OFFSET_OFF)),
                    FamiliaPin::Pin if legivel(JOURNAL_QUESTMAPPIN_REFERENCE_OFF + 0x20) => (
                        rd_u64(b as *const c_void, JOURNAL_QUESTMAPPIN_REFERENCE_OFF),
                        rd_u64(b as *const c_void, JOURNAL_QUESTMAPPIN_REFERENCE_OFF + ENTITYREFERENCE_DINAMICA_OFF),
                        f32x3(b, JOURNAL_PINBASE_OFFSET_OFF),
                    ),
                    FamiliaPin::Multi | FamiliaPin::QuestPoi if legivel(JOURNAL_PINBASE_OFFSET_OFF + 12) => (0, 0, f32x3(b, JOURNAL_PINBASE_OFFSET_OFF)),
                    _ => (0, 0, [0.0; 3]),
                };
                let hash = bwms_primitives::murmur3_32(caminho.as_bytes(), bwms_primitives::RECORDS_SEED);
                c.pins.push(PinDeJournal { caminho: caminho.clone(), hash, familia: *familia, entrada: f as usize, noderef, dinamica, offset });
            }
            if let Some(cc) = journal_container_de(f) {
                pilha.push((cc, caminho));
            }
        }
    }
    let mut vistos: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for p in &c.pins {
        *vistos.entry(p.hash).or_insert(0) += 1;
    }
    c.hashes = vistos.len();
    c.colisoes = vistos.values().map(|n| n - 1).sum();
    c
}

/// O que `f` tira do pin `indice` da família (0 QuestMapPin, 1 Multi, 2 QuestPoi, 3 POI) na coleta do recurso, que o `BwmsJournalMappinCensus` monta.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_pin_com<T>(res: *mut c_void, familia: i32, indice: i32, f: impl Fn(&PinDeJournal) -> T) -> Option<T> {
    let fam = match familia {
        0 => FamiliaPin::Pin,
        1 => FamiliaPin::Multi,
        2 => FamiliaPin::QuestPoi,
        3 => FamiliaPin::Poi,
        _ => return None,
    };
    let cache = COLETA_PINS.lock().ok()?;
    let (r, c) = cache.as_ref()?;
    if *r != res as usize || indice < 0 {
        return None;
    }
    c.pins.iter().filter(|p| p.familia == fam).nth(indice as usize).map(f)
}

/// `(resource, family, index)` dos argumentos das natives de pin.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_pin_args(frame: *mut c_void) -> (*mut c_void, i32, i32) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    (
        args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut()),
        args.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(-1),
        args.get(2).map(|(v, _)| *v as u32 as i32).unwrap_or(-1),
    )
}

/// `BwmsJournalPinHash(resource, family, index) -> Int32`: hash Murmur3 do caminho do pin; 0 fora do alcance ou sem coleta desse recurso.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_pin_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (res, familia, indice) = journal_pin_args(frame);
    write_uint_ret(out, journal_pin_com(res, familia, indice, |p| p.hash).unwrap_or(0) as u64, 4);
}

/// `BwmsJournalPinNodeRef(resource, family, index) -> Uint64`: hash do NodeRef do pin; 0 sem NodeRef, fora do alcance ou sem coleta desse recurso.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_pin_noderef(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (res, familia, indice) = journal_pin_args(frame);
    write_uint_ret(out, journal_pin_com(res, familia, indice, |p| p.noderef).unwrap_or(0), 8);
}

/// `BwmsJournalPinModHash(resource, family, index) -> Int32`: Murmur3 (seed `0x5EEDBA5E`) do caminho do pin com `_bwms` no último id; 0 fora do alcance.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_pin_mod_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (res, familia, indice) = journal_pin_args(frame);
    let h = journal_pin_com(res, familia, indice, |p| bwms_primitives::murmur3_32(format!("{}_bwms", p.caminho).as_bytes(), bwms_primitives::RECORDS_SEED));
    write_uint_ret(out, h.unwrap_or(0) as u64, 4);
}

/// `BwmsMappinModRegisterAt(system, hash, x, y, z, poi) -> Int32`: registra o pin de mod na posição dada, na busca de mappin (`poi` 0) ou de POI (1);
/// códigos do `mappin_de_mod_registrar`, -4 com coordenada não finita.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mappin_mod_register_at(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let hash = args.get(1).map(|(v, _)| *v as u32).unwrap_or(0);
    let eixo = |i: usize| args.get(i).map_or(f32::NAN, |(v, _)| f32::from_bits(*v as u32));
    let posicao = [eixo(2), eixo(3), eixo(4)];
    let poi = args.get(5).map_or(0, |(v, _)| *v as u32);
    let r = if posicao.iter().all(|c| c.is_finite()) {
        crate::missoes_e_mapa::mappin_de_mod_registrar(busca_do_codigo(if poi != 0 { 10 } else { 0 }), sys, hash, Some(posicao), [0, 0])
    } else {
        -4
    };
    write_uint_ret(out, r as u32 as u64, 4);
}

pub(crate) unsafe fn register_mappin_teste_natives(reg: &Registry, proto: *mut c_void) {
    let ok1 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "Int32"], "BwmsMappinTestInject", "BwmsMappinTestInject", corpo_de_prova!(tramp_mappin_test_inject)));
    let ok2 = registro_de_prova!(register_argful_by_types(reg, proto, &["Int32", "Int32"], "BwmsMappinTestPosition", "BwmsMappinTestPosition", corpo_de_prova!(tramp_mappin_test_position)));
    let ok3 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "Int32"], "BwmsMappinCookedPosition", "BwmsMappinCookedPosition", corpo_de_prova!(tramp_mappin_cooked_position)));
    let ok4 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "handle:IScriptable", "Int32"], "BwmsJournalMappinCensus", "BwmsJournalMappinCensus", corpo_de_prova!(tramp_journal_mappin_census)));
    let ok5 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "Int32"], "BwmsJournalPinHash", "BwmsJournalPinHash", corpo_de_prova!(tramp_journal_pin_hash)));
    let ok6 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "Int32"], "BwmsJournalPinNodeRef", "BwmsJournalPinNodeRef", corpo_de_prova!(tramp_journal_pin_noderef)));
    let ok7 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "Int32"], "BwmsJournalPinModHash", "BwmsJournalPinModHash", corpo_de_prova!(tramp_journal_pin_mod_hash)));
    let ok8 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "Float", "Float", "Float", "Int32"], "BwmsMappinModRegisterAt", "BwmsMappinModRegisterAt", corpo_de_prova!(tramp_mappin_mod_register_at)));
    let ok9 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Int32", "Int32"], "BwmsMappinCookedVolume", "BwmsMappinCookedVolume", corpo_de_prova!(tramp_mappin_cooked_volume)));
    crate::log_dev!(
        "[reg] register_all: journal-44 7b MappinTestInject={ok1} MappinTestPosition={ok2} MappinCookedPosition={ok3} 7d-2b JournalMappinCensus={ok4} PinHash={ok5} PinNodeRef={ok6} PinModHash={ok7} MappinModRegisterAt={ok8} 7d-2b-3 MappinCookedVolume={ok9}"
    );
}

// Journal de mod, parte 7d-2b-3a (2026-09-12, RE offline do volume de mappin): volume de um mappin cozido, só leitura. O
// elemento do `cookedData` (busca original) guarda o volume em Handle `@0x10`; `gamemappinsOutlineMappinVolume` (vptr `0x107a9d098`) tem `height` `@0x30` e
// `outlinePoints` DynArray<Vector2> `@0x38` (tamanho `@0x44`). Os pontos são locais (relativos à posição do elemento), girados pela orientação do nó.
#[cfg(feature = "devtools")]
pub(crate) const MAPPIN_VOLUME_OFF: usize = 0x10;

// Acesso ao grafo de fase (`QuestPhaseGraphAccessor`, 2026-09-12): consulta de grafo de fase. Nós por nome exato de classe, só do grafo ou
// com as fases aninhadas, na ordem de coleta (nós do grafo e depois cada `phaseGraph` não nulo); os 14 `Find*`; `GetAllGraphNodePaths`.
// Campos lidos pelo nome no RTTI (subindo pelos pais), `Cast` pela cadeia de pais da classe e valores de enum pelo nome; nada é escrito. Critério do
// teste: prova de 2026-09-12 (censo offline dos grafos).
pub(crate) const ACESSOR_BUSCAS: [&str; 14] = [
    "InputNode",
    "PointOfInterestMappin",
    "CompletedPointOfInterestMappin",
    "Communities",
    "SpawnSets",
    "Spawners",
    "FactChanges",
    "FactConditions",
    "ManagerEvents",
    "JournalEntries",
    "JournalConditions",
    "LootConditions",
    "LootContainerCondition",
    "CharacterKillCondition",
];

pub(crate) unsafe fn acessor_classe_exata(obj: *const u8, hash: u64) -> bool {
    rtti::type_name_hash(rtti::class_of(obj as *mut c_void)) == hash
}

/// Ponteiro do valor do campo `nome` do objeto e o tipo do campo.
pub(crate) unsafe fn acessor_campo(obj: *const u8, nome: &str) -> Option<(*const u8, *mut c_void)> {
    let prop = rtti::find_property_in_class(rtti::class_of(obj as *mut c_void), nome);
    if !rtti::sane(prop) {
        return None;
    }
    Some((obj.add(rd_u32(prop as *const c_void, 0x20) as usize), rd_u64(prop as *const c_void, 0x00) as *mut c_void))
}

/// Instância de um campo `Handle`/`WeakHandle` pelo nome; `None` se nulo ou ilegível.
pub(crate) unsafe fn acessor_handle(obj: *const u8, nome: &str) -> Option<*const u8> {
    let (p, _) = acessor_campo(obj, nome)?;
    let inst = (p as *const u64).read_unaligned() as *const u8;
    (rtti::sane(inst as *mut c_void) && crate::memoria::is_readable(inst as *const c_void, 0x40)).then_some(inst)
}

/// Valor inteiro de um campo na largura do tipo dele (1, 2, 4 ou 8 B: Bool, enum, Uint16, CName...).
pub(crate) unsafe fn acessor_inteiro(obj: *const u8, nome: &str) -> Option<u64> {
    let (p, ty) = acessor_campo(obj, nome)?;
    Some(match rtti::type_size(ty) {
        1 => p.read() as u64,
        2 => (p as *const u16).read_unaligned() as u64,
        4 => (p as *const u32).read_unaligned() as u64,
        8 => (p as *const u64).read_unaligned(),
        _ => return None,
    })
}

/// Nós na ordem de coleta (nós do grafo e depois cada `phaseGraph` não nulo).
pub(crate) unsafe fn acessor_coleta(g: *const u8, recursivo: bool, v: &mut Vec<*const u8>, prof: usize) {
    if prof > 16 || v.len() > 100_000 {
        return;
    }
    let mut aninhados = Vec::new();
    for n in handles_legiveis_em(g, GRAPHDEF_NODES_OFF, 65536) {
        v.push(n);
        if recursivo && acessor_e_da_classe(n, "questPhaseNodeDefinition") {
            if let Some(pg) = acessor_handle(n, "phaseGraph") {
                aninhados.push(pg);
            }
        }
    }
    for pg in aninhados {
        acessor_coleta(pg, recursivo, v, prof + 1);
    }
}

pub(crate) unsafe fn acessor_nos(graph: *const u8, recursivo: bool) -> Vec<*const u8> {
    let mut v = Vec::new();
    if rtti::sane(graph as *mut c_void) && crate::memoria::is_readable(graph as *const c_void, 0x40) {
        acessor_coleta(graph, recursivo, &mut v, 0);
    }
    v
}

/// Resultado de um `Find*` (pelo nome da busca); `None` se a busca não existe.
pub(crate) unsafe fn acessor_find(graph: *const u8, recursivo: bool, busca: &str) -> Option<Vec<*const u8>> {
    let reg = rtti::Registry::obtain()?;
    let nos = acessor_nos(graph, recursivo);
    let da_classe = |c: &str| -> Vec<*const u8> {
        let h = cname(c);
        nos.iter().copied().filter(|n| acessor_classe_exata(*n, h)).collect()
    };
    let enum_v = |e: &str, m: &str| rtti::resolve_enum_value(&reg, e, m);
    let igual = |a: Option<u64>, b: Option<u64>| a.is_some() && a == b;
    let poi = cname("gameJournalPointOfInterestMappin");
    let caminho_de_poi = |t: *const u8| acessor_handle(t, "path").and_then(|p| acessor_inteiro(p, "className")) == Some(poi);
    let condicoes = |no: &str, cond: &str| -> Vec<*const u8> {
        da_classe(no).into_iter().filter_map(|n| acessor_handle(n, "condition")).filter(|c| acessor_e_da_classe(*c, cond)).filter_map(|c| acessor_handle(c, "type")).collect()
    };
    let r: Vec<*const u8> = match busca {
        "InputNode" => da_classe("questInputNodeDefinition").into_iter().take(1).collect(),
        "PointOfInterestMappin" => da_classe("questJournalNodeDefinition")
            .into_iter()
            .filter_map(|n| acessor_handle(n, "type"))
            .filter(|t| acessor_e_da_classe(*t, "questJournalEntry_NodeType") && caminho_de_poi(*t))
            .take(1)
            .collect(),
        "CompletedPointOfInterestMappin" => {
            let concluida = enum_v("gamedataMappinPhase", "CompletedPhase");
            da_classe("questJournalNodeDefinition")
                .into_iter()
                .filter_map(|n| acessor_handle(n, "type"))
                .filter(|t| acessor_e_da_classe(*t, "questJournalChangeMappinPhase_NodeType") && caminho_de_poi(*t) && igual(acessor_inteiro(*t, "phase"), concluida))
                .take(1)
                .collect()
        }
        "Communities" | "SpawnSets" | "Spawners" => {
            let alvo = match busca {
                "Communities" => "questCommunityTemplate_NodeType",
                "SpawnSets" => "questSpawnSet_NodeType",
                _ => "questSpawner_NodeType",
            };
            let (ativa, reativa) = (enum_v("populationSpawnerObjectCtrlAction", "Activate"), enum_v("populationSpawnerObjectCtrlAction", "Reactivate"));
            let ent = reg.class_by_name("questSpawnManagerNodeActionEntry");
            let tprop = if rtti::sane(ent) { rtti::find_property_in_class(ent, "type") } else { std::ptr::null_mut() };
            let mut v = Vec::new();
            if rtti::sane(tprop) {
                let tam = rtti::type_size(ent) as usize;
                let off = rd_u32(tprop as *const c_void, 0x20) as usize;
                for n in da_classe("questSpawnManagerNodeDefinition") {
                    let Some((p, _)) = acessor_campo(n, "actions") else { continue };
                    let ptr = (p as *const u64).read_unaligned() as *const u8;
                    let qtd = (p.add(0x0C) as *const u32).read_unaligned() as usize;
                    if tam == 0 || ptr.is_null() || qtd > 4096 || !crate::memoria::is_readable(ptr as *const c_void, qtd * tam) {
                        continue;
                    }
                    for k in 0..qtd {
                        let t = (ptr.add(k * tam + off) as *const u64).read_unaligned() as *const u8;
                        if rtti::sane(t as *mut c_void) && acessor_e_da_classe(t, alvo) {
                            let a = acessor_inteiro(t, "action");
                            if igual(a, ativa) || igual(a, reativa) {
                                v.push(t);
                            }
                        }
                    }
                }
            }
            v
        }
        "FactChanges" => da_classe("questFactsDBManagerNodeDefinition")
            .into_iter()
            .filter_map(|n| acessor_handle(n, "type"))
            .filter(|t| acessor_e_da_classe(*t, "questSetVar_NodeType"))
            .collect(),
        "FactConditions" => {
            condicoes("questPauseConditionNodeDefinition", "questFactsDBCondition").into_iter().filter(|t| acessor_e_da_classe(*t, "questVarComparison_ConditionType")).collect()
        }
        "ManagerEvents" => da_classe("questEventManagerNodeDefinition"),
        "JournalEntries" => da_classe("questJournalNodeDefinition"),
        "JournalConditions" => {
            let ativa_usuario = enum_v("gameJournalEntryUserState", "Active");
            let ativa = enum_v("gameJournalEntryState", "Active");
            let mut v = Vec::new();
            for t in condicoes("questPauseConditionNodeDefinition", "questJournalCondition") {
                if acessor_e_da_classe(t, "questJournalEntry_ConditionType") {
                    if igual(acessor_inteiro(t, "state"), ativa_usuario) {
                        v.extend(acessor_handle(t, "path"));
                    }
                    continue;
                }
                if acessor_e_da_classe(t, "questJournalEntryState_ConditionType") && igual(acessor_inteiro(t, "state"), ativa) && acessor_inteiro(t, "inverted") == Some(0) {
                    v.extend(acessor_handle(t, "path"));
                }
            }
            v
        }
        "LootConditions" => condicoes("questPauseConditionNodeDefinition", "questObjectCondition")
            .into_iter()
            .filter(|t| acessor_e_da_classe(*t, "questInventory_ConditionType") && acessor_inteiro(*t, "isPlayer").map_or(false, |b| b != 0))
            .collect(),
        "LootContainerCondition" => {
            let executada = enum_v("questObjectInteractionEventType", "Executed");
            condicoes("questPauseConditionNodeDefinition", "questObjectCondition")
                .into_iter()
                .filter(|t| acessor_e_da_classe(*t, "questInteraction_ConditionType") && igual(acessor_inteiro(*t, "eventType"), executada))
                .take(1)
                .collect()
        }
        "CharacterKillCondition" => condicoes("questPauseConditionNodeDefinition", "questCharacterCondition")
            .into_iter()
            .chain(condicoes("questConditionNodeDefinition", "questCharacterCondition"))
            .filter(|t| acessor_e_da_classe(*t, "questCharacterKilled_ConditionType"))
            .take(1)
            .collect(),
        _ => return None,
    };
    Some(r)
}

/// `GetAllGraphNodePaths`: `(ids das fases de fora até o grafo, id do nó)` de cada `questNodeDefinition`, entrando nos `phaseGraph` não nulos.
pub(crate) unsafe fn acessor_paths(g: *const u8, caminho: &mut Vec<u16>, v: &mut Vec<(Vec<u16>, u16)>, prof: usize) {
    if prof > 16 || v.len() > 100_000 {
        return;
    }
    for n in handles_legiveis_em(g, GRAPHDEF_NODES_OFF, 65536) {
        if !acessor_e_da_classe(n, "questNodeDefinition") {
            continue;
        }
        let Some(id) = acessor_inteiro(n, "id") else { continue };
        v.push((caminho.clone(), id as u16));
        if acessor_e_da_classe(n, "questPhaseNodeDefinition") {
            if let Some(pg) = acessor_handle(n, "phaseGraph") {
                caminho.push(id as u16);
                acessor_paths(pg, caminho, v, prof + 1);
                caminho.pop();
            }
        }
    }
}

pub(crate) unsafe fn acessor_paths_de(graph: *const u8) -> Vec<(Vec<u16>, u16)> {
    let mut v = Vec::new();
    if rtti::sane(graph as *mut c_void) && crate::memoria::is_readable(graph as *const c_void, 0x40) {
        acessor_paths(graph, &mut Vec::new(), &mut v, 0);
    }
    v
}

pub(crate) fn acessor_busca_do_hash(h: u64) -> Option<&'static str> {
    ACESSOR_BUSCAS.iter().copied().find(|b| cname(b) == h)
}

/// `BwmsQuestPhaseResourceGraph(resource) -> ref<IScriptable>`: o grafo (`questGraphDefinition`) de uma fase carregada.
pub(crate) unsafe extern "C" fn tramp_questphase_resource_graph(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let res = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    write_handle_ret(out, questphase_grafo_de(res).map_or(0, |g| g as u64));
}

/// Registro das chamadas do acessor (as primeiras 64): no boot 65 o `recursive` chegou falso em todas e 6 buscas vazias somaram 672.
pub(crate) static ACESSOR_LOGS: AtomicU64 = AtomicU64::new(0);

pub(crate) unsafe fn acessor_log(texto: &str) {
    if crate::primeiras(&ACESSOR_LOGS, 64) {
        crate::veredito_dev!("[cw160] {texto}");
    }
}

/// `BwmsQuestAccessorCount(graph, recursive, typeName) -> Int32`: nós com essa classe exata; `recursive` 0 ou 1.
pub(crate) unsafe extern "C" fn tramp_quest_accessor_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let g = args.first().map(|(v, _)| *v as *const u8).unwrap_or(std::ptr::null());
    let bruto = args.get(1).map_or(0, |(v, _)| *v);
    let rec = bruto as u32 != 0;
    let h = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let n = acessor_nos(g, rec).into_iter().filter(|n| acessor_classe_exata(*n, h)).count();
    acessor_log(&format!("count graph={g:p} recursive bruto={bruto:#x} classe={} -> {n}", crate::cname::resolve_cname(h)));
    write_i32_ret(out, n as i32);
}

/// `BwmsQuestAccessorAt(graph, recursive, typeName, index) -> ref<IScriptable>`.
pub(crate) unsafe extern "C" fn tramp_quest_accessor_at(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let g = args.first().map(|(v, _)| *v as *const u8).unwrap_or(std::ptr::null());
    let rec = args.get(1).map_or(false, |(v, _)| *v as u32 != 0);
    let h = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let i = args.get(3).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let no = acessor_nos(g, rec).into_iter().filter(|n| acessor_classe_exata(*n, h)).nth(i);
    write_handle_ret(out, no.map_or(0, |p| p as u64));
}

/// `BwmsQuestAccessorFindCount(graph, recursive, kind) -> Int32`: tamanho do resultado da busca; -1 se a busca não existe.
pub(crate) unsafe extern "C" fn tramp_quest_accessor_find_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let g = args.first().map(|(v, _)| *v as *const u8).unwrap_or(std::ptr::null());
    let bruto = args.get(1).map_or(0, |(v, _)| *v);
    let rec = bruto as u32 != 0;
    let h = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let busca = acessor_busca_do_hash(h);
    let n = busca.and_then(|b| acessor_find(g, rec, b)).map_or(-1, |v| v.len() as i32);
    acessor_log(&format!("find graph={g:p} recursive bruto={bruto:#x} busca={} ({h:#x}) -> {n}", busca.unwrap_or("desconhecida")));
    write_i32_ret(out, n);
}

/// `BwmsQuestAccessorFindAt(graph, recursive, kind, index) -> ref<IScriptable>`.
pub(crate) unsafe extern "C" fn tramp_quest_accessor_find_at(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let g = args.first().map(|(v, _)| *v as *const u8).unwrap_or(std::ptr::null());
    let rec = args.get(1).map_or(false, |(v, _)| *v as u32 != 0);
    let h = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let i = args.get(3).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let item = acessor_busca_do_hash(h).and_then(|b| acessor_find(g, rec, b)).and_then(|v| v.get(i).copied());
    write_handle_ret(out, item.map_or(0, |p| p as u64));
}

/// `BwmsQuestAccessorPathCount(graph) -> Int32`.
pub(crate) unsafe extern "C" fn tramp_quest_accessor_path_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let g = args.first().map(|(v, _)| *v as *const u8).unwrap_or(std::ptr::null());
    write_i32_ret(out, acessor_paths_de(g).len() as i32);
}

/// `BwmsQuestAccessorPathDepth(graph, index) -> Int32`: quantas fases há no caminho; -1 fora do alcance.
pub(crate) unsafe extern "C" fn tramp_quest_accessor_path_depth(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let g = args.first().map(|(v, _)| *v as *const u8).unwrap_or(std::ptr::null());
    let i = args.get(1).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    write_i32_ret(out, acessor_paths_de(g).get(i).map_or(-1, |(c, _)| c.len() as i32));
}

/// `BwmsQuestAccessorPathId(graph, index, level) -> Int32`: id da fase no nível `level` do caminho; com `level` igual à profundidade, o id do nó; -1
/// fora do alcance.
pub(crate) unsafe extern "C" fn tramp_quest_accessor_path_id(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let g = args.first().map(|(v, _)| *v as *const u8).unwrap_or(std::ptr::null());
    let i = args.get(1).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let nivel = args.get(2).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let id = acessor_paths_de(g).get(i).and_then(|(c, id)| if nivel == c.len() { Some(*id) } else { c.get(nivel).copied() });
    write_i32_ret(out, id.map_or(-1, |x| x as i32));
}

// O `QuestContext` do Mac (pré-requisito do `ExecuteNode` abaixo), 2026-09-16.
//
// O `ExecuteNode` pede 4 peças; 3 já tinham endereço (`ExecutePath 0x102808484`, `ExecuteGraph 0x10280b7d0`,
// `ExecuteNode 0x10280df0c`) e a 4ª, `CreateContext`, está INLINADA no Mac. O que sobra dela é o construtor `0x102803660` — o thunk `0x10280379c` relê e
// regrava os mesmos slots de pilha e salta pra ele, então é chamada direta — e o destrutor `0x102a12b7c`.
//
// ABI conferida no desmonte: `ctor(ctx, w1, w2, w3, w4, x5, x6, x7, p1, p2, p3, p4)` grava `w1` em `+0x00` e `+0x01`, `w2` em `+0x02`, `w3`/`w4` em
// `+0x04`/`+0x08`, `x5`/`x6` em `+0x10`/`+0x18`, `x7`/`p1` em `+0x20`/`+0x28` e `p2`/`p3`/`p4` em `+0x60`/`+0x68`/`+0x70`; inicializa as `DynArray` de
// `+0xe0` e `+0xf8` (`phaseStack`, tamanho em `+0x104`), monta o `phaseContext` em `+0x110` (vtable `0x1079e2f98`) e guarda o próprio endereço em `+0x330`.
// Os valores do sistema saem de `[sys+0xe8]`, `sys+0xa8`, `sys+0xb8` e, na pilha, `sys+0x78`, `sys+0x1d0`, `[sys+0x2c8]`, `[sys+0x40]`. A chamada pelo
// script usa `(1, 0, -1, -1)`; o tick do Mac usa `(2, 0, -1, índice)`.
//
// Esta parte NÃO executa nó nenhum: monta o contexto num buffer nosso, confere pelos campos que o construtor grava e destrói. Zero mutação de quest, zero
// fact no save — executar um nó de verdade avança progresso e é decisão do usuário.
pub(crate) const QUESTCTX_CTOR_VM: u64 = 0x1_0280_3660;

pub(crate) const QUESTCTX_DTOR_VM: u64 = 0x1_02a1_2b7c;

// `QuestsSystem.ExecuteNode(node, opt inputSocket)` exposto ao script, 2026-09-18.
//
// A chamada trava o `Mutex` do sistema (`SharedSpinLock` em `+0x60`), monta o contexto com `(1, 0, -1, -1)`, empilha a fase raiz no `phaseStack` e
// chama o `ExecuteNode` com o socket `{inputSocket, 0}`. No Mac a trava foi medida num método do próprio sistema (`0x102a0c704`): ele monta o contexto,
// empilha a raiz (`0x1028037a0`), chama `0x1000015f0(sys+0x60)` — espera o byte zerar e troca 0 por `0x80` com `casalb` —, executa, solta com
// `ldclralb #0x80` e só então desempilha (`0x10280384c`) e destrói. O byte `0x80` é a escrita; leitores somam 1 nos bits de baixo (`0x10000162c`).
// Aqui a ordem é a do Mac e a trava é por tentativa com teto: se a própria thread já a segura (nó chamado de dentro de um callback do sistema), a
// espera infinita do motor travaria o jogo, e a tentativa devolve "ocupado" sem executar.
//
// A saída (`DynArray<QuestNodeSocket>`, elemento de 0x10) nasce vazia e o nó empurra nela pelo `realloc` do motor (`0x100028674`); ela é devolvida pelo
// mesmo `realloc` com capacidade 0, que é o caminho do destrutor. O nó vem do script dentro de um `BwmsSerializable` (nó de quest deriva só de
// `ISerializable`); o leitor de parâmetros desembrulha, e a classe é conferida contra `questNodeDefinition` antes de qualquer chamada.
pub(crate) const QUESTS_TRAVA_OFF: usize = 0x60;

pub(crate) const QUESTCTX_EMPILHA_VM: u64 = 0x1_0280_37a0;

pub(crate) const QUESTCTX_DESEMPILHA_VM: u64 = 0x1_0280_384c;

/// Tentativas da trava antes de desistir: a seção crítica do motor dura microssegundos.
pub(crate) const QUESTS_TRAVA_TENTATIVAS: u32 = 1 << 14;

/// O que uma execução mediu, pro veredito.
pub(crate) struct QuestsExecucao {
    pub(crate) resultado: u32,
    pub(crate) saidas: Vec<u64>,
    pub(crate) pilha_empilhada: u32,
    pub(crate) pilha_depois: u32,
}

/// `BwmsQuestsSystemExecuteNode(sys, node, inputSocket) -> Int32`: o resultado do `ExecuteNode` (0–3) ou o código de erro negativo.
pub(crate) unsafe extern "C" fn tramp_quests_execute_node(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let no = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let socket = args.get(2).map_or(0, |(v, _)| *v);
    let deriva = rtti::sane(sys) && classe_deriva_de_nome(rtti::class_of(sys), crate::cname::cname("questQuestsSystem"));
    let r = if deriva { quests_execute_node(sys, no, socket) } else { Err(-1) };
    let codigo = match &r {
        Ok(e) => e.resultado as i32,
        Err(c) => *c,
    };
    if let Ok(mut u) = QUESTS_ULTIMA.lock() {
        *u = r.as_ref().ok().map(|e| (e.saidas.clone(), e.pilha_empilhada, e.pilha_depois));
    }
    write_i32_ret(out, codigo);
}

/// `BwmsQuestNodeNew(className) -> ref<IScriptable>`: nó de quest novo pelo `CreateInstance` da classe, dentro de um `BwmsSerializable`; nulo fora de
/// `questNodeDefinition` ou em classe abstrata.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_quest_node_new(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let h = args.first().map_or(0, |(v, _)| *v);
    let cls = rtti::Registry::obtain().map_or(std::ptr::null_mut(), |reg| reg.class_by_hash(h));
    let no = if rtti::sane(cls) && classe_deriva_de_nome(cls, crate::cname::cname("questNodeDefinition")) {
        rtti::create_instance(cls)
    } else {
        std::ptr::null_mut()
    };
    entregar_recurso(out, if rtti::sane(no) { no } else { std::ptr::null_mut() });
}

/// Monta o nó sintético da busca; `(nó, dono)` ou o código de erro.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn acessor_no_sintetico(reg: &Registry, busca: &str, certo: bool) -> Result<(*mut u8, [u64; 2]), i32> {
    let novo = |c: &str| rtti::new_object(reg, c) as *mut u8;
    match busca {
        "LootConditions" => {
            let (no, cond, tipo) = (novo("questPauseConditionNodeDefinition"), novo("questObjectCondition"), novo("questInventory_ConditionType"));
            if no.is_null() || cond.is_null() || tipo.is_null() {
                return Err(-6);
            }
            let mut dono = [0u64; 2];
            rtti::make_handle(dono.as_mut_ptr() as *mut c_void, no as *mut c_void);
            let ok = campo_inteiro_grava(tipo, "isPlayer", certo as u64) && campo_handle_grava(cond, "type", tipo) && campo_handle_grava(no, "condition", cond);
            if ok { Ok((no, dono)) } else { let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono); Err(-5) }
        }
        "CompletedPointOfInterestMappin" => {
            let (no, tipo, caminho) = (novo("questJournalNodeDefinition"), novo("questJournalChangeMappinPhase_NodeType"), novo("gameJournalPath"));
            if no.is_null() || tipo.is_null() || caminho.is_null() {
                return Err(-6);
            }
            let Some(concluida) = rtti::resolve_enum_value(reg, "gamedataMappinPhase", "CompletedPhase") else { return Err(-5) };
            let mut dono = [0u64; 2];
            rtti::make_handle(dono.as_mut_ptr() as *mut c_void, no as *mut c_void);
            let fase = if certo { concluida } else { rtti::resolve_enum_value(reg, "gamedataMappinPhase", "DiscoveredPhase").unwrap_or(0) };
            let ok = campo_inteiro_grava(caminho, "className", cname("gameJournalPointOfInterestMappin"))
                && campo_inteiro_grava(tipo, "phase", fase)
                && campo_handle_grava(tipo, "path", caminho)
                && campo_handle_grava(no, "type", tipo);
            if ok { Ok((no, dono)) } else { let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono); Err(-5) }
        }
        _ => Err(-2),
    }
}

/// Resultado da busca com o nó sintético no grafo (tamanho do resultado); negativo em erro: -1 recurso, -2 busca sem teste sintético, -4 mover ou folga,
/// -5 campo, -6 classe/objeto.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn acessor_teste_sintetico(res: *mut c_void, busca: &str, certo: bool) -> i32 {
    let Some(graph) = questphase_grafo_de(res) else { return -1 };
    let Some(reg) = rtti::Registry::obtain() else { return -6 };
    if !handles_folga(graph, GRAPHDEF_NODES_OFF, GRAPHNODE_MOVER_GOT_VM, GRAPHNODE_MOVER_VM) {
        return -4;
    }
    let (no, mut dono) = match acessor_no_sintetico(&reg, busca, certo) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let antes = acessor_find(graph, true, busca).map_or(-1, |v| v.len() as i32);
    let (_, _, tam) = handles_arr_em(graph, GRAPHDEF_NODES_OFF);
    handle_strong_soma(&dono, 1);
    if !handles_insere(graph, GRAPHDEF_NODES_OFF, tam as usize, dono) {
        handle_strong_soma(&dono, -1);
        let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono);
        return -4;
    }
    let achados = acessor_find(graph, true, busca).unwrap_or_default();
    let classe = achados.first().map_or(String::from("-"), |p| crate::cname::resolve_cname(rtti::type_name_hash(rtti::class_of(*p as *mut c_void))));
    let n = achados.len() as i32;
    if let Some(i) = handle_indice_em(graph, GRAPHDEF_NODES_OFF, no, 65536) {
        let h = handles_tira(graph, GRAPHDEF_NODES_OFF, i);
        handle_strong_soma(&h, -1);
    }
    let destruido = crate::jogo::objetos::soltar_handle_contando(&mut dono);
    crate::veredito_dev!(
        "[cw160] sintético {busca} certo={certo}: antes {antes}, com o nó {n} (1º {classe}), nó destruído pelo motor={destruido}"
    );
    n - antes
}

/// `BwmsQuestAccessorSyntheticTest(resource, kind, right) -> Int32`: quantos resultados a busca ganha com o nó sintético (`right` 1 = dado certo, 0 = campo
/// do filtro errado); negativo em erro.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_quest_accessor_synthetic_test(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let res = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let h = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let certo = args.get(2).map_or(false, |(v, _)| *v as u32 != 0);
    let r = match acessor_busca_do_hash(h) {
        Some(b) => acessor_teste_sintetico(res, b, certo),
        None => -2,
    };
    write_i32_ret(out, r);
}

// Journal de mod, parte 5d-1 (2026-09-12): mescla a seco de um `.journal` carregado na árvore viva. Percorre o container do recurso contra o container
// vivo casando filhos por `id` (o passo da mescla antes de inserir) e soma casados, faltando, casados que são o MESMO objeto (se a
// cópia carregada dividir objetos com a árvore viva, a 5d-2 não pode mexer na cópia) e containers visitados. Só leitura, na game thread.
#[cfg(feature = "devtools")]
pub(crate) static JOURNAL_MESCLA_SECO: std::sync::Mutex<[i32; 4]> = std::sync::Mutex::new([0; 4]);

/// Mescla a seco de `modc` em `vivo`, somando em `acc` [casados, faltando, mesmo objeto, containers]; loga até 4 caminhos faltando.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_mescla_seco(modc: *mut c_void, vivo: *mut c_void, caminho: &str, prof: u32, acc: &mut [i32; 4]) {
    acc[3] += 1;
    let vivos = journal_ids_mapa(vivo);
    for (id, h) in journal_ids_mapa(modc) {
        let p = if caminho.is_empty() { id.clone() } else { format!("{caminho}/{id}") };
        match vivos.get(&id) {
            None => {
                acc[1] += 1;
                if acc[1] <= 4 {
                    crate::veredito_dev!("[a44-mescla] faltando na árvore viva: {p}");
                }
            }
            Some(hv) => {
                acc[0] += 1;
                if hv[0] == h[0] {
                    acc[2] += 1;
                }
                if prof < 16 {
                    if let (Some(mc), Some(vc)) = (journal_container_de(h[0] as *mut c_void), journal_container_de(hv[0] as *mut c_void)) {
                        journal_mescla_seco(mc, vc, &p, prof + 1, acc);
                    }
                }
            }
        }
    }
}

/// `BwmsJournalMergeDryRun(jm, resource, root, level) -> Int32`: mescla a seco do root do `.journal` na raiz viva `root` (`level` 0) ou na 1ª pasta
/// dela (`level` 1, controle de nível errado). Devolve os casados (-1 recurso sem root container, -2 raiz viva inválida) e guarda os 4 contadores.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_merge_dry_run(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let jm = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let res = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let raiz = args.get(2).map(|(v, _)| *v as u32 as usize).unwrap_or(0);
    let nivel = args.get(3).map(|(v, _)| *v as u32 as i32).unwrap_or(0);
    let r: Result<[i32; 4], i32> = (|| {
        let modc = journal_resource_root(res).and_then(|r| journal_container_de(r)).ok_or(-1)?;
        let mut vivo = journal_tree_raiz(jm, raiz).and_then(|r| journal_container_de(r)).ok_or(-2)?;
        if nivel == 1 {
            let primeiro = journal_filhos(vivo).first().copied().ok_or(-2)?;
            vivo = journal_container_de(primeiro).ok_or(-2)?;
        }
        let mut acc = [0i32; 4];
        journal_mescla_seco(modc, vivo, "", 0, &mut acc);
        Ok(acc)
    })();
    let (ret, acc) = match r {
        Ok(a) => (a[0], a),
        Err(e) => (e, [0; 4]),
    };
    if let Ok(mut s) = JOURNAL_MESCLA_SECO.lock() {
        *s = acc;
    }
    crate::veredito_dev!(
        "[a44-mescla] raiz {raiz} nível {nivel}: casados {} faltando {} mesmo-objeto {} containers {} (ret {ret})",
        acc[0], acc[1], acc[2], acc[3]
    );
    write_uint_ret(out, ret as u32 as u64, 4);
}

/// `BwmsJournalMergeStat(which) -> Int32`: 0 casados, 1 faltando, 2 mesmo objeto, 3 containers da última mescla a seco; -1 fora da faixa.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_merge_stat(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let q = args.first().map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let v = JOURNAL_MESCLA_SECO.lock().map(|a| a.get(q).copied().unwrap_or(-1)).unwrap_or(-1);
    write_uint_ret(out, v as u32 as u64, 4);
}

// Journal de mod, parte 6b (2026-09-12): conversão de LocKeys. Em cada campo `LocalizationString` da entrada (com os herdados; a String fica em `+0x08` do
// valor) cujo texto não começa com `LocKey#`, o texto vira `LocKey#<FNV-1a64 decimal do texto>`. O texto novo passa de 19 bytes: a `CString` sai do
// construtor do motor `0x10002ccac(out, const char*)` — o mesmo que a via B do `RegisterArchive` usa ao vivo —, com o allocator dele, e é movida pro
// campo depois de o valor antigo ser destruído pelo tipo `String` (`vt+0x48`, tipo tirado do campo `id` da entrada).
pub(crate) const CSTRING_CTOR_VM: u64 = 0x1_0002_ccac;

/// Offsets (no objeto) das Strings dos campos `LocalizationString` da classe, subindo pelos pais (`+0x10`). O nome do tipo do campo sai do `GetName`
/// (`type_name_getname`): `LocalizationString` não é classe, e o `+0x18` lido pelo `type_name_hash` não é o nome dele (boot 62: 0 campos achados em
/// 28475 entradas).
pub(crate) unsafe fn journal_lockey_offsets(cls0: *mut c_void) -> Vec<usize> {
    let alvo = cname("LocalizationString");
    let mut v = Vec::new();
    let mut cls = cls0;
    let mut niveis = 0;
    while rtti::sane(cls) && niveis < 16 {
        for prop in rtti::class_own_properties(cls) {
            let ptype = rd_u64(prop as *const c_void, 0x00) as *mut c_void;
            if rtti::sane(ptype) && rtti::type_name_getname(ptype) == alvo {
                v.push(rd_u32(prop as *const c_void, 0x20) as usize + 0x08);
            }
        }
        cls = rd_u64(cls as *const c_void, 0x10) as *mut c_void;
        niveis += 1;
    }
    v
}

/// Slots das Strings dos campos `LocalizationString` da entrada.
pub(crate) unsafe fn journal_lockey_slots(entrada: *mut c_void) -> Vec<*mut u8> {
    journal_lockey_offsets(rtti::class_of(entrada)).into_iter().map(|o| (entrada as *mut u8).add(o)).collect()
}

/// Censo dos campos `LocalizationString` da árvore do recurso (até `limite` entradas). No boot 61 nenhuma entrada tinha os textos todos com `LocKey#`
/// (a 6b saiu com -3): o formato dos textos cozidos sai daqui, e o teste monta os textos que precisa.
#[cfg(feature = "devtools")]
pub(crate) struct CensoLocKey {
    pub(crate) entrada: Option<*mut c_void>,
    pub(crate) entradas: usize,
    pub(crate) com_campo: usize,
    pub(crate) lockey: usize,
    pub(crate) vazios: usize,
    pub(crate) outros: usize,
    pub(crate) exemplo: String,
}

#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_lockey_censo(raiz: *mut c_void, limite: usize) -> CensoLocKey {
    let mut c = CensoLocKey { entrada: None, entradas: 0, com_campo: 0, lockey: 0, vazios: 0, outros: 0, exemplo: String::new() };
    let mut offsets: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    let mut pilha = vec![raiz];
    'arvore: while let Some(cont) = pilha.pop() {
        for f in journal_filhos(cont) {
            if c.entradas >= limite {
                break 'arvore;
            }
            c.entradas += 1;
            if !rtti::sane(f) {
                continue;
            }
            let cls = rtti::class_of(f);
            let offs = offsets.entry(cls as usize).or_insert_with(|| journal_lockey_offsets(cls));
            let slots: Vec<*mut u8> = offs.iter().map(|o| (f as *mut u8).add(*o)).collect();
            if !slots.is_empty() {
                c.com_campo += 1;
                if c.entrada.is_none() {
                    c.entrada = Some(f);
                }
            }
            for s in &slots {
                let t = rtti::read_cstring(*s).unwrap_or_default();
                if t.starts_with(LOCKEY_PREFIXO) {
                    c.lockey += 1;
                } else if t.is_empty() {
                    c.vazios += 1;
                } else {
                    c.outros += 1;
                    if c.exemplo.is_empty() {
                        c.exemplo = t.chars().take(40).collect();
                    }
                }
            }
            if let Some(cc) = journal_container_de(f) {
                pilha.push(cc);
            }
        }
    }
    c
}

/// Devolve os bytes originais dos slots; o que o teste deixou no heap (bytes diferentes do original, com a flag de heap) é destruído antes.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_lockey_devolver(tipo: *mut c_void, slots: &[*mut u8], originais: &[[u8; 0x20]]) {
    for (s, o) in slots.iter().zip(originais) {
        let atual = std::slice::from_raw_parts(*s as *const u8, 0x20);
        let heap = (s.add(0x14) as *const u32).read_unaligned() >= 0x4000_0000;
        if heap && atual != &o[..] {
            destruir_string(tipo, *s);
        }
        std::ptr::copy_nonoverlapping(o.as_ptr(), *s, 0x20);
    }
}

/// Teste da 6b numa cópia carregada: a 1ª entrada com campo `LocalizationString` recebe `LocKey#1` inline em todos (controle: nada convertido) e
/// depois `bwms_teste_6` no 1º (1 convertido, lido `LocKey#<FNV-1a64>`, no heap); os originais voltam. 1 = ok; -1 recurso sem root; -2 sem tipo
/// `String`; -3 sem entrada com o campo; -4 controle converteu; -5 escrita inline; -6 conversão, releitura ou devolução diferente do esperado.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn journal_lockey_teste(res: *mut c_void) -> i32 {
    let Some(raiz) = journal_resource_root(res).and_then(|r| journal_container_de(r)) else { return -1 };
    let censo = journal_lockey_censo(raiz, 40000);
    crate::veredito_dev!(
        "[a44-lockey] censo: {} entrada(s), {} com campo LocalizationString; textos com LocKey# {}, vazios {}, outros {} (1º '{}')",
        censo.entradas, censo.com_campo, censo.lockey, censo.vazios, censo.outros, censo.exemplo
    );
    let Some(entrada) = censo.entrada else { return -3 };
    let tipo = tipo_string_da_entrada(entrada);
    if !rtti::sane(tipo) {
        return -2;
    }
    let slots = journal_lockey_slots(entrada);
    let originais: Vec<[u8; 0x20]> = slots
        .iter()
        .map(|s| {
            let mut b = [0u8; 0x20];
            std::ptr::copy_nonoverlapping(*s as *const u8, b.as_mut_ptr(), 0x20);
            b
        })
        .collect();
    let textos: Vec<String> = slots.iter().map(|s| rtti::read_cstring(*s).unwrap_or_default()).collect();
    if !slots.iter().all(|s| rtti::red_string_write_inline(*s, "LocKey#1")) {
        journal_lockey_devolver(tipo, &slots, &originais);
        return -5;
    }
    let controle = match journal_converter_lockeys(entrada) {
        Ok((_, c)) => c,
        Err(e) => {
            journal_lockey_devolver(tipo, &slots, &originais);
            return e;
        }
    };
    if controle != 0 {
        journal_lockey_devolver(tipo, &slots, &originais);
        return -4;
    }
    if !rtti::red_string_write_inline(slots[0], "bwms_teste_6") {
        journal_lockey_devolver(tipo, &slots, &originais);
        return -5;
    }
    let (campos, convertidos) = match journal_converter_lockeys(entrada) {
        Ok(v) => v,
        Err(e) => {
            journal_lockey_devolver(tipo, &slots, &originais);
            return e;
        }
    };
    let lido = rtti::read_cstring(slots[0]).unwrap_or_default();
    let esperado = format!("{LOCKEY_PREFIXO}{}", bwms_primitives::fnv1a64(b"bwms_teste_6"));
    let heap = (slots[0].add(0x14) as *const u32).read_unaligned() >= 0x4000_0000;
    journal_lockey_devolver(tipo, &slots, &originais);
    let devolvido = slots.iter().zip(&textos).all(|(s, t)| rtti::read_cstring(*s).unwrap_or_default() == *t);
    let classe = crate::cname::resolve_cname(rtti::type_name_hash(rtti::class_of(entrada)));
    crate::veredito_dev!(
        "[a44-lockey] entrada {classe} ({entrada:p}): {campos} campo(s) (1º original '{}'), controle {controle} convertido(s); depois da troca {convertidos} convertido(s), lido '{lido}' (esperado '{esperado}', heap={heap}); originais devolvidos={devolvido}",
        textos[0]
    );
    if convertidos == 1 && lido == esperado && heap && devolvido { 1 } else { -6 }
}

/// `BwmsJournalLocKeyTest(resource) -> Int32`: teste da 6b (códigos em `journal_lockey_teste`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_journal_lockey_test(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let res = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let r = journal_lockey_teste(res);
    write_uint_ret(out, r as u32 as u64, 4);
}

pub(crate) unsafe fn register_journal_arvore_natives(reg: &Registry, proto: *mut c_void) {
    let ok1 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalChildCount", "BwmsJournalChildCount", corpo_de_prova!(tramp_journal_child_count)))
    };
    let ok2 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalChildId", "BwmsJournalChildId", corpo_de_prova!(tramp_journal_child_id)))
    };
    let ok3 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalResourceRootIsFolder", "BwmsJournalResourceRootIsFolder", corpo_de_prova!(tramp_journal_resource_root_is_folder)))
    };
    let ok4 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalResourceRootChildCount", "BwmsJournalResourceRootChildCount", corpo_de_prova!(tramp_journal_resource_root_child_count)))
    };
    let ok5 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "Int32", "Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalResourceRootChildIdPart", "BwmsJournalResourceRootChildIdPart", corpo_de_prova!(tramp_journal_resource_root_child_id_part)))
    };
    let ok6 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalTreeRootCount", "BwmsJournalTreeRootCount", corpo_de_prova!(tramp_journal_tree_root_count)))
    };
    let ok7 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalTreeRootChildCount", "BwmsJournalTreeRootChildCount", corpo_de_prova!(tramp_journal_tree_root_child_count)))
    };
    let ok8 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "Int32", "Int32", "Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalTreeRootChildIdPart", "BwmsJournalTreeRootChildIdPart", corpo_de_prova!(tramp_journal_tree_root_child_id_part)))
    };
    let ok9 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalTreeIndexCount", "BwmsJournalTreeIndexCount", corpo_de_prova!(tramp_journal_tree_index_count)))
    };
    let ok10 = {
        let p = compose_params_from_types(reg, &["String"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalPathHash", "BwmsJournalPathHash", corpo_de_prova!(tramp_journal_path_hash)))
    };
    let ok11 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "Int32", "Int32"]);
        params_compostos(p)
            && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalTreeFolderChildCount", "BwmsJournalTreeFolderChildCount", corpo_de_prova!(tramp_journal_tree_folder_child_count)))
    };
    let ok12 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "Int32", "Int32", "Int32", "Int32"]);
        params_compostos(p)
            && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalTreeFolderChildIdPart", "BwmsJournalTreeFolderChildIdPart", corpo_de_prova!(tramp_journal_tree_folder_child_id_part)))
    };
    let ok13 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "Int32", "Int32", "Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalTreeClassName", "BwmsJournalTreeClassName", corpo_de_prova!(tramp_journal_tree_class_name)))
    };
    let ok14 = registro_de_prova!(register_global(reg, proto, "BwmsJournalMoverGotOk", "BwmsJournalMoverGotOk", corpo_de_prova!(tramp_journal_mover_got_ok)));
    crate::log_dev!(
        "[reg] register_all: journal-44 5c-1 PathHash={ok10} FolderChildCount={ok11} FolderChildIdPart={ok12} ClassName={ok13} MoverGotOk={ok14}"
    );
    let ok15 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalTestEntry", "BwmsJournalTestEntry", corpo_de_prova!(tramp_journal_test_entry)))
    };
    crate::log_dev!("[reg] register_all: journal-44 5c-2 TestEntry={ok15}");
    let ok16 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "handle:IScriptable", "Int32", "Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalMergeDryRun", "BwmsJournalMergeDryRun", corpo_de_prova!(tramp_journal_merge_dry_run)))
    };
    let ok17 = {
        let p = compose_params_from_types(reg, &["Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalMergeStat", "BwmsJournalMergeStat", corpo_de_prova!(tramp_journal_merge_stat)))
    };
    crate::log_dev!("[reg] register_all: journal-44 5d-1 MergeDryRun={ok16} MergeStat={ok17}");
    let ok18 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "handle:IScriptable", "Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalMergeTestEntry", "BwmsJournalMergeTestEntry", corpo_de_prova!(tramp_journal_merge_test_entry)))
    };
    let ok19 = {
        let p = compose_params_from_types(reg, &["Int32"]);
        params_compostos(p)
            && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalMergeTestPathPart", "BwmsJournalMergeTestPathPart", corpo_de_prova!(tramp_journal_merge_test_path_part)))
    };
    let ok20 = registro_de_prova!(register_global(reg, proto, "BwmsJournalMergeTestClass", "BwmsJournalMergeTestClass", corpo_de_prova!(tramp_journal_merge_test_class)));
    crate::log_dev!("[reg] register_all: journal-44 5d-2 MergeTestEntry={ok18} PathPart={ok19} Class={ok20}");
    let ok21 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsJournalLocKeyTest", "BwmsJournalLocKeyTest", corpo_de_prova!(tramp_journal_lockey_test)))
    };
    crate::log_dev!("[reg] register_all: journal-44 6b LocKeyTest={ok21}");
    crate::log_dev!(
        "[reg] register_all: journal-44 journal-arvore ChildCount={ok1} ChildId={ok2} | carga RootIsFolder={ok3} RootChildCount={ok4} RootChildIdPart={ok5} | vivo RootCount={ok6} RootChildCount={ok7} RootChildIdPart={ok8} IndexCount={ok9}"
    );
}

/// `BwmsQuestPhaseNodeCount(resource) -> Int32`: nós do grafo da fase; `-1` se não for `questQuestPhaseResource` com grafo legível.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_questphase_node_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let res = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let n = questphase_grafo_de(res)
        .and_then(|graph| {
            let base = graph.add(GRAPHDEF_NODES_OFF);
            let cap = (base.add(0x08) as *const u32).read_unaligned();
            let tam = (base.add(0x0C) as *const u32).read_unaligned();
            (tam <= cap && tam < 1 << 20).then_some(tam as i32)
        })
        .unwrap_or(-1);
    write_uint_ret(out, n as u32 as u64, 4);
}

/// `BwmsQuestPhaseConnectionEdit(res, acao) -> Int32`: 1 tira 1 conexão, 2 recoloca, 3 controle, 4 cria 1 conexão nova, 5 desfaz a criada, 6 cria 1 nó
/// com 1 socket, 7 desfaz o nó, 8 `PatchPhase` de teste (entrada 17, saída 9), 9 o mesmo com `intercept`, 10 desfaz o patch, 11 controle com nó de entrada
/// inexistente; 12–16 patches aplicados na carga pela tabela (ver `questphase_patch_carga_acao`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_questphase_connection_edit(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let res = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let acao = args.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(0);
    let r = match acao {
        1 => questphase_conexao_tirar(res),
        2 => questphase_conexao_recolocar(),
        3 => questphase_conexao_controle(res),
        4 => questphase_conexao_criar(res),
        5 => questphase_conexao_desfazer_criada(),
        6 => questphase_no_criar(res),
        7 => questphase_no_desfazer(),
        8 | 9 => questphase_patch_teste(res, &PATCH_TESTE_ENTRADA, acao == 9),
        10 => questphase_patch_desfazer(),
        11 => questphase_patch_teste(res, &[999], false),
        12..=16 => questphase_patch_carga_acao(res, acao),
        _ => 0,
    };
    write_uint_ret(out, r as u32 as u64, 4);
}

/// `BwmsQuestPhaseGraphStat(res, qual, desloc) -> Int32`: a estatística `qual` (0..6, ver `questphase_grafo_estatistica`); -1 inválido.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_questphase_graph_stat(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let res = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let qual = args.get(1).map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let desloc = args.get(2).map(|(v, _)| *v as u32 as usize).unwrap_or(0).min(16);
    let v = questphase_grafo_estatistica(res, desloc).and_then(|e| e.get(qual).copied()).unwrap_or(-1);
    write_uint_ret(out, v as u32 as u64, 4);
}


/// Lê `capacity`(u32)@+0x8/`size`(u32)@+0xC de um `DynArray<T>` (16 bytes) num offset de
/// instância — guarda de leitura cobrindo o array inteiro antes de ler qualquer campo. Devolve
/// `(size, capacity)`.
pub(crate) unsafe fn read_dynarray_len(ptr: *mut c_void, array_offset: usize) -> Option<(u32, u32)> {
    if !crate::memoria::is_readable((ptr as *const u8).add(array_offset) as *const c_void, 16) {
        return None;
    }
    let capacity = rd_u32(ptr, array_offset + 0x08);
    let size = rd_u32(ptr, array_offset + 0x0C);
    Some((size, capacity))
}

// ===== Journal de mod, parte 3 — árvore do journal (2026-09-11) =====
// Layout pelo registro RTTI (DATABASE, "Layout da árvore do journal"): `gameJournalContainerEntry.entries` em `0x78`
// (`DynArray<Handle<JournalEntry>>`, 16 B por entrada) e `gameJournalEntry.id` (String) em `0x40`. Só leitura; a prova compara
// contagem e ids com o `JournalManager.GetChildren` vanilla.
pub(crate) const JOURNAL_CONTAINER_ENTRIES_OFF: usize = 0x78;

pub(crate) const JOURNAL_ENTRY_ID_OFF: usize = 0x40;

#[cfg(feature = "devtools")]
pub(crate) const JOURNALTREE_INDICE_HASH_COUNT_OFF: usize = 0x78;

#[cfg(feature = "devtools")]
pub(crate) const JOURNALTREE_INDICE_ENTRADA_COUNT_OFF: usize = 0xa8;

pub(crate) const GRAPHDEF_NODES_OFF: usize = 0x30;

pub(crate) const QUESTNODE_ID_OFF: usize = 0x40;

/// Instâncias legíveis (0x60 B) de um `DynArray<Handle<T>>` em `base+off` (16 B por elemento); vazio se incoerente.
pub(crate) unsafe fn handles_legiveis_em(base: *const u8, off: usize, teto: usize) -> Vec<*const u8> {
    let arr = base.add(off);
    let ptr = (arr as *const u64).read_unaligned() as *const u8;
    let cap = (arr.add(0x08) as *const u32).read_unaligned() as usize;
    let n = (arr.add(0x0C) as *const u32).read_unaligned() as usize;
    if n == 0 || n > cap || n > teto || ptr.is_null() || !crate::memoria::is_readable(ptr as *const c_void, n * 16) {
        return Vec::new();
    }
    (0..n)
        .map(|i| (ptr.add(i * 16) as *const u64).read_unaligned() as *const u8)
        .filter(|p| rtti::sane(*p as *mut c_void) && crate::memoria::is_readable(*p as *const c_void, 0x60))
        .collect()
}

/// Tira o elemento `i` (16 B) de um array de Handles, deslocando os seguintes; devolve os 16 B.
pub(crate) unsafe fn handles_tira(base: *const u8, off: usize, i: usize) -> [u64; 2] {
    let (ptr, _, tam) = handles_arr_em(base, off);
    let h = [ptr.add(i * 2).read_unaligned(), ptr.add(i * 2 + 1).read_unaligned()];
    std::ptr::copy(ptr.add((i + 1) * 2), ptr.add(i * 2), (tam as usize - i - 1) * 2);
    (base.add(off + 0x0C) as *mut u32).write_unaligned(tam - 1);
    h
}

/// Põe os 16 B na posição `i`, deslocando os seguintes; `false` sem capacidade.
pub(crate) unsafe fn handles_insere(base: *const u8, off: usize, i: usize, h: [u64; 2]) -> bool {
    let (ptr, cap, tam) = handles_arr_em(base, off);
    if ptr.is_null() || tam >= cap || i > tam as usize {
        return false;
    }
    std::ptr::copy(ptr.add(i * 2), ptr.add((i + 1) * 2), (tam as usize - i) * 2);
    ptr.add(i * 2).write_unaligned(h[0]);
    ptr.add(i * 2 + 1).write_unaligned(h[1]);
    (base.add(off + 0x0C) as *mut u32).write_unaligned(tam + 1);
    true
}

// Fase de quest, parte 35c-2c-2 (2026-09-12): 1 nó novo (`questInputNodeDefinition`) com 1 socket (`questSocketDefinition`, `Output`) num grafo de fase
// carregada que não executa (epilogue), desfeito na mesma execução. Os dois objetos saem do `CreateInstance` (AL-2c); o id do nó é o maior do grafo + 1;
// id, nome e tipo vão pelo RTTI; o dono do socket, pelo set-owner vanilla `0x10494e068(socket, nó)`. O socket entra no `sockets` do nó e o nó no `nodes`
// do grafo pelo reserve do motor, com os movers de `THandle<GraphSocketDefinition>` (GOT `0x1077a73f8`) e `THandle<GraphNodeDefinition>` (GOT
// `0x1077a73f0`). Desfazer tira o nó do `nodes` e solta os handles da native, nó primeiro: o destrutor do nó solta o `sockets`, e o hook da destruição
// genérica diz se os dois foram destruídos pelo motor.
pub(crate) const GRAPHNODE_MOVER_GOT_VM: u64 = 0x1_077a_73f0;

pub(crate) const GRAPHNODE_MOVER_VM: u64 = 0x1_0494_a470;

/// Garante 1 posição livre no `DynArray<Handle>` em `base+off` pelo reserve do motor (`tam + 1`; mover conferido no GOT, nulo com o array vazio).
pub(crate) unsafe fn handles_folga(base: *const u8, off: usize, got_vm: u64, mover_vm: u64) -> bool {
    let (_, cap, tam) = handles_arr_em(base, off);
    if tam < cap {
        return true;
    }
    let mover = (crate::rebase(got_vm) as *const u64).read_unaligned();
    if mover != crate::rebase(mover_vm) as u64 {
        return false;
    }
    crate::jogo::listas::reservar(base.add(off) as *mut c_void, tam + 1, 0x10, 8, if tam == 0 { std::ptr::null() } else { mover as *const c_void });
    let (ptr, cap_nova, _) = handles_arr_em(base, off);
    !ptr.is_null() && cap_nova > tam
}

/// Índice do objeto `obj` no `DynArray<Handle>` em `base+off`.
pub(crate) unsafe fn handle_indice_em(base: *const u8, off: usize, obj: *const u8, teto: u32) -> Option<usize> {
    let (ptr, cap, tam) = handles_arr_em(base, off);
    if ptr.is_null() || tam > cap || tam > teto {
        return None;
    }
    (0..tam as usize).find(|i| ptr.add(i * 2).read_unaligned() == obj as u64)
}

/// Resolve o contexto (game thread, idempotente); `false` com offset diferente da RE, classe ou enum faltando, campo da conexão que não seja
/// `WeakHandle`, mover diferente no GOT ou sem a AL-2c.
pub(crate) unsafe fn patch_ctx_iniciar() -> bool {
    let Ok(mut guarda) = PATCH_CTX.lock() else { return false };
    if guarda.is_some() {
        return true;
    }
    let Some(reg) = rtti::Registry::obtain() else { return false };
    let (no, socket, conexao) = (reg.class_by_name("questPhaseNodeDefinition"), reg.class_by_name("questSocketDefinition"), reg.class_by_name("graphGraphConnectionDefinition"));
    if no.is_null() || socket.is_null() || conexao.is_null() {
        crate::veredito_dev!("[a35-patch] contexto recusado: classe não achada");
        return false;
    }
    let off = |c: *mut c_void, n: &str| {
        let p = rtti::find_property_in_class(c, n);
        rtti::sane(p).then(|| rd_u32(p as *const c_void, 0x20) as usize)
    };
    let conferidos = [
        ("id", off(no, "id"), QUESTNODE_ID_OFF),
        ("sockets", off(no, "sockets"), GRAPHNODE_SOCKETS_OFF),
        ("phaseGraph", off(no, "phaseGraph"), QUESTPHASENODE_GRAFO_OFF),
        ("phaseResource", off(no, "phaseResource"), QUESTPHASENODE_RECURSO_OFF),
        ("name", off(socket, "name"), QUESTSOCKET_NOME_OFF),
        ("type", off(socket, "type"), QUESTSOCKET_TIPO_OFF),
        ("connections", off(socket, "connections"), GRAPHSOCKET_CONNECTIONS_OFF),
        ("source", off(conexao, "source"), GRAPHCONNECTION_SOURCE_OFF),
        ("destination", off(conexao, "destination"), GRAPHCONNECTION_DESTINATION_OFF),
    ];
    let divergentes: Vec<String> = conferidos.iter().filter(|(_, lido, re)| *lido != Some(*re)).map(|(n, lido, re)| format!("{n} {lido:?} ≠ {re:#x}")).collect();
    let tipo_prop = rtti::find_property_in_class(socket, "type");
    let tipo_largura = if rtti::sane(tipo_prop) { rtti::type_size(rd_u64(tipo_prop as *const c_void, 0) as *mut c_void) as usize } else { 0 };
    let (tipo_in, tipo_out) = (rtti::resolve_enum_value(&reg, "questSocketType", "Input"), rtti::resolve_enum_value(&reg, "questSocketType", "Output"));
    let weak = ertti_da_propriedade(conexao, "source") == 10 && ertti_da_propriedade(conexao, "destination") == 10;
    let movers = [(GRAPHNODE_MOVER_GOT_VM, GRAPHNODE_MOVER_VM), (GRAPHSOCKET_MOVER_GOT_VM, GRAPHSOCKET_MOVER_VM), (GRAPHCONNECTION_MOVER_GOT_VM, GRAPHCONNECTION_MOVER_VM)]
        .iter()
        .all(|(g, m)| (crate::rebase(*g) as *const u64).read_unaligned() == crate::rebase(*m) as u64);
    let ok = divergentes.is_empty() && matches!(tipo_largura, 1 | 2 | 4 | 8) && tipo_in.is_some() && tipo_out.is_some() && weak && movers;
    crate::veredito_dev!(
        "[a35-patch] contexto: offsets {}; socket type {tipo_largura} B (Input {tipo_in:?}, Output {tipo_out:?}); conexão com WeakHandle {weak}; movers {movers} -> {}",
        if divergentes.is_empty() { String::from("iguais à RE") } else { divergentes.join(", ") },
        if ok { "pronto" } else { "recusado" }
    );
    if let (true, Some(tipo_in), Some(tipo_out)) = (ok, tipo_in, tipo_out) {
        *guarda = Some(PatchCtx { cls_no: no as usize, cls_socket: socket as usize, cls_conexao: conexao as usize, tipo_in, tipo_out, tipo_largura });
    }
    ok
}

/// `obj` legível com esse vptr.
pub(crate) unsafe fn vptr_e(obj: *const u8, vm: u64) -> bool {
    rtti::sane(obj as *mut c_void) && crate::memoria::is_readable(obj as *const c_void, 8) && (obj as *const u64).read_unaligned() == crate::rebase(vm) as u64
}

pub(crate) struct PatchAplicado {
    pub(crate) graph: usize,
    pub(crate) no: usize,
    pub(crate) dono_no: [u64; 2],
    /// `(nó, socket, dono)` de cada socket criado.
    pub(crate) sockets: Vec<(usize, usize, [u64; 2])>,
    /// `(socket de saída, socket de entrada, objeto, dono)` de cada conexão criada.
    pub(crate) conexoes: Vec<(usize, usize, usize, [u64; 2])>,
    /// `(socket, índice, 16 B)` de cada conexão tirada pelo `intercept`, na ordem da retirada.
    pub(crate) tiradas: Vec<(usize, usize, [u64; 2])>,
    /// `(bloco, strong)` de cada socket que já existia e ganhou conexão, lido antes.
    pub(crate) tocados: Vec<(usize, u32)>,
    /// `WeakHandle` em que o strong temporário não pôde ser devolvido (bloco com strong < 2).
    pub(crate) weak_com_strong: usize,
}

/// `ResolveSocket`: socket do nó com esse tipo e nome; sem ele, cria (reserve do `sockets`, dono pelo set-owner vanilla) e guarda em `p.sockets`.
pub(crate) unsafe fn fase_socket_resolve(ctx: &PatchCtx, no: *const u8, tipo: u64, nome: &str, p: &mut PatchAplicado) -> Result<*const u8, i32> {
    let h = cname(nome);
    let achado = handles_legiveis_em(no, GRAPHNODE_SOCKETS_OFF, 256).into_iter().find(|s| {
        vptr_e(*s, QUESTSOCKET_VPTR_VM) && inteiro_le(*s, QUESTSOCKET_TIPO_OFF, ctx.tipo_largura) == tipo && (s.add(QUESTSOCKET_NOME_OFF) as *const u64).read_unaligned() == h
    });
    if let Some(s) = achado {
        if let Some(i) = handle_indice_em(no, GRAPHNODE_SOCKETS_OFF, s, 256) {
            let bloco = handles_arr_em(no, GRAPHNODE_SOCKETS_OFF).0.add(i * 2 + 1).read_unaligned() as usize;
            if bloco != 0 && !p.tocados.iter().any(|(b, _)| *b == bloco) {
                p.tocados.push((bloco, (bloco as *const u32).read_unaligned()));
            }
        }
        return Ok(s);
    }
    if !handles_folga(no, GRAPHNODE_SOCKETS_OFF, GRAPHSOCKET_MOVER_GOT_VM, GRAPHSOCKET_MOVER_VM) {
        return Err(-4);
    }
    let (s, mut dono) = patch_objeto_novo(ctx.cls_socket, QUESTSOCKET_VPTR_VM)?;
    (s.add(QUESTSOCKET_NOME_OFF) as *mut u64).write_unaligned(h);
    inteiro_grava(s, QUESTSOCKET_TIPO_OFF, ctx.tipo_largura, tipo);
    let set_owner: unsafe extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(crate::rebase(SOCKET_SET_OWNER_VM));
    set_owner(s as *mut c_void, no as *mut c_void);
    let (_, _, tam) = handles_arr_em(no, GRAPHNODE_SOCKETS_OFF);
    handle_strong_soma(&dono, 1);
    if !handles_insere(no, GRAPHNODE_SOCKETS_OFF, tam as usize, dono) {
        handle_strong_soma(&dono, -1);
        let _ = crate::jogo::objetos::soltar_handle_contando(&mut dono);
        return Err(-4);
    }
    p.sockets.push((no as usize, s as usize, dono));
    Ok(s)
}

/// Patches aplicados na carga (pra consulta e desfazer) e o código da última tentativa.
pub(crate) static PATCHES_NA_CARGA: std::sync::Mutex<Vec<PatchAplicado>> = std::sync::Mutex::new(Vec::new());

pub(crate) static PATCH_CARGA_ULTIMO: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

pub(crate) const FACTS_STORE_NOMEADO: u32 = 1;

/// Caminho do `.quest` → ids dos nós de fase de mod forçados nele.
pub(crate) static FASES_FORCADAS: Mutex<std::collections::BTreeMap<u64, std::collections::BTreeSet<u16>>> = Mutex::new(std::collections::BTreeMap::new());

/// FactManager do QuestsSystem com a vtable conferida slot a slot contra a RE; nulo se algo diverge.
pub(crate) unsafe fn factmanager_de(sys: *const c_void) -> *mut c_void {
    if sys.is_null() || !crate::memoria::is_readable((sys as usize + FACTMANAGER_OFF) as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    let fm = rd_u64(sys, FACTMANAGER_OFF) as *mut c_void;
    if fm.is_null() || !crate::memoria::is_readable(fm as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    let vt = rd_u64(fm, 0) as *const c_void;
    let ok = !vt.is_null()
        && crate::memoria::is_readable(vt, 0x50)
        && rd_u64(vt, 0x18) == crate::rebase(FACT_GET_VM) as u64
        && rd_u64(vt, 0x20) == crate::rebase(FACT_SET_VM) as u64
        && rd_u64(vt, 0x48) == crate::rebase(FACT_REMOVER_VM) as u64;
    if ok { fm } else { std::ptr::null_mut() }
}

/// `GetFact` do store nomeado; `None` sem FactManager conferido.
pub(crate) unsafe fn fact_ler(sys: *const c_void, fact: u32) -> Option<i32> {
    let fm = factmanager_de(sys);
    if fm.is_null() {
        return None;
    }
    let f: extern "C" fn(*mut c_void, u32, u32) -> i32 = std::mem::transmute(crate::rebase(FACT_GET_VM));
    Some(f(fm, FACTS_STORE_NOMEADO, fact))
}

/// Caminho de nós da chave no NodeHashMap, varrido pelos baldes.
pub(crate) unsafe fn nodehashmap_caminho(sys: *const c_void, chave: u32) -> Option<Vec<u16>> {
    crate::register::mapa_do_motor_nos(sys as usize + NODEHASHMAP_OFF).into_iter().find(|n| ((n + 8) as *const u32).read_unaligned() == chave).map(|n| {
        let (e, t, _) = cust_arr(n + 0x10);
        if e == 0 || t > 64 || !crate::memoria::is_readable(e as *const c_void, t * 2) {
            Vec::new()
        } else {
            (0..t).map(|i| ((e + i * 2) as *const u16).read_unaligned()).collect()
        }
    })
}

/// Confere uma vez o `Assign` dos tipos (`rtti::type_assign`) numa String e num Vector3 temporários construídos pelo tipo: o destino recebe o valor e a
/// origem fica igual. A String sai do campo `id` da entrada; o Vector3, do 1º campo desse tipo nas propriedades editáveis dela. No boot 85 o Vector3 tirado
/// do `offset` de `gameJournalQuestMapPinBase` pelo nome da classe não conferiu e nada foi editado; o texto do log agora leva tipos e tamanhos.
pub(crate) unsafe fn assign_confere(entrada: *mut c_void) -> bool {
    use std::sync::atomic::Ordering;
    match ASSIGN_CONFERIDO.load(Ordering::Relaxed) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    let tipo_s = tipo_string_da_entrada(entrada);
    let vetor = cname("Vector3");
    let tipo_v = journal_campos_editaveis(rtti::class_of(entrada)).into_iter().map(|(t, _)| t).find(|t| rtti::type_name_getname(*t) == vetor);
    let Some(tipo_v) = tipo_v else {
        // Entrada sem campo Vector3: sem o 2º tipo a conferência fica pra próxima edição, e esta não edita.
        crate::veredito_dev!("[a44-7e4] Assign dos tipos: a entrada não tem campo Vector3 pra conferir (tipo String {tipo_s:p})");
        return false;
    };
    let (tam_s, tam_v) = (rtti::type_size(tipo_s), rtti::type_size(tipo_v));
    let mut texto = format!("tipo String {tipo_s:p} tamanho {tam_s:#x}, tipo Vector3 {tipo_v:p} tamanho {tam_v:#x}");
    let ok = rtti::sane(tipo_s) && rtti::sane(tipo_v) && tam_s == 0x20 && tam_v == 12 && {
        let (a, na) = rtti::type_inst_alloc(tipo_s);
        let (b, nb) = rtti::type_inst_alloc(tipo_s);
        let s_ok = !a.is_null()
            && !b.is_null()
            && rtti::red_string_write_inline(b as *mut u8, "bwms_assign")
            && rtti::type_assign(tipo_s, a, b)
            && rtti::read_cstring(a as *const u8).as_deref() == Some("bwms_assign")
            && rtti::read_cstring(b as *const u8).as_deref() == Some("bwms_assign");
        rtti::type_inst_free(tipo_s, a, na);
        rtti::type_inst_free(tipo_s, b, nb);
        let (c, nc) = rtti::type_inst_alloc(tipo_v);
        let (d, nd) = rtti::type_inst_alloc(tipo_v);
        let valores = [1.5f32, -2.25, 3.0];
        let v_ok = !c.is_null() && !d.is_null() && {
            for (k, x) in valores.iter().enumerate() {
                (d as *mut f32).add(k).write_unaligned(*x);
            }
            rtti::type_assign(tipo_v, c, d) && f32x3(c as *const u8, 0) == valores && f32x3(d as *const u8, 0) == valores
        };
        rtti::type_inst_free(tipo_v, c, nc);
        rtti::type_inst_free(tipo_v, d, nd);
        texto = format!("{texto}; String {s_ok}, Vector3 {v_ok}");
        s_ok && v_ok
    };
    ASSIGN_CONFERIDO.store(if ok { 1 } else { 2 }, Ordering::Relaxed);
    crate::veredito_dev!("[a44-7e4] Assign dos tipos (vt+0x58) conferido: {ok} ({texto})");
    ok
}

pub(crate) const QUESTCTX_TAM: usize = 0x340;

/// Buffer do contexto com o alinhamento que os `stp q0, q0` do construtor pedem.
#[repr(align(16))]
pub(crate) struct BufQuestCtx(pub(crate) [u8; QUESTCTX_TAM]);

// O `ExecuteNode` num nó SINTÉTICO, 2026-09-18.
//
// Com o `QuestContext` provado ao vivo (boots 223/224), falta a 2ª metade da receita: o `ExecuteNode(fase, nó, ctx, &socket, saída)` do motor
// (`0x10280df0c`). Ele pega o tipo do nó pelo slot `+0x8`, e — fora da família testada no `IsA` — chama o thunk `0x10286b874`, que despacha o
// **`Execute` do próprio nó no slot `+0x158`** da vtable dele, com o `phaseContext` (`ctx+0x110`).
//
// O nó escolhido é o `questInputNodeDefinition` (0x50 B), porque o `Execute` dele (`0x102834424`, achado pelos fixups: vtable da instância
// `0x1079e9770`, `GetType` `0x10284390c` no `+0x8`) decide tudo por UM byte do socket (`[socket+8]`): modo 1 devolve 1 **sem chamar mais nada**;
// modos 2 e 3 chamam o slot `+0x180` do nó (que este teste não toca); qualquer outro devolve 0, também sem efeito. Então o `ExecuteNode` com o socket no
// modo 1 percorre a receita inteira — contexto real, nó despachado pelo motor ao `Execute` da classe certa, resultado devolvido — com ZERO efeito
// colateral. O modo 0, chamado direto no `Execute`, é o controle negativo, igualmente sem efeito. Nada disso toca progresso de quest nem grava fact.
pub(crate) const EXECUTENODE_VM: u64 = 0x1_0280_df0c;

pub(crate) const QUESTS_TRAVA_ESCRITA: u8 = 0x80;


pub(crate) unsafe fn recurso_da_busca(b: &BuscaCozida, sys: *mut c_void) -> *mut c_void {
    if sys.is_null() || !crate::memoria::is_readable(sys as *const c_void, b.recurso_off + 8) {
        return std::ptr::null_mut();
    }
    ((sys as *const u8).add(b.recurso_off) as *const u64).read_unaligned() as *mut c_void
}

/// Registra um mappin de mod na busca `b`, na `posicao` dada (7d-2b-2b: a do nó do NodeRef) ou, sem ela, na do 1º elemento cozido + 1 m em Z (7d-2a), com
/// o Handle `volume` (7e-3c; `[0, 0]` sem volume) cujo strong passa pra tabela — em recusa ele é solto aqui. 1 = ok; -1 sem o hook; -2 sem recurso ou
/// elemento; -3 o hash já existe no recurso ou na tabela; -9 trava.
pub(crate) unsafe fn mappin_de_mod_registrar(b: &BuscaCozida, sys: *mut c_void, hash: u32, posicao_dada: Option<[f32; 3]>, volume: [u64; 2]) -> i32 {
    let recusa = |codigo: i32| {
        if volume[0] != 0 {
            let mut v = volume;
            let _ = crate::jogo::objetos::soltar_handle_contando(&mut v);
        }
        codigo
    };
    let orig = b.orig.load(Ordering::Relaxed);
    if orig == 0 {
        return recusa(-1);
    }
    let res = recurso_da_busca(b, sys);
    let Some((ptr, _, tam)) = cookeddata_de(res) else { return recusa(-2) };
    if ptr.is_null() || tam == 0 || !crate::memoria::is_readable(ptr as *const c_void, 0x20) {
        return recusa(-2);
    }
    let f: MappinBuscaFn = std::mem::transmute(orig);
    if !f(res, hash).is_null() {
        return recusa(-3);
    }
    let p = ptr.add(b.posicao_off) as *const f32;
    let posicao = posicao_dada.unwrap_or([p.read_unaligned(), p.add(1).read_unaligned(), p.add(2).read_unaligned() + 1.0]);
    let poi = std::ptr::eq(b, &BUSCA_POI);
    let Ok(mut t) = MAPPINS_DE_MOD.lock() else { return recusa(-9) };
    if t.iter().any(|m| m.hash == hash && m.poi == poi) {
        drop(t);
        return recusa(-3);
    }
    t.push(MappinDeMod { hash, posicao, poi, volume });
    MAPPIN_MODS.fetch_add(1, Ordering::Relaxed);
    crate::veredito_dev!(
        "[mappin-mod] registrado hash {hash:#x} {} na posição {posicao:?}, volume {}",
        if poi { "de POI" } else { "de mappin" },
        volume[0] != 0
    );
    1
}


// ===== `QuestRootInstance::Start` (2026-09-18) =====
// `0x102811df0` (slot `+0xe8` do `questRootInstance`, `(root, QuestContext*, &Handle<questQuestResource>)`), chamado só pelo processador da fila de
// início (`0x102a0fb54`, Tick do QuestsSystem) com as travas `sys+0x128`/`+0x60` seguras. Antes da original, o hook marca os facts dos
// nós forçados (`register::questphase_forcados_no_inicio`, com o QuestsSystem do contexto em `ctx+0x128`) e segue pra original. Com o recurso do teste
// e sem a fase filha dele (o `Start` criaria uma 2ª instância da quest), não segue.
pub(crate) const A34_START: (u64, u32) = (0x1_0281_1df0, 0xd102_03ff);

pub(crate) const QUESTSYS_VTBL_VM: u64 = 0x1_07a2_f268;

pub(crate) static ORIG_A34_START: AtomicU64 = AtomicU64::new(0);

pub(crate) unsafe extern "C" fn a34_start(root: u64, ctx: u64, recurso: u64) -> u64 {
    A34_CHAMADAS.fetch_add(1, Ordering::Relaxed);
    let sys = if ctx != 0 && crate::memoria::is_readable((ctx + 0x128) as *const c_void, 8) { ler_u64(ctx + 0x128) } else { 0 };
    if sys != 0 && crate::memoria::is_readable(sys as *const c_void, 8) && ler_u64(sys) == crate::rebase(QUESTSYS_VTBL_VM) as u64 {
        A34_FACTS.fetch_add(crate::mundo::questphase_forcados_no_inicio(sys as *const c_void) as u64, Ordering::Relaxed);
    }
    let res = if recurso != 0 && crate::memoria::is_readable(recurso as *const c_void, 8) { ler_u64(recurso) } else { 0 };
    // Quest nova: os patches da tabela no `.quest` antes do grafo rodar (o `.quest` não passa pela rota de fase).
    if res != 0 && crate::memoria::is_readable((res + 0x30) as *const c_void, 8) {
        crate::mundo::questphase_patches_na_carga(ler_u64(res + 0x30), recurso as *mut c_void);
    }
    // O recurso de teste do `Start` só é gravado pelo selftest.
    #[cfg(feature = "devtools")]
    {
        let teste = A34_RECURSO_TESTE.load(Ordering::Relaxed);
        if teste != 0 && res == teste {
            let filho: unsafe extern "C" fn(u64, u64) -> u64 = std::mem::transmute(crate::rebase(A34_FILHO_DO_RECURSO_VM));
            if filho(root, res) == 0 {
                A34_RECUSADAS.fetch_add(1, Ordering::Relaxed);
                return 0;
            }
        }
    }
    let f: unsafe extern "C" fn(u64, u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_A34_START));
    f(root, ctx, recurso)
}

/// Recriação das quests na carga de save (`0x102812f24`, gêmea do `Start` sem executar grafo, em thread de job, um `.quest` por vez na ordem da
/// QuestList): os patches da tabela no `.quest` antes da instância ser recriada. Nó de mod num `.quest` entra no
/// registro de forçados (`register::questphase_patches_na_carga`) e o `OnGameRestored` o força.
pub(crate) const A35_RECRIAR: (u64, u32) = (0x1_0281_2f24, 0xd102_03ff);

pub(crate) static ORIG_A35_RECRIAR: AtomicU64 = AtomicU64::new(0);

pub(crate) unsafe extern "C" fn a35_recriar(root: u64, ctx: u64, recurso: u64) -> u64 {
    A35_RECRIADAS.fetch_add(1, Ordering::Relaxed);
    let res = if recurso != 0 && crate::memoria::is_readable(recurso as *const c_void, 8) { ler_u64(recurso) } else { 0 };
    if res != 0 && crate::memoria::is_readable((res + 0x30) as *const c_void, 8) {
        crate::mundo::questphase_patches_na_carga(ler_u64(res + 0x30), recurso as *mut c_void);
    }
    let f: unsafe extern "C" fn(u64, u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_A35_RECRIAR));
    f(root, ctx, recurso)
}

/// Na carga da tabela de patches de fase (`mod_pipeline`) e com o marcador do teste; um só ponto de instalação.
pub(crate) unsafe fn install_a35_recriar() {
    if ORIG_A35_RECRIAR.load(Ordering::Relaxed) != 0 {
        return;
    }
    let h = instalar_conferindo_prologo(A35_RECRIAR.0, A35_RECRIAR.1, a35_recriar as *mut c_void, &ORIG_A35_RECRIAR);
    crate::veredito_dev!("[a35] hook da recriação das quests na carga {h:?}");
}

/// No `on_load` com o marcador do teste, e na carga da tabela de patches de fase (`mod_pipeline`); um só ponto de instalação.
pub(crate) unsafe fn install_a34() {
    if ORIG_A34_START.load(Ordering::Relaxed) != 0 {
        return;
    }
    let h = instalar_conferindo_prologo(A34_START.0, A34_START.1, a34_start as *mut c_void, &ORIG_A34_START);
    crate::veredito_dev!("[a34] hook do QuestRootInstance::Start {h:?}");
}

// questphase-apply: QuestsSystem::OnGameRestored candidato — achado 2026-07-28 AO VIVO via
// `getquestsys` (GameInstance.GetQuestsSystem(game) com a ABI corrigida do getcustsys) + leitura
// direta da vtable no slot 0x158 (offset Windows 0x150 de `IGameSystem`,
// +0x08 pela correção Itanium de dtor extra já documentada no
// projeto). `peekq` confirma prólogo REAL (STP X20,X19 + STP X29,X30), não thunk. Ainda não
// confirmado que É OnGameRestored (offset estimado, não literal) — hook observe-only decide:
// deve disparar exatamente 1x por save-load real (autocontinue já dispara 1 nesta sessão).
pub(crate) const QUESTSYS_ONGAMERESTORED_VM: u64 = 0x1_02a0_b26c;

pub(crate) static ORIG_QUESTSYS_ONGAMERESTORED: AtomicU64 = AtomicU64::new(0);

// ===== questphase-apply: QuestsSystem::OnGameRestored candidato C++ hook =====
// ABI C++ ARM64 (virtual, IGameSystem::OnGameRestored): x0=this, retorno bool (w0).
// HookWrap (não HookBefore): propaga o retorno real do original — é um virtual bool,
// alguém pode ramificar no resultado; não arriscar mudar comportamento, só observar.
pub(crate) unsafe extern "C" fn questsys_ongamerestored_handler(this: *mut c_void) -> bool {
    let n = QUESTSYS_ONGAMERESTORED_CNT.fetch_add(1, Ordering::Relaxed);
    // O início forçado dos nós de mod, antes da original. O teste de fase de quest adia pra depois da trava de
    // save (o `OnGameRestored` roda antes dela) e chama a mesma função.
    if marcador_individual("a35-teste") {
        A35_RESTAURACOES_ADIADAS.fetch_add(1, Ordering::Relaxed);
    } else if !this.is_null() && crate::memoria::is_readable(this as *const c_void, 8) && (this as *const u64).read_unaligned() == crate::rebase(QUESTSYS_VTBL_VM) as u64 {
        A55_FORCADOS_NA_RESTAURACAO.fetch_add(crate::mundo::questphase_forcar_na_restauracao(this) as u64, Ordering::Relaxed);
    }
    let f: unsafe extern "C" fn(*mut c_void) -> bool = std::mem::transmute(original_publicado(&ORIG_QUESTSYS_ONGAMERESTORED));
    let ret = f(this);
    if n < 8 {
        crate::log_dev!("[questphase] QuestsSystem::OnGameRestored-cand#{n} this={this:p} ret={ret}");
    }
    ret
}

/// Um só ponto de instalação do hook do `OnGameRestored` (o `install_hook_probe!` não reinstala com o original já publicado): `install_postload_hooks`,
/// a tabela de patches de fase (`mod_pipeline`) e o marcador do teste.
pub(crate) unsafe fn install_questsys_ongamerestored() {
    install_hook_probe!(
        QUESTSYS_ONGAMERESTORED_VM, questsys_ongamerestored_handler, ORIG_QUESTSYS_ONGAMERESTORED,
        &format!("[questphase] QuestsSystem::OnGameRestored hook @ {QUESTSYS_ONGAMERESTORED_VM:#012x}"),
        "[questphase] FALHA OnGameRestored hook"
    );
}

// ===== Journal de mod, partes 7a/7b — busca de mappin cozido (2026-09-12) =====
// A busca `0x1030f2ff0(MappinResource, hash)` percorre `cookedData` (entradas `@0x40`, tamanho `@0x4c`, elemento de 0x20 B com `journalPathHash` u32
// `@0`) e devolve o elemento ou 0. `GetMappinData` (`0x1030f7920`) é um wrapper de 16 B que salta pra ela, e o slot `0x260` do `MappinSystem`
// (`0x103106d24`) chama a busca direto e copia a posição de `+0x04..+0x10`. O hook fica na busca (72 B; o `cbz` do prólogo o relocador conserta).
// 7a: conta buscas e faltas. 7b: pra um hash de teste que falta, devolve um elemento do BWMS — cópia de um elemento cozido com o hash trocado, que vive
// até o fim do processo —, conferido pelo slot `0x260` vanilla. O `GetQuestMappinPosition` do script NÃO passa pela busca (boot 62): o handler
// `0x10310a928` chama `vt[0x288]` = `0x103108764`, que procura o mappin de runtime por `vt[0x290]` e lê a posição do `IMappin` (`vt[0x110]`).
// Marcador `~/.bwms-hook-mappin-busca`.
pub(crate) const MAPPIN_BUSCA_VM: u64 = 0x1_030f_2ff0;

pub(crate) type MappinBuscaFn = unsafe extern "C" fn(*mut c_void, u32) -> *const u8;

// Parte 7c (2026-09-12): a busca de POI cozido `0x1031138f4` tem o mesmo corpo da de mappin (204 B no 2.5), sobre `PointOfInterestMappinResource.cookedData@0x40`
// (elemento de 0x20 B: `journalPathHash` `@0`, `entityID` `@0x08`, posição `@0x10`); o recurso fica em `MappinSystem+0x68`. O único chamador direto é o
// envelope do `GetPoiData` (`0x1030f7c40`, wrapper de 20 B; `bl busca` em `0x1030f7c70`), que confere a 7c. As duas buscas dividem o código por uma `BuscaCozida`.
pub(crate) const POI_BUSCA_VM: u64 = 0x1_0311_38f4;

/// Estado de uma busca cozida hookada: contadores, tabela de teste, onde fica o recurso no `MappinSystem` e onde fica a posição no elemento.
pub(crate) struct BuscaCozida {
    pub(crate) nome: &'static str,
    pub(crate) orig: AtomicU64,
    pub(crate) buscas: AtomicU64,
    pub(crate) faltas: AtomicU64,
    pub(crate) injetadas: AtomicU64,
    pub(crate) teste: std::sync::Mutex<Vec<(u32, usize)>>,
    pub(crate) recurso_off: usize,
    pub(crate) posicao_off: usize,
}

pub(crate) static BUSCA_MAPPIN: BuscaCozida = BuscaCozida {
    nome: "mappin-busca",
    orig: AtomicU64::new(0),
    buscas: AtomicU64::new(0),
    faltas: AtomicU64::new(0),
    injetadas: AtomicU64::new(0),
    teste: std::sync::Mutex::new(Vec::new()),
    recurso_off: 0x58,
    posicao_off: 0x04,
};

pub(crate) static BUSCA_POI: BuscaCozida = BuscaCozida {
    nome: "poi-busca",
    orig: AtomicU64::new(0),
    buscas: AtomicU64::new(0),
    faltas: AtomicU64::new(0),
    injetadas: AtomicU64::new(0),
    teste: std::sync::Mutex::new(Vec::new()),
    recurso_off: 0x68,
    posicao_off: 0x10,
};

/// Corpo comum dos dois hooks: chama a original; se ela não achou, devolve a entrada de teste desse hash, se houver.
pub(crate) unsafe fn busca_cozida(b: &BuscaCozida, res: *mut c_void, hash: u32) -> *const u8 {
    let f: MappinBuscaFn = std::mem::transmute(original_publicado(&b.orig));
    let r = f(res, hash);
    b.buscas.fetch_add(1, Ordering::Relaxed);
    if !r.is_null() {
        return r;
    }
    let faltas = b.faltas.fetch_add(1, Ordering::Relaxed);
    let empurrado = mappin_de_mod_empurrar(b, res, hash);
    if !empurrado.is_null() {
        return empurrado;
    }
    if let Ok(t) = b.teste.try_lock() {
        if let Some((_, p)) = t.iter().find(|(h, _)| *h == hash) {
            b.injetadas.fetch_add(1, Ordering::Relaxed);
            return *p as *const u8;
        }
    }
    if faltas < 8 {
        crate::veredito_dev!("[{}] falta hash {hash:#x} no recurso {res:p}", b.nome);
    }
    std::ptr::null()
}

pub(crate) unsafe extern "C" fn mappin_busca_replacement(res: *mut c_void, hash: u32) -> *const u8 {
    busca_cozida(&BUSCA_MAPPIN, res, hash)
}

pub(crate) unsafe extern "C" fn poi_busca_replacement(res: *mut c_void, hash: u32) -> *const u8 {
    busca_cozida(&BUSCA_POI, res, hash)
}

// Parte 7d-1 (2026-09-12): folga nos `cookedData` na carga do mundo. Hook no slot 48 do `MappinSystem` (`OnStreamingWorldLoaded`,
// `0x1030fb9f0`, `(this, worldRuntimeScene*)`): depois da original, os `cookedData` de mappin (`+0x58`) e de POI (`+0x68`) sem folga ganham
// `max(2 × mappins de mod, tamanho / 2)` pelo reserve do motor, com os movers conferidos no GOT. Assim o PushBack da 7d não realoca ponteiro já entregue
// pela busca. Marcador `~/.bwms-hook-mappin-reserva`.
pub(crate) const MAPPINSYSTEM_MUNDO_CARREGADO_VM: u64 = 0x1_030f_b9f0;

pub(crate) const COOKEDMAPPIN_MOVER_GOT_VM: u64 = 0x1_077a_88a0;

pub(crate) const COOKEDMAPPIN_MOVER_VM: u64 = 0x1_030f_45b0;

pub(crate) const COOKEDPOI_MOVER_GOT_VM: u64 = 0x1_077a_8f48;

pub(crate) const COOKEDPOI_MOVER_VM: u64 = 0x1_0310_adf0;

pub(crate) static ORIG_MAPPIN_MUNDO: AtomicU64 = AtomicU64::new(0);

/// Mappins de mod coletados (a folga da reserva conta 2 por mappin).
pub(crate) static MAPPIN_MODS: AtomicU64 = AtomicU64::new(0);

pub(crate) type MundoCarregadoFn = unsafe extern "C" fn(*mut c_void, *mut c_void);

/// Dá folga ao `cookedData` (elementos de 0x20 B) do recurso: `(tamanho, capacidade antes, capacidade depois, 1º elemento igual depois, busca do 1º hash
/// devolve o mesmo elemento)`; `None` se o recurso, o array ou o mover não conferem.
pub(crate) unsafe fn cookeddata_folga(res: *mut c_void, got_vm: u64, mover_vm: u64, busca: &BuscaCozida, busca_vm: u64) -> Option<(u32, u32, u32, bool, bool)> {
    if res.is_null() || !crate::memoria::is_readable(res as *const c_void, 0x50) {
        return None;
    }
    let arr = (res as *mut u8).add(0x40);
    let ptr0 = (arr as *const u64).read_unaligned() as *const u8;
    let cap0 = (arr.add(0x08) as *const u32).read_unaligned();
    let tam = (arr.add(0x0C) as *const u32).read_unaligned();
    if ptr0.is_null() || tam == 0 || tam > cap0 || !crate::memoria::is_readable(ptr0 as *const c_void, tam as usize * 0x20) {
        return None;
    }
    let mover = (crate::rebase(got_vm) as *const u64).read_unaligned();
    if mover != crate::rebase(mover_vm) as u64 {
        return None;
    }
    let mut primeiro = [0u8; 0x20];
    std::ptr::copy_nonoverlapping(ptr0, primeiro.as_mut_ptr(), 0x20);
    if tam == cap0 {
        let extra = ((2 * MAPPIN_MODS.load(Ordering::Relaxed)) as u32).max(tam / 2).max(1);
        crate::jogo::listas::reservar(arr as *mut c_void, tam + extra, 0x20, 8, mover as *const c_void);
    }
    let ptr1 = (arr as *const u64).read_unaligned() as *const u8;
    let cap1 = (arr.add(0x08) as *const u32).read_unaligned();
    let tam1 = (arr.add(0x0C) as *const u32).read_unaligned();
    let igual = !ptr1.is_null() && tam1 == tam && std::slice::from_raw_parts(ptr1, 0x20) == &primeiro[..];
    let hash = (primeiro.as_ptr() as *const u32).read_unaligned();
    let orig = busca.orig.load(Ordering::Relaxed);
    let f: MappinBuscaFn = std::mem::transmute(if orig == 0 { crate::rebase(busca_vm) as u64 } else { orig });
    let e = f(res, hash);
    let achou = !e.is_null() && std::slice::from_raw_parts(e, 0x20) == &primeiro[..];
    Some((tam, cap0, cap1, igual, achou))
}

/// `MappinSystem` do último `OnStreamingWorldLoaded` (journal de mod, 7e-3: o tick registra nele os pins do journal de mod).
pub(crate) static MAPPIN_SISTEMA: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(crate) unsafe extern "C" fn mappin_mundo_carregado_replacement(sys: *mut c_void, cena: *mut c_void) {
    let f: MundoCarregadoFn = std::mem::transmute(original_publicado(&ORIG_MAPPIN_MUNDO));
    f(sys, cena);
    if sys.is_null() || !crate::memoria::is_readable(sys as *const c_void, 0x70) {
        return;
    }
    MAPPIN_SISTEMA.store(sys as usize, Ordering::Relaxed);
    let res_m = ((sys as *const u8).add(BUSCA_MAPPIN.recurso_off) as *const u64).read_unaligned() as *mut c_void;
    let res_p = ((sys as *const u8).add(BUSCA_POI.recurso_off) as *const u64).read_unaligned() as *mut c_void;
    let m = cookeddata_folga(res_m, COOKEDMAPPIN_MOVER_GOT_VM, COOKEDMAPPIN_MOVER_VM, &BUSCA_MAPPIN, MAPPIN_BUSCA_VM);
    let p = cookeddata_folga(res_p, COOKEDPOI_MOVER_GOT_VM, COOKEDPOI_MOVER_VM, &BUSCA_POI, POI_BUSCA_VM);
    let texto = |x: Option<(u32, u32, u32, bool, bool)>| {
        x.map_or("sem recurso, array ou mover".to_string(), |(t, a, b, i, h)| format!("tam {t} cap {a} → {b}, 1º elemento igual={i}, busca do 1º hash={h}"))
    };
    let certo = |x: Option<(u32, u32, u32, bool, bool)>| x.map_or(false, |(t, _, b, i, h)| b > t && i && h);
    crate::veredito_dev!("[mappin-reserva] VEREDITO mappin: {}; POI: {}; reserva-ok={}", texto(m), texto(p), certo(m) && certo(p));
}

/// Journal de mod, 7e-5: com tabela de journal de mod, os hooks da busca de mappin, da busca de POI e da reserva entram sem marcador — os pins das entradas
/// novas só chegam ao consumidor vanilla pela busca hookada, e o `MappinSystem` pro tick sai do hook da reserva. Cada hook só instala se o original ainda não
/// foi guardado (o marcador pode ter instalado no `on_load`; hook empilhado duas vezes vira laço).
pub(crate) unsafe fn instalar_hooks_de_pins_de_mod() {
    if BUSCA_MAPPIN.orig.load(Ordering::Relaxed) == 0 {
        install_hook_probe!(
            MAPPIN_BUSCA_VM, mappin_busca_replacement, BUSCA_MAPPIN.orig,
            &format!("[mappin-busca] busca de mappin cozido hook @ {MAPPIN_BUSCA_VM:#012x} (tabela de journal de mod)"),
            "[mappin-busca] FALHA ao hookar a busca de mappin cozido"
        );
    }
    if BUSCA_POI.orig.load(Ordering::Relaxed) == 0 {
        install_hook_probe!(
            POI_BUSCA_VM, poi_busca_replacement, BUSCA_POI.orig,
            &format!("[poi-busca] busca de POI cozido hook @ {POI_BUSCA_VM:#012x} (tabela de journal de mod)"),
            "[poi-busca] FALHA ao hookar a busca de POI cozido"
        );
    }
    if ORIG_MAPPIN_MUNDO.load(Ordering::Relaxed) == 0 {
        install_hook_probe!(
            MAPPINSYSTEM_MUNDO_CARREGADO_VM, mappin_mundo_carregado_replacement, ORIG_MAPPIN_MUNDO,
            &format!("[mappin-reserva] OnStreamingWorldLoaded do MappinSystem hook @ {MAPPINSYSTEM_MUNDO_CARREGADO_VM:#012x} (tabela de journal de mod)"),
            "[mappin-reserva] FALHA ao hookar o OnStreamingWorldLoaded do MappinSystem"
        );
    }
}

// Parte 7d-2a (2026-09-12): mappins de mod. Quando a busca não acha um hash registrado, o elemento
// cozido é montado e entra no fim do `cookedData` do recurso — só dentro da folga reservada na carga do mundo (7d-1), pra não realocar ponteiro já
// entregue — e a busca devolve o elemento dentro do array; dali em diante a própria busca do motor acha o hash. Elemento de mappin: hash `@0`, posição
// `@0x04`, volume (Handle) `@0x10`; de POI: hash `@0`, `entityID` `@0x08` nulo, posição `@0x10`.
// Parte 7e-3c (2026-09-12): o mappin de mod pode trazer o Handle de um `gamemappinsOutlineMappinVolume` (1 strong da tabela). Ao empurrar, a cópia do
// elemento soma 1 no strong (o array do motor solta ao descarregar o recurso); ao retirar, a cópia do array é descontada e a da tabela é solta pela
// sequência do motor, que destrói o objeto pelo allocator da classe (ele vem do `CreateInstance` real).
pub(crate) const MAPPIN_ELEMENTO_VOLUME_OFF: usize = 0x10;

pub(crate) struct MappinDeMod {
    pub(crate) hash: u32,
    pub(crate) posicao: [f32; 3],
    pub(crate) poi: bool,
    pub(crate) volume: [u64; 2],
}

pub(crate) static MAPPINS_DE_MOD: std::sync::Mutex<Vec<MappinDeMod>> = std::sync::Mutex::new(Vec::new());

/// `(ponteiro dos elementos, capacidade, tamanho)` do `cookedData` do recurso; `None` se ilegível.
pub(crate) unsafe fn cookeddata_de(res: *mut c_void) -> Option<(*mut u8, u32, u32)> {
    if res.is_null() || !crate::memoria::is_readable(res as *const c_void, 0x50) {
        return None;
    }
    let arr = (res as *const u8).add(0x40);
    Some(((arr as *const u64).read_unaligned() as *mut u8, (arr.add(0x08) as *const u32).read_unaligned(), (arr.add(0x0C) as *const u32).read_unaligned()))
}

/// Empurra no fim do `cookedData` o mappin de mod desse hash, se ele estiver registrado e couber na folga; devolve o elemento no array ou nulo.
pub(crate) unsafe fn mappin_de_mod_empurrar(b: &BuscaCozida, res: *mut c_void, hash: u32) -> *const u8 {
    let poi = std::ptr::eq(b, &BUSCA_POI);
    let Ok(t) = MAPPINS_DE_MOD.try_lock() else { return std::ptr::null() };
    let Some(m) = t.iter().find(|m| m.hash == hash && m.poi == poi) else { return std::ptr::null() };
    let Some((ptr, cap, tam)) = cookeddata_de(res) else { return std::ptr::null() };
    if ptr.is_null() || tam >= cap || !crate::memoria::is_readable(ptr.add(tam as usize * 0x20) as *const c_void, 0x20) {
        crate::veredito_dev!("[mappin-mod] sem folga no cookedData ({tam}/{cap}) pro hash {hash:#x}");
        return std::ptr::null();
    }
    let e = ptr.add(tam as usize * 0x20);
    std::ptr::write_bytes(e, 0, 0x20);
    (e as *mut u32).write_unaligned(hash);
    let p = e.add(b.posicao_off) as *mut f32;
    for (k, v) in m.posicao.iter().enumerate() {
        p.add(k).write_unaligned(*v);
    }
    if !poi && m.volume[0] != 0 {
        handle_strong_soma(&m.volume, 1);
        (e.add(MAPPIN_ELEMENTO_VOLUME_OFF) as *mut u64).write_unaligned(m.volume[0]);
        (e.add(MAPPIN_ELEMENTO_VOLUME_OFF + 8) as *mut u64).write_unaligned(m.volume[1]);
    }
    ((res as *mut u8).add(0x4c) as *mut u32).write_unaligned(tam + 1);
    MAPPIN_EMPURRADOS.fetch_add(1, Ordering::Relaxed);
    crate::veredito_dev!(
        "[mappin-mod] hash {hash:#x} empurrado no cookedData {} no índice {tam} (capacidade {cap}), posição {:?}, volume {}",
        if poi { "de POI" } else { "de mappin" },
        m.posicao,
        m.volume[0] != 0
    );
    e
}

// ===== Fase de quest, parte c (2026-09-11): carga de fase de quest, só observando =====
// RE offline (prova de 2026-09-11, fase de quest parte b2): `0x1029e8c48(loader, ResourcePath, &Handle<questQuestPhaseResource>)` é a rota
// específica de fase. Os 4 chamadores chegam nela depois de GetType/IsA, em thread
// de job, antes do spinlock `loader+0x18` e do registro no mapa por path. O original não atribui x0 e o chamador reaproveita esse x0: o
// hook devolve o x0 do original. Corpo: leituras diretas (tipo pelo vptr, sem RTTI) e nenhum lock do motor. Critério ao vivo: path
// 0x84642c97f5c71116 com vptr certo e 29 nós; controle negativo: outra fase com contagem diferente.
pub(crate) const QUESTPHASE_CARGA_VM: u64 = 0x1_029e_8c48;

pub(crate) static ORIG_QUESTPHASE_CARGA: AtomicU64 = AtomicU64::new(0);

pub(crate) type QuestPhaseCargaFn = unsafe extern "C" fn(*mut c_void, u64, *mut c_void) -> u64;

pub(crate) unsafe extern "C" fn questphase_carga_replacement(loader: *mut c_void, path: u64, fase: *mut c_void) -> u64 {
    let k = QUESTPHASE_CARGAS.fetch_add(1, Ordering::Relaxed);
    crate::mundo::questphase_patches_na_carga(path, fase);
    if QUESTPHASE_OBSERVAR.load(Ordering::Relaxed) && (k < 48 || path == QUESTPHASE_RAIZ_PATH) && !fase.is_null() {
        let obj = (fase as *const u64).read_unaligned();
        let vptr_ok = obj != 0 && (obj as *const u64).read_unaligned() == crate::rebase(crate::jogo::mundo::FASE_DE_MISSAO_VPTR_VM) as u64;
        let grafo = if vptr_ok { ((obj + 0x40) as *const u64).read_unaligned() } else { 0 };
        let nos = if grafo != 0 { ((grafo + 0x3c) as *const u32).read_unaligned() } else { u32::MAX };
        crate::veredito_dev!(
            "[a35-carga] #{k} path={path:#018x} obj={obj:#x} vptr_ok={vptr_ok} grafo={grafo:#x} nos={nos}{}",
            if path == QUESTPHASE_RAIZ_PATH { " <- raiz" } else { "" }
        );
    }
    let f: QuestPhaseCargaFn = std::mem::transmute(original_publicado(&ORIG_QUESTPHASE_CARGA));
    f(loader, path, fase)
}

/// Instala o hook da rota de carga de fase: no `on_load` só com `~/.bwms-hook-questphase-carga` (registra as cargas, 35c-1); com `aplicar` (tabela de
/// patches lida, 35c-2d-3), instala mesmo sem o marcador. Um só ponto de instalação, então o hook nunca entra duas vezes.
pub(crate) unsafe fn install_questphase_carga_probe(aplicar: bool) {
    let observar = bwms_hook_enabled_global("questphase-carga");
    if observar {
        QUESTPHASE_OBSERVAR.store(true, Ordering::Relaxed);
    }
    if !(observar || aplicar) {
        return;
    }
    install_hook_probe!(
        QUESTPHASE_CARGA_VM, questphase_carga_replacement, ORIG_QUESTPHASE_CARGA,
        &format!("[a35-carga] rota de carga de fase observada @ {QUESTPHASE_CARGA_VM:#012x}"),
        "[a35-carga] FALHA ao hookar a rota de carga de fase"
    );
}

// ===== `OpenWorldTracker`: `PointOfInterestMappin::SetPhase` =====
// Endereço pela RE offline (DATABASE, seção `PointOfInterestMappin`): `vt+0x1d0` da vtable `0x107a9e860`, prólogo sem instrução relativa ao PC.
// ABI: x0 = mappin, w1 = `gamedataMappinPhase`, w0 = bool. O hook chama a original primeiro e só observa; na fase `CompletedPhase` (0) guarda o hash
// de journal do PoI (`GetJournalQuestPathHash`, `+0x8c`) e o tick chama `BwmsOpenWorldTrackerAplicar` (`bwms-openworldtracker.reds`), que leva a
// entrada a Inativa e de volta a Ativa sem notificação. Prova: boot 230
// (prova de 2026-09-13 do ciclo do tracker).
// Liga com `openworldtracker = true` no `red4ext/bwms.toml`; desligado por padrão porque o estado da entrada de journal vai pro save. No dev,
// `~/.bwms-hook-openworldtracker` instala o hook só observando e `~/.bwms-hook-openworldtracker-aplicar` deixa o tick mudar o journal.
pub(crate) const POI_SETPHASE_VM: u64 = 0x1_0314_0998;

pub(crate) static ORIG_POI_SETPHASE: AtomicU64 = AtomicU64::new(0);

pub(crate) type PoiSetPhaseFn = unsafe extern "C" fn(*mut c_void, u32) -> u32;

/// Chave do `bwms.toml` que liga o tracker.
pub(crate) const OPENWORLDTRACKER_CHAVE: &str = "openworldtracker";

/// Global de reds que aplica o tracker numa entrada (`bwms-openworldtracker.reds`).
pub(crate) const OPENWORLDTRACKER_APLICAR_REDS: &str = "BwmsOpenWorldTrackerAplicar";

static POI_SETPHASE_CNT: AtomicU64 = AtomicU64::new(0);

/// Hashes de journal dos PoIs que chegaram em `CompletedPhase`, à espera do tick (até 256).
static OWT_CONCLUIDOS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// `openworldtracker = true` no `bwms.toml`, lido no `on_load`.
static OWT_LIGADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// O tracker só age quando o PoI chega na fase `CompletedPhase` e tem entrada de journal.
fn poi_concluido(fase: u32, hash_quest: u32) -> bool {
    fase == 0 && hash_quest != 0
}

unsafe extern "C" fn poi_setphase_replacement(this: *mut c_void, fase: u32) -> u32 {
    let f: PoiSetPhaseFn = std::mem::transmute(original_publicado(&ORIG_POI_SETPHASE));
    let ret = f(this, fase);
    let n = POI_SETPHASE_CNT.fetch_add(1, Ordering::Relaxed);
    if crate::memoria::is_readable(this as *const c_void, 0xb8) {
        let base = this as *const u8;
        let quest = (base.add(0x8c) as *const u32).read_unaligned();
        #[cfg(feature = "devtools")] crate::selftest::c142_setphase_visto(this as u64, fase, quest); // anotação do teste 142, só no dev
        // Só com log: a classe do dado custa mais uma checagem de memória por chamada.
        if cfg!(feature = "devlog") && n < 32 {
            let journal = (base.add(0x88) as *const u32).read_unaligned();
            let dado = (base.add(0xb0) as *const u64).read_unaligned() as *const c_void;
            let vt_dado = if !dado.is_null() && crate::memoria::is_readable(dado, 8) {
                crate::un_rebase((dado as *const *const c_void).read_unaligned())
            } else {
                0
            };
            crate::veredito_dev!(
                "[owt] SetPhase(poi={this:p}, fase={fase}) -> {} journal={journal:#x} quest={quest:#x} vt_dado={vt_dado:#x}",
                ret & 0xff
            );
        }
        if poi_concluido(fase, quest) {
            if let Ok(mut v) = OWT_CONCLUIDOS.try_lock() {
                if v.len() < 256 {
                    v.push(quest);
                }
            }
        }
    }
    ret
}

/// Instala o hook do `OpenWorldTracker` no `on_load`: com `openworldtracker = true` no `bwms.toml` ou, no dev, com `~/.bwms-hook-openworldtracker`.
pub(crate) unsafe fn install_openworldtracker() {
    let ligado = crate::configuracao::do_mod::ler_bool(OPENWORLDTRACKER_CHAVE) == Some(true);
    OWT_LIGADO.store(ligado, Ordering::Relaxed);
    if !(ligado || bwms_hook_enabled_global("openworldtracker")) {
        return;
    }
    install_hook_probe!(
        POI_SETPHASE_VM, poi_setphase_replacement, ORIG_POI_SETPHASE,
        &format!("[owt] PointOfInterestMappin::SetPhase hook @ {POI_SETPHASE_VM:#012x}"),
        "[owt] FALHA ao hookar PointOfInterestMappin::SetPhase"
    );
}

/// Esvazia a fila de PoIs concluídos na thread do jogo e chama o global de reds `BwmsOpenWorldTrackerAplicar(game, hash)` com a `GameInstance` do
/// player (dentro de uma chamada crua, `GetGameInstance()` devolve instância morta).
pub(crate) unsafe fn aplicar_openworld_concluidos(reg: &Registry, player: *mut c_void) {
    match OWT_CONCLUIDOS.try_lock() {
        Ok(v) if !v.is_empty() => {}
        _ => return,
    }
    // Só em gameplay real (o critério do heartbeat): o player do menu também passa pelo tick, e a fila espera até lá. O byte de fase do
    // `GameSessionDesc` (`+0x84 == 5`) não serve de filtro: em gameplay ele não vale 5 (boot 229).
    if !crate::inicio::boot::gameplay_real() {
        return;
    }
    let lote: Vec<u32> = match OWT_CONCLUIDOS.try_lock() {
        Ok(mut v) if !v.is_empty() => std::mem::take(&mut *v),
        _ => return,
    };
    if !(OWT_LIGADO.load(Ordering::Relaxed) || marcador_individual("openworldtracker-aplicar")) {
        crate::veredito_dev!(
            "[owt] {} PoI(s) concluído(s), journal não alterado (sem `openworldtracker = true` no bwms.toml nem ~/.bwms-hook-openworldtracker-aplicar): {lote:x?}",
            lote.len()
        );
        return;
    }
    let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
    let f = crate::register::get_function(reg, OPENWORLDTRACKER_APLICAR_REDS);
    let (Some(gi), true) = (gi, rtti::sane(f)) else {
        crate::veredito_dev!("[owt] {} PoI(s) concluído(s) sem aplicar: GameInstance ou função de reds indisponível", lote.len());
        return;
    };
    // A função de reds devolve `antes*100 + meio*10 + depois` (estados do journal) ou um código negativo; sem o `ret_type` real o executor corrompe a
    // chamada de função que devolve valor (nota do `ret_type_of`).
    let rf = rtti::ResolvedFn { func: f, ret_type: rtti::ret_type_of(f), is_static: true };
    for h in lote {
        let r = rtti::call_func(&rf, std::ptr::null_mut(), &[rtti::Arg::Raw(gi), rtti::Arg::I32(h)]);
        let codigo = r.map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        crate::veredito_dev!("[owt] PoI concluído quest={h:#x} -> {OPENWORLDTRACKER_APLICAR_REDS} estados (antes, meio, depois) {codigo:?}");
        #[cfg(feature = "devtools")] // anotação do teste 142, só no dev
        {
            if let Some(c) = codigo {
                crate::selftest::c142_tracker_aplicado(h, c);
            }
        }
    }
}

#[cfg(test)]
mod teste_openworldtracker {
    use super::*;

    #[test]
    fn so_fase_concluida_com_journal() {
        assert!(poi_concluido(0, 0x1234));
        assert!(!poi_concluido(0, 0));
        assert!(!poi_concluido(2, 0x1234));
        assert!(!poi_concluido(3, 0x1234));
    }

    /// O tick chama o global de reds pelo nome com `(GameInstance, Uint32)` e lê um `Int32`: arquivo ausente, nome ou forma diferente deixa o
    /// tracker sem efeito e sem erro. É `public func` com corpo (script, não native), então não pede registro no dylib. A âncora é a pasta
    /// `blackwall-mods-dev/` mais próxima, como a `pasta_da_referencia` do `overlay`: achada a pasta, o `.reds` tem de estar nela, e apagar ou
    /// renomear o arquivo falha aqui em vez de cair na árvore de cima. Sem a pasta em nenhum ancestral o crate está fora do projeto: o gate [2d/4]
    /// do `dist/export-github.sh` roda esta suíte em `$OUT/cp77-console`, que não leva `blackwall-mods-dev/`.
    #[test]
    fn global_de_reds_bate_com_a_chamada_do_tick() {
        let Some(dev) = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .map(|d| d.join("blackwall-mods-dev"))
            .find(|p| p.is_dir())
        else {
            return;
        };
        let reds = dev.join("bwms-openworldtracker.reds");
        assert!(reds.is_file(), "{reds:?} ausente: o tick chama `{OPENWORLDTRACKER_APLICAR_REDS}`, declarado nesse arquivo");
        let texto = std::fs::read_to_string(&reds).expect("bwms-openworldtracker.reds legível");
        let assinatura = format!("public func {OPENWORLDTRACKER_APLICAR_REDS}(game: GameInstance, hash: Uint32) -> Int32 {{");
        assert_eq!(texto.matches(&assinatura).count(), 1, "{reds:?} sem `{assinatura}`");
    }
}
