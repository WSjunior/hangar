#!/usr/bin/env node
// Hook (PreToolUse, Bash) — recusa `gh pr create` cuja ponta não passou no scripts/verificar-local.
//
// O PR inteiro conta: compara a ponta com o ponto em que ela saiu da base do PR, não só com o último
// push. PR aberto pelo site não passa por aqui; quem confere esse é o CI.
// Escape explícito, o mesmo do pre-push:  HANGAR_SEM_VERIFICACAO=1 gh pr create ...
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

let dados;
try {
  dados = JSON.parse(readFileSync(0, 'utf8'));
} catch {
  process.exit(0);
}
// Quebra o comando em trechos simples (; & | ( e quebra de linha fora de aspas) e procura o trecho
// que É a chamada: `gh pr create`, com variáveis antes. Texto entre aspas (mensagem de commit,
// argumento de printf) que cite o comando não conta.
function trechos(texto) {
  const saida = [];
  let atual = '', aspas = '';
  for (let i = 0; i < texto.length; i++) {
    const c = texto[i];
    if (aspas) {
      if (c === '\\' && aspas === '"') { atual += c + (texto[++i] ?? ''); continue; }
      if (c === aspas) aspas = '';
      atual += c;
    } else if (c === "'" || c === '"') {
      aspas = c; atual += c;
    } else if (c === '\\') {
      atual += c + (texto[++i] ?? '');
    } else if (';&|(\n'.includes(c)) {
      saida.push(atual); atual = '';
    } else {
      atual += c;
    }
  }
  saida.push(atual);
  return saida;
}
const achado = trechos(dados?.tool_input?.command ?? '')
  .map(t => t.match(/^\s*((?:\w+=\S*\s+)*)gh\s+pr\s+create\b(.*)$/s))
  .find(Boolean);
if (!achado) process.exit(0);
const [, prefixo, comando] = achado;
if (process.env.HANGAR_SEM_VERIFICACAO || /\bHANGAR_SEM_VERIFICACAO=1\b/.test(prefixo)) process.exit(0);
// Como o pre-push: a verificação só é exigida no Linux.
if (process.platform !== 'linux') process.exit(0);

const git = (cwd, ...args) => execFileSync('git', ['-C', cwd, ...args], { encoding: 'utf8' }).trim();
const opcao = (curta, longa) => comando.match(new RegExp(`(?:^|\\s)(?:${curta}|${longa})[ =]+['"]?([^'"\\s]+)`))?.[1];

let raiz;
try {
  raiz = git(dados.cwd || process.cwd(), 'rev-parse', '--show-toplevel');
} catch {
  process.exit(0);
}
const script = join(raiz, 'scripts', 'verificar-local');
if (!existsSync(script)) process.exit(0);

const head = opcao('-H', '--head');
const base = opcao('-B', '--base');
const resolver = (...refs) => {
  for (const ref of refs) {
    try {
      return git(raiz, 'rev-parse', '--verify', '--quiet', `${ref}^{commit}`);
    } catch {}
  }
  return null;
};
const ponta = head ? resolver(`origin/${head}`, head) : resolver('HEAD');
let padrao = 'origin/main';
try {
  padrao = git(raiz, 'rev-parse', '--abbrev-ref', 'origin/HEAD');
} catch {}
const alvo = base ? `origin/${base}` : padrao;
if (!ponta || !resolver(alvo)) {
  console.error(`[pr] não consegui resolver a ponta (${head || 'HEAD'}) ou a base (${alvo}) do PR para conferir a verificação local.\n` +
    '     Abra com a base e a branch existentes, ou, sabendo o que faz:  HANGAR_SEM_VERIFICACAO=1 gh pr create ...');
  process.exit(2);
}

const r = spawnSync(script, ['--base', alvo, '--exigir', ponta], { cwd: raiz, encoding: 'utf8' });
if (r.status === 0) process.exit(0);
console.error(
  `[pr] o PR ainda não passou no verificar-local (ponta ${ponta.slice(0, 10)}, desde ${alvo}).\n` +
    (r.stderr || '').trimEnd() + '\n' +
    `     Rode:  scripts/verificar-local --commit ${ponta.slice(0, 10)} --base ${alvo}\n` +
    '     Emergência, sabendo o que faz:  HANGAR_SEM_VERIFICACAO=1 gh pr create ...',
);
process.exit(2);
