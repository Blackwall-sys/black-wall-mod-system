// ponte-log-do-jogo.reds — natives globais de log do jogo que o bundle 2.5 não declara e o binário 2.5 registra. Mod do Nexus chama
// `Log`/`LogChannel` e não compila sem a declaração.
// Registro no Steam 2.5.0: stubs `0x1011c0484`–`0x1011c07cc` (nome → `CGlobalFunction` com handler → `RegisterFunction`, vtable `+0xa0`
// do `CRTTISystem`). Cada handler é o do 2.31 instrução a instrução (só mudam os deslocamentos de página), e o de `Log`/`LogError`/
// `LogWarning` é idêntico ao de `ScriptLog`/`ScriptLogError`/`ScriptLogWarning`, que o bundle 2.5 declara com
// `(const text: script_ref<String>)`. Assinaturas = as do bundle 2.31 (`cp77-symbols/redscript-src/orphans.script`).
// Só compila contra o bundle 2.5: o 2.31 já declara os 8 nomes. Global de propósito: native dentro de `module` não faz bind.
public native func Log(const text: script_ref<String>) -> Void
public native func LogWarning(const text: script_ref<String>) -> Void
public native func LogError(const text: script_ref<String>) -> Void
public native func LogChannel(channel: CName, const text: script_ref<String>) -> Void
public native func LogChannelWarning(channel: CName, const text: script_ref<String>) -> Void
public native func LogChannelError(channel: CName, const text: script_ref<String>) -> Void
public native func Trace() -> Void
public native func TraceToString() -> String
