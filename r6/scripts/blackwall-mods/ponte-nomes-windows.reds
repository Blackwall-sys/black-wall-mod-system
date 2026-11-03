// ponte-nomes-windows.reds — nomes que os mods do Windows chamam, sobre natives que o BWMS já tem com nome
// próprio. Do Windows vem só a forma do plugue (nome + assinatura); o corpo é do BWMS
// (erros de compilação da F0, `tmp/f0-scc/`). O módulo raiz da fachada de versão não é declarado de propósito: o `ModuleExists`
// dele segue `false` e mod com `@if` nesse módulo fica no ramo que roda hoje (medido no scc em 2026-09-28).
//
// Resultado de native vai para um `let` antes de comparar ou repassar (memórias bwms-comparacao-inline-native e
// cp77-native-paramend-args-deslocados).

public func UTF8StrLower(s: String) -> String {
  let minusculo: String = BwmsUTF8StrLower(s);
  return minusculo;
}

// No Mac o archive de mod não fica em `archive/pc/mod/`: vem da pasta `Mods/` e existe só carregado no depot. Caminho de archive do
// Windows é respondido pelo nome do archive carregado.
public func GameFileExists(path: String) -> Bool {
  let noDisco: Bool = BwmsGameFileExists(path);
  if noDisco {
    return true;
  }
  let barra: String = StrReplaceAll(path, "\\", "/");
  if !StrBeginsWith(barra, "archive/pc/mod/") {
    return false;
  }
  let nome: String = StrAfterLast(barra, "/");
  let carregado: Bool = BwmsArchiveExists(nome);
  return carregado;
}

public class ResourceDepot extends IScriptable {
  public func ArchiveExists(name: String) -> Bool {
    let carregado: Bool = BwmsArchiveExists(name);
    return carregado;
  }
}

@addMethod(GameInstance)
public static func GetResourceDepot() -> ref<ResourceDepot> {
  return new ResourceDepot();
}

// O motor registra a classe do record pelo nome nativo (`gamedataClothing_Record`) e o mod passa o de script (`Clothing_Record`);
// o nome que não é classe falha no `GetClass`, antes de varrer a tabela.
@addMethod(TweakDBInterface)
public static func GetRecords(type: CName) -> array<ref<TweakDBRecord>> {
  let saida: array<ref<TweakDBRecord>>;
  let nome: String = NameToString(type);
  let achados: array<ref<IScriptable>> = BwmsGetRecordsArray(nome, 1000000);
  if ArraySize(achados) == 0 && !StrBeginsWith(nome, "gamedata") {
    let nativo: String = "gamedata" + nome;
    achados = BwmsGetRecordsArray(nativo, 1000000);
  }
  for r in achados {
    let rec: ref<TweakDBRecord> = r as TweakDBRecord;
    if IsDefined(rec) {
      ArrayPush(saida, rec);
    }
  }
  return saida;
}

// Posição do filho no widget composto, -1 se não for filho. O campo `children` não é declarado para script na 2.5, então
// a busca vai pelos natives do jogo.
@addMethod(inkCompoundWidget)
public func GetChildIndex(child: wref<inkWidget>) -> Int32 {
  if !IsDefined(child) {
    return -1;
  }
  let total: Int32 = this.GetNumChildren();
  let i: Int32 = 0;
  while i < total {
    let filho: wref<inkWidget> = this.GetWidgetByIndex(i);
    if filho == child {
      return i;
    }
    i += 1;
  }
  return -1;
}
