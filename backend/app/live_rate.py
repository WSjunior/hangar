"""Velocidade de geração (tok/s) medida no stream da resposta, por sessão.

Não importa nada do app: `stats` e os adapters se importam em ciclo.
"""
from __future__ import annotations

from collections import deque
from collections.abc import Sequence

# Chamadas que entram na média "recente" da velocidade.
RECENT_CALLS = 10
# Resposta mais curta que isto é ruído de relógio, não velocidade.
MIN_GEN_S = 0.2


def rates(calls: Sequence[tuple[int, float]]) -> dict:
    """`tok_s_now`: a última resposta; `tok_s_recent`: as últimas chamadas somadas."""
    if not calls:
        return {}
    return {"tok_s_now": round(calls[-1][0] / calls[-1][1], 1),
            "tok_s_recent": round(sum(t for t, _ in calls) / sum(s for _, s in calls), 1)}


class LiveRate:
    """Do primeiro pedaço da resposta ao fim dela, com o `output_tokens` real. Sem estimativa
    durante a geração: o pensamento chega resumido, e contar caracteres dava metade do real."""

    def __init__(self) -> None:
        self._calls: deque[tuple[int, float]] = deque(maxlen=RECENT_CALLS)

    def close(self, tokens: int, seconds: float) -> None:
        if tokens > 0 and seconds >= MIN_GEN_S:
            self._calls.append((tokens, seconds))

    def snapshot(self) -> dict:
        out = rates(self._calls)
        if out:
            out["tok_s_exact"] = True
        return out


# ponytail: um por nome de sessão, sem limpeza; cada um guarda 10 pares.
_live_rates: dict[str, LiveRate] = {}


def live_rate(name: str) -> LiveRate:
    return _live_rates.setdefault(name, LiveRate())


def live_snapshot(name: str) -> dict:
    rate = _live_rates.get(name)
    return rate.snapshot() if rate else {}
