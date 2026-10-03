"""Identidade de UMA vida da sessão: muda quando a sessão morre e renasce com o mesmo nome.

Sessão sem terminal e Codex têm chave no sidecar que sobrevive a rename e /clear; sessão tmux
só tem o horário de nascimento.
"""
from __future__ import annotations

from app import tmux
from app.adapters.claude_headless import sessions as headless_sessions
from app.adapters.codex import sessions as codex_sessions


_UNSET = object()


def session_life(name: str, *, meta=_UNSET, birth=_UNSET) -> str | None:
    metadata = ((headless_sessions.load(name), codex_sessions.load(name))
                if meta is _UNSET else (meta,))
    for item in metadata:
        if item and item.get("key"):
            if item.get("transfer_id"):
                from app.conversation_transfer import load_transfer, TransferPhase
                record = load_transfer(item["transfer_id"])
                if record and record.name == name and record.phase not in {
                        TransferPhase.COMPLETE, TransferPhase.REJECTED, TransferPhase.ROLLED_BACK}:
                    return record.source_life
            return f"k:{item['key']}"
    created = _tmux_birth(name) if birth is _UNSET else birth
    if not created:
        from app.conversation_transfer import list_incomplete
        record = next((r for r in list_incomplete() if r.name == name), None)
        if record:
            return record.source_life
    return f"t:{created}" if created else None


def _tmux_birth(name: str) -> int | None:
    # `-t =nome` sem os dois pontos imprime linha vazia com rc 0 no tmux 3.x; `=nome:` devolve a época.
    cp = tmux._run(["tmux", "display-message", "-p", "-t", f"={name}:", "#{session_created}"])
    if cp.returncode != 0:
        return None
    try:
        return int(float(cp.stdout.strip()))
    except ValueError:
        return None
