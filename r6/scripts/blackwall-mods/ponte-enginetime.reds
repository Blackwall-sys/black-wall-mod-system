// ponte-enginetime.reds — `FromTicks`/`ToTicks`/`GetFrequency` no `EngineTime`
// vanilla. `FromTicks(t)` = `{t}` / `ToTicks(t)` = `t.ticks` —
// `EngineTime` é `importonly struct` vanilla de um único `ticks: Uint64`, a mesma identidade de 8 bytes de CRUID/NodeRef/TweakDBID/
// EntityID/ResRef (`ponte-casts.reds`). `@addMethod(EngineTime)` com os nomes que os mods chamam, como `TDBID.FromNumber`.
native func BwmsTicksToEngineTime(ticks: Uint64) -> EngineTime
native func BwmsEngineTimeToTicks(value: EngineTime) -> Uint64

@addMethod(EngineTime)
public static func FromTicks(value: Uint64) -> EngineTime = BwmsTicksToEngineTime(value)

@addMethod(EngineTime)
public static func ToTicks(self: EngineTime) -> Uint64 = BwmsEngineTimeToTicks(self)

// `GetFrequency()`: ticks por segundo. Os ticks do `EngineTime` são nanossegundos
// (`1e9` por segundo), medido ao vivo contra intervalos reais de 10 s com erro abaixo de 0,01% (prova de 2026-08-17).
@addMethod(EngineTime)
public static func GetFrequency() -> Uint64 = 1000000000ul
