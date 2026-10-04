"""Contém apenas os escritores filhos do Rust e comprova o encerramento deles."""
from __future__ import annotations

import os
import json
import signal
import subprocess
import sys
import time
import uuid
import threading
import tempfile
from pathlib import Path
from dataclasses import dataclass

import psutil
_record_guard = threading.RLock()


class WindowsJob:
    def __init__(self, name=None, *, existing=False):
        import ctypes as c
        from ctypes import wintypes as w
        self.c = c
        self.api = c.WinDLL('kernel32', use_last_error=True)
        signatures = {
            'CreateJobObjectW': ([w.LPVOID, w.LPCWSTR], w.HANDLE),
            'OpenJobObjectW': ([w.DWORD, w.BOOL, w.LPCWSTR], w.HANDLE),
            'SetInformationJobObject': ([w.HANDLE, c.c_int, w.LPVOID, w.DWORD], w.BOOL),
            'AssignProcessToJobObject': ([w.HANDLE, w.HANDLE], w.BOOL),
            'TerminateJobObject': ([w.HANDLE, w.UINT], w.BOOL),
            'QueryInformationJobObject': ([w.HANDLE, c.c_int, w.LPVOID, w.DWORD, w.LPVOID], w.BOOL),
            'CloseHandle': ([w.HANDLE], w.BOOL),
        }
        for api_name, (args, result) in signatures.items():
            fn = getattr(self.api, api_name)
            fn.argtypes, fn.restype = args, result
        class Basic(c.Structure):
            _fields_ = [('user', c.c_int64), ('kernel', c.c_int64), ('flags', w.DWORD),
                ('min_ws', c.c_size_t), ('max_ws', c.c_size_t), ('limit', w.DWORD),
                ('affinity', c.c_size_t), ('priority', w.DWORD), ('scheduling', w.DWORD)]
        class Io(c.Structure):
            _fields_ = [(name, c.c_uint64) for name in ('read_ops','write_ops','other_ops','read_bytes','write_bytes','other_bytes')]
        class Extended(c.Structure):
            _fields_ = [('basic', Basic), ('io', Io), ('process_memory', c.c_size_t),
                ('job_memory', c.c_size_t), ('peak_process', c.c_size_t), ('peak_job', c.c_size_t)]
        class Accounting(c.Structure):
            _fields_ = [(name, c.c_int64) for name in ('user','kernel','period_user','period_kernel')] + [
                (name, w.DWORD) for name in ('faults','total','active','terminated')]
        self.Accounting = Accounting
        self.name = name or 'Global\\Hangar-runtime-' + uuid.uuid4().hex
        self.handle = (self.api.OpenJobObjectW(0x0004 | 0x0008, False, self.name) if existing
            else self.api.CreateJobObjectW(None, self.name))
        if not self.handle:
            if existing and c.get_last_error() == 2:
                return
            raise c.WinError(c.get_last_error())
        if existing:
            return
        try:
            limits = Extended()
            limits.basic.flags = 0x2000  # O handle pertence só à vigia; fechar encerra o grupo.
            self.check(self.api.SetInformationJobObject(self.handle, 9, c.byref(limits), c.sizeof(limits)))
        except BaseException:
            self.close()
            raise

    def check(self, result):
        if not result:
            raise self.c.WinError(self.c.get_last_error())

    def assign(self, proc):
        self.check(self.api.AssignProcessToJobObject(self.handle, int(proc._handle)))

    def terminate(self):
        self.check(self.api.TerminateJobObject(self.handle, 1))

    def active(self):
        data = self.Accounting()
        self.check(self.api.QueryInformationJobObject(self.handle, 1, self.c.byref(data), self.c.sizeof(data), None))
        return data.active

    def close(self):
        if self.handle:
            self.check(self.api.CloseHandle(self.handle))
            self.handle = None


@dataclass
class Containment:
    pid: int
    birth: float
    job: WindowsJob | None = None
    cleaned: bool = False
    record: Path | None = None
    record_lease: object | None = None
    life: str | None = None
    members: dict | None = None


def spawn_contained(args, *, env, record_path=None):
    from app.runtime_coordinator import WriterLease
    record = (record_path if isinstance(record_path, Path) else Path(record_path)) if record_path is not None else None
    lease = WriterLease(record.with_suffix('.lock')) if record is not None else None
    if record is not None:
        try:
            reconcile_startup(record, lease=lease)
        except BaseException:
            lease.close()
            raise
    job = None
    proc = None
    try:
        job = WindowsJob() if sys.platform == 'win32' else None
        kw = {'creationflags':0x08000000 | 0x00000004} if job else {'start_new_session':True}
        proc = subprocess.Popen(args, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, **kw)
        proc.runtime_containment = Containment(proc.pid, psutil.Process(proc.pid).create_time(), job,
            record=record, record_lease=lease, life=uuid.uuid4().hex)
        if job:
            job.assign(proc)
            psutil.Process(proc.pid).resume()
        elif os.getpgid(proc.pid) != proc.pid:
            raise RuntimeError('filho Rust sem grupo próprio')
        if record is not None:
            refresh_members(proc)
        return proc
    except BaseException:
        if proc is not None:
            proc.kill()
            proc.wait(timeout=5)
        if job:
            job.close()
        if lease:
            lease.close()
        raise


def _group_members(containment):
    members = []
    for proc in psutil.process_iter(['pid','status','create_time']):
        try:
            if os.getpgid(proc.pid) == containment.pid and proc.status() != psutil.STATUS_ZOMBIE:
                if proc.pid == containment.pid and proc.create_time() != containment.birth:
                    raise RuntimeError('identidade do grupo Rust mudou')
                members.append(proc.pid)
        except (psutil.NoSuchProcess, ProcessLookupError):
            continue
    return members


def cleanup(proc, timeout=5):
    containment = getattr(proc, 'runtime_containment', None)
    if containment is None:
        raise RuntimeError('fim dos descendentes Rust sem prova de contenção')
    if containment.cleaned:
        return True
    deadline = time.monotonic() + timeout
    if containment.job:
        containment.job.terminate()
        while containment.job.active():
            if time.monotonic() >= deadline:
                raise RuntimeError('descendentes Rust ainda ativos no Job')
            time.sleep(.02)
        containment.job.close()
    else:
        while members := _group_members(containment):
            try:
                os.killpg(containment.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            if time.monotonic() >= deadline:
                raise RuntimeError('descendentes Rust ainda ativos no grupo')
            time.sleep(.02)
    proc.poll()
    containment.cleaned = True
    if containment.record is not None:
        with _record_guard:
            if containment.record.exists():
                saved = json.loads(containment.record.read_bytes())
                if saved.get('life') != containment.life:
                    raise RuntimeError('registro de contenção mudou durante a limpeza')
                containment.record.unlink()
        containment.record_lease.close()
    return True


def record_path():
    from app import log_paths
    return log_paths.base().parent / 'runtime-process.json'


def boot_identity():
    if sys.platform.startswith('linux'):
        return Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    return None


def _same_process(pid, birth):
    try:
        proc = psutil.Process(pid)
        return proc.create_time() == birth and proc.status() != psutil.STATUS_ZOMBIE
    except psutil.NoSuchProcess:
        return False


def refresh_members(proc):
    containment = proc.runtime_containment
    if containment.record is None or containment.cleaned:
        return
    if not _same_process(containment.pid, containment.birth):
        return
    members = {}
    if containment.job is None:
        for pid in _group_members(containment):
            try:
                members[str(pid)] = psutil.Process(pid).create_time()
            except psutil.NoSuchProcess:
                continue
    containment.members = {**(containment.members or {}), **members}
    owner = psutil.Process(os.getpid())
    data = dict(version=1, life=containment.life, platform=sys.platform, owner_pid=owner.pid,
        owner_birth=owner.create_time(), pid=containment.pid, birth=containment.birth,
        pgid=containment.pid, members=containment.members, boot=boot_identity(),
        job_name=containment.job.name if containment.job else None)
    from app import atomico
    with _record_guard:
        path = containment.record
        path.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(json.dumps(data).encode())
            stream.flush()
            os.fsync(stream.fileno())
        try:
            atomico.substituir(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)


def reconcile_startup(path=None, *, allow_current=False, lease=None):
    from app.runtime_coordinator import WriterLease
    path = path or record_path()
    path = path if isinstance(path, Path) else Path(path)
    with _record_guard:
        saved = json.loads(path.read_bytes()) if path.exists() else None
        if saved:
            if (saved.get('version') != 1 or saved.get('platform') != sys.platform
                    or type(saved.get('pid')) is not int or saved['pid'] <= 0
                    or saved.get('pgid') != saved['pid'] or type(saved.get('owner_pid')) is not int
                    or not isinstance(saved.get('members'), dict) or not isinstance(saved.get('life'), str)
                    or not isinstance(saved.get('birth'), (int,float)) or not isinstance(saved.get('owner_birth'), (int,float))):
                raise RuntimeError('registro de contenção inválido; escrita suspensa')
            owner_alive = _same_process(saved['owner_pid'], saved['owner_birth'])
            rust_alive = _same_process(saved['pid'], saved['birth'])
            if owner_alive:
                if allow_current and saved['owner_pid'] == os.getpid() and rust_alive:
                    return True
                raise RuntimeError('outro ciclo do backend ainda possui escritores Rust')
        acquired = WriterLease(path.with_suffix('.lock')) if lease is None else lease
        try:
            if saved is None:
                return True
            if sys.platform.startswith('linux') and saved.get('boot') != boot_identity():
                path.unlink()
                return True
            if sys.platform == 'win32':
                import re
                if not isinstance(saved.get('job_name'), str) or not re.fullmatch(r'Global\\Hangar-runtime-[a-f0-9]{32}', saved['job_name']):
                    raise RuntimeError('Job anterior sem identidade; escrita suspensa')
                job = WindowsJob(saved['job_name'], existing=True)
                if job.handle is not None:
                    orphan = Containment(saved['pid'], saved['birth'], job)
                    holder = type('ContainedProcess', (), {'runtime_containment':orphan, 'poll':lambda self:None})()
                    cleanup(holder)
            else:
                containment = Containment(saved['pid'], saved['birth'])
                members = _group_members(containment)
                if members and not any(_same_process(pid, saved['members'].get(str(pid))) for pid in members):
                    raise RuntimeError('grupo órfão sem prova de nascimento; escrita suspensa')
                holder = type('ContainedProcess', (), {'runtime_containment':containment, 'poll':lambda self:None})()
                cleanup(holder)
            if path.exists() and json.loads(path.read_bytes()).get('life') != saved['life']:
                raise RuntimeError('contenção mudou durante a reconciliação')
            path.unlink(missing_ok=True)
            return True
        finally:
            if lease is None:
                acquired.close()
