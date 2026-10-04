"""As regras de projeto aceitam barras dos transcripts sem depender do host."""
import hashlib

import pytest

from app import uso_areas


@pytest.mark.parametrize("key", ["C:/acme", r"C:\acme"])
def test_project_paths_keep_order_case_and_component_boundaries(monkeypatch, key):
    monkeypatch.setattr(uso_areas, "_mapa", lambda: ("synthetic", [("generic", ["*.py"])], {
        "acme": [("first", ["backend/*"])], key: [("second", ["*.py"])],
    }))
    for cwd in ["C:/acme", r"C:\acme", "C:/acme/sub", r"C:\acme\sub", r"C:/acme\sub"]:
        assert [name for name, _ in uso_areas.regras_de(cwd)] == ["first", "second", "generic"]
    for cwd in ["C:/acme-other", r"C:\acme-other", "C:/ACME/sub", r"C:\ACME\sub"]:
        assert [name for name, _ in uso_areas.regras_de(cwd)] == ["generic"]


def test_forward_slash_project_path_stays_valid(monkeypatch):
    monkeypatch.setattr(uso_areas, "_mapa", lambda: ("synthetic", [], {
        "/tmp/acme": [("second", ["*.py"])],
    }))
    assert uso_areas.regras_de("/tmp/acme/sub") == [("second", ["*.py"])]
    assert uso_areas.regras_de("/tmp/acme-other") == []


def test_matching_signature_invalidates_saved_areas_with_the_same_map(tmp_path, monkeypatch):
    file = tmp_path / "areas.json"
    text = '{"projetos":{"C:/acme":[["second",["*.py"]]]}}'
    file.write_text(text, encoding="utf-8")
    monkeypatch.setattr(uso_areas, "_arquivo", lambda: file)
    uso_areas.recarregar()
    try:
        legacy = hashlib.sha256((f"divisao:{uso_areas._DIVISAO}" + repr(uso_areas.PADRAO) + text).encode()).hexdigest()[:12]
        assert uso_areas.assinatura() != legacy
        assert uso_areas.regras_de(r"C:\acme\sub")[0] == ("second", ["*.py"])
    finally:
        uso_areas.recarregar()
