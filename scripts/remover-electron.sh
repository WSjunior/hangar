#!/usr/bin/env bash
# Tira o Electron (shell/) das máquinas Linux que ainda o têm: o app nativo já atende tudo o que só ele fazia
# aqui (o navegador do hangar-preview e a tela remota no celular). Roda pelo passo de atualização.
#
# Nunca falha: passo que falha derruba o Atualizar inteiro. Sem o app nativo ou sem um Chromium para o navegador
# dele, não mexe em nada (o Electron segue sendo o único navegador da máquina). Com o Electron aberto tira só o
# lançador; o shell/node_modules fica até a próxima vez com ele fechado. O código de shell/ fica (o hangar-preview
# importa módulos dele), e o ~/.config/Electron também (o app nativo importa servidores e aparência de lá).
# Uso: scripts/remover-electron.sh
set -uo pipefail
cd "$(dirname "$0")/.."

MARCA_DIR="$HOME/.hangar/native"
NATIVO="$HOME/.local/bin/hangar-native"
MODULOS="$PWD/shell/node_modules"
LANCADOR="${XDG_DATA_HOME:-$HOME/.local/share}/applications/hangar.desktop"

marca() { mkdir -p "$MARCA_DIR"; printf '%s\n' "$1" > "$MARCA_DIR/electron-removido"; }

if [ "$(uname -s)" != Linux ]; then
  marca "nao e Linux: nada removido"; exit 0
fi
if [ ! -x "$NATIVO" ]; then
  echo "electron: o app nativo não está instalado; o Electron fica"
  marca "sem app nativo: nada removido"; exit 0
fi
# App nativo de antes do motor Chromium ainda não atende o hangar-preview: o Electron fica nele.
if ! grep -aq -- '--remote-debugging-pipe' "$NATIVO"; then
  echo "electron: o app nativo instalado ainda não tem o navegador Chromium; o Electron fica"
  marca "app nativo sem o navegador Chromium: nada removido"; exit 0
fi
# Mesma busca do app (desktop-native/src/browser/chromium/launch.rs); o install-chromium.sh já rodou antes.
tem_chromium() {
  local nome bin real
  [ -n "${HANGAR_CHROMIUM:-}" ] && [ -x "$HANGAR_CHROMIUM" ] && return 0
  [ -x "$MARCA_DIR/chromium/chrome-headless-shell" ] && return 0
  for nome in google-chrome-stable google-chrome chromium chromium-browser; do
    bin=$(command -v "$nome" 2>/dev/null) || continue
    real=$(readlink -f "$bin" 2>/dev/null || echo "$bin")
    case "$real" in /snap/*|*/snap) continue ;; esac
    head -c 4096 "$real" 2>/dev/null | grep -aq '/snap/' && continue
    return 0
  done
  return 1
}
if ! tem_chromium; then
  echo "electron: sem Chrome ou Chromium para o navegador do app nativo; o Electron fica"
  marca "sem Chromium: nada removido"; exit 0
fi

# Pelo executável de cada processo, não pela linha de comando: o texto do caminho também aparece em shells.
electron_aberto() {
  local exe
  for exe in /proc/[0-9]*/exe; do
    case "$(readlink "$exe" 2>/dev/null)" in */shell/node_modules/electron/dist/electron) return 0 ;; esac
  done
  return 1
}

# Só o lançador que abre o Electron deste checkout; outro hangar.desktop não é nosso.
if [ -f "$LANCADOR" ] && grep -q '/shell/node_modules/electron/' "$LANCADOR"; then
  rm -f "$LANCADOR"
  if command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database "$(dirname "$LANCADOR")" 2>/dev/null || true; fi
  echo "electron: lançador removido: $LANCADOR"
fi

if [ ! -d "$MODULOS" ]; then
  marca "lancador removido; shell/node_modules ja nao existia"
elif electron_aberto; then
  echo "electron: o Hangar (Electron) está aberto; o lançador saiu, e o shell/node_modules fica até ser apagado com ele fechado"
  marca "lancador removido; shell/node_modules mantido porque o Electron estava aberto"
else
  rm -rf "$MODULOS"
  if [ -d "$MODULOS" ]; then
    echo "electron: não consegui apagar $MODULOS"
    marca "lancador removido; shell/node_modules nao saiu"
  else
    echo "electron: $MODULOS removido"
    marca "lancador e shell/node_modules removidos"
  fi
fi
exit 0
