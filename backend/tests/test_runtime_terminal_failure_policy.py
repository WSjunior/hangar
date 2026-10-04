import asyncio
import json

import pytest

from app import runtime_adapter as ra, runtime_coordinator as rc, runtime_queue as rq, runtime_terminal as terminal
from app.runtime_coordinator import Phase, WriterLease
from app.rust_server import RustOpError
from test_runtime_terminal import live_owner


class Gateway:
    instance = 'terminal-policy-test'
    alive = True

    def __init__(self, code, failures):
        self.code, self.failures = code, failures
        self.leases, self.attempts, self.events_log = {}, [], []
        self.drain_attempts = []

    def snapshot(self, target, revision=0, error=None):
        return {'key': target['key'], 'generation': target['generation'], 'revision': revision,
                'view': {'terminal': True, 'conversation': target['meta']['session_id'],
                         'deliverable': error is None, 'preserve_binding': None, 'clear_barrier': None},
                'channels': {}, 'error': error}

    async def op(self, target, command, operation_id, clock):
        name, kind = target['name'], command['kind']
        if kind == 'open':
            self.leases[name] = WriterLease(target['lock_path'])
            return {'opened': True, 'instance': self.instance, 'key': target['key'],
                    'generation': target['generation'], 'state': self.snapshot(target)}
        if kind == 'close':
            self.events_log.append(('close', name))
            self.leases.pop(name).close()
            return {'closed': True}
        if kind == 'submit' and name == 'session':
            self.attempts.append(operation_id)
            self.events_log.append(('attempt', len(self.attempts)))
            if len(self.attempts) <= self.failures:
                if self.code in {'queue_io', 'body_unknown'}:
                    store = rq.QueueStore(target['state_path'], target['projection_dir'],
                        rq.initial_state(target['key'], target['generation'], name, []))
                    body = {'kind': 'input', 'payload': {'text': command['text'],
                            '_terminal_generation': target['generation']}}
                    for index, action in enumerate([
                        {'kind': 'prepare', 'id': operation_id, 'payload': body, 'entry_id': operation_id},
                        {'kind': 'append', 'text': command['text'], 'entry_id': operation_id,
                         'delivered': False, 'ts': clock['epoch_s'], 'pre_transcript': False},
                        {'kind': 'begin_dispatch', 'id': operation_id, 'wire_id': 'terminal:1:root'},
                    ]):
                        store.exec(target['generation'], f'fake-dispatch:{index}', clock, action)
                    if self.code == 'body_unknown':
                        result = {'operation_id': operation_id, 'disposition': 'unknown',
                                  'payload': {'stage': 'write', 'code': 'mux_effect_failed'}}
                        store.exec(target['generation'], 'fake-finish', clock,
                            {'kind': 'finish', 'id': operation_id, 'status': 'unknown', 'result': result})
                        return result
                raise RustOpError(f'IPC recusou a operação (503: {self.code})', 503, self.code)
        if kind == 'snapshot':
            return self.snapshot(target, 1, self.code if self.code == 'terminal_facts' else None)
        if kind == 'drain' and self.code == 'terminal_facts':
            self.drain_attempts.append(operation_id)
            raise RustOpError('IPC recusou a operação (503: terminal_facts)', 503, self.code)
        return {'operation_id': operation_id, 'disposition': 'accepted', 'payload': {}}

    def close(self):
        for lease in self.leases.values():
            lease.close()
        self.leases.clear()


@pytest.fixture(autouse=True)
def isolated_owner(monkeypatch):
    monkeypatch.setattr(rc, '_current', None)
    monkeypatch.setattr(rq, '_coordinator', None)


def setup(monkeypatch, tmp_path, gateway):
    from app import diag, terminal_input
    owner, slot, collected = live_owner(monkeypatch, tmp_path, gateway=gateway)
    monkeypatch.setattr(terminal, '_collect', lambda name: {
        **collected, 'name': name, 'pane': '%1' if name == 'session' else '%2',
        'session_id': 'sid' if name == 'session' else 'other-sid',
        'jsonl': str(tmp_path / ('sid.jsonl' if name == 'session' else 'other-sid.jsonl')),
    })
    effects, records = [], []
    monkeypatch.setattr(terminal, '_plugin_current', lambda binding: False)
    monkeypatch.setattr(terminal_input.TerminalInput, 'send_prompt',
        lambda self, name, text: effects.append((name, text)) or 'sent')
    monkeypatch.setattr(diag, 'registrar', lambda event, level='info', **fields:
        records.append((event, level, fields)))
    original_sleep = asyncio.sleep

    async def pause(delay):
        gateway.events_log.append(('pause', delay))
        await original_sleep(0)
    monkeypatch.setattr(rc.asyncio, 'sleep', pause)
    return owner, slot, effects, records


@pytest.mark.parametrize('failures,fallback', [(3, False), (99, True)])
def test_terminal_pre_effect_retry_budget_pause_and_session_isolation(monkeypatch, tmp_path, failures, fallback):
    gateway = Gateway('runtime_lease', failures)
    owner, slot, effects, records = setup(monkeypatch, tmp_path, gateway)

    async def flow():
        other = owner.register(terminal.resolve_binding('other'))
        assert await owner.adopt('session') and await owner.adopt('other')
        result = await owner.op('session', {'kind': 'submit', 'text': 'private-test-message'}, 'root')
        assert result['disposition'] == 'accepted'
        assert gateway.attempts == ['root'] * 4
        assert gateway.events_log[:5] == [('attempt', 1), ('attempt', 2), ('attempt', 3),
                                      ('pause', rc._RETRY_PAUSE_S), ('attempt', 4)]
        assert effects == ([('session', 'private-test-message')] if fallback else [])
        assert slot.phase == (Phase.Python if fallback else Phase.Rust)
        assert slot.rust_refused == (1 if fallback else None)
        assert other.phase == Phase.Rust and other.rust_refused is None
        failures_logged = [fields for event, _, fields in records if event == 'runtime.rust_op_failed']
        assert len(failures_logged) == min(failures, 4)
        assert all(fields['codigo'] == 'RustOpError' and 'runtime_lease' in fields['detalhe']
                   for fields in failures_logged)
        assert 'private-test-message' not in json.dumps(records)
        if fallback:
            assert await owner.prepare_session('session', 'claude')
            await owner.op('session', {'kind': 'submit', 'text': 'next-input'}, 'next')
            assert gateway.attempts == ['root'] * 4
            assert effects[-1] == ('session', 'next-input')
        assert (await owner.op('other', {'kind': 'submit', 'text': 'other-input'}, 'other'))['disposition'] == 'accepted'
        assert all(name != 'other' for name, _ in effects)

    try:
        asyncio.run(flow())
    finally:
        gateway.close()
        owner.close_python_leases()


def test_terminal_possible_effect_is_not_repeated_and_row_stays_protected(monkeypatch, tmp_path):
    gateway = Gateway('queue_io', 99)
    owner, slot, effects, records = setup(monkeypatch, tmp_path, gateway)

    async def flow():
        assert await owner.adopt('session')
        with pytest.raises(RustOpError):
            await owner.op('session', {'kind': 'submit', 'text': 'private-test-message'}, 'root')
        assert gateway.attempts == ['root'] and effects == []
        assert slot.phase == Phase.Python and slot.rust_refused == 1
        assert slot.store.state['operations']['root']['status'] == 'unknown'
        assert slot.store.state['rows'][0]['delivered'] is True
        assert (await owner.op('session', {'kind': 'drain'}, 'drain'))['sent'] == 0
        assert effects == [] and not any(kind == 'pause' for kind, _ in gateway.events_log)
        assert 'private-test-message' not in json.dumps(records)

    try:
        asyncio.run(flow())
    finally:
        gateway.close()
        owner.close_python_leases()


def test_terminal_normal_refusal_does_not_retry_or_change_owner(monkeypatch, tmp_path):
    gateway = Gateway('claude_command', 99)
    owner, slot, effects, records = setup(monkeypatch, tmp_path, gateway)

    async def flow():
        assert await owner.adopt('session')
        with pytest.raises(RustOpError):
            await owner.op('session', {'kind': 'submit', 'text': 'stale-control'}, 'root')
        assert gateway.attempts == ['root'] and effects == []
        assert slot.phase == Phase.Rust and slot.rust_refused is None
        assert not any(event == 'runtime.rust_op_failed' for event, _, _ in records)
    try:
        asyncio.run(flow())
    finally:
        gateway.close()
        owner.close_python_leases()


def test_terminal_retry_never_moves_original_input_to_new_generation(monkeypatch, tmp_path):
    gateway = Gateway('runtime_lease', 3)
    owner, slot, effects, _ = setup(monkeypatch, tmp_path, gateway)
    async def rotate(delay):
        slot.binding.generation += 1
        slot.binding.meta['terminal']['generation'] = slot.binding.generation
        slot.binding.meta['terminal']['conversation'] = 'new-sid'
    monkeypatch.setattr(rc.asyncio, 'sleep', rotate)
    async def flow():
        assert await owner.adopt('session')
        with pytest.raises(RuntimeError, match='vida'):
            await owner.op('session', {'kind': 'submit', 'text': 'old-generation-input'}, 'root')
        assert gateway.attempts == ['root'] * 3 and effects == []
    try:
        asyncio.run(flow())
    finally:
        gateway.close()
        owner.close_python_leases()


def test_terminal_private_snapshot_is_accepted_without_public_state(monkeypatch, tmp_path):
    gateway = Gateway('runtime_lease', 0)
    owner, slot, _, _ = setup(monkeypatch, tmp_path, gateway)
    async def flow():
        assert await owner.adopt('session')
        data = gateway.snapshot(slot.binding.descriptor(), 1)
        event = {'key': slot.binding.key, 'generation': 1, 'revision': 1, 'channel': 'snapshot', 'data': data}
        assert ra.apply_event(slot, event)
        assert slot.cache_valid and ra.native_slot('session') is None
        assert 'public_state' not in slot.view['view'] and slot.view['channels'] == {}
        data['view']['conversation'] = 'another-conversation'
        assert not ra.apply_event(slot, event)
        assert not ra.apply_event(slot, {**event, 'revision': 2, 'channel': 'state', 'data': {}})
    try:
        asyncio.run(flow())
    finally:
        gateway.close()
        owner.close_python_leases()


def test_terminal_unknown_reply_is_logged_and_handed_over_without_replay(monkeypatch, tmp_path):
    gateway = Gateway('body_unknown', 99)
    owner, slot, effects, records = setup(monkeypatch, tmp_path, gateway)
    async def flow():
        assert await owner.adopt('session')
        result = await owner.op('session', {'kind': 'submit', 'text': 'private-test-message'}, 'root')
        assert result['disposition'] == 'unknown'
        assert gateway.attempts == ['root'] and effects == []
        assert slot.phase == Phase.Python and slot.rust_refused == 1
        assert slot.store.state['operations']['root']['status'] == 'unknown'
        assert (await owner.op('session', {'kind': 'drain'}, 'drain'))['sent'] == 0
        assert effects == [] and 'private-test-message' not in json.dumps(records)
        assert any(event == 'runtime.rust_delivery_failed' and 'terminal_delivery_unknown' in fields['detalhe']
                   for event, _, fields in records)
    try:
        asyncio.run(flow())
    finally:
        gateway.close()
        owner.close_python_leases()


def test_terminal_error_snapshot_triggers_shared_maintenance_retries_without_device(monkeypatch, tmp_path):
    original_sleep = asyncio.sleep
    gateway = Gateway('terminal_facts', 99)
    owner, slot, effects, records = setup(monkeypatch, tmp_path, gateway)
    finished = None
    async def events():
        data = gateway.snapshot(slot.binding.descriptor(), 1, 'terminal_facts')
        yield {'key': slot.binding.key, 'generation': 1, 'revision': 1, 'channel': 'snapshot', 'data': data}
        await finished.wait()
    gateway.events = events
    async def flow():
        nonlocal finished
        finished = asyncio.Event()
        owner.loop = asyncio.get_running_loop()
        assert await owner.adopt('session')
        task = asyncio.create_task(owner._events(gateway, gateway.instance))
        try:
            for _ in range(100):
                if slot.phase == Phase.Python:
                    break
                await original_sleep(.01)
            assert slot.phase == Phase.Python and slot.rust_refused == 1
            assert len(gateway.drain_attempts) == 4 and len(set(gateway.drain_attempts)) == 1
            assert [event for event in gateway.events_log if event[0] == 'pause'] == [('pause', rc._RETRY_PAUSE_S)]
            assert effects == []
            assert sum(event == 'runtime.rust_op_failed' for event, _, _ in records) == 4
        finally:
            task.cancel()
            await asyncio.gather(task, return_exceptions=True)
    try:
        asyncio.run(flow())
    finally:
        gateway.close()
        owner.close_python_leases()
