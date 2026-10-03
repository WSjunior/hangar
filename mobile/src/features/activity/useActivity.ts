// Hook incremental para derivar Activity a partir do fluxo de ChatEvent.
// Porte do padrão de Chat.svelte:831 — createActivityFolder() alimentado evento a evento.
import { useMemo, useRef } from 'react';
import { createActivityFolder } from '@hangar/core';
import type { Activity, ChatEvent } from '@hangar/core';

type Folder = ReturnType<typeof createActivityFolder>;
export type FeedCursor = { count: number; firstId: string | undefined };

// Lista que encolheu ou trocou de cabeça (loadOlder põe o histórico antigo NA FRENTE) não é
// crescimento pela cauda: empurrar só o fim repetiria eventos e perderia os antigos.
export function feedFolder(folder: Folder, cursor: FeedCursor, events: ChatEvent[]): FeedCursor {
  const firstId = events[0]?.id;
  if (events.length < cursor.count || firstId !== cursor.firstId) {
    folder.reset(events);
  } else {
    for (let i = cursor.count; i < events.length; i++) {
      const ev = events[i];
      if (ev) folder.push(ev);
    }
  }
  return { count: events.length, firstId };
}

export function useActivity(events: ChatEvent[]): Activity {
  const folder = useRef(createActivityFolder());
  const cursor = useRef<FeedCursor>({ count: 0, firstId: undefined });

  // precisa acontecer durante o render, antes do snapshot — é o que evita re-varrer tudo
  cursor.current = feedFolder(folder.current, cursor.current, events);

  return useMemo(() => folder.current.snapshot(), [events]);
}
