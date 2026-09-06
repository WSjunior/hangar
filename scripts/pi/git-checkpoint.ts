// Objetos ficam num Git próprio; o índice e as referências do projeto são somente leitura.
import type { ExtensionAPI, ExtensionCommandContext, ExtensionContext } from '@earendil-works/pi-coding-agent'
import type { AgentContext } from './agent-context'
import { getAgentContext } from './agent-context'
import { execFile } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import * as fs from 'node:fs'
import { devNull } from 'node:os'
import * as path from 'node:path'
import { promisify } from 'node:util'
import { once } from 'node:events'

const execFileAsync = promisify(execFile)
const CUSTOM_TYPE = 'git-checkpoint'
const ORIGIN_FILE = 'hangar-origin.json'
const RESTORE_MODES = ['Código e conversa', 'Somente conversa', 'Somente código']
const OMP_SESSION_STEM = /^\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z_[0-9a-fA-F-]{36}$/
// O prazo próprio precisa vencer antes de o dispatcher liberar o handler.
const OMP_CAPTURE_TIMEOUT_MS = 25_000

interface GitOptions {
  cwd: string
  shadowDir?: string
  indexFile?: string
  input?: Buffer
  isolated?: boolean
  signal?: AbortSignal
}
interface GitResult { code: number; stdout: Buffer; stderr: Buffer }
interface Repository { workTree: string; repository: string }
interface CheckpointRecord extends Repository {
  version: 2
  ref: string
  shadowDir: string
  prompt: string
  createdAt: string
}
interface Checkpoint extends Repository {
  entryId: string
  ref: string
  shadowDir: string
  prompt: string
  createdAt: string
  legacy: boolean
}
interface ActiveSession {
  key: string
  cwd: string
  file: string | undefined
  shadowDir?: string
  repository?: Repository
}
interface CaptureJob {
  token: ActiveSession
  controller: AbortController
  reason?: 'transition' | 'loop' | 'deadline'
}

export function sessionSlug(sessionFile: string | undefined): string {
  if (!sessionFile) return `ephemeral-${process.pid}`
  return path.basename(sessionFile).replace(/[^\w.-]+/g, '_')
}

function extractText(content: unknown): string {
  if (typeof content === 'string') return content
  if (!Array.isArray(content)) return ''
  return content.filter((part): part is { type: 'text'; text: string } =>
    !!part && typeof part === 'object' && part.type === 'text' && typeof part.text === 'string')
    .map(part => part.text).join(' ')
}

function contained(root: string, target: string): boolean {
  const relative = path.relative(root, target)
  return relative !== '' && relative !== '..' && !relative.startsWith('..' + path.sep) && !path.isAbsolute(relative)
}

function pathsFromGit(bytes: Buffer): string[] {
  const text = bytes.toString('utf8')
  if (!Buffer.from(text).equals(bytes)) throw new Error('Git retornou um caminho que não pode ser representado em UTF-8')
  return text.split('\0').filter(Boolean)
}

async function gitResult(args: string[], options: GitOptions): Promise<GitResult> {
  const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('GIT_')))
  env.GIT_OPTIONAL_LOCKS = '0'
  if (options.isolated || options.shadowDir) {
    env.GIT_CONFIG_GLOBAL = devNull
    env.GIT_CONFIG_SYSTEM = devNull
    env.GIT_CONFIG_NOSYSTEM = '1'
  }
  if (options.indexFile) env.GIT_INDEX_FILE = options.indexFile
  const argv = ['-c', `core.hooksPath=${devNull}`, '-c', 'core.fsmonitor=false', '-c', 'commit.gpgsign=false']
  if (options.shadowDir) argv.push('--git-dir', options.shadowDir, '--work-tree', options.cwd)
  const operation = execFileAsync('git', [...argv, ...args], {
    cwd: options.cwd, env, encoding: 'buffer', maxBuffer: 64 * 1024 * 1024, windowsHide: true, signal: options.signal,
  })
  let inputError: Error | undefined
  operation.child.stdin?.on('error', error => {
    inputError = error
    operation.child.kill()
  })
  operation.child.stdin?.end(options.input)
  try {
    const { stdout, stderr } = await operation
    if (inputError) throw inputError
    return { code: 0, stdout, stderr }
  } catch (error) {
    const failure = error as Error & { code?: number | string; stdout?: Buffer; stderr?: Buffer }
    if (typeof failure.code !== 'number') throw error
    return { code: failure.code, stdout: failure.stdout ?? Buffer.alloc(0), stderr: failure.stderr ?? Buffer.alloc(0) }
  }
}

async function git(args: string[], options: GitOptions): Promise<Buffer> {
  const result = await gitResult(args, options)
  if (result.code !== 0) throw new Error(`Git recusou a operação (${result.code}): ${result.stderr.toString('utf8').trim()}`)
  return result.stdout
}

async function repositoryAt(cwd: string, signal?: AbortSignal): Promise<Repository | undefined> {
  const inside = await gitResult(['rev-parse', '--is-inside-work-tree'], { cwd, signal })
  if (inside.code !== 0 || inside.stdout.toString('utf8').trim() !== 'true') return undefined
  const [top, directory] = await Promise.all([
    git(['rev-parse', '--show-toplevel'], { cwd, signal }),
    git(['rev-parse', '--absolute-git-dir'], { cwd, signal }),
  ])
  return {
    workTree: fs.realpathSync(top.toString('utf8').replace(/\r?\n$/, '')),
    repository: fs.realpathSync(directory.toString('utf8').replace(/\r?\n$/, '')),
  }
}

export function createCheckpointExtension(pi: ExtensionAPI, agentContext: AgentContext = getAgentContext()): void {
  if (Number(process.env.PI_SUBAGENT_DEPTH ?? '0') > 0) return
  const checkpointRoot = path.join(agentContext.agentDir, 'checkpoints')
  const mainStems = new Set<string>()
  const captures = new Set<CaptureJob>()
  let active: ActiveSession | undefined
  let queue: Promise<unknown> = Promise.resolve()
  let ownTreeTarget: string | undefined

  function subagent(ctx: ExtensionContext): boolean {
    const file = ctx.sessionManager.getSessionFile()
    return agentContext.harness === 'omp' && !!file &&
      (OMP_SESSION_STEM.test(path.basename(path.dirname(file))) || mainStems.has(path.dirname(path.resolve(file))))
  }

  function identity(ctx: ExtensionContext): ActiveSession {
    const cwd = fs.realpathSync(ctx.cwd)
    const file = ctx.sessionManager.getSessionFile()
    return { cwd, file, key: [ctx.sessionManager.getSessionId(), file ?? '', cwd].join('\0') }
  }

  function cancelCaptures(reason: CaptureJob['reason'], key?: string): boolean {
    let cancelled = false
    for (const job of captures) {
      if (key !== undefined && job.token.key !== key) continue
      job.reason ??= reason
      job.controller.abort()
      // Um processo abortado pode demorar a sair; não reutilizar seu índice em outra captura.
      if (active?.key === job.token.key) active = { key: active.key, cwd: active.cwd, file: active.file }
      cancelled = true
    }
    return cancelled
  }

  function activate(ctx: ExtensionContext): void {
    if (subagent(ctx)) return
    cancelCaptures('transition')
    const next = identity(ctx)
    if (active?.key === next.key) {
      next.shadowDir = active.shadowDir
      next.repository = active.repository
    }
    active = next
    if (next.file) mainStems.add(path.resolve(next.file).replace(/\.jsonl$/, ''))
  }

  function current(token: ActiveSession, ctx: ExtensionContext): boolean {
    return active === token && identity(ctx).key === token.key && !subagent(ctx)
  }

  function exclusive<T>(operation: () => Promise<T>): Promise<T> {
    const next = queue.then(operation, operation)
    // A falha continua no retorno do chamador, mas não inutiliza a fila das próximas operações.
    queue = next.catch(() => undefined)
    return next
  }

  function rootDirectory(): string {
    fs.mkdirSync(checkpointRoot, { recursive: true, mode: 0o700 })
    return fs.realpathSync(checkpointRoot)
  }

  async function storage(token: ActiveSession, repo: Repository, signal: AbortSignal): Promise<string> {
    if (token.repository && (token.repository.workTree !== repo.workTree || token.repository.repository !== repo.repository)) {
      throw new Error('O repositório da sessão mudou durante a captura')
    }
    if (token.shadowDir) return token.shadowDir
    const directory = path.join(rootDirectory(), `${sessionSlug(token.file).slice(0, 100)}-${randomUUID()}`)
    fs.mkdirSync(directory, { mode: 0o700 })
    await git(['init', '--bare', '--template=', '--initial-branch=main', directory], { cwd: repo.workTree, isolated: true, signal })
    fs.writeFileSync(path.join(directory, ORIGIN_FILE), JSON.stringify({ version: 2, ...repo }), { flag: 'wx' })
    fs.mkdirSync(path.join(directory, 'info'), { recursive: true })
    // Os bytes do checkpoint não passam por filtros, normalização de EOL ou expansões de ident.
    fs.writeFileSync(path.join(directory, 'info/attributes'), '* -text -eol -filter -ident -working-tree-encoding\n')
    token.shadowDir = fs.realpathSync(directory)
    token.repository = repo
    return token.shadowDir
  }

  async function captureBeforePrompt(prompt: string, ctx: ExtensionContext): Promise<void> {
    if (subagent(ctx)) return
    if (!active || active.key !== identity(ctx).key) activate(ctx)
    const token = active!
    const job: CaptureJob = { token, controller: new AbortController() }
    const listener = new AbortController()
    captures.add(job)
    const cancelled = once(job.controller.signal, 'abort', { signal: listener.signal }).then(() => {
      throw new Error('Captura cancelada antes de concluir o checkpoint')
    })
    const timer = agentContext.harness === 'omp' ? setTimeout(() => {
      cancelCaptures('deadline', token.key)
      if (active?.key === token.key) ctx.abort()
    }, OMP_CAPTURE_TIMEOUT_MS) : undefined
    const signal = job.controller.signal
    const operation = exclusive(async () => {
      if (signal.aborted || !current(token, ctx)) return
      const repo = await repositoryAt(token.cwd, signal)
      if (!repo || signal.aborted || !current(token, ctx)) return
      const shadowDir = await storage(token, repo, signal)
      if (signal.aborted || !current(token, ctx)) return
      const options = { cwd: repo.workTree, shadowDir, indexFile: path.join(shadowDir, 'index'), signal }
      // O Git do projeto enumera as exclusões locais/globais; nenhum comando escreve seu índice.
      const candidates = pathsFromGit(await git(['ls-files', '--cached', '--others', '--exclude-standard', '-z'], { cwd: repo.workTree, signal }))
      const expected = new Set<string>()
      const ownRoot = fs.realpathSync(checkpointRoot)
      for (const name of candidates) {
        const full = path.resolve(repo.workTree, name)
        if (!contained(repo.workTree, full) || full === ownRoot || contained(ownRoot, full)) continue
        try {
          const info = fs.lstatSync(full)
          if (info.isFile() || info.isSymbolicLink()) expected.add(name)
        } catch (error) {
          if (!['ENOENT', 'ENOTDIR'].includes((error as NodeJS.ErrnoException).code ?? '')) throw error
        }
      }
      const previous = pathsFromGit(await git(['ls-files', '-z'], options))
      const removed = previous.filter(name => !expected.has(name))
      if (removed.length) await git(['update-index', '--force-remove', '-z', '--stdin'], { ...options, input: Buffer.from(removed.join('\0') + '\0') })
      if (expected.size) await git(['--literal-pathspecs', 'add', '--force', '--pathspec-from-file=-', '--pathspec-file-nul'], {
        ...options, input: Buffer.from([...expected].join('\0') + '\0'),
      })
      else await git(['read-tree', '--empty'], options)
      const tree = (await git(['write-tree'], options)).toString('utf8').trim()
      const ref = (await git(['-c', 'user.name=Hangar checkpoint', '-c', 'user.email=checkpoint@hangar.invalid',
        'commit-tree', tree, '-m', 'checkpoint'], options)).toString('utf8').trim()
      if (!/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(ref)) throw new Error('Git não devolveu uma revisão válida')
      await git(['update-ref', `refs/checkpoints/${randomUUID()}`, ref], options)
      if (signal.aborted || !current(token, ctx)) return
      const record: CheckpointRecord = { version: 2, ...repo, shadowDir, ref, prompt, createdAt: new Date().toISOString() }
      // O próprio registro fica antes do pedido e vira a âncora; não procurar o último usuário depois.
      pi.appendEntry(CUSTOM_TYPE, record)
    })
    try {
      await Promise.race([operation, cancelled])
    } catch (error) {
      if (job.reason === 'transition' || job.reason === 'loop') return
      if (agentContext.harness === 'omp' && job.reason !== 'deadline' && active?.key === token.key) ctx.abort()
      throw error
    } finally {
      clearTimeout(timer)
      listener.abort()
      captures.delete(job)
    }
  }

  function checkpoints(ctx: ExtensionContext, fromId?: string): Checkpoint[] {
    const branch = ctx.sessionManager.getBranch(fromId)
    const views: Checkpoint[] = []
    for (const entry of branch) {
      if (entry.type !== 'custom' || entry.customType !== CUSTOM_TYPE || !entry.data || typeof entry.data !== 'object') continue
      const data = entry.data as Record<string, unknown>
      if (typeof data.ref !== 'string' || typeof data.prompt !== 'string' || typeof data.createdAt !== 'string') continue
      if (data.version === 2 && typeof data.workTree === 'string' && typeof data.repository === 'string' && typeof data.shadowDir === 'string') {
        views.push({ entryId: entry.id, ref: data.ref, prompt: data.prompt, createdAt: data.createdAt,
          workTree: data.workTree, repository: data.repository, shadowDir: data.shadowDir, legacy: false })
      } else if (data.version === undefined && typeof data.entryId === 'string' && agentContext.harness === 'pi') {
        const file = ctx.sessionManager.getSessionFile()
        const header = ctx.sessionManager.getHeader()
        const user = branch.find(item => item.id === data.entryId)
        if (!file || !header?.cwd || !user || user.type !== 'message' || user.message.role !== 'user') continue
        const cwd = fs.realpathSync(header.cwd)
        if (cwd !== fs.realpathSync(ctx.cwd)) continue
        views.push({ entryId: data.entryId, ref: data.ref, prompt: extractText(user.message.content), createdAt: data.createdAt,
          workTree: cwd, repository: '', shadowDir: path.join(checkpointRoot, sessionSlug(file)), legacy: true })
      }
    }
    return views
  }

  async function validateOrigin(checkpoint: Checkpoint, ctx: ExtensionContext): Promise<{ origin: string; repo: Repository }> {
    if (!/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(checkpoint.ref)) throw new Error('Checkpoint sem revisão de código válida')
    const repo = await repositoryAt(fs.realpathSync(ctx.cwd))
    if (!repo || checkpoint.workTree !== (checkpoint.legacy ? fs.realpathSync(ctx.cwd) : repo.workTree)) {
      throw new Error('O checkpoint pertence a outro diretório ou projeto')
    }
    const root = fs.realpathSync(checkpointRoot)
    const origin = fs.realpathSync(checkpoint.shadowDir)
    if (!contained(root, origin)) throw new Error('Origem do checkpoint fora do armazenamento permitido')
    if (fs.existsSync(path.join(origin, 'objects/info/alternates'))) throw new Error('Checkpoint com armazenamento externo de objetos')
    const options = { cwd: checkpoint.workTree, shadowDir: origin }
    const bare = (await git(['--git-dir', origin, 'rev-parse', '--is-bare-repository'], { cwd: origin, isolated: true })).toString('utf8').trim()
    if (bare !== 'true') throw new Error('Origem do checkpoint não é um repositório de snapshots')
    if (checkpoint.legacy) {
      const owner = (await git(['config', '--get', 'user.name'], options)).toString('utf8').trim()
      if (owner !== 'pi-code-checkpoint') throw new Error('Não foi possível demonstrar a origem do checkpoint legado')
    } else {
      const originData = JSON.parse(fs.readFileSync(path.join(origin, ORIGIN_FILE), 'utf8'))
      if (originData?.version !== 2 || originData.workTree !== repo.workTree || originData.repository !== repo.repository || checkpoint.repository !== repo.repository) {
        throw new Error('A origem dos objetos não corresponde ao projeto do checkpoint')
      }
    }
    const type = (await git(['cat-file', '-t', checkpoint.ref], options)).toString('utf8').trim()
    if (type !== 'commit') throw new Error('A referência do checkpoint não é um commit')
    return { origin, repo }
  }

  function validateSelection(token: ActiveSession, ctx: ExtensionContext, checkpoint: Checkpoint): void {
    if (!current(token, ctx) || !ctx.isIdle()) throw new Error('Sessão, diretório ou estado mudou durante a escolha; selecione novamente')
    if (!ctx.sessionManager.getBranch().some(entry => entry.id === checkpoint.entryId)) throw new Error('O checkpoint não pertence mais ao ramo atual')
  }

  async function restoreCode(checkpoint: Checkpoint, ctx: ExtensionContext, token: ActiveSession): Promise<void> {
    const { origin } = await validateOrigin(checkpoint, ctx)
    const names = pathsFromGit(await git(['ls-tree', '-r', '-z', '--name-only', checkpoint.ref], { cwd: checkpoint.workTree, shadowDir: origin }))
    for (const name of names) {
      const target = path.resolve(checkpoint.workTree, name)
      if (!contained(checkpoint.workTree, target)) throw new Error('Caminho do snapshot fora do projeto')
      for (let parent = path.dirname(target); parent !== checkpoint.workTree; parent = path.dirname(parent)) {
        try { if (fs.lstatSync(parent).isSymbolicLink()) throw new Error('Restauração recusada: diretório ancestral é um symlink') }
        catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error }
      }
      try { if (fs.lstatSync(target).isDirectory()) throw new Error('Restauração recusada: um diretório posterior ocupa o lugar do arquivo') }
      catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error }
    }
    validateSelection(token, ctx, checkpoint)
    const operationDir = fs.mkdtempSync(path.join(rootDirectory(), 'restore-'))
    try {
      const options = { cwd: checkpoint.workTree, shadowDir: origin, indexFile: path.join(operationDir, 'index') }
      await git(['read-tree', checkpoint.ref], options)
      validateSelection(token, ctx, checkpoint)
      // checkout-index repõe somente os arquivos da árvore; não remove os criados depois.
      await git(['checkout-index', '--all', '--force'], options)
    } finally {
      fs.rmSync(operationDir, { recursive: true, force: true })
    }
  }

  pi.on('session_start', (_event, ctx) => activate(ctx))
  pi.on('session_switch', (_event, ctx) => activate(ctx))
  pi.on('session_tree', (_event, ctx) => activate(ctx))
  pi.on('session_before_switch', (_event, ctx) => {
    if (!subagent(ctx)) { cancelCaptures('transition'); active = undefined }
  })
  pi.on('session_before_tree', (event, ctx) => {
    if (event.preparation.targetId !== ownTreeTarget) activate(ctx)
  })
  pi.on('session_shutdown', (_event, ctx) => {
    if (!subagent(ctx)) { cancelCaptures('transition'); active = undefined }
  })
  const stopLateCapture = (_event: unknown, ctx: ExtensionContext) => {
    if (!subagent(ctx) && cancelCaptures('loop', identity(ctx).key)) {
      ctx.abort()
      ctx.ui.notify('Captura não concluída antes do turno; operação interrompida', 'error')
    }
  }
  pi.on('agent_start', stopLateCapture)
  pi.on('turn_start', stopLateCapture)
  pi.on('before_agent_start', async (event, ctx) => {
    try { await captureBeforePrompt(event.prompt, ctx) }
    catch (error) {
      ctx.ui.notify(`Não foi possível capturar o checkpoint: ${String(error)}`, 'error')
      throw error
    }
  })

  const commandName = agentContext.harness === 'omp' ? 'hangar-rewind' : 'rewind'
  pi.registerCommand(commandName, {
    description: 'Restaura código e/ou conversa; arquivos criados depois do checkpoint são preservados',
    handler: async (_args, ctx) => {
      if (!ctx.hasUI || subagent(ctx)) return
      if (!active || active.key !== identity(ctx).key) activate(ctx)
      const token = active!
      try {
        if (!ctx.isIdle()) throw new Error('Aguarde a sessão ficar ociosa antes de restaurar')
        const ordered = checkpoints(ctx).reverse()
        if (!ordered.length) { ctx.ui.notify('Nenhum checkpoint disponível neste ramo', 'info'); return }
        const labels = ordered.map((checkpoint, index) => `${index + 1}. ${new Date(checkpoint.createdAt).toLocaleTimeString()}  ${checkpoint.prompt.replace(/\s+/g, ' ').slice(0, 60)}`)
        const selected = await ctx.ui.select('Restaurar até qual checkpoint?', labels)
        if (selected === undefined) return
        const checkpoint = ordered[labels.indexOf(selected)]
        if (!checkpoint) return
        const mode = await ctx.ui.select('Modo de restauração (arquivos novos serão preservados):', [...RESTORE_MODES])
        if (!mode || !RESTORE_MODES.includes(mode)) return
        await exclusive(async () => {
          validateSelection(token, ctx, checkpoint)
          let restoredCode = false
          if (mode !== RESTORE_MODES[1]) {
            await restoreCode(checkpoint, ctx, token)
            restoredCode = true
          }
          if (mode !== RESTORE_MODES[2]) {
            validateSelection(token, ctx, checkpoint)
            ownTreeTarget = checkpoint.entryId
            try {
              const result = await ctx.navigateTree(checkpoint.entryId, { summarize: false })
              if (result.cancelled || identity(ctx).key !== token.key) throw new Error('Navegação cancelada ou sessão alterada')
              ctx.ui.setEditorText(checkpoint.prompt)
            } catch (error) {
              ctx.ui.notify(`${restoredCode ? 'Arquivos restaurados, mas a restauração da conversa não foi concluída' : 'Restauração da conversa não concluída'}: ${String(error)}`, 'warning')
              return
            } finally {
              ownTreeTarget = undefined
            }
          }
          ctx.ui.notify('Restauração concluída', 'info')
        })
      } catch (error) {
        ctx.ui.notify(`Restauração não concluída: ${String(error)}`, 'error')
      }
    },
  })

  const beforeBranch = async (event: { entryId: string }, ctx: ExtensionContext) => {
    if (subagent(ctx)) return
    activate(ctx)
    if (ownTreeTarget || !ctx.hasUI) return
    const checkpoint = checkpoints(ctx, event.entryId).at(-1)
    if (!checkpoint) return
    const token = active!
    const choice = await ctx.ui.select('Restaurar também o código antes de ramificar?', ['Restaurar código', 'Manter código atual'])
    if (choice !== 'Restaurar código') return
    try {
      await exclusive(() => restoreCode(checkpoint, ctx, token))
      ctx.ui.notify('Código restaurado; arquivos posteriores preservados', 'info')
    } catch (error) {
      ctx.ui.notify(`Código não restaurado: ${String(error)}`, 'error')
      return { cancel: true }
    }
  }
  if (agentContext.harness === 'omp') {
    pi.on('session_before_branch', beforeBranch)
    pi.on('session_branch', (_event, ctx) => activate(ctx))
  } else {
    // O Pi chama fork ao evento que o OMP chama branch.
    pi.on('session_before_fork', beforeBranch)
    pi.on('session_fork', (_event, ctx) => activate(ctx))
  }
}

export default function gitCheckpointExtension(pi: ExtensionAPI): void {
  createCheckpointExtension(pi, getAgentContext())
}
