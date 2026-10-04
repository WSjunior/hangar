# Custos (tela inicial) e lista de worktrees no Rust, do jeito certo

Decisão do dono (04/10/2026): as duas telas vão para o Rust, sem conserto paliativo no Python.
Regra vigente da migração (README, "O Rust é o único dono do que já migrou"): com o Rust de pé, a
rota migrada é só dele; falha vira erro com código e motivo (503 `{ok:false,error_code,message,
detail:{code,params:{motivo},msg}}`, diário via `DiagClient`), nunca repasse ao Python por falha. O
Python só atende quando o Rust inteiro não está de pé.

Base das duas frentes: `feat/rust-single-owner-789` em `e0d13084` (dono único Tasks 1–9, contrato 17).
O kick-off diz qual frente é a sua. Contrato interno: frente A usa **18**, frente B usa **19**, se
mudarem rota `/internal`, evento ou variável do filho (subir `RUST_SERVER_PROTOCOL` e
`INTERNAL_PROTOCOL` juntos).

## Medição de partida (04/10, esta máquina, subagente; scripts em
`/tmp/claude-1000/-home-jefferson-pessoal-hangar--claude-worktrees-hangar-server-parte1/1b48af83-457b-4a01-956f-c726fbc9e9d6/scratchpad/`)

- `GET /api/worktrees` (2 repos, 38 worktrees): ~1,0 s. 80–85% são 9–11 processos `git` por
  worktree, **em sequência** (`worktrees.list_all` → `status()`); os mesmos comandos com 8 em
  paralelo: 850 → 205 ms. `ahead/behind` cabem num `rev-list --left-right --count`; `git config`
  por repo, `head_info(main)` uma vez por repo. Tamanho em disco fora da resposta (cache 15 min,
  fila de fundo; `du` 5× mais rápido que `os.scandir`). Scripts: `bench.py`, `du.py`.
- `GET /api/costs` (tela inicial de web, nativo e app): 2–8 ms pronto, 90–200 ms remontando; não lê
  `.jsonl` na requisição (índice SQLite em segundo plano). A resposta tem 1 MB (135 KB gzip) e os
  três clientes usam só `totals`, `by_model`, `by_day`, `sem_tarifa`, `usd_brl`, `applied` (17 KB);
  `combos` sozinho tem 843 KB. Em `_somar` a identidade da sessão é gerada 7× por linha. Reconstrução
  do zero do índice: ~29 s em Python sequencial. Scripts em `medir/`.

## Frente A — parte 3: custos e uso no Rust (branch `feat/parte3-custos-rust`)

A parte 3 já foi feita na máquina `casa`: `origin/hangar-server-parte3` (`1a2bc1b9`), spec e plano
em `docs/migracao-rust/parte3/`. CI falhava em macOS (`costs_routes.rs:571`, relatório não
invalidado após gravar no índice) e Windows (`contract_costs_index.rs:68`, obtido 1 esperado 0 na
releitura incremental).

1. Leia `docs/migracao-rust/parte3/` (spec e plano), o README da migração e o `CLAUDE.md`.
2. Faça merge de `origin/hangar-server-parte3` na sua branch (nunca rebase), resolvendo com o que
   veio depois (dono único, contrato 17 → o seu é 18). Ajuste a parte 3 à regra do dono único:
   sem repasse por falha ao Python com o Rust de pé.
3. Corrija pela causa as duas falhas de CI acima, com teste.
4. Tela inicial: os clientes passam a pedir só o que usam (parâmetro de visão enxuta no
   `/api/costs`, decidido por você, com o mesmo formato dos campos usados); o Rust não monta
   `combos`/`by_project` quando não pedidos; cliente antigo sem o parâmetro continua recebendo o
   relatório inteiro. Atualize web, nativo e `mobile/` (lógica em `packages/core`).
5. Prova: medição antes/depois de `/api/costs` (pronto, remontando, tamanho) e da reconstrução do
   índice do zero; paridade com o Python pelos golden do repositório; teste manual das telas de
   Custos e Uso fica registrado como pendente para o dono.

## Frente B — lista de worktrees no Rust (branch `feat/worktrees-rust`)

1. Leia `backend/app/worktrees.py`, as rotas `/api/worktrees*` em `backend/app/api.py`,
   `crates/hangar-workspace` e `crates/hangar-server/src/workspace_routes.rs`, e o que os clientes
   (web `frontend/src/screens/Worktrees.svelte`, nativo, `mobile/app/config/worktrees.tsx`) leem.
2. Escreva um plano curto em `docs/migracao-rust/worktrees/plano.md` (formato `### Task N:` /
   `- [ ] **Step N: …**`) e siga sem esperar aprovação, exceto se algo mudar o que o dono vê.
3. Porte para o `hangar-workspace`/`hangar-server` a leitura da lista e do estado de cada worktree
   (o que `list_all`/`status` entregam hoje, mesmo formato), com os `git` em paralelo e menos
   chamadas por worktree; tamanho em disco calculado por trás no Rust, com cache. As mutações
   (`delete`, lote, criar) que já usam o núcleo continuam corretas; leve-as para a rota do Rust se
   for natural no mesmo trabalho.
4. Paridade com o Python (teste comparando as saídas sobre repositórios de teste), medição
   antes/depois no backend de teste (meta: ~200 ms ou menos para 38 worktrees).

## Regras das duas frentes

- Teste que falha sem cada mudança; revisão por subagente (`ecc:rust-reviewer` +
  `ecc:silent-failure-hunter` + o da stack do front); `cargo test --locked --workspace`, pytest dos
  arquivos tocados, `npm run check` na raiz (falhas conhecidas: 3 erros em
  `mobile/src/features/ditado/useDitado.ts`, 4 testes de `semTerminal.test.tsx`); CI do `server.yml`
  job por job (fora do filtro de caminho → dispare à mão); não espere o CI para seguir trabalhando.
- Backend de teste isolado quando precisar (`HOME` temporário, portas livres diferentes de
  8765/8766/8768 e das outras sessões, `CP_AUTH_TOKEN` próprio, `tmux -L` próprio). Pare tudo e
  apague o `HOME` no fim.
- Commits em inglês, `git add` de caminhos explícitos, `HANGAR_SEM_PASSO=1`. Push só da sua branch;
  não junte: me avise. Quando a `feat/rust-single-owner` andar, faça merge dela.
- Proibido: subir/reiniciar/parar o backend real ou o `hangar-backend`; usar 8765/8766/8768; tocar
  no tmux padrão ou em sessão real; rodar instaladores; mexer na `main`, na `hangar-server-parte1`
  ou na `feat/rust-single-owner`; `push --force`; texto de conversa em log.

Resultados e perguntas para `migracao-rust-2`.
