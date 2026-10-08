import { getContext, setContext } from 'svelte';

// Conteúdo que cresce sozinho dentro da conversa (página viva) pede à lista para seguir o fim.
// Contexto, não prop: o cartão pode estar dentro de um ToolGroup, e a lista só segue se a pessoa
// estava colada no fim.
const KEY = Symbol('followTail');

export function provideFollowTail(follow: () => void): void {
  setContext(KEY, follow);
}

export function useFollowTail(): () => void {
  return getContext<(() => void) | undefined>(KEY) ?? (() => {});
}
