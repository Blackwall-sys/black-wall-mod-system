// bwms-ui-resolutionwatcher.reds — reage a mudança de resolução. Depende de `bwms-ui-screenhelper.reds`.
//
// Os objetos ficam em `array<wref<T>>` no `ScriptableSystem`, nunca `array<ref<T>>`: com `ref`, o motor trata os objetos como posse
// do sistema e, ao destruí-lo no world-swap do autocontinue (`phase=-1`), tenta liberá-los — crash em `red::memory::Free<PoolEngine>`,
// null deref na GameThread. Com `wref` o sistema só observa; quem criou o objeto continua dono dele.
//
// PARA QUE SERVE: UI de mod é montada com tamanhos calculados na resolução do momento. Quando o
// jogador muda a resolução (ou alterna janela/tela cheia), esses números ficam errados e o mod
// não fica sabendo. Este watcher avisa.
//
// COMO: `ScriptableSystem` que guarda a última resolução vista e a compara quando `Poll` é
// chamado. NÃO instala hook nem tick próprio — um sistema que roda a cada frame para vigiar algo
// que muda uma vez por sessão é desperdício, e tick de mod é justamente o que já causou
// travamento neste projeto. Quem quiser vigilância contínua chama `Poll` no próprio tick.

public abstract class BwmsResolutionListener extends IScriptable {
    // Chamado quando a resolução muda. `first` é `true` na primeira leitura (nada mudou ainda,
    // é só o valor inicial) — deixa o listener distinguir "comecei" de "mudou".
    public func OnResolutionChanged(size: BwmsScreenSize, first: Bool) -> Void {}
}

public class BwmsResolutionWatcher extends ScriptableSystem {

    private let m_listeners: array<wref<BwmsResolutionListener>>;
    // CORREÇÃO 2026-08-22 (medido ao vivo): guardar um `struct` PRÓPRIO como campo de
    // `ScriptableSystem` devolveu lixo na leitura (`Width` saiu -65064928 em vez de 1280) e o boot
    // crashou. Campos escalares não têm esse problema — o sistema guarda números, e monta o struct
    // na hora de devolver.
    private let m_lastW: Int32;
    private let m_lastH: Int32;
    private let m_seen: Bool;

    public static func Get(game: GameInstance) -> ref<BwmsResolutionWatcher> {
        let container: ref<ScriptableSystemsContainer> = GameInstance.GetScriptableSystemsContainer(game);
        if !IsDefined(container) {
            return null;
        }
        return container.Get(n"BwmsResolutionWatcher") as BwmsResolutionWatcher;
    }

    public func Register(listener: wref<BwmsResolutionListener>) -> Bool {
        if !IsDefined(listener) || ArrayContains(this.m_listeners, listener) {
            return false;
        }
        ArrayPush(this.m_listeners, listener);
        return true;
    }

    public func Unregister(listener: wref<BwmsResolutionListener>) -> Bool {
        let i: Int32 = ArraySize(this.m_listeners) - 1;
        while i >= 0 {
            if this.m_listeners[i] == listener {
                ArrayErase(this.m_listeners, i);
                return true;
            }
            i -= 1;
        }
        return false;
    }

    // MEDIDO AO VIVO (2026-08-22): devolver um `struct` PRÓPRIO de um método de
    // `ScriptableSystem` não sobrevive ao marshalling — a leitura sai lixo (1871884128 em vez de
    // 1280), mesmo com os campos guardados corretamente. Escalar funciona. Então a API expõe
    // números, não struct. `ScreenHelper.GetScreenSize` continua devolvendo struct porque é
    // classe estática comum, onde isso funciona (provado: 1280x800).
    public func GetLastWidth() -> Int32 {
        return this.m_lastW;
    }

    public func GetLastHeight() -> Int32 {
        return this.m_lastH;
    }

    public func HasReading() -> Bool {
        return this.m_seen;
    }

    // Lê a resolução atual e avisa os listeners se mudou. Devolve `true` quando houve mudança.
    public func Poll() -> Bool {
        let game: GameInstance = GetGameInstance();
        let now: BwmsScreenSize = ScreenHelper.GetScreenSize(game);
        // Leitura inválida não vira notificação: avisar de uma mudança que não houve faria o mod
        // remontar a UI à toa.
        if !now.IsValid {
            return false;
        }
        let first: Bool = !this.m_seen;
        let changed: Bool = first || now.Width != this.m_lastW || now.Height != this.m_lastH;
        if !changed {
            return false;
        }
        this.m_lastW = now.Width;
        this.m_lastH = now.Height;
        this.m_seen = true;
        for listener in this.m_listeners {
            if IsDefined(listener) {
                listener.OnResolutionChanged(now, first);
            }
        }
        return true;
    }
}
