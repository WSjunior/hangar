"""Origem privada e etapas confirmadas da transferência de uma conversa.

O registro é a verdade; índices só apontam para ele e nunca confirmam uma etapa.
Não abre processos, não grava rollout e não guarda configuração ou credenciais.
"""
from __future__ import annotations

import hashlib
import json
import os
import tempfile
import threading
import uuid
from contextlib import contextmanager
from dataclasses import asdict, dataclass, replace
from enum import StrEnum
from pathlib import Path

from app import atomico
from app.share_life import session_life


class TransferError(ValueError):
    """Erro público da troca; os parâmetros nunca carregam conversa/configuração."""

    def __init__(self, code: str, *, status: int = 409,
                 params: dict[str, str | int] | None = None):
        if not isinstance(code, str) or not code.startswith("session_transfer_") \
                or not all(c.islower() or c == "_" or c.isdigit() for c in code):
            raise ValueError("código de transferência inválido")
        if type(status) is not int or not 400 <= status <= 599:
            raise ValueError("status de transferência inválido")
        allowed = {"model", "effort", "account", "phase", "limit", "estimated"}
        if params is not None and (not isinstance(params, dict) or set(params) - allowed
                or any(type(v) not in (str, int) or
                       (isinstance(v, str) and (len(v) > 200 or "\n" in v or "\r" in v))
                       for v in params.values())):
            raise ValueError("parâmetros públicos de transferência inválidos")
        self.code, self.status, self.params = code, status, dict(params or {})
        super().__init__(code)


class TransferPhase(StrEnum):
    PREPARING = "preparing"
    SOURCE_STOPPED = "source_stopped"
    IMPORTED = "imported"
    PUBLISHING = "publishing"
    COMPLETE = "complete"
    RESTORING = "restoring"
    RESTORE_FAILED = "restore_failed"
    REJECTED = "rejected"
    ROLLED_BACK = "rolled_back"


@dataclass(frozen=True)
class ConversationSource:
    path: str
    provider: str
    digest: str
    selected_uuids: tuple[str, ...]


@dataclass(frozen=True)
class ImportBoundary:
    thread_id: str
    rollout_path: str
    min_offset: int
    imported_item_ids: tuple[str, ...]
    prefix_digest: str | None = None


@dataclass(frozen=True)
class TransferRecord:
    id: str
    name: str
    source_life: str
    phase: TransferPhase
    source: ConversationSource | None
    origin_meta: dict[str, object]
    destination_meta: dict[str, object] | None
    boundary: ImportBoundary | None
    error_code: str | None


_TERMINAL = {TransferPhase.COMPLETE, TransferPhase.REJECTED, TransferPhase.ROLLED_BACK}
# Somente escolhas, identidade e referências necessárias à retomada.
_META_FIELDS = {
    "name", "provider", "cwd", "jsonl", "session_id", "key", "config_dir", "headless",
    "engine", "model", "effort", "context_window", "permission_mode", "previous_non_plan",
    "subagent_model", "jev", "thread_id", "rollout_path", "codex_home", "codex_account",
    "lifecycle_id", "endpoint", "app_pid", "tui_pid", "launcher_pid", "pane_id", "pane_pid",
    "tool_output_token_limit", "transfer_id",
}
_lock = threading.RLock()
_UNSET = object()


def _base() -> Path:
    return Path.home() / ".hangar" / "conversation-transfers"


def _id(value: str) -> str:
    return str(uuid.UUID(value))


def _hash(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _thread_key(codex_home: str, thread_id: str) -> str:
    home = str(Path(codex_home).expanduser().resolve(strict=False))
    return json.dumps([home, thread_id], ensure_ascii=False)


def _record_path(transfer_id: str) -> Path:
    return _base() / f"{_id(transfer_id)}.json"


def _index_path(kind: str, key: str) -> Path:
    return _base() / "indexes" / f"{kind}-{_hash(key)}.json"


@contextmanager
def _locked():
    _base().mkdir(parents=True, exist_ok=True, mode=0o700)
    with _lock, (_base() / ".lock").open("a+b") as lock:
        if os.name == "nt":
            import msvcrt
            lock.seek(0)
            if not lock.read(1):
                lock.write(b"\0")
                lock.flush()
            lock.seek(0)
            msvcrt.locking(lock.fileno(), msvcrt.LK_LOCK, 1)
        else:
            import fcntl
            fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            yield
        finally:
            if os.name == "nt":
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(lock, fcntl.LOCK_UN)


def _write_bytes(path: Path, raw: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, suffix=".tmp", delete=False) as tmp:
            temporary = Path(tmp.name)
            tmp.write(raw)
            tmp.flush()
            os.fsync(tmp.fileno())
        atomico.substituir(temporary, path)
        if os.name != "nt":
            # O rename precisa estar durável antes de publicar a referência ao snapshot.
            fd = os.open(path.parent, os.O_RDONLY)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def _write_json(path: Path, value: object) -> None:
    _write_bytes(path, json.dumps(value, ensure_ascii=False).encode("utf-8"))


def _object(value: object, fields: set[str]) -> dict:
    if not isinstance(value, dict) or set(value) != fields:
        raise ValueError("registro de transferência com campos inválidos")
    return value


def _string(value: object) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError("registro de transferência com texto inválido")
    return value


def _strings(value: object) -> tuple[str, ...]:
    if not isinstance(value, (list, tuple)):
        raise ValueError("registro de transferência com lista inválida")
    return tuple(_string(item) for item in value)


def _metadata(value: object) -> dict[str, object]:
    if not isinstance(value, dict) or set(value) - _META_FIELDS:
        raise ValueError("metadados de transferência contêm campos não permitidos")
    for key, item in value.items():
        if item is None:
            continue
        expected = (bool if key in {"headless", "jev"} else
                    int if key in {"context_window", "app_pid", "tui_pid", "launcher_pid", "pane_pid", "tool_output_token_limit"}
                    else str)
        if type(item) is not expected:
            raise ValueError(f"metadado de transferência inválido: {key}")
    return dict(value)


def _decode(value: object) -> TransferRecord:
    data = _object(value, {"id", "name", "source_life", "phase", "source", "origin_meta",
                           "destination_meta", "boundary", "error_code"})
    source = None
    if data["source"] is not None:
        s = _object(data["source"], {"path", "provider", "digest", "selected_uuids"})
        digest = _string(s["digest"])
        if len(digest) != 64 or any(c not in "0123456789abcdef" for c in digest):
            raise ValueError("digest da origem inválido")
        if s["provider"] != "claude":
            raise ValueError("origem da transferência não é Claude")
        source = ConversationSource(_string(s["path"]), _string(s["provider"]), digest,
                                    _strings(s["selected_uuids"]))
    boundary = None
    if data["boundary"] is not None:
        if not isinstance(data["boundary"], dict):
            raise ValueError("fronteira de importação inválida")
        b = _object({"prefix_digest": None, **data["boundary"]},
                    {"thread_id", "rollout_path", "min_offset", "imported_item_ids", "prefix_digest"})
        prefix_digest = b["prefix_digest"]
        if prefix_digest is not None and (not isinstance(prefix_digest, str)
                or len(prefix_digest) != 64
                or any(c not in "0123456789abcdef" for c in prefix_digest)):
            raise ValueError("digest da fronteira inválido")
        if type(b["min_offset"]) is not int or b["min_offset"] < 0:
            raise ValueError("fronteira de importação inválida")
        boundary = ImportBoundary(_string(b["thread_id"]), _string(b["rollout_path"]),
                                  b["min_offset"], _strings(b["imported_item_ids"]), prefix_digest)
    error = None if data["error_code"] is None else _string(data["error_code"])
    destination = None if data["destination_meta"] is None else _metadata(data["destination_meta"])
    if boundary and destination and destination.get("thread_id") not in (None, boundary.thread_id):
        raise ValueError("thread da fronteira diverge do destino")
    return TransferRecord(_id(_string(data["id"])), _string(data["name"]),
                          _string(data["source_life"]), TransferPhase(data["phase"]), source,
                          _metadata(data["origin_meta"]),
                          destination, boundary, error)


def load_transfer(transfer_id: str) -> TransferRecord | None:
    path = _record_path(transfer_id)
    # Somente ausência de arquivo é ausência de registro; JSON null é corrupção.
    try:
        raw = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    record = _decode(json.loads(raw))
    if record.id != _id(transfer_id):
        raise ValueError("identidade do registro de transferência divergente")
    return record


def _index_ids(kind: str, key: str) -> list[str]:
    path = _index_path(kind, key)
    try:
        raw = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return []
    values = json.loads(raw)
    if not isinstance(values, list):
        raise ValueError("índice de transferência inválido")
    return [_id(_string(value)) for value in values]


def _append_index(kind: str, key: str, transfer_id: str) -> None:
    ids = _index_ids(kind, key)
    if transfer_id not in ids:
        _write_json(_index_path(kind, key), [*ids, transfer_id])


def _destination(record: TransferRecord) -> tuple[str, str] | None:
    meta = record.destination_meta or {}
    home = meta.get("codex_home")
    thread = meta.get("thread_id") or (record.boundary.thread_id if record.boundary else None)
    if isinstance(home, str) and home and isinstance(thread, str) and thread:
        return home, thread
    return None


def _save_locked(record: TransferRecord) -> None:
    # Índices aditivos antes do commit: uma interrupção deixa o registro anterior legível.
    # Um ponteiro novo só vale quando os campos do registro confirmam a associação.
    _append_index("session", record.name, record.id)
    _append_index("life", record.source_life, record.id)
    life = (record.destination_meta or {}).get("lifecycle_id")
    if isinstance(life, str) and life:
        _append_index("life", life, record.id)
    if destination := _destination(record):
        _append_index("thread", _thread_key(*destination), record.id)
    _write_json(_record_path(record.id), asdict(record))


def prepare_origin(origin_meta: dict[str, object]) -> dict[str, object]:
    meta = _metadata(origin_meta)
    if not meta.get("key"):
        meta["key"] = uuid.uuid4().hex
    return meta


def save_transfer(record: TransferRecord) -> None:
    record = _decode(asdict(record))
    with _locked():
        previous = load_transfer(record.id)
        if not record.origin_meta.get("key"):
            key = (previous.origin_meta.get("key") if previous else None) or uuid.uuid4().hex
            record = replace(record, origin_meta={**record.origin_meta, "key": key})
        elif previous and previous.origin_meta.get("key") != record.origin_meta["key"]:
            raise ValueError("a chave da origem da transferência mudou")
        if previous and previous.source_life != record.source_life:
            raise ValueError("a vida da origem da transferência mudou")
        _save_locked(record)


def capture_snapshot(record: TransferRecord, source_path: str | Path,
                     selected_uuids: tuple[str, ...]) -> TransferRecord:
    """Copia a fonte depois da parada confirmada pelo coordenador, sem avançar a fase."""
    if record.phase != TransferPhase.SOURCE_STOPPED:
        raise ValueError("snapshot exige parada confirmada da origem")
    _strings(selected_uuids)
    raw = Path(source_path).read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    snapshot = _base() / "snapshots" / f"{_id(record.id)}-{digest}.jsonl"
    with _locked():
        previous = load_transfer(record.id)
        if previous and (previous.phase != TransferPhase.SOURCE_STOPPED
                         or previous.source_life != record.source_life or previous.name != record.name):
            raise ValueError("a etapa da transferência mudou antes do snapshot")
        record = replace(record, origin_meta=prepare_origin(previous.origin_meta if previous else record.origin_meta))
        _write_bytes(snapshot, raw)
        if hashlib.sha256(snapshot.read_bytes()).hexdigest() != digest:
            raise ValueError("snapshot da origem diverge da cópia")
        captured = replace(record, source=ConversationSource(str(snapshot), "claude", digest,
                                                            selected_uuids))
        _save_locked(_decode(asdict(captured)))
    return captured


def verify_source(source: ConversationSource) -> None:
    if hashlib.sha256(Path(source.path).read_bytes()).hexdigest() != source.digest:
        raise ValueError("snapshot da origem mudou")


def transfer_for_thread(codex_home: str, thread_id: str) -> TransferRecord | None:
    key = _thread_key(codex_home, thread_id)
    for transfer_id in reversed(_index_ids("thread", key)):
        record = load_transfer(transfer_id)
        destination = _destination(record) if record else None
        if destination and _thread_key(*destination) == key:
            return record
    return None


def _belongs(record: TransferRecord, life: str | None, meta: dict | None, source_path: str | None = None) -> bool:
    marked = meta and meta.get("transfer_id") == record.id
    if record.phase == TransferPhase.COMPLETE:
        destination = record.destination_meta or {}
        if not marked and (not life or life != destination.get("lifecycle_id")):
            return False
        # /clear preserva a chave da sessão, mas inicia outra conversa.
        if meta and meta.get("provider") == "codex":
            association = _destination(record)
            if not association or meta.get("thread_id") != association[1]:
                return False
            return (not meta.get("codex_home") or
                    _thread_key(meta["codex_home"], meta["thread_id"]) == _thread_key(*association))
        return True
    if marked:
        return True
    if life != record.source_life or life is None:
        return False
    origin_sid = record.origin_meta.get("session_id")
    if source_path:
        origin_path = record.origin_meta.get("jsonl")
        if origin_path and os.path.realpath(source_path) != os.path.realpath(origin_path):
            return False
        if not origin_path and origin_sid and Path(source_path).stem != origin_sid:
            return False
    if meta and origin_sid and meta.get("session_id") and meta["session_id"] != origin_sid:
        return False
    return True


def transfer_for_session(name: str, *, lifecycle_id=_UNSET, meta: dict | None = None,
                         source_path: str | None = None) -> TransferRecord | None:
    if meta is None:
        from app.adapters.claude_headless import sessions as hl_sessions
        from app.adapters.codex import sessions as cx_sessions
        meta = cx_sessions.load(name) or hl_sessions.load(name)
    life = session_life(name) if lifecycle_id is _UNSET else lifecycle_id
    parked = lifecycle_id is _UNSET and life is None and meta is None
    if meta and meta.get("transfer_id"):
        record = load_transfer(meta["transfer_id"])
        if record is None:
            raise ValueError("sidecar aponta para transferência ausente")
        if record.name == name and _belongs(record, life, meta, source_path):
            return record
    for transfer_id in reversed(_index_ids("session", name)):
        record = load_transfer(transfer_id)
        if record and record.name == name:
            association_life = (record.source_life if parked and record.phase not in _TERMINAL else life)
            if _belongs(record, association_life, meta, source_path):
                return record
    return None


def transfer_active(name: str) -> bool:
    record = transfer_for_session(name)
    return record is not None and record.phase not in _TERMINAL


def list_incomplete() -> list[TransferRecord]:
    try:
        paths = sorted(path for path in _base().iterdir() if path.suffix == ".json")
    except FileNotFoundError:
        return []
    records = [load_transfer(path.stem) for path in paths]
    return [record for record in records if record is not None and record.phase not in _TERMINAL]


def rename_transfer(old: str, new: str) -> None:
    """Move somente a associação ativa; a fonte e a associação histórica não mudam."""
    for transfer_id in reversed(_index_ids("session", old)):
        record = load_transfer(transfer_id)
        if record and record.name == old and transfer_for_session(new, meta=None) is None:
            # O registry já moveu o sidecar; sua identidade confirma qual registro acompanha.
            from app.adapters.codex import sessions as cx_sessions
            from app.adapters.claude_headless import sessions as hl_sessions
            meta = cx_sessions.load(new) or hl_sessions.load(new)
            life = session_life(new)
            if _belongs(record, life, meta):
                save_transfer(replace(record, name=new))
                return
