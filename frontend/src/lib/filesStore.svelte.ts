// Estado + acoes da aba Arquivos (FileTree/FileViewer/FileSearchBar): lista, le, busca e diff de
// arquivos do repo da sessao. .svelte.ts permite runes fora de componente. Uma instancia serve
// UMA sessao (nome fixo no construtor) — quem mantem a instancia viva e o registry abaixo, que
// vive no MODULO: o App remonta o Chat por {#key} a cada troca de sessao (App.svelte:462), e um
// store criado no mount do componente morreria junto com as pastas abertas. O precedente e o
// sessionsStore (singleton com retain/release); o Git.svelte cria o store no componente e o
// $effect o RECRIA quando a sessao muda — o padrao contrario, que faz a regua "pasta aberta
// continua aberta ao voltar" falhar sem erro nenhum.
import * as m from '../paraglide/messages';
import { listFiles, readFile, readCitedFile, searchFiles, pathDiff, writeFile, writeCitedFile } from '@hangar/core';
import { cleanErr } from './gitStore.svelte';
import { sessionServerFor, type SessionServer } from './sessionServer';
import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import type { FileContent, PathDiff, FileSearchHit, TreeEntry } from '@hangar/core';

// Uma aba da faixa. `externo` decide o endpoint de leitura e de gravacao (arquivo citado fora da
// raiz nao passa pelo /files/read), entao ele viaja com a aba — reabrir pelo caminho errado da
// 404 num arquivo que esta na tela.
export interface Aba {
  path: string;
  externo: boolean;
}

export class FilesStore {
  // Pastas expandidas na arvore (caminho absoluto dentro do repo). SvelteSet, nao Set: `$state`
  // so faz proxy de objeto simples e array (svelte/internal/client/proxy.js), entao `.add`/
  // `.delete` num Set cru nao repintariam a arvore.
  abertos = new SvelteSet<string>();
  // Arquivo selecionado na arvore (caminho absoluto). E sempre a aba ATIVA.
  selecionado = $state<string | null>(null);
  linha = $state<number | null>(null);
  // Abas abertas, na ordem de abertura. Array simples (nao SvelteSet): a ORDEM e o que a faixa
  // desenha, e `$state` faz proxy de array.
  abas = $state<Aba[]>([]);
  // Texto digitado e ainda nao gravado, por caminho. Trocar de aba nao pode perder digitacao —
  // e o rascunho e o que acende o ponto de "nao salvo" na aba mesmo quando ela nao esta ativa.
  rascunhos = new SvelteMap<string, string>();
  // Erro da ultima gravacao de cada aba. Mora AQUI, e nao no visor, porque ha um FileViewer so
  // para todas as abas: como estado do componente, o erro era apagado ao trocar de `path` e a
  // resposta que chegasse depois da troca nao tinha onde aterrissar — a tentativa que falhou
  // sumia, e a aba ficava com o mesmo ponto de "nao salvo" de quem ainda nem tentou.
  errosSalvar = new SvelteMap<string, string>();
  // Caminhos com gravacao em voo. Aqui e nao no visor pelo mesmo motivo do erro: o `salvando`
  // do componente era zerado a cada troca de `path`, entao VOLTAR para uma aba que ainda estava
  // gravando destravava o botao e deixava disparar uma segunda gravacao do mesmo arquivo, as
  // duas com o mesmo digest.
  salvandoEm = new SvelteSet<string>();
  // Ultima resposta boa de cada aba. Trocar de aba pinta DAQUI na hora e revalida por tras: sem
  // o cache, voltar pra uma aba mostrava o esqueleto de novo a cada ida e volta.
  private cache = new SvelteMap<string, { conteudo: FileContent; diff: PathDiff | null }>();
  // Aberto de FORA do cwd (citado na conversa, servido pelo /file): so leitura, e nao vai pro
  // localStorage — no reload o readFile do cwd daria 404 num caminho que nunca foi da arvore.
  externo = $state(false);
  // Conteudo do arquivo selecionado (nulo antes da primeira abertura).
  conteudo = $state<FileContent | null>(null);
  // Diff do arquivo selecionado no escopo atual (nulo quando o diff falha — fora de repo git).
  diff = $state<PathDiff | null>(null);
  // Escopo do diff: desde a base da branch (soma dos turnos) ou so o nao-commitado.
  escopo = $state<'branch' | 'nao_commitado'>('branch');
  // Achados da ultima busca; no modo `names`, line e text vem null.
  resultados = $state<FileSearchHit[]>([]);
  // Erro legivel da ultima operacao (nulo quando a operacao foi limpa).
  erro = $state<string | null>(null);
  // Listar so arquivos modificados (a arvore inteira quando false).
  soModificados = $state(true);
  // O backend cortou os achados em 200 (filesearch.MAX_HITS). Uma resposta so, sem paralelismo.
  buscaCortada = $state(false);

  // Abrir em voo (verdadeiro entre o clique e a resposta). O FileViewer precisa saber que NAO
  // pode afirmar "sem diferencas" enquanto o diff nao chegou — sem o sinal, a janela cai no
  // {:else} e a tela mente sobre arquivo que tem diff.
  loading = $state(false);

  // Um contador POR ALVO: `abrir`, `buscar` e `_listar` pintam campos diferentes e nao podem
  // cancelar uns aos outros. Contador unico descartava o arquivo que estava abrindo assim que
  // qualquer outra operacao comecasse. O de lista e POR PASTA: `recarregar()` re-lista varias
  // pastas em paralelo e uma nao pode cancelar a outra.
  private gArquivo = 0;
  // Geracao da abertura mais recente DE CADA CAMINHO. A poda da aba morta precisa disto: com
  // duas tentativas do mesmo arquivo em voo (abrir A, sair, voltar em A, sair de novo), a mais
  // VELHA podia chegar rejeitada e derrubar a aba enquanto a mais nova, que ia dar certo,
  // ainda estava vindo — a aba de um arquivo que carregou sumia da faixa.
  private ultimaAberturaDe = new Map<string, number>();
  private gBusca = 0;
  private gLista = new Map<string, number>();

  // Geracao do DONO do campo `erro` (parecer Task 11, B1): `abrir`, `buscar` e `recarregar`
  // capturam uma geracao nova ao comecar e so escrevem/limpam `erro` se ainda forem o dono
  // quando a resposta voltar. Sem isto, a recuperacao do 404 de um arquivo antigo (que roda
  // `await recarregar()` no meio) sobrescrevia o erro de uma abertura mais nova com o aviso
  // do arquivo que ja foi abandonado.
  private gErro = 0;
  // Geracao da busca que gravou `resultados` — o 404 de abrir so remove o hit morto da lista
  // se os resultados ainda forem dessa busca (B4); busca nova em voo tem prioridade.
  private gResultados = -1;

  // Conteudo de cada pasta ja listada ('' = raiz). A arvore mostra varias pastas abertas ao
  // mesmo tempo (docs/mocks/2026-08-15-arvore/arvore.js), entao um diretorio de cada vez nao
  // serve.
  private porPasta = new SvelteMap<string, TreeEntry[]>();

  // O corte e POR PASTA: `recarregar()` lista a raiz e as abertas em paralelo, e um campo unico
  // ficava com o valor de quem respondeu por ultimo — a raiz cortada sumia assim que uma
  // subpasta inteira respondesse depois dela.
  private cortePorPasta = new SvelteMap<string, boolean>();

  private readonly sessao: string;
  // Chave de persistencia (serverId::sessao): as pastas abertas sobrevivem ao recarregar a
  // pagina (a selecao nao — ver _restaurar). Vazia = nao persiste (testes/instancia solta).
  private readonly chaveLS: string;
  // Servidor da sessão, tirado da chave: as chamadas vão a ele, não ao ativo do momento.
  private readonly server: SessionServer;

  constructor(sessao: string, chave = '') {
    this.sessao = sessao;
    this.server = sessionServerFor(chave.includes('::') ? chave.slice(0, chave.indexOf('::')) : '');
    this.chaveLS = chave ? `cp_files_${chave}` : '';
    this._restaurar();
  }

  // A raiz ja respondeu? Enquanto nao, o painel mostra o esqueleto (nao "nada mudou").
  get raizCarregada(): boolean {
    return this.porPasta.has('');
  }

  recolherTudo() {
    this.abertos.clear();
    this._persistir();
  }

  private _restaurar() {
    if (!this.chaveLS) return;
    try {
      const raw = localStorage.getItem(this.chaveLS);
      if (!raw) return;
      // So as PASTAS abertas voltam. A selecao nao: reabrir o visor no Ctrl+R cobria o chat com
      // um arquivo que ninguem pediu de novo (medido 26/08 — e vinha vazio, "sem diferencas").
      const d = JSON.parse(raw) as { abertos?: unknown };
      if (Array.isArray(d.abertos)) for (const p of d.abertos) if (typeof p === 'string') this.abertos.add(p);
    } catch {
      // storage bloqueado ou JSON velho: comeca vazio, como sempre comecou
    }
  }

  private _persistir() {
    if (!this.chaveLS) return;
    try {
      localStorage.setItem(this.chaveLS, JSON.stringify({ abertos: [...this.abertos] }));
    } catch {
      // idem: persistir e conveniencia, nunca erro
    }
  }

  // A arvore achatada: a raiz e, logo depois de cada pasta aberta, os filhos dela.
  get entries(): TreeEntry[] {
    const saida: TreeEntry[] = [];
    const empilha = (dir: string) => {
      for (const e of this.porPasta.get(dir) ?? []) {
        saida.push(e);
        if (e.is_dir && this.abertos.has(e.path)) empilha(e.path);
      }
    };
    empilha('');
    return saida;
  }

  // Cortou em alguma pasta que o usuario esta vendo? Pasta colapsada nao conta: os filhos dela
  // nao estao na arvore.
  get listaCortada(): boolean {
    for (const [dir, cortou] of this.cortePorPasta) {
      if (cortou && (dir === '' || this.abertos.has(dir))) return true;
    }
    return false;
  }

  // Grava o arquivo aberto. Devolve a MENSAGEM do erro em vez de levantar: o visor a mostra e
  // mantem o texto digitado na tela — perder a edicao por causa de um conflito seria trocar um
  // problema por outro pior. Mensagem, nao codigo: o `errorDetail` do api.ts ja traduziu o
  // `{code, params}` do backend, e o codigo nao sobrevive ao Error que ele levanta.
  async salvar(path: string, texto: string): Promise<string | null> {
    const atual = this.conteudo;
    if (!atual || atual.path !== path) return 'erro_arq_inexistente';
    // Uma gravacao por caminho. A trava fica no store, nao no botao: o botao vive num visor so,
    // compartilhado por todas as abas, e por isso nao sabe o que as outras estao fazendo.
    if (this.salvandoEm.has(path)) return null;
    this.salvandoEm.add(path);
    try {
      // Arquivo de fora da raiz grava pelo endpoint da citacao: o `files/write` recusaria o
      // caminho absoluto antes de olhar o conteudo. `eraExterno` e capturado ANTES do await:
      // `this.externo` e do arquivo que esta na tela AGORA, e trocar de aba com a gravacao em
      // voo fazia a decisao de reler o diff usar a flag do arquivo errado.
      const eraExterno = this.externo;
      const gravar = eraExterno ? writeCitedFile : writeFile;
      const r = await gravar(this.sessao, path, texto, atual.digest, this.server());
      // So atualiza se ainda for o mesmo arquivo na tela (o usuario pode ter trocado no meio).
      if (this.conteudo?.path === path) {
        this.conteudo = { ...this.conteudo, text: texto, size: r.size, digest: r.digest };
      }
      // O cache da aba tambem: ela pode nem estar ativa (Ctrl+S com a gravacao em voo e troca
      // de aba no meio), e voltar pra ela mostrando o texto de antes de gravar seria mentira.
      const emCache = this.cache.get(path);
      if (emCache) this.cache.set(path, { ...emCache, conteudo: { ...emCache.conteudo, text: texto, size: r.size, digest: r.digest } });
      // Gravou: o que estava digitado virou o arquivo, entao a aba para de acender o ponto — e
      // a falha da tentativa anterior deixa de valer.
      this.rascunhos.delete(path);
      this.errosSalvar.delete(path);
      // O diff da tela envelheceu no instante da gravacao: reler e o que impede o visor de
      // afirmar um diff que nao existe mais. Externo nao tem diff (nem esta no repo da sessao).
      if (!eraExterno) void this.recarregarDiff(path);
      return null;
    } catch (e) {
      const falha = (e as Error)?.message || 'erro_arq_salvar_falhou';
      // Registrado POR CAMINHO antes de devolver: quem pediu a gravacao pode nao estar mais na
      // tela, e devolver a mensagem para um visor que ja trocou de arquivo era o mesmo que
      // jogar fora. Assim voltar para a aba mostra por que ela nao gravou.
      // So enquanto a aba EXISTE: fechada no meio da gravacao, ela ja descartou o rascunho de
      // proposito, e reinserir o erro aqui o deixaria preso no mapa para sempre — e aceso no
      // ponto vermelho se o mesmo caminho fosse reaberto depois, acusando uma tentativa que a
      // aba nova nunca fez.
      if (this.abas.some((a) => a.path === path)) this.errosSalvar.set(path, falha);
      return falha;
    } finally {
      this.salvandoEm.delete(path);
    }
  }

  async recarregarDiff(path: string) {
    try {
      const d = await pathDiff(this.sessao, path, this.escopo, this.server());
      if (this.selecionado === path) this.diff = d;
      const emCache = this.cache.get(path);
      if (emCache) this.cache.set(path, { ...emCache, diff: d });
    } catch {
      // Diff e enfeite aqui: a gravacao ja aconteceu, e falhar em reler nao pode virar erro na
      // cara de quem acabou de salvar com sucesso.
    }
  }

  // Abre um arquivo: pinta conteudo + diff (no escopo atual) quando a resposta voltar.
  // Arquivo de fora do cwd, pelo endpoint /file/text (so serve o que esta no transcript): texto
  // no MESMO visor da arvore, sem diff mas EDITAVEL — vem com digest, e o `salvar` roteia pro
  // endpoint citado. Midia continua no navegador.
  // Devolve se ESTA abertura deu certo (abertura mais nova por cima conta como "nao falhou").
  async abrirExterno(cru: string, linha: number | null = null): Promise<boolean> {
    this.selecionado = cru;
    this.linha = linha;
    this.externo = true;
    this.erro = null;
    this._registrarAba(cru, true);
    // Com cache, a aba ja aparece preenchida e a releitura corre por tras: `loading` ligado
    // poria o esqueleto por cima de um conteudo que ja esta certo na tela.
    const tinhaCache = this._pintarDoCache(cru);
    this.loading = !tinhaCache;
    const g = ++this.gArquivo;
    this.ultimaAberturaDe.set(cru, g);
    const ge = ++this.gErro;
    try {
      const c = await readCitedFile(this.sessao, cru, this.server());
      if (g !== this.gArquivo) return true;
      this.conteudo = c;
      this.diff = null;
      this.cache.set(cru, { conteudo: c, diff: null });
      return true;
    } catch (e) {
      if (g !== this.gArquivo) { this._podarAbaMorta(cru, e, ge, g); return true; }
      this.conteudo = null;
      this.diff = null;
      this.selecionado = null;
      this.externo = false;
      this._descartarAba(cru);
      if (ge === this.gErro) {
        this.erro = (e as { status?: number }).status === 404 ? m.erro_arq_inexistente() : cleanErr(e);
      }
      return false;
    } finally {
      if (g === this.gArquivo) this.loading = false;
    }
  }

  async abrir(path: string, linha: number | null = null): Promise<boolean> {
    this.selecionado = path;
    this.linha = linha;
    this.externo = false;
    this._persistir();
    this.erro = null;
    this._registrarAba(path, false);
    const tinhaCache = this._pintarDoCache(path);
    this.loading = !tinhaCache;
    const g = ++this.gArquivo;
    this.ultimaAberturaDe.set(path, g);
    const ge = ++this.gErro;   // esta abertura e a dona do erro a partir de agora
    const gr = this.gResultados;   // geracao dos resultados que ESTA abertura pode podar (B4)
    const gb = this.gBusca;        // ... e a geracao da busca vigente no momento do clique
    // A poda so vale se a busca vigente JA pintou os resultados que esta abertura viu: com uma
    // busca nova em voo (gBusca avancou, gResultados nao), o painel ainda mostra a lista
    // antiga e a abertura nao pode poda-la — a busca em voo pode falhar e o hit era a unica
    // resposta (B4, rodada 5 — o clique no hit antigo depois da busca nova comecar).
    const podePodar = gb === gr;
    // allSettled, nao all: o conteudo MANDA. Fora de repositorio git o path_diff sempre responde
    // 409 (git_ops.py) e a arvore tem que continuar lendo arquivo (regra do usuario, 15/08).
    const [c, d] = await Promise.allSettled([
      readFile(this.sessao, path, this.server()),
      pathDiff(this.sessao, path, this.escopo, this.server()),
    ]);
    // Uma abertura mais nova ja tomou o lugar: esta nao pinta nada — nao e falha DESTA. Mas se
    // ELA falhou, a aba que o clique registrou nunca carregou, e sem a poda abaixo ficaria na
    // faixa como um nome clicavel que nao abre nada.
    if (g !== this.gArquivo) {
      if (c.status === 'rejected') this._podarAbaMorta(path, c.reason, ge, g);
      return true;
    }
    this.loading = false;
    if (c.status === 'rejected') {
      // Falha ao abrir nao pode deixar o conteudo do arquivo anterior na tela sob o nome novo.
      this.conteudo = null;
      this.diff = null;
      // Nem o nome do arquivo que falhou: com selecionado marcado e conteudo/diff nulos, o
      // FileViewer cairia no "sem diferencas" — mentira sobre um arquivo que nem abriu (medido
      // ao vivo com um binario: o erro sumia no recarregar e a tela afirmava que o png nao tem
      // diff). Sem selecao, o painel mostra o aviso de erro e a arvore continua navegavel.
      this.selecionado = null;
      this._descartarAba(path);
      this._persistir();
      // Linha fantasma (Task 11): arquivo apagado entre listar e abrir. Alem do erro certo,
      // RE-LISTA — sem a recarga, a linha continua clicavel para sempre apontando pra nada
      // (o 404 do readFile e o sinal; o status vem na propriedade, como no _listar). O erro
      // so e pintado depois da recarga: a raiz limpa this.erro no sucesso, e o recarregar
      // pode falhar com algo mais grave (sessao encerrada) que nao pode ser sobrescrito.
      const status = (c.reason as Error & { status?: number })?.status;
      if (status === 404) {
        // A pasta PAI e re-listada MESMO colapsada (B3): o recarregar so cobre raiz e abertas,
        // e sem esta passada o cache velho da pasta fechada devolveria a linha apagada na
        // proxima expansao. Invalida a geracao ANTES (resposta velha da pasta nao repovoa).
        const pai = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
        // Invalida o cache do pai e descendentes ANTES de qualquer await (B3, rodada 3): se o
        // usuario expandir a pasta enquanto a recuperacao esta em voo, o alternarPasta nao
        // pode achar a listagem velha (com o arquivo apagado) e pular a rede.
        this._invalidarSubarvore(pai);
        await this.recarregar(ge);
        // Forca a listagem do pai SEM condicionar em abertos.has(pai): o recarregar so cobre
        // raiz e pastas abertas, e a expansao durante a recuperacao pode ja ter listado com
        // geracao mais nova — o _listar abaixo deixa gLista descartar a resposta mais velha.
        if (pai !== '') await this._listar(pai, ge);
        // Hit de busca morto (B4): o arquivo apagado sai dos resultados, senao o botao
        // continua clicavel para sempre. So se a busca vigente ja tinha pintado os resultados
        // que esta abertura viu (podePodar) E nada mudou desde o clique (gb/gr iguais aos
        // capturados): busca nova em voo ou concluida nunca e alterada pela resposta velha.
        if (podePodar && gb === this.gBusca && gr === this.gResultados) {
          this.resultados = this.resultados.filter((h) => h.path !== path);
        }
        // B1: so pinta o erro se ESTA abertura ainda for a dona — uma abertura/busca nova
        // venceu durante a recarga e nao pode ser sobrescrita pelo aviso do arquivo antigo.
        if (ge === this.gErro && !this.erro) this.erro = m.erro_arq_inexistente();
      } else if (ge === this.gErro) {
        this.erro = cleanErr(c.reason);
      }
      return false;
    }
    this.conteudo = c.value;
    // Diff que falha NAO derruba a leitura: fora de repositorio git o path_diff responde 409 e
    // a arvore tem que continuar lendo arquivo. `diff = null` e o estado de "sem alteracao".
    this.diff = d.status === 'fulfilled' ? d.value : null;
    this.cache.set(path, { conteudo: this.conteudo, diff: this.diff });
    return true;
  }

  // Troca a aba ativa. Passa pelo mesmo `abrir`/`abrirExterno` de sempre — o cache pinta na hora
  // e a releitura corre por tras — pra nao existirem dois caminhos de abertura com guardas
  // diferentes (e diff de aba parada, que envelhece calado, volta fresco na troca).
  async ativar(path: string): Promise<void> {
    if (this.selecionado === path) return;
    const aba = this.abas.find((a) => a.path === path);
    if (!aba) return;
    await (aba.externo ? this.abrirExterno(path) : this.abrir(path));
  }

  // Fecha uma aba. Fechando a ATIVA, a vizinha da direita assume (a da esquerda quando era a
  // ultima) — o mesmo comportamento de todo editor, e evita cair na tela vazia com abas abertas.
  async fecharAba(path: string): Promise<void> {
    const i = this.abas.findIndex((a) => a.path === path);
    if (i === -1) return;
    const eraAtiva = this.selecionado === path;
    this._descartarAba(path);
    if (!eraAtiva) return;
    const proxima = this.abas[i] ?? this.abas[i - 1];
    if (!proxima) {
      this.selecionado = null;
      this.conteudo = null;
      this.diff = null;
      this.externo = false;
      this.erro = null;
      // Invalida a leitura em voo: sem isto a resposta da aba fechada aterrissa e repinta o
      // visor que o usuario acabou de fechar.
      this.gArquivo++;
      this._persistir();
      return;
    }
    await (proxima.externo ? this.abrirExterno(proxima.path) : this.abrir(proxima.path));
  }

  // Texto digitado numa aba. `null` apaga o rascunho (descartar, ou voltar ao texto do disco).
  anotarRascunho(path: string, texto: string | null): void {
    if (texto === null) {
      this.rascunhos.delete(path);
      // Descartar o que estava escrito encerra a tentativa: manter o aviso de gravacao falhada
      // sobre o texto do disco acusaria um erro que ja nao tem dono.
      this.errosSalvar.delete(path);
    } else {
      this.rascunhos.set(path, texto);
    }
  }

  // Aba vizinha na faixa, em passos de +1/-1, dando a volta. Devolve null com menos de duas abas.
  abaVizinha(passo: number): string | null {
    if (this.abas.length < 2 || this.selecionado === null) return null;
    const i = this.abas.findIndex((a) => a.path === this.selecionado);
    if (i === -1) return null;
    const n = this.abas.length;
    return this.abas[(i + passo + n) % n].path;
  }

  private _registrarAba(path: string, externo: boolean): void {
    if (!this.abas.some((a) => a.path === path)) this.abas.push({ path, externo });
  }

  // Poda a aba de uma abertura VELHA que falhou. Só tira o que nunca chegou a carregar: com o
  // arquivo na tela, ou com resposta boa em cache, a aba é de uma abertura mais nova que deu
  // certo — e derrubá-la por causa do 404 de uma tentativa abandonada tiraria da faixa um
  // arquivo que o usuário está lendo.
  // A aba não some calada: o motivo vai para `erro` pela MESMA regra de dono do resto do
  // arquivo. Com uma abertura mais nova em andamento ela é a dona, e o aviso dela é que vale —
  // quem está esperando o arquivo que pediu por último não quer notícia do que abandonou.
  private _podarAbaMorta(path: string, motivo: unknown, ge: number, g: number): void {
    if (this.selecionado === path || this.cache.has(path)) return;
    // Outra tentativa MAIS NOVA deste mesmo caminho esta em voo (ou ja respondeu): a aba e dela,
    // e esta rejeicao velha nao manda nela.
    if (this.ultimaAberturaDe.get(path) !== g) return;
    this.ultimaAberturaDe.delete(path);
    this._descartarAba(path);
    if (ge !== this.gErro) return;
    const status = (motivo as { status?: number })?.status;
    this.erro = status === 404 ? m.erro_arq_inexistente() : cleanErr(motivo);
  }

  // Tira a aba da faixa e joga fora tudo que era dela. NAO mexe em selecionado/conteudo: quem
  // chama decide o que fica na tela (o 404 limpa, o fechar passa pra vizinha).
  private _descartarAba(path: string): void {
    const i = this.abas.findIndex((a) => a.path === path);
    if (i !== -1) this.abas.splice(i, 1);
    this.rascunhos.delete(path);
    this.errosSalvar.delete(path);
    this.cache.delete(path);
  }

  // Pinta o visor com a ultima resposta boa desta aba. Devolve se havia cache — e o que decide
  // entre mostrar o esqueleto e revalidar em silencio.
  private _pintarDoCache(path: string): boolean {
    const c = this.cache.get(path);
    if (!c) return false;
    this.conteudo = c.conteudo;
    this.diff = c.diff;
    return true;
  }

  // Expande/colapsa uma pasta; ao expandir, lista o conteudo dela (uma vez so).
  async alternarPasta(path: string) {
    if (this.abertos.has(path)) {
      this.abertos.delete(path); // colapsar nao re-lista nem volta pra raiz
      this._persistir();
      return;
    }
    this.abertos.add(path);
    this._persistir();
    if (!this.porPasta.has(path)) await this._listar(path, ++this.gErro);
  }

  // Busca por nome ou conteudo; os achados vao para `resultados`.
  async buscar(q: string, mode: 'names' | 'contents') {
    this.erro = null;
    const g = ++this.gBusca;
    const ge = ++this.gErro;   // esta busca e a dona do erro
    try {
      const r = await searchFiles(this.sessao, q, mode, this.server());
      if (g !== this.gBusca) return;
      this.resultados = r.hits;
      this.gResultados = g;
      this.buscaCortada = r.truncated;
    } catch (e) {
      if (g === this.gBusca && ge === this.gErro) this.erro = cleanErr(e);
    }
  }

  // Re-lista a raiz e todas as pastas abertas, com o filtro `soModificados` de agora.
  // `ge` e a geracao do dono do erro quando chamada POR uma operacao (a recuperacao do 404
  // passa a da abertura); chamada pelo botao, captura geracao nova (e o dono).
  async recarregar(ge?: number) {
    const dona = ge ?? ++this.gErro;
    await Promise.all(['', ...this.abertos].map((p) => this._listar(p, dona)));
  }

  // Troca o escopo do diff e reabre o arquivo selecionado. Quem le o controle da tela e o
  // componente — o store nao depende de DOM (o seletor da barra e um botao, nao um <select>).
  async trocarEscopo(escopo: 'branch' | 'nao_commitado') {
    if (escopo === this.escopo) return;
    this.escopo = escopo;
    // Todo diff em cache e do escopo ANTERIOR: mantido, a proxima troca de aba pintaria a
    // comparacao errada sob o rotulo do escopo novo. O texto do arquivo fica (nao depende do
    // escopo), e cada aba relê o diff dela quando for ativada.
    for (const [p, c] of this.cache) this.cache.set(p, { ...c, diff: null });
    // Externo nao tem diff (nem esta no cwd): reabrir pelo readFile daria 404 num arquivo que
    // acabou de ser mostrado.
    if (this.selecionado && !this.externo) await this.abrir(this.selecionado);
  }

  // Invalida o cache de uma subarvore (a pasta e todos os descendentes): incrementa a
  // geracao de cada pasta — resposta em voo nao pode repovoa-la — e remove o conteudo de
  // porPasta/cortePorPasta. NAO mexe em `abertos`: a pasta continua aberta, so o conteudo e
  // re-listado. E o caminho do 404 de arquivo (B3); a poda do _listar, que remove `abertos`
  // porque a PASTA sumiu do disco, e outra (e por isso nao reusa esta).
  private _invalidarSubarvore(path: string) {
    for (const p of [...this.porPasta.keys(), ...this.cortePorPasta.keys()]) {
      if (p === path || (path !== '' && p.startsWith(path + '/'))) {
        this.gLista.set(p, (this.gLista.get(p) ?? 0) + 1);
        this.cortePorPasta.delete(p);
        this.porPasta.delete(p);
      }
    }
  }

  private async _listar(path: string, ge: number) {
    const g = (this.gLista.get(path) ?? 0) + 1;
    this.gLista.set(path, g);
    // A raiz e sempre listada no recarregar: sucesso limpa o erro — mas so se quem pediu a
    // listagem ainda e o dono (B1): uma abertura/busca nova nao pode ter seu erro apagado
    // pela resposta velha de uma listagem abandonada.
    if (path === '' && ge === this.gErro) this.erro = null;
    try {
      const r = await listFiles(this.sessao, path || undefined, this.soModificados, this.server());
      if (g !== this.gLista.get(path)) return;
      this.porPasta.set(path, r.entries);
      this.cortePorPasta.set(path, r.truncated);
    } catch (e) {
      if (g !== this.gLista.get(path)) return;
      // O status vem na propriedade, nao no texto: `ensureOk` (api.ts:111) deixa a MENSAGEM
      // limpa e anexa `.status` — ler o texto quebra calado (api.ts:533 avisa com todas as
      // letras; medido pelo revisor: o 404 real chega como "Nao deu pra acessar..." + status).
      const status = (e as Error & { status?: number }).status;
      if (status === 404) {
        // Pasta que sumiu do disco (filetree.py responde 404 erro_arq_inexistente): poda a
        // arvore — a pasta aberta aponta pra lugar que nao existe, e o aviso de corte ficaria
        // ligado com a arvore vazia. 404 na RAIZ nao e pasta sumida: e o cwd da sessao que
        // morreu (ou a sessao encerrou) — vira aviso visivel.
        if (path === '') {
          if (ge === this.gErro) this.erro = m.arq_sessao_encerrada();
        } else {
          // A pasta e TODOS os descendentes — abertos OU colapsados: um filho colapsado
          // mantem o cache em porPasta/cortePorPasta, e sem apagar esse cache o arquivo
          // velho reapareceria quando a arvore voltasse a existir (medido pelo revisor).
          // Antes de apagar cada estado, incrementa o contador dele em gLista — invalida
          // resposta em voo de uma subpasta (a resposta atrasada nao pode repor estado
          // obsoleto na arvore).
          const chaves = new Set([...this.abertos, ...this.porPasta.keys(), ...this.cortePorPasta.keys()]);
          for (const p of chaves) {
            if (p === path || p.startsWith(path + '/')) {
              this.gLista.set(p, (this.gLista.get(p) ?? 0) + 1);
              this.abertos.delete(p);
              this.cortePorPasta.delete(p);
              this.porPasta.delete(p);
            }
          }
          this._persistir();
        }
        return;
      }
      if (ge === this.gErro) this.erro = cleanErr(e);
    }
  }
}

// Registry do FilesStore: uma instancia por IDENTIDADE composta (serverId::sessionName), vivendo
// no MODULO. O App remonta o Chat por {#key} a cada troca de sessao, entao um store criado no
// onMount morreria — e a regua "pasta aberta continua aberta ao voltar" exige que o estado
// sobreviva ao remount. A chave leva o serverId porque dois servidores podem ter sessoes com o
// MESMO nome (parecer Task 11, B2): sem o servidor na chave, abrir um arquivo no servidor A
// deixava selecionado/conteudo/cache no store que o servidor B recebia ao abrir a homonima.
const stores = new Map<string, FilesStore>();
const refs = new Map<string, number>();

export const filesStores = {
  // `chave` e a identidade composta (serverId::sessionName); `sessao` e o NOME, que e o que as
  // chamadas de API usam (filesStore.sessao). A identidade vem do mesmo lugar que remonta o
  // Chat (DesktopShell.workspaceSessionKey), nunca calculada diferente por caller.
  retain(chave: string, sessao: string): FilesStore {
    let s = stores.get(chave);
    if (!s) {
      s = new FilesStore(sessao, chave);
      stores.set(chave, s);
    }
    refs.set(chave, (refs.get(chave) ?? 0) + 1);
    return s;
  },
  release(chave: string): void {
    const n = (refs.get(chave) ?? 1) - 1;
    if (n <= 0) refs.delete(chave);
    else refs.set(chave, n);
    // ponytail: com refs 0 o store continua no map — o estado (pastas abertas) e a regua de
    // "pasta aberta continua aberta ao voltar", e a memoria por identidade e barata (o app
    // reusa nomes). Se um dia virar vazamento real, um LRU por antiguedade.
  },
};
