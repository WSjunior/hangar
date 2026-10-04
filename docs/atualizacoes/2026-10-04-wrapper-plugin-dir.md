---
id: 2026-10-04-wrapper-plugin-dir
titulo: Sessão Claude aberta pelo terminal mostra no app a faixa dos mods
comando_posix: ./scripts/install-claude-wrapper.sh
prova: ~/.local/bin/hangar-engine
destrutivo: false
---

A faixa que os mods do Claude Code desenham acima do prompt (a barra de progresso de um plano,
por exemplo) só aparecia no app nas sessões criadas pelo próprio Hangar. Agora a sessão aberta
digitando `claude` no terminal também a mostra.

Vale para sessões abertas depois da atualização, num terminal novo. A que já estava aberta
continua sem a faixa até ser reaberta.
