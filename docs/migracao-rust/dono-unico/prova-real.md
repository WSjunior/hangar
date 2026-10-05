# Dono único — prova de uso real (Task 11)

Rodada de 04/10/2026 sobre `d6dbdc36` (branch `feat/rust-single-owner`), pelo roteiro
`scripts/prova-dono-unico.py --n 10 --conta-b ~/.claude-jefferson`: backend desta árvore isolado
como unit transiente do systemd de usuário, HOME temporário, portas livres fora de 8765/8766/8768,
`tmux -L` próprio, `matar_orfaos` desligado, respostas do Haiku na conta 02-200 (a
`claude-jefferson` só como destino da troca de conta). Entregas contadas no transcript cru por
marcador único; "religou"/"desligou" são as linhas do cliente Python no log; `runtime.*` é o que o
diário ganhou na janela do caso. Nada de sessão, porta ou tmux reais foi tocado, e a limpeza
apagou unit, tmux, HOME e pastas de transcrição da prova.

A tabela é a da segunda rodada completa. A única diferença desse backend para o commit é uma
linha de log com a pilha do `_registration_failed`, posta para pegar o erro intermitente descrito
abaixo e retirada depois.

| Step | Caso | Resultado | Evidência |
|---|---|---|---|
| 49 | criar e mandar na hora, sem terminal | ok | 10/10 com 1 entrega; erros nenhum; religou 0, desligou 0, runtime.* {} |
| 49 | criar e mandar na hora, com terminal | ok | 10/10 com 1 entrega; erros nenhum; religou 0, desligou 0, runtime.* {} |
| 50 | 3 mensagens durante um turno longo, sem terminal | ok | ocupada ao enviar: sim; entregas [1, 1, 1]; em ordem: sim; status [200, 200, 200]; runtime.* {} |
| 50 | 3 mensagens durante um turno longo, com terminal | ok | ocupada ao enviar: sim; entregas [1, 1, 1]; em ordem: sim; status [200, 200, 200]; runtime.* {} |
| 51 | /clear com o chat aberto, sem terminal | ok | /clear 200; mensagem seguinte: 1 entrega; chat abriu 1x, caiu 0x; runtime.* {} |
| 51 | /clear com o chat aberto, com terminal | ok | /clear 200; mensagem seguinte: 1 entrega; chat abriu 1x, caiu 0x; runtime.* {} |
| 52 | restart com fila, sem terminal | ok | envio 200; depois do restart: 1 entrega; parada 0,17 s, SIGKILL não; runtime.* {events_interrupted[RuntimeError]: 1} |
| 52 | restart com fila, com terminal | ok | envio 200; depois do restart: 1 entrega; parada 0,17 s, SIGKILL não; runtime.* {events_interrupted[RuntimeError]: 1} |
| 52 | restart com fila e cano morto antes da subida | ok | cano morto: sim; envio 200; depois: 1 entrega; parada 0,21 s, SIGKILL não |
| 52 | parada final da unit | ok | 0,6 s, SIGKILL não |
| 53 | uma queda do Rust, sem terminal | ok | Rust novo de pé; envio durante a queda 200, 1 entrega; Python assumiu: não; religou 0 |
| 53 | uma queda do Rust, com terminal | ok | Rust novo de pé; envio durante a queda 200, 1 entrega; Python assumiu: não; religou 0 |
| 53 | três quedas do Rust em 60 s | ok | 2 kills nesta fase em 1,6 s, mais a queda anterior; Python assumiu: sim; retomada sem terminal 1 vez; mensagem depois [1, 1]; duplicadas nenhuma |
| 54 | trava de escrita no estado da fila (`chmod`) | ok | envio travado 400 `erro_envio_falhou`, 0 entrega; destravado 200, 1 entrega; runtime.* {rust_op_failed[queue_io]: 1, send_failed[queue_io]: 1} |
| 54 | Git ocupado com 4 pushes lentos | ok | commit 503 `workspace_busy` em 0,01 s, `Retry-After: 2`; pedidos do commit no Python: 0 |
| 54b | entrega incerta forçada no terminal (`terminal_delivery_unknown`) | **manual pendente** | agente congelado 1 s depois do envio: o Rust adia a entrada em vez de perder a prova do envio; nenhuma faixa; seguinte 1 entrega; nenhuma duplicada |
| 55 | troca de conta, sem terminal | ok | troca 200 em 0,61 s; mensagem seguinte na conta B; chave nova: não; 1 entrega |
| 55 | troca de conta, com terminal | ok | troca 200 em 0,68 s; mensagem seguinte na conta B; chave nova: não; 1 entrega |
| 55 | transferência Claude → Codex | **manual pendente** | o backend isolado não enxerga a conta Codex real sem gravar hooks e skills nela |

`events_interrupted` é o canal de eventos do Rust caindo junto com o processo (restart e quedas):
aviso esperado, nunca passagem de dono.

## O que fica com o dono

- **54b:** forçar `terminal_delivery_unknown` numa sessão com terminal e ver a faixa na tela e a
  operação seguinte reabrir no Rust. Congelar o agente não serve: o Rust adia a entrada.
- **Transferência Claude → Codex:** no app real, concluir sem cliente Python.

## Achados das rodadas

- **Primeira rodada, Step 52 (restart com fila):** as duas linhas falharam com um
  `runtime.reopen_failed[RuntimeError]` na sessão com terminal, 1,3 s depois da subida, sem
  `open_failed` antes (a recusa veio antes do pedido ao Rust). A entrega estava certa (1 vez) e a
  sessão seguiu. Não voltou em seis repetições: quatro do 52 sozinho, uma do 55 seguido do 52 e a
  segunda rodada completa, todas com o log da pilha ligado. Sem a frase, a causa não está provada;
  as candidatas são as recusas do lado Python em `prepare_session`/`_open_slot_in_rust`
  ("fila da sessão em uso no Python", "vínculo gerenciado indisponível", "sessão em
  transferência"), todas por corrida com outra operação no boot. Antes de `82b27772` o mesmo erro
  também era `reopen_failed`: o rebaixamento para aviso só pegava conexão e `_transport_lost`.
- **`POST /internal/runtime/policy` com geração antiga** (pedido do Rust que chega depois do
  `/clear`) responde 500 com a pilha no log (`internal_api.py:92`, "serviço de outra posse ou
  geração"). O Rust trata qualquer não-2xx como `policy_refused`, e o teste
  `test_native_message_still_needs_its_journal_attempt` fixa o 500: é ruído de log, não falha de
  entrega. Fica anotado para uma resposta 409 sem pilha.
