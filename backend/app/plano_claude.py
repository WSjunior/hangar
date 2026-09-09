"""Descoberta do plano nativo associado a uma sessão Claude Code."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any


_SLUG = re.compile(r"^[a-z0-9][a-z0-9-]{0,119}$")


@dataclass(frozen=True)
class PlanoClaude:
    nome: str
    caminho: Path


def _ler_json(caminho: Path) -> dict[str, Any]:
    try:
        valor = json.loads(caminho.read_text(encoding="utf-8"))
        return valor if isinstance(valor, dict) else {}
    except (OSError, ValueError):
        return {}


def _config_dir_do_transcript(transcript: Path) -> Path:
    """O transcript Claude sempre fica em <config>/projects/<slug>/<id>.jsonl."""
    pais = transcript.parents
    if len(pais) >= 3 and pais[1].name == "projects":
        return pais[2]
    return Path.home() / ".claude"


def _diretorio_de_planos(transcript: Path, cwd: Path) -> Path:
    config_dir = _config_dir_do_transcript(transcript)
    configuracao: dict[str, Any] = {}
    for arquivo in (
        config_dir / "settings.json",
        config_dir / "settings.local.json",
        cwd / ".claude" / "settings.json",
        cwd / ".claude" / "settings.local.json",
    ):
        configuracao.update(_ler_json(arquivo))

    bruto = configuracao.get("plansDirectory")
    if not isinstance(bruto, str) or not bruto.strip():
        return (config_dir / "plans").resolve()
    caminho = Path(bruto).expanduser()
    return (caminho if caminho.is_absolute() else cwd / caminho).resolve()


def _caminho_de_bloco(bloco: dict[str, Any]) -> str | None:
    entrada = bloco.get("input")
    if not isinstance(entrada, dict):
        return None
    if bloco.get("name") in {"Write", "Edit"}:
        caminho = entrada.get("file_path")
        return caminho if isinstance(caminho, str) else None
    # Mantém compatibilidade com versões que venham a expor o arquivo no evento do modo plano.
    if bloco.get("name") == "ExitPlanMode":
        caminho = entrada.get("planFilePath")
        return caminho if isinstance(caminho, str) else None
    return None


def descobrir(transcript: str | Path, cwd: str | Path) -> PlanoClaude | None:
    """Encontra o último plano confirmado pelo transcript principal da sessão."""
    arquivo = Path(transcript)
    raiz = _diretorio_de_planos(arquivo, Path(cwd))
    candidatos: dict[str, Path] = {}
    saidas_do_modo_plano: dict[str, str] = {}
    ultimo: Path | None = None
    ultimo_slug: str | None = None
    slug_confirmado: str | None = None

    try:
        linhas = arquivo.open(encoding="utf-8")
    except OSError:
        return None
    with linhas:
        for linha in linhas:
            try:
                evento = json.loads(linha)
            except (ValueError, TypeError):
                continue
            if not isinstance(evento, dict) or evento.get("isSidechain") is True:
                continue
            slug = evento.get("slug")
            if isinstance(slug, str) and _SLUG.fullmatch(slug):
                ultimo_slug = slug
            mensagem = evento.get("message")
            conteudo = mensagem.get("content") if isinstance(mensagem, dict) else None
            if not isinstance(conteudo, list):
                continue
            for bloco in conteudo:
                if not isinstance(bloco, dict):
                    continue
                if bloco.get("type") == "tool_use":
                    identificador = bloco.get("id")
                    if (bloco.get("name") == "ExitPlanMode" and isinstance(identificador, str)
                            and ultimo_slug is not None):
                        saidas_do_modo_plano[identificador] = ultimo_slug
                    bruto = _caminho_de_bloco(bloco)
                    if not bruto or not isinstance(identificador, str):
                        continue
                    caminho = Path(bruto).expanduser()
                    caminho = (caminho if caminho.is_absolute() else Path(cwd) / caminho).resolve()
                    if caminho.suffix.lower() == ".md" and caminho.is_relative_to(raiz):
                        candidatos[identificador] = caminho
                elif bloco.get("type") == "tool_result":
                    identificador = bloco.get("tool_use_id")
                    caminho = candidatos.pop(identificador, None)
                    if caminho is not None and bloco.get("is_error") is not True:
                        ultimo = caminho
                    slug_da_saida = saidas_do_modo_plano.pop(identificador, None)
                    if slug_da_saida is not None and bloco.get("is_error") is not True:
                        slug_confirmado = slug_da_saida

    # Uma escrita confirmada traz o caminho exato e prevalece sobre o slug genérico da sessão.
    if ultimo is not None:
        return PlanoClaude(nome=ultimo.stem, caminho=ultimo)
    if slug_confirmado is not None:
        caminho = (raiz / f"{slug_confirmado}.md").resolve()
        # A validação do slug torna a checagem redundante no caso normal, mas mantém a fronteira
        # explícita caso a construção do caminho mude no futuro.
        if caminho.is_relative_to(raiz):
            return PlanoClaude(nome=slug_confirmado, caminho=caminho)
    return None
