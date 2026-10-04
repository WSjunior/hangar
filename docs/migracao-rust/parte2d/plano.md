# Parte 2D — plano de implementação

> Execução por `superpowers:executing-plans`, com delegação de unidades e revisão independente
> por Task exigidas pelo pedido. Autorizado executar sem nova aprovação.

**Objetivo:** transferir a esteira Claude terminal inteira ao Rust, conservando fila e prova.
**Arquitetura:** executor terminal serializado reutiliza fila/recibos 2B; observador 2C permanece
somente leitura. Python fornece identidade/estado/política e publicação primitiva do plugin.
**Stack:** Rust/Tokio/serde já instalados; Python/FastAPI existente; nenhuma dependência nova.
**Spec:** `docs/migracao-rust/parte2d/spec.md`.

## Restrições globais

- Contrato interno 10 na junção da fila v2, Python e Rust juntos; não tocar custos/uso.
- Identificadores novos em inglês; textos/comentários em português; sem conteúdo em logs.
- Nenhuma sessão real/serviço/instalador; tmux de teste obrigatoriamente `-L` próprio.
- Testes focados por Task, suíte inteira Rust somente ao final; caminhos explícitos no stage.
- Não duplicar fila, observador ou estado público; `unknown` nunca autoriza nova digitação.

## Foco da revisão

- Plugin sem aviso após preencher não autoriza tmux nem Enter sem prova.
- Socket conectado com escrita parcial não autoriza outro transporte.
- Mesmo nome com outro pane/conversa/geração não recebe efeito antigo.
- Textos iguais/anexos requerem ocorrências distintas; working não reenvia.
- Timeout/cancelamento não solta posse com subprocesso/escrita ainda em curso.

### Task 1: Transportes e prova de entrega

**Arquivos:** criar `crates/hangar-server/src/terminal_input.rs` e testes; exportar em `lib.rs`.
**Interfaces:** `TerminalBinding` tipado (nome, pane, conversa, nascimento, comando mux),
`TerminalDriver` (capture/key/text/prompt/select/answer/steer), `DeliveryResult`
(disposição, estágio, limpeza, nativo) e serviço assíncrono primitivo de plugin/fatos.
Driver sem fila; executor da Task 2 fornece serviços registrados antes do efeito.

- [x] **Step 1: Escrever testes de prova e transportes**

Cobrir composer/região/whitespace, novo placeholder de texto/imagem, texto curto, literal CR,
multiline, overlay, falha parcial, Enter incerto, limpeza somente própria, allowlists e seleção.
Teste com IO falso deve afirmar zero redigitação após resultado incerto. Bloquear operação na
serialização, mudar vínculo e liberar: zero publicação/tecla antiga, em envio e controle.

- [x] **Step 2: Conferir falha antes do código**

Rodar `cargo test --locked -p hangar-server terminal_input` em `crates/`; registrar falha esperada.

- [x] **Step 3: Implementar driver completo**

Executar socket/plugin/tmux na ordem da spec; adaptar primitivas do multiplexador à plataforma.
Escrita/timeout depois do efeito produz incerteza; não introduzir driver pela tela do Codex.

- [x] **Step 4: Conferir testes focados e revisar diff independente**

Mesmo comando, esperado verde. Revisor lê spec e diff inteiro da Task; corrigir problemas relevantes.

- [x] **Step 5: Marcar progresso e commitar Task 1**

Stage explícito; mensagem `feat(server): add Claude terminal delivery driver`.

### Task 2: Executor Rust, fila e recuperação

**Arquivos:** criar `runtime/terminal.rs`; integrar `runtime/gateway.rs`, `runtime/mod.rs`,
`runtime/actor.rs` somente interfaces necessárias, `runtime/receipt.rs` se necessário.
`backend/app/runtime_receipt.py` e seus testes acompanham o cursor do primeiro transcript e a
identidade da conversa, para a reserva Python consumir as provas novas sem perder confirmação.
`backend/app/rust_server.py` e `crates/hangar-server/src/lib.rs` fixam protocolo 9 neste commit,
junto da primeira mudança do descriptor privado.
**Interfaces:** adotar descriptor terminal via gateway existente; submit/control/queue/snapshot/
drain/confirm/detach com mesmos envelopes e fila 2B. Serviços primitivos ficam registrados no
diário e usam `PolicyClient`. Reutilizar lease/Store/QueueActor/ReceiptIndex existentes.

- [x] **Step 1: Escrever testes de exclusividade e diário**

Adotar terminal sem cano; repetir operação não escreve novamente; troca de geração é recusada;
duas entradas iguais confirmam em ocorrências distintas; unknown sobrevive a restart; idle drena
sem SSE; claim limita uma entrada; lease permanece até terminar operações em voo.

- [x] **Step 2: Conferir falha antes do código**

Rodar `cargo test --locked -p hangar-server terminal_runtime`; esperado falha por runtime ausente.

- [x] **Step 3: Implementar executor e gateway**

Serializar o driver da Task 1, gravar intento/cursor/dispatch antes de efeitos e finish depois;
usar confirmação incremental com identificação da conversa; timer não redigita incerto.
Não publicar segunda fonte de estado/prévia; controles seguem allowlists e posse.

- [x] **Step 4: Conferir testes focados e revisão independente**

Rodar testes de runtime terminal, fila, recibos e gateway; esperado verde.

- [x] **Step 5: Marcar progresso e commitar Task 2**

Stage explícito; mensagem `feat(server): own Claude terminal delivery and queue`.

### Task 3: Transferência Python, todos os gatilhos e contrato terminal

**Arquivos:** `backend/app/runtime_coordinator.py`, `runtime_adapter.py`, `runtime_policy.py`,
`internal_api.py`, `terminal_input.py`, `api.py`, `plugin_bridge.py`,
`plugins/hangar/hooks/input.ts`, `ask.ts` e `perm.ts`, adapter Claude terminal e testes tocados;
Versão 9 já aplicada junto do contrato privado na Task 2.
`backend/app/runtime_process.py` e `rust_server.py` contêm e encerram os descendentes do Rust,
para provar fim dos escritores antes de liberar a reserva ou readotar a vida após morte abrupta.
**Interfaces:** Binding terminal resolve pane/conversa registrados; serviço de fatos lê estado;
serviço plugin publica/aguarda aviso sem dirigir terminal; fachada encaminha todas as ações.

- [x] **Step 1: Escrever testes dos caminhos públicos/reserva**

Novo terminal gerenciado sem cano; preparar/adotar; envio comum/drain/confirm/teclas seguem Rust;
plugin primitivo nunca tecla; geração antiga não toca plugin; queda confirmada restaura lease;
Rust vivo sem resposta não libera Python; slash/clear e estado público preservados. Aviso plugin
com publicação/geração antiga (ou sem correlação) não confirma nova operação. Convidado permitido
segue mesmo dono; share/par somente leitura recusa escrita com zero efeito. Filho mux/PowerShell
bloqueado e pai Rust morto: nenhum novo escritor antes de provar término dos descendentes próprios
(grupo POSIX/Job Windows); canos e sessões reais preservados.

- [x] **Step 2: Conferir falha antes do código**

Rodar pytest dos arquivos novos/tocados num comando; esperado falha pelos caminhos ausentes.

- [x] **Step 3: Integrar transferência e reserva completa**

Remover seleção exclusiva headless somente para Claude terminal. Registrar identidade durável,
conferir após locks, encaminhar envio e todos os controles, bloquear escritores antigos. Reserva
opera mesmo diário e preserva incerto. Conservar os dois números de protocolo em 9.
Aplicar a regra de falha recebida em `74177469`: quatro tentativas apenas antes de efeito,
pausa antes da última, diário com código/motivo e reserva somente da sessão afetada; incerteza
nunca repete. Integrar a parte1 após concluir esta Task, incluindo a correção de PID `fac8d377`.

- [x] **Step 4: Conferir testes focados e revisão independente**

Rodar pytest dos arquivos tocados, testes Rust do contrato e exercício isolado de gateway/CLI.
Esperado verde; uso real explicitamente adiado conforme pedido.

- [x] **Step 5: Marcar progresso e commitar Task 3**

Stage explícito; mensagem `feat(runtime): route Claude terminal actions through Rust`.

### Task 4: Verificação final e publicação

**Arquivos:** documentação de prova na pasta 2D e `server.yml`: incluir somente o teste de
contenção de processos Python e as regressões da revisão nos três sistemas, para comprovar o
Job Windows real e executar o hook TypeScript com Node 24 no CI.
O contrato do cano continua somente Linux; nenhuma suíte Python inteira é adicionada à matriz.
**Interfaces:** entrega da branch própria à sessão `Migracao-Rust`.

- [ ] **Step 1: Rodar verificações finais autorizadas**

Em `crates/`: `cargo test --locked --workspace` e
`cargo check --locked --target x86_64-pc-windows-gnu -p hangar-server --tests`.
Em `backend/`: pytest de todos os arquivos tocados num comando. Esperado tudo verde.
As verificações anteriores passaram no contrato 9. A instrução `c195a6a4` exige juntar a fila
v2, conservar as provas da 2D, usar protocolo 10 e repetir os checks afetados antes da publicação.

- [ ] **Step 2: Revisão independente final da branch**

Revisor novo confere `eae31286..HEAD`, spec/plano/provas, segurança e paridade; corrigir problemas
relevantes com testes de regressão e repetir checks afetados.

- [ ] **Step 3: Registrar prova e publicar branch**

Commit documentação; push somente `hangar-server-parte2d`; acompanhar `server.yml` com
`gh run watch` e conferir jobs Linux/macOS/Windows. Esperado CI verde nas três plataformas.

- [ ] **Step 4: Informar resultado à sessão solicitante**

Após ler `hangar-send --help`, enviar commits, testes, link CI e pendências de uso real/Windows.
Conferir branch/hash/status real; não criar PR nem operar serviço.
