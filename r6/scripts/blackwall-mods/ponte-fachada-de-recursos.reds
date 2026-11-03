// ponte-fachada-de-recursos.reds — a classe-fachada de recursos: os 8 métodos que mods do Windows chamam.
// As funções que o BWMS criou sobre aparência dinâmica, tags visuais e deslocamento de roupa ficam em `bwms-aparencia.reds`.
//
// Controle: `RegisterArchive`/`RegisterDir` carregam `.archive` em runtime (o diretório sem recursão); `Reload` relê as tabelas que o
// `bwms install` gera dos `.xl` ativos (`resource.link`, fábricas, patches) sem reiniciar o jogo.
//
// `GetBodyType` devolve sempre `n"BaseBody"`: o BWMS não lê o `player.bodyTypes` dos `.xl`. `EnableGarmentOffsets`/`DisableGarmentOffsets`
// são a chave global dos deslocamentos de `BwmsAparencia.AddOffsetOverride` na montagem da roupa.
public abstract native class ArchiveXL {
  public static native func Require(version: String) -> Bool;
  public static native func Version() -> String;
  public static native func RegisterArchive(path: String) -> Bool;
  public static native func RegisterDir(path: String) -> Bool;
  public static native func Reload() -> Void;
  public static native func EnableGarmentOffsets() -> Void;
  public static native func DisableGarmentOffsets() -> Void;
  public static native func GetBodyType(puppet: wref<GameObject>) -> CName;
}
