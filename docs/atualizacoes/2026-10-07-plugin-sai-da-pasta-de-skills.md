---
id: 2026-10-07-plugin-sai-da-pasta-de-skills
titulo: O aviso "hangar is already taken" some do /plugin do Claude Code
comando_posix: ./scripts/install-hangar-send.sh
comando_windows: powershell -ExecutionPolicy Bypass -File install.ps1 -Update
prova: ~/.local/bin/hangar-send
destrutivo: false
---

O plugin do Hangar chegava às sessões por dois caminhos ao mesmo tempo, e o Claude Code acusava o
segundo como erro em todo `/plugin`. Agora ele chega só pelo caminho que o Hangar e o atalho
`claude` do terminal já usavam, e o link antigo em `~/.claude/skills/hangar` é apagado. Quem abre o
Claude com `command claude`, pulando o atalho, deixa de receber o plugin.
