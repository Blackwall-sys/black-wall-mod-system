// bwms-npc-scan.reds — quem está ao redor, e de que gangue (2026-08-22).
//
// POR QUE ISTO EXISTE: um mod que queira reagir a gangue — spawnar reforço da facção certa,
// mudar preço por reputação, marcar inimigo no HUD, contar presença de Maelstrom num distrito —
// precisa primeiro conseguir a pergunta mais básica: *quem está aqui perto e de que lado?*
// Isso não estava exposto em lugar nenhum do BWMS.
//
// TUDO AQUI É COMPOSIÇÃO DE API VANILLA. Zero endereço nativo, zero forge de classe, zero hook.
// Confirmado por grep em `cp77-symbols/redscript-all.reds` ANTES de escrever:
//   - `GameObject.GetNPCsAroundObject(range) -> [ref<NPCPuppet>]`   (`:12058`)
//   - `class NPCPuppet extends ScriptedPuppet`                      (`:119803`)
//   - `ScriptedPuppet.GetRecord() -> ref<Character_Record>`         (`:20452`)
//   - `Character_Record.Affiliation() -> wref<Affiliation_Record>`  (`:19073`)
//   - `Affiliation_Record.Type() -> gamedataAffiliation`            (uso vanilla, `:39437`)
//   - `enum gamedataAffiliation` com as 29 facções do jogo          (`:4411`)
//   - `GameObject.GetWorldPosition() -> Vector4`                    (`:11309`)
//   - `Vector4.Distance(from, to) -> Float`                         (`:102829`)
//
// DECISÃO DE DESIGN: a API devolve **handles de NPC** e faz perguntas sobre UM npc de cada vez,
// em vez de devolver um array de struct rico. Não é preferência: struct próprio atravessando
// fronteira de marshalling já devolveu lixo neste projeto hoje (o watcher de resolução leu
// 1871884128 em vez de 1280). Handle e escalar atravessam; struct montado por nós, nem sempre.

public abstract class BwmsNPC {

    // Raio padrão. Generoso o bastante para pegar um grupo de rua inteiro, curto o bastante para
    // não varrer meio distrito num frame.
    public static func DefaultRange() -> Float = 40.0

    // --- Quem está aqui ---------------------------------------------------------------------

    public static func Around(observer: ref<GameObject>, range: Float) -> array<ref<NPCPuppet>> {
        let vazio: array<ref<NPCPuppet>>;
        if !IsDefined(observer) {
            return vazio;
        }
        return observer.GetNPCsAroundObject(range);
    }

    public static func CountAround(observer: ref<GameObject>, range: Float) -> Int32 {
        // `ArraySize` direto sobre o array que uma função devolve lê lixo no bytecode do nosso scc (boot de 2026-09-28: 115 no lugar de 1).
        let npcs: array<ref<NPCPuppet>> = BwmsNPC.Around(observer, range);
        return ArraySize(npcs);
    }

    // --- De que lado ------------------------------------------------------------------------

    // A facção do NPC. Devolve `Unaffiliated` quando o registro não resolve — nunca inventa.
    public static func GangOf(npc: ref<NPCPuppet>) -> gamedataAffiliation {
        if !IsDefined(npc) {
            return gamedataAffiliation.Unaffiliated;
        }
        let record: ref<Character_Record> = npc.GetRecord();
        if !IsDefined(record) {
            return gamedataAffiliation.Unaffiliated;
        }
        let affiliation: wref<Affiliation_Record> = record.Affiliation();
        if !IsDefined(affiliation) {
            return gamedataAffiliation.Unaffiliated;
        }
        return affiliation.Type();
    }

    // Nome legível da facção, para log e UI de mod.
    public static func GangNameOf(npc: ref<NPCPuppet>) -> String {
        return BwmsNPC.GangName(BwmsNPC.GangOf(npc));
    }

    public static func GangName(gang: gamedataAffiliation) -> String {
        // Só as facções que um mod realmente distingue no dia a dia; o resto cai no genérico.
        // Preferi cobrir explicitamente a mentir um nome bonito para valor desconhecido.
        if Equals(gang, gamedataAffiliation.Arasaka) { return "Arasaka"; }
        if Equals(gang, gamedataAffiliation.Militech) { return "Militech"; }
        if Equals(gang, gamedataAffiliation.KangTao) { return "Kang Tao"; }
        if Equals(gang, gamedataAffiliation.NetWatch) { return "NetWatch"; }
        if Equals(gang, gamedataAffiliation.Maelstrom) { return "Maelstrom"; }
        if Equals(gang, gamedataAffiliation.MaelstromAndroid) { return "Maelstrom"; }
        if Equals(gang, gamedataAffiliation.TygerClaws) { return "Tyger Claws"; }
        if Equals(gang, gamedataAffiliation.Scavengers) { return "Scavengers"; }
        if Equals(gang, gamedataAffiliation.ScavengersAndroid) { return "Scavengers"; }
        if Equals(gang, gamedataAffiliation.SixthStreet) { return "6th Street"; }
        if Equals(gang, gamedataAffiliation.SixthStreetAndroid) { return "6th Street"; }
        if Equals(gang, gamedataAffiliation.TheMox) { return "The Mox"; }
        if Equals(gang, gamedataAffiliation.Animals) { return "Animals"; }
        if Equals(gang, gamedataAffiliation.Barghest) { return "Barghest"; }
        if Equals(gang, gamedataAffiliation.Aldecaldos) { return "Aldecaldos"; }
        if Equals(gang, gamedataAffiliation.AfterlifeMercs) { return "Afterlife Mercs"; }
        if Equals(gang, gamedataAffiliation.NCPD) { return "NCPD"; }
        if Equals(gang, gamedataAffiliation.TraumaTeam) { return "Trauma Team"; }
        if Equals(gang, gamedataAffiliation.Civilian) { return "Civilian"; }
        if Equals(gang, gamedataAffiliation.Unaffiliated) { return "Unaffiliated"; }
        return "Other";
    }

    // Quantos de uma facção específica estão por perto — a pergunta que um mod de território faz.
    public static func CountGang(observer: ref<GameObject>, range: Float,
                                 gang: gamedataAffiliation) -> Int32 {
        let total: Int32 = 0;
        for npc in BwmsNPC.Around(observer, range) {
            if Equals(BwmsNPC.GangOf(npc), gang) {
                total += 1;
            }
        }
        return total;
    }

    // A facção mais presente ao redor — serve para "de quem é este quarteirão".
    // Ignora civis e não-afiliados: eles estão em todo lugar e não dizem nada sobre território.
    public static func DominantGang(observer: ref<GameObject>, range: Float) -> gamedataAffiliation {
        let melhor: gamedataAffiliation = gamedataAffiliation.Unaffiliated;
        let melhorN: Int32 = 0;
        for npc in BwmsNPC.Around(observer, range) {
            let gang: gamedataAffiliation = BwmsNPC.GangOf(npc);
            if !Equals(gang, gamedataAffiliation.Unaffiliated) && !Equals(gang, gamedataAffiliation.Civilian) {
                let n: Int32 = BwmsNPC.CountGang(observer, range, gang);
                if n > melhorN {
                    melhorN = n;
                    melhor = gang;
                }
            }
        }
        return melhor;
    }

    // --- Onde ---------------------------------------------------------------------------------

    public static func DistanceTo(observer: ref<GameObject>, npc: ref<NPCPuppet>) -> Float {
        if !IsDefined(observer) || !IsDefined(npc) {
            return -1.0;
        }
        return Vector4.Distance(observer.GetWorldPosition(), npc.GetWorldPosition());
    }

    // O mais próximo de uma facção — útil para apontar, marcar no HUD ou mirar.
    public static func NearestOfGang(observer: ref<GameObject>, range: Float,
                                     gang: gamedataAffiliation) -> ref<NPCPuppet> {
        let melhor: ref<NPCPuppet>;
        let melhorD: Float = -1.0;
        for npc in BwmsNPC.Around(observer, range) {
            if Equals(BwmsNPC.GangOf(npc), gang) {
                let d: Float = BwmsNPC.DistanceTo(observer, npc);
                if d >= 0.0 && (melhorD < 0.0 || d < melhorD) {
                    melhorD = d;
                    melhor = npc;
                }
            }
        }
        return melhor;
    }

    // Vivo? Um NPC morto continua na lista de "ao redor" por um tempo — um mod que conta ameaça
    // precisa saber a diferença.
    public static func IsAlive(npc: ref<NPCPuppet>) -> Bool {
        if !IsDefined(npc) {
            return false;
        }
        // `ScriptedPuppet.IsAlive`/`IsDefeated` são estáticas que tomam `ref<GameObject>`
        // (`redscript-all.reds:20783` e `:20790`) — o NPC entra como GameObject, não como puppet.
        let obj: ref<GameObject> = npc;
        return ScriptedPuppet.IsAlive(obj) && !ScriptedPuppet.IsDefeated(obj);
    }

    public static func CountAliveAround(observer: ref<GameObject>, range: Float) -> Int32 {
        let total: Int32 = 0;
        for npc in BwmsNPC.Around(observer, range) {
            if BwmsNPC.IsAlive(npc) {
                total += 1;
            }
        }
        return total;
    }
}
