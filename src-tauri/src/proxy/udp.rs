//! Trojan UDP framing and SOCKS5 UDP datagram helpers.
//!
//! A Trojan UDP relay session is a normal TLS stream whose payload is a
//! sequence of datagram frames:
//!
//! ```text
//! ATYP + ADDR + PORT   SOCKS5 address of the datagram's peer
//! u16 BE length        payload length
//! CRLF
//! payload
//! ```
//!
//! The address block includes the port and there is no separator between the
//! address and the length — an extra CRLF (or a SOCKS5 RSV/FRAG header) makes
//! the server reject the frame. Lengths above the server's 8 KiB limit are
//! rejected by the server, so they are dropped here first.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// Matches `Session::MAX_LENGTH` on the server (`src/session/session.h`).
pub const MAX_DATAGRAM: usize = 8192;

const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpFrame {
    pub address: SocketAddr,
    pub payload: Vec<u8>,
}

/// Address as it appears in a SOCKS5 UDP request: usually an IP, but clients
/// are allowed to send a hostname, which then has to be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocksAddress {
    Ip(SocketAddr),
    Domain(String, u16),
}

fn push_ip_address(out: &mut Vec<u8>, ip: IpAddr, port: u16) {
    match ip {
        IpAddr::V4(v4) => {
            out.push(ATYP_IPV4);
            out.extend_from_slice(&v4.octets());
        }
        IpAddr::V6(v6) => {
            out.push(ATYP_IPV6);
            out.extend_from_slice(&v6.octets());
        }
    }
    out.extend_from_slice(&port.to_be_bytes());
}

/// Encode one datagram frame (`address + length + CRLF + payload`).
pub fn encode_frame(address: SocketAddr, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 16 + 2 + 4 + payload.len());
    push_ip_address(&mut out, address.ip(), address.port());
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(payload);
    out
}

/// Parse one frame. Returns `None` when the buffer does not yet hold a whole
/// frame; callers should read more bytes and retry.
pub fn decode_frame(buf: &[u8]) -> Option<(UdpFrame, usize)> {
    let (address, address_len) = decode_ip_address(buf)?;
    if buf.len() < address_len + 4 {
        return None;
    }
    let length = u16::from_be_bytes([buf[address_len], buf[address_len + 1]]) as usize;
    if &buf[address_len + 2..address_len + 4] != b"\r\n" {
        return None;
    }
    let end = address_len + 4 + length;
    if buf.len() < end {
        return None;
    }
    Some((
        UdpFrame {
            address,
            payload: buf[address_len + 4..end].to_vec(),
        },
        end,
    ))
}

fn decode_ip_address(buf: &[u8]) -> Option<(SocketAddr, usize)> {
    let atyp = *buf.first()?;
    match atyp {
        ATYP_IPV4 => {
            if buf.len() < 7 {
                return None;
            }
            let mut octets = [0u8; 4];
            octets.copy_from_slice(&buf[1..5]);
            let port = u16::from_be_bytes([buf[5], buf[6]]);
            Some((SocketAddr::new(IpAddr::V4(Ipv4Addr::from(octets)), port), 7))
        }
        ATYP_IPV6 => {
            if buf.len() < 19 {
                return None;
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&buf[1..17]);
            let port = u16::from_be_bytes([buf[17], buf[18]]);
            Some((SocketAddr::new(IpAddr::V6(Ipv6Addr::from(octets)), port), 19))
        }
        // The server always answers with an IP address; a domain here is a
        // protocol violation.
        _ => None,
    }
}

/// Parse a SOCKS5 UDP request datagram: `RSV(2) FRAG ATYP ADDR PORT payload`.
pub fn parse_socks5_udp(packet: &[u8]) -> Option<(u8, SocksAddress, &[u8])> {
    if packet.len() < 4 || packet[0] != 0 || packet[1] != 0 {
        return None;
    }
    let frag = packet[2];
    let (address, address_len) = decode_socks_address(&packet[3..])?;
    Some((frag, address, &packet[3 + address_len..]))
}

fn decode_socks_address(buf: &[u8]) -> Option<(SocksAddress, usize)> {
    let atyp = *buf.first()?;
    match atyp {
        ATYP_IPV4 => {
            if buf.len() < 7 {
                return None;
            }
            let mut octets = [0u8; 4];
            octets.copy_from_slice(&buf[1..5]);
            let port = u16::from_be_bytes([buf[5], buf[6]]);
            Some((
                SocksAddress::Ip(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(octets)), port)),
                7,
            ))
        }
        ATYP_IPV6 => {
            if buf.len() < 19 {
                return None;
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&buf[1..17]);
            let port = u16::from_be_bytes([buf[17], buf[18]]);
            Some((
                SocksAddress::Ip(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(octets)), port)),
                19,
            ))
        }
        ATYP_DOMAIN => {
            let len = *buf.get(1)? as usize;
            if len == 0 || buf.len() < 2 + len + 2 {
                return None;
            }
            let name = String::from_utf8(buf[2..2 + len].to_vec()).ok()?;
            let port = u16::from_be_bytes([buf[2 + len], buf[3 + len]]);
            Some((SocksAddress::Domain(name, port), 4 + len))
        }
        _ => None,
    }
}

/// Build a SOCKS5 UDP reply datagram (`RSV(2) FRAG=0 ATYP ADDR PORT payload`).
pub fn encode_socks5_udp(address: SocketAddr, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 16 + 2 + payload.len());
    out.extend_from_slice(&[0, 0, 0]);
    push_ip_address(&mut out, address.ip(), address.port());
    out.extend_from_slice(payload);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn frame_round_trip_v4() {
        let payload = b"hello".to_vec();
        let encoded = encode_frame(addr("1.2.3.4:53"), &payload);
        let (frame, consumed) = decode_frame(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(frame.address, addr("1.2.3.4:53"));
        assert_eq!(frame.payload, payload);
    }

    #[test]
    fn frame_round_trip_v6() {
        let encoded = encode_frame(addr("[2001:db8::1]:3074"), b"abc");
        let (frame, consumed) = decode_frame(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(frame.address, addr("[2001:db8::1]:3074"));
    }

    #[test]
    fn frame_truncated_is_incomplete() {
        let encoded = encode_frame(addr("1.2.3.4:53"), b"hello");
        for cut in 1..encoded.len() {
            assert_eq!(decode_frame(&encoded[..cut]), None, "cut at {}", cut);
        }
    }

    #[test]
    fn frame_rejects_bad_separator() {
        let mut encoded = encode_frame(addr("1.2.3.4:53"), b"hello");
        encoded[9] = b'X';
        assert_eq!(decode_frame(&encoded), None);
    }

    #[test]
    fn socks5_udp_v4_round_trip() {
        let packet = encode_socks5_udp(addr("1.1.1.1:53"), b"dns");
        let (frag, address, payload) = parse_socks5_udp(&packet).unwrap();
        assert_eq!(frag, 0);
        assert_eq!(address, SocksAddress::Ip(addr("1.1.1.1:53")));
        assert_eq!(payload, b"dns");
    }

    #[test]
    fn socks5_udp_domain() {
        let mut packet = vec![0, 0, 0, ATYP_DOMAIN, 11];
        packet.extend_from_slice(b"example.com");
        packet.extend_from_slice(&53u16.to_be_bytes());
        packet.extend_from_slice(b"q");
        let (_, address, payload) = parse_socks5_udp(&packet).unwrap();
        assert_eq!(address, SocksAddress::Domain("example.com".into(), 53));
        assert_eq!(payload, b"q");
    }
}
