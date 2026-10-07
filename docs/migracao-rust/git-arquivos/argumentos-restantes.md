# Argumentos que caíam no Python — levados ao Rust

Data: 04/10/2026. Branch `feat/workspace-missing-args` (de `hangar-server-parte1` `1b2be6f9`).
Contrato interno 12 → 13: o binário antigo recusa os campos novos, e com o número igual o Python
mandaria `create_worktree` com `new_branch` a um Rust que responderia "resultado incerto".

## O que mudou

| Argumento | Antes | Agora |
|---|---|---|
| `create_worktree(new_branch, base)` | fora do padrão, a função inteira rodava no Python | Rust: nome validado (`check-ref-format --branch`, sem `-` inicial nem espaço nas pontas), 409 se a branch existe, base padrão = branch atual, base remota via `origin/<base>`, `worktree add --no-track -b`, grava `branch.<b>.hangar-base` (falha só no log) |
| cópia dos ignorados (`copy_ignored`/`_main_root`) | só no Python; o caminho padrão pelo Rust criava worktree **sem** `.env` | Rust copia os ignorados da raiz do repositório principal em toda criação, sem seguir link e sem sobrescrever |
| `remove_worktree(force)` | fora do padrão, Python | Rust: `worktree remove --force`, 500 com a mensagem do git |
| `rows` de `citation_cwds`, `cited_elsewhere`, `find_elsewhere` | com linhas em memória, Python | as linhas vão como texto (`text_rows`) e o Rust lê delas em vez do arquivo (`citations::Transcript::Rows`) |

Pedido à ponte acima de 4 MiB (o `MAX_BODY` do Rust) não é enviado. Antes ele recebia 400 e,
numa escrita, virava "resultado incerto".

> Deixou de valer com o contrato 17 (`dono-unico/plano.md`, Task 8): o pedido grande demais vira
> erro 413 `workspace_request_too_large` em vez de rodar no Python, e `python_args` saiu do
> `delegate` — argumento novo da `main` entra no núcleo Rust junto.

## Testes

- `cargo test --locked --workspace`: 394 passaram, 0 falharam, 3 ignorados. `cargo check` do
  `desktop-native`: compila.
- pytest de `test_workspace_parity`, `test_workspace_bridge`, `test_worktrees`,
  `test_workspace_concurrency`, `test_rust_server`, `test_git_ops`, `test_file_citations`,
  `test_conversation_history`, `test_serve_file_cache`, `test_terminal_observer`: 431 passaram.
- Paridade nova (Python × `workspace_contract`): 7 casos de `create_worktree` (branch nova com
  base padrão, local e inexistente, branch já existente, nome `-x` e com espaço, branch existente
  sem `new_branch`), base só remota, remoção de worktree suja com e sem `force`, e as três
  citações com `rows` (o arquivo diz outra coisa). Os 11 falham contra o binário de `1b2be6f9`.
- O teste de assinaturas Python × campos Rust ficou sem exceções.

## Uso real

Backend isolado: `HOME` temporário em `/tmp`, porta 18965 (Rust), convite 18966, Connect 18968,
`CP_AUTH_TOKEN` próprio, `TMUX_TMPDIR` próprio, ambiente limpo (`env -i`), varredura de órfãos
desligada pelo lançador, `claude` falso. Quem rodou cada `git` foi anotado por um `git` de
passagem no `PATH` do teste (processo pai).

| Caso | Resultado | Quem rodou o git |
|---|---|---|
| Sessão em worktree com branch nova `feat-nova` e base `outra` | 200; `.env` copiado; sem upstream; `hangar-base=outra` | `hangar-server` |
| Sessão com branch nova sobre base só remota `so-remota` | 200; `worktree add --no-track -b … origin/so-remota`; `hangar-base=so-remota` | `hangar-server` |
| Branch nova `-x` | 400 `nome de branch inválido`, nada criado | — |
| Remoção em lote sem confirmar (worktree mesclada com arquivo não versionado) | `removed: []` | — |
| Remoção em lote confirmada (`paths` + `lossy`) | removida; `worktree remove --force` | `hangar-server` |
| Apagar individual sem confirmar / confirmado (arquivo ignorado) | 409 "há arquivos que serão perdidos" / removida com `--force` | `hangar-server` |
| `file/text` de `nota.md` e `docs/nota.md` citados com `cwd` de outra pasta | 200 com o texto da outra pasta | Rust (nenhum `file/text` no log do Python) |
| Absoluto nunca citado / arquivo não citado | 403 / 403 | Rust |

Diário: nenhum `workspace.reserva_python` nem `workspace.ponte`. Tudo parado e o `HOME` apagado
no fim.

Não conferido no uso real: `rows` pela ponte viva. Ele só existe em sessão Codex transferida
(registro concluído, com digest e fronteira); está coberto pelos testes de paridade e da ponte.
A regra de "mesclada" (`worktrees.is_merged`) nunca lê como mesclada uma worktree cuja base é só
remota, então ela não sai pelo lote; é comportamento anterior, fora deste trabalho.
