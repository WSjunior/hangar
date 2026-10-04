import { describe, it, expect } from 'vitest';
import { computeEditDiff, extractEdits, extractFilePath, pairRows, diffFromPatch, diffFromEdits, numberedDiff, wordSpans, extractEditPaths } from './editdiff';

describe('computeEditDiff', () => {
  it('troca simples de 1 linha vira par alterado', () => {
    const d = computeEditDiff('envFrom:\n  - secretRef:', 'env:\n  - name: X');
    expect(d.del).toBe(2);
    expect(d.add).toBe(2);
    expect(d.rows).toEqual([
      { left: { num: 1, text: 'envFrom:' }, right: { num: 1, text: 'env:' } },
      { left: { num: 2, text: '  - secretRef:' }, right: { num: 2, text: '  - name: X' } },
    ]);
  });

  it('insercao pura: so lado direito, esquerdo vazio', () => {
    const d = computeEditDiff('a\nb', 'a\nx\ny\nb');
    expect(d.add).toBe(2);
    expect(d.del).toBe(0);
    expect(d.rows).toEqual([
      { left: { num: 1, text: 'a' }, right: { num: 1, text: 'a' } },
      { left: null, right: { num: 2, text: 'x' } },
      { left: null, right: { num: 3, text: 'y' } },
      { left: { num: 2, text: 'b' }, right: { num: 4, text: 'b' } },
    ]);
  });

  it('remocao pura: so lado esquerdo, direito vazio', () => {
    const d = computeEditDiff('a\nx\ny\nb', 'a\nb');
    expect(d.del).toBe(2);
    expect(d.add).toBe(0);
    expect(d.rows[1]).toEqual({ left: { num: 2, text: 'x' }, right: null });
    expect(d.rows[2]).toEqual({ left: { num: 3, text: 'y' }, right: null });
  });

  it('bloco trocado com sobra: pareia pelo indice e o resto fica sem par', () => {
    // 2 linhas saem, 4 entram (o caso do print: envFrom -> env)
    const d = computeEditDiff('old1\nold2', 'new1\nnew2\nnew3\nnew4');
    expect(d.rows).toEqual([
      { left: { num: 1, text: 'old1' }, right: { num: 1, text: 'new1' } },
      { left: { num: 2, text: 'old2' }, right: { num: 2, text: 'new2' } },
      { left: null, right: { num: 3, text: 'new3' } },
      { left: null, right: { num: 4, text: 'new4' } },
    ]);
  });

  it('old vazio (arquivo/trecho novo): tudo adicionado', () => {
    const d = computeEditDiff('', 'linha1\nlinha2');
    expect(d.del).toBe(0);
    expect(d.add).toBe(2);
    expect(d.rows.every((r) => r.left === null)).toBe(true);
  });

  it('identicos: tudo contexto, sem add/del', () => {
    const d = computeEditDiff('a\nb', 'a\nb');
    expect(d.add).toBe(0);
    expect(d.del).toBe(0);
    expect(d.rows).toHaveLength(2);
  });

  it('trailing newline nao vira linha fantasma', () => {
    const d = computeEditDiff('a\n', 'a\nb\n');
    expect(d.add).toBe(1);
    expect(d.rows).toHaveLength(2);
  });

  it('linhas movidas voltam como del+add (diff valido, nao necessariamente "move")', () => {
    const d = computeEditDiff('a\nb\nc', 'b\na\nc');
    // toda sequencia del/add fecha: reconstruir old e new a partir das ops
    const oldLines = d.ops.filter((o) => o.op !== 'add').map((o) => o.text);
    const newLines = d.ops.filter((o) => o.op !== 'del').map((o) => o.text);
    expect(oldLines).toEqual(['a', 'b', 'c']);
    expect(newLines).toEqual(['b', 'a', 'c']);
  });

  it('grande demais pro Myers cai no fallback prefixo/sufixo sem travar', () => {
    const n = 1200; // 1200*1200 > MYERS_MAX_PRODUCT
    const a = Array.from({ length: n }, (_, i) => `a${i}`);
    const b = Array.from({ length: n }, (_, i) => `b${i}`);
    a[0] = b[0] = 'igual';
    const d = computeEditDiff(a.join('\n'), b.join('\n'));
    expect(d.rows[0]).toEqual({ left: { num: 1, text: 'igual' }, right: { num: 1, text: 'igual' } });
    expect(d.del).toBe(n - 1);
    expect(d.add).toBe(n - 1);
  });
});

describe('pairRows', () => {
  it('ops vazias -> sem linhas', () => {
    expect(pairRows([])).toEqual([]);
  });

  it('del nao adjacente a add nao pareia', () => {
    const rows = pairRows([
      { op: 'del', text: 'x' },
      { op: 'ctx', text: 'meio' },
      { op: 'add', text: 'y' },
    ]);
    expect(rows).toEqual([
      { left: { num: 1, text: 'x' }, right: null },
      { left: { num: 2, text: 'meio' }, right: { num: 1, text: 'meio' } },
      { left: null, right: { num: 2, text: 'y' } },
    ]);
  });
});

describe('extractEdits', () => {
  it('Edit: extrai old/new do input', () => {
    const edits = extractEdits('Edit', { file_path: '/x', old_string: 'a', new_string: 'b' });
    expect(edits).toEqual([{ oldText: 'a', newText: 'b' }]);
  });

  it('MultiEdit: extrai a lista edits[]', () => {
    const edits = extractEdits('MultiEdit', {
      file_path: '/x',
      edits: [
        { old_string: 'a', new_string: 'b' },
        { old_string: 'c', new_string: 'd', replace_all: true },
      ],
    });
    expect(edits).toHaveLength(2);
    expect(edits?.[1]).toEqual({ oldText: 'c', newText: 'd' });
  });

  it('shape desconhecido -> null (card cai no pre cru)', () => {
    expect(extractEdits('Edit', { file_path: '/x' })).toBeNull();
    expect(extractEdits('MultiEdit', { edits: [{ old_string: 1 }] })).toBeNull();
    expect(extractEdits('MultiEdit', { edits: [] })).toBeNull();
    expect(extractEdits('Bash', { command: 'ls' })).toBeNull();
    expect(extractEdits('Edit', null)).toBeNull();
  });

  it('Pi (edit minusculo): edits[] com oldText/newText + path', () => {
    const edits = extractEdits('edit', {
      path: '/x.ts',
      edits: [{ oldText: 'a', newText: 'b' }, { oldText: 'c', newText: 'd' }],
    });
    expect(edits).toEqual([
      { oldText: 'a', newText: 'b' },
      { oldText: 'c', newText: 'd' },
    ]);
  });

  it('nome case-insensitive e misto de dialetos', () => {
    expect(extractEdits('EDIT', { oldText: 'a', newText: 'b' })).toEqual([{ oldText: 'a', newText: 'b' }]);
    expect(extractEdits('multiedit', { edits: [{ oldText: 'a', newText: 'b' }] })).toHaveLength(1);
  });

  it('Write: vira uma edicao de old vazio (tudo adicao) nos tres dialetos', () => {
    // Claude Code (file_path), Pi (write minusculo + path), Kimi (Write + path).
    expect(extractEdits('Write', { file_path: '/x.ts', content: 'a\nb' })).toEqual([{ oldText: '', newText: 'a\nb' }]);
    expect(extractEdits('write', { path: '/x.ts', content: 'a\nb' })).toEqual([{ oldText: '', newText: 'a\nb' }]);
    expect(extractEdits('Write', { path: '/x.ts', content: 'a', mode: 'overwrite' })).toEqual([{ oldText: '', newText: 'a' }]);
  });

  it('Write com mode append: mesmo diff (as linhas do append SAO adicao)', () => {
    const edits = extractEdits('Write', { path: '/x.ts', mode: 'append', content: 'nova\n' });
    expect(edits).toEqual([{ oldText: '', newText: 'nova\n' }]);
    expect(computeEditDiff(edits![0].oldText, edits![0].newText).add).toBe(1);
  });

  it('Write sem conteudo (ou vazio) cai no pre cru', () => {
    expect(extractEdits('Write', { file_path: '/x.ts' })).toBeNull();
    expect(extractEdits('Write', { file_path: '/x.ts', content: '' })).toBeNull();
    expect(extractEdits('Write', { file_path: '/x.ts', content: 42 })).toBeNull();
  });

  it('extractFilePath: file_path (Claude) ou path (Pi)', () => {
    expect(extractFilePath({ file_path: '/a.ts' })).toBe('/a.ts');
    expect(extractFilePath({ path: '/b.ts' })).toBe('/b.ts');
    expect(extractFilePath({})).toBe('');
    expect(extractFilePath(null)).toBe('');
  });

  it('pega o primeiro caminho quando o patch toca vários arquivos', () => {
    // `String(['/a','/b'])` daria "/a,/b", que não é caminho nenhum — e daqui sai a linguagem do
    // realce.
    expect(extractFilePath({ file_path: ['/a.ts', '/b.ts'] })).toBe('/a.ts');
    expect(extractFilePath({ file_path: [] })).toBe('');
  });
});

// A edição de arquivo do Codex (`apply_patch`) chega como TEXTO de patch, não como campos. O
// patch abaixo é copiado de um rollout real desta máquina (fixture do ticket 06).
describe('extractEdits com apply_patch', () => {
  const PATCH = [
    '*** Begin Patch',
    '*** Update File: /home/u/proj/tests/test_x.py',
    '@@',
    ' from app.registry import SessionRegistry',
    ' from app.adapters.codex import sessions as codex_sessions',
    '+from app.adapters.codex import adapter as codex_adapter',
    ' from app.adapters.codex.adapter import CodexAdapter',
    '*** End Patch',
  ].join('\n');

  it('reconstrói os dois lados a partir do patch', () => {
    const edits = extractEdits('apply_patch', { code: PATCH });
    expect(edits).toHaveLength(1);
    // Lado velho: contexto. Lado novo: contexto + a linha que entrou, na posição dela.
    expect(edits![0].oldText.split('\n')).toEqual([
      'from app.registry import SessionRegistry',
      'from app.adapters.codex import sessions as codex_sessions',
      'from app.adapters.codex.adapter import CodexAdapter',
    ]);
    expect(edits![0].newText).toContain('from app.adapters.codex import adapter as codex_adapter');
    // E o diff resultante é o que o cartão desenha: uma adição, nenhuma remoção.
    const d = computeEditDiff(edits![0].oldText, edits![0].newText);
    expect([d.add, d.del]).toEqual([1, 0]);
  });

  it('separa um bloco por arquivo', () => {
    const dois = [
      '*** Begin Patch',
      '*** Update File: /a.py',
      '@@',
      '-um',
      '+dois',
      '*** Update File: /b.py',
      '@@',
      '-tres',
      '+quatro',
      '*** End Patch',
    ].join('\n');
    // Um bloco só juntaria o fim de um arquivo com o começo do outro, e o diff inventaria uma
    // mudança que ninguém fez.
    expect(extractEdits('apply_patch', { code: dois })).toEqual([
      { oldText: 'um', newText: 'dois' },
      { oldText: 'tres', newText: 'quatro' },
    ]);
  });

  it('linha de conteúdo que começa com *** não fecha o bloco', () => {
    // Um divisor de markdown sendo editado começa igual a um marcador. Fechando ali, o resto do
    // diff sumia do cartão sem aviso.
    const patch = [
      '*** Begin Patch',
      '*** Update File: /doc.md',
      '@@',
      ' titulo',
      '+*** divisor ***',
      '+depois',
      '*** End Patch',
    ].join('\n');
    const edits = extractEdits('apply_patch', { code: patch })!;
    expect(edits).toHaveLength(1);
    expect(edits[0].newText).toBe('titulo\n*** divisor ***\ndepois');
  });

  it('o renomear não vira linha do arquivo', () => {
    // `*** Move to:` vem DENTRO de um bloco `Update File:` (é o rename do formato do Codex). Caindo
    // no ramo de contexto, ele aparecia nos dois lados do diff como se fosse texto do arquivo.
    const patch = [
      '*** Begin Patch',
      '*** Update File: /velho.ts',
      '*** Move to: /novo.ts',
      '@@',
      '-antes',
      '+depois',
      '*** End Patch',
    ].join('\n');
    expect(extractEdits('apply_patch', { code: patch })).toEqual([
      { oldText: 'antes', newText: 'depois' },
    ]);
  });

  it('a nota "No newline at end of file" não vira linha do arquivo', () => {
    const patch = [
      '*** Begin Patch',
      '*** Update File: /a.txt',
      '@@',
      '-antes',
      '\\ No newline at end of file',
      '+depois',
      '*** End Patch',
    ].join('\n');
    expect(extractEdits('apply_patch', { code: patch })).toEqual([
      { oldText: 'antes', newText: 'depois' },
    ]);
  });

  it('sem patch reconhecível volta null e o cartão cai no texto cru', () => {
    expect(extractEdits('apply_patch', { code: 'nada disso' })).toBeNull();
    expect(extractEdits('apply_patch', {})).toBeNull();
  });
});

describe('wordSpans', () => {
  it('marca só o número trocado', () => {
    expect(wordSpans('const TIMEOUT_MS = 4000;', 'const TIMEOUT_MS = 8000;'))
      .toEqual({ del: [[19, 23]], add: [[19, 23]] });
  });

  it('junta tokens vizinhos num trecho só', () => {
    expect(wordSpans('x = a+b;', 'x = a??b;')).toEqual({ del: [[5, 6]], add: [[5, 7]] });
  });

  it('o espaço em comum separa dois trechos', () => {
    expect(wordSpans('return a + b;', 'return a - c;'))
      .toEqual({ del: [[9, 10], [11, 12]], add: [[9, 10], [11, 12]] });
  });

  it('linhas com quase nada em comum ficam sem realce', () => {
    expect(wordSpans('import x from "y";', 'export default function z() {}')).toBeNull();
  });

  it('linha vazia de um dos lados fica sem realce', () => {
    expect(wordSpans('', 'novo')).toBeNull();
  });

  it('linha com muitos tokens fica sem realce (evita Myers caro)', () => {
    // 400 palavras em cada lado = ~799 tokens, excedem o teto de 600. Deve retornar null sem rodar Myers.
    const manyWords = Array.from({ length: 400 }, (_, i) => `word${i}`).join(' ');
    const changed = Array.from({ length: 400 }, (_, i) => i === 0 ? 'changed' : `word${i}`).join(' ');
    expect(wordSpans(manyWords, changed)).toBeNull();
  });
});

describe('diffFromPatch', () => {
  const hunk = { old_start: 8, new_start: 8, lines: [' ', '-const T = 4000;', '+const T = 8000;', ' '] };

  it('numera pela linha real do arquivo', () => {
    const d = diffFromPatch([hunk]);
    expect(d.hunks[0].map((l) => [l.op, l.oldNum, l.newNum])).toEqual([
      ['ctx', 8, 8], ['del', 9, null], ['add', null, 9], ['ctx', 10, 10],
    ]);
    expect([d.add, d.del]).toEqual([1, 1]);
  });

  it('põe o realce por palavra no par removida/adicionada', () => {
    const [, del, add] = diffFromPatch([hunk]).hunks[0];
    expect(del.spans).toEqual([[10, 14]]);
    expect(add.spans).toEqual([[10, 14]]);
  });

  it('ignora a nota de fim de arquivo sem deslocar a numeração', () => {
    const d = diffFromPatch([{ old_start: 3, new_start: 3, lines: ['-a', '\\ No newline at end of file', '+b', ' c'] }]);
    expect(d.hunks[0].map((l) => [l.op, l.oldNum, l.newNum, l.text])).toEqual([
      ['del', 3, null, 'a'], ['add', null, 3, 'b'], ['ctx', 4, 4, 'c'],
    ]);
  });

  it('um trecho por hunk, cada um com a sua posição', () => {
    const d = diffFromPatch([hunk, { old_start: 40, new_start: 40, lines: ['+x'] }]);
    expect(d.hunks.length).toBe(2);
    expect(d.hunks[1][0].newNum).toBe(40);
  });
});

describe('diffFromEdits e numberedDiff', () => {
  it('sem patch numera a partir de 1, como hoje', () => {
    const d = diffFromEdits([{ oldText: 'a\nb', newText: 'a\nc' }]);
    expect(d.hunks[0].map((l) => [l.op, l.oldNum, l.newNum])).toEqual([
      ['ctx', 1, 1], ['del', 2, null], ['add', null, 2],
    ]);
  });

  it('numberedDiff prefere o patch e cai nas edições quando ele falta ou vem vazio', () => {
    const edits = [{ oldText: 'a', newText: 'b' }];
    expect(numberedDiff([{ old_start: 9, new_start: 9, lines: ['-a', '+b'] }], edits).hunks[0][0].oldNum).toBe(9);
    expect(numberedDiff(null, edits).hunks[0][0].oldNum).toBe(1);
    expect(numberedDiff([], edits).hunks[0][0].oldNum).toBe(1);
  });
});

describe('extractEditPaths', () => {
  const dois = ['*** Begin Patch', '*** Update File: /a.py', '@@', '-um', '+dois', '*** Delete File: /x.txt',
    '*** Add File: /b.py', '+novo', '*** End Patch'].join('\n');

  it('apply_patch dá o caminho de cada bloco que virou edição, na mesma ordem', () => {
    expect(extractEdits('apply_patch', { code: dois })).toHaveLength(2);
    expect(extractEditPaths('apply_patch', { code: dois })).toEqual(['/a.py', '/b.py']);
  });

  it('um arquivo só repete o mesmo caminho, uma vez por edição', () => {
    const input = { file_path: '/f.ts', edits: [{ old_string: 'a', new_string: 'b' }, { old_string: 'c', new_string: 'd' }] };
    expect(extractEditPaths('MultiEdit', input)).toEqual(['/f.ts', '/f.ts']);
    expect(extractEditPaths('Read', { file_path: '/f.ts' })).toBeNull();
  });

  it('os trechos levam o caminho de cada edição e o patch do resultado não leva', () => {
    const edits = extractEdits('apply_patch', { code: dois })!;
    const paths = extractEditPaths('apply_patch', { code: dois })!;
    expect(numberedDiff(null, edits, paths).paths).toEqual(['/a.py', '/b.py']);
    expect(numberedDiff([{ old_start: 1, new_start: 1, lines: ['-a', '+b'] }], edits, paths).paths).toBeUndefined();
    expect(numberedDiff(null, edits).paths).toBeUndefined();
  });

  it('edição sem diferença some do diff e leva o caminho junto', () => {
    const d = diffFromEdits([{ oldText: '', newText: '' }, { oldText: 'a', newText: 'b' }], ['/vazio', '/b']);
    expect(d.paths).toEqual(['/b']);
  });
});

describe('wordSpans com texto acentuado', () => {
  it('cruza o limiar de 40% como o nativo', () => {
    expect(wordSpans('// Ação já está pronta', '// Função não está pronta, então espera')).not.toBeNull();
  });
});

describe('pareamento com contagens diferentes', () => {
  it('a primeira removida faz par com a adicionada e a segunda fica sem marcas', () => {
    const d = diffFromEdits([{ oldText: 'let value = 4000;\nother line here', newText: 'let value = 8000;' }]);
    expect(d.hunks[0].map((l) => [l.op, l.spans.length > 0])).toEqual([['del', true], ['del', false], ['add', true]]);
  });
});
