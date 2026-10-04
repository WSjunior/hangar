"""Contexto de uma sessão Claude lido do transcript, sem depender da statusline.

A statusline só traz o contexto quando é a do Hangar; com outra, o app ficava sem o número. O
transcript sempre traz: o `usage` da última resposta do agente principal é o pedido inteiro
(entrada + cache lido + cache escrito). O tamanho da janela não vem nele (o id do modelo não diz
se é a versão de 1M), então sai do modelo configurado e do próprio uso.
"""
from __future__ import annotations

import json
import os
import re
from pathlib import Path

# Fim do arquivo lido: a última resposta fica perto do fim, e um transcript longo pesa megabytes.
_TAIL = 512 * 1024
WINDOW_DEFAULT = 200_000
WINDOW_1M = 1_000_000
# `claude-haiku-4-5-20251001`: a data do snapshot não é parte do nome que a tela mostra.
_DATED = re.compile(r"-\d{8}$")
_FAMILY = re.compile(r"^(?:claude-)?(opus|sonnet|haiku|fable)\b", re.I)


def from_transcript(jsonl: str | Path | None, config_dir: str | Path | None = None,
                    model: str | None = None, window_tokens: int | None = None) -> dict | None:
    """`{"used", "window"}` da última resposta do agente principal, ou None sem resposta com uso.

    `model` é o modelo da própria sessão e vence o da conta; `window_tokens` é a janela declarada
    (`CLAUDE_CODE_MAX_CONTEXT_TOKENS`) e vence os dois."""
    return read(jsonl, config_dir, model, window_tokens)[0]


def read(jsonl: str | Path | None, config_dir: str | Path | None = None,
         model: str | None = None, window_tokens: int | None = None) -> tuple[dict | None, str | None]:
    """Contexto (como `from_transcript`) e id do modelo da última resposta do agente principal,
    numa leitura só; cada um é None quando o transcript não traz."""
    if not jsonl:
        return None, None
    try:
        with open(jsonl, "rb") as fh:
            fh.seek(0, os.SEEK_END)
            fh.seek(max(0, fh.tell() - _TAIL))
            lines = fh.read().split(b"\n")
    except OSError:
        return None, None
    for line in reversed(lines):
        if b'"usage"' not in line:
            continue
        try:
            obj = json.loads(line)
        except ValueError:
            continue
        # Subagente roda noutro contexto; a resposta sintética (erro, interrupção) não tem uso real.
        if not isinstance(obj, dict) or obj.get("type") != "assistant" or obj.get("isSidechain"):
            continue
        message = obj.get("message") or {}
        usage = message.get("usage") if isinstance(message, dict) else None
        if not isinstance(usage, dict) or message.get("model") == "<synthetic>":
            continue
        used = sum(int(usage.get(k) or 0) for k in
                   ("input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"))
        if used > 0:
            answered = message.get("model")
            return ({"used": used, "window": window(used, config_dir, model, window_tokens)},
                    answered if isinstance(answered, str) and answered else None)
    return None, None


def session_model(answered: str | None, opened: str | None, config_dir: str | Path | None = None,
                  used: int = 0, engine: bool = False) -> str | None:
    """Id do modelo em uso na sessão, para a tela mostrar sem depender da statusline: o da última
    resposta, senão o da abertura (`--model` ou sidecar) e por fim o `model` da conta.

    O transcript não diz se é a variante de 1M: o `[1m]` volta quando o uso só cabe nela ou quando
    o modelo configurado é a variante de 1M da mesma família. Sessão de motor não cai na conta, que
    guarda o modelo da Anthropic."""
    configured = opened or (None if engine else _account_model(config_dir))
    if not answered:
        return configured
    base = _DATED.sub("", answered)
    family = _family(base)
    if not family or base.lower().endswith("[1m]"):
        return base
    same_1m = bool(configured) and configured.lower().endswith("[1m]") and _family(configured) == family
    return f"{base}[1m]" if used > WINDOW_DEFAULT or same_1m else base


def _family(model: str) -> str | None:
    found = _FAMILY.search(model)
    return found.group(1).lower() if found else None


def window(used: int, config_dir: str | Path | None = None, model: str | None = None,
           window_tokens: int | None = None) -> int:
    """1M quando o modelo é a variante `[1m]` ou quando o uso já passou da janela padrão (só cabe
    na de 1M); senão a padrão. O modelo da sessão vence o da conta: o Hangar abre a sessão com
    `--model opus[1m]` sem mexer no `settings.json`."""
    if window_tokens:
        return window_tokens
    is_1m = model.lower().endswith("[1m]") if model else _model_1m(config_dir)
    if used > WINDOW_DEFAULT or is_1m:
        return WINDOW_1M
    return WINDOW_DEFAULT


def _account_model(config_dir: str | Path | None) -> str | None:
    base = Path(config_dir) if config_dir else Path(os.environ.get("CLAUDE_CONFIG_DIR") or Path.home() / ".claude")
    try:
        model = json.loads((base / "settings.json").read_text(encoding="utf-8")).get("model")
    except (OSError, ValueError, AttributeError):
        return None
    return model if isinstance(model, str) and model else None


def _model_1m(config_dir: str | Path | None) -> bool:
    return (_account_model(config_dir) or "").lower().endswith("[1m]")
