use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use tokio::net::{TcpListener, TcpStream};

pub(crate) struct Listeners {
    pub v4: TcpListener,
    pub v6: Option<TcpListener>,
}

impl Listeners {
    pub async fn ephemeral_v4() -> std::io::Result<Self> {
        Ok(Self {
            v4: TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?,
            v6: None,
        })
    }

    pub fn exact_loopbacks(port: u16) -> std::io::Result<Self> {
        let v6 = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
        v6.set_only_v6(true)?;
        v6.set_nonblocking(true)?;
        v6.bind(&SocketAddr::from((Ipv6Addr::LOCALHOST, port)).into())?;
        v6.listen(128)?;
        let v4 = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
        v4.set_nonblocking(true)?;
        v4.bind(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)).into())?;
        v4.listen(128)?;
        Ok(Self {
            v4: TcpListener::from_std(v4.into())?,
            v6: Some(TcpListener::from_std(v6.into())?),
        })
    }

    pub async fn accept_v6(&self) -> std::io::Result<(TcpStream, SocketAddr)> {
        match &self.v6 {
            Some(v6) => v6.accept().await,
            None => std::future::pending().await,
        }
    }
}
