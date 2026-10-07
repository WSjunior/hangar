"""Golden das políticas puras que o ator Rust roda no lugar do Python (prepare_prompt, format_status,
skill_catalog). A saída vem das funções Python reais; o teste Rust compara byte a byte.

Uso, de backend/: uv run python tests/fixtures/contract/gen_local_policy.py
"""
import base64
import json
import os
import sys
import tempfile
import time
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[2]))

TZ = "BRT3"          # UTC-3 sem horário de verão: prova que a hora é a local, não UTC
NOW = 1_800_000_000.0
PNG = base64.b64encode(bytes.fromhex(
    "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d49444154789c6360000002000001"
    "e221bc330000000049454e44ae426082")).decode()
MIB = 1024 * 1024


def _sub(value, root):
    if isinstance(value, str):
        return value.replace("{ROOT}", root)
    if isinstance(value, list):
        return [_sub(item, root) for item in value]
    if isinstance(value, dict):
        return {key: _sub(item, root) for key, item in value.items()}
    return value


def _materialize(files, root):
    for rel, spec in (files or {}).items():
        path = Path(root) / rel
        if spec.get("dir"):
            path.mkdir(parents=True, exist_ok=True)
            continue
        path.parent.mkdir(parents=True, exist_ok=True)
        if "b64" in spec:
            path.write_bytes(base64.b64decode(spec["b64"]))
        elif "size" in spec:
            path.write_bytes(b"\0" * spec["size"])
        else:
            path.write_text(spec["text"], encoding="utf-8")


def _claude_status(payload, meta, windows, now):
    from app.adapters.claude_headless.adapter import ClaudeHeadlessAdapter, _hora_local
    data = SimpleNamespace(model=payload.get("model"), effort=payload.get("effort"), usage=payload.get("usage"),
        context_window=payload.get("context_window"), cost=payload.get("cost"), meta=meta,
        janelas=[SimpleNamespace(**window) for window in windows])
    rate = payload.get("rate_limit_info") or {}
    with patch("time.time", return_value=now):
        line = ClaudeHeadlessAdapter.status_line(None, data)
    return {"status_line": line, "limit_reset": _hora_local(rate.get("resetsAt")) if rate.get("status") == "rejected" else None}


def reference(case, root):
    """O que o `runtime_policy.run` devolvia antes da migração, chamando as funções que ficaram."""
    kind, payload, meta = case["kind"], case["payload"], case["meta"]
    now = case.get("now", NOW)
    try:
        if kind == "prepare_prompt":
            text = payload.get("text")
            if not isinstance(text, str):
                raise ValueError("entrada sem texto")
            if meta["provider"] == "claude":
                from app import uds_messaging
                from app.adapters.claude_headless.adapter import _blocos_do_prompt
                content, notices = _blocos_do_prompt(text)
                sender, _ = uds_messaging.separar_prefixo(text)
                return {"content": content, "notices": notices, "native_candidate": sender is not None}
            return {"input": [{"type": "text", "text": text}],
                    "skill_name": text.lstrip().split()[0][1:] if text.lstrip().startswith("/") else None}
        if kind == "skill_catalog":
            from app.adapters.codex.chat_controls import skills_do_catalogo
            skills = skills_do_catalogo(payload["catalog"])
            return {"skill": next((skill for skill in skills if skill["name"] == payload.get("name")), None)}
        if kind == "format_status":
            if meta["provider"] == "codex":
                from app.adapters.codex.adapter import format_status_line
                return {"status_line": format_status_line(payload.get("model") or payload.get("default_model"),
                    payload.get("effort") or payload.get("default_effort"), payload.get("token_usage"),
                    payload.get("rate_limits"), now=now)}
            return _claude_status(payload, meta, (case.get("quota") or {}).get("windows", []), now)
    except Exception:
        return {"error": True}
    raise ValueError(kind)


def run_case(case):
    with tempfile.TemporaryDirectory() as root:
        _materialize(case.get("files"), root)
        live = _sub(case, root)
        saved = {key: os.environ.get(key) for key in ("CLAUDE_CODE_EFFORT_LEVEL", "HOME")}
        os.environ.pop("CLAUDE_CODE_EFFORT_LEVEL", None)
        os.environ["HOME"] = str(Path(root) / "home")
        os.environ.update(live.get("env") or {})
        try:
            expected = reference(live, root)
        finally:
            for key, value in saved.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value
    # O texto do Codex volta como veio, com a raiz temporária dentro: o golden guarda o marcador.
    return {**case, "expected": json.loads(json.dumps(expected, ensure_ascii=False).replace(json.dumps(root)[1:-1], "{ROOT}"))}


CLAUDE = {"provider": "claude"}


def prepare_cases():
    out = []

    def add(name, text, files=None, provider="claude", meta=None):
        out.append({"name": name, "kind": "prepare_prompt", "payload": {"text": text}, "files": files,
                    "meta": {**(meta or {}), "provider": provider}})

    png = {"b64": PNG}
    add("plain", "só texto, sem imagem")
    add("one_image", "veja 📎 imagem: {ROOT}/a.png", {"a.png": png})
    add("two_images", "x 📎 imagem: {ROOT}/a.jpg 📎 imagem: {ROOT}/b.webp", {"a.jpg": png, "b.webp": png})
    add("uppercase_suffix", "📎 imagem: {ROOT}/A.PNG", {"A.PNG": png})
    add("missing_file", "📎 imagem: {ROOT}/nope.png")
    add("one_ok_one_missing", "📎 imagem: {ROOT}/a.gif 📎 imagem: {ROOT}/nope.jpeg", {"a.gif": png})
    add("directory_named_like_image", "📎 imagem: {ROOT}/dir.png", {"dir.png": {"dir": True}})
    add("over_cap_7mb", "📎 imagem: {ROOT}/big.png", {"big.png": {"size": 7 * MIB}})
    add("over_cap_by_one_byte", "📎 imagem: {ROOT}/big.png", {"big.png": {"size": 5 * MIB + 1}})
    add("trailing_punctuation", "olha (📎 imagem: {ROOT}/a.png).", {"a.png": png})
    add("path_with_space", "📎 imagem: {ROOT}/with space.png", {"with space.png": png})
    add("first_word_fallback", "📎 imagem: {ROOT}/a.png e mais texto colado", {"a.png": png})
    add("unsupported_suffix", "📎 imagem: {ROOT}/a.txt", {"a.txt": png})
    add("no_suffix", "📎 imagem: {ROOT}/semsufixo", {"semsufixo": png})
    add("multiline", "legenda\n📎 imagem: {ROOT}/a.png\noutra linha 📎 imagem: {ROOT}/b.png", {"a.png": png, "b.png": png})
    add("newline_after_marker", "📎 imagem:\n {ROOT}/a.png", {"a.png": png})
    add("marker_without_path", "📎 imagem:")
    add("marker_blanks_only", "📎 imagem:   \n")
    add("marker_no_space", "📎imagem:{ROOT}/a.png", {"a.png": png})
    add("emoji_without_label", "📎 {ROOT}/a.png", {"a.png": png})
    add("prefix_de", "[de: sessao-x] oi")
    add("prefix_grupo", "[grupo: g1]\nmsg")
    add("prefix_painel", "[painel: Meu painel] aviso")
    add("prefix_blank_name", "[de:  ] x")
    add("prefix_not_at_start", "texto [de: x] oi")
    add("prefix_leading_space", " [de: x] oi")
    add("prefix_unknown_tag", "[para: x] oi")
    add("prefix_with_image", "[de: x] 📎 imagem: {ROOT}/a.png", {"a.png": png})
    add("unicode_text", "ação — 日本語 📎 imagem: {ROOT}/ü.png", {"ü.png": png})
    out.append({"name": "text_not_a_string", "kind": "prepare_prompt", "payload": {"text": 5}, "meta": CLAUDE})
    out.append({"name": "text_missing", "kind": "prepare_prompt", "payload": {}, "meta": CLAUDE})
    add("codex_plain", "olá", provider="codex")
    add("codex_slash", "/minha-skill arg1 arg2", provider="codex")
    add("codex_slash_leading_blank", "  \n /outra\tx", provider="codex")
    add("codex_slash_only", "/", provider="codex")
    add("codex_slash_with_image_marker", "/s 📎 imagem: {ROOT}/a.png", {"a.png": png}, provider="codex")
    add("codex_prefix_is_not_native", "[de: x] oi", provider="codex")
    return out


def status_cases():
    out = []
    usage = {"input_tokens": 1500, "cache_creation_input_tokens": 1000, "cache_read_input_tokens": 0, "output_tokens": 500}

    def add(name, payload, meta=None, quota=None, env=None, files=None, now=NOW):
        case = {"name": name, "kind": "format_status", "payload": payload, "meta": {"provider": "claude", **(meta or {})},
                "now": now}
        if quota is not None:
            case["quota"] = {"windows": quota}
        if env:
            case["env"] = env
        if files:
            case["files"] = files
        out.append(case)

    for model in ("claude-opus-5", "claude-opus-4-7", "claude-opus-5[1m]", "claude-opus-5[1M]", "claude-sonnet-4-5-20250929",
                  "claude-haiku-5", "claude-fable-5-1", "claude-fable-5-1[1m]", "opus", "sonnet[1m]", "claude-sonnet",
                  "gpt-5.5", "  claude-opus-5  ", "claude-opus-4-1-20250805", "[1m]", "claude-"):
        add(f"model_{model.strip() or 'blank'}", {"model": model, "effort": "high"})
    add("model_engine_slash_with_account", {"model": "deepseek/deepseek-v4", "effort": "low"}, meta={"engine_account": "ds"})
    add("model_engine_slash_without_account", {"model": "deepseek/deepseek-v4", "effort": "low"})
    add("model_engine_family_after_slash", {"model": "anthropic/claude-sonnet-4-6"}, meta={"engine_account": "x"},
        env={"CLAUDE_CODE_EFFORT_LEVEL": "max"})
    add("model_empty", {"model": ""})
    add("model_null_everything_null", {})
    add("effort_from_env", {"model": "claude-opus-5"}, env={"CLAUDE_CODE_EFFORT_LEVEL": "xhigh"})
    add("effort_session_beats_env", {"model": "claude-opus-5", "effort": "low"}, env={"CLAUDE_CODE_EFFORT_LEVEL": "xhigh"})
    cfg = {"cfg/settings.json": {"text": json.dumps({"effortLevel": "medium"})}}
    add("effort_from_account_settings", {"model": "claude-opus-5"}, meta={"config_dir": "{ROOT}/cfg"}, files=cfg)
    add("effort_env_beats_settings", {"model": "claude-opus-5"}, meta={"config_dir": "{ROOT}/cfg"}, files=cfg,
        env={"CLAUDE_CODE_EFFORT_LEVEL": "high"})
    add("effort_settings_not_json", {"model": "claude-opus-5"}, meta={"config_dir": "{ROOT}/cfg"},
        files={"cfg/settings.json": {"text": "{nao"}})
    add("effort_settings_not_object", {"model": "claude-opus-5"}, meta={"config_dir": "{ROOT}/cfg"},
        files={"cfg/settings.json": {"text": "[1]"}})
    add("effort_settings_empty_level", {"model": "claude-opus-5"}, meta={"config_dir": "{ROOT}/cfg"},
        files={"cfg/settings.json": {"text": json.dumps({"effortLevel": ""})}})
    add("effort_settings_level_not_string", {"model": "claude-opus-5"}, meta={"config_dir": "{ROOT}/cfg"},
        files={"cfg/settings.json": {"text": json.dumps({"effortLevel": 3})}})
    add("effort_settings_missing", {"model": "claude-opus-5"}, meta={"config_dir": "{ROOT}/vazio"})
    add("effort_from_home_default", {"model": "claude-opus-5"},
        files={"home/.claude/settings.json": {"text": json.dumps({"effortLevel": "low"})}})

    ctx = {"model": "claude-opus-5", "effort": "high", "context_window": 200000}
    add("usage_basic", {**ctx, "usage": usage})
    add("usage_tie_rounds_to_even", {**ctx, "usage": {"input_tokens": 2500}})
    add("usage_tie_rounds_to_even_odd", {**ctx, "usage": {"input_tokens": 1500, "output_tokens": 3500}})
    add("usage_millions", {**ctx, "usage": {"input_tokens": 1_234_567, "output_tokens": 2_500_000}, "context_window": 2_500_000})
    add("usage_below_1000", {**ctx, "usage": {"input_tokens": 999, "output_tokens": 1}, "context_window": 999})
    add("usage_float_values", {**ctx, "usage": {"input_tokens": 1499.5, "output_tokens": 0.5}})
    add("usage_zero", {**ctx, "usage": {"input_tokens": 0}})
    add("usage_null_values", {**ctx, "usage": {"input_tokens": None, "cache_read_input_tokens": 12000}})
    add("usage_without_window", {"model": "claude-opus-5", "usage": usage})
    add("usage_window_zero", {"model": "claude-opus-5", "usage": usage, "context_window": 0})
    add("usage_null", {**ctx, "usage": None})
    add("usage_cache_only", {**ctx, "usage": {"cache_read_input_tokens": 150_000, "output_tokens": 800}})

    for cost in (0, 0.0, 0.125, 0.375, 0.005, 1.005, 2.675, 12.3456, 3, 1234.5, 0.994999):
        add(f"cost_{cost!r}", {"model": "claude-opus-5", "cost": cost})
    add("cost_null", {"model": "claude-opus-5", "cost": None})
    add("cost_only", {"cost": 0.5})

    windows = [{"rotulo": "5h", "pct": 42, "reset_ts": NOW + 90, "por_modelo": False},
               {"rotulo": "7d", "pct": 7.5, "reset_ts": NOW + 2 * 86400 + 3600, "por_modelo": False}]
    add("windows_both", {"model": "claude-opus-5"}, quota=windows)
    add("windows_five_hours_only", {"model": "claude-opus-5"}, quota=windows[:1])
    add("windows_without_model", {}, quota=windows)
    add("windows_empty", {"model": "claude-opus-5"}, quota=[])
    add("windows_unknown_label_skipped", {"model": "claude-opus-5"},
        quota=[{"rotulo": "1h", "pct": 10, "reset_ts": None}, windows[0]])
    for index, (pct, reset) in enumerate(((0.5, None), (1.5, NOW), (2.5, NOW - 100), (99.5, NOW + 59), (100, NOW + 3600),
                                          (41.5, NOW + 3600 * 5 + 60), (12, NOW + 86400), (12, NOW + 86400 * 9 + 3 * 3600 + 59),
                                          (12, NOW + 60), (12, NOW + 3599.9), (33, 0), (33, NOW + 0.5))):
        add(f"window_pct_reset_{index}", {"model": "claude-opus-5"}, quota=[{"rotulo": "5h", "pct": pct, "reset_ts": reset}])
    add("window_seven_days_hours_only", {}, quota=[{"rotulo": "7d", "pct": 5, "reset_ts": NOW + 3 * 86400}])
    add("windows_with_usage_cost_and_model", {"model": "claude-sonnet-4-5", "effort": "medium", "usage": usage,
        "context_window": 1000000, "cost": 4.2}, quota=windows)

    rate = lambda status, resets: {"rate_limit_info": {"status": status, "resetsAt": resets}}
    add("limit_rejected", {"model": "claude-opus-5", **rate("rejected", NOW + 5 * 3600)})
    add("limit_rejected_float", {"model": "claude-opus-5", **rate("rejected", NOW + 12345.678)})
    add("limit_rejected_local_midnight", {**rate("rejected", 1_800_000_000 - 1_800_000_000 % 86400 + 3 * 3600 - 1)})
    add("limit_allowed_ignores_reset", {**rate("allowed", NOW)})
    add("limit_rejected_without_reset", {"rate_limit_info": {"status": "rejected"}})
    add("limit_rejected_reset_text", {**rate("rejected", "amanhã")})
    add("limit_null_info", {"model": "claude-opus-5", "rate_limit_info": None})
    add("limit_info_not_an_object", {"model": "claude-opus-5", "rate_limit_info": "boom"})
    add("limit_rejected_negative_epoch", {**rate("rejected", -3600)})
    return out


def codex_cases():
    out = []

    def add(name, payload, now=NOW):
        out.append({"name": name, "kind": "format_status", "payload": payload, "meta": {"provider": "codex"}, "now": now})

    usage = {"last": {"inputTokens": 14389, "outputTokens": 1200}, "modelContextWindow": 258400}
    primary = {"windowDurationMins": 300, "usedPercent": 41.6, "resetsAt": NOW + 3 * 3600 + 120}
    secondary = {"windowDurationMins": 10080, "usedPercent": 12, "resetsAt": NOW + 5 * 86400}
    add("full", {"model": "GPT-5.5", "effort": "high", "token_usage": usage, "rate_limits": {"primary": primary, "secondary": secondary}})
    add("only_model", {"model": "gpt-5.5"})
    add("model_default_fallback", {"model": None, "default_model": "gpt-5.5", "effort": None, "default_effort": "medium"})
    add("model_blank_falls_back", {"model": "", "default_model": "gpt-5.5", "effort": "", "default_effort": "low"})
    add("model_beats_default", {"model": "a", "default_model": "b", "effort": "x", "default_effort": "y"})
    add("effort_without_model", {"effort": "high"})
    add("effort_default_without_model", {"default_effort": "high"})
    add("everything_missing", {})
    add("usage_needs_window", {"token_usage": {"last": {"inputTokens": 1}}})
    add("usage_needs_input", {"token_usage": {"last": {"outputTokens": 1}, "modelContextWindow": 1000}})
    add("usage_zero_input", {"token_usage": {"last": {"inputTokens": 0}, "modelContextWindow": 1000}})
    add("usage_without_output", {"token_usage": {"last": {"inputTokens": 2500}, "modelContextWindow": 1_000_000}})
    add("usage_empty_object", {"token_usage": {}})
    add("windows_five_hours_edges", {"rate_limits": {"primary": {"windowDurationMins": 270, "usedPercent": 1}, "secondary": {"windowDurationMins": 330, "usedPercent": 2}}})
    add("windows_outside_ranges", {"rate_limits": {"primary": {"windowDurationMins": 269, "usedPercent": 1}, "secondary": {"windowDurationMins": 331, "usedPercent": 2}}})
    add("windows_seven_days_edges", {"rate_limits": {"primary": {"windowDurationMins": 10020, "usedPercent": 1}, "secondary": {"windowDurationMins": 10140, "usedPercent": 2}}})
    add("windows_seven_days_outside", {"rate_limits": {"primary": {"windowDurationMins": 10019, "usedPercent": 1}, "secondary": {"windowDurationMins": 10141, "usedPercent": 2}}})
    add("window_missing_percent", {"rate_limits": {"primary": {"windowDurationMins": 300}}})
    add("window_missing_minutes", {"rate_limits": {"primary": {"usedPercent": 5}}})
    add("window_no_reset", {"rate_limits": {"primary": {"windowDurationMins": 300, "usedPercent": 50.5}}})
    add("window_banker_percent", {"rate_limits": {"primary": {"windowDurationMins": 300, "usedPercent": 0.5}, "secondary": {"windowDurationMins": 10080, "usedPercent": 2.5}}})
    add("window_reset_in_past", {"rate_limits": {"primary": {"windowDurationMins": 300, "usedPercent": 5, "resetsAt": NOW - 50}}})
    add("window_reset_days_only", {"rate_limits": {"secondary": {"windowDurationMins": 10080, "usedPercent": 5, "resetsAt": NOW + 2 * 86400}}})
    add("window_reset_hours_only", {"rate_limits": {"secondary": {"windowDurationMins": 10080, "usedPercent": 5, "resetsAt": NOW + 7200}}})
    add("window_reset_minutes", {"rate_limits": {"primary": {"windowDurationMins": 300, "usedPercent": 5, "resetsAt": NOW + 1799}}})
    add("rate_limits_empty", {"rate_limits": {}})
    add("rate_limits_null_window", {"rate_limits": {"primary": None, "secondary": secondary}})
    return out


def catalog_cases():
    out = []

    def skill(name, path, enabled=True, **extra):
        return {"name": name, "path": path, "enabled": enabled, **extra}

    def add(name, catalog, wanted):
        out.append({"name": name, "kind": "skill_catalog", "payload": {"catalog": catalog, "name": wanted},
                    "meta": {"provider": "codex"}})

    group = {"data": [{"skills": [skill("beta", "/s/beta", description="B"), skill("alpha", "/s/alpha")]}]}
    add("found", group, "alpha")
    add("not_found", group, "gamma")
    add("name_null", group, None)
    twins = {"data": [{"skills": [skill("dup", "/a/dup"), skill("solo", "/s/solo")]}, {"skills": [skill("dup", "/b/dup")]}]}
    add("homonyms_first", twins, "dup:" + __import__("hashlib").sha256(b"/a/dup").hexdigest()[:8])
    add("homonyms_second", twins, "dup:" + __import__("hashlib").sha256(b"/b/dup").hexdigest()[:8])
    add("homonyms_bare_name_not_found", twins, "dup")
    add("disabled_and_incomplete_skipped", {"data": [{"skills": [skill("off", "/s/off", False), skill("", "/s/x"),
        skill("nopath", ""), {"name": "nokey", "path": "/s/nokey"}, skill("on", "/s/on")]}]}, "on")
    add("disabled_not_found", {"data": [{"skills": [skill("off", "/s/off", False)]}]}, "off")
    add("same_path_listed_twice_last_wins", {"data": [{"skills": [skill("old", "/s/p", description="1")]},
        {"skills": [skill("new", "/s/p", description="2")]}]}, "new")
    add("empty_data", {"data": []}, "x")
    add("no_data_key", {}, "x")
    add("group_without_skills", {"data": [{}]}, "x")
    add("unicode_name_sorted_by_code_point", {"data": [{"skills": [skill("zé", "/s/1"), skill("Zebra", "/s/2"),
        skill("ze", "/s/3"), skill("é", "/s/4")]}]}, "zé")
    out.append({"name": "catalog_missing", "kind": "skill_catalog", "payload": {"name": "x"}, "meta": {"provider": "codex"}})
    return out


def write(out_dir):
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    os.environ["TZ"] = TZ
    time.tzset()
    groups = {"prepare_prompt.json": prepare_cases(), "format_status_claude.json": status_cases(),
              "format_status_codex.json": codex_cases(), "skill_catalog.json": catalog_cases()}
    for name, cases in groups.items():
        rows = [run_case(case) for case in cases]
        (out_dir / name).write_text(json.dumps({"tz": TZ, "cases": rows}, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")


if __name__ == "__main__":
    write(HERE / "local_policy")
