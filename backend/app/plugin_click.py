"""Clique de botão de mod pedido pelo app: acha o botão na tela do pane e clica pelo mouse do terminal.

Nenhuma API do engine deixa um plugin disparar o botão de outro (o `onPress` mora no ambiente do mod).
O clique entra como clique de mouse SGR no pane, na célula do rótulo, e o plugin do Hangar confirma
pelo `ui.press` que o press chegou ao botão certo."""
import asyncio
import os
import time

from app import plugin_bridge, tmux
from app.mensagens import erro
from app.plugin_screen import anchor_row, band_start, find_label, prompt_top
from app.state import run_tmux

BAND_SITE = "above-prompt"
CLOSE_KEY = "__close__"
CLOSE_LABEL = "✕"
CONFIRM_S = 2.0
# O `onPress` do mod costuma copiar ou abrir sem `await`: o efeito pode chegar logo depois do press.
EFFECT_S = 0.3
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


def _tmux_format(name: str, fmt: str) -> str:
    cp = tmux._run(["tmux", "display-message", "-p", "-t", tmux._pane_target(name), fmt])
    return cp.stdout.strip() if cp.returncode == 0 else ""


def terminal_refusal(name: str) -> str | None:
    """Por que o terminal não pode receber o clique agora; None quando pode."""
    # O psmux não tem a flag de mouse, e o `send-keys -l` com ESC lá não foi provado: sem prova,
    # bytes de mouse num terminal sem mouse cairiam como texto no prompt.
    if os.name == "nt":
        return "erro_mod_clique_windows"
    mouse, em_modo = (_tmux_format(name, "#{mouse_sgr_flag} #{pane_in_mode}").split() + ["", ""])[:2]
    # Em copy-mode o ESC do clique cancela o modo e o resto da sequência vira texto no prompt.
    if em_modo == "1":
        return "erro_mod_terminal_em_modo"
    return None if mouse == "1" else "erro_mod_mouse_desligado"


_RECUSAS = {
    "erro_mod_clique_windows": "No Windows o clique pelo app ainda não está disponível; clique pelo terminal.",
    "erro_mod_terminal_em_modo": "O terminal da sessão está em modo de rolagem; saia dele e tente de novo.",
    "erro_mod_mouse_desligado": "O terminal da sessão não está com o mouse ligado.",
}


def screen(name: str) -> list[str]:
    """Só a parte visível: é nela que as coordenadas do mouse valem."""
    return tmux.capture_pane(name, 0).split("\n")


def click(name: str, row: int, col: int) -> bool:
    return tmux.send_keys(name, f"\x1b[<0;{col + 1};{row + 1}M\x1b[<0;{col + 1};{row + 1}m", literal=True)


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
    ancora = plugin_bridge.band_anchor(name)
    if site == BAND_SITE:
        return range(band_start(tela, top, ancora), top), 0, largura
    if placement == "dock":
        return range(0, top), largura or 0, None
    # Inline fica acima da faixa; sem faixa visível (mod que só abre painel), logo acima do prompt.
    linha = anchor_row(tela, top, ancora)
    return range(0, top if linha is None else linha), 0, None


async def press(name: str, site: str, key: str) -> dict:
    async with _locks.setdefault(name, asyncio.Lock()):
        tree, placement = _site(name, site)
        rotulo = CLOSE_LABEL if key == CLOSE_KEY and site != BAND_SITE else button_label(tree, key)
        if not rotulo:
            raise PressRefused("erro_mod_botao_inexistente", "O botão não está mais na tela do mod.")
        recusa = await run_tmux(terminal_refusal, name)
        if recusa:
            raise PressRefused(recusa, _RECUSAS[recusa])
        tela = await run_tmux(screen, name)
        regiao = _regiao(tela, name, site, placement)
        achados = find_label(tela, rotulo, *regiao) if regiao else []
        if not achados:
            raise PressRefused("erro_mod_botao_nao_achado", f"Não achei “{rotulo}” na tela do terminal.", rotulo=rotulo)
        if len(achados) > 1:
            raise PressRefused("erro_mod_botao_ambiguo", f"“{rotulo}” aparece mais de uma vez na tela.", rotulo=rotulo)
        linha, coluna = achados[0]
        if key == CLOSE_KEY:
            if not await run_tmux(click, name, linha, coluna):
                raise PressRefused("erro_mod_clique_sem_resposta", "O clique não chegou ao terminal.")
            if not await plugin_bridge.esperar_sem_painel(name, site, CONFIRM_S):
                raise PressRefused("erro_mod_clique_sem_resposta", "O painel não fechou.")
            return {"ok": True}
        tentativa = plugin_bridge.esperar_clique_do_app(name, site, key, CONFIRM_S)
        try:
            desde = time.monotonic()
            if not await run_tmux(click, name, linha, coluna):
                raise PressRefused("erro_mod_clique_sem_resposta", "O clique não chegou ao terminal.")
            if not await plugin_bridge.esperar_press(name, site, key, desde, CONFIRM_S):
                raise PressRefused("erro_mod_clique_sem_resposta", "O mod não confirmou o clique.")
            copiado, aberto = await plugin_bridge.esperar_efeito(name, tentativa, EFFECT_S)
        finally:
            # Fechada aqui: efeito que chegar depois é recusado e acontece no terminal.
            plugin_bridge.encerrar_clique_do_app(name, tentativa)
        resposta: dict = {"ok": True}
        if copiado:
            resposta["copied"] = copiado
        if aberto:
            resposta["opened"] = aberto
        return resposta
