#!/usr/bin/env node
// Roda `typecheck` ou `test` do app nativo a partir da RAIZ.
//
// Existe porque `mobile/` não é workspace (para um `npm ci` na raiz não baixar o toolchain do
// React Native), então `npm run <script> -w mobile` não funciona e o app ficava fora do
// `npm run check`/`npm run test` da raiz — que é como uma mudança no `@hangar/core` podia quebrar
// o app com o verde na mão.
//
// A regra que dá sentido a ele: dependência do app ausente FALHA, nunca é pulada em silêncio. Um
// "check" que passa por não ter olhado é pior que um que não roda.
import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const script = process.argv[2];
if (script !== 'typecheck' && script !== 'test') {
  console.error('uso: node scripts/verificar-app.mjs <typecheck|test>');
  process.exit(2);
}

const raiz = dirname(dirname(fileURLToPath(import.meta.url)));
const app = join(raiz, 'mobile');

if (!existsSync(join(app, 'node_modules'))) {
  console.error(`\n  As dependências do app nativo não estão instaladas, então "${script}" NÃO rodou nele.`);
  console.error('  Instale com:  cd mobile && npm install');
  console.error('  (o app não é workspace da raiz de propósito — o build dele é no Expo)\n');
  process.exit(1);
}

const r = spawnSync('npm', ['run', script], { cwd: app, stdio: 'inherit', shell: process.platform === 'win32' });
process.exit(r.status ?? 1);
