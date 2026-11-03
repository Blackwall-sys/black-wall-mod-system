// bwms-textedit.reds — camada redscript do modelo de edição de texto.
//
// O buffer, o cursor e a seleção vivem em `bwms_primitives::editor_texto` (Rust, com testes offline); aqui só há chamada. O motivo é
// técnico: o modelo indexa em codepoint (`char`) e o redscript só oferece `String` (bytes). Manter o buffer do lado que sabe contar
// codepoints torna impossível fatiar um codepoint ao meio.
//
// A medição de texto fica em `bwms-text-measurer.reds`: `GetDesiredSize()` só sai de (0,0) com família de fonte, e a 1ª medida
// espera a fonte carregar (boot 151).

// Set, Insert, Move, Erase e Select devolvem o que mudou em bits (`BwmsTextEditMudou`); 0 = nada mudou, id desconhecido ou
// movimento inválido. O Rust escreve um Int32 no retorno: declarar Bool aqui faz a escrita passar do slot.
native func BwmsTextEditOpen() -> Int32;
native func BwmsTextEditSet(id: Int32, texto: String) -> Int32;
native func BwmsTextEditInsert(id: Int32, texto: String) -> Int32;
native func BwmsTextEditMove(id: Int32, movimento: Int32, estender: Bool) -> Int32;
native func BwmsTextEditErase(id: Int32, depois: Bool) -> Int32;
native func BwmsTextEditSelect(id: Int32, tudo: Bool) -> Int32;
native func BwmsTextEditCut(id: Int32) -> String;
native func BwmsTextEditText(id: Int32, selecionado: Bool) -> String;
native func BwmsTextEditNum(id: Int32, qual: Int32) -> Int32;
native func BwmsTextEditDrop(id: Int32) -> Bool;
native func BwmsTextEditScroll(conteudo: Float, janela: Float, caret: Float, largCaret: Float, atual: Float) -> Float;

// Códigos de movimento. Espelham `bwms_primitives::editor_texto::Movimento` — um valor fora
// da faixa é no-op do lado Rust, então errar aqui não corrompe estado.
public class BwmsTextEditMov {
    public static func Esquerda() -> Int32 { return 0; }
    public static func Direita() -> Int32 { return 1; }
    public static func PalavraEsquerda() -> Int32 { return 2; }
    public static func PalavraDireita() -> Int32 { return 3; }
    public static func Inicio() -> Int32 { return 4; }
    public static func Fim() -> Int32 { return 5; }
}

// Bits do retorno dos mutadores. Espelham `bwms_primitives::editor_texto::MUDOU_TEXTO`/`MUDOU_CARET`: com Texto o campo remede e avisa
// quem ouve; só com Caret refaz caret e seleção. Dois bits porque colar sobre a seleção o mesmo texto muda o caret e não o texto.
public class BwmsTextEditMudou {
    public static func Texto() -> Int32 { return 1; }
    public static func Caret() -> Int32 { return 2; }
}

// Handle de um editor. O id vem do `BwmsTextEditOpen` (contador no Rust, nunca 0 nem repetido
// entre editores abertos), então dois campos não dividem estado. Depois do `Fechar` o id é
// desconhecido: escrita vira no-op e leitura devolve "" ou 0, sem recriar o editor.
public class BwmsTextEdit {
    private let m_id: Int32;

    public static func Abrir() -> ref<BwmsTextEdit> {
        let e: ref<BwmsTextEdit> = new BwmsTextEdit();
        e.m_id = BwmsTextEditOpen();
        return e;
    }

    public func Fechar() -> Bool { return BwmsTextEditDrop(this.m_id); }

    public func DefinirTexto(t: String) -> Int32 { return BwmsTextEditSet(this.m_id, t); }
    public func Inserir(t: String) -> Int32 { return BwmsTextEditInsert(this.m_id, t); }
    public func Backspace() -> Int32 { return BwmsTextEditErase(this.m_id, false); }
    public func Delete() -> Int32 { return BwmsTextEditErase(this.m_id, true); }
    public func SelecionarTudo() -> Int32 { return BwmsTextEditSelect(this.m_id, true); }
    public func LimparSelecao() -> Int32 { return BwmsTextEditSelect(this.m_id, false); }
    public func Recortar() -> String { return BwmsTextEditCut(this.m_id); }

    public func Mover(mov: Int32, estender: Bool) -> Int32 {
        return BwmsTextEditMove(this.m_id, mov, estender);
    }

    public func Texto() -> String { return BwmsTextEditText(this.m_id, false); }
    public func TextoSelecionado() -> String { return BwmsTextEditText(this.m_id, true); }

    public func Cursor() -> Int32 { return BwmsTextEditNum(this.m_id, 0); }
    public func Comprimento() -> Int32 { return BwmsTextEditNum(this.m_id, 1); }
    public func SelecaoInicio() -> Int32 { return BwmsTextEditNum(this.m_id, 2); }
    public func SelecaoFim() -> Int32 { return BwmsTextEditNum(this.m_id, 3); }
    public func TemSelecao() -> Bool { return this.SelecaoFim() > this.SelecaoInicio(); }
}
