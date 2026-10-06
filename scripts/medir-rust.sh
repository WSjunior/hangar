#!/usr/bin/env bash
# Compara o mesmo pedido pelo hangar-server (Rust, porta 8765) e direto no Python (porta interna),
# para cada parte já migrada. Só leitura: não muda nada no servidor. Parte que o Rust ainda não
# atende ele repassa ao Python, e aí os dois tempos ficam parecidos.
set -euo pipefail

porta=${HANGAR_PORT:-8765}
RUST=127.0.0.1:$porta

# Token: HANGAR_TOKEN, o .env deste checkout, ou o do processo que escuta a porta. Achar o backend
# pelo nome do processo falha quando o venv chama o executável de `python`.
token_do_processo() {
  local pid p t
  pid=$(ss -Hltnp "sport = :$porta" 2>/dev/null | grep -o 'pid=[0-9]*' | head -1 | cut -d= -f2 || true)
  [ -n "$pid" ] || return 0
  # Com o Rust na porta, o Python é o pai dele; os dois recebem o token no ambiente.
  for p in "$pid" "$(ps -o ppid= -p "$pid" | tr -d ' ')"; do
    t=$(tr '\0' '\n' < "/proc/$p/environ" 2>/dev/null | grep '^CP_AUTH_TOKEN=' | cut -d= -f2- || true)
    [ -n "$t" ] || t=$(grep -s '^CP_AUTH_TOKEN=' "$(readlink "/proc/$p/cwd")/.env" | tail -1 | cut -d= -f2- || true)
    [ -n "$t" ] && { echo "$t"; return 0; }
  done
  return 0
}
estado() {  # resposta + código HTTP na última linha; sai se não houver conexão
  local rc=0 r
  r=$(curl -s -m 10 -w '\n%{http_code}' -H "Authorization: Bearer $1" "http://$RUST/api/migration/status") || rc=$?
  case $rc in
    0) printf '%s' "$r" ;;
    28) echo "o backend em $RUST aceitou a conexão mas não respondeu em 10 s"; exit 1 ;;
    *) echo "backend do Hangar fora: nada responde em $RUST (curl $rc)"; exit 1 ;;
  esac
}
token=${HANGAR_TOKEN:-}
[ -n "$token" ] || token=$(grep -s '^CP_AUTH_TOKEN=' "$(dirname "$0")/../backend/.env" | tail -1 | cut -d= -f2- || true)
[ -n "$token" ] || token=$(token_do_processo)
if [ -z "$token" ]; then
  ss -Hltn "sport = :$porta" 2>/dev/null | grep -q . || { echo "backend do Hangar fora: nada escuta em $RUST"; exit 1; }
  echo "token não encontrado: defina HANGAR_TOKEN ou rode o script do checkout do backend"; exit 1
fi

resposta=$(estado "$token") || { echo "$resposta"; exit 1; }
codigo=${resposta##*$'\n'}
if [ "$codigo" = 401 ] && [ -z "${HANGAR_TOKEN:-}" ]; then
  # O .env deste checkout pode ser de outro backend (o app roda de outro checkout): vale o do processo.
  outro=$(token_do_processo)
  if [ -n "$outro" ] && [ "$outro" != "$token" ]; then
    token=$outro
    resposta=$(estado "$token") || { echo "$resposta"; exit 1; }
    codigo=${resposta##*$'\n'}
  fi
fi
case $codigo in
  200) ;;
  401|403) echo "o backend recusou o token (HTTP $codigo): confira o CP_AUTH_TOKEN"; exit 1 ;;
  404) echo "o backend não tem /api/migration/status (versão anterior à tela de migração): atualize"; exit 1 ;;
  *) echo "o estado da migração respondeu HTTP $codigo"; exit 1 ;;
esac
leitura=$(python3 -c '
import json, sys
s = json.load(sys.stdin); p = s.get("python") or {}
print(s.get("served_by") or "-", p.get("mode") or "-", p.get("reason") or "-", p.get("port") or "-",
      s.get("python_error") or "-", (p.get("binary") or {}).get("path") or "-")' <<< "${resposta%$'\n'*}") ||
  { echo "resposta do estado da migração ilegível"; exit 1; }
read -r atende modo motivo python_port erro_python binario <<< "$leitura"
if [ "$atende" != rust ]; then
  case $motivo in
    sem_binario) motivo="binário hangar-server ausente (CP_RUST_SERVER_BIN, crates/target/release ou ~/.hangar/bin)" ;;
    desligado) motivo="CP_RUST_SERVER=0 no backend/.env" ;;
    reload) motivo="backend rodando com --reload" ;;
    protocolo) motivo="o binário fala outro contrato interno (atualize os binários)" ;;
    sem_resposta) motivo="o binário não respondeu em 10 s" ;;
    endereco_privado) motivo="o binário não anunciou o endereço privado" ;;
    quedas) motivo="o Rust caiu 3 vezes em 60 s" ;;
    erro) motivo="a vigia do Rust falhou (veja o diário)" ;;
  esac
  echo "o Rust não está atendendo a porta $porta: o Python está sozinho (modo $modo) — $motivo"; exit 1
fi
[ "$python_port" != - ] || { echo "o Rust respondeu, mas o Python atrás dele não mandou os dados (erro: $erro_python)"; exit 1; }
echo "Rust na porta $porta (modo $modo, binário $binario); Python em 127.0.0.1:$python_port"
PY="127.0.0.1:$python_port"

# Tempos internos em microssegundos: rota que responde abaixo de 1 ms não pode virar 0.
pedido_us() {  # $1 = host:porta, $2 = caminho
  curl -s -o /dev/null -w '%{http_code} %{time_total}\n' -H "Authorization: Bearer $token" "http://$1$2" |
    awk '$1 != 200 {print "erro HTTP " $1 > "/dev/stderr"; exit 1} {printf "%.0f", $2 * 1000000}'
}

primeira_mensagem_us() {  # $1 = host:porta, $2 = sessão; o ping inicial não conta
  local start end
  start=$(date +%s%N)
  end=$( { curl -sN --max-time 3 -H "Authorization: Bearer $token" "http://$1/api/sessions/$2/events" || true; } |
    { grep -m1 -q '^event: message' && date +%s%N; } ) || true
  [ -n "$end" ] || { echo "sem mensagem em 3 s" >&2; return 1; }
  echo $(( (end - start) / 1000 ))
}

par() {  # $1 = função, $2.. = argumentos; Rust e Python alternados, 1 aquecimento + 5 medidas
  local fn=$1; shift
  local r=0 p=0 i a b
  for i in 0 1 2 3 4 5; do
    a=$($fn "$RUST" "$@") || return 1
    b=$($fn "$PY" "$@") || return 1
    [ "$i" -gt 0 ] && { r=$((r + a)); p=$((p + b)); }
  done
  echo "$((r / 5)) $((p / 5))"
}

linha() {  # $1 = rótulo, $2 = tipo, $3 = "Rust Python"
  read -r r p <<< "$3"
  local ganho
  ganho=$(awk -v r="$r" -v p="$p" 'BEGIN { if (r > 0) printf "%.1fx", p / r; else print "-" }')
  # O awk formata: o printf do bash segue o locale e recusa "1.5" em pt_BR.
  printf '%-34s %-16s %11s %11s %7s\n' "$1" "$2" "$(awk -v v="$r" 'BEGIN{printf "%.1f ms", v/1000}')" \
    "$(awk -v v="$p" 'BEGIN{printf "%.1f ms", v/1000}')" "$ganho"
}

sessoes=$(curl -s -H "Authorization: Bearer $token" "http://$RUST/api/sessions" |
  python3 -c 'import json,sys; [print(x["name"], x.get("provider")) for x in json.load(sys.stdin) if x.get("jsonl")]')
[ -n "$sessoes" ] || { echo "nenhuma sessão aberta com conversa"; exit 1; }

printf '%-34s %-16s %11s %11s %7s\n' medida tipo Rust Python ganho
echo "— Abrir o chat (últimas 200 mensagens, parte 1)"
while read -r nome tipo; do
  [ -n "$nome" ] || continue
  t=$(par pedido_us "/api/sessions/$nome/history?limit=200") && linha "$nome" "$tipo" "$t"
done <<< "$sessoes"

echo "— Chat ao vivo: até a primeira mensagem (parte 1 e 2B)"
while read -r nome tipo; do
  [ -n "$nome" ] || continue
  t=$(par primeira_mensagem_us "$nome") && linha "$nome" "$tipo" "$t"
done <<< "$sessoes"

# Com o Rust de pé o Python não roda Git/arquivos: a rota dele chama o núcleo Rust pela ponte,
# então a diferença aqui é o custo da ponte, não Python contra Rust.
echo "— Git e arquivos do painel da sessão (PR #30), na sessão $(head -1 <<< "$sessoes" | cut -d' ' -f1)"
nome=$(head -1 <<< "$sessoes" | cut -d' ' -f1)
for rota in "branches" "git/files" "git/log?n=50" "files/list?so_modificados=false" \
            "files/read?path=README.md" "files/search?q=import&mode=names"; do
  t=$(par pedido_us "/api/sessions/$nome/$rota") && linha "${rota%%\?*}" git "$t" || echo "${rota%%\?*}: não respondeu 200, pulado"
done
t=$(par pedido_us /api/fs/roots) && linha "fs/roots" arquivos "$t"

echo "— Lista de worktrees (Rust desde feat/worktrees-rust; o Python ainda tem a rota antiga)"
t=$(par pedido_us /api/worktrees) && linha worktrees lista "$t" || echo "worktrees: não respondeu 200, pulado"

# Rust serve a lista publicada pelo ListHub; o Python direto ainda varre /proc e tmux e classifica
# cada pane, como fazia antes da troca. Com a lista do app aberta o Python reaproveita o snapshot do
# refresher dele, que no modo rust não roda: a coluna Python é a varredura inteira.
echo "— Lista de sessões do dono (Rust desde feat/session-list-state)"
t=$(par pedido_us /api/sessions) && linha "lista do app" lista "$t" || echo "lista do app: não respondeu 200, pulado"
echo "  (pedir a lista custa ~1 ms nos dois: ambos servem uma lista já montada. O ganho é montar e"
echo "   avisar: mudança de estado chega ao app em ~0,16 s no Rust contra 1,4–2,3 s no Python, e o pior"
echo "   caso de uma rodada cai de 1,5 s para 24 ms — medido por scripts/medir-lista-hub.py)"

# A tela inicial pede ?view=summary; o Python ignora o parâmetro e manda o relatório inteiro, que
# era o que a tela recebia antes. Por isso a coluna Python desta linha é o "antes".
echo "— Custos e Uso (parte 3)"
aquecer() {  # $1 = caminho; o lado que não é dono responde 202 até montar o próprio índice
  local alvo h c
  for h in "$RUST" "$PY"; do
    for _ in $(seq 1 120); do
      c=$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $token" "http://$h$1")
      [ "$c" = 200 ] && break
      sleep 1
    done
    [ "$c" = 200 ] || echo "$1 em $h ainda respondia $c depois de 120 s" >&2
  done
}
aquecer "/api/costs?period=all"; aquecer /api/uso
t=$(par pedido_us "/api/costs?period=all&view=summary") && linha "custos: tela inicial" resumo "$t"
t=$(par pedido_us "/api/costs?period=all") && linha "custos: tela Custos" inteiro "$t"
t=$(par pedido_us /api/uso) && linha uso - "$t"
for alvo in "/api/costs?period=all&view=summary" "/api/costs?period=all"; do
  r=$(curl -s -H "Authorization: Bearer $token" -H 'Accept-Encoding: gzip' -o /dev/null -w '%{size_download}' "http://$RUST$alvo")
  p=$(curl -s -H "Authorization: Bearer $token" -H 'Accept-Encoding: gzip' -o /dev/null -w '%{size_download}' "http://$PY$alvo")
  printf '%-34s %-16s %6s KB %6s KB  (baixado, comprimido)\n' "${alvo#/api/}" tamanho "$((r / 1024))" "$((p / 1024))"
done

echo
echo "Média de 5 medidas depois de 1 de aquecimento, Rust e Python alternados. Menos é melhor."
echo "Ganho perto de 1x = o Rust ainda repassa essa parte ao Python."
echo "Fora da medição: envio de mensagem, fila e controle das sessões (2B, 2C, 2D, dono único), as"
echo "ações de Git e de worktree que escrevem e a reconstrução do índice de custos — medir exigiria"
echo "mandar mensagem, mudar o repositório ou apagar o índice de verdade."
