"""Identidade e rota exclusiva das credenciais OAuth locais; importável pelo wrapper sem venv."""
import json
from pathlib import Path
import re

from app import codex_contas

_SAFE_ID = re.compile(r"[a-z0-9][a-z0-9_-]{0,31}")
_SAFE_PREFIX = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,63}")
_SAFE_MODEL = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.:\[\]-]*")


def auth_dir() -> Path:
    return Path.home() / ".cli-proxy-api"


def _credentials() -> list[tuple[Path, dict]]:
    out = []
    try:
        paths = sorted(auth_dir().glob("*.json"))
        for path in paths:
            if path.is_symlink():
                raise ValueError("CLIProxyAPI: credencial por link não é permitida")
            raw = path.read_bytes()
            data = json.loads(raw)
            if not isinstance(data, dict):
                raise ValueError("CLIProxyAPI: arquivo de credencial inválido")
            if "disabled" in data and not isinstance(data["disabled"], bool):
                raise ValueError("CLIProxyAPI: campo disabled inválido")
            out.append((path, data))
    except (OSError, UnicodeError, json.JSONDecodeError):
        raise ValueError("CLIProxyAPI: não consegui ler as credenciais locais") from None
    return out


def _associated(credentials: list, accounts=None) -> list[tuple[dict, tuple]]:
    result = []
    for account in codex_contas.list_visible_accounts() if accounts is None else accounts:
        try:
            data = json.loads((account.home / "auth.json").read_bytes())
        except FileNotFoundError:
            continue
        except (OSError, UnicodeError, json.JSONDecodeError):
            raise ValueError(f"CLIProxyAPI: login Codex ilegível na conta {account.id}") from None
        tokens = data.get("tokens") if isinstance(data, dict) else None
        account_id = tokens.get("account_id") if isinstance(tokens, dict) else None
        if not isinstance(account_id, str) or not account_id:
            continue
        matches = [c for c in credentials if c[1].get("type") == "codex"
                   and c[1].get("account_id") == account_id
                   and c[1].get("disabled") is not True]
        if len(matches) > 1:
            raise ValueError(f"CLIProxyAPI: mais de uma credencial ativa para a conta {account.id}")
        if not matches:
            continue
        record = matches[0]
        if any(previous[1][0] == record[0] for previous in result):
            raise ValueError("CLIProxyAPI: a mesma credencial corresponde a mais de uma conta Hangar")
        email = record[1].get("email")
        email = email if isinstance(email, str) else ""
        result.append(({"account": account.id,
                        "credential_id": f"codex:{account.home.expanduser().resolve()}",
                        "email": email, "label": email or account.id,
                        "prefix": record[1].get("prefix") or None}, record))
    return result


def _exclusive(prefix: object, record: tuple, credentials: list) -> str:
    if not isinstance(prefix, str) or not _SAFE_PREFIX.fullmatch(prefix):
        raise ValueError("CLIProxyAPI: conta sem prefixo válido; prepare as contas primeiro")
    if any(c[0] != record[0] and c[1].get("prefix") == prefix for c in credentials):
        raise ValueError("CLIProxyAPI: prefixo compartilhado entre credenciais; prepare as contas primeiro")
    return prefix


def list_accounts() -> list[dict]:
    credentials = _credentials()
    entries = _associated(credentials)
    for entry, record in entries:
        if entry["prefix"] is not None:
            _exclusive(entry["prefix"], record, credentials)
    return [entry for entry, _ in entries]


def resolve(account: str, home: str | None = None) -> dict:
    if not isinstance(account, str) or not _SAFE_ID.fullmatch(account):
        raise ValueError("CLIProxyAPI: identificador de conta inválido")
    if home is None:
        entries = list_accounts()
    else:
        if any(c in home for c in "\n\r\x00") or not Path(home).is_absolute():
            raise ValueError("CLIProxyAPI: raiz Codex inválida")
        credentials = _credentials()
        associated = _associated(credentials, [codex_contas.Account(account, Path(home), account == "default")])
        for entry, record in associated:
            _exclusive(entry["prefix"], record, credentials)
        entries = [entry for entry, _ in associated]
    for entry in entries:
        if entry["account"] == account:
            if not entry["prefix"]:
                raise ValueError("CLIProxyAPI: conta sem prefixo; prepare as contas primeiro")
            return entry
    raise ValueError(f"CLIProxyAPI: conta {account} desconhecida, desconectada ou desativada")


def prefix_model(model: str, prefix: str) -> str:
    if not _SAFE_PREFIX.fullmatch(prefix):
        raise ValueError("CLIProxyAPI: prefixo inválido")
    if model.startswith(prefix + "/"):
        model = model[len(prefix) + 1:]
    if not _SAFE_MODEL.fullmatch(model):
        raise ValueError("CLIProxyAPI: modelo inválido ou prefixo de outra conta")
    return f"{prefix}/{model}"


def base_model(model: str, prefix: str) -> str:
    return prefix_model(model, prefix).split("/", 1)[1]


def models_for(models: list[dict], prefix: str) -> list[dict]:
    start = prefix + "/"
    return [{**model, "id": model["id"][len(start):]} for model in models
            if isinstance(model.get("id"), str) and model["id"].startswith(start)
            and _SAFE_MODEL.fullmatch(model["id"][len(start):])
            and not model["id"][len(start):].startswith("gpt-image-")]
