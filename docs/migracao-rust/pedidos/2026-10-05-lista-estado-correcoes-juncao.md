# Correções da revisão da junção (antes das Tasks 17–18)

Fonte: `2026-10-05-lista-estado-revisao-juncao.md` (13 de falha calada) e os ALTOS de desempenho da
coordenação. Siga `2026-10-05-lista-estado-task-kickoff.md` (sem push, `cargo-slot`, sem LSP,
velocidade primeiro, revisão `ecc:rust-reviewer` + `ecc:silent-failure-hunter`). Cada item: teste que
falha sem a correção; falha vai ao diário com código e campo (`diag.report`, teto por arquivo como o
`warn_file`), nunca só `debug`/`warn` sem teto. Goldens sem mudança, salvo regra que mude no Python.

Divisão por arquivo, para um escritor por arquivo. Caminhos em `crates/hangar-server/src/list/`.

## lista-tdesc — descoberta (`procs.rs`, `mux.rs`, `discover.rs`, `discover_other.rs`, `links.rs`, `hangar-workspace/src/worktrees.rs`)

- Falha calada: 1 (`sidecars()` → `list_sidecar_unreadable`), 5 (`argv`/`fds`/`marker_by_pids`
  vazios por EACCES/corrida: Pi/omp/Kimi não viram `claude` calados), 6 (`links.rs:54-59`,
  `removed_at`), 7 (`mux.rs` conta descartes → `mux_unparsed`), BAIXO do índice Kimi com UTF-8
  inválido.
- Desempenho (varreduras por tique que crescem com o disco): `newest`/`newest_after_clear`,
  `marker_by_pids`, `find_in_root` do omp, índice do Kimi sem cache negativo,
  `headless_transcript` e `sidecars()` relidos por tique. Cache por mtime da pasta (ou do arquivo)
  com teto; medir antes/depois em release com 20 sessões e com a pasta real desta máquina
  (~600 marcadores, ~150 projetos).

## lista-tfatos — fatos, decoração e ponte (`facts.rs`, `facts_files.rs`, `plan.rs`, `context.rs`, `capped.rs`, `bridge.rs`, `backend/app/list_facts.py`)

- Falha calada: 3 (`ask`/`failed`/`demote` com motivo + status no diário, também no `produce`
  normal), 4 (fatos velhos marcam `problema` em TODAS as linhas que dependem deles, inclusive
  Claude escondida/compartilhada), 6 (`facts_files.rs:135-137`, `plan.rs:230`), 8 (pergunta fora
  da janela de 256 KB não vira "respondida" calada), 9 (despejo do teto avisa, teto dimensionado
  pelas linhas vivas), 10 (`_orq_rows` falha → fato com erro, não lista sem orq), 11 (`refresh_git`
  marca contagem velha), 12 (`read_tail`/`.ok()` do contexto), BAIXO `entries()` com teto.
- Desempenho: `HookStates` sem dedupe; travas seguradas durante I/O em `bridge.rs:183-258` e
  `seed`/`forget`/`rename` esperando a produção — I/O fora da trava, trava só para trocar o valor.

## lista-tsombra — instrumento (`shadow.rs`, `classify.rs:106`)

- Itens 2 e 13, antes de qualquer resultado do teste isolado (já pedido).

## Rust: desempenho e correção, médios e baixo (da coordenação)

Numeração da coordenação (1–6 são os ALTOS acima).

- **lista-tdesc:** 9 e 10 (`procs.rs` sysinfo atualiza `cmd`/`environ` de todos os processos, e
  pid ausente refaz o retrato inteiro); 11 (`links.rs` assume `/` como separador no Windows); 12 em
  `links.rs` (`contains` ingênuo → `memchr::memmem`); baixo: `env_var` relê o `environ` 5–8× por
  tique.
- **lista-tfatos:** 7 (`mem::take` dos hooks: duas produções juntas releem tudo); 8 (travas);
  13 (`Snapshot` com `ProduceFacts::default` alterna a chave do hash com o hub); 14 em
  `context.rs` (`tail_lines` com `to_vec`); 12 se houver o mesmo `contains` em arquivo seu;
  15 (`UNKNOWN_STATUSES` sem teto).

Ao terminar, cada sessão manda a `lista-org` o hash, o que mudou por item e as medidas.
