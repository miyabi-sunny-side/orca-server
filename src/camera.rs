//! One still image from the printer's LAN camera (P1/A1 series, TLS port 6000).
//!
//! Protocol as implemented by ha-bambulab `ChamberImageThread` (cd67ed9): an 80-byte login
//! (`0x40`, `0x3000`, two zero words, user and access code padded to 32 bytes), then frames of a
//! 16-byte header whose first little-endian word is the JPEG size, followed by the JPEG.

/// Largest frame accepted; P1S frames are well below 1 MiB.
const MAX_FRAME: usize = 4 * 1024 * 1024;

/// The login packet.
pub(crate) fn login(access_code: &str) -> Result<[u8; 80], &'static str> {
    if access_code.is_empty() || access_code.len() > 32 || !access_code.is_ascii() {
        return Err("Access code must be 1-32 ASCII characters for the camera");
    }
    let mut packet = [0u8; 80];
    packet[0] = 0x40;
    packet[5] = 0x30;
    packet[16..20].copy_from_slice(b"bblp");
    packet[48..48 + access_code.len()].copy_from_slice(access_code.as_bytes());
    Ok(packet)
}

/// The JPEG size announced by a frame header.
pub(crate) fn frame_size(header: &[u8; 16]) -> Result<usize, &'static str> {
    let size = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err("Camera announced an invalid image size");
    }
    Ok(size)
}

/// Whether `bytes` is one complete JPEG image.
pub(crate) fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes.starts_with(&[0xff, 0xd8]) && bytes.ends_with(&[0xff, 0xd9])
}

/// Connect, log in and read one frame. Errors name the failed stage only.
pub(crate) async fn snapshot(
    tls: std::sync::Arc<rustls::ClientConfig>,
    ip: std::net::IpAddr,
    port: u16,
    access_code: &str,
) -> Result<Vec<u8>, &'static str> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let login = login(access_code)?;
    let stream = tokio::net::TcpStream::connect((ip, port))
        .await
        .map_err(|_| "connect")?;
    let mut stream = suppaftp::tokio_rustls::TlsConnector::from(tls)
        .connect(rustls::pki_types::ServerName::IpAddress(ip.into()), stream)
        .await
        .map_err(|_| "tls")?;
    stream.write_all(&login).await.map_err(|_| "login")?;
    let mut header = [0u8; 16];
    stream.read_exact(&mut header).await.map_err(|_| "frame")?;
    let mut image = vec![0u8; frame_size(&header)?];
    stream.read_exact(&mut image).await.map_err(|_| "frame")?;
    if is_jpeg(&image) {
        Ok(image)
    } else {
        Err("frame")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_pads_user_and_access_code() {
        let packet = login("12345678").unwrap();
        assert_eq!(
            &packet[..16],
            &[0x40, 0, 0, 0, 0, 0x30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(&packet[16..20], b"bblp");
        assert!(packet[20..48].iter().all(|b| *b == 0));
        assert_eq!(&packet[48..56], b"12345678");
        assert!(packet[56..].iter().all(|b| *b == 0));
        assert!(login(&"x".repeat(33)).is_err());
        assert!(login("").is_err());
        assert!(login("ünicode").is_err());
    }

    #[test]
    fn frame_headers_bound_the_image_size() {
        let header = |size: u32| {
            let mut h = [0u8; 16];
            h[..4].copy_from_slice(&size.to_le_bytes());
            h[8] = 1;
            h
        };
        assert_eq!(frame_size(&header(54_321)), Ok(54_321));
        assert!(frame_size(&header(0)).is_err());
        assert!(frame_size(&header(u32::try_from(MAX_FRAME).unwrap() + 1)).is_err());
    }

    #[test]
    fn only_complete_jpegs_are_images() {
        assert!(is_jpeg(&[0xff, 0xd8, 0xff, 0xe0, 1, 2, 0xff, 0xd9]));
        assert!(!is_jpeg(&[0xff, 0xd8, 0xff, 0xe0, 1, 2]));
        assert!(!is_jpeg(b"not an image"));
        assert!(!is_jpeg(&[]));
    }
}
