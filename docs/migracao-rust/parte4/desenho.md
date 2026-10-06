# Parte 4: estado, prévia e terminal real no Rust — desenho

Pedido: `../pedidos/2026-10-06-parte4-planejamento-kickoff.md`. Inventário com arquivo:linha:
`inventario.md`. Parte da Fase C de `../lista-estado/` (Tasks 19–24), refeita sobre o código de
`0eb41ec58`. Revisão adversarial (`ecc:architect`) incorporada; ver "Achados da revisão" no fim.
Nada disto foi implementado.

## A regra, em uma frase

Com o Rust de pé, **o Rust é o único dono do estado ao vivo, da prévia, da pergunta nativa, da
sugestão e do `problema` das sessões Claude com terminal, e de todo PTY do terminal real** — não
importa por qual porta o cliente entrou (8765, convite 8766, Connect 8768). O Python vira
fornecedor de fatos do que ainda é dele (plugin, trocas, operação de permissão controlada,
presença do app) e porteiro de quem entra pelas portas dele. Falha vira erro com código, nunca
passagem ao Python; só a reserva do processo inteiro (`pending`/`rust`/`python`) roda o código
atual.

## Fora desta parte

- Pi, omp e Kimi com terminal: o estado segue no `StateMonitor` Python (provedores não migrados,
  parte 5). Codex com e sem terminal: estado vem do app-server Python (parte 5).
- Fatos de entrada do executor (`runtime_terminal.facts`, com captura própria, socket nativo,
  clipboard): envio, parte 5.
- `/clear` logo depois de interromper um AskUserQuestion (item 16 do handoff): caminho de
  entrada, parte 5. Fica anotado.

## Estado ao vivo: `state::Monitor`

**Vida.** Um `Monitor` por sessão Claude com terminal enquanto o hub da sessão (`side.rs`) tiver
assinante — o `/events` do dono na 8765 ou o canal privado que o Python abre para quem entrou
pelas portas dele (ver "Clientes que entram pelo Python"). Nenhum `Monitor` é criado em produção
antes da Task da troca de dono.

**Época.** A troca de época vem do `rebind` do hub (`side.rs:248`), sob a trava dele: no `/clear`
ou na troca do filho, o `Monitor` zera quadro, chave de repetição e memória temporal e publica o
estado novo depois do `rebind` (que apaga o último estado guardado, `side.rs:263`). A saída do
`Monitor` passa pelo `SideCache::record` e pela regra de repetido do hub (`side.rs:105-119,
431-441`), para a pergunta guardada no retrato não se perder.

**Pane do agente.** A captura mira o pane do agente como hoje
(`agentpane.resolve_target`, cache de 60 s, `agentpane.py:95-125`), portado sobre a escolha de
pane que a lista já faz (`Pane::target`).

**Uma captura por rodada, dois consumidores.** O `Monitor` pega o quadro do `TerminalPool` em
processo (sem HTTP) e o entrega ao redutor de estado e à prévia. Rodada de estado a cada 0,75 s
ou quando o plugin acorda; entre elas, capturas de 0,15 s só enquanto a prévia depende do pane
(sessão `working` sem arquivo do hook) e só para a prévia. O redutor temporal conta exatamente
as rodadas que o Python conta hoje (as de 0,75 s **e** as acordadas pelo plugin,
`state.py:1074-1078`), para as sequências gravadas pelo Python valerem como prova. Sem
assinante, nenhuma captura.

**Falha da observação.** O pool devolve a mesma falha guardada até a nova tentativa
(`terminal_control.rs:181-183, 381-387`). O `Monitor` repete o último evento com
`problema=terminal_observacao_falhou` e só confere `has-session` ao entrar em falha e no ritmo
da nova tentativa do pool, não a cada rodada.

**`reduce` deixa de ser referência.** `terminal_state::reduce` vira o redutor de produção e ganha
o que só o Python tinha: `retired`, chave de repetição (modo de permissão, `previous_non_plan`,
pids dos shells, `problema` do runtime), log de divergência do plugin (só código e nome da
sessão). Completam o evento: statusline (sidecar `facts_files.rs:538`, pane de reserva),
overlay/login/limite (`analyze`), loop (`list/links.rs`), shells (porte de
`procinfo.shells_de` sobre `list/procs.rs`; `sysinfo` fora do Linux), modo de permissão (porte
de `parse_permission_mode`).

**Fatos do Python por empurrão, com idades.** O Python manda `state.facts` pela ponte privada
(a mesma porta do `list.*`) só para sessões com interesse registrado. Conteúdo por sessão:
estado do plugin com idade em ms, long-poll aberto (`waiter_aberto`), idade da última batida,
pergunta segurada com idade do último `visto`, sugestão, largura da prévia (`bodyColumns`),
âncora da faixa, `em_troca`, operação de permissão controlada em curso. O Rust aplica com o
próprio relógio as validades do Python: `vivo` = long-poll aberto ou batida com menos de 35 s
(`plugin_bridge.py:818-824`); estado do plugin vale 90 s, sem prazo com long-poll aberto
(`:843-853`); pergunta segurada vence 35 s depois do último `visto` (`:1290`). Quando enviar:
quando um desses valores muda, no início e no fim de cada long-poll e a cada `/ask` (no máximo um
por 25 s por sessão). Cada envio leva sequência por sessão; sequência menor que a última é
descartada; o envio acorda o `Monitor` (idade 0, como `esperar_evento`).

**Interesse e retrato.** O `Monitor` lê o retrato inteiro ao nascer
(`GET /internal/sessions/{name}/state-facts`), o que registra o interesse; relê quando a
sequência pula e a cada 30 s, o que renova o interesse (vence sem renovação). Retrato que falha
vira `problema=state_facts_unavailable` com o último valor, nunca estado inventado. Envio que
falha vai ao diário do Python uma vez por queda do Rust, não uma por envio.

**Serviços que o Rust pede ao Python** (padrão `runtime_policy`, com prazo e código de erro):
- `permission.observe(sid, name, mode)` → `(mode, previous_non_plan)`: a memória é do Python, por
  session-id (`state.py:920-924`, `api.py:9179-9181`), e o evento usa o que ele devolve. Pergunta
  quando a leitura difere do último modo devolvido, a cada rodada durante a operação controlada
  e uma vez quando ela acaba.
- `session.dead(name)` → `ok` | `em_troca`: o Python confere o `em_troca` na hora da chamada
  (hoje é lido na decisão, `state.py:891-899`) e só com `ok` faz `esquecer` + `forget_frame`; o
  Rust só publica `dead` com `ok`. Sem isso a troca de conta mataria o tmux antes de o fato
  chegar e apagaria o plugin de uma sessão em transferência.
- `session.deliverable(name)`: ver Entrega.

**Rebaixamento do registro nativo.** Hoje fica na memória do Python (`hook_state.py:171-176`) e
a próxima escrita do arquivo o desfaz (`:126`). Quem decide rebaixar já é o Rust (lista,
`facts.rs:188`); o mapa de rebaixados passa ao Rust, compartilhado por lista e `Monitor`,
invalidado pela versão do arquivo (`statusUpdatedAt`). O aviso ao Python
(`/internal/list/demote`) continua para `stall_watch` e `_on_hook_transition`.

## Prévia

- 1ª fonte: arquivo do hook MessageDisplay (`.hangar-preview/<stem>.json`). O `Monitor` confere o
  mtime na própria rodada (0,15 s trabalhando) e lê só quando mudou; arquivo com mais de 600 s
  vale se o marcador diz `working` (`preview.py:448-458`). Não entra no observador de arquivos da
  lista (ele só vive com a lista aberta e reclassificaria a cada pedaço da resposta).
- 2ª fonte: o mesmo quadro do estado, cortado na largura `bodyColumns+5` e na âncora da faixa
  **antes** da análise (hoje o corte invalida a análise do Rust e o Python refaz).
- Só publica quando muda; não publica o que já está no transcript e limpa a prévia quando o bloco
  entra nele (porte de `sse.py:54-72, 852-855`; o hub já tem o `FileTail`); vaga única; campos
  `text, md, full, vivo`. Acúmulo de 150 ms só na fonte arquivo; na fonte pane a amostragem de
  0,15 s já agrupa.

## Pergunta nativa, sugestão, `problema`, entrega

- `ask_question`: lê `.hangar-askq` (`facts_files.rs`), casa com o menu do pane (porte de
  `sse.py:75-184`), uma vez por pergunta, zera no `/clear` e na troca de provider. Limpar o
  arquivo ao responder continua no Python (`/answer`).
- `suggest`: vem do fato do plugin, sai quando muda.
- `problema` do runtime: lido do `view` do ator em processo, com o mapa `_STALLED_INPUT`
  portado, na chave de repetição; eventos do ator acordam o `Monitor`. Acaba a volta
  Rust → Python → Rust.
- **Entrega:** na borda "voltou a aceitar texto" (`idle`, composer pronto, sem pergunta) o Rust
  chama `session.deliverable(name)`, uma chamada por borda e uma ao nascer o `Monitor` (hoje é uma
  por conexão). O Python roda o `adapter.drain` de hoje, que passa por `prepare_session` e abre o
  executor se ele não existe (`runtime_terminal.py:840-845`), caso de depois de um reinício do
  Rust. Chamar o ator direto perderia isso.
- Códigos novos de `problema` (`state_facts_unavailable`, falha do `observe`) entram nos três
  clientes (`lib/problema.ts`, `SessionProblem.tsx`, i18n do nativo).

## Clientes que entram pelo Python

Quem não é dono na 8765 é repassado ao Python (`routes.rs:378-381`), e as portas 8766 (convite) e
8768 (Connect, onde o dono também entra, `main.py:162-174`) são do Python. No modo `rust`:
- **Chat:** o `merged_events` de Claude com terminal **não sobe** `StateMonitor` nem
  `PreviewBroker`; lê `state`, `preview`, `ask_question` e `suggest` do hub por um canal privado
  (`GET /__hangar_server/state/{name}/events`, segredo e loopback), que conta como assinante. Sem
  isso a sessão teria dois donos com um convidado olhando: captura dupla, `esquecer` e
  `permission.observe` duas vezes.
- **Terminal:** toda conexão que chega ao `termsock` (convidado de convite, convidado com login,
  dono pelo Connect) passa pela porta de entrada de hoje (`_porta_de_entrada`, `share_gate.watch`,
  4410) e liga os bytes a `/__hangar_server/term` na porta privada. O Python não abre PTY.

## A troca de dono

O Python para de produzir `state`, `preview`, `ask_question`, `suggest` e o gatilho de entrega
de Claude com terminal **na mesma Task** em que o hub passa a publicar os do Rust e o
`merged_events` passa a ler o canal privado; antes disso o `Monitor` é testado só por sequências
gravadas, sem existir em produção. No modo interno (`side=True`) de Claude com terminal o
`side-events` continua mandando `info` (dispara o `rebind`), `message`/`queue_confirmed`, `nav`,
`ping` (o hub reconecta depois de 30 s calado, `side.rs:78`), `plugin_toast`, `plugin_ui`,
`stats`, `pensamento`/`ferramenta`; o `tail_pump` desse modo é desligado (só servia à supressão
da prévia, `sse.py:834-869`). Se o Python mandar um dos quatro eventos para uma sessão do Rust,
o hub descarta e registra uma vez por sessão (a troca vazou). Nada de rodada em sombra.

## Lista e o cartão de permissão

- Com `Monitor` vivo, a classificação da lista lê o estado dele em vez de capturar de novo:
  lista e chat dizem a mesma coisa e a captura não dobra. Sem `Monitor`, segue a classificação
  atual (marcador primeiro, captura avulsa).
- Cartão de permissão do Claude com terminal (lista `working` com o marcador preso depois do
  `Bash`; item 5 do handoff): reproduzir com sessão real, provar a causa, corrigir no Rust (lista
  e `Monitor`) com teste que falha antes.

## Terminal real

- **No Rust:** `WS /api/sessions/{name}/term` (`?shortcut=<id>`) e
  `WS /api/hangar-terminals/{ident}/term` na 8765 para o dono, e `/__hangar_server/term` na porta
  privada para o que o Python liga. Mesmo protocolo (binário cru; texto `{"t":"resize"}` com o
  mesmo clamp), mesmos códigos de fechamento, `tmux attach -t ={name}:` com o mesmo ambiente.
  Servidor WS pela feature `ws` do axum já instalado. Ping a cada 20 s; sem resposta em 20 s,
  fecha e desmonta (o padrão do uvicorn hoje; sem isso, celular que perdeu a rede segura o painel
  e o tamanho da janela).
- **Porta de entrada do dono:** só `?token=` nas rotas de terminal, como o Python
  (`termsock.py:373-379`); `Bearer` e cookie do `auth.rs` não abrem shell. IP bloqueado e falha
  registrada pelo `auth.rs`. Origin perguntada ao Python uma vez por conexão
  (`POST /internal/term/origin` com Origin e o `Host` original, prazo 1 s; a regra e as fontes —
  `public_url`, `CP_TERM_ORIGINS`, `term_origins`, `peers.json` — continuam lá); falha da pergunta
  recusa com código no diário. Alvo resolvido no Rust: `has-session`, e atalho por porte de
  `shortcut_terminals.find`/`find_hangar` (um `list-sessions -F` por abertura).
- **PTY:** `portable-pty` 0.9. Leitor bloqueante numa thread por painel, blocos de 64 KiB num
  canal com limite de 1 MiB (cheio = o leitor espera, a contrapressão de hoje); quadros sempre
  abaixo de 1 MiB (limite do cliente nativo). Teto de 64 painéis contando as threads vivas;
  acima, recusa 1013 com código.
- **Desmontar igual ao Python:** `detach-client -t <tty>` (nunca `-s`), SIGHUP, espera de até
  3 s, depois SIGKILL no cliente; espera o tty sair do `list-clients` e repõe o tamanho; árvore
  pelo `terminal_process.rs`. Ao anexar, o tamanho original fica numa opção da sessão tmux
  (`@hangar_term_size`); ao subir, o Rust repõe o tamanho das sessões que têm a opção e nenhum
  painel (queda do Rust no meio de um painel).
- **Um painel por sessão**, no Rust, para todas as portas: a nova conexão espera o escritor
  antigo, fecha a antiga com 1000 "outra conexao assumiu" e desmonta.
- **O 409:** `_recusa_se_painel_aberto` pergunta ao Rust (`term.active`, pela ponte) nas rotas
  raras que já o chamam; erro da ponte é 503 com código.
- **Capacidade:** a saúde do `hangar-server` passa a dizer `terminal_panel`; `/api/config` e
  `/run-code` leem esse valor no modo `rust`.

## Windows

- Estado: o mesmo `Monitor` com captura avulsa pelo psmux (`-C` segue desligado,
  `terminal_control.rs:173`). A captura lê a saída, não só o código de retorno; "não existe" se
  separa de "falhou" por `has-session`; quadro com U+FFFD não vale como bom. A mesma fonte serve a
  lista (`classify.rs:407-408` hoje decide pelo código).
- Terminal real: `portable-pty` cria o ConPTY com `INHERIT_CURSOR | RESIZE_QUIRK |
  WIN32_INPUT_MODE` fixos (`pseudocon.rs:85-87`); o Python usa 0. Com `INHERIT_CURSOR` o console
  pede a posição do cursor (`ESC[6n`) e espera resposta do cliente. Antes de juntar, prova na VM
  com web e nativo; se travar ou sujar a tela, troca o lado Windows por um ConPTY próprio (porte
  de `conpty.py` em `windows-sys`, flags 0) na mesma Task. Matar o filho antes do
  `ClosePseudoConsole` é ordem nossa, não do `Drop` do crate.

## Contrato interno

Sobe duas vezes, cada uma no próximo número livre na junção (hoje 27): fatos, serviços,
`state-facts` e canal privado do estado; ponte `term.*`, `/internal/term/origin`,
`/__hangar_server/term` e `terminal_panel` na saúde.

## Desempenho (conferido contra "erros que já custaram")

| Item | Como fica |
|---|---|
| Laço apertado | captura só com assinante; 0,75 s, 0,15 s só com a prévia dependendo do pane; `has-session` só ao entrar em falha e na nova tentativa |
| Chamar o Python repetido | fatos por empurrão (mudança, long-poll, `/ask` limitado a 1 por 25 s); retrato ao nascer, no pulo de sequência ou a cada 30 s; `observe` na diferença ou durante operação controlada; `dead` e `deliverable` por borda; Origin por conexão |
| Republicar o que não mudou | chave de repetição no estado e na prévia, `SideCache` do hub |
| fsync e leitura que grava | nenhuma escrita no caminho do estado; `@hangar_term_size` só ao anexar e desmontar |
| Processo filho | `-C` em processo; avulso só no Windows (como hoje); `list-sessions` só ao abrir atalho |
| Memória | quadro por `Arc` entre estado, prévia e lista; canal do PTY com limite; teto de painéis |
| Medir | release, backend isolado: CPU por chat aberto (Python hoje: 1,26 ms por captura, ~1,3/s), 1/5/20 chats, parado e trabalhando; latência marcador → `state`; PTY: MB/s, eco de tecla, CPU por MB, RSS por painel |

## Como provar

- Sequências gravadas pelo Python (`gen_terminal.py` estendido com `dead`/`em_troca`, falha da
  observação, permissão com operação controlada, shells, loop, chave de repetição, acordar pelo
  plugin, idades dos fatos) repetidas no `Monitor` com relógio injetado; golden de prévia e de
  pergunta do mesmo jeito.
- PTY: testes com tmux `-L` próprio (eco, resize, duas conexões, desmontagem, contrapressão,
  ping sem resposta, tamanho reposto depois de queda).
- Uso real em backend isolado: Claude Haiku com e sem terminal, Codex `gpt-6-luna` com e sem
  terminal **fora do YOLO** (cartão de aprovação), cartão de permissão do Claude com terminal,
  pergunta nativa, prévia, `/clear`, sessão morta, troca de conta (`em_troca`), convidado olhando
  o chat, terminal no web, PWA e nativo, convidado e Connect no terminal, atalho, 409, reserva
  `CP_RUST_SERVER=0` e 3 quedas do Rust. VM Windows com o dono.

## Regras que ficam falsas

`harnesses.md` "Observação terminal tem uma captura canônica por rodada" (o estado temporal sai do
Python), `CLAUDE.md` "Observação terminal Rust" e "Terminal real no rodapé e no celular",
`plataforma.md` (entrada da observação), `frontend.md` (terminal real: motores e capacidade),
`superado.md` (`StateMonitor` e `PreviewBroker` de Claude com terminal, `termsock` como dono do
PTY). Cada Task corrige a que tornar falsa, no mesmo commit.

## Achados da revisão (06/10/2026)

Mudaram o desenho: (1) convidado e Connect subiam `StateMonitor` e `PreviewBroker` no Python —
viraram leitores do hub por canal privado; (2) o dono pelo Connect abriria PTY no Python — toda
conexão que chega ao Python liga ao PTY do Rust; (3) fatos do plugin vencem com o tempo — vão
com idades, e o envio acompanha long-poll e `/ask`; (4) `dead` contra `em_troca` — `dead` virou
pergunta ao Python; (5) `permission.observe` — chave por session-id, o Python devolve
`previous_non_plan`, pergunta também quando a operação acaba; (6) entrega direto no ator perdia o
`prepare_session` — voltou a ser serviço; (7) prévia pelo observador da lista — virou mtime na
rodada; (8) `/clear` — época pelo `rebind` do hub e saída pelo `SideCache`; (9) pane do agente
com cache; (10) `has-session` por rodada em falha; (11) ping do WebSocket; (12) queda do Rust com
painel aberto — tamanho guardado no tmux e SIGKILL depois de 3 s; (13) só `?token=` no terminal;
(14) interesse por sessão nos fatos; (15) limpeza da prévia e `problema` na chave; (16) contagem
de rodadas igual ao Python; (17) o que o `side-events` mantém; (18) nenhum `Monitor` antes da
troca; (19) rebaixamento invalidado pela versão do arquivo.
