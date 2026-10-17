use std::net::{IpAddr, SocketAddr};
use url::Url;

pub(crate) struct Authority {
    pub origin: String,
    pub host: String,
    pub connect: String,
    pub addresses: Vec<SocketAddr>,
}

impl Authority {
    pub fn new(origin: String, addresses: Vec<SocketAddr>) -> Result<Self, ()> {
        let url = Url::parse(&origin).map_err(|_| ())?;
        if url.scheme() != "https"
            || url.origin().ascii_serialization() != origin
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || addresses.is_empty()
            || addresses.len() > 8
        {
            return Err(());
        }
        let host = url.host_str().ok_or(())?;
        let port = url.port_or_known_default().ok_or(())?;
        if addresses.iter().any(|address| {
            address.port() != port || address.ip().is_unspecified() || address.ip().is_multicast()
        }) {
            return Err(());
        }
        let connect = format!("{host}:{port}");
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        if let Ok(ip) = host.parse::<IpAddr>() {
            if addresses.iter().any(|address| address.ip() != ip) {
                return Err(());
            }
        } else if host == "localhost" && addresses.iter().any(|address| !address.ip().is_loopback())
        {
            return Err(());
        }
        Ok(Self {
            origin,
            host,
            connect,
            addresses,
        })
    }

    pub fn admits_host(&self, value: &str) -> bool {
        if value.is_empty() || value.contains(['/', '?', '#', '@', '\\']) {
            return false;
        }
        Url::parse(&format!("https://{value}/")).is_ok_and(|url| {
            url.origin().ascii_serialization() == self.origin
                && url.username().is_empty()
                && url.password().is_none()
        })
    }

    pub fn admits_sni(&self, value: Option<&str>) -> bool {
        match self.host.parse::<IpAddr>() {
            Ok(_) => value.is_none(),
            Err(_) => value.is_some_and(|value| value.eq_ignore_ascii_case(&self.host)),
        }
    }
}
