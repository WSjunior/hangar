# backend/app/rust_release.py
"""Baixa o hangar-server e o hangar-cano da release `server-latest` para ~/.hangar/bin/.

Um módulo só para os dois instaladores e o botão Atualizar. Sem os binários o Python atende
sozinho, então nada aqui levanta: cada falha vira aviso e linha no diário.
Uso: python -m app.rust_release [--never-fail]   (sai 0 no lugar, 1 falhou, 2 sem build)
"""
from __future__ import annotations

import hashlib
import http.client
import json
import logging
import os
import platform
import secrets
import sys
import urllib.request
from pathlib import Path

from app import atomico, diag

_log = logging.getLogger("hangar.rust_release")

RELEASE_URL = "https://github.com/jeffer1312/hangar/releases/download/server-latest"
NAMES = ("hangar-server", "hangar-cano")

# Constante de módulo pelo motivo do atualizar.py: o teste troca a decisão sem mexer no os.name.
_E_WINDOWS = os.name == "nt"

# Corpo cortado no meio (IncompleteRead) é HTTPException, não OSError.
_DOWNLOAD_ERRORS = (OSError, http.client.HTTPException, ValueError)


def platform_key() -> str | None:
    machine = platform.machine().lower()
    if sys.platform.startswith("linux") and machine in ("x86_64", "amd64"):
        return "linux-x86_64"
    if sys.platform == "win32" and machine in ("amd64", "x86_64"):
        return "windows-x86_64"
    if sys.platform == "darwin" and machine == "arm64":
        return "macos-aarch64"
    return None


def bin_dir() -> Path:
    return Path.home() / ".hangar" / "bin"


def _get(url: str, timeout: float) -> bytes:
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return r.read()


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def _already_there(target: Path, sha: str) -> bool:
    try:
        return target.is_file() and _sha256_file(target) == sha
    except OSError:
        return False                 # ilegível: baixa de novo em vez de dar o binário por bom


def _free(path: Path) -> bool:
    try:
        os.lstat(path)
    except FileNotFoundError:
        return True
    except OSError:
        return False                 # nega até a leitura (apagado mas ainda aberto): ocupa o nome
    return False


def _sweep_old(target: Path) -> None:
    """Apaga os `<nome>.old*` de trocas anteriores, sem olhar a caixa (o NTFS não diferencia)."""
    prefix = f"{target.name}.old".lower()
    try:
        entries = list(target.parent.iterdir())
    except OSError:
        return
    for entry in entries:
        if entry.name.lower().startswith(prefix):
            try:
                entry.unlink()
            except OSError:
                pass                 # ainda é a imagem de um processo vivo: sai numa próxima rodada


def _old_name(target: Path) -> Path:
    """`<nome>.old`, ou `<nome>.old-N` quando um anterior ainda preso ocupa o nome."""
    candidate = target.with_name(f"{target.name}.old")
    n = 0
    while not _free(candidate):
        n += 1
        candidate = target.with_name(f"{target.name}.old-{n}")
    return candidate


def _install(data: bytes, target: Path) -> None:
    tmp = target.with_name(f".{target.name}.{secrets.token_hex(4)}.tmp")
    try:
        tmp.write_bytes(data)
        tmp.chmod(0o755)
        if _E_WINDOWS and not _free(target):
            # Exe em uso não pode ser sobrescrito nem apagado, mas pode ser renomeado: o processo
            # vivo segue no nome velho e o próximo a subir pega o novo.
            old = _old_name(target)
            target.rename(old)
            try:
                atomico.substituir(tmp, target)
            except OSError:
                old.rename(target)   # sem desfazer, o caminho do binário ficaria vazio
                raise
        else:
            atomico.substituir(tmp, target)
    finally:
        tmp.unlink(missing_ok=True)


def _fetch_one(url: str, files: object, plat: str, name: str, target: Path) -> str | None:
    entry = files.get(f"{plat}/{name}") if isinstance(files, dict) else None
    if not isinstance(entry, dict) or not isinstance(entry.get("name"), str) \
            or not isinstance(entry.get("sha256"), str):
        diag.registrar("hangar_server.baixar", "erro", etapa="manifesto", detalhe=name)
        return f"{name}: a release não traz build para {plat}"
    sha = entry["sha256"].lower()
    if _already_there(target, sha):
        return None
    try:
        data = _get(f"{url}/{entry['name']}", 300)
    except _DOWNLOAD_ERRORS as e:
        diag.registrar("hangar_server.baixar", "erro", etapa="download", detalhe=name, **diag.erro_campos(e))
        return f"{name}: o download falhou ({e})"
    if hashlib.sha256(data).hexdigest() != sha:
        diag.registrar("hangar_server.baixar", "erro", etapa="sha256", detalhe=name)
        return f"{name}: o sha256 não confere com a release; o binário anterior ficou"
    try:
        _install(data, target)
    except OSError as e:
        diag.registrar("hangar_server.baixar", "erro", etapa="gravar", detalhe=name, **diag.erro_campos(e))
        return f"{name}: não consegui gravar em {target.parent} ({e})"
    diag.registrar("hangar_server.baixar", codigo="trocado", detalhe=name)
    return None


def fetch(base_url: str | None = None, dest: Path | None = None) -> list[str] | None:
    """Põe em `dest` os binários desta máquina, conferidos pelo sha256 do manifesto.

    `None` = a release não tem build para esta máquina; `[]` = tudo no lugar; senão, os avisos.
    Nunca levanta: quem chama (instalador, botão Atualizar) segue sem os binários.
    """
    plat = platform_key()
    if plat is None:
        diag.registrar("hangar_server.baixar", "aviso", codigo="sem_build")
        return None
    url = (base_url or os.environ.get("HANGAR_SERVER_RELEASE_URL") or RELEASE_URL).rstrip("/")
    dest = dest or bin_dir()
    ext = ".exe" if plat.startswith("windows") else ""
    try:
        files = json.loads(_get(f"{url}/server-latest.json", 15))["files"]
        dest.mkdir(parents=True, exist_ok=True)
    except (*_DOWNLOAD_ERRORS, KeyError, TypeError) as e:
        diag.registrar("hangar_server.baixar", "erro", etapa="manifesto", **diag.erro_campos(e))
        return [f"binários Rust não baixados: não consegui ler o manifesto da release ({e})"]
    for name in NAMES:
        _sweep_old(dest / f"{name}{ext}")
    avisos = [aviso for name in NAMES
              if (aviso := _fetch_one(url, files, plat, name, dest / f"{name}{ext}"))]
    for aviso in avisos:
        _log.warning(aviso)
    return avisos


def main(argv: list[str]) -> int:
    avisos = fetch()
    if avisos is None:
        print("binários Rust: a release não tem build para esta máquina; o Python atende sozinho")
        code = 2
    elif avisos:
        for aviso in avisos:
            print(aviso)
        code = 1
    else:
        print(f"binários Rust em {bin_dir()}")
        code = 0
    # O passo de atualização não pode parar a atualização inteira por um extra.
    return 0 if "--never-fail" in argv else code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
