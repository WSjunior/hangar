import { transcribeUploadedForServer, uploadFileForServer, type Server } from '@hangar/core';
import * as m from '../../paraglide/messages';
import { chatStore } from '../../stores/chat';
import { finishDictation, readDictation, setDictationInFlight, writeDictation, type DictationDraft } from '../../stores/drafts';

export type DictationSource = { file: File };

export interface DictationOutcome {
  completed: ReturnType<typeof finishDictation>;
  text: string;
  raw: string;
  aviso: string;
  path: string;
  applied: string;
}

// `lost`: o ditado sumiu do armazenamento e a repetição fica só na tela; `storageIssue`: guardar a falha falhou.
export class DictationError extends Error {
  constructor(message: string, readonly lost: boolean, readonly storageIssue: string) {
    super(message);
  }
}

// O nome que o servidor deu ao áudio na pasta da conversa: `?arquivo=` e a rota de anexos pedem só ele.
export const uploadName = (path: string) => path.split(/[\\/]/).pop() || path;

// Outra montagem da conversa relê o ditado guardado quando o draftUpdate muda.
function notify(serverId: string, name: string): void {
  chatStore(serverId, name).use.setState((s) => ({ draftUpdate: s.draftUpdate + 1 }));
}

/**
 * Ditado preso à conversa de origem, fora do ciclo de vida do Composer: trocar de conversa no meio
 * não descarta o resultado. Quem chama já gravou `voice`. `check` roda antes da rede; `isActive`, na
 * chegada, decide se o texto entra no campo aberto ou fica `ready` para a montagem.
 */
export async function runDictation(server: Server, serverId: string, name: string, voice: DictationDraft,
  source: DictationSource, opts: { isActive: () => boolean; check?: () => void }): Promise<DictationOutcome> {
  setDictationInFlight(serverId, name, voice.id, true);
  notify(serverId, name);
  try {
    opts.check?.();
    // Sobe e guarda o caminho antes de transcrever: transcrição que cai não custa outra cópia.
    const { path: uploaded } = await uploadFileForServer(server, name, source.file, { audioOnly: true });
    const pending = readDictation(serverId, name);
    if (pending?.id === voice.id) writeDictation(serverId, name, { ...pending, serverPath: uploaded });
    const res = await transcribeUploadedForServer(server, name, uploadName(uploaded), { limpar: true, estilo: voice.estilo });
    const text = res.text.trim();
    if (!text) throw new Error(m.composer_transcricao_vazia());
    const raw = res.raw?.trim() ?? '';
    const aviso = res.aviso ?? '';
    const path = res.path || uploaded;
    const applied = res.estilo_aplicado ?? voice.estilo ?? 'cru';
    const completed = finishDictation(serverId, name, voice.id,
      { text, raw, issue: aviso, serverPath: path, applied }, opts.isActive());
    return { completed, text, raw, aviso, path, applied };
  } catch (e) {
    const detail = e instanceof Error ? e.message : m.composer_falha_transcricao();
    const issue = /^(501|503):/.test(detail) ? `${m.composer_ditado_indisponivel()}: ${detail}` : detail;
    let lost = false;
    let storageIssue = '';
    try {
      const latest = readDictation(serverId, name);
      lost = !latest;
      if (latest?.id === voice.id) {
        writeDictation(serverId, name, { ...latest, status: latest.text ? 'ready' : 'failed', issue });
      }
    } catch (storageError) {
      storageIssue = storageError instanceof Error ? storageError.message : m.draft_write_error();
    }
    throw new DictationError(issue, lost, storageIssue);
  } finally {
    setDictationInFlight(serverId, name, voice.id, false);
    notify(serverId, name);
  }
}
