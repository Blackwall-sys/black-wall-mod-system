//! Leitor do formato CR2W (o "record" interno de cada resource do .archive do Cyberpunk 2077).
//!
//! O `archive-tool` já extrai os bytes de cada resource (RDAR + Kraken); ESTE módulo lê a ESTRUTURA
//! CR2W desses bytes — o ENVELOPE (magic + FileHeader + as 10 tabelas: strings/names/imports/props/
//! chunks/buffers/embeds). É a fase 1 do porte WolvenKit (a "espinha"): destrava ler factory-index,
//! localization onscreens, appearance/garment nativamente em Rust, sem .NET.
//!
//! Layout (WolvenKit `CR2WHeaderStructs.cs` + `CR2WReader.File.cs`), little-endian:
//!   [0..4)   magic "CR2W"
//!   [4..40)  CR2WFileHeader (36B): version(u32) flags(u32) timeStamp(u64) buildVersion(u32)
//!            objectsEnd(u32) buffersEnd(u32) crc32(u32) numChunks(u32)
//!   [40..160) 10× CR2WTable (12B cada): offset(u32) itemCount(u32) crc32(u32)
//! version válida = 163..=195 (2.x = 195). VERIFICADO contra um record real (onscreens_final).

/// Cabeçalho do arquivo CR2W (após o magic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cr2wHeader {
    pub version: u32,
    pub flags: u32,
    pub timestamp: u64,
    pub build_version: u32,
    pub objects_end: u32,
    pub buffers_end: u32,
    pub crc32: u32,
    pub num_chunks: u32,
}

/// Uma das 10 tabelas do índice (strings, names, imports, properties, exports/chunks, buffers,
/// embeds…). `offset`/`item_count` = onde e quantos itens; a semântica por índice vem na fase 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cr2wTable {
    pub offset: u32,
    pub item_count: u32,
    pub crc32: u32,
}

/// Índice CR2W parseado: header + as 10 tabelas.
#[derive(Debug, Clone)]
pub struct Cr2wIndex {
    pub header: Cr2wHeader,
    pub tables: [Cr2wTable; 10],
}

pub const CR2W_MAGIC: &[u8; 4] = b"CR2W";
/// Tamanho do índice = magic(4) + FileHeader(36) + 10×CR2WTable(12) = 160.
pub const CR2W_INDEX_SIZE: usize = 4 + 36 + 10 * 12;

#[inline]
fn rd_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
#[inline]
fn rd_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        b[off], b[off + 1], b[off + 2], b[off + 3], b[off + 4], b[off + 5], b[off + 6], b[off + 7],
    ])
}

/// CRC-32 IEEE (= `Crc32Algorithm`/Force.Crc32 do WolvenKit) — delega à fonte única `bwms-hashes`.
pub use bwms_hashes::crc32;

/// Recomputa o crc32 do HEADER exatamente como o WolvenKit (`CalculateHeaderCRC32`): CRC sobre
/// MAGIC+version+flags+timeStamp+buildVersion+objectsEnd+buffersEnd+`0xDEADBEEF`(no lugar do campo
/// crc32)+numChunks + as 10 tabelas (offset+itemCount+crc32). Devolve o valor recomputado.
pub fn header_crc32(h: &Cr2wHeader, tables: &[Cr2wTable; 10]) -> u32 {
    let mut buf = Vec::with_capacity(160);
    buf.extend_from_slice(CR2W_MAGIC);
    buf.extend_from_slice(&h.version.to_le_bytes());
    buf.extend_from_slice(&h.flags.to_le_bytes());
    buf.extend_from_slice(&h.timestamp.to_le_bytes());
    buf.extend_from_slice(&h.build_version.to_le_bytes());
    buf.extend_from_slice(&h.objects_end.to_le_bytes());
    buf.extend_from_slice(&h.buffers_end.to_le_bytes());
    buf.extend_from_slice(&0xDEAD_BEEF_u32.to_le_bytes()); // placeholder do próprio campo crc32
    buf.extend_from_slice(&h.num_chunks.to_le_bytes());
    for t in tables {
        buf.extend_from_slice(&t.offset.to_le_bytes());
        buf.extend_from_slice(&t.item_count.to_le_bytes());
        buf.extend_from_slice(&t.crc32.to_le_bytes());
    }
    crc32(&buf)
}

/// Stride em bytes de cada item por índice de tabela (0 = a tabela mede `item_count` em BYTES, não itens):
/// t0=strings(blob), t1=names(4+4=8), t2=imports(4+2+2=8), t3=props(2+2+2+2+8=16), t4=exports(2+2+4·5=24),
/// t5=buffers(6·4=24), t6=embeds(4+4+8=16). t7-9 não usados. Fonte: WolvenKit.RED4 Sections/CR2W*.cs.
pub const TABLE_STRIDE: [usize; 10] = [0, 8, 8, 16, 24, 24, 16, 0, 0, 0];

/// Fatia crua de bytes da tabela `i` dentro do resource completo (para recomputar o crc32).
pub fn table_bytes<'a>(data: &'a [u8], t: &Cr2wTable, i: usize) -> &'a [u8] {
    let stride = TABLE_STRIDE[i];
    let byte_len = if stride == 0 { t.item_count as usize } else { t.item_count as usize * stride };
    let start = (t.offset as usize).min(data.len());
    let end = start.saturating_add(byte_len).min(data.len());
    &data[start..end]
}

/// Parseia o ÍNDICE CR2W (header + 10 tabelas) dos bytes de um resource. Valida magic + faixa de
/// versão. NÃO lê os conteúdos das tabelas (fase 2). Erro se muito curto / magic errado / versão fora.
pub fn parse_cr2w_index(data: &[u8]) -> Result<Cr2wIndex, String> {
    if data.len() < CR2W_INDEX_SIZE {
        return Err(format!("curto demais p/ CR2W: {} < {}", data.len(), CR2W_INDEX_SIZE));
    }
    if &data[0..4] != CR2W_MAGIC {
        return Err(format!("magic != CR2W (achado {:02x?})", &data[0..4]));
    }
    let header = Cr2wHeader {
        version: rd_u32(data, 4),
        flags: rd_u32(data, 8),
        timestamp: rd_u64(data, 12),
        build_version: rd_u32(data, 20),
        objects_end: rd_u32(data, 24),
        buffers_end: rd_u32(data, 28),
        crc32: rd_u32(data, 32),
        num_chunks: rd_u32(data, 36),
    };
    if !(163..=195).contains(&header.version) {
        return Err(format!("versão CR2W fora da faixa 163..=195: {}", header.version));
    }
    let mut tables = [Cr2wTable::default(); 10];
    for (i, t) in tables.iter_mut().enumerate() {
        let base = 40 + i * 12;
        *t = Cr2wTable {
            offset: rd_u32(data, base),
            item_count: rd_u32(data, base + 4),
            crc32: rd_u32(data, base + 8),
        };
    }
    Ok(Cr2wIndex { header, tables })
}

impl Cr2wIndex {
    /// Consistência estrutural contra o tamanho TOTAL do resource (quando disponível): toda tabela
    /// não-vazia tem offset dentro do arquivo, e `buffers_end` cabe. Não prova a semântica dos
    /// valores (isso é golden/fase 2), mas pega um parse torto. Devolve a lista de problemas (vazia = ok).
    pub fn structural_issues(&self, total_len: usize) -> Vec<String> {
        let mut v = Vec::new();
        if (self.header.buffers_end as usize) > total_len + 4096 {
            v.push(format!("buffers_end {} >> tamanho {}", self.header.buffers_end, total_len));
        }
        for (i, t) in self.tables.iter().enumerate() {
            if t.item_count > 0 && (t.offset as usize) < CR2W_INDEX_SIZE {
                v.push(format!("tabela {i}: offset {} dentro do header (<160)", t.offset));
            }
            if t.item_count > 0 && (t.offset as usize) > total_len {
                v.push(format!("tabela {i}: offset {} > tamanho {}", t.offset, total_len));
            }
        }
        v
    }
}

/// Lê o STRING DICT (tabela 0) — um blob de strings ASCII null-terminadas em
/// `[offset .. offset+item_count)`. Devolve `(offset_relativo -> string)`: names/imports/embeds
/// referenciam as strings por esse offset relativo ao início do blob (WolvenKit `ReadStringDict`).
/// O offset 0 costuma ser a string vazia (o blob começa com um `\0`).
pub fn read_string_dict(
    data: &[u8],
    table: &Cr2wTable,
) -> Result<std::collections::HashMap<u32, String>, String> {
    let start = table.offset as usize;
    let end = start
        .checked_add(table.item_count as usize)
        .ok_or("string dict: overflow")?;
    if end > data.len() {
        return Err(format!("string dict [{start}..{end}) fora do arquivo ({}B)", data.len()));
    }
    let blob = &data[start..end];
    let mut map = std::collections::HashMap::new();
    let mut i = 0usize;
    while i < blob.len() {
        let rel = i as u32;
        let s0 = i;
        while i < blob.len() && blob[i] != 0 {
            i += 1;
        }
        let s = std::str::from_utf8(&blob[s0..i]).map_err(|_| "string não-UTF8 no dict".to_string())?;
        map.insert(rel, s.to_string());
        i += 1; // pula o terminador
    }
    Ok(map)
}

/// Lê a tabela de NAMES (tabela 1): `item_count` entradas `CR2WNameInfo` de 8B cada
/// `{offset:u32 (no string dict), hash:u32}`. Resolve cada uma pra string via `dict`. Names são o
/// que os chunks referenciam como fieldName/redType. Devolve `[(hash, string)]` na ordem do arquivo.
pub fn read_names(
    data: &[u8],
    table: &Cr2wTable,
    dict: &std::collections::HashMap<u32, String>,
) -> Result<Vec<(u32, String)>, String> {
    let start = table.offset as usize;
    let n = table.item_count as usize;
    let end = start.checked_add(n * 8).ok_or("names: overflow")?;
    if end > data.len() {
        return Err(format!("names [{start}..{end}) fora do arquivo ({}B)", data.len()));
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let base = start + i * 8;
        let off = rd_u32(data, base);
        let hash = rd_u32(data, base + 4);
        let s = dict.get(&off).cloned().unwrap_or_default();
        out.push((hash, s));
    }
    Ok(out)
}

/// Uma entrada de IMPORT (tabela 2, dependência externa — outro resource): `depot_path` = caminho
/// pro `.ent`/`.mesh`/etc referenciado (resolvido via string dict, path pode ter `\`, por isso NÃO
/// é uma `name` curta); `class_name` = tipo esperado (via names); `flags` cru.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cr2wImport {
    pub depot_path: String,
    pub class_name: String,
    pub flags: u16,
}

/// Lê a tabela de IMPORTS (tabela 2): `item_count` entradas de 8B `{depotPathOffset:u32 (no string
/// dict), className:u16 (índice em names), flags:u16}`. É o que um `raRef:X`/`rRef:X` referencia por
/// índice (1-based, igual ao esquema handle: mas mirando aqui em vez das chunks/exports).
pub fn read_imports(
    data: &[u8],
    table: &Cr2wTable,
    dict: &std::collections::HashMap<u32, String>,
    names: &[(u32, String)],
) -> Result<Vec<Cr2wImport>, String> {
    let start = table.offset as usize;
    let n = table.item_count as usize;
    let end = start.checked_add(n * 8).ok_or("imports: overflow")?;
    if end > data.len() {
        return Err(format!("imports [{start}..{end}) fora do arquivo ({}B)", data.len()));
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let base = start + i * 8;
        let path_off = rd_u32(data, base);
        let class_idx = u16::from_le_bytes([data[base + 4], data[base + 5]]) as usize;
        let flags = u16::from_le_bytes([data[base + 6], data[base + 7]]);
        out.push(Cr2wImport {
            depot_path: dict.get(&path_off).cloned().unwrap_or_default(),
            class_name: names.get(class_idx).map(|(_, s)| s.clone()).unwrap_or_default(),
            flags,
        });
    }
    Ok(out)
}

/// Um CHUNK (export) do CR2W: o objeto raiz + os aninhados. `class_name` = tipo RED (resolvido via
/// names); `data_offset`/`data_size` apontam o payload do chunk no arquivo (as triplas de campos).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cr2wExport {
    pub class_name: String,
    pub parent_id: u32,
    pub data_offset: u32,
    pub data_size: u32,
}

/// Lê a tabela de EXPORTS/CHUNKS (tabela 4): `item_count` entradas `CR2WExportInfo` de 24B cada.
/// `className` é índice na tabela `names` (resolvido pra string). É onde estão os OBJETOS do resource
/// (ex.: `JsonResource` → `localizationPersistenceOnScreenDataResource`). data_offset/size = payload.
pub fn read_exports(
    data: &[u8],
    table: &Cr2wTable,
    names: &[(u32, String)],
) -> Result<Vec<Cr2wExport>, String> {
    let start = table.offset as usize;
    let n = table.item_count as usize;
    let end = start.checked_add(n * 24).ok_or("exports: overflow")?;
    if end > data.len() {
        return Err(format!("exports [{start}..{end}) fora do arquivo ({}B)", data.len()));
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = start + i * 24;
        let class_idx = u16::from_le_bytes([data[b], data[b + 1]]) as usize;
        let class_name = names.get(class_idx).map(|(_, s)| s.clone()).unwrap_or_default();
        out.push(Cr2wExport {
            class_name,
            parent_id: rd_u32(data, b + 4),
            data_size: rd_u32(data, b + 8),
            data_offset: rd_u32(data, b + 12),
        });
    }
    Ok(out)
}

/// Um CAMPO deserializado de um chunk: nome + tipo RED (resolvidos via names) + os bytes crus do
/// valor (o size-4 do descriptor). A interpretação tipada do valor (String/CName/Uint64/array…) é o
/// passo seguinte; aqui já temos a ESTRUTURA (quais campos, de que tipo, e o payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cr2wField {
    pub name: String,
    pub red_type: String,
    pub value: Vec<u8>,
}

/// Deserializa os CAMPOS do payload de um chunk (WolvenKit `CR2WReader.ReadClass`/`ReadVariable`):
/// byte líder `0`, depois um loop de `[varName:u16 (idx em names)][redType:u16][size:u32 (inclui 4)]
/// [valor: size-4 bytes]`, terminado por `varName == 0`. Não interpreta o valor por tipo (fase
/// seguinte) — devolve os bytes crus. VERIFICADO: o chunk `JsonResource` do onscreens dá exatamente
/// `cookingPlatform`(ECookingPlatform) + `root`(handle:ISerializable).
pub fn read_chunk_fields(
    data: &[u8],
    export: &Cr2wExport,
    names: &[(u32, String)],
) -> Result<(Vec<Cr2wField>, Vec<u8>), String> {
    let start = export.data_offset as usize;
    let end = start
        .checked_add(export.data_size as usize)
        .ok_or("chunk: overflow")?;
    if end > data.len() || start >= end {
        return Err(format!("chunk [{start}..{end}) fora do arquivo ({}B)", data.len()));
    }
    let mut pos = start;
    if data[pos] != 0 {
        return Err(format!("chunk não começa com o byte 0 (achado {:#x})", data[pos]));
    }
    pos += 1;
    let name_of = |i: usize| names.get(i).map(|(_, s)| s.clone()).unwrap_or_default();
    let mut fields = Vec::new();
    loop {
        if pos + 2 > end {
            break;
        }
        let name_idx = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        if name_idx == 0 {
            break; // terminador (CName vazio)
        }
        if pos + 6 > end {
            return Err("chunk: descriptor de campo truncado".into());
        }
        let type_idx = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        let size = rd_u32(data, pos);
        pos += 4;
        let val_len = (size as usize).checked_sub(4).ok_or("chunk: size < 4")?;
        if pos + val_len > end {
            return Err(format!("chunk: valor do campo '{}' estoura ({val_len}B)", name_of(name_idx)));
        }
        fields.push(Cr2wField {
            name: name_of(name_idx),
            red_type: name_of(type_idx),
            value: data[pos..pos + val_len].to_vec(),
        });
        pos += val_len;
    }
    // APPENDIX: bytes após o terminador até o fim do chunk (o `IRedAppendix` do WolvenKit — algumas
    // classes, ex. gameDeviceResourceData/streamingsector, põem dado extra aqui). Preserva byte-a-byte.
    let appendix = data[pos.min(end)..end].to_vec();
    Ok((fields, appendix))
}

/// VLQ signed int do CDPR (LEB128 modificado; WolvenKit `ReadVLQInt32`): 1º octeto bit7=sinal,
/// bit6=continuação, bits0-5=valor baixo; octetos seguintes bit7=continuação + 7 bits. Avança `pos`.
pub fn read_vlq_i32(data: &[u8], pos: &mut usize) -> Result<i32, String> {
    let rd = |p: &mut usize| -> Result<u8, String> {
        let b = *data.get(*p).ok_or("vlq: fim dos dados")?;
        *p += 1;
        Ok(b)
    };
    let b0 = rd(pos)?;
    let negative = b0 & 0x80 != 0;
    let mut value = (b0 & 0x3f) as i32;
    if b0 & 0x40 != 0 {
        let mut shift = 6;
        loop {
            let b = rd(pos)?;
            value |= ((b & 0x7f) as i32) << shift;
            shift += 7;
            if b & 0x80 == 0 || shift > 28 {
                break;
            }
        }
    }
    Ok(if negative { -value } else { value })
}

/// Lê uma String RED (WolvenKit `ReadLengthPrefixedString`): prefixo VLQ SIGNED = comprimento em
/// CHARS; sinal decide a largura — **prefixo > 0 = UTF-16LE** (len*2 bytes), **< 0 = UTF-8** (len
/// bytes), 0 = vazia. Avança `pos`. É como todo texto (localization, nomes) é guardado no CR2W.
pub fn read_red_string(data: &[u8], pos: &mut usize) -> Result<String, String> {
    let prefix = read_vlq_i32(data, pos)?;
    let len = prefix.unsigned_abs() as usize;
    if len == 0 {
        return Ok(String::new());
    }
    if prefix > 0 {
        let need = len * 2;
        let bytes = data.get(*pos..*pos + need).ok_or("string UTF-16 estoura")?;
        *pos += need;
        let u16s: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Ok(String::from_utf16_lossy(&u16s))
    } else {
        let bytes = data.get(*pos..*pos + len).ok_or("string UTF-8 estoura")?;
        *pos += len;
        Ok(String::from_utf8_lossy(bytes).to_string())
    }
}

/// Lê os campos de um ELEMENTO de array-de-classe. Cada elemento é lido como CLASSE (WolvenKit
/// `ReadClass`), então tem o MESMO byte líder `0` do chunk-raiz, seguido do loop
/// `[varName:u16][redType:u16][size:u32 incl.4][valor]` até varName=0. Avança `pos` pro próximo
/// elemento (consome o líder + os campos + o terminador u16=0).
pub fn read_element_fields(
    data: &[u8],
    pos: &mut usize,
    end: usize,
    names: &[(u32, String)],
) -> Result<Vec<Cr2wField>, String> {
    let name_of = |i: usize| names.get(i).map(|(_, s)| s.clone()).unwrap_or_default();
    // byte líder do elemento (o "joke da CDPR": um 0 antes dos campos).
    if *pos < end && data[*pos] == 0 {
        *pos += 1;
    }
    let mut fields = Vec::new();
    loop {
        if *pos + 2 > end {
            break;
        }
        let name_idx = u16::from_le_bytes([data[*pos], data[*pos + 1]]) as usize;
        *pos += 2;
        if name_idx == 0 {
            break;
        }
        if *pos + 6 > end {
            return Err("elemento: descriptor truncado".into());
        }
        let type_idx = u16::from_le_bytes([data[*pos], data[*pos + 1]]) as usize;
        *pos += 2;
        let size = rd_u32(data, *pos);
        *pos += 4;
        let val_len = (size as usize).checked_sub(4).ok_or("elemento: size < 4")?;
        if *pos + val_len > end {
            return Err("elemento: valor estoura".into());
        }
        fields.push(Cr2wField {
            name: name_of(name_idx),
            red_type: name_of(type_idx),
            value: data[*pos..*pos + val_len].to_vec(),
        });
        *pos += val_len;
    }
    Ok(fields)
}

/// Uma entrada de localização onscreen: a chave + os textos (feminino/masculino).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocEntry {
    pub primary_key: u64,
    pub secondary_key: String,
    pub female: String,
    pub male: String,
}

/// Extrai as entradas de localização do VALOR do campo `entries` (um `array:localizationPersistence
/// OnScreenEntry`): `[count:i32][elemento]*`, cada elemento = campos (sem leading-0). Decodifica
/// primaryKey(Uint64)/secondaryKey/femaleVariant/maleVariant(String RED). Lê no máx. `max` (0=todas).
/// Devolve (count_total, entradas). É o payoff do parser CR2W: ler a localização do jogo em Rust puro.
pub fn extract_localization(
    entries_value: &[u8],
    names: &[(u32, String)],
    max: usize,
) -> Result<(usize, Vec<LocEntry>), String> {
    if entries_value.len() < 4 {
        return Err("array de entries curto".into());
    }
    let count = i32::from_le_bytes([entries_value[0], entries_value[1], entries_value[2], entries_value[3]]) as usize;
    let end = entries_value.len();
    let mut pos = 4;
    let take = if max == 0 { count } else { count.min(max) };
    let mut out = Vec::with_capacity(take);
    for _ in 0..take {
        let fields = read_element_fields(entries_value, &mut pos, end, names)?;
        if fields.is_empty() {
            break;
        }
        let mut e = LocEntry::default();
        for f in &fields {
            match f.name.as_str() {
                // chave: `primaryKey` (onscreens, Uint64) OU `stringId` (subtitles, Uint64/Uint32).
                "primaryKey" | "stringId" if f.value.len() >= 8 => {
                    e.primary_key = u64::from_le_bytes(f.value[..8].try_into().unwrap());
                }
                "stringId" if f.value.len() >= 4 => {
                    e.primary_key = u32::from_le_bytes(f.value[..4].try_into().unwrap()) as u64;
                }
                "secondaryKey" => e.secondary_key = read_red_string(&f.value, &mut 0).unwrap_or_default(),
                "femaleVariant" => e.female = read_red_string(&f.value, &mut 0).unwrap_or_default(),
                "maleVariant" => e.male = read_red_string(&f.value, &mut 0).unwrap_or_default(),
                _ => {}
            }
        }
        // pula entradas SEM nenhum campo reconhecido (ex.: o índice SubtitleMap, que é
        // subtitleGroup→subtitleFile, não texto) — só entra o que tem chave ou texto de verdade.
        if e.primary_key != 0 || !e.secondary_key.is_empty() || !e.female.is_empty() || !e.male.is_empty() {
            out.push(e);
        }
    }
    Ok((count, out))
}

/// Edições de texto de localização por chave: `primaryKey`/`stringId` → (femaleVariant, maleVariant)
/// novos (`None` = não mexe naquele variant). Passado a `rebuild_entries_value`/`repack_localization_edit`.
pub type LocEdits = std::collections::HashMap<u64, (Option<String>, Option<String>)>;

/// Re-serializa o VALOR do campo `entries` (array `localizationPersistenceOnScreenEntry`) aplicando
/// `edits`. **Elementos NÃO editados são copiados byte-a-byte** (round-trip trivialmente exato — as
/// fatias consecutivas ladrilham o blob inteiro). Só o elemento editado é re-serializado, e ANTES de
/// aplicar a troca verifica que re-encodar os valores ORIGINAIS reproduz a fatia original (senão há
/// appendix/ambiguidade e recusa — disciplina "não chutar"). Sem edições, devolve o blob idêntico.
pub fn rebuild_entries_value(
    entries_value: &[u8],
    names: &[(u32, String)],
    idx_of: &impl Fn(&str) -> Option<u16>,
    edits: &LocEdits,
) -> Result<Vec<u8>, String> {
    if entries_value.len() < 4 {
        return Err("array de entries curto".into());
    }
    let count = i32::from_le_bytes(entries_value[..4].try_into().unwrap());
    let end = entries_value.len();
    let mut pos = 4;
    let mut out = Vec::with_capacity(entries_value.len());
    out.extend_from_slice(&count.to_le_bytes());
    for _ in 0..count.max(0) {
        if pos >= end {
            break;
        }
        let elem_start = pos;
        let fields = read_element_fields(entries_value, &mut pos, end, names)?;
        let orig = &entries_value[elem_start..pos];
        let key = fields
            .iter()
            .find(|f| f.name == "primaryKey" || f.name == "stringId")
            .and_then(|f| f.value.get(..8).map(|b| u64::from_le_bytes(b.try_into().unwrap())))
            .unwrap_or(0);
        match edits.get(&key) {
            Some((nf, nm)) if nf.is_some() || nm.is_some() => {
                // 1) prova de segurança: re-encodar os campos ORIGINAIS reproduz a fatia? (sem appendix)
                let re_orig = write_chunk_fields(&fields, &[], idx_of, true)?;
                if re_orig != orig {
                    return Err(format!(
                        "elemento key={key} não re-encoda byte-exato ({}B vs {}B) — tem appendix; edição recusada",
                        re_orig.len(), orig.len()
                    ));
                }
                // 2) aplica a troca de texto e re-encoda
                let mut ed = fields.clone();
                for f in ed.iter_mut() {
                    if f.name == "femaleVariant" {
                        if let Some(s) = nf {
                            f.value = write_red_string(s);
                        }
                    }
                    if f.name == "maleVariant" {
                        if let Some(s) = nm {
                            f.value = write_red_string(s);
                        }
                    }
                }
                out.extend_from_slice(&write_chunk_fields(&ed, &[], idx_of, true)?);
            }
            _ => out.extend_from_slice(orig), // inalterado: byte-a-byte
        }
    }
    if pos < end {
        out.extend_from_slice(&entries_value[pos..end]); // cauda/padding do array
    }
    Ok(out)
}

/// Uma entrada de localização NOVA a adicionar: chave + textos. `secondary_key` pode ser vazio.
pub struct LocAdd {
    pub primary_key: u64,
    pub secondary_key: String,
    pub female: String,
    pub male: String,
}

/// ADICIONA entradas novas ao array `entries` (append + count++), espelhando o schema (nomes/tipos/
/// ordem dos campos) do PRIMEIRO elemento existente — robusto a onscreens (primaryKey) e subtitles
/// (stringId). Cada elemento novo é re-serializado via `write_chunk_fields` (leader=true). Re-empacota
/// (recomputa offsets/crc). Completa o CRUD de localização (ler+editar+adicionar). Chaves já existentes
/// são RECUSADAS (use edição). Devolve o CR2W novo.
pub fn repack_localization_add(data: &[u8], adds: &[LocAdd]) -> Result<Vec<u8>, String> {
    if adds.is_empty() {
        return Err("nada a adicionar".into());
    }
    let idx = parse_cr2w_index(data)?;
    let dict = read_string_dict(data, &idx.tables[0])?;
    let names = read_names(data, &idx.tables[1], &dict)?;
    let exports = read_exports(data, &idx.tables[4], &names)?;
    let idx_of = |s: &str| names.iter().position(|(_, n)| n == s).map(|i| i as u16);

    for (ci, e) in exports.iter().enumerate() {
        let (mut fields, appendix) = read_chunk_fields(data, e, &names)?;
        if let Some(fi) = fields.iter().position(|f| f.name == "entries") {
            let ev = &fields[fi].value;
            if ev.len() < 4 {
                return Err("array de entries curto".into());
            }
            let mut count = i32::from_le_bytes(ev[..4].try_into().unwrap());
            // template = campos do 1º elemento (nomes/tipos/ordem reais deste arquivo).
            let mut tpos = 4usize;
            let template = read_element_fields(ev, &mut tpos, ev.len(), &names)?;
            if template.is_empty() {
                return Err("não há elemento-modelo p/ inferir o schema".into());
            }
            let key_field = template
                .iter()
                .find(|f| f.name == "primaryKey" || f.name == "stringId")
                .map(|f| (f.name.clone(), f.value.len()))
                .ok_or("modelo sem primaryKey/stringId")?;
            // chaves existentes (p/ recusar duplicata).
            let (_t, existing) = extract_localization(ev, &names, 0)?;
            let have: std::collections::HashSet<u64> = existing.iter().map(|e| e.primary_key).collect();

            let mut appended: Vec<u8> = Vec::new();
            for a in adds {
                if have.contains(&a.primary_key) {
                    return Err(format!("chave {} já existe — use edição, não adição", a.primary_key));
                }
                // monta os campos do novo elemento espelhando o template, trocando os valores.
                let mut ef: Vec<Cr2wField> = Vec::with_capacity(template.len());
                for t in &template {
                    let value = match t.name.as_str() {
                        "primaryKey" | "stringId" => {
                            // respeita a largura do modelo (Uint64=8, Uint32=4).
                            if key_field.1 >= 8 {
                                a.primary_key.to_le_bytes().to_vec()
                            } else {
                                (a.primary_key as u32).to_le_bytes().to_vec()
                            }
                        }
                        "secondaryKey" => write_red_string(&a.secondary_key),
                        "femaleVariant" => write_red_string(&a.female),
                        "maleVariant" => write_red_string(&a.male),
                        _ => t.value.clone(), // campo desconhecido: mantém o do modelo
                    };
                    ef.push(Cr2wField { name: t.name.clone(), red_type: t.red_type.clone(), value });
                }
                appended.extend_from_slice(&write_chunk_fields(&ef, &[], &idx_of, true)?);
                count += 1;
            }

            // novo valor do campo = count atualizado + bytes originais dos elementos + os novos.
            let mut new_val = Vec::with_capacity(ev.len() + appended.len());
            new_val.extend_from_slice(&count.to_le_bytes());
            new_val.extend_from_slice(&ev[4..]);
            new_val.extend_from_slice(&appended);
            fields[fi].value = new_val;
            let new_chunk = write_chunk_fields(&fields, &appendix, &idx_of, true)?;
            return repack_replace_chunk(data, ci, &new_chunk);
        }
    }
    Err("nenhum chunk com campo `entries`".into())
}

/// Edita a localização de um resource onscreens/subtitles inteiro e re-empacota: acha o chunk com o
/// campo `entries`, reconstrói o valor via `rebuild_entries_value`, re-serializa o chunk e chama
/// `repack_replace_chunk` (recomputa offsets/crc). Sem edições → arquivo byte-idêntico. `idx_of` é
/// posicional (mesma via do writer já provado byte-exato). Devolve o CR2W novo pronto pra empacotar.
pub fn repack_localization_edit(data: &[u8], edits: &LocEdits) -> Result<Vec<u8>, String> {
    let idx = parse_cr2w_index(data)?;
    let dict = read_string_dict(data, &idx.tables[0])?;
    let names = read_names(data, &idx.tables[1], &dict)?;
    let exports = read_exports(data, &idx.tables[4], &names)?;
    let idx_of = |s: &str| names.iter().position(|(_, n)| n == s).map(|i| i as u16);
    for (ci, e) in exports.iter().enumerate() {
        let (mut fields, appendix) = read_chunk_fields(data, e, &names)?;
        if let Some(fi) = fields.iter().position(|f| f.name == "entries") {
            fields[fi].value = rebuild_entries_value(&fields[fi].value, &names, &idx_of, edits)?;
            let new_chunk = write_chunk_fields(&fields, &appendix, &idx_of, true)?;
            return repack_replace_chunk(data, ci, &new_chunk);
        }
    }
    Err("nenhum chunk com campo `entries` (não parece localização onscreens/subtitles)".into())
}

/// Valida que cada linha nova tem EXATAMENTE `cols_len` células (= nº de colunas do header do
/// C2dArray). Autoritativo mesmo quando o factory está vazio (0 linhas) — o bug antigo lia a
/// largura da 1ª linha e, sem linhas, pulava a checagem, deixando corromper o .csv.
pub fn check_row_widths(cols_len: usize, new_rows: &[Vec<String>]) -> Result<(), String> {
    for (k, nr) in new_rows.iter().enumerate() {
        if nr.len() != cols_len {
            return Err(format!(
                "linha nova {k} tem {} células, esperado {cols_len} (colunas do header)",
                nr.len()
            ));
        }
    }
    Ok(())
}

/// ADICIONA linhas a um `C2dArray` (factory/stat .csv) e re-empacota: acha o chunk `C2dArray`, lê
/// headers+data, anexa `new_rows` (cada uma = células), re-serializa e chama `repack_replace_chunk`
/// (recomputa offset/crc). Cada nova linha deve ter o MESMO nº de células das existentes (ou 1 célula
/// comma-joined, conforme a forma do arquivo). Adiciona item ao factory = adicionar `[name,path,preload]`.
pub fn repack_c2d_add(data: &[u8], new_rows: &[Vec<String>]) -> Result<Vec<u8>, String> {
    if new_rows.is_empty() {
        return Err("nada a adicionar".into());
    }
    let idx = parse_cr2w_index(data)?;
    let dict = read_string_dict(data, &idx.tables[0])?;
    let names = read_names(data, &idx.tables[1], &dict)?;
    let exports = read_exports(data, &idx.tables[4], &names)?;
    let idx_of = |s: &str| names.iter().position(|(_, n)| n == s).map(|i| i as u16);

    for (ci, e) in exports.iter().enumerate() {
        if e.class_name != "C2dArray" {
            continue;
        }
        let (mut fields, appendix) = read_chunk_fields(data, e, &names)?;
        let hv = fields.iter().find(|f| f.name == "headers").map(|f| f.value.clone());
        let di = fields.iter().position(|f| f.name == "data");
        if let (Some(hv), Some(di)) = (hv, di) {
            let (cols, mut rows) = read_c2d_array(&hv, &fields[di].value)?;
            // valida a largura das novas linhas contra o HEADER (cols.len()), não contra a 1ª
            // linha: um factory de 0 linhas tem `rows.first()==None` e aceitava qualquer largura,
            // corrompendo o .csv. O nº de colunas é o schema autoritativo (não-chute).
            check_row_widths(cols.len(), new_rows)?;
            for nr in new_rows {
                rows.push(nr.clone());
            }
            let (_rhv, rdv) = write_c2d_array(&cols, &rows);
            fields[di].value = rdv;
            let new_chunk = write_chunk_fields(&fields, &appendix, &idx_of, true)?;
            return repack_replace_chunk(data, ci, &new_chunk);
        }
    }
    Err("nenhum chunk C2dArray com headers+data".into())
}

/// Lê um `C2dArray` (o formato das `.csv` do jogo — factories, stats, etc.): `headers`
/// (`array:String` = `[count:i32][RED String]*`) + `data` (`array:array:String` = `[rows:i32]
/// [ [cells:i32][RED String]* ]*`). Elementos de `array:String` são RED Strings DIRETAS (sem leading-0
/// de classe). Devolve (colunas, linhas). Base p/ ler/editar factory (ArchiveXL adicionar item) em Rust.
pub fn read_c2d_array(headers_value: &[u8], data_value: &[u8]) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let read_str_array = |v: &[u8], pos: &mut usize| -> Result<Vec<String>, String> {
        if *pos + 4 > v.len() {
            return Err("array:String curto".into());
        }
        let n = i32::from_le_bytes(v[*pos..*pos + 4].try_into().unwrap());
        *pos += 4;
        let mut out = Vec::with_capacity(n.max(0) as usize);
        for _ in 0..n.max(0) {
            out.push(read_red_string(v, pos)?);
        }
        Ok(out)
    };
    let mut hp = 0usize;
    let headers = read_str_array(headers_value, &mut hp)?;
    // data = array de linhas; cada linha = array:String.
    let mut dp = 0usize;
    if data_value.len() < 4 {
        return Err("data curto".into());
    }
    let rows_n = i32::from_le_bytes(data_value[..4].try_into().unwrap());
    dp = 4;
    let mut rows = Vec::with_capacity(rows_n.max(0) as usize);
    for _ in 0..rows_n.max(0) {
        rows.push(read_str_array(data_value, &mut dp)?);
    }
    let _ = &mut dp;
    Ok((headers, rows))
}

/// Serializa um `C2dArray` de volta pros valores dos campos `headers` + `data` (inverso de
/// `read_c2d_array`). Round-trip byte-exato prova a interpretação (não-chute). Base p/ EDITAR/ADICIONAR
/// linha de factory (item novo) = append de linha + re-encode + `repack_replace_chunk`.
pub fn write_c2d_array(headers: &[String], rows: &[Vec<String>]) -> (Vec<u8>, Vec<u8>) {
    let write_str_array = |arr: &[String]| -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(arr.len() as i32).to_le_bytes());
        for s in arr {
            out.extend_from_slice(&write_red_string(s));
        }
        out
    };
    let hv = write_str_array(headers);
    let mut dv = Vec::new();
    dv.extend_from_slice(&(rows.len() as i32).to_le_bytes());
    for r in rows {
        dv.extend_from_slice(&write_str_array(r));
    }
    (hv, dv)
}

/// Serializa o ÍNDICE CR2W (inverso de `parse_cr2w_index`): magic "CR2W" + FileHeader (36B) + as 10
/// tabelas (12B cada) = 160B. Round-trip byte-exato do envelope. Junto com `write_chunk_fields` é a
/// base do re-pack (falta só a montagem final: recomputar offsets/sizes/crc32 two-pass ao editar).
pub fn write_cr2w_index(idx: &Cr2wIndex) -> Vec<u8> {
    let mut out = Vec::with_capacity(CR2W_INDEX_SIZE);
    out.extend_from_slice(CR2W_MAGIC);
    let h = &idx.header;
    out.extend_from_slice(&h.version.to_le_bytes());
    out.extend_from_slice(&h.flags.to_le_bytes());
    out.extend_from_slice(&h.timestamp.to_le_bytes());
    out.extend_from_slice(&h.build_version.to_le_bytes());
    out.extend_from_slice(&h.objects_end.to_le_bytes());
    out.extend_from_slice(&h.buffers_end.to_le_bytes());
    out.extend_from_slice(&h.crc32.to_le_bytes());
    out.extend_from_slice(&h.num_chunks.to_le_bytes());
    for t in &idx.tables {
        out.extend_from_slice(&t.offset.to_le_bytes());
        out.extend_from_slice(&t.item_count.to_le_bytes());
        out.extend_from_slice(&t.crc32.to_le_bytes());
    }
    out
}

/// Reconstrói um CR2W trocando os bytes de UM chunk (índice na tabela de exports/chunks). Faz splice
/// byte-a-byte (preserva padding/appendix/buffers) e recomputa TUDO que a mudança de tamanho afeta:
/// `dataSize` do chunk-alvo, `dataOffset` dos chunks posteriores, `offset` dos buffers posteriores,
/// `objectsEnd`/`buffersEnd` do header e os crc32 das tabelas exports(4)+buffers(5)+header.
///
/// **Round-trip = a prova offline**: se `new_bytes` forem os bytes ORIGINAIS do chunk (delta 0), a
/// saída é BYTE-IDÊNTICA à entrada — validando a recomputação de offset/crc SEM ligar o jogo. É o
/// núcleo da edição de localização (trocar texto no chunk-raiz do onscreens/subtitles e re-empacotar).
pub fn repack_replace_chunk(data: &[u8], chunk_index: usize, new_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let idx = parse_cr2w_index(data)?;
    let dict = read_string_dict(data, &idx.tables[0])?;
    let names = read_names(data, &idx.tables[1], &dict)?;
    let exports = read_exports(data, &idx.tables[4], &names)?;
    let tgt = exports.get(chunk_index).ok_or("chunk_index fora de faixa")?;
    let old_off = tgt.data_offset as usize;
    let old_size = tgt.data_size as usize;
    if old_off.checked_add(old_size).map_or(true, |e| e > data.len()) {
        return Err(format!("chunk [{old_off}..+{old_size}) fora do arquivo ({}B)", data.len()));
    }
    let delta: i64 = new_bytes.len() as i64 - old_size as i64;

    // 1) splice no nível de bytes — tudo antes do chunk fica intacto (o índice/tabelas moram lá).
    let mut out = Vec::with_capacity((data.len() as i64 + delta).max(0) as usize);
    out.extend_from_slice(&data[..old_off]);
    out.extend_from_slice(new_bytes);
    out.extend_from_slice(&data[old_off + old_size..]);

    // 2) tabela EXPORTS (24B/entry: dataSize@+8, dataOffset@+12). A tabela está antes de old_off,
    //    então a posição dela em `out` == em `data`. Chunks posteriores (offset > alvo) deslocam +delta.
    let et = idx.tables[4];
    for (i, e) in exports.iter().enumerate() {
        let b = et.offset as usize + i * 24;
        let nsize = e.data_size as i64 + if i == chunk_index { delta } else { 0 };
        let noff = e.data_offset as i64 + if (e.data_offset as usize) > old_off { delta } else { 0 };
        out[b + 8..b + 12].copy_from_slice(&(nsize as u32).to_le_bytes());
        out[b + 12..b + 16].copy_from_slice(&(noff as u32).to_le_bytes());
    }

    // 3) tabela BUFFERS (24B/entry: offset@+8). Os buffers ficam DEPOIS do chunk data → deslocam +delta.
    let bt = idx.tables[5];
    for i in 0..bt.item_count as usize {
        let b = bt.offset as usize + i * 24;
        let boff = rd_u32(data, b + 8) as i64;
        if boff as usize > old_off {
            out[b + 8..b + 12].copy_from_slice(&((boff + delta) as u32).to_le_bytes());
        }
    }

    // 4) recomputa crc32 das tabelas patcheadas + header e reescreve o índice (160B).
    let mut nidx = idx.clone();
    nidx.tables[4].crc32 = crc32(table_bytes(&out, &nidx.tables[4], 4));
    nidx.tables[5].crc32 = crc32(table_bytes(&out, &nidx.tables[5], 5));
    nidx.header.objects_end = (idx.header.objects_end as i64 + delta) as u32;
    nidx.header.buffers_end = (idx.header.buffers_end as i64 + delta) as u32;
    nidx.header.crc32 = header_crc32(&nidx.header, &nidx.tables);
    out[..CR2W_INDEX_SIZE].copy_from_slice(&write_cr2w_index(&nidx));
    Ok(out)
}

/// Serializa os campos de um chunk-raiz de volta pro formato CR2W (inverso de `read_chunk_fields`):
/// byte líder `0`, depois `[varName:u16][redType:u16][size:u32 = valor+4][valor]` por campo, e o
/// terminador `u16 0`. `idx_of` resolve um nome→índice na tabela de names. É o fundamento do WRITER
/// (round-trip byte-exato) e da edição de localização. `leader` = true no chunk-raiz e nos elementos
/// de array (ambos têm o líder 0).
pub fn write_chunk_fields(
    fields: &[Cr2wField],
    appendix: &[u8],
    idx_of: &impl Fn(&str) -> Option<u16>,
    leader: bool,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    if leader {
        out.push(0u8);
    }
    for f in fields {
        let ni = idx_of(&f.name).ok_or_else(|| format!("name '{}' não está na tabela", f.name))?;
        let ti = idx_of(&f.red_type).ok_or_else(|| format!("redType '{}' não está na tabela", f.red_type))?;
        out.extend_from_slice(&ni.to_le_bytes());
        out.extend_from_slice(&ti.to_le_bytes());
        out.extend_from_slice(&((f.value.len() as u32) + 4).to_le_bytes());
        out.extend_from_slice(&f.value);
    }
    out.extend_from_slice(&0u16.to_le_bytes()); // terminador (CName vazio)
    out.extend_from_slice(appendix); // dado de IRedAppendix (preservado byte-a-byte)
    Ok(out)
}

/// Codifica uma String RED (inverso de `read_red_string`): escolhe UTF-8 (prefixo VLQ negativo) se o
/// texto for ASCII/latin1, senão UTF-16LE (prefixo positivo). Prefixo = VLQ signed do comprimento em
/// chars. Base p/ EDITAR localização (trocar o texto e re-serializar). Testado (round-trip);
/// wiring da edição/re-pack é o próximo passo do writer.
#[allow(dead_code)]
pub fn write_red_string(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    if s.is_empty() {
        out.push(0);
        return out;
    }
    let ascii = s.is_ascii();
    let char_len = if ascii { s.len() } else { s.chars().count() };
    let prefix = if ascii { -(char_len as i32) } else { char_len as i32 };
    write_vlq_i32(prefix, &mut out);
    if ascii {
        out.extend_from_slice(s.as_bytes());
    } else {
        for u in s.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
    out
}

/// Escreve um VLQ signed no formato CDPR (inverso de `read_vlq_i32`).
pub fn write_vlq_i32(value: i32, out: &mut Vec<u8>) {
    let negative = value < 0;
    let mut mag = value.unsigned_abs();
    // 1º octeto: bit7=sinal, bit6=continuação, bits0-5 = 6 bits baixos.
    let mut b = (mag & 0x3f) as u8;
    if negative {
        b |= 0x80;
    }
    mag >>= 6;
    if mag > 0 {
        b |= 0x40; // continua
    }
    out.push(b);
    // octetos seguintes: LEB128 padrão (bit7=continuação, 7 bits de valor).
    while mag > 0 {
        let mut b = (mag & 0x7f) as u8;
        mag >>= 7;
        if mag > 0 {
            b |= 0x80;
        }
        out.push(b);
    }
}

/// Decodifica UM valor de tipo `t` a partir de `data[*pos..]`, avançando `*pos` pelo tamanho
/// consumido. Cobre os primitivos (fixos) + `handle:`/`whandle:`/`raRef:`/`rRef:` (refs fixas) +
/// `array:X` (recursivo: `[count:u32]` + X repetido) + qualquer tipo desconhecido tratado como
/// CLASSE ANINHADA (mesmo esquema líder-0 + loop nome/tipo/tamanho de `read_element_fields`,
/// decodificado campo-a-campo recursivamente). É o que fecha o RE dos tipos ricos de appearance/
/// entity (`appearanceAppearancePart`, `...PartOverrides`, `appearancePartComponentOverrides`,
/// `redTagList`) sem precisar de um caso especial por struct — descoberto e validado em
/// `t0_000_*_fpp__full.app` vs `t0_000_base__full.app` (2026-07-15).
fn decode_typed_at(t: &str, data: &[u8], pos: &mut usize, names: &[(u32, String)]) -> String {
    let name_at = |i: usize| names.get(i).map(|(_, s)| s.as_str()).unwrap_or("?").to_string();
    // Sentinela de falha: nunca bate com `v.len()` real (arquivos não chegam a usize::MAX bytes),
    // então o gate em `decode_field_value` (`pos == v.len()`) rejeita corretamente qualquer tentativa
    // que truncou ou não achou a estrutura esperada — sem isso um erro "por sorte" no tamanho certo
    // passaria como decodificação válida (quebraria a heurística de enum pra tipos não-classe curtos).
    macro_rules! take {
        ($n:expr) => {{
            let end = *pos + $n;
            if end > data.len() {
                *pos = usize::MAX;
                return format!("<trunc:{t}>");
            }
            let s = &data[*pos..end];
            *pos = end;
            s
        }};
    }
    match t {
        "CName" => {
            let b = take!(2);
            format!("CName({})", name_at(u16::from_le_bytes([b[0], b[1]]) as usize))
        }
        "Uint64" => u64::from_le_bytes(take!(8).try_into().unwrap()).to_string(),
        "Int64" => i64::from_le_bytes(take!(8).try_into().unwrap()).to_string(),
        "Uint32" => u32::from_le_bytes(take!(4).try_into().unwrap()).to_string(),
        "Int32" => i32::from_le_bytes(take!(4).try_into().unwrap()).to_string(),
        "Uint16" => u16::from_le_bytes(take!(2).try_into().unwrap()).to_string(),
        "Uint8" => take!(1)[0].to_string(),
        "Float" => f32::from_le_bytes(take!(4).try_into().unwrap()).to_string(),
        "Bool" => (take!(1)[0] != 0).to_string(),
        _ if t.starts_with("handle:") || t.starts_with("whandle:") => {
            let r = u32::from_le_bytes(take!(4).try_into().unwrap());
            if r == 0 { "null".into() } else { format!("→chunk #{}", r - 1) }
        }
        // raRef/rRef: índice 1-based na tabela de IMPORTS (dependência externa, ex. um .ent/.mesh).
        _ if t.starts_with("raRef:") || t.starts_with("rRef:") => {
            let r = u16::from_le_bytes(take!(2).try_into().unwrap());
            if r == 0 { "null".into() } else { format!("→import #{r}") }
        }
        _ if t.starts_with("array:") => {
            let inner = &t[6..];
            let n = u32::from_le_bytes(take!(4).try_into().unwrap()) as usize;
            let items: Vec<String> = (0..n).map(|_| decode_typed_at(inner, data, pos, names)).collect();
            format!("[{n}] {}", items.join(", "))
        }
        "String" | "CString" => read_red_string(data, pos).unwrap_or_else(|e| format!("<str: {e}>")),
        // tipo desconhecido = CLASSE aninhada (appearanceAppearancePart, redTagList, …): líder-0 +
        // loop de campos, cada um decodificado recursivamente por seu próprio red_type.
        _ => match read_element_fields(data, pos, data.len(), names) {
            Ok(fields) => {
                let parts: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        let mut p = 0;
                        format!("{}={}", f.name, decode_typed_at(&f.red_type, &f.value, &mut p, names))
                    })
                    .collect();
                format!("{{{}}}", parts.join(", "))
            }
            Err(e) => {
                *pos = usize::MAX;
                format!("<classe '{t}': {e}>")
            }
        },
    }
}

/// Decodifica o VALOR de um campo pro tipo RED declarado (`decode_typed_at` na raiz). Cobre desde os
/// primitivos simples até arrays de structs aninhados (appearance/entity) recursivamente. Fallback:
/// heurística de enum (2 bytes = índice em names) e, por fim, hex cru do que sobrar sem decodificar.
pub fn decode_field_value(f: &Cr2wField, names: &[(u32, String)]) -> String {
    let v = &f.value;
    let t = f.red_type.as_str();
    // Tenta SEMPRE via decode_typed_at (primitivos/array/handle/raRef/String pegam a interpretação
    // exata; tipo desconhecido cai no caminho "classe aninhada") — só aceita se consumir TODO o
    // valor (senão o tipo não bate com esse esquema, cai no heurístico/hex abaixo).
    if !v.is_empty() {
        let mut pos = 0;
        let s = decode_typed_at(t, v, &mut pos, names);
        if pos == v.len() {
            return s;
        }
    }
    let name_at = |i: usize| names.get(i).map(|(_, s)| s.as_str()).unwrap_or("?");
    // enum: o valor é um u16 índice em names (nome do membro). Heurística: 2 bytes + tipo não-primitivo.
    if v.len() == 2 {
        return name_at(u16::from_le_bytes([v[0], v[1]]) as usize).to_string();
    }
    let hex: String = v.iter().map(|b| format!("{b:02x}")).collect();
    format!("<{}B {hex}>", v.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Golden: os 160 bytes do header de um record CR2W REAL — `base/localization/en-us/onscreens/
    // onscreens_final.json` (extraído de lang_en_text.archive; tamanho total 8_784_924).
    const GOLDEN: [u8; 160] = [
        0x43, 0x52, 0x32, 0x57, 0xc3, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1c, 0x0c, 0x86, 0x00, 0x1c, 0x0c,
        0x86, 0x00, 0x12, 0x15, 0xd9, 0x60, 0x06, 0x00, 0x00, 0x00, 0xa0, 0x00, 0x00, 0x00, 0xf0,
        0x00, 0x00, 0x00, 0x31, 0xe3, 0x0c, 0x42, 0x90, 0x01, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00,
        0x02, 0x99, 0x18, 0x65, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x10, 0x02, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x55, 0x4b, 0xbb, 0xec, 0x20, 0x02,
        0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x81, 0xf6, 0x63, 0x2d, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn c2d_array_round_trip_sintetico() {
        // headers + rows arbitrários → write → read → igual (prova a interpretação do C2dArray).
        let cols = vec!["name".to_string(), "path".to_string(), "preload".to_string()];
        let rows = vec![
            vec!["carro_a".to_string(), "base\\x.ent".to_string(), "TRUE".to_string()],
            vec!["arma_b".to_string(), "base\\y.ent".to_string(), "FALSE".to_string()],
            vec!["".to_string(), "".to_string(), "".to_string()], // linha vazia (edge)
        ];
        let (hv, dv) = write_c2d_array(&cols, &rows);
        let (rcols, rrows) = read_c2d_array(&hv, &dv).unwrap();
        assert_eq!(rcols, cols);
        assert_eq!(rrows, rows);
        // e a forma "1 célula comma-joined" (a outra variante real, ex.: vehicles.csv).
        let cols1 = vec!["name,path,preload".to_string()];
        let rows1 = vec![vec!["a,b,c".to_string()], vec!["d,e,f".to_string()]];
        let (hv1, dv1) = write_c2d_array(&cols1, &rows1);
        let (rc1, rr1) = read_c2d_array(&hv1, &dv1).unwrap();
        assert_eq!(rc1, cols1);
        assert_eq!(rr1, rows1);
    }

    #[test]
    fn factory_vazio_round_trip_e_bytes() {
        // Um factory de 0 linhas (só headers) codifica `data` como só o contador rows_n=0 (4 bytes
        // LE), e volta como (cols, []). Pin do formato de fio p/ o caso que o bug corrompia.
        let cols = vec!["name".to_string(), "path".to_string(), "preload".to_string()];
        let rows: Vec<Vec<String>> = vec![];
        let (hv, dv) = write_c2d_array(&cols, &rows);
        assert_eq!(dv, 0i32.to_le_bytes(), "data de factory vazio = [rows_n=0]");
        let (rcols, rrows) = read_c2d_array(&hv, &dv).unwrap();
        assert_eq!(rcols, cols);
        assert!(rrows.is_empty());
    }

    #[test]
    fn check_row_widths_valida_contra_header() {
        // 3 colunas: linha de 3 células passa; de 2 ou 4 falha.
        assert!(check_row_widths(3, &[vec!["a".into(), "b".into(), "c".into()]]).is_ok());
        assert!(check_row_widths(3, &[vec!["a".into(), "b".into()]]).is_err());
        assert!(check_row_widths(3, &[vec!["a".into(), "b".into(), "c".into(), "d".into()]]).is_err());
        // nenhuma linha nova = ok trivial.
        assert!(check_row_widths(3, &[]).is_ok());
    }

    #[test]
    fn factory_vazio_rejeita_largura_errada() {
        // O CERNE do fix: um factory de 0 linhas (cols.len()=3) NÃO pode aceitar uma linha de
        // largura diferente. Antes, `rows.first()==None` pulava a checagem e corrompia o .csv.
        let cols_len = 3usize; // header com 3 colunas, 0 linhas de dados
        let wrong = vec![vec!["só".into(), "duas".into()]];
        assert!(
            check_row_widths(cols_len, &wrong).is_err(),
            "factory vazio tem que rejeitar largura errada (regressão do bug 706)"
        );
        // e a largura certa passa.
        let ok = vec![vec!["a".into(), "b".into(), "c".into()]];
        assert!(check_row_widths(cols_len, &ok).is_ok());
    }

    #[test]
    fn red_string_round_trip_utf8_e_utf16() {
        // ASCII → UTF-8 (prefixo VLQ negativo); acento → UTF-16 (positivo). Round-trip exato.
        for s in ["News", "Notícias", "Psicocibernético à solta!", "", "a,b,c\\d"] {
            let enc = write_red_string(s);
            let mut pos = 0usize;
            let dec = read_red_string(&enc, &mut pos).unwrap();
            assert_eq!(dec, s, "round-trip falhou p/ {s:?}");
            assert_eq!(pos, enc.len(), "não consumiu tudo p/ {s:?}");
        }
    }

    #[test]
    fn crc32_vetor_padrao_ieee() {
        // KAT do CRC-32 IEEE: a string "123456789" tem check value 0xCBF43926 (o mesmo que o
        // Crc32Algorithm/Force.Crc32 do WolvenKit produz). Trava o algoritmo p/ sempre.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(&[0u8; 4]), crc32(&[0u8; 4])); // determinístico
    }

    #[test]
    fn header_crc32_bate_com_golden_real() {
        // O header golden (onscreens_final real) guarda crc32 = 0x60D91512 (bytes 32..36 = 12 15 d9 60).
        // header_crc32 tem que recomputar exatamente esse valor (0xDEADBEEF no lugar do campo).
        let idx = parse_cr2w_index(&GOLDEN).unwrap();
        assert_eq!(idx.header.crc32, 0x60D9_1512);
        assert_eq!(header_crc32(&idx.header, &idx.tables), idx.header.crc32);
    }

    #[test]
    fn strides_das_tabelas_corretos() {
        // Tamanhos das structs CR2W* da RED4 (WolvenKit Sections/): names=8, imports=8, props=16,
        // exports=24, buffers=24, embeds=16. Um erro aqui quebra o crc32 por-tabela.
        assert_eq!(TABLE_STRIDE, [0, 8, 8, 16, 24, 24, 16, 0, 0, 0]);
    }

    #[test]
    fn parseia_header_real() {
        let idx = parse_cr2w_index(&GOLDEN).unwrap();
        assert_eq!(idx.header.version, 195); // CP2077 2.x
        assert_eq!(idx.header.flags, 0);
        assert_eq!(idx.header.num_chunks, 6);
        assert_eq!(idx.header.objects_end, 0x0086_0c1c);
        assert_eq!(idx.header.buffers_end, 0x0086_0c1c); // ~= tamanho do arquivo
    }

    #[test]
    fn parseia_tabelas_reais() {
        let idx = parse_cr2w_index(&GOLDEN).unwrap();
        // tabela 0 = strings: offset 160 (= logo após o índice de 160B), 240 itens.
        assert_eq!(idx.tables[0], Cr2wTable { offset: 160, item_count: 240, crc32: 0x420c_e331 });
        // tabela 1: offset 400, 16 itens.
        assert_eq!(idx.tables[1].offset, 400);
        assert_eq!(idx.tables[1].item_count, 16);
        // tabelas 5..10 vazias.
        for t in &idx.tables[5..] {
            assert_eq!(t.item_count, 0);
        }
    }

    #[test]
    fn consistencia_estrutural_ok() {
        let idx = parse_cr2w_index(&GOLDEN).unwrap();
        // com o tamanho real do arquivo, os offsets das tabelas cabem.
        assert!(idx.structural_issues(8_784_924).is_empty());
    }

    #[test]
    fn le_string_dict_real() {
        // conteúdo REAL do string dict de onscreens_final (as strings, unidas por \0 como no arquivo).
        let strings = [
            "", "JsonResource", "cookingPlatform", "ECookingPlatform", "PLATFORM_Mac", "root",
            "handle:ISerializable", "localizationPersistenceOnScreenEntries", "entries",
            "array:localizationPersistenceOnScreenEntry", "primaryKey", "Uint64", "secondaryKey",
            "String", "femaleVariant", "maleVariant",
        ];
        let mut blob: Vec<u8> = Vec::new();
        for s in &strings {
            blob.extend_from_slice(s.as_bytes());
            blob.push(0);
        }
        // monta um "arquivo" = 160B de header dummy + o blob na tabela 0.
        let mut data = vec![0u8; 160];
        data.extend_from_slice(&blob);
        let table = Cr2wTable { offset: 160, item_count: blob.len() as u32, crc32: 0 };
        let dict = read_string_dict(&data, &table).unwrap();
        // offsets relativos: 0="" , 1="JsonResource", depois cada +len+1.
        assert_eq!(dict[&0], "");
        assert_eq!(dict[&1], "JsonResource");
        assert_eq!(dict[&14], "cookingPlatform");
        // os nomes que importam pra localização estão lá (indexados pelo offset que os names apontam):
        let vals: std::collections::HashSet<&str> = dict.values().map(|s| s.as_str()).collect();
        for want in ["primaryKey", "secondaryKey", "femaleVariant", "maleVariant", "String", "Uint64"] {
            assert!(vals.contains(want), "faltou '{want}' no dict");
        }
        assert_eq!(dict.len(), strings.len()); // 16 entradas
    }

    #[test]
    fn le_names_resolvendo_dict() {
        // dict sintético: 0="", 1="entries", 9="String".
        let mut dict = std::collections::HashMap::new();
        dict.insert(0u32, String::new());
        dict.insert(1u32, "entries".to_string());
        dict.insert(9u32, "String".to_string());
        // 2 names: offset 1 (entries) hash 0xAAAA; offset 9 (String) hash 0xBBBB.
        let mut data = vec![0u8; 200];
        let base = 200 - 16; // 2 names * 8B
        data.truncate(base);
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&0xAAAAu32.to_le_bytes());
        data.extend_from_slice(&9u32.to_le_bytes());
        data.extend_from_slice(&0xBBBBu32.to_le_bytes());
        let table = Cr2wTable { offset: base as u32, item_count: 2, crc32: 0 };
        let names = read_names(&data, &table, &dict).unwrap();
        assert_eq!(names, vec![(0xAAAA, "entries".to_string()), (0xBBBB, "String".to_string())]);
    }

    #[test]
    fn vlq_round_trip() {
        for v in [0i32, 1, -1, 42, -42, 63, -63, 64, 1000, -1000, 60303, -60303, 1_000_000] {
            let mut buf = Vec::new();
            write_vlq_i32(v, &mut buf);
            assert_eq!(read_vlq_i32(&buf, &mut 0).unwrap(), v, "vlq {v}");
        }
    }

    #[test]
    fn string_red_round_trip() {
        for s in ["", "Hi", "News", "Gameplay-Devices-Computers-Common-NewsFeed", "café ☕ 日本"] {
            let bytes = write_red_string(s);
            assert_eq!(read_red_string(&bytes, &mut 0).unwrap(), s, "string '{s}'");
        }
    }

    #[test]
    fn decodifica_valores_de_campo() {
        let names = vec![
            (0u32, "".into()), (0, "JsonResource".into()), (0, "cookingPlatform".into()),
            (0, "ECookingPlatform".into()), (0, "PLATFORM_Mac".into()), (0, "root".into()),
            (0, "handle:ISerializable".into()),
        ];
        // enum ECookingPlatform, valor u16=4 -> names[4]="PLATFORM_Mac".
        let enum_f = Cr2wField { name: "cookingPlatform".into(), red_type: "ECookingPlatform".into(), value: vec![4, 0] };
        assert_eq!(decode_field_value(&enum_f, &names), "PLATFORM_Mac");
        // handle, valor u32=2 -> chunk ref (1-based) -> #1.
        let h = Cr2wField { name: "root".into(), red_type: "handle:ISerializable".into(), value: vec![2, 0, 0, 0] };
        assert_eq!(decode_field_value(&h, &names), "→chunk #1");
        // Uint64.
        let u = Cr2wField { name: "primaryKey".into(), red_type: "Uint64".into(), value: 42u64.to_le_bytes().to_vec() };
        assert_eq!(decode_field_value(&u, &names), "42");
        // String.
        let s = Cr2wField { name: "x".into(), red_type: "String".into(), value: write_red_string("News") };
        assert_eq!(decode_field_value(&s, &names), "News");
        // handle null.
        let n = Cr2wField { name: "y".into(), red_type: "handle:Z".into(), value: vec![0, 0, 0, 0] };
        assert_eq!(decode_field_value(&n, &names), "null");
    }

    #[test]
    fn index_round_trip_byte_exato() {
        // parse do índice REAL -> write -> tem que dar os 160 bytes ORIGINAIS.
        let idx = parse_cr2w_index(&GOLDEN).unwrap();
        assert_eq!(write_cr2w_index(&idx), GOLDEN.to_vec());
    }

    #[test]
    fn chunk_fields_round_trip_byte_exato() {
        // parse do chunk 0 REAL -> write -> tem que dar os 25 bytes ORIGINAIS.
        let chunk: [u8; 25] = [
            0x00, 0x02, 0x00, 0x03, 0x00, 0x06, 0x00, 0x00, 0x00, 0x04, 0x00, 0x05, 0x00, 0x06,
            0x00, 0x08, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let names = vec![
            (0u32, "".to_string()), (0, "JsonResource".to_string()), (0, "cookingPlatform".to_string()),
            (0, "ECookingPlatform".to_string()), (0, "PLATFORM_Mac".to_string()), (0, "root".to_string()),
            (0, "handle:ISerializable".to_string()),
        ];
        let export = Cr2wExport { class_name: "JsonResource".into(), parent_id: 0, data_offset: 0, data_size: 25 };
        let (fields, appendix) = read_chunk_fields(&chunk, &export, &names).unwrap();
        // name -> índice (1ª ocorrência).
        let idx_of = |name: &str| names.iter().position(|(_, s)| s == name).map(|i| i as u16);
        let out = write_chunk_fields(&fields, &appendix, &idx_of, true).unwrap();
        assert_eq!(out, chunk.to_vec(), "round-trip do chunk não é byte-exato");
    }

    #[test]
    fn vlq_e_string_red() {
        // VLQ: 0x2a = +42 (bit7=0 pos, bit6=0 sem cont). 0xaa = -42 (bit7=1 neg).
        assert_eq!(read_vlq_i32(&[0x2a], &mut 0).unwrap(), 42);
        assert_eq!(read_vlq_i32(&[0xaa], &mut 0).unwrap(), -42);
        // String UTF-8: prefixo NEGATIVO. "Hi" (2 chars) -> prefix -2 = byte 0x82, depois "Hi".
        let utf8 = [0x82u8, b'H', b'i'];
        assert_eq!(read_red_string(&utf8, &mut 0).unwrap(), "Hi");
        // String UTF-16: prefixo POSITIVO. "Hi" -> prefix +2 = byte 0x02, depois H\0 i\0.
        let utf16 = [0x02u8, b'H', 0, b'i', 0];
        assert_eq!(read_red_string(&utf16, &mut 0).unwrap(), "Hi");
        // vazia.
        assert_eq!(read_red_string(&[0x00], &mut 0).unwrap(), "");
    }

    #[test]
    fn decodifica_chave_localizacao_real() {
        // valor REAL do secondaryKey do 1º entry do onscreens: 0xaa (=-42, UTF-8) + 42 bytes ASCII.
        let key = "Gameplay-Devices-Computer-Terminal-Access1"; // 42 chars
        assert_eq!(key.len(), 42);
        let mut bytes = vec![0xaau8];
        bytes.extend_from_slice(key.as_bytes());
        assert_eq!(read_red_string(&bytes, &mut 0).unwrap(), key);
    }

    #[test]
    fn deserializa_campos_do_chunk_real() {
        // bytes REAIS do chunk 0 (JsonResource) do onscreens_final, e os names que ele referencia.
        let chunk: [u8; 25] = [
            0x00, 0x02, 0x00, 0x03, 0x00, 0x06, 0x00, 0x00, 0x00, 0x04, 0x00, 0x05, 0x00, 0x06,
            0x00, 0x08, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let names = vec![
            (0u32, "".to_string()),
            (0, "JsonResource".to_string()),
            (0, "cookingPlatform".to_string()),
            (0, "ECookingPlatform".to_string()),
            (0, "PLATFORM_Mac".to_string()),
            (0, "root".to_string()),
            (0, "handle:ISerializable".to_string()),
        ];
        let export = Cr2wExport {
            class_name: "JsonResource".to_string(),
            parent_id: 0,
            data_offset: 0,
            data_size: 25,
        };
        let (fields, appendix) = read_chunk_fields(&chunk, &export, &names).unwrap();
        assert!(appendix.is_empty()); // JsonResource não tem appendix
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "cookingPlatform");
        assert_eq!(fields[0].red_type, "ECookingPlatform");
        assert_eq!(fields[0].value, vec![0x04, 0x00]); // enum (2B)
        assert_eq!(fields[1].name, "root");
        assert_eq!(fields[1].red_type, "handle:ISerializable");
        assert_eq!(fields[1].value, vec![0x02, 0x00, 0x00, 0x00]); // handle -> chunk ref (4B)
    }

    #[test]
    fn le_exports_resolvendo_tipo() {
        let names = vec![(0u32, String::new()), (0u32, "JsonResource".to_string())];
        // 1 export: className idx=1 (JsonResource), parentID 0, dataSize 25, dataOffset 592.
        let mut data = vec![0u8; 24];
        data[0..2].copy_from_slice(&1u16.to_le_bytes()); // className idx
        data[8..12].copy_from_slice(&25u32.to_le_bytes()); // dataSize
        data[12..16].copy_from_slice(&592u32.to_le_bytes()); // dataOffset
        let table = Cr2wTable { offset: 0, item_count: 1, crc32: 0 };
        let ex = read_exports(&data, &table, &names).unwrap();
        assert_eq!(ex.len(), 1);
        assert_eq!(ex[0].class_name, "JsonResource");
        assert_eq!(ex[0].data_offset, 592);
        assert_eq!(ex[0].data_size, 25);
    }

    #[test]
    fn string_dict_fora_do_arquivo_erra() {
        let table = Cr2wTable { offset: 1000, item_count: 50, crc32: 0 };
        assert!(read_string_dict(&[0u8; 200], &table).is_err());
    }

    #[test]
    fn rejeita_lixo() {
        assert!(parse_cr2w_index(b"NOPE").is_err()); // curto + magic errado
        let mut bad = GOLDEN;
        bad[0] = b'X'; // magic quebrado
        assert!(parse_cr2w_index(&bad).is_err());
        let mut badver = GOLDEN;
        badver[4] = 50; // versão 50, fora da faixa
        assert!(parse_cr2w_index(&badver).is_err());
    }
}
