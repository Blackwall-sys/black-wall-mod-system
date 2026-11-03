// bwms-long-string.reds — texto em pedaços: `BwmsLongStr(fonte, arg, i)` devolve o pedaço `i` e `BwmsLongStrJoin` junta.
//
// O retorno de String de native não tem limite de tamanho (sai pelo construtor de texto do motor); este fatiador é o caminho de
// `BwmsDepotClasseLonga` e de quem já o usa. O corte é por CARACTERE (nunca por byte, que partiria UTF-8 no meio de um codepoint), e
// UMA native atende todas as fontes.

native func BwmsLongStr(fonte: String, arg: String, i: Int32) -> String;

// Junta os pedaços. `fonte` é validada por um `match` FIXO no Rust — o redscript escolhe entre
// nomes conhecidos, nunca um caminho de código arbitrário.
public func BwmsLongStrJoin(fonte: String, arg: String) -> String {
  let saida: String = "";
  let i: Int32 = 0;
  let acabou: Bool = false;
  // Sem `break`: este redscript não tem a palavra-chave (zero ocorrências no corpus vanilla
  // decompilado). Teto de 32 pedaços = 512 caracteres, folgado pra qualquer nome de classe ou path de recurso.
  while i < 32 && !acabou {
    let p: String = BwmsLongStr(fonte, arg, i);
    if Equals(p, "") {
      acabou = true;
    } else {
      saida += p;
      i += 1;
    };
  };
  return saida;
}

// Classe RTTI do recurso carregado no cache do depot.
public func BwmsDepotClasseLonga(path: String) -> String {
  return BwmsLongStrJoin("sectorclass", path);
}
