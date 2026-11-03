// Disposable executable used only by administrator-only routing checks.
use std::{
    io::{Read, Write},
    net::{TcpStream, UdpSocket},
    time::Duration,
};
fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let result = if mode == "udp" {
        (|| {
            let socket = UdpSocket::bind("0.0.0.0:0")?;
            socket.set_read_timeout(Some(Duration::from_secs(3)))?;
            socket.send_to(b"nodecloak-probe", "198.18.0.10:43210")?;
            let mut bytes = [0u8; 128];
            let (n, _) = socket.recv_from(&mut bytes)?;
            Ok::<_, std::io::Error>(bytes[..n].to_vec())
        })()
    } else {
        (|| {
            let mut stream = TcpStream::connect_timeout(
                &"198.18.0.10:43210".parse().unwrap(),
                Duration::from_secs(3),
            )?;
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            stream.write_all(b"nodecloak-probe")?;
            let mut bytes = [0u8; 9];
            stream.read_exact(&mut bytes)?;
            Ok::<_, std::io::Error>(bytes.to_vec())
        })()
    };
    match result {
        Ok(bytes) if bytes == b"via-proxy" => println!("via-proxy"),
        _ => std::process::exit(2),
    }
}
