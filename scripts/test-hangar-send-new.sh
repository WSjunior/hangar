#!/usr/bin/env bash
# Trava o que o hangar-send manda ao backend — o POST /api/sessions do `--new` e o POST /input do
# recado com HANGAR_SEND_PAINEL=1 —, contra um backend FALSO que só grava o corpo. É shell, a suíte
# pytest não alcança; e o script de verdade é copiado (não colado aqui), pra o teste quebrar junto
# com ele.
#
# Existe pelo --headless: só a tela de criar sessão abria sessão sem terminal, e o flag faltando no
# script fazia quem orquestra por hangar-send cair calado numa sessão com terminal.
#
# Uso: ./scripts/test-hangar-send-new.sh
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
falhas=0

# O hangar-send chama `python3`. No Windows ele pode ser o atalho da Microsoft Store, que existe no
# PATH e não executa nada: acha um Python que RODA e o expõe como python3 só pra este teste.
PY=""
for c in python3 python py; do
    if "$c" -c 'pass' >/dev/null 2>&1; then PY="$(command -v "$c")"; break; fi
done
[[ -n "$PY" ]] || { echo "FALHA: nenhum Python que execute no PATH"; exit 1; }
mkdir -p "$TMP/bin"
printf '#!/bin/sh\nexec "%s" "$@"\n' "$PY" > "$TMP/bin/python3"
chmod +x "$TMP/bin/python3"
export PATH="$TMP/bin:$PATH"

PORTA=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
mkdir -p "$TMP/scripts" "$TMP/backend"
cp "$REPO/scripts/hangar-send" "$TMP/scripts/hangar-send"
printf 'CP_AUTH_TOKEN=teste\nCP_PORT=%s\n' "$PORTA" > "$TMP/backend/.env"

python3 - "$PORTA" "$TMP/corpo.json" <<'PY' &
import http.server, sys
porta, destino = int(sys.argv[1]), sys.argv[2]
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.send_header("Content-Type", "application/json"); self.end_headers()
        self.wfile.write(b'{"claude":{"disponivel":true,"default":true},"codex":{"disponivel":true,"default":false}}')
    def do_POST(self):
        corpo = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        open(destino, "wb").write(corpo)
        self.send_response(200); self.send_header("Content-Type", "application/json"); self.end_headers()
        self.wfile.write(b'{"avisos":["aviso-teste"]}')
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", porta), H).serve_forever()
PY
SERVIDOR=$!
trap 'kill "$SERVIDOR" 2>/dev/null; rm -rf "$TMP"' EXIT
no_ar=""
for _ in $(seq 1 50); do
    python3 -c 'import socket,sys; socket.create_connection(("127.0.0.1", int(sys.argv[1])), 0.2)' "$PORTA" 2>/dev/null && { no_ar=1; break; }
    sleep 0.1
done
# Sem isto a falha aparecia como "valor diferente" nos casos, e não como o que é.
[[ -n "$no_ar" ]] || { echo "FALHA: o backend falso não subiu na porta $PORTA"; exit 1; }

campo() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(json.dumps(d.get(sys.argv[2])))' "$TMP/corpo.json" "$1"; }
checa() { # <caso> <esperado> <obtido>
    if [[ "$2" == "$3" ]]; then printf 'ok   %s -> %s\n' "$1" "$3"
    else printf 'FALHA %s -> esperava %s, veio %s\n' "$1" "$2" "$3"; falhas=$((falhas + 1)); fi
}

# 1. --headless vai no corpo, junto com o modelo.
rm -f "$TMP/corpo.json"
bash "$TMP/scripts/hangar-send" --new hl-x /tmp --headless --model haiku >/dev/null 2>&1
checa "--headless no corpo" 'true' "$(campo headless 2>/dev/null)"
checa "--model junto" '"haiku"' "$(campo model 2>/dev/null)"

# 2. Sem o flag, a chave nem vai (o default do backend é false).
rm -f "$TMP/corpo.json"
bash "$TMP/scripts/hangar-send" --new sem-flag /tmp >/dev/null 2>&1
checa "sem --headless, sem a chave" 'null' "$(campo headless 2>/dev/null)"

# Auxiliares herdam o padrão; gravá-lo exige escolha explícita.
checa "sem flag, não grava provedor" 'null' "$(campo remember_provider 2>/dev/null)"
resultado=$(bash "$TMP/scripts/hangar-send" --new lembrar /tmp --provider codex --remember-provider 2>&1)
checa "--remember-provider no corpo" 'true' "$(campo remember_provider 2>/dev/null)"
[[ "$resultado" == *"aviso-teste"* ]]
checa "aviso da criação aparece" '0' "$?"

# 3. Codex também aceita o modo sem terminal.
rm -f "$TMP/corpo.json"
bash "$TMP/scripts/hangar-send" --new hl-codex /tmp --headless --provider codex --permissao "Full Access" >/dev/null 2>&1
checa "--headless com codex: código" '0' "$?"
checa "--headless com codex: provider" '"codex"' "$(campo provider 2>/dev/null)"
checa "--headless com codex: flag" 'true' "$(campo headless 2>/dev/null)"
checa "--headless com codex: permissão" '"Full Access"' "$(campo permission_mode 2>/dev/null)"

# 4. Repetido é erro, como os outros flags.
bash "$TMP/scripts/hangar-send" --new hl-2x /tmp --headless --headless >/dev/null 2>&1
checa "--headless duas vezes: código" '2' "$?"

# 5. O --help documenta o flag (a ajuda é o cabeçalho do script).
bash "$REPO/scripts/hangar-send" --help 2>/dev/null | grep -q -- '--headless'
checa "--help menciona --headless" '0' "$?"

# 6. HANGAR_SEND_PAINEL=1: aviso de programa vai como está, sem [de: …] e sem procurar o remetente.
rm -f "$TMP/corpo.json"
HANGAR_SEND_PAINEL=1 bash "$TMP/scripts/hangar-send" alvo "[painel: orquestrador g1] oi" >/dev/null 2>&1
checa "painel: código" '0' "$?"
checa "painel: texto intacto" '"[painel: orquestrador g1] oi"' "$(campo text 2>/dev/null)"
checa "painel: orienta o turno" 'true' "$(campo steer 2>/dev/null)"

# 7. Sem o rótulo de painel, ou para outro servidor, recusa sem enviar nada.
rm -f "$TMP/corpo.json"
HANGAR_SEND_PAINEL=1 bash "$TMP/scripts/hangar-send" alvo "oi" >/dev/null 2>&1
checa "painel sem [painel:]: código" '2' "$?"
checa "painel sem [painel:]: nada enviado" 'ausente' "$([[ -e "$TMP/corpo.json" ]] && echo enviado || echo ausente)"
HANGAR_SEND_PAINEL=1 bash "$TMP/scripts/hangar-send" srv::alvo "[painel: x] oi" >/dev/null 2>&1
checa "painel para outro servidor: código" '2' "$?"

# 8. O --help documenta a variável (a ajuda é o cabeçalho do script).
bash "$REPO/scripts/hangar-send" --help 2>/dev/null | grep -q HANGAR_SEND_PAINEL
checa "--help menciona HANGAR_SEND_PAINEL" '0' "$?"

if [[ $falhas -gt 0 ]]; then echo "$falhas falha(s)"; exit 1; fi
echo "tudo ok"
