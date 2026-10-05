# Lista de sessões + estado: medição (05/10/2026)

Pedido: `../pedidos/2026-10-05-lista-estado-kickoff.md`. Inventário: `inventario.md`. Proposta no
lugar desta parte: `plano.md`.

## Conclusão

**O ganho de levar a lista ao Rust é pequeno, e o "5% em repouso" não é da lista.**

- Com zero sessões e nenhum cliente, o Python gasta **4,2% de um núcleo**. **3,7 pontos** disso são
  o Supervisor do Rust: a cada 0,25 s, `refresh_members` (`backend/app/runtime_process.py:248`,
  chamado em `backend/app/rust_server.py:378-382`) percorre todos os processos da máquina com
  `psutil.process_iter` e regrava `runtime-process.json` com `fsync`. São 4 varreduras e 4
  `fsync` por segundo, enquanto o backend estiver de pé, em qualquer máquina.
- A lista (descoberta + decoração + stream) soma ao repouso **0,7 ponto com 1 sessão, 1,9 com 10 e
  2,5 com 20** (cresce ~0,1 ponto por sessão). É o teto do que a migração economizaria, e o Rust
  ainda pagaria os `tmux` e o `/proc`.
- O estado chega à lista em **0,1–1,6 s** (limitado pelo poll de 1,5 s), o `GET /api/sessions`
  responde em ~2 ms e o laço de eventos não atrasou mais de 10 ms em nenhuma janela. Não há
  problema de latência a resolver.

**Recomendação:** não planejar a lista agora. Fazer antes o conserto do Supervisor (`plano.md`,
2 Tasks pequenas): tira ~3,7–4,3 pontos fixos e ~345 mil `fsync` por dia, mais do que a lista
inteira renderia com 20 sessões. Depois, medir de novo o backend real; a lista volta à fila quando
o número de sessões subir (a 50 sessões a projeção é ~5–6 pontos) ou junto da parte 4 (estado e
terminal), com que divide captura e classificação.

## Números

CPU em % de um núcleo (`utime+stime` de `/proc/<pid>/stat` no início e no fim de cada janela de
30 s; "+" = filhos já colhidos pelo processo, isto é, os `tmux`/`git` que ele lançou). Rust = o
`hangar-server` filho; tmux = servidor do `tmux -L` da medição.

| Sessões | Cenário | Python | Rust | tmux |
|---:|---|---:|---:|---:|
| 0 | sem cliente | 4,2–4,3 | 0,0 | 0,0 |
| 1 | sem cliente da lista | 4,1* | 0,0 | 0,0 |
| 1 | 1 cliente da lista | 4,8 + 0,4 | 0,1 | 0,0 |
| 1 | 1 cliente, 1 trabalhando | 5,1 + 0,7 | 0,1 | 0,1 |
| 10 | sem cliente da lista | 4,7 + 0,1 | 0,3 | 0,0 |
| 10 | 1 cliente da lista | 6,1 + 0,5 | 0,4 | 0,0 |
| 10 | 1 cliente, 2 trabalhando | 7,5 + 0,8 | 1,2 | 0,2 |
| 20 | sem cliente da lista | 5,2–5,4 + 0,2 | 0,5 | 0,0 |
| 20 | 1 cliente da lista | 7,3–7,4 + 0,6–0,7 | 0,8 | 0,0–0,1 |
| 20 | 1 cliente, 5 trabalhando | 11,6–11,8 + 1,8 | 2,3–2,5 | 0,5–0,6 |

\* Uma janela de 1 sessão sem cliente deu 12,7% na rodada principal e 4,1% no ensaio; a lista
rodou 1 vez nela, então o pico veio de fora dela (os observadores de arquivo acompanham também a
pasta de registro nativo `~/.claude/sessions`, comum às sessões reais da máquina). Fica registrado,
não entra nas contas.

Linhas com intervalo = as duas rodadas que mediram o mesmo cenário (principal e atribuição).

### Uma rodada da lista (cliente aberto, média da janela)

| Sessões | `registry.list` parede / CPU | decoração `list_with_state` (parede) | mapa de `/proc` (CPU, a cada 3 s) | chamadas `tmux`/s | `capture-pane`/s |
|---:|---:|---:|---:|---:|---:|
| 1 | 8,9 / 5,1 ms | 1,3 ms | 7,3 ms | 1,4 | 0,07 |
| 10 | 18,4 / 13,4 ms | 7,5 ms | 7,4 ms | 2,1 | 0,23 |
| 20 | 27,5 / 21,1 ms | 14,0 ms | 7,3 ms | 2,8 | 0,47 |

A rodada sai a cada ~1,5 s (0,67/s). `registry.list` cresce ~0,8 ms de CPU por sessão (leitura de
`cmdline`/`environ`/fd por processo, resolução do transcript, worktree, conta) sobre ~4–5 ms fixos.

### Onde vai a CPU da lista, 20 sessões, cliente aberto (ms de CPU por segundo)

| Parte | ms/s | Fonte |
|---|---:|---|
| Descoberta (`registry.list`: `/proc`, panes, resolução por sessão) | 14,1 | temporizador na função |
| Subprocessos `tmux` (`list-panes`, `list-sessions`, `capture-pane`, `display`) | ~6,3 | CPU dos filhos colhidos; o lado Python deles é 0,6 |
| Transcript (contexto do fim do arquivo, última resposta, pergunta aberta, mtime) | ~1,5 | `_claude_reading`, `merged_history`, `pergunta_aberta`, `_jsonl_mtime` |
| Planos (`_decorate_plan`) | 0,35–0,4 | — |
| Git | 0,5 do Python | só a chamada: o `git status`/`diff` já roda no Rust (`git_ops.py:1325-1327` → `hangar-workspace`) |
| Classificação (`classify`, spinner, statusline, limite) | 0,05 | — |
| Assinatura e JSON da lista (`_list_sig`, `json.dumps`) e o stream | resto, ≤ ~1 | por diferença (o laço do uvicorn usa uvloop, os callbacks não foram atribuídos) |

Total com cliente − sem cliente (Python + filhos) = **2,4–2,5 pontos**.

### Atribuição do repouso (CPU por tarefa do pool de threads, ms/s)

| Tarefa | 0 sessões | 20, sem cliente | 20, cliente | 20, 5 trabalhando |
|---|---:|---:|---:|---:|
| `runtime_process.refresh_members` | 36,9 | 38,6 | 39,4 | 42,7 |
| `api._guardar_snap` (lista) | — | 0,5 | 14,2 | 14,0 |
| `runtime_policy.run` | — | — | — | 5,1 |
| `runtime_adapter.LegacyBridge.binding` (11 ms cada) | — | — | — | 4,8 |
| leitura do stream de eventos do runtime | 0,0 | 0,4 | 0,4 | 3,1 |
| `plugin_bridge._conversation_mismatch` | — | 2,7 | 0,4 | 0,7 |

Cada `refresh_members` custa 9,5–11 ms (≈ 560 processos na máquina); o custo acompanha o número de
processos da máquina, não o de sessões. Com sessões trabalhando, o Python ainda gasta ~4 pontos no
laço principal (SSE, eventos do runtime, hooks), fora do pool.

### Latência até o cliente da lista ver a mudança

| Sessões | Envio respondido → lista mostra `working` | Fim do transcript → lista mostra `idle` |
|---:|---|---|
| 1 | 0,44–0,79 s | 0,91–1,02 s |
| 10 | 1,04–1,07 s | 0,68–1,44 s |
| 20 | 0,07–1,22 s (10 medidas) | 0,35–1,56 s |

`GET /api/sessions` com a lista aberta (servido do retrato decorado, `sse.recent_list`): 1,6–2,7 ms
em 10 chamadas, igual com 1, 10 e 20 sessões. Atraso do laço de eventos (sonda de 50 ms): média
0,2–0,3 ms, nenhuma amostra acima de 10 ms em todas as janelas.

## Metodologia

- **Base:** `plan/session-list-state` em `920473435`; Python 3.14.6, tmux 3.7b, Claude Code
  2.1.289; `hangar-server` e `hangar-cano` em release (`cargo build --release`, 4 jobs). Máquina do
  dono, 16 threads, ~560 processos, outras sessões reais rodando (ruído de alguns décimos de ponto).
- **Isolamento:** a classe `Prova` de `scripts/prova-dono-unico.py` (HOME temporário, portas
  livres fora de 8765/8766/8768, token próprio, `tmux -L` próprio, unit transiente do systemd,
  `matar_orfaos` desligado). Sessões Claude só Haiku na conta `~/.claude-02-200`; metade com
  terminal, metade sem; cada uma aquecida com uma mensagem. Os marcadores dos hooks da conta real
  ficaram visíveis ao backend isolado (como na máquina de verdade), para as sessões com terminal não
  caírem todas na raspagem do pane.
- **Cenários:** sem cliente (só os laços de fundo, inclusive o `stall_watch` a cada 30 s), um
  cliente SSE de `/api/sessions/events` aberto, e o mesmo com 1 de cada 4 sessões recebendo um
  pedido longo (escrever 1 a 400 por extenso). Uma janela de 30 s por cenário.
- **Temporizadores** (fora do repositório, carregados pelo lançador): parede e CPU da thread em
  `registry.list`, `api._guardar_snap`, `tmux._run` (por subcomando), mapa de `/proc`,
  `classify`, leituras de transcript/sidecar, `git_summary`/`git_diffstat`, `_list_sig`; parede de
  `list_with_state` e `_cached_list`; sonda de atraso do laço. Na rodada de atribuição, CPU de cada
  tarefa do `ThreadPoolExecutor`. O `py-spy` não serviu: com `ptrace_scope=1` só amostra processo
  filho e o mata no fim.
- **Latência:** do retorno do `POST /input` ao primeiro `sessions` com `working` para aquela sessão;
  do mtime final do transcript ao primeiro `sessions` com `idle`, no relógio monotônico do cliente.
- **Custo da própria medição:** a sonda (20 acordadas/s) e o despejo dos contadores (2/s) estão
  dentro dos ~0,45 ponto que sobram no repouso fora do `refresh_members`.
- **Limpeza:** unit parada, `tmux -L` encerrado, processos da prova encerrados, HOME e pasta de
  transcrições da prova apagados ao fim de cada rodada.

## O que não foi medido

- Mais de 20 sessões (a projeção de 50 é linear a partir de 1/10/20), Windows (psmux custa ~50 ms
  por comando e a lista lá faz mais capturas), Pi/Kimi/omp/Codex, memória (RSS).
- Chat aberto: o custo do estado por sessão (`StateMonitor`) já está medido em
  `docs/decisoes/harnesses.md`, "Custo da observação terminal": 1,26 ms de CPU do Python por
  captura, ~1,3 captura/s por chat aberto (≈ 0,17 ponto por chat).
- O backend real: não foi tocado. O número de processos da máquina muda o custo do
  `refresh_members` lá.
