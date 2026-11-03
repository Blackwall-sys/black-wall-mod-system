// ponte-weathersystem.reds — ler, trocar e restaurar o clima, com nomes do
// BWMS. As leituras de estado são campos de DADO e a troca usa a
// sequência que o próprio motor já tem.
//
// Leitura do estado atual/anterior, pelos campos de `worldRuntimeSystemWeather` (offsets fixos, sem vtable nem hash):
//
//   WeatherSystem+0x40             = worldRuntimeSystemWeather*
//   worldRuntimeSystemWeather+0x8C = cycleWeather: bool
//   worldRuntimeSystemWeather+0x90 = currentStateIndex: uint32
//   worldRuntimeSystemWeather+0x98 = currentSource: CName
//   worldRuntimeSystemWeather+0xB8 = previousStateIndex: uint32
//
// Composição em mundo/mod.rs (`register_weathersystem_state_natives`/`weathersystem_read_state`/`decode_weather_runtime_state`): segue
// `WeatherSystem+0x40 -> worldRuntimeSystemWeather*`, lê um bloco de 0xC0 bytes (`memoria::read_chunk`) e decodifica os 3 campos, sem
// chamar função nenhuma. `WeatherSystem` (`importonly class WeatherSystem extends IScriptable`, `orphans.script:43653`) e
// `GameInstance.GetWeatherSystem(self: GameInstance) -> ref<WeatherSystem>` (`orphans.script:11547`) são vanilla.

native func BwmsWeatherSystemGetCurrentStateIndex(sys: ref<WeatherSystem>) -> Uint32;
native func BwmsWeatherSystemGetCurrentSource(sys: ref<WeatherSystem>) -> CName;
native func BwmsWeatherSystemGetPreviousStateIndex(sys: ref<WeatherSystem>) -> Uint32;

// GetCurrentWeatherStateIndex()/GetPreviousWeatherStateIndex(): índice do estado de clima
// (dentro da tabela `worldEnvironmentDefinition`) atual/anterior. GetCurrentWeatherSource():
// CName da fonte que setou o clima atual (ex. nome da quest/área que forçou o clima, ou
// n"None"/hash-0 se nunca setado nesta sessão). Todos LEITURA PURA — seguro chamar a qualquer
// momento, nunca muta estado.
@addMethod(WeatherSystem)
public func GetCurrentWeatherStateIndex() -> Uint32 {
    return BwmsWeatherSystemGetCurrentStateIndex(this);
}

@addMethod(WeatherSystem)
public func GetCurrentWeatherSource() -> CName {
    return BwmsWeatherSystemGetCurrentSource(this);
}

@addMethod(WeatherSystem)
public func GetPreviousWeatherStateIndex() -> Uint32 {
    return BwmsWeatherSystemGetPreviousStateIndex(this);
}

// TROCAR o clima (`register_weathersystem_set_natives`, mundo/mod.rs): o próprio motor tem a sequência pronta
// (`SetWeatherByName` + ciclo desligado). Nome próprio do BWMS (`SetWeatherState`/`ResetWeatherState`).
//
//   SetWeatherState(n"24h_weather_rain", 10.0, 9)  -> força o estado pelo nome; desliga o ciclo natural
//   ResetWeatherState(true, 5.0)                    -> religa o ciclo; `true` volta já pro estado 0
//
// Muta o mundo: o clima forçado pode ir pro save se o jogador salvar antes de resetar.
native func BwmsWeatherSystemSetWeather(sys: ref<WeatherSystem>, weather: CName, blendTime: Float, priority: Int32) -> Bool;
native func BwmsWeatherSystemResetWeather(sys: ref<WeatherSystem>, forceRestore: Bool, blendTime: Float) -> Bool;

@addMethod(WeatherSystem)
public func SetWeatherState(weather: CName, blendTime: Float, priority: Int32) -> Bool {
    return BwmsWeatherSystemSetWeather(this, weather, blendTime, priority);
}

@addMethod(WeatherSystem)
public func ResetWeatherState(forceRestore: Bool, blendTime: Float) -> Bool {
    return BwmsWeatherSystemResetWeather(this, forceRestore, blendTime);
}

// LER o ambiente e o estado de clima (`GetEnvironmentDefinition`/`GetWeatherState`, com nome próprio do BWMS). Os 2 objetos
// do motor (`worldEnvironmentDefinition` e `worldWeatherState`) não derivam de `IScriptable`: chegam dentro de um `BwmsSerializable`, e o
// `GetObjectClassName()` dele dá a classe. Só leitura.
//
//   let estado: ref<BwmsSerializable> = GameInstance.GetWeatherSystem(game).GetWeatherStateObject();
native func BwmsWeatherSystemGetEnvironmentDefinition(sys: ref<WeatherSystem>) -> ref<BwmsSerializable>;
native func BwmsWeatherSystemGetWeatherState(sys: ref<WeatherSystem>) -> ref<BwmsSerializable>;

@addMethod(WeatherSystem)
public func GetEnvironmentDefinitionObject() -> ref<BwmsSerializable> {
    return BwmsWeatherSystemGetEnvironmentDefinition(this);
}

@addMethod(WeatherSystem)
public func GetWeatherStateObject() -> ref<BwmsSerializable> {
    return BwmsWeatherSystemGetWeatherState(this);
}
