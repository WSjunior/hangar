"""Worktrees: onde a sessão está de verdade, a situação de cada worktree e a remoção segura."""
import json
import logging
import os
import re
import threading
from dataclasses import dataclass
from pathlib import Path

from app.git_ops import head_info

_log = logging.getLogger("hangar.worktrees")

REMOVED_FILE = Path.home() / ".hangar" / "worktrees-removidas.json"
_TAIL = 256 * 1024


@dataclass
class Location:
    branch: str | None
    worktree: bool
    worktree_path: str | None
    worktree_gone: bool


def repo_root_of(path: str | None) -> str | None:
    """Sobe até achar `.git` (pasta ou arquivo), sem subprocesso: roda por sessão na listagem."""
    if not path:
        return None
    p = Path(path)
    for d in (p, *p.parents):
        if (d / ".git").exists():
            return str(d)
    return None


def main_repo_of(root: str) -> str:
    """A pasta principal: ela mesma, ou a dona da worktree (`gitdir: <repo>/.git/worktrees/<n>`)."""
    dot = Path(root, ".git")
    try:
        if dot.is_file():
            txt = dot.read_text(encoding="utf-8", errors="replace").strip()
            if txt.startswith("gitdir: "):
                # normpath: com `worktree.useRelativePaths` o ponteiro vem com `..`.
                gitdir = Path(os.path.normpath(os.path.join(root, txt[len("gitdir: "):])))
                if gitdir.parent.name == "worktrees":
                    return str(gitdir.parent.parent.parent)
    except OSError:
        pass
    return root


def worktree_paths(main: str) -> list[str]:
    """As worktrees ligadas ao repo, de `.git/worktrees/*/gitdir` (aponta pro `.git` delas)."""
    out: list[str] = []
    try:
        entries = sorted(Path(main, ".git", "worktrees").iterdir())
    except OSError:
        return out
    for e in entries:
        try:
            g = (e / "gitdir").read_text(encoding="utf-8", errors="replace").strip()
        except OSError:
            continue
        if g:
            out.append(os.path.dirname(os.path.normpath(os.path.join(e, g))))
    return out


_cache: dict[str, tuple[tuple[int, int], object]] = {}
_cache_lock = threading.Lock()


def _cached(path: str, read):
    try:
        st = os.stat(path)
    except OSError:
        return None
    key = (st.st_mtime_ns, st.st_size)
    with _cache_lock:
        hit = _cache.get(path)
        if hit and hit[0] == key:
            return hit[1]
    value = read(path)
    with _cache_lock:
        _cache[path] = (key, value)
    return value


def _tail_lines(path: str) -> list[bytes]:
    with open(path, "rb") as fh:
        fh.seek(0, os.SEEK_END)
        size = fh.tell()
        fh.seek(max(0, size - _TAIL))
        lines = fh.read().split(b"\n")
    return lines[1:] if size > _TAIL else lines   # a primeira veio cortada


def claude_cwd(jsonl: str) -> str | None:
    """`cwd` da última linha que o tem: o Claude grava a pasta em cada linha, e ela muda no
    `EnterWorktree` e no `cd`."""
    def read(p: str) -> str | None:
        for raw in reversed(_tail_lines(p)):
            if b'"cwd"' not in raw:
                continue
            try:
                cwd = json.loads(raw).get("cwd")
            except (ValueError, AttributeError):
                continue
            if isinstance(cwd, str) and cwd:
                return cwd
        return None
    return _cached(jsonl, read)


_WORKDIR_RE = re.compile(r'"?workdir"?\s*:\s*"(/[^"]+)"')
_CD_RE = re.compile(r'"?cmd"?\s*:\s*"\s*cd\s+(/[^\s&;"]+)')
_PATCH_RE = re.compile(r'\*\*\* (?:Add|Update|Delete) File: (/[^\s\\"]+)')


def _codex_paths(rollout: str) -> list[tuple[str, bool]]:
    """(caminho, é pasta?) citados pelos comandos do Codex, da chamada mais recente para a mais
    antiga. O Codex não troca de pasta: trabalha numa worktree por `workdir`, `cd X &&` ou patch
    com caminho absoluto. Dentro de uma chamada, `cd` vence `workdir` (o comando roda onde entrou),
    e os dois vencem o arquivo do patch."""
    def read(p: str) -> list[tuple[str, bool]]:
        out: list[tuple[str, bool]] = []
        for raw in reversed(_tail_lines(p)):
            if b"_call" not in raw:   # function_call e custom_tool_call
                continue
            try:
                line = json.loads(raw)
            except ValueError:
                continue
            payload = line.get("payload") if isinstance(line, dict) else None
            if not isinstance(payload, dict):
                continue
            if payload.get("type") not in ("function_call", "custom_tool_call"):
                continue
            text = payload.get("arguments") or payload.get("input") or ""
            if not isinstance(text, str):
                continue
            for rx, is_dir in ((_CD_RE, True), (_WORKDIR_RE, True), (_PATCH_RE, False)):
                out.extend((m.group(1), is_dir) for m in reversed(list(rx.finditer(text))))
            if len(out) >= 50:
                break
        return out
    return _cached(rollout, read) or []


def _owner(path: str, candidates: list[str]) -> str | None:
    best = None
    for c in candidates:
        if (path == c or path.startswith(c.rstrip("/") + "/")) and (best is None or len(c) > len(best)):
            best = c
    return best


def codex_cwd(cwd: str, rollout: str) -> str | None:
    """A worktree (ou a principal) do MESMO repo onde o último comando rodou; outro repo não conta."""
    root = repo_root_of(cwd)
    if not root:
        return None
    main = main_repo_of(root)
    gone = [k for k, v in removed().items() if v == main]
    candidates = [main, *worktree_paths(main), *gone]
    for p, is_dir in _codex_paths(rollout):
        if os.path.exists(p):
            owner = _owner(p, candidates)
            if owner:
                return owner
        elif is_dir and _of_this_repo(p, main, gone):
            # Pasta que sumiu: a worktree foi removida. Volta como está para o `locate` marcar
            # "apagada". Arquivo de patch inexistente não conta (um "Delete File" é legítimo).
            return _owner(p, gone) or p
    return None


def _of_this_repo(path: str, main: str, gone: list[str]) -> bool:
    """Pasta sumida só conta se for deste repo: dentro dele, de uma worktree removida dele ou de
    uma irmã no padrão do Hangar (`<repo>-<x>`). Pasta sumida de outro repo não diz nada."""
    if _owner(path, [main, *gone]):
        return True
    parent, name = os.path.split(main.rstrip("/"))
    prefix = parent.rstrip("/") + "/"
    if not path.startswith(prefix):
        return False
    return path[len(prefix):].split("/", 1)[0].startswith(name + "-")


def removed() -> dict[str, str]:
    try:
        data = json.loads(REMOVED_FILE.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}
    return data if isinstance(data, dict) else {}


def locate(provider: str, cwd: str | None, jsonl: str | None) -> Location:
    real = None
    try:
        if jsonl and provider == "claude":
            real = claude_cwd(jsonl)
        elif jsonl and provider == "codex" and cwd:
            real = codex_cwd(cwd, jsonl)
    except Exception as e:   # transcript torto nunca derruba a listagem inteira
        _log.debug("locate: sem leitura de %s: %s", jsonl, e)
    real = real or cwd
    if not real:
        return Location(None, False, None, False)
    if not os.path.isdir(real):
        # ponytail: pasta sumida que não é a de abertura vira "worktree apagada"; uma subpasta
        # comum apagada também cairia aqui. Afinar só se aparecer falso positivo.
        gone = real != cwd or real in removed()
        return Location(None, False, real if gone else None, gone)
    root = repo_root_of(real) or real
    branch, wt = head_info(root)
    return Location(branch, wt, root if wt else None, False)
