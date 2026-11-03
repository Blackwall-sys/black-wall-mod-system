// bwms-traceresult-hit.reds — o objeto que um raycast acertou, a partir do próprio TraceResult
// (Codeware `Physics/TraceResult.reds`: `GetHitObject`/`GetHitEntity`, catálogo `#44`/`#210`).
//
// No Codeware são métodos do struct `TraceResult`. Aqui ficam GLOBAIS (`@addMethod` nativo sobre
// struct quebra o compilador) e com nome BWMS.
// O resultado precisa vir de um raycast feito agora (`SpatialQueriesSystem.SyncRaycastBy...`): o
// TraceResult guarda um id escondido, e é por ele que o motor acha o objeto.
//
//   BwmsTraceResultGetHitObject(r)  -> o 1º objeto não vazio do resultado
//   BwmsTraceResultGetHitEntity(r)  -> o 1º que for Entity (o teste é do próprio motor)
//
// Diferença do Codeware: lá o retorno é `ref<ISerializable>`, tipo que o compilador de redscript do
// jogo não conhece. Aqui é `ref<IScriptable>`, e o native só entrega objeto que seja IScriptable de
// verdade — o resto volta null (e fica no log com o nome da classe).

native func BwmsTraceResultGetHitObject(result: script_ref<TraceResult>) -> ref<IScriptable>;
native func BwmsTraceResultGetHitEntity(result: script_ref<TraceResult>) -> ref<Entity>;
// A classe do objeto atingido, seja qual for (o chão é `worldCollisionNodeInstance`, que não sobe como ref).
native func BwmsTraceResultGetHitObjectClass(result: script_ref<TraceResult>) -> CName;
