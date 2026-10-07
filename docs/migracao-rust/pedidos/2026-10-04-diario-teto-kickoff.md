# O diário enche de sucesso e para de registrar falhas

## O defeito

O diário de uso (`~/.hangar/logs/diario/uso-AAAA-MM-DD.jsonl`, `backend/app/diag.py`) tem teto de
4 MB por dia (`_TETO_DIA`). No notebook do dono ele bateu o teto às 10:15 de 04/10 (evento
`diag.teto`) e parou de gravar: a recusa "escrita suspensa" das 11:10 não ficou em lugar nenhum. O
que enche é sobretudo `api.servidor` com nível `ok` (uma linha por pedido HTTP bem-sucedido,
`backend/app/api.py` ~640) e `lista.falhou`/leituras da tela. Este diário é a única fonte para
diagnosticar falhas da migração (regra: toda falha do Rust registrada com código e motivo).

## O que fazer

1. Meça num diário real desta máquina (`~/.hangar/logs/diario/uso-*.jsonl`, só leitura) quanto do
   volume é `ok` e de quais eventos.
2. Conserte pela causa, para que falha (nível `aviso`/`erro`) nunca deixe de ser gravada por causa de
   sucesso: por exemplo, sucesso de `api.servidor` sem valor de diagnóstico deixa de ir ao diário (ou
   vai agregado/amostrado), e o teto passa a valer para `ok` antes de valer para falha, com um teto
   rígido maior só para não encher o disco. Escolha a forma mais simples que garanta isso e explique.
   Não tire do diário o que hoje é usado para diagnóstico (etapas, códigos, origem da falha, `ms`
   dos pedidos lentos) — leia "Diário de uso" em `docs/decisoes/plataforma.md` antes.
3. Teste que falha sem o conserto: diário com o teto de sucesso batido ainda grava um `erro`.
4. Uso real: backend de teste isolado desta branch (`HOME` temporário, portas livres diferentes de
   8765/8766/8768, `CP_AUTH_TOKEN` próprio, `tmux -L` próprio; a branch já tem `bb80cf31`), gere
   pedidos até passar o teto de sucesso e confira que uma falha provocada aparece no diário. Pare
   tudo e apague o `HOME` no fim.
5. pytest dos arquivos tocados; revisão independente por subagente antes do push.

## Entrega

Branch `fix/diary-cap` (este cwd, de `origin/hangar-server-parte1` `ac5c975d`). Commits em inglês,
`git add` de caminhos explícitos, `HANGAR_SEM_PASSO=1`. Push só desta branch, CI verde. Mande para
`migracao-rust-2`: medição, conserto, commits, testes, link do CI. Não junte na
`hangar-server-parte1`.

## Proibido

Subir/reiniciar/parar o backend real ou o `hangar-backend`; usar 8765/8766/8768; tocar no tmux
padrão ou em sessão real; rodar instaladores; mexer na `main` ou na `hangar-server-parte1`; texto de
conversa ou credencial no diário.
