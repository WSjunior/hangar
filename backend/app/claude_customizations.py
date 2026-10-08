import copy
import http.client
import ipaddress
import json
import uuid
from pathlib import Path


SESSION_SETTINGS_ENV = "HANGAR_CLAUDE_SETTINGS"
_config: tuple[str, str] | None = None
_MAX_RESPONSE = 4 * 1024 * 1024
_TIMEOUT = 25.0


class CustomizationsError(RuntimeError):
    def __init__(self, status: int, code: str, detail: str):
        super().__init__(detail)
        self.status, self.code, self.detail = status, code, detail


def configure(address: str | None, secret: str | None) -> None:
    global _config
    if not address or not secret:
        _config = None
        return
    host, port = address.rsplit(":", 1)
    if not ipaddress.ip_address(host.strip("[]")).is_loopback or not 1 <= int(port) <= 65535:
        raise ValueError("endereço privado de customização inválido")
    _config = (address, secret)


def _request(operation: str, arguments: dict) -> dict:
    config = _config
    if config is None:
        raise CustomizationsError(503, "claude_customizations_unavailable",
                                  "A seleção de plugins e skills precisa do servidor Rust disponível.")
    host, port = config[0].rsplit(":", 1)
    connection = http.client.HTTPConnection(host.strip("[]"), int(port), timeout=_TIMEOUT)
    try:
        connection.request("POST", "/__hangar_server/claude/customizations",
                           body=json.dumps({"op": operation, "args": arguments},
                                           ensure_ascii=False).encode("utf-8"),
                           headers={"content-type": "application/json", "x-hangar-internal": config[1]})
        response = connection.getresponse()
        if response.status != 200:
            raise CustomizationsError(503, "claude_customizations_unavailable",
                                      "O servidor não respondeu à seleção de plugins e skills.")
        body = response.read(_MAX_RESPONSE + 1)
        if len(body) > _MAX_RESPONSE:
            raise ValueError("resposta grande demais")
        value = json.loads(body)
    except (OSError, ValueError, http.client.HTTPException) as exc:
        raise CustomizationsError(503, "claude_customizations_unavailable",
                                  "Não foi possível consultar os plugins e skills desta sessão.") from exc
    finally:
        connection.close()
    if not isinstance(value, dict):
        raise CustomizationsError(503, "claude_customizations_unavailable", "Resposta de customização inválida.")
    if value.get("ok") is not True:
        error = value.get("error")
        if not isinstance(error, dict):
            error = {}
        status = error.get("status", 503)
        raise CustomizationsError(status if type(status) is int and 400 <= status <= 599 else 503,
                                  str(error.get("code") or "claude_customizations_unavailable"),
                                  str(error.get("detail") or "Não foi possível aplicar a seleção da sessão."))
    result = value.get("result")
    if not isinstance(result, dict):
        raise CustomizationsError(503, "claude_customizations_unavailable", "Resposta de customização inválida.")
    return result


def catalog(cwd: str, config_dir: str) -> dict:
    return _request("catalog", {"cwd": cwd, "config_dir": config_dir})


def _validated_settings(settings: dict | None) -> dict | None:
    if settings is None:
        return None
    if not isinstance(settings, dict) or set(settings) - {"enabledPlugins", "skillOverrides", "permissions"}:
        raise ValueError("configuração da sessão inválida")
    plugins = settings.get("enabledPlugins", {})
    skills = settings.get("skillOverrides", {})
    permissions = settings.get("permissions", {})
    if (not isinstance(plugins, dict) or any(not isinstance(k, str) or type(v) is not bool for k, v in plugins.items())
            or not isinstance(skills, dict) or any(not isinstance(k, str) or v not in ("on", "off") for k, v in skills.items())
            or not isinstance(permissions, dict) or set(permissions) - {"deny"}):
        raise ValueError("configuração da sessão inválida")
    denied = permissions.get("deny", [])
    if not isinstance(denied, list) or any(not isinstance(rule, str) or not rule.startswith("Skill(")
                                          or not rule.endswith(")") for rule in denied):
        raise ValueError("configuração da sessão inválida")
    return copy.deepcopy(settings)


def stored_settings(session_id: str) -> dict | None:
    sid = str(uuid.UUID(session_id))
    path = Path.home() / ".hangar" / "claude-customizations" / f"{sid}.json"
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None
    except (OSError, ValueError) as exc:
        raise CustomizationsError(503, "claude_customizations_unavailable",
                                  "Não foi possível ler as escolhas guardadas desta conversa.") from exc
    if not isinstance(value, dict) or "settings" not in value:
        raise ValueError("configuração guardada da sessão inválida")
    return _validated_settings(value["settings"])


def prepare(session_id: str, cwd: str, config_dir: str, selection: dict | None, *, resume: bool) -> dict | None:
    if selection is None:
        return stored_settings(session_id) if resume else None
    result = _request("prepare", {"session_id": session_id, "cwd": cwd, "config_dir": config_dir,
                                  "selection": selection, "resume": resume})
    return _validated_settings(result.get("settings"))


def resume_settings(session_id: str, current_session_id: str | None, process_settings: dict | None) -> dict | None:
    if session_id == current_session_id and process_settings is not None:
        return _validated_settings(process_settings)
    return stored_settings(session_id)


def remember(session_id: str, settings: dict | None) -> None:
    settings = _validated_settings(settings)
    if settings:
        _request("remember", {"session_id": session_id, "settings": settings})


def environment(settings: dict | None) -> dict[str, str]:
    settings = _validated_settings(settings)
    return {SESSION_SETTINGS_ENV: json.dumps(settings, ensure_ascii=False, separators=(",", ":")) if settings else ""}


def from_environment(value: str | None) -> dict | None:
    return _validated_settings(json.loads(value)) if value else None


def apply_settings(argv: list[str], settings: dict | None, *, cwd: str | Path | None = None) -> list[str]:
    settings = _validated_settings(settings)
    if not settings:
        return list(argv)
    from app.engines import _read_settings

    args, existing, insertion = [], {}, None
    ix = 0
    while ix < len(argv):
        arg = argv[ix]
        if arg == "--":
            args.extend(argv[ix:])
            break
        if arg == "--settings":
            if ix + 1 >= len(argv):
                raise ValueError("--settings precisa de um valor")
            insertion = len(args) if insertion is None else insertion
            existing = _read_settings(argv[ix + 1], cwd=Path(cwd) if cwd is not None else None)
            ix += 2
        elif arg.startswith("--settings="):
            insertion = len(args) if insertion is None else insertion
            existing = _read_settings(arg.split("=", 1)[1], cwd=Path(cwd) if cwd is not None else None)
            ix += 1
        else:
            args.append(arg)
            ix += 1
    merged = copy.deepcopy(existing)
    for key in ("enabledPlugins", "skillOverrides"):
        if key in settings:
            current = merged.get(key, {})
            if not isinstance(current, dict):
                raise ValueError("--settings: configuração inválida")
            merged[key] = {**current, **settings[key]}
    if "permissions" in settings:
        current = merged.get("permissions", {})
        if not isinstance(current, dict) or not isinstance(current.get("deny", []), list):
            raise ValueError("--settings: permissões inválidas")
        denied = list(dict.fromkeys([*current.get("deny", []), *settings["permissions"].get("deny", [])]))
        merged["permissions"] = {**current, "deny": denied}
    insertion = len(args) if insertion is None else insertion
    args[insertion:insertion] = ["--settings", json.dumps(merged, ensure_ascii=False, separators=(",", ":"))]
    return args
