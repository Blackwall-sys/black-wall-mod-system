// bwms-resourcedepot.reds — `ResourceDepot`: `ResourceExists`/`ArchiveExists`, o cache de recursos vivos do
// motor, `RemoveResourceFromCache` e `LoadResource`, com nomes do BWMS (a classe `ResourceDepot` com os nomes que os mods chamam e o
// `GameInstance.GetResourceDepot()` estão em `ponte-nomes-windows.reds`).
native func BwmsResourceExists(path: String) -> Bool
native func BwmsArchiveExists(name: String) -> Bool

// O cache de recursos vivos do motor (`HashMap<ResourcePath, CResource*>` em `+0x00` do bank do loader, `[loader+0x48]`), identificado
// pelo conteúdo: o hash que o motor gera pra `...atlas_common.inkatlas` é chave dele, e o RTTI nomeia o valor (`inkTextureAtlas`). Estas
// duas só consultam o que já está carregado e devolvem fatos, não o `CResource` (o redscript tomaria refcount de um objeto que não possui).
// "Carregado" = presente no cache E utilizável: o `CResource` entra no cache antes de a vtable ser escrita, então
// `BwmsDepotIsResourceLoaded(p)` é exatamente `NotEquals(BwmsDepotLoadedResourceClass(p), "")`.
native func BwmsDepotIsResourceLoaded(path: String) -> Bool
native func BwmsDepotLoadedResourceClass(path: String) -> String

// `BwmsDepotRemoveResourceFromCache(path)` — o `RemoveResourceFromCache`: `ResourceBank::ForgetResource`
// no bank do loader. Tira o recurso do cache e do mapa do bank; o próximo load do path lê de novo do archive. `path` é o hash
// FNV-1a64 do ResRef (`BwmsResRefToHash`). `false` só quando não há loader; o log diz se o bank tinha o recurso.
native func BwmsDepotRemoveResourceFromCache(path: Uint64) -> Bool

// `BwmsDepotLoadResource(path)` — o `ResourceDepot.LoadResource`: pede a carga ao loader do motor e devolve
// o `ResourceToken` (sem esperar; `WaitForJob()` ou `RegisterCallback` pra saber quando terminou). `null` sem loader.
native func BwmsDepotLoadResourceToken(path: Uint64) -> Uint64

public static func BwmsDepotLoadResource(path: ResRef) -> ref<ResourceToken> {
    let hash: Uint64 = BwmsResRefToHash(path);
    let id: Uint64 = BwmsDepotLoadResourceToken(hash);
    if id == 0ul {
        return null;
    }
    return ResourceToken.FromId(id);
}
