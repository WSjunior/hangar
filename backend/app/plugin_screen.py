"""A tela do pane em células: largura de caractere, corte de coluna e onde fica um rótulo.

O Claude Code desenha a faixa e os painéis dos mods no terminal. O Hangar acha ali o botão que o app
pediu para clicar, e a prévia corta a coluna do painel ancorado ao lado da conversa."""
import unicodedata

from app.state import _RULE_RE

# Linhas acima da caixa de digitar onde a faixa pode estar quando a âncora não aparece.
MAX_BAND_ROWS = 20


def cell_width(ch: str) -> int:
    """Células que o caractere ocupa no terminal."""
    if unicodedata.combining(ch) or ch in "​‍️":
        return 0
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


def cells(text: str) -> int:
    return sum(cell_width(c) for c in text)


def crop_cells(line: str, columns: int) -> str:
    """A linha até `columns` células, sem o espaço que sobra no fim."""
    usado = 0
    for i, ch in enumerate(line):
        usado += cell_width(ch)
        if usado > columns:
            return line[:i].rstrip()
    return line


def prompt_top(screen: list[str]) -> int | None:
    """A régua de cima da caixa de digitar: a última régua com a linha do ❯ logo abaixo."""
    for i in range(len(screen) - 2, -1, -1):
        if _RULE_RE.match(screen[i]) and screen[i + 1].lstrip().startswith("❯"):
            return i
    return None


def anchor_row(screen: list[str], top: int, anchor: str | None) -> int | None:
    """A linha da âncora (primeiro texto da faixa) mais perto da caixa de digitar, se aparece."""
    if anchor:
        for r in range(top - 1, max(0, top - MAX_BAND_ROWS) - 1, -1):
            if anchor in screen[r]:
                return r
    return None


def band_start(screen: list[str], top: int, anchor: str | None) -> int:
    """Primeira linha da faixa: a da âncora; sem ela, o teto de linhas acima da caixa de digitar."""
    linha = anchor_row(screen, top, anchor)
    return max(0, top - MAX_BAND_ROWS) if linha is None else linha


def find_label(screen: list[str], label: str, rows: range, lo: int, hi: int | None) -> list[tuple[int, int]]:
    """Cada ocorrência inteira de `label` dentro da região: (linha, célula do meio do rótulo)."""
    alvo = label.strip()
    if not alvo:
        return []
    largura = cells(alvo)
    achados = []
    for r in rows:
        if r < 0 or r >= len(screen):
            continue
        linha = screen[r]
        inicio = linha.find(alvo)
        while inicio >= 0:
            c0 = cells(linha[:inicio])
            if c0 >= lo and (hi is None or c0 + largura <= hi):
                achados.append((r, c0 + (largura - 1) // 2))
            inicio = linha.find(alvo, inicio + 1)
    return achados
