"""As entradas e saídas gravadas da lista que o Rust repete saem do código atual: mudou a regra, regenere."""
import subprocess
import sys
from pathlib import Path

import pytest

CONTRACT = Path(__file__).parent / "fixtures" / "contract"
NAMES = {"list_discovery.json", "list_decorate.json", "list_state.json"}


@pytest.mark.skipif(sys.platform != "linux", reason="golden gerado e conferido só no Linux (fd aberto, /proc)")
def test_list_golden_is_current(tmp_path):
    subprocess.run([sys.executable, str(CONTRACT / "gen_list.py"), str(tmp_path)], check=True, timeout=600)
    assert {p.name for p in tmp_path.iterdir()} == NAMES
    for name in sorted(NAMES):
        have = (CONTRACT / "golden" / name).read_text(encoding="utf-8")
        assert (tmp_path / name).read_text(encoding="utf-8") == have, \
            f"{name} desatualizado: rode uv run python tests/fixtures/contract/gen_list.py"
