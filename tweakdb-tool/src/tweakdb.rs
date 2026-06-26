//! Leitor do `tweakdb.bin` do Cyberpunk 2077 — port do codec do WolvenKit
//! (`WolvenKit.RED4/TweakDB`). Os flats NÃO são comprimidos, então é tudo
//! leitura direta (sem Kraken).
//!
//! Layout (little-endian):
//! ```text
//! magic u32 = 0x0BB1DB47
//! Header(28): blobVersion i32(=8), parserVersion i32(=4), recordChecksum u32,
//!             flatsOffset i32, recordsOffset i32, queriesOffset i32, groupTagsOffset i32
//! @flatsOffset    numFlatTypes i32; FlatTypeInfo×N {typeHash u64, valueCount u32,
//!                 keyCount u32, offset u32}; e em cada offset: numValues u32, valores,
//!                 numKeys u32, {keyId u64 (TweakDBID), valueIndex i32}×K
//! @recordsOffset  numRecords i32; {id u64 (TweakDBID), typeKey u32 (Murmur32)}×N
//! @queriesOffset  numQueries i32; {id u64, numResults u32, result u64×R}×N
//! @groupTagsOffset numGroupTags i32; {id u64, tag u8}×N
//! ```
//! O `typeHash` de um grupo de flats é o FNV-1a64 do nome RED do tipo (escalar)
//! ou de `"array:"+nome` (array). `TweakDBID` é um u64: CRC32(nome) | (len<<32).

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::hashes::fnv1a64;

pub const MAGIC: u32 = 0x0BB1_DB47;
pub const BLOB_VERSION: i32 = 8;
pub const PARSER_VERSION: i32 = 4;

/// Os 20 `ETweakType`, em ordem, com (rótulo, nome RED). O nome RED é o que
/// entra no FNV-1a64 para formar o `typeHash`.
pub const TWEAK_TYPES: [(&str, &str); 20] = [
    ("CName", "CName"),
    ("CString", "String"),
    ("TweakDBID", "TweakDBID"),
    ("CResource", "raRef:CResource"),
    ("CFloat", "Float"),
    ("CBool", "Bool"),
    ("CUint8", "Uint8"),
    ("CUint16", "Uint16"),
    ("CUint32", "Uint32"),
    ("CUint64", "Uint64"),
    ("CInt8", "Int8"),
    ("CInt16", "Int16"),
    ("CInt32", "Int32"),
    ("CInt64", "Int64"),
    ("CColor", "Color"),
    ("CEulerAngles", "EulerAngles"),
    ("CQuaternion", "Quaternion"),
    ("CVector2", "Vector2"),
    ("CVector3", "Vector3"),
    ("LocKey", "gamedataLocKeyWrapper"),
];

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Format(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "erro de E/S: {e}"),
            Error::Format(m) => write!(f, "tweakdb.bin inválido: {m}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;

/// Resolução de um `typeHash`: índice em [`TWEAK_TYPES`] e se é array.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedType {
    pub index: usize,
    pub is_array: bool,
}

impl ResolvedType {
    pub fn label(&self) -> String {
        let base = TWEAK_TYPES[self.index].0;
        if self.is_array {
            format!("array:{base}")
        } else {
            base.to_string()
        }
    }
}

/// Um grupo de flats: todos os valores de um mesmo tipo, e as chaves que os referenciam.
#[derive(Debug)]
pub struct FlatType {
    pub type_hash: u64,
    pub resolved: Option<ResolvedType>,
    pub value_count: u32,
    pub key_count: u32,
    pub offset: u32,
}

/// Valor de um flat (escalar ou array). Suficiente para inspecionar dano/stats.
#[derive(Debug, Clone, PartialEq)]
pub enum FlatValue {
    Bool(bool),
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    Float(f32),
    Str(String),
    /// TweakDBID / CResource / LocKey — todos u64 no disco.
    Id(u64),
    Color([u8; 4]),
    Floats(Vec<f32>), // Vector2/3, EulerAngles, Quaternion
    Array(Vec<FlatValue>),
}

impl fmt::Display for FlatValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlatValue::Bool(v) => write!(f, "{v}"),
            FlatValue::I8(v) => write!(f, "{v}"),
            FlatValue::U8(v) => write!(f, "{v}"),
            FlatValue::I16(v) => write!(f, "{v}"),
            FlatValue::U16(v) => write!(f, "{v}"),
            FlatValue::I32(v) => write!(f, "{v}"),
            FlatValue::U32(v) => write!(f, "{v}"),
            FlatValue::I64(v) => write!(f, "{v}"),
            FlatValue::U64(v) => write!(f, "{v}"),
            FlatValue::Float(v) => write!(f, "{v}"),
            FlatValue::Str(v) => write!(f, "{v:?}"),
            FlatValue::Id(v) => write!(f, "TDBID({v:016x})"),
            FlatValue::Color(c) => write!(f, "Color{c:?}"),
            FlatValue::Floats(v) => write!(f, "{v:?}"),
            FlatValue::Array(items) => {
                write!(f, "[")?;
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{it}")?;
                }
                write!(f, "]")
            }
        }
    }
}

/// Um record: id (TweakDBID) e a chave do tipo (Murmur32 do nome da classe).
#[derive(Debug, Clone, Copy)]
pub struct Record {
    pub id: u64,
    pub type_key: u32,
}

/// Um grupo de flats com os valores como **bytes crus** (para reescrita exata).
#[derive(Debug, Clone)]
pub struct RawGroup {
    pub type_hash: u64,
    pub values: Vec<Vec<u8>>,
    pub keys: Vec<(u64, i32)>,
}

/// Codifica um valor ESCALAR editável a partir de texto, nos bytes que o tweakdb
/// espera. Cobre os 20 ETweakType escalares: numéricos/bool (parse direto = faixa
/// checada), strings (VLQ), TweakDBID/CResource/LocKey (u64; aceita `$Nome`, hex
/// `0x..` ou decimal), Color (`r,g,b,a` 0-255 ou `#RRGGBBAA`) e vetores (floats
/// por vírgula). O writer recomputa offsets, então valores de tamanho variável
/// (string) são OK. Arrays são tratados à parte (decode→modifica→re-encode).
pub fn encode_value(type_index: usize, text: &str) -> std::result::Result<Vec<u8>, String> {
    let t = text.trim();
    Ok(match type_index {
        0 => encode_lp_string(&normalize_cname(t)),            // CName (aceita n"X"/CName("X"))
        1 => encode_lp_string(t),                              // CString (literal)
        2 | 3 | 19 => parse_id(t)?.to_le_bytes().to_vec(),     // TweakDBID / CResource / LocKey
        4 => t.parse::<f32>().map_err(|_| invalid(t, "Float"))?.to_le_bytes().to_vec(),
        5 => vec![parse_bool(t)?],
        6 => t.parse::<u8>().map_err(|_| invalid(t, "Uint8"))?.to_le_bytes().to_vec(),
        7 => t.parse::<u16>().map_err(|_| invalid(t, "Uint16"))?.to_le_bytes().to_vec(),
        8 => t.parse::<u32>().map_err(|_| invalid(t, "Uint32"))?.to_le_bytes().to_vec(),
        9 => t.parse::<u64>().map_err(|_| invalid(t, "Uint64"))?.to_le_bytes().to_vec(),
        10 => t.parse::<i8>().map_err(|_| invalid(t, "Int8"))?.to_le_bytes().to_vec(),
        11 => t.parse::<i16>().map_err(|_| invalid(t, "Int16"))?.to_le_bytes().to_vec(),
        12 => t.parse::<i32>().map_err(|_| invalid(t, "Int32"))?.to_le_bytes().to_vec(),
        13 => t.parse::<i64>().map_err(|_| invalid(t, "Int64"))?.to_le_bytes().to_vec(),
        14 => parse_color(t)?.to_vec(),                         // Color [R,G,B,A]
        15 => parse_floats(t, 3, "EulerAngles")?,              // pitch,yaw,roll
        16 => parse_floats(t, 4, "Quaternion")?,               // i,j,k,r
        17 => parse_floats(t, 2, "Vector2")?,
        18 => parse_floats(t, 3, "Vector3")?,
        _ => return Err(format!("índice de tipo {type_index} desconhecido")),
    })
}

fn invalid(text: &str, label: &str) -> String {
    format!("'{text}' inválido ou fora da faixa para {label}")
}

/// Bool case-insensitive; rejeita texto não reconhecido (não vira `false`
/// silencioso — o bug que reportava sucesso e gravava o byte errado).
fn parse_bool(text: &str) -> std::result::Result<u8, String> {
    match text.to_ascii_lowercase().as_str() {
        "1" | "true" => Ok(1),
        "0" | "false" => Ok(0),
        _ => Err(format!("'{text}' não é um Bool válido (use true/false/1/0)")),
    }
}

/// Remove aspas simples/duplas externas, se houver.
fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Normaliza um CName das formas do TweakXL (`n"X"`, `CName("X")`) para o texto
/// nu. `None` e demais passam direto.
fn normalize_cname(text: &str) -> String {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix('n') {
        let q = rest.trim();
        if (q.starts_with('"') || q.starts_with('\'')) && q.len() >= 2 {
            return strip_quotes(q).to_string();
        }
    }
    if let Some(inner) = t.strip_prefix("CName(").and_then(|x| x.strip_suffix(')')) {
        return strip_quotes(inner).to_string();
    }
    t.to_string()
}

/// u64 de um id/ref. Aceita as formas do TweakXL além das nossas:
/// `$Nome` · `t"Nome"`/`t'Nome'` · `TweakDBID("Nome")` · `LocKey#N` ·
/// `<TDBID:CRC:LEN>` · `None`/vazio (=0) · `0x..` hex · decimal u64 ·
/// e foreign-key implícito (token com `.`/letra → nome).
fn parse_id(text: &str) -> std::result::Result<u64, String> {
    let t = text.trim();
    if t.is_empty() || t == "None" {
        return Ok(0);
    }
    if t.starts_with("r\"") || t.starts_with("r'") || t.starts_with("ResRef(") {
        return Err(format!("'{t}': ref de recurso (ResRef) por caminho ainda não suportado"));
    }
    if let Some(name) = t.strip_prefix('$') {
        return Ok(crate::hashes::tweak_db_id(name));
    }
    // t"Nome" / t'Nome' (estilo redscript)
    if let Some(rest) = t.strip_prefix('t') {
        let q = rest.trim();
        if (q.starts_with('"') || q.starts_with('\'')) && q.len() >= 2 {
            return Ok(crate::hashes::tweak_db_id(strip_quotes(q)));
        }
    }
    if let Some(inner) = t.strip_prefix("TweakDBID(").and_then(|x| x.strip_suffix(')')) {
        return Ok(crate::hashes::tweak_db_id(strip_quotes(inner)));
    }
    if let Some(num) = t.strip_prefix("LocKey#") {
        return num.trim().parse::<u64>().map_err(|_| invalid(t, "LocKey#<número>"));
    }
    // <TDBID:CRC32:LEN> (estilo debug)
    if let Some(inner) = t.strip_prefix("<TDBID:").and_then(|x| x.strip_suffix('>')) {
        let mut it = inner.split(':');
        if let (Some(crc), Some(len)) = (it.next(), it.next()) {
            let crc = u32::from_str_radix(crc.trim(), 16).map_err(|_| invalid(t, "TDBID crc"))?;
            let len = u8::from_str_radix(len.trim(), 16).map_err(|_| invalid(t, "TDBID len"))?;
            return Ok(u64::from(crc) | (u64::from(len) << 32));
        }
    }
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).map_err(|_| invalid(t, "id (hex u64)"));
    }
    if let Ok(n) = t.parse::<u64>() {
        return Ok(n);
    }
    // Foreign-key implícito: `Vehicle.X`, `Items.Y` → TweakDBID por nome.
    if t.chars().any(|c| c == '.' || c.is_ascii_alphabetic()) {
        return Ok(crate::hashes::tweak_db_id(t));
    }
    Err(invalid(t, "id ($Nome, t\"Nome\", LocKey#N, 0xHEX, decimal ou None)"))
}

/// Color de `r,g,b,a` (0-255) ou `#RRGGBBAA` → [R,G,B,A].
fn parse_color(text: &str) -> std::result::Result<[u8; 4], String> {
    if let Some(hex) = text.strip_prefix('#') {
        if hex.len() != 8 {
            return Err(format!("'{text}': Color hex precisa de 8 dígitos (#RRGGBBAA)"));
        }
        let nib = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| invalid(text, "Color"));
        return Ok([nib(0)?, nib(2)?, nib(4)?, nib(6)?]);
    }
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(format!("'{text}': Color precisa de 'r,g,b,a' (0-255) ou '#RRGGBBAA'"));
    }
    let mut out = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p.parse::<u8>().map_err(|_| invalid(p, "Color (0-255)"))?;
    }
    Ok(out)
}

/// `n` floats separados por vírgula → n*4 bytes LE.
fn parse_floats(text: &str, n: usize, label: &str) -> std::result::Result<Vec<u8>, String> {
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    if parts.len() != n {
        return Err(format!("'{text}': {label} precisa de {n} floats separados por vírgula"));
    }
    let mut out = Vec::with_capacity(n * 4);
    for p in parts {
        out.extend_from_slice(&p.parse::<f32>().map_err(|_| invalid(p, label))?.to_le_bytes());
    }
    Ok(out)
}

/// String length-prefixed do tweakdb: VLQ(sinal) + bytes. Escreve UTF-8 com
/// prefixo NEGATIVO (= -nº de bytes), forma que o reader espera (prefix < 0).
fn encode_lp_string(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = vlq_encode(-(bytes.len() as i32));
    out.extend_from_slice(bytes);
    out
}

/// Inverso do `Cursor::vlq_i32` — VLQ assinado da CDPR (bit7=sinal, bit6=cont no
/// 1º octeto; demais LEB128).
pub fn vlq_encode(value: i32) -> Vec<u8> {
    let neg = value < 0;
    let mut v = value.unsigned_abs();
    let mut out = Vec::new();
    let mut first = (v & 0x3F) as u8;
    if neg {
        first |= 0x80;
    }
    v >>= 6;
    if v > 0 {
        first |= 0x40;
    }
    out.push(first);
    while v > 0 {
        let mut b = (v & 0x7F) as u8;
        v >>= 7;
        if v > 0 {
            b |= 0x80;
        }
        out.push(b);
    }
    out
}

/// Monta o valor cru de um array: VLQ(count) + bytes de cada elemento.
pub fn encode_array(elements: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vlq_encode(elements.len() as i32);
    for e in elements {
        out.extend_from_slice(e);
    }
    out
}

/// Quebra o valor cru de um array (VLQ count + elementos) em blobs por elemento,
/// medindo cada um pelo tipo escalar do elemento. Permite append/remove byte-exato.
pub fn split_array_elements(
    raw: &[u8],
    element_type_index: usize,
) -> Result<Vec<Vec<u8>>> {
    let mut cur = Cursor::new(raw);
    let count = cur.vlq_i32()?;
    if count < 0 {
        return Err(Error::Format("contagem de array negativa".into()));
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let start = cur.pos();
        read_scalar(&mut cur, element_type_index)?; // só pra medir o tamanho
        out.push(raw[start..cur.pos()].to_vec());
    }
    Ok(out)
}

pub struct TweakDb {
    pub path: PathBuf,
    pub data: Vec<u8>,
    pub blob_version: i32,
    pub parser_version: i32,
    pub record_checksum: u32,
    pub flats_offset: u32,
    pub records_offset: u32,
    pub queries_offset: u32,
    pub group_tags_offset: u32,
    pub flat_types: Vec<FlatType>,
    pub records: Vec<Record>,
    pub query_count: u32,
    pub group_tag_count: u32,
}

impl TweakDb {
    /// Abre e parseia a estrutura (header + tabela de flat types + records/
    /// queries/group tags). NÃO lê os valores dos flats (use [`Self::read_values`]).
    pub fn open(path: &Path) -> Result<TweakDb> {
        let data = fs::read(path)?;
        let mut cur = Cursor::new(&data);

        if cur.u32()? != MAGIC {
            return Err(Error::Format("magic não é 0x0BB1DB47".into()));
        }
        let blob_version = cur.i32()?;
        let parser_version = cur.i32()?;
        if blob_version != BLOB_VERSION || parser_version != PARSER_VERSION {
            return Err(Error::Format(format!(
                "versão não suportada (blob {blob_version}, parser {parser_version}; \
                 esperado {BLOB_VERSION}/{PARSER_VERSION})"
            )));
        }
        let record_checksum = cur.u32()?;
        let flats_offset = cur.i32()? as u32;
        let records_offset = cur.i32()? as u32;
        let queries_offset = cur.i32()? as u32;
        let group_tags_offset = cur.i32()? as u32;

        // Mapa typeHash -> ResolvedType (escalar e array de cada ETweakType).
        let mut type_hashes = Vec::with_capacity(40);
        for (index, (_, red_name)) in TWEAK_TYPES.iter().enumerate() {
            type_hashes.push((
                fnv1a64(red_name.as_bytes()),
                ResolvedType { index, is_array: false },
            ));
            let arr = format!("array:{red_name}");
            type_hashes.push((
                fnv1a64(arr.as_bytes()),
                ResolvedType { index, is_array: true },
            ));
        }
        let resolve = |h: u64| type_hashes.iter().find(|(hh, _)| *hh == h).map(|(_, r)| *r);

        // --- Tabela de flat types ---
        cur.seek(flats_offset as usize)?;
        let num_flat_types = cur.i32()?;
        if num_flat_types < 0 {
            return Err(Error::Format("numFlatTypes negativo".into()));
        }
        let mut flat_types = Vec::with_capacity(num_flat_types as usize);
        for _ in 0..num_flat_types {
            let type_hash = cur.u64()?;
            let value_count = cur.u32()?;
            let key_count = cur.u32()?;
            let offset = cur.u32()?;
            flat_types.push(FlatType {
                type_hash,
                resolved: resolve(type_hash),
                value_count,
                key_count,
                offset,
            });
        }

        // --- Records (id + typeKey) ---
        cur.seek(records_offset as usize)?;
        let num_records = cur.i32()?;
        if num_records < 0 {
            return Err(Error::Format("numRecords negativo".into()));
        }
        let mut records = Vec::with_capacity(num_records as usize);
        for _ in 0..num_records {
            let id = cur.u64()?;
            let type_key = cur.u32()?;
            records.push(Record { id, type_key });
        }

        // --- Queries / GroupTags: só as contagens (estrutura) ---
        cur.seek(queries_offset as usize)?;
        let query_count = cur.i32()?.max(0) as u32;
        cur.seek(group_tags_offset as usize)?;
        let group_tag_count = cur.i32()?.max(0) as u32;

        Ok(TweakDb {
            path: path.to_path_buf(),
            data,
            blob_version,
            parser_version,
            record_checksum,
            flats_offset,
            records_offset,
            queries_offset,
            group_tags_offset,
            flat_types,
            records,
            query_count,
            group_tag_count,
        })
    }

    /// Lê os valores de um grupo de flats (por índice na tabela), devolvendo os
    /// pares (keyId, valor). `keyId` é o TweakDBID (CRC32(nome) | len<<32).
    pub fn read_values(&self, flat_type_index: usize) -> Result<Vec<(u64, FlatValue)>> {
        let ft = self
            .flat_types
            .get(flat_type_index)
            .ok_or_else(|| Error::Format("índice de flat type fora do alcance".into()))?;
        let resolved = ft
            .resolved
            .ok_or_else(|| Error::Format(format!("typeHash {:016x} desconhecido", ft.type_hash)))?;

        let mut cur = Cursor::new(&self.data);
        cur.seek(ft.offset as usize)?;

        let num_values = cur.u32()?;
        let mut values = Vec::with_capacity(num_values as usize);
        for _ in 0..num_values {
            values.push(read_value(&mut cur, resolved)?);
        }

        let num_keys = cur.u32()?;
        let mut out = Vec::with_capacity(num_keys as usize);
        for _ in 0..num_keys {
            let key_id = cur.u64()?;
            let value_index = cur.i32()?;
            let value = values
                .get(value_index as usize)
                .ok_or_else(|| Error::Format("valueIndex fora do alcance".into()))?
                .clone();
            out.push((key_id, value));
        }
        Ok(out)
    }

    /// Lê um grupo de flats preservando os **bytes crus** de cada valor (para
    /// round-trip/edição byte-exata — strings VLQ são ambíguas ao decodificar).
    pub fn read_group_raw(&self, flat_type_index: usize) -> Result<RawGroup> {
        let ft = self
            .flat_types
            .get(flat_type_index)
            .ok_or_else(|| Error::Format("índice de flat type fora do alcance".into()))?;
        let resolved = ft
            .resolved
            .ok_or_else(|| Error::Format(format!("typeHash {:016x} desconhecido", ft.type_hash)))?;

        let mut cur = Cursor::new(&self.data);
        cur.seek(ft.offset as usize)?;

        let num_values = cur.u32()?;
        let mut values = Vec::with_capacity(num_values as usize);
        for _ in 0..num_values {
            let start = cur.pos();
            read_value(&mut cur, resolved)?; // só para medir o tamanho do valor
            values.push(self.data[start..cur.pos()].to_vec());
        }

        let num_keys = cur.u32()?;
        let mut keys = Vec::with_capacity(num_keys as usize);
        for _ in 0..num_keys {
            let id = cur.u64()?;
            let value_index = cur.i32()?;
            keys.push((id, value_index));
        }
        Ok(RawGroup {
            type_hash: ft.type_hash,
            values,
            keys,
        })
    }

    /// Lê a seção de queries completa: (id, resultados).
    pub fn read_queries(&self) -> Result<Vec<(u64, Vec<u64>)>> {
        let mut cur = Cursor::new(&self.data);
        cur.seek(self.queries_offset as usize)?;
        let n = cur.i32()?.max(0) as usize;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let id = cur.u64()?;
            let num = cur.u32()?;
            let mut results = Vec::with_capacity(num as usize);
            for _ in 0..num {
                results.push(cur.u64()?);
            }
            out.push((id, results));
        }
        Ok(out)
    }

    /// Lê a seção de group tags completa: (id, tag).
    pub fn read_group_tags(&self) -> Result<Vec<(u64, u8)>> {
        let mut cur = Cursor::new(&self.data);
        cur.seek(self.group_tags_offset as usize)?;
        let n = cur.i32()?.max(0) as usize;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let id = cur.u64()?;
            let tag = cur.u8()?;
            out.push((id, tag));
        }
        Ok(out)
    }

    /// Total de chaves de flat (recursos de stats endereçáveis).
    pub fn total_flat_keys(&self) -> u64 {
        self.flat_types.iter().map(|t| u64::from(t.key_count)).sum()
    }

    /// Quantidade de tipos de record distintos (chaves Murmur32).
    pub fn distinct_record_types(&self) -> usize {
        let mut keys: Vec<u32> = self.records.iter().map(|r| r.type_key).collect();
        keys.sort_unstable();
        keys.dedup();
        keys.len()
    }
}

/// Lê um valor do tipo resolvido (escalar ou array).
fn read_value(cur: &mut Cursor, ty: ResolvedType) -> Result<FlatValue> {
    if ty.is_array {
        let count = cur.vlq_i32()?;
        if count < 0 {
            return Err(Error::Format("contagem de array negativa".into()));
        }
        let mut items = Vec::with_capacity(count as usize);
        for _ in 0..count {
            items.push(read_scalar(cur, ty.index)?);
        }
        Ok(FlatValue::Array(items))
    } else {
        read_scalar(cur, ty.index)
    }
}

/// Lê um valor escalar do `ETweakType` dado pelo índice.
fn read_scalar(cur: &mut Cursor, type_index: usize) -> Result<FlatValue> {
    Ok(match type_index {
        0 => FlatValue::Str(cur.lp_string()?),  // CName
        1 => FlatValue::Str(cur.lp_string()?),  // CString
        2 => FlatValue::Id(cur.u64()?),         // TweakDBID
        3 => FlatValue::Id(cur.u64()?),         // CResource (raRef)
        4 => FlatValue::Float(cur.f32()?),      // Float
        5 => FlatValue::Bool(cur.u8()? != 0),   // Bool
        6 => FlatValue::U8(cur.u8()?),          // Uint8
        7 => FlatValue::U16(cur.u16()?),        // Uint16
        8 => FlatValue::U32(cur.u32()?),        // Uint32
        9 => FlatValue::U64(cur.u64()?),        // Uint64
        10 => FlatValue::I8(cur.u8()? as i8),   // Int8
        11 => FlatValue::I16(cur.u16()? as i16), // Int16
        12 => FlatValue::I32(cur.i32()?),       // Int32
        13 => FlatValue::I64(cur.u64()? as i64), // Int64
        14 => FlatValue::Color([cur.u8()?, cur.u8()?, cur.u8()?, cur.u8()?]), // Color
        15 => FlatValue::Floats(vec![cur.f32()?, cur.f32()?, cur.f32()?]), // EulerAngles
        16 => FlatValue::Floats(vec![cur.f32()?, cur.f32()?, cur.f32()?, cur.f32()?]), // Quaternion
        17 => FlatValue::Floats(vec![cur.f32()?, cur.f32()?]), // Vector2
        18 => FlatValue::Floats(vec![cur.f32()?, cur.f32()?, cur.f32()?]), // Vector3
        19 => FlatValue::Id(cur.u64()?),        // LocKey (wrapper de u64)
        _ => return Err(Error::Format("índice de tipo desconhecido".into())),
    })
}

/// Cursor little-endian sobre os bytes do arquivo.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Cursor { data, pos: 0 }
    }
    fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(Error::Format(format!("offset {pos} além do fim")));
        }
        self.pos = pos;
        Ok(())
    }
    fn pos(&self) -> usize {
        self.pos
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.data.len())
            .ok_or_else(|| Error::Format(format!("leitura de {n} bytes além do fim em {}", self.pos)))?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// VLQ signed (LEB128 modificado da CDPR): bit7=sinal, bit6=continuação no
    /// 1º octeto; demais octetos LEB128 padrão (máx. 5 bytes). Retorna o inteiro.
    fn vlq_i32(&mut self) -> Result<i32> {
        let b = self.u8()?;
        let negative = b & 0b1000_0000 != 0;
        let mut value: u32 = (b & 0b0011_1111) as u32;
        let mut more = b & 0b0100_0000 != 0;
        let mut shift = 6;
        let mut guard = 0;
        while more {
            let nb = self.u8()?;
            value |= ((nb & 0b0111_1111) as u32) << shift;
            more = nb & 0b1000_0000 != 0;
            shift += 7;
            guard += 1;
            if guard >= 4 && more {
                return Err(Error::Format("VLQ excede 5 bytes".into()));
            }
        }
        let v = value as i32;
        Ok(if negative { -v } else { v })
    }

    /// String com prefixo VLQ: |prefix| = nº de caracteres; sinal do prefixo
    /// indica a codificação (>0 = UTF-16LE, <0 = UTF-8).
    fn lp_string(&mut self) -> Result<String> {
        let prefix = self.vlq_i32()?;
        let len = prefix.unsigned_abs() as usize;
        if len == 0 {
            return Ok(String::new());
        }
        if prefix > 0 {
            let bytes = self.take(len * 2)?;
            let utf16: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            Ok(String::from_utf16_lossy(&utf16))
        } else {
            Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlq_decodifica_valores() {
        // 0 -> um octeto 0x00.
        let mut c = Cursor::new(&[0x00]);
        assert_eq!(c.vlq_i32().unwrap(), 0);
        // 5 cabe nos 6 bits do 1º octeto, sem continuação.
        let mut c = Cursor::new(&[0x05]);
        assert_eq!(c.vlq_i32().unwrap(), 5);
        // Negativo: bit7 setado, valor 1 -> -1.
        let mut c = Cursor::new(&[0b1000_0001]);
        assert_eq!(c.vlq_i32().unwrap(), -1);
        // 64 = precisa do 2º octeto: 1º = cont(0x40), 2º = 1 -> 1<<6 = 64.
        let mut c = Cursor::new(&[0b0100_0000, 0x01]);
        assert_eq!(c.vlq_i32().unwrap(), 64);
    }

    #[test]
    fn string_utf8_negativa() {
        // prefix -3 (UTF-8, 3 chars) seguido de "abc".
        let mut buf = vec![0b1000_0011]; // sinal + valor 3
        buf.extend_from_slice(b"abc");
        let mut c = Cursor::new(&buf);
        assert_eq!(c.lp_string().unwrap(), "abc");
    }

    #[test]
    fn type_hashes_distintos_e_resolvem() {
        // Cada ETweakType (escalar e array) gera um typeHash distinto.
        let mut seen = std::collections::HashSet::new();
        for (_, red) in TWEAK_TYPES {
            assert!(seen.insert(fnv1a64(red.as_bytes())));
            assert!(seen.insert(fnv1a64(format!("array:{red}").as_bytes())));
        }
        assert_eq!(seen.len(), 40);
    }

    #[test]
    fn encode_bool_case_insensitive_e_rejeita_lixo() {
        assert_eq!(encode_value(5, "true").unwrap(), vec![1]);
        assert_eq!(encode_value(5, "TRUE").unwrap(), vec![1]); // antes virava false
        assert_eq!(encode_value(5, "false").unwrap(), vec![0]);
        assert_eq!(encode_value(5, "0").unwrap(), vec![0]);
        assert!(encode_value(5, "2").is_err()); // antes reportava sucesso e gravava false
        assert!(encode_value(5, "yes").is_err());
    }

    #[test]
    fn encode_int_rejeita_overflow() {
        assert_eq!(encode_value(12, "1000").unwrap(), 1000i32.to_le_bytes().to_vec());
        assert!(encode_value(12, "5000000000").is_err()); // > i32::MAX (antes truncava)
        assert!(encode_value(6, "256").is_err()); // > u8::MAX
        assert!(encode_value(6, "-1").is_err()); // negativo em Uint8
    }

    #[test]
    fn vlq_round_trip() {
        for v in [0i32, 1, 5, -1, 63, 64, -64, 127, 128, 8191, 8192, 1_000_000, -1_000_000] {
            let bytes = vlq_encode(v);
            let mut c = Cursor::new(&bytes);
            assert_eq!(c.vlq_i32().unwrap(), v, "vlq round-trip de {v}");
        }
    }

    #[test]
    fn string_round_trip() {
        for s in ["", "base\\test.ent", "Hello World", "café"] {
            let bytes = encode_lp_string(s);
            let mut c = Cursor::new(&bytes);
            assert_eq!(c.lp_string().unwrap(), s, "string round-trip de {s:?}");
        }
    }

    #[test]
    fn array_round_trip() {
        // array de Uint32 (idx 8): encode → split devolve os mesmos blobs.
        let elems = vec![
            encode_value(8, "1").unwrap(),
            encode_value(8, "2").unwrap(),
            encode_value(8, "300").unwrap(),
        ];
        let raw = encode_array(&elems);
        assert_eq!(split_array_elements(&raw, 8).unwrap(), elems);
        // array vazio.
        assert!(split_array_elements(&encode_array(&[]), 8).unwrap().is_empty());
    }

    #[test]
    fn id_de_nome_e_hex() {
        assert_eq!(
            encode_value(2, "$Items.Test").unwrap(),
            crate::hashes::tweak_db_id("Items.Test").to_le_bytes().to_vec()
        );
        assert_eq!(
            encode_value(2, "0x1122334455667788").unwrap(),
            0x1122_3344_5566_7788_u64.to_le_bytes().to_vec()
        );
    }
}
