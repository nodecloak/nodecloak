use std::{
    io::{Read, Write},
    net::{TcpListener, UdpSocket},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
pub(super) struct Fixture {
    pub(super) port: u16,
    pub(super) hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}
impl Fixture {
    pub(super) fn start() -> Self {
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = tcp.local_addr().unwrap().port();
        tcp.set_nonblocking(true).unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let udp_port = udp.local_addr().unwrap().port();
        udp.set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let th = hits.clone();
        let ts = stop.clone();
        std::thread::spawn(move || {
            while !ts.load(Ordering::Acquire) {
                match tcp.accept() {
                    Ok((mut stream, _)) => {
                        // Windows inherits the listener's nonblocking mode;
                        // handshake reads must wait for the next SOCKS frame.
                        stream.set_nonblocking(false).unwrap();
                        let hits = th.clone();
                        std::thread::spawn(move || {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(5)))
                                .unwrap();
                            let mut greeting = [0; 2];
                            if stream.read_exact(&mut greeting).is_err() {
                                return;
                            }
                            let mut methods = vec![0; greeting[1] as usize];
                            if stream.read_exact(&mut methods).is_err() {
                                return;
                            }
                            eprintln!("fixture SOCKS greeting: {greeting:?}, methods: {methods:?}");
                            if stream.write_all(&[5, 0]).is_err() {
                                return;
                            }
                            let mut request = [0; 4];
                            if stream.read_exact(&mut request).is_err() {
                                return;
                            }
                            eprintln!("fixture SOCKS request: {request:?}");
                            let count = match request[3] {
                                1 => 4,
                                4 => 16,
                                3 => {
                                    let mut n = [0];
                                    if stream.read_exact(&mut n).is_err() {
                                        return;
                                    }
                                    n[0] as usize
                                }
                                _ => return,
                            };
                            let mut target = vec![0; count + 2];
                            if stream.read_exact(&mut target).is_err() {
                                return;
                            }
                            if request[1] == 3 {
                                let mut reply = vec![5, 0, 0, 1, 127, 0, 0, 1];
                                reply.extend(udp_port.to_be_bytes());
                                let _ = stream.write_all(&reply);
                                let mut byte = [0];
                                let _ = stream.read(&mut byte);
                            } else if request[1] == 1 {
                                let _ = stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]);
                                let mut bytes = [0; 15];
                                let read = stream.read_exact(&mut bytes);
                                eprintln!("fixture TCP payload: {read:?}, {bytes:?}");
                                if read.is_ok() && &bytes == b"nodecloak-probe" {
                                    hits.fetch_add(1, Ordering::AcqRel);
                                    let _ = stream.write_all(b"via-proxy");
                                    std::thread::sleep(Duration::from_millis(250));
                                }
                            }
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        });
        let uh = hits.clone();
        let us = stop.clone();
        std::thread::spawn(move || {
            let mut bytes = [0; 2048];
            while !us.load(Ordering::Acquire) {
                if let Ok((n, peer)) = udp.recv_from(&mut bytes) {
                    let header = match bytes.get(3) {
                        Some(1) => 10,
                        Some(4) => 22,
                        Some(3) => 7 + bytes[4] as usize,
                        _ => continue,
                    };
                    if n >= header && &bytes[header..n] == b"nodecloak-probe" {
                        uh.fetch_add(1, Ordering::AcqRel);
                        let mut reply = bytes[..header].to_vec();
                        reply.extend(b"via-proxy");
                        let _ = udp.send_to(&reply, peer);
                    }
                }
            }
        });
        Self { port, hits, stop }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
