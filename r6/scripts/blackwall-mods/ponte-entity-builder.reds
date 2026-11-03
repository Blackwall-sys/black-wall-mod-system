// -----------------------------------------------------------------------------
// Codeware — `Entity` (item #24): `AddComponent`, `FindComponentByType`, `GetComponents`, `GetTemplatePath`, `SetWorldTransform`,
// `ApplyMorphTarget`.
// -----------------------------------------------------------------------------
//
// O Codeware estende a classe `Entity` existente (`App/Entity/EntityEx.hpp`: `AddComponent` é `components.PushBack(aComponent)`), sem
// passar pelo `EntityBuilder`, que só entra no "Entity/Extract", antes de a entidade existir. Aqui é `@addMethod(Entity)` (mecanismo do
// compilador, sem forja de RTTI) sobre globais nativas do BWMS: `register_method` só tem precedente em classes forjadas pelo BWMS, não
// numa classe vanilla já populada pelo motor como `Entity`. As globais leem e escrevem o `DynArray<Handle<IComponent>>` do motor em
// `Entity+0xA0` (`tramp_entity_add_component` e vizinhas, entidades/mod.rs).

native func BwmsEntityAddComponent(entity: ref<Entity>, component: ref<IComponent>) -> Bool;

@addMethod(Entity)
public func AddComponent(component: ref<IComponent>) -> Void {
    BwmsEntityAddComponent(this, component);
}

// `FindComponentByType`: lê a mesma lista que `AddComponent` escreve, pelo tipo de cada componente. Assinatura do Codeware.
native func BwmsEntityFindComponentByType(entity: ref<Entity>, type: CName) -> ref<IComponent>;

@addMethod(Entity)
public func FindComponentByType(type: CName) -> ref<IComponent> {
    return BwmsEntityFindComponentByType(this, type);
}

// `GetComponents`: o array de retorno usa o trailer do próprio `Entity+0xA0` que está lendo (o array local de retorno precisa do
// trailer do alocador pra não quebrar no teardown).
native func BwmsEntityGetComponents(entity: ref<Entity>) -> array<ref<IComponent>>;

@addMethod(Entity)
public func GetComponents() -> array<ref<IComponent>> {
    return BwmsEntityGetComponents(this);
}

// `GetTemplatePath`: `templatePath: ResourcePath @ Entity+0x60` (header vendorizado `entEntity.hpp`); o `ResourcePath` é o hash de
// 8 bytes, lido direto do campo.
native func BwmsEntityGetTemplatePath(entity: ref<Entity>) -> ResRef;

@addMethod(Entity)
public func GetTemplatePath() -> ResRef {
    return BwmsEntityGetTemplatePath(this);
}

// `SetWorldTransform`: o Codeware chama
// `IPlacedComponent::SetTransform` por endereço; aqui só o `transformComponent@0xB0` é nativo (layout do
// IPlacedComponent no Mac conferido no registro RTTI: `localTransform@0xc0`) e a escrita é o native vanilla
// `SetLocalTransform`. No componente raiz, local = mundo. Devolve false se a entidade não tem componente de
// transformação.
native func BwmsEntityGetTransformComponent(entity: ref<Entity>) -> ref<IPlacedComponent>;

@addMethod(Entity)
public func SetWorldTransform(transform: WorldTransform) -> Bool {
    let placed: ref<IPlacedComponent> = BwmsEntityGetTransformComponent(this);
    if !IsDefined(placed) {
        return false;
    }
    // Os campos do WorldTransform não são visíveis pro script; os getters estáticos são (orphans.script).
    placed.SetLocalTransform(
        WorldPosition.ToVector4(WorldTransform.GetWorldPosition(transform)),
        WorldTransform.GetOrientation(transform)
    );
    return true;
}

// Codeware `#24` `ApplyMorphTarget`: `MorphTargetManager::ApplyMorphTarget` (`0x100c6c0c4`).
native func BwmsEntityApplyMorphTarget(entity: ref<Entity>, target: CName, region: CName, value: Float) -> Bool;

@addMethod(Entity)
public func ApplyMorphTarget(target: CName, region: CName, value: Float) -> Bool {
    return BwmsEntityApplyMorphTarget(this, target, region, value);
}
