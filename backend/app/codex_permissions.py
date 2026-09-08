"""Parsing puro do picker do `/permissions` do Codex — o modo de permissao de uma sessao viva.

E o equivalente Codex do BTab do Claude, e por que ele existe esta medido em 08/09/2026 contra o
codex-cli 0.153.4. Os outros dois caminhos possiveis nao servem:

- `turn/start` aceita `approvalPolicy` e ele APLICA, mas `sandbox` no mesmo lugar e ignorado em
  silencio (pedi `workspace-write`, o `turn_context` gravou `read-only`).
- `thread/settings/update` do app-server exige a capability `experimentalApi` no `initialize`, so
  aceita a forma `{threadId, settings:{...}}` (campo solto volta OK e nao faz nada) e tambem nao
  move o sandbox. O `permissionProfile` que cobriria isso foi descontinuado ("`permissionProfile`
  is no longer supported").

E nenhuma resposta serve de prova: o schema NAO recusa campo desconhecido, entao `OK` nao quer
dizer que aplicou, e a notificacao `thread/settings/updated` nao veio de forma confiavel nem
quando a troca funcionou. O picker e o unico caminho provado que troca OS DOIS eixos ao vivo, sem
recriar o pane.

Layout real capturado (`tests/fixtures/pane_codex_permissions.txt`, Full Access ativo):

      Update Model Permissions

      1. Ask for approval       Codex can read and edit files in the current workspace, e run
                                commands. Approval is required to access the internet ...
      2. Approve for me         Only ask for actions detected as potentially unsafe.
    > 3. Full Access (current)  Codex can edit files outside this workspace and access ...
                                without asking for approval. Exercise caution when using.

      Press enter to confirm or esc to go back

Achados das medicoes ao vivo (tmux):
- `›` = cursor; `(current)` = modo ativo. O picker abre com o cursor sobre o atual.
- A descricao QUEBRA em ate 3 linhas continuadas, so com espaco na frente. Contar linha de tela
  pra navegar erra por isso — quem conta e a lista de linhas numeradas.
- Escolher "Full Access" dispara um SEGUNDO dialogo ("Enable full access?", 1. Yes, continue
  anyway / 2. Cancel), que os outros dois nao disparam.
- O cabecalho de largada do Codex segue dizendo `permissions: YOLO mode` DEPOIS da troca: e o
  banner, nao o estado. Quem responde "qual e o modo agora" e o `(current)` daqui.

Tudo aqui e puro (sem IO) pra ser testavel com fixtures de pane capturados de verdade.
"""

import re

_TITULO = "Update Model Permissions"
_RODAPE = "Press enter to confirm or esc to go back"
_TITULO_CONFIRMA = "Enable full access?"

# Linha de modo: espacos, cursor opcional, "N.", rotulo ate 2+ espacos, e o resto = descricao.
# A quebra da descricao nao casa (nao tem "N."), que e o que faz o parse ignorar as continuacoes.
_LINHA_RE = re.compile(r"^\s*([›❯]?)\s*(\d+)\.\s+(.+?)\s{2,}(.*)$")


def _regiao(pane: str, titulo: str = _TITULO) -> list[str]:
    """So o bloco do picker (do titulo ate o rodape).

    O picker e um overlay e nao vai pro scrollback, entao com ele fechado o recorte fica vazio —
    e isso e o que impede uma lista numerada do proprio chat de passar por picker.
    """
    linhas = pane.splitlines()
    inicio = None
    for i, ln in enumerate(linhas):
        if titulo in ln:
            inicio = i           # a ultima ocorrencia: o overlay e unico
    if inicio is None:
        return []
    fim = len(linhas)
    for j in range(inicio, len(linhas)):
        if _RODAPE in linhas[j]:
            fim = j + 1
            break
    return linhas[inicio:fim]


def picker_aberto(pane: str) -> bool:
    return bool(_regiao(pane))


def picker_desenhado(pane: str) -> bool:
    """True so com o picker INTEIRO na tela.

    Mesmo motivo do `/model` do Claude: no instante em que o titulo aparece as linhas ainda estao
    sendo pintadas, e ler ali devolve a lista pela metade. O rodape vem depois delas.
    """
    return any(_RODAPE in ln for ln in _regiao(pane))


def confirmacao_de_full_access(pane: str) -> bool:
    """True quando o segundo dialogo, o de "Enable full access?", esta na tela."""
    return bool(_regiao(pane, _TITULO_CONFIRMA))


def parse_modos(pane: str) -> list[dict]:
    """Modos do picker: numero, rotulo, descricao, cursor e qual esta ativo.

    `nome` e o rotulo sem o `(current)` — e o que a tela mostra e o que o app exibe na pilula.
    """
    modos: list[dict] = []
    for ln in _regiao(pane):
        m = _LINHA_RE.match(ln)
        if not m:
            continue
        cursor, numero, rotulo, desc = m.group(1), int(m.group(2)), m.group(3).strip(), m.group(4)
        modos.append({
            "numero": numero,
            "nome": rotulo.replace("(current)", "").strip(),
            "desc": desc.strip(),
            "cursor": bool(cursor),
            "atual": "(current)" in rotulo,
        })
    return modos


def modo_atual(pane: str) -> str | None:
    for modo in parse_modos(pane):
        if modo["atual"]:
            return modo["nome"]
    return None


def passos_ate(pane: str, alvo: str) -> tuple[str, int] | None:
    """('Up'|'Down', quantas vezes) pra levar o cursor ate `alvo`. None se o alvo nao esta na lista.

    Conta POSICAO na lista, nunca linha de tela: a descricao de um modo ocupa de uma a tres linhas,
    entao a distancia em linhas nao e a distancia em toques de seta.
    """
    modos = parse_modos(pane)
    origem = next((i for i, m in enumerate(modos) if m["cursor"]), None)
    destino = next((i for i, m in enumerate(modos) if m["nome"].lower() == alvo.strip().lower()),
                   None)
    if origem is None or destino is None:
        return None
    delta = destino - origem
    return ("Down" if delta > 0 else "Up", abs(delta))
