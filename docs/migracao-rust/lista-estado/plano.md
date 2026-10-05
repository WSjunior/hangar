# Custo fixo do Supervisor — plano (proposto no lugar da lista no Rust)

> A lista no Rust não foi planejada: com 20 sessões ela soma 2,4–2,5 pontos de um núcleo, e o
> Supervisor gasta 3,7–4,3 pontos sozinho, com zero sessões (`medicao.md`). Este plano tira o
> custo do Supervisor. Execução só depois da aprovação do dono.

**Objetivo:** o vigia do `hangar-server` continua provando a contenção do grupo do Rust, mas sem
percorrer todos os processos da máquina a cada 0,25 s e sem regravar o registro quando nada mudou.
**Medida de pronto:** backend isolado com zero sessões e sem cliente abaixo de 1% de um núcleo
(hoje 4,2%), e `runtime-process.json` regravado só quando aparece um processo novo no grupo.

## Desenho

Hoje (`backend/app/rust_server.py:378-382` → `backend/app/runtime_process.py:248-282`), a cada
volta de 0,25 s do vigia: `_group_members` (`:148-158`) chama `psutil.process_iter` e lê `status`
e `create_time` de todo processo da máquina (~560 aqui, 9,5–11 ms de CPU), acrescenta os do grupo
ao dicionário `members` (pid → nascimento, que só cresce) e regrava o registro com `fsync`.

O registro é lido antes de cada subida do Rust (`reconcile_startup`, `:284-332`, chamado em
`spawn_contained`, `:117`) e pelo `cleanup` (`:208-212`, só o `life`). Depois de uma queda do
backend, o grupo órfão só é morto se algum processo vivo dele tem o nascimento anotado
(`:321-323`). O próprio Rust é membro
sempre (o grupo é a sessão dele, `start_new_session`). Os outros membros são filhos que ele lança
(cliente `tmux -C`, `git`); um neto cujo pai morreu sai da árvore, mas continua na sessão.

Mudanças, só no Python (o Supervisor fica no Python até a parte 7, por desenho):

| O quê | Como |
|---|---|
| Achar membros (só no `refresh_members`) | No Linux, descer a árvore a partir do pid do Rust por `/proc/<pid>/task/*/children`, conferindo `os.getsid` de cada um: custo do tamanho do grupo. A varredura da máquina inteira (`process_iter`) fica como reforço a cada 5 s, para o neto que saiu da árvore. Fora do Linux (macOS), só a varredura de 5 s. No Windows nada muda: o Job já lista os membros. |
| Gravar | Só quando `members` ganhou pid novo, na primeira volta, ou enquanto a última gravação não deu certo: disco cheio continua sendo tentado a cada volta, como hoje (`runtime_process.py:273`, `rust_server.py:383-393`). O conteúdo do registro é o mesmo; nenhum leitor muda. |
| `cleanup` e `reconcile_startup` | Continuam com a varredura da máquina inteira (`_group_members`, `:189`, `:321`): rodam raramente, e depois de uma queda o Rust já morreu, então a descida pela árvore não acharia os netos que ficaram na sessão. |
| Vigia | O poll de 0,25 s continua (é ele que vê o Rust cair). |

Fica igual: formato do registro, o comportamento de `cleanup` e `reconcile_startup`, contrato interno (22), Rust.

## Restrições

- Testes escritos primeiro, vistos falhar sem a Task, rodados só os dos arquivos tocados
  (`cd backend && uv run pytest tests/test_runtime_process.py tests/test_rust_server.py`).
- Ler antes "Regras vigentes" de `docs/decisoes/windows.md` (subprocesso, trava de arquivo, teste
  que simula plataforma: usar o Job falso, nunca `monkeypatch` de `os.name`).
- Backend de prova isolado (`HOME` temporário, portas fora de 8765/8766/8768, `tmux -L` próprio,
  lançador com `matar_orfaos` desligado); nada no backend real.
- `git add` por caminho; commit em inglês.

## Lotes

Serial: Task 1 → Task 2.

### Task 1: Membros do grupo pela árvore e registro gravado só quando muda

**Arquivos:** `backend/app/runtime_process.py` (`refresh_members` e uma função nova de descida pela árvore; `_group_members` intocada),
`backend/tests/test_runtime_process.py`, `docs/decisoes/plataforma.md` (entrada do hangar-server:
uma linha com a regra e a medida).

**Falha sem ela:** `test_refresh_members_writes_only_when_members_change` (duas voltas sem processo
novo: uma gravação só, contada pelo `atomico.substituir`); `test_refresh_members_follows_tree_without_full_scan`
(processo contido que lança um filho: o filho entra em `members` com `psutil.process_iter`
trocado por um que falha, dentro dos 5 s); `test_full_sweep_catches_orphan_in_session` (filho
lança um neto e sai: com o intervalo do reforço zerado, o neto entra em `members`);
`test_windows_job_refresh_writes_once` (contenção com Job falso: nenhuma varredura, uma gravação);
`test_refresh_members_retries_after_failed_write` (primeira gravação levanta `OSError`: a volta
seguinte, sem pid novo, grava de novo); `test_reconcile_still_finds_orphan_grandchild` (Rust morto,
neto vivo na sessão com nascimento anotado: `reconcile_startup` o mata, como hoje).

- [ ] **Step 1: Ler "Regras vigentes" de `docs/decisoes/windows.md` e conferir as linhas citadas no Desenho na base**
- [ ] **Step 2: Testes acima, vistos falhar**
- [ ] **Step 3: Descida pela árvore `/proc/<pid>/task/*/children` com `os.getsid` só no `refresh_members`, reforço por `process_iter` a cada 5 s (e sempre fora do Linux), gravação só com `members` mudado ou gravação anterior falha**
- [ ] **Step 4: Linha da regra em `docs/decisoes/plataforma.md`; testes focados; revisar**

### Task 2: Medida antes e depois, isolado e no uso real

**Arquivos:** `docs/migracao-rust/lista-estado/medicao.md` (seção "Depois do conserto").

**Falha sem ela:** não há teste automatizado; a prova é a medida.

- [ ] **Step 5: Backend isolado da branch, zero sessões e sem cliente, 3 janelas de 30 s: CPU do Python (`/proc/<pid>/stat`) e número de gravações do `runtime-process.json` (mtime a cada segundo); repetir com 20 sessões Haiku e um cliente da lista**
- [ ] **Step 6: Queda simulada no isolado: `kill -9` no backend com um `tmux -C` do Rust vivo, subir de novo e conferir que `reconcile_startup` mata o grupo órfão (o caso que o registro existe para cobrir)**
- [ ] **Step 7: Uso real com o dono, depois do Atualizar: CPU do backend em repouso por 60 s e mtime do `runtime-process.json` (verificação manual)**

## Pergunta ao dono

Uma, fechada:

- **A) Consertar o Supervisor agora (este plano), medir o backend real e deixar a lista na fila**
  até o número de sessões pedir (≈ 50) ou junto da parte 4. ➡️
- **B) Planejar a lista no Rust mesmo assim**, sabendo que rende ~2 pontos com 20 sessões.
- **C) Ir para outra parte do roteiro** (4, 5 ou Codex sem terminal) e deixar as duas para depois.

## Quando a lista for ao Rust

O que a medição e o inventário já fixam para esse plano futuro:

- Um stream por servidor, nunca um por card; o produtor continua único, como o `_ListRefresher`
  (`backend/app/sse.py:409-555`), e o `GET` continua servindo o retrato decorado.
- O Rust é o único dono do que migrar: falha vira `list_error`/503 com código, nunca a lista
  passando ao Python com o Rust de pé. Convidados e os filtros de visibilidade
  (`guest_users.filter_visible`, `guest_safe`) precisam ir junto ou receber a lista pronta do Rust.
- O maior custo é a descoberta (`registry.list`, 14 ms/s com 20 sessões), não a classificação
  (0,05 ms/s) nem o git (já no Rust). Os efeitos colaterais da descoberta (varredura de pares
  mortos, rebaixar marcador) e o `stall_watch` têm de ter dono definido.
- Quatro estados em toda tela que consome a lista (carregando, vazio, erro e sucesso) já existem
  nos clientes (`list_error`); o contrato de eventos não muda.
- Não existe tipo da linha da lista no `crates/hangar-api`; o nativo lê campos que o TS não
  declara (`inventario.md`, seção 7).
