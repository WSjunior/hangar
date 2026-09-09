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
import { existsSync, readFileSync } from 'node:fs';
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
//
// A pergunta é por CONTEÚDO, não por data. Comparar o mtime do package.json com o da marca do npm
// foi a primeira tentativa e não serve: qualquer restauração de node_modules por cache, tar ou
// rsync grava a data de AGORA na marca, que passa a parecer mais nova que um package.json mais
// recente — e aí o portão dá verde justamente no caso que ele existe para pegar. Data responde
// "quem foi escrito por último"; o que importa é "o que está declarado está instalado".
const modulos = join(app, 'node_modules');
const manifesto = join(app, 'package.json');

function avisar(motivo) {
  console.error(`\n  ${motivo}, então "${script}" NÃO rodou no app nativo.`);
  console.error('  Resolva com:  cd mobile && npm install');
  console.error('  (o app não é workspace da raiz de propósito — o build dele é no Expo)\n');
  process.exit(1);
}

if (!existsSync(modulos)) avisar('As dependências do app nativo não estão instaladas');
if (!existsSync(manifesto)) avisar(`Não achei o package.json do app (${manifesto})`);

let declaradas;
try {
  const pkg = JSON.parse(readFileSync(manifesto, 'utf8'));
  declaradas = Object.keys({ ...pkg.dependencies, ...pkg.devDependencies });
} catch (e) {
  avisar(`O package.json do app não pôde ser lido (${e.message})`);
}

// Só as diretas: peer e optional podem legitimamente não estar lá, e acusá-las seria falso alarme.
const faltando = declaradas.filter((nome) => !existsSync(join(modulos, ...nome.split('/'))));
if (faltando.length) {
  avisar(
    `${faltando.length} dependência(s) do app declarada(s) e não instalada(s) — ` +
      faltando.slice(0, 5).join(', ') +
      (faltando.length > 5 ? ', …' : ''),
  );
}

const r = spawnSync('npm', ['run', script], { cwd: app, stdio: 'inherit', shell: process.platform === 'win32' });
process.exit(r.status ?? 1);
