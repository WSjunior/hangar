# Revisão da junção da lista + estado na parte1 (05/10/2026)

Revisão do `git diff origin/hangar-server-parte1 HEAD` na junção de `feat/session-list-state`
(221f4c7c5) pela coordenação (migracao-rust-3). Nada disso muda o que o dono vê hoje: com
`CP_LIST_SHADOW` desligado a lista segue do Python. **Tudo precisa estar corrigido antes das
Tasks 17-18 (troca)**, e os itens de comparação antes do relatório da `lista-tsombra`.

## Falha calada (silent-failure-hunter)

Caminhos relativos a `crates/hangar-server/src/list/` ou `backend/app/`.

### ALTO

1. `discover_other.rs:380-396` `sidecars()`: sidecar corrompido/parcial (Codex em
   `~/.hangar/codex-sessions`, Claude sem terminal em `~/.hangar/claude-headless`) é descartado
   pelo `filter_map` sem log — sessão viva some da lista. Registrar `list_sidecar_unreadable`
   (arquivo + tipo) no diário.
2. `shadow.rs:41,55-60` `accepted()`: `RUNTIME_FIELDS` aceitos para TODA linha Claude sem
   terminal, não só quando falta o snapshot do runtime — esconde qualquer erro de `state`,
   `question`, `pending_questions`. `classify.rs:106` cai em `"idle"` sem `problema` quando
   `facts.headless` não tem a sessão. Aceitar só com `ProduceFacts.headless` vazio.
3. `facts.rs:153-170` `ask()`: `map_err(|_| code)` joga fora o erro (hyper/serde);
   `facts.rs:105-111` `failed()` só faz `warn!` (1/min, fora do diário exportado); só o laço da
   sombra reporta `facts_unavailable`, o `produce` normal (o que vai servir) não.
   `demote()` (`facts.rs:177-181`) sem status nem erro. Levar motivo + status HTTP e chamar
   `diag.report` em `fetch` e `demote`.

### MÉDIO

4. `facts.rs:105-111` + `apply()`: depois de uma falha os fatos velhos (`hidden`, `shared`,
   `owners`, `overrides`, `frozen`) são reusados e só as linhas `OTHERS`/`aside` ganham
   `problema`; linhas Claude não. Ex.: sessão escondida depois de convidado, Python não
   responde, linha servida com acesso velho sem aviso.
5. `procs.rs:134-136,248-250` `argv()` vazio em EACCES/hidepid/corrida →
   `discover.rs:405-412` classifica Pi/omp/Kimi como `claude`, sem log. `fds()` e
   `marker_by_pids` (`discover.rs:236,251`) também calados.
6. JSON corrompido vira "sem estado" sem log: `facts_files.rs:135-137` (e fica em cache até o
   arquivo mudar), `links.rs:54-59` (par/grupo/loop somem), `plan.rs:230`,
   `hangar-workspace/src/worktrees.rs removed_at`. Aviso por arquivo com teto, como o
   `warn_file` já faz.
7. `mux.rs:46-52`: linha do `list-panes` com menos de 5 campos é descartada; só reclama se TODAS
   falharem. Cwd com `\n` some com a sessão. Contar descartes e acusar `mux_unparsed`.
8. `facts_files.rs:372-376`: pergunta pendente fora da janela de 256 KB vira "respondida" com log
   em `debug` — sessão sai de `awaiting_input` calada.
9. `capped.rs:48-56`: despejo do teto 256 sem log. `ContextCache` perde ctx/modelo;
   acima de 256 sessões/transcripts os caches por sessão relêem do zero a cada tique. Avisar no
   despejo ou dimensionar pelo número de linhas vivas.
10. `list_facts.py:135-141` `_orq_rows`: `except Exception: return []` só com log; o Rust recebe
    `ok=true` e mostra a lista sem orquestrações.
11. `bridge.rs:272-296` `refresh_git`: `keep(new, old)` mantém o último número bom quando o git
    falha (ex.: `safe.directory`) — contagem congelada sem marca.
12. `context.rs:200-206` `read_tail` → `(None, None)` em erro de I/O; `bridge.rs:454-457`
    `.ok()`; contexto fica velho ou cai em janela de 200k.
13. `shadow.rs:140-146,213-216`: diferença só é reportada após duas rodadas seguidas e qualquer
    rodada cega zera — diferença intermitente nunca chega ao diário (afeta o teste da
    `lista-tsombra`).

### BAIXO

- `facts_files.rs:169-178`: `warn!` sem teto em `entries()` repete a cada 1,5 s.
- `discover_other.rs:317`: UTF-8 inválido no índice do Kimi → `None` sem log.

## Rust: desempenho e correção (rust-reviewer)

Sem crítico; sem `unwrap` em dado externo, I/O bloqueante em `spawn_blocking`, capturas com teto 4,
nenhuma trava atravessando `.await`. O que sobra é custo por tique que cresce com o disco.

### ALTO — custo por tique proporcional a histórico que só cresce

1. `discover.rs:362-376,141-144` `newest_after_clear`: a cada tique, `read_dir` + `stat` de todo
   `*.jsonl` do projeto, por sessão Claude sem irmã no cwd (1000+ transcripts = 1000+ stats).
   Memorizar pelo mtime da pasta (o `/clear` cria arquivo e muda o mtime).
2. `discover_other.rs:185-188,258-267` (omp): caminho normal `sessions/-/<nome>` não existe →
   `find_in_root` varre a raiz a cada tique por sessão omp. Cache nome→caminho em `Capped`.
3. `discover_other.rs:316-327` (Kimi): sessão fora do `session_index.jsonl` relê e analisa o
   índice inteiro a cada tique; só acerto é guardado. Guardar o negativo por alguns segundos ou ler
   só a cauda nova pela chave do arquivo.
4. `discover_other.rs:480-481` `headless_transcript`: caminho esperado ausente → `read_dir` de
   `projects/` + `exists()` por pasta a cada tique; `sidecars()` (`:380-396`) relê todo JSON de
   `codex-sessions/` e `claude-headless/` a cada tique (conferir poda).
5. `facts_files.rs:225-240` `HookStates::refresh`: `stat` de todo marcador do `.hangar-state` por
   conta a cada tique; contas com symlink para a mesma pasta leem N vezes. Deduplicar pelo
   `canonicalize` como já faz com `sessions/`.
6. `discover.rs:249-279` `marker_by_pids`: `read_dir` + `stat` de todo `.hangar-active/*.json`
   (centenas) por sessão sem `--session-id`, por tique.

### MÉDIO

7. `bridge.rs:227-239,249-258`: `mem::take(&mut c.hooks)` deixa `Default` para `produce`
   simultâneo (sombra, `Snapshot`, hub) → leitura fria de todos os marcadores. Pôr `HookStates`
   sob a trava do `classifier`.
8. `bridge.rs:183-184,238-258`: trava `caches` segurada durante toda a I/O de descoberta e
   decoração (caudas até 8 MB, `/proc`, contexto, plano); `bridge.rs:236` segura `classifier`
   durante capturas tmux (até 5 s). `seed`/`forget`/`rename` do hub (criar/fechar sessão)
   esperam essas travas — centenas de ms, segundos a frio.
9. `procs.rs:228` (macOS/Windows): a cada 3 s atualiza cmd+environ de TODOS os processos para
   ~40 pids; no Windows é `ReadProcessMemory` por processo. Atualizar tudo no modo mínimo e
   cmd/environ só dos pids de agente.
10. `procs.rs:208` (macOS/Windows): pid ausente com retrato fresco ainda chama
    `refresh_processes_specifics(Some(pid))` (retrato inteiro no Windows); pids mortos batem nisso
    a cada tique. Retrato fresco + pid ausente → `None`.
11. `links.rs:589,605,611,650-652` (Windows): `owner()`, `of_this_repo`, `expand_user` e regex do
    Codex assumem `/`; com `\`/letra de unidade a worktree cai no `cwd` calada (branch errada).
12. `links.rs:553`, `context.rs:237`: `contains` por `windows(n).any(==)`, duas vezes por linha
    em até 8 MB; 40 sessões a frio ≈ 1 s. Usar `memchr::memmem::Finder` (já vem pelo `regex`).
13. `bridge.rs:503` `Operation::Snapshot` com `ProduceFacts::default()`: `owner_clients=0` entra
    no hash do pedido (`facts.rs:128`) e alterna a chave com o hub (ida ao Python a cada vez);
    Claude sem terminal classificado só pelo marcador. Resolver na Task 16/17.
14. `links.rs:656-666` `tail_lines`: `to_vec` de toda linha, a maioria descartada; rollout do
    Codex ativo muda a chave a cada tique → 256 KB lidos+copiados por tique por sessão.
15. `facts_files.rs:49,159`: `UNKNOWN_STATUSES` `HashSet<String>` sem teto com texto do disco.

### BAIXO

- `procs.rs` `env_var` relê `/proc/<pid>/environ` 5–8 vezes por sessão por tique; memo por tique.
