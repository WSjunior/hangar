import { describe, it, expect } from 'vitest';
import { parseHash } from './route';

describe('parseHash ignora o eixo do painel de configuracoes', () => {
  // Sem o corte da query, `path === '/board'` falha e a rota cai em `sessions`: abrir Configuracoes
  // TELEPORTAVA o usuario do quadro pra lista.
  it('#/board com painel aberto continua sendo o quadro', () => {
    expect(parseHash('#/board?config=aparencia')).toEqual({ name: 'board', sessionName: null, serverId: null });
  });
  it('#/canvas com painel aberto continua sendo o canvas', () => {
    expect(parseHash('#/canvas?config=motores&srv=a')).toEqual({ name: 'canvas', sessionName: null, serverId: null });
  });
  // Sem o corte, o regex ganancioso de chat capturava "x?config=motores" como NOME DA SESSAO: a
  // {#key} mudava e o Chat remontava (SSE derrubado) a cada abrir/fechar, e o SSE ia pra
  // /api/sessions/x%3Fconfig=motores/events -> 404 em loop.
  it('nome da sessao nao engole a query', () => {
    expect(parseHash('#/chat/127/x?config=motores&srv=127')).toEqual({ name: 'chat', sessionName: 'x', serverId: '127' });
  });
  it('chat na forma legada tambem', () => {
    expect(parseHash('#/chat/x?config=root')).toEqual({ name: 'chat', sessionName: 'x', serverId: null });
  });
  it('#/costs com painel aberto continua sendo custos', () => {
    expect(parseHash('#/costs?config=avancado')).toEqual({ name: 'costs' });
  });
  it('#/uso é a tela de uso de skills e tools', () => {
    expect(parseHash('#/uso')).toEqual({ name: 'uso' });
    expect(parseHash('#/uso?config=avancado')).toEqual({ name: 'uso' });
  });
});

describe('parseHash sem painel segue como era', () => {
  it('raiz vira a tela inicial', () => {
    expect(parseHash('#/')).toEqual({ name: 'home' });
    expect(parseHash('')).toEqual({ name: 'home' });
    expect(parseHash('#/?config=aparencia')).toEqual({ name: 'home' });
  });
  it('#/sessions é a lista e rota desconhecida também cai nela', () => {
    expect(parseHash('#/sessions')).toEqual({ name: 'sessions' });
    expect(parseHash('#/xyz')).toEqual({ name: 'sessions' });
  });
  it('auto-cura de nome invalido continua valendo', () => {
    expect(parseHash('#/chat/undefined')).toEqual({ name: 'sessions' });
    expect(parseHash('#/chat/null')).toEqual({ name: 'sessions' });
  });
  it('worktrees', () => { expect(parseHash('#/worktrees')).toEqual({ name: 'worktrees' }); });
  it('overlay do quadro continua lendo servidor e sessao', () => {
    expect(parseHash('#/board/127/minha')).toEqual({ name: 'board', sessionName: 'minha', serverId: '127' });
  });
});
