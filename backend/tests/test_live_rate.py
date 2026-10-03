"""tok/s: medida do stream (live_rate) e reserva do transcript (stats)."""
import json
import time

from app.live_rate import LiveRate
from app.stats import Accumulator


def test_medida_vale_so_para_a_conversa_e_ate_o_transcript_passar_a_frente():
    r = LiveRate()
    r.close(100, 1.0, "a")
    r.close(10, 0.1, "a")                 # curta demais: ruído de relógio
    assert r.snapshot("a", time.time()) == {"tok_s_now": 100.0, "tok_s_recent": 100.0,
                                            "tok_s_exact": True}
    # Transcript com resposta bem mais nova: quem media parou (plugin fora, CLI sem ele).
    assert r.snapshot("a", time.time() + 60) == {}
    # /clear ou sessão nova com o mesmo nome: outra conversa não herda as medidas.
    assert r.snapshot("b", None) == {}
    r.close(50, 1.0, "b")
    assert r.snapshot("b", None)["tok_s_recent"] == 50.0
    # Ler não apaga: duas conexões olhando conversas diferentes não se atrapalham.
    assert r.snapshot("a", None)["tok_s_now"] == 100.0


def test_reserva_vai_ate_o_ultimo_bloco_e_parte_do_recado(tmp_path):
    p = tmp_path / "s.jsonl"
    linhas = [
        # Recado de outra sessão: isMeta, não é turno, mas o relógio parte dele.
        {"type": "user", "isMeta": True, "timestamp": "2026-10-03T10:00:00Z",
         "message": {"content": "recado"}},
        {"type": "assistant", "timestamp": "2026-10-03T10:00:08Z",
         "message": {"id": "m1", "content": [{"type": "thinking"}], "usage": {"output_tokens": 1000}}},
        # Resultado de ferramenta gravado entre os blocos da mesma mensagem não fecha a chamada.
        {"type": "user", "timestamp": "2026-10-03T10:00:09Z",
         "message": {"content": [{"type": "tool_result", "tool_use_id": "x"}]}},
        {"type": "assistant", "timestamp": "2026-10-03T10:00:10Z",
         "message": {"id": "m1", "content": [{"type": "text"}], "usage": {"output_tokens": 1000}}},
    ]
    p.write_text("".join(json.dumps(o) + "\n" for o in linhas))
    snap = Accumulator("claude", str(p)).collect()
    assert snap["turns"] == 0
    assert snap["tok_s_now"] == 100.0     # 1000 tok / 10 s
    assert "tok_s_exact" not in snap


def test_subagente_fica_fora_do_tok_s(tmp_path):
    p = tmp_path / "s.jsonl"
    linhas = [
        {"type": "user", "timestamp": "2026-10-03T10:00:00Z", "message": {"content": "oi"}},
        {"type": "assistant", "isSidechain": True, "timestamp": "2026-10-03T10:00:05Z",
         "message": {"id": "s1", "content": [{"type": "text"}], "usage": {"output_tokens": 5000}}},
        {"type": "assistant", "timestamp": "2026-10-03T10:00:10Z",
         "message": {"id": "m1", "content": [{"type": "text"}], "usage": {"output_tokens": 500}}},
    ]
    p.write_text("".join(json.dumps(o) + "\n" for o in linhas))
    snap = Accumulator("claude", str(p)).collect()
    assert snap["out_tok"] == 5500
    assert snap["tok_s"] == 50.0          # 500 tok / 10 s; os 5000 do subagente não têm tempo aqui
