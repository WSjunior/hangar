# Finalizar a 2D (envio ao Claude com terminal pelo Rust) com testes reais

A 2D foi interrompida na última etapa. Termine-a: confira os consertos pendentes, prove o
comportamento com sessões reais do Claude usando o modelo Haiku e deixe a branch pronta para o PR.

## Onde trabalhar

Pasta `/home/jefferson/pessoal/hangar/.claude/worktrees/hangar-server-parte2d`, branch
`hangar-server-parte2d`, último commit `1f28144b`. Use `git -C <essa pasta>`. Ela já tem a
`hangar-server-parte1` até `c195a6a4` (limpeza da fila, contrato 8); a 2D usa o contrato **10**.

## Estado deixado pela sessão anterior

- Tasks 1–3 feitas e aprovadas. Task 4: verificação final rodou antes da última correção (335 Rust,
  1446 Python verdes).
- A revisão final sobre `6a94ca12` reprovou 4 problemas; os consertos estão em `1f28144b`, **sem
  releitura**:
  1. uma ocorrência confirmava duas entradas quando o transcript estava inicialmente ausente;
  2. a próxima entrada podia apagar um preenchimento incerto;
  3. timeout deixava um processo escritor descendente vivo antes da transferência;
  4. identidade ausente no início podia bloquear a sessão para sempre.
- `backend/tests/test_runtime_final_fixes.py` não está na lista Python do `.github/workflows/server.yml`;
  `docs/migracao-rust/parte2d/verificacao.md` e o relatório da correção estão desatualizados.

## O que fazer

1. Leia `docs/migracao-rust/README.md`, `docs/migracao-rust/parte2d/` (spec, plano, verificação) e a
   seção 2D de `docs/migracao-rust/parte2-claude-codex/spec.md`. Regra da migração (vale para a 2D):
   toda falha do Rust fica registrada com código e motivo; falha antes de qualquer efeito tenta 3 vezes
   + 1 após pausa e então só aquela sessão/parte vai para o Python; falha que pode ter tido efeito nunca
   se repete (reaproveite `RuntimeCoordinator.op`, `_hand_to_python` e `failure_reason`).
2. Revisão independente (subagente de contexto limpo) dos 4 consertos de `1f28144b`; corrija o que
   for apontado pela causa, com teste que falha sem o conserto.
3. Testes automatizados completos: `cargo test --locked --workspace` em `crates/`,
   `cargo check --locked --target x86_64-pc-windows-gnu -p hangar-server --tests` (se faltar
   compilador C para Windows, registre e deixe para o CI), pytest dos arquivos ligados à 2D. Inclua
   `test_runtime_final_fixes.py` no `server.yml`.
4. **Testes reais, só com o modelo Haiku** (`claude-haiku-4-5`) nas sessões Claude de teste:
   - Backend de teste ISOLADO desta branch: `HOME` temporário (ex.: `/tmp/hangar-2d-real/home`),
     portas livres diferentes de 8765/8766/8768, `CP_AUTH_TOKEN` próprio, servidor tmux próprio
     (`tmux -L hangar-2d-real`). Ele não pode enxergar nem adotar nenhuma sessão real.
   - Conta para o Claude de teste: use `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-claude-200-3`
     direto (não copie credenciais), sempre com `--model claude-haiku-4-5`.
   - Roteiro mínimo, pela API do backend de teste: abrir sessão Claude com terminal; enviar mensagem
     simples e confirmar uma única entrega no transcript; mensagem com várias linhas; mensagem
     enquanto o Claude está ocupado (fila) e confirmação ao ficar ocioso; responder uma opção/pergunta;
     Esc; `/clear`; reiniciar só o backend de teste com mensagem na fila e conferir que nada é
     redigitado; derrubar o `hangar-server` de teste e conferir que o Python assume sem duplicar;
     forçar falha antes de efeito e conferir 3+1 tentativas e a passagem só daquela sessão para o
     Python, com motivo no diário e no log do Rust.
   - Anote cada caso com resultado e evidência (trecho do diário/log, contagem de entregas no
     transcript) em `docs/migracao-rust/parte2d/verificacao.md`.
   - No fim, encerre as sessões de teste, mate o servidor tmux de teste e o backend de teste, apague
     o `HOME` temporário.
5. Commit por etapa (inglês, `git add` de caminhos explícitos, `HANGAR_SEM_PASSO=1`). Com tudo verde,
   push só da `hangar-server-parte2d` e acompanhe o `server.yml` nos três sistemas (`gh run watch`).
6. Mande o resultado para `Migracao-Rust` com `hangar-send Migracao-Rust "…"`: hashes, testes com
   números, casos reais com resultado, link do CI, pendências. Se travar numa decisão que só o dono
   toma, pergunte a `Migracao-Rust` e espere.

## Proibido

- Subir, reiniciar ou parar o backend real ou o serviço `hangar-backend`; usar as portas 8765, 8766
  ou 8768; tocar no tmux padrão do usuário ou em qualquer sessão real; rodar instaladores; mexer na
  `main` ou na `hangar-server-parte1`.
- Modelo diferente do Haiku nas sessões Claude de teste.
- Log com texto de conversa; credencial copiada ou exibida.
