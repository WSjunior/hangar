# Sombra da lista com sessões reais no backend isolado

Substitui o Step 34 (pedido `docs/migracao-rust/pedidos/2026-10-05-lista-estado-sombra-isolada.md`).
Rodada final em 05/10/2026 sobre a integração `5d3a74928` (merge `68447c244` na
`feat/session-list-state-sombra`), com as correções da revisão da junção. Roteiro:
`scripts/prova-lista-sombra.py` (reaproveita `prova-dono-unico.py`).

## Montagem

- Backend isolado (`HOME`, portas, token e `tmux -L` próprios), `matar_orfaos` e a poda
  desligados no lançador, `hangar-server` release desta branch com `CP_LIST_SHADOW=1`.
- Claude em `claude-haiku-4-5` na conta 02-200 (troca de conta para a `claude-200-3`), com e sem
  terminal; Codex em `gpt-6-luna` com e sem terminal, numa `CODEX_HOME` temporária com a cópia do
  login (sem renovação durante a prova: conferido no fim, nada copiado de volta).
- A lista do dono fica aberta o tempo todo (sem ela o Python não tem lista para comparar).
- O `claude` roda na conta real, e o hook dele grava marcador, registro nativo, pergunta e
  statusline nas pastas dela. O `HOME` isolado liga essas quatro pastas; sem isso Python e Rust
  caíam no pane e as rodadas 1 e 2 mediram um caminho que não é o de uso.
- Valores das diferenças: só num binário de diagnóstico (despejo local em arquivo da pasta da
  prova, fora do commit), apagado no fim. O diário e este relatório têm só sessão, campo e contagem.

## O que a prova achou e foi corrigido

| Achado | Correção |
|---|---|
| Sombra aceitava os campos do runtime de toda linha Claude sem terminal; sem retrato, o estado virava `idle` calado (revisão, item 2) | `4f33be2e4`: `Facts.headless: Option`; sem retrato, `problema = list_runtime_absent`; com retrato, sessão fora dele é parada, como `hl.snapshot() = None` |
| Diferença só ia ao diário após duas rodadas seguidas; rodada cega zerava (revisão, item 13) | `4f33be2e4`: contagem por (sessão, campo), janela de 60 s, `campo_N_of_M`, pânico descarrega a janela |
| **Nenhuma diferença chegava ao diário:** o `/internal/diag` recusava (400) o código `campo:N/M` | `c1e3004d4`: formato `[a-z0-9_]`, teste sobre todos os campos da assinatura |
| `last_reply` da linha sem retrato divergia junto com o estado (11 de 41 rodadas) | este commit: aceita só com o estado divergindo; com o mesmo estado, compara |

## Resultado por cenário (rodada final)

`campo N/M`: divergiu em N das M rodadas da janela de 60 s. Tempo por atualização: Python do
início do `_cached_list` ao `_list_sig` do refresher; Rust, o intervalo entre pedidos de fatos da
sombra menos o tique de 1,5 s (a produção inteira, com a ida e volta dos fatos). 3 a 6 sessões.

| Cenário | Diferenças | Causa | Python (mediana/máx) | Rust (mediana/máx) |
|---|---|---|---|---|
| nascer | c00 `conta`, `jsonl`, `model`, `question`, `state`, `status_line` 1/29; ct `status_line` 1/29 | transição (A); c00 é a sessão do diálogo de confiança | 15 / 1487 ms | 12 / 23 ms |
| trabalhando → parada | ct `state`, `context`, `status_line`, `last_reply(_at)` 1/41; ch `last_reply(_at)` 1/41 | transição (A) | 15 / 779 ms | 11 / 18 ms |
| cartão de permissão | pt `label` 14/41 e 15/41; pt `state` 1/41; ph `context`, `model` 1/41 | rótulo: cache de 20 s (B); resto (A) | 18 / 1387 ms | 13 / 21 ms |
| AskUserQuestion | ch `context` 10/41, `last_reply(_at)` 11/41 e 1/41; ct `question` 2/41, `state` 1–2/41, `last_reply(_at)` 1/41 | contexto (B); resposta de ch (C, corrigido); resto (A) | 20 / 852 ms | 14 / 20 ms |
| `/clear` | ct e ch `jsonl` 1/41, `context`, `model`, `state`, `status_line`, `last_reply(_at)` 1–2/41 | transição (A) | 18 / 1192 ms | 12 / 17 ms |
| matar a sessão | pt, xt `row_missing` 1/41 | (A): o Rust vê o pane sumir antes | 17 / 952 ms | 12 / 17 ms |
| fechar e recriar com o mesmo nome | ch `last_reply(_at)` 1/41; ct `state`, `status_line` 1/41 | (A) | 16 / 1260 ms | 12 / 19 ms |
| par e grupo | ch `last_reply(_at)` 2/41; ct `state`, `last_reply(_at)` 1/41 | (A): o protocolo do grupo põe as sessões para trabalhar | 14 / 854 ms | 11 / 18 ms |
| worktree com branch | wt `state`, `status_line` 1/41 | (A) | 18 / 1507 ms | 13 / 17 ms |
| plano com barra (passo marcado, pino) | nenhuma | — | 18 / 997 ms | 13 / 17 ms |
| troca de conta | ct `row_missing` 1/41 | (A): a sessão sai e volta na outra conta | 18 / 1343 ms | 13 / 16 ms |
| restart do backend com sessão viva | nenhuma | — | 18 / 912 ms | 12 / 24 ms |
| **total** | | | **16 / 1507 ms** (685) | **13 / 24 ms** (654) |

Nenhuma rodada cega (`rust.list_shadow_blind`) nem falha (`rust.list_shadow_failed`) e nenhum
envio recusado pelo diário na rodada final.

## Causas aceitas (registradas em `desenho.md`, "Sombra (Task 15)")

- **(A) Transição, 1 a 3 rodadas:** a lista do Python comparada tem até 3 s
  (`list_facts._SHADOW_MAX_AGE`) e a do Rust é da rodada; numa mudança de estado, criação, `/clear`
  ou fim de sessão um lado vê antes. Some na rodada seguinte; regra igual pelo golden.
- **(B) Cache de 20 s lido em instantes diferentes:** contexto (`claude_context`, 20 s por
  transcript) e rótulo da sessão `working` pelo marcador (captura da statusline com `STATUS_TTL`).
  Cada lado lê num momento: diverge até a próxima leitura (≤ 13 rodadas). Mesma regra
  (`contract_list::context_cases`, `state_sequences`).
- **(C) Sem retrato do runtime:** já aceito; agora a resposta só é aceita com o estado divergindo.

## Não provado aqui

- Cartão de permissão do Claude **com terminal** não apareceu (`pt` ficou `working` nos dois lados
  com o marcador preso depois do `Bash`); o sem terminal (`ph`) chegou a `awaiting_input`. Igual
  nos dois lados, então não é diferença da sombra; fica para a Task 24.
- Codex abre em modo YOLO: sem cartão de permissão para provocar.
- O estado de Claude sem terminal não é comparado na sombra (retrato do runtime só com o hub,
  Task 17); a linha mostra `list_runtime_absent`.

## Armadilhas do ambiente (já no roteiro)

Pastas de estado da conta ligadas no `HOME` (senão nenhum marcador); poda desligada (apagaria
sidecar real); `CP_SCAN_ROOTS` na pasta da prova (worktree fora dela dá 403); `codex` e `node` do
fnm no `PATH`; seletores do Codex na primeira TUI (confiança nos hooks, aviso de versão,
`check_for_update_on_startup = false`).
