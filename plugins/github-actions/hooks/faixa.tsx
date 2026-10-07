import type { Elements, RenderSurface } from 'claude-code'
import { emAndamento } from './gh'
import type { GhView, Job, Pr, Situacao, Workflow } from './gh'
import type { Checks } from '../types'

type Tabela = Elements[RenderSurface]

export const ICONE: Record<Situacao, string> = {
  ok: '✓', falhou: '✕', rodando: '●', esperando: '○', pulado: '–', cancelado: '⊘',
}

// Hex, não nome de tema: o app nativo só entende hex e as cores ANSI.
const VERDE = '#3fb97a'
const VERMELHO = '#e5534b'
const AZUL = '#5aa6ff'
const AMBAR = '#e8a33d'
const ROXO = '#a371f7'
const APAGADO = '#4a505c'
const COR: Partial<Record<Situacao, string>> = { ok: VERDE, falhou: VERMELHO, cancelado: VERMELHO, rodando: AZUL }

const MAX_PASSO = 30
const curto = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s)

/** O que a revisão do PR diz, em palavras e cor. */
export function revisaoDe(p: Pr): { texto: string; cor?: string } | null {
  if (p.estado === 'MERGED') return { texto: 'juntado', cor: ROXO }
  if (p.estado === 'CLOSED') return { texto: 'fechado', cor: VERMELHO }
  if (p.rascunho) return { texto: 'rascunho' }
  if (p.revisao === 'APPROVED') return { texto: 'aprovado', cor: VERDE }
  if (p.revisao === 'CHANGES_REQUESTED') return { texto: 'pediu mudanças', cor: VERMELHO }
  if (p.revisao === 'REVIEW_REQUIRED') return { texto: 'aguarda revisão', cor: AMBAR }
  return null
}

/** Job de matriz vira nome e o primeiro eixo: `build (windows-latest, x86_64, true)` → `build windows`. */
export function nomeJob(nome: string): string {
  // Termina em `)` ou no `...` com que o GitHub corta nome de matriz longo; `Lint (x) / report` fica como está.
  const m = /^(.+?)\s*\(([^,)]+)[^)]*(?:\)|\.\.\.|…)$/.exec(nome)
  if (!m) return nome
  const eixo = (m[2] ?? '').trim().replace(/-latest$/, '')
  return eixo ? `${m[1]} ${eixo}` : (m[1] ?? nome)
}

const MAX_ROTULO = 48

/** Nome de cada job na faixa: o curto, ou o completo cortado quando o curto se repete no workflow. */
export function nomesJobs(js: readonly Job[]): string[] {
  const curtos = js.map(j => nomeJob(j.nome))
  return curtos.map((c, i) => curto(curtos.indexOf(c) !== curtos.lastIndexOf(c) ? js[i]?.nome ?? c : c, MAX_ROTULO))
}

/** Chave de cada job: o nome e, entre homônimos (nome cortado pelo GitHub), a ordem dele. */
export function chavesJobs(wf: string, js: readonly Job[]): string[] {
  const vistos = new Map<string, number>()
  return js.map(j => {
    const n = vistos.get(j.nome) ?? 0
    vistos.set(j.nome, n + 1)
    return chaveDe('job', `${wf}\n${j.nome}\n${n}`)
  })
}

/** O passo sem o `Run ` que o Actions põe em todo `run:`, cortado. */
export const passoCurto = (p: string) => curto(p.replace(/^Run\s+/, ''), MAX_PASSO)

export function contar(js: readonly Job[]): Checks {
  const c: Checks = { ok: 0, falhou: 0, rodando: 0 }
  for (const j of js) {
    if (j.situacao === 'ok' || j.situacao === 'pulado') c.ok++
    else if (j.situacao === 'rodando' || j.situacao === 'esperando') c.rodando++
    else c.falhou++
  }
  return c
}

/** O run mais novo de cada workflow (a lista vem do commit mais novo ao mais velho) e os já superados que ainda rodam. */
export function separarPorCommit(ws: readonly Workflow[]): { atual: Workflow[]; anteriores: Workflow[] } {
  const vistos = new Set<string>()
  const atual: Workflow[] = []
  const anteriores: Workflow[] = []
  for (const w of ws) {
    if (vistos.has(w.nome)) anteriores.push(w)
    else {
      vistos.add(w.nome)
      atual.push(w)
    }
  }
  return { atual, anteriores }
}

/** Quanto do job já andou: inteiro quando terminou, pelos passos quando roda. */
function fracao(j: Job): number {
  if (j.situacao === 'esperando') return 0
  if (j.situacao !== 'rodando') return 1
  // Estado antigo, gravado antes de `feitos`/`total` existirem, chega sem eles.
  const total = j.total ?? 0
  return total ? (j.feitos ?? 0) / total : 0
}

export const percentual = (js: readonly Job[]) =>
  `${js.length ? Math.round((js.reduce((a, j) => a + fracao(j), 0) / js.length) * 100) : 0}%`

type Segmento = { inicio: number; w: number; job: Job }

/** Um segmento por job, em colunas, com uma de vão entre eles quando cabe. */
function segmentos(js: readonly Job[], largura: number): Segmento[] {
  const n = js.length
  if (!n || largura < 1) return []
  const vao = largura - n >= n ? 1 : 0
  const util = Math.max(n, largura - vao * (n - 1))
  let x = 0
  return js.map((job, i) => {
    const w = Math.floor(util / n) + (i < util % n ? 1 : 0)
    const s = { inicio: x, w, job }
    x += w + vao
    return s
  })
}

export type Trecho = { texto: string; s: Situacao }

/** A barra no terminal, em texto (lá a fonte é monoespaçada). */
export function barra(js: readonly Job[], largura: number): Trecho[] {
  const out: Trecho[] = []
  const por = (s: Situacao, texto: string) => {
    if (!texto) return
    const ult = out[out.length - 1]
    if (ult && ult.s === s) ult.texto += texto
    else out.push({ s, texto })
  }
  let x = 0
  for (const { inicio, w, job } of segmentos(js, largura)) {
    por('esperando', ' '.repeat(inicio - x))
    const cheio = Math.round(w * fracao(job))
    por(job.situacao, '━'.repeat(cheio))
    por(job.situacao === 'rodando' ? 'rodando' : 'esperando', '─'.repeat(w - cheio))
    x = inicio + w
  }
  return out
}

export const PX_POR_COLUNA = 7
export const ALTURA_PX = 6
const ESTILO = '<style>@keyframes r{0%,100%{opacity:.5}50%{opacity:1}}.a{animation:r 1.4s ease-in-out infinite}'
  + '@media (prefers-reduced-motion:reduce){rect{animation:none!important}}</style>'

/** A barra fora do terminal: um retângulo arredondado por job, o que roda pulsando. */
export function svgBarra(js: readonly Job[], colunas: number): string {
  const largura = colunas * PX_POR_COLUNA
  const h = ALTURA_PX - 2
  const rects = segmentos(js, colunas).map(({ inicio, w, job }) => {
    const x = inicio * PX_POR_COLUNA + 1
    const wp = Math.max(2, w * PX_POR_COLUNA - 2)
    const f = fracao(job)
    // Pulado conta como resolvido, mas não como passou: verde apagado, para não parecer job na fila.
    const cor = job.situacao === 'pulado' ? `${VERDE}" fill-opacity=".35` : COR[job.situacao] ?? APAGADO
    const fundo = f < 1 ? `<rect x="${x}" y="1" width="${wp}" height="${h}" rx="2" fill="${APAGADO}"/>` : ''
    const cheio = f > 0
      ? `<rect x="${x}" y="1" width="${Math.max(2, Math.round(wp * f))}" height="${h}" rx="2" fill="${cor}"${job.situacao === 'rodando' ? ' class="a"' : ''}/>`
      : ''
    return fundo + cheio
  }).join('')
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${largura}" height="${ALTURA_PX}" viewBox="0 0 ${largura} ${ALTURA_PX}">${ESTILO}${rects}</svg>`
}

export function duracao(d: number): string {
  const s = Math.max(0, Math.round(d / 1000))
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, '0')}m`
}

/** `3/7 jobs · 4m 12s`: jobs terminados, e o tempo do run mais longo (até agora, se ainda roda). */
export function progresso(ws: readonly Workflow[], agora: number): string {
  const js = ws.flatMap(w => w.jobs)
  const feitos = js.filter(j => j.situacao !== 'rodando' && j.situacao !== 'esperando').length
  // Só o que ainda roda conta até agora; terminado sem data de fim fica sem tempo.
  const tempos = ws.flatMap(w => {
    const fim = w.fim ?? (emAndamento(w.situacao) ? agora : null)
    return w.inicio && fim !== null ? [fim - w.inicio] : []
  })
  const tempo = tempos.length ? ` · ${duracao(Math.max(...tempos))}` : ''
  return js.length ? `${feitos}/${js.length} jobs${tempo}` : tempo.slice(3)
}

/** Rótulo de cada run de commit anterior, único na faixa: o Hangar acha o botão pelo texto. */
export function rotulosAnteriores(ws: readonly Workflow[]): string[] {
  const base = ws.map(w => `${ICONE[w.situacao]} ${w.nome} ${w.sha.slice(0, 7)}`)
  return base.map((r, i) => (base.indexOf(r) !== base.lastIndexOf(r) ? `${r} #${ws[i]?.id ?? i}` : r))
}

/** Chave de um item que abre e fecha: curta (o escopo de hover vai até 64 caracteres, e nome de job de matriz
 *  passa disso) e pelo nome, não pelo id do run, para seguir aberto quando chega um run novo. */
export function chaveDe(tipo: 'wf' | 'job', nome: string): string {
  let h = 5381
  for (const c of nome) h = (Math.imul(h, 33) ^ (c.codePointAt(0) ?? 0)) >>> 0
  return `${tipo}-${h.toString(36)}-${nome.length}`
}

type Acoes = { abrir: (url: string) => void; alternar: () => void; alternarItem: (chave: string) => void }
type Opcoes = { superficie: RenderSurface; colunas: number; recolhida: boolean; agora: number; abertos: readonly string[] }

const BARRA_RECOLHIDA = 16

export function desenharFaixa(t: Tabela, v: GhView, o: Opcoes, acoes: Acoes): JSX.Element {
  const { Box, Text, Button } = t
  const Svg = o.superficie !== 'terminal' && 'Svg' in t ? t.Svg : null
  const pinta = (s: Situacao, texto: string, bold = false) =>
    COR[s] ? <Text color={COR[s]} bold={bold}>{texto}</Text> : <Text dimColor bold={bold}>{texto}</Text>
  // Ao menos uma coluna por job: menos que isso, os últimos segmentos ficariam de fora da barra.
  const desenhaBarra = (js: readonly Job[], largura: number, alt: string) => {
    const colunas = Math.max(largura, js.length)
    return Svg
      ? <Svg source={svgBarra(js, colunas)} alt={alt} width={colunas * PX_POR_COLUNA} height={ALTURA_PX} />
      : <Box flexDirection="row" flexShrink={0}>{barra(js, colunas).map(tr => pinta(tr.s, tr.texto))}</Box>
  }
  // Abre e fecha o detalhe. A `key` é a chave do item: o mesmo rótulo (`build windows`) pode vir de dois workflows.
  const alterna = (chave: string, label: string) => (
    <Button key={chave} label={label} plain hover={{ scope: chave, color: AZUL, underline: true }}
      onPress={() => acoes.alternarItem(chave)} />
  )
  const contagem = (c: Checks) => (
    <Box flexDirection="row" columnGap={1} flexShrink={0}>
      {c.falhou ? <Text color={VERMELHO} bold>{`✕${c.falhou}`}</Text> : null}
      {c.rodando ? <Text color={AZUL}>{`●${c.rodando}`}</Text> : null}
      {c.ok ? <Text color={VERDE}>{`✓${c.ok}`}</Text> : null}
    </Box>
  )

  const { atual, anteriores } = separarPorCommit(v.workflows)
  const p = v.pr
  const rev = p ? revisaoDe(p) : null
  const total = p ? p.checks : contar(atual.flatMap(w => w.jobs))
  const temTotal = total.ok + total.falhou + total.rodando > 0
  const todos = atual.flatMap(w => w.jobs)

  const cabecalho = (
    <Box key="cab" flexDirection="row" justifyContent="space-between" alignItems="center" width={o.colunas} columnGap={1}>
      <Box flexDirection="row" alignItems="center" flexShrink={1} columnGap={1}>
        <Button key="alternar" label={o.recolhida ? '▸' : '▾'} plain onPress={acoes.alternar} />
        {p ? <Text bold>{`PR #${p.numero}`}</Text> : <Text bold>{`⎇ ${v.branch || 'GitHub'}`}</Text>}
        {p ? <Box flexShrink={1}><Text dimColor wrap="truncate-end">{p.titulo}</Text></Box> : null}
        {temTotal ? contagem(total) : null}
        {rev ? (rev.cor ? <Text color={rev.cor}>{rev.texto}</Text> : <Text dimColor>{rev.texto}</Text>) : null}
        {v.aviso && o.recolhida
          ? <Box flexShrink={1}><Text color={AMBAR} wrap="truncate-end">{`⚠ ${v.aviso}`}</Text></Box>
          : null}
      </Box>
      <Box flexDirection="row" alignItems="center" columnGap={1} flexShrink={0}>
        {o.recolhida && todos.length ? desenhaBarra(todos, BARRA_RECOLHIDA, percentual(todos)) : null}
        {o.recolhida && atual.length ? <Text dimColor>{progresso(atual, o.agora)}</Text> : null}
        {p ? <Button key="abrir-pr" label="abrir PR" dimColor onPress={() => acoes.abrir(p.url)} /> : null}
      </Box>
    </Box>
  )
  if (o.recolhida) return <Box flexDirection="column">{cabecalho}</Box>

  const largura = Math.max(o.colunas - 2, 1)
  const corpo: JSX.Element[] = []
  if (v.aviso) {
    corpo.push(
      <Box key="aviso" flexDirection="row" width={largura}>
        <Text color={AMBAR} bold>{'⚠ GitHub: '}</Text>
        <Text color={AMBAR} wrap="truncate-end">{v.aviso}</Text>
        {v.workflows.length || p ? <Text dimColor>{' · abaixo, a última leitura'}</Text> : null}
      </Box>,
    )
  }
  // A barra fica sozinha na linha: no app a coluna é mais estreita que `colunas` e ela encolhe para caber,
  // o que empurrava para baixo o que viesse ao lado.
  const colunasBarra = Math.max(8, largura - 2)
  for (const w of atual) {
    const chaveWf = chaveDe('wf', w.nome)
    const wfAberto = o.abertos.includes(chaveWf)
    corpo.push(
      <Box key={`wf-${w.id}`} flexDirection="row" justifyContent="space-between" width={largura} columnGap={1} marginTop={1}>
        <Box flexDirection="row" flexShrink={1} columnGap={1}>
          {pinta(w.situacao, ICONE[w.situacao], true)}
          {alterna(chaveWf, `${wfAberto ? '▾' : '▸'} ${curto(w.nome, MAX_ROTULO)}`)}
          <Text dimColor>{w.sha.slice(0, 7)}</Text>
        </Box>
        <Box flexDirection="row" columnGap={2} flexShrink={0}>
          <Text dimColor>{progresso([w], o.agora)}</Text>
          {w.jobs.length ? pinta(w.situacao, percentual(w.jobs), true) : null}
          {/* Um run por workflow aqui: o nome já faz o rótulo único, que é como o Hangar acha o botão. */}
          <Button key={`abrir-${w.id}`} label={`abrir ${w.nome}`} dimColor onPress={() => acoes.abrir(w.url)} />
        </Box>
      </Box>,
    )
    if (w.jobs.length) {
      corpo.push(
        <Box key={`br-${w.id}`} flexDirection="row" paddingLeft={2}>
          {desenhaBarra(w.jobs, colunasBarra, `${w.nome}: ${progresso([w], o.agora)}`)}
        </Box>,
      )
    }
    if (wfAberto) {
      // Aberto: todos os jobs, cada um com os passos atrás de outro clique.
      const nomes = nomesJobs(w.jobs)
      const chaves = chavesJobs(w.nome, w.jobs)
      w.jobs.forEach((j, i) => {
        const chaveJob = chaves[i] ?? chaveDe('job', `${w.nome}\n${j.nome}`)
        const nome = nomes[i] ?? j.nome
        const jobAberto = o.abertos.includes(chaveJob)
        const passos = j.passos ?? []
        const falhou = j.situacao === 'falhou' || j.situacao === 'cancelado'
        const detalhe = falhou && j.passo ? `falhou em “${passoCurto(j.passo)}”`
          : j.situacao === 'rodando' && j.passo ? passoCurto(j.passo) : ''
        corpo.push(
          <Box key={`j-${w.id}-${i}`} flexDirection="row" paddingLeft={2} width={largura} columnGap={1}>
            {pinta(j.situacao, ICONE[j.situacao])}
            {passos.length ? alterna(chaveJob, `${jobAberto ? '▾' : '▸'} ${nome}`) : <Text>{nome}</Text>}
            {j.total ? <Text dimColor>{`${j.feitos}/${j.total} passos`}</Text> : null}
            {detalhe ? <Box flexShrink={1}><Text dimColor wrap="truncate-end">{`· ${detalhe}`}</Text></Box> : null}
          </Box>,
        )
        if (!jobAberto) return
        passos.forEach((p, k) => corpo.push(
          <Box key={`p-${w.id}-${i}-${k}`} flexDirection="row" paddingLeft={6} width={largura} columnGap={1}>
            {pinta(p.situacao, ICONE[p.situacao])}
            <Box flexShrink={1}>
              {p.situacao === 'esperando' || p.situacao === 'pulado'
                ? <Text dimColor wrap="truncate-end">{p.nome}</Text>
                : <Text wrap="truncate-end">{p.nome}</Text>}
            </Box>
          </Box>,
        ))
      })
      continue
    }
    // Fechado: só a falha com o passo, e o que roda com a etapa atual: o que a barra não diz.
    const nomes = nomesJobs(w.jobs)
    w.jobs.forEach((j, i) => {
      const falhou = j.situacao === 'falhou' || j.situacao === 'cancelado'
      if (!falhou && j.situacao !== 'rodando') return
      const detalhe = j.situacao === 'cancelado' ? ' cancelado'
        : falhou ? (j.passo ? ` falhou em “${passoCurto(j.passo)}”` : ' falhou')
          : j.passo ? ` · ${passoCurto(j.passo)}` : ''
      corpo.push(
        <Box key={`j-${w.id}-${i}`} flexDirection="row" paddingLeft={2} width={largura}>
          {pinta(j.situacao, `${ICONE[j.situacao]} ${nomes[i] ?? j.nome}`)}
          <Box flexShrink={1}><Text dimColor wrap="truncate-end">{detalhe}</Text></Box>
        </Box>,
      )
    })
  }
  if (anteriores.length) {
    const r = rotulosAnteriores(anteriores)
    corpo.push(
      <Box key="anteriores" flexDirection="row" flexWrap="wrap" columnGap={1} width={largura} marginTop={1}>
        <Text dimColor>commits anteriores:</Text>
        {anteriores.map((w, i) => (
          <Button key={`ant-${w.id}`} label={r[i] ?? String(w.id)} plain dimColor onPress={() => acoes.abrir(w.url)} />
        ))}
      </Box>,
    )
  }
  return (
    <Box flexDirection="column">
      {cabecalho}
      <Box key="corpo" flexDirection="column" paddingLeft={2}>{corpo}</Box>
    </Box>
  )
}
