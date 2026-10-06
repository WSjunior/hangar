"""Copia capturas do terminal (tmux e psmux) para dados de teste do repositório, que é público.

Uso, da raiz do repositório:
  python3 crates/hangar-server/tests/fixtures/mods_screen/limpar_capturas.py <pasta-das-medicoes>
  python3 crates/hangar-server/tests/fixtures/mods_screen/limpar_capturas.py --conferir

A pasta das medições tem `terminal/capturas/` e `terminal-psmux/capturas/` com pares `.txt`/`.ansi`.
Só o `.ansi` é copiado. Toda palavra fora da lista permitida (vocabulário do Claude Code, da API dos
mods e do mod de vitrine) vira `x` (letra), `0` (dígito) ou `_`, célula por célula: a geometria da tela
fica igual e nada pessoal nem de mod de empresa entra no repositório. A decisão é tomada sobre a
palavra inteira do texto sem estilo, e a troca é aplicada no `.ansi`, entre as sequências de cor."""
import json
import re
import sys
import unicodedata
from pathlib import Path

AQUI = Path(__file__).resolve().parent
ESC = re.compile(r"\x1b(\[[0-9;:?]*[@-~]|\][^\x07\x1b]*(\x07|\x1b\\)|[()][A-Za-z0-9])")
PALAVRA = re.compile(r"\w+")

PERMITIDAS = frozenset("""
Claude Code Opus Sonnet Fable Haiku Max Pro manual mode on auto accept edits for agents Try how do I
plugin panel hidden ctrl or click to show more Do you want proceed Yes No Esc cancel Tab amend How is
doing this session optional Bad Fine Good Dismiss Bash PowerShell command Permission rule requires
confirmation permissions update rules MR Jenkins x a main
vitrine Vitrine Texto texto Botões botões Botão botão Hover hover Campos campos Conteúdo conteúdo rico
Longo longo Abas abas Sozinho sozinho Inválida inválida Grande grande Mídia mídia Quebrado quebrado
Contar cliques desenhos Igual comeco começo meio fim do da de no na um uma linha linhas enchimento faixa
painel painéis trecho outro mod mods next fica acima abaixo superfície terminal viewport tela cheia sim
ligadas escopo Escopo Cartão cartão absolute revelado passe ponteiro nesta vizinho idem onde está quando
aparece muda cor borda Box key compartilhado acende junto com recebido pedido rascunho teste
""".split())


def permitida(palavra: str) -> bool:
    return (palavra in PERMITIDAS or re.fullmatch(r"V\d{2}[a-z]?", palavra) is not None
            or re.fullmatch(r"\d{1,3}", palavra) is not None or re.fullmatch(r"[x0_]+", palavra) is not None)


def _largura(ch: str) -> int:
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


def _troca(ch: str) -> str:
    if ch == "_":
        return "_"
    return ("0" if ch.isdigit() else "x") * _largura(ch)


def limpar(ansi: str) -> str:
    saida = list(ansi)
    inicio = 0
    for linha in ansi.split("\n"):
        texto: list[str] = []
        mapa: list[int] = []
        i = 0
        while i < len(linha):
            m = ESC.match(linha, i) if linha[i] == "\x1b" else None
            if m:
                if linha[i + 1] == "]":
                    # Hiperlink OSC 8: o endereço e o id podem trazer identificador de sessão. Zero
                    # largura na tela, mas a troca mantém o tamanho e a estrutura da sequência.
                    for k in range(i + 4, m.end()):
                        if linha[k].isalnum() and linha[k] != "\x1b":
                            saida[inicio + k] = _troca(linha[k])
                i = m.end()
                continue
            texto.append(linha[i])
            mapa.append(inicio + i)
            i += 1
        plano = "".join(texto)
        for m in PALAVRA.finditer(plano):
            if not permitida(m.group()):
                for k in range(m.start(), m.end()):
                    saida[mapa[k]] = _troca(plano[k])
        inicio += len(linha) + 1
    return "".join(saida)


def main(origem: Path) -> None:
    casos = json.loads((AQUI / "casos.json").read_text(encoding="utf-8"))
    for caso in casos:
        bruto = (origem / caso["origem"]).read_text(encoding="utf-8")
        (AQUI / f"{caso['nome']}.ansi").write_text(limpar(bruto), encoding="utf-8", newline="")
    print(f"{len(casos)} capturas limpas")


def conferir() -> int:
    """O que o teste Rust `mods_captures` pede: toda palavra permitida ou trocada, nos dados e nos casos,
    e a limpeza sem mudar a largura de uma linha. Devolve o código de saída."""
    erros = []
    bruto = "\x1b[1;7m AB-12345 \x1b[0m│ ab_cdefgh Contar 界x │"
    limpo = limpar(bruto)
    if limpo != "\x1b[1;7m xx-00000 \x1b[0m│ xx_xxxxxx Contar xxx │":
        erros.append(f"limpeza de exemplo: {limpo!r}")
    osc = limpar("\x1b]8;id=ab12;https://example.invalid/s1\x1b\\x\x1b]8;;\x1b\\")
    if "example" in osc or "ab12" in osc:
        erros.append(f"hiperlink OSC 8 sem limpar: {osc!r}")
    largura = lambda texto: sum(_largura(c) for c in ESC.sub("", texto))
    if largura(limpo) != largura(bruto):
        erros.append("a limpeza mudou a largura: a borda depois do caractere largo andou de coluna")
    for caso in json.loads((AQUI / "casos.json").read_text(encoding="utf-8")):
        cru = (AQUI / f"{caso['nome']}.ansi").read_text(encoding="utf-8")
        for m in ESC.finditer(cru):
            if m.group().startswith("\x1b]") and re.search(r"[A-Za-wyz1-9]", m.group()[4:].rstrip("\x07\x1b\\")):
                erros.append(f"{caso['nome']}: sequência OSC com texto: {m.group()!r}")
                break
        texto = ESC.sub("", cru)
        estranhas = sorted({p for p in PALAVRA.findall(texto) if not permitida(p)})
        if estranhas:
            erros.append(f"{caso['nome']}: {estranhas[:10]}")
        for texto in [*caso["titulos"], caso["ancora"] or ""]:
            if not all(permitida(p) for p in PALAVRA.findall(texto)):
                erros.append(f"{caso['nome']}: título ou âncora fora da lista: {texto!r}")
    for erro in erros:
        print(erro)
    return 1 if erros else 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--conferir"]:
        sys.exit(conferir())
    main(Path(sys.argv[1]))
