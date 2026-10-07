import type { CommandInfo } from './types';

// Sugestões enquanto a pessoa digita `/nome`: prefixo vence substring, no máximo `max`.
// `query` é o que veio depois da barra; null = o campo não é um comando sendo digitado.
export function slashMatches(commands: CommandInfo[], query: string | null, max = 8): CommandInfo[] {
  if (query === null) return [];
  const token = query.toLowerCase();
  return commands
    .map((c) => {
      const n = c.name.toLowerCase();
      return { c, r: !token ? 1 : n.startsWith(token) ? 0 : n.includes(token) ? 1 : -1 };
    })
    .filter((x) => x.r >= 0)
    .sort((a, b) => a.r - b.r)
    .slice(0, max)
    .map((x) => x.c);
}

// A palavra sob o cursor é um `/nome` sendo digitado, em qualquer ponto do texto, como o `@` de
// arquivos. `query` vai da barra até o cursor; `start..end` é a palavra inteira. `whole` diz se a
// palavra é a mensagem toda: só então escolher roteia o comando (envia, preenche, abre o seletor);
// no meio do texto, escolher só completa o nome.
export interface SlashToken { start: number; end: number; query: string; whole: boolean }

// Nome de comando ou skill: letra, dígito, `_`, `-`, `:` (plugin) e `.`. Sem segunda barra, para
// caminho colado (`/home/x/y.py`) e URL não abrirem a lista.
const NOME_COMANDO = /^[\w:.-]*$/;

const SPACE = /\s/;

export function slashTokenAt(text: string, cursor: number): SlashToken | null {
  if (cursor < 1 || cursor > text.length) return null;
  // Anda só pela palavra: um regex ancorado no fim do texto voltaria sobre todo bloco sem espaço
  // (JSON, base64) a cada tecla.
  let start = cursor;
  while (start > 0 && !SPACE.test(text[start - 1])) start--;
  if (start === cursor || text[start] !== '/') return null;
  let end = cursor;
  while (end < text.length && !SPACE.test(text[end])) end++;
  if (!NOME_COMANDO.test(text.slice(start + 1, end))) return null;
  const whole = !/\S/.test(text.slice(0, start)) && !/\S/.test(text.slice(end));
  return { start, end, query: text.slice(start + 1, cursor), whole };
}

// Troca a palavra do token por `/nome ` e devolve o texto novo e onde o cursor fica. Um espaço que
// já vinha depois é reaproveitado: o cursor sai da palavra, e a lista não reabre com o nome pronto.
export function replaceSlashToken(text: string, token: SlashToken, name: string): { text: string; cursor: number } {
  const end = text[token.end] === ' ' ? token.end + 1 : token.end;
  const insert = `/${name} `;
  return { text: text.slice(0, token.start) + insert + text.slice(end), cursor: token.start + insert.length };
}

// `/btw <pergunta>` abre a pergunta lateral em vez de virar mensagem. Devolve a pergunta (pode ser
// vazia) ou null quando o texto não é esse comando.
export function sideQuestionOf(text: string): string | null {
  const btw = /^\/btw(?:\s+([\s\S]*))?$/i.exec(text.trim());
  return btw ? (btw[1] ?? '').trim() : null;
}
