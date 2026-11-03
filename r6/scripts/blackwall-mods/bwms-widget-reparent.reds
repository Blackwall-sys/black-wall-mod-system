// Codeware #90 — a superfície de `inkComponent` por composição: `IsAttached()` e os dois `Reparent`, com nome BWMS.
//
// O Codeware forja a classe `inkComponent` (com o campo `detached` e um `OnConstruct`); aqui ela não é declarada, porque `inkComponent`
// não existe no corpus vanilla (a categoria do `CMesh`, que derruba o bind). Os dois `Reparent` já são nativas vanilla:
//
//   core/ui/baseWidgets/abstractWidgets.script:225
//     inkWidget.Reparent(newParent: wref<inkCompoundWidget>, opt index: Int32) -> Void
//   core/ui/widgetReference.script:215
//     inkWidgetRef.Reparent(self, newParent: wref<inkCompoundWidget>, opt index: Int32) -> Void
//
// O `opt index` cobre as duas formas (com e sem índice). `IsAttached` é `IsDefined(BwmsWidgetParent(w))`: a native sobe um nível
// lendo o `parentWidget` que o motor tem.
//
// Diferença de semântica: o `IsAttached` do Codeware é `widget && !detached`, com `detached` marcado pelo ciclo de vida da classe
// forjada; o daqui é "o widget está pendurado numa árvore". Pra saber se dá pra reparentar ou se já saiu da tela, os dois respondem
// igual; pro ciclo de vida de um `inkComponent` em particular, não.

// PARÂMETRO `ref<IScriptable>` NAS FUNÇÕES GLOBAIS: `ref<inkWidget>` como parâmetro
// de função GLOBAL é o padrão que derrubou o boot no `#91`. O cast
// pra `inkWidget` acontece DENTRO da função, onde é local, e não numa assinatura registrada no RTTI.
native func BwmsWidgetParent(widget: ref<IScriptable>) -> ref<IScriptable>;
native func BwmsReflGetClass(name: CName) -> Uint64;
native func BwmsReflClassGetProperty(cls: Uint64, name: CName) -> Uint64;
native func BwmsReflPropIsSet(obj: ref<IScriptable>, prop: Uint64) -> Bool;

// `IsAttached` — o widget está pendurado numa árvore? Rota canônica: a native já provada.
public func BwmsWidgetIsAttached(w: ref<IScriptable>) -> Bool {
  if !IsDefined(w) { return false; };
  // NUNCA comparar retorno de native inline — o valor vai pra LOCAL antes do teste.
  let pai: ref<IScriptable> = BwmsWidgetParent(w);
  return IsDefined(pai);
}

// Os dois `Reparent`: repasses diretos pra nativa vanilla, com o nome do BWMS.
public func BwmsWidgetReparent(w: ref<IScriptable>, novoPai: wref<inkCompoundWidget>) -> Bool {
  let alvo: ref<inkWidget> = w as inkWidget;
  if !IsDefined(alvo) || !IsDefined(novoPai) { return false; };
  alvo.Reparent(novoPai);
  return true;
}

public func BwmsWidgetReparentAt(w: ref<IScriptable>, novoPai: wref<inkCompoundWidget>, indice: Int32) -> Bool {
  let alvo: ref<inkWidget> = w as inkWidget;
  if !IsDefined(alvo) || !IsDefined(novoPai) { return false; };
  alvo.Reparent(novoPai, indice);
  return true;
}
