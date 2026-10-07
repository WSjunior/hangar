#!/usr/bin/env bash
# hangar — clonar e instalar numa linha só, no Linux/macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/jeffer1312/hangar/main/bootstrap.sh | bash
#
# Com argumentos: sob `curl | bash` eles vão DEPOIS de `-s --`, senão quem os come é o próprio
# bash, não este script.
#
#   curl -fsSL .../bootstrap.sh | bash -s -- ~/apps/hangar
#   curl -fsSL .../bootstrap.sh | bash -s -- --check
#   curl -fsSL .../bootstrap.sh | bash -s -- ~/apps/hangar --no-frontend
#
# O PRIMEIRO argumento é a pasta de destino (default: $HOME/hangar). Se ele começar
# com '-', já é flag e o destino fica no default. Tudo o que sobra vai inteiro pro ./install.sh.
#
# O assistente do app nativo roda este arquivo com a pasta do Hangar que encontrou (quando há uma)
# e --app; aí as falhas daqui também saem marcadas (##HANGAR-ERRO##, ##HANGAR-FIM##) e o install.sh
# nunca recebe o terminal.
#
# Instale num disco LOCAL. Numa pasta compartilhada por rede (Samba/NFS montado de outra
# máquina) o `uv sync` e o `npm ci` recriariam `backend/.venv` e `frontend/node_modules` por
# cima dos da máquina de origem — e esses são dela, não seus: o venv aponta pro
# `/usr/bin/python3.14` de lá e o node_modules traz o binário `@esbuild/linux-x64` dela.
# Instalar de uma segunda máquina quebra a instalação da primeira.
set -euo pipefail

REPO_URL="https://github.com/jeffer1312/hangar.git"
RAMO="main"
APP=0
for a in "$@"; do case $a in --app) APP=1 ;; esac; done
# Senha do celular e auxiliar de administrador do app: só o install.sh as recebe, não o git.
APP_TOKEN=${HANGAR_TOKEN-}; APP_ASKPASS=${HANGAR_ASKPASS-}; APP_ASKPASS_CODE=${HANGAR_ASKPASS_CODE-}
unset HANGAR_TOKEN HANGAR_ASKPASS HANGAR_ASKPASS_CODE

say()  { printf '\n\033[1m%s\033[0m\n' "$*"; }
ok()   { printf '  \033[32mok\033[0m  %s\n' "$*"; }
nota() { printf '      \033[2m%s\033[0m\n' "$*"; }
fail() {
  printf '  \033[31mX\033[0m   %s\n' "$*" >&2
  # No --app a última linha diz ao assistente que parou aqui, antes do install.sh.
  [ "$APP" = 1 ] && echo "##HANGAR-FIM## falhou"
  exit 1
}
# O código vem logo antes da falha, também no terminal: o app troca o texto pela frase e pelo botão.
fail_with() { # fail_with <código ou vazio> <mensagem>
  [ -n "$1" ] && echo "##HANGAR-ERRO## $1"
  shift; fail "$*"
}
# Vazio quando a internet responde: aí a falha é de outra coisa.
net_code() {
  command -v curl >/dev/null || return 0
  curl -fsS --max-time 10 -o /dev/null https://github.com 2>/dev/null || echo sem-internet
}

DEST="$HOME/hangar"
case "${1-}" in
  ''|-*) ;;               # sem argumento, ou o primeiro já é flag do install.sh
  *)     DEST=$1; shift ;;
esac

# Mesma detecção do `detecta_pkg` do install.sh, e duplicada de propósito: aqui o repositório
# ainda NÃO existe, então não há de onde importá-la. Vazio = gerenciador não reconhecido.
sugere_git() {
  for p in pacman apt-get dnf zypper apk brew; do
    command -v "$p" >/dev/null || continue
    case "$p" in
      pacman)  echo "sudo pacman -S --needed git" ;;
      apt-get) echo "sudo apt-get install -y git" ;;
      dnf)     echo "sudo dnf install -y git" ;;
      zypper)  echo "sudo zypper install -y git" ;;
      apk)     echo "sudo apk add git" ;;
      brew)    echo "brew install git" ;;   # brew nunca com sudo, de propósito
    esac
    return
  done
}

# 0 = o remoto desta pasta é o hangar. Checar o REMOTO, não só a existência da pasta:
# "tem um .git aqui" não quer dizer que é este projeto, e dar pull no repo errado é pior que
# parar.
mesma_origem() {
  local url
  url=$(git -C "$1" remote get-url origin 2>/dev/null) || return 1
  url=${url%/}; url=${url%.git}
  case "$url" in
    *github.com[:/]jeffer1312/hangar) return 0 ;;
  esac
  return 1
}

clona() {
  say "Clonando em $DEST"
  git clone --branch "$RAMO" "$REPO_URL" "$DEST" || fail_with "$(net_code)" "git clone falhou"
  ok "clonado"
}

say "hangar — instalação em uma linha"

if ! command -v git >/dev/null; then
  cmd=$(sugere_git)
  if [ -n "$cmd" ]; then nota "instale com:  $cmd"
  else nota "instale o git pelo gerenciador de pacotes do teu sistema"; fi
  fail "git é obrigatório — sem ele não há como clonar o repositório"
fi
ok "git $(git --version | awk '{print $3}')"

if [ ! -e "$DEST" ]; then
  clona
elif [ ! -d "$DEST" ]; then
  fail "$DEST existe e não é uma pasta — escolha outro destino"
elif mesma_origem "$DEST"; then
  ok "$DEST já é este repositório — atualizando em vez de clonar"
  # No --app o pull não roda o hook post-merge: ele faria um --update inteiro, sem marcas, antes do
  # ##HANGAR-PROTOCOLO##; o install.sh --app logo abaixo já faz o mesmo trabalho.
  sem_hooks=''; [ "$APP" = 1 ] && sem_hooks='-c core.hooksPath=/dev/null'
  # shellcheck disable=SC2086  # de propósito: vazio some, cheio vira dois argumentos
  if ! git $sem_hooks -C "$DEST" pull --ff-only origin "$RAMO"; then
    # Mudança local é o caso comum, e tem frase e botão próprios no app.
    if [ -n "$(git -C "$DEST" status --porcelain --untracked-files=no 2>/dev/null)" ]; then
      fail_with checkout-sujo "git pull falhou em $DEST: há mudança local — resolva na mão e rode de novo"
    fi
    fail_with "$(net_code)" "git pull falhou em $DEST — resolva na mão e rode de novo"
  fi
  ok "atualizado"
elif [ -n "$(ls -A "$DEST" 2>/dev/null)" ]; then
  nota "outro destino: bootstrap.sh ~/apps/hangar"
  fail "$DEST já existe e NÃO é o hangar — não vou mexer no que é seu"
else
  clona   # pasta existe mas está vazia: o git clona pra dentro dela
fi

say "Instalando: ./install.sh $*"
cd "$DEST"
[ -n "$APP_TOKEN" ] && export HANGAR_TOKEN=$APP_TOKEN
[ -n "$APP_ASKPASS" ] && export HANGAR_ASKPASS=$APP_ASKPASS
[ -n "$APP_ASKPASS_CODE" ] && export HANGAR_ASKPASS_CODE=$APP_ASKPASS_CODE
# Sob `curl | bash` o stdin DESTE script é o cano do curl, e o install.sh herdaria isso: os
# `read` dele leriam EOF na hora. Aqui entregamos o terminal de verdade. (O install.sh também
# se defende sozinho — isto é o cinto além do suspensório.) Sem terminal, ele cai no default
# seguro por conta própria. No --app nunca: quem responde é o assistente.
if [ "$APP" = 0 ] && { : </dev/tty; } 2>/dev/null; then
  exec ./install.sh "$@" </dev/tty
else
  exec ./install.sh "$@"
fi
