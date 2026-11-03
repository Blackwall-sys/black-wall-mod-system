// Codeware #91 — campos nativos de widget, lidos de verdade.
//
// A ficha refutou a via do `@addField`: daria "storage próprio mas DESCONECTADO da lógica real de
// render — não seria capacidade genuína, só decoração inerte". A refutação está certa **para o
// `@addField`**. Ela não fala do caminho usado aqui: ler o CAMPO REAL do objeto, pelo offset que o
// RTTI desta build reporta. Nada é declarado e nada é storage paralelo.
//
// O TESTE é o ponto: em vez de comparar contra um valor que eu mesmo escrevi na memória, o
// **motor** escreve pelo setter nativo dele (`SetLetterCase`, `SetFontSize`, …) e a native lê o
// campo. Se os dois batem, a leitura está no lugar certo E conectada à lógica real — que é
// exatamente a objeção que a ficha levantava.

native func BwmsPropOffset(obj: ref<IScriptable>, prop: String) -> Int32;
native func BwmsPropU64(obj: ref<IScriptable>, prop: String, largura: Int32) -> Uint64;
// ESCRITA (2026-09-07) — o lado que faltava. O item pede `Get/Set`; até aqui só o `Get` existia,
// e metade da superfície ficava de fora. A largura é obrigatória pelo mesmo motivo da leitura,
// só que pior: ler um enum de 1 byte com 8 devolve um número esquisito e você percebe; ESCREVER
// 8 num campo de 1 destrói os vizinhos em silêncio e o efeito aparece longe da causa.
native func BwmsPropSetU64(obj: ref<IScriptable>, prop: String, largura: Int32, valor: Uint64) -> Bool;
