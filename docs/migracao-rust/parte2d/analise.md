# Parte 2D — entrada do Claude com terminal

Base: `eae31286`, branch `hangar-server-parte2d`. Pedido de execução:
`../pedidos/2026-10-03-parte2d-kickoff.md` na árvore da parte 1.

## O que já existe

- `backend/app/api.py:3750` encaminha `_send_one` ao coordenador. Os gatilhos de
  confirmação/drenagem também consultam `managed_runtime`; o predicado hoje exige sem terminal.
- `runtime_coordinator.py:334` registra posse, `:440` protege fila e `:521` transfere a posse.
  `runtime_adapter.py:210` resolve apenas sidecars do cano. Terminal novo não é registrado.
- `crates/hangar-server/src/runtime/queue.rs` implementa trava entre processos, diário antes
  do efeito, proteção de operações incertas e projeção JSONL. `receipt.rs` distingue ocorrências
  iguais, arquivo, offsets e conversa. Reaproveitar esses dois mecanismos.
- A observação 2C (`terminal_control.rs`, `terminal_state.rs`) é somente leitura; estado temporal
  continua no Python. Não transformar observação em escritor nem publicar outro estado público.

## Esteira a preservar

`api.py:3819` usa socket nativo somente para recados, plugin com long-poll vivo, teclado como
reserva e fila. `terminal_input.py:1631` valida texto, serializa, verifica overlay, espera a TUI,
esvazia composer, cola, confere entrada e submissão. `:1866` e `:1881` fixam allowlists distintas.
`tmux.py` resolve pane do agente, usa CR para Enter POSIX e clipboard no Windows.

Dois resultados atuais não devem ser copiados: `plugin_bridge.py:544–555` ignora o resultado da
limpeza antes de permitir teclado; `uds_messaging.py:210` não distingue falha antes e depois
de começar a escrita. A regra já aprovada exige resultado incerto nesses casos.

## Fronteira

Rust decide transporte, prova, limpeza, fila, confirmação e controles. Python continua resolvendo
identidade da conversa/pane, política de recados e fontes do estado, e hospeda o long-poll do
plugin. A ponte do plugin oferece apenas publicação/aviso; nunca chama envio/Enter/limpeza.
O mesmo coordenador bloqueia caminhos Python quando Rust possui a vida. A reserva conserva o
diário e só assume após liberação confirmada ou morte confirmada do filho.

Não foi feita medição de ganho nem uso real. Não operar sessões reais, serviços ou instaladores.

## Fronteira remanescente do Codex terminal

A afirmação do pedido de que a 2D fecha toda a parte 2 não corresponde à base desta árvore.
Codex com terminal continua no WebSocket/RPC Python: `runtime_coordinator.py:151` e `:410`
selecionam headless; `runtime_adapter.py:225` e `:928` deixam terminal no adapter original;
`adapters/codex/adapter.py:1775` chama `turn/start`, e `appserver.py:309` escreve no WebSocket.
A extensão de descriptor da 2D aceita terminal somente para Claude. Esse envio Codex permanece
fora do escopo desta execução; a entrega da 2D não comprova seu porte nem o fechamento da parte 2.
