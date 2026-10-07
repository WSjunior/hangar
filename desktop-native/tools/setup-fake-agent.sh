#!/usr/bin/env bash
# Dublê do Claude Code para provar "Pedir ajuda ao agente" sem chamar modelo nenhum:
#   HANGAR_SETUP_FAKE_CLAUDE=desktop-native/tools/setup-fake-agent.sh
# Responde ao `auth status --json` como logado e, no `-p`, lê o prompt da entrada e imprime eventos stream-json.
# Como HANGAR_SETUP_FAKE_CODEX responde ao `login status` (sai 0) e, no `exec`, imprime os eventos `--json` do Codex.
# FAKE_AGENT_MODE: fora (padrão: "conserta" fora da pasta criando $FAKE_FIXED) | dentro (só edita install.sh na pasta)
#                  | trava (edita install.sh e dorme até ser parado). FAKE_AGENT_PROMPT: arquivo onde guardar o prompt.
set -u
if [ "${1:-}" = auth ]; then echo '{"loggedIn":true,"authMethod":"claude.ai"}'; exit 0; fi
if [ "${1:-}" = login ]; then echo 'Logged in using ChatGPT'; exit 0; fi
codex=0; [ "${1:-}" = exec ] && codex=1
# O último --add-dir é a pasta do Hangar (no Codex ainda vem o "-" da entrada depois dele).
dest=""; prev=""
for a in "$@"; do [ "$prev" = --add-dir ] && dest=$a; prev=$a; done
[ $codex = 0 ] && dest="${!#}"
prompt=$(cat)
[ -n "${FAKE_AGENT_PROMPT:-}" ] && printf '%s' "$prompt" > "$FAKE_AGENT_PROMPT"
ev() { printf '%s\n' "$1"; sleep "${FAKE_PAUSE:-1}"; }
cmd() {
  if [ $codex = 1 ]; then ev "{\"type\":\"item.started\",\"item\":{\"type\":\"command_execution\",\"command\":\"$1\"}}"
  else ev "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"$1\"}}]}}"; fi
}
edit() {
  if [ $codex = 1 ]; then ev "{\"type\":\"item.completed\",\"item\":{\"type\":\"file_change\",\"changes\":[{\"path\":\"$1\"}]}}"
  else ev "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"$1\"}}]}}"; fi
}
done_text() {
  if [ $codex = 1 ]; then ev "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"$1\"}}"
  else ev "{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"$1\"}"; fi
}
if [ $codex = 1 ]; then ev '{"type":"thread.started"}'; else ev '{"type":"system","subtype":"init"}'; fi
cmd "systemctl --user status"
case "${FAKE_AGENT_MODE:-fora}" in
  fora)
    cmd "touch ${FAKE_FIXED:?}"
    touch "$FAKE_FIXED"
    done_text '**Achei:** o serviço do usuário não subia.\n\n**Fiz:** liguei o serviço do usuário. Nada pendente.'
    ;;
  dentro)
    edit "$dest/install.sh"
    printf '# conserto do dublê\n' >> "$dest/install.sh"
    done_text '**Achei:** o instalador não trata este Linux.\n\n**Fiz:** ajustei o install.sh; isso precisa entrar no Hangar.'
    ;;
  trava)
    edit "$dest/install.sh"
    printf '# edição que ficou no meio\n' >> "$dest/install.sh"
    sleep 600
    ;;
esac
