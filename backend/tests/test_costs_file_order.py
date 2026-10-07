"""A soma conserva a ordem do arquivo sem depender da descoberta ou dos commits do índice."""

import json
import subprocess
import sys
from pathlib import Path

import pytest

CONTRACT = Path(__file__).parent / "fixtures" / "contract"


def test_reports_are_identical_with_reversed_filesystem_discovery(tmp_path):
    script = """
import runpy, sys
from pathlib import Path
from unittest.mock import patch
module = runpy.run_path(sys.argv[1])
from app import costs_cache
original = costs_cache.listar
module['main'](Path(sys.argv[2]))
with patch.object(costs_cache, 'listar', lambda *args: list(reversed(original(*args)))):
    module['main'](Path(sys.argv[3]))
"""
    normal, reversed_order = tmp_path / "normal", tmp_path / "reversed"
    subprocess.run([sys.executable, "-c", script, str(CONTRACT / "gen_costs.py"),
                    str(normal), str(reversed_order)], check=True)
    for name in ("costs_index.json", "costs_reports.json"):
        assert json.loads((normal / name).read_text()) == json.loads((reversed_order / name).read_text()), name


@pytest.mark.parametrize("initial_order", [("a", "b"), ("b", "a")])
def test_existing_index_reads_by_path_after_insertion_and_update(tmp_path, monkeypatch, initial_order):
    from app import costs, costs_cache  # noqa: F401

    monkeypatch.setattr(costs_cache, "_CACHE_DIR", tmp_path / "index")
    connection = costs_cache._abrir()
    try:
        for name in initial_order:
            connection.execute("INSERT INTO files(path,scope,versao) VALUES(?, 'scope', 'old')", (f"/synthetic/{name}.jsonl",))
        ids = dict(connection.execute("SELECT substr(path,12,1), id FROM files"))

        def insert(name):
            values = [12, 11] if name == "a" else [22, 21]
            for value in values:
                connection.execute("INSERT INTO custo(file_id,input) VALUES(?,?)", (ids[name], value))
                connection.execute("INSERT INTO uso(file_id,input) VALUES(?,?)", (ids[name], value))

        for name in initial_order:
            insert(name)

        def assert_order():
            column = costs_cache.CAMPOS_CUSTO.index("input")
            assert [row[column] for row in costs_cache.ler_custos("scope")] == [12, 11, 22, 21]
            column = costs_cache.CAMPOS_USO.index("input")
            assert [row[column] for row in costs_cache.iter_usage_rows("scope", "account")] == [12, 11, 22, 21]

        assert_order()
        for table in ("custo", "uso"):
            connection.execute(f"DELETE FROM {table} WHERE file_id=?", (ids["a"],))
        insert("a")
        assert_order()
    finally:
        connection.close()
