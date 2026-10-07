"""O golden de /input e /steer sai das rotas do Python: mudou a rota, regenere."""
import subprocess
import sys
from pathlib import Path

CONTRACT = Path(__file__).parent / "fixtures" / "contract"
NAMES = {"input.json", "steer.json", "control.json"}


def test_session_write_golden_is_current(tmp_path):
    # Processo à parte: o gerador troca funções do `app.api` e não pode vazar para os outros testes.
    code = "import sys; sys.path.insert(0, sys.argv[1]); import gen_golden; gen_golden.write_session_write(__import__('pathlib').Path(sys.argv[2]))"
    subprocess.run([sys.executable, "-c", code, str(CONTRACT), str(tmp_path)], check=True, timeout=600,
                   cwd=Path(__file__).parents[1])
    assert {p.name for p in tmp_path.iterdir()} == NAMES
    for name in sorted(NAMES):
        have = (CONTRACT / "session_write" / name).read_text(encoding="utf-8")
        assert (tmp_path / name).read_text(encoding="utf-8") == have, \
            f"{name} desatualizado: rode write_session_write de tests/fixtures/contract/gen_golden.py"
