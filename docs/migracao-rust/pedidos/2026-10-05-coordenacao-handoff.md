# Estado da migração para Rust (05/10/2026, ~05:00) — para retomar

Leia este arquivo e `docs/migracao-rust/README.md`. Responda ao dono (Jefferson) em pt-BR, curto,
uma pergunta por vez, opções letradas. Branch da migração: `hangar-server-parte1` (PR #24),
pasta de coordenação `/home/jefferson/pessoal/hangar/.claude/worktrees/hangar-server-parte1`.

## Onde está (`hangar-server-parte1` em `5cc4d2af6`)

- Partes 1, 2A, 2B, 2C, 2D, PR #30 (Git/arquivos no núcleo `hangar-workspace`).
- **Dono único** (`docs/migracao-rust/dono-unico/`, 11 Tasks, prova real `prova-real.md`): o Rust é
  o único dono do que já migrou; só a reserva do processo inteiro (modo `pending`/`rust`/`python`).
  Falha de operação = erro com código e motivo (503 `{ok:false,error_code,message,detail:{code,
  params:{motivo},msg}}`, diário via `/internal/diag`), nunca passar sessão ao Python com o Rust vivo.
- **Parte 3** (custos e uso no Rust): `/api/costs`, `/api/uso`, `/api/cotacao`; tela inicial pede
  `?view=summary` (1 MB → 16 KB); índice do zero 41 s → 6,4 s.
- **Lista de worktrees no Rust** (`/api/worktrees`, detalhe): ~1 s → ~135 ms. Mutações de worktree
  (apagar, lote, criar, fetch) seguem no Python (movem conversas entre contas).
- **PRs organizados (05/10):** na `main`, #51 bandeja do nativo (`7f0a893ce`, com o ícone do botão
  de remover atalho, que ficava em branco: `9a0e95230`) e #52 faixa de mods (`a0873c394`, com
  `a6b0d7da7`: a linha de rótulos perdia padding/borda/fundo). Na migração, `5cc4d2af6` juntou a
  `main` inteira (#45–#48, #51, #52) e os PRs #49 (git da sessão segue a worktree, `git_cwd` no
  contexto interno; trava de branch do loop lê a worktree), #50 (pele Terminal, `ChatEvent.patch` nos
  dois parsers; teto do diff na soma das linhas, `f4b2a6ea8`) e #53 (clique e painel de mod; causa
  da recusa no log, `30678808f`). Os avisos de mod (`plugin_toast`, #45) chegam pelo Rust e são
  repostos a quem abre a conversa depois, com o tempo que resta (`49d39bcaa`, `919cf0476`).
- Contrato interno atual: **22** (`RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL`).
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
8. Instáveis no CI também: `large_chunked_body_is_not_held_by_nagle` (`proxy.rs:359`, Linux e
   local sob carga) e `runtime_open::open_answers_before_initialize` (Windows, 1,506 s contra
   teto de 1,5 s).
9. CI não testa Rust nem o nativo nos PRs (`server.yml` só em push no repositório, `native.yml` só
   compila e só na `main`): o dono pediu os dois em `pull_request` nos três sistemas, com
   `cargo test` no nativo.
10. Do #49: `bastao.py` (`_onde_esta_o_trabalho`) ainda descreve o git do `cwd`, não da worktree
    (muda o que o dono vê; decisão dele).
11. Do #50: Write que sobrescreve com patch acima de 2000 linhas aparece como "arquivo novo"; patch
    descartado sem log nos dois parsers; `editdiff.rs from_patch` esvazia linha sem prefixo que
    começa com multibyte; o teto conta linhas, não bytes.
12. Do #52: célula contada por `chars().count()` (CJK/emoji desalinham); rótulo maior que o
    `Raster` vaza do trilho.
13. `side.rs record_toast`: aviso sem `id`/`timeoutMs` sai da reposição sem log (o ao vivo chega).
14. Compilação na máquina: no máximo 2 `cargo` ao mesmo tempo, `CARGO_BUILD_JOBS=4`, cada uma no
    `target/` da própria worktree (target compartilhado entre checkouts roda binário velho), e
    apagar o `target/` ao terminar: cinco ao mesmo tempo levaram a carga a 134 e encheram o disco.
15. `fix/plugin-send-under-load` entrou em `ba416224a` (envio pelo plugin no modo user espera 30 s
    pelos hooks do `UserPromptSubmit`; CPU do Rust em git/terminal). Ficou: o ~1 núcleo da produção
    não foi reproduzido; 2 de 6 rodadas isoladas não entregaram nada (tudo `deferred` em 0,1 s, nas
    duas versões, causa não achada); `_commit_change` segura `slot.guard` durante o fsync; valor
    velho do git summary sem marca de idade.

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
