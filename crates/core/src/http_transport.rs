//! Reusable stateless transports. Authentication and model resolvers stay request-scoped.
use anyhow::Result;
use parking_lot::Mutex;
use std::{collections::HashMap, sync::LazyLock, time::Duration};

pub(crate) async fn bounded_body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    const MESSAGE: &str =
        "Service response exceeds the safety size limit. Retry with a smaller response.";
    anyhow::ensure!(
        response
            .content_length()
            .is_none_or(|len| len <= limit as u64),
        MESSAGE
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(chunk.len() <= limit.saturating_sub(bytes.len()), MESSAGE);
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) fn llm(local: bool) -> Result<reqwest_genai::Client> {
    static CLIENTS: LazyLock<Mutex<HashMap<bool, reqwest_genai::Client>>> =
        LazyLock::new(Default::default);
    let mut clients = CLIENTS.lock();
    if let Some(client) = clients.get(&local) {
        return Ok(client.clone());
    }
    let builder = reqwest_genai::Client::builder()
        .redirect(reqwest_genai::redirect::Policy::none())
        .timeout(Duration::from_secs(if local { 600 } else { 120 }));
    let client = if local { builder.no_proxy() } else { builder }.build()?;
    clients.insert(local, client.clone());
    Ok(client)
}
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub(crate) enum Transport {
    Translation,
    Catalog,
    OllamaMetadata,
}
pub(crate) fn client(transport: Transport) -> Result<reqwest::Client> {
    static CLIENTS: LazyLock<Mutex<HashMap<Transport, reqwest::Client>>> =
        LazyLock::new(Default::default);
    let mut clients = CLIENTS.lock();
    if let Some(client) = clients.get(&transport) {
        return Ok(client.clone());
    }
    let builder = reqwest::Client::builder().timeout(Duration::from_secs(match transport {
        Transport::Translation => 60,
        Transport::Catalog => 20,
        Transport::OllamaMetadata => 5,
    }));
    let builder = if transport == Transport::Catalog {
        builder
    } else {
        builder.redirect(reqwest::redirect::Policy::none())
    };
    let client = if transport == Transport::OllamaMetadata {
        builder.no_proxy()
    } else {
        builder
    }
    .build()?;
    clients.insert(transport, client.clone());
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn unknown_length_bodies_enforce_the_limit_between_chunks() -> Result<()> {
        for (last, valid) in [("", true), ("x", false)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let url = format!("http://{}", listener.local_addr()?);
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await?;
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).await?;
                    header.push(byte[0]);
                    anyhow::ensure!(header.len() < 16384, "Mock header exceeded limit");
                }
                stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await?;
                for chunk in ["中", "文", last].into_iter().filter(|s| !s.is_empty()) {
                    stream
                        .write_all(format!("{:x}\r\n{chunk}\r\n", chunk.len()).as_bytes())
                        .await?;
                }
                stream.write_all(b"0\r\n\r\n").await?;
                Ok::<_, anyhow::Error>(())
            });
            let response = client(Transport::OllamaMetadata)?.get(url).send().await?;
            assert_eq!(response.content_length(), None);
            let result = bounded_body(response, 6).await;
            server.await??;
            if valid {
                assert_eq!(result?, "中文".as_bytes());
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("safety size limit")
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn bounded_catalog_metadata_and_conventional_bodies_keep_exact_bytes() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        for (body, limit, valid) in [("中文", 6, true), ("中文x", 6, false), ("", 0, true)] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(body.as_bytes()))
                .expect(1)
                .mount(&server)
                .await;
            let response = client(Transport::Catalog)
                .unwrap()
                .get(server.uri())
                .send()
                .await
                .unwrap();
            let result = bounded_body(response, limit).await;
            if valid {
                assert_eq!(result.unwrap(), body.as_bytes());
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("safety size limit")
                );
            }
        }
    }

    #[tokio::test]
    async fn transports_reuse_connections_without_reusing_authentication() -> Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let accepted = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = tokio_util::sync::CancellationToken::new();
        let (count, headers, token) = (accepted.clone(), requests.clone(), stop.clone());
        let server = tokio::spawn(async move {
            let mut workers = tokio::task::JoinSet::new();
            loop {
                let stream = tokio::select! { _ = token.cancelled() => break, stream = listener.accept() => stream?.0 };
                count.fetch_add(1, Ordering::SeqCst);
                let token = token.clone();
                let headers = headers.clone();
                workers.spawn(async move {
                    let mut stream = stream;
                    loop {
                        let mut request = Vec::new();
                        while !request.ends_with(b"\r\n\r\n") {
                            let mut byte = [0];
                            let n = tokio::select! { _ = token.cancelled() => return Ok::<_, anyhow::Error>(()), n = stream.read(&mut byte) => n? };
                            if n == 0 { return Ok(()); }
                            request.push(byte[0]);
                            anyhow::ensure!(request.len() <= 16384, "Mock request exceeded header limit");
                        }
                        headers.lock().push(String::from_utf8(request)?);
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nok").await?;
                    }
                });
            }
            while let Some(result) = workers.join_next().await {
                result??;
            }
            Ok::<_, anyhow::Error>(())
        });
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            for token in ["first-dummy-key", "second-dummy-key"] {
                assert_eq!(
                    client(Transport::Translation)?
                        .get(&url)
                        .bearer_auth(token)
                        .send()
                        .await?
                        .text()
                        .await?,
                    "ok"
                );
            }
            for token in ["third-dummy-key", "fourth-dummy-key"] {
                assert_eq!(
                    llm(true)?
                        .get(&url)
                        .bearer_auth(token)
                        .send()
                        .await?
                        .text()
                        .await?,
                    "ok"
                );
            }
            Ok::<_, anyhow::Error>(())
        })
        .await;
        stop.cancel();
        server.await??;
        result??;
        assert_eq!(
            accepted.load(Ordering::SeqCst),
            2,
            "One persistent connection per pinned reqwest runtime"
        );
        let requests = requests.lock();
        assert_eq!(requests.len(), 4);
        for (request, key) in requests.iter().zip([
            "first-dummy-key",
            "second-dummy-key",
            "third-dummy-key",
            "fourth-dummy-key",
        ]) {
            assert!(request.contains(key));
            assert_eq!(
                request
                    .to_ascii_lowercase()
                    .matches("authorization:")
                    .count(),
                1
            );
        }
        Ok(())
    }
}
