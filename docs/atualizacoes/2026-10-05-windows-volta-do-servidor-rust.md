---
id: 2026-10-05-windows-volta-do-servidor-rust
titulo: No Windows, voltar da versão de teste com servidor em Rust para a principal funciona
destrutivo: false
---

Nada a rodar nesta máquina. No Windows, atualizar da versão de teste (a que tem o servidor em
Rust) de volta para a principal parava em "Porta 8765 ocupada por outro processo" e voltava para
a versão anterior. Agora o reinício encerra também o servidor em Rust e a atualização termina.
