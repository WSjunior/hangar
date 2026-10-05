// Escolhas e carregadores da tela inicial de nova conversa (celular). Reimplementa só o pedaço do
// CreateSessionSheet que esta tela usa — duplicação aceita: a folha carrega bastão, motor, contas
// novas e retomada, que aqui não existem.
import {
  createSessionForServer, sendInputForServer, fetchSessionsForServer, getCodexAccountsForServer,
  getFolderBranchesForServer, getRootsForServer, listClaudeConfigs, getClaudeAccountSuggestion, getProviders,
  uniqueSessionName, basename, effortLevels, defaultCodexAccount, SESSION_PROVIDERS,
  type CodexAccount, type ConfigDirInfo, type FolderBranches, type FsRoot, type ModelOption, type Provider,
  type WorktreeChoice,
} from '@hangar/core';
import { listOwnServers, selectServer, getActiveId, type Server } from './auth';
import { carregarModelos } from './modelosPorConta';
import { bestAccountWithQuota, exhaustedWindow, type ContaCota } from './cota';
import * as m from '../paraglide/messages';

type ProviderProbe = Record<string, { disponivel: boolean; motivo: string | null; default?: boolean }>;

const errText = (e: unknown, fallback: string) => (e instanceof Error && e.message ? e.message : fallback);
export function isNotRepo(e: unknown): boolean {
  const status = (e as { status?: number } | null)?.status;
  return status === 404 || (status === 409 && errText(e, '').includes('not a git repository'));
}
export function worktreeChoiceOf(d: { branch: string; newBranch: boolean; base: string; branchName: string }): WorktreeChoice | null {
  if (d.newBranch && d.branchName.trim()) return { branch: d.branchName.trim(), new_branch: true, base: d.base || null };
  return d.branch ? { branch: d.branch } : null;
}

const cwdKey =(server: string) => `cp_newchat_cwd:${server}`;

function readStorage(key: string): string | null {
  try { return localStorage.getItem(key); } catch { return null; }
}
function writeStorage(key: string, value: string) {
  try {
    if (value) localStorage.setItem(key, value);
    else localStorage.removeItem(key);
  } catch { /* storage bloqueado: segue sem memória */ }
}

export class NewChatDraft {
  servers = $state<Server[]>([]);
  server = $state('');
  cwd = $state<string | null>(null);
  provider = $state<Provider>('claude');
  configDir = $state<string | null>(null);
  codexAccount = $state('');
  model = $state('');
  effort = $state('');
  branch = $state('');
  newBranch = $state(false);
  base = $state('');
  branchName = $state('');

  providers = $state<ProviderProbe>({});
  providersLoading = $state(false);
  providersError = $state('');
  roots = $state<FsRoot[] | null>(null);
  rootsError = $state('');
  configs = $state<ConfigDirInfo[]>([]);
  configsLoading = $state(false);
  configsError = $state('');
  codexAccounts = $state<CodexAccount[]>([]);
  codexLoading = $state(false);
  codexError = $state('');
  models = $state<ModelOption[]>([]);
  modelsLoading = $state(false);
  modelsError = $state('');
  branches = $state<FolderBranches | null>(null);
  branchesLoading = $state(false);
  branchesError = $state('');

  sending = $state(false);
  sendError = $state('');

  // Um contador por carregador: resposta de servidor/conta/pasta já trocados é descartada.
  #provSeq = 0;
  #rootsSeq = 0;
  #cfgSeq = 0;
  #codexSeq = 0;
  #modSeq = 0;
  #branchSeq = 0;
  #modelTouched = false;
  #providerPicked = false;
  // Só a escolha manual de CONTA desliga a troca automática por cota.
  #accountPicked = false;
  // Sessão já criada cujo primeiro envio falhou: tentar de novo reenvia nela em vez de criar outra,
  // desde que nenhuma escolha tenha mudado desde então.
  #created: { choices: string; name: string } | null = null;

  get serverObj(): Server | null { return this.servers.find((s) => s.id === this.server) ?? null; }
  get #choices(): string {
    return JSON.stringify([this.server, this.cwd, this.provider, this.configDir, this.codexAccount, this.model, this.effort, this.branch,
      this.newBranch, this.base, this.branchName]);
  }
  /** Contas ou modelos ainda chegando: enviar agora mandaria conta/modelo vazios e cairia no padrão do servidor calado. */
  get loading(): boolean { return this.providersLoading || this.configsLoading || this.codexLoading || this.modelsLoading; }
  get levels(): readonly string[] { return effortLevels(this.provider, this.models, this.model); }
  get providerAvailable(): boolean { return this.providers[this.provider]?.disponivel !== false; }

  init() {
    this.servers = listOwnServers();
    const active = getActiveId();
    const first = this.servers.some((s) => s.id === active) ? active! : (this.servers[0]?.id ?? '');
    if (first) this.pickServer(first);
  }

  pickServer(id: string) {
    this.#providerPicked = false;
    this.#modelTouched = false;
    this.server = id;
    selectServer(id);
    this.cwd = readStorage(cwdKey(id));
    this.branch = ''; this.newBranch = false; this.base = ''; this.branchName = '';
    void this.loadProviders();
    void this.loadRoots();
    this.loadAccounts();
    void this.loadBranches();
  }

  setCwd(path: string) {
    this.cwd = path;
    this.branch = ''; this.newBranch = false; this.base = ''; this.branchName = '';
    writeStorage(cwdKey(this.server), path);
    void this.loadBranches();
  }

  setProvider(p: Provider) {
    this.#providerPicked = true;
    if (p === this.provider) return;
    this.provider = p;
    this.loadAccounts();
  }

  setConfig(path: string) {
    this.#modelTouched = true;
    this.#accountPicked = true;
    if (path === this.configDir) return;
    this.configDir = path;
    void this.loadModels();
  }

  /** Conta escolhida sem limite e sem escolha manual: troca sozinha pra conta com folga. */
  switchFromExhausted(linha: ContaCota[] | null) {
    if (this.provider !== 'claude' || this.#accountPicked || this.configsLoading || !this.configDir) return;
    const atual = linha?.find((c) => c.id === `claude:${this.configDir}`);
    if (!exhaustedWindow(atual)) return;
    const alvo = bestAccountWithQuota(linha, this.configs.map((c) => c.path));
    if (!alvo || alvo === this.configDir) return;
    this.configDir = alvo;
    void this.loadModels();
  }

  setCodexAccount(id: string) {
    this.#modelTouched = true;
    if (id === this.codexAccount) return;
    this.codexAccount = id;
    void this.loadModels();
  }

  setModel(value: string) {
    this.#modelTouched = true;
    this.model = value;
    if (!this.levels.includes(this.effort)) this.effort = '';
  }

  // Mesma chave da folha (`CreateSessionSheet`): a escolha feita num lugar vale no outro. Sem motor
  // aqui, então o último segmento é `-` fora do Codex.
  memoryKey(): string {
    return `cp_last_model:${this.server}:${this.provider}:${this.provider === 'codex' ? this.codexAccount : '-'}`;
  }

  async loadProviders() {
    const seq = ++this.#provSeq;
    this.providers = {};
    this.providersLoading = true;
    this.providersError = '';
    try {
      const res = await getProviders();
      if (seq !== this.#provSeq) return;
      this.providers = res;
      const preferred = SESSION_PROVIDERS.find((provider) => res[provider]?.default && res[provider]?.disponivel);
      if (!this.#providerPicked && !this.#modelTouched && !this.sending && preferred && preferred !== this.provider) {
        this.provider = preferred;
        this.loadAccounts();
      }
    } catch (e) {
      if (seq === this.#provSeq) this.providersError = errText(e, m.criar_providers_erro());
    } finally {
      if (seq === this.#provSeq) this.providersLoading = false;
    }
  }

  async loadRoots() {
    const seq = ++this.#rootsSeq;
    const server = this.serverObj;
    this.roots = null;
    this.rootsError = '';
    if (!server) return;
    try {
      const res = await getRootsForServer(server);
      if (seq === this.#rootsSeq) this.roots = res;
    } catch (e) {
      if (seq === this.#rootsSeq) this.rootsError = errText(e, m.falha_conexao());
    }
  }

  loadAccounts() {
    // O modelo é da conta/provider anterior: invalida a lista em voo e limpa a escolha já, para um
    // catálogo atrasado (ou a falha da conta nova) não deixar modelo de outro provider no envio.
    ++this.#cfgSeq; ++this.#codexSeq; ++this.#modSeq;
    this.models = []; this.model = ''; this.effort = ''; this.modelsLoading = false; this.modelsError = '';
    this.configs = []; this.configDir = null; this.configsLoading = false; this.configsError = '';
    this.codexAccounts = []; this.codexAccount = ''; this.codexLoading = false; this.codexError = '';
    if (this.provider === 'claude') void this.loadConfigs();
    else if (this.provider === 'codex') void this.loadCodex();
    else void this.loadModels();
  }

  async loadConfigs() {
    const seq = ++this.#cfgSeq;
    this.#modelTouched = false;
    this.#accountPicked = false;
    this.configsLoading = true;
    try {
      const list = await listClaudeConfigs();
      if (seq !== this.#cfgSeq) return;
      this.configs = list;
      this.configDir = list.find((c) => c.active)?.path ?? list[0]?.path ?? null;
      this.configsLoading = false;
      const modelGen = this.#modSeq + 1;
      void this.loadModels();
      // Conta sugerida pela cota, como a folha: nunca passa por cima de uma escolha já feita.
      getClaudeAccountSuggestion().then(({ path }) => {
        if (seq !== this.#cfgSeq || modelGen !== this.#modSeq || this.#modelTouched || this.provider !== 'claude'
            || !this.configs.some((c) => c.path === path) || path === this.configDir) return;
        this.configDir = path;
        void this.loadModels();
      }).catch(() => { /* sem leitura de cota, fica a conta ativa */ });
    } catch (e) {
      if (seq !== this.#cfgSeq) return;
      this.configsLoading = false;
      this.configsError = errText(e, m.falha_conexao());
      void this.loadModels();
    }
  }

  async loadCodex() {
    const seq = ++this.#codexSeq;
    const server = this.serverObj;
    if (!server) return;
    this.codexLoading = true;
    try {
      const list = await getCodexAccountsForServer(server);
      if (seq !== this.#codexSeq) return;
      this.codexAccounts = list;
      this.codexAccount = defaultCodexAccount(list)?.id ?? '';
      this.codexLoading = false;
      void this.loadModels();
    } catch (e) {
      if (seq !== this.#codexSeq) return;
      this.codexLoading = false;
      this.codexError = errText(e, m.falha_conexao());
    }
  }

  async loadModels() {
    const seq = ++this.#modSeq;
    this.models = []; this.model = ''; this.effort = ''; this.modelsError = '';
    const server = this.serverObj;
    if (this.provider === 'codex' && (!this.codexAccount || !server)) { this.modelsLoading = false; return; }
    this.modelsLoading = true;
    try {
      const r = await carregarModelos({
        provider: this.provider, configDir: this.provider === 'claude' ? this.configDir : null,
        ...(this.provider === 'codex' ? { codexAccount: this.codexAccount, server } : {}),
      }, this.memoryKey());
      if (seq !== this.#modSeq) return;
      this.models = r.models;
      this.model = r.lembrado;
      this.effort = this.levels.includes(r.esforcoLembrado) ? r.esforcoLembrado : '';
    } catch (e) {
      if (seq === this.#modSeq) this.modelsError = errText(e, m.criar_modelos_erro());
    } finally {
      if (seq === this.#modSeq) this.modelsLoading = false;
    }
  }

  async loadBranches() {
    const seq = ++this.#branchSeq;
    const server = this.serverObj, cwd = this.cwd;
    this.branches = null;
    this.branchesError = '';
    if (!server || !cwd) { this.branchesLoading = false; return; }
    this.branchesLoading = true;
    try {
      const res = await getFolderBranchesForServer(server, cwd);
      if (seq === this.#branchSeq) this.branches = res;
    } catch (e) {
      // Pasta fora de repositório não é falha: só não há pílula de branch (mesma regra do nativo).
      if (seq === this.#branchSeq && !isNotRepo(e)) this.branchesError = errText(e, m.falha_conexao());
    } finally {
      if (seq === this.#branchSeq) this.branchesLoading = false;
    }
  }

  /** Aviso embaixo das pílulas, na prioridade do `note()` do nativo. */
  get note(): { text: string; warning: boolean } | null {
    if (this.sending) return { text: m.native_new_chat_sending(), warning: false };
    const claude = this.provider === 'claude';
    const text = [
      this.sendError,
      this.rootsError && `${m.native_create_roots_failed()} ${this.rootsError}`,
      claude ? this.configsError : '',
      this.provider === 'codex' ? this.codexError : '',
      this.modelsError && `${m.native_create_models_failed()}: ${this.modelsError}`,
      this.branchesError && m.native_create_checkout_failed({ reason: this.branchesError }),
      this.roots?.length === 0 ? m.native_new_chat_no_roots() : '',
      claude && !this.configsLoading && !this.configsError && this.configs.length === 0 && this.#cfgSeq > 0
        ? m.native_new_chat_no_accounts() : '',
      !this.providerAvailable ? m.native_create_provider_missing({ p: this.provider }) : '',
      this.providersError && m.criar_providers_erro_detalhe({ erro: this.providersError }),
    ].find(Boolean);
    return text ? { text, warning: true } : null;
  }

  async send(text: string): Promise<{ serverId: string; name: string }> {
    if (this.sending) throw new Error(m.native_new_chat_sending());
    this.sendError = '';
    const server = this.serverObj, cwd = this.cwd;
    try {
      if (!server || !cwd) throw new Error(m.newchat_sem_pasta());
      if (!this.providerAvailable) throw new Error(m.native_create_provider_missing({ p: this.provider }));
      if (this.loading) throw new Error(m.comum_carregando());
      if (this.provider === 'codex' && !this.codexAccount) throw new Error(m.newchat_sem_conta_codex());
      this.sending = true;
      const choices = this.#choices;
      let name = this.#created?.choices === choices ? this.#created.name : '';
      if (!name) {
        const taken = new Set((await fetchSessionsForServer(server)).map((s) => s.name));
        // Memória antes de criar, como a folha: a escolha não se perde se a criação falhar.
        const key = this.memoryKey();
        writeStorage(key, this.model);
        writeStorage(`${key}:effort`, this.effort);
        const sessionName = uniqueSessionName(basename(cwd), taken);
        if (this.newBranch && !this.branchName.trim()) this.branchName = sessionName;
        const info = await createSessionForServer(server, {
          name: sessionName, cwd, provider: this.provider, remember_provider: true,
          config_dir: this.provider === 'claude' ? this.configDir : null,
          codex_account: this.provider === 'codex' ? this.codexAccount : undefined,
          model: this.model || null, effort: this.effort || null,
          ...(worktreeChoiceOf(this) ?? {}),
        });
        // O nome que vale é o devolvido pelo backend: ele pode desempatar de novo.
        name = info.name;
        // Releitura das escolhas: o nome da branch nova pode ter sido preenchido acima.
        this.#created = { choices: this.#choices, name };
      }
      await sendInputForServer(server, name, text);
      this.#created = null;
      return { serverId: server.id, name };
    } catch (e) {
      this.sendError = errText(e, m.criar_sessao_erro());
      throw e;
    } finally {
      this.sending = false;
    }
  }
}

export function createNewChatDraft(): NewChatDraft {
  return new NewChatDraft();
}
