---
id: 2026-10-06-mods-do-hangar-por-plugin-dir
titulo: Os mods que vêm com o Hangar já chegam ligados em toda sessão Claude
comando_posix: ./scripts/install-claude-wrapper.sh
prova: ~/.local/bin/hangar-engine
destrutivo: false
---

Além do plugin do próprio Hangar, os outros mods da pasta `plugins/` do repositório passam a
entrar ligados em toda sessão Claude, sem o Claude Code perguntar e sem passar pelo marketplace.
Vale para as sessões abertas pelo app e para as abertas digitando `claude` no terminal.

O comando reinstala o atalho `claude` do fish, que é uma cópia; no bash, no zsh e no PowerShell
o atalho já é lido do repositório. Vale para sessões abertas depois da atualização, num
terminal novo.
