// -----------------------------------------------------------------------------
// Codeware.Localization.LocalizationSystem / ModLocalizationProvider
// -----------------------------------------------------------------------------
//
// `cw-localization` (2026-07-18, sessão `handle-ctor-re`, 12ª rodada da cadeia). Fonte real:
// `enablers/Codeware/scripts/Localization/LocalizationSystem.reds` +
// `.../Module/ModLocalizationProvider.reds`. ACHADO-CHAVE desta rodada: as duas classes são
// `public class ... extends ScriptableSystem` — **100% REDSCRIPT PURO, ScriptableSystem é NATIVA
// VANILLA** (`core/systems/scriptableSystem.script`, já usada sem forge por dezenas de sistemas
// reais do jogo, ex. `HUDManager`/`FastTravelSystem`/`PreventionSystem`). Diferente de TODOS os
// gaps de CallbackSystem fechados hoje (que exigiram forjar classes nativas do zero via
// `register_type_instantiable_with_parent` + toda a RE de `ref`/`wref`/refcount) — aqui NÃO HÁ
// NENHUM RISCO de RTTI/forge: é o MESMO padrão já usado (e provado seguro) por `codeware-
// delaysystem.reds` (`@addMethod(DelaySystem)`, `DelaySystem`/`IDelaySystem` também nativas
// vanilla). O motor auto-descobre e instancia TODA classe que estende `ScriptableSystem` no RTTI
// compilado — zero registro manual necessário (confirmado pelo padrão universal `GameInstance.
// GetScriptableSystemsContainer(game).Get(n"NomeDaClasse")` usado por dezenas de sistemas vanilla
// reais em `redscript-src/`).
//
// DIVERGÊNCIA DOCUMENTADA (simplificação deliberada, dentro do escopo do `proof_needed`): a fonte
// real usa uma arquitetura de PACOTES por idioma/gênero (`GetPackage(language)->
// ModLocalizationPackage`, `EntryType`, `GenderSensitiveEntry`/`GenderNeutralEntry`, `inkHashMap`,
// fila de requests, watchers de idioma/gênero) — infra grande, não exigida pelo `proof_needed`
// literal ("Mod registra ModLocalizationProvider e GetText('key') devolve texto do mod in-game").
// Implementado aqui: `ModLocalizationProvider.GetText(key: String) -> String` DIRETO (o mod
// override devolve o texto ou "" se não reconhece a chave); `LocalizationSystem.GetText` itera os
// providers registrados e devolve o 1º resultado não-vazio, caindo pra `GetLocalizedText(key)`
// (nativa vanilla real, mesmo fallback usado no `GetTranslationFrom` da fonte original) e por fim
// pra `key` cru. Mantém os NOMES/MÓDULO REAIS (`module Codeware.Localization`, `LocalizationSystem`,
// `ModLocalizationProvider`, `RegisterProvider`, `GetInstance`) — um mod real que faça `import
// Codeware.Localization.*` e implemente só `GetText` (não `GetPackage`/`GetFallback`) resolve
// contra esta implementação sem edição.

module Codeware.Localization

public class LocalizationSystem extends ScriptableSystem {
    private let m_providers: array<ref<ModLocalizationProvider>>;

    public func GetText(key: String) -> String {
        for provider in this.m_providers {
            let text: String = provider.GetText(key);
            if StrLen(text) > 0 {
                return text;
            }
        }

        let fallback: String = GetLocalizedText(key);
        if StrLen(fallback) > 0 {
            return fallback;
        }

        return key;
    }

    public func RegisterProvider(provider: ref<ModLocalizationProvider>) -> Void {
        ArrayPush(this.m_providers, provider);
    }

    public static func GetInstance(game: GameInstance) -> ref<LocalizationSystem> {
        return GameInstance.GetScriptableSystemsContainer(game).Get(n"Codeware.Localization.LocalizationSystem") as LocalizationSystem;
    }
}

public abstract class ModLocalizationProvider extends ScriptableSystem {
    protected func OnAttach() -> Void {
        LocalizationSystem.GetInstance(this.GetGameInstance()).RegisterProvider(this);
    }

    public func GetText(key: String) -> String {
        return "";
    }
}
