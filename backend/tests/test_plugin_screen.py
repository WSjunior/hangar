from app.plugin_screen import band_start, cell_width, cells, crop_cells, find_label, prompt_top


def test_largura_de_celula():
    assert cell_width("a") == 1
    assert cell_width("●") == 1
    assert cell_width("界") == 2
    assert cell_width("́") == 0


def test_corte_em_celulas_tira_a_coluna_do_painel():
    linha = "ok" + " " * 85 + "│" + " Review !577"
    assert crop_cells(linha, 87) == "ok"
    assert crop_cells("界界界", 3) == "界"


def test_prompt_top_e_a_regua_acima_do_cursor():
    tela = ["● resposta", "", "▸ Review !577", "─" * 40, "❯ ", "─" * 40, "  status"]
    assert prompt_top(tela) == 3


def test_inicio_da_faixa_pela_ancora():
    tela = ["● resposta", "", "▸ Review !577", "─" * 40, "❯ "]
    assert band_start(tela, 3, "Review !577") == 2
    assert band_start(tela, 3, None) == 0


def test_rotulo_depois_de_caractere_largo_cai_na_celula_certa():
    tela = ["界 [ copiar link ]"]
    (linha, col), = find_label(tela, "[ copiar link ]", range(1), 0, None)
    assert linha == 0
    assert col == 3 + (cells("[ copiar link ]") - 1) // 2


def test_rotulo_fora_da_regiao_nao_conta():
    tela = ["fechar" + " " * 81 + "│ fechar"]
    assert find_label(tela, "fechar", range(1), 87, None) == [(0, 89 + 2)]
