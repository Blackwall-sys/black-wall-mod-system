// ponte-inkinputdisplaycontroller-fields.reds — `@addField(inkInputDisplayController)` pros 4 campos que `PopupButton.CreateWidgets()`
// do Codeware escreve direto (item `#107`, `InMenuPopup`/`PopupButton`).
//
// O `orphans.script:40044` lista só os 6 MÉTODOS nativos vanilla do `inkInputDisplayController` (`SetInputKey`/`SetFixedIconHeight`/
// `GetInputAction`/`SetInputAction`/`SetVisible`/`SetHoldIndicatorType`), sem campo; o header GERADO da reflection do jogo
// (`RED4ext.SDK/include/RED4ext/Scripting/Natives/Generated/ink/InputDisplayController.hpp`) tem os campos, com offsets
// auto-consistentes:
//
//   struct InputDisplayController : ink::WidgetLogicController {   // parent = 0x78 bytes
//       ink::WidgetReference iconRef;       // @0x78 (logo após o parent)
//       ink::WidgetReference nameRef;       // @0x90 (0x78+0x18, WidgetReference=0x18)
//       ink::WidgetReference canvasRef;     // @0xA8 (0x90+0x18)
//       ink::TextWidgetReference iconAND;   // @0xC0 (0xA8+0x18)
//       ...
//       float fixedIconHeight;              // @0x120
//   };
//   RED4EXT_ASSERT_SIZE(InputDisplayController, 0x150);
//
// É o mesmo `@addField` do `enablers/Codeware/scripts/Base/Addons/inkInputDisplayController.reds`: o campo existe na reflection do jogo,
// só falta na API de script padrão — não é a categoria do `CMesh` (tipo inteiro sem presença na reflection). `inkWidgetRef`
// (`WidgetReference`, 0x18 bytes) é tipo vanilla.
//
// Só os 4 campos que o `PopupButton` usa (`iconRef`/`nameRef`/`canvasRef`/`fixedIconHeight`). Escopo global (sem `module`), como em
// `bwms-inkwidget-ext.reds`: declaração `native` dentro de `module X.Y` derruba o bind.
@addField(inkInputDisplayController)
public native let iconRef: inkWidgetRef;

@addField(inkInputDisplayController)
public native let nameRef: inkWidgetRef;

@addField(inkInputDisplayController)
public native let canvasRef: inkWidgetRef;

@addField(inkInputDisplayController)
public native let fixedIconHeight: Float;
