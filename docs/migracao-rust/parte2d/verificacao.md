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
com 68 fora da seleção. A segunda releitura independente está pendente.

A política comum de falhas de `74177469` será integrada com a parte1 após a revisão desta Task.
O caminho terminal já separa `_op_once`, evitando duplicar essa política.

## Ainda pendente

- Task 3: revisão independente do diff; depois integrar a parte1 e conferir a política de falhas.
- Task 4: workspace Rust inteiro, compilação cruzada Windows, pytest dos arquivos tocados,
  revisão final, push e CI nas três plataformas.
- Uso real do Claude e Windows instalado: somente com o dono, conforme o pedido.
- Porte do envio/controle Codex com terminal: permanece Python/RPC nesta árvore; fora da 2D.
