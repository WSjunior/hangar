# Desempenho da Fase A antes do ListHub

Siga as regras de `2026-10-05-lista-estado-task-kickoff.md` (sem push, `cargo-slot`, sem LSP,
revisão `ecc:rust-reviewer` + `ecc:silent-failure-hunter`). Branch `feat/session-list-state-perf`.

As funções de `crates/hangar-server/src/list/` vão rodar a cada tique de 1,5 s do `ListHub`
(Task 17) com até 20 sessões. A auditoria apontou o que custa por tique. Corrija, com teste que
prove a reutilização (contador de leituras ou de chamadas no fake) e sem mudar a saída dos goldens:

1. `procs.rs:173-181` (sysinfo, Windows/macOS): cada leitura por pid tira um snapshot de todos os
   processos; ~15–20 por sessão e por tique. Um refresh por tique (todos os pids de uma vez,
   `cmd`+`environ`) servindo `argv`/`env_var`/`start_time` do mesmo `System`, ou memo por
   `(pid, start_time)` podado no `children()`.
2. `facts_files.rs:152-188` `HookStates::load`: relê todo `.hangar-state/*.json` e
   `sessions/*.json` por tique. Manter o estado e reler só o arquivo cujo mtime mudou (como
   `Resolver.markers`, `discover.rs:248`); arquivo sumido sai do mapa.
3. `links.rs:325-410` `claude_tail`: a chave `(mtime, len)` muda a cada escrita e a falha lê até
   8 MB para trás com bloco de 256 KB zerado. Teto de frequência (reusar o valor com menos de 10 s)
   e ler só o que cresceu desde a última leitura.
4. `links.rs:415-421` `repo_candidates`: `worktree_paths` (read_dir + leitura por worktree) por
   linha e por tique; `removed()` relido em `:294` e `:417`. Cache por repositório principal com
   chave no mtime de `.git/worktrees` e do `worktrees-removidas.json`.
5. `facts_files.rs:239-292` `answered_after`: cache por (mtime+len do jsonl, mtime do sidecar).
6. Menores: `btime` em `OnceLock` (`procs.rs:95`); `external_pairs.json` lido uma vez por tique
   (`links.rs:100-107`); cache positivo `sid → wire` do índice Kimi (`discover_other.rs:299-309`).
7. Falhas caladas em `facts_files.rs` (JSON torto em `open_question`/`published_status` só em
   debug, EACCES ignorado): aviso com `warn_limit`, código e nome do campo.

Fora: poda dos caches (`forget`/`retain`) é da Task 17. Meça antes/depois em release o custo de
um tique da descoberta+decoração com 20 sessões sintéticas (teste `#[ignore]` como o `tick_cost`
da Task 12) e mande os números no relatório a `lista-org`.
