use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper::{body, Request, Response, Version};
use hyper_util::rt::TokioIo;
use rustls::ServerConfig;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 设置更详细的日志级别
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

    tracing::info!("Loading TLS certificates...");
    let cert_file = std::fs::File::open("cert.pem")?;
    let key_file = std::fs::File::open("key.pem")?;

    let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(cert_file))
        .filter_map(|result| result.ok())
        .collect::<Vec<_>>();

    let key =
        rustls_pemfile::private_key(&mut std::io::BufReader::new(key_file))?.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "no private key found")
        })?;

    tracing::info!("Loaded {} certificate(s)", certs.len());

    // 配置 TLS，添加 ALPN 支持
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    // 设置 ALPN 协议，支持 h2 和 http/1.1
    config.alpn_protocols = vec![b"h2".to_vec()];

    let tls_acceptor = TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Listening on https://{}", addr);

    loop {
        let (tcp_stream, peer_addr) = listener.accept().await?;
        tracing::info!("New connection from {}", peer_addr);
        let tls_acceptor = tls_acceptor.clone();

        tokio::spawn(async move {
            match tls_acceptor.accept(tcp_stream).await {
                Ok(tls_stream) => {
                    tracing::info!("TLS handshake successful with {}", peer_addr);

                    // 获取协商的 ALPN 协议
                    if let Some(protocol) = tls_stream.get_ref().1.alpn_protocol() {
                        tracing::info!(
                            "Negotiated protocol: {:?}",
                            String::from_utf8_lossy(protocol)
                        );
                    }

                    let connection = http2::Builder::new(hyper_util::rt::TokioExecutor::new());

                    if let Err(e) = connection
                        .serve_connection(
                            TokioIo::new(tls_stream),
                            service_fn(|req: Request<body::Incoming>| async move {
                                if req.version() == Version::HTTP_11 {
                                    Ok(Response::new(Full::<Bytes>::from("Hello, HTTP1.1 World")))
                                } else if req.version() == Version::HTTP_2 {
                                    Ok(Response::new(Full::<Bytes>::from("Hello, HTTP2 World")))
                                } else {
                                    // Note: it's usually better to return a Response
                                    // with an appropriate StatusCode instead of an Err.
                                    Err("not HTTP/ 1.1, abort connection")
                                }
                            }),
                        )
                        .await
                    {
                        tracing::error!("Error serving connection from {}: {}", peer_addr, e);
                    }
                }
                Err(e) => {
                    tracing::error!("TLS handshake failed with {}: {}", peer_addr, e);
                }
            }
        });
    }
}
