// bwms-object-serializer.reds — gravar e ler objetos do motor em arquivo (Codeware `#252`, `ObjectSerializer::SaveToFile`/`ReadFromFile`).
// Os arquivos ficam em `r6/storage/bwms/objetos/`, e o caminho é relativo a essa pasta: partes de letras, dígitos, `_`, `-` e `.` separadas por `/`, sem `..`.
// O arquivo guarda o nome da classe, então a leitura não pede o tipo. Guarde o retorno numa variável antes de comparar.

// Grava `obj` em `r6/storage/bwms/objetos/<path>` pelo serializador de pacotes do motor. `false` com caminho inválido, objeto nulo ou erro de disco.
public native func BwmsObjectSave(obj: ref<IScriptable>, path: String) -> Bool

// Lê um objeto gravado pelo `BwmsObjectSave` numa instância nova da mesma classe; nulo se o arquivo não existe ou não é de objeto.
public native func BwmsObjectLoad(path: String) -> ref<IScriptable>
