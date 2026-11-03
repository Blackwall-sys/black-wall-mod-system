// Campos do motor visíveis ao script como `native let`, criados pelo BWMS na fase do bind (tabela `CAMPOS_NATIVOS`). Mods chamam direto,
// sem getter: `let pai: wref<inkWidget> = widget.parentWidget;`.

@addField(inkWidget)
public native let parentWidget: wref<inkWidget>;

// O mesh que um componente com esqueleto carregou e o gênero de uma fala. O script vanilla da 2.5 não declara estes tipos (o RTTI
// tem); `CResource` vem do vanilla.
public importonly abstract class resStreamedResource extends CResource {}

public importonly class CMesh extends resStreamedResource {}

public importonly abstract class entISkinTargetComponent extends IVisualComponent {}

public importonly class entSkinnedMeshComponent extends entISkinTargetComponent {}

@addField(entSkinnedMeshComponent)
public native let meshResource: ref<CMesh>;

public importonly struct audioDialogLineEventData {
  public native let gender: Int32;
}
