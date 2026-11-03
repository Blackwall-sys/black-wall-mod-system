// bwms-leitura-de-records.reds — as natives de leitura de flat e de records, declaradas pra que um
// mod as alcance (registradas no Rust; sem declaração `.reds`, nenhum mod chama).
native func BwmsGetFlatFloat(path: TweakDBID) -> Float
native func BwmsGetRecordCount(className: String) -> Int32
native func BwmsGetRecordByIndex(className: String, index: Int32) -> ref<IScriptable>

// `GetRecords<R>() -> array<ref<R>>`: `BwmsGetRecordsArray` devolve o `array<T>` inteiro (`tramp_records_da_classe`,
// tweakdb/natives.rs): `get_records_of_class` + um buffer com N slots de 16B (`{instance,refCount=0}`, o layout de
// `dynarray_push_handle16`) + 1 slot de 8B de TRAILER com o vft do alocador EMPRESTADO de um `DynArray<Handle<T>>` vivo (`Entity+0xA0`
// do player, `ComponentsStorage.components`) — o mesmo mecanismo do `mkarr`/`set_flat_array_u64`, no stride de 16B. Sem o trailer, o
// teardown do array LOCAL de retorno quebra (SIGSEGV); com ele, `ArraySize`/`IsDefined`/`GetClassName` saem exatos (prova de 2026-08-10).
native func BwmsGetRecordsArray(className: String, limit: Int32) -> array<ref<IScriptable>>
