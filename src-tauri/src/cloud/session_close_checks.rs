use super::*;
use std::{net::TcpListener, process::Command, sync::mpsc};

#[test]
fn isolated_quit_releases_transport_before_reporting_closed() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (released_tx, released_rx) = mpsc::channel();
    let peer = std::thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut socket = tungstenite::accept(tcp).unwrap();
        let request: Value =
            serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(
            request,
            json!({"id":1,"method":"browser.close","params":{}})
        );
        socket
            .send(Message::Text(
                json!({"type":"success","id":1,"result":{}})
                    .to_string()
                    .into(),
            ))
            .unwrap();
        // Firefox's server cannot complete shutdown while its control transport
        // remains open. An acknowledgement alone must not leave that transport held.
        let released = matches!(socket.read(), Ok(Message::Close(_)));
        released_tx.send(released).unwrap();
    });
    let (socket, _) = tungstenite::connect(format!("ws://{addr}")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let mut child = Command::new("cmd.exe")
        .args(["/C", "exit", "0"])
        .spawn()
        .unwrap();
    #[cfg(not(windows))]
    let mut child = Command::new("/bin/sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    child.wait().unwrap();
    let mut browser = Browser {
        socket,
        child,
        next: 0,
        firefox: true,
        target: String::new(),
        session: None,
        closed: false,
        path: dir.path().to_owned(),
        waiting: HashSet::new(),
        replies: HashMap::new(),
        frames: HashMap::new(),
        eval_target: None,
        eval_context: None,
    };
    browser.close().unwrap();
    assert!(
        released_rx.recv_timeout(Duration::from_secs(4)).unwrap(),
        "quit left the owned control transport open"
    );
    peer.join().unwrap();
    assert!(browser.closed);
}
