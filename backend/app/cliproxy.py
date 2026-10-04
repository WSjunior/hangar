"""Instância local do CLIProxyAPI: endereço e chave lidos do config dele.

A chave nunca sai do servidor: a tela recebe só o endereço e os modelos, e o motor é gravado com
`use_cliproxy_key`, que pega a chave daqui. O navegador pode ser o celular.
"""
import http.client
import json
import os
import re
import tempfile
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

import yaml

_PORTA_PADRAO = 8317


def config_path() -> Path:
    return Path.home() / ".cli-proxy-api" / "config.yaml"


def normalize_base(url: str) -> str:
    url = url.strip().rstrip("/")
    return url[:-3].rstrip("/") if url.endswith("/v1") else url


def local() -> dict[str, str] | None:
    """`{base_url, api_key}` da instância desta máquina; None sem config.

    Config que existe e não serve (ilegível, sem api-keys) levanta ValueError: "não instalado" e
    "instalado sem chave" são respostas diferentes para quem configura.
    """
    try:
        texto = config_path().read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    except OSError as e:
        raise ValueError(f"{config_path()}: {e.strerror or e}") from None
    try:
        dados: Any = yaml.safe_load(texto)
    except yaml.YAMLError:
        raise ValueError(f"{config_path()}: YAML inválido") from None
    if not isinstance(dados, dict):
        return None
    lista = dados.get("api-keys")
    if lista is not None and not isinstance(lista, list):
        raise ValueError(f"{config_path()}: api-keys deve ser uma lista")
    chaves = [k.strip() for k in (lista or []) if isinstance(k, str) and k.strip()]
    if not chaves:
        raise ValueError(f"{config_path()}: sem api-keys")
    host = str(dados.get("host") or "").strip()
    # Escutar em todas as interfaces inclui o loopback, que é por onde este servidor fala com ele.
    if host in ("", "0.0.0.0", "::"):
        host = "127.0.0.1"
    if ":" in host and not host.startswith("["):
        host = f"[{host}]"
    try:
        porta = int(dados.get("port") or _PORTA_PADRAO)
    except (TypeError, ValueError):
        raise ValueError(f"{config_path()}: port inválida") from None
    if not 0 < porta < 65536:
        raise ValueError(f"{config_path()}: port inválida")
    tls = dados.get("tls")
    esquema = "https" if isinstance(tls, dict) and tls.get("enable") is True else "http"
    return {"base_url": f"{esquema}://{host}:{porta}", "api_key": chaves[0]}


def is_local_engine(cfg: dict) -> bool:
    inst = local()
    return bool(inst and normalize_base(str(cfg.get("base_url") or "")) == inst["base_url"])


def account_for_engine(cfg: dict, account: str, home: str | None = None) -> dict:
    from app.cliproxy_accounts import resolve
    inst = local()
    if not inst or normalize_base(str(cfg.get("base_url") or "")) != inst["base_url"]:
        raise ValueError("a conta ChatGPT só pode ser fixada no CLIProxyAPI local")
    entry = resolve(account, home=home)
    return {**entry, "home": entry["credential_id"].removeprefix("codex:"),
            "base_url": inst["base_url"]}


def validate_models(cfg: dict, model: str, account: dict, models: list[dict] | None = None) -> list[dict]:
    from app import engine_probe
    from app.cliproxy_accounts import base_model, models_for
    if models is None:
        try:
            models = engine_probe.listar_modelos(cfg["base_url"], cfg["api_key"])
        except (RuntimeError, ValueError) as exc:
            raise ValueError("CLIProxyAPI: catálogo indisponível: " + redact(str(exc), cfg["api_key"])) from None
    catalog = models_for(models, account["prefix"])
    available = {item["id"] for item in catalog}
    for label, selected in (("principal", model), ("dos subagentes", cfg.get("subagent_model") or model)):
        if base_model(selected, account["prefix"]) not in available:
            raise ValueError(f"CLIProxyAPI: modelo {label} indisponível nesta conta")
    return catalog


def engine_env(name: str, model: str | None, context_window: int | None, account_id: str, *,
               home: str | None = None, models: list[dict] | None = None,
               expected_base: str | None = None) -> dict[str, str]:
    from app import engines
    cfg = engines.listar().get(name)
    if not cfg:
        raise ValueError("CLIProxyAPI: motor indisponível")
    account = account_for_engine(cfg, account_id, home=home)
    if expected_base is not None and normalize_base(expected_base) != account["base_url"]:
        raise ValueError("CLIProxyAPI: endereço da conta fixa mudou")
    validate_models(cfg, model or cfg["model"], account, models)
    return engines.env_de(name, model, context_window, engine_account=account_id,
                          engine_account_home=account["home"], engine_account_base_url=account["base_url"])


def is_engine_model(model_id: object) -> bool:
    # gpt-image-* gera imagem e não responde /v1/messages: não serve de motor.
    return isinstance(model_id, str) and not model_id.startswith("gpt-image-")


def redact(texto: str, api_key: str) -> str:
    # O erro do proxy pode repetir a chave ("invalid key …"), e ele vai para a tela.
    return texto.replace(api_key, "***") if api_key else texto


# Prefixo que a primeira versão gravou à mão; vale como "sem nome" para a nomeação automática.
_PREFIXO_ANTIGO = re.compile(r"hangar-[0-9a-f]{24}")
_MGMT_ARQUIVO = "cliproxy-management.key"
_chave_recusada: str | None = None


def _mgmt_path() -> Path:
    from app import contas
    return contas.compartilhado() / _MGMT_ARQUIVO


def management_key() -> str | None:
    try:
        chave = _mgmt_path().read_text(encoding="utf-8").strip()
    except OSError:
        return None
    return chave or None


def set_management_key(chave: str) -> None:
    """Grava (ou apaga, vazia) a senha de gerenciamento. 0600: ela edita todas as credenciais do proxy."""
    from app import atomico
    alvo = _mgmt_path()
    chave = chave.strip()
    if not chave:
        alvo.unlink(missing_ok=True)
        return
    # Vai num cabeçalho HTTP: fora de ASCII o http.client estoura e o erro ecoaria um pedaço dela.
    if not chave.isascii() or any(c in chave for c in "\r\n\x00"):
        raise ValueError("senha de gerenciamento só com caracteres ASCII, sem quebra de linha")
    alvo.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=alvo.parent, prefix=".mgmt-")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(chave)
        atomico.substituir(tmp, alvo)
    except OSError:
        Path(tmp).unlink(missing_ok=True)
        raise


def prefix_from_email(email: str, taken: set[str]) -> str | None:
    """`ana.souza@empresa.com` → `ana.souza`; com o domínio junto se o nome já existe."""
    from app.cliproxy_accounts import _SAFE_PREFIX
    local_part, _, domain = email.strip().lower().partition("@")

    def limpo(s: str) -> str:
        return re.sub(r"[^a-z0-9_.-]+", "-", s).strip("-._")[:64]

    for nome in (limpo(local_part), limpo(f"{local_part}-{domain.split('.')[0]}")):
        if nome and _SAFE_PREFIX.fullmatch(nome) and nome not in taken:
            return nome
    return None


def unnamed_accounts() -> list[tuple[dict, tuple]]:
    """Contas associadas sem prefixo ou com o antigo `hangar-<hex>`."""
    from app.cliproxy_accounts import _associated, _credentials
    return [(e, r) for e, r in _associated(_credentials())
            if not e["prefix"] or _PREFIXO_ANTIGO.fullmatch(e["prefix"])]


def name_accounts() -> list[str]:
    """Dá às contas sem nome o prefixo do e-mail, pela rota de gerenciamento do proxy.

    Quem grava o arquivo é o próprio proxy: ele também o reescreve ao renovar o token, e uma
    escrita daqui no meio disso poderia voltar um token velho. Sem senha salva, não faz nada.
    """
    global _chave_recusada
    from app.cliproxy_accounts import _credentials
    pendentes = unnamed_accounts()
    chave = management_key()
    inst = local()
    if not pendentes or not chave or not inst:
        return []
    # Senha já recusada não é reenviada a cada detecção: o proxy pode bloquear o IP por insistência.
    if chave == _chave_recusada:
        raise ValueError("CLIProxyAPI recusou a senha de gerenciamento")
    taken = {c[1]["prefix"] for c in _credentials() if isinstance(c[1].get("prefix"), str)}
    nomeadas, erros = [], []
    for entry, record in pendentes:
        nome = prefix_from_email(entry["email"], taken)
        if not nome:
            erros.append(f"{entry['label']}: sem e-mail que sirva de nome")
            continue
        corpo = json.dumps({"name": record[0].name, "prefix": nome}).encode()
        req = urllib.request.Request(f"{inst['base_url']}/v0/management/auth-files/fields", data=corpo,
                                     method="PATCH", headers={"X-Management-Key": chave,
                                                              "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=10):
                pass
        except urllib.error.HTTPError as e:
            if e.code == 401:
                _chave_recusada = chave
                raise ValueError("CLIProxyAPI recusou a senha de gerenciamento") from None
            if e.code == 403:
                raise ValueError("CLIProxyAPI bloqueou o gerenciamento (403): confira o remote-management no config") from None
            erros.append(f"{entry['label']}: HTTP {e.code}")
            continue
        except (urllib.error.URLError, OSError, http.client.HTTPException) as e:
            raise ValueError(f"CLIProxyAPI não respondeu ao nomear as contas: {e}") from None
        taken.add(nome)
        nomeadas.append(nome)
    # 200 sem o prefixo no arquivo: o proxy aceitou e não gravou, e a detecção repetiria isso calada.
    teimosas = {e["email"] for e, _ in unnamed_accounts()} & {e["email"] for e, _ in pendentes
                                                                 if prefix_from_email(e["email"], set())}
    if nomeadas and teimosas:
        erros.append("o proxy aceitou o nome mas não o gravou em " + ", ".join(sorted(teimosas)))
    if erros:
        raise ValueError(f"{len(nomeadas)} de {len(pendentes)} contas nomeadas — " + "; ".join(erros))
    return nomeadas
