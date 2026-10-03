#!/usr/bin/env bash
# Garante um Chromium para o navegador do app nativo no Linux (painel, hangar-preview e tela remota no celular):
# o Chrome ou o Chromium do sistema, ou o chrome-headless-shell do Chrome for Testing baixado em
# ~/.hangar/native/chromium. Nunca falha nem pede senha: sem Chromium o app funciona, só sem o Navegador.
# A marca ~/.hangar/native/chromium-ok é a prova do passo de atualização e diz o que foi encontrado.
# O do sistema vem antes do baixado: a distro o atualiza, e o baixado só se renova quando este script roda.
# Uso: scripts/install-chromium.sh
set -uo pipefail

[ -n "${HOME:-}" ] || { echo "navegador do app nativo: sem HOME, nada a fazer"; exit 0; }
MARCA_DIR="$HOME/.hangar/native"
DESTINO="$MARCA_DIR/chromium"
VERSOES=https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json
BAIXAR=https://storage.googleapis.com/chrome-for-testing-public
# Nunca cair para http num redirecionamento: o que vem daqui é um navegador que vai abrir qualquer site.
CURL=(curl -fsSL --proto '=https' --proto-redir '=https' --retry 2 --connect-timeout 15)

marca() { mkdir -p "$MARCA_DIR"; printf '%s\n' "$1" > "$MARCA_DIR/chromium-ok"; }

# O Chromium do snap não lê o perfil em pasta oculta da home (~/.config): vale como ausente. Mesma regra do app
# (desktop-native/src/browser/chromium/launch.rs).
e_snap() {
  local real; real=$(readlink -f "$1" 2>/dev/null || echo "$1")
  case "$real" in /snap/*|*/snap) return 0 ;; esac
  head -c 4096 "$real" 2>/dev/null | grep -aq '/snap/' && return 0
  return 1
}

achar_sistema() {
  local nome bin
  for nome in google-chrome-stable google-chrome chromium chromium-browser; do
    bin=$(command -v "$nome" 2>/dev/null) || continue
    e_snap "$bin" && continue
    echo "$bin"; return 0
  done
  return 1
}

if [ "$(uname -s)" != Linux ]; then
  marca "nao e Linux: nada a fazer"
  exit 0
fi
if [ -n "${HANGAR_CHROMIUM:-}" ] && [ -x "$HANGAR_CHROMIUM" ]; then
  marca "HANGAR_CHROMIUM: $HANGAR_CHROMIUM"; exit 0
fi
if BIN=$(achar_sistema); then
  echo "navegador do app nativo: usando $BIN"
  marca "sistema: $BIN"; exit 0
fi
if [ "$(uname -m)" != x86_64 ]; then
  echo "navegador do app nativo: aviso — nenhum Chrome ou Chromium encontrado, e não há download pronto para $(uname -m)."
  echo "  O app funciona sem ele, só sem o Navegador. Instale o chromium pelo gerenciador da sua distro."
  marca "sem chromium ($(uname -m))"; exit 0
fi

ATUAL=""
[ -x "$DESTINO/chrome-headless-shell" ] && ATUAL=$(cat "$DESTINO/.versao" 2>/dev/null || echo "desconhecida")

# Já existe um baixado: falha ao renovar mantém o que está lá, que continua servindo.
falhou() {
  if [ -n "$ATUAL" ]; then
    echo "navegador do app nativo: aviso — $1; segue o chrome-headless-shell $ATUAL que já estava aqui."
    marca "baixado: $DESTINO/chrome-headless-shell ($ATUAL; renovar falhou: $1)"
    exit 0
  fi
  echo "navegador do app nativo: aviso — $1. O app funciona sem ele, só sem o Navegador."
  echo "  Para ter, instale o google-chrome ou o chromium, ou rode de novo scripts/install-chromium.sh."
  marca "download falhou: $1"
  exit 0
}

mkdir -p "$MARCA_DIR"
# Na mesma pasta do destino: o mv final é um rename, que não deixa diretório pela metade.
TMP=$(mktemp -d -p "$MARCA_DIR" .chromium-baixando.XXXXXX) || falhou "não consegui criar a pasta temporária"
trap 'rm -rf "$TMP"' EXIT
"${CURL[@]}" -o "$TMP/versoes.json" "$VERSOES" || falhou "não consegui ler as versões do Chrome for Testing"
# `"Stable":{"channel":"Stable","version":"141.0.7390.54","revision":"..."}`
VERSAO=$(tr -d '\n ' < "$TMP/versoes.json" | grep -o '"Stable":{[^}]*}' | grep -o '"version":"[0-9.]*"' | grep -o '[0-9][0-9.]*')
[[ "$VERSAO" =~ ^[0-9]+(\.[0-9]+){3}$ ]] || falhou "a lista de versões do Chrome for Testing veio sem uma versão estável válida"
if [ "$ATUAL" = "$VERSAO" ]; then
  marca "baixado: $DESTINO/chrome-headless-shell ($VERSAO)"; exit 0
fi
echo "navegador do app nativo: baixando o chrome-headless-shell $VERSAO (cerca de 100 MB)"
"${CURL[@]}" -o "$TMP/shell.zip" "$BAIXAR/$VERSAO/linux64/chrome-headless-shell-linux64.zip" \
  || falhou "o download do chrome-headless-shell $VERSAO falhou"
if command -v unzip >/dev/null 2>&1; then
  unzip -q "$TMP/shell.zip" -d "$TMP/x" || falhou "não consegui descompactar o chrome-headless-shell"
elif command -v python3 >/dev/null 2>&1; then
  python3 -m zipfile -e "$TMP/shell.zip" "$TMP/x" || falhou "não consegui descompactar o chrome-headless-shell"
  # O zipfile do Python não guarda o bit de execução.
  chmod +x "$TMP/x/chrome-headless-shell-linux64/chrome-headless-shell" "$TMP/x/chrome-headless-shell-linux64/chrome_crashpad_handler" 2>/dev/null || true
else
  falhou "falta o unzip para descompactar o chrome-headless-shell"
fi
NOVO="$TMP/x/chrome-headless-shell-linux64"
[ -x "$NOVO/chrome-headless-shell" ] || falhou "o pacote do chrome-headless-shell veio sem o executável"
# Sem libnss3/libatk o binário existe mas não abre: melhor saber agora do que gravar "baixado".
"$NOVO/chrome-headless-shell" --version >/dev/null 2>&1 \
  || falhou "o chrome-headless-shell $VERSAO não abre nesta máquina (falta alguma biblioteca do sistema?)"
printf '%s\n' "$VERSAO" > "$NOVO/.versao"
rm -rf "$DESTINO.velho"
if [ -e "$DESTINO" ]; then mv "$DESTINO" "$DESTINO.velho" || falhou "não consegui tirar o chrome-headless-shell anterior do lugar"; fi
if ! mv "$NOVO" "$DESTINO"; then
  [ -e "$DESTINO.velho" ] && mv "$DESTINO.velho" "$DESTINO"
  falhou "não consegui pôr o chrome-headless-shell $VERSAO no lugar"
fi
rm -rf "$DESTINO.velho"
echo "navegador do app nativo: chrome-headless-shell $VERSAO em $DESTINO"
marca "baixado: $DESTINO/chrome-headless-shell ($VERSAO)"
exit 0
