// bwms-ink-key-event.reds — a tecla que chega ao widget com foco: tecla, ação e caractere.
// 2026-09-13.
//
// PARA QUE SERVE: campo de texto, atalho num painel, qualquer widget que precise da tecla crua (press e release, inclusive setas, Backspace e
// Enter) e não só do caractere. O motor já entrega ao widget com foco (`InkSystem.inputWidget`) o `OnCharacterKey`, só com o caractere; este
// evento traz a tecla e a ação. A entrega é pelo hook do `ProcessCharacterEvent` do `inkSystem`, ligado quando este arquivo está no bundle.
//
// Os `Is*Down()` herdados de `inkInputEvent` leem o estado do teclado do próprio `InkSystem`.
//
// `GetCharacter()` segue US-QWERTY com shift (a mesma conversão do `ProcessCharacterEvent`
// do Mac), não o layout do sistema.
//
// O corpo mora no Rust (`register_bwmsinkkeyevent`): a classe é forjada no validador, com pai `inkInputEvent`.

public native class BwmsInkKeyEvent extends inkInputEvent {
    public native func GetKey() -> EInputKey
    public native func GetAction() -> EInputAction
    public native func GetCharacter() -> String
    public native func IsCharacter() -> Bool
}
