use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use umanga_core::{providers, types::*};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn profile(endpoint: String) -> ProviderProfile {
    ProviderProfile {
        endpoint,
        protocol: "openai".into(),
        model: "mock".into(),
        simple_translation: true,
        thinking_policy: Some(Default::default()),
        ..Default::default()
    }
}
async fn translate(endpoint: String) -> anyhow::Result<Vec<TranslationItem>> {
    providers::llm(
        &profile(endpoint),
        "mock",
        &TranslationSettings::default(),
        &[Region {
            source: "こんにちは".into(),
            ..Default::default()
        }],
        None,
        "",
    )
    .await
}

#[tokio::test]
async fn oversized_llm_envelope_is_rejected_before_parsing_ignored_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices":[{"message":{"role":"assistant","content":"译文"},"finish_reason":"stop"}],
            "unused": "x".repeat(16 * 1024 * 1024)
        })))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        translate(server.uri())
            .await
            .unwrap_err()
            .to_string()
            .contains("16 MiB")
    );
}

// Raw loopback HTTP exercises unknown-length/error bodies rather than only parsed JSON.
async fn chunked_server(
    status: u16,
    count: usize,
) -> (
    String,
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Receiver<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (ready, received) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(stream.read_u8().await.unwrap());
            assert!(request.len() < 64 * 1024);
        }
        let length: usize = String::from_utf8(request)
            .unwrap()
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_owned)
            })
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0; length];
        stream.read_exact(&mut body).await.unwrap();
        stream.write_all(format!("HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let _ = ready.send(());
        let chunk = vec![b'x'; 64 * 1024];
        if count == 0 {
            // Leave a partial response open; dropping the operation must release the socket.
            stream.write_all(b"1\r\nx\r\n").await.unwrap();
            let _ = stream.read_u8().await;
        } else {
            for _ in 0..count {
                if stream.write_all(b"10000\r\n").await.is_err()
                    || stream.write_all(&chunk).await.is_err()
                    || stream.write_all(b"\r\n").await.is_err()
                {
                    break;
                }
            }
            let _ = stream.write_all(b"0\r\n\r\n").await;
        }
    });
    (endpoint, task, received)
}

#[tokio::test]
async fn chunked_success_and_error_envelopes_are_bounded_without_retry() {
    for status in [200, 429] {
        let (url, task, _) = chunked_server(status, 257).await;
        let error = tokio::time::timeout(std::time::Duration::from_secs(10), translate(url))
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("16 MiB"), "{error}");
        task.await.unwrap();
    }
}

#[tokio::test]
async fn cancelling_a_partial_envelope_releases_the_connection() {
    let (url, server, ready) = chunked_server(200, 0).await;
    let request = tokio::spawn(translate(url));
    tokio::time::timeout(std::time::Duration::from_secs(5), ready)
        .await
        .unwrap()
        .unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    tokio::time::timeout(std::time::Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn normal_unicode_and_bom_decoding_are_preserved() {
    for bom in ["", "\u{feff}"] {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(format!("{bom}{}", json!({
                "choices":[{"message":{"role":"assistant","content":"你好♥\n再见"},"finish_reason":"stop"}]
            })), "application/json; charset=utf-8"))
            .expect(1).mount(&server).await;
        let result = translate(server.uri()).await.unwrap();
        assert_eq!(result[0].target, "你好♥\n再见");
    }
}
