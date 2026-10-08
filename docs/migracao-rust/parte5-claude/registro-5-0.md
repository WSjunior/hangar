# Parte 5, Task 5-0: registro da execução

Plano `plano-5-0.md`, executado em 07/10/2026 por subagente com revisão por Task e revisão final
da branch (base `dcae49227`, branch `hangar-server-parte5-claude`, contrato 37). Aqui ficam as
decisões tomadas durante a execução que mudam o plano ou o comportamento, e o que ficou para
depois.

## Decisões que mudam o plano ou o que o usuário vê

- **A porta é por nome da sessão**, e a entrada do ator é procurada só depois de passar por ela;
  repasse ao Python nunca acontece com o passe na mão (o `freeze` do Python esperaria a própria
  rota). Fechamentos são contados; fechamento cancelado ou estourado (60 s, `ingress_busy`) se
  desfaz sozinho.
- **Porta retida pela troca de agente recusa na hora** (409 `session_transfer_busy`, como o
  `_transfer_guard` fazia); o congelamento curto de relançamento continua esperando até 30 s.
- **"Saudável" é a negação do `_rust_failed` do Python, sem `initialized`**; vínculo trocado
  (`terminal_facts`) conta como doente e vai ao Python, que refaz o vínculo.
- **`/select`:** resultado incerto responde `erro_sem_confirmacao_resposta`; recusa responde com
  `detalhe` e diário `opcao.*`; só o código `no_pending_permission` vira "nenhum pedido de
  permissão pendente" (adiado, recusado ou incerto ficam 503 com o motivo). O Python espelha o
  Rust, então convidado e Connect veem o mesmo.
- **Falha do runtime nas rotas portadas é 502 `erro_envio_falhou`** (antes 500 no Python, que
  agora espelha); `/steer` e `/interrupt` sem turno são 409 `erro_sem_turno`.
- **"Conversar sobre isso" com texto não confirmado é 502**, não 409: a pergunta já foi fechada
  no terminal, o app precisa abrir o espelho e o usuário não deve reenviar às cegas.
- **Falha ao montar a linha de status é cosmética**: não derruba a sessão.
- **Cota do Claude:** o Rust pede `GET /internal/quota` com cache próprio de 5 min (o Python não
  guarda mais) e guarda falha de transporte por 30 s.
- **`_born_in_rust` = sem terminal e o Rust é dono do provedor**: "nascer no Rust" só existe para
  sessão sem terminal; a com terminal nasce pelo `_await_birth`.
- **Sem medição nesta execução**: o dono não autorizou subir backend; o roteiro está em
  `medicao-5-0.md` e os números saem do uso real.
- **Perda conhecida**: se o Rust cai no meio de um envio, o app vê a conexão cortada e um reenvio
  leva id novo (antes o Python repetia o mesmo id no Rust novo).

## Para depois

- `_rebind` passou a congelar depois de fechar a porta (até 60 s); uma escrita que nasce no
  Python nessa janela pode ir ao vínculo velho. Reconferir na C3, quando o `_rebind` e os fatos do
  terminal forem para o Rust.
- Reabertura que falha (Rust de pé e a chamada falhando) só vai ao diário: a porta fica presa até
  o Rust reiniciar. Reabrir falhar quase sempre é o Rust caído, e a porta mora na memória dele.
- `reopen(held=true)` com retenção zerada ainda desconta um fechamento comum (janela estreita
  depois de reinício do Rust); guarda: `if held && holds == 0 { return }` em `ingress.rs`.
- Corrida entre `_enter_rust` reaplicando retenções e uma troca terminando ao mesmo tempo.
- `parse_owns` aceita lista vazia; `start_sessions` pode rodar antes do primeiro
  `configure_transport` (importa quando o Codex for do Rust).
- `default_model`/`default_effort` do Codex não vêm no payload do motor (metade Codex, 5E).
- `Terms::mark_open_for_test` é público (escondido da documentação) para os testes de integração.
- Testes: `promoted` nulo vira falso no Rust (o ator não produz nulo); o teste do `/steer` adiado
  prova o id só indiretamente; o teste de 413 manda 100 MiB.
- README da migração: data do cabeçalho e a frase de "Ainda no Python" sobre `/then`, `/loop`,
  criar e `/model-effort`.
