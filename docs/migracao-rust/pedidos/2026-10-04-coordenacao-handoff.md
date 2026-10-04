# Passagem da coordenação da migração para Rust (04/10/2026, ~06:40)

Você passa a coordenar a migração do backend do Hangar para Rust no lugar da sessão `Migracao-Rust`
(conta claude-200-1, contexto cheio). Leia este arquivo inteiro, depois `docs/migracao-rust/README.md`.
Responda ao dono (Jefferson) em pt-BR, curto, uma pergunta por vez, opções letradas.

## Onde trabalhar

Pasta `/home/jefferson/pessoal/hangar/.claude/worktrees/hangar-server-parte1`, branch
`hangar-server-parte1` (PR #24), último commit `871aca25`. Use `/usr/bin/git -C <pasta>`.
O `gh` está logado como `jeffer1312` (refeito hoje por código de dispositivo:
`gh auth login -h github.com --git-protocol https --web`, o dono autoriza no celular).

## Sessões vivas e o que cada uma faz

| Sessão | Trabalho | Quando termina, você |
|---|---|---|
| `rust-parte2d-fim` | finaliza a 2D (envio às sessões com terminal pelo Rust) na `hangar-server-parte2d`, com testes reais Haiku num backend isolado (HOME temporário, porta própria, `tmux -L`), conta `~/.claude-02-200` | confere os testes e o CI e manda a `integracao-main` fazer a Etapa 2 |
| `revisao-2b-2c` | revisa e corrige a 2B e a 2C na `revisao-2b-2c` partindo do Python antigo, com testes reais Haiku. Já achou: (1) mensagem enviada com o Claude ocupado fica `deferred` e o drain reusa `call_id` fixos (`prepare:<id>:prepare_prompt`/`dispatch:…`) → policy 500 → mensagem `unknown` marcada entregue e nunca enviada; (2) no caminho Rust ninguém confirma as entradas sem terminal (`apos_entrega` só no adapter Python) → fila trava em 1000; (3) 62 regravações do estado por mensagem | confere e manda a `integracao-main` juntar junto com a 2D |
| `integracao-main` | Etapa 1: juntou #29, #32, #33, #34 na `main` (agora `134d10cb`); está corrigindo #28 e #31 (sem comentar nos PRs — o dono não quer comentários) e juntando. Etapa 2 só sob sua ordem: junta na `integracao-main` a 2D, a `revisao-2b-2c`, a `origin/main` (merge, nunca rebase/force) e o PR #30, roda as suítes e faz push como avanço direto para `hangar-server-parte1` | — |

Todas mandam resultado para `Migracao-Rust`. **Avise cada uma do seu nome** (`hangar-send <sessão>
"agora mande para <seu nome>"`). Pedidos completos em `docs/migracao-rust/pedidos/2026-10-04-*.md`.

## Decisões do dono que valem

- Testes reais sempre com sessões de verdade e **só o modelo Haiku**; backend de teste isolado.
  Nunca a conta claude-200-1 nem a claude-200-3 (deslogada). O teste não pode tocar a fila real:
  `~/.claude-02-200/.hangar-queue` é link para `~/.claude/.hangar-queue` (já vazou uma vez e foi limpo).
  Subir backend de teste exige o conserto `bb80cf31` (órfãos só do próprio dono), senão mata canos reais.
- Regra da migração: toda falha do Rust registrada com código e motivo (diário e
  `hangar-server.log`); falha antes de efeito → 3 tentativas + 1 após pausa e só aquela sessão/parte
  vai para o Python; falha com efeito possível nunca se repete.
- A 2B **não vai para a main** antes dos consertos da revisão. O #24 só vai para a `main` depois do
  uso real; o dono testa pelo canal de testes (`hangar-server-parte1`) "como se fosse a main". Um
  interruptor no "Sobre" sozinho não isola a 2B: ela mudou também o Python (fila, trava, coordenador).
- Contrato interno: 8 = limpeza da fila (já na parte1); parte 3 vira 9; 2D vira 10; o #30 pega o
  próximo livre. Subir `RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL` juntos.
- Notebook e esta máquina ficam como estão até os consertos (dono decidiu); risco conhecido: mensagem
  enviada com o Claude ocupado numa sessão sem terminal pode sumir.
- Push na `hangar-server-parte1`: liberado. Na `main`: só via PRs revisados pela `integracao-main`.

## Notebook do dono (`notebook-jefferson`)

Roda `5ea4e297`. Acesso pela API do peer (token em `/home/jefferson/hangar/backend/peers.json`,
chave `notebook-jefferson`, `base_url` pelo hub):
- versão/atualização: `GET /api/atualizacao`; atualizar: `POST /api/atualizacao/iniciar` (só depois
  do build do `server.yml` da mesma branch publicar o Rust — senão vem Python novo com Rust velho) e
  esperar `versoes.backend` == commit novo;
- diário: `GET /api/diag/arquivo` (teto de 4 MB/dia; o de 03/10 parou às 23:17);
- `hangar-send` para lá com `API_CONNECT_TIMEOUT=15 API_TIMEOUT=150` (o padrão desiste em 4 s);
- a sessão `notebook-jefferson::hangar-3` roda diagnósticos e `./scripts/medir-rust.sh` sob pedido.

Última medição lá (`medir-rust.sh`): abrir o chat 1,1–2,2× mais rápido pelo Rust; chat ao vivo
1,7–2,8×; Custos 117 ms pelo Rust contra 11 ms no Python — o repasse do Rust para respostas grandes
está lento (+20 a +100 ms) e precisa ser investigado antes da parte 3 entrar.

## Pendências fora das sessões

- **Parte 3** (custos e uso, feita pela máquina `casa` do Waldir, par externo encerrado): publicada em
  `hangar-server-parte3` = `1a2bc1b9`; CI falha em macOS (`costs_routes.rs:571`, relatório não
  invalidado após gravar no índice) e Windows (`contract_costs_index.rs:68`, obtido 1 esperado 0 na
  releitura incremental). Falta corrigir (o dono pode reconvidar a casa ou pedir para você), subir o
  contrato para 9 na junção e o teste manual das telas de Custos e Uso.
- PR #33 e minha correção `5ea4e297` resolvem o mesmo bloqueio do tmux de jeitos diferentes; os dois
  convivem.
- Investigar o repasse lento do Rust (Custos).
- `hangar-3` no notebook tem 682 operações `prepared` órfãs que a poda guarda (670 KB, cresce
  devagar): algo na 2B prepara e nunca fecha — conferir se a `revisao-2b-2c` cobre.
- O diário enche os 4 MB/dia com `api.servidor` de sucesso e para de registrar falhas.
- `scripts/medir-rust.ps1` (Windows) não foi testado.
