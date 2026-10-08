from types import SimpleNamespace

import pytest

from app import rust_server
from app.runtime_coordinator import Binding, RuntimeCoordinator

CLAUDE = [{"provider": "claude", "headless": True}, {"provider": "claude", "headless": False}]
WITH_CODEX = CLAUDE + [{"provider": "codex", "headless": True}]


def binding(tmp_path, provider="claude", headless=True):
    return Binding(name="s", key="k", provider=provider, headless=headless, meta={}, jsonl="x",
        projection_dir=tmp_path, state_path=tmp_path / "q.json", lock_path=tmp_path / "q.lock", generation=1)


def coordinator(owns, transport=object()):
    owner = RuntimeCoordinator(None, None)
    owner.transport = transport
    if owns is not None:
        owner._owns = {(o["provider"], o["headless"]) for o in owns}
    return owner


def test_only_claude_owned_keeps_codex_in_python(tmp_path):
    owner = coordinator(CLAUDE)
    assert owner.rust_owns("claude", True) and owner.rust_owns("claude", False)
    assert not owner.rust_owns("codex", True) and not owner.rust_owns("codex", False)
    assert owner._born_in_rust(binding(tmp_path))
    assert not owner._born_in_rust(binding(tmp_path, headless=False)), "terminal tem caminho próprio"
    assert not owner._born_in_rust(binding(tmp_path, "codex"))


def test_codex_headless_owned_is_born_in_rust(tmp_path):
    owner = coordinator(WITH_CODEX)
    assert owner._born_in_rust(binding(tmp_path, "codex"))
    assert not owner._born_in_rust(binding(tmp_path, "codex", headless=False))


def test_no_transport_is_never_born_in_rust(tmp_path):
    assert not coordinator(WITH_CODEX, transport=None)._born_in_rust(binding(tmp_path, "codex"))


def test_default_before_the_handshake_is_claude():
    owner = coordinator(None)
    assert owner.rust_owns("claude", True) and not owner.rust_owns("codex", True)


def test_configure_transport_stores_the_advertised_owns():
    import asyncio
    owner = RuntimeCoordinator(None, None)
    async def scenario():
        owner.configure_transport(SimpleNamespace(instance="i"), WITH_CODEX)
        owner.events_task.cancel()
    transport_events = owner._events
    owner._events = lambda *a: asyncio.sleep(0)
    asyncio.run(scenario())
    owner._events = transport_events
    assert owner.rust_owns("codex", True)


@pytest.mark.parametrize("bad", [None, "claude", [{"provider": "claude"}], [{"provider": 1, "headless": True}],
                                 [{"provider": "claude", "headless": "yes"}], ["claude"]])
def test_health_owns_must_be_well_formed(bad):
    assert rust_server.parse_owns(bad) is None


def test_health_owns_parses():
    assert rust_server.parse_owns(CLAUDE) == CLAUDE
