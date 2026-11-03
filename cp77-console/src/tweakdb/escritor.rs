//! Escritor único do TweakDB: toda gravação de valor de flat passa por aqui — o YAML e o `.tweak` dos mods, o `SetFlat` do script e a
//! C-ABI dos plugins. Três regras que antes cada caminho fazia (ou deixava de fazer) do seu jeito:
//!
//! 1. **Nunca gravar no valor que já existe.** O `flatDataBuffer` deduplica valores: o mesmo `FlatValue` serve a todo flat que tem aquele
//!    valor, então gravar nele muda todos os records que o dividem. O valor novo vai num `FlatValue` próprio (reaproveitado se idêntico,
//!    pelo pool do [`super::create_flat_value_by_id`]) e só a entrada do flat pedido é re-apontada.
//! 2. **A largura sai do tipo do próprio valor** (`GetTypeName`, vt+0xE8), nunca de quem chama: `TweakDBID` e `CName` têm 8 bytes, e o
//!    `SetFlat` gravava os dois como `u32`, deixando a metade de cima do valor antigo.
//! 3. **Toda mudança fica no diário** (o offset original de cada flat), o que permite desfazer: recarregar ou desligar um mod sem sobra.
//! 4. **A mudança chega nos clones que herdaram o flat** e ainda usam o valor antigo (o registro de herança do `super::registro`); clone que
//!    sobrescreveu o flat fica com o valor dele.
//!
//! Depois de re-apontar, o byte de versão global (`TweakDB+0x160`) sobe 1 (no lote de arquivo, 1 vez no fim): cada campo de record re-resolve o valor na próxima leitura
//! (mecanismo do `UpdateRecord`, 2026-07-17), então o efeito continua imediato como era com a gravação no lugar.

use super::{create_flat_value_by_id, entry_tdb_offset, find_flat_entry_by_id, set_flat_offset_by_id, singleton, VERSION_BYTE_OFF};
use core::ffi::c_void;
use std::collections::HashMap;
use std::sync::Mutex;

/// flat → (offset que ele tinha antes da 1ª gravação da sessão, gravado por arquivo de mod).
static DIARIO: Mutex<Option<HashMap<u64, (u32, bool)>>> = Mutex::new(None);

/// tipo → offset do valor de um flat vivo do tipo ([`offset_vivo_do_tipo`]).
static ACHADOS_POR_TIPO: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

/// O motor recarregou o TweakDB: o diário e os offsets por tipo apontam para o buffer velho. Esquece sem desfazer.
pub(crate) fn esquecer() {
    *DIARIO.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *ACHADOS_POR_TIPO.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

thread_local! {
    /// Ligado enquanto um arquivo de mod (YAML, `.tweak`) está sendo aplicado nesta thread: as gravações dele entram no diário como
    /// de arquivo, e só essas voltam na recarga — o que um script mudou em jogo fica.
    static DE_ARQUIVO: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Alguma gravação do lote de arquivo pediu a subida da versão: ela sai uma vez, no fim do `com_origem_arquivo`.
    static VERSAO_PENDENTE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Ligado enquanto um `OnApply` roda nesta thread ([`em_lote_de_versao`]); conta as subidas que viraram uma só.
    static EM_LOTE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SUBIDAS_DO_LOTE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Roda `f` com as gravações marcadas como vindas de arquivo de mod.
pub(crate) fn com_origem_arquivo<R>(f: impl FnOnce() -> R) -> R {
    DE_ARQUIVO.with(|d| d.set(true));
    let r = f();
    DE_ARQUIVO.with(|d| d.set(false));
    if VERSAO_PENDENTE.with(|p| p.replace(false)) {
        unsafe {
            if let Some(t) = singleton() {
                subir_versao(t);
            }
        }
    }
    r
}

/// Roda `f` (um `OnApply`) com a versão do TweakDB subindo uma vez só, no fim, se alguma gravação pediu; devolve também quantas subidas
/// viraram essa uma. O `PatchOriginaltems` do Equipment-EX grava 3766 flats num `OnApply`: subindo a cada um, um campo de record cujo byte em
/// cache volta a bater com o global (o acessor `0x10165c104` só compara u8 com u8) devolve o valor velho. No TweakXL de referência as
/// gravações do `OnApply` também só valem no record pelo `UpdateRecord`/`Commit`, não a cada `SetFlat`.
pub(crate) fn em_lote_de_versao<R>(f: impl FnOnce() -> R) -> (R, u32) {
    EM_LOTE.with(|l| l.set(true));
    SUBIDAS_DO_LOTE.with(|s| s.set(0));
    let r = f();
    EM_LOTE.with(|l| l.set(false));
    let subidas = SUBIDAS_DO_LOTE.with(|s| s.replace(0));
    if !DE_ARQUIVO.with(|d| d.get()) && VERSAO_PENDENTE.with(|p| p.replace(false)) {
        unsafe {
            if let Some(t) = singleton() {
                subir_versao(t);
            }
        }
    }
    (r, subidas)
}

/// Tipo do valor de um flat (`Int32`, `Float`, `TweakDBID`, `CName`, `array:TweakDBID`…), pelo `GetTypeName` da vtable nativa do
/// `FlatValue`.
pub(crate) unsafe fn tipo_do_valor(t: *const u8, id: u64) -> Option<String> {
    tipo_do_flat_value(super::get_flat_value_by_id(t, id)?)
}

/// O tipo de um `FlatValue` pelo `GetTypeName` da vtable dele.
unsafe fn tipo_do_flat_value(fv: *const u8) -> Option<String> {
    let vt = (fv as *const u64).read_unaligned();
    if !crate::memoria::is_readable(vt as *const c_void, 0xF0) {
        return None;
    }
    let get_type_name = ((vt as *const u8).add(0xE8) as *const u64).read_unaligned();
    if !crate::rtti::sane(get_type_name as *mut c_void) {
        return None;
    }
    let mut nome: u64 = 0;
    let f: extern "C" fn(*mut c_void, *mut u64) -> *mut u64 = std::mem::transmute(get_type_name);
    let ret = f(fv as *mut c_void, &mut nome as *mut u64) as u64;
    Some(TIPOS_DE_FLAT.iter().find(|n| crate::cname::cname(n) == ret).map_or_else(|| crate::cname::resolve_cname(ret), |n| n.to_string()))
}

/// Os tipos de flat do TweakDB (`ERTDBFlatType` da referência, escalar e `array:`): o hash que o `GetTypeName` devolve é o FNV-1a do nome, e
/// estes saem dele sem o pool de nomes do motor; outro `raRef:` cai no pool.
const TIPOS_DE_FLAT: [&str; 26] = [
    "Int32", "Float", "Bool", "String", "CName", "gamedataLocKeyWrapper", "raRef:CResource", "TweakDBID", "Quaternion", "EulerAngles", "Vector3",
    "Vector2", "Color", "array:Int32", "array:Float", "array:Bool", "array:String", "array:CName", "array:gamedataLocKeyWrapper",
    "array:raRef:CResource", "array:TweakDBID", "array:Quaternion", "array:EulerAngles", "array:Vector3", "array:Vector2", "array:Color",
];

/// Bytes do valor de um tipo escalar; `None` pra tipo que não cabe num escalar (array, String, struct).
pub(crate) fn largura_do_tipo(tipo: &str) -> Option<usize> {
    match tipo {
        "Bool" | "Int8" | "Uint8" => Some(1),
        "Int16" | "Uint16" => Some(2),
        "Int32" | "Uint32" | "Float" => Some(4),
        "Int64" | "Uint64" | "Double" | "TweakDBID" | "CName" | "redResourceReferenceScriptToken" => Some(8),
        _ => None,
    }
}

/// Largura de um valor escalar no motor: os tipos de [`largura_do_tipo`] mais `LocKey` e `raRef:*` (u64).
pub(crate) fn largura_do_valor(tipo: &str) -> Option<usize> {
    largura_do_tipo(tipo).or(if tipo == "gamedataLocKeyWrapper" || tipo.starts_with("raRef:") { Some(8) } else { None })
}

fn anotar(id: u64, offset_antes: u32) {
    let de_arquivo = DE_ARQUIVO.with(|d| d.get());
    if let Ok(mut d) = DIARIO.lock() {
        d.get_or_insert_with(HashMap::new).entry(id).or_insert((offset_antes, de_arquivo));
    }
}

/// Aponta o flat `id` pro valor já existente em `novo_off` (herança: o flat de um clone passa a usar o valor do base), com diário e versão.
pub(crate) unsafe fn reapontar(id: u64, novo_off: u32) -> bool {
    let Some(t) = singleton() else { return false };
    let Some(entrada) = find_flat_entry_by_id(t as *const u8, id) else { return false };
    let antes = entry_tdb_offset(entrada.read_unaligned());
    if antes == novo_off {
        return true;
    }
    if !set_flat_offset_by_id(t as *const u8, id, novo_off) {
        return false;
    }
    anotar(id, antes);
    subir_versao(t);
    true
}

/// O acessor do motor só re-resolve um campo quando a versão em cache difere da global (byte, compara u8 com u8): 256 subidas entre duas
/// leituras dão a volta e deixam o valor velho. Por isso o lote de arquivo e o do `OnApply` sobem uma vez só, e a subida é atômica (threads de
/// mod em paralelo).
unsafe fn subir_versao(t: *mut u8) {
    if DE_ARQUIVO.with(|d| d.get()) {
        VERSAO_PENDENTE.with(|p| p.set(true));
        return;
    }
    if EM_LOTE.with(|l| l.get()) {
        VERSAO_PENDENTE.with(|p| p.set(true));
        SUBIDAS_DO_LOTE.with(|s| s.set(s.get().saturating_add(1)));
        return;
    }
    (*(t.add(VERSION_BYTE_OFF) as *const std::sync::atomic::AtomicU8)).fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// Grava `dados` como o valor novo do flat `id`: `FlatValue` próprio com a vtable do tipo copiada de `doador` (um flat do MESMO tipo; o
/// próprio `id` serve quando ele já existe), re-aponta só esta entrada, anota o offset anterior no diário e sobe a versão. Devolve o offset
/// novo.
pub(crate) unsafe fn gravar_valor(id: u64, doador: u64, dados: &[u8], alinhamento: usize) -> Option<u32> {
    let t = singleton()?;
    let entrada = find_flat_entry_by_id(t as *const u8, id)?;
    let antes = entry_tdb_offset(entrada.read_unaligned());
    let novo = create_flat_value_by_id(t, doador, dados, alinhamento)? as u32;
    if !set_flat_offset_by_id(t as *const u8, id, novo) {
        return None;
    }
    anotar(id, antes);
    propagar(t, id, antes, novo, 0);
    subir_versao(t);
    Some(novo)
}

/// Leva o valor novo de `flat` aos flats de clones que herdaram dele e ainda apontam pro valor antigo, e daí aos herdeiros deles.
unsafe fn propagar(t: *mut u8, flat: u64, antes: u32, novo: u32, profundidade: u32) {
    if profundidade > 32 {
        return;
    }
    for h in super::registro::herdeiros(flat) {
        let Some(entrada) = find_flat_entry_by_id(t as *const u8, h) else { continue };
        if entry_tdb_offset(entrada.read_unaligned()) == antes && set_flat_offset_by_id(t as *const u8, h, novo) {
            anotar(h, antes);
            propagar(t, h, antes, novo, profundidade + 1);
        }
    }
}

/// Offset (no buffer de valores) do valor padrão do tipo `tipo`, pelo `defaultValues` do TweakDB (`HashMap<CName, FlatValue*>`): o valor que o
/// motor dá a um flat que nenhum record define.
pub(crate) unsafe fn offset_do_padrao(t: *const u8, tipo: &str) -> Option<u32> {
    let chave = crate::cname::cname(tipo);
    let no = crate::jogo::mapas::nos(t.add(super::buffer::VALORES_PADRAO_OFF))?.into_iter().find(|n| (n.add(0x08) as *const u64).read_unaligned() == chave)?;
    let valor = (no.add(0x10) as *const u64).read_unaligned();
    let base = (t.add(super::FLAT_DATA_BUFFER_OFF) as *const u64).read_unaligned();
    (valor >= base && valor - base <= super::buffer::CAPACIDADE_MAXIMA as u64).then(|| (valor - base) as u32)
}

/// Offset do valor de algum flat vivo do tipo `tipo`, para o tipo que não tem valor padrão no TweakDB (`Quaternion`, `array:Vector2`): a
/// vtable dele é a do tipo, e o flat novo nasce com ela. Varre o índice de flats lendo só a vtable de cada valor e pergunta o nome uma vez
/// por vtable nova; o achado fica guardado por tipo. `None` sem nenhum flat do tipo no jogo (`Color`).
unsafe fn offset_vivo_do_tipo(t: *const u8, tipo: &str) -> Option<u32> {
    let mut achados = ACHADOS_POR_TIPO.lock().unwrap_or_else(|p| p.into_inner());
    let achados = achados.get_or_insert_with(HashMap::new);
    if let Some(off) = achados.get(tipo) {
        return Some(*off);
    }
    let entradas = (t.add(super::FLATS_OFF) as *const u64).read_unaligned() as *const u64;
    let n = (t.add(super::FLATS_SIZE_OFF) as *const u32).read_unaligned() as usize;
    let fdb = (t.add(super::FLAT_DATA_BUFFER_OFF) as *const u64).read_unaligned() as *const u8;
    if entradas.is_null() || fdb.is_null() || n == 0 || n > 10_000_000 || !crate::memoria::is_readable(entradas as *const c_void, n * 8) {
        return None;
    }
    let mut vistas = std::collections::HashSet::new();
    for i in 0..n {
        let off = super::entry_tdb_offset(entradas.add(i).read_unaligned());
        let fv = fdb.add(off as usize);
        if vistas.insert((fv as *const u64).read_unaligned()) {
            if let Some(nome) = tipo_do_flat_value(fv) {
                achados.entry(nome).or_insert(off);
            }
        }
        if let Some(off) = achados.get(tipo) {
            return Some(*off);
        }
    }
    None
}

/// Aponta cada flat de `flats` pro valor padrão do tipo dele (TweakXL `#14`: record de `$type` nasce com os padrões, não com os valores do
/// record-modelo de onde a estrutura foi clonada). `(apontados, sem padrão do tipo)`.
pub(crate) unsafe fn aplicar_padroes(flats: &[u64]) -> (usize, usize) {
    let Some(t) = singleton() else { return (0, flats.len()) };
    let (mut apontados, mut sem) = (0, 0);
    for flat in flats {
        match tipo_do_valor(t as *const u8, *flat).and_then(|tipo| offset_do_padrao(t as *const u8, &tipo)) {
            Some(off) if reapontar(*flat, off) => apontados += 1,
            _ => sem += 1,
        }
    }
    (apontados, sem)
}

/// Grava um valor escalar no flat `id`, com a largura do tipo do próprio valor (os bytes de cima de `bruto` que não cabem no tipo são
/// ignorados). `false` se o flat não existe ou o tipo não é escalar.
pub(crate) unsafe fn gravar_escalar(id: u64, bruto: u64) -> bool {
    let Some(t) = singleton() else { return false };
    let Some(tipo) = tipo_do_valor(t as *const u8, id) else { return false };
    let Some(n) = largura_do_tipo(&tipo) else {
        crate::log_dev!("[tweakdb] gravar_escalar({id:#x}): tipo '{tipo}' não é escalar");
        return false;
    };
    let mut dados = [0u8; 8];
    dados[..n].copy_from_slice(&bruto.to_le_bytes()[..n]);
    gravar_valor(id, id, &dados, 8).is_some()
}

/// Número do texto de um mod: decimal, `0x` hexadecimal ou (pra `Float`) com sufixo `f`.
fn numero(v: &str) -> Option<i128> {
    match v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        Some(h) => i128::from_str_radix(h, 16).ok(),
        None => v.parse::<i128>().ok(),
    }
}

/// Valor escalar de 8 bytes a partir do texto, pelo tipo do flat. `None` quando o texto não serve pro tipo.
pub(super) fn escalar_do_texto(tipo: &str, v: &str) -> Option<u64> {
    // Os formatos explícitos do TweakXL (`n"X"`, `t"X"`, `LocKey(N)`, `ResRef(N)`, `<TDBID:…>`…) numa propriedade de record chegam crus:
    // o tipo só se sabe aqui, no flat vivo.
    let normal = bwms_primitives::tweakxl::normalizar_para(tipo, v);
    let v = normal.as_str();
    let nulo = v.is_empty() || v == "None";
    Some(match tipo {
        "Bool" => match v.to_ascii_lowercase().as_str() {
            "true" | "1" => 1,
            "false" | "0" => 0,
            _ => return None,
        },
        "Int8" | "Int16" | "Int32" | "Int64" | "Uint8" | "Uint16" | "Uint32" | "Uint64" => numero(v)? as u64,
        // Hexadecimal num Float são os bits crus (a convenção do `setflat`); o resto é número.
        "Float" => match v.strip_prefix("0x") {
            Some(h) => u32::from_str_radix(h, 16).ok()? as u64,
            None => v.trim_end_matches(['f', 'F']).parse::<f32>().ok()?.to_bits() as u64,
        },
        "Double" => v.trim_end_matches(['d', 'D']).parse::<f64>().ok()?.to_bits(),
        "TweakDBID" => if nulo { 0 } else { numero(v).map(|n| n as u64).unwrap_or_else(|| super::tweak_db_id(v)) },
        "CName" => if nulo { 0 } else { crate::cname::cname(v) },
        "gamedataLocKeyWrapper" => bwms_primitives::lockey_chave(v),
        t if t.starts_with("raRef:") || t == "redResourceReferenceScriptToken" => {
            // Inteiro puro é o próprio hash (`ResRef(N)` e o implícito `N` da wiki); o resto é caminho.
            if nulo { 0 } else { numero(v).map_or_else(|| bwms_primitives::resource_path_hash(v), |n| n as u64) }
        }
        _ => return None,
    })
}

/// `(bytes, alinhamento)` de um valor do tipo `tipo` na memória do motor no Mac, que é também o passo do elemento num `array:<tipo>`.
/// Medido no binário 2.5 (`cp77-symbols/binarios/Cyberpunk2077-2.5.0-steam.bin`): os criadores de `FlatValue` de array do próprio motor
/// (`0x1038044f0`…`0x103805744`) chamam o `DynArray::Reserve` (`0x100028674`) com `(elemento, alinhamento)` = (1,1), (4,4) ×2, (4,1),
/// (8,8) ×3, (8,4) ×2, (0xc,4) ×2, (0x10,0x10), (0x20,8) e copiam `n << log2(elemento)` bytes — elementos contíguos no passo do tamanho. A
/// atribuição de cada par aos 13 tipos de array de flat sai do layout dos tipos (`Bool` 1; `Int32`/`Float` 4; `Color` = 4×u8; `CName`,
/// `TweakDBID`, `gamedataLocKeyWrapper`, `raRef:*` = u64; `Vector2` 2×f32; `Vector3`/`EulerAngles` 3×f32; `Quaternion` 4×f32 alinhado a
/// 16; `String` = `CString` de 0x20 B, o mesmo que o `jogo::cstring` monta).
pub(crate) fn elemento_do_tipo(tipo: &str) -> Option<(usize, usize)> {
    Some(match tipo {
        "Bool" => (1, 1),
        "Int32" | "Uint32" | "Float" => (4, 4),
        "Color" => (4, 1),
        "CName" | "TweakDBID" | "gamedataLocKeyWrapper" | "Int64" | "Uint64" | "Double" => (8, 8),
        t if t.starts_with("raRef:") => (8, 8),
        "Vector2" => (8, 4),
        "Vector3" | "EulerAngles" => (12, 4),
        "Quaternion" => (16, 16),
        "String" => (0x20, 8),
        _ => return None,
    })
}

fn floats(v: &str, n: usize) -> Option<Vec<u8>> {
    let p: Vec<f32> = v.split(',').map(|x| x.trim().trim_end_matches(['f', 'F']).parse().ok()).collect::<Option<_>>()?;
    (p.len() == n).then(|| p.iter().flat_map(|f| f.to_le_bytes()).collect())
}

/// Bytes de um valor que não é `String` (escalar ou elemento de array) a partir do texto do mod, no layout do motor. Struct vem na forma
/// posicional do pipeline (`x,y,z`; `i,j,k,r`; `red,green,blue,alpha`; `EulerAngles` em `pitch,yaw,roll`, a ordem do YAML e do
/// `tweakdb.bin`, que na memória é `{Roll, Pitch, Yaw}`, `EulerAngles.hpp` do SDK). `None` quando o texto não serve pro tipo.
pub(crate) fn bytes_do_texto(tipo: &str, v: &str) -> Option<Vec<u8>> {
    // Struct parcial do arquivo (`{y:6.0}`): o tipo do flat vivo põe os campos na ordem e os ausentes em 0. Tupla do `.tweak` sem tipo
    // declarado (`( 0.f, -85.f, -360.f )`) vem na ordem de memória: o `EulerAngles` vivo a gira para `pitch,yaw,roll`.
    let posicional = match v.trim_start().as_bytes().first() {
        Some(b'{') => bwms_primitives::tweakxl::struct_em_posicional(tipo, v),
        Some(b'(') => bwms_primitives::tweakxl::tupla_em_posicional(tipo, v),
        _ => None,
    };
    let v = posicional.as_deref().unwrap_or(v);
    match tipo {
        "Vector2" => floats(v, 2),
        "Vector3" => floats(v, 3),
        "Quaternion" => floats(v, 4),
        "EulerAngles" => {
            let b = floats(v, 3)?;
            Some([&b[8..12], &b[0..4], &b[4..8]].concat())
        }
        "Color" => {
            let p: Vec<u8> = v.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
            (p.len() == 4).then_some(p)
        }
        _ => {
            let n = largura_do_valor(tipo)?;
            Some(escalar_do_texto(tipo, v)?.to_le_bytes()[..n].to_vec())
        }
    }
}

/// O elemento `texto` de um `array:<elem>` nos bytes do motor; o `String` é um `CString` montado pelo construtor do jogo.
pub(crate) unsafe fn elemento_em_bytes(elem: &str, texto: &str) -> Option<Vec<u8>> {
    if elem == "String" {
        let mut cs = vec![0u8; 0x20];
        crate::jogo::cstring::nova(cs.as_mut_ptr(), texto);
        return Some(cs);
    }
    bytes_do_texto(elem, texto.trim())
}

/// Dois elementos de `array:<elem>` iguais como o motor compara: `String` pelo texto (o `CString` guarda ponteiro), o resto byte a byte.
pub(crate) unsafe fn elementos_iguais(elem: &str, a: &[u8], b: &[u8]) -> bool {
    if elem == "String" {
        crate::jogo::cstring::texto(a.as_ptr()) == crate::jogo::cstring::texto(b.as_ptr())
    } else {
        a == b
    }
}

/// O tipo do elemento e os elementos (os bytes de cada um, no passo de [`elemento_do_tipo`]) do flat array `id`.
pub(crate) unsafe fn elementos_do_array(t: *const u8, id: u64) -> Option<(String, Vec<Vec<u8>>)> {
    let tipo = tipo_do_valor(t, id)?;
    let elem = tipo.strip_prefix("array:")?.to_string();
    let (tam, _) = elemento_do_tipo(&elem)?;
    let fv = super::get_flat_value_by_id(t, id)?;
    let entries = (fv.add(0x8) as *const u64).read_unaligned();
    let cap = (fv.add(0x10) as *const u32).read_unaligned();
    let size = (fv.add(0x14) as *const u32).read_unaligned() as usize;
    if !crate::register::dynarray_coerente(entries as usize, size as u32, cap) {
        return None;
    }
    if size == 0 {
        return Some((elem, Vec::new()));
    }
    if !crate::memoria::is_readable(entries as *const c_void, size * tam) {
        return None;
    }
    let base = entries as *const u8;
    let itens = (0..size).map(|i| std::slice::from_raw_parts(base.add(i * tam), tam).to_vec()).collect();
    Some((elem, itens))
}

/// Grava `elementos` (bytes de `array:<elem>`) no flat array `id`: `FlatValue` novo com a vtable e o alocador do valor atual do próprio flat.
pub(crate) unsafe fn gravar_elementos(id: u64, elem: &str, elementos: &[Vec<u8>]) -> bool {
    let Some(t) = singleton() else { return false };
    let Some((tam, alinh)) = elemento_do_tipo(elem) else { return false };
    let Some(dv) = super::get_flat_value_by_id(t as *const u8, id) else { return false };
    let Some(carga) = super::carga_de_array_bytes(dv, tam, alinh, &elementos.concat(), elementos.len()) else { return false };
    gravar_valor(id, id, &carga, 8).is_some()
}

/// Grava no flat `id` um valor dado como TEXTO de mod (YAML, `.tweak`), convertido pelo tipo do próprio flat: número, `Bool`, `TweakDBID`
/// (caminho do record), `CName`, `LocKey`, recurso, `String`, struct (`Vector2/3`, `Quaternion`, `EulerAngles`, `Color`) e array de
/// qualquer um deles (lista `[a, b]` de [`bwms_primitives::operacoes::lista_em_texto`]). Todos os formatos de autoria convertem igual. `false`
/// se o flat não existe ou o texto não serve pro tipo.
pub(crate) unsafe fn gravar_texto(id: u64, texto: &str) -> bool {
    let Some(t) = singleton() else { return false };
    let Some(tipo) = tipo_do_valor(t as *const u8, id) else { return false };
    if let Some(elem) = tipo.strip_prefix("array:") {
        // Array só por lista `[a, b]` (o que o YAML e o `.tweak` produzem pra sequência), como o TweakXL: texto solto não vira array de 1.
        let Some(itens) = bwms_primitives::operacoes::texto_em_lista(texto) else {
            crate::log_dev!("[tweakdb] gravar_texto({id:#x}): '{texto}' não é lista pra um {tipo}");
            return false;
        };
        let bytes: Option<Vec<Vec<u8>>> = itens.iter().map(|s| elemento_em_bytes(elem, s)).collect();
        return match bytes {
            Some(b) => gravar_elementos(id, elem, &b),
            None => {
                crate::log_dev!("[tweakdb] gravar_texto({id:#x}): elemento de {texto} que não serve pro tipo '{elem}'");
                false
            }
        };
    }
    // Formato explícito (`n"X"`, `t"X"`, `r"X"`, `l"X"`) antes de tirar aspas: sem a aspa do fim ele deixa de ser reconhecido.
    let normal = bwms_primitives::tweakxl::normalizar_para(&tipo, texto.trim());
    let v = normal.trim_matches('"');
    if tipo == "String" {
        return gravar_string(id, string_de_lockey(texto.trim()).as_deref().unwrap_or(v));
    }
    let Some(mut dados) = bytes_do_texto(&tipo, v) else {
        crate::log_dev!("[tweakdb] gravar_texto({id:#x}): '{v}' não serve pro tipo '{tipo}'");
        return false;
    };
    dados.resize(dados.len().max(8), 0);
    gravar_valor(id, id, &dados, elemento_do_tipo(&tipo).map_or(8, |(_, a)| a)).is_some()
}

/// Texto de uma propriedade String com formato explícito de LocKey (`l"X"`, `LocKey(…)`, `LocKey#…`): o TweakXL grava `LocKey#<chave
/// primária>` (`YamlReader::ConvertValue<CString>`, wiki "Localization keys"), com a secundária no hash. O texto entre aspas no arquivo já
/// chega sem elas e passa como está: diverge do TweakXL só em `"LocKey#Nome"` entre aspas, que lá fica literal.
fn string_de_lockey(v: &str) -> Option<String> {
    let explicito = (v.starts_with("l\"") && v.ends_with('"') && v.len() >= 3) || (v.starts_with("LocKey(") && v.ends_with(')')) || v.starts_with("LocKey#");
    explicito.then(|| format!("LocKey#{}", bwms_primitives::lockey_chave(&bwms_primitives::tweakxl::normalizar_para("gamedataLocKeyWrapper", v))))
}

/// Flat `id` com o tipo `tipo` e o valor `texto` de arquivo (`Op::Criar`): o que já existe grava pelo tipo dele ([`gravar_texto`]); o que não
/// existe nasce no índice com a vtable do valor padrão do tipo (`defaultValues`, o mesmo do `$type`) — o `AssignFlat` do TweakXL num flat
/// inexistente. No diário o "antes" é o valor padrão: a recarga desfaz pra ele, como o changelog do TweakXL (`GetDefault` do tipo).
pub(crate) unsafe fn criar_flat_tipado(id: u64, tipo: &str, texto: &str) -> bool {
    let Some(t) = singleton() else { return false };
    if find_flat_entry_by_id(t as *const u8, id).is_some() {
        return gravar_texto(id, texto);
    }
    // Sem valor padrão do tipo, a vtable de um flat vivo dele; o desfazer volta o flat novo para esse valor do jogo.
    let Some(off_padrao) = offset_do_padrao(t as *const u8, tipo).or_else(|| offset_vivo_do_tipo(t as *const u8, tipo)) else {
        crate::log_dev!("[tweakdb] criar_flat_tipado({id:#x}): tipo '{tipo}' sem valor padrão e sem flat vivo do tipo no TweakDB");
        return false;
    };
    let padrao = ((t.add(super::FLAT_DATA_BUFFER_OFF) as *const u64).read_unaligned() as *const u8).add(off_padrao as usize);
    if criar_flat_com_padrao(t, id, tipo, texto, padrao).is_none() {
        return false;
    }
    anotar(id, off_padrao);
    subir_versao(t);
    true
}

/// O flat novo `id` no TweakDB `t`, com a vtable do `FlatValue` `padrao` (o valor padrão do tipo) e o valor `texto` convertido pelo `tipo`.
/// Devolve o offset do valor.
unsafe fn criar_flat_com_padrao(t: *mut u8, id: u64, tipo: &str, texto: &str, padrao: *const u8) -> Option<u32> {
    let vft = (padrao as *const u64).read_unaligned();
    let dados = if let Some(elem) = tipo.strip_prefix("array:") {
        let (tam, alinh) = elemento_do_tipo(elem)?;
        let itens = bwms_primitives::operacoes::texto_em_lista(texto)?;
        let bytes = itens.iter().map(|s| elemento_em_bytes(elem, s)).collect::<Option<Vec<_>>>()?;
        // O alocador sai do padrão do tipo; vazio sem alocador (`entries` 0), sai do array doador do `SetFlat`.
        let doador = if (padrao.add(0x8) as *const u64).read_unaligned() != 0 {
            Some((padrao, tam))
        } else {
            doador_do_tipo("array:CName").and_then(|d| super::get_flat_value_by_id(t as *const u8, super::tweak_db_id(d))).map(|d| (d as *const u8, 8))
        };
        let (fonte, tam_fonte) = doador?;
        carga_com_alocador_de(fonte, tam_fonte, tam, alinh, &bytes)?.to_vec()
    } else if tipo == "String" {
        let mut cs = vec![0u8; 0x20];
        crate::jogo::cstring::nova(cs.as_mut_ptr(), texto.trim().trim_matches('"'));
        cs
    } else {
        let mut d = bytes_do_texto(tipo, texto.trim().trim_matches('"'))?;
        d.resize(d.len().max(8), 0);
        d
    };
    let alinh = if tipo.starts_with("array:") { 8 } else { elemento_do_tipo(tipo).map_or(8, |(_, a)| a) };
    let off = super::criar_valor_com_vft(t, vft, &dados, alinh)? as u32;
    super::inserir_flat_com_offset(t, id, off).then_some(off)
}

/// A carga de array com os `bytes` (elementos de `tam` bytes), com o alocador do array `fonte` (elementos de `tam_fonte` bytes).
unsafe fn carga_com_alocador_de(fonte: *const u8, tam_fonte: usize, tam: usize, alinh: usize, bytes: &[Vec<u8>]) -> Option<[u8; 16]> {
    let vazia = super::carga_de_array_bytes(fonte, tam_fonte, 8, &[], 0)?;
    let alocador = u64::from_le_bytes(vazia[..8].try_into().ok()?);
    let mut falso = [0u8; 0x18];
    falso[0x8..0x10].copy_from_slice(&alocador.to_le_bytes());
    super::carga_de_array_bytes(falso.as_ptr(), tam, alinh, &bytes.concat(), bytes.len())
}

/// Grava `texto` no flat `String` `id` (o `enumName` de um stat novo, o `localizedName` de um slot): o valor é um `CString` do motor, montado
/// pelo construtor do próprio jogo. `false` se o flat não existe ou não é `String`.
pub(crate) unsafe fn gravar_string(id: u64, texto: &str) -> bool {
    let Some(t) = singleton() else { return false };
    if tipo_do_valor(t as *const u8, id).as_deref() != Some("String") {
        return false;
    }
    let mut cs = [0u8; 0x20];
    crate::jogo::cstring::nova(cs.as_mut_ptr(), texto);
    gravar_valor(id, id, &cs, 8).is_some()
}

/// Grava `elementos` no flat array `id` de elemento de 8 bytes (`array:TweakDBID`, `array:CName`), quando o tipo do flat é `tipo`, o tipo do
/// valor que o script mandou: o `placementSlots` dos itens e o `attachmentSlots` dos personagens que o Equipment-EX estende no `OnApply`.
/// `false` se o flat não existe ou é de outro tipo.
pub(crate) unsafe fn gravar_lista(id: u64, tipo: &str, elementos: &[u64]) -> bool {
    let Some(t) = singleton() else { return false };
    if tipo_do_valor(t as *const u8, id).as_deref() != Some(tipo) {
        return false;
    }
    super::set_flat_array_u64_by_id(t, id, id, elementos).is_some()
}

/// Flat vanilla de cada tipo que o `SetFlat` cria: dá a vtable nativa do `FlatValue` e, no array, a carga com o alocador do motor no fim do
/// buffer (o `carga_de_array` copia o alocador de lá). São os doadores do teste do ArchiveXL `#40` (`selftest.rs`, `A40_DOADOR_*`,
/// flats criados e relidos ao vivo em 2026-09-13); o `array:CName` é o `tags` do mesmo record.
const DOADORES_DO_TIPO: [(&str, &str); 3] = [
    ("TweakDBID", "Items.Preset_Yukimura_Default.quality"),
    ("array:TweakDBID", "Items.GenericHeadClothing.placementSlots"),
    ("array:CName", "Items.GenericHeadClothing.tags"),
];

/// Doador do tipo de valor `tipo` que o `SetFlat` sabe criar; `None` pros outros tipos (o `SetFlat` recusa, como antes).
pub(crate) fn doador_do_tipo(tipo: &str) -> Option<&'static str> {
    DOADORES_DO_TIPO.iter().find(|(t, _)| *t == tipo).map(|(_, d)| *d)
}

/// Cria o flat `id`, que nenhum record define, com o tipo e o valor do `SetFlat`: `bruto` no `TweakDBID`, `lista` no array. É o que o TweakXL faz
/// num `SetFlat` de flat inexistente (`TweakDBManager::AssignFlat`: valor novo no buffer e `InsertOrAssign` no índice, sem olhar o schema do
/// record). O `RegisterOutfitSlots` do Equipment-EX grava assim o `.parentSlot` e o `.dependencySlots` dos slots que ele cria, e o ArchiveXL `#40`
/// os lê pelo índice. `false` sem doador do tipo, com o doador de outro tipo, ou se a inserção falhou.
pub(crate) unsafe fn criar_flat(id: u64, tipo: &str, bruto: u64, lista: Option<&[u64]>) -> bool {
    match singleton() {
        Some(t) => criar_flat_em(t, id, tipo, bruto, lista),
        None => false,
    }
}

/// [`criar_flat`] no TweakDB `t`. O tipo do doador sai do hash do `GetTypeName` (vt+0xE8), sem o pool de nomes. O valor nasce pronto num
/// `FlatValue` com a vtable do doador (no array, o buffer próprio com o alocador do doador, a mesma carga do `set_flat_array_u64_by_id`) e só a
/// entrada nova entra no índice: nenhum outro flat aponta pro valor antigo de ninguém, e o diário não ganha um "antes". Sobe a versão
/// (segurada no lote do `OnApply`).
pub(crate) unsafe fn criar_flat_em(t: *mut u8, id: u64, tipo: &str, bruto: u64, lista: Option<&[u64]>) -> bool {
    let Some(nome_doador) = doador_do_tipo(tipo) else { return false };
    let doador = super::tweak_db_id(nome_doador);
    if super::dado_do_flat(t as *const u8, doador, tipo).is_none() {
        crate::log_dev!("[tweakdb] criar_flat({id:#x}, {tipo}): doador '{nome_doador}' ausente ou de outro tipo");
        return false;
    }
    let carga: Vec<u8> = match (tipo.starts_with("array:"), lista) {
        (false, _) => bruto.to_le_bytes().to_vec(),
        (true, Some(elementos)) => match super::carga_de_array(t as *const u8, doador, elementos) {
            Some(c) => c.to_vec(),
            None => return false,
        },
        (true, None) => return false,
    };
    let criou = super::add_extra_flat_bytes_by_id(t, id, doador, &carga);
    if criou {
        subir_versao(t);
    }
    criou
}

/// Volta o flat `id` ao offset que ele tinha antes da 1ª gravação da sessão. `false` se ele não foi gravado por aqui.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn desfazer(id: u64) -> bool {
    let Some(t) = singleton() else { return false };
    let antes = DIARIO.lock().ok().and_then(|mut d| d.as_mut().and_then(|m| m.remove(&id)));
    let Some((off, _)) = antes else { return false };
    let ok = set_flat_offset_by_id(t as *const u8, id, off);
    subir_versao(t);
    ok
}

/// `(capacidade, em uso, livre)` do `flatDataBuffer` em bytes (`+0x148` início, `+0x150` capacidade u32, `+0x158` fim). Quando o livre
/// acaba, o `create_flat_value_by_id` cresce o buffer (`buffer::crescer`).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn folga_do_buffer() -> Option<(u64, u64, u64)> {
    let t = singleton()? as *const u8;
    let inicio = (t.add(super::FLAT_DATA_BUFFER_OFF) as *const u64).read_unaligned();
    let cap = (t.add(super::FLAT_DATA_CAP_OFF) as *const u32).read_unaligned() as u64;
    let fim = (t.add(super::FLAT_DATA_END_OFF) as *const u64).read_unaligned();
    (inicio != 0 && fim >= inicio && fim - inicio <= cap).then(|| (cap, fim - inicio, cap - (fim - inicio)))
}

/// Offset anotado no diário pro flat `id` (o de antes da 1ª gravação), se houver.
#[cfg(feature = "devtools")]
pub(crate) fn offset_original(id: u64) -> Option<u32> {
    DIARIO.lock().ok().and_then(|d| d.as_ref().and_then(|m| m.get(&id).map(|(o, _)| *o)))
}

/// Volta ao offset original todo flat gravado por arquivo de mod (a recarga desfaz antes de reaplicar: sem `!append` duplicado, sem sobra
/// de linha apagada). Devolve quantos voltaram.
pub(crate) unsafe fn desfazer_arquivos() -> usize {
    let Some(t) = singleton() else { return 0 };
    let voltas: Vec<(u64, u32)> = match DIARIO.lock() {
        Ok(mut d) => {
            let m = d.get_or_insert_with(HashMap::new);
            let ids: Vec<u64> = m.iter().filter(|(_, (_, arq))| *arq).map(|(id, _)| *id).collect();
            ids.into_iter().filter_map(|id| m.remove(&id).map(|(off, _)| (id, off))).collect()
        }
        Err(_) => return 0,
    };
    let n = voltas.iter().filter(|(id, off)| set_flat_offset_by_id(t as *const u8, *id, *off)).count();
    if n > 0 {
        subir_versao(t);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::escalar_do_texto;

    #[test]
    fn lote_de_versao_segura_as_subidas_do_onapply() {
        let mut tdb = vec![0u8; 0x168];
        let t = tdb.as_mut_ptr();
        // Fora do lote, 256 subidas devolvem o byte ao valor de partida: o campo de record com esse byte em cache não re-resolve.
        for _ in 0..256 {
            unsafe { super::subir_versao(t) };
        }
        assert_eq!(tdb[super::VERSION_BYTE_OFF], 0);
        // As 3766 gravações do `PatchOriginaltems` (1883 `placementSlots` + 1883 `garmentOffset`) dentro do lote não mexem no byte; a subida
        // única do fim procura o TweakDB vivo, que fora do jogo não existe.
        let ((), seguradas) = super::em_lote_de_versao(|| {
            for _ in 0..3766 {
                unsafe { super::subir_versao(t) };
            }
        });
        assert_eq!(seguradas, 3766);
        assert_eq!(tdb[super::VERSION_BYTE_OFF], 0);
        // Depois do lote a pendência foi consumida e a subida volta a ser imediata.
        unsafe { super::subir_versao(t) };
        assert_eq!(tdb[super::VERSION_BYTE_OFF], 1);
        assert!(!super::VERSAO_PENDENTE.with(|p| p.get()));
    }

    // TweakDB falso pro `criar_flat_em`: o `GetTypeName` (vt+0xE8) devolve o hash do nome do tipo e o `GetDataPtr` (vt+0xF0) o valor em +0x08,
    // como o `FlatValue` do motor.
    extern "C" fn tipo_tdbid(_fv: *mut core::ffi::c_void, _out: *mut u64) -> *mut u64 {
        crate::cname::cname("TweakDBID") as *mut u64
    }
    extern "C" fn tipo_arr_tdbid(_fv: *mut core::ffi::c_void, _out: *mut u64) -> *mut u64 {
        crate::cname::cname("array:TweakDBID") as *mut u64
    }
    extern "C" fn tipo_arr_cname(_fv: *mut core::ffi::c_void, _out: *mut u64) -> *mut u64 {
        crate::cname::cname("array:CName") as *mut u64
    }
    extern "C" fn dado(fv: *mut core::ffi::c_void) -> *mut core::ffi::c_void {
        unsafe { (fv as *mut u8).add(8) as *mut core::ffi::c_void }
    }

    fn vtable(tipo: extern "C" fn(*mut core::ffi::c_void, *mut u64) -> *mut u64) -> &'static [u64; 0x20] {
        let mut vt = [0u64; 0x20];
        vt[0xE8 / 8] = tipo as usize as u64;
        vt[0xF0 / 8] = dado as *const () as u64;
        Box::leak(Box::new(vt))
    }

    /// Entrada do índice: chave (hash+len) do id e o offset do valor nos bytes 5-7, big-endian.
    fn entrada(id: u64, off: u32) -> u64 {
        bwms_primitives::tweak_db_id_chave(id) | ((off as u64 >> 16) & 0xFF) << 40 | ((off as u64 >> 8) & 0xFF) << 48 | (off as u64 & 0xFF) << 56
    }

    fn alocador() -> u64 {
        crate::jogo::listas::alocador_de_teste()
    }

    /// O `SetFlat` do `RegisterOutfitSlots` (Equipment-EX) em flat que o `AttachmentSlot_Record` não tem: o `.parentSlot` (`TweakDBID`) e o
    /// `.dependencySlots` (`array:TweakDBID`) nascem no índice com a vtable do doador, e o leitor do ArchiveXL `#40` (`get_flat_tdbid` e
    /// `get_flat_array_u64_by_id` pelo `tweak_db_id_derive(slot, ".parentSlot")`, o caminho do `a40_initialize_slots`, do
    /// `terceira_pessoa::slot_base` e do `onattachtpp_compose`) acha o valor que o mod gravou.
    #[test]
    fn setflat_cria_o_flat_que_o_record_nao_tem_e_o_leitor_do_40_acha() {
        use super::super::{get_flat_array_u64_by_id, get_flat_tdbid, tweak_db_id, tweak_db_id_derive, FLATS_OFF, FLAT_DATA_BUFFER_OFF, FLAT_DATA_CAP_OFF, FLAT_DATA_END_OFF};
        let id = tweak_db_id;
        // Elementos dos arrays doadores, com o alocador no fim do buffer (o que o `carga_de_array` copia).
        let elem_tdbid: &'static [u64; 2] = Box::leak(Box::new([id("AttachmentSlots.Head"), alocador()]));
        let elem_cname: &'static [u64; 2] = Box::leak(Box::new([crate::cname::cname("Clothing"), alocador()]));
        // Buffer de valores: o TweakDBID em 0x00, o array:TweakDBID em 0x10 e o array:CName em 0x28; o resto é folga pros valores novos.
        let mut valores = vec![0u64; 0x200];
        let vb = valores.as_mut_ptr();
        unsafe {
            *vb = vtable(tipo_tdbid).as_ptr() as u64;
            *vb.add(1) = id("Quality.Rare");
            *vb.add(2) = vtable(tipo_arr_tdbid).as_ptr() as u64;
            *vb.add(3) = elem_tdbid.as_ptr() as u64;
            *vb.add(4) = 1 | 1 << 32;
            *vb.add(5) = vtable(tipo_arr_cname).as_ptr() as u64;
            *vb.add(6) = elem_cname.as_ptr() as u64;
            *vb.add(7) = 1 | 1 << 32;
        }
        let mut doadores = vec![
            entrada(id("Items.Preset_Yukimura_Default.quality"), 0x00),
            entrada(id("Items.GenericHeadClothing.placementSlots"), 0x10),
            entrada(id("Items.GenericHeadClothing.tags"), 0x28),
        ];
        doadores.sort_by_key(|e| ((*e & 0xFFFF_FFFF) as u32, ((*e >> 32) & 0xFF) as u8));
        let cap = 8usize;
        let mut indice = vec![0u64; cap + 1];
        indice[..3].copy_from_slice(&doadores);
        indice[cap] = alocador();
        let mut tdb = vec![0u8; 0x168];
        let t = tdb.as_mut_ptr();
        unsafe {
            (t.add(FLATS_OFF) as *mut u64).write_unaligned(indice.as_mut_ptr() as u64);
            (t.add(FLATS_OFF + 8) as *mut u32).write_unaligned(cap as u32);
            (t.add(FLATS_OFF + 0x0C) as *mut u32).write_unaligned(3);
            (t.add(FLAT_DATA_BUFFER_OFF) as *mut u64).write_unaligned(vb as u64);
            (t.add(FLAT_DATA_CAP_OFF) as *mut u32).write_unaligned((valores.len() * 8) as u32);
            (t.add(FLAT_DATA_END_OFF) as *mut u64).write_unaligned(vb as u64 + 0x40);
        }

        let head = id("AttachmentSlots.Head");
        let (eyes, chest) = (id("AttachmentSlots.Eyes"), id("AttachmentSlots.Chest"));
        let (slot_head, slot_orelha) = (id("OutfitSlots.Head"), id("OutfitSlots.EarLeft"));
        unsafe {
            // O id chega do script como `slotID + t".parentSlot"`; o leitor do #40 deriva do slot.
            assert!(super::criar_flat_em(t, id("OutfitSlots.Head.parentSlot"), "TweakDBID", head, None));
            assert_eq!(get_flat_tdbid(t, tweak_db_id_derive(slot_head, ".parentSlot")), Some(head));
            assert!(super::criar_flat_em(t, id("OutfitSlots.EarLeft.dependencySlots"), "array:TweakDBID", 0, Some(&[head, chest])));
            assert_eq!(get_flat_array_u64_by_id(t, tweak_db_id_derive(slot_orelha, ".dependencySlots")), Some(vec![head, chest]));
            let tags = [crate::cname::cname("Clothing"), crate::cname::cname("Mask")];
            assert!(super::criar_flat_em(t, id("OutfitSlots.EarLeft.bwmsTags"), "array:CName", 0, Some(&tags)));
            assert_eq!(get_flat_array_u64_by_id(t, id("OutfitSlots.EarLeft.bwmsTags")), Some(tags.to_vec()));
            // O array novo tem o alocador do doador no fim do buffer próprio.
            let fv = super::super::get_flat_value_by_id(t, id("OutfitSlots.EarLeft.dependencySlots")).unwrap();
            let entries = (fv.add(8) as *const u64).read_unaligned() as *const u64;
            assert_eq!(*entries.add(2), alocador());
            // O doador continua com o valor dele.
            assert_eq!(get_flat_array_u64_by_id(t, id("Items.GenericHeadClothing.placementSlots")), Some(vec![head]));
            // Recusas: flat que já existe, tipo sem doador, array sem lista.
            assert!(!super::criar_flat_em(t, id("OutfitSlots.Head.parentSlot"), "TweakDBID", eyes, None));
            assert_eq!(get_flat_tdbid(t, id("OutfitSlots.Head.parentSlot")), Some(head));
            assert!(!super::criar_flat_em(t, id("OutfitSlots.Head.localizedName"), "String", 0, None));
            assert!(!super::criar_flat_em(t, id("OutfitSlots.Mask.dependencySlots"), "array:TweakDBID", 0, None));
        }
        // 3 flats novos no índice, ainda ordenado por (hash, len); 3 subidas da versão fora do lote.
        let n = unsafe { (t.add(FLATS_OFF + 0x0C) as *const u32).read_unaligned() } as usize;
        assert_eq!(n, 6);
        let entradas = unsafe { std::slice::from_raw_parts((t.add(FLATS_OFF) as *const u64).read_unaligned() as *const u64, n) };
        assert!(entradas.windows(2).all(|p| super::super::id_key_lt(p[0], p[1])));
        assert_eq!(tdb[super::VERSION_BYTE_OFF], 3);
        drop(valores);
        drop(indice);
    }

    #[test]
    fn setflat_so_cria_os_tipos_que_o_eex_grava() {
        assert_eq!(super::doador_do_tipo("TweakDBID"), Some("Items.Preset_Yukimura_Default.quality"));
        assert_eq!(super::doador_do_tipo("array:TweakDBID"), Some("Items.GenericHeadClothing.placementSlots"));
        assert_eq!(super::doador_do_tipo("array:CName"), Some("Items.GenericHeadClothing.tags"));
        for tipo in ["String", "Int32", "Float", "CName", "array:Int32", "array:String", "gamedataLocKeyWrapper"] {
            assert_eq!(super::doador_do_tipo(tipo), None, "{tipo}");
        }
        // Fora do jogo não há TweakDB: recusa sem tocar em memória.
        assert!(!unsafe { super::criar_flat(1, "TweakDBID", 2, None) });
    }

    /// Propriedade de record com os formatos explícitos da wiki do TweakXL (YAML Tweaks): o mesmo valor do formato implícito.
    #[test]
    fn formatos_explicitos_do_tweakxl_na_propriedade_de_record() {
        let caminho = r"base\characters\entities\player\photo_mode\judy_alvarez\judy_photomode.ent";
        let hash = 0x7b09_932c_9d2e_d4bf_u64; // `Character.Judy_Puppet_Photomode.entityTemplatePath` no tweakdb.bin da 2.5
        let rr = "raRef:CResource";
        assert_eq!(escalar_do_texto(rr, &format!("r\"{caminho}\"")), Some(hash));
        assert_eq!(escalar_do_texto(rr, &format!("ResRef(\"{}\")", caminho.replace('\\', "/"))), Some(hash));
        assert_eq!(escalar_do_texto(rr, &caminho.replace('\\', r"\\")), Some(hash));
        assert_eq!(escalar_do_texto(rr, &format!("ResRef({hash})")), Some(hash));
        assert_eq!(escalar_do_texto(rr, &hash.to_string()), Some(hash));
        let id = bwms_primitives::tweak_db_id("Quality.Random");
        assert_eq!(id, 0x0e_2928_b37a);
        assert_eq!(escalar_do_texto("TweakDBID", "t\"Quality.Random\""), Some(id));
        assert_eq!(escalar_do_texto("TweakDBID", "TweakDBID(\"Quality.Random\")"), Some(id));
        assert_eq!(escalar_do_texto("TweakDBID", "<TDBID:2928B37A:0E>"), Some(id));
        assert_eq!(escalar_do_texto("TweakDBID", "None"), Some(0));
        let lk = "gamedataLocKeyWrapper";
        assert_eq!(escalar_do_texto(lk, "l\"BwmsFmt\""), escalar_do_texto(lk, "BwmsFmt"));
        assert_eq!(escalar_do_texto(lk, "LocKey(\"BwmsFmt\")"), escalar_do_texto(lk, "BwmsFmt"));
        assert_eq!(escalar_do_texto(lk, "LocKey(48683)"), Some(48683));
    }

    #[test]
    fn lockey_em_propriedade_string_vira_lockey_numerico() {
        let fnv = bwms_primitives::fnv1a64;
        assert_eq!(super::string_de_lockey("LocKey#BwmsMagico"), Some(format!("LocKey#{}", fnv(b"BwmsMagico"))));
        assert_eq!(super::string_de_lockey("LocKey#45371").as_deref(), Some("LocKey#45371"));
        assert_eq!(super::string_de_lockey("LocKey(45371)").as_deref(), Some("LocKey#45371"));
        assert_eq!(super::string_de_lockey("l\"BwmsMagico\""), Some(format!("LocKey#{}", fnv(b"BwmsMagico"))));
        assert_eq!(super::string_de_lockey("texto comum"), None);
    }

    #[test]
    fn lockey_do_texto_pela_regra_do_bwms_primitives() {
        let fnv = bwms_primitives::fnv1a64;
        let t = "gamedataLocKeyWrapper";
        assert_eq!(escalar_do_texto(t, "LocKey#45371"), Some(45371));
        assert_eq!(escalar_do_texto(t, "LocKey#abc"), Some(fnv(b"abc")));
        assert_eq!(escalar_do_texto(t, "abc"), Some(fnv(b"abc")));
        // Sem o prefixo, até o decimal e o vazio são chave secundária: é a chave que mod instalado já recebe (TODO em `lockey_chave`).
        assert_eq!(escalar_do_texto(t, "45371"), Some(fnv(b"45371")));
        assert_eq!(escalar_do_texto(t, ""), Some(fnv(b"")));
        assert_eq!(escalar_do_texto(t, "LocKey#"), Some(fnv(b"")));
    }

    // ---- TweakDB falso com flats de qualquer tipo: as 5 diferenças frente ao TweakXL no caminho de arquivo ----

    extern "C" fn tipo_int32(_fv: *mut core::ffi::c_void, _out: *mut u64) -> *mut u64 {
        crate::cname::cname("Int32") as *mut u64
    }
    extern "C" fn tipo_arr_int32(_fv: *mut core::ffi::c_void, _out: *mut u64) -> *mut u64 {
        crate::cname::cname("array:Int32") as *mut u64
    }
    extern "C" fn tipo_arr_float(_fv: *mut core::ffi::c_void, _out: *mut u64) -> *mut u64 {
        crate::cname::cname("array:Float") as *mut u64
    }
    extern "C" fn tipo_arr_bool(_fv: *mut core::ffi::c_void, _out: *mut u64) -> *mut u64 {
        crate::cname::cname("array:Bool") as *mut u64
    }
    extern "C" fn tipo_arr_vec3(_fv: *mut core::ffi::c_void, _out: *mut u64) -> *mut u64 {
        crate::cname::cname("array:Vector3") as *mut u64
    }

    /// Buffer de elementos no layout do motor: os bytes, alinhados a 8, e o alocador no fim.
    fn elementos_com_alocador(bytes: &[u8]) -> &'static [u8] {
        let corpo = (bytes.len() + 7) & !7;
        let mut b = vec![0u8; corpo + 8];
        b[..bytes.len()].copy_from_slice(bytes);
        b[corpo..].copy_from_slice(&alocador().to_le_bytes());
        Box::leak(b.into_boxed_slice())
    }

    /// TweakDB falso: `flats` = `(nome, offset)`, `valores` = conteúdo inicial do buffer de valores (palavras de 8 B), com folga pros novos.
    struct Falso {
        tdb: Vec<u8>,
        _indice: Vec<u64>,
        _valores: Vec<u64>,
    }

    impl Falso {
        fn novo(flats: &[(&str, u32)], iniciais: &[u64]) -> Falso {
            use super::super::{tweak_db_id, FLATS_OFF, FLAT_DATA_BUFFER_OFF, FLAT_DATA_CAP_OFF, FLAT_DATA_END_OFF};
            let mut valores = vec![0u64; 0x400];
            valores[..iniciais.len()].copy_from_slice(iniciais);
            let mut entradas: Vec<u64> = flats.iter().map(|(n, o)| entrada(tweak_db_id(n), *o)).collect();
            entradas.sort_by_key(|e| ((*e & 0xFFFF_FFFF) as u32, ((*e >> 32) & 0xFF) as u8));
            let cap = 32usize;
            let mut indice = vec![0u64; cap + 1];
            indice[..entradas.len()].copy_from_slice(&entradas);
            indice[cap] = alocador();
            let mut tdb = vec![0u8; 0x168];
            let t = tdb.as_mut_ptr();
            unsafe {
                (t.add(FLATS_OFF) as *mut u64).write_unaligned(indice.as_mut_ptr() as u64);
                (t.add(FLATS_OFF + 8) as *mut u32).write_unaligned(cap as u32);
                (t.add(FLATS_OFF + 0x0C) as *mut u32).write_unaligned(entradas.len() as u32);
                (t.add(FLAT_DATA_BUFFER_OFF) as *mut u64).write_unaligned(valores.as_mut_ptr() as u64);
                (t.add(FLAT_DATA_CAP_OFF) as *mut u32).write_unaligned((valores.len() * 8) as u32);
                (t.add(FLAT_DATA_END_OFF) as *mut u64).write_unaligned(valores.as_ptr() as u64 + iniciais.len() as u64 * 8);
            }
            Falso { tdb, _indice: indice, _valores: valores }
        }
        fn t(&mut self) -> *mut u8 {
            self.tdb.as_mut_ptr()
        }
    }

    /// Palavras de um `FlatValue` de array: vtable, entries, `cap | size << 32`.
    fn array(vt: extern "C" fn(*mut core::ffi::c_void, *mut u64) -> *mut u64, bytes: &[u8], n: u32) -> [u64; 3] {
        let entries = if n == 0 { alocador() } else { elementos_com_alocador(bytes).as_ptr() as u64 };
        [vtable(vt).as_ptr() as u64, entries, n as u64 | (n as u64) << 32]
    }

    fn f32s(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// Grava `bytes` no flat array `id` como o escritor faz (carga com o alocador do valor atual, `FlatValue` novo, entrada re-apontada), sem o
    /// diário e a versão, que pedem o TweakDB vivo.
    unsafe fn gravar_no_falso(t: *mut u8, id: u64, elem: &str, itens: &[Vec<u8>]) {
        let (tam, al) = super::elemento_do_tipo(elem).unwrap();
        let dv = super::super::get_flat_value_by_id(t, id).unwrap();
        let carga = super::super::carga_de_array_bytes(dv, tam, al, &itens.concat(), itens.len()).unwrap();
        let off = super::super::criar_valor_com_vft(t, (dv as *const u64).read_unaligned(), &carga, 8).unwrap();
        assert!(super::super::set_flat_offset_by_id(t, id, off as u32));
    }

    /// Diferença 2: array de elemento de 4 B (`Float`, `Int32`), 1 B (`Bool`) e 12 B (`Vector3`): lido no passo do tipo e gravado com o
    /// alocador em `entries + alinha8(n * tam)`, a regra do `DynArray::Reserve` do motor.
    #[test]
    fn array_de_elemento_que_nao_e_de_8_bytes() {
        use super::super::tweak_db_id as id;
        let mut f = Falso::novo(
            &[("Items.X.pesos", 0x00), ("Items.X.ligados", 0x18), ("Items.X.pontos", 0x30)],
            &[array(tipo_arr_float, &f32s(&[1.5, 2.5, 3.5]), 3), array(tipo_arr_bool, &[1, 0], 2), array(tipo_arr_vec3, &f32s(&[1.0, 2.0, 3.0]), 1)]
                .concat(),
        );
        let t = f.t();
        unsafe {
            let (elem, itens) = super::elementos_do_array(t, id("Items.X.pesos")).unwrap();
            assert_eq!(elem, "Float");
            assert_eq!(itens, [1.5f32, 2.5, 3.5].map(|x| x.to_le_bytes().to_vec()));
            assert_eq!(super::elementos_do_array(t, id("Items.X.ligados")).unwrap(), ("Bool".to_string(), vec![vec![1], vec![0]]));
            assert_eq!(super::elementos_do_array(t, id("Items.X.pontos")).unwrap().1, vec![f32s(&[1.0, 2.0, 3.0])]);

            // 5 Float (20 B): o alocador vai em +24.
            let novos: Vec<Vec<u8>> = ["1.5", "2.5", "3.5", "4.5", "5"].iter().map(|v| super::elemento_em_bytes("Float", v).unwrap()).collect();
            gravar_no_falso(t, id("Items.X.pesos"), "Float", &novos);
            assert_eq!(super::elementos_do_array(t, id("Items.X.pesos")).unwrap().1, novos);
            let fv = super::super::get_flat_value_by_id(t, id("Items.X.pesos")).unwrap();
            let entries = (fv.add(8) as *const u64).read_unaligned() as *const u8;
            assert_eq!((entries.add(24) as *const u64).read_unaligned(), alocador());
            assert_eq!((fv.add(0x10) as *const u32).read_unaligned(), 5);

            // 3 Bool (3 B): o alocador vai em +8.
            gravar_no_falso(t, id("Items.X.ligados"), "Bool", &[vec![1], vec![1], vec![0]]);
            let fv = super::super::get_flat_value_by_id(t, id("Items.X.ligados")).unwrap();
            let entries = (fv.add(8) as *const u64).read_unaligned() as *const u8;
            assert_eq!((entries.add(8) as *const u64).read_unaligned(), alocador());
            assert_eq!(super::elementos_do_array(t, id("Items.X.ligados")).unwrap().1, vec![vec![1], vec![1], vec![0]]);
        }
        assert_eq!(super::elemento_do_tipo("Float"), Some((4, 4)));
        assert_eq!(super::elemento_do_tipo("Bool"), Some((1, 1)));
        assert_eq!(super::elemento_do_tipo("Vector3"), Some((12, 4)));
        assert_eq!(super::elemento_do_tipo("Quaternion"), Some((16, 16)));
        assert_eq!(super::elemento_do_tipo("String"), Some((0x20, 8)));
        assert_eq!(super::elemento_do_tipo("raRef:CResource"), Some((8, 8)));
        // Struct: EulerAngles do texto (pitch,yaw,roll) pra memória {Roll, Pitch, Yaw}; Color 4×u8.
        assert_eq!(super::bytes_do_texto("EulerAngles", "10,20,30"), Some(f32s(&[30.0, 10.0, 20.0])));
        assert_eq!(super::bytes_do_texto("Color", "1,2,3,255"), Some(vec![1, 2, 3, 255]));
        assert_eq!(super::bytes_do_texto("Quaternion", "0,0,0,1"), Some(f32s(&[0.0, 0.0, 0.0, 1.0])));
        assert_eq!(super::bytes_do_texto("Int32", "-2"), Some((-2i32).to_le_bytes().to_vec()));
        assert_eq!(super::bytes_do_texto("Vector3", "1,2"), None);
        // Tupla do `.tweak` sem tipo declarado: já na ordem de memória (`world_map.tweak`, `rotationMin = ( 0.f, -85.f, -360.f )`, igual ao
        // `tweakdb.bin` da 2.5).
        assert_eq!(super::bytes_do_texto("EulerAngles", "(0.,-85.,-360.)"), Some(f32s(&[0.0, -85.0, -360.0])));
        assert_eq!(super::bytes_do_texto("Vector3", "(1,2,3)"), Some(f32s(&[1.0, 2.0, 3.0])));
    }

    /// Diferenças 1 e 4: o flat que o arquivo declara com tipo (`$type`+`$value`, tipo do `.tweak`, `$props: AutoFlats`) nasce no índice com a
    /// vtable do valor padrão do tipo; o array novo leva o alocador do padrão vazio (`entries` = alocador com `cap` 0).
    #[test]
    fn flat_novo_de_arquivo_nasce_com_a_vtable_do_padrao_do_tipo() {
        use super::super::tweak_db_id as id;
        let int_padrao = [vtable(tipo_int32).as_ptr() as u64, 0];
        let float_padrao = array(tipo_arr_float, &[], 0);
        let mut f = Falso::novo(&[("Items.X.dano", 0x00)], &[int_padrao.as_slice(), float_padrao.as_slice()].concat());
        let t = f.t();
        unsafe {
            let vb = (t.add(super::super::FLAT_DATA_BUFFER_OFF) as *const u64).read_unaligned() as *const u8;
            let off = super::criar_flat_com_padrao(t, id("Items.X.bwmsNivel"), "Int32", "7", vb).unwrap();
            assert!(off >= 0x28);
            assert_eq!(super::tipo_do_valor(t, id("Items.X.bwmsNivel")).as_deref(), Some("Int32"));
            // O leitor do jogo (`GetDataPtr`, vt+0xF0) acha o valor no `+8`.
            assert_eq!(super::super::dado_do_flat(t, id("Items.X.bwmsNivel"), "Int32").map(|p| (p as *const i32).read_unaligned()), Some(7));

            super::criar_flat_com_padrao(t, id("Items.X.bwmsPesos"), "array:Float", "[1.5, 2]", vb.add(0x10)).unwrap();
            assert_eq!(super::elementos_do_array(t, id("Items.X.bwmsPesos")).unwrap(), ("Float".to_string(), vec![f32s(&[1.5]), f32s(&[2.0])]));
            let fv = super::super::get_flat_value_by_id(t, id("Items.X.bwmsPesos")).unwrap();
            let entries = (fv.add(8) as *const u64).read_unaligned() as *const u8;
            assert_eq!((entries.add(8) as *const u64).read_unaligned(), alocador());
        }
    }

    /// Diferença 5 no runtime: a mutação do conjunto é calculada sobre o valor que a atribuição deixou, qualquer que seja a ordem dos arquivos.
    #[test]
    fn mutacao_do_conjunto_sobre_o_valor_atribuido() {
        use super::super::tweak_db_id as id;
        use bwms_primitives::operacoes::Conjunto;
        use bwms_primitives::tweakxl::{interpret_from, yaml};
        let i32s = |v: &[i32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let mut f = Falso::novo(&[("Items.X.tags", 0x00)], &array(tipo_arr_int32, &i32s(&[1, 2]), 2));
        let t = f.t();
        let ops = |s: &str| interpret_from(&yaml::parse(s).unwrap(), "t.yaml").unwrap();
        let c = Conjunto::montar([ops("Items.X:\n  tags:\n    - !append 9\n    - !prepend 0\n"), ops("Items.X:\n  tags: [4]\n")]);
        unsafe {
            // A fase de atribuição grava o `[4]` do 2º arquivo.
            gravar_no_falso(t, id("Items.X.tags"), "Int32", &[i32s(&[4])]);
            let (elem, novo) = super::super::mutacao_calculada(t, &c, "Items.X.tags").unwrap();
            assert_eq!(elem, "Int32");
            assert_eq!(novo.concat(), i32s(&[0, 4, 9]));
        }
    }

    /// Diferença 3 no runtime: as perguntas da re-herança respondidas pelo TweakDB (mesmo offset = mesmo valor; array que começa com o do
    /// base).
    #[test]
    fn consulta_de_heranca_pelo_tweakdb() {
        use bwms_primitives::operacoes::ConsultaDeHeranca;
        let i32s = |v: &[i32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let mut f = Falso::novo(
            &[("Items.Base.tags", 0x00), ("Items.Filho.tags", 0x18), ("Items.Outro.tags", 0x30), ("Items.Igual.tags", 0x00)],
            &[array(tipo_arr_int32, &i32s(&[1, 2]), 2), array(tipo_arr_int32, &i32s(&[1, 2, 3]), 3), array(tipo_arr_int32, &i32s(&[5]), 1)].concat(),
        );
        let c = super::super::ConsultaDoTweakDb(f.t());
        assert!(c.flat_existe("Items.Filho.tags") && !c.flat_existe("Items.Nada.tags"));
        assert!(c.mesmo_valor("Items.Base.tags", "Items.Igual.tags"));
        assert!(!c.mesmo_valor("Items.Base.tags", "Items.Filho.tags"));
        assert!(c.e_array("Items.Base.tags"));
        assert!(c.comeca_com("Items.Base.tags", "Items.Filho.tags"));
        assert!(!c.comeca_com("Items.Base.tags", "Items.Outro.tags"));
    }
}
