from types import SimpleNamespace
import app.doctor as doctor


def _settings(**kw):
    base = dict(auth_token="segredo-forte", port=8765, public_url="", lan_bind_ip="0.0.0.0")
    base.update(kw)
    return SimpleNamespace(**base)


def _tudo_ok(monkeypatch):
    monkeypatch.setattr(doctor, "_porta_responde", lambda porta: True)
    monkeypatch.setattr(doctor, "_binario", lambda nome: "/usr/bin/x")
    monkeypatch.setattr(doctor, "_claude_logado", lambda: True)
    monkeypatch.setattr(doctor, "_tailscale", lambda: ("instalado", "logado"))
    monkeypatch.setattr(doctor, "_lan_responde", lambda s: True)


def test_token_de_fabrica_e_erro(monkeypatch):
    _tudo_ok(monkeypatch)
    linhas = doctor.diagnosticar(_settings(auth_token="change-me"))
    erro = [l for l in linhas if l.nivel == "erro"]
    assert erro and "token" in erro[0].titulo.lower()
    assert "backend/.env" in erro[0].conserto


def test_tudo_ok_sai_zero(monkeypatch, capsys):
    _tudo_ok(monkeypatch)
    monkeypatch.setattr(doctor, "settings", _settings(public_url="https://pc.tail.ts.net"))
    assert doctor.main([]) == 0
    saida = capsys.readouterr().out
    assert "X   " not in saida and "ok  " in saida


def test_backend_fora_do_ar_e_erro_com_conserto_do_so(monkeypatch):
    _tudo_ok(monkeypatch)
    monkeypatch.setattr(doctor, "_porta_responde", lambda porta: False)
    linhas = doctor.diagnosticar(_settings())
    porta = next(l for l in linhas if "8765" in l.titulo)
    assert porta.nivel == "erro"
    assert ("systemctl --user restart hangar-backend" in porta.conserto
            or "Start-ScheduledTask" in porta.conserto)


def test_sem_tailscale_e_aviso_nao_erro(monkeypatch):
    _tudo_ok(monkeypatch)
    monkeypatch.setattr(doctor, "_tailscale", lambda: ("ausente", ""))
    linhas = doctor.diagnosticar(_settings())
    ts = next(l for l in linhas if "Tailscale" in l.titulo)
    assert ts.nivel == "aviso"


def test_lan_responde_le_o_shape_real_do_alcance(monkeypatch):
    # shape real de levantar_estados: `tipo` + `estado` ("ok"/"falhou"); NÃO existe chave `ok`.
    import app.alcance as alcance
    monkeypatch.setattr(alcance, "levantar_estados", lambda s: {"enderecos": [
        {"tipo": "nesta_maquina", "url": "http://127.0.0.1:8765", "estado": "ok"},
        {"tipo": "rede_local", "url": "http://10.0.0.5:8765", "estado": "ok"},
    ]})
    assert doctor._lan_responde(_settings()) is True
    monkeypatch.setattr(alcance, "levantar_estados", lambda s: {"enderecos": [
        {"tipo": "rede_local", "url": "http://10.0.0.5:8765", "estado": "falhou", "motivo": "timeout"},
    ]})
    assert doctor._lan_responde(_settings()) is False


def test_claude_logado_usa_loggedIn(monkeypatch):
    import app.conta_estado as ce
    monkeypatch.setattr(ce, "listar_contas", lambda: [
        SimpleNamespace(login=SimpleNamespace(estado="ok", loggedIn=False)),
    ])
    assert doctor._claude_logado() is False
    monkeypatch.setattr(ce, "listar_contas", lambda: [
        SimpleNamespace(login=SimpleNamespace(estado="ok", loggedIn=True)),
    ])
    assert doctor._claude_logado() is True
