"""Parser do rollout JSONL do Codex CLI -> ChatEvent (o mesmo shape neutro que o Claude produz).
Traduz o envelope `{type, payload}` do Codex; regras confirmadas contra codex-cli 0.141.0
(fixture em tests/fixtures/codex/rollout_sample.jsonl)."""
import hashlib
import json
import re

from app.transcript import ChatEvent

# message.role que sao system prompt/instrucoes internas do Codex, nao chat do usuario.
_NON_CHAT_ROLES = {"developer", "system"}

# O Codex injeta, nos 1os response_item de toda thread, role:"user" cujo conteudo e contexto interno
# (nao chat do usuario). Formatos vistos:
#   - wrapper em tag: <environment_context>...</environment_context>, <user_instructions>..., etc.
#   - blocos do host: <recommended_plugins>... e <permissions instructions>... (este ultimo usa
#     ESPACO no nome da tag, portanto nao casava o antigo `\w+_instructions`);
#   - AGENTS.md: cabeçalho com "for <path>" ou global, sem caminho e seguido de <INSTRUCTIONS>.
# Só aceita o marcador no começo, para preservar mensagens reais que apenas o mencionam.
_CONTEXT_WRAPPER_RE = re.compile(
    r"^(<(?:environment_context|recommended_plugins|[a-z][a-z_ ]*instructions)>|"
    r"# AGENTS\.md instructions(?: for |[ \t]*\r?\n\s*<INSTRUCTIONS>))")


def _is_context_wrapper(text: str) -> bool:
    return bool(_CONTEXT_WRAPPER_RE.match(text.strip()))


def _event_id(obj: dict) -> str:
    # O rollout do Codex nao tem uuid por entrada (diferente do jsonl do Claude). Hash
    # deterministico da linha inteira -> id estavel entre re-leituras (o front dedup por id).
    return hashlib.sha1(json.dumps(obj, sort_keys=True, default=str).encode()).hexdigest()


def _blocks_text(content, block_type: str) -> str:
    """Concatena o texto dos blocos `block_type` de `content`. Aceita `content` como string OU
    lista de blocos; blocos de tipo desconhecido sao ignorados sem quebrar."""
    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return ""
    return "".join(
        block.get("text", "")
        for block in content
        if isinstance(block, dict) and block.get("type") == block_type
    )


# Which tool the `exec` wrapper actually invoked. Codex calls a tool in two shapes: WRAPPED, as JS
# inside an `exec` (`tools.apply_patch(...)`), or LOOSE, as a tool of its own with JSON arguments.
# Counted per rollout on this machine, 30/08/2026: the two live SIDE BY SIDE — one 0.144.6 session
# has 65 wrapped calls and 23 loose `apply_patch`, while every 0.146.1 and 0.151.0 session here is
# wrapped only. So this is NOT a version boundary, and nothing may branch on version: what decides
# is the shape of the line in hand. Naming the inner tool lands both on ONE vocabulary, so the
# front doesn't have to know either shape exists.
_TOOL_IN_CODE_RE = re.compile(r"\btools\.(\w+)\s*\(")
# First JS string literal after the call — how `apply_patch` receives the whole patch.
_FIRST_STRING_RE = re.compile(r'"((?:[^"\\]|\\.)*)"')
# Fields of an `update_plan` item. The object is JS, not JSON (unquoted keys), so they are read one
# by one instead of parsed whole.
_STEP_RE = re.compile(r'\bstep\s*:\s*"((?:[^"\\]|\\.)*)"')
_STATUS_RE = re.compile(r'\bstatus\s*:\s*"(\w+)"')

# The `exec` command comes INSIDE JavaScript code, not as an argument:
#   const r = await tools.exec_command({cmd:"echo oi","workdir":"/tmp",...}); text(r.output);
# The key appears with and without quotes (both forms are in this machine's rollouts), and the
# value is a JS string literal — it can carry escaped quotes and backslashes (`$'1\\n2'`, `\"...\"`).
_CMD_RE = re.compile(r'\bcmd"?\s*:\s*"((?:[^"\\]|\\.)*)"')


def _command_from_code(code: str) -> str:
    """The command inside the code, or "" when it can't be extracted.

    "" is a legitimate and frequent answer: `tools.write_stdin(...)` has no `cmd` at all. The caller
    keeps the raw code in that case — an empty `command` would draw an EMPTY summary line, which is
    worse than showing the code that actually ran. Single quotes and template literals also land
    here; neither appears in this machine's rollouts, so the fallback is what covers them.
    """
    m = _CMD_RE.search(code or "")
    return _unescape_js(m.group(1)) if m else ""


# `apply_patch` is the other custom tool, and it is Codex's `Edit`. Its input is the patch itself,
# which names each file it touches on a `*** <verb> File: <path>` line.
_PATCH_FILE_RE = re.compile(r"^\*{3} (?:Add|Update|Delete) File: (.+)$", re.MULTILINE)


def _unescape_js(raw: str) -> str:
    """Contents of a JS string literal, unescaped. Falls back to the raw value.

    The JS literal matches JSON on the escapes seen here (\\" \\\\ \\n \\t \\uXXXX). On an escape
    only JS has (\\' , \\x41) json fails and the raw value serves — it is already readable.
    """
    try:
        return json.loads(f'"{raw}"')
    except (json.JSONDecodeError, ValueError):
        return raw


def _js_string(text: str) -> str:
    """First JS string literal in `text`, unescaped, or "" when there is none."""
    m = _FIRST_STRING_RE.search(text or "")
    return _unescape_js(m.group(1)) if m else ""


def _plan_from_code(code: str) -> list[dict]:
    """Plan items of an `update_plan` call, or []. Each item keeps the shape the older rollouts
    used (`step` + `status`), so both formats reach the front as the same thing.

    Walks step by step instead of splitting the text into `{...}` blocks: a step whose own words
    contain a brace ("lidar com {config}") breaks the block split, and the item would vanish from
    the task panel — the person would read a plan with a stage missing and blame the agent.
    """
    out = []
    passos = list(_STEP_RE.finditer(code or ""))
    for i, step in enumerate(passos):
        # The status of THIS item: the first one before the next step starts. Beyond it belongs to
        # the following item, and reading it here would shift every status by one.
        fim = passos[i + 1].start() if i + 1 < len(passos) else len(code)
        status = _STATUS_RE.search(code, step.end(), fim)
        out.append({"step": _unescape_js(step.group(1)),
                    "status": status.group(1) if status else "pending"})
    return out


def _files_from_patch(code: str) -> list[str]:
    """Paths a patch touches, or [] when it doesn't look like one.

    Without this the whole diff would be the summary line: the front has no `apply_patch` case, so
    it falls back to the first known key — and `file_path` is exactly that key. Handing over the raw
    patch would trade "invisible" for "truncated garbage", which is not an improvement.
    """
    return _PATCH_FILE_RE.findall(code or "")


_SCRIPT_HEADER_RE = re.compile(
    r"^Script (completed|failed)\nWall time [\d.]+ seconds\nOutput:\n")


def _command_output(value) -> tuple[str, bool] | None:
    if isinstance(value, list):
        parts = [_command_output(item) for item in value]
        if parts and all(part is not None for part in parts):
            return "\n\n".join(part[0] for part in parts), any(part[1] for part in parts)
        return None
    if not isinstance(value, dict):
        return None
    if value.get("status") == "fulfilled" and set(value) == {"status", "value"}:
        return _command_output(value["value"])
    if value.get("status") == "rejected" and set(value) == {"status", "reason"}:
        reason = value["reason"]
        return (reason if isinstance(reason, str) else json.dumps(reason, ensure_ascii=False)), True
    # Um JSON de domínio também pode ter `output`: os metadados provam o envelope do executor.
    if not (isinstance(value.get("output"), str) and "chunk_id" in value
            and "wall_time_seconds" in value):
        return None
    text = value["output"]
    failed = isinstance(value.get("exit_code"), int) and value["exit_code"] != 0
    if failed:
        text = f"exit_code: {value['exit_code']}\n{text}"
    elif value.get("session_id") is not None:
        text = f"session_id: {value['session_id']}\n{text}"
    return text, failed


def _output_result(output) -> tuple[str | None, bool]:
    raw = _output_text(output)
    if raw is None:
        return None, False
    header = _SCRIPT_HEADER_RE.match(raw)
    if not header:
        return raw, False
    failed = header.group(1) == "failed"
    # Os blocos precisam permanecer separados: dois text(obj) viram JSONs adjacentes no rollout.
    blocks = ([b["text"] for b in output if isinstance(b, dict)
               and b.get("type") == "input_text" and isinstance(b.get("text"), str)]
              if isinstance(output, list) else [raw])
    blocks[0] = blocks[0][header.end():]
    parts = []
    for block in blocks:
        if not block:
            continue
        try:
            value = json.loads(block)
        except (json.JSONDecodeError, ValueError):
            parts.append(block)
            continue
        parsed = _command_output(value)
        if parsed is None:
            parts.append(block)
        else:
            parts.append(parsed[0])
            failed = failed or parsed[1]
    return "\n\n".join(parts), failed


def _output_text(output) -> str | None:
    """Text of a tool output: a LIST of blocks or a raw string — both shapes appear in real
    rollouts, so handling only the list would leave half the results empty."""
    if output is None:
        return None
    if isinstance(output, str):
        return output
    if isinstance(output, list):
        # Same block reader the messages use, with the same type filter: a block of another type
        # entering here and NOT there would be a difference nobody wrote on purpose.
        return _blocks_text(output, "input_text")
    return str(output)


def parse_rollout_obj(obj: dict) -> list[ChatEvent]:
    """Eventos de chat de UMA linha ja parseada do rollout. So `response_item` vira chat —
    session_meta/turn_context/world_state/compacted/event_msg sao estado, nao conversa."""
    if obj.get("type") != "response_item":
        return []
    payload = obj.get("payload")
    if not isinstance(payload, dict):
        return []
    ptype = payload.get("type")

    if ptype == "message":
        role = payload.get("role")
        if role in _NON_CHAT_ROLES:
            return []
        if role == "user":
            text = _blocks_text(payload.get("content"), "input_text")
            if _is_context_wrapper(text):
                return []
            return [ChatEvent(kind="user_msg", id=_event_id(obj), text=text)]
        if role == "assistant":
            text = _blocks_text(payload.get("content"), "output_text")
            return [ChatEvent(kind="assistant_msg", id=_event_id(obj), text=text)]
        return []

    if ptype == "function_call":
        try:
            tool_input = json.loads(payload.get("arguments") or "{}")
        except (json.JSONDecodeError, ValueError):
            tool_input = {}
        return [ChatEvent(
            kind="tool_use", id=_event_id(obj),
            tool_name=payload.get("name"), tool_use_id=payload.get("call_id"),
            tool_input=tool_input if isinstance(tool_input, dict) else {},
        )]

    if ptype == "custom_tool_call":
        # `exec`, the tool Codex uses the most. Unlike `function_call`: `input` is a STRING of
        # code, not JSON arguments.
        code = payload.get("input")
        code = code if isinstance(code, str) else ""
        name = payload.get("name")
        tool_input = {"code": code}
        # The wrapper's own name (`exec`) says nothing about what ran. The inner call does, and on
        # the older shape it IS the name — so both end up here under the same one.
        #
        # `inner` is only asked for when the payload IS the wrapper. On the older shape `code` is
        # the raw patch, and a patch that happens to quote `tools.apply_patch(` — a patch editing
        # THIS file would — used to match here and send the reader hunting for a JS string in the
        # middle of a diff, dropping the real patch without a word.
        embrulhado = name == "exec"
        calls = list(_TOOL_IN_CODE_RE.finditer(code)) if embrulhado else []
        inner = calls[0] if len(calls) == 1 else None
        if inner:
            name = inner.group(1)
        # Each tool has a different salient field, and an empty value would make the front draw an
        # EMPTY summary line — worse than showing the code (see summarizeToolInput on the front).
        # `command`, `file_path` and `plan` are names the front already knows how to render.
        patch = ""
        if name == "apply_patch":
            # Older shape: `input` IS the patch. Newer shape: it is a JS string argument of
            # `tools.apply_patch(...)`, escaped — the `\n` there are two characters, so the patch
            # has to come out of the literal before anything can read its file lines.
            patch = _js_string(code[inner.end():]) if inner else code
        if name == "update_plan":
            plan = _plan_from_code(code)
            if plan:
                tool_input["plan"] = plan
        elif patch:
            tool_input["patch"] = patch
            files = _files_from_patch(patch)
            if files:
                tool_input["file_path"] = files
        elif len(calls) > 1:
            tool_input["command"] = "\n".join(
                _unescape_js(m.group(1)) for m in _CMD_RE.finditer(code)
            ) or ", ".join(dict.fromkeys(m.group(1) for m in calls))
        else:
            command = _command_from_code(code)
            if command:
                tool_input["command"] = command
        return [ChatEvent(
            kind="tool_use", id=_event_id(obj),
            tool_name=name, tool_use_id=payload.get("call_id"),
            tool_input=tool_input,
        )]

    if ptype in {"custom_tool_call_output", "function_call_output"}:
        result, failed = _output_result(payload.get("output"))
        return [ChatEvent(
            kind="tool_result", id=_event_id(obj),
            tool_use_id=payload.get("call_id"),
            result=result, is_error=failed,
        )]

    # reasoning: encrypted_content opaco no rollout -> ignora no v1 (texto legivel so ao vivo).
    return []


def parse_rollout_line(line: str) -> list[ChatEvent]:
    """Parseia uma linha crua do rollout .jsonl -> ChatEvent. Espelha transcript.parse_line."""
    line = line.strip()
    if not line:
        return []
    try:
        obj = json.loads(line)
    except (json.JSONDecodeError, ValueError):
        return []
    return parse_rollout_obj(obj)
