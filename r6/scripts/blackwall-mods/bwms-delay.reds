// bwms-delay.reds — agendamento com atraso (catálogo Codeware #56).
//
// O QUE FALTAVA: o jogo tem `DelaySystem.DelayCallback(cb, segundos, ...)`
// (`redscript-all.reds:14521`), mas usá-lo exige que o mod escreva uma subclasse de
// `DelayCallback` para CADA coisa que quer adiar. Na prática isso vira uma dúzia de classes de
// uma linha espalhadas pelo mod.
//
// AQUI: uma classe de callback genérica que chama de volta um alvo por nome, e helpers para os
// dois casos que aparecem sempre — "daqui a N segundos" e "no próximo frame".
//
// SOBRE "PRÓXIMO FRAME": não existe API de próximo-frame no jogo; o que existe é atraso em
// segundos. Um atraso muito pequeno cai no frame seguinte na prática. Isso está dito aqui em vez
// de escondido atrás de um nome que promete precisão que a engine não dá.

public class BwmsDelayedCall extends DelayCallback {

    public let Target: wref<IScriptable>;
    public let FunctionName: CName;

    public func Call() -> Void {
        let target: ref<IScriptable> = this.Target;
        if !IsDefined(target) {
            return;
        }
        // Despacho por reflexão do próprio motor — o mesmo mecanismo que o BWMS já usa para
        // chamar método por nome, sem inventar tabela de callback própria.
        BwmsCallMethod(target, this.FunctionName);
    }
}

public abstract class BwmsDelay {

    // Um frame, na prática. Ver a nota de honestidade no topo.
    public static func OneFrame() -> Float = 0.016

    // Chama `functionName` em `target` daqui a `seconds`. Devolve `false` se não deu para
    // agendar — nunca finge que agendou.
    public static func After(game: GameInstance, seconds: Float,
                             target: ref<IScriptable>, functionName: CName) -> Bool {
        if !IsDefined(target) {
            return false;
        }
        let delaySystem: ref<DelaySystem> = GameInstance.GetDelaySystem(game);
        if !IsDefined(delaySystem) {
            return false;
        }
        let call: ref<BwmsDelayedCall> = new BwmsDelayedCall();
        call.Target = target;
        call.FunctionName = functionName;
        delaySystem.DelayCallback(call, seconds, false);
        return true;
    }

    // O caso mais comum: adiar para depois que a engine terminar o que está fazendo. É o que
    // resolve "montei a UI mas ela ainda não está na árvore".
    public static func NextFrame(game: GameInstance, target: ref<IScriptable>, functionName: CName) -> Bool {
        return BwmsDelay.After(game, BwmsDelay.OneFrame(), target, functionName);
    }
}
