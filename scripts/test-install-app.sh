#!/usr/bin/env bash
# Testa o install.sh no modo --app (assistente do app nativo) sem instalar nada: o install.sh é
# copiado para um repositório falso, o HOME é temporário e o PATH só tem programas falsos e o mínimo
# do sistema. Nunca toca o checkout de verdade nem o HOME de quem roda.
#
#   ./scripts/test-install-app.sh              # roda todos os casos
#   source scripts/test-install-app.sh --lib   # só define as funções e mantém a pasta (uso real)
set -uo pipefail

LIB_ONLY=0
[ "${1:-}" = --lib ] && LIB_ONLY=1
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(mktemp -d)"
[ "$LIB_ONLY" = 1 ] || trap 'rm -rf "$ROOT"' EXIT
BASH_BIN=$(command -v bash)
fail=0

# Ferramentas reais que o install.sh usa. Fora delas não há tmux, git, sudo, gerenciador de pacotes
# nem tailscale: cada caso põe em $B os falsos que quiser.
SYS_TOOLS="bash sh env cat grep sed tr cut tail head date mkdir rm mv cp mktemp tee dirname basename uname id sleep seq awk find timeout setsid openssl python3 wc sort ls chmod ln touch readlink"

# fake <comando> <corpo sh>: programa falso que registra a chamada em $S/log/calls e roda o corpo.
fake() {
  printf '#!/bin/sh\necho "%s $*" >> "%s/log/calls"\n%s\n' "$1" "$S" "$2" > "$B/$1"
  chmod +x "$B/$1"
}

# new_sandbox <nome>: define S (raiz do caso), R (repositório falso) e B (programas falsos).
new_sandbox() {
  S="$ROOT/$1"; R="$S/repo"; B="$S/bin"
  mkdir -p "$R/backend" "$R/scripts" "$R/frontend" "$S/home" "$B" "$S/sys" "$S/log"
  cp "$REPO/install.sh" "$R/install.sh"
  local t p
  for t in $SYS_TOOLS; do
    p=$(command -v "$t" 2>/dev/null) && [ -x "$p" ] && ln -sf "$p" "$S/sys/$t"
  done
  # Sub-scripts do repositório: só registram que foram chamados.
  for t in install-native.sh install-claude-wrapper.sh services-setup.sh install-hangar-send.sh \
           install-skills-bridge.sh tmux-persist-setup.sh lan-setup.sh install-hangar-panel.sh; do
    printf '#!/bin/sh\necho "$0 $*" >> "%s/log/scripts"\nexit 0\n' "$S" > "$R/scripts/$t"
    chmod +x "$R/scripts/$t"
  done
  fake tmux 'exit 0'
  fake claude 'exit 0'
  fake uv 'exit 0'
  fake hostname 'echo 192.168.0.10'
  fake systemctl 'case "$*" in *is-active*) echo active ;; *list-unit-files*|*" cat "*) exit 1 ;; esac; exit 0'
}

# run_install <saída> [args]: sem terminal (setsid), stdin nulo, teto de tempo; TEST_TOKEN vira
# HANGAR_TOKEN. O `| cat` só termina quando o tee do log do install.sh fecha a saída.
run_install() {
  local out=$1; shift
  ( cd "$R" && env -i HOME="$S/home" PATH="$B:$S/sys" HANGAR_TOKEN="${TEST_TOKEN-}" \
      timeout 120 setsid -w "$BASH_BIN" ./install.sh "$@" </dev/null 2>&1 ) | cat > "$out"
  return "${PIPESTATUS[0]}"
}

pass()  { echo "ok   $1"; }
flunk() { echo "FAIL $1"; if [ -n "${2:-}" ] && [ -f "$2" ]; then tail -40 "$2" | sed 's/^/     | /'; fi; fail=1; }
expect_line() { if grep -Eq -- "$3" "$2" 2>/dev/null; then pass "$1"; else flunk "$1 (sem /$3/)" "$2"; fi; }
expect_no()   { if grep -Eq -- "$3" "$2" 2>/dev/null; then flunk "$1 (achou /$3/)" "$2"; else pass "$1"; fi; }
expect_eq()   { if [ "$2" = "$3" ]; then pass "$1"; else flunk "$1 (veio '$2', esperado '$3')"; fi; }
last_line()   { grep -av '^[[:space:]]*$' "$1" | tail -1; }
first_mark()  { grep -a -m1 '^##HANGAR-' "$1"; }
steps()       { grep -a '^##HANGAR-PASSO## ' "$1" | cut -d' ' -f2- | paste -sd'|'; }
# before <arquivo> <regex A> <regex B>: a primeira linha com A vem antes da primeira com B
before() {
  local a b
  a=$(grep -nE -m1 -- "$2" "$1" | cut -d: -f1); b=$(grep -nE -m1 -- "$3" "$1" | cut -d: -f1)
  [ -n "$a" ] && [ -n "$b" ] && [ "$a" -lt "$b" ]
}

[ "$LIB_ONLY" = 1 ] && return 0

APP_ARGS=(--app --tailscale=nao --sem-nativo --no-frontend)

# --- Caso: caminho feliz do assistente, e rodar de novo ---
new_sandbox feliz
TEST_TOKEN='segredo-do-app-123' run_install "$S/out" "${APP_ARGS[@]}"; rc=$?
expect_eq "feliz: sai 0" "$rc" 0
expect_eq "feliz: PROTOCOLO é a primeira marca" "$(first_mark "$S/out")" '##HANGAR-PROTOCOLO## 1'
expect_eq "feliz: FIM ok é a última linha" "$(last_line "$S/out")" '##HANGAR-FIM## ok'
expect_eq "feliz: etapas na ordem" "$(steps "$S/out")" \
  'preparar fazendo|preparar ok|instalar fazendo|celular fazendo|celular ok|instalar fazendo|instalar ok|final fazendo|final ok'
expect_line "feliz: senha do app gravada" "$R/backend/.env" '^CP_AUTH_TOKEN=segredo-do-app-123$'
expect_no "feliz: senha fora da saída" "$S/out" 'segredo-do-app-123'
expect_no "feliz: senha fora do log" "$S/home/.hangar/logs/privado/install.log" 'segredo-do-app-123'
expect_no "feliz: nenhuma pergunta ficou sem resposta" "$S/out" 'assumindo NÃO'
expect_line "feliz: ask responde sim (serviços)" "$S/log/scripts" 'services-setup.sh --backend-only'
expect_no "feliz: ask_extra responde não (persistência)" "$S/log/scripts" 'tmux-persist-setup'
expect_no "feliz: --sem-nativo não roda o install-native" "$S/log/scripts" 'install-native'
expect_no "feliz: --tailscale=nao não cria etapa" "$S/out" '^##HANGAR-PASSO## tailscale'
TEST_TOKEN='outra-senha-456' run_install "$S/out2" "${APP_ARGS[@]}"
expect_eq "de novo: FIM ok" "$(last_line "$S/out2")" '##HANGAR-FIM## ok'
expect_eq "de novo: uma linha de senha só" "$(grep -c '^CP_AUTH_TOKEN=' "$R/backend/.env")" 1
expect_line "de novo: a senha atual é mantida" "$R/backend/.env" '^CP_AUTH_TOKEN=segredo-do-app-123$'
expect_no "de novo: a senha nova não vaza" "$S/out2" 'outra-senha-456'

# --- Caso: senha com espaço no meio e acento chega ao backend igual ---
new_sandbox especial
TEST_TOKEN='minha senha ção' run_install "$S/out" "${APP_ARGS[@]}"
# O backend lê o .env pelo python-dotenv; sem o venv aqui, confere a linha gravada.
PY="$REPO/backend/.venv/bin/python"
if [ -x "$PY" ] && "$PY" -c 'import dotenv' 2>/dev/null; then
  expect_eq "especial: o backend lê igual" \
    "$("$PY" -c 'import sys; from dotenv import dotenv_values; print(dotenv_values(sys.argv[1])["CP_AUTH_TOKEN"])' "$R/backend/.env")" 'minha senha ção'
else
  expect_eq "especial: gravada literal" "$(grep '^CP_AUTH_TOKEN=' "$R/backend/.env")" 'CP_AUTH_TOKEN=minha senha ção'
fi

# --- Caso: senha que o .env não guarda inteira é recusada ---
for t in 'abc #def' 'x${HOME}y' '"com-aspas"' ' espaco-inicial'; do
  new_sandbox recusada
  TEST_TOKEN=$t run_install "$S/out" "${APP_ARGS[@]}"; rc=$?
  expect_eq "recusada [$t]: falha" "$rc" 1
  expect_line "recusada [$t]: diz quais caracteres" "$S/out" '^##HANGAR-FALHA## .*nem espaço no começo ou no fim$'
  expect_eq "recusada [$t]: FIM falhou" "$(last_line "$S/out")" '##HANGAR-FIM## falhou'
  if grep -qF -- "$t" "$S/out"; then flunk "recusada [$t]: senha na saída" "$S/out"; else pass "recusada [$t]: senha fora da saída"; fi
  rm -rf "$S"
done

# --- Caso: sem senha do app, gera uma aleatória e não mostra ---
new_sandbox aleatoria
TEST_TOKEN='' run_install "$S/out" "${APP_ARGS[@]}"
tok=$(grep '^CP_AUTH_TOKEN=' "$R/backend/.env" | cut -d= -f2-)
expect_eq "aleatória: 48 caracteres" "${#tok}" 48
expect_no "aleatória: fora da saída" "$S/out" "${tok:-nada-gravado}"

# --- Caso: senha curta do app para a instalação ---
new_sandbox curta
TEST_TOKEN='1234' run_install "$S/out" "${APP_ARGS[@]}"; rc=$?
expect_eq "curta: falha" "$rc" 1
expect_eq "curta: FIM falhou" "$(last_line "$S/out")" '##HANGAR-FIM## falhou'

# --- Caso: --tailscale=nao vale mesmo com a Tailscale instalada ---
new_sandbox ts-nao
fake tailscale 'exit 0'
run_install "$S/out" "${APP_ARGS[@]}"
expect_no "ts-nao: tailscale nunca é chamada" "$S/log/calls" '^tailscale '
expect_no "ts-nao: sem etapa tailscale" "$S/out" '^##HANGAR-PASSO## tailscale'

# --- Caso: falha essencial termina com PASSO falhou e FIM falhou ---
new_sandbox uv-falha
fake uv '[ "$1" = sync ] && exit 1; exit 0'
run_install "$S/out" "${APP_ARGS[@]}"; rc=$?
expect_eq "uv-falha: sai com erro" "$rc" 1
expect_line "uv-falha: FALHA com o texto de sempre" "$S/out" '^##HANGAR-FALHA## uv sync falhou — o backend ficou sem as dependências$'
expect_line "uv-falha: etapa instalar falhou" "$S/out" '^##HANGAR-PASSO## instalar falhou$'
expect_eq "uv-falha: FIM falhou é a última linha" "$(last_line "$S/out")" '##HANGAR-FIM## falhou'

# --- Caso: pendência termina com FIM pendente (e sai 1, como hoje) ---
new_sandbox pendente
fake systemctl 'case "$*" in *is-active*) echo inactive ;; *list-unit-files*|*" cat "*) exit 1 ;; esac; exit 0'
run_install "$S/out" "${APP_ARGS[@]}"; rc=$?
expect_eq "pendente: sai 1" "$rc" 1
expect_line "pendente: etapa final pendente" "$S/out" '^##HANGAR-PASSO## final pendente$'
expect_eq "pendente: FIM pendente" "$(last_line "$S/out")" '##HANGAR-FIM## pendente'

# --- Caso: --check --app só confere ---
new_sandbox check
run_install "$S/out" --check --app --no-frontend; rc=$?
expect_eq "check: sai 0" "$rc" 0
expect_eq "check: PROTOCOLO primeiro" "$(first_mark "$S/out")" '##HANGAR-PROTOCOLO## 1'
expect_eq "check: etapas" "$(steps "$S/out")" 'preparar fazendo|preparar ok'
expect_eq "check: FIM ok" "$(last_line "$S/out")" '##HANGAR-FIM## ok'
if [ ! -e "$R/backend/.env" ] && [ ! -e "$S/home/.hangar" ]; then pass "check: nada escrito"; else flunk "check: nada escrito"; fi

# --- Caso: sem --app a saída não ganha marcas do assistente ---
new_sandbox terminal
run_install "$S/out" --check --no-frontend
expect_no "terminal: sem marcas do assistente" "$S/out" '^##HANGAR-(PROTOCOLO|PASSO|ITEM|LINK|PENDENCIA|FIM)'

# --- Caso: --tailscale com valor inválido ---
new_sandbox ts-invalido
run_install "$S/out" --app --tailscale=talvez; rc=$?
expect_eq "ts-inválido: recusa" "$rc" 1
expect_line "ts-inválido: diz o que aceita" "$S/out" '--tailscale aceita sim ou nao'

# --- fim dos casos ---
if [ "$fail" = 0 ]; then echo "tudo ok"; else echo "houve falha"; fi
exit "$fail"
