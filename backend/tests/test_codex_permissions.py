from pathlib import Path

from app import codex_permissions as perm

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
