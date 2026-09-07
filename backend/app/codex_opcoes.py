"""Contexto estendido pela configuração oficial, respeitando o teto nativo de cada modelo."""
import json
import tomllib

from app.codex_arquivos import exclusivo, gravar, json_bytes, json_obj, ler

CONTEXTO = 1_000_000


def ler_opcoes(servico) -> dict:
    raw = ler(servico.codex_home / "config.toml")
    config = tomllib.loads(raw.decode()) if raw else {}
    modelos = []
    try:
        catalogo = json.loads((servico.codex_home / "models_cache.json").read_text())
        for model in catalogo.get("models", []):
            base, limite = model.get("context_window"), model.get("max_context_window")
            if isinstance(base, int) and isinstance(limite, int) and limite > base:
                modelos.append({"model": model["slug"], "default": base, "max": limite})
    except (OSError, ValueError, KeyError, TypeError):
        pass
    atual = config.get("model_context_window")
    return {"contexto_estendido": isinstance(atual, int) and atual >= CONTEXTO,
            "contexto_configurado": atual, "compactacao": config.get("model_auto_compact_token_limit"),
            "modelos": modelos}


async def salvar_opcoes(servico, habilitado: bool) -> dict:
    async with exclusivo(servico.codex_home / ".hangar-integracao.lock"):
        servico.raiz.mkdir(parents=True, exist_ok=True, mode=0o700)
        path = servico.raiz / "contexto-anterior.json"

        def preparar(config):
            atual = config.get("model_context_window")
            registro = json_obj(path)
            if habilitado:
                if isinstance(atual, int) and atual >= CONTEXTO:
                    return [], lambda: None
                # Guarda antes de gravar a config: uma queda não perde o valor a restaurar.
                registro = {"anterior": atual}
                gravar(path, json_bytes(registro), ler(path))
                querido = CONTEXTO
            else:
                if not isinstance(atual, int) or atual < CONTEXTO:
                    return [], lambda: None
                querido = registro.get("anterior")
            edits = [{"keyPath": "model_context_window", "value": querido, "mergeStrategy": "replace"}]
            return edits, lambda: None

        await servico._editar_config(preparar)
    return ler_opcoes(servico)
