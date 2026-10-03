use hangar_server::costs::index::{Fold, Index, IndexError, Progress};
use hangar_server::costs::rows::FoldOutput;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Default, Serialize, Deserialize)]
struct Empty;
impl Fold for Empty {
    fn line(&mut self, raw: &[u8]) { assert!(raw != b"panic\n", "falha sintética"); }
    fn close(&mut self) -> FoldOutput { FoldOutput { costs: vec![], usage: vec![], areas: None } }
}
fn fold(_: &Path) -> Empty { Empty }
#[test]
fn generation_is_shared_by_clones_and_changes_only_after_committed_mutations() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("file.jsonl"); std::fs::write(&path, "{}\n").unwrap();
    let index = Index::open(&d.path().join("index")).unwrap();
    let clone = index.clone();
    let before = index.generation();
    index.sync_file(&path, &fold, "empty:1", "scope", "", &|_| vec![]).unwrap();
    assert!(index.generation() > before);
    assert_eq!(index.generation(), clone.generation());
    let before = index.generation();
    assert!(!index.sync("scope", &[path.clone()], &fold, "empty:1", "", &|_| vec![], &Progress::default()).unwrap());
    assert_eq!(index.generation(), before);
    std::fs::remove_file(&path).unwrap();
    assert!(clone.forget_outside(&[]).unwrap());
    assert!(index.generation() > before);
}
#[test]
fn partial_success_before_reader_panic_advances_generation() {
    hangar_server::install_panic_hook();
    let d = tempfile::tempdir().unwrap();
    let good = d.path().join("good.jsonl"); let bad = d.path().join("bad.jsonl");
    std::fs::write(&good, "{}\n").unwrap(); std::fs::write(&bad, "panic\n").unwrap();
    let index = Index::open(&d.path().join("index")).unwrap();
    let before = index.generation();
    assert!(matches!(index.sync("scope", &[good, bad], &fold, "empty:1", "", &|_| vec![], &Progress::default()), Err(IndexError::ReaderPanic)));
    assert!(index.generation() > before, "o commit parcial invalida mesmo com retorno de erro");
}

#[derive(Default, Serialize, Deserialize)]
struct WithAreas;
impl Fold for WithAreas {
    fn line(&mut self, _: &[u8]) {}
    fn close(&mut self) -> FoldOutput {
        use hangar_server::costs::areas::{AreaEntries, AreaHeader};
        FoldOutput { costs: vec![], usage: vec![], areas: Some(AreaEntries {
            header: AreaHeader { fonte: None, session_id: None, subagente: None }, turns: vec![],
        }) }
    }
}
#[test]
fn area_rebuild_ownership_and_forgetting_all_advance_generation() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("file.jsonl"); std::fs::write(&path, "{}\n").unwrap();
    let index = Index::open(&d.path().join("index")).unwrap();
    let new = |_: &Path| WithAreas;
    let progress = Progress::default();
    index.sync("first", &[path.clone()], &new, "area:1", "1", &|_| vec![], &progress).unwrap();
    let before = index.generation();
    let never_read = |_: &Path| -> WithAreas { panic!("não deve reler"); };
    index.sync("first", &[path.clone()], &never_read, "area:1", "2", &|_| vec![], &progress).unwrap();
    assert!(index.generation() > before);
    let before = index.generation();
    index.sync("second", &[path.clone()], &never_read, "area:1", "2", &|_| vec![], &progress).unwrap();
    assert!(index.generation() > before, "a troca de escopo em autocommit invalida");
    let before = index.generation();
    std::fs::remove_file(&path).unwrap();
    index.sync("second", &[], &new, "area:1", "2", &|_| vec![], &progress).unwrap();
    assert!(index.generation() > before);
}

#[test]
fn failed_area_write_rolls_back_without_changing_generation() {
    hangar_server::install_panic_hook();
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("file.jsonl"); std::fs::write(&path, "{}\n").unwrap();
    let index = Index::open(&d.path().join("index")).unwrap();
    let before = index.generation();
    let new = |_: &Path| WithAreas;
    let fail = |_: &hangar_server::costs::rows::AreaEntries| -> Vec<hangar_server::costs::rows::UsoLinha> { panic!("falha sintética"); };
    assert!(index.sync_file(&path, &new, "area:1", "scope", "1", &fail).is_none());
    assert_eq!(index.generation(), before);
}

#[test]
fn committed_batch_generation_is_visible_before_later_reader_panic() {
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};
    hangar_server::install_panic_hook();
    let d = tempfile::tempdir().unwrap();
    let good = d.path().join("good.jsonl"); let bad = d.path().join("bad.jsonl");
    std::fs::write(&good, "{}\n").unwrap(); std::fs::write(&bad, "panic\n").unwrap();
    let pool = Arc::new(rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap());
    let index = Index::open(&d.path().join("index")).unwrap().with_pool(pool);
    let before = index.generation();
    let gate = (Mutex::new(false), Condvar::new());
    std::thread::scope(|threads| {
        let scan = threads.spawn(|| {
            let new = |path: &Path| {
                if path == bad {
                    let locked = gate.0.lock().unwrap();
                    let _ = gate.1.wait_timeout_while(locked, Duration::from_secs(4), |released| !*released).unwrap();
                }
                Empty
            };
            index.sync("scope", &[good, bad.clone()], &new, "empty:1", "", &|_| vec![], &Progress::default())
        });
        let start = Instant::now();
        while index.generation() == before && start.elapsed() < Duration::from_secs(3) { std::thread::yield_now(); }
        let observed = index.generation();
        *gate.0.lock().unwrap() = true; gate.1.notify_all();
        assert!(matches!(scan.join().unwrap(), Err(IndexError::ReaderPanic)));
        assert!(observed > before, "a geração avança logo após o commit do lote bom");
        assert!(index.generation() >= observed);
    });
}

#[test]
fn rebuilding_the_schema_invalidates_rows_even_when_reading_only() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("index");
    let index = Index::open(&dir).unwrap();
    let before = index.generation();
    let conn = rusqlite::Connection::open(dir.join(hangar_server::costs::index::FILE_NAME)).unwrap();
    conn.execute("UPDATE meta SET v='999' WHERE k='esquema'", []).unwrap();
    drop(conn);
    assert!(index.read_costs(None, None, None).unwrap().is_empty());
    assert!(index.generation() > before, "a recriação do esquema também confirma uma mudança");
}
