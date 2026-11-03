// bwms-renderproxy.reds — `RenderProxy`.
//
// O proxy de render de um componente (o objeto que o renderer desenha), pelo RE offline (DATABASE, seção `RenderProxy`), com nomes do
// BWMS (framework de terceiros). Sem proxy vivo — o corpo do V em primeira pessoa, por exemplo — as funções devolvem `false`/`-1`.
native func BwmsRenderProxyFlags(component: ref<IComponent>) -> Int32

// `IsVisible`: os dois bits da máscara 6 do byte de flags ligados.
public func BwmsRenderProxyIsVisible(component: ref<IComponent>) -> Bool {
    let flags: Int32 = BwmsRenderProxyFlags(component);
    if flags < 0 {
        return false;
    };
    return (flags & 6) == 6;
}
