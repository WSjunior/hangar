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
FAIXA = {"type": "Box", "children": [{"type": "Button", "props": {"key": "rv-1", "label": "▸ Review !577"}, "press": {"plugin": "review", "handle": 1}}]}
PAINEL = {"type": "Box", "children": [
    {"type": "Button", "props": {"key": "cp-1", "label": "[ copiar link ]"}, "press": {"plugin": "review", "handle": 1}},
    {"type": "Button", "props": {"key": "x-1", "label": "fechar"}, "press": {"plugin": "review", "handle": 1}}]}


async def _sem_efeito(*a):
    return None, None


async def _confirma(*a):
    return True


@pytest.fixture
def sessao(monkeypatch):
    pb._guardar_faixa("clk", FAIXA, 87, [{"id": "review-mr", "title": "Review", "placement": "dock", "columns": 30, "tree": PAINEL}])
    cliques = []
    monkeypatch.setattr(pc, "terminal_refusal", lambda name: None)
    monkeypatch.setattr(pc, "screen", lambda name: list(TELA))
    monkeypatch.setattr(pc, "click", lambda name, row, col: cliques.append((row, col)) or True)
    # Padrão: o plugin confirma e o clique não copia nem abre; quem precisa de outro, sobrescreve.
    monkeypatch.setattr(pb, "esperar_press", _confirma)
    monkeypatch.setattr(pb, "esperar_efeito", _sem_efeito)
    yield cliques
    pb.esquecer("clk")


def test_rotulo_do_botao_vem_da_arvore():
    assert pc.button_label(PAINEL, "cp-1", "review") == "[ copiar link ]"
    assert pc.button_label(PAINEL, "nao-existe", "review") is None


def test_rotulo_e_o_do_botao_do_mod_pedido():
    # Dois mods com a mesma `key` no mesmo lugar: o rótulo é o do botão do mod que o app disse.
    dois = {"type": "Box", "children": [
        {"type": "Button", "props": {"key": "k", "label": "do um"}, "press": {"plugin": "um", "handle": 1}},
        {"type": "Button", "props": {"key": "k", "label": "do outro"}, "press": {"plugin": "outro", "handle": 2}}]}
    assert pc.button_label(dois, "k", "outro") == "do outro"
    assert pc.button_label(dois, "k", "um") == "do um"
    assert pc.button_label(dois, "k", "terceiro") is None
    # Sem o mod (app de antes desta versão): a `key` em dois mods não diz qual é; a de um só, sim.
    assert pc.button_label(dois, "k", None) is None
    assert pc.button_label(PAINEL, "cp-1", None) == "[ copiar link ]"


@pytest.mark.asyncio
async def test_clique_na_faixa_confirmado(sessao, monkeypatch):
    async def confirma(name, site, key, desde, timeout):
        return (site, key) == ("above-prompt", "rv-1")
    monkeypatch.setattr(pb, "esperar_press", confirma)
    assert await pc.press("clk", "above-prompt", "rv-1", "review") == {"ok": True}
    # a linha da faixa, dentro da coluna da conversa
    assert sessao[0][0] == 2 and sessao[0][1] < 87


@pytest.mark.asyncio
async def test_rotulo_igual_na_conversa_nao_conta_para_o_painel(sessao, monkeypatch):
    # "fechar" aparece na conversa (coluna < 87) e no painel: a região do painel ancorado só olha a direita.
    await pc.press("clk", "review-mr", "x-1", "review")
    (linha, col), = sessao
    assert linha == 2 and col >= 87


@pytest.mark.asyncio
async def test_fechar_clica_no_x_do_painel_e_espera_ele_sair(sessao, monkeypatch):
    tela = list(TELA)
    tela[2] = tela[2].replace("fechar", "✕")
    monkeypatch.setattr(pc, "screen", lambda name: list(tela))
    saiu = []

    async def sem_painel(name, site, timeout):
        saiu.append(site)
        return True
    monkeypatch.setattr(pb, "esperar_sem_painel", sem_painel)
    assert await pc.close("clk", "review-mr") == {"ok": True}
    (linha, col), = sessao
    assert linha == 2 and col >= 87 and saiu == ["review-mr"]


@pytest.mark.asyncio
async def test_faixa_nao_tem_x_para_fechar(sessao):
    with pytest.raises(pc.PressRefused) as e:
        await pc.close("clk", "above-prompt")
    assert e.value.detail["code"] == "erro_mod_painel_inexistente"
    assert sessao == []


@pytest.mark.asyncio
async def test_copia_volta_ao_app(sessao, monkeypatch):
    async def copia(*a):
        return "https://gitlab.exemplo/mr/577", None
    monkeypatch.setattr(pb, "esperar_efeito", copia)
    assert await pc.press("clk", "review-mr", "cp-1", "review") == {"ok": True, "copied": "https://gitlab.exemplo/mr/577"}


@pytest.mark.asyncio
async def test_sem_mouse_recusa_sem_clicar(sessao, monkeypatch):
    monkeypatch.setattr(pc, "terminal_refusal", lambda name: "erro_mod_mouse_desligado")
    with pytest.raises(pc.PressRefused) as e:
        await pc.press("clk", "above-prompt", "rv-1", "review")
    assert e.value.detail["code"] == "erro_mod_mouse_desligado"
    assert sessao == []


@pytest.mark.asyncio
async def test_botao_inexistente_e_rotulo_ausente(sessao, monkeypatch):
    with pytest.raises(pc.PressRefused) as e:
        await pc.press("clk", "above-prompt", "nada", "review")
    assert e.value.detail["code"] == "erro_mod_botao_inexistente"
    monkeypatch.setattr(pc, "screen", lambda name: ["─" * 40, "❯ "])
    with pytest.raises(pc.PressRefused) as e:
        await pc.press("clk", "above-prompt", "rv-1", "review")
    assert e.value.detail["code"] == "erro_mod_botao_nao_achado"


@pytest.mark.asyncio
async def test_rotulo_duas_vezes_na_regiao_e_ambiguo(sessao, monkeypatch):
    dupla = list(TELA)
    dupla[1] = "▸ Review !577"
    monkeypatch.setattr(pc, "screen", lambda name: dupla)
    monkeypatch.setattr(pb, "band_anchor", lambda name: None)
    with pytest.raises(pc.PressRefused) as e:
        await pc.press("clk", "above-prompt", "rv-1", "review")
    assert e.value.detail["code"] == "erro_mod_botao_ambiguo"
    assert sessao == []


@pytest.mark.asyncio
async def test_sem_confirmacao_e_erro(sessao, monkeypatch):
    async def nunca(*a):
        return False
    monkeypatch.setattr(pb, "esperar_press", nunca)
    with pytest.raises(pc.PressRefused) as e:
        await pc.press("clk", "above-prompt", "rv-1", "review")
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
    await asyncio.gather(pc.press("clk", "review-mr", "cp-1", "review"), pc.press("clk", "review-mr", "x-1", "review"))
    assert [o[0] for o in ordem] == ["inicio", "fim", "inicio", "fim"]


@pytest.mark.asyncio
async def test_abertura_volta_ao_app(sessao, monkeypatch):
    async def abre(*a):
        return None, "https://gitlab.exemplo/mr/577"
    monkeypatch.setattr(pb, "esperar_efeito", abre)
    assert await pc.press("clk", "above-prompt", "rv-1", "review") == {"ok": True, "opened": "https://gitlab.exemplo/mr/577"}


@pytest.mark.asyncio
async def test_clique_avisa_a_ponte_que_e_do_app(sessao, monkeypatch):
    marcados = []
    monkeypatch.setattr(pb, "esperar_clique_do_app", lambda *a: marcados.append(a[1:3]) or "t1")
    await pc.press("clk", "above-prompt", "rv-1", "review")
    assert marcados == [("above-prompt", "rv-1")]


@pytest.mark.asyncio
async def test_sem_posse_da_escrita_recusa_em_vez_de_500(sessao, monkeypatch):
    # Rust mudo no detach (TimeoutError) ou vínculo em dúvida (RuntimeError): o app recebe o motivo.
    def mudo(name, row, col):
        raise TimeoutError("silent Rust")
    monkeypatch.setattr(pc, "click", mudo)
    with pytest.raises(pc.PressRefused) as e:
        await pc.press("clk", "above-prompt", "rv-1", "review")
    assert e.value.detail["code"] == "erro_mod_clique_sem_resposta"


@pytest.mark.asyncio
async def test_recusa_sem_posse_deixa_a_causa_no_log(sessao, monkeypatch, caplog):
    # Sem posse, vínculo em dúvida e Rust mudo viram o mesmo 409: só o log separa um do outro.
    def sem_posse(name, row, col):
        raise RuntimeError("Python sem posse da escrita terminal")
    monkeypatch.setattr(pc, "click", sem_posse)
    with caplog.at_level("WARNING", logger="hangar.plugin_click"), pytest.raises(pc.PressRefused):
        await pc.press("clk", "above-prompt", "rv-1", "review")
    assert "sem posse da escrita" in caplog.text


def test_clique_sem_coordenador_vai_direto_ao_terminal(monkeypatch):
    # Sem o coordenador do runtime (a main e a reserva), o clique envolvido é o de sempre.
    from app import runtime_coordinator, tmux
    enviados = []
    monkeypatch.setattr(runtime_coordinator, "current", lambda: None)
    monkeypatch.setattr(tmux, "send_keys", lambda name, keys, literal=False: enviados.append((name, keys, literal)) or True)
    assert pc.click("clk", 2, 5)
    assert enviados == [("clk", "\x1b[<0;6;3M\x1b[<0;6;3m", True)]


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


def test_windows_le_o_mouse_pela_tela_alternativa(monkeypatch):
    # O psmux não tem as flags de mouse: a tela cheia do Claude Code (tela alternativa) é o sinal.
    respostas = {"#{alternate_on} #{pane_in_mode}": "1 0"}
    monkeypatch.setattr(pc.os, "name", "nt")
    monkeypatch.setattr(pc, "_tmux_format", lambda name, fmt: respostas[fmt])
    assert pc.terminal_refusal("x") is None
    respostas["#{alternate_on} #{pane_in_mode}"] = "0 0"
    assert pc.terminal_refusal("x") == "erro_mod_mouse_desligado"


@pytest.mark.asyncio
async def test_painel_inline_sem_ancora_acha_o_botao_logo_acima_do_prompt(sessao, monkeypatch):
    # Sem faixa (mod que só abre painel) não há âncora: o painel inline fica logo acima do prompt.
    tela = ["● resposta", "", "╭ Review ─╮", "│ [ copiar link ] │", "╰──────────╯", "─" * 40, "❯ ", "─" * 40]
    pb._guardar_faixa("clk", None, 40, [{"id": "rv", "title": "Review", "placement": "inline", "columns": 30, "tree": PAINEL}])
    monkeypatch.setattr(pc, "screen", lambda name: tela)
    await pc.press("clk", "rv", "cp-1", "review")
    assert sessao[0][0] == 3


def test_sessao_claude_nasce_em_tela_cheia():
    # É na tela cheia que o Claude Code liga o mouse; no Windows por SSH ele a desliga sozinho.
    from app import registry
    assert registry._env_sessao(None, False)["env"]["CLAUDE_CODE_NO_FLICKER"] == "1"
    assert "CLAUDE_CODE_NO_FLICKER" not in registry._env_sessao(None, False, provider="codex")["env"]
