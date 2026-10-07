# Revisão do PR #30 — casos reais

Data: 04/10/2026. Backend de teste isolado: `HOME` temporário, porta pública 18865 (Rust), Python
atrás numa porta de loopback, convite e Connect movidos para 18866/18868, `CP_AUTH_TOKEN` próprio,
tmux próprio (`TMUX_TMPDIR`). Binário `hangar-server` debug desta branch, protocolo 12. Sessões:
`teste` (bash, sem transcript) e `haiku` (Claude Haiku 4.5, conta `~/.claude-02-200`).
Repositório e remoto bare dentro do `HOME` temporário. Quem atendeu cada pedido foi conferido no
log do Python: pedido atendido pelo Rust só aparece lá como `GET /internal/workspace/context`.

## Antes das correções (cabeça `b97382c9`)

| Caso | Resultado | Quem atendeu |
|---|---|---|
| Árvore `files/list` | 200, entradas com nome acentuado e espaço | Rust |
| Leitura `files/read` de `notas.txt` | 200 com digest | Rust |
| Leitura com `../../fora/citado.txt` | 400 `erro_arq_fora_da_raiz` | Rust |
| Leitura por symlink `link-fora/citado.txt` | 400 `erro_arq_fora_da_raiz` | Rust |
| Leitura de `.git/config` | 403 `erro_arq_area_do_git` | Rust |
| Arquivo grande (2,7 MB) | 200 truncado no limite | Rust |
| Busca por conteúdo e por nome acentuado | 200 | Rust |
| Escrita com digest certo | 200, disco alterado | Rust |
| Escrita repetida com o digest antigo | 409 `erro_arq_mudou_no_disco`, disco intacto | Rust |
| Escrita fora da raiz, por symlink e em `.git/hooks` | 400/400/403, nada escrito | Rust |
| diff, commit, last-message, push ao remoto local | 200, remoto recebeu o commit | Rust |
| fetch, log, pull fast-forward de commit feito por outro clone | 200, HEAD avançou | Rust |
| Branch `--orphan=x` no checkout e `-x` na criação | 400, nada criado | Rust |
| `/file` sem transcript | 404 `erro_sessao_inexistente` (mesmo do Python) | Python |
| `/file` citado fora da raiz com `Range: bytes=0-2` | 206, `content-range: bytes 0-2/8` | Rust |
| `Range: bytes=x` | 400 | Rust |
| `/file` de HTML | documento com iframe `sandbox`, conteúdo em `data:`, sem token | Rust |
| `file/text` GET e POST do citado fora da raiz | 200; POST com digest velho 409 | Rust |
| `file/text` GET e POST de arquivo não citado | 403 `erro_arquivo_nao_citado` (o mesmo do Python) | Rust |
| `kill -9` no `hangar-server` durante push lento (hook de 4 s) | cliente recebe conexão cortada; hook rodou uma vez; remoto intacto; nenhum `git` vivo; Supervisor subiu outro `hangar-server` | — |

## Depois das correções (`066660bd`)

| Caso | Resultado | Quem atendeu |
|---|---|---|
| Árvore, leitura fora da raiz, `.git/config`, escrita com digest certo e velho | iguais à rodada anterior | Rust |
| `~ninguem/../../fora/citado.txt` | 400 `erro_arq_fora_da_raiz` | Rust |
| Commit com `post-commit` que deixa um job em segundo plano | commit 200; o job terminou e gravou o arquivo dele | Rust |
| Remoto com `refs/remotes/origin/--detach`, checkout `--detach` | 400 `branch inexistente`; HEAD continua em `refs/heads/master` | Rust |
| Haiku cita `repo/gl/config` (`gl` → `.git`); `file/text` GET e POST com `config`, `gl/config` e o absoluto | 403 `erro_arq_area_do_git` nos seis; `.git/config` intacto | Rust |
| 4 pushes lentos (hook de 4 s) ocupando as vagas de escrita + commit no meio | commit respondido em 22 ms pelo Python, uma vez; `hangar-server.log`: `vagas de Git/arquivos cheias; repassa ao Python code="workspace_busy"`; hook do remoto rodou 4 vezes (um por push) | Python (repasse) |
| `kill -9` no `hangar-server` três vezes em 60 s | Supervisor: `hangar-server desligado (quedas); o Python assume a porta 18865` | — |
| Com o Python dono da porta: árvore, leitura, escrita, escrita com digest velho, commit, histórico, `/file` com Range | mesmos formatos e status do Rust (200, 409, commit único, 206 `bytes 0-2/10`) | Python |

## PWA (390 × 844, navegador automatizado, backend de teste com o Rust)

- Lista de sessões → chat da sessão `haiku` → painel do repositório: Mudanças, Arquivos e
  Histórico carregaram; nenhum desses pedidos apareceu no log do Python.
- Arquivos → mostrar tudo → `notas.txt` → Arquivo → digitado → Salvar: o disco recebeu o texto e
  o contador passou a `+2 −1`. Sem pedido de escrita no log do Python.
- Histórico listou `pelo python`, `durante pushes`, `rodada 2` (com `origin/master`) e `primeiro`.
- Fora do PR: o anexo `pagina.html` de uma mensagem que citava dois caminhos absolutos ligados por
  " e " abriu o visor com 404 `erro_arquivo_nao_encontrado`, porque o front mandou os dois
  caminhos como um só (`…/citado.txt e …/pagina.html`). É o parser de caminhos do front; o Python
  devolveria o mesmo 404.
- Não foi um aparelho físico nem o app Expo.
