"""Velocidade de geração (tok/s) medida no stream da resposta, por sessão.

Não importa nada do app: `stats` e os adapters se importam em ciclo.
"""
from __future__ import annotations

import time
from collections import deque
from collections.abc import Sequence

# Chamadas que entram na média "recente" da velocidade.
RECENT_CALLS = 10
# Resposta mais curta que isto é ruído de relógio, não velocidade.
MIN_GEN_S = 0.2
# Transcript com resposta mais nova que a última medida por esta folga: a fonte parou de medir.
_STALE_S = 30.0


def rates(calls: Sequence[tuple[int, float]]) -> dict:
    """`tok_s_now`: a última resposta; `tok_s_recent`: as últimas chamadas somadas."""
    if not calls:
        return {}
    return {"tok_s_now": round(calls[-1][0] / calls[-1][1], 1),
            "tok_s_recent": round(sum(t for t, _ in calls) / sum(s for _, s in calls), 1)}


class LiveRate:
    """Do `message_start` ao fim da resposta, com o `output_tokens` real. O relógio não parte do
    primeiro pedaço de texto: o pensamento resumido chega segundos depois de gerado. Sem
    estimativa em voo, porque contar caracteres desse resumo dava metade do real."""

    def __init__(self) -> None:
        # (tokens, segundos, conversa, relógio de parede do fim)
        self._calls: deque[tuple[int, float, str, float]] = deque(maxlen=RECENT_CALLS)

    def close(self, tokens: int, seconds: float, conversation: str) -> None:
        if tokens > 0 and seconds >= MIN_GEN_S:
            self._calls.append((tokens, seconds, conversation, time.time()))

    def snapshot(self, conversation: str, transcript_call_ts: float | None) -> dict:
        # Só a conversa mostrada: /clear e sessão nova com o mesmo nome trocam o id.
        calls = [c for c in self._calls if c[2] == conversation]
        if not calls:
            return {}
        if transcript_call_ts is not None and transcript_call_ts > calls[-1][3] + _STALE_S:
            return {}
        out = rates([(t, s) for t, s, _, _ in calls])
        out["tok_s_exact"] = True
        return out


# ponytail: um por nome de sessão, sem limpeza; cada um guarda 10 medidas.
_live_rates: dict[str, LiveRate] = {}


def live_rate(name: str) -> LiveRate:
    return _live_rates.setdefault(name, LiveRate())


def live_snapshot(name: str, conversation: str, transcript_call_ts: float | None) -> dict:
    rate = _live_rates.get(name)
    return rate.snapshot(conversation, transcript_call_ts) if rate else {}
