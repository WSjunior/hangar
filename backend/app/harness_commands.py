"""Comando oficial de instalação de cada harness, só com a biblioteca padrão.

Uma tabela para dois leitores: o botão do painel de Harnesses (`app/harness_install.py`) e os
instaladores (`install.sh`/`install.ps1`), que rodam `python -m app.harness_commands <cli>` antes de
o backend existir de fato — por isso nada aqui importa o resto do app.

Só comando conferido no site do fornecedor entra em `COMMANDS`. Chutar nome de pacote instala
software errado: `oh-my-pi` no npm não é o omp, `kimi-code` no npm é um proxy de terceiro que roda o
`claude` por baixo, e `kimi` é uma biblioteca de máquina de estados. Harness sem comando conferido
PARA ESTE SISTEMA fica com `None` e a pessoa recebe o link de `MANUAL`.

O Claude Code não está aqui: o instalador já o instala pelo caminho próprio (no Windows ele
precisa do conserto de PATH que o `Instale-ClaudeCode` faz).
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys

# Constante de módulo, e não `os.name` lido na hora: `monkeypatch.setattr(os, "name", "nt")` leva o
# `pathlib` junto e estoura no primeiro `Path(...)`.
IS_WINDOWS = os.name == "nt"


def _ps(line: str) -> list[str]:
    return ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", line]


def _sh(line: str) -> list[str]:
    """`pipefail` nas FLAGS do bash: sem ele um `curl … | sh` sai 0 com o `curl` falhando."""
    return ["bash", "-o", "pipefail", "-c", line]


# Conferidos na documentação de cada fornecedor em 07/09/2026.
COMMANDS: dict[str, list[str] | None] = {
    "codex": ["npm", "install", "-g", "@openai/codex"],
    "pi": ["npm", "install", "-g", "--ignore-scripts", "@earendil-works/pi-coding-agent"],
    "omp": (_ps("irm https://omp.sh/install.ps1 | iex") if IS_WINDOWS
            else _sh("curl -fsSL https://omp.sh/install | sh")),
    "kimi": (None if IS_WINDOWS
             else _sh("curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash")),
}

MANUAL = {
    "codex": "https://github.com/openai/codex",
    "pi": "https://pi.dev/docs/latest",
    "omp": "https://github.com/can1357/oh-my-pi",
    "kimi": "https://kimi.com/code",
}


def display(argv: list[str]) -> str:
    """O comando como a pessoa lê: o do fornecedor, não o embrulho que o roda."""
    return argv[-1] if argv[0] in ("bash", "powershell") else " ".join(argv)


def resolve(argv: list[str]) -> list[str]:
    """O CAMINHO do executável: no Windows o `CreateProcess` não aplica PATHEXT (`npm` é `npm.cmd`)."""
    exe = shutil.which(argv[0])
    if not exe:
        raise ValueError(f"{argv[0]} não está nesta máquina")
    return [exe, *argv[1:]]


def main(args: list[str] | None = None) -> int:
    """0 = o comando do fornecedor saiu 0; 2 = sem comando para este sistema; 3 = falta a ferramenta
    que o roda (npm); 64 = uso errado. Outro número é o código do próprio comando."""
    args = sys.argv[1:] if args is None else args
    if len(args) != 1 or args[0] not in MANUAL:
        print(f"uso: python -m app.harness_commands <{'|'.join(MANUAL)}>", file=sys.stderr)
        return 64
    cli = args[0]
    argv = COMMANDS[cli]
    if argv is None:
        print(f"{cli}: sem instalador conferido para este sistema; veja {MANUAL[cli]}", file=sys.stderr)
        return 2
    try:
        full = resolve(argv)
    except ValueError as e:
        print(f"{cli}: {e}", file=sys.stderr)
        return 3
    print(f"$ {display(argv)}", flush=True)
    return subprocess.run(full).returncode


if __name__ == "__main__":
    sys.exit(main())
