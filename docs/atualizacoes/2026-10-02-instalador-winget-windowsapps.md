---
id: 2026-10-02-instalador-winget-windowsapps
titulo: Instalador do Windows acha o winget quando a pasta WindowsApps saiu do PATH
destrutivo: false
---

Nada a rodar nesta máquina: o que mudou é o próprio instalador, nas próximas execuções dele.
Num Windows cujo PATH de usuário perdeu a pasta `WindowsApps`, onde mora o atalho do winget, o
instalador dizia "winget nao encontrado" mesmo com o App Installer instalado, e a atualização do
servidor parava no primeiro passo. Agora ele acrescenta essa pasta ao PATH da própria execução,
sem mexer no registro.
