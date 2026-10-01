use std::path::Path;

fn tauri_config() -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn the_webview_runs_under_a_restrictive_content_security_policy() {
    let config = tauri_config();
    let csp = config["app"]["security"]["csp"]
        .as_str()
        .expect("a Content Security Policy must be configured");

    assert!(csp.contains("default-src 'self'"), "{csp}");
    assert!(csp.contains("script-src 'self'"), "{csp}");
    assert!(csp.contains("img-src 'self' blob:"), "{csp}");
    assert!(csp.contains("object-src 'none'"), "{csp}");
    assert!(!csp.contains("unsafe-eval"), "{csp}");
    assert!(
        !csp.contains("http:") || csp.contains("http://ipc.localhost"),
        "{csp}"
    );
}
