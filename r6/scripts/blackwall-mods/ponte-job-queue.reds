// BWMS — Codeware `#23` (`JobQueue`): atalho SÍNCRONO em fachada 100% estática. `JobQueue.DispatchJob(job)` acha o método `Handle()` do
// objeto e chama na hora, dentro da própria chamada: `true` se achou e chamou, `false` com objeto nulo ou sem `Handle()`.
// `SyncWaitUntilFinished` é no-op — o trabalho já terminou quando `DispatchJob` devolve. A fila de verdade do motor (pool de threads,
// lotes `Handle(de, até)` + `Finish`, `DispatchWait`, `Finalize`) é o `BwmsJobQueue` (`bwms-job-queue.reds`).
public abstract native class JobQueue {
  public static native func DispatchJob(job: ref<IScriptable>) -> Bool;
  public static native func SyncWaitUntilFinished(opt timeout: Uint32) -> Void;
}
