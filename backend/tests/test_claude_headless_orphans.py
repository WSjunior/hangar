from app.adapters.claude_headless.adapter import _orphan_key


def env(**fields):
    return b"\0".join(f"{k}={v}".encode() for k, v in fields.items()) + b"\0"


def test_cano_of_another_backend_for_same_user_is_not_orphan():
    alheio = env(HOME="/home/u", HANGAR_CANO_KEY="k1", HANGAR_CANO_OWNER="/home/u")
    assert _orphan_key(alheio, set(), "/tmp/teste/home") is None
    assert _orphan_key(alheio, set(), "/home/u") == "k1"


def test_live_session_and_unmarked_process_stay():
    assert _orphan_key(env(HANGAR_CANO_KEY="k1", HANGAR_CANO_OWNER="/h"), {"k1"}, "/h") is None
    assert _orphan_key(env(HOME="/h", PATH="/bin"), set(), "/h") is None


def test_legacy_cano_without_owner_uses_inherited_home():
    assert _orphan_key(env(HOME="/h", HANGAR_CANO_KEY="k1"), set(), "/h") == "k1"
    # Codex troca o HOME pelo da conta: sem prova de dono, fica vivo.
    assert _orphan_key(env(HOME="/h/.hangar/codex-contas/a", HANGAR_CANO_KEY="k2"), set(), "/h") is None
    assert _orphan_key(env(HANGAR_CANO_KEY="k3"), set(), "/h") is None
