import json
import os
import stat
import subprocess

import pytest

from app import share_store, share_life
from app.share_store import ShareError


@pytest.fixture(autouse=True)
def _arquivo_isolado(tmp_path, monkeypatch):
    monkeypatch.setattr(share_store, "_path_override", tmp_path / "shares.json")
    share_store._reset()
    yield
    share_store._reset()


def test_create_grava_so_hash_e_arquivo_0600(tmp_path):
    s, code = share_store.create("proj", "t:100", now=1000.0)
    assert len(code) == 26 and code.isalnum() and code.upper() == code
    raw = (tmp_path / "shares.json").read_text()
    assert code not in raw
    assert s.code_hash in raw
    assert s.code_expires_at == 1000.0 + share_store.CODE_TTL
    if os.name != "nt":
        assert stat.S_IMODE(os.stat(tmp_path / "shares.json").st_mode) == 0o600


def test_redeem_e_de_uso_unico():
    _, code = share_store.create("proj", "t:100", now=1000.0)
    s, token = share_store.redeem(code, "notebook do fulano", now=1001.0)
    assert s.device == "notebook do fulano" and s.redeemed_at == 1001.0
    assert share_store.lookup_token(token).share_for("proj").id == s.id
    with pytest.raises(ShareError) as e:
        share_store.redeem(code, "outro", now=1002.0)
    assert e.value.reason == "used"


def test_codigo_aceita_minusculas_e_espacos():
    _, code = share_store.create("proj", "t:100", now=1000.0)
    assert share_store.peek(f"  {code.lower()} ", now=1000.0).session == "proj"


def test_codigo_vencido_e_desconhecido():
    _, code = share_store.create("proj", "t:100", now=1000.0)
    with pytest.raises(ShareError) as e:
        share_store.peek(code, now=1000.0 + share_store.CODE_TTL + 1)
    assert e.value.reason == "expired"
    with pytest.raises(ShareError) as e:
        share_store.peek("AAAAAAAAAAAAAAAAAAAAAAAAAA", now=1000.0)
    assert e.value.reason == "unknown"


def test_revogar_derruba_token_mas_lookup_ainda_ve():
    s, code = share_store.create("proj", "t:100", now=1000.0)
    _, token = share_store.redeem(code, "x", now=1001.0)
    assert share_store.revoke(s.id) is True
    assert share_store.revoke(s.id) is False
    guest = share_store.lookup_token(token)
    assert not guest.sees("proj")
    assert guest.share_for("proj").revoked_at is not None
    with pytest.raises(ShareError) as e:
        share_store.peek(code, now=1002.0)
    assert e.value.reason == "revoked"


def test_revoke_session_rename_e_ativos():
    share_store.create("a", "t:1", now=1000.0)
    share_store.create("a", "t:1", now=1000.0)
    share_store.create("b", "t:2", now=1000.0)
    assert share_store.active_sessions(now=1000.0) == {"a", "b"}
    share_store.rename("b", "c")
    assert share_store.active_sessions(now=1000.0) == {"a", "c"}
    assert share_store.revoke_session("a") == 2
    assert share_store.list_for("a", now=1000.0) == []
    assert share_store.has_active(now=1000.0) is True


def test_codigo_vencido_sem_uso_nao_conta_como_ativo():
    share_store.create("a", "t:1", now=1000.0)
    assert share_store.has_active(now=1000.0 + share_store.CODE_TTL + 1) is False


def test_sweep_revoga_sessao_que_mudou_de_vida():
    _, code = share_store.create("a", "t:1", now=1000.0)
    _, token = share_store.redeem(code, "x", now=1001.0)
    share_store.create("b", "t:2", now=1000.0)
    n = share_store.sweep(lambda session, life: session == "b", now=1002.0)
    assert n == 1
    assert not share_store.lookup_token(token).sees("a")
    assert share_store.active_sessions(now=1002.0) == {"b"}


def test_sweep_apaga_codigo_vencido_velho():
    share_store.create("a", "t:1", now=1000.0)
    share_store.sweep(lambda s, l: True, now=1000.0 + share_store.CODE_TTL + share_store._KEEP_EXPIRED + 1)
    assert share_store._load() == {}


def test_persistencia_entre_recargas():
    s, code = share_store.create("a", "t:1", now=1000.0)
    _, token = share_store.redeem(code, "x", now=1001.0)
    share_store._reset()
    assert share_store.lookup_token(token).share_for("a").id == s.id


def test_arquivo_corrompido_nao_derruba(tmp_path):
    (tmp_path / "shares.json").write_text("{nao e json")
    share_store._reset()
    assert share_store.has_active() is False
    # Vai pro lado, senão o próximo save o apagaria.
    share_store.create("a", "t:1")
    assert [p.read_text() for p in tmp_path.glob("shares.json.bad-*")] == ["{nao e json"]


def test_set_life():
    s, _ = share_store.create("a", "t:1", now=1000.0)
    share_store.set_life("a", "k:abc")
    assert share_store._load()[s.id].life == "k:abc"


def test_session_life_prefere_chave_do_sidecar(monkeypatch):
    monkeypatch.setattr(share_life.headless_sessions, "load", lambda n: {"key": "abc"} if n == "h" else None)
    monkeypatch.setattr(share_life.codex_sessions, "load", lambda n: None)
    monkeypatch.setattr(share_life, "_tmux_birth", lambda n: 1700000000 if n == "t" else None)
    assert share_life.session_life("h") == "k:abc"
    assert share_life.session_life("t") == "t:1700000000"
    assert share_life.session_life("nada") is None


def _fake_run(monkeypatch, rc, out):
    calls = []

    def run(args, input=None):
        calls.append(args)
        return subprocess.CompletedProcess(args, rc, out, "")

    monkeypatch.setattr(share_life.tmux, "_run", run)
    return calls


def test_tmux_birth_usa_alvo_com_dois_pontos(monkeypatch):
    monkeypatch.setattr(share_life.headless_sessions, "load", lambda n: None)
    monkeypatch.setattr(share_life.codex_sessions, "load", lambda n: None)
    calls = _fake_run(monkeypatch, 0, "1790624703\n")
    assert share_life.session_life("proj") == "t:1790624703"
    assert calls[0][calls[0].index("-t") + 1] == "=proj:"


@pytest.mark.parametrize("rc,out", [(0, "\n"), (0, ""), (1, "1790624703\n"), (0, "abc\n")])
def test_tmux_birth_vazio_erro_ou_lixo_e_none(monkeypatch, rc, out):
    monkeypatch.setattr(share_life.headless_sessions, "load", lambda n: None)
    monkeypatch.setattr(share_life.codex_sessions, "load", lambda n: None)
    _fake_run(monkeypatch, rc, out)
    assert share_life.session_life("proj") is None


def test_sweep_pergunta_ao_tmux_sem_segurar_o_lock():
    import threading
    share_store.create("a", "t:1", now=1000.0)
    livres = []

    def alive(session, life):
        # RLock: só outra thread prova que o lock está solto (a mesma reentraria).
        def tenta():
            ok = share_store._lock.acquire(blocking=False)
            livres.append(ok)
            if ok:
                share_store._lock.release()
        t = threading.Thread(target=tenta)
        t.start()
        t.join()
        return True

    share_store.sweep(alive, now=1001.0)
    assert livres == [True]


def test_sweep_nao_revoga_convite_que_mudou_durante_a_consulta():
    s, _ = share_store.create("a", "t:1", now=1000.0)

    def alive(session, life):
        share_store.set_life("a", "t:2")     # troca de modo terminou no meio da consulta
        return False

    assert share_store.sweep(alive, now=1001.0) == 0
    assert share_store._load()[s.id].revoked_at is None


def test_has_any():
    assert share_store.has_any() is False
    share_store.create("a", "t:1", now=1000.0)
    assert share_store.has_any() is True


def test_recently_ended_so_dentro_da_janela():
    s, _ = share_store.create("a", "t:1", now=1000.0)
    assert share_store.recently_ended(120.0) is False
    share_store.revoke(s.id)
    revogado = share_store._load()[s.id].revoked_at
    assert share_store.recently_ended(120.0, now=revogado + 119) is True
    assert share_store.recently_ended(120.0, now=revogado + 121) is False


def test_redeem_com_token_existente_reaproveita_o_token():
    a, ca = share_store.create("proj-a", "t:1", now=1000.0)
    b, cb = share_store.create("proj-b", "t:2", now=1000.0)
    _, tok = share_store.redeem(ca, "Pixel", now=1001.0)
    _, tok2 = share_store.redeem(cb, "Pixel", now=1002.0, token=tok)
    assert tok2 == tok
    guest = share_store.lookup_token(tok)
    assert guest.sessions() == {"proj-a", "proj-b"}


def test_redeem_com_token_revogado_gera_token_novo():
    a, ca = share_store.create("proj-a", "t:1", now=1000.0)
    b, cb = share_store.create("proj-b", "t:2", now=1000.0)
    _, tok = share_store.redeem(ca, "Pixel", now=1001.0)
    share_store.revoke(a.id)
    _, tok2 = share_store.redeem(cb, "Pixel", now=1002.0, token=tok)
    assert tok2 != tok
    assert share_store.lookup_token(tok2).sessions() == {"proj-b"}


def test_revogar_um_convite_mantem_a_outra_sessao_do_token():
    a, ca = share_store.create("proj-a", "t:1", now=1000.0)
    b, cb = share_store.create("proj-b", "t:2", now=1000.0)
    _, tok = share_store.redeem(ca, "Pixel", now=1001.0)
    share_store.redeem(cb, "Pixel", now=1002.0, token=tok)
    share_store.revoke(a.id)
    guest = share_store.lookup_token(tok)
    assert guest.sessions() == {"proj-b"}
    assert guest.share_for("proj-a").revoked_at is not None


def test_mesma_sessao_convidada_duas_vezes_sobrevive_a_um_revoke():
    a, ca = share_store.create("proj", "t:1", now=1000.0)
    b, cb = share_store.create("proj", "t:1", now=1000.5)
    _, tok = share_store.redeem(ca, "Pixel", now=1001.0)
    share_store.redeem(cb, "Pixel", now=1002.0, token=tok)
    share_store.revoke(a.id)
    assert share_store.lookup_token(tok).sees("proj")


def test_codigo_de_par_nao_resgata_como_convite_e_vice_versa():
    _, cp = share_store.create("proj", "t:1", now=1000.0, kind="pair")
    _, cs = share_store.create("proj", "t:1", now=1000.0)
    with pytest.raises(ShareError) as e:
        share_store.peek(cp, now=1001.0)
    assert e.value.reason == "unknown"
    with pytest.raises(ShareError):
        share_store.peek(cs, now=1001.0, kind="pair")


def test_attach_liga_as_sessoes_do_outro_token_e_revogar_o_pai_revoga_o_filho():
    pai, tok_par = share_store.create_redeemed("proj-y", "t:9", kind="pair", now=1000.0)
    _, c = share_store.create("proj-z", "t:3", now=1000.0)
    _, tok = share_store.redeem(c, "Pixel", now=1001.0)
    assert share_store.attach(tok, tok_par, now=1002.0) == 1
    guest = share_store.lookup_token(tok)
    assert guest.sessions() == {"proj-y", "proj-z"}
    assert guest.kind_of("proj-y") == "pair" and guest.kind_of("proj-z") == "share"
    assert guest.pair_share() is None  # filho ligado não fala pelo par
    share_store.revoke(pai.id)
    assert share_store.lookup_token(tok).sessions() == {"proj-z"}


def test_attach_exige_os_dois_tokens_validos():
    _, c = share_store.create("proj-z", "t:3", now=1000.0)
    _, tok = share_store.redeem(c, "Pixel", now=1001.0)
    assert share_store.attach(tok, "nao-existe", now=1002.0) == 0
    assert share_store.attach("nao-existe", tok, now=1002.0) == 0


def test_lista_do_dialogo_e_marca_compartilhada_ignoram_par():
    share_store.create("proj", "t:1", now=1000.0, kind="pair")
    assert share_store.list_for("proj", now=1001.0) == []
    assert share_store.active_sessions(now=1001.0) == set()


def test_attach_em_cadeia_aponta_para_a_raiz_e_revogar_a_raiz_derruba_tudo():
    pai, tok_p = share_store.create_redeemed("proj-y", "t:9", kind="pair", now=1000.0)
    _, c1 = share_store.create("proj-z", "t:3", now=1000.0)
    _, tok_t = share_store.redeem(c1, "Pixel", now=1001.0)
    _, c2 = share_store.create("proj-w", "t:4", now=1000.0)
    _, tok_t2 = share_store.redeem(c2, "PC", now=1001.0)
    assert share_store.attach(tok_t, tok_p, now=1002.0) == 1
    assert share_store.attach(tok_t2, tok_t, now=1003.0) == 2
    assert share_store.attach(tok_t2, tok_t, now=1004.0) == 0  # sem duplicar
    copia = share_store.lookup_token(tok_t2).share_for("proj-y")
    assert copia.parent_id == pai.id
    share_store.revoke(pai.id)
    assert share_store.lookup_token(tok_t2).sessions() == {"proj-w", "proj-z"}
    assert share_store.lookup_token(tok_t).sessions() == {"proj-z"}


def test_revoke_session_filtra_por_tipo():
    share_store.create("a", "t:1", now=1000.0)
    par, token = share_store.create_redeemed("a", "t:1", kind="pair", now=1000.0)
    assert share_store.revoke_session("a", kind="share") == 1
    assert share_store.lookup_token(token).share_for("a").revoked_at is None
    assert share_store.revoke_session("a") == 1
    assert share_store.lookup_token(token).share_for("a").revoked_at is not None


def test_sessao_compartilhada_e_pareada_no_mesmo_token_resolve_para_o_convite():
    _, c = share_store.create("proj", "t:1", now=1000.0)
    _, tok = share_store.redeem(c, "Pixel", now=1001.0)
    _, tok_par = share_store.create_redeemed("proj", "t:1", kind="pair", now=1002.0)
    assert share_store.attach(tok, tok_par, now=1003.0) == 1
    guest = share_store.lookup_token(tok)
    # A cópia do par é mais nova, mas o convite de verdade vence.
    assert guest.kind_of("proj") == "share"



def test_session_life_uses_batch_birth_without_subprocess(monkeypatch):
    def forbidden(*args, **kwargs):
        raise AssertionError("a listagem já trouxe o nascimento")
    monkeypatch.setattr(share_life, "_tmux_birth", forbidden)
    monkeypatch.setattr(share_life.headless_sessions, "load", forbidden)
    monkeypatch.setattr(share_life.codex_sessions, "load", forbidden)
    assert share_life.session_life("terminal", meta=None, birth=100) == "t:100"
    assert share_life.session_life("terminal", meta=None, birth=None) is None
    assert share_life.session_life("codex", meta={"key": "preserved"}, birth=100) == "k:preserved"
