use hangar_server::runtime::gateway::{self,RuntimeRegistry};
use std::sync::Arc;

#[tokio::test]
async fn wrong_secret_instance_or_generation_denied() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let registry = Arc::new(RuntimeRegistry::new("127.0.0.1:9".parse().unwrap(),"secret-test".into(),"instance-test".into()));
    let server = tokio::spawn(gateway::serve(listener,registry,"secret-test".into(),"instance-test".into(),hangar_server::INTERNAL_PROTOCOL));
    let client = reqwest::Client::new();
    for (secret,instance) in [("wrong","instance-test"),("secret-test","wrong")] {
        let response = client.post(format!("http://{address}/runtime/op"))
            .header("x-hangar-internal",secret).header("x-hangar-runtime-instance",instance)
            .body("not-json").send().await.unwrap();
        assert_eq!(response.status(),404);
    }
    server.abort();
}

#[tokio::test]
async fn private_port_loopback_only() {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let registry = Arc::new(RuntimeRegistry::new("127.0.0.1:9".parse().unwrap(),"secret-test".into(),"instance-test".into()));
    assert!(gateway::serve(listener,registry,"secret-test".into(),"instance-test".into(),hangar_server::INTERNAL_PROTOCOL).await.is_err());
}

#[test]
fn startup_one_line_no_secret() {
    let line = gateway::startup_line(hangar_server::INTERNAL_PROTOCOL,"instance-test",1234);
    assert!(!line.contains("secret-test"));
    let value:serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["type"],"runtime_ready");
    assert_eq!(value["port"],1234);
    assert_eq!(value.as_object().unwrap().len(),4);
}

#[tokio::test]
async fn unknown_command_fields_are_rejected() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let registry = Arc::new(RuntimeRegistry::new("127.0.0.1:9".parse().unwrap(),"secret-test".into(),"instance-test".into()));
    let server = tokio::spawn(gateway::serve(listener,registry,"secret-test".into(),"instance-test".into(),hangar_server::INTERNAL_PROTOCOL));
    let response = reqwest::Client::new().post(format!("http://{address}/runtime/op"))
        .header("x-hangar-internal","secret-test").header("x-hangar-runtime-instance","instance-test")
        .header("content-type","application/json")
        .body(serde_json::json!({"protocol":hangar_server::INTERNAL_PROTOCOL,"instance":"instance-test","key":"key",
            "generation":1,"operation_id":"op","clock":{"monotonic_s":0.0,"epoch_s":0.0},
            "command":{"kind":"detach","unexpected":true}}).to_string()).send().await.unwrap();
    assert!(!response.status().is_success());
    server.abort();
}
