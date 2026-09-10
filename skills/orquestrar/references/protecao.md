# Proteção do código durante pesquisa e revisão

Leia ao planejar ou abrir uma sessão de pesquisa, revisão, revisão final ou verificação.
O objetivo é impedir edições acidentais no checkout do executor e nos seus metadados Git,
mantendo relatórios, runtime do agente e ambientes descartáveis graváveis fora dele.

## Abertura e prova

Acrescente `--read-only` ao comando de abertura definido no contrato:

```bash
hangar-send --new <nome> <repo> --provider <provider> --model <modelo> --effort <esforco> --read-only
```

Use também os flags de conta/motor autorizados. A opção protege o repositório do `cwd`,
inclusive quando ele é uma subpasta ou worktree; relatórios devem ficar fora dessa árvore.
No Linux, o backend usa `bwrap` no comando **do pane**, envolvendo também o app-server do Codex.
Envolver só o `hangar-send` ou só a TUI não protegeria quem executa as ferramentas.

A abertura testa a capacidade antes de criar a sessão. Sem suporte, falha com o motivo;
não repita removendo o flag. No planejamento, resolva a falta de suporte antes de liberar a
revisão. Uma restrição nativa de outro harness só substitui esta com a mesma prova de escrita.
Depois de atualizar o Hangar, o backend precisa ser reiniciado para reconhecer a opção;
um backend anterior com corpo estrito recusa o campo com HTTP 422.

Antes de ler o diff ou executar o roteiro, confira o comando real do pane e faça uma prova
dentro da sessão: abrir um arquivo de código existente em modo de escrita deve ser recusado,
enquanto um arquivo temporário na pasta de artefatos deve aceitar escrita. Abrir sem truncar
nem escrever é suficiente para detectar a ausência da proteção sem alterar o código:

```python
import errno
import os
import tempfile

try:
    fd = os.open("<arquivo de código existente>", os.O_WRONLY)
except OSError as error:
    assert error.errno in (errno.EROFS, errno.EACCES, errno.EPERM), error
else:
    os.close(fd)
    raise RuntimeError("O código ainda aceita escrita")
with tempfile.TemporaryFile(dir="<pasta de artefatos>") as artifact:
    artifact.write(b"ok")
```

Registre a prova no primeiro parecer; numa nova sessão, repita. Uma sessão antiga não ganha
proteção só por receber instruções novas. Pesquisa por subagente exige proteção herdada ou
restrição nativa conferida, não apenas uma descrição de papel dizendo que ele é somente leitura.

## Testes que escrevem

Build, cache e mutação rodam em uma cópia descartável fora da árvore protegida. Use o objeto
congelado da rodada (ou a ponta aprovada na revisão final), sem escrever no Git do executor:

```bash
git clone --no-hardlinks --no-checkout -- <repo> <sandbox-novo>
git -C <sandbox-novo> checkout --detach <objeto>
```

Confira o `HEAD` da cópia antes de testar e use a preparação de ambiente definida no plano.
A cópia tem objetos Git próprios: `git worktree add` escreveria nos metadados protegidos.
Só a cópia pode receber a mutação; correções de produto e de testes continuam com o executor.
Copie os resultados para a pasta durável e remova apenas o sandbox criado por esta rodada.

## Limites

Esta opção é para Linux com `bwrap`; não declara proteção equivalente no Windows. Ela bloqueia
escrita pelos caminhos protegidos, não isola o agente de toda a máquina: serviços externos,
API do Hangar, tmux e caminhos alternativos por `/proc` podem atuar fora dessa montagem.
Por isso o roteiro também proíbe pedir a outro processo que altere o checkout. Reabrir uma
sessão deve preservar a opção e repetir a prova; proteção não é inferida do nome da sessão.
A retomada viva que recriaria o pane é recusada para não perder a proteção. Recrie pela abertura
com `--read-only`; uma conversa retomada do Arquivo também precisa da opção.
