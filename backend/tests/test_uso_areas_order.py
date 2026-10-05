"""Prende a ordem das áreas por ferramenta às três sementes de hash."""

import json
import os
import subprocess
import sys
from datetime import datetime

import pytest

from app import costs, costs_cache as cc, uso_areas
from app.uso_claude import UsoLinha


class SavedAreasFold:
    def __init__(self, cwd):
        self.cwd = cwd

    def linha(self, raw):
        pass

    def fechar(self):
        regs = [("P", self.cwd, self.cwd, ("a.css", "a.py"))]
        units = [("d", self.cwd, "m", False, 11, 7, 5, 3, 2)]
        tool = UsoLinha(dia="d", cwd=self.cwd, model="m", tipo="tool", nome="Read", chamadas=1)
        cost = costs.UsageRow(ts=datetime.fromisoformat("2026-09-30T12:00:00-03:00"),
                             source="synthetic", provider="p", model="m", project=self.cwd,
                             session_id="s", input=11, output=7, cache_write=5, cache_read=3,
                             cache_write_1h=2)
        return [cost], [tool], ({}, [(regs, units)])


@pytest.mark.parametrize("hash_seed", ["0", "1", "42"])
def test_areas_per_tool_have_common_name_order(tmp_path, hash_seed):
    program = r"""
import json
import sys
from app import uso_areas
from app.uso_claude import linhas_de_area

cwd = sys.argv[1]
uso_areas._mapa = lambda: ("synthetic", uso_areas.PADRAO, {})
regs = [
    ("S", cwd, "skill:database"),
    ("P", cwd, cwd, ("a.css", "a.py", "b.css")),
    ("C", cwd, cwd, ("a.md", "a.py", "/outside/file.py")),
]
counts = uso_areas.contar_areas(regs)
rows = linhas_de_area(({}, [(regs, [("d", cwd, "m", False, 11, 7, 5, 3, 2)])]))
print(json.dumps({
    "counts": list(counts.items()),
    "rows": [[r.nome, r.chamadas, r.input, r.output, r.cache_write,
              r.cache_read, r.cache_write_1h] for r in rows],
}))
"""
    result = subprocess.run(
        [sys.executable, "-c", program, str(tmp_path)],
        check=True, capture_output=True, text=True,
        env={**os.environ, "PYTHONHASHSEED": hash_seed},
    )
    assert json.loads(result.stdout) == {
        "counts": [["banco", 1], ["back", 2], ["front", 1], ["docs", 1]],
        "rows": [
            ["banco", 1, 2, 2, 1, 1, 1],
            ["back", 2, 5, 3, 2, 1, 1],
            ["front", 1, 2, 1, 1, 0, 0],
            ["docs", 1, 2, 1, 1, 1, 0],
        ],
    }


def test_division_signature_refolds_only_saved_areas(tmp_path, monkeypatch):
    monkeypatch.setattr(cc, "_CACHE_DIR", tmp_path / "cache")
    monkeypatch.setattr(uso_areas, "_arquivo", lambda: tmp_path / "areas.json")
    current_version = uso_areas._DIVISAO
    transcript = tmp_path / "synthetic.jsonl"
    transcript.write_bytes(b"synthetic\n")
    monkeypatch.setattr(uso_areas, "_DIVISAO", 2)
    uso_areas.recarregar()
    try:
        old_signature = uso_areas.assinatura()
        cc.sincronizar("synthetic", [transcript], lambda p: SavedAreasFold(str(tmp_path)), "v1")
        conn = cc._abrir()
        try:
            metadata = conn.execute("SELECT versao,dev,ino,size,mtime_ns,offset,cauda,estado,areas FROM files").fetchall()
            other_usage = conn.execute("SELECT * FROM uso WHERE tipo<>'area' ORDER BY rowid").fetchall()
            costs_before = conn.execute("SELECT * FROM custo ORDER BY rowid").fetchall()
            assert costs_before and other_usage
        finally:
            conn.close()
        monkeypatch.setattr(uso_areas, "_DIVISAO", current_version)
        uso_areas.recarregar()
        new_signature = uso_areas.assinatura()
        assert new_signature != old_signature

        reads = []

        def forbid_read(*args):
            reads.append(args)
            raise AssertionError("a troca de assinatura não pode reler o transcript")

        monkeypatch.setattr(cc, "_ler_novo", forbid_read)
        cc.sincronizar("synthetic", [transcript], forbid_read, "v1")
        assert reads == []
        conn = cc._abrir()
        try:
            assert conn.execute("SELECT versao,dev,ino,size,mtime_ns,offset,cauda,estado,areas FROM files").fetchall() == metadata
            assert conn.execute("SELECT * FROM uso WHERE tipo<>'area' ORDER BY rowid").fetchall() == other_usage
            assert conn.execute("SELECT * FROM custo ORDER BY rowid").fetchall() == costs_before
            assert conn.execute("SELECT areas_sig FROM files").fetchone() == (new_signature,)
            assert conn.execute("SELECT nome,input,output,cache_write,cache_read,cache_write_1h FROM uso WHERE tipo='area' ORDER BY rowid").fetchall() == [
                ("back", 6, 4, 3, 2, 1), ("front", 5, 3, 2, 1, 1),
            ]
        finally:
            conn.close()
    finally:
        uso_areas.recarregar()
