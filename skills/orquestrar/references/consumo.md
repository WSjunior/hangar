# Consumo por papel na execução

Quem abre uma sessão mede seu intervalo; o revisor faz isso pelo verificador. O árbitro mede
também seu próprio período. Leia na abertura/encerramento e na retrospectiva. Não há coleta
periódica, consulta de preços ou varredura de conversas alheias.

## Capturar início e fim

Use o coletor existente do Hangar (`stats.Accumulator`) pelo comando abaixo. O plano registra
o caminho do checkout do Hangar. Crie `<duravel>/medicao/` e substitua todos os placeholders.
O transcript deve ser o caminho absoluto confirmado da sessão, não um nome tmux ou um glob.

```bash
uv run --directory <hangar>/backend --no-sync python -m app.orq_consumo snapshot \
  --provider <claude|codex|pi|omp|kimi> --transcript <caminho-absoluto> \
  --role <papel> --session <nome> --model <modelo-observado> --effort <esforco-observado> \
  > <duravel>/medicao/<nome>-inicio.json
```

Capture antes do primeiro pedido e repita o comando ao encerrar, usando `<nome>-fim.json`.
Confira o código de saída antes de registrar o artefato. Se a sessão já trabalhou, o início
marca apenas o período observado: o consumo anterior fica fora. Não invente um início zerado.
Na troca de sessão, papel ou modelo, feche o par atual e abra outro, com nomes de arquivo novos.
O mesmo vale ao delimitar Tasks/rodadas: intervalos distintos, sem somar o total e suas partes.

Arquivo ainda inexistente: aguarde sua criação confirmada antes do pedido. Sessão perdida ou
fonte inacessível: registre a lacuna; a falta do final não vira consumo zero. Modelo e esforço
são observações da sessão, não valores deduzidos do contrato nem atribuição por chamada.
JSON inválido, contador reiniciado e linha ainda pela metade impedem medição exata. Linha parcial
exige esperar a escrita terminar; um par sem uso novo é informado, não vira consumo comprovado.

## Relatório

```bash
uv run --directory <hangar>/backend --no-sync python -m app.orq_consumo report \
  --pair <duravel>/medicao/<sessao1>-inicio.json <duravel>/medicao/<sessao1>-fim.json \
  --pair <duravel>/medicao/<sessao2>-inicio.json <duravel>/medicao/<sessao2>-fim.json \
  > <duravel>/medicao/relatorio.json
```

O relatório soma **final menos inicial**, por papel, separando entrada sem cache, cache lido,
cache criado e saída. Fontes, identidade e intervalos acompanham os números. Repetição da mesma
fonte em intervalos sobrepostos, arquivo trocado/truncado/reescrito e contadores incompatíveis
falham explicitamente. Preserve os transcripts para a conferência dos prefixos registrados.
Gere o relatório na mesma máquina e antes de mover/arquivar as fontes.

O total cobre **as fontes registradas**. Subagentes com transcript próprio precisam de seus
próprios pares, vinculados ao papel de quem os abriu; sem eles, declare essa cobertura ausente.
Não procure filhos por coincidência de diretório ou nome. O JSONL de eventos continua com seus
tipos atuais; acrescente só os caminhos dos relatórios ao registro e ao pedido da retrospectiva.

Janela observada inclui espera e não mede produtividade. Tokens não equivalem a preço nem a
porcentagem da assinatura. Uma variação de cota é da conta inteira; outras sessões e a renovação
da janela impedem atribuí-la a este trabalho sem medição separada.

## Comparar configurações

A retrospectiva usa estes dados nas seções de desperdício e de modelos que já existem.
Para testar uma troca de modelo, proponha tarefas representativas e repita o mesmo pedido,
base de código e critérios nas configurações autorizadas, duas ou três vezes cada. Registre
versões, tokens separados por cache e papel, reprovações e resultado da verificação.
Execuções de comparação consomem cota e exigem autorização; não são disparadas pela retrospectiva.
Uma execução isolada é observação, não prova de que uma configuração economiza ou entrega melhor.
