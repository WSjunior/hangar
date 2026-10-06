// Campo de texto (`Input`) de mod: quando o valor que o mod desenha entra no campo. Espelha o `FieldSync` do app
// nativo (`desktop-native/src/plugin_ui.rs`), e os casos de teste são os mesmos nos dois lados.

/** Quando o valor que o mod desenha entra no campo. Só conta como posto quando é posto: com a pessoa no campo ele fica
 *  pendente (se mudou em relação ao desenho anterior) e entra quando o campo perde o foco, salvo se a pessoa digitou
 *  depois que ele chegou. Logo depois do envio do próprio campo, o desenho seguinte entra mesmo com foco: é como o mod
 *  limpa o campo depois do envio, e o valor pode ser igual ao de antes (vazio). Os ecos atrasados dos `change` de antes do envio não tomam a vez da resposta:
 *  `sent` guarda o que o campo mandou como `change` desde o último valor aplicado, e um desenho com um desses valores
 *  é eco velho. */
export class FieldSync {
  private pending: string | null = null;
  private submitted_ = false;
  private readonly sent = new Set<string>();
  /** O valor do desenho anterior do mod, para saber se o novo mudou (`null`: nenhum ainda). */
  private last: string | null;

  constructor(first: string | null = null) {
    this.last = first;
  }

  /** Um desenho. `drawn` é o valor de um desenho novo do mod (`null` quando o campo só perdeu o foco ou o app
   *  redesenhou sem evento novo), `shown` o que o campo mostra e `focused` se a pessoa está nele. Devolve o valor a pôr
   *  no campo agora, ou `null`. */
  draw(drawn: string | null, shown: string, focused: boolean): string | null {
    if (drawn === null) {
      if (focused || this.pending === null) return null;
      const pending = this.pending;
      this.pending = null;
      return this.apply(pending, shown);
    }
    // Depois do envio, um desenho igual ao que se vê ou a um `change` mandado (eco atrasado de antes do envio) não é
    // a resposta: a vez fica, e ele espera como pendente. Limite: uma resposta igual a um valor digitado antes (o
    // vazio depois de a pessoa apagar tudo) só entra quando o campo perde o foco.
    const answer = this.submitted_ && drawn !== shown && !this.sent.has(drawn);
    const changed = drawn !== this.last;
    this.last = drawn;
    if (!focused || answer) {
      this.pending = null;
      this.submitted_ = false;
      return this.apply(drawn, shown);
    }
    // Só vira pendente o valor que mudou em relação ao desenho anterior: um redesenho sem mudança (outro mod que
    // redesenha o lugar) não traz nada do mod para este campo, e no blur apagaria o que se digitou num mod que não
    // ecoa o `value`. O pendente que já havia fica. Na vez da resposta ao envio vale qualquer desenho: a pessoa não
    // digitou depois do envio, e a resposta pode repetir o valor de antes (o vazio que limpa o campo).
    if (changed || this.submitted_) this.pending = drawn;
    return null;
  }

  /** O foco saiu do campo para o rótulo de envio (Tab até ele): o pendente sai sem entrar, porque o envio que vem a
   *  seguir manda o que está no campo, e não o valor do mod. A resposta ao envio entra no desenho seguinte. */
  leftToSend(): void {
    this.pending = null;
  }

  /** O campo mandou `submit` (Enter ou o rótulo de envio): o próximo desenho do mod entra mesmo com foco. O pendente
   *  sai: o que foi enviado é o que está no campo, e um eco de antes do envio não volta ao perder o foco. */
  submitted(): void {
    this.pending = null;
    this.submitted_ = true;
  }

  /** A pessoa digitou, e o campo mandou `sent` como `change`: o pendente é descartado (perder o foco nunca apaga texto
   *  digitado e não enviado), acaba a vez do desenho que responde ao envio, e um desenho com `sent` passa a ser eco. */
  typed(sent: string): void {
    this.sent.add(sent);
    this.pending = null;
    this.submitted_ = false;
  }

  /** O valor do mod entra: o que se mandou antes dele deixa de contar como eco. */
  private apply(value: string, shown: string): string | null {
    this.sent.clear();
    return value !== shown ? value : null;
  }
}
