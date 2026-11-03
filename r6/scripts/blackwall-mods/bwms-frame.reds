// Dados do frame corrente (deltaTime e timeDilation do FrameInfo do motor), pra lógica ligada aos estágios do pipeline: os eventos
// Pipeline/FrameBegin, Pipeline/PreBuckets (antes dos buckets de entidades) e Pipeline/PreRenderUpdate (antes do render), pelo CallbackSystem.
native func BwmsFrameDeltaTime() -> Float;
native func BwmsFrameTimeDilation() -> Float;
