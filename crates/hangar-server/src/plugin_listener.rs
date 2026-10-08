use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio::net::TcpListener;

pub(crate) async fn bind(public: &TcpListener) -> io::Result<Option<TcpListener>> {
    let address = public.local_addr()?;
    let v6_only = if address.is_ipv6() {
        socket2::SockRef::from(public).only_v6().map_err(|error| {
            io::Error::new(error.kind(), format!("não foi possível conferir IPv6 da ponte do plugin: {error}"))
        })?
    } else {
        false
    };
    if covers_loopback(address, v6_only) {
        return Ok(None);
    }
    let loopback = SocketAddr::from((Ipv4Addr::LOCALHOST, address.port()));
    TcpListener::bind(loopback).await.map(Some).map_err(|error| {
        io::Error::new(error.kind(), format!("não foi possível abrir a ponte do plugin em {loopback}: {error}"))
    })
}

fn covers_loopback(address: SocketAddr, v6_only: bool) -> bool {
    match address.ip() {
        IpAddr::V4(ip) => ip.is_unspecified() || ip == Ipv4Addr::LOCALHOST,
        IpAddr::V6(ip) => !v6_only && (ip.is_unspecified()
            || ip.to_ipv4_mapped().is_some_and(|ip| ip.is_unspecified() || ip == Ipv4Addr::LOCALHOST)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_uses_the_announced_ipv4_address_and_the_socket_family() {
        for (address, v6_only, expected) in [
            ("127.0.0.1:8765", false, true),
            ("0.0.0.0:8765", false, true),
            ("127.0.0.2:8765", false, false),
            ("192.0.2.1:8765", false, false),
            ("100.64.0.1:8765", false, false),
            ("[::1]:8765", false, false),
            ("[::]:8765", false, true),
            ("[::]:8765", true, false),
            ("[::ffff:127.0.0.1]:8765", false, true),
            ("[::ffff:127.0.0.2]:8765", false, false),
            ("[::ffff:127.0.0.1]:8765", true, false),
        ] {
            assert_eq!(covers_loopback(address.parse().unwrap(), v6_only), expected, "{address}, v6_only={v6_only}");
        }
    }

    #[tokio::test]
    async fn specific_loopback_reserves_the_announced_address_at_the_effective_port() {
        let public = TcpListener::bind("127.0.0.2:0").await.unwrap();
        let address = public.local_addr().unwrap();
        let bridge = bind(&public).await.unwrap().unwrap();
        assert_eq!(bridge.local_addr().unwrap(), SocketAddr::from((Ipv4Addr::LOCALHOST, address.port())));
        assert_ne!(address.port(), 0);
        assert!(TcpListener::bind(bridge.local_addr().unwrap()).await.is_err());
    }

    #[tokio::test]
    async fn ipv4_wildcard_and_exact_loopback_do_not_bind_twice() {
        for address in ["0.0.0.0:0", "127.0.0.1:0"] {
            let public = TcpListener::bind(address).await.unwrap();
            assert!(bind(&public).await.unwrap().is_none(), "{address}");
        }
    }

    #[tokio::test]
    async fn occupied_loopback_is_not_mistaken_for_our_listener() {
        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = occupied.local_addr().unwrap().port();
        let public = TcpListener::bind((Ipv4Addr::new(127, 0, 0, 2), port)).await.unwrap();
        let error = bind(&public).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert!(error.to_string().contains("ponte do plugin"));
    }

    #[tokio::test]
    async fn ipv6_wildcard_inspects_the_real_dual_stack_setting() {
        for v6_only in [false, true] {
            let socket = socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::STREAM, Some(socket2::Protocol::TCP)).unwrap();
            socket.set_only_v6(v6_only).unwrap();
            socket.set_nonblocking(true).unwrap();
            socket.bind(&"[::]:0".parse::<SocketAddr>().unwrap().into()).unwrap();
            socket.listen(128).unwrap();
            let public = TcpListener::from_std(socket.into()).unwrap();
            let bridge = bind(&public).await.unwrap();
            assert_eq!(bridge.is_some(), v6_only);
            let ipv4 = SocketAddr::from((Ipv4Addr::LOCALHOST, public.local_addr().unwrap().port()));
            assert!(tokio::net::TcpStream::connect(ipv4).await.is_ok());
        }
    }
}
