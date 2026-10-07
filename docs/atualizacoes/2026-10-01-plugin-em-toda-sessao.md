---
id: 2026-10-01-plugin-em-toda-sessao
titulo: O plugin do Hangar passa a carregar em toda sessão do Claude, inclusive a aberta no terminal
comando_posix: ./scripts/install-hangar-send.sh
comando_windows: powershell -ExecutionPolicy Bypass -File install.ps1 -Update
prova: ~/.local/bin/hangar-send
destrutivo: false
---

O plugin do Hangar fica na pasta de skills do Claude e o Claude Code (2.1.287 ou mais novo) o
carrega sozinho em toda sessão. Mensagens do app passam a entrar sem simular teclado também no
`claude` aberto pelo terminal. Quando o plugin não responde, o Hangar volta ao caminho de antes, e
a opção "Caminho do plugin do Hangar" desliga tudo isso. Rodar a atualização de novo mantém o link
que já aponta para o plugin. Uma pasta `hangar` nas skills que não seja o plugin fica intacta, e a
atualização avisa em vez de apagá-la.
