// Valor de campo de qualquer tipo pelo próprio tipo do campo: a cópia, a comparação e o texto que o motor usa no `=`, no `==` e no ToString.
// `prop` vem de BwmsReflClassGetProperty. GetValue devolve Variant (FromVariant<T> desembrulha); SetValue recusa valor de tipo que não cabe no
// campo (handle de classe filha cabe em handle da mãe).
native func BwmsReflPropGetValue(obj: ref<IScriptable>, prop: Uint64) -> Variant;
native func BwmsReflPropSetValue(obj: ref<IScriptable>, prop: Uint64, value: Variant) -> Bool;
native func BwmsReflPropGetValueString(obj: ref<IScriptable>, prop: Uint64) -> String;
native func BwmsReflPropSetValueString(obj: ref<IScriptable>, prop: Uint64, value: String) -> Bool;
native func BwmsReflPropToString(obj: ref<IScriptable>, prop: Uint64) -> String;
native func BwmsReflPropIsEqual(a: ref<IScriptable>, b: ref<IScriptable>, prop: Uint64) -> Bool;
native func BwmsValueToString(value: Variant) -> String;
native func BwmsValueIsEqual(a: Variant, b: Variant) -> Bool;
