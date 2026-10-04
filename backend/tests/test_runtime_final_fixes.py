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
