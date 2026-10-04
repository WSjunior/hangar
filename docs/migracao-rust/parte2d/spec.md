# Parte 2D — envio Claude terminal pelo Rust

Execução autorizada pelo pedido de 03/10/2026. Escopo: Claude com terminal; custos/uso,
Codex, demais provedores, interfaces, serviços e instaladores preservados.

## Posse e identidade

Usar `RuntimeCoordinator`, `QueueActor`, `Store` e `ReceiptIndex` existentes. Uma vida leva chave
durável, geração, conversa, pane e nascimento do multiplexador. Registrar terminal sem cano;
transferir sob barreira, aguardar operações antigas e liberar a trava Python antes de adotar.
Conferir identidade depois de esperar a trava e antes de cada efeito. `/clear`, rename, troca de
modo e recriação passam pela barreira. Timeout depois de despachar nunca repete a operação;
transferir a posse ao Python exige antes comprovar o fim dos escritores antigos.

Contrato interno **9** em `RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL`, juntos. Porta pública,
porteiro, autenticação de convidados, portas 8766/8768 e segredo em memória ficam iguais.

## Entrega

1. Validar texto antes de qualquer efeito: tab/newline permitidos, demais controles recusados.
2. Socket nativo apenas para recados reconhecidos. Python resolve endereço/origem/modo;
   Rust escreve o envelope. Ausência/falha de conexão antes da escrita permite próximo transporte.
   Falha após possível escrita produz `unknown` e conserva a entrada.
3. Plugin: somente long-poll da conversa acompanhada; slash fica no teclado da TUI.
   Rust escolhe `user` apenas com capacidade,
   idle e sem menção `@`, slash ou bash; demais casos usam `fill`. Python publica e retorna aviso,
   sem tecla. Aviso carrega publicação/geração; aviso atrasado ou sem correlação não confirma
   publicação nova. Aviso não recebido produz incerteza, salvo prova segura de não escrita. Rust dirige
   Enter e a prova de composer. Nunca digitar por cima de fill incerto.
4. Reserva tmux: guard de overlay/pergunta, prontidão, limpeza prévia do composer, paste/literal,
   prova de entrada, Enter e prova de submissão. Preserve slash, placeholders novos e Unicode.
   Windows usa alvo completo e clipboard com exclusão global até provar a colagem; nunca confiar
   em buffer psmux que devolveu zero. POSIX multiline usa buffer via stdin e CR de submissão.
5. Resultado `accepted/deferred/rejected/unknown` explicita o estágio. Retry somente se não houve
   submissão e a limpeza do texto próprio foi comprovada; máximo duas reentregas, no mesmo
   contador da fila (original mais duas). Driver não cria orçamento de retry próprio. Após Enter,
   limpeza não comprova ausência de entrega: fica incerto. Logs só contagens/códigos/identidade.

## Fila e controles

Todos os gatilhos (HTTP, recado, hook, loop, SSE e timers) seguem o dono. Claim de uma entrada,
diário/cursor antes de efeito; resultado ambíguo permanece protegido. Confirmação usa transcript
cru e fila interna da TUI, preserva anexos e ocorrências distintas de textos iguais. Working nunca
autoriza requeue. Confirmar/drenar também sem aparelho/SSE. Fila projetada continua visível.

Opções, resposta de pergunta, navegação, terminal interativo, interrupção e promoção explícita
da fila usam a mesma serialização. `steer:true` de recado nunca emite `C-x C-s` automaticamente.
Política pública e validação de permissão/pergunta permanecem na API. Controles administrativos
não portados precisam de barreira de posse antes da reserva; nenhum escritor paralelo.

## Falhas do Rust

A regra recebida de `Migracao-Rust` em `74177469` também vale no terminal. Reaproveitar
`RuntimeCoordinator.op`, `_hand_to_python` e `failure_reason`: registrar código e motivo de
cada falha; recusa comprovadamente anterior a qualquer efeito tenta três vezes, espera e tenta
uma última vez. Esgotadas as quatro, somente a sessão afetada passa ao Python até reiniciar o
backend. A mesma operação pode então executar pela reserva, sob a mesma posse e diário.
Efeito possível impede toda repetição; o erro aparece e a entrada permanece protegida. Uma
resposta normal, como sessão ocupada ou botão antigo, não é defeito do Rust. Esse orçamento de
falha de operação não cria um segundo contador de reentrega de mensagem na fila.

## Prova

Testes primeiro com CLI falsa e `tmux -L` isolado: socket parcial, plugin incerto, placeholders,
composer ocupado/ilegível, texto curto/multiline/imagem/backslash, overlay, slash/clear, controles,
concorrência, perda de resposta, geração, retomada e fila sem SSE. Revisão independente por Task.
Ao final workspace Rust inteiro, check Windows e pytest tocado; revisão final, push desta branch
e CI nas três plataformas. Uso real/VM instalada ficam explicitamente pendentes do dono.
