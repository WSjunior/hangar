from pathlib import Path
from unittest.mock import patch

import pytest

from app import codex_permissions as perm
from app import model_picker as mp
from app import terminal_input
from app.terminal_input import TerminalInput

FIXTURES = Path(__file__).parent / "fixtures"
PICKER = (FIXTURES / "pane_codex_permissions.txt").read_text(encoding="utf-8")
CONFIRMA = (FIXTURES / "pane_codex_permissions_confirma.txt").read_text(encoding="utf-8")


def test_le_os_tres_modos_com_o_atual_marcado():
    modos = perm.parse_modos(PICKER)
    assert [m["nome"] for m in modos] == ["Ask for approval", "Approve for me", "Full Access"]
    assert perm.modo_atual(PICKER) == "Full Access"
    assert [m["numero"] for m in modos] == [1, 2, 3]


# A descricao quebra em ate 3 linhas continuadas, so com espaco na frente. Se elas casassem como
# linha de modo, a lista teria 5+ entradas e a navegacao por posicao erraria o alvo.
def test_quebra_de_descricao_nao_vira_modo():
    assert len(perm.parse_modos(PICKER)) == 3
    assert all(m["desc"] for m in perm.parse_modos(PICKER))


def test_cursor_abre_sobre_o_modo_atual():
    modos = perm.parse_modos(PICKER)
    assert [m["nome"] for m in modos if m["cursor"]] == ["Full Access"]


# Contar POSICAO, nao linha de tela: "Full Access" esta 5 linhas abaixo de "Ask for approval" no
# pane (as descricoes ocupam o resto), e sao 2 toques de seta.
def test_passos_sao_por_posicao_e_nao_por_linha():
    assert perm.passos_ate(PICKER, "Ask for approval") == ("Up", 2)
    assert perm.passos_ate(PICKER, "Approve for me") == ("Up", 1)
    assert perm.passos_ate(PICKER, "Full Access") == ("Up", 0)
    assert perm.passos_ate(PICKER, "Modo Que Nao Existe") is None


# O picker e overlay e nao vai pro scrollback: com ele fechado nao ha o que parsear. Sem esta
# guarda, uma lista numerada do proprio chat viraria "o picker esta aberto".
def test_pane_sem_picker_nao_tem_modo_nenhum():
    assert perm.parse_modos("› ola\n  1. isso nao e o picker   descricao\n") == []
    assert not perm.picker_aberto("qualquer coisa")
    assert perm.modo_atual("qualquer coisa") is None


# Ler no instante em que o titulo aparece devolve a lista pela metade; o rodape prova que ela
# terminou de ser pintada.
def test_desenhado_exige_o_rodape():
    meio = PICKER.split("Press enter to confirm")[0]
    assert perm.picker_aberto(meio) and not perm.picker_desenhado(meio)
    assert perm.picker_desenhado(PICKER)


# Escolher Full Access dispara um SEGUNDO dialogo que os outros dois modos nao disparam. Sem
# reconhece-lo, o driver daria a troca por feita com a confirmacao ainda na tela.
def test_reconhece_a_confirmacao_de_full_access():
    assert perm.confirmacao_de_full_access(CONFIRMA)
    assert not perm.confirmacao_de_full_access(PICKER)
    assert not perm.picker_desenhado(CONFIRMA)


# ── driver (IO mockado) ──────────────────────────────────────────────────────
PANE_IDLE = "assistant: pronto\n❯ \n"


def _driver(panes: list[str], prazo: float = 0.0):
    """Patches do IO pro driver: telas em sequencia, teclas espionadas, sem espera real.

    A ultima tela REPETE quando a sequencia acaba: as sondas leem em laco ate o prazo, e com
    `side_effect` seco o teste morreria de StopIteration por causa do numero de voltas, que nao e o
    que ele quer medir. `prazo` e o `_OPEN_PRAZO` da instancia — 0 faz cada sonda dar uma volta so.
    """
    restantes = list(panes)

    def proxima(*_a, **_k):
        return restantes.pop(0) if len(restantes) > 1 else restantes[0]

    return (
        patch.object(terminal_input.tmux, "capture_pane", side_effect=proxima),
        patch.object(terminal_input, "send_keys"),
        patch.object(terminal_input.tmux, "has_session", return_value=True),
        patch.object(terminal_input.time, "sleep"),
        patch.object(TerminalInput, "_OPEN_PRAZO", prazo),
    )


# O 2o Enter do `_abrir_picker_permissoes` sai quando a leitura diz que o picker nao abriu — e uma
# tela com a confirmacao do Full Access pendurada diz exatamente isso. Sem esta recusa, esse Enter
# cai no "Yes, continue anyway" dela.
def test_confirmacao_pendurada_recusa_em_vez_de_teclar():
    # telas: guard, espera do picker (nao abriu), e a leitura que decide o 2o Enter.
    cap, sk, hs, sl, pz = _driver([PANE_IDLE, PANE_IDLE, CONFIRMA])
    with cap, sk as teclas, hs, sl, pz, pytest.raises(mp.PickerError) as exc:
        TerminalInput().list_codex_permissions("cx")
    assert exc.value.status == 409
    # Digitou o comando e o 1o Enter; ao ver o dialogo parou — sem 2o Enter e sem Esc, que seriam
    # respostas a um dialogo que nao e nosso.
    assert [c.args[1] for c in teclas.call_args_list] == ["/permissions", "Enter"]


# O dialogo do Full Access e desenhado tarde em maquina carregada. Com uma foto unica o driver nao
# o via, seguia pra releitura e digitava `/permissions` POR CIMA dele — o Enter caia no "Yes,
# continue anyway" e o Full Access era aplicado enquanto a rota devolvia 409.
def test_espera_a_confirmacao_de_full_access_que_demora():
    # 1a leitura: ainda o picker fechando. 2a: o dialogo chegou.
    cap, sk, hs, sl, pz = _driver([PICKER, CONFIRMA], prazo=1.0)
    with cap, sk, hs, sl, pz:
        assert TerminalInput()._espera_confirmacao_full("cx", "Full Access") is True


# Modo comum: o Codex nao abre dialogo nenhum. A sonda tem que sair na primeira leitura, senao toda
# troca pagaria o prazo inteiro de espera por um dialogo que nunca vem.
def test_modo_comum_nao_espera_dialogo():
    # Prazo LARGO de propósito: o teste prova que a saída é pelo modo, não por o prazo ter acabado.
    cap, sk, hs, sl, pz = _driver([PANE_IDLE], prazo=30.0)
    with cap, sk, hs, sl, pz:
        assert TerminalInput()._espera_confirmacao_full("cx", "Approve for me") is False
