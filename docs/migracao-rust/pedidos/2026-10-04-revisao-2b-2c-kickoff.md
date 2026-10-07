# Revisão com correção da 2B e da 2C, provada com sessões reais

A 2B (sessões sem terminal no Rust) e a 2C (leitura da tela das sessões com terminal pelo Rust)
foram feitas e revisadas pelo Codex, com testes que só usavam peças falsas. No uso real do dono
apareceram defeitos graves, todos já corrigidos na `hangar-server-parte1`:

| Defeito | Parte | Commit da correção |
|---|---|---|
| pid do cano comparado com o do agente: toda sessão recusada | 2B | `fac8d377` |
| fila gravando o estado inteiro com fsync segurando a trava do laço (congelamentos de 6 s) | 2B | `089fec02` |
| estado da fila sem limite (88 MB) | 2B | `2176c49c`, `dee5c3e6` |
| Rust em erro para sempre sem o Python assumir; sem registro do motivo | 2B | `74177469`, `32a6b203`, `5008e3b8` |
| observador do tmux read-only bloqueando a digitação | 2C | `5ea4e297` |

A causa comum: a 2B escreveu fila, trava de dono e recuperação do zero
(`runtime_queue.py`, `runtime_coordinator.py`, `runtime_policy.py`, `runtime_receipt.py`,
`runtime_adapter.py` e `crates/hangar-server/src/runtime/`) em vez de partir do que o Python já
resolvia, e a 2C acrescentou comportamento que o Python nunca teve.

## Objetivo

Achar e corrigir os defeitos que ainda restam na 2B e na 2C, pela causa, e provar com sessões
reais. Para cada problema que o Python antigo já resolvia, leia como ele resolvia
(`backend/app/pqueue.py`, `adapters/claude_headless/adapter.py`, `adapters/codex/sem_terminal.py`,
`state.py`, `preview.py`, `tmux.py`, `terminal_input.py`, e `git log` desses arquivos para ver os
bugs que já custaram) e faça do jeito do Rust, para ganhar desempenho. Se achar solução melhor para
o mesmo problema, use a melhor e explique por quê.

Procure especialmente, no Python novo e no Rust:
- trava segurada durante disco, rede ou processo externo; laço de eventos esperando E/S;
- dado que só cresce (estado, mapas, filas, caches, logs) e cópia ou serialização do estado inteiro
  por operação;
- falha do Rust que não passa só aquela parte daquela sessão para o Python (regra da migração: falha
  antes de qualquer efeito tenta 3 + 1 e passa; falha com efeito possível nunca se repete), e falha
  sem motivo no diário ou no log;
- mensagem duplicada ou perdida na troca de dono (memória do projeto: a fila antiga já redigitou
  mensagens em loop; regras: matching cru + `queue-operation`, nunca reenviar em `working`,
  confirmar em todo `idle`, dedup integral no front);
- teste que só passa com peça falsa (dados iguais dos dois lados, processo que não existe na vida
  real);
- diferenças de comportamento contra as "Regras vigentes" de `docs/decisoes/harnesses.md`,
  `windows.md` e `plataforma.md`.

## Onde trabalhar

Pasta `/home/jefferson/pessoal/hangar/.claude/worktrees/revisao-2b-2c`, branch `revisao-2b-2c`,
criada de `origin/hangar-server-parte1` em `2d91c651`. Use `git -C <essa pasta>`. A 2D (envio às
sessões com terminal) está sendo finalizada em paralelo na branch `hangar-server-parte2d` pela
sessão `rust-parte2d-fim`: não mexa no caminho de envio do terminal (`terminal_input.py`, driver de
envio da 2D); se um defeito seu cair lá, me avise em vez de corrigir.

## Testes reais — leia antes de subir qualquer backend

- **Primeiro passo obrigatório:** traga o commit `bb80cf31` da branch `hangar-server-parte2d`
  (`git cherry-pick bb80cf31`). Sem ele, subir um backend de teste mata os canos das sessões reais do
  dono: a varredura de órfãos olha todos os processos do usuário. Isso já aconteceu hoje às 05:36.
- Backend de teste isolado: `HOME` temporário, portas livres diferentes de 8765/8766/8768,
  `CP_AUTH_TOKEN` próprio, servidor tmux próprio (`tmux -L <nome>`). Ele não pode enxergar nem
  adotar nenhuma sessão real.
- Sessões Claude de teste **só com o modelo Haiku** (`--model claude-haiku-4-5`), conta
  `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200` (logada, sem sessões vivas). Nunca use a
  `claude-200-1` (é a da sessão coordenadora) nem copie credenciais. O backend de teste não lê nem
  grava arquivos do Hangar dentro dessa pasta.
- Roteiro mínimo, sessões sem terminal (Claude, e Codex se houver login): mensagem simples (uma
  entrega no transcript), várias seguidas com o Claude ocupado (fila, sem duplicar), pergunta e
  permissão respondidas, `/clear`, reiniciar só o backend de teste com mensagem na fila, derrubar o
  `hangar-server` de teste no meio de uma entrega, forçar falha antes de efeito (3+1 tentativas e só
  aquela sessão no Python, com motivo no diário e no log do Rust), 200 mensagens seguidas medindo o
  tamanho do estado e o tempo por mensagem. Sessões com terminal (2C): estado e prévia certos
  enquanto o Claude trabalha, e digitação pelo Python funcionando com o observador ligado.
- Anote cada caso com resultado e evidência em `docs/migracao-rust/parte2b/revisao-real.md`. No fim,
  encerre as sessões e o tmux de teste, pare o backend de teste e apague o `HOME` temporário.

## Como entregar

- Uma correção por commit, cada uma com teste que falha sem ela (em inglês, `git add` de caminhos
  explícitos, `HANGAR_SEM_PASSO=1`). Revisão independente por subagente do conjunto antes de subir.
- Testes completos: `cargo test --locked --workspace`, pytest dos arquivos do runtime, terminal e
  fila (falhas conhecidas desta máquina: `omp` não instalado; 3 de `test_update_channel` que dependem
  da ordem).
- Push só da `revisao-2b-2c` e CI do `server.yml` verde nos três sistemas. Não junte na
  `hangar-server-parte1`: quem junta é a sessão `integracao-main`.
- Mande para `Migracao-Rust` (`hangar-send Migracao-Rust "…"`): cada defeito achado (onde, efeito,
  como o Python resolvia, como ficou), commits, testes com números, casos reais com resultado, link do
  CI. Decisão que mude comportamento visto pelo dono → pergunte a `Migracao-Rust` e espere.

## Proibido

Subir, reiniciar ou parar o backend real ou o serviço `hangar-backend`; usar as portas
8765/8766/8768; tocar no tmux padrão ou em sessão real; rodar instaladores; mexer na `main` ou na
`hangar-server-parte1`; modelo diferente do Haiku nas sessões de teste; log com texto de conversa.
