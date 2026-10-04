# Parte 2D — registro de verificação

## Task 1 — driver terminal

Commits `fb0fda6b` e `71278ab9`; revisão independente aprovada após uma rodada de correções.

Comando: `cargo test --locked -p hangar-server terminal_input`, em `crates/`.
Resultado final: **37 passaram**, sendo 36 testes de driver/IO/serviços falsos e um fluxo
com tmux em socket próprio. O fluxo isolado conferiu bytes de texto curto, Unicode/emoji,
contrabarras, três linhas, `/clear` e argumento terminado em `;`. Encerrou só o tmux que criou.
`git diff --check` passou. Não houve sessão Claude real nem operação de serviço.

O teste inicial falhou pela ausência do módulo. A revisão encontrou três problemas: buffer
interativo compartilhado entre sessões, guard desatualizado depois de esperar plugin/clipboard
e incerteza perdida durante navegação de pergunta. Cinco regressões reproduziram esses problemas
antes da correção. A conferência independente do diff de correção aprovou os três ajustes.

Outras provas: socket parcial não muda de transporte, plugin incerto não digita, Enter incerto
não reenvia, limpeza só do texto próprio, placeholders novos, filas de espera com vínculo
alterado, respostas com revisão, seleção, interrupção sem abrir rewind e exclusão do clipboard
por arquivo até comprovar a colagem. O driver não tem orçamento de retry próprio.

O aviso preexistente de campo `kind` não lido em `runtime/actor.rs` continua aparecendo.

## Task 2 — executor, fila e protocolo

Commits `a3e02d6a`, `80d1c279` e `042eb9b0`; revisão independente aprovada após duas rodadas.
Contrato interno **9** aplicado no mesmo commit em Python e Rust.

Comando final Rust focado: `cargo test --locked -p hangar-server --test runtime_queue
--test terminal_runtime --test terminal_runtime_tmux --test runtime_actor --test runtime_receipt
--test runtime_recovery --test runtime_gateway --test runtime_contract`, em `crates/`.
Resultado: **77 passaram, dois ignorados** por exigirem Python configurado. As provas afetadas
de lease e de Store foram executadas separadamente com `HANGAR_TEST_PYTHON` e passaram.

Comando Python: `uv run pytest tests/test_runtime_queue.py tests/test_runtime_receipt.py
tests/test_runtime_failure_matrix.py tests/test_runtime_ownership.py
tests/test_runtime_lifecycle.py -q`, em `backend/`. **102 passaram**.
As contagens são da última execução de cada comando, sem somar repetições.

A revisão reproduziu Prepare sem Append, Finish separado de contador/fila e recibo nativo de
Append sem root. As correções espelham a recuperação e a finalização atômica nos dois Stores.
A primeira releitura encontrou reconstrução de entrada já removida; a prova durável de criação
agora preserva a remoção. Os testes reabrem o estado e exigem zero nova tecla, publicação ou
socket. A interoperabilidade Rust → Python → Rust confirma as flags privadas e os contadores.

O runtime e o driver também passaram num fluxo real com tmux isolado e CLI falsa: primeiro
transcript inexistente, Unicode/emoji/contrabarras, recibo e repetição do mesmo id sem reenvio.
Nenhuma sessão real ou serviço foi operado.

Obrigação transferida à Task 3: conter e comprovar término dos filhos mux/PowerShell após morte abrupta do
Rust antes de ativar a reserva ou readotar. A proteção de uma entrada incerta não resolve, por
si só, a disputa com um descendente antigo ainda escrevendo.

## Task 3 — integração Python e contenção

Implementados vínculo terminal durável, encaminhamento dos produtores e controles, reserva no
mesmo diário, correlação de avisos do plugin e barreira dos controles administrativos. O estado
e a prévia terminal continuam com a observação 2C. Protocolo permanece 9.

Os testes iniciais falharam pelos caminhos ausentes; as regressões também reproduziram aviso
antigo, mudança de vínculo, prova parcial em thread reutilizada e troca de modo ainda sem CLI
pronto. A última execução terminal focada passou em **68 casos**.

A matriz Python pertinente passou em **1182 casos**, com **três falhas e um caso Windows
ignorado**. As três falhas eram isolamento dos testes de supervisor: o lançamento direto do
processo falso deixava registro de posse entre testes. Após limpeza explícita e registro
temporário por teste, **os 23 casos de `test_rust_server.py` passaram**. A regressão de colisão
do nome de Job reproduziu o problema antes da correção; depois passou isoladamente. As contagens
são por execução, sem somar testes repetidos.

Rust focado: `cargo test --locked -p hangar-server --test terminal_input --test terminal_runtime
--test runtime_gateway`, em `crates/`: **73 passaram**. O teste de pergunta conserva a identidade
do contrato público; o Python normaliza `indices:null` antes do controle.

Grupo POSIX e Job Windows contêm só os filhos do Rust. Processos falsos comprovaram limpeza
após morte do pai/backend e recusa diante de PID reutilizado. O teste real do Job Windows será
executado no CI; Linux tem também um teste da API Windows simulada para nomes únicos e explícitos.
O fluxo com CLI falsa e tmux isolado comprovou primeiro transcript ausente, recibo e envio sem
duplicação pela reserva. Nenhuma sessão real, serviço ou instalador foi operado.

A revisão independente encontrou seis problemas importantes: primeira identidade inconclusiva
permitia caminho antigo, publicação atrasada atravessava `/clear` no hook, reserva enviava Enter
sem prova prévia, controle adiado era tratado como sucesso, confirmação retornava `None` e buffer
multiline da reserva era compartilhado. A primeira rodada corrigiu esses pontos, com **16
regressões novas verdes** e **634 passagens** na execução dos 12 arquivos afetados. O teste do
hook executa o TypeScript real com Node 24, nos três modos, com conversa atual e após `/clear`.
O teste tmux intercalou duas sessões e reproduziu A recebendo os bytes de B antes da correção;
depois, cada pane recebeu seu próprio texto. A releitura aprovou quatro problemas; restam a
classificação por provedor declarado antigo e a correspondência por cauda de um caractere.
A segunda rodada corrigiu esses caminhos com processos frescos e correspondência mínima por
trecho; conservou forks de Pi comprovados. A execução final selecionada passou em **44 casos**,
com 68 fora da seleção. A segunda releitura independente aprovou conformidade e qualidade,
fechando os seis problemas. Commits da Task 3: `bead8a45`, `58c75289` e `a767febd`.

A política comum de falhas de `74177469` foi integrada na Task 4 após a aprovação desta Task.
O caminho terminal separa `_op_once`, evitando duplicar essa política.

## Task 4 — verificações anteriores à fila v2

Merge da parte1 em `958486f6`, incluindo `fac8d377` e `74177469`. A resolução conserva a
serialização, o vínculo durável, o congelamento de `/clear` e o parâmetro de restauração do
terminal, junto da política compartilhada. Python/Rust continuam no protocolo **9**. A
compilação revelou uma referência a `target` adicionada pelo merge dentro de `run_for`; o
diagnóstico agora usa os argumentos reais `key` e `generation`, sem conteúdo do pedido.

A conferência pós-merge reproduziu três lacunas: rejeição do snapshot privado saudável por
ausência de `public_state`, entrega incerta sem transferência e falha automática sem aviso ao
coordenador. O consumidor terminal valida vínculo/geração/conversa e conserva somente canais
privados. O Rust registra código/motivo, publica `problem` e conserva erro no snapshot; o timer
para até a recuperação coordenada. Fatos e leitura do transcript anteriores a qualquer envio
usam as mesmas quatro tentativas do `RuntimeCoordinator.op`, com pausa antes da última e
transferência somente da sessão afetada. A resposta `unknown` mantém seu contrato e o diário
protegido, registra o motivo e transfere a posse sem repetir o efeito. Resultados normais de
adiamento/recusa continuam fora da contagem de defeito.

As oito provas Python da política passaram, incluindo pausa, mesmo id, outra sessão no Rust,
resposta incerta, aviso automático sem aparelho e diagnóstico sem texto. Os dois casos Rust
novos reproduziram a ausência do aviso antes da correção; depois o arquivo de runtime passou
em **33 casos**, incluindo interrupção do timer e recuperação posterior da leitura.
A regressão adicional reproduziu o envio de texto antigo na quarta tentativa depois de mudar
a geração durante a pausa. A operação agora confere a vida original antes de cada tentativa e
da reserva. O comando focado de política, posse e ciclo de vida passou em **36 casos**; a prova
da geração passou novamente após a última guarda. Nenhum quarto envio foi realizado.

`cargo test --locked --workspace`, em `crates/`: **324 passaram, zero falhou, dois ignorados**.
Método: soma das contagens `test result` dos 35 blocos da execução, sem somar rodadas anteriores.
Os ignorados exigem Python configurado; suas provas de lease e Store já passaram na Task 2.
`cargo check --locked --target x86_64-pc-windows-gnu -p hangar-server --tests` passou. Há avisos
preexistentes de campos/imports não usados; nenhum foi ocultado.

Pytest pertinente, em uma invocação com **44 arquivos**: **1420 passaram, três falharam e um
caso Windows foi ignorado**. Duas falhas eram o dublê headless sem `meta`, corrigido mantendo
suas assertivas. O terceiro teste observou leituras contraditórias de `/proc` por PID; o processo
já estava ausente no diagnóstico. A repetição dos três casos passou; o teste do filho foi
fortalecido para conferir PID e nascimento e passou novamente. Repetições não são somadas às
1420 passagens iniciais. Nenhum caso permanece falhando na conferência focada.

O `server.yml` passa a executar somente as provas de contenção/política e as regressões da
revisão Python nos três sistemas, com Node 24 para executar o hook real. O contrato do cano
continua Linux; não foi adicionada suíte Python inteira à matriz. CI e revisão final ainda
pendentes. `git diff --check` passou.

## Junção da fila v2 — contrato atual 10

Instrução posterior de `Migracao-Rust`: `c195a6a4` integra a limpeza 2B com protocolo 8 e estado
da fila v2 (`seq/next_seq`); a parte 3 passa a 9, e a 2D usa **10** nesta junção. Os checks acima
pertencem ao estado anterior e serão repetidos para a nova integração.

A comparação identificou cinco adaptações: conservar metadados pequenos de recibo ao reduzir
respostas, respeitar Prepare já finalizado, reconhecer a prova de Finish reduzida, recuperar
as flags terminal antes da poda v1 e continuar a sequência por `next_seq`. Foram integradas em
`984801af`, com protocolo 10 pareado. A recuperação também grava `_terminal_generation` das
intenções antigas comprovadas antes de podar o recibo que as identificava.

Provas focadas após a última alteração: **152 Python**, **58 Rust** e **dois testes explícitos
Rust → Python → Rust passaram**. Cobrem lease alternada, contador/finalização, metadados nativos,
remoção após poda, intenção antes do Append e zero efeito para linha já confirmada. A janela
limitada e o oráculo de compactação 2B continuaram passando.

Verificação final sobre v2/protocolo 10: `cargo test --locked --workspace` passou em **335 casos**,
com zero falha e três ignorados. Método: soma dos 36 blocos `test result` dessa execução, sem
somar rodadas anteriores. Dois ignorados de interoperabilidade foram executados explicitamente
na junção; o terceiro de exclusão de lease passou na Task 2. O check GNU Windows de todos os
testes passou novamente.

Pytest final pertinente em **45 arquivos**, uma única invocação: **1446 passaram, zero falhou,
um caso Windows ignorado**. Esse caso é executado no runner Windows pelo `server.yml`, junto
das provas v2 e do hook TypeScript real com Node 24. Os avisos preexistentes permanecem nos logs.
`git diff --check` passou. A revisão final e o CI ainda estão pendentes.

A revisão final de `eae31286..6a94ca12` reprovou quatro problemas importantes: ocorrência
reutilizada quando o primeiro arquivo estava ausente, preenchimento incerto apagado pela próxima
entrada, descendente sobrevivendo ao timeout antes do detach e cadastro sem identidade inicial
que não se recupera quando o vínculo volta. As regressões combinadas estão em implementação.
Os checks verdes acima não cobriam essas quatro combinações; publicação bloqueada até corrigi-las
e aprovar a releitura independente.

## Releitura dos quatro consertos e correções seguintes

A releitura independente de `1f28144b` aprovou os quatro consertos: cada teste novo falhou com o
trecho revertido numa cópia. Ela achou um bloqueio permanente novo: a trava de escrita nunca saía
depois de o transcript confirmar a entrada incerta, e a fila parava. `09829f55` passa a trava
para a próxima incerta da conversa ou a retira, nos dois lados. Também aposenta o registro em
espera que volta com outra chave, só recolhe o líder do comando depois do grupo, avisa
`command_tree_stuck` sem soltar a posse e alinha `/clear` com argumentos. Uma segunda releitura
achou a lacuna da segunda conversa: incerta depois de `/clear` ficava sem trava. `7ddb27dd`
barra a escrita enquanto houver incerta na conversa pedida, nos dois lados.

## Testes reais com Claude Haiku

Ambiente: backend desta branch com `HOME` temporário, portas 18765/18766/18768, `CP_AUTH_TOKEN`
próprio e tmux `-L hangar-2d-real` via embrulho no `PATH`. O embrulho do `claude` fixava
`--model claude-haiku-4-5` e a conta; todas as sessões mostraram "Haiku 4.5" na linha de status.
A conta 200-3 estava sem `refreshToken` ("Login expired"); por decisão de `Migracao-Rust` os
casos com resposta usaram a 02-200. Entregas contadas no transcript cru (mensagem do usuário ou
`queued_command` da fila interna da TUI).

| Caso | Resultado | Evidência |
|---|---|---|
| Abrir sessão com terminal | ok | `POST /api/sessions`; Rust dono (sem `rust_op_failed`) |
| Mensagem simples | ok, 1 entrega | `ok tres`: contagem 1; caminho do plugin (`Prompt from the hangar plugin`) |
| Várias linhas, Unicode, contrabarras | ok, 1 entrega | três linhas, `ção 🚀` e `\\` intactos; contagem 1 |
| Mensagem com o Claude ocupado | ok, 1 entrega | entrou pela fila interna da TUI (`queue-operation` + `queued_command`); fila do Hangar vazia depois |
| Responder pergunta (`AskUserQuestion`) | ok após `9d2e2c10` | antes: `erro_opcao_nao_convergiu`; depois: "Qual cor você prefere? → Verde" |
| Esc | ok | "Interrupted · What should Claude do instead?"; estado ocioso |
| `/clear` | ok após `fcca5525` | antes: entrega incerta e sessão ao Python; depois: conversa nova e mensagem seguinte com 1 entrega |
| Restart do backend com mensagem na fila | ok, nada redigitado | com o seletor aberto a entrada ficou `delivered:false` antes e depois do restart; após a resposta, 1 entrega |
| Queda do `hangar-server` | ok após `c7046d0f`/`97cc3f9c` | três `kill -9` em 60 s: "o Python assume a porta 18765"; mensagem da fila entregue 1 vez (com SSE aberto); mensagem nova 1 vez |
| Falha antes de efeito, só uma sessão | ok | ver abaixo |

Falha antes de efeito, com `t2d-b` e `t2d-c` no Rust, seletor aberto na `t2d-c` e SIGTERM no
backend: o Python recusa os fatos durante o encerramento. Diário só da `t2d-c`:
`runtime.rust_op_failed` 1, 2 e 3 em 07:00:42.713–.723, pausa, 4 em 07:00:44.733 e
`runtime.parte_para_python` em 07:00:44.767, código `RustOpError`, motivo
`IPC recusou a operação (503: terminal_facts …)`. Log do Rust: `política do Python falhou …
code=policy_transport`, `entrada terminal entrou em erro … code=terminal_facts` e
`runtime recusou operação kind=drain code=terminal_facts`. Nada foi digitado no seletor.

Defeitos que só os testes reais mostraram, cada um com teste que falha sem o conserto:

- `c3f7705c`: o Claude 2.1.289 desenha uma sugestão esmaecida no composer vazio; o Rust adiava
  toda entrada com `composer_busy` e a prova de submissão nunca via o composer vazio.
- `9d2e2c10`: `select` em pergunta `ask:` ia ao plugin e voltava recusado; agora cai no teclado
  da TUI. Pedido de permissão continua recusado.
- `fcca5525`: a prova de submissão do `/clear` exigia o vínculo antigo.
- `b78bcd26`: fechar sessão de terminal dava 500 ao revalidar o vínculo do pane já morto.
- `09b330b5` e `524447f9`: criar e fechar sessão com nome reaproveitado dava 500. A causa era o
  registro da vida antiga, esperando identidade depois de restart.
- `c7046d0f` e `97cc3f9c`: a morte do `hangar-server` derrubava o backend Python inteiro, porque
  a recuperação de um registro morto lançava exceção. Agora cada sessão recupera sozinha, com
  `runtime.recover_failed` no diário. Registros em espera não são recuperados. O erro de vínculo
  diz quais campos mudaram.

Incidentes do teste: a subida do backend isolado matou o cano real do `Migracao-Rust`, porque
`matar_orfaos` varre todo o `/proc` do usuário. `bb80cf31` grava `HANGAR_CANO_OWNER` e colhe só os
canos do próprio backend; o lançador de teste também desligava a varredura. Com `CP_PROJECTS_DIR`
na pasta da conta, a fila e os demais arquivos do Hangar saíam de `projects_dir.parent`, que na
conta é link para o `~/.claude` real. Foram apagados só os seis arquivos do teste, conferidos por
nome, horário e chave; depois o `CP_PROJECTS_DIR` passou a ser um link dentro do `HOME` temporário.

## Verificação final

A revisão independente dos consertos vindos dos testes reais (`bb80cf31`, `c3f7705c..97cc3f9c`)
reprovou três pontos, corrigidos em `80cd98d9` e `3ce0693b` com testes que falham sem o conserto:
sessão cuja recuperação falhava ficava presa até o restart (o registro agora é aposentado e o
próximo `prepare_session` o refaz pelo estado durável); `select` em pergunta `ask:` chegava ao
teclado sem a trava do painel aberto (agora com a trava e `require_cursor`); fechar sessão com
vínculo já mudado antes do kill dava 500. A leitura que prova o `/clear` continua conferindo o
pane. O teste de runtime que fixava `/clear` como incerto passou a exigir aceito (`7457941a`).
Esses fluxos foram repetidos no backend isolado depois do conserto: escolha "Dois" aceita,
`/clear` aceito, queda do Rust sem `recover_failed` e entrega seguinte com 1, fechar com `ok`.

Antes do push, o `silent-failure-hunter` pediu traceback no log quando uma sessão não volta,
o último código de erro dentro de `submit_unproved:<código>`, aviso quando o fim do comando não é
comprovado, contagem dos canos pulados por dono diferente e aceitar envio só se o texto também
sumiu da tela com estilo (esmaecido nunca prova envio). Falha antes da prova de contenção continua
bloqueando a sessão: soltá-la deixaria o Python escrever com descendente do Rust possivelmente vivo.

Sobre o commit final da branch, uma única execução, sem somar rodadas anteriores:

- `cargo test --locked --workspace` em `crates/`: **348 passaram, zero falhou, três ignorados**
  (soma dos 36 blocos `test result`).
- `cargo check --locked --target x86_64-pc-windows-gnu -p hangar-server --tests`: passou.
- pytest em **47 arquivos** ligados à 2D e aos consertos, uma invocação: **1506 passaram, zero
  falhou, um caso Windows pulado**.
- `git diff --check`: passou.

Uma rodada intermediária do workspace falhou em um caso: o teste antigo do `/clear`, ainda com a
expectativa de incerto. Isolado, ele falhou 4 vezes seguidas com `Unknown`; depois de restaurar
os arquivos instrumentados, passou 5 de 5 e na suíte inteira. A causa daquelas 4 falhas não foi
comprovada.

`test_runtime_final_fixes.py` entrou no `server.yml` (caminho e etapa de pytest nos três sistemas).
Nenhuma sessão real, serviço ou instalador foi operado depois dos dois incidentes registrados acima.
Observação fora da 2D: o backend não apaga `cc-socks/<pid>.sock` ao sair; os 15 do teste foram
removidos à mão, conferindo que nenhum PID estava vivo.

## CI do `server.yml`

Primeira execução da branch (`d22ff6c5`, run 37196525368): Linux verde. macOS falhou uma vez em
`terminal_control::capture_waits_for_end_marker_and_rejects_missing_or_wrong_end` (teste da 2C,
limite de 250 ms) e passou na repetição. Windows falhou nas duas tentativas em cinco testes de
`terminal_runtime` da Task 2, que nunca tinham rodado no Windows. Um commit só de diagnóstico
(`f2065619`) mediu no runner: cada leitura de fatos leva ~0,1–0,3 s, porque grava `prepare`,
`dispatch` e `finish` duráveis (`fsync` e `rename`). As operações avançavam e terminavam
`accepted`, sem travar; a drenagem de duas entradas passava de 2 s. O processo alheio sobreviveu:
o Job não alcança processo fora da árvore do comando. A 2D liga no Windows em produção, então as
esperas por condição ganharam teto de 10 s, e o processo alheio dorme 60 s; condições iguais.

Com o Rust verde no Windows, a etapa Python mostrou três falhas de `test_runtime_process.py`
(contenção pelo Job, nunca executada antes). No Windows, o processo encerrado segue listado com o
mesmo nascimento enquanto alguém segura um handle dele, e `_same_process` o tratava como vivo:
`reconcile_startup` recusava com "outro ciclo do backend ainda possui escritores Rust" por um
backend já morto. Agora, no Windows, processo sem threads conta como encerrado; o teste da
limpeza usa o mesmo critério em vez do status de zumbi, que lá não existe.

## Ainda pendente

- Reserva Python sem aparelho conectado não drena a fila sozinha; com o Rust de pé o timer cobre.
  Era assim antes da 2D (`api.py`, comentário do gatilho do drain).
- Entrada nova enviada com a sessão pronta passa na frente de entrada antiga parada na fila.
- Registro durável de sessão fechada volta como "esperando identidade" a cada restart.
- O backend não apaga o próprio socket em `cc-socks` ao sair.
- No Windows, cada leitura de fatos custa ~0,1–0,3 s de diário durável; uma entrega faz várias.
- `terminal_control::capture_waits_for_end_marker…` (2C) é instável no macOS com limite de 250 ms.
- Uso real com o dono e Windows instalado; captura `-e` do composer fica só no POSIX (psmux sem prova).
- Porte do envio/controle Codex com terminal: permanece Python/RPC nesta árvore; fora da 2D.
