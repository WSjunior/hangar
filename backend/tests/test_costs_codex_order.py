"""A ordem de turnos do Codex não depende da semente de hash do processo."""
import json
import os
import subprocess
import sys

import pytest


PROBE = """
from app import costs
from app.costs_sources import DobraCodex
from pathlib import Path
import json

fold = DobraCodex(Path("synthetic.jsonl"))
events = [
    ("session_meta", 0, {"id": "s", "cwd": "/repo/synthetic"}),
    ("turn_context", 1, {"turn_id": "first", "model": "gpt-5.6-sol", "cwd": "/repo/first"}),
    ("token_usage_record", 2, {"thread_id": "s", "turn_id": "first", "response_id": "first-r", "usage": {"input_tokens": 100}}),
    ("turn_context", 3, {"turn_id": "second", "model": "gpt-5.6-sol", "cwd": "/repo/second"}),
    ("token_usage_record", 4, {"thread_id": "s", "turn_id": "second", "response_id": "second-r", "usage": {"input_tokens": 200}}),
    ("token_usage_record", 5, {"thread_id": "s", "turn_id": "first", "response_id": "first-r", "usage": {"input_tokens": 9999}}),
]
for kind, second, payload in events:
    fold.linha(json.dumps({"type": kind, "timestamp": f"2026-10-01T12:00:{second:02}Z", "payload": payload}).encode())
result = fold.fechar()
print(json.dumps({"turns": list(fold.respostas.por_turno()), "timestamp": result[0][0].ts.isoformat(),
                  "input": result[0][0].input, "project": result[0][0].project,
                  "area_cwd": [units[0][1] for _, units in result[2][1]]}))
"""


@pytest.mark.parametrize("seed", ["0", "1", "4", "42"])
def test_codex_order_and_group_timestamp_are_stable_between_processes(seed):
    env = dict(os.environ, PYTHONHASHSEED=seed)
    result = subprocess.run([sys.executable, "-c", PROBE], env=env, text=True,
                            capture_output=True, check=True)
    assert json.loads(result.stdout) == {
        "turns": ["first", "second"], "timestamp": "2026-10-01T09:00:02-03:00",
        "input": 300, "project": "/repo/synthetic", "area_cwd": ["/repo/first", "/repo/second"]}


def test_codex_order_preserves_legacy_then_modern_insertion():
    from app import costs
    from app.costs_sources import RespostasCodex

    reader = RespostasCodex("s", None)
    reader.registro({"type": "session_meta", "timestamp": "2026-10-01T12:00:00Z", "payload": {"id": "s"}})
    for turn, kind, tokens in [("modern-first", "token_usage_record", 100), ("legacy-first", "event_msg", 200)]:
        reader.registro({"type": "turn_context", "timestamp": "2026-10-01T12:00:01Z", "payload": {"turn_id": turn}})
        payload = ({"thread_id": "s", "usage": {"input_tokens": tokens}}
                   if kind == "token_usage_record" else {"type": "token_count", "info": {"total_token_usage": {"input_tokens": tokens}}})
        reader.registro({"type": kind, "timestamp": "2026-10-01T12:00:02Z", "payload": payload})
    assert list(reader.por_turno()) == ["legacy-first", "modern-first"]
