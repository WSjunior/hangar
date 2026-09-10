"""Bloqueia edição direta do código pelos processos da sessão.

Não isola o host: aliases por /proc e serviços externos (tmux, API, MCP) ficam fora da proteção.
"""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import sys


def prepare(cwd: str, runtime_dirs: tuple[str, ...] = ()) -> list[str]:
    if sys.platform != "linux":
        raise ValueError("read-only exige Linux com bubblewrap (bwrap)")
    bwrap = shutil.which("bwrap")
    if not bwrap:
        raise ValueError("read-only exige bubblewrap: instale bwrap antes de criar a sessão")
    try:
        root = Path(cwd).expanduser().resolve(strict=True)
        if not root.is_dir():
            raise ValueError("read-only exige um diretório de trabalho existente")
        paths = {root}
        git = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "--path-format=absolute",
             "--git-dir", "--git-common-dir", "--show-toplevel"],
            env={key: value for key, value in os.environ.items() if not key.startswith("GIT_")},
            capture_output=True, text=True, timeout=5,
        )
        if git.returncode == 0:
            paths.update(Path(path).resolve(strict=True) for path in git.stdout.splitlines())
        elif any((parent / ".git").exists() for parent in (root, *root.parents)):
            raise ValueError("read-only não conseguiu localizar os metadados Git")
        if any(Path.home().resolve().is_relative_to(path) for path in paths):
            raise ValueError("read-only não pode proteger a HOME inteira ou um diretório acima dela")
        runtimes = [Path.home() / name for name in
                    (".claude", ".codex", ".pi", ".omp", ".kimi-code", ".hangar")]
        runtimes += [Path(path).expanduser() for path in runtime_dirs if path]
        runtimes += [Path(os.environ[key]).expanduser() for key in
                     ("CLAUDE_CONFIG_DIR", "CODEX_HOME", "PI_CODING_AGENT_DIR",
                      "PI_CODING_AGENT_SESSION_DIR", "PI_CONFIG_DIR", "KIMI_CODE_HOME")
                     if os.environ.get(key)]
        if any(runtime.resolve().is_relative_to(path) for runtime in runtimes for path in paths):
            raise ValueError("read-only recusado: o código protegido contém o runtime de um agente")
        # O bind comum desabilita dispositivos: Git precisa de /dev/null e a TUI do terminal.
        argv = [bwrap, "--bind", "/", "/", "--dev-bind", "/dev", "/dev",
                "--setenv", "HANGAR_ORQ_READ_ONLY", "1"]
        for path in sorted(paths, key=lambda path: (len(path.parts), str(path))):
            argv += ["--ro-bind", str(path), str(path)]
        argv += ["--"]
        probe = subprocess.run([*argv, "/bin/true"], capture_output=True, text=True, timeout=5)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise ValueError(f"read-only não pôde preparar a proteção: {exc}") from None
    if probe.returncode:
        raise ValueError(f"read-only recusado pelo bubblewrap: {probe.stderr.strip()}")
    return argv
