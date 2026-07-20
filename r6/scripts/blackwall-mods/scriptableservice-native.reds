// BWMS — `cw-scriptableservice` (2026-07-15): MESMA receita já provada 2x (Facade sem-extends +
// CallbackSystem extends IGameSystem), zero RE nova pra EXISTÊNCIA das classes. Declaração
// idêntica à fonte real (enablers/Codeware/scripts/Scripting/ScriptableService{,Container}.reds).
//
// 🎉 FECHADO (2026-07-18, sessão `dynarraygrowth-probe`): a saga de crash aberta em 2026-07-15
// ("registrar um MÉTODO cujo param/retorno é `ref<X>` de classe forjada por nós crasha o boot")
// teve a causa raiz ACHADA E CORRIGIDA. NÃO era o validador de tipo (`BindFunctionSignature`,
// REFUTADO em 2026-07-17) — era um HASHMAP EMBUTIDO no forge da `CClass` (offsets +0x78/+0xA8,
// ver `register.rs::fix_embedded_allocator_vtables`) cujo alocador interno ficava com vtable NULO
// (o forge zera a struct inteira e só escreve um punhado de campos conhecidos). Na 1ª inserção
// nesse hashmap (dispara quando uma classe forjada ganha seu 1º método), o engine desreferencia
// esse vtable nulo -> SIGSEGV. Fix: copia o vtable do alocador de uma classe DONOR real (que já
// tem o campo populado, mesmo quando "vazia") pro forjado, ANTES do RegisterType. PROVADO ao vivo:
// boot completo até GAMEPLAY, `GetService` REGISTRADO + CHAMADO via `callon` com sucesso, jogo
// estável. Ver `cp77-symbols/notes/proofs/2026-07-18-cw-scriptableservice-getservice-FIX-PROVADO.log`
// + memória `cp77-native-addr-re-arm64` pro mecanismo completo. Mesma causa provavelmente explica
// o crash histórico de `CallbackSystemHandler` (mesma categoria — ver `callbacksystem-native.reds`)
// já que o fix está no forge COMPARTILHADO (`register_type_min`/`register_type_instantiable_with_
// parent`), usado por toda classe nativa forjada do projeto — não re-testado nesta sessão.

public abstract native class ScriptableService {
  // OnLoad/OnReload/OnInitialize/OnUninitialize da fonte real são callbacks de SCRIPT
  // (comentados na fonte, nada a registrar nativamente aqui).
}

public abstract native class ScriptableServiceContainer extends IGameSystem {
  // `GetService(name: CName) -> ref<ScriptableService>` — histórico da saga de crash (3 tentativas
  // de reativação, causa raiz achada na 3ª):
  //   RETRY 1 (2026-07-17): priming via `GetType("handle:X")` — REFUTADO (GetType devolveu NULL
  //     mesmo pra classe já forjada; não é o writer do cache).
  //   RETRY 2 (2026-07-17, `bindsig-probe`): hook no validador `BindFunctionSignature`
  //     (0x1021ea1b8) confirmou `[type_ref+0x18]` JÁ POPULADO pro retorno/param de GetService —
  //     REFUTA de vez a hipótese "cache de tipo null". Apontou o crash-report pra outro lugar.
  //   RETRY 3 (2026-07-18, `dynarraygrowth-probe`): rastreou o crash-report até a rotina de
  //     crescimento de container (0x10096ca74) — hashmap embutido na CClass com alocador NULO.
  //     Fix implementado (`register.rs::fix_embedded_allocator_vtables`), PROVADO ao vivo (boot→
  //     gameplay, GetService chamado via `callon`, zero crash). FECHADO — handler funcional
  //     abaixo (retorna null por ora: nenhuma instância real de ScriptableService ainda existe,
  //     mesmo padrão documentado em `tramp_cbs_register_callback`; registrar instâncias reais é
  //     gap separado, não bloqueado por crash).
  public native func GetService(name: CName) -> ref<ScriptableService>;
}

// `GameInstance.GetScriptableServiceContainer()` FOI TENTADO como `@addMethod(GameInstance)` e
// CAUSARIA o mesmo crash já documentado pro `GetCallbackSystem` (classe "GameInstance" valida
// MUITO cedo no bundle, antes de qualquer hook nosso rodar) — mesmo fix pragmático: exposto como
// GLOBAL `BwmsGetScriptableServiceContainer()` (register.rs::tramp_get_scriptableservicecontainer,
// já registrado no Rust desde 2026-07-15 — só faltava esta declaração `.reds`, sem ela nenhum mod
// conseguia sequer CHAMAR o native já registrado). Achado 2026-07-18, sessão `handle-ctor-re`.
public static native func BwmsGetScriptableServiceContainer() -> ref<ScriptableServiceContainer>;
