// bwms-character-input.reds — Codeware `App/UI/WidgetInputService.hpp` (item #176): `IsCharacterInput(EInputKey)` e
// `ToCharacter(EInputKey)`, a parte de conversão tecla→caractere, com nome do BWMS. A entrega da tecla ao widget com foco (o hook do
// `inkSystem::ProcessCharacterEvent`) é o `BwmsInkKeyEvent` (`bwms-ink-key-event.reds`).
//
// Pra letras, dígitos e espaço, `EInputKey` já é o código ASCII do caractere; pontuação e numpad usam tabela ESTÁTICA US-QWERTY
// (`is_character_input_key`/`to_character_key`, entrada/mod.rs). Diferença do Codeware, que usa o `ToUnicode` do Windows (layout e
// locale do usuário): aqui é sempre US-QWERTY, sem Caps Lock persistente, e `shift` é parâmetro explícito.
native func BwmsIsCharacterInput(key: EInputKey) -> Bool
native func BwmsToCharacterKey(key: EInputKey, shift: Bool) -> String
