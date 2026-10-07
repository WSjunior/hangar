#!/usr/bin/env bash
# Dublê do bootstrap para provar as telas do assistente sem instalar nada:
#   HANGAR_SETUP_BOOTSTRAP=desktop-native/tools/setup-fake-bootstrap.sh hangar-native
# Imprime as marcas do contrato (spec "Marcas na saída") com pausas. Não escreve em pasta nenhuma.
# FAKE_SCENARIO: ok (padrão) | falha | tailscale | protocolo | askpass | interrompe. FAKE_PAUSE: segundos entre marcas (1).
set -u
cenario=${FAKE_SCENARIO:-ok}
pausa=${FAKE_PAUSE:-1}
m() { printf '%s\n' "$*"; sleep "$pausa"; }

check=0
fora=0
for a in "$@"; do
  case "$a" in
    --check|-SoChecar) check=1 ;;
    --tailscale=sim) fora=1 ;;
  esac
done

echo "dublê: argumentos $*"
# Nunca o valor: só se chegou.
echo "dublê: HANGAR_TOKEN $([ -n "${HANGAR_TOKEN:-}" ] && echo presente || echo ausente)"
m "##HANGAR-PROTOCOLO## $([ "$cenario" = protocolo ] && echo 99 || echo 1)"
m "##HANGAR-PASSO## preparar fazendo"
m "##HANGAR-ITEM## tmux ok tmux 3.5a · encontrado"
m "##HANGAR-ITEM## codex ok Codex 0.48.0 · encontrado"
m "##HANGAR-ITEM## claude fila Claude Code"
m "##HANGAR-ITEM## uv fila uv"
if [ $check = 1 ]; then
  m "##HANGAR-PASSO## preparar ok"
  m "##HANGAR-FIM## ok"
  exit 0
fi
m "##HANGAR-ITEM## claude fazendo Claude Code · baixando 31 de 48 MB"
echo '$ curl -fsSL https://claude.ai/install.sh | bash'
m "##HANGAR-ITEM## claude ok Claude Code · instalado"
if [ "$cenario" = askpass ]; then
  m "##HANGAR-ITEM## pacotes fazendo Pacotes do sistema"
  if "${HANGAR_ASKPASS:?HANGAR_ASKPASS ausente}" "instalar os pacotes do sistema" >/dev/null; then
    echo "dublê: senha recebida"
    m "##HANGAR-ITEM## pacotes ok Pacotes do sistema"
  else
    m "##HANGAR-ERRO## senha-cancelada"
    m "##HANGAR-FALHA## a senha de administrador não foi informada"
    m "##HANGAR-FIM## falhou"
    exit 1
  fi
fi
m "##HANGAR-ITEM## uv ok uv 0.9.2 · instalado"
m "##HANGAR-PASSO## preparar ok"
m "##HANGAR-PASSO## instalar fazendo"
m "##HANGAR-ITEM## servidor fazendo Baixar o servidor"
m "##HANGAR-ITEM## servidor ok Baixar o servidor"
m "##HANGAR-ITEM## inicio fazendo Iniciar com o computador"
if [ "$cenario" = falha ]; then
  m "##HANGAR-ERRO## sem-systemd"
  m "##HANGAR-FALHA## este Linux não tem como iniciar o Hangar sozinho"
  m "##HANGAR-FIM## falhou"
  exit 1
fi
if [ "$cenario" = interrompe ]; then echo "dublê: saindo sem FIM"; exit 1; fi
m "##HANGAR-ITEM## inicio ok Iniciar com o computador"
m "##HANGAR-PASSO## instalar ok"
pendencia=0
if [ $fora = 1 ]; then
  m "##HANGAR-PASSO## tailscale fazendo"
  m "##HANGAR-ITEM## tailscale ok Tailscale instalada"
  if [ "$cenario" = tailscale ]; then
    m "##HANGAR-LINK## tailscale-login https://login.tailscale.com/a/dubl3"
    m "##HANGAR-ITEM## tailscale-conta ok Conta conectada"
    m "##HANGAR-ITEM## tailscale-https pendente Endereço seguro (HTTPS)"
    m "##HANGAR-PENDENCIA## tailscale-https o HTTPS da conta Tailscale está desligado"
    m "##HANGAR-PASSO## tailscale pendente"
    pendencia=1
  else
    m "##HANGAR-ITEM## tailscale-conta ok Conta conectada"
    m "##HANGAR-ITEM## tailscale-https ok Endereço seguro (HTTPS)"
    m "##HANGAR-PASSO## tailscale ok"
  fi
fi
m "##HANGAR-PASSO## final fazendo"
m "##HANGAR-ITEM## servidor-responde ok Servidor respondendo"
m "##HANGAR-ITEM## agentes ok Agentes prontos"
m "##HANGAR-PASSO## final ok"
m "##HANGAR-FIM## $([ $pendencia = 1 ] && echo pendente || echo ok)"
