// bwms-visualcontroller.reds — Codeware `#213` (`Red/VisualController.hpp`): o `VisualController` interno do
// `entVisualControllerComponent` (`Controller@0xD8`), que não é RTTI visível ao script. São 4 leituras de campo, sem mutação nem
// chamada de função: `entity.FindComponentByType(n"entVisualControllerComponent")` dá o componente como `ref<IComponent>`, que vai
// direto pras natives. Layout conferido no construtor do Mac e ao vivo: `dependencies@10`, `looseDependencies@20`, `job@70`,
// `status@78`, `lock@7C` (0x80 no total).
//
// `entVisualControllerComponent` não aparece no corpus vanilla (a categoria do `CMesh`, que derruba o bind) e por isso não é
// declarada: só o NOME (CName) é usado na busca. Componente que não é `VisualControllerComponent` lê dado sem sentido no `+0xD8` —
// sempre atrás de `memoria::is_readable` no Rust, então nunca crasha.
native func BwmsVisualControllerHasController(component: ref<IComponent>) -> Bool
native func BwmsVisualControllerStatus(component: ref<IComponent>) -> Int32
native func BwmsVisualControllerDependencyCount(component: ref<IComponent>) -> Int32
native func BwmsVisualControllerLooseDependencyCount(component: ref<IComponent>) -> Int32
