# Verificação — Git e arquivos em Rust

Data: 03/10/2026. Base do trabalho: PR #24 em `128262597d926416cb3a5e91f5ac3a2f99b798e1`.

## Uso real

Fixture `desktop-native/tools/workspace_fixture.py`, repositório e remoto bare descartáveis,
com o servidor Rust compilado para Windows. O Python da fixture **recusa** qualquer operação
de arquivos/Git: somente metadados e as outras rotas são atendidos por ele.

- Aplicativo nativo instalado: abriu a sessão, a árvore e Markdown; editou `notas.txt` e salvou
  pelo atalho. A tela mostrou o salvamento e a releitura da API confirmou os bytes no disco.
- PWA, viewport 390 × 844, contra o mesmo servidor Windows: abriu a sessão, o arquivo citado,
  a árvore e o histórico; editou `README.md`, acionou Salvar e confirmou a nova impressão/leitura.
- Contador `python_domain_calls` permaneceu **zero** depois dos fluxos.
- O PWA usou um navegador automatizado em viewport móvel; não foi um aparelho físico nem o app Expo.
- A instalação em uso foi preservada: configuração, processos e portas da validação eram separados.

## Testes e verificações

| Ambiente | Verificação | Resultado |
|---|---|---|
| Linux | pytest dos módulos ligados à migração, paridade, transporte e concorrência | 339 passaram |
| Linux | núcleo Rust, rotas e repasse | 22 passaram |
| Linux | testes locais de Git e árvore do desktop | 3 passaram |
| Linux | regressão do salvamento no GitTabs | 12 passaram |
| Linux | check do frontend, build para uso real e check do desktop | passaram |
| Linux | Clippy do núcleo com `-D warnings` | passou |
| Windows | Rust: núcleo, rotas e repasse | 21 passaram |
| Windows | paridade Python/Rust e ponte | 46 passaram, 3 casos exclusivos de POSIX ignorados |

A regressão de morte do executor foi executada também no Windows: consulta de estado por
`GetExitCodeProcess`, sem usar `os.kill(pid, 0)`, que não é uma sonda segura nessa plataforma.
Os avisos preexistentes do desktop, Svelte e depreciações do websocket não foram alterados.
Não foi executada a suíte completa do repositório. macOS e execução dos workflows remotos
ficam para o CI; o workflow do nativo agora observa também `hangar-workspace`.

## Revisão independente

Corrigidos e cobertos por regressões: CORS; campos privados/extras nos POSTs; nomes escapados
no numstat; barra invertida literal no POSIX; validação de seleção sem refazer o repositório
por arquivo; encerramento dos descendentes quando o executor morre. O teste do desktop foi
adaptado à listagem compartilhada. A inspeção adicional também separou vagas de metadados,
leituras e alterações, e tornou a trava do cache individual por cwd.

## Medição local

Dados sintéticos, 30 chamadas por caso, binário **debug**, mediana da operação sem serializar
a resposta HTTP. Arquivo de leitura: 450.000 bytes. Pico RSS inclui os imports do processo
Python; esses números não são uma comparação de todo o backend.

| Operação | Python | Rust | Pico RSS Python | Pico RSS Rust |
|---|---:|---:|---:|---:|
| Ler texto e calcular digest | 663 µs | 332 µs | 47,5 MiB | 8,7 MiB |
| Consultar arquivos alterados em repositório pequeno | 2.032 µs | 2.758 µs | 47,5 MiB | 6,5 MiB |

Git pequeno continua ligeiramente mais lento nesta medição; o monitor de vida acrescenta um
processo no POSIX. A espera inicial de 5 ms no executor foi reduzida para 1 ms depois da
medição. A migração não promete acelerar o comando Git: memória, código compartilhado e
retirada das operações do Python são os resultados desta parte.
