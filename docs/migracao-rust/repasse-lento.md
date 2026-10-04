# Repasse lento do hangar-server ao Python

Pedido: `pedidos/2026-10-04-repasse-lento-kickoff.md`. Branch `fix/proxy-latency`.

## Causa

Nenhum socket TCP do `hangar-server` ligava `TCP_NODELAY`. Numa resposta enviada em pedaços,
o último pedaço pequeno (no chunked, o terminador `0\r\n\r\n`) fica preso pelo algoritmo de
Nagle até o cliente confirmar o anterior, e o cliente atrasa essa confirmação (ACK atrasado:
40 ms no Linux, até 200 ms no Windows). O asyncio do Python liga `TCP_NODELAY` sozinho em toda
conexão, por isso o mesmo pedido direto no Python não pagava.

Depende do ritmo da leitura do cliente: o `curl` (buffer de 100 KB) trava quase sempre; o
`http.client` do Python, quase nunca. Por isso o teste lê como o `curl`.

## Conserto

`crate::nodelay` (`crates/hangar-server/src/lib.rs`) aplicado a todo socket do servidor:
porta pública e porta privada da observação (`routes.rs`, `tap_io`), porta do runtime
(`runtime/gateway.rs`), conexões do repasse ao Python (`HttpConnector::set_nodelay`), a
conexão própria do WebSocket (`proxy.rs`) e a do cano por TCP (`runtime/cano.rs`).
Teste: `large_chunked_body_is_not_held_by_nagle` em `crates/hangar-server/tests/proxy.rs` —
sem o conserto 45,9 ms contra 4,3 ms direto; com ele passa.

## Medições (04/10, i5-13400F, servidor de teste isolado)

`hangar-server` desta branch na porta 18900 repassando a um FastAPI de teste na 18901.
Mediana de 7 pedidos `curl` (o primeiro de 8 descartado), conexão nova a cada pedido.

| Resposta | Antes: Rust | Depois: Rust | Python direto |
|---|---|---|---|
| 1 KB, Content-Length | 0,7 ms | 0,5 ms | 0,4–0,8 ms |
| 100 KB, Content-Length | 0,9 ms | 0,6 ms | 0,5–0,6 ms |
| 1 MB, Content-Length | 1,6 ms | 1,5 ms | 1,1–1,5 ms |
| 4 MB, Content-Length | 4,1 ms | 4,4 ms | 3,7–3,8 ms |
| 1 MB chunked, pedaços de 16 KB | 1,1 ms | 1,1 ms | 0,7–0,9 ms |
| 1 MB chunked, pedaços de 1 KB | **44,2 ms** | 4,9 ms | 1,7–2,0 ms |
| 200 KB chunked, pedaços de 1 KB | **42,3 ms** | 1,8 ms | 0,7–1,3 ms |
| 20 KB chunked, pedaços de 1 KB | 0,8 ms | 0,8 ms | 0,5 ms |
| upload 5 MB, Content-Length | 2,1 ms | 2,0 ms | 1,6–1,7 ms |
| upload 5 MB, chunked | 3,5 ms | 2,0 ms | 1,7–1,8 ms |

Content-Length com o corpo chegando em pedaços (1 KB a 64 KB, 937 KB no total) e Rust e
Python presos no mesmo núcleo não reproduziram o atraso aqui, antes nem depois. SSE repassado
(6 eventos a cada 50 ms) chega no ritmo certo antes e depois; o upload chega inteiro.

No backend real desta máquina (só leitura), `/api/costs` (939 KB) com `--compressed`, que é
como o navegador pede e sai em chunked pelo gzip do Python: 6 de 7 pedidos em ~9 ms nos dois
lados e um em 120 ms só pelo Rust. Sem gzip, ~3 ms nos dois. O notebook do dono (106 ms × 12 ms
sem gzip) não foi medido de novo: falta rodar `scripts/medir-rust.sh` lá com o binário novo.
