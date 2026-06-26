//! Reescrita do `tweakdb.bin` — modelo editável + serializador que reproduz o
//! layout REAL (o que o reader lê / o jogo carrega), com offsets recalculados.
//!
//! Importante: o `TweakDBWriter` do WolvenKit grava um formato **delta** (tabela
//! de flat types de 12 bytes, sem offsets) que NÃO é o do jogo — por isso aqui
//! reproduzimos o layout do reader, não portamos o writer. Para ser byte-exato,
//! os valores são preservados como **bytes crus** (strings VLQ são ambíguas ao
//! decodificar); só os offsets são recalculados.

use crate::hashes::tweak_db_id;
use crate::names::NameDb;
use crate::tweakdb::{self, RawGroup, TweakDb, MAGIC};

pub struct Model {
    pub blob_version: i32,
    pub parser_version: i32,
    pub record_checksum: u32,
    pub groups: Vec<RawGroup>,
    /// Índice do `ETweakType` de cada grupo (paralelo a `groups`), para o `set`.
    pub group_type_index: Vec<usize>,
    /// Se cada grupo é um array (paralelo a `groups`). O `set`/`batch` editam só
    /// escalares — escrever um escalar onde o flat é array corromperia o bloco.
    pub group_is_array: Vec<bool>,
    /// Counts originais da TABELA de flat types (valueCount, keyCount). O reader
    /// os IGNORA (usa os counts do bloco), mas eles diferem dos reais no arquivo
    /// do jogo — preservá-los é o que torna o round-trip byte-exato.
    pub table_counts: Vec<(u32, u32)>,
    pub records: Vec<(u64, u32)>,
    pub queries: Vec<(u64, Vec<u64>)>,
    pub group_tags: Vec<(u64, u8)>,
}

impl Model {
    /// Constrói o modelo completo a partir de um tweakdb aberto.
    pub fn from_db(db: &TweakDb) -> Result<Model, String> {
        let mut groups = Vec::with_capacity(db.flat_types.len());
        let mut group_type_index = Vec::with_capacity(db.flat_types.len());
        let mut group_is_array = Vec::with_capacity(db.flat_types.len());
        let mut table_counts = Vec::with_capacity(db.flat_types.len());
        for (i, ft) in db.flat_types.iter().enumerate() {
            let resolved = ft
                .resolved
                .ok_or_else(|| format!("typeHash {:016x} não resolvido; não dá para reescrever", ft.type_hash))?;
            groups.push(db.read_group_raw(i).map_err(|e| e.to_string())?);
            group_type_index.push(resolved.index);
            group_is_array.push(resolved.is_array);
            table_counts.push((ft.value_count, ft.key_count));
        }
        Ok(Model {
            blob_version: db.blob_version,
            parser_version: db.parser_version,
            record_checksum: db.record_checksum,
            groups,
            group_type_index,
            group_is_array,
            table_counts,
            records: db.records.iter().map(|r| (r.id, r.type_key)).collect(),
            queries: db.read_queries().map_err(|e| e.to_string())?,
            group_tags: db.read_group_tags().map_err(|e| e.to_string())?,
        })
    }

    /// Serializa de volta para o layout do jogo. Sem edições, é byte-idêntico ao
    /// original.
    pub fn serialize(&self) -> Vec<u8> {
        let mut w = Writer::with_header();
        const HEADER: usize = 32;

        // --- Flats ---
        let flats_offset = w.len() as u32; // 32
        w.i32(self.groups.len() as i32);

        // Offsets dos blocos: tabela em [agora, agora + N*20], blocos depois.
        let table_start = w.len();
        let table_size = self.groups.len() * 20;
        let mut running = (table_start + table_size) as u32;
        let mut offsets = Vec::with_capacity(self.groups.len());
        for g in &self.groups {
            offsets.push(running);
            let values_bytes: usize = g.values.iter().map(Vec::len).sum();
            let block = 4 + values_bytes + 4 + g.keys.len() * 12;
            running += block as u32;
        }

        // Tabela de flat types. valueCount/keyCount vêm dos counts ORIGINAIS da
        // tabela (o reader os ignora; no arquivo do jogo eles diferem dos reais).
        for ((g, &off), &(vc, kc)) in self.groups.iter().zip(&offsets).zip(&self.table_counts) {
            w.u64(g.type_hash);
            w.u32(vc);
            w.u32(kc);
            w.u32(off);
        }
        // Blocos: valores + chaves.
        for g in &self.groups {
            w.u32(g.values.len() as u32);
            for v in &g.values {
                w.bytes(v);
            }
            w.u32(g.keys.len() as u32);
            // Flats: idem records, ordenados por crc32 (low 32) primeiro p/ a busca binária.
            let mut keys_sorted: Vec<(u64, i32)> = g.keys.clone();
            keys_sorted.sort_by_key(|&(id, _)| ((id & 0xFFFF_FFFF) as u32, (id >> 32) as u32));
            for &(id, idx) in &keys_sorted {
                w.u64(id);
                w.i32(idx);
            }
        }

        // --- Records ---
        let records_offset = w.len() as u32;
        w.i32(self.records.len() as i32);
        // TESTE: ordena por crc32 (low 32) primeiro, depois len (high 32).
        let mut recs_sorted: Vec<(u64, u32)> = self.records.clone();
        recs_sorted.sort_by_key(|&(id, _)| ((id & 0xFFFF_FFFF) as u32, (id >> 32) as u32));
        for &(id, key) in &recs_sorted {
            w.u64(id);
            w.u32(key);
        }

        // --- Queries ---
        let queries_offset = w.len() as u32;
        w.i32(self.queries.len() as i32);
        for (id, results) in &self.queries {
            w.u64(*id);
            w.u32(results.len() as u32);
            for r in results {
                w.u64(*r);
            }
        }

        // --- Group tags ---
        let group_tags_offset = w.len() as u32;
        w.i32(self.group_tags.len() as i32);
        for &(id, tag) in &self.group_tags {
            w.u64(id);
            w.u8(tag);
        }

        // --- Backfill do header (0..32) ---
        let mut head = Writer::new();
        head.u32(MAGIC);
        head.i32(self.blob_version);
        head.i32(self.parser_version);
        head.u32(self.record_checksum);
        head.u32(flats_offset);
        head.u32(records_offset);
        head.u32(queries_offset);
        head.u32(group_tags_offset);
        debug_assert_eq!(head.buf.len(), HEADER);
        w.buf[..HEADER].copy_from_slice(&head.buf);

        w.buf
    }

    /// Acha o grupo (tipo) e a chave de um flat pelo NOME, ou `None` se não existir.
    fn locate(&self, id: u64) -> Option<(usize, usize)> {
        for (gi, g) in self.groups.iter().enumerate() {
            if let Some(ki) = g.keys.iter().position(|&(kid, _)| kid == id) {
                return Some((gi, ki));
            }
        }
        None
    }

    /// Aplica uma edição (classificada) a um flat pelo NOME. Cobre escalares
    /// (`Assign`) e arrays (`Assign [..]`, `Append`, `Remove`). Adiciona o novo
    /// valor ao pool do grupo (ou reusa um idêntico) e repont a a chave.
    pub fn apply(&mut self, name: &str, op: &EditOp) -> SetOutcome {
        let id = tweak_db_id(name);
        let Some((gi, ki)) = self.locate(id) else {
            return SetOutcome::NotFound;
        };
        let type_index = self.group_type_index[gi];
        let is_array = self.group_is_array[gi];
        let label = self.label(gi);

        let new_bytes = match self.compute_value(gi, ki, type_index, is_array, op) {
            Ok(b) => b,
            Err(reason) => return SetOutcome::NotEditable { ty: label, reason },
        };

        let group = &mut self.groups[gi];
        let value_index = match group.values.iter().position(|v| *v == new_bytes) {
            Some(idx) => idx,
            None => {
                group.values.push(new_bytes);
                group.values.len() - 1
            }
        };
        group.keys[ki].1 = value_index as i32;
        SetOutcome::Applied(label)
    }

    /// Wrapper de atribuição pro `set` da CLI: aplica `= valor` e devolve Result
    /// com o rótulo do tipo.
    pub fn set_flat(&mut self, name: &str, value: &str) -> Result<String, String> {
        match self.apply(name, &EditOp::Assign(value.to_string())) {
            SetOutcome::Applied(ty) => Ok(ty),
            SetOutcome::NotFound => Err(format!(
                "flat '{name}' (id {:016x}) não existe neste tweakdb",
                tweak_db_id(name)
            )),
            SetOutcome::NotEditable { ty, reason } => {
                Err(format!("flat '{name}' ({ty}): {reason}"))
            }
        }
    }

    fn label(&self, gi: usize) -> String {
        let base = tweakdb::TWEAK_TYPES[self.group_type_index[gi]].0;
        if self.group_is_array[gi] {
            format!("array:{base}")
        } else {
            base.to_string()
        }
    }

    fn current_value_bytes(&self, gi: usize, ki: usize) -> Result<Vec<u8>, String> {
        let g = &self.groups[gi];
        let idx = g.keys[ki].1 as usize;
        g.values
            .get(idx)
            .cloned()
            .ok_or_else(|| "valueIndex atual fora do alcance".to_string())
    }

    /// Bytes do novo valor conforme a operação e o tipo do flat.
    fn compute_value(
        &self,
        gi: usize,
        ki: usize,
        type_index: usize,
        is_array: bool,
        op: &EditOp,
    ) -> Result<Vec<u8>, String> {
        match op {
            EditOp::Assign(v) => {
                if is_array {
                    let elems = parse_array_literal(v)?;
                    let mut blobs = Vec::with_capacity(elems.len());
                    for e in &elems {
                        blobs.push(tweakdb::encode_value(type_index, e)?);
                    }
                    Ok(tweakdb::encode_array(&blobs))
                } else {
                    tweakdb::encode_value(type_index, v)
                }
            }
            EditOp::Append(v) => {
                if !is_array {
                    return Err("'+=' só vale para array (escalar usa '=')".into());
                }
                let cur = self.current_value_bytes(gi, ki)?;
                let mut blobs =
                    tweakdb::split_array_elements(&cur, type_index).map_err(|e| e.to_string())?;
                blobs.push(tweakdb::encode_value(type_index, v)?);
                Ok(tweakdb::encode_array(&blobs))
            }
            EditOp::AppendOnce(v) => {
                if !is_array {
                    return Err("'!append-once' só vale para array".into());
                }
                let elem = tweakdb::encode_value(type_index, v)?;
                let cur = self.current_value_bytes(gi, ki)?;
                let mut blobs =
                    tweakdb::split_array_elements(&cur, type_index).map_err(|e| e.to_string())?;
                if !blobs.contains(&elem) {
                    blobs.push(elem);
                }
                Ok(tweakdb::encode_array(&blobs))
            }
            EditOp::Prepend(v) => {
                if !is_array {
                    return Err("'!prepend' só vale para array".into());
                }
                let cur = self.current_value_bytes(gi, ki)?;
                let mut blobs =
                    tweakdb::split_array_elements(&cur, type_index).map_err(|e| e.to_string())?;
                blobs.insert(0, tweakdb::encode_value(type_index, v)?);
                Ok(tweakdb::encode_array(&blobs))
            }
            EditOp::PrependOnce(v) => {
                if !is_array {
                    return Err("'!prepend-once' só vale para array".into());
                }
                let elem = tweakdb::encode_value(type_index, v)?;
                let cur = self.current_value_bytes(gi, ki)?;
                let mut blobs =
                    tweakdb::split_array_elements(&cur, type_index).map_err(|e| e.to_string())?;
                if !blobs.contains(&elem) {
                    blobs.insert(0, elem);
                }
                Ok(tweakdb::encode_array(&blobs))
            }
            EditOp::Remove(v) => {
                if !is_array {
                    return Err("'-=' só vale para array".into());
                }
                let target = tweakdb::encode_value(type_index, v)?;
                let cur = self.current_value_bytes(gi, ki)?;
                let mut blobs =
                    tweakdb::split_array_elements(&cur, type_index).map_err(|e| e.to_string())?;
                let before = blobs.len();
                blobs.retain(|b| *b != target);
                if blobs.len() == before {
                    return Err(format!("elemento '{v}' não está no array"));
                }
                Ok(tweakdb::encode_array(&blobs))
            }
            EditOp::AppendFrom(src) | EditOp::PrependFrom(src) => {
                if !is_array {
                    return Err("'!append-from'/'!merge'/'!prepend-from' só valem para array".into());
                }
                // Lê os elementos do array-FONTE (outro flat array do mesmo tipo).
                let src_blobs = self.source_array_blobs(src, type_index)?;
                let cur = self.current_value_bytes(gi, ki)?;
                let mut blobs =
                    tweakdb::split_array_elements(&cur, type_index).map_err(|e| e.to_string())?;
                match op {
                    EditOp::AppendFrom(_) => blobs.extend(src_blobs),
                    _ => {
                        // prepend: fonte vai na frente, preservando a ordem dela.
                        let mut merged = src_blobs;
                        merged.append(&mut blobs);
                        blobs = merged;
                    }
                }
                Ok(tweakdb::encode_array(&blobs))
            }
        }
    }

    /// Lê os blobs de elemento de um flat-FONTE (array) pelo nome, exigindo o
    /// mesmo `ETweakType` do alvo (compatibilidade de elemento). É o lado offline
    /// do `AppendFrom`/`!merge` do TweakXL.
    fn source_array_blobs(&self, src: &str, want_type: usize) -> Result<Vec<Vec<u8>>, String> {
        let src_id = tweak_db_id(src);
        let (sgi, ski) = self
            .locate(src_id)
            .ok_or_else(|| format!("flat-fonte '{src}' não existe neste tweakdb"))?;
        if !self.group_is_array[sgi] {
            return Err(format!("flat-fonte '{src}' não é array"));
        }
        if self.group_type_index[sgi] != want_type {
            return Err(format!(
                "flat-fonte '{src}' tem tipo de elemento diferente do alvo — não dá pra mesclar"
            ));
        }
        let bytes = self.current_value_bytes(sgi, ski)?;
        tweakdb::split_array_elements(&bytes, want_type).map_err(|e| e.to_string())
    }

    /// Clona um record: cria `dst` como cópia de `src` — todos os flats
    /// `src.prop` viram `dst.prop` (apontando o mesmo valor) e adiciona a entrada
    /// de record `dst` com o MESMO type_key de `src`. É o "copia e ajusta" do
    /// TweakXL, viável offline (não precisa do schema RTTI, só copia o existente).
    /// Depois use `set dst.prop ...` pra customizar. Devolve o nº de flats clonados.
    pub fn clone_record(&mut self, src: &str, dst: &str, names: &NameDb) -> Result<usize, String> {
        let src_id = tweak_db_id(src);
        let dst_id = tweak_db_id(dst);
        let type_key = self
            .records
            .iter()
            .find(|(id, _)| *id == src_id)
            .map(|(_, k)| *k)
            .ok_or_else(|| format!("record '{src}' não existe (ou não é um record)"))?;
        if self.records.iter().any(|(id, _)| *id == dst_id) {
            return Err(format!("record '{dst}' já existe — escolha outro nome"));
        }

        let prefix = format!("{src}.");
        // Coleta antes de mutar (não dá pra mutar self.groups enquanto itera).
        let mut additions: Vec<(usize, u64, i32)> = Vec::new();
        for (gi, g) in self.groups.iter().enumerate() {
            for &(kid, vidx) in &g.keys {
                if let Some(name) = names.resolve(kid) {
                    if let Some(prop) = name.strip_prefix(&prefix) {
                        additions.push((gi, tweak_db_id(&format!("{dst}.{prop}")), vidx));
                    }
                }
            }
        }
        if additions.is_empty() {
            return Err(format!(
                "nenhum flat '{src}.*' encontrado (record sem flats ou nomes indisponíveis)"
            ));
        }

        let mut cloned = 0usize;
        for (gi, fid, vidx) in additions {
            if !self.groups[gi].keys.iter().any(|&(k, _)| k == fid) {
                self.groups[gi].keys.push((fid, vidx));
                cloned += 1;
            }
        }
        self.records.push((dst_id, type_key));
        Ok(cloned)
    }

    /// Cria `dst` do ZERO a partir de uma CLASSE RED (`$type` do TweakXL). O
    /// schema é inferido offline: acha um record de AMOSTRA da mesma classe
    /// (type_key = murmur3 do nome, ver [`crate::hashes::record_type_key`]) e o
    /// clona — `dst` herda todos os flats da classe com os valores da amostra,
    /// que o usuário sobrescreve depois. Não precisa do RTTI do jogo.
    /// Devolve `(nome_da_amostra, nº de flats)`.
    pub fn create_record(
        &mut self,
        dst: &str,
        class_name: &str,
        names: &NameDb,
    ) -> Result<(String, usize), String> {
        let k = crate::hashes::record_type_key(class_name);
        let sample = self
            .records
            .iter()
            .filter(|(_, tk)| *tk == k)
            .find_map(|(id, _)| names.resolve(*id))
            .ok_or_else(|| {
                format!(
                    "classe '{class_name}' (type_key {k:08x}) sem record de amostra neste tweakdb — \
                     não dá pra inferir o schema (confira o nome da classe)"
                )
            })?
            .to_string();
        let n = self.clone_record(&sample, dst, names)?;
        Ok((sample, n))
    }
}

/// Operação de edição de um flat. As variantes de array espelham as tags do
/// TweakXL (`!append`, `!prepend`, `!append-once`, `!prepend-once`, `!remove`).
pub enum EditOp {
    /// `= valor` (escalar) ou `= [a, b, c]` (array inteiro).
    Assign(String),
    /// `+= valor` / `!append` — adiciona um elemento ao FIM do array.
    Append(String),
    /// `!append-once` — adiciona ao fim só se ainda não estiver presente.
    AppendOnce(String),
    /// `!prepend` — adiciona um elemento ao INÍCIO do array.
    Prepend(String),
    /// `!prepend-once` — adiciona ao início só se ainda não estiver presente.
    PrependOnce(String),
    /// `-= valor` / `!remove` — remove os elementos iguais (igualdade de bytes).
    Remove(String),
    /// `!append-from` / `!merge` — anexa ao FIM os elementos de outro flat array
    /// (pelo nome). O TweakXL trata `!merge` e `!append-from` como o mesmo op.
    AppendFrom(String),
    /// `!prepend-from` — anexa ao INÍCIO os elementos de outro flat array.
    PrependFrom(String),
}

/// Resultado de aplicar um edit a um flat.
pub enum SetOutcome {
    /// Editado; rótulo do tipo (ex.: "CFloat", "array:TweakDBID").
    Applied(String),
    /// Nenhum flat com esse nome neste tweakdb.
    NotFound,
    /// Existe, mas a operação/valor não casa (tipo, faixa, sintaxe de array).
    NotEditable { ty: String, reason: String },
}

/// Parseia `[a, b, c]` (ou `[]`) em elementos-texto. Separa por vírgula → não
/// cobre elementos com vírgula interna (Vector/Color); pra esses use `+=` por
/// elemento. Cobre arrays escalares comuns (IDs, names, floats, ints).
fn parse_array_literal(v: &str) -> Result<Vec<String>, String> {
    let inner = v
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| format!("array exige '[a, b, c]' (ou use += / -=); recebido '{v}'"))?;
    let inner = inner.trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    Ok(inner.split(',').map(|s| s.trim().to_string()).collect())
}

/// Escritor LE simples.
struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Writer { buf: Vec::new() }
    }
    /// Começa com 32 bytes zerados reservados para o header.
    fn with_header() -> Self {
        Writer { buf: vec![0u8; 32] }
    }
    fn len(&self) -> usize {
        self.buf.len()
    }
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }
}
