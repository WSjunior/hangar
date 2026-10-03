"""Recibo por ocorrência posterior ao despacho, sem inferir reenvio pela ausência."""
from __future__ import annotations

import copy
import hashlib
import json
import os
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


def _anchor(data: bytes, offset: int):
    if offset < 0 or offset > len(data):
        return None
    return hashlib.sha1(data[max(0, offset - 256):offset]).hexdigest()


class ReceiptIndex:
    def __init__(self, provider: str, conversation: str):
        if provider not in {"claude", "codex"}:
            raise ValueError("provedor fora do escopo")
        self.provider, self.conversation = provider, conversation
        self.data = b""
        self.file_identity = None
        self.occurrences = []
        self.scan_offset = 0

    def capture(self, path: Path) -> dict:
        try:
            with Path(path).open("rb") as stream:
                identity = _identity(stream)
                offset = stream.seek(0, os.SEEK_END)
                stream.seek(max(0, offset - 256))
                data = stream.read(min(offset, 256))
        except FileNotFoundError:
            identity, data, offset = None, b"", 0
        return {"conversation": self.conversation, "file_identity": identity,
                "offset": offset, "anchor": _anchor(data, len(data))}

    def scan(self, path: Path) -> list[dict]:
        try:
            with Path(path).open("rb") as stream:
                identity = _identity(stream)
                size = os.fstat(stream.fileno()).st_size
                unchanged = identity == self.file_identity and size >= len(self.data)
                if unchanged:
                    stream.seek(max(0, len(self.data) - 256))
                    unchanged = stream.read(min(len(self.data), 256)) == self.data[-256:]
                if unchanged:
                    stream.seek(len(self.data))
                    data = self.data + stream.read()
                    occurrences = list(self.occurrences)
                    start_offset = self.scan_offset
                else:
                    stream.seek(0)
                    data, occurrences, start_offset = stream.read(), [], 0
            with Path(path).open("rb") as current:
                if _identity(current) != identity or os.fstat(current.fileno()).st_size < len(data):
                    raise OSError("transcript mudou durante a leitura")
        except FileNotFoundError:
            self.data, self.file_identity, self.occurrences = b"", None, []
            self.scan_offset = 0
            return []
        from app.pqueue import _ts_of_obj
        from app.adapters.codex.rollout import parse_rollout_obj
        offset = start_offset
        complete_offset = start_offset
        for raw in data[start_offset:].splitlines(keepends=True):
            start, offset = offset, offset + len(raw)
            if not raw.endswith(b"\n"):
                break
            complete_offset = offset
            try:
                obj = json.loads(raw)
            except (ValueError, UnicodeError):
                continue
            if not isinstance(obj, dict):
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
            occurrences.append({"id": f"{self.conversation}|{identity}|{record_id}",
                "conversation": self.conversation, "file_identity": identity, "offset": start,
                "end_offset": offset, "text": text, "kind": kind, "timestamp": _ts_of_obj(obj) or None})
        self.data, self.file_identity, self.occurrences = data, identity, occurrences
        self.scan_offset = complete_offset
        return copy.deepcopy(occurrences)

    def match_after(self, cursor: dict, row: dict, used_occurrences: dict) -> dict | None:
        from app.pqueue import _chaves_de_commit, _linhas_da_entrada
        if cursor["conversation"] != self.conversation or not cursor["file_identity"] or cursor["file_identity"] != self.file_identity:
            return None
        observed = _anchor(self.data, cursor["offset"])
        if observed is None or observed != cursor["anchor"]:
            return None
        for occurrence in self.occurrences:
            if occurrence["id"] in used_occurrences or occurrence["offset"] < cursor["offset"]:
                continue
            matches = _linhas_da_entrada(row) & _chaves_de_commit(occurrence["text"])
            if matches:
                return {"cursor": copy.deepcopy(cursor), "occurrence": copy.deepcopy(occurrence),
                        "normalized_text": sorted(matches)[0], "observed_anchor": observed}
        return None


def validate_proof(proof: dict, cursor: dict, row: dict) -> bool:
    from app.pqueue import _chaves_de_commit, _linhas_da_entrada
    occurrence = proof["occurrence"]
    return (proof["cursor"] == cursor and cursor["file_identity"] is not None
        and occurrence["conversation"] == cursor["conversation"]
        and occurrence["file_identity"] == cursor["file_identity"]
        and occurrence["offset"] >= cursor["offset"] and occurrence["end_offset"] > occurrence["offset"]
        and occurrence["kind"] in {"user", "dequeue", "steer"}
        and proof["observed_anchor"] == cursor["anchor"]
        and proof["normalized_text"] in _linhas_da_entrada(row) & _chaves_de_commit(occurrence["text"]))
