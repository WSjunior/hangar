# Estado da migração para Rust (05/10/2026, ~05:00) — para retomar

Leia este arquivo e `docs/migracao-rust/README.md`. Responda ao dono (Jefferson) em pt-BR, curto,
uma pergunta por vez, opções letradas. Branch da migração: `hangar-server-parte1` (PR #24),
pasta de coordenação `/home/jefferson/pessoal/hangar/.claude/worktrees/hangar-server-parte1`.

## Onde está (`hangar-server-parte1` em `883b78e8`)

- Partes 1, 2A, 2B, 2C, 2D, PR #30 (Git/arquivos no núcleo `hangar-workspace`).
- **Dono único** (`docs/migracao-rust/dono-unico/`, 11 Tasks, prova real `prova-real.md`): o Rust é
  o único dono do que já migrou; só a reserva do processo inteiro (modo `pending`/`rust`/`python`).
  Falha de operação = erro com código e motivo (503 `{ok:false,error_code,message,detail:{code,
  params:{motivo},msg}}`, diário via `/internal/diag`), nunca passar sessão ao Python com o Rust vivo.
- **Parte 3** (custos e uso no Rust): `/api/costs`, `/api/uso`, `/api/cotacao`; tela inicial pede
  `?view=summary` (1 MB → 16 KB); índice do zero 41 s → 6,4 s.
- **Lista de worktrees no Rust** (`/api/worktrees`, detalhe): ~1 s → ~135 ms. Mutações de worktree
  (apagar, lote, criar, fetch) seguem no Python (movem conversas entre contas).
- Contrato interno atual: **20** (`RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL`).
- As duas máquinas (esta e `notebook-jefferson`) rodam `5974a283` com o binário Rust novo.
- `scripts/medir-rust.sh`: comparação Rust × Python de tudo que é medível (chat, Git/arquivos,
  worktrees, custos com tamanho baixado). `scripts/prova-dono-unico.py`: prova real com backend
  isolado (`--n 10 --conta-b /home/jefferson/.claude-jefferson`).

## Ainda no Python

Lista de sessões/quadro/canvas; Codex sem terminal (decisão 2A) e envio do Codex com terminal;
Pi/Kimi/omp/orq; estado das sessões e terminal real (parte 4); contas, convidados, pareamento,
MCP, push, atualização, uploads, ditado (parte 6); cotas (o tempo é rede; mostrar o valor guardado
e atualizar por trás quando forem para o Rust); leitura de tela no Windows (por desenho); o
Supervisor que sobe o Rust (sai na parte 7).

## Pendências

1. **Teste manual do dono:** entrega duvidosa no terminal (`terminal_delivery_unknown`, não forçável
   por roteiro); transferência Claude → Codex no app real; telas de Custos e Uso (web, celular,
   nativo) e a frase do 503 num card.
2. `reopen_failed[RuntimeError]` intermitente na sessão com terminal 1,3 s após restart (1 vez em 7
   rodadas; entrega certa; causa não provada).
3. `/internal/runtime/policy` com geração antiga responde 500 com pilha (o Rust lê como
   `policy_refused`); deveria responder recusa.
4. Testes de tempo instáveis no Windows do CI (`terminal_runtime`, `hangar-cano peek_keeps_old_writer_tcp`)
   e no macOS (`terminal_routes.rs:165`): repetir passa; se voltarem, medir o que fica lento.
5. App não avisa quando uma sessão para num cartão de permissão (lista mostrava "trabalhando").
6. `scripts/medir-rust.ps1` (Windows) sem as partes novas.
7. Próximas partes do roteiro: 4 (estado e terminal real), 5 (adaptadores/envio dos provedores), 6
   (resto da API), 7 (remover o Python); Codex sem terminal no Rust (Task própria).

## Regras e decisões que valem

- Testes reais só com Haiku, backend isolado (`HOME` temporário, portas fora de 8765/8766/8768,
  `tmux -L` próprio, `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`); nunca `claude-200-1` nem
  `claude-200-3`; nunca subir backend extra sem o conserto de órfãos (já na branch).
- CI: conferir **job por job** (o resumo fica verde com job do macOS/Windows vermelho); não esperar
  o CI para seguir trabalhando.
- Push na `hangar-server-parte1` liberado; na `main` só por PR revisado. Merge sempre, nunca rebase
  nem `push --force`. Não comentar nos PRs.
- Antes de atualizar uma máquina, esperar o `server.yml` publicar o binário da mesma versão.
- Sessão que bate limite de uso: trocar de conta mantendo a conversa (`POST /api/sessions/<nome>/conta`
  ou resume do arquivo), nunca fechar e abrir outra.
- Sessões filhas não podem parar em cartão de permissão: vigia que aprova (`select` opção 1).
- O dono quer o que está migrando feito no Rust, não conserto paliativo no Python.
