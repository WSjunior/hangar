import { describe, expect, it } from 'vitest';
import { FieldSync } from './pluginField';

// Os mesmos casos dos testes do `FieldSync` em `desktop-native/src/plugin_ui.rs`: a regra é uma só nos dois apps.
describe('FieldSync', () => {
  it('redesenho em foco mantém o texto digitado, e o blur aplica o pendente', () => {
    const sync = new FieldSync();
    // Desenho novo do mod com a pessoa digitando: o texto fica, o valor fica pendente.
    expect(sync.draw('ab', 'abc', true)).toBeNull();
    // Redesenho do app ainda em foco: nada muda.
    expect(sync.draw(null, 'abc', true)).toBeNull();
    // Fora de foco, o pendente entra uma vez só.
    expect(sync.draw(null, 'abc', false)).toBe('ab');
    expect(sync.draw(null, 'ab', false)).toBeNull();
    // O desenho que chega com o campo fora de foco entra na hora; igual ao que se vê, nada a fazer.
    expect(sync.draw('x', 'ab', false)).toBe('x');
    expect(sync.draw('x', 'x', false)).toBeNull();
  });

  it('digitar depois que o pendente chegou o descarta, e o blur mantém o texto digitado', () => {
    const sync = new FieldSync();
    expect(sync.draw('ab', 'abc', true)).toBeNull();
    sync.typed('abcd');
    // Fora de foco, o texto digitado e não enviado fica.
    expect(sync.draw(null, 'abcd', false)).toBeNull();
    // Um desenho novo depois disso volta a ficar pendente e entra ao perder o foco.
    expect(sync.draw('x', 'abcd', true)).toBeNull();
    expect(sync.draw(null, 'abcd', false)).toBe('x');
  });

  it('o redesenho logo depois do próprio envio entra mesmo com foco', () => {
    // É assim que o mod limpa o campo depois do envio: o valor desenhado é o mesmo de antes (vazio), mas entra.
    let sync = new FieldSync();
    sync.submitted();
    expect(sync.draw('', 'abc', true)).toBe('');
    // Só o primeiro: o seguinte, em foco, volta a esperar.
    expect(sync.draw('zz', 'd', true)).toBeNull();
    // O eco da digitação que chega depois do Enter (igual ao que se vê) não gasta a vez da resposta ao envio.
    sync = new FieldSync();
    sync.submitted();
    expect(sync.draw('abc', 'abc', true)).toBeNull();
    expect(sync.draw('', 'abc', true)).toBe('');
    // Um eco atrasado que chega antes do Enter fica pendente e não atrapalha a resposta ao envio.
    sync = new FieldSync();
    expect(sync.draw('ab', 'abc', true)).toBeNull();
    sync.submitted();
    // Esse eco não volta ao perder o foco antes da resposta: o texto enviado fica.
    expect(sync.draw(null, 'abc', false)).toBeNull();
    expect(sync.draw('', 'abc', true)).toBe('');
    // Digitar de novo fecha a vez: o desenho seguinte não apaga o que se digita.
    sync = new FieldSync();
    sync.submitted();
    sync.typed('abcd');
    expect(sync.draw('', 'abcd', true)).toBeNull();
  });

  it('depois do envio, os ecos atrasados dos change mandados não tomam a vez da resposta', () => {
    // `a`, `ab` e `abc` vão como `change` com o eco atrasado, e o Enter sai antes de qualquer eco.
    const typed = (sync: FieldSync) => { for (const v of ['a', 'ab', 'abc']) sync.typed(v); };
    let sync = new FieldSync();
    typed(sync);
    sync.submitted();
    // Os ecos de antes do envio chegam um a um: o campo segue com o texto enviado.
    expect(sync.draw('a', 'abc', true)).toBeNull();
    expect(sync.draw('ab', 'abc', true)).toBeNull();
    // A resposta, fora do que foi mandado, entra mesmo com foco.
    expect(sync.draw('', 'abc', true)).toBe('');
    // Ecos e resposta no mesmo desenho: vale o último valor, que é a resposta.
    sync = new FieldSync();
    typed(sync);
    sync.submitted();
    expect(sync.draw('', 'abc', true)).toBe('');
    // O histórico zera quando um valor entra: depois da resposta, `a` volta a ser um valor como outro qualquer.
    sync.submitted();
    expect(sync.draw('a', '', true)).toBe('a');
  });

  it('limite conhecido: resposta igual a um valor digitado antes espera o blur', () => {
    // A pessoa apagou tudo (`change ""`) antes de digitar `x`. A resposta `""` ao envio é igual a um valor mandado,
    // então passa por eco velho, fica pendente em foco e só entra quando o campo perde o foco.
    const sync = new FieldSync();
    for (const v of ['a', '', 'x']) sync.typed(v);
    sync.submitted();
    expect(sync.draw('', 'x', true)).toBeNull();
    expect(sync.draw(null, 'x', true)).toBeNull();
    expect(sync.draw(null, 'x', false)).toBe('');
  });

  it('o foco que sai para o rótulo de envio descarta o pendente sem aplicá-lo', () => {
    const sync = new FieldSync();
    expect(sync.draw('ab', 'abc', true)).toBeNull();
    sync.leftToSend();
    expect(sync.draw(null, 'abc', false)).toBeNull();
  });
});
