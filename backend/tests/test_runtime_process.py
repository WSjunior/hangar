import os
import subprocess
import sys
import time
from pathlib import Path

import psutil
import pytest


def _ended(proc):
    # Zumbi no POSIX; no Windows o encerrado segue listado sem threads até o último handle fechar.
    try:
        return not proc.is_running() or proc.status() == psutil.STATUS_ZOMBIE or proc.num_threads() == 0
    except psutil.NoSuchProcess:
        return True


def test_exited_windows_process_held_by_a_handle_is_not_the_same_process(monkeypatch):
    from app import runtime_process
    class Exited:
        def __init__(self, pid): pass
        def create_time(self): return 7.0
        def status(self): return psutil.STATUS_RUNNING
        def num_threads(self): return 0
    monkeypatch.setattr(runtime_process.psutil, 'Process', Exited)
    monkeypatch.setattr(runtime_process.sys, 'platform', 'win32')
    assert runtime_process._same_process(1, 7.0) is False
    monkeypatch.setattr(runtime_process.sys, 'platform', 'linux')
    assert runtime_process._same_process(1, 7.0) is True


def test_child_cleanup_after_abrupt_parent_death(tmp_path):
    from app.runtime_process import spawn_contained, cleanup
    pid_path = tmp_path / 'child.pid'
    script = "import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); open(sys.argv[1],'w').write(str(p.pid)); time.sleep(60)"
    proc = spawn_contained([sys.executable, '-c', script, str(pid_path)], env=dict(os.environ))
    try:
        deadline = time.monotonic()+5
        while not pid_path.exists() and time.monotonic()<deadline: time.sleep(.01)
        child = psutil.Process(int(pid_path.read_text()))
        assert child.is_running()
        proc.kill()
        proc.wait(5)
        assert cleanup(proc, timeout=3) is True
        assert _ended(child)
        assert proc.runtime_containment.cleaned
    finally:
        cleanup(proc, timeout=3)


@pytest.mark.skipif(sys.platform == 'win32', reason='POSIX: grupo auxiliar dentro da sessão contida')
def test_cleanup_contains_separate_auxiliary_group_in_same_rust_session(tmp_path):
    from app.runtime_process import spawn_contained, cleanup, refresh_members
    pid_path = tmp_path / 'auxiliary.pid'
    script = "import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)'],process_group=0);open(sys.argv[1],'w').write(str(p.pid));time.sleep(60)"
    proc = spawn_contained([sys.executable, '-c', script, str(pid_path)], env=dict(os.environ), record_path=tmp_path / 'containment.json')
    other = subprocess.Popen([sys.executable, '-c', 'import time;time.sleep(60)'])
    try:
        deadline = time.monotonic() + 5
        while not pid_path.exists() and time.monotonic() < deadline:
            time.sleep(.01)
        auxiliary = psutil.Process(int(pid_path.read_text()))
        assert os.getpgid(auxiliary.pid) != proc.pid and os.getsid(auxiliary.pid) == proc.pid
        refresh_members(proc)
        assert str(auxiliary.pid) in proc.runtime_containment.members
        proc.kill(); proc.wait(5)
        assert cleanup(proc, timeout=3) is True
        assert not auxiliary.is_running() or auxiliary.status() == psutil.STATUS_ZOMBIE
        assert other.poll() is None
    finally:
        cleanup(proc, timeout=3)
        other.kill(); other.wait(5)


@pytest.mark.skipif(sys.platform != 'win32', reason='Windows Job real no CI')
def test_windows_job_contains_grandchild_before_resume(tmp_path):
    test_child_cleanup_after_abrupt_parent_death(tmp_path)


def test_restart_after_fake_backend_and_rust_death_cleans_old_writer(tmp_path):
    import json
    from app import runtime_process
    record=tmp_path/'containment.json'
    child_file=tmp_path/'child.pid'
    ready=tmp_path/'ready.json'
    fake_backend=tmp_path/'backend.py'
    fake_backend.write_text('''import json,os,sys,time
from pathlib import Path
from app.runtime_process import spawn_contained, refresh_members
script="import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); open(sys.argv[1],'w').write(str(p.pid)); time.sleep(60)"
p=spawn_contained([sys.executable,'-c',script,sys.argv[2]],env=dict(os.environ),record_path=Path(sys.argv[1]))
while not Path(sys.argv[2]).exists() or not Path(sys.argv[2]).read_text():time.sleep(.01)
refresh_members(p)
Path(sys.argv[3]).write_text(json.dumps({'rust':p.pid,'child':int(Path(sys.argv[2]).read_text())}))
time.sleep(60)
''')
    backend=subprocess.Popen([sys.executable,str(fake_backend),str(record),str(child_file),str(ready)],
        env={**os.environ,'PYTHONPATH':str(Path(__file__).resolve().parents[1])})
    info=None
    try:
        deadline=time.monotonic()+6
        while not ready.exists() and backend.poll() is None and time.monotonic()<deadline:time.sleep(.01)
        assert ready.exists()
        info=json.loads(ready.read_text())
        backend.kill()
        backend.wait(5)
        try:psutil.Process(info['rust']).kill()
        except psutil.NoSuchProcess:pass
        assert runtime_process.reconcile_startup(record) is True
        child=psutil.Process(info['child']) if psutil.pid_exists(info['child']) else None
        assert child is None or not child.is_running() or child.status()==psutil.STATUS_ZOMBIE
        assert not record.exists()
    finally:
        if backend.poll() is None:
            backend.kill()
            backend.wait(5)
        if record.exists():runtime_process.reconcile_startup(record)


def test_recycled_group_leader_birth_blocks_cleanup_without_killing(tmp_path):
    import json
    from app import runtime_process
    proc=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'],start_new_session=sys.platform!='win32')
    record=tmp_path/'containment.json'
    try:
        birth=psutil.Process(proc.pid).create_time()
        record.write_text(json.dumps({'version':1,'life':'test','platform':sys.platform,'owner_pid':999999999,
            'owner_birth':1,'pid':proc.pid,'birth':birth-1,'pgid':proc.pid,'members':{str(proc.pid):birth-1},
            'boot':runtime_process.boot_identity(),'job_name':None}))
        with pytest.raises(RuntimeError):runtime_process.reconcile_startup(record)
        assert proc.poll() is None and record.exists()
    finally:
        proc.kill()
        proc.wait(5)


def test_windows_jobs_keep_unique_and_explicit_names_with_fake_api(monkeypatch):
    import ctypes
    from app.runtime_process import WindowsJob
    calls=[]
    class Function:
        def __init__(self,name):self.name=name
        def __call__(self,*args):
            calls.append((self.name,args))
            return 100 if self.name in {'CreateJobObjectW','OpenJobObjectW'} else 1
    class Api:
        def __init__(self):self.functions={}
        def __getattr__(self,name):return self.functions.setdefault(name,Function(name))
    monkeypatch.setattr(ctypes,'WinDLL',lambda *args,**kwargs:Api(),raising=False)
    monkeypatch.setattr(ctypes,'get_last_error',lambda:0,raising=False)
    first,second=WindowsJob(),WindowsJob()
    supplied='Global\\Hangar-runtime-'+('a'*32)
    existing=WindowsJob(supplied,existing=True)
    try:
        assert first.name.startswith('Global\\Hangar-runtime-') and first.name!=second.name
        assert existing.name==supplied
        assert [args[2] for name,args in calls if name=='OpenJobObjectW']==[supplied]
    finally:
        first.close();second.close();existing.close()
