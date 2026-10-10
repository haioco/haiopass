use std::time::Duration;
use tokio::net::TcpStream;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

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
