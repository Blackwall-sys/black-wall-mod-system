// ponte-journalmanager-tracking.reds — os 6 métodos de rastreio do `JournalManager` do ArchiveXL (`Red/JournalManager.hpp`,
// item #77) sobre natives vanilla do `JournalManager`, sem endereço nem vtable:
//   `GetEntryByHash(hash)`      = `GetEntry(hash)` vanilla.
//   `GetEntryHash(entry)`       já é native vanilla com o mesmo nome; não é redeclarado (seria redefinição).
//   `GetTrackedQuest`/`GetTrackedPointOfInterest` partem do único `GetTrackedEntry()` vanilla.
//   `TrackQuestByPath`/`TrackPointOfInterest` usam `GetEntryByString(path, classe)` + `TrackEntry`, o mesmo caminho que o jogo usa
//   pra shards (`scriptedPuppet.script:563`). Diferença de forma: o ArchiveXL recebe a struct binária `JournalPath`; aqui é o caminho
//   único em string. Classes: `"gameJournalQuest"` e `"gameJournalQuestMapPin"` (o concreto mais comum de `JournalQuestMapPinBase`).
//
// `ChangeEntryState`/`GetEntryState`/`GetEntryTimestamp` (comentados no header do ArchiveXL) e `ChangeEntryStateByHash` já são natives
// vanilla da classe `JournalManager` com os mesmos nomes — chame direto. `GetEntryTimestamp` devolve `GameTime` (o ArchiveXL devolve
// `uint32_t`); `GameTime.GetSeconds` converte.

@addMethod(JournalManager)
public func GetEntryByHash(hash: Uint32) -> wref<JournalEntry> {
    return this.GetEntry(hash);
}

// O tracked entry do jogo é o OBJETIVO (`JournalQuestObjective`), não a quest — o script vanilla assume isso
// (`journalManager.script:179` e `:282` fazem `GetTrackedEntry() as JournalQuestObjective`). Por isso sobe pela cadeia de pais com o
// `GetParentEntry` vanilla até achar a quest, que é o que o `GetTrackedQuest` do ArchiveXL entrega.
@addMethod(JournalManager)
public func GetTrackedQuest() -> wref<JournalQuest> {
    let atual: wref<JournalEntry> = this.GetTrackedEntry();
    let passos: Int32 = 0;
    while IsDefined(atual) && passos < 8 {
        let quest: wref<JournalQuest> = atual as JournalQuest;
        if IsDefined(quest) {
            return quest;
        };
        atual = this.GetParentEntry(atual);
        passos += 1;
    };
    return null;
}

@addMethod(JournalManager)
public func GetTrackedPointOfInterest() -> wref<JournalQuestMapPinBase> {
    return this.GetTrackedEntry() as JournalQuestMapPinBase;
}

@addMethod(JournalManager)
public func TrackQuestByPath(uniquePath: String) -> Bool {
    let entry: wref<JournalEntry> = this.GetEntryByString(uniquePath, "gameJournalQuest");
    if !IsDefined(entry) {
        return false;
    };
    this.TrackEntry(entry);
    return true;
}

@addMethod(JournalManager)
public func TrackPointOfInterest(uniquePath: String) -> Bool {
    let entry: wref<JournalEntry> = this.GetEntryByString(uniquePath, "gameJournalQuestMapPin");
    if !IsDefined(entry) {
        return false;
    };
    this.TrackEntry(entry);
    return true;
}
