#!/usr/bin/env bash
# Testa o bootstrap.sh sem clonar nada: git e curl falsos no PATH e um install.sh falso no destino,
# que grava os argumentos e se recebeu terminal.
#
#   ./scripts/test-bootstrap-app.sh
#   source scripts/test-bootstrap-app.sh --lib   # só as funções, pasta mantida (uso real)
set -uo pipefail

LIB_ONLY=0
[ "${1:-}" = --lib ] && LIB_ONLY=1
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(mktemp -d)"
[ "$LIB_ONLY" = 1 ] || trap 'rm -rf "$ROOT"' EXIT
BASH_BIN=$(command -v bash)
fail=0
SYS_TOOLS="bash sh env cat mkdir cp printf awk dirname ls timeout setsid rm"

# new_box <nome>: define S (raiz do caso) e B (programas falsos)
new_box() {
  S="$ROOT/$1"; B="$S/bin"
  mkdir -p "$B" "$S/sys" "$S/home"
  local t p
  for t in $SYS_TOOLS; do
    p=$(command -v "$t" 2>/dev/null) && [ -x "$p" ] && ln -sf "$p" "$S/sys/$t"
  done
  cat > "$S/install-fake.sh" <<'EOF'
#!/bin/sh
d=$(dirname "$0")
printf '%s\n' "$@" > "$d/args"
if [ -t 0 ]; then echo tty > "$d/stdin"; else echo notty > "$d/stdin"; fi
exit 0
EOF
  chmod +x "$S/install-fake.sh"
  cat > "$B/git" <<'EOF'
#!/bin/sh
echo "git $*" >> "$FAKE_LOG"
case "$*" in
  *"remote get-url origin"*) echo https://github.com/jeffer1312/hangar.git ;;
  *"pull --ff-only"*) exit "$FAKE_PULL_RC" ;;
  *"status --porcelain"*) [ "$FAKE_DIRTY" = 1 ] && echo " M install.sh"; exit 0 ;;
  --version) echo "git version 2.50.0" ;;
  clone*)
    [ "$FAKE_CLONE_RC" = 0 ] || exit 128
    for a in "$@"; do dest=$a; done
    mkdir -p "$dest" && cp "$FAKE_INSTALL" "$dest/install.sh" ;;
esac
exit 0
EOF
  printf '#!/bin/sh\nexit "$FAKE_CURL_RC"\n' > "$B/curl"
  chmod +x "$B/git" "$B/curl"
}
# repo_existente <pasta>: o destino já é um checkout do Hangar (o git falso confirma a origem)
repo_existente() { mkdir -p "$1/.git"; cp "$S/install-fake.sh" "$1/install.sh"; }
# run_boot <saída> [args]; FAKE_PULL_RC, FAKE_DIRTY, FAKE_CLONE_RC, FAKE_CURL_RC vêm de quem chama
run_boot() {
  local out=$1; shift
  ( env -i HOME="$S/home" PATH="$B:$S/sys" FAKE_LOG="$S/git.log" FAKE_INSTALL="$S/install-fake.sh" \
      FAKE_PULL_RC="${FAKE_PULL_RC:-0}" FAKE_DIRTY="${FAKE_DIRTY:-0}" FAKE_CLONE_RC="${FAKE_CLONE_RC:-0}" \
      FAKE_CURL_RC="${FAKE_CURL_RC:-0}" timeout 60 setsid -w "$BASH_BIN" "$REPO/bootstrap.sh" "$@" </dev/null 2>&1 ) > "$out"
}

pass()  { echo "ok   $1"; }
flunk() { echo "FAIL $1"; if [ -n "${2:-}" ] && [ -f "$2" ]; then sed 's/^/     | /' "$2" | tail -30; fi; fail=1; }
expect_line() { if grep -Eq -- "$3" "$2" 2>/dev/null; then pass "$1"; else flunk "$1 (sem /$3/)" "$2"; fi; }
expect_no()   { if grep -Eq -- "$3" "$2" 2>/dev/null; then flunk "$1 (achou /$3/)" "$2"; else pass "$1"; fi; }
expect_eq()   { if [ "$2" = "$3" ]; then pass "$1"; else flunk "$1 (veio '$2', esperado '$3')"; fi; }
last_line()   { grep -av '^[[:space:]]*$' "$1" | tail -1; }

[ "$LIB_ONLY" = 1 ] && return 0

# --- Caso: pasta existente do Hangar, opções repassadas, sem terminal ---
new_box repasse
D="$S/hangar"; repo_existente "$D"
run_boot "$S/out" "$D" --app --tailscale=nao --sem-nativo --agentes=codex,pi
expect_eq "repasse: opções intactas" "$(paste -sd' ' "$D/args")" '--app --tailscale=nao --sem-nativo --agentes=codex,pi'
expect_eq "repasse: install.sh sem terminal" "$(cat "$D/stdin")" notty
expect_line "repasse: atualizou em vez de clonar" "$S/git.log" 'pull --ff-only origin main'

# --- Caso: pasta com espaço e acento ---
new_box acento
D="$S/Área de trabalho/hangar"; repo_existente "$D"
run_boot "$S/out" "$D" --app --tailscale=sim
expect_eq "acento: opções intactas" "$(paste -sd' ' "$D/args")" '--app --tailscale=sim'

# --- Caso: mudança local barra o pull ---
new_box sujo
D="$S/hangar"; repo_existente "$D"
FAKE_PULL_RC=1 FAKE_DIRTY=1 run_boot "$S/out" "$D" --app
expect_line "sujo: código" "$S/out" '^##HANGAR-ERRO## checkout-sujo$'
expect_eq "sujo: FIM falhou é a última linha" "$(last_line "$S/out")" '##HANGAR-FIM## falhou'
[ -e "$D/args" ] && flunk "sujo: install.sh não roda" || pass "sujo: install.sh não roda"

# --- Caso: clone sem internet ---
new_box sem-internet
FAKE_CLONE_RC=1 FAKE_CURL_RC=7 run_boot "$S/out" "$S/novo" --app
expect_line "sem-internet: código" "$S/out" '^##HANGAR-ERRO## sem-internet$'
expect_eq "sem-internet: FIM falhou" "$(last_line "$S/out")" '##HANGAR-FIM## falhou'

# --- Caso: clone feliz numa pasta nova ---
new_box clone
run_boot "$S/out" "$S/novo" --app --tailscale=nao
expect_eq "clone: opções repassadas" "$(paste -sd' ' "$S/novo/args")" '--app --tailscale=nao'

# --- Caso: sem --app o código sai, a FIM não ---
new_box terminal
D="$S/hangar"; repo_existente "$D"
FAKE_PULL_RC=1 FAKE_DIRTY=1 run_boot "$S/out" "$D"
expect_line "terminal: código também aqui" "$S/out" '^##HANGAR-ERRO## checkout-sujo$'
expect_no "terminal: sem FIM" "$S/out" '^##HANGAR-FIM##'

# --- fim dos casos ---
if [ "$fail" = 0 ]; then echo "tudo ok"; else echo "houve falha"; fi
exit "$fail"
