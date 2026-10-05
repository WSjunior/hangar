# backend/tests/fixtures/contract/gen_api_samples.py
"""Amostras JSON dos formatos da conversa, geradas pelos modelos reais de app/models.py.

O crate crates/hangar-api lê cada arquivo, escreve de volta e compara: campo que mudar aqui e não
lá quebra o teste de ida e volta. Depois de mexer em ChatEvent, StateEvent, PreviewEvent,
AskQuestion ou SessionInfo, rodar de backend/:

    uv run python tests/fixtures/contract/gen_api_samples.py
"""
from __future__ import annotations

import sys
from pathlib import Path
from typing import get_args

HERE = Path(__file__).resolve().parent
OUT = HERE / "api_samples"
# Rodado como script, o pacote `app` não está no caminho de import: a raiz é backend/.
sys.path.insert(0, str(HERE.parents[2]))

from app.models import (  # noqa: E402
    AskOption,
    AskQuestion,
    AskQuestionItem,
    ChatEvent,
    ChatKind,
    PreviewEvent,
    SessionInfo,
    ShellVivo,
    StateEvent,
)


def _models() -> dict:
    cases = {
        "chat_minimal": ChatEvent(kind="user_msg", id="u1"),
        "chat_full": ChatEvent(
            kind="tool_use", id="toolu_01", text="acentuação e emoji 🚀", tool_name="Bash",
            tool_input={"command": "ls -la", "nested": {"list": [1, 2.5, None, True, "x"]}},
            tool_use_id="toolu_01", result="ok", is_error=False,
            patch=[{"old_start": 6, "new_start": 6, "lines": [" a", "-b", "+B"]}], ts=1727712000.123,
            cache_read=1234, cache_ttl_s=3600, desistiu=True, hook_error="hook recusou",
            skill={"name": "pdf", "path": "/skills/pdf/SKILL.md", "body": "# PDF"},
            orq={"kind": "woke", "task": 4, "body": "texto", "alarm": False},
            queued_delivered=True, queued_ts=1727712001.5, queued_confirmed=False, image_count=2,
            offset=999,  # exclude=True: não aparece no JSON
        ),
        "state_minimal": StateEvent(session="s1", state="idle"),
        "state_full": StateEvent(
            session="s1", state="awaiting_input", codex_mode="plan",
            codex_question={"provider": "codex", "request_id": 7, "is_async": False, "questions": []},
            codex_buffering=True, claude_permission_mode="acceptEdits",
            claude_previous_non_plan="default",
            claude_plan_pending={"plan": "# Plano", "path": "/tmp/p.md", "tool_use_id": "toolu_9"},
            label="Pensando…", question="Continuar?", options=["Sim", "Não"],
            status_line="opus · 12%", overlay=True, login=True, limited=True, limit_reset="3pm",
            loop_status="rodando", loop_iter=2, loop_max=5, problema="turno_com_erro",
            problema_detalhe="detalhe", headless=True, recarregar_motivo="config",
            shells=[ShellVivo(pid=42, cmd="sleep 999", desde=1727712000.5),
                    ShellVivo(pid=43, cmd="tail -f x")],
        ),
        "preview_minimal": PreviewEvent(session="s1", text=""),
        "preview_full": PreviewEvent(session="s1", text="**negrito** e acentuação", md=True,
                                     full=True, vivo=True),
        "ask_minimal": AskQuestion(questions=[]),
        "ask_full": AskQuestion(questions=[AskQuestionItem(
            header="Escolha", question="Qual caminho?", multiSelect=True,
            options=[AskOption(label="A", description="primeiro", preview="```\nA\n```"),
                     AskOption(label="B")],
        )]),
        "session_minimal": SessionInfo(name="s1"),
        "session_full": SessionInfo(
            name="s1", lifecycle_id="lc-1", transfer_id="tr-1", transfer_phase="copying",
            cwd="/home/u/repo", jsonl="/home/u/.claude/projects/x/u.jsonl", provider="codex",
            headless=True, engine="kimi", engine_account="kimi-coding", codex_home="/home/u/.codex",
            conta="claude:/home/u/.claude", state="awaiting_input", last_activity=1727712000.5,
            last_reply="acentuação e emoji 🚀", last_reply_at=1727712001.25, tracked=False,
            branch="feat/x", worktree=True, worktree_path="/home/u/repo-wt", worktree_gone=True,
            git_cwd="/home/u/repo-wt", git_dirty=3, git_ahead=1, git_behind=0, git_added=10,
            git_removed=2, avisos=["plugin sem instalação"], label="Pensando…",
            startup_steps=["hooks", "mcp"], question="Continuar?", pending_questions=2,
            options=["Sim", "Não"], stalled=True, problema="codex_hooks_nao_aprovados",
            limited=True, limit_reset="3pm", shared=True, guest_kind="pair", owner="ana",
            then_target="s2", status_line="opus · 12%",
            context={"used": 120000, "window": 1000000}, model="claude-opus-5-5[1m]",
            pair_peers=["s2", "s3"], pair_external={"alias": "bia", "owner": "Bia", "session": "x"},
            pair_gid="g1", pair_task="ABC-1234", orq_arbiter="arb-1", loop_status="rodando",
            loop_iter=2, loop_max=5, plan_name="plano.md", plan_task=0, plan_task_total=3,
            plan_done=4, plan_total=9, plan_complete=False, plan_tasks=[(2, 2), (2, 4), (0, 3)],
            plan_hidden=True,
        ),
    }
    # Um por tipo: tipo novo no models.py vira amostra nova, e o teste Rust cobra a variante.
    for kind in get_args(ChatKind):
        cases[f"chat_kind_{kind}"] = ChatEvent(kind=kind, id=f"{kind}-1", text="x")
    return cases


def samples() -> dict[str, str]:
    """Nome do arquivo -> conteúdo exato, como o backend serializa (`model_dump_json`)."""
    return {f"{name}.json": model.model_dump_json() + "\n" for name, model in _models().items()}


def main() -> None:
    OUT.mkdir(exist_ok=True)
    want = samples()
    for old in OUT.glob("*.json"):
        if old.name not in want:
            old.unlink()
    for name, text in want.items():
        # newline="\n": gerado no Windows sai igual ao do Linux.
        (OUT / name).write_text(text, encoding="utf-8", newline="\n")
    print(f"{len(want)} amostras em {OUT}")


if __name__ == "__main__":
    main()
