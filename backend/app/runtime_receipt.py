"""Recibo por ocorrência posterior ao despacho, sem inferir reenvio pela ausência."""
from __future__ import annotations

import copy
import hashlib
import json
import os
import time
from pathlib import Path


def _identity(stream) -> str:
    if os.name != "nt":
        stat = os.fstat(stream.fileno())
        return f"{stat.st_dev:x}:{stat.st_ino:x}"
    import ctypes
    import msvcrt

    class FileIdInfo(ctypes.Structure):
        _fields_ = [("volume", ctypes.c_uint64), ("file_id", ctypes.c_ubyte * 16)]

    get_info = ctypes.WinDLL("kernel32", use_last_error=True).GetFileInformationByHandleEx
    get_info.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
    get_info.restype = ctypes.c_int
    info = FileIdInfo()
    if not get_info(msvcrt.get_osfhandle(stream.fileno()), 18, ctypes.byref(info), ctypes.sizeof(info)):
        raise ctypes.WinError(ctypes.get_last_error())
    return f"{info.volume:x}:{int.from_bytes(bytes(info.file_id), 'little'):x}"


def _anchor(data: bytes) -> str:
    return hashlib.sha1(data).hexdigest()


def _bytes_before(stream, offset: int) -> bytes | None:
    """Os até 256 bytes que terminam em `offset`; None se o arquivo é menor que isso."""
    if offset > os.fstat(stream.fileno()).st_size:
        return None
    stream.seek(max(0, offset - 256))
    return stream.read(min(offset, 256))


class ReceiptIndex:
    """Só o necessário para continuar a leitura: identidade, até onde leu e os 256 bytes antes
    disso. Guardar o transcript inteiro custava o tamanho dele em memória e um arquivo relido."""

    def __init__(self, provider: str, conversation: str):
        if provider not in {"claude", "codex"}:
            raise ValueError("provedor fora do escopo")
        self.provider, self.conversation = provider, conversation
        self.path = None
        self.file_identity = None
        self.tail = b""
        self.occurrences = []
        # Conversa do `session_meta` do rollout: o Codex não grava a conversa em cada linha.
        self.meta_conversation = None
        self.scan_offset = 0

    def capture(self, path: Path) -> dict:
        try:
            with Path(path).open("rb") as stream:
                identity = _identity(stream)
                offset = stream.seek(0, os.SEEK_END)
                data = _bytes_before(stream, offset) or b""
        except FileNotFoundError:
            identity, data, offset = None, b"", 0
        cursor = {"conversation": self.conversation, "file_identity": identity,
                  "offset": offset, "anchor": _anchor(data)}
        if identity is None:
            cursor["absent_since"] = time.time()
        return cursor

    def scan(self, path: Path) -> list[dict]:
        """Lê só o que foi acrescentado; troca de arquivo ou reescrita relê do início."""
        from app.pqueue import _ts_of_obj
        from app.adapters.codex.rollout import parse_rollout_obj
        self.path = path
        try:
            with Path(path).open("rb") as stream:
                identity = _identity(stream)
                if identity != self.file_identity or _bytes_before(stream, self.scan_offset) != self.tail:
                    self.occurrences, self.scan_offset, self.meta_conversation = [], 0, None
                stream.seek(self.scan_offset)
                offset = self.scan_offset
                for raw in stream:
                    if not raw.endswith(b"\n"):
                        break
                    start, offset = offset, offset + len(raw)
                    try:
                        obj = json.loads(raw)
                    except (ValueError, UnicodeError):
                        continue
                    if not isinstance(obj, dict):
                        continue
                    if self.provider == "codex" and obj.get("type") == "session_meta":
                        meta = (obj.get("payload") or {}).get("id")
                        self.meta_conversation = meta if isinstance(meta, str) else None
                        continue
                    text, kind = None, "user"
                    if self.provider == "codex":
                        text = "\n".join(event.text for event in parse_rollout_obj(obj)
                                         if event.kind == "user_msg" and event.text)
                    elif obj.get("type") == "user":
                        content = (obj.get("message") or {}).get("content")
                        text = content if isinstance(content, str) else "\n".join(
                            block["text"] for block in content or [] if isinstance(block, dict)
                            and block.get("type") == "text" and isinstance(block.get("text"), str))
                    elif obj.get("type") == "queue-operation" and obj.get("operation") == "dequeue":
                        text, kind = obj.get("content"), "dequeue"
                    elif obj.get("type") == "attachment" and (obj.get("attachment") or {}).get("type") == "queued_command":
                        text, kind = "\n".join(block["text"] for block in obj["attachment"].get("prompt") or []
                            if isinstance(block, dict) and block.get("type") == "text" and isinstance(block.get("text"), str)), "steer"
                    if not isinstance(text, str) or not text.strip():
                        continue
                    provider_id = obj.get("uuid") or obj.get("id") or (obj.get("payload") or {}).get("id")
                    record_id = f"id:{provider_id}" if isinstance(provider_id, str) and provider_id else f"offset:{start}"
                    recorded = self.meta_conversation if self.provider == "codex" else obj.get("sessionId")
                    occurrence = {"id": f"{self.conversation}|{identity}|{record_id}",
                        "conversation": self.conversation, "file_identity": identity, "offset": start,
                        "end_offset": offset, "text": text, "kind": kind, "timestamp": _ts_of_obj(obj) or None,
                        "recorded_conversation": recorded if isinstance(recorded, str) else None}
                    if self.provider == "codex" and occurrence["recorded_conversation"] is None:
                        occurrence["identity_unprovable"] = True
                    self.occurrences.append(occurrence)
                with Path(path).open("rb") as current:
                    if _identity(current) != identity or os.fstat(current.fileno()).st_size < offset:
                        self.file_identity = None
                        raise OSError("transcript mudou durante a leitura")
                self.tail = _bytes_before(stream, offset) or b""
                self.file_identity, self.scan_offset = identity, offset
        except FileNotFoundError:
            self.file_identity, self.tail, self.occurrences, self.scan_offset = None, b"", [], 0
            self.meta_conversation = None
        return list(self.occurrences)

    def match_after(self, cursor: dict, row: dict, used_occurrences: dict) -> dict | None:
        """A âncora do cursor é relida do arquivo: confere que os bytes antes do despacho não mudaram."""
        from app.pqueue import _chaves_de_commit, _linhas_da_entrada
        # Cursor sem arquivo: o despacho veio antes de o transcript existir (primeira mensagem da
        # sessão), então tudo no arquivo da mesma conversa é posterior a ele.
        born_after = cursor["file_identity"] is None and cursor["offset"] == 0
        if (cursor["conversation"] != self.conversation or self.file_identity is None
                or not born_after and cursor["file_identity"] != self.file_identity or self.path is None
                or cursor["offset"] > self.scan_offset):
            return None
        try:
            with Path(self.path).open("rb") as stream:
                if _identity(stream) != self.file_identity:
                    return None
                before = _bytes_before(stream, cursor["offset"])
        except FileNotFoundError:
            return None
        if before is None:
            return None
        observed = _anchor(before)
        if observed != cursor["anchor"]:
            return None
        for occurrence in self.occurrences:
            if occurrence["id"] in used_occurrences or occurrence["offset"] < cursor["offset"]:
                continue
            if not _cursor_accepts(cursor, occurrence):
                continue
            matches = _linhas_da_entrada(row) & _chaves_de_commit(occurrence["text"])
            if matches:
                return {"cursor": copy.deepcopy(cursor), "occurrence": copy.deepcopy(occurrence),
                        "normalized_text": sorted(matches)[0], "observed_anchor": observed}
        return None


def validate_proof(proof: dict, cursor: dict, row: dict) -> bool:
    from app.pqueue import _chaves_de_commit, _linhas_da_entrada
    occurrence = proof["occurrence"]
    return (proof["cursor"] == cursor and _cursor_accepts(cursor, occurrence)
        and occurrence["conversation"] == cursor["conversation"]
        and occurrence["offset"] >= cursor["offset"] and occurrence["end_offset"] > occurrence["offset"]
        and occurrence["kind"] in {"user", "dequeue", "steer"}
        and proof["observed_anchor"] == cursor["anchor"]
        and proof["normalized_text"] in _linhas_da_entrada(row) & _chaves_de_commit(occurrence["text"]))


def _cursor_accepts(cursor: dict, occurrence: dict) -> bool:
    if cursor["file_identity"] is not None:
        return occurrence["file_identity"] == cursor["file_identity"]
    if cursor["offset"] != 0:
        return False
    since = cursor.get("absent_since")
    if since is None:
        # Cursor gravado antes do `absent_since`: tudo no arquivo da conversa é posterior a ele.
        return True
    if occurrence.get("identity_unprovable"):
        # Rollout do Codex sem `session_meta`: não há como provar a conversa; vale o cursor sem arquivo.
        return True
    # Arquivo novo só comprova a conversa explícita e uma ocorrência posterior ao despacho.
    timestamp = occurrence.get("timestamp")
    return (occurrence.get("recorded_conversation") == cursor["conversation"]
        and type(since) in {int, float} and type(timestamp) in {int, float} and timestamp >= since)
