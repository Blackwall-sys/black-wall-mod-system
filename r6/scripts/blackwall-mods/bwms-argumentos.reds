// Argumentos de lançamento do jogo: BwmsGetLaunchArg("nome") devolve o valor (vazio se não veio ou veio sem valor); BwmsHasLaunchArg("nome")
// diz se veio. Aceita -nome valor, --nome valor e -nome=valor.
native func BwmsGetLaunchArg(name: String) -> String;
native func BwmsHasLaunchArg(name: String) -> Bool;
