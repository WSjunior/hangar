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

## Ainda pendente

- Task 4: revisão final, push e CI nas três plataformas sobre v2/protocolo 10.
- Uso real do Claude e Windows instalado: somente com o dono, conforme o pedido.
- Porte do envio/controle Codex com terminal: permanece Python/RPC nesta árvore; fora da 2D.
