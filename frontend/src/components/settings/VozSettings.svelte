<script lang="ts">
  import type { ConfigServidorStore } from '../../lib/serverConfig.svelte';
  import LinhaConfig from './LinhaConfig.svelte';
  import EscopoChip from './EscopoChip.svelte';
  import Select from '../Select.svelte';
  import SegmentedPicker from '../SegmentedPicker.svelte';
  import { lerMaosLivres, setMaosLivres } from '../../lib/maosLivres';
  import { ditadoEstilo } from '../../lib/ditadoEstilo.svelte';
  import { estilosDitado, parseTranscriptionProviders, type EstiloDitado } from '@hangar/core';
  import TranscriptionProviders from './TranscriptionProviders.svelte';
  import { listarVozesTts, saldoTts, type TtsVoz } from '@hangar/core';
  import { ttsPlayer } from '../../lib/ttsPlayer.svelte';
  import { ouvirAmostra } from '../../lib/ouvir';
  import { podeLerCriterio } from '../../lib/segredos.svelte';
  import { cortarAmostra } from '../../lib/ttsFormat';
  import { intlLocale } from '../../lib/locale';
  import * as m from '../../paraglide/messages';

  // A tela reúne o caminho inteiro do áudio, hoje partido em três telas (Ditado, Anexos, Avançado):
  // ditar -> transcrever -> limpar o texto -> ler em voz alta. A ordem das seções é o fluxo real.
  interface Props {
    store: ConfigServidorStore;
  }
  let { store }: Props = $props();

  // --- Ditar ---------------------------------------------------------------------------------
  // Mãos-livres é preferência do APARELHO (localStorage), não do servidor — mesmo store que o
  // antigo DictationSettings usava.
  let maosLivres = $state(lerMaosLivres());

  // Estilo do ditado: revalida ao ABRIR a tela, pelo mesmo motivo do DitadoEstiloPopover — o valor
  // pode ter mudado noutro aparelho e a tela não pode mostrar valor de horas atrás.
  $effect(() => { void ditadoEstilo.revalidar(); });

  let estiloErro = $state('');
  async function escolherEstilo(v: EstiloDitado) {
    estiloErro = '';
    try {
      await ditadoEstilo.trocar(v);
    } catch (e) {
      estiloErro = e instanceof Error ? e.message : m.config_motores_erro_salvar();
    }
  }
  const OPCOES_ESTILO = $derived(estilosDitado().map((e) => ({ v: e.valor, label: e.rotulo, aria: e.hint })));

  const CAMPO_VOCABULARIO = {
    chave: 'ditado_vocabulario', tipo: 'texto' as const,
    rotulo: m.config_server_vocabulario(), ajuda: m.config_server_vocabulario_ajuda(),
  };

  // --- Transcrever -----------------------------------------------------------------------------
  // A chave da Groq também serve à organização do texto; a transcrição usa a lista quando há uma.
  const groqChaveOk = $derived(
    store.campos['groq_api_key']?.definido === true && !store.remocaoPendente('groq_api_key'),
  );
  const servicos = $derived(parseTranscriptionProviders(store.valorBruto('transcription_providers')));
  const transcreverOk = $derived(servicos.length ? servicos.some((p) => p.api_key) : groqChaveOk);
  const transcricaoPersonalizada = $derived(
    servicos.length > 0 || String(store.valorAtual('transcription_base_url') ?? '').trim().length > 0,
  );
  let servicosAbertos = $state(false);
  let servicosDecidido = $state(false);
  const CAMPO_TRANSCRICAO_CHAVE = {
    chave: 'groq_api_key', tipo: 'segredo' as const,
    rotulo: m.config_server_groq(), ajuda: m.config_server_groq_ajuda(),
  };
  const CAMPO_TRANSCRICAO_ENDPOINT = {
    chave: 'transcription_base_url', tipo: 'texto' as const,
    rotulo: m.config_server_transcription_endpoint(), ajuda: m.config_server_transcription_endpoint_ajuda(),
  };
  const CAMPO_TRANSCRICAO_MODELO = {
    chave: 'transcription_model', tipo: 'texto' as const,
    rotulo: m.config_server_transcription_model(), ajuda: m.config_server_transcription_model_ajuda(),
  };
  let transcricaoAvancadaAberta = $state(false);
  let transcricaoAvancadaDecidida = $state(false);

  // --- Limpar o texto ----------------------------------------------------------------------------
  // Sozinho, sem chave nenhuma: usa Groq com o padrão do app. O acordeão só existe pra quem quer
  // trocar de provedor. `{#if avancadoAberto}` (não só o `open` do <details>) tira o conteúdo do
  // DOM de verdade quando fechado — o próprio elemento nativo mantém os filhos montados mesmo
  // colapsado, e isso deixaria a tela "dizendo" o endpoint do LLM mesmo com o acordeão fechado.
  let avancadoAberto = $state(false);
  let avancadoDecidido = $state(false);
  let briefingAberto = $state(false);
  // Tipado à mão porque só uma das linhas tem veredito: sem a anotação, o TypeScript infere a
  // união dos objetos e ler `c.veredito` nas outras vira erro.
  const CAMPOS_LLM: {
    chave: string; tipo: 'texto' | 'segredo' | 'escolha'; rotulo: string; ajuda: string;
    opcoes?: { value: string; label: string }[]; veredito?: string; motivo?: string;
  }[] = [
    { chave: 'llm_base_url', tipo: 'texto' as const, rotulo: m.config_server_endpoint_llm(), ajuda: m.config_server_endpoint_llm_ajuda() },
    { chave: 'llm_api_key', tipo: 'segredo' as const, rotulo: m.config_server_chave_llm(), ajuda: m.config_server_chave_llm_ajuda() },
    { chave: 'llm_model', tipo: 'texto' as const, rotulo: m.config_server_modelo_llm(), ajuda: m.config_server_modelo_llm_ajuda() },
    { chave: 'llm_reasoning_effort', tipo: 'escolha' as const, rotulo: m.config_server_raciocinio_llm(), ajuda: m.config_server_raciocinio_llm_ajuda(),
      veredito: m.config_motores_recomendado_none(), motivo: m.config_server_raciocinio_llm_porque(),
      opcoes: [{ value: '', label: m.config_server_raciocinio_padrao() },
               { value: 'none', label: 'none' }, { value: 'low', label: 'low' },
               { value: 'medium', label: 'medium' }, { value: 'high', label: 'high' }] },
  ];
  const CAMPOS_BRIEFING = [
    { chave: 'llm_briefing_base_url', tipo: 'texto' as const, rotulo: m.config_server_endpoint_llm_briefing(), ajuda: m.config_server_endpoint_llm_briefing_ajuda() },
    { chave: 'llm_briefing_api_key', tipo: 'segredo' as const, rotulo: m.config_server_chave_llm_briefing(), ajuda: m.config_server_chave_llm_briefing_ajuda() },
    { chave: 'llm_briefing_model', tipo: 'texto' as const, rotulo: m.config_server_modelo_llm_briefing(), ajuda: m.config_server_modelo_llm_briefing_ajuda() },
  ];
  const organizacaoPersonalizada = $derived(
    String(store.valorAtual('llm_base_url') ?? '').trim().length > 0,
  );
  const organizacaoOk = $derived(
    organizacaoPersonalizada
      ? store.campos['llm_api_key']?.definido === true && !store.remocaoPendente('llm_api_key')
      : groqChaveOk && !String(store.valorAtual('transcription_base_url') ?? '').trim(),
  );

  // Nasce ABERTO quando quem já configurou um provedor próprio chega na tela — fechado por padrão
  // parecia configuração perdida. Decide UMA vez, quando os campos terminam de carregar: sem o
  // `avancadoDecidido`, um Salvar qualquer (que troca a referência de `store.campos`) reabriria o
  // acordeão por cima de um fechamento manual do usuário.
  $effect(() => {
    if (avancadoDecidido || store.carregando || !Object.keys(store.campos).length) return;
    avancadoDecidido = true;
    if ([...CAMPOS_LLM, ...CAMPOS_BRIEFING].some((c) => String(store.valorAtual(c.chave) ?? '').trim())) {
      avancadoAberto = true;
    }
    if (CAMPOS_BRIEFING.some((c) => String(store.valorAtual(c.chave) ?? '').trim())) briefingAberto = true;
  });

  $effect(() => {
    if (transcricaoAvancadaDecidida || store.carregando || !Object.keys(store.campos).length) return;
    transcricaoAvancadaDecidida = true;
    if (String(store.valorAtual('transcription_base_url') ?? '').trim()
      || String(store.valorAtual('transcription_model') ?? '').trim()) {
      transcricaoAvancadaAberta = true;
    }
  });

  // Nasce aberto para quem já tem lista; decide uma vez só, como os outros acordeões.
  $effect(() => {
    if (servicosDecidido || store.carregando || !Object.keys(store.campos).length) return;
    servicosDecidido = true;
    if (servicos.length) servicosAbertos = true;
  });

  // --- Ler em voz alta -----------------------------------------------------------------------
  const lerOk = $derived(
    store.campos['elevenlabs_api_key']?.definido === true
      && !store.remocaoPendente('elevenlabs_api_key'),
  );
  // `lerOk` só decide a UI extra da ElevenLabs (voz/naturalidade/amostra — não existe pro comando
  // local). `podeLerAgora` é a pergunta de verdade "já dá pra ouvir alguma coisa" — mesmo critério
  // de segredos.podeLer(), aqui contra o valor AO VIVO do rascunho, sem esperar o Salvar.
  const podeLerAgora = $derived(podeLerCriterio(lerOk, store.valorAtual('tts_local_cmd')));
  const vozLocalOk = $derived(String(store.valorAtual('tts_local_cmd') ?? '').trim().length > 0);
  let elevenAberto = $state(false);
  let localAberto = $state(false);
  let ajustesAbertos = $state(false);
  $effect(() => { if (lerOk) elevenAberto = true; });
  $effect(() => { if (vozLocalOk) localAberto = true; });
  const CAMPO_ELEVEN = {
    chave: 'elevenlabs_api_key', tipo: 'segredo' as const,
    rotulo: m.config_server_elevenlabs(), ajuda: m.config_server_elevenlabs_ajuda(),
  };
  const CAMPO_MAX_CHARS = {
    chave: 'tts_max_chars', tipo: 'numero' as const, sufixo: m.config_server_car(),
    rotulo: m.config_server_confirmar_leitura(), ajuda: m.config_server_confirmar_leitura_ajuda(),
  };
  const CAMPO_CMD_LOCAL = {
    chave: 'tts_local_cmd', tipo: 'texto' as const,
    rotulo: m.config_server_comando_voz(), ajuda: m.config_server_comando_voz_ajuda(),
  };

  // Vozes/saldo/naturalidade/amostra: movidos de ServerSettings.svelte, inclusive o carregamento
  // SOB DEMANDA — abrir esta tela não pode disparar rede pra fora só por estar aberta.
  let vozes = $state<TtsVoz[]>([]);
  let vozErro = $state('');
  let carregandoVozes = $state(false);
  let saldo = $state<{ usados: number | null; limite: number | null } | null>(null);
  let saldoErro = $state('');
  const vozSelecionadaId = $derived(String(store.valorAtual('elevenlabs_voice_id') ?? '').trim());
  const vozSelecionada = $derived(
    vozes.find((v) => v.id === vozSelecionadaId)?.nome || vozSelecionadaId || m.config_server_padrao_servidor(),
  );

  function carregarVozes() {
    vozErro = '';
    carregandoVozes = true;
    listarVozesTts()
      .then((v) => { vozes = v; })
      .catch((e: Error) => { vozErro = e.message; })
      .finally(() => { carregandoVozes = false; });
    saldoErro = '';
    saldoTts().then((s) => { saldo = s; }).catch((e: Error) => { saldoErro = e.message; });
  }

  const amostraTexto = $derived(cortarAmostra(ttsPlayer.ultimoTexto));
  function ouvirAmostraDaVoz() {
    const voz = (store.valorAtual('elevenlabs_voice_id') as string) || '';
    ouvirAmostra(amostraTexto, voz);
  }

  interface AjusteSlider {
    chave: string;
    rotulo: string;
    padrao: number;
    min: number;
    max: number;
    esquerda: string;
    direita: string;
    ajuda: string;
  }

  const AJUSTES_VOZ: AjusteSlider[] = [
    { chave: 'tts_stability', rotulo: m.config_server_estabilidade(), padrao: 50, min: 0, max: 100,
      esquerda: m.config_server_mais_emotiva(), direita: m.config_server_mais_constante(),
      ajuda: m.config_server_estabilidade_ajuda() },
    { chave: 'tts_similarity_boost', rotulo: m.config_server_aderencia(), padrao: 75, min: 0, max: 100,
      esquerda: m.config_server_mais_livre(), direita: m.config_server_mais_fiel(),
      ajuda: m.config_server_aderencia_ajuda() },
    { chave: 'tts_style', rotulo: m.config_server_exagero(), padrao: 0, min: 0, max: 100,
      esquerda: m.config_server_neutro(), direita: m.config_server_marcante(),
      ajuda: m.config_server_exagero_ajuda() },
    { chave: 'tts_speed', rotulo: m.config_server_velocidade(), padrao: 100, min: 70, max: 120,
      esquerda: m.config_server_mais_devagar(), direita: m.config_server_mais_rapido(),
      ajuda: m.config_server_velocidade_ajuda() },
  ];

  function ajusteValor(a: AjusteSlider): number {
    const bruto = store.valorAtual(a.chave);
    const n = typeof bruto === 'number' ? bruto : parseInt(String(bruto), 10);
    return Number.isFinite(n) ? n : a.padrao;
  }
  function ajusteDefinir(a: AjusteSlider, n: number) {
    store.setRascunho(a.chave, n);
  }
  function ajusteResetar(a: AjusteSlider) {
    store.removerRascunho(a.chave);
  }

  // O rodapé só existe quando há o que salvar — e, quando existe, a tela reserva a altura dele:
  // grudado no pé, ele cobria o último campo de quem estava justamente editando aquele campo.
  const rodapeVisivel = $derived(
    !store.carregando && Object.keys(store.campos).length > 0
      && (store.temMudanca || store.salvando || store.salvo),
  );
</script>

<div class="voz" class:com-rodape={rodapeVisivel}>
  {#if store.carregando}
    <p class="aviso">{m.comum_carregando()}</p>
  {:else if store.erro && !Object.keys(store.campos).length}
    <p class="aviso erro">{store.erro}</p>
    <button class="btn" onclick={() => void store.carregar()}>{m.config_server_tentar_de_novo()}</button>
  {:else}
    <!-- Transcrição -->
    <section class="secao">
      <div class="secao-cabeca">
        <div>
          <h3>{m.voz_transcrever()}</h3>
          <p class="secao-ajuda">{m.voz_transcrever_ajuda()}</p>
        </div>
        <span class="estado" class:ativo={transcreverOk}>
          {transcreverOk
            ? transcricaoPersonalizada ? m.voz_status_personalizado() : m.voz_status_ativo()
            : m.voz_status_desativado()}
        </span>
      </div>

      {#if !transcreverOk}<p class="aviso">{m.voz_transcrever_sem_chave()}</p>{/if}
      <LinhaConfig campo={CAMPO_TRANSCRICAO_CHAVE} {store} removivel />
      <a class="link" href="https://console.groq.com/keys" target="_blank" rel="noopener noreferrer">
        {m.voz_criar_chave()}
      </a>

      <details class="detalhes transcription-provider" bind:open={transcricaoAvancadaAberta}>
        <summary>{m.voz_transcricao_outro_servico()}</summary>
        {#if transcricaoAvancadaAberta}
          <div class="detalhes-corpo">
            <LinhaConfig campo={CAMPO_TRANSCRICAO_ENDPOINT} {store} removivel />
            <LinhaConfig campo={CAMPO_TRANSCRICAO_MODELO} {store} removivel />
          </div>
        {/if}
      </details>

      {#if servicos.length}<p class="aviso">{m.voz_servicos_em_uso()}</p>{/if}
      <details class="detalhes transcription-services" bind:open={servicosAbertos}>
        <summary>{m.native_voice_providers()}</summary>
        {#if servicosAbertos}
          <TranscriptionProviders {store} />
        {/if}
      </details>

      <div class="preferencias">
        <p class="grupo-rotulo">{m.voz_preferencias_transcricao()}</p>

        <div class="linha-maos-livres">
          <div class="txt">
            <label class="rot" for="voz-maos-livres">{m.config_ditado_titulo()}</label>
            <span class="ajuda">{m.config_ditado_desc()}</span>
            <span class="nota">{m.voz_so_neste_aparelho()}</span>
          </div>
          <input id="voz-maos-livres" class="switch" type="checkbox" bind:checked={maosLivres}
            onchange={() => setMaosLivres(maosLivres)} />
        </div>

        <div class="estilo">
          <div class="txt">
            <span class="rot">{m.voz_estilo()} <EscopoChip escopo="servidor" /></span>
            <span class="ajuda">{m.voz_estilo_ajuda()}</span>
          </div>
          <SegmentedPicker value={ditadoEstilo.valor} options={OPCOES_ESTILO}
            ariaLabel={m.voz_estilo()} onPick={(v) => void escolherEstilo(v)} />
        </div>
        {#if estiloErro}<p class="aviso erro">{estiloErro}</p>{/if}

        <LinhaConfig campo={CAMPO_VOCABULARIO} {store} removivel />
      </div>
    </section>

    <!-- Organização do texto -->
    <section class="secao">
      <div class="secao-cabeca">
        <div>
          <h3>{m.voz_limpar()}</h3>
          <p class="secao-ajuda">{m.voz_limpar_ajuda()}</p>
        </div>
        <span class="estado" class:ativo={organizacaoOk}>
          {organizacaoOk
            ? organizacaoPersonalizada ? m.voz_status_personalizado() : m.voz_status_padrao()
            : m.voz_status_desativado()}
        </span>
      </div>
      <details class="detalhes" bind:open={avancadoAberto}>
        <summary>{m.voz_usar_outro_servico()}</summary>
        {#if avancadoAberto}
          <div class="detalhes-corpo">
            {#each CAMPOS_LLM as c (c.chave)}
              <LinhaConfig campo={c} {store} veredito={c.veredito} motivo={c.motivo} removivel />
            {/each}
            <details class="subdetalhes" bind:open={briefingAberto}>
              <summary>{m.voz_briefing_proprio()}</summary>
              {#if briefingAberto}
                {#each CAMPOS_BRIEFING as c (c.chave)}
                  <LinhaConfig campo={c} {store} removivel />
                {/each}
              {/if}
            </details>
          </div>
        {/if}
      </details>
    </section>

    <!-- Ler em voz alta -->
    <section class="secao">
      <div class="secao-cabeca">
        <div>
          <h3>{m.voz_ler()}</h3>
          <p class="secao-ajuda">{m.voz_ler_ajuda()}</p>
        </div>
        <span class="estado" class:ativo={podeLerAgora}>
          {lerOk ? m.voz_status_elevenlabs() : vozLocalOk ? m.voz_status_local() : m.voz_status_desativado()}
        </span>
      </div>
      {#if !podeLerAgora}<p class="aviso">{m.voz_ler_sem_chave()}</p>{/if}

      <details class="provedor" bind:open={elevenAberto}>
        <summary>{m.voz_provedor_elevenlabs()}</summary>
        {#if elevenAberto}
          <div class="provedor-corpo">
            <LinhaConfig campo={CAMPO_ELEVEN} {store} removivel />
          {#if lerOk}
            <div class="tts-extra">
          <p class="config-atual">{m.voz_configuracao_atual({ valor: vozSelecionada })}</p>
          {#if vozErro}
            <p class="aviso erro">{vozErro}</p>
            <button class="btn" onclick={carregarVozes} disabled={carregandoVozes}>{m.config_server_tentar_de_novo()}</button>
          {:else if vozes.length}
            <!-- Rótulo VISÍVEL, não só `ariaLabel`: sem ele a etiqueta de escopo ficaria solta ao
                 lado de um select sem nome, e a regra "sem etiqueta = neste aparelho" leria errado
                 num controle que grava no servidor. Mesmo par dos sliders logo abaixo (span com o
                 rótulo, `aria-label` próprio no controle). -->
            <span class="rot rot-voz">{m.config_server_voz()} <EscopoChip escopo="servidor" /></span>
            <Select
              class="campo-select"
              ariaLabel={m.config_server_voz()}
              value={String(store.valorAtual('elevenlabs_voice_id') ?? '')}
              opcoes={[{ value: '', label: m.config_server_padrao_servidor() },
                       ...vozes.map((v) => ({ value: v.id, label: v.nome }))]}
              onchange={(v) => store.setRascunho('elevenlabs_voice_id', v)}
            />
            <span class="ajuda">{m.config_server_voz_ajuda()}</span>
            {#if store.campos['elevenlabs_voice_id']?.origem === 'app'}
              <button class="ajuste-reset" onclick={() => store.removerRascunho('elevenlabs_voice_id')}>
                {m.config_server_voltar_padrao()}
              </button>
            {/if}
          {:else}
            <button class="btn" onclick={carregarVozes} disabled={carregandoVozes}>
              {carregandoVozes ? m.comum_carregando() : m.config_server_carregar_vozes()}
            </button>
          {/if}

          <details class="ajustes" bind:open={ajustesAbertos}>
            <summary>{m.voz_ajustar()}</summary>
            {#if ajustesAbertos}
              <div class="naturalidade">
                {#each AJUSTES_VOZ as a (a.chave)}
                  {@const valor = ajusteValor(a)}
                  <div class="ajuste">
                    <div class="ajuste-cabeca">
                      <span class="ajuste-rot">{a.rotulo} <em>{valor}</em> <EscopoChip escopo="servidor" /></span>
                      {#if store.campos[a.chave]?.origem === 'app' && !store.remocaoPendente(a.chave)}
                        <button class="ajuste-reset" onclick={() => ajusteResetar(a)}
                                aria-label={`${a.rotulo}, ${m.config_server_voltar_padrao()}`}
                          >{m.config_server_voltar_padrao()}</button>
                      {/if}
                    </div>
                    <span class="ajuda">{a.ajuda}</span>
                    <div class="ajuste-slider">
                      <span class="ponta">{a.esquerda}</span>
                      <input type="range" aria-label={a.rotulo} min={a.min} max={a.max} step="1" value={valor}
                        oninput={(e) => ajusteDefinir(a, +e.currentTarget.value)} />
                      <span class="ponta">{a.direita}</span>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          </details>

          <div class="amostra">
            <!-- O motivo de estar apagado viaja com o botão: solto numa linha ao lado, ele lia como
                 botão quebrado. -->
            <button class="btn" onclick={ouvirAmostraDaVoz} disabled={!ttsPlayer.ultimoTexto}
                    title={ttsPlayer.ultimoTexto ? '' : m.config_server_ouca_antes()}>
              {m.config_server_ouvir_amostra()}{ttsPlayer.ultimoTexto ? m.config_server_caracteres({ n: amostraTexto.length.toLocaleString(intlLocale()) }) : ''}
            </button>
            {#if !ttsPlayer.ultimoTexto}
              <span class="ajuda">{m.config_server_ouca_antes()}</span>
            {/if}
            {#if ttsPlayer.error}<p class="aviso erro">{ttsPlayer.error}</p>{/if}
          </div>

          {#if saldo}
            <p class="sub">{m.config_server_consumo({ usados: saldo.usados ?? '?', limite: saldo.limite ?? '?' })}</p>
          {/if}
          {#if saldoErro}<p class="aviso erro">{saldoErro}</p>{/if}
            </div>
          {/if}
          </div>
        {/if}
      </details>

      <details class="provedor" bind:open={localAberto}>
        <summary>{m.voz_provedor_local()}</summary>
        {#if localAberto}
          <div class="provedor-corpo">
            <LinhaConfig campo={CAMPO_CMD_LOCAL} {store} removivel
              veredito={m.config_server_comando_voz_vered()} motivo={m.config_server_comando_voz_porque()} />
          </div>
        {/if}
      </details>

      {#if podeLerAgora}<LinhaConfig campo={CAMPO_MAX_CHARS} {store} removivel />{/if}
    </section>
  {/if}

  {#if store.erro && Object.keys(store.campos).length}<p class="aviso erro">{store.erro}</p>{/if}
</div>

{#if rodapeVisivel}
  <div class="rodape">
    {#if store.salvo}<span class="ok">{m.config_server_salvo()}</span>{/if}
    {#if store.salvarBloqueado}<span class="bloqueio" role="status">{m.native_server_save_blocked_provider_key()}</span>{/if}
    {#if store.temMudanca || store.salvando}
      <button class="btn primario" onclick={store.salvar} disabled={store.salvando || store.salvarBloqueado}>
        {store.salvando ? m.config_motores_salvando() : m.ctx_salvar()}
      </button>
    {/if}
  </div>
{/if}

<style>
  /* Container query, nunca media query: quem aperta a linha e a largura do PAINEL. */
  /* O respiro de baixo é a altura do rodapé grudado: sem ele o último campo fica escondido atrás
     do botão Salvar, e a pessoa nem sabe que ele existe. */
  .voz { container-type: inline-size; padding: var(--space-2) var(--space-4) var(--space-4); display: flex; flex-direction: column; gap: var(--space-5); }
  .voz.com-rodape { padding-bottom: calc(84px + env(safe-area-inset-bottom)); }

  .secao { min-width: 0; }
  .secao-cabeca {
    display: flex; align-items: flex-start; justify-content: space-between; flex-wrap: wrap;
    gap: var(--space-3); padding-bottom: var(--space-3);
    border-bottom: 1px solid var(--border-subtle);
  }
  .secao-cabeca > div { flex: 1 1 260px; min-width: 0; }
  @container (min-width: 600px) { .secao-cabeca { padding-right: 52px; } }
  .secao h3 {
    margin: 0; font-size: var(--text-base); font-weight: 650; letter-spacing: -0.01em;
    color: var(--text-primary);
  }
  .secao-ajuda {
    max-width: 64ch; margin: 4px 0 0; color: var(--text-muted);
    font-size: var(--text-xs); line-height: 1.45;
  }
  .estado {
    flex: 0 0 auto; margin-top: 1px; padding: 3px 8px;
    border-radius: var(--radius-full); background: var(--surface-inset);
    color: var(--text-muted); font-size: 11px; font-weight: 650;
  }
  .estado.ativo { background: var(--pill-idle-bg); color: var(--pill-idle-fg); }

  .txt { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .rot { font-size: var(--text-base); font-weight: 600; color: var(--text-primary); }
  .ajuda { font-size: var(--text-xs); color: var(--text-muted); line-height: 1.45; min-width: 0; }
  .nota { margin: var(--space-1) 0 0; font-size: var(--text-xs); color: var(--text-muted); }

  .preferencias { margin-top: var(--space-4); border-top: 1px solid var(--border-subtle); }
  .grupo-rotulo {
    margin: var(--space-3) 0 var(--space-1); color: var(--text-muted);
    font-size: var(--label-size); font-weight: var(--label-weight);
    text-transform: uppercase; letter-spacing: var(--label-tracking);
  }
  .linha-maos-livres { display: flex; align-items: center; justify-content: space-between; gap: var(--space-4); padding: var(--space-3) 0; }
  .estilo { display: flex; flex-direction: column; gap: var(--space-2); padding: var(--space-3) 0; border-top: 1px solid var(--border-subtle); }

  .link { display: inline-block; margin-top: var(--space-2); font-size: var(--text-xs); color: var(--accent); }

  /* Acordeão nativo, fechado por padrão — o próprio marcador (▶/▼) já diz que há mais coisa dentro. */
  .detalhes { margin-top: var(--space-2); }
  .detalhes summary { cursor: pointer; font-size: var(--text-sm); font-weight: 600; color: var(--accent); }
  .detalhes-corpo { margin-top: var(--space-2); }
  .subdetalhes { margin-top: var(--space-3); }
  .subdetalhes > summary { color: var(--text-secondary); }

  .provedor {
    padding: var(--space-3) 0; border-bottom: 1px solid var(--border-subtle);
  }
  .provedor > summary, .ajustes > summary {
    cursor: pointer; color: var(--text-primary); font-size: var(--text-sm); font-weight: 600;
  }
  .provedor-corpo { margin-top: var(--space-2); }
  .ajustes { margin-top: var(--space-3); }

  .tts-extra { container-type: inline-size; margin-top: var(--space-2); }
  .config-atual { margin: 0 0 var(--space-2); color: var(--text-secondary); font-size: var(--text-xs); }
  /* O rótulo do seletor de voz é a única coisa acima do select: vira bloco pra o select não subir
     pra linha dele. */
  .rot-voz { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-2); margin-bottom: 4px; }
  .tts-extra :global(.campo-select) { width: 100%; font-family: var(--font-ui); font-size: var(--text-sm); }
  @container (min-width: 360px) { .tts-extra :global(.campo-select) { width: auto; min-width: 220px; } }

  /* A explicação cola no botão (gap curto): com o respiro de sempre ela parecia legenda de outro
     controle, e o botão apagado ficava sem motivo à vista. */
  .amostra { display: flex; flex-direction: column; gap: 6px; margin-top: var(--space-3); align-items: flex-start; }
  .amostra .ajuda { padding-left: 2px; }

  .naturalidade { display: flex; flex-direction: column; gap: var(--space-3); margin: var(--space-3) 0; }
  .ajuste { display: flex; flex-direction: column; gap: 2px; }
  .ajuste-cabeca { display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: var(--space-2); }
  .ajuste-rot { font-size: var(--text-sm); font-weight: 600; color: var(--text-primary); }
  .ajuste-rot em { margin-left: var(--space-2); font-style: normal; color: var(--text-muted); font-size: var(--text-xs); }
  .ajuste-reset { flex-shrink: 0; font-size: var(--text-xs); color: var(--accent); background: none; border: none; padding: 0; }
  /* Trilho contido: esticado na largura toda do painel, um passo do slider virava um pixel e as
     pontas ("neutro"/"marcante") ficavam longe demais do que elas descrevem. */
  .ajuste-slider { display: flex; align-items: center; gap: var(--space-2); margin-top: var(--space-1); max-width: 460px; }
  .ajuste-slider input { flex: 1; min-width: 100px; accent-color: var(--accent); }
  .ajuste-slider .ponta { font-size: var(--text-xs); color: var(--text-muted); white-space: nowrap; flex-shrink: 0; }

  .aviso { font-size: var(--text-sm); color: var(--text-muted); margin: var(--space-2) 0; }
  .aviso.erro { color: var(--error); }
  .sub { margin: var(--space-2) 0 0; font-size: var(--text-xs); color: var(--text-muted); }

  /* CHROME FUNCIONAL, sólido de propósito: grudado no fim da folha — mesma exceção que o
     .rodape do ServerSettings já documenta. NÃO converter pra token de véu. */
  /* Rodapé de ponta a ponta: as margens negativas cancelam o respiro de quem envolve o rodapé — e
     esse respiro muda por modo (a folha no celular tem --space-5 de sobra + faixa segura embaixo,
     a coluna do modal dividido tem --space-4). Base calibrada pra folha; o override abaixo troca
     pra coluna. */
  .rodape {
    position: sticky; bottom: calc(env(safe-area-inset-bottom) * -1 - var(--space-5));
    display: flex; align-items: center; justify-content: flex-end; gap: var(--space-3);
    margin: 0 calc(-1 * var(--space-5)) calc(env(safe-area-inset-bottom) * -1 - var(--space-5));
    padding: var(--space-3) var(--space-4);
    padding-bottom: calc(var(--space-3) + var(--space-4) + env(safe-area-inset-bottom));
    /* Tom do PRÓPRIO painel, mas opaco: com `--bg-surface` a barra era de outra cor, e com o véu do
       `--glass-modal` o texto que rola por baixo aparecia através dela. */
    background: rgb(var(--glass-panel-rgb));
    border-top: 1px solid var(--border-subtle);
  }
  :global(.st-conteudo) .rodape {
    bottom: calc(-1 * var(--space-4));
    margin: 0 calc(-1 * var(--space-4)) calc(-1 * var(--space-4));
  }
  .ok { font-size: var(--text-xs); color: var(--success); }
  .bloqueio { flex: 1 1 auto; min-width: 0; text-align: right; font-size: var(--text-xs); color: var(--warning); }
  .btn {
    height: 40px; padding: 0 var(--space-4);
    border-radius: var(--radius-md);
    background: var(--surface-raised); color: var(--text-primary);
    font-size: var(--text-sm); font-weight: 600;
    transition: transform 160ms ease-out;
  }
  .btn:not(:disabled):active { transform: scale(0.97); }
  .btn.primario { background: var(--accent); color: #fff; }
  .btn:disabled { opacity: 0.45; }
</style>
