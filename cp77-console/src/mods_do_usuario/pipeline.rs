//! Pipeline unificado de carga de mods — "6 serem 1" no runtime do BWMS.
//!
//! As 6 capacidades do Windows convergem aqui em 2 fases:
//!
//!   RED4ext    → plugins Rust de 3os (boot_phase)
//!   redscript  → compilado pela toolchain; motor carrega automaticamente
//!   CET        → overlay/console (overlay.rs, sempre ativo)
//!   TweakXL    → YAML/.tweak auto-apply dos mods ativos (session_phase) + ScriptableTweak dispatch (menu, depois dos arquivos; lib.rs)
//!   Codeware   → register::register_all (selfboot on_load — antes desta fase)
//!   ArchiveXL  → resource.link (boot_phase); o factory index é do jogo, pelo `.json` (`xl_no_jogo`)
//!
//! Chamadas de lib.rs:
//!   `boot_phase()`       — primeira iteração do tick, antes do gate de player
//!   `session_phase(reg)` — no timing do Session/Start (player presente, RTTI completo)

use bwms_primitives::tabelas;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static BOOT_DONE: AtomicBool = AtomicBool::new(false);

// `tweakxl-hot-reload`: polling a cada HOT_RELOAD_INTERVAL ticks (~10s).
static HOT_RELOAD_LAST_TICK: AtomicU64 = AtomicU64::new(0);
static HOT_RELOAD_LAST_MTIME: AtomicU64 = AtomicU64::new(0);
const HOT_RELOAD_INTERVAL: u64 = 600;

/// **Fase 1** — roda UMA VEZ na primeira iteração do tick (antes do gate de player).
///
/// - ArchiveXL: resource.link (bwms-reslink.txt, gravado dos `.xl` da pasta `Mods/` no `on_load`)
/// - RED4ext: plugins Rust de 3os (red4ext/plugins/*.dylib, opt-in)
pub(crate) fn boot_phase() {
    if BOOT_DONE.swap(true, Ordering::Relaxed) {
        return;
    }

    recarregar_tabelas_de_runtime(false);

    // --- RED4ext: plugins Rust de 3os --- (normalmente já carregados no bind; aqui só o que faltou)
    // Os `register_method` de plugin em classe de script drenam aqui: no 1º tick a classe existe (boots 46 e 271, `register_method
    // ExamplePluginFieldTest -> true`); no bind das funções, mesmo depois do `register_all`, ainda não (boots 272-274, `classe
    // 'PlayerPuppet' não existe`). Antes do `carregar_plugins`: plugin que só carrega aqui acha a classe viva e registra na hora.
    unsafe { crate::mods_do_usuario::api::registrar_metodos_adiados() };
    carregar_plugins();
}

/// Carrega os `.dylib` de `red4ext/plugins/` (sem pasta, nada). Chamado no bind dos scripts, logo depois do registro das natives do BWMS,
/// pra que a native que um plugin registra já exista quando o motor liga o `.reds` que a declara — carregando só no 1º tick, depois do
/// bind, o `.reds` do plugin derrubava o jogo (SIGTRAP de native declarada sem registro).
pub(crate) fn carregar_plugins() {
    if let Some(red4) = crate::dylib_dir() {
        crate::mods_do_usuario::plugins::load_plugins(&std::path::Path::new(&red4).join("plugins"));
    }
}

/// Carga das tabelas que o boot grava dos `.xl` da `Mods/` (`pasta_mods::preparar_no_boot`; `bwms-reslink.txt` e as das seções de `carregar_tabelas_do_archivexl`): no boot
/// (`boot_phase`, 1×, atrás do guard `BOOT_DONE`) e no `ArchiveXL.Reload()` (`PENDENCIAS-UNIFICADAS.md`, Facade.hpp; `tramp_recarregar_tabelas_de_runtime`), sem
/// rebootar o jogo. Os hooks têm guarda própria de "só instala 1x" (`install_reslink`). O `factories` não tem tabela: o jogo o aplica pelo `.json`
/// (`xl_no_jogo`).
///
/// `substituir` (só o `ArchiveXL.Reload`): cada tabela em memória vira a do arquivo, e o arquivo ausente a esvazia, porque a gravação o apaga
/// quando nenhum mod gera linha. Saem os pares de `RegisterLink`, como no `Configure` da referência (`ResourceLink/Extension.cpp:38-40`); do
/// journal fica, além do que o arquivo lista, só o já mesclado na árvore viva.
/// Sem `substituir` (o boot) o que já está na memória fica: a memória do processo começa limpa, e antes desta carga o `on_load` já gravou os pares
/// de teste do `#11`/`#47a`, e o `OnLoad` de um `ScriptableService` (chamado pelo contêiner do jogo) pode ter gravado pares de `RegisterLink`.
pub(crate) fn recarregar_tabelas_de_runtime(substituir: bool) {
    if let Some(red4) = crate::dylib_dir() {
        carregar_tabelas_do_archivexl(std::path::Path::new(&red4), substituir);
    }

    // --- ArchiveXL #51 (51j-b): reaplica os patches de `persistentStateData` quando a tabela tem patch; os marcadores de sonda e teste são do dev ---
    unsafe { crate::arquivos_do_jogo::preparar_hook_de_psd() };
}

/// As tabelas de `red4` (a pasta `red4ext/`, onde o boot as grava), com a regra de `substituir` de `recarregar_tabelas_de_runtime`.
fn carregar_tabelas_do_archivexl(red4: &std::path::Path, substituir: bool) {
    // --- ArchiveXL: resource.link ---
    // bwms-reslink.txt é gerado por bwms_primitives::instalacao::tabelas_de_runtime::emit_reslink_do_plano a partir das
    // seções `resource.link` dos .xl ativos. O hook é idempotente (só instala 1x).
    let f = red4.join(tabelas::RESLINK.arquivo);
    if f.is_file() {
        unsafe { crate::arquivos_do_jogo::install_reslink() };
    }
    if f.is_file() || substituir {
        crate::arquivos_do_jogo::reslink_file(&f.to_string_lossy(), substituir);
        crate::log_dev!("[pipeline] ArchiveXL resource.link: tabela carregada");
    }

    // --- ArchiveXL: streaming.sectors (deleções de node, `#49` parte 4d) ---
    // bwms-streaming.txt gerado por bwms_primitives::instalacao::tabelas_de_runtime::emit_streaming_table. O hook do `PostLoad` do setor instala no on_load quando o
    // arquivo existe.
    let f = red4.join(tabelas::STREAMING.arquivo);
    if f.is_file() || substituir {
        crate::mundo::streaming_file(&f.to_string_lossy());
    }

    // --- ArchiveXL: quest.phases (patch de fase na carga, `#35` parte 35c-2d-3) ---
    // bwms-questphase.txt gerado por bwms_primitives::instalacao::tabelas_de_runtime::emit_questphase_table. O contexto do patch é resolvido aqui (game thread) e o hook
    // da rota de carga de fase instala quando a tabela tem linha.
    let f = red4.join(tabelas::QUESTPHASE.arquivo);
    if (f.is_file() || substituir) && unsafe { crate::mundo::questphase_tabela(&f.to_string_lossy()) } > 0 {
        unsafe { crate::missoes_e_mapa::install_questphase_carga_probe(true) };
        unsafe { crate::missoes_e_mapa::install_a34() };
        unsafe { crate::missoes_e_mapa::install_questsys_ongamerestored() };
        unsafe { crate::missoes_e_mapa::install_a35_recriar() };
    }

    // --- ArchiveXL: journal (journals de mod, `#44` parte 7e-2) ---
    // bwms-journal.txt gerado por bwms_primitives::instalacao::tabelas_de_runtime::emit_journal_table. Aqui entram a lista e, com linha na tabela, os hooks de busca e
    // reserva de mappins (7e-5, antes de qualquer carga de mundo); a carga e a mescla na árvore viva rodam em gameplay (`register::journal_mods_tick`).
    let f = red4.join(tabelas::JOURNAL.arquivo);
    if (f.is_file() || substituir) && crate::mundo::journal_tabela(&f.to_string_lossy(), substituir) > 0 {
        unsafe { crate::missoes_e_mapa::instalar_hooks_de_pins_de_mod() };
        // 7e-6b: classes do journal guardadas aqui (game thread) pro hook do `Initialize` da raiz, que roda em thread de job.
        unsafe { crate::mundo::journal_classes_guardar() };
        // Teste da 7e-6b: com `~/.bwms-hook-journal-arquivo-menu`, o archive nomeado no marcador entra em runtime já na 1ª volta do tick (menu), fora do
        // glob do jogo, pra o journal de mod existir antes da carga do save.
        #[cfg(feature = "devtools")]
        if let Some(texto) = crate::caminho_de_marcador(".bwms-hook-journal-arquivo-menu").and_then(|p| std::fs::read_to_string(p).ok()) {
            let caminho = texto.trim();
            if !caminho.is_empty() {
                let r = unsafe { crate::arquivos_do_jogo::facade_register_archive(caminho) };
                crate::veredito_dev!("[a44-7e6] archive de teste registrado na carga das tabelas: '{caminho}' -> {r}");
            }
        }
    }

    // --- ArchiveXL: resource.patch nas partes de aparência (`#17` 17d) ---
    // bwms-resource-patch.txt gerado por bwms_primitives::instalacao::tabelas_de_runtime::emit_resource_patch_table. Aqui (game thread) entram as classes da mescla, o
    // token de cada patch e o hook do `ExtractPartComponents`; a mescla roda no hook, a cada extração de parte alvo.
    unsafe { carregar_resource_patch(red4) };

    // --- ArchiveXL: resource.fix (nomes de material do mesh, `#51` 51i-1) ---
    // bwms-resfixes.txt gerado por bwms_primitives::instalacao::tabelas_de_runtime::emit_resfixes. Com linha `names` na tabela (ou `~/.bwms-hook-a51i-teste`), os
    // offsets do `CMesh` saem do RTTI e o hook do `PostLoad` instala; a troca de nomes roda no hook. As linhas `paths` servem ao `FixCustomizationOptions`
    // do `#47` (caminho do recurso de aparência de uma opção de customização) e entram fora da conferência de offsets do `CMesh`. Uma leitura e um leitor
    // (`resfixes_linhas`) servem as 2 tabelas, então as 2 seguem a mesma regra.
    let f = red4.join(tabelas::RESFIXES.arquivo);
    match std::fs::read_to_string(&f) {
        Ok(texto) => {
            let (nomes, caminhos) = resfixes_linhas(&texto);
            unsafe { crate::aparencia::carregar_fixes_de_nome_do_mesh(nomes, substituir) };
            crate::corpo_e_rosto::cust_fix_caminhos_carregar(caminhos);
            unsafe { crate::aparencia::carregar_contextos_do_mesh(resfixes_contexto(&texto), substituir) };
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            unsafe { crate::aparencia::carregar_fixes_de_nome_do_mesh(HashMap::new(), substituir) };
            unsafe { crate::aparencia::carregar_contextos_do_mesh(HashMap::new(), substituir) };
            if substituir {
                crate::corpo_e_rosto::cust_fix_caminhos_carregar(HashMap::new());
            }
        }
        // Erro que não é de arquivo ausente (permissão, disco que caiu): as 2 tabelas ficam como a carga anterior as deixou, porque esvaziar por erro
        // transitório tiraria o fix até o próximo `Reload`. Sem `substituir`, o marcador de teste ainda liga o hook.
        Err(e) => {
            crate::log_dev!("[a51i] não leu '{}': {e}; tabelas do resource.fix mantidas", f.display());
            unsafe { crate::aparencia::carregar_fixes_de_nome_do_mesh(HashMap::new(), false) };
        }
    }

    // --- ArchiveXL: customizations (`#47` 47c) ---
    // bwms-customizations.txt gerado por bwms_primitives::instalacao::tabelas_de_runtime::emit_customization_table. Aqui (game thread) o token de cada recurso é pedido e
    // os hooks do fluxo de edição instalam; a mescla roda no hook do `Initialize` do `CharacterCustomizationSystem`, na abertura do criador ou do espelho.
    // Sem o arquivo, a carga desfaz a tabela anterior.
    if let Ok(texto) = std::fs::read_to_string(red4.join(tabelas::CUSTOMIZATIONS.arquivo)) {
        // Teste 47d: com `~/.bwms-hook-a47d-arquivo`, o archive nomeado no marcador entra por `RegisterArchive` antes da tabela, pra o recurso de
        // mod existir quando a tabela o pedir.
        #[cfg(feature = "devtools")]
        if let Some(caminho) = crate::caminho_de_marcador(".bwms-hook-a47d-arquivo").and_then(|p| std::fs::read_to_string(p).ok()) {
            let caminho = caminho.trim();
            if !caminho.is_empty() {
                let r = unsafe { crate::arquivos_do_jogo::facade_register_archive(caminho) };
                crate::veredito_dev!("[a47d] archive de teste registrado antes da tabela de customizations: '{caminho}' -> {r}");
            }
        }
        unsafe { crate::corpo_e_rosto::cust_tabela(&texto) };
    } else {
        unsafe { crate::corpo_e_rosto::cust_tabela_ausente() };
    }

    // --- ArchiveXL: resource.scope em runtime (`#20` 20e-2, o `InScope` do `FixCustomizationAppearance`) ---
    // bwms-resscopes.txt gerado por bwms_primitives::instalacao::tabelas_de_runtime::emit_resscopes (os escopos de todos os .xl, expandidos).
    // Só o conserto de aparência do selftest consulta o escopo em runtime; no público os escopos valem só no install (link e cópia expandidos).
    #[cfg(any(test, feature = "devtools"))]
    match std::fs::read_to_string(red4.join(tabelas::RESSCOPES.arquivo)) {
        Ok(texto) => {
            let n = crate::corpo_e_rosto::es20_carregar_escopos(&texto);
            crate::log_dev!("[a20e2] escopos de recurso: {n} par(es) de bwms-resscopes.txt");
        }
        Err(e) if substituir && e.kind() == std::io::ErrorKind::NotFound => {
            crate::corpo_e_rosto::es20_carregar_escopos("");
        }
        Err(_) => {}
    }
}

/// Linhas de `bwms-resfixes.txt` (`<recurso> | names|paths | <antigo> | <novo>`, `bwms_primitives::tabelas::Resfix`; linha fora do formato fica de fora)
/// em `(nomes, caminhos)`, cada um `recurso → [(antigo, novo)]` em hash: o nome de material em `CName` (`None` = 0), o caminho em `ResourcePath`. Nos
/// dois, o mesmo antigo no mesmo recurso fica com o novo da última linha, a regra do `Merge` da referência (`ResourceMeta/Config.hpp:41-57`, chamado
/// por `.xl` em `ResourceMeta/Extension.cpp:36-38`): o install concatena as linhas de todos os `.xl`, e 2 mods podem consertar o mesmo antigo. Caminho
/// novo vazio não entra, e o par da linha anterior fica; a referência grava 0 e o último vence (divergência registrada em `caminhos/xl.md`).
pub(crate) fn resfixes_linhas(texto: &str) -> (HashMap<u64, Vec<(u64, u64)>>, HashMap<u64, Vec<(u64, u64)>>) {
    use tabelas::{Resfix, TipoDeFix};
    let (mut nomes, mut caminhos): (HashMap<u64, Vec<(u64, u64)>>, HashMap<u64, Vec<(u64, u64)>>) = (HashMap::new(), HashMap::new());
    let caminho = bwms_primitives::resource_path_hash;
    for f in tabelas::ler(texto, Resfix::ler).flatten() {
        let (mapa, antigo, novo) = match f.tipo {
            TipoDeFix::Nomes => (&mut nomes, crate::texto_e_numeros::cname_ou_zero(f.antigo), crate::texto_e_numeros::cname_ou_zero(f.novo)),
            TipoDeFix::Caminhos if !f.novo.is_empty() => (&mut caminhos, caminho(f.antigo), caminho(f.novo)),
            TipoDeFix::Caminhos => continue,
            // O `context` vai ao estado de mesh nativo (`resfixes_contexto`, `aparencia::injetar_contexto_do_mesh`), não a esta tabela de trocas.
            TipoDeFix::Contexto => continue,
        };
        let pares = mapa.entry(caminho(f.recurso)).or_default();
        match pares.iter_mut().find(|p| p.0 == antigo) {
            Some(p) => p.1 = novo,
            None => pares.push((antigo, novo)),
        }
    }
    (nomes, caminhos)
}

/// As linhas `context` de `bwms-resfixes.txt` (S12): caminho do mesh → `(nome do atributo em snake_case, valor)`. O mesmo parâmetro no mesmo mesh
/// fica com o valor da última linha (o `Merge` da referência, `ResourceMeta/Config.hpp:53-55`); duas chaves cruas que dão o mesmo nome ficam com a
/// 1ª, como o `emplace` do `PrefillContext`.
pub(crate) fn resfixes_contexto(texto: &str) -> HashMap<u64, Vec<(String, String)>> {
    use tabelas::{Resfix, TipoDeFix};
    let mut por_mesh: HashMap<u64, Vec<(String, String, String)>> = HashMap::new();
    for f in tabelas::ler(texto, Resfix::ler).flatten().filter(|f| f.tipo == TipoDeFix::Contexto) {
        let pares = por_mesh.entry(bwms_primitives::resource_path_hash(f.recurso)).or_default();
        match pares.iter_mut().find(|p| p.0 == f.antigo) {
            Some(p) => p.2 = f.novo.to_string(),
            None => pares.push((f.antigo.to_string(), tabelas::nome_de_atributo(f.antigo), f.novo.to_string())),
        }
    }
    por_mesh
        .into_iter()
        .map(|(mesh, pares)| {
            let mut vistos = std::collections::HashSet::new();
            (mesh, pares.into_iter().filter(|(_, nome, _)| vistos.insert(nome.clone())).map(|(_, nome, valor)| (nome, valor)).collect())
        })
        .collect()
}

/// `bwms-resource-patch.txt` de `red4` na tabela do `#17` (game thread).
unsafe fn carregar_resource_patch(red4: &std::path::Path) {
    let f = red4.join(tabelas::RESOURCE_PATCH.arquivo);
    #[cfg(feature = "devtools")]
    if f.is_file() {
        // Teste da 51e: com `~/.bwms-hook-arquivo-teste`, o archive nomeado no marcador entra por `RegisterArchive` antes da tabela, pra o token do
        // recurso de mod (um block) existir quando a tabela pedir.
        if let Some(texto) = crate::caminho_de_marcador(".bwms-hook-arquivo-teste").and_then(|p| std::fs::read_to_string(p).ok()) {
            let caminho = texto.trim();
            if !caminho.is_empty() {
                let r = crate::arquivos_do_jogo::facade_register_archive(caminho);
                crate::veredito_dev!("[a51e] archive de teste registrado antes da tabela de resource.patch: '{caminho}' -> {r}");
            }
        }
    }
    // Arquivo ausente também carrega: a gravação do boot o apaga quando nenhum mod gera linha, e a tabela da carga anterior tem de sair.
    crate::arquivos_do_jogo::carregar_tabela_de_patches(&f.to_string_lossy());
}

/// **Fase 2** — roda no timing do Session/Start (player presente, RTTI completo).
///
/// - TweakXL: auto-apply dos tweaks da pasta `Mods/` (`apply_mod_tweaks`)
/// - (o `OnApply` dos `ScriptableTweak` sai do `cp77_tick` depois destes arquivos, ver [`arquivos_da_sessao_aplicados`])
pub(crate) unsafe fn session_phase(reg: &crate::rtti::Registry) {
    let _ = reg;

    // --- TweakXL: auto-apply YAML dos mods ativos ---
    apply_mod_tweaks();
    ARQUIVOS_DA_SESSAO.store(true, Ordering::Release);
}

/// O 1º `apply_mod_tweaks` do processo já pôs os arquivos de tweak na fila (o contador `tweakdb::arquivos_em_aplicacao` sobe na mesma
/// chamada).
static ARQUIVOS_DA_SESSAO: AtomicBool = AtomicBool::new(false);

/// Os arquivos de tweak da 1ª sessão (a do menu, quando o jogador do menu aparece antes do load) foram aplicados: o `OnApply` vem depois
/// deles, na ordem do TweakXL de referência (`ImportTweaks` e depois `ExecuteTweaks`, no `TweakDB_TryLoad`).
pub(crate) fn arquivos_da_sessao_aplicados() -> bool {
    ARQUIVOS_DA_SESSAO.load(Ordering::Acquire) && crate::tweakdb::arquivos_em_aplicacao() == 0
}

/// TweakDB em que o despacho do `OnApply` gravou, pra o veredito de gameplay conferir que é o mesmo.
pub(crate) static TWEAKDB_DO_ONAPPLY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// AL-2: a forja do `ScriptableTweak` leva a vtable de uma classe nativa (a doadora), cujo `ConstructCls` escreve até o tamanho dela. Com o
/// size mínimo (0x40) a subclasse reds do mod estourava o bloco no `CreateInstance` e o jogo caía depois do `OnApply` (`+0x1afd4`,
/// `proofs/20260925-al2-sem-promocao-crash-subclasse-scriptabletweak.ips`). `(tamanho da forja, tamanho da doadora)` quando a forja é menor.
unsafe fn forja_menor_que_a_doadora(reg: &crate::rtti::Registry) -> Option<(u32, u32)> {
    use std::ffi::c_void;
    let forja = reg.class_by_name("ScriptableTweak") as *const u8;
    if !crate::memoria::is_readable(forja as *const c_void, 0x78) {
        return None;
    }
    let vtable = (forja as *const u64).read_unaligned();
    let tamanho = (forja.add(0x68) as *const u32).read_unaligned();
    crate::register::DOADORAS_DA_FORJA_MINIMA.iter().find_map(|n| {
        let d = reg.class_by_name(n) as *const u8;
        if !crate::memoria::is_readable(d as *const c_void, 0x78) || (d as *const u64).read_unaligned() != vtable {
            return None;
        }
        let tamanho_da_doadora = (d.add(0x68) as *const u32).read_unaligned();
        (tamanho < tamanho_da_doadora).then_some((tamanho, tamanho_da_doadora))
    })
}

// ── Funções internas ──────────────────────────────────────────────────────────

/// Raiz do jogo (2 níveis acima da nossa dylib: red4ext/ → <jogo>/).
fn game_root() -> Option<std::path::PathBuf> {
    crate::dylib_dir()
        .map(std::path::PathBuf::from)
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
}

/// Recarrega os tweaks de mod sem empilhar: desfaz o que os arquivos gravaram (o diário do escritor do TweakDB), reaplica os arquivos de
/// agora e pede o redespacho dos ScriptableTweak. `!append` não duplica e linha apagada do arquivo volta ao valor de antes.
pub(crate) unsafe fn recarregar_tweaks() {
    let voltaram = crate::tweakdb::escritor::desfazer_arquivos();
    crate::log_dev!("[pipeline] recarga: {voltaram} flat(s) de arquivo desfeito(s) antes de reaplicar");
    apply_mod_tweaks();
    ONAPPLY_PENDENTE.store(true, Ordering::Release);
}

/// Depois do `TweakDB::Reload` do motor (thread de job): o estado do TweakDB velho é esquecido e, se a sessão já tinha aplicado os tweaks,
/// eles voltam por cima do TweakDB novo — os arquivos da `Mods/` agora e o `OnApply` pelo tick, como na recarga do BWMS, mas sem desfazer pelo
/// diário (o índice é outro).
pub(crate) fn depois_da_recarga_do_motor() {
    crate::tweakdb::esquecer_o_tweakdb_velho();
    if !ARQUIVOS_DA_SESSAO.load(Ordering::Acquire) {
        return;
    }
    crate::veredito_dev!("[pipeline] o motor recarregou o TweakDB: tweaks da sessão reaplicados = 1");
    apply_mod_tweaks();
    ONAPPLY_PENDENTE.store(true, Ordering::Release);
}

/// `OnApply` pedido pela recarga. Os arquivos reaplicam numa thread própria (`tweakdb::aplicar_conjunto`, 500 ms depois); despachar na hora
/// rodava o `OnApply` contra o TweakDB desfeito.
static ONAPPLY_PENDENTE: AtomicBool = AtomicBool::new(false);

/// Chamado pelo `cp77_tick` depois do 1º `OnApply`: despacha o da recarga quando as threads de arquivo terminaram, nunca com o
/// executor aninhado (a mesma regra do disparo de gameplay).
pub(crate) unsafe fn despachar_onapply_da_recarga(reg: &crate::rtti::Registry) {
    if crate::tweakdb::arquivos_em_aplicacao() != 0 || crate::inicio::boot::exec_nested() {
        return;
    }
    if ONAPPLY_PENDENTE.load(Ordering::Acquire) {
        crate::log_dev!("[pipeline] recarga: OnApply depois dos arquivos reaplicados");
        dispatch_scriptable_tweaks(reg);
    }
}

/// Aplica os tweaks da pasta `Mods/` (`pasta_mods::tweaks`, em ordem de caminho) num conjunto só pelo `tweakdb::aplicar_arquivos` (o
/// `.tweak` passa pelo `bwms_primitives::tweakxl::tweak_source::parse`).
pub(crate) fn apply_mod_tweaks() {
    let Some(root) = game_root() else { return };
    let da_pasta = crate::mods_do_usuario::pasta_mods::tweaks(&crate::mods_do_usuario::pasta_mods::pasta(&root));
    let caminhos: Vec<String> = da_pasta
        .iter()
        .map(|p| {
            crate::log_dev!("[pipeline] tweak da pasta Mods: {}", p.display());
            p.to_string_lossy().into_owned()
        })
        .collect();
    // Todos num conjunto só (o TweakXL junta os arquivos e aplica as mutações de array depois das atribuições de todos).
    crate::tweakdb::aplicar_arquivos(&caminhos);
    // O roteiro de prova lê daqui quando os tweaks da `Mods/` foram para a fila de aplicação (cada arquivo grava ~500 ms depois, na vez dele),
    // em que momento do boot e em que TweakDB: o `flatconf` de gameplay compara o ponteiro.
    if !da_pasta.is_empty() {
        crate::veredito_dev!(
            "[pipeline] pasta Mods: {} tweak(s) na fila de aplicação (TweakDB {:?}, phase 5 já visto {}, gameplay real {}), nesta ordem: {:?}",
            da_pasta.len(),
            unsafe { crate::tweakdb::singleton() },
            crate::inicio::boot::PHASE_REACHED_5.load(Ordering::Relaxed),
            crate::inicio::boot::gameplay_real(),
            da_pasta.iter().map(|p| p.strip_prefix(&root).unwrap_or(p).display().to_string()).collect::<Vec<_>>()
        );
        crate::log_dev!("[pipeline] TweakXL: {} arquivo(s) da pasta Mods", da_pasta.len());
    }
}

/// Enumera todas as subclasses concretas de ScriptableTweak no RTTI e chama
/// `OnApply()` em cada uma. Chamado pelo `cp77_tick` em 2 pontos: o 1º despacho (no menu depois dos arquivos de tweak, ou em gameplay quando
/// o load começou antes; lib.rs) e, depois dele, o redespacho da recarga (`despachar_onapply_da_recarga`, pedido pelo `recarregar_tweaks`).
pub(crate) unsafe fn dispatch_scriptable_tweaks(reg: &crate::rtti::Registry) {
    use std::ffi::c_void;
    // Um despacho atende a recarga pedida antes dele: sem isto, recarga no menu rodava o `OnApply` 2× no gameplay.
    ONAPPLY_PENDENTE.store(false, Ordering::Release);
    let st_cname = crate::cname::cname("ScriptableTweak");
    if reg.class_by_name("ScriptableTweak").is_null() {
        crate::log_dev!("[tweakxl] ScriptableTweak não no RTTI — dispatch pulado");
        return;
    }
    if let Some((forja, doadora)) = forja_menor_que_a_doadora(reg) {
        crate::log(&format!(
            "[tweakxl] ScriptableTweak forjada com {forja:#x} B, menor que os {doadora:#x} B que o construtor da doadora escreve: OnApply não roda"
        ));
        return;
    }
    if let Some(t) = crate::tweakdb::singleton() {
        TWEAKDB_DO_ONAPPLY.store(t as usize, Ordering::Release);
    }

    // GetAllTypes(this, DynArray<IRTTIType*>&) — slot +0x40 (provável; GetType@+0x38).
    // Se errado: log imprime count inválido e nada é chamado (sem crash).
    const GET_ALL_TYPES_SLOT: usize = 0x40;
    let fn_ptr = reg.vtbl_slot(GET_ALL_TYPES_SLOT);
    if fn_ptr.is_null() {
        crate::log_dev!("[tweakxl] GetAllTypes@vtbl+0x40 ilegível");
        return;
    }

    // DynArray<IRTTIType*>: ptr(u64)@0x00 + capacity(u32)@0x08 + size(u32)@0x0C (layout real,
    // confirmado por disassembly ARM64 em 2026-08-11). Ler `capacity` aqui (buf[8..12]) é CORRETO
    // porque este slot é GetNativeTypes/GetAllTypes especificamente, a ÚNICA família sem filtro —
    // cap==size sempre pra ela (confirmado: 27798==27798). As famílias COM filtro (`GetClasses`
    // etc., cap≠size) leem `size`: ver `rtti.rs::call_dynarray_out0`.
    let mut arr = [0u8; 16];
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(fn_ptr);
    f(reg.reg, arr.as_mut_ptr() as *mut c_void);

    let entries_raw = u64::from_le_bytes(arr[0..8].try_into().unwrap_or([0u8; 8]));
    let count = u32::from_le_bytes(arr[8..12].try_into().unwrap_or([0u8; 4])) as usize;
    let entries = entries_raw as *const *mut c_void;

    if count < 100 || count > 300_000 || entries.is_null()
        || !crate::memoria::is_readable(entries as *const c_void, 8)
    {
        crate::log_dev!(
            "[tweakxl] GetAllTypes slot+0x40 inválido (count={count} ptr={entries_raw:#x}) — slot errado?"
        );
        if crate::dev_mode() {
            for probe_off in [0x38usize, 0x48, 0x50, 0x58] {
                let pfp = reg.vtbl_slot(probe_off);
                if pfp.is_null() {
                    continue;
                }
                let mut pa = [0u8; 16];
                let pf: unsafe extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(pfp);
                pf(reg.reg, pa.as_mut_ptr() as *mut c_void);
                let pc = u32::from_le_bytes(pa[8..12].try_into().unwrap_or([0u8; 4])) as usize;
                let pe = u64::from_le_bytes(pa[0..8].try_into().unwrap_or([0u8; 8]));
                crate::log_dev!(
                    "[tweakxl] probe vtbl+{probe_off:#x}: count={pc} ptr={pe:#x}"
                );
            }
        }
        return;
    }
    crate::log_dev!(
        "[tweakxl] GetAllTypes: {count} tipos, varrendo ScriptableTweak"
    );

    let mut dispatched = 0u32;
    for i in 0..count {
        let ty = *entries.add(i);
        if ty.is_null() || !crate::memoria::is_readable(ty as *const c_void, 0x80) {
            continue;
        }
        let clsb = ty as *const u8;

        // CClass+0x70 = flags; bit0 = isAbstract → skip
        let flags = (clsb.add(0x70) as *const u32).read_unaligned();
        if crate::rtti::classe_abstrata(flags) {
            continue;
        }

        // Verifica ancestral: CClass+0x10=parent, +0x18=name(CName hash)
        let self_name = (clsb.add(0x18) as *const u64).read_unaligned();
        if self_name == st_cname {
            continue; // skip a própria base
        }
        let mut parent = (clsb.add(0x10) as *const *mut u8).read_unaligned();
        let mut found = false;
        let mut depth = 0usize;
        while !parent.is_null() && depth < 16 {
            depth += 1;
            if !crate::memoria::is_readable(parent as *const c_void, 0x20) {
                break;
            }
            let pname = (parent.add(0x18) as *const u64).read_unaligned();
            if pname == st_cname {
                found = true;
                break;
            }
            parent = (parent.add(0x10) as *const *mut u8).read_unaligned();
        }
        if !found {
            continue;
        }

        // IType::GetName() em vtbl+0x10 → CName hash → string
        let name_str = {
            let vt = (ty as *const *const u8).read_unaligned();
            if vt.is_null() || !crate::memoria::is_readable(vt as *const c_void, 0x18) {
                continue;
            }
            let get_name_fn = *(vt.add(0x10) as *const *const c_void);
            if get_name_fn.is_null() {
                continue;
            }
            let get_name: unsafe extern "C" fn(*mut c_void) -> u64 =
                std::mem::transmute(get_name_fn);
            crate::cname::resolve_cname(get_name(ty))
        };

        let obj = crate::rtti::new_object(reg, &name_str);
        if obj.is_null() {
            crate::log_dev!("[tweakxl] new_object falhou: {name_str}");
            continue;
        }
        // Referência forte retida pra sempre antes do `call_func`: o objeto nasce sem bloco de refcount e não tem outro dono.
        // Defensivo — não é o conserto do crash de load (boot 35 crashou com ele); o conserto é a AL-2, conferida no começo do despacho.
        let dono: &'static mut [u8; 16] = Box::leak(Box::new([0u8; 16]));
        crate::rtti::make_handle(dono.as_mut_ptr() as *mut c_void, obj);
        // Diagnóstico do objeto destruído 2x (boot 49), só no dev: registra objeto e bloco do nosso handle e loga strong/weak em volta do `call_func`.
        #[cfg(feature = "devtools")]
        let bloco = (dono.as_ptr().add(8) as *const u64).read_unaligned();
        #[cfg(feature = "devtools")]
        let idx = crate::selftest::onapply_objeto_registrar(obj as u64, bloco);
        #[cfg(feature = "devtools")]
        let vt = (obj as *const u64).read_unaligned();
        #[cfg(feature = "devtools")]
        let slot_d8 = if vt != 0 { ((vt + 0xd8) as *const u64).read_unaligned() } else { 0 };
        #[cfg(feature = "devtools")]
        let (s0, w0) = crate::selftest::refcnt_contagens(bloco);
        if let Some(rf) = crate::rtti::resolve_func(reg, &name_str, "OnApply") {
            #[cfg(feature = "devlog")]
            let _ = crate::tweakdb::natives::tirar_contagem();
            #[cfg(feature = "devlog")]
            let inicio = std::time::Instant::now();
            let (_, subidas) = crate::tweakdb::escritor::em_lote_de_versao(|| crate::rtti::call_func(&rf, obj, &[]));
            dispatched += 1;
            crate::log_dev!("[tweakxl] OnApply → {name_str}");
            // Separa "o mod não achou slot/record" de "a escrita do BWMS falhou": o Equipment-EX monta a grade do guarda-roupa pelo último
            // `placementSlots` de cada roupa, que o `PatchOriginaltems` grava aqui (`array:TweakDBID`).
            #[cfg(feature = "devlog")]
            {
                let (contagem, falhos) = crate::tweakdb::natives::tirar_contagem();
                crate::veredito_dev!("[tweakxl] VEREDITO-ONAPPLY {name_str}: {contagem}; escritas-ok={} = {}", falhos == 0, (falhos == 0) as u8);
                crate::veredito_dev!(
                    "[tweakxl] ONAPPLY-MEDIDA {name_str}: {subidas} subida(s) do byte de versão do TweakDB viraram {}; {} ms com o log de dev",
                    (subidas > 0) as u8,
                    inicio.elapsed().as_millis()
                );
            }
            #[cfg(not(feature = "devlog"))]
            let _ = subidas;
        }
        #[cfg(feature = "devtools")]
        let (s1, w1) = crate::selftest::refcnt_contagens(bloco);
        #[cfg(feature = "devtools")]
        let vt1 = (obj as *const u64).read_unaligned();
        #[cfg(feature = "devtools")]
        crate::veredito_dev!(
            "[onapply-diag] {name_str} objeto {idx:?} obj={obj:p} bloco={bloco:#x} vptr={:#x} vt[0xd8]={:#x} strong/weak depois do make_handle {s0}/{w0}, depois do call_func {s1}/{w1}, vptr depois={:#x}",
            crate::vm_do_jogo(vt),
            crate::vm_do_jogo(slot_d8),
            crate::vm_do_jogo(vt1)
        );
        // `obj` e o handle ficam vazados de propósito (instância efêmera, nunca liberada por nós)
    }
    crate::log_dev!(
        "[tweakxl] dispatch_scriptable_tweaks: {dispatched} OnApply() chamados"
    );
    // Stat novo criado pelo `OnApply` (`CreateRecord` + `SetFlat` do `enumName`) entra no sistema de stats.
    crate::tweakdb::stats::registrar_tardio();
}

/// **TweakXL hot-reload** — roda a cada `HOT_RELOAD_INTERVAL` ticks em gameplay.
/// Varre o mtime máximo dos yamls ativos; se mudou desde a última checagem, re-aplica.
pub(crate) unsafe fn hot_reload_tick() {
    let now = crate::ticks();
    let last = HOT_RELOAD_LAST_TICK.load(Ordering::Relaxed);
    if !crate::tempo::ja_passou(last, now, HOT_RELOAD_INTERVAL) {
        return;
    }
    HOT_RELOAD_LAST_TICK.store(now, Ordering::Relaxed);

    // Tweak da pasta `Mods/` editado dispara a recarga.
    let da_pasta = crate::mods_do_usuario::pasta_mods::tweaks_do_boot();
    let new_mt = da_pasta.iter().map(|p| mtime_em_segundos(p)).max().unwrap_or(0);
    if new_mt == 0 {
        return;
    }
    let old_mt = HOT_RELOAD_LAST_MTIME.swap(new_mt, Ordering::Relaxed);
    if old_mt != 0 && new_mt != old_mt {
        crate::log_dev!("[tweakxl-hotreload] arquivo de tweak modificado → recarga sem empilhar");
        recarregar_tweaks();
    }
}

/// O mtime de `p` em segundos; 0 se não se lê.
fn mtime_em_segundos(p: &std::path::Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .map(|mt| mt.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) mod testes {
    use crate::arquivos_do_jogo::{PatchDeRecurso, TabelaDePatches, PATCHES_DE_RECURSO, PATCHES_DE_RECURSO_ATIVOS, A17_PROP_COMPONENTS, ESPERA_DE_PATCH_ATIVA, ESPERAS_DE_PATCH};
    use std::sync::atomic::Ordering;

    /// As tabelas que a carga do ArchiveXL grava são do processo de teste inteiro, e a recarga com `substituir` troca todas de uma vez: todo teste que
    /// grava ou confere uma delas segura esta trava, ou a recarga de outro teste a esvazia no meio da asserção.
    static TRAVA_DAS_TABELAS_DO_ARCHIVEXL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub(crate) fn travar_tabelas_do_archivexl() -> std::sync::MutexGuard<'static, ()> {
        TRAVA_DAS_TABELAS_DO_ARCHIVEXL.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Pasta do teste em `temp_dir`, vazia; o teste a apaga por caminho explícito no fim.
    fn pasta_vazia(nome: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("bwms-f127-{nome}-{}", std::process::id()));
        std::fs::create_dir_all(&p).expect("pasta do teste");
        p
    }

    /// Caminhos `bwms\f127p\…`, só destes testes.
    fn h(nome: &str) -> u64 {
        bwms_primitives::resource_path_hash(&format!("bwms\\f127p\\{nome}"))
    }

    const JOURNAL_MESCLADO: &str = "mod\\f127p\\mesclado.journal";
    const JOURNAL_PEDIDO: &str = "mod\\f127p\\pedido.journal";
    const ESCOPO: u64 = 0x127;

    /// Uma linha em cada tabela que a carga grava, como deixam a carga anterior e as gravações de fora dela (`RegisterLink`, o fix e o escopo
    /// de teste). No journal, um já mesclado (estado 2) e um com a carga pedida e não mesclado (estado 1).
    fn semear(pasta: &std::path::Path) {
        use crate::mundo::{journal_tabela, linha_de_delecao, PatchDaTabela, DELECOES_DE_SETOR, JOURNAL_MODS, PATCHES_DA_TABELA};
        assert!(crate::arquivos_do_jogo::reslink_add(h("pedido.mesh"), h("servido.mesh")));
        assert!(crate::arquivos_do_jogo::depot::adicionar_copia(h("copia.mesh"), h("original.mesh")));
        *DELECOES_DE_SETOR.lock().expect("deleções de setor") =
            vec![linha_de_delecao(&bwms_primitives::tabelas::Streaming { mod_id: "m", setor: "bwms\\f127p\\setor.streamingsector", esperado: 3, indice: 1, tipo: "worldNode", op: None })];
        *PATCHES_DA_TABELA.lock().expect("patches de fase") = vec![PatchDaTabela {
            mod_id: "m".into(),
            pai: 1,
            fase: "f".into(),
            entrada: Vec::new(),
            socket_entrada: None,
            saida: Vec::new(),
            socket_saida: None,
            intercept: false,
        }];
        let semente = pasta.join("semente-journal.txt");
        std::fs::write(&semente, format!("m1 | {JOURNAL_MESCLADO}\nm2 | {JOURNAL_PEDIDO}\n")).expect("semente do journal");
        JOURNAL_MODS.lock().expect("journals de mod").clear();
        assert_eq!(journal_tabela(&semente.to_string_lossy(), false), 2);
        std::fs::remove_file(&semente).expect("semente do journal");
        {
            let mut g = JOURNAL_MODS.lock().expect("journals de mod");
            (g[0].token, g[0].estado) = (7, 2);
            (g[1].token, g[1].estado) = (8, 1);
        }
        *crate::aparencia::FIXES_DE_NOME_DO_MESH.write().expect("fix de nomes") = Some(std::collections::HashMap::from([(h("fix.mesh"), vec![(1, 2)])]));
        crate::corpo_e_rosto::cust_fix_caminho_teste(h("base.app"), h("antigo.app"), h("novo.app"), true);
        assert_eq!(crate::corpo_e_rosto::es20_carregar_escopos(&format!("{ESCOPO:#018x} | bwms\\f127p\\escopo.app\n")), 1);
    }

    /// Qual tabela ainda tem a linha de `semear`, e os caminhos da lista de journals.
    fn semeadas() -> ([(&'static str, bool); 7], Vec<String>) {
        use crate::mundo::{DELECOES_DE_SETOR, JOURNAL_MODS, PATCHES_DA_TABELA};
        let tabelas = [
            ("link", crate::arquivos_do_jogo::reslink_alvo(h("pedido.mesh")).is_some()),
            ("cópia", crate::arquivos_do_jogo::depot::original_de(h("copia.mesh")).is_some()),
            ("setor", !DELECOES_DE_SETOR.lock().expect("deleções de setor").is_empty()),
            ("fase", !PATCHES_DA_TABELA.lock().expect("patches de fase").is_empty()),
            ("fix de nomes", crate::aparencia::FIXES_DE_NOME_DO_MESH.read().expect("fix de nomes").is_some()),
            (
                "fix de caminhos",
                crate::corpo_e_rosto::CUST_FIX_CAMINHOS.read().expect("fix de caminhos").as_ref().is_some_and(|m| m.contains_key(&h("base.app"))),
            ),
            ("escopo", crate::corpo_e_rosto::es20_no_escopo(ESCOPO, h("escopo.app"))),
        ];
        let journals = JOURNAL_MODS.lock().expect("journals de mod").iter().map(|j| j.caminho.clone()).collect();
        (tabelas, journals)
    }

    #[test]
    fn recarga_sem_a_tabela_de_resource_patch_desliga_os_patches() {
        let _trava = travar_tabelas_do_archivexl();
        // Carga anterior com 1 patch ligado e 1 espera no `Deserialize`; a gravação do boot apaga o arquivo quando nenhum mod gera linha, e a
        // recarga não o acha.
        *PATCHES_DE_RECURSO.write().expect("tabela do #17") =
            TabelaDePatches::nova(vec![PatchDeRecurso { mod_id: "m".into(), patch: 1, alvo: 2, props: A17_PROP_COMPONENTS, token: [0; 2], alvo_token: [0; 2], app: false }]);
        PATCHES_DE_RECURSO_ATIVOS.store(true, Ordering::Relaxed);
        *ESPERAS_DE_PATCH.write().expect("esperas do 51k") = Some(std::collections::HashMap::from([(2u64, vec![1u64])]));
        ESPERA_DE_PATCH_ATIVA.store(true, Ordering::Relaxed);
        let red4 = std::env::temp_dir().join(format!("bwms-teste-sem-resource-patch-{}", std::process::id()));
        unsafe { super::carregar_resource_patch(&red4) };
        assert!(PATCHES_DE_RECURSO.read().expect("tabela do #17").patches.is_empty());
        assert!(!PATCHES_DE_RECURSO_ATIVOS.load(Ordering::Relaxed));
        assert!(ESPERAS_DE_PATCH.read().expect("esperas do 51k").is_none(), "a espera do patch sai com a tabela");
        assert!(!ESPERA_DE_PATCH_ATIVA.load(Ordering::Relaxed));
    }

    /// `apply-03`, a fiação: a carga contra uma pasta sem nenhuma tabela. Sem `substituir` (o boot) nada muda, e as linhas gravadas antes da carga (o
    /// `RegisterLink` do `OnLoad`) ficam. Com `substituir` (o `ArchiveXL.Reload`) cada tabela esvazia, porque a gravação
    /// do boot apaga o arquivo de que nenhum mod gera linha; do journal fica só o mesclado.
    #[test]
    fn carga_sem_arquivos_so_esvazia_as_tabelas_na_recarga() {
        let _trava = travar_tabelas_do_archivexl();
        let red4 = pasta_vazia("sem-tabelas");
        semear(&red4);

        super::carregar_tabelas_do_archivexl(&red4, false);
        let (tabelas, journals) = semeadas();
        assert!(tabelas.iter().all(|(_, tem)| *tem), "o boot sem arquivo não mexe: {tabelas:?}");
        assert_eq!(journals, [JOURNAL_MESCLADO, JOURNAL_PEDIDO]);

        super::carregar_tabelas_do_archivexl(&red4, true);
        let (tabelas, journals) = semeadas();
        assert!(tabelas.iter().all(|(_, tem)| !*tem), "a recarga sem arquivo esvazia: {tabelas:?}");
        assert_eq!(journals, [JOURNAL_MESCLADO], "o pedido e não mesclado sai; o mesclado está na árvore viva");

        std::fs::remove_dir(&red4).expect("pasta do teste");
    }

    /// `apply-03`: `bwms-resfixes.txt` que existe e não se lê (aqui, uma pasta com o nome dele; no jogo, permissão ou disco que caiu) não mexe em nenhuma
    /// das 2 tabelas que ele alimenta, nem na recarga, porque esvaziar por erro transitório tiraria o fix até o próximo `Reload`. As outras tabelas, sem
    /// arquivo, esvaziam: a recarga rodou.
    #[test]
    fn recarga_com_resfixes_ilegivel_mantem_as_duas_tabelas_do_fix() {
        let _trava = travar_tabelas_do_archivexl();
        let red4 = pasta_vazia("resfixes-ilegivel");
        semear(&red4);
        let resfixes = red4.join(bwms_primitives::tabelas::RESFIXES.arquivo);
        std::fs::create_dir(&resfixes).expect("pasta no lugar da tabela");
        assert!(std::fs::read_to_string(&resfixes).is_err_and(|e| e.kind() != std::io::ErrorKind::NotFound));

        super::carregar_tabelas_do_archivexl(&red4, true);
        let (tabelas, _) = semeadas();
        for (nome, tem) in tabelas {
            assert_eq!(tem, nome.starts_with("fix de "), "{nome}: {tabelas:?}");
        }

        std::fs::remove_dir(&resfixes).expect("pasta do teste");
        std::fs::remove_dir(&red4).expect("pasta do teste");
    }

    /// `xl:XLP-09`: com só linhas `paths` (2 `.xl` com o mesmo caminho antigo), a carga do boot põe na tabela do `FixCustomizationOptions` o novo da
    /// última linha e não liga o fix de nomes: sem linha `names`, a conferência de offsets do `CMesh` e o hook não rodam. Na recarga sem o arquivo, a
    /// tabela dos caminhos esvazia.
    /// S12: o `context` sai por mesh em snake_case; o mesmo parâmetro fica com o valor da última linha e duas chaves com o mesmo nome em
    /// snake_case ficam com a 1ª; as outras linhas não entram.
    #[test]
    fn contexto_do_resfixes_por_mesh_em_snake_case() {
        let texto = "a.mesh | context | appearanceExpansionSource | X\na.mesh | names | m1 | m2\na.mesh | context | appearanceExpansionSource | Y\n\
                     a.mesh | context | fooBar | 1\na.mesh | context | foo_bar | 2\nb.mesh | context | Cor | azul\n";
        let c = super::resfixes_contexto(texto);
        let a = &c[&bwms_primitives::resource_path_hash("a.mesh")];
        assert_eq!(a, &vec![("appearance_expansion_source".to_string(), "Y".to_string()), ("foo_bar".to_string(), "1".to_string())]);
        assert_eq!(c[&bwms_primitives::resource_path_hash("b.mesh")], vec![("cor".to_string(), "azul".to_string())]);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn carga_do_resfixes_so_com_paths_fica_com_o_ultimo_sem_o_fix_de_nomes() {
        let _trava = travar_tabelas_do_archivexl();
        let red4 = pasta_vazia("resfixes-paths");
        let resfixes = red4.join(bwms_primitives::tabelas::RESFIXES.arquivo);
        std::fs::write(
            &resfixes,
            "# bwms-resfixes (gerado de .xl)\n\
             bwms\\f127p\\base.app | paths | bwms\\f127p\\antigo.app | bwms\\f127p\\mod1.app\n\
             bwms\\f127p\\base.app | paths | bwms\\f127p\\antigo.app | bwms\\f127p\\mod2.app\n",
        )
        .expect("tabela do resource.fix");
        *crate::aparencia::FIXES_DE_NOME_DO_MESH.write().expect("fix de nomes") = None;
        *crate::corpo_e_rosto::CUST_FIX_CAMINHOS.write().expect("fix de caminhos") = None;
        let caminhos_de_base = || {
            crate::corpo_e_rosto::CUST_FIX_CAMINHOS.read().expect("fix de caminhos").as_ref().map(|m| m.get(&h("base.app")).cloned().unwrap_or_default())
        };

        super::carregar_tabelas_do_archivexl(&red4, false);
        assert_eq!(caminhos_de_base(), Some(vec![(h("antigo.app"), h("mod2.app"))]));
        assert!(crate::aparencia::FIXES_DE_NOME_DO_MESH.read().expect("fix de nomes").is_none());

        std::fs::remove_file(&resfixes).expect("tabela do teste");
        super::carregar_tabelas_do_archivexl(&red4, true);
        assert_eq!(caminhos_de_base(), Some(Vec::new()));

        std::fs::remove_dir(&red4).expect("pasta do teste");
    }

    /// `apply-03`, a ponta: a native `ArchiveXL.Reload` recarrega com `substituir`. No processo de teste, a pasta das tabelas é a do executável do
    /// teste (`crate::dylib_dir`), sem nenhuma tabela: o par de `RegisterLink` e o resto do que `semear` grava saem.
    #[test]
    fn native_reload_substitui_as_tabelas() {
        let _trava = travar_tabelas_do_archivexl();
        let red4 = std::path::PathBuf::from(crate::dylib_dir().expect("pasta do executável do teste"));
        for nome in bwms_primitives::tabelas::TODAS.map(|t| t.arquivo) {
            assert!(!red4.join(nome).exists(), "{nome} em {}: o teste supõe a pasta sem tabela", red4.display());
        }
        let semente = pasta_vazia("native");
        semear(&semente);

        unsafe { crate::aparencia::tramp_recarregar_tabelas_de_runtime(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0) };
        let (tabelas, journals) = semeadas();
        assert!(tabelas.iter().all(|(_, tem)| !*tem), "{tabelas:?}");
        assert_eq!(journals, [JOURNAL_MESCLADO]);

        std::fs::remove_dir(&semente).expect("pasta do teste");
    }

    /// `apply-03`: com `substituir`, a lista de journals vira a do arquivo mais o já mesclado (estado 2), que está na árvore viva. O que tem a carga pedida
    /// e não foi mesclado (estado 1) sai quando o arquivo não o lista, e fica, com o mesmo token, quando lista. O arquivo ausente vale como vazio. Na carga
    /// do boot, o arquivo ausente não mexe na lista.
    #[test]
    fn journal_com_substituir_fica_com_o_arquivo_e_com_o_mesclado() {
        use crate::mundo::{journal_tabela, JOURNAL_MODS};
        let _trava = travar_tabelas_do_archivexl();
        let dir = pasta_vazia("journal");
        let (dois, b_e_c, so_c, ausente) = (dir.join("dois.txt"), dir.join("b_e_c.txt"), dir.join("so_c.txt"), dir.join("ausente.txt"));
        std::fs::write(&dois, "# bwms-journal\nm1 | mod\\a.journal\nm2 | mod\\b.journal\n").expect("tabela com a e b");
        std::fs::write(&b_e_c, "m2 | mod\\b.journal\nm3 | mod\\c.journal\n").expect("tabela com b e c");
        std::fs::write(&so_c, "m3 | mod\\c.journal\n").expect("tabela com c");
        let caminhos = || JOURNAL_MODS.lock().expect("journals de mod").iter().map(|j| j.caminho.clone()).collect::<Vec<_>>();
        let token_de = |c: &str| JOURNAL_MODS.lock().expect("journals de mod").iter().find(|j| j.caminho == c).map(|j| j.token);

        JOURNAL_MODS.lock().expect("journals de mod").clear();
        assert_eq!(journal_tabela(&dois.to_string_lossy(), false), 2);
        {
            let mut g = JOURNAL_MODS.lock().expect("journals de mod");
            (g[0].token, g[0].estado) = (7, 2);
            (g[1].token, g[1].estado) = (8, 1);
        }
        assert_eq!(journal_tabela(&ausente.to_string_lossy(), false), 0);
        assert_eq!(caminhos(), ["mod\\a.journal", "mod\\b.journal"]);
        assert_eq!(journal_tabela(&b_e_c.to_string_lossy(), true), 2);
        assert_eq!(caminhos(), ["mod\\a.journal", "mod\\b.journal", "mod\\c.journal"]);
        assert_eq!(token_de("mod\\b.journal"), Some(8), "o pedido que o arquivo ainda lista não pede a carga de novo");
        assert_eq!(journal_tabela(&so_c.to_string_lossy(), true), 1);
        assert_eq!(caminhos(), ["mod\\a.journal", "mod\\c.journal"]);
        assert_eq!(journal_tabela(&ausente.to_string_lossy(), true), 0);
        assert_eq!(caminhos(), ["mod\\a.journal"]);

        for f in [&dois, &b_e_c, &so_c] {
            std::fs::remove_file(f).expect("arquivo do teste");
        }
        std::fs::remove_dir(&dir).expect("pasta do teste");
    }
}
