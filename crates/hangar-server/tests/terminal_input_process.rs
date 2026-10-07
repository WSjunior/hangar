use hangar_server::terminal_input::{ProcessIo,TerminalIo,CommandRequest};

#[tokio::test]
async fn terminal_runtime_process_strips_internal_credentials_from_children() {
    if std::env::var_os("HANGAR_PROCESS_CHILD_CHECK").is_some() {
        let io=ProcessIo::default();
        let result=io.command(CommandRequest {program:std::env::current_exe().unwrap().to_str().unwrap().into(),args:vec!["--exact".into(),"terminal_runtime_process_probe".into(),"--nocapture".into()],stdin:vec![]}).await.unwrap();
        assert!(result.success);
        let output=String::from_utf8(result.stdout).unwrap();
        assert!(output.contains("secret=false instance=false token=false path=true"),"child environment did not match policy");
        return;
    }
    let output=std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact","terminal_runtime_process_strips_internal_credentials_from_children","--nocapture"])
        .env("HANGAR_PROCESS_CHILD_CHECK","1").env("HANGAR_INTERNAL_SECRET","test-secret")
        .env("HANGAR_RUNTIME_INSTANCE","test-instance").env("CP_AUTH_TOKEN","test-token")
        .output().unwrap();
    assert!(output.status.success(),"child verification failed: {}",String::from_utf8_lossy(&output.stdout));
}
#[test]
fn terminal_runtime_process_probe() {
    println!("secret={} instance={} token={} path={}",std::env::var_os("HANGAR_INTERNAL_SECRET").is_some(),std::env::var_os("HANGAR_RUNTIME_INSTANCE").is_some(),std::env::var_os("CP_AUTH_TOKEN").is_some(),std::env::var_os("PATH").is_some());
}
