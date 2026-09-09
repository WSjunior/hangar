#!/usr/bin/env node
// Hook (PostToolUse) — avisa quando `packages/core` é editado.
//
// O core é a única parte do repositório consumida pelas DUAS interfaces: o front web em Svelte e o
// app nativo em Expo. Editar um arquivo dele e verificar só o front é o erro que este aviso existe
// para pegar — `npm run check -w frontend` fica verde com o app quebrado, porque nenhum dos dois
// comandos cobre o outro.
//
// Só avisa. Bloquear a edição seria pior: o caminho normal é editar e depois verificar.
import { readFileSync } from 'node:fs';

let entrada = '';
try {
  entrada = readFileSync(0, 'utf8');
} catch {
  process.exit(0);
}

let caminho = '';
try {
  caminho = JSON.parse(entrada)?.tool_input?.file_path ?? '';
} catch {
  process.exit(0);
}

// Barra normal e invertida: no Windows o caminho vem com `\`.
if (!/packages[/\\]core[/\\]/.test(caminho)) process.exit(0);
// Gerado pelo paraglide a cada compilação; editá-lo não é mexer no core.
if (/[/\\]paraglide[/\\]/.test(caminho)) process.exit(0);

console.error(
  '[core] Este arquivo é consumido pelo front web E pelo app nativo.\n' +
    '       Verifique os dois antes de dar por pronto:  npm run check   (na raiz, cobre os três)',
);
process.exit(0);
