use axum::Router;
use axum::serve::Listener;
use rustls::ServerConfig;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;
use tokio_util::sync::CancellationToken;

const HANDSHAKE: Duration = Duration::from_secs(10);
const WAITING: usize = 64;
pub(crate) const SHAKES: usize = 16;

pub(crate) struct TlsIncoming {
    rx: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    addr: SocketAddr,
}

impl Listener for TlsIncoming {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.rx.recv().await {
            Some(next) => next,
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.addr)
    }
}

pub(crate) struct PlainIncoming {
    rx: mpsc::Receiver<(TcpStream, SocketAddr)>,
    addr: SocketAddr,
}

impl Listener for PlainIncoming {
    type Io = TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.rx.recv().await {
            Some(next) => next,
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.addr)
    }
}

async fn next_tcp(listener: &TcpListener, stop: &CancellationToken) -> Option<(TcpStream, SocketAddr)> {
    loop {
        tokio::select! {
            _ = stop.cancelled() => return None,
            got = listener.accept() => match got {
                Ok(pair) => return Some(pair),
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            },
        }
    }
}

pub(crate) fn serve_tls(listener: TcpListener, config: Arc<ServerConfig>, app: Router, stop: CancellationToken) -> Vec<JoinHandle<()>> {
    let addr = listener.local_addr().unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
    let (tx, rx) = mpsc::channel(WAITING);
    let acceptor = TlsAcceptor::from(config);
    let accept_stop = stop.clone();
    let shakes = Arc::new(tokio::sync::Semaphore::new(SHAKES));
    let accept = tokio::spawn(async move {
        while let Some((tcp, peer)) = next_tcp(&listener, &accept_stop).await {
            let Ok(permit) = shakes.clone().try_acquire_owned() else {
                drop(tcp);
                continue;
            };
            let acceptor = acceptor.clone();
            let tx = tx.clone();
            let stop = accept_stop.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let shake = tokio::time::timeout(HANDSHAKE, acceptor.accept(tcp));
                let done = tokio::select! {
                    _ = stop.cancelled() => return,
                    done = shake => done,
                };
                if let Ok(Ok(tls)) = done {
                    let _ = tx.send((tls, peer)).await;
                }
            });
        }
    });
    let server = tokio::spawn(async move {
        let _ = axum::serve(TlsIncoming { rx, addr }, app).with_graceful_shutdown(stop.cancelled_owned()).await;
    });
    vec![accept, server]
}

pub(crate) fn serve_plain(listener: TcpListener, app: Router, stop: CancellationToken) -> Vec<JoinHandle<()>> {
    let addr = listener.local_addr().unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
    let (tx, rx) = mpsc::channel(WAITING);
    let accept_stop = stop.clone();
    let accept = tokio::spawn(async move {
        while let Some(pair) = next_tcp(&listener, &accept_stop).await {
            if tx.send(pair).await.is_err() {
                break;
            }
        }
    });
    let server = tokio::spawn(async move {
        let _ = axum::serve(PlainIncoming { rx, addr }, app).with_graceful_shutdown(stop.cancelled_owned()).await;
    });
    vec![accept, server]
}
