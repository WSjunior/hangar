# Parte 3 — custos e uso em Rust: plano de implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task, com um revisor independente por Task e uma revisão final da branch. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `/api/costs`, `/api/uso`, `/api/cotacao` e `/api/sessions/{name}/cost` atendidos pelo `hangar-server` com índice SQLite próprio, mesmos números e chaves do Python, e o Python como reserva.

**Architecture:** O Python entrega as contas por `GET /internal/costs/scopes` (contrato versão 5) e deixa de varrer no boot quando o Rust v5 está de pé. O Rust porta os leitores (Claude, Codex, Pi/omp, Kimi), o índice incremental (`custos-rust.sqlite3`), os dois relatórios, o preço e a cotação. Toda falha do lado Rust vira repasse ao Python. A paridade é provada por golden gerado pelos leitores Python a partir de transcripts sintéticos.

**Tech Stack:** Rust 1.98.1 (axum, tokio, serde_json com `preserve_order`, `rusqlite` com `bundled`, `rayon`, `flate2`, `indexmap`, `regex`, `chrono`), Python 3.14 / FastAPI.

**Spec:** [`spec.md`](spec.md) (aprovada pelo dono em 03/10/2026, opção A: Pi/omp/Kimi entram). Medições: [`analise.md`](analise.md).

**Como este plano entrega o código:** o que é contrato, esquema, encaixe e teste vem completo aqui. Os corpos dos leitores e relatórios são **port linha a linha** das faixas Python citadas em cada Task (o Python é a referência, não este texto); cada Task lista as regras que não podem mudar na tradução e o golden que decide. Antes de portar, leia a faixa inteira.

## Global Constraints

- Branch `hangar-server-parte3`, base `00be339c`. Contrato interno **versão 5**: `RUST_SERVER_PROTOCOL = 5` (`backend/app/rust_server.py`) e `INTERNAL_PROTOCOL = 5` (`crates/hangar-server/src/lib.rs`) no **mesmo commit**. Depende da 2B (versão 4) entrar antes; na integração, se a 2B tiver outro número, esta vira o seguinte.
- Nunca subir, reiniciar ou parar o backend nem o `hangar-backend.service`; nunca um segundo backend; nunca instalador. Medições contra arquivos reais só em processo avulso com índice numa pasta descartável.
- Nunca tocar `~/.claude/.hangar-custos/custos.sqlite3` (do Python) nem `custos-rust.sqlite3` real em teste: todo teste recebe a pasta do índice por parâmetro.
- Versões fixadas com `=` no `crates/Cargo.toml`, como as existentes. Dependências novas: `rusqlite` (feature `bundled`), `rayon`, `indexmap` (a do `Cargo.lock`, `=2.14.2`).
- Identificadores novos em inglês; comentários curtos em português, só o porquê. Texto de conversa, caminho de transcript com conteúdo e mensagem de erro do serde **nunca** no log: só código do motivo, contagens e arquivo:linha.
- Testes focados por Task (o kick-off da execução autoriza); suíte inteira só se o dono pedir. Comandos: `cargo test --manifest-path crates/Cargo.toml -p hangar-server --test <arquivo>`; `(cd backend && uv run pytest tests/<arquivo> -q)`.
- Commits por Task com `git add` de caminhos explícitos e mensagem descritiva em inglês. Sem push sem autorização do dono.
- Respostas JSON com os mesmos campos, na mesma ordem dos modelos pydantic (`backend/app/models.py:375-575`), `null` onde o Python manda `None`.
- Chaves que a web soma entre servidores (`anthropic:<uuid>`, `codex:<home>`, dia `YYYY-MM-DD`, modelo canônico, `session_ids`) saem byte a byte iguais.
- Fuso: `LOCAL = UTC-3` fixo, como `costs_sources.LOCAL`. Timestamp sem fuso é lido em UTC-3 (as CLIs sempre mandam `Z`; o Python usaria o fuso da máquina, que aqui é o mesmo).
- Ordem de iteração = ordem de inserção do Python (`dict`): use `IndexMap`/`Vec`, nunca `HashMap`, onde a ordem chega a uma soma de ponto flutuante ou à saída.

## Review Focus

1. **Linha com surrogate solto (`\ud800`) ou byte UTF-8 inválido:** o Python decodifica com `replace` e o `json.loads` aceita surrogate; o Rust não pode pular a linha nem contar caracteres diferente. Teste na Task 6.
2. **Arquivo crescendo durante a leitura (última linha sem `\n`) e arquivo trocado no mesmo caminho:** nada contado duas vezes, nada perdido. Teste na Task 4.
3. **Rollout do Codex alcançável por duas contas (link):** fica com uma só ou com nenhuma, nunca as duas. Teste na Task 9.
4. **Python sem a rota de escopos (versão velha) ou fora do ar no meio:** o Rust repassa, nunca responde vazio como se fosse "sem gasto". Teste na Task 9.
5. **Dois aparelhos pedindo `fresco=1` juntos com o índice frio:** uma varredura só, os dois recebem 202 com progresso e depois o mesmo relatório. Teste na Task 9.

---

### Task 1: Contrato versão 5 — escopos no Python, aquecimento só sem o Rust

**Files:**
- Modify: `backend/app/costs_sources.py` (extrair os escopos de `_sincronizar`, marca "servido pelo Rust", alvo do timer de boot)
- Modify: `backend/app/internal_api.py` (rota `/internal/costs/scopes`)
- Modify: `backend/app/rust_server.py:33` (versão 5), `Supervisor.run` (marca), `_take_over` (aquecimento)
- Modify: `crates/hangar-server/src/lib.rs:15`, `crates/hangar-server/tests/proxy.rs:30`, `crates/hangar-server/tests/terminal_routes.rs:214`
- Test: `backend/tests/test_internal_costs.py` (novo), `backend/tests/test_rust_server.py`

**Interfaces:**
- Produces: `costs_sources.scopes_for_rust() -> dict` (formato da spec, seção 2); `costs_sources.set_served_by_rust(on: bool)`; rota `GET /internal/costs/scopes`; `INTERNAL_PROTOCOL == 5`.

- [x] **Step 1: Testes que falham**

`backend/tests/test_internal_costs.py`:

```python
"""Escopos de custos para o hangar-server (contrato versão 5)."""
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from app import costs_sources, internal_api
from app.api import app
from app.config import ConfigDirInfo


@pytest.fixture
def client(monkeypatch):
    internal_api.set_secret("s3")
    yield TestClient(app, client=("127.0.0.1", 5000))
    internal_api.set_secret(None)


def _fake_roots(tmp_path, monkeypatch, *, omp_igual_pi=False):
    cfg = tmp_path / "cfg"
    (cfg / "projects").mkdir(parents=True)
    (cfg / ".claude.json").write_text('{"oauthAccount": {"accountUuid": "u1", "emailAddress": "a@b"}}',
                                      encoding="utf-8")
    monkeypatch.setattr(costs_sources, "list_config_dirs",
                        lambda: [ConfigDirInfo(path=str(cfg), label="padrão", active=True)])
    home = tmp_path / "codex"
    (home / "sessions").mkdir(parents=True)
    from app import codex_contas
    monkeypatch.setattr(costs_sources, "_contas_codex",
                        lambda: [codex_contas.Account("default", home, True)])
    pi = tmp_path / "pi"
    pi.mkdir()
    monkeypatch.setattr(costs_sources, "raiz_pi", lambda: pi)
    monkeypatch.setattr(costs_sources, "raiz_omp", lambda: pi if omp_igual_pi else tmp_path / "omp-sem")
    monkeypatch.setattr(costs_sources, "raiz_kimi", lambda: tmp_path / "kimi-sem")
    return cfg, home, pi


def test_scopes_carry_accounts_roots_and_repo(client, tmp_path, monkeypatch):
    cfg, home, pi = _fake_roots(tmp_path, monkeypatch)
    r = client.get("/internal/costs/scopes", headers={"x-hangar-internal": "s3"})
    assert r.status_code == 200
    body = r.json()
    assert body["claude"] == [{"root": str(cfg / "projects"), "account": "anthropic:u1", "label": "a@b"}]
    identidade = f"codex:{home.resolve()}"
    assert body["codex"] == [{"home": str(home.resolve()), "account": identidade, "label": "Codex · default"}]
    assert body["pi"] == [{"root": str(pi), "source": "pi"}]
    assert body["kimi"] is None
    assert Path(body["repo"], "skills").is_dir()


def test_omp_on_the_pi_root_is_left_out(client, tmp_path, monkeypatch):
    _fake_roots(tmp_path, monkeypatch, omp_igual_pi=True)
    body = client.get("/internal/costs/scopes", headers={"x-hangar-internal": "s3"}).json()
    assert [p["source"] for p in body["pi"]] == ["pi"]


def test_scopes_refuse_without_secret(client):
    assert client.get("/internal/costs/scopes").status_code == 404


def test_boot_warmup_skips_while_rust_serves(monkeypatch):
    chamou = []
    monkeypatch.setattr(costs_sources, "aquecer_em_background", lambda: chamou.append(1))
    costs_sources.set_served_by_rust(True)
    try:
        costs_sources._boot_warmup()
        assert chamou == []
    finally:
        costs_sources.set_served_by_rust(False)
    costs_sources._boot_warmup()
    assert chamou == [1]


def test_request_reaching_python_still_warms_on_demand(monkeypatch):
    """Pedido repassado ao Python com o Rust de pé dispara a varredura dele (202), nunca fica preso."""
    chamou = []
    monkeypatch.setattr(costs_sources, "aquecer_em_background", lambda: chamou.append(1))
    monkeypatch.setattr(costs_sources._aquecido, "is_set", lambda: False)
    costs_sources.set_served_by_rust(True)
    try:
        with pytest.raises(costs_sources.Aquecendo):
            costs_sources.preparar()
    finally:
        costs_sources.set_served_by_rust(False)
    assert chamou == [1]
```

Em `backend/tests/test_rust_server.py`, acrescentar (reaproveita `fake_bin` e `events` do arquivo; o binário falso já responde a saúde com `RUST_SERVER_PROTOCOL`):

```python
def test_rust_up_marks_costs_served_and_takeover_warms_python(fake_bin, tmp_path, events, monkeypatch):
    from app import costs_sources
    marcas, agendas = [], []
    monkeypatch.setattr(costs_sources, "set_served_by_rust", marcas.append)
    monkeypatch.setattr(costs_sources, "agendar_aquecimento", agendas.append)
    # Mesmo roteiro de test_three_crashes_in_a_minute_hand_the_public_port_to_python: o filho cai 3x.
    _run_until_takeover(fake_bin, tmp_path, monkeypatch, crash=True)
    assert True in marcas and marcas[-1] is False
    assert agendas == [0]
```

`_run_until_takeover` é o corpo de `test_three_crashes_in_a_minute_hand_the_public_port_to_python` extraído para função (mesmo arquivo); o teste antigo passa a chamá-la.

- [x] **Step 2: Rodar e ver falhar**

Run: `(cd backend && uv run pytest tests/test_internal_costs.py tests/test_rust_server.py -q)`
Expected: FAIL — `scopes_for_rust`, `set_served_by_rust`, `_boot_warmup` e a rota não existem.

- [x] **Step 3: Implementar no Python**

Em `costs_sources.py`, extrair de `_sincronizar` (linhas 850-889) a parte que calcula escopos, sem mudar o comportamento dele:

```python
_REPO = Path(__file__).resolve().parents[2]
_servido_pelo_rust = False


def set_served_by_rust(on: bool) -> None:
    """Com o hangar-server v5 de pé, o boot não varre: as telas falam com o índice dele."""
    global _servido_pelo_rust
    _servido_pelo_rust = on


def _boot_warmup() -> None:
    # Só o boot pula. Pedido que chega aqui (repasse do Rust) aquece por `_pronto`, como sempre.
    if not _servido_pelo_rust:
        aquecer_em_background()


def _raizes_pi() -> list[tuple[Path, str]]:
    out = []
    for nome, raiz in (("pi", raiz_pi()), ("omp", raiz_omp())):
        if nome == "omp" and raiz == raiz_pi():
            if not _AVISOU_RAIZ_UNICA:
                _AVISOU_RAIZ_UNICA.add(str(raiz))
                _log.warning("custos: omp e pi na mesma raiz (%s) — gasto do omp somado como pi", raiz)
            continue
        if raiz.is_dir():
            out.append((raiz, nome))
    return out


def scopes_for_rust() -> dict:
    """O que `_sincronizar` decide antes de ler arquivo, no formato do contrato versão 5."""
    claude = [{"root": str(costs_claude_transcript.raiz_projetos(Path(caminho))), "account": conta,
               "label": _ROTULOS.get(conta) or conta}
              for caminho, conta in _config_dirs()]
    codex = []
    for conta in _contas_codex():
        home = conta.home.expanduser().absolute().resolve(strict=False)
        identidade = f"codex:{home}"
        _ROTULOS[identidade] = f"Codex · {conta.id}"
        codex.append({"home": str(home), "account": identidade, "label": _ROTULOS[identidade]})
    kimi = raiz_kimi()
    return {"claude": claude, "codex": codex,
            "pi": [{"root": str(r), "source": s} for r, s in _raizes_pi()],
            "kimi": ({"root": str(kimi), "index": str(kimi_sessions.kimi_home() / "session_index.jsonl")}
                     if kimi.is_dir() else None),
            "repo": str(_REPO)}
```

`_sincronizar` passa a usar `_raizes_pi()` no laço do Pi/omp (mesmo resultado). Em `agendar_aquecimento`, o `Timer` chama `_boot_warmup` em vez de `aquecer_em_background`.

Observação para o revisor: `codex` lista **todas** as contas de `list_accounts()`, inclusive as sem rollout; o Python só cria escopo para contas com rollout atribuído, e o Rust faz o mesmo na Task 9 (escopo sem arquivo não gera linha).

Em `internal_api.py`, abaixo de `session_info`:

```python
@router.get("/costs/scopes")
async def costs_scopes() -> dict:
    from app import costs_sources
    return await asyncio.to_thread(costs_sources.scopes_for_rust)
```

Em `rust_server.py`: `RUST_SERVER_PROTOCOL = 5`. Em `Supervisor.run`, depois de `state == "up"` (antes do laço `while state == "up"`) chamar `costs_sources.set_served_by_rust(True)`; logo depois do laço (filho saiu) e em `stop()` chamar `set_served_by_rust(False)` (import tardio de `app.costs_sources`, como o de `internal_api`). Em `_take_over`, depois de `diag.registrar(...)`: `costs_sources.set_served_by_rust(False)` e `costs_sources.agendar_aquecimento(0)`.

- [x] **Step 4: Subir a versão no Rust**

`crates/hangar-server/src/lib.rs`: `pub const INTERNAL_PROTOCOL: u32 = 5;` e o comentário acima ganha "5: rota `/internal/costs/scopes`". Trocar `3` por `5` nos dois `assert_eq!` de teste citados.

- [x] **Step 5: Rodar os testes**

Run: `(cd backend && uv run pytest tests/test_internal_costs.py tests/test_rust_server.py tests/test_costs_sources.py tests/test_costs_cache.py -q)` e `cargo test --manifest-path crates/Cargo.toml -p hangar-server --test proxy --test terminal_routes`
Expected: PASS.

- [x] **Step 6: Revisão e commit**

```bash
git add backend/app/costs_sources.py backend/app/internal_api.py backend/app/rust_server.py \
  backend/tests/test_internal_costs.py backend/tests/test_rust_server.py \
  crates/hangar-server/src/lib.rs crates/hangar-server/tests/proxy.rs crates/hangar-server/tests/terminal_routes.rs
git commit -m "feat(server): internal contract v5 exposes cost scopes and skips Python boot scan behind Rust"
```

---

### Task 2: Transcripts sintéticos e golden dos leitores, do preço e dos relatórios

**Files:**
- Create: `backend/tests/fixtures/contract/costs/` (árvore abaixo), `backend/tests/fixtures/contract/gen_costs.py`
- Create: `backend/tests/fixtures/contract/golden/costs_index.json`, `costs_reports.json`, `costs_pricing.json`, `costs_areas.json`
- Test: `backend/tests/test_costs_golden.py`

**Interfaces:**
- Produces: os quatro golden, lidos pelas Tasks 3–11. Formatos:
  - `costs_index.json`: `{"<caminho relativo a costs/>": {"custo": [[...colunas CAMPOS_CUSTO...]], "uso": [[...colunas CAMPOS_USO...]], "areas": [[...colunas CAMPOS_USO das linhas tipo area...]]}}` — linhas na ordem de `rowid`; `custo` sem a coluna `dia` (ela é derivada de `ts`). Mais a chave `"__resumed__"` com o mesmo mapa obtido lendo cada arquivo em duas metades (Task 4).
  - `costs_pricing.json`: `[{"model": id, "canon": str, "rate": null | {input, output, cache_read, cache_write, provider, origin, cache_estimado}, "fast": idem, "codex_long": idem}]`.
  - `costs_areas.json`: `{"comando_bash": [[cmd, nome]], "candidatos": [[cmd, [..]]], "skill_do_caminho": [[caminho, [nome, e_skill_md] | null]], "repartir": [[valor, pesos, saida]], "area_do_alvo": [[alvo, area | null]]}` (regras = `uso_areas.PADRAO`).
  - `costs_reports.json`: `{"now": iso, "costs": {"all": CostReport, "7d": CostReport}, "uso": {"all": UsoReport, "conta": UsoReport, "projeto": UsoReport, "foco_skill": UsoReport, "foco_area": UsoReport}}`, com `usd_brl: null` e origens de skill fixas.

Árvore de fixtures (transcripts escritos à mão, curtos, sem conversa real; cada arquivo cobre um caso nomeado no nome):

```
costs/
  pricing/models.dev.json            # {"modelos": {...}}: claude-opus-5, claude-sonnet-5, claude-haiku-4-5-20251001,
                                     # gpt-5.6-sol (com cache_write), gpt-5.5 (sem cache_write), kimi-k3 (sem cache)
  pricing/overrides.json             # {"modelo-caro": {"input": 1, "output": 2, "provider": "override"}}
  claude/projects/-repo-a/s1.jsonl              # blocos da mesma resposta (mesmo requestId+id), cache 1h, speed fast,
                                                # skill por barra (<command-name>) e texto isMeta, Read em SKILL.md
                                                # e em references/, Bash com cd &&, imagem PNG (base64 com cabeçalho),
                                                # hook SessionStart com e sem conteúdo, invoked_skills após compactar
  claude/projects/-repo-a/s1/subagents/agent-ab12.jsonl   # subagente; o pai tem tool_use Agent + toolUseResult.agentId=ab12
  claude/projects/-repo-a/s2.jsonl              # compact_boundary, cache expirado (regravado), modelo <synthetic>,
                                                # mcp__srv__tool, Agent com "paralelo" no prompt (pedido)
  claude/projects/-repo-b/s3.jsonl              # surrogate solto em texto, byte inválido, linha "null", linha "[1]",
                                                # linha truncada no fim SEM \n, modelo de motor (gpt-5.6-sol no Claude)
  codex/sessions/2026/09/30/rollout-c1.jsonl    # token_usage_record com response_id repetido, contador legado,
                                                # exec com tools.exec_command/apply_patch/view_image, spawn_agent, compacted
  codex/sessions/2026/09/30/rollout-c2.jsonl    # fork: session_meta do pai depois da própria; turn_context posterior
  codex/sessions/2026/09/30/rollout-c3.jsonl    # entrada > 272000 (contexto longo), subagente (source.subagent)
  pi/--repo--/2026-09-30_s.jsonl                # session, model_change (provider/modelId) e mensagens com usage
  pi/--repo--/2026-09-30_s/t1/run-1/session.jsonl   # subagente do Pi, model "openrouter/deepseek/deepseek-v4-flash"
  kimi/sessions/wd_x/session_k1/agents/main/wire.jsonl     # usage.record com time em ms, modelo "apikey/k3"
  kimi/sessions/wd_x/session_k1/agents/agent-1/wire.jsonl  # subagente
  kimi/session_index.jsonl                      # {"sessionId": "session_k1", "workDir": "/repo/k"}
```

Datas entre `2026-09-24` e `2026-10-01`, e uma linha às `02:30Z` (vira o dia anterior em UTC-3). `now` fixo: `2026-10-01T12:00:00-03:00`.

- [ ] **Step 1: Escrever as fixtures** conforme a árvore. Cada linha JSON com só os campos que os leitores olham (ver `costs_claude_transcript.py:122-163`, `uso_claude.py:318-545`, `costs_sources.py:210-280`, `uso_codex.py:84-170`, `costs_sources.py:400-561`).

- [ ] **Step 2: Escrever o gerador**

`backend/tests/fixtures/contract/gen_costs.py`:

```python
"""Golden dos leitores de custo/uso, do preço e dos relatórios Python, sobre transcripts sintéticos.

Uso, de backend/: uv run python tests/fixtures/contract/gen_costs.py
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
from app import costs  # noqa: E402  (ordem do api.py: evita o import circular)
from app import costs_cache, costs_claude_transcript, costs_sources, pricing, uso_areas, uso_claude, uso_report  # noqa: E402

NOW = datetime.fromisoformat("2026-10-01T12:00:00-03:00")
ORIGENS = {"brainstorming": "superpowers", "minha-skill": "@pessoal"}
MODELOS = ["claude-opus-5", "anthropic/claude-opus-5", "Claude-Opus-5", "claude-haiku-4.5", "k3", "apikey/k3",
           "openrouter/deepseek/deepseek-v4-flash", "gpt-5.6-sol", "gpt-5.5", "modelo-caro", "<synthetic>", "", "nao-existe"]


def _rate(r):
    return None if r is None else {k: getattr(r, k) for k in
                                   ("input", "output", "cache_read", "cache_write", "provider", "origin", "cache_estimado")}


def _scopes(base: Path):
    claude = base / "claude" / "projects"
    return {"claude": [(claude, "anthropic:u-fixture")],
            "codex": [("codex:" + str(base / "codex"), base / "codex")],
            "pi": [(base / "pi", "pi")], "kimi": base / "kimi" / "sessions"}


def _sync(base: Path, s) -> None:
    for raiz, _conta in s["claude"]:
        costs_claude_transcript.sincronizar(raiz)
    for ident, home in s["codex"]:
        arquivos = sorted(costs_cache.listar(home / "sessions", lambda n: n.startswith("rollout-") and n.endswith(".jsonl")))
        costs_cache.sincronizar(ident, arquivos, costs_sources._dobra_codex,
                                f"codex:{costs_sources.CACHE_VERSAO}:{costs_sources._USO_CODEX_VERSAO}")
    for raiz, nome in s["pi"]:
        costs_sources._sincronizar_pi(raiz, nome)
    costs_sources._sincronizar_kimi(s["kimi"])


def _dump(base: Path) -> dict:
    conn = costs_cache._abrir()
    try:
        out = {}
        for fid, path in conn.execute("SELECT id, path FROM files ORDER BY path"):
            rel = Path(path).relative_to(base).as_posix()
            custo = [list(r) for r in conn.execute(
                f"SELECT {', '.join(costs_cache.CAMPOS_CUSTO)} FROM custo WHERE file_id=? ORDER BY rowid", (fid,))]
            uso = [list(r) for r in conn.execute(
                f"SELECT {', '.join(costs_cache.CAMPOS_USO)} FROM uso WHERE file_id=? AND tipo<>'area' ORDER BY rowid", (fid,))]
            area = [list(r) for r in conn.execute(
                f"SELECT {', '.join(costs_cache.CAMPOS_USO)} FROM uso WHERE file_id=? AND tipo='area' ORDER BY rowid", (fid,))]
            out[rel] = {"custo": custo, "uso": uso, "areas": area}
        return out
    finally:
        conn.close()


def _copiar_em_metades(dst: Path) -> list[tuple[Path, bytes]]:
    """Cada arquivo fica só até a metade (em fronteira de linha); devolve o resto para anexar."""
    resto = []
    for p in sorted(dst.rglob("*.jsonl")):
        if p.name == "session_index.jsonl":
            continue
        dados = p.read_bytes()
        corte = dados.find(b"\n", len(dados) // 2) + 1 or len(dados)
        p.write_bytes(dados[:corte])
        resto.append((p, dados[corte:]))
    return resto


def main() -> None:
    golden = HERE / "golden"
    with tempfile.TemporaryDirectory() as tmp, \
            patch.object(pricing, "_CACHE_DIR", FIX / "pricing"), \
            patch.object(uso_areas, "_arquivo", lambda: Path(tmp) / "sem-mapa.json"), \
            patch.object(costs_sources.kimi_sessions, "kimi_home", lambda: Path(tmp) / "inteiro" / "kimi"):
        pricing.invalidar_cache()
        uso_areas.recarregar()
        (golden / "costs_pricing.json").write_text(json.dumps([{
            "model": m, "canon": pricing.canonizar(m), "rate": _rate(pricing.rate_for(m)),
            "fast": _rate(pricing.rate_fast(r, m)) if (r := pricing.rate_for(m)) else None,
            "codex_long": _rate(pricing.rate_codex(r, m, True)) if r else None} for m in MODELOS],
            indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
        regras = uso_areas.PADRAO
        cmds = ["cd x && npm run build", "FOO=$(git rev-parse) make", "sudo -E rtk pytest -q", "timeout 300 npx vitest",
                "(cd a; ls) | wc", "for f in *.py; do echo $f; done", "./scripts/x.sh --flag", "'/usr/bin/env' node a.js"]
        caminhos = ["/h/.claude/skills/minha/SKILL.md", "/h/.claude/skills/minha/references/a.md",
                    "/h/.claude/plugins/cache/mkt/superpowers/6.4/skills/brainstorming/SKILL.md",
                    "/h/.claude/plugins/marketplaces/pmedico-marketplace/skills/x/SKILL.md", "/tmp/a.md", "/h/skills/x.txt"]
        alvos = ["backend/app/x.py", "frontend/src/a.svelte", "docs/a.md", "migrations/001.sql", "a/b/Dockerfile.dev",
                 "skill:database-x", "README", "x.ps1"]
        (golden / "costs_areas.json").write_text(json.dumps({
            "comando_bash": [[c, uso_claude.comando_bash(c)] for c in cmds],
            "candidatos": [[c, list(uso_areas.candidatos_do_comando(c))] for c in cmds],
            "skill_do_caminho": [[c, (lambda r: list(r) if r else None)(uso_claude.skill_do_caminho(c))] for c in caminhos],
            "repartir": [[v, p, uso_areas.repartir(v, p)] for v, p in
                         ((10, {"front": 1, "back": 2}), (7, {"a": 1, "b": 1, "c": 1}), (0, {"x": 3}), (101, {"z": 2, "y": 2}))],
            "area_do_alvo": [[a, uso_areas.area_do_alvo(a, regras)] for a in alvos],
        }, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")

        indice = {}
        for nome, metades in (("inteiro", False), ("__resumed__", True)):
            base = Path(tmp) / nome
            shutil.copytree(FIX, base)
            with patch.object(costs_cache, "_CACHE_DIR", Path(tmp) / f"idx-{nome}"):
                s = _scopes(base)
                if metades:
                    resto = _copiar_em_metades(base)
                    _sync(base, s)
                    for p, cauda in resto:
                        with p.open("ab") as f:
                            f.write(cauda)
                _sync(base, s)
                d = _dump(base)
                if not metades:
                    costs_sources._escopos = {"claude": s["claude"], "codex": [i for i, _h in s["codex"]],
                                              "pi": s["pi"], "kimi": s["kimi"]}
                    costs_sources._ROTULOS.update({"anthropic:u-fixture": "fixture@exemplo",
                                                   s["codex"][0][0]: "Codex · default"})
                    with patch.object(costs, "usd_brl", lambda: None):
                        # A identidade do Codex leva o caminho da cópia: o teste Rust troca de volta.
                        reports = json.loads(json.dumps(_reports(), ensure_ascii=False).replace(str(base), "__BASE__"))
            indice.update(d if not metades else {"__resumed__": d})
        (golden / "costs_index.json").write_text(json.dumps(indice, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
        (golden / "costs_reports.json").write_text(json.dumps(reports, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")


def _reports() -> dict:
    def custos(period):
        dias = costs.PERIODOS.get(period)
        desde = (NOW.date().fromordinal(NOW.date().toordinal() - (dias * 2 - 1))).isoformat() if dias else None
        return costs.montar(costs_sources._ler_custos(desde), period=period, now=NOW).model_dump(mode="json")

    def uso(**filtros):
        return uso_report.montar(*costs_sources._ler_uso(None), period="all", now=NOW,
                                 origens=ORIGENS, **filtros).model_dump(mode="json")

    return {"now": NOW.isoformat(), "costs": {"all": custos("all"), "7d": custos("7d")},
            "uso": {"all": uso(), "conta": uso(conta=["anthropic:u-fixture"]), "projeto": uso(projeto=["/repo/a"]),
                    "foco_skill": uso(foco="brainstorming"), "foco_area": uso(foco="back")}}


if __name__ == "__main__":
    main()
```

O caminho `/repo/a` em `projeto` é o `cwd` usado em `s1.jsonl`/`s2.jsonl`; mantenha o fixture e o filtro iguais.

- [ ] **Step 3: Gerar e conferir à mão**

Run: `(cd backend && uv run python tests/fixtures/contract/gen_costs.py)`
Conferir no JSON: cada caso da árvore aparece (ex.: `s3.jsonl` tem as linhas boas contadas e as ruins puladas; `rollout-c2` não soma o histórico do pai; `__resumed__` igual ao inteiro). Se um caso não aparece, a fixture está errada: corrija a fixture, não o gerador.

- [ ] **Step 4: Teste que prende o golden ao Python atual**

`backend/tests/test_costs_golden.py`:

```python
"""Mudou um leitor Python sem regerar o golden? O Rust passaria a divergir calado."""
import json
import subprocess
import sys
from pathlib import Path

CONTRACT = Path(__file__).parent / "fixtures" / "contract"


def test_costs_golden_matches_current_python(tmp_path):
    golden = CONTRACT / "golden"
    antes = {n: (golden / n).read_text(encoding="utf-8") for n in
             ("costs_index.json", "costs_reports.json", "costs_pricing.json", "costs_areas.json")}
    subprocess.run([sys.executable, str(CONTRACT / "gen_costs.py")], check=True, cwd=CONTRACT.parents[2])
    depois = {n: (golden / n).read_text(encoding="utf-8") for n in antes}
    for n in antes:
        (golden / n).write_text(antes[n], encoding="utf-8")
    for n in antes:
        assert json.loads(depois[n]) == json.loads(antes[n]), f"{n} desatualizado: rode gen_costs.py"
```

- [ ] **Step 5: Rodar**

Run: `(cd backend && uv run pytest tests/test_costs_golden.py -q)` — Expected: PASS.

- [ ] **Step 6: Revisão e commit**

```bash
git add backend/tests/fixtures/contract/costs backend/tests/fixtures/contract/gen_costs.py \
  backend/tests/fixtures/contract/golden/costs_index.json backend/tests/fixtures/contract/golden/costs_reports.json \
  backend/tests/fixtures/contract/golden/costs_pricing.json backend/tests/fixtures/contract/golden/costs_areas.json \
  backend/tests/test_costs_golden.py
git commit -m "test(costs): synthetic transcripts and Python golden for the Rust port"
```

---

### Task 3: Base do módulo `costs` no Rust — tempo, números e preço

**Files:**
- Create: `crates/hangar-server/src/costs/mod.rs`, `costs/py.rs`, `costs/pricing.rs`
- Modify: `crates/hangar-server/src/lib.rs` (`pub mod costs;`), `crates/Cargo.toml` (`indexmap = "=2.14.2"`), `crates/hangar-server/Cargo.toml` (`indexmap.workspace = true`)
- Test: `crates/hangar-server/tests/contract_costs_pricing.rs`

**Interfaces:**
- Produces (em `costs::py`):
  - `pub const LOCAL_OFFSET_S: i32 = -3 * 3600;`
  - `pub struct LocalTs(pub i64)` — microssegundos desde a época; `fn iso(&self) -> String` (formato `datetime.isoformat()` com `-03:00`, microssegundos só quando ≠ 0, com 6 dígitos); `fn day(&self) -> String` (`YYYY-MM-DD` em UTC-3); `fn from_iso(s: &str) -> Option<LocalTs>` (aceita o que `datetime.fromisoformat(s.replace("Z", "+00:00"))` aceita, via `crate::transcript::py::iso_timestamp` quando ele cobre; sem fuso = UTC-3); `fn from_millis_f64(ms: f64) -> LocalTs` (arredonda ao microssegundo, metade para par, como `datetime.fromtimestamp`).
  - `pub fn py_int(v: Option<&Value>) -> i64` — regra de `_int`: ausente/`null`/`false`/`0`/`""` → 0; `true` → 1; inteiro → ele; fracionário finito → truncado; texto com espaços em volta e dígitos com sinal → o número; resto → 0; fora de i64 → satura.
  - `pub fn char_len(s: &str) -> usize` — `len()` do Python (pontos de código; marcador de surrogate do `transcript::py` conta 1).
  - `pub fn parse_obj(raw: &[u8]) -> Option<Map<String, Value>>` — mesma regra de `_dict_da_linha`/`DobraClaude.linha`: tira espaços, decodifica UTF-8 com substituição, aceita surrogate solto (usa `crate::transcript::decode_line`), só objeto.
- Produces (em `costs::pricing`): `pub struct Rate { input, output, cache_read, cache_write: f64, provider: String, origin: String, cache_estimado: bool }`; `pub struct Pricing` com `fn load(dir: &Path) -> Pricing` (lê `models.dev.json` e `overrides.json` de `dir`; sem catálogo, o snapshot embutido `include_str!("../../../../backend/app/pricing_data.json")`), `fn generation(&self) -> u64`, `fn reload_if_changed(&mut self) -> bool` (mtimes dos dois arquivos), `fn canonizar(&self, m: &str) -> String`, `fn rate_for(&self, m: &str) -> Option<Rate>`, `fn rate_fast(&self, r: &Rate, m: &str) -> Rate`, `fn rate_codex(&self, r: &Rate, m: &str, long: bool) -> Rate`, `fn provider_for(&self, m: &str) -> Option<String>`; `pub fn custo(r: &Rate, i: i64, o: i64, cw: i64, cr: i64) -> [f64; 4]` (ordem input, output, cache_write, cache_read); `pub fn canonizar_provedor(p: &str) -> String`; `pub const IGNORADOS: [&str; 4]`; `pub fn default_dir() -> PathBuf` (`~/.claude/.hangar-pricing`).

- [ ] **Step 1: Teste de paridade que falha**

`crates/hangar-server/tests/contract_costs_pricing.rs`:

```rust
use hangar_server::costs::pricing::Pricing;
use hangar_server::costs::py::{LocalTs, py_int};
use serde_json::{Value, json};
use std::path::Path;

fn contract() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../backend/tests/fixtures/contract")
}

fn rate_json(r: Option<hangar_server::costs::pricing::Rate>) -> Value {
    r.map_or(Value::Null, |r| json!({"input": r.input, "output": r.output, "cache_read": r.cache_read,
        "cache_write": r.cache_write, "provider": r.provider, "origin": r.origin, "cache_estimado": r.cache_estimado}))
}

#[test]
fn pricing_matches_python() {
    let p = Pricing::load(&contract().join("costs/pricing"));
    let rows: Value = serde_json::from_slice(&std::fs::read(contract().join("golden/costs_pricing.json")).unwrap()).unwrap();
    for row in rows.as_array().unwrap() {
        let m = row["model"].as_str().unwrap();
        assert_eq!(p.canonizar(m), row["canon"].as_str().unwrap(), "canon {m}");
        let r = p.rate_for(m);
        assert_eq!(rate_json(r.clone()), row["rate"], "rate {m}");
        assert_eq!(rate_json(r.as_ref().map(|r| p.rate_fast(r, m))), row["fast"], "fast {m}");
        assert_eq!(rate_json(r.as_ref().map(|r| p.rate_codex(r, m, true))), row["codex_long"], "codex {m}");
    }
}

#[test]
fn snapshot_is_used_without_cache_dir() {
    let p = Pricing::load(Path::new("/nao/existe"));
    assert_eq!(p.rate_for("claude-opus-5").unwrap().origin, "snapshot");
}

#[test]
fn py_int_follows_python_int_or_zero() {
    for (v, want) in [(json!(null), 0), (json!(false), 0), (json!(true), 1), (json!(7), 7), (json!(7.9), 7),
                      (json!(-2.5), -2), (json!(" 12 "), 12), (json!("-3"), -3), (json!("3.5"), 0), (json!(""), 0),
                      (json!([1]), 0), (json!({"a": 1}), 0)] {
        assert_eq!(py_int(Some(&v)), want, "{v}");
    }
    assert_eq!(py_int(None), 0);
}

#[test]
fn local_time_matches_python_isoformat() {
    let t = LocalTs::from_iso("2026-09-30T02:30:00.120Z").unwrap();
    assert_eq!(t.iso(), "2026-09-29T23:30:00.120000-03:00");
    assert_eq!(t.day(), "2026-09-29");
    assert_eq!(LocalTs::from_iso("2026-09-30T10:00:00Z").unwrap().iso(), "2026-09-30T07:00:00-03:00");
    assert_eq!(LocalTs::from_millis_f64(1_790_935_200_123.4).iso(), "2026-10-02T07:00:00.123400-03:00");
    assert!(LocalTs::from_iso("ontem").is_none());
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test --manifest-path crates/Cargo.toml -p hangar-server --test contract_costs_pricing`
Expected: FAIL — módulo `costs` não existe.

- [ ] **Step 3: Implementar**

`costs/mod.rs`:

```rust
//! Custos e uso: leitores dos transcripts, índice incremental próprio e relatórios.
//! Porte de backend/app/costs*.py, uso_*.py e pricing.py; o Python é a referência (golden).
pub mod py;
pub mod pricing;
```

`costs/py.rs` — porte das regras do Step "Interfaces". Pontos que o golden pega:
- `iso()`: `chrono::DateTime<FixedOffset>` com `FixedOffset::west_opt(3*3600)`; `%Y-%m-%dT%H:%M:%S` e, se micro ≠ 0, `.%6f`; sufixo `-03:00`.
- `from_iso`: reaproveitar `crate::transcript::py::iso_timestamp` (paridade com `fromisoformat` já provada pelo `golden/isotime.json` da parte 1, que trata "sem fuso" como UTC). Para "sem fuso", somar 3 h depois (UTC-3). Torne `iso_timestamp` `pub(crate)` se ainda não for.
- `parse_obj`: `let s = String::from_utf8_lossy(trim_ascii(raw)); serde_json::from_str::<Value>(&s)` e, se falhar, `crate::transcript::decode_line(raw)`; só `Value::Object`.

`costs/pricing.rs` — porte de `pricing.py:44-104` (`slim` não: só `_rate`), `:110-160` (`_PREFIXOS`, `_APELIDOS`, `IGNORADOS`, `_APELIDOS_PROVEDOR`, `canonizar_provedor`), `:212-250` (carga: cache `{"modelos": {...}}` → origem `models.dev`; senão snapshot `{"modelos": ...}` → `snapshot`; overrides só com `input` e `output`), `:300-405` (`_canonizar` com laço de prefixos, catálogo, cru, apelido, minúsculas; `rate_for`; `custo`; `_FAST`; `rate_fast`; `rate_codex`). `_rate` usa `float()` do Python: aceite número ou texto numérico no JSON. Memorize `canonizar`/`rate_for` num `Mutex<HashMap>` limpo em `reload_if_changed`. `generation()` sobe a cada recarga.

- [ ] **Step 4: Rodar**

Run: `cargo test --manifest-path crates/Cargo.toml -p hangar-server --test contract_costs_pricing` — Expected: PASS.

- [ ] **Step 5: Revisão e commit**

```bash
git add crates/Cargo.toml crates/Cargo.lock crates/hangar-server/Cargo.toml crates/hangar-server/src/lib.rs \
  crates/hangar-server/src/costs crates/hangar-server/tests/contract_costs_pricing.rs
git commit -m "feat(server): cost module base with Python-compatible time, ints and pricing"
```

---

### Task 4: Índice incremental em SQLite (`custos-rust.sqlite3`)

**Files:**
- Create: `crates/hangar-server/src/costs/index.rs`, `crates/hangar-server/src/costs/rows.rs`
- Modify: `crates/hangar-server/src/costs/mod.rs`, `crates/Cargo.toml` (`rusqlite = { version = "=<atual>", features = ["bundled"] }`: rode `cargo search rusqlite --limit 1` e fixe a versão que ele mostrar), `crates/hangar-server/Cargo.toml`
- Test: `crates/hangar-server/tests/costs_index.rs`

**Interfaces:**
- Produces (em `costs::rows`, colunas na ordem do Python):

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageRow {           // costs_sources.UsageRow, mesma ordem de campos
    pub ts: LocalTs, pub source: String, pub provider: String, pub model: String, pub project: String,
    pub session_id: String, pub input: i64, pub output: i64, pub cache_write: i64, pub cache_read: i64,
    pub subagente: bool, pub account_id: Option<String>, pub codex_long_context: bool, pub cache_write_1h: i64,
    pub fast: bool, pub regravado: i64, pub regravado_1h: i64,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsoLinha {           // uso_claude.UsoLinha sem `conta` (carimbada na leitura)
    pub dia: String, pub cwd: String, pub model: String, pub tipo: String, pub nome: String, pub plugin: String,
    pub detalhe: String, pub origem: String, pub chamadas: i64, pub ctx_chars: i64, pub tokens_est: i64,
    pub input: i64, pub output: i64, pub cache_write: i64, pub cache_read: i64, pub cache_write_1h: i64,
    pub fast: bool, pub ocupados: i64, pub respostas: i64, pub ocupados_eq: i64, pub fonte: String,
    pub subagente: bool, pub session_id: String,
}
pub struct FoldOutput { pub costs: Vec<UsageRow>, pub usage: Vec<UsoLinha>, pub areas: Option<AreaEntries> }
```

`AreaEntries` é definido na Task 5 (`costs::areas`); nesta Task use `pub type AreaEntries = serde_json::Value;` provisório em `rows.rs` e troque na Task 5 (o índice só guarda e devolve).

- Produces (em `costs::index`):

```rust
pub trait Fold: Serialize + DeserializeOwned + Send {
    fn line(&mut self, raw: &[u8]);
    /// Pode estragar o estado: o índice serializa ANTES de passar a linha sem `\n` e fechar.
    fn close(&mut self) -> FoldOutput;
}
pub struct Index { /* caminho, conexão por operação */ }
impl Index {
    pub fn open(dir: &Path) -> Result<Index, IndexError>;            // cria pasta, WAL, esquema
    pub fn sync<F: Fold>(&self, scope: &str, files: &[PathBuf], new_fold: &(dyn Fn(&Path) -> F + Sync),
                         version: &str, areas_sig: &str, redo_areas: &dyn Fn(&AreaEntries) -> Vec<UsoLinha>,
                         progress: &Progress) -> Result<bool, IndexError>;   // true = mudou algo
    pub fn sync_file<F: Fold>(&self, path: &Path, new_fold: &dyn Fn(&Path) -> F, version: &str, scope: &str,
                              areas_sig: &str, redo_areas: &dyn Fn(&AreaEntries) -> Vec<UsoLinha>) -> Option<i64>;
    pub fn forget_outside(&self, active: &[String]) -> Result<bool, IndexError>;
    pub fn read_costs(&self, scope: Option<&str>, since: Option<&str>, file_id: Option<i64>) -> Result<Vec<UsageRow>, IndexError>;
    pub fn read_usage(&self, scope: &str, since: Option<&str>) -> Result<Vec<UsoLinha>, IndexError>;
}
#[derive(Default)]
pub struct Progress { /* AtomicUsize lidos/total por escopo */ }
impl Progress { pub fn reset(&self); pub fn total(&self) -> (usize, usize); }
pub const SCHEMA: u32 = 1;
pub const FILE_NAME: &str = "custos-rust.sqlite3";
pub fn default_dir() -> PathBuf; // Linux/macOS ~/.claude/.hangar-custos; Windows %LOCALAPPDATA%\hangar\custos
```

`sync` lê os arquivos que mudaram em paralelo (`rayon`, `par_iter` no pool global montado pela Task 9; nesta Task a ordem de gravação é a da lista) e grava numa thread só, em transações de até 1 s, como `costs_cache.sincronizar` (`costs_cache.py:362-447`).

- [ ] **Step 1: Testes que falham**

`crates/hangar-server/tests/costs_index.rs` — com uma dobra de teste que soma um número por linha:

```rust
use hangar_server::costs::index::{Fold, Index, Progress};
use hangar_server::costs::py::LocalTs;
use hangar_server::costs::rows::{FoldOutput, UsageRow, UsoLinha};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Default)]
struct Soma { n: i64, linhas: i64 }

impl Fold for Soma {
    fn line(&mut self, raw: &[u8]) {
        if let Ok(v) = std::str::from_utf8(raw).unwrap_or("").trim().parse::<i64>() { self.n += v; self.linhas += 1 }
    }
    fn close(&mut self) -> FoldOutput {
        let ts = LocalTs::from_iso("2026-09-30T12:00:00Z").unwrap();
        FoldOutput { costs: vec![UsageRow { ts, source: "t".into(), provider: "".into(), model: "m".into(),
            project: "".into(), session_id: "s".into(), input: self.n, output: self.linhas, cache_write: 0,
            cache_read: 0, subagente: false, account_id: None, codex_long_context: false, cache_write_1h: 0,
            fast: false, regravado: 0, regravado_1h: 0 }], usage: vec![], areas: None }
    }
}

fn nova(_: &Path) -> Soma { Soma::default() }
fn sem_areas(_: &hangar_server::costs::rows::AreaEntries) -> Vec<UsoLinha> { vec![] }

fn sync(ix: &Index, files: &[PathBuf]) -> bool {
    ix.sync("t", files, &nova, "v1", "sig", &sem_areas, &Progress::default()).unwrap()
}

fn soma(ix: &Index) -> (i64, i64) {
    ix.read_costs(Some("t"), None, None).unwrap().iter().fold((0, 0), |a, r| (a.0 + r.input, a.1 + r.output))
}

#[test]
fn append_reads_only_new_bytes_and_matches_full_read() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.jsonl");
    std::fs::write(&f, "1\n2\n").unwrap();
    let ix = Index::open(&d.path().join("idx")).unwrap();
    assert!(sync(&ix, &[f.clone()]));
    std::fs::OpenOptions::new().append(true).open(&f).unwrap().write_all(b"3\n").unwrap();
    sync(&ix, &[f.clone()]);
    assert_eq!(soma(&ix), (6, 3));
    assert!(!sync(&ix, &[f.clone()]), "sem mudança não grava");
}

#[test]
fn line_without_newline_counts_once_and_is_not_saved_in_state() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.jsonl");
    std::fs::write(&f, "1\n2").unwrap();
    let ix = Index::open(&d.path().join("idx")).unwrap();
    sync(&ix, &[f.clone()]);
    assert_eq!(soma(&ix), (3, 2));
    std::fs::OpenOptions::new().append(true).open(&f).unwrap().write_all(b"0\n").unwrap();
    sync(&ix, &[f.clone()]);
    assert_eq!(soma(&ix), (20, 2), "\"2\" virou \"20\": a linha incompleta não fica no estado");
}

#[test]
fn shrunk_replaced_or_rewritten_file_is_reread_from_zero() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.jsonl");
    std::fs::write(&f, "5\n5\n").unwrap();
    let ix = Index::open(&d.path().join("idx")).unwrap();
    sync(&ix, &[f.clone()]);
    std::fs::write(&f, "1\n").unwrap();                       // encolheu
    sync(&ix, &[f.clone()]);
    assert_eq!(soma(&ix), (1, 1));
    std::fs::write(&f, "9\n1\n7\n").unwrap();                 // mesmo inode, maior, começo mudado
    sync(&ix, &[f.clone()]);
    assert_eq!(soma(&ix), (17, 3));
    let g = d.path().join("b.jsonl");
    std::fs::write(&g, "4\n").unwrap();
    std::fs::rename(&g, &f).unwrap();                         // outro inode
    sync(&ix, &[f.clone()]);
    assert_eq!(soma(&ix), (4, 1));
}

#[test]
fn vanished_file_leaves_the_scope_and_other_version_rereads() {
    let d = tempfile::tempdir().unwrap();
    let (a, b) = (d.path().join("a.jsonl"), d.path().join("b.jsonl"));
    std::fs::write(&a, "1\n").unwrap();
    std::fs::write(&b, "2\n").unwrap();
    let ix = Index::open(&d.path().join("idx")).unwrap();
    sync(&ix, &[a.clone(), b.clone()]);
    sync(&ix, &[a.clone()]);
    assert_eq!(soma(&ix), (1, 1));
}

#[test]
fn unreadable_database_is_rebuilt() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("idx")).unwrap();
    std::fs::write(d.path().join("idx").join(hangar_server::costs::index::FILE_NAME), b"lixo que nao e sqlite").unwrap();
    let f = d.path().join("a.jsonl");
    std::fs::write(&f, "2\n").unwrap();
    let ix = Index::open(&d.path().join("idx")).unwrap();
    sync(&ix, &[f]);
    assert_eq!(soma(&ix), (2, 1));
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test --manifest-path crates/Cargo.toml -p hangar-server --test costs_index` — Expected: FAIL (módulo inexistente).

- [ ] **Step 3: Implementar**

Esquema (mesmos nomes do Python; `estado` e `areas` são `serde_json` + `flate2` nível 1):

```rust
const TABLES: &str = "
CREATE TABLE meta(k TEXT PRIMARY KEY, v TEXT);
CREATE TABLE files(id INTEGER PRIMARY KEY, path TEXT UNIQUE NOT NULL, scope TEXT NOT NULL,
    versao TEXT NOT NULL, dev INTEGER, ino INTEGER, size INTEGER, mtime_ns INTEGER,
    offset INTEGER, cauda BLOB, estado BLOB, areas BLOB, areas_sig TEXT);
CREATE INDEX files_scope ON files(scope);
CREATE TABLE custo(file_id INTEGER NOT NULL, dia TEXT, ts, source, provider, model, project, session_id,
    input, output, cache_write, cache_read, subagente, account_id, codex_long_context, cache_write_1h, fast,
    regravado, regravado_1h);
CREATE INDEX custo_file ON custo(file_id);
CREATE TABLE uso(file_id INTEGER NOT NULL, dia, cwd, model, tipo, nome, plugin, detalhe, origem, chamadas,
    ctx_chars, tokens_est, input, output, cache_write, cache_read, cache_write_1h, fast, ocupados, respostas,
    ocupados_eq, fonte, subagente, session_id);
CREATE INDEX uso_file ON uso(file_id);";
```

Porte de `costs_cache.py`: `_preparar` (`:158-188`, esquema diferente apaga e refaz numa transação), `_abrir` (`:202-234`: arquivo ilegível → apaga `''`, `-wal`, `-shm` e refaz; sem disco → `IndexError::NoDisk`, e quem chama repassa ao Python em vez de usar memória), `_ler` (`:266-301`: retomada só com estado, mesma versão, mesmo `(dev, ino)`, `offset <= size` e os 64 bytes antes do offset iguais; estado serializado ANTES do fragmento), `_gravar_arquivo` (`:308-327`, upsert com `manter_escopo`), `_refazer_areas` (`:330-345`, chamada com `redo_areas`), `sincronizar` (`:362-447`: `conhecidos` sem o estado; arquivo `NotFound` some do conjunto mas não do índice; outro erro de `stat` mantém; leitura que falha vira `tracing::warn!(code = "leitura_custos", ...)` sem caminho de conversa e mantém as linhas; sumidos apagados no fim), `sincronizar_arquivo` (`:450-480`), `esquecer_fora` (`:483-503`), `ler_custos`/`iter_usage_rows` (`:508-553`, `ORDER BY rowid`, filtro por subconsulta de escopo). `(dev, ino)` com máscara de 63 bits (`std::os::unix::fs::MetadataExt` / no Windows `file_index` indisponível → `(0, 0)`, como o Python que recebe `st_ino` 0). `mtime_ns` inteiro.

- [ ] **Step 4: Rodar**

Run: `cargo test --manifest-path crates/Cargo.toml -p hangar-server --test costs_index` — Expected: PASS.

- [ ] **Step 5: Revisão e commit**

```bash
git add crates/Cargo.toml crates/Cargo.lock crates/hangar-server/Cargo.toml \
  crates/hangar-server/src/costs crates/hangar-server/tests/costs_index.rs
git commit -m "feat(server): incremental SQLite cost index for the Rust readers"
```

---

### Task 5: Áreas do código e regras puras do uso

**Files:**
- Create: `crates/hangar-server/src/costs/areas.rs`, `crates/hangar-server/src/costs/uso_rules.rs`
- Modify: `crates/hangar-server/src/costs/mod.rs`, `costs/rows.rs` (o alias provisório vira `pub use crate::costs::areas::AreaEntries;`, para o teste da Task 4 seguir compilando)
- Test: `crates/hangar-server/tests/contract_costs_areas.rs`

**Interfaces:**
- Produces (em `costs::areas`):

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolReg {              // os registros de uso_areas.py:178-180
    P { rules_cwd: String, cwd: String, paths: Vec<String> },
    C { rules_cwd: String, cwd: String, candidates: Vec<String> },
    S { rules_cwd: String, target: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Unit { pub dia: String, pub cwd: String, pub model: String, pub fast: bool, pub values: [i64; 5] }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AreaHeader { pub fonte: Option<String>, pub session_id: Option<String>, pub subagente: Option<bool> }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AreaEntries { pub header: AreaHeader, pub turns: Vec<(Vec<ToolReg>, Vec<Unit>)> }
pub struct AreaMap { /* regras padrão + por projeto, assinatura */ }
impl AreaMap {
    pub fn load(file: &Path) -> AreaMap;          // ~/.hangar/uso-areas.json; ausente/ilegível = PADRAO
    pub fn signature(&self) -> &str;              // própria do Rust (md5 do texto + versão da divisão)
    pub fn rules_for(&self, cwd: &str) -> Vec<(String, Vec<String>)>;
    pub fn area_of_target(&self, target: &str, rules: &[(String, Vec<String>)]) -> Option<String>;
    pub fn area_of_path(&self, path: &str, cwd: &str, rules: &[(String, Vec<String>)]) -> String;
    pub fn count_areas(&self, regs: &[ToolReg]) -> IndexMap<String, i64>;
    pub fn area_lines(&self, entries: &AreaEntries) -> Vec<UsoLinha>;   // uso_claude.linhas_de_area
}
pub fn repartir(valor: i64, pesos: &IndexMap<String, i64>) -> IndexMap<String, i64>;
pub fn candidates(cmd: &str) -> Vec<String>;      // candidatos_do_comando
pub fn default_map_file() -> PathBuf;             // ~/.hangar/uso-areas.json
```

- Produces (em `costs::uso_rules`): `pub fn comando_bash(cmd: &str) -> String`, `pub fn skill_do_caminho(p: &str) -> Option<(String, bool)>`, `pub fn plugin_de(nome: &str) -> String`, `pub fn plugin_de_hook(primeira: &str) -> String`, `pub fn pede_agente(prompt: &str) -> bool`, `pub fn tokens_de_imagem(bloco: &Map<String, Value>) -> (i64, i64)`, `pub fn texto_len(c: Option<&Value>) -> i64` (`_texto`).

- [ ] **Step 1: Teste que falha**

`crates/hangar-server/tests/contract_costs_areas.rs`:

```rust
use hangar_server::costs::areas::{AreaMap, candidates, repartir};
use hangar_server::costs::uso_rules::{comando_bash, skill_do_caminho};
use indexmap::IndexMap;
use serde_json::Value;
use std::path::Path;

fn golden() -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../backend/tests/fixtures/contract/golden/costs_areas.json");
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

#[test]
fn area_rules_match_python() {
    let g = golden();
    for r in g["comando_bash"].as_array().unwrap() {
        assert_eq!(comando_bash(r[0].as_str().unwrap()), r[1].as_str().unwrap(), "{r}");
    }
    for r in g["candidatos"].as_array().unwrap() {
        let want: Vec<String> = serde_json::from_value(r[1].clone()).unwrap();
        assert_eq!(candidates(r[0].as_str().unwrap()), want, "{r}");
    }
    for r in g["skill_do_caminho"].as_array().unwrap() {
        let got = skill_do_caminho(r[0].as_str().unwrap()).map(|(n, md)| serde_json::json!([n, md]));
        assert_eq!(got.unwrap_or(Value::Null), r[1], "{r}");
    }
    for r in g["repartir"].as_array().unwrap() {
        let pesos: IndexMap<String, i64> = serde_json::from_value(r[1].clone()).unwrap();
        let want: IndexMap<String, i64> = serde_json::from_value(r[2].clone()).unwrap();
        assert_eq!(repartir(r[0].as_i64().unwrap(), &pesos), want, "{r}");
    }
    let map = AreaMap::load(Path::new("/nao/existe.json"));
    let rules = map.rules_for("/qualquer");
    for r in g["area_do_alvo"].as_array().unwrap() {
        assert_eq!(map.area_of_target(r[0].as_str().unwrap(), &rules).map(Value::from).unwrap_or(Value::Null), r[1], "{r}");
    }
}

#[test]
fn repo_root_comes_from_the_file_not_the_cwd() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("vizinho");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join("backend")).unwrap();
    let map = AreaMap::load(Path::new("/nao/existe.json"));
    let rules = map.rules_for("/outro");
    let alvo = repo.join("backend/x.py");
    assert_eq!(map.area_of_path(alvo.to_str().unwrap(), "/outro", &rules), "back");
    assert_eq!(map.area_of_path("/tmp/solto.py", "/outro", &rules), "outros");
}
```

- [ ] **Step 2: Rodar e ver falhar** — `cargo test ... --test contract_costs_areas` → FAIL.

- [ ] **Step 3: Implementar**

Porte de `uso_areas.py:44-199` e `uso_claude.py:74-238`. Regras que o golden e a revisão conferem:
- `fnmatch.translate` portado à mão em `areas.rs` (`*` → `.*` com asteriscos seguidos colapsados, `?` → `.`, `[...]`/`[!...]` como conjunto, resto escapado com `regex::escape`), dentro de `(?s:...)\z`; o casador é `^(?:/(?s:P)\z|(?s:.*/P)\z)` por padrão, unidos por `|` (mesma forma de `_casador`), sensível a caixa. Memorize por tupla de padrões.
- `repartir`: maior resto, desempate pelo nome (`sorted(..., key=(inteiro - exato, nome))`), em `f64` como o Python.
- `raiz_do_repo`: sobe procurando `.git` (arquivo ou pasta); cache `Mutex<HashMap>` limitado a 4096 entradas, limpo ao recarregar.
- `area_of_path`: `os.path.normpath`/`isabs`/`join`/`relpath` em POSIX; no Windows (`cfg(windows)`) `normcase` (minúsculas e `\`). Porte linha a linha de `area_do_caminho` (`:143-154`).
- `comando_bash`: separadores `&&|\|\||[;|\n]`, prefixos e palavras de shell iguais (`uso_claude.py:31-116`); `str.split()` do Python separa por qualquer espaço Unicode — use `char::is_whitespace`.
- `pede_agente`: a regex `_PEDE_AGENTE` com `(?i)` e `\b` Unicode (padrão do crate `regex`, igual ao `re` com `str`).
- `tokens_de_imagem`: `base64` dos primeiros 32 caracteres (decodificar à mão os 24 bytes necessários, sem crate novo), PNG por assinatura, `largura * altura // 750`.
- `area_lines`: porte de `linhas_de_area` (`uso_claude.py:213-238`), saída na ordem de inserção do dicionário `(dia, cwd, model, area)`; `header` preenche `fonte`/`session_id`/`subagente` (ausentes = padrão de `UsoLinha`).

- [ ] **Step 4: Rodar** — Expected: PASS.

- [ ] **Step 5: Revisão e commit**

```bash
git add crates/hangar-server/src/costs crates/hangar-server/tests/contract_costs_areas.rs
git commit -m "feat(server): code areas and usage rules ported from uso_areas and uso_claude"
```

---

### Task 6: Leitor do Claude (custo + uso) com paridade de índice

**Files:**
- Create: `crates/hangar-server/src/costs/claude.rs`, `crates/hangar-server/src/costs/accumulator.rs`
- Create: `crates/hangar-server/tests/common/costs.rs` (helper de paridade, usado pelas Tasks 6–8)
- Test: `crates/hangar-server/tests/contract_costs_index.rs`

**Interfaces:**
- Consumes: `Fold`, `Index`, `UsoLinha`, `UsageRow` (Task 4); `AreaMap`, `ToolReg`, `AreaEntries`, regras (Task 5); `py` (Task 3).
- Produces:
  - `costs::accumulator::Accumulator` — porte de `uso_claude.Acumulador` (`uso_claude.py:245-562`), `Serialize/Deserialize`, com `pub fn line(&mut self, d: &Map<String, Value>, dia: &str, areas: &AreaMap)`, `pub fn entries(&self) -> AreaEntries`, `pub fn lines_without_area(&self) -> Vec<UsoLinha>`, e os métodos `pub(crate)` que o Codex reutiliza: `somar`, `resposta_nova`, `ler_arquivo_de_skill`, `carregar`.
  - `costs::claude::ClaudeFold` — porte de `DobraClaude` (`costs_claude_transcript.py:102-212`), implementa `Fold`; `pub fn new_fold(root: &Path) -> impl Fn(&Path) -> ClaudeFold + Sync` (identidade pelo caminho relativo com `/`, `subagents` no caminho); `pub const VERSION: &str = "claude:12";` (acompanha `CACHE_VERSAO` do Python; o golden prende).
- `tests/common/costs.rs`:

```rust
use hangar_server::costs::index::{Index, Progress};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn contract() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../backend/tests/fixtures/contract")
}

/// Copia as fixtures para um tmp (o golden usa caminhos relativos a `costs/`).
pub fn fixtures_copy() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let base = d.path().join("costs");
    copy_dir(&contract().join("costs"), &base);
    (d, base)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() { copy_dir(&p, &to.join(e.file_name())) } else { std::fs::copy(&p, to.join(e.file_name())).unwrap(); }
    }
}

/// Mesmo formato de `_dump` do gen_costs.py, para os arquivos sob `prefix`.
pub fn dump(ix: &Index, base: &Path, prefix: &str) -> Value { hangar_server::costs::index::dump_for_tests(ix, base, prefix) }

/// Números fracionários com erro relativo de 1e-9; todo o resto exato.
pub fn assert_close(got: &Value, want: &Value, at: &str) {
    match (got, want) {
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            assert!((a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0), "{at}: {a} != {b}");
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{at}: tamanho");
            for (i, (x, y)) in a.iter().zip(b).enumerate() { assert_close(x, y, &format!("{at}[{i}]")) }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>(), "{at}: chaves");
            for (k, x) in a { assert_close(x, &b[k], &format!("{at}.{k}")) }
        }
        _ => assert_eq!(got, want, "{at}"),
    }
}

pub fn golden_index() -> Value {
    serde_json::from_slice(&std::fs::read(contract().join("golden/costs_index.json")).unwrap()).unwrap()
}

pub fn progress() -> Progress { Progress::default() }
```

`costs::index::dump_for_tests(ix: &Index, base: &Path, prefix: &str) -> Value` (função livre, pública, `#[doc(hidden)]`) devolve `{rel: {"custo": [...], "uso": [...], "areas": [...]}}` com as mesmas colunas e ordem do `_dump` Python (sem `dia` no custo; booleanos como `0/1`, porque o golden lê do SQLite do Python).

- [ ] **Step 1: Teste que falha**

`crates/hangar-server/tests/contract_costs_index.rs`:

```rust
mod common;
use common::costs::*;
use hangar_server::costs::areas::AreaMap;
use hangar_server::costs::claude;
use hangar_server::costs::index::Index;

fn sync_claude(ix: &Index, base: &std::path::Path, areas: &AreaMap) {
    let root = base.join("claude/projects");
    let files = hangar_server::costs::collect::list_files(&root, |n| n.ends_with(".jsonl"));
    ix.sync(&format!("claude:{}", root.display()), &files, &claude::new_fold(&root), claude::VERSION,
            areas.signature(), &|e| areas.area_lines(e), &progress()).unwrap();
}

#[test]
fn claude_index_matches_python() {
    let (_d, base) = fixtures_copy();
    let areas = AreaMap::load(std::path::Path::new("/nao/existe.json"));
    let ix = Index::open(&base.join("../idx")).unwrap();
    sync_claude(&ix, &base, &areas);
    let g = golden_index();
    let want: serde_json::Map<_, _> = g.as_object().unwrap().iter()
        .filter(|(k, _)| k.starts_with("claude/")).map(|(k, v)| (k.clone(), v.clone())).collect();
    assert_close(&dump(&ix, &base, "claude/"), &serde_json::Value::Object(want), "claude");
}

#[test]
fn claude_resumed_in_two_halves_matches_python() {
    let (_d, base) = fixtures_copy();
    let areas = AreaMap::load(std::path::Path::new("/nao/existe.json"));
    let ix = Index::open(&base.join("../idx")).unwrap();
    let rest = halve_all(&base);
    sync_claude(&ix, &base, &areas);
    for (p, tail) in rest { append(&p, &tail) }
    sync_claude(&ix, &base, &areas);
    let g = golden_index();
    let want: serde_json::Map<_, _> = g["__resumed__"].as_object().unwrap().iter()
        .filter(|(k, _)| k.starts_with("claude/")).map(|(k, v)| (k.clone(), v.clone())).collect();
    assert_close(&dump(&ix, &base, "claude/"), &serde_json::Value::Object(want), "claude retomado");
}
```

Acrescentar a `tests/common/costs.rs` `halve_all` (mesma regra de `_copiar_em_metades`: corte no primeiro `\n` depois da metade, ignora `session_index.jsonl`, ordem de `rglob` ordenada) e `append`. `collect::list_files` nasce nesta Task em `costs/collect.rs` como porte de `costs_cache.listar` (`:556-575`, sem seguir link de pasta); a Task 9 completa o resto do módulo.

- [ ] **Step 2: Rodar e ver falhar** — `cargo test ... --test contract_costs_index` → FAIL.

- [ ] **Step 3: Implementar**

Porte linha a linha. Regras que o golden e a revisão conferem:
- Pré-filtro de bytes antes de decodificar (`"usage"`, `"user"`, `"attachment"`, `"compact_boundary"`, com aspas), `numero` incrementa em toda linha, inclusive as filtradas.
- Decodificação por `py::parse_obj` (surrogate solto e byte inválido não pulam a linha; linha `null`/lista pula). Para velocidade, tente primeiro um struct tipado com só os campos lidos (`type`, `subtype`, `timestamp`, `cwd`, `requestId`, `promptId`, `isMeta`, `message`, `attachment`, `rendered`, `toolUseResult`, todos `Option<Value>`, desconhecidos ignorados) e caia no `parse_obj` se o serde recusar; o comportamento tem de ser idêntico nos dois caminhos (o teste do surrogate cobre o segundo).
- Modelo em `IGNORADOS` (depois de `strip`) descarta a linha inteira, inclusive para o acumulador.
- Chave da resposta `(requestId, message.id)` quando há `id`, senão o número da linha; última versão vence sem mudar a posição (`IndexMap::insert` mantém a posição — igual ao `dict`).
- `usos()`: regravado só com `perdido * 2 >= contexto_antes`, fora das respostas depois de compactar; grupo `(data local, modelo, cwd, fast)`; saída ordenada por `(ts, model, cwd)` estável.
- `cache_write_1h = min(max(0, 1h), max(0, cache_creation))`.
- `fechar()`: custos, depois uso sem área (ordenado por `(dia, tipo, nome, detalhe)`), entradas de área com `session_id`/`subagente` (do caminho: `subagente_uso` olha o id relativo).
- Contagem de caracteres com `py::char_len`, nunca `str::len`.
- `_respostas` do acumulador (`:520-531`) e `_turnos` (`:533-545`) com `ToolReg` (Task 5).

- [ ] **Step 4: Rodar** — Expected: PASS nos dois testes.

- [ ] **Step 5: Revisão e commit**

```bash
git add crates/hangar-server/src/costs crates/hangar-server/tests/common/costs.rs crates/hangar-server/tests/common/mod.rs \
  crates/hangar-server/tests/contract_costs_index.rs
git commit -m "feat(server): Claude cost and usage reader with index parity"
```

(Se `tests/common/mod.rs` já existir, só acrescente `pub mod costs;` a ele.)

---

### Task 7: Leitor do Codex (custo + uso) e custo de um rollout

**Files:**
- Create: `crates/hangar-server/src/costs/codex.rs`
- Modify: `crates/hangar-server/tests/contract_costs_index.rs` (testes do Codex)

**Interfaces:**
- Consumes: Tasks 3–6 (`Accumulator` e seus métodos `pub(crate)`).
- Produces: `costs::codex::CodexFold` (porte de `DobraCodex` = `RespostasCodex` `costs_sources.py:192-319` + `AcumuladorCodex` `uso_codex.py:72-170` + `_agrupar_rollout` `:330-339`), `pub fn new_fold(p: &Path) -> CodexFold`, `pub const VERSION: &str = "codex:1:2";` (`CACHE_VERSAO`:`_USO_CODEX_VERSAO`); `pub fn session_rows(ix: &Index, rollout: &Path, areas: &AreaMap) -> Option<Vec<UsageRow>>` (porte de `custos_do_rollout`, escopo `codex:avulso`).

- [ ] **Step 1: Testes que falham** — acrescentar a `contract_costs_index.rs`:

```rust
use hangar_server::costs::codex;

fn sync_codex(ix: &Index, base: &std::path::Path, areas: &AreaMap) {
    let files = hangar_server::costs::collect::list_files(&base.join("codex/sessions"),
        |n| n.starts_with("rollout-") && n.ends_with(".jsonl"));
    let mut files = files;
    files.sort();
    ix.sync(&format!("codex:{}", base.join("codex").display()), &files, &codex::new_fold, codex::VERSION,
            areas.signature(), &|e| areas.area_lines(e), &progress()).unwrap();
}

#[test]
fn codex_index_matches_python() {
    let (_d, base) = fixtures_copy();
    let areas = AreaMap::load(std::path::Path::new("/nao/existe.json"));
    let ix = Index::open(&base.join("../idx")).unwrap();
    sync_codex(&ix, &base, &areas);
    let g = golden_index();
    let want: serde_json::Map<_, _> = g.as_object().unwrap().iter()
        .filter(|(k, _)| k.starts_with("codex/")).map(|(k, v)| (k.clone(), v.clone())).collect();
    assert_close(&dump(&ix, &base, "codex/"), &serde_json::Value::Object(want), "codex");
}

#[test]
fn codex_resumed_in_two_halves_matches_python() {
    let (_d, base) = fixtures_copy();
    let areas = AreaMap::load(std::path::Path::new("/nao/existe.json"));
    let ix = Index::open(&base.join("../idx")).unwrap();
    let rest = halve_all(&base);
    sync_codex(&ix, &base, &areas);
    for (p, tail) in rest { append(&p, &tail) }
    sync_codex(&ix, &base, &areas);
    let g = golden_index();
    let want: serde_json::Map<_, _> = g["__resumed__"].as_object().unwrap().iter()
        .filter(|(k, _)| k.starts_with("codex/")).map(|(k, v)| (k.clone(), v.clone())).collect();
    assert_close(&dump(&ix, &base, "codex/"), &serde_json::Value::Object(want), "codex retomado");
}

#[test]
fn single_rollout_cost_reads_only_growth_and_keeps_existing_scope() {
    let (_d, base) = fixtures_copy();
    let areas = AreaMap::load(std::path::Path::new("/nao/existe.json"));
    let ix = Index::open(&base.join("../idx")).unwrap();
    let p = base.join("codex/sessions/2026/09/30/rollout-c1.jsonl");
    let rows = codex::session_rows(&ix, &p, &areas).unwrap();
    assert!(!rows.is_empty());
    sync_codex(&ix, &base, &areas);
    assert_eq!(codex::session_rows(&ix, &p, &areas).unwrap(), rows);
}
```

- [ ] **Step 2: Rodar e ver falhar.**

- [ ] **Step 3: Implementar** — porte linha a linha. Regras conferidas:
- `session_meta`: só a primeira identifica o arquivo; a seguinte com outra identidade marca `herdado`, que `turn_context` posterior ao início desfaz.
- `token_usage_record` de outra thread só abre o turno; `response_id` repetido ignora; contador legado com reinício (algum campo menor) conta a resposta inteira.
- `entrada > 272_000` marca contexto longo; `cache = min(entrada, cached)`; `escrita = min(entrada - cache, cache_write)`; `input = entrada - cache - escrita`.
- `por_turno` não muda o estado (pode ser chamada a cada retomada): o `close()` do índice chama depois de serializar.
- Uso: regex `_CHAMADA`, `_CMD`, `_WORKDIR`, `_PATH`, `_PATCH`, janela de 4000 **caracteres**; `_literal` (JSON entre aspas duplas, senão troca de escapes); saída dividida igual (`chars // n`); `spawn_agent` lê `arguments` como JSON.
- `entradas_de_area_codex`: cabeçalho `{"fonte": "codex", "session_id", "subagente"}`, unidade com dia local da resposta, `fast=false`, 1h=0.
- `provider` = `canonizar_provedor(model_provider) or "openai"`.

- [ ] **Step 4: Rodar** — PASS.

- [ ] **Step 5: Revisão e commit**

```bash
git add crates/hangar-server/src/costs crates/hangar-server/tests/contract_costs_index.rs
git commit -m "feat(server): Codex cost and usage reader with index parity"
```

---

### Task 8: Leitores do Pi/omp e do Kimi

**Files:**
- Create: `crates/hangar-server/src/costs/simple.rs`
- Modify: `crates/hangar-server/tests/contract_costs_index.rs`

**Interfaces:**
- Produces: `costs::simple::PiFold` (porte de `DobraPi`, `costs_sources.py:400-449`; `new_pi_fold(root: &Path, source: &str) -> impl Fn(&Path) -> PiFold + Sync`, id pelo caminho relativo sem extensão), `KimiFold` (porte de `DobraKimi`, `:520-567`; `new_kimi_fold(p: &Path) -> KimiFold`, id = nome de `parent.parent.parent`, subagente por `kimi_sessions.is_subagent_wire` — porte da regra: o diretório do agente não é `main`), `pub const PI_VERSION: &str = "pi:1"; pub const KIMI_VERSION: &str = "kimi:1";`, `pub fn kimi_projects(index_file: &Path) -> HashMap<String, String>` (porte de `_kimi_index`), aplicada na leitura (Task 9).

- [ ] **Step 1: Teste que falha** — acrescentar:

```rust
use hangar_server::costs::simple;

#[test]
fn pi_and_kimi_index_match_python() {
    let (_d, base) = fixtures_copy();
    let ix = Index::open(&base.join("../idx")).unwrap();
    let pi = base.join("pi");
    let files = hangar_server::costs::collect::list_files(&pi, |n| n.ends_with(".jsonl"));
    ix.sync(&format!("pi:{}", pi.display()), &files, &simple::new_pi_fold(&pi, "pi"), simple::PI_VERSION,
            "", &|_| vec![], &progress()).unwrap();
    let kimi = base.join("kimi/sessions");
    let files = hangar_server::costs::collect::list_files(&kimi, |n| n == "wire.jsonl");
    ix.sync(&format!("kimi:{}", kimi.display()), &files, &simple::new_kimi_fold, simple::KIMI_VERSION,
            "", &|_| vec![], &progress()).unwrap();
    let g = golden_index();
    for prefix in ["pi/", "kimi/"] {
        let want: serde_json::Map<_, _> = g.as_object().unwrap().iter()
            .filter(|(k, _)| k.starts_with(prefix)).map(|(k, v)| (k.clone(), v.clone())).collect();
        assert_close(&dump(&ix, &base, prefix), &serde_json::Value::Object(want), prefix);
    }
}
```

- [ ] **Step 2: Rodar e ver falhar.**
- [ ] **Step 3: Implementar.** Kimi: pré-filtro `usage.record` em bytes; `time` em ms vira `LocalTs::from_millis_f64`; provedor = prefixo do alias canonizado, senão o prefixo, senão `"?"`; projeto sempre `desconhecido` no índice. Pi: `model_change` com `/` separa provedor e id; soma `input/output/cacheRead/cacheWrite`; sem uso ou sem `ts` → nada.
- [ ] **Step 4: Rodar** — PASS.
- [ ] **Step 5: Revisão e commit**

```bash
git add crates/hangar-server/src/costs/simple.rs crates/hangar-server/src/costs/mod.rs crates/hangar-server/tests/contract_costs_index.rs
git commit -m "feat(server): Pi, omp and Kimi cost readers with index parity"
```

---

### Task 9: Coleta — escopos do Python, posse dos rollouts, varredura e aquecimento

**Files:**
- Modify: `crates/hangar-server/src/costs/collect.rs`, `crates/Cargo.toml` (`rayon`, versão atual com `=`), `crates/hangar-server/Cargo.toml`
- Test: `crates/hangar-server/tests/costs_collect.rs`

**Interfaces:**
- Consumes: Tasks 3–8; rota `/internal/costs/scopes` (Task 1).
- Produces:

```rust
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Scopes { pub claude: Vec<ClaudeScope>, pub codex: Vec<CodexScope>, pub pi: Vec<PiScope>,
                    pub kimi: Option<KimiScope>, pub repo: PathBuf }
#[derive(Clone, Debug, Deserialize, PartialEq)] pub struct ClaudeScope { pub root: PathBuf, pub account: String, pub label: String }
#[derive(Clone, Debug, Deserialize, PartialEq)] pub struct CodexScope { pub home: PathBuf, pub account: String, pub label: String }
#[derive(Clone, Debug, Deserialize, PartialEq)] pub struct PiScope { pub root: PathBuf, pub source: String }
#[derive(Clone, Debug, Deserialize, PartialEq)] pub struct KimiScope { pub root: PathBuf, pub index: PathBuf }

pub trait ScopeSource: Send + Sync { fn fetch(&self) -> Result<Scopes, CollectError>; }
pub struct HttpScopes { /* upstream + segredo; GET /internal/costs/scopes com prazo de 10 s */ }

pub enum Ready { Go, Warming { read: usize, total: usize } }
pub struct Collector { /* Index, Pricing, AreaMap, Progress, escopos atuais, rótulos, versão dos dados, travas */ }
impl Collector {
    pub fn new(index_dir: PathBuf, pricing_dir: PathBuf, area_file: PathBuf, scopes: Arc<dyn ScopeSource>) -> Collector;
    pub fn schedule_warmup(self: &Arc<Self>, delay: Duration);
    /// Regra de `costs_sources._pronto`: antes da primeira varredura → dispara e devolve Warming;
    /// `fresh` → varre esperando até 3 s pela vez (senão Warming); velha (> 30 s) → refaz atrás.
    pub fn prepare(self: &Arc<Self>, fresh: bool) -> Result<Ready, CollectError>;
    pub fn read_costs(&self, since: Option<&str>) -> Result<Vec<UsageRow>, CollectError>;   // `_ler_custos`
    pub fn read_usage(&self, since: Option<&str>) -> Result<(Vec<(UsoLinha, String)>, Vec<UsageRow>), CollectError>; // `_ler_uso`, conta junto
    pub fn label(&self, key: &str) -> Option<String>;     // rotulo_de_provedor
    pub fn labels_key(&self) -> Vec<(String, String)>;    // chave_rotulos
    pub fn data_version(&self) -> u64;                    // costs_cache._versao_dados
    pub fn pricing(&self) -> MutexGuard<'_, Pricing>;
    pub fn areas(&self) -> &AreaMap;
    pub fn repo(&self) -> Option<PathBuf>;
}
pub fn list_files(root: &Path, keep: impl Fn(&str) -> bool) -> Vec<PathBuf>;   // já criado na Task 6
pub fn rollout_owners(codex: &[CodexScope]) -> IndexMap<String, (CodexScope, Vec<PathBuf>)>;
pub enum CollectError { NoScopes, Index(IndexError) }  // qualquer um = repassar ao Python
```

- [ ] **Step 1: Testes que falham**

`crates/hangar-server/tests/costs_collect.rs`:

```rust
mod common;
use common::costs::*;
use hangar_server::costs::collect::*;
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
use std::time::Duration;

struct Fixed(Mutex<Result<Scopes, ()>>, AtomicUsize);
impl ScopeSource for Fixed {
    fn fetch(&self) -> Result<Scopes, CollectError> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.lock().unwrap().clone().map_err(|_| CollectError::NoScopes)
    }
}

fn scopes(base: &std::path::Path) -> Scopes {
    Scopes {
        claude: vec![ClaudeScope { root: base.join("claude/projects"), account: "anthropic:u-fixture".into(), label: "fixture@exemplo".into() }],
        codex: vec![CodexScope { home: base.join("codex"), account: format!("codex:{}", base.join("codex").display()), label: "Codex · default".into() }],
        pi: vec![PiScope { root: base.join("pi"), source: "pi".into() }],
        kimi: Some(KimiScope { root: base.join("kimi/sessions"), index: base.join("kimi/session_index.jsonl") }),
        repo: base.to_path_buf(),
    }
}

fn collector(base: &std::path::Path, src: Arc<Fixed>) -> Arc<Collector> {
    Arc::new(Collector::new(base.join("../idx"), base.join("pricing"), base.join("../sem-mapa.json"), src))
}

fn wait_ready(c: &Arc<Collector>) {
    for _ in 0..200 {
        if matches!(c.prepare(false).unwrap(), Ready::Go) { return }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("a varredura não terminou");
}

#[test]
fn first_request_warms_and_then_reads_all_sources() {
    let (_d, base) = fixtures_copy();
    let c = collector(&base, Arc::new(Fixed(Mutex::new(Ok(scopes(&base))), AtomicUsize::new(0))));
    assert!(matches!(c.prepare(false).unwrap(), Ready::Warming { .. }));
    wait_ready(&c);
    let rows = c.read_costs(None).unwrap();
    for source in ["claude", "codex", "pi", "kimi"] {
        assert!(rows.iter().any(|r| r.source == source), "{source}");
    }
    assert!(rows.iter().filter(|r| r.source == "kimi").all(|r| r.project == "/repo/k"), "projeto do Kimi vem do índice de sessões");
    assert_eq!(c.label("anthropic:u-fixture").as_deref(), Some("fixture@exemplo"));
}

#[test]
fn missing_scopes_are_an_error_not_an_empty_report() {
    let (_d, base) = fixtures_copy();
    let c = collector(&base, Arc::new(Fixed(Mutex::new(Err(())), AtomicUsize::new(0))));
    c.schedule_warmup(Duration::ZERO);
    std::thread::sleep(Duration::from_millis(200));
    assert!(matches!(c.prepare(true), Err(CollectError::NoScopes)));
}

#[test]
fn last_good_scopes_survive_a_python_hiccup() {
    let (_d, base) = fixtures_copy();
    let src = Arc::new(Fixed(Mutex::new(Ok(scopes(&base))), AtomicUsize::new(0)));
    let c = collector(&base, src.clone());
    c.prepare(false).unwrap();
    wait_ready(&c);
    *src.0.lock().unwrap() = Err(());
    assert!(matches!(c.prepare(true).unwrap(), Ready::Go));
    assert!(!c.read_costs(None).unwrap().is_empty());
}

#[test]
fn concurrent_fresh_requests_share_one_scan() {
    let (_d, base) = fixtures_copy();
    let src = Arc::new(Fixed(Mutex::new(Ok(scopes(&base))), AtomicUsize::new(0)));
    let c = collector(&base, src.clone());
    c.prepare(false).unwrap();
    wait_ready(&c);
    let before = src.1.load(Ordering::SeqCst);
    let hs: Vec<_> = (0..4).map(|_| { let c = c.clone(); std::thread::spawn(move || c.prepare(true).unwrap()) }).collect();
    for h in hs { assert!(matches!(h.join().unwrap(), Ready::Go | Ready::Warming { .. })) }
    assert!(src.1.load(Ordering::SeqCst) - before <= 4, "uma varredura por vez");
}

#[cfg(unix)]
#[test]
fn rollout_reachable_by_two_accounts_belongs_to_one_or_none() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a");
    std::fs::create_dir_all(a.join("sessions/x")).unwrap();
    std::fs::write(a.join("sessions/x/rollout-1.jsonl"), b"{}\n").unwrap();
    let b = d.path().join("b");
    std::fs::create_dir_all(b.join("sessions")).unwrap();
    std::os::unix::fs::symlink(a.join("sessions/x"), b.join("sessions/x")).unwrap();
    let mk = |h: &std::path::Path| CodexScope { home: h.to_path_buf(), account: format!("codex:{}", h.display()), label: String::new() };
    let owners = rollout_owners(&[mk(&a), mk(&b)]);
    let total: usize = owners.values().map(|(_, f)| f.len()).sum();
    assert_eq!(total, 1);
    assert!(owners.contains_key(&format!("codex:{}", a.display())));
}
```

- [ ] **Step 2: Rodar e ver falhar.**

- [ ] **Step 3: Implementar**

Porte de `costs_sources.py:574-742` e `:801-889`, com estas regras:
- `rollout_owners`: porte de `_rollouts_codex_por_conta` + `codex_contas.account_for_rollout`: enumera `sessions` e `archived_sessions` de cada conta (sem seguir link de pasta na listagem — `list_files`), canoniza (`std::fs::canonicalize`), dono = a única conta cujo `home` canônico contém o arquivo; zero ou mais de uma → fora. Escopo só para contas com arquivo.
- Escopos: `fetch()` a cada varredura; sucesso substitui os guardados e os rótulos (`label` do Claude e do Codex); falha com escopos guardados → usa os guardados e registra `tracing::warn!(code = "escopos_custos")`; falha sem nada guardado → `CollectError::NoScopes`.
- Uma varredura por vez (`Mutex` com `try_lock` em laço até o prazo); `Progress` zerado no início; `Warming { read, total }` = soma de todos os escopos.
- Leitura em paralelo: `rayon::ThreadPoolBuilder::new().num_threads(min(4, available_parallelism)).thread_name(|i| format!("custos-{i}"))` criado uma vez; a varredura inteira roda numa `std::thread` própria (nunca numa thread do tokio). A gravação no SQLite fica na thread da varredura.
- `forget_outside` com os escopos ativos no fim (`esquecer_fora`); conjunto de escopos mudou → sobe a versão dos dados.
- `read_costs` aplica as regras de leitura: Claude com provedor resolvido por modelo (`_linhas_claude_do_indice`, `:133-158`), Codex com `account_id` e provedor `openai` trocado pela identidade (`:723-729`), Kimi com projeto do `session_index` (`:512-517`).
- Pricing: `reload_if_changed()` a cada `prepare`.
- Aquecimento de boot: `schedule_warmup(Duration::from_secs(30))` chamado ao subir o servidor (Task 10).

- [ ] **Step 4: Rodar** — `cargo test ... --test costs_collect` → PASS.

- [ ] **Step 5: Revisão e commit**

```bash
git add crates/Cargo.toml crates/Cargo.lock crates/hangar-server/Cargo.toml crates/hangar-server/src/costs \
  crates/hangar-server/tests/costs_collect.rs
git commit -m "feat(server): cost collector with Python scopes, rollout ownership and warmup"
```

---

### Task 10: Relatório de custos, cotação e rotas `/api/costs` e `/api/cotacao`

**Files:**
- Create: `crates/hangar-server/src/costs/report_costs.rs`, `crates/hangar-server/src/costs/fx.rs`, `crates/hangar-server/src/costs_routes.rs`
- Modify: `crates/hangar-server/src/lib.rs` (`pub mod costs_routes;`), `crates/hangar-server/src/routes.rs` (`AppState` ganha `costs: Arc<Collector>`, `fx: Arc<Fx>`; rotas no `router`), `crates/hangar-server/src/main.rs` (cria o `Collector` com `HttpScopes` e chama `schedule_warmup(30 s)`)
- Test: `crates/hangar-server/tests/contract_costs_reports.rs`, `crates/hangar-server/tests/costs_routes.rs`

**Interfaces:**
- Produces:
  - `costs::report_costs::CostReport` e os structs `DimBucket`, `KindBucket`, `RateInfo`, `ComboRow`, `SessaoCusto`, `Applied` com `Serialize` na ordem de `models.py:375-575`; `pub const PERIODS: [(&str, i64); 4]` (`1d`,`7d`,`30d`,`90d`); `pub fn build(rows: Vec<UsageRow>, period: &str, now: LocalTs, pricing: &Pricing, label: &dyn Fn(&str) -> Option<String>) -> CostReport` (porte de `costs.montar`, `usd_brl: None`); `pub fn since(period: &str, now: LocalTs) -> Option<String>` (`desde` de `costs.report`); `pub fn row_cost(r: &UsageRow, p: &Pricing) -> Option<[f64; 4]>` (`_custo_da_linha`, usada pelo uso e pela sessão).
  - `costs::fx::Fx` com `pub fn usd_brl(&self) -> Option<f64>` (regra de `costs.usd_brl`: só a primeira chamada do processo espera; falha conta como tentativa; vencida devolve a última e busca atrás) e `Fx::with_fetch(f)` para teste.
  - `costs_routes::{costs, cotacao}` handlers e `pub(crate) fn warming(read, total) -> Response` (202 `{"aquecendo": true, "lidos": r, "total": t}`).
  - Cache de relatórios prontos: `costs::ReportCache` (até 8, chave = versão dos dados + geração do preço + assinatura das áreas + chave da rota), porte de `costs_cache.relatorio`.

- [ ] **Step 1: Testes que falham**

`crates/hangar-server/tests/contract_costs_reports.rs`:

```rust
mod common;
use common::costs::*;
use hangar_server::costs::collect::*;
use hangar_server::costs::py::LocalTs;
use hangar_server::costs::report_costs;
use std::sync::Arc;

pub fn ready_collector(base: &std::path::Path) -> Arc<Collector> {
    struct S(Scopes);
    impl ScopeSource for S { fn fetch(&self) -> Result<Scopes, CollectError> { Ok(self.0.clone()) } }
    let s = Scopes {
        claude: vec![ClaudeScope { root: base.join("claude/projects"), account: "anthropic:u-fixture".into(), label: "fixture@exemplo".into() }],
        codex: vec![CodexScope { home: base.join("codex"), account: format!("codex:{}", base.join("codex").display()), label: "Codex · default".into() }],
        pi: vec![PiScope { root: base.join("pi"), source: "pi".into() }],
        kimi: Some(KimiScope { root: base.join("kimi/sessions"), index: base.join("kimi/session_index.jsonl") }),
        repo: base.to_path_buf(),
    };
    let c = Arc::new(Collector::new(base.join("../idx"), base.join("pricing"), base.join("../sem-mapa.json"), Arc::new(S(s))));
    c.prepare(true).unwrap();
    while !matches!(c.prepare(false).unwrap(), Ready::Go) { std::thread::sleep(std::time::Duration::from_millis(20)) }
    c
}

/// O golden tem os caminhos do tmp do gerador; troca-os pelo `base` deste teste.
pub fn rebase(v: &serde_json::Value, golden_base_marker: &str, base: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&v.to_string().replace(golden_base_marker, &base.display().to_string())).unwrap()
}

#[test]
fn cost_reports_match_python() {
    let (_d, base) = fixtures_copy();
    let c = ready_collector(&base);
    let g: serde_json::Value = serde_json::from_slice(&std::fs::read(contract().join("golden/costs_reports.json")).unwrap()).unwrap();
    let now = LocalTs::from_iso(g["now"].as_str().unwrap()).unwrap();
    for period in ["all", "7d"] {
        let rows = c.read_costs(report_costs::since(period, now).as_deref()).unwrap();
        let got = report_costs::build(rows, period, now, &c.pricing(), &|k| c.label(k));
        let want = rebase(&g["costs"][period], "__BASE__", &base);
        assert_close(&serde_json::to_value(&got).unwrap(), &want, period);
    }
}
```

O gerador da Task 2 grava os relatórios com o caminho da cópia trocado por `__BASE__`; o `rebase` desfaz a troca.

`crates/hangar-server/tests/costs_routes.rs` — monta o `router` com um upstream falso (mesmo padrão de `tests/proxy.rs`: um axum local que responde `/internal/costs/scopes` com o JSON das fixtures e registra o que recebeu por repasse):

```rust
// Casos (um #[tokio::test] cada):
// 1. GET /api/costs com token do dono e índice frio → 202 {"aquecendo": true, "lidos": n, "total": m}; depois → 200
//    com "applied": {"period": "all"} e "usd_brl" do Fx falso.
// 2. GET /api/costs?period=nada → applied.period == "all" (não 422).
// 3. GET /api/costs sem token → repassado ao upstream (o falso devolve 401 e a rota devolve 401).
// 4. Upstream sem /internal/costs/scopes (404) → GET /api/costs repassado ao upstream, nunca 200 vazio.
// 5. Pasta do índice sem permissão de escrita (chmod 0o500, só unix) → repassado ao upstream.
// 6. GET /api/cotacao → {"usd_brl": 5.25} do Fx falso; POST /api/cotacao → repassado.
// 7. Corpo do 200 com Accept-Encoding: gzip → Content-Encoding: gzip (reusa maybe_gzip) e CORS igual ao /history.
```

Escreva os sete como funções completas seguindo `tests/proxy.rs` (servidor em `127.0.0.1:0`, `reqwest` com `Authorization: Bearer <token>`), afirmando status, corpo e, nos de repasse, que o upstream recebeu o pedido.

- [ ] **Step 2: Rodar e ver falhar.**

- [ ] **Step 3: Implementar**

- `report_costs.rs`: porte de `costs.py:27-256`. Regras: janela anterior calculada antes do corte e `None` com menos de 1/3 dos dias; `custos` indexado depois do corte; identidade da sessão = `pyjson::dumps(["source", account_id || provider, session_id, subagente], false)` (separadores do Python); `by_*` ordenados por `(-cost, key)` estável e `by_day` por chave decrescente; `combos` ordenados por `(dia, -cost)` e `session_ids` ordenados; `sem_tarifa` sem `IGNORADOS`; `rates` por modelo; top 100 sessões com subagente somado no pai (`split("/subagents/")`) e modelo de maior custo (`max` com empate = primeiro inserido, como o `max` do Python sobre dict); `equivalente_cobrado` truncado com `as i64` (o `int()` do Python).
- `fx.rs`: o `proxy::client()` só fala HTTP e a cotação é HTTPS. Promova o `reqwest` do workspace (`=0.13.4`, hoje só em `[dev-dependencies]`) a dependência normal do `hangar-server`, acrescentando a feature de TLS com rustls que a documentação da 0.13.4 nomeia (confira em docs.rs antes; não troque a versão). Chamada bloqueante numa thread própria (`reqwest::blocking` exige a feature `blocking`; alternativa: `tokio::spawn` com o cliente async). URL `https://economia.awesomeapi.com.br/json/last/USD-BRL`, prazo 3 s, `USDBRL.bid` como float; erro vira `tracing::warn!(code = "cotacao")`.
- `costs_routes.rs`: para cada rota, `gate()`; sem dono ou método ≠ GET → `pass`. Com dono: `spawn_blocking` com `collector.prepare(fresh)`; `Warming` → `warming()`; erro → `pass` com `tracing::warn!(code = ...)`; `Go` → relatório do cache ou `build`, `usd_brl` aplicado por cima (a cotação tem ciclo próprio, como `model_copy(update=...)`), `serde_json::to_vec`, `maybe_gzip`, `cors`. `period` fora de `PERIODS` e ≠ `all` vira `all`; `fresco` aceita o que o FastAPI aceita para `bool` (`1/0/true/false/on/off/yes/no`, sem diferenciar caixa); valor que não for bool → `pass` (o FastAPI responde 422).
- `routes.rs::router`: `.route("/api/costs", get(costs_routes::costs).fallback(pass_any))`, idem `/api/cotacao`. `AppState::with_terminal_pool` recebe o `Collector` e o `Fx` (crie `AppState::with_parts` se mudar a assinatura quebrar muitos testes; mantenha `new` montando os padrões: `index::default_dir()`, `pricing::default_dir()`, `areas::default_map_file()`, `HttpScopes`).
- `main.rs`: depois de montar o estado, `state.costs.schedule_warmup(Duration::from_secs(30))`.

- [ ] **Step 4: Rodar** — `cargo test ... --test contract_costs_reports --test costs_routes --test proxy` → PASS.

- [ ] **Step 5: Revisão e commit**

```bash
git add crates/hangar-server/src crates/hangar-server/Cargo.toml crates/Cargo.toml crates/Cargo.lock \
  crates/hangar-server/tests/contract_costs_reports.rs crates/hangar-server/tests/costs_routes.rs \
  backend/tests/fixtures/contract/gen_costs.py backend/tests/fixtures/contract/golden/costs_reports.json
git commit -m "feat(server): serve /api/costs and /api/cotacao from Rust with Python fallback"
```

---

### Task 11: Relatório de uso, origens de skill e rota `/api/uso`

**Files:**
- Create: `crates/hangar-server/src/costs/report_uso.rs`, `crates/hangar-server/src/costs/origins.rs`
- Modify: `crates/hangar-server/src/costs_routes.rs`, `crates/hangar-server/src/routes.rs`
- Test: `crates/hangar-server/tests/contract_costs_reports.rs`, `crates/hangar-server/tests/costs_routes.rs`

**Interfaces:**
- Consumes: `Collector::read_usage`, `report_costs::row_cost`, `Pricing`.
- Produces: `report_uso::UsoReport`/`UsoBucket` (ordem de `models.py:482-545`); `pub struct UsoFilters { pub conta: Vec<String>, pub projeto: Vec<String>, pub modelo: Vec<String>, pub plugin: Vec<String>, pub foco: Option<String> }`; `pub fn build(uso: &[(UsoLinha, String)], tokens: &[UsageRow], period: &str, now: LocalTs, f: &UsoFilters, origins: Option<&IndexMap<String, String>>, pricing: &Pricing, label: &dyn Fn(&str) -> Option<String>) -> UsoReport`; `origins::Origins` com `pub fn recent(&self) -> (u64, IndexMap<String, String>)` (porte de `_origens_recentes`: primeira chamada varre e espera; depois confere mtimes a cada 30 s numa thread e só varre se mudou; o `u64` é o instante da última mudança, que entra na chave do cache) e `pub fn scan(home: &Path, repo: &Path) -> IndexMap<String, String>` (`origens_de_skill`).

- [ ] **Step 1: Testes que falham** — em `contract_costs_reports.rs`:

```rust
use hangar_server::costs::report_uso::{self, UsoFilters};

#[test]
fn usage_reports_match_python() {
    let (_d, base) = fixtures_copy();
    let c = ready_collector(&base);
    let g: serde_json::Value = serde_json::from_slice(&std::fs::read(contract().join("golden/costs_reports.json")).unwrap()).unwrap();
    let now = LocalTs::from_iso(g["now"].as_str().unwrap()).unwrap();
    let origins: indexmap::IndexMap<String, String> =
        [("brainstorming", "superpowers"), ("minha-skill", "@pessoal")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let (uso, tokens) = c.read_usage(None).unwrap();
    let f = |conta: &[&str], projeto: &[&str], foco: Option<&str>| UsoFilters {
        conta: conta.iter().map(|s| s.to_string()).collect(), projeto: projeto.iter().map(|s| s.to_string()).collect(),
        modelo: vec![], plugin: vec![], foco: foco.map(str::to_string) };
    for (key, filters) in [("all", f(&[], &[], None)), ("conta", f(&["anthropic:u-fixture"], &[], None)),
                           ("projeto", f(&[], &["/repo/a"], None)), ("foco_skill", f(&[], &[], Some("brainstorming"))),
                           ("foco_area", f(&[], &[], Some("back")))] {
        let got = report_uso::build(&uso, &tokens, "all", now, &filters, Some(&origins), &c.pricing(), &|k| c.label(k));
        let want = rebase(&g["uso"][key], "__BASE__", &base);
        assert_close(&serde_json::to_value(&got).unwrap(), &want, key);
    }
}
```

Em `costs_routes.rs`, acrescentar: `/api/uso?conta=a&conta=&projeto=/repo/a` (vazio descartado, filtros ecoados em `conta`/`projeto`), `foco` vazio vira `null`, 202 com índice frio, repasse sem token.

- [ ] **Step 2: Rodar e ver falhar.**

- [ ] **Step 3: Implementar** — porte de `uso_report.py:33-544`. Regras:
- Corte por `dia >= corte` (texto) nas linhas de uso e por data local nos tokens; custo de agente do transcript filho (`/subagents/agent-`), refeito com filtro de conta/projeto quando há; seletores (`by_conta`, `by_projeto`, `by_modelo`) somam antes dos filtros de dimensão.
- `_custos_reais` monta a linha com provedor `openai` para `codex` e `anthropic` para o resto e passa por `row_cost`.
- `_conta_no_total`, `_TIPOS_CONTADOS`, réguas 4 e 2,5 (`int(x / regua)` = truncar), ordenações de `_ordenar`, `_dimension`, `_por_dia` (estáveis), rótulo de `by_area_dia` = parte depois de `|`.
- `foco`: série diária do item; `foco` que é uma área usa `_somar_area`.
- Rota: chave do cache = `("uso", period, dia de hoje, geração do preço, rótulos, instante das origens, filtros ordenados por nome)`; filtros vazios removidos; `period` inválido → `all`.

- [ ] **Step 4: Rodar** — PASS.

- [ ] **Step 5: Revisão e commit**

```bash
git add crates/hangar-server/src crates/hangar-server/tests/contract_costs_reports.rs crates/hangar-server/tests/costs_routes.rs
git commit -m "feat(server): serve /api/uso from Rust with skill origins and Python fallback"
```

---

### Task 12: Custo de uma sessão Codex (`/api/sessions/{name}/cost`)

**Files:**
- Modify: `crates/hangar-server/src/costs_routes.rs`, `crates/hangar-server/src/routes.rs`
- Test: `crates/hangar-server/tests/costs_routes.rs`

**Interfaces:**
- Consumes: `fetch_info` (`routes.rs`, já existe; devolve `provider` e `jsonl`), `codex::session_rows`, `row_cost`.
- Produces: handler `session_cost`; resposta `{"cost_usd": f64 | null, "missing_models": [..], "has_usage": bool}` (porte de `session_cost.py`).

- [ ] **Step 1: Testes que falham** — em `costs_routes.rs`, com o upstream falso respondendo `/internal/sessions/{name}/info`:

```rust
// 1. info {"provider": "codex", "jsonl": <rollout-c1 da cópia>} → 200 com cost_usd > 0, missing_models [] e has_usage true;
//    o valor bate com a soma de row_cost das linhas agrupadas por (modelo, contexto longo).
// 2. info {"provider": "claude", ...} → 404 com o corpo exato do Python (mensagens.erro, api.py:3122):
//    {"detail": {"code": "erro_sessao_inexistente", "params": {}, "msg": "Codex session not found"}}
// 3. info 404 (sessão inexistente) → mesmo 404.
// 4. jsonl apontando para arquivo apagado →
//    {"detail": {"code": "erro_sessao_inexistente", "params": {}, "msg": "Codex rollout not found"}}
// 5. Índice indisponível → repassado ao upstream.
```

Mesmas regras de escrita da Task 10 (funções completas, afirmando status e corpo).

- [ ] **Step 2: Rodar e ver falhar.**
- [ ] **Step 3: Implementar** — `info` sem cache (como `history`, sessão recriada não pode ver a morta); provider ≠ `codex` ou sem `jsonl` → 404 com o `detail` do Python; `canonicalize` falhou → 404 do rollout; linhas vazias de tokens puladas; agrupamento `(model, codex_long_context)` em ordem de inserção; `cost_usd = None` sem linhas ou com modelo sem tarifa; `missing_models` ordenado; `session_rows` roda em `spawn_blocking`.
- [ ] **Step 4: Rodar** — PASS.
- [ ] **Step 5: Revisão e commit**

```bash
git add crates/hangar-server/src crates/hangar-server/tests/costs_routes.rs
git commit -m "feat(server): Codex session cost from the Rust index"
```

---

### Task 13: Prova com os dados desta máquina, medidas e documentação

**Files:**
- Create: `crates/hangar-server/examples/custos.rs`, `scripts/comparar-custos.py`
- Modify: `docs/migracao-rust/README.md` (linha da parte 3), `docs/decisoes/plataforma.md` (entrada nova), `CLAUDE.md` (uma frase na regra "A porta 8765 é do `hangar-server`…")

**Interfaces:**
- Consumes: tudo acima.
- Produces: `cargo run --release --example custos -- --scopes <json> --index <pasta> --now <iso> [--period all]` imprime `{"costs": CostReport, "uso": UsoReport, "scan_s": f64, "peak_rss_mb": u64}`; `scripts/comparar-custos.py` gera os escopos com `costs_sources.scopes_for_rust()`, roda o Python avulso com índice descartável (o mesmo isolamento de `analise.md`: `costs_cache._CACHE_DIR` trocado antes de qualquer leitura) e o exemplo Rust com outro índice descartável, e compara inteiros e chaves exatos e frações com erro relativo de 1e-9.

- [ ] **Step 1: Escrever o exemplo e o script**

`scripts/comparar-custos.py`:

```python
"""Compara os relatórios de custo/uso do Python e do Rust sobre os transcripts reais, só leitura.

Uso, da raiz: cd backend && uv run python ../scripts/comparar-custos.py
Nunca toca o índice do backend vivo: os dois lados usam pastas temporárias.
"""
import json
import subprocess
import sys
import tempfile
from datetime import datetime
from pathlib import Path

RAIZ = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RAIZ / "backend"))
from app import costs  # noqa: E402
from app import costs_cache  # noqa: E402

with tempfile.TemporaryDirectory() as tmp:
    costs_cache._CACHE_DIR = Path(tmp) / "py"
    assert "hangar-custos" not in str(costs_cache._CACHE_DIR)
    from app import costs_sources, uso_report  # noqa: E402

    now = datetime.now(costs_sources.LOCAL).replace(microsecond=0)
    escopos = Path(tmp) / "scopes.json"
    escopos.write_text(json.dumps(costs_sources.scopes_for_rust()), encoding="utf-8")
    costs_sources.sincronizar_tudo()
    costs.usd_brl = lambda: None
    py = {"costs": costs.montar(costs_sources._ler_custos(), period="all", now=now).model_dump(mode="json"),
          "uso": uso_report.montar(*costs_sources._ler_uso(None), period="all", now=now,
                                   origens=uso_report._origens_recentes()[1]).model_dump(mode="json")}
    rs = json.loads(subprocess.run(
        ["cargo", "run", "-q", "--release", "--manifest-path", str(RAIZ / "crates/Cargo.toml"), "-p", "hangar-server",
         "--example", "custos", "--", "--scopes", str(escopos), "--index", str(Path(tmp) / "rs"), "--now", now.isoformat()],
        check=True, capture_output=True, text=True).stdout)


def comparar(a, b, onde, erros):
    if isinstance(a, float) or isinstance(b, float):
        if abs(a - b) > 1e-9 * max(abs(a), abs(b), 1.0):
            erros.append(f"{onde}: {a} != {b}")
    elif isinstance(a, dict) and isinstance(b, dict):
        if list(a) != list(b):
            erros.append(f"{onde}: chaves {list(a)} != {list(b)}")
        for k in a:
            if k in b:
                comparar(a[k], b[k], f"{onde}.{k}", erros)
    elif isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            erros.append(f"{onde}: {len(a)} itens != {len(b)}")
        for i, (x, y) in enumerate(zip(a, b)):
            comparar(x, y, f"{onde}[{i}]", erros)
    elif a != b:
        erros.append(f"{onde}: {a!r} != {b!r}")


erros: list[str] = []
for parte in ("costs", "uso"):
    comparar(py[parte], rs[parte], parte, erros)
print(f"varredura Rust {rs['scan_s']:.2f} s, pico {rs['peak_rss_mb']} MB; diferenças: {len(erros)}")
for e in erros[:40]:
    print(" ", e)
sys.exit(1 if erros else 0)
```

`crates/hangar-server/examples/custos.rs`: lê os argumentos, monta `Collector` com uma `ScopeSource` que devolve o JSON lido, mede o tempo de `prepare(true)` até `Ready::Go`, monta os dois relatórios (origens por `origins::scan(home, repo)`) e imprime o JSON; pico de memória por `/proc/self/status` (`VmHWM`) no Linux, `0` nos outros.

- [ ] **Step 2: Rodar contra esta máquina**

Run: `cd backend && uv run python ../scripts/comparar-custos.py`
Expected: `diferenças: 0`, varredura abaixo de 5 s, pico abaixo de 100 MB. Diferença encontrada → é bug de port: volte à Task do leitor ou relatório, acrescente ao golden um transcript sintético que reproduza o caso (nunca conversa real) e corrija.

- [ ] **Step 3: Documentar**

- `docs/migracao-rust/README.md`: linha da parte 3 → "Feita na branch `hangar-server-parte3`, contrato versão 5 (depende da 2B); falta uso real".
- `docs/decisoes/plataforma.md`: entrada "Custos e uso no hangar-server" com as medidas antes (de `analise.md`) e depois (Step 2), o porquê de cotas/stats ficarem no Python e o índice próprio.
- `CLAUDE.md`, regra da porta 8765: acrescentar "e `/api/costs`, `/api/uso`, `/api/cotacao` e o custo de sessão Codex, com índice próprio (`custos-rust.sqlite3`); cotas ficam no Python".

- [ ] **Step 4: Revisão e commit**

```bash
git add crates/hangar-server/examples/custos.rs scripts/comparar-custos.py docs/migracao-rust/README.md \
  docs/decisoes/plataforma.md CLAUDE.md
git commit -m "docs(migracao-rust): part 3 measured against real data and documented"
```

- [ ] **Step 5: Uso real com o dono (verificação manual)**

Roteiro da spec ("Uso real com o dono, no fim"), feito pelo dono no canal de testes depois do push autorizado: Custos e Uso no web, card no celular e no nativo; "Atualizar dados"; filtro e clique num item do Uso; custo de sessão Codex; apagar `custos-rust.sqlite3` e ver o "aquecendo" durar segundos; `CP_RUST_SERVER=0`.
