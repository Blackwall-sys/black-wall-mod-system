// Codeware #153: entidade criada por mod sobrevive a salvar e carregar.
// O BWMS guarda o spec num arquivo PRÓPRIO por save (r6/storage/bwms/entidades) e recria na carga.
// Nada de mod entra no save: desinstalar o BWMS deixa o jogo salvo vanilla e íntegro.
// Este arquivo só informa ao runtime QUAL save está entrando — o menu de load é redscript, então
// não precisa de engenharia reversa pra isso.

native func BwmsSaveCarregado(nome: String) -> Bool;

@addMethod(LoadListItem)
public func BwmsNomeInternoDoSave() -> String {
  if IsDefined(this.m_metadata) {
    return this.m_metadata.internalName;
  }
  return "";
}

@wrapMethod(LoadGameMenuGameController)
private final func LoadGame(controller: ref<LoadListItem>) -> Void {
  let nome: String = controller.BwmsNomeInternoDoSave();
  BwmsSaveCarregado(nome);
  wrappedMethod(controller);
}

// API do #153 pro modder: quantas entidades persistentes a sessão tem, gravar o arquivo de um save
// na hora (o BWMS já grava sozinho quando vê um save novo) e recriar as do save corrente.
native func BwmsEntidadesPersistentes() -> Int32;
native func BwmsGravarEntidadesDoSave(save: String) -> Bool;
native func BwmsRecriarEntidades() -> Int32;
