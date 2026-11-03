// Os 3 eventos nativos de customização do jogo
// (`gameuiCharacterCustomizationSystem_On{AppearanceApplied,AppearanceSwitched,OptionUpdated}Event`, usados pelo vanilla em
// `characterCreationBodyMorphMenu.script`) encaminhados pro CallbackSystem do BWMS por `@wrapMethod` na classe vanilla
// `characterCreationBodyMorphMenu`. Essa tela é a da criação de personagem e também a do espelho do mundo
// (`MenuScenario_CharacterCustomizationMirror.OnCCOPuppetReady`), então o despacho cobre os dois.
//
// Despacho em 2 estágios, de propósito: o wrap só chama `BwmsCustomizationEdge(kind)`, que marca um atômico; o evento
// (`CustomizationEvent`) é montado e despachado no tick seguinte do BWMS (`corpo_e_rosto::drain_customization_edges`, chamado do
// `cp77_tick`). Despachar outro evento de dentro de uma chamada de bytecode já em curso é a reentrância que o `ExecDepthGuard` evita.
// `kind`: 0 = AppearanceApplied, 1 = AppearanceSwitched, 2 = OptionUpdated; os eventos se distinguem pelo nome de despacho
// (`Customization/AppearanceApplied` etc.), não por campo do objeto.
//
// O resto da customização em runtime (aparência dinâmica, opções de mod) vive em outros arquivos e no Rust. O disparo
// destes 3 eventos pela tela não tem prova ao vivo: a tela só abre navegando a UI de customização.

public native class CustomizationEvent extends CallbackSystemEvent {}

public static native func BwmsCustomizationEdge(kind: Int32) -> Void

@wrapMethod(characterCreationBodyMorphMenu)
protected cb func OnAppearanceAppliedEvent(evt: ref<gameuiCharacterCustomizationSystem_OnAppearanceAppliedEvent>) -> Bool {
  let result: Bool = wrappedMethod(evt);
  BwmsCustomizationEdge(0);
  return result;
}

@wrapMethod(characterCreationBodyMorphMenu)
protected cb func OnAppearanceSwitched(evt: ref<gameuiCharacterCustomizationSystem_OnAppearanceSwitchedEvent>) -> Bool {
  let result: Bool = wrappedMethod(evt);
  BwmsCustomizationEdge(1);
  return result;
}

@wrapMethod(characterCreationBodyMorphMenu)
protected cb func OnOptionUpdated(evt: ref<gameuiCharacterCustomizationSystem_OnOptionUpdatedEvent>) -> Bool {
  let result: Bool = wrappedMethod(evt);
  BwmsCustomizationEdge(2);
  return result;
}
