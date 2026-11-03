// bwms-job-queue.reds — `JobQueue`: roda o método `Handle` de um objeto de script como job no pool de threads do motor, pela fila
// (`JobQueue`) e pelo despachante que o `MeshComponent::LoadResource` usa. Sem tamanho, `Handle()` roda uma vez; com tamanho, `Handle(from: Int32, to: Int32)`
// roda em lotes que cobrem [0, size) e `Finish(size: Int32)` uma vez, depois de todos os lotes. O último parâmetro `group: Uint64`, opcional nos dois,
// recebe o grupo do job, e `Create(group)` abre uma fila nele pra despachar de dentro do job. `DispatchWait` faz os próximos jobs da fila esperarem o job
// de outra (`Finalize`); `SyncWaitUntilFinished` segura a thread até os jobs da fila terminarem ou o tempo acabar. Os dois fecham a fila. O `Handle`
// roda fora da thread do jogo: só mexe no próprio objeto.

native func BwmsJobQueueCreate(group: Uint64) -> Uint64;
native func BwmsJobQueueDispatch(queue: Uint64, job: ref<IScriptable>, size: Int32, batch: Int32) -> Bool;
native func BwmsJobQueueWait(queue: Uint64, job: Uint64) -> Bool;
native func BwmsJobQueueFinalize(queue: Uint64) -> Uint64;
native func BwmsJobQueueSyncWait(queue: Uint64, timeout: Uint32) -> Bool;

public class BwmsJobQueue extends IScriptable {
    private let m_queue: Uint64;

    public static func Create(opt group: Uint64) -> ref<BwmsJobQueue> {
        let fila: ref<BwmsJobQueue> = new BwmsJobQueue();
        fila.m_queue = BwmsJobQueueCreate(group);
        return fila;
    }

    public func IsValid() -> Bool {
        return this.m_queue != 0ul;
    }

    public func DispatchJob(job: ref<IScriptable>, opt size: Int32, opt batch: Int32) -> Bool {
        let ok: Bool = BwmsJobQueueDispatch(this.m_queue, job, size, batch);
        return ok;
    }

    public func DispatchWait(job: Uint64) -> Bool {
        let ok: Bool = BwmsJobQueueWait(this.m_queue, job);
        return ok;
    }

    public func Finalize() -> Uint64 {
        let job: Uint64 = BwmsJobQueueFinalize(this.m_queue);
        this.m_queue = 0ul;
        return job;
    }

    public func SyncWaitUntilFinished(opt timeout: Uint32) -> Bool {
        let ok: Bool = BwmsJobQueueSyncWait(this.m_queue, timeout);
        this.m_queue = 0ul;
        return ok;
    }
}
