# Lista de worktrees no Rust — plano

Frente B do pedido `pedidos/2026-10-04-custos-e-worktrees-rust-kickoff.md`. Branch
`feat/worktrees-rust`, base `e0d13084` (dono único, contrato 17). Contrato desta frente: **19**.

## Desenho

- `GET /api/worktrees` e `GET /api/worktrees/detail` passam a ser do Rust (só o dono; convidado
  segue ao Python, que responde 403 como hoje). Com o Rust de pé, falha vira 503 com código e
  motivo, nunca repasse ao Python.
- O Python só entrega metadados por uma rota nova, `GET /internal/worktrees/context`: raízes,
  pastas já filtradas pelas raízes (sessões vivas + pastas com conversa nos últimos 30 dias),
  sessões (`name`, `cwd`, `worktree_path`, `jsonl`) e as pastas `projects/` das contas.
  Rota `/internal` nova → `RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL` sobem para 19.
- Núcleo em `crates/hangar-workspace/src/worktrees.rs`, exposto como `Operation::ListWorktrees`
  e `Operation::WorktreeStatus` (a ponte de contrato `workspace_contract` compara com o Python).
- Menos `git` por worktree (9–11 → 4–5): `config --get-regexp` uma vez por repositório (base e
  upstream de todas as branches); `rev-list --left-right --count base...branch` dá ahead e behind
  e substitui `rev-parse` + `merge-base --is-ancestor` (branch é ancestral da base ⇔ ahead = 0);
  o último commit sai do `log base..branch` quando ahead > 0; `rev-parse @{upstream}` só quando a
  branch tem upstream e não é ancestral. `status --porcelain` e `ls-files --ignored` ficam
  separados (formatos de caminho diferentes que a tela já lê).
- Worktrees medidas em paralelo, 8 por vez, numa vaga de leitura do servidor.
- Tamanho em disco no Rust: cache de 15 min (falha volta em 60 s), uma fila de fundo, uma
  medição por vez; a chave inclui a data de criação da worktree, então recriar no mesmo caminho
  não herda o tamanho velho.
- Mutações (`delete`, `delete-merged`, `create`, `fetch`) ficam no Python: movem conversas entre
  contas e gravam o mapa de remoções, fora do que o núcleo faz hoje. Não é natural no mesmo
  trabalho.

### Task 1: Núcleo da lista e da situação no hangar-workspace

- [ ] **Step 1: Teste de paridade** em `backend/tests/test_worktrees_parity.py`: repositório com
  worktrees mesclada, suja, com ignorado, à frente/atrás, upstream sumido, pasta apagada e
  sessão dentro; compara `worktrees.list_all`/`status` com a ponte `workspace_contract`.
- [ ] **Step 2: `worktrees.rs`** com `list_all`/`status` paralelos e as operações no `Operation`.
- [ ] **Step 3: Testes Rust** do que a paridade não cobre (paralelo preserva ordem, falha de git
  marca `degraded`, chave de tamanho).

### Task 2: Tamanho em disco por trás

- [ ] **Step 1: Medição em fila única** com cache, falha com nova tentativa e total parcial.
- [ ] **Step 2: Teste** de pasta medida, pendente e recriada.

### Task 3: Contexto no Python e rotas no Rust

- [ ] **Step 1: `/internal/worktrees/context`** no `internal_api.py` + teste; protocolo 19.
- [ ] **Step 2: Rotas no hangar-server** (validação de repositório e de worktree iguais às do
  `api.py`, 503 com código quando o contexto falha) + testes com o Python falso.

### Task 4: Prova

- [ ] **Step 1: Medição antes/depois** no backend de teste isolado (38 worktrees; meta ≤ 200 ms).
- [ ] **Step 2: Revisão por subagente** (`ecc:rust-reviewer`, `ecc:silent-failure-hunter`,
  `ecc:python-reviewer`) e correções.
- [ ] **Step 3: Testes** (`cargo test --locked --workspace`, pytest dos arquivos tocados,
  `npm run check` na raiz) e CI do `server.yml` job por job.
