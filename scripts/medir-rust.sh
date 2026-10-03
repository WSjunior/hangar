#!/usr/bin/env bash
# Compara, nas sessões abertas, o tempo de abrir o chat pelo hangar-server (Rust, porta 8765) e
# pelo Python (porta interna). Só leitura: não muda nada no servidor.
set -euo pipefail

pid=$(pgrep -f "python3 -m app.main" | head -1 || true)
[ -n "$pid" ] || { echo "backend do Hangar não está rodando"; exit 1; }
backend=$(readlink "/proc/$pid/cwd")
token=$(grep '^CP_AUTH_TOKEN=' "$backend/.env" | cut -d= -f2-)
log=~/.hangar/logs/privado/hangar-server.log

if ! curl -sf http://127.0.0.1:8765/__hangar_server/health >/dev/null; then
  echo "o Rust não está atendendo a porta 8765 (o Python está sozinho)"; exit 1
fi
python_port=$(grep -o 'upstream=127.0.0.1:[0-9]*' "$log" | tail -1 | cut -d= -f2)

media_ms() {  # $1 = host:porta, $2 = sessão
  for _ in 1 2 3 4 5 6; do
    curl -s -o /dev/null -w '%{time_total}\n' -H "Authorization: Bearer $token" \
      "http://$1/api/sessions/$2/history?limit=200"
  done | tail -5 | awk '{s+=$1} END {printf "%.0f", s/NR*1000}'
}

sessoes=$(curl -s -H "Authorization: Bearer $token" http://127.0.0.1:8765/api/sessions |
  python3 -c 'import json,sys; [print(x["name"], x.get("provider")) for x in json.load(sys.stdin) if x.get("jsonl")]')

printf '%-28s %-16s %9s %9s %7s\n' sessão tipo Rust Python ganho
while read -r nome tipo; do
  [ -n "$nome" ] || continue
  r=$(media_ms 127.0.0.1:8765 "$nome")
  p=$(media_ms "$python_port" "$nome")
  ganho=$(awk -v r="$r" -v p="$p" 'BEGIN { if (r > 0) printf "%.1fx", p / r; else print "-" }')
  printf '%-28s %-16s %6s ms %6s ms %7s\n' "$nome" "$tipo" "$r" "$p" "$ganho"
done <<< "$sessoes"
echo
echo "Média de 5 aberturas (últimas 200 mensagens) depois de 1 de aquecimento. Menos é melhor."
echo "Pi, Kimi e omp ainda passam pelo Python: nelas os dois tempos ficam parecidos."
