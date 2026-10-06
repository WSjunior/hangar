#!/usr/bin/env python3
"""Gera os dados de teste da superfície remota dos mods a partir das conversas gravadas pela sonda.

Uso: python3 limpar_sonda.py <pasta com as conversas .jsonl da sonda>

Entram só os pedidos `ui_*` da superfície e as respostas deles, o pedido `ui_copy` do Claude Code com a
resposta da sonda, e os avisos `system` de interface, sem `uuid` e `session_id`. Ficam de fora o
`initialize` (que traz a conta logada e a lista de comandos da máquina), as anotações da sonda, o stderr
e qualquer outro evento. Só biblioteca padrão."""
import json
import sys
from pathlib import Path

ARQUIVOS = {
    "vitrine-desktop.jsonl": "vitrine.jsonl",
    "sem-plugins-desktop.jsonl": "sem-plugins.jsonl",
    "sem-vitrine-desktop.jsonl": "sem-vitrine.jsonl",
}
AVISOS = {"ui_panes", "ui_invalidate", "ui_toast", "ui_status"}


def limpar(linhas):
    pedidos = set()
    saida = []
    for bruta in linhas:
        registro = json.loads(bruta)
        direcao, msg = registro.get("dir"), registro.get("msg")
        if direcao not in ("out", "in") or not isinstance(msg, dict):
            continue
        tipo = msg.get("type")
        if tipo == "control_request":
            if not str((msg.get("request") or {}).get("subtype", "")).startswith("ui_"):
                continue
            pedidos.add(json.dumps(msg.get("request_id")))
            saida.append({"dir": direcao, "msg": msg})
        elif tipo == "control_response":
            if json.dumps((msg.get("response") or {}).get("request_id")) in pedidos:
                saida.append({"dir": direcao, "msg": msg})
        elif tipo == "system" and msg.get("subtype") in AVISOS:
            limpo = {chave: valor for chave, valor in msg.items() if chave not in ("uuid", "session_id")}
            saida.append({"dir": direcao, "msg": limpo})
    return saida


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit("uso: limpar_sonda.py <pasta das conversas da sonda>")
    origem, destino = Path(sys.argv[1]), Path(__file__).resolve().parent
    for nome, saida in ARQUIVOS.items():
        linhas = limpar((origem / nome).read_text(encoding="utf-8").splitlines())
        texto = "".join(json.dumps(linha, ensure_ascii=False, separators=(",", ":")) + "\n" for linha in linhas)
        (destino / saida).write_text(texto, encoding="utf-8")
        print(f"{saida}: {len(linhas)} linhas")


if __name__ == "__main__":
    main()
