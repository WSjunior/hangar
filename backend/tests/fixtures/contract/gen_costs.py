"""Golden de custo, uso, preços e relatórios sobre transcripts inteiramente sintéticos.

Uso, de backend/: uv run python tests/fixtures/contract/gen_costs.py [pasta-de-saída]
"""
import json
import shutil
import sys
import tempfile
from datetime import datetime
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
FIX = HERE / "costs"
sys.path.insert(0, str(HERE.parents[2]))

from app import costs  # noqa: E402
from app import costs_cache, costs_claude_transcript, costs_sources, pricing, uso_areas, uso_claude, uso_report  # noqa: E402

NOW = datetime.fromisoformat("2026-10-01T12:00:00-03:00")
ORIGINS = {"brainstorming": "superpowers", "minha-skill": "@pessoal"}
MODELS = ["claude-opus-5", "anthropic/claude-opus-5", "Claude-Opus-5", "claude-haiku-4.5", "k3", "apikey/k3",
          "openrouter/deepseek/deepseek-v4-flash", "gpt-5.6-sol", "gpt-5.5", "modelo-caro", "<synthetic>", "", "nao-existe"]


def _rate(rate):
    return None if rate is None else {key: getattr(rate, key) for key in
                                     ("input", "output", "cache_read", "cache_write", "provider", "origin", "cache_estimado")}


def _scopes(base: Path):
    return {"claude": [(base / "claude" / "projects", "anthropic:u-fixture")],
            "codex": [("codex:" + str(base / "codex"), base / "codex")],
            "pi": [(base / "pi", "pi")], "kimi": base / "kimi" / "sessions"}


def _sync(scopes) -> None:
    for root, _account in scopes["claude"]:
        costs_claude_transcript.sincronizar(root)
    for identity, home in scopes["codex"]:
        files = sorted(costs_cache.listar(home / "sessions", lambda name: name.startswith("rollout-") and name.endswith(".jsonl")))
        costs_cache.sincronizar(identity, files, costs_sources._dobra_codex,
                               f"codex:{costs_sources.CACHE_VERSAO}:{costs_sources._USO_CODEX_VERSAO}")
    for root, source in scopes["pi"]:
        costs_sources._sincronizar_pi(root, source)
    costs_sources._sincronizar_kimi(scopes["kimi"])


def _dump(base: Path) -> dict:
    connection = costs_cache._abrir()
    try:
        result = {}
        for file_id, path in connection.execute("SELECT id, path FROM files ORDER BY path"):
            relative = Path(path).relative_to(base).as_posix()
            cost = [list(row) for row in connection.execute(
                f"SELECT {', '.join(costs_cache.CAMPOS_CUSTO)} FROM custo WHERE file_id=? ORDER BY rowid", (file_id,))]
            usage = [list(row) for row in connection.execute(
                f"SELECT {', '.join(costs_cache.CAMPOS_USO)} FROM uso WHERE file_id=? AND tipo<>'area' ORDER BY rowid", (file_id,))]
            areas = [list(row) for row in connection.execute(
                f"SELECT {', '.join(costs_cache.CAMPOS_USO)} FROM uso WHERE file_id=? AND tipo='area' ORDER BY rowid", (file_id,))]
            result[relative] = {"custo": cost, "uso": usage, "areas": areas}
        return result
    finally:
        connection.close()


def _copy_halves(destination: Path) -> list[tuple[Path, bytes]]:
    """Corta na fronteira de linha para exercitar a retomada sem reescrever o prefixo."""
    tails = []
    for path in sorted(destination.rglob("*.jsonl")):
        if path.name == "session_index.jsonl":
            continue
        data = path.read_bytes()
        split = data.find(b"\n", len(data) // 2) + 1 or len(data)
        path.write_bytes(data[:split])
        tails.append((path, data[split:]))
    return tails


def _reports() -> dict:
    def cost_report(period):
        days = costs.PERIODOS.get(period)
        since = (NOW.date().fromordinal(NOW.date().toordinal() - (days * 2 - 1))).isoformat() if days else None
        return costs.montar(costs_sources._ler_custos(since), period=period, now=NOW).model_dump(mode="json")

    def usage_report(**filters):
        return uso_report.montar(*costs_sources._ler_uso(None), period="all", now=NOW,
                                 origens=ORIGINS, **filters).model_dump(mode="json")

    return {"now": NOW.isoformat(), "costs": {"all": cost_report("all"), "7d": cost_report("7d")},
            "uso": {"all": usage_report(), "conta": usage_report(conta=["anthropic:u-fixture"]),
                    "projeto": usage_report(projeto=["/repo/a"]), "foco_skill": usage_report(foco="brainstorming"),
                    "foco_area": usage_report(foco="back")}}


def _areas() -> dict:
    commands = ["cd x && npm run build", "FOO=$(git rev-parse) make", "sudo -E rtk pytest -q", "timeout 300 npx vitest",
                "(cd a; ls) | wc", "for f in *.py; do echo $f; done", "./scripts/x.sh --flag", "'/usr/bin/env' node a.js"]
    paths = ["/h/.claude/skills/minha/SKILL.md", "/h/.claude/skills/minha/references/a.md",
             "/h/.claude/plugins/cache/mkt/superpowers/6.4/skills/brainstorming/SKILL.md",
             "/h/.claude/plugins/marketplaces/pmedico-marketplace/skills/x/SKILL.md", "/tmp/a.md", "/h/skills/x.txt"]
    targets = ["backend/app/x.py", "frontend/src/a.svelte", "docs/a.md", "migrations/001.sql", "a/b/Dockerfile.dev",
               "skill:database-x", "README", "x.ps1"]
    return {
        "comando_bash": [[command, uso_claude.comando_bash(command)] for command in commands],
        "candidatos": [[command, list(uso_areas.candidatos_do_comando(command))] for command in commands],
        "skill_do_caminho": [[path, list(found) if (found := uso_claude.skill_do_caminho(path)) else None] for path in paths],
        "repartir": [[value, weights, uso_areas.repartir(value, weights)] for value, weights in
                     ((10, {"front": 1, "back": 2}), (7, {"a": 1, "b": 1, "c": 1}), (0, {"x": 3}), (101, {"z": 2, "y": 2}))],
        "area_do_alvo": [[target, uso_areas.area_do_alvo(target, uso_areas.PADRAO)] for target in targets],
    }


def _write(path: Path, value) -> None:
    path.write_text(json.dumps(value, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")


def main(output: Path | None = None) -> None:
    golden = output if output is not None else HERE / "golden"
    golden.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary, \
            patch.object(pricing, "_CACHE_DIR", FIX / "pricing"), \
            patch.object(uso_areas, "_arquivo", lambda: Path(temporary) / "sem-mapa.json"), \
            patch.object(costs_sources.kimi_sessions, "kimi_home", lambda: Path(temporary) / "inteiro" / "kimi"):
        pricing.invalidar_cache()
        uso_areas.recarregar()
        _write(golden / "costs_pricing.json", [{
            "model": model, "canon": pricing.canonizar(model), "rate": _rate(rate := pricing.rate_for(model)),
            "fast": _rate(pricing.rate_fast(rate, model)) if rate else None,
            "codex_long": _rate(pricing.rate_codex(rate, model, True)) if rate else None} for model in MODELS])
        _write(golden / "costs_areas.json", _areas())
        index = {}
        for name, resumed in (("inteiro", False), ("__resumed__", True)):
            base = Path(temporary) / name
            shutil.copytree(FIX, base)
            with patch.object(costs_cache, "_CACHE_DIR", Path(temporary) / f"idx-{name}"):
                scopes = _scopes(base)
                if resumed:
                    tails = _copy_halves(base)
                    _sync(scopes)
                    for path, tail in tails:
                        with path.open("ab") as stream:
                            stream.write(tail)
                _sync(scopes)
                rows = _dump(base)
                if not resumed:
                    with patch.object(costs_sources, "_escopos", {
                        "claude": scopes["claude"], "codex": [identity for identity, _home in scopes["codex"]],
                        "pi": scopes["pi"], "kimi": scopes["kimi"]}), \
                            patch.object(costs_sources, "_ROTULOS", {
                                "anthropic:u-fixture": "fixture@exemplo", scopes["codex"][0][0]: "Codex · default"}), \
                            patch.object(costs, "usd_brl", lambda: None):
                        # A conta Codex contém a raiz temporária; o consumidor repõe a sua cópia.
                        escaped_base = json.dumps(str(base), ensure_ascii=False)[1:-1]
                        reports = json.loads(json.dumps(_reports(), ensure_ascii=False).replace(escaped_base, "__BASE__"))
                index.update(rows if not resumed else {"__resumed__": rows})
        assert index["__resumed__"] == {key: value for key, value in index.items() if key != "__resumed__"}
        _write(golden / "costs_index.json", index)
        _write(golden / "costs_reports.json", reports)


if __name__ == "__main__":
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else None)
