# Revisão com correção do PR #30 (Git e arquivos no Rust), provada no uso real

O PR #30 (`feat/rust-workspace-io`, fork `WSjunior/hangar`, base `hangar-server-parte1`) foi feito
pelo Codex: move as operações de Git e arquivos do celular para o `hangar-server`, com o crate
`hangar-workspace` compartilhado pelo desktop nativo. CI verde, mas a 2B e a 2C também tinham CI
verde e quebraram no uso real (pid comparado errado, trava segurando disco, estado sem limite, Rust
em erro sem o Python assumir). Revise, corrija pela causa e prove com o backend de verdade.

## Onde trabalhar

Pasta `/home/jefferson/pessoal/hangar/.claude/worktrees/pr30-workspace-io`, branch local
`pr30-workspace-io` = cabeça do PR (`64b68e7c`). Use `git -C <essa pasta>`. Push vai para a branch do
PR no fork (`maintainerCanModify` ligado):
`git push https://github.com/WSjunior/hangar.git pr30-workspace-io:feat/rust-workspace-io`.
Nunca `push --force`: a branch é pública. Não comente no PR (o dono não quer comentários).

## Antes de revisar

1. Leia `docs/migracao-rust/README.md`, `docs/migracao-rust/git-arquivos/` (spec, plano, execução,
   verificação) e o `CLAUDE.md` da raiz, incluindo as regras de "Arquivo citado na conversa" e do
   `hangar-server` em `docs/decisoes/plataforma.md`.
2. Ponha a branch em dia: `git fetch origin` e **merge** de `origin/hangar-server-parte1` (nunca rebase).
3. **Contrato interno:** o PR subiu para 10, que é da 2D. Pela ordem da migração (8 fila, 9 parte 3,
   10 2D, 11 revisão 2B/2C) o #30 fica com **12**, nos dois lados juntos (`RUST_SERVER_PROTOCOL` e
   `INTERNAL_PROTOCOL`).

## O que procurar

Compare com o Python que ele substitui (`git_ops.py`, `filetree.py`, `filesearch.py`, `fs.py`, rotas
em `api.py`, `_resolver_citado()`) e com o `git log` desses arquivos (bugs que já custaram):
- autorização de caminho: `..`, symlink para fora da raiz, `.git` por componente do realpath,
  arquivo citado fora da raiz (GET e POST com a mesma política); token nunca em log;
- escrita: digest da leitura conferido, tmp+rename, nada repetido quando a resposta foi incerta;
- trava ou laço esperando disco/processo; processo `git` que fica vivo depois do prazo; limite de
  concorrência que trava em vez de recusar;
- regra da migração: toda falha do Rust registrada com código e motivo (diário e
  `hangar-server.log`); falha antes de efeito → 3 tentativas + 1 após pausa e só aquela operação vai
  para o Python; falha com efeito possível nunca se repete;
- paridade de formato com o Python (o que o PWA, o app `mobile/` e o nativo leem); Windows
  (`docs/decisoes/windows.md`, regras vigentes);
- teste que só passa com peça falsa (fixture recusando o Python não prova o caminho real).

## Testes reais

- Backend de teste isolado: traga `bb80cf31` da `hangar-server-parte2d` (`git cherry-pick`) se o
  merge não o trouxer — sem ele, subir um segundo backend mata os canos das sessões reais do dono.
  `HOME` temporário, portas livres diferentes de 8765/8766/8768, `CP_AUTH_TOKEN` próprio, tmux próprio
  (`tmux -L <nome>`). Não pode enxergar nem adotar sessão real.
- Use um repositório Git de teste dentro do `HOME` temporário. Pela API do backend de teste (passando
  pelo Rust na porta pública): árvore, leitura, busca, edição com digest certo e errado, arquivo
  citado fora da raiz, diff, histórico, stage/commit/fetch/pull/push contra um remoto local, caminho
  malicioso, arquivo grande, Range. Derrube o `hangar-server` de teste no meio e confira que o Python
  assume sem repetir escrita. Se precisar de sessão Claude para o arquivo citado: **só Haiku**
  (`--model claude-haiku-4-5`), `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`, sem copiar
  credencial. Nunca a `claude-200-1` nem a `claude-200-3`.
- Tela: o PWA da branch servido pelo backend de teste, conferido no navegador em largura de celular
  (abrir arquivo, editar, salvar, painel de git).
- Anote cada caso com resultado e evidência em `docs/migracao-rust/git-arquivos/revisao-real.md`. No
  fim, pare tudo que subiu e apague o `HOME` temporário.

## Como entregar

- Uma correção por commit, com teste que falha sem ela (inglês, `git add` de caminhos explícitos,
  `HANGAR_SEM_PASSO=1`). Revisão independente do conjunto por subagente antes de subir.
- `cargo test --locked --workspace` em `crates/`, `cargo check` do `desktop-native`, pytest dos
  arquivos tocados, `npm run check` na raiz se mexer no front. Falhas conhecidas desta máquina: `omp`
  não instalado; 3 de `test_update_channel` que dependem da ordem.
- Push na branch do PR e CI (`ci.yml` e `server.yml`) verde nos três sistemas (`gh run watch`).
- Mande para `migracao-rust-2` (`hangar-send migracao-rust-2 "…"`): defeitos achados (onde, efeito,
  como o Python resolvia, como ficou), commits, testes com números, casos reais, link do CI.
  Decisão que mude comportamento visto pelo dono → pergunte a `migracao-rust-2` e espere.

## Proibido

Subir, reiniciar ou parar o backend real ou o serviço `hangar-backend`; usar as portas
8765/8766/8768; tocar no tmux padrão ou em sessão real; rodar instaladores; mexer na `main` ou na
`hangar-server-parte1`; juntar o PR (quem junta é a `integracao-main`); log com texto de conversa.
