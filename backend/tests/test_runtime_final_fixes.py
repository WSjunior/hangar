import copy
import json
import asyncio
from datetime import datetime, timezone

from app import runtime_queue as rq
from app.runtime_receipt import ReceiptIndex
from test_runtime_queue import CLOCK, open_store, fill
from test_runtime_terminal import isolated_owner, live_owner
import pytest


def test_absent_transcript_echo_confirms_only_one_equal_input_after_compact_reopen(tmp_path, monkeypatch):
    monkeypatch.setattr(rq, '_RECENT_CALLS', 8)
    store = open_store(tmp_path)
    transcript = tmp_path / 'absent.jsonl'
    index = ReceiptIndex('claude', 'sid')
    cursor = index.capture(transcript)
    assert cursor['file_identity'] is None
    for number in (1, 2):
        operation = f'input-{number}'
        for kind, action in [
            ('append', {'kind': 'append', 'text': 'same-input', 'entry_id': operation}),
            ('prepare', {'kind': 'prepare', 'id': operation, 'entry_id': operation, 'payload': {'kind': 'input'}}),
            ('cursor', {'kind': 'bind_dispatch', 'id': operation, 'cursor': cursor}),
            ('dispatch', {'kind': 'begin_dispatch', 'id': operation, 'wire_id': operation}),
        ]:
            store.exec(1, f'{kind}:{number}', CLOCK, action)
    timestamp = datetime.fromtimestamp(cursor['absent_since'] + 1, timezone.utc).isoformat()
    transcript.write_text(json.dumps({'type': 'user', 'uuid': 'one-echo', 'sessionId': 'sid',
        'timestamp': timestamp, 'message': {'content': 'same-input'}}) + '\n')
    index.scan(transcript)
    proof = index.match_after(cursor, store.state['rows'][0], store.state['used_occurrences'])
    assert proof is not None
    assert store.exec(1, 'confirm-first', CLOCK, {'kind': 'confirm_occurrence', 'id': 'input-1', 'proof': proof}) is True
    fill(store, 20)
    store = open_store(tmp_path)
    assert list(store.state['used_occurrences']) == [proof['occurrence']['id']]
    assert index.match_after(cursor, store.state['rows'][1], store.state['used_occurrences']) is None
    duplicate = copy.deepcopy(proof)
    assert store.exec(1, 'confirm-second', CLOCK, {'kind': 'confirm_occurrence', 'id': 'input-2', 'proof': duplicate}) is False
    assert store.state['rows'][0]['confirmed'] is True
    assert not store.state['rows'][1].get('confirmed')


def test_startup_unresolved_same_terminal_life_recovers_without_second_restart(tmp_path, monkeypatch):
    from app import runtime_coordinator as rc, runtime_terminal as terminal, pqueue
    from app.adapters.claude_headless import sessions as claude_sessions
    from app.adapters.codex import sessions as codex_sessions
    owner, slot, collected = live_owner(monkeypatch, tmp_path)
    slot.store.exec(1, 'append', CLOCK, {'kind': 'append', 'text': 'pending-input', 'entry_id': 'pending'})
    original = copy.deepcopy(slot.store.state['rows'])
    owner.close_python_leases()
    monkeypatch.setattr(pqueue, '_queue_dir', lambda: tmp_path)
    monkeypatch.setattr(claude_sessions, 'list_all', lambda: [])
    monkeypatch.setattr(codex_sessions, 'list_all', lambda: [])
    identity = {'value': None}
    monkeypatch.setattr(terminal, '_collect', lambda name: identity['value'])
    restored = rc.RuntimeCoordinator()
    async def flow():
        await restored.start_sessions({'claude': object(), 'codex': object()})
        waiting = restored.slot('session')
        assert waiting.phase == rc.Phase.RecoveringPython and waiting.store is None and waiting.lease is None
        identity['value'] = collected
        assert await restored.prepare_session('session', 'claude') is True
        current = restored.slot('session')
        assert current.phase == rc.Phase.Python and current.lease is not None and not current.lease.closed
        assert current.binding.key == slot.binding.key and current.binding.generation == slot.binding.generation
        assert current.store.state['rows'] == original
        current.lease.close()
    asyncio.run(flow())


@pytest.mark.parametrize('plugin_live', [False, True])
@pytest.mark.parametrize('transition', ['restart', 'same_sid_generation'])
def test_unknown_fill_blocks_next_entry_and_submit_across_transition(tmp_path, monkeypatch, plugin_live, transition):
    from app import runtime_terminal as terminal, terminal_input as ti
    owner, slot, _ = live_owner(monkeypatch, tmp_path)
    screen, effects = {'text': ''}, []
    phase = {'first': True}
    real_service = terminal._service
    def service(coordinator, descriptor, operation, request, kind, payload):
        if kind == 'terminal_facts':
            return dict(binding=payload['binding'], ready=True, idle=True, open_question=False,
                plugin_live=phase['first'] or plugin_live, plugin_user=False, native=None, clipboard_available=False)
        if kind == 'terminal_publish':
            effects.append('publication')
            screen['text'] = payload['publication']['text']
            return 'unknown'
        return real_service(coordinator, descriptor, operation, request, kind, payload)
    monkeypatch.setattr(terminal, '_service', service)
    monkeypatch.setattr(terminal, '_python_prompt', lambda *args: effects.extend(['cleanup', 'text', 'Enter']) or ('sent', False, ''))
    monkeypatch.setattr(ti, '_capture', lambda name: '─'*30+'\n❯ '+screen['text']+'\n'+'─'*30+'\n')
    async def flow():
        nonlocal owner, slot
        assert (await owner.op('session', {'kind': 'submit', 'text': 'A'}, 'A'))['disposition'] == 'unknown'
        slot.store.exec(1, 'append-B', CLOCK, {'kind': 'append', 'text': 'B', 'entry_id': 'B'})
        phase['first'] = False
        if transition == 'restart':
            owner.close_python_leases()
            owner, slot, _ = live_owner(monkeypatch, tmp_path)
        else:
            async def noop():
                return None
            await owner.change('session', noop, advance=True, reopen=False)
            assert slot.binding.generation == 2 and slot.binding.meta['terminal']['conversation'] == 'sid'
        assert (await owner.op('session', {'kind': 'drain'}, 'drain-B'))['sent'] == 0
        result = await owner.op('session', {'kind': 'submit', 'text': 'C'}, 'C')
        assert result['disposition'] == 'deferred'
        assert effects == ['publication'] and screen['text'] == 'A'
        assert slot.store.state['operations']['A']['status'] == 'unknown'
        assert not next(row for row in slot.store.state['rows'] if row['id'] == 'B')['delivered']
        assert not next(row for row in slot.store.state['rows'] if row['id'] == 'C')['delivered']
    asyncio.run(flow())


def test_unknown_composer_allows_control_and_proven_new_conversation_without_retyping(tmp_path, monkeypatch):
    from app import runtime_terminal as terminal, terminal_input as ti
    owner, slot, collected = live_owner(monkeypatch, tmp_path)
    phase, effects = {'first': True}, []
    real_service = terminal._service
    def service(coordinator, descriptor, operation, request, kind, payload):
        if kind == 'terminal_facts':
            return dict(binding=payload['binding'], ready=True, idle=True, open_question=False,
                plugin_live=phase['first'], plugin_user=False, native=None, clipboard_available=False)
        if kind == 'terminal_publish':
            effects.append('A-publication'); return 'unknown'
        return real_service(coordinator, descriptor, operation, request, kind, payload)
    monkeypatch.setattr(terminal, '_service', service)
    monkeypatch.setattr(ti.TerminalInput, 'interrupt', lambda *args, **kwargs: effects.append('control'))
    monkeypatch.setattr(terminal, '_python_prompt', lambda *args: effects.append('new-conversation-input') or ('sent', False, ''))
    async def flow():
        assert (await owner.op('session', {'kind': 'submit', 'text': 'A'}, 'A'))['disposition'] == 'unknown'
        phase['first'] = False
        assert (await owner.op('session', {'kind': 'control', 'control': 'interrupt', 'payload': {'clear': False}}, 'control'))['disposition'] == 'accepted'
        assert (await owner.op('session', {'kind': 'submit', 'text': 'B'}, 'B'))['disposition'] == 'deferred'
        assert effects == ['A-publication', 'control']
        collected.update(session_id='new-sid', jsonl=str(tmp_path / 'new-sid.jsonl'))
        assert await owner.prepare_session('session', 'claude')
        assert (await owner.op('session', {'kind': 'submit', 'text': 'C'}, 'C'))['disposition'] == 'accepted'
        assert effects == ['A-publication', 'control', 'new-conversation-input']
    asyncio.run(flow())


def test_lost_fill_ack_retains_publication_across_generation_and_stale_ack_cannot_confirm_new_conversation(monkeypatch):
    import time
    from app import plugin_bridge as pb
    name = 'publication-barrier-test'
    conversation = {'sid': 'A'}
    monkeypatch.setattr(pb, 'tracked_session_id', lambda name: conversation['sid'])
    monkeypatch.setattr(pb, 'CONFIRMA_S', .02)
    async def flow():
        pb._loop = asyncio.get_running_loop()
        queue = pb._waiters[name] = asyncio.Queue()
        pb._donos[name] = ('test', {'fill', 'receipt_v2'}, time.monotonic())
        first = asyncio.create_task(asyncio.to_thread(pb.publish_terminal, name, 'A', 1,
            {'id': 'pub-A', 'text': 'A', 'mode': 'fill'}, lambda: None))
        published = await asyncio.wait_for(queue.get(), .2)
        assert published['text'] == 'A'
        assert await first == 'unknown'
        assert pb._publications[name]['id'] == 'pub-A'
        assert await asyncio.to_thread(pb.publish_terminal, name, 'A', 2,
            {'id': 'pub-B', 'text': 'B', 'mode': 'fill'}, lambda: None) == 'unknown'
        assert queue.empty()
        conversation['sid'] = 'B'
        second = asyncio.create_task(asyncio.to_thread(pb.publish_terminal, name, 'B', 3,
            {'id': 'pub-C', 'text': 'C', 'mode': 'fill'}, lambda: None))
        assert (await asyncio.wait_for(queue.get(), .2))['publication_id'] == 'pub-C'
        body = dict(sessao=name, token='test', ok=True)
        pb._terminal_ack(pb.FilledBody(**body, publication_id='pub-A', generation=1, session_id='A'), 'fill')
        assert pb._publications[name]['result'] == 'unknown'
        pb._terminal_ack(pb.FilledBody(**body, publication_id='pub-C', generation=3, session_id='B'), 'fill')
        assert await second == 'filled'
    try:
        asyncio.run(flow())
    finally:
        pb._publications.pop(name, None)
        pb._waiters.pop(name, None)
        pb._donos.pop(name, None)


def _uncertain_terminal_inputs(tmp_path, texts):
    store = open_store(tmp_path)
    transcript = tmp_path / 'sid.jsonl'
    index = ReceiptIndex('claude', 'sid')
    cursor = index.capture(transcript)
    for text in texts:
        for kind, action in [
            ('append', {'kind': 'append', 'text': text, 'entry_id': text}),
            ('prepare', {'kind': 'prepare', 'id': text, 'entry_id': text,
                         'payload': {'kind': 'input', 'payload': {'text': text, '_terminal_generation': 1}}}),
            ('cursor', {'kind': 'bind_dispatch', 'id': text, 'cursor': cursor}),
            ('dispatch', {'kind': 'begin_dispatch', 'id': text, 'wire_id': f'terminal:1:{text}'}),
            ('finish', {'kind': 'finish', 'id': text, 'status': 'unknown',
                        'result': {'operation_id': text, 'disposition': 'unknown', 'payload': {'cleanup': 'uncertain'}}}),
        ]:
            store.exec(1, f'{kind}:{text}', CLOCK, action)
    timestamp = datetime.fromtimestamp(cursor['absent_since'] + 1, timezone.utc).isoformat()
    transcript.write_text(''.join(json.dumps({'type': 'user', 'uuid': f'echo-{text}', 'sessionId': 'sid',
        'timestamp': timestamp, 'message': {'content': text}}) + '\n' for text in texts))
    index.scan(transcript)
    def confirm(text):
        row = next(row for row in store.state['rows'] if row['id'] == text)
        proof = index.match_after(cursor, row, store.state['used_occurrences'])
        return store.exec(1, f'confirm:{text}', CLOCK, {'kind': 'confirm_occurrence', 'id': text, 'proof': proof})
    return store, confirm


@pytest.mark.parametrize('resolution', ['confirm', 'late_accepted'])
def test_terminal_write_barrier_follows_remaining_uncertain_input_and_lifts_after_last(tmp_path, resolution):
    store, confirm = _uncertain_terminal_inputs(tmp_path, ['A', 'B'])
    assert store.state['runtime_state']['terminal_write_barrier']['operation_id'] == 'A'
    assert rq.terminal_write_blocked(store.state, 'sid')
    if resolution == 'confirm':
        assert confirm('A') is True
    else:
        store.exec(1, 'late:A', CLOCK, {'kind': 'finish', 'id': 'A', 'status': 'accepted',
            'result': {'operation_id': 'A', 'disposition': 'accepted', 'payload': {}}})
    # B continua incerta na mesma conversa: a trava passa para ela.
    assert store.state['runtime_state']['terminal_write_barrier']['operation_id'] == 'B'
    assert rq.terminal_write_blocked(store.state, 'sid')
    assert confirm('B') is True
    assert 'terminal_write_barrier' not in store.state['runtime_state']
    assert not rq.terminal_write_blocked(store.state, 'sid')
    reopened = open_store(tmp_path)
    assert not rq.terminal_write_blocked(reopened.state, 'sid')


def test_confirmed_uncertain_input_releases_drain_and_next_submit(tmp_path, monkeypatch):
    from app import runtime_terminal as terminal, terminal_input as ti
    owner, slot, _ = live_owner(monkeypatch, tmp_path)
    phase, effects = {'first': True}, []
    real_service = terminal._service
    def service(coordinator, descriptor, operation, request, kind, payload):
        if kind == 'terminal_facts':
            return dict(binding=payload['binding'], ready=True, idle=True, open_question=False,
                plugin_live=phase['first'], plugin_user=False, native=None, clipboard_available=False)
        if kind == 'terminal_publish':
            effects.append('A-publication'); return 'unknown'
        return real_service(coordinator, descriptor, operation, request, kind, payload)
    monkeypatch.setattr(terminal, '_service', service)
    monkeypatch.setattr(terminal, '_python_prompt', lambda *args: effects.append('typed') or ('sent', False, ''))
    async def flow():
        assert (await owner.op('session', {'kind': 'submit', 'text': 'A'}, 'A'))['disposition'] == 'unknown'
        phase['first'] = False
        slot.store.exec(1, 'append-B', CLOCK, {'kind': 'append', 'text': 'B', 'entry_id': 'B'})
        assert (await owner.op('session', {'kind': 'drain'}, 'drain-blocked'))['sent'] == 0
        operation = slot.store.state['operations']['A']
        transcript = tmp_path / 'sid.jsonl'
        cursor = operation['dispatch_cursor']
        stamp = cursor.get('absent_since') or 0
        transcript.write_text(json.dumps({'type': 'user', 'uuid': 'echo-A', 'sessionId': 'sid',
            'timestamp': datetime.fromtimestamp(stamp + 1, timezone.utc).isoformat(),
            'message': {'content': 'A'}}) + '\n')
        await owner.op('session', {'kind': 'confirm'}, 'confirm-A')
        assert slot.store.state['operations']['A']['status'] == 'confirmed'
        assert (await owner.op('session', {'kind': 'drain'}, 'drain-free'))['sent'] == 1
        assert effects == ['A-publication', 'typed']
    asyncio.run(flow())


@pytest.mark.parametrize('ok', [True, False])
def test_late_matching_fill_ack_drops_retained_publication_without_touching_queue(monkeypatch, ok):
    import time
    from app import plugin_bridge as pb
    name = 'publication-late-ack-test'
    monkeypatch.setattr(pb, 'tracked_session_id', lambda name: 'A')
    monkeypatch.setattr(pb, 'CONFIRMA_S', .02)
    async def flow():
        pb._loop = asyncio.get_running_loop()
        queue = pb._waiters[name] = asyncio.Queue()
        pb._donos[name] = ('test', {'fill', 'receipt_v2'}, time.monotonic())
        first = asyncio.create_task(asyncio.to_thread(pb.publish_terminal, name, 'A', 1,
            {'id': 'pub-A', 'text': 'A', 'mode': 'fill'}, lambda: None))
        await asyncio.wait_for(queue.get(), .2)
        assert await first == 'unknown'
        body = dict(sessao=name, token='test', ok=ok)
        assert pb._terminal_ack(pb.FilledBody(**body, publication_id='pub-A', generation=1, session_id='A'), 'fill')
        # O plugin respondeu: a publicação não está mais em voo; a submissão segue a cargo da fila.
        assert name not in pb._publications
        second = asyncio.create_task(asyncio.to_thread(pb.publish_terminal, name, 'A', 1,
            {'id': 'pub-B', 'text': 'B', 'mode': 'fill'}, lambda: None))
        assert (await asyncio.wait_for(queue.get(), .2))['publication_id'] == 'pub-B'
        pb._terminal_ack(pb.FilledBody(**body, publication_id='pub-B', generation=1, session_id='A'), 'fill')
        assert await second == ('filled' if ok else 'unknown')
    try:
        asyncio.run(flow())
    finally:
        pb._publications.pop(name, None)
        pb._waiters.pop(name, None)
        pb._donos.pop(name, None)


def test_awaiting_identity_record_retires_when_binding_returns_with_other_key(tmp_path, monkeypatch):
    from app import runtime_coordinator as rc, runtime_terminal as terminal, pqueue
    from app.adapters.claude_headless import sessions as claude_sessions
    from app.adapters.codex import sessions as codex_sessions
    owner, slot, collected = live_owner(monkeypatch, tmp_path)
    old_key = slot.binding.key
    owner.close_python_leases()
    monkeypatch.setattr(pqueue, '_queue_dir', lambda: tmp_path)
    monkeypatch.setattr(claude_sessions, 'list_all', lambda: [])
    monkeypatch.setattr(codex_sessions, 'list_all', lambda: [])
    identity = {'value': None}
    monkeypatch.setattr(terminal, '_collect', lambda name: identity['value'])
    restored = rc.RuntimeCoordinator()
    async def flow():
        await restored.start_sessions({'claude': object(), 'codex': object()})
        waiting = restored.slot('session')
        assert waiting.awaiting_identity and waiting.lease is None and waiting.store is None
        identity['value'] = {**collected, 'namespace': 'socket:other-pid:other-birth'}
        for _ in range(3):
            assert await restored.prepare_session('session', 'claude') is True
        current = restored.slot('session')
        assert current.binding.key != old_key
        assert current.phase == rc.Phase.Python and current.lease is not None and not current.lease.closed
        assert waiting.phase == rc.Phase.RecoveringPython and waiting.lease is None
        current.lease.close()
    asyncio.run(flow())


@pytest.mark.parametrize('text', ['/clear', '  /clear keep'])
def test_clear_with_arguments_passes_terminal_write_barrier_like_rust(tmp_path, monkeypatch, text):
    from app import runtime_terminal as terminal
    owner, slot, _ = live_owner(monkeypatch, tmp_path)
    phase, effects = {'first': True}, []
    real_service = terminal._service
    def service(coordinator, descriptor, operation, request, kind, payload):
        if kind == 'terminal_facts':
            return dict(binding=payload['binding'], ready=True, idle=True, open_question=False,
                plugin_live=phase['first'], plugin_user=False, native=None, clipboard_available=False)
        if kind == 'terminal_publish':
            effects.append('A-publication'); return 'unknown'
        return real_service(coordinator, descriptor, operation, request, kind, payload)
    monkeypatch.setattr(terminal, '_service', service)
    monkeypatch.setattr(terminal, '_python_prompt', lambda *args: effects.append('typed') or ('sent', False, ''))
    async def flow():
        assert (await owner.op('session', {'kind': 'submit', 'text': 'A'}, 'A'))['disposition'] == 'unknown'
        phase['first'] = False
        result = await owner.op('session', {'kind': 'submit', 'text': text}, 'clear')
        assert (result.get('payload') or {}).get('code') != 'terminal_write_barrier'
        assert effects == ['A-publication', 'typed']
    asyncio.run(flow())


def test_new_plugin_instance_drops_returned_publication_but_same_instance_keeps_it(monkeypatch):
    import time
    from app import plugin_bridge as pb
    name = 'publication-new-instance-test'
    monkeypatch.setattr(pb, 'tracked_session_id', lambda name: 'A')
    monkeypatch.setattr(pb, 'CONFIRMA_S', .02)
    monkeypatch.setattr(pb, 'ESPERA_S', .01)
    monkeypatch.setattr(pb, '_confere', lambda *a: None)
    monkeypatch.setattr(pb, '_conversation_mismatch', lambda *a: None)
    def pull(instance):
        return pb.pull(pb.PullBody(sessao=name, token='t', instance=instance,
            modos=['fill', 'receipt_v2'], session_id='A'))
    async def flow():
        pb._loop = asyncio.get_running_loop()
        queue = pb._waiters[name] = asyncio.Queue()
        pb._donos[name] = ('old', {'fill', 'receipt_v2'}, time.monotonic())
        first = asyncio.create_task(asyncio.to_thread(pb.publish_terminal, name, 'A', 1,
            {'id': 'pub-A', 'text': 'A', 'mode': 'fill'}, lambda: None))
        await asyncio.wait_for(queue.get(), .2)
        assert await first == 'unknown'
        await pull('old')
        assert pb._publications[name]['id'] == 'pub-A'
        pb._donos[name] = ('old', {'fill', 'receipt_v2'}, time.monotonic() - pb.ESPERA_S - 11)
        await pull('new')
        assert name not in pb._publications
    try:
        asyncio.run(flow())
    finally:
        pb._publications.pop(name, None)
        pb._waiters.pop(name, None)
        pb._donos.pop(name, None)
