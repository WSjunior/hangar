import { describe, expect, it, vi } from 'vitest';
import { FieldSync, InputOutbox, fieldSender } from './pluginField';

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
    // O mod não ecoou: o desenho anterior também era `""`, e mesmo assim a resposta espera o blur.
    const sync = new FieldSync('');
    for (const v of ['a', '', 'x']) sync.typed(v);
    sync.submitted();
    expect(sync.draw('', 'x', true)).toBeNull();
    expect(sync.draw(null, 'x', true)).toBeNull();
    expect(sync.draw(null, 'x', false)).toBe('');
  });

  it('redesenho sem mudança do valor desenhado não cria pendente (mod que não ecoa o value)', () => {
    // O mod desenha `""` e não devolve o que se digita; outro mod redesenha a faixa no meio da digitação.
    const sync = new FieldSync('');
    sync.typed('abc');
    expect(sync.draw('', 'abc', true)).toBeNull();
    // A pessoa clica fora: o texto digitado fica.
    expect(sync.draw(null, 'abc', false)).toBeNull();
    // Valor que mudou em relação ao desenho anterior continua pendente e entra no blur.
    expect(sync.draw('x', 'abc', true)).toBeNull();
    expect(sync.draw(null, 'abc', false)).toBe('x');
  });

  it('redesenho sem mudança mantém o pendente que já havia', () => {
    const sync = new FieldSync('');
    expect(sync.draw('ab', 'abc', true)).toBeNull();
    expect(sync.draw('ab', 'abc', true)).toBeNull();
    expect(sync.draw(null, 'abc', false)).toBe('ab');
  });

  it('sem desenho anterior, o primeiro valor conta como mudança', () => {
    const sync = new FieldSync();
    expect(sync.draw('', 'abc', true)).toBeNull();
    expect(sync.draw(null, 'abc', false)).toBe('');
  });

  it('a resposta ao envio igual ao desenho anterior entra mesmo assim', () => {
    // O mod limpa o campo com o mesmo vazio que já desenhava: a vez da resposta não depende de o valor mudar.
    const sync = new FieldSync('');
    sync.typed('abc');
    sync.submitted();
    expect(sync.draw('', 'abc', true)).toBe('');
  });

  it('o foco que sai para o rótulo de envio descarta o pendente sem aplicá-lo', () => {
    const sync = new FieldSync();
    expect(sync.draw('ab', 'abc', true)).toBeNull();
    sync.leftToSend();
    expect(sync.draw(null, 'abc', false)).toBeNull();
  });
});

// Os mesmos casos do `Outbox` em `desktop-native/src/plugin_ui.rs`.
describe('InputOutbox', () => {
  it('um pedido em voo por vez; enquanto ele voa, só o change mais recente fica guardado', () => {
    const box = new InputOutbox();
    expect(box.push('change', 'a')).toEqual({ kind: 'change', value: 'a' });
    expect(box.push('change', 'ab')).toBeNull();
    expect(box.push('change', 'abc')).toBeNull();
    expect(box.done()).toEqual({ kind: 'change', value: 'abc' });
    expect(box.done()).toBeNull();
    // Livre de novo: o próximo sai na hora.
    expect(box.push('change', 'abcd')).toEqual({ kind: 'change', value: 'abcd' });
  });

  it('o submit sai depois dos change pendentes, e o que se digita depois dele sai depois', () => {
    const box = new InputOutbox();
    expect(box.push('change', 'a')).toEqual({ kind: 'change', value: 'a' });
    box.push('change', 'ab');
    box.push('submit', 'ab');
    box.push('change', 'abc');
    box.push('change', 'abcd');
    expect(box.done()).toEqual({ kind: 'change', value: 'ab' });
    expect(box.done()).toEqual({ kind: 'submit', value: 'ab' });
    expect(box.done()).toEqual({ kind: 'change', value: 'abcd' });
    expect(box.done()).toBeNull();
  });

  it('dois submit seguidos saem os dois, na ordem', () => {
    const box = new InputOutbox();
    box.push('submit', 'a');
    box.push('submit', 'b');
    box.push('submit', 'c');
    expect(box.done()).toEqual({ kind: 'submit', value: 'b' });
    expect(box.done()).toEqual({ kind: 'submit', value: 'c' });
    expect(box.done()).toBeNull();
  });
});

describe('fieldSender', () => {
  // Transporte falso: cada pedido demora o que `delays` disser, e os de depois respondem mais rápido que os de antes.
  function transport(delays: number[]) {
    const calls: string[] = [];
    let inFlight = 0;
    let maxInFlight = 0;
    let n = 0;
    const send = (kind: string, value: string) => {
      calls.push(`${kind}:${value}`);
      inFlight++;
      maxInFlight = Math.max(maxInFlight, inFlight);
      const ms = delays[n++] ?? 0;
      return new Promise<void>((resolve, reject) => setTimeout(() => {
        inFlight--;
        if (value === 'falha') reject(new Error('falha')); else resolve();
      }, ms));
    };
    return { send, calls, maxInFlight: () => maxInFlight };
  }

  it('as teclas chegam ao mod na ordem, com um pedido em voo por vez, mesmo com respostas fora de ordem', async () => {
    vi.useFakeTimers();
    try {
      const t = transport([50, 10, 5, 1]);
      const input = fieldSender(t.send, () => {});
      input('change', 'a');
      input('change', 'ab');
      input('change', 'abc');
      input('submit', 'abc');
      input('change', 'abcd');
      await vi.runAllTimersAsync();
      expect(t.calls).toEqual(['change:a', 'change:abc', 'submit:abc', 'change:abcd']);
      expect(t.maxInFlight()).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('uma falha avisa e não trava a fila', async () => {
    vi.useFakeTimers();
    try {
      const t = transport([10, 10]);
      const errors: unknown[] = [];
      const input = fieldSender(t.send, (err) => errors.push(err));
      input('change', 'falha');
      input('submit', 'ok');
      await vi.runAllTimersAsync();
      expect(t.calls).toEqual(['change:falha', 'submit:ok']);
      expect(errors).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
