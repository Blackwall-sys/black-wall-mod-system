// bwms-inksystem-clipboard-longo.reds — o texto da área de transferência em pedaços.
//
// `BwmsInkSystemGetClipboardTextFull()` junta o texto em pedaços cortados em fronteira de caractere. `BwmsGetClipboardText` já devolve o
// texto inteiro (o retorno de String de native não tem limite de tamanho); esta forma fica pra quem já a usa.
native func BwmsClipboardTextPart(part: Int32) -> String

public func BwmsInkSystemGetClipboardTextFull() -> String {
    let texto: String = "";
    let parte: Int32 = 0;
    while parte < 4096 {
        let pedaco: String = BwmsClipboardTextPart(parte);
        if StrLen(pedaco) == 0 {
            return texto;
        };
        texto = texto + pedaco;
        parte += 1;
    };
    return texto;
}
