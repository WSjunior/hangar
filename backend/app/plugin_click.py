"""Clique de botão de mod pedido pelo app: acha o botão na tela do pane e clica pelo mouse do terminal.

Nenhuma API do engine deixa um plugin disparar o botão de outro (o `onPress` mora no ambiente do mod).
O clique entra como clique de mouse SGR no pane, na célula do rótulo, e o plugin do Hangar confirma
pelo `ui.press` que o press chegou ao botão certo."""
import asyncio
import os
import time

from app import plugin_bridge, tmux
from app.mensagens import erro
from app.plugin_screen import band_start, find_label, prompt_top
from app.state import run_tmux

BAND_SITE = "above-prompt"
CLOSE_KEY = "__close__"
CLOSE_LABEL = "✕"
CONFIRM_S = 2.0
COPY_S = 0.3
_locks: dict[str, asyncio.Lock] = {}


class PressRefused(Exception):
    def __init__(self, code: str, msg: str, **params):
        super().__init__(msg)
        self.detail = erro(code, msg, **params)


def button_label(tree, key: str) -> str | None:
    """O rótulo do `Button` de `key`: `label`, ou o texto dos filhos."""
    pilha = [tree]
    while pilha:
        no = pilha.pop()
        if not isinstance(no, dict):
            continue
        props = no.get("props") or {}
        if no.get("type") == "Button" and props.get("key") == key:
            texto = props.get("label") or "".join(c for c in no.get("children") or [] if isinstance(c, str))
            return texto.strip() or None
        pilha.extend(no.get("children") or [])
    return None


def mouse_ready(name: str) -> bool:
    """O Claude Code pediu mouse ao terminal? O psmux não tem a flag de mouse; a tela alternativa
    (tela cheia do Claude Code) é o que ele expõe."""
    fmt = "#{alternate_on}" if os.name == "nt" else "#{mouse_sgr_flag}"
    cp = tmux._run(["tmux", "display-message", "-p", "-t", tmux._pane_target(name), fmt])
    return cp.returncode == 0 and cp.stdout.strip() == "1"


def screen(name: str) -> list[str]:
    """Só a parte visível: é nela que as coordenadas do mouse valem."""
    return tmux.capture_pane(name, 0).split("\n")


def click(name: str, row: int, col: int) -> bool:
    seq = f"\x1b[<0;{col + 1};{row + 1}M\x1b[<0;{col + 1};{row + 1}m"
    return tmux._run(["tmux", "send-keys", "-t", tmux._pane_target(name), "-l", "--", seq]).returncode == 0


def _site(name: str, site: str) -> tuple[dict | None, str | None]:
    """Árvore e posição do site: a faixa, ou um painel aberto."""
    _, dados = plugin_bridge.band(name)
    if site == BAND_SITE:
        return dados["above"], None
    painel = next((p for p in dados["panes"] if p.get("id") == site), None)
    if painel is None:
        raise PressRefused("erro_mod_botao_inexistente", "O painel do mod não está aberto.")
    return painel.get("tree"), painel.get("placement")


def _regiao(tela: list[str], name: str, site: str, placement: str | None) -> tuple[range, int, int | None] | None:
    top = prompt_top(tela)
    if top is None:
        return None
    largura = plugin_bridge.band_columns(name)
    inicio = band_start(tela, top, plugin_bridge.band_anchor(name))
    if site == BAND_SITE:
        return range(inicio, top), 0, largura
    if placement == "dock":
        return range(0, top), largura or 0, None
    return range(0, inicio), 0, None


async def _esperar_fechar(name: str, site: str, timeout: float) -> bool:
    fim = time.monotonic() + timeout
    versao, dados = plugin_bridge.band(name)
    while any(p.get("id") == site for p in dados["panes"]):
        resta = fim - time.monotonic()
        if resta <= 0:
            return False
        versao = await plugin_bridge.esperar_faixa(name, versao, resta)
        dados = plugin_bridge.band(name)[1]
    return True


async def press(name: str, site: str, key: str) -> dict:
    async with _locks.setdefault(name, asyncio.Lock()):
        tree, placement = _site(name, site)
        rotulo = CLOSE_LABEL if key == CLOSE_KEY and site != BAND_SITE else button_label(tree, key)
        if not rotulo:
            raise PressRefused("erro_mod_botao_inexistente", "O botão não está mais na tela do mod.")
        if not await run_tmux(mouse_ready, name):
            raise PressRefused("erro_mod_mouse_desligado", "O terminal da sessão não está com o mouse ligado.")
        tela = await run_tmux(screen, name)
        regiao = _regiao(tela, name, site, placement)
        achados = find_label(tela, rotulo, *regiao) if regiao else []
        if not achados:
            raise PressRefused("erro_mod_botao_nao_achado", f"Não achei “{rotulo}” na tela do terminal.", rotulo=rotulo)
        if len(achados) > 1:
            raise PressRefused("erro_mod_botao_ambiguo", f"“{rotulo}” aparece mais de uma vez na tela.", rotulo=rotulo)
        linha, coluna = achados[0]
        desde = time.monotonic()
        if not await run_tmux(click, name, linha, coluna):
            raise PressRefused("erro_mod_clique_sem_resposta", "O clique não chegou ao terminal.")
        if key == CLOSE_KEY:
            if not await _esperar_fechar(name, site, CONFIRM_S):
                raise PressRefused("erro_mod_clique_sem_resposta", "O painel não fechou.")
            return {"ok": True}
        if not await plugin_bridge.esperar_press(name, site, key, desde, CONFIRM_S):
            raise PressRefused("erro_mod_clique_sem_resposta", "O mod não confirmou o clique.")
        copiado = await plugin_bridge.esperar_copia(name, desde, COPY_S)
        return {"ok": True, "copied": copiado} if copiado else {"ok": True}
