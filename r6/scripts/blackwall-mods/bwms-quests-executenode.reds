// bwms-quests-executenode.reds — executar um nó de quest a partir do script (Codeware #48).
//
// O Codeware expõe `QuestsSystem.ExecuteNode(node: ref<questNodeDefinition>, opt inputSocket: CName)`.
// No Mac, declarar `questNodeDefinition` no script derruba o jogo: a classe deriva só de
// `ISerializable`, e esse ancestral não passa pelo bind. Por isso o BWMS entrega o nó dentro de um
// `BwmsSerializable` (ver `bwms-serializable.reds`) e o recebe de volta como `ref<IScriptable>`.
//
// `BwmsExecuteNode` roda o nó na fase raiz do sistema de quests, com a trava do próprio sistema,
// como a referência. Devolve o resultado do motor (0 a 3) ou um código negativo:
//   -1 sistema inválido · -2 sem fase raiz · -3 o objeto não é nó de quest
//   -5 sistema ocupado (a trava já está segura, por exemplo num callback do próprio sistema)
// Executar um nó de verdade avança o que o nó faz no jogo (fact, fase, objetivo): é responsabilidade
// do mod que chama.

native func BwmsQuestsSystemExecuteNode(sys: ref<QuestsSystem>, node: ref<IScriptable>, inputSocket: CName) -> Int32;

@addMethod(QuestsSystem)
public func BwmsExecuteNode(node: ref<IScriptable>, opt inputSocket: CName) -> Int32 {
    return BwmsQuestsSystemExecuteNode(this, node, inputSocket);
}
