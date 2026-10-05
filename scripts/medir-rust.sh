#!/usr/bin/env bash
# Compara o mesmo pedido pelo hangar-server (Rust, porta 8765) e direto no Python (porta interna),
# para cada parte já migrada. Só leitura: não muda nada no servidor. Parte que o Rust ainda não
# atende ele repassa ao Python, e aí os dois tempos ficam parecidos.
set -euo pipefail

pid=$(pgrep -f "python3 -m app.main" | head -1 || true)
[ -n "$pid" ] || { echo "backend do Hangar não está rodando"; exit 1; }
backend=$(readlink "/proc/$pid/cwd")
token=$(grep '^CP_AUTH_TOKEN=' "$backend/.env" | cut -d= -f2-)
log=~/.hangar/logs/privado/hangar-server.log

if ! curl -sf http://127.0.0.1:8765/__hangar_server/health >/dev/null; then
  echo "o Rust não está atendendo a porta 8765 (o Python está sozinho)"; exit 1
fi
python_port=$(grep -o 'upstream=127.0.0.1:[0-9]*' "$log" 2>/dev/null | tail -1 | cut -d= -f2 || true)
[ -n "$python_port" ] || { echo "porta do Python não encontrada em $log"; exit 1; }
RUST=127.0.0.1:8765
PY="$python_port"

pedido_ms() {  # $1 = host:porta, $2 = caminho
  curl -s -o /dev/null -w '%{http_code} %{time_total}\n' -H "Authorization: Bearer $token" "http://$1$2" |
    awk '$1 != 200 {print "erro HTTP " $1 > "/dev/stderr"; exit 1} {printf "%.0f", $2 * 1000}'
}

primeira_mensagem_ms() {  # $1 = host:porta, $2 = sessão; o ping inicial não conta
  local start end
  start=$(date +%s%N)
  end=$( { curl -sN --max-time 3 -H "Authorization: Bearer $token" "http://$1/api/sessions/$2/events" || true; } |
    { grep -m1 -q '^event: message' && date +%s%N; } ) || true
  [ -n "$end" ] || { echo "sem mensagem em 3 s" >&2; return 1; }
  echo $(( (end - start) / 1000000 ))
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
  printf '%-34s %-16s %6s ms %6s ms %7s\n' "$1" "$2" "$r" "$p" "$ganho"
}

sessoes=$(curl -s -H "Authorization: Bearer $token" "http://$RUST/api/sessions" |
  python3 -c 'import json,sys; [print(x["name"], x.get("provider")) for x in json.load(sys.stdin) if x.get("jsonl")]')
[ -n "$sessoes" ] || { echo "nenhuma sessão aberta com conversa"; exit 1; }

printf '%-34s %-16s %9s %9s %7s\n' medida tipo Rust Python ganho
echo "— Abrir o chat (últimas 200 mensagens, parte 1)"
while read -r nome tipo; do
  [ -n "$nome" ] || continue
  t=$(par pedido_ms "/api/sessions/$nome/history?limit=200") && linha "$nome" "$tipo" "$t"
done <<< "$sessoes"

echo "— Chat ao vivo: até a primeira mensagem (parte 1 e 2B)"
while read -r nome tipo; do
  [ -n "$nome" ] || continue
  t=$(par primeira_mensagem_ms "$nome") && linha "$nome" "$tipo" "$t"
done <<< "$sessoes"

# Com o Rust de pé o Python não roda Git/arquivos: a rota dele chama o núcleo Rust pela ponte,
# então a diferença aqui é o custo da ponte, não Python contra Rust.
echo "— Git e arquivos do painel da sessão (PR #30), na sessão $(head -1 <<< "$sessoes" | cut -d' ' -f1)"
nome=$(head -1 <<< "$sessoes" | cut -d' ' -f1)
for rota in "branches" "git/files" "git/log?n=50" "files/list?so_modificados=false" \
            "files/read?path=README.md" "files/search?q=import&mode=names"; do
  t=$(par pedido_ms "/api/sessions/$nome/$rota") && linha "${rota%%\?*}" git "$t" || echo "${rota%%\?*}: não respondeu 200, pulado"
done
t=$(par pedido_ms /api/fs/roots) && linha "fs/roots" arquivos "$t"

echo "— Lista de worktrees (Rust desde feat/worktrees-rust; o Python ainda tem a rota antiga)"
t=$(par pedido_ms /api/worktrees) && linha worktrees lista "$t" || echo "worktrees: não respondeu 200, pulado"

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
t=$(par pedido_ms "/api/costs?period=all&view=summary") && linha "custos: tela inicial" resumo "$t"
t=$(par pedido_ms "/api/costs?period=all") && linha "custos: tela Custos" inteiro "$t"
t=$(par pedido_ms /api/uso) && linha uso - "$t"
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
