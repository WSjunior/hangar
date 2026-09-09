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
import { existsSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const script = process.argv[2];
if (script !== 'typecheck' && script !== 'test') {
  console.error('uso: node scripts/verificar-app.mjs <typecheck|test>');
  process.exit(2);
}

const raiz = dirname(dirname(fileURLToPath(import.meta.url)));
const app = join(raiz, 'mobile');

// Duas perguntas, não uma. "A pasta existe?" deixava passar o caso mais comum no dia a dia:
// alguém adiciona uma dependência no package.json e não reinstala. Aí o `tsc` sai 0 enquanto
// nada ainda importa o pacote novo, e o check fica verde escondendo a instalação atrasada —
// que é o mesmo defeito que este arquivo existe para evitar, um degrau abaixo.
// A marca é `node_modules/.package-lock.json`, que o npm reescreve a CADA instalação; a data da
// pasta `node_modules` não acompanha o que acontece dentro dela. Mesmo truque do install.sh.
const marca = join(app, 'node_modules', '.package-lock.json');
const manifesto = join(app, 'package.json');

function avisar(motivo) {
  console.error(`\n  ${motivo}, então "${script}" NÃO rodou no app nativo.`);
  console.error('  Resolva com:  cd mobile && npm install');
  console.error('  (o app não é workspace da raiz de propósito — o build dele é no Expo)\n');
  process.exit(1);
}

if (!existsSync(join(app, 'node_modules')) || !existsSync(marca)) {
  avisar('As dependências do app nativo não estão instaladas');
}
if (statSync(manifesto).mtimeMs > statSync(marca).mtimeMs) {
  avisar('O package.json do app mudou depois da última instalação');
}

const r = spawnSync('npm', ['run', script], { cwd: app, stdio: 'inherit', shell: process.platform === 'win32' });
process.exit(r.status ?? 1);
