//! Bound sockets as well as HTTP work. Idle connections have a 30-second read deadline.
use std::{
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{Instant, Sleep},
};
use tokio_util::sync::CancellationToken;
pub struct BoundedListener {
    listener: TcpListener,
    slots: Arc<Semaphore>,
    stop: CancellationToken,
}
impl BoundedListener {
    pub fn new(listener: TcpListener, count: usize, stop: CancellationToken) -> Self {
        Self {
            listener,
            slots: Arc::new(Semaphore::new(count)),
            stop,
        }
    }
}
pub struct Connection {
    io: TcpStream,
    _permit: OwnedSemaphorePermit,
    deadline: Pin<Box<Sleep>>,
    stop: CancellationToken,
}
impl AsyncRead for Connection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        use std::future::Future;
        if self.stop.is_cancelled() {
            return Poll::Ready(Ok(()));
        }
        match Pin::new(&mut self.io).poll_read(cx, buf) {
            Poll::Ready(result) => {
                self.deadline
                    .as_mut()
                    .reset(Instant::now() + Duration::from_secs(30));
                Poll::Ready(result)
            }
            Poll::Pending => {
                if self.deadline.as_mut().poll(cx).is_ready() {
                    Poll::Ready(Err(std::io::ErrorKind::TimedOut.into()))
                } else {
                    Poll::Pending
                }
            }
        }
    }
}
impl AsyncWrite for Connection {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let result = Pin::new(&mut self.io).poll_write(cx, data);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            self.deadline
                .as_mut()
                .reset(Instant::now() + Duration::from_secs(30));
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}
impl axum::serve::Listener for BoundedListener {
    type Io = Connection;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            if let Ok((io, addr)) = self.listener.accept().await {
                if !addr.ip().is_ipv4()
                    || !super::security::private_peer(match addr.ip() {
                        std::net::IpAddr::V4(ip) => ip,
                        _ => unreachable!(),
                    })
                {
                    continue;
                }
                if let Ok(permit) = self.slots.clone().try_acquire_owned() {
                    return (
                        Connection {
                            io,
                            _permit: permit,
                            deadline: Box::pin(tokio::time::sleep(Duration::from_secs(30))),
                            stop: self.stop.clone(),
                        },
                        addr,
                    );
                }
            } else {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}
#[derive(Clone, Copy)]
pub struct Peer(pub SocketAddr);
impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, BoundedListener>>
    for Peer
{
    fn connect_info(stream: axum::serve::IncomingStream<'_, BoundedListener>) -> Self {
        Self(*stream.remote_addr())
    }
}
