//! Teste ao vivo do escritor único (`~/.bwms-hook-tdb-escritor-teste`), em gameplay com a trava de save: dois flats que dividem o mesmo
//! `FlatValue` (o buffer deduplica), um é gravado pelo [`super::escritor`] e o outro tem que ficar como estava — a gravação no lugar, que
//! o `SetFlat` fazia até 2026-09-19, mudaria os dois. Depois o diário desfaz e o flat volta ao offset e ao valor exatos de antes.

use super::escritor::{desfazer, gravar_escalar, largura_do_tipo, largura_do_valor, offset_original, tipo_do_valor};
use super::{entry_tdb_offset, find_flat_entry_by_id, get_flat_value_by_id, singleton, tweak_db_id, tweak_db_id_chave, FLATS_OFF, FLATS_SIZE_OFF};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Flats candidatos (o 1º que existir e dividir o valor com outro flat é o alvo).
const CANDIDATOS: &[&str] = &[
    "Items.Preset_Yukimura_Default.quality",
    "Items.Preset_Lexington_Default.quality",
    "Items.GrenadeIncendiarySticky.deepWaterDepth",
];

unsafe fn valor(t: *const u8, id: u64, mascara: u64) -> Option<u64> {
    let fv = get_flat_value_by_id(t, id)?;
    Some((fv.add(0x08) as *const u64).read_unaligned() & mascara)
}

unsafe fn offset(t: *const u8, id: u64) -> Option<u32> {
    find_flat_entry_by_id(t, id).map(|e| entry_tdb_offset(e.read_unaligned()))
}

/// Outro flat do buffer com o mesmo offset de valor (o 1º achado na varredura da lista ordenada).
unsafe fn vizinho_de_valor(t: *const u8, id: u64, off: u32) -> Option<u64> {
    let entradas = (t.add(FLATS_OFF) as *const u64).read_unaligned() as *const u64;
    let n = (t.add(FLATS_SIZE_OFF) as *const u32).read_unaligned() as usize;
    if entradas.is_null() || n == 0 || n > 10_000_000 || !crate::memoria::is_readable(entradas as *const core::ffi::c_void, n * 8) {
        return None;
    }
    let chave = tweak_db_id_chave(id);
    (0..n).map(|i| entradas.add(i).read_unaligned()).find(|e| tweak_db_id_chave(*e) != chave && entry_tdb_offset(*e) == off)
        .map(tweak_db_id_chave)
}

/// YAML de teste: clone de arma com atribuição de record (`TweakDBID`), `CName` e array de `CName` por texto, e clone de stat modifier
/// (classe fora da antiga lista fixa de 6).
const YAML_TESTE: &str = "Items.BwmsTesteArma:\n  $base: Items.Preset_Yukimura_Default\n  quality: Quality.Legendary\n  appearanceName: bwms_teste_aparencia\n  tags: [Weapon, BwmsTeste]\nModifiers.BwmsTesteMod:\n  $base: Modifiers.BleedingApplicationRateRandom\n";
/// `.tweak` de teste: o formato-texto nativo, pelo mesmo aplicador.
const TWEAK_TESTE: &str = "Items.BwmsTesteArma2 : Items.Preset_Yukimura_Default {\n    quality = \"Quality.Epic\";\n}\n";

/// Arquivos de tweak de verdade (`~/.bwms-hook-tdb-arquivos-teste`): um YAML e um `.tweak` gravados em disco e aplicados pelo
/// [`super::aplicar_arquivo`] (o caminho dos mods), conferidos no TweakDB vivo uns segundos depois.
pub(crate) fn tick_arquivos() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FASE: AtomicU64 = AtomicU64::new(0);
    static INICIO: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    static APLICADO: std::sync::OnceLock<(std::time::Instant, u64, Option<(u64, u64, u64)>)> = std::sync::OnceLock::new();
    if FASE.load(Ordering::Relaxed) >= 2 || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("tdb-arquivos-teste") {
        return;
    }
    if !crate::selftest::trava_de_save_confirmada() || !crate::inicio::boot::gameplay_real() || INICIO.get_or_init(std::time::Instant::now).elapsed().as_secs() < 40 {
        return;
    }
    unsafe {
        let Some(t) = singleton() else { return };
        let t = t as *const u8;
        if FASE.load(Ordering::Relaxed) == 0 {
            let base_quality = super::get_flat_tdbid(t, tweak_db_id("Items.Preset_Yukimura_Default.quality")).unwrap_or(0);
            let dir = std::env::temp_dir().join("bwms-tdb-teste");
            let _ = std::fs::create_dir_all(&dir);
            let (yaml, tweak) = (dir.join("teste.yaml"), dir.join("teste.tweak"));
            let gravou = std::fs::write(&yaml, YAML_TESTE).is_ok() && std::fs::write(&tweak, TWEAK_TESTE).is_ok();
            super::aplicar_arquivo(&yaml.to_string_lossy());
            super::aplicar_arquivo(&tweak.to_string_lossy());
            let _ = APLICADO.set((std::time::Instant::now(), base_quality, super::escritor::folga_do_buffer()));
            crate::log_veredito(&format!("[tdbf] arquivos gravados {gravou} e aplicados; quality do base antes {base_quality:#x}"));
            FASE.store(1, Ordering::Relaxed);
            return;
        }
        let Some((quando, base_quality, folga_antes)) = APLICADO.get().copied() else { return };
        let folga_depois = super::escritor::folga_do_buffer();
        if quando.elapsed().as_secs() < 4 {
            return;
        }
        FASE.store(2, Ordering::Relaxed);
        let existe = |nome: &str| !super::get_record_instance(t as *mut u8, tweak_db_id(nome)).is_null();
        let tdbid = |flat: &str| super::get_flat_tdbid(t, tweak_db_id(flat));
        let arma = existe("Items.BwmsTesteArma");
        let quality = tdbid("Items.BwmsTesteArma.quality");
        let aparencia = super::get_flat_cname(t, tweak_db_id("Items.BwmsTesteArma.appearanceName"));
        let tags = super::get_flat_array_u64_by_id(t, tweak_db_id("Items.BwmsTesteArma.tags"));
        let base_depois = tdbid("Items.Preset_Yukimura_Default.quality");
        let modificador = existe("Modifiers.BwmsTesteMod");
        let stat_mod = tdbid("Modifiers.BwmsTesteMod.statType");
        let stat_base = tdbid("Modifiers.BleedingApplicationRateRandom.statType");
        let arma2 = existe("Items.BwmsTesteArma2");
        let quality2 = tdbid("Items.BwmsTesteArma2.quality");
        let tags_esperadas = vec![crate::cname::cname("Weapon"), crate::cname::cname("BwmsTeste")];
        let ok = arma
            && quality == Some(tweak_db_id("Quality.Legendary"))
            && aparencia == Some(crate::cname::cname("bwms_teste_aparencia"))
            && tags.as_ref() == Some(&tags_esperadas)
            && base_depois == Some(base_quality)
            && base_quality != 0
            && modificador
            && stat_mod.is_some()
            && stat_mod == stat_base
            && arma2
            && quality2 == Some(tweak_db_id("Quality.Epic"));
        crate::log_veredito(&format!(
            "[tdbf] VEREDITO-TDBF YAML: clone da arma {arma}, quality {quality:x?} (Legendary {:#x}), appearanceName {aparencia:x?}, tags \
             {tags:x?}; base intacto {base_quality:#x} -> {base_depois:x?}; clone do stat modifier (classe fora da lista antiga) {modificador}, \
             statType {stat_mod:x?} = base {stat_base:x?}; .tweak: clone {arma2}, quality {quality2:x?} (Epic {:#x}); buffer de valores (capacidade, \
             em uso, livre) antes {folga_antes:?} depois {folga_depois:?}; arquivos-ok={ok}",
            tweak_db_id("Quality.Legendary"),
            tweak_db_id("Quality.Epic"),
        ));
    }
}

/// Recarga (`~/.bwms-hook-tdb-recarga-teste`): v1 clona uma arma com `quality` e `!append` numa tag e cria um stat modifier por `$type`
/// com o nome curto da classe; recarregar com o mesmo arquivo não pode duplicar a tag; recarregar com a v2 (sem as linhas, `$base` trocado)
/// tem que devolver os valores e re-herdar o record que já existe do base novo.
const RECARGA_V1: &str = "Items.BwmsTesteArma3:\n  $base: Items.Preset_Yukimura_Default\n  quality: Quality.Legendary\n  tags:\n    - !append BwmsTag1\nModifiers.BwmsTesteTipo:\n  $type: ConstantStatModifier\n";
const RECARGA_V2: &str = "Items.BwmsTesteArma3:\n  $base: Items.Preset_Lexington_Default\n";

pub(crate) fn tick_recarga() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FASE: AtomicU64 = AtomicU64::new(0);
    static INICIO: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    static ULTIMA: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    static NOTAS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let fase = FASE.load(Ordering::Relaxed);
    if fase >= 4 || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("tdb-recarga-teste") {
        return;
    }
    if !crate::selftest::trava_de_save_confirmada() || !crate::inicio::boot::gameplay_real() || INICIO.get_or_init(std::time::Instant::now).elapsed().as_secs() < 70 {
        return;
    }
    let Ok(mut ultima) = ULTIMA.lock() else { return };
    if ultima.is_some_and(|u| u.elapsed().as_secs() < 4) {
        return;
    }
    *ultima = Some(std::time::Instant::now());
    unsafe {
        let Some(t) = singleton() else { return };
        let t = t as *const u8;
        let dir = std::env::temp_dir().join("bwms-tdb-teste");
        let _ = std::fs::create_dir_all(&dir);
        let arquivo = dir.join("recarga.yaml");
        let caminho = arquivo.to_string_lossy().to_string();
        let tags = |rec: &str| super::get_flat_array_u64_by_id(t, tweak_db_id(&format!("{rec}.tags")));
        let quality = |rec: &str| super::get_flat_tdbid(t, tweak_db_id(&format!("{rec}.quality")));
        let tag1 = crate::cname::cname("BwmsTag1");
        let mut notas = NOTAS.lock().map(|n| n.clone()).unwrap_or_default();
        match fase {
            0 => {
                let _ = std::fs::write(&arquivo, RECARGA_V1);
                super::aplicar_arquivo(&caminho);
            }
            1 | 2 => {
                let base = tags("Items.Preset_Yukimura_Default").unwrap_or_default();
                let clone = tags("Items.BwmsTesteArma3").unwrap_or_default();
                let tag_uma_vez = clone.len() == base.len() + 1 && clone.last() == Some(&tag1) && clone.iter().filter(|x| **x == tag1).count() == 1;
                let q = quality("Items.BwmsTesteArma3") == Some(tweak_db_id("Quality.Legendary"));
                let tipo = super::get_record_instance(t as *mut u8, tweak_db_id("Modifiers.BwmsTesteTipo"));
                let classe = if tipo.is_null() {
                    String::new()
                } else {
                    crate::cname::resolve_cname(crate::rtti::type_name_hash(crate::rtti::class_of(tipo as *mut core::ffi::c_void)))
                };
                notas.push(format!(
                    "{} aplicação: tags {} (base {}) tag uma vez {tag_uma_vez}, quality Legendary {q}, $type curto -> '{classe}'",
                    if fase == 1 { "1ª" } else { "depois da recarga com o mesmo arquivo:" },
                    clone.len(),
                    base.len()
                ));
                notas.push(format!("ok{fase}={}", tag_uma_vez && q && classe == "gamedataConstantStatModifier_Record"));
                let voltaram = super::escritor::desfazer_arquivos();
                let _ = std::fs::write(&arquivo, if fase == 1 { RECARGA_V1 } else { RECARGA_V2 });
                super::aplicar_arquivo(&caminho);
                notas.push(format!("recarga {fase}: {voltaram} flat(s) desfeito(s)"));
            }
            _ => {
                let lex_tags = tags("Items.Preset_Lexington_Default");
                let lex_q = quality("Items.Preset_Lexington_Default");
                let (c_tags, c_q) = (tags("Items.BwmsTesteArma3"), quality("Items.BwmsTesteArma3"));
                let rearmou = c_tags.is_some() && c_tags == lex_tags && c_q == lex_q && !c_tags.as_ref().is_some_and(|v| v.contains(&tag1));
                let ok = notas.iter().any(|n| n == "ok1=true") && notas.iter().any(|n| n == "ok2=true") && rearmou;
                crate::log_veredito(&format!(
                    "[tdbr] VEREDITO-TDBR {}; v2 (linhas apagadas, $base Lexington): tags {:?} = Lexington {:?}, quality {c_q:x?} = Lexington {lex_q:x?}, \
                     sem BwmsTag1 {rearmou}; recarga-ok={ok}",
                    notas.join("; "),
                    c_tags.as_ref().map(|v| v.len()),
                    lex_tags.as_ref().map(|v| v.len()),
                ));
            }
        }
        if let Ok(mut n) = NOTAS.lock() {
            *n = notas;
        }
        FASE.store(fase + 1, Ordering::Relaxed);
    }
}

/// Crescimento do buffer de valores (`~/.bwms-hook-tdb-buffer-teste`): o buffer cresce à força pro máximo e o jogo
/// tem que continuar lendo tudo — o valor antigo pelo índice e, pelo caminho do próprio motor (o getter `DeepWaterDepth()` do record, que lê
/// pela cópia do ponteiro em `TweakDB+0x00`), um valor gravado DEPOIS do crescimento, que só existe no buffer novo. Depois o diário desfaz.
pub(crate) fn tick_buffer() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FEITO: AtomicBool = AtomicBool::new(false);
    static INICIO: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    if FEITO.load(Ordering::Relaxed) || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("tdb-buffer-teste") {
        return;
    }
    if !crate::selftest::trava_de_save_confirmada() || !crate::inicio::boot::gameplay_real() || INICIO.get_or_init(std::time::Instant::now).elapsed().as_secs() < 35 {
        return;
    }
    FEITO.store(true, Ordering::Relaxed);
    let linha = unsafe { rodar_buffer() };
    crate::log_veredito(&linha);
}

unsafe fn rodar_buffer() -> String {
    let (Some(t), Some(reg)) = (singleton(), crate::rtti::Registry::obtain()) else {
        return "[tdbb] VEREDITO-TDBB sem TweakDB ou registry; buffer-ok=false".into();
    };
    let flat = tweak_db_id("Items.GrenadeIncendiarySticky.deepWaterDepth");
    let registro = super::get_record_instance(t, tweak_db_id("Items.GrenadeIncendiarySticky"));
    let getter = crate::rtti::resolve_func(&reg, "gamedataGrenade_Record", "DeepWaterDepth");
    let ler_getter = || -> Option<f32> {
        let rf = getter.as_ref()?;
        let b = crate::rtti::call_func(rf, registro as *mut core::ffi::c_void, &[])?;
        Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let ler_indice = |id: u64| valor(t as *const u8, id, u64::MAX);
    let quality = tweak_db_id("Items.Preset_Yukimura_Default.quality");
    let (getter_antes, quality_antes) = (ler_getter(), ler_indice(quality));
    let crescimento = super::buffer::crescer();
    let estatico = (t as *const u64).read_unaligned();
    let folga = super::escritor::folga_do_buffer();
    let quality_depois = ler_indice(quality);
    let novo_valor = -7.25f32;
    let gravou = gravar_escalar(flat, novo_valor.to_bits() as u64);
    let getter_depois = ler_getter();
    let desfez = desfazer(flat);
    let getter_volta = ler_getter();
    let (ok_cresceu, novo) = match &crescimento {
        Ok((antigo, novo, cap, _)) => (novo != antigo && *cap == super::buffer::CAPACIDADE_MAXIMA && estatico == *novo, *novo),
        Err(_) => (false, 0),
    };
    let ok = ok_cresceu
        && !registro.is_null()
        && getter_antes.is_some()
        && quality_depois == quality_antes
        && gravou
        && getter_depois == Some(novo_valor)
        && desfez
        && getter_volta == getter_antes;
    format!(
        "[tdbb] VEREDITO-TDBB crescimento {crescimento:x?}; TweakDB+0x00 {estatico:#x} = buffer novo {}; folga depois {folga:?}; quality da \
         Yukimura pelo índice {quality_antes:x?} -> {quality_depois:x?}; DeepWaterDepth() pelo getter do jogo {getter_antes:?}, valor novo \
         gravado depois do crescimento {gravou} -> getter {getter_depois:?} (esperado {novo_valor}), desfez {desfez} -> {getter_volta:?}; \
         buffer-ok={ok}",
        estatico == novo
    )
}

/// UpdateRecord (`~/.bwms-hook-tdb-updaterecord-teste`): o flat `deepWaterDepth` da granada é re-apontado direto no índice, sem o escritor (sem
/// subir a versão do TweakDB); o getter nativo `DeepWaterDepth()` do record tem que ler o valor novo depois do `UpdateRecord`, e o de antes
/// depois da volta do offset e de outro `UpdateRecord`. A prova de 2026-07-18 refeita na 2.5.
pub(crate) fn tick_updaterecord() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FEITO: AtomicBool = AtomicBool::new(false);
    static INICIO: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    if FEITO.load(Ordering::Relaxed) || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("tdb-updaterecord-teste") {
        return;
    }
    if !crate::selftest::trava_de_save_confirmada() || !crate::inicio::boot::gameplay_real() || INICIO.get_or_init(std::time::Instant::now).elapsed().as_secs() < 35 {
        return;
    }
    FEITO.store(true, Ordering::Relaxed);
    let linha = unsafe { rodar_updaterecord() };
    crate::log_veredito(&linha);
}

unsafe fn rodar_updaterecord() -> String {
    let (Some(t), Some(reg)) = (singleton(), crate::rtti::Registry::obtain()) else {
        return "[tdbu] VEREDITO-TDBU sem TweakDB ou registry; updaterecord-ok=false".into();
    };
    let record = tweak_db_id("Items.GrenadeIncendiarySticky");
    let flat = tweak_db_id("Items.GrenadeIncendiarySticky.deepWaterDepth");
    let instancia = super::get_record_instance(t, record);
    let getter = crate::rtti::resolve_func(&reg, "gamedataGrenade_Record", "DeepWaterDepth");
    let ler_getter = || -> Option<f32> {
        let rf = getter.as_ref()?;
        let b = crate::rtti::call_func(rf, instancia as *mut core::ffi::c_void, &[])?;
        Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let versao = || *t.add(super::VERSION_BYTE_OFF);
    let Some(off0) = offset(t as *const u8, flat) else {
        return "[tdbu] VEREDITO-TDBU flat deepWaterDepth não achado; updaterecord-ok=false".into();
    };
    let antes = ler_getter();
    let novo = if antes == Some(5.5) { 6.5f32 } else { 5.5 };
    let mut dados = [0u8; 8];
    dados[..4].copy_from_slice(&novo.to_le_bytes());
    let Some(off_novo) = super::create_flat_value_by_id(t, flat, &dados, 8) else {
        return "[tdbu] VEREDITO-TDBU valor novo não criado; updaterecord-ok=false".into();
    };
    let v0 = versao();
    let reapontou = super::set_flat_offset_by_id(t, flat, off_novo as u32);
    // Controle: sem a subida, o campo do record ainda tem o offset velho em cache (o getter devolve o valor de antes).
    let sem_update = ler_getter();
    let atualizou = super::update_record_by_id(record);
    let v1 = versao();
    let depois = ler_getter();
    let voltou = super::set_flat_offset_by_id(t, flat, off0);
    let atualizou_volta = super::update_record_by_id(record);
    let volta = ler_getter();
    let ok = !instancia.is_null() && antes.is_some() && reapontou && atualizou && depois == Some(novo) && voltou && atualizou_volta && volta == antes;
    format!(
        "[tdbu] VEREDITO-TDBU DeepWaterDepth() {antes:?}; flat re-apontado sem subir a versão {reapontou} -> getter {sem_update:?} (controle, \
         esperado o de antes); UpdateRecord {atualizou}, versão +0x160 {v0:#04x} -> {v1:#04x} -> getter {depois:?} (esperado {novo}); volta \
         {voltou}/{atualizou_volta} -> {volta:?}; updaterecord-ok={ok}"
    )
}

pub(crate) fn tick() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FEITO: AtomicBool = AtomicBool::new(false);
    static INICIO: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    if FEITO.load(Ordering::Relaxed) || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("tdb-escritor-teste") {
        return;
    }
    if !crate::selftest::trava_de_save_confirmada() || !crate::inicio::boot::gameplay_real() || INICIO.get_or_init(std::time::Instant::now).elapsed().as_secs() < 30 {
        return;
    }
    FEITO.store(true, Ordering::Relaxed);
    let linha = unsafe { rodar() };
    crate::log_veredito(&linha);
}

unsafe fn rodar() -> String {
    let Some(t) = singleton() else { return "[tdbw] VEREDITO-TDBW sem TweakDB; escritor-ok=false".into() };
    let t = t as *const u8;
    let alvo = CANDIDATOS.iter().find_map(|nome| {
        let id = tweak_db_id(nome);
        let off = offset(t, id)?;
        let viz = vizinho_de_valor(t, id, off)?;
        Some((*nome, id, off, viz))
    });
    let Some((nome, f1, off0, f2)) = alvo else {
        return "[tdbw] VEREDITO-TDBW nenhum candidato dividindo valor com outro flat; escritor-ok=false".into();
    };
    let tipo = tipo_do_valor(t, f1).unwrap_or_default();
    let Some(n) = largura_do_tipo(&tipo) else {
        return format!("[tdbw] VEREDITO-TDBW '{nome}' tipo '{tipo}' não escalar; escritor-ok=false");
    };
    let mascara = if n == 8 { u64::MAX } else { (1u64 << (8 * n)) - 1 };
    let (Some(v1_0), Some(v2_0)) = (valor(t, f1, mascara), valor(t, f2, mascara)) else {
        return format!("[tdbw] VEREDITO-TDBW '{nome}' valores ilegíveis; escritor-ok=false");
    };
    // Valor novo do mesmo tipo e diferente do atual: pra TweakDBID, outra qualidade; pro resto, o bit mais baixo trocado.
    let novo = if tipo == "TweakDBID" {
        let lendario = tweak_db_id("Quality.Legendary");
        if v1_0 == lendario { tweak_db_id("Quality.Epic") } else { lendario }
    } else {
        v1_0 ^ 1
    };
    let gravou = gravar_escalar(f1, novo);
    let (off1, off2) = (offset(t, f1), offset(t, f2));
    let (v1, v2) = (valor(t, f1, mascara), valor(t, f2, mascara));
    let anotado = offset_original(f1);
    let desfez = desfazer(f1);
    let (off1_volta, v1_volta) = (offset(t, f1), valor(t, f1, mascara));
    let ok = gravou
        && v1 == Some(novo)
        && v2 == Some(v2_0)
        && off1 != Some(off0)
        && off2 == Some(off0)
        && anotado == Some(off0)
        && desfez
        && off1_volta == Some(off0)
        && v1_volta == Some(v1_0);
    format!(
        "[tdbw] VEREDITO-TDBW '{nome}' ({tipo}, {n} B) divide o valor em {off0:#x} com o flat {f2:#x}; gravou {gravou}: '{nome}' {v1_0:#x} -> \
         {v1:x?} (offset {off1:x?}), vizinho {v2_0:#x} -> {v2:x?} (offset {off2:x?}); diário {anotado:x?}; desfez {desfez}: offset {off1_volta:x?}, \
         valor {v1_volta:x?}; escritor-ok={ok}"
    )
}

const PADROES: &str = "Modifiers.BwmsTestePadroes270:\n  $type: ConstantStatModifier\n  value: 7\n";

/// Padrões do tipo (`~/.bwms-hook-tdb-padroes-teste`): um record de `$type` sem `$base` nasce com o valor padrão do tipo em cada flat (o
/// `defaultValues` do TweakDB), e não com os valores do record vivo da classe usado como estrutura; o flat que o arquivo define fica com o
/// valor do arquivo. Controle: o modelo tem pelo menos um flat fora do padrão, senão o teste não diria nada.
pub(crate) fn tick_padroes() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FASE: AtomicU64 = AtomicU64::new(0);
    static INICIO: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let fase = FASE.load(Ordering::Relaxed);
    if fase >= 2 || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("tdb-padroes-teste") {
        return;
    }
    if !crate::selftest::trava_de_save_confirmada() || !crate::inicio::boot::gameplay_real() || INICIO.get_or_init(std::time::Instant::now).elapsed().as_secs() < 50 {
        return;
    }
    let arquivo = std::env::temp_dir().join("bwms-tdb-teste").join("padroes.yaml");
    unsafe {
        let Some(t) = singleton() else { return };
        if fase == 0 {
            let _ = std::fs::create_dir_all(arquivo.parent().unwrap_or(std::path::Path::new("/tmp")));
            let _ = std::fs::write(&arquivo, PADROES);
            super::aplicar_arquivo(&arquivo.to_string_lossy());
            FASE.store(1, Ordering::Relaxed);
            return;
        }
        if INICIO.get().is_some_and(|i| i.elapsed().as_secs() < 56) {
            return;
        }
        FASE.store(2, Ordering::Relaxed);
        let _ = std::fs::remove_file(&arquivo);
        let classe_nome = "gamedataConstantStatModifier_Record";
        let novo = tweak_db_id("Modifiers.BwmsTestePadroes270");
        let rec = super::get_record_instance(t, novo);
        let classe = if rec.is_null() {
            String::new()
        } else {
            crate::cname::resolve_cname(crate::rtti::type_name_hash(crate::rtti::class_of(rec as *mut core::ffi::c_void)))
        };
        let modelo = crate::rtti::Registry::obtain()
            .and_then(|reg| super::get_records_of_class(t, &reg, classe_nome, usize::MAX))
            .and_then(|v| v.iter().map(|(id, _)| *id).find(|id| tweak_db_id_chave(*id) != novo));
        let valor = super::get_flat_value_by_id(t as *const u8, tweak_db_id("Modifiers.BwmsTestePadroes270.value"))
            .map(|fv| (fv.add(0x08) as *const f32).read_unaligned())
            .unwrap_or(f32::NAN);
        let (mut outros, mut no_padrao, mut modelo_fora) = (0, 0, 0);
        let mut sem_padrao = Vec::new();
        if !rec.is_null() {
            for g in super::enum_record_getters(crate::rtti::class_of(rec as *mut core::ffi::c_void)) {
                let Some((nome, en)) = super::resolve_getter_flat_by_id(t, novo, &g) else { continue };
                if nome == "value" {
                    continue;
                }
                outros += 1;
                let flat = tweak_db_id_chave(en.read_unaligned());
                let padrao = super::escritor::tipo_do_valor(t as *const u8, flat).and_then(|tipo| super::escritor::offset_do_padrao(t as *const u8, &tipo));
                if padrao == Some(super::entry_tdb_offset(en.read_unaligned())) {
                    no_padrao += 1;
                } else {
                    sem_padrao.push(nome.clone());
                }
                let fora = modelo.and_then(|m| super::resolve_getter_flat_by_id(t, tweak_db_id_chave(m), &g)).map(|(_, em)| super::entry_tdb_offset(em.read_unaligned()));
                if padrao.is_some() && fora.is_some() && fora != padrao {
                    modelo_fora += 1;
                }
            }
        }
        let ok = classe == classe_nome && valor == 7.0 && outros > 0 && no_padrao == outros && modelo_fora > 0;
        crate::log_veredito(&format!(
            "[tdbp] VEREDITO-TDBP record de $type '{classe}', value {valor} (do arquivo), {no_padrao} de {outros} outros flats no padrão do tipo \
             {sem_padrao:?}, modelo {modelo:?} com {modelo_fora} flat(s) fora do padrão; padroes-ok={ok}"
        ));
    }
}

/// Remoção de record (`~/.bwms-hook-tdb-remover-teste`): um clone de mod sai dos dois mapas do TweakDB (o `RecordExists` e o `GetRecord` do motor
/// deixam de achar, o array do tipo perde um), o registro esquece a genealogia, record do jogo é recusado, e o mesmo id volta a ser criado e
/// removido (o nó livre do mapa por id é reusado).
pub(crate) fn tick_remover() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FEITO: AtomicBool = AtomicBool::new(false);
    static INICIO: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    if FEITO.load(Ordering::Relaxed) || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("tdb-remover-teste") {
        return;
    }
    if !crate::selftest::trava_de_save_confirmada() || !crate::inicio::boot::gameplay_real() || INICIO.get_or_init(std::time::Instant::now).elapsed().as_secs() < 40 {
        return;
    }
    FEITO.store(true, Ordering::Relaxed);
    unsafe {
        let (Some(t), Some(reg)) = (singleton(), crate::rtti::Registry::obtain()) else { return };
        let id = super::registro::nomear("Items.BwmsRemoverTeste273");
        let base = tweak_db_id("Items.Preset_Lexington_Default");
        let existe = |x: u64| super::record_existe(x);
        let criar = || super::clone_record_by_id(&reg, id, base) && {
            super::registro::clonado(id, base);
            true
        };
        let criou = criar();
        let rec = super::get_record_instance(t, id) as u64;
        let (existe_antes, n_antes) = (existe(id), super::remover::contagem_do_tipo(rec));
        let removeu = super::remover::remover_record(id);
        let (existe_depois, get_depois) = (existe(id), super::get_record_instance(t, id).is_null());
        let n_depois = super::remover::contagem_do_tipo(super::get_record_instance(t, base) as u64);
        let esquecido = super::registro::original(id);
        let recusou_vanilla = super::remover::remover_record(base).is_err() && existe(base);
        let recriou = criar() && existe(id);
        let n_recriado = super::remover::contagem_do_tipo(super::get_record_instance(t, id) as u64);
        let removeu2 = super::remover::remover_record(id).is_ok() && !existe(id);
        let ok = criou
            && existe_antes
            && removeu.is_ok()
            && !existe_depois
            && get_depois
            && n_antes.is_some()
            && n_depois == n_antes.map(|n| n - 1)
            && esquecido
            && recusou_vanilla
            && recriou
            && n_recriado == n_antes
            && removeu2;
        crate::log_veredito(&format!(
            "[tdbx] VEREDITO-TDBX clone criado {criou}, existe {existe_antes}, array do tipo {n_antes:?}; remoção {removeu:?}: RecordExists {existe_depois}, \
             GetRecord nulo {get_depois}, array do tipo {n_depois:?}, genealogia esquecida {esquecido}; record do jogo recusado {recusou_vanilla}; \
             recriado {recriou} (array {n_recriado:?}) e removido de novo {removeu2}; remover-ok={ok}"
        ));
    }
}

/// Os `n` bytes de baixo de um valor de flat.
fn mascara(n: usize) -> u64 {
    if n >= 8 { u64::MAX } else { (1u64 << (8 * n)) - 1 }
}

/// O valor que o `flatconf` espera num flat escalar do tipo `tipo`: o texto convertido pela regra do escritor (`escritor::escalar_do_texto`, a
/// mesma dos arquivos de tweak), na largura do tipo. `None` pra tipo que não é escalar ou texto que não serve pro tipo.
fn esperado_do_texto(tipo: &str, texto: &str) -> Option<u64> {
    let n = largura_do_valor(tipo)?;
    super::escritor::escalar_do_texto(tipo, texto.trim().trim_matches('"')).map(|v| v & mascara(n))
}

/// O valor no texto do veredito: `Float` com o número e os bits, o resto em hexadecimal.
fn descrever(tipo: &str, v: u64) -> String {
    if tipo == "Float" { format!("{} ({v:#x})", f32::from_bits(v as u32)) } else { format!("{v:#x}") }
}

/// `flatconf <flat> <esperado>` (canal `/tmp/cp77-cmd.txt`): o valor vivo de um flat escalar contra o texto esperado, numa linha do canal de
/// veredito (o `getflat` só fala pelo log amostrado). Confere o que um arquivo de tweak gravou; com arquivo ainda na fila (`em aplicação` > 0)
/// o veredito é cedo, não negativo.
pub(crate) unsafe fn conferir_flat(flat: &str, esperado: &str) -> String {
    let fila = super::arquivos_em_aplicacao();
    let Some(t) = singleton() else {
        return format!("[flatconf] VEREDITO {flat}: sem TweakDB; em aplicação {fila}; flat-ok=false");
    };
    let t = t as *const u8;
    let id = tweak_db_id(flat);
    let Some(tipo) = tipo_do_valor(t, id) else {
        return format!("[flatconf] VEREDITO {flat}: flat não achado (id {id:#x}); em aplicação {fila}; flat-ok=false");
    };
    if let Some(elem) = tipo.strip_prefix("array:") {
        return conferir_array(t, id, flat, elem, esperado, fila);
    }
    if tipo == "String" {
        let lido = super::get_flat_texto(t, id);
        let ok = lido.as_deref() == Some(esperado.trim().trim_matches('"'));
        return format!("[flatconf] VEREDITO {flat} (String): lido {lido:?}, esperado '{esperado}'; TweakDB {t:p}; em aplicação {fila}; flat-ok={ok}");
    }
    if matches!(tipo.as_str(), "Vector2" | "Vector3" | "Quaternion" | "EulerAngles" | "Color") {
        return conferir_struct(t, id, flat, &tipo, esperado, fila);
    }
    let Some(n) = largura_do_valor(&tipo) else {
        return format!("[flatconf] VEREDITO {flat}: tipo '{tipo}' sem conferência por texto; em aplicação {fila}; flat-ok=false");
    };
    let lido = valor(t, id, mascara(n));
    let quer = esperado_do_texto(&tipo, esperado);
    let ok = lido.is_some() && lido == quer;
    format!(
        "[flatconf] VEREDITO {flat} ({tipo}): lido {}, esperado '{esperado}' = {}; TweakDB {t:p}; em aplicação {fila}; flat-ok={ok}",
        lido.map_or_else(|| "ilegível".to_string(), |v| descrever(&tipo, v)),
        quer.map_or_else(|| "texto que não serve pro tipo".to_string(), |v| descrever(&tipo, v)),
    )
}

/// `flatconf` de um flat de struct: os bytes vivos (em `+8`, ou no alinhamento do tipo quando passa de 8, como o criador do motor
/// `0x1037f65c0` põe o `Quaternion` em `+0x10`) contra o esperado convertido pelo escritor (posicional ou nomeado, `{y:6.0}`).
unsafe fn conferir_struct(t: *const u8, id: u64, flat: &str, tipo: &str, esperado: &str, fila: usize) -> String {
    let Some((tam, alinh)) = super::escritor::elemento_do_tipo(tipo) else {
        return format!("[flatconf] VEREDITO {flat}: struct '{tipo}' sem layout; em aplicação {fila}; flat-ok=false");
    };
    let lido = get_flat_value_by_id(t, id).map(|fv| std::slice::from_raw_parts(fv.add(alinh.max(8)), tam).to_vec());
    let quer = super::escritor::bytes_do_texto(tipo, esperado);
    let ok = lido.is_some() && lido == quer;
    let texto = |b: &Option<Vec<u8>>| match (b, tipo) {
        (None, _) => "ilegível".to_string(),
        (Some(b), "Color") => format!("{b:?}"),
        (Some(b), _) => format!("{:?}", b.chunks(4).map(|c| f32::from_le_bytes(c.try_into().unwrap_or_default())).collect::<Vec<_>>()),
    };
    format!(
        "[flatconf] VEREDITO {flat} ({tipo}): lido {}, esperado '{esperado}' = {}; TweakDB {t:p}; em aplicação {fila}; flat-ok={ok}",
        texto(&lido),
        texto(&quer)
    )
}

/// Um elemento de array no texto do veredito: `Float` e `Int32` pelo número, `Bool` 0/1, `String` pelo texto, o resto em hexadecimal.
unsafe fn descrever_elemento(elem: &str, b: &[u8]) -> String {
    match (elem, b.len()) {
        ("Float", 4) => f32::from_le_bytes(b.try_into().unwrap_or_default()).to_string(),
        ("Int32", 4) => i32::from_le_bytes(b.try_into().unwrap_or_default()).to_string(),
        ("Bool", 1) => b[0].to_string(),
        ("String", _) => format!("{:?}", crate::jogo::cstring::texto(b.as_ptr())),
        _ => b.iter().rev().map(|x| format!("{x:02x}")).collect::<String>(),
    }
}

/// `flatconf` de um flat array: os elementos vivos contra a lista esperada (`[a, b]`), cada um convertido pela regra do escritor
/// (`escritor::elemento_em_bytes`) e comparado como o motor compara (`escritor::elementos_iguais`).
unsafe fn conferir_array(t: *const u8, id: u64, flat: &str, elem: &str, esperado: &str, fila: usize) -> String {
    let lidos = super::escritor::elementos_do_array(t, id).map(|(_, l)| l);
    let quer: Option<Vec<Vec<u8>>> = bwms_primitives::operacoes::texto_em_lista(esperado)
        .and_then(|l| l.iter().map(|s| super::escritor::elemento_em_bytes(elem, s)).collect());
    let ok = match (&lidos, &quer) {
        (Some(l), Some(q)) => l.len() == q.len() && l.iter().zip(q).all(|(a, b)| super::escritor::elementos_iguais(elem, a, b)),
        _ => false,
    };
    let texto = |v: &Option<Vec<Vec<u8>>>| {
        v.as_ref().map_or_else(|| "ilegível".to_string(), |l| format!("[{}]", l.iter().map(|b| descrever_elemento(elem, b)).collect::<Vec<_>>().join(", ")))
    };
    format!(
        "[flatconf] VEREDITO {flat} (array:{elem}): lido {}, esperado '{esperado}' = {}; TweakDB {t:p}; em aplicação {fila}; flat-ok={ok}",
        texto(&lidos),
        texto(&quer)
    )
}

/// Os TweakDBID que o flat `id` guarda: o próprio, num `TweakDBID`; os elementos, num `array:TweakDBID`.
unsafe fn ids_do_flat(t: *const u8, id: u64) -> Option<Vec<u64>> {
    match tipo_do_valor(t, id)?.as_str() {
        "TweakDBID" => valor(t, id, u64::MAX).map(|v| vec![v]),
        "array:TweakDBID" => super::escritor::elementos_do_array(t, id)
            .map(|(_, l)| l.iter().map(|b| u64::from_le_bytes(b.as_slice().try_into().unwrap_or_default())).collect()),
        _ => None,
    }
}

/// `refconf <flat> <classe1,classe2,…>`: cada record apontado pelo flat existe e é da classe da mesma posição (o nome do record em linha é
/// sintético, com o hash do arquivo; o registro o devolve no veredito). Responde como o `recconf`, para o roteiro contar junto.
pub(crate) unsafe fn conferir_referencias(flat: &str, classes: &str) -> String {
    let fila = super::arquivos_em_aplicacao();
    let Some(t) = singleton() else {
        return format!("[recconf] VEREDITO {flat}: sem TweakDB; em aplicação {fila}; record-ok=false");
    };
    let quer: Vec<&str> = classes.split(',').collect();
    let Some(ids) = ids_do_flat(t as *const u8, tweak_db_id(flat)) else {
        return format!("[recconf] VEREDITO {flat}: flat não achado ou não é TweakDBID; em aplicação {fila}; record-ok=false");
    };
    let lidos: Vec<String> = ids
        .iter()
        .map(|id| {
            let inst = super::get_record_instance(t, *id);
            let classe = if inst.is_null() {
                "(sem record)".to_string()
            } else {
                crate::cname::resolve_cname(crate::rtti::type_name_hash(crate::rtti::class_of(inst as *mut core::ffi::c_void)))
            };
            format!("{}={classe}", super::registro::nome(*id).unwrap_or_else(|| format!("{id:#x}")))
        })
        .collect();
    let ok = ids.len() == quer.len() && lidos.iter().zip(&quer).all(|(l, q)| l.ends_with(&format!("={q}")));
    format!("[recconf] VEREDITO {flat} -> {lidos:?} (esperadas {quer:?}); TweakDB {t:p}; em aplicação {fila}; record-ok={ok}")
}


/// `arrconf <flat> <tamanho> <índice> <record>`: o array de TweakDBID tem `tamanho` elementos e o `índice` aponta `record` (lista longa de mod
/// real, `!append-once` no `Vehicle.vehicle_list.list`, sem escrever as 802 entradas no esperado).
pub(crate) unsafe fn conferir_array_por_tamanho(flat: &str, tamanho: &str, indice: &str, record: &str) -> String {
    let fila = super::arquivos_em_aplicacao();
    let Some(t) = singleton() else {
        return format!("[flatconf] VEREDITO {flat}: sem TweakDB; em aplicação {fila}; flat-ok=false");
    };
    let ids = ids_do_flat(t as *const u8, tweak_db_id(flat)).unwrap_or_default();
    let no_indice = indice.parse::<usize>().ok().and_then(|i| ids.get(i).copied()).map(super::tweak_db_id_chave);
    let ok = tamanho.parse::<usize>().ok() == Some(ids.len()) && no_indice == Some(super::tweak_db_id_chave(tweak_db_id(record)));
    format!(
        "[flatconf] VEREDITO {flat}: {} elementos (esperado {tamanho}), [{indice}] = {} (esperado {record}); TweakDB {t:p}; em aplicação {fila}; flat-ok={ok}",
        ids.len(),
        no_indice.map_or_else(|| "nada".to_string(), |x| format!("{x:#x}"))
    )
}
/// `refflat <flat> <índice> <prop> <esperado>`: o `flatconf` da propriedade `prop` do record que o elemento `índice` do flat aponta (o nome
/// sai do registro, porque o record em linha tem nome sintético).
pub(crate) unsafe fn conferir_flat_referenciado(flat: &str, indice: &str, prop: &str, esperado: &str) -> String {
    let fila = super::arquivos_em_aplicacao();
    let alvo = singleton()
        .and_then(|t| ids_do_flat(t as *const u8, tweak_db_id(flat)))
        .and_then(|ids| ids.get(indice.parse::<usize>().ok()?).copied())
        .and_then(super::registro::nome);
    match alvo {
        Some(nome) => conferir_flat(&format!("{nome}.{prop}"), esperado),
        None => format!("[flatconf] VEREDITO {flat}[{indice}].{prop}: record apontado sem nome no registro; em aplicação {fila}; flat-ok=false"),
    }
}

/// `iconconf <record>`: o caminho que o jogo usa para o ícone do item (`"UIIcon." + iconPath`) chega a um `gamedataUIIcon_Record` vivo, e é
/// o mesmo record que a propriedade `.icon` aponta (o `icon:` em linha do arquivo de tweak).
pub(crate) unsafe fn conferir_icone(record: &str) -> String {
    let fila = super::arquivos_em_aplicacao();
    let Some(t) = singleton() else {
        return format!("[recconf] VEREDITO {record}.iconPath: sem TweakDB; em aplicação {fila}; record-ok=false");
    };
    let caminho = super::get_flat_texto(t as *const u8, tweak_db_id(&format!("{record}.iconPath"))).unwrap_or_default();
    let icone = tweak_db_id(&format!("UIIcon.{caminho}"));
    let inst = super::get_record_instance(t, icone);
    let classe = if inst.is_null() {
        String::new()
    } else {
        crate::cname::resolve_cname(crate::rtti::type_name_hash(crate::rtti::class_of(inst as *mut core::ffi::c_void)))
    };
    let apontado = ids_do_flat(t as *const u8, tweak_db_id(&format!("{record}.icon"))).and_then(|ids| ids.first().copied());
    let ok = !caminho.is_empty() && classe == "gamedataUIIcon_Record" && apontado.map(super::tweak_db_id_chave) == Some(super::tweak_db_id_chave(icone));
    format!(
        "[recconf] VEREDITO {record}.iconPath '{caminho}' -> UIIcon.{caminho} classe '{classe}', .icon aponta o mesmo {}; TweakDB {t:p}; em aplicação {fila}; record-ok={ok}",
        apontado.map(super::tweak_db_id_chave) == Some(super::tweak_db_id_chave(icone))
    )
}

/// `recconf <record> <classe>`: o record está no mapa do motor (`RecordExists`) e a instância é da classe pedida, pelo nome nativo
/// (`gamedataGrenade_Record`), numa linha do canal de veredito.
pub(crate) unsafe fn conferir_record(record: &str, classe: &str) -> String {
    let id = tweak_db_id(record);
    let existe = super::record_existe(id);
    let t = singleton();
    let inst = t.map_or(std::ptr::null_mut(), |t| super::get_record_instance(t, id));
    let lida = if inst.is_null() {
        String::new()
    } else {
        crate::cname::resolve_cname(crate::rtti::type_name_hash(crate::rtti::class_of(inst as *mut core::ffi::c_void)))
    };
    let ok = existe && lida == classe;
    format!(
        "[recconf] VEREDITO {record}: RecordExists {existe}, classe '{lida}' (esperada '{classe}'); TweakDB {t:?}; em aplicação {}; record-ok={ok}",
        super::arquivos_em_aplicacao()
    )
}

#[cfg(test)]
mod tests {
    use super::{descrever, esperado_do_texto};

    /// O esperado do `flatconf` sai pela conversão do escritor, na largura do tipo: o que o arquivo grava é o que o comando compara.
    #[test]
    fn flatconf_converte_o_esperado_pela_regra_do_escritor() {
        assert_eq!(esperado_do_texto("Float", "-0.25"), Some(0xbe80_0000));
        assert_eq!(esperado_do_texto("Float", "4.75f"), Some(0x4098_0000));
        assert_eq!(esperado_do_texto("Float", "0x3fc00000"), Some(0x3fc0_0000), "hexadecimal num Float são os bits, como no setflat");
        assert_eq!(esperado_do_texto("Float", "\"1.4\""), Some(1.4f32.to_bits() as u64));
        assert_eq!(esperado_do_texto("Int32", "-1"), Some(0xffff_ffff), "cortado na largura do Int32");
        assert_eq!(esperado_do_texto("Bool", "true"), Some(1));
        assert_eq!(esperado_do_texto("TweakDBID", "Quality.Rare"), Some(bwms_primitives::tweak_db_id("Quality.Rare")));
        // `Character.Judy_Puppet_Photomode.entityTemplatePath` no `tweakdb.bin` da 2.5 = 0x7b09932c9d2ed4bf; texto puro em `LocKey` = FNV-1a.
        assert_eq!(
            esperado_do_texto("raRef:CResource", r"base\characters\entities\player\photo_mode\judy_alvarez\judy_photomode.ent"),
            Some(0x7b09_932c_9d2e_d4bf)
        );
        assert_eq!(esperado_do_texto("gamedataLocKeyWrapper", "BwmsWkit"), Some(bwms_primitives::fnv1a64(b"BwmsWkit")));
        assert_eq!(esperado_do_texto("Float", "abc"), None);
        assert_eq!(esperado_do_texto("array:CName", "[a]"), None);
        assert_eq!(esperado_do_texto("String", "x"), None);
        assert_eq!(descrever("Float", 0xbe80_0000), "-0.25 (0xbe800000)");
        assert_eq!(descrever("TweakDBID", 0x2a), "0x2a");
    }
}
