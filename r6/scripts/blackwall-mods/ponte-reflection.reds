// -----------------------------------------------------------------------------
// Reflexão — `Reflection`/`ReflectionClass`/`ReflectionProp`/`ReflectionFunc`/`ReflectionEnum` como classes de script
// -----------------------------------------------------------------------------
//
// Como `native class`/`native struct` sem pai vanilla na cadeia, essas classes exigiriam forja de RTTI de tipo desconhecido.
// Aqui são classes COMUNS (sem `native`, sem RegisterType) que guardam um ponteiro opaco (`Uint64`)
// pro `CClass*`/`CProperty*`/`CBaseFunction*`/`CEnum*` real que o Rust resolve, sobre as globais `BwmsRefl*` abaixo.
//
// Sem `module` de propósito: declaração nativa dentro de `module X.Y` ganha o prefixo no `fullName`, o bind não casa com o nome que o
// Rust registra e o jogo trava no boot.
//
// Formas:
// - `GetProperties() -> array<ref<ReflectionProp>>` vira `GetPropertyCount()` + `GetPropertyByIndex(i)`.
// - `ReflectionFunc.Call` só chama SEM argumentos (retorno truncado pra `Bool`).
// - `ReflectionClass.GetFunction` não distingue estático de instância: devolve o 1º com o nome, nos dois arrays (funcs@+0x48/
//   staticFuncs@+0x58).
// - `GetType`/`IsNative`/`IsAbstract`, `GetConstants` de bitfield e a família de enumerar tudo (`GetClasses`/`GetTypes`/
//   `GetDerivedClasses`/`GetEnums`/`GetBitfields`/`GetGlobalFunctions`) não estão aqui; o tipo interno de um composto sai da global
//   `BwmsGetInnerTypeName`.

native func BwmsReflGetClass(name: CName) -> Uint64;
native func BwmsReflGetEnum(name: CName) -> Uint64;
native func BwmsReflGetGlobalFunction(name: CName) -> Uint64;
native func BwmsReflClassGetName(cls: Uint64) -> CName;
native func BwmsReflClassGetParent(cls: Uint64) -> Uint64;
native func BwmsReflClassGetProperty(cls: Uint64, name: CName) -> Uint64;
native func BwmsReflClassGetPropertyCount(cls: Uint64) -> Int32;
native func BwmsReflClassGetPropertyByIndex(cls: Uint64, idx: Int32) -> Uint64;
native func BwmsReflClassGetFunction(cls: Uint64, name: CName) -> Uint64;
native func BwmsReflPropGetName(prop: Uint64) -> CName;
native func BwmsReflPropGetValueFloat(obj: ref<IScriptable>, prop: Uint64) -> Float;
native func BwmsReflPropSetValueFloat(obj: ref<IScriptable>, prop: Uint64, value: Float) -> Bool;
// Acessores de enum/Int32 e CName, os tipos que os campos de widget/UI usam além de `Bool` e `Float`:
// **enum e Int32** (letterCase,
// justification, os dois alinhamentos, overflow policy) e **CName** (fontStyle, textIdKey).
// `-1` é o código de falha do Int32 de propósito: `0` confundiria "falhou" com "é o 1º membro do
// enum". `BwmsReflPropIsSet` responde "essa property de PONTEIRO está preenchida?" devolvendo só
// um Bool — nunca o handle cru, porque handle de tipo não verificado vira chamada virtual num
// objeto que não é aquilo (a lição do `depot_resources_by_class`).
native func BwmsReflPropGetValueInt32(obj: ref<IScriptable>, prop: Uint64) -> Int32;
native func BwmsReflPropSetValueInt32(obj: ref<IScriptable>, prop: Uint64, value: Int32) -> Bool;
native func BwmsReflPropGetValueCName(obj: ref<IScriptable>, prop: Uint64) -> Uint64;
native func BwmsReflPropSetValueCName(obj: ref<IScriptable>, prop: Uint64, value: CName) -> Bool;
native func BwmsReflPropIsSet(obj: ref<IScriptable>, prop: Uint64) -> Bool;
native func BwmsReflPropGetValueBool(obj: ref<IScriptable>, prop: Uint64) -> Bool;
native func BwmsReflPropSetValueBool(obj: ref<IScriptable>, prop: Uint64, value: Bool) -> Bool;
native func BwmsReflFuncGetName(func: Uint64) -> CName;
native func BwmsReflFuncCall(func: Uint64, obj: ref<IScriptable>) -> Bool;
native func BwmsReflEnumGetValue(en: Uint64, name: CName) -> Uint64;
native func BwmsReflMakeHandle(cls: Uint64) -> Uint64;
// `ReflectionType.GetInnerType()`: tipo interno de um
// composto (array/handle/weak-handle). `typeName` é o nome literal do IType (ex.
// `n"array:Float"`/`n"handle:PlayerSystem"` — mesma convenção `"prefixo:Tipo"` já usada por
// `compose_params_from_types`/`GetType`). Devolve `n""` (hash 0) se não-composto ou não achado.
native func BwmsGetInnerTypeName(typeName: CName) -> CName;

public class ReflectionProp {
    private let m_ptr: Uint64;

    public func SetPtr(ptr: Uint64) -> Void {
        this.m_ptr = ptr;
    }
    public func GetPtr() -> Uint64 {
        return this.m_ptr;
    }
    public func IsValid() -> Bool {
        return this.m_ptr != 0ul;
    }

    public func GetName() -> CName {
        return BwmsReflPropGetName(this.m_ptr);
    }

    public func GetValueFloat(obj: ref<IScriptable>) -> Float {
        return BwmsReflPropGetValueFloat(obj, this.m_ptr);
    }
    public func SetValueFloat(obj: ref<IScriptable>, value: Float) -> Bool {
        return BwmsReflPropSetValueFloat(obj, this.m_ptr, value);
    }
    public func GetValueBool(obj: ref<IScriptable>) -> Bool {
        return BwmsReflPropGetValueBool(obj, this.m_ptr);
    }
    public func SetValueBool(obj: ref<IScriptable>, value: Bool) -> Bool {
        return BwmsReflPropSetValueBool(obj, this.m_ptr, value);
    }
}

public class ReflectionFunc {
    private let m_ptr: Uint64;

    public func SetPtr(ptr: Uint64) -> Void {
        this.m_ptr = ptr;
    }
    public func GetPtr() -> Uint64 {
        return this.m_ptr;
    }
    public func IsValid() -> Bool {
        return this.m_ptr != 0ul;
    }

    public func GetName() -> CName {
        return BwmsReflFuncGetName(this.m_ptr);
    }

    // Divergência documentada: só chamada SEM ARGS (ver nota de topo do arquivo).
    public func Call(obj: ref<IScriptable>) -> Bool {
        return BwmsReflFuncCall(this.m_ptr, obj);
    }
}

public class ReflectionEnum {
    private let m_ptr: Uint64;

    public func SetPtr(ptr: Uint64) -> Void {
        this.m_ptr = ptr;
    }
    public func GetPtr() -> Uint64 {
        return this.m_ptr;
    }
    public func IsValid() -> Bool {
        return this.m_ptr != 0ul;
    }

    public func GetValue(name: CName) -> Uint64 {
        return BwmsReflEnumGetValue(this.m_ptr, name);
    }
}

public class ReflectionClass {
    private let m_ptr: Uint64;

    public func SetPtr(ptr: Uint64) -> Void {
        this.m_ptr = ptr;
    }
    public func GetPtr() -> Uint64 {
        return this.m_ptr;
    }
    public func IsValid() -> Bool {
        return this.m_ptr != 0ul;
    }

    public func GetName() -> CName {
        return BwmsReflClassGetName(this.m_ptr);
    }

    public func GetParent() -> ref<ReflectionClass> {
        let parentPtr = BwmsReflClassGetParent(this.m_ptr);
        if parentPtr == 0ul {
            return null;
        }
        let result: ref<ReflectionClass> = new ReflectionClass();
        result.SetPtr(parentPtr);
        return result;
    }

    public func GetProperty(name: CName) -> ref<ReflectionProp> {
        let propPtr = BwmsReflClassGetProperty(this.m_ptr, name);
        if propPtr == 0ul {
            return null;
        }
        let result: ref<ReflectionProp> = new ReflectionProp();
        result.SetPtr(propPtr);
        return result;
    }

    // Divergência documentada: substitui `GetProperties()->array<ref<ReflectionProp>>` (ver nota
    // de topo do arquivo) por count+índice.
    public func GetPropertyCount() -> Int32 {
        return BwmsReflClassGetPropertyCount(this.m_ptr);
    }
    public func GetPropertyByIndex(idx: Int32) -> ref<ReflectionProp> {
        let propPtr = BwmsReflClassGetPropertyByIndex(this.m_ptr, idx);
        if propPtr == 0ul {
            return null;
        }
        let result: ref<ReflectionProp> = new ReflectionProp();
        result.SetPtr(propPtr);
        return result;
    }

    public func GetFunction(name: CName) -> ref<ReflectionFunc> {
        let funcPtr = BwmsReflClassGetFunction(this.m_ptr, name);
        if funcPtr == 0ul {
            return null;
        }
        let result: ref<ReflectionFunc> = new ReflectionFunc();
        result.SetPtr(funcPtr);
        return result;
    }

    // `MakeHandle()` (item #51): devolve o ponteiro OPACO da instância nova
    // (Uint64, mesma convenção deste arquivo inteiro — não um `ref<T>` de verdade).
    public func MakeHandle() -> Uint64 {
        return BwmsReflMakeHandle(this.m_ptr);
    }
}

public class Reflection {
    public static func GetClass(name: CName) -> ref<ReflectionClass> {
        let clsPtr = BwmsReflGetClass(name);
        if clsPtr == 0ul {
            return null;
        }
        let result: ref<ReflectionClass> = new ReflectionClass();
        result.SetPtr(clsPtr);
        return result;
    }

    public static func GetEnum(name: CName) -> ref<ReflectionEnum> {
        let enPtr = BwmsReflGetEnum(name);
        if enPtr == 0ul {
            return null;
        }
        let result: ref<ReflectionEnum> = new ReflectionEnum();
        result.SetPtr(enPtr);
        return result;
    }

    public static func GetGlobalFunction(name: CName) -> ref<ReflectionFunc> {
        let funcPtr = BwmsReflGetGlobalFunction(name);
        if funcPtr == 0ul {
            return null;
        }
        let result: ref<ReflectionFunc> = new ReflectionFunc();
        result.SetPtr(funcPtr);
        return result;
    }
}
