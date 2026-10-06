# Parte 4: medições

Release, máquina do dono (i5-13400F, CachyOS), sem backend nem sessão real.

## Task 2: uma rodada do `Monitor` (captura + `reduce`), 06/10/2026

| Medida | Por rodada |
|---|---:|
| CPU do processo Rust (captura pelo `TerminalPool` + análise + `reduce`) | 0,077–0,080 ms |
| CPU do servidor tmux | 0,107–0,113 ms |
| CPU do cliente `-C` vivo | abaixo de 1 tick em 3000 rodadas (< 0,003 ms) |
| **Total** | **0,183–0,193 ms** |
| Parede | 0,182–0,193 ms |
| Só `analyze` + `reduce` (sem captura) | 0,041 ms |
| Pico de memória do processo de teste | 6,7 MB |

Hoje, com o chat aberto, o Python gasta **1,261 ms** de CPU por captura, mais 0,200 ms do Rust
(rota privada) e 0,207 ms do tmux (`harnesses.md`, "Custo da observação terminal"): 1,668 ms por
rodada no total. A rodada do `Monitor` custa 0,19 ms com o tmux incluído, sem HTTP e sem o
Python.

Método: `crates/hangar-server/tests/state_monitor_round.rs` (`--ignored`, à mão), tmux isolado
`-L` com `-f /dev/null`, pane 100×40 com 180 linhas e spinner parado (o mesmo pane sintético da
medida do Python), captura `-S -200` pelo `TerminalPool` (cliente `-C`, sem processo novo por
rodada). 20 rodadas de aquecimento, 4 lotes de 3000 rodadas seguidas, sem espera entre elas. CPU
= delta de `utime+stime` em `/proc/<pid>/stat` (ticks de 10 ms) do processo de teste, do servidor
tmux e do cliente de controle, dividido pelas rodadas. Fatos do Python e arquivos da sessão
ficaram de fora (a Task 2 não liga o `Monitor` a eles); entram na medida da Task 5.

## Terminal real (Task 8, 06/10/2026)

**O PTY do dono no Rust tira o Python do caminho dos bytes e corta a CPU por MB pela metade;
vazão e eco não mudam, porque quem limita é o servidor tmux.**

Como: `scripts/medir-terminal.py`, backend isolado (classe `Prova`: HOME, portas, token e
`tmux -L` próprios), `hangar-server` em release, nenhum outro cliente. Uma sessão `bash` de
200×50 no tmux da prova, painel aberto pelo WebSocket do dono. Vazão = `cat` de um arquivo de
50 MB (linhas de 100 caracteres), do Enter até o marcador do fim chegar ao cliente. Eco = 50
teclas, do envio ao primeiro quadro binário com a tecla de volta. CPU = `utime+stime` de
`/proc/<pid>/stat` durante o `cat`, dividida pelos 50 MB (o cliente `tmux attach` de cada painel
fica fora da conta, nos dois lados). Memória = RSS e threads antes e depois de abrir 10 painéis,
dividido por 10. "Antes" = o código da base `82017cb7d` (o Rust repassa o WebSocket ao Python, que
abre o PTY); "reserva" = a mesma base com `CP_RUST_SERVER=0`; "depois" = o código desta Task.
Amostras separadas por `/`; a última de cada coluna é do script com o eco filtrado por quadro
binário.

| Medida | Antes (Python atrás do Rust) | Reserva (Python sozinho) | Depois (Rust) |
|---|---:|---:|---:|
| Vazão, MB/s | 61,3 / 55,2 / 59,6 | 61,4 / 61,7 | 60,6 / 61,1 / 60,0 |
| Eco, mediana ms | 0,56 / 0,62 / 0,56 | 0,51 / 0,59 | 0,55 / 0,52 / 0,58 |
| Eco, p95 ms | 0,79 / 0,85 / 0,82 | 0,75 / 3,55 | 0,93 / 0,75 / 0,86 |
| CPU por MB, Python | 7,8 / 12,6 / 7,6 ms | 6,8 / 6,4 ms | 0,0 / 0,0 / 0,4 ms |
| CPU por MB, Rust | 3,0 / 3,4 / 3,0 ms (repasse) | — | 4,6 / 5,0 / 5,2 ms |
| CPU por MB, Python + Rust | 10,8 / 16,0 / 10,6 ms | 6,8 / 6,4 ms | 4,6 / 5,0 / 5,6 ms |
| CPU por MB, servidor tmux | 16,0 / 17,8 / 16,4 ms | 16,2 / 16,0 ms | 16,2 / 16,4 / 16,6 ms |
| Por painel, Rust | ~0 MB, 0 thread | — | ~0,25 MB, +2 threads |

- O tmux gasta ~16 ms por MB em qualquer cenário (ele redesenha a tela): é ele que segura a vazão
  em ~60 MB/s, então trocar o dono do PTY não mexe nela nem no eco.
- Python + Rust por MB cai de 10,6–16,0 ms para 4,6–5,6 ms; contra o Python sozinho (6,4–6,8 ms)
  o Rust gasta ~20% menos e livra o laço de eventos do Python dos bytes do terminal.
- Cada painel no Rust custa 2 threads (leitor e escritor do PTY, teto de 64 painéis) e ~0,25 MB.
  A memória do Python por painel não se mede assim: o RSS dele oscila mais que o painel (de −1,09
  a +0,05 MB nas amostras).

Não medido aqui: celular e nativo de verdade (Task 12), Windows (Task 10), Origin (a medida
conecta sem `Origin`).

## Task 5: estado ao vivo no `Monitor` do Rust, 06/10/2026

**Com o `Monitor` dono do estado, o Python sai do custo por chat aberto: com 20 chats trabalhando
ele cai de 176,5 para 29 ms de CPU por segundo, e Python + Rust + tmux de 297,5 para 146.** O Rust
fica parecido (a captura já era dele; agora a análise e a memória também), e a latência
marcador → `state` fica igual à do Python.

Como: `scripts/medir-estado.py`, backend isolado (classe `Prova`), binários release. "Antes" =
`git archive` de `4a46110b2` (a base desta Task, com o roteiro copiado para dentro); "depois" = esta
Task. 20 sessões `claude --session-id <sid>` de mentira (um Python que desenha a tela do Claude a
80×24), sem Claude real. Chats = `/api/sessions/<nome>/events` do dono abertos por threads. Modo
`parado` = tela sem spinner; `trabalhando` = spinner girando e resposta crescendo a cada 0,1 s.
CPU = `utime+stime` (com filhos) de `/proc/<pid>/stat` em 20 s, depois de 8 s assentando, em ms por
segundo. Latência = escrita do marcador `.hangar-state/<sid>.json` até o `state` novo chegar ao
chat, com o pane de spinner parado (o marcador decide), 10 amostras.

| Chats | Modo | Python | Rust | tmux | Total |
|---:|---|---:|---:|---:|---:|
| 1 | parado | 12,5 → 11,0 | 5,0 → 5,5 | 3,5 → 4,0 | 21,0 → 20,5 |
| 1 | trabalhando | 19,5 → 9,5 | 8,5 → 7,0 | 6,0 → 7,0 | 34,0 → 23,5 |
| 5 | parado | 34,5 → 14,0 | 10,5 → 9,5 | 7,0 → 6,5 | 52,0 → 30,0 |
| 5 | trabalhando | 55,5 → 14,0 | 19,0 → 18,5 | 18,5 → 19,0 | 93,0 → 51,5 |
| 20 | parado | 96,5 → 29,0 | 25,5 → 27,5 | 14,5 → 16,0 | 136,5 → 72,5 |
| 20 | trabalhando | 176,5 → 29,0 | 60,0 → 55,5 | 61,0 → 61,5 | 297,5 → 146,0 |

| Medida | Antes | Depois |
|---|---:|---:|
| `state` por s, 20 trabalhando | 20,65 | 20,2 |
| `preview` por s, 20 trabalhando | 125,6 | 133,0 |
| Latência marcador → `state`, 1 chat (mediana / máx.) | 0,459 / 0,678 s | 0,457 / 0,708 s |
| Latência marcador → `state`, 20 chats (mediana / máx.) | 0,472 / 0,841 s | 0,446 / 0,491 s |
| RSS com 20 chats, Python / Rust | 151,2 / 32,5 MB | 144,5 / 40,2 MB |

- O que sobra no Python com 20 chats (29 ms/s) é o resto da conexão interna de cada sessão
  (estatísticas a 1 s, fila, faixa, avisos), o mesmo parado e trabalhando.
- Os marcadores e o registro nativo vêm de uma cópia compartilhada pelos `Monitor`s que só é relida
  quando o observador das pastas de estado vê uma escrita. A primeira versão relia por prazo
  (250 ms): com 20 chats, metade das amostras de latência foi a 0,84–0,87 s (a rodada lia a cópia
  velha e esperava a seguinte).
- Não medido aqui: Windows (psmux avulso por rodada), convidado de verdade e celular.

## Task 6: lista lê o `Monitor`, 06/10/2026

**Com 5 chats abertos de 20 sessões, a lista deixa de capturar as 5 que têm `Monitor`: 13,0 →
9,75 capturas por segundo paradas e 23,5 → 18 trabalhando, e o Rust cai de 39 para 30,5 ms de CPU
por segundo parado e de 75,8 para 64 trabalhando.** Python e tmux ficam no ruído. A queda das
capturas (5 de 20 linhas, 25%) também prova, com vínculo real, que o session id do hub casa com o do
transcript da linha.

Como: `scripts/medir-lista-monitor.py` (montagem de `medir-estado.py`), backend isolado, binários
release. "Antes" = `git archive` de `f99826e47` (base desta Task, com o backend dela e o roteiro
copiado para dentro); "depois" = esta Task. 20 sessões de mentira **sem marcador do hook** (o caso
em que a lista cai no pane a cada tique), lista do dono aberta (`/api/sessions/events`) e 5 chats.
Capturas = `capture-pane` contados pelo `tmux` do PATH (a captura do `Monitor` é o cliente `-C` e
fica de fora nos dois lados). CPU como na Task 5, janela de 20 s depois de 8 s assentando. Duas
rodadas de cada lado; a tabela traz a média.

| Modo | Capturas/s | Python | Rust | tmux |
|---|---:|---:|---:|---:|
| parado | 13,0 → 9,75 | 20,3 → 19,5 | 39,0 → 30,5 | 8,3 → 6,8 |
| trabalhando | 23,5 → 18,0 | 21,3 → 20,3 | 75,8 → 64,0 | 46,3 → 44,5 |

- Rodadas (Rust, trabalhando): antes 70,0 e 81,5; depois 67,5 e 60,5. RSS sem mudança (Python
  ~140 MB, Rust ~34 MB).
- Com marcador do hook nas sessões a lista quase não captura (só a statusline, 2 por tique, e o
  radar de limite); a economia aqui é o teto, não o caso comum.

## Eco de tecla no Windows (VM DELPHI-02, 06/10/2026)

Tecla mandada pelo WebSocket do terminal real até o eco voltar, 40 teclas por rodada, sessão
psmux com PowerShell, backend da VM em `641838a3` (binário compilado na VM).

| Caminho | Mediana | Máximo |
|---|---|---|
| Rust, dentro da VM | 20,0–20,3 ms | 27,8 ms |
| Rust, daqui pela rede (Tailscale, ping 2 ms) | 22,8–22,9 ms | 39,9 ms |
| Python (`CP_RUST_SERVER=0`), dentro da VM | 20,4–20,5 ms | 25,3 ms |
| ConPTY puro com `cmd.exe`, sem psmux | 0,1 ms | 0,4 ms |
| ConPTY com `tmux attach` do psmux, sem o Hangar | 19,1 ms | 24,8 ms |

Os ~20 ms são do psmux (o cliente dele faz tecla → TCP → servidor → ConPTY interno → parse →
JSON → TCP → render), não do Hangar: Rust e Python dão o mesmo, e o console do Windows sozinho
custa 0,1 ms. No Linux o mesmo eco com tmux é ~0,55 ms (Task 8).

Saída grande (PowerShell imprimindo 3000 linhas de ~90 caracteres) até a marca do fim chegar,
mesma VM e sessão psmux: `tmux attach` direto num ConPTY sem o Hangar 0,92–0,94 s; painel do
Hangar (Rust) dentro da VM 0,91–0,94 s; painel daqui pela rede 0,92–0,93 s. O psmux manda só o
que mudou na tela (~15 KB no total), e o caminho do servidor não soma tempo.
