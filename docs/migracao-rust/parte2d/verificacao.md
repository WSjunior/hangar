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

## Ainda pendente

- Task 2: posse/fila/diário/recuperação/gateway e protocolo 9.
- Task 3: encaminhamento Python completo, correlação do plugin e trava de clipboard comum.
- Task 4: workspace Rust inteiro, compilação cruzada Windows, pytest dos arquivos tocados,
  revisão final, push e CI nas três plataformas.
- Uso real do Claude e Windows instalado: somente com o dono, conforme o pedido.
- Porte do envio/controle Codex com terminal: permanece Python/RPC nesta árvore; fora da 2D.
