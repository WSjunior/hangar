---
id: 2026-10-06-nova-sessao-herda-da-criadora
titulo: Sessão criada por outra herda a conta, o modo de permissão e o sem terminal de quem a criou
comando_posix: ./scripts/install-hangar-send.sh
comando_windows: powershell -ExecutionPolicy Bypass -File install.ps1 -Update
prova: ~/.local/bin/hangar-send ~/.claude/CLAUDE.md
destrutivo: false
---

Quando uma sessão cria outra (`new_session` ou `hangar-send --new`) sem dizer conta, modo de
permissão ou sem terminal, a nova usa os da criadora; o que for informado continua valendo. Se a
conta da criadora estiver com 95% ou mais de uso, a sessão nasce na conta com mais folga, e a
resposta da criação diz de onde veio a conta. As sessões abertas depois desta atualização passam
a ler a regra nova.
