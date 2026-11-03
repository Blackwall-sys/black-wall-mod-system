//! As natives da TweakDB (base de dados do jogo): registros por índice, descendentes, nomes, pai original, remover registro e flats.

#![allow(dead_code)]

use crate::tempo::tramp_frame_delta;
use crate::tempo::tramp_frame_dilatacao;
use crate::tipos_do_motor::register_type_instantiable_with_parent;
use crate::mods_do_usuario::tramp_require;
use crate::mods_do_usuario::tramp_version;
use crate::mods_do_usuario::PLUGUE_TWEAKS;
use crate::aparencia::*;
use std::ffi::c_void;
use crate::rtti::{self, Registry};
use crate::register::*;

// ===== Leitura dinâmica de flat (`GetFlat -> Variant`) —
// `BwmsGetFlatFloat(path: TweakDBID) -> Float`, leitura DINÂMICA de um flat TweakDB por tipo,
// redscript-callable. NÃO é literalmente `-> Variant` (isso exigiria um proto donor com esse
// tipo de retorno — nenhuma native vanilla real declara `-> Variant`, confirmado por grep no
// `redscript-src/` inteiro — e forjar o descritor de retorno à mão é RE nova não tentada) —
// mas entrega a MESMA capacidade prática (ler um flat de tipo conhecido dinamicamente por
// TweakDBID, sem precisar saber o offset/registro na hora de compilar o mod), usando `Cos`
// como donor de retorno Float (mesmo padrão já usado por `BlackwallPing`/etc). Composição de
// peças TODAS já provadas nesta sessão: `get_flat_value_by_id` (leitura do flat) + `GetTypeName`
// @vtable+0xE8 (confirma o tipo antes de interpretar os bytes, evita ler um Int32 como Float por
// engano) + `GetDataPtr`@vtable+0xF0 (endereço do valor real).
pub(crate) unsafe extern "C" fn tramp_get_flat_float(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let mut result = 0.0f32;
    let mut ok = false;
    if let Some(t) = crate::tweakdb::singleton() {
        if let Some(fv) = crate::tweakdb::get_flat_value_by_id(t as *const u8, id) {
            let vt = (fv as *const u64).read_unaligned();
            if crate::memoria::is_readable(vt as *const c_void, 0xF8) {
                let get_type_name = ((vt as *const u8).add(0xE8) as *const u64).read_unaligned();
                let get_data_ptr = ((vt as *const u8).add(0xF0) as *const u64).read_unaligned();
                if rtti::sane(get_type_name as *mut c_void) && rtti::sane(get_data_ptr as *mut c_void) {
                    let mut name_out: u64 = 0;
                    let f_gtn: extern "C" fn(*mut c_void, *mut u64) -> *mut u64 = std::mem::transmute(get_type_name);
                    let ret = f_gtn(fv as *mut c_void, &mut name_out as *mut u64);
                    if ret as u64 == crate::cname::cname("Float") {
                        let f_gdp: extern "C" fn(*mut c_void) -> *mut c_void = std::mem::transmute(get_data_ptr);
                        let data_ptr = f_gdp(fv as *mut c_void);
                        if crate::memoria::is_readable(data_ptr, 4) {
                            result = (data_ptr as *const f32).read_unaligned();
                            ok = true;
                        }
                    } else {
                        crate::log_dev!("[getflatfloat] id={id:#x}: tipo real='{}', não é Float — recusado", crate::cname::resolve_cname(ret as u64));
                    }
                }
            }
        }
    }
    crate::log_dev!("[getflatfloat] id={id:#x} -> {result} (ok={ok})");
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut f32, result);
    }
}

/// `GetRecordCount(type) -> Int32` — `BwmsGetRecordCount(className: String) ->
/// Int32`, reusa `get_records_of_class` (já provado ao vivo — 1823 records de
/// `gamedataWeaponItem_Record`) só contando em vez de logar cada um.
pub(crate) unsafe extern "C" fn tramp_get_record_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let class_name = strings.first().cloned().flatten().unwrap_or_default();
    let mut count: u32 = 0;
    if let (Some(t), Some(reg)) = (crate::tweakdb::singleton(), crate::rtti::Registry::obtain()) {
        if let Some(records) = crate::tweakdb::get_records_of_class(t, &reg, &class_name, 5_000_000) {
            count = records.len() as u32;
        }
    }
    crate::log_dev!("[getrecordcount] '{class_name}' -> {count}");
    write_uint_ret(out, count as u64, 4);
}

/// `GetRecordByIndex(type, index) -> handle` —`BwmsGetRecordByIndex(className:
/// String, index: Int32) -> handle:gamedataTweakDBRecord`. Mesma composição de `BwmsGetRecordCount`
/// (`get_records_of_class`, já provado) + `write_handle_ret` (padrão 100% provado por
/// `GetCallbackSystem`/`StartBatch` — {ptr,0} explícito, handle "raw" sem refcount nosso, evita o
/// SIGBUS documentado de deixar `[+0x08]` com lixo da stack). Zero RE nova.
pub(crate) unsafe extern "C" fn tramp_get_record_by_index(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let class_name = strings.first().cloned().flatten().unwrap_or_default();
    // `*v as i32` (não `as i64`): o slot lido é sempre 8 bytes (`read8`), mas Int32 só ocupa os
    // 4 baixos — `as i32` TRUNCA pros 32 bits baixos (regra de cast do Rust), `as i64` manteria
    // lixo dos 32 bits altos do slot (achado ao vivo: 2ª chamada na mesma função devolveu
    // `0x7000000000 | valor_certo` em vez do valor puro — mesmo padrão já usado em `tramp_utf8strleft`).
    let index = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let mut instance: *mut c_void = std::ptr::null_mut();
    if index >= 0 {
        if let (Some(t), Some(reg)) = (crate::tweakdb::singleton(), crate::rtti::Registry::obtain()) {
            if let Some(records) = crate::tweakdb::get_records_of_class(t, &reg, &class_name, 5_000_000) {
                if let Some((_, ptr)) = records.get(index as usize) {
                    instance = *ptr;
                }
            }
        }
    }
    crate::log_dev!("[getrecbyindex] '{class_name}'[{index}] -> {instance:p}");
    write_handle_ret(out, instance as u64);
}

/// `GetRecords<R>() -> array<ref<R>>` — 2026-08-10 (cont.188), a peça final que
/// faltava do item (GetFlat/GetRecordCount/GetRecordByIndex já fechados, cont.163/169). Composição
/// de `get_records_of_class` (já provado) + o MESMO layout de elemento 16 bytes (`{instance,
/// refCount}`) já usado por `dynarray_push_handle16`/`AddComponent` — a novidade real aqui é o
/// RETORNO ser um `array<T>` inteiro, nunca tentado antes neste projeto (toda native anterior
/// devolve escalar/handle único). Cabe no MESMO sret de 16 bytes (`out`) que `write_handle_ret`
/// já usa — um DynArray header É 16 bytes (`entries:ptr@0, cap:u32@8, size:u32@12`), a MESMA forma
/// já lida (não só escrita) em `get_flat_array_u64_by_id`/etc. Buffer alocado via `rtti::pool_alloc`
/// (mesma via de `dynarray_push_ptr`/`CreateScriptedBitfield`, já provada). `limit` protege contra
/// alocar um array gigante sem querer (classes com milhares de records).
pub(crate) unsafe extern "C" fn tramp_records_da_classe(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let class_name = strings.first().cloned().flatten().unwrap_or_default();
    // ACHADO (cont.188, 1ª tentativa deu ArraySize=0): o `limit` de `get_records_of_class` é um
    // corte de NÓS VISITADOS na tabela inteira (193354 nós), não um teto de RESULTADOS — passar o
    // `out_cap` do redscript direto ali faz escanear só os primeiros N nós (quase sempre 0 matches
    // da classe pedida, que estão espalhados pelo hash). Fix: sempre escaneia a tabela INTEIRA
    // (5_000_000, mesmo valor já usado por `tramp_get_record_count`/`tramp_get_record_by_index`) e
    // só DEPOIS corta a saída pro `out_cap` pedido pelo redscript.
    let out_cap = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0).max(0) as usize;
    let mut n: usize = 0;
    let mut entries_ptr: *mut c_void = std::ptr::null_mut();
    // 2ª TENTATIVA (cont.189): a 1ª (pool_alloc puro, sem trailer) crashou reproduzivelmente no
    // teardown do array LOCAL (ver proofs/..-CRASH-NAO-FECHADO.log). Hipótese: falta o "trailer de
    // allocator-vft" que `mkarr`/`set_flat_array_u64` já usam pra arrays TweakDB-flat (mas esses
    // nunca são expostos como variável LOCAL sujeita a release de escopo). Fix testado aqui:
    // pega o trailer EMPRESTADO de um DynArray<Handle<T>> REAL e vivo — `Entity+0xA0`
    // (ComponentsStorage.components do player, já confirmado real/gerenciado pelo motor,
    // cont.186/187) — mesmo stride de 16 bytes/elemento do nosso array de handles.
    let alloc_vft: u64 = {
        let player = crate::current_player();
        if rtti::sane(player) {
            let arr = (player as *mut u8).add(0xA0) as *const c_void;
            if crate::memoria::is_readable(arr, 0x10) {
                let d_entries = rd_u64(arr, 0x00);
                let d_cap = core::ptr::read_unaligned((arr as *const u8).add(0x08) as *const u32);
                if d_cap > 0 && d_entries != 0 {
                    let tr = (d_entries + d_cap as u64 * 16 + 7) & !7;
                    if crate::memoria::is_readable(tr as *const c_void, 8) { rd_u64(tr as *const c_void, 0) } else { 0 }
                } else {
                    0
                }
            } else {
                0
            }
        } else {
            0
        }
    };
    // Sem alocador emprestado (menu principal, V ainda não existe), array vazio: o motor solta o array pela palavra depois dos
    // `cap` elementos, e com 0 ali o teardown do escopo cai (SIGSEGV 2/2, bwms-leitura-de-records.reds). Com cap 0 não solta nada.
    if out_cap > 0 && alloc_vft != 0 {
        if let (Some(t), Some(reg)) = (crate::tweakdb::singleton(), crate::rtti::Registry::obtain()) {
            if let Some(records) = crate::tweakdb::get_records_of_class(t, &reg, &class_name, 5_000_000) {
                n = records.len().min(out_cap);
                if n > 0 {
                    // buffer = n slots de 16B (handle) + 1 slot de 8B (trailer vft), alinhado —
                    // mesma forma de `set_flat_array_u64_by_id` (elems+trailer), stride diferente.
                    let total = n * 16 + 8;
                    let buf = rtti::pool_alloc(total, 8) as *mut u8;
                    if !buf.is_null() {
                        for (i, (_, ptr)) in records.iter().take(n).enumerate() {
                            let slot = buf.add(i * 16);
                            core::ptr::write_unaligned(slot as *mut u64, *ptr as u64);
                            core::ptr::write_unaligned(slot.add(8) as *mut u64, 0u64); // refCount=null, "raw"
                        }
                        core::ptr::write_unaligned(buf.add(n * 16) as *mut u64, alloc_vft);
                        entries_ptr = buf as *mut c_void;
                    } else {
                        n = 0;
                    }
                }
            }
        }
    }
    crate::log_dev!(
        "[getrecordsarr] '{class_name}' out_cap={out_cap} -> {n} elementos (entries={entries_ptr:p} alloc_vft={alloc_vft:#x})"
    );
    // DynArray header de 16 bytes: entries(8) + capacity(u32) + size(u32). Cap==size (buffer
    // exato, sem slack) — array só-leitura do lado redscript, não precisa de espaço de grow.
    core::ptr::write_unaligned(out as *mut u64, entries_ptr as u64);
    core::ptr::write_unaligned((out as *mut u8).add(8) as *mut u32, n as u32);
    core::ptr::write_unaligned((out as *mut u8).add(12) as *mut u32, n as u32);
}

pub(crate) unsafe fn registrar_leitura_de_flats_e_records(reg: &Registry, proto: *mut c_void) {
    // proto de RETORNO Float/Int32 (4 bytes escalares — mesmo cluster usado por `BlackwallPing`
    // e por `BwmsParseInt32`/etc, que JÁ prova que o byte-width importa mais que o tipo exato
    // clonado pro retorno): reusa Cos/Sin/AbsF/SqrtF/LogF — só a vtable é clonada, os PARAMS vêm
    // de `type_names` abaixo.
    let mut float_proto = std::ptr::null_mut();
    for n in ["Cos", "Sin", "AbsF", "SqrtF", "LogF"] {
        let p = get_function(reg, n);
        if rtti::sane(p) {
            float_proto = p;
            break;
        }
    }
    if !rtti::sane(float_proto) {
        crate::log_dev!("[reg] register_all: GetFlat* — proto Float não resolveu, abortado");
        return;
    }
    let ok1 = register_argful_by_types(reg, float_proto, &["TweakDBID"], "BwmsGetFlatFloat", "BwmsGetFlatFloat", tramp_get_flat_float);
    crate::log_dev!("[reg] register_all: GetFlat (BwmsGetFlatFloat)={ok1}");
    let ok2 = register_argful_by_types(reg, float_proto, &["String"], "BwmsGetRecordCount", "BwmsGetRecordCount", tramp_get_record_count);
    crate::log_dev!("[reg] register_all: GetRecordCount (BwmsGetRecordCount)={ok2}");
    let ok3 = register_argful_by_types(reg, float_proto, &["String", "Int32"], "BwmsGetRecordByIndex", "BwmsGetRecordByIndex", tramp_get_record_by_index);
    crate::log_dev!("[reg] register_all: GetRecordByIndex (BwmsGetRecordByIndex)={ok3}");
    // cont.188: `GetRecords(className, limit) -> array<ref<IScriptable>>` — 1ª native deste
    // projeto a devolver um ARRAY inteiro (não escalar/handle único). Mesmo donor float_proto
    // (shape-only, o retorno real vem da declaração .reds + do que o handler escreve em `out`).
    let ok4 = register_argful_by_types(reg, float_proto, &["String", "Int32"], "BwmsGetRecordsArray", "BwmsGetRecordsArray", tramp_records_da_classe);
    crate::log_dev!("[reg] register_all: GetRecords (BwmsGetRecordsArray)={ok4}");
    let _ = proto; // proto (donor String) não usado aqui — cada família resolve seu próprio donor
}

// Genealogia e nome dos records criados por mod, pelo registro do BWMS (`tweakdb::registro`). Descendentes por contagem e
// índice: o script não precisa de array montado pela native.
pub(crate) unsafe fn id_do_parametro(frame: *mut c_void) -> (u64, rtti::ArgsLidos) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    (args.first().map_or(0, |(v, _)| *v), args)
}

/// `BwmsTweakIsOriginalRecord(id) -> Bool`: o record existe e não foi criado por mod.
pub(crate) unsafe extern "C" fn tramp_tweak_is_original(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (id, _) = id_do_parametro(frame);
    let existe = crate::tweakdb::singleton().is_some_and(|t| !crate::tweakdb::get_record_instance(t, bwms_primitives::tweak_db_id_chave(id)).is_null());
    write_bool_ret(out, existe && crate::tweakdb::registro::original(id));
}

/// `BwmsTweakGetOriginalParent(id) -> TweakDBID`: o primeiro ancestral que não é de mod; vazio pra record sem pai (original ou de `$type`).
pub(crate) unsafe extern "C" fn tramp_tweak_original_parent(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (id, _) = id_do_parametro(frame);
    write_uint_ret(out, crate::tweakdb::registro::pai_original(id).unwrap_or(0), 8);
}

/// `BwmsTweakGetDescendantCount(id) -> Int32` e `BwmsTweakGetDescendant(id, índice) -> TweakDBID`: os records de mod clonados de `id`, em
/// qualquer geração.
pub(crate) unsafe extern "C" fn tramp_tweak_descendant_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (id, _) = id_do_parametro(frame);
    write_i32_ret(out, crate::tweakdb::registro::descendentes(id).len() as i32);
}

pub(crate) unsafe extern "C" fn tramp_tweak_descendant(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (id, args) = id_do_parametro(frame);
    let indice = args.get(1).map_or(-1, |(v, _)| *v as u32 as i32);
    let d = crate::tweakdb::registro::descendentes(id);
    write_uint_ret(out, usize::try_from(indice).ok().and_then(|i| d.get(i)).copied().unwrap_or(0), 8);
}

/// `BwmsTweakGetName(id) -> String`: o nome de um id que um mod escreveu (arquivo de tweak ou `BwmsTweakRegisterName`); vazio se não se sabe.
pub(crate) unsafe extern "C" fn tramp_tweak_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (id, _) = id_do_parametro(frame);
    write_string_ret(out, &crate::tweakdb::registro::nome(id).unwrap_or_default());
}

/// `BwmsTweakRemoveRecord(id) -> Bool`: tira do TweakDB um record criado por mod; recusa record do jogo.
pub(crate) unsafe extern "C" fn tramp_tweak_remove_record(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (id, _) = id_do_parametro(frame);
    let r = crate::tweakdb::remover::remover_record(id);
    if let Err(e) = &r {
        crate::log_dev!("[tweakdb] RemoveRecord({id:#x}): {e}");
    }
    write_bool_ret(out, r.is_ok());
}

/// `BwmsTweakRegisterName(nome: String) -> TweakDBID`: guarda o nome e devolve o id dele.
pub(crate) unsafe extern "C" fn tramp_tweak_register_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let nome = strings.first().cloned().flatten().unwrap_or_default();
    let id = if nome.is_empty() { 0 } else { crate::tweakdb::registro::nomear(&nome) };
    write_uint_ret(out, id, 8);
}


/// `ScriptableTweak` — classe abstrata nativa que mods estendem; BWMS instancia cada subclasse
/// concreta e chama `OnApply()` uma vez, no menu antes do load do save (ou em gameplay, se o load começou antes; `cp77_tick`). Semelhante a ScriptableService mas sem lifecycle.
pub unsafe fn register_scriptabletweak(reg: &Registry) -> String {
    let forged = register_type_min(reg, "ScriptableTweak");
    format!("[reg] ScriptableTweak: forjada={}", !forged.is_null())
}

/// `Reload() -> Void` da fachada de tweaks — gatilho MANUAL (não
/// FSEventStream automático) que reprocessa os `.yaml` já carregados. Vai por `recarregar_tweaks`:
/// desfaz o que os arquivos gravaram, reaplica por `apply_mod_tweaks()` (o mesmo pipeline que já
/// roda automático em `Session/Start`) e pede o redespacho do `OnApply` — chamável a qualquer momento
/// (ex. depois de editar um `.yaml` de mod ativo em disco), sem precisar rebootar o jogo.
pub(crate) unsafe extern "C" fn tramp_recarregar_tweaks(_c: *mut c_void, _f: *mut c_void, _out: *mut c_void, _rt: i64) {
    crate::log_dev!("[reg] >>> Reload dos tweaks chamado — recarga dos tweaks de mod sem empilhar");
    crate::mods_do_usuario::pipeline::recarregar_tweaks();
}

pub unsafe fn registrar_fachada_de_tweaks(reg: &Registry) -> String {
    register_facade_methods(
        reg,
        "TweakXL",
        &[
            ("Version", tramp_version::<PLUGUE_TWEAKS>),
            ("Require", tramp_require::<PLUGUE_TWEAKS>),
            ("Reload", tramp_recarregar_tweaks),
        ],
    )
}

/// Conversão do valor de `SetFlat` (2026-08-11): quando o valor de um `SetFlat`
/// é um `CName` cujo texto começa com "LocKey#",
/// ele vira AUTOMATICAMENTE o flat-type `LocKey` (`gamedataLocKeyWrapper`, 8
/// bytes: só `primaryKey: u64`), em vez de ser armazenado como
/// CName cru. Algoritmo: extrai o sufixo após o prefixo; tenta
/// parsear como inteiro; se não-numérico, `primaryKey = FNV1a64(sufixo)`. Doador real confirmado
/// no schema deste projeto (`tweakdb-base.tsv`): `Items.AdvancedKiroshiOpticsPiercingCommon.
/// displayName` é um flat `LocKey` genuíno e não-nulo (`primaryKey=0x16d2d`) — usado como donor
/// pro `create_flat_value`/`set_flat_nonscalar` (mecanismo já provado).
pub(crate) const LOCKEY_DONOR_FIELD: &str = "Items.AdvancedKiroshiOpticsPiercingCommon.displayName";

/// Conversão do valor de `SetFlat` (2026-08-11, 2º incremento), parte
/// `ResRefToken` — quando o Variant é um `ResRef` redscript real, o tipo de entrada tem CNAME
/// `redResourceReferenceScriptToken` (confirmado ao vivo — mesmo nome já achado pra marshalling
/// de PARÂMETRO em `rtti.rs::read_params_inner`). A conversão é uma REINTERPRETAÇÃO PURA de tipo
/// (`Variant(type, aVariant.GetDataPtr())`,
/// MESMOS bytes) — sem transformação de dado, só troca o flat-type de `ResRefToken` pro
/// flat-type `ResRef` do motor (`"raRef:CResource"`).
/// Donor real confirmado no schema deste projeto (`tweakdb-base.tsv`): `UIIcon.clothing_player_
/// legs_item_l1_pants_06_basic_02__Male_.atlasResourcePath` (não-nulo, `0x...9862eb62`).
pub(crate) const RESREF_DONOR_FIELD: &str =
    "UIIcon.clothing_player_legs_item_l1_pants_06_basic_02__Male_.atlasResourcePath";

/// O valor do `Variant` do `SetFlat`, copiado no visitante do parâmetro, antes do destrutor (o buffer do array e o texto da `String` morrem
/// com ele): o nome do tipo, os 8 primeiros bytes (escalar inline), os elementos de um array de elemento de 8 bytes e o texto de uma `String`.
#[derive(Default)]
struct ValorDoSetFlat {
    tipo: String,
    bruto: u64,
    lista: Option<Vec<u64>>,
    texto: Option<String>,
}

unsafe fn valor_do_variant(variante: *const u8) -> Option<ValorDoSetFlat> {
    let (tipo_rtti, valor) = rtti::variante_partes(variante)?;
    let tipo = crate::cname::resolve_cname(rtti::type_name_getname(tipo_rtti));
    let mut v = ValorDoSetFlat { bruto: (valor as *const u64).read_unaligned(), ..Default::default() };
    if let Some(elemento) = tipo.strip_prefix("array:") {
        let (buffer, n, cap) = crate::jogo::listas::dynarray_no_campo(valor as *const u8, 0);
        let legivel = n == 0 || crate::memoria::is_readable(buffer as *const c_void, n * 8);
        if crate::tweakdb::escritor::largura_do_tipo(elemento) == Some(8) && n <= cap && legivel {
            v.lista = Some((0..n).map(|i| (buffer.add(i * 8) as *const u64).read_unaligned()).collect());
        }
    } else if tipo == "String" {
        v.texto = Some(crate::jogo::cstring::texto(valor as *const u8));
    }
    v.tipo = tipo;
    Some(v)
}

/// `SetFlat` e `CreateRecord` por tipo, `(chave, certos, falhos)`, desde o último [`tirar_contagem`]: o dispatch dos `OnApply` fecha
/// cada `ScriptableTweak` com essa linha de veredito.
#[cfg(feature = "devlog")]
static CONTAGEM: std::sync::Mutex<Vec<(String, u32, u32)>> = std::sync::Mutex::new(Vec::new());

fn contar(operacao: &str, tipo: &str, ok: bool) {
    #[cfg(feature = "devlog")]
    if let Ok(mut c) = CONTAGEM.lock() {
        let chave = format!("{operacao} {tipo}");
        let i = match c.iter().position(|(k, _, _)| *k == chave) {
            Some(i) => i,
            None => {
                c.push((chave, 0, 0));
                c.len() - 1
            }
        };
        if ok {
            c[i].1 += 1;
        } else {
            c[i].2 += 1;
        }
    }
    #[cfg(not(feature = "devlog"))]
    let _ = (operacao, tipo, ok);
}

/// A contagem desde a última chamada, em texto (`SetFlat array:TweakDBID 1892 certo(s)/0 falho(s); …`), e zera.
#[cfg(feature = "devlog")]
pub(crate) fn tirar_contagem() -> (String, u32) {
    let c = CONTAGEM.lock().map(|mut c| std::mem::take(&mut *c)).unwrap_or_default();
    let falhos = c.iter().map(|(_, _, f)| *f).sum();
    let texto = if c.is_empty() {
        "nenhuma chamada".to_string()
    } else {
        c.iter().map(|(k, o, f)| format!("{k} {o} certo(s)/{f} falho(s)")).collect::<Vec<_>>().join("; ")
    };
    (texto, falhos)
}

pub(crate) unsafe extern "C" fn tramp_tdbm_setflat(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut valor: Option<ValorDoSetFlat> = None;
    let args = rtti::read_params_consuming_visit(current_native_func(), frame, &mut |i, _, inst| {
        if i == 1 {
            valor = valor_do_variant(inst as *const u8);
        }
    });
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let ok = match &valor {
        Some(ValorDoSetFlat { tipo, bruto: raw, lista, texto }) => {
            let (type_name, raw) = (tipo.as_str(), *raw);
            if type_name == "CName" {
                let text = crate::cname::resolve_cname(raw);
                if text.starts_with(bwms_primitives::LOCKEY_PREFIXO) {
                    let primary_key = bwms_primitives::lockey_chave(&text);
                    let t = crate::tweakdb::singleton();
                    let converted = match t {
                        Some(tp) => crate::tweakdb::set_flat_nonscalar_by_id(
                            tp as *mut u8,
                            id,
                            bwms_primitives::tweak_db_id(LOCKEY_DONOR_FIELD),
                            &primary_key.to_le_bytes(),
                            8,
                        )
                        .is_some(),
                        None => false,
                    };
                    crate::log_dev!(
                        "[tdbmanager] SetFlat(id={id:#x}): CName '{text}' -> LocKey primaryKey={primary_key:#x} (donor='{LOCKEY_DONOR_FIELD}') converted={converted}"
                    );
                    converted
                } else {
                    crate::tweakdb::escritor::gravar_escalar(id, raw)
                }
            } else if type_name == "redResourceReferenceScriptToken" {
                // ResRefToken: reinterpretação pura de tipo, MESMOS bytes (o `raw` já é o hash
                // de resource path — mesma leitura inline já usada pra CName/TweakDBID).
                let t = crate::tweakdb::singleton();
                let converted = match t {
                    Some(tp) => crate::tweakdb::set_flat_nonscalar_by_id(
                        tp as *mut u8,
                        id,
                        bwms_primitives::tweak_db_id(RESREF_DONOR_FIELD),
                        &raw.to_le_bytes(),
                        8,
                    )
                    .is_some(),
                    None => false,
                };
                crate::log_dev!(
                    "[tdbmanager] SetFlat(id={id:#x}): ResRefToken raw={raw:#x} -> ResRef (donor='{RESREF_DONOR_FIELD}') converted={converted}"
                );
                converted
            } else {
                match (type_name, lista, texto) {
                    // Escalar pelo escritor único: largura tirada do tipo do valor (8 B no `TweakDBID`), gravada num FlatValue próprio.
                    ("Int32" | "Uint32" | "Float" | "Bool" | "TweakDBID", _, _) => crate::tweakdb::escritor::gravar_escalar(id, raw),
                    (_, Some(elementos), _) => crate::tweakdb::escritor::gravar_lista(id, type_name, elementos),
                    (_, _, Some(texto)) => crate::tweakdb::escritor::gravar_string(id, texto),
                    (other, _, _) => {
                        crate::log_dev!("[tdbmanager] SetFlat(id={id:#x}): tipo '{other}' ainda não suportado (array só de elemento de 8 bytes)");
                        false
                    }
                }
            }
        }
        None => {
            crate::log_dev!("[tdbmanager] SetFlat(id={id:#x}): param 1 não é Variant (inesperado)");
            false
        }
    };
    // Flat que o record não tem (o `.parentSlot`/`.dependencySlots` de mod de roupa): nasce com o tipo do valor. A busca no índice só
    // roda quando a gravação falhou, fora do caminho dos milhares de `SetFlat` em flat que existe.
    let ok = match &valor {
        Some(v)
            if !ok
                && crate::tweakdb::escritor::doador_do_tipo(&v.tipo).is_some()
                && crate::tweakdb::singleton().is_some_and(|t| crate::tweakdb::find_flat_entry_by_id(t as *const u8, id).is_none()) =>
        {
            let criou = crate::tweakdb::escritor::criar_flat(id, &v.tipo, v.bruto, v.lista.as_deref());
            crate::log_dev!("[tdbmanager] SetFlat(id={id:#x}): flat novo {} -> {criou}", v.tipo);
            criou
        }
        _ => ok,
    };
    contar("SetFlat", valor.as_ref().map_or("?", |v| v.tipo.as_str()), ok);
    crate::log_dev!("[tdbmanager] SetFlat(id={id:#x}) -> {ok}");
    write_bool_ret(out, ok);
}

pub(crate) unsafe extern "C" fn tramp_tdbm_createrecord(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let type_cname_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let class_name = crate::cname::resolve_cname(type_cname_hash);
    let t = crate::tweakdb::singleton();
    let ok = match (crate::rtti::Registry::obtain(), t) {
        _ if class_name.is_empty() => {
            crate::log_dev!("[tdbmanager] CreateRecord(id={id:#x}): CName do tipo ({type_cname_hash:#x}) não resolveu nome");
            false
        }
        // Record que já existe: recusa.
        (_, Some(t)) if !crate::tweakdb::get_record_instance(t, id).is_null() => false,
        (Some(reg), Some(t)) => match crate::tweakdb::criar_record_do_tipo(&reg, t, id, &class_name) {
            Some((classe, modelo, padroes, sem)) => {
                crate::log_dev!("[tdbmanager] CreateRecord(id={id:#x}, '{classe}') pela estrutura de {modelo:#x}, {padroes} flat(s) no padrão, {sem} sem padrão");
                true
            }
            None => {
                crate::log_dev!("[tdbmanager] CreateRecord(id={id:#x}, '{class_name}'): sem record vivo da classe ou clone falhou");
                false
            }
        },
        _ => false,
    };
    contar("CreateRecord", &class_name, ok);
    if ok {
        crate::tweakdb::registro::criado(id);
    }
    write_bool_ret(out, ok);
}

pub(crate) unsafe extern "C" fn tramp_tdbm_updaterecord(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let ok = crate::tweakdb::update_record_by_id(id);
    write_bool_ret(out, ok);
}

/// `CloneRecord(id: TweakDBID, base: TweakDBID) -> Bool`.
/// `tweak_db_id_derive` (`bwms-primitives`) é o que deixa a herança de flats funcionar sem o NOME do
/// record (mods reais só têm o TweakDBID numérico, nunca a string).
pub(crate) unsafe extern "C" fn tramp_tdbm_clonerecord(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let base = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let ok = match crate::rtti::Registry::obtain() {
        Some(reg) => crate::tweakdb::clone_record_by_id(&reg, id, base),
        None => false,
    };
    if ok {
        crate::tweakdb::registro::clonado(id, base);
    }
    write_bool_ret(out, ok);
}

/// `RegisterEnum(id: TweakDBID)` — o `.reds` declara `-> Bool` (mesma convenção usada pra `ScriptBatch.
/// RegisterEnum`, que RETORNA bool) — devolve sempre `true` (registro em memória nunca falha).
pub(crate) unsafe extern "C" fn tramp_tdbm_registerenum(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let ok = crate::tweakdb::register_enum_id(id);
    crate::log_dev!("[tdbmanager] RegisterEnum(id={id:#x}) -> {ok}");
    write_bool_ret(out, ok);
}

/// `RegisterName(name: CName) -> Bool` — ensina o nome ORIGINAL de
/// um TweakDBID introduzido por mod pro registro reverso (hash->string) de debug. Literal
/// `n"..."`/`t"..."` de `.reds` já entra no CNamePool do motor no load do bundle; o handler resolve
/// o CName e grava o nome no registro reverso (`registro::nomear`). `false` se o CName não resolve.
pub(crate) unsafe extern "C" fn tramp_tdbm_registername(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(hash);
    let ok = !name.is_empty();
    if ok {
        crate::tweakdb::registro::nomear(&name);
    }
    crate::log_dev!("[tdbmanager] RegisterName(hash={hash:#x}) -> '{name}' ok={ok}");
    write_bool_ret(out, ok);
}

pub(crate) unsafe fn register_registro_de_records(reg: &Registry, proto: *mut c_void) {
    let d = register_argful_by_types(reg, proto, &[], "BwmsFrameDeltaTime", "BwmsFrameDeltaTime", tramp_frame_delta as NativeHandler);
    let t = register_argful_by_types(reg, proto, &[], "BwmsFrameTimeDilation", "BwmsFrameTimeDilation", tramp_frame_dilatacao as NativeHandler);
    crate::veredito_dev!("[reg] leitura do frame: deltaTime {d}, timeDilation {t}");
    let natives: [(&[&str], &str, NativeHandler); 7] = [
        (&["TweakDBID"], "BwmsTweakIsOriginalRecord", tramp_tweak_is_original),
        (&["TweakDBID"], "BwmsTweakGetOriginalParent", tramp_tweak_original_parent),
        (&["TweakDBID"], "BwmsTweakGetDescendantCount", tramp_tweak_descendant_count),
        (&["TweakDBID", "Int32"], "BwmsTweakGetDescendant", tramp_tweak_descendant),
        (&["TweakDBID"], "BwmsTweakGetName", tramp_tweak_name),
        (&["String"], "BwmsTweakRegisterName", tramp_tweak_register_name),
        (&["TweakDBID"], "BwmsTweakRemoveRecord", tramp_tweak_remove_record),
    ];
    let registradas = natives.iter().filter(|(tipos, nome, h)| register_argful_by_types(reg, proto, tipos, nome, nome, *h)).count();
    crate::veredito_dev!("[reg] registro de records de mod: {registradas} de {}", natives.len());
}

/// `StartBatch() -> ref<TweakDBBatch>` — mesmo padrão 100% provado de `tramp_cbs_register_callback`
/// (`RegisterCallback` devolvendo o handler recém-criado, ver nota lá): `rtti::new_object` constrói
/// a instância pelo pool do RED (free casa com o do engine) + `rtti::make_handle` embrulha num
/// `ref<T>` REAL via a rotina de ctor do próprio motor (não uma reimplementação de refcount nossa).
pub(crate) unsafe extern "C" fn tramp_tdbm_startbatch(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut instance: *mut c_void = std::ptr::null_mut();
    if let Some(reg) = crate::rtti::Registry::obtain() {
        instance = rtti::new_object(&reg, "TweakDBBatch");
    }
    crate::log_dev!("[tdbmanager] StartBatch() -> {instance:p}");
    rtti::make_handle(out, instance);
}

/// `TweakDBBatch.Commit()` — no-op: cada método do batch já aplica IMEDIATAMENTE (sem staging):
/// as mesmas mutações, uma por vez — mesmo efeito final.
pub(crate) unsafe extern "C" fn tramp_tdbm_commit(_c: *mut c_void, _f: *mut c_void, _out: *mut c_void, _rt: i64) {
    crate::log_dev!("[tdbmanager] TweakDBBatch.Commit() -> (no-op, mutações já aplicadas ao chamar cada método)");
}

/// `ReinheritFlat` (2026-08-10): pra quando
/// um mod sobrescreve 1 flat específico de um record que é ele mesmo um clone/herdeiro, apontando
/// esse flat pra um record-fonte DIFERENTE do usado no `CloneRecord` original. Método
/// PRÓPRIO do BWMS, chamável de redscript. `flatId` (o flat do clone,
/// já existente via CloneRecord/InheritProps) tem seu offset REPONTADO pro do flat
/// `sourceId + "." + appendix` — ver `tweakdb::reinherit_flat_by_id`.
pub(crate) unsafe extern "C" fn tramp_tdbm_reinheritflat(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let flat_id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let source_id = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let appendix = strings.get(2).cloned().flatten().unwrap_or_default();
    let ok = match crate::tweakdb::singleton() {
        Some(t) => crate::tweakdb::reinherit_flat_by_id(t, flat_id, source_id, &appendix),
        None => {
            crate::log_dev!("[tdbmanager] ReinheritFlat: singleton TweakDB indisponível");
            false
        }
    };
    write_bool_ret(out, ok);
}

/// Flat extra (2026-08-10). `AddFlat(flatId, donorTypeFlat, value)` —
/// `flatId`/`donorTypeFlat` chegam PRONTOS do compilador (`TDBID.Create("Items.X.prop")`
/// resolvido em compile-time pelo `scc`, mesmo padrão de `ReinheritFlat`) — evita precisar ler
/// String+Variant juntos (o projeto só tem extração combinada de UM dos dois por chamada).
/// `donorTypeFlat` = TweakDBID de um flat JÁ EXISTENTE do MESMO tipo que `value` (empresta a
/// vtable nativa pra `create_flat_value_by_id`, mesma técnica de `mkflat`/`SetFlat` não-escalar).
pub(crate) unsafe extern "C" fn tramp_tdbm_addflat(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, variants) = rtti::read_params_consuming_with_variants(func, frame);
    let flat_id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let donor_type_flat_id = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let ok = match variants.get(2).and_then(|v| v.as_ref()) {
        Some(buf) => {
            let type_hash = rtti::variant_type_cname(buf);
            let type_name = crate::cname::resolve_cname(type_hash);
            let raw = rtti::variant_inline_u64(buf);
            match type_name.as_str() {
                "Int32" | "Uint32" | "Float" | "Bool" | "TweakDBID" | "CName" => match crate::tweakdb::singleton() {
                    // TweakDBID (crc + tamanho + tdbOffset) e CName (hash) têm 8 bytes; os outros 4 tipos, 4.
                    Some(t) => {
                        let largura = if matches!(type_name.as_str(), "TweakDBID" | "CName") { 8 } else { 4 };
                        crate::tweakdb::add_extra_flat_bytes_by_id(t, flat_id, donor_type_flat_id, &raw.to_le_bytes()[..largura])
                    }
                    None => false,
                },
                other => {
                    crate::log_dev!("[tdbmanager] AddFlat: tipo '{other}' não-escalar — ainda não suportado");
                    false
                }
            }
        }
        None => {
            crate::log_dev!("[tdbmanager] AddFlat: param 3 não é Variant (inesperado)");
            false
        }
    };
    crate::log_dev!("[tdbmanager] AddFlat(flatId={flat_id:#x}) -> {ok}");
    write_bool_ret(out, ok);
}

pub unsafe fn register_tweakdbmanager(reg: &Registry) -> String {
    let _forged = register_type_min(reg, "TweakDBManager");
    register_facade_methods(
        reg,
        "TweakDBManager",
        &[
            ("SetFlat", tramp_tdbm_setflat),
            ("CreateRecord", tramp_tdbm_createrecord),
            ("CloneRecord", tramp_tdbm_clonerecord),
            ("UpdateRecord", tramp_tdbm_updaterecord),
            ("RegisterEnum", tramp_tdbm_registerenum),
            ("RegisterName", tramp_tdbm_registername),
            ("StartBatch", tramp_tdbm_startbatch),
        ],
    )
}

/// `TweakDBBatch`. Mesma receita 100% provada de `register_callbacksystemhandler`
/// (`CallbackSystemHandler`, classe instanciável sem `extends` explícito = parent
/// `IScriptable` implícito, mesmo donor `gameuiInGameMenuGameController`, mesmo fix de alocador
/// embutido já aplicado dentro de `register_type_instantiable_with_parent`). Reusa os MESMOS
/// handlers de `TweakDBManager` pros 6 métodos delegados (nenhum lê `self`/estado de instância —
/// cada chamada é stateless e aplica na hora, então o mesmo trampolim serve pro slot estático E
/// pro de instância) + `Commit()` próprio (no-op).
pub unsafe fn register_tweakdbbatch(reg: &Registry) -> String {
    let iscriptable = reg.class_by_name("IScriptable");
    if !rtti::sane(iscriptable) {
        return "[reg] register_tweakdbbatch: 'IScriptable' não resolveu".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "TweakDBBatch", "gameuiInGameMenuGameController", iscriptable);
    if forged.is_null() {
        return "[reg] register_tweakdbbatch: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_tweakdbbatch: classe=OK mas sem protótipo (AddGodMode não resolveu)".into(),
    };
    let m1 = register_method(reg, "TweakDBBatch", proto, "SetFlat", "SetFlat", tramp_tdbm_setflat, false);
    let m2 = register_method(reg, "TweakDBBatch", proto, "CreateRecord", "CreateRecord", tramp_tdbm_createrecord, false);
    let m3 = register_method(reg, "TweakDBBatch", proto, "CloneRecord", "CloneRecord", tramp_tdbm_clonerecord, false);
    let m4 = register_method(reg, "TweakDBBatch", proto, "UpdateRecord", "UpdateRecord", tramp_tdbm_updaterecord, false);
    let m5 = register_method(reg, "TweakDBBatch", proto, "RegisterEnum", "RegisterEnum", tramp_tdbm_registerenum, false);
    let m6 = register_method(reg, "TweakDBBatch", proto, "RegisterName", "RegisterName", tramp_tdbm_registername, false);
    let m7 = register_method(reg, "TweakDBBatch", proto, "Commit", "Commit", tramp_tdbm_commit, false);
    let m8 = register_method(reg, "TweakDBBatch", proto, "ReinheritFlat", "ReinheritFlat", tramp_tdbm_reinheritflat, false);
    let m9 = register_method(reg, "TweakDBBatch", proto, "AddFlat", "AddFlat", tramp_tdbm_addflat, false);
    format!("[reg] TweakDBBatch.SetFlat={m1} CreateRecord={m2} CloneRecord={m3} UpdateRecord={m4} RegisterEnum={m5} RegisterName={m6} Commit={m7} ReinheritFlat={m8} AddFlat={m9}")
}

/// Registro do TweakDB pelo id (`Handle`, strong +1), como o `0x10201eaec` sem a checagem de classe.
pub(crate) unsafe fn a9_registro(id: u64) -> [u64; 2] {
    let tdb: unsafe extern "C" fn() -> u64 = std::mem::transmute(crate::rebase(crate::jogo::itens::TWEAKDB_VM));
    chamar_sret_handle(crate::jogo::itens::TWEAKDB_REGISTRO_VM, tdb(), id)
}

