"""Os arquivos que o `hooks.json` do Codex aponta e o importador nativo não trouxe.

O importador do Codex reescreve os comandos de `~/.claude/hooks/<x>` para `~/.codex/hooks/<x>` e
COPIA o arquivo — mas só quando ele é um arquivo de verdade. Hook que é symlink (o caso de quem
versiona hooks num repo e linka: `~/.claude/hooks/x.py -> ~/Projetos/skills/hooks/x.py`) ele pula,
e o comando reescrito fica apontando pro vazio. Medido em 07/09/2026 numa HOME descartável, contra
o codex-cli 0.153.4: `real.sh` foi copiado, `linkado.sh` não, e os DOIS comandos foram reescritos.

O estrago não é cosmético: hook de `PreToolUse` que falha BLOQUEIA a ferramenta, então a sessão
Codex fica inutilizável com "python3: can't open file ... No such file or directory".

Aqui a integração fecha esse buraco: para cada caminho sob `<codex>/hooks/` citado no `hooks.json`
que não existe, se houver um arquivo de mesmo nome em `~/.claude/hooks/`, cria um SYMLINK para o
alvo real. Symlink, e não cópia, pelo mesmo motivo do `~/.codex/skills`: uma fonte só — editar o
script continua valendo para os dois agentes no mesmo instante, sem cópia para envelhecer.
"""
from __future__ import annotations

import json
import os
import re
from pathlib import Path

# Caminho citado num comando de shell: entre aspas (simples ou duplas) ou solto até o espaço. O
# comando é escrito pelo Codex, não pelo usuário, então este reconhecimento é sobre um formato
# conhecido — e o que não casar simplesmente não é tocado.
_CITADO = re.compile(r"""'([^']+)'|"([^"]+)"|(\S+)""")


def _caminhos_do_comando(comando: str) -> list[str]:
    return [g for m in _CITADO.finditer(comando) for g in m.groups() if g]


def caminhos_citados(doc: dict) -> list[str]:
    """Todo caminho que aparece nos comandos do documento de hooks, sem repetir e na ordem."""
    vistos: list[str] = []
    eventos = doc.get("hooks")
    if not isinstance(eventos, dict):
        return vistos
    for grupos in eventos.values():
        if not isinstance(grupos, list):
            continue
        for grupo in grupos:
            if not isinstance(grupo, dict) or not isinstance(grupo.get("hooks"), list):
                continue
            for hook in grupo["hooks"]:
                if not isinstance(hook, dict):
                    continue
                comando = hook.get("command")
                if not isinstance(comando, str):
                    continue
                for c in _caminhos_do_comando(comando):
                    if c not in vistos:
                        vistos.append(c)
    return vistos


def faltantes(doc: dict, codex_home: Path) -> list[Path]:
    """Caminhos citados que moram sob `<codex>/hooks/` e não existem no disco."""
    pasta = (codex_home / "hooks").resolve()
    out: list[Path] = []
    for bruto in caminhos_citados(doc):
        p = Path(bruto)
        if not p.is_absolute():
            continue
        try:
            dentro = p.resolve().parent == pasta or p.parent.resolve(strict=False) == pasta
        except OSError:
            continue
        # `exists()` segue symlink: link pendurado conta como faltante, que é o que ele é na prática.
        if dentro and not p.exists():
            out.append(p)
    return out


def materializar(codex_home: Path, home: Path, doc: dict | None = None) -> tuple[list[str], list[str]]:
    """Cria os que faltam apontando pro hook do Claude. Devolve (criados, não resolvidos).

    Não sobrescreve nada: só age onde não há arquivo. Um link pendurado (alvo apagado) é refeito,
    porque ali não há conteúdo de ninguém a preservar.
    """
    if doc is None:
        try:
            doc = json.loads((codex_home / "hooks.json").read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return [], []
    if not isinstance(doc, dict):
        return [], []
    origem = home / ".claude" / "hooks"
    criados: list[str] = []
    orfaos: list[str] = []
    for destino in faltantes(doc, codex_home):
        fonte = origem / destino.name
        if not fonte.exists():           # segue o symlink: fonte quebrada também não serve
            orfaos.append(destino.name)
            continue
        alvo = fonte.resolve()           # o arquivo REAL, não o link do meio do caminho
        try:
            destino.parent.mkdir(parents=True, exist_ok=True)
            if destino.is_symlink():     # link pendurado: só ele é substituído
                destino.unlink()
            # No Windows symlink exige privilégio; cair na cópia lá é melhor que não ter o hook.
            if os.name == "nt":
                destino.write_bytes(alvo.read_bytes())
            else:
                destino.symlink_to(alvo)
        except OSError:
            orfaos.append(destino.name)
            continue
        criados.append(destino.name)
    return criados, orfaos
