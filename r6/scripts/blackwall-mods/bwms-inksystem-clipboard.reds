// bwms-inksystem-clipboard.reds — `inkSystem.GetClipboardText`/`SetClipboardText` do Codeware (item #100) como globais do BWMS
// (`BwmsGetClipboardText`/`BwmsSetClipboardText`), sem forjar a classe `inkSystem` só pra hospedar 2 métodos sem estado (os outros
// métodos dela estão nos outros `bwms-inksystem-*.reds`). A área de transferência é a do sistema, pela mesma
// ponte NSPasteboard do terminal (`clipboard_get`/`clipboard_set`, terminal/overlay.rs). Ler e escrever não têm limite de tamanho.
native func BwmsGetClipboardText() -> String
native func BwmsSetClipboardText(text: String) -> Bool
