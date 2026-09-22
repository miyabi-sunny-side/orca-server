use std::io::{self, Read};

pub fn packet(header: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![header];
    let mut length = body.len();
    loop {
        let digit = u8::try_from(length % 128).unwrap();
        length /= 128;
        bytes.push(digit | if length == 0 { 0 } else { 128 });
        if length == 0 {
            break;
        }
    }
    bytes.extend_from_slice(body);
    bytes
}

pub fn read_packet(peer: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0];
    peer.read_exact(&mut header)?;
    let mut length = 0;
    for shift in [0, 7, 14, 21] {
        let mut digit = [0];
        peer.read_exact(&mut digit)?;
        length |= usize::from(digit[0] & 127) << shift;
        if length > 1024 * 1024 {
            break;
        }
        if digit[0] & 128 == 0 {
            let mut body = vec![0; length];
            peer.read_exact(&mut body)?;
            return Ok((header[0], body));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid fixture MQTT length",
    ))
}
