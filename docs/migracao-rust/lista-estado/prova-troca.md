# Prova da troca: a lista do dono servida pelo `ListHub`, com sessões reais

Pedido `docs/migracao-rust/pedidos/2026-10-05-lista-estado-prova-troca.md`. Rodadas em 05/10/2026
sobre a integração `072002d1f` (Tasks 1–18), binários release desta branch. Roteiro:
`scripts/prova-lista-troca.py` (base `prova-lista-sombra.py`); medidas: `scripts/medir-lista-hub.py`.

## Montagem

- Backend isolado: `HOME`, portas, token e `tmux -L` próprios; `matar_orfaos` e a poda desligados no
  lançador; **sem** `CP_LIST_SHADOW`. Claude `claude-haiku-4-5` na conta 02-200 (troca de conta para
  a `~/.claude-jefferson`), com e sem terminal; Codex `gpt-6-luna` com e sem terminal.
- Lista do dono aberta o tempo todo no SSE `/api/sessions/events`; cada cenário confere o esperado
  (escrito no roteiro, `ESPERADO`) no `GET /api/sessions` **e** no último `sessions` do SSE; o que só
  aparece de passagem (`working`, cartão, pergunta) é conferido no histórico do SSE do cenário.
- **Contador `registry.PYTHON_DISCOVERY`:** o lançador grava num arquivo da pasta da prova, a cada
  0,5 s, o contador e o modo do dono do runtime (`runtime_coordinator.current().mode`); o roteiro lê
  o arquivo ao fim de cada cenário.
- **Marcador → lista:** um laço de 10 ms vê o `mtime` do marcador do hook (`.hangar-state/<sid>.json`)
  de cada sessão Claude com terminal mudar e procura a primeira publicação do SSE com o mesmo estado.

## Resultado (modo `rust`)

Rodada 2 (completa) com o esperado corrigido; `conta` da rodada 3. Todas as sessões sem terminal
mostraram o estado do runtime: nenhuma publicação com `list_runtime_absent` em nenhuma rodada.

| Cenário | Esperado | Visto | Marcador → lista |
|---|---|---|---|
| nascer | ct, ch, xt, xh `idle` com provider e `headless` certos; ct/ch com `jsonl` | sim | 652 / 1148 ms (sessão nova espera o tique, regra da Task 18) |
| trabalhando → parada | as quatro vistas `working` no SSE e `idle` no fim | sim | 140 / 154 ms |
| cartão de permissão | ph `awaiting_input` visto (4 de 10 publicações) | sim | 383 / 612 ms (pt, sessão nova) |
| AskUserQuestion | ct e ch `awaiting_input` com `question`; `idle` depois | sim | 133 ms |
| `/clear` | ct e ch `idle` com `jsonl` novo | ch sim; **ct não** — o `/clear` não chegou ao agente (abaixo) | — |
| matar | pt e xt fora da lista; ph não `working` | sim (ph `idle`: o runtime Rust o mantém) | — |
| recriar com o mesmo nome | ch, xt, ct `idle`; pt e ph fora | sim | 114 / 153 ms |
| par e grupo | ct com `pair_peers` ⊇ {xt, ch, xh}, mesmo `pair_gid` nos quatro, `pair_task` | sim | 136 / 154 ms |
| worktree com branch | wt `idle`, `branch = prova-wt`, `worktree` | sim | 139 / 153 ms |
| plano com pino | ct `plan_name = prova-lista`, barra 1/3 | sim | — |
| troca de conta | ct `idle` na conta B | sim (assentou em 4,7 s) | 139 / 154 ms |
| convidado | ver "Convidado" | sim | — |
| restart com sessão viva | vivas do dono na lista depois | sim (restart 0,18 s, sem SIGKILL) | — |

**Zero descoberta Python:** o contador ficou `{}` ao fim de todos os 13 cenários nas duas rodadas
completas (e nas duas de `nascer,conta`), com o modo `rust`. Nenhum `list_error` no SSE (77 `sessions`, 43 `ping`, 5 `shortcut_terminals`).

## Reserva

- **`CP_RUST_SERVER=0`** (nascer, trabalhando, pergunta, `/clear`, matar): lista pelo Python, modo
  `python`, contador subindo (`list` 186, `list_with_state` 173, `refresher` 162, `resolve_tracked`
  334 no fim). Mesmos resultados, inclusive o `/clear` do ct não chegando. Única diferença: matar o
  cano de ph tira ph da lista no Python; no modo `rust` o runtime o mantém `idle`. É quem é dono das
  sessões sem terminal, não a lista. Marcador → lista no Python: 877 / 4671 ms.
- **Rust derrubado 3 vezes (SIGKILL, 3 quedas em 2 s):** o Python assumiu a porta
  (`o Python assume a porta` no log), `GET` 200 com as vivas do dono, o SSE seguiu publicando, modo
  `python` e a descoberta Python passou a contar.

## Convidado

Convidado com `owner_sees=false`, `sees_owner=false` cria `gh` sem terminal. A lista dele (`GET` e
SSE) traz só `gh`; o filtro `guest_users.filter_visible` do Python rodou 2 vezes com ele (contado no
lançador), então a lista do convidado passou pelo Python. O dono não vê `gh` no `GET` nem no SSE.

## Tempo e custo (release, 20 sessões, `medir-lista-hub.py`, duas rodadas)

| | Rodada 1 | Rodada 2 |
|---|---|---|
| Marcador → `sessions` no SSE (mediana / máx, 10 vezes) | 157 / 162 ms | 162 / 162 ms |
| `GET /api/sessions` (mediana / máx, 20 vezes) | 0,7 / 1,1 ms | 0,8 / 0,9 ms |
| CPU parado, sem lista (ms por s): Python / Rust com filhos | 7,3 / 0,5 | 7,2 / 0,8 |
| CPU com 1 lista aberta: Python / Rust com filhos | 11,5 / 7,5 | 12,5 / 7,7 |
| Pico de RSS do Rust | 27,8 MB | 28,3 MB |

Com sessões reais, marcador → lista: mediana 153 ms, máx 1148 ms (14 medidas; os acima de 200 ms
são sessões nascidas no próprio cenário, que esperam o tique). `GET` no isolado real: ≤ 10 ms (o
relógio da base arredonda a 10 ms; o roteiro agora mede em µs).

## Achados

- **Nenhum defeito da lista.** As falhas da rodada 1 eram do esperado: `plan_name` sai sem o prefixo
  de data; a conta é a pasta real em que o `claude` roda (o embrulho troca `.claude-provab` pela
  conta B, igual ao Python); `gh` é escondida do dono e não podia estar entre as vivas esperadas.
  O `conta` da rodada 2 conferiu 3 s depois de mandar a mensagem, com o turno ainda rodando; a
  conferência agora espera até 20 s a lista assentar e anota o tempo.
- **Fora da lista: `/clear` perdido no Claude com terminal** depois de interromper um
  `AskUserQuestion`. Repetiu nas três rodadas, nos modos `rust` e `python`: `POST /input` 200, o
  pane não mostra o `/clear`, nenhum transcript novo, e o "Responda só OK." seguinte fica na fila
  durável com `delivered: false` com a sessão parada. A lista mostra certo o transcript em uso. Não
  corrigido aqui: é o caminho de entrada (`terminal_input`/`_send_managed`), anterior à troca.

## Não provado

- Cartão de permissão do Claude **com terminal** (`pt` fica `working` com o marcador depois do
  `Bash`), como na sombra; o sem terminal chegou a `awaiting_input`.
- Codex abre em modo YOLO: sem cartão para provocar.
