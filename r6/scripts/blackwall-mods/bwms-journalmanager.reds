// bwms-journalmanager.reds — Codeware item #47 (`Quest/JournalManager.reds`).
//
// O Codeware declara `@addMethod(JournalManager) public native func GetEntries(request: script_ref<JournalRequestContext>) ->
// array<wref<JournalEntry>>`, que chama `Raw::JournalManager::GetEntries` (`App/Journal/JournalManagerEx.hpp`), um RawFunc genérico
// que decide a categoria por dentro; o endereço dele não está resolvido no Mac.
//
// Aqui, `BwmsGetEntries` recebe o `kind` explícito e despacha pro getter vanilla da categoria — `GetQuests`/`GetMetaQuests`/
// `GetContacts`/`GetTarots`/`GetInternetSites`/`GetInternetPages`/`GetCodexCategories`/`GetBriefings`, todos `native const func` de
// `redscript-src/core/systems/journalManager.script`. Só redscript, sem native do BWMS.
public enum BwmsJournalKind {
    Quests = 0,
    MetaQuests = 1,
    Contacts = 2,
    Tarots = 3,
    InternetSites = 4,
    InternetPages = 5,
    CodexCategories = 6,
    Briefings = 7,
}

@addMethod(JournalManager)
public func BwmsGetEntries(context: JournalRequestContext, kind: BwmsJournalKind, out entries: array<wref<JournalEntry>>) -> Void {
    switch kind {
        case BwmsJournalKind.Quests:
            this.GetQuests(context, entries);
            break;
        case BwmsJournalKind.MetaQuests:
            this.GetMetaQuests(context, entries);
            break;
        case BwmsJournalKind.Contacts:
            this.GetContacts(context, entries);
            break;
        case BwmsJournalKind.Tarots:
            this.GetTarots(context, entries);
            break;
        case BwmsJournalKind.InternetSites:
            this.GetInternetSites(context, entries);
            break;
        case BwmsJournalKind.InternetPages:
            this.GetInternetPages(context, entries);
            break;
        case BwmsJournalKind.CodexCategories:
            this.GetCodexCategories(context, entries);
            break;
        case BwmsJournalKind.Briefings:
            this.GetBriefings(context, entries);
            break;
        default:
            break;
    }
}
