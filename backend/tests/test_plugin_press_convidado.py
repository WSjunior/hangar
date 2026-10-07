"""O convidado não clica em mod de sessão cujo terminal é do Rust (fase 3 dos mods).

O Rust repassa ao Python todo pedido de app que não é do dono, e o convite entra pela porta 8766 sem
passar por ele: a recusa mora no `plugin_press` daqui, depois da autenticação dos dois porteiros.
"""
import dataclasses

import pytest
from fastapi.testclient import TestClient

from app import api, guest_users, plugin_click, runtime_coordinator, share_api, share_gate, share_store
from app.config import settings
from app.runtime_coordinator import Binding, Phase, RuntimeCoordinator
from app.share_tunnel import GUEST_PORT

OWNER = "segredo-do-dono"
INVITE = "token-do-convite"
SHARE = share_store.Share(
    id="s1", session="t", life="L1", created_at=0.0, code_expires_at=0.0, code_hash="c",
    token_hash="h", device="Pixel", redeemed_at=1.0, revoked_at=None)
BODY = {"site": "above-prompt", "key": "mr-a", "plugin": "pm-mock"}


def _register(coordinator, tmp_path, *, headless):
    meta = {"key": "key"} if headless else {"key": "key", "terminal": {}}
    slot = coordinator.register(Binding(name="t", key="key", provider="claude", headless=headless, meta=meta,
        jsonl=str(tmp_path / "chat.jsonl"), projection_dir=tmp_path / "projection",
        state_path=tmp_path / "key.json", lock_path=tmp_path / "key.lock", generation=1))
    slot.lease.close()
    return slot


@pytest.fixture
def env(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "auth_token", OWNER)
    monkeypatch.setattr(guest_users, "_path_override", tmp_path / "guests.json")
    monkeypatch.setattr(guest_users, "session_life", lambda name: "L1" if name == "t" else None)
    guest_users._reset()
    (tmp_path / "raiz").mkdir()
    guest, guest_token = guest_users.create("ana", str(tmp_path / "raiz"), False, True)
    guest_users.claim("t", guest.id)
    # Convite da sessão `t`, vivo.
    monkeypatch.setattr(share_store, "lookup_token", lambda token: share_store.Guest([SHARE]) if token == INVITE else None)
    monkeypatch.setattr(share_gate, "session_life", lambda name: "L1" if name == "t" else None)
    monkeypatch.setattr(share_api, "confirmed_absent", lambda name: False)
    share_gate._life_cache.clear()
    pressed = []

    async def press(name, site, key, plugin):
        pressed.append((name, site, key, plugin))
        return {"ok": True}

    async def close(name, site):
        pressed.append(("close", name, site))
        return {"ok": True}

    monkeypatch.setattr(plugin_click, "press", press)
    monkeypatch.setattr(plugin_click, "close", close)
    api.app.dependency_overrides[api._transfer_guard] = lambda: None
    coordinator = RuntimeCoordinator()
    coordinator.instance = "instance-1"
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    yield coordinator, guest_token, pressed
    api.app.dependency_overrides.pop(api._transfer_guard, None)
    guest_users._reset()


def _press(token, *, invite_port=False, route="press", body=BODY):
    base = f"http://testserver:{GUEST_PORT}" if invite_port else "http://testserver"
    return TestClient(api.app, base_url=base).post(f"/api/sessions/t/plugin/{route}", json=body,
                                                   headers={"Authorization": f"Bearer {token}"})


def _refused(response):
    return response.status_code == 403 and response.json()["detail"]["code"] == "erro_mod_convidado"


def test_terminal_in_rust_needs_rust_phase_and_a_claude_terminal(env, tmp_path):
    coordinator, _, _ = env
    slot = _register(coordinator, tmp_path, headless=False)
    assert not coordinator.terminal_in_rust("t"), "terminal ainda na posse do Python"
    slot.phase = Phase.Rust
    assert coordinator.terminal_in_rust("t")
    assert not coordinator.terminal_in_rust("outra")


def test_guest_with_login_is_refused_on_a_rust_terminal(env, tmp_path):
    coordinator, guest_token, pressed = env
    _register(coordinator, tmp_path, headless=False).phase = Phase.Rust
    response = _press(guest_token)
    assert _refused(response), response.text
    assert "convidado" in response.json()["detail"]["msg"]
    assert pressed == []


def test_invite_guest_is_refused_on_a_rust_terminal(env, tmp_path):
    coordinator, _, pressed = env
    _register(coordinator, tmp_path, headless=False).phase = Phase.Rust
    response = _press(INVITE, invite_port=True)
    assert _refused(response), response.text
    assert pressed == []


def test_owner_still_presses_on_a_rust_terminal(env, tmp_path):
    coordinator, _, pressed = env
    _register(coordinator, tmp_path, headless=False).phase = Phase.Rust
    response = _press(OWNER)
    assert response.status_code == 200, response.text
    assert pressed == [("t", "above-prompt", "mr-a", "pm-mock")]


@pytest.mark.parametrize("headless,phase", [(False, Phase.Python), (True, Phase.Rust)])
def test_guests_still_press_when_the_terminal_is_not_the_rusts(env, tmp_path, headless, phase):
    coordinator, guest_token, pressed = env
    _register(coordinator, tmp_path, headless=headless).phase = phase
    assert _press(guest_token).status_code == 200
    assert _press(INVITE, invite_port=True).status_code == 200
    assert pressed == [("t", "above-prompt", "mr-a", "pm-mock")] * 2


def test_guests_still_press_without_a_runtime_registry(env, monkeypatch):
    _, guest_token, pressed = env
    monkeypatch.setattr(runtime_coordinator, "_current", None)
    assert _press(guest_token).status_code == 200
    assert pressed == [("t", "above-prompt", "mr-a", "pm-mock")]


def test_revoked_invite_never_reaches_the_refusal(env, tmp_path, monkeypatch):
    # O porteiro do convite responde antes: o 410 diz "encerrado", não a recusa do mod.
    coordinator, _, pressed = env
    _register(coordinator, tmp_path, headless=False).phase = Phase.Rust
    revoked = dataclasses.replace(SHARE, revoked_at=2.0)
    monkeypatch.setattr(share_store, "lookup_token", lambda token: share_store.Guest([revoked]) if token == INVITE else None)
    assert _press(INVITE, invite_port=True).status_code == 410
    assert pressed == []


def test_refusal_under_the_barrier_reaches_the_app_as_the_guest_code(env, monkeypatch):
    # Sem slot, a recusa rápida não vê o Rust; a do empréstimo do teclado (`GuestRefused`) vira o mesmo
    # 403. A marca de convidado chega ao clique; a do dono não.
    from app import runtime_terminal
    _, guest_token, _ = env
    marks = []

    async def press(name, site, key, plugin):
        marks.append(runtime_terminal.guest_admin.get())
        if runtime_terminal.guest_admin.get():
            raise runtime_terminal.GuestRefused("convidado")
        return {"ok": True}

    monkeypatch.setattr(plugin_click, "press", press)
    assert _refused(_press(guest_token))
    assert _refused(_press(INVITE, invite_port=True))
    assert _press(OWNER).status_code == 200
    assert marks == [True, True, False]


def test_the_button_comes_with_its_mod(env):
    # A `key` só é única dentro de um mod: sem ele o pedido não diz qual botão é.
    _, _, pressed = env
    for body in ({"site": "above-prompt", "key": "mr-a"}, {"site": "above-prompt", "key": "mr-a", "plugin": ""}):
        assert _press(OWNER, body=body).status_code == 422
    assert pressed == []


def test_closing_a_pane_has_its_own_route_and_the_same_guest_refusal(env, tmp_path):
    coordinator, guest_token, pressed = env
    close = {"site": "pm-mock-mr"}
    assert _press(OWNER, route="close", body=close).status_code == 200
    assert _press(OWNER, route="close", body={"site": "pm-mock-mr", "key": "x"}).status_code == 422
    assert pressed == [("close", "t", "pm-mock-mr")]
    _register(coordinator, tmp_path, headless=False).phase = Phase.Rust
    assert _refused(_press(guest_token, route="close", body=close))
    assert _refused(_press(INVITE, invite_port=True, route="close", body=close))
    assert pressed == [("close", "t", "pm-mock-mr")]
