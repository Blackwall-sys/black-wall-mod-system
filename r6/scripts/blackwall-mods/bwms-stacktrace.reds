// bwms-stacktrace.reds — Codeware `Scripting.GetStackTrace` (item #60).
// Divergência de escopo CONSCIENTE em relação à fonte real (array<StackTraceEntry> completo):
// getter indexado (`GetStackDepth`+`GetStackFrameFunction`/`GetStackFrameClass`), mesmo padrão de
// segurança já usado em `ReflectionClass.GetPropertyCount`/`GetPropertyByIndex` — evita a técnica
// nunca-testada de array-retorno de STRUCT composto (só ponteiros/handles têm essa técnica provada
// neste projeto). Offsets de campo confirmados no header oficial vendorizado (`Scripting/Stack.hpp`).
native func BwmsGetStackDepth() -> Int32
native func BwmsGetStackFrameFunction(index: Int32) -> CName
native func BwmsGetStackFrameClass(index: Int32) -> CName
