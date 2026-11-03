// bwms-lookat.reds — o que o jogador está olhando.
//
// POR QUE ISTO EXISTE, e por que agora: em VR a pergunta "para onde estou olhando" deixa de ser
// detalhe e vira a interação principal — mirar, destacar, apontar e selecionar passam todos por
// ela. O CPVR precisa disso do lado do jogo, e o BWMS não expunha.
//
// Cobre o caso comum dos itens `#44`/`#210` (o que está sob a mira) por uma via que **não precisa
// de endereço nativo nenhum**: o próprio motor já mantém o alvo sob a mira e o expõe. O raio arbitrário `start -> end` está na
// seção do `#210`, no fim do arquivo; o objeto que um raycast acertou, pelo id escondido do `TraceResult`, está em
// `bwms-traceresult-hit.reds`.
//
// DEPENDÊNCIAS confirmadas por grep ANTES de escrever (`cp77-symbols/redscript-all.reds`):
//   - `GameInstance.GetTargetingSystem(self) -> ref<TargetingSystem>`                (`:14138`)
//   - `TargetingSystem.GetLookAtObject(instigator, withLOS, ignoreTransparent)`      (`:103100`)
//   - `TargetingSystem.GetLookAtComponent(...)`                                      (`:103102`)
//   - `GameObject.GetWorldPosition() -> Vector4`                                     (`:11309`)
//   - `Vector4.Distance(from, to) -> Float`                                          (`:102829`)
// Zero endereço nativo, zero hook, zero forge.

public abstract class BwmsLookAt {

    // O objeto sob a mira. `withLOS` exige linha de visão desobstruída — é o que se quer para
    // interação (não faz sentido "olhar" através de uma parede); passe `false` para depuração.
    public static func Object(observer: ref<GameObject>, withLOS: Bool) -> ref<GameObject> {
        if !IsDefined(observer) {
            return null;
        }
        let targeting: ref<TargetingSystem> = GameInstance.GetTargetingSystem(observer.GetGame());
        if !IsDefined(targeting) {
            return null;
        }
        return targeting.GetLookAtObject(observer, withLOS, true);
    }

    // O componente específico sob a mira — mais fino que o objeto inteiro. Serve para acertar
    // uma parte (cabeça, painel, porta) em vez da entidade toda.
    public static func Component(observer: ref<GameObject>, withLOS: Bool) -> wref<IComponent> {
        let vazio: wref<IComponent>;
        if !IsDefined(observer) {
            return vazio;
        }
        let targeting: ref<TargetingSystem> = GameInstance.GetTargetingSystem(observer.GetGame());
        if !IsDefined(targeting) {
            return vazio;
        }
        return targeting.GetLookAtComponent(observer, withLOS, true);
    }

    // Está olhando para alguma coisa? A pergunta mais barata, para um tick de UI não pagar o
    // custo de resolver o resto quando não há nada ali.
    public static func HasTarget(observer: ref<GameObject>) -> Bool {
        return IsDefined(BwmsLookAt.Object(observer, true));
    }

    // Distância até o que está sob a mira. Negativo = não há alvo — nunca zero, que se confunde
    // com "está encostado".
    public static func Distance(observer: ref<GameObject>) -> Float {
        let alvo: ref<GameObject> = BwmsLookAt.Object(observer, true);
        if !IsDefined(observer) || !IsDefined(alvo) {
            return -1.0;
        }
        return Vector4.Distance(observer.GetWorldPosition(), alvo.GetWorldPosition());
    }

    // O alvo é um NPC? Combina com `bwms-npc-scan.reds`: dá para perguntar a gangue de quem está
    // sob a mira sem escrever a ponte duas vezes.
    public static func NPC(observer: ref<GameObject>) -> ref<NPCPuppet> {
        return BwmsLookAt.Object(observer, true) as NPCPuppet;
    }

    // A facção de quem está sob a mira. `Unaffiliated` quando não há NPC ali — a mesma sentinela
    // que `BwmsNPC.GangOf` usa, para o chamador não ter dois casos de "não sei".
    public static func Gang(observer: ref<GameObject>) -> gamedataAffiliation {
        let npc: ref<NPCPuppet> = BwmsLookAt.NPC(observer);
        if !IsDefined(npc) {
            return gamedataAffiliation.Unaffiliated;
        }
        return BwmsNPC.GangOf(npc);
    }

    public static func GangName(observer: ref<GameObject>) -> String {
        return BwmsNPC.GangName(BwmsLookAt.Gang(observer));
    }

    // ── RAIO ARBITRÁRIO ──────────────────────────────────────────────
    //
    // O caminho nativo: lê um campo `resultID` ESCONDIDO dentro do
    // `TraceResult` e chama `Raw::PhysicsTraceResult::GetHitObject(obj, resultID, i)` para
    // i=0,1 — dois slots, devolvendo o primeiro não-nulo. No BWMS isso é o `bwms-traceresult-hit.reds`.
    //
    // Aqui, a composição só com API vanilla:
    //   `SpatialQueriesSystem.SyncRaycastByCollisionPreset(start, end, preset, out result)`  (`:170160`)
    //     -> devolve o PONTO de impacto (`result.position`), mas não o objeto;
    //   `TargetingSystem.GetTargetClosestByDistance(instigator, query, position, partInfo)`  (`:—`)
    //     -> aceita uma POSIÇÃO ARBITRÁRIA e devolve o objeto alvo mais próximo dela.
    // Compostos, respondem "o que este raio acertou" sem endereço nativo nenhum.
    //
    // LIMITE: `GetTargetClosestByDistance` resolve objetos do sistema de MIRA (entidades). Geometria estática do mundo — parede, chão — tem
    // ponto de impacto mas não vira objeto aqui. Para "acertei o quê?" no sentido amplo, o
    // `BwmsTraceResultGetHitObject`/`GetHitObjectClass` (`bwms-traceresult-hit.reds`) lê o id escondido do `TraceResult`
    // (geometria estática devolve a classe, não o objeto).

    // O PONTO onde um raio arbitrário bate. `false` no retorno = nada no caminho.
    public static func RayHitPoint(observer: ref<GameObject>, start: Vector4, end: Vector4, out ponto: Vector4) -> Bool {
        if !IsDefined(observer) {
            return false;
        }
        let spatial: ref<SpatialQueriesSystem> = GameInstance.GetSpatialQueriesSystem(observer.GetGame());
        if !IsDefined(spatial) {
            return false;
        }
        let r: TraceResult;
        // "World Static" é o preset que o próprio jogo usa para raio de geometria.
        if !spatial.SyncRaycastByCollisionPreset(start, end, n"World Static", r) {
            return false;
        }
        if !TraceResult.IsValid(r) {
            return false;
        }
        ponto = Vector4(r.position.X, r.position.Y, r.position.Z, 1.0);
        return true;
    }

    // O OBJETO que um raio arbitrário acertou. `null` = nada, ou nada que o sistema de mira
    // reconheça como alvo (ver o limite na nota acima — não é o mesmo que "o raio não bateu").
    //
    // POR QUE PELO ARRAY, e não pelo retorno de `GetTargetClosestByDistance`. Aquele método
    // devolve um `ref<GameObject>` que passa no `IsDefined` e **CRASHA no primeiro
    // `GetEntityID()`** — medido ao vivo em 2026-09-05, três rodadas, sempre no mesmo ponto
    // (`SIGSEGV`; a instrumentação imprimiu a linha ANTES da chamada e nunca a seguinte). O
    // retorno não é um handle com contagem; é ponteiro cru. `GetTargetParts` devolve um ARRAY
    // de `TS_TargetPartInfo` e a cadeia `GetComponent -> GetEntity -> GetEntityID` sobre ele
    // foi provada segura na mesma sessão (17-18 entradas reais, id válido).
    //
    // O filtro é `TSF_Any(IntEnum<TSFMV>(31))` — os 5 bits de `Obj_*` (Player/Puppet/Sensor/
    // Device/Other), ou seja QUALQUER tipo de alvo. A máscara de `TSFMV` é bitfield por POSIÇÃO
    // (`1 << enum`); `TSF_NPC()` do jogo usa `2050` = `1<<1 | 1<<11`.
    //
    // Essa máscara chegou a ser acusada de causar o crash, e a acusação era FALSA: as rodadas em
    // que ela rodou morriam antes, no `GetEntityID()` do retorno inseguro. Testada isolada pela
    // rota do array (2026-09-05), ela sobrevive e devolve **128 partes**, contra 17 do
    // `TSF_NPC()` — 7x mais alcance, mesmo custo.
    public static func RayHitObject(observer: ref<GameObject>, start: Vector4, end: Vector4) -> ref<GameObject> {
        let ponto: Vector4;
        if !BwmsLookAt.RayHitPoint(observer, start, end, ponto) {
            return null;
        }
        let targeting: ref<TargetingSystem> = GameInstance.GetTargetingSystem(observer.GetGame());
        if !IsDefined(targeting) {
            return null;
        }
        let q: TargetSearchQuery;
        q.testedSet = TargetingSet.Complete;
        q.searchFilter = TSF_Any(IntEnum<TSFMV>(31));
        q.ignoreInstigator = true;
        let partes: array<TS_TargetPartInfo>;
        targeting.GetTargetParts(observer, q, partes);

        let melhor: ref<GameObject>;
        let menorDist: Float = 0.0;
        let i: Int32 = 0;
        while i < ArraySize(partes) {
            let comp: wref<TargetingComponent> = TS_TargetPartInfo.GetComponent(partes[i]);
            if IsDefined(comp) {
                let ent: ref<Entity> = comp.GetEntity();
                let alvo: ref<GameObject> = ent as GameObject;
                if IsDefined(alvo) {
                    let d: Float = Vector4.Distance(ponto, alvo.GetWorldPosition());
                    if !IsDefined(melhor) || d < menorDist {
                        melhor = alvo;
                        menorDist = d;
                    };
                };
            };
            i += 1;
        };
        return melhor;
    }

    // Conveniência para o caso dominante: "o raio acertou um NPC?".
    public static func RayHitNPC(observer: ref<GameObject>, start: Vector4, end: Vector4) -> ref<NPCPuppet> {
        return BwmsLookAt.RayHitObject(observer, start, end) as NPCPuppet;
    }
}
