//! API C-ABI exposta aos plugins Rust (`BwmsApi`). O plugin recebe um `*const BwmsApi`
//! no entry e chama a vtable pra HOOKAR e REFLETIR sem linkar nada do nosso crate.
//!
//! v1 (ALFA): `log` + `vtable_hook`/`vtable_unhook` (o motor de hook do BWMS, já provado
//! in-game, COW em __DATA_CONST) + `field_ptr`/`call_method` (reflection por nome, o mesmo do
//! getf/setf/callf). As versões seguintes (inline hook, `register_native`, janela no terminal, TweakDB…)
//! entram SÓ no fim da struct e sobem `abi_version` (ABI estável pra plugins antigos).

use std::ffi::{c_void, CStr};
use std::os::raw::c_char;

/// Vtable C-ABI passada ao plugin. `#[repr(C)]` => layout estável; o plugin declara a MESMA
/// struct (idêntica) e chama os ponteiros. Campos novos entram só no FIM (compat pra frente).
#[repr(C)]
pub struct BwmsApi {
    /// Versão da ABI (= `plugins::BWMS_PLUGIN_API`). O plugin pode checar antes de usar campos novos.
    pub abi_version: u32,
    /// Escreve `[plugin] msg` em `<jogo>/r6/logs/bwms-mods.log` só se o arquivo já existir (criar
    /// liga, apagar desliga), com teto de 8 MB e 200 linhas/s. No build dev também vai pro
    /// `/tmp/cp77-console.log` (`log_dev!`). `msg` = C-string UTF-8.
    pub log: unsafe extern "C" fn(*const c_char),
    /// Hook de VTABLE: troca o slot `slot_idx` (índice em u64) de `vtbl` por `repl`; devolve o
    /// ponteiro ORIGINAL do slot (pra encadear). null = falhou. Não patcha __TEXT (COW na vtable).
    pub vtable_hook: unsafe extern "C" fn(vtbl: *mut u64, slot_idx: usize, repl: *const c_void) -> *const c_void,
    /// Desfaz um `vtable_hook` (restaura `orig` no slot).
    pub vtable_unhook: unsafe extern "C" fn(vtbl: *mut u64, slot_idx: usize, orig: *const c_void),
    /// Endereço do CAMPO `field` (por nome, via RTTI) no objeto vivo `obj`. null = não achou.
    /// O plugin lê/escreve direto nesse ponteiro — é o getf/setf cru.
    pub field_ptr: unsafe extern "C" fn(obj: *mut c_void, field: *const c_char) -> *mut c_void,
    /// Chama o método `method` SEM args em `obj` (por nome); escreve 16 bytes de retorno em
    /// `ret16` (pode ser null). true = chamou. (Com args tipados: `call_method_args`, v6.) Retorno `ref<T>`: com `ret16` nulo o BWMS solta a
    /// referência; com `ret16`, os 16 bytes são um `Handle` que traz +1 forte do plugin (a C-ABI ainda não tem a função de soltar).
    pub call_method: unsafe extern "C" fn(obj: *mut c_void, method: *const c_char, ret16: *mut u8) -> bool,
    // --- v2 (abi_version 2): hook INLINE (qualquer função, não só método virtual) ---
    /// Hook inline: troca `target` por `repl`; devolve o TRAMPOLIM (chame-o pra invocar a
    /// função original) ou null. Relocator arm64 + COW em __TEXT (provado in-game). `repl` já
    /// empilhado em `target` devolve null: o 2º trampolim voltaria pro próprio `repl`. Os 16 bytes
    /// de `target` mudados durante a instalação (outro hook escrito nele ao mesmo tempo) também
    /// devolvem null, sem patch: o trampolim relocou bytes velhos. Nesse caso, chame de novo.
    pub inline_hook: unsafe extern "C" fn(target: *mut c_void, repl: *mut c_void) -> *mut c_void,
    /// Desfaz o hook mais recente em `target`, de qualquer dono, com as outras threads paradas.
    pub inline_revert: unsafe extern "C" fn(target: *mut c_void),
    // --- v3 (abi_version 3): registrar native global no RTTI ---
    /// Registra uma native global no RTTI chamável do redscript: `full`=nome completo, `short`=nome
    /// curto, `handler`=`extern "C" fn(ctx, frame, ret, a4)`. Devolve true se re-resolve OK.
    /// (Sem args; com args: `register_native_argful`, v4; método: `register_method`, v7.) PROVADO internamente (register_all/BlackwallPing).
    pub register_native: unsafe extern "C" fn(
        full: *const c_char,
        short: *const c_char,
        handler: crate::register::NativeHandler,
    ) -> bool,
    // --- v4 (abi_version 4): API simétrica ao núcleo — nativa COM ARGS + emitir evento ---
    /// Registra uma native global COM ARGS chamável do redscript. `param_types` = array de
    /// `n_params` C-strings com os NOMES DOS TIPOS RED (ex.: `"Float"`, `"CName"`, `"Int32"`).
    /// O handler lê os args via o frame (padrão read_params). Devolve true se re-resolve OK.
    /// Fecha a assimetria: antes o plugin só registrava nativa SEM-args (o núcleo já fazia argful).
    pub register_native_argful: unsafe extern "C" fn(
        full: *const c_char,
        short: *const c_char,
        handler: crate::register::NativeHandler,
        param_types: *const *const c_char,
        n_params: usize,
    ) -> bool,
    /// Dispara um evento do CallbackSystem por nome (ex.: `"Input/Custom"`): chama todos os
    /// callbacks registrados nesse evento. Devolve quantos dispararam. Deixa um plugin EMITIR
    /// eventos que mods (redscript/plugin) escutam — a outra ponta do RegisterCallback.
    pub fire_event: unsafe extern "C" fn(name: *const c_char) -> usize,
    // --- v5 (abi_version 5): Reflection TIPADA (get/set de campo por nome, sem ponteiro cru) ---
    /// Lê um campo Float por nome no objeto vivo. `0.0` se não achar (o plugin não faz cast de
    /// ponteiro na mão como no `field_ptr`). Parity com o getf interno.
    pub prop_get_f32: unsafe extern "C" fn(obj: *mut c_void, field: *const c_char) -> f32,
    /// Escreve um campo Float por nome. Devolve true se achou+escreveu.
    pub prop_set_f32: unsafe extern "C" fn(obj: *mut c_void, field: *const c_char, val: f32) -> bool,
    /// Lê um campo Int32/Uint32 por nome (0 se não achar).
    pub prop_get_i32: unsafe extern "C" fn(obj: *mut c_void, field: *const c_char) -> i32,
    // --- v6 (abi_version 6): call de método COM ARGS tipados (parity com o callf interno) ---
    /// Chama `method` em `obj` com `n_args` args tipados. Cada `args[i]` é uma C-string no formato
    /// do loop de cmd: `i:5` (Int32), `f:1.5` (Float), `b:true` (Bool), `n:Nome` (CName), `s:txt`
    /// (String), `e:3` (Enum). Escreve 16B de retorno em `ret16` (pode ser null). true = chamou. Retorno `ref<T>`: mesma regra do
    /// `call_method` (nulo = o BWMS solta; preenchido = +1 forte do plugin).
    /// Fecha o gap do `call_method` sem-args: reusa `parse_cmd_arg` + `call_func` (callf, provados).
    pub call_method_args: unsafe extern "C" fn(
        obj: *mut c_void,
        method: *const c_char,
        args: *const *const c_char,
        n_args: usize,
        ret16: *mut u8,
    ) -> bool,
    // --- v7 (abi_version 7): registrar MÉTODO novo numa classe EXISTENTE (@addMethod-style) ---
    /// Registra um método novo, chamável do redscript, numa classe REAL já existente (ex.:
    /// GameObject, Entity, gameGodModeSystem — o padrão de `@addMethod`).
    /// `class`=nome da classe-alvo, `full`=nome completo do método, `short`=nome curto,
    /// `handler`=`extern "C" fn(ctx, frame, ret, a4)`. Devolve true se registrou OK. Até aqui só `register_native` (função GLOBAL) era exposto
    /// a plugins — isto fecha o caso "estender classe existente", já provado internamente
    /// (register_method/regmethod_selftest, `gameGodModeSystem.BwmsRegTest`, 2026-07-12).
    /// Classe de SCRIPT (ex.: `PlayerPuppet`) ainda não existe quando o plugin carrega (a carga roda
    /// na 1ª validação de classes, antes do bind das funções): o pedido fica numa fila, devolve true
    /// (aceito) e é registrado no 1º tick do jogo, quando a classe já existe. Falha nesse registro
    /// adiado só aparece no log de dev (`[plugin] register_method <classe>.<nome> adiado FALHOU`).
    /// Pedido feito depois desse ponto com classe que não existe devolve false na hora.
    pub register_method: unsafe extern "C" fn(
        class: *const c_char,
        full: *const c_char,
        short: *const c_char,
        handler: crate::register::NativeHandler,
    ) -> bool,
    // --- v8 (abi_version 8): TweakDB — flat escalar por nome + clone com herança ---
    /// Lê o valor escalar (4 bytes crus em `+0x08` do FlatValue) do flat `name` (formato
    /// `Record.campo`, ex. `"Items.GrenadeIncendiarySticky.deepWaterDepth"`). Devolve os bits em
    /// `out_bits` (o plugin reinterpreta como f32/i32 conforme o tipo do campo); true = achou.
    pub tweakdb_get_flat: unsafe extern "C" fn(name: *const c_char, out_bits: *mut u32) -> bool,
    /// Escreve o valor escalar de `name` (mesmo formato) num FlatValue próprio e re-aponta só a
    /// entrada desse flat: records que dividiam o valor antigo não mudam. A mudança chega aos
    /// records clonados via `tweakdb_clone_record` que herdaram o campo sem override. A largura
    /// gravada sai do tipo do valor (8 B pra `TweakDBID`/`CName`), não dos 32 bits de `bits`.
    /// true = achou+escreveu; false se o flat não existe ou o tipo não é escalar.
    pub tweakdb_set_flat: unsafe extern "C" fn(name: *const c_char, bits: u32) -> bool,
    /// Clona `source` (record existente) como `new_name`, herdando os flats reais (stats) via
    /// `InheritFlats`, e registra o record novo no TweakDB vivo (`CreateRecord`). Assíncrono
    /// (roda numa thread própria, ~1.5s de atraso) — chamar de dentro de um handler de native/
    /// callback é seguro, o resultado aparece no log (`[clone] N flats herdados...`). Sempre
    /// devolve true se os 3 argumentos forem C-strings válidas (aceito para processamento; não
    /// é confirmação de sucesso — ver log).
    pub tweakdb_clone_record: unsafe extern "C" fn(
        class_name: *const c_char,
        source: *const c_char,
        new_name: *const c_char,
    ) -> bool,
    // --- v9 (abi_version 9): logger por nível + SemVer runtime ---
    /// Loga `msg` marcado com `level` (0=Trace 1=Debug 2=Info 3=Warning 4=Error 5=Critical).
    /// Níveis fora de 0..5 caem em "Info". Parity
    /// com o `log` sem-nível (v1), que continua funcionando igual.
    pub log_level: unsafe extern "C" fn(level: u8, msg: *const c_char),
    /// Compara duas versões pela precedência SemVer 2.0 completa (`semver_precedence`): o
    /// pre-release conta (`"1.2.3-rc1"` NÃO satisfaz `"1.2.3"`), o metadado de build (`+…`) não.
    /// Cobre o caso real de `Require("1.2.0")` de um plugue de versão checando "a versão instalada é >= a
    /// exigida". Devolve `actual >= required`; componente ausente ou malformado vale 0; ponteiro
    /// nulo ou UTF-8 inválido devolve false. Ex.: `semver_satisfies("1.2.0", "1.3.0") == true`.
    pub semver_satisfies: unsafe extern "C" fn(required: *const c_char, actual: *const c_char) -> bool,
    // --- v10 (abi_version 10): janela própria do plugin no terminal ---
    /// Registra `cb` pra ser chamado a cada quadro com o terminal BWMS aberto, depois do terminal
    /// (`overlay::desenhar_terminal`). O plugin usa `janela_abrir/janela_texto/janela_fechar` (abaixo)
    /// de dentro de `cb` pra desenhar a PRÓPRIA janela sem linkar nada: os slots chamam os widgets do
    /// terminal (`terminal::imediato`). Multi-registro (Vec interno) — plugins não pisam uns nos
    /// outros. Só aceita durante `bwms_plugin_main` (devolve `false` fora dele).
    pub register_draw_callback: unsafe extern "C" fn(cb: extern "C" fn()) -> bool,
    /// Abre a janela do plugin pelo título (`api_janela_abrir`): a 1ª vez ela nasce em (60, 60), na
    /// frente, e o tamanho segue o conteúdo. `"Nome##id"` mostra só `Nome` e `"Nome###id"` é a mesma
    /// janela com qualquer texto antes do `###`. Devolve `false` se ela está recolhida. Fora do
    /// callback de desenho não faz nada e devolve `true`. Os 3 slots de janela ficam no lugar e com a
    /// assinatura da v10 (o nome do campo não existe no binário): plugin compilado da v10 em diante
    /// desenha por aqui sem recompilar.
    pub janela_abrir: unsafe extern "C" fn(title: *const c_char) -> bool,
    /// Texto na janela aberta por `janela_abrir` (`api_janela_texto`): `\n` quebra a linha e `\r` não
    /// aparece. Sem janela aberta (ou recolhida), nada.
    pub janela_texto: unsafe extern "C" fn(text: *const c_char),
    /// Fecha a janela aberta por `janela_abrir` (`api_janela_fechar`). Fechar sem abrir não faz nada,
    /// e a janela que o callback deixou aberta fecha no fim dele.
    pub janela_fechar: unsafe extern "C" fn(),
    // --- v11 (abi_version 11): `scripts.Add(path)` — plugin registra um `.reds` FORA de
    // r6/scripts pro compilador incluir ---
    /// Registra `path` (caminho ABSOLUTO de um `.reds` fora de `r6/scripts`) num manifesto
    /// persistente (`~/.bwms-scripts-add.txt`, 1 path por linha, dedup). **NÃO recompila nem
    /// afeta o boot ATUAL** — este processo já carregou o `final.redscripts` que o compile
    /// anterior gerou, ANTES do plugin sequer rodar (redscript não hot-recarrega em runtime,
    /// diferente de Lua). O efeito aparece no PRÓXIMO ciclo compile+boot: o passo de
    /// compile (`bwms-fastboot.sh compile`, ou o instalador) lê o manifesto e passa cada path
    /// via `-compilePathsFile` do `scc` (achado nesta rodada — `scc` aceita uma lista de
    /// arquivos AVULSOS, além do diretório `r6/scripts` normal, no MESMO passe de compilação;
    /// verificado num scratch-copy seguro antes de mexer no deploy real). Devolve `true` se
    /// escreveu (ou já estava) no manifesto; `false` só em erro de I/O real.
    pub scripts_add: unsafe extern "C" fn(path: *const c_char) -> bool,
    // --- v12 (abi_version 12): callbacks de estado de jogo ---
    /// Registra callbacks de ciclo de vida de estado de jogo.
    /// `state_type`: 0=BaseInitialization 1=Initialization 2=Running 3=Shutdown (EGameStateType).
    /// Callbacks recebem o `CGameApplication` do jogo (`aApp`, abaixo). **Cobertura completa dos 4 estados (fechado 2026-08-11,
    /// PENDENCIAS-UNIFICADAS.md)**, todos disparando na ordem real
    /// `BaseInit→Init→Running→Shutdown`, mapeados nos eventos já detectados:
    ///   BaseInit.OnEnter/OnExit → disparam 1x, cedo no boot (logo após `boot_phase()`, o 1º ponto
    ///                             idempotente onde plugins já tiveram chance de `add_game_state`)
    ///   Init.OnEnter            → dispara na sequência, mesmo instante (mesmo guard)
    ///   Init.OnUpdate           → dispara a cada ~2s de heartbeat, enquanto ainda em Initialization
    ///   Init.OnExit             → dispara na transição de PRESENÇA do player em `cp77_tick` (mesmo
    ///                             mecanismo confiável do Running.OnEnter, não mais o byte de fase —
    ///                             que uma investigação anterior (cont.192) provou ser flaky nessa
    ///                             leitura), IMEDIATAMENTE ANTES de Running.OnEnter — latch único
    ///                             (Initialization só transiciona pra Running 1x por processo)
    ///   Running.OnEnter  → dispara quando o player spawna (presença detectada no cp77_tick)
    ///   Running.OnUpdate → dispara a cada ~180 ticks de gameplay (enquanto player presente)
    ///   Running.OnExit   → dispara quando o player despawna
    ///   Shutdown.OnEnter → dispara quando `exit()` é hookado (clean-exit)
    /// `aApp` é o `CGameApplication` do jogo (`aplicacao()`): `states` em `+0x278`, `currState` em `+0x290`, `name` em
    /// `+0x2A8` ("cpLauncher"); nulo só antes de o `main` gravar o global.
    pub add_game_state: unsafe extern "C" fn(
        state_type: u32,
        on_enter: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
        on_update: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
        on_exit: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
    ) -> bool,
    // --- v13 (abi_version 13): handle de plugin e posse de hook ---
    /// Handle ÚNICO deste plugin (atribuído sequencialmente em `plugins::load_one`, 1 cópia de
    /// `BwmsApi` por plugin — antes só existia UMA instância `static` compartilhada por todos).
    /// Guardar este valor e passá-lo pras 2 chamadas abaixo.
    pub my_handle: u64,
    /// Igual a `inline_hook`, com os mesmos casos de null, mas REGISTRA `handle` como DONO do hook em `target` — habilita
    /// `inline_detach` a recusar outro plugin tentando soltá-lo.
    pub inline_hook_owned: unsafe extern "C" fn(handle: u64, target: *mut c_void, repl: *mut c_void) -> *mut c_void,
    /// `inline_detach(handle, target)`: desfaz o hook mais recente de `handle` em `target`, posto
    /// por `inline_hook_owned`, só se ele é o do topo do alvo. Com outro hook empilhado por cima:
    /// recusa, loga e devolve `false`; os dois hooks e a posse continuam, e o detach funciona
    /// depois que o de cima sair. `handle` sem hook próprio em `target` também recusa. Hook do
    /// plugin já tirado por `inline_revert`: devolve `true` e solta a posse. Escrita no alvo
    /// recusada pelo kernel: devolve `false`, o hook continua no alvo e o plugin continua dono.
    pub inline_detach: unsafe extern "C" fn(handle: u64, target: *mut c_void) -> bool,
    /// Igual a `vtable_hook`, mas REGISTRA `handle` como dono do slot `(vtbl, slot_idx)`.
    pub vtable_hook_owned: unsafe extern "C" fn(handle: u64, vtbl: *mut u64, slot_idx: usize, repl: *const c_void) -> *const c_void,
    /// O detach pro caso vtable: restaura o slot original SÓ se `handle` for o dono
    /// (o `orig` fica guardado no registro interno — o plugin não precisa lembrar dele, ao
    /// contrário do `vtable_unhook` cru).
    pub vtable_detach: unsafe extern "C" fn(handle: u64, vtbl: *mut u64, slot_idx: usize) -> bool,
    // --- v14 (abi_version 14): comparação SemVer com pre-release — precedência SemVer
    // 2.0 COMPLETA (major.minor.patch + identificadores de pre-release), não só o triplet. Fecha
    // o bug real onde `semver_satisfies`/o `Require` dos plugues tratava `1.2.3-rc1` como IGUAL a
    // `1.2.3` (deveria ser MENOR — regra 11.3 da spec: release > pre-release no mesmo triplet).
    /// `compare_semver_prerelease(lhs, rhs) -> int32`: `<0` se `lhs<rhs`, `0` se iguais, `>0` se
    /// `lhs>rhs`, pela precedência SemVer 2.0 completa (não só major.minor.patch). Strings
    /// inválidas/nulas comparam como `0.0.0` sem pre-release (nunca crasha).
    pub compare_semver_prerelease: unsafe extern "C" fn(lhs: *const c_char, rhs: *const c_char) -> i32,
    // --- v15 (abi_version 15): argumentos de lançamento.
    /// `launch_arg(nome, saida, capacidade) -> int32`: copia o valor do argumento `nome` (sem o `-`; aceita `-nome valor`, `--nome valor`,
    /// `-nome=valor`) pra `saida` com NUL e devolve o tamanho do valor em bytes (0 pra argumento sem valor); `-1` se o argumento não veio. O
    /// valor é cortado em `capacidade - 1` bytes, mas o retorno é o tamanho inteiro (pra quem quiser pedir de novo com mais espaço).
    pub launch_arg: unsafe extern "C" fn(nome: *const c_char, saida: *mut c_char, capacidade: usize) -> i32,
    // --- v16 (abi_version 16): valor de campo de qualquer tipo pelo próprio tipo do campo: a cópia, a
    // comparação e o texto que o motor usa no `=`, no `==` e no `ToString` dos scripts. Os textos voltam como no `launch_arg`: copiados com NUL
    // e cortados em `capacidade - 1`, com o tamanho inteiro no retorno; `-1` sem o campo.
    /// Nome do tipo do campo (`String`, `array:Int32`, `handle:gameObject`…).
    pub prop_type_name: unsafe extern "C" fn(obj: *mut c_void, field: *const c_char, saida: *mut c_char, capacidade: usize) -> i32,
    /// Texto do valor do campo, o mesmo do `ToString` dos scripts (handle vira classe e endereço).
    pub prop_to_string: unsafe extern "C" fn(obj: *mut c_void, field: *const c_char, saida: *mut c_char, capacidade: usize) -> i32,
    /// Lê um campo `String`; `-1` se o campo não existe ou não é `String`.
    pub prop_get_string: unsafe extern "C" fn(obj: *mut c_void, field: *const c_char, saida: *mut c_char, capacidade: usize) -> i32,
    /// Grava um campo `String` de qualquer tamanho (alocado pelo motor).
    pub prop_set_string: unsafe extern "C" fn(obj: *mut c_void, field: *const c_char, texto: *const c_char) -> bool,
    /// Copia o campo `field` de `origem` pra `destino` pelo `Assign` do tipo — qualquer tipo (array, handle, struct, String). O tipo do campo
    /// de origem tem que caber no de destino (o mesmo tipo, ou handle de classe filha em handle da mãe).
    pub prop_copy: unsafe extern "C" fn(destino: *mut c_void, origem: *mut c_void, field: *const c_char) -> bool,
    /// Compara o campo `field` de dois objetos pelo `IsEqual` do tipo: `1` igual, `0` diferente, `-1` sem o campo ou tipos diferentes.
    pub prop_is_equal: unsafe extern "C" fn(a: *mut c_void, b: *mut c_void, field: *const c_char) -> i32,
    /// Genealogia dos records criados por mod: `1` se o record existe e não é de mod, `0` se é de mod, `-1` se não existe.
    pub tweak_is_original: unsafe extern "C" fn(id: u64) -> i32,
    /// O primeiro ancestral de `id` que não é de mod; `0` sem pai (record original ou criado por `$type`).
    pub tweak_original_parent: unsafe extern "C" fn(id: u64) -> u64,
    /// Os records de mod clonados de `id`, em qualquer geração, copiados pra `saida` (até `capacidade`); devolve quantos são.
    pub tweak_descendants: unsafe extern "C" fn(id: u64, saida: *mut u64, capacidade: usize) -> i32,
    /// Nome de um id que um mod escreveu; `-1` se não se sabe.
    pub tweak_name: unsafe extern "C" fn(id: u64, saida: *mut c_char, capacidade: usize) -> i32,
    /// Guarda o nome e devolve o id dele (`0` com nome vazio ou nulo).
    pub tweak_register_name: unsafe extern "C" fn(nome: *const c_char) -> u64,
    /// Chama uma função global de script por nome: argumentos no formato do
    /// `call_method_args`, mais `h:0x…` pra passar um objeto vivo como handle; 16 bytes de retorno em `ret16` (pode ser nulo); retorno `ref<T>` segue a regra do `call_method`.
    pub call_global: unsafe extern "C" fn(name: *const c_char, args: *const *const c_char, n_args: usize, ret16: *mut u8) -> bool,
    /// Tira do TweakDB um record criado por mod; recusa record do jogo. Os flats ficam.
    pub tweak_remove_record: unsafe extern "C" fn(id: u64) -> bool,
    /// Classe nativa nova, instanciável, filha de `mae` (nome nativo ou de script), com `extra` bytes pros campos do plugin.
    /// Devolve o offset onde esses bytes começam (`0` se falhou). Só vale chamada no carregamento do plugin, que roda antes do bind dos
    /// scripts: o `.reds` do plugin declara `public native class <nome> extends <mae>`.
    pub class_create: unsafe extern "C" fn(nome: *const c_char, mae: *const c_char, extra: u32) -> u32,
    /// Propriedade nativa nova: `tipo` é o nome do tipo no RTTI (`Int32`, `Float`, `CName`, `handle:Classe`,
    /// `whandle:Classe`, `array:Tipo`…) e `offset` a posição do valor no objeto, dentro do tamanho da classe. Também só no carregamento do
    /// plugin; o `.reds` declara `native let` (ou `(Classe) public native let` numa classe do jogo). Nome que já existe com o mesmo
    /// tipo devolve `true` sem criar outro.
    pub class_add_property: unsafe extern "C" fn(classe: *const c_char, nome: *const c_char, tipo: *const c_char, offset: u32) -> bool,
    /// Screenshot pelo motor: PNG em `caminho`, no próximo quadro. `false` sem motor pronto.
    pub take_screenshot: unsafe extern "C" fn(caminho: *const c_char) -> bool,
}

impl Clone for BwmsApi {
    fn clone(&self) -> Self {
        *self
    }
}
impl Copy for BwmsApi {}

/// Próximo handle a atribuir (sequencial, começa em 1 — `0` fica reservado como "sem dono"
/// nos registros de ownership abaixo, nunca um handle real).
static NEXT_PLUGIN_HANDLE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Chamado 1x por plugin em `plugins::load_one`, antes de montar a cópia pessoal da API.
pub(crate) fn next_plugin_handle() -> u64 {
    NEXT_PLUGIN_HANDLE.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

/// Hooks INLINE com dono, `(alvo, handle, trampolim)` na ordem de instalação. Só entra por `push`: o `inline_hook_owned` de B no mesmo alvo não tira
/// a posse de A, e o `inline_detach` confere pelo trampolim, dentro da trava de `ganchos`, se o hook do dono é o do topo do alvo.
static INLINE_HOOK_OWNERS: std::sync::Mutex<Vec<(usize, u64, u64)>> = std::sync::Mutex::new(Vec::new());
/// Dono registrado de cada slot de vtable hookado (`(vtbl addr, slot_idx) -> (handle, orig)`).
static VTABLE_HOOK_OWNERS: std::sync::Mutex<Vec<(usize, usize, u64, usize)>> = std::sync::Mutex::new(Vec::new());

fn register_inline_owner(target: usize, handle: u64, tramp: u64) {
    if let Ok(mut v) = INLINE_HOOK_OWNERS.lock() {
        v.push((target, handle, tramp));
    }
}

/// Trampolim do hook mais recente de `handle` em `target`.
fn owned_inline_tramp(target: usize, handle: u64) -> Option<u64> {
    let v = INLINE_HOOK_OWNERS.lock().ok()?;
    v.iter().rev().find(|&&(t, h, _)| t == target && h == handle).map(|&(_, _, tramp)| tramp)
}

fn release_inline_owner(target: usize, handle: u64, tramp: u64) {
    if let Ok(mut v) = INLINE_HOOK_OWNERS.lock() {
        v.retain(|&e| e != (target, handle, tramp));
    }
}

fn register_vtable_owner(vtbl: usize, slot: usize, handle: u64, orig: usize) {
    if let Ok(mut v) = VTABLE_HOOK_OWNERS.lock() {
        v.retain(|(vp, s, _, _)| !(*vp == vtbl && *s == slot));
        v.push((vtbl, slot, handle, orig));
    }
}

fn take_vtable_owner(vtbl: usize, slot: usize) -> Option<(u64, usize)> {
    if let Ok(mut v) = VTABLE_HOOK_OWNERS.lock() {
        if let Some(pos) = v.iter().position(|(vp, s, _, _)| *vp == vtbl && *s == slot) {
            let (_, _, h, o) = v.remove(pos);
            return Some((h, o));
        }
    }
    None
}

/// Registry de callbacks de estado de jogo (add_game_state).
/// Estado do jogo da C-ABI (o `EGameStateType`): o plugin registra callbacks por ele, e na ABI ele vai como `u32`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EstadoDoJogo {
    BaseInitialization = 0,
    Initialization = 1,
    Running = 2,
    Shutdown = 3,
}

struct GameStateEntry {
    state_type: u32,
    on_enter: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
    on_update: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
    on_exit: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
}

static GAME_STATES: std::sync::Mutex<Vec<GameStateEntry>> = std::sync::Mutex::new(Vec::new());

// ===== Janela de registro restrita ao load do plugin (`PENDENCIAS-UNIFICADAS.md`)
// — um mod só pode se registrar durante sua PRÓPRIA inicialização, nunca
// depois: `plugins::load_one` abre a janela só durante a chamada
// SÍNCRONA de `bwms_plugin_main`, fecha assim que ela retorna — as 5 funções de REGISTRO
// (register_native/register_native_argful/register_method/add_game_state/
// register_draw_callback) recusam fora dessa janela. NÃO se aplica a hooks/reflection/TweakDB
// (chamáveis a qualquer momento por design — só "registrar uma capacidade nomeada nova" é
// restrito).
static REGISTRATION_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn registration_window_open() -> bool {
    REGISTRATION_OPEN.load(std::sync::atomic::Ordering::Acquire)
}

/// RAII: `plugins::load_one` chama `open_registration_window()` antes de `entry()` e deixa o
/// guard cair no escopo — fecha a janela na saída (retorno normal OU panic desenrolado por
/// `catch_unwind`, nunca fica presa aberta). Mesmo espírito do `ExecDepthGuard` já usado no
/// projeto (contador thread_local + `Drop`, `inicio/boot.rs`) — RAII em vez de par
/// abre/fecha manual, que um `return`/panic no meio esqueceria de fechar.
pub(crate) struct RegistrationWindowGuard;

impl Drop for RegistrationWindowGuard {
    fn drop(&mut self) {
        REGISTRATION_OPEN.store(false, std::sync::atomic::Ordering::Release);
    }
}

pub(crate) fn open_registration_window() -> RegistrationWindowGuard {
    REGISTRATION_OPEN.store(true, std::sync::atomic::Ordering::Release);
    RegistrationWindowGuard
}

/// O `CGameApplication` do jogo, o que os callbacks de estado recebem: o `main` o monta na pilha com o nome "cpLauncher"
/// (construtor `0x104d6a108`) e guarda o ponteiro no global `0x1099d9360` antes do `Run` (`0x104d6a564`), o laço que chama `OnEnter`,
/// `OnUpdate` e `OnExit` do estado corrente (`+0x290`) com ele. Conferido pelo nome em `+0x2A8`; nulo antes de o global ser escrito.
const APP_GLOBAL_VM: u64 = 0x1_099d_9360;

pub(crate) fn aplicacao() -> *mut c_void {
    unsafe {
        let g = crate::rebase(APP_GLOBAL_VM) as *const u64;
        if !crate::memoria::is_readable(g as *const c_void, 8) {
            return std::ptr::null_mut();
        }
        let app = g.read_unaligned() as *mut u8;
        if app.is_null() || !crate::memoria::is_readable(app.add(0x2A8) as *const c_void, 8) {
            return std::ptr::null_mut();
        }
        let nome = (app.add(0x2A8) as *const *const c_char).read_unaligned();
        if nome.is_null() || !crate::memoria::is_readable(nome as *const c_void, 11) || CStr::from_ptr(nome).to_bytes() != b"cpLauncher" {
            return std::ptr::null_mut();
        }
        app as *mut c_void
    }
}

/// Registra um callback de estado sem a janela dos plugins (os testes do próprio BWMS).
#[cfg(feature = "devtools")]
pub(crate) fn registrar_estado_interno(estado: EstadoDoJogo, on_update: unsafe extern "C" fn(*mut c_void) -> bool) {
    if let Ok(mut states) = GAME_STATES.lock() {
        states.push(GameStateEntry { state_type: estado as u32, on_enter: None, on_update: Some(on_update), on_exit: None });
    }
}

/// Chama on_enter de todos os estados registrados com `state_type`.
pub fn call_game_state_enter(estado: EstadoDoJogo) {
    if let Ok(states) = GAME_STATES.lock() {
        for s in states.iter().filter(|s| s.state_type == estado as u32) {
            if let Some(f) = s.on_enter {
                let _ = unsafe { f(aplicacao()) };
            }
        }
    }
}

/// Chama on_update de todos os estados registrados com `state_type`.
pub fn call_game_state_update(estado: EstadoDoJogo) {
    if let Ok(states) = GAME_STATES.lock() {
        for s in states.iter().filter(|s| s.state_type == estado as u32) {
            if let Some(f) = s.on_update {
                let _ = unsafe { f(aplicacao()) };
            }
        }
    }
}

/// Chama on_exit de todos os estados registrados com `state_type`.
pub fn call_game_state_exit(estado: EstadoDoJogo) {
    if let Ok(states) = GAME_STATES.lock() {
        for s in states.iter().filter(|s| s.state_type == estado as u32) {
            if let Some(f) = s.on_exit {
                let _ = unsafe { f(aplicacao()) };
            }
        }
    }
}

unsafe extern "C" fn api_add_game_state(
    state_type: u32,
    on_enter: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
    on_update: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
    on_exit: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
) -> bool {
    if !registration_window_open() {
        crate::log_dev!("[api] add_game_state: recusado — chamado fora da janela de registro (só durante bwms_plugin_main)");
        return false;
    }
    if let Ok(mut states) = GAME_STATES.lock() {
        states.push(GameStateEntry { state_type, on_enter, on_update, on_exit });
        crate::log_dev!("[api] add_game_state: type={state_type} on_enter={} on_update={} on_exit={}",
            on_enter.is_some(), on_update.is_some(), on_exit.is_some());
        return true;
    }
    false
}

unsafe extern "C" fn api_log(msg: *const c_char) {
    if msg.is_null() {
        return;
    }
    crate::mods_do_usuario::log_de_mod::gravar("plugin", &CStr::from_ptr(msg).to_string_lossy());
}

unsafe extern "C" fn api_vtable_hook(vtbl: *mut u64, slot_idx: usize, repl: *const c_void) -> *const c_void {
    crate::ganchos::trocar_slot_da_vtable(vtbl, slot_idx, repl).unwrap_or(std::ptr::null())
}

unsafe extern "C" fn api_vtable_unhook(vtbl: *mut u64, slot_idx: usize, orig: *const c_void) {
    crate::ganchos::restaurar_slot_da_vtable(vtbl, slot_idx, orig);
}

unsafe extern "C" fn api_field_ptr(obj: *mut c_void, field: *const c_char) -> *mut c_void {
    if obj.is_null() || field.is_null() {
        return std::ptr::null_mut();
    }
    let name = match CStr::from_ptr(field).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let cls = crate::rtti::class_of(obj);
    if cls.is_null() {
        return std::ptr::null_mut();
    }
    let p = crate::rtti::find_property_in_class(cls, name);
    if p.is_null() {
        return std::ptr::null_mut();
    }
    let vo = crate::rtti::prop_value_offset(p) as usize;
    (obj as *mut u8).add(vo) as *mut c_void
}

unsafe extern "C" fn api_call_method(obj: *mut c_void, method: *const c_char, ret16: *mut u8) -> bool {
    if obj.is_null() || method.is_null() {
        return false;
    }
    let name = match CStr::from_ptr(method).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let cls = crate::rtti::class_of(obj);
    if cls.is_null() {
        return false;
    }
    match crate::rtti::resolve_in_class(cls, name) {
        Some(rf) => match crate::rtti::call_func(&rf, obj, &[]) {
            Some(r) => {
                if ret16.is_null() {
                    crate::rtti::soltar_retorno(&rf, &r);
                } else {
                    std::ptr::copy_nonoverlapping(r.as_ptr(), ret16, 16);
                }
                true
            }
            None => false,
        },
        None => false,
    }
}

unsafe extern "C" fn api_inline_hook(target: *mut c_void, repl: *mut c_void) -> *mut c_void {
    if target.is_null() || repl.is_null() {
        return std::ptr::null_mut();
    }
    // O alvo pode estar rodando em outras threads do jogo: o salto é escrito com elas paradas.
    crate::ganchos::instalar_com_threads_paradas(target, repl, None).unwrap_or(std::ptr::null_mut())
}

unsafe extern "C" fn api_inline_revert(target: *mut c_void) {
    if target.is_null() {
        return;
    }
    // TODO(decisão do dono, consolidação F1-11): o revert tira o hook do topo sem conferir dono, inclusive um `inline_hook_owned` de outro plugin.
    // Restringir a hooks sem dono muda o contrato da C-ABI pública, e não se sabe se algum plugin depende disso.
    // A ABI de `inline_revert` é `void`: o resultado da remoção não chega ao plugin.
    crate::ganchos::remover_parado(target, None);
}

unsafe extern "C" fn api_inline_hook_owned(handle: u64, target: *mut c_void, repl: *mut c_void) -> *mut c_void {
    if target.is_null() || repl.is_null() {
        return std::ptr::null_mut();
    }
    let trampoline = crate::ganchos::instalar_com_trampolim_novo(target, repl).unwrap_or(std::ptr::null_mut());
    if !trampoline.is_null() {
        register_inline_owner(target as usize, handle, trampoline as u64);
        crate::log_dev!("[api] inline_hook_owned: handle={handle} target={target:p} -> ok (dono registrado)");
    }
    trampoline
}

unsafe extern "C" fn api_inline_detach(handle: u64, target: *mut c_void) -> bool {
    if target.is_null() {
        return false;
    }
    let Some(tramp) = owned_inline_tramp(target as usize, handle) else {
        crate::log_dev!("[api] inline_detach: RECUSADO — handle={handle} sem hook próprio em target={target:p}");
        return false;
    };
    // Só tira se o hook deste plugin é o do topo: tirar o do topo com outro por cima derrubaria o hook do outro.
    let (restaurado, _) = crate::ganchos::remover_parado(target, Some(tramp));
    if crate::ganchos::gancho_no_ar(target, tramp) {
        crate::log_dev!(
            "[api] inline_detach: RECUSADO — handle={handle} target={target:p}: outro hook por cima ou escrita recusada; o hook e a posse continuam"
        );
        return false;
    }
    // Fora da lista: tirado agora, ou antes por um `inline_revert`. Nada mais a desfazer.
    release_inline_owner(target as usize, handle, tramp);
    crate::log_dev!("[api] inline_detach: handle={handle} target={target:p} -> OK (tirado agora: {restaurado})");
    true
}

unsafe extern "C" fn api_vtable_hook_owned(handle: u64, vtbl: *mut u64, slot_idx: usize, repl: *const c_void) -> *const c_void {
    match crate::ganchos::trocar_slot_da_vtable(vtbl, slot_idx, repl) {
        Some(orig) => {
            register_vtable_owner(vtbl as usize, slot_idx, handle, orig as usize);
            crate::log_dev!(
                "[api] vtable_hook_owned: handle={handle} vtbl={vtbl:p} slot={slot_idx} -> ok (dono registrado)"
            );
            orig
        }
        None => std::ptr::null(),
    }
}

unsafe extern "C" fn api_vtable_detach(handle: u64, vtbl: *mut u64, slot_idx: usize) -> bool {
    match take_vtable_owner(vtbl as usize, slot_idx) {
        Some((owner, orig)) if owner == handle => {
            crate::ganchos::restaurar_slot_da_vtable(vtbl, slot_idx, orig as *const c_void);
            crate::log_dev!("[api] vtable_detach: handle={handle} vtbl={vtbl:p} slot={slot_idx} -> OK");
            true
        }
        Some((owner, orig)) => {
            register_vtable_owner(vtbl as usize, slot_idx, owner, orig);
            crate::log_dev!(
                "[api] vtable_detach: RECUSADO — handle={handle} tentou soltar slot {slot_idx} de vtbl={vtbl:p} pertencente a handle={owner}"
            );
            false
        }
        None => {
            crate::log_dev!(
                "[api] vtable_detach: vtbl={vtbl:p} slot={slot_idx} sem dono rastreado (handle={handle}) — recusado"
            );
            false
        }
    }
}

unsafe extern "C" fn api_register_native(
    full: *const c_char,
    short: *const c_char,
    handler: crate::register::NativeHandler,
) -> bool {
    if !registration_window_open() {
        crate::log_dev!("[api] register_native: recusado — chamado fora da janela de registro (só durante bwms_plugin_main)");
        return false;
    }
    if full.is_null() || short.is_null() {
        return false;
    }
    let fulls = match CStr::from_ptr(full).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let shorts = match CStr::from_ptr(short).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    // mesmo caminho do register_all: Registry::obtain + proto global clonável (Cos/Sin/...).
    let reg = match crate::rtti::Registry::obtain() {
        Some(r) => r,
        None => return false,
    };
    let mut proto = std::ptr::null_mut();
    for n in ["Cos", "Sin", "AbsF", "SqrtF", "LogF"] {
        let p = crate::register::get_function(&reg, n);
        if crate::rtti::sane(p) {
            proto = p;
            break;
        }
    }
    if !crate::rtti::sane(proto) {
        return false;
    }
    crate::register::register_global(&reg, proto, fulls, shorts, handler)
}

unsafe extern "C" fn api_register_method(
    class: *const c_char,
    full: *const c_char,
    short: *const c_char,
    handler: crate::register::NativeHandler,
) -> bool {
    if !registration_window_open() {
        crate::log_dev!("[api] register_method: recusado — chamado fora da janela de registro (só durante bwms_plugin_main)");
        return false;
    }
    if class.is_null() || full.is_null() || short.is_null() {
        return false;
    }
    let classs = match CStr::from_ptr(class).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let fulls = match CStr::from_ptr(full).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let shorts = match CStr::from_ptr(short).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let reg = match crate::rtti::Registry::obtain() {
        Some(r) => r,
        None => return false,
    };
    let classe_existe = crate::rtti::sane(reg.class_by_name(classs));
    let pedido = MetodoAdiado { classe: classs.to_owned(), nome_cheio: fulls.to_owned(), nome_curto: shorts.to_owned(), handler };
    let adiado = match METODOS_ADIADOS.lock() {
        Ok(mut fila) => fila.adiar(classe_existe, pedido),
        Err(_) => return false,
    };
    if adiado {
        crate::log_dev!("[api] register_method {classs}.{shorts}: classe ainda não existe — adiado até o 1º tick");
        return true;
    }
    let proto = match proto_de_metodo(&reg) {
        Some(p) => p,
        None => return false,
    };
    crate::register::register_method(&reg, classs, proto, fulls, shorts, handler, true)
}

/// Proto do `register_method` de plugin = método estático nativo conhecido (mesmo donor do registrar_fachada_de_versao/regmethod_selftest,
/// provado in-game 2026-07-12).
unsafe fn proto_de_metodo(reg: &crate::rtti::Registry) -> Option<*mut c_void> {
    crate::rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode").map(|rf| rf.func)
}

/// Pedido de `register_method` de plugin cuja classe ainda não existia quando o plugin carregou.
struct MetodoAdiado {
    classe: String,
    nome_cheio: String,
    nome_curto: String,
    handler: crate::register::NativeHandler,
}

/// Os plugins carregam na 1ª validação de classes (`inicio/boot.rs`, `class_validate_probe_hook`), pra criar classe e propriedade antes
/// dela; nessa hora as classes de script (ex.: `PlayerPuppet`) ainda não existem, nem no bind das funções depois do `register_all`
/// (boots 272-274: `[reg] classe 'PlayerPuppet' não existe`). No 1º tick (`pipeline::boot_phase`) elas existem (boots 46 e 271:
/// `register_method ExamplePluginFieldTest -> true`): a fila espera esse ponto e drena uma vez só. Depois da drenagem, classe ausente
/// não é mais adiável.
struct FilaDeMetodosAdiados {
    pedidos: Vec<MetodoAdiado>,
    drenada: bool,
}

impl FilaDeMetodosAdiados {
    /// true = o pedido ficou na fila. Classe viva registra na hora; classe ausente depois da drenagem segue pro registro, que recusa.
    fn adiar(&mut self, classe_existe: bool, pedido: MetodoAdiado) -> bool {
        if classe_existe || self.drenada {
            return false;
        }
        self.pedidos.push(pedido);
        true
    }

    fn drenar(&mut self) -> Vec<MetodoAdiado> {
        self.drenada = true;
        std::mem::take(&mut self.pedidos)
    }
}

static METODOS_ADIADOS: std::sync::Mutex<FilaDeMetodosAdiados> =
    std::sync::Mutex::new(FilaDeMetodosAdiados { pedidos: Vec::new(), drenada: false });

/// Registra os `register_method` de plugin adiados por falta da classe. Chamado só no 1º tick (`pipeline::boot_phase`), antes do
/// `carregar_plugins` daquele ponto; uma 2ª chamada não acha nada.
pub(crate) unsafe fn registrar_metodos_adiados() {
    let pedidos = match METODOS_ADIADOS.lock() {
        Ok(mut fila) => fila.drenar(),
        Err(_) => return,
    };
    if pedidos.is_empty() {
        return;
    }
    let reg = crate::rtti::Registry::obtain();
    let proto = reg.as_ref().and_then(|r| proto_de_metodo(r));
    for p in pedidos {
        let ok = match (&reg, proto) {
            (Some(r), Some(proto)) => crate::register::register_method(r, &p.classe, proto, &p.nome_cheio, &p.nome_curto, p.handler, true),
            _ => false,
        };
        if ok {
            crate::log_dev!("[plugin] register_method {}.{} adiado -> true", p.classe, p.nome_curto);
        } else {
            crate::log_dev!("[plugin] register_method {}.{} adiado FALHOU", p.classe, p.nome_curto);
        }
    }
}

unsafe extern "C" fn api_register_native_argful(
    full: *const c_char,
    short: *const c_char,
    handler: crate::register::NativeHandler,
    param_types: *const *const c_char,
    n_params: usize,
) -> bool {
    if !registration_window_open() {
        crate::log_dev!("[api] register_native_argful: recusado — chamado fora da janela de registro (só durante bwms_plugin_main)");
        return false;
    }
    if full.is_null() || short.is_null() {
        return false;
    }
    let fulls = match CStr::from_ptr(full).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let shorts = match CStr::from_ptr(short).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    // lê os NOMES DOS TIPOS do array de C-strings (null em qualquer um = aborta seguro).
    let mut types: Vec<&str> = Vec::with_capacity(n_params);
    if n_params > 0 {
        if param_types.is_null() {
            return false;
        }
        for i in 0..n_params {
            let p = *param_types.add(i);
            if p.is_null() {
                return false;
            }
            match CStr::from_ptr(p).to_str() {
                Ok(s) => types.push(s),
                Err(_) => return false,
            }
        }
    }
    let reg = match crate::rtti::Registry::obtain() {
        Some(r) => r,
        None => return false,
    };
    // proto = uma global argful já existente (Cos/Sin tomam Float) — mesma descoberta do register_native.
    let mut proto = std::ptr::null_mut();
    for n in ["Cos", "Sin", "AbsF", "SqrtF", "LogF"] {
        let p = crate::register::get_function(&reg, n);
        if crate::rtti::sane(p) {
            proto = p;
            break;
        }
    }
    if !crate::rtti::sane(proto) {
        return false;
    }
    crate::register::register_argful_by_types(&reg, proto, &types, fulls, shorts, handler)
}

unsafe extern "C" fn api_fire_event(name: *const c_char) -> usize {
    if name.is_null() {
        return 0;
    }
    match CStr::from_ptr(name).to_str() {
        Ok(s) => crate::tipos_do_motor::fire_event(s),
        Err(_) => 0,
    }
}

/// Resolve o PONTEIRO da propriedade `field` (por nome, via RTTI) no objeto `obj`. Base dos get/set
/// tipados (v5). null-safe (obj/field nulos → null).
unsafe fn resolve_prop(obj: *mut c_void, field: *const c_char) -> *mut c_void {
    if obj.is_null() || field.is_null() {
        return std::ptr::null_mut();
    }
    let name = match CStr::from_ptr(field).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let cls = crate::rtti::class_of(obj);
    if cls.is_null() {
        return std::ptr::null_mut();
    }
    crate::rtti::find_property_in_class(cls, name)
}

unsafe extern "C" fn api_prop_get_f32(obj: *mut c_void, field: *const c_char) -> f32 {
    let p = resolve_prop(obj, field);
    if p.is_null() {
        return 0.0;
    }
    crate::rtti::prop_get_f32(p, obj)
}
unsafe extern "C" fn api_prop_set_f32(obj: *mut c_void, field: *const c_char, val: f32) -> bool {
    let p = resolve_prop(obj, field);
    if p.is_null() {
        return false;
    }
    crate::rtti::prop_set_f32(p, obj, val)
}
unsafe extern "C" fn api_prop_get_i32(obj: *mut c_void, field: *const c_char) -> i32 {
    let p = resolve_prop(obj, field);
    if p.is_null() {
        return 0;
    }
    crate::rtti::prop_get_u32(p, obj) as i32
}

unsafe extern "C" fn api_call_method_args(
    obj: *mut c_void,
    method: *const c_char,
    args: *const *const c_char,
    n_args: usize,
    ret16: *mut u8,
) -> bool {
    if obj.is_null() || method.is_null() {
        return false;
    }
    let name = match CStr::from_ptr(method).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let Some(parsed) = ler_args(args, n_args) else { return false };
    let cls = crate::rtti::class_of(obj);
    if cls.is_null() {
        return false;
    }
    match crate::rtti::resolve_in_class(cls, name) {
        Some(rf) => match crate::rtti::call_func(&rf, obj, &parsed) {
            Some(r) => {
                if ret16.is_null() {
                    crate::rtti::soltar_retorno(&rf, &r);
                } else {
                    std::ptr::copy_nonoverlapping(r.as_ptr(), ret16, 16);
                }
                true
            }
            None => false,
        },
        None => false,
    }
}

/// Os `n` argumentos em texto (`i:`, `f:`, `b:`, `n:`, `e:`, `s:`, `h:0x…`, o formato do `callf`); `None` com qualquer um nulo ou inválido.
unsafe fn ler_args(args: *const *const c_char, n: usize) -> Option<Vec<crate::rtti::Arg>> {
    if n > 0 && args.is_null() {
        return None;
    }
    (0..n)
        .map(|i| {
            let p = *args.add(i);
            if p.is_null() {
                return None;
            }
            CStr::from_ptr(p).to_str().ok().map(crate::parse_cmd_arg)
        })
        .collect()
}

/// `call_global(nome, args, n, ret16)`: função global de script por nome, com os argumentos do [`ler_args`].
unsafe extern "C" fn api_call_global(name: *const c_char, args: *const *const c_char, n_args: usize, ret16: *mut u8) -> bool {
    if name.is_null() {
        return false;
    }
    let (Ok(nome), Some(parsed)) = (CStr::from_ptr(name).to_str(), ler_args(args, n_args)) else { return false };
    let Some(reg) = crate::rtti::Registry::obtain() else { return false };
    let func = crate::rtti::resolve_global_function_robust(&reg, nome);
    if func.is_null() {
        return false;
    }
    let rf = crate::rtti::ResolvedFn { func, ret_type: crate::rtti::ret_type_of(func), is_static: true };
    match crate::rtti::call_func(&rf, std::ptr::null_mut(), &parsed) {
        Some(r) => {
            if ret16.is_null() {
                crate::rtti::soltar_retorno(&rf, &r);
            } else {
                std::ptr::copy_nonoverlapping(r.as_ptr(), ret16, 16);
            }
            true
        }
        None => false,
    }
}

unsafe extern "C" fn api_tweakdb_get_flat(name: *const c_char, out_bits: *mut u32) -> bool {
    if name.is_null() || out_bits.is_null() {
        return false;
    }
    let names = match CStr::from_ptr(name).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    match crate::tweakdb::api_get_flat_scalar(names) {
        Some(bits) => {
            *out_bits = bits;
            true
        }
        None => false,
    }
}

unsafe extern "C" fn api_tweakdb_set_flat(name: *const c_char, bits: u32) -> bool {
    if name.is_null() {
        return false;
    }
    let names = match CStr::from_ptr(name).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    crate::tweakdb::api_set_flat_scalar(names, bits)
}

/// `class_name` segue na assinatura e não pode ser nulo (o contrato da v8 não muda), mas o valor é ignorado: o clone tira a classe do
/// record base vivo.
unsafe extern "C" fn api_tweakdb_clone_record(
    class_name: *const c_char,
    source: *const c_char,
    new_name: *const c_char,
) -> bool {
    if class_name.is_null() || source.is_null() || new_name.is_null() {
        return false;
    }
    let (s, n) = match (CStr::from_ptr(source).to_str(), CStr::from_ptr(new_name).to_str()) {
        (Ok(s), Ok(n)) => (s, n),
        _ => return false,
    };
    crate::tweakdb::clone_record_api(s, n);
    true
}

unsafe extern "C" fn api_log_level(level: u8, msg: *const c_char) {
    if msg.is_null() {
        return;
    }
    let tag = match level {
        0 => "TRACE",
        1 => "DEBUG",
        2 => "INFO",
        3 => "WARN",
        4 => "ERROR",
        5 => "CRIT",
        _ => "INFO",
    };
    crate::mods_do_usuario::log_de_mod::gravar(&format!("plugin][{tag}"), &CStr::from_ptr(msg).to_string_lossy());
}

/// Parseia "major.minor.patch" (componentes ausentes/não-numéricos viram 0). Descarta o sufixo de
/// pre-release/build do SemVer (tudo a partir do 1º `-` ou `+`) ANTES de separar por `.` — senão
/// `"1.2.3-rc1"` dava `(1,2,0)` (o `"3-rc1"` não parseia como número e o patch caía pra 0). Aceita
/// também um prefixo `v`/`V` (`"v1.2.3"`). Cobre o caso real do `Require` dos plugues com versão de mod.
pub(crate) fn parse_semver_triplet(s: &str) -> (u32, u32, u32) {
    let core = s.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next().unwrap_or(""); // fora pre-release/build
    let mut parts = core.splitn(3, '.');
    let major = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

/// Identificador de pre-release SemVer 2.0 (ex.: em `1.2.3-rc.1`, os identificadores são `rc` e
/// `1`). Numérico vs alfanumérico importa pra precedência (regra 11.4.3 da spec): identificador
/// puramente numérico compara por VALOR; senão compara como string ASCII; numérico sempre tem
/// precedência MENOR que alfanumérico quando comparados entre si.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PreIdent {
    Num(u64),
    Alpha(String),
}

impl PreIdent {
    fn parse(s: &str) -> Self {
        match s.parse::<u64>() {
            Ok(n) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => PreIdent::Num(n),
            _ => PreIdent::Alpha(s.to_string()),
        }
    }
}

impl Ord for PreIdent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            (PreIdent::Num(a), PreIdent::Num(b)) => a.cmp(b),
            (PreIdent::Alpha(a), PreIdent::Alpha(b)) => a.cmp(b),
            (PreIdent::Num(_), PreIdent::Alpha(_)) => Ordering::Less,
            (PreIdent::Alpha(_), PreIdent::Num(_)) => Ordering::Greater,
        }
    }
}
impl PartialOrd for PreIdent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Parseia só a lista de identificadores de pre-release (a parte entre `-` e `+`, dot-separated).
/// `""` (release, sem pre-release) devolve vetor vazio — usado pra decidir a regra 11.3 da spec
/// (uma versão SEM pre-release tem precedência MAIOR que a mesma versão COM pre-release).
fn parse_semver_prerelease(s: &str) -> Vec<PreIdent> {
    let core = s.trim().trim_start_matches(['v', 'V']);
    let after_dash = match core.split_once('-') {
        Some((_, rest)) => rest,
        None => return Vec::new(),
    };
    // o pre-release termina no primeiro '+' (metadado de build, sem peso de precedência).
    let pre = after_dash.split('+').next().unwrap_or("");
    if pre.is_empty() {
        return Vec::new();
    }
    pre.split('.').map(PreIdent::parse).collect()
}

/// Compara duas strings de versão pela precedência COMPLETA do SemVer 2.0 (spec item 11):
/// major.minor.patch primeiro; se empatado, versão sem pre-release > versão com pre-release;
/// se ambas têm pre-release, compara identificador por identificador (numérico por valor,
/// alfanumérico por ASCII, numérico sempre < alfanumérico), e se todos os identificadores em
/// comum empatarem, quem tem MAIS identificadores tem precedência maior.
pub(crate) fn semver_precedence(a: &str, b: &str) -> std::cmp::Ordering {
    let triplet_cmp = parse_semver_triplet(a).cmp(&parse_semver_triplet(b));
    if triplet_cmp != std::cmp::Ordering::Equal {
        return triplet_cmp;
    }
    let (pre_a, pre_b) = (parse_semver_prerelease(a), parse_semver_prerelease(b));
    match (pre_a.is_empty(), pre_b.is_empty()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater, // release > pre-release
        (false, true) => std::cmp::Ordering::Less,
        (false, false) => pre_a.cmp(&pre_b), // Vec<PreIdent>::cmp já compara elemento-a-elemento + tamanho
    }
}

unsafe extern "C" fn api_semver_satisfies(required: *const c_char, actual: *const c_char) -> bool {
    if required.is_null() || actual.is_null() {
        return false;
    }
    let (req, act) = match (CStr::from_ptr(required).to_str(), CStr::from_ptr(actual).to_str()) {
        (Ok(r), Ok(a)) => (r, a),
        _ => return false,
    };
    semver_precedence(act, req) != std::cmp::Ordering::Less
}

unsafe extern "C" fn api_launch_arg(nome: *const c_char, saida: *mut c_char, capacidade: usize) -> i32 {
    if nome.is_null() {
        return -1;
    }
    let nome = CStr::from_ptr(nome).to_string_lossy();
    let Some(v) = crate::configuracao::argumentos::valor(&crate::configuracao::argumentos::do_processo(), &nome) else { return -1 };
    texto_pra_buffer(&v, saida, capacidade)
}

/// Copia `v` pra `saida` com NUL, cortado em `capacidade - 1` bytes; devolve o tamanho inteiro de `v`.
unsafe fn texto_pra_buffer(v: &str, saida: *mut c_char, capacidade: usize) -> i32 {
    if !saida.is_null() && capacidade > 0 {
        let n = v.len().min(capacidade - 1);
        std::ptr::copy_nonoverlapping(v.as_ptr(), saida as *mut u8, n);
        *saida.add(n) = 0;
    }
    v.len() as i32
}

/// `(campo, tipo)` do campo `field` de `obj`, pelo mesmo núcleo das natives de script.
unsafe fn campo_por_nome(obj: *mut c_void, field: *const c_char) -> Option<(*mut c_void, *mut c_void)> {
    crate::rtti::campo_e_tipo(obj, resolve_prop(obj, field))
}

unsafe fn e_string(tipo: *mut c_void) -> bool {
    crate::rtti::type_name_getname(tipo) == crate::cname::cname("String")
}

unsafe extern "C" fn api_prop_type_name(obj: *mut c_void, field: *const c_char, saida: *mut c_char, capacidade: usize) -> i32 {
    match campo_por_nome(obj, field) {
        Some((_, tipo)) => texto_pra_buffer(&crate::cname::resolve_cname(crate::rtti::type_name_getname(tipo)), saida, capacidade),
        None => -1,
    }
}

unsafe extern "C" fn api_prop_to_string(obj: *mut c_void, field: *const c_char, saida: *mut c_char, capacidade: usize) -> i32 {
    match campo_por_nome(obj, field).and_then(|(campo, tipo)| crate::rtti::type_to_string(tipo, campo)) {
        Some(t) => texto_pra_buffer(&t, saida, capacidade),
        None => -1,
    }
}

unsafe extern "C" fn api_prop_get_string(obj: *mut c_void, field: *const c_char, saida: *mut c_char, capacidade: usize) -> i32 {
    match campo_por_nome(obj, field).filter(|(_, tipo)| e_string(*tipo)) {
        Some((campo, _)) => texto_pra_buffer(&crate::jogo::cstring::texto(campo as *const u8), saida, capacidade),
        None => -1,
    }
}

unsafe extern "C" fn api_prop_set_string(obj: *mut c_void, field: *const c_char, texto: *const c_char) -> bool {
    let Some((campo, tipo)) = campo_por_nome(obj, field).filter(|(_, tipo)| e_string(*tipo)) else { return false };
    if texto.is_null() {
        return false;
    }
    let mut temp = [0u64; 4];
    let cs = temp.as_mut_ptr() as *mut u8;
    crate::jogo::cstring::nova(cs, &CStr::from_ptr(texto).to_string_lossy());
    let ok = crate::rtti::type_assign(tipo, campo, cs as *const c_void);
    crate::jogo::cstring::destroi(cs);
    ok
}

unsafe extern "C" fn api_prop_copy(destino: *mut c_void, origem: *mut c_void, field: *const c_char) -> bool {
    match (campo_por_nome(destino, field), campo_por_nome(origem, field)) {
        (Some((cd, td)), Some((co, to))) => crate::rtti::tipo_cabe_em(to, td) && crate::rtti::type_assign(td, cd, co),
        _ => false,
    }
}

unsafe extern "C" fn api_tweak_is_original(id: u64) -> i32 {
    let existe = crate::tweakdb::singleton().is_some_and(|t| !crate::tweakdb::get_record_instance(t, bwms_primitives::tweak_db_id_chave(id)).is_null());
    if !existe {
        return -1;
    }
    crate::tweakdb::registro::original(id) as i32
}
unsafe extern "C" fn api_tweak_original_parent(id: u64) -> u64 {
    crate::tweakdb::registro::pai_original(id).unwrap_or(0)
}
unsafe extern "C" fn api_tweak_descendants(id: u64, saida: *mut u64, capacidade: usize) -> i32 {
    let d = crate::tweakdb::registro::descendentes(id);
    if !saida.is_null() {
        std::ptr::copy_nonoverlapping(d.as_ptr(), saida, d.len().min(capacidade));
    }
    d.len() as i32
}
unsafe extern "C" fn api_tweak_name(id: u64, saida: *mut c_char, capacidade: usize) -> i32 {
    crate::tweakdb::registro::nome(id).map_or(-1, |n| texto_pra_buffer(&n, saida, capacidade))
}
unsafe extern "C" fn api_tweak_remove_record(id: u64) -> bool {
    crate::tweakdb::remover::remover_record(id).is_ok()
}
unsafe fn textos(ps: &[*const c_char]) -> Option<Vec<String>> {
    ps.iter().map(|p| (!p.is_null()).then(|| CStr::from_ptr(*p).to_string_lossy().into_owned()).filter(|s| !s.is_empty())).collect()
}
unsafe extern "C" fn api_take_screenshot(caminho: *const c_char) -> bool {
    match textos(&[caminho]) {
        Some(t) => crate::jogo::motor::screenshot(&t[0], crate::jogo::motor::MODO_NORMAL),
        None => false,
    }
}
unsafe extern "C" fn api_class_create(nome: *const c_char, mae: *const c_char, extra: u32) -> u32 {
    let Some(t) = textos(&[nome, mae]) else { return 0 };
    let Some(reg) = crate::rtti::Registry::obtain() else { return 0 };
    match crate::tipos_do_motor::classe_nativa(&reg, &t[0], &t[1], extra) {
        Ok((_, base)) => base,
        Err(e) => {
            crate::log_dev!("[api] class_create '{}': {e}", t[0]);
            0
        }
    }
}
unsafe extern "C" fn api_class_add_property(classe: *const c_char, nome: *const c_char, tipo: *const c_char, offset: u32) -> bool {
    let Some(t) = textos(&[classe, nome, tipo]) else { return false };
    let Some(reg) = crate::rtti::Registry::obtain() else { return false };
    crate::tipos_do_motor::propriedade_nativa(&reg, &t[0], &t[1], &t[2], offset).map_err(|e| crate::log_dev!("[api] class_add_property: {e}")).is_ok()
}
unsafe extern "C" fn api_tweak_register_name(nome: *const c_char) -> u64 {
    if nome.is_null() {
        return 0;
    }
    let nome = CStr::from_ptr(nome).to_string_lossy();
    if nome.is_empty() { 0 } else { crate::tweakdb::registro::nomear(&nome) }
}

unsafe extern "C" fn api_prop_is_equal(a: *mut c_void, b: *mut c_void, field: *const c_char) -> i32 {
    match (campo_por_nome(a, field), campo_por_nome(b, field)) {
        (Some((ca, ta)), Some((cb, tb))) if ta == tb => crate::rtti::type_is_equal(ta, ca, cb).map_or(-1, |igual| igual as i32),
        _ => -1,
    }
}

/// `compare_semver_prerelease` — precedência SemVer 2.0 COMPLETA como
/// `int32` (convenção `strcmp`: `<0`/`0`/`>0`). String nula/inválida em qualquer lado vira
/// `"0.0.0"` sem pre-release (nunca crasha, nunca panica).
unsafe extern "C" fn api_compare_semver_prerelease(lhs: *const c_char, rhs: *const c_char) -> i32 {
    let l = if lhs.is_null() { "0.0.0" } else { CStr::from_ptr(lhs).to_str().unwrap_or("0.0.0") };
    let r = if rhs.is_null() { "0.0.0" } else { CStr::from_ptr(rhs).to_str().unwrap_or("0.0.0") };
    match semver_precedence(l, r) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Callbacks de draw registrados por plugins (janela própria no terminal). Chamados 1x por quadro com o
/// terminal aberto, por `desenhar_plugins` (`overlay::desenhar_terminal`). `Mutex<Vec<>>` — plugins
/// carregam 1x no boot (thread única), mas o registro pode, em teoria, vir de qualquer thread; a
/// CHAMADA em si é sempre na render.
static PLUGIN_DRAW_CALLBACKS: std::sync::Mutex<Vec<extern "C" fn()>> = std::sync::Mutex::new(Vec::new());

unsafe extern "C" fn api_register_draw_callback(cb: extern "C" fn()) -> bool {
    if !registration_window_open() {
        crate::log_dev!("[api] register_draw_callback: recusado — chamado fora da janela de registro (só durante bwms_plugin_main)");
        return false;
    }
    if let Ok(mut v) = PLUGIN_DRAW_CALLBACKS.lock() {
        v.push(cb);
    }
    true
}

// Janelas de plugin no terminal: as 3 funções abaixo ocupam os slots 22–24 do `BWMS_API` (campos `janela_*`, mesmos offsets e
// assinaturas da v10), e o overlay chama `desenhar_plugins` a cada quadro com o terminal aberto.

thread_local! {
    /// A `Tela` do quadro enquanto um callback de desenho de plugin roda, só na thread de render que a montou. Nula fora disso: aí as 3
    /// funções de janela não fazem nada, e uma chamada de outra thread nunca alcança o quadro.
    static TELA_DOS_PLUGINS: std::cell::Cell<*mut std::ffi::c_void> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Roda `f` (um callback de plugin) com a `Tela` ao alcance das funções de janela. Um panic do plugin para ali; a janela que ele deixou
/// aberta fecha no fim.
pub(crate) fn com_tela_dos_plugins(t: &mut crate::terminal::imediato::Tela<'_>, f: impl FnOnce()) {
    TELA_DOS_PLUGINS.with(|c| c.set(t as *mut crate::terminal::imediato::Tela<'_> as *mut std::ffi::c_void));
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    TELA_DOS_PLUGINS.with(|c| c.set(std::ptr::null_mut()));
    t.fechar_janela_de_plugin();
}

/// Os callbacks de desenho dos plugins, cada um com a `Tela` do quadro (`com_tela_dos_plugins`).
pub(crate) fn desenhar_plugins(t: &mut crate::terminal::imediato::Tela<'_>) {
    let cbs: Vec<extern "C" fn()> = match PLUGIN_DRAW_CALLBACKS.lock() {
        Ok(v) => v.clone(),
        Err(_) => return,
    };
    for cb in cbs {
        com_tela_dos_plugins(t, || cb());
    }
}

pub(crate) fn com_tela<R>(f: impl FnOnce(&mut crate::terminal::imediato::Tela<'static>) -> R) -> Option<R> {
    let p = TELA_DOS_PLUGINS.with(|c| c.get());
    if p.is_null() {
        return None;
    }
    // SAFETY: o ponteiro só fica posto dentro de `com_tela_dos_plugins`, nesta thread, enquanto a `Tela` emprestada lá está viva; as
    // funções de janela não guardam referência nenhuma dela.
    Some(f(unsafe { &mut *(p as *mut crate::terminal::imediato::Tela<'static>) }))
}

/// Abre a janela do plugin pelo título. Fora do desenho devolve `true` sem fazer nada: o plugin segue o caminho normal até o
/// `janela_fechar`, que lá também não faz nada.
pub(crate) unsafe extern "C" fn api_janela_abrir(titulo: *const c_char) -> bool {
    if titulo.is_null() {
        return true;
    }
    let titulo = CStr::from_ptr(titulo).to_string_lossy();
    com_tela(|t| t.abrir_janela_de_plugin(&titulo)).unwrap_or(true)
}

/// Texto na janela aberta do plugin, um item só mesmo com `\n`; sem janela aberta (ou recolhida), nada.
pub(crate) unsafe extern "C" fn api_janela_texto(texto: *const c_char) {
    if texto.is_null() {
        return;
    }
    let texto = CStr::from_ptr(texto).to_string_lossy();
    com_tela(|t| {
        if t.janela_de_plugin_aberta() {
            t.texto(&texto);
        }
    });
}

/// Fecha a janela do plugin; fechar sem ter aberto não faz nada.
pub(crate) unsafe extern "C" fn api_janela_fechar() {
    com_tela(|t| t.fechar_janela_de_plugin());
}

/// Caminho do manifesto persistente (fora do save, mesmo padrão de `~/.bwms-modconfig.txt`). A compilação dos scripts lê o mesmo
/// caminho (`scripts::carregador::texto_do_manifesto`, que o `mods_extras` reparte em mods).
pub(crate) fn scripts_add_manifest_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".bwms-scripts-add.txt"))
}

/// Lê+escreve o manifesto com dedup (1 path por linha). `pub(crate)` pra `bwms-fastboot.sh`/testes
/// não precisarem — o SHELL lê o arquivo direto; isto só existe pro handler da API escrever.
unsafe extern "C" fn api_scripts_add(path: *const c_char) -> bool {
    if path.is_null() {
        return false;
    }
    let p = match CStr::from_ptr(path).to_str() {
        Ok(s) => s.trim().to_string(),
        Err(_) => return false,
    };
    if p.is_empty() {
        return false;
    }
    let manifest = match scripts_add_manifest_path() {
        Some(m) => m,
        None => return false,
    };
    let existing = std::fs::read_to_string(&manifest).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == p) {
        crate::log_dev!("[api] scripts_add: '{p}' já estava no manifesto (sem duplicar)");
        return true;
    }
    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&p);
    content.push('\n');
    match std::fs::write(&manifest, content) {
        Ok(_) => {
            crate::log_dev!(
                "[api] scripts_add: '{p}' registrado em {manifest:?} (efetivo no PRÓXIMO compile+boot, redscript não hot-recarrega)"
            );
            true
        }
        Err(e) => {
            crate::log_dev!("[api] scripts_add: falha ao escrever manifesto '{manifest:?}': {e}");
            false
        }
    }
}

/// Protótipo da API: `plugins::load_one` dá a cada plugin uma cópia (`Box::leak`, vive o processo
/// inteiro — ponteiro sempre válido) com o `my_handle` dele.
pub static BWMS_API: BwmsApi = BwmsApi {
    abi_version: crate::mods_do_usuario::plugins::BWMS_PLUGIN_API,
    log: api_log,
    vtable_hook: api_vtable_hook,
    vtable_unhook: api_vtable_unhook,
    field_ptr: api_field_ptr,
    call_method: api_call_method,
    inline_hook: api_inline_hook,
    inline_revert: api_inline_revert,
    register_native: api_register_native,
    register_native_argful: api_register_native_argful,
    fire_event: api_fire_event,
    prop_get_f32: api_prop_get_f32,
    prop_set_f32: api_prop_set_f32,
    prop_get_i32: api_prop_get_i32,
    call_method_args: api_call_method_args,
    register_method: api_register_method,
    tweakdb_get_flat: api_tweakdb_get_flat,
    tweakdb_set_flat: api_tweakdb_set_flat,
    tweakdb_clone_record: api_tweakdb_clone_record,
    log_level: api_log_level,
    semver_satisfies: api_semver_satisfies,
    register_draw_callback: api_register_draw_callback,
    janela_abrir: api_janela_abrir,
    janela_texto: api_janela_texto,
    janela_fechar: api_janela_fechar,
    scripts_add: api_scripts_add,
    add_game_state: api_add_game_state,
    my_handle: 0, // prototype; `plugins::load_one` clona isto com `my_handle` real por plugin.
    inline_hook_owned: api_inline_hook_owned,
    inline_detach: api_inline_detach,
    vtable_hook_owned: api_vtable_hook_owned,
    vtable_detach: api_vtable_detach,
    compare_semver_prerelease: api_compare_semver_prerelease,
    launch_arg: api_launch_arg,
    prop_type_name: api_prop_type_name,
    prop_to_string: api_prop_to_string,
    prop_get_string: api_prop_get_string,
    prop_set_string: api_prop_set_string,
    prop_copy: api_prop_copy,
    prop_is_equal: api_prop_is_equal,
    tweak_is_original: api_tweak_is_original,
    tweak_original_parent: api_tweak_original_parent,
    tweak_descendants: api_tweak_descendants,
    tweak_name: api_tweak_name,
    tweak_register_name: api_tweak_register_name,
    call_global: api_call_global,
    tweak_remove_record: api_tweak_remove_record,
    class_create: api_class_create,
    class_add_property: api_class_add_property,
    take_screenshot: api_take_screenshot,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn api_version_matches() {
        assert_eq!(BWMS_API.abi_version, crate::mods_do_usuario::plugins::BWMS_PLUGIN_API);
    }

    // Os ponteiros de função são ITENS => não-nulos por construção. Aqui provo os caminhos
    // null-safe (o plugin pode passar lixo sem derrubar o jogo). Tudo bate no guard antes de
    // tocar o RTTI, então roda sem o runtime/jogo (autônomo).
    #[test]
    fn null_safe_paths() {
        unsafe {
            assert!((BWMS_API.vtable_hook)(std::ptr::null_mut(), 0, std::ptr::null()).is_null());
            let f = CString::new("Health").unwrap();
            assert!((BWMS_API.field_ptr)(std::ptr::null_mut(), f.as_ptr()).is_null());
            assert!(!(BWMS_API.call_method)(std::ptr::null_mut(), f.as_ptr(), std::ptr::null_mut()));
            let mut dummy = 0u64;
            let obj = &mut dummy as *mut u64 as *mut c_void;
            assert!((BWMS_API.field_ptr)(obj, std::ptr::null()).is_null());
            assert!(!(BWMS_API.call_method)(obj, std::ptr::null(), std::ptr::null_mut()));
            assert!((BWMS_API.inline_hook)(std::ptr::null_mut(), std::ptr::null_mut()).is_null());
        }
    }

    // handler no-op só p/ satisfazer a assinatura nos testes (nunca é chamado: os guards abortam antes).
    unsafe extern "C" fn test_handler(_c: *mut c_void, _f: *mut c_void, _r: *mut c_void, _rt: i64) {}

    // v4: as duas fns novas são null-safe e não tocam o RTTI antes dos guards (autônomo, sem jogo).
    #[test]
    fn v4_null_safe() {
        unsafe {
            let dummy: crate::register::NativeHandler = test_handler;
            let s = CString::new("X").unwrap();
            // full/short null → false; param_types null com n>0 → false.
            assert!(!(BWMS_API.register_native_argful)(std::ptr::null(), s.as_ptr(), dummy, std::ptr::null(), 0));
            assert!(!(BWMS_API.register_native_argful)(s.as_ptr(), std::ptr::null(), dummy, std::ptr::null(), 0));
            assert!(!(BWMS_API.register_native_argful)(s.as_ptr(), s.as_ptr(), dummy, std::ptr::null(), 2));
            // fire_event(null) → 0; nome válido sem jogo/callbacks → 0 (sem crash).
            assert_eq!((BWMS_API.fire_event)(std::ptr::null()), 0);
            assert_eq!((BWMS_API.fire_event)(s.as_ptr()), 0);
            // v5: get/set tipado null-safe (obj/field nulos → default, sem tocar RTTI).
            assert_eq!((BWMS_API.prop_get_f32)(std::ptr::null_mut(), s.as_ptr()), 0.0);
            assert!(!(BWMS_API.prop_set_f32)(std::ptr::null_mut(), s.as_ptr(), 1.0));
            assert_eq!((BWMS_API.prop_get_i32)(std::ptr::null_mut(), s.as_ptr()), 0);
            let mut d = 0u64;
            let obj = &mut d as *mut u64 as *mut c_void;
            assert_eq!((BWMS_API.prop_get_f32)(obj, std::ptr::null()), 0.0);
            // v6: call_method_args null-safe (obj/method nulos e args null com n>0 → false, sem tocar RTTI).
            assert!(!(BWMS_API.call_method_args)(std::ptr::null_mut(), s.as_ptr(), std::ptr::null(), 0, std::ptr::null_mut()));
            assert!(!(BWMS_API.call_method_args)(obj, std::ptr::null(), std::ptr::null(), 0, std::ptr::null_mut()));
            assert!(!(BWMS_API.call_method_args)(obj, s.as_ptr(), std::ptr::null(), 2, std::ptr::null_mut()));
            // v8: TweakDB null-safe (name nulo, out_bits nulo, ou singleton indisponível fora do
            // jogo real → false/sem crash, nunca toca o TweakDB sem checar antes).
            let mut bits: u32 = 0;
            assert!(!(BWMS_API.tweakdb_get_flat)(std::ptr::null(), &mut bits));
            assert!(!(BWMS_API.tweakdb_get_flat)(s.as_ptr(), std::ptr::null_mut()));
            assert!(!(BWMS_API.tweakdb_get_flat)(s.as_ptr(), &mut bits)); // sem jogo: singleton() None
            assert!(!(BWMS_API.tweakdb_set_flat)(std::ptr::null(), 0));
            assert!(!(BWMS_API.tweakdb_set_flat)(s.as_ptr(), 0)); // sem jogo: singleton() None
            assert!(!(BWMS_API.tweakdb_clone_record)(std::ptr::null(), s.as_ptr(), s.as_ptr()));
            assert!(!(BWMS_API.tweakdb_clone_record)(s.as_ptr(), std::ptr::null(), s.as_ptr()));
            assert!(!(BWMS_API.tweakdb_clone_record)(s.as_ptr(), s.as_ptr(), std::ptr::null()));
            // v9: semver_satisfies null-safe + lógica de comparação (autônomo, sem RTTI/jogo).
            assert!(!(BWMS_API.semver_satisfies)(std::ptr::null(), s.as_ptr()));
            assert!(!(BWMS_API.semver_satisfies)(s.as_ptr(), std::ptr::null()));
        }
    }

    #[test]
    fn semver_satisfies_compara_versoes() {
        unsafe {
            let req = CString::new("1.2.0").unwrap();
            let higher = CString::new("1.3.0").unwrap();
            let equal = CString::new("1.2.0").unwrap();
            let lower = CString::new("1.1.9").unwrap();
            let major_higher = CString::new("2.0.0").unwrap();
            assert!((BWMS_API.semver_satisfies)(req.as_ptr(), higher.as_ptr()));
            assert!((BWMS_API.semver_satisfies)(req.as_ptr(), equal.as_ptr()));
            assert!(!(BWMS_API.semver_satisfies)(req.as_ptr(), lower.as_ptr()));
            assert!((BWMS_API.semver_satisfies)(req.as_ptr(), major_higher.as_ptr()));
            // componente ausente/malformado vira 0 — não crasha, só compara.
            let partial_req = CString::new("1.2").unwrap();
            let partial_act = CString::new("1").unwrap();
            assert!(!(BWMS_API.semver_satisfies)(partial_req.as_ptr(), partial_act.as_ptr()));
        }
    }

    #[test]
    fn parse_semver_triplet_pre_release_e_v_prefixo() {
        // O bug corrigido: pre-release não pode zerar o patch.
        assert_eq!(parse_semver_triplet("1.2.3-rc1"), (1, 2, 3));
        assert_eq!(parse_semver_triplet("1.2.3+build.5"), (1, 2, 3));
        assert_eq!(parse_semver_triplet("1.2.3-rc.1+build"), (1, 2, 3));
        assert_eq!(parse_semver_triplet("v1.2.3"), (1, 2, 3));
        assert_eq!(parse_semver_triplet("V2.0.0-beta"), (2, 0, 0));
        assert_eq!(parse_semver_triplet(" 1.2.3 "), (1, 2, 3));
        // sem o fix, "1.0.0" >= "1.0.0-rc1" seria falso-negativo (rc1 virava 1.0.0 tb, ok aqui;
        // o ponto é o patch não sumir): 1.2.3-rc1 satisfaz require 1.2.3.
        assert!(parse_semver_triplet("1.2.3-rc1") >= parse_semver_triplet("1.2.3"));
    }

    #[test]
    fn semver_precedence_prerelease_sequencia_oficial_semver_org() {
        // Sequência de precedência CRESCENTE do exemplo oficial (semver.org, item 11):
        // 1.0.0-alpha < 1.0.0-alpha.1 < 1.0.0-alpha.beta < 1.0.0-beta < 1.0.0-beta.2
        // < 1.0.0-beta.11 < 1.0.0-rc.1 < 1.0.0
        let seq = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
        ];
        for w in seq.windows(2) {
            assert_eq!(
                semver_precedence(w[0], w[1]),
                std::cmp::Ordering::Less,
                "{} deveria ser < {}",
                w[0],
                w[1]
            );
            assert_eq!(semver_precedence(w[1], w[0]), std::cmp::Ordering::Greater);
        }
        // release > pre-release no mesmo triplet — o bug original: antes disto,
        // "1.2.3-rc1" e "1.2.3" comparavam IGUAIS (o sufixo era descartado antes de comparar).
        assert_eq!(semver_precedence("1.2.3-rc1", "1.2.3"), std::cmp::Ordering::Less);
        assert_eq!(semver_precedence("1.2.3", "1.2.3-rc1"), std::cmp::Ordering::Greater);
        // igual é igual (com e sem pre-release).
        assert_eq!(semver_precedence("1.2.3", "1.2.3"), std::cmp::Ordering::Equal);
        assert_eq!(semver_precedence("1.2.3-rc.1", "1.2.3-rc.1"), std::cmp::Ordering::Equal);
        // build metadata (`+...`) nunca pesa na precedência.
        assert_eq!(semver_precedence("1.2.3+build1", "1.2.3+build2"), std::cmp::Ordering::Equal);
        assert_eq!(semver_precedence("1.2.3-rc.1+build1", "1.2.3-rc.1+build9"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn compare_semver_prerelease_c_abi() {
        unsafe {
            let a = CString::new("1.2.3-rc1").unwrap();
            let b = CString::new("1.2.3").unwrap();
            let eq = CString::new("1.2.3").unwrap();
            assert!((BWMS_API.compare_semver_prerelease)(a.as_ptr(), b.as_ptr()) < 0);
            assert!((BWMS_API.compare_semver_prerelease)(b.as_ptr(), a.as_ptr()) > 0);
            assert_eq!((BWMS_API.compare_semver_prerelease)(b.as_ptr(), eq.as_ptr()), 0);
            // nulo em qualquer lado = trata como "0.0.0", nunca crasha.
            assert!((BWMS_API.compare_semver_prerelease)(std::ptr::null(), b.as_ptr()) < 0);
            assert!((BWMS_API.compare_semver_prerelease)(b.as_ptr(), std::ptr::null()) > 0);
            assert_eq!((BWMS_API.compare_semver_prerelease)(std::ptr::null(), std::ptr::null()), 0);
        }
    }

    // v16: sem objeto ou sem campo nada chega ao RTTI; o texto volta cortado com NUL e com o tamanho inteiro no retorno.
    #[test]
    fn v16_sem_campo_e_texto_cortado() {
        unsafe {
            let f = CString::new("texto").unwrap();
            let mut buf = [0 as c_char; 8];
            let nulo = std::ptr::null_mut();
            assert_eq!((BWMS_API.prop_type_name)(nulo, f.as_ptr(), buf.as_mut_ptr(), buf.len()), -1);
            assert_eq!((BWMS_API.prop_to_string)(nulo, std::ptr::null(), buf.as_mut_ptr(), buf.len()), -1);
            assert_eq!((BWMS_API.prop_get_string)(nulo, f.as_ptr(), buf.as_mut_ptr(), buf.len()), -1);
            assert!(!(BWMS_API.prop_set_string)(nulo, f.as_ptr(), f.as_ptr()));
            assert!(!(BWMS_API.prop_copy)(nulo, nulo, f.as_ptr()));
            assert_eq!((BWMS_API.prop_is_equal)(nulo, nulo, f.as_ptr()), -1);
            assert_eq!((BWMS_API.tweak_register_name)(std::ptr::null()), 0);
            assert_eq!((BWMS_API.class_create)(std::ptr::null(), f.as_ptr(), 8), 0);
            assert!(!(BWMS_API.class_add_property)(f.as_ptr(), std::ptr::null(), f.as_ptr(), 0));
            let nome = CString::new("Items.BwmsTesteApi").unwrap();
            let id = (BWMS_API.tweak_register_name)(nome.as_ptr());
            let mut nome_lido = [0 as c_char; 64];
            assert_eq!((BWMS_API.tweak_name)(id, nome_lido.as_mut_ptr(), nome_lido.len()), 18);
            assert_eq!(CStr::from_ptr(nome_lido.as_ptr()).to_str().unwrap(), "Items.BwmsTesteApi");
            assert_eq!((BWMS_API.tweak_descendants)(id, std::ptr::null_mut(), 0), 0);
            assert_eq!(texto_pra_buffer("abcdefghij", buf.as_mut_ptr(), buf.len()), 10);
            assert_eq!(CStr::from_ptr(buf.as_ptr()).to_str().unwrap(), "abcdefg");
        }
    }

    // Remoção recusada pelo kernel: o detach devolve false e o plugin continua dono do hook que ficou no alvo. Alvo sem hook: nada a desfazer, o dono sai.
    #[test]
    fn inline_detach_com_escrita_recusada_mantem_o_dono() {
        // Acima de todo endereço de usuário do arm64: o `set_prot` da remoção é recusado. Diferente do alvo do teste de `ganchos`, que roda em paralelo.
        const ALVO: u64 = 0x00FF_0000_0001_0000;
        const TRAMP: u64 = 0x7A;
        let alvo = ALVO as *mut c_void;
        crate::ganchos::empilhar_sem_patch(ALVO, TRAMP);
        register_inline_owner(ALVO as usize, 7, TRAMP);
        // SAFETY: o kernel recusa a escrita no alvo antes de qualquer cópia.
        let soltou = unsafe { api_inline_detach(7, alvo) };
        let dono = owned_inline_tramp(ALVO as usize, 7);
        let hooks = crate::ganchos::ganchos_no_endereco(alvo);
        crate::ganchos::esquecer_sem_restaurar(ALVO);
        assert!(!soltou, "detach com a escrita recusada devolveu sucesso");
        assert_eq!(dono, Some(TRAMP), "o plugin perdeu a posse de um hook que continua no alvo");
        assert_eq!(hooks, 1, "a entrada saiu da lista sem o alvo ser restaurado");

        // SAFETY: sem entrada na lista, a remoção não toca o alvo.
        assert!(unsafe { api_inline_detach(7, alvo) }, "alvo sem hook: nada a desfazer");
        assert_eq!(owned_inline_tramp(ALVO as usize, 7), None);
    }

    // log_level não tem valor de retorno pra checar, mas prova que não crasha (inclui msg null).
    #[test]
    fn log_level_nao_crasha() {
        unsafe {
            (BWMS_API.log_level)(2, std::ptr::null());
            let msg = CString::new("teste de log com nivel").unwrap();
            for lvl in 0..=6u8 {
                (BWMS_API.log_level)(lvl, msg.as_ptr());
            }
        }
    }

    /// Plugin compilado da v10 em diante chama as janelas pelo offset: trocar a implementação (D1-F5) ou o nome não pode mover o slot.
    #[test]
    fn slots_das_janelas_de_plugin_ficam_no_lugar() {
        assert_eq!(std::mem::offset_of!(BwmsApi, register_draw_callback), 0xA8);
        assert_eq!(std::mem::offset_of!(BwmsApi, janela_abrir), 0xB0);
        assert_eq!(std::mem::offset_of!(BwmsApi, janela_texto), 0xB8);
        assert_eq!(std::mem::offset_of!(BwmsApi, janela_fechar), 0xC0);
        assert_eq!(std::mem::offset_of!(BwmsApi, take_screenshot), 0x188);
        assert_eq!(std::mem::size_of::<BwmsApi>(), 0x190);
    }

    /// Os slots 22–24 chamam as janelas do terminal próprio: o plugin que abre a janela pelo slot desenha nos widgets do projeto.
    #[test]
    fn slots_22_a_24_chamam_as_janelas_do_terminal() {
        assert_eq!(BWMS_API.janela_abrir as usize, api_janela_abrir as *const () as usize);
        assert_eq!(BWMS_API.janela_texto as usize, api_janela_texto as *const () as usize);
        assert_eq!(BWMS_API.janela_fechar as usize, api_janela_fechar as *const () as usize);
        let titulo = CString::new("Plugin pelo slot").unwrap();
        let mut p = Persistente::default();
        for _ in 0..2 {
            quadro_com_plugin(&mut p, || unsafe {
                (BWMS_API.janela_abrir)(titulo.as_ptr());
                (BWMS_API.janela_fechar)();
            });
        }
        assert_eq!(p.janelas.len(), 2, "a janela aberta pelo slot entrou na tela");
        assert_eq!(p.janelas[1].titulo, "Plugin pelo slot");
    }

    use crate::terminal::imediato::{Entrada, Persistente, Tela};

    /// Um quadro: o terminal e depois o callback do plugin `f`, pelo caminho que o overlay vai usar. Devolve quantos índices a malha teve.
    fn quadro_com_plugin(p: &mut Persistente, f: impl FnOnce()) -> usize {
        let mut t = Tela::nova(Entrada::default(), p, 1);
        t.abrir_janela("terminal");
        com_tela_dos_plugins(&mut t, f);
        t.terminar().malha.indices.len()
    }

    #[test]
    fn janela_de_plugin_nasce_na_frente_mede_no_1o_quadro_e_aparece_no_2o() {
        let titulo = CString::new("Plugin").unwrap();
        let linha = CString::new("oi").unwrap();
        // O callback roda dentro de `catch_unwind`: o que ele devolve sai por fora, senão um assert falso ali passaria em silêncio.
        let abriu = std::cell::Cell::new(false);
        let desenhar = || unsafe {
            abriu.set(api_janela_abrir(titulo.as_ptr()));
            api_janela_texto(linha.as_ptr());
            api_janela_fechar();
        };
        let mut so_terminal = Persistente::default();
        let base = quadro_com_plugin(&mut so_terminal, || {});
        let mut p = Persistente::default();
        assert_eq!(quadro_com_plugin(&mut p, desenhar), base, "no 1º quadro a janela do plugin só mede o conteúdo");
        assert!(quadro_com_plugin(&mut p, desenhar) > base, "no 2º ela aparece");
        assert!(abriu.get(), "aberta, não recolhida");
        assert_eq!((p.ordem.clone(), p.janela_em_foco), (vec![0, 1], Some(1)), "nasce na frente, com o foco");
        // "oi" = 14 px + 2 × 8 de respiro = 30, abaixo do mínimo de 32; altura = 13 + 2 × 8 + 19 de título.
        assert_eq!(p.janelas[1].ret, [60.0, 60.0, 32.0, 48.0]);
    }

    #[test]
    fn janela_de_outra_thread_nao_alcanca_o_quadro() {
        let mut p = Persistente::default();
        let mut devolveu = None;
        quadro_com_plugin(&mut p, || {
            devolveu = std::thread::spawn(|| unsafe {
                let t = CString::new("Intrusa").unwrap();
                let r = api_janela_abrir(t.as_ptr());
                api_janela_fechar();
                r
            })
            .join()
            .ok();
        });
        assert_eq!(devolveu, Some(true), "fora do quadro, abrir devolve true sem fazer nada");
        assert_eq!(p.janelas.len(), 1);
    }

    #[test]
    fn pares_quebrados_nao_derrubam_e_abrir_sem_fechar_fecha_no_fim() {
        let titulo = CString::new("Plugin").unwrap();
        let linha = CString::new("oi").unwrap();
        let mut p = Persistente::default();
        for _ in 0..2 {
            quadro_com_plugin(&mut p, || unsafe {
                api_janela_fechar();
                api_janela_texto(linha.as_ptr());
                api_janela_abrir(titulo.as_ptr());
                api_janela_texto(linha.as_ptr());
            });
        }
        assert_eq!(p.janelas.len(), 2);
        assert_eq!(p.janelas[1].ret[3], 48.0, "o texto de dentro da janela contou no ajuste");
        // Fora de qualquer quadro: nada acontece.
        unsafe {
            api_janela_fechar();
            api_janela_texto(linha.as_ptr());
            assert!(api_janela_abrir(std::ptr::null()));
        }
    }

    /// Dois quadros de plugin com `titulo` e `texto` (o 2º é o que aparece): o retângulo da janela e os índices da malha.
    fn janela_de_plugin_depois_do_ajuste(titulo: &str, texto: Option<&str>) -> ([f32; 4], usize) {
        let titulo = CString::new(titulo).unwrap();
        let texto = texto.map(|t| CString::new(t).unwrap());
        let mut p = Persistente::default();
        let mut indices = 0;
        for _ in 0..2 {
            indices = quadro_com_plugin(&mut p, || unsafe {
                api_janela_abrir(titulo.as_ptr());
                if let Some(t) = &texto {
                    api_janela_texto(t.as_ptr());
                }
                api_janela_fechar();
            });
        }
        (p.janelas[1].ret, indices)
    }

    /// Texto com `\n` é um item de N linhas, como no `igTextUnformatted` da v10: a janela se ajusta às 2 linhas e à mais larga, e o `\r`
    /// do CRLF não ocupa lugar nem desenha.
    #[test]
    fn texto_de_plugin_com_quebra_ajusta_a_janela_as_linhas() {
        let (ret, indices) = janela_de_plugin_depois_do_ajuste("Plugin", Some("linha um\nlinha 2"));
        // 8 colunas × 7 + 2 × 8 de respiro = 72; 2 × 13 + 2 × 8 + 19 de título = 61.
        assert_eq!(ret, [60.0, 60.0, 72.0, 61.0]);
        assert_eq!(janela_de_plugin_depois_do_ajuste("Plugin", Some("linha um\r\nlinha 2\r\n")), (ret, indices));
    }

    /// O título segue o `igBegin` da v10: o que vem depois do `##` não aparece, e com `###` o texto da frente muda sem abrir outra janela.
    #[test]
    fn titulo_de_plugin_esconde_o_identificador() {
        assert_eq!(janela_de_plugin_depois_do_ajuste("Nome##plugin.exemplo", None), janela_de_plugin_depois_do_ajuste("Nome", None));
        let mut p = Persistente::default();
        for titulo in ["FPS: 60###fps", "FPS: 59###fps", "FPS: 58###fps"] {
            let t = CString::new(titulo).unwrap();
            quadro_com_plugin(&mut p, || unsafe {
                api_janela_abrir(t.as_ptr());
                api_janela_fechar();
            });
        }
        assert_eq!(p.janelas.len(), 2, "uma janela de plugin só");
    }

    fn pedido_de_metodo(classe: &str) -> MetodoAdiado {
        MetodoAdiado { classe: classe.into(), nome_cheio: format!("{classe}::Teste"), nome_curto: "Teste".into(), handler: test_handler }
    }

    /// O pedido de classe de script espera o 1º tick e sai da fila uma vez só: a 2ª drenagem não registra de novo, e depois dela classe
    /// ausente não volta pra fila (o registro recusa na hora).
    #[test]
    fn metodo_adiado_drena_uma_vez() {
        let mut fila = FilaDeMetodosAdiados { pedidos: Vec::new(), drenada: false };
        assert!(fila.adiar(false, pedido_de_metodo("PlayerPuppet")));
        let drenados = fila.drenar();
        assert_eq!(drenados.iter().map(|p| (p.classe.as_str(), p.nome_cheio.as_str())).collect::<Vec<_>>(), [("PlayerPuppet", "PlayerPuppet::Teste")]);
        assert!(fila.drenar().is_empty());
        assert!(!fila.adiar(false, pedido_de_metodo("ClasseQueNaoExiste")));
        assert!(fila.drenar().is_empty());
    }

    #[test]
    fn metodo_de_classe_existente_nao_entra_na_fila() {
        let mut fila = FilaDeMetodosAdiados { pedidos: Vec::new(), drenada: false };
        assert!(!fila.adiar(true, pedido_de_metodo("gameGodModeSystem")));
        assert!(fila.drenar().is_empty());
    }

    /// A fila drena no 1º tick, o único ponto em que as provas acham a classe de script viva (boots 46 e 271: `register_method
    /// ExamplePluginFieldTest -> true`); no bind das funções, depois do `register_all`, os boots 272-274 dão `classe 'PlayerPuppet' não
    /// existe`. O ponto depende do boot do jogo, então o teste confere o texto de `src/`:
    /// 1. `cp77_tick` chama `boot_phase`, e `boot_phase` drena a fila antes do seu `carregar_plugins()`;
    /// 2. em `src/` inteiro, fora a definição e este módulo de teste, `registrar_metodos_adiados(` aparece uma vez só, a de `pipeline.rs`:
    ///    nem `bind_orch_hook` nem `class_validate_probe_hook` drenam.
    #[test]
    fn metodos_adiados_drenam_so_no_primeiro_tick() {
        const MARCA_DO_TESTE: &str = "#[cfg(test)]\nmod tests {";
        let corpo = |texto: &'static str, assinatura: &str| -> &'static str {
            let resto = &texto[texto.find(assinatura).unwrap_or_else(|| panic!("`{assinatura}` sumiu"))..];
            &resto[..resto.find("\n}\n").expect("fim da função")]
        };
        assert!(corpo(include_str!("../lib.rs"), "pub extern \"C\" fn cp77_tick()").contains("mods_do_usuario::pipeline::boot_phase();"));
        let boot_phase = corpo(include_str!("pipeline.rs"), "pub(crate) fn boot_phase()");
        let drena = boot_phase.find("api::registrar_metodos_adiados()").expect("boot_phase não drena a fila");
        assert!(drena < boot_phase.find("carregar_plugins();").expect("boot_phase não carrega plugins"));

        let src = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
        let este = src.join("mods_do_usuario/api.rs");
        let mut chamadas = Vec::new();
        let mut pastas = vec![src.clone()];
        while let Some(pasta) = pastas.pop() {
            for item in std::fs::read_dir(&pasta).expect("pasta de src/ legível") {
                let caminho = item.expect("item de src/").path();
                if caminho.is_dir() {
                    pastas.push(caminho);
                    continue;
                }
                if caminho.extension().map_or(true, |ext| ext != "rs") {
                    continue;
                }
                let bytes = std::fs::read(&caminho).expect("fonte .rs legível");
                let texto = String::from_utf8_lossy(&bytes);
                let texto = if caminho == este { &texto[..texto.find(MARCA_DO_TESTE).expect("módulo de teste de api.rs")] } else { &texto[..] };
                // `fn nome(` também casa com `nome(`: a definição sai da contagem.
                let n = texto.matches("registrar_metodos_adiados(").count() - texto.matches("fn registrar_metodos_adiados(").count();
                chamadas.extend(std::iter::repeat(caminho.clone()).take(n));
            }
        }
        assert_eq!(chamadas, vec![src.join("mods_do_usuario/pipeline.rs")], "a fila drena só no 1º tick");
    }
}
