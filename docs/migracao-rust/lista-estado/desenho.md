# Lista de sessões + estado no Rust: desenho

Pedido do dono (05/10/2026, via `migracao-rust-2`): a migração vai até o fim; o critério é
concentrar no Rust. Medição: `medicao.md`. Inventário com arquivo:linha: `inventario.md`. Plano:
`plano.md`. Revisão adversarial (`ecc:architect`) incorporada; ver "Achados da revisão" no fim.
Nada disto foi implementado.

## A regra, em uma frase

Com o Rust de pé, **o Rust é o único dono da descoberta das sessões, da resolução do transcript de
cada uma, da lista (`GET /api/sessions`, `/api/sessions/events`) e, na Fase C, do estado do chat
das sessões Claude com terminal**. O Python vira **fornecedor de fatos** do que ainda é dele
(provedores não migrados, convidados, compartilhamento, navegador, terminais de atalho, plugin,
trocas em curso) e cliente da descoberta do Rust nas rotas que continuam nele. Reserva: só a do
processo inteiro (`pending`/`rust`/`python`); só no modo `python` o código atual roda.

## O que vai para o Rust

| Peça | Hoje (Python) | No Rust |
|---|---|---|
| Descoberta (todos os provedores) | `registry.list` (`registry.py:1299-1497`) | `list::discover`, função pura sobre entradas lidas (processos, panes, arquivos, relógio) e caches; leitores de `/proc` no Linux e `sysinfo` no Windows/macOS |
| Resolução do transcript e o cache dela | `resolve_tracked` (`registry.py:1064-1190`) com `_jsonl_cache`/`_fd_locked`, semeado na criação, troca de modo, transferência, rename e resume (`:2302, 2359, 2448, 2599, 2682, 2786, 2819, 3166`) e consultado fora da lista (`runtime_terminal.py:115`, `plugin_bridge.py:936`, `api.py:4023, 4120, 4257`) | o Rust é dono do cache; o Python chama `list.resolve`, `list.seed`, `list.forget`, `list.rename` pela ponte |
| Decoração independente de provedor | marcadores, registro nativo, `.hangar-askq`, statusline, contexto e modelo, última resposta, planos, loop, `stalled`, git | `list::decorate` (git já no `hangar-workspace`; última resposta pelo histórico Rust) |
| Classificação da lista (Claude com terminal) | marcador primeiro; pane só quando preciso; segunda captura para o spinner; varredura de statusline com teto | `list::classify` com `terminal_state::analyze`, captura avulsa (sessão sem chat aberto não tem cliente `-C`) |
| Claude sem terminal | `hl.snapshot` (lê o `public_state` que o Rust calculou) | direto do `RuntimeRegistry` |
| Produtor e distribuição | `_ListRefresher` (`sse.py:409-555`) | `ListHub`: **um produtor por servidor**, tique de 1,5 s + despertar por arquivo; `GET` serve o retrato; SSE por cliente só lê; produz também sob demanda para quem pede o retrato sem cliente aberto (vigia de travada, `prune`) |
| Estado do chat, Claude com terminal (Fase C) | `StateMonitor` (`state.py:794-1080`) | `state::Monitor`: aluguel da observação em processo, `terminal_state::reduce` deixa de ser só referência, publica `state`, `ask_question`, `suggest` no hub da sessão |

## O que fica no Python, e como entra

**Fatos por pergunta e resposta, não por stream.** A cada produção da lista, o Rust faz
`POST /internal/list/facts` (atrás do `require_internal`) com as linhas que descobriu e a contagem
de clientes do dono; o Python responde com os fatos, prazo de 1 s. Estourou ou falhou: as linhas que
dependem de fato ficam com o último valor e `problema = list_facts_unavailable`; a lista do dono
nunca passa ao Python. Não há dependência circular (as linhas vão no pedido) e funciona sem cliente
aberto.

| Fato | Dono | Na resposta |
|---|---|---|
| Estado, rótulo, pergunta, opções, `problema`, statusline das linhas Codex (com e sem terminal), Pi, omp e Kimi | adaptadores Python (`registry.py:1572-1690`, `1764-1780`) | por sessão, calculados com o código de hoje só sobre essas linhas |
| Transferência em curso | `_decorate_transfers` (`registry.py:165-206`) | **substituição por nome** (`cwd`, `jsonl`, `provider`, `headless`, `lifecycle_id`, `problema`, `transfer_*`) e linhas sintéticas; o Rust não classifica nem drena essas linhas (`:1532-1535`) |
| Linhas `orq` | `orq/runs.py`, `orq_conductor` | linhas prontas |
| `shared`, `owner`, escondida do dono | `share_store`, `guest_users.visible_to` | por sessão |
| Navegador pendente | `nav_vivos` (`sse.py:254-263`, vence em 600 s a cada leitura) | mapa atual; o Rust entrega a cada cliente o que ele ainda não viu; confirmar continua no Python |
| Terminais de atalho | `shortcut_terminals.list_all` | lista (a leitura lenta não segura a resposta; vai a última boa) |
| Presença do app do dono | `plugin_bridge.app_entrou/app_saiu` (`sse.py:593-594, 669-670`), que decide segurar permissão no app (`plugin_bridge.py:1350-1351`) | o pedido leva a contagem; o Python ajusta a presença |
| Fase C: plugin (estado, pergunta segurada, vivo, sugestão), `em_troca` (`backend/app/adapters/claude_headless/sessions.py:88`), transferência ativa, operação de permissão controlada | `plugin_bridge`, `permission_mode` | por sessão Claude com terminal; o plugin também acorda o `Monitor` por um aviso curto do Python à porta privada, sem esperar o tique |

**Serviços que o Rust pede ao Python** (padrão `runtime_policy`): `hooks.demote_awaiting` (o mapa
de `hook_state` e o registro nativo em memória são do Python, `hook_state.py:163-189`, e `preview.py:449`
e `on_transition` leem dele); `permission.observe` (Fase C, `permission_mode.py:90-104`: pergunta
quando a leitura muda **e** enquanto houver operação controlada, de novo quando ela termina);
`session.dead` (Fase C: `plugin_bridge.esquecer` e `forget_frame`, `state.py:897-898`);
`session.deliverable` (Fase C: a borda "voltou a aceitar texto" entra no `adapter.drain` de hoje,
`sse.py:1224-1237`, que passa pelas travas de entrega e de transferência e decide o dono da entrada;
o Rust só avisa).

**Ponte privada Python → Rust** (porta privada já existente, segredo e loopback como
`workspace_bridge.py`): `list.discover` (com "só mais novo que t" e mapa de processos novo, o caso
da sessão criada há menos de 1 s, `api.py:1025-1028`), `list.snapshot`, `list.invalidate`,
`list.resolve`, `list.seed`, `list.forget`, `list.rename`. Erro **levanta**, nunca devolve lista
vazia (a varredura de pares mortos lê vazio como "ninguém vivo" e desfaz pares; o `prune` já se
protege, `prune.py:191-207`, mas perde a rodada); o
multiplexador fora do ar volta como `tmux.MuxIndisponivel`, e o 503 `erro_mux_indisponivel`
(`api.py:574-590`) continua igual. Com o Rust em `pending`, espera até 30 s e falha com código.

**Ficam no Python, lendo do Rust:** vigia de travada (`stall_watch`), `_on_hook_transition`
(`api.py:1500-1559`), `prune`, lista do convidado (filtra o retrato do Rust como hoje), varredura de
pares mortos (sai de dentro da descoberta para um laço próprio de 2 s).

## Estado do chat (Fase C)

- O `Monitor` Rust aluga a observação em processo, lê a cada 0,75 s ou ao aviso do plugin, aplica
  `reduce` (`terminal_state.rs:373-436`) com os fatos, completa com `shells`, loop, overlay, login,
  limite e `dead` (respeitando `em_troca`, `state.py:892-895`), e publica no hub (`side.rs`).
- O `StateMonitor` Python não sobe para essas sessões (`sse.py:730, 1038`): sem isso haveria captura
  dupla, permissão observada duas vezes e gatilho de entrega duplicado.
- Pi, omp, Kimi e Codex seguem com o estado do Python repassado.

## Windows

- Descoberta por `sysinfo` e psmux `list-panes` (`=<sessão>:<janela>.<pane>`, bilhete por
  `PSMUX_SESSION`, `registry.py:739-756`); código de retorno não decide falha sozinho; saída com
  troca de byte inválido.
- Observação `-C` continua desligada (`terminal_control.rs:173`): lista e `Monitor` usam captura
  avulsa pelo psmux, o custo de hoje.
- Sem `/proc/locks` e sem fd aberto: o Rust copia a reserva que o Python já usa (mtime; resolução
  sem fd).

## Como provar

- **Descoberta e classificação como funções puras** sobre entradas gravadas pelo Python
  (`procinfo` e `tmux` trocados por fakes que registram), em **sequências de tiques com relógio
  injetado**: os caches dependem da ordem (`_fd_locked`, `_idle_conferido`, teto de 2 capturas de
  statusline com 20 s, limite 30 s, segunda captura do spinner, colisão de transcript).
- **Rodada em sombra** antes de trocar as rotas: o Rust produz a lista a cada tique sem servir, compara
  a assinatura de cada linha com a do Python e grava no diário só o nome do campo divergente. Nada do
  Rust é entregue; o dono único não é violado.
- `Monitor`: sequências em `contract_terminal.rs` com âncora do plugin, marcador, `em_troca` e `dead`.

### Sombra (Task 15)

`CP_LIST_SHADOW=1` no ambiente do `hangar-server` (`list/shadow.rs`). A cada 1,5 s o Rust produz a
lista sem servir e sem rebaixar marcador; o pedido de fatos vai com `shadow: true`, e o Python não
mexe na presença do app, não reclassifica (o estado de Codex/Pi/omp/Kimi sai da lista que ele
serviu) e devolve a assinatura de cada linha dessa lista (`list_facts.SIG_FIELDS`, os campos do
`_list_sig`). Sem lista do Python com até 3 s (ninguém com ela aberta), o Rust espera 10 s.
Toda rodada comparada conta cada (sessão, campo) divergente; a cada janela de 60 s (o limite do diário) o que divergiu vai
ao diário como `rust.list_shadow_diff`, `sessao` + `codigo` = `campo_N_of_M` (divergiu em N das M rodadas
da janela; `row_missing`/`row_extra` para a linha inteira), nunca o valor. Diferença intermitente
também chega: N baixo é o atraso de até um tique da lista do Python, N perto de M é regra diferente.
No máximo 50 por janela, e `diffs_dropped_K` diz quantas ficaram de fora. Rodada sem comparar não
conta nem zera a contagem, e vai ao diário uma vez por sequência: falha (fatos sem resposta = `facts_unavailable`, erro da produção,
pânico do laço, que recomeça) na hora, como `rust.list_shadow_failed`; motivo esperado
(`python_list_absent`, `mux_refused`/`mux_unparsed`) só depois de 2 min seguidos, como
`rust.list_shadow_blind`. A sombra tem cliente de fatos próprio: não espera a produção de verdade
nem lhe passa uma falha. Ela só faz sentido com o Python dono (antes da Task 16).

**Diferenças aceitas** (fora do diário, cada uma da Task que a criou):

| Diferença | Onde aparece | Motivo |
|---|---|---|
| `mux_refused`/`mux_unparsed` | a rodada inteira | o Python lia a recusa do multiplexador como zero sessões; o Rust levanta (Task 6) |
| `problema = list_capture_failed` | `state`, `problema`, `label` da linha | captura falhou: o Rust fica no marcador sem rebaixar e mostra a falha (Task 12) |
| `problema = list_runtime_unavailable` | `state`, `problema` | runtime sem terminal com erro aparece na linha em vez de sessão parada calada (Task 12) |
| Claude sem terminal com `problema = list_runtime_absent` | `state`, `label`, `question`, `status_line`, `pending_questions`, `problema` | ninguém forneceu o retrato do runtime (`ProduceFacts.headless = None`: a sombra, até o hub da Task 17); o estado sai do marcador e a linha diz. Com retrato fornecido, sessão fora dele está parada (como `hl.snapshot() = None`) e tudo é comparado |
| nome com letra fora do português | a linha casa pelo transcript e pela pasta (os dois conhecidos) e pelo nome do Rust contido no do Python, mesma primeira letra | `sanitize_session_name` do Rust só desfaz os acentos do português; outra letra some (Task 7) |
| `conta` de Kimi/Pi/omp vazia no Rust | `conta` | a descoberta não sabe a credencial; só o fato a preenche (Tasks 7 e 14); preenchida, é comparada |

Diferença nova que o canal de testes mostrar: corrigir, ou entrar aqui com o motivo (Step 34).

## O que não muda

Formato da linha (`SessionInfo`, `models.py:73-210`), eventos, ping de 8 s, `list_error`, `nav`,
`shortcut_terminals`, os quatro estados das telas; convidados, pareamento, compartilhamento, push,
criação/rename/fechamento (parte 6); prévia (parte 4); entrada e entrega (donos atuais).

## Contrato interno

Próximo número livre na junção (hoje 22): ponte `list.*` + fatos + `hooks.demote_awaiting` sobem
uma vez (Fase B); os fatos e serviços da Fase C, outra.

## Achados da revisão (05/10/2026)

Mudaram o desenho: (1) dois resolvedores de transcript divergiriam — o cache passou ao Rust com
`resolve/seed/forget/rename`; (2) gatilho de entrega direto no Rust pularia as travas — virou aviso
ao `adapter.drain`; (3) `em_troca`, `esquecer` do plugin e rebaixamento em memória do registro
nativo — viraram fatos e serviços; (4) presença do app — vai no pedido de fatos; (5) permissão só
na mudança ficaria velha depois de operação controlada — pergunta também enquanto houver uma;
(6) transferência sobrescreve linhas — virou substituição por nome; (7) vencimento do navegador —
o mapa vem a cada pedido, já vencido pelo Python; stream de fatos trocado por pergunta e resposta
(circularidade e leitores sem cliente); erro da ponte levanta, nunca lista vazia; prova por
entradas gravadas, sequências e sombra. Sugestão não adotada como regra, mas virou a pergunta ao
dono: adiar a Fase C para a parte 4, junto da entrada e da prévia.
