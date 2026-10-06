/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import type { RecordingStatus } from 'expo-audio';

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const { fakeRec, createRecorder, requestPermission, setAudioMode, appState, recorderStatus, removeStatusListener } = vi.hoisted(() => {
  const fakeRec: {
    uri: string | null;
    stop: ReturnType<typeof vi.fn>;
    prepareToRecordAsync: ReturnType<typeof vi.fn>;
    record: ReturnType<typeof vi.fn>;
    getStatus: ReturnType<typeof vi.fn>;
    release: ReturnType<typeof vi.fn>;
    addListener: ReturnType<typeof vi.fn>;
  } = {
    uri: 'file://fake.m4a',
    stop: vi.fn(async () => {}),
    prepareToRecordAsync: vi.fn(async () => {}),
    record: vi.fn(() => {}),
    getStatus: vi.fn(() => ({ metering: -160 })),
    release: vi.fn(),
    addListener: vi.fn(),
  };
  return {
    fakeRec,
    createRecorder: vi.fn(function () { return fakeRec; }),
    requestPermission: vi.fn(async () => ({ granted: true })),
    setAudioMode: vi.fn(async () => {}),
    appState: { currentState: 'active', listener: null as ((next: string) => void) | null },
    recorderStatus: { listener: null as ((status: RecordingStatus) => void) | null },
    removeStatusListener: vi.fn(),
  };
});

vi.mock('expo-audio', () => ({
  AudioModule: { AudioRecorder: createRecorder },
  RecordingPresets: { HIGH_QUALITY: { extension: '.m4a', android: { outputFormat: 'mpeg4', audioEncoder: 'aac' } } },
  requestRecordingPermissionsAsync: requestPermission,
  setAudioModeAsync: setAudioMode,
}));

vi.mock('react-native', async () => {
  const actual = (await vi.importActual<typeof import('../../__mocks__/react-native')>('../../__mocks__/react-native')) as Record<string, unknown>;
  return {
    ...actual,
    AppState: {
      addEventListener: vi.fn((_event: string, listener: (next: string) => void) => {
        appState.listener = listener;
        return { remove: () => { appState.listener = null; } };
      }),
      get currentState() { return appState.currentState; },
    },
  };
});

import { useDitado } from './useDitado';
import type { MotivoFim } from '@hangar/core';

function mountHook(opts: { onFim: (f: File, m: MotivoFim, uri: string) => void; onErroParada?: (e: Error) => void }) {
  let hook: ReturnType<typeof useDitado> | null = null;
  function Comp(p: typeof opts) {
    hook = useDitado(p);
    return null;
  }
  const container = document.createElement('div');
  document.body.appendChild(container);
  const root = createRoot(container);
  return {
    // monta e atualiza hook via render
    mount: async (props: typeof opts) => {
      await act(async () => {
        root.render(React.createElement(Comp, props));
      });
      if (!hook) throw new Error('hook not mounted');
    },
    getHook: () => {
      if (!hook) throw new Error('hook not mounted');
      return hook;
    },
    unmount: async () => {
      await act(async () => {
        root.unmount();
      });
      container.remove();
    },
  };
}

beforeEach(() => {
  vi.useFakeTimers();
  document.body.innerHTML = '';
  appState.currentState = 'active';
  appState.listener = null;
  createRecorder.mockClear();
  requestPermission.mockReset().mockResolvedValue({ granted: true });
  setAudioMode.mockReset().mockResolvedValue(undefined);
  recorderStatus.listener = null;
  removeStatusListener.mockClear();
  fakeRec.uri = 'file://fake.m4a';
  fakeRec.stop = vi.fn(async () => { emitRecordingStatus(); });
  fakeRec.prepareToRecordAsync = vi.fn(async () => {});
  fakeRec.record = vi.fn(() => {});
  fakeRec.getStatus = vi.fn(() => ({ metering: -160 }));
  fakeRec.release = vi.fn();
  fakeRec.addListener = vi.fn((_event: string, listener: (status: RecordingStatus) => void) => {
    recorderStatus.listener = listener;
    return { remove: () => { removeStatusListener(); recorderStatus.listener = null; } };
  });
  vi.stubGlobal('fetch', vi.fn(async () => ({ blob: async () => new Blob(['audio'], { type: 'audio/m4a' }) } as unknown as Response)));
});

afterEach(async () => {
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  vi.clearAllTimers();
  vi.useRealTimers();
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function changeAppState(next: string) {
  appState.currentState = next;
  appState.listener?.(next);
}

function emitRecordingStatus(status: Partial<RecordingStatus> = {}) {
  recorderStatus.listener?.({
    id: 'fake', isFinished: true, hasError: false, error: null, url: fakeRec.uri, ...status,
  });
}

describe('useDitado — parar() propaga falha via onErroParada', () => {
  it('stop() rejeita → onErroParada é chamado e onFim não', async () => {
    const onFim = vi.fn();
    const onErroParada = vi.fn();
    const h = mountHook({ onFim, onErroParada });
    await h.mount({ onFim, onErroParada });
    await act(async () => {
      await h.getHook().iniciar();
    });

    const erro = new Error('mic falhou');
    fakeRec.stop = vi.fn(async () => {
      throw erro;
    });

    await act(async () => {
      await h.getHook().parar('botao');
    });

    expect(onErroParada).toHaveBeenCalledTimes(1);
    expect(onErroParada.mock.calls[0][0]).toBe(erro);
    expect(onFim).not.toHaveBeenCalled();

    await h.unmount();
  });

  it('uri nulo → onErroParada é chamado e onFim não', async () => {
    const onFim = vi.fn();
    const onErroParada = vi.fn();
    const h = mountHook({ onFim, onErroParada });
    await h.mount({ onFim, onErroParada });
    await act(async () => {
      await h.getHook().iniciar();
    });

    fakeRec.uri = null;
    fakeRec.stop = vi.fn(async () => { emitRecordingStatus(); });

    await act(async () => {
      await h.getHook().parar('botao');
    });

    expect(onErroParada).toHaveBeenCalledTimes(1);
    expect(onErroParada.mock.calls[0][0].message).toBe('ditado_parada_falhou');
    expect(onFim).not.toHaveBeenCalled();

    await h.unmount();
  });

  it('parar com sucesso chama onFim e não chama onErroParada', async () => {
    const onFim = vi.fn();
    const onErroParada = vi.fn();
    const h = mountHook({ onFim, onErroParada });
    await h.mount({ onFim, onErroParada });
    await act(async () => {
      await h.getHook().iniciar();
    });

    fakeRec.uri = 'file://fake.m4a';
    fakeRec.stop = vi.fn(async () => { emitRecordingStatus(); });

    await act(async () => {
      await h.getHook().parar('botao');
    });

    expect(onFim).toHaveBeenCalledTimes(1);
    expect(onErroParada).not.toHaveBeenCalled();
    const [file, motivo, uri] = onFim.mock.calls[0];
    expect(file).toBeInstanceOf(File);
    expect(motivo).toBe('botao');
    expect(uri).toBe('file://fake.m4a');
    expect(fakeRec.release).toHaveBeenCalledTimes(1);
    expect(removeStatusListener).toHaveBeenCalledTimes(1);

    await h.unmount();
  });

  it('sobrevive a re-render com onErroParada inline (bloqueador 2)', async () => {
    const onFim = vi.fn();
    const onErro = vi.fn();
    const h = mountHook({ onFim, onErroParada: (e) => onErro(e) });
    await h.mount({ onFim, onErroParada: (e) => onErro(e) });
    await act(async () => {
      await h.getHook().iniciar();
    });
    // força UM re-render com NOVA inline arrow (nova identidade) — como o Composer faz a cada setRms/setText
    await h.mount({ onFim, onErroParada: (e) => onErro(e) });
    await act(async () => {
      await h.getHook().parar('botao');
    });
    expect(onFim).toHaveBeenCalledTimes(1);
    expect(onErro).not.toHaveBeenCalled();
    await h.unmount();
  });
});

describe('useDitado — interrupção e origem da gravação', () => {
  it('bloqueia preparação dupla e conserva o callback capturado antes da permissão', async () => {
    const permission = deferred<{ granted: boolean }>();
    requestPermission.mockReturnValue(permission.promise);
    const original = vi.fn();
    const replacement = vi.fn();
    const h = mountHook({ onFim: original });
    await h.mount({ onFim: original });
    let starting!: Promise<void>;
    await act(async () => {
      starting = h.getHook().iniciar();
      await h.getHook().iniciar();
    });
    expect(requestPermission).toHaveBeenCalledTimes(1);
    expect(createRecorder).not.toHaveBeenCalled();
    await h.mount({ onFim: replacement });
    await act(async () => {
      permission.resolve({ granted: true });
      await starting;
      await h.getHook().parar('botao');
    });
    expect(createRecorder).toHaveBeenCalledTimes(1);
    expect(fakeRec.prepareToRecordAsync).toHaveBeenCalledTimes(1);
    expect(original).toHaveBeenCalledTimes(1);
    expect(replacement).not.toHaveBeenCalled();
    await h.unmount();
  });

  it('permite o primeiro ditado após inactive transitório do pedido de permissão', async () => {
    const permission = deferred<{ granted: boolean }>();
    requestPermission.mockReturnValue(permission.promise);
    const onFim = vi.fn();
    const h = mountHook({ onFim });
    await h.mount({ onFim });
    let starting!: Promise<void>;
    await act(async () => {
      starting = h.getHook().iniciar();
      changeAppState('inactive');
      changeAppState('active');
      permission.resolve({ granted: true });
      await starting;
    });
    expect(fakeRec.record).toHaveBeenCalledTimes(1);
    await act(async () => { await h.getHook().parar('botao'); });
    expect(onFim).toHaveBeenCalledTimes(1);
    await h.unmount();
  });

  it.each([
    ['permission', 'unmount'],
    ['prepare', 'background'], ['prepare', 'unmount'], ['prepare', 'inactive'],
  ] as const)('cancela início pendente em %s após %s', async (stage, interruption) => {
    const permission = deferred<{ granted: boolean }>();
    const preparation = deferred<void>();
    if (stage === 'permission') requestPermission.mockReturnValue(permission.promise);
    else fakeRec.prepareToRecordAsync.mockReturnValue(preparation.promise);
    const onFim = vi.fn();
    const h = mountHook({ onFim });
    await h.mount({ onFim });
    let starting!: Promise<void>;
    await act(async () => { starting = h.getHook().iniciar(); });
    if (stage === 'prepare') expect(fakeRec.prepareToRecordAsync).toHaveBeenCalledTimes(1);
    if (interruption === 'unmount') await h.unmount();
    else await act(async () => {
      changeAppState(interruption === 'inactive' ? 'inactive' : 'background');
      changeAppState('active');
    });
    expect(fakeRec.release).not.toHaveBeenCalled();
    await act(async () => {
      permission.resolve({ granted: true });
      preparation.resolve();
      await starting;
    });
    expect(fakeRec.record).not.toHaveBeenCalled();
    expect(fakeRec.stop).not.toHaveBeenCalled();
    expect(onFim).not.toHaveBeenCalled();
    expect(fakeRec.release).toHaveBeenCalledTimes(stage === 'prepare' ? 1 : 0);
    if (interruption !== 'unmount') {
      await act(async () => {
        await h.getHook().iniciar();
        await h.getHook().parar('botao');
      });
      expect(fakeRec.record).toHaveBeenCalledTimes(1);
      await h.unmount();
    }
  });

  it.each([
    ['Android', 'background'], ['iOS', 'inactive'],
  ] as const)('%s: permissão resolvida antes do active inicia o primeiro ditado', async (_platform, suspended) => {
    const permission = deferred<{ granted: boolean }>();
    requestPermission.mockReturnValue(permission.promise);
    const onFim = vi.fn();
    const h = mountHook({ onFim });
    await h.mount({ onFim });
    let starting!: Promise<void>;
    await act(async () => {
      starting = h.getHook().iniciar();
      changeAppState(suspended);
      permission.resolve({ granted: true });
      await starting;
    });
    expect(fakeRec.record).toHaveBeenCalledTimes(1);
    expect(h.getHook().gravando).toBe(true);
    await act(async () => {
      changeAppState('active');
      await h.getHook().parar('botao');
    });
    expect(onFim).toHaveBeenCalledTimes(1);
    expect(onFim.mock.calls[0]?.[1]).toBe('botao');
    await h.unmount();
  });

  it('propaga falha de preparação, libera o objeto e permite outro início', async () => {
    const erro = new Error('prepare falhou');
    fakeRec.prepareToRecordAsync.mockRejectedValueOnce(erro);
    const onFim = vi.fn();
    const h = mountHook({ onFim });
    await h.mount({ onFim });
    await act(async () => {
      await expect(h.getHook().iniciar()).rejects.toBe(erro);
    });
    expect(h.getHook().gravando).toBe(false);
    expect(fakeRec.record).not.toHaveBeenCalled();
    expect(fakeRec.release).toHaveBeenCalledTimes(1);
    await act(async () => {
      await h.getHook().iniciar();
      await h.getHook().parar('botao');
    });
    expect(fakeRec.record).toHaveBeenCalledTimes(1);
    await h.unmount();
  });

  it.each(['background', 'unmount'] as const)('finaliza áudio após %s antes de liberar o recorder', async (interruption) => {
    const stop = deferred<void>();
    const delivered = deferred<void>();
    fakeRec.stop.mockReturnValue(stop.promise);
    const onFim = vi.fn(() => {
      expect(fakeRec.release).not.toHaveBeenCalled();
      delivered.resolve();
    });
    const h = mountHook({ onFim });
    await h.mount({ onFim });
    await act(async () => { await h.getHook().iniciar(); });
    if (interruption === 'unmount') await h.unmount();
    else await act(async () => { changeAppState('inactive'); });
    expect(fakeRec.stop).toHaveBeenCalledTimes(1);
    expect(fakeRec.release).not.toHaveBeenCalled();
    await act(async () => {
      stop.resolve();
      emitRecordingStatus();
      await delivered.promise;
    });
    expect(onFim.mock.calls[0]).toEqual([expect.any(File), 'escondeu', 'file://fake.m4a']);
    expect(fakeRec.release).toHaveBeenCalledTimes(1);
    if (interruption === 'background') await h.unmount();
  });

  it('não reinicia nem para duas vezes durante a parada e leitura do áudio', async () => {
    const stop = deferred<void>();
    const audio = deferred<Blob>();
    fakeRec.stop.mockReturnValue(stop.promise);
    vi.stubGlobal('fetch', vi.fn(async () => ({ blob: () => audio.promise })));
    const onFim = vi.fn();
    const h = mountHook({ onFim });
    await h.mount({ onFim });
    await act(async () => { await h.getHook().iniciar(); });
    let ending!: Promise<void>;
    await act(async () => {
      ending = h.getHook().parar('silencio');
      expect(h.getHook().parar('escondeu')).toBe(ending);
      await h.getHook().iniciar();
    });
    expect(fakeRec.record).toHaveBeenCalledTimes(1);
    expect(h.getHook().gravando).toBe(true);
    await act(async () => {
      stop.resolve();
      emitRecordingStatus();
      await h.getHook().iniciar();
    });
    expect(fakeRec.record).toHaveBeenCalledTimes(1);
    expect(fakeRec.release).not.toHaveBeenCalled();
    expect(h.getHook().gravando).toBe(true);
    await act(async () => {
      audio.resolve(new Blob(['audio']));
      await ending;
    });
    expect(onFim).toHaveBeenCalledTimes(1);
    expect(onFim.mock.calls[0]?.[1]).toBe('escondeu');
    expect(fakeRec.stop).toHaveBeenCalledTimes(1);
    expect(h.getHook().gravando).toBe(false);
    await h.unmount();
  });

  it('aguarda o evento nativo quando stop resolve e depois recebe hasError', async () => {
    fakeRec.stop = vi.fn(async () => {});
    const onFim = vi.fn();
    const onErroParada = vi.fn();
    const h = mountHook({ onFim, onErroParada });
    await h.mount({ onFim, onErroParada });
    await act(async () => { await h.getHook().iniciar(); });
    let ending!: Promise<void>;
    await act(async () => { ending = h.getHook().parar('botao'); });
    expect(onFim).not.toHaveBeenCalled();
    expect(fakeRec.release).not.toHaveBeenCalled();
    await act(async () => {
      emitRecordingStatus({ hasError: true, error: 'stop nativo falhou', url: null });
      await ending;
    });
    expect(onFim).not.toHaveBeenCalled();
    expect(onErroParada).toHaveBeenCalledTimes(1);
    expect(onErroParada.mock.calls[0][0].message).toBe('stop nativo falhou');
    expect(fetch).not.toHaveBeenCalled();
    expect(removeStatusListener).toHaveBeenCalledTimes(1);
    expect(fakeRec.release).toHaveBeenCalledTimes(1);
    await h.unmount();
  });

  it('falha nativa durante a gravação interrompe e informa o erro sem entregar áudio', async () => {
    const onFim = vi.fn();
    const onErroParada = vi.fn();
    const h = mountHook({ onFim, onErroParada });
    await h.mount({ onFim, onErroParada });
    await act(async () => { await h.getHook().iniciar(); });
    await act(async () => { emitRecordingStatus({ hasError: true, error: 'microfone interrompido', url: null }); });
    expect(h.getHook().gravando).toBe(false);
    expect(onErroParada).toHaveBeenCalledTimes(1);
    expect(onErroParada.mock.calls[0][0].message).toBe('microfone interrompido');
    expect(onFim).not.toHaveBeenCalled();
    expect(fakeRec.stop).toHaveBeenCalledTimes(1);
    expect(removeStatusListener).toHaveBeenCalledTimes(1);
    await h.unmount();
  });

  it.each(['stop', 'status'] as const)('falha de forma finita quando %s não confirma a parada', async (missing) => {
    const stop = deferred<void>();
    fakeRec.stop = vi.fn(() => missing === 'stop' ? stop.promise : Promise.resolve());
    const onFim = vi.fn();
    const onErroParada = vi.fn();
    const h = mountHook({ onFim, onErroParada });
    await h.mount({ onFim, onErroParada });
    await act(async () => { await h.getHook().iniciar(); });
    let ending!: Promise<void>;
    await act(async () => { ending = h.getHook().parar('botao'); vi.advanceTimersByTime(4999); });
    expect(onFim).not.toHaveBeenCalled();
    expect(onErroParada).not.toHaveBeenCalled();
    expect(h.getHook().gravando).toBe(true);
    await act(async () => { vi.advanceTimersByTime(1); await ending; });
    expect(onFim).not.toHaveBeenCalled();
    expect(onErroParada).toHaveBeenCalledTimes(1);
    expect(onErroParada.mock.calls[0][0].cause).toBe('recording_status_timeout');
    expect(h.getHook().gravando).toBe(false);
    expect(removeStatusListener).toHaveBeenCalledTimes(1);
    expect(fakeRec.release).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
    stop.resolve();
    await h.unmount();
  });

  it('erro nativo e rejeição de prepare propagam o erro somente uma vez', async () => {
    fakeRec.prepareToRecordAsync = vi.fn(async () => {
      emitRecordingStatus({ hasError: true, error: 'erro nativo de preparação', url: null });
      throw new Error('prepare rejeitou');
    });
    const onFim = vi.fn();
    const onErroParada = vi.fn();
    const h = mountHook({ onFim, onErroParada });
    await h.mount({ onFim, onErroParada });
    await act(async () => { await expect(h.getHook().iniciar()).resolves.toBeUndefined(); });
    expect(onErroParada).toHaveBeenCalledTimes(1);
    expect(onErroParada.mock.calls[0][0].message).toBe('erro nativo de preparação');
    expect(onFim).not.toHaveBeenCalled();
    expect(fakeRec.record).not.toHaveBeenCalled();
    expect(fakeRec.release).toHaveBeenCalledTimes(1);
    await h.unmount();
  });

  it('falha ao liberar no cleanup é informada sem rejeição pendente', async () => {
    const released = deferred<void>();
    const erro = new Error('release falhou');
    fakeRec.release = vi.fn(() => { throw erro; });
    const onFim = vi.fn();
    const onErroParada = vi.fn(() => { released.resolve(); });
    const h = mountHook({ onFim, onErroParada });
    await h.mount({ onFim, onErroParada });
    await act(async () => { await h.getHook().iniciar(); });
    await h.unmount();
    await act(async () => { await released.promise; });
    expect(onErroParada).toHaveBeenCalledTimes(1);
    expect(onErroParada).toHaveBeenCalledWith(erro);
    expect(removeStatusListener).toHaveBeenCalledTimes(1);
  });

  it('desmontar gravando (troca de conversa) para e entrega o áudio ao onFim de quem gravou', async () => {
    const onFim = vi.fn();
    const h = mountHook({ onFim });
    await h.mount({ onFim });
    await act(async () => { await h.getHook().iniciar(); });
    await h.unmount();
    await act(async () => { await vi.runAllTimersAsync(); });
    expect(onFim).toHaveBeenCalledTimes(1);
    expect(onFim.mock.calls[0][1]).toBe('escondeu');
  });
});

describe('useDitado — VAD e teto', () => {
  it('encerra por silêncio depois de ouvir fala usando o RMS bruto', async () => {
    fakeRec.getStatus.mockReturnValue({ metering: -20 });
    const onFim = vi.fn();
    const h = mountHook({ onFim });
    await h.mount({ onFim });
    await act(async () => {
      await h.getHook().iniciar();
      vi.advanceTimersByTime(600);
    });
    expect(h.getHook().rms).toBeCloseTo(0.5);
    fakeRec.getStatus.mockReturnValue({ metering: -160 });
    await act(async () => { vi.advanceTimersByTime(2300); });
    expect(onFim.mock.calls[0]?.[1]).toBe('silencio');
    expect(fakeRec.stop).toHaveBeenCalledTimes(1);
    await h.unmount();
  });

  it('sem metering não usa VAD e encerra pelo teto', async () => {
    fakeRec.getStatus.mockReturnValue({});
    const onFim = vi.fn();
    const h = mountHook({ onFim });
    await h.mount({ onFim });
    await act(async () => {
      await h.getHook().iniciar();
      vi.advanceTimersByTime(5000);
    });
    expect(h.getHook().rms).toBe(0);
    expect(fakeRec.stop).not.toHaveBeenCalled();
    await act(async () => { vi.advanceTimersByTime(175000); });
    expect(onFim.mock.calls[0]?.[1]).toBe('teto');
    expect(fakeRec.stop).toHaveBeenCalledTimes(1);
    await h.unmount();
  });
});
