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
