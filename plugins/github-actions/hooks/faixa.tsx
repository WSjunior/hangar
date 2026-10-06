import type { Elements, RenderSurface } from 'claude-code'
import { rotulosAbrir } from './gh'
import type { GhView, Job, Pr, Situacao, Workflow } from './gh'

type Tabela = Elements[RenderSurface]

export const ICONE: Record<Situacao, string> = {
  ok: '✓', falhou: '✕', rodando: '●', esperando: '○', pulado: '–', cancelado: '⊘',
}
const COR: Partial<Record<Situacao, string>> = { ok: 'success', falhou: 'error', rodando: 'warning' }

const MAX_PASSO = 32
const curto = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s)

/** O que a revisão do PR diz, em palavras e cor de tema. */
export function revisaoDe(p: Pr): { texto: string; cor?: string } | null {
  if (p.estado === 'MERGED') return { texto: 'juntado', cor: 'merged' }
  if (p.estado === 'CLOSED') return { texto: 'fechado', cor: 'error' }
  if (p.rascunho) return { texto: 'rascunho' }
  if (p.revisao === 'APPROVED') return { texto: 'aprovado', cor: 'success' }
  if (p.revisao === 'CHANGES_REQUESTED') return { texto: 'pediu mudanças', cor: 'error' }
  if (p.revisao === 'REVIEW_REQUIRED') return { texto: 'aguarda revisão', cor: 'warning' }
  return null
}

/** Job que passou ou foi pulado vira só o ícone; o resto leva o nome, e o que roda, a etapa. */
export function rotuloJob(j: Job): string {
  if (j.situacao === 'ok' || j.situacao === 'pulado') return ICONE[j.situacao]
  const etapa = j.situacao === 'rodando' && j.passo ? ` · ${curto(j.passo, MAX_PASSO)}` : ''
  return `${ICONE[j.situacao]} ${j.nome}${etapa}`
}

type Pinta = (s: Situacao, texto: string, bold?: boolean) => JSX.Element

export function desenharFaixa(t: Tabela, v: GhView, colunas: number, abrir: (url: string) => void): JSX.Element {
  const { Box, Text, Button } = t
  const pinta: Pinta = (s, texto, bold = false) =>
    COR[s] ? <Text color={COR[s]} bold={bold}>{texto}</Text> : <Text dimColor bold={bold}>{texto}</Text>
  const linhas: JSX.Element[] = []

  if (v.pr) {
    const p = v.pr
    const rev = revisaoDe(p)
    const c = p.checks
    linhas.push(
      <Box key="pr" flexDirection="row" justifyContent="space-between" width={colunas}>
        <Box flexDirection="row" flexShrink={1}>
          <Text bold>{`⎇ PR #${p.numero} `}</Text>
          <Text dimColor wrap="truncate-end">{p.titulo}</Text>
          {c.ok + c.falhou + c.rodando > 0 ? <Text dimColor>{'  checks '}</Text> : null}
          {c.ok ? <Text color="success">{`✓${c.ok} `}</Text> : null}
          {c.falhou ? <Text color="error">{`✕${c.falhou} `}</Text> : null}
          {c.rodando ? <Text color="warning">{`●${c.rodando} `}</Text> : null}
          {rev ? <Text dimColor>{' · '}</Text> : null}
          {rev ? (rev.cor ? <Text color={rev.cor}>{rev.texto}</Text> : <Text dimColor>{rev.texto}</Text>) : null}
        </Box>
        <Button key="abrir-pr" label="abrir PR" dimColor onPress={() => abrir(p.url)} />
      </Box>,
    )
  }

  // Runs de mais de um commit na tela: cada linha diz de qual commit é.
  const comSha = new Set(v.workflows.map(w => w.sha)).size > 1
  const rotulos = rotulosAbrir(v.workflows)
  v.workflows.forEach((w, i) => linhas.push(...linhaWorkflow(t, w, colunas, comSha, rotulos[i] ?? `abrir ${w.id}`, pinta, abrir)))
  return <Box flexDirection="column">{linhas}</Box>
}

function linhaWorkflow(
  t: Tabela, w: Workflow, colunas: number, comSha: boolean, rotulo: string, pinta: Pinta, abrir: (url: string) => void,
): JSX.Element[] {
  const { Box, Text, Button } = t
  const sha = comSha ? w.sha.slice(0, 7) : ''
  const out = [
    <Box key={`wf-${w.id}`} flexDirection="row" justifyContent="space-between" width={colunas}>
      <Box flexDirection="row" flexWrap="wrap" flexShrink={1} columnGap={1}>
        {pinta(w.situacao, `${ICONE[w.situacao]} ${w.nome}`, true)}
        {sha ? <Text dimColor>{sha}</Text> : null}
        {w.jobs.map((j, i) => <Box key={`j-${i}`}>{pinta(j.situacao, rotuloJob(j))}</Box>)}
      </Box>
      <Button key={`abrir-${w.id}`} label={rotulo} dimColor onPress={() => abrir(w.url)} />
    </Box>,
  ]
  w.jobs.forEach((j, i) => {
    if (j.situacao !== 'falhou') return
    out.push(
      <Box key={`f-${w.id}-${i}`} flexDirection="row" paddingLeft={2}>
        <Text color="error">{`✕ ${j.nome}`}</Text>
        <Text dimColor wrap="truncate-end">{j.passo ? ` → falhou em “${j.passo}”` : ' → falhou'}</Text>
      </Box>,
    )
  })
  return out
}
