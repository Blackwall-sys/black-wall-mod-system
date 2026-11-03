// ponte-mensagem-generica.reds — o popup de mensagem do jogo (`GenericMessageNotification`) com o que mods do Windows pedem a mais:
// `Show` com `inkTextParams` (variáveis na mensagem, Equipment-EX) e `ShowInput` (o mesmo popup com um campo de texto; o texto volta em
// `GenericMessageNotificationCloseData.input`). O campo é o `HubTextInput` da ponte, pendurado logo abaixo da mensagem.

import Codeware.UI.*

@addField(GenericMessageNotificationData)
public let params: ref<inkTextParams>;

@addField(GenericMessageNotificationData)
public let isInput: Bool;

@addField(GenericMessageNotificationCloseData)
public let input: String;

@addField(GenericMessageNotification)
private let m_bwmsCampo: ref<HubTextInput>;

@addMethod(GenericMessageNotification)
public final static func Show(controller: ref<worlduiIGameController>, title: String, message: String, params: ref<inkTextParams>, type: GenericMessageNotificationType) -> ref<inkGameNotificationToken> {
    let dados: ref<GenericMessageNotificationData> = GenericMessageNotification.BwmsDados(title, message, type);
    dados.params = params;
    return controller.ShowGameNotification(dados);
}

@addMethod(GenericMessageNotification)
public final static func ShowInput(controller: ref<worlduiIGameController>, title: String, message: String, type: GenericMessageNotificationType) -> ref<inkGameNotificationToken> {
    let dados: ref<GenericMessageNotificationData> = GenericMessageNotification.BwmsDados(title, message, type);
    dados.isInput = true;
    return controller.ShowGameNotification(dados);
}

// O dado-base do jogo (fila modal, cursor, widget do popup) com título, mensagem e tipo.
@addMethod(GenericMessageNotification)
private final static func BwmsDados(title: String, message: String, type: GenericMessageNotificationType) -> ref<GenericMessageNotificationData> {
    let dados: ref<GenericMessageNotificationData> = GenericMessageNotification.GetBaseData();
    dados.title = title;
    dados.message = message;
    dados.type = type;
    return dados;
}

@wrapMethod(GenericMessageNotification)
protected cb func OnInitialize() -> Bool {
    let resultado: Bool = wrappedMethod();
    if IsDefined(this.m_data.params) {
        inkTextRef.SetTextParameters(this.m_message, this.m_data.params);
    }
    if this.m_data.isInput {
        let mensagem: ref<inkWidget> = inkWidgetRef.Get(this.m_message);
        let pai: ref<inkCompoundWidget> = mensagem.GetParentWidget() as inkCompoundWidget;
        if IsDefined(pai) {
            let campo: ref<HubTextInput> = HubTextInput.Create();
            campo.Reparent(pai);
            campo.BwmsFocus(this);
            this.m_bwmsCampo = campo;
        }
    }
    return resultado;
}

// O texto digitado entra no resultado antes do aviso ao mod, que o jogo dá dentro do próprio `Close`; o resto é o `Close` do jogo.
@replaceMethod(GenericMessageNotification)
private final func Close(result: GenericMessageNotificationResult) -> Void {
    this.m_closeData = new GenericMessageNotificationCloseData();
    this.m_closeData.identifier = this.m_data.identifier;
    this.m_closeData.result = result;
    let campo: ref<HubTextInput> = this.m_bwmsCampo;
    if IsDefined(campo) {
        this.m_closeData.input = campo.GetText();
    }
    this.PlaySound(n"Button", n"OnPress");
    this.PlayLibraryAnimation(n"outro");
    this.m_data.token.TriggerCallback(this.m_closeData);
}
