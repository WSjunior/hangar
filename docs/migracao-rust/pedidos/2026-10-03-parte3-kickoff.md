# Parte 3 da migração para Rust: custos e uso — analisar, especificar e planejar

Você planeja a parte 3 da migração do backend do Hangar para Rust. Não implementa código nesta
sessão: o resultado é análise, spec e plano para outra sessão executar.

## Onde trabalhar

Pasta `/home/jefferson/pessoal/hangar/.claude/worktrees/hangar-server-parte3`, branch
`hangar-server-parte3`, criada da `hangar-server-parte1` (`2c66a347`, branch do PR #24). Use
`/usr/bin/git -C <essa pasta>` nos comandos de git.

## Leia antes

1. `docs/migracao-rust/README.md` — roteiro, estado das partes, lições (medir antes de afirmar,
   reserva para todo binário novo, log sem texto de conversa).
2. `docs/migracao-rust/analise-inicial.md` — linha "Custos/uso": varredura fria de 5,4 GB = 38 s em
   1 núcleo (61% transformação Python), `/api/uso` sem cache 0,26–1,6 s.
3. `docs/migracao-rust/parte1/spec.md` e `docs/migracao-rust/parte2c/` — como uma parte já foi
   especificada e planejada (contrato interno versionado, paridade provada por fixture dourada,
   reserva no Python).
4. `CLAUDE.md` da raiz, regra "A porta 8765 é do `hangar-server`…" e a entrada em
   `docs/decisoes/plataforma.md` "hangar-server: a porta pública em Rust, o Python atrás".

## Escopo

Custos e uso: `backend/app/costs.py`, `costs_cache.py`, `costs_claude_transcript.py`,
`costs_sources.py`, `session_cost.py`, `uso_claude.py`, `uso_codex.py`, `uso_report.py`,
`uso_areas.py`, `stats.py`, `cotas.py` e as rotas de `api.py` que os usam. Descubra no código quais
são as rotas, quem as chama no front (`frontend/`, `mobile/`, `desktop-native/`) e o que delas pesa.
Fique só com Claude e Codex se Pi/Kimi/omp/opencode complicarem: o dono não usa esses agora; o que
ficar de fora continua no Python.

## O que entregar, em `docs/migracao-rust/parte3/`

1. `analise.md` — medições reais nesta máquina (só leitura: rodar as funções Python num processo
   avulso contra os arquivos reais é ok; nunca o backend vivo). Onde vai o tempo e a memória, o que
   trava o resto do backend, e o ganho esperado em Rust medido com protótipo quando der.
2. `spec.md` — o desenho: quais rotas o Rust atende, como o cache em disco e o respeito a 429
   (regra "Cota tem cache em disco…" do `CLAUDE.md`) continuam valendo, contrato interno e versão
   (`RUST_SERVER_PROTOCOL`/`INTERNAL_PROTOCOL`), reserva no Python, como provar paridade.
   **Mostre a spec ao dono e espere a aprovação dele antes do plano.** Pergunta em texto, uma por
   vez, com opções letradas e a recomendação marcada.
3. `plano.md` — no formato do `superpowers:writing-plans` (`### Task N:` e `- [ ] **Step N: …**`),
   pronto para execução por subagente com revisor por Task.

Pode commitar e dar push só de `docs/migracao-rust/parte3/` na branch `hangar-server-parte3`, com
mensagem descritiva em inglês e `git add` de caminhos explícitos.

## Regras

- Nunca suba, reinicie ou pare o backend nem o serviço `hangar-backend`; nunca suba um segundo
  backend; nunca rode instaladores.
- Não mexa na `main` nem na `hangar-server-parte1`. A parte 2B está em execução na sessão
  `rust-parte2b` (branch `hangar-server-parte2b`, contrato versão 4): se a parte 3 precisar mexer no
  contrato interno, planeje a partir da versão 5 e anote a dependência.
- Ao terminar a spec e ao terminar o plano, avise a sessão `hangar` com `hangar-send hangar "…"`
  (caminho do arquivo e uma linha do que muda).
