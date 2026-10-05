# Envio pelo plugin sai "resultado incerto" com a máquina carregada, e o Rust gasta 1 núcleo

## O que aconteceu (05/10, 04:52–04:56, esta máquina, `hangar-server-parte1` `5974a283`)

- Dois envios do dono pelo app para a sessão `migracao-rust-2` (Claude com terminal, plugin
  `hangar`) mostraram "não deu pra enviar: resultado incerto; entrada conservada sem reenvio" — mas
  **as duas mensagens chegaram** ao Claude ("Prompt from the hangar plugin" no terminal, e a sessão
  respondeu). O texto ficou no campo: o dono pode reenviar e duplicar.
- No mesmo minuto o hook `skill-suggester.py` estourou 5 s, e o `hangar-server.log` tem vários
  `consulta de Git sem resposta code="git_summary_failed"`.
- Carga média 109 em 16 núcleos (compilações Rust de 5 sessões em paralelo). O `hangar-server` usou
  ~1 núcleo o tempo todo (ps: 99,5% ao longo de 15 min de vida; `top -H`: várias threads
  `tokio-runtime` de 5–15% e a thread `notify` ~10%). Diário: `POST /api/plugin/ui` a cada ~200 ms.

## O que fazer

1. **Envio incerto que chegou**: ache no código (plugin → `hangar-server`/runtime do terminal →
   recibo; `plugin_uncertain`, `terminal_delivery_unknown`, prazos de confirmação) por que a entrega
   vira incerta sob carga. Conserte pela causa, sem relaxar a regra de nunca repetir efeito incerto:
   - o prazo de confirmação não pode ser tão curto que carga normal de máquina ocupada o estoure;
   - quando a mensagem aparece no transcript depois, a entrada incerta tem que ser reconciliada
     como entregue (uma vez) e o app tem que limpar o aviso e o campo — hoje o dono vê erro de algo
     que deu certo e é convidado a reenviar.
   Teste que falha sem cada conserto (inclusive com atraso artificial na confirmação).
2. **Consumo do Rust**: meça com evidência (sem reiniciar o backend real; `perf top -p`/`perf record`
   por poucos segundos no processo é aceitável, só leitura) o que consome ~1 núcleo: rajada de
   `/api/plugin/ui`, SSE, observador de terminal, leitura de transcript, `notify`. Reproduza num
   backend de teste isolado com várias sessões e carga sintética. Corrija o que for desperdício
   (laço que não dorme, releitura inteira, evento duplicado, publicação sem mudança).
3. **`git_summary_failed` sob carga**: veja se o prazo e o teto de simultâneos do Git no Rust são
   adequados com a máquina ocupada (o painel não pode ficar sem dados por carga normal).
4. Prova: backend isolado (`HOME` temporário, portas fora de 8765/8766/8768, `tmux -L` próprio,
   sessões só Haiku, `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`), com carga artificial
   (ex.: `stress-ng` ou compilação em paralelo): envio pelo plugin confirma uma vez e não mostra
   incerto; medição de CPU do `hangar-server` antes/depois. `cargo test --locked --workspace`,
   pytest dos tocados, `npm run check` se mexer no front; revisão por subagente; CI do `server.yml`
   job por job. **Limite de compilação: no máximo 2 builds na máquina, `CARGO_BUILD_JOBS=4`.**

## Entrega

Branch `fix/plugin-send-under-load` (este cwd, de `origin/hangar-server-parte1`). Commits em inglês,
`git add` explícito, `HANGAR_SEM_PASSO=1`. Push só desta branch. Avise a sessão `organiza-prs`
(que coordena agora) com causa, conserto, números e CI; ela junta na `hangar-server-parte1`.
Proibido: reiniciar/parar o backend real, portas 8765/8766/8768, tmux padrão, sessão real,
instaladores, `push --force`, texto de conversa em log.
