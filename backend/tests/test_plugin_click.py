import asyncio

import pytest

from app import plugin_bridge as pb
from app import plugin_click as pc

TELA = [
    "● resposta que fala em fechar",
    "",
    "▸ Review !577 Promedico · mergeado" + " " * 53 + "│ [ copiar link ]   fechar",
    "─" * 120,
    "❯ ",
    "─" * 120,
]
FAIXA = {"type": "Box", "children": [{"type": "Button", "props": {"key": "rv-1", "label": "▸ Review !577"}}]}
PAINEL = {"type": "Box", "children": [
    {"type": "Button", "props": {"key": "cp-1", "label": "[ copiar link ]"}},
    {"type": "Button", "props": {"key": "x-1", "label": "fechar"}}]}


async def _sem_efeito(*a):
    return None, None


@pytest.fixture
def sessao(monkeypatch):
    pb._guardar_faixa("clk", FAIXA, 87, [{"id": "review-mr", "title": "Review", "placement": "dock", "columns": 30, "tree": PAINEL}])
    cliques = []
    monkeypatch.setattr(pc, "terminal_refusal", lambda name: None)
    monkeypatch.setattr(pc, "screen", lambda name: list(TELA))
    monkeypatch.setattr(pc, "click", lambda name, row, col: cliques.append((row, col)) or True)
    yield cliques
    pb.esquecer("clk")


def test_rotulo_do_botao_vem_da_arvore():
    assert pc.button_label(PAINEL, "cp-1") == "[ copiar link ]"
    assert pc.button_label(PAINEL, "nao-existe") is None


@pytest.mark.asyncio
async def test_clique_na_faixa_confirmado(sessao, monkeypatch):
    async def confirma(name, site, key, desde, timeout):
        return (site, key) == ("above-prompt", "rv-1")
    monkeypatch.setattr(pb, "esperar_press", confirma)
    monkeypatch.setattr(pb, "esperar_efeito", _sem_efeito)
    assert await pc.press("clk", "above-prompt", "rv-1") == {"ok": True}
    # a linha da faixa, dentro da coluna da conversa
    assert sessao[0][0] == 2 and sessao[0][1] < 87


@pytest.mark.asyncio
async def test_rotulo_igual_na_conversa_nao_conta_para_o_painel(sessao, monkeypatch):
    # "fechar" aparece na conversa (coluna < 87) e no painel: a região do painel ancorado só olha a direita.
    async def confirma(*a):
        return True
    monkeypatch.setattr(pb, "esperar_press", confirma)
    monkeypatch.setattr(pb, "esperar_efeito", _sem_efeito)
    await pc.press("clk", "review-mr", "x-1")
    (linha, col), = sessao
    assert linha == 2 and col >= 87


@pytest.mark.asyncio
async def test_copia_volta_ao_app(sessao, monkeypatch):
    async def confirma(*a):
        return True
    async def copia(*a):
        return "https://gitlab.exemplo/mr/577", None
    monkeypatch.setattr(pb, "esperar_press", confirma)
    monkeypatch.setattr(pb, "esperar_efeito", copia)
    assert await pc.press("clk", "review-mr", "cp-1") == {"ok": True, "copied": "https://gitlab.exemplo/mr/577"}


@pytest.mark.asyncio
async def test_sem_mouse_recusa_sem_clicar(sessao, monkeypatch):
    monkeypatch.setattr(pc, "terminal_refusal", lambda name: "erro_mod_mouse_desligado")
    with pytest.raises(pc.PressRefused) as e:
        await pc.press("clk", "above-prompt", "rv-1")
    assert e.value.detail["code"] == "erro_mod_mouse_desligado"
    assert sessao == []


@pytest.mark.asyncio
async def test_botao_inexistente_e_rotulo_ausente(sessao, monkeypatch):
    with pytest.raises(pc.PressRefused) as e:
        await pc.press("clk", "above-prompt", "nada")
    assert e.value.detail["code"] == "erro_mod_botao_inexistente"
    monkeypatch.setattr(pc, "screen", lambda name: ["─" * 40, "❯ "])
    with pytest.raises(pc.PressRefused) as e:
        await pc.press("clk", "above-prompt", "rv-1")
    assert e.value.detail["code"] == "erro_mod_botao_nao_achado"


@pytest.mark.asyncio
async def test_rotulo_duas_vezes_na_regiao_e_ambiguo(sessao, monkeypatch):
    dupla = list(TELA)
    dupla[1] = "▸ Review !577"
    monkeypatch.setattr(pc, "screen", lambda name: dupla)
    monkeypatch.setattr(pb, "band_anchor", lambda name: None)
    with pytest.raises(pc.PressRefused) as e:
        await pc.press("clk", "above-prompt", "rv-1")
    assert e.value.detail["code"] == "erro_mod_botao_ambiguo"
    assert sessao == []


@pytest.mark.asyncio
async def test_sem_confirmacao_e_erro(sessao, monkeypatch):
    async def nunca(*a):
        return False
    monkeypatch.setattr(pb, "esperar_press", nunca)
    with pytest.raises(pc.PressRefused) as e:
        await pc.press("clk", "above-prompt", "rv-1")
    assert e.value.detail["code"] == "erro_mod_clique_sem_resposta"


@pytest.mark.asyncio
async def test_dois_cliques_da_mesma_sessao_nao_se_cruzam(sessao, monkeypatch):
    ordem = []

    async def confirma(name, site, key, desde, timeout):
        ordem.append(("inicio", key))
        await asyncio.sleep(0.05)
        ordem.append(("fim", key))
        return True
    monkeypatch.setattr(pb, "esperar_press", confirma)
    monkeypatch.setattr(pb, "esperar_efeito", _sem_efeito)
    await asyncio.gather(pc.press("clk", "review-mr", "cp-1"), pc.press("clk", "review-mr", "x-1"))
    assert [o[0] for o in ordem] == ["inicio", "fim", "inicio", "fim"]


@pytest.mark.asyncio
async def test_abertura_volta_ao_app(sessao, monkeypatch):
    async def confirma(*a):
        return True
    async def abre(*a):
        return None, "https://gitlab.exemplo/mr/577"
    monkeypatch.setattr(pb, "esperar_press", confirma)
    monkeypatch.setattr(pb, "esperar_efeito", abre)
    assert await pc.press("clk", "above-prompt", "rv-1") == {"ok": True, "opened": "https://gitlab.exemplo/mr/577"}


@pytest.mark.asyncio
async def test_clique_avisa_a_ponte_que_e_do_app(sessao, monkeypatch):
    marcados = []
    monkeypatch.setattr(pb, "esperar_clique_do_app", lambda *a: marcados.append(a[1:3]) or "t1")
    async def confirma(*a):
        return True
    monkeypatch.setattr(pb, "esperar_press", confirma)
    monkeypatch.setattr(pb, "esperar_efeito", _sem_efeito)
    await pc.press("clk", "above-prompt", "rv-1")
    assert marcados == [("above-prompt", "rv-1")]


def test_recusa_do_terminal_por_motivo(monkeypatch):
    # Copy-mode: o ESC do clique cancelaria o modo e o resto da sequência cairia como texto no prompt.
    respostas = {"#{mouse_sgr_flag} #{pane_in_mode}": "1 1"}
    monkeypatch.setattr(pc.os, "name", "posix")
    monkeypatch.setattr(pc, "_tmux_format", lambda name, fmt: respostas[fmt])
    assert pc.terminal_refusal("x") == "erro_mod_terminal_em_modo"
    respostas["#{mouse_sgr_flag} #{pane_in_mode}"] = "0 0"
    assert pc.terminal_refusal("x") == "erro_mod_mouse_desligado"
    respostas["#{mouse_sgr_flag} #{pane_in_mode}"] = "1 0"
    assert pc.terminal_refusal("x") is None


def test_windows_recusa_ate_o_psmux_ser_provado(monkeypatch):
    monkeypatch.setattr(pc.os, "name", "nt")
    assert pc.terminal_refusal("x") == "erro_mod_clique_windows"


@pytest.mark.asyncio
async def test_painel_inline_sem_ancora_acha_o_botao_logo_acima_do_prompt(sessao, monkeypatch):
    # Sem faixa (mod que só abre painel) não há âncora: o painel inline fica logo acima do prompt.
    tela = ["● resposta", "", "╭ Review ─╮", "│ [ copiar link ] │", "╰──────────╯", "─" * 40, "❯ ", "─" * 40]
    pb._guardar_faixa("clk", None, 40, [{"id": "rv", "title": "Review", "placement": "inline", "columns": 30, "tree": PAINEL}])
    monkeypatch.setattr(pc, "screen", lambda name: tela)
    async def confirma(*a):
        return True
    monkeypatch.setattr(pb, "esperar_press", confirma)
    monkeypatch.setattr(pb, "esperar_efeito", _sem_efeito)
    await pc.press("clk", "rv", "cp-1")
    assert sessao[0][0] == 3
