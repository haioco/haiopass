use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub async fn dial_socks5(
    socks_addr: &str,
    target_host: &str,
    target_port: u16,
) -> crate::error::Result<TcpStream> {
    let stream = TcpStream::connect(socks_addr).await?;

    let buf = vec![0x05, 0x01, 0x00]; // SOCKS version, one auth method, no auth

    let mut stream = stream;
    stream.write_all(&buf).await?;

    let mut resp = [0u8; 2];
    stream.read_exact(&mut resp).await?;

    // CONNECT request
    let mut connect = vec![0x05, 0x01, 0x00, 0x03, target_host.len() as u8];
    connect.extend_from_slice(target_host.as_bytes());
    connect.extend_from_slice(&target_port.to_be_bytes());

    stream.write_all(&connect).await?;

    let mut resp = [0u8; 4];
    stream.read_exact(&mut resp).await?;

    if resp[1] != 0x00 {
        return Err(crate::error::HaioError::Proxy(format!(
            "SOCKS5 connection failed with code 0x{:02X}",
            resp[1]
        )));
    }

    // Skip remaining address bytes (variable length based on address type)
    match resp[3] {
        0x01 => { let mut buf = [0u8; 4]; stream.read_exact(&mut buf).await?; } // IPv4
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut domain = vec![0u8; len[0] as usize];
            stream.read_exact(&mut domain).await?;
            let mut port = [0u8; 2];
            stream.read_exact(&mut port).await?;
        } // Domain
        0x04 => { let mut buf = [0u8; 18]; stream.read_exact(&mut buf).await?; } // IPv6
        _ => {}
    }

    Ok(stream)
}

pub async fn dial_direct(
    target_host: &str,
    target_port: u16,
) -> crate::error::Result<TcpStream> {
    let addr = format!("{}:{}", target_host, target_port);
    TcpStream::connect(addr).await.map_err(|e| e.into())
}
