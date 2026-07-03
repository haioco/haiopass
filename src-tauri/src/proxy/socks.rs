use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RETRIES: u8 = 1;
const RETRY_DELAY: Duration = Duration::from_millis(500);

pub async fn dial_socks5(
    socks_addr: &str,
    target_host: &str,
    target_port: u16,
) -> crate::error::Result<TcpStream> {
    let mut last_err = None;

    for attempt in 0..=MAX_RETRIES {
        if attempt > 0 {
            tokio::time::sleep(RETRY_DELAY).await;
            tracing::info!(
                "SOCKS5 retry {}/{} for {}:{}",
                attempt,
                MAX_RETRIES,
                target_host,
                target_port
            );
        }

        match try_dial_socks5(socks_addr, target_host, target_port).await {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                tracing::warn!(
                    "SOCKS5 attempt {}/{} failed for {}:{}: {}",
                    attempt + 1,
                    MAX_RETRIES + 1,
                    target_host,
                    target_port,
                    e
                );
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        crate::error::HaioError::Proxy("SOCKS5: all retries exhausted".into())
    }))
}

async fn try_dial_socks5(
    socks_addr: &str,
    target_host: &str,
    target_port: u16,
) -> crate::error::Result<TcpStream> {
    // Connect with timeout
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(socks_addr))
        .await
        .map_err(|_| {
            crate::error::HaioError::Proxy(format!(
                "SOCKS5 connect to {} timed out after {:?}",
                socks_addr, CONNECT_TIMEOUT
            ))
        })?
        .map_err(|e| {
            crate::error::HaioError::Proxy(format!("SOCKS5 connect to {} failed: {}", socks_addr, e))
        })?;

    let mut stream = stream;

    // SOCKS5 handshake with timeout
    let auth_result = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        // Auth greeting: version 5, one method, no auth
        stream.write_all(&[0x05, 0x01, 0x00]).await?;

        let mut resp = [0u8; 2];
        stream.read_exact(&mut resp).await?;

        if resp[0] != 0x05 {
            return Err(crate::error::HaioError::Proxy(format!(
                "SOCKS5 server returned unexpected version: 0x{:02X}",
                resp[0]
            )));
        }
        if resp[1] != 0x00 {
            return Err(crate::error::HaioError::Proxy(format!(
                "SOCKS5 auth failed, method: 0x{:02X}",
                resp[1]
            )));
        }

        // CONNECT request: version 5, cmd connect, reserved, domain addr type
        let mut connect = vec![0x05, 0x01, 0x00, 0x03, target_host.len() as u8];
        connect.extend_from_slice(target_host.as_bytes());
        connect.extend_from_slice(&target_port.to_be_bytes());
        stream.write_all(&connect).await?;

        let mut resp = [0u8; 4];
        stream.read_exact(&mut resp).await?;

        if resp[1] != 0x00 {
            return Err(crate::error::HaioError::Proxy(format!(
                "SOCKS5 CONNECT failed with reply code: 0x{:02X}",
                resp[1]
            )));
        }

        // Skip variable-length bind address
        match resp[3] {
            0x01 => {
                let mut buf = [0u8; 4];
                stream.read_exact(&mut buf).await?;
            }
            0x03 => {
                let mut len = [0u8; 1];
                stream.read_exact(&mut len).await?;
                let mut domain = vec![0u8; len[0] as usize];
                stream.read_exact(&mut domain).await?;
                let mut port = [0u8; 2];
                stream.read_exact(&mut port).await?;
            }
            0x04 => {
                let mut buf = [0u8; 18];
                stream.read_exact(&mut buf).await?;
            }
            _ => {}
        }

        Ok::<_, crate::error::HaioError>(stream)
    })
    .await
    .map_err(|_| {
        crate::error::HaioError::Proxy(format!(
            "SOCKS5 handshake to {} timed out after {:?}",
            socks_addr, HANDSHAKE_TIMEOUT
        ))
    })??;

    Ok(auth_result)
}

pub async fn dial_direct(
    target_host: &str,
    target_port: u16,
) -> crate::error::Result<TcpStream> {
    let addr = format!("{}:{}", target_host, target_port);
    tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
        .await
        .map_err(|_| {
            crate::error::HaioError::Proxy(format!(
                "Direct connect to {} timed out after {:?}",
                addr, CONNECT_TIMEOUT
            ))
        })?
        .map_err(crate::error::HaioError::Io)
}
